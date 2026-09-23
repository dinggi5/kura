// ~/.jigap 파일을 읽는 **규칙의 정본** — GUI(src-tauri)와 MCP·CLI(kura-mcp)가 같은 소스 파일을
// 컴파일한다 (개발 56).
//
// 크레이트가 아니다. 두 크레이트의 lib.rs 가 `#[path = "../../shared/policy.rs"] mod policy;` 로
// 이 파일을 제 모듈로 끌어들인다 → Cargo 의존성·워크스페이스 변화 0(「공유 크레이트를 만들지
// 않는다 — Tauri 빌드 위험 0」 정책은 그대로), 그러나 정본은 하나다. 개발 52·54 에 생긴 「같은 파일을
// 두 곳(세 곳)에서 다르게 읽는」 뿌리를 여기서 뽑는다:
//   - settings.json → 어느 체인인가 (chain.rs 의 단독 읽기 vs settings.rs 의 Settings 파싱, + MCP 사본)
//   - settings.json → 사용자 지정 RPC (app 은 Settings 전체 파싱, MCP 는 rpc_url 단독 읽기 — 개발 57)
//   - settings.json → ERC-8004 조회 스위치 (같은 구조, 개발 57)
//   - history 파일의 항목 형식 HistoryEntry (app 이 쓰고 MCP·CLI 가 읽는다 — 어긋나면 AI 가 내역을 못 본다)
//   - AI·화면으로 나가는 문자열의 URL 가리기 redact_urls (키 유출 방지 — 두 벌이면 한쪽만 구멍 난다)
//   - ~/.jigap 디렉터리 이름
//   - wallet.enc   → 계정 목록·활성 계정 정규화 (app EncryptedWallet vs MCP EncMeta)
//   - 체인별·계정별 데이터 파일 이름 (chain_file · account_file_name, 양쪽 사본)
//   - 「지갑이 이미 있는가」 (설정·체인·언어 기본값이 전부 이 질문으로 신규/기존을 가른다)
//
// 여기 두는 것의 조건: **IO 판정과 순수 규칙만.** i18n 매크로(`ts!`/`tf!`)·체인 상수 묶음(ChainConfig —
// 공식 RPC 주소 포함)·에러 문구는 각 크레이트가 계속 따로 가진다 — 이 파일은 두 크레이트 어느 쪽의 모듈도 참조하지
// 않아야 양쪽에서 그대로 컴파일된다(의존은 std + serde + serde_json 만).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── 체인 ID 정본 ────────────────────────────────────────────────────────────────────────────
// 두 크레이트의 ChainConfig 상수가 이 값을 쓴다. 폴백 판정(아래 chain_id_for)이 테스트넷/메인넷을
// 가리키므로 최소 그 둘은 여기 있어야 하고, Arc 는 짝을 맞추려고 함께 둔다.

/// Base Sepolia (테스트넷) — 기본 체인. 데이터 파일이 접미사 없이 저장되는 「원본」 체인.
pub const BASE_SEPOLIA_ID: u64 = 84_532;
/// Base 메인넷 (실제 자금). 진짜 신규 설치의 기본(개발 39).
pub const BASE_MAINNET_ID: u64 = 8453;
/// Arc 테스트넷 (Circle L1, 개발 50).
pub const ARC_TESTNET_ID: u64 = 5_042_002;
/// Arc 메인넷 (실제 자금, 개발 62). 2026-09-16 공개 — `eth_chainId` = 0x13b2 실응답.
pub const ARC_MAINNET_ID: u64 = 5042;
/// 두 크레이트가 아는 체인 전부 — settings.json 의 chain_id 가 이 밖이면 크레이트가 Base Sepolia 로 접는다.
/// 체인을 추가하면 여기와 양쪽 ChainConfig 탐색에 같이 넣는다(각 크레이트 테스트가 짝을 검사한다).
pub const SUPPORTED_CHAIN_IDS: [u64; 4] = [
    BASE_SEPOLIA_ID,
    BASE_MAINNET_ID,
    ARC_TESTNET_ID,
    ARC_MAINNET_ID,
];

// ── ~/.jigap ────────────────────────────────────────────────────────────────────────────────

/// 홈 아래 데이터 디렉터리 이름. 두 크레이트의 `jigap_dir()` 이 홈을 각자 구해(dirs 크레이트 + i18n 에러
/// 문구) 여기에 붙인다 — 이름이 갈리면 GUI 가 만든 지갑을 MCP 가 못 찾는다.
pub const JIGAP_DIR_NAME: &str = ".jigap";

/// 홈 → 데이터 디렉터리 경로.
pub fn jigap_dir_in(home: &Path) -> PathBuf {
    home.join(JIGAP_DIR_NAME)
}

// ── 지갑 유무 ───────────────────────────────────────────────────────────────────────────────

/// `~/.jigap` 에 지갑 파일이 이미 있는가 — 암호화본(wallet.enc)과 옛 평문(wallet.json) 둘 다 본다.
/// settings.json 이 없거나 필드가 비었을 때 「진짜 신규 설치」와 「기존 사용자」를 가르는 질문이고,
/// 체인 기본값(아래)·언어 기본값(i18n::init)·설정 기본값(Settings) 이 전부 같은 답을 봐야 한다.
pub fn wallet_exists_in(dir: &Path) -> bool {
    dir.join("wallet.enc").exists() || dir.join("wallet.json").exists()
}

// ── settings.json → 체인 ────────────────────────────────────────────────────────────────────

/// settings.json 을 읽은 결과의 세 갈래. 「없음」과 「있는데 못 읽음」이 다른 답을 내므로(개발 39 —
/// 신규 기본이 메인넷이 된 순간부터) 읽기 실패를 한 덩이로 뭉치면 안 된다.
#[derive(Debug, PartialEq)]
pub enum SettingsFile {
    /// 파일이 없다(첫 실행, 또는 저장을 눌러야만 파일이 생기던 개발 31 이전 설치).
    Missing,
    /// 파일이 있는데 못 읽었다(권한 등) — 깨진 파일과 같이 보수적으로 다룬다.
    Unreadable,
    /// 본문. 해석은 호출자 몫(깨졌을 수도 있다).
    Text(String),
}

