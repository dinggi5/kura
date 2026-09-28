// 거래 한도 (Session 7) — 단일/일일 누적 한도 검사 + 오늘 사용액 장부(~/.jigap/spend.json).

use crate::i18n::{tf, ts};
use alloy::primitives::{
    utils::{format_ether, format_units, parse_ether, parse_units},
    U256,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::chain::{active_chain, chain_file};
use crate::store::{current_day, jigap_dir, write_json};

/// 일일 한도 장부(spend.json) 직렬화 락. read-check-write 를 하나의 임계영역으로 묶어, 동시 결제가
/// 같은 누적액을 보고 둘 다 통과(일일 한도 우회)하는 경합을 막는다. **락은 빠른 파일 I/O 구간만**
/// 잡는다 — 느린/멈춘 RPC 가 모든 결제를 전역 정지시키지 않게, 네트워크 전송은 락 밖에서.
/// std Mutex 는 .await 를 못 넘으므로 tokio 비동기 Mutex 를 쓴다.
static SPEND_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 오늘 장부에서 토큰별 누적액(base unit)을 읽는다.
fn spent_of(spend: &Spend, token: &str) -> U256 {
    let s = if token == "ETH" {
        &spend.eth
    } else {
        &spend.usdc
    };
    s.parse().unwrap_or(U256::ZERO)
}

/// 토큰별 누적액을 설정한다.
fn set_spent(spend: &mut Spend, token: &str, v: U256) {
    if token == "ETH" {
        spend.eth = v.to_string();
    } else {
        spend.usdc = v.to_string();
    }
}

/// 일일 장부에 사용액을 **예약**한다(낙관적 선반영) — 락 안에서 read-check-write 를 빠른 I/O 로만.
/// 한도 통과 + 디스크 기록 성공 시 **예약한 날(UTC 에포크일)** 을 돌려준다(환불이 같은 날에만 적용되게).
/// 한도 초과/기록 실패면 Err — 기록이 안 됐는데 Ok 를 주면 나중에 헛환불로 다른 사용액을 깎는다.
/// 네트워크 전송은 호출자가 락 밖에서 수행하고, 실패하면 refund_spend 로 되돌린다(reserve-then-execute).
pub(crate) async fn reserve_spend(
    token: &str,
    value: U256,
    single: U256,
    daily: U256,
    decimals: u8,
) -> Result<u64, String> {
    let _g = SPEND_LOCK.lock().await;
    let mut spend = spend_for_reserve().ok_or_else(|| {
        ts!(
            "오늘 쓴 금액 장부를 읽지 못해 결제를 막았어요. 한도를 지킬 수 없어서예요 — 내일(UTC) 새 장부로 풀리거나, 파일을 고치면 됩니다.",
            "Couldn't read today's spending ledger, so the payment was blocked — the limits can't be enforced without it. It clears with a fresh ledger tomorrow (UTC), or once the file is fixed."
        )
        .to_string()
    })?;
    let spent = spent_of(&spend, token);
    enforce_caps(value, single, spent, daily, token, decimals)?;
    let before = serde_json::to_string_pretty(&spend).map_err(|e| e.to_string())?;
    set_spent(&mut spend, token, spent.saturating_add(value));
    // 기록 실패면 예약도 실패(헛환불 방지). 디스크까지 내린다 — 이 뒤에 돈이 나간다(개발 71).
    let json = serde_json::to_string_pretty(&spend).map_err(|e| e.to_string())?;
    let path = spend_path()?;
    match crate::store::write_atomic_durable(&path, json.as_bytes()) {
        Ok(()) => Ok(spend.day),
        Err(crate::store::DurableError::NotWritten(e)) => Err(e),
        // 새 장부는 제자리인데 디스크까지 못 내렸다 — 예약 전으로 되돌리고 이 결제는 거절한다(개발 71 코덱스 2·3차 절충).
        // 되돌리기마저 실패하면 한도만 깎인 채 남는다(사용자에게 불리할 뿐 돈은 안 나간다).
        Err(crate::store::DurableError::NotDurable(e)) => {
            let _ = crate::store::write_atomic(&path, before.as_bytes());
            Err(e)
        }
    }
}

/// 전송/서명 실패 시 reserve_spend 로 선반영했던 사용액을 되돌린다(환불). 락 안에서.
/// **예약한 그 날에만** 차감한다 — 자정을 넘겨 장부가 리셋된 뒤 환불하면 새 날의 정상 사용액을
/// 깎아 일일 한도가 우회될 수 있으므로, 날이 다르면 그 예약은 이미 사라진 것으로 보고 no-op.
pub(crate) async fn refund_spend(token: &str, value: U256, reserved_day: u64) {
    let _g = SPEND_LOCK.lock().await;
    // 못 읽는 장부엔 환불하지 않는다 — 0 에서 빼 봐야 0 이고, 새로 쓰면 깨진 파일(오늘 쓴 기록)을 덮는다.
    let Some(mut spend) = spend_for_reserve() else {
        return;
    };
    if spend.day != reserved_day {
        return; // 다른 날 → 예약분은 이미 리셋됨, 건드리지 않는다
    }
    let spent = spent_of(&spend, token);
    set_spent(&mut spend, token, spent.saturating_sub(value));
    let _ = spend_path().and_then(|p| write_json(p, &spend));
}

/// 체인 확인이 확정한 「돈이 안 나간 결제」의 한도를 **기록 하나당 한 번만** 돌려준다 (개발 71 코덱스 3차).
/// `key` = 내역 기록의 열쇠(고유 id, 옛 기록은 시각·해시). 오늘 장부가 아니거나(예약한 날이 지났다) 이미 돌려준 열쇠면 아무것도 안 한다.
/// 못 읽는 장부엔 손대지 않는다. 돌려줬으면 true.
pub(crate) async fn refund_once(token: &str, value: U256, day: u64, key: &str) -> bool {
    let _g = SPEND_LOCK.lock().await;
    let Some(mut spend) = spend_for_reserve() else {
        return false;
    };
    if !apply_refund_once(&mut spend, token, value, day, key) {
        return false;
    }
    spend_path().and_then(|p| write_json(p, &spend)).is_ok()
}

/// `refund_once` 의 판단 (순수 — 테스트용).
fn apply_refund_once(spend: &mut Spend, token: &str, value: U256, day: u64, key: &str) -> bool {
    if spend.day != day || key.is_empty() || spend.refunded.iter().any(|k| k == key) {
        return false;
    }
    let spent = spent_of(spend, token);
    set_spent(spend, token, spent.saturating_sub(value));
    spend.refunded.push(key.to_string());
    true
}

/// USDC 금액/한도 문자열을 base unit U256 로 파싱한다. **음수는 거부**한다.
/// (alloy `parse_units` 는 "-1" 을 I256 으로 받아 `get_absolute()`=`into_raw()` 가 2의 보수
///  거대 U256 을 돌려준다 → 음수 한도가 "무제한"으로, 음수 금액이 거대 송금으로 둔갑하는 함정.)
pub(crate) fn parse_usdc_nonneg(s: &str, dec: u8) -> Result<U256, String> {
    let pu = parse_units(s.trim(), dec).map_err(|e| {
        tf!(
            "금액 형식 오류: {e}",
            "That amount isn't a valid number: {e}"
        )
    })?;
    if pu.is_negative() {
        return Err(ts!(
            "음수 금액은 허용되지 않습니다",
            "A negative amount isn't allowed"
        )
        .into());
    }
    Ok(pu.get_absolute())
}

/// ETH 금액/한도 문자열을 wei U256 로 파싱한다. **음수는 거부**한다.
/// (`parse_ether` 도 내부적으로 `get_absolute()` 를 거치므로 "-1" 이 거대 U256 이 된다.)
pub(crate) fn parse_eth_nonneg(s: &str) -> Result<U256, String> {
    let t = s.trim();
    if t.starts_with('-') {
        return Err(ts!(
            "음수 금액은 허용되지 않습니다",
            "A negative amount isn't allowed"
        )
        .into());
    }
    parse_ether(t).map_err(|e| {
        tf!(
            "금액 형식 오류: {e}",
            "That amount isn't a valid number: {e}"
        )
    })
}

/// 오늘 누적 사용액 장부. 금액은 base unit(USDC=6decimals, ETH=wei) U256 문자열.
/// `day`(UTC 일 단위)가 바뀌면 자동 리셋한다.
#[derive(Serialize, Deserialize, Default)]
pub(crate) struct Spend {
    pub(crate) day: u64,
    pub(crate) usdc: String,
    pub(crate) eth: String,
    /// 오늘 **체인 확인으로** 환불해 준 내역 기록의 열쇠(개발 71 코덱스 3차) — 같은 기록을 두 번 돌려주지 않는 근거.
    /// 환불을 「내역 파일에 결말을 적었는가」에 매달면, 사본 둘(본 파일·보관 파일) 중 하나만 써지는 실패에서 두 번 주거나
    /// 영영 안 줬다. 장부 한 파일 안에서 「빼기」와 「적어 두기」를 같이 쓰므로 둘이 갈리지 않는다. 날이 바뀌면 같이 비워진다.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) refunded: Vec<String>,
}

/// 프론트로 주는 오늘 사용액 (보기 좋은 십진수).
#[derive(Serialize)]
pub(crate) struct SpendView {
    usdc: String,
    eth: String,
}

pub(crate) fn spend_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join(chain_file("spend")))
}

