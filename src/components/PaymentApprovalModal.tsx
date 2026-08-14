// 결제 승인 팝업 (AI 에이전트 → 사람 승인) — 모든 하위 화면 위에 오버레이로 뜬다.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { motion } from "framer-motion";
import {
  AlertTriangle,
  ArrowUpRight,
  Bot,
  FileSignature,
  Globe,
  Loader2,
  Lock,
  ShieldAlert,
  ShieldCheck,
} from "lucide-react";
import { cn } from "@/lib/cn";
import { useChain } from "@/lib/chain";
import { fmtAmount, fmtCountdown, secsLeft, shortenAddress } from "@/lib/format";
import type { Balances, PaymentRequest } from "@/lib/types";
import { modalCard, modalOverlay, primaryBtn, secondaryBtn, PwInput } from "@/components/ui";

/// 십진 문자열을 토큰 decimals 기준 base unit(BigInt)으로. 형식이 아니면 null(검사 생략).
/// 부동소수 비교의 정밀도 손실을 피하려고 정수 비교용으로 쓴다.
function toBaseUnits(decimal: string, decimals: number): bigint | null {
  const s = decimal.trim();
  if (!/^\d+(\.\d+)?$/.test(s)) return null;
  const [whole, frac = ""] = s.split(".");
  const fracPadded = (frac + "0".repeat(decimals)).slice(0, decimals);
  try {
    return BigInt(whole + fracPadded);
  } catch {
    return null;
  }
}