impl SettingsFile {
    /// 경로에서 읽어 세 갈래로 나눈다. `NotFound` 만 Missing, 그 외 IO 실패는 전부 Unreadable.
    pub fn read(path: &Path) -> SettingsFile {
        match std::fs::read_to_string(path) {
            Ok(text) => SettingsFile::Text(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => SettingsFile::Missing,
            Err(_) => SettingsFile::Unreadable,
        }
    }
}

/// settings.json 에서 선택된 체인 ID 만 읽는 가벼운 뷰 — **다른 필드는 무시**한다. 한도 필드 하나가
/// 깨진 파일도 chain_id 는 그대로 읽힌다. 돈이 나가는 쪽(서명·송금)이 이 판정을 쓰므로, 화면·설정
/// (Settings 파싱)도 같은 판정을 따라야 한다(개발 52: 「연습용」 라벨에 메인넷 송금).
#[derive(Deserialize)]
struct ChainSel {
    chain_id: u64,
}

/// settings.json 본문 → chain_id. 깨졌거나 필드가 없으면 테스트넷.
/// 값은 **정규화하지 않는다**(미지원 id 도 그대로) — 지원 여부는 각 크레이트의 ChainConfig 탐색이
/// 판단한다(미지원 → Base Sepolia 폴백). 여기서 접으면 화면이 「모르는 체인」을 알릴 수 없다.
pub fn chain_id_in(text: &str) -> u64 {
    serde_json::from_str::<ChainSel>(text)
        .map(|c| c.chain_id)
        .unwrap_or(BASE_SEPOLIA_ID)
}

/// **사용자가 선택한 체인 ID — 유일한 판정.** 폴백은 넷으로 갈린다(개발 39 — 신규 기본이 메인넷이
/// 되면서 「없음」과 「못 읽음」이 다른 답이 됐다):
/// - 파일 없음 + **지갑도 없음** = 진짜 신규 → 메인넷(신규 기본)
/// - 파일 없음 + 지갑 있음 = 개발 31 이전 설치(저장을 눌러야만 settings.json 이 생겼다)
///   → 테스트넷(그 시절 사용자를 조용히 실돈 체인으로 옮기지 않는다 — 코덱스 개발 39 P1)
/// - 있는데 못 읽음(권한·홈 못 정함) → 테스트넷(보수적)
/// - 본문 → `chain_id_in` (정상이면 그 값, 깨졌거나 옛 파일이라 필드가 없으면 테스트넷)
///
/// `wallet_exists` 는 클로저다 — 파일이 없을 때만 묻는다(이 함수는 체인을 쓰는 모든 곳에서 자주 불린다).
pub fn chain_id_for(file: &SettingsFile, wallet_exists: impl FnOnce() -> bool) -> u64 {
    match file {
        SettingsFile::Missing if wallet_exists() => BASE_SEPOLIA_ID,
        SettingsFile::Missing => BASE_MAINNET_ID,
        SettingsFile::Unreadable => BASE_SEPOLIA_ID,
        SettingsFile::Text(text) => chain_id_in(text),
    }
}

// ── settings.json → RPC ────────────────────────────────────────────────────────────────────

/// settings.json 본문 → 사용자 지정 RPC(앞뒤 공백 제거). 없거나 비었거나 깨졌으면 빈 값 =
/// 「활성 체인의 공식 RPC 를 따라간다」. ChainSel 과 같은 이유로 **다른 필드는 무시**한다 — 한도
/// 필드 하나가 깨진 파일에서 GUI 는 Settings 파싱에 실패해 공식 RPC 로 접고 MCP 는 이 필드만 읽어
/// 커스텀 RPC 를 쓰던 것(개발 51 하네스 실측·개발 56 대체 리뷰 P3)이 두 판정의 뿌리였다.
///
/// 🔴 **단, 파일의 `chain_id` 를 못 알아보면 지정 RPC 도 버린다**(코덱스 개발 57 1차 P1). 지정 RPC 는 파일이
/// 말하는 체인의 엔드포인트다. chain_id 가 깨졌거나(타입 틀림·null·키 중복) 미지원 값이면 크레이트는
/// 체인을 Base Sepolia 로 접는데, 그 RPC 가 메인넷 것이면 화면·한도는 「연습용」인 채 서명 provider 는
/// 메인넷에 붙는다 — 서명자는 체인 ID 를 못박지 않고 RPC 가 답하는 값을 쓰므로 진짜 송금이 나간다.
/// `pick_rpc` 의 `forced_other_chain` 과 같은 이유(딴 체인의 RPC 는 쓰지 않는다). 필드가 **없는** 옛 파일
/// (개발 20 이전, Sepolia 뿐이던 시절)은 그 RPC 도 Sepolia 것이라 유지 — `chain_id_in` 도 같은 답(Sepolia).
///
/// 🔴 chain_id 는 **`chain_id_in` 과 같은 방식(serde 파생)** 으로 읽는다(코덱스 2차 P1). `serde_json::Value`
/// 로 읽으면 같은 키가 두 번 있을 때 마지막 값을 조용히 쓰는데 파생은 중복을 거부한다 —
/// `{"chain_id":84532,"chain_id":8453,"rpc_url":메인넷}` 에서 체인은 Sepolia 로 접히고 RPC 만 메인넷이
/// 살아남는 갈림이 생긴다. 파생끼리면 「체인을 못 읽는 파일 = RPC 도 못 읽는 파일」이 구조로 보장된다
/// (`rpc_kept_implies_chain_recognized` 테스트).
///
/// 값 자체는 검사하지 않는다 — http(s) 여부는 저장 경로(set_settings)의 몫이고, 이미 파일에 있는 값은
/// 어느 프로세스든 **같은 값**을 써야 한다.
pub fn rpc_url_in(text: &str) -> String {
    let Ok(sel) = serde_json::from_str::<RpcSel>(text) else {
        return String::new();
    };
    let chain_known = match sel.chain_id {
        None => true, // 옛 파일 — Sepolia 시절
        Some(id) => SUPPORTED_CHAIN_IDS.contains(&id),
    };
    if chain_known {
        sel.rpc_url.trim().to_string()
    } else {
        String::new()
    }
}

/// `rpc_url_in` 의 뷰 — 다른 필드는 무시하되 **이 두 필드의 형식은 ChainSel 만큼 엄격**하다(중복 키·
/// 타입 틀림·null 이면 파일 전체를 못 읽은 것으로). `chain_id` 가 없는 건 허용(옛 파일)하지만 null 은
/// 아니다 — `Option` 기본 역직렬화는 null 을 None 으로 받으므로 `present_u64` 로 막는다.
#[derive(Deserialize)]
struct RpcSel {
    #[serde(default, deserialize_with = "present_u64")]
    chain_id: Option<u64>,
    #[serde(default)]
    rpc_url: String,
}

/// 필드가 **있으면** u64 여야 한다(null·문자열·실수는 오류). 없을 때만 serde default(None).
fn present_u64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    u64::deserialize(d).map(Some)
}

/// **사용자가 지정한 RPC — 유일한 판정.** 파일이 없거나 못 읽으면 빈 값(공식 RPC). 체인 판정과
/// 달리 지갑 유무는 묻지 않는다 — 어느 갈래든 「지정한 게 없다」 는 같은 답이다.
pub fn rpc_url_for(file: &SettingsFile) -> String {
    match file {
        SettingsFile::Missing | SettingsFile::Unreadable => String::new(),
        SettingsFile::Text(text) => rpc_url_in(text),
    }
}

/// 지정 RPC 와 활성 체인의 공식 RPC 중 무엇을 쓸지.
///
/// `forced_other_chain` = 환경변수(`KURA_CHAIN_ID`, MCP·CLI 만)가 settings 와 **다른** 체인을
/// 강제한 상태. 그때 settings 의 rpc_url 은 **딴 체인의 엔드포인트**다 — 그대로 쓰면 이 체인의
/// 컨트랙트를 저쪽 체인에 물어 잔액이 `returned no data ("0x")` 로 죽는다(개발 48 실측 → 개발 49).
/// 커스텀 RPC 를 조용히 버리는 셈이지만, 대안이 「조용히 안 되는 것」이라 이쪽이 낫다. GUI 에는
/// 환경변수로 체인을 갈아타는 경로가 없으므로 항상 false 를 준다.
pub fn pick_rpc(custom: &str, forced_other_chain: bool, default_rpc: &str) -> String {
    if custom.is_empty() || forced_other_chain {
        default_rpc.to_string()
    } else {
        custom.to_string()
    }
}

