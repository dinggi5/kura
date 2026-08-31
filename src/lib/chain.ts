// 프론트 체인 설정 — 표시 이름·익스플로러·기본 RPC 같은 "보여주기" 값 + 활성 체인 컨텍스트.
//
// src-tauri/src/chain.rs · kura-mcp/src/chain.rs 와 평행한 프론트 사본이다(세 레이어가 같은
// 체인을 각자 기술 — 공유 소스 없음, 프로젝트 정책). 체인 추가 시 세 곳을 함께 갱신.
//
// 활성 체인은 더 이상 컴파일 고정 상수가 아니라 settings.chain_id 로 런타임 선택한다.
// WalletScreen 이 ChainProvider 로 활성 ChainConfig 를 내려주고, 화면들은 useChain() 으로 읽는다.

import { createContext, useContext } from "react";

import { t } from "./i18n";

export interface ChainConfig {
  /** EIP-155 체인 ID — settings.chain_id 와 매칭. */
  id: number;
  /** 사람용 표시 이름 (예: "Base Sepolia"). 언어를 타는 값이라 `chainName()` 으로 읽는다. */
  name: string;
  /** 익스플로러 이름 (버튼 라벨용, 예: "BaseScan"). */
  explorerName: string;
  /** 익스플로러 트랜잭션 URL 접두사 (뒤에 tx 해시를 붙인다). */
  explorerTx: string;
  /** 설정 화면의 "공식 RPC" 옵션 URL (빈 값 저장 시 백엔드가 이 값으로 폴백). */
  defaultRpc: string;
  /** "로그 안 남김" 표방 대체 공개 RPC (설정 프리셋용). **없는 체인은 undefined** —
   *  Arc 엔 PublicNode 가 없다(개발 50 확인). 없는 걸 있는 척 채우면 프라이버시 약속이 거짓말이 되므로
   *  프리셋에서 빼고 "공식 / 직접 입력" 둘만 보여준다. */
  publicNode?: string;
  /** 테스트넷이면 true — 받기 화면의 Faucet 노출, 메인넷 경고 등에 쓴다. */
  testnet: boolean;
  /** **거래소·다른 지갑의 네트워크 목록에 뜨는 이름** (메인넷만 필요). 표시 이름(`name`)과 다르다 —
   *  거래소 드롭다운엔 「Base 메인넷」이 아니라 「Base」로 뜬다. 이 값을 틀리게 안내하면 사용자가
   *  다른 네트워크로 입금해 **자금을 잃는다** → 체인을 추가할 때 반드시 같이 채운다.
   *  테스트넷은 거래소에서 보낼 일이 없어 비워 둔다(받기 화면의 그 문단이 안 뜬다). */
  depositNetwork?: string;
  /** **네이티브(가스) 토큰이 이 체인의 USDC 와 같은 자산인가** (개발 50, Arc).
   *  true 면 "USDC 잔액"과 "가스 잔액"이 같은 돈이라 따로 보여주면 두 번 세는 화면이 된다 →
   *  가스 줄·ETH 보내기 탭·ETH 한도·ETH Faucet 을 전부 감춘다. 백엔드도 같은 이유로 네이티브
   *  잔액을 아예 안 내려보내고(Balances.eth 없음) 네이티브 송금을 막는다. */
  nativeIsUsdc: boolean;
  /** 테스트 USDC Faucet (testnet 전용). */
  usdcFaucet?: string;
  /** 테스트 가스 토큰 Faucet — nativeIsUsdc 인 체인엔 없다(가스가 곧 위 USDC). */
  gasFaucet?: string;
  /** **송금할 때 가스 몫으로 남겨 둬야 하는 USDC** — nativeIsUsdc 인 체인에만 있다 (개발 50).
   *  가스가 같은 잔액에서 나가므로 «잔액 전부»를 보내면 가스를 못 내 실패한다. 근거(개발 50 실측,
   *  Arc 테스트넷): ERC-20 transfer `eth_estimateGas` = 49,314 · `eth_gasPrice` = 21 gwei 상당
   *  → 한 번에 **약 0.00104 USDC**. 여기 값은 그 10배로 잡았다(혼잡·가격 변동 여유). */
  gasReserveUsdc?: number;
  /** ERC-8004 레지스트리가 이 체인에 배포돼 있나 (개발 47). false 면 신원 조회 설정을
   *  아예 안 보여준다 — 켤 수 없는 스위치를 두는 것보다 없는 게 정직하다. */
  erc8004: boolean;
}

