// 화면 언어 (개발 42) — 한국어/영어 두 벌.
//
// **왜 키 사전이 아니라 문구 두 개를 나란히 두는가.** 2 언어뿐이고 대부분의 문구가
// 한 자리에서만 쓰인다. `t("send.title")` 로 감추면 화면에 뭐라고 뜨는지 보려고 사전 파일을
// 왕복해야 하고, 번역 빠짐은 런타임에야 드러난다. `t("보내기", "Send")` 는 두 언어를
// 코드 옆에 붙여 둬서 **빠뜨리면 타입 오류**이고, 리뷰어가 한 화면에서 둘 다 본다.
//
// **언어를 바꾸면 창을 다시 읽는다(reload).** 렌더 트리 전체를 구독시키는 대신 모듈 전역
// 하나를 쓰는 대가다 — 대신 화면 어디서나(훅 밖·상수·헬퍼) 같은 함수를 쓸 수 있고,
// "일부만 옛 언어로 남는" 상태가 원리적으로 안 생긴다. 언어 변경은 평생 한두 번이다.

import { invoke } from "@tauri-apps/api/core";

export type Lang = "ko" | "en";

/** 백엔드 왕복 없이 첫 프레임을 맞는 언어로 그리기 위한 거울. 진실은 settings.json 이다. */
const CACHE_KEY = "kura.lang";

function cached(): Lang | null {
  try {
    const v = localStorage.getItem(CACHE_KEY);
    return v === "ko" || v === "en" ? v : null;
  } catch {
    return null;
  }
}

/** 캐시에 적었는가. **실패를 삼키면 안 된다** — 리로드로 언어를 적용하는 구조라,
 *  저장이 안 되는 환경에서 그대로 리로드하면 같은 불일치를 매번 다시 만나 무한 리로드가 된다
 *  (코덱스 개발 42 2차 P2). 그래서 성공 여부를 돌려주고, 실패하면 URL 로 나른다. */
function remember(l: Lang): boolean {
  try {
    localStorage.setItem(CACHE_KEY, l);
    return true;
  } catch {
    return false;
  }
}

/** 리로드를 건너뛰어도 살아남는 두 번째 통로. 저장이 막힌 환경에서만 쓴다. */
function fromUrl(): Lang | null {
  try {
    const v = new URLSearchParams(location.search).get("lang");
    return v === "ko" || v === "en" ? v : null;
  } catch {
    return null;
  }
}

/** WKWebView 의 `navigator.language` 는 macOS 선호 언어를 그대로 반영한다(백엔드의
 *  `defaults read -g AppleLanguages` 와 같은 값). 처음 켠 순간에도 물어볼 곳이 있다는 뜻. */
function detect(): Lang {
  return typeof navigator !== "undefined" && navigator.language?.toLowerCase().startsWith("en")
    ? "en"
    : "ko";
}

/**
 * 🔴 **모듈이 로드되는 시점에 확정된다.** 상수 안에서도 `t()` 를 쓸 수 있어야 하는데
 * (예: chain.ts 의 체인 표시 이름), 그 상수들은 첫 렌더보다 먼저 굳는다. 그래서 언어는
 * 비동기 왕복을 기다리지 않고 **동기적으로** 정한다: 캐시 → 시스템 언어.
 * 백엔드 설정과 어긋나는 드문 경우는 `initLang()` 이 뒤늦게 바로잡는다.
 */
let current: Lang = fromUrl() ?? cached() ?? detect();

export function lang(): Lang {
  return current;
}

/** 현재 언어의 문구를 고른다. 문자열·JSX·배열 무엇이든 같은 자리에서 두 벌로 둔다. */
export function t<T>(ko: T, en: T): T {
  return current === "en" ? en : ko;
}

/** `toLocaleDateString` 등에 넘길 로케일. */
export function locale(): string {
  return current === "en" ? "en-US" : "ko-KR";
}

/**
 * 백엔드(settings.json)와 대조한다. 어긋나면 캐시를 고치고 **한 번** 다시 읽는다.
 *
 * 어긋나는 경우는 드물다 — 캐시가 지워졌는데 설정에는 시스템과 다른 언어를 골라 둔 때뿐.
 * 다시 읽은 뒤에는 캐시=백엔드라 두 번 반복되지 않는다. 실패하면 그냥 지금 언어로 간다.
 */
export async function initLang(): Promise<void> {
  document.documentElement.lang = current;
  let backend: Lang;
  try {
    backend = (await invoke<string>("get_lang")) === "en" ? "en" : "ko";
  } catch {
    return; // 언어 때문에 앱이 안 뜨는 일은 없어야 한다
  }
  if (backend === current) {
    remember(current);
    return;
  }
  if (remember(backend)) {
    window.location.reload();
    return;
  }
  // 저장이 막힌 환경 — URL 에 실어 한 번만 다시 읽는다. 이미 실어 온 길이면 그만두고
  // 이번 세션은 메모리 값으로 간다(이미 그려진 상수는 옛 언어로 남지만, 무한 리로드보다 낫다).
  if (fromUrl()) {
    current = backend;
    document.documentElement.lang = current;
    return;
  }
  location.search = `?lang=${backend}`;
}

/**
 * 언어를 고른다. 저장에 성공했을 때만 창을 다시 읽는다 —
 * 저장이 실패했는데 화면만 바뀌면 다음 실행에 조용히 되돌아간다(그게 더 나쁘다).
 */
export async function chooseLang(next: Lang): Promise<void> {
  if (next === current) return;
  await invoke("set_lang", { lang: next });
  if (remember(next)) {
    window.location.reload();
  } else {
    // 캐시가 막혔으면 URL 이 그 자리를 대신한다(이것도 리로드를 일으킨다).
    location.search = `?lang=${next}`;
  }
}
