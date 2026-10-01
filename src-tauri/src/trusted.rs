// 신뢰 주소(화이트리스트) — 자율 결제 가드 (Session 16).
//
// 자율 한도(금액)만으로는 "처음 보는 주소"로도 비번 없이 나간다(악의적 엔드포인트가
// 소액으로 일일 한도까지 야금야금). 사람이 비번으로 승인/송금한 적 있는 받는 주소를
// 학습해 두고, 자율 승인은 그 주소들에만 허용한다(설정 auto_trusted_only, 기본 켜짐).
// 신뢰 목록 = 받는 주소(공개 정보)일 뿐 비밀 없음.

use std::fs;
use std::path::PathBuf;

use crate::chain::chain_file;
use crate::i18n::tf;
use crate::store::{jigap_dir, write_json};

// 화이트리스트도 체인별로 분리(chain_file) — 테스트넷에서 학습한 신뢰 주소가 메인넷 자율 결제를
// 비번 없이 통과시키면 안 된다(실돈 가드). 메인넷은 빈 목록에서 시작 → 첫 결제마다 비번 → 재학습.
fn trusted_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join(chain_file("trusted")))
}

/// 보기·판정용 — 못 읽으면 빈 목록(= 신뢰하는 곳 없음, 자율 결제가 닫히는 쪽이라 안전하다).
fn read_trusted() -> Vec<String> {
    read_trusted_checked().unwrap_or_default()
}

/// 고치기용 — 없으면 빈 목록, **있는데 못 읽으면 Err** (개발 73, 코덱스 1차 P1). 빈 목록으로 접으면 철회는 「지울 게 없다」며
/// 아무것도 안 쓰고 성공을 돌려줬고(다시 읽히면 그 주소가 자율 결제를 계속 통과), 학습은 깨진 파일을 한 줄로 덮었다.
fn read_trusted_checked() -> Result<Vec<String>, String> {
    let path = trusted_path()?;
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// 주소 비교는 항상 소문자로 — EIP-55 대소문자 표기 차이로 같은 주소가 갈라지지 않게.
fn normalize_addr(addr: &str) -> String {
    addr.trim().to_lowercase()
}

/// 목록에 추가 (빈 값·중복 무시). 바뀌었으면 true. 순수 함수 — 테스트용 분리.
fn add_trusted(list: &mut Vec<String>, addr: &str) -> bool {
    let a = normalize_addr(addr);
    if a.is_empty() || list.contains(&a) {
        return false;
    }
    list.push(a);
    true
}

/// 🔴 신뢰 목록의 읽기-수정-쓰기를 한 줄로 세운다(개발 70, 코덱스 1차 P2) — 승인 완료의 학습과 설정 화면의 철회가
/// 겹치면 나중에 쓴 쪽이 다른 쪽을 되돌렸다. 철회가 되돌려지면 그 주소는 계속 자율 결제를 통과한다.
/// 이 파일을 쓰는 건 GUI 뿐이라 프로세스 안 잠금이면 된다.
static TRUSTED_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn trusted_guard() -> std::sync::MutexGuard<'static, ()> {
    TRUSTED_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// 사람이 비번으로 승인한 송금/서명의 받는 주소를 학습한다. 기록 실패가 결제를 막진 않는다.
pub(crate) fn record_trusted(addr: &str) {
    let _g = trusted_guard();
    let Ok(mut list) = read_trusted_checked() else {
        return; // 못 읽는 목록을 한 줄로 덮지 않는다 — 학습을 한 번 건너뛸 뿐이다.
    };
    if add_trusted(&mut list, addr) {
        if let Ok(p) = trusted_path() {
            let _ = write_json(p, &list);
        }
    }
}

pub(crate) fn is_trusted(addr: &str) -> bool {
    read_trusted().contains(&normalize_addr(addr))
}

/// 승인 팝업 표시용 — 이 주소가 이전에 비번으로 승인된 적 있는지.
#[tauri::command]
pub(crate) fn is_trusted_addr(to: String) -> bool {
    is_trusted(&to)
}

/// 설정 화면 관리용 신뢰 주소 목록. 받는 주소 = 공개 정보라 비번 불필요.
#[tauri::command]
pub(crate) fn get_trusted_addrs() -> Vec<String> {
    read_trusted()
}

/// 목록에서 제거 (빈 값·없는 주소 무시). 바뀌었으면 true. 순수 함수 — 테스트용 분리.
fn remove_trusted(list: &mut Vec<String>, addr: &str) -> bool {
    let a = normalize_addr(addr);
    let before = list.len();
    list.retain(|x| x != &a);
    list.len() != before
}

/// 신뢰 철회 — 이후 이 주소로의 자율 결제는 다시 비번 승인이 필요하다.
#[tauri::command]
pub(crate) fn remove_trusted_addr(to: String) -> Result<(), String> {
    let _g = trusted_guard();
    let mut list = read_trusted_checked().map_err(|e| {
        tf!(
            "신뢰 주소 목록을 읽지 못해 지우지 못했어요: {e}",
            "Couldn't read the trusted-address list, so nothing was removed: {e}"
        )
    })?;
    if remove_trusted(&mut list, &to) {
        write_json(trusted_path()?, &list)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 신뢰 주소 학습: 대소문자 표기(EIP-55)가 달라도 같은 주소로 취급, 중복/빈 값 무시.
    #[test]
    fn trusted_list_normalizes_and_dedupes() {
        let mut list = Vec::new();
        assert!(add_trusted(&mut list, "0xAbCd1234"));
        assert!(!add_trusted(&mut list, "0xabcd1234")); // 같은 주소(소문자) → 중복
        assert!(!add_trusted(&mut list, "  0xABCD1234  ")); // 공백+대문자 → 중복
        assert!(!add_trusted(&mut list, "")); // 빈 값 무시
        assert_eq!(list, vec!["0xabcd1234"]);
        assert!(list.contains(&normalize_addr("0xAbCd1234")));
    }

    // 신뢰 철회: 대소문자 표기가 달라도 같은 주소를 제거, 없는 주소는 변경 없음.
    #[test]
    fn trusted_remove_normalizes() {
        let mut list = vec!["0xabcd1234".to_string(), "0xeeff5678".to_string()];
        assert!(remove_trusted(&mut list, "  0xABCD1234  ")); // 공백+대문자 → 같은 주소
        assert_eq!(list, vec!["0xeeff5678"]);
        assert!(!remove_trusted(&mut list, "0x없는주소")); // 없는 주소 → false
        assert!(!remove_trusted(&mut list, "")); // 빈 값 → false
        assert_eq!(list, vec!["0xeeff5678"]);
    }
}
