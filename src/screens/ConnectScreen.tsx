// AI 연결 화면 (개발 35) — 지금까지 앱 밖(릴리스 페이지의 .mcpb, 터미널의 claude mcp add)에
// 있던 연결을 앱 안으로 들인다: 설치 → 지갑 생성 → 연결이 이 창 안에서 끝난다.
//
// 감지(설치·등록)는 3초 폴링 — 전부 로컬 파일 읽기라 싸고, 사용자가 Claude 쪽에서
// '설치'를 누르고 돌아오면 화면이 스스로 따라잡는다. 연결의 최종 진실은 상단의
// 연결 배지(AgentStatus·1초 폴링)다.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { motion } from "framer-motion";
import {
  Bot,
  Check,
  Copy,
  ExternalLink,
  Loader2,
  MessageSquare,
  Plug,
  Terminal,
} from "lucide-react";
import { cn } from "@/lib/cn";
import { GITHUB_URL } from "@/lib/helpContent";
import { useCopy } from "@/lib/useCopy";
import type { AgentStatus, ConnectStatus } from "@/lib/types";
import { cardBase, enter, primaryBtn, secondaryBtn, shell } from "@/components/ui";
import { t } from "@/lib/i18n";

const em = "text-[var(--color-ink-700)] dark:text-[#E8E5DD]";

/** 명령·경로를 보여주는 모노 박스 (helpContent 의 코드 박스와 같은 결). */
function CodeBox({ text }: { text: string }) {
  return (
    <div className="px-3 py-2 rounded-[10px] bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)] border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] font-mono text-[11px] leading-relaxed text-[var(--color-ink-900)] dark:text-[#E8E5DD] select-all break-all">
      {text}
    </div>
  );
}

/** 카드 우상단 상태 라벨. */
function StateTag({ ok, label }: { ok?: boolean; label: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 text-[11px] tracking-tight",
        ok ? "text-[var(--color-accent)]" : "text-[var(--color-ink-300)]",
      )}
    >
      {ok && <Check size={11} />}
      {label}
    </span>
  );
}

function ClientCard({
  icon,
  title,
  tag,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  tag?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className={cn(cardBase, "px-5 py-4")}>
      <div className="flex items-center justify-between">
        <h2 className="flex items-center gap-2 text-[13px] tracking-tight text-[var(--color-ink-700)] dark:text-[#E8E5DD]">
          <span className="text-[var(--color-ink-300)]">{icon}</span>
          {title}
        </h2>
        {tag}
      </div>
      <div className="mt-3 text-[12px] leading-relaxed text-[var(--color-ink-500)]">{children}</div>
    </section>
  );
}

