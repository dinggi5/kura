// 결제 요청 IPC (개발 9, Session 10) — MCP가 결제를 "요청"하고 GUI 승인을 기다린다.
//
// 비번은 절대 여기로 들어오지 않는다. MCP는 ~/.jigap에 요청 파일만 쓰고, 실제 서명·전송은
// GUI 앱(src-tauri)이 한다. 흐름:
//   write_request_agent() → GUI가 팝업으로 사람 승인 → GUI가 결과 파일 작성 → await_result()가 읽어 반환.
//
// single-flight: 한 번에 대기 요청 1건. 앱이 안 켜져 있으면(하트비트 신선도) 즉시 안내.

use crate::chain::chain_file;
use crate::erc8004::AgentTrust;
use crate::wallet::jigap_dir;
use crate::{tf, ts};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// GUI가 살아있다고 볼 하트비트 최대 나이(초). 이보다 오래되면 앱이 꺼진 것으로 본다.
const ALIVE_SECS: u64 = 10;
/// 사용자 승인 대기 최대 시간(초). GUI 팝업 카운트다운과 일치(5분).
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

/// 에이전트가 보내는 결제 요청. 비밀은 없다(비번은 GUI에서만).
#[derive(Serialize, Deserialize, Clone)]
pub struct PaymentRequest {
    pub id: String,
    pub token: String,
    pub to: String,
    pub amount: String,
    pub memo: String,
    pub created: u64,
    /// "transfer"(온체인 송금, 기본) | "x402"(EIP-3009 오프체인 서명) |
    /// "x402-direct"(EIP-3009 인가를 **우리가 직접 체인에 올린다** — 개발 64, Arc).
    /// 기존 요청 파일 호환을 위해 default = "transfer".
    #[serde(default = "default_kind")]
    pub kind: String,
    /// `x402-direct` 일 때 **서명에 쓸 EIP-3009 nonce**(0x + 32바이트 hex). 다른 kind 면 빈 값.
    ///
    /// 왜 MCP 가 정해 주나: 이 값은 서버 요구사항에서 규격대로 유도한 것이라(arc_direct 참고)
    /// 랜덤이면 안 된다 — 서버가 자기 요구사항으로 같은 값을 다시 만들어 대조한다. GUI 는 이걸
    /// **불투명한 32바이트**로 받아 그대로 서명한다(어떤 값이든 인가의 의미는 to·value 가 정한다).
    #[serde(default)]
    pub nonce: String,
    /// x402일 때 결제 대상 리소스 URL (사용자가 팝업에서 본다). transfer면 빈 문자열.
    #[serde(default)]
    pub resource: String,
    /// 요청 생성 시점의 활성 체인 ID — GUI가 승인 시 현재 체인과 다르면 거부한다(코덱스 개발20 #2).
    #[serde(default)]
    pub chain_id: u64,
    /// 요청 생성 시점의 **활성 계정** (개발 54) — 파생 인덱스 + 주소. GUI 가 승인 시 지금 계정과
    /// 대조해 다르면 거부하고, 같으면 승인 작업 전체를 이 계정으로 고정한다(chain_id 와 같은 처방).
    #[serde(default)]
    pub account: u32,
    #[serde(default)]
    pub from: String,
    /// ERC-8004 대조 결과 (개발 47). AI 가 에이전트 번호를 함께 준 x402 결제에서만 채워진다 —
    /// 없으면 승인 창은 예전 그대로다(**말할 사실이 있을 때만 한 줄이 붙는다**).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentTrust>,
}

fn default_kind() -> String {
    "transfer".to_string()
}

/// GUI가 쓰는 처리 결과.
#[derive(Deserialize)]
pub struct PaymentResult {
    pub id: String,
    pub status: String,
    pub tx_hash: String,
    pub detail: String,
    /// x402 승인일 때 GUI가 서명해 돌려준 결제 인가. transfer면 None.
    #[serde(default)]
    pub x402: Option<crate::x402::X402Payment>,
}

