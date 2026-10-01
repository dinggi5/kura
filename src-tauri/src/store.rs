// 공용 저장 유틸 — ~/.jigap 디렉터리, 원자적 파일 쓰기, 시간 헬퍼.
// 도메인 파일 경로(wallet.enc, settings.json 등)는 각 도메인 모듈이 정의한다.

use crate::i18n::{tf, ts};
use crate::policy;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// ~/.jigap 디렉터리 경로. 이름은 `policy::JIGAP_DIR_NAME`(MCP 와 같은 상수, 개발 57).
pub(crate) fn jigap_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or(ts!(
        "홈 디렉터리를 찾을 수 없습니다",
        "Couldn't find your home folder"
    ))?;
    Ok(policy::jigap_dir_in(&home))
}

/// 데이터 폴더당 앱 하나 (개발 73, 코덱스 1차 P1). 승인·한도 장부·내역·지갑 파일의 잠금은 전부 **프로세스 안** `Mutex` 라,
/// 같은 폴더를 쓰는 앱이 둘 뜨면(DMG 사본 + 설치본, 자동 시작 + 손으로 연 것) 둘 다 「첫 승인」으로 보고 같은 요청을
/// 두 번 보내거나 장부를 서로 덮을 수 있었다. 폴더의 잠금 파일을 OS 잠금(flock)으로 쥐고, 프로세스가 끝나면 OS 가 푼다.
static APP_LOCK: std::sync::OnceLock<std::fs::File> = std::sync::OnceLock::new();

#[derive(Debug, PartialEq)]
pub(crate) enum AppLock {
    /// 이 프로세스가 쥐었다.
    Held,
    /// 다른 앱이 쥐고 있다 — 이 프로세스는 끝내야 한다.
    Busy,
    /// 잠금 파일을 못 만들었다(홈·권한) — 그 폴더라면 지갑도 못 연다. 막지 않고 진행한다.
    Unavailable,
}

/// 잠금을 잡는다. 이미 잡혀 있으면 `wait` 동안 다시 해 본다 — 업데이트 재시작은 새 앱을 먼저 띄우고 옛 앱이 곧 끝나서
/// 둘이 잠깐 겹친다. 바로 포기하면 업데이트 뒤 앱이 사라진다.
pub(crate) fn hold_app_lock(wait: std::time::Duration) -> AppLock {
    let Ok(dir) = jigap_dir() else {
        return AppLock::Unavailable;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return AppLock::Unavailable;
    }
    let (verdict, file) = lock_at(&dir.join("app.lock"), wait);
    if let Some(f) = file {
        let _ = APP_LOCK.set(f);
    }
    verdict
}

/// `hold_app_lock` 의 몸통 — 경로를 받는다(테스트가 실지갑 폴더를 안 건드리게). 잡았으면 쥘 파일도 돌려준다.
fn lock_at(path: &std::path::Path, wait: std::time::Duration) -> (AppLock, Option<std::fs::File>) {
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
    else {
        return (AppLock::Unavailable, None);
    };
    let until = std::time::Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return (AppLock::Held, Some(file)),
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < until => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(std::fs::TryLockError::WouldBlock) => return (AppLock::Busy, None),
            Err(std::fs::TryLockError::Error(_)) => return (AppLock::Unavailable, None),
        }
    }
}

/// 현재 유닉스 시각(초).
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// UTC 기준 에포크 일수 (날짜 파싱 없이 일 단위 리셋용).
pub(crate) fn current_day() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

/// 원자 쓰기의 임시 파일 경로 — **작성자마다 다르다** (개발 66, 코덱스 P0).
///
/// 예전엔 `path.with_extension("tmp")` 하나를 모두가 같이 썼다. 같은 파일을 두 작성자가 동시에 쓰면
/// (예: 세션 잠금 해제의 KDF 업그레이드와 계정 이름 바꾸기가 둘 다 wallet.enc 를) 그 한 임시 파일을 번갈아
/// truncate·쓰기 하다 **섞인 내용을 rename** 했다 — 재현 테스트로 확인(256KB·64KB 두 스레드, 40회 중 발생).
/// 프로세스 id + 프로세스 안 순번이면 두 프로세스(GUI·MCP) 사이에서도 겹치지 않는다. 결과는 「마지막
/// 작성자가 이긴다」 — 내용이 섞이는 일은 없다.
fn unique_tmp(path: &Path) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.{}.{n}.tmp", std::process::id()))
}

