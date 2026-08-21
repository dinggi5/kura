// 사용자 설정 (Session 7~) — 한도·자율 결제·RPC·잠금 동작. ~/.jigap/settings.json 영속화.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::chain::{active_chain, chain_by_id, BASE_MAINNET, BASE_SEPOLIA};
use crate::limits::{parse_eth_nonneg, parse_usdc_nonneg};
use crate::store::{jigap_dir, write_json};

/// 한도 기본값 (사용자가 설정 화면에서 조정 가능). 권한 모델의 가드레일.
/// 개발 39: 신규 기본 체인이 메인넷이 되면서 이 값이 곧 **실돈 한도**다.
/// USDC 5/20 은 실돈으로도 "AI 소액 결제 가드레일"로 적정해 유지. ETH 는 가스·소액
/// 이체용인데 0.05/0.2 는 달러 환산이 USDC 축의 수십 배라 축을 맞춰 0.01/0.05 로 내림.
const DEFAULT_SINGLE_ETH: &str = "0.01";
const DEFAULT_DAILY_ETH: &str = "0.05";
const DEFAULT_SINGLE_USDC: &str = "5";
const DEFAULT_DAILY_USDC: &str = "20";

/// 자율 결제 기본값 (Session 14). 자율 한도 0 = 자율 결제 꺼짐 = 항상 사람 비번 승인(=기존 동작).
/// 보호자가 설정에서 켜야만 작동한다 → 디폴트는 보안 우선.
const DEFAULT_AUTO_APPROVE_USDC: &str = "0";
/// 세션 자동 잠금까지 유휴 시간(분). 0 = 유휴 잠금 안 함(앱 종료·긴급 잠금 시엔 항상 잠김).
const DEFAULT_AUTO_LOCK_MINS: &str = "30";

/// **옛 설정 파일**(chain_id 필드가 없던 개발 20 이전)의 serde 기본값 — 테스트넷.
///
/// 개발 39에서 신규 기본이 메인넷으로 바뀌었지만, 이건 "새 사용자"의 기본이지
/// "필드가 없는 옛 파일"의 해석이 아니다. 옛 파일의 주인은 테스트넷 시절 사용자라,
/// 여기를 메인넷으로 바꾸면 앱 업데이트 한 번에 조용히 실돈 체인으로 옮겨진다.
/// 신규 기본(파일 자체가 없음)은 `Settings::default()` 가 맡는다.
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

    // ── 아래 둘은 **앱이 관리하는 필드**다. 설정 화면 폼에서 오는 값이 아니라서
    //    set_settings 가 클라이언트가 보낸 값을 무시하고 디스크 값을 지킨다(preserve_managed).
    //    프론트 Settings 타입엔 이 둘이 없으므로, 안 지키면 "저장하고 닫기" 한 번에 기본값으로 덮인다.
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

    /// 시작할 때 업데이트가 있는지 확인 (개발 31). **신규 기본 꺼짐 (개발 39)** —
    /// 확인은 깃허브에 HTTPS GET 한 번이고, 그쪽엔 IP 와 현재 버전이 남는다. 로컬 전용을
    /// 내세운 앱의 첫인상이 "말없이 바깥에 안 묻는다"여야 해서 기본을 껐다(켜는 게 선택).
    /// serde 기본은 켜짐 유지 — 이 필드가 없는 옛 파일의 주인은 지금까지 켜진 채 써 온
    /// 사용자라, 그 동작을 보존한다(신규 기본만 바꾼다). 신규는 `Settings::default()`.
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
            // 신규 기본 = 메인넷 (개발 39, 사장 지시). 이 지갑의 용도가 "AI 가 실제 결제를
            // 하는 지갑"이라 첫 화면부터 진짜 지갑이어야 한다. 실돈 안전은 체인이 아니라
            // 한도(위 5/20)·자율 결제 기본 꺼짐·비번 승인 기본이 맡는다.
            // 옛 파일(필드 없음)은 default_chain_id(테스트넷)가, 깨진 파일은
            // conservative()(테스트넷)가 따로 맡는다 — 여기는 "파일이 없는 첫 실행"만.
            chain_id: BASE_MAINNET.chain_id,
            autostart: None, // 아직 모름 → 첫 실행에서 OS 상태를 채택
            auto_check_update: false, // 신규 기본 꺼짐 (개발 39) — 필드 doc 참고
        }
    }
}

