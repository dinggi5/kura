// 거래 내역 (Session 8) + x402 정산 추적 (Session 14 후속).
//
// 모든 송금/서명 시도(성공·차단·실패)를 ~/.jigap/history.json 에 남긴다 (감사 로그).
// x402 정산 tx 해시는 MCP만 안다(GUI가 서명 → MCP가 제출 → 페이실리테이터 온체인 정산 →
// MCP가 PAYMENT-RESPONSE로 받음). MCP가 ~/.jigap/x402_settlements.json 에 {nonce, tx, success}를
// 기록하면, GUI 폴링이 읽어 매칭되는 "signed" 내역(detail=nonce)을 "settled"+tx 로 갱신한다.

use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

use crate::chain::chain_file;
use crate::settings::redact_urls;
use crate::store::{jigap_dir, now_secs, write_json};
use crate::wallet::{account_file, account_file_name};

/// 송금 시도 1건의 기록 — 형식의 정본은 `policy::HistoryEntry`(MCP·CLI 가 같은 타입으로 읽는다, 개발 57).
pub(crate) use crate::policy::HistoryEntry;

/// 거래 내역 최대 보관 개수 (오래된 건 자동으로 밀려난다).
const HISTORY_CAP: usize = 200;

/// 활성 계정(작업이 고정했으면 그 계정)의 내역 파일 (개발 54: 체인별 + 계정별).
/// 내역은 주소의 것이다 — 계정 2 의 화면에 계정 1 의 송금이 보이면 안 된다.
fn history_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join(account_file("history")))
}

/// 특정 계정의 내역 파일 — x402 정산 반영이 모든 계정을 훑을 때 쓴다.
fn history_path_for(index: u32) -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join(account_file_name(&chain_file("history"), index)))
}

fn read_history_at(path: &PathBuf) -> Vec<HistoryEntry> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 저장된 거래 내역을 읽는다 (최신순으로 저장돼 있다).
fn read_history() -> Vec<HistoryEntry> {
    history_path()
        .map(|p| read_history_at(&p))
        .unwrap_or_default()
}

/// 새 기록을 맨 앞에 넣고 최대 cap개로 자른다 (순수 함수 — 파일 I/O 없음, 테스트용).
fn with_entry(mut list: Vec<HistoryEntry>, entry: HistoryEntry, cap: usize) -> Vec<HistoryEntry> {
    list.insert(0, entry);
    list.truncate(cap);
    list
}

/// 송금 시도 1건을 내역에 추가한다. 실패해도 송금 흐름은 막지 않는다(로그는 부가 기능).
pub(crate) fn log_attempt(token: &str, to: &str, amount: &str, status: &str, detail: &str) {
    let entry = HistoryEntry {
        ts: now_secs(),
        token: token.into(),
        to: to.into(),
        amount: amount.into(),
        status: status.into(),
        detail: detail.into(),
        settle_tx: String::new(),
    };
    let list = with_entry(read_history(), entry, HISTORY_CAP);
    let _ = history_path().and_then(|p| write_json(p, &list));
}

/// detail 의 URL 을 가린다(출력용). 이번 패치 이전이 기록한 비redact 에러에 RPC URL·키가
/// 들어 있어도 GUI 로 다시 새지 않게 — 순수 함수라 파일 I/O 없이 검증 가능(코덱스 High 반영).
fn redact_details(mut list: Vec<HistoryEntry>) -> Vec<HistoryEntry> {
    for e in &mut list {
        e.detail = redact_urls(&e.detail);
    }
    list
}

/// 거래 내역을 최신순으로 돌려준다.
#[tauri::command]
pub(crate) fn get_history() -> Vec<HistoryEntry> {
    redact_details(read_history())
}

/// MCP가 기록한 정산 결과 1건. nonce 로 "signed" 내역과 매칭한다.
#[derive(Deserialize)]
struct Settlement {
    /// 서명 인가의 nonce("0x..") — history 의 detail 과 일치해야 매칭.
    nonce: String,
    /// 온체인 정산 tx 해시.
    tx: String,
    /// 페이실리테이터 정산 성공 여부.
    success: bool,
}

