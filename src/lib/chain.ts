// 프론트 체인 설정 — 표시 이름·익스플로러·기본 RPC 같은 "보여주기" 값 + 활성 체인 컨텍스트.
//
// src-tauri/src/chain.rs · kura-mcp/src/chain.rs 와 평행한 프론트 사본이다(세 레이어가 같은
// 체인을 각자 기술 — 공유 소스 없음, 프로젝트 정책). 체인 추가 시 세 곳을 함께 갱신.
//
// 활성 체인은 더 이상 컴파일 고정 상수가 아니라 settings.chain_id 로 런타임 선택한다.
// WalletScreen 이 ChainProvider 로 활성 ChainConfig 를 내려주고, 화면들은 useChain() 으로 읽는다.

import { createContext, useContext } from "react";

export interface ChainConfig {
  /** EIP-155 체인 ID — settings.chain_id 와 매칭. */
  id: number;
  /** 사람용 표시 이름 (예: "Base Sepolia"). */
  name: string;
  /** 익스플로러 이름 (버튼 라벨용, 예: "BaseScan"). */
  explorerName: string;
  /** 익스플로러 트랜잭션 URL 접두사 (뒤에 tx 해시를 붙인다). */
  explorerTx: string;
  /** 설정 화면의 "공식 RPC" 옵션 URL (빈 값 저장 시 백엔드가 이 값으로 폴백). */
  defaultRpc: string;
  /** "로그 안 남김" 표방 대체 공개 RPC (설정 프리셋용). */
  publicNode: string;
  /** 테스트넷이면 true — 받기 화면의 Faucet 노출, 메인넷 경고 등에 쓴다. */
  testnet: boolean;
}

/** Base Sepolia (테스트넷). 기본 체인. */
export const BASE_SEPOLIA: ChainConfig = {
  id: 84532,
  name: "Base Sepolia",
  explorerName: "BaseScan",
  explorerTx: "https://sepolia.basescan.org/tx/",
  defaultRpc: "https://sepolia.base.org",
  publicNode: "https://base-sepolia-rpc.publicnode.com",
  testnet: true,
};

/** Base 메인넷 (실제 자금). */
export const BASE_MAINNET: ChainConfig = {
  id: 8453,
  name: "Base 메인넷",
  explorerName: "BaseScan",
  explorerTx: "https://basescan.org/tx/",
  defaultRpc: "https://mainnet.base.org",
  publicNode: "https://base-rpc.publicnode.com",
  testnet: false,
};

/** 선택 가능한 체인 목록 (설정 토글 순서). */
export const CHAINS: ChainConfig[] = [BASE_SEPOLIA, BASE_MAINNET];

/** chain_id → ChainConfig. 알 수 없거나 미정이면 테스트넷으로 폴백(안전 쪽). */
export function chainFromId(id: number | undefined): ChainConfig {
  return CHAINS.find((c) => c.id === id) ?? BASE_SEPOLIA;
}

// 활성 체인 컨텍스트 — WalletScreen 이 settings.chain_id 로 파생해 Provider 로 내려준다.
const ChainContext = createContext<ChainConfig>(BASE_SEPOLIA);
export const ChainProvider = ChainContext.Provider;
export function useChain(): ChainConfig {
  return useContext(ChainContext);
}