export function PaymentApprovalModal({
  request,
  locked,
  balances,
  onResolved,
}: {
  request: PaymentRequest;
  locked: boolean;
  // 메인 화면이 이미 폴링 중인 잔액 — 사전 검사용(없으면 검사 생략, 백엔드가 최종 방어).
  balances: Balances | null;
  onResolved: () => void;
}) {
  const chain = useChain();
  const [pw, setPw] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [remaining, setRemaining] = useState(() => secsLeft(request.created));
  // 받는 주소가 이전에 비번으로 승인된 적 있는지 (null = 조회 중).
  const [trusted, setTrusted] = useState<boolean | null>(null);

  // 5분 카운트다운. 0이 되면 MCP가 요청을 거둬가고 폴링이 팝업을 닫는다.
  useEffect(() => {
    const h = setInterval(() => setRemaining(secsLeft(request.created)), 1000);
    return () => clearInterval(h);
  }, [request.created]);

  useEffect(() => {
    invoke<boolean>("is_trusted_addr", { to: request.to })
      .then(setTrusted)
      .catch(() => setTrusted(null));
  }, [request.to]);

  const isX402 = request.kind === "x402";
  // x402는 USDC 서명만(온체인 전송 X). transfer는 USDC/ETH 송금.
  const tokenOk = isX402 ? request.token === "USDC" : request.token === "USDC" || request.token === "ETH";

  // 사전 잔액 검사 — 거부밖에 못 누르는 막다른 길(잔액 부족 revert)을 미리 막는다. 잔액을 아직
  // 못 읽었으면(balances=null)·파싱 실패면 검사를 생략한다. ⚠️ 이건 UX 가드일 뿐 보안 경계가
  // 아니다 — 우회해도 자금 손실은 없다(transfer 는 체인에서, x402 는 정산 시 온체인에서 revert).
  // 실제 방어선은 백엔드의 한도·긴급잠금·체인 revert. x402 도 USDC 부족이면 정산이 실패하므로 동일 검사.
  // 비교는 토큰 decimals 기준 base unit BigInt 로 — JS Number 는 18자리 ETH·큰 수에서 정밀도 손실.
  const haveStr = request.token === "ETH" ? balances?.eth : balances?.usdc;
  const decimals = request.token === "ETH" ? 18 : 6; // 두 Base 체인 모두 USDC=6
  const needUnits = toBaseUnits(request.amount, decimals);
  const haveUnits = haveStr != null ? toBaseUnits(haveStr, decimals) : null;
  const insufficient = needUnits != null && haveUnits != null && needUnits > haveUnits;

  async function approve() {
    setBusy(true);
    setError(null);
    try {
      await invoke("approve_payment", { id: request.id, password: pw });
      setPw(""); // 성공 후 비번 즉시 비움 (입력칸 수명에만 두는 불변식)
      onResolved(); // 성공 → 팝업 닫고 잔액/내역 갱신
    } catch (e) {
      // 비번 오류·잠금·한도 등 — 요청은 살아있으니 재시도하거나 거부할 수 있다.
      setError(String(e));
      setPw("");
      setBusy(false);
    }
  }

  async function reject() {
    setBusy(true);
    try {
      await invoke("reject_payment", { id: request.id, reason: "사용자가 거부함" });
    } catch {
      /* 거부 실패는 무시 — 어차피 닫는다 */
    }
    onResolved();
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
        {/* 요청 출처 + 카운트다운 */}
        <div className="flex items-center justify-between">
          <span className="flex items-center gap-1.5 text-[11px] tracking-[0.04em] text-[var(--color-accent)]">
            <Bot size={13} />
            {isX402 ? "AI x402 결제 요청" : "AI 결제 요청"}
          </span>
          <span className="num text-[11px] text-[var(--color-ink-300)]">{fmtCountdown(remaining)}</span>
        </div>

        {/* 어느 네트워크에서 나가는 결제인지 — 메인넷이면 실제 자금이라 강조 */}
        <div
          className={cn(
            "mt-2 inline-flex items-center gap-1 text-[11px]",
            chain.testnet ? "text-[var(--color-ink-300)]" : "text-amber-600 dark:text-amber-500",
          )}
        >
          <Globe size={11} />
          {chain.name}
          {!chain.testnet && " · 실제 자금"}
        </div>

        {/* 무엇에 대한 결제인지 */}
        {request.memo ? (
          <p className="mt-4 text-[13px] leading-relaxed text-[var(--color-ink-700)] dark:text-[#B5AFA2]">
            {request.memo}
          </p>
        ) : (
          <p className="mt-4 text-[13px] text-[var(--color-ink-300)]">결제 사유가 제공되지 않았어요.</p>
        )}

        {/* x402: 결제 대상 리소스 URL */}
        {isX402 && request.resource && (
          <p className="mt-2 flex items-center gap-1.5 text-[11px] font-mono text-[var(--color-ink-500)] break-all">
            <Globe size={12} className="shrink-0" />
            {request.resource}
          </p>
        )}

        {/* 금액 + 받는 주소 (도장 찍듯 확정) */}
        <div className="mt-4 flex flex-col items-center py-2">
          <div className="flex items-baseline gap-1.5">
            <span className="balance text-[40px] leading-none num">{fmtAmount(request.amount, 6)}</span>
            <span className="text-[14px] text-[var(--color-ink-500)]">{request.token}</span>
          </div>
          <p className="mt-2.5 flex items-center gap-1.5 text-[12px] font-mono text-[var(--color-ink-500)]">
            <ArrowUpRight size={12} />
            {shortenAddress(request.to)}
          </p>
          {/* 신뢰 여부 (Session 16) — 보호자가 승인 판단에 쓰는 핵심 신호 */}
          {trusted === true && (
            <p className="mt-1.5 flex items-center gap-1.5 text-[11px] text-[var(--color-ink-300)]">
              <ShieldCheck size={11} />
              이전에 승인한 주소
            </p>
          )}
          {trusted === false && (
            <p className="mt-1.5 flex items-center gap-1.5 text-[11px] text-amber-600 dark:text-amber-500">
              <AlertTriangle size={11} />
              처음 보는 주소예요
            </p>
          )}
          {isX402 && (
            <p className="mt-1.5 flex items-center gap-1.5 text-[11px] text-[var(--color-ink-300)]">
              <FileSignature size={11} />
              오프체인 서명 · 가스 없음 (정산은 페이실리테이터)
            </p>
          )}
        </div>

        {locked && (
          <p className="mt-1 flex items-center gap-1.5 text-[11px] text-red-600 dark:text-red-500">
            <ShieldAlert size={12} />
            긴급 잠금 중 — 승인해도 차단돼요. 먼저 잠금을 해제하세요.
          </p>
        )}
        {!tokenOk && (
          <p className="mt-1 text-[11px] text-red-500">알 수 없는 토큰: {request.token}</p>
        )}
        {insufficient && (
          <p className="mt-1 flex items-center gap-1.5 text-[11px] text-red-600 dark:text-red-500">
            <AlertTriangle size={12} className="shrink-0" />
            {request.token} 잔액이 부족해요 · 보유 {fmtAmount(haveStr ?? "0", 6)} / 필요{" "}
            {fmtAmount(request.amount, 6)}
            {isX402 && " (정산 시 실패)"}
          </p>
        )}

        <div className="mt-5">
          <PwInput
            value={pw}
            onChange={setPw}
            placeholder={isX402 ? "비밀번호로 서명 승인" : "비밀번호로 승인"}
            autoFocus
            onEnter={() => pw && !busy && tokenOk && !locked && !insufficient && approve()}
          />
        </div>

        {error && <p className="mt-3 text-[12px] text-red-500 font-mono break-all">{error}</p>}

        <div className="mt-5 grid grid-cols-[1fr_1.8fr] gap-2">
          <button type="button" onClick={reject} disabled={busy} className={cn(secondaryBtn, "w-full")}>
            거부
          </button>
          <button
            type="button"
            onClick={approve}
            disabled={!pw || busy || !tokenOk || locked || insufficient}
            className={primaryBtn}
          >
            {busy ? (
              <Loader2 size={15} className="animate-spin" />
            ) : isX402 ? (
              <FileSignature size={15} />
            ) : (
              <Lock size={15} />
            )}
            {isX402 ? "승인하고 서명" : "승인하고 보내기"}
          </button>
        </div>
      </motion.section>
    </motion.div>
  );
}
