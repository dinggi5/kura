// 앱 루트 — 지갑 상태(생성/마이그레이션/정상)에 따라 첫 화면을 고른다.
//
// 파일 지도 (개발 17 구조 분리 — 동작 무변경):
//   lib/types.ts      백엔드와 주고받는 공유 타입
//   lib/i18n.ts       화면 언어(한국어·영어) — t(ko, en) (개발 42)
//   lib/format.ts     표시용 포맷 헬퍼 + 상수
//   lib/useCopy.ts    클립보드 복사 훅
//   components/ui.tsx 셸/카드/버튼 스타일 + 작은 공용 컴포넌트
//   components/…      BalanceCard·ReceiveCard·SendCard·PaymentApprovalModal·banners·SessionBar
//   screens/…         SetupScreen·WalletScreen·HistoryScreen·BackupFlow·SettingsScreen

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { WalletStatus } from "@/lib/types";
import { initLang, t } from "@/lib/i18n";
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

  // 언어는 모듈 로드 때 이미 정해져 있다(캐시·시스템). 이건 백엔드 설정과의 사후 대조 —
  // 어긋날 때만 창을 한 번 다시 읽는다.
  useEffect(() => {
    void initLang();
  }, []);

  // ⌘W = 창 닫기(= 숨기기). 개발 58 — 실물에서 이 키가 아무 일도 안 하는 걸 발견했다.
  // macOS 는 ⌘W 를 앱 메뉴의 「닫기」로 보내는데 이 창은 테두리가 없어(decorations: false)
  // AppKit 이 그 항목을 비활성화한다 → 러스트의 CloseRequested 핸들러까지 오지 않는다.
  // 그래서 웹뷰에서 직접 받아 같은 종착지(tray::hide_by_user)로 보낸다.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key.toLowerCase() !== "w") return;
      e.preventDefault();
      void invoke("hide_main_window");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(loadStatus, [loadStatus]);

  // 로딩
  if (!status && !statusError) {
    return (
      <main className={shell}>
        <div />
        <span className="text-[13px] text-[var(--color-ink-300)] font-mono">
          {t("지갑 여는 중…", "Opening your wallet…")}
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
          {t("지갑 상태 확인 실패: ", "Couldn't read wallet status: ")}
          {statusError}
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
          // 방금 만든(가져온) 지갑은 계정 0 하나다 (개발 54).
          setStatus({
            state: "encrypted",
            address,
            backed_up: backedUp,
            accounts: [{ index: 0, address, label: "" }],
            active: 0,
          })
        }
      />
    );
  }

  return (
    <WalletScreen
      address={status!.address!}
      initialBackedUp={status!.backed_up}
      accounts={status!.accounts}
      active={status!.active}
      // 계정 추가·전환·이름 변경은 백엔드가 새 상태를 통째로 돌려준다 — 그걸 그대로 받는다.
      onAccountsChange={setStatus}
    />
  );
}

export default App;
