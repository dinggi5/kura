// 메뉴바 상주 (개발 26) — 트레이 아이콘 + 팝오버 창 제어.
//
// 설계 결정:
//   · 도크 아이콘은 그대로 둔다(ActivationPolicy 를 Accessory 로 바꾸지 않음). 결제 승인 팝업을
//     앞으로 끌어올리는 경로를 도크·트레이 둘로 남겨 두려는 것 — 승인 팝업이 안 뜨면 5분
//     타임아웃으로 결제가 거부되므로(lib.rs raise_main_window 주석의 실사례) 이 경로는 여유 있게.
//   · 팝오버 위치는 TrayIcon::rect() 로 직접 계산한다 → 위치 플러그인 의존성 0.
//   · 승인 대기 중에는 blur 로 창을 숨기지 않는다(hold). 비번 입력 중 다른 창을 클릭해 팝오버가
//     사라지면 승인 자체를 못 하게 되기 때문.
//   · 단 **사용자가 일부러 닫는 건 존중한다**(개발 53) — 트레이 클릭·Cmd+W 는 승인 대기 중에도
//     창을 숨긴다(승인 전에 브라우저를 잠깐 봐야 할 때가 있다). 그 요청에 한해 「닫아 둠」을
//     기억해 감시 스레드가 다시 띄우지 않고, 만료 직전 한 번만 되살린다(REMIND_SECS).
//     blur 는 여전히 닫기가 아니다 — 다른 창을 클릭하는 건 「잠깐 저쪽」이지 「치워」가 아니다.

use crate::i18n::ts;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime, WebviewWindow, Window,
};

/// 트레이 아이콘 id — rect() 조회·아이콘 교체 때 다시 찾으려고 고정한다.
const TRAY_ID: &str = "kura";

/// 팝오버와 메뉴바 사이 여백(논리 px). **0 = 메뉴바 경계에 딱 붙임**.
/// 네이티브 메뉴바 앱들(AdGuard 등)이 작업 영역 상단에 그대로 붙어 있어서, 몇 px 만 띄워도
/// 나란히 놓고 보면 혼자 처져 보인다(라이브 비교로 6 → 4 → 0 으로 좁혀 맞춘 값).
/// 그림자는 창 프레임 바깥에 그려지므로 붙여도 죽지 않는다.
const TRAY_GAP: f64 = 0.0;
/// 화면 가장자리 최소 여백 — 트레이가 오른쪽 끝일 때 팝오버가 잘리지 않게.
const SCREEN_MARGIN: f64 = 8.0;

/// 창 기본 크기(논리 px) — **tauri.conf.json 의 width/height 와 같아야 한다.**
/// 지금 창 크기를 읽지 않고 이 값을 기준으로 삼는 이유: 좁은 화면에서 한 번 줄인 뒤
/// 넓은 화면으로 옮겼을 때 원래 크기로 되돌아와야 하기 때문.
const WINDOW_W: f64 = 420.0;
const WINDOW_H: f64 = 640.0;

/// blur 로 숨긴 직후 들어오는 트레이 클릭은 "닫기"로 본다. macOS 는 트레이를 누르면 창이 먼저
/// 포커스를 잃으므로, 이 유예가 없으면 열린 팝오버를 클릭해도 닫혔다 곧바로 다시 열린다.
const REOPEN_GUARD: Duration = Duration::from_millis(250);

const ICON_NORMAL: &[u8] = include_bytes!("../icons/tray-normal.png");
const ICON_LOCKED: &[u8] = include_bytes!("../icons/tray-locked.png");

/// 닫아 둔 승인 창을 만료 이만큼 전에 한 번 되살린다(초). 개발 53.
///
/// 사용자가 닫은 건 「지금 말고」지 「영영」이 아니다 — 그런데 MCP 는 5분이 지나면 요청을 거둬
/// 가고, 닫힌 채로는 그 사실을 알 길이 없다(개발 49 가 막으려던 «조용한 실패» 그대로).
/// 그래서 한 번, 비번을 넣기에 충분한 시간을 남기고 되살린다. 다시 닫으면 그걸로 끝이다.
/// 5분 자체를 늘리지 않는 이유: 그 값은 두 크레이트(앱·사이드카)의 파일 계약이자 승인 창
/// 카운트다운이자 AI 클라이언트 쪽 도구 타임아웃과도 맞물려 있다.
pub(crate) const REMIND_SECS: u64 = 60;

/// 팝오버 런타임 상태.
#[derive(Default)]
pub(crate) struct PopoverState {
    /// blur 로 자동으로 숨긴 시각 (REOPEN_GUARD 참고).
    last_auto_hide: Mutex<Option<Instant>>,
    /// 사용자가 **일부러 닫아 둔** 승인 요청의 id (개발 53). 요청 id 단위라 다음 요청은
    /// 평소대로 창을 깨운다. 어떤 경로로든 창이 다시 뜨면(`show`) 지운다 — 그 뒤 또 닫으면
    /// 그때 다시 기록된다.
    dismissed: Mutex<Option<String>>,
    /// 만료 직전 한 번 되살린 요청의 id — 같은 요청을 두 번 되살리지 않는다.
    reminded: Mutex<Option<String>>,
}