// ── settings.json → ERC-8004 조회 스위치 ────────────────────────────────────────────────────────

/// `agent_lookup` 한 필드만 읽는 뷰 — RPC 와 같은 구조(개발 57). **기본 켜짐**: 새 바깥 연결을 여는 게
/// 아니라 이미 잔액을 읽는 그 RPC 로 읽기 한 번을 더 하는 것이라, 끄는 쪽이 명시적 선택이다.
#[derive(Deserialize)]
struct LookupSel {
    #[serde(default = "yes")]
    agent_lookup: bool,
}

fn yes() -> bool {
    true
}

/// settings.json 본문 → ERC-8004 조회를 켜 뒀는가. 필드 없음·깨짐(중복 키 포함) → 켜짐.
/// 명시적으로 끈 파일만 꺼짐 — 사용자 선택이 기본값에 먹히지 않게.
pub fn agent_lookup_in(text: &str) -> bool {
    serde_json::from_str::<LookupSel>(text)
        .map(|s| s.agent_lookup)
        .unwrap_or(true)
}

/// 파일 → 조회 스위치. 없거나 못 읽으면 켜짐(기능이 조용히 사라지는 것보다 낫다). 앱 `read_settings` 와
/// MCP `erc8004::lookup_enabled` 가 같은 함수를 쓴다 — 깨진 파일에서 설정 화면은 「켜짐」인데 MCP 는
/// 조회를 건너뛰던 갈림(돈은 안 움직이지만 판단 재료가 다르다)을 없앤다.
pub fn agent_lookup_for(file: &SettingsFile) -> bool {
    match file {
        SettingsFile::Missing | SettingsFile::Unreadable => true,
        SettingsFile::Text(text) => agent_lookup_in(text),
    }
}

// ── 체인별·계정별 데이터 파일 이름 ─────────────────────────────────────────────────────────────

/// 체인별로 분리하는 데이터 파일 이름(spend/history/x402_settlements/trusted).
/// 기본 체인(Base Sepolia)은 **기존 이름 그대로**("history.json") → 무손실 마이그레이션, 그 외는
/// "-{chain_id}" 접미("history-8453.json") — 테스트넷/메인넷의 사용액·내역·신뢰 목록이 절대 섞이지 않게.
/// `chain_id` 는 **정규화된(지원되는) 값**을 넘긴다 — 호출자는 `active_chain().chain_id` 를 준다.
/// 미지원 id 가 여기까지 오면 Sepolia 파일과 어긋난 새 파일이 생겨 한도·신뢰 목록이 조용히 리셋된다.
pub fn chain_file_name(chain_id: u64, stem: &str) -> String {
    if chain_id == BASE_SEPOLIA_ID {
        format!("{stem}.json")
    } else {
        format!("{stem}-{chain_id}.json")
    }
}

/// **계정별로도** 분리하는 데이터 파일 이름(개발 54) — 지금은 내역(history)만. 체인 접미 뒤에 계정
/// 접미를 덧붙인다: 계정 0 은 **기존 이름 그대로**(무손실), 그 외는 `-a{n}`("history-a2.json",
/// "history-8453-a2.json"). 어긋나면 GUI 가 적은 내역을 AI 가 못 본다.
pub fn account_file_name(chain_base: &str, index: u32) -> String {
    match index {
        0 => chain_base.to_string(),
        n => format!("{}-a{n}.json", chain_base.trim_end_matches(".json")),
    }
}

// ── wallet.enc → 계정 ───────────────────────────────────────────────────────────────────────

/// 계정 하나(개발 54) — 같은 시드의 HD 파생 인덱스(m/44'/60'/0'/0/n) + 그 주소(공개정보, 평문) +
/// 사람이 붙인 라벨(빈 값 = 화면이 「계정 N」으로). wallet.enc 의 `accounts` 항목이자 MCP 상태의 항목.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Account {
    /// 파생 인덱스 n. 0 = 지갑을 만들 때부터 있던 원래 계정.
    pub index: u32,
    pub address: String,
    #[serde(default)]
    pub label: String,
}

/// wallet.enc 의 계정 목록 정규화 — 인덱스 순, **계정 0 은 항상 있고 그 주소는 파일의 `address`
/// 필드가 정본**이다(목록에 0 이 따로 적혀 있어도 주소는 `address` 가 이기고 라벨만 목록 것을 쓴다 —
/// 옛 빌드도 그 필드를 읽으니 둘이 어긋나면 안 된다). 옛 파일(목록 없음)은 계정 0 하나가 된다.
pub fn normalize_accounts(address: &str, listed: &[Account]) -> Vec<Account> {
    let mut list: Vec<Account> = listed.iter().filter(|a| a.index != 0).cloned().collect();
    let zero_label = listed
        .iter()
        .find(|a| a.index == 0)
        .map(|a| a.label.clone())
        .unwrap_or_default();
    list.push(Account {
        index: 0,
        address: address.to_string(),
        label: zero_label,
    });
    list.sort_by_key(|a| a.index);
    list.dedup_by_key(|a| a.index);
    list
}

/// 활성 계정 — `active` 가 목록에 없으면(손상·옛 빌드가 쓴 파일) 계정 0. 돈이 나가는 계정이
/// 「없는 계정」이 되면 안 된다. `list` 는 `normalize_accounts` 의 결과(계정 0 이 반드시 있다).
pub fn pick_active(list: &[Account], active: u32) -> Account {
    list.iter()
        .find(|a| a.index == active)
        .or_else(|| list.iter().find(|a| a.index == 0))
        .or_else(|| list.first())
        .cloned()
        .expect("normalize_accounts 는 계정 0 을 항상 넣는다")
}

// ── history 파일 항목 ───────────────────────────────────────────────────────────────────────────

/// 송금 시도 1건의 기록(감사 로그) — GUI 가 history 파일에 쓰고 MCP·CLI 가 그대로 읽는다.
/// 성공/차단/실패를 모두 남긴다. 필드를 더할 땐 `#[serde(default)]` 를 붙여 옛 기록이 계속 읽히게 한다.
#[derive(Serialize, Deserialize, Clone)]
pub struct HistoryEntry {
    /// 유닉스 초.
    pub ts: u64,
    /// "ETH" | "USDC".
    pub token: String,
    /// 받는 주소 (시도 당시 입력값).
    pub to: String,
    /// 금액 (십진수 문자열).
    pub amount: String,
    /// "sent" | "blocked" | "failed" | "signed"(x402 서명·정산 대기) | "settled"(x402 정산됨) | "settle_failed"
    /// | "unknown"(개발 66 — 서명한 tx 를 냈는데 체인이 받았는지 모름. 한도는 환불하지 않았다).
    pub status: String,
    /// sent·unknown=tx 해시, blocked/failed=사유, signed=인가 nonce(정산 매칭용).
    pub detail: String,
    /// x402 정산 tx 해시(페이실리테이터가 온체인 제출). 정산 전엔 빈 문자열. (Session 14)
    #[serde(default)]
    pub settle_tx: String,
}

