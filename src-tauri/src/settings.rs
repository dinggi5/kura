// 사용자 설정 (Session 7~) — 한도·자율 결제·RPC·잠금 동작. ~/.jigap/settings.json 영속화.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::chain::{active_chain, chain_by_id, BASE_SEPOLIA};
use crate::limits::{parse_eth_nonneg, parse_usdc_nonneg};
use crate::store::{jigap_dir, write_json};

/// 한도 기본값 (사용자가 설정 화면에서 조정 가능). 권한 모델의 가드레일.
const DEFAULT_SINGLE_ETH: &str = "0.05";
const DEFAULT_DAILY_ETH: &str = "0.2";
const DEFAULT_SINGLE_USDC: &str = "5";
const DEFAULT_DAILY_USDC: &str = "20";

/// 자율 결제 기본값 (Session 14). 자율 한도 0 = 자율 결제 꺼짐 = 항상 사람 비번 승인(=기존 동작).
/// 보호자가 설정에서 켜야만 작동한다 → 디폴트는 보안 우선.
const DEFAULT_AUTO_APPROVE_USDC: &str = "0";
/// 세션 자동 잠금까지 유휴 시간(분). 0 = 유휴 잠금 안 함(앱 종료·긴급 잠금 시엔 항상 잠김).
const DEFAULT_AUTO_LOCK_MINS: &str = "30";

fn default_chain_id() -> u64 {
    BASE_SEPOLIA.chain_id
}

