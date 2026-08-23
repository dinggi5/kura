// 공용 저장 유틸 — ~/.jigap 디렉터리, 원자적 파일 쓰기, 시간 헬퍼.
// 도메인 파일 경로(wallet.enc, settings.json 등)는 각 도메인 모듈이 정의한다.

use crate::i18n::{tf, ts};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// ~/.jigap 디렉터리 경로.
pub(crate) fn jigap_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or(ts!(
        "홈 디렉터리를 찾을 수 없습니다",
        "Couldn't find your home folder"
    ))?;
    Ok(home.join(".jigap"))
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

/// 임시 파일에 쓴 뒤 rename 으로 원자 교체한다 — 쓰는 도중 크래시해도 기존 파일이 절반만
/// 써진 채 깨지지 않는다(wallet.enc 손상 = 키 유실, spend.json 손상 = 일일 한도 리셋이라 치명적).
/// ~/.jigap 디렉터리는 0700, 파일은 0600 — 내역·설정도 같은 머신의 타 계정에게 안 보이게.
/// **권한은 내용을 쓰기 "전"에** 좁힌다: 디렉터리는 생성 직후 chmod, 임시 파일은 0600 으로 생성 →
/// umask 가 느슨해도 평문 직전 데이터(wallet.tmp 의 salt/nonce/ciphertext 등)가 잠깐도 넓게 노출되지 않게.
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
    let tmp = path.with_extension("tmp");
    write_file_private(&tmp, bytes)?;
    fs::rename(&tmp, path).map_err(|e| tf!("파일 교체 실패: {e}", "Couldn't replace the file: {e}"))
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

    // 원자 쓰기: 내용이 교체되고 임시 파일이 안 남는다 (크래시 시 절반 써진 파일 방지의 기반).
    #[test]
    fn write_atomic_roundtrip() {
        let dir = std::env::temp_dir().join(format!("kura-test-{}", std::process::id()));
        let path = dir.join("atomic.json");
        write_atomic(&path, b"one").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "one");
        write_atomic(&path, b"two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        assert!(!path.with_extension("tmp").exists()); // rename 후 임시 파일 없음

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
