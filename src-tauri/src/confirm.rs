// 내역의 결말을 체인에서 확인한다 (개발 71 — 개발 64·66 「다음」 이월 셋을 한 틀로).
//
// 내역은 **보낸 순간** 적힌다. 그 뒤에 체인에서 벌어진 일은 여태 아무도 되돌아보지 않았다:
//   · "sent"     — 제출은 받혔는데 체인에서 **되돌려졌을(revert)** 수 있다. 그래도 내역은 「보냄」, 오늘 한도는 깎인 채였다
//                  (개발 64 opus·코덱스 공통 이월). 되돌려진 결제는 돈이 안 나갔다(가스만) — 한도를 돌려줘야 맞다.
//   · "unknown"  — 제출이 받혔는지 모른 채 끝났다(개발 66). 나중에 영수증이 생겨도 「확인 필요」로 영영 남았다.
//   · "signed"   — x402 서명만 하고 정산은 페이실리테이터 몫인데, 정산 결과가 MCP 로 안 돌아오면(응답 유실·MCP 가 죽음)
//                  「정산 대기」로 영영 남았고 한도도 깎인 채였다. 인가의 유효 시간이 지난 뒤 `authorizationState` 를
//                  체인에 물으면 **쓰였는지 아닌지가 확정**된다(개발 66 「다음」 4번).
//
// 🔴 **판단은 GUI 가 체인에서 직접 본 것으로만 한다.** MCP 가 「revert 했다」고 말하는 걸 믿고 한도를 돌려주면 그게
// 한도 우회 구멍이 된다(개발 64 코덱스). 여기서 묻는 RPC 는 사용자가 정한 그 RPC 이고, 체인 ID 부터 대조한다.
//
// 한도 환불은 **내역을 먼저 고치고 그다음**이다 — 반대 순서면 내역 쓰기가 실패할 때 다음 차례가 또 환불한다(한도 우회).
// 내역만 고치고 환불을 못 하면 사용자에게 불리할 뿐(한도가 덜 남는다) 돈이 새지는 않는다.

use alloy::primitives::{Address, B256};
use alloy::providers::{Provider, ProviderBuilder};
use std::time::Duration;

use crate::chain::{chain_by_id, with_pinned_chain, IEIP3009};
use crate::history::HistoryEntry;
use crate::settings::effective_rpc;
use crate::store::now_secs;

/// 한 차례에 묻는 건수 상한 — 옛 기록이 많아도 RPC 를 몰아치지 않게.
const MAX_PER_TICK: usize = 12;
/// 이보다 오래된 기록은 묻지 않는다(한도 환불은 어차피 같은 날만, 상태 표시는 일주일이면 충분).
const LOOKBACK_SECS: u64 = 7 * 86_400;
/// 보낸 직후엔 아직 채굴 전이다 — 이만큼 지난 뒤부터 영수증을 묻는다.
const MINE_GRACE_SECS: u64 = 15;
/// x402 서명의 유효 시간 — `x402::do_sign_x402_inner` 의 기본값과 같다(승인 경로는 값을 안 넘긴다).
const SIGN_VALID_SECS: u64 = crate::x402::DEFAULT_VALID_SECS;
/// 유효 시간이 끝난 뒤 더 기다리는 여유 — RPC 노드가 몇 블록 늦어도 「안 쓰였다」로 잘못 읽지 않게.
const SIGN_EXPIRY_MARGIN_SECS: u64 = 300;
/// RPC 한 번의 상한.
const CALL_WAIT: Duration = Duration::from_secs(10);

/// 이 기록에 물어볼 것.
#[derive(Debug, PartialEq)]
enum Ask {
    /// tx 해시로 영수증.
    Receipt(B256),
    /// x402 인가 nonce 로 `authorizationState` — 유효 시간이 이미 끝난 뒤에만.
    AuthUsed(B256),
}