/// 사용자 조정 가능한 한도 설정 (십진수 문자열). 권한 모델의 가드레일.
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct Settings {
    pub(crate) single_usdc: String,
    pub(crate) daily_usdc: String,
    pub(crate) single_eth: String,
    pub(crate) daily_eth: String,
    /// 자율 결제 한도 (USDC, 십진수). 이 금액 이하의 AI 결제 요청은 세션이 잠금 해제돼 있으면
    /// 비번 없이 자동 승인된다. "0"이면 자율 결제 꺼짐(항상 사람 비번). 옛 설정 파일엔 없어서 기본 "0".
    #[serde(default = "default_auto_approve_usdc")]
    pub(crate) auto_approve_usdc: String,
    /// 세션 자동 잠금 유휴 시간(분). "0"이면 유휴 잠금 안 함. 옛 파일 호환 위해 기본값 지정.
    #[serde(default = "default_auto_lock_mins")]
    pub(crate) auto_lock_mins: String,
    /// 잔액 조회·송금에 쓸 RPC 엔드포인트. 사용자가 프라이버시·속도 이유로 바꿀 수 있다(Session 14).
    /// **빈 값 = "활성 체인의 공식 RPC를 따라간다"** (effective_rpc 가 active_chain().default_rpc 로 해석).
    /// 기본을 구체 URL 로 박아 저장하면 active_chain() 을 메인넷으로 바꿔도 옛 RPC 에 고정되는
    /// 함정이 생긴다 → 기본은 빈 값으로 둬서 체인 전환을 자동으로 따라가게 한다. (개발 18 코덱스 리뷰)
    #[serde(default)]
    pub(crate) rpc_url: String,
    /// 자리비움 자동 잠금: 창이 포커스를 잃으면(다른 앱 전환·화면 잠금) 세션을 즉시 잠근다. 기본 꺼짐.
    #[serde(default)]
    pub(crate) lock_on_blur: bool,
    /// 자율 결제 알림: 비번 없이 자동 승인된 결제를 OS 알림으로 사후 통지. 기본 켜짐
    /// (자율 = 보호자가 모르는 새 돈이 나가는 유일한 경로라, 끄는 쪽이 명시적 선택이어야 함).
    #[serde(default = "default_true")]
    pub(crate) notify_auto: bool,
    /// 자율 결제는 신뢰 주소(비번으로 승인한 적 있는 받는 주소)만. 기본 켜짐 —
    /// 끄면 한도 이하 금액이면 처음 보는 주소에도 비번 없이 나간다.
    #[serde(default = "default_true")]
    pub(crate) auto_trusted_only: bool,
    /// 활성 체인 ID — 테스트넷(84532) ↔ 메인넷(8453) 런타임 전환. chain::active_chain() 이 이 값을 읽는다.
    /// 옛 설정엔 없어서 기본 = 테스트넷(실돈 안전). 체인별로 사용액·내역·신뢰목록 파일이 분리된다.
    #[serde(default = "default_chain_id")]
    pub(crate) chain_id: u64,

    // ── 아래 셋은 **앱이 관리하는 필드**다. 설정 화면 폼에서 오는 값이 아니라서
    //    set_settings 가 클라이언트가 보낸 값을 무시하고 디스크 값을 지킨다(preserve_managed).
    //    프론트 Settings 타입엔 이 셋이 없으므로, 안 지키면 "저장하고 닫기" 한 번에 전부 None 이 된다.
    /// 로그인 시 자동 시작 **희망값** (개발 31).
    ///
    /// OS 로그인 아이템(LaunchAgent)만 보면 "사용자가 껐다"와 "패키지 관리자가 지웠다"를 구별할
    /// 수 없다. 캐스크의 `uninstall launchctl:` 은 `brew upgrade`·`brew reinstall` 때도 돌아서
    /// plist 를 내린다 → 업데이트 한 번에 자동 시작이 조용히 꺼진다(개발 30 코덱스 P2).
    /// 그래서 희망값을 여기 따로 적어 두고, 시작할 때 대조한다(autostart::reconcile).
    ///
    /// None = 아직 모름(이 필드가 없던 시절의 설정 파일) → 첫 실행에서 OS 상태를 그대로 채택한다.
    /// **기본을 false 로 두면 안 된다** — 자동 시작을 켜 둔 기존 사용자가 "꺼짐을 원했다"로
    /// 기록되고, 그 뒤 진짜로 꺼졌을 때 복구 대상에서 빠진다.
    #[serde(default)]
    pub(crate) autostart: Option<bool>,

    /// 마지막으로 실행된 앱 버전 (개발 31). 자동 시작 복구를 **버전이 바뀐 실행에서만** 하기 위한 표식.
    /// 매번 무조건 복구하면, 사용자가 시스템 설정 > 로그인 항목에서 끈 것을 앱이 다시 켜 버린다 —
    /// 지갑 앱이 할 짓이 아니다. 자세한 판정은 autostart::reconcile.
    #[serde(default)]
    pub(crate) last_run_version: Option<String>,

    /// 시작할 때 업데이트가 있는지 확인 (개발 31). 기본 켜짐.
    /// 확인은 깃허브에 HTTPS GET 한 번이고, 그쪽엔 IP 와 현재 버전이 남는다.
    /// 끄면 업데이트는 사용자가 설정에서 직접 눌러야만 확인된다(설치는 어느 쪽이든 수동).
    #[serde(default = "default_true")]
    pub(crate) auto_check_update: bool,
}

fn default_true() -> bool {
    true
}

fn default_auto_approve_usdc() -> String {
    DEFAULT_AUTO_APPROVE_USDC.to_string()
}

fn default_auto_lock_mins() -> String {
    DEFAULT_AUTO_LOCK_MINS.to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            single_usdc: DEFAULT_SINGLE_USDC.into(),
            daily_usdc: DEFAULT_DAILY_USDC.into(),
            single_eth: DEFAULT_SINGLE_ETH.into(),
            daily_eth: DEFAULT_DAILY_ETH.into(),
            auto_approve_usdc: DEFAULT_AUTO_APPROVE_USDC.into(),
            auto_lock_mins: DEFAULT_AUTO_LOCK_MINS.into(),
            rpc_url: String::new(), // 빈 값 = 활성 체인 공식 RPC 따라감 (effective_rpc 가 해석)
            lock_on_blur: false,
            notify_auto: true,
            auto_trusted_only: true,
            chain_id: BASE_SEPOLIA.chain_id, // Base Sepolia(테스트넷) — 신규/옛 설정은 항상 테스트넷
            autostart: None,                 // 아직 모름 → 첫 실행에서 OS 상태를 채택
            last_run_version: None,
            auto_check_update: true,
        }
    }
}

