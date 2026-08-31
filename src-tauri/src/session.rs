// 자율 결제 — 세션 잠금 해제 + 소액 자동 승인 (Session 14).
//
// 컨셉: 쿠라 = AI 에이전트 전용 지갑, 사람은 수호자. 매 결제마다 비번을 누르면 자율성이 없다.
// → 보호자가 비번을 "한 번" 입력해 세션을 잠금 해제하면, 그 키를 GUI 프로세스 "메모리에만"
//   (zeroize 보호) 들고 있다가, 자율 한도 이하의 AI 결제 요청은 비번 없이 자동 승인한다.
//
// 보안 불변식:
//   - 키(니모닉)는 디스크에 평문으로 절대 안 나간다. 잠금 해제 동안만 RAM에 존재.
//   - 앱 종료 → 메모리 소멸. 긴급 잠금 → 즉시 소멸(set_locked). 유휴 타임아웃 → 자동 소멸.
//   - 한도 초과·ETH 송금은 자율 불가 → 항상 사람 비번(기존 모달). 자율 한도 0 = 자율 꺼짐(=기존 동작).
//   - 자율 승인도 do_send_usdc/do_sign_x402 를 그대로 거치므로 긴급잠금·단일/일일 한도·내역이 동일 적용.

use crate::i18n::ts;
use alloy::primitives::U256;
use alloy::signers::local::PrivateKeySigner;
use serde::Serialize;
use std::sync::Mutex;
use zeroize::Zeroizing;

use crate::chain::active_chain;
use crate::ipc::{read_request, resolve_request, PaymentResult};
use crate::limits::parse_usdc_nonneg;
use crate::notify::{auto_pay_notice, show_notification};
use crate::settings::{read_settings, Settings};
use crate::store::now_secs;
use crate::transfer::do_send_usdc;
use crate::trusted::is_trusted;
use crate::wallet::{decrypt_wallet, read_encrypted, signer_from_phrase};
use crate::x402::do_sign_x402;

/// 자율 승인 불가(세션 잠김·한도 초과·자율 꺼짐) 신호. 프론트가 이걸 보면 사람 승인 모달을 띄운다.
const NEEDS_PASSWORD: &str = "NEEDS_PASSWORD";

/// 잠금 해제된 세션 키(니모닉)를 메모리에만 보관한다. 디스크에 절대 쓰지 않는다.
#[derive(Default)]
pub(crate) struct SessionKey(pub(crate) Mutex<Option<SessionInner>>);

pub(crate) struct SessionInner {
    /// 복호화된 12단어 니모닉. Drop 시 메모리를 0으로 지운다(zeroize).
    mnemonic: Zeroizing<String>,
    /// 마지막 자율 결제 시각(유닉스 초). 유휴 타임아웃 계산용.
    last_active: u64,
}

/// 설정의 자동 잠금(분)을 초로 환산한다. 0이면 유휴 잠금 안 함.
/// 깨진 값(수동 편집 등)이면 기본 30분 — 보안 기능의 실패 방향이 "조용히 꺼짐"이면 안 된다.
fn auto_lock_secs(settings: &Settings) -> u64 {
    settings
        .auto_lock_mins
        .trim()
        .parse::<u64>()
        .unwrap_or(30)
        .saturating_mul(60)
}

/// 자율 한도 이하인지 — 순수 판정(테스트용). USDC 표시 결제만, 한도>0, 금액≤한도.
fn within_auto_limit(token: &str, amount: U256, auto_limit: U256) -> bool {
    token == "USDC" && !auto_limit.is_zero() && amount <= auto_limit
}

/// 세션에 보관된 키로 서명자를 만든다. 유휴 타임아웃이 지났으면 자동 잠그고 None.
/// 성공 시 마지막 활동 시각을 갱신한다(MutexGuard 는 .await 전에 반드시 해제).
fn session_signer(state: &SessionKey, idle_secs: u64) -> Option<PrivateKeySigner> {
    let phrase = {
        let mut g = state.0.lock().ok()?;
        let expired = g
            .as_ref()
            .map(|inner| idle_secs > 0 && now_secs().saturating_sub(inner.last_active) > idle_secs)
            .unwrap_or(true);
        if expired {
            *g = None;
            return None;
        }
        let inner = g.as_mut()?;
        inner.last_active = now_secs();
        inner.mnemonic.clone()
    };
    signer_from_phrase(&phrase).ok()
}

