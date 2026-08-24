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
  /** 시작 시 업데이트 자동 확인 (개발 31). 읽기 전용으로만 쓴다 — 변경은 set_auto_check_update. */
  auto_check_update: boolean;
};

/** 백엔드가 찾은 업데이트 (개발 31). 사람이 설치를 누르기 전에 보는 값 = 판단 근거 전부. */
export type UpdateInfo = {
  version: string;
  current_version: string;
  notes: string | null;
  date: string | null;
};

/** `update://progress` 이벤트 payload. total 은 서버가 크기를 안 알려주면 null. */
export type UpdateProgress = {
  downloaded: number;
  total: number | null;
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

/** AI 연결 화면(개발 35) 감지 상태 — 전부 로컬 파일 감지, 최종 진실은 AgentStatus. */
export type ConnectStatus = {
  desktop_installed: boolean;
  desktop_ext_installed: boolean;
  cli_path: string | null;
  /** kura 등록이 있고 command 가 이 빌드의 kura-mcp 경로와 일치. */
  cli_registered: boolean;
  /** kura 등록은 있는데 다른 경로를 가리킴(옛 설치 등) — 재등록 안내 대상. */
  cli_registered_other: boolean;
  /** 등록·안내용 kura-mcp 경로. 임시 위치 실행 중이면 설치본으로 해석된 값. */
  mcp_path: string | null;
  /** 임시 위치(App Translocation·DMG)에서 실행 중 — mcp_path 까지 없으면 이전 안내. */
  temp_location: boolean;
  /** 지금 실행 중인 이 앱의 버전. */
  app_version: string;
  /**
   * 임시 위치 실행이라 **설치본** 경로를 등록하게 되는데 그 설치본이 이 앱과 다른
   * 버전일 때, 그 설치본 버전. AI 에 붙을 kura-mcp 는 설치본 안의 것이라 화면과
   * 실물이 갈린다 (코덱스 개발38 2차 P2). 같거나 못 읽으면 null.
   */
  installed_version_mismatch: string | null;
};

/** connect_claude_code 가 실패할 때 주는 모양 (코덱스 개발38 2차 P2). */
export type ConnectError = {
  message: string;
  /**
   * 옛 kura 등록을 되살리는 `claude mcp add-json …` 명령. 화면이 안내하는 수동
   * 재등록은 `remove; add` 라서, add 가 또 실패하면 원복해 둔 등록까지 지운다 —
   * 그때 되돌릴 손잡이. 지우기 전에 옛 항목이 있었을 때만 채워진다.
   */
  restore_command: string | null;
};
