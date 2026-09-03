// Kura CLI 어댑터 (개발 22) — 터미널/스크립트에서 로컬 지갑을 다룬다.
//
// 코어 설계 `[Rust 코어] ← MCP / CLI / App Intents` 의 CLI 자리. MCP 서버(main.rs)와 **같은 lib**
// (kura_mcp::{wallet, flow, payment, chain})을 공유하므로 결제·보안 로직이 한 벌이다(분기 없음).
//
// 핵심 보안: 비밀번호는 절대 CLI/인자로 받지 않는다. 결제(pay/fetch)는 지갑 앱(GUI)이 팝업으로
// 사람 승인을 받아야만 실행된다. 한도·긴급잠금·화이트리스트는 앱이 강제한다.
//
// 읽기 명령(status/balance/history)은 비번 없이 즉시. 결제 명령은 GUI 승인을 최대 5분 기다린다.

use std::collections::HashMap;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use kura_mcp::chain::active_chain;
use kura_mcp::erc8004::AgentTrust;
use kura_mcp::flow::{self, X402Outcome};
use kura_mcp::i18n::{lang, Lang};
use kura_mcp::wallet;
use kura_mcp::{tf, ts};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP_KO: &str = "\
Kura — AI 에이전트 전용 로컬 지갑 CLI