/// 사용자가 닫아 둔 요청인가 (개발 53).
pub(crate) fn user_dismissed<R: Runtime>(app: &AppHandle<R>, req_id: &str) -> bool {
    app.state::<PopoverState>()
        .dismissed
        .lock()
        .map(|g| g.as_deref() == Some(req_id))
        .unwrap_or(false)
}

/// 닫아 둔 승인 창을 지금 어떻게 할지 (순수 — 테스트 가능).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Dismissal {
    /// 닫아 둔 요청이 아니다 — 평소 규칙(잠든 프론트면 깨운다)대로.
    No,
    /// 닫아 둔 채 둔다.
    Hold,
    /// 만료 직전 — 한 번 되살린다.
    Remind,
}

/// `dismissed`·`reminded` = 상태에 적힌 id, `req_id` = 지금 대기 중인 요청, `secs_left` = 그
/// 요청의 남은 승인 시간. 이미 0 이면 되살리지 않는다 — MCP 가 곧 거둬 갈 요청을 띄워 봐야
/// 비번을 넣는 순간 「시간이 지났어요」다(살아 있는 요청으로 잡히는 유예 60초 동안의 일).
pub(crate) fn dismissal(
    dismissed: Option<&str>,
    reminded: Option<&str>,
    req_id: &str,
    secs_left: u64,
) -> Dismissal {
    if dismissed != Some(req_id) {
        return Dismissal::No;
    }
    if reminded == Some(req_id) || secs_left == 0 || secs_left > REMIND_SECS {
        return Dismissal::Hold;
    }
    Dismissal::Remind
}

/// 감시 스레드용: 닫아 둔 요청을 지금 되살려야 하면 **되살린 것으로 적고** Remind 를 돌려준다.
/// 적는 것과 판단을 한 잠금 순서로 묶어 두 번 되살리는 일이 없게 한다(스레드는 하나지만).
pub(crate) fn dismissal_for<R: Runtime>(
    app: &AppHandle<R>,
    req_id: &str,
    secs_left: u64,
) -> Dismissal {
    let st = app.state::<PopoverState>();
    let (Ok(d), Ok(mut r)) = (st.dismissed.lock(), st.reminded.lock()) else {
        return Dismissal::No;
    };
    let verdict = dismissal(d.as_deref(), r.as_deref(), req_id, secs_left);
    if verdict == Dismissal::Remind {
        *r = Some(req_id.to_string());
    }
    verdict
}

/// 승인 대기 중인가 = 지금 대기 중인 결제 요청이 있는가.
///
/// 프론트가 "고정해줘/풀어줘"를 보내는 방식이 아니라 **백엔드 상태에서 파생**시킨다.
/// 신호 방식은 raise/release 가 각각 비동기라 순서가 뒤집히면(요청이 즉시 처리될 때 등)
/// 게이트가 영구히 켜진 채(팝오버가 영영 안 닫힘) 또는 꺼진 채(승인 창이 사라짐) 남는다.
/// 상태 파생은 그런 경합이 원천적으로 없고, 파일 하나 stat 이라 blur 마다 불러도 싸다.
fn is_held() -> bool {
    crate::ipc::has_pending()
}

/// "항상 위" 고정을 원하는 값으로 맞춘다. 1초 폴링(get_pending_request)마다 불려
/// 어긋난 상태를 스스로 고친다 — raise/release 두 커맨드의 도착 순서가 뒤집혀도
/// 최대 1초 뒤엔 올바른 값으로 수렴한다.
///
/// 일부러 캐시(“값이 바뀔 때만 호출”)를 두지 않는다. 캐시를 두면 창이 아직 없거나 호출이
/// 실패한 순간의 값이 적용된 것처럼 남아 이후 재시도를 영구히 막는다 — 자가 치유하려고 만든
/// 함수가 반대로 고착의 원인이 된다. 초당 setLevel 한 번은 무시할 수 있는 비용이라
/// 매번 무조건 적용하는 쪽이 옳다.
pub(crate) fn set_pinned<R: Runtime>(app: &AppHandle<R>, pinned: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_always_on_top(pinned);
    }
}

/// 지금 상태(대기 요청 유무)에 맞춰 고정을 재조정한다.
pub(crate) fn sync_always_on_top<R: Runtime>(app: &AppHandle<R>) {
    set_pinned(app, is_held());
}

/// 사용자가 **명시적으로** 창을 닫는다(트레이 토글·Cmd+W). 실제로 숨겼으면 true.
///
/// 개발 53 전엔 승인 대기 중이면 숨기지 않고 앞으로만 가져왔다(치우면 5분 타임아웃을 놓치니까).
/// 지금은 숨기되 **그 요청을 「닫아 둠」으로 적는다** — 감시 스레드가 잠든 프론트를 깨우는
/// 경로(ipc::watchdog)와 프론트의 raise_main_window 가 이 표식을 보고 다시 띄우지 않는다.
/// 만료 직전 한 번 되살리는 건 감시 스레드 몫(REMIND_SECS).
pub(crate) fn hide_by_user<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(win) = app.get_webview_window("main") else {
        return false;
    };
    // 숨기기 전에 적는다 — 숨긴 뒤 적으면 그 사이 1초 루프가 「깨울 요청」으로 볼 수 있다.
    if let Some(req) = crate::ipc::live_request() {
        if let Ok(mut g) = app.state::<PopoverState>().dismissed.lock() {
            *g = Some(req.id);
        }
    }
    win.hide().is_ok()
}