fn settings_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join("settings.json"))
}

/// 설정을 읽는다. 파일이 없거나 깨졌으면 기본값.
pub(crate) fn read_settings() -> Settings {
    settings_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 설정을 그대로 저장한다 (앱 내부용 — 사용자 입력 검증 없음).
///
/// `set_settings` 는 폼에서 온 값을 검증하는 경로라, 앱이 스스로 한 필드만 갱신할 때
/// 그걸 통과시킬 이유가 없다(그리고 통과시키면 옛 설정 파일의 어긋난 값 때문에
/// 자동 시작 기록 같은 게 저장에 실패한다). 검증이 필요한 값은 여기로 오지 않는다.
pub(crate) fn save_settings(settings: &Settings) -> Result<(), String> {
    write_json(settings_path()?, settings)
}

/// 설정의 RPC를 돌려준다. 비어 있으면 공식 RPC로 폴백.
pub(crate) fn effective_rpc() -> String {
    let url = read_settings().rpc_url.trim().to_string();
    if url.is_empty() {
        active_chain().default_rpc.to_string()
    } else {
        url
    }
}

/// 사용자/AI 에 노출되는 에러·로그 문자열에서 URL 을 통째로 `[RPC]` 로 가린다.
/// 커스텀 RPC 경로·쿼리엔 API 키가 들어가곤 한다(예: alchemy `…/v2/KEY`). alloy/reqwest 에러는
/// URL 을 그대로 실어 나르므로 — 특히 MCP/CLI 결과·거래내역은 AI 채팅으로 나가 키가 LLM 에 샐 수 있다.
/// **설정을 다시 읽지 않고 문자열에 보이는 URL 자체를 가린다** → 설정 변경·host 대소문자 정규화·
/// ws/wss 등에 흔들리지 않는다(코덱스 리뷰 반영). URL 외 문자는 그대로 둔다.
/// 양 크레이트(src-tauri·kura-mcp)에 의도적 중복 — 공유 크레이트를 안 만드는 정책.
pub(crate) fn redact_urls(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(rel) = input[cursor..].find("://") {
        let sep = cursor + rel;
        if let Some(start) = scheme_start(input, cursor, sep) {
            out.push_str(&input[cursor..start]);
            // 토큰 끝: 공백·제어문자 또는 URL 에 못 들어가는 문자("· <· >· `)에서 멈춘다.
            // `)`·`,`·`'` 는 URL sub-delim 이라 종료자로 안 씀 — 그 뒤 키가 새지 않게 보수적으로(코덱스).
            let token = &input[start..];
            let end = token
                .find(|c: char| {
                    c.is_whitespace() || c.is_control() || matches!(c, '"' | '<' | '>' | '`')
                })
                .unwrap_or(token.len());
            out.push_str("[RPC]");
            cursor = start + end;
        } else {
            // "://" 앞에 유효 scheme 이 없다 → 그대로 두고 그 뒤부터 계속 스캔.
            out.push_str(&input[cursor..sep + 3]);
            cursor = sep + 3;
        }
    }
    out.push_str(&input[cursor..]);
    out
}

/// `sep`(="://"의 시작 인덱스) 앞에서 scheme 시작 인덱스를 찾는다. scheme = `[A-Za-z][A-Za-z0-9+.-]*`.
/// `min` 미만으로는 내려가지 않는다(이미 처리한 영역 침범 방지). 유효 scheme 없으면 None.
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
    // 최소 1글자 + 맨 앞은 영문자(RFC 3986 scheme).
    if start < sep && bytes[start].is_ascii_alphabetic() {
        Some(start)
    } else {
        None
    }
}

