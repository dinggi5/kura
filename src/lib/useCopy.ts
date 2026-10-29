import { useCallback, useState } from "react";

/** 클립보드 복사 + 잠깐 "복사됨" 표시. 잔액·받기·백업 카드가 공유한다. */
export function useCopy(): [boolean, (text: string) => void] {
  const [copied, setCopied] = useState(false);
  const copy = useCallback((text: string) => {
    // 「복사됨」은 실제로 들어갔을 때만(개발 81, 코덱스 1차 P2) — 실패했는데 성공으로 보이면 클립보드의 옛 주소를 붙여 넣는다.
    navigator.clipboard
      ?.writeText(text)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1400);
      })
      .catch(() => {});
  }, []);
  return [copied, copy];
}
