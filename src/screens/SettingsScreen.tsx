// 설정 화면 — 한도·자율 결제·화이트리스트·네트워크·RPC·앱(상시 구동/자동 시작).
// 도메인별 그룹 카드 스택(토스/Apple 설정 관용구) — 길게 쌓이던 한 덩어리를 끊어 스캔 가능하게.

import { useEffect, useRef, useState, type ReactNode } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { AnimatePresence, motion } from "framer-motion";
import {
  ArrowUpCircle,
  Check,
  ExternalLink,
  Gauge,
  Globe,
  Info,
  Loader2,
  Network,
  Power,
  RefreshCw,
  Settings as SettingsIcon,
  ShieldCheck,
  Trash2,
  X,
  Zap,
} from "lucide-react";
import { cn } from "@/lib/cn";
import { CHAINS, chainFromId, type ChainConfig } from "@/lib/chain";
import { fmtAmount, shortenAddress } from "@/lib/format";
import { GITHUB_URL } from "@/lib/helpContent";
import type { Settings, SpendView } from "@/lib/types";
import type { UpdateHook } from "@/lib/useUpdate";
import { cardBase, enter, inputBase, modalCard, modalOverlay, primaryBtn, shell } from "@/components/ui";
import { chooseLang, lang, t, type Lang } from "@/lib/i18n";

const RPC_CUSTOM = "__custom__";

// Settings 의 boolean 필드만 — toggle() 이 실수로 문자열/숫자 필드를 뒤집지 못하게 좁힌다(코덱스 리뷰 low).
type BoolSettingKey =
  | "lock_on_blur"
  | "notify_auto"
  | "notify_hide_amount"
  | "auto_trusted_only"
  | "agent_lookup";

// RPC 프리셋은 활성 체인에 따라 달라진다(공식·PublicNode URL이 체인별로 다름).
// ①공식 ②로그 안 남긴다 표방하는 대체 공개 RPC ③직접 입력(본인 키/노드 = 진짜 프라이버시).
// "공식"의 url 은 빈 값 = "활성 체인의 공식 RPC를 따라간다"(백엔드 effective_rpc 가 해석). 구체 URL 을
// 저장하면 체인을 바꿔도 옛 RPC 에 고정되는 함정이 생긴다 (개발 18 코덱스 리뷰 #1).
function rpcPresets(chain: ChainConfig): { label: string; url: string }[] {
  const presets = [{ label: t(`${chain.name} 공식`, `${chain.name} official`), url: "" }];
  // PublicNode 가 없는 체인(Arc)은 이 줄 자체를 안 만든다 — 없는 대체 RPC 를 있는 척 채우면
  // "로그 안 남김"이라는 프라이버시 약속이 거짓이 된다. 그 체인은 공식 / 직접 입력 둘뿐.
  if (chain.publicNode) {
    presets.push({ label: t("PublicNode (로그 없음 표방)", "PublicNode (claims no logs)"), url: chain.publicNode });
  }
  return presets;
}

// 저장된 rpc_url 을 프리셋 표현으로 정규화. 옛 설정이 구체 공식 URL 을 박아 뒀어도 "공식"(빈 값)으로
// 인식되게 해, 드롭다운이 "직접 입력"으로 잘못 보이지 않게 한다.
function presetUrl(rpc: string | undefined, chain: ChainConfig): string {
  return !rpc || rpc === chain.defaultRpc ? "" : rpc;
}

