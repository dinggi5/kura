// AI 연결 (개발 35) — Claude 데스크톱/Claude Code 연결을 앱 밖(릴리스 페이지·터미널)이
// 아니라 앱 안에서 끝낸다.
//
//   Claude 데스크톱: 앱 Resources 에 동봉한 확장(kura.mcpb)을 열면 설치 다이얼로그가 뜬다.
//   Claude Code:     claude CLI 를 찾아 `mcp add --scope user` 등록을 대행한다.
//
// 여기의 "감지"는 전부 이 맥의 파일 상태를 읽는 것뿐이다(네트워크 0). 파일이 있다고
// 연결이 살아 있다는 뜻은 아니므로, 연결 여부의 최종 진실은 ipc::get_agent_status
// (메인 화면 배지)다 — 이 화면은 그 옆에서 "연결까지 남은 단계"를 보여주는 역할.

use crate::i18n::{tf, ts};
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
    /// 등록·안내에 쓸 kura-mcp 절대경로 — 수동 등록 명령 조립용. 임시 위치에서 실행
    /// 중이면 설치본(/Applications 등)으로 해석된 경로다.
    pub(crate) mcp_path: Option<String>,
    /// 실행 파일이 임시 위치(App Translocation·DMG 마운트)에서 돌고 있는지.
    /// 이때 설치본마저 없으면 mcp_path 가 None — 프론트가 "응용 프로그램 폴더로
    /// 옮겨 실행" 안내를 띄운다(임시 경로를 등록해 주는 것보다 낫다).
    pub(crate) temp_location: bool,
    /// 지금 실행 중인 이 앱의 버전.
    pub(crate) app_version: String,
    /// 임시 위치에서 실행 중이라 **설치본** 경로를 등록하게 되는데, 그 설치본이 지금
    /// 실행 중인 이 앱과 **다른 버전**일 때 그 버전 문자열 (코덱스 개발38 2차 P2).
    /// 등록될 kura-mcp 는 그 설치본 안의 것이라, 사용자가 보고 있는 화면(이 앱)과
    /// 실제로 AI 에 붙을 바이너리가 갈린다 — 파일 IPC(~/.jigap)가 어긋날 수 있다.
    /// 같거나·임시 실행이 아니거나·버전을 못 읽으면 None.
    pub(crate) installed_version_mismatch: Option<String>,
}

/// 등록 대행 실패를 프론트에 넘기는 모양 (코덱스 개발38 2차 P2).
///
/// 문자열 하나로 넘기던 걸 쪼갠 이유: 실패 화면이 안내하는 수동 명령은
/// `claude mcp remove …; claude mcp add …` 시퀀스다(bare add 는 "already exists" 로
/// 거부되므로). 그런데 대행이 실패한 환경에서는 손으로 쳐도 add 가 또 실패하기 쉽고,
/// 그러면 remove 만 성공해서 **우리가 방금 원복해 둔 옛 등록까지 날아간다.**
/// 되돌릴 명령을 같이 줘야 그 구멍이 막힌다.
#[derive(Serialize)]
pub(crate) struct ConnectError {
    /// 사람이 읽는 실패 사유 (+ 원복 결과 한 줄).
    pub(crate) message: String,
    /// 옛 kura 등록을 그대로 되살리는 `claude mcp add-json …` 명령.
    /// 지우기 전에 옛 항목이 있었을 때만 Some — 없었으면 잃을 게 없다.
    pub(crate) restore_command: Option<String>,
}

