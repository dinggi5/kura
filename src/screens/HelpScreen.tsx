// 도움말 화면 — 헤더 ⓘ 버튼에서 언제든 열람. 섹션 카드 스택(스크롤형 레퍼런스).
// 콘텐츠는 helpContent.tsx 한 곳에서 환영 투어(WelcomeTour)와 공유한다.

import { openUrl } from "@tauri-apps/plugin-opener";
import { motion } from "framer-motion";
import { ExternalLink, HelpCircle } from "lucide-react";
import { cn } from "@/lib/cn";
import { GITHUB_URL, HELP_SECTIONS } from "@/lib/helpContent";
import { cardBase, shell } from "@/components/ui";
import { t } from "@/lib/i18n";

export function HelpScreen({ onClose }: { onClose: () => void }) {
  return (
    <main className={cn(shell, "justify-start gap-4")}>
      <header className="w-full max-w-md flex items-center justify-between text-[12px] text-[var(--color-ink-500)]">
        <span className="flex items-center gap-2">
          <HelpCircle size={12} className="text-[var(--color-accent)]" />
          {t("도움말", "Help")}
        </span>
        <button
          type="button"
          onClick={onClose}
          className="hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD] transition-colors"
        >
          {t("닫기", "Close")}
        </button>
      </header>

      <div className="w-full max-w-md flex flex-col gap-3 pb-4">
        {HELP_SECTIONS.map((s, i) => (
          <motion.section
            key={s.id}
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.3, delay: i * 0.04, ease: [0.4, 0, 0.2, 1] }}
            className={cn(cardBase, "px-6 py-5")}
          >
            <div className="flex items-start gap-3.5">
              <div className="shrink-0 w-10 h-10 rounded-full flex items-center justify-center bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-700)] text-[var(--color-accent)]">
                {s.icon}
              </div>
              <div className="flex-1 min-w-0 pt-0.5">
                <h2 className="text-[15px] tracking-tight text-[var(--color-ink-900)] dark:text-[#E8E5DD]">
                  {s.title}
                </h2>
                <div className="mt-2 text-[12.5px] leading-relaxed text-[var(--color-ink-500)]">
                  {s.body}
                </div>
              </div>
            </div>
          </motion.section>
        ))}

        <button
          type="button"
          onClick={() => openUrl(GITHUB_URL).catch(() => {})}
          className="mt-1 inline-flex items-center justify-center gap-1.5 text-[12px] text-[var(--color-ink-500)] hover:text-[var(--color-accent)] transition-colors"
        >
          <ExternalLink size={12} />
          {t("GitHub에서 전체 문서 보기", "Read the full docs on GitHub")}
        </button>
      </div>
    </main>
  );
}
