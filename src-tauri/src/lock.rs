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

/// 긴급 잠금 상태를 읽는다. 파일이 없거나 깨졌으면 "해제"로 본다.
pub(crate) fn read_lock() -> bool {
    lock_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<LockState>(&s).ok())
        .map(|l| l.locked)
        .unwrap_or(false)
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
}