/** Base Sepolia (테스트넷). 연습용 — 옛 파일·깨진 설정의 보수적 폴백 체인(백엔드 기준). */
export const BASE_SEPOLIA: ChainConfig = {
  id: 84532,
  name: "Base Sepolia",
  explorerName: "BaseScan",
  explorerTx: "https://sepolia.basescan.org/tx/",
  defaultRpc: "https://sepolia.base.org",
  publicNode: "https://base-sepolia-rpc.publicnode.com",
  testnet: true,
  nativeIsUsdc: false,
  usdcFaucet: "https://faucet.circle.com",
  gasFaucet: "https://portal.cdp.coinbase.com/products/faucet",
  erc8004: true,
};

/** Base 메인넷 (실제 자금). 신규 기본 체인 (개발 39). */
export const BASE_MAINNET: ChainConfig = {
  id: 8453,
  name: t("Base 메인넷", "Base mainnet"),
  explorerName: "BaseScan",
  explorerTx: "https://basescan.org/tx/",
  defaultRpc: "https://mainnet.base.org",
  publicNode: "https://base-rpc.publicnode.com",
  testnet: false,
  depositNetwork: "Base",
  nativeIsUsdc: false,
  erc8004: true,
};

/** Arc 테스트넷 (Circle L1, 개발 50). **가스도 USDC로 낸다** — 이 체인만 nativeIsUsdc.
 *  ERC-8004 레지스트리가 Base Sepolia 와 같은 주소로 여기에도 있다(개발 50 온체인 확인) → true. */
export const ARC_TESTNET: ChainConfig = {
  id: 5042002,
  name: t("Arc 테스트넷", "Arc testnet"),
  explorerName: "Arcscan",
  explorerTx: "https://testnet.arcscan.app/tx/",
  defaultRpc: "https://rpc.testnet.arc.network",
  // publicNode 없음 — Arc 를 서비스하는 PublicNode 엔드포인트가 아직 없다(개발 50 실측).
  testnet: true,
  nativeIsUsdc: true,
  usdcFaucet: "https://faucet.circle.com",
  gasReserveUsdc: 0.01,
  // gasFaucet 없음 — 위 USDC 가 곧 가스다.
  erc8004: true,
};

/** 선택 가능한 체인 목록 (설정 토글 순서 — 메인넷이 왼쪽/기본, 개발 39). */
export const CHAINS: ChainConfig[] = [BASE_MAINNET, BASE_SEPOLIA, ARC_TESTNET];

/** chain_id → ChainConfig. 두 폴백이 다르다(코덱스 개발 39 P2):
 *  - undefined(설정 로드 전) → 메인넷 — 신규 기본과 일치시켜 로드 전후 화면이 안 흔들리게.
 *  - **모르는 id**(미래 체인에서 내려온 설정 등) → 테스트넷 — 백엔드 active_chain()·MCP 가
 *    같은 id 를 테스트넷으로 정규화하므로, 여기서 메인넷을 그리면 잔액은 테스트넷인데
 *    라벨·익스플로러만 메인넷인 어긋남이 생긴다. */
export function chainFromId(id: number | undefined): ChainConfig {
  if (id === undefined) return BASE_MAINNET;
  return CHAINS.find((c) => c.id === id) ?? BASE_SEPOLIA;
}

// 활성 체인 컨텍스트 — WalletScreen 이 settings.chain_id 로 파생해 Provider 로 내려준다.
const ChainContext = createContext<ChainConfig>(BASE_MAINNET);
export const ChainProvider = ChainContext.Provider;
export function useChain(): ChainConfig {
  return useContext(ChainContext);
}
