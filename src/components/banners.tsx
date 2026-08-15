// 메인 화면 상단 배너 — 시드 백업 경고 + 긴급 잠금 안내 + 업데이트 알림.

import { motion } from "framer-motion";
import { AlertTriangle, ShieldAlert, X, ArrowUpCircle } from "lucide-react";
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

/** 업데이트 알림 (개발 31).
 *
 *  메뉴바 상주 앱이라 사용자가 설정 화면을 안 열면 새 버전이 나온 걸 영영 모른다 —
 *  그러면 자동 업데이트를 넣은 이유(보안 수정판이 사용자에게 닿는 것)가 그대로 사라진다.
 *  그래서 지갑 화면에도 한 줄 띄운다.
 *
 *  다만 **여기서 바로 설치되지 않는다.** 버튼은 설정의 정보 카드로 보낸다 —
 *  버전과 릴리스 노트를 본 뒤 누르는 게 설치 승인이어야 한다(update.rs 의 정책과 같은 결).
 *  경고가 아니라 안내라서 색도 강조색 계열로, 접을 수 있게 둔다. */
export function UpdateBanner({
  version,
  onOpen,
  onHide,
}: {
  version: string;
  onOpen: () => void;
  onHide: () => void;
}) {
  return (
    <motion.div
      {...enter}
      className={cn(
        "w-full max-w-md flex items-center gap-3 px-4 py-3",
        "rounded-[var(--radius-card)]",
        "bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-800)]",
        "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
      )}
    >
      <ArrowUpCircle size={16} className="shrink-0 text-[var(--color-accent)]" />
      <p className="flex-1 min-w-0 text-[12px] leading-snug text-[var(--color-ink-500)]">
        새 버전 <span className="num font-mono">{version}</span> 이 나왔어요.
      </p>
      <button
        type="button"
        onClick={onOpen}
        className={cn(
          "shrink-0 h-8 px-3.5 rounded-[var(--radius-pill)] text-[12px] tracking-tight",
          "bg-[var(--color-accent)] text-white hover:bg-[var(--color-accent-hover)]",
          "transition-colors duration-[var(--duration-base)]",
        )}
      >
        살펴보기
      </button>
      <button
        type="button"
        onClick={onHide}
        aria-label="업데이트 알림 접기"
        className={cn(
          "shrink-0 h-8 w-8 flex items-center justify-center rounded-[var(--radius-pill)]",
          "text-[var(--color-ink-300)] hover:text-[var(--color-ink-500)]",
          "transition-colors duration-[var(--duration-base)]",
        )}
      >
        <X size={14} />
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