#[derive(Deserialize)]
struct Heartbeat {
    ts: u64,
    /// **승인 창을 실제로 띄울 수 있나** (개발 51). GUI 의 러스트 스레드는 프로세스가 살아 있는
    /// 한 하트비트를 찍으므로, WebView 만 죽으면 「살아 있다」고 말하면서 창은 안 뜬다.
    /// 그쪽이 창을 여러 번 깨워 보고도 안 되면 이 값을 false 로 내린다 → 즉시 정직하게 거절한다.
    /// 없으면 true (이 필드가 없던 옛 앱과의 호환 — 예전과 똑같이 동작).
    #[serde(default = "ui_ok_default")]
    ui_ok: bool,
    /// **앱이 처리할 수 있는 결제 방식** (개발 64). 새 방식을 옛 앱에 보내지 않으려고 본다 —
    /// 옛 앱은 모르는 kind 를 **평범한 송금으로 처리**했다(돈은 나가고 결제는 성립하지 않는다).
    /// 필드가 없으면 그 시절 앱이므로 송금·x402 서명 둘만 할 수 있는 것으로 본다.
    #[serde(default = "kinds_default")]
    kinds: Vec<String>,
}

fn ui_ok_default() -> bool {
    true
}

fn kinds_default() -> Vec<String> {
    vec!["transfer".into(), "x402".into()]
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

fn mcp_alive_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join("mcp_alive.json"))
}

fn settlements_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join(chain_file("x402_settlements")))
}

/// x402 정산 결과 1건 — GUI가 읽어 내역의 "signed"(nonce 매칭)을 "settled"+tx 로 갱신한다.
#[derive(Serialize, Deserialize)]
struct Settlement {
    nonce: String,
    tx: String,
    success: bool,
}

/// 임시 파일에 쓴 뒤 rename 으로 원자 교체 — GUI(별도 프로세스)가 폴링으로 읽는 파일들이라,
/// 쓰는 도중의 절반 써진 내용을 GUI가 읽는 일이 없게 한다.
/// 권한은 src-tauri 의 store::write_atomic 과 동일하게 디렉터리 0700 / 파일 0600 으로 맞춘다
/// (모든 ~/.jigap 파일 권한 일관 적용 불변식 — MCP 가 먼저 파일을 만들어도 넓게 노출되지 않게).
fn write_atomic(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|e| tf!("디렉터리 생성 실패: {e}", "Couldn't create the folder: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
        }
    }
    let tmp = path.with_extension("tmp");
    write_file_private(&tmp, bytes)?;
    fs::rename(&tmp, path).map_err(|e| tf!("파일 교체 실패: {e}", "Couldn't replace the file: {e}"))
}

/// 파일을 0600 으로 생성해 내용을 쓴다 (생성 후 chmod 사이의 노출 창 제거).
#[cfg(unix)]
fn write_file_private(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| tf!("파일 저장 실패: {e}", "Couldn't save the file: {e}"))?;
    let _ = f.set_permissions(fs::Permissions::from_mode(0o600));
    f.write_all(bytes)
        .map_err(|e| tf!("파일 저장 실패: {e}", "Couldn't save the file: {e}"))
}

#[cfg(not(unix))]
fn write_file_private(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|e| tf!("파일 저장 실패: {e}", "Couldn't save the file: {e}"))
}

/// 정산 결과를 ~/.jigap/x402_settlements.json 에 추가한다(append). 실패해도 결제 흐름은 안 막는다.
/// nonce = 서명 인가의 nonce(GUI 내역 detail 과 매칭). 비밀 아님(공개 결제 증빙).
pub fn record_settlement(nonce: &str, tx: &str, success: bool) -> Result<(), String> {
    let path = settlements_path()?;
    let mut list: Vec<Settlement> = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    list.push(Settlement {
        nonce: nonce.to_string(),
        tx: tx.to_string(),
        success,
    });
    let json = serde_json::to_string(&list).map_err(|e| {
        tf!(
            "정산 기록 직렬화 실패: {e}",
            "Couldn't serialize the settlement record: {e}"
        )
    })?;
    write_atomic(&path, json.as_bytes())
}

/// MCP(=AI 클라이언트) 생존 표시를 쓴다. GUI가 이걸 보고 "AI 연결됨" 배지를 띄운다.
/// client = 연결한 클라이언트 이름(예: "claude-code"). 빈 문자열이면 GUI가 일반 표기.
pub fn write_mcp_heartbeat(client: &str) -> Result<(), String> {
    let body = serde_json::json!({ "ts": now_secs(), "client": client });
    write_atomic(&mcp_alive_path()?, body.to_string().as_bytes())
}

