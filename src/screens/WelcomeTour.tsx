// 첫 실행 환영 투어 — 지갑 생성 직후 1회. 페이지형(컨셉→충전→AI연결→안전→시작).
// 충전·안전 페이지 콘텐츠는 helpContent.tsx 의 HELP_SECTIONS 를 재사용한다(도움말과 한 벌).
// AI 연결 페이지만 읽을거리가 아니라 **그 자리에서 연결하는** 단계다(개발 74 — ConnectStep).

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AnimatePresence, motion } from "framer-motion";
import { ArrowLeft, ArrowRight, Check, Loader2, Sparkles } from "lucide-react";
import { cn } from "@/lib/cn";
import { HELP_SECTIONS, type HelpSection } from "@/lib/helpContent";
import type { ConnectError, ConnectStatus } from "@/lib/types";
import { cardBase, primaryBtn, secondaryBtn, shell, FlowIcon, Switch } from "@/components/ui";
import { t } from "@/lib/i18n";

const byId = (id: string): HelpSection => HELP_SECTIONS.find((s) => s.id === id)!;

type Page =
  | { kind: "intro" }
  | { kind: "section"; section: HelpSection }
  | { kind: "connect" }
  | { kind: "outro" };

// 순서 보장. 컨셉은 인트로가, 백업은 직전 백업 플로우가 다뤘다.
const PAGES: Page[] = [
  { kind: "intro" },
  { kind: "section", section: byId("fund") },
  { kind: "connect" },
  { kind: "section", section: byId("safety") },
  { kind: "outro" },
];

const em = "text-[var(--color-ink-700)] dark:text-[#E8E5DD]";

/** 연결 단계의 한 줄 — 이미 돼 있으면 ✓, 아니면 스위치(기본 켬). */
function StepRow({
  title,
  desc,
  done,
  doneLabel,
  hint,
  checked,
  onToggle,
}: {
  title: string;
  desc: string;
  done: boolean;
  doneLabel?: string;
  /** 앱이 대신 해 줄 수 없는 일 — 스위치 대신 할 일을 적는다(예: 꺼 둔 확장). */
  hint?: string;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="flex items-center justify-between gap-3 py-2.5">
      <div className="min-w-0">
        <p className="text-[13px] tracking-tight text-[var(--color-ink-700)] dark:text-[#E8E5DD]">
          {title}
        </p>
        <p className="mt-0.5 text-[11px] leading-snug text-[var(--color-ink-300)]">{desc}</p>
      </div>
      {hint && !done ? (
        <span className="shrink-0 text-right text-[11px] leading-snug text-[var(--color-ink-500)]">
          {hint}
        </span>
      ) : done ? (
        <span className="shrink-0 inline-flex items-center gap-1 text-[11px] text-[var(--color-accent)]">
          <Check size={12} /> {doneLabel ?? t("돼 있어요", "Done")}
        </span>
      ) : (
        <Switch checked={checked} onToggle={onToggle} label={title} />
      )}
    </div>
  );
}

/** 「Claude 와 연결할까요?」 — 지갑을 만든 직후 한 번 묻는 자리 (개발 74 「깔면 늘 붙어 있다」).
 *
 *  전엔 투어가 연결 **방법**만 읽어 주고, 실제 연결은 사람이 「AI 연결」 화면을 스스로 찾아가야 했다.
 *  여기서는 이 맥에서 찾은 것만 줄로 보여 주고(못 찾은 건 아예 안 그린다), 「연결하기」 한 번에 끝낸다.
 *  스위치 기본값은 켬이지만 **누르기 전엔 아무것도 안 한다** — 남의 설정(~/.claude.json·로그인 항목)을
 *  동의 없이 만들지 않는다. 건너뛰기(아래 「다음」)도 그대로 열려 있다.
 *
 *  데스크톱 확장은 맨 마지막에 연다 — Claude 앱이 앞으로 나와 설치 창을 띄우므로, 그 뒤의 일은 이 창이
 *  못 본다. 자동 시작도 같이 묻는 이유: 앱이 떠 있어야 입금이 기록되고 승인 창이 바로 뜬다(꺼져 있으면
 *  결제 요청 때 Claude 가 깨우긴 한다). */