/// 체인에서 확인한 결말.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum Verdict {
    /// 채굴됐고 성공.
    Mined,
    /// 채굴됐는데 되돌려짐 — 돈은 안 나갔다(가스만).
    Reverted,
    /// x402 인가가 쓰였다 = 정산됐다.
    AuthUsed,
    /// x402 인가가 유효 시간 안에 안 쓰였다 = 앞으로도 못 쓴다.
    AuthExpired,
}

/// 기록 하나에 무엇을 물을지 (순수 — 테스트용). 물을 게 없으면 None.
fn ask_for(e: &HistoryEntry, now: u64) -> Option<Ask> {
    if e.checked || e.ts > now || now - e.ts > LOOKBACK_SECS {
        return None;
    }
    match e.status.as_str() {
        "sent" | "unknown" if now - e.ts >= MINE_GRACE_SECS => {
            e.detail.parse().ok().map(Ask::Receipt)
        }
        "signed" if now - e.ts > SIGN_VALID_SECS + SIGN_EXPIRY_MARGIN_SECS => {
            e.detail.parse().ok().map(Ask::AuthUsed)
        }
        _ => None,
    }
}

/// 결말을 기록에 적는다 (순수 — 테스트용). 적었으면 true. 이미 다른 상태가 됐거나 확인된 기록은 안 건드린다.
/// 돌려주는 두 번째 값 = 한도를 돌려줘야 하는가(돈이 안 나간 것으로 확정됐다).
pub(crate) fn apply_verdict(e: &mut HistoryEntry, v: Verdict) -> (bool, bool) {
    if e.checked {
        return (false, false);
    }
    let (from, to, refund): (&[&str], &str, bool) = match v {
        Verdict::Mined => (&["sent", "unknown"], "sent", false),
        Verdict::Reverted => (&["sent", "unknown"], "reverted", true),
        Verdict::AuthUsed => (&["signed"], "settled", false),
        Verdict::AuthExpired => (&["signed"], "expired", true),
    };
    if !from.contains(&e.status.as_str()) {
        return (false, false);
    }
    e.status = to.into();
    e.checked = true;
    (true, refund)
}

/// 환불을 적용할 날 — 기록의 날이 **오늘**이고 자정 언저리가 아닐 때만(순수 — 테스트용).
/// 한도는 예약한 날(UTC 일)에 걸리는데 기록엔 그 날이 없다. 예약은 기록보다 최대 몇 분 앞서므로(전송 상한 합 ≈ 100초),
/// 기록 시각 5분 전과 같은 날일 때만 「예약한 날 = 기록한 날」이 확실하다. 아니면 환불하지 않는다(사용자에게 불리한 쪽).
fn refund_day(ts: u64, now: u64) -> Option<u64> {
    let day = ts / 86_400;
    (day == now / 86_400 && ts.saturating_sub(300) / 86_400 == day).then_some(day)
}

/// 한 차례 — 활성 체인의 모든 계정 내역에서 물을 것을 모아 묻고 적는다. 반환 = 고친 기록 수.
pub(crate) async fn tick() -> Result<usize, String> {
    if crate::wallet::needs_setup() {
        return Ok(0);
    }
    let chain = crate::chain::active_chain();
    let url = effective_rpc();
    with_pinned_chain(chain.chain_id, tick_with(chain.chain_id, url)).await
}

struct Job {
    index: u32,
    owner: Address,
    entry: HistoryEntry,
    ask: Ask,
}

