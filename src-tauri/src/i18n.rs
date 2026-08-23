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

/// 시작할 때 한 번 — 설정에 고른 언어가 있으면 그것, 없으면 시스템 언어.
pub(crate) fn init() {
    let chosen = crate::settings::read_settings().lang;
    set(match chosen.as_deref() {
        Some(c) if !c.trim().is_empty() => parse(c),
        _ => detect_system(),
    });
}

/// 현재 언어의 문구를 고른다 (리터럴 그대로 — 포맷 인자가 없을 때).
///
/// ```ignore
/// ts!("보내기", "Send")
/// ```
macro_rules! ts {
    ($ko:expr, $en:expr $(,)?) => {
        match $crate::i18n::lang() {
            $crate::i18n::Lang::Ko => $ko,
            $crate::i18n::Lang::En => $en,
        }
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

    // 매크로가 현재 언어를 따라간다. 전역을 만지므로 테스트 하나에 몰아넣는다
    // (러스트 테스트는 같은 프로세스에서 병렬 실행 — 언어를 바꾸는 테스트가 둘이면 서로를 깨뜨린다).
    #[test]
    fn macros_follow_current_lang() {
        set(Lang::Ko);
        assert_eq!(ts!("보내기", "Send"), "보내기");
        assert_eq!(tf!("{}개", "{} items", 3), "3개");
        set(Lang::En);
        assert_eq!(ts!("보내기", "Send"), "Send");
        assert_eq!(tf!("{}개", "{} items", 3), "3 items");
        set(Lang::Ko); // 뒷정리 — 다른 테스트의 기본 기대치가 한국어다
    }
}
