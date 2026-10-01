// 입금 기록 (개발 69) — 이 주소로 **들어온** 돈을 체인에서 찾아 내역에 남긴다.
//
// 여태 내역은 우리가 보낸 것(송금·서명 시도)뿐이었다. 들어온 돈은 잔액에만 반영되고 기록이 없어서,
// 「언제 누구에게서 얼마」를 볼 수 없었다(0.5.0 세금용 내보내기의 선행 조건).
//
// **데이터 출처는 이미 쓰는 RPC 하나뿐이다.** 익스플로러 API(Blockscout 등)는 과거를 한 번에 주지만
// 제3자에게 주소를 한 번 더 알리고, 체인마다 있다 없다 한다(개발 69 실측: Arc 메인넷 익스플로러 API = 403).
// 공개 RPC 로 되는 두 갈래:
//
// ① **토큰 입금 = Transfer 로그** (`eth_getLogs`, to = 나). 공개 RPC 의 조회 범위 상한(개발 69 실측):
//    Base 메인넷 2,000 블록 · Base Sepolia 1,000 · Arc 9,999. 넘으면 에러 → 청크를 반으로 줄여 다시.
//    - Base: USDC 컨트랙트의 로그(6dp).
//    - 🔴 Arc: USDC 가 네이티브라 **`0xff…fe` 미러의 로그(18dp)만** 본다. 실측(개발 69, Arc 테스트넷):
//      ERC-20 transfer 는 로그를 둘 낸다(`0x3600…` 6dp + 미러 18dp) · **네이티브 value 송금은 미러 하나뿐**.
//      `0x3600…` 만 보면 거래소 출금 같은 네이티브 입금을 통째로 놓치고, 둘 다 보면 같은 돈을 두 번 센다.
// ② **ETH 입금(Base 가스용) = 로그가 없다.** 대신 논스로 가른 잔액 이분 탐색:
//    블록 a·b 사이에 내 논스가 그대로면 내가 보낸 거래가 없다 → 그 사이 ETH 는 **늘기만** 한다.
//    그러니 잔액까지 같으면 입금 0 이 확정이고, 다르면 반으로 쪼개 한 블록까지 내려간다. 그 블록의 거래 중
//    to = 나 인 것이 입금이다(해시·보낸 사람까지 나온다). 컨트랙트가 보낸 내부 전송은 거래 목록에 없어서
//    금액만 남는다. 공개 Base RPC 가 90일 전 상태까지 답하는 것을 확인했다(개발 69: mainnet·sepolia 둘 다).
//
// 범위: 활성 체인 × 활성 계정. 앞으로(새 블록)는 계속, 뒤로는 **90일**까지(사장 확정 09-26) 한 번 채운다.
// 어디까지 봤는지는 `deposit_scan*.json` 에 남겨 앱을 껐다 켜도 이어 간다. 기록은 키로 중복을 막아서
// 같은 구간을 두 번 훑어도 두 번 적히지 않는다 — 그래서 커서는 **기록을 쓴 뒤에** 옮긴다.
//
// 알고 남기는 것(DEVLOG 개발 69):
// - 내가 거래를 낸 **바로 그 블록**에 컨트랙트가 보낸 ETH(내부 전송)는 못 잡는다 — 논스가 바뀐 블록에선
//   잔액 차이가 내 지출과 섞여서, 거래 목록에 보이는 직접 입금만 센다.
// - 0 원 Transfer 는 버린다(주소 오염 공격의 전형). 아주 작은 금액은 그대로 남긴다(x402 소액 수입일 수 있다).

use alloy::primitives::{address, utils::format_units, Address, B256, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::{Filter, Log};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::chain::{chain_by_id, with_pinned_chain, ChainConfig};
use crate::policy;
use crate::settings::effective_rpc;
use crate::store::{jigap_dir, write_json};

/// Arc 의 네이티브 USDC 미러 — 네이티브 이동마다 여기서 18dp Transfer 로그가 나온다(위 주석 ①).
const ARC_NATIVE_MIRROR: Address = address!("0xfffffffffffffffffffffffffffffffffffffffe");
/// keccak256("Transfer(address,address,uint256)").
const TRANSFER_TOPIC: B256 =
    alloy::primitives::b256!("0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef");

/// 뒤로 채우는 깊이(사장 확정, 개발 69).
const BACKFILL_SECS: u64 = 90 * 24 * 60 * 60;
/// 맨 끝 블록은 뒤집힐 수 있다 — 이만큼 뒤까지만 본다(Base 2초 × 3 = 6초. Arc 는 즉시 확정이라 무해).
const CONFIRMATIONS: u64 = 3;
/// ETH 이분 탐색 한 창의 최대 폭(블록). Base 2초 기준 하루. 창이 끝나야 커서가 옮겨진다.
const ETH_WINDOW: u64 = 43_200;
/// 한 번 돌 때 쓰는 시간 상한 — 넘으면 다음 차례로 넘긴다(커서는 끝난 청크까지만 옮긴다).
const TICK_BUDGET: Duration = Duration::from_secs(20);
/// RPC 호출 사이 최소 간격의 바닥값. 🔴 고정값으로는 안 된다(개발 69 실측): Arc 테스트넷 공개 RPC 가
/// 80ms 간격의 `eth_getLogs` 에 2초 만에 429 를 냈는데, `eth_blockNumber` 는 100ms 간격 60번에도 멀쩡했다 —
/// 제한이 호출 수가 아니라 **호출 무게**에 걸린다. Base 메인넷은 100ms 간격 `eth_getLogs` 30번에 429 가 0건.
/// 그래서 간격을 **체인마다** 배운다: 429 를 받으면 두 배(최대 2초), 20번 연달아 무사하면 3/4 로 줄인다.
/// 🔴 한때 이 값이 체인 공용이었다 — Arc 에서 2초로 늘어난 간격이 Base 로 옮겨 가 청크마다 ~4초가 걸렸다.
const CALL_GAP: Duration = Duration::from_millis(80);
const MAX_GAP: Duration = Duration::from_secs(2);
/// 체인 ID → 배운 간격(ms). 앱이 떠 있는 동안 유지한다.
static PACE_MS: std::sync::LazyLock<std::sync::Mutex<HashMap<u64, u64>>> =
    std::sync::LazyLock::new(Default::default);

fn learned_gap(chain_id: u64) -> Duration {
    let ms = PACE_MS
        .lock()
        .map(|m| m.get(&chain_id).copied())
        .ok()
        .flatten();
    ms.map(Duration::from_millis)
        .unwrap_or(CALL_GAP)
        .clamp(CALL_GAP, MAX_GAP)
}

fn remember_gap(chain_id: u64, gap: Duration) {
    if let Ok(mut m) = PACE_MS.lock() {
        m.insert(chain_id, gap.as_millis() as u64);
    }
}

/// 초당 제한에 걸렸는가 — HTTP 429 거나 공급자 문구.
fn is_rate_limited(e: &str) -> bool {
    let e = e.to_ascii_lowercase();
    e.contains("429") || e.contains("rate limit") || e.contains("too many requests")
}

// 입금 기록엔 상한이 없다(개발 70, 코덱스 1차 P1). 한때 5,000건에서 잘랐는데, 커서는 그 구간을 이미 지나 있어
// 잘린 입금은 다시 찾지도 않았다 = 조용한 유실. 한 건이 250바이트 안팎이라 5만 건이어도 12MB 다. 0 원 잡음은 애초에
// 안 적는다(`deposit_from_log`). 소액 잡음 공격은 건마다 수수료가 드는 일이라 그 값으로 막는다.

/// 들어온 돈 1건 — 형식의 정본은 `policy::Deposit`(MCP·CLI 가 같은 타입으로 읽는다).
pub(crate) use crate::policy::Deposit;

/// 어디까지 훑었는지 — 구간 (low, high] 을 다 봤다는 뜻.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
struct Span {
    low: u64,
    high: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct ScanState {
    /// 이 커서가 누구의 것인지 — 파일 이름은 계정 번호로 가르지만 주소로 한 번 더 확인한다.
    address: String,
    /// 뒤로 채우는 바닥 블록(처음 만들 때 90일 전으로 정한다).
    floor: u64,
    /// 토큰 로그 커서.
    logs: Span,
    /// ETH 이분 탐색 커서(네이티브가 USDC 인 체인은 안 쓴다).
    eth: Span,
}

fn deposits_path(chain_id: u64, index: u32) -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join(policy::account_file_name(
        &policy::chain_file_name(chain_id, "deposits"),
        index,
    )))
}