async fn tick_with(chain_id: u64, url: String) -> Result<usize, String> {
    let now = now_secs();
    let accounts = crate::wallet::read_encrypted()?.accounts();
    let mut jobs: Vec<Job> = Vec::new();
    'outer: for a in &accounts {
        let Ok(owner) = a.address.parse::<Address>() else {
            continue;
        };
        for e in crate::history::read_account_history(a.index) {
            if !crate::policy::history_owned_by(&e, &a.address) {
                continue;
            }
            if let Some(ask) = ask_for(&e, now) {
                jobs.push(Job {
                    index: a.index,
                    owner,
                    entry: e,
                    ask,
                });
                if jobs.len() >= MAX_PER_TICK {
                    break 'outer;
                }
            }
        }
    }
    if jobs.is_empty() {
        return Ok(0); // 대부분의 차례 — RPC 를 부르지 않는다.
    }
    let provider = ProviderBuilder::new()
        .connect(&url)
        .await
        .map_err(|e| crate::settings::redact_urls(&e.to_string()))?;
    // 지정 RPC 가 다른 체인이면 남의 체인 영수증으로 이 체인 기록을 고치게 된다 — 입금 찾기와 같은 대조.
    let got = tokio::time::timeout(CALL_WAIT, provider.get_chain_id())
        .await
        .map_err(|_| "RPC 시간 초과".to_string())?
        .map_err(|e| crate::settings::redact_urls(&e.to_string()))?;
    if chain_by_id(got).map(|c| c.chain_id) != Some(chain_id) {
        return Err(format!("RPC 체인 {got} ≠ {chain_id}"));
    }
    let usdc = crate::chain::active_chain().usdc_address;
    let mut fixed = 0usize;
    for job in jobs {
        let verdict = match job.ask {
            Ask::Receipt(hash) => {
                match tokio::time::timeout(CALL_WAIT, provider.get_transaction_receipt(hash)).await
                {
                    Ok(Ok(Some(r))) => {
                        if r.status() {
                            Verdict::Mined
                        } else {
                            Verdict::Reverted
                        }
                    }
                    _ => continue, // 아직 없음·RPC 오류 — 다음 차례에 다시
                }
            }
            Ask::AuthUsed(nonce) => {
                let c = IEIP3009::new(usdc, &provider);
                match tokio::time::timeout(CALL_WAIT, c.authorizationState(job.owner, nonce).call())
                    .await
                {
                    Ok(Ok(true)) => Verdict::AuthUsed,
                    Ok(Ok(false)) => Verdict::AuthExpired,
                    _ => continue,
                }
            }
        };
        match crate::history::apply_confirmation(job.index, &job.entry, verdict) {
            Ok(true) => {
                fixed += 1;
                let refund = matches!(verdict, Verdict::Reverted | Verdict::AuthExpired);
                if refund {
                    if let (Some(day), Some(value)) =
                        (refund_day(job.entry.ts, now), refund_value(&job.entry))
                    {
                        crate::limits::refund_spend(&job.entry.token, value, day).await;
                    }
                }
            }
            Ok(false) => {}
            Err(e) => eprintln!("[confirm] {e}"),
        }
    }
    Ok(fixed)
}

fn refund_value(e: &HistoryEntry) -> Option<alloy::primitives::U256> {
    if e.token == "ETH" {
        crate::limits::parse_eth_nonneg(&e.amount).ok()
    } else {
        crate::limits::parse_usdc_nonneg(&e.amount, crate::chain::active_chain().usdc_decimals).ok()
    }
}

