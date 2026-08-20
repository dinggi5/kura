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
use kura_mcp::flow::{self, X402Outcome};
use kura_mcp::wallet;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
Kura — AI 에이전트 전용 로컬 지갑 CLI

사용법:
  kura status                  지갑 상태와 주소
  kura balance                 ETH(가스) · USDC(결제) 잔액
  kura history [--limit N]     최근 거래 내역 (기본 20, 최대 200)
  kura pay <주소> <금액> [옵션]    결제(송금) 요청 → 지갑 앱에서 비번 승인
       --token USDC|ETH        토큰 (기본 USDC)
       --memo \"사유\"            승인 팝업에 보일 결제 사유
  kura fetch <URL> [--memo \"사유\"]   x402 유료 리소스를 결제하고 가져온다

전역 옵션:
  --json        기계가 읽는 JSON 으로 출력 (스크립트용)
  -h, --help    이 도움말
  -V, --version 버전

보안: 비밀번호는 절대 CLI 로 받지 않습니다. 결제는 기본값으로 지갑 앱이 팝업으로 사람 승인을
받아야 실행되고(최대 5분 대기; 앱에서 자율 결제를 켠 경우만 그 한도 안에서 자동 승인),
단일/일일 한도·긴급잠금·화이트리스트는 앱이 강제합니다.";

/// 파싱된 명령줄 — 전역 플래그 + 위치 인자 + 값 옵션.
struct Cli {
    json: bool,
    positionals: Vec<String>,
    opts: HashMap<String, String>,
}

/// 값을 받는 옵션들(`--key value` 또는 `--key=value`). 나머지 `--flag` 는 불리언.
const VALUE_OPTS: [&str; 3] = ["token", "memo", "limit"];

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
                        return Parsed::Error(format!("알 수 없는 옵션: --{k}"));
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
                            return Parsed::Error(format!(
                                "--{body} 에 값이 필요해요 (값이 --로 시작하면 --{body}=값 형태로 주세요)"
                            ))
                        }
                    }
                } else {
                    return Parsed::Error(format!("알 수 없는 옵션: {a}"));
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
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Parsed::Version => {
            println!("kura {VERSION}");
            return ExitCode::SUCCESS;
        }
        Parsed::Error(msg) => {
            eprintln!("{msg}\n\n{HELP}");
            return ExitCode::FAILURE;
        }
        Parsed::Run(cli) => cli,
    };

    let cmd = cli.positionals.first().map(String::as_str).unwrap_or("");
    if cmd.is_empty() {
        println!("{HELP}");
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
        other => Err(format!("알 수 없는 명령: {other}\n\n{HELP}")),
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
        "encrypted" => "암호화됨 ✓",
        "legacy" => "평문 (앱에서 비번 설정 필요)",
        _ => "없음 (앱에서 지갑을 먼저 만드세요)",
    };
    println!("지갑      {state}");
    if let Some(addr) = &s.address {
        println!("주소      {addr}");
        println!("백업      {}", if s.backed_up { "완료" } else { "안 됨 — 시드 백업 권장" });
    }
    println!("네트워크  {}", network_label());
    Ok(())
}

async fn cmd_balance(cli: &Cli) -> Result<(), String> {
    let s = wallet::wallet_status()?;
    let addr = s.address.ok_or("지갑이 아직 없어요. 앱에서 먼저 만드세요.")?;
    let b = wallet::get_balances(&addr).await?;
    if cli.json {
        print_json(&b)?;
        return Ok(());
    }
    println!("USDC  {}  (결제용)", trim_zeros(&b.usdc));
    println!("ETH   {}  (가스용)", trim_zeros(&b.eth));
    println!("네트워크  {}", network_label());
    Ok(())
}

