// 읽기 전용 지갑 조회 — ~/.jigap 파일을 읽고 Base Sepolia RPC로 잔액을 본다.
//
// 비밀(니모닉/키)은 절대 건드리지 않는다. wallet.enc 의 평문 address 필드만 읽고,
// 잔액은 공개 RPC로 조회한다. → 비번 없이도 안전하게 노출 가능한 정보들.
//
// ⚠️ 체인 기본값(RPC·USDC 주소·decimals)은 chain.rs 의 active_chain() 에서 온다. 실제 RPC는
//    effective_rpc() 가 settings.json(GUI와 공유)을 읽어 결정하므로 사용자가 RPC를 바꾸면
//    두 프로세스가 자동으로 같은 RPC를 쓴다. default_rpc 는 설정이 비었을 때의 폴백.

use crate::{tf, ts};
use alloy::primitives::{
    utils::{format_ether, format_units},
    Address, U256,
};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::sol;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::chain::{active_chain, chain_file};

sol! {
    #[sol(rpc)]
    interface IERC20 {
        function balanceOf(address owner) external view returns (uint256);
    }
}

pub fn jigap_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or(ts!(
        "홈 디렉터리를 찾을 수 없습니다",
        "Couldn't find the home folder"
    ))?;
    Ok(home.join(".jigap"))
}

fn enc_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join("wallet.enc"))
}

fn legacy_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join("wallet.json"))
}

fn history_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join(chain_file("history")))
}

fn settings_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join("settings.json"))
}

/// settings.json 에서 RPC 만 읽는다(다른 필드는 무시). src-tauri 와 같은 파일을 공유한다.
#[derive(Deserialize, Default)]
struct RpcSettings {
    #[serde(default)]
    rpc_url: String,
}

/// settings 의 rpc_url 과 활성 체인의 기본 RPC 중 무엇을 쓸지 — 순수 판정(테스트용).
///
/// `forced_other_chain` = `KURA_CHAIN_ID` 가 settings 와 **다른** 체인을 강제한 상태.
/// 그때 settings 의 rpc_url 은 **딴 체인의 엔드포인트**다 — 그대로 쓰면 이 체인의 컨트랙트를
/// 저쪽 체인에 물어 잔액이 `returned no data ("0x")` 로 죽는다(개발 48 실측 → 개발 49 수정).
/// 커스텀 RPC 를 조용히 버리는 셈이지만, 대안이 "조용히 안 되는 것"이라 이쪽이 낫다.
fn pick_rpc(custom: &str, forced_other_chain: bool, default_rpc: &str) -> String {
    if custom.is_empty() || forced_other_chain {
        default_rpc.to_string()
    } else {
        custom.to_string()
    }
}

/// 사용자가 설정한 RPC를 돌려준다. 없거나 비어 있으면 활성 체인의 공식 RPC(default_rpc)로 폴백.
/// GUI(src-tauri)와 동일한 settings.json 을 읽으므로 두 프로세스의 RPC가 자동으로 일치한다.
/// 단, 환경변수로 체인을 갈아탄 경우엔 그 rpc_url 이 딴 체인 것이므로 쓰지 않는다(pick_rpc).
pub fn effective_rpc() -> String {
    let url = settings_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<RpcSettings>(&s).ok())
        .map(|s| s.rpc_url.trim().to_string())
        .unwrap_or_default();
    pick_rpc(
        &url,
        crate::chain::env_forces_other_chain(),
        active_chain().default_rpc,
    )
}

/// AI(MCP)/CLI 로 나가는 에러·로그 문자열에서 URL 을 통째로 `[RPC]` 로 가린다.
/// 커스텀 RPC 경로·쿼리엔 API 키가 들어가곤 한다(예: alchemy `…/v2/KEY`) → 그대로 두면 LLM 채팅에 샌다.
/// **설정을 다시 읽지 않고 문자열에 보이는 URL 자체를 가린다** → 설정 변경·host 대소문자 정규화·
/// ws/wss 등에 흔들리지 않는다(코덱스 리뷰 반영). URL 외 문자는 그대로 둔다.
/// (src-tauri settings.rs 와 의도적 중복 — 공유 크레이트 안 만드는 정책.)
pub fn redact_urls(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(rel) = input[cursor..].find("://") {
        let sep = cursor + rel;
        if let Some(start) = scheme_start(input, cursor, sep) {
            out.push_str(&input[cursor..start]);
            let token = &input[start..];
            let end = token
                .find(|c: char| {
                    c.is_whitespace() || c.is_control() || matches!(c, '"' | '<' | '>' | '`')
                })
                .unwrap_or(token.len());
            out.push_str("[RPC]");
            cursor = start + end;
        } else {
            out.push_str(&input[cursor..sep + 3]);
            cursor = sep + 3;
        }
    }
    out.push_str(&input[cursor..]);
    out
}

