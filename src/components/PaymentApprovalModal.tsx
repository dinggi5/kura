// 결제 승인 팝업 (AI 에이전트 → 사람 승인) — 모든 하위 화면 위에 오버레이로 뜬다.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { motion } from "framer-motion";
import {
  AlertTriangle,
  ArrowUpRight,
  Bot,
  FileSignature,
  Fingerprint,
  Globe,
  Loader2,
  Lock,
  ShieldAlert,
  ShieldCheck,
} from "lucide-react";
import { cn } from "@/lib/cn";
import { useChain } from "@/lib/chain";
import { fmtAmount, fmtCountdown, secsLeft, shortenAddress } from "@/lib/format";
import type { AgentTrust, Balances, PaymentRequest } from "@/lib/types";
import { modalCard, modalOverlay, primaryBtn, secondaryBtn, PwInput } from "@/components/ui";
import { t } from "@/lib/i18n";

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
      // 이 문구는 내역에 그대로 저장된다 — 나중에 언어를 바꿔도 옛 기록은 그때 언어로 남는다
      // (기록은 "그때 그렇게 보였다"의 사본이라 소급해 고치지 않는다).
      await invoke("reject_payment", {
        id: request.id,
        reason: t("사용자가 거부함", "Rejected by the user"),
      });
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
            {isX402
              ? t("AI x402 결제 요청", "AI x402 payment request")
              : t("AI 결제 요청", "AI payment request")}
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
          {!chain.testnet && t(" · 실제 자금", " · real funds")}
        </div>

        {/* 무엇에 대한 결제인지 */}
        {request.memo ? (
          <p className="mt-4 text-[13px] leading-relaxed text-[var(--color-ink-700)] dark:text-[#B5AFA2]">
            {request.memo}
          </p>
        ) : (
          <p className="mt-4 text-[13px] text-[var(--color-ink-300)]">
            {t("결제 사유가 제공되지 않았어요.", "No reason was given for this payment.")}
          </p>
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
              {t("이전에 승인한 주소", "You've approved this address before")}
            </p>
          )}
          {trusted === false && (
            <p className="mt-1.5 flex items-center gap-1.5 text-[11px] text-amber-600 dark:text-amber-500">
              <AlertTriangle size={11} />
              {t("처음 보는 주소예요", "You haven't sent here before")}
            </p>
          )}
          {/* ERC-8004 대조 (개발 47) — AI 가 에이전트 번호를 준 결제에만 붙는다.
              번호가 없으면 아무것도 안 붙고 창은 예전 그대로다: 말할 사실이 있을 때만 말한다. */}
          {request.agent && <AgentTrustLines agent={request.agent} />}
          {isX402 && (
            <p className="mt-1.5 flex items-center gap-1.5 text-[11px] text-[var(--color-ink-300)]">
              <FileSignature size={11} />
              {t(
                "오프체인 서명 · 가스 없음 (정산은 페이실리테이터)",
                "Off-chain signature · no gas (the facilitator settles it)",
              )}
            </p>
          )}
        </div>

        {locked && (
          <p className="mt-1 flex items-center gap-1.5 text-[11px] text-red-600 dark:text-red-500">
            <ShieldAlert size={12} />
            {t(
              "긴급 잠금 중 — 승인해도 차단돼요. 먼저 잠금을 해제하세요.",
              "Emergency lock is on — approving won't send. Turn the lock off first.",
            )}
          </p>
        )}
        {!tokenOk && (
          <p className="mt-1 text-[11px] text-red-500">
            {t(`알 수 없는 토큰: ${request.token}`, `Unknown token: ${request.token}`)}
          </p>
        )}
        {insufficient && (
          <p className="mt-1 flex items-center gap-1.5 text-[11px] text-red-600 dark:text-red-500">
            <AlertTriangle size={12} className="shrink-0" />
            {t(
              `${request.token} 잔액이 부족해요 · 보유 ${fmtAmount(haveStr ?? "0", 6)} / 필요 ${fmtAmount(request.amount, 6)}`,
              `Not enough ${request.token} · you have ${fmtAmount(haveStr ?? "0", 6)}, this needs ${fmtAmount(request.amount, 6)}`,
            )}
            {isX402 && t(" (정산 시 실패)", " (settlement would fail)")}
          </p>
        )}

        <div className="mt-5">
          <PwInput
            value={pw}
            onChange={setPw}
            placeholder={
              isX402
                ? t("비밀번호로 서명 승인", "Password to approve the signature")
                : t("비밀번호로 승인", "Password to approve")
            }
            autoFocus
            onEnter={() => pw && !busy && tokenOk && !locked && !insufficient && approve()}
          />
        </div>

        {error && <p className="mt-3 text-[12px] text-red-500 font-mono break-all">{error}</p>}

        <div className="mt-5 grid grid-cols-[1fr_1.8fr] gap-2">
          <button type="button" onClick={reject} disabled={busy} className={cn(secondaryBtn, "w-full")}>
            {t("거부", "Reject")}
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
            {isX402 ? t("승인하고 서명", "Approve and sign") : t("승인하고 보내기", "Approve and send")}
          </button>
        </div>
      </motion.section>
    </motion.div>
  );
}