// ---------- 좌표 ----------
// 계산은 전부 **물리 픽셀**로 한다. 모니터 좌표·창 크기가 이미 물리라, 논리로 바꾸면
// "어느 모니터의 배율로 나눌 것인가" 문제가 생긴다(트레이가 있는 화면과 창이 있던 화면의
// 배율이 다를 수 있음 — 혼합 배율 멀티모니터에서 창이 엉뚱한 곳에 뜨는 원인).

/// 사각형(물리 px): x, y, 너비, 높이.
type Rect4 = (f64, f64, f64, f64);

fn tray_rect_physical(r: tauri::Rect, fallback_scale: f64) -> Rect4 {
    // macOS 의 tray-icon 은 물리 좌표를 준다. Logical 로 오는 경우만 배율을 곱하는데,
    // 이때 쓸 배율은 아직 어느 모니터인지 모르므로 주 모니터 배율로 근사한다.
    let (x, y) = match r.position {
        tauri::Position::Physical(v) => (v.x as f64, v.y as f64),
        tauri::Position::Logical(v) => (v.x * fallback_scale, v.y * fallback_scale),
    };
    let (w, h) = match r.size {
        tauri::Size::Physical(v) => (v.width as f64, v.height as f64),
        tauri::Size::Logical(v) => (v.width * fallback_scale, v.height * fallback_scale),
    };
    (x, y, w, h)
}

/// 팝오버 좌상단 위치를 계산한다 (순수 계산 — 테스트 가능). 전부 물리 px.
///
/// 가로는 트레이 아이콘 가운데, **세로는 작업 영역(work area) 상단 = 메뉴바 바로 아래**에
/// 맞춘다. 트레이 rect 의 아래쪽에 붙이면 안 된다 — 상태 아이템 프레임이 메뉴바보다 아래까지
/// 내려오는 경우가 있어 팝오버만 혼자 처져 보인다(네이티브 메뉴바 앱과 눈에 띄게 어긋남).
/// 작업 영역은 메뉴바·독을 제외한 영역이라 노치 유무·메뉴바 높이에 상관없이 정확하다.
///
/// 트레이도 작업 영역도 모르면 None — 이때는 창을 옮기지 않는다(엉뚱한 좌표로 화면 밖에
/// 두느니 있던 자리에 그대로 띄우는 게 낫다).
fn popover_origin(
    tray: Option<Rect4>,
    win_w: f64,
    win_h: f64,
    screen: Option<Rect4>,
    work: Option<Rect4>,
    margin: f64,
) -> Option<(f64, f64)> {
    let gap = TRAY_GAP * margin;
    let pad = SCREEN_MARGIN * margin;

    let (x, y) = match (tray, work) {
        // 가로 = 아이콘 중앙, 세로 = 메뉴바 바로 아래.
        (Some((tx, _, tw, _)), Some((_, wy, _, _))) => (tx + tw / 2.0 - win_w / 2.0, wy + gap),
        // 작업 영역을 모르면 트레이 아래로(차선책).
        (Some((tx, ty, tw, th)), None) => (tx + tw / 2.0 - win_w / 2.0, ty + th + gap),
        // 트레이를 모르면 작업 영역 오른쪽 위(트레이가 사는 자리).
        (None, Some((wx, wy, ww, _))) => (wx + ww - win_w - pad, wy + gap),
        (None, None) => return None,
    };

    // 가로는 **모니터 전체**로 자른다. 작업 영역으로 자르면 독이 좌우에 있을 때 그 폭만큼
    // 팝오버가 아이콘에서 밀려나 시각적 연결이 끊긴다(팝오버가 독 위에 겹치는 건 정상).
    let x = match screen.or(work) {
        Some((sx, _, sw, _)) => {
            let min_x = sx + pad;
            x.clamp(min_x, (sx + sw - win_w - pad).max(min_x))
        }
        None => x,
    };

    // 세로는 작업 영역으로 — 메뉴바를 침범하지도, 독 아래로 내려가지도 않게.
    let y = match work {
        Some((_, wy, _, wh)) => {
            // 하한은 메뉴바 바로 아래(gap 없이).
            let min_y = wy;
            y.clamp(min_y, (wy + wh - win_h - pad).max(min_y))
        }
        None => y,
    };

    Some((x, y))
}

/// 작업 영역에 들어가도록 창 높이를 정한다(물리 px). 순수 계산 — 테스트 가능.
///
/// 좌표만으로는 작업 영역보다 큰 고정 창을 다 보여줄 수 없다. 아래가 잘리면 팝오버 하단의
/// **승인/거부 버튼에 손이 닿지 않아 결제가 그대로 타임아웃**된다. 셸이 이미 내부 스크롤이라
/// 높이를 줄여도 내용은 다 볼 수 있으므로, 화면에 맞춰 줄이는 쪽이 옳다.
///
/// 최소 높이 하한을 두지 않는다. 하한을 두면 작업 영역이 그보다 작을 때 다시 아래가 잘려
/// 이 함수의 목적 자체가 무너진다 — "읽기 좋은 크기"보다 "승인 버튼에 닿는다"가 우선이다.
/// (실제로 작업 영역이 그 정도로 작은 Mac 은 없지만, 보장은 예외 없이 성립해야 한다.)
fn fit_height(desired_h: f64, work_h: Option<f64>, margin: f64) -> f64 {
    let Some(wh) = work_h else {
        return desired_h;
    };
    let room = wh - SCREEN_MARGIN * margin * 2.0;
    // 값이 비정상(0 이하)이면 손대지 않는다.
    if room <= 0.0 {
        return desired_h;
    }
    desired_h.min(room)
}