export function SettingsScreen({
  current,
  spend,
  update,
  onClose,
}: {
  current: Settings | null;
  spend: SpendView | null;
  /** 업데이트 상태 (개발 31) — 지갑 화면과 같은 인스턴스를 쓴다. 화면을 여닫아도
   *  확인 결과·다운로드 진행이 유지되게(여기서 훅을 새로 부르면 열 때마다 초기화된다). */
  update: UpdateHook;
  onClose: () => void;
}) {
  const [s, setS] = useState<Settings | null>(current);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  // 저장 안 한 폼 변경이 있는가 — 언어 전환이 창을 다시 읽어서 이 값들을 버리기 때문에 필요하다
  // (코덱스 개발 42 P2: 한도를 낮추거나 테스트넷으로 바꿔 두고 언어를 누르면 그 변경이 사라졌다).
  const [dirty, setDirty] = useState(false);
  // RPC 직접 입력 모드(프리셋에 없는 URL이면 처음부터 직접 입력으로). 프리셋은 체인별로 다르다.
  const [rpcCustom, setRpcCustom] = useState<boolean>(() => {
    const c = chainFromId(current?.chain_id);
    return !rpcPresets(c).some((p) => p.url === presetUrl(current?.rpc_url, c));
  });
  // 화이트리스트 주소 목록 (Session 17) — null=로딩 중. 철회는 즉시 반영(저장 버튼과 무관).
  const [trusted, setTrusted] = useState<string[] | null>(null);
  const [showTrusted, setShowTrusted] = useState(false);
  // 로그인 시 자동 시작 (Session 18) — OS 로그인 아이템 상태가 진실 원천이라 즉시 적용(저장 버튼과 무관).
  const [autostart, setAutostart] = useState<boolean | null>(null);
  // 저장 성공 후 자동 닫기 타이머 — 언마운트/수동 닫기 때 정리해 옛 인스턴스 타이머가 부모를 다시 닫지 않게.
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // 이미 닫혔는지 — 저장 invoke 가 늦게 resolve 돼도(닫기를 그 사이 눌렀을 때) 상태 변경·타이머 생성을 막는다.
  const closed = useRef(false);

  useEffect(() => {
    invoke<string[]>("get_trusted_addrs")
      .then(setTrusted)
      .catch(() => setTrusted([]));
    invoke<boolean>("get_autostart")
      .then(setAutostart)
      .catch(() => setAutostart(false));
  }, []);

  // 언마운트 시 닫힘 표시 + 자동 닫기 타이머 정리(코덱스 리뷰 — 이중 onClose·늦은 setState 방지).
  useEffect(
    () => () => {
      closed.current = true;
      if (closeTimer.current) clearTimeout(closeTimer.current);
    },
    [],
  );

  async function toggleAutostart() {
    if (autostart === null) return;
    const next = !autostart;
    setAutostart(next); // 낙관적 반영, 실패하면 원복
    try {
      await invoke("set_autostart", { enabled: next });
    } catch {
      setAutostart(!next);
    }
  }

  async function revokeTrusted(addr: string) {
    try {
      await invoke("remove_trusted_addr", { to: addr });
      setTrusted((prev) => (prev ? prev.filter((a) => a !== addr) : prev));
    } catch {
      /* 철회 실패는 목록 유지 — 다음 시도에서 다시 */
    }
  }

  // 설정이 늦게 로드되면 동기화. rpcCustom 은 마운트 시 한 번만 초기화되므로(useState),
  // current 가 null 로 시작해 늦게 로드되는 경우 여기서 RPC 모드도 함께 재계산해야
  // 커스텀 RPC 가 "직접 입력"으로 안 뜨는 문제를 막는다 (개발 18 코덱스 리뷰 P3).
  useEffect(() => {
    if (current && !s) {
      setS(current);
      const c = chainFromId(current.chain_id);
      setRpcCustom(!rpcPresets(c).some((p) => p.url === presetUrl(current.rpc_url, c)));
    }
  }, [current, s]);

  // 화면 안에서 쓰는 활성 체인 = 편집 중인 s 기준(토글하면 즉시 프리셋·경고가 따라온다).
  const chain = chainFromId(s?.chain_id);
  const presets = rpcPresets(chain);

  function field(key: keyof Settings, value: string) {
    setS((prev) => (prev ? { ...prev, [key]: value } : prev));
    setSaved(false);
    setDirty(true);
  }

  function toggle(key: BoolSettingKey) {
    setS((prev) => (prev ? { ...prev, [key]: !prev[key] } : prev));
    setSaved(false);
    setDirty(true);
  }

  // 체인 전환 — RPC "선택 종류"는 유지한다. 공식/PublicNode 는 체인별로 정의돼 있으니 새 체인 값으로
  // 매핑하고, 직접 입력한 커스텀 URL 만 새 체인 공식으로 되돌린다(커스텀은 보통 체인 전용 — 예: Alchemy
  // 키에 base-sepolia/base-mainnet 이 박힘). 옛 체인 전용 RPC 가 새 체인에 남지 않게 한다는 개발 18
  // 코덱스 리뷰 #1 의 취지는 그대로 유지(커스텀만 리셋, 프리셋은 안전하게 재매핑). 저장해야 적용.
  function selectChain(id: number) {
    const next = chainFromId(id);
    setS((prev) => {
      if (!prev) return prev;
      const rpc_url = rpcCustom
        ? "" // 커스텀 → 새 체인 공식 (옛 체인 전용 URL 은 못 옮긴다)
        : chain.publicNode && presetUrl(prev.rpc_url, chain) === chain.publicNode
          ? (next.publicNode ?? "") // PublicNode 선택 유지 (없는 체인이면 그 체인 공식으로)
          : ""; // 공식 유지
      return { ...prev, chain_id: id, rpc_url };
    });
    setRpcCustom(false);
    setSaved(false);
    setDirty(true);
  }

  async function save() {
    if (!s) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("set_settings", { settings: s });
      if (closed.current) return; // 저장 도중 닫혔으면 늦은 setState·자동 닫기 타이머 생성을 막는다
      setSaved(true);
      setDirty(false);
      // 저장 성공 → "저장됨"을 잠깐 보여준 뒤 메인 화면으로 자동 복귀(매번 닫기를 또 누르는 번거로움 제거).
      // busy 를 유지해 닫히는 동안 폼 전체(아래 fieldset)가 잠긴다 → 이 0.5초 사이 입력이 조용히
      // 버려지는 일을 막는다(코덱스 리뷰 medium). onClose 는 부모가 설정을 다시 불러오게 한다.
      closeTimer.current = setTimeout(onClose, 500);
    } catch (e) {
      if (closed.current) return;
      setError(String(e));
      setBusy(false);
    }
  }

  // 수동 닫기 — 닫힘 표시 후 자동 닫기 타이머를 정리하고 닫는다(이중 onClose·늦은 setState 방지).
  function close() {
    closed.current = true;
    if (closeTimer.current) clearTimeout(closeTimer.current);
    onClose();
  }

  return (
    <main className={shell}>
      <div className="w-full max-w-md flex flex-col gap-3">
        <header className="flex items-center justify-between text-[12px] text-[var(--color-ink-500)]">
          <span className="flex items-center gap-2">
            <SettingsIcon size={12} className="text-[var(--color-accent)]" />
            {t("설정", "Settings")}
          </span>
          <button
            type="button"
            onClick={close}
            disabled={busy}
            className="hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD] transition-colors disabled:opacity-40 disabled:pointer-events-none"
          >
            {t("닫기", "Close")}
          </button>
        </header>

        {!s ? (
          <motion.section {...enter} className={cn(cardBase, "px-6 py-10")}>
            <div className="flex flex-col items-center">
              <Loader2 size={24} className="animate-spin text-[var(--color-accent)]" />
              <p className="mt-3 text-[13px] text-[var(--color-ink-500)]">
                {t("설정 불러오는 중…", "Loading settings…")}
              </p>
            </div>
          </motion.section>
        ) : (
          // 저장 직후 자동 닫기 대기(busy) 동안 폼 전체를 잠가, 그 사이 입력이 조용히 버려지지 않게 한다.
          <fieldset disabled={busy} className="flex flex-col gap-3 border-0 p-0 m-0 min-w-0">
            {/* 한도 — 단일/하루 누적 한도(USDC·ETH) */}
            <Section
              icon={<Gauge size={13} className="text-[var(--color-accent)]" />}
              title={t("한도", "Limits")}
              delay={0}
            >
              <p className="text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                {t(
                  "단일 거래·하루 누적 한도를 넘으면 송금을 막아줘요. AI의 큰 지출을 막기 위한 안전 장치예요.",
                  "Anything over the per-payment or daily limit is blocked. This is the guardrail against a big spend by the AI.",
                )}
              </p>
              <p className="mt-1.5 text-[11px] text-[var(--color-ink-300)]">
                {t("0으로 두면 무제한이에요.", "Leave a limit at 0 for no limit.")}
              </p>

              <div className="mt-5 space-y-3">
                <LimitGroup
                  token="USDC"
                  single={s.single_usdc}
                  daily={s.daily_usdc}
                  onSingle={(v) => field("single_usdc", v)}
                  onDaily={(v) => field("daily_usdc", v)}
                  usedToday={spend?.usdc}
                />
                {/* ETH 한도는 ETH 를 보낼 수 있는 체인에서만. 가스가 곧 USDC 인 체인(Arc)엔
                    네이티브 송금 경로 자체가 없어서(백엔드가 막는다) 걸 한도도 없다. */}
                {!chain.nativeIsUsdc && (
                  <LimitGroup
                    token="ETH"
                    single={s.single_eth}
                    daily={s.daily_eth}
                    onSingle={(v) => field("single_eth", v)}
                    onDaily={(v) => field("daily_eth", v)}
                    usedToday={spend?.eth}
                    frac={5}
                  />
                )}
              </div>
            </Section>

            {/* 자율 결제 (Session 14) — 한도 이하 AI 결제를 비번 없이 자동 승인 */}
            <Section
              icon={<Zap size={13} className="text-[var(--color-accent)]" />}
              title={t("자율 결제", "Autopay")}
              delay={0.04}
            >
              <p className="text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                {t(
                  "이 금액 이하의 AI 결제 요청은, 세션을 잠금 해제해 두면 비번 없이 자동 승인돼요. 넘는 금액은 항상 비번을 받아요.",
                  "With the session unlocked, AI payments at or under this amount go through without your password. Anything larger always asks.",
                )}
              </p>
              <p className="mt-1.5 text-[11px] text-[var(--color-ink-300)]">
                {t(
                  "한도를 0으로 두면 자율 결제가 꺼져요(항상 비번). 단일·하루 한도는 그대로 함께 적용돼요.",
                  "Set the limit to 0 to turn autopay off (always ask). The per-payment and daily limits still apply on top.",
                )}
              </p>

              <div className="mt-5 rounded-[var(--radius-card)] border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] px-4 py-4">
                <div className="grid grid-cols-2 gap-3">
                  <UnitInput
                    label={t("자율 한도", "Autopay limit")}
                    unit="USDC"
                    value={s.auto_approve_usdc}
                    onChange={(v) => field("auto_approve_usdc", v)}
                  />
                  <UnitInput
                    label={t("자동 잠금", "Auto-lock")}
                    unit={t("분", "min")}
                    intOnly
                    value={s.auto_lock_mins}
                    onChange={(v) => field("auto_lock_mins", v)}
                  />
                </div>
                <p className="mt-2.5 text-[11px] text-[var(--color-ink-300)]">
                  {t(
                    "유휴 시간이 지나면 자동으로 다시 잠겨요. 0이면 유휴 잠금 안 함(앱 종료·긴급 잠금 시엔 항상 잠김).",
                    "The session locks itself after this much idle time. 0 means no idle lock (quitting the app or the emergency lock still locks it).",
                  )}
                </p>
              </div>

              {/* 자율 결제 부속 토글·목록 — 낱개 카드 대신 구분선 리스트 하나로(개발 39 정리).
                  상자 안 상자가 다섯 겹 쌓이던 걸 한 컨테이너로 접어 시각 소음을 줄인다. */}
              <RowGroup>
                {/* 자리비움 자동 잠금 (Session 14) */}
                <ToggleRow
                  title={t("자리 비우면 자동 잠금", "Lock when you step away")}
                  desc={t(
                    "다른 앱으로 전환하거나 화면이 잠기면 세션을 즉시 잠가요.",
                    "Switching to another app or locking your screen locks the session at once.",
                  )}
                  checked={s.lock_on_blur}
                  onToggle={() => toggle("lock_on_blur")}
                />
                {/* 자율 결제 알림 (Session 15) — 비번 없이 나간 돈을 보호자가 사후 인지 */}
                <ToggleRow
                  title={t("자율 결제 알림", "Autopay notifications")}
                  desc={t(
                    "비번 없이 자동 승인된 결제를 macOS 알림으로 알려줘요.",
                    "A macOS notification tells you about payments approved without your password.",
                  )}
                  checked={s.notify_auto}
                  onToggle={() => toggle("notify_auto")}
                />
                {/* 알림 금액 숨기기 (개발 46) — macOS 알림은 잠금 화면·화면 공유에도 뜬다.
                    알림이 꺼져 있으면 의미 없는 토글이라 행 자체를 접는다. */}
                {s.notify_auto && (
                  <ToggleRow
                    title={t("알림에 금액 숨기기", "Hide amounts in notifications")}
                    desc={t(
                      "잠금 화면·화면 공유에 금액이 보이지 않게 「자율 결제」로만 알려줘요.",
                      "Notifications say just “Autopay,” so amounts don't show on the lock screen or a shared screen.",
                    )}
                    checked={s.notify_hide_amount}
                    onToggle={() => toggle("notify_hide_amount")}
                  />
                )}
                {/* 자율 결제 화이트리스트 (Session 16) — 처음 보는 주소는 자율 대상에서 제외 */}
                <ToggleRow
                  title={t("자율 결제 화이트리스트", "Autopay allowlist")}
                  desc={t(
                    "비번으로 승인한 적 있는 주소에만 자율 결제를 허용해요. 새 주소는 첫 결제만 비번이 필요해요.",
                    "Autopay only goes to addresses you've approved with your password before. A new address needs your password once.",
                  )}
                  checked={s.auto_trusted_only}
                  onToggle={() => toggle("auto_trusted_only")}
                />
                {/* 화이트리스트 주소 (Session 17) — 목록은 모달로 (많아지면 설정이 길어지니) */}
                <div className="flex items-center justify-between px-4 py-3.5">
                  <div className="min-w-0 pr-3">
                    <p className="text-[13px] tracking-tight">{t("화이트리스트 주소", "Allowed addresses")}</p>
                    <p className="mt-0.5 text-[11px] leading-snug text-[var(--color-ink-300)]">
                      {trusted === null
                        ? t("불러오는 중…", "Loading…")
                        : trusted.length === 0
                          ? t(
                              "아직 없어요 — 비번으로 승인하면 자동으로 학습돼요.",
                              "None yet — approving with your password adds one.",
                            )
                          : t(`${trusted.length}개 학습됨`, `${trusted.length} learned`)}
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={() => setShowTrusted(true)}
                    disabled={!trusted || trusted.length === 0}
                    className="shrink-0 text-[12px] text-[var(--color-accent)] disabled:text-[var(--color-ink-300)] hover:underline transition-colors"
                  >
                    {t("주소 보기", "View")}
                  </button>
                </div>
              </RowGroup>
            </Section>

            {/* 네트워크 (개발 20) — 메인넷 ↔ 테스트넷 런타임 전환. 개발 39부터 메인넷이
                왼쪽·기본(CHAINS 순서). 메인넷 선택 시엔 실돈 경고를 콜아웃으로 올려 위계를 준다. */}
            <Section
              icon={<Network size={13} className="text-[var(--color-accent)]" />}
              title={t("네트워크", "Network")}
              delay={0.08}
            >
              <p className="text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                {t(
                  "잔액·결제가 이뤄지는 블록체인이에요. 메인넷은 실제 자금이 오가고, 테스트넷은 가짜 코인으로 연습해요. 체인별로 한도·사용액·내역·화이트리스트가 따로 관리돼요.",
                  "The blockchain your balance and payments live on. Mainnet moves real funds; the testnet is practice with fake coins. Limits, spending, history, and the allowlist are kept per chain.",
                )}
              </p>
              <div className="mt-4 grid grid-cols-3 gap-2">
                {CHAINS.map((c) => {
                  const active = c.id === s.chain_id;
                  return (
                    <button
                      key={c.id}
                      type="button"
                      onClick={() => selectChain(c.id)}
                      className={cn(
                        "rounded-[var(--radius-card)] border px-2 py-3 text-[12px] tracking-tight transition-colors",
                        active
                          ? "border-[var(--color-accent)] bg-[var(--color-accent)]/10 text-[var(--color-accent)] font-medium"
                          : "border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] text-[var(--color-ink-500)] hover:border-[var(--color-ink-300)]",
                      )}
                    >
                      {c.name}
                      <span className="block mt-0.5 text-[10px] text-[var(--color-ink-300)]">
                        {c.testnet ? t("연습용", "Practice") : t("실제 자금", "Real funds")}
                      </span>
                    </button>
                  );
                })}
              </div>
              {chain.testnet ? (
                <p className="mt-2.5 text-[11px] text-[var(--color-ink-300)]">
                  {t("바꾼 뒤 저장을 누르면 적용돼요.", "Press save to apply the change.")}
                </p>
              ) : (
                // 실돈 경고는 문장 한 줄이 아니라 콜아웃 — 일상 설정과 위험 설정의 시각 위계 분리(개발 39).
                <div className="mt-2.5 rounded-[var(--radius-card)] border border-amber-500/30 bg-amber-500/5 px-3.5 py-2.5">
                  <p className="text-[11px] leading-relaxed text-amber-600 dark:text-amber-500">
                    {t(
                      "메인넷은 실제 USDC·ETH가 오가요. 운영 예산만 소액 충전하고, 위의 한도를 확인해 두세요. 바꾼 뒤 저장을 누르면 적용돼요.",
                      "Mainnet moves real USDC and ETH. Top up only what the agent needs, and check your limits above. Press save to apply the change.",
                    )}
                  </p>
                </div>
              )}

              {/* ERC-8004 신원 조회 (개발 47) — 레지스트리가 배포된 체인에서만 보여준다.
                  네트워크 섹션에 두는 이유: 하는 일이 "이 체인의 레지스트리를 읽는 것"이고,
                  체인을 바꾸면 있고 없고가 갈리는 설정이라 선택 바로 아래가 제자리다. */}
              {chain.erc8004 && (
                <RowGroup>
                  <ToggleRow
                    title={t("에이전트 신원 조회 (ERC-8004)", "Agent identity lookup (ERC-8004)")}
                    desc={t(
                      "AI가 상대의 에이전트 번호를 알려주면, 받는 주소·도메인이 온체인 기록과 같은지 대조해 승인 창에 알려줘요. 쓰던 RPC로 읽기만 하고, 상대 웹사이트에는 접속하지 않아요.",
                      "When the AI knows the other side's agent number, Kura checks whether the address and domain match the on-chain record and says so in the approval window. It only reads over your existing RPC — it never visits their website.",
                    )}
                    checked={s.agent_lookup}
                    onToggle={() => toggle("agent_lookup")}
                  />
                </RowGroup>
              )}
            </Section>

            {/* RPC 서버 (Session 14) — 프라이버시: 공개 RPC는 내 IP↔지갑을 볼 수 있음 */}
            <Section
              icon={<Globe size={13} className="text-[var(--color-accent)]" />}
              title={t("RPC 서버", "RPC server")}
              delay={0.12}
            >
              <p className="text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                {t(
                  "잔액 조회·송금에 쓰는 서버예요. 공개 RPC는 내 IP와 지갑 주소를 볼 수 있어요. 프라이버시가 중요하면 본인 키 엔드포인트(Alchemy/노드 등)를 직접 넣으세요.",
                  "The server used to read balances and send payments. A public RPC can see your IP and wallet address. If that matters to you, paste your own endpoint (Alchemy, your own node, and so on).",
                )}
              </p>
              <select
                value={rpcCustom ? RPC_CUSTOM : presetUrl(s.rpc_url, chain)}
                onChange={(e) => {
                  const v = e.target.value;
                  if (v === RPC_CUSTOM) {
                    setRpcCustom(true);
                  } else {
                    setRpcCustom(false);
                    field("rpc_url", v);
                  }
                }}
                className={cn(inputBase, "mt-4 text-[13px]")}
              >
                {presets.map((p) => (
                  <option key={p.label} value={p.url}>
                    {p.label}
                  </option>
                ))}
                <option value={RPC_CUSTOM}>{t("직접 입력…", "Enter your own…")}</option>
              </select>
              {rpcCustom && (
                <input
                  value={s.rpc_url}
                  onChange={(e) => field("rpc_url", e.target.value)}
                  placeholder="https://…"
                  spellCheck={false}
                  autoCapitalize="off"
                  className={cn(inputBase, "mt-2 font-mono text-[12px]")}
                />
              )}
            </Section>

            {/* 앱 (Session 18) — 상시 구동: 창을 닫아도 백그라운드에서 결제 요청을 받는다 */}
            <Section
              icon={<Power size={13} className="text-[var(--color-accent)]" />}
              title={t("앱", "App")}
              delay={0.16}
            >
              <p className="text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                {t(
                  "창을 닫아도 앱은 종료되지 않고 백그라운드에서 결제 요청을 받아요. 완전히 끄려면 ⌘Q를 누르세요.",
                  "Closing the window doesn't quit Kura — it keeps taking payment requests in the background. Press ⌘Q to quit for real.",
                )}
              </p>
              <RowGroup>
                <ToggleRow
                  title={t("로그인 시 자동 시작", "Start at login")}
                  desc={t(
                    "Mac에 로그인하면 지갑이 자동으로 켜져요. 끄고 켜는 즉시 적용돼요.",
                    "The wallet opens when you log in to your Mac. This applies the moment you flip it.",
                  )}
                  checked={autostart ?? false}
                  onToggle={toggleAutostart}
                />
                <LanguageRow dirty={dirty} />
              </RowGroup>
            </Section>

            {error && <p className="text-[12px] text-red-500 font-mono break-all">{error}</p>}

            <button type="button" onClick={save} disabled={busy} className={cn(primaryBtn, "mt-2")}>
              {saved ? (
                <Check size={15} />
              ) : busy ? (
                <Loader2 size={15} className="animate-spin" />
              ) : null}
              {saved ? t("저장됨", "Saved") : t("저장하고 닫기", "Save and close")}
            </button>
          </fieldset>
        )}

        {/* 정보 — 읽기 전용이라 fieldset(저장 중 잠금) 밖에 두고, 설정 로드가 실패해도 보이게
            조건 분기 바깥에 둔다. 버전·소스 링크는 "이 앱이 뭘 하는지 직접 확인할 통로"라
            설정이 안 열리는 상황일수록 오히려 더 필요하다. */}
        {/* 폴백 false = 신규 기본(개발 39). 실제 값은 백엔드 설정이 로드되면 그걸 따른다. */}
        <AboutSection update={update} autoCheck={current?.auto_check_update ?? false} />
      </div>

      <AnimatePresence>
        {showTrusted && trusted && (
          <TrustedAddrsModal
            addrs={trusted}
            onRevoke={revokeTrusted}
            onClose={() => setShowTrusted(false)}
          />
        )}
      </AnimatePresence>
    </main>
  );
}

