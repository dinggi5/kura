// 앱 루트 — 지갑 상태(생성/마이그레이션/정상)에 따라 첫 화면을 고른다.
//
// 파일 지도 (개발 17 구조 분리 — 동작 무변경):
//   lib/types.ts      백엔드와 주고받는 공유 타입
//   lib/format.ts     표시용 포맷 헬퍼 + 상수
//   lib/useCopy.ts    클립보드 복사 훅
//   components/ui.tsx 셸/카드/버튼 스타일 + 작은 공용 컴포넌트
//   components/…      BalanceCard·ReceiveCard·SendCard·PaymentApprovalModal·banners·SessionBar
//   screens/…         SetupScreen·WalletScreen·HistoryScreen·BackupFlow·SettingsScreen

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { WalletStatus } from "@/lib/types";
import { shell } from "@/components/ui";
import { SetupScreen } from "@/screens/SetupScreen";
import { WalletScreen } from "@/screens/WalletScreen";
import "./App.css";

function App() {
  const [status, setStatus] = useState<WalletStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);

  const loadStatus = useCallback(() => {
    invoke<WalletStatus>("get_wallet_status")
      .then(setStatus)
      .catch((e) => setStatusError(String(e)));
  }, []);

  useEffect(loadStatus, [loadStatus]);

  // 로딩
  if (!status && !statusError) {
    return (
      <main className={shell}>
        <div />
        <span className="text-[13px] text-[var(--color-ink-300)] font-mono">
          지갑 여는 중…
        </span>
        <div />
      </main>
    );
  }

  if (statusError) {
    return (
      <main className={shell}>
        <div />
        <p className="text-[12px] text-red-500 font-mono max-w-md text-center">
          지갑 상태 확인 실패: {statusError}
        </p>
        <div />
      </main>
    );
  }

  // 비번 미설정 (legacy 평문 / 신규) → 보호 화면
  if (status!.state !== "encrypted") {
    return (
      <SetupScreen
        status={status!}
        onDone={(address, backedUp) =>
          setStatus({ state: "encrypted", address, backed_up: backedUp })
        }
      />
    );
  }

  return (
    <WalletScreen address={status!.address!} initialBackedUp={status!.backed_up} />
  );
}

export default App;