/// 이번에 적용할 창 크기 (순수 계산 — 물리 px).
///
/// `grow` = 기본 크기로 되돌려도 되는가. 숨어 있다 뜨는 창(show)은 사용자가 보기 전에
/// 끝나므로 되돌려도 된다. **이미 보이는 창**은 다르다 — 승인 모달에 비번을 입력하는 중일
/// 수 있어, 굳이 키우면 버튼이 손 아래에서 움직인다. 그래서 그때는 화면에 안 들어가는
/// 경우(줄여야 하는 경우)에만 손대고 그 외엔 지금 크기를 그대로 둔다.
/// "키우지 않는다"는 **축마다** 성립해야 한다 — 높이만 제한하면, 2x 화면에서 1x 화면으로
/// 불러온 창이 옛 배율 너비(840px)를 그대로 달고 있게 된다.
fn target_size(cur: Option<(f64, f64)>, want: (f64, f64), grow: bool) -> (f64, f64) {
    match cur {
        Some((cw, ch)) if !grow => (cw.min(want.0), ch.min(want.1)),
        _ => want,
    }
}

/// 점을 품는 모니터를 찾는다 (물리 좌표). 트레이가 있는 화면을 기준으로 잘라야
/// 창이 트레이와 다른 화면으로 튀지 않는다.
fn monitor_containing<R: Runtime>(app: &AppHandle<R>, x: f64, y: f64) -> Option<tauri::Monitor> {
    app.available_monitors().ok()?.into_iter().find(|m| {
        let p = m.position();
        let s = m.size();
        x >= p.x as f64
            && x < p.x as f64 + s.width as f64
            && y >= p.y as f64
            && y < p.y as f64 + s.height as f64
    })
}

/// 모니터 전체 경계 — 가로 클램프와 트레이가 어느 화면인지 찾는 데 쓴다.
fn monitor_rect(m: &tauri::Monitor) -> Rect4 {
    let p = m.position();
    let s = m.size();
    (p.x as f64, p.y as f64, s.width as f64, s.height as f64)
}

/// 작업 영역(메뉴바·독 제외). macOS 의 visibleFrame — 팝오버를 붙일 기준이자 세로 클램프.
fn work_rect(m: &tauri::Monitor) -> Rect4 {
    let w = m.work_area();
    (
        w.position.x as f64,
        w.position.y as f64,
        w.size.width as f64,
        w.size.height as f64,
    )
}

/// 팝오버를 트레이 아이콘 바로 아래로 옮긴다(필요하면 크기도 맞춘다).
/// `grow` 의 의미는 target_size 주석 참고 — 보이는 창을 불러올 때는 false.
fn position_at_tray<R: Runtime>(app: &AppHandle<R>, win: &WebviewWindow<R>, grow: bool) {
    let primary = app.primary_monitor().ok().flatten();
    let primary_scale = primary.as_ref().map(|m| m.scale_factor()).unwrap_or(1.0);

    let tray = app
        .tray_by_id(TRAY_ID)
        .and_then(|t| t.rect().ok().flatten())
        .map(|r| tray_rect_physical(r, primary_scale));

    // 기준 화면 = 트레이가 놓인 화면(트레이는 메뉴바 안에 있으므로 '작업 영역'이 아니라
    // 모니터 전체 경계로 찾아야 한다). 트레이를 모르면 주 모니터(메뉴바가 있는 곳).
    let screen_monitor = tray
        .and_then(|(x, y, w, h)| monitor_containing(app, x + w / 2.0, y + h / 2.0))
        .or(primary);
    let scale = screen_monitor
        .as_ref()
        .map(|m| m.scale_factor())
        .unwrap_or(primary_scale);
    let work = screen_monitor.as_ref().map(work_rect);
    let screen = screen_monitor.as_ref().map(monitor_rect);

    // 크기·여백 상수는 논리 px 기준이라 대상 화면 배율만큼 키워 쓴다.
    // 낮은 화면이면 먼저 줄여서 아래가 잘리지 않게 한다(셸이 내부 스크롤이라 내용은 다 보인다).
    // 넓은 화면으로 옮기면 다시 기본 크기로 돌아온다 — 그래서 지금 크기가 아니라 상수 기준.
    let want = (
        WINDOW_W * scale,
        fit_height(WINDOW_H * scale, work.map(|(_, _, _, wh)| wh), scale),
    );
    let cur = win
        .outer_size()
        .ok()
        .map(|s| (s.width as f64, s.height as f64));
    let (win_w, win_h) = target_size(cur, want, grow);

    // 너비까지 함께 비교해야 한다 — 높이만 보면, 배율이 다른 화면으로 옮겨 목표 높이가
    // 우연히 같아진 경우(예: 2x 에서 줄어든 높이 == 1x 기본 높이) 호출을 건너뛰어
    // 너비가 옛 배율 그대로 남는다. 그러면 위치 계산(win_w 전제)과 실제 창이 어긋난다.
    let needs_resize = match cur {
        Some((cw, ch)) => (cw - win_w).abs() > 0.5 || (ch - win_h).abs() > 0.5,
        None => true,
    };
    if needs_resize {
        let _ = win.set_size(tauri::PhysicalSize::new(win_w, win_h));
    }

    if let Some((x, y)) = popover_origin(tray, win_w, win_h, screen, work, scale) {
        let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
    }
}

