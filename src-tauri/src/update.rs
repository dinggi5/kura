// 인앱 자동 업데이트 (개발 31) — 검사·다운로드·설치.
//
// 왜 지갑에 자동 업데이트를 넣는가: 0.1.0 사용자는 보안 수정판이 나와도 직접 DMG 를 다시
// 받아 덮어쓰는 수밖에 없다. 실제로는 대부분 안 한다 → 고친 취약점이 사용자에게 안 닿는다.
//
// 그 대가로 **두 번째 신뢰 뿌리**가 생긴다. 업데이트 서명 개인키를 쥔 쪽은 이 앱으로 임의
// 코드를 밀어넣을 수 있고, 그 코드는 ~/.jigap/wallet.enc 를 읽는다. 그래서 이 모듈은
// 편의보다 통제를 우선한다:
//
//   1. **조용한 설치를 안 한다.** 검사와 설치가 분리된 커맨드이고, 설치는 사람이 버전과
//      릴리스 노트를 본 뒤 누를 때만 돈다. 자동으로 도는 건 "검사"까지다.
//   2. **웹뷰에 updater 권한을 안 준다.** capabilities 에 `updater:default` 를 넣지 않았다 —
//      넣으면 프론트가 `@tauri-apps/plugin-updater` 로 플러그인을 직접 호출할 수 있어
//      아래 가드를 전부 우회한다. 러스트 쪽 `app.updater()` 는 ACL 을 안 거치므로,
//      권한을 안 줘도 이 모듈은 동작한다. 프론트가 쓸 수 있는 건 이 파일의 커맨드뿐.
//   3. **서명 검증은 못 끈다.** 플러그인이 minisign 으로 강제한다(tauri.conf.json 의 pubkey).
//   4. **승인 대기 중엔 설치를 막는다.** 설치 끝에 앱을 재시작하는데, 그때 대기 중인 결제
//      요청이 있으면 그대로 죽는다 — MCP 쪽은 응답을 영영 못 받는다.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};

/// 프론트가 진행률을 받는 이벤트 이름. payload = `UpdateProgress`.
const PROGRESS_EVENT: &str = "update://progress";

/// `check_update` 가 찾은 업데이트를 `install_update` 까지 들고 있는 자리.
///
/// 왜 들고 있나: 설치는 검사와 별개 커맨드다(사람이 노트를 읽고 누르는 사이가 있다).
/// 다시 `check()` 를 부르면 그 사이 릴리스가 바뀌었을 때 **사용자가 승인한 버전과 다른 걸**
/// 설치하게 된다 — 지갑에서는 그 틈이 그대로 공격면이다. 보여준 그 객체로만 설치한다.
#[derive(Default)]
pub(crate) struct PendingUpdate(pub(crate) Mutex<Option<Update>>);

/// 프론트에 넘기는 업데이트 정보. 사람이 설치를 누르기 전에 보는 값이라,
/// 여기 있는 것만이 판단 근거가 된다.
#[derive(Serialize, Clone)]
pub(crate) struct UpdateInfo {
    /// 새 버전 (latest.json 의 version)
    pub(crate) version: String,
    /// 지금 돌고 있는 버전 — 프론트가 따로 물을 필요 없게 같이 준다
    pub(crate) current_version: String,
    /// 릴리스 노트. 없을 수 있다
    pub(crate) notes: Option<String>,
    /// 배포 시각 문자열. 없을 수 있다
    pub(crate) date: Option<String>,
}

#[derive(Serialize, Clone)]
struct UpdateProgress {
    downloaded: u64,
    /// 서버가 Content-Length 를 안 주면 없다 → 프론트는 불확정 진행 표시로 떨어진다
    total: Option<u64>,
}

/// 업데이트가 있는지 확인한다. 있으면 정보를 돌려주고 객체를 `PendingUpdate` 에 담아 둔다.
///
/// 네트워크가 없거나 엔드포인트가 안 뜨면 Err — 이건 정상적으로 자주 일어나는 일이라
/// 프론트는 조용히 무시한다(시작 시 자동 검사에서 에러 토스트가 뜨면 안 된다).
#[tauri::command]
pub(crate) async fn check_update(
    app: AppHandle,
    state: State<'_, PendingUpdate>,
) -> Result<Option<UpdateInfo>, String> {
    let updater = app
        .updater()
        .map_err(|e| format!("업데이트 확인을 준비하지 못했어요: {e}"))?;

    let found = updater
        .check()
        .await
        .map_err(|e| format!("업데이트 확인 실패: {e}"))?;

    // 락은 await 가 끝난 뒤에만 잡는다 (std Mutex 가드를 await 너머로 들고 가면 안 된다).
    let mut slot = state.0.lock().map_err(|_| "업데이트 상태가 깨졌어요".to_string())?;
    match found {
        Some(update) => {
            let info = UpdateInfo {
                version: update.version.clone(),
                current_version: update.current_version.clone(),
                notes: update.body.clone(),
                date: update.date.map(|d| d.to_string()),
            };
            *slot = Some(update);
            Ok(Some(info))
        }
        None => {
            // 최신이면 들고 있던 옛 후보를 버린다 — 안 그러면 릴리스가 내려간 뒤에도
            // 설치 버튼이 살아 있다.
            *slot = None;
            Ok(None)
        }
    }
}

