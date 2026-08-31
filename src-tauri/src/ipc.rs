// 결제 승인 IPC (Session 10) + AI 연결 배지 (개발 9).
//
// AI 에이전트가 MCP로 결제를 "요청"하면, 별도 프로세스인 MCP 서버가 ~/.jigap에 요청 파일을
// 쓴다. GUI 앱(이 프로세스)이 1초마다 폴링해 발견 → 승인 팝업 → 사용자가 비번 입력 → 실제
// 송금 → 결과 파일 작성 → MCP가 읽어 에이전트에 반환.
//
// 핵심 보안: 비번은 절대 요청/결과 파일이나 MCP에 들어가지 않는다. 키 접근(서명)은 오직 이
// GUI 프로세스만 한다 → MCP는 "요청"만, 승인은 사람이. 잠금·한도·내역은 기존 send 경로를
// 그대로 재사용하므로 자동 적용된다.

use crate::i18n::{tf, ts};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::store::{jigap_dir, now_secs, write_json};
use crate::transfer::{send_eth, send_usdc};
use crate::x402::{sign_x402_payment, X402Payment};

/// AI 에이전트가 보낸 결제 요청 1건. 비밀은 없다(비번은 GUI에서만 입력).
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct PaymentRequest {
    /// 고유 id (MCP가 생성한 유닉스 나노초 문자열). 결과 매칭용.
    pub(crate) id: String,
    /// "USDC" | "ETH".
    pub(crate) token: String,
    /// 받는 주소.
    pub(crate) to: String,
    /// 금액 (십진수 문자열).
    pub(crate) amount: String,
    /// 무엇에 대한 결제인지 — 사용자가 승인 팝업에서 보고 판단한다.
    pub(crate) memo: String,
    /// 요청 생성 유닉스 초 (타임아웃 카운트다운용).
    pub(crate) created: u64,
    /// "transfer"(온체인 송금, 기본) | "x402"(EIP-3009 오프체인 서명).
    /// 기존(Session 10) 요청 파일 호환을 위해 default = "transfer".
    #[serde(default = "default_kind")]
    pub(crate) kind: String,
    /// x402일 때 결제 대상 리소스 URL (팝업 표시용). transfer면 빈 문자열.
    #[serde(default)]
    pub(crate) resource: String,
    /// 요청이 만들어진 시점의 체인 ID (MCP가 활성 체인으로 각인). 승인 시 현재 활성 체인과 다르면
    /// 거부한다 — "테스트넷이라 생각하고 만든 대기 요청"이 메인넷으로 바뀐 뒤 실행되는 것 차단(코덱스 #2).
    /// 0 = 미각인(옛 요청 파일 호환) → 체인 검사 건너뜀.
    #[serde(default)]
    pub(crate) chain_id: u64,
    /// ERC-8004 대조 결과 (개발 47) — AI 가 에이전트 번호를 함께 준 x402 결제에만 붙는다.
    /// 없으면 승인 창은 예전 그대로다(**말할 사실이 있을 때만 한 줄이 붙는다**).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) agent: Option<AgentTrust>,
}

/// MCP 가 온체인에서 읽어 **대조까지 마친 사실**. GUI 는 판정하지 않고 그대로 보여준다.
///
/// kura-mcp 의 erc8004::AgentTrust 와 같은 모양(두 크레이트는 공유 크레이트를 만들지 않는 정책 —
/// 파일 JSON 이 계약이다). 필드를 바꾸면 양쪽을 함께 고쳐야 한다.
///
/// `wallet_check` = match | differs | unset | unknown, `domain_check` = match | differs | unknown.
/// **여기에 "안전/검증됨" 같은 판정은 없다** — 등록은 무허가라 누구나 아무 도메인이나 적을 수
/// 있고, 이 조회가 주는 건 일치·다름·모름뿐이다.
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct AgentTrust {
    pub(crate) agent_id: u64,
    pub(crate) chain_id: u64,
    pub(crate) registered: bool,
    pub(crate) wallet: String,
    pub(crate) wallet_check: String,
    pub(crate) uri_domain: String,
    pub(crate) resource_domain: String,
    pub(crate) domain_check: String,
    /// `None` = 못 읽음(0 과 구별한다 — 0 은 "아무도 안 남겼다"는 사실, None 은 "모른다").
    pub(crate) feedback_clients: Option<u32>,
}

fn default_kind() -> String {
    "transfer".to_string()
}

/// AI 가 주장한 신원을 **온체인이 부정하는가** — 자율 승인(비번 없이 나가는 경로)을 막을 조건.
///
/// 왜 필요한가: 승인 창에 경고를 만들어 놓아도, 자율 결제는 그 창을 **띄우지 않고** 지나간다
/// (`auto_approve_payment` 가 먼저 처리하고 성공하면 모달이 안 뜬다). 그러면 "받는 주소가
/// 등록 지갑과 다르다"는 경고가 정작 사람 눈에 닿지 않는다 — 경고가 없는 것과 같다
/// (코덱스 개발47 2차 P1).
///
/// 주장이 **없으면**(None) 막지 않는다. 이 규칙은 없던 결제를 새로 막는 게 아니라, 모순이
/// 드러난 결제만 사람 앞으로 돌린다. 그래서 방향이 한쪽뿐이다 — 자율을 **열어 주는 일은
/// 절대 없고**, 좁히기만 한다.
pub(crate) fn agent_contradicts(agent: Option<&AgentTrust>) -> bool {
    match agent {
        None => false,
        Some(a) => {
            !a.registered || a.wallet_check == "differs" || a.domain_check == "differs"
        }
    }
}