/// 현재 한도 설정을 돌려준다 (없으면 기본값).
#[tauri::command]
pub(crate) fn get_settings() -> Settings {
    read_settings()
}

/// 시작 시 업데이트 자동 확인 토글 (개발 31).
/// 앱 관리 필드라 "저장하고 닫기"와 무관하게 즉시 적용된다(자동 시작 토글과 같은 결).
#[tauri::command]
pub(crate) fn set_auto_check_update(enabled: bool) -> Result<(), String> {
    let mut s = read_settings();
    s.auto_check_update = enabled;
    save_settings(&s)
}

/// 한도 설정을 저장한다. 모든 값이 십진수로 파싱되는지 검증 후 기록.
#[tauri::command]
pub(crate) fn set_settings(mut settings: Settings) -> Result<(), String> {
    // 체인을 먼저 검증한다(미지원 거부) — 그리고 한도는 **들어온 chain_id 의 decimals** 로 검증한다.
    // 저장 전 활성 체인(active_chain) 기준으로 검증하면, 다른 decimals 의 체인으로 토글하는 저장에서
    // 한도 의미가 어긋난다(현재 두 체인 다 6자리라 무해하나, 미래 체인 대비 원자성 — 코덱스 개발20 리뷰).
    let dec = chain_by_id(settings.chain_id)
        .ok_or("지원하지 않는 체인입니다")?
        .usdc_decimals;
    // 음수 거부(parse_*_nonneg) — 음수 한도가 거대 U256 = "사실상 무제한"으로 둔갑해 가드레일이
    // 조용히 무력화되는 함정 차단. 0 은 정상(=무제한 의도).
    parse_usdc_nonneg(&settings.single_usdc, dec)
        .map_err(|_| "단일 USDC 한도가 올바르지 않습니다 (음수 불가)".to_string())?;
    parse_usdc_nonneg(&settings.daily_usdc, dec)
        .map_err(|_| "일일 USDC 한도가 올바르지 않습니다 (음수 불가)".to_string())?;
    parse_eth_nonneg(&settings.single_eth)
        .map_err(|_| "단일 ETH 한도가 올바르지 않습니다 (음수 불가)".to_string())?;
    parse_eth_nonneg(&settings.daily_eth)
        .map_err(|_| "일일 ETH 한도가 올바르지 않습니다 (음수 불가)".to_string())?;
    parse_usdc_nonneg(&settings.auto_approve_usdc, dec)
        .map_err(|_| "자율 결제 한도가 올바르지 않습니다 (음수 불가)".to_string())?;
    settings
        .auto_lock_mins
        .trim()
        .parse::<u64>()
        .map_err(|_| "자동 잠금(분)은 정수로 입력하세요".to_string())?;
    let rpc = settings.rpc_url.trim();
    if !(rpc.is_empty() || rpc.starts_with("http://") || rpc.starts_with("https://")) {
        return Err("RPC 주소는 http(s):// 로 시작해야 합니다".into());
    }
    // 앱 관리 필드는 클라이언트가 보낸 값을 버리고 디스크 값을 지킨다.
    // 프론트 Settings 타입엔 이 셋이 없어서 폼 저장은 항상 기본값(None/true)을 실어 보낸다 —
    // 그대로 쓰면 "저장하고 닫기" 한 번에 자동 시작 희망값이 지워진다(=고치려던 버그의 재발).
    preserve_managed(&mut settings, &read_settings());
    save_settings(&settings)
}

/// 앱이 관리하는 필드(자동 시작 희망값·마지막 실행 버전·자동 업데이트 확인)를 `from` 에서 가져온다.
fn preserve_managed(settings: &mut Settings, from: &Settings) {
    settings.autostart = from.autostart;
    settings.last_run_version = from.last_run_version.clone();
    settings.auto_check_update = from.auto_check_update;
}

