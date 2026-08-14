// 표시용 포맷 헬퍼 + 공용 상수.

/** 단일 거래 안전 상한 (백엔드 상수와 일치). */
export const MAX_ETH = 0.05;
export const MAX_USDC = 5;

/** 결제 승인 대기 시간(초). MCP/백엔드 타임아웃과 일치(5분). */
export const PAYMENT_TTL = 300;

/** 요청 생성 시각 기준 남은 승인 시간(초). */
export function secsLeft(created: number): number {
  return Math.max(0, PAYMENT_TTL - (Math.floor(Date.now() / 1000) - created));
}

/** 남은 초를 m:ss 로. */
export function fmtCountdown(s: number): string {
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export function shortenAddress(addr: string): string {
  if (!addr || addr.length < 12) return addr;
  return `${addr.slice(0, 6)}…${addr.slice(-4)}`;
}

/** 십진수 문자열을 보기 좋게. 값 없으면 "—". */
export function fmtAmount(s: string | undefined, maxFrac: number, minFrac = 0): string {
  if (s == null) return "—";
  const n = Number(s);
  if (!Number.isFinite(n)) return "—";
  return n.toLocaleString("en-US", {
    maximumFractionDigits: maxFrac,
    minimumFractionDigits: minFrac,
  });
}

export function isAddressLike(s: string): boolean {
  return /^0x[0-9a-fA-F]{40}$/.test(s.trim());
}

/** 유닉스 초 → 상대 시각 ("방금", "N분 전", 날짜). */
export function fmtRelTime(ts: number): string {
  const diff = Date.now() / 1000 - ts;
  if (diff < 60) return "방금";
  if (diff < 3600) return `${Math.floor(diff / 60)}분 전`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}시간 전`;
  if (diff < 7 * 86400) return `${Math.floor(diff / 86400)}일 전`;
  return new Date(ts * 1000).toLocaleDateString("ko-KR", { month: "short", day: "numeric" });
}