/// 대기 요청이 만들어진 체인과 현재 활성 체인이 같은지 검사한다(코덱스 개발20 #2). 다르면 거부 —
/// 승인/자율 경로 공용. chain_id 0 = 옛 미각인 요청이므로 검사를 건너뛴다(후방호환).
pub(crate) fn ensure_request_chain(req: &PaymentRequest) -> Result<(), String> {
    if req.chain_id != 0 && req.chain_id != crate::chain::active_chain().chain_id {
        return Err(
            ts!("결제 요청이 만들어진 네트워크와 현재 네트워크가 달라요 — 네트워크를 확인하고 다시 요청하세요.", "This request was made on a different network than the one you're on now — check the network and ask again.")
                .into(),
        );
    }
    Ok(())
}

/// 결제 요청 처리 결과. GUI가 쓰고 MCP가 읽는다.
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct PaymentResult {
    pub(crate) id: String,
    /// "approved" | "rejected" | "failed".
    pub(crate) status: String,
    /// approved + transfer 일 때 tx 해시, 아니면 "".
    pub(crate) tx_hash: String,
    /// rejected/failed 사유.
    pub(crate) detail: String,
    /// x402 승인일 때 서명한 결제 인가(MCP가 X-PAYMENT 헤더로 조립). transfer면 None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) x402: Option<X402Payment>,
}

/// GUI 생존 표시(하트비트). MCP는 이게 신선해야만 요청을 띄운다(앱 꺼져 있으면 5분 안 기다림).
#[derive(Serialize, Deserialize)]
struct Heartbeat {
    ts: u64,
    /// **승인 창을 실제로 띄울 수 있는 상태인가** (개발 51). 러스트 스레드는 프로세스가 살아
    /// 있는 한 계속 하트비트를 찍으므로, WebView 만 죽으면 「앱은 살아 있다」고 말하면서
    /// 승인 창은 영영 안 뜬다 → MCP 가 요청을 받아 두고 5분을 조용히 기다린다.
    /// 창을 여러 번 깨워도 폴링이 안 돌아오면 이 값을 false 로 내려 MCP 가 **즉시 정직하게**
    /// 거절하게 한다. 기본 true = 이 필드가 없던 옛 파일과의 호환.
    #[serde(default = "ui_ok_default")]
    ui_ok: bool,
}

fn ui_ok_default() -> bool {
    true
}

fn request_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join("payment_request.json"))
}

fn result_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join("payment_result.json"))
}

fn heartbeat_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join("app_alive.json"))
}

/// 대기 중인 결제 요청을 읽는다 (없으면 None).
pub(crate) fn read_request() -> Option<PaymentRequest> {
    request_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
}

/// MCP 가 승인을 기다리는 시간(초). kura-mcp 의 APPROVAL_TIMEOUT 과 같은 값.
const APPROVAL_WINDOW_SECS: u64 = 300;
/// 그 뒤로도 이만큼은 "살아 있는 요청"으로 본다 — GUI 가 MCP 보다 먼저 포기해서
/// 사용자가 승인할 수 있는 요청을 못 보게 되는 일이 없도록 한 여유분.
const STALE_GRACE_SECS: u64 = 60;

/// 승인 창이 닫혔어도 소용없을 만큼 오래된(또는 시각이 말이 안 되는) 요청인가.
/// 정상 타임아웃이면 MCP 가 cancel_request 로 파일을 치우지만, MCP 프로세스가 대기 중
/// 죽으면(예: AI 앱 재시작) 파일이 그대로 남는다. 그걸 영원히 "대기 중"으로 보면
/// 팝오버가 영영 안 닫힌다.
///
/// created 는 프로세스를 넘나드는 파일이라 벽시계일 수밖에 없다. 시계가 크게 뒤로 밀려
/// created 가 "미래"가 되면 경과가 늘 0이라 영원히 안 늙으므로, 유예를 넘는 미래 시각도
/// 만료로 본다(잠깐의 시계 오차는 유예 안에서 그대로 통과).
fn is_stale(req: &PaymentRequest) -> bool {
    let now = now_secs();
    if req.created > now.saturating_add(STALE_GRACE_SECS) {
        return true;
    }
    now.saturating_sub(req.created) > APPROVAL_WINDOW_SECS + STALE_GRACE_SECS
}

/// 지금 사람 승인을 기다리는 살아 있는 요청 (없거나 만료면 None).
/// 팝오버 자동 숨김·항상 위 고정(tray)과 프론트 승인 모달이 **같은 이 값**에서 나와야
/// "모달은 떠 있는데 게이트는 꺼진" 불일치가 생기지 않는다.
pub(crate) fn live_request() -> Option<PaymentRequest> {
    read_request().filter(|r| !is_stale(r))
}