/// 셸에 그대로 붙여넣을 수 있게 작은따옴표로 감싼다(안의 ' 는 '\'' 로 탈출).
/// 프론트가 경로에 하는 것과 같은 처리 — 여기서는 JSON 이라 따옴표가 반드시 들어간다.
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// 임시 실행 위치인가 — App Translocation(/private/var/folders/…)과 DMG 마운트.
/// 이 경로를 등록하면 마운트 해제·재실행 순간 죽는 등록이 남는다(코덱스 개발35 3차).
///
/// /Volumes 프리픽스만으로 자르면 외장 SSD·2차 볼륨에 설치한 정상 사용자까지
/// 거부한다(코덱스 개발38 1차) — DMG 는 읽기 전용으로 마운트되고 외장 디스크는
/// 쓰기 가능하니, /Volumes 는 볼륨이 읽기 전용일 때만 임시로 본다.
/// 판정 로직은 순수 심장부(closure 주입, 테스트 대상)로 분리.
fn classify_temp_location(
    p: &std::path::Path,
    volume_is_read_only: impl Fn(&std::path::Path) -> bool,
) -> bool {
    // /var 는 /private/var 의 심링크라 current_exe 가 어느 쪽으로 줄지 몰라 둘 다 본다.
    let s = p.to_string_lossy();
    if s.starts_with("/private/var/folders/") || s.starts_with("/var/folders/") {
        return true;
    }
    s.starts_with("/Volumes/") && volume_is_read_only(p)
}

/// 이 경로가 놓인 파일시스템이 읽기 전용으로 마운트돼 있는가 (statfs MNT_RDONLY).
/// 경로가 없거나 statfs 가 실패하면 false — 판별 불가로 사용자를 막지 않는다.
fn volume_is_read_only(p: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else {
        return false;
    };
    let mut fs: libc::statfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statfs(c.as_ptr(), &mut fs) };
    rc == 0 && (fs.f_flags & libc::MNT_RDONLY as u32) != 0
}

fn path_is_temp_location(p: &std::path::Path) -> bool {
    classify_temp_location(p, volume_is_read_only)
}

/// 설치된 Kura.app 의 kura-mcp — mcpb 런처와 같은 후보 순서(/Applications → ~/Applications).
fn installed_mcp_path() -> Option<PathBuf> {
    let mut apps = vec![PathBuf::from("/Applications/Kura.app")];
    if let Some(h) = dirs::home_dir() {
        apps.push(h.join("Applications/Kura.app"));
    }
    apps.into_iter()
        .map(|a| a.join("Contents/MacOS/kura-mcp"))
        .find(|p| p.is_file())
}

/// 등록·안내에 쓸 kura-mcp 절대경로 + 임시 위치 여부.
///
/// 평소엔 실행 파일 옆의 사이드카다(릴리스 = Kura.app/Contents/MacOS/, dev =
/// target/debug/ — /Applications 하드코딩이 아니라서 ~/Applications 설치도 정확).
/// 임시 위치(Translocation·DMG)에서 실행 중이면 그 옆 경로는 곧 사라지므로
/// 설치본으로 해석하고, 설치본도 없으면 None 을 준다.
fn registerable_mcp_path() -> (Option<PathBuf>, bool) {
    let Ok(exe) = std::env::current_exe() else {
        return (None, false);
    };
    if path_is_temp_location(&exe) {
        return (installed_mcp_path(), true);
    }
    let p = exe.parent().map(|d| d.join("kura-mcp"));
    (p.filter(|p| p.is_file()), false)
}