사용법:
  kura status                  지갑 상태와 주소
  kura balance                 ETH(가스) · USDC(결제) 잔액
  kura history [--limit N]     최근 거래 내역 (기본 20, 최대 200)
  kura pay <주소> <금액> [옵션]    결제(송금) 요청 → 지갑 앱에서 비번 승인
       --token USDC|ETH        토큰 (기본 USDC)
       --memo \"사유\"            승인 팝업에 보일 결제 사유
       --agent N               받는 쪽 ERC-8004 번호 (등록 지갑과 대조)
  kura fetch <URL> [--memo \"사유\"] [--agent N]   x402 유료 리소스를 결제하고 가져온다

전역 옵션:
  --json        기계가 읽는 JSON 으로 출력 (스크립트용)
  -h, --help    이 도움말
  -V, --version 버전

보안: 비밀번호는 절대 CLI 로 받지 않습니다. 결제는 기본값으로 지갑 앱이 팝업으로 사람 승인을
받아야 실행되고(최대 5분 대기; 앱에서 자율 결제를 켠 경우만 그 한도 안에서 자동 승인),
단일/일일 한도·긴급잠금·화이트리스트는 앱이 강제합니다.";

const HELP_EN: &str = "\
Kura — a local wallet CLI for AI agents

Usage:
  kura status                     wallet state and address
  kura balance                    ETH (gas) · USDC (payments) balances
  kura history [--limit N]        recent transactions (default 20, max 200)
  kura pay <address> <amount>     ask to pay → approve with your password in the app
       --token USDC|ETH           token (USDC by default)
       --memo \"reason\"            what the payment is for, shown in the approval window
       --agent N                  recipient's ERC-8004 number (compared with the registered wallet)
  kura fetch <URL> [--memo \"reason\"] [--agent N]   pay for an x402 resource and fetch it

Global options:
  --json        machine-readable JSON output (for scripts)
  -h, --help    this help
  -V, --version version

Security: this CLI never takes your password. By default a payment only goes out after you
approve it in the wallet app (it waits up to 5 minutes; automatic approval happens only within
the limit you set if you turned autopay on), and the app enforces the per-payment and daily
limits, the emergency lock, and the allowlist.";

/// 도움말은 사람이 읽는 화면이라 언어를 탄다. 상수는 두 벌로 두고 여기서 고른다.
fn help() -> &'static str {
    ts!(HELP_KO, HELP_EN)
}

/// 파싱된 명령줄 — 전역 플래그 + 위치 인자 + 값 옵션.
struct Cli {
    json: bool,
    positionals: Vec<String>,
    opts: HashMap<String, String>,
}

/// 값을 받는 옵션들(`--key value` 또는 `--key=value`). 나머지 `--flag` 는 불리언.
const VALUE_OPTS: [&str; 4] = ["token", "memo", "limit", "agent"];

enum Parsed {
    Run(Cli),
    Help,
    Version,
    Error(String),
}

/// 인자를 파싱한다. `--help`/`--version` 은 어디에 있어도 즉시 처리. `--key value` 와
/// `--key=value` 둘 다 지원. 알 수 없는 옵션은 오류(오타를 조용히 삼키지 않게).
fn parse(args: &[String]) -> Parsed {
    let mut cli = Cli {
        json: false,
        positionals: Vec::new(),
        opts: HashMap::new(),
    };
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "-h" | "--help" => return Parsed::Help,
            "-V" | "--version" => return Parsed::Version,
            "--json" => cli.json = true,
            _ if a.starts_with("--") => {
                let body = &a[2..];
                // --key=value 형태.
                if let Some((k, v)) = body.split_once('=') {
                    if !VALUE_OPTS.contains(&k) {
                        return Parsed::Error(tf!(
                            "알 수 없는 옵션: --{k}",
                            "Unknown option: --{k}"
                        ));
                    }
                    cli.opts.insert(k.to_string(), v.to_string());
                } else if VALUE_OPTS.contains(&body) {
                    // --key value 형태: 다음 인자가 값. 값이 옵션처럼(--) 시작하면 값 누락으로 본다
                    // (예: `--memo --json` 이 --json 을 memo 로 삼키는 실수 방지). 의도적으로 --로
                    // 시작하는 값을 주려면 `--key=값` 형태를 쓴다.
                    match args.get(i + 1) {
                        Some(v) if !v.starts_with("--") => {
                            cli.opts.insert(body.to_string(), v.clone());
                            i += 1;
                        }
                        _ => {
                            return Parsed::Error(tf!(
                                "--{body} 에 값이 필요해요 (값이 --로 시작하면 --{body}=값 형태로 주세요)",
                                "--{body} needs a value (if the value starts with --, write it as --{body}=value)"
                            ))
                        }
                    }
                } else {
                    return Parsed::Error(tf!("알 수 없는 옵션: {a}", "Unknown option: {a}"));
                }
            }
            _ => cli.positionals.push(a.clone()),
        }
        i += 1;
    }
    Parsed::Run(cli)
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match parse(&args) {
        Parsed::Help => {
            println!("{}", help());
            return ExitCode::SUCCESS;
        }
        Parsed::Version => {
            println!("kura {VERSION}");
            return ExitCode::SUCCESS;
        }
        Parsed::Error(msg) => {
            eprintln!("{msg}\n\n{}", help());
            return ExitCode::FAILURE;
        }
        Parsed::Run(cli) => cli,
    };

    let cmd = cli.positionals.first().map(String::as_str).unwrap_or("");
    if cmd.is_empty() {
        println!("{}", help());
        return ExitCode::SUCCESS;
    }
    // 잘못된 KURA_CHAIN_ID 면 어떤 출력보다 먼저 즉시 종료한다(부분 출력 방지 — 검증은 active_chain
    // 안에 있고 lazy 라서, 명시적으로 한 번 당겨 fail-fast 시킨다).
    let _ = active_chain();
    let rest = &cli.positionals[cli.positionals.len().min(1)..];

    // 결제 명령은 성공 여부(bool)를 돌려준다. 읽기 명령은 성공하면 true.
    // ⚠️ stdout 출력 뒤 process::exit 를 쓰지 않는다 — 그러면 파이프/리다이렉트 시 버퍼가 flush 안
    //    돼 결과가 유실될 수 있다. main 이 ExitCode 를 정상 반환하면 런타임이 stdout 을 flush 한다.
    let result: Result<bool, String> = match cmd {
        "status" => cmd_status(&cli).await.map(|_| true),
        "balance" | "balances" => cmd_balance(&cli).await.map(|_| true),
        "history" => cmd_history(&cli).await.map(|_| true),
        "pay" => cmd_pay(&cli, rest).await,
        "fetch" => cmd_fetch(&cli, rest).await,
        other => {
            let help = help();
            Err(tf!(
                "알 수 없는 명령: {other}\n\n{help}",
                "Unknown command: {other}\n\n{help}"
            ))
        }
    };

    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE, // 결제 거부/실패 — 출력은 이미 했고 정상 리턴이라 flush 보장
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

// ── 읽기 명령 (비번 불필요, 즉시) ───────────────────────────────────────────

