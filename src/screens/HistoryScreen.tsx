// 거래 내역 화면 — 모든 송금/서명 시도(성공·차단·실패·정산)를 최신순으로.

import { openUrl } from "@tauri-apps/plugin-opener";
import { motion } from "framer-motion";
import {
  AlertTriangle,
  ArrowUpRight,
  Ban,
  Check,
  ExternalLink,
  FileSignature,
  History,
  Loader2,
} from "lucide-react";
import { cn } from "@/lib/cn";
import { useChain } from "@/lib/chain";
import { fmtAmount, fmtRelTime, shortenAddress } from "@/lib/format";
import type { HistoryEntry } from "@/lib/types";
import { cardBase, enter, shell } from "@/components/ui";
import { t } from "@/lib/i18n";

export function HistoryScreen({
  entries,
  onClose,
}: {
  entries: HistoryEntry[] | null;
  onClose: () => void;
}) {
  return (
    <main className={shell}>
      <header className="w-full max-w-md flex items-center justify-between text-[12px] text-[var(--color-ink-500)]">
        <span className="flex items-center gap-2">
          <History size={12} className="text-[var(--color-accent)]" />
          {t("거래 내역", "History")}
        </span>
        <button
          type="button"
          onClick={onClose}
          className="hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD] transition-colors"
        >
          {t("닫기", "Close")}
        </button>
      </header>

      <motion.section {...enter} className={cn(cardBase, "max-w-md px-5 py-4")}>
        {entries == null ? (
          <div className="flex flex-col items-center py-12">
            <Loader2 size={22} className="animate-spin text-[var(--color-accent)]" />
            <p className="mt-3 text-[13px] text-[var(--color-ink-500)]">{t("불러오는 중…", "Loading…")}</p>
          </div>
        ) : entries.length === 0 ? (
          <div className="flex flex-col items-center py-12 text-center">
            <div className="w-12 h-12 rounded-full flex items-center justify-center bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-700)] text-[var(--color-ink-300)]">
              <History size={20} />
            </div>
            <p className="mt-4 text-[13px] text-[var(--color-ink-500)]">
              {t("아직 거래 내역이 없어요.", "No transactions yet.")}
            </p>
            <p className="mt-1 text-[11px] text-[var(--color-ink-300)]">
              {t(
                "보낸 송금과 차단된 시도가 여기에 쌓여요.",
                "Payments you send and attempts that were blocked show up here.",
              )}
            </p>
          </div>
        ) : (
          <ul className="divide-y divide-[var(--color-ivory-300)] dark:divide-[var(--color-night-700)]">
            {entries.map((e, i) => (
              <HistoryRow key={`${e.ts}-${i}`} entry={e} />
            ))}
          </ul>
        )}
      </motion.section>

      <div />
    </main>
  );
}

const HISTORY_META: Record<string, { icon: React.ReactNode; ring: string; label: string; labelColor: string }> = {
  sent: { icon: <ArrowUpRight size={15} />, ring: "bg-[var(--color-accent)]/10 text-[var(--color-accent)]", label: t("보냄", "Sent"), labelColor: "" },
  settled: { icon: <Check size={15} />, ring: "bg-[var(--color-accent)]/10 text-[var(--color-accent)]", label: t("정산됨", "Settled"), labelColor: "" },
  signed: { icon: <FileSignature size={15} />, ring: "bg-[var(--color-ink-500)]/10 text-[var(--color-ink-500)] dark:text-[#B5AFA2]", label: t("정산 대기", "Awaiting settlement"), labelColor: "text-[var(--color-ink-300)]" },
  blocked: { icon: <Ban size={15} />, ring: "bg-amber-500/10 text-amber-600 dark:text-amber-500", label: t("차단됨", "Blocked"), labelColor: "text-amber-600 dark:text-amber-500" },
  failed: { icon: <AlertTriangle size={15} />, ring: "bg-red-500/10 text-red-600 dark:text-red-500", label: t("실패", "Failed"), labelColor: "text-red-500/80" },
  settle_failed: { icon: <AlertTriangle size={15} />, ring: "bg-red-500/10 text-red-600 dark:text-red-500", label: t("정산 실패", "Settlement failed"), labelColor: "text-red-500/80" },
};

function HistoryRow({ entry }: { entry: HistoryEntry }) {
  const chain = useChain();
  const meta = HISTORY_META[entry.status] ?? HISTORY_META.failed;

  // BaseScan 링크 대상 tx: 송금="sent"의 detail, x402 정산="settled"의 settle_tx.
  const linkTx = entry.status === "sent" ? entry.detail : entry.status === "settled" ? entry.settle_tx ?? "" : "";
  const hasLink = linkTx.length > 0;
  // 사유는 사람이 읽을 차단/실패에만 표시(signed/settled의 detail은 nonce라 숨김).
  const showReason = (entry.status === "blocked" || entry.status === "failed") && !!entry.detail;

  return (
    <li className="flex items-center gap-3 py-3">
      <div className={cn("shrink-0 w-9 h-9 rounded-full flex items-center justify-center", meta.ring)}>
        {meta.icon}
      </div>

      <div className="flex-1 min-w-0">
        <div className="flex items-baseline gap-1.5">
          <span className="num text-[14px] tracking-tight text-[var(--color-ink-900)] dark:text-[#E8E5DD]">
            {fmtAmount(entry.amount, entry.token === "ETH" ? 5 : 2)}
          </span>
          <span className="text-[11px] text-[var(--color-ink-500)]">{entry.token}</span>
        </div>
        <p className="mt-0.5 flex items-center gap-1 text-[11px] text-[var(--color-ink-300)] font-mono truncate">
          <ArrowUpRight size={10} className="shrink-0" />
          {shortenAddress(entry.to)}
        </p>
        {showReason && (
          <p className="mt-0.5 text-[11px] text-[var(--color-ink-300)] truncate">{entry.detail}</p>
        )}
      </div>

      <div className="shrink-0 flex flex-col items-end gap-1">
        <span className="text-[11px] text-[var(--color-ink-300)] num">{fmtRelTime(entry.ts)}</span>
        {hasLink ? (
          <button
            type="button"
            onClick={() => openUrl(chain.explorerTx + linkTx).catch(() => {})}
            className="inline-flex items-center gap-1 text-[11px] text-[var(--color-ink-500)] hover:text-[var(--color-accent)] transition-colors"
          >
            <ExternalLink size={11} /> {t("보기", "View")}
          </button>
        ) : (
          <span className={cn("text-[11px]", meta.labelColor || "text-[var(--color-ink-300)]")}>{meta.label}</span>
        )}
      </div>
    </li>
  );
}