/// 상주 작업 — 30초마다 한 차례. 고친 게 있으면 화면에 `history-changed` 를 보낸다(내역·오늘 한도를 다시 읽게).
pub(crate) fn spawn(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        use tauri::Emitter;
        loop {
            match tick().await {
                Ok(n) if n > 0 => {
                    let _ = app.emit("history-changed", n);
                }
                Ok(_) => {}
                Err(e) => eprintln!("[confirm] {}", crate::settings::redact_urls(&e)),
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";

    fn rec(ts: u64, status: &str, detail: &str) -> HistoryEntry {
        HistoryEntry {
            ts,
            token: "USDC".into(),
            to: "0xabc".into(),
            amount: "1".into(),
            status: status.into(),
            detail: detail.into(),
            ..Default::default()
        }
    }

    // 무엇을 묻나: 보낸 것·불명은 채굴 여유 뒤 영수증, 서명은 유효 시간 + 여유가 지난 뒤 인가 상태.
    // 확인한 것·오래된 것·해시가 아닌 것·다른 상태는 안 묻는다.
    #[test]
    fn what_to_ask() {
        let now = 1_000_000;
        let h: B256 = HASH.parse().unwrap();
        assert_eq!(
            ask_for(&rec(now - 20, "sent", HASH), now),
            Some(Ask::Receipt(h))
        );
        assert_eq!(
            ask_for(&rec(now - 20, "unknown", HASH), now),
            Some(Ask::Receipt(h))
        );
        assert_eq!(ask_for(&rec(now - 5, "sent", HASH), now), None); // 채굴 여유 전
        let expired = now - SIGN_VALID_SECS - SIGN_EXPIRY_MARGIN_SECS - 1;
        assert_eq!(
            ask_for(&rec(expired, "signed", HASH), now),
            Some(Ask::AuthUsed(h))
        );
        assert_eq!(ask_for(&rec(expired + 2, "signed", HASH), now), None); // 아직 쓰일 수 있다
        let mut done = rec(now - 20, "sent", HASH);
        done.checked = true;
        assert_eq!(ask_for(&done, now), None);
        assert_eq!(
            ask_for(&rec(now - LOOKBACK_SECS - 1, "sent", HASH), now),
            None
        );
        assert_eq!(ask_for(&rec(now - 20, "sent", "사유"), now), None);
        for s in [
            "blocked", "failed", "settled", "received", "reverted", "expired",
        ] {
            assert_eq!(ask_for(&rec(now - 20_000, s, HASH), now), None, "{s}");
        }
        assert_eq!(ask_for(&rec(now + 60, "sent", HASH), now), None); // 미래 시각
    }

    // 결말 적기: 기대한 상태에서만, 한 번만. 환불 여부는 「돈이 안 나간 것으로 확정」일 때만.
    #[test]
    fn verdicts_change_only_expected_states() {
        let mut e = rec(0, "unknown", HASH);
        assert_eq!(apply_verdict(&mut e, Verdict::Mined), (true, false));
        assert_eq!((e.status.as_str(), e.checked), ("sent", true));
        assert_eq!(apply_verdict(&mut e, Verdict::Reverted), (false, false)); // 이미 확인됨

        let mut e = rec(0, "sent", HASH);
        assert_eq!(apply_verdict(&mut e, Verdict::Reverted), (true, true));
        assert_eq!(e.status, "reverted");

        let mut e = rec(0, "signed", HASH);
        assert_eq!(apply_verdict(&mut e, Verdict::AuthUsed), (true, false));
        assert_eq!(e.status, "settled");
        let mut e = rec(0, "signed", HASH);
        assert_eq!(apply_verdict(&mut e, Verdict::AuthExpired), (true, true));
        assert_eq!(e.status, "expired");

        // 그새 MCP 정산이 먼저 와서 「settled」 가 된 기록엔 만료를 적지 않는다.
        let mut e = rec(0, "settled", HASH);
        assert_eq!(apply_verdict(&mut e, Verdict::AuthExpired), (false, false));
        let mut e = rec(0, "signed", HASH);
        assert_eq!(apply_verdict(&mut e, Verdict::Mined), (false, false));
    }

    // 환불은 기록이 오늘이고 자정 5분 안쪽이 아닐 때만.
    #[test]
    fn refund_only_same_day_away_from_midnight() {
        let day = 20_000u64;
        let noon = day * 86_400 + 43_200;
        assert_eq!(refund_day(noon, noon + 60), Some(day));
        assert_eq!(refund_day(noon, noon + 86_400), None); // 어제 기록
        let just_after_midnight = day * 86_400 + 100;
        assert_eq!(
            refund_day(just_after_midnight, just_after_midnight + 10),
            None
        );
    }
}
