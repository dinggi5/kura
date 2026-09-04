// 잔액 카드 — 메인 화면의 첫 카드. 큰 USDC 잔액 + 가스 한 줄 + 주소 복사.

import { motion } from "framer-motion";
import { Check, Copy, Fuel, RefreshCw } from "lucide-react";
import { cn } from "@/lib/cn";
import { useChain } from "@/lib/chain";
import { fmtAmount, shortenAddress } from "@/lib/format";
import type { Balances } from "@/lib/types";
import { cardBase, enter } from "@/components/ui";
import { t } from "@/lib/i18n";

export function BalanceCard({
  address,
  copied,
  onCopy,
  balances,
  balanceError,
  refreshing,
  onRefresh,
}: {
  address: string;
  copied: boolean;
  onCopy: () => void;
  balances: Balances | null;
  balanceError: string | null;
  refreshing: boolean;
  onRefresh: () => void;
}) {
  const chain = useChain();
  return (
    <motion.section {...enter} className={cn(cardBase, "px-8 py-10")}>
      <div className="flex items-center justify-between">
        {/* 개발 54 는 여기(「총 잔액」 라벨 자리)에 계정 칩을 뒀지만, 11px 회색이라 «라벨»로
            읽혀 아무도 누를 생각을 안 했다(개발 59 실사용). 계정은 화면 맨 위 헤더로 옮겼고
            여기는 비운다 — 라벨을 다시 채우는 건 개발 54 가 「숫자가 잔액인 건 자명하다」고
            뺀 것을 되돌리는 일이라 하지 않는다. */}
        <span aria-hidden />
        <button
          type="button"
          onClick={onRefresh}
          disabled={refreshing}
          aria-label={t("잔액 새로고침", "Refresh balance")}
          className={cn(
            "inline-flex items-center justify-center w-7 h-7 rounded-full",
            "text-[var(--color-ink-500)] dark:text-[#B5AFA2]",
            "bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-700)]",
            "border border-[var(--color-ivory-300)] dark:border-[var(--color-night-600)]",
            "hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD]",
            "hover:bg-[var(--color-ivory-300)] dark:hover:bg-[var(--color-night-600)]",
            "transition-colors duration-[var(--duration-base)]",
            "disabled:opacity-40 disabled:cursor-not-allowed",
          )}
        >
          <RefreshCw size={14} className={refreshing ? "animate-spin" : ""} />
        </button>
      </div>

      <div className="mt-6 flex items-baseline gap-2">
        <span className="balance text-[56px] leading-none num">
          {balanceError ? "—" : fmtAmount(balances?.usdc, 2, 2)}
        </span>
        <span className="text-[15px] text-[var(--color-ink-500)] tracking-tight">USDC</span>
      </div>

      <p className="mt-2 flex items-center gap-1.5 text-[13px] text-[var(--color-ink-500)] num">
        <Fuel size={12} className="text-[var(--color-ink-300)]" />
        {balanceError ? (
          <span className="text-red-500/80 text-[12px]">{t("잔액 조회 실패", "Couldn't load balance")}</span>
        ) : chain.nativeIsUsdc ? (
          // 가스가 곧 위 USDC 인 체인(Arc) — 숫자를 하나 더 놓으면 같은 돈을 두 번 센다.
          // 줄을 지우는 대신 "그 돈에서 나간다"를 말해 준다(빈자리보다 사실이 낫다).
          <>{t("가스도 이 USDC에서 나가요", "Gas comes out of this USDC")}</>
        ) : (
          <>{t("가스용 ETH ", "ETH for gas ")}{fmtAmount(balances?.eth, 5)}</>
        )}
      </p>

      <div className="mt-8">
        <button
          type="button"
          onClick={onCopy}
          className={cn(
            "group inline-flex items-center gap-2",
            "text-[13px] text-[var(--color-ink-700)] dark:text-[#B5AFA2]",
            "font-mono tracking-tight",
            "transition-colors duration-[var(--duration-base)] ease-[cubic-bezier(0.4,0,0.2,1)]",
            "hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD]",
          )}
          title={address}
        >
          <span>{shortenAddress(address)}</span>
          {copied ? (
            <Check size={13} className="text-[var(--color-accent)]" />
          ) : (
            <Copy size={13} className="text-[var(--color-ink-300)] group-hover:text-[var(--color-ink-700)] transition-colors" />
          )}
        </button>
      </div>
    </motion.section>
  );
}
