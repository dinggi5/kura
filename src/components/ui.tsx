// 공용 UI 프리미티브 — 셸/카드/버튼 스타일 + 작은 컴포넌트 (디자인 시스템의 실체).

import { Bot, ChevronRight, X } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";

// 메뉴바 팝오버 셸(개발 26) — 창이 고정 크기·투명이라 배경과 둥근 모서리를 여기서 칠하고,
// 넘치는 내용은 바깥이 아니라 이 안에서 스크롤한다. justify-between 은 내용이 넘칠 때
// 첫 아이템을 시작점에 두므로(space-between) 위가 잘리지 않는다.
export const shell = cn(
  "h-screen w-full overflow-y-auto flex flex-col items-center justify-between",
  "rounded-[var(--radius-window)]",
  "bg-[var(--color-ivory-100)] text-[var(--color-ink-900)]",
  "dark:bg-[var(--color-night-900)] dark:text-[#E8E5DD]",
  "px-6 py-8",
  // 큰 창(개발 75): 보통 맥 창이라 모서리는 창이 둥글린다. 위는 신호등 자리(56px)를 비우고, 내용은
  // 팝오버 높이(640)만큼의 판을 창 한가운데에 둔다 — 가운데 맞춤(justify-center)이 아니라 **위아래 여백을
  // 늘려서**. 가운데 맞춤은 내용이 창보다 길 때 위가 잘려 스크롤로도 닿지 않는다(위 justify-between 주석).
  "win:rounded-none win:px-10",
  "win:pt-[max(3.5rem,calc((100vh_-_640px)/2))] win:pb-[max(2.5rem,calc((100vh_-_640px)/2))]",
);

// 화면 전체를 덮는 모달 배경 — 팝오버 모서리를 넘어 사각으로 칠해지지 않게 같이 둥글린다.
// items-start + 카드의 my-auto 조합인 이유: items-center 로 가운데 정렬하면 카드가 창보다
// 길 때 위아래가 동시에 잘리고 스크롤로도 닿지 않는다(플렉스 오버플로 함정). auto 마진은
// 여유가 있을 땐 가운데로, 넘칠 땐 0 으로 접혀 위부터 스크롤된다.
export const modalOverlay = cn(
  "fixed inset-0 z-50 overflow-y-auto flex items-start justify-center p-5",
  "rounded-[var(--radius-window)] bg-black/40 backdrop-blur-sm",
  "win:rounded-none",
);

export const cardBase = cn(
  "w-full",
  "bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-800)]",
  "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
  "rounded-[var(--radius-card)]",
  "shadow-[var(--shadow-soft)]",
);

// 모달 카드 공통 — 팝오버(420x640)보다 길어져도 닿을 수 있게 my-auto 를 반드시 함께 쓴다
// (modalOverlay 의 items-start 와 짝).
export const modalCard = cn(cardBase, "relative w-full max-w-sm my-auto");

export const enter = {
  initial: { opacity: 0, y: 8 },
  animate: { opacity: 1, y: 0 },
  exit: { opacity: 0, y: -8 },
  transition: { duration: 0.32, ease: [0.4, 0, 0.2, 1] as const },
};

export const inputBase = cn(
  "w-full h-11 px-3.5 rounded-[var(--radius-card)]",
  "bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]",
  "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
  "text-[var(--color-ink-900)] dark:text-[#E8E5DD]",
  "placeholder:text-[var(--color-ink-300)]",
  "outline-none transition-colors duration-[var(--duration-base)]",
  "focus:border-[var(--color-accent)]",
);

export const primaryBtn = cn(
  "w-full h-11 inline-flex items-center justify-center gap-2",
  "text-[14px] tracking-tight",
  "rounded-[var(--radius-card)]",
  "bg-[var(--color-accent)] text-white",
  "transition-all duration-[var(--duration-base)] ease-[cubic-bezier(0.4,0,0.2,1)]",
  "hover:not-disabled:brightness-105 hover:not-disabled:translate-y-[-1px]",
  "hover:not-disabled:shadow-[var(--shadow-soft)]",
  "active:not-disabled:translate-y-0",
  "disabled:opacity-40 disabled:cursor-not-allowed",
);

export const secondaryBtn = cn(
  "h-10 inline-flex items-center justify-center gap-2",
  "text-[13px] tracking-tight",
  "rounded-[var(--radius-card)]",
  "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
  "bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]",
  "text-[var(--color-ink-700)] dark:text-[#B5AFA2]",
  "transition-all duration-[var(--duration-base)] ease-[cubic-bezier(0.4,0,0.2,1)]",
  "hover:not-disabled:bg-[var(--color-ivory-200)]",
  "disabled:opacity-50",
);