/// 앱 번들의 CFBundleShortVersionString. `/usr/bin/plutil` 로 읽는다 — Info.plist 가
/// 텍스트든 바이너리든 같은 답을 주고, 절대경로라 PATH 에 가짜를 둔 환경을 안 탄다
/// (mcpb 런처와 같은 원칙). 못 읽으면 None — 버전을 모른다고 사용자를 막지는 않는다.
fn bundle_short_version(app: &std::path::Path) -> Option<String> {
    let out = Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// 임시 실행 → 설치본 등록 조합에서 버전이 갈리는가 (코덱스 개발38 2차 P2).
/// 순수 심장부: 설치본 버전을 읽는 함수를 주입받아 없는 경로로도 테스트한다.
///
/// mcp_path 는 `…/Kura.app/Contents/MacOS/kura-mcp` 라서 조상 셋을 올라가면 번들이다.
/// 임시 실행이 아니면 볼 것도 없다 — 그때 mcp_path 는 지금 이 앱 자신의 사이드카다.
fn classify_installed_mismatch(
    temp_location: bool,
    mcp_path: Option<&std::path::Path>,
    running_version: &str,
    read_version: impl Fn(&std::path::Path) -> Option<String>,
) -> Option<String> {
    if !temp_location {
        return None;
    }
    let app = mcp_path?.parent()?.parent()?.parent()?;
    let installed = read_version(app)?;
    (installed != running_version).then_some(installed)
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
/// ~/.claude.json 사용자 범위의 kura 항목 **전체** — 재등록이 실패했을 때 원복
/// (`claude mcp add-json`)에 쓴다. command 유무와 무관하게 항목이 있으면 준다
/// (command 없는 HTTP 서버 등록도 지웠으면 되돌려야 한다).
fn claude_json_kura_entry(json: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()?
        .get("mcpServers")?
        .get("kura")
        .cloned()
        .filter(|v| !v.is_null())
}

fn claude_json_kura_command(json: &str) -> Option<String> {
    claude_json_kura_entry(json)?
        .get("command")?
        .as_str()
        .map(|s| s.to_string())
}

/// claude CLI 가 실제로 읽는 .claude.json 경로 — CLAUDE_CONFIG_DIR 이 잡혀 있으면
/// 그 아래(코덱스 개발38 1차: 우리만 ~/.claude.json 을 보면 감지·스냅숏이 헛것을 본다).
fn claude_json_path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !dir.trim().is_empty() {
            return Some(PathBuf::from(dir).join(".claude.json"));
        }
    }
    dirs::home_dir().map(|h| h.join(".claude.json"))
}

fn read_claude_json() -> Option<String> {
    std::fs::read_to_string(claude_json_path()?).ok()
}

fn registered_cli_command() -> Option<String> {
    read_claude_json().and_then(|s| claude_json_kura_command(&s))
}

fn registered_cli_entry() -> Option<serde_json::Value> {
    read_claude_json().and_then(|s| claude_json_kura_entry(&s))
}

/// AI 연결 화면 상태 한 벌. 전부 로컬 파일 읽기라 폴링해도 싸다.
#[tauri::command]
pub(crate) fn get_connect_status() -> ConnectStatus {
    let (mcp, temp_location) = registerable_mcp_path();
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let installed_version_mismatch = classify_installed_mismatch(
        temp_location,
        mcp.as_deref(),
        &app_version,
        bundle_short_version,
    );
    let mcp_path = mcp.map(|p| p.to_string_lossy().into_owned());
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
        temp_location,
        app_version,
        installed_version_mismatch,
    }
}