async fn cmd_status(cli: &Cli) -> Result<(), String> {
    let s = wallet::wallet_status()?;
    if cli.json {
        print_json(&s)?;
        return Ok(());
    }
    let state = match s.state.as_str() {
        "encrypted" => ts!("암호화됨 ✓", "encrypted ✓"),
        "legacy" => ts!(
            "평문 (앱에서 비번 설정 필요)",
            "unencrypted (set a password in the app)"
        ),
        _ => ts!(
            "없음 (앱에서 지갑을 먼저 만드세요)",
            "none (create a wallet in the app first)"
        ),
    };
    println!("{}", tf!("지갑      {state}", "Wallet    {state}"));
    if let Some(addr) = &s.address {
        println!("{}", tf!("주소      {addr}", "Address   {addr}"));
        // 계정이 둘 이상일 때만 어느 계정인지 말한다 (개발 54) — 하나뿐이면 말할 게 없다.
        if s.accounts.len() > 1 {
            let label = s
                .accounts
                .iter()
                .find(|a| a.index == s.account)
                .map(|a| a.label.clone())
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| tf!("계정 {}", "Account {}", s.account + 1));
            println!(
                "{}",
                tf!(
                    "계정      {label} ({}개 중 활성)",
                    "Account   {label} (active of {})",
                    s.accounts.len()
                )
            );
        }
        println!(
            "{}",
            tf!(
                "백업      {}",
                "Backup    {}",
                if s.backed_up {
                    ts!("완료", "done")
                } else {
                    ts!("안 됨 — 시드 백업 권장", "not yet — back up your 12 words")
                }
            )
        );
    }
    println!("{}", tf!("네트워크  {}", "Network   {}", network_label()));
    Ok(())
}

async fn cmd_balance(cli: &Cli) -> Result<(), String> {
    let s = wallet::wallet_status()?;
    let addr = s.address.ok_or_else(|| {
        ts!(
            "지갑이 아직 없어요. 앱에서 먼저 만드세요.",
            "There's no wallet yet. Create one in the app first."
        )
        .to_string()
    })?;
    let b = wallet::get_balances(&addr).await?;
    if cli.json {
        print_json(&b)?;
        return Ok(());
    }
    println!(
        "{}",
        tf!(
            "USDC  {}  (결제용)",
            "USDC  {}  (for payments)",
            trim_zeros(&b.usdc)
        )
    );
    // 네이티브가 곧 USDC 인 체인(Arc)엔 따로 셀 가스 잔액이 없다 — 줄을 지우는 대신
    // "가스도 여기서 나간다"를 한 줄로 말해 준다(빈자리보다 사실이 낫다).
    match &b.eth {
        Some(eth) => println!(
            "{}",
            tf!("ETH   {}  (가스용)", "ETH   {}  (for gas)", trim_zeros(eth))
        ),
        None => println!(
            "{}",
            ts!(
                "가스   위 USDC 에서 나가요 (이 체인은 가스도 USDC)",
                "Gas    comes out of the USDC above (this chain pays gas in USDC)"
            )
        ),
    }
    println!("{}", tf!("네트워크  {}", "Network   {}", network_label()));
    Ok(())
}

async fn cmd_history(cli: &Cli) -> Result<(), String> {
    // 저장소가 최대 200건만 보관 → help 와 일치하게 200 으로 클램프(그 이상은 보여줄 게 없다).
    let limit = match cli.opts.get("limit") {
        Some(v) => v
            .parse::<usize>()
            .map_err(|_| {
                tf!(
                    "--limit 은 0 이상의 정수여야 해요: {v}",
                    "--limit must be a whole number, 0 or more: {v}"
                )
            })?
            .min(200),
        None => 20,
    };
    let mut list = wallet::read_history();
    list.truncate(limit);
    if cli.json {
        print_json(&list)?;
        return Ok(());
    }
    if list.is_empty() {
        println!(
            "{}",
            ts!("아직 거래 내역이 없어요.", "No transactions yet.")
        );
        return Ok(());
    }
    let now = now_secs();
    for e in &list {
        // 정산된 x402 는 정산 tx, 그 외는 detail(보통 tx 해시)을 증빙으로 보여준다.
        let hash = if !e.settle_tx.is_empty() {
            &e.settle_tx
        } else {
            &e.detail
        };
        let mut line = format!(
            "{}  ·  {} {}  ·  {}  ·  → {}",
            status_label(&e.status, lang()),
            e.token,
            e.amount,
            rel_time(now, e.ts, lang()),
            shorten(&e.to),
        );
        if hash.starts_with("0x") {
            line.push_str(&format!("  ·  tx {}", shorten(hash)));
        }
        println!("{line}");
    }
    Ok(())
}

// ── 결제 명령 (지갑 앱 승인 필요, 최대 5분 대기) ────────────────────────────

