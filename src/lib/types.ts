// 백엔드(Tauri 커맨드)와 주고받는 공유 타입. Rust 구조체와 필드가 일치해야 한다.

export type WalletInfo = { address: string };

export type WalletStatus = {
  state: "encrypted" | "legacy" | "none";
  address: string | null;
  backed_up: boolean;
};

export type Balances = { eth: string; usdc: string };

export type Settings = {
  single_usdc: string;
  daily_usdc: string;
  single_eth: string;
  daily_eth: string;
  /** 자율 결제 한도(USDC). 이 금액 이하 AI 결제는 세션 잠금 해제 시 비번 없이 자동 승인. "0"=꺼짐. */
  auto_approve_usdc: string;
  /** 세션 자동 잠금 유휴 시간(분). "0"=유휴 잠금 안 함. */
  auto_lock_mins: string;
  /** 잔액 조회·송금에 쓸 RPC 엔드포인트. 빈 값이면 공식 RPC. */
  rpc_url: string;
  /** 자리비움 자동 잠금: 창 포커스 잃으면 세션 즉시 잠금. */
  lock_on_blur: boolean;
  /** 자율 결제 알림: 비번 없이 자동 승인된 결제를 OS 알림으로 사후 통지. */
  notify_auto: boolean;
  /** 자율 결제는 신뢰 주소(비번으로 승인한 적 있는 주소)만. */
  auto_trusted_only: boolean;
  /** 활성 체인 ID — 84532=Base Sepolia(테스트넷) / 8453=Base 메인넷. 체인별 데이터 파일 분리. */
  chain_id: number;
};

/** 자율 결제 세션 상태 (메모리의 잠금 해제 키). */
export type SessionStatus = {
  unlocked: boolean;
  remaining_secs: number;
  auto_limit: string;
};

export type SpendView = { usdc: string; eth: string };

export type HistoryEntry = {
  ts: number;
  token: string;
  to: string;
  amount: string;
  status: string; // "sent"|"blocked"|"failed"|"signed"(x402 정산 대기)|"settled"(x402 정산됨)|"settle_failed"
  detail: string;
  /** x402 정산 tx 해시. 정산 전엔 빈 값. */
  settle_tx?: string;
};

export type PaymentRequest = {
  id: string;
  token: string;
  to: string;
  amount: string;
  memo: string;
  created: number;
  /** "transfer"(온체인 송금) | "x402"(EIP-3009 오프체인 서명). 없으면 transfer. */
  kind?: string;
  /** x402일 때 결제 대상 리소스 URL. */
  resource?: string;
  /** 요청 생성 시점의 체인 ID. 승인 시 현재 활성 체인과 다르면 백엔드가 거부. */
  chain_id?: number;
};

export type AgentStatus = { connected: boolean; client: string };