fn scan_path(chain_id: u64, index: u32) -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join(policy::account_file_name(
        &policy::chain_file_name(chain_id, "deposit_scan"),
        index,
    )))
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &PathBuf) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 활성 체인·계정의 입금 기록(최신순). 주인 주소가 활성 계정과 다르면 빈 목록(`policy::deposits_of`).
pub(crate) fn read_deposits() -> Vec<Deposit> {
    let chain_id = crate::chain::active_chain().chain_id;
    let Ok(account) = crate::wallet::active_account() else {
        return Vec::new();
    };
    deposits_path(chain_id, account.index)
        .map(|p| policy::deposits_of(&p, &account.address))
        .unwrap_or_default()
}

/// 새 입금을 기존 목록에 합친다(순수 함수) — 키가 같은 건 버리고, 블록 시각 최신순.
/// 반환 = (합친 목록, 새로 들어간 개수).
fn merge_deposits(mut list: Vec<Deposit>, found: Vec<Deposit>) -> (Vec<Deposit>, usize) {
    let mut seen: HashSet<String> = list.iter().map(|d| d.key.clone()).collect();
    let mut added = 0;
    for d in found {
        if seen.insert(d.key.clone()) {
            list.push(d);
            added += 1;
        }
    }
    // 같은 시각이면 블록·키로 — 순서가 매번 같아야 화면이 안 흔들린다.
    list.sort_by(|a, b| {
        b.ts.cmp(&a.ts)
            .then(b.block.cmp(&a.block))
            .then(b.key.cmp(&a.key))
    });
    (list, added)
}

/// 주소별 보관 파일 — `deposits-8453.json` → `deposits-8453.0xabc….json`.
fn aside_path(dp: &std::path::Path, address: &str) -> PathBuf {
    dp.with_extension(format!("{}.json", address.to_ascii_lowercase()))
}

/// 기록 파일의 주인을 `address` 로 맞추고 그 주소의 기록을 돌려준다.
/// - 주인이 같으면 그대로.
/// - 다른 주소의 기록(지갑을 지우고 다른 시드를 가져왔다)이면 **그 주소의 보관 파일에 합쳐** 옆으로 치운다
///   (🔴 코덱스 개발 69 2차: 이름만 바꿔 치우면 A→B→A→B 에서 A 의 옛 보관분을 덮어써 잃었다).
/// - 이 주소의 보관 파일이 있으면 되찾는다(A→B→A 에서 A 의 90일 넘은 기록이 돌아온다).
///
/// 🔴 파일을 못 읽으면 **에러로 멈춘다** — 빈 목록으로 읽고 합쳐 쓰면 옛 기록이 지워지고, 커서는 그 구간을
/// 이미 지나 다시 찾지도 않는다(코덱스 개발 69 1차).
fn claim_owner(dp: &PathBuf, address: &str) -> Result<Vec<Deposit>, String> {
    let mut items = match policy::read_deposit_log(dp)? {
        Some(log) if log.address.eq_ignore_ascii_case(address) => return Ok(log.items),
        Some(log) => {
            let aside = aside_path(dp, &log.address);
            let kept = policy::read_deposit_log(&aside)?
                .map(|l| l.items)
                .unwrap_or_default();
            let (merged, _) = merge_deposits(kept, log.items);
            let moved = policy::DepositLog {
                address: log.address,
                items: merged,
            };
            write_json(aside, &moved)?;
            std::fs::remove_file(dp).map_err(|e| e.to_string())?;
            Vec::new()
        }
        None => Vec::new(),
    };
    let mine = aside_path(dp, address);
    if let Some(back) = policy::read_deposit_log(&mine)? {
        items = merge_deposits(items, back.items).0;
        let log = policy::DepositLog {
            address: address.to_string(),
            items: items.clone(),
        };
        // 본 파일에 먼저 쓰고 보관 파일을 지운다 — 사이에 죽어도 기록은 두 곳 중 하나엔 있다(키가 중복을 막는다).
        write_json(dp.clone(), &log)?;
        std::fs::remove_file(&mine).map_err(|e| e.to_string())?;
    }
    Ok(items)
}

/// 찾은 입금을 기록 파일에 합쳐 쓴다 — 새로 들어간 수를 돌려준다. 주인 맞추기·못 읽으면 멈추기는 `claim_owner`.
fn store_found(dp: &PathBuf, address: &str, found: Vec<Deposit>) -> Result<usize, String> {
    if found.is_empty() {
        return Ok(0);
    }
    let items = claim_owner(dp, address)?;
    let (items, n) = merge_deposits(items, found);
    if n > 0 {
        let log = policy::DepositLog {
            address: address.to_string(),
            items,
        };
        write_json(dp.clone(), &log)?;
    }
    Ok(n)
}

