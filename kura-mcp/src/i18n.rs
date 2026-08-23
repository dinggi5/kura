// 언어 (개발 42) — **사람이 읽는 문자열만** 두 벌로 둔다.
//
// 이 크레이트에는 독자가 둘이다:
//   · MCP 서버(main.rs) — 읽는 쪽이 모델이다. `#[tool(description = …)]` 이 컴파일 타임
//     리터럴이라 런타임 전환도 안 되고, 무엇보다 모델은 영어로 읽고 사용자 언어로 답한다.
//     → **영어 한 벌.**
//   · CLI(bin/kura.rs) — 읽는 쪽이 사람이다. → **GUI 와 같은 언어를 따라간다.**
//
// 언어는 GUI 와 같은 `~/.jigap/settings.json` 의 `lang` 을 읽어 정한다(체인을 그렇게 맞추는
// 것과 같은 결 — 두 프로세스가 공유 파일 하나로 자동 일치한다). 값이 없으면 시스템 언어.

use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Ko,
    En,
}

/// 프로세스 수명 동안 한 번만 정한다 — CLI 는 한 번 실행에 한 화면이라 중간에 바뀔 일이 없다.
static LANG: OnceLock<Lang> = OnceLock::new();

pub fn lang() -> Lang {
    *LANG.get_or_init(resolve)
}

fn parse(code: &str) -> Lang {
    if code.trim().to_ascii_lowercase().starts_with("en") {
        Lang::En
    } else {
        Lang::Ko
    }
}

fn resolve() -> Lang {
    // 테스트·스크립트가 사용자의 라이브 설정에 의존하지 않게 환경변수를 먼저 본다(체인과 같은 규칙).
    if let Ok(v) = std::env::var("KURA_LANG") {
        if !v.trim().is_empty() {
            return parse(&v);
        }
    }
    #[derive(serde::Deserialize)]
    struct LangSel {
        #[serde(default)]
        lang: Option<String>,
    }
    let chosen = crate::wallet::jigap_dir()
        .ok()
        .and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok())
        .and_then(|t| serde_json::from_str::<LangSel>(&t).ok())
        .and_then(|s| s.lang);
    match chosen.as_deref() {
        Some(c) if !c.trim().is_empty() => parse(c),
        // 아직 안 고른 사용자 — GUI(i18n::detect_system)와 같은 곳을 본다.
        _ => detect_system(),
    }
}

/// macOS 선호 언어. 못 읽으면 한국어(기존 사용자 전원이 한국어라, 모를 때의 안전한 쪽).
fn detect_system() -> Lang {
    let out = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            match text.split('"').nth(1) {
                Some(first) if !first.to_ascii_lowercase().starts_with("ko") => Lang::En,
                _ => Lang::Ko,
            }
        }
        _ => Lang::Ko,
    }
}

/// 현재 언어의 문구 (포맷 인자 없음).
#[macro_export]
macro_rules! ts {
    ($ko:expr, $en:expr $(,)?) => {
        match $crate::i18n::lang() {
            $crate::i18n::Lang::Ko => $ko,
            $crate::i18n::Lang::En => $en,
        }
    };
}

/// 현재 언어의 문구를 `format!` 한다.
#[macro_export]
macro_rules! tf {
    ($ko:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        match $crate::i18n::lang() {
            $crate::i18n::Lang::Ko => format!($ko $(, $arg)*),
            $crate::i18n::Lang::En => format!($en $(, $arg)*),
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_codes() {
        assert_eq!(parse("en"), Lang::En);
        assert_eq!(parse("en-GB"), Lang::En);
        assert_eq!(parse("ko-KR"), Lang::Ko);
        assert_eq!(parse(""), Lang::Ko);
        assert_eq!(parse("ja"), Lang::Ko);
    }
}
