// 긴급 잠금 (Session 8) — 켜지면 모든 송금·서명이 차단된다 (AI 폭주·키 노출 대비 비상 스위치).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::session::SessionKey;
use crate::store::{jigap_dir, write_json};

#[derive(Serialize, Deserialize, Default)]
struct LockState {
    locked: bool,
}

fn lock_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join("lock.json"))
}

/// 긴급 잠금 상태를 읽는다. 파일이 없으면 "해제"(한 번도 안 켰다).
/// 🔴 **있는데 못 읽거나 깨졌으면 "잠금"** (개발 70, 코덱스 1차) — 예전엔 "해제"로 봐서, 잠금을 켠 뒤 파일이 상하면
/// 비상 스위치가 조용히 풀렸다. 갇히지는 않는다: 해제 버튼(`set_locked(false)`)이 새 파일을 쓴다.
pub(crate) fn read_lock() -> bool {
    lock_path().map(|p| lock_at(&p)).unwrap_or(true)
}

fn lock_at(path: &std::path::Path) -> bool {
    match fs::read_to_string(path) {
        Ok(s) => serde_json::from_str::<LockState>(&s)
            .map(|l| l.locked)
            .unwrap_or(true),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// 긴급 잠금 상태를 알려준다 (비번 불필요).
#[tauri::command]
pub(crate) fn is_locked() -> bool {
    read_lock()
}

/// 긴급 잠금을 켜고 끈다. 켜져 있으면 send_eth/send_usdc가 즉시 거부한다.
/// 켤 때는 자율 결제용 세션 키도 즉시 메모리에서 소멸시킨다(비상 스위치 = 자율 결제도 멈춤).
/// 저장에 성공하면 메뉴바 아이콘도 잠금/해제 모양으로 바꾼다(개발 26).
#[tauri::command]
pub(crate) fn set_locked(
    app: tauri::AppHandle,
    locked: bool,
    session: tauri::State<'_, SessionKey>,
) -> Result<(), String> {
    if locked {
        if let Ok(mut g) = session.0.lock() {
            *g = None;
        }
    }
    write_json(lock_path()?, &LockState { locked })?;
    crate::tray::refresh_icon(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 긴급 잠금 기본값은 "해제".
    #[test]
    fn lock_state_default_is_unlocked() {
        assert!(!LockState::default().locked);
    }

    // 없으면 해제, 깨졌으면 잠금(비상 스위치는 닫힌 쪽으로 실패한다).
    #[test]
    fn missing_is_unlocked_broken_is_locked() {
        let dir = std::env::temp_dir().join(format!("kura-lock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("lock.json");
        assert!(!lock_at(&p));
        fs::write(&p, r#"{"locked":false}"#).unwrap();
        assert!(!lock_at(&p));
        fs::write(&p, r#"{"locked":true}"#).unwrap();
        assert!(lock_at(&p));
        fs::write(&p, r#"{"lock"#).unwrap();
        assert!(lock_at(&p));
        let _ = fs::remove_dir_all(&dir);
    }
}
