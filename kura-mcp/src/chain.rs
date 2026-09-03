// 체인 설정 (MCP 측) — 잔액 조회·x402 결제 제출·익스플로러 링크에 쓰는 값을 한 묶음으로.
//
// src-tauri/src/chain.rs 와 평행한 별도 사본이다(두 크레이트는 의도적으로 분리 —
// 공유 크레이트를 만들지 않는 게 이 프로젝트의 정책: Tauri 빌드 위험 0). 활성 체인은 GUI 와
// 공유하는 settings.json 의 chain_id 로 **런타임 선택**한다(두 프로세스가 자동으로 같은 체인).
// 그 선택 판정과 체인 ID·데이터 파일 이름 규칙은 shared/policy.rs 를 GUI 와 **같은 소스로**
// 컴파일한다(개발 56) — 사본으로 남는 건 ChainConfig 의 나머지 값(RPC·주소·x402 표기)뿐이다.

use alloy::primitives::{address, Address};

use crate::policy;

/// 한 체인(EVM)의 MCP 측 설정 묶음.
#[derive(Clone, Copy)]
pub struct ChainConfig {
    /// EIP-155 체인 ID — settings.json 의 chain_id 와 매칭 + 체인별 데이터 파일 접미사에 쓴다.
    pub chain_id: u64,
    /// settings.json(GUI와 공유)의 rpc_url 이 비어 있을 때의 폴백 공개 RPC.
    pub default_rpc: &'static str,
    /// 이 체인의 Circle USDC 컨트랙트.
    pub usdc_address: Address,
    /// USDC 소수 자릿수 (base unit ↔ 십진 변환에 쓴다).
    pub usdc_decimals: u8,
    /// USDC(FiatToken)의 EIP-712 도메인 이름/버전 — src-tauri/src/chain.rs 와 같은 값.
    /// MCP 는 서명하지 않지만, 서버가 결제 요구에 붙여 보내는 `extra`(어느 도메인에 서명하라)가
    /// 우리가 실제로 서명할 도메인과 같은지 대조하는 데 쓴다 (개발 50 — x402 extra 가드).
    pub usdc_eip712_name: &'static str,
    pub usdc_eip712_version: &'static str,
    /// x402 네트워크 표기 — V1 단축명(있는 체인만)과 V2 CAIP-2 (서버가 둘 중 하나로 제시).
    /// `None` = 이 체인엔 통용되는 V1 단축명이 없다(Arc). 빈 문자열로 두면 `network` 가 없는
    /// 요구가 빈 문자열과 같다고 판정돼 **아무 체인 요구나 통과**하므로 Option 이어야 한다.
    pub x402_network_v1: Option<&'static str>,
    pub x402_network_caip2: &'static str,
    /// **네이티브(가스) 토큰이 위 USDC 와 같은 자산인가** (개발 50, Arc). 자세한 배경은
    /// src-tauri/src/chain.rs 의 같은 필드 주석 참고 — 한 잔액에 인터페이스가 둘(18dp/6dp)이라
    /// "USDC + 가스 토큰"을 따로 세면 같은 돈을 두 번 세게 된다. MCP 는 잔액 보고 문구에 쓴다.
    pub native_is_usdc: bool,
    /// 익스플로러 트랜잭션 URL 접두사 (뒤에 tx 해시를 붙인다).
    pub explorer_tx_prefix: &'static str,
    /// ERC-8004 IdentityRegistry (개발 47). `None` = 이 체인엔 레지스트리가 없다 →
    /// 에이전트 신원 조회를 아예 안 한다(예: 아직 배포 안 된 신규 체인).
    pub erc8004_identity: Option<Address>,
    /// ERC-8004 ReputationRegistry — 피드백 클라이언트 수 조회용. 없으면 피드백은 0으로 둔다.
    pub erc8004_reputation: Option<Address>,
}

/// Base Sepolia (테스트넷). 기본 체인 — 데이터 파일이 접미사 없이 저장되는 "원본" 체인.
pub const BASE_SEPOLIA: ChainConfig = ChainConfig {
    chain_id: policy::BASE_SEPOLIA_ID,
    default_rpc: "https://sepolia.base.org",
    usdc_address: address!("0x036CbD53842c5426634e7929541eC2318f3dCF7e"),
    usdc_decimals: 6,
    usdc_eip712_name: "USDC",
    usdc_eip712_version: "2",
    x402_network_v1: Some("base-sepolia"),
    x402_network_caip2: "eip155:84532",
    explorer_tx_prefix: "https://sepolia.basescan.org/tx/",
    erc8004_identity: Some(address!("0x8004A818BFB912233c491871b3d84c89A494BD9e")),
    erc8004_reputation: Some(address!("0x8004B663056A597Dffe9eCcC1965A193B7388713")),
    native_is_usdc: false,
};