/// `sep`(="://"의 시작 인덱스) 앞에서 scheme 시작을 찾는다. scheme = `[A-Za-z][A-Za-z0-9+.-]*`.
/// `min` 미만으로 안 내려간다. 유효 scheme 없으면 None.
fn scheme_start(s: &str, min: usize, sep: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut start = sep;
    while start > min {
        let c = bytes[start - 1];
        if c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.') {
            start -= 1;
        } else {
            break;
        }
    }
    if start < sep && bytes[start].is_ascii_alphabetic() {
        Some(start)
    } else {
        None
    }
}

/// 지갑 상태 + 주소. 프론트/에이전트가 어떤 상태인지 알 수 있게.
/// - "encrypted": 정상 (wallet.enc 존재)
/// - "legacy":    평문 wallet.json 만 존재 (앱에서 비번 설정 필요)
/// - "none":      지갑 없음
#[derive(Serialize)]
pub struct WalletStatus {
    pub state: String,
    pub address: Option<String>,
    pub backed_up: bool,
}

/// wallet.enc 에서 공개 정보(주소·백업여부)만 읽는다. 니모닉(ciphertext)은 무시.
#[derive(Deserialize)]
struct EncMeta {
    address: String,
    #[serde(default)]
    backed_up: bool,
}

#[derive(Deserialize)]
struct LegacyMeta {
    address: String,
}

/// 지갑 파일 상태를 알려준다 (비번 불필요).
pub fn wallet_status() -> Result<WalletStatus, String> {
    if enc_path()?.exists() {
        let data = fs::read_to_string(enc_path()?).map_err(|e| {
            tf!(
                "지갑 파일 읽기 실패: {e}",
                "Couldn't read the wallet file: {e}"
            )
        })?;
        let m: EncMeta = serde_json::from_str(&data).map_err(|e| {
            tf!(
                "지갑 파일 파싱 실패: {e}",
                "Couldn't parse the wallet file: {e}"
            )
        })?;
        return Ok(WalletStatus {
            state: "encrypted".into(),
            address: Some(m.address),
            backed_up: m.backed_up,
        });
    }
    if legacy_path()?.exists() {
        let data = fs::read_to_string(legacy_path()?).map_err(|e| {
            tf!(
                "지갑 파일 읽기 실패: {e}",
                "Couldn't read the wallet file: {e}"
            )
        })?;
        let m: LegacyMeta = serde_json::from_str(&data).map_err(|e| {
            tf!(
                "지갑 파일 파싱 실패: {e}",
                "Couldn't parse the wallet file: {e}"
            )
        })?;
        return Ok(WalletStatus {
            state: "legacy".into(),
            address: Some(m.address),
            backed_up: false,
        });
    }
    Ok(WalletStatus {
        state: "none".into(),
        address: None,
        backed_up: false,
    })
}

/// 잔액 — 십진수 문자열.
///
/// `eth` = **네이티브(가스) 토큰이 USDC 와 다른 자산인 체인에서만 있다** (개발 50). Arc 처럼
/// 네이티브가 곧 USDC 인 체인에선 아예 안 내보낸다 — 같은 잔액의 18dp 뷰를 나란히 주면 읽는 쪽
/// (여기선 AI 모델)이 **같은 돈을 두 몫으로 센다**. src-tauri/src/transfer.rs 의 Balances 와 같은 규칙.
#[derive(Serialize)]
pub struct Balances {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eth: Option<String>,
    pub usdc: String,
}

