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

/// 🔴 **내역 파일의 읽기-수정-쓰기를 한 줄로 세운다** (개발 66, 코덱스 P1). 내역은 통째로 읽어 한 줄 넣고
/// 통째로 쓴다 — 두 송금이 거의 같이 끝나면 둘 다 같은 목록을 읽고, 나중에 쓴 쪽이 먼저 쓴 기록을 지운다.
/// 정산 반영(`apply_x402_settlements`)도 같은 파일을 고친다. 내역을 쓰는 건 이 프로세스(GUI)뿐이라
/// 프로세스 안 잠금이면 된다. 독살(poison)돼도 계속 쓴다 — 기록을 멈추는 편이 더 나쁘다.
static HISTORY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn history_guard() -> std::sync::MutexGuard<'static, ()> {
    HISTORY_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// 송금 시도 1건의 기록 — 형식의 정본은 `policy::HistoryEntry`(MCP·CLI 가 같은 타입으로 읽는다, 개발 57).
pub(crate) use crate::policy::HistoryEntry;

/// 내역 본 파일이 품는 최신 기록 수 — 넘치면 보관 파일로 옮긴다(`policy::HISTORY_HOT_CAP`, 개발 70).
const HISTORY_CAP: usize = crate::policy::HISTORY_HOT_CAP;

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

