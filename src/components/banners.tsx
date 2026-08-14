// 메인 화면 상단 배너 — 시드 백업 경고 + 긴급 잠금 안내.

import { motion } from "framer-motion";
import { AlertTriangle, ShieldAlert } from "lucide-react";
import { cn } from "@/lib/cn";
import { enter } from "@/components/ui";

export function BackupNag({ onBackup }: { onBackup: () => void }) {
  return (
    <motion.div
      {...enter}
      className={cn(
        "w-full max-w-md flex items-center gap-3 px-4 py-3",
        "rounded-[var(--radius-card)]",
        "bg-amber-50 dark:bg-amber-950/30",
        "border border-amber-200 dark:border-amber-900/50",
      )}
    >
      <AlertTriangle size={16} className="shrink-0 text-amber-600 dark:text-amber-500" />
      <p className="flex-1 min-w-0 text-[12px] leading-snug text-amber-800 dark:text-amber-300">
        시드 12단어를 아직 백업 안 했어요. 비번을 잊으면 복구 못 해요.
      </p>
      <button
        type="button"
        onClick={onBackup}
        className={cn(
          "shrink-0 h-8 px-3.5 rounded-[var(--radius-pill)] text-[12px] tracking-tight",
          "bg-amber-600 text-white hover:bg-amber-700",
          "transition-colors duration-[var(--duration-base)]",
        )}
      >
        백업하기
      </button>
    </motion.div>
  );
}

export function LockBanner({ onUnlock }: { onUnlock: () => void }) {
  return (
    <motion.div
      {...enter}
      className={cn(
        "w-full max-w-md flex items-center gap-3 px-4 py-3",
        "rounded-[var(--radius-card)]",
        "bg-red-50 dark:bg-red-950/30",
        "border border-red-200 dark:border-red-900/50",
      )}
    >
      <ShieldAlert size={16} className="shrink-0 text-red-600 dark:text-red-500" />
      <p className="flex-1 min-w-0 text-[12px] leading-snug text-red-800 dark:text-red-300">
        긴급 잠금이 켜져 있어요. 모든 송금이 차단돼요.
      </p>
      <button
        type="button"
        onClick={onUnlock}
        className={cn(
          "shrink-0 h-8 px-3.5 rounded-[var(--radius-pill)] text-[12px] tracking-tight",
          "bg-red-600 text-white hover:bg-red-700",
          "transition-colors duration-[var(--duration-base)]",
        )}
      >
        해제
      </button>
    </motion.div>
  );
}
