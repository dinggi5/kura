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
// 한도 환불은 결말 쓰기와 **따로**, 장부에서 기록마다 한 번이다(`refund_pass` → `limits::refund_once`, 개발 71 코덱스 2·3차) —
// 결말이 사본 둘에 걸쳐 원자적으로 안 써져도 두 번 주지도, 영영 안 주지도 않는다.

use alloy::primitives::{Address, B256};
use alloy::providers::{Provider, ProviderBuilder};
use std::time::Duration;

use crate::chain::{chain_by_id, with_pinned_chain, IEIP3009};
use crate::history::HistoryEntry;
use crate::settings::effective_rpc;
use crate::store::now_secs;

/// 한 차례에 묻는 건수 상한 — 옛 기록이 많아도 RPC 를 몰아치지 않게.
const MAX_PER_TICK: usize = 12;
/// 돌려 가며 고를 후보의 상한 — 이만큼 모아 그중 `MAX_PER_TICK` 건을 차례로 묻는다.
const MAX_CANDIDATES: usize = 512;

/// 차례마다 MAX_PER_TICK 씩 밀리는 시작 자리.
fn next_offset() -> usize {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    N.fetch_add(MAX_PER_TICK, std::sync::atomic::Ordering::Relaxed)
}

/// `list` 를 `offset` 자리부터 한 바퀴 돌며 `n` 건 (순수 — 테스트용).
fn rotate_take<T>(mut list: Vec<T>, offset: usize, n: usize) -> Vec<T> {
    if list.is_empty() {
        return list;
    }
    let k = offset % list.len();
    list.rotate_left(k);
    list.truncate(n);
    list
}
/// 이보다 오래된 기록은 묻지 않는다(한도 환불은 어차피 같은 날만, 상태 표시는 일주일이면 충분).
const LOOKBACK_SECS: u64 = 7 * 86_400;
/// 보낸 직후엔 아직 채굴 전이다 — 이만큼 지난 뒤부터 영수증을 묻는다.
const MINE_GRACE_SECS: u64 = 15;
/// x402 서명의 유효 시간 — 모든 서명이 이 값이다(개발 71 에 `valid_secs` 인자를 없애 못박았다 — 긴 인가를 「만료」로 오판해 환불하지 않게).
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
        // 「정산 실패」도 묻는다(개발 73, 코덱스 1차 P1) — 서버가 실패라고 해도 서명은 유효 시간 동안 살아 있어 누가 정산할 수 있다.
        "signed" | "settle_failed" if now - e.ts > SIGN_VALID_SECS + SIGN_EXPIRY_MARGIN_SECS => {
            e.detail.parse().ok().map(Ask::AuthUsed)
        }
        _ => None,
    }
}

/// 이 결말이 기록에 남기는 상태.
pub(crate) fn verdict_status(v: Verdict) -> &'static str {
    match v {
        Verdict::Mined => "sent",
        Verdict::Reverted => "reverted",
        Verdict::AuthUsed => "settled",
        Verdict::AuthExpired => "expired",
    }
}

