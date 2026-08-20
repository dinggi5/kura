// AI 연결 (개발 35) — Claude 데스크톱/Claude Code 연결을 앱 밖(릴리스 페이지·터미널)이
// 아니라 앱 안에서 끝낸다.
//
//   Claude 데스크톱: 앱 Resources 에 동봉한 확장(kura.mcpb)을 열면 설치 다이얼로그가 뜬다.
//   Claude Code:     claude CLI 를 찾아 `mcp add --scope user` 등록을 대행한다.
//
// 여기의 "감지"는 전부 이 맥의 파일 상태를 읽는 것뿐이다(네트워크 0). 파일이 있다고
// 연결이 살아 있다는 뜻은 아니므로, 연결 여부의 최종 진실은 ipc::get_agent_status
// (메인 화면 배지)다 — 이 화면은 그 옆에서 "연결까지 남은 단계"를 보여주는 역할.

use serde::Serialize;
use std::path::PathBuf;
use std::process::Command;

/// Claude 데스크톱 번들 ID — /Applications/Claude.app 에서 실측한 공개 값.
/// `open -b` 로 이 앱을 콕 집어 열어야, .mcpb 기본 핸들러가 다른 앱으로 바뀐
/// 환경에서도 설치 다이얼로그가 Claude 에 뜬다.
const CLAUDE_DESKTOP_BUNDLE_ID: &str = "com.anthropic.claudefordesktop";

/// AI 연결 화면이 그리는 상태 한 벌. 전부 로컬 파일 감지 결과다.
#[derive(Serialize)]
pub(crate) struct ConnectStatus {
    /// Claude 데스크톱 앱이 설치돼 있는지 (/Applications · ~/Applications).
    pub(crate) desktop_installed: bool,
    /// Claude 데스크톱에 Kura 확장이 설치돼 있는지 — 확장 폴더의 manifest 를 읽는
    /// 최선 노력 감지. 폴더 구조가 바뀌면 false 로 남을 수 있다(연결 배지가 보완).
    pub(crate) desktop_ext_installed: bool,
    /// 찾아낸 claude CLI 절대경로. 없으면 None → 프론트가 수동 명령 복사로 안내.
    pub(crate) cli_path: Option<String>,
    /// ~/.claude.json 사용자 범위(mcpServers)에 kura 가 등록돼 있고, 그 command 가
    /// **이 빌드의 kura-mcp 경로와 일치**하는지. 이름만 있고 경로가 다르면 false 가
    /// 아니라 cli_registered_other 로 따로 알린다 — "등록됨"이라 속이지도,
    /// 멀쩡한 다른 등록을 없는 셈 치지도 않게.
    pub(crate) cli_registered: bool,
    /// kura 항목은 있는데 command 가 이 빌드 경로와 다른 경우(옛 설치·다른 사본).
    pub(crate) cli_registered_other: bool,
    /// 이 앱 번들 안 kura-mcp 절대경로 — 수동 등록 명령 조립용.
    pub(crate) mcp_path: Option<String>,
}

/// 이 앱 번들 안의 kura-mcp 사이드카 절대경로. 실행 파일 옆에 있다
/// (릴리스 = Kura.app/Contents/MacOS/, dev = target/debug/).
/// /Applications 를 하드코딩하지 않는 이유: 앱을 ~/Applications 등에 둔 사용자도
/// 자기 번들의 바이너리를 정확히 등록하게.
fn bundled_mcp_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let p = exe.parent()?.join("kura-mcp");
    p.is_file().then_some(p)
}

/// Claude 데스크톱 설치 여부. 표준 폴더 둘을 먼저 보고(비용 0), 없을 때만 Spotlight 로
/// 번들 ID 를 찾는다(코덱스 개발35 2차 — 다른 폴더·볼륨에 설치해도 `open -b` 는 여는데
/// 감지만 "미설치"라 하면 연결 버튼이 사라진다). mcpb 런처와 같은 패턴: /usr/bin 절대경로.
fn desktop_installed() -> bool {
    if PathBuf::from("/Applications/Claude.app").is_dir()
        || dirs::home_dir().is_some_and(|h| h.join("Applications/Claude.app").is_dir())
    {
        return true;
    }
    Command::new("/usr/bin/mdfind")
        .arg(format!(
            "kMDItemCFBundleIdentifier == '{CLAUDE_DESKTOP_BUNDLE_ID}'"
        ))
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.trim_ascii().is_empty())
}

