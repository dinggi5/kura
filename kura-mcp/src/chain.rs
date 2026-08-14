// 체인 설정 (MCP 측) — 잔액 조회·x402 결제 제출·익스플로러 링크에 쓰는 값을 한 묶음으로.
//
// src-tauri/src/chain.rs 와 평행한 별도 사본이다(두 크레이트는 의도적으로 분리 —
// 공유 크레이트를 만들지 않는 게 이 프로젝트의 정책: Tauri 빌드 위험 0). 활성 체인은 GUI 와
// 공유하는 settings.json 의 chain_id 로 **런타임 선택**한다(두 프로세스가 자동으로 같은 체인).

use alloy::primitives::{address, Address};

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
    /// x402 네트워크 표기 — V1 단축명과 V2 CAIP-2 (서버가 둘 중 하나로 제시).
    pub x402_network_v1: &'static str,
    pub x402_network_caip2: &'static str,
    /// 익스플로러 트랜잭션 URL 접두사 (뒤에 tx 해시를 붙인다).
    pub explorer_tx_prefix: &'static str,
}

/// Base Sepolia (테스트넷). 기본 체인 — 데이터 파일이 접미사 없이 저장되는 "원본" 체인.
pub const BASE_SEPOLIA: ChainConfig = ChainConfig {
    chain_id: 84_532,
    default_rpc: "https://sepolia.base.org",
    usdc_address: address!("0x036CbD53842c5426634e7929541eC2318f3dCF7e"),
    usdc_decimals: 6,
    x402_network_v1: "base-sepolia",
    x402_network_caip2: "eip155:84532",
    explorer_tx_prefix: "https://sepolia.basescan.org/tx/",
};

/// Base 메인넷 (실제 자금). x402 네트워크명은 "base" / CAIP-2 "eip155:8453".
pub const BASE_MAINNET: ChainConfig = ChainConfig {
    chain_id: 8453,
    default_rpc: "https://mainnet.base.org",
    usdc_address: address!("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
    usdc_decimals: 6,
    x402_network_v1: "base",
    x402_network_caip2: "eip155:8453",
    explorer_tx_prefix: "https://basescan.org/tx/",
};

/// 사용자가 선택한 체인 ID. 없거나 chain_id 없는 옛 설정이면 Base Sepolia(84532).
fn selected_chain_id() -> u64 {
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
            eprintln!("KURA_CHAIN_ID 값이 정수가 아니에요: {v:?}");
            std::process::exit(1);
        });
        if id != BASE_SEPOLIA.chain_id && id != BASE_MAINNET.chain_id {
            eprintln!(
                "KURA_CHAIN_ID={id} 는 지원하지 않는 체인이에요(지원: {} / {}).",
                BASE_SEPOLIA.chain_id, BASE_MAINNET.chain_id
            );
            std::process::exit(1);
        }
        return id;
    }
    // 단위 테스트는 실제 ~/.jigap/settings.json 을 읽지 않는다 — 사용자의 지갑 설정(메인넷 등)에
    // 테스트 결과가 좌우되지 않게 기본 체인(Base Sepolia)으로 고정해 항상 결정론적이게 한다.
    #[cfg(test)]
    {
        BASE_SEPOLIA.chain_id
    }
    #[cfg(not(test))]
    {
        /// settings.json 에서 선택된 체인 ID만 읽는 가벼운 뷰(다른 필드 무시).
        #[derive(serde::Deserialize)]
        struct ChainSel {
            chain_id: u64,
        }
        crate::wallet::jigap_dir()
            .ok()
            .map(|d| d.join("settings.json"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<ChainSel>(&s).ok())
            .map(|c| c.chain_id)
            .unwrap_or(BASE_SEPOLIA.chain_id)
    }
}

/// 현재 활성 체인 — settings.json 의 chain_id 로 런타임 선택(GUI 와 동일 파일 → 자동 일치).
pub fn active_chain() -> ChainConfig {
    match selected_chain_id() {
        id if id == BASE_MAINNET.chain_id => BASE_MAINNET,
        _ => BASE_SEPOLIA,
    }
}

/// 체인별로 분리되는 데이터 파일 이름(history/x402_settlements). src-tauri 의 chain_file 과 동일 규칙 —
/// 기본 체인(Base Sepolia)은 접미사 없이("history.json"), 그 외는 "-{chain_id}". 두 프로세스가
/// 같은 settings.json 을 읽으므로 GUI 와 MCP 가 항상 같은 파일을 가리킨다.
/// active_chain() 으로 정규화(미지원 id 는 Sepolia 폴백)해 활성 체인과 파일이 어긋나지 않게 한다.
pub fn chain_file(stem: &str) -> String {
    match active_chain().chain_id {
        id if id == BASE_SEPOLIA.chain_id => format!("{stem}.json"),
        id => format!("{stem}-{id}.json"),
    }
}
