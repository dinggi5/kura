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
// 그래서 희망값(settings.autostart)을 따로 적고, 시작할 때 대조한다.
//
// **되살려도 되는 근거** (개발 31 코덱스 P2 반영). 처음엔 "버전이 바뀐 실행에서만"으로
// 좁혔다. 사용자가 직접 끈 걸 앱이 다시 켜는 게 무서웠기 때문인데, 그 게이트는 필요도
// 없으면서 `brew reinstall`(같은 버전)에서 복구를 놓치는 순수한 손해였다.
//
// auto-launch 0.5.0 의 LaunchAgent 모드에서 `is_enabled()` 는 **plist 파일이 있느냐**이고
// `disable()` 은 그 파일을 지운다(`macos.rs:145,162`). 즉 파일이 사라지는 경로는 둘뿐이다:
//   ① 우리 토글(set_autostart) — 이때 희망값도 **같이** 끔으로 적는다
//   ② 패키지 관리자(캐스크 uninstall launchctl) — 희망값은 켬으로 남는다
// 그래서 "희망값이 켬인데 파일이 없다" 는 곧 ②다. 되살리는 게 맞다.
//
// macOS 시스템 설정 > 로그인 항목의 토글은 launchd 의 비활성 DB 에 기록할 뿐 남의 plist
// 파일을 지우지 않는다 → 그 경우 `is_enabled()` 는 계속 true 라 이 분기에 아예 안 들어온다.
// (이건 소스를 읽고 내린 판단이고 실물로 재보진 않았다 — DEVLOG "검증 안 한 것" 참고.)

use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;

use crate::settings::{read_settings_for_update, save_settings};

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
    // 🔴 희망값을 **먼저** 적는다. reconcile 이 "희망=켬 + 파일 없음 → 되살린다" 로 도는데,
    // 순서가 반대면 이런 창이 생긴다: OS 를 껐는데 저장이 실패 → 희망값은 켬으로 남음 →
    // 다음 실행에서 방금 끈 자동 시작이 되살아난다.
    //
    // 해석 안 되는 설정 파일 위에는 안 쓴다(read_settings_for_update 주석 참고).
    let before = read_settings_for_update();
    if let Some(s) = &before {
        let mut next = s.clone();
        next.autostart = Some(enabled);
        let _ = save_settings(&next);
    }

    let result = os_set(&app, enabled);

    // 🔴 켜기가 실패했으면 희망값을 되돌린다.
    // 안 그러면 다음 실행에서 reconcile 이 "희망=켬 + 파일 없음"을 보고 **패키지 관리자가
    // 지웠다**고 오해해서 켜 버린다 — 방금 화면은 사용자에게 "실패했다"고 말했는데도.
    // 지갑이 실패했다고 알린 뒤 조용히 로그인 항목을 얻는 건, 이 복구 기능이 애초에
    // 피하려던 바로 그 행동이다.
    if enabled && result.is_err() {
        if let Some(s) = before {
            let _ = save_settings(&s);
        }
    }

    // 저장 실패 자체는 삼키고 OS 반영 결과만 올린다. 이 커맨드의 약속은 "OS 설정을 바꾼다"고,
    // 저장이 안 됐다고 Err 를 내면 프론트가 낙관적 토글을 되돌려 **실제로 바뀐 상태를
    // 안 바뀐 것처럼** 보여준다(= 화면이 거짓말을 한다).
    result
}

/// 되살릴 것인가. reconcile 의 판단만 떼어낸 순수 함수 —
/// 여기가 이 기능에서 유일하게 미묘한 부분이라 테스트로 못을 박아 둔다.
///
/// - `want`: 희망값 (None = 아직 모름 — 이 필드가 없던 시절 설정이거나 첫 실행)
/// - `os_on`: 지금 OS 로그인 아이템(plist)이 있는지
///
/// 근거는 파일 머리말 참고. 요약하면 **"희망=켬 + 파일 없음" = 패키지 관리자가 지웠다**이다.
fn should_restore(want: Option<bool>, os_on: bool) -> bool {
    want == Some(true) && !os_on
}

/// 시작할 때 한 번 — OS 상태와 희망값을 맞춘다.
///
/// - 희망값 "켬" + OS "꺼짐" → 되살린다 (업데이트·재설치가 지우고 간 경우)
/// - 그 외 → **OS 가 진실 원천**. 지금 상태를 희망값으로 채택한다
///
/// 실패는 전부 조용히 넘긴다. 자동 시작 기록 때문에 앱이 안 뜨면 그게 더 큰 손해다.
pub(crate) fn reconcile(app: &AppHandle) {
    let Ok(cur) = os_enabled(app) else {
        // OS 상태를 못 읽으면 아무것도 안 한다. 모르는 채로 희망값을 덮으면 기록이 오염된다.
        return;
    };

    // 🔴 파일이 있는데 해석이 안 되면 손대지 않는다. 여기가 **시작할 때마다** 도는
    // 읽고-쓰기 경로라, 기본값을 덮어쓰면 사용자의 한도·RPC·chain_id 가 조용히 사라진다
    // (메인넷 사용자가 테스트넷으로 돌아간다). 자동 시작 기록보다 그쪽이 훨씬 비싸다.
    let Some(mut s) = read_settings_for_update() else {
        return;
    };

    if should_restore(s.autostart, cur) {
        // 여기만이 앱이 사용자 대신 OS 설정을 바꾸는 자리다.
        if os_set(app, true).is_err() {
            // 못 켰으면 희망값을 그대로 둔다 → 다음 실행에서 다시 시도.
            return;
        }
        s.autostart = Some(true);
    } else {
        s.autostart = Some(cur);
    }

    let _ = save_settings(&s);
}

#[cfg(test)]
mod tests {
    use super::should_restore;

    // 고치려던 그 버그: 켜 뒀는데 패키지 관리자가 LaunchAgent 를 지웠다 → 되살린다.
    //
    // `brew upgrade`(버전 바뀜)와 `brew reinstall`(버전 그대로)이 **같은 한 줄로** 잡히는 게
    // 요점이다. 처음엔 "버전이 바뀐 실행에서만" 되살리게 짰다가 재설치를 놓쳤다(코덱스 P2) —
    // 판단에서 버전을 빼자 두 경우가 하나가 됐다. 그래서 이 테스트도 하나면 된다.
    #[test]
    fn restores_what_package_manager_wiped() {
        assert!(should_restore(Some(true), false));
    }

    // 희망값이 "끔"이면 켜지 않는다. 우리 토글로 끈 경우가 여기다 —
    // set_autostart 가 OS 를 건드리기 **전에** 희망값을 적으므로 이 값은 믿을 수 있다.
    #[test]
    fn never_enables_what_user_did_not_want() {
        assert!(!should_restore(Some(false), false));
    }

    // 아직 모르는 상태(옛 설정 파일·첫 실행)에서는 아무것도 안 한다 — OS 상태를 채택할 뿐.
    #[test]
    fn does_nothing_when_wish_unknown() {
        assert!(!should_restore(None, false));
        assert!(!should_restore(None, true));
    }

    // 이미 켜져 있으면 할 일이 없다.
    #[test]
    fn no_op_when_already_on() {
        assert!(!should_restore(Some(true), true));
        assert!(!should_restore(Some(false), true));
    }
}