/// 지갑 주소의 네이티브(가스) + USDC(결제) 잔액을 활성 체인에서 조회한다.
pub async fn get_balances(addr_hex: &str) -> Result<Balances, String> {
    let addr: Address = addr_hex
        .parse()
        .map_err(|e| tf!("주소 파싱 실패: {e}", "Couldn't read that address: {e}"))?;

    let provider = ProviderBuilder::new()
        .connect(&effective_rpc())
        .await
        .map_err(|e| {
            tf!(
                "RPC 연결 실패: {}",
                "Couldn't reach the RPC server: {}",
                redact_urls(&e.to_string())
            )
        })?;

    let usdc_contract = IERC20::new(active_chain().usdc_address, &provider);
    let (wei, raw): (Option<U256>, U256) = tokio::try_join!(
        async {
            // 네이티브가 곧 USDC 인 체인(Arc)에선 네이티브 조회를 아예 건너뛴다 — 같은 잔액이다.
            if active_chain().native_is_usdc {
                return Ok(None);
            }
            provider.get_balance(addr).await.map(Some).map_err(|e| {
                tf!(
                    "ETH 잔액 조회 실패: {}",
                    "Couldn't read the ETH balance: {}",
                    redact_urls(&e.to_string())
                )
            })
        },
        async {
            usdc_contract.balanceOf(addr).call().await.map_err(|e| {
                tf!(
                    "USDC 잔액 조회 실패: {}",
                    "Couldn't read the USDC balance: {}",
                    redact_urls(&e.to_string())
                )
            })
        },
    )?;

    Ok(Balances {
        eth: wei.map(format_ether),
        usdc: format_units(raw, active_chain().usdc_decimals).map_err(|e| {
            tf!(
                "USDC 단위 변환 실패: {e}",
                "Couldn't convert the USDC amount: {e}"
            )
        })?,
    })
}

/// 거래 내역 1건 (감사 로그). src-tauri 가 기록한 history.json 을 그대로 읽는다.
#[derive(Serialize, Deserialize, Clone)]
pub struct HistoryEntry {
    pub ts: u64,
    pub token: String,
    pub to: String,
    pub amount: String,
    pub status: String,
    pub detail: String,
    /// x402 정산 tx 해시 (status가 settled/settle_failed일 때). 옛 기록엔 없어서 기본 빈 문자열.
    #[serde(default)]
    pub settle_tx: String,
}

