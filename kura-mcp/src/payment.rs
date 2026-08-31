// 결제 요청 IPC (개발 9, Session 10) — MCP가 결제를 "요청"하고 GUI 승인을 기다린다.
//
// 비번은 절대 여기로 들어오지 않는다. MCP는 ~/.jigap에 요청 파일만 쓰고, 실제 서명·전송은
// GUI 앱(src-tauri)이 한다. 흐름:
//   write_request() → GUI가 팝업으로 사람 승인 → GUI가 결과 파일 작성 → await_result()가 읽어 반환.
//
// single-flight: 한 번에 대기 요청 1건. 앱이 안 켜져 있으면(하트비트 신선도) 즉시 안내.

use crate::chain::chain_file;
use crate::erc8004::AgentTrust;
use crate::wallet::jigap_dir;
use crate::{tf, ts};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
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
    /// "transfer"(온체인 송금, 기본) | "x402"(EIP-3009 오프체인 서명).
    /// 기존 요청 파일 호환을 위해 default = "transfer".
    #[serde(default = "default_kind")]
    pub kind: String,
    /// x402일 때 결제 대상 리소스 URL (사용자가 팝업에서 본다). transfer면 빈 문자열.
    #[serde(default)]
    pub resource: String,
    /// 요청 생성 시점의 활성 체인 ID — GUI가 승인 시 현재 체인과 다르면 거부한다(코덱스 개발20 #2).
    #[serde(default)]
    pub chain_id: u64,
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

fn now_secs() -> u64 {
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
pub fn write_request(token: &str, to: &str, amount: &str, memo: &str) -> Result<String, String> {
    write_request_kind(token, to, amount, memo, "transfer", "", None)
}

/// 온체인 송금 요청 + ERC-8004 대조 결과 (개발 51). `resource` 는 빈 문자열 — 송금엔
/// 요청 URL 이라는 게 없어 도메인 대조가 성립하지 않는다(승인 창도 그렇게 읽는다).
pub fn write_request_agent(
    token: &str,
    to: &str,
    amount: &str,
    memo: &str,
    agent: Option<AgentTrust>,
) -> Result<String, String> {
    write_request_kind(token, to, amount, memo, "transfer", "", agent)
}

/// x402 결제 서명 요청을 파일에 쓴다 (kind="x402", USDC 고정).
/// amount 는 십진 USDC 문자열, resource 는 결제 대상 URL.
pub fn write_x402_request(
    to: &str,
    amount: &str,
    memo: &str,
    resource: &str,
    agent: Option<AgentTrust>,
) -> Result<String, String> {
    write_request_kind("USDC", to, amount, memo, "x402", resource, agent)
}

/// 공통 요청 작성기 — kind/resource 만 다르고 나머지 single-flight 로직은 동일.
#[allow(clippy::too_many_arguments)]
fn write_request_kind(
    token: &str,
    to: &str,
    amount: &str,
    memo: &str,
    kind: &str,
    resource: &str,
    agent: Option<AgentTrust>,
) -> Result<String, String> {
    let id = new_id();
    let chain_id = crate::chain::active_chain().chain_id;
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
        resource: resource.to_string(),
        chain_id, // 요청 시점 활성 체인 각인(승인 시 GUI가 대조)
        agent,
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
    Ok(id)
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
    let mine = request_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<PaymentRequest>(&s).ok())
        .map(|r| r.id == id)
        .unwrap_or(false);
    if mine {
        if let Ok(p) = request_path() {
            let _ = fs::remove_file(p);
        }
    }
}

/// 결과를 timeout까지 폴링한다. 오면 Some(소비 후 파일 정리), 타임아웃이면 None.
pub async fn await_result(id: &str, timeout: Duration) -> Option<PaymentResult> {
    let start = SystemTime::now();
    loop {
        if let Some(r) = read_result(id) {
            if let Ok(p) = result_path() {
                let _ = fs::remove_file(p);
            }
            return Some(r);
        }
        let elapsed = SystemTime::now().duration_since(start).unwrap_or(timeout);
        if elapsed >= timeout {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            resource: String::new(),
            chain_id: 84_532,
            agent: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: PaymentRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "123");
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
            resource: "https://api.example.com/x".into(),
            chain_id: 8453,
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
}