/// 십진 금액 — 끝의 0 과 점을 뗀다("1.500000" → "1.5", "2.000000" → "2").
fn fmt_amount(v: U256, decimals: u8) -> String {
    let s = format_units(v, decimals).unwrap_or_else(|_| v.to_string());
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

fn topic_addr(t: &B256) -> Address {
    Address::from_slice(&t[12..])
}

/// Transfer 로그 1건 → 입금(순수 함수). 0 원·내가 나에게·모양이 안 맞는 로그는 None.
fn deposit_from_log(log: &Log, me: Address, decimals: u8, ts: u64) -> Option<Deposit> {
    let topics = log.topics();
    if topics.len() != 3 || topics[0] != TRANSFER_TOPIC || topic_addr(&topics[2]) != me {
        return None;
    }
    let from = topic_addr(&topics[1]);
    if from == me {
        return None;
    }
    let data = log.data().data.as_ref();
    if data.len() != 32 {
        return None;
    }
    let value = U256::from_be_slice(data);
    if value.is_zero() {
        return None; // 주소 오염 공격의 전형(0 원 Transfer) — 기록하면 그 주소가 화면에 올라온다.
    }
    let tx = log.transaction_hash?;
    let block = log.block_number?;
    Some(Deposit {
        ts,
        token: "USDC".into(),
        from: from.to_checksum(None),
        amount: fmt_amount(value, decimals),
        tx: format!("{tx:#x}"),
        block,
        key: format!("{tx:#x}:{}", log.log_index.unwrap_or(0)),
    })
}

// ─────────────────────────────── ETH 이분 탐색 ───────────────────────────────

/// 한 블록 안에서 나에게 직접 온 ETH 거래(성공한 것만).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DirectIn {
    pub tx: String,
    pub from: Address,
    pub value: U256,
}

/// 이분 탐색이 체인에 묻는 것 — 테스트에서 가짜 체인으로 바꿔 끼운다.
pub(crate) trait EthView {
    /// 블록 n 끝의 (잔액, 논스).
    async fn state(&mut self, n: u64) -> Result<(U256, u64), String>;
    /// 블록 n 의 시각과, 그 블록에서 나에게 직접 온 성공한 ETH 거래.
    async fn direct_in(&mut self, n: u64) -> Result<(u64, Vec<DirectIn>), String>;
}

/// (a, b] 에서 ETH 입금을 찾는다. a·b 의 상태는 호출 쪽이 알고 있다(창 끝을 이웃 창과 나눠 쓴다).
pub(crate) async fn eth_deposits_between<V: EthView>(
    view: &mut V,
    a: u64,
    sa: (U256, u64),
    b: u64,
    sb: (U256, u64),
) -> Result<Vec<Deposit>, String> {
    let mut out = Vec::new();
    // 재귀 대신 손으로 쌓는다(async 재귀는 박싱이 필요하다).
    let mut stack = vec![(a, sa, b, sb)];
    while let Some((a, sa, b, sb)) = stack.pop() {
        if b <= a {
            continue;
        }
        let same_nonce = sa.1 == sb.1;
        // 논스가 그대로 = 내가 보낸 게 없다 = 잔액은 늘기만 한다 → 같으면 입금 0 확정.
        if same_nonce && sa.0 == sb.0 {
            continue;
        }
        if b == a + 1 {
            let (ts, direct) = view.direct_in(b).await?;
            let mut direct_sum = U256::ZERO;
            for d in &direct {
                direct_sum += d.value;
                out.push(Deposit {
                    ts,
                    token: "ETH".into(),
                    from: d.from.to_checksum(None),
                    amount: fmt_amount(d.value, 18),
                    tx: d.tx.clone(),
                    block: b,
                    key: format!("eth:{}", d.tx),
                });
            }
            // 논스가 그대로인데 늘어난 몫이 직접 입금보다 크면, 나머지는 컨트랙트가 보낸 내부 전송이다.
            // (논스가 바뀐 블록에선 차이가 내 지출과 섞여 있어 가를 수 없다 — 머리 주석 「알고 남기는 것」.)
            if same_nonce && sb.0 > sa.0 {
                let rest = sb.0 - sa.0;
                if rest > direct_sum {
                    out.push(Deposit {
                        ts,
                        token: "ETH".into(),
                        from: String::new(),
                        amount: fmt_amount(rest - direct_sum, 18),
                        tx: String::new(),
                        block: b,
                        key: format!("eth-int:{b}"),
                    });
                }
            }
            continue;
        }
        let mid = a + (b - a) / 2;
        let sm = view.state(mid).await?;
        stack.push((mid, sm, b, sb));
        stack.push((a, sa, mid, sm));
    }
    Ok(out)
}

// ─────────────────────────────── RPC 붙이기 ───────────────────────────────

/// 한 번 도는 동안의 RPC — 호출 간격을 지키고, 블록 시각을 모아 둔다.
struct Rpc<P> {
    provider: P,
    me: Address,
    chain_id: u64,
    /// 이 체인에서 지금 지키는 호출 간격과, 그 뒤로 연달아 무사한 호출 수.
    gap: Duration,
    calm: u32,
    last_call: Option<Instant>,
    block_ts: HashMap<u64, u64>,
    /// 블록별 (잔액, 논스) — 이웃한 두 창이 끝점을 나눠 쓴다.
    states: HashMap<u64, (U256, u64)>,
}

impl<P: Provider> Rpc<P> {
    /// 제한에 걸렸으면 간격을 두 배로 늘리고 조금 쉰다 — true 면 같은 일을 다시 하면 된다.
    async fn back_off(&mut self, e: &str) -> bool {
        if !is_rate_limited(e) {
            return false;
        }
        self.gap = (self.gap * 2).min(MAX_GAP);
        self.calm = 0;
        remember_gap(self.chain_id, self.gap);
        tokio::time::sleep(Duration::from_secs(2)).await;
        true
    }

    async fn pace(&mut self) {
        // 20번 연달아 제한 없이 지나갔으면 간격을 조금 줄인다(한 번 늘린 간격이 영영 안 줄지 않게).
        self.calm += 1;
        if self.calm >= 20 && self.gap > CALL_GAP {
            self.gap = (self.gap * 3 / 4).max(CALL_GAP);
            self.calm = 0;
            remember_gap(self.chain_id, self.gap);
        }
        if let Some(t) = self.last_call {
            let left = self.gap.saturating_sub(t.elapsed());
            if !left.is_zero() {
                tokio::time::sleep(left).await;
            }
        }
        self.last_call = Some(Instant::now());
    }

    async fn block_json(&mut self, n: u64, full: bool) -> Result<serde_json::Value, String> {
        self.pace().await;
        // 원시 JSON 으로 받는다 — Base 블록의 첫 거래는 OP 예치 거래(type 0x7E)라 이더리움 타입으로는 안 풀린다.
        let v: serde_json::Value = self
            .provider
            .raw_request("eth_getBlockByNumber".into(), (format!("{n:#x}"), full))
            .await
            .map_err(|e| e.to_string())?;
        if let Some(ts) = v.get("timestamp").and_then(hex_u64) {
            self.block_ts.insert(n, ts);
        }
        Ok(v)
    }

    async fn block_ts(&mut self, n: u64) -> Result<u64, String> {
        if let Some(t) = self.block_ts.get(&n) {
            return Ok(*t);
        }
        let v = self.block_json(n, false).await?;
        v.get("timestamp")
            .and_then(hex_u64)
            .ok_or_else(|| format!("블록 {n} 시각 없음"))
    }

