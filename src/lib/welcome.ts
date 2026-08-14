// 환영 투어 1회 표시 상태 — 지갑 주소별로 localStorage 에 보관한다.
// 새 지갑 생성 시 pending 으로 표시 → 투어 완료/건너뛰기 시 소거.
// 영속이라 투어 도중 앱을 종료해도 다음 실행 때 다시 뜬다(중간 종료 시 영영 스킵되던 문제 방지).

const key = (address: string) => `kura.welcomePending.${address.toLowerCase()}`;

export function markWelcomePending(address: string): void {
  try {
    localStorage.setItem(key(address), "1");
  } catch {
    /* localStorage 불가(프라이빗 모드 등)면 조용히 무시 — 투어는 부가 기능 */
  }
}

export function isWelcomePending(address: string): boolean {
  try {
    return localStorage.getItem(key(address)) === "1";
  } catch {
    return false;
  }
}

export function clearWelcomePending(address: string): void {
  try {
    localStorage.removeItem(key(address));
  } catch {
    /* 무시 */
  }
}
