// 거래 내역 화면 — 모든 송금/서명 시도(성공·차단·실패·정산)와 들어온 돈(개발 69)을 최신순으로.
// 큰 창을 넓게 펴면(768px 이상, 개발 75) 같은 기록을 표로 — 날짜는 「3시간 전」 대신 실제 시각, 주소는 줄이지 않는다.
// 넓은 화면에서 내역을 여는 이유가 대개 「그 건이 언제, 어디로」를 맞춰 보는 것이라서.

import { openUrl } from "@tauri-apps/plugin-opener";
import { motion } from "framer-motion";
import {
  AlertTriangle,
  ArrowDownLeft,
  ArrowUpRight,
  Ban,
  Check,
  ExternalLink,
  FileSignature,
  History,
  Loader2,
  Clock,
  Undo2,
} from "lucide-react";
import { cn } from "@/lib/cn";
import { useChain } from "@/lib/chain";
import { fmtAmount, fmtRelTime, shortenAddress } from "@/lib/format";
import type { HistoryEntry } from "@/lib/types";
import { cardBase, enter, shell } from "@/components/ui";
import { locale, t } from "@/lib/i18n";
import { useWide } from "@/lib/win";

export function HistoryScreen({
  entries,
  onMore,
  onClose,
}: {
  entries: HistoryEntry[] | null;
  /** 더 오래된 기록이 있을 수 있을 때만(읽어 온 줄이 요청한 만큼 꽉 찼다). */
  onMore?: () => void;
  onClose: () => void;
}) {
  const wide = useWide();
  return (
    // gap — 기록이 창보다 길면 justify-between 의 사이가 0 이 되어 머리줄이 카드에 붙는다.
    <main className={cn(shell, "gap-4")}>
      <header className="w-full max-w-md md:max-w-3xl flex items-center justify-between text-[12px] text-[var(--color-ink-500)]">
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

      <motion.section {...enter} className={cn(cardBase, "max-w-md md:max-w-3xl px-5 py-4 md:px-6")}>
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
                "받은 돈과 보낸 송금, 차단된 시도가 여기에 쌓여요.",
                "Money you receive, payments you send, and blocked attempts show up here.",
              )}
            </p>
          </div>
        ) : (
          <>
            {wide ? (
              <HistoryTable entries={entries} />
            ) : (
              <ul className="divide-y divide-[var(--color-ivory-300)] dark:divide-[var(--color-night-700)]">
                {entries.map((e, i) => (
                  <HistoryRow key={`${e.ts}-${i}`} entry={e} />
                ))}
              </ul>
            )}
            {onMore && (
              <button
                type="button"
                onClick={onMore}
                className="mt-2 w-full py-3 text-[12px] text-[var(--color-ink-500)] hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD] transition-colors"
              >
                {t("더 보기", "Show more")}
              </button>
            )}
          </>
        )}
      </motion.section>

      <div />
    </main>
  );
}

const HISTORY_META: Record<string, { icon: React.ReactNode; ring: string; label: string; labelColor: string }> = {
  // 들어온 돈(개발 69) — 보낸 것과 방향만 다르다. 색은 같은 강조색, 화살표로 가른다.
  received: { icon: <ArrowDownLeft size={15} />, ring: "bg-[var(--color-accent)]/10 text-[var(--color-accent)]", label: t("받음", "Received"), labelColor: "" },
  sent: { icon: <ArrowUpRight size={15} />, ring: "bg-[var(--color-accent)]/10 text-[var(--color-accent)]", label: t("보냄", "Sent"), labelColor: "" },
  settled: { icon: <Check size={15} />, ring: "bg-[var(--color-accent)]/10 text-[var(--color-accent)]", label: t("정산됨", "Settled"), labelColor: "" },
  signed: { icon: <FileSignature size={15} />, ring: "bg-[var(--color-ink-500)]/10 text-[var(--color-ink-500)] dark:text-[#B5AFA2]", label: t("정산 대기", "Awaiting settlement"), labelColor: "text-[var(--color-ink-300)]" },
  blocked: { icon: <Ban size={15} />, ring: "bg-amber-500/10 text-amber-600 dark:text-amber-500", label: t("차단됨", "Blocked"), labelColor: "text-amber-600 dark:text-amber-500" },
  failed: { icon: <AlertTriangle size={15} />, ring: "bg-red-500/10 text-red-600 dark:text-red-500", label: t("실패", "Failed"), labelColor: "text-red-500/80" },
  settle_failed: { icon: <AlertTriangle size={15} />, ring: "bg-red-500/10 text-red-600 dark:text-red-500", label: t("정산 실패", "Settlement failed"), labelColor: "text-red-500/80" },
  // 서명한 tx 를 냈는데 체인이 받았는지 모름(개발 66) — 나갔을 수 있다. 실패(빨강)가 아니라 확인이 필요한 상태.
  unknown: { icon: <AlertTriangle size={15} />, ring: "bg-amber-500/10 text-amber-600 dark:text-amber-500", label: t("확인 필요", "Unconfirmed"), labelColor: "text-amber-600 dark:text-amber-500" },
  // 지갑이 체인에서 결말을 확인한 둘(개발 71) — 돈은 안 나갔다(되돌려짐은 가스만). 오늘 한도도 돌려받았다.
  reverted: { icon: <Undo2 size={15} />, ring: "bg-[var(--color-ink-500)]/10 text-[var(--color-ink-500)] dark:text-[#B5AFA2]", label: t("되돌려짐", "Reverted"), labelColor: "text-[var(--color-ink-300)]" },
  expired: { icon: <Clock size={15} />, ring: "bg-[var(--color-ink-500)]/10 text-[var(--color-ink-500)] dark:text-[#B5AFA2]", label: t("만료 · 안 나감", "Expired · not paid"), labelColor: "text-[var(--color-ink-300)]" },
};

