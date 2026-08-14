// 받기 카드 — 주소 QR + 복사 + (테스트넷) 테스트 코인 Faucet / (메인넷) 거래소 입금 안내.

import { openUrl } from "@tauri-apps/plugin-opener";
import { motion } from "framer-motion";
import { Check, Copy, Droplets, Fuel } from "lucide-react";
import { QRCodeSVG } from "qrcode.react";
import { cn } from "@/lib/cn";
import { useChain } from "@/lib/chain";
import { useCopy } from "@/lib/useCopy";
import { cardBase, enter, secondaryBtn, CloseButton, FaucetButton } from "@/components/ui";

export function ReceiveCard({ address, onClose }: { address: string; onClose: () => void }) {
  const chain = useChain();
  const [copied, copy] = useCopy();

  async function openFaucet(url: string) {
    navigator.clipboard?.writeText(address).catch(() => {});
    try {
      await openUrl(url);
    } catch {
      /* opener 실패는 조용히 무시 */
    }
  }

  return (
    <motion.section {...enter} className={cn(cardBase, "relative px-8 py-8")}>
      <CloseButton onClose={onClose} />

      <p className="text-[11px] tracking-[0.04em] text-[var(--color-ink-500)]">받는 주소</p>

      <div className="mt-6 flex justify-center">
        <div className="p-4 rounded-[10px] bg-white dark:bg-[#E8E5DD] border border-[var(--color-ivory-300)] dark:border-transparent">
          <QRCodeSVG value={address} size={176} bgColor="transparent" fgColor="#1A1814" level="M" />
        </div>
      </div>

      <p className="mt-6 text-[11px] font-mono text-[var(--color-ink-700)] dark:text-[#B5AFA2] break-all leading-relaxed text-center select-all">
        {address}
      </p>

      <button type="button" onClick={() => copy(address)} className={cn(secondaryBtn, "mt-4 w-full")}>
        {copied ? (
          <>
            <Check size={13} className="text-[var(--color-accent)]" />
            복사됨
          </>
        ) : (
          <>
            <Copy size={13} />
            주소 복사
          </>
        )}
      </button>

      {chain.testnet ? (
        <div className="mt-6 pt-5 border-t border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)]">
          <p className="text-[11px] tracking-[0.04em] text-[var(--color-ink-500)]">테스트 코인 받기</p>
          <p className="mt-1.5 text-[11px] leading-relaxed text-[var(--color-ink-300)]">
            주소가 복사돼요. 열리는 페이지에 붙여넣고 {chain.name}를 고르세요.
          </p>
          <div className="mt-3 grid grid-cols-2 gap-2">
            <FaucetButton icon={<Droplets size={13} />} label="USDC" sub="Circle" onClick={() => openFaucet("https://faucet.circle.com")} />
            <FaucetButton icon={<Fuel size={13} />} label="ETH" sub="Base" onClick={() => openFaucet("https://portal.cdp.coinbase.com/products/faucet")} />
          </div>
        </div>
      ) : (
        <div className="mt-6 pt-5 border-t border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)]">
          <p className="text-[11px] tracking-[0.04em] text-[var(--color-ink-500)]">USDC 충전</p>
          <p className="mt-1.5 text-[11px] leading-relaxed text-[var(--color-ink-300)]">
            거래소·다른 지갑에서 이 주소로 보낼 때 반드시 <b>Base</b> 네트워크를 고르세요. 다른
            네트워크(이더리움 등)로 보내면 자금을 잃을 수 있어요. 에이전트 운영 예산만 소액 충전하세요.
          </p>
        </div>
      )}
    </motion.section>
  );
}