/// 비번으로 세션을 잠금 해제한다 — 복호화한 니모닉을 메모리에만 보관(디스크 X).
/// 이후 자율 한도 이하 결제는 비번 없이 자동 승인된다. 비번이 틀리면 복호화에서 거부.
#[tauri::command]
pub(crate) fn unlock_session(
    password: String,
    session: tauri::State<'_, SessionKey>,
) -> Result<(), String> {
    let password = Zeroizing::new(password);
    let w = read_encrypted()?;
    let phrase = decrypt_wallet(&w, &password)?; // 비번 검증(틀리면 GCM 실패)
    crate::wallet::maybe_upgrade_kdf(&w, &phrase, &password); // 옛 KDF(v2)면 v3로 강화
    let mut g = session
        .0
        .lock()
        .map_err(|_| ts!("세션 상태 잠금 실패", "Couldn't read the session state").to_string())?;
    *g = Some(SessionInner {
        mnemonic: phrase,
        last_active: now_secs(),
    });
    Ok(())
}

/// 세션을 수동으로 잠근다 — 메모리의 키를 즉시 소멸시킨다.
#[tauri::command]
pub(crate) fn lock_session(session: tauri::State<'_, SessionKey>) {
    if let Ok(mut g) = session.0.lock() {
        *g = None;
    }
}

/// 프론트로 주는 자율 결제 세션 상태.
#[derive(Serialize)]
pub(crate) struct SessionStatus {
    /// 세션이 잠금 해제돼 있는지(=자율 결제 가능).
    unlocked: bool,
    /// 자동 잠금까지 남은 초. 유휴 타임아웃 0이거나 잠겨 있으면 0.
    remaining_secs: u64,
    /// 자율 결제 한도(USDC, 십진수). "0"이면 자율 결제 꺼짐.
    auto_limit: String,
}

/// 세션 상태를 알려준다(프론트 1초 폴링). 유휴 타임아웃이 지났으면 여기서도 자동 잠근다.
/// 단순 조회이므로 last_active 는 갱신하지 않는다(폴링이 세션을 영원히 살리지 않게).
#[tauri::command]
pub(crate) fn session_status(session: tauri::State<'_, SessionKey>) -> SessionStatus {
    let settings = read_settings();
    let idle = auto_lock_secs(&settings);
    let (unlocked, remaining) = match session.0.lock() {
        Ok(mut g) => match g.as_ref() {
            Some(inner) => {
                let elapsed = now_secs().saturating_sub(inner.last_active);
                if idle > 0 && elapsed > idle {
                    *g = None; // 유휴 → 자동 잠금
                    (false, 0)
                } else {
                    let rem = if idle > 0 {
                        idle.saturating_sub(elapsed)
                    } else {
                        0
                    };
                    (true, rem)
                }
            }
            None => (false, 0),
        },
        Err(_) => (false, 0),
    };
    SessionStatus {
        unlocked,
        remaining_secs: remaining,
        auto_limit: settings.auto_approve_usdc,
    }
}

/// 대기 중인 AI 결제 요청을 "자율 승인"으로 처리 시도한다(비번 없이).
/// 자율 불가(세션 잠김·한도 초과·자율 꺼짐·ETH 송금)면 NEEDS_PASSWORD 를 돌려준다
/// → 프론트가 사람 승인 모달을 띄운다. do_* 가 긴급잠금·한도·내역을 그대로 적용한다.
#[tauri::command]
pub(crate) async fn auto_approve_payment(
    id: String,
    session: tauri::State<'_, SessionKey>,
) -> Result<PaymentResult, String> {
    let req = read_request().ok_or(ts!(
        "대기 중인 결제 요청이 없습니다",
        "There's no payment request waiting"
    ))?;
    if req.id != id {
        return Err(ts!(
            "요청 ID가 일치하지 않습니다",
            "That request ID doesn't match"
        )
        .into());
    }
    crate::ipc::ensure_request_chain(&req)?; // 요청 시점 체인 ≠ 현재 체인이면 거부(자율 경로도 동일)

    // 🔴 **여기서부터 체인을 못박는다** (코덱스 개발51 2차 P1). 위 검사는 *그 순간*만 본다 —
    // 그 뒤로 잔액 조회·서명·전송에 `.await` 가 여러 번 있고, 그동안 사용자가 네트워크를 바꾸면
    // `do_send_usdc` 가 **그때 활성인 체인**을 고정해 버린다. 즉 Arc 로 만들어진 자율 결제가
    // 비번 없이 **Base 로 나갈 수 있다** — 요청-체인 가드가 있으나 마나가 된다.
    // 작업 전체를 요청의 체인으로 묶으면 그 창이 사라진다(안쪽 do_* 의 고정은 같은 값이 된다).
    // chain_id 0 = 옛 미각인 요청 → 예전대로 활성 체인을 쓴다.
    let pinned = if req.chain_id != 0 {
        req.chain_id
    } else {
        active_chain().chain_id
    };
    crate::chain::with_pinned_chain(pinned, auto_approve_pinned(req, session)).await
}