impl Settings {
    /// **깨진 설정 파일**을 읽기 전용으로 대신할 보수적 기본값 — 체인만 테스트넷.
    ///
    /// `Settings::default()` 가 메인넷이 되면서(개발 39) "파일이 깨졌다 → 기본값" 경로가
    /// 테스트넷 사용자를 조용히 실돈 체인으로 옮길 수 있게 됐다. 파일이 **없는** 건 새
    /// 사용자라 메인넷 기본이 맞지만, 파일이 **있는데 못 읽는** 건 기존 사용자의 설정을
    /// 잃어버린 상황이라 돈 축은 안전한 쪽(테스트넷)으로 접는다. 저장은 어차피
    /// read_settings_for_update 가 막는다 — 이건 표시·동작용 폴백일 뿐이다.
    fn conservative() -> Self {
        Settings {
            chain_id: BASE_SEPOLIA.chain_id,
            ..Settings::default()
        }
    }
}

fn settings_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join("settings.json"))
}

/// 설정을 읽는다. 파일이 **없으면** 기본값(신규 = 메인넷), 있는데 **못 읽으면** 보수적
/// 기본값(테스트넷). 이 구분은 개발 39에서 기본 체인이 메인넷이 된 순간 생긴 것 —
/// 판단만 떼어낸 `settings_for_read` 에 이유와 테스트가 있다.
pub(crate) fn read_settings() -> Settings {
    match settings_path().map(fs::read_to_string) {
        Ok(Ok(text)) => settings_for_read(Some(&text)),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => settings_for_read(None),
        // 경로를 못 정했거나(홈 없음) 있는 파일을 못 읽었다(권한 등) — 깨진 파일과 같이 취급.
        _ => Settings::conservative(),
    }
}

/// `read_settings` 의 판단만 떼어낸 순수 함수 (IO 없이 테스트하려고).
/// None = 파일 없음(첫 실행) → 신규 기본(메인넷). Some(깨진 JSON) → 보수적 기본(테스트넷).
fn settings_for_read(existing: Option<&str>) -> Settings {
    match existing {
        None => Settings::default(),
        Some(text) => serde_json::from_str(text).unwrap_or_else(|_| Settings::conservative()),
    }
}

/// **덮어쓸 목적으로** 설정을 읽는다. 파일이 있는데 해석이 안 되면 `None`.
///
/// `read_settings` 는 어떤 실패든 기본값으로 삼킨다. 읽기만 할 땐 그게 맞다 — 설정을 못
/// 읽었다고 앱이 안 뜨면 더 손해다. 하지만 **읽은 값을 다시 저장하는 경로**에서는 그 관대함이
/// 그대로 데이터 유실이 된다: 해석 못 한 파일 위에 기본값을 쓰면 사용자의 한도·커스텀 RPC·
/// chain_id 가 한 번에 사라진다(메인넷 사용자가 조용히 테스트넷으로 돌아간다).
///
/// 개발 31 이전에는 settings.json 을 쓰는 곳이 `set_settings`(사용자가 저장 버튼을 누를 때)
/// 하나뿐이라 이 문제가 없었다. autostart::reconcile 이 **시작할 때마다** 읽고 쓰게 되면서
/// 생긴 위험이라, 저장 경로는 이쪽을 쓴다.
pub(crate) fn read_settings_for_update() -> Option<Settings> {
    let path = settings_path().ok()?;
    match fs::read_to_string(&path) {
        Ok(text) => settings_for_update(Some(&text)),
        // 아직 파일이 없는 건 정상(첫 실행) — 기본값에서 시작해 만들면 된다.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => settings_for_update(None),
        // 있는데 못 읽었다(권한 등) → 덮어쓰지 않는다.
        Err(_) => None,
    }
}

