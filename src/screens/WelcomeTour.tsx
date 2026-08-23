// 첫 실행 환영 투어 — 지갑 생성 직후 1회. 페이지형(컨셉→충전→AI연결→안전→시작).
// 중간 페이지 콘텐츠는 helpContent.tsx 의 HELP_SECTIONS 를 재사용한다(도움말과 한 벌).

import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { ArrowLeft, ArrowRight, Check, Sparkles } from "lucide-react";
import { cn } from "@/lib/cn";
import { HELP_SECTIONS, type HelpSection } from "@/lib/helpContent";
import { cardBase, primaryBtn, shell, FlowIcon } from "@/components/ui";
import { t } from "@/lib/i18n";

// 투어에 띄울 핵심 섹션(순서 보장). 컨셉은 인트로가, 백업은 직전 백업 플로우가 다뤘다.
const TOUR_IDS = ["fund", "connect", "safety"] as const;
const byId = (id: string): HelpSection => HELP_SECTIONS.find((s) => s.id === id)!;

type Page =
  | { kind: "intro" }
  | { kind: "section"; section: HelpSection }
  | { kind: "outro" };

const PAGES: Page[] = [
  { kind: "intro" },
  ...TOUR_IDS.map((id) => ({ kind: "section" as const, section: byId(id) })),
  { kind: "outro" },
];

// 페이지 전환 — 진행 방향(dir)에 따라 들어오고 나가는 쪽을 바꾼다(dynamic variants).
const slide = {
  enter: (d: number) => ({ opacity: 0, x: d * 24 }),
  center: { opacity: 1, x: 0 },
  exit: (d: number) => ({ opacity: 0, x: d * -24 }),
};

