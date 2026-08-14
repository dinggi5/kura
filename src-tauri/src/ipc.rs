// 결제 승인 IPC (Session 10) + AI 연결 배지 (개발 9).
//
// AI 에이전트가 MCP로 결제를 "요청"하면, 별도 프로세스인 MCP 서버가 ~/.jigap에 요청 파일을
// 쓴다. GUI 앱(이 프로세스)이 1초마다 폴링해 발견 → 승인 팝업 → 사용자가 비번 입력 → 실제
// 송금 → 결과 파일 작성 → MCP가 읽어 에이전트에 반환.
//
// 핵심 보안: 비번은 절대 요청/결과 파일이나 MCP에 들어가지 않는다. 키 접근(서명)은 오직 이
// GUI 프로세스만 한다 → MCP는 "요청"만, 승인은 사람이. 잠금·한도·내역은 기존 send 경로를
// 그대로 재사용하므로 자동 적용된다.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

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
}

fn default_kind() -> String {
    "transfer".to_string()
}

/// 대기 요청이 만들어진 체인과 현재 활성 체인이 같은지 검사한다(코덱스 개발20 #2). 다르면 거부 —
/// 승인/자율 경로 공용. chain_id 0 = 옛 미각인 요청이므로 검사를 건너뛴다(후방호환).
pub(crate) fn ensure_request_chain(req: &PaymentRequest) -> Result<(), String> {
    if req.chain_id != 0 && req.chain_id != crate::chain::active_chain().chain_id {
        return Err(
            "결제 요청이 만들어진 네트워크와 현재 네트워크가 달라요 — 네트워크를 확인하고 다시 요청하세요."
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
pub(crate) fn resolve_request(result: &PaymentResult) -> Result<(), String> {
    write_json(result_path()?, result)?;
    if let Ok(p) = request_path() {
        let _ = fs::remove_file(p);
    }
    Ok(())
}

/// GUI가 1초마다 폴링한다. 대기 요청을 돌려주고 하트비트를 갱신한다(앱 생존 표시).
/// 하트비트 디스크 쓰기는 2초에 한 번이면 충분하다 (MCP 신선도 기준 10초 = 5배 여유).
#[tauri::command]
pub(crate) fn get_pending_request(app: tauri::AppHandle) -> Option<PaymentRequest> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST_BEAT: AtomicU64 = AtomicU64::new(0);
    let now = now_secs();
    if now.saturating_sub(LAST_BEAT.load(Ordering::Relaxed)) >= 2 {
        LAST_BEAT.store(now, Ordering::Relaxed);
        let _ = write_json(heartbeat_path().unwrap_or_default(), &Heartbeat { ts: now });
    }
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
    let req = read_request().ok_or("대기 중인 결제 요청이 없습니다")?;
    if req.id != id {
        return Err("요청 ID가 일치하지 않습니다".into());
    }
    ensure_request_chain(&req)?; // 요청 시점 체인 ≠ 현재 체인이면 거부(메인넷 오발사 차단)

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
                other => Err(format!("지원하지 않는 토큰: {other}")),
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
    let req = read_request().ok_or("대기 중인 결제 요청이 없습니다")?;
    if req.id != id {
        return Err("요청 ID가 일치하지 않습니다".into());
    }
    resolve_request(&PaymentResult {
        id: req.id,
        status: "rejected".into(),
        tx_hash: String::new(),
        detail: reason.unwrap_or_else(|| "사용자가 거부했습니다".into()),
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
        }
    }

    // 방금 만들어진 요청은 살아 있다 = 팝오버를 붙잡아야 한다.
    #[test]
    fn fresh_request_is_not_stale() {
        assert!(!is_stale(&req_created(now_secs())));
    }

    // 승인 시간(5분) 안이면 아직 살아 있다.
    #[test]
    fn request_within_window_is_not_stale() {
        assert!(!is_stale(&req_created(now_secs() - (APPROVAL_WINDOW_SECS - 10))));
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
        let json =
            r#"{"id":"1","token":"USDC","to":"0xabc","amount":"1","memo":"","created":1}"#;
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
