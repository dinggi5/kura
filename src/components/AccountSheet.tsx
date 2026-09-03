// 계정 시트 (개발 54) — 잔액 카드 머리의 계정 이름을 누르면 뜬다. 전환·추가·이름 바꾸기.
//
// 시드는 하나고 계정은 그 시드의 HD 파생이라(m/44'/60'/0'/0/n) 백업은 여전히 12단어 하나다.
// 추가에만 비번이 든다(주소를 얻으려면 시드를 풀어야 한다). 전환·이름은 비번 없이.
// 승인 대기 중인 결제가 있으면 백엔드가 전환·추가를 거절한다 — 요청이 그 순간의 계정으로
// 각인돼 있어서, 바꾼 뒤 승인하면 어차피 거절되기 때문(「화면 계정 ≠ 돈 나가는 계정」 차단).

import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { motion } from "framer-motion";
import { Check, Loader2, Pencil, Plus } from "lucide-react";
import { cn } from "@/lib/cn";
import { accountName, shortenAddress } from "@/lib/format";
import type { Account, WalletStatus } from "@/lib/types";
import { inputBase, modalCard, modalOverlay, primaryBtn, secondaryBtn, CloseButton, PwInput } from "@/components/ui";
import { t } from "@/lib/i18n";

export function AccountSheet({
  accounts,
  active,
  onChange,
  onClose,
}: {
  accounts: Account[];
  active: number;
  /** 백엔드가 돌려준 새 지갑 상태를 그대로 올린다(활성 주소가 바뀌면 화면이 따라간다). */
  onChange: (status: WalletStatus) => void;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // 추가: 버튼을 누르면 비번 칸이 펼쳐진다(시트를 하나 더 띄우지 않는다).
  const [adding, setAdding] = useState(false);
  const [pw, setPw] = useState("");
  // 이름 바꾸기: 한 번에 한 줄만 편집.
  const [editing, setEditing] = useState<number | null>(null);
  const [draft, setDraft] = useState("");

  async function run(cmd: string, args: Record<string, unknown>, close = false) {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const s = await invoke<WalletStatus>(cmd, args);
      onChange(s);
      if (close) onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const switchTo = (index: number) => {
    if (index === active) {
      onClose();
      return;
    }
    void run("switch_account", { index }, true);
  };

  const add = async () => {
    if (!pw) return;
    await run("add_account", { password: pw }, true);
    setPw(""); // 성공이든 실패든 비번은 입력칸 수명에만 둔다
  };

  const startEdit = (a: Account) => {
    setEditing(a.index);
    setDraft(a.label);
  };
  const saveEdit = () => {
    if (editing === null) return;
    const index = editing;
    setEditing(null);
    void run("rename_account", { index, label: draft });
  };

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.2 }}
      className={modalOverlay}
      onClick={onClose}
    >
      <motion.section
        initial={{ scale: 0.95, opacity: 0, y: 12 }}
        animate={{ scale: 1, opacity: 1, y: 0 }}
        exit={{ scale: 0.97, opacity: 0 }}
        transition={{ type: "spring", stiffness: 340, damping: 26 }}
        className={cn(modalCard, "px-6 py-6")}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label={t("계정", "Accounts")}
      >
        <CloseButton onClose={onClose} />
        <p className="text-[11px] tracking-[0.04em] text-[var(--color-ink-500)]">{t("계정", "Accounts")}</p>

        <ul className="mt-4 flex flex-col">
          {accounts.map((a) => {
            const isActive = a.index === active;
            const isEditing = editing === a.index;
            return (
              <li
                key={a.index}
                className={cn(
                  "flex items-center gap-3 -mx-2 px-2 py-2.5 rounded-[10px]",
                  "border-b border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] last:border-b-0",
                )}
              >
                {isEditing ? (
                  <input
                    autoFocus
                    value={draft}
                    maxLength={24}
                    placeholder={t(`계정 ${a.index + 1}`, `Account ${a.index + 1}`)}
                    onChange={(e) => setDraft(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") saveEdit();
                      if (e.key === "Escape") setEditing(null);
                    }}
                    onBlur={saveEdit}
                    className={cn(inputBase, "h-9 text-[13px] flex-1 min-w-0")}
                  />
                ) : (
                  <button
                    type="button"
                    onClick={() => switchTo(a.index)}
                    disabled={busy}
                    className="flex-1 min-w-0 flex items-center gap-3 text-left disabled:opacity-50"
                  >
                    <span
                      className={cn(
                        "shrink-0 w-4 flex justify-center",
                        isActive ? "text-[var(--color-accent)]" : "text-transparent",
                      )}
                      aria-hidden
                    >
                      <Check size={14} />
                    </span>
                    <span className="flex-1 min-w-0">
                      <span
                        className={cn(
                          "block truncate text-[13px] tracking-tight",
                          isActive
                            ? "text-[var(--color-ink-900)] dark:text-[#E8E5DD]"
                            : "text-[var(--color-ink-700)] dark:text-[#B5AFA2]",
                        )}
                      >
                        {accountName(a)}
                      </span>
                      <span className="block text-[11px] font-mono text-[var(--color-ink-300)]" title={a.address}>
                        {shortenAddress(a.address)}
                      </span>
                    </span>
                  </button>
                )}
                {!isEditing && (
                  <button
                    type="button"
                    onClick={() => startEdit(a)}
                    disabled={busy}
                    aria-label={t("이름 바꾸기", "Rename")}
                    className={cn(
                      "shrink-0 w-7 h-7 inline-flex items-center justify-center rounded-full",
                      "text-[var(--color-ink-300)] hover:text-[var(--color-ink-700)]",
                      "hover:bg-[var(--color-ivory-200)] dark:hover:bg-[var(--color-night-700)]",
                      "transition-colors duration-[var(--duration-base)] disabled:opacity-40",
                    )}
                  >
                    <Pencil size={12} />
                  </button>
                )}
              </li>
            );
          })}
        </ul>

        {adding ? (
          <div className="mt-4">
            <p className="text-[11px] leading-relaxed text-[var(--color-ink-300)]">
              {t(
                "같은 12단어에서 다음 계정을 만들어요. 백업은 지금 그대로예요 — 그 12단어로 이 계정도 다시 나와요.",
                "Makes the next account from the same 12 words. Your backup stays the same — those words bring this account back too.",
              )}
            </p>
            <div className="mt-3">
              <PwInput
                value={pw}
                onChange={setPw}
                placeholder={t("비밀번호로 계정 추가", "Password to add an account")}
                autoFocus
                onEnter={add}
              />
            </div>
            <div className="mt-3 grid grid-cols-[1fr_1.8fr] gap-2">
              <button
                type="button"
                onClick={() => {
                  setAdding(false);
                  setPw("");
                  setError(null);
                }}
                disabled={busy}
                className={cn(secondaryBtn, "w-full")}
              >
                {t("취소", "Cancel")}
              </button>
              <button type="button" onClick={add} disabled={!pw || busy} className={primaryBtn}>
                {busy ? <Loader2 size={15} className="animate-spin" /> : <Plus size={15} />}
                {t("계정 추가", "Add account")}
              </button>
            </div>
          </div>
        ) : (
          <button
            type="button"
            onClick={() => {
              setAdding(true);
              setError(null);
            }}
            disabled={busy}
            className={cn(secondaryBtn, "mt-4 w-full")}
          >
            <Plus size={13} />
            {t("새 계정", "New account")}
          </button>
        )}

        {error && <p className="mt-3 text-[12px] text-red-500 font-mono break-all">{error}</p>}
      </motion.section>
    </motion.div>
  );
}