/// 체인이 고정된 채로 도는 자율 승인 본체 — 한도·잔액·서명·전송·장부가 모두 같은 체인을 본다.
async fn auto_approve_pinned(
    req: crate::ipc::PaymentRequest,
    session: tauri::State<'_, SessionKey>,
) -> Result<PaymentResult, String> {
    // 자율 한도 판정 — USDC 표시 결제(x402 서명·USDC 송금)만, 한도 이하만.
    // 음수 한도는 0(자율 꺼짐)으로, 음수 금액은 오류로 — 거대 U256 둔갑 함정 차단(parse_usdc_nonneg).
    let dec = active_chain().usdc_decimals;
    let settings = read_settings();
    let auto_limit: U256 =
        parse_usdc_nonneg(&settings.auto_approve_usdc, dec).unwrap_or(U256::ZERO);
    let amount: U256 = parse_usdc_nonneg(&req.amount, dec)?;
    if !within_auto_limit(&req.token, amount, auto_limit) {
        return Err(NEEDS_PASSWORD.into());
    }

    // 신뢰 주소 가드 (Session 16) — 사람이 비번으로 승인한 적 없는 새 주소는 자율 대상이
    // 아니다(금액 한도만으로는 처음 보는 주소로도 비번 없이 나가는 구멍). 첫 1회만 비번.
    if settings.auto_trusted_only && !is_trusted(&req.to) {
        return Err(NEEDS_PASSWORD.into());
    }

    // ERC-8004 대조가 어긋난 결제는 자율 대상이 아니다 (개발 47, 코덱스 2차 P1).
    // 승인 창에 경고를 만들어 두고 그 창을 건너뛰면 경고가 없는 것과 같다 → 사람 앞으로 돌린다.
    // 주장이 없는 결제(대다수)는 이 조건에 걸리지 않아 예전 동작 그대로다.
    if crate::ipc::agent_contradicts(req.agent.as_ref()) {
        return Err(NEEDS_PASSWORD.into());
    }

    // 여기서부터는 돈이 나갈 수 있는 구간 — 감시 스레드가 이 요청을 실패로 끝내면 안 된다
    // (코덱스 개발51 1차 P1: 자율 경로는 창이 없어 「프론트가 잔다」와 구별이 더 어렵다).
    let _in_flight = crate::ipc::ApprovalGuard::new();

    // 세션이 잠금 해제돼 있어야 키가 메모리에 있다(유휴 타임아웃도 여기서 검사).
    let idle = auto_lock_secs(&settings);
    let signer = session_signer(&session, idle).ok_or_else(|| NEEDS_PASSWORD.to_string())?;

    // 가스가 곧 USDC 인 체인(Arc): 보낼 금액과 가스가 **같은 잔액**에서 나간다 → 잔액에 딱 맞는
    // 송금은 가스를 못 내 체인에서 실패한다. 보내기 화면(SendCard)과 승인 창은 여유분을 빼고
    // 보여주지만 **자율 경로는 그 화면들을 안 지난다**(개발 50 이월 P2).
    //
    // 여기서 하는 일은 «승인된 금액을 깎는 것»이 아니라 **아직 아무도 승인하지 않은 자율 처리를
    // 사람 앞으로 돌리는 것**이다(NEEDS_PASSWORD). 사람은 승인 창에서 이유를 보고 정한다 —
    // 백엔드가 금액을 말없이 줄이는 것은 이 지갑이 하지 않는 일이다.
    // 잔액을 못 읽으면(RPC 오류) 검사를 건너뛴다 — 조회 실패가 자율 결제를 막는 사유는 아니다.
    // x402 는 제외: 우리는 서명만 하고 온체인 제출·가스는 페이실리테이터 몫이라 가스가 안 나간다.
    if req.kind != "x402" && active_chain().native_is_usdc {
        let reserve = parse_usdc_nonneg(active_chain().gas_reserve_usdc, dec).unwrap_or(U256::ZERO);
        // 🔴 **시간 상한을 둔다** (코덱스 개발51 3차 P1). RPC 가 멎으면 이 조회가 무한정 걸리고,
        // 그 사이 상대(MCP)는 5분 만에 요청을 거둬간다 → 나중에 깨어난 이 명령이 **이미 만료된
        // 결제를 보낼** 수 있다. 조회는 판단 재료일 뿐이라 못 읽으면 건너뛰면 된다(아래 재확인이
        // 진짜 방어선). 5초는 개발 50 실측 RPC 왕복(수백 ms)의 열 배 남짓.
        let lookup = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            crate::transfer::usdc_balance_units(signer.address()),
        )
        .await;
        if let Ok(Ok(bal)) = lookup {
            if amount.saturating_add(reserve) > bal {
                return Err(NEEDS_PASSWORD.into());
            }
        }
    }

    // 🔴 **await 를 지나온 뒤, 그 요청이 아직 대기 중인지 다시 본다** (코덱스 개발51 3차 P1).
    // `do_send_usdc` 는 요청 파일을 보지 않으므로, 위 조회가 늦어 상대가 시간 초과로 요청을
    // 거둬간 뒤에도 그대로 돈을 보내 버린다 — 그쪽은 이미 재시도했을 수 있다(이중 결제).
    // 사라졌거나 다른 요청으로 바뀌었으면 여기서 멈춘다(아무도 승인하지 않은 자율 처리다).
    if read_request().map(|r| r.id).as_deref() != Some(req.id.as_str()) {
        return Err(ts!(
            "결제 요청이 이미 취소됐거나 시간이 지났어요.",
            "That payment request was already cancelled or timed out."
        )
        .into());
    }

    // 실제 처리 — 긴급잠금·단일/일일 한도·내역·누적은 do_* 가 송금과 동일하게 적용.
    // 여기서 Err(잠금·한도 등)이면 요청을 치우지 않는다 → 프론트가 모달로 사람에게 넘긴다.
    let notice = auto_pay_notice(
        &req.kind,
        &req.token,
        &req.amount,
        &req.to,
        &req.resource,
        settings.notify_hide_amount,
    );
    let result = match req.kind.as_str() {
        "x402" => {
            let payment = do_sign_x402(&signer, req.to.clone(), req.amount.clone(), None).await?;
            PaymentResult {
                id: req.id,
                status: "approved".into(),
                tx_hash: String::new(),
                detail: String::new(),
                x402: Some(payment),
            }
        }
        _ => {
            let hash = do_send_usdc(&signer, req.to.clone(), req.amount.clone()).await?;
            PaymentResult {
                id: req.id,
                status: "approved".into(),
                tx_hash: hash,
                detail: String::new(),
                x402: None,
            }
        }
    };
    resolve_request(&result)?;

    // 자율 결제 사후 통지 (Session 15) — 비번 없이 돈이 나간 유일한 경로이므로,
    // 보호자가 자리에 없어도 OS 알림으로 인지하게 한다. 알림 실패가 결제를 막으면 안 된다.
    if settings.notify_auto {
        show_notification(&notice.0, &notice.1);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 자율 한도 판정: USDC 표시 + 한도>0 + 금액≤한도 일 때만 자율 가능.
    #[test]
    fn within_auto_limit_gating() {
        let one = U256::from(1_000_000u64); // 1 USDC
        let two = U256::from(2_000_000u64); // 2 USDC
                                            // 한도 2, 금액 1 → 자율 가능
        assert!(within_auto_limit("USDC", one, two));
        // 한도 2, 금액 정확히 2 → 가능(이하 포함)
        assert!(within_auto_limit("USDC", two, two));
        // 한도 1, 금액 2 → 초과 → 사람 승인 필요
        assert!(!within_auto_limit("USDC", two, one));
        // 한도 0 = 자율 꺼짐 → 항상 불가
        assert!(!within_auto_limit("USDC", one, U256::ZERO));
        // ETH 송금은 자율 대상 아님(USDC만)
        assert!(!within_auto_limit("ETH", one, two));
    }

    // 자동 잠금(분) → 초 환산. 0이면 유휴 잠금 안 함.
    #[test]
    fn auto_lock_secs_converts_minutes() {
        let mut s = Settings {
            auto_lock_mins: "30".into(),
            ..Default::default()
        };
        assert_eq!(auto_lock_secs(&s), 1800);
        s.auto_lock_mins = "0".into();
        assert_eq!(auto_lock_secs(&s), 0); // 명시적 0 = 사용자가 끈 것 → 존중
        s.auto_lock_mins = "쓰레기".into(); // 깨진 값 → 기본 30분(잠금이 조용히 꺼지면 안 됨)
        assert_eq!(auto_lock_secs(&s), 1800);
    }
}