// ── URL 가리기 ──────────────────────────────────────────────────────────────────────────────

/// 사용자/AI 에 노출되는 에러·로그 문자열에서 URL 을 통째로 `[RPC]` 로 가린다.
/// 커스텀 RPC 경로·쿼리엔 API 키가 들어가곤 한다(예: alchemy `…/v2/KEY`). alloy/reqwest 에러는
/// URL 을 그대로 실어 나르므로 — 특히 MCP/CLI 결과·거래내역은 AI 채팅으로 나가 키가 LLM 에 샐 수 있다.
/// **설정을 다시 읽지 않고 문자열에 보이는 URL 자체를 가린다** → 설정 변경·host 대소문자 정규화·
/// ws/wss 등에 흔들리지 않는다(코덱스 리뷰 반영). URL 외 문자는 그대로 둔다.
/// 개발 57 까지 두 크레이트에 같은 코드가 두 벌 있었다 — 한쪽만 고치면 다른 쪽으로 키가 샌다.
pub fn redact_urls(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(rel) = input[cursor..].find("://") {
        let sep = cursor + rel;
        if let Some(start) = scheme_start(input, cursor, sep) {
            out.push_str(&input[cursor..start]);
            // 토큰 끝: 공백·제어문자 또는 URL 에 못 들어가는 문자("· <· >· `)에서 멈춘다.
            // `)`·`,`·`'` 는 URL sub-delim 이라 종료자로 안 씀 — 그 뒤 키가 새지 않게 보수적으로(코덱스).
            let token = &input[start..];
            let end = token
                .find(|c: char| {
                    c.is_whitespace() || c.is_control() || matches!(c, '"' | '<' | '>' | '`')
                })
                .unwrap_or(token.len());
            out.push_str("[RPC]");
            cursor = start + end;
        } else {
            // "://" 앞에 유효 scheme 이 없다 → 그대로 두고 그 뒤부터 계속 스캔.
            out.push_str(&input[cursor..sep + 3]);
            cursor = sep + 3;
        }
    }
    out.push_str(&input[cursor..]);
    out
}

/// `sep`(="://"의 시작 인덱스) 앞에서 scheme 시작 인덱스를 찾는다. scheme = `[A-Za-z][A-Za-z0-9+.-]*`.
/// `min` 미만으로는 내려가지 않는다(이미 처리한 영역 침범 방지). 유효 scheme 없으면 None.
fn scheme_start(s: &str, min: usize, sep: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut start = sep;
    while start > min {
        let c = bytes[start - 1];
        if c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.') {
            start -= 1;
        } else {
            break;
        }
    }
    // 최소 1글자 + 맨 앞은 영문자(RFC 3986 scheme).
    if start < sep && bytes[start].is_ascii_alphabetic() {
        Some(start)
    } else {
        None
    }
}

// ── 결제 시도 기록 `~/.jigap/approvals/` (개발 66) ─────────────────────────────────────────────
//
// 🔴 **「결과 불명」을 프로세스 밖에 남기는 자리.** 개발 65 까지 GUI 의 「승인 진행 중」은 메모리
// (`APPROVALS_IN_FLIGHT`)에만 있었다. 그래서 MCP 가 승인 대기 5분을 채우면 — 사람이 막 비번을 넣어
// GUI 가 **전송 중**이어도 — 요청을 거두고 「사용자가 응답하지 않았어요」라고 답했다. 돈은 나가고,
// GUI 의 결과는 요청이 사라졌다고 버려지고, AI 는 재시도한다 = 이중 결제(개발 65 코덱스 #1).
//
// 요청 id 하나에 파일 둘. **파일마다 쓰는 쪽은 하나뿐이다**(두 프로세스가 같은 파일을 고치지 않는다):
//   - `<id>.json`       — GUI 가 쓴다. 승인 처리의 상태(sending → done | failed | unknown)와 결과.
//   - `<id>.proof.json` — MCP 가 쓴다. x402 직접 제출의 증거 재료(뒤늦게 서버에 낼 수 있게 — 개발 65
//                          「다음」 2번의 형식. 읽는 쪽(재제출 경로)은 다음 세션이다).
//
// 순서 규약(경합을 닫는 것은 이 순서다 — 두 프로세스 사이엔 잠금이 없다):
//   GUI  : 기록을 `sending` 으로 **먼저 쓰고** → 그다음 요청 파일이 아직 있는지 본다 → 있으면 진행.
//   MCP  : 시간 초과면 요청 파일을 **먼저 지우고** → 그다음 기록을 읽는다.
// 그러면 GUI 가 요청을 보고 진행했다면 그 기록은 MCP 가 지우기 전에 쓰였으므로 MCP 가 반드시 본다.
// GUI 가 요청을 못 봤다면 진행하지 않고 기록을 되돌린다(MCP 가 그 사이 `sending` 을 봤다면 곧 사라진다).

/// 결제 시도 기록 디렉터리 이름.
pub const APPROVALS_DIR: &str = "approvals";

/// 기록을 남겨 두는 기간(초). 뒤늦은 증거 제출 창(상대 서버 600초)과 사람이 내역을 보고 따져 볼 시간을
/// 넉넉히 덮는다. 파일은 수백 바이트라 7일치여도 작다.
pub const APPROVAL_KEEP_SECS: u64 = 7 * 24 * 3600;

/// 승인 처리를 시작했다(GUI). 아직 체인으로 나갔는지 모른다 — **이 상태로 멈춰 있으면 「불명」이다**
/// (앱이 전송 중에 죽은 경우).
pub const ATTEMPT_SENDING: &str = "sending";
/// 끝났다 — 송금은 체인이 받았고, 서명은 만들어졌다. 결과(`status == "approved"`)가 함께 있다.
pub const ATTEMPT_DONE: &str = "done";
/// 확실히 아무것도 안 나갔다(비번 오류·한도·잠금·체인이 거절). 요청은 살아 있어 다시 승인할 수 있다.
pub const ATTEMPT_FAILED: &str = "failed";
/// 서명한 트랜잭션을 냈는데 **받혔는지 모른다**(응답 유실·시간 초과). tx 해시는 안다.
pub const ATTEMPT_UNKNOWN: &str = "unknown";

/// 요청 id 가 파일 이름으로 써도 되는 모양인가. id 는 MCP 가 만든 나노초 숫자지만 요청 파일은 로컬의
/// 다른 프로세스가 쓴 입력이다 — `../` 같은 값으로 디렉터리 밖을 쓰게 두지 않는다.
pub fn attempt_id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// GUI 가 쓰는 기록 경로. id 가 이상하면 None(기록을 안 남긴다 — 그 요청은 GUI 도 처리하지 않는다).
pub fn attempt_path(dir: &Path, id: &str) -> Option<PathBuf> {
    attempt_id_ok(id).then(|| dir.join(APPROVALS_DIR).join(format!("{id}.json")))
}