    async fn logs(&mut self, token: Address, from: u64, to: u64) -> Result<Vec<Log>, String> {
        self.pace().await;
        let filter = Filter::new()
            .address(token)
            .event_signature(TRANSFER_TOPIC)
            .topic2(self.me.into_word())
            .from_block(from)
            .to_block(to);
        self.provider
            .get_logs(&filter)
            .await
            .map_err(|e| e.to_string())
    }
}

fn hex_u64(v: &serde_json::Value) -> Option<u64> {
    u64::from_str_radix(v.as_str()?.trim_start_matches("0x"), 16).ok()
}

fn hex_u256(v: &serde_json::Value) -> Option<U256> {
    U256::from_str_radix(v.as_str()?.trim_start_matches("0x"), 16).ok()
}

impl<P: Provider> EthView for Rpc<P> {
    async fn state(&mut self, n: u64) -> Result<(U256, u64), String> {
        if let Some(s) = self.states.get(&n) {
            return Ok(*s);
        }
        self.pace().await;
        let bal = self
            .provider
            .get_balance(self.me)
            .block_id(n.into())
            .await
            .map_err(|e| e.to_string())?;
        self.pace().await;
        let nonce = self
            .provider
            .get_transaction_count(self.me)
            .block_id(n.into())
            .await
            .map_err(|e| e.to_string())?;
        self.states.insert(n, (bal, nonce));
        Ok((bal, nonce))
    }

    async fn direct_in(&mut self, n: u64) -> Result<(u64, Vec<DirectIn>), String> {
        let v = self.block_json(n, true).await?;
        let ts = v
            .get("timestamp")
            .and_then(hex_u64)
            .ok_or_else(|| format!("블록 {n} 시각 없음"))?;
        let mut out = Vec::new();
        // 응답이 모자라면 「입금 없음」이 아니라 오류다(개발 73, 코덱스 1차 P2) — 빈 것으로 넘기면 커서가 지나가 그 입금을 영영 못 찾는다.
        let txs = v
            .get("transactions")
            .and_then(|t| t.as_array())
            .ok_or_else(|| format!("블록 {n} 거래 목록 없음"))?;
        for tx in txs {
            let to = tx
                .get("to")
                .and_then(|t| t.as_str())
                .and_then(|s| s.parse::<Address>().ok());
            let from = tx
                .get("from")
                .and_then(|t| t.as_str())
                .and_then(|s| s.parse::<Address>().ok());
            let value = tx.get("value").and_then(hex_u256);
            let hash = tx
                .get("hash")
                .and_then(|h| h.as_str())
                .unwrap_or_default()
                .to_string();
            let (Some(to), Some(from)) = (to, from) else {
                continue;
            };
            if to != self.me || from == self.me {
                continue;
            }
            let Some(value) = value else {
                return Err(format!("블록 {n} 거래 금액을 못 읽음"));
            };
            if value.is_zero() {
                continue;
            }
            if hash.is_empty() {
                return Err(format!("블록 {n} 거래 해시 없음"));
            }
            // 실패한 거래의 value 는 옮겨지지 않는다 — 영수증 상태로 거른다.
            self.pace().await;
            let r: serde_json::Value = self
                .provider
                .raw_request("eth_getTransactionReceipt".into(), (hash.clone(),))
                .await
                .map_err(|e| e.to_string())?;
            // 영수증이 없거나 상태 칸이 없으면 모름 — 실패로 치고 건너뛰지 않는다(다음 차례에 이 구간을 다시).
            match r.get("status").and_then(hex_u64) {
                Some(1) => {}
                Some(_) => continue, // 실패한 거래 — value 가 옮겨지지 않았다
                None => return Err(format!("거래 {hash} 영수증 상태를 모름")),
            }
            out.push(DirectIn {
                tx: hash,
                from,
                value,
            });
        }
        Ok((ts, out))
    }
}

/// 조회 범위 초과 에러인가 — 공급자마다 문구가 다르다(개발 69 실측:
/// Base 「eth_getLogs is limited to a 2,000 range」, Arc 「requested range too large」).
fn is_range_error(e: &str) -> bool {
    // 「limit」 만으로 잡으면 초당 제한(rate limit)을 범위 초과로 읽고 청크를 1 까지 깎는다.
    let e = e.to_ascii_lowercase();
    (e.contains("range") && !e.contains("rate")) || e.contains("too large")
}

/// 체인별 첫 청크 폭 — 실측한 공개 RPC 상한(위 머리 주석). 지정 RPC 가 더 좁으면 에러를 보고 줄인다.
fn initial_chunk(chain: &ChainConfig) -> u64 {
    match chain.chain_id {
        policy::BASE_MAINNET_ID => 2_000,
        policy::BASE_SEPOLIA_ID => 1_000,
        _ => 9_999,
    }
}

/// 90일 전 블록 — 끝 블록과 10만 블록 전의 시각으로 평균 블록 간격을 재서 거꾸로 센다(대략이면 된다).
async fn floor_block<P: Provider>(rpc: &mut Rpc<P>, tip: u64) -> Result<u64, String> {
    let back = tip.min(100_000);
    if back == 0 {
        return Ok(0);
    }
    let t1 = rpc.block_ts(tip).await?;
    let t0 = rpc.block_ts(tip - back).await?;
    let per_block = (t1.saturating_sub(t0)) as f64 / back as f64;
    if per_block <= 0.0 {
        return Ok(tip.saturating_sub(back));
    }
    let blocks = (BACKFILL_SECS as f64 / per_block) as u64;
    Ok(tip.saturating_sub(blocks))
}

/// 한 번 돈 결과.
#[derive(Default, Debug)]
pub(crate) struct TickResult {
    /// 새로 적힌 입금 수.
    pub added: usize,
    /// 90일 바닥까지 다 채웠는가 — 아니면 곧 다시 돈다.
    pub caught_up: bool,
}

/// 활성 체인·계정으로 한 번 훑는다. 도중에 설정이 바뀌어도 이번 차례는 시작할 때의 체인·계정으로 끝낸다.
pub(crate) async fn scan_once() -> Result<TickResult, String> {
    if crate::wallet::needs_setup() {
        return Ok(TickResult {
            caught_up: true,
            ..Default::default()
        });
    }
    let account = crate::wallet::active_account()?;
    let chain = crate::chain::active_chain();
    let url = effective_rpc();
    with_pinned_chain(
        chain.chain_id,
        scan_with(chain, account.index, account.address, url),
    )
    .await
}