fn settlements_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join(chain_file("x402_settlements")))
}

/// 정산 1건을 내역 목록에 적용한다 (순수 함수 — 테스트용).
/// nonce 가 일치하고 아직 "signed" 인 첫 항목을 status="settled"/"settle_failed" + settle_tx 로 갱신.
/// 적용됐으면 true.
fn apply_settlement(list: &mut [HistoryEntry], s: &Settlement) -> bool {
    for e in list.iter_mut() {
        if e.status == "signed" && e.detail == s.nonce {
            e.status = if s.success {
                "settled"
            } else {
                "settle_failed"
            }
            .into();
            e.settle_tx = s.tx.clone();
            return true;
        }
    }
    false
}

/// MCP가 남긴 x402 정산 결과를 읽어 내역에 반영한다 (GUI 1초 폴링). 반영 건수를 돌려준다.
/// 처리 후 정산 파일을 비운다(중복 적용 방지). 매칭 안 되는 건 그냥 버린다.
///
/// 정산 파일은 체인별 하나(계정 공용)인데 내역은 계정별이다 (개발 54). 서명한 계정과 정산이
/// 도착했을 때의 활성 계정이 다를 수 있으므로(서명 → 사용자가 계정 전환 → 페이실리테이터 정산),
/// 활성 계정부터 보고 안 맞으면 **나머지 계정의 내역까지 훑는다** — 안 그러면 그 「signed」 항목이
/// 영영 정산 대기로 남는다.
#[tauri::command]
pub(crate) fn apply_x402_settlements() -> u32 {
    let path = match settlements_path() {
        Ok(p) => p,
        Err(_) => return 0,
    };
    let Ok(raw) = fs::read_to_string(&path) else {
        return 0; // 파일 없음 = 처리할 정산 없음 (대부분의 폴링)
    };
    let settlements: Vec<Settlement> = serde_json::from_str(&raw).unwrap_or_default();
    // 활성 계정 먼저, 그다음 나머지 — 대부분은 첫 파일에서 끝난다.
    let active = crate::wallet::active_account_index();
    let mut indices: Vec<u32> = vec![active];
    if let Ok(w) = crate::wallet::read_encrypted() {
        indices.extend(
            w.accounts()
                .iter()
                .map(|a| a.index)
                .filter(|i| *i != active),
        );
    }
    let mut pending: Vec<&Settlement> = settlements.iter().collect();
    let mut applied = 0u32;
    for index in indices {
        if pending.is_empty() {
            break;
        }
        let Ok(hp) = history_path_for(index) else {
            continue;
        };
        let mut list = read_history_at(&hp);
        if list.is_empty() {
            continue;
        }
        let before = pending.len();
        pending.retain(|s| !apply_settlement(&mut list, s));
        let hit = (before - pending.len()) as u32;
        if hit > 0 {
            applied += hit;
            let _ = write_json(hp, &list);
        }
    }
    let _ = fs::remove_file(&path); // 읽었으면 비운다(매칭 실패분 포함)
    applied
}

#[cfg(test)]
mod tests {
    use super::*;