/// 예약·환불용 오늘 장부 (개발 71, 코덱스 1차 P1). `read_spend_today` 는 못 읽으면 0 으로 삼킨다 — 화면엔 그래도 되지만
/// **돈이 나가기 전의 한도 검사**에선 「오늘 19 USDC 를 썼는데 장부가 상해 0 부터」가 되어 일일 한도를 넘겨 보냈다
/// (내역·입금에서 두 번 본 「못 읽으면 빈 값」 병). 파일이 있는데 못 읽으면 None — 단 마지막 수정이 **어제 이전**이면 그 파일은
/// 오늘 쓴 돈을 담을 수 없으니 새 날 장부로 간다(안 그러면 한 번 상한 장부가 영영 결제를 막는다).
fn spend_for_reserve() -> Option<Spend> {
    let path = spend_path().ok()?;
    let raw = match fs::read_to_string(&path) {
        Ok(t) => Some(Ok(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(Err(())),
    };
    let mtime_day = fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() / 86_400);
    judge_spend(raw, mtime_day, current_day())
}

/// `spend_for_reserve` 의 판단 (순수 — 테스트용). `raw` = None(파일 없음) | Ok(본문) | Err(있는데 못 읽음).
fn judge_spend(
    raw: Option<Result<String, ()>>,
    mtime_day: Option<u64>,
    today: u64,
) -> Option<Spend> {
    let fresh = Spend {
        day: today,
        usdc: "0".into(),
        eth: "0".into(),
        ..Default::default()
    };
    let parsed = match raw {
        None => return Some(fresh),
        Some(Ok(t)) => serde_json::from_str::<Spend>(&t).ok(),
        Some(Err(())) => None,
    };
    match parsed {
        Some(s) if s.day == today => Some(s),
        Some(_) => Some(fresh),
        None if mtime_day.is_some_and(|d| d < today) => Some(fresh),
        None => None,
    }
}

/// 오늘 장부를 읽는다(화면용 — 못 읽으면 0). 날이 바뀌었으면 0으로 리셋한 새 장부를 돌려준다.
/// 🔴 한도 검사엔 쓰지 않는다 — `spend_for_reserve` 를 쓴다.
pub(crate) fn read_spend_today() -> Spend {
    let today = current_day();
    let s = spend_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Spend>(&s).ok())
        .unwrap_or_default();
    if s.day == today {
        s
    } else {
        Spend {
            day: today,
            usdc: "0".into(),
            eth: "0".into(),
            ..Default::default()
        }
    }
}