/// Base 메인넷 (실제 자금). x402 네트워크명은 "base" / CAIP-2 "eip155:8453".
pub const BASE_MAINNET: ChainConfig = ChainConfig {
    chain_id: policy::BASE_MAINNET_ID,
    default_rpc: "https://mainnet.base.org",
    usdc_address: address!("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
    usdc_decimals: 6,
    usdc_eip712_name: "USD Coin",
    usdc_eip712_version: "2",
    x402_network_v1: Some("base"),
    x402_network_caip2: "eip155:8453",
    explorer_tx_prefix: "https://basescan.org/tx/",
    erc8004_identity: Some(address!("0x8004A169FB4a3325136EB29fA0ceB6D2e539a432")),
    erc8004_reputation: Some(address!("0x8004BAa17C55a88189AE136b182e5fdA19dE9b63")),
    native_is_usdc: false,
};

/// Arc 테스트넷 (Circle L1, 개발 50). USDC 가 네이티브 가스 토큰인 체인.
///
/// x402: **V1 단축명이 없다** — 이 체인을 아는 서버는 CAIP-2 `eip155:5042002` 로만 제시한다.
/// ⚠️ 개발 50 시점에 이 네트워크를 지원하는 페이실리테이터는 Circle Gateway 하나뿐인데, 그쪽은
/// USDC EIP-3009 가 아니라 **GatewayWallet 도메인**에 서명하라고 요구한다(우리 형식과 다름).
/// 그래서 Arc 결제는 "서명 경로는 서 있지만 받아 줄 상대가 아직 없다" 상태다 — 서버가 우리 형식으로
/// 제시하면 그대로 동작하고, Gateway 형식으로 제시하면 x402.rs 의 extra 가드가 걸러 낸다.
///
/// ERC-8004 레지스트리는 **Base Sepolia 와 같은 주소로 Arc 테스트넷에도 있다**(결정론적 배포).
/// 개발 50 에서 두 주소 모두 Arc RPC 로 `getVersion()` = "2.0.0" 실응답 확인 — 그래서 Some 이다.
pub const ARC_TESTNET: ChainConfig = ChainConfig {
    chain_id: policy::ARC_TESTNET_ID,
    default_rpc: "https://rpc.testnet.arc.network",
    usdc_address: address!("0x3600000000000000000000000000000000000000"),
    usdc_decimals: 6,
    usdc_eip712_name: "USDC",
    usdc_eip712_version: "2",
    x402_network_v1: None,
    x402_network_caip2: "eip155:5042002",
    explorer_tx_prefix: "https://testnet.arcscan.app/tx/",
    erc8004_identity: Some(address!("0x8004A818BFB912233c491871b3d84c89A494BD9e")),
    erc8004_reputation: Some(address!("0x8004B663056A597Dffe9eCcC1965A193B7388713")),
    native_is_usdc: true,
};

/// 사용자가 선택한 체인 ID. 파일 없음(첫 실행)=메인넷, 깨짐/옛 설정=테스트넷 — 본문 주석 참고.
fn selected_chain_id() -> u64 {
    env_chain_id().unwrap_or_else(settings_chain_id)
}

/// `KURA_CHAIN_ID` 로 강제된 체인 ID (없으면 None). 잘못된 값이면 즉시 종료한다.
fn env_chain_id() -> Option<u64> {
    // 환경변수 우선 — 테스트·스크립트가 사용자의 라이브 settings.json 에 의존하지 않게 체인을 고정한다.
    // GUI 와 다른 (정상) 체인을 가리키더라도, 결제 요청에 각인된 chain_id 를 GUI 가 승인 시 대조해
    // 거부하므로(개발 20 가드) 잘못된 체인으로 송금되지 않는다. 단, **잘못/미지원 값은 조용히
    // settings 로 폴백하지 않는다** — 오타(예: "84532x")가 사용자 의도와 다른 체인(메인넷=실돈)으로
    // 조용히 동작하는 footgun 을 막으려고, 명시 override 가 유효하지 않으면 즉시 종료한다.
    if let Some(v) = std::env::var("KURA_CHAIN_ID")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        let id = v.parse::<u64>().unwrap_or_else(|_| {
            eprintln!("KURA_CHAIN_ID is not an integer: {v:?}");
            std::process::exit(1);
        });
        if id != BASE_SEPOLIA.chain_id && id != BASE_MAINNET.chain_id && id != ARC_TESTNET.chain_id
        {
            eprintln!(
                "KURA_CHAIN_ID={id} is not a supported chain (supported: {} / {} / {}).",
                BASE_SEPOLIA.chain_id, BASE_MAINNET.chain_id, ARC_TESTNET.chain_id
            );
            std::process::exit(1);
        }
        return Some(id);
    }
    None
}