/// 결말을 기록에 적는다 (순수 — 테스트용). 적었으면 true. 이미 다른 상태가 됐거나 확인된 기록은 안 건드린다.
/// 돌려주는 두 번째 값 = 한도를 돌려줘야 하는가(돈이 안 나간 것으로 확정됐다).
pub(crate) fn apply_verdict(e: &mut HistoryEntry, v: Verdict) -> (bool, bool) {
    if e.checked {
        return (false, false);
    }
    let (from, refund): (&[&str], bool) = match v {
        Verdict::Mined => (&["sent", "unknown"], false),
        Verdict::Reverted => (&["sent", "unknown"], true),
        Verdict::AuthUsed => (&["signed", "settle_failed"], false),
        Verdict::AuthExpired => (&["signed", "settle_failed"], true),
    };
    if !from.contains(&e.status.as_str()) {
        return (false, false);
    }
    e.status = verdict_status(v).into();
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

/// 이 서명이 **체인 시각으로** 이 시각을 넘긴 블록에서만 「안 쓰였다 = 만료」를 확정한다 (개발 73, 코덱스 1차 P1).
/// `validBefore` = 서명 시각 + 유효 시간이고 서명 시각 ≤ 기록 시각이라, 기록 시각 + 유효 시간 + 여유는 그보다 늦다.
/// 내 시계만 보면 시계가 앞서거나 RPC 가 뒤처졌을 때 아직 쓰일 수 있는 서명을 「안 나감」으로 적고 한도를 돌려줬다.
fn auth_deadline(e: &HistoryEntry) -> u64 {
    e.ts + SIGN_VALID_SECS + SIGN_EXPIRY_MARGIN_SECS
}

/// 판단 (순수 — 테스트용): 이 블록 시각에서 인가 상태를 물어도 되는가.
fn auth_settled_by(block_ts: u64, e: &HistoryEntry) -> bool {
    block_ts > auth_deadline(e)
}

/// 최신 블록의 번호·시각 — 원시 JSON 으로(Base 블록의 OP 예치 거래는 이더리움 타입으로 안 풀린다, deposits.rs 와 같다).
async fn latest_block<P: Provider>(provider: &P) -> Option<(u64, u64)> {
    let v: serde_json::Value = tokio::time::timeout(
        CALL_WAIT,
        provider.raw_request("eth_getBlockByNumber".into(), ("latest", false)),
    )
    .await
    .ok()?
    .ok()?;
    let hex = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .and_then(|x| u64::from_str_radix(x.trim_start_matches("0x"), 16).ok())
    };
    Some((hex("number")?, hex("timestamp")?))
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
                if jobs.len() >= MAX_CANDIDATES {
                    break 'outer;
                }
            }
        }
    }
    // 한 차례에 묻는 건 MAX_PER_TICK 건 — 시작 자리를 차례마다 돌린다(개발 73, 코덱스 1차 P2). 늘 앞에서부터 고르면 영수증이
    // 끝내 안 나오는 앞쪽 12건이 매번 자리를 차지해 뒤쪽 기록은 영영 확인되지 않았다(환불도 없이).
    let jobs = rotate_take(jobs, next_offset(), MAX_PER_TICK);
    // 환불은 RPC 없이 장부만 본다 — 체인 확인보다 먼저, 매 차례(지난 차례에 결말만 적고 못 준 것까지).
    let refunded = refund_pass(&accounts, now).await;
    if jobs.is_empty() {
        return Ok(refunded); // 대부분의 차례 — RPC 를 부르지 않는다.
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
    // 인가 상태는 한 블록에 못박아 묻는다 — 시각을 본 블록과 상태를 읽은 블록이 같아야 「그 시각에 안 쓰였다」가 성립한다.
    let head = if jobs.iter().any(|j| matches!(j.ask, Ask::AuthUsed(_))) {
        latest_block(&provider).await
    } else {
        None
    };
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
                let Some((block, block_ts)) = head else {
                    continue; // 체인 시각을 모르면 만료를 확정하지 않는다 — 다음 차례에 다시
                };
                if !auth_settled_by(block_ts, &job.entry) {
                    continue; // 체인 시각으로는 아직 유효 시간 안(내 시계가 앞섰거나 RPC 가 뒤처짐)
                }
                let c = IEIP3009::new(usdc, &provider);
                match tokio::time::timeout(
                    CALL_WAIT,
                    c.authorizationState(job.owner, nonce)
                        .call()
                        .block(alloy::eips::BlockId::number(block)),
                )
                .await
                {
                    Ok(Ok(true)) => Verdict::AuthUsed,
                    Ok(Ok(false)) => Verdict::AuthExpired,
                    _ => continue,
                }
            }
        };
        // 환불은 여기서 하지 않는다 — 아래 `refund_pass` 가 장부에서 기록마다 한 번으로 한다(개발 71 코덱스 3차).
        match crate::history::apply_confirmation(job.index, &job.entry, verdict) {
            Ok(true) => fixed += 1,
            Ok(false) => {}
            Err(e) => eprintln!("[confirm] {e}"),
        }
    }
    // 이번 차례에 적은 결말의 환불도 바로 — 다음 차례를 30초 기다리지 않게.
    Ok(refunded + fixed + refund_pass(&accounts, now).await)
}

/// 이 기록의 환불 열쇠 — 고유 id, 옛 기록(개발 71 이전)은 시각·해시.
fn refund_key(e: &HistoryEntry) -> String {
    if e.id.is_empty() {
        format!("{}:{}", e.ts, e.detail)
    } else {
        e.id.clone()
    }
}

/// 환불 대상인가 (순수 — 테스트용): 체인 확인이 「돈이 안 나갔다」로 확정했고(reverted·expired) 오늘 기록이면 그 날.
fn refund_due(e: &HistoryEntry, now: u64) -> Option<u64> {
    (e.checked && matches!(e.status.as_str(), "reverted" | "expired"))
        .then(|| refund_day(e.ts, now))
        .flatten()
}