/// MCP 종료 시 하트비트를 지운다 → GUI가 즉시 "연결 안 됨"으로 본다.
pub fn clear_mcp_heartbeat() {
    if let Ok(p) = mcp_alive_path() {
        let _ = fs::remove_file(p);
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 하트비트가 신선한지(순수 함수 — 테스트용).
fn is_fresh(now: u64, beat: u64) -> bool {
    now.saturating_sub(beat) <= ALIVE_SECS
}

/// GUI 앱이 최근에 살아있었는지. 결제 요청을 띄울 사람이 있는지 확인용.
/// 「살아 있다」 = 프로세스가 있다가 아니라 **여기서 사람이 승인까지 할 수 있다** → 화면이
/// 죽은 상태(`ui_ok:false`)는 살아 있는 걸로 치지 않는다(개발 51).
pub fn app_alive() -> bool {
    read_heartbeat().is_some_and(|h| h.ui_ok)
}

/// 신선한 하트비트(없거나 낡았으면 None). 이유를 갈라 안내하려고 `ui_ok` 까지 돌려준다.
fn read_heartbeat() -> Option<Heartbeat> {
    let h: Heartbeat = heartbeat_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())?;
    is_fresh(now_secs(), h.ts).then_some(h)
}

/// **지금 켜져 있는 앱이 이 결제 방식을 아는가** (개발 64). 모르면 요청을 아예 쓰지 않는다 —
/// 옛 앱에 `x402-direct` 를 보내면 그쪽은 그걸 평범한 송금으로 처리한다(서버가 알아볼 수 없는
/// 전송이 나가고 돈만 없어진다). 하트비트가 없으면(앱 꺼짐) false — 그 경우는 호출자가 이미
/// `app_alive()` 로 갈라 안내한다.
pub fn app_supports(kind: &str) -> bool {
    read_heartbeat().is_some_and(|h| h.kinds.iter().any(|k| k == kind))
}

/// 앱은 떠 있는데 **화면(WebView)이 죽어** 승인 창을 못 띄우는 상태인가 (개발 51).
/// 「앱을 켜세요」와 「앱을 다시 시작하세요」는 사용자가 할 일이 다르므로 갈라서 안내한다.
pub fn ui_stalled() -> bool {
    read_heartbeat().is_some_and(|h| !h.ui_ok)
}

/// 이미 대기 중인 요청이 있는지 (single-flight 가드).
pub fn has_pending() -> bool {
    request_path().map(|p| p.exists()).unwrap_or(false)
}

/// 새 요청 id — 유닉스 나노초. 로컬 single-flight 환경엔 충분히 고유하다.
fn new_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        .to_string()
}

/// 온체인 송금 요청을 파일에 쓴다 (kind="transfer"). 반환된 id로 결과를 매칭한다.
/// `agent` = ERC-8004 대조 결과(개발 51, 번호를 준 경우에만). `resource` 는 빈 문자열 —
/// 송금엔 요청 URL 이라는 게 없어 도메인 대조가 성립하지 않는다(승인 창도 그렇게 읽는다).
pub fn write_request_agent(
    token: &str,
    to: &str,
    amount: &str,
    memo: &str,
    agent: Option<AgentTrust>,
) -> Result<(String, Option<AgentTrust>), String> {
    write_request_kind(token, to, amount, memo, "transfer", "", "", agent)
}

/// x402 결제 서명 요청을 파일에 쓴다 (kind="x402", USDC 고정).
/// amount 는 십진 USDC 문자열, resource 는 결제 대상 URL.
pub fn write_x402_request(
    to: &str,
    amount: &str,
    memo: &str,
    resource: &str,
    agent: Option<AgentTrust>,
) -> Result<(String, Option<AgentTrust>), String> {
    write_request_kind("USDC", to, amount, memo, "x402", resource, "", agent)
}

/// x402 **직접 제출** 요청 (개발 64) — 서명만 받는 게 아니라 **온체인 전송까지** GUI 에 맡긴다.
/// GUI 는 이 kind 를 송금과 같은 것으로 다룬다(가스 여유분·내역 "sent"·tx 해시 반환).
pub fn write_x402_direct_request(
    to: &str,
    amount: &str,
    memo: &str,
    resource: &str,
    nonce: &str,
    agent: Option<AgentTrust>,
) -> Result<(String, Option<AgentTrust>), String> {
    write_request_kind(
        "USDC",
        to,
        amount,
        memo,
        "x402-direct",
        resource,
        nonce,
        agent,
    )
}

