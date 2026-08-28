// OS 알림 (Session 15) — 자율 결제(비번 없이 승인)를 보호자에게 사후 통지한다.
// 수동 승인(비번 입력)은 사람이 이미 봤으므로 알림 대상이 아니다.

/// macOS 알림. tauri-plugin-notification(notify-rust)은 옛 NSUserNotificationCenter 를 쓰는데
/// 최신 macOS(26에서 확인)가 이를 조용히 버린다 — show()가 Ok 인데 화면엔 안 뜸.
/// 동작이 확인된 osascript 경로를 쓴다. (장기적으론 UNUserNotificationCenter + 서명 번들이 정석)
use crate::i18n::{tf, ts};

pub(crate) fn show_notification(title: &str, body: &str) {
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        applescript_escape(body),
        applescript_escape(title)
    );
    match std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .spawn()
    {
        // 끝나길 기다리는 스레드 — 결제 경로를 안 막으면서 좀비 프로세스도 안 남긴다.
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => eprintln!("자율 결제 알림 실패: {e}"),
    }
}

/// AppleScript 문자열 리터럴 이스케이프. 알림 내용에 외부 입력(x402 리소스 URL)이 들어가므로
/// 따옴표/백슬래시를 막아 스크립트 주입을 차단한다.
fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// 자율 승인 알림 문구 (title, body). 순수 함수 — 테스트용 분리.
///
/// `hide_amount`(개발 46, 프라이버시): macOS 알림은 잠금 화면에도 뜨고 화면 공유·녹화에도
/// 잡힌다. 켜면 제목에서 금액을 뺀다("자율 결제"만). 대상(주소·리소스)은 body 에 남긴다 —
/// 알림의 목적이 "돈이 어디로 나갔는지 사후 인지"라, 다 가리면 알림을 끄는 것과 같아진다.
pub(crate) fn auto_pay_notice(
    kind: &str,
    token: &str,
    amount: &str,
    to: &str,
    resource: &str,
    hide_amount: bool,
) -> (String, String) {
    let title = if hide_amount {
        ts!("자율 결제", "Autopay").to_string()
    } else {
        tf!("자율 결제 {amount} {token}", "Autopay {amount} {token}")
    };
    let body = if kind == "x402" {
        let target = if resource.is_empty() { to } else { resource };
        tf!("x402 서명 · {target}", "x402 signature · {target}")
    } else {
        tf!("송금 · {}", "Transfer · {}", short_addr(to))
    };
    (title, body)
}

/// 주소 축약 표시 ("0x8b7ba5…0161"). 주소는 ASCII hex 라 바이트 슬라이스 안전.
fn short_addr(a: &str) -> String {
    if a.len() > 12 {
        format!("{}…{}", &a[..8], &a[a.len() - 4..])
    } else {
        a.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 자율 승인 알림 문구: x402는 리소스 URL, 송금은 축약 주소. 알림은 비밀(비번·니모닉) 미포함.
    #[test]
    fn auto_pay_notice_formats() {
        let (t, b) = auto_pay_notice(
            "x402",
            "USDC",
            "0.01",
            "0x209693Bc6afc0C5328bA36FaF03C514EF312287C",
            "https://www.x402.org/protected",
            false,
        );
        assert_eq!(t, "자율 결제 0.01 USDC");
        assert_eq!(b, "x402 서명 · https://www.x402.org/protected");

        // x402인데 리소스가 비면 주소로 폴백
        let (_, b) = auto_pay_notice("x402", "USDC", "0.01", "0xABCD", "", false);
        assert_eq!(b, "x402 서명 · 0xABCD");

        let (t, b) = auto_pay_notice(
            "transfer",
            "USDC",
            "1.5",
            "0x8b7ba5077d261739f5FeBB31B10167671e590161",
            "",
            false,
        );
        assert_eq!(t, "자율 결제 1.5 USDC");
        assert_eq!(b, "송금 · 0x8b7ba5…0161");
    }

    // 금액 숨기기(개발 46): 제목에서 금액·토큰이 빠지고, body(대상)는 그대로 남는다.
    // 잠금 화면·화면 공유에 금액이 안 보이게 하는 옵션이라 제목만 본다.
    #[test]
    fn auto_pay_notice_hides_amount() {
        let (t, b) = auto_pay_notice(
            "x402",
            "USDC",
            "0.01",
            "0xABCD",
            "https://www.x402.org/protected",
            true,
        );
        assert_eq!(t, "자율 결제");
        assert!(!t.contains("0.01") && !t.contains("USDC"));
        assert_eq!(b, "x402 서명 · https://www.x402.org/protected");

        let (t, b) = auto_pay_notice(
            "transfer",
            "USDC",
            "1.5",
            "0x8b7ba5077d261739f5FeBB31B10167671e590161",
            "",
            true,
        );
        assert_eq!(t, "자율 결제");
        assert_eq!(b, "송금 · 0x8b7ba5…0161");
    }

    // 알림은 osascript 문자열 리터럴로 들어간다 — 외부 입력(리소스 URL)의 따옴표/백슬래시가
    // 스크립트를 깨거나 주입하지 못하게 이스케이프되는지.
    #[test]
    fn applescript_escape_blocks_injection() {
        assert_eq!(
            applescript_escape(r#"a" & (do shell script "rm") & ""#),
            r#"a\" & (do shell script \"rm\") & \""#
        );
        assert_eq!(applescript_escape(r"back\slash"), r"back\\slash");
        assert_eq!(applescript_escape("평범한 문장"), "평범한 문장");
    }
}
