// 로그인 시 자동 시작 (Session 18) + 희망값 보존·복구 (개발 31).
//
// Session 18 에서는 "OS 로그인 아이템이 진실 원천"이었다 — 앱은 아무것도 기억하지 않고
// 켤 때 `is_enabled()` 를 물었다. 상태를 한 곳에만 두는 건 옳았지만, **그 한 곳을 우리가
// 아닌 것도 지운다**는 게 문제였다.
//
// 캐스크의 `uninstall launchctl: "Kura"` 는 삭제뿐 아니라 `brew upgrade`·`brew reinstall`
// 에서도 돈다. 즉 업데이트 한 번에 LaunchAgent 가 사라지고, 앱은 "꺼짐"만 보게 된다 —
// 사용자가 켜 뒀다는 사실은 어디에도 안 남아 있으니 되살릴 수도 없다(개발 30 코덱스 P2).
//
// 그래서 희망값(settings.autostart)을 따로 적고, 시작할 때 대조한다. 다만 **무조건 되살리진
// 않는다** — 시스템 설정 > 로그인 항목에서 사용자가 직접 끈 것을 앱이 매번 다시 켜면
// 그건 지갑이 아니라 악성 소프트웨어의 행동이다. 되살리는 건 "버전이 바뀐 실행",
// 즉 방금 업데이트가 지나갔다고 확신할 수 있을 때뿐이다(reconcile).

use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;

use crate::settings::{read_settings, save_settings};

fn os_enabled(app: &AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

fn os_set(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let l = app.autolaunch();
    if enabled { l.enable() } else { l.disable() }.map_err(|e| e.to_string())
}

/// 현재 자동 시작 여부. **OS 상태를 그대로 돌려준다** — 희망값이 아니라.
/// 설정 화면의 토글은 지금 실제로 어떤지를 보여야 한다(희망값은 복구 판단에만 쓴다).
#[tauri::command]
pub(crate) fn get_autostart(app: AppHandle) -> Result<bool, String> {
    os_enabled(&app)
}

#[tauri::command]
pub(crate) fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    // OS 부터 바꾼다. 실패하면 희망값도 안 적는다 — 켜지지도 않았는데 희망값만 남으면
    // 다음 업데이트에서 사용자가 켠 적 없는 자동 시작이 "복구"된다.
    os_set(&app, enabled)?;

    let mut s = read_settings();
    s.autostart = Some(enabled);
    // 저장 실패는 삼킨다. 이 커맨드의 약속은 "OS 설정을 바꾼다"이고 그건 이미 성공했다 —
    // 여기서 Err 를 올리면 프론트가 낙관적 토글을 되돌려, 실제로 바뀐 상태를 안 바뀐 것처럼
    // 보여준다(= 화면이 거짓말을 한다). 못 적었을 때의 대가는 "다음 업데이트에서 복구 안 됨"
    // 하나뿐이고, 그건 개발 30 이전의 동작이다.
    let _ = save_settings(&s);
    Ok(())
}

/// 되살릴 것인가. reconcile 의 판단만 떼어낸 순수 함수 —
/// 여기가 이 기능에서 유일하게 미묘한 부분이라 테스트로 못을 박아 둔다.
///
/// - `last_run`: 설정에 적힌 마지막 실행 버전 (None = 이 필드가 없던 시절 설정이거나 첫 실행)
/// - `want`: 희망값 (None = 아직 모름)
/// - `os_on`: 지금 OS 로그인 아이템이 켜져 있는지
/// - `version`: 지금 앱 버전
fn should_restore(last_run: Option<&str>, want: Option<bool>, os_on: bool, version: &str) -> bool {
    // 버전이 바뀐 실행 = 방금 업데이트가 지나갔다 → 꺼진 이유를 우리가 안다.
    // last_run 이 None 이면 업데이트로 안 친다(희망값도 같이 None 이라 복구할 근거가 없다).
    let updated = last_run.is_some_and(|v| v != version);
    updated && want == Some(true) && !os_on
}

/// 시작할 때 한 번 — OS 상태와 희망값을 맞춘다.
///
/// - 버전이 바뀐 실행(=방금 업데이트) + 희망값 "켬" + OS "꺼짐" → 되살린다
/// - 그 외 → **OS 가 진실 원천**. 사용자가 시스템 설정에서 바꾼 걸 희망값으로 채택한다
///
/// 실패는 전부 조용히 넘긴다. 자동 시작 기록 때문에 앱이 안 뜨면 그게 더 큰 손해다.
pub(crate) fn reconcile(app: &AppHandle) {
    let version = app.package_info().version.to_string();
    let Ok(cur) = os_enabled(app) else {
        // OS 상태를 못 읽으면 아무것도 안 한다. 모르는 채로 희망값을 덮으면 기록이 오염된다.
        return;
    };

    let mut s = read_settings();

    if should_restore(s.last_run_version.as_deref(), s.autostart, cur, &version) {
        // 여기만이 앱이 사용자 대신 OS 설정을 바꾸는 자리다.
        if os_set(app, true).is_err() {
            // 못 켰으면 희망값도 last_run_version 도 그대로 둔다 → 다음 실행에서 다시 시도.
            return;
        }
        s.autostart = Some(true);
    } else {
        s.autostart = Some(cur);
    }

    s.last_run_version = Some(version);
    let _ = save_settings(&s);
}

#[cfg(test)]
mod tests {
    use super::should_restore;

    // 고치려던 그 버그: 켜 뒀는데 업데이트가 지나가며 LaunchAgent 를 지웠다 → 되살린다.
    #[test]
    fn restores_after_update_wiped_it() {
        assert!(should_restore(Some("0.1.1"), Some(true), false, "0.1.2"));
    }

    // 🔴 사용자가 시스템 설정 > 로그인 항목에서 직접 끈 경우. 버전이 그대로면 손대지 않는다 —
    // 지갑 앱이 사용자가 끈 걸 매번 다시 켜면 그건 악성 소프트웨어의 행동이다.
    #[test]
    fn leaves_user_turned_off_alone() {
        assert!(!should_restore(Some("0.1.2"), Some(true), false, "0.1.2"));
    }

    // 희망값이 "끔"이면 업데이트가 지나가도 켜지 않는다.
    #[test]
    fn never_enables_what_user_did_not_want() {
        assert!(!should_restore(Some("0.1.1"), Some(false), false, "0.1.2"));
        assert!(!should_restore(Some("0.1.1"), None, false, "0.1.2"));
    }

    // 이미 켜져 있으면 할 일이 없다.
    #[test]
    fn no_op_when_already_on() {
        assert!(!should_restore(Some("0.1.1"), Some(true), true, "0.1.2"));
    }

    // 0.1.0 → 0.1.1 업그레이드의 실제 모습: 0.1.0 은 이 필드들을 안 썼으므로 둘 다 None 이다.
    // 이 한 번은 복구가 안 된다(희망값이 어디에도 안 적혀 있었다). 0.1.1 부터 보존된다 —
    // README 업데이트 절에 같은 말을 적어 뒀다.
    #[test]
    fn first_upgrade_from_unaware_version_cannot_restore() {
        assert!(!should_restore(None, None, false, "0.1.1"));
    }
}