#[cfg(test)]
mod tests {
    use super::*;

    // 기본 설정값이 메모리 기준(단일 5 / 일일 20 USDC)과 일치해야 한다.
    #[test]
    fn default_settings_values() {
        let s = Settings::default();
        assert_eq!(s.single_usdc, "5");
        assert_eq!(s.daily_usdc, "20");
        assert_eq!(s.single_eth, "0.05");
        assert_eq!(s.daily_eth, "0.2");
        // Session 14: 자율 결제는 기본 꺼짐(0), 유휴 잠금 30분, RPC=공식, 자리비움잠금 꺼짐.
        assert_eq!(s.auto_approve_usdc, "0");
        assert_eq!(s.auto_lock_mins, "30");
        assert!(s.rpc_url.is_empty()); // 빈 값 = 활성 체인 공식 RPC 따라감
        assert!(!s.lock_on_blur);
        assert_eq!(s.chain_id, BASE_SEPOLIA.chain_id); // 기본 = 테스트넷(실돈 안전)
    }

    // 옛 settings.json(자율 결제 필드 없음)도 손실 없이 로드되고 새 필드는 기본값이 된다.
    // (#[serde(default)] 가 없으면 파싱 실패 → 사용자의 기존 한도가 통째로 날아가는 버그)
    #[test]
    fn old_settings_without_auto_fields_loads() {
        let old = r#"{"single_usdc":"7","daily_usdc":"30","single_eth":"0.1","daily_eth":"0.5"}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(s.single_usdc, "7"); // 기존 값 보존
        assert_eq!(s.daily_usdc, "30");
        assert_eq!(s.auto_approve_usdc, "0"); // 새 필드는 기본값
        assert_eq!(s.auto_lock_mins, "30");
        assert!(s.rpc_url.is_empty()); // 옛 파일엔 RPC 없음 → 빈 값(=공식 폴백)
        assert!(!s.lock_on_blur);
        assert!(s.notify_auto); // 알림은 기본 켜짐 (끄는 쪽이 명시적 선택)
        assert!(s.auto_trusted_only); // 신뢰 주소 가드도 기본 켜짐 (안전 쪽 디폴트)
        assert_eq!(s.chain_id, BASE_SEPOLIA.chain_id); // 옛 파일엔 chain_id 없음 → 테스트넷
        // 개발 31 필드도 옛 파일에서 안전하게 온다. autostart 기본은 **None**(false 아님) —
        // false 면 자동 시작을 켜 둔 기존 사용자가 "끔을 원했다"로 기록돼 복구 대상에서 빠진다.
        assert_eq!(s.autostart, None);
        assert_eq!(s.last_run_version, None);
        assert!(s.auto_check_update); // 업데이트 확인은 기본 켜짐
    }

    // 🔴 폼 저장이 앱 관리 필드를 지우면 안 된다.
    // 프론트 Settings 타입엔 autostart·last_run_version·auto_check_update 가 없어서, 설정 화면의
    // "저장하고 닫기"는 항상 이 셋을 기본값(None/None/true)으로 실어 보낸다. 그대로 쓰면
    // 저장 한 번에 자동 시작 희망값이 날아가고 — 이번 세션이 고치려던 버그가 그대로 되살아난다.
    #[test]
    fn form_save_does_not_wipe_app_managed_fields() {
        let on_disk = Settings {
            autostart: Some(true),
            last_run_version: Some("0.1.1".into()),
            auto_check_update: false,
            ..Default::default()
        };
        // 프론트가 보낸 것 = 폼 필드만 있고 관리 필드는 기본값인 상태.
        let mut from_form = Settings {
            single_usdc: "9".into(),
            ..Default::default()
        };
        assert_eq!(from_form.autostart, None); // 전제 확인

        preserve_managed(&mut from_form, &on_disk);

        assert_eq!(from_form.autostart, Some(true)); // 희망값 보존
        assert_eq!(from_form.last_run_version.as_deref(), Some("0.1.1"));
        assert!(!from_form.auto_check_update); // 꺼 둔 것도 보존
        assert_eq!(from_form.single_usdc, "9"); // 폼 값은 그대로 반영
    }