/// 승인 대기 중인 결제 요청이 있는가 (single-flight 라 파일 하나가 진실 원천).
/// 프론트가 보내는 신호로 게이트를 켜고 끄면 raise/release 가 비동기라 순서가 뒤집힐 때
/// 게이트가 영구히 켜지거나 꺼진 채 남는다. 상태에서 파생시키면 그런 경합 자체가 없다.
pub(crate) fn has_pending() -> bool {
    live_request().is_some()
}

/// 처리 결과를 기록하고 대기 요청을 치운다 (single-flight 해제).
///
/// 🔴 **결과 파일도 요청 파일도, 「이 결과의 요청」이 아직 대기 중일 때만 건드린다**
/// (코덱스 개발51 1차·3차 P1). 결과 파일은 **한 칸뿐**이라, 늦게 끝난 승인 A 가 그 뒤
/// 승인된 B 의 결과를 덮으면 **B 를 기다리던 쪽은 돈이 나갔는데도 시간 초과**로 보고
/// 재시도한다(이중 결제). 요청 파일 쪽도 같은 이유로 남의 것을 치우면 안 된다.
///
/// 내 요청이 이미 사라졌다면(상대가 시간 초과로 거둬감) **조용히 아무것도 안 쓴다** —
/// 그 결과를 기다리는 쪽은 이미 없고, 결과를 남기면 다음 사람의 답을 가린다.
/// 사람에게는 어차피 호출자(승인 창)가 반환값으로 결과를 보여준다.
pub(crate) fn resolve_request(result: &PaymentResult) -> Result<(), String> {
    let pending = read_request().map(|r| r.id);
    if !owns_pending(pending.as_deref(), &result.id) {
        return Ok(());
    }
    write_json(result_path()?, result)?;
    if let Ok(p) = request_path() {
        let _ = fs::remove_file(p);
    }
    Ok(())
}

/// 이 결과가 「지금 대기 중인 그 요청」의 것인가 (순수 — 테스트용).
/// 대기 요청이 없으면 상대는 이미 떠난 뒤다. **다른 id** 면 남의 요청이므로 절대 건드리지 않는다.
fn owns_pending(pending_id: Option<&str>, result_id: &str) -> bool {
    pending_id == Some(result_id)
}

/// 지금 **승인 처리가 진행 중인** 결제 수. 0 이 아니면 서명·전송이 체인으로 나가는 중일 수 있다.
static APPROVALS_IN_FLIGHT: AtomicU64 = AtomicU64::new(0);

/// 승인 처리 구간을 표시하는 RAII 가드. 중간에 `?` 로 빠져나가도 Drop 이 카운터를 되돌린다.
pub(crate) struct ApprovalGuard;