export function WelcomeTour({
  onDone,
  inert,
}: {
  onDone: () => void;
  // true면 투어를 비활성(결제 승인 모달이 위에 떴을 때 — 동시에 두 모달이 활성되지 않게).
  inert?: boolean;
}) {
  const [i, setI] = useState(0);
  const [dir, setDir] = useState(1);
  const page = PAGES[i];
  const last = i === PAGES.length - 1;

  const go = (next: number) => {
    setDir(next > i ? 1 : -1);
    setI(next);
  };

  return (
    <motion.main
      role="dialog"
      aria-modal={inert ? undefined : "true"}
      aria-label={t("Kura 환영 투어", "Welcome tour")}
      inert={inert ? true : undefined}
      className={cn(shell, "fixed inset-0 z-40 overflow-y-auto")}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.24, ease: [0.4, 0, 0.2, 1] }}
    >
      <header className="w-full max-w-md flex items-center justify-between text-[12px] text-[var(--color-ink-500)]">
        <span className="flex items-center gap-2">
          <span className="inline-block w-1.5 h-1.5 rounded-full bg-[var(--color-accent)]" aria-hidden />
          Kura
        </span>
        {!last && (
          <button
            type="button"
            onClick={onDone}
            className="hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD] transition-colors"
          >
            {t("건너뛰기", "Skip")}
          </button>
        )}
      </header>

      <div className="w-full max-w-md min-h-[19rem] flex items-center">
        <AnimatePresence mode="wait" custom={dir}>
          <motion.section
            key={i}
            custom={dir}
            variants={slide}
            initial="enter"
            animate="center"
            exit="exit"
            transition={{ duration: 0.3, ease: [0.4, 0, 0.2, 1] }}
            className={cn(cardBase, "px-8 py-10")}
          >
            {page.kind === "intro" && (
              <div className="text-center">
                <FlowIcon><Sparkles size={22} /></FlowIcon>
                <h1 className="mt-5 text-[20px] tracking-tight">
                  {t("Kura에 오신 걸 환영해요", "Welcome to Kura")}
                </h1>
                <p className="mt-3 text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                  {t(
                    <>
                      AI가 결제를{" "}
                      <b className="text-[var(--color-ink-700)] dark:text-[#E8E5DD]">요청</b>하고,
                      당신이{" "}
                      <b className="text-[var(--color-ink-700)] dark:text-[#E8E5DD]">
                        비밀번호로 승인
                      </b>
                      하는 지갑이에요.
                      <br />
                      열쇠는 이 컴퓨터를 떠나지 않아요.
                    </>,
                    <>
                      A wallet where the AI{" "}
                      <b className="text-[var(--color-ink-700)] dark:text-[#E8E5DD]">asks</b> and you{" "}
                      <b className="text-[var(--color-ink-700)] dark:text-[#E8E5DD]">
                        approve with your password
                      </b>
                      .
                      <br />
                      The key never leaves this computer.
                    </>,
                  )}
                </p>
                <p className="mt-4 text-[12px] text-[var(--color-ink-300)]">
                  {t("몇 가지만 짚고 시작할게요.", "A few things before you start.")}
                </p>
              </div>
            )}

            {page.kind === "section" && (
              <div>
                <FlowIcon>{page.section.icon}</FlowIcon>
                <h1 className="mt-5 text-center text-[19px] tracking-tight">
                  {page.section.title}
                </h1>
                <div className="mt-4 text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                  {page.section.body}
                </div>
              </div>
            )}

            {page.kind === "outro" && (
              <div className="text-center">
                <FlowIcon><Check size={22} /></FlowIcon>
                <h1 className="mt-5 text-[20px] tracking-tight">{t("준비됐어요", "You're set")}</h1>
                <p className="mt-3 text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                  {t(
                    <>
                      받기로 테스트 코인을 채우고, AI를 연결해 보세요.
                      <br />
                      궁금하면 언제든 헤더의{" "}
                      <b className="text-[var(--color-ink-700)] dark:text-[#E8E5DD]">ⓘ 도움말</b>을
                      누르면 돼요.
                    </>,
                    <>
                      Top up with test coins from Receive, then connect your AI.
                      <br />
                      The{" "}
                      <b className="text-[var(--color-ink-700)] dark:text-[#E8E5DD]">ⓘ Help</b>{" "}
                      button in the header is always there.
                    </>,
                  )}
                </p>
              </div>
            )}
          </motion.section>
        </AnimatePresence>
      </div>

      <div className="w-full max-w-md flex flex-col gap-5">
        {/* 진행 점 */}
        <div className="flex justify-center gap-1.5">
          {PAGES.map((_, idx) => (
            <button
              key={idx}
              type="button"
              onClick={() => go(idx)}
              aria-label={t(`${idx + 1}페이지로`, `Go to page ${idx + 1}`)}
              className={cn(
                "h-1.5 rounded-full transition-all duration-[var(--duration-base)]",
                idx === i
                  ? "w-5 bg-[var(--color-accent)]"
                  : "w-1.5 bg-[var(--color-ivory-400)] dark:bg-[var(--color-night-700)] hover:bg-[var(--color-ink-300)]",
              )}
            />
          ))}
        </div>

        <div className="flex items-center gap-3">
          {i > 0 ? (
            <button
              type="button"
              onClick={() => go(i - 1)}
              aria-label={t("이전", "Back")}
              className={cn(
                "shrink-0 inline-flex items-center justify-center w-11 h-11 rounded-[var(--radius-card)]",
                "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
                "bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]",
                "text-[var(--color-ink-500)] hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD]",
                "transition-colors duration-[var(--duration-base)]",
              )}
            >
              <ArrowLeft size={16} />
            </button>
          ) : (
            <div className="shrink-0 w-11" />
          )}

          <button
            type="button"
            autoFocus
            onClick={() => (last ? onDone() : go(i + 1))}
            className={cn(primaryBtn, "flex-1")}
          >
            {last ? (
              <>
                <Check size={15} /> {t("시작하기", "Get started")}
              </>
            ) : (
              <>
                {t("다음", "Next")} <ArrowRight size={15} />
              </>
            )}
          </button>
        </div>
      </div>
    </motion.main>
  );
}
