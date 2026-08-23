// 언어 (개발 42) — 한국어/영어 두 벌. GUI 가 사람에게 보여주는 모든 문자열이 여기를 지난다.
//
// **왜 키(key) 사전이 아니라 문구 두 개를 나란히 두는가.**
// 이 앱의 문자열은 2 언어뿐이고, 대부분 "이 자리에서 한 번" 쓰인다. 키 사전을 두면
// 부르는 자리에서는 `t("send.confirm")` 만 보여서, 돈이 오가는 코드를 읽을 때 화면에
// 실제로 뭐라고 뜨는지 알 수 없다. 두 문구를 call site 에 붙여 두면 번역 빠짐이
// **문법 오류**가 되고(둘 다 안 쓰면 컴파일이 안 된다), 리뷰어가 코드와 문구를 한 화면에서 본다.
//
// MCP 사이드카(kura-mcp)는 이 모듈을 쓰지 않는다 — 그쪽 독자는 사람이 아니라 모델이라
// 영어 한 벌로 통일했다(kura-mcp/src/lib.rs 주석 참고).

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Lang {
    Ko,
    En,
}

const KO: u8 = 0;
const EN: u8 = 1;

/// 현재 언어. 원자값 하나 — 읽는 쪽이 결제 경로·트레이·알림이라 잠금을 안 쓴다.
///
/// **게으른 초기화(OnceLock 등)를 쓰지 않는 이유**: `init()` 이 설정을 읽는데, 그 경로의
/// 함수들이 에러 문구에 ts! 를 쓴다 → 게으른 초기화면 자기 초기화 중에 자신을 다시 부른다
/// (사이드카에서 실제로 이 재진입으로 프로세스가 멈췄다 — kura-mcp/src/i18n.rs 참고).
/// 원자값은 언제 읽어도 답이 있다: 정해지기 전이면 기본값(한국어)이고, 그 구간에 나오는
/// 문구는 사용자에게 보이기 전에 버려진다.
static CURRENT: AtomicU8 = AtomicU8::new(KO);

pub(crate) fn lang() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == EN {
        Lang::En
    } else {
        Lang::Ko
    }
}

pub(crate) fn set(l: Lang) {
    CURRENT.store(if l == Lang::En { EN } else { KO }, Ordering::Relaxed);
}

/// 설정 문자열 → 언어. 모르는 값·빈 값은 한국어(기존 사용자의 동작 보존).
pub(crate) fn parse(code: &str) -> Lang {
    if code.trim().to_ascii_lowercase().starts_with("en") {
        Lang::En
    } else {
        Lang::Ko
    }
}

/// 언어 → 설정·프론트에 넘기는 코드.
pub(crate) fn code(l: Lang) -> &'static str {
    match l {
        Lang::Ko => "ko",
        Lang::En => "en",
    }
}

/// macOS 시스템 언어. 설정에 언어가 **아직 없을 때만**(=한 번도 안 고른 사용자) 쓴다.
///
/// `AppleLocale`(ko_KR) 이 아니라 `AppleLanguages`(선호 언어 **순서**)를 본다 — 지역은
/// 한국인데 언어는 영어로 쓰는 사람이 있고, 그 사람이 원하는 건 영어다.
/// 못 읽으면 한국어 — 이 앱의 기존 사용자가 전부 한국어라, 모를 때의 안전한 쪽이다.
fn detect_system() -> Lang {
    let out = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            // 출력은 plist 배열: (\n    "en-US",\n    "ko-KR"\n)
            match text.split('"').nth(1) {
                Some(first) if first.to_ascii_lowercase().starts_with("ko") => Lang::Ko,
                Some(_) => Lang::En,
                None => Lang::Ko,
            }
        }
        _ => Lang::Ko,
    }
}