/// 팝오버를 트레이 아래에 띄우고 포커스를 준다.
pub(crate) fn show<R: Runtime>(app: &AppHandle<R>) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    // 크기를 기본값으로 되돌려도 되는 건 **지금 사용자 눈에 없는 창**일 때뿐이다. show 는 이미
    // 보이는 창에도 불린다(트레이 메뉴 '열기', 결제 요청 도착 시 raise_main_window) — 그때
    // 창이 커지면 승인 버튼이 손 아래에서 움직인다.
    // · 최소화된 창은 macOS 에서 visible 로 잡히지만 화면엔 없다 → 되돌려도 안전(안 그러면
    //   줄어든 크기가 숨겼다 열기 전까지 고착된다).
    // · 조회 실패는 "보이는 중"으로 본다 — 틀렸을 때 손해가 작은 쪽(안 키움).
    let hidden = !win.is_visible().unwrap_or(true) || win.is_minimized().unwrap_or(false);
    position_at_tray(app, &win, hidden);
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
    // 어떤 경로로든 창이 다시 떴다 = 「닫아 둠」은 끝났다(개발 53). 트레이·독·메뉴 '열기'·
    // 만료 직전 되살리기 전부. 여기서 지우지 않으면 되살린 창을 사용자가 그대로 두고 승인해도
    // 표식이 남아, 같은 id 가 아닌 한 해가 없긴 하지만 상태가 사실과 어긋난 채 남는다.
    if let Ok(mut g) = app.state::<PopoverState>().dismissed.lock() {
        *g = None;
    }
}

/// 트레이 아이콘 좌클릭 — 열려 있으면 닫고, 닫혀 있으면 연다.
fn toggle<R: Runtime>(app: &AppHandle<R>) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };

    // 방금 blur 로 스스로 숨었다면, 그 blur 를 일으킨 게 바로 이 클릭이다 → 다시 열지 않는다.
    if let Ok(mut g) = app.state::<PopoverState>().last_auto_hide.lock() {
        if g.map(|t| t.elapsed() < REOPEN_GUARD).unwrap_or(false) {
            *g = None;
            return;
        }
        *g = None;
    }

    if win.is_visible().unwrap_or(false) {
        // 승인 대기 중이면 포커스를 따지지 않고 닫는다(개발 53). 이때 창은 항상-위라 「뒤에
        // 있어서 앞으로」가 성립하지 않고, 트레이 클릭이 먼저 blur 를 일으켜 is_focused 가
        // 이미 false 일 수 있다(on_blur 는 hold 라 숨기지도, 재열림 가드를 걸지도 않는다) —
        // 그래서 예전엔 승인 창을 트레이로 눌러도 «앞으로 가져오기»만 되고 안 닫혔다.
        if is_held() {
            hide_by_user(app);
            return;
        }
        // 보이지만 뒤에 있으면 닫지 말고 앞으로.
        if win.is_focused().unwrap_or(false) {
            hide_by_user(app);
        } else {
            // 숨김 없이 앞으로만 올리는 경로에서도 자리를 다시 잡는다
            // (hide_unless_held 의 hold 분기와 같은 이유 — 보이는 동안은 재배치가 없다).
            position_at_tray(app, &win, false);
            let _ = win.set_focus();
        }
    } else {
        show(app);
    }
}

/// 창이 포커스를 잃었을 때 — 팝오버를 숨긴다. 단 승인 대기 중이면 그대로 둔다.
/// (lib.rs 의 on_window_event 에서 호출)
pub(crate) fn on_blur<R: Runtime>(win: &Window<R>) {
    let app = win.app_handle();
    if is_held() {
        return;
    }
    // 실제로 숨었을 때만 재열림 가드를 건다 — hide 가 실패했는데 가드가 걸리면
    // 뒤이은 트레이 클릭이 통째로 무시돼 토글이 한 번 안 먹는다.
    if win.hide().is_ok() {
        if let Ok(mut g) = app.state::<PopoverState>().last_auto_hide.lock() {
            *g = Some(Instant::now());
        }
    }
}

/// 긴급 잠금 상태를 메뉴바 아이콘에 반영한다 (평상시 = 열쇠구멍 동전 / 잠김 = 자물쇠).
pub(crate) fn refresh_icon<R: Runtime>(app: &AppHandle<R>) {
    let locked = crate::lock::read_lock();
    let bytes = if locked { ICON_LOCKED } else { ICON_NORMAL };
    let Ok(icon) = Image::from_bytes(bytes) else {
        return;
    };
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        // 아이콘과 템플릿 지정을 한 번에 — 따로 하면 macOS 에서 한 프레임 깜빡인다.
        let _ = tray.set_icon_with_as_template(Some(icon), true);
        let _ = tray.set_tooltip(Some(if locked {
            ts!("Kura — 긴급 잠금", "Kura — emergency lock")
        } else {
            "Kura"
        }));
    }
}

