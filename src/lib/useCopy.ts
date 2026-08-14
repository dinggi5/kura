import { useCallback, useState } from "react";

/** 클립보드 복사 + 잠깐 "복사됨" 표시. 잔액·받기·백업 카드가 공유한다. */
export function useCopy(): [boolean, (text: string) => void] {
  const [copied, setCopied] = useState(false);
  const copy = useCallback((text: string) => {
    navigator.clipboard?.writeText(text).catch(() => {});
    setCopied(true);
    setTimeout(() => setCopied(false), 1400);
  }, []);
  return [copied, copy];
}