export function FlowIcon({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex justify-center">
      <div className="w-12 h-12 rounded-full flex items-center justify-center bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-700)] text-[var(--color-accent)]">
        {children}
      </div>
    </div>
  );
}

export function FieldHint({ children }: { children: React.ReactNode }) {
  return <p className="mt-1.5 text-[11px] text-red-500/80">{children}</p>;
}

export function PwInput({
  id,
  value,
  onChange,
  placeholder,
  autoFocus,
  onEnter,
}: {
  id?: string;
  value: string;
  onChange: (v: string) => void;
  placeholder: string;
  autoFocus?: boolean;
  onEnter?: () => void;
}) {
  return (
    <input
      id={id}
      type="password"
      value={value}
      autoFocus={autoFocus}
      onChange={(e) => onChange(e.target.value)}
      onKeyDown={(e) => e.key === "Enter" && onEnter?.()}
      placeholder={placeholder}
      className={cn(inputBase, "text-[14px] tracking-wide")}
    />
  );
}

export function CloseButton({ onClose }: { onClose: () => void }) {
  return (
    <button
      type="button"
      onClick={onClose}
      className={cn(
        "absolute top-4 right-4 p-1.5 rounded-full",
        "text-[var(--color-ink-300)] hover:text-[var(--color-ink-700)]",
        "hover:bg-[var(--color-ivory-200)] dark:hover:bg-[var(--color-night-700)]",
        "transition-colors duration-[var(--duration-base)]",
      )}
      aria-label={t("닫기", "Close")}
    >
      <X size={14} />
    </button>
  );
}

/** 헤더 우측 아이콘 버튼 (잠금·내역·백업·설정 공용). tone으로 강조색을 바꾼다. */
export function HeaderIconButton({
  onClick,
  label,
  tone = "default",
  children,
}: {
  onClick: () => void;
  label: string;
  tone?: "default" | "danger" | "warn";
  children: React.ReactNode;
}) {
  const toneClass =
    tone === "danger"
      ? "text-red-600 dark:text-red-500 hover:text-red-700 hover:bg-red-50 dark:hover:bg-red-950/30"
      : tone === "warn"
        ? "text-amber-600 dark:text-amber-500 hover:text-amber-700 hover:bg-amber-50 dark:hover:bg-amber-950/30"
        : cn(
            "text-[var(--color-ink-500)] dark:text-[#B5AFA2]",
            "hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD]",
            "hover:bg-[var(--color-ivory-200)] dark:hover:bg-[var(--color-night-700)]",
          );
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      title={label}
      className={cn(
        "inline-flex items-center justify-center w-7 h-7 rounded-full",
        "transition-colors duration-[var(--duration-base)]",
        toneClass,
      )}
    >
      {children}
    </button>
  );
}

export function ActionButton({
  icon,
  label,
  disabled,
  onClick,
  active,
}: {
  icon: React.ReactNode;
  label: string;
  disabled?: boolean;
  onClick?: () => void;
  active?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "h-11 inline-flex items-center justify-center gap-2",
        "text-[14px] tracking-tight",
        "rounded-[var(--radius-card)]",
        "border",
        active
          ? "border-[var(--color-accent)] bg-[var(--color-ivory-50)] text-[var(--color-accent)]"
          : "border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)] bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-800)] text-[var(--color-ink-700)] dark:text-[#B5AFA2]",
        "transition-all duration-[var(--duration-base)] ease-[cubic-bezier(0.4,0,0.2,1)]",
        "hover:not-disabled:bg-[var(--color-ivory-200)] hover:not-disabled:translate-y-[-1px]",
        "hover:not-disabled:shadow-[var(--shadow-soft)]",
        "active:not-disabled:translate-y-0",
        "disabled:opacity-50 disabled:cursor-not-allowed",
      )}
    >
      {icon}
      {label}
    </button>
  );
}

export function FaucetButton({
  icon,
  label,
  sub,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  sub: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "h-12 inline-flex items-center justify-center gap-2",
        "rounded-[var(--radius-card)]",
        "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
        "bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]",
        "text-[var(--color-ink-700)] dark:text-[#B5AFA2]",
        "transition-all duration-[var(--duration-base)] ease-[cubic-bezier(0.4,0,0.2,1)]",
        "hover:bg-[var(--color-ivory-200)] hover:translate-y-[-1px]",
        "hover:shadow-[var(--shadow-soft)]",
        "active:translate-y-0",
      )}
    >
      <span className="text-[var(--color-ink-300)]">{icon}</span>
      <span className="text-[13px] tracking-tight">{label}</span>
      <span className="text-[10px] text-[var(--color-ink-300)]">{sub}</span>
    </button>
  );
}