/** 두 값의 대조 결과. `warn` 이면 **줄을 따로 세우고**(경고), 아니면 신원 줄 뒤에 짧게 붙인다.
 *  — 조용한 경우는 조용하게, 말할 게 생겼을 때만 자리를 차지한다.
 *
 *  왜 "검증됨"이라 안 쓰나: ERC-8004 등록은 무허가다. 누구나 아무 도메인이나 적어 등록할 수
 *  있어서, 일치는 "그 주장이 자기 자신과 앞뒤가 맞는다"는 뜻이지 안전을 뜻하지 않는다.
 *  반대로 **불일치는 강한 신호**다(받는 주소가 바꿔치기된 정황) → 그때만 색과 줄을 준다. */
function comparison(agent: AgentTrust): { warn: boolean; text: string } {
  const w = agent.wallet_check;
  const d = agent.domain_check;

  // ── 다름: 경고 한 줄 (완결된 문장으로 또렷하게)
  if (w === "differs" && d === "differs")
    return {
      warn: true,
      text: t(
        "받는 주소·기재 도메인이 모두 온체인 기록과 달라요",
        "Both the address and the listed domain differ from the record",
      ),
    };
  if (w === "differs")
    return {
      warn: true,
      text: t(
        "받는 주소가 온체인 등록 지갑과 달라요",
        "The address differs from the registered wallet",
      ),
    };
  if (d === "differs")
    return {
      warn: true,
      text: t("기재 도메인이 이 리소스와 달라요", "The listed domain differs from this resource"),
    };

  // ── 같음·모름: 신원 줄 꼬리에 붙는 짧은 말
  if (w === "match" && d === "match")
    return { warn: false, text: t("주소·도메인 일치", "address and domain match") };
  if (w === "match") return { warn: false, text: t("주소 일치", "address matches") };
  if (w === "unset" && d === "match")
    return { warn: false, text: t("도메인 일치 · 등록 지갑 없음", "domain matches · no wallet on record") };
  if (w === "unset") return { warn: false, text: t("등록 지갑 없음", "no wallet on record") };
  if (d === "match") return { warn: false, text: t("도메인 일치", "domain matches") };
  return { warn: false, text: t("대조할 값 없음", "nothing to compare") };
}

/** 온체인 기록 대조 (개발 47). 조용한 결과는 **한 줄**, 어긋난 결과만 경고 줄을 하나 더 쓴다.
 *
 *  일부러 뺀 것 둘 —
 *  ① 등록 문서의 자기신고 **이름**: 사람 눈앞에 이름을 크게 띄우는 순간 그게 사칭의 통로가 된다
 *     (누구나 "Coinbase"로 등록할 수 있다). AI 는 lookup_agent 결과로 따로 본다.
 *  ② **피드백 건수**: 시빌 가능해서 정직하려면 "누구나 남길 수 있어요" 단서를 늘 달아야 하는데,
 *     그러면 승인 판단에 보탬은 없이 줄만 하나 더 든다. 숫자는 lookup_agent 로 넘겼다. */
function AgentTrustLines({ agent }: { agent: AgentTrust }) {
  const gray = "mt-1.5 flex items-center gap-1.5 text-[11px] text-[var(--color-ink-300)]";
  const amber = "mt-1.5 flex items-center gap-1.5 text-[11px] text-amber-600 dark:text-amber-500";

  // 번호는 왔는데 온체인에 없다 = 그 자체가 말할 사실이다.
  if (!agent.registered) {
    return (
      <p className={amber}>
        <AlertTriangle size={11} className="shrink-0" />
        {t(
          `온체인에 없는 에이전트 번호예요 · #${agent.agent_id}`,
          `No agent #${agent.agent_id} exists on-chain`,
        )}
      </p>
    );
  }

  const cmp = comparison(agent);
  return (
    <>
      <p className={cn(gray, "max-w-full break-all text-center")}>
        <Fingerprint size={11} className="shrink-0" />
        <span>
          {t("온체인 기재", "On-chain record")} · #{agent.agent_id}
          {agent.uri_domain && ` · ${agent.uri_domain}`}
          {!cmp.warn && ` · ${cmp.text}`}
        </span>
      </p>
      {cmp.warn && (
        <p className={amber}>
          <AlertTriangle size={11} className="shrink-0" />
          {cmp.text}
        </p>
      )}
    </>
  );
}