/// 확정된 「안 나간 결제」의 한도를 돌려준다 — **기록 하나당 한 번**(`limits::refund_once` 가 장부에 열쇠를 적는다).
///
/// 🔴 환불을 결말 쓰기에서 떼어 낸 이유(코덱스 개발 71 3차 P1): 결말은 사본 둘(본 파일·보관 파일)에 적히는데 두 쓰기는 원자적이지
/// 않다. 「적는 데 성공했을 때 환불」이면 한쪽만 써진 실패에서 **영영 안 주거나**(3차) **두 번 줬다**(2차). 이제 결말이 적힌 사본이
/// 하나라도 보이면 차례마다 여기서 장부에 묻는다 — 이미 준 열쇠면 아무 일도 없다. 내역을 쓰고 환불 전에 죽어도 다음 차례에 준다.
async fn refund_pass(accounts: &[crate::policy::Account], now: u64) -> usize {
    let mut n = 0;
    for a in accounts {
        for e in crate::history::read_account_history(a.index) {
            if !crate::policy::history_owned_by(&e, &a.address) {
                continue;
            }
            let (Some(day), Some(value)) = (refund_due(&e, now), refund_value(&e)) else {
                continue;
            };
            if crate::limits::refund_once(&e.token, value, day, &refund_key(&e)).await {
                n += 1;
            }
        }
    }
    n
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
        // 🔴 개발 73: 서버가 「정산 실패」라고 한 서명도 체인에서 묻는다 — 서명은 살아 있다.
        assert_eq!(
            ask_for(&rec(expired, "settle_failed", HASH), now),
            Some(Ask::AuthUsed(h))
        );
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

    // 🔴 개발 73(코덱스 1차 P1): 만료는 **체인 블록 시각**이 기록 시각 + 유효 + 여유를 넘긴 뒤에만.
    #[test]
    fn expiry_judged_by_chain_time() {
        let e = rec(1_000, "signed", HASH);
        let d = 1_000 + SIGN_VALID_SECS + SIGN_EXPIRY_MARGIN_SECS;
        assert!(!auth_settled_by(d, &e));
        assert!(!auth_settled_by(d - 100, &e)); // RPC 가 뒤처짐
        assert!(auth_settled_by(d + 1, &e));
    }

    // 🔴 개발 73(코덱스 1차 P2): 차례마다 시작 자리를 돌려, 앞쪽이 끝내 안 풀려도 뒤쪽이 언젠가 확인된다.
    #[test]
    fn rotation_reaches_every_candidate() {
        let list: Vec<u32> = (0..30).collect();
        let mut seen = std::collections::HashSet::new();
        for t in 0..3 {
            for x in rotate_take(list.clone(), t * 12, 12) {
                seen.insert(x);
            }
        }
        assert_eq!(seen.len(), 30);
        assert_eq!(rotate_take(list.clone(), 0, 12), (0..12).collect::<Vec<_>>());
        assert_eq!(rotate_take(Vec::<u32>::new(), 5, 12), Vec::<u32>::new());
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

        // 🔴 개발 73: 「정산 실패」도 체인 결말로 바뀐다.
        let mut e = rec(0, "settle_failed", HASH);
        assert_eq!(apply_verdict(&mut e, Verdict::AuthUsed), (true, false));
        assert_eq!(e.status, "settled");
        let mut e = rec(0, "settle_failed", HASH);
        assert_eq!(apply_verdict(&mut e, Verdict::AuthExpired), (true, true));
        assert_eq!(e.status, "expired");

        // 그새 MCP 정산이 먼저 와서 「settled」 가 된 기록엔 만료를 적지 않는다.
        let mut e = rec(0, "settled", HASH);
        assert_eq!(apply_verdict(&mut e, Verdict::AuthExpired), (false, false));
        let mut e = rec(0, "signed", HASH);
        assert_eq!(apply_verdict(&mut e, Verdict::Mined), (false, false));
    }

    // 🔴 개발 71(코덱스 3차): 환불 대상은 체인 확인이 「안 나갔다」로 확정한 오늘 기록뿐. 열쇠는 id, 옛 기록은 시각·해시.
    #[test]
    fn refund_due_only_for_confirmed_unpaid_today() {
        let now = 20_000 * 86_400 + 43_200;
        let mut e = rec(now - 60, "reverted", HASH);
        assert_eq!(refund_due(&e, now), None); // 확인 표시가 없다
        e.checked = true;
        assert_eq!(refund_due(&e, now), Some(20_000));
        e.status = "expired".into();
        assert_eq!(refund_due(&e, now), Some(20_000));
        for s in ["sent", "settled", "unknown", "failed"] {
            e.status = s.into();
            assert_eq!(refund_due(&e, now), None, "{s}");
        }
        e.status = "reverted".into();
        e.ts = now - 86_400;
        assert_eq!(refund_due(&e, now), None); // 어제
        assert_eq!(refund_key(&e), format!("{}:{HASH}", e.ts));
        e.id = "abc".into();
        assert_eq!(refund_key(&e), "abc");
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