/// 동봉한 확장(kura.mcpb)을 Claude 데스크톱으로 연다 → 설치 다이얼로그 직행.
/// 설치 승인 자체는 Claude 데스크톱 UI 에서 사람이 누른다(우리는 문만 두드린다).
#[tauri::command]
pub(crate) fn connect_claude_desktop(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::path::BaseDirectory;
    use tauri::Manager;
    if !desktop_installed() {
        return Err(ts!(
            "Claude 데스크톱이 설치돼 있지 않아요. claude.ai/download 에서 먼저 설치하세요.",
            "The Claude desktop app isn't installed. Get it from claude.ai/download first."
        )
        .into());
    }
    let mcpb = app
        .path()
        .resolve("kura.mcpb", BaseDirectory::Resource)
        .map_err(|e| {
            tf!(
                "동봉된 확장을 찾지 못했어요: {e}",
                "Couldn't find the bundled extension: {e}"
            )
        })?;
    if !mcpb.is_file() {
        return Err(ts!(
            "동봉된 확장(kura.mcpb)이 이 빌드에 없어요. 앱을 다시 설치해 보세요.",
            "This build has no bundled extension (kura.mcpb). Try reinstalling the app."
        )
        .into());
    }
    // /usr/bin/open 절대경로 — PATH 의 가짜 open 을 타지 않게 (mcpb 런처와 같은 원칙).
    let out = Command::new("/usr/bin/open")
        .args(["-b", CLAUDE_DESKTOP_BUNDLE_ID])
        .arg(&mcpb)
        .output()
        .map_err(|e| {
            tf!(
                "Claude 데스크톱을 열지 못했어요: {e}",
                "Couldn't open the Claude desktop app: {e}"
            )
        })?;
    if !out.status.success() {
        return Err(tf!(
            "Claude 데스크톱을 열지 못했어요: {}",
            "Couldn't open the Claude desktop app: {}",
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
pub(crate) fn connect_claude_code() -> Result<(), ConnectError> {
    // 아직 아무것도 안 지운 실패들 — 되돌릴 게 없으니 restore_command 도 없다.
    let bare = |message: String| ConnectError {
        message,
        restore_command: None,
    };
    let Some(cli) = find_claude_cli() else {
        return Err(bare(
            ts!(
                "claude 명령을 찾지 못했어요. 아래 수동 등록 명령을 터미널에서 실행하세요.",
                "Couldn't find the claude command. Run the manual command below in a terminal."
            )
            .into(),
        ));
    };
    let (mcp, temp) = registerable_mcp_path();
    let Some(mcp) = mcp else {
        if temp {
            // 임시 경로(Translocation·DMG)를 등록하면 마운트 해제·재실행 순간 Claude 가
            // 서버를 못 띄우는 등록이 남는다(코덱스 개발35 3차) — 등록하느니 거부가 낫다.
            return Err(bare(
                ts!(
                    "앱이 임시 위치(디스크 이미지 등)에서 실행 중이라 등록할 경로가 없어요. \
                     Kura 를 응용 프로그램 폴더로 옮겨서 실행한 뒤 다시 연결하세요.",
                    "The app is running from a temporary location (a disk image, for example), so \
                     there's no path worth registering. Move Kura to your Applications folder, \
                     open it from there, and connect again."
                )
                .into(),
            ));
        }
        return Err(bare(
            ts!(
                "이 빌드에 kura-mcp 가 없어요. 앱을 다시 설치해 보세요.",
                "This build has no kura-mcp. Try reinstalling the app."
            )
            .into(),
        ));
    };
    // 멱등 재등록: 같은 이름이 이미 있으면 add 가 "already exists" 로 거부하는데,
    // 그걸 성공으로 치면 옛 경로를 가리키는 등록이 영영 안 고쳐진다(코덱스 개발35 1차).
    // 지우기 전에 옛 항목을 통째로 떠 둔다 — remove 만 성공하고 add 가 실패하면 멀쩡하던
    // 등록마저 사라지므로(코덱스 개발35 3차), 그때 add-json 으로 원복한다.
    // remove 는 스냅숏 유무와 무관하게 **무조건** 돌린다(코덱스 개발38 1차): 우리가
    // .claude.json 을 못 읽는 환경에서도 CLI 는 항목을 볼 수 있고, 그때 remove 를
    // 건너뛰면 add 가 "already exists" 로 죽는다. 없어서 실패하는 remove 는 정상.
    let old_entry = registered_cli_entry();
    let _ = Command::new(&cli)
        .args(["mcp", "remove", "--scope", "user", "kura"])
        .output();
    let failure = match Command::new(&cli)
        .args(["mcp", "add", "--scope", "user", "kura", "--"])
        .arg(&mcp)
        .output()
    {
        Ok(out) if out.status.success() => return Ok(()),
        Ok(out) => tf!(
            "등록 실패: {}",
            "Registration failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => tf!("claude 실행 실패: {e}", "Couldn't run claude: {e}"),
    };
    let (restore_note, restore_command) = match old_entry {
        Some(old) => {
            let old_json = old.to_string();
            let restored = Command::new(&cli)
                .args(["mcp", "add-json", "--scope", "user", "kura"])
                .arg(&old_json)
                .output()
                .is_ok_and(|o| o.status.success());
            let note = if restored {
                ts!(
                    " 기존 등록은 그대로 되돌려 놨어요.",
                    " Your previous entry was put back."
                )
            } else {
                ts!(" 기존 kura 등록을 복원하지 못했어요 — 아래 명령으로 직접 등록하세요.", " Couldn't restore your previous kura entry — register it yourself with the command below.")
            };
            // 원복이 성공했든 실패했든 이 명령을 준다. 성공했어도 아래 수동 시퀀스의
            // remove 가 그걸 다시 지울 수 있으니, 되돌릴 손잡이는 손에 쥐고 있어야 한다.
            (
                note,
                Some(format!(
                    "claude mcp add-json --scope user kura {}",
                    shell_single_quote(&old_json)
                )),
            )
        }
        None => ("", None),
    };
    Err(ConnectError {
        message: format!("{failure}{restore_note}"),
        restore_command,
    })
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
        assert_eq!(
            claude_json_kura_command(r#"{"mcpServers":{"kura":{}}}"#),
            None
        );
        // 프로젝트 범위에만 있는 건 등록으로 안 친다.
        assert_eq!(
            claude_json_kura_command(
                r#"{"projects":{"/a":{"mcpServers":{"kura":{"command":"x"}}}}}"#
            ),
            None
        );
        assert_eq!(claude_json_kura_command("{}"), None);
        assert_eq!(claude_json_kura_command("broken"), None);
    }

    // 원복용 항목 스냅숏: command 가 없어도 항목이 있으면 통째로 돌려준다(HTTP 서버
    // 등록도 지웠으면 되돌려야 한다). null·부재는 "항목 없음".
    #[test]
    fn claude_json_entry_snapshot() {
        let json = r#"{"mcpServers":{"kura":{"type":"stdio","command":"/a/kura-mcp","args":[]}}}"#;
        let entry = claude_json_kura_entry(json).expect("항목이 있어야 한다");
        assert_eq!(entry["command"], "/a/kura-mcp");
        assert_eq!(entry["type"], "stdio");
        assert!(claude_json_kura_entry(r#"{"mcpServers":{"kura":{}}}"#).is_some());
        assert_eq!(
            claude_json_kura_entry(r#"{"mcpServers":{"kura":null}}"#),
            None
        );
        assert_eq!(claude_json_kura_entry(r#"{"mcpServers":{}}"#), None);
        assert_eq!(claude_json_kura_entry("{}"), None);
    }

    // 임시 실행 위치 감지 (코덱스 개발35 3차 + 개발38 1차): Translocation 은 무조건 임시,
    // /Volumes 는 읽기 전용(DMG)일 때만 임시 — 외장 SSD 설치(쓰기 가능)는 정상 취급.
    #[test]
    fn temp_location_detection() {
        use std::path::Path;
        let ro = |_: &Path| true; // DMG 처럼 읽기 전용 볼륨
        let rw = |_: &Path| false; // 외장 SSD 처럼 쓰기 가능 볼륨
        assert!(classify_temp_location(
            Path::new(
                "/private/var/folders/ab/xyz/T/AppTranslocation/UUID/d/Kura.app/Contents/MacOS/kura"
            ),
            rw, // Translocation 은 볼륨 상태와 무관하게 임시
        ));
        assert!(classify_temp_location(
            Path::new("/var/folders/ab/xyz/T/AppTranslocation/UUID/d/Kura.app/Contents/MacOS/kura"),
            rw,
        ));
        assert!(classify_temp_location(
            Path::new("/Volumes/Kura 0.1.2/Kura.app/Contents/MacOS/kura"),
            ro,
        ));
        assert!(!classify_temp_location(
            Path::new("/Volumes/외장SSD/Applications/Kura.app/Contents/MacOS/kura"),
            rw,
        ));
        assert!(!classify_temp_location(
            Path::new("/Applications/Kura.app/Contents/MacOS/kura"),
            ro, // 프리픽스가 아니면 볼륨 상태는 보지도 않는다
        ));
        assert!(!classify_temp_location(
            Path::new("/Users/a/Applications/Kura.app/Contents/MacOS/kura"),
            rw,
        ));
        assert!(!classify_temp_location(
            Path::new("/Users/a/프로젝트/지갑지갑/src-tauri/target/debug/kura"),
            rw,
        ));
    }

    // 임시 실행 중 설치본 버전 대조 (코덱스 개발38 2차 P2). 등록될 kura-mcp 는 설치본
    // 안의 것이라, 그게 지금 화면을 그리는 이 앱과 다른 버전이면 사용자에게 알려야 한다.
    #[test]
    fn installed_version_mismatch_detection() {
        use std::path::{Path, PathBuf};
        let mcp: PathBuf = "/Applications/Kura.app/Contents/MacOS/kura-mcp".into();
        let reads = |v: &'static str| move |_: &Path| Some(v.to_string());

        // 임시 실행이 아니면 볼 것도 없다 — mcp_path 는 이 앱 자신의 사이드카다.
        assert_eq!(
            classify_installed_mismatch(false, Some(&mcp), "0.2.1", reads("0.2.0")),
            None
        );
        // 임시 실행 + 설치본 버전이 다름 → 그 버전을 알린다.
        assert_eq!(
            classify_installed_mismatch(true, Some(&mcp), "0.2.1", reads("0.2.0")),
            Some("0.2.0".to_string())
        );
        // 같은 버전이면 조용히.
        assert_eq!(
            classify_installed_mismatch(true, Some(&mcp), "0.2.1", reads("0.2.1")),
            None
        );
        // 설치본이 아예 없어 등록할 경로가 없는 상태(프론트는 "옮기기" 안내를 띄운다).
        assert_eq!(
            classify_installed_mismatch(true, None, "0.2.1", reads("0.2.0")),
            None
        );
        // 버전을 못 읽으면 막지 않는다 — 모르는 걸 경고로 바꾸지 않는다.
        assert_eq!(
            classify_installed_mismatch(true, Some(&mcp), "0.2.1", |_: &Path| None),
            None
        );
        // 조상 셋을 못 올라가는 짧은 경로 → None (패닉이 아니라).
        assert_eq!(
            classify_installed_mismatch(true, Some(Path::new("/kura-mcp")), "0.2.1", reads("0.2.0")),
            None
        );
    }

    // 실물 plutil 스모크: 이 맥에 깔린 Kura.app 이 있으면 버전 문자열이 나와야 하고,
    // 없는 번들은 None 이어야 한다(못 읽는다고 패닉하지 않는다).
    #[test]
    fn bundle_version_smoke() {
        use std::path::Path;
        assert_eq!(bundle_short_version(Path::new("/no/such/App.app")), None);
        let installed = Path::new("/Applications/Kura.app");
        if installed.is_dir() {
            let v = bundle_short_version(installed).expect("설치된 앱은 버전이 읽혀야 한다");
            assert!(
                v.chars().next().is_some_and(|c| c.is_ascii_digit()),
                "버전 같지 않은 값: {v}"
            );
        }
    }

    // 원복 명령의 셸 인용: JSON 은 따옴표를 반드시 물고 있고, 값 안에 작은따옴표가
    // 들어와도 명령이 쪼개지면 안 된다.
    #[test]
    fn restore_command_quoting() {
        assert_eq!(
            shell_single_quote(r#"{"command":"/a/kura-mcp"}"#),
            r#"'{"command":"/a/kura-mcp"}'"#
        );
        assert_eq!(
            shell_single_quote("/Users/a/it's here/kura-mcp"),
            r#"'/Users/a/it'\''s here/kura-mcp'"#
        );
    }

    // 실물 statfs 스모크: 루트 볼륨(쓰기 가능한 데이터 볼륨에 병합 마운트)은 이 판정에서
    // 읽기 전용이 아니어야 하고, 없는 경로는 판별 불가 → false(사용자를 막지 않는다).
    #[test]
    fn volume_read_only_smoke() {
        use std::path::Path;
        assert!(!volume_is_read_only(Path::new("/Applications")));
        assert!(!volume_is_read_only(Path::new("/no/such/path/anywhere")));
    }
}