/// settings.json 이 말하는 체인 ID (환경변수를 보지 않는다).
fn settings_chain_id() -> u64 {
    // 단위 테스트는 실제 ~/.jigap/settings.json 을 읽지 않는다 — 사용자의 지갑 설정(메인넷 등)에
    // 테스트 결과가 좌우되지 않게 기본 체인(Base Sepolia)으로 고정해 항상 결정론적이게 한다.
    #[cfg(test)]
    {
        BASE_SEPOLIA.chain_id
    }
    #[cfg(not(test))]
    {
        // 판정은 GUI(src-tauri chain.rs selected_chain_id)와 **같은 함수** `policy::chain_id_for`
        // — 두 프로세스가 여기서 어긋나면 잔액·결제가 서로 다른 체인을 본다. 폴백 네 갈래
        // (신규=메인넷 / 설정 파일 없는 기존 지갑·깨짐·못 읽음·옛 파일=테스트넷)의 이유는 그쪽 주석.
        let Ok(dir) = crate::wallet::jigap_dir() else {
            return BASE_SEPOLIA.chain_id; // 홈을 못 정함 = 못 읽음(보수적)
        };
        let file = policy::SettingsFile::read(&dir.join("settings.json"));
        policy::chain_id_for(&file, || policy::wallet_exists_in(&dir))
    }
}

/// **환경변수가 settings 와 다른 체인을 강제하고 있는가** (개발 49).
///
/// `KURA_CHAIN_ID` 는 체인만 바꾸고 `settings.json` 의 `rpc_url` 은 그대로 뒀다 → 커스텀 RPC
/// (또는 프리셋 RPC)를 쓰는 사람이 이 환경변수로 체인을 넘기면 **딴 체인의 RPC 에 이 체인의
/// 컨트랙트를 묻는 꼴**이 되어 잔액 조회가 `returned no data ("0x")` 로 죽는다(개발 48 실측).
/// GUI 로 체인을 바꾸면 프리셋이 새 체인으로 재매핑되므로 정상 — 환경변수 경로만의 함정이다.
///
/// 두 체인이 **같으면 false** — 그때 rpc_url 은 이 체인의 것이 맞으니 그대로 존중한다.
pub fn env_forces_other_chain() -> bool {
    match env_chain_id() {
        Some(id) => id != settings_chain_id(),
        None => false,
    }
}

/// 현재 활성 체인 — settings.json 의 chain_id 로 런타임 선택(GUI 와 동일 파일 → 자동 일치).
pub fn active_chain() -> ChainConfig {
    match selected_chain_id() {
        id if id == BASE_MAINNET.chain_id => BASE_MAINNET,
        id if id == ARC_TESTNET.chain_id => ARC_TESTNET,
        _ => BASE_SEPOLIA,
    }
}