/// 반환: 결제 성공(approved) 여부. 거부/실패면 false → main 이 종료코드 1.
async fn cmd_pay(cli: &Cli, rest: &[String]) -> Result<bool, String> {
    let to = rest.first().ok_or_else(|| {
        ts!(
            "받는 주소가 필요해요.\n\nkura pay <주소> <금액> [--token USDC|ETH] [--memo \"사유\"]",
            "A recipient address is required.\n\nkura pay <address> <amount> [--token USDC|ETH] [--memo \"reason\"]"
        )
        .to_string()
    })?;
    let amount = rest.get(1).ok_or_else(|| {
        ts!(
            "금액이 필요해요.\n\nkura pay <주소> <금액> [--token USDC|ETH] [--memo \"사유\"]",
            "An amount is required.\n\nkura pay <address> <amount> [--token USDC|ETH] [--memo \"reason\"]"
        )
        .to_string()
    })?;
    let token = cli.opts.get("token").map(String::as_str).unwrap_or("USDC");
    let memo = cli.opts.get("memo").map(String::as_str).unwrap_or("");
    let agent_id = agent_opt(cli)?;

    // 「기다리는 중」은 **요청이 실제로 나간 뒤** 찍는다(개발 50 이월). 예전엔 이 줄이 먼저라,
    // 토큰이 틀렸거나 앱이 꺼져 있어 요청이 나가지도 않았는데 기다린다고 말했다.
    let quiet = cli.json;
    let out = flow::run_payment(token, to, amount, memo, agent_id, || {
        if !quiet {
            // 🔴 `println!` 을 쓰면 안 된다 (코덱스 개발51 2차 P2). 이 콜백은 **요청 파일을 이미
            // 만든 뒤**에 불린다 — stdout 이 닫혀 있으면(`kura pay … | head -1`) println! 이
            // 패닉하고, 그 되감기가 await_result·cancel_request 를 건너뛰어 **요청 파일이 고아로
            // 남는다**. MCP 의 `has_pending()` 은 파일 존재만 보므로 그 뒤 모든 결제가 막힌다.
            // 실패해도 무시하는 쓰기로 바꾼다 — 안내 한 줄 때문에 지갑이 잠기면 안 된다.
            use std::io::Write;
            let _ = writeln!(
                std::io::stdout(),
                "{}",
                ts!(
                    "지갑 앱 승인을 기다리는 중… (최대 5분, 앱 팝업에서 비번 입력)",
                    "Waiting for approval in the wallet app… (up to 5 minutes; type your password there)"
                )
            );
        }
    })
    .await?;
    let success = out.status == "approved";

    if cli.json {
        let mut body = serde_json::json!({
            "status": out.status,
            "tx_hash": out.tx_hash,
            "detail": out.detail,
            "explorer": out.explorer,
        });
        if let Some(a) = &out.agent {
            body["agent"] = serde_json::to_value(a).unwrap_or(serde_json::Value::Null);
        }
        if !out.agent_note.is_empty() {
            body["agent_note"] = serde_json::Value::String(out.agent_note.clone());
        }
        print_json(&body)?;
    } else {
        if let Some(a) = &out.agent {
            println!("{}", agent_line(a));
        } else if !out.agent_note.is_empty() {
            println!("{}", out.agent_note);
        }
        match out.status.as_str() {
            "approved" => {
                println!("{}", ts!("✓ 승인됨", "✓ Approved"));
                if !out.tx_hash.is_empty() {
                    println!("  tx    {}", out.tx_hash);
                }
                if !out.explorer.is_empty() {
                    println!("{}", tf!("  보기  {}", "  view  {}", out.explorer));
                }
            }
            "rejected" => println!(
                "{}",
                ts!("✗ 사용자가 거부했어요.", "✗ The user rejected it.")
            ),
            _ => eprintln!(
                "{}",
                tf!(
                    "✗ 실패: {}",
                    "✗ Failed: {}",
                    if out.detail.is_empty() {
                        ts!("알 수 없는 오류", "unknown error")
                    } else {
                        &out.detail
                    }
                )
            ),
        }
    }
    // 거부/실패는 종료코드 1(json/사람 모드 동일). process::exit 대신 bool 을 올려 main 이 정상
    // 반환하게 한다 — stdout 이 flush 되도록(파이프에서도 결과 유실 없음).
    Ok(success)
}