async fn cmd_history(cli: &Cli) -> Result<(), String> {
    // 저장소가 최대 200건만 보관 → help 와 일치하게 200 으로 클램프(그 이상은 보여줄 게 없다).
    let limit = match cli.opts.get("limit") {
        Some(v) => v
            .parse::<usize>()
            .map_err(|_| format!("--limit 은 0 이상의 정수여야 해요: {v}"))?
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
        println!("아직 거래 내역이 없어요.");
        return Ok(());
    }
    let now = now_secs();
    for e in &list {
        // 정산된 x402 는 정산 tx, 그 외는 detail(보통 tx 해시)을 증빙으로 보여준다.
        let hash = if !e.settle_tx.is_empty() { &e.settle_tx } else { &e.detail };
        let mut line = format!(
            "{}  ·  {} {}  ·  {}  ·  → {}",
            status_ko(&e.status),
            e.token,
            e.amount,
            rel_time(now, e.ts),
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
    let to = rest
        .first()
        .ok_or("받는 주소가 필요해요.\n\nkura pay <주소> <금액> [--token USDC|ETH] [--memo \"사유\"]")?;
    let amount = rest
        .get(1)
        .ok_or("금액이 필요해요.\n\nkura pay <주소> <금액> [--token USDC|ETH] [--memo \"사유\"]")?;
    let token = cli.opts.get("token").map(String::as_str).unwrap_or("USDC");
    let memo = cli.opts.get("memo").map(String::as_str).unwrap_or("");

    if !cli.json {
        println!("지갑 앱 승인을 기다리는 중… (최대 5분, 앱 팝업에서 비번 입력)");
    }
    let out = flow::run_payment(token, to, amount, memo).await?;
    let success = out.status == "approved";

    if cli.json {
        print_json(&serde_json::json!({
            "status": out.status,
            "tx_hash": out.tx_hash,
            "detail": out.detail,
            "explorer": out.explorer,
        }))?;
    } else {
        match out.status.as_str() {
            "approved" => {
                println!("✓ 승인됨");
                if !out.tx_hash.is_empty() {
                    println!("  tx    {}", out.tx_hash);
                }
                if !out.explorer.is_empty() {
                    println!("  보기  {}", out.explorer);
                }
            }
            "rejected" => println!("✗ 사용자가 거부했어요."),
            _ => eprintln!(
                "✗ 실패: {}",
                if out.detail.is_empty() { "알 수 없는 오류" } else { &out.detail }
            ),
        }
    }
    // 거부/실패는 종료코드 1(json/사람 모드 동일). process::exit 대신 bool 을 올려 main 이 정상
    // 반환하게 한다 — stdout 이 flush 되도록(파이프에서도 결과 유실 없음).
    Ok(success)
}

/// 반환: 성공 여부(결제 불필요로 콘텐츠 수신 또는 결제·정산 완료). 거부/실패/정산실패면 false.
async fn cmd_fetch(cli: &Cli, rest: &[String]) -> Result<bool, String> {
    let url = rest
        .first()
        .ok_or("URL 이 필요해요.\n\nkura fetch <URL> [--memo \"사유\"]")?;
    let memo = cli.opts.get("memo").map(String::as_str);

    if !cli.json {
        println!("리소스를 가져오는 중… (결제가 필요하면 지갑 앱 승인을 기다립니다, 최대 5분)");
    }
    let out = flow::run_x402(url, memo).await?;
    // 성공 = 결제 불필요(콘텐츠 받음) 또는 결제·정산까지 완료. 거부/실패/정산실패는 종료코드 1.
    let success = matches!(
        out,
        X402Outcome::NotPaid { .. } | X402Outcome::Paid { ok: true, .. }
    );

    if cli.json {
        print_json(&x402_json(&out))?;
    } else {
        match out {
            X402Outcome::NotPaid { http_status, body } => {
                println!("결제 불필요 (HTTP {http_status})\n");
                println!("{body}");
            }
            X402Outcome::Declined { status, detail } => {
                if status == "rejected" {
                    eprintln!("✗ 사용자가 결제를 거부했어요.");
                } else {
                    eprintln!(
                        "✗ 결제 실패: {}",
                        if detail.is_empty() { "알 수 없는 오류" } else { &detail }
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
                    println!("✓ 결제 완료 — {amount} USDC → {}", shorten(&pay_to));
                } else {
                    println!("△ 서명·전송했으나 정산 실패 (HTTP {http_status})");
                }
                println!("  리소스  {resource}");
                if !settlement.is_empty() {
                    // settlement 은 base64(ASCII) → 바이트 슬라이스 안전. 앞부분만 미리보기.
                    println!("  정산증빙  {}…", &settlement[..settlement.len().min(24)]);
                }
                println!("\n{body}");
            }
        }
    }
    Ok(success)
}

/// X402Outcome → MCP 와 동일한 JSON 형태(스크립트가 두 어댑터를 같게 다루게).
fn x402_json(out: &X402Outcome) -> serde_json::Value {
    match out {
        X402Outcome::NotPaid { http_status, body } => serde_json::json!({
            "paid": false, "status": "ok", "http_status": http_status, "body": body,
        }),
        X402Outcome::Declined { status, detail } => serde_json::json!({
            "paid": false, "status": status, "detail": detail,
        }),
        X402Outcome::Paid {
            http_status, ok, amount, pay_to, resource, settlement, body,
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
    let s = serde_json::to_string_pretty(v).map_err(|e| format!("직렬화 실패: {e}"))?;
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
        84_532 => "Base Sepolia (테스트넷)".into(),
        8453 => "Base 메인넷 · 실제 자금 ⚠️".into(),
        id => format!("체인 {id}"),
    }
}

/// 내역 status 코드를 한글로.
fn status_ko(status: &str) -> &str {
    match status {
        "sent" => "보냄",
        "blocked" => "차단됨",
        "failed" => "실패",
        "signed" => "서명됨(정산대기)",
        "settled" => "정산됨",
        "settle_failed" => "정산실패",
        other => other,
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
fn rel_time(now: u64, ts: u64) -> String {
    let d = now.saturating_sub(ts);
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
        assert!(matches!(parse(&["pay".into(), "--help".into()]), Parsed::Help));
    }

    #[test]
    fn parse_unknown_option_errors() {
        assert!(matches!(parse(&["status".into(), "--bogus".into()]), Parsed::Error(_)));
        assert!(matches!(parse(&["--nope=1".into()]), Parsed::Error(_)));
    }

    #[test]
    fn parse_missing_value_errors() {
        // --token 이 마지막이라 값이 없음.
        assert!(matches!(parse(&["pay".into(), "--token".into()]), Parsed::Error(_)));
    }

    #[test]
    fn parse_value_opt_rejects_dashed_next() {
        // `--memo --json` 은 --json 을 memo 로 삼키지 않고 값 누락 오류로 본다.
        let a = vec!["pay".into(), "a".into(), "1".into(), "--memo".into(), "--json".into()];
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
        assert_eq!(shorten("0x8b7ba5077d261739f5FeBB31B10167671e590161"), "0x8b7b…0161");
        assert_eq!(shorten(""), "-");
        assert_eq!(shorten("short"), "short");
    }

    #[test]
    fn rel_time_buckets() {
        assert_eq!(rel_time(1000, 1000), "방금");
        assert_eq!(rel_time(1000, 970), "방금"); // 30초
        assert_eq!(rel_time(1000, 880), "2분 전"); // 120초
        assert_eq!(rel_time(10_000, 2_800), "2시간 전"); // 7200초
        assert_eq!(rel_time(200_000, 27_200), "2일 전"); // 172800초
        assert_eq!(rel_time(1000, 2000), "방금"); // 미래(시계 역전)도 방금
    }

    #[test]
    fn status_ko_maps_known_and_passthrough() {
        assert_eq!(status_ko("sent"), "보냄");
        assert_eq!(status_ko("settled"), "정산됨");
        assert_eq!(status_ko("weird"), "weird");
    }
}