/// MCP 가 쓰는 증거 재료 경로. (GUI 는 안 쓴다 — 디렉터리째 청소만 한다.)
#[allow(dead_code)]
pub fn proof_path(dir: &Path, id: &str) -> Option<PathBuf> {
    attempt_id_ok(id).then(|| dir.join(APPROVALS_DIR).join(format!("{id}.proof.json")))
}

/// GUI 의 승인 처리 기록 1건 — `PaymentResult`(결과 파일)와 같은 필드를 싣는다. 결과 파일은 **한 칸뿐**이라
/// 요청이 사라지면 GUI 가 결과를 안 쓴다(남의 결과를 덮지 않으려고, 개발 51). 이 기록은 id 마다 따로라
/// 그 제약이 없다 — MCP 는 시간 초과 뒤에도 여기서 결말을 읽는다.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AttemptRecord {
    /// 형식 판. 필드를 더할 땐 `#[serde(default)]` 로 — 판을 올리는 건 뜻이 바뀔 때만.
    pub v: u32,
    pub id: String,
    /// `ATTEMPT_*` 중 하나.
    pub state: String,
    /// 요청의 kind(transfer | x402 | x402-direct) — 읽는 쪽이 「서명만 했나, 전송했나」를 가른다.
    pub kind: String,
    pub chain_id: u64,
    pub started: u64,
    pub updated: u64,
    /// 끝난 뒤의 결과 — `PaymentResult` 와 같은 뜻(approved | unknown | failed).
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub tx_hash: String,
    #[serde(default)]
    pub detail: String,
    /// x402 서명 결과(서명 갈래에서만). 크레이트마다 타입이 달라 값 그대로 싣는다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x402: Option<serde_json::Value>,
}

/// 이 기록이 있는 id 로 **승인을 다시 시작해도 되는가**(GUI `begin_approval`).
///
/// `failed` 와 기록 없음만 된다. `done`·`unknown` 은 돈이 나갔거나 나갔을 수 있다 — 같은 요청을 한 번 더
/// 승인하면 두 번째 결제다. `sending` 이 남아 있는데 진행 중인 승인이 없다면 **앱이 전송 중에 죽은 것**이라
/// 그것도 「불명」이다.
pub fn attempt_allows_retry(prev: Option<&AttemptRecord>) -> bool {
    match prev {
        None => true,
        Some(r) => r.state == ATTEMPT_FAILED,
    }
}

/// MCP 가 승인 대기 시간을 넘긴 뒤(요청 파일을 거둔 **뒤**) 기록을 보고 할 일. (MCP 만 쓴다.)
#[allow(dead_code)]
#[derive(Debug, PartialEq)]
pub enum AfterTimeout {
    /// 승인이 시작된 적 없거나 확실히 실패했다 — 아무것도 안 나갔다. 예전의 「시간 초과」 그대로.
    NothingSent,
    /// GUI 가 아직 처리 중이다 — 조금 더 기다린다.
    StillSending,
    /// 끝났다(done | unknown) — 이 기록의 결과를 결과 파일 대신 쓴다.
    Finished,
}

#[allow(dead_code)]
pub fn after_timeout(rec: Option<&AttemptRecord>) -> AfterTimeout {
    match rec.map(|r| r.state.as_str()) {
        Some(ATTEMPT_SENDING) => AfterTimeout::StillSending,
        Some(ATTEMPT_DONE) | Some(ATTEMPT_UNKNOWN) => AfterTimeout::Finished,
        // failed(비번 오류 뒤 사람이 떠난 경우 등)·모르는 상태·기록 없음 → 나간 것이 없다.
        // 모르는 상태를 여기로 접는 게 위험하지 않은가: 모르는 값은 새 앱이 새 상태를 만든 경우인데,
        // 그 앱은 하트비트 kinds 로 이미 걸러진다. 그래도 한 줄 남긴다 — 새 상태를 만들면 여기부터 고칠 것.
        _ => AfterTimeout::NothingSent,
    }
}

// ── 요청 임대 (개발 66, 코덱스 P0) ──────────────────────────────────────────────────────────
//
// MCP 가 승인을 기다리다 **SIGKILL 로 죽으면** `CancelOnDrop` 이 못 돌아 요청 파일이 남고, GUI 는 그걸
// 5분+유예 동안 「살아 있는 요청」으로 보여줬다. 사람이 승인하면 돈은 나가는데 결과를 받을 상대가 없고,
// 다시 켠 AI 는 「아까 실패했다」고 알고 또 요청한다. 이제 기다리는 쪽이 요청 파일의 수정 시각을 주기적으로
// 갱신하고(임대), GUI 는 임대가 끊긴 요청을 보여주지도 승인하지도 않는다. 파일은 지우지 않는다 — 노트북이
// 잠들었다 깨어난 직후처럼 **잠깐** 끊긴 임대는 MCP 가 다시 갱신하면 되살아나야 한다.

/// 기다리는 쪽(MCP)이 요청 파일 수정 시각을 갱신하는 주기(초). (MCP 만 쓴다.)
#[allow(dead_code)]
pub const LEASE_TOUCH_SECS: u64 = 3;
/// 이만큼 갱신이 없으면 기다리는 쪽이 없다고 본다(초). 갱신 주기의 열 배 — 한두 번 밀린 것과 가른다.
pub const REQUEST_LEASE_SECS: u64 = 30;

/// 이 요청의 임대가 끊겼나. `leased` = 요청이 임대를 약속했나(옛 MCP 는 false → 늘 false),
/// `age` = 요청 파일 수정 시각으로부터 지난 초(못 읽으면 None → 끊겼다고 단정하지 않는다).
pub fn lease_lapsed(leased: bool, age: Option<u64>) -> bool {
    leased && age.is_some_and(|a| a > REQUEST_LEASE_SECS)
}

/// 이 기록 파일을 지워도 되는가(GUI 감시 스레드의 청소). 수정 시각 기준.
pub fn attempt_prunable(age_secs: u64) -> bool {
    age_secs > APPROVAL_KEEP_SECS
}

#[cfg(test)]
mod tests {
    use super::*;