/// 저장된 거래 내역을 읽는다 (최신순). 없거나 깨졌으면 빈 목록.
/// detail 은 출력 시점에 redact — 이번 패치 이전(또는 다른 빌드)이 기록한 비redact 에러에
/// RPC URL·키가 들어 있어도 get_history(→AI)·CLI 로 다시 새지 않게 한다(코덱스 High 반영).
pub fn read_history() -> Vec<HistoryEntry> {
    let mut entries: Vec<HistoryEntry> = history_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    for e in &mut entries {
        e.detail = redact_urls(&e.detail);
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RPC 선택 (개발 49). 환경변수로 체인을 갈아탄 경우엔 settings 의 커스텀 RPC 를 버린다 —
    /// 그 URL 은 **딴 체인의 엔드포인트**라 그대로 쓰면 잔액 조회가 조용히 죽는다(개발 48 실측).
    #[test]
    fn pick_rpc_drops_custom_when_env_forces_other_chain() {
        let custom = "https://base-mainnet.example/v2/KEY";
        let default = "https://sepolia.base.org";
        // 평소: 커스텀이 있으면 커스텀.
        assert_eq!(pick_rpc(custom, false, default), custom);
        // 커스텀이 비면 언제나 기본값.
        assert_eq!(pick_rpc("", false, default), default);
        assert_eq!(pick_rpc("", true, default), default);
        // 환경변수가 다른 체인을 강제 → 커스텀을 버리고 그 체인의 기본 RPC 로.
        assert_eq!(pick_rpc(custom, true, default), default);
    }

    /// 핵심 계약: wallet.enc 를 읽을 때 주소만 가져오고 ciphertext(니모닉)는 무시한다.
    /// EncMeta 에 ciphertext 필드 자체가 없으므로 역직렬화 결과에 비밀이 들어올 수 없다.
    #[test]
    fn enc_meta_reads_address_and_ignores_secrets() {
        let json = r#"{
            "version": 2,
            "address": "0x8b7ba5077d261739f5FeBB31B10167671e590161",
            "salt": "c2FsdA==",
            "nonce": "bm9uY2U=",
            "ciphertext": "c2VjcmV0LW1uZW1vbmlj",
            "backed_up": true
        }"#;
        let m: EncMeta = serde_json::from_str(json).unwrap();
        assert_eq!(m.address, "0x8b7ba5077d261739f5FeBB31B10167671e590161");
        assert!(m.backed_up);
        // EncMeta 구조체엔 ciphertext/salt/nonce 필드가 없다 → 컴파일 타임에 비밀 차단.
    }

    /// 옛 파일에 backed_up 필드가 없어도 기본값 false 로 로드된다.
    #[test]
    fn enc_meta_defaults_backed_up_false() {
        let json = r#"{"version":2,"address":"0xabc","salt":"x","nonce":"y","ciphertext":"z"}"#;
        let m: EncMeta = serde_json::from_str(json).unwrap();
        assert!(!m.backed_up);
    }

    /// 평문 legacy 파일에서도 주소만 읽는다 (니모닉 필드는 무시).
    #[test]
    fn legacy_meta_reads_only_address() {
        let json = r#"{"mnemonic":"word1 word2 ...","address":"0xdef"}"#;
        let m: LegacyMeta = serde_json::from_str(json).unwrap();
        assert_eq!(m.address, "0xdef");
    }

    /// 거래 내역 항목이 src-tauri 가 쓴 형식과 호환되게 역직렬화된다.
    #[test]
    fn history_entry_roundtrips() {
        let json = r#"{"ts":1780623842,"token":"USDC","to":"0xabc","amount":"1","status":"sent","detail":"0xhash"}"#;
        let e: HistoryEntry = serde_json::from_str(json).unwrap();
        assert_eq!(e.token, "USDC");
        assert_eq!(e.status, "sent");
        assert_eq!(e.ts, 1780623842);
        assert_eq!(e.settle_tx, ""); // settle_tx 없는 옛 기록도 기본값으로 호환
    }

    /// MCP/CLI 에러는 AI 채팅으로 나가므로 URL 의 API 키가 새면 안 된다.
    #[test]
    fn redact_hides_url_api_key() {
        let raw =
            "RPC 연결 실패: error sending request for url (https://h/v2/SUPERSECRETKEY): timed out";
        let red = redact_urls(raw);
        assert!(!red.contains("SUPERSECRETKEY"), "키가 남음: {red}");
        assert!(
            red.contains("[RPC]") && red.contains("timed out"),
            "형태 깨짐: {red}"
        );
    }

    /// 코덱스: host 대소문자·query 콤마·ws/wss 우회 차단.
    #[test]
    fn redact_handles_case_subdelims_and_ws() {
        assert_eq!(
            redact_urls("x HTTPS://BASE.ALCHEMY.com/v2/KEY y"),
            "x [RPC] y"
        );
        let red = redact_urls("https://rpc.example/rpc?x=a,api_key=SECRET done");
        assert!(!red.contains("SECRET"), "{red}");
        assert_eq!(red, "[RPC] done");
        assert_eq!(redact_urls("wss://node/abc end"), "[RPC] end");
    }

    /// 과거 history 의 비redact detail 도 읽기 시점에 가려진다(코덱스 High).
    #[test]
    fn read_history_redacts_detail() {
        let e = HistoryEntry {
            ts: 1,
            token: "ETH".into(),
            to: "0xabc".into(),
            amount: "0".into(),
            status: "failed".into(),
            detail: "RPC 연결 실패: https://base.alchemy.com/v2/LEAKEDKEY".into(),
            settle_tx: String::new(),
        };
        // read_history 가 적용하는 redact 를 같은 함수로 검증(파일 I/O 없이).
        let red = redact_urls(&e.detail);
        assert!(!red.contains("LEAKEDKEY"), "{red}");
        assert_eq!(red, "RPC 연결 실패: [RPC]");
    }

    /// URL 아닌 텍스트·빈 scheme 은 그대로(멀티바이트 안전).
    #[test]
    fn redact_leaves_non_urls() {
        assert_eq!(redact_urls("주소 파싱 실패: bad"), "주소 파싱 실패: bad");
        assert_eq!(redact_urls("just :// floating"), "just :// floating");
        assert_eq!(
            redact_urls("잔액 조회 실패: https://x/y 입니다"),
            "잔액 조회 실패: [RPC] 입니다"
        );
    }
}
