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

const RPC_CUSTOM = "__custom__";

// Settings 의 boolean 필드만 — toggle() 이 실수로 문자열/숫자 필드를 뒤집지 못하게 좁힌다(코덱스 리뷰 low).
type BoolSettingKey = "lock_on_blur" | "notify_auto" | "auto_trusted_only";

// RPC 프리셋은 활성 체인에 따라 달라진다(공식·PublicNode URL이 체인별로 다름).
// ①공식 ②로그 안 남긴다 표방하는 대체 공개 RPC ③직접 입력(본인 키/노드 = 진짜 프라이버시).
// "공식"의 url 은 빈 값 = "활성 체인의 공식 RPC를 따라간다"(백엔드 effective_rpc 가 해석). 구체 URL 을
// 저장하면 체인을 바꿔도 옛 RPC 에 고정되는 함정이 생긴다 (개발 18 코덱스 리뷰 #1).
function rpcPresets(chain: ChainConfig): { label: string; url: string }[] {
  return [
    { label: `${chain.name} 공식`, url: "" },
    { label: "PublicNode (로그 없음 표방)", url: chain.publicNode },
  ];
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
  }

  function toggle(key: BoolSettingKey) {
    setS((prev) => (prev ? { ...prev, [key]: !prev[key] } : prev));
    setSaved(false);
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
        : presetUrl(prev.rpc_url, chain) === chain.publicNode
          ? next.publicNode // PublicNode 선택 유지 (새 체인의 PublicNode 로)
          : ""; // 공식 유지
      return { ...prev, chain_id: id, rpc_url };
    });
    setRpcCustom(false);
    setSaved(false);
  }

  async function save() {
    if (!s) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("set_settings", { settings: s });
      if (closed.current) return; // 저장 도중 닫혔으면 늦은 setState·자동 닫기 타이머 생성을 막는다
      setSaved(true);
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
            설정
          </span>
          <button
            type="button"
            onClick={close}
            disabled={busy}
            className="hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD] transition-colors disabled:opacity-40 disabled:pointer-events-none"
          >
            닫기
          </button>
        </header>

        {!s ? (
          <motion.section {...enter} className={cn(cardBase, "px-6 py-10")}>
            <div className="flex flex-col items-center">
              <Loader2 size={24} className="animate-spin text-[var(--color-accent)]" />
              <p className="mt-3 text-[13px] text-[var(--color-ink-500)]">설정 불러오는 중…</p>
            </div>
          </motion.section>
        ) : (
          // 저장 직후 자동 닫기 대기(busy) 동안 폼 전체를 잠가, 그 사이 입력이 조용히 버려지지 않게 한다.
          <fieldset disabled={busy} className="flex flex-col gap-3 border-0 p-0 m-0 min-w-0">
            {/* 한도 — 단일/하루 누적 한도(USDC·ETH) */}
            <Section
              icon={<Gauge size={13} className="text-[var(--color-accent)]" />}
              title="한도"
              delay={0}
            >
              <p className="text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                단일 거래·하루 누적 한도를 넘으면 송금을 막아줘요. AI의 큰 지출을 막기 위한 안전 장치예요.
              </p>
              <p className="mt-1.5 text-[11px] text-[var(--color-ink-300)]">0으로 두면 무제한이에요.</p>

              <div className="mt-5 space-y-3">
                <LimitGroup
                  token="USDC"
                  single={s.single_usdc}
                  daily={s.daily_usdc}
                  onSingle={(v) => field("single_usdc", v)}
                  onDaily={(v) => field("daily_usdc", v)}
                  usedToday={spend?.usdc}
                />
                <LimitGroup
                  token="ETH"
                  single={s.single_eth}
                  daily={s.daily_eth}
                  onSingle={(v) => field("single_eth", v)}
                  onDaily={(v) => field("daily_eth", v)}
                  usedToday={spend?.eth}
                  frac={5}
                />
              </div>
            </Section>

            {/* 자율 결제 (Session 14) — 한도 이하 AI 결제를 비번 없이 자동 승인 */}
            <Section
              icon={<Zap size={13} className="text-[var(--color-accent)]" />}
              title="자율 결제"
              delay={0.04}
            >
              <p className="text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                이 금액 이하의 AI 결제 요청은, 세션을 잠금 해제해 두면 비번 없이 자동 승인돼요.
                넘는 금액은 항상 비번을 받아요.
              </p>
              <p className="mt-1.5 text-[11px] text-[var(--color-ink-300)]">
                한도를 0으로 두면 자율 결제가 꺼져요(항상 비번). 단일·하루 한도는 그대로 함께 적용돼요.
              </p>

              <div className="mt-5 rounded-[var(--radius-card)] border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] px-4 py-4">
                <div className="grid grid-cols-2 gap-3">
                  <label className="block">
                    <span className="text-[11px] text-[var(--color-ink-500)]">자율 한도 (USDC)</span>
                    <input
                      value={s.auto_approve_usdc}
                      onChange={(e) => field("auto_approve_usdc", e.target.value)}
                      inputMode="decimal"
                      className={cn(inputBase, "mt-1.5 num text-[14px]")}
                    />
                  </label>
                  <label className="block">
                    <span className="text-[11px] text-[var(--color-ink-500)]">자동 잠금 (분)</span>
                    <input
                      value={s.auto_lock_mins}
                      onChange={(e) => field("auto_lock_mins", e.target.value)}
                      inputMode="numeric"
                      className={cn(inputBase, "mt-1.5 num text-[14px]")}
                    />
                  </label>
                </div>
                <p className="mt-2.5 text-[11px] text-[var(--color-ink-300)]">
                  유휴 시간이 지나면 자동으로 다시 잠겨요. 0이면 유휴 잠금 안 함(앱 종료·긴급 잠금 시엔 항상 잠김).
                </p>
              </div>

              {/* 자리비움 자동 잠금 (Session 14) */}
              <ToggleRow
                title="자리 비우면 자동 잠금"
                desc="다른 앱으로 전환하거나 화면이 잠기면 세션을 즉시 잠가요."
                checked={s.lock_on_blur}
                onToggle={() => toggle("lock_on_blur")}
              />

              {/* 자율 결제 알림 (Session 15) — 비번 없이 나간 돈을 보호자가 사후 인지 */}
              <ToggleRow
                title="자율 결제 알림"
                desc="비번 없이 자동 승인된 결제를 macOS 알림으로 알려줘요."
                checked={s.notify_auto}
                onToggle={() => toggle("notify_auto")}
              />

              {/* 자율 결제 화이트리스트 (Session 16) — 처음 보는 주소는 자율 대상에서 제외 */}
              <ToggleRow
                title="자율 결제 화이트리스트"
                desc="비번으로 승인한 적 있는 주소에만 자율 결제를 허용해요. 새 주소는 첫 결제만 비번이 필요해요."
                checked={s.auto_trusted_only}
                onToggle={() => toggle("auto_trusted_only")}
              />

              {/* 화이트리스트 주소 (Session 17) — 목록은 모달로 (많아지면 설정이 길어지니) */}
              <div className="mt-4 flex items-center justify-between rounded-[var(--radius-card)] border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] px-4 py-3.5">
                <div className="min-w-0 pr-3">
                  <p className="text-[13px] tracking-tight">화이트리스트 주소</p>
                  <p className="mt-0.5 text-[11px] leading-snug text-[var(--color-ink-300)]">
                    {trusted === null
                      ? "불러오는 중…"
                      : trusted.length === 0
                        ? "아직 없어요 — 비번으로 승인하면 자동으로 학습돼요."
                        : `${trusted.length}개 학습됨`}
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => setShowTrusted(true)}
                  disabled={!trusted || trusted.length === 0}
                  className="shrink-0 text-[12px] text-[var(--color-accent)] disabled:text-[var(--color-ink-300)] hover:underline transition-colors"
                >
                  주소 보기
                </button>
              </div>
            </Section>

            {/* 네트워크 (개발 20) — 테스트넷 ↔ 메인넷 런타임 전환. 메인넷은 실제 자금. */}
            <Section
              icon={<Network size={13} className="text-[var(--color-accent)]" />}
              title="네트워크"
              delay={0.08}
            >
              <p className="text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                잔액·결제가 이뤄지는 블록체인이에요. 테스트넷은 가짜 코인으로 연습하고, 메인넷은 실제
                자금이 오가요. 체인별로 한도·사용액·내역·화이트리스트가 따로 관리돼요.
              </p>
              <div className="mt-4 grid grid-cols-2 gap-2">
                {CHAINS.map((c) => {
                  const active = c.id === s.chain_id;
                  return (
                    <button
                      key={c.id}
                      type="button"
                      onClick={() => selectChain(c.id)}
                      className={cn(
                        "rounded-[var(--radius-card)] border px-3 py-3 text-[13px] tracking-tight transition-colors",
                        active
                          ? "border-[var(--color-accent)] bg-[var(--color-accent)]/10 text-[var(--color-accent)] font-medium"
                          : "border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] text-[var(--color-ink-500)] hover:border-[var(--color-ink-300)]",
                      )}
                    >
                      {c.name}
                      <span className="block mt-0.5 text-[10px] text-[var(--color-ink-300)]">
                        {c.testnet ? "테스트넷" : "실제 자금"}
                      </span>
                    </button>
                  );
                })}
              </div>
              {chain.testnet ? (
                <p className="mt-2.5 text-[11px] text-[var(--color-ink-300)]">
                  바꾼 뒤 저장을 누르면 적용돼요.
                </p>
              ) : (
                <p className="mt-2.5 text-[11px] leading-relaxed text-amber-600 dark:text-amber-500">
                  메인넷은 실제 USDC가 오가요. 운영 예산만 소액 충전하고 한도를 꼭 확인하세요. 저장을 눌러야 적용돼요.
                </p>
              )}
            </Section>

            {/* RPC 서버 (Session 14) — 프라이버시: 공개 RPC는 내 IP↔지갑을 볼 수 있음 */}
            <Section
              icon={<Globe size={13} className="text-[var(--color-accent)]" />}
              title="RPC 서버"
              delay={0.12}
            >
              <p className="text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                잔액 조회·송금에 쓰는 서버예요. 공개 RPC는 내 IP와 지갑 주소를 볼 수 있어요. 프라이버시가
                중요하면 본인 키 엔드포인트(Alchemy/노드 등)를 직접 넣으세요.
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
                <option value={RPC_CUSTOM}>직접 입력…</option>
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
              title="앱"
              delay={0.16}
            >
              <p className="text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                창을 닫아도 앱은 종료되지 않고 백그라운드에서 결제 요청을 받아요. 완전히 끄려면
                ⌘Q를 누르세요.
              </p>
              <ToggleRow
                title="로그인 시 자동 시작"
                desc="Mac에 로그인하면 지갑이 자동으로 켜져요. 끄고 켜는 즉시 적용돼요."
                checked={autostart ?? false}
                onToggle={toggleAutostart}
              />
            </Section>

            {error && <p className="text-[12px] text-red-500 font-mono break-all">{error}</p>}

            <button type="button" onClick={save} disabled={busy} className={cn(primaryBtn, "mt-2")}>
              {saved ? (
                <Check size={15} />
              ) : busy ? (
                <Loader2 size={15} className="animate-spin" />
              ) : null}
              {saved ? "저장됨" : "저장하고 닫기"}
            </button>
          </fieldset>
        )}

        {/* 정보 — 읽기 전용이라 fieldset(저장 중 잠금) 밖에 두고, 설정 로드가 실패해도 보이게
            조건 분기 바깥에 둔다. 버전·소스 링크는 "이 앱이 뭘 하는지 직접 확인할 통로"라
            설정이 안 열리는 상황일수록 오히려 더 필요하다. */}
        <AboutSection update={update} autoCheck={current?.auto_check_update ?? true} />
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
    <Section icon={<Info size={13} className="text-[var(--color-accent)]" />} title="정보" delay={0.2}>
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
        Kura는 소스 코드가 MIT 라이선스로 공개돼 있어요. 열쇠와 돈을 다루는 코드를 직접 읽어
        확인하고, 직접 빌드해서 쓸 수 있어요.
      </p>
      {/* 저장(로컬)과 전송(RPC)을 나눠 쓴 건 개발 27 판단 그대로. 개발 29 에서 한 겹 더 —
          "내역이 ~/.jigap 에만 있다"가 "내 거래가 비공개다"로 읽히면 안 된다. 보낸 거래는
          공개 체인에 영구히 남는다. 지갑에서 이 오해는 값이 비싸다. */}
      <p className="mt-1.5 text-[11px] leading-snug text-[var(--color-ink-300)]">
        지갑 키·설정·거래 내역은 이 컴퓨터의 <span className="font-mono">~/.jigap</span> 폴더에만
        저장돼요. 잔액을 확인하고 송금할 때만 위에서 고른 RPC 서버로 요청이 나가요 — 그쪽은 내
        주소를 볼 수 있고, 보낸 거래는 공개 블록체인에 남아요.
      </p>

      <div className="mt-4 flex items-center justify-between">
        <button
          type="button"
          onClick={() => openUrl(GITHUB_URL).catch(() => {})}
          className="inline-flex items-center gap-1.5 text-[12px] text-[var(--color-accent)] hover:underline transition-colors"
        >
          <ExternalLink size={12} />
          소스 코드 보기
        </button>
        <span className="text-[11px] text-[var(--color-ink-300)]">MIT 라이선스</span>
      </div>
      {/* 글꼴 고지 — 앱에 Pretendard 를 함께 담아 배포하므로 OFL 1.1 이 고지를 요구한다.
          라이선스 전문은 번들 안(fonts/LICENSE-Pretendard.txt)에도 같이 들어간다. */}
      <p className="mt-3 text-[10px] text-[var(--color-ink-300)]">
        글꼴 Pretendard — SIL Open Font License 1.1
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
            화이트리스트 주소
          </span>
          <button
            type="button"
            onClick={onClose}
            aria-label="닫기"
            className="p-1 text-[var(--color-ink-300)] hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD] transition-colors"
          >
            <X size={14} />
          </button>
        </div>
        <p className="mt-3 text-[12px] leading-relaxed text-[var(--color-ink-500)]">
          비번으로 승인하면 자동으로 학습돼요. 철회하면 다음 결제 때 다시 비번을 받아요.
        </p>

        {addrs.length === 0 ? (
          <p className="mt-5 mb-1 text-[12px] text-[var(--color-ink-300)]">
            모두 철회했어요. 비번으로 승인하면 다시 학습돼요.
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
                    <span className="text-[11px] text-[var(--color-ink-500)]">철회할까요?</span>
                    <button
                      type="button"
                      onClick={() => {
                        setConfirming(null);
                        void onRevoke(addr);
                      }}
                      className="text-[11px] font-medium text-red-500 hover:underline"
                    >
                      철회
                    </button>
                    <button
                      type="button"
                      onClick={() => setConfirming(null)}
                      className="text-[11px] text-[var(--color-ink-300)] hover:underline"
                    >
                      취소
                    </button>
                  </span>
                ) : (
                  <button
                    type="button"
                    onClick={() => setConfirming(addr)}
                    aria-label={`${shortenAddress(addr)} 철회`}
                    title="화이트리스트에서 철회"
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

/** 설정 화면 공용 on/off 토글 행 (자리비움 잠금·자율 결제 알림 등). */
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
            업데이트 설치 중…
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
              ? `${fmtBytes(progress?.downloaded ?? 0)} 받는 중`
              : `${pct}% · ${fmtBytes(progress?.downloaded ?? 0)}`}
            {" — "}끝나면 앱이 저절로 다시 시작돼요.
          </p>
        </div>
      ) : info ? (
        <div>
          <div className="flex items-baseline justify-between gap-3">
            <p className="text-[13px] tracking-tight">
              새 버전 <span className="num font-mono">{info.version}</span>
            </p>
            <span className="num font-mono text-[11px] text-[var(--color-ink-300)]">
              지금 {info.current_version}
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
            지금 설치하고 다시 시작
          </button>
        </div>
      ) : (
        <div className="flex items-center justify-between gap-3">
          <p className="min-w-0 text-[12px] text-[var(--color-ink-500)]">
            {checking ? "확인 중…" : upToDate ? "최신 버전이에요." : "업데이트를 확인할 수 있어요."}
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
            업데이트 확인
          </button>
        </div>
      )}

      {error && <p className="mt-2 text-[11px] leading-snug text-red-500">{error}</p>}

      <div className="mt-3 flex items-center justify-between gap-3 border-t border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] pt-3">
        <div className="min-w-0 pr-1">
          <p className="text-[12px] tracking-tight">시작할 때 확인</p>
          {/* 무엇이 나가는지 적는다 — 로컬 전용을 내세운 앱이라 조용한 바깥 통신이 있으면 안 된다. */}
          <p className="mt-0.5 text-[11px] leading-snug text-[var(--color-ink-300)]">
            앱을 켤 때 새 버전이 있는지 깃허브에 물어봐요(현재 버전과 IP가 그쪽에 남아요).
          </p>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={autoCheckOn}
          onClick={onToggleAutoCheck}
          className={cn(
            "shrink-0 relative w-10 h-6 rounded-full transition-colors duration-[var(--duration-base)]",
            autoCheckOn
              ? "bg-[var(--color-accent)]"
              : "bg-[var(--color-ivory-400)] dark:bg-[var(--color-night-700)]",
          )}
        >
          <span
            className={cn(
              "absolute top-1 w-4 h-4 rounded-full bg-white shadow-sm transition-all duration-[var(--duration-base)]",
              autoCheckOn ? "left-5" : "left-1",
            )}
          />
        </button>
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
    <div className="mt-4 flex items-center justify-between rounded-[var(--radius-card)] border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] px-4 py-3.5">
      <div className="min-w-0 pr-3">
        <p className="text-[13px] tracking-tight">{title}</p>
        <p className="mt-0.5 text-[11px] leading-snug text-[var(--color-ink-300)]">{desc}</p>
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
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
    </div>
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
          오늘 {fmtAmount(usedToday, frac)} 사용
        </span>
      </div>
      <div className="mt-3 grid grid-cols-2 gap-3">
        <label className="block">
          <span className="text-[11px] text-[var(--color-ink-500)]">단일 거래 한도</span>
          <input
            value={single}
            onChange={(e) => onSingle(e.target.value)}
            inputMode="decimal"
            className={cn(inputBase, "mt-1.5 num text-[14px]")}
          />
        </label>
        <label className="block">
          <span className="text-[11px] text-[var(--color-ink-500)]">하루 누적 한도</span>
          <input
            value={daily}
            onChange={(e) => onDaily(e.target.value)}
            inputMode="decimal"
            className={cn(inputBase, "mt-1.5 num text-[14px]")}
          />
        </label>
      </div>
    </div>
  );
}
