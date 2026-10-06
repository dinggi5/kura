// 두 창이 같은 상태를 보게 한다 (개발 75).
//
// 정본은 백엔드(~/.jigap)라 두 창이 읽는 값은 원래 같다 — 어긋나는 건 **한 창에서 바꾼 뒤 다른 창이
// 아직 다시 읽지 않은 동안**뿐이다. 그 틈을 둘로 메운다:
//   ① 바꾼 쪽이 `notifySync()` 로 알린다 → 모든 창(자기 포함)이 다시 읽는다. 뒤에 떠 있는 큰 창도 맞는다.
//   ② 창이 앞으로 돌아오면(포커스·보이기) 다시 읽는다 — ①을 빠뜨린 변경도 사람이 보는 순간엔 맞는다.
// 다시 읽기는 파일 몇 개와 잔액 RPC 하나라 싸다. 잔액은 부르는 쪽이 스로틀한다.

import { useEffect, useRef } from "react";
import { emit, listen } from "@tauri-apps/api/event";

const EVENT = "kura-sync";

/** 상태를 바꿨다고 모든 창에 알린다. 실패해도 조용히 — ②가 남는다. */
export function notifySync(): void {
  emit(EVENT).catch(() => {});
}

/** 다른 창(또는 이 창)이 상태를 바꿨을 때와, 이 창이 앞으로 돌아왔을 때 `reload` 를 부른다. */
export function useSync(reload: () => void): void {
  // 부르는 쪽이 매 렌더 새 함수를 넘겨도 구독을 다시 걸지 않게 ref 로 든다.
  const ref = useRef(reload);
  ref.current = reload;
  useEffect(() => {
    const run = () => ref.current();
    const off = listen(EVENT, run);
    const onReturn = () => {
      if (!document.hidden) run();
    };
    window.addEventListener("focus", onReturn);
    document.addEventListener("visibilitychange", onReturn);
    return () => {
      void off.then((f) => f());
      window.removeEventListener("focus", onReturn);
      document.removeEventListener("visibilitychange", onReturn);
    };
  }, []);
}