/// 반환: 성공 여부(결제 불필요로 콘텐츠 수신 또는 결제·정산 완료). 거부/실패/정산실패면 false.
async fn cmd_fetch(cli: &Cli, rest: &[String]) -> Result<bool, String> {
    let url = rest.first().ok_or_else(|| {
        ts!(
            "URL 이 필요해요.\n\nkura fetch <URL> [--memo \"사유\"]",
            "A URL is required.\n\nkura fetch <URL> [--memo \"reason\"]"
        )
        .to_string()
    })?;
    let memo = cli.opts.get("memo").map(String::as_str);
    // --agent N: 상대의 ERC-8004 번호(선택). 주면 온체인 기록과 대조해 사실 한 줄을 덧붙인다.
    let agent_id = agent_opt(cli)?;

    if !cli.json {
        println!(
            "{}",
            ts!(
                "리소스를 가져오는 중… (결제가 필요하면 지갑 앱 승인을 기다립니다, 최대 5분)",
                "Fetching the resource… (if it needs payment, this waits up to 5 minutes for your approval)"
            )
        );
    }
    let res = flow::run_x402(url, memo, agent_id).await?;
    let (agent, agent_note, out) = (res.agent, res.agent_note, res.outcome);
    // 성공 = 결제 불필요(콘텐츠 받음) 또는 결제·정산까지 완료. 거부/실패/정산실패는 종료코드 1.
    // **신원 조회 결과는 성공 판정에 넣지 않는다** — 사실을 보여줄 뿐 결제를 막지 않는다.
    let success = matches!(
        out,
        X402Outcome::NotPaid { .. } | X402Outcome::Paid { ok: true, .. }
    );

    if cli.json {
        print_json(&x402_json(&out, agent.as_ref(), &agent_note))?;
    } else {
        if let Some(a) = &agent {
            println!("{}", agent_line(a));
        } else if !agent_note.is_empty() {
            println!("{agent_note}");
        }
        match out {
            X402Outcome::NotPaid { http_status, body } => {
                println!(
                    "{}",
                    tf!(
                        "결제 불필요 (HTTP {http_status})\n",
                        "No payment required (HTTP {http_status})\n"
                    )
                );
                println!("{body}");
            }
            X402Outcome::Declined { status, detail } => {
                if status == "rejected" {
                    eprintln!(
                        "{}",
                        ts!(
                            "✗ 사용자가 결제를 거부했어요.",
                            "✗ The user rejected the payment."
                        )
                    );
                } else {
                    eprintln!(
                        "{}",
                        tf!(
                            "✗ 결제 실패: {}",
                            "✗ Payment failed: {}",
                            if detail.is_empty() {
                                ts!("알 수 없는 오류", "unknown error")
                            } else {
                                &detail
                            }
                        )
                    );
                }
            }
            X402Outcome::Paid {
                http_status,
                ok,
                amount,
                pay_to,
                resource,
                settlement,
                body,
            } => {
                if ok {
                    println!(
                        "{}",
                        tf!(
                            "✓ 결제 완료 — {amount} USDC → {}",
                            "✓ Paid — {amount} USDC → {}",
                            shorten(&pay_to)
                        )
                    );
                } else {
                    println!(
                        "{}",
                        tf!(
                            "△ 서명·전송했으나 정산 실패 (HTTP {http_status})",
                            "△ Signed and sent, but settlement failed (HTTP {http_status})"
                        )
                    );
                }
                println!("{}", tf!("  리소스  {resource}", "  resource  {resource}"));
                if !settlement.is_empty() {
                    // settlement 은 base64(ASCII) → 바이트 슬라이스 안전. 앞부분만 미리보기.
                    println!(
                        "{}",
                        tf!(
                            "  정산증빙  {}…",
                            "  receipt   {}…",
                            &settlement[..settlement.len().min(24)]
                        )
                    );
                }
                println!("\n{body}");
            }
        }
    }
    Ok(success)
}