/** 클라이언트 이름을 보기 좋게. 모르면 "AI". */
function prettyClient(c: string): string {
  if (!c) return "AI";
  const l = c.toLowerCase();
  if (l.includes("claude")) return "Claude";
  if (l.includes("cursor")) return "Cursor";
  if (l.includes("cline")) return "Cline";
  if (l.includes("windsurf")) return "Windsurf";
  return c;
}

/** AI 연결 배지 — 제품 컨셉(AI 전용 지갑)의 시각화. 상태는 셋이다 (개발 74).
 *
 *   쓰는 중  — MCP 가 지금 떠 있다(하트비트). Claude 가 켜져 있고 Kura 를 물고 있다.
 *   준비됨   — 등록이 살아 있다. Claude 를 켜면 Claude 가 우리를 띄운다 — 사람이 할 일이 없다.
 *   연결 필요 — 등록이 없거나 다른 곳을 가리킨다. 이때만 「할 일」처럼 보인다.
 *
 *  🔴 예전엔 하트비트만 봐서 둘째를 셋째로 그렸다 — 등록이 멀쩡한데 Claude 를 안 켠 동안 내내
 *  「AI 연결 안 됨」. MCP 엔 상주 연결이 없으니 사람들은 매일 「연결」을 다시 누르게 됐다.
 *  ready 가 null 이면(첫 조회 전) 그 자리만 비워 둔다 — 「연결 필요」가 번쩍였다 사라지지 않게.
 *  onClick 을 주면 버튼이 된다 — AI 연결 화면(개발 35) 진입점. */
export function AgentBadge({
  connected,
  ready,
  client,
  onClick,
}: {
  connected: boolean;
  ready: boolean | null;
  client: string;
  onClick?: () => void;
}) {
  const name = prettyClient(client);
  if (!connected && ready === null) return <span className="h-[26px]" aria-hidden />;
  const state = connected ? "live" : ready ? "ready" : "needs";
  const title =
    state === "live"
      ? t(
          `${name}가 지금 이 지갑을 쓰고 있어요. 결제를 요청하면 승인 창이 떠요.`,
          `${name} is using this wallet right now. When it asks to pay, an approval window opens.`,
        )
      : state === "ready"
        ? t(
            "연결은 끝나 있어요. Claude를 켜면 알아서 붙고, Kura가 꺼져 있어도 결제를 요청할 때 깨워요.",
            "You're connected. Open Claude and it attaches on its own — it even wakes Kura when it needs to pay.",
          )
        : t(
            "아직 이 앱과 연결된 AI가 없어요. 누르면 한 번에 연결할 수 있어요.",
            "No AI is connected to this app yet. Tap to connect in one step.",
          );
  const className = cn(
    "inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full",
    "text-[11px] tracking-tight border select-none",
    "bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-800)]",
    "transition-colors duration-[var(--duration-base)]",
    state === "live"
      ? "border-[var(--color-accent)] text-[var(--color-accent)]"
      : state === "ready"
        ? "border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)] text-[var(--color-ink-500)]"
        : "border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)] text-[var(--color-ink-300)]",
    onClick && "cursor-pointer hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]",
  );
  const inner = (
    <>
      <Bot size={12} />
      {state === "live" ? (
        <>
          <span>{t(`${name} 쓰는 중`, `${name} in use`)}</span>
          <span className="relative flex w-1.5 h-1.5" aria-hidden>
            <span className="absolute inline-flex w-full h-full rounded-full bg-[var(--color-accent)] opacity-60 animate-ping" />
            <span className="relative inline-flex w-1.5 h-1.5 rounded-full bg-[var(--color-accent)]" />
          </span>
        </>
      ) : state === "ready" ? (
        <>
          <span className="inline-flex w-1.5 h-1.5 rounded-full bg-[var(--color-accent)] opacity-70" aria-hidden />
          <span>{t("준비됨 · Claude를 켜면 자동으로 붙어요", "Ready · attaches when you open Claude")}</span>
        </>
      ) : (
        <>
          <span>{t("AI 연결 필요", "Connect an AI")}</span>
          <ChevronRight size={12} aria-hidden />
        </>
      )}
    </>
  );
  return onClick ? (
    <button type="button" onClick={onClick} title={title} className={className}>
      {inner}
    </button>
  ) : (
    <span title={title} className={className}>
      {inner}
    </span>
  );
}

/** 공용 on/off 스위치 — 설정의 ToggleRow·업데이트 블록, 환영 투어의 연결 단계(개발 74)가 같은 스위치를 쓴다(개발 39 정리 전엔
 *  같은 마크업이 두 벌 복사돼 있었다). 접근성 라벨은 행 제목을 그대로 받는다. */
export function Switch({
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

