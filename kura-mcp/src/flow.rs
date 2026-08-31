// 어댑터 공통 결제 흐름 (개발 22) — MCP(main.rs)와 CLI(bin/kura.rs)가 같은 로직을 공유한다.
//
// 결제·x402 의 보안 민감한 부분(리다이렉트 가드·single-flight·정산 게이팅·승인 대기)을 여기 한 곳에
// 모은다. 두 어댑터가 각자 구현하면 한쪽만 고쳐져 보안이 갈라질 위험이 있다 → 하나의 진실.
//
// 비번은 절대 여기로 들어오지 않는다. 이 모듈은 GUI 에 "요청"만 하고, 서명·전송은 GUI 가
// 사람 승인을 받아 수행한다(payment.rs 의 파일 IPC). 한도·긴급잠금·화이트리스트도 GUI 가 강제한다.

use crate::chain::active_chain;
use crate::erc8004::{self, AgentTrust};
use crate::{payment, x402};
use crate::{tf, ts};

/// 외부 서버가 거대 본문으로 어댑터 메모리를 채우지 못하게 하는 상한(바이트). 본문을 읽기 전에
/// Content-Length 로 먼저 거른다(버퍼링 후 자르면 메모리 보호가 안 됨).
const MAX_BODY_BYTES: u64 = 4 * 1024 * 1024; // 4 MiB

/// 표시용 본문 문자 상한 — 에이전트/터미널 컨텍스트 보호. 문자 단위라 UTF-8 안전.
const MAX_BODY_CHARS: usize = 100_000;

/// 본문이 너무 길면 잘라서 돌려준다.
fn cap_body(body: String) -> String {
    if body.chars().count() <= MAX_BODY_CHARS {
        return body;
    }
    let mut s: String = body.chars().take(MAX_BODY_CHARS).collect();
    s.push_str(ts!(
        "\n…(본문이 길어 잘렸어요)",
        "\n…(body truncated — it was long)"
    ));
    s
}

/// 응답 본문을 **스트리밍**으로 읽되 누적 바이트가 상한을 넘으면 즉시 중단한다. Content-Length 가
/// 없는(chunked) 응답도 전량 버퍼링하지 않으므로 악성 서버의 메모리 폭주(DoS)를 막는다.
/// 반환: (읽은 바이트, 상한 초과로 잘렸는지). 헤더는 호출자가 먼저 챙긴 뒤 호출한다.
async fn read_limited(mut resp: reqwest::Response) -> Result<(Vec<u8>, bool), String> {
    let cap = MAX_BODY_BYTES as usize;
    let mut buf: Vec<u8> = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| tf!("본문 읽기 실패: {e}", "Couldn't read the body: {e}"))?
    {
        if buf.len() + chunk.len() > cap {
            buf.extend_from_slice(&chunk[..cap - buf.len()]);
            truncated = true;
            break;
        }
        buf.extend_from_slice(&chunk);
    }
    Ok((buf, truncated))
}

/// 응답 본문을 메모리 안전하게 읽어 표시용으로 캡한다(바이트 상한 스트리밍 + 문자 상한).
async fn read_body_capped(resp: reqwest::Response) -> String {
    let (bytes, truncated) = match read_limited(resp).await {
        Ok(v) => v,
        Err(e) => return format!("({e})"),
    };
    let mut s = cap_body(String::from_utf8_lossy(&bytes).into_owned());
    if truncated {
        s.push_str(ts!(
            "\n…(본문이 너무 커서 잘렸어요)",
            "\n…(body truncated — it was too large)"
        ));
    }
    s
}

/// 최초 probe GET 용 HTTP 클라이언트 — 리다이렉트를 따라간다(결제 헤더가 없어 유출 위험 없음 —
/// "리다이렉트 후 402" 같은 정상 케이스를 깨지 않게). 요청당 30초 타임아웃.
fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| {
            tf!(
                "HTTP 클라이언트 생성 실패: {e}",
                "Couldn't create the HTTP client: {e}"
            )
        })
}

