// 시드 백업 플로우.
// onboard: 새 지갑 생성 직후 (비번 보유) → 안내 → 12단어 → 확인 퀴즈
// review:  나중에 다시 보기 → 비번 입력 → 안내 → 12단어 → 확인(퀴즈 없음)

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { motion } from "framer-motion";
import { ArrowLeft, Check, Copy, Eye, EyeOff, KeyRound, Loader2 } from "lucide-react";
import { cn } from "@/lib/cn";
import { useCopy } from "@/lib/useCopy";
import {
  cardBase,
  enter,
  primaryBtn,
  secondaryBtn,
  shell,
  FlowIcon,
  PwInput,
} from "@/components/ui";

type BackupStep = "password" | "loading" | "intro" | "words" | "verify";
type Challenge = { index: number; answer: string; options: string[] };

function shuffle<T>(arr: T[]): T[] {
  const a = [...arr];
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

/** 12단어 중 3개를 골라 객관식 확인 문제를 만든다. 보기는 같은 시드의 다른 단어들. */
function makeChallenges(words: string[]): Challenge[] {
  const idxs = shuffle(words.map((_, i) => i)).slice(0, 3).sort((a, b) => a - b);
  return idxs.map((index) => {
    const answer = words[index];
    const decoyPool = shuffle([...new Set(words.filter((w) => w !== answer))]);
    const options = shuffle([answer, ...decoyPool.slice(0, 3)]);
    return { index, answer, options };
  });
}

export function BackupFlow({
  mode,
  initialPassword,
  onComplete,
  onExit,
}: {
  mode: "onboard" | "review";
  initialPassword?: string;
  onComplete: () => void;
  onExit: () => void;
}) {
  const [step, setStep] = useState<BackupStep>(initialPassword ? "loading" : "password");
  const [pw, setPw] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [words, setWords] = useState<string[] | null>(null);
  const [revealed, setRevealed] = useState(false);
  const [copied, copy] = useCopy();
  const [challenges, setChallenges] = useState<Challenge[] | null>(null);

  const reveal = useCallback(async (password: string) => {
    setBusy(true);
    setError(null);
    try {
      const w = await invoke<string[]>("reveal_mnemonic", { password });
      setWords(w);
      setStep("intro");
    } catch (e) {
      setError(String(e));
      setStep("password");
    } finally {
      setBusy(false);
    }
  }, []);

  // onboard: 비번을 들고 있으니 바로 열람한다.
  useEffect(() => {
    if (initialPassword) void reveal(initialPassword);
  }, [initialPassword, reveal]);

  async function finish() {
    try {
      await invoke("mark_backed_up");
    } catch {
      /* 표시 실패해도 백업 자체는 됐으니 통과 */
    }
    onComplete();
  }

  return (
    <main className={shell}>
      <header className="w-full max-w-md flex items-center justify-between text-[12px] text-[var(--color-ink-500)]">
        <span className="flex items-center gap-2">
          <KeyRound size={12} className="text-[var(--color-accent)]" />
          시드 백업
        </span>
        <button
          type="button"
          onClick={onExit}
          className="hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD] transition-colors"
        >
          {mode === "onboard" ? "나중에" : "닫기"}
        </button>
      </header>

      <motion.section {...enter} className={cn(cardBase, "max-w-md px-8 py-9")}>
        {/* 0) 비번 입력 (review 진입 시) */}
        {step === "password" && (
          <>
            <FlowIcon><KeyRound size={22} /></FlowIcon>
            <h1 className="mt-5 text-center text-[19px] tracking-tight">시드 12단어 보기</h1>
            <p className="mt-2 text-center text-[13px] leading-relaxed text-[var(--color-ink-500)]">
              키를 복호화하려면 비밀번호가 필요해요.
            </p>
            <div className="mt-7">
              <PwInput value={pw} onChange={setPw} placeholder="비밀번호" autoFocus onEnter={() => pw && reveal(pw)} />
            </div>
            {error && <p className="mt-4 text-[12px] text-red-500 font-mono break-all">{error}</p>}
            <button type="button" onClick={() => reveal(pw)} disabled={!pw || busy} className={cn(primaryBtn, "mt-6")}>
              {busy ? <Loader2 size={15} className="animate-spin" /> : <Eye size={15} />}
              단어 보기
            </button>
          </>
        )}

        {/* 로딩 (onboard 초기 복호화) */}
        {step === "loading" && (
          <div className="flex flex-col items-center py-10">
            <Loader2 size={26} className="animate-spin text-[var(--color-accent)]" />
            <p className="mt-4 text-[13px] text-[var(--color-ink-500)]">시드 준비 중…</p>
            {error && <p className="mt-4 text-[12px] text-red-500 font-mono break-all">{error}</p>}
          </div>
        )}

        {/* 1) 안내 */}
        {step === "intro" && (
          <>
            <FlowIcon><KeyRound size={22} /></FlowIcon>
            <h1 className="mt-5 text-center text-[19px] tracking-tight">복구 시드 백업</h1>
            <p className="mt-3 text-center text-[13px] leading-relaxed text-[var(--color-ink-500)]">
              이 12단어가 자산의 <b className="text-[var(--color-ink-700)] dark:text-[#E8E5DD]">진짜 소유 증명</b>이에요.
              비밀번호를 잊어도 이 단어만 있으면 가져오기나 다른 표준 지갑에서 자산을 되찾을 수 있어요.
            </p>
            <ul className="mt-5 space-y-2 text-[12px] leading-relaxed text-[var(--color-ink-500)]">
              <li>• 종이에 적거나 비밀번호 관리자에 저장하세요.</li>
              <li>• 누구에게도 보여주지 마세요 — 단어 = 자산.</li>
              <li>• 스크린샷·캡처는 피하세요.</li>
            </ul>
            <button type="button" onClick={() => setStep("words")} className={cn(primaryBtn, "mt-7")}>
              <Eye size={15} />
              12단어 보기
            </button>
          </>
        )}

        {/* 2) 12단어 표시 */}
        {step === "words" && words && (
          <>
            <div className="flex items-center justify-between">
              <p className="text-[11px] tracking-[0.04em] text-[var(--color-ink-500)]">복구 시드 (12단어)</p>
              <button
                type="button"
                onClick={() => setRevealed((v) => !v)}
                className="inline-flex items-center gap-1.5 text-[11px] text-[var(--color-ink-500)] hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD] transition-colors"
              >
                {revealed ? <EyeOff size={13} /> : <Eye size={13} />}
                {revealed ? "가리기" : "보기"}
              </button>
            </div>

            <div className="relative mt-4">
              <div className={cn("grid grid-cols-2 gap-2 transition-all", !revealed && "blur-sm select-none")}>
                {words.map((w, i) => (
                  <div
                    key={i}
                    className={cn(
                      "flex items-center gap-2.5 h-10 px-3 rounded-[10px]",
                      "bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]",
                      "border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)]",
                    )}
                  >
                    <span className="w-4 text-right num text-[11px] text-[var(--color-ink-300)]">{i + 1}</span>
                    <span className="font-mono text-[13px] text-[var(--color-ink-900)] dark:text-[#E8E5DD]">{w}</span>
                  </div>
                ))}
              </div>
              {!revealed && (
                <button
                  type="button"
                  onClick={() => setRevealed(true)}
                  className="absolute inset-0 flex items-center justify-center text-[13px] text-[var(--color-ink-700)] dark:text-[#B5AFA2]"
                >
                  <span className="inline-flex items-center gap-2 px-4 py-2 rounded-[var(--radius-pill)] bg-[var(--color-ivory-50)]/80 dark:bg-[var(--color-night-800)]/80 border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)]">
                    <Eye size={14} /> 탭해서 보기
                  </span>
                </button>
              )}
            </div>

            <button type="button" onClick={() => copy(words.join(" "))} className={cn(secondaryBtn, "mt-4 w-full")}>
              {copied ? <><Check size={13} className="text-[var(--color-accent)]" /> 복사됨</> : <><Copy size={13} /> 12단어 복사</>}
            </button>
            <p className="mt-2 text-center text-[11px] text-[var(--color-ink-300)]">
              클립보드는 다른 앱이 읽을 수 있어요. 저장 후 지우는 걸 권장해요.
            </p>

            {mode === "onboard" ? (
              <button
                type="button"
                onClick={() => {
                  setChallenges(makeChallenges(words));
                  setStep("verify");
                }}
                className={cn(primaryBtn, "mt-5")}
              >
                적었어요, 확인하기
              </button>
            ) : (
              <button type="button" onClick={finish} className={cn(primaryBtn, "mt-5")}>
                <Check size={15} /> 확인했어요
              </button>
            )}
          </>
        )}

        {/* 3) 확인 퀴즈 (onboard 전용) */}
        {step === "verify" && challenges && (
          <BackupQuiz challenges={challenges} onPass={finish} onBack={() => setStep("words")} />
        )}
      </motion.section>

      <div />
    </main>
  );
}

function BackupQuiz({
  challenges,
  onPass,
  onBack,
}: {
  challenges: Challenge[];
  onPass: () => void;
  onBack: () => void;
}) {
  const [qi, setQi] = useState(0);
  const [wrong, setWrong] = useState(false);
  const c = challenges[qi];

  function pick(opt: string) {
    if (opt === c.answer) {
      setWrong(false);
      if (qi + 1 >= challenges.length) onPass();
      else setQi(qi + 1);
    } else {
      setWrong(true);
    }
  }

  return (
    <div>
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={onBack}
          className="text-[var(--color-ink-300)] hover:text-[var(--color-ink-700)] transition-colors"
          aria-label="뒤로"
        >
          <ArrowLeft size={16} />
        </button>
        <p className="text-[11px] tracking-[0.04em] text-[var(--color-ink-500)]">
          백업 확인 ({qi + 1}/{challenges.length})
        </p>
      </div>

      <p className="mt-6 text-center text-[15px] tracking-tight">
        <span className="num text-[var(--color-accent)]">{c.index + 1}</span>번째 단어는?
      </p>

      <div className="mt-5 grid grid-cols-2 gap-2">
        {c.options.map((opt) => (
          <button
            key={opt}
            type="button"
            onClick={() => pick(opt)}
            className={cn(
              "h-11 rounded-[var(--radius-card)] font-mono text-[13px]",
              "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
              "bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]",
              "text-[var(--color-ink-900)] dark:text-[#E8E5DD]",
              "transition-all duration-[var(--duration-base)] ease-[cubic-bezier(0.4,0,0.2,1)]",
              "hover:border-[var(--color-accent)] hover:translate-y-[-1px]",
              "active:translate-y-0",
            )}
          >
            {opt}
          </button>
        ))}
      </div>

      {wrong && (
        <p className="mt-4 text-center text-[12px] text-red-500/90">
          틀렸어요. 다시 확인하고 골라주세요.
        </p>
      )}
    </div>
  );
}