/// 단일 + 일일 누적 한도를 검사한다 (체인에 보내기 전에). 통과하면 Ok.
/// 한도가 0이면 "무제한"으로 보고 그 검사를 건너뛴다.
pub(crate) fn enforce_caps(
    amount: U256,
    single_cap: U256,
    spent_today: U256,
    daily_cap: U256,
    unit: &str,
    decimals: u8,
) -> Result<(), String> {
    if !single_cap.is_zero() && amount > single_cap {
        let s = format_units(single_cap, decimals).unwrap_or_default();
        return Err(tf!(
            "단일 거래 한도({s} {unit})를 초과했습니다",
            "Over the per-payment limit ({s} {unit})"
        ));
    }
    if !daily_cap.is_zero() && spent_today.saturating_add(amount) > daily_cap {
        let remaining = daily_cap.saturating_sub(spent_today);
        let r = format_units(remaining, decimals).unwrap_or_default();
        return Err(tf!(
            "오늘 한도를 초과했습니다 — 남은 한도 {r} {unit}",
            "Over today's limit — {r} {unit} left"
        ));
    }
    Ok(())
}

/// 오늘 누적 사용액을 보기 좋은 십진수로 돌려준다.
#[tauri::command]
pub(crate) fn get_today_spend() -> SpendView {
    let s = read_spend_today();
    let usdc_raw: U256 = s.usdc.parse().unwrap_or(U256::ZERO);
    let eth_raw: U256 = s.eth.parse().unwrap_or(U256::ZERO);
    SpendView {
        usdc: format_units(usdc_raw, active_chain().usdc_decimals).unwrap_or_else(|_| "0".into()),
        eth: format_ether(eth_raw),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 🔴 개발 71(코덱스 3차): 체인 확인 환불은 기록 하나당 한 번 — 같은 열쇠는 두 번째에 아무것도 안 한다. 다른 날·빈 열쇠도.
    #[test]
    fn refund_once_per_record() {
        let mut s = Spend {
            day: 7,
            usdc: "5000000".into(),
            eth: "0".into(),
            ..Default::default()
        };
        let one = U256::from(1_000_000u64);
        assert!(apply_refund_once(&mut s, "USDC", one, 7, "id-a"));
        assert!(!apply_refund_once(&mut s, "USDC", one, 7, "id-a"));
        assert_eq!(s.usdc, "4000000");
        assert!(!apply_refund_once(&mut s, "USDC", one, 6, "id-b")); // 어제 예약
        assert!(!apply_refund_once(&mut s, "USDC", one, 7, "")); // 열쇠 없음
        assert!(apply_refund_once(&mut s, "USDC", one, 7, "id-b"));
        assert_eq!(s.usdc, "3000000");
        // 새 날 장부는 목록도 비어 있다(직렬화에서도 빠진다).
        let fresh = judge_spend(None, None, 8).unwrap();
        assert!(fresh.refunded.is_empty());
        assert!(!serde_json::to_string(&fresh).unwrap().contains("refunded"));
    }

    // 🔴 개발 71(코덱스 1차 P1): 오늘 고친 장부를 못 읽으면 예약을 거부하고, 어제 이전 파일이면 새 날로 간다.
    #[test]
    fn unreadable_ledger_blocks_only_today() {
        let today = 20_000;
        let ok = |day: u64| {
            Some(Ok(format!(
                r#"{{"day":{day},"usdc":"19000000","eth":"0"}}"#
            )))
        };
        assert_eq!(
            judge_spend(ok(today), Some(today), today).unwrap().usdc,
            "19000000"
        );
        assert_eq!(
            judge_spend(ok(today - 1), Some(today - 1), today)
                .unwrap()
                .usdc,
            "0"
        );
        assert_eq!(judge_spend(None, None, today).unwrap().usdc, "0");
        assert!(judge_spend(Some(Ok("{깨짐".into())), Some(today), today).is_none());
        assert!(judge_spend(Some(Err(())), Some(today), today).is_none());
        assert!(judge_spend(Some(Err(())), None, today).is_none());
        assert_eq!(
            judge_spend(Some(Ok("{깨짐".into())), Some(today - 1), today)
                .unwrap()
                .usdc,
            "0"
        );
    }

    // 단일 한도 초과는 거부.
    #[test]
    fn enforce_caps_rejects_over_single() {
        let r = enforce_caps(
            U256::from(6_000_000u64), // 6 USDC
            U256::from(5_000_000u64), // 단일 5
            U256::ZERO,
            U256::from(20_000_000u64),
            "USDC",
            6,
        );
        assert!(r.is_err());
    }

    // 단일은 통과해도 오늘 누적이 일일 한도를 넘기면 거부.
    #[test]
    fn enforce_caps_rejects_over_daily() {
        // 단일 5 이하지만, 이미 18 썼고 4 더 보내면 22 > 20.
        let r = enforce_caps(
            U256::from(4_000_000u64),
            U256::from(5_000_000u64),
            U256::from(18_000_000u64),
            U256::from(20_000_000u64),
            "USDC",
            6,
        );
        assert!(r.is_err());
    }

    // 한도가 0이면 무제한 → 아무리 커도 통과.
    #[test]
    fn enforce_caps_zero_means_unlimited() {
        let r = enforce_caps(
            U256::from(999_000_000u64),
            U256::ZERO, // 단일 무제한
            U256::from(500_000_000u64),
            U256::ZERO, // 일일 무제한
            "USDC",
            6,
        );
        assert!(r.is_ok());
    }

    // 단일·일일 모두 안쪽이면 통과.
    #[test]
    fn enforce_caps_allows_within_limits() {
        let r = enforce_caps(
            U256::from(2_000_000u64),
            U256::from(5_000_000u64),
            U256::from(10_000_000u64),
            U256::from(20_000_000u64),
            "USDC",
            6,
        );
        assert!(r.is_ok());
    }

    // 음수 USDC 금액/한도는 거부돼야 한다 (parse_units 가 음수를 거대 U256 으로 둔갑시키는 함정 차단).
    #[test]
    fn parse_usdc_rejects_negative() {
        assert!(parse_usdc_nonneg("-1", 6).is_err());
        assert!(parse_usdc_nonneg("-0.000001", 6).is_err());
        assert!(parse_usdc_nonneg("  -5  ", 6).is_err());
        // 양수·0 은 정상.
        assert_eq!(
            parse_usdc_nonneg("1.5", 6).unwrap(),
            U256::from(1_500_000u64)
        );
        assert_eq!(parse_usdc_nonneg("0", 6).unwrap(), U256::ZERO);
    }

    // 음수 ETH 금액/한도도 거부 (parse_ether 도 내부적으로 get_absolute 를 거친다).
    #[test]
    fn parse_eth_rejects_negative() {
        assert!(parse_eth_nonneg("-1").is_err());
        assert!(parse_eth_nonneg("-.1").is_err());
        // 양수·0 은 정상.
        assert!(parse_eth_nonneg("0.05").unwrap() > U256::ZERO);
        assert_eq!(parse_eth_nonneg("0").unwrap(), U256::ZERO);
    }
}