async fn scan_with(
    chain: ChainConfig,
    index: u32,
    address: String,
    url: String,
) -> Result<TickResult, String> {
    let me: Address = address.parse().map_err(|e| format!("주소: {e}"))?;
    let provider = ProviderBuilder::new()
        .connect(&url)
        .await
        .map_err(|e| crate::settings::redact_urls(&e.to_string()))?;
    // 지정 RPC 가 다른 체인을 가리키면 남의 체인 입금을 이 체인 기록에 적게 된다 — 먼저 확인한다.
    let got = provider
        .get_chain_id()
        .await
        .map_err(|e| crate::settings::redact_urls(&e.to_string()))?;
    if chain_by_id(got).map(|c| c.chain_id) != Some(chain.chain_id) {
        return Err(format!("RPC 체인 {got} ≠ {}", chain.chain_id));
    }
    let mut rpc = Rpc {
        provider,
        me,
        chain_id: chain.chain_id,
        gap: learned_gap(chain.chain_id),
        calm: 0,
        last_call: None,
        block_ts: HashMap::new(),
        states: HashMap::new(),
    };
    let started = Instant::now();
    let dp = deposits_path(chain.chain_id, index)?;
    let sp = scan_path(chain.chain_id, index)?;

    rpc.pace().await;
    let head = rpc
        .provider
        .get_block_number()
        .await
        .map_err(|e| crate::settings::redact_urls(&e.to_string()))?;
    let tip = head.saturating_sub(CONFIRMATIONS);

    // 기록 파일의 주인을 **매 차례** 맞춘다 — 새 입금이 없어도 옛 주소의 기록이 화면에 남지 않고, 돌아온 주소는
    // 보관분을 바로 되찾게. 주인이 같으면 파일 하나 읽고 끝난다.
    // 🔴 한때 아래 「커서 새로 만들기」 안에서만 불렀다(코덱스 개발 69 3차 P2) — 주소를 바꾼 차례에 바닥 블록 찾기나
    // 커서 저장이 실패하고 곧장 원래 주소로 돌아오면, 커서는 이미 원래 주소의 것이라 다시 안 불려 보관분을 못 되찾았다.
    claim_owner(&dp, &address)?;
    let mut st: ScanState = read_json(&sp);
    if !st.address.eq_ignore_ascii_case(&address) || st.logs.high == 0 {
        // 처음이거나 다른 주소의 커서 — 지금 끝에서 시작해 90일 전까지 거꾸로.
        let floor = floor_block(&mut rpc, tip).await?;
        st = ScanState {
            address: address.clone(),
            floor,
            logs: Span {
                low: tip,
                high: tip,
            },
            eth: Span {
                low: tip,
                high: tip,
            },
        };
        write_json(sp.clone(), &st)?;
    }

    let token = if chain.native_is_usdc {
        ARC_NATIVE_MIRROR
    } else {
        chain.usdc_address
    };
    let decimals = if chain.native_is_usdc {
        18
    } else {
        chain.usdc_decimals
    };
    let mut chunk = initial_chunk(&chain);
    let mut added = 0usize;

    // 기록을 먼저 쓰고 커서를 옮긴다 — 그 사이에 죽으면 다음에 같은 구간을 다시 보고, 키가 중복을 막는다.
    // 기록을 못 쓰거나 못 읽으면 `?` 로 멈춘다 = 커서가 안 옮겨진다(`store_found`).
    let mut commit = |found: Vec<Deposit>, st: &ScanState| -> Result<(), String> {
        added += store_found(&dp, &address, found)?;
        write_json(sp.clone(), st)
    };

    // 앞으로(새 입금)를 두 갈래 다 먼저 끝내고, 그다음 뒤로 채운다 — 90일 채우기가 새 입금을 막지 않게.
    // 두 갈래는 **한 걸음씩 번갈아** 간다. 🔴 개발 69 실측: 갈래마다 따로 돌렸더니 로그 뒤로 채우기가 매번 시간
    // 예산을 다 써서, Base 메인넷에서 9분 동안 로그는 23만 블록을 거슬러 갔는데 ETH 커서는 한 칸도 안 움직였다.
    for forward_only in [true, false] {
        loop {
            if started.elapsed() > TICK_BUDGET {
                break;
            }
            let mut moved = false;

            // ── 토큰 로그 한 청크.
            let next = if st.logs.high < tip {
                Some((st.logs.high + 1, tip.min(st.logs.high + chunk), true))
            } else if !forward_only && st.logs.low > st.floor {
                Some((
                    st.floor.max(st.logs.low.saturating_sub(chunk)) + 1,
                    st.logs.low,
                    false,
                ))
            } else {
                None
            };
            if let Some((from, to, forward)) = next {
                moved = true;
                // None = 같은 청크를 다음 걸음에 다시(제한에 걸렸거나 범위를 줄였다).
                let found: Option<Vec<Deposit>> = match rpc.logs(token, from, to).await {
                    Err(e) if rpc.back_off(&e).await => None,
                    Err(e) if is_range_error(&e) && chunk > 1 => {
                        chunk = (chunk / 2).max(1);
                        None
                    }
                    Err(e) => return Err(crate::settings::redact_urls(&e)),
                    Ok(logs) => {
                        let mut found = Vec::new();
                        let mut ok = true;
                        for log in &logs {
                            let Some(bn) = log.block_number else { continue };
                            let ts = match log.block_timestamp {
                                Some(t) => t,
                                // 찾은 시각은 Rpc 가 기억해서, 다시 할 때 다시 안 묻는다.
                                None => match rpc.block_ts(bn).await {
                                    Ok(t) => t,
                                    Err(e) if rpc.back_off(&e).await => {
                                        ok = false;
                                        break;
                                    }
                                    Err(e) => return Err(crate::settings::redact_urls(&e)),
                                },
                            };
                            if let Some(d) = deposit_from_log(log, me, decimals, ts) {
                                found.push(d);
                            }
                        }
                        ok.then_some(found)
                    }
                };
                if let Some(found) = found {
                    if forward {
                        st.logs.high = to;
                    } else {
                        st.logs.low = from - 1;
                    }
                    commit(found, &st)?;
                }
            }

            // ── ETH(Base 만) 한 창. 창의 끝점·중간 상태는 Rpc 가 기억해서 이웃 창·다시 할 때 다시 안 묻는다.
            let next = if chain.native_is_usdc {
                None
            } else if st.eth.high < tip {
                Some((st.eth.high, tip.min(st.eth.high + ETH_WINDOW), true))
            } else if !forward_only && st.eth.low > st.floor {
                Some((
                    st.floor.max(st.eth.low.saturating_sub(ETH_WINDOW)),
                    st.eth.low,
                    false,
                ))
            } else {
                None
            };
            if let Some((a, b, forward)) = next {
                moved = true;
                let res = async {
                    let sa = rpc.state(a).await?;
                    let sb = rpc.state(b).await?;
                    eth_deposits_between(&mut rpc, a, sa, b, sb).await
                }
                .await;
                match res {
                    Ok(found) => {
                        if forward {
                            st.eth.high = b;
                        } else {
                            st.eth.low = a;
                        }
                        commit(found, &st)?;
                    }
                    Err(e) if rpc.back_off(&e).await => {}
                    Err(e) => return Err(crate::settings::redact_urls(&e)),
                }
            }

            if !moved {
                break;
            }
        }
    }

    let eth_done = chain.native_is_usdc || (st.eth.low <= st.floor && st.eth.high >= tip);
    Ok(TickResult {
        added,
        caught_up: st.logs.low <= st.floor && st.logs.high >= tip && eth_done,
    })
}