/// 공통 요청 작성기 — kind/resource 만 다르고 나머지 single-flight 로직은 동일.
#[allow(clippy::too_many_arguments)]
/// 반환: (요청 id, **실제로 요청에 실린** 대조 결과). 두 번째 값을 돌려주는 이유 —
/// 아래 체인 필터가 대조를 버릴 수 있는데, 호출자가 필터 전 값을 그대로 응답에 실으면
/// **승인 창에도 안 뜨고 자율 차단에도 안 쓰인 대조**를 AI 에게 사실처럼 말하게 된다
/// (코덱스 개발51 1차 P2). 요청에 실린 것과 응답에 실리는 것이 같아야 한다.
fn write_request_kind(
    token: &str,
    to: &str,
    amount: &str,
    memo: &str,
    kind: &str,
    resource: &str,
    nonce: &str,
    agent: Option<AgentTrust>,
) -> Result<(String, Option<AgentTrust>), String> {
    let id = new_id();
    let chain_id = crate::chain::active_chain().chain_id;
    // 활성 계정 각인 (개발 54). 여기서 못 읽으면(지갑 파일 없음·깨짐) 요청을 만들지 않는다 —
    // 어느 계정에서 나갈지 모르는 결제를 사람 앞에 띄우지 않는다.
    let account = crate::wallet::active_account()?;
    // 조회 시점과 요청 각인 시점 사이(조회 상한 10초)에 사용자가 네트워크를 바꿨을 수 있다.
    // 다른 체인에서 읽은 대조를 이번 체인 결제에 붙이면, GUI 는 request.chain_id 만 검사하므로
    // **옛 체인 사실이 이번 결제의 사실인 양** 표시되고 자율 차단 판단에까지 쓰인다
    // (코덱스 개발47 3차 P2). 그런 대조는 버린다 — 줄이 안 붙을 뿐 결제는 그대로 간다.
    let agent = agent.filter(|a| a.chain_id == chain_id);
    let req = PaymentRequest {
        id: id.clone(),
        token: token.to_string(),
        to: to.to_string(),
        amount: amount.to_string(),
        memo: memo.to_string(),
        created: now_secs(),
        kind: kind.to_string(),
        nonce: nonce.to_string(),
        resource: resource.to_string(),
        chain_id,               // 요청 시점 활성 체인 각인(승인 시 GUI가 대조)
        account: account.index, // 요청 시점 활성 계정 각인(승인 시 GUI가 대조, 개발 54)
        from: account.address,
        agent: agent.clone(),
    };
    let json = serde_json::to_string_pretty(&req)
        .map_err(|e| tf!("직렬화 실패: {e}", "Couldn't serialize the request: {e}"))?;

    // 원자적 single-flight 획득: 요청 파일을 create_new(O_EXCL)로 만든다. 이미 있으면(대기 중) 거절.
    // has_pending() 사전검사와 파일 쓰기 사이의 경합(동시 호출 둘 다 통과해 한쪽 유실)을 닫는다.
    claim_request_file(&request_path()?, json.as_bytes())?;

    // 슬롯을 확보한 뒤에야 이전 요청의 결과 파일 잔재를 치운다(새 폴링이 옛 결과를 잡지 않게).
    if let Ok(p) = result_path() {
        let _ = fs::remove_file(p);
    }
    Ok((id, agent))
}

/// 요청 파일을 create_new(O_EXCL)로 원자적으로 만들어 single-flight 슬롯을 획득한다.
/// 이미 존재하면(다른 요청이 대기 중) AlreadyExists → 사용자에게 안내.
fn claim_request_file(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = match opts.open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(ts!("이미 승인 대기 중인 결제가 있어요. 먼저 처리한 뒤 다시 요청하세요.", "A payment is already waiting for approval. Let the user handle it, then ask again.").into());
        }
        Err(e) => {
            return Err(tf!(
                "요청 파일 생성 실패: {e}",
                "Couldn't create the request file: {e}"
            ))
        }
    };
    // 쓰기 실패 시 부분 파일을 반드시 치운다 — 안 그러면 has_pending()=true 인데 GUI 는 파싱 못 해
    // None 으로 보는 영구 wedge(single-flight 가 영영 막힘)가 된다.
    if let Err(e) = f.write_all(bytes) {
        drop(f);
        let _ = fs::remove_file(path);
        return Err(tf!(
            "요청 파일 저장 실패: {e}",
            "Couldn't write the request file: {e}"
        ));
    }
    Ok(())
}