    // 음수 한도는 저장을 거부해야 한다 (거대 U256=무제한 둔갑 차단). 양수·0 은 통과.
    #[test]
    fn set_settings_rejects_negative_limits() {
        let neg_single = Settings {
            single_usdc: "-1".into(),
            ..Default::default()
        };
        assert!(set_settings(neg_single).is_err());

        let neg_auto = Settings {
            auto_approve_usdc: "-0.01".into(),
            ..Default::default()
        };
        assert!(set_settings(neg_auto).is_err());

        let neg_eth = Settings {
            daily_eth: "-0.2".into(),
            ..Default::default()
        };
        assert!(set_settings(neg_eth).is_err());
    }

    // 알 수 없는 체인 ID는 저장 거부 (지원 체인=테스트넷/메인넷만). chain_id:1(이더리움 L1)=미지원.
    // (Err 경로만 검사 — 유효 입력으로 set_settings 를 부르면 실제 ~/.jigap/settings.json 을 덮어써서
    //  테스트가 사용자 설정·체인을 바꿔버린다. 음수 한도 테스트와 같은 이유로 거부 케이스만.)
    #[test]
    fn set_settings_rejects_unknown_chain() {
        let bad = Settings {
            chain_id: 1,
            ..Default::default()
        };
        let err = set_settings(bad).unwrap_err();
        assert!(err.contains("지원하지 않는 체인"), "체인 검증 메시지가 아님: {err}");
    }

    // URL(경로의 API 키)이 에러 메시지에서 통째로 가려져야 한다.
    #[test]
    fn redact_hides_url_api_key() {
        assert_eq!(
            redact_urls("RPC 연결 실패: https://base-sepolia.g.alchemy.com/v2/SUPERSECRETKEY"),
            "RPC 연결 실패: [RPC]",
        );
        // reqwest 처럼 괄호 안에 URL 이 박힌 경우 — 키는 사라지고 뒤 메시지는 남는다.
        let red = redact_urls("error sending request for url (https://h/v2/KEY): connection closed");
        assert!(!red.contains("KEY"), "키가 남음: {red}");
        assert!(red.contains("[RPC]") && red.contains("connection closed"), "형태 깨짐: {red}");
    }

    // 코덱스 리뷰: host 대소문자 정규화·query 콤마 뒤 키·ws/wss 우회를 막아야 한다.
    #[test]
    fn redact_handles_case_subdelims_and_ws() {
        assert_eq!(redact_urls("x HTTPS://BASE.g.ALCHEMY.com/v2/KEY y"), "x [RPC] y"); // 대소문자
        let red = redact_urls("https://rpc.example/rpc?x=a,api_key=SECRET done");
        assert!(!red.contains("SECRET"), "콤마 뒤 키가 남음: {red}");
        assert_eq!(red, "[RPC] done");
        assert_eq!(redact_urls("wss://node/abc end"), "[RPC] end"); // websocket RPC
    }

    // URL 아닌 텍스트·빈 scheme 은 그대로 둔다(멀티바이트 안전).
    #[test]
    fn redact_leaves_non_urls() {
        assert_eq!(redact_urls("주소 파싱 실패: bad input"), "주소 파싱 실패: bad input");
        assert_eq!(redact_urls("just :// floating"), "just :// floating");
        assert_eq!(redact_urls("잔액 조회 실패: https://x/y 입니다"), "잔액 조회 실패: [RPC] 입니다");
    }
}