/// 결제 재요청 전용 — **리다이렉트를 따라가지 않는다**(`Policy::none`). PAYMENT-SIGNATURE/X-PAYMENT
/// (서명된 EIP-3009 인가)는 reqwest 의 cross-host 민감 헤더 제거 대상이 아니라, 리다이렉트를 따라가면
/// 다른 origin 으로 서명이 유출될 수 있다 → 402 를 낸 바로 그 최종 URL 에만 결제 헤더를 보낸다.
fn http_client_no_redirect() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| {
            tf!(
                "HTTP 클라이언트 생성 실패: {e}",
                "Couldn't create the HTTP client: {e}"
            )
        })
}

/// 결제(송금) 요청 결과 — 사람 승인 팝업에서 사용자가 응답한 결과.
pub struct PayOutcome {
    /// approved | rejected | failed.
    pub status: String,
    pub tx_hash: String,
    pub detail: String,
    /// tx 해시가 있을 때의 익스플로러 링크(없으면 빈 문자열).
    pub explorer: String,
    /// ERC-8004 대조 결과 (번호를 준 경우에만). 승인 창에도 같은 값이 붙는다.
    pub agent: Option<AgentTrust>,
    /// 조회를 못 한 이유(꺼짐·읽기 실패). 빈 문자열 = 할 말 없음.
    pub agent_note: String,
}

/// 승인 창을 띄울 수 없을 때의 안내. **하트비트가 없는 이유는 하나가 아니다.**
///
/// GUI 는 지갑이 아직 없으면(첫 실행·평문 마이그레이션 대기) 하트비트를 **일부러 안 찍는다**
/// (`src-tauri/src/ipc.rs` watchdog: `!needs_setup()` 일 때만 찍는다 — 승인할 사람이 없는데
/// 살아 있다고 하면 MCP 가 요청을 두고 5분을 조용히 기다린다). 그래서 `app_alive()==false` 는
/// 「앱이 꺼짐」이거나 「앱은 켜져 있는데 지갑이 없음」이다. 둘을 한 문구로 뭉치면 사용자가
/// **이미 켜져 있는 앱을 다시 켜려 한다**(개발 49 이월). 지갑 파일 상태로 갈라서 안내한다.
fn app_unavailable() -> String {
    // 앱은 떠 있는데 화면만 죽은 경우 — 켜라고 하면 안 된다(이미 켜져 있다). 다시 시작해야 한다.
    if payment::ui_stalled() {
        return ts!(
            "지갑 앱 화면이 응답하지 않아요. 앱을 완전히 종료했다 다시 켠 뒤 시도하세요.",
            "The wallet app's window isn't responding. Quit the app completely, open it again, then try."
        )
        .to_string();
    }
    match crate::wallet::wallet_status().map(|s| s.state) {
        Ok(s) if s == "none" => ts!(
            "지갑을 아직 만들지 않았어요. 지갑 앱에서 지갑을 만든 뒤 다시 시도하세요.",
            "The wallet hasn't been created yet. Create one in the wallet app, then try again."
        )
        .to_string(),
        Ok(s) if s == "legacy" => ts!(
            "지갑에 비밀번호가 아직 없어요. 지갑 앱에서 비밀번호 설정을 끝낸 뒤 다시 시도하세요.",
            "The wallet isn't password-protected yet. Finish setting a password in the wallet app, then try again."
        )
        .to_string(),
        // 지갑은 있는데 하트비트가 없다 = 앱이 꺼져 있다. 상태를 못 읽는 경우도 여기로 —
        // 「앱을 켜라」가 둘 다에 맞는 안내다(지갑 파일은 앱이 만든다).
        _ => ts!(
            "지갑 앱이 실행 중이 아니에요. 앱을 켠 뒤 다시 시도하세요.",
            "The wallet app isn't running. Open it and try again."
        )
        .to_string(),
    }
}