/// 내 id와 일치하는 결과를 읽는다.
fn read_result(id: &str) -> Option<PaymentResult> {
    result_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<PaymentResult>(&s).ok())
        .filter(|r| r.id == id)
}

/// 타임아웃 시 내 요청 파일을 치운다 (다른 요청이 덮어쓴 경우는 건드리지 않음).
pub fn cancel_request(id: &str) {
    if let Ok(p) = request_path() {
        cancel_request_at(&p, id);
    }
}

/// `cancel_request` 의 속알맹이 — 경로를 받는다. **경로를 안에서 구하면 테스트가 실지갑
/// (`~/.jigap`)을 건드리게 되므로** 여기를 갈라 두고 테스트는 임시 폴더를 넘긴다
/// (`claim_request_file` 과 같은 이음매). 파일 안의 id 가 내 것일 때만 지운다.
fn cancel_request_at(path: &Path, id: &str) {
    let mine = fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<PaymentRequest>(&s).ok())
        .map(|r| r.id == id)
        .unwrap_or(false);
    if mine {
        let _ = fs::remove_file(path);
    }
}

/// 대기 중에 이 프로세스가 사라지면 내 요청을 거둔다 (개발 59 에 넣었다 되돌리고, 개발 63 에 복원).
///
/// AI 클라이언트(Claude 등)가 승인 대기 중에 죽으면 stdin 이 닫혀 MCP 서버도 따라 죽는다
/// (개발 59 실측: 1초 안에). 그때 요청 파일을 그냥 두면 **기다리는 사람이 아무도 없는 승인 창**이
/// 최대 6분(GUI `is_stale` 의 5분 + 유예 60초) 화면에 남는다. 사용자가 그걸 승인하면 돈은 나가는데
/// 결과를 받을 상대가 없고, 다시 켠 AI 는 「아까 결제가 실패했다」고 알고 있어 한 번 더 요청한다
/// → 이중 결제(개발 51 과 같은 모양). 요청을 거두면 승인 창이 곧바로 닫히고(GUI 의 `live_request()`
/// 가 파일에서 파생된다) 아무 일도 안 난다.
///
/// 시간 초과 때 호출자가 하던 것과 **같은 함수**를 부른다 — `cancel_request` 는 파일 안의 id 가
/// 내 것일 때만 지우므로 남의 요청은 건드리지 않는다.
///
/// **거두기가 못 하는 일과, 그래도 안전한 이유** (개발 59 코덱스 2차 P1 → 개발 63 에서 닫음):
/// GUI 가 이미 `begin_approval` 을 지나 서명·전송 중이면 이 삭제는 그 결제를 멈추지 못한다 —
/// 돈은 나가고 GUI 의 `resolve_request` 는 요청이 사라진 걸 보고 결과를 조용히 버린다(죽은
/// 클라이언트는 어느 쪽이든 결과를 못 받는다). 문제는 그 사이 슬롯이 비어 **새 요청 B 가
/// 생길 수 있다**는 것이었다(전송은 RPC 가 느리면 30초까지). 개발 59 엔 GUI 가 B 의 승인을
/// A 와 겹쳐 시작할 수 있어 자율 승인이면 사람 없이 두 건이 나갔다 → 되돌렸다. 개발 63 부터
/// GUI 의 `begin_approval` 이 **진행 중인 승인이 있으면 두 번째 승인을 거절**하므로(ipc.rs),
/// B 는 A 가 끝날 때까지 자율로도 수동으로도 시작되지 않는다.
///
/// ⚠️ 닫히는 것은 **겹치는 창**뿐이다. A 가 끝난 **뒤** 다시 켜진 AI 가 같은 결제를 또
/// 요청하면(순차 재시도) 그건 한도·신뢰 주소·승인 창이 막는 몫이고, 이 가드와 무관하다.
///
/// SIGKILL 로 이 프로세스가 죽으면 Drop 은 안 돈다 — 그건 원래도 그랬고 막을 방법이 없다.
/// 그렇게 남은 고아 요청은 `has_pending()` 이 **존재만** 보므로 그 뒤 모든 결제를 막는데,
/// 개발 63 부터 **GUI 의 감시 스레드가 승인 창 시간(5분+유예)을 넘긴 요청 파일을 지운다**
/// (`src-tauri/src/ipc.rs` watchdog). 개발 63 이전엔 그 청소가 어디에도 없어서 — `is_stale`
/// 은 보여줄지 말지를 거르기만 한다 — 터미널에서 손으로 지우는 수밖에 없었다.
struct CancelOnDrop<'a> {
    id: &'a str,
    /// 거둘 요청 파일. 경로를 들고 있어야 테스트가 임시 폴더로 이 가드를 그대로 시험할 수
    /// 있다(`cancel_request_at` 과 같은 이음매). 실제 경로는 `await_result` 가 넘긴다.
    path: &'a Path,
    /// 정상 종료(결과 수신·시간 초과)면 내려서 Drop 이 아무것도 안 하게 한다.
    armed: bool,
}