/// 확장 manifest JSON 이 우리 확장(kura)인가 — 감지 로직의 순수 심장부(테스트 대상).
fn manifest_is_kura(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(|n| n == "kura"))
        .unwrap_or(false)
}

/// Claude 데스크톱 확장 폴더에서 kura 확장을 찾는다. 폴더가 없으면(확장을 하나도
/// 안 깔았으면 안 생긴다) 그냥 false.
fn desktop_ext_installed() -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    let dir = home.join("Library/Application Support/Claude/Claude Extensions");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        std::fs::read_to_string(e.path().join("manifest.json"))
            .map(|s| manifest_is_kura(&s))
            .unwrap_or(false)
    })
}

/// claude CLI 를 찾는다. GUI 앱의 PATH 는 셸과 달라(로그인 셸 rc 를 안 읽는다)
/// `which` 에 기댈 수 없으므로, 알려진 설치 위치를 먼저 보고 PATH 는 보조로 본다.
fn find_claude_cli() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local/bin/claude")); // 공식 네이티브 설치
        candidates.push(home.join(".claude/local/claude")); // 구 로컬 설치
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin/claude"));
    candidates.push(PathBuf::from("/usr/local/bin/claude"));
    if let Ok(path) = std::env::var("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join("claude")));
    }
    // is_file 은 심링크를 따라간다 — 공식 설치가 심링크라 이게 맞다.
    candidates.into_iter().find(|p| p.is_file())
}

/// ~/.claude.json 사용자 범위에 등록된 kura 서버의 command — 순수 심장부(테스트 대상).
/// 프로젝트 범위(.mcp.json 등)는 안 본다: 우리가 대행하는 등록이 사용자 범위고,
/// 프로젝트 범위는 그 폴더에서만 살아서 "연결됨"이라 말하면 거짓말이 되는 경우가 많다.
///
/// 이름만 보고 "등록됨"이라 하지 않는 이유(코덱스 개발35 1차): 옛 설치·옮긴 앱을
/// 가리키는 kura 항목이 있으면 화면은 "등록됨"인데 Claude 는 서버를 못 띄운다.
/// command 까지 돌려줘서 호출부가 현재 경로와 대조하게 한다.
fn claude_json_kura_command(json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()?
        .get("mcpServers")?
        .get("kura")?
        .get("command")?
        .as_str()
        .map(|s| s.to_string())
}

fn registered_cli_command() -> Option<String> {
    dirs::home_dir()
        .and_then(|h| std::fs::read_to_string(h.join(".claude.json")).ok())
        .and_then(|s| claude_json_kura_command(&s))
}

/// AI 연결 화면 상태 한 벌. 전부 로컬 파일 읽기라 폴링해도 싸다.
#[tauri::command]
pub(crate) fn get_connect_status() -> ConnectStatus {
    let mcp_path = bundled_mcp_path().map(|p| p.to_string_lossy().into_owned());
    let registered_cmd = registered_cli_command();
    let matches = match (&registered_cmd, &mcp_path) {
        (Some(cmd), Some(ours)) => cmd == ours,
        _ => false,
    };
    ConnectStatus {
        desktop_installed: desktop_installed(),
        desktop_ext_installed: desktop_ext_installed(),
        cli_path: find_claude_cli().map(|p| p.to_string_lossy().into_owned()),
        cli_registered: matches,
        cli_registered_other: registered_cmd.is_some() && !matches,
        mcp_path,
    }
}