impl ApprovalGuard {
    pub(crate) fn new() -> Self {
        APPROVALS_IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for ApprovalGuard {
    fn drop(&mut self) {
        APPROVALS_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

fn approval_in_flight() -> bool {
    APPROVALS_IN_FLIGHT.load(Ordering::SeqCst) > 0
}

// ---------- 생존 감시 (개발 49) ----------
//
// 개발 48 실측: 창을 안 건드리면 **약 5분 뒤 프론트 폴링이 완전히 멎는다**(WebView 타이머
// 억제). 하트비트가 그 폴링에만 매달려 있어 같이 멎었고, MCP 는 그 뒤 도착한 결제를
// 「지갑 앱이 실행 중이 아니에요」로 거절했다. 트레이 상주 앱이라 **창이 숨어 있는 게 평상시**고,
// "AI 와 대화하다 결제가 필요해진" 순간은 대개 지갑을 5분 넘게 안 건드린 뒤다 → 드문 일이 아니다.
//
// 고침은 **두 개가 한 쌍**이다. ① 하트비트를 러스트가 찍는다(프로세스가 살아 있다는 건 러스트만
// 정직하게 안다). ②만 빼고 ①만 하면 **개악**이다 — 정직한 즉시 거절이 "요청은 받았는데 창은
// 안 뜸"이라는 5분짜리 침묵으로 바뀐다. 그래서 ② **요청이 왔는데 프론트가 자고 있으면 러스트가
// 창을 깨운다**. 창이 뜨면 WebView 가 되살아나 기존 경로(자율 승인 시도 → 실패 시 모달)가
// 그대로 이어진다.

/// 하트비트를 다시 쓰는 주기(초). MCP 신선도 기준(payment.rs ALIVE_SECS = 10초)의 5배 여유.
const HEARTBEAT_SECS: u64 = 2;

/// 프론트 폴링이 이만큼 끊기면 "WebView 가 잠들었다"로 본다. 폴링 주기가 1초라 3초면
/// 두 번을 내리 놓친 뒤다 — 한 박자 밀린 것과 구별된다.
const FRONT_STALE_SECS: u64 = 3;

/// 잠든 프론트를 깨우려고 창을 다시 띄우는 최소 간격(초). 한 번의 show 로 안 깨어난
/// 경우에만 다시 시도한다(깨어나면 폴링이 돌아와 아래 조건에서 걸러진다).
const WAKE_RETRY_SECS: u64 = 15;

/// 창을 이만큼 깨워 봐도 폴링이 안 돌아오면 **WebView 가 죽었다**고 본다 (개발 51).
/// `WAKE_RETRY_SECS` 간격이므로 3회 = 약 45초.
///
/// **창을 띄우는 것 자체가 확인 절차다** — 살아만 있으면 숨어 있던 WebView 도 창이 뜨는 순간
/// 타이머가 되살아난다. 개발 51 실측(디버그 번들 + 가짜 HOME):
/// · 창을 한 번도 안 띄운 앱은 **약 7초 뒤 폴링이 멎는다**(그 뒤 400초 넘게 0회) — 즉 결제가
///   올 때 프론트가 «자고 있는» 건 예외가 아니라 **평상시**다.
/// · 그 상태에 요청이 오면 깨움 **1회로 폴링이 5초 안에 돌아오고**(1회/초 재개) 이 카운터는
///   0으로 리셋된다 → **정상 앱을 죽었다고 오판하지 않는다**(실측: ui_ok 계속 true, 요청은
///   사람을 기다린 채 유지).
/// · 반대로 렌더러가 멈춘(hang) 앱은 깨움 3회를 채워 **36초 만에** 정직한 실패로 끝났다.
/// · WebContent 프로세스를 SIGKILL 하면 **WebKit 이 스스로 되살린다**(폴링 계속) → 오탐 없음.
const DEAD_WAKES: u32 = 3;

/// 프론트(WebView)가 마지막으로 폴링한 시각. 0 = 아직 한 번도.
static LAST_POLL: AtomicU64 = AtomicU64::new(0);

/// 하트비트를 찍고, 프론트가 자는 동안 도착한 결제 요청에 창을 깨우는 상주 스레드.
///
/// **프론트가 깨어 있으면 아무것도 하지 않는다**(하트비트만 계속 찍는다). 이게 중요하다 —
/// 요청이 오자마자 창을 띄우면 자율 결제(창 없이 조용히 끝나야 하는 경로)까지 창이 튀어나온다.
/// 지금 동작은 "프론트가 살아 있으면 예전 그대로, 자고 있을 때만 러스트가 대신 깨운다"다.
///
/// 창 조작은 AppKit 을 건드리므로 반드시 메인 스레드에서 한다(트레이 rect 조회 포함).
fn watchdog(app: tauri::AppHandle) {
    let mut last_beat = 0u64;
    // 마지막으로 창을 깨운 시각. 0 = 아직 안 깨움 → 다음 대기 요청에 즉시 깨운다.
    let mut last_wake = 0u64;
    // 우리가 마지막으로 적용한 "항상 위" 값. None = 아직 한 번도 안 건드림.
    let mut last_pinned: Option<bool> = None;
    // 창을 깨운 뒤에도 프론트가 폴링을 안 한 횟수. 폴링이 한 번이라도 돌아오면 0으로 리셋.
    let mut wakes_without_poll: u32 = 0;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let now = now_secs();
        let front_alive = now.saturating_sub(LAST_POLL.load(Ordering::Relaxed)) <= FRONT_STALE_SECS;
        if front_alive {
            wakes_without_poll = 0;
        }
        // 창을 여러 번 깨워도 폴링이 안 돌아온다 = WebView 가 죽었다(개발 51).
        //
        // **마지막 깨움에도 회복할 시간을 준다** (코덱스 개발51 2차 P2). 카운터는 깨운 *뒤*에
        // 올라가므로, 이 조건만 보면 3번째 깨움은 다음 1초 루프에서 곧장 사망 판정을 맞는다 —
        // 실측(개발 51)상 깨운 뒤 폴링이 돌아오는 데 5초쯤 걸리므로 1초는 너무 짧다.
        // 마지막 깨움이 `WAKE_RETRY_SECS` 만큼 묵은 뒤에야 판정한다 = 주석대로 "3회 ≈ 45초".
        let ui_ok = wakes_without_poll < DEAD_WAKES
            || now.saturating_sub(last_wake) < WAKE_RETRY_SECS;
        // 하트비트가 뜻하는 건 "프로세스가 살아 있다"가 아니라 **"여기서 사람이 승인까지 할 수
        // 있다"**이다. 지갑이 아직 없으면(첫 실행·평문 마이그레이션 대기) 프론트는 SetupScreen 을
        // 그리고 WalletScreen 은 아예 안 뜬다 → 승인 창을 띄울 경로가 없다. 그 상태에서 살아
        // 있다고 하면 MCP 가 요청을 받아 두고 **5분을 조용히 기다린다**(코덱스 개발49 1차 P2).
        // 옛 코드도 결과적으로 같은 조건이었다 — 하트비트를 WalletScreen 의 폴링이 찍었으니
        // 지갑이 없으면 안 찍혔다. 여기서 명시적으로 같은 선을 긋는다.
        if now.saturating_sub(last_beat) >= HEARTBEAT_SECS && !crate::wallet::needs_setup() {
            last_beat = now;
            let _ = write_json(
                heartbeat_path().unwrap_or_default(),
                &Heartbeat { ts: now, ui_ok },
            );
        }
        // 프론트가 살아 있다 → 폴링이 이미 같은 일(고정 수렴·모달·자율 승인)을 하고 있다.
        if front_alive {
            last_pinned = None; // 주도권을 넘긴다 — 다시 잠들면 그때 처음부터 다시 맞춘다.
            continue;
        }
        // 여기서부터는 "프론트가 잔다" — 고정 수렴도 우리가 대신한다.
        let pinned = has_pending();
        if !pinned {
            last_wake = 0;
        }
        let wake = pinned && now.saturating_sub(last_wake) >= WAKE_RETRY_SECS;
        if wake {
            last_wake = now;
            wakes_without_poll = wakes_without_poll.saturating_add(1);
        }
        // WebView 가 죽었다고 판정된 순간, **기다리는 요청을 5분 침묵으로 두지 않는다** (개발 51).
        // 승인 창이 뜰 수 없다는 걸 아는 쪽이 러스트뿐이므로 여기서 끝내 준다. 「사람이 거부」가
        // 아니라 「띄우지 못했다」라 status 는 failed 다 — 아무도 승인하지 않았고, AI 는 45초 안에
        // 이유를 읽는다. 사람에게도 알린다: 화면이 죽은 걸 알 방법이 알림뿐이다(창이 안 뜬다).
        // 🔴 **승인이 진행 중이면 손대지 않는다** (코덱스 개발51 1차 P1). 사람이 비번을 넣은
        // 직후 WebView 가 죽으면 러스트는 아직 서명·전송 중일 수 있다 — 그때 「실패」라고 답하면
        // AI 가 재시도해 **같은 돈이 두 번** 나갈 수 있다. 진짜로 나가는 중인 결제는
        // 5분 대기가 정직한 답이다(그 5분 안에 성공하면 성공이 그대로 전달된다).
        if !ui_ok && !approval_in_flight() {
            if let Some(req) = live_request() {
                let _ = resolve_request(&PaymentResult {
                    id: req.id,
                    status: "failed".into(),
                    tx_hash: String::new(),
                    detail: ts!(
                        "지갑 앱 화면이 응답하지 않아 승인 창을 띄우지 못했어요. 앱을 다시 시작한 뒤 다시 요청하세요.",
                        "The wallet app's window isn't responding, so the approval prompt couldn't be shown. Restart the app and try again."
                    )
                    .into(),
                    x402: None,
                });
                crate::notify::show_notification(
                    ts!("지갑 화면이 응답하지 않아요", "The wallet window isn't responding"),
                    ts!(
                        "결제 승인 창을 띄우지 못했어요. 앱을 완전히 종료했다 다시 켜 주세요.",
                        "A payment approval prompt couldn't be shown. Quit the app completely and open it again."
                    ),
                );
            }
        }
        // **대기 요청이 없고 값도 그대로면 메인 스레드를 깨우지 않는다.** 트레이 상주 앱은
        // 하루 종일 이 상태로 있다 — 여기서 매초 메시지를 보내면 앱이 영영 못 쉰다.
        // 반대로 승인 대기 중(pinned=true)에는 매초 다시 적용한다: 그 몇 초가 돈이 걸린
        // 구간이고, tray::set_pinned 의 자가 치유(값을 캐시하지 않는다)가 필요한 곳이다.
        let converge = pinned || last_pinned == Some(true);
        if !converge && !wake {
            continue;
        }
        last_pinned = Some(pinned);
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            crate::tray::set_pinned(&handle, pinned);
            if wake {
                crate::tray::show(&handle);
            }
        });
    }
}

/// 감시 스레드를 띄운다 (setup 에서 한 번).
pub(crate) fn spawn_watchdog(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || watchdog(app));
}