/// 결제(송금)를 사용자에게 요청한다. 앱이 켜져 있고 대기 중인 요청이 없어야 한다.
/// 토큰 검증 → 앱 생존/single-flight 확인 → 요청 파일 작성 → 최대 5분 승인 대기.
///
/// `Err` = 요청 자체를 띄울 수 없음(잘못된 토큰·앱 꺼짐·이미 대기 중·시간 초과).
/// `Ok`  = 사용자가 응답함(status 가 approved/rejected/failed 중 하나).
///
/// `on_pending` 은 **요청 파일을 실제로 쓴 직후** 한 번 불린다 — 여기서부터가 진짜 대기 구간이다.
/// CLI 가 「승인을 기다리는 중…」을 이 콜백에서 찍는다: 예전엔 호출 **전에** 찍어서, 토큰 오류나
/// 앱 꺼짐으로 요청이 나가지도 않았는데 기다린다고 말했다(개발 50 이월). 검사를 CLI 에 복사하면
/// 두 벌이 갈라지므로, 판단은 여기 한 곳에 두고 **시점만** 넘긴다.
pub async fn run_payment(
    token: &str,
    to: &str,
    amount: &str,
    memo: &str,
    agent_id: Option<u64>,
    on_pending: impl FnOnce(),
) -> Result<PayOutcome, String> {
    let token = token.trim().to_uppercase();
    if token != "USDC" && token != "ETH" {
        return Err(ts!(
            "token은 USDC 또는 ETH 여야 합니다",
            "token must be either USDC or ETH"
        )
        .into());
    }
    // 네이티브가 곧 USDC 인 체인(Arc)엔 "ETH 송금"이라는 게 없다 — 네이티브 전송은 같은 USDC 를
    // 18dp 로 보내는 것이라 한도·장부·내역(전부 6dp 기준)과 어긋난다. 승인 창을 띄우기 전에 막는다
    // (앱도 같은 걸 막지만, 여기서 걸러야 사람에게 창이 안 뜨고 AI 가 이유를 바로 읽는다).
    if token == "ETH" && crate::chain::active_chain().native_is_usdc {
        return Err(ts!(
            "이 체인은 가스도 USDC로 내요. ETH 송금은 없으니 token은 USDC로 요청하세요.",
            "On this chain gas is paid in USDC, so there is no ETH to send — request token USDC instead."
        )
        .into());
    }
    // 앱이 안 켜져 있으면 승인할 사람이 없다 → 즉시 안내(5분 대기 안 함).
    if !payment::app_alive() {
        return Err(app_unavailable());
    }
    // single-flight: 이미 대기 중인 요청이 있으면 거절.
    if payment::has_pending() {
        return Err(ts!(
            "이미 승인 대기 중인 결제가 있어요. 먼저 처리한 뒤 다시 요청하세요.",
            "A payment is already waiting for approval. Let the user handle it, then ask again."
        )
        .into());
    }

    // ERC-8004 대조 (개발 51) — AI 가 번호를 준 경우에만. x402 와 달리 **도메인 앵커가 없다**
    // (요청 URL 이라는 게 없으니 domain_check 는 늘 "unknown"). 남는 건 「받는 주소 ↔ 등록 지갑」
    // 하나뿐인데, 그 값은 **비대칭**이다:
    //   · 일치 = 안전의 증거가 아니다(등록은 무허가라 공격자도 자기 주소로 등록한다) → 승인 창은
    //     회색 꼬리말 한 마디로만 붙인다. 개발 47 이 「신호 가치가 약하다」고 안 붙인 이유가 이것.
    //   · 불일치 = 값이 있다. AI 가 「에이전트 42에게」라고 해 놓고 주소가 42의 등록 지갑과 다르면
    //     **주소 바꿔치기 정황**이다 → 호박색 경고 + `agent_contradicts` 가 자율 승인도 막는다.
    // 그래서 「일치를 자랑하려고」가 아니라 **「불일치를 잡으려고」** 붙인다.
    // 여기서 실패해도 결제는 계속된다 — 조회는 판단 재료 하나이지 결제의 전제가 아니다.
    let mut agent_note = String::new();
    let agent = match agent_id {
        None => None,
        Some(_) if !erc8004::lookup_enabled() => {
            agent_note = ts!(
                "에이전트 신원 조회가 설정에서 꺼져 있어요.",
                "Agent identity lookup is turned off in the wallet settings."
            )
            .to_string();
            None
        }
        // 대조 상대는 받는 주소뿐 — 리소스 URL 이 없으므로 빈 문자열을 넘긴다(도메인 = 모름).
        Some(id) => match erc8004::lookup(id).await {
            Ok(rec) => Some(erc8004::trust_from(&rec, to.trim(), "")),
            Err(e) => {
                agent_note = e;
                None
            }
        },
    };

    // 조회가 최대 10초를 먹을 수 있어 위 확인이 낡았을 수 있다 → 쓰기 직전에 한 번 더 본다
    // (x402 경로와 같은 이유 — 승인할 UI 가 없는데 요청만 남기면 5분을 헛기다린다).
    if agent_id.is_some() && !payment::app_alive() {
        return Err(app_unavailable());
    }
    let id = payment::write_request_agent(
        &token,
        to.trim(),
        amount.trim(),
        memo.trim(),
        agent.clone(),
    )?;
    on_pending();

    match payment::await_result(&id, payment::APPROVAL_TIMEOUT).await {
        Some(r) => {
            let explorer = if r.tx_hash.is_empty() {
                String::new()
            } else {
                format!("{}{}", active_chain().explorer_tx_prefix, r.tx_hash)
            };
            Ok(PayOutcome {
                status: r.status,
                tx_hash: r.tx_hash,
                detail: r.detail,
                explorer,
                agent,
                agent_note,
            })
        }
        None => {
            payment::cancel_request(&id);
            Err(ts!(
                "승인 시간 초과(5분). 사용자가 응답하지 않았어요.",
                "Approval timed out after 5 minutes — the user didn't respond."
            )
            .into())
        }
    }
}