/// 앱이 떠 있는 동안 계속 돈다. 90일 채우기가 남았으면 곧바로 이어 돌고, 다 채웠으면 1분마다 새 블록만.
/// 새 입금이 적히면 화면에 `deposits-changed` 를 보낸다(내역·잔액을 다시 읽게).
pub(crate) fn spawn(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        use tauri::Emitter;
        let mut fails = 0u32;
        loop {
            let wait = match scan_once().await {
                Ok(r) => {
                    fails = 0;
                    if r.added > 0 {
                        let _ = app.emit("deposits-changed", r.added);
                    }
                    if r.caught_up {
                        Duration::from_secs(60)
                    } else {
                        Duration::from_secs(2)
                    }
                }
                Err(e) => {
                    // 네트워크가 없거나 RPC 가 거절 — 조용히 물러난다(1·2·4…분, 최대 10분).
                    fails = fails.saturating_add(1);
                    eprintln!("[deposits] {}", crate::settings::redact_urls(&e));
                    Duration::from_secs(60 * [1, 2, 4, 8, 10][(fails as usize - 1).min(4)])
                }
            };
            tokio::time::sleep(wait).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{Bytes, LogData};

    fn me() -> Address {
        address!("0x1111111111111111111111111111111111111111")
    }
    fn other() -> Address {
        address!("0x2222222222222222222222222222222222222222")
    }

    fn transfer_log(from: Address, to: Address, value: u64, tx: u8, idx: u64) -> Log {
        let inner = alloy::primitives::Log {
            address: ARC_NATIVE_MIRROR,
            data: LogData::new_unchecked(
                vec![TRANSFER_TOPIC, from.into_word(), to.into_word()],
                Bytes::from(U256::from(value).to_be_bytes::<32>().to_vec()),
            ),
        };
        Log {
            inner,
            block_number: Some(10),
            transaction_hash: Some(B256::repeat_byte(tx)),
            log_index: Some(idx),
            ..Default::default()
        }
    }

    // 로그 → 입금: 나에게 온 것만, 0 원·내가 나에게는 버린다. Arc 미러는 18dp 라 10^12 배로 온다.
    #[test]
    fn log_to_deposit_filters_and_scales() {
        let d =
            deposit_from_log(&transfer_log(other(), me(), 1_500_000, 1, 0), me(), 6, 7).unwrap();
        assert_eq!(d.amount, "1.5");
        assert_eq!(d.from, other().to_checksum(None));
        assert_eq!(d.ts, 7);
        assert_eq!(d.block, 10);
        // 🔴 Arc 미러: 0.01 USDC = 10^16 (18dp). 6 으로 풀면 1조 배가 된다.
        let arc = deposit_from_log(
            &transfer_log(other(), me(), 10_000_000_000_000_000, 2, 1),
            me(),
            18,
            0,
        )
        .unwrap();
        assert_eq!(arc.amount, "0.01");
        assert!(deposit_from_log(&transfer_log(other(), me(), 0, 3, 0), me(), 6, 0).is_none());
        assert!(deposit_from_log(&transfer_log(me(), me(), 5, 4, 0), me(), 6, 0).is_none());
        assert!(deposit_from_log(&transfer_log(other(), other(), 5, 5, 0), me(), 6, 0).is_none());
        // 같은 거래의 다른 로그는 다른 키.
        let a = deposit_from_log(&transfer_log(other(), me(), 5, 6, 0), me(), 6, 0).unwrap();
        let b = deposit_from_log(&transfer_log(other(), me(), 5, 6, 1), me(), 6, 0).unwrap();
        assert_ne!(a.key, b.key);
    }

    fn dep(key: &str, ts: u64) -> Deposit {
        Deposit {
            ts,
            token: "USDC".into(),
            from: String::new(),
            amount: "1".into(),
            tx: String::new(),
            block: ts,
            key: key.into(),
        }
    }

    // 같은 구간을 두 번 훑어도 두 번 안 적힌다(커서를 기록 뒤에 옮기는 근거) · 최신순 · 자르지 않는다(개발 70).
    #[test]
    fn merge_dedupes_sorts_and_keeps_all() {
        let (list, n) = merge_deposits(
            vec![dep("a", 5)],
            vec![dep("a", 5), dep("b", 9), dep("c", 1)],
        );
        assert_eq!(n, 2);
        assert_eq!(
            list.iter().map(|d| d.key.as_str()).collect::<Vec<_>>(),
            ["b", "a", "c"]
        );
        let (again, n2) = merge_deposits(list.clone(), vec![dep("b", 9)]);
        assert_eq!(n2, 0);
        assert_eq!(again, list);
        let many: Vec<Deposit> = (0..6_000).map(|i| dep(&format!("k{i}"), i)).collect();
        let (all, n3) = merge_deposits(list, many);
        assert_eq!((all.len(), n3), (6_003, 6_000)); // 예전 상한 5,000 을 넘어도 전부
    }

    // 🔴 기록 쓰기(코덱스 개발 69 1차): 깨진 파일은 덮지 않고 에러, 남의 주소 기록은 옆으로 치우고 새로 시작.
    #[test]
    fn store_found_refuses_broken_and_sets_aside_foreign() {
        let dir = std::env::temp_dir().join(format!("kura-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dp = dir.join("deposits-8453.json");
        assert_eq!(store_found(&dp, "0xMe", vec![dep("a", 1)]).unwrap(), 1);
        assert_eq!(
            store_found(&dp, "0xMe", vec![dep("a", 1), dep("b", 2)]).unwrap(),
            1
        );
        assert_eq!(policy::deposits_of(&dp, "0xme").len(), 2);

        std::fs::write(&dp, "{ 깨짐").unwrap();
        assert!(store_found(&dp, "0xMe", vec![dep("c", 3)]).is_err());
        assert_eq!(std::fs::read_to_string(&dp).unwrap(), "{ 깨짐"); // 안 덮었다

        let foreign = policy::DepositLog {
            address: "0xOld".into(),
            items: vec![dep("old", 1)],
        };
        std::fs::write(&dp, serde_json::to_string(&foreign).unwrap()).unwrap();
        assert_eq!(store_found(&dp, "0xMe", vec![dep("c", 3)]).unwrap(), 1);
        assert_eq!(policy::deposits_of(&dp, "0xMe").len(), 1);
        let aside = dir.join("deposits-8453.0xold.json");
        assert_eq!(policy::deposits_of(&aside, "0xOld").len(), 1);

        // 🔴 A→B→A→B (코덱스 2차): 돌아온 주소는 보관분을 되찾고, 다시 치울 때 옛 보관분을 덮어쓰지 않는다.
        assert_eq!(
            store_found(&dp, "0xOld", vec![dep("new-old", 4)]).unwrap(),
            1
        );
        let back: Vec<_> = policy::deposits_of(&dp, "0xOld")
            .into_iter()
            .map(|d| d.key)
            .collect();
        assert_eq!(back, ["new-old", "old"]);
        assert!(!aside.exists());
        let me_aside = dir.join("deposits-8453.0xme.json");
        assert_eq!(policy::deposits_of(&me_aside, "0xMe").len(), 1);
        std::fs::write(&aside, serde_json::to_string(&foreign).unwrap()).unwrap(); // 겹치는 옛 보관분이 또 있을 때
        assert_eq!(claim_owner(&dp, "0xMe").unwrap().len(), 1);
        let kept: Vec<_> = policy::deposits_of(&aside, "0xOld")
            .into_iter()
            .map(|d| d.key)
            .collect();
        assert_eq!(kept, ["new-old", "old"]); // 둘 다 남았다(덮어쓰지 않았다)
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn amounts_trim_trailing_zeros() {
        assert_eq!(fmt_amount(U256::from(2_000_000u64), 6), "2");
        assert_eq!(fmt_amount(U256::from(1u64), 6), "0.000001");
        assert_eq!(fmt_amount(U256::from(10u64).pow(U256::from(18)), 18), "1");
    }

    /// 가짜 체인 — 블록마다 (내 논스 변화, 직접 입금, 내부 입금, 내 지출).
    struct FakeChain {
        /// 블록 n 에서 일어난 일.
        events: HashMap<u64, (u64, Vec<DirectIn>, u64, u64)>,
        calls: usize,
    }

    impl FakeChain {
        fn at(&self, n: u64) -> (U256, u64) {
            let mut bal: i128 = 1_000_000;
            let mut nonce = 0;
            for (b, (dn, direct, internal, spent)) in &self.events {
                if *b <= n {
                    nonce += dn;
                    bal += direct
                        .iter()
                        .map(|d| d.value.to::<u64>() as i128)
                        .sum::<i128>();
                    bal += *internal as i128;
                    bal -= *spent as i128;
                }
            }
            (U256::from(bal as u128), nonce)
        }
    }

    impl EthView for FakeChain {
        async fn state(&mut self, n: u64) -> Result<(U256, u64), String> {
            self.calls += 1;
            Ok(self.at(n))
        }
        async fn direct_in(&mut self, n: u64) -> Result<(u64, Vec<DirectIn>), String> {
            self.calls += 1;
            Ok((
                n * 2,
                self.events.get(&n).map(|e| e.1.clone()).unwrap_or_default(),
            ))
        }
    }

    fn din(tx: &str, v: u64) -> DirectIn {
        DirectIn {
            tx: tx.into(),
            from: other(),
            value: U256::from(v),
        }
    }

    // 🔴 ETH 이분 탐색: 직접 입금은 해시와 함께, 내부 전송은 금액만, 내 지출은 입금이 아니다.
    // 지출과 입금이 서로 상쇄돼 양 끝 잔액이 같아도(논스가 달라서) 놓치지 않는다.
    #[tokio::test]
    async fn eth_bisection_finds_direct_internal_and_ignores_spends() {
        let mut events = HashMap::new();
        events.insert(100, (0, vec![din("0xaa", 500)], 0, 0)); // 직접 입금
        events.insert(250, (1, vec![], 0, 300)); // 내 송금(지출 300)
        events.insert(400, (0, vec![din("0xbb", 300)], 0, 0)); // 지출을 상쇄하는 입금 → 끝 잔액 = 시작+500
        events.insert(777, (0, vec![], 42, 0)); // 컨트랙트 내부 전송
        let mut fake = FakeChain { events, calls: 0 };
        let (sa, sb) = (fake.at(0), fake.at(1000));
        let mut got = eth_deposits_between(&mut fake, 0, sa, 1000, sb)
            .await
            .unwrap();
        got.sort_by_key(|d| d.block);
        let summary: Vec<_> = got
            .iter()
            .map(|d| (d.block, d.tx.as_str(), d.amount.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                (100, "0xaa", "0.0000000000000005"),
                (400, "0xbb", "0.0000000000000003"),
                (777, "", "0.000000000000000042"),
            ]
        );
        assert_eq!(got[0].ts, 200); // 블록 시각(가짜 = 블록×2)
        assert!(got[2].from.is_empty());
    }

    // 🔴 창 양 끝의 잔액이 **정확히 같아도** 논스가 다르면 들여다본다 — 300 을 보내고 300 을 받은 창.
    // (「잔액이 같으면 입금 0」은 논스가 같을 때만 참이다. 위 검사는 끝 잔액이 달라 이 조건을 안 물었다.)
    #[tokio::test]
    async fn eth_bisection_sees_deposit_hidden_by_equal_spend() {
        let mut events = HashMap::new();
        events.insert(250, (1, vec![], 0, 300));
        events.insert(400, (0, vec![din("0xcc", 300)], 0, 0));
        let mut fake = FakeChain { events, calls: 0 };
        let (sa, sb) = (fake.at(0), fake.at(1000));
        assert_eq!(sa.0, sb.0);
        let got = eth_deposits_between(&mut fake, 0, sa, 1000, sb)
            .await
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].tx, "0xcc");
    }

    // 조용한 구간은 끝점 둘만 보고 끝난다 — 90일 채우기의 비용이 여기서 갈린다.
    #[tokio::test]
    async fn eth_bisection_skips_quiet_span_cheaply() {
        let mut fake = FakeChain {
            events: HashMap::new(),
            calls: 0,
        };
        let s = fake.at(0);
        let got = eth_deposits_between(&mut fake, 0, s, 43_200, s)
            .await
            .unwrap();
        assert!(got.is_empty());
        assert_eq!(fake.calls, 0);
    }

    async fn live(url: &str, me: Address) -> Rpc<impl Provider> {
        Rpc {
            provider: ProviderBuilder::new().connect(url).await.unwrap(),
            me,
            chain_id: 0,
            gap: CALL_GAP,
            calm: 0,
            last_call: None,
            block_ts: HashMap::new(),
            states: HashMap::new(),
        }
    }

    /// 실체인 대조(개발 69) — 네트워크가 필요해 `--ignored` 로만 돈다. 실지갑 파일은 안 건드린다.
    /// 표본은 개발 69 에 공개 RPC 로 찾은 남의 거래다(금액·블록·해시를 영수증에서 옮겨 적었다).
    /// `cargo test deposits_live -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn deposits_live_matches_real_chains() {
        // ① Base 메인넷 ETH, 조용한 주소: ±2000 블록 동안 논스 0 그대로, 블록 51770619 에 직접 입금 하나.
        let quiet = address!("0x3e578d1b67ed1cca5e827db5aaf8fb4b1c39c7f3");
        let mut r = live("https://mainnet.base.org", quiet).await;
        let (a, b) = (51_770_619 - 2_000, 51_770_619 + 2_000);
        let (sa, sb) = (r.state(a).await.unwrap(), r.state(b).await.unwrap());
        let got = eth_deposits_between(&mut r, a, sa, b, sb).await.unwrap();
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].block, 51_770_619);
        assert_eq!(
            got[0].tx,
            "0x7c69a26bd0e1eec16ee0173b7308aa65d0618660116d38eed1422b7cc5df5e45"
        );
        assert_eq!(got[0].amount, "0.0220587");

        // ② 받고 나서 곧 세 번 보낸 주소(논스 0 → 3): 지출이 섞여도 입금 한 건을 해시와 함께 찾는다.
        let busy = address!("0x61c1f53ff754a437ce3fd2153ad5faa28dca4f96");
        let mut r = live("https://mainnet.base.org", busy).await;
        let (a, b) = (51_770_620 - 2_000, 51_770_620 + 2_000);
        let (sa, sb) = (r.state(a).await.unwrap(), r.state(b).await.unwrap());
        assert_ne!(sa.1, sb.1);
        let got = eth_deposits_between(&mut r, a, sa, b, sb).await.unwrap();
        let hit: Vec<_> = got.iter().filter(|d| d.block == 51_770_620).collect();
        assert_eq!(hit.len(), 1, "{got:?}");
        assert_eq!(
            hit[0].tx,
            "0xb420222f559c9639fdd19ad1accf7a118b99af18871df234b7fa1e60a151fbaa"
        );
        assert_eq!(hit[0].amount, "0.00057697247943057");

        // ③ Arc 테스트넷: 개발 65 의 x402 직접 제출(0xfc59…) — 받는 쪽에 0.01 USDC 가 **한 번만** 잡힌다
        //    (그 거래는 `0x3600…` 과 미러에 로그를 하나씩 냈다. 미러만 보니 하나).
        let payee = address!("0x039b1e021c0b4b62df22bf380fcadf34bf1f7778");
        let mut r = live("https://rpc.testnet.arc.network", payee).await;
        let blk = 0x3c9ab4a;
        let logs = r.logs(ARC_NATIVE_MIRROR, blk - 10, blk + 10).await.unwrap();
        let deps: Vec<_> = logs
            .iter()
            .filter_map(|l| deposit_from_log(l, payee, 18, 0))
            .filter(|d| d.tx.starts_with("0xfc59f0b8"))
            .collect();
        assert_eq!(deps.len(), 1, "{deps:?}");
        assert_eq!(deps[0].amount, "0.01");
    }

    /// 실제 돌기(`scan_with`)를 끝까지 — 임시 HOME 에서만(실지갑 `~/.jigap` 을 안 건드린다). 커서를 이어 가며
    /// 뒤로 채우다 표본 입금을 만나면 멈추고, 한 번 더 돌려 같은 입금이 두 번 적히지 않는지 본다.
    /// HOME 을 바꾸므로 **이 검사만 따로** 돌린다: `cargo test deposits_scan_live -- --ignored --test-threads=1`
    #[tokio::test]
    #[ignore]
    async fn deposits_scan_live_end_to_end() {
        let home = std::env::temp_dir().join(format!("kura-dep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_var("HOME", &home);
        assert!(jigap_dir().unwrap().starts_with(&home));

        use crate::chain::{ARC_TESTNET, BASE_MAINNET};
        let cases = [
            // (체인, 주소, 찾을 해시 앞부분, 금액, 토큰)
            (
                ARC_TESTNET,
                "0x039b1e021c0b4b62df22bf380fcadf34bf1f7778",
                "0xfc59f0b8",
                "0.01",
                "USDC",
            ),
            (
                BASE_MAINNET,
                "0x3e578d1b67ed1cca5e827db5aaf8fb4b1c39c7f3",
                "0x7c69a26b",
                "0.0220587",
                "ETH",
            ),
        ];
        for (chain, addr, tx, amount, token) in cases {
            let dp = deposits_path(chain.chain_id, 0).unwrap();
            let find = || -> Option<Deposit> {
                policy::deposits_of(&dp, addr)
                    .into_iter()
                    .find(|d| d.tx.starts_with(tx))
            };
            let mut ticks = 0;
            while find().is_none() {
                ticks += 1;
                assert!(ticks <= 40, "{} 에서 {tx} 를 못 찾았다", chain.chain_id);
                // 네트워크가 잠깐 끊겨도 앱처럼 다음 차례에 이어 간다(커서는 끝난 청크까지 저장돼 있다).
                let r = with_pinned_chain(
                    chain.chain_id,
                    scan_with(chain, 0, addr.into(), chain.default_rpc.into()),
                )
                .await;
                eprintln!("[{}] tick {ticks}: {r:?}", chain.chain_id);
            }
            let d = find().unwrap();
            assert_eq!((d.amount.as_str(), d.token.as_str()), (amount, token));
            let sp = scan_path(chain.chain_id, 0).unwrap();
            let mut st: ScanState = read_json(&sp);
            eprintln!(
                "[{}] floor {} logs {:?} eth {:?}",
                chain.chain_id, st.floor, st.logs, st.eth
            );
            assert!(st.floor < d.block);
            // 커서를 표본 블록 위로 되감아 같은 구간을 다시 훑게 한다 — 기록 수가 그대로여야 한다(키 중복 방지).
            let before = policy::deposits_of(&dp, addr).len();
            st.logs.low = st.logs.low.max(d.block + 5);
            st.eth.low = st.eth.low.max(d.block + 5);
            write_json(sp.clone(), &st).unwrap();
            // 한 차례의 시간(20초)을 앞으로 가는 데 다 쓸 수 있어서(429 로 쉬면) 몇 차례까지 기다린다.
            for n in 1..=6 {
                let r = with_pinned_chain(
                    chain.chain_id,
                    scan_with(chain, 0, addr.into(), chain.default_rpc.into()),
                )
                .await;
                let again: ScanState = read_json(&sp);
                eprintln!(
                    "[{}] rerun {n}: {r:?} logs {:?} eth {:?}",
                    chain.chain_id, again.logs, again.eth
                );
                if again.logs.low < d.block && (chain.native_is_usdc || again.eth.low < d.block) {
                    break;
                }
                assert!(n < 6, "되감은 구간을 다시 안 봤다");
            }
            let after = policy::deposits_of(&dp, addr);
            assert!(
                after.iter().filter(|d| d.tx.starts_with(tx)).count() == 1,
                "같은 입금이 두 번 적혔다"
            );
            assert_eq!(after.len(), before);
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn range_errors_are_recognized() {
        assert!(is_range_error("eth_getLogs is limited to a 2,000 range"));
        assert!(is_range_error("requested range too large"));
        assert!(!is_range_error("connection refused"));
        assert!(!is_range_error("rate limit exceeded"));
        // 개발 69 실측 문구(Arc 테스트넷).
        assert!(is_rate_limited(
            r#"HTTP error 429 with body: {"jsonrpc":"2.0","id":10,"error":{"code":-32005,"message":"rate limit exceeded"}}"#
        ));
        assert!(!is_rate_limited("requested range too large"));
    }
}