/// 우클릭 메뉴 — 언어가 바뀌면 통째로 다시 만든다(항목 핸들을 들고 있지 않아도 되게).
/// 메뉴 이벤트 핸들러는 트레이 쪽에 붙어 있고 id 는 그대로라, 바꿔 껴도 동작이 유지된다.
fn menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let open_i = MenuItem::with_id(
        app,
        "open",
        ts!("Kura 열기", "Open Kura"),
        true,
        None::<&str>,
    )?;
    let quit_i = MenuItem::with_id(app, "quit", ts!("종료", "Quit"), true, None::<&str>)?;
    Menu::with_items(app, &[&open_i, &quit_i])
}

/// 언어를 바꾼 뒤 메뉴바의 글자를 새 언어로 갈아 끼운다 (개발 42).
/// 실패해도 조용히 넘어간다 — 메뉴 글자 하나 때문에 언어 변경 자체를 실패로 돌릴 이유가 없다.
pub(crate) fn retitle<R: Runtime>(app: &AppHandle<R>) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(m) = menu(app) {
            let _ = tray.set_menu(Some(m));
        }
    }
    refresh_icon(app); // 툴팁도 언어를 탄다
}

/// 메뉴바 아이콘을 만든다. 좌클릭 = 팝오버 토글, 우클릭 = 메뉴.
pub(crate) fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = menu(app)?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(ICON_NORMAL)?)
        .icon_as_template(true)
        .tooltip("Kura")
        .menu(&menu)
        // 좌클릭은 팝오버 토글로 쓰므로 메뉴는 우클릭에만.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle(tray.app_handle());
            }
        })
        .build(app)?;

    // 앱을 껐다 켜도 잠금 상태가 아이콘에 남아 있어야 한다.
    refresh_icon(app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f64 = WINDOW_W;
    const H: f64 = WINDOW_H;
    /// 메뉴바 24 를 뺀 1x 작업 영역 (독 없음).
    const WORK: Option<Rect4> = Some((0.0, 24.0, 1440.0, 876.0));
    /// 같은 화면의 모니터 전체 경계.
    const SCREEN: Option<Rect4> = Some((0.0, 0.0, 1440.0, 900.0));

    fn origin(tray: Option<Rect4>) -> (f64, f64) {
        popover_origin(tray, W, H, SCREEN, WORK, 1.0).expect("작업 영역을 아는 한 위치가 나온다")
    }

    // ---------- 닫아 둔 승인 창 (개발 53) ----------

    // 닫아 둔 요청이 아니면 평소 규칙 — 다른 id 도, 아무것도 안 닫았을 때도.
    #[test]
    fn dismissal_ignores_other_requests() {
        assert_eq!(dismissal(None, None, "b", 200), Dismissal::No);
        assert_eq!(dismissal(Some("a"), None, "b", 200), Dismissal::No);
        // 되살린 기록만 있고 닫힌 기록이 없으면(창이 다시 떠서 지워짐) 평소 규칙.
        assert_eq!(dismissal(None, Some("b"), "b", 30), Dismissal::No);
    }

    // 닫아 둔 동안은 깨우지 않는다 — 남은 시간이 넉넉한 동안.
    #[test]
    fn dismissed_holds_until_deadline_nears() {
        assert_eq!(dismissal(Some("a"), None, "a", 299), Dismissal::Hold);
        assert_eq!(dismissal(Some("a"), None, "a", REMIND_SECS + 1), Dismissal::Hold);
    }

    // 만료 REMIND_SECS 전부터는 한 번 되살린다. 경계는 리터럴로 — 상수를 상수로 검증하면
    // 값을 0 으로 바꿔도 통과한다(위 sits_flush_under_menubar 와 같은 이유).
    #[test]
    fn dismissed_reminds_once_near_deadline() {
        assert_eq!(dismissal(Some("a"), None, "a", 60), Dismissal::Remind);
        assert_eq!(dismissal(Some("a"), None, "a", 1), Dismissal::Remind);
        // 이미 되살렸으면 다시 닫아도 그대로 둔다.
        assert_eq!(dismissal(Some("a"), Some("a"), "a", 30), Dismissal::Hold);
    }

    // 이미 만료된(유예 구간의) 요청은 되살려 봐야 승인이 안 된다 → 그대로 둔다.
    #[test]
    fn dismissed_expired_is_not_revived() {
        assert_eq!(dismissal(Some("a"), None, "a", 0), Dismissal::Hold);
    }

    // 가로는 트레이 아이콘 가운데, 세로는 메뉴바 바로 아래에 "딱 붙는다".
    // 기댓값을 TRAY_GAP 이 아니라 리터럴로 둔다 — 상수를 상수로 검증하면 값을 4·6 으로
    // 되돌려도 통과해서 "붙인다"는 요구사항을 전혀 못 지킨다(코덱스 지적).
    #[test]
    fn sits_flush_under_menubar() {
        let (x, y) = origin(Some((700.0, 0.0, 24.0, 24.0)));
        assert_eq!(x, 700.0 + 12.0 - W / 2.0);
        assert_eq!(y, 24.0, "메뉴바 경계에 딱 붙어야 한다(간격 0)");
    }

    // 🔴 회귀: 상태 아이템 프레임이 메뉴바보다 아래까지 내려와도(높이 40) 팝오버는
    // 메뉴바 바로 아래에 붙는다. 트레이 rect 아래에 붙이던 옛 방식은 여기서 처졌다.
    #[test]
    fn anchors_to_menubar_not_tray_bottom() {
        let (_, y) = origin(Some((700.0, 0.0, 24.0, 40.0)));
        assert_eq!(y, 24.0);
    }

    // 트레이가 화면 오른쪽 끝이어도 화면 밖으로 나가지 않는다.
    #[test]
    fn clamps_to_right_edge() {
        let (x, _) = origin(Some((1420.0, 0.0, 20.0, 24.0)));
        assert_eq!(x, 1440.0 - W - SCREEN_MARGIN);
    }

    // 왼쪽 끝도 마찬가지.
    #[test]
    fn clamps_to_left_edge() {
        let (x, _) = origin(Some((0.0, 0.0, 20.0, 24.0)));
        assert_eq!(x, SCREEN_MARGIN);
    }

    // 독이 우측에 있어도 가로는 모니터 전체 기준으로 자른다 — 작업 영역으로 자르면
    // 독 폭만큼 팝오버가 아이콘에서 밀려나 시각적 연결이 끊긴다.
    #[test]
    fn side_dock_does_not_pull_popover_off_icon() {
        // 우측 100 이 독. 작업영역 기준 최대 x = 1340-420-8 = 912,
        // 모니터 기준 최대 x = 1440-420-8 = 1012. 그 사이(950)에 놓이는 트레이를 고른다 —
        // 옛 정책(작업영역으로 가로 클램프)이었다면 912 로 끌려갔을 자리.
        let work_with_side_dock = Some((0.0, 24.0, 1340.0, 876.0));
        let (x, _) = popover_origin(
            Some((1148.0, 0.0, 24.0, 24.0)),
            W,
            H,
            SCREEN,
            work_with_side_dock,
            1.0,
        )
        .unwrap();
        assert_eq!(x, 950.0, "아이콘 중앙(950)을 유지해야 한다");
        assert!(
            x > 1340.0 - W - SCREEN_MARGIN,
            "작업영역 클램프였다면 912 로 밀렸다"
        );
    }

    // 트레이 위치를 못 얻으면 메뉴바가 있는 오른쪽 위로.
    #[test]
    fn falls_back_to_top_right() {
        let (x, y) = origin(None);
        assert_eq!(x, 1440.0 - W - SCREEN_MARGIN);
        assert_eq!(y, 24.0);
    }

    // 창 아래가 독 위에 머문다(세로는 작업 영역 기준).
    #[test]
    fn clamps_above_dock() {
        let (_, y) = origin(Some((700.0, 0.0, 24.0, 24.0)));
        assert!(y + H <= 24.0 + 876.0, "창 아래가 작업 영역 안이어야 한다");
    }

    // 작업 영역을 모르면 차선책으로 트레이 아래에 붙인다.
    #[test]
    fn no_work_area_falls_back_to_tray_bottom() {
        let (x, y) = popover_origin(Some((700.0, 0.0, 24.0, 24.0)), W, H, None, None, 1.0).unwrap();
        assert_eq!(x, 700.0 + 12.0 - W / 2.0);
        assert_eq!(y, 24.0 + TRAY_GAP);
    }

    // 트레이도 화면도 모르면 아예 옮기지 않는다 — 옛 코드는 여기서 f64::MAX 로 창을 던졌다.
    #[test]
    fn unknown_tray_and_screen_does_not_move() {
        assert!(popover_origin(None, W, H, None, None, 1.0).is_none());
    }

    // 레티나: 창 크기·여백이 배율만큼 커진 물리값으로 들어와야 가운데 정렬과 클램프가 맞는다.
    #[test]
    fn scales_with_display() {
        let screen = Some((0.0, 0.0, 2880.0, 1800.0));
        let work = Some((0.0, 48.0, 2880.0, 1752.0));
        let (x, y) = popover_origin(
            Some((1400.0, 0.0, 48.0, 48.0)),
            W * 2.0,
            H * 2.0,
            screen,
            work,
            2.0,
        )
        .unwrap();
        assert_eq!(y, 48.0, "레티나에서도 메뉴바에 딱 붙는다");
        assert_eq!(x, 1400.0 + 24.0 - W); // 트레이 중심 - 창 절반(= W*2/2)
    }

    // ---------- 창 높이 맞추기 ----------

    // 넉넉한 화면에서는 기본 높이 그대로.
    #[test]
    fn keeps_default_height_when_it_fits() {
        assert_eq!(fit_height(H, Some(876.0), 1.0), H);
    }

    // 🔴 작업 영역이 창보다 낮으면 **줄인다** — 안 줄이면 팝오버 하단의 승인 버튼이
    // 화면 밖에 남아 결제가 그대로 타임아웃된다(코덱스 High).
    #[test]
    fn shrinks_to_fit_short_work_area() {
        let h = fit_height(H, Some(500.0), 1.0);
        assert_eq!(h, 500.0 - SCREEN_MARGIN * 2.0);
        assert!(h < H && h + SCREEN_MARGIN * 2.0 <= 500.0);
    }

    // 🔴 핵심 보장: **여백을 뺀 가용 높이가 남는 작업 영역이면** 창 전체가 그 안에 들어간다.
    // (fit_height + popover_origin 을 함께 검증 — 최소 높이 하한을 두면 여기가 깨진다.
    //  하한이 있던 버전은 작업 영역 300 에서 창 320 을 반환해 아래 20px 가 잘렸다.)
    // 가용 높이가 0 이하인 비정상 입력은 이 보장 밖 — absurd_work_area_leaves_size_alone 참고.
    #[test]
    fn fits_inside_any_usable_work_area() {
        for wh in [
            SCREEN_MARGIN * 2.0 + 1.0,
            300.0_f64,
            500.0,
            660.0,
            876.0,
            1200.0,
        ] {
            let work = Some((0.0, 24.0, 1440.0, wh));
            let h = fit_height(H, Some(wh), 1.0);
            let (_, y) =
                popover_origin(Some((700.0, 0.0, 24.0, 24.0)), W, h, SCREEN, work, 1.0).unwrap();
            assert!(y >= 24.0, "메뉴바를 침범하면 안 된다 (wh={wh})");
            assert!(
                y + h <= 24.0 + wh,
                "창 아래가 작업 영역을 넘으면 안 된다 (wh={wh})"
            );
        }
    }

    // 작업 영역을 모르면 기본 높이를 그대로 쓴다.
    #[test]
    fn unknown_work_area_keeps_desired_height() {
        assert_eq!(fit_height(H, None, 1.0), H);
    }

    // 레티나에서도 물리 기준으로 맞춘다(여백에 배율 적용).
    #[test]
    fn fits_height_on_retina() {
        assert_eq!(
            fit_height(H * 2.0, Some(1000.0), 2.0),
            1000.0 - SCREEN_MARGIN * 2.0 * 2.0
        );
        // 레티나에서도 "항상 들어간다" 보장이 성립한다.
        let work = Some((0.0, 48.0, 2880.0, 1000.0));
        let h = fit_height(H * 2.0, Some(1000.0), 2.0);
        let (_, y) =
            popover_origin(Some((1400.0, 0.0, 48.0, 48.0)), W * 2.0, h, None, work, 2.0).unwrap();
        assert!(y >= 48.0 && y + h <= 48.0 + 1000.0);
    }

    // 작업 영역 값이 비정상(가용 높이 0 이하)이면 손대지 않는다 — 0 이하 크기로 창을
    // 만들지 않으려는 방어. 이 경우엔 "항상 들어간다" 보장이 성립하지 않는다(의도된 예외:
    // 실제 macOS 작업 영역이 여백 이하로 내려오는 경로는 없다).
    #[test]
    fn absurd_work_area_leaves_size_alone() {
        assert_eq!(fit_height(H, Some(SCREEN_MARGIN * 2.0), 1.0), H);
        assert_eq!(fit_height(H, Some(0.0), 1.0), H);
    }

    // 숨어 있다 뜨는 창은 기본 크기로 복원한다(사용자가 보기 전에 끝난다).
    #[test]
    fn hidden_window_restores_design_size() {
        assert_eq!(target_size(Some((300.0, 500.0)), (W, H), true), (W, H));
        assert_eq!(target_size(None, (W, H), true), (W, H));
    }

    // 🔴 보이는 창은 키우지 않는다 — 승인 모달에 비번을 넣는 중에 창이 커지면 버튼이
    // 손 아래에서 움직인다. 크기를 모를 때만 목표값으로 간다.
    #[test]
    fn visible_window_is_never_grown() {
        assert_eq!(target_size(Some((W, 500.0)), (W, H), false), (W, 500.0));
        assert_eq!(target_size(None, (W, H), false), (W, H));
        // 너비도 마찬가지 — 축마다 성립해야 한다(좁은 창을 넓히지 않는다).
        assert_eq!(target_size(Some((300.0, H)), (W, H), false), (300.0, H));
    }

    // 🔴 반대 방향: 2x 화면에서 1x 화면으로 불러온 창은 옛 배율 너비를 달고 있다.
    // "키우지 않는다"를 높이에만 적용하면 그 과대 너비가 그대로 남는다.
    #[test]
    fn visible_window_shrinks_stale_scaled_width() {
        assert_eq!(target_size(Some((W * 2.0, H * 2.0)), (W, H), false), (W, H));
    }

    // 다만 화면에 안 들어가면(줄여야 하면) 보이는 창도 줄인다 — 승인 버튼 접근성이 우선.
    #[test]
    fn visible_window_still_shrinks_when_too_tall() {
        let want = (W, 400.0); // 낮은 화면이라 목표 높이가 400
        assert_eq!(target_size(Some((W, H)), want, false), (W, 400.0));
    }

    // 🔴 드리프트 가드: WINDOW_W/H 는 "tauri.conf.json 과 같아야 한다"가 주석으로만 있었다.
    // 어긋나면 좁은 화면에서 줄인 창이 **원래 크기가 아닌 값으로 복원**된다(설계상 지금 창
    // 크기가 아니라 이 상수로 되돌리기 때문). 두 곳 중 한쪽만 고치는 사고를 컴파일·테스트로 막는다.
    #[test]
    fn window_size_matches_tauri_conf() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json 파싱");
        let win = conf["app"]["windows"]
            .as_array()
            .and_then(|ws| ws.iter().find(|w| w["label"] == "main"))
            .expect("main 창 설정이 있어야 한다");
        assert_eq!(win["width"].as_f64(), Some(WINDOW_W));
        assert_eq!(win["height"].as_f64(), Some(WINDOW_H));
    }
}
