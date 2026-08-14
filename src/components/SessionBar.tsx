// 자율 결제 세션 (Session 14) — 메인 화면 상태 바 + 잠금 해제 모달.

import { useState } from "react";
import { motion } from "framer-motion";
import { Clock, Loader2, Unlock, Zap } from "lucide-react";
import { cn } from "@/lib/cn";
import { fmtAmount, fmtCountdown } from "@/lib/format";
import type { SessionStatus } from "@/lib/types";
import { enter, modalCard, modalOverlay, primaryBtn, secondaryBtn, PwInput } from "@/components/ui";

/** 메인 화면의 자율 결제 상태 표시 + 잠금 해제/잠그기. 자율 한도가 설정돼 있을 때만 보인다. */
export function SessionBar({
  session,
  onUnlock,
  onLock,
}: {
  session: SessionStatus;
  onUnlock: () => void;
  onLock: () => void;
}) {
  const limit = fmtAmount(session.auto_limit, 2);

  if (session.unlocked) {
    return (
      <motion.div
        {...enter}
        className={cn(
          "w-full max-w-md flex items-center gap-3 px-4 py-3",
          "rounded-[var(--radius-card)] border border-[var(--color-accent)]",
          "bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-800)]",
        )}
      >
        <Zap size={16} className="shrink-0 text-[var(--color-accent)]" />
        <div className="flex-1 min-w-0">
          <p className="text-[12px] leading-snug tracking-tight text-[var(--color-ink-900)] dark:text-[#E8E5DD]">
            자율 결제 켜짐 · 한도 <span className="num">{limit}</span> USDC
          </p>
          {session.remaining_secs > 0 && (
            <p className="mt-0.5 flex items-center gap-1 text-[11px] text-[var(--color-ink-300)]">
              <Clock size={10} />
              <span className="num">{fmtCountdown(session.remaining_secs)}</span> 후 자동 잠금
            </p>
          )}
        </div>
        <button
          type="button"
          onClick={onLock}
          className={cn(
            "shrink-0 h-8 px-3.5 rounded-[var(--radius-pill)] text-[12px] tracking-tight",
            "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
            "text-[var(--color-ink-500)] dark:text-[#B5AFA2]",
            "hover:bg-[var(--color-ivory-200)] dark:hover:bg-[var(--color-night-700)]",
            "transition-colors duration-[var(--duration-base)]",
          )}
        >
          잠그기
        </button>
      </motion.div>
    );
  }

  return (
    <motion.div
      {...enter}
      className={cn(
        "w-full max-w-md flex items-center gap-3 px-4 py-3",
        "rounded-[var(--radius-card)] border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)]",
        "bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-800)]",
      )}
    >
      <Zap size={16} className="shrink-0 text-[var(--color-ink-300)]" />
      <p className="flex-1 min-w-0 text-[12px] leading-snug text-[var(--color-ink-500)] dark:text-[#B5AFA2]">
        자율 결제 잠김 — 해제하면 <span className="num">{limit}</span> USDC까지 AI가 비번 없이 결제해요.
      </p>
      <button
        type="button"
        onClick={onUnlock}
        className={cn(
          "shrink-0 h-8 px-3.5 rounded-[var(--radius-pill)] text-[12px] tracking-tight inline-flex items-center gap-1.5",
          "bg-[var(--color-accent)] text-white hover:opacity-90",
          "transition-opacity duration-[var(--duration-base)]",
        )}
      >
        <Unlock size={12} />
        잠금 해제
      </button>
    </motion.div>
  );
}

export function UnlockSessionModal({
  autoLimit,
  onUnlock,
  onClose,
}: {
  autoLimit: string;
  onUnlock: (password: string) => Promise<void>;
  onClose: () => void;
}) {
  const [pw, setPw] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    if (!pw || busy) return;
    setBusy(true);
    setError(null);
    try {
      await onUnlock(pw);
      onClose();
    } catch (e) {
      setError(String(e));
      setPw("");
      setBusy(false);
    }
  }

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.2 }}
      className={modalOverlay}
    >
      <motion.section
        initial={{ scale: 0.9, opacity: 0, y: 12 }}
        animate={{ scale: 1, opacity: 1, y: 0 }}
        exit={{ scale: 0.95, opacity: 0 }}
        transition={{ type: "spring", stiffness: 340, damping: 24 }}
        className={cn(modalCard, "px-8 py-7")}
      >
        <span className="flex items-center gap-1.5 text-[11px] tracking-[0.04em] text-[var(--color-accent)]">
          <Zap size={13} />
          자율 결제 잠금 해제
        </span>
        <p className="mt-4 text-[13px] leading-relaxed text-[var(--color-ink-700)] dark:text-[#B5AFA2]">
          비밀번호를 한 번 입력하면, <span className="num">{fmtAmount(autoLimit, 2)}</span> USDC 이하의 AI 결제는
          비번 없이 자동 승인돼요.
        </p>
        <p className="mt-2 flex items-center gap-1.5 text-[11px] text-[var(--color-ink-300)]">
          <Clock size={11} />
          앱 종료·긴급 잠금·유휴 시간이 지나면 자동으로 다시 잠겨요.
        </p>

        <div className="mt-5">
          <PwInput
            value={pw}
            onChange={setPw}
            placeholder="비밀번호로 잠금 해제"
            autoFocus
            onEnter={submit}
          />
        </div>

        {error && <p className="mt-3 text-[12px] text-red-500 font-mono break-all">{error}</p>}

        <div className="mt-5 grid grid-cols-[1fr_1.8fr] gap-2">
          <button type="button" onClick={onClose} disabled={busy} className={cn(secondaryBtn, "w-full")}>
            취소
          </button>
          <button type="button" onClick={submit} disabled={!pw || busy} className={primaryBtn}>
            {busy ? <Loader2 size={15} className="animate-spin" /> : <Unlock size={15} />}
            잠금 해제
          </button>
        </div>
      </motion.section>
    </motion.div>
  );
}