/// X402Outcome → MCP 와 동일한 JSON 형태(스크립트가 두 어댑터를 같게 다루게).
/// ERC-8004 대조를 사람이 읽을 한 줄로 (개발 47). **판정하지 않는다** — 일치/다름/모름만.
fn agent_line(a: &AgentTrust) -> String {
    if !a.registered {
        return tf!(
            "온체인에 없는 에이전트 번호예요 (#{})",
            "No agent #{} exists on-chain",
            a.agent_id
        );
    }
    let w = match a.wallet_check.as_str() {
        "match" => ts!("등록 지갑 일치", "wallet matches"),
        "differs" => ts!("⚠ 등록 지갑과 다름", "⚠ differs from registered wallet"),
        "unset" => ts!("등록 지갑 없음", "no wallet on record"),
        _ => ts!("등록 지갑 모름", "wallet unknown"),
    };
    // 대조할 도메인이 **아예 없는** 경우(직접 송금 — 요청 URL 이라는 게 없다)엔 그 칸을 안 쓴다.
    // "모름"을 찍으면 「알아보려다 실패했다」로 읽히는데, 여기선 알아볼 대상 자체가 없다.
    if a.resource_domain.trim().is_empty() {
        return tf!("에이전트 #{} · {}", "Agent #{} · {}", a.agent_id, w);
    }
    let d = match a.domain_check.as_str() {
        "match" => ts!("기재 도메인 일치", "listed domain matches"),
        "differs" => ts!("⚠ 기재 도메인과 다름", "⚠ differs from listed domain"),
        _ => ts!("기재 도메인 모름", "listed domain unknown"),
    };
    tf!(
        "에이전트 #{} · {} · {}",
        "Agent #{} · {} · {}",
        a.agent_id,
        w,
        d
    )
}

/// `--agent N` 파싱 (pay·fetch 공용). 숫자가 아니면 **조용히 무시하지 않고** 바로 알린다 —
/// 오타가 "조회 안 함"으로 묻히면 사용자는 대조가 된 줄 안다.
fn agent_opt(cli: &Cli) -> Result<Option<u64>, String> {
    match cli.opts.get("agent") {
        Some(v) => Ok(Some(v.trim().parse::<u64>().map_err(|_| {
            tf!(
                "--agent 는 숫자여야 해요: {v:?}",
                "--agent must be a number: {v:?}"
            )
        })?)),
        None => Ok(None),
    }
}

fn x402_json(
    out: &X402Outcome,
    agent: Option<&AgentTrust>,
    agent_note: &str,
) -> serde_json::Value {
    let mut v = x402_outcome_json(out);
    if let Some(a) = agent {
        v["agent"] = serde_json::to_value(a).unwrap_or(serde_json::Value::Null);
    }
    if !agent_note.is_empty() {
        v["agent_lookup_note"] = serde_json::json!(agent_note);
    }
    v
}

fn x402_outcome_json(out: &X402Outcome) -> serde_json::Value {
    match out {
        X402Outcome::NotPaid { http_status, body } => serde_json::json!({
            "paid": false, "status": "ok", "http_status": http_status, "body": body,
        }),
        X402Outcome::Declined { status, detail } => serde_json::json!({
            "paid": false, "status": status, "detail": detail,
        }),
        X402Outcome::Paid {
            http_status,
            ok,
            amount,
            pay_to,
            resource,
            settlement,
            body,
        } => serde_json::json!({
            "paid": true,
            "status": if *ok { "ok" } else { "settlement_failed" },
            "http_status": http_status,
            "amount": amount,
            "asset": "USDC",
            "pay_to": pay_to,
            "resource": resource,
            "settlement": settlement,
            "body": body,
        }),
    }
}

// ── 표시 헬퍼 (순수 함수 — 테스트됨) ────────────────────────────────────────

