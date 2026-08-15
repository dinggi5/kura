// Kura 코어 — 로컬 전용 AI 에이전트 지갑. (코드네임: 지갑지갑)
//
// 모듈 지도 (개발 17 구조 분리 — 동작 무변경):
//   autostart 로그인 시 자동 시작 + 희망값 보존·복구(개발 31)
//   chain     체인 상수(Base Sepolia RPC·USDC·chainId) + 온체인 타입(sol!)
//   store     ~/.jigap 경로, 원자적 파일 쓰기, 시간 헬퍼
//   wallet    니모닉 생성·주소 파생·비번 암호화(Argon2id + AES-256-GCM)
//   settings  사용자 설정(한도·자율·RPC·잠금 동작)
//   limits    단일/일일 한도 검사 + 오늘 사용액 장부
//   lock      긴급 잠금 (비상 스위치)
//   trusted   화이트리스트(신뢰 주소) — 자율 결제 가드
//   history   거래 내역 + x402 정산 추적
//   transfer  온체인 송금(ETH/USDC) + 잔액 조회
//   x402      EIP-3009 오프체인 서명 (x402 "exact")
//   ipc       MCP ↔ GUI 결제 승인 파일 IPC + AI 연결 배지
//   session   자율 결제 세션(메모리 키) + 자동 승인
//   notify    OS 알림 (자율 결제 사후 통지)
//   tray      메뉴바 상주 — 트레이 아이콘 + 팝오버 위치·자동 숨김
//   update    인앱 자동 업데이트(개발 31) — 검사·설치, 승인 대기 중 재시작 차단
//
// 이 파일에는 앱 셸만 남긴다: 창 제어 커맨드 + run().

mod autostart;
mod chain;
mod history;
mod ipc;
mod limits;
mod lock;
mod notify;
mod session;
mod settings;
mod store;
mod transfer;
mod tray;
mod trusted;
mod update;
mod wallet;
mod x402;

use session::SessionKey;

/// 결제 승인 팝업이 창에 가려져 5분 타임아웃을 놓친 실사례 → 사람 승인이 필요한 동안 창을 전면 고정.
/// macOS 14+는 백그라운드 앱의 포커스 뺏기(activateIgnoringOtherApps)를 무시하므로 set_focus만으론
/// 안 뜬다 — 항상 위(always-on-top)로 고정해 두고, 승인이 끝나면 release_main_window로 푼다.
///
/// 개발 26(팝오버화) 이후: 팝오버가 blur 로 자동으로 숨는데, 그러면 비번을 넣다가 다른 창을
/// 클릭한 순간 승인 창이 사라진다. 그 게이팅은 tray::is_held() 가 **대기 요청 유무에서
/// 직접 파생**하므로(ipc::has_pending) 여기서 따로 표식을 켜고 끌 필요가 없다.
/// 이 커맨드가 하는 일은 창을 앞으로 올리는 것뿐.
#[tauri::command]
fn raise_main_window(app: tauri::AppHandle) {
    // 이미 처리된 요청에 대한 늦은 호출이면 아무것도 하지 않는다(항상-위가 켜진 채 남지 않게).
    if !ipc::has_pending() {
        return;
    }
    // set_pinned 를 거쳐야 폴링 쪽 캐시와 어긋나지 않는다.
    tray::set_pinned(&app, true);
    tray::show(&app);
}

/// 승인 처리(승인/거부/타임아웃)가 끝나면 항상-위 고정을 즉시 재조정.
/// 다음 요청이 이미 대기 중이면 켜진 채로 남는다(A→B 전환에서 A 의 늦은 해제가 B 의 전면
/// 고정을 꺼버리지 않게). 어차피 1초 폴링도 같은 값으로 수렴시킨다 — 이건 그 대기를 없애는 용.
#[tauri::command]
fn release_main_window(app: tauri::AppHandle) {
    tray::sync_always_on_top(&app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(SessionKey::default())
        .manage(tray::PopoverState::default())
        .manage(update::PendingUpdate::default())
        .setup(|app| {
            tray::build(app.handle())?;
            // 업데이트가 자동 시작 설정을 지우고 갔는지 확인해 되살린다(개발 31).
            // 트레이보다 뒤, 창을 띄우기 전 — 실패해도 앱은 그대로 뜬다.
            autostart::reconcile(app.handle());
            // 평소엔 트레이에만 조용히 상주한다(로그인 자동 시작 때 창이 튀어나오지 않게).
            // 단 첫 실행은 예외 — 메뉴바 아이콘만 뜨고 아무 일도 없으면 뭘 해야 할지 알 수 없다.
            if wallet::needs_setup() {
                tray::show(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // 상시 구동(Session 18): 창 닫기는 종료가 아니라 숨기기 — 백그라운드에서 결제 요청을
            // 계속 받는다. 완전 종료는 Cmd+Q(ExitRequested는 안 막음) 또는 트레이 메뉴의 "종료".
            tauri::WindowEvent::CloseRequested { api, .. } => {
                use tauri::Manager;
                api.prevent_close();
                // 승인 대기 중이면 숨기지 않는다 — Cmd+W 로 승인 창을 치워버리면
                // 다시 열지 않는 한 요청이 그대로 타임아웃된다.
                tray::hide_unless_held(window.app_handle());
            }
            tauri::WindowEvent::Focused(false) => {
                use tauri::Manager;
                // 자리비움 자동 잠금(Session 14): 설정이 켜져 있으면 세션 키를 즉시 소멸.
                if settings::read_settings().lock_on_blur {
                    if let Ok(mut g) = window.state::<SessionKey>().0.lock() {
                        *g = None;
                    }
                }
                // 팝오버 자동 숨김(개발 26). 승인 대기 중이면 tray 쪽에서 걸러 안 숨긴다.
                tray::on_blur(window);
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            wallet::get_wallet_status,
            wallet::create_wallet,
            wallet::migrate_wallet,
            wallet::import_wallet,
            wallet::reveal_mnemonic,
            wallet::mark_backed_up,
            transfer::get_balances,
            transfer::send_eth,
            transfer::send_usdc,
            settings::get_settings,
            settings::set_settings,
            limits::get_today_spend,
            lock::is_locked,
            lock::set_locked,
            history::get_history,
            ipc::get_pending_request,
            ipc::approve_payment,
            ipc::reject_payment,
            x402::sign_x402_payment,
            ipc::get_agent_status,
            session::unlock_session,
            session::lock_session,
            session::session_status,
            session::auto_approve_payment,
            history::apply_x402_settlements,
            trusted::is_trusted_addr,
            trusted::get_trusted_addrs,
            trusted::remove_trusted_addr,
            raise_main_window,
            release_main_window,
            autostart::get_autostart,
            autostart::set_autostart,
            settings::set_auto_check_update,
            update::check_update,
            update::install_update
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // 숨겨진 창을 Dock 아이콘 클릭으로 복원 (macOS applicationShouldHandleReopen).
            // 도크 아이콘은 유지하기로 했으므로(개발 26) 이 경로도 그대로 살려 둔다 —
            // 트레이와 함께 팝오버를 띄우는 두 번째 경로.
            if let tauri::RunEvent::Reopen { .. } = event {
                tray::show(app);
            }
        });
}