/** 설정 화면의 그룹 섹션 — 아이콘+제목 헤더를 단 카드. 길게 쌓이던 설정을 도메인별로 끊어 준다.
 *  delay = 카드별 미세 스태거(코다와리 — 위에서부터 차례로 떠오르게). */
function Section({
  icon,
  title,
  delay = 0,
  children,
}: {
  icon: ReactNode;
  title: string;
  delay?: number;
  children: ReactNode;
}) {
  return (
    <motion.section
      {...enter}
      transition={{ ...enter.transition, delay }}
      className={cn(cardBase, "px-6 py-5")}
    >
      <div className="flex items-center gap-1.5">
        {icon}
        <span className="text-[13px] tracking-tight">{title}</span>
      </div>
      <div className="mt-3">{children}</div>
    </motion.section>
  );
}

/** 정보 섹션 (개발 27) — 버전 + "직접 감사할 수 있다"는 신뢰 프레이밍 + 소스 링크.
 *  키를 맡기는 앱이라 "믿어달라"가 아니라 "직접 읽어보라"가 유일하게 정직한 근거 —
 *  그래서 묻힌 링크가 아니라 설정 하단의 제 카드로 둔다. */
function AboutSection({ update, autoCheck }: { update: UpdateHook; autoCheck: boolean }) {
  // 버전은 tauri.conf.json(=설치된 앱 번들)이 진실 원천. 실패해도 섹션 자체는 뜬다.
  const [version, setVersion] = useState<string | null>(null);
  // 앱 관리 필드라 "저장하고 닫기"와 무관하게 즉시 적용(자동 시작 토글과 같은 결).
  const [autoCheckOn, setAutoCheckOn] = useState(autoCheck);

  useEffect(() => {
    let alive = true;
    getVersion()
      .then((v) => alive && setVersion(v))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  // 설정이 늦게 로드되면 동기화 (RPC 모드가 같은 이유로 하는 것과 같은 결 — 개발 18 P3).
  // 이 화면이 열려 있는 동안 current 는 안 바뀌므로(get_settings 는 마운트·닫기에서만 돈다)
  // 낙관적 토글을 되돌리지 않는다.
  useEffect(() => setAutoCheckOn(autoCheck), [autoCheck]);

  async function toggleAutoCheck() {
    const next = !autoCheckOn;
    setAutoCheckOn(next); // 낙관적 반영, 실패하면 원복
    try {
      await invoke("set_auto_check_update", { enabled: next });
    } catch {
      setAutoCheckOn(!next);
    }
  }

  return (
    <Section icon={<Info size={13} className="text-[var(--color-accent)]" />} title={t("정보", "About")} delay={0.2}>
      <div className="flex items-baseline justify-between">
        <span className="text-[13px] tracking-tight">Kura</span>
        <span className="num font-mono text-[11px] text-[var(--color-ink-300)]">
          {version ? `v${version}` : "—"}
        </span>
      </div>

      <UpdateBlock update={update} autoCheckOn={autoCheckOn} onToggleAutoCheck={toggleAutoCheck} />

      {/* 개발 29에서 현재형으로 복원 — 저장소가 실제로 공개됐다. 다시 비공개로 돌리는 일이
          생기면 이 문장부터 미래형으로 되돌릴 것. 신뢰의 근거로 내세운 문장이라, 사실과
          어긋나면 뒤에 단서를 붙이는 걸로는 못 고친다(개발 27 코덱스 2라운드 지적). */}
      <p className="mt-3 text-[13px] leading-relaxed text-[var(--color-ink-500)]">
        {t(
          "Kura는 소스 코드가 MIT 라이선스로 공개돼 있어요. 열쇠와 돈을 다루는 코드를 직접 읽어 확인하고, 직접 빌드해서 쓸 수 있어요.",
          "Kura's source is public under the MIT license. You can read the code that handles your key and your money, and build it yourself.",
        )}
      </p>
      {/* 저장(로컬)과 전송(RPC)을 나눠 쓴 건 개발 27 판단 그대로. 개발 29 에서 한 겹 더 —
          "내역이 ~/.jigap 에만 있다"가 "내 거래가 비공개다"로 읽히면 안 된다. 보낸 거래는
          공개 체인에 영구히 남는다. 지갑에서 이 오해는 값이 비싸다. */}
      <p className="mt-1.5 text-[11px] leading-snug text-[var(--color-ink-300)]">
        {t(
          <>
            지갑 키·설정·거래 내역은 이 컴퓨터의 <span className="font-mono">~/.jigap</span>{" "}
            폴더에만 저장돼요. 잔액을 확인하고 송금할 때만 위에서 고른 RPC 서버로 요청이 나가요 —
            그쪽은 내 주소를 볼 수 있고, 보낸 거래는 공개 블록체인에 남아요.
          </>,
          <>
            Your key, settings, and history stay in the <span className="font-mono">~/.jigap</span>{" "}
            folder on this computer. Requests only leave for the RPC server you picked above, to read
            balances and send payments — that server can see your address, and payments you send stay
            on a public blockchain.
          </>,
        )}
      </p>

      <div className="mt-4 flex items-center justify-between">
        <button
          type="button"
          onClick={() => openUrl(GITHUB_URL).catch(() => {})}
          className="inline-flex items-center gap-1.5 text-[12px] text-[var(--color-accent)] hover:underline transition-colors"
        >
          <ExternalLink size={12} />
          {t("소스 코드 보기", "Read the source")}
        </button>
        <span className="text-[11px] text-[var(--color-ink-300)]">{t("MIT 라이선스", "MIT license")}</span>
      </div>
      {/* 글꼴 고지 — 앱에 Pretendard 를 함께 담아 배포하므로 OFL 1.1 이 고지를 요구한다.
          라이선스 전문은 번들 안(fonts/LICENSE-Pretendard.txt)에도 같이 들어간다. */}
      <p className="mt-3 text-[10px] text-[var(--color-ink-300)]">
        {t("글꼴 Pretendard — SIL Open Font License 1.1", "Pretendard typeface — SIL Open Font License 1.1")}
      </p>
    </Section>
  );
}

/** 화이트리스트 주소 목록 모달 (Session 17) — 주소가 많아져도 설정 화면이 안 길어지게 분리. */
function TrustedAddrsModal({
  addrs,
  onRevoke,
  onClose,
}: {
  addrs: string[];
  onRevoke: (addr: string) => Promise<void>;
  onClose: () => void;
}) {
  // 철회는 실수 방지로 행별 2단계 확인 (휴지통 → "철회할까요?" → 철회).
  const [confirming, setConfirming] = useState<string | null>(null);

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.2 }}
      onClick={onClose}
      className={modalOverlay}
    >
      <motion.section
        initial={{ scale: 0.9, opacity: 0, y: 12 }}
        animate={{ scale: 1, opacity: 1, y: 0 }}
        exit={{ scale: 0.95, opacity: 0 }}
        transition={{ type: "spring", stiffness: 340, damping: 24 }}
        onClick={(e) => e.stopPropagation()}
        className={cn(modalCard, "px-7 py-6")}
      >
        <div className="flex items-center justify-between">
          <span className="flex items-center gap-1.5 text-[11px] tracking-[0.04em] text-[var(--color-accent)]">
            <ShieldCheck size={13} />
            {t("화이트리스트 주소", "Allowed addresses")}
          </span>
          <button
            type="button"
            onClick={onClose}
            aria-label={t("닫기", "Close")}
            className="p-1 text-[var(--color-ink-300)] hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD] transition-colors"
          >
            <X size={14} />
          </button>
        </div>
        <p className="mt-3 text-[12px] leading-relaxed text-[var(--color-ink-500)]">
          {t(
            "비번으로 승인하면 자동으로 학습돼요. 철회하면 다음 결제 때 다시 비번을 받아요.",
            "Approving with your password adds an address here. Remove one and the next payment to it asks again.",
          )}
        </p>

        {addrs.length === 0 ? (
          <p className="mt-5 mb-1 text-[12px] text-[var(--color-ink-300)]">
            {t(
              "모두 철회했어요. 비번으로 승인하면 다시 학습돼요.",
              "All removed. Approving with your password adds them back.",
            )}
          </p>
        ) : (
          <ul className="mt-4 max-h-72 overflow-y-auto space-y-0.5 -mx-2">
            {addrs.map((addr) => (
              <li
                key={addr}
                className="flex items-center justify-between gap-2 rounded-lg px-2 py-2 hover:bg-[var(--color-ivory-200)] dark:hover:bg-[var(--color-night-700)]/50 transition-colors"
              >
                <span className="num font-mono text-[12px] text-[var(--color-ink-500)]" title={addr}>
                  {shortenAddress(addr)}
                </span>
                {confirming === addr ? (
                  <span className="flex items-center gap-2 shrink-0">
                    <span className="text-[11px] text-[var(--color-ink-500)]">{t("철회할까요?", "Remove it?")}</span>
                    <button
                      type="button"
                      onClick={() => {
                        setConfirming(null);
                        void onRevoke(addr);
                      }}
                      className="text-[11px] font-medium text-red-500 hover:underline"
                    >
                      {t("철회", "Remove")}
                    </button>
                    <button
                      type="button"
                      onClick={() => setConfirming(null)}
                      className="text-[11px] text-[var(--color-ink-300)] hover:underline"
                    >
                      {t("취소", "Cancel")}
                    </button>
                  </span>
                ) : (
                  <button
                    type="button"
                    onClick={() => setConfirming(addr)}
                    aria-label={t(
                      `${shortenAddress(addr)} 철회`,
                      `Remove ${shortenAddress(addr)}`,
                    )}
                    title={t("화이트리스트에서 철회", "Remove from the allowlist")}
                    className="shrink-0 p-1 rounded-md text-[var(--color-ink-300)] hover:text-red-500 transition-colors"
                  >
                    <Trash2 size={13} />
                  </button>
                )}
              </li>
            ))}
          </ul>
        )}
      </motion.section>
    </motion.div>
  );
}