/// x402 리소스 가져오기 결과.
pub enum X402Outcome {
    /// 402 가 아니었다 — 결제 불필요, 본문 그대로.
    NotPaid { http_status: u16, body: String },
    /// 결제가 필요했지만 사용자가 승인하지 않았다(rejected | failed).
    Declined { status: String, detail: String },
    /// 결제하고 콘텐츠를 받았다.
    Paid {
        http_status: u16,
        /// 2xx(정산 성공) 여부. false 면 결제 헤더는 보냈으나 정산 단계에서 실패.
        ok: bool,
        amount: String,
        pay_to: String,
        resource: String,
        /// 정산 증빙 헤더(base64). 없으면 빈 문자열.
        settlement: String,
        body: String,
    },
}

/// x402 실행 결과 + (요청했다면) ERC-8004 대조 결과.
///
/// 대조를 결과에 함께 싣는 이유: 승인 창의 사람과 **AI 가 같은 사실을 본다**. AI 는 이걸 읽고
/// "주소가 기재와 다르니 결제를 접겠다"는 판단을 스스로 할 수 있고, 사람은 창에서 같은 줄을 본다.
pub struct X402Result {
    pub outcome: X402Outcome,
    /// 온체인 대조 결과. 번호를 안 줬거나 조회를 못 했으면 None.
    pub agent: Option<AgentTrust>,
    /// 조회를 못 한 이유(꺼짐·레지스트리 없는 체인·RPC 실패). **결제 흐름은 막지 않는다** —
    /// 신원 조회 실패로 결제가 죽으면, 조회를 켠 순간 앱이 더 잘 깨지는 셈이 된다.
    pub agent_note: String,
}