/// 체인별로 분리되는 데이터 파일 이름(history/x402_settlements) — 규칙은 `policy::chain_file_name`
/// (src-tauri 의 chain_file 과 같은 함수). 두 프로세스가 같은 settings.json 을 같은 판정으로 읽으므로
/// GUI 와 MCP 가 항상 같은 파일을 가리킨다. active_chain() 으로 정규화(미지원 id 는 Sepolia 폴백)한
/// id 를 넘겨 활성 체인과 파일이 어긋나지 않게 한다.
pub fn chain_file(stem: &str) -> String {
    policy::chain_file_name(active_chain().chain_id, stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 레지스트리 주소를 문자로 못박는다 — 하드코딩한 컨트랙트 주소의 오타는 조용히
    /// **엉뚱한 컨트랙트를 신뢰 근거로 읽는** 실패라, 눈으로 못 잡는다.
    /// 값 출처: erc-8004/erc-8004-contracts 공식 배포 목록 + 개발 47에서 두 체인 모두
    /// 온체인 `getVersion()` = "2.0.0" 실응답으로 교차 확인.
    #[test]
    fn erc8004_registry_addresses_are_pinned() {
        assert_eq!(
            BASE_MAINNET.erc8004_identity.unwrap().to_string(),
            "0x8004A169FB4a3325136EB29fA0ceB6D2e539a432"
        );
        assert_eq!(
            BASE_MAINNET.erc8004_reputation.unwrap().to_string(),
            "0x8004BAa17C55a88189AE136b182e5fdA19dE9b63"
        );
        assert_eq!(
            BASE_SEPOLIA.erc8004_identity.unwrap().to_string(),
            "0x8004A818BFB912233c491871b3d84c89A494BD9e"
        );
        assert_eq!(
            BASE_SEPOLIA.erc8004_reputation.unwrap().to_string(),
            "0x8004B663056A597Dffe9eCcC1965A193B7388713"
        );
        // 두 체인의 레지스트리는 서로 다르다(체인별 배포) — 한쪽을 복사하다 생기는 사고 방지.
        assert_ne!(BASE_MAINNET.erc8004_identity, BASE_SEPOLIA.erc8004_identity);
        // Arc 테스트넷은 **Base Sepolia 와 같은 주소**다(결정론적 배포 — 개발 50 에서 Arc RPC 로
        // getVersion()="2.0.0" 실응답 확인). 우연히 같은 게 아니라 그래야 맞는 값이다.
        assert_eq!(ARC_TESTNET.erc8004_identity, BASE_SEPOLIA.erc8004_identity);
        assert_eq!(
            ARC_TESTNET.erc8004_reputation,
            BASE_SEPOLIA.erc8004_reputation
        );
    }

    /// Arc 테스트넷 상수 회귀 가드 (개발 50). 전부 라이브 RPC 실응답을 옮겨 적은 값.
    #[test]
    fn arc_testnet_constants_are_pinned() {
        assert_eq!(ARC_TESTNET.chain_id, 5_042_002);
        assert_eq!(
            ARC_TESTNET.usdc_address.to_string().to_lowercase(),
            "0x3600000000000000000000000000000000000000"
        );
        // 🔴 ERC-20 뷰는 6dp. 네이티브 뷰(18dp)를 쓰면 금액이 1조 배 어긋난다.
        assert_eq!(ARC_TESTNET.usdc_decimals, 6);
        assert_eq!(ARC_TESTNET.usdc_eip712_name, "USDC");
        assert_eq!(ARC_TESTNET.usdc_eip712_version, "2");
        const { assert!(ARC_TESTNET.native_is_usdc) };
        // V1 단축명이 없는 체인 — CAIP-2 로만 매칭한다(빈 문자열이 아니라 None 이어야 하는 이유는
        // x402::network_supported 주석 참고).
        assert_eq!(ARC_TESTNET.x402_network_v1, None);
        assert_eq!(ARC_TESTNET.x402_network_caip2, "eip155:5042002");
        const { assert!(!BASE_SEPOLIA.native_is_usdc) };
        const { assert!(!BASE_MAINNET.native_is_usdc) };
    }

    // policy::SUPPORTED_CHAIN_IDS 와 이 크레이트의 체인 셋(active_chain 의 match·env_chain_id 의 검사)이
    // 같은 집합이어야 한다(개발 57) — policy 는 이 목록으로 「chain_id 를 못 알아보는 파일의 지정 RPC 를
    // 버릴지」 정한다. src-tauri 의 같은 테스트와 짝.
    #[test]
    fn supported_chain_ids_match_this_crate() {
        let mine = [BASE_SEPOLIA, BASE_MAINNET, ARC_TESTNET].map(|c| c.chain_id);
        assert_eq!(mine.len(), policy::SUPPORTED_CHAIN_IDS.len());
        for id in policy::SUPPORTED_CHAIN_IDS {
            assert!(
                mine.contains(&id),
                "policy 엔 있는데 이 크레이트엔 없다: {id}"
            );
        }
    }
}