    // ~/.jigap 이름과 조회 스위치.
    #[test]
    fn jigap_dir_and_agent_lookup() {
        assert_eq!(
            jigap_dir_in(Path::new("/Users/x")),
            PathBuf::from("/Users/x/.jigap")
        );
        assert!(agent_lookup_in(r#"{"chain_id":8453}"#)); // 필드 없음 → 켜짐
        assert!(agent_lookup_in("{ 깨진 JSON"));
        assert!(agent_lookup_in(r#"{"agent_lookup":true}"#));
        // 명시적으로 끈 파일만 꺼짐. 한도 필드가 깨진 파일에서도 스위치는 읽힌다(갈리던 자리).
        assert!(!agent_lookup_in(r#"{"agent_lookup":false}"#));
        assert!(!agent_lookup_in(
            r#"{"single_usdc":5,"agent_lookup":false}"#
        ));
        assert!(agent_lookup_in(r#"{"agent_lookup":"false"}"#)); // 타입 틀림 → 못 읽음 → 켜짐
        assert!(agent_lookup_for(&SettingsFile::Missing));
        assert!(agent_lookup_for(&SettingsFile::Unreadable));
        assert!(!agent_lookup_for(&SettingsFile::Text(
            r#"{"agent_lookup":false}"#.to_string()
        )));
    }

    // history 파일 형식 — settle_tx 없는 옛 기록도 기본값으로 호환.
    #[test]
    fn history_entry_roundtrips() {
        let json = r#"{"ts":1780623842,"token":"USDC","to":"0xabc","amount":"1","status":"sent","detail":"0xhash"}"#;
        let e: HistoryEntry = serde_json::from_str(json).unwrap();
        assert_eq!(e.token, "USDC");
        assert_eq!(e.status, "sent");
        assert_eq!(e.ts, 1780623842);
        assert_eq!(e.settle_tx, "");
        let back: HistoryEntry = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back.to, "0xabc");
    }

    // URL(경로의 API 키)이 에러 메시지에서 통째로 가려져야 한다.
    #[test]
    fn redact_hides_url_api_key() {
        assert_eq!(
            redact_urls("RPC 연결 실패: https://base-sepolia.g.alchemy.com/v2/SUPERSECRETKEY"),
            "RPC 연결 실패: [RPC]",
        );
        // reqwest 처럼 괄호 안에 URL 이 박힌 경우 — 키는 사라지고 뒤 메시지는 남는다.
        let red =
            redact_urls("error sending request for url (https://h/v2/KEY): connection closed");
        assert!(!red.contains("KEY"), "키가 남음: {red}");
        assert!(
            red.contains("[RPC]") && red.contains("connection closed"),
            "형태 깨짐: {red}"
        );
    }

    // 코덱스 리뷰: host 대소문자 정규화·query 콤마 뒤 키·ws/wss 우회를 막아야 한다.
    #[test]
    fn redact_handles_case_subdelims_and_ws() {
        assert_eq!(
            redact_urls("x HTTPS://BASE.g.ALCHEMY.com/v2/KEY y"),
            "x [RPC] y"
        ); // 대소문자
        let red = redact_urls("https://rpc.example/rpc?x=a,api_key=SECRET done");
        assert!(!red.contains("SECRET"), "콤마 뒤 키가 남음: {red}");
        assert_eq!(red, "[RPC] done");
        assert_eq!(redact_urls("wss://node/abc end"), "[RPC] end"); // websocket RPC
    }

    // URL 아닌 텍스트·빈 scheme 은 그대로 둔다(멀티바이트 안전).
    #[test]
    fn redact_leaves_non_urls() {
        assert_eq!(
            redact_urls("주소 파싱 실패: bad input"),
            "주소 파싱 실패: bad input"
        );
        assert_eq!(redact_urls("just :// floating"), "just :// floating");
        assert_eq!(
            redact_urls("잔액 조회 실패: https://x/y 입니다"),
            "잔액 조회 실패: [RPC] 입니다"
        );
    }

    // settings.json 본문 → chain_id (개발 39). 깨진 JSON·필드 없는 옛 파일은 테스트넷으로 접는다.
    // 🔴 다른 필드가 깨져도 chain_id 는 그대로 — 화면(Settings 파싱)도 이 답을 따라야 한다(개발 52).
    #[test]
    fn chain_id_in_reads_only_chain_id() {
        assert_eq!(chain_id_in(r#"{"chain_id":8453}"#), BASE_MAINNET_ID);
        assert_eq!(chain_id_in(r#"{"chain_id":84532}"#), BASE_SEPOLIA_ID);
        assert_eq!(chain_id_in(r#"{"single_usdc":"5"}"#), BASE_SEPOLIA_ID); // 옛 파일
        assert_eq!(chain_id_in("{ 깨진 JSON"), BASE_SEPOLIA_ID);
        assert_eq!(chain_id_in(""), BASE_SEPOLIA_ID);
        // 타입 틀림(문자열) → 못 읽음 → 테스트넷.
        assert_eq!(chain_id_in(r#"{"chain_id":"8453"}"#), BASE_SEPOLIA_ID);
        // 한도 필드가 빠졌거나 타입이 틀린 파일 — Settings 로는 못 읽지만 체인은 읽힌다(개발 51 하네스).
        assert_eq!(
            chain_id_in(r#"{"single_usdc":5,"chain_id":8453}"#),
            BASE_MAINNET_ID
        );
        assert_eq!(
            chain_id_in(r#"{"daily_usdc":"20","chain_id":5042002}"#),
            ARC_TESTNET_ID
        );
        assert_eq!(chain_id_in(r#"{"chain_id":5042}"#), ARC_MAINNET_ID);
        // 정규화하지 않는다 — 미지원 id 는 그대로(지원 판단은 ChainConfig 탐색 몫).
        assert_eq!(chain_id_in(r#"{"chain_id":1}"#), 1);
    }

    // 🔴 신규(파일 없음)와 깨진 파일(있는데 못 읽음)은 다른 답이어야 한다 (개발 39). 신규 기본이
    // 메인넷이 된 순간 「깨졌으면 기본값」 경로는 테스트넷 사용자를 조용히 실돈 체인으로 옮기는 문이 된다.
    #[test]
    fn chain_id_for_splits_new_legacy_and_broken() {
        // 파일 없음 + 지갑 없음 = 진짜 신규 → 메인넷.
        assert_eq!(
            chain_id_for(&SettingsFile::Missing, || false),
            BASE_MAINNET_ID
        );
        // 파일 없음 + 지갑 있음 = 개발 31 이전 설치 → 테스트넷(코덱스 개발 39 P1).
        assert_eq!(
            chain_id_for(&SettingsFile::Missing, || true),
            BASE_SEPOLIA_ID
        );
        // 있는데 못 읽음 → 테스트넷. 지갑 유무는 묻지도 않는다.
        assert_eq!(
            chain_id_for(&SettingsFile::Unreadable, || unreachable!()),
            BASE_SEPOLIA_ID
        );
        // 본문이 있으면 지갑 유무와 무관하게 chain_id_in.
        let text = |s: &str| SettingsFile::Text(s.to_string());
        assert_eq!(
            chain_id_for(&text(r#"{"chain_id":8453}"#), || unreachable!()),
            BASE_MAINNET_ID
        );
        assert_eq!(
            chain_id_for(&text("{ 깨진 JSON"), || unreachable!()),
            BASE_SEPOLIA_ID
        );
    }

    // settings.json 본문 → rpc_url. 한도 필드가 깨져 Settings 로는 못 읽는 파일에서도 지정 RPC 는
    // 그대로 — GUI(Settings 파싱)와 MCP(단독 읽기)가 같은 답을 내야 한다(개발 56 대체 리뷰 P3 → 개발 57).
    #[test]
    fn rpc_url_in_reads_only_rpc_url() {
        assert_eq!(
            rpc_url_in(r#"{"rpc_url":"https://example.invalid/v2/KEY"}"#),
            "https://example.invalid/v2/KEY"
        );
        // 앞뒤 공백은 잘린다(양쪽 effective_rpc 가 trim 하던 값).
        assert_eq!(
            rpc_url_in(r#"{"rpc_url":"  https://x.invalid  "}"#),
            "https://x.invalid"
        );
        assert_eq!(rpc_url_in(r#"{"chain_id":8453}"#), ""); // 필드 없음(옛 파일) = 공식
        assert_eq!(rpc_url_in(r#"{"rpc_url":""}"#), "");
        assert_eq!(rpc_url_in("{ 깨진 JSON"), "");
        assert_eq!(rpc_url_in(""), "");
        // 타입 틀림 → 못 읽음 → 공식.
        assert_eq!(rpc_url_in(r#"{"rpc_url":7}"#), "");
        // 개발 51 하네스의 그 파일 — single_usdc 가 빠져 Settings 파싱은 실패하지만 RPC 는 살아 있다.
        assert_eq!(
            rpc_url_in(
                r#"{"daily_usdc":"20","single_eth":"0.01","daily_eth":"0.05","chain_id":8453,"rpc_url":"http://127.0.0.1:8545"}"#
            ),
            "http://127.0.0.1:8545"
        );
        // 🔴 chain_id 를 못 알아보는 파일은 지정 RPC 도 버린다(코덱스 개발 57 P1) — 체인은 Sepolia 로
        // 접히는데 RPC 는 메인넷 것이면 「연습용」 화면 뒤에서 진짜 송금이 나간다.
        let custom = r#""rpc_url":"https://base-mainnet.example/v2/KEY""#;
        // 타입 틀림·null·미지원·음수 — 전부 「못 알아봄」.
        assert_eq!(
            rpc_url_in(&format!(r#"{{"chain_id":"8453",{custom}}}"#)),
            ""
        );
        assert_eq!(rpc_url_in(&format!(r#"{{"chain_id":null,{custom}}}"#)), "");
        assert_eq!(rpc_url_in(&format!(r#"{{"chain_id":1,{custom}}}"#)), ""); // 미지원
        assert_eq!(rpc_url_in(&format!(r#"{{"chain_id":-1,{custom}}}"#)), "");
        // 키 중복(코덱스 2차 P1)·실수 — Value 로 읽으면 마지막 값이 조용히 이기지만 파생은 거부한다.
        assert_eq!(
            rpc_url_in(&format!(r#"{{"chain_id":84532,"chain_id":8453,{custom}}}"#)),
            ""
        );
        assert_eq!(
            rpc_url_in(&format!(
                r#"{{"chain_id":8453,{custom},"rpc_url":"https://other.invalid"}}"#
            )),
            ""
        );
        assert_eq!(
            rpc_url_in(&format!(r#"{{"chain_id":8453.0,{custom}}}"#)),
            ""
        );
        // 지원 체인이면 유지, 필드가 없는 옛 파일도 유지(Sepolia 시절 — chain_id_in 과 같은 답).
        for id in SUPPORTED_CHAIN_IDS {
            assert_eq!(
                rpc_url_in(&format!(r#"{{"chain_id":{id},{custom}}}"#)),
                "https://base-mainnet.example/v2/KEY"
            );
        }
        assert_eq!(
            rpc_url_in(&format!("{{{custom}}}")),
            "https://base-mainnet.example/v2/KEY"
        );
        // 파일 갈래: 없음·못 읽음은 빈 값, 본문은 rpc_url_in.
        assert_eq!(rpc_url_for(&SettingsFile::Missing), "");
        assert_eq!(rpc_url_for(&SettingsFile::Unreadable), "");
        assert_eq!(
            rpc_url_for(&SettingsFile::Text(
                r#"{"rpc_url":"https://x.invalid"}"#.to_string()
            )),
            "https://x.invalid"
        );
    }

    // 🔴 구조 계약(코덱스 개발 57 1·2차 P1): **지정 RPC 를 살린 파일이면 `chain_id_in` 이 알아본 체인**
    // (지원 목록 안)이어야 한다. 한쪽만 못 읽는 파일이 하나라도 있으면 「연습용」 화면 뒤에서 메인넷
    // RPC 에 서명이 나갈 수 있다. 새 케이스는 여기에 보태라 — 두 함수가 서로 다른 파서를 쓰기 시작하면
    // 여기서 잡힌다.
    #[test]
    fn rpc_kept_implies_chain_recognized() {
        let rpc = r#""rpc_url":"https://base-mainnet.example/v2/KEY""#;
        let cases = [
            format!("{{{rpc}}}"),
            format!(r#"{{"chain_id":8453,{rpc}}}"#),
            format!(r#"{{"chain_id":84532,{rpc}}}"#),
            format!(r#"{{"chain_id":5042002,{rpc}}}"#),
            format!(r#"{{"chain_id":5042,{rpc}}}"#),
            format!(r#"{{"chain_id":1,{rpc}}}"#),
            format!(r#"{{"chain_id":"8453",{rpc}}}"#),
            format!(r#"{{"chain_id":null,{rpc}}}"#),
            format!(r#"{{"chain_id":8453.0,{rpc}}}"#),
            format!(r#"{{"chain_id":-8453,{rpc}}}"#),
            format!(r#"{{"chain_id":84532,"chain_id":8453,{rpc}}}"#),
            format!(r#"{{"chain_id":8453,"chain_id":84532,{rpc}}}"#),
            format!(r#"{{"chain_id":8453,{rpc},{rpc}}}"#),
            format!(r#"{{"single_usdc":5,"chain_id":8453,{rpc}}}"#),
            format!(r#"[{{"chain_id":8453,{rpc}}}]"#),
            "{ 깨진 JSON".to_string(),
        ];
        for text in &cases {
            let kept = !rpc_url_in(text).is_empty();
            let chain = chain_id_in(text);
            if kept {
                assert!(
                    SUPPORTED_CHAIN_IDS.contains(&chain),
                    "RPC 는 살았는데 체인은 못 알아봄: {text}"
                );
            }
        }
        // 살아남는 쪽도 실제로 있다(전부 버리는 구현이 이 검사를 공짜로 통과하지 않게).
        assert!(!rpc_url_in(&cases[1]).is_empty());
        assert!(!rpc_url_in(&cases[0]).is_empty());
    }

    // RPC 선택 (개발 49). 환경변수로 체인을 갈아탄 경우엔 settings 의 커스텀 RPC 를 버린다 —
    // 그 URL 은 **딴 체인의 엔드포인트**라 그대로 쓰면 잔액 조회가 조용히 죽는다(개발 48 실측).
    #[test]
    fn pick_rpc_drops_custom_when_env_forces_other_chain() {
        let custom = "https://base-mainnet.example/v2/KEY";
        let default = "https://sepolia.base.org";
        // 평소: 커스텀이 있으면 커스텀.
        assert_eq!(pick_rpc(custom, false, default), custom);
        // 커스텀이 비면 언제나 기본값.
        assert_eq!(pick_rpc("", false, default), default);
        assert_eq!(pick_rpc("", true, default), default);
        // 환경변수가 다른 체인을 강제 → 커스텀을 버리고 그 체인의 기본 RPC 로.
        assert_eq!(pick_rpc(custom, true, default), default);
    }

    // 파일 → 세 갈래. Unreadable 은 「디렉터리를 파일처럼 읽기」로 만든다(권한 조작 없이 재현).
    #[test]
    fn settings_file_read_classifies_io() {
        let dir = std::env::temp_dir().join(format!("jigap-policy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            SettingsFile::read(&dir.join("settings.json")),
            SettingsFile::Missing
        );
        std::fs::write(dir.join("settings.json"), r#"{"chain_id":8453}"#).unwrap();
        assert_eq!(
            SettingsFile::read(&dir.join("settings.json")),
            SettingsFile::Text(r#"{"chain_id":8453}"#.to_string())
        );
        assert_eq!(SettingsFile::read(&dir), SettingsFile::Unreadable);
        // 지갑 유무: 둘 중 하나만 있어도 true.
        assert!(!wallet_exists_in(&dir));
        std::fs::write(dir.join("wallet.json"), "{}").unwrap();
        assert!(wallet_exists_in(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // 체인별·계정별 파일 이름: 기본 체인·계정 0 은 기존 이름 그대로(무손실), 그 외는 접미.
    #[test]
    fn data_file_names_keep_defaults_and_suffix_others() {
        assert_eq!(chain_file_name(BASE_SEPOLIA_ID, "history"), "history.json");
        assert_eq!(
            chain_file_name(BASE_MAINNET_ID, "history"),
            "history-8453.json"
        );
        assert_eq!(
            chain_file_name(ARC_TESTNET_ID, "spend"),
            "spend-5042002.json"
        );
        // Arc 메인넷(5042)은 테스트넷(5042002)과 접두가 같다 — 접미사가 통째로 달라야 파일이 안 섞인다.
        assert_eq!(chain_file_name(ARC_MAINNET_ID, "spend"), "spend-5042.json");
        assert_ne!(
            chain_file_name(ARC_MAINNET_ID, "history"),
            chain_file_name(ARC_TESTNET_ID, "history")
        );
        assert_eq!(account_file_name("history.json", 0), "history.json");
        assert_eq!(
            account_file_name("history-8453.json", 0),
            "history-8453.json"
        );
        assert_eq!(account_file_name("history.json", 2), "history-a2.json");
        assert_eq!(
            account_file_name("history-8453.json", 3),
            "history-8453-a3.json"
        );
        // 둘을 이어 쓰는 실제 경로.
        assert_eq!(
            account_file_name(&chain_file_name(BASE_MAINNET_ID, "history"), 2),
            "history-8453-a2.json"
        );
    }

    // 계정 정규화: 옛 파일(목록 없음)은 계정 0 하나, 주소는 address 필드.
    #[test]
    fn legacy_wallet_normalizes_to_single_account() {
        let list = normalize_accounts("0xAbc", &[]);
        assert_eq!(
            list,
            vec![Account {
                index: 0,
                address: "0xAbc".into(),
                label: String::new()
            }]
        );
        assert_eq!(pick_active(&list, 0).address, "0xAbc");
        assert_eq!(pick_active(&list, 7).index, 0); // 없는 계정 → 0
    }

    // 인덱스 순 정렬, 계정 0 의 주소는 address 필드가 이긴다(라벨은 목록 것), 중복 인덱스는 하나로,
    // 활성이 목록에 없으면 계정 0.
    #[test]
    fn accounts_sorted_zero_from_address_and_active_fallback() {
        let listed = vec![
            Account {
                index: 2,
                address: "0xTwo".into(),
                label: "AI".into(),
            },
            Account {
                index: 0,
                address: "0xStale".into(),
                label: "나".into(),
            },
            Account {
                index: 1,
                address: "0xOne".into(),
                label: String::new(),
            },
            Account {
                index: 2,
                address: "0xDup".into(),
                label: String::new(),
            },
        ];
        let list = normalize_accounts("0xZero", &listed);
        assert_eq!(
            list.iter().map(|a| a.index).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(list[0].address, "0xZero"); // 정본은 address 필드
        assert_eq!(list[0].label, "나");
        assert_eq!(list[2].address, "0xTwo"); // 중복은 먼저 온 것(안정 정렬)
        assert_eq!(pick_active(&list, 2).address, "0xTwo");
        assert_eq!(pick_active(&list, 9).index, 0);
    }

    // 🔴 결제 시도 기록 (개발 66) — 다시 승인해도 되는 건 「기록 없음」과 「확실한 실패」뿐.
    #[test]
    fn attempt_retry_and_timeout_rules() {
        let mk = |state: &str| AttemptRecord {
            v: 1,
            id: "1".into(),
            state: state.into(),
            kind: "transfer".into(),
            chain_id: 5042,
            started: 1,
            updated: 1,
            status: String::new(),
            tx_hash: String::new(),
            detail: String::new(),
            x402: None,
        };
        assert!(attempt_allows_retry(None));
        assert!(attempt_allows_retry(Some(&mk(ATTEMPT_FAILED))));
        for s in [ATTEMPT_SENDING, ATTEMPT_DONE, ATTEMPT_UNKNOWN, "weird"] {
            assert!(
                !attempt_allows_retry(Some(&mk(s))),
                "{s} 는 다시 승인하면 안 된다"
            );
        }
        assert_eq!(after_timeout(None), AfterTimeout::NothingSent);
        assert_eq!(
            after_timeout(Some(&mk(ATTEMPT_FAILED))),
            AfterTimeout::NothingSent
        );
        assert_eq!(
            after_timeout(Some(&mk(ATTEMPT_SENDING))),
            AfterTimeout::StillSending
        );
        assert_eq!(
            after_timeout(Some(&mk(ATTEMPT_DONE))),
            AfterTimeout::Finished
        );
        assert_eq!(
            after_timeout(Some(&mk(ATTEMPT_UNKNOWN))),
            AfterTimeout::Finished
        );
        // 옛 필드만 있는 기록도 읽힌다(결과 필드는 기본값).
        let r: AttemptRecord = serde_json::from_str(
            r#"{"v":1,"id":"9","state":"sending","kind":"x402","chain_id":1,"started":1,"updated":1}"#,
        )
        .unwrap();
        assert_eq!(r.status, "");
        assert!(r.x402.is_none());
    }

    // id 가 경로를 벗어나지 못한다.
    #[test]
    fn attempt_paths_refuse_odd_ids() {
        let d = Path::new("/h/.jigap");
        assert_eq!(
            attempt_path(d, "1789").unwrap(),
            PathBuf::from("/h/.jigap/approvals/1789.json")
        );
        assert_eq!(
            proof_path(d, "1789").unwrap(),
            PathBuf::from("/h/.jigap/approvals/1789.proof.json")
        );
        for bad in ["", "../x", "a/b", "1.json", &"9".repeat(65)] {
            assert!(attempt_path(d, bad).is_none(), "{bad}");
        }
        assert!(
            !lease_lapsed(false, Some(10_000)),
            "옛 MCP 요청엔 임대 규칙이 없다"
        );
        assert!(
            !lease_lapsed(true, None),
            "수정 시각을 못 읽으면 끊겼다고 단정하지 않는다"
        );
        assert!(!lease_lapsed(true, Some(REQUEST_LEASE_SECS)));
        assert!(lease_lapsed(true, Some(REQUEST_LEASE_SECS + 1)));
        assert!(!attempt_prunable(APPROVAL_KEEP_SECS));
        assert!(attempt_prunable(APPROVAL_KEEP_SECS + 1));
    }
}
