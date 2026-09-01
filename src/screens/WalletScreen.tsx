// 지갑 메인 화면 — 잔액/받기/보내기 카드 + 헤더 + 배너 + 1초 폴링(AI 연결·세션·결제 요청).

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AnimatePresence } from "framer-motion";
import {
  ArrowDownLeft,
  ArrowUpRight,
  HelpCircle,
  History,
  KeyRound,
  Settings as SettingsIcon,
  ShieldAlert,
  ShieldCheck,
} from "lucide-react";
import type {
  AgentStatus,
  Balances,
  HistoryEntry,
  PaymentRequest,
  SessionStatus,
  Settings,
  SpendView,
} from "@/lib/types";
import { cn } from "@/lib/cn";
import { chainFromId, ChainProvider } from "@/lib/chain";
import { useCopy } from "@/lib/useCopy";
import { clearWelcomePending, isWelcomePending } from "@/lib/welcome";
import { shell, ActionButton, AgentBadge, HeaderIconButton } from "@/components/ui";
import { BalanceCard } from "@/components/BalanceCard";
import { ReceiveCard } from "@/components/ReceiveCard";
import { SendCard } from "@/components/SendCard";
import { PaymentApprovalModal } from "@/components/PaymentApprovalModal";
import { BackupNag, LockBanner, SettingsBrokenBanner, UpdateBanner } from "@/components/banners";
import { useUpdate } from "@/lib/useUpdate";
import { SessionBar, UnlockSessionModal } from "@/components/SessionBar";
import { BackupFlow } from "@/screens/BackupFlow";
import { ConnectScreen } from "@/screens/ConnectScreen";
import { HelpScreen } from "@/screens/HelpScreen";
import { HistoryScreen } from "@/screens/HistoryScreen";
import { SettingsScreen } from "@/screens/SettingsScreen";
import { WelcomeTour } from "@/screens/WelcomeTour";
import { t } from "@/lib/i18n";

type Mode = "balance" | "receive" | "send";