/// `read_settings_for_update` 의 판단만 떼어낸 순수 함수 (IO 없이 테스트하려고).
fn settings_for_update(existing: Option<&str>) -> Option<Settings> {
    match existing {
        None => Some(Settings::default()),
        Some(text) => serde_json::from_str(text).ok(),
    }
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
    // 해석 안 되는 파일 위에 기본값을 쓰지 않는다(read_settings_for_update 주석 참고).
    // 여기서 Err 를 내면 프론트가 토글을 원복하는데, 실제로 아무것도 안 바뀌었으니 맞는 표시다.
    let mut s = read_settings_for_update()
        .ok_or("설정 파일을 읽지 못해서 저장하지 않았어요")?;
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

/// 앱이 관리하는 필드(자동 시작 희망값·자동 업데이트 확인)를 `from` 에서 가져온다.
fn preserve_managed(settings: &mut Settings, from: &Settings) {
    settings.autostart = from.autostart;
    settings.auto_check_update = from.auto_check_update;
}

#[cfg(test)]
mod tests {
    use super::*;

    // 기본 설정값 — 신규 기본은 메인넷(개발 39), 한도는 실돈 가드레일 축(USDC 5/20, ETH 0.01/0.05).
    #[test]
    fn default_settings_values() {
        let s = Settings::default();
        assert_eq!(s.single_usdc, "5");
        assert_eq!(s.daily_usdc, "20");
        assert_eq!(s.single_eth, "0.01");
        assert_eq!(s.daily_eth, "0.05");
        // Session 14: 자율 결제는 기본 꺼짐(0), 유휴 잠금 30분, RPC=공식, 자리비움잠금 꺼짐.
        assert_eq!(s.auto_approve_usdc, "0");
        assert_eq!(s.auto_lock_mins, "30");
        assert!(s.rpc_url.is_empty()); // 빈 값 = 활성 체인 공식 RPC 따라감
        assert!(!s.lock_on_blur);
        assert_eq!(s.chain_id, BASE_MAINNET.chain_id); // 신규 기본 = 메인넷 (개발 39)
        assert!(!s.auto_check_update); // 신규 기본 꺼짐 (개발 39, 프라이버시)
    }

    // 🔴 신규(파일 없음)와 깨진 파일(있는데 못 읽음)은 다른 답이어야 한다 (개발 39).
    // 신규 기본이 메인넷이 된 순간, "깨졌으면 기본값" 경로가 테스트넷 사용자를 조용히
    // 실돈 체인으로 옮기는 문이 된다 — 깨진 파일은 돈 축을 테스트넷으로 접는다.
    #[test]
    fn read_fallbacks_split_new_vs_corrupt() {
        // 파일 없음 = 첫 실행 → 신규 기본(메인넷).
        assert_eq!(settings_for_read(None).chain_id, BASE_MAINNET.chain_id);
        // 깨진 JSON → 보수적(테스트넷). 나머지 값은 기본과 동일.
        let c = settings_for_read(Some("{ 이건 JSON 이 아니다"));
        assert_eq!(c.chain_id, BASE_SEPOLIA.chain_id);
        assert_eq!(c.single_usdc, "5");
        assert!(!c.auto_check_update);
        // 정상 JSON 은 그대로.
        let ok = settings_for_read(Some(r#"{"single_usdc":"7","daily_usdc":"30",
            "single_eth":"0.1","daily_eth":"0.5","chain_id":8453}"#));
        assert_eq!(ok.chain_id, 8453);
        assert_eq!(ok.single_usdc, "7");
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
        // 필드 없는 옛 파일은 켜짐 유지 — 그 사용자는 지금까지 켜진 채 써 왔다(동작 보존).
        // 신규(파일 없음)만 꺼짐이 기본이다(개발 39, default_settings_values 참고).
        assert!(s.auto_check_update);
    }

    // 🔴 해석 안 되는 설정 파일 위에 기본값을 쓰면 안 된다.
    //
    // read_settings 는 어떤 실패든 기본값으로 삼키는데, 개발 31 부터 autostart::reconcile 이
    // **시작할 때마다** 읽고-쓰기를 한다. 그 경로가 read_settings 를 쓰면, 파일이 한 번 깨진
    // 순간 사용자의 한도·커스텀 RPC·chain_id 가 통째로 기본값으로 덮인다 —
    // 메인넷(8453)을 쓰던 사람이 아무 말 없이 테스트넷으로 돌아간다.
    #[test]
    fn corrupt_settings_are_never_overwritten() {
        // 파일 없음 = 첫 실행 → 기본값에서 시작해도 안전하다.
        assert!(settings_for_update(None).is_some());

        // 정상 JSON → 그대로 읽힌다.
        let ok = settings_for_update(Some(r#"{"single_usdc":"7","daily_usdc":"30",
            "single_eth":"0.1","daily_eth":"0.5","chain_id":8453}"#));
        assert_eq!(ok.expect("정상 JSON 인데 None").chain_id, 8453);

        // 🔴 깨진 JSON·타입이 어긋난 값 → None. 호출부는 여기서 손을 떼야 한다.
        assert!(settings_for_update(Some("{ 이건 JSON 이 아니다")).is_none());
        assert!(settings_for_update(Some(r#"{"single_usdc": 5}"#)).is_none()); // 문자열이어야 함
        assert!(settings_for_update(Some("")).is_none());
    }

    // 🔴 폼 저장이 앱 관리 필드를 지우면 안 된다.
    // 프론트 Settings 타입엔 autostart·auto_check_update 가 없어서, 설정 화면의
    // "저장하고 닫기"는 항상 이 셋을 기본값(None/None/true)으로 실어 보낸다. 그대로 쓰면
    // 저장 한 번에 자동 시작 희망값이 날아가고 — 이번 세션이 고치려던 버그가 그대로 되살아난다.
    #[test]
    fn form_save_does_not_wipe_app_managed_fields() {
        let on_disk = Settings {
            autostart: Some(true),
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