/// GUI가 1초마다 폴링한다. 대기 요청을 돌려준다.
///
/// **하트비트는 여기서 찍지 않는다**(개발 49). WebView 의 타이머는 창이 숨은 채 5분이 지나면
/// 멎어서(개발 48 실측: 0~300초 정상 → 약 305초부터 10분간 0회) 하트비트가 같이 멎었고,
/// 그동안 MCP 가 결제를 「앱이 실행 중이 아니에요」로 거절했다 — 앱은 멀쩡히 떠 있는데도.
/// 지금은 러스트 스레드(watchdog)가 찍고, 이 커맨드는 **프론트가 아직 깨어 있다는 표식**만
/// 남긴다. watchdog 이 그 표식을 보고 "WebView 가 잠들었는지"를 판단한다.
#[tauri::command]
pub(crate) fn get_pending_request(app: tauri::AppHandle) -> Option<PaymentRequest> {
    LAST_POLL.store(now_secs(), Ordering::Relaxed);
    // 파일은 한 번만 읽고 그 값으로 둘 다 정한다 — 두 번 읽으면 그 사이에 요청이
    // 생기거나 사라질 때 한 폴링 안에서 "모달은 뜨는데 고정은 꺼진" 상태가 나온다.
    let pending = live_request();
    // 승인 대기 여부에 맞춰 "항상 위" 고정을 매 초 다시 맞춘다(개발 26).
    // raise/release 커맨드는 프론트에서 비동기로 날아와 도착 순서가 뒤집힐 수 있는데,
    // 이 폴링이 상태에서 다시 파생시키므로 어긋나도 1초 안에 스스로 고쳐진다.
    crate::tray::set_pinned(&app, pending.is_some());
    pending
}