/// 임시 파일에 쓴 뒤 rename 으로 원자 교체한다 — 쓰는 도중 크래시해도 기존 파일이 절반만
/// 써진 채 깨지지 않는다(wallet.enc 손상 = 키 유실, spend.json 손상 = 일일 한도 리셋이라 치명적).
/// ~/.jigap 디렉터리는 0700, 파일은 0600 — 내역·설정도 같은 머신의 타 계정에게 안 보이게.
/// **권한은 내용을 쓰기 "전"에** 좁힌다: 디렉터리는 생성 직후 chmod, 임시 파일은 0600 으로 생성 →
/// umask 가 느슨해도 평문 직전 데이터(임시 파일의 salt/nonce/ciphertext 등)가 잠깐도 넓게 노출되지 않게.
pub(crate) fn write_atomic(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or(ts!(
        "경로에 부모 디렉터리가 없습니다",
        "That path has no parent folder"
    ))?;
    fs::create_dir_all(dir)
        .map_err(|e| tf!("디렉터리 생성 실패: {e}", "Couldn't create the folder: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 파일을 쓰기 전에 디렉터리부터 0700 으로 좁힌다.
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    let tmp = unique_tmp(path);
    write_file_private(&tmp, bytes)?;
    fs::rename(&tmp, path).map_err(|e| tf!("파일 교체 실패: {e}", "Couldn't replace the file: {e}"))
}

/// `write_atomic_durable` 의 실패 — 둘은 뒷수습이 다르다.
#[derive(Debug)]
pub(crate) enum DurableError {
    /// 새 내용이 제자리에 안 갔다 — 파일은 예전 그대로다.
    NotWritten(String),
    /// 새 내용은 제자리에 갔는데 디렉터리를 디스크까지 못 내렸다 — 전원이 나가면 예전으로 돌아갈 수 있다.
    NotDurable(String),
}

/// `write_atomic` + **전원이 나가도 남는다** (개발 71, 코덱스 1차): 교체 전에 파일을, 교체 뒤에 디렉터리를 디스크까지 내린다.
/// 돈이 나가기 **전**에 써야 하는 기록(한도 예약)에만 쓴다 — 예약을 쓰고 tx 를 낸 직후 전원이 나가면 체인엔 송금이 남는데
/// 장부는 예약 전으로 돌아가 한도를 한 번 더 쓸 수 있었다. macOS 의 `sync_all` 은 F_FULLFSYNC(수~수십 ms)라 2초마다 쓰는
/// 하트비트 같은 곳엔 넣지 않는다.
///
/// 디렉터리 동기화 실패는 `NotDurable` 로 따로 돌려준다(개발 71 코덱스 2·3차) — 성공으로 치면 전원 장애 때 예약이 사라지고(3차 P1),
/// 그냥 실패로 치면 새 장부는 이미 제자리라 결제 없이 한도만 깎인다(2차 P2). 호출자가 예전 내용으로 되돌리고 거절한다.
pub(crate) fn write_atomic_durable(path: &PathBuf, bytes: &[u8]) -> Result<(), DurableError> {
    let not = |e: String| DurableError::NotWritten(e);
    let dir = path.parent().ok_or_else(|| {
        not(ts!(
            "경로에 부모 디렉터리가 없습니다",
            "That path has no parent folder"
        )
        .into())
    })?;
    fs::create_dir_all(dir).map_err(|e| {
        not(tf!(
            "디렉터리 생성 실패: {e}",
            "Couldn't create the folder: {e}"
        ))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 디렉터리 생성·0700 은 write_atomic 과 같다.
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    let tmp = unique_tmp(path);
    write_file_private(&tmp, bytes).map_err(not)?;
    fs::File::open(&tmp)
        .and_then(|f| f.sync_all())
        .map_err(|e| not(tf!("파일 저장 실패: {e}", "Couldn't save the file: {e}")))?;
    fs::rename(&tmp, path)
        .map_err(|e| not(tf!("파일 교체 실패: {e}", "Couldn't replace the file: {e}")))?;
    fs::File::open(dir).and_then(|d| d.sync_all()).map_err(|e| {
        DurableError::NotDurable(tf!("파일 저장 실패: {e}", "Couldn't save the file: {e}"))
    })
}

/// 임시 파일을 처음부터 0600 으로 생성해 내용을 쓴다 (생성 후 chmod 사이의 노출 창 제거).
#[cfg(unix)]
fn write_file_private(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| tf!("파일 저장 실패: {e}", "Couldn't save the file: {e}"))?;
    // 기존 tmp 가 느슨한 권한으로 남아 있던 경우(create 시 mode 미적용)까지 보장.
    let _ = f.set_permissions(fs::Permissions::from_mode(0o600));
    f.write_all(bytes)
        .map_err(|e| tf!("파일 저장 실패: {e}", "Couldn't save the file: {e}"))
}

#[cfg(not(unix))]
fn write_file_private(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|e| tf!("파일 저장 실패: {e}", "Couldn't save the file: {e}"))
}

pub(crate) fn write_json<T: Serialize>(path: PathBuf, value: &T) -> Result<(), String> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| tf!("직렬화 실패: {e}", "Couldn't serialize the data: {e}"))?;
    write_atomic(&path, json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 🔴 개발 73(코덱스 1차 P1): 잠금을 쥔 동안 두 번째는 Busy, 첫째가 놓으면(프로세스 끝) 잡힌다.
    #[test]
    fn second_app_lock_is_busy_until_the_first_lets_go() {
        let dir = std::env::temp_dir().join(format!("kura-applock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.lock");
        let (a, held) = lock_at(&path, std::time::Duration::ZERO);
        assert_eq!(a, AppLock::Held);
        let (b, _) = lock_at(&path, std::time::Duration::from_millis(300));
        assert_eq!(b, AppLock::Busy);
        drop(held);
        let (c, _) = lock_at(&path, std::time::Duration::ZERO);
        assert_eq!(c, AppLock::Held);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 🔴 **동시에 같은 파일을 원자 쓰기해도 결과는 둘 중 하나의 온전한 내용이어야 한다** (개발 66, 코덱스 P0).
    /// 예전엔 임시 파일 이름이 대상마다 하나(`wallet.tmp`)라, 두 작성자가 그 한 파일을 번갈아 truncate·쓰기
    /// 하다 **섞인 내용을 rename** 할 수 있었다 — wallet.enc 면 키 유실이다.
    #[test]
    fn concurrent_atomic_writes_never_mix() {
        let dir = std::env::temp_dir().join(format!("kura-test-race-{}", std::process::id()));
        let path = dir.join("wallet.enc");
        let a = vec![b'a'; 256 * 1024];
        let b = vec![b'b'; 64 * 1024];
        for _ in 0..40 {
            let (p1, p2) = (path.clone(), path.clone());
            let (a1, b1) = (a.clone(), b.clone());
            let t1 = std::thread::spawn(move || {
                for _ in 0..5 {
                    let _ = write_atomic(&p1, &a1);
                }
            });
            let t2 = std::thread::spawn(move || {
                for _ in 0..5 {
                    let _ = write_atomic(&p2, &b1);
                }
            });
            t1.join().unwrap();
            t2.join().unwrap();
            let got = fs::read(&path).unwrap();
            assert!(got == a || got == b, "섞인 내용: len={}", got.len());
        }
        let _ = fs::remove_dir_all(&dir);
    }

    // 원자 쓰기: 내용이 교체되고 임시 파일이 안 남는다 (크래시 시 절반 써진 파일 방지의 기반).
    #[test]
    fn write_atomic_roundtrip() {
        let dir = std::env::temp_dir().join(format!("kura-test-{}", std::process::id()));
        let path = dir.join("atomic.json");
        write_atomic(&path, b"one").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "one");
        write_atomic(&path, b"two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        // rename 후 임시 파일 없음(이름이 작성자마다 달라 디렉터리를 훑는다 — 개발 66).
        let leftovers = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);

        // 파일은 0600, 디렉터리는 0700 (타 계정 차단).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let fmode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            let dmode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(fmode, 0o600, "파일 권한 0600");
            assert_eq!(dmode, 0o700, "디렉터리 권한 0700");
        }
        let _ = fs::remove_dir_all(dir);
    }
}