/** 금액 — 보통은 USDC 2자리·ETH 5자리로 줄이되, 0 이 아닌 금액이 「0」으로 보이면 원래 값 그대로
 *  (코덱스 개발 69 1차: 0.000001 USDC 입금이 「0 USDC」로 보였다). */
function amountText(entry: HistoryEntry): string {
  const short = fmtAmount(entry.amount, entry.token === "ETH" ? 5 : 2);
  return short === "0" && Number(entry.amount) > 0 ? entry.amount : short;
}

/** 한 줄이 무엇을 보여 줄지 — 목록과 표가 같은 규칙을 쓴다(둘이 갈리면 표에서만 링크가 사라지는 식의 어긋남이 생긴다). */
function rowFacts(entry: HistoryEntry) {
  const meta = HISTORY_META[entry.status] ?? HISTORY_META.failed;

  // BaseScan 링크 대상 tx: 송금="sent"의 detail, x402 정산="settled"의 settle_tx.
  // 확인 필요("unknown")도 detail 이 tx 해시다 — 익스플로러에서 들어갔는지 볼 수 있어야 한다.
  // 받음("received")도 detail 이 tx 해시다 — 컨트랙트가 보낸 ETH 는 해시를 몰라 빈 값(링크 없음).
  const received = entry.status === "received";
  const linkTx =
    entry.status === "sent" || entry.status === "unknown" || entry.status === "reverted" || received
      ? entry.detail
      : entry.status === "settled"
        ? entry.settle_tx ?? ""
        : "";
  const hasLink = linkTx.length > 0;
  // 사유는 사람이 읽을 차단/실패에만 표시(signed/settled의 detail은 nonce라 숨김).
  const showReason = (entry.status === "blocked" || entry.status === "failed") && !!entry.detail;
  return { meta, received, linkTx, hasLink, showReason };
}