/// 결제 요청을 승인한다 — 비번으로 실제 송금을 실행하고 결과를 기록한다.
/// 기존 send_eth/send_usdc 를 그대로 호출하므로 긴급잠금·한도·거래내역이 자동 적용된다.
/// 실패(비번 오류·잠금·한도 등) 시엔 요청을 치우지 않는다 → 팝업에서 재시도하거나 거부할 수 있다.
#[tauri::command]
pub(crate) async fn approve_payment(id: String, password: String) -> Result<PaymentResult, String> {
    let req = read_request().ok_or(ts!(
        "대기 중인 결제 요청이 없습니다",
        "There's no payment request waiting"
    ))?;
    if req.id != id {
        return Err(ts!(
            "요청 ID가 일치하지 않습니다",
            "That request ID doesn't match"
        )
        .into());
    }
    ensure_request_chain(&req)?; // 요청 시점 체인 ≠ 현재 체인이면 거부(메인넷 오발사 차단)

    // 여기서부터는 돈이 나갈 수 있는 구간 — 감시 스레드가 이 요청을 실패로 끝내면 안 된다.
    let _in_flight = ApprovalGuard::new();

    // kind 에 따라 처리 경로가 다르다. 둘 다 잠금·한도·내역을 자동 적용한다(같은 하부 함수 재사용).
    // 실패하면 ?로 즉시 반환(요청 파일 유지) → 팝업에서 재시도/거부 가능.
    let result = match req.kind.as_str() {
        // x402: 온체인 전송이 아니라 EIP-3009 인가를 서명만 한다(페이실리테이터가 정산).
        "x402" => {
            let payment =
                sign_x402_payment(password, req.to.clone(), req.amount.clone(), None).await?;
            PaymentResult {
                id: req.id,
                status: "approved".into(),
                tx_hash: String::new(),
                detail: String::new(),
                x402: Some(payment),
            }
        }
        // transfer(기본): 실제 온체인 송금 — 기존 경로 재사용.
        _ => {
            let hash = match req.token.as_str() {
                "USDC" => send_usdc(password, req.to.clone(), req.amount.clone()).await,
                "ETH" => send_eth(password, req.to.clone(), req.amount.clone()).await,
                other => Err(tf!(
                    "지원하지 않는 토큰: {other}",
                    "Unsupported token: {other}"
                )),
            }?;
            PaymentResult {
                id: req.id,
                status: "approved".into(),
                tx_hash: hash,
                detail: String::new(),
                x402: None,
            }
        }
    };
    resolve_request(&result)?;
    Ok(result)
}

/// 결제 요청을 거부한다 — MCP에 "거부됨"을 알리고 대기 요청을 치운다.
#[tauri::command]
pub(crate) fn reject_payment(id: String, reason: Option<String>) -> Result<(), String> {
    let req = read_request().ok_or(ts!(
        "대기 중인 결제 요청이 없습니다",
        "There's no payment request waiting"
    ))?;
    if req.id != id {
        return Err(ts!(
            "요청 ID가 일치하지 않습니다",
            "That request ID doesn't match"
        )
        .into());
    }
    resolve_request(&PaymentResult {
        id: req.id,
        status: "rejected".into(),
        tx_hash: String::new(),
        detail: reason
            .unwrap_or_else(|| ts!("사용자가 거부했습니다", "Rejected by the user").into()),
        x402: None,
    })
}

/// MCP 서버(=AI 클라이언트)의 생존 하트비트. MCP가 살아있는 동안 주기적으로 쓴다.
#[derive(Deserialize)]
struct McpAlive {
    ts: u64,
    #[serde(default)]
    client: String,
}

/// 프론트로 주는 AI 연결 상태 — 메인 화면 "연결됨" 배지용.
#[derive(Serialize)]
pub(crate) struct AgentStatus {
    connected: bool,
    /// 연결한 클라이언트 이름(예: "claude-code"). 모르면 빈 문자열.
    client: String,
}

fn mcp_alive_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join("mcp_alive.json"))
}