/** 업데이트 블록 (개발 31) — 정보 카드 안.
 *
 *  여기가 **설치 승인 화면**이다. 지갑에 새 코드를 넣는 일이라, 버전과 릴리스 노트를
 *  보여준 뒤 사람이 누를 때만 설치된다(update.rs 가 같은 정책을 백엔드에서 강제한다).
 *  자동으로 도는 건 "확인"까지고, 그 확인조차 아래 토글로 끌 수 있다. */
function UpdateBlock({
  update,
  autoCheckOn,
  onToggleAutoCheck,
}: {
  update: UpdateHook;
  autoCheckOn: boolean;
  onToggleAutoCheck: () => void;
}) {
  const { info, checking, installing, progress, error, upToDate } = update;
  const pct =
    progress && progress.total
      ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100))
      : null;

  return (
    <div className="mt-4 rounded-[var(--radius-card)] border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] px-4 py-3.5">
      {installing ? (
        // 설치 중 — 다른 조작을 안 내준다. 성공하면 앱이 재시작되므로 이 화면이 마지막이다.
        <div>
          <p className="flex items-center gap-2 text-[13px] tracking-tight">
            <Loader2 size={13} className="animate-spin text-[var(--color-accent)]" />
            {t("업데이트 설치 중…", "Installing the update…")}
          </p>
          {/* 크기를 모르면(Content-Length 없음) 퍼센트 대신 받은 용량만 보여준다 —
              0% 에 멈춘 것처럼 보이는 게 제일 나쁘다. */}
          <div className="mt-2.5 h-1 w-full overflow-hidden rounded-full bg-[var(--color-ivory-400)] dark:bg-[var(--color-night-700)]">
            <div
              className={cn(
                "h-full bg-[var(--color-accent)] transition-[width] duration-[var(--duration-base)]",
                pct === null && "animate-pulse w-1/3",
              )}
              style={pct === null ? undefined : { width: `${pct}%` }}
            />
          </div>
          <p className="mt-2 text-[11px] text-[var(--color-ink-300)]">
            {pct === null
              ? t(
                  `${fmtBytes(progress?.downloaded ?? 0)} 받는 중`,
                  `${fmtBytes(progress?.downloaded ?? 0)} downloaded`,
                )
              : `${pct}% · ${fmtBytes(progress?.downloaded ?? 0)}`}
            {" — "}
            {t("끝나면 앱이 저절로 다시 시작돼요.", "Kura restarts itself when it's done.")}
          </p>
        </div>
      ) : info ? (
        <div>
          <div className="flex items-baseline justify-between gap-3">
            <p className="text-[13px] tracking-tight">
              {t(
                <>
                  새 버전 <span className="num font-mono">{info.version}</span>
                </>,
                <>
                  Version <span className="num font-mono">{info.version}</span> is available
                </>,
              )}
            </p>
            <span className="num font-mono text-[11px] text-[var(--color-ink-300)]">
              {t(`지금 ${info.current_version}`, `now ${info.current_version}`)}
            </span>
          </div>
          {info.notes && (
            // 릴리스 노트는 사람이 "이 코드를 내 지갑에 넣을지" 판단하는 유일한 근거라
            // 자르지 않고 그대로 둔다(길면 카드가 늘어난다 — 그게 맞다).
            <p className="mt-2 whitespace-pre-wrap text-[11px] leading-snug text-[var(--color-ink-500)]">
              {info.notes}
            </p>
          )}
          <button
            type="button"
            onClick={() => void update.install()}
            className={cn(primaryBtn, "mt-3 h-9 text-[13px]")}
          >
            <ArrowUpCircle size={14} />
            {t("지금 설치하고 다시 시작", "Install now and restart")}
          </button>
        </div>
      ) : (
        <div className="flex items-center justify-between gap-3">
          <p className="min-w-0 text-[12px] text-[var(--color-ink-500)]">
            {checking
              ? t("확인 중…", "Checking…")
              : upToDate
                ? t("최신 버전이에요.", "You're on the latest version.")
                : t("업데이트를 확인할 수 있어요.", "You can check for an update.")}
          </p>
          <button
            type="button"
            onClick={() => void update.check()}
            disabled={checking}
            className={cn(
              "shrink-0 inline-flex items-center gap-1.5 h-8 px-3 rounded-[var(--radius-pill)]",
              "text-[12px] tracking-tight",
              "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
              "hover:border-[var(--color-accent)] disabled:opacity-40",
              "transition-colors duration-[var(--duration-base)]",
            )}
          >
            <RefreshCw size={12} className={cn(checking && "animate-spin")} />
            {t("업데이트 확인", "Check now")}
          </button>
        </div>
      )}

      {error && <p className="mt-2 text-[11px] leading-snug text-red-500">{error}</p>}

      <div className="mt-3 flex items-center justify-between gap-3 border-t border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] pt-3">
        <div className="min-w-0 pr-1">
          <p className="text-[12px] tracking-tight">{t("시작할 때 확인", "Check at startup")}</p>
          {/* 무엇이 나가는지 적는다 — 로컬 전용을 내세운 앱이라 조용한 바깥 통신이 있으면 안 된다. */}
          <p className="mt-0.5 text-[11px] leading-snug text-[var(--color-ink-300)]">
            {t(
              "앱을 켤 때 새 버전이 있는지 깃허브에 물어봐요(현재 버전과 IP가 그쪽에 남아요).",
              "On launch, Kura asks GitHub whether a new version exists (your current version and IP show up there).",
            )}
          </p>
        </div>
        <Switch
          checked={autoCheckOn}
          onToggle={onToggleAutoCheck}
          label={t("시작할 때 확인", "Check at startup")}
        />
      </div>
    </div>
  );
}