/// 내역 파일이 **있는데** 못 읽거나 깨졌는가 — 없는 것(아직 기록 없음)과 가른다.
fn history_unreadable(path: &PathBuf) -> bool {
    match fs::read_to_string(path) {
        Ok(s) => serde_json::from_str::<Vec<HistoryEntry>>(&s).is_err(),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// 새 기록을 맨 앞에 넣고, cap 을 넘친 오래된 기록을 떼어 돌려준다 (순수 함수 — 파일 I/O 없음, 테스트용).
/// 반환 = (본 파일에 남길 목록, 보관 파일로 옮길 기록 — 최신순).
fn with_entry(
    mut list: Vec<HistoryEntry>,
    entry: HistoryEntry,
    cap: usize,
) -> (Vec<HistoryEntry>, Vec<HistoryEntry>) {
    list.insert(0, entry);
    let evicted = if list.len() > cap {
        list.split_off(cap)
    } else {
        Vec::new()
    };
    (list, evicted)
}

/// 보관 파일에 쓸 줄들 — 넘겨받은 기록(최신순)을 **오래된 것부터** 한 줄씩. 순수 함수(테스트용).
/// `tail` = 보관 파일 끝의 기록들(오래된 순). 쓸 기록의 **앞부분이 보관 파일의 끝부분과 겹치면** 그만큼 뺀다 —
/// 덧붙인 뒤 본 파일을 쓰기 전에 죽으면 그 기록들이 본 파일에도 남아, 다음 번에 또 밀려나며 두 번 적힌다.
/// 🔴 한때 마지막 한 줄만 비교했다(코덱스 개발 70 2차 P1) — 보관에 실패해 본 파일이 cap 을 넘긴 뒤 여러 건을 한꺼번에
/// 옮기다 죽으면, 첫 기록이 마지막 줄과 달라 겹친 구간 전체가 다시 적혔다.
/// (똑같은 기록 — 같은 초·같은 금액·같은 사유 — 이 연달아 있으면 하나로 볼 수 있다. 그 값으로 중복을 막는다.)
fn archive_lines(evicted: &[HistoryEntry], tail: &[HistoryEntry]) -> String {
    let oldest_first: Vec<&HistoryEntry> = evicted.iter().rev().collect();
    let overlap = (1..=oldest_first.len().min(tail.len()))
        .rev()
        .find(|&m| {
            tail[tail.len() - m..]
                .iter()
                .zip(&oldest_first[..m])
                .all(|(a, b)| a == *b)
        })
        .unwrap_or(0);
    let mut out = String::new();
    for e in &oldest_first[overlap..] {
        if let Ok(line) = serde_json::to_string(e) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// 보관 파일 끝의 기록들(오래된 순) — 끝의 256KB 만 읽는다(보관 파일은 몇 MB 까지 커질 수 있다).
/// 자른 자리가 한글 글자 중간일 수 있어 손실 허용 변환으로 읽는다(`read_to_string` 은 거기서 통째로 실패했다).
/// 잘린 첫 줄은 JSON 이 아니라 걸러진다.
fn archive_tail(path: &std::path::Path) -> Vec<HistoryEntry> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = fs::File::open(path) else {
        return Vec::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if f.seek(SeekFrom::Start(len.saturating_sub(256 * 1024)))
        .is_err()
    {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if f.read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// 밀려난 기록을 보관 파일 끝에 덧붙인다. 한 번의 쓰기로(줄 사이에서 끊기지 않게).
/// 앞선 쓰기가 줄 중간에 죽어 파일이 개행으로 안 끝나면 개행부터 — 새 줄이 반쪽 줄에 붙어 같이 버려지지 않게.
fn append_archive(path: &std::path::Path, evicted: &[HistoryEntry]) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom, Write};
    let mut body = archive_lines(evicted, &archive_tail(path));
    if body.is_empty() {
        return Ok(());
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    if len > 0 {
        let mut last = [0u8; 1];
        f.seek(SeekFrom::Start(len - 1))
            .map_err(|e| e.to_string())?;
        f.read_exact(&mut last).map_err(|e| e.to_string())?;
        if last[0] != b'\n' {
            body.insert(0, '\n');
        }
    }
    f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())
}

/// 송금 시도 1건을 내역에 추가한다. 실패해도 송금 흐름은 막지 않는다(로그는 부가 기능).
///
/// 🔴 (개발 70) 두 가지 조용한 유실을 막는다:
/// - **200건 넘친 기록** — 예전엔 잘라 버렸다. 이제 보관 파일에 덧붙이고, 덧붙이기가 실패하면 **자르지 않는다**
///   (본 파일이 잠시 200건을 넘을 뿐, 다음 기록 때 다시 옮긴다).
/// - **깨진 본 파일** — 예전엔 빈 목록으로 읽고 새 한 건으로 덮어써 옛 기록을 통째로 지웠다(개발 69 입금 기록과 같은
///   병). 이제 깨진 파일은 `….broken.<초>.json` 으로 옆에 치우고 새로 시작한다 — 기록은 멈추지 않고 옛 파일은 남는다.
///   치우지도 못하면(권한 등) 이번 기록을 포기한다 — 덮어쓰는 것보다 낫다.
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
    let _g = history_guard();
    let Ok(path) = history_path() else {
        return;
    };
    let _ = record_at(&path, entry, HISTORY_CAP);
}

/// `log_attempt` 의 본체 — 경로를 받는다(테스트가 실지갑을 안 건드리게).
fn record_at(path: &PathBuf, entry: HistoryEntry, cap: usize) -> Result<(), String> {
    if history_unreadable(path) {
        let aside = path.with_extension(format!("broken.{}.json", now_secs()));
        fs::rename(path, &aside).map_err(|e| e.to_string())?;
    }
    let (mut list, evicted) = with_entry(read_history_at(path), entry, cap);
    if !evicted.is_empty()
        && append_archive(&crate::policy::history_archive_path(path), &evicted).is_err()
    {
        list.extend(evicted); // 옮기지 못했으면 자르지 않는다
    }
    write_json(path.clone(), &list)
}

/// detail 의 URL 을 가린다(출력용). 이번 패치 이전이 기록한 비redact 에러에 RPC URL·키가
/// 들어 있어도 GUI 로 다시 새지 않게 — 순수 함수라 파일 I/O 없이 검증 가능(코덱스 High 반영).
fn redact_details(mut list: Vec<HistoryEntry>) -> Vec<HistoryEntry> {
    for e in &mut list {
        e.detail = redact_urls(&e.detail);
    }
    list
}

/// 거래 내역을 최신순으로 돌려준다 — 보낸 기록에 **입금 기록**(개발 69)을 시각순으로 섞는다
/// (`policy::merge_received` — MCP·CLI 와 같은 함수).
///
/// `limit` = 돌려줄 줄 수(화면의 「더 보기」가 늘린다, 개발 70). 보낸 기록은 본 파일 다음에 보관 파일까지 읽는다.
#[tauri::command]
pub(crate) fn get_history(limit: Option<usize>) -> Vec<HistoryEntry> {
    let limit = limit.unwrap_or(HISTORY_CAP);
    let sent = history_path()
        .map(|p| crate::policy::read_sent_history(&p, limit))
        .unwrap_or_default();
    let mut list =
        crate::policy::merge_received(redact_details(sent), &crate::deposits::read_deposits());
    list.truncate(limit);
    list
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

/// 정산 파일을 고유한 이름으로 가져와(rename) 가져온 묶음 전부를 읽는다 — 전에 읽다 실패해 남겨 둔 묶음 포함.
/// 반환 = (정산들, 다 읽어서 지워도 되는 파일들). **경로를 받는다** — 테스트가 실지갑을 안 건드리게.
fn claim_settlements(path: &std::path::Path) -> (Vec<Settlement>, Vec<PathBuf>) {
    let Some(dir) = path.parent().map(PathBuf::from) else {
        return (Vec::new(), Vec::new());
    };
    let stem = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let claim_prefix = format!(".{stem}.claimed.");
    if fs::metadata(path).is_ok() {
        // 이름은 가져올 때마다 다르다(초 단위로 지으면 같은 초의 두 번째가 첫 번째를 덮었다 — 2차 P1).
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // 프로세스 id 도 싣는다 — 순번은 프로세스마다 0 부터라, 같은 초에 앱이 재시작하면 남겨 둔 묶음을 덮는다(3차 P1).
        let name = format!("{claim_prefix}{}.{}.{n}", now_secs(), std::process::id());
        let _ = fs::rename(path, dir.join(name));
    }
    // 가져온 묶음 전부 — 방금 것과, 전에 **읽다 실패해 남겨 둔** 것(2차 P2: 못 읽었다고 지우면 영영 잃는다).
    let mut settlements: Vec<Settlement> = Vec::new();
    let mut consumed: Vec<PathBuf> = Vec::new();
    for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
        if !e.file_name().to_string_lossy().starts_with(&claim_prefix) {
            continue;
        }
        let Ok(raw) = fs::read_to_string(e.path()) else {
            continue; // 다음 폴링에 다시
        };
        // 못 읽는 JSON 도 바로 지우지 않는다(개발 68, 코덱스 1차) — MCP 가 가져가기 직전에 열어 둔 파일에 아직
        // 쓰는 중이면 반쪽이 읽힌다. 남겨 두면 다 쓰인 뒤 다음 폴링에 읽힌다. 단 마지막 수정이 60초를 넘었는데도
        // 못 읽으면 쓰기가 끝내 멈춘 것이라 지운다(코덱스 2차 P2 — 안 지우면 1초마다 다시 읽으며 쌓인다).
        let Ok(batch) = serde_json::from_str::<Vec<Settlement>>(&raw) else {
            let stale = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > std::time::Duration::from_secs(60));
            if stale {
                let _ = fs::remove_file(e.path());
            }
            continue;
        };
        settlements.extend(batch);
        consumed.push(e.path());
    }
    (settlements, consumed)
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
    // 🔴 **읽기 전에 파일을 통째로 가져온다(rename)** (개발 66, 코덱스 1·2차). 예전엔 읽고 → 반영하고 → 지웠다.
    // 그 사이 MCP 가 새 정산을 덧붙이면 **읽지 않은 그 건까지 지웠고**, 그 내역은 영영 「정산 대기」로 남았다.
    // rename 은 원자적이다 — 가져간 뒤 MCP 가 쓰는 건 새 파일로 가서 다음 폴링에 반영된다.
    // (MCP 가 가져가기 전 목록을 읽고 가져간 뒤에 쓰면 옛 건이 새 파일에 한 번 더 실리는데, 이미 「signed」가
    // 아니라서 다시 매칭되지 않는다 — 두 번 반영되지 않는다.)
    // 잠금은 **가져오기 전에** 잡는다 — 두 폴링이 겹쳐도 가져오기·읽기·반영이 한 줄로 선다(2차 P1).
    let _g = history_guard();
    let (settlements, consumed) = claim_settlements(&path);
    if consumed.is_empty() {
        return 0; // 처리할 정산 없음 (대부분의 폴링)
    }
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
    let all_indices = indices.clone();
    let mut pending: Vec<&Settlement> = settlements.iter().collect();
    let mut applied = 0u32;
    // 내역 저장이 하나라도 실패하면 묶음을 지우지 않는다(개발 68, 코덱스 1차) — 지우면 그 「signed」 는 영영
    // 정산 대기로 남는다. 남긴 묶음은 다음 폴링에 다시 반영된다(저장된 건은 이미 signed 가 아니라 다시 안 맞는다).
    let mut write_failed = false;
    for index in indices {
        if pending.is_empty() {
            break;
        }
        let Ok(hp) = history_path_for(index) else {
            continue;
        };
        let mut list = read_history_at(&hp);
        if list.is_empty() {
            // 파일이 있는데 못 읽은 것이면 묶음을 남긴다(코덱스 개발 69 1차) — 빈 목록으로 보고 넘어가면 아래에서
            // 묶음을 지워, 그 계정의 「signed」 가 파일이 되살아나도 영영 정산 대기로 남는다.
            if history_unreadable(&hp) {
                write_failed = true;
            }
            continue;
        }
        let before = pending.len();
        pending.retain(|s| !apply_settlement(&mut list, s));
        let hit = (before - pending.len()) as u32;
        if hit > 0 {
            applied += hit;
            if write_json(hp, &list).is_err() {
                write_failed = true;
            }
        }
    }
    // 본 파일에서 못 찾은 정산은 **보관 파일**에서 찾는다(개발 70, 코덱스 1차 P1) — 정산이 늦게 오는 사이 시도가 200건을
    // 넘게 쌓이면 그 「signed」 는 보관 파일로 밀려나 있다. 드문 경로라 보관 파일을 통째로 다시 써도 된다.
    if !pending.is_empty() {
        for index in all_indices {
            if pending.is_empty() {
                break;
            }
            let Ok(hp) = history_path_for(index) else {
                continue;
            };
            match settle_in_archive(&crate::policy::history_archive_path(&hp), &mut pending) {
                Ok(hit) => applied += hit,
                Err(_) => write_failed = true,
            }
        }
    }
    if write_failed {
        return applied;
    }
    // 읽은 묶음은 지운다 — 매칭 실패분은 버린다(예전과 같다).
    for p in consumed {
        let _ = fs::remove_file(p);
    }
    applied
}

/// 보관 파일에서 정산을 찾아 반영한다 — 맞은 게 있으면 파일을 통째로 원자 교체한다. 반환 = 반영 건수.
/// 없는 파일은 0. 못 읽는 줄은 그대로 둔다(버리면 다시 쓸 때 사라진다).
fn settle_in_archive(path: &PathBuf, pending: &mut Vec<&Settlement>) -> Result<u32, String> {
    let raw = match fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.to_string()),
    };
    // 줄마다 (원문, 풀린 기록). 고친 줄만 다시 직렬화한다.
    let mut rows: Vec<(String, Option<HistoryEntry>)> = raw
        .lines()
        .map(|l| (l.to_string(), serde_json::from_str(l).ok()))
        .collect();
    let mut hit = 0u32;
    pending.retain(|s| {
        for (line, entry) in rows.iter_mut() {
            let Some(e) = entry else { continue };
            if apply_settlement(std::slice::from_mut(e), s) {
                if let Ok(new_line) = serde_json::to_string(e) {
                    *line = new_line;
                }
                hit += 1;
                return false;
            }
        }
        true
    });
    if hit > 0 {
        let mut body = String::with_capacity(raw.len());
        for (line, _) in &rows {
            body.push_str(line);
            body.push('\n');
        }
        crate::store::write_atomic(path, body.as_bytes())?;
    }
    Ok(hit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 🔴 정산 가져오기 (개발 66, 코덱스 1·2차) — 가져간 뒤 새로 쓰인 정산은 다음 번에 잡히고, 같은 순간에
    /// 두 번 가져가도 겹치지 않으며, 못 읽은 묶음은 지우지 않는다.
    #[test]
    fn settlements_are_claimed_without_loss() {
        let dir = std::env::temp_dir().join(format!("kura-settle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x402_settlements.json");
        let one = |n: &str| format!(r#"[{{"nonce":"{n}","tx":"0x1","success":true}}]"#);

        fs::write(&path, one("A")).unwrap();
        let (s1, c1) = claim_settlements(&path);
        assert_eq!(s1.len(), 1);
        // 같은 초에 새 정산 B 가 쓰이고 또 가져간다 — A 의 묶음을 덮지 않는다(아직 안 지웠어도).
        fs::write(&path, one("B")).unwrap();
        let (s2, _) = claim_settlements(&path);
        let nonces: Vec<_> = s2.iter().map(|s| s.nonce.as_str()).collect();
        assert!(nonces.contains(&"A") && nonces.contains(&"B"), "{nonces:?}");
        for p in c1 {
            let _ = fs::remove_file(p);
        }
        // 못 읽는 묶음(여기선 같은 접두어의 디렉터리)은 소비 목록에 안 들어간다 = 지우지 않는다.
        let stuck = dir.join(".x402_settlements.json.claimed.0.stuck");
        fs::create_dir_all(&stuck).unwrap();
        let (_, c3) = claim_settlements(&path);
        assert!(!c3.contains(&stuck));
        let _ = fs::remove_dir_all(&dir);
    }

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

    fn temp_hot(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kura-hist-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("history-8453.json")
    }

    fn tags(list: &[HistoryEntry]) -> Vec<String> {
        list.iter().map(|e| e.detail.clone()).collect()
    }

    /// 🔴 개발 70: 200건을 넘친 기록은 지워지지 않고 보관 파일로 간다 — 본 파일은 최신 cap 건, 나머지는 보관 파일에
    /// 오래된 순으로, 둘을 이어 읽으면 빠짐없이 최신순.
    #[test]
    fn overflow_moves_to_archive_without_loss() {
        let hot = temp_hot("overflow");
        for i in 0..7 {
            record_at(&hot, entry(&i.to_string()), 3).unwrap();
        }
        assert_eq!(tags(&read_history_at(&hot)), ["6", "5", "4"]);
        let raw = fs::read_to_string(crate::policy::history_archive_path(&hot)).unwrap();
        assert_eq!(raw.lines().count(), 4, "{raw}");
        assert_eq!(
            tags(&crate::policy::read_sent_history(&hot, 100)),
            ["6", "5", "4", "3", "2", "1", "0"]
        );
        // 본 파일로 충분하면 딱 그만큼, 모자라면 보관 파일에서 이어서.
        assert_eq!(tags(&crate::policy::read_sent_history(&hot, 2)), ["6", "5"]);
        assert_eq!(
            tags(&crate::policy::read_sent_history(&hot, 5)),
            ["6", "5", "4", "3", "2"]
        );
        let _ = fs::remove_dir_all(hot.parent().unwrap());
    }

    /// 덧붙인 뒤 본 파일을 쓰기 전에 죽은 경우 — 같은 기록이 본 파일 끝과 보관 파일 끝에 다 있다. 다음 번에 또
    /// 밀려나도 두 번 적히지 않는다. 쓰다 죽은 반쪽 줄은 건너뛰고, 그 뒤에 붙인 줄은 살아 있다.
    #[test]
    fn crash_between_append_and_write_does_not_duplicate() {
        let hot = temp_hot("crash");
        for i in 0..4 {
            record_at(&hot, entry(&i.to_string()), 3).unwrap();
        }
        // 본 파일 = [3,2,1], 보관 = [0]. 「1」 을 보관 파일에 덧붙이고 죽었다고 치자.
        let archive = crate::policy::history_archive_path(&hot);
        let mut raw = fs::read_to_string(&archive).unwrap();
        raw.push_str(&serde_json::to_string(&entry("1")).unwrap());
        raw.push('\n');
        fs::write(&archive, &raw).unwrap();
        record_at(&hot, entry("4"), 3).unwrap();
        assert_eq!(
            tags(&crate::policy::read_sent_history(&hot, 100)),
            ["4", "3", "2", "1", "0"]
        );
        // 반쪽 줄(개행 없이 끝남) 뒤에 덧붙여도 새 줄은 멀쩡하다.
        let mut raw = fs::read_to_string(&archive).unwrap();
        raw.push_str(r#"{"ts":0,"tok"#);
        fs::write(&archive, &raw).unwrap();
        record_at(&hot, entry("5"), 3).unwrap();
        assert_eq!(
            tags(&crate::policy::read_sent_history(&hot, 100)),
            ["5", "4", "3", "2", "1", "0"]
        );
        let _ = fs::remove_dir_all(hot.parent().unwrap());
    }

    /// 🔴 코덱스 개발 70 2차 P1: 여러 건을 한꺼번에 옮기다 죽어도(본 파일이 cap 을 넘겨 있던 경우) 겹친 구간 전체를
    /// 알아보고 새 것만 붙인다. 한글 사유가 있어 끝 읽기가 글자 중간에서 잘려도 같다.
    #[test]
    fn multi_evict_crash_does_not_duplicate() {
        // 덧대는 바이트를 1·2·3 으로 — 끝 읽기의 자른 자리가 3바이트 글자의 세 위상을 다 밟는다(적어도 한 번은 글자 중간).
        for pad in ["x", "xx", "xxx"] {
            let hot = temp_hot(&format!("multi{}", pad.len()));
            let e = |t: &str| entry(&format!("{t} 한글 사유"));
            // 본 파일이 cap(2)을 넘겨 5건 = 보관에 실패하던 뒤.
            let list: Vec<HistoryEntry> = ["4", "3", "2", "1", "0"].iter().map(|t| e(t)).collect();
            write_json(hot.clone(), &list).unwrap();
            // 맨 앞에 256KB 넘는 한글 기록(뒤에 덧대 JSON 이 아니게 = 읽을 땐 걸러진다), 그 뒤
            // 「0·1·2」 를 보관 파일에 덧붙이고 본 파일을 쓰기 전에 죽었다.
            let archive = crate::policy::history_archive_path(&hot);
            let mut raw =
                serde_json::to_string(&entry(&format!("f {}", "가".repeat(90_000)))).unwrap();
            raw.push_str(pad);
            raw.push('\n');
            for t in ["0", "1", "2"] {
                raw.push_str(&serde_json::to_string(&e(t)).unwrap());
                raw.push('\n');
            }
            fs::write(&archive, raw).unwrap();
            record_at(&hot, e("5"), 2).unwrap();
            let got: Vec<String> = crate::policy::read_sent_history(&hot, 100)
                .iter()
                .map(|x| x.detail.split(' ').next().unwrap().to_string())
                .collect();
            assert_eq!(got, ["5", "4", "3", "2", "1", "0"], "pad {pad}");
            let _ = fs::remove_dir_all(hot.parent().unwrap());
        }
    }

    /// 🔴 개발 70: 깨진 본 파일을 빈 목록으로 읽고 덮어쓰지 않는다 — 옆으로 치우고 새로 시작.
    #[test]
    fn broken_history_is_set_aside_not_overwritten() {
        let hot = temp_hot("broken");
        fs::write(&hot, "[{\"ts\":1,").unwrap();
        record_at(&hot, entry("new"), 3).unwrap();
        assert_eq!(tags(&read_history_at(&hot)), ["new"]);
        let aside: Vec<_> = fs::read_dir(hot.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".broken."))
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(fs::read_to_string(aside[0].path()).unwrap(), "[{\"ts\":1,");
        let _ = fs::remove_dir_all(hot.parent().unwrap());
    }

    /// 🔴 개발 70(코덱스 1차 P1): 정산이 오기 전에 「signed」 가 보관 파일로 밀려났어도 정산이 반영된다.
    #[test]
    fn settlement_reaches_archived_signed_entry() {
        let hot = temp_hot("settle-archive");
        let mut signed = entry("0xnonce");
        signed.status = "signed".into();
        record_at(&hot, signed, 2).unwrap();
        for i in 0..3 {
            record_at(&hot, entry(&i.to_string()), 2).unwrap();
        }
        let archive = crate::policy::history_archive_path(&hot);
        let s = Settlement {
            nonce: "0xnonce".into(),
            tx: "0xsettle".into(),
            success: true,
        };
        let mut pending = vec![&s];
        assert_eq!(settle_in_archive(&archive, &mut pending).unwrap(), 1);
        assert!(pending.is_empty());
        let all = crate::policy::read_sent_history(&hot, 100);
        let got = all.iter().find(|e| e.detail == "0xnonce").unwrap();
        assert_eq!(
            (got.status.as_str(), got.settle_tx.as_str()),
            ("settled", "0xsettle")
        );
        assert_eq!(all.len(), 4);
        // 없는 보관 파일은 0, 에러 아님.
        let none = hot.with_file_name("nope.archive.jsonl");
        assert_eq!(settle_in_archive(&none, &mut vec![&s]).unwrap(), 0);
        let _ = fs::remove_dir_all(hot.parent().unwrap());
    }

    /// 보관 파일에 못 쓰면 자르지 않는다 — 본 파일이 cap 을 넘길 뿐 잃지 않는다.
    #[test]
    fn archive_failure_keeps_everything_in_hot() {
        let hot = temp_hot("nofail");
        // 보관 파일 자리에 디렉터리 = 열 수 없다.
        fs::create_dir_all(crate::policy::history_archive_path(&hot)).unwrap();
        for i in 0..5 {
            record_at(&hot, entry(&i.to_string()), 3).unwrap();
        }
        assert_eq!(tags(&read_history_at(&hot)), ["4", "3", "2", "1", "0"]);
        let _ = fs::remove_dir_all(hot.parent().unwrap());
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

    // 없는 내역 파일은 「못 읽음」이 아니다 — 깨진 파일만(개발 69, 코덱스 1차: 정산 묶음을 남길지 가른다).
    #[test]
    fn history_unreadable_only_when_present_and_broken() {
        let dir = std::env::temp_dir().join(format!("kura-hist-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("history.json");
        assert!(!history_unreadable(&p));
        fs::write(&p, "[]").unwrap();
        assert!(!history_unreadable(&p));
        fs::write(&p, "[{ 반쪽").unwrap();
        assert!(history_unreadable(&p));
        let _ = fs::remove_dir_all(&dir);
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