/// 시작할 때 한 번 — 언어를 정하고, 아직 고른 적이 없으면 **정한 값을 설정에 못박는다**.
///
/// 고른 적 없는 사용자를 둘로 가른다 (코덱스 개발 42 P2):
///   · **지갑이 이미 있다** = 이 패치 전부터 쓰던 사람. 그동안 한국어로 써 왔으니 한국어다.
///     여기서 시스템 언어를 따르면, 영어로 맞춰 둔 맥을 쓰는 기존 사용자의 지갑이
///     업데이트 한 번에 통째로 영어로 바뀐다.
///   · **지갑도 없다** = 진짜 첫 실행 → 시스템 언어.
/// (설정 파일 유무가 아니라 지갑 유무로 가르는 건 개발 39 의 체인 기본값과 같은 판단이다 —
///  설정 파일은 autostart::reconcile 이 첫 실행에 만들어 버려서 기준이 못 된다.)
///
/// **못박는 이유**: 안 박으면 신규 사용자가 지갑을 만든 다음 실행부터 "지갑 있음"으로 분류돼
/// 언어가 영어 → 한국어로 뒤집힌다. 저장에 실패해도 이번 실행은 정한 값으로 간다.
pub(crate) fn init() {
    if let Some(code) = crate::settings::read_settings().lang {
        if !code.trim().is_empty() {
            set(parse(&code));
            return;
        }
    }
    let chosen = if crate::settings::wallet_exists() {
        Lang::Ko
    } else {
        detect_system()
    };
    set(chosen);
    if let Some(mut s) = crate::settings::read_settings_for_update() {
        s.lang = Some(code(chosen).to_string());
        let _ = crate::settings::save_settings(&s);
    }
}

/// 언어에 따라 둘 중 하나를 고른다 — 매크로가 실제로 하는 일 전부.
///
/// 매크로에서 이 함수로 한 겹 뺀 이유는 **테스트 때문**이다(코덱스 개발 42 P2).
/// 전역 언어를 바꿔 가며 매크로를 검증하면, 같은 프로세스에서 병렬로 도는 다른 테스트
/// (한국어 문구를 값으로 비교하는 notify·settings 테스트)가 그 사이에 영어를 보고 깨진다.
/// 고르는 규칙은 여기서 값으로 검증하고, 전역은 아무도 흔들지 않는다.
pub(crate) fn pick<T>(l: Lang, ko: T, en: T) -> T {
    match l {
        Lang::Ko => ko,
        Lang::En => en,
    }
}

/// 현재 언어의 문구를 고른다 (리터럴 그대로 — 포맷 인자가 없을 때).
///
/// ```ignore
/// ts!("보내기", "Send")
/// ```
macro_rules! ts {
    ($ko:expr, $en:expr $(,)?) => {
        $crate::i18n::pick($crate::i18n::lang(), $ko, $en)
    };
}

/// 현재 언어의 문구를 `format!` 한다 (포맷 인자가 있을 때).
///
/// ```ignore
/// tf!("{}개 남음", "{} left", n)
/// ```
/// 인자는 두 언어가 **같은 값**을 받는다 — 어순은 `{0}`·`{name}` 으로 각자 바꾼다.
macro_rules! tf {
    ($ko:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        match $crate::i18n::lang() {
            $crate::i18n::Lang::Ko => format!($ko $(, $arg)*),
            $crate::i18n::Lang::En => format!($en $(, $arg)*),
        }
    };
}

pub(crate) use {tf, ts};

#[cfg(test)]
mod tests {
    use super::*;

    // 언어 코드 해석: 지역 꼬리표가 붙어도(en-US) 영어, 모르는 값은 한국어(기존 동작 보존).
    #[test]
    fn parse_codes() {
        assert_eq!(parse("en"), Lang::En);
        assert_eq!(parse("en-US"), Lang::En);
        assert_eq!(parse("EN"), Lang::En);
        assert_eq!(parse("ko"), Lang::Ko);
        assert_eq!(parse("ko-KR"), Lang::Ko);
        assert_eq!(parse(""), Lang::Ko);
        assert_eq!(parse("fr"), Lang::Ko);
        assert_eq!(code(Lang::En), "en");
        assert_eq!(code(Lang::Ko), "ko");
    }

    // 고르는 규칙 자체는 값으로 검증한다 — 전역을 건드리지 않아 병렬 테스트에 안전하다.
    #[test]
    fn pick_follows_lang() {
        assert_eq!(pick(Lang::Ko, "보내기", "Send"), "보내기");
        assert_eq!(pick(Lang::En, "보내기", "Send"), "Send");
        assert_eq!(pick(Lang::Ko, 1, 2), 1);
    }

    // 기본값은 한국어 — 기존 사용자의 동작이자, 언어를 못 정했을 때의 안전한 쪽.
    #[test]
    fn default_is_korean() {
        assert_eq!(lang(), Lang::Ko);
    }
}