export function ConnectScreen({ agent, onClose }: { agent: AgentStatus; onClose: () => void }) {
  const [status, setStatus] = useState<ConnectStatus | null>(null);
  const [copied, copy] = useCopy();
  const [pathCopied, copyPath] = useCopy();
  // Claude 데스크톱: 열기 시도 결과 — 성공하면 "설치 창에서 '설치'를 누르라"는 다음 단계 안내.
  const [desktopBusy, setDesktopBusy] = useState(false);
  const [desktopMsg, setDesktopMsg] = useState<{ ok: boolean; text: string } | null>(null);
  // Claude Code: 등록 대행 결과.
  const [cliBusy, setCliBusy] = useState(false);
  const [cliError, setCliError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    invoke<ConnectStatus>("get_connect_status").then(setStatus).catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    const h = setInterval(refresh, 3000);
    return () => clearInterval(h);
  }, [refresh]);

  const connectDesktop = async () => {
    setDesktopBusy(true);
    setDesktopMsg(null);
    try {
      await invoke("connect_claude_desktop");
      setDesktopMsg({
        ok: true,
        text: t(
          "Claude 데스크톱에 설치 창이 떴어요 — 거기서 '설치'를 누르면 끝나요.",
          "Claude desktop opened its installer — press Install there and you're done.",
        ),
      });
    } catch (e) {
      setDesktopMsg({ ok: false, text: String(e) });
    } finally {
      setDesktopBusy(false);
    }
  };

  const connectCode = async () => {
    setCliBusy(true);
    setCliError(null);
    try {
      await invoke("connect_claude_code");
      refresh(); // 등록 직후 상태 태그가 바로 "등록됨"으로 바뀌게.
    } catch (e) {
      setCliError(String(e));
    } finally {
      setCliBusy(false);
    }
  };

  // 수동 등록 명령. mcp_path 는 이 빌드의 실제 사이드카 경로 — dev 에서도 맞는 걸 보여준다.
  // 셸 인용(코덱스 개발35 2차): 앱이 공백 든 경로에 있으면 인용 없는 복사 명령이 경로를
  // 여러 인자로 쪼개 command 가 반토막 난 채 등록된다. 작은따옴표로 감싸고 안의 '는 탈출.
  const mcpPath = status?.mcp_path ?? "/Applications/Kura.app/Contents/MacOS/kura-mcp";
  const shellQuoted = `'${mcpPath.replace(/'/g, `'\\''`)}'`;
  const cliCommand = `claude mcp add --scope user kura -- ${shellQuoted}`;
  // 등록 대행이 실패했을 때의 수동 명령. 실패 시 옛 등록을 원복해 두므로(백엔드) kura
  // 항목이 남아 있을 수 있고, bare add 는 "already exists" 로 거부된다(코덱스 개발38 1차)
  // — 지우고 다시 등록하는 시퀀스로 안내한다(항목이 없으면 remove 만 실패하고 add 는 된다).
  const reRegisterCommand = `claude mcp remove --scope user kura; ${cliCommand}`;
  // 임시 위치(디스크 이미지·Translocation) 실행 + 설치본 없음 — 등록할 유효 경로 자체가
  // 없다. 이 상태에선 모든 클라이언트 카드가 경로 대신 "옮기기" 안내를 보여준다.
  const tempNoPath = !!status?.temp_location && !status.mcp_path;

  return (
    <main className={shell}>
      <header className="w-full max-w-md flex items-center justify-between text-[12px] text-[var(--color-ink-500)]">
        <span className="flex items-center gap-2">
          <Plug size={12} className="text-[var(--color-accent)]" />
          {t("AI 연결", "Connect an AI")}
        </span>
        <button
          type="button"
          onClick={onClose}
          className="hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD] transition-colors"
        >
          {t("닫기", "Close")}
        </button>
      </header>

      <motion.div {...enter} className="w-full max-w-md flex flex-col gap-3">
        {/* 지금 연결 상태 — 최종 진실(1초 폴링 배지와 같은 소스) */}
        <section className={cn(cardBase, "px-5 py-4")}>
          {agent.connected ? (
            <div className="flex items-center gap-2.5">
              <span className="relative flex w-2 h-2 shrink-0" aria-hidden>
                <span className="absolute inline-flex w-full h-full rounded-full bg-[var(--color-accent)] opacity-60 animate-ping" />
                <span className="relative inline-flex w-2 h-2 rounded-full bg-[var(--color-accent)]" />
              </span>
              <p className="text-[13px] text-[var(--color-ink-700)] dark:text-[#E8E5DD]">
                {t(
                  <>
                    <b className={em}>연결돼 있어요.</b> AI가 결제를 요청하면 승인 팝업이 떠요.
                  </>,
                  <>
                    <b className={em}>Connected.</b> When the AI asks to pay, an approval window
                    opens.
                  </>,
                )}
              </p>
            </div>
          ) : (
            <p className="text-[12px] leading-relaxed text-[var(--color-ink-500)]">
              {t(
                <>
                  <b className={em}>아직 연결된 AI가 없어요.</b> 아래에서 쓰는 앱을 골라 연결하면,
                  AI가 잔액을 읽고 결제를 <b className={em}>요청</b>할 수 있어요. 승인은 언제나 이
                  앱에서 해요.
                </>,
                <>
                  <b className={em}>No AI is connected yet.</b> Pick the app you use below, and it
                  can read your balance and <b className={em}>ask</b> to pay. Approving always
                  happens here.
                </>,
              )}
            </p>
          )}
        </section>

        {/* Claude 데스크톱 — 동봉 확장(.mcpb) 열기 = 설치 다이얼로그 직행 */}
        <ClientCard
          icon={<MessageSquare size={14} />}
          title={t("Claude 데스크톱", "Claude desktop")}
          tag={
            status &&
            (status.desktop_ext_installed ? (
              <StateTag ok label={t("확장 설치됨", "Extension installed")} />
            ) : status.desktop_installed ? undefined : (
              <StateTag label={t("앱 미설치", "App not installed")} />
            ))
          }
        >
          {status && !status.desktop_installed ? (
            <div className="space-y-2.5">
              <p>
                {t(
                  "Claude 데스크톱 앱이 이 맥에 없어요. 먼저 설치하고 다시 오세요.",
                  "The Claude desktop app isn't on this Mac. Install it first, then come back.",
                )}
              </p>
              <button
                type="button"
                onClick={() => openUrl("https://claude.ai/download").catch(() => {})}
                className={cn(secondaryBtn, "w-full")}
              >
                <ExternalLink size={13} /> {t("claude.ai/download 열기", "Open claude.ai/download")}
              </button>
            </div>
          ) : (
            <div className="space-y-2.5">
              <p>
                {t(
                  "버튼을 누르면 Claude에 확장 설치 창이 떠요. 거기서 '설치'만 누르면 돼요.",
                  "Press the button and Claude opens its extension installer. Press Install there.",
                )}
              </p>
              <button
                type="button"
                onClick={() => void connectDesktop()}
                disabled={desktopBusy || !status}
                className={cn(
                  status?.desktop_ext_installed ? secondaryBtn : primaryBtn,
                  "w-full",
                )}
              >
                {desktopBusy ? (
                  <Loader2 size={14} className="animate-spin" />
                ) : status?.desktop_ext_installed ? (
                  t("확장 다시 설치", "Reinstall the extension")
                ) : (
                  t("Claude 데스크톱에 연결", "Connect Claude desktop")
                )}
              </button>
              {desktopMsg && (
                <p
                  className={cn(
                    "text-[11px] leading-relaxed",
                    desktopMsg.ok ? "text-[var(--color-accent)]" : "text-red-500/90",
                  )}
                >
                  {desktopMsg.text}
                </p>
              )}
            </div>
          )}
        </ClientCard>

        {/* Claude Code — claude CLI 등록 대행 (없으면 명령 복사 폴백) */}
        <ClientCard
          icon={<Terminal size={14} />}
          title="Claude Code"
          tag={
            status &&
            (status.cli_registered ? (
              <StateTag ok label={t("등록됨", "Registered")} />
            ) : status.cli_registered_other ? (
              <StateTag label={t("다른 경로 등록됨", "Points elsewhere")} />
            ) : status.cli_path ? undefined : (
              <StateTag label={t("CLI 못 찾음", "CLI not found")} />
            ))
          }
        >
          {status?.cli_registered ? (
            <p>
              {t(
                <>
                  등록돼 있어요. 아무 폴더에서나 <b className={em}>claude</b>를 실행하면 자동으로
                  연결돼요.
                </>,
                <>
                  Registered. Run <b className={em}>claude</b> in any folder and it connects on its
                  own.
                </>,
              )}
            </p>
          ) : tempNoPath ? (
            // 임시 경로를 등록해 주면 마운트 해제 순간 죽는 등록이 남는다.
            <p>
              {t(
                <>
                  앱이 임시 위치(디스크 이미지 등)에서 실행 중이에요. Kura를{" "}
                  <b className={em}>응용 프로그램</b> 폴더로 옮겨서 실행한 뒤 다시 연결하세요.
                </>,
                <>
                  The app is running from a temporary location (a disk image, for example). Move
                  Kura to <b className={em}>Applications</b>, open it from there, and connect again.
                </>,
              )}
            </p>
          ) : status?.cli_path ? (
            <div className="space-y-2.5">
              <p>
                {status.cli_registered_other
                  ? t(
                      "kura 등록이 있지만 다른 경로(옛 설치 등)를 가리키고 있어요. 다시 등록하면 이 앱으로 바로잡아요.",
                      "A kura entry exists, but it points somewhere else (an older install). Registering again points it at this app.",
                    )
                  : t(
                      "버튼 한 번으로 등록돼요. 다음 claude 실행부터 어느 폴더에서든 연결돼요.",
                      "One button registers it. From the next claude run on, it works in any folder.",
                    )}
              </p>
              <button
                type="button"
                onClick={() => void connectCode()}
                disabled={cliBusy}
                className={cn(primaryBtn, "w-full")}
              >
                {cliBusy ? (
                  <Loader2 size={14} className="animate-spin" />
                ) : status.cli_registered_other ? (
                  t("이 앱으로 다시 등록", "Point it at this app")
                ) : (
                  t("Claude Code에 연결", "Connect Claude Code")
                )}
              </button>
              {cliError && (
                <div className="space-y-2">
                  <p className="text-[11px] leading-relaxed text-red-500/90">{cliError}</p>
                  {/* 대행이 실패한 사람에게 남은 길 = 수동 등록. 에러 문구가 "아래 명령"을
                      가리키므로 여기서 실제로 보여준다. */}
                  <CodeBox text={reRegisterCommand} />
                  <button
                    type="button"
                    onClick={() => copy(reRegisterCommand)}
                    className={cn(secondaryBtn, "w-full")}
                  >
                    {copied ? <Check size={13} /> : <Copy size={13} />} {t("명령 복사", "Copy command")}
                  </button>
                </div>
              )}
            </div>
          ) : (
            <div className="space-y-2.5">
              <p>
                {t(
                  "claude 명령을 찾지 못했어요. Claude Code를 쓰고 있다면 터미널에서 직접 등록하세요:",
                  "Couldn't find the claude command. If you use Claude Code, register it yourself in a terminal:",
                )}
              </p>
              <CodeBox text={cliCommand} />
              <button
                type="button"
                onClick={() => copy(cliCommand)}
                className={cn(secondaryBtn, "w-full")}
              >
                {copied ? <Check size={13} /> : <Copy size={13} />} {t("명령 복사", "Copy command")}
              </button>
            </div>
          )}
        </ClientCard>

        {/* 그 외 MCP 클라이언트 — 경로만 있으면 어디든 */}
        <ClientCard icon={<Bot size={14} />} title={t("다른 AI 앱 (Cursor 등)", "Other AI apps (Cursor and friends)")}>
          {tempNoPath ? (
            // 이 상태의 mcpPath 폴백(/Applications/…)은 아직 없는 파일이다 — 복사하게
            // 두면 죽은 경로를 등록한다(코덱스 개발38 1차). 같은 "옮기기" 안내로 대체.
            <p>
              {t(
                <>
                  앱이 임시 위치(디스크 이미지 등)에서 실행 중이에요. Kura를{" "}
                  <b className={em}>응용 프로그램</b> 폴더로 옮겨서 실행하면 등록할 경로가 생겨요.
                </>,
                <>
                  The app is running from a temporary location (a disk image, for example). Move
                  Kura to <b className={em}>Applications</b> and open it from there to get a path
                  worth registering.
                </>,
              )}
            </p>
          ) : (
          <div className="space-y-2.5">
            <p>
              {t(
                "MCP를 지원하는 앱이면 어디든 붙어요. 서버로 이 경로를 등록하면 돼요:",
                "Any app that speaks MCP can use this. Register this path as the server:",
              )}
            </p>
            <CodeBox text={mcpPath} />
            <div className="flex items-center justify-between gap-2">
              <button
                type="button"
                onClick={() => copyPath(mcpPath)}
                className="inline-flex items-center gap-1 text-[11px] text-[var(--color-ink-500)] hover:text-[var(--color-accent)] transition-colors"
              >
                {pathCopied ? <Check size={11} /> : <Copy size={11} />} {t("경로 복사", "Copy path")}
              </button>
              <button
                type="button"
                onClick={() => openUrl(`${GITHUB_URL}#readme`).catch(() => {})}
                className="inline-flex items-center gap-1 text-[11px] text-[var(--color-ink-500)] hover:text-[var(--color-accent)] transition-colors"
              >
                <ExternalLink size={11} /> {t("GitHub 문서", "GitHub docs")}
              </button>
            </div>
          </div>
          )}
        </ClientCard>
      </motion.div>

      <div />
    </main>
  );
}