/// 담아 둔 업데이트를 내려받아 설치하고 앱을 재시작한다.
/// 진행률은 `update://progress` 이벤트로 나간다.
#[tauri::command]
pub(crate) async fn install_update(
    app: AppHandle,
    state: State<'_, PendingUpdate>,
) -> Result<(), String> {
    // 🔴 설치는 재시작으로 끝난다. 승인 대기 중인 결제가 있으면 그 요청은 응답 없이 죽고,
    // MCP 쪽은 타임아웃까지 매달린다. 사람이 결정할 게 남아 있는 동안엔 앱을 안 내린다.
    if crate::ipc::has_pending() {
        return Err("승인 대기 중인 결제가 있어요. 먼저 처리한 뒤 업데이트하세요.".into());
    }

    // 객체를 꺼내 온다(락을 await 너머로 안 들고 가려고). 실패하면 아래에서 되돌려 놓는다 —
    // 다운로드가 한 번 끊겼다고 사용자가 검사부터 다시 하게 만들 이유가 없다.
    let update = {
        let mut slot = state.0.lock().map_err(|_| "업데이트 상태가 깨졌어요".to_string())?;
        slot.take()
            .ok_or_else(|| "설치할 업데이트가 없어요. 다시 확인해 주세요.".to_string())?
    };

    // 받기와 깔기를 **나눠서** 한다. `download_and_install` 한 방이면 위의 대기 검사와
    // 재시작 사이에 다운로드(수십 초)가 통째로 들어가는데, 그 사이 MCP 가 올린 결제 요청은
    // 검사에 안 걸린 채로 재시작에 휩쓸린다(코덱스 P1). 받아 놓고 다시 확인한 뒤 깐다.
    let progress_app = app.clone();
    let mut downloaded: u64 = 0;
    let bytes = match update
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                // 이벤트 전송 실패는 무시한다 — 진행률이 안 보이는 것뿐이고,
                // 그것 때문에 설치를 중단시킬 이유는 없다.
                let _ = progress_app.emit(PROGRESS_EVENT, UpdateProgress { downloaded, total });
            },
            || {},
        )
        .await
    {
        Ok(b) => b,
        Err(e) => return Err(restore_slot(&state, update, format!("업데이트 내려받기 실패: {e}"))),
    };

    // 🔴 다시 확인한다. 여기까지가 되돌릴 수 있는 마지막 지점이다 — 아직 앱을 안 건드렸고,
    // 받아 둔 바이트는 메모리에만 있다. 요청을 처리한 뒤 다시 누르면 된다.
    if crate::ipc::has_pending() {
        return Err(restore_slot(
            &state,
            update,
            "내려받는 사이에 결제 승인 요청이 들어왔어요. 먼저 처리한 뒤 다시 시도하세요.".into(),
        ));
    }

    if let Err(e) = update.install(bytes) {
        return Err(restore_slot(&state, update, format!("업데이트 설치 실패: {e}")));
    }

    // 여기까지 왔으면 새 번들이 디스크에 깔렸다. 재시작해야 새 코드가 돈다 —
    // 안 하면 사용자는 "설치됐다"는 말을 보고도 옛 버전을 계속 쓴다.
    //
    // install() 과 restart() 사이에도 요청이 들어올 틈은 남아 있다. 다만 그건 밀리초 단위고,
    // 무엇보다 **이미 깔린 뒤라 되돌릴 수가 없다** — 여기서 안 죽으면 구버전이 계속 도는
    // 더 나쁜 상태가 된다. 줄일 수 있는 창(다운로드)을 줄이는 것이 이 분리의 목적이다.
    // restart() 는 돌아오지 않는다(-> !).
    app.restart()
}

/// 실패한 업데이트를 대기 슬롯에 되돌려 놓고 에러 메시지를 그대로 돌려준다.
/// 사용자가 승인한 그 버전으로 재시도할 수 있게 — 검사부터 다시 하게 만들 이유가 없다.
fn restore_slot(state: &State<'_, PendingUpdate>, update: Update, msg: String) -> String {
    if let Ok(mut slot) = state.0.lock() {
        *slot = Some(update);
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    // 대기 슬롯의 기본값은 비어 있어야 한다 — 검사 전에 설치가 되면 안 된다.
    #[test]
    fn pending_update_starts_empty() {
        let p = PendingUpdate::default();
        assert!(p.0.lock().unwrap().is_none());
    }
}
