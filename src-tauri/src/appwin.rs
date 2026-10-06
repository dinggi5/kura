// 「창으로 열기」 (개발 75) — 팝오버(420×640) 옆에 두는 보통 맥 창.
//
// 설계 결정:
//   · **창이 둘이다** — 메뉴바 팝오버(`main`)와 이 창(`window`). 팝오버를 키웠다 줄였다 하는
//     한 창 방식은 버렸다: 테두리 없는 투명 창에 런타임으로 테두리를 붙였다 떼는 건 macOS 에서
//     흔들리고, 무엇보다 결제 승인이 그 큰 창 안에 뜨게 된다. 1Password 의 미니/메인 창 구조처럼
//     **돈이 나가는 순간은 작은 팝오버가 늘 위에서** 받고, 큰 창은 보고·고르는 곳이다.
//   · 그래서 **결제 승인은 팝오버만** 한다 — 이 창의 프론트는 승인 모달·자율 승인을 안 돌린다
//     (src/lib/win.ts `isWindow`). 두 창이 같은 요청을 동시에 자율 승인하려 드는 갈래가 원천적으로 없다.
//   · 닫으면 **없앤다**(숨기지 않는다). 팝오버는 결제 요청을 받으려고 늘 살아 있어야 하지만 이 창은
//     아니다 — 숨겨 두면 1초 폴링이 하나 더 도는 셈이고, 다시 열 때 옛 화면 상태가 남는다.
//   · 상태는 백엔드가 정본이라 두 창이 같은 값을 본다. 한쪽에서 바꾼 걸 다른 쪽이 다시 읽는 건
//     프론트의 `kura-sync` 이벤트와 포커스 복귀가 맡는다(src/lib/sync.ts).

use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// 창 라벨 — capabilities/default.json 의 `windows` 에도 같은 이름이 있어야 커맨드가 닿는다.
pub(crate) const LABEL: &str = "window";

/// 처음 여는 크기(논리 px). 가운데 한 단(448) + 넓은 여백, 내역 표(최대 768)가 들어가는 폭.
const OPEN_W: f64 = 960.0;
const OPEN_H: f64 = 720.0;
/// 이보다 좁히면 팝오버와 다를 게 없다 — 팝오버 폭(420)보다 조금 넓게.
const MIN_W: f64 = 480.0;
const MIN_H: f64 = 560.0;
// 최소 폭이 팝오버(420)보다 좁으면 「창으로 열기」의 이유가 사라진다 — 컴파일에서 막는다.
const _: () = assert!(MIN_W > 420.0 && OPEN_W >= MIN_W && OPEN_H >= MIN_H);

/// 창을 연다. 이미 있으면 앞으로 가져온다.
pub(crate) fn open<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::default())
        .title("Kura")
        .inner_size(OPEN_W, OPEN_H)
        .min_inner_size(MIN_W, MIN_H)
        .resizable(true)
        .center()
        // 신호등만 남기고 제목 막대는 내용과 한 판으로 — 프론트가 맨 위 띠를 끌기 영역으로 둔다.
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .build();
    match built {
        Ok(w) => {
            let _ = w.set_focus();
        }
        Err(e) => eprintln!("[kura] 창을 못 열었어요: {e}"),
    }
}

/// 이 창이 떠 있으면 앞으로 가져오고 true. 독 아이콘 클릭이 쓴다.
pub(crate) fn focus_if_open<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(w) = app.get_webview_window(LABEL) else {
        return false;
    };
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
    true
}

/// Kura 의 창 중 하나라도 포커스를 쥐고 있는가 — 「자리비움 잠금」이 창 사이 이동을 자리비움으로
/// 착각하지 않게(팝오버 → 큰 창으로 옮기는 순간 팝오버는 blur 를 받는다).
pub(crate) fn any_focused<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.webview_windows()
        .values()
        .any(|w| w.is_focused().unwrap_or(false))
}

/// 큰 창을 지금 닫으면 안 되는가 — 화면에서 직접 보낸 송금이 나가는 중이다 (개발 75, 코덱스 1차 P1).
///
/// 송금은 백엔드에서 끝까지 가지만 결과(해시·「불명」 경고)를 받을 화면은 이 창의 보내기 카드뿐이다. 닫으면
/// 사람은 결과를 못 보고 다시 보낸다 — 이중 송금. 그래서 송금이 끝날 때까지(최대 한 건의 채우기+제출) 안 닫힌다.
/// 어느 창이 보낸 건지는 가리지 않는다 — 다른 창의 송금 때문에 잠깐 안 닫히는 건 손해가 작다.
pub(crate) fn busy_sending() -> bool {
    crate::transfer::manual_send_in_flight()
}

/// 팝오버의 확장 아이콘이 부른다.
#[tauri::command]
pub(crate) fn open_app_window(app: AppHandle) {
    open(&app);
}

/// 큰 창에서 「승인하러 가기」 — 대기 중인 결제 요청을 팝오버로 띄운다.
///
/// `raise_main_window` 와 달리 「닫아 둠」(개발 53)을 따지지 않는다: 그 표식은 「지금은 띄우지 마」
/// 였고, 이 버튼은 사람이 「이제 띄워」라고 누른 것이다. 띄우면 `tray::show` 가 표식을 지운다.
#[tauri::command]
pub(crate) fn show_approval(app: AppHandle) {
    if crate::ipc::live_request().is_none() {
        return;
    }
    crate::tray::set_pinned(&app, true);
    crate::tray::show(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    // 🔴 드리프트 가드: 라벨이 capabilities 의 windows 목록에 없으면 이 창의 **모든 커맨드가
    // 권한 오류로 막힌다**(창은 뜨는데 잔액·내역이 비어 있는 상태). 한쪽만 고치는 사고를 막는다.
    #[test]
    fn label_is_in_capabilities() {
        let cap: serde_json::Value = serde_json::from_str(include_str!("../capabilities/default.json"))
            .expect("capabilities 파싱");
        let wins = cap["windows"].as_array().expect("windows 목록");
        assert!(wins.iter().any(|w| w == LABEL), "capabilities 에 {LABEL} 가 없다");
        assert!(wins.iter().any(|w| w == "main"), "팝오버 권한이 빠졌다");
        let perms = cap["permissions"].as_array().expect("permissions");
        assert!(
            perms.iter().any(|p| p == "core:window:allow-start-dragging"),
            "제목 막대를 숨겼으니 끌기 권한이 있어야 창을 옮길 수 있다"
        );
    }
}
