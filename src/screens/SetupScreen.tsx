// 비번 설정 / 마이그레이션 / 복구 문구 가져오기 — 첫 진입 화면.
// - none + create : 새 지갑 생성 (생성 직후 시드 백업 플로우)
// - none + import : 다른 지갑의 12~24단어를 가져와 비번으로 암호화 (이미 시드 보유 → 백업 생략)
// - legacy        : 평문 wallet.json 을 비번으로 암호화(마이그레이션)

import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { motion } from "framer-motion";
import { Loader2, Lock, ShieldCheck, KeyRound, Download } from "lucide-react";
import { cn } from "@/lib/cn";
import type { WalletInfo, WalletStatus } from "@/lib/types";
import { markWelcomePending } from "@/lib/welcome";
import { cardBase, enter, primaryBtn, shell, FieldHint, FlowIcon, PwInput } from "@/components/ui";
import { BackupFlow } from "@/screens/BackupFlow";
import { t } from "@/lib/i18n";

// BIP-39 표준 길이. 백엔드(import_wallet)와 같은 집합으로 버튼 활성을 가볍게 게이팅한다.
const VALID_WORD_COUNTS = [12, 15, 18, 21, 24];

export function SetupScreen({
  status,
  onDone,
}: {
  status: WalletStatus;
  onDone: (address: string, backedUp: boolean) => void;
}) {
  const isLegacy = status.state === "legacy";
  const [mode, setMode] = useState<"create" | "import">("create");
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");
  const [phrase, setPhrase] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // 새 지갑 생성 직후 → 시드 백업 플로우. (비번을 들고 있어 바로 열람 가능)
  const [created, setCreated] = useState<{ address: string; pw: string } | null>(null);

  const isImport = !isLegacy && mode === "import";

  const tooShort = pw.length > 0 && pw.length < 8;
  const mismatch = pw2.length > 0 && pw !== pw2;
  const pwOk = pw.length >= 8 && pw === pw2;

  // 단어 수만 가볍게 본다 — 체크섬·단어 유효성은 백엔드가 최종 판정한다.
  const wordCount = phrase.trim().split(/\s+/).filter(Boolean).length;
  const phraseOk = VALID_WORD_COUNTS.includes(wordCount);

  const canSubmit = pwOk && !busy; // create / migrate
  const canImport = pwOk && phraseOk && !busy;

  // create(none) 와 migrate(legacy) 공용.
  async function submit() {
    if (!canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      const cmd = isLegacy ? "migrate_wallet" : "create_wallet";
      const w = await invoke<WalletInfo>(cmd, { password: pw });
      if (isLegacy) {
        // 마이그레이션은 기존 지갑 — 백업은 경고 배너로 유도한다(투어 없음).
        onDone(w.address, false);
      } else {
        // 생성 성공(=wallet.enc 존재) 즉시 투어를 예약한다. 백업 플로우 도중 앱이
        // 꺼져도 다음 실행에 투어가 다시 뜨도록(영구 스킵 방지). 완료/건너뛰기 시 소거.
        markWelcomePending(w.address);
        setCreated({ address: w.address, pw });
      }
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  // 복구 문구 가져오기. 성공 = 기존 지갑이고 사용자가 시드를 직접 입력했으니
  // 백업 플로우·투어 없이 바로 진입한다(backed_up=true).
  async function submitImport() {
    if (!canImport) return;
    setBusy(true);
    setError(null);
    try {
      const w = await invoke<WalletInfo>("import_wallet", { password: pw, phrase });
      // 메모리 위생: 화면 전환 전에 민감 상태(문구·비번)를 비운다(완벽한 제로화는 불가하나
      // 문구만 비우고 비번은 남기던 비대칭 제거). create 경로는 BackupFlow 가 비번을 써야 해서
      // 의도적으로 유지하므로 여기(import)만 정리한다.
      setPhrase("");
      setPw("");
      setPw2("");
      onDone(w.address, true);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  function switchMode(next: "create" | "import") {
    setMode(next);
    setError(null);
    setPhrase(""); // 모드 전환 시 입력 중인 문구는 남기지 않는다.
  }

  if (created) {
    return (
      <BackupFlow
        mode="onboard"
        initialPassword={created.pw}
        onComplete={() => onDone(created.address, true)}
        onExit={() => onDone(created.address, false)}
      />
    );
  }

  const title = isImport
    ? t("복구 문구로 가져오기", "Import a recovery phrase")
    : isLegacy
      ? t("지갑을 비밀번호로 보호하기", "Protect your wallet with a password")
      : t("새 지갑 만들기", "Create a new wallet");

  return (
    <main className={shell}>
      <header className="w-full max-w-md flex items-center gap-2 text-[12px] text-[var(--color-ink-500)]">
        <span className="inline-block w-1.5 h-1.5 rounded-full bg-[var(--color-accent)]" aria-hidden />
        Kura
      </header>

      <motion.section {...enter} className={cn(cardBase, "max-w-md px-8 py-10")}>
        <FlowIcon>{isImport ? <KeyRound size={22} /> : <ShieldCheck size={22} />}</FlowIcon>

        <h1 className="mt-5 text-center text-[19px] tracking-tight">{title}</h1>

        <p className="mt-2 text-center text-[13px] leading-relaxed text-[var(--color-ink-500)]">
          {isImport ? (
            t(
              <>
                다른 지갑의 복구 문구(12~24단어)를 입력하면
                <br />그 지갑을 Kura로 가져와요. 새 비밀번호로 이 기기에 암호화돼요.
              </>,
              <>
                Enter another wallet's recovery phrase (12–24 words)
                <br />to bring that wallet into Kura. It's encrypted on this Mac with a new password.
              </>,
            )
          ) : isLegacy ? (
            t(
              <>
                지금은 키가 잠금 없이 저장돼 있어요.
                <br />
                비밀번호로 암호화하면 이 비번이 있어야만 송금할 수 있어요.
              </>,
              <>
                Right now your key is stored unlocked.
                <br />
                Encrypt it with a password and nothing can be sent without that password.
              </>,
            )
          ) : (
            t(
              <>
                송금할 때마다 입력할 비밀번호를 정하세요.
                <br />이 비번으로 키가 암호화돼 저장돼요.
              </>,
              <>
                Choose the password you'll type for every payment.
                <br />Your key is stored encrypted with it.
              </>,
            )
          )}
        </p>

        {isImport && (
          <div className="mt-6">
            <textarea
              value={phrase}
              onChange={(e) => setPhrase(e.target.value)}
              rows={3}
              spellCheck={false}
              autoCapitalize="none"
              autoCorrect="off"
              placeholder={t(
                "복구 문구를 공백으로 구분해 입력하세요",
                "Type your recovery phrase, words separated by spaces",
              )}
              className={cn(
                "w-full px-3.5 py-3 rounded-[var(--radius-card)] resize-none",
                "bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]",
                "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
                "text-[var(--color-ink-900)] dark:text-[#E8E5DD]",
                "placeholder:text-[var(--color-ink-300)]",
                "outline-none transition-colors duration-[var(--duration-base)]",
                "focus:border-[var(--color-accent)]",
                "text-[14px] leading-relaxed font-mono tracking-wide",
              )}
            />
            <div className="mt-1.5 flex items-center justify-between text-[11px]">
              <span className="text-[var(--color-ink-300)]">
                {t("주변에 보는 사람이 없는 곳에서 입력하세요.", "Type this where nobody can see your screen.")}
              </span>
              {wordCount > 0 && (
                <span
                  className={cn(
                    "tabular-nums",
                    phraseOk ? "text-[var(--color-accent)]" : "text-[var(--color-ink-300)]",
                  )}
                >
                  {t(`단어 ${wordCount}개`, `${wordCount} words`)}
                </span>
              )}
            </div>
          </div>
        )}

        {!isImport && !isLegacy && (
          <p className="mt-3 text-center text-[11px] leading-relaxed text-[var(--color-ink-300)]">
            {t(
              "다음 단계에서 복구용 12단어를 백업해요. 비밀번호를 잊으면 이 12단어로만 되찾을 수 있어요 — Kura 가져오기나 다른 표준 지갑(BIP-39)에서.",
              "Next you'll back up your 12 recovery words. If you forget your password, those words are the only way back — through Import here, or any standard BIP-39 wallet.",
            )}
          </p>
        )}

        <div className="mt-6 space-y-3">
          <PwInput
            value={pw}
            onChange={setPw}
            placeholder={
              isImport
                ? t("새 비밀번호 (8자 이상)", "New password (8 characters or more)")
                : t("비밀번호 (8자 이상)", "Password (8 characters or more)")
            }
            onEnter={() => document.getElementById("pw2")?.focus()}
          />
          {tooShort && <FieldHint>{t("8자 이상으로 정해주세요.", "Use at least 8 characters.")}</FieldHint>}
          <PwInput
            id="pw2"
            value={pw2}
            onChange={setPw2}
            placeholder={t("비밀번호 확인", "Confirm password")}
            onEnter={isImport ? submitImport : submit}
          />
          {mismatch && <FieldHint>{t("비밀번호가 서로 달라요.", "These don't match.")}</FieldHint>}
        </div>

        {error && (
          <p className="mt-4 text-[12px] text-red-500 font-mono break-all">{error}</p>
        )}

        <button
          type="button"
          onClick={isImport ? submitImport : submit}
          disabled={isImport ? !canImport : !canSubmit}
          className={cn(primaryBtn, "mt-6")}
        >
          {busy ? (
            <Loader2 size={15} className="animate-spin" />
          ) : isImport ? (
            <Download size={15} />
          ) : (
            <Lock size={15} />
          )}
          {isImport
            ? t("가져오기", "Import")
            : isLegacy
              ? t("비밀번호로 보호", "Protect with password")
              : t("지갑 만들기", "Create wallet")}
        </button>

        {!isLegacy && (
          <div className="mt-5 pt-5 border-t border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] text-center">
            {isImport ? (
              <button
                type="button"
                onClick={() => switchMode("create")}
                disabled={busy}
                className="text-[12px] text-[var(--color-ink-500)] hover:text-[var(--color-ink-900)] transition-colors disabled:opacity-50"
              >
                {t("← 새 지갑 만들기로 돌아가기", "← Back to creating a new wallet")}
              </button>
            ) : (
              <button
                type="button"
                onClick={() => switchMode("import")}
                disabled={busy}
                className="text-[12px] text-[var(--color-ink-500)] hover:text-[var(--color-ink-900)] transition-colors disabled:opacity-50"
              >
                {t("이미 복구 문구가 있나요?", "Already have a recovery phrase?")}{" "}
                <span className="text-[var(--color-accent)]">{t("가져오기", "Import it")}</span>
              </button>
            )}
          </div>
        )}
      </motion.section>

      <div />
    </main>
  );
}