/// x402 유료 리소스를 가져온다. GET → 402 면 결제 요구 파싱 → GUI 승인(서명) → 결제 헤더로 재요청.
///
/// `memo` = 호출자가 준 결제 사유(없으면 서버 설명으로 폴백). `agent_id` = AI 가 알아낸 상대의
/// ERC-8004 에이전트 번호(선택) — 주면 온체인 기록과 대조해 승인 창에 사실 한 줄이 붙는다.
/// `Err` = 진행 불가(요청/파싱 실패·앱 꺼짐·대기 중·시간 초과·서명 누락·재요청 실패).
pub async fn run_x402(
    url: &str,
    memo: Option<&str>,
    agent_id: Option<u64>,
) -> Result<X402Result, String> {
    let url = url.trim().to_string();
    let client = http_client()?;

    // 1) 먼저 그냥 가져와 본다. 402가 아니면 결제 불필요 → 본문 그대로 반환.
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| tf!("요청 실패: {e}", "The request failed: {e}"))?;
    let http_status = resp.status().as_u16();
    if http_status != 402 {
        let body = read_body_capped(resp).await;
        // 결제가 없으면 대조할 결제도 없다 — 조회를 아예 하지 않는다(불필요한 RPC 0).
        return Ok(X402Result {
            outcome: X402Outcome::NotPaid { http_status, body },
            agent: None,
            agent_note: String::new(),
        });
    }

    // probe 가 리다이렉트를 따라갔을 수 있으니 402 를 실제로 낸 "최종" URL 을 잡아둔다 —
    // 결제 재요청(서명 헤더 포함)은 이 최종 URL 에만 보낸다(원본 URL 로 보내면 다시 리다이렉트).
    let final_url = resp.url().clone();

    // 2) 결제 요구 파싱 → 우리가 처리 가능한 요구 선택.
    // V2는 요구가 `payment-required` 헤더(base64)에, V1은 본문에 온다. 본문을 소비하기 전에 헤더 먼저.
    let pr_header = resp
        .headers()
        .get("payment-required")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    // 402 본문도 스트리밍 상한으로 읽는다(Content-Length 없는 거대 응답 차단). 잘렸으면 = 비정상.
    let (body402_bytes, truncated) = read_limited(resp).await?;
    if truncated {
        return Err(ts!(
            "402 응답 본문이 너무 큽니다",
            "The 402 response body is too large"
        )
        .into());
    }
    let body402 = String::from_utf8_lossy(&body402_bytes).into_owned();
    let required = x402::parse_required(pr_header.as_deref(), &body402).map_err(|e| {
        tf!(
            "402 형식 파싱 실패: {e}",
            "Couldn't parse the 402 response: {e}"
        )
    })?;
    let req = x402::pick_requirement(&required)?;
    let amount_usdc = x402::base_units_to_usdc(&req.amount)?;
    // 🔴 승인 창·내역·알림에 보이는 리소스 URL은 **우리가 실제로 요청한 최종 URL**이다 (개발 47 이월).
    // 예전엔 402 응답이 주장한 `resource` 문자열을 우선 썼다 — 그러면 evil.example 이
    // `resource: "https://api.trusted.io/x"` 라고 적어 두는 것만으로 **사람이 보는 창에 신뢰
    // 도메인이 뜬다**(비번을 넣는 판단 근거가 공격자가 쓴 문자열이 된다). 개발 47 은 ERC-8004
    // 도메인 대조만 final_url 로 옮겼고 **표시는 주장값 그대로 남아** 있었다 — 같은 창에서 위쪽
    // URL(주장)과 아래쪽 경고의 「실제 요청 주소」(final_url)가 서로 다를 수 있었다.
    // 서버의 주장은 프로토콜 제출(V2 raw 에코)에만 쓰고, 사람에게는 보여주지 않는다.
    let resource = final_url.to_string();
    // 팝업에 보일 사유: 호출자 memo > 서버 description(V1 요구별/V2 최상위) > 빈 값.
    let memo = memo
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| {
            let d = required.description(&req);
            (!d.is_empty()).then_some(d)
        })
        .unwrap_or_default();

    // 3) 승인할 앱이 켜져 있는지 + single-flight. **조회보다 먼저** 본다 — 어차피 못 띄울
    // 요청이면 레지스트리를 4번 읽어 봐야 결과를 버릴 뿐이고, 느린 RPC 만큼 즉시 줘야 할
    // 안내가 늦어진다(코덱스 개발47 1차 P2).
    if !payment::app_alive() {
        return Err(app_unavailable());
    }
    if payment::has_pending() {
        return Err(ts!(
            "이미 승인 대기 중인 결제가 있어요. 먼저 처리한 뒤 다시 요청하세요.",
            "A payment is already waiting for approval. Let the user handle it, then ask again."
        )
        .into());
    }

    // 4) ERC-8004 대조 (개발 47) — AI 가 번호를 준 경우에만. **여기서 실패해도 결제는 계속된다**:
    // 조회는 판단 재료를 하나 더 얹는 일이지 결제의 전제조건이 아니다. 못 읽으면 줄이 안 붙고,
    // 사용자는 예전과 똑같은 승인 창을 본다.
    let mut agent_note = String::new();
    let agent = match agent_id {
        None => None,
        Some(_) if !erc8004::lookup_enabled() => {
            agent_note = ts!(
                "에이전트 신원 조회가 설정에서 꺼져 있어요.",
                "Agent identity lookup is turned off in the wallet settings."
            )
            .to_string();
            None
        }
        // 🔴 대조 상대는 서버가 주장한 `resource` 가 **아니라** `final_url` 이다(코덱스 개발47 2차 P1).
        // 공격자의 주장을 레지스트리와 맞춰 보면 검사 의미가 사라진다. 우리가 서명한 결제 헤더를
        // 실제로 받는 곳이 final_url 이다. (개발 51 부터는 표시용 `resource` 도 같은 값이다.)
        Some(id) => match erc8004::lookup(id).await {
            Ok(rec) => Some(erc8004::trust_from(
                &rec,
                req.pay_to.trim(),
                final_url.as_str(),
            )),
            Err(e) => {
                agent_note = e;
                None
            }
        },
    };

    // 5) GUI에 서명 요청 → 사람 승인 → 서명 페이로드 수신.
    // 조회가 최대 10초를 먹을 수 있어 3)의 확인이 낡았을 수 있다 → 쓰기 직전에 한 번 더 본다
    // (코덱스 개발47 2차 P2). 승인할 UI 가 없는데 요청만 남기면 5분을 헛기다린다.
    if !payment::app_alive() {
        return Err(app_unavailable());
    }
    let id = payment::write_x402_request(
        req.pay_to.trim(),
        &amount_usdc,
        &memo,
        &resource,
        agent.clone(),
    )?;
    let result = match payment::await_result(&id, payment::APPROVAL_TIMEOUT).await {
        Some(r) => r,
        None => {
            payment::cancel_request(&id);
            return Err(ts!(
                "승인 시간 초과(5분). 사용자가 응답하지 않았어요.",
                "Approval timed out after 5 minutes — the user didn't respond."
            )
            .into());
        }
    };
    if result.status != "approved" {
        return Ok(X402Result {
            outcome: X402Outcome::Declined {
                status: result.status, // rejected | failed
                detail: result.detail,
            },
            agent,
            agent_note,
        });
    }
    let payment = result.x402.ok_or(ts!(
        "승인됐지만 서명 페이로드가 비어 있어요",
        "Approved, but the signature payload is empty"
    ))?;

    // 6) 결제 헤더를 붙여 재요청 → 콘텐츠 수신.
    // V2면 PAYMENT-SIGNATURE(+ resource/accepted 에코), V1이면 X-PAYMENT. 정산 응답 헤더 이름도 버전별.
    let sub = required.build_submission(&req, &payment)?;
    // 결제 헤더(서명된 인가)는 리다이렉트를 따라가지 않는 클라이언트로 최종 URL 에만 보낸다.
    let pay_client = http_client_no_redirect()?;
    let paid_resp = pay_client
        .get(final_url)
        .header(sub.header_name, &sub.value)
        .send()
        .await
        .map_err(|e| tf!("결제 재요청 실패: {e}", "The paid re-request failed: {e}"))?;
    let paid_status = paid_resp.status().as_u16();
    let settlement = paid_resp
        .headers()
        .get(sub.response_header)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .unwrap_or_default();
    let body = read_body_capped(paid_resp).await;

    // 정산 tx 해시를 뽑아 GUI 내역에 반영되도록 기록(nonce 로 "signed" 항목과 매칭).
    // **2xx(실제 정산 성공) 응답일 때만** 기록한다 — 비-2xx 응답에 위조 PAYMENT-RESPONSE 헤더를
    // 실어 보내 GUI 내역을 가짜 "정산됨"으로 오염시키는 걸 막는다.
    let ok = (200..300).contains(&paid_status);
    if ok && !settlement.is_empty() {
        if let Some((tx, success)) = x402::parse_settlement(&settlement) {
            let _ = payment::record_settlement(&payment.authorization.nonce, &tx, success);
        }
    }

    Ok(X402Result {
        outcome: X402Outcome::Paid {
            http_status: paid_status,
            ok,
            amount: amount_usdc,
            pay_to: req.pay_to,
            resource,
            settlement,
            body,
        },
        agent,
        agent_note,
    })
}
