// 체인 설정 — 한 체인의 모든 식별 정보(RPC·USDC·chainId·익스플로러·x402 네트워크명)를
// ChainConfig 한 묶음으로 모은다. 체인 추가 = 아래에 const 항목 하나 더. 활성 체인은 더 이상
// 컴파일 고정이 아니라 settings.json 의 chain_id 로 **런타임 선택**한다(테스트넷↔메인넷 토글).

use alloy::primitives::{address, Address};
use alloy::sol;
use serde::Deserialize;

use crate::store::jigap_dir;

/// 한 체인(EVM)의 백엔드 설정 묶음 — 잔액 조회·송금·x402 서명에 쓰는 값만 담는다.
/// (표시 이름·익스플로러 URL 같은 "보여주기" 값은 백엔드가 안 쓰므로 프론트 src/lib/chain.ts
///  와 MCP kura-mcp/src/chain.rs 가 각자 보유한다. 세 레이어가 같은 체인을 평행하게 기술.)
/// Copy — 작고 모든 필드가 Copy(주소·정수·&'static str)라 값으로 들고 다녀도 가볍다.
#[derive(Clone, Copy)]
pub(crate) struct ChainConfig {
    /// EIP-712 도메인에 들어가는 체인 ID. 서명 재생(다른 체인 재사용)을 막는다.
    pub(crate) chain_id: u64,
    /// 설정(rpc_url)이 비어 있을 때의 폴백 공개 RPC.
    pub(crate) default_rpc: &'static str,
    /// 이 체인의 Circle USDC 컨트랙트.
    pub(crate) usdc_address: Address,
    /// USDC 소수 자릿수 (Circle USDC는 모든 체인에서 6, 단 체인 종속값으로 묶어 둔다).
    pub(crate) usdc_decimals: u8,
    /// USDC(FiatToken)의 EIP-712 도메인 이름/버전. 서명 도메인이 컨트랙트와 일치해야
    /// transferWithAuthorization 으로 정산된다 (x402_domain_matches_usdc_onchain 테스트가 검증).
    pub(crate) usdc_eip712_name: &'static str,
    pub(crate) usdc_eip712_version: &'static str,
}

/// Base Sepolia (테스트넷). 기본 체인 — 데이터 파일이 접미사 없이 저장되는 "원본" 체인이기도 하다.
pub(crate) const BASE_SEPOLIA: ChainConfig = ChainConfig {
    chain_id: 84_532,
    default_rpc: "https://sepolia.base.org",
    usdc_address: address!("0x036CbD53842c5426634e7929541eC2318f3dCF7e"),
    usdc_decimals: 6,
    usdc_eip712_name: "USDC",
    usdc_eip712_version: "2",
};