fn print_json<T: serde::Serialize>(v: &T) -> Result<(), String> {
    let s = serde_json::to_string_pretty(v)
        .map_err(|e| tf!("직렬화 실패: {e}", "Couldn't serialize the output: {e}"))?;
    println!("{s}");
    Ok(())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 활성 체인의 사람용 라벨. 메인넷은 실제 자금임을 강조한다.
fn network_label() -> String {
    match active_chain().chain_id {
        84_532 => ts!("Base Sepolia (테스트넷)", "Base Sepolia (testnet)").into(),
        8453 => ts!("Base 메인넷 · 실제 자금 ⚠️", "Base mainnet · real funds ⚠️").into(),
        5_042_002 => ts!("Arc 테스트넷", "Arc testnet").into(),
        id => tf!("체인 {id}", "chain {id}"),
    }
}

/// 내역 status 코드를 사람 말로.
///
/// `rel_time` 과 같은 이유로 언어를 인자로 받는다 — 테스트가 값으로 검증하는 순수 함수라,
/// 전역 언어를 읽으면 사용자가 앱을 영어로 바꾼 순간 `cargo test` 가 깨진다.
fn status_label(status: &str, lang: Lang) -> &str {
    let (ko, en) = match status {
        "sent" => ("보냄", "sent"),
        "blocked" => ("차단됨", "blocked"),
        "failed" => ("실패", "failed"),
        "signed" => ("서명됨(정산대기)", "signed (awaiting settlement)"),
        "settled" => ("정산됨", "settled"),
        "settle_failed" => ("정산실패", "settlement failed"),
        // 모르는 코드는 그대로 — 새 status 가 생겨도 원문이 보이는 편이 낫다(옛 status_ko 와 같다).
        other => return other,
    };
    match lang {
        Lang::Ko => ko,
        Lang::En => en,
    }
}

/// 사람용 금액 표시 — 소수점 뒤 꼬리 0 을 정리한다("0.000000"→"0", "1.50"→"1.5").
/// JSON 출력엔 쓰지 않는다(MCP 와 동일한 raw 값 유지).
fn trim_zeros(s: &str) -> String {
    if !s.contains('.') {
        return s.to_string();
    }
    let t = s.trim_end_matches('0').trim_end_matches('.');
    if t.is_empty() {
        "0".into()
    } else {
        t.to_string()
    }
}

/// 0x주소/해시를 0x1234…abcd 로 줄인다. 짧으면 그대로, 비었으면 "-".
fn shorten(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return "-".into();
    }
    if s.len() <= 12 {
        return s.to_string();
    }
    format!("{}…{}", &s[..6], &s[s.len() - 4..])
}

