// ~/.jigap 파일을 읽는 **규칙의 정본** — GUI(src-tauri)와 MCP·CLI(kura-mcp)가 같은 소스 파일을
// 컴파일한다 (개발 56).
//
// 크레이트가 아니다. 두 크레이트의 lib.rs 가 `#[path = "../../shared/policy.rs"] mod policy;` 로
// 이 파일을 제 모듈로 끌어들인다 → Cargo 의존성·워크스페이스 변화 0(「공유 크레이트를 만들지
// 않는다 — Tauri 빌드 위험 0」 정책은 그대로), 그러나 정본은 하나다. 개발 52·54 에 생긴 「같은 파일을
// 두 곳(세 곳)에서 다르게 읽는」 뿌리를 여기서 뽑는다:
//   - settings.json → 어느 체인인가 (chain.rs 의 단독 읽기 vs settings.rs 의 Settings 파싱, + MCP 사본)
//   - settings.json → 사용자 지정 RPC (app 은 Settings 전체 파싱, MCP 는 rpc_url 단독 읽기 — 개발 57)
//   - wallet.enc   → 계정 목록·활성 계정 정규화 (app EncryptedWallet vs MCP EncMeta)
//   - 체인별·계정별 데이터 파일 이름 (chain_file · account_file_name, 양쪽 사본)
//   - 「지갑이 이미 있는가」 (설정·체인·언어 기본값이 전부 이 질문으로 신규/기존을 가른다)
//
// 여기 두는 것의 조건: **IO 판정과 순수 규칙만.** i18n 매크로(`ts!`/`tf!`)·체인 상수 묶음(ChainConfig —
// 공식 RPC 주소 포함)·에러 문구는 각 크레이트가 계속 따로 가진다 — 이 파일은 두 크레이트 어느 쪽의 모듈도 참조하지
// 않아야 양쪽에서 그대로 컴파일된다(의존은 std + serde + serde_json 만).

use serde::{Deserialize, Serialize};
use std::path::Path;

// ── 체인 ID 정본 ────────────────────────────────────────────────────────────────────────────
// 두 크레이트의 ChainConfig 상수가 이 값을 쓴다. 폴백 판정(아래 chain_id_for)이 테스트넷/메인넷을
// 가리키므로 최소 그 둘은 여기 있어야 하고, Arc 는 짝을 맞추려고 함께 둔다.

/// Base Sepolia (테스트넷) — 기본 체인. 데이터 파일이 접미사 없이 저장되는 「원본」 체인.
pub const BASE_SEPOLIA_ID: u64 = 84_532;
/// Base 메인넷 (실제 자금). 진짜 신규 설치의 기본(개발 39).
pub const BASE_MAINNET_ID: u64 = 8453;
/// Arc 테스트넷 (Circle L1, 개발 50).
pub const ARC_TESTNET_ID: u64 = 5_042_002;

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

/// settings.json 에서 사용자 지정 RPC 만 읽는 가벼운 뷰 — ChainSel 과 같은 이유로 **다른 필드는
/// 무시**한다. 한도 필드 하나가 깨진 파일에서 GUI 는 Settings 파싱에 실패해 공식 RPC 로 접고 MCP 는
/// 이 필드만 읽어 커스텀 RPC 를 쓰던 것(개발 51 하네스 실측·개발 56 대체 리뷰 P3)이 두 판정의 뿌리였다.
#[derive(Deserialize, Default)]
struct RpcSel {
    #[serde(default)]
    rpc_url: String,
}

/// settings.json 본문 → 사용자 지정 RPC(앞뒤 공백 제거). 없거나 비었거나 깨졌으면 빈 값 =
/// 「활성 체인의 공식 RPC 를 따라간다」. 값은 검사하지 않는다 — http(s) 여부는 저장 경로(set_settings)
/// 의 몫이고, 이미 파일에 있는 값은 어느 프로세스든 **같은 값**을 써야 한다.
pub fn rpc_url_in(text: &str) -> String {
    serde_json::from_str::<RpcSel>(text)
        .map(|s| s.rpc_url.trim().to_string())
        .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