function ConnectStep({ onSettled }: { onSettled: () => void }) {
  const [status, setStatus] = useState<ConnectStatus | null>(null);
  const [autostartOn, setAutostartOn] = useState<boolean | null>(null);
  const [pickCode, setPickCode] = useState(true);
  const [pickDesktop, setPickDesktop] = useState(true);
  const [pickAutostart, setPickAutostart] = useState(true);
  const [busy, setBusy] = useState(false);
  // 누른 뒤의 결과. 실행한 줄은 스위치 대신 결과를 보인다 — 남은 스위치는 「아직 할 일」처럼 읽힌다.
  const [result, setResult] = useState<{
    errors: string[];
    codeOk: boolean;
    desktopOpened: boolean;
  } | null>(null);

  useEffect(() => {
    invoke<ConnectStatus>("get_connect_status").then(setStatus).catch(() => {});
    invoke<boolean>("get_autostart").then(setAutostartOn).catch(() => setAutostartOn(false));
  }, []);

  // 처음부터 할 일이 없으면(다 돼 있음·Claude 못 찾음) 「나중에」라 부를 것도 없다 — 아래 버튼을 「다음」으로.
  const loaded = !!status && autostartOn !== null;
  const actionable =
    loaded &&
    ((!!status.cli_path && !status.cli_registered && !(status.temp_location && !status.mcp_path)) ||
      (status.desktop_installed && !status.desktop_ext_installed) ||
      !autostartOn);
  useEffect(() => {
    if (loaded && !actionable && !result) onSettled();
    // 첫 감지 한 번만 본다 — 스위치를 다 꺼서 할 일이 없어진 건 「나중에」가 맞다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded]);

  if (!status || autostartOn === null) {
    return (
      <div className="flex justify-center py-6 text-[var(--color-ink-300)]">
        <Loader2 size={16} className="animate-spin" />
      </div>
    );
  }

  const tempNoPath = status.temp_location && !status.mcp_path;
  const codeDone = status.cli_registered;
  const codeShown = codeDone || (!!status.cli_path && !tempNoPath);
  // 깔려 있어도 Claude 설정에서 꺼 뒀으면 Claude 를 켜도 우리를 안 띄운다 — 「돼 있어요」가 아니다(코덱스 개발74 1차 P2).
  // 다시 설치 창을 여는 건 답이 아니라(켜고 끄는 건 Claude 쪽 스위치) 할 일을 적는다.
  const desktopOff = status.desktop_ext_installed && status.desktop_ext_disabled;
  const desktopDone = status.desktop_ext_installed && !status.desktop_ext_disabled;
  const desktopShown = status.desktop_installed;
  const doCode = codeShown && !codeDone && pickCode;
  const doDesktop = desktopShown && !status.desktop_ext_installed && pickDesktop;
  const doAutostart = !autostartOn && pickAutostart;
  const nothingToDo = !doCode && !doDesktop && !doAutostart;

  const run = async () => {
    setBusy(true);
    const errors: string[] = [];
    let desktopOpened = false;
    let codeOk = false;
    if (doAutostart) {
      await invoke("set_autostart", { enabled: true })
        .then(() => setAutostartOn(true))
        .catch((e) => errors.push(t(`자동 시작: ${String(e)}`, `Start at login: ${String(e)}`)));
    }
    if (doCode) {
      await invoke("connect_claude_code")
        .then(() => (codeOk = true))
        .catch((e) => {
        const err = e as Partial<ConnectError> | undefined;
        errors.push(`Claude Code: ${typeof err?.message === "string" ? err.message : String(e)}`);
      });
    }
    if (doDesktop) {
      await invoke("connect_claude_desktop")
        .then(() => (desktopOpened = true))
        .catch((e) => errors.push(t(`Claude 데스크톱: ${String(e)}`, `Claude desktop: ${String(e)}`)));
    }
    await invoke<ConnectStatus>("get_connect_status").then(setStatus).catch(() => {});
    setResult({ errors, codeOk, desktopOpened });
    setBusy(false);
    onSettled();
  };

  return (
    <div className="space-y-3">
      <p>
        {t(
          <>
            한 번 연결해 두면 <b className={em}>다시 누를 일이 없어요.</b> Claude를 켤 때마다 알아서
            붙어요.
          </>,
          <>
            Connect once and <b className={em}>you never have to again.</b> Claude attaches every
            time you open it.
          </>,
        )}
      </p>

      <div className="divide-y divide-[var(--color-ivory-300)] dark:divide-[var(--color-night-700)]">
        {codeShown && (
          <StepRow
            title="Claude Code"
            desc={t("어느 폴더에서 claude를 켜도 붙어요", "Attaches in any folder you run claude in")}
            done={codeDone || !!result?.codeOk}
            checked={pickCode}
            onToggle={() => setPickCode((v) => !v)}
          />
        )}
        {desktopShown && (
          <StepRow
            title={t("Claude 데스크톱", "Claude desktop")}
            desc={t("Claude에 설치 창이 떠요 — '설치'를 누르세요", "Claude shows an installer — press Install")}
            done={desktopDone || !!result?.desktopOpened}
            doneLabel={desktopDone ? undefined : t("설치 창 열림", "Installer open")}
            hint={desktopOff ? t("Claude 설정에서 켜 주세요", "Turn it on in Claude") : undefined}
            checked={pickDesktop}
            onToggle={() => setPickDesktop((v) => !v)}
          />
        )}
        <StepRow
          title={t("로그인할 때 Kura 켜기", "Open Kura at login")}
          desc={t("메뉴 막대에 조용히 떠 있어요", "Sits quietly in the menu bar")}
          done={autostartOn}
          checked={pickAutostart}
          onToggle={() => setPickAutostart((v) => !v)}
        />
      </div>

      {!codeShown && !desktopShown && (
        <p className="text-[12px] text-[var(--color-ink-300)]">
          {tempNoPath
            ? t(
                "Kura를 응용 프로그램 폴더로 옮겨서 열면 연결할 수 있어요.",
                "Move Kura to Applications and open it from there to connect.",
              )
            : t(
                "이 맥에서 Claude를 찾지 못했어요. 설치한 뒤 메인 화면의 연결 배지를 누르면 돼요.",
                "Couldn't find Claude on this Mac. Install it, then tap the connect badge on the main screen.",
              )}
        </p>
      )}

      {result ? (
        <div className="space-y-1.5 pt-1">
          {result.errors.length === 0 ? (
            <p className="flex items-center justify-center gap-1.5 text-[12px] text-[var(--color-accent)]">
              <Check size={13} />
              {result.desktopOpened
                ? t("됐어요. Claude 창에서 '설치'만 누르면 끝이에요.", "Done. Press Install in Claude and you're set.")
                : t("연결했어요.", "Connected.")}
            </p>
          ) : (
            <>
              {result.errors.map((e) => (
                <p key={e} className="text-[11px] leading-relaxed text-red-500/90 break-words">
                  {e}
                </p>
              ))}
              <p className="text-[11px] text-[var(--color-ink-300)]">
                {t(
                  "나머지는 메인 화면의 연결 배지에서 다시 할 수 있어요.",
                  "You can finish from the connect badge on the main screen.",
                )}
              </p>
            </>
          )}
        </div>
      ) : (
        !nothingToDo && (
          <button
            type="button"
            onClick={() => void run()}
            disabled={busy}
            className={cn(primaryBtn, "w-full")}
          >
            {busy ? <Loader2 size={15} className="animate-spin" /> : t("연결하기", "Connect")}
          </button>
        )
      )}
    </div>
  );
}

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
  // 연결 단계에서 「연결하기」를 눌렀나. 누르기 전엔 아래 버튼이 「나중에」(보조) — 파란 버튼이 둘이면
  // 어느 쪽이 연결인지 흐려진다.
  const [connectSettled, setConnectSettled] = useState(false);
  const page = PAGES[i];
  const deferring = page.kind === "connect" && !connectSettled;
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
            className={cn(cardBase, page.kind === "connect" ? "px-7 py-7" : "px-8 py-10")}
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

            {/* 연결 단계는 줄·버튼이 붙어 다른 페이지보다 길다 — 아이콘을 빼고 여백을 줄여 팝오버(640) 안에 담는다. */}
            {page.kind === "connect" && (
              <div>
                <h1 className="text-center text-[19px] tracking-tight">
                  {t("Claude와 연결할까요?", "Connect Claude?")}
                </h1>
                <div className="mt-4 text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                  <ConnectStep onSettled={() => setConnectSettled(true)} />
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
                      받기로 코인을 채우면 바로 쓸 수 있어요.
                      <br />
                      궁금하면 언제든 헤더의{" "}
                      <b className="text-[var(--color-ink-700)] dark:text-[#E8E5DD]">ⓘ 도움말</b>을
                      누르면 돼요.
                    </>,
                    <>
                      Top up from Receive and you're ready to go.
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
            className={cn(deferring ? cn(secondaryBtn, "h-11") : primaryBtn, "flex-1")}
          >
            {deferring ? (
              t("나중에", "Later")
            ) : last ? (
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