/// now-ts 를 사람용 상대 시간으로(달력 변환 불필요 — 타임존 함정 회피). 미래면 "방금".
///
/// 언어를 전역에서 읽지 않고 **인자로 받는다** — 이 함수는 테스트가 값으로 검증하는
/// 순수 함수라, 전역을 보면 테스트 결과가 사용자의 settings.json 에 좌우된다.
fn rel_time(now: u64, ts: u64, lang: Lang) -> String {
    let d = now.saturating_sub(ts);
    match lang {
        Lang::Ko => {
            if d < 60 {
                "방금".into()
            } else if d < 3_600 {
                format!("{}분 전", d / 60)
            } else if d < 86_400 {
                format!("{}시간 전", d / 3_600)
            } else {
                format!("{}일 전", d / 86_400)
            }
        }
        Lang::En => {
            if d < 60 {
                "just now".into()
            } else if d < 3_600 {
                format!("{}m ago", d / 60)
            } else if d < 86_400 {
                format!("{}h ago", d / 3_600)
            } else {
                format!("{}d ago", d / 86_400)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_subcommand_and_positionals() {
        let a = vec!["pay".into(), "0xabc".into(), "1.5".into()];
        match parse(&a) {
            Parsed::Run(c) => {
                assert_eq!(c.positionals, vec!["pay", "0xabc", "1.5"]);
                assert!(!c.json);
            }
            _ => panic!("Run 이어야 함"),
        }
    }

    #[test]
    fn parse_value_opts_both_forms() {
        // --key value 와 --key=value 가 같게 파싱된다.
        let a = vec![
            "pay".into(),
            "0xabc".into(),
            "1".into(),
            "--token".into(),
            "ETH".into(),
            "--memo=커피값".into(),
        ];
        match parse(&a) {
            Parsed::Run(c) => {
                assert_eq!(c.opts.get("token").unwrap(), "ETH");
                assert_eq!(c.opts.get("memo").unwrap(), "커피값");
                assert_eq!(c.positionals, vec!["pay", "0xabc", "1"]);
            }
            _ => panic!("Run 이어야 함"),
        }
    }

    #[test]
    fn parse_json_flag_anywhere() {
        let a = vec!["balance".into(), "--json".into()];
        match parse(&a) {
            Parsed::Run(c) => assert!(c.json),
            _ => panic!("Run"),
        }
    }

    #[test]
    fn parse_help_and_version_short_circuit() {
        assert!(matches!(parse(&["--help".into()]), Parsed::Help));
        assert!(matches!(parse(&["-h".into()]), Parsed::Help));
        assert!(matches!(parse(&["--version".into()]), Parsed::Version));
        assert!(matches!(parse(&["-V".into()]), Parsed::Version));
        // 하위명령보다 우선.
        assert!(matches!(
            parse(&["pay".into(), "--help".into()]),
            Parsed::Help
        ));
    }

    /// `--agent` 는 값을 받는 옵션이다 — VALUE_OPTS 에 없으면 두 형태 모두 "알 수 없는 옵션"
    /// 으로 튕겨서, 사용법에 적어 둔 명령이 통째로 죽는다(코덱스 개발47 1차 P1 — 실제로 죽어 있었다).
    #[test]
    fn parse_agent_option_both_forms() {
        for a in [
            vec!["fetch".into(), "u".into(), "--agent".into(), "7".into()],
            vec!["fetch".into(), "u".into(), "--agent=7".into()],
        ] {
            match parse(&a) {
                Parsed::Run(c) => assert_eq!(c.opts.get("agent").map(String::as_str), Some("7")),
                _ => panic!("Run 이어야 한다: {a:?}"),
            }
        }
    }

    #[test]
    fn parse_unknown_option_errors() {
        assert!(matches!(
            parse(&["status".into(), "--bogus".into()]),
            Parsed::Error(_)
        ));
        assert!(matches!(parse(&["--nope=1".into()]), Parsed::Error(_)));
    }

    #[test]
    fn parse_missing_value_errors() {
        // --token 이 마지막이라 값이 없음.
        assert!(matches!(
            parse(&["pay".into(), "--token".into()]),
            Parsed::Error(_)
        ));
    }

    #[test]
    fn parse_value_opt_rejects_dashed_next() {
        // `--memo --json` 은 --json 을 memo 로 삼키지 않고 값 누락 오류로 본다.
        let a = vec![
            "pay".into(),
            "a".into(),
            "1".into(),
            "--memo".into(),
            "--json".into(),
        ];
        assert!(matches!(parse(&a), Parsed::Error(_)));
        // 의도적 --값은 = 형태로 허용.
        match parse(&["pay".into(), "a".into(), "1".into(), "--memo=--keep".into()]) {
            Parsed::Run(c) => assert_eq!(c.opts.get("memo").unwrap(), "--keep"),
            _ => panic!("Run"),
        }
    }

    #[test]
    fn trim_zeros_amounts() {
        assert_eq!(trim_zeros("0.000000000000000000"), "0");
        assert_eq!(trim_zeros("0.000000"), "0");
        assert_eq!(trim_zeros("19.920000"), "19.92");
        assert_eq!(trim_zeros("1.50"), "1.5");
        assert_eq!(trim_zeros("100"), "100"); // 소수점 없으면 그대로
        assert_eq!(trim_zeros("0"), "0");
    }

    #[test]
    fn shorten_addr() {
        assert_eq!(
            shorten("0x8b7ba5077d261739f5FeBB31B10167671e590161"),
            "0x8b7b…0161"
        );
        assert_eq!(shorten(""), "-");
        assert_eq!(shorten("short"), "short");
    }

    #[test]
    fn rel_time_buckets() {
        assert_eq!(rel_time(1000, 1000, Lang::Ko), "방금");
        assert_eq!(rel_time(1000, 970, Lang::Ko), "방금"); // 30초
        assert_eq!(rel_time(1000, 880, Lang::Ko), "2분 전"); // 120초
        assert_eq!(rel_time(10_000, 2_800, Lang::Ko), "2시간 전"); // 7200초
        assert_eq!(rel_time(200_000, 27_200, Lang::Ko), "2일 전"); // 172800초
        assert_eq!(rel_time(1000, 2000, Lang::Ko), "방금"); // 미래(시계 역전)도 방금
                                                            // 영어도 같은 경계로 갈린다 (개발 42).
        assert_eq!(rel_time(1000, 1000, Lang::En), "just now");
        assert_eq!(rel_time(1000, 880, Lang::En), "2m ago");
        assert_eq!(rel_time(10_000, 2_800, Lang::En), "2h ago");
        assert_eq!(rel_time(200_000, 27_200, Lang::En), "2d ago");
    }

    #[test]
    fn status_ko_maps_known_and_passthrough() {
        assert_eq!(status_label("sent", Lang::Ko), "보냄");
        assert_eq!(status_label("settled", Lang::Ko), "정산됨");
        assert_eq!(status_label("sent", Lang::En), "sent");
        assert_eq!(status_label("settle_failed", Lang::En), "settlement failed");
        // 모르는 코드는 그대로 통과한다(언어와 무관).
        assert_eq!(status_label("weird", Lang::Ko), "weird");
        assert_eq!(status_label("weird", Lang::En), "weird");
    }
}