export function WalletScreen({
  address,
  initialBackedUp,
}: {
  address: string;
  initialBackedUp: boolean;
}) {
  const [mode, setMode] = useState<Mode>("balance");
  const [copied, copy] = useCopy();
  const [balances, setBalances] = useState<Balances | null>(null);
  const [balanceError, setBalanceError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [backedUp, setBackedUp] = useState(initialBackedUp);
  const [showBackup, setShowBackup] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [settings, setSettings] = useState<Settings | null>(null);
  // 설정 파일이 있는데 못 읽는 중인가 (개발 52) — 그때 위 settings 는 기본값이고, 그 사실을 배너로 알린다.
  const [settingsBroken, setSettingsBroken] = useState(false);
  const [spend, setSpend] = useState<SpendView | null>(null);
  const [locked, setLocked] = useState(false);
  const [showHistory, setShowHistory] = useState(false);
  const [history, setHistory] = useState<HistoryEntry[] | null>(null);
  const [showHelp, setShowHelp] = useState(false);
  const [showConnect, setShowConnect] = useState(false);
  // 새 지갑 생성 직후 1회 환영 투어(주소별 localStorage). 메인 화면 위 오버레이로 띄워
  // 결제 폴링·하트비트·승인 모달이 투어 중에도 살아 있게 한다.
  const [showTour, setShowTour] = useState(() => isWelcomePending(address));
  // 업데이트 (개발 31) — 여기서 한 번만 부르고 설정 화면에 내려준다.
  // 설정 안에서 부르면 화면을 닫을 때마다 확인 결과와 다운로드 진행이 사라진다.
  const update = useUpdate();
  // AI 에이전트(MCP)가 보낸 결제 승인 요청. 1초마다 폴링한다.
  const [pending, setPending] = useState<PaymentRequest | null>(null);
  // AI(MCP 클라이언트)가 지금 이 지갑에 연결돼 있는지 — 메인 화면 배지용.
  const [agent, setAgent] = useState<AgentStatus>({ connected: false, client: "" });
  // 자율 결제 세션 상태(메모리의 잠금 해제 키). 잠금 해제 시 한도 이하는 비번 없이 자동 승인.
  const [session, setSession] = useState<SessionStatus>({ unlocked: false, remaining_secs: 0, auto_limit: "0" });
  const [showUnlock, setShowUnlock] = useState(false);
  // 결제 요청별 자율 승인 1회 시도 가드(중복 시도·진행 중 모달 깜빡임 방지).
  const autoTried = useRef<Set<string>>(new Set());
  const autoBusy = useRef<string | null>(null);

  // 활성 체인 — settings.chain_id 로 파생(미로드 시 메인넷 = 신규 기본, 개발 39). ChainProvider 로 내려준다.
  const chain = chainFromId(settings?.chain_id);

  const loadHistory = useCallback(() => {
    invoke<HistoryEntry[]>("get_history").then(setHistory).catch(() => {});
  }, []);

  // 긴급 잠금 토글. 켜면 안전을 위해 보내기 화면도 닫는다.
  const toggleLock = useCallback(async () => {
    const next = !locked;
    try {
      await invoke("set_locked", { locked: next });
      setLocked(next);
      if (next) setMode("balance");
    } catch {
      /* 토글 실패는 조용히 무시 */
    }
  }, [locked]);

  // 잔액 요청 세대 번호(코덱스 개발35 2차) — 늦게 도착한 옛 응답이 더 새 값을 덮지 않게.
  // 체인 전환 직후가 전형: 배경 30초 요청이 옛 체인에 나가 있는 사이 전환이 새 요청을
  // 쏘면, 옛 응답이 나중에 도착해 새 체인 화면에 옛 체인 잔액을 그린다. 수동·배경이
  // 같은 카운터를 쓰므로 마지막으로 시작한 요청만 화면에 닿는다.
  const balanceReq = useRef(0);
  // 수동 새로고침 진행 카운터(ref) — 배경 갱신이 수동 진행 중에 세대를 올리면 수동 쪽
  // finally 가 스피너를 못 끄고 영영 돈다. 배경은 수동이 하나라도 진행 중이면 쉰다.
  // (겹친 수동끼리는 세대 번호가 정리한다: 마지막 수동만 화면·스피너를 만진다.)
  const manualBusy = useRef(0);
  const refreshBalances = useCallback(async () => {
    const req = ++balanceReq.current;
    manualBusy.current += 1;
    setRefreshing(true);
    setBalanceError(null);
    try {
      const b = await invoke<Balances>("get_balances", { addrHex: address });
      if (balanceReq.current === req) setBalances(b);
    } catch (e) {
      if (balanceReq.current === req) setBalanceError(String(e));
    } finally {
      manualBusy.current -= 1;
      if (balanceReq.current === req) setRefreshing(false);
    }
  }, [address]);

  // 입금 자동 반영(개발 35)의 배경 갱신 — 스피너·에러 없이 조용히. 실패하면 기존 값을
  // 유지한다(외부 입금 감지가 목적이라, 일시적 RPC 오류로 멀쩡한 화면을 더럽힐 이유가 없다).
  // in-flight 가드(코덱스 개발35 1차): 느린 RPC 에서 30초 주기가 요청을 겹겹이 쌓지 않게 —
  // 진행 중이면 이번 차례는 그냥 쉰다. 응답 순서 꼬임은 위의 세대 번호가 막는다.
  const silentBusy = useRef(false);
  const refreshBalancesSilent = useCallback(async () => {
    if (silentBusy.current || manualBusy.current > 0) return;
    silentBusy.current = true;
    const req = ++balanceReq.current;
    try {
      const b = await invoke<Balances>("get_balances", { addrHex: address });
      if (balanceReq.current === req) {
        setBalances(b);
        setBalanceError(null);
      }
    } catch {
      /* 다음 주기에 다시 */
    } finally {
      silentBusy.current = false;
    }
  }, [address]);

  // 한도 설정 + 오늘 사용액 (송금 후 갱신).
  const loadLimits = useCallback(() => {
    invoke<Settings>("get_settings").then(setSettings).catch(() => {});
    invoke<boolean>("settings_file_broken").then(setSettingsBroken).catch(() => {});
    invoke<SpendView>("get_today_spend").then(setSpend).catch(() => {});
  }, []);

  useEffect(() => {
    void refreshBalances();
    loadLimits();
    invoke<boolean>("is_locked").then(setLocked).catch(() => {});
  }, [refreshBalances, loadLimits]);

  // 시작 시 업데이트 자동 확인 (개발 31). 설정이 로드된 뒤 **실행당 한 번만** 돈다 —
  // loadLimits 가 설정 화면을 닫을 때마다 settings 를 새로 받아오므로 ref 로 못을 박는다.
  // silent: 네트워크가 없는 흔한 경우에 지갑 화면이 에러로 더러워지지 않게.
  const autoCheckDone = useRef(false);
  const autoCheckWanted = settings?.auto_check_update;
  const { check: checkUpdate } = update;
  useEffect(() => {
    if (autoCheckDone.current || autoCheckWanted === undefined) return;
    autoCheckDone.current = true;
    if (autoCheckWanted) void checkUpdate({ silent: true });
  }, [autoCheckWanted, checkUpdate]);

  // 체인이 바뀌면(설정에서 테스트넷↔메인넷 전환) 잔액·내역은 체인별로 다르므로 다시 불러온다.
  // (사용액 spend 는 loadLimits 가 설정 저장 후 onClose 에서 갱신한다.)
  const chainId = settings?.chain_id;
  useEffect(() => {
    if (chainId === undefined) return;
    void refreshBalances();
    loadHistory();
  }, [chainId, refreshBalances, loadHistory]);

  // 입금 자동 반영(개발 35): 잔액이 시작·체인 전환·결제 직후·수동 ↻에서만 갱신돼서 외부
  // 입금은 재시작해야 보이던 문제(실사용 발견). 창이 보일 때만 30초 폴링 + 창이 다시
  // 보이거나 포커스가 돌아오면 즉시 1회. 팝오버가 숨으면 WKWebView 가 document.hidden 을
  // 켜므로 숨은 동안은 RPC 를 안 부른다.
  useEffect(() => {
    let last = 0;
    const tick = () => {
      if (document.hidden) return;
      last = Date.now();
      void refreshBalancesSilent();
    };
    const h = setInterval(tick, 30_000);
    // 복귀 1회 — 직전 갱신과 겹치지 않게 5초 스로틀.
    const onReturn = () => {
      if (document.hidden || Date.now() - last < 5_000) return;
      last = Date.now();
      void refreshBalancesSilent();
      // 설정도 다시 읽는다(코덱스 개발 52 P2): 이 화면은 창이 숨어도 마운트된 채라, 사용자가
      // 배너대로 settings.json 을 고쳐도 다음 결제·설정 닫기·재시작 전엔 배너와 체인 라벨이
      // 옛 값에 머문다. 창이 돌아올 때 한 번 재확인하면 그 사이가 사라진다(파일 읽기 둘뿐).
      loadLimits();
    };
    document.addEventListener("visibilitychange", onReturn);
    window.addEventListener("focus", onReturn);
    return () => {
      clearInterval(h);
      document.removeEventListener("visibilitychange", onReturn);
      window.removeEventListener("focus", onReturn);
    };
  }, [refreshBalancesSilent, loadLimits]);

  // 1초 폴링: ① AI 연결 상태 ② 세션 상태 ③ 결제 요청(=앱 생존 하트비트 갱신, 자율 승인 우선 시도).
  // 새 결제 요청 1건당 자율 승인을 먼저 시도 → 자율 불가면(NEEDS_PASSWORD·차단) 사람 승인 모달.
  // 같은 id면 객체를 유지(입력 안 끊기게).
  useEffect(() => {
    let active = true;
    const tick = async () => {
      invoke<AgentStatus>("get_agent_status")
        .then((s) => {
          if (active) setAgent((prev) => (prev.connected === s.connected && prev.client === s.client ? prev : s));
        })
        .catch(() => {});
      invoke<SessionStatus>("session_status")
        .then((s) => {
          if (active)
            setSession((prev) =>
              prev.unlocked === s.unlocked &&
              prev.remaining_secs === s.remaining_secs &&
              prev.auto_limit === s.auto_limit
                ? prev
                : s,
            );
        })
        .catch(() => {});
      // x402 정산 결과(MCP가 기록)를 내역에 반영. 반영됐으면 내역·잔액 새로고침.
      invoke<number>("apply_x402_settlements")
        .then((n) => {
          if (active && n > 0) {
            loadHistory();
            void refreshBalances();
          }
        })
        .catch(() => {});

      let r: PaymentRequest | null = null;
      try {
        r = await invoke<PaymentRequest | null>("get_pending_request");
      } catch {
        return;
      }
      if (!active) return;
      if (!r) {
        if (autoBusy.current === null) setPending(null);
        return;
      }
      // 자율 승인 처리 중인 요청이면 모달을 띄우지 않고 기다린다.
      if (autoBusy.current === r.id) return;
      // 이미 자율 시도를 해본 요청이면 곧장 모달(같은 id면 객체 유지).
      if (autoTried.current.has(r.id)) {
        const req = r;
        setPending((prev) => (prev?.id === req.id ? prev : req));
        return;
      }
      // 처음 보는 요청 → 자율 승인 1회 시도.
      autoTried.current.add(r.id);
      autoBusy.current = r.id;
      const req = r;
      try {
        await invoke("auto_approve_payment", { id: req.id });
        // 자율 승인 성공 → 모달 없이 조용히 처리. 잔액/내역 갱신.
        if (active) {
          setPending(null);
          void refreshBalances();
          loadLimits();
          loadHistory();
        }
      } catch {
        // 자율 불가(세션 잠김·한도 초과·자율 꺼짐) 또는 차단 → 사람 승인 모달.
        if (active) setPending(req);
      } finally {
        autoBusy.current = null;
      }
    };
    void tick();
    const h = setInterval(() => void tick(), 1000);
    return () => {
      active = false;
      clearInterval(h);
    };
  }, [refreshBalances, loadLimits, loadHistory]);

  // 사람 승인이 필요한 동안 창을 전면 고정 (가려진 채 5분 타임아웃을 놓친 실사례 방지).
  // macOS 14+는 포커스 뺏기를 무시해서 항상-위 고정 방식 — 승인이 끝나면(cleanup) 해제.
  const pendingId = pending?.id;
  useEffect(() => {
    if (!pendingId) return;
    // 순서가 뒤집혀도(둘 다 비동기) 백엔드가 대기 요청 유무를 직접 보고 판단하므로 안전하다.
    invoke("raise_main_window").catch(() => {});
    return () => {
      invoke("release_main_window").catch(() => {});
    };
  }, [pendingId]);

  // 세션 잠금 해제 — 비번 한 번 입력 → 메모리에 키 보관, 한도 이하 자동 결제 활성화.
  const unlockSession = useCallback(async (password: string) => {
    await invoke("unlock_session", { password });
    const s = await invoke<SessionStatus>("session_status");
    setSession(s);
  }, []);

  // 세션 수동 잠금 — 메모리 키 즉시 소멸.
  const lockSession = useCallback(async () => {
    try {
      await invoke("lock_session");
    } catch {
      /* 무시 */
    }
    setSession((prev) => ({ ...prev, unlocked: false, remaining_secs: 0 }));
  }, []);

  // 결제 승인 팝업은 어떤 하위 화면(백업/설정/내역) 위에도 떠야 하므로 모든 반환을 감싼다.
  // 활성 체인을 모든 하위 화면·모달에 내려주고(useChain), 결제 승인 팝업도 그 안에 둔다.
  const withModal = (node: React.ReactNode) => (
    <ChainProvider value={chain}>
      {/* 투어 중에는 배경을 inert 처리 — Tab 포커스가 뒤 버튼으로 새지 않게(시각 z-40 + 키보드 차단). */}
      <div inert={showTour ? true : undefined}>{node}</div>
      {/* 환영 투어 오버레이(z-40). 결제 모달(z-50)보다 아래라, 투어 중 결제 요청이 와도
          승인 팝업이 그 위에 정상으로 뜬다. */}
      <AnimatePresence>
        {showTour && (
          <WelcomeTour
            inert={!!pending}
            onDone={() => {
              clearWelcomePending(address);
              setShowTour(false);
            }}
          />
        )}
      </AnimatePresence>
      <AnimatePresence>
        {pending && (
          <PaymentApprovalModal
            key={pending.id}
            request={pending}
            locked={locked}
            balances={balances}
            onResolved={() => {
              setPending(null);
              void refreshBalances();
              loadLimits();
              loadHistory();
            }}
          />
        )}
      </AnimatePresence>
    </ChainProvider>
  );

  // 시드 백업 화면 (헤더 버튼 또는 경고 배너에서 진입) — 비번 입력부터 시작.
  if (showBackup) {
    return withModal(
      <BackupFlow
        mode="review"
        onComplete={() => {
          setBackedUp(true);
          setShowBackup(false);
        }}
        onExit={() => setShowBackup(false)}
      />
    );
  }

  // 설정 화면.
  if (showSettings) {
    return withModal(
      <SettingsScreen
        current={settings}
        spend={spend}
        update={update}
        onClose={() => {
          setShowSettings(false);
          loadLimits();
        }}
      />,
    );
  }

  // 거래 내역 화면.
  if (showHistory) {
    return withModal(
      <HistoryScreen entries={history} onClose={() => setShowHistory(false)} />,
    );
  }

  // 도움말 화면.
  if (showHelp) {
    return withModal(<HelpScreen onClose={() => setShowHelp(false)} />);
  }

  // AI 연결 화면 (개발 35) — 연결 배지가 진입점.
  if (showConnect) {
    return withModal(<ConnectScreen agent={agent} onClose={() => setShowConnect(false)} />);
  }

  return withModal(
    <main className={shell}>
      <div className="w-full max-w-md flex flex-col gap-4">
        <header className="flex items-center justify-between text-[12px]">
          <span className="flex items-center gap-2 text-[var(--color-ink-500)]">
            <span className="inline-block w-1.5 h-1.5 rounded-full bg-[var(--color-accent)]" aria-hidden />
            Kura
          </span>
          <div className="flex items-center gap-1.5">
            <span
              className={cn(
                "mr-1 tracking-tight",
                chain.testnet ? "text-[var(--color-ink-500)]" : "text-[var(--color-accent)] font-medium",
              )}
            >
              {chain.name}
            </span>
            <HeaderIconButton onClick={() => setShowHelp(true)} label={t("도움말", "Help")}>
              <HelpCircle size={14} />
            </HeaderIconButton>
            <HeaderIconButton
              onClick={toggleLock}
              label={
                locked
                  ? t("긴급 잠금 해제", "Turn off emergency lock")
                  : t("긴급 잠금 (모든 송금 차단)", "Emergency lock (blocks every payment)")
              }
              tone={locked ? "danger" : "default"}
            >
              {locked ? <ShieldAlert size={14} /> : <ShieldCheck size={14} />}
            </HeaderIconButton>
            <HeaderIconButton
              onClick={() => {
                loadHistory();
                setShowHistory(true);
              }}
              label={t("거래 내역", "History")}
            >
              <History size={14} />
            </HeaderIconButton>
            <HeaderIconButton
              onClick={() => setShowBackup(true)}
              label={t("시드 백업", "Back up your words")}
              tone={backedUp ? "default" : "warn"}
            >
              <KeyRound size={14} />
            </HeaderIconButton>
            <HeaderIconButton onClick={() => setShowSettings(true)} label={t("설정", "Settings")}>
              <SettingsIcon size={14} />
            </HeaderIconButton>
          </div>
        </header>

        <div className="flex">
          <AgentBadge
            connected={agent.connected}
            client={agent.client}
            onClick={() => setShowConnect(true)}
          />
        </div>

        {locked && <LockBanner onUnlock={toggleLock} />}
        {!locked && (Number(session.auto_limit) > 0 || session.unlocked) && (
          <SessionBar
            session={session}
            onUnlock={() => setShowUnlock(true)}
            onLock={lockSession}
          />
        )}
        {!backedUp && <BackupNag onBackup={() => setShowBackup(true)} />}
        {/* 설정 파일 못 읽음(개발 52) — 백업보다 아래, 업데이트보다 위. 돈은 안 걸렸지만
            「고른 RPC 가 안 쓰인다」는 프라이버시 사실이라 새 버전 안내보다 먼저다. */}
        {settingsBroken && <SettingsBrokenBanner />}
        {/* 업데이트 안내는 맨 아래 — 긴급 잠금·시드 백업이 먼저다(돈이 걸린 순서).
            버튼은 설치가 아니라 설정의 정보 카드로 보낸다: 릴리스 노트를 보고 누르는 게 승인. */}
        {update.info && !update.bannerHidden && (
          <UpdateBanner
            version={update.info.version}
            onOpen={() => {
              update.hideBanner();
              setShowSettings(true);
            }}
            onHide={update.hideBanner}
          />
        )}
      </div>

      <div className="w-full max-w-md">
        <AnimatePresence mode="wait">
          {mode === "balance" && (
            <BalanceCard
              key="balance"
              address={address}
              copied={copied}
              onCopy={() => copy(address)}
              balances={balances}
              balanceError={balanceError}
              refreshing={refreshing}
              onRefresh={refreshBalances}
            />
          )}
          {mode === "receive" && (
            <ReceiveCard key="receive" address={address} onClose={() => setMode("balance")} />
          )}
          {mode === "send" && (
            <SendCard
              key="send"
              usdcBalance={balances?.usdc}
              ethBalance={balances?.eth}
              settings={settings}
              spend={spend}
              onClose={() => setMode("balance")}
              onSent={() => {
                setMode("balance");
                void refreshBalances();
                loadLimits();
                loadHistory();
              }}
            />
          )}
        </AnimatePresence>
      </div>

      <div className="w-full max-w-md grid grid-cols-2 gap-3">
        <ActionButton
          icon={<ArrowDownLeft size={16} />}
          label={t("받기", "Receive")}
          onClick={() => setMode(mode === "receive" ? "balance" : "receive")}
          active={mode === "receive"}
        />
        <ActionButton
          icon={<ArrowUpRight size={16} />}
          label={t("보내기", "Send")}
          onClick={() => setMode(mode === "send" ? "balance" : "send")}
          active={mode === "send"}
          disabled={locked}
        />
      </div>

      <AnimatePresence>
        {showUnlock && (
          <UnlockSessionModal
            autoLimit={session.auto_limit}
            onUnlock={unlockSession}
            onClose={() => setShowUnlock(false)}
          />
        )}
      </AnimatePresence>
    </main>
  );
}