/// Base 메인넷 (실제 자금). USDC EIP-712 도메인 name 은 Sepolia("USDC")와 달리 "USD Coin" 이다
/// (온체인 name() 확인: Base 메인넷 USDC=0x8335…2913 의 토큰명은 "USD Coin"). 이 값이 틀리면
/// 서명 도메인이 컨트랙트와 안 맞아 정산이 거부된다 → x402_domain_matches_usdc_onchain 가 양 체인 검증.
pub(crate) const BASE_MAINNET: ChainConfig = ChainConfig {
    chain_id: 8453,
    default_rpc: "https://mainnet.base.org",
    usdc_address: address!("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
    usdc_decimals: 6,
    usdc_eip712_name: "USD Coin",
    usdc_eip712_version: "2",
};

/// settings.json 에서 선택된 체인 ID만 읽는 가벼운 뷰(다른 필드는 무시). settings.rs 의 Settings 를
/// 통째로 역직렬화하지 않는 이유 = settings → chain 모듈 순환 의존을 피하려고(MCP wallet.rs 와 같은 패턴).
#[derive(Deserialize)]
struct ChainSel {
    chain_id: u64,
}

tokio::task_local! {
    /// 한 작업(송금/서명/승인) 동안 고정된 체인 ID. with_pinned_chain 으로 설정하면 그 작업의 모든
    /// active_chain()/chain_file()/effective_rpc 호출이 이 값을 본다 — 작업 도중 settings.json 이
    /// 바뀌어도 흔들리지 않게(코덱스 개발20 #1: 한 결제 안에서 장부·전송·서명 체인이 갈라지는 것 차단).
    static PINNED_CHAIN: u64;
}

/// 작업 단위로 체인을 고정해 fut 를 실행한다 — 진입 시 한 번 정한 chain_id 로 전체 작업을 묶는다.
/// 같은 태스크 내 .await 를 넘어 유지되므로(중간에 spawn 안 함), 장부·RPC·서명·내역이 같은 체인을 본다.
pub(crate) async fn with_pinned_chain<F: std::future::Future>(chain_id: u64, fut: F) -> F::Output {
    PINNED_CHAIN.scope(chain_id, fut).await
}

/// 사용자가 선택한 체인 ID. 작업이 체인을 고정했으면 그 값, 아니면 settings.json.
///
/// 폴백은 settings::read_settings 와 같은 결로 갈라진다(개발 39 — 신규 기본이 메인넷이
/// 되면서 "없음"과 "못 읽음"이 다른 답이 됐다):
/// - 파일 **없음** = 첫 실행 → 메인넷(신규 기본, `Settings::default()` 와 일치)
/// - 파일이 있는데 깨짐/못 읽음/홈 못 정함 → 테스트넷(보수적 — 기존 사용자를 조용히
///   실돈 체인으로 옮기지 않는다)
/// - 정상 JSON 인데 chain_id 없음(개발 20 이전 옛 파일) → 테스트넷(그 시절 사용자 보존)
fn selected_chain_id() -> u64 {
    if let Ok(id) = PINNED_CHAIN.try_with(|id| *id) {
        return id; // 작업이 고정한 체인 — 도중에 settings 가 바뀌어도 불변
    }
    let Ok(dir) = jigap_dir() else {
        return BASE_SEPOLIA.chain_id;
    };
    match std::fs::read_to_string(dir.join("settings.json")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BASE_MAINNET.chain_id,
        Err(_) => BASE_SEPOLIA.chain_id,
        Ok(text) => chain_id_in(&text),
    }
}

/// settings.json 본문에서 chain_id 를 뽑는다(깨졌거나 필드가 없으면 테스트넷).
/// selected_chain_id 의 파싱 판단만 떼어낸 순수 함수 (IO 없이 테스트하려고).
fn chain_id_in(text: &str) -> u64 {
    serde_json::from_str::<ChainSel>(text)
        .map(|c| c.chain_id)
        .unwrap_or(BASE_SEPOLIA.chain_id)
}

/// 체인 ID로 ChainConfig 를 찾는다. 미지원이면 None. (set_settings 가 들어온 설정의 chain_id 를
/// 그 체인 기준으로 검증할 때 쓴다 — 저장 전 활성 체인이 아니라.)
pub(crate) fn chain_by_id(id: u64) -> Option<ChainConfig> {
    match id {
        id if id == BASE_SEPOLIA.chain_id => Some(BASE_SEPOLIA),
        id if id == BASE_MAINNET.chain_id => Some(BASE_MAINNET),
        _ => None,
    }
}

/// 현재 활성 체인 — settings.json 의 chain_id 로 런타임 선택. 알 수 없는 ID는 Base Sepolia 로 폴백.
pub(crate) fn active_chain() -> ChainConfig {
    chain_by_id(selected_chain_id()).unwrap_or(BASE_SEPOLIA)
}

/// 체인별로 분리해야 하는 데이터 파일 이름(spend/history/x402_settlements/trusted).
/// 기본 체인(Base Sepolia)은 **기존 이름 그대로**(예: "history.json") → 무손실 마이그레이션(접미사 없음).
/// 그 외 체인은 "-{chain_id}" 접미사("history-8453.json") → 테스트넷/메인넷의 사용액·내역·신뢰목록이
/// 절대 섞이지 않게(테스트넷 1 USDC 가 메인넷 일일 한도를 깎거나, 내역이 딴 체인인 척 보이는 것 차단).
/// **active_chain() 과 동일하게 정규화**한다(미지원/손상 chain_id 는 Base Sepolia 로) — 그래야 알 수 없는
/// id 가 들어와도 활성 체인(Sepolia로 폴백)과 데이터 파일이 어긋나 한도·신뢰목록을 조용히 리셋하지 않는다.
pub(crate) fn chain_file(stem: &str) -> String {
    match active_chain().chain_id {
        id if id == BASE_SEPOLIA.chain_id => format!("{stem}.json"),
        id => format!("{stem}-{id}.json"),
    }
}

sol! {
    #[sol(rpc)]
    interface IERC20 {
        function balanceOf(address owner) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
        // EIP-712 도메인 세퍼레이터 — 우리가 만든 도메인이 컨트랙트와 일치하는지 검증용.
        function DOMAIN_SEPARATOR() external view returns (bytes32);
    }

    // EIP-3009 결제 인가 — x402 "exact" 스킴이 오프체인으로 서명하는 구조체.
    // 필드명·순서·타입이 USDC(FiatToken) 컨트랙트가 기대하는 것과 정확히 같아야
    // 서명이 transferWithAuthorization 으로 정산된다.
    struct TransferWithAuthorization {
        address from;
        address to;
        uint256 value;
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 메인넷 상수 회귀 가드(비네트워크) — 오타로 도메인 name/주소/체인ID가 바뀌면 일반 cargo test 에서
    // 즉시 실패. (온체인 DOMAIN_SEPARATOR() 일치는 x402_domain_matches_usdc_onchain 라이브 테스트가 본다.)
    #[test]
    fn mainnet_constants_are_pinned() {
        assert_eq!(BASE_MAINNET.chain_id, 8453);
        assert_eq!(BASE_MAINNET.usdc_decimals, 6);
        // 🔴 메인넷 USDC 도메인 name 은 "USD Coin" (Sepolia "USDC" 와 다름) — 틀리면 서명이 정산 거부됨.
        assert_eq!(BASE_MAINNET.usdc_eip712_name, "USD Coin");
        assert_eq!(BASE_MAINNET.usdc_eip712_version, "2");
        assert_eq!(
            BASE_MAINNET.usdc_address,
            address!("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913")
        );
        assert_eq!(BASE_SEPOLIA.chain_id, 84_532);
        // chain_by_id 정규화: 미지원 ID는 None(→ active_chain/chain_file 이 Sepolia 로 폴백).
        assert!(chain_by_id(1).is_none());
        assert_eq!(chain_by_id(8453).unwrap().chain_id, 8453);
        assert_eq!(chain_by_id(84_532).unwrap().chain_id, 84_532);
    }

    // settings.json 본문 → chain_id 해석 (개발 39). 깨진 JSON·필드 없는 옛 파일은
    // 테스트넷으로 접는다 — "파일 없음 = 메인넷"은 selected_chain_id 의 IO 분기가 맡는다.
    #[test]
    fn chain_id_in_falls_back_conservatively() {
        assert_eq!(chain_id_in(r#"{"chain_id":8453}"#), BASE_MAINNET.chain_id);
        assert_eq!(chain_id_in(r#"{"chain_id":84532}"#), BASE_SEPOLIA.chain_id);
        assert_eq!(chain_id_in(r#"{"single_usdc":"5"}"#), BASE_SEPOLIA.chain_id); // 옛 파일
        assert_eq!(chain_id_in("{ 깨진 JSON"), BASE_SEPOLIA.chain_id);
    }
}