/** 바이트를 사람이 읽는 단위로. 업데이트 진행률에만 쓴다. */
function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** 관련 행들을 하나의 카드로 묶는 구분선 리스트 (개발 39 정리 — 토스/Apple 설정 관용구).
 *  행마다 테두리 상자를 두르지 않고, 컨테이너 하나 + 헤어라인 구분선으로 접는다. */
function RowGroup({ children }: { children: ReactNode }) {
  return (
    <div
      className={cn(
        "mt-4 rounded-[var(--radius-card)] border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)]",
        "divide-y divide-[var(--color-ivory-300)] dark:divide-[var(--color-night-700)]",
      )}
    >
      {children}
    </div>
  );
}

/** 공용 on/off 스위치 — ToggleRow 와 업데이트 블록이 같은 스위치를 쓴다(개발 39 정리 전엔
 *  같은 마크업이 두 벌 복사돼 있었다). 접근성 라벨은 행 제목을 그대로 받는다. */
function Switch({
  checked,
  onToggle,
  label,
}: {
  checked: boolean;
  onToggle: () => void;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={onToggle}
      className={cn(
        "shrink-0 relative w-10 h-6 rounded-full transition-colors duration-[var(--duration-base)]",
        checked
          ? "bg-[var(--color-accent)]"
          : "bg-[var(--color-ivory-400)] dark:bg-[var(--color-night-700)]",
      )}
    >
      <span
        className={cn(
          "absolute top-1 w-4 h-4 rounded-full bg-white shadow-sm transition-all duration-[var(--duration-base)]",
          checked ? "left-5" : "left-1",
        )}
      />
    </button>
  );
}