/// AI 에이전트(MCP)가 지금 이 지갑에 연결돼 있는지 알려준다.
/// MCP가 5초마다 하트비트를 쓰므로, 15초 이내면 "연결됨"으로 본다(3회 여유).
#[tauri::command]
pub(crate) fn get_agent_status() -> AgentStatus {
    let alive = mcp_alive_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<McpAlive>(&s).ok());
    match alive {
        Some(a) if now_secs().saturating_sub(a.ts) <= 15 => AgentStatus {
            connected: true,
            client: a.client,
        },
        _ => AgentStatus {
            connected: false,
            client: String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::x402::X402Authorization;

    fn req_created(created: u64) -> PaymentRequest {
        PaymentRequest {
            id: "1".into(),
            token: "USDC".into(),
            to: "0x0".into(),
            amount: "1".into(),
            memo: String::new(),
            created,
            kind: "transfer".into(),
            resource: String::new(),
            chain_id: 0,
            agent: None,
        }
    }

    /// 🔴 늦게 끝난 승인은 **결과 칸도 요청 파일도** 건드리면 안 된다
    /// (코덱스 개발51 1차·3차 P1). 남의 요청을 치우면 single-flight 가 깨지고,
    /// 남의 결과를 덮으면 그쪽은 돈이 나갔는데도 시간 초과로 보고 재시도한다.
    #[test]
    fn resolve_only_touches_its_own_request() {
        assert!(owns_pending(Some("A"), "A"), "같은 요청이면 기록한다");
        assert!(!owns_pending(Some("B"), "A"), "남의 요청은 건드리지 않는다");
        assert!(!owns_pending(None, "A"), "이미 거둬간 요청이면 남길 것도 없다");
    }

    /// 승인 처리 중에는 감시 스레드가 요청을 실패로 끝내면 안 된다 — 가드가 그 구간을 표시한다.
    /// Drop 으로 되돌아가야 `?` 로 중간에 빠져나가도 카운터가 새지 않는다.
    #[test]
    fn approval_guard_marks_in_flight() {
        assert!(!approval_in_flight());
        {
            let _g = ApprovalGuard::new();
            assert!(approval_in_flight());
            {
                let _g2 = ApprovalGuard::new(); // 겹쳐도(자율+수동) 카운트로 버틴다
                assert!(approval_in_flight());
            }
            assert!(approval_in_flight(), "안쪽 하나가 끝나도 바깥은 진행 중");
        }
        assert!(!approval_in_flight());
    }

    // 방금 만들어진 요청은 살아 있다 = 팝오버를 붙잡아야 한다.
    #[test]
    fn fresh_request_is_not_stale() {
        assert!(!is_stale(&req_created(now_secs())));
    }

    // 승인 시간(5분) 안이면 아직 살아 있다.
    #[test]
    fn request_within_window_is_not_stale() {
        assert!(!is_stale(&req_created(
            now_secs() - (APPROVAL_WINDOW_SECS - 10)
        )));
    }

    // 유예분까지 지나면 죽은 요청 — MCP 가 죽어 남긴 파일이 팝오버를 영영 붙잡지 못하게.
    #[test]
    fn long_expired_request_is_stale() {
        let old = now_secs() - (APPROVAL_WINDOW_SECS + STALE_GRACE_SECS + 10);
        assert!(is_stale(&req_created(old)));
    }

    // 살짝 미래(시계 오차 수준)는 그대로 살아 있는 요청으로 본다.
    #[test]
    fn slightly_future_created_is_not_stale() {
        assert!(!is_stale(&req_created(now_secs() + STALE_GRACE_SECS / 2)));
    }

    // 유예를 크게 넘는 미래 시각은 만료로 본다 — 경과가 늘 0이라 영원히 안 늙는
    // 요청이 팝오버를 영구히 붙잡는 구멍을 막는다(시계가 뒤로 밀린 경우).
    #[test]
    fn far_future_created_is_stale() {
        assert!(is_stale(&req_created(now_secs() + 10_000)));
    }

    /// **크레이트 간 계약**: kura-mcp 가 쓰는 `agent` 필드를 GUI 가 그대로 읽어야 한다.
    /// 두 크레이트는 코드를 공유하지 않으므로(정책) 이 JSON 자체가 계약이다 — 한쪽에서
    /// 필드 이름을 바꾸면 조용히 None 이 되어 승인 창의 줄만 사라진다. 그래서 값까지 본다.
    #[test]
    fn request_carries_agent_trust_from_mcp() {
        let json = r#"{"id":"1","token":"USDC","to":"0xB0b","amount":"0.01","memo":"",
          "created":1,"kind":"x402","resource":"https://api.example.com/x","chain_id":8453,
          "agent":{"agent_id":123,"chain_id":8453,"registered":true,
          "wallet":"0xB0b","wallet_check":"match","uri_domain":"api.example.com",
          "resource_domain":"api.example.com","domain_check":"match","feedback_clients":20}}"#;
        let r: PaymentRequest = serde_json::from_str(json).unwrap();
        let a = r.agent.expect("agent 필드");
        assert_eq!(a.agent_id, 123);
        assert!(a.registered);
        assert_eq!(a.wallet_check, "match");
        assert_eq!(a.domain_check, "match");
        assert_eq!(a.uri_domain, "api.example.com");
        assert_eq!(a.feedback_clients, Some(20));
    }

    /// 번호를 안 준 결제(대다수)는 `agent` 가 아예 없다 → None. 옛 요청 파일도 마찬가지.
    /// 이게 깨지면 승인 창이 못 뜨거나(파싱 실패) 없는 사실을 말하게 된다.
    #[test]
    fn request_without_agent_is_none() {
        let json = r#"{"id":"1","token":"USDC","to":"0xabc","amount":"1","memo":"","created":1}"#;
        let r: PaymentRequest = serde_json::from_str(json).unwrap();
        assert!(r.agent.is_none());
    }

    fn trust(registered: bool, wallet: &str, domain: &str) -> AgentTrust {
        AgentTrust {
            agent_id: 1,
            chain_id: 8453,
            registered,
            wallet: "0xB0b".into(),
            wallet_check: wallet.into(),
            uri_domain: "api.example.com".into(),
            resource_domain: "api.example.com".into(),
            domain_check: domain.into(),
            feedback_clients: None,
        }
    }

    /// 자율 승인 차단 조건 — 모순이 드러난 경우만 막고, 주장이 없으면 예전 그대로 간다.
    #[test]
    fn agent_contradiction_blocks_only_when_contradicted() {
        // 주장 없음 = 예전과 동일하게 자율 통과(이 기능이 기존 결제를 새로 막지 않는다).
        assert!(!agent_contradicts(None));
        // 일치·모름은 막지 않는다.
        assert!(!agent_contradicts(Some(&trust(true, "match", "match"))));
        assert!(!agent_contradicts(Some(&trust(true, "unset", "unknown"))));
        assert!(!agent_contradicts(Some(&trust(true, "unknown", "unknown"))));
        // 어긋난 것들은 사람 앞으로.
        assert!(agent_contradicts(Some(&trust(true, "differs", "match"))));
        assert!(agent_contradicts(Some(&trust(true, "match", "differs"))));
        assert!(agent_contradicts(Some(&trust(false, "unknown", "unknown"))));
    }

    // 결제 요청 JSON 왕복 — MCP가 쓰는 형식과 호환돼야 한다 (memo 한글 포함).
    #[test]
    fn payment_request_roundtrip() {
        let r = PaymentRequest {
            id: "1780623842000000000".into(),
            token: "USDC".into(),
            to: "0xabc".into(),
            amount: "1.5".into(),
            memo: "데이터 API 1회 호출".into(),
            created: 1780623842,
            kind: "transfer".into(),
            resource: String::new(),
            chain_id: 84_532,
            agent: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: PaymentRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "1780623842000000000");
        assert_eq!(back.token, "USDC");
        assert_eq!(back.memo, "데이터 API 1회 호출");
        assert_eq!(back.created, 1780623842);
        assert_eq!(back.kind, "transfer");
    }

    // 기존(Session 10) 요청 파일은 kind/resource 가 없다 → default 로 채워진다(무손실 호환).
    #[test]
    fn legacy_payment_request_defaults() {
        let json = r#"{"id":"1","token":"USDC","to":"0xabc","amount":"1","memo":"","created":1}"#;
        let r: PaymentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(r.kind, "transfer");
        assert!(r.resource.is_empty());
        assert_eq!(r.chain_id, 0); // 옛 요청엔 chain_id 없음 → 0(=체인 검사 건너뜀, 후방호환)
    }

    // 결제 결과 JSON 왕복 — GUI가 쓰고 MCP가 읽는 형식.
    #[test]
    fn payment_result_roundtrip() {
        let r = PaymentResult {
            id: "1".into(),
            status: "approved".into(),
            tx_hash: "0xdeadbeef".into(),
            detail: String::new(),
            x402: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: PaymentResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.status, "approved");
        assert_eq!(back.tx_hash, "0xdeadbeef");
        assert!(back.detail.is_empty());
        // transfer 결과는 x402 필드를 직렬화하지 않는다(skip_serializing_if).
        assert!(!json.contains("x402"));
    }

    // x402 결과 JSON 왕복 — GUI가 서명 페이로드를 함께 쓴다.
    #[test]
    fn x402_payment_result_roundtrip() {
        let r = PaymentResult {
            id: "1".into(),
            status: "approved".into(),
            tx_hash: String::new(),
            detail: String::new(),
            x402: Some(X402Payment {
                signature: "0xsig".into(),
                authorization: X402Authorization {
                    from: "0xa".into(),
                    to: "0xb".into(),
                    value: "10000".into(),
                    valid_after: "0".into(),
                    valid_before: "99".into(),
                    nonce: "0x1".into(),
                },
            }),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"x402\""));
        let back: PaymentResult = serde_json::from_str(&json).unwrap();
        let p = back.x402.expect("x402 페이로드");
        assert_eq!(p.signature, "0xsig");
        assert_eq!(p.authorization.value, "10000");
    }

    // MCP 하트비트 파싱 — client 필드 있/없 둘 다 (옛 형식 호환).
    #[test]
    fn mcp_alive_parses() {
        let a: McpAlive =
            serde_json::from_str(r#"{"ts":1780775675,"client":"claude-code"}"#).unwrap();
        assert_eq!(a.ts, 1780775675);
        assert_eq!(a.client, "claude-code");
        // client 없으면 기본 빈 문자열.
        let b: McpAlive = serde_json::from_str(r#"{"ts":1}"#).unwrap();
        assert!(b.client.is_empty());
    }
}