    // 거래 내역: 최신 항목이 맨 앞에 오고 cap 개수로 잘린다.
    #[test]
    fn history_caps_and_orders_newest_first() {
        fn entry(tag: &str) -> HistoryEntry {
            HistoryEntry {
                ts: 0,
                token: "USDC".into(),
                to: "0x0".into(),
                amount: "1".into(),
                status: "sent".into(),
                detail: tag.into(),
                settle_tx: String::new(),
            }
        }
        let mut list: Vec<HistoryEntry> = Vec::new();
        for i in 0..5 {
            list = with_entry(list, entry(&i.to_string()), 3);
        }
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].detail, "4"); // 마지막에 넣은 게 맨 앞
        assert_eq!(list[2].detail, "2"); // 가장 오래된 2건은 밀려남
    }

    // 거래 내역 항목 JSON 왕복 (한글 사유 포함).
    #[test]
    fn history_entry_roundtrip() {
        let e = HistoryEntry {
            ts: 123,
            token: "ETH".into(),
            to: "0xabc".into(),
            amount: "0.01".into(),
            status: "blocked".into(),
            detail: "긴급 잠금".into(),
            settle_tx: String::new(),
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: HistoryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ts, 123);
        assert_eq!(back.status, "blocked");
        assert_eq!(back.detail, "긴급 잠금");
    }

    // 옛 history.json(settle_tx 필드 없음)도 무손실 로드.
    #[test]
    fn old_history_without_settle_tx_loads() {
        let old = r#"{"ts":1,"token":"USDC","to":"0xabc","amount":"0.01","status":"signed","detail":"0xnonce"}"#;
        let e: HistoryEntry = serde_json::from_str(old).unwrap();
        assert_eq!(e.status, "signed");
        assert_eq!(e.settle_tx, ""); // 새 필드 기본값
    }

    // x402 정산 적용: 매칭되는 "signed" 항목만 "settled"+tx 로 바뀐다.
    #[test]
    fn apply_settlement_updates_matching_signed_entry() {
        let mut list = vec![
            HistoryEntry {
                ts: 1,
                token: "USDC".into(),
                to: "0xpay".into(),
                amount: "0.01".into(),
                status: "signed".into(),
                detail: "0xNONCE".into(),
                settle_tx: String::new(),
            },
            HistoryEntry {
                ts: 2,
                token: "USDC".into(),
                to: "0xother".into(),
                amount: "0.5".into(),
                status: "sent".into(),
                detail: "0xtxhash".into(),
                settle_tx: String::new(),
            },
        ];
        // 매칭 성공 → settled
        let ok = Settlement {
            nonce: "0xNONCE".into(),
            tx: "0xSETTLE".into(),
            success: true,
        };
        assert!(apply_settlement(&mut list, &ok));
        assert_eq!(list[0].status, "settled");
        assert_eq!(list[0].settle_tx, "0xSETTLE");
        assert_eq!(list[1].status, "sent"); // 무관한 항목 불변

        // 같은 nonce 재적용 → 이미 signed 아니라 매칭 안 됨(중복 방지)
        assert!(!apply_settlement(&mut list, &ok));

        // 매칭 없는 nonce → false
        let miss = Settlement {
            nonce: "0xZZZ".into(),
            tx: "0xT".into(),
            success: true,
        };
        assert!(!apply_settlement(&mut list, &miss));

        // 정산 실패 → settle_failed
        let mut list2 = vec![HistoryEntry {
            ts: 1,
            token: "USDC".into(),
            to: "0xpay".into(),
            amount: "0.01".into(),
            status: "signed".into(),
            detail: "0xN2".into(),
            settle_tx: String::new(),
        }];
        let fail = Settlement {
            nonce: "0xN2".into(),
            tx: "0xT2".into(),
            success: false,
        };
        assert!(apply_settlement(&mut list2, &fail));
        assert_eq!(list2[0].status, "settle_failed");
    }

    // 과거에 기록된 비redact detail(RPC URL·키)은 출력 시점에 가려진다(코덱스 High).
    #[test]
    fn get_history_redacts_leaked_url_in_detail() {
        let list = vec![HistoryEntry {
            ts: 1,
            token: "ETH".into(),
            to: "0xabc".into(),
            amount: "0".into(),
            status: "failed".into(),
            detail: "RPC 연결 실패: https://base.alchemy.com/v2/LEAKEDKEY".into(),
            settle_tx: String::new(),
        }];
        let out = redact_details(list);
        assert!(
            !out[0].detail.contains("LEAKEDKEY"),
            "키가 남음: {}",
            out[0].detail
        );
        assert_eq!(out[0].detail, "RPC 연결 실패: [RPC]");
    }
}
