// 보내기 카드 — 입력 → 비번 승인(도장 찍듯 확정) → 전송 → 완료.

import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { motion } from "framer-motion";
import { ArrowLeft, ArrowUpRight, Check, ExternalLink, Loader2, Lock } from "lucide-react";
import { cn } from "@/lib/cn";
import { useChain } from "@/lib/chain";
import { MAX_ETH, MAX_USDC, fmtAmount, isAddressLike, shortenAddress } from "@/lib/format";
import type { Settings, SpendView } from "@/lib/types";
import {
  cardBase,
  enter,
  inputBase,
  primaryBtn,
  secondaryBtn,
  CloseButton,
  FieldHint,
  PwInput,
} from "@/components/ui";
import { t } from "@/lib/i18n";

type SendStep = "form" | "confirm" | "sending" | "done";
type SendToken = "USDC" | "ETH";

export function SendCard({
  usdcBalance,
  ethBalance,
  settings,
  spend,
  onClose,
  onSent,
}: {
  usdcBalance: string | undefined;
  ethBalance: string | undefined;
  settings: Settings | null;
  spend: SpendView | null;
  onClose: () => void;
  onSent: () => void;
}) {
  const chain = useChain();
  const [step, setStep] = useState<SendStep>("form");
  const [token, setToken] = useState<SendToken>("USDC");
  const [to, setTo] = useState("");
  const [amount, setAmount] = useState("");
  const [pw, setPw] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [txHash, setTxHash] = useState<string | null>(null);
  // 입력 검증 안내는 칸을 벗어났을 때(blur)만 보여준다 — 타이핑 중엔 빨간 에러 안 뜨게.
  const [toTouched, setToTouched] = useState(false);
  const [amountTouched, setAmountTouched] = useState(false);

  // 토큰별 설정. USDC가 이 지갑의 본질이라 기본값.
  const cfg =
    token === "USDC"
      ? { balance: usdcBalance, cmd: "send_usdc", argKey: "amountUsdc", balFrac: 2 }
      : { balance: ethBalance, cmd: "send_eth", argKey: "amountEth", balFrac: 5 };

  // 한도 (설정값 → 없으면 기본 상수). 일일 남은 한도 = 일일 한도 − 오늘 사용액.
  const single = Number(
    token === "USDC" ? settings?.single_usdc ?? MAX_USDC : settings?.single_eth ?? MAX_ETH,
  );
  const daily = Number(token === "USDC" ? settings?.daily_usdc ?? 0 : settings?.daily_eth ?? 0);
  const spent = Number(token === "USDC" ? spend?.usdc ?? 0 : spend?.eth ?? 0);
  // 한도 0 = 무제한.
  const singleUnlimited = single === 0;
  const dailyUnlimited = daily === 0;
  const remaining =
    !dailyUnlimited && settings && spend && Number.isFinite(daily - spent)
      ? Math.max(0, daily - spent)
      : null;

  const amountNum = Number(amount);
  const amountOk =
    amount.trim() !== "" &&
    Number.isFinite(amountNum) &&
    amountNum > 0 &&
    (singleUnlimited || amountNum <= single);
  const toOk = isAddressLike(to);
  const overBalance =
    cfg.balance != null && Number.isFinite(Number(cfg.balance)) && amountNum > Number(cfg.balance);
  const overDaily = remaining != null && amountNum > remaining;
  // USDC 송금도 가스는 ETH로 낸다. ETH가 0이면 전송이 실패하니 미리 안내.
  const noGas = token === "USDC" && (ethBalance == null || Number(ethBalance) === 0);

  function switchToken(t: SendToken) {
    setToken(t);
    setAmount("");
    setAmountTouched(false);
    setError(null);
  }

  async function send() {
    setStep("sending");
    setError(null);
    try {
      const hash = await invoke<string>(cfg.cmd, {
        password: pw,
        to: to.trim(),
        [cfg.argKey]: amount.trim(),
      });
      setTxHash(hash);
      setPw(""); // 전송 성공 후 비번 즉시 비움 (완료 화면 동안 메모리에 남기지 않게)
      setStep("done");
    } catch (e) {
      setError(String(e));
      setStep("confirm");
      setPw("");
    }
  }

  return (
    <motion.section {...enter} className={cn(cardBase, "relative px-8 py-8")}>
      {step !== "sending" && step !== "done" && <CloseButton onClose={onClose} />}

      {/* 헤더 */}
      <div className="flex items-center gap-2">
        {step === "confirm" && (
          <button
            type="button"
            onClick={() => {
              setStep("form");
              setError(null);
              setPw("");
            }}
            className="text-[var(--color-ink-300)] hover:text-[var(--color-ink-700)] transition-colors"
            aria-label={t("뒤로", "Back")}
          >
            <ArrowLeft size={16} />
          </button>
        )}
        <p className="text-[11px] tracking-[0.04em] text-[var(--color-ink-500)]">
          {step === "form" && t(`${token} 보내기`, `Send ${token}`)}
          {step === "confirm" && t("비밀번호로 승인", "Approve with password")}
          {step === "sending" && t("보내는 중", "Sending")}
          {step === "done" && t("보냄", "Sent")}
        </p>
      </div>

      {/* 1) 입력 */}
      {step === "form" && (
        <div className="mt-6 space-y-4">
          {/* 토큰 선택 (세그먼트 컨트롤) */}
          <div className="grid grid-cols-2 gap-1 p-1 rounded-[var(--radius-card)] bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-900)]">
            {(["USDC", "ETH"] as SendToken[]).map((tk) => (
              <button
                key={tk}
                type="button"
                onClick={() => switchToken(tk)}
                className={cn(
                  "h-9 rounded-[10px] text-[13px] tracking-tight",
                  "transition-all duration-[var(--duration-base)] ease-[cubic-bezier(0.4,0,0.2,1)]",
                  token === tk
                    ? "bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-700)] text-[var(--color-ink-900)] dark:text-[#E8E5DD] shadow-[var(--shadow-soft)]"
                    : "text-[var(--color-ink-500)] hover:text-[var(--color-ink-700)]",
                )}
              >
                {tk}
              </button>
            ))}
          </div>

          <div>
            <label className="text-[11px] text-[var(--color-ink-500)]">
              {t("받는 주소", "Recipient address")}
            </label>
            <input
              value={to}
              onChange={(e) => {
                setTo(e.target.value);
                setToTouched(false);
              }}
              onBlur={() => setToTouched(true)}
              placeholder="0x…"
              spellCheck={false}
              className={cn(inputBase, "mt-1.5 font-mono text-[13px]")}
            />
            {toTouched && to.length > 0 && !toOk && (
              <FieldHint>
                {t("0x로 시작하는 42자 주소예요.", "That's a 42-character address starting with 0x.")}
              </FieldHint>
            )}
          </div>

          <div>
            <label className="text-[11px] text-[var(--color-ink-500)]">
              {t(`금액 (${token})`, `Amount (${token})`)}
            </label>
            <input
              value={amount}
              onChange={(e) => {
                setAmount(e.target.value);
                setAmountTouched(false);
              }}
              onBlur={() => setAmountTouched(true)}
              placeholder={token === "USDC" ? "1.00" : "0.001"}
              inputMode="decimal"
              className={cn(inputBase, "mt-1.5 num text-[15px]")}
            />
            <div className="mt-1.5 flex items-center justify-between text-[11px] text-[var(--color-ink-300)]">
              <span>
                {t(
                  `단일 한도 ${singleUnlimited ? "무제한" : `${single} ${token}`}`,
                  `Per-payment limit ${singleUnlimited ? "unlimited" : `${single} ${token}`}`,
                )}
              </span>
              <span className="num">
                {t(
                  `보유 ${fmtAmount(cfg.balance, cfg.balFrac)} ${token}`,
                  `Balance ${fmtAmount(cfg.balance, cfg.balFrac)} ${token}`,
                )}
              </span>
            </div>
            {dailyUnlimited ? (
              <div className="mt-0.5 text-[11px] text-[var(--color-ink-300)]">
                {t("하루 누적 한도 무제한", "Daily total unlimited")}
              </div>
            ) : remaining != null ? (
              <div className="mt-0.5 text-[11px] text-[var(--color-ink-300)] num">
                {t(
                  `오늘 남은 한도 ${fmtAmount(String(remaining), cfg.balFrac)} ${token}`,
                  `${fmtAmount(String(remaining), cfg.balFrac)} ${token} left today`,
                )}
              </div>
            ) : null}
            {amountTouched && amount.length > 0 && !amountOk && (
              <FieldHint>
                {singleUnlimited
                  ? t("0보다 큰 금액을 입력하세요.", "Enter an amount greater than 0.")
                  : t(
                      `${single} ${token} 이하로 입력하세요.`,
                      `Enter ${single} ${token} or less.`,
                    )}
              </FieldHint>
            )}
            {amountTouched && amountOk && overDaily && (
              <FieldHint>
                {t(
                  `오늘 한도를 넘어요 (남은 ${fmtAmount(String(remaining), cfg.balFrac)} ${token}).`,
                  `That's over today's limit (${fmtAmount(String(remaining), cfg.balFrac)} ${token} left).`,
                )}
              </FieldHint>
            )}
            {amountTouched && amountOk && !overDaily && overBalance && (
              <FieldHint>{t(`보유 ${token}보다 많아요.`, `That's more ${token} than you have.`)}</FieldHint>
            )}
            {noGas && (
              <p className="mt-1.5 text-[11px] text-amber-600 dark:text-amber-500">
                {t(
                  "USDC 송금엔 가스용 ETH가 조금 필요해요. 받기 → Faucet에서 ETH 먼저 받으세요.",
                  "Sending USDC needs a little ETH for gas. Go to Receive → Faucet and get some ETH first.",
                )}
              </p>
            )}
          </div>

          <button
            type="button"
            onClick={() => setStep("confirm")}
            disabled={!toOk || !amountOk || overDaily || overBalance || noGas}
            className={cn(primaryBtn, "mt-2")}
          >
            {t("다음", "Next")}
          </button>
        </div>
      )}

      {/* 2) 비번 승인 — 미타테: 금액을 도장 찍듯 확정 */}
      {step === "confirm" && (
        <div className="mt-6">
          <div className="flex flex-col items-center py-2">
            <div className="flex items-baseline gap-1.5">
              <span className="balance text-[44px] leading-none num">{fmtAmount(amount, 6)}</span>
              <span className="text-[14px] text-[var(--color-ink-500)]">{token}</span>
            </div>
            <p className="mt-3 flex items-center gap-1.5 text-[12px] font-mono text-[var(--color-ink-500)]">
              <ArrowUpRight size={12} />
              {shortenAddress(to.trim())}
            </p>
          </div>

          <div className="mt-5">
            <PwInput
              value={pw}
              onChange={setPw}
              placeholder={t("비밀번호", "Password")}
              autoFocus
              onEnter={() => pw && send()}
            />
          </div>

          {error && <p className="mt-3 text-[12px] text-red-500 font-mono break-all">{error}</p>}

          <button type="button" onClick={send} disabled={!pw} className={cn(primaryBtn, "mt-5")}>
            <Lock size={15} />
            {t("보내기", "Send")}
          </button>
        </div>
      )}

      {/* 3) 전송 중 */}
      {step === "sending" && (
        <div className="mt-8 flex flex-col items-center py-8">
          <Loader2 size={28} className="animate-spin text-[var(--color-accent)]" />
          <p className="mt-4 text-[13px] text-[var(--color-ink-500)]">
            {t("서명하고 전송하는 중…", "Signing and sending…")}
          </p>
        </div>
      )}

      {/* 4) 완료 — 도장 찍히는 스프링 모션 */}
      {step === "done" && txHash && (
        <div className="mt-6 flex flex-col items-center py-4">
          <motion.div
            initial={{ scale: 0.4, opacity: 0, rotate: -12 }}
            animate={{ scale: 1, opacity: 1, rotate: 0 }}
            transition={{ type: "spring", stiffness: 380, damping: 16 }}
            className="w-14 h-14 rounded-full flex items-center justify-center bg-[var(--color-accent)] text-white"
          >
            <Check size={26} strokeWidth={3} />
          </motion.div>
          <p className="mt-4 text-[15px] tracking-tight">{t("전송 완료", "Sent")}</p>
          <p className="mt-1 flex items-baseline gap-1 text-[13px] text-[var(--color-ink-500)] num">
            {fmtAmount(amount, 6)} {token}
          </p>

          <button
            type="button"
            onClick={() => openUrl(chain.explorerTx + txHash).catch(() => {})}
            className={cn(secondaryBtn, "mt-5 w-full")}
          >
            <ExternalLink size={13} />
            {t(`${chain.explorerName}에서 보기`, `View on ${chain.explorerName}`)}
          </button>
          <button type="button" onClick={onSent} className={cn(primaryBtn, "mt-2")}>
            {t("확인", "Done")}
          </button>
        </div>
      )}
    </motion.section>
  );
}