impl Drop for CancelOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            cancel_request_at(self.path, self.id);
        }
    }
}

/// 결과를 timeout까지 폴링한다. 오면 Some(소비 후 파일 정리), 타임아웃이면 None.
///
/// 이 함수의 future 가 **완료되기 전에 버려지면**(= 클라이언트가 죽어 런타임이 내려가면)
/// 대기 중이던 요청을 거둔다 — 위 `CancelOnDrop` 참고.
pub async fn await_result(id: &str, timeout: Duration) -> Option<PaymentResult> {
    // 경로를 못 구하는 상황(홈 디렉터리 없음)은 애초에 요청도 못 썼다는 뜻이라, 거둘 것도 없다.
    let req_path = request_path().unwrap_or_default();
    let mut guard = CancelOnDrop {
        id,
        path: &req_path,
        armed: true,
    };
    let start = SystemTime::now();
    loop {
        if let Some(r) = read_result(id) {
            if let Ok(p) = result_path() {
                let _ = fs::remove_file(p);
            }
            guard.armed = false;
            return Some(r);
        }
        let elapsed = SystemTime::now().duration_since(start).unwrap_or(timeout);
        if elapsed >= timeout {
            // 시간 초과의 뒷정리는 호출자가 한다(안내 문구와 한 자리에 있다).
            guard.armed = false;
            return None;
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 🔴 개발 64 — **옛 앱의 하트비트엔 `kinds` 가 없다.** 그때 새 방식을 「지원한다」로 읽으면
    /// 옛 앱에 `x402-direct` 요청이 가고, 그 앱은 그것을 **평범한 송금으로** 처리한다(돈은 나가고
    /// 결제는 성립하지 않는다). 기본값은 그 시절 앱이 실제로 할 수 있던 둘뿐이어야 한다.
    #[test]
    fn heartbeat_kinds_default_is_the_old_apps_abilities() {
        let old: Heartbeat = serde_json::from_str(r#"{"ts":1}"#).unwrap();
        assert_eq!(old.kinds, vec!["transfer".to_string(), "x402".to_string()]);
        assert!(!old.kinds.iter().any(|k| k == "x402-direct"));
        // 새 앱이 적어 주면 그대로 읽는다.
        let new: Heartbeat = serde_json::from_str(
            r#"{"ts":1,"ui_ok":true,"kinds":["transfer","x402","x402-direct"]}"#,
        )
        .unwrap();
        assert!(new.kinds.iter().any(|k| k == "x402-direct"));
    }

    /// 결제 요청 JSON 왕복 — src-tauri가 읽는 형식과 호환돼야 한다.
    #[test]
    fn payment_request_roundtrip() {
        let r = PaymentRequest {
            id: "123".into(),
            token: "USDC".into(),
            to: "0xabc".into(),
            amount: "1.5".into(),
            memo: "데이터 API 호출".into(),
            created: 100,
            kind: "transfer".into(),
            nonce: String::new(),
            resource: String::new(),
            chain_id: 84_532,
            account: 1,
            from: "0xOne".into(),
            agent: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: PaymentRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "123");
        assert_eq!((back.account, back.from.as_str()), (1, "0xOne"));
        assert_eq!(back.token, "USDC");
        assert_eq!(back.memo, "데이터 API 호출");
        assert_eq!(back.kind, "transfer");
    }

    /// 🔴 옛 앱이 쓴 하트비트에는 `ui_ok` 가 없다 → **true 로 읽혀야** 한다 (개발 51).
    /// 기본값을 빠뜨리면 serde 가 false 로 채워 **모든 결제가 「화면이 죽었다」로 거절**된다 —
    /// 앱만 업데이트가 늦어도 지갑이 통째로 먹통이 되는 자리라 테스트로 못박는다.
    #[test]
    fn heartbeat_without_ui_ok_is_alive() {
        let h: Heartbeat = serde_json::from_str(r#"{"ts":100}"#).unwrap();
        assert!(h.ui_ok, "옛 하트비트는 「띄울 수 있다」로 읽혀야 한다");
        let h: Heartbeat = serde_json::from_str(r#"{"ts":100,"ui_ok":false}"#).unwrap();
        assert!(!h.ui_ok);
    }

    /// 기존(Session 10) 요청 파일은 kind/resource 가 없다 → default 로 채워져야 한다(무손실 호환).
    #[test]
    fn legacy_request_defaults_to_transfer() {
        let json = r#"{"id":"1","token":"USDC","to":"0xabc","amount":"1","memo":"","created":1}"#;
        let r: PaymentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(r.kind, "transfer");
        assert_eq!(r.resource, "");
        assert!(r.agent.is_none());
    }

    /// 대조 결과가 없으면 JSON 에 `agent` 키 자체가 없어야 한다 — 옛 GUI 가 읽어도
    /// 달라지는 게 없고, "조회를 했는데 결과가 비었다"와 "조회를 안 했다"가 안 섞인다.
    #[test]
    fn agent_field_is_omitted_when_absent() {
        let r = PaymentRequest {
            id: "1".into(),
            token: "USDC".into(),
            to: "0xabc".into(),
            amount: "1".into(),
            memo: String::new(),
            created: 1,
            kind: "x402".into(),
            nonce: String::new(),
            resource: "https://api.example.com/x".into(),
            chain_id: 8453,
            account: 0,
            from: String::new(),
            agent: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("agent"), "{json}");
    }

    /// 결과 파싱 — GUI가 쓴 형식. x402 필드 없는 기존 결과도 파싱돼야 한다.
    #[test]
    fn payment_result_parses() {
        let json = r#"{"id":"1","status":"approved","tx_hash":"0xhash","detail":""}"#;
        let r: PaymentResult = serde_json::from_str(json).unwrap();
        assert_eq!(r.status, "approved");
        assert_eq!(r.tx_hash, "0xhash");
        assert!(r.x402.is_none());
    }

    /// x402 결과: GUI가 서명 페이로드를 함께 돌려준다.
    #[test]
    fn x402_result_carries_payment() {
        let json = r#"{"id":"1","status":"approved","tx_hash":"","detail":"",
          "x402":{"signature":"0xsig","authorization":{"from":"0xa","to":"0xb",
          "value":"10000","validAfter":"0","validBefore":"99","nonce":"0x1"}}}"#;
        let r: PaymentResult = serde_json::from_str(json).unwrap();
        let p = r.x402.expect("x402 페이로드");
        assert_eq!(p.signature, "0xsig");
        assert_eq!(p.authorization.value, "10000");
    }

    /// 다른 체인에서 읽은 대조는 요청에 실리지 않는다 — 조회 도중 사용자가 네트워크를
    /// 바꾼 경우, 옛 체인의 사실이 이번 체인 결제의 사실인 양 보이면 안 된다(3차 P2).
    /// (테스트 환경의 활성 체인은 Base Sepolia 로 고정된다 — chain.rs 의 cfg(test).)
    #[test]
    fn agent_from_another_chain_is_dropped() {
        let here = crate::chain::active_chain().chain_id;
        let mk = |chain_id: u64| AgentTrust {
            agent_id: 1,
            chain_id,
            registered: true,
            wallet: "0xB0b".into(),
            wallet_check: "match".into(),
            uri_domain: "api.example.com".into(),
            resource_domain: "api.example.com".into(),
            domain_check: "match".into(),
            feedback_clients: None,
        };
        // 같은 체인 = 그대로 실린다 / 다른 체인 = 버린다.
        assert!(Some(mk(here)).filter(|a| a.chain_id == here).is_some());
        assert!(Some(mk(here + 1)).filter(|a| a.chain_id == here).is_none());
    }

    /// 하트비트 신선도: 10초 이내면 살아있음, 넘으면 죽음.
    #[test]
    fn heartbeat_freshness() {
        assert!(is_fresh(1000, 1000)); // 같은 순간
        assert!(is_fresh(1010, 1000)); // 10초 경과(경계)
        assert!(!is_fresh(1011, 1000)); // 11초 → 만료
        assert!(is_fresh(1000, 2000)); // 시계 역전도 살아있음으로(saturating)
    }

    /// 요청 id는 비어있지 않다.
    #[test]
    fn new_id_is_nonempty() {
        assert!(!new_id().is_empty());
    }

    /// single-flight 원자 획득: 첫 claim 은 성공, 파일이 남아 있는 동안 두 번째 claim 은 거절.
    /// 첫 내용은 덮어쓰이지 않는다(둘 다 통과해 한쪽 유실되던 경합 방지).
    #[test]
    fn claim_request_is_single_flight() {
        let dir = std::env::temp_dir().join(format!("kura-mcp-claim-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("payment_request.json");
        let _ = fs::remove_file(&path);

        assert!(claim_request_file(&path, b"first").is_ok());
        assert!(claim_request_file(&path, b"second").is_err()); // 이미 대기 중 → 거절
        assert_eq!(fs::read_to_string(&path).unwrap(), "first"); // 첫 내용 보존

        // 처리 후(파일 제거) 다시 획득 가능.
        let _ = fs::remove_file(&path);
        assert!(claim_request_file(&path, b"third").is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    /// 요청 하나를 그 경로에 써 둔다(테스트 도우미).
    fn write_req(path: &Path, id: &str) {
        let r = PaymentRequest {
            id: id.into(),
            token: "USDC".into(),
            to: "0x0".into(),
            amount: "1".into(),
            memo: String::new(),
            created: 0,
            kind: "transfer".into(),
            nonce: String::new(),
            resource: String::new(),
            chain_id: 0,
            account: 0,
            from: String::new(),
            agent: None,
        };
        fs::write(path, serde_json::to_string(&r).unwrap()).unwrap();
    }

    /// 🔴 거두기는 **내 요청일 때만** 지운다 (개발 59·63). 남의 요청을 지우면 그쪽의
    /// single-flight 슬롯이 깨져 겹친 결제가 생긴다.
    #[test]
    fn cancel_only_removes_my_request() {
        let dir = std::env::temp_dir().join(format!("kura-mcp-cancel-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("payment_request.json");

        write_req(&path, "A");
        cancel_request_at(&path, "B"); // 남의 요청 → 안 건드린다
        assert!(path.exists(), "다른 id 의 요청은 남아야 한다");
        cancel_request_at(&path, "A"); // 내 요청 → 지운다
        assert!(!path.exists(), "내 요청은 거둬야 한다");

        cancel_request_at(&path, "A"); // 이미 없는 경우도 조용히 통과
        let _ = fs::remove_dir_all(&dir);
    }

    /// 🔴 **대기 중에 future 가 버려지면 요청을 거둔다** (`CancelOnDrop`, 개발 63 복원).
    /// 클라이언트가 죽으면 런타임이 내려가며 이 future 가 버려지는데, 그때 요청 파일이 남으면
    /// 「아무도 안 기다리는 승인 창」이 뜬다. 결과를 받았거나 시간 초과로 정상 반환한 경우엔
    /// **거두면 안 된다**(그 뒷정리는 호출자 몫이고, 결과를 받은 요청은 이미 남의 것일 수 있다).
    #[test]
    fn drop_while_waiting_cancels_but_normal_return_does_not() {
        let dir = std::env::temp_dir().join(format!("kura-mcp-drop-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("payment_request.json");

        // 대기 중 버려짐 → 거둔다.
        write_req(&path, "A");
        {
            let _g = CancelOnDrop {
                id: "A",
                path: &path,
                armed: true,
            };
        }
        assert!(!path.exists(), "대기 중 버려지면 요청을 거둔다");

        // 정상 반환(결과 수신·시간 초과)에서 내려 둔 가드는 아무것도 안 한다.
        write_req(&path, "A");
        {
            let mut g = CancelOnDrop {
                id: "A",
                path: &path,
                armed: true,
            };
            g.armed = false;
        }
        assert!(path.exists(), "정상 반환이면 거두지 않는다");
        let _ = fs::remove_dir_all(&dir);
    }
}