/// 동봉한 확장(kura.mcpb)을 Claude 데스크톱으로 연다 → 설치 다이얼로그 직행.
/// 설치 승인 자체는 Claude 데스크톱 UI 에서 사람이 누른다(우리는 문만 두드린다).
#[tauri::command]
pub(crate) fn connect_claude_desktop(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::path::BaseDirectory;
    use tauri::Manager;
    if !desktop_installed() {
        return Err("Claude 데스크톱이 설치돼 있지 않아요. claude.ai/download 에서 먼저 설치하세요.".into());
    }
    let mcpb = app
        .path()
        .resolve("kura.mcpb", BaseDirectory::Resource)
        .map_err(|e| format!("동봉된 확장을 찾지 못했어요: {e}"))?;
    if !mcpb.is_file() {
        return Err("동봉된 확장(kura.mcpb)이 이 빌드에 없어요. 앱을 다시 설치해 보세요.".into());
    }
    // /usr/bin/open 절대경로 — PATH 의 가짜 open 을 타지 않게 (mcpb 런처와 같은 원칙).
    let out = Command::new("/usr/bin/open")
        .args(["-b", CLAUDE_DESKTOP_BUNDLE_ID])
        .arg(&mcpb)
        .output()
        .map_err(|e| format!("Claude 데스크톱을 열지 못했어요: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "Claude 데스크톱을 열지 못했어요: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// claude CLI 에 사용자 범위 MCP 등록을 대행한다:
///   claude mcp add --scope user kura -- <이 번들의 kura-mcp>
/// --scope user 인 이유: 기본(local)은 "지금 폴더"에만 살아서, GUI 앱이 대신 등록하면
/// 앱의 cwd 라는 아무 데도 아닌 곳에 등록된다. 지갑은 어느 폴더에서든 쓰는 도구다.
#[tauri::command]
pub(crate) fn connect_claude_code() -> Result<(), String> {
    let Some(cli) = find_claude_cli() else {
        return Err("claude 명령을 찾지 못했어요. 아래 수동 등록 명령을 터미널에서 실행하세요.".into());
    };
    let Some(mcp) = bundled_mcp_path() else {
        return Err("이 빌드에 kura-mcp 가 없어요. 앱을 다시 설치해 보세요.".into());
    };
    // 멱등 재등록: 같은 이름이 이미 있으면 add 가 "already exists" 로 거부하는데,
    // 그걸 성공으로 치면 옛 경로를 가리키는 등록이 영영 안 고쳐진다(코덱스 개발35 1차).
    // 우리 이름(kura)의 사용자 범위 항목만 지우고 다시 등록한다 — 없어서 실패하는 건 정상.
    let _ = Command::new(&cli)
        .args(["mcp", "remove", "--scope", "user", "kura"])
        .output();
    let out = Command::new(&cli)
        .args(["mcp", "add", "--scope", "user", "kura", "--"])
        .arg(&mcp)
        .output()
        .map_err(|e| format!("claude 실행 실패: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "등록 실패: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 확장 감지: manifest 의 name 이 정확히 "kura" 일 때만 우리 것으로 본다.
    #[test]
    fn manifest_detection() {
        assert!(manifest_is_kura(r#"{"name":"kura","version":"0.1.2"}"#));
        assert!(!manifest_is_kura(r#"{"name":"kura-fake"}"#));
        assert!(!manifest_is_kura(r#"{"version":"1.0"}"#)); // name 없음
        assert!(!manifest_is_kura("not json"));
    }

    // CLI 등록 감지: 사용자 범위 mcpServers 의 command 를 돌려준다(이름만 보지 않는다 —
    // 옛 경로를 가리키는 등록을 "등록됨"으로 속이지 않기 위해. 코덱스 개발35 1차).
    #[test]
    fn claude_json_detection() {
        assert_eq!(
            claude_json_kura_command(r#"{"mcpServers":{"kura":{"command":"/a/kura-mcp"}}}"#),
            Some("/a/kura-mcp".to_string())
        );
        assert_eq!(
            claude_json_kura_command(r#"{"mcpServers":{"playwright":{}}}"#),
            None
        );
        // command 없는 항목(HTTP 서버 등)은 경로 대조가 불가 → None.
        assert_eq!(claude_json_kura_command(r#"{"mcpServers":{"kura":{}}}"#), None);
        // 프로젝트 범위에만 있는 건 등록으로 안 친다.
        assert_eq!(
            claude_json_kura_command(r#"{"projects":{"/a":{"mcpServers":{"kura":{"command":"x"}}}}}"#),
            None
        );
        assert_eq!(claude_json_kura_command("{}"), None);
        assert_eq!(claude_json_kura_command("broken"), None);
    }
}