/** 설정 화면 공용 on/off 토글 행 — RowGroup 안에서 쓴다(테두리는 그룹이 두른다). */
function ToggleRow({
  title,
  desc,
  checked,
  onToggle,
}: {
  title: string;
  desc: string;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="flex items-center justify-between px-4 py-3.5">
      <div className="min-w-0 pr-3">
        <p className="text-[13px] tracking-tight">{title}</p>
        <p className="mt-0.5 text-[11px] leading-snug text-[var(--color-ink-300)]">{desc}</p>
      </div>
      <Switch checked={checked} onToggle={onToggle} label={title} />
    </div>
  );
}

/** 언어 선택 행 (개발 42) — 고른 즉시 저장하고 창을 새 언어로 다시 읽는다.
 *
 *  "저장하고 닫기"에 묶지 않는다: 언어는 폼 값이 아니라 앱 관리 필드고(자동 시작·업데이트
 *  확인과 같은 결), 저장 버튼을 누르기 전까지 옛 언어로 남아 있으면 뭘 고른 건지 알 수 없다.
 *  저장이 실패하면 언어를 바꾸지 않고 그 자리에서 말해 준다 — 화면만 바뀌고 다음 실행에
 *  되돌아오는 게 제일 나쁘다. */
function LanguageRow({ dirty }: { dirty: boolean }) {
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const current = lang();

  function pick(next: Lang) {
    if (next === current || busy || dirty) return;
    setBusy(true);
    setFailed(false);
    // 성공하면 창을 다시 읽으므로 이 컴포넌트는 그대로 사라진다(busy 를 되돌릴 필요 없음).
    chooseLang(next).catch(() => {
      setBusy(false);
      setFailed(true);
    });
  }

  return (
    <div className="flex items-center justify-between px-4 py-3.5">
      <div className="min-w-0 pr-3">
        <p className="text-[13px] tracking-tight">{t("언어", "Language")}</p>
        <p
          className={cn(
            "mt-0.5 text-[11px] leading-snug",
            failed ? "text-red-500/90" : "text-[var(--color-ink-300)]",
          )}
        >
          {failed
            ? t("언어를 저장하지 못했어요. 그대로 뒀어요.", "Couldn't save the language, so nothing changed.")
            : dirty
              ? t(
                  "저장 안 한 변경이 있어요. 먼저 저장한 뒤에 바꿔주세요.",
                  "You have unsaved changes. Save them first, then switch.",
                )
              : t("고르면 창이 새 언어로 다시 열려요.", "Picking one reopens the window in that language.")}
        </p>
      </div>
      <div className="shrink-0 flex gap-1 p-1 rounded-[var(--radius-pill)] bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-900)]">
        {(
          [
            { code: "ko", label: "한국어" },
            { code: "en", label: "English" },
          ] as { code: Lang; label: string }[]
        ).map((o) => (
          <button
            key={o.code}
            type="button"
            onClick={() => pick(o.code)}
            aria-pressed={o.code === current}
            disabled={dirty}
            className={cn(
              "h-7 px-3 rounded-[var(--radius-pill)] text-[12px] tracking-tight",
              "transition-colors duration-[var(--duration-base)]",
              "disabled:opacity-40 disabled:cursor-not-allowed",
              o.code === current
                ? "bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-700)] text-[var(--color-ink-900)] dark:text-[#E8E5DD] shadow-[var(--shadow-soft)]"
                : "text-[var(--color-ink-500)] hover:text-[var(--color-ink-700)]",
            )}
          >
            {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}

/** 단위가 붙는 숫자 입력 — 단위를 필드 안 오른쪽에 고정해 표기·정렬을 통일한다(개발 39 정리).
 *  전엔 라벨에 "(USDC)" "(분)" 처럼 괄호로 섞여 붙어 있었다. */
function UnitInput({
  label,
  unit,
  value,
  onChange,
  intOnly = false,
}: {
  label: string;
  unit: string;
  value: string;
  onChange: (v: string) => void;
  intOnly?: boolean;
}) {
  return (
    <label className="block">
      <span className="text-[11px] text-[var(--color-ink-500)]">{label}</span>
      <span className="relative block mt-1.5">
        <input
          value={value}
          onChange={(e) => onChange(e.target.value)}
          inputMode={intOnly ? "numeric" : "decimal"}
          className={cn(inputBase, "num text-[14px] pr-14")}
        />
        <span
          aria-hidden
          className="pointer-events-none absolute right-3.5 top-1/2 -translate-y-1/2 text-[11px] text-[var(--color-ink-300)]"
        >
          {unit}
        </span>
      </span>
    </label>
  );
}

function LimitGroup({
  token,
  single,
  daily,
  onSingle,
  onDaily,
  usedToday,
  frac = 2,
}: {
  token: string;
  single: string;
  daily: string;
  onSingle: (v: string) => void;
  onDaily: (v: string) => void;
  usedToday: string | undefined;
  frac?: number;
}) {
  return (
    <div className="rounded-[var(--radius-card)] border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] px-4 py-4">
      <div className="flex items-center justify-between">
        <span className="text-[13px] tracking-tight">{token}</span>
        <span className="text-[11px] text-[var(--color-ink-300)] num">
          {t(`오늘 ${fmtAmount(usedToday, frac)} 사용`, `${fmtAmount(usedToday, frac)} used today`)}
        </span>
      </div>
      <div className="mt-3 grid grid-cols-2 gap-3">
        <UnitInput
          label={t("단일 거래 한도", "Per payment")}
          unit={token}
          value={single}
          onChange={onSingle}
        />
        <UnitInput
          label={t("하루 누적 한도", "Per day")}
          unit={token}
          value={daily}
          onChange={onDaily}
        />
      </div>
    </div>
  );
}