/** 표의 날짜 — 올해면 월·일·시각, 아니면 연도까지. */
function fmtWhen(ts: number): string {
  const d = new Date(ts * 1000);
  const thisYear = d.getFullYear() === new Date().getFullYear();
  return d.toLocaleString(locale(), {
    ...(thisYear ? {} : { year: "numeric" }),
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** 상태 글자 — 탐색기 링크가 있으면 글자 자체가 링크다(개발 73). 목록·표 공용. */
function StatusLabel({ entry }: { entry: HistoryEntry }) {
  const chain = useChain();
  const { meta, linkTx, hasLink } = rowFacts(entry);
  return hasLink ? (
    <button
      type="button"
      onClick={() => openUrl(chain.explorerTx + linkTx).catch(() => {})}
      aria-label={`${meta.label} — ${t("탐색기에서 보기", "View in explorer")}`}
      className={cn(
        "inline-flex items-center gap-1 text-[11px] hover:text-[var(--color-accent)] transition-colors",
        meta.labelColor || "text-[var(--color-ink-500)]",
      )}
    >
      {meta.label} <ExternalLink size={10} />
    </button>
  ) : (
    <span className={cn("text-[11px]", meta.labelColor || "text-[var(--color-ink-300)]")}>{meta.label}</span>
  );
}

function HistoryTable({ entries }: { entries: HistoryEntry[] }) {
  const th = "pb-2 text-left font-normal text-[11px] text-[var(--color-ink-300)]";
  return (
    <table className="w-full table-fixed">
      <colgroup>
        <col className="w-[9rem]" />
        <col />
        <col className="w-[8rem]" />
        <col className="w-[8rem]" />
      </colgroup>
      <thead>
        <tr>
          <th className={th}>{t("시각", "When")}</th>
          <th className={th}>{t("상대", "Counterparty")}</th>
          <th className={cn(th, "text-right pr-6")}>{t("금액", "Amount")}</th>
          <th className={th}>{t("상태", "Status")}</th>
        </tr>
      </thead>
      <tbody className="divide-y divide-[var(--color-ivory-300)] dark:divide-[var(--color-night-700)]">
        {entries.map((e, i) => (
          <HistoryTableRow key={`${e.ts}-${i}`} entry={e} />
        ))}
      </tbody>
    </table>
  );
}

function HistoryTableRow({ entry }: { entry: HistoryEntry }) {
  const { received, showReason } = rowFacts(entry);
  return (
    <tr className="align-top">
      <td className="py-3 text-[12px] text-[var(--color-ink-500)] num whitespace-nowrap">{fmtWhen(entry.ts)}</td>
      <td className="py-3 pr-4 min-w-0">
        <p className="flex items-center gap-1.5 text-[11px] text-[var(--color-ink-700)] dark:text-[#B5AFA2] font-mono truncate">
          <span className="shrink-0 text-[var(--color-ink-300)]">
            {received ? <ArrowDownLeft size={12} /> : <ArrowUpRight size={12} />}
          </span>
          {/* 넓으니 주소를 줄이지 않는다 — 표에서 맞춰 보는 건 대개 이 값이다. */}
          <span className="truncate select-text">{entry.to || t("컨트랙트에서", "From a contract")}</span>
        </p>
        {showReason && <p className="mt-0.5 pl-[18px] text-[11px] text-[var(--color-ink-300)] truncate">{entry.detail}</p>}
      </td>
      <td className="py-3 pr-6 text-right whitespace-nowrap">
        <span className="num text-[14px] tracking-tight text-[var(--color-ink-900)] dark:text-[#E8E5DD]">
          {received ? "+" : ""}
          {amountText(entry)}
        </span>
        <span className="ml-1 text-[11px] text-[var(--color-ink-500)]">{entry.token}</span>
      </td>
      <td className="py-3">
        <StatusLabel entry={entry} />
      </td>
    </tr>
  );
}

function HistoryRow({ entry }: { entry: HistoryEntry }) {
  const { meta, received, showReason } = rowFacts(entry);

  return (
    <li className="flex items-center gap-3 py-3">
      <div className={cn("shrink-0 w-9 h-9 rounded-full flex items-center justify-center", meta.ring)}>
        {meta.icon}
      </div>

      <div className="flex-1 min-w-0">
        <div className="flex items-baseline gap-1.5">
          <span className="num text-[14px] tracking-tight text-[var(--color-ink-900)] dark:text-[#E8E5DD]">
            {amountText(entry)}
          </span>
          <span className="text-[11px] text-[var(--color-ink-500)]">{entry.token}</span>
        </div>
        <p className="mt-0.5 flex items-center gap-1 text-[11px] text-[var(--color-ink-300)] font-mono truncate">
          {received ? (
            <ArrowDownLeft size={10} className="shrink-0" />
          ) : (
            <ArrowUpRight size={10} className="shrink-0" />
          )}
          {/* 받음의 상대는 보낸 주소 — 컨트랙트 내부 전송이라 모르면 비운다. */}
          {entry.to ? shortenAddress(entry.to) : t("컨트랙트에서", "From a contract")}
        </p>
        {showReason && (
          <p className="mt-0.5 text-[11px] text-[var(--color-ink-300)] truncate">{entry.detail}</p>
        )}
      </div>

      <div className="shrink-0 flex flex-col items-end gap-1">
        <span className="text-[11px] text-[var(--color-ink-300)] num">{fmtRelTime(entry.ts)}</span>
        <StatusLabel entry={entry} />
      </div>
    </li>
  );
}
