// 지갑 키 관리 (Session 2~5) — 니모닉 생성·주소 파생·비번 암호화(Argon2id + AES-256-GCM).
// 키는 ~/.jigap/wallet.enc 에 암호화 저장. 비번이 있어야만 복호화/서명 가능.
// 기존 평문 wallet.json 은 최초 1회 마이그레이션으로 암호화 후 삭제.

use crate::i18n::{tf, ts};
use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use alloy::signers::local::{
    coins_bip39::{English, Mnemonic},
    MnemonicBuilder, PrivateKeySigner,
};
use argon2::Argon2;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use zeroize::Zeroizing;

use crate::store::{jigap_dir, write_atomic};

/// KDF(Argon2id) 파라미터 — wallet.enc v3 (개발 17, 메인넷 전 보안 강화).
/// RFC 9106의 메모리 제약 환경 권장값(64 MiB, t=3). 비번 1회 입력당 ~0.5초 수준의 비용으로
/// 오프라인 무차별 대입(파일 탈취 후 비번 추측)을 크게 비싸게 만든다.
const KDF_M_V3: u32 = 65536; // KiB = 64 MiB
const KDF_T_V3: u32 = 3;
const KDF_P_V3: u32 = 1;

/// v2 시절 파라미터 = argon2 crate 0.5의 `Argon2::default()` (19 MiB, t=2, p=1).
/// kdf 필드가 없는 옛 파일은 이 값으로 복호화해야 한다 — 바꾸면 기존 지갑이 안 열린다.
const KDF_M_V2: u32 = 19_456;
const KDF_T_V2: u32 = 2;
const KDF_P_V2: u32 = 1;

fn default_kdf_m() -> u32 {
    KDF_M_V2
}
fn default_kdf_t() -> u32 {
    KDF_T_V2
}
fn default_kdf_p() -> u32 {
    KDF_P_V2
}

/// 프론트로 넘기는 지갑 정보. 니모닉/키는 절대 포함하지 않는다.
#[derive(Serialize)]
pub(crate) struct WalletInfo {
    address: String,
}

/// 지갑 파일 상태. 프론트가 어떤 화면을 띄울지 결정한다.
/// - "encrypted": wallet.enc 존재 (정상)
/// - "legacy":    평문 wallet.json 만 존재 → 비번으로 보호 필요(마이그레이션)
/// - "none":      지갑 없음 → 새로 생성
#[derive(Serialize)]
pub(crate) struct WalletStatus {
    state: String,
    address: Option<String>,
    /// 시드 백업 완료 여부 (encrypted 상태일 때만 의미 있음).
    backed_up: bool,
}

/// 아직 암호화 지갑이 없는가 (= 첫 화면이 설정/가져오기여야 하는가).
/// 메뉴바 앱은 평소엔 조용히 트레이에만 있지만, 첫 실행에는 팝오버를 띄워야 한다(개발 26).
/// 읽기에 실패하면 보수적으로 true — 안내 화면을 띄우는 쪽이 안전하다.
pub(crate) fn needs_setup() -> bool {
    get_wallet_status()
        .map(|s| s.state != "encrypted")
        .unwrap_or(true)
}

/// 암호화된 지갑 파일 (v2/v3). 주소만 평문(공개정보)이라 비번 없이도 잔액/QR 표시 가능.
/// v3 = KDF 파라미터를 파일에 명시 + 강화값(KDF_*_V3). v2 파일은 kdf 필드가 없어서
/// serde default(=옛 기본값)로 복호화되고, 비번 잠금 해제 성공 시 v3로 재암호화된다.
#[derive(Serialize, Deserialize)]
pub(crate) struct EncryptedWallet {
    version: u32,
    pub(crate) address: String,
    salt: String,       // base64
    nonce: String,      // base64
    ciphertext: String, // base64 (암호화된 니모닉)
    /// 사용자가 시드 12단어를 직접 백업했는지. 옛 파일엔 없어서 기본값 false.
    #[serde(default)]
    backed_up: bool,
    /// Argon2id 파라미터 (KiB / 반복 / 병렬). 옛 v2 파일엔 없어서 v2 기본값으로 폴백.
    #[serde(default = "default_kdf_m")]
    kdf_m: u32,
    #[serde(default = "default_kdf_t")]
    kdf_t: u32,
    #[serde(default = "default_kdf_p")]
    kdf_p: u32,
}

/// 옛 평문 형식 (v1). 읽기 전용 — 마이그레이션 때만 사용.
#[derive(Deserialize)]
struct LegacyWallet {
    mnemonic: String,
    address: String,
}

fn enc_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join("wallet.enc"))
}

fn legacy_path() -> Result<PathBuf, String> {
    Ok(jigap_dir()?.join("wallet.json"))
}

/// 니모닉 문구에서 서명자(개인키)를 파생한다.
/// 파생 경로는 이더리움 표준 m/44'/60'/0'/0/0 (alloy 기본값).
pub(crate) fn signer_from_phrase(phrase: &str) -> Result<PrivateKeySigner, String> {
    MnemonicBuilder::<English>::default()
        .phrase(phrase)
        .build()
        .map_err(|e| {
            tf!(
                "니모닉에서 키 파생 실패: {e}",
                "Couldn't derive the key from the recovery phrase: {e}"
            )
        })
}

/// 니모닉 문구에서 EVM 주소(EIP-55 체크섬)를 파생한다.
fn derive_address(phrase: &str) -> Result<String, String> {
    Ok(signer_from_phrase(phrase)?.address().to_string())
}

/// 비번 + 솔트 + 명시적 Argon2id 파라미터로 32바이트 대칭키를 유도한다.
/// Zeroizing 반환 — 어느 경로로 드랍돼도(틀린 비번 early-return 포함) 메모리에서 0으로 지워진다.
fn derive_key(
    password: &str,
    salt: &[u8],
    m: u32,
    t: u32,
    p: u32,
) -> Result<Zeroizing<[u8; 32]>, String> {
    use argon2::{Algorithm, Params, Version};
    let params = Params::new(m, t, p, Some(32)).map_err(|e| {
        tf!(
            "KDF 파라미터 오류: {e}",
            "Key-derivation parameters are invalid: {e}"
        )
    })?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut *key)
        .map_err(|e| tf!("키 유도 실패: {e}", "Couldn't derive the key: {e}"))?;
    Ok(key)
}

/// 니모닉을 비번으로 암호화해서 저장용 구조체로 만든다 (파라미터 지정 — 테스트가 v2 재현용으로도 사용).
fn encrypt_with(
    phrase: &str,
    address: &str,
    password: &str,
    version: u32,
    m: u32,
    t: u32,
    p: u32,
) -> Result<EncryptedWallet, String> {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);

    let key = derive_key(password, &salt, m, t, p)?;
    let cipher = Aes256Gcm::new_from_slice(&*key)
        .map_err(|e| tf!("암호기 생성 실패: {e}", "Couldn't set up encryption: {e}"))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, phrase.as_bytes())
        .map_err(|e| tf!("암호화 실패: {e}", "Encryption failed: {e}"))?;

    Ok(EncryptedWallet {
        version,
        address: address.to_string(),
        salt: B64.encode(salt),
        nonce: B64.encode(nonce_bytes),
        ciphertext: B64.encode(ciphertext),
        backed_up: false,
        kdf_m: m,
        kdf_t: t,
        kdf_p: p,
    })
}

/// 니모닉을 비번으로 암호화한다 — 항상 최신(v3, 강화 KDF)으로 쓴다.
fn encrypt_wallet(phrase: &str, address: &str, password: &str) -> Result<EncryptedWallet, String> {
    encrypt_with(phrase, address, password, 3, KDF_M_V3, KDF_T_V3, KDF_P_V3)
}

/// 암호화된 지갑에서 니모닉을 복호화한다. 비번이 틀리면 GCM 인증 실패 → 에러.
/// Zeroizing 반환 — 니모닉은 키 그 자체이므로 모든 호출 경로에서 드랍 시 메모리를 0으로 지운다.
pub(crate) fn decrypt_wallet(
    w: &EncryptedWallet,
    password: &str,
) -> Result<Zeroizing<String>, String> {
    let salt = B64
        .decode(&w.salt)
        .map_err(|e| tf!("솔트 디코드 실패: {e}", "Couldn't decode the salt: {e}"))?;
    let nonce_bytes = B64
        .decode(&w.nonce)
        .map_err(|e| tf!("논스 디코드 실패: {e}", "Couldn't decode the nonce: {e}"))?;
    let ciphertext = B64.decode(&w.ciphertext).map_err(|e| {
        tf!(
            "암호문 디코드 실패: {e}",
            "Couldn't decode the ciphertext: {e}"
        )
    })?;

    // 파일에 기록된 파라미터로 키 유도 — v2(필드 없음)는 serde default = 옛 기본값.
    let key = derive_key(password, &salt, w.kdf_m, w.kdf_t, w.kdf_p)?;
    let cipher = Aes256Gcm::new_from_slice(&*key)
        .map_err(|e| tf!("암호기 생성 실패: {e}", "Couldn't set up encryption: {e}"))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let plaintext = cipher.decrypt(nonce, ciphertext.as_ref()).map_err(|_| {
        ts!("비밀번호가 올바르지 않습니다", "That password isn't right").to_string()
    })?;

    String::from_utf8(plaintext)
        .map(Zeroizing::new)
        .map_err(|e| {
            tf!(
                "복호화 데이터 손상: {e}",
                "The decrypted data is damaged: {e}"
            )
        })
}

/// 암호화된 지갑을 디스크에 저장 (소유자만 읽기/쓰기, 원자 교체 —
/// mark_backed_up 등 재작성 중 크래시로 지갑 파일이 깨지면 키 유실이라 치명적).
fn write_encrypted(w: &EncryptedWallet) -> Result<(), String> {
    let json = serde_json::to_string_pretty(w)
        .map_err(|e| tf!("직렬화 실패: {e}", "Couldn't serialize the wallet: {e}"))?;
    write_atomic(&enc_path()?, json.as_bytes())
}

pub(crate) fn read_encrypted() -> Result<EncryptedWallet, String> {
    let data = fs::read_to_string(enc_path()?).map_err(|e| {
        tf!(
            "지갑 파일 읽기 실패: {e}",
            "Couldn't read the wallet file: {e}"
        )
    })?;
    serde_json::from_str(&data).map_err(|e| {
        tf!(
            "지갑 파일 파싱 실패: {e}",
            "Couldn't parse the wallet file: {e}"
        )
    })
}

/// v2(옛 KDF) 지갑을 강화 파라미터(v3)로 재암호화한 사본을 만든다. 이미 v3면 None.
/// 순수 함수 — 디스크에 안 쓴다(쓰기는 maybe_upgrade_kdf). backed_up 플래그는 보존.
fn upgraded_wallet(w: &EncryptedWallet, phrase: &str, password: &str) -> Option<EncryptedWallet> {
    if w.version >= 3 {
        return None;
    }
    let mut nw = encrypt_wallet(phrase, &w.address, password).ok()?;
    nw.backed_up = w.backed_up;
    Some(nw)
}

/// 비번 잠금 해제 성공 시 옛 KDF(v2) 파일을 v3로 재암호화한다 (lazy 업그레이드).
/// 실패해도 조용히 넘어간다 — 업그레이드는 부가 기능, 잠금 해제를 막으면 안 된다.
/// (write_encrypted = 원자 교체라 도중 크래시에도 기존 파일이 깨지지 않는다.)
pub(crate) fn maybe_upgrade_kdf(w: &EncryptedWallet, phrase: &str, password: &str) {
    if let Some(nw) = upgraded_wallet(w, phrase, password) {
        let _ = write_encrypted(&nw);
    }
}

/// 비번으로 저장된 키를 복호화해 서명자를 만든다 (송금·서명 공용).
/// 비번이 틀리면 복호화 단계에서 거부된다. 성공 시 옛 KDF 파일은 v3로 업그레이드.
pub(crate) fn unlock_signer(password: &str) -> Result<PrivateKeySigner, String> {
    let w = read_encrypted()?;
    let phrase = decrypt_wallet(&w, password)?;
    maybe_upgrade_kdf(&w, &phrase, password);
    signer_from_phrase(&phrase)
}

/// 지갑 파일 상태를 알려준다. 비번 없이 호출 가능 (주소는 공개정보).
#[tauri::command]
pub(crate) fn get_wallet_status() -> Result<WalletStatus, String> {
    if enc_path()?.exists() {
        let w = read_encrypted()?;
        return Ok(WalletStatus {
            state: "encrypted".into(),
            address: Some(w.address),
            backed_up: w.backed_up,
        });
    }
    if legacy_path()?.exists() {
        let data = fs::read_to_string(legacy_path()?).map_err(|e| {
            tf!(
                "지갑 파일 읽기 실패: {e}",
                "Couldn't read the wallet file: {e}"
            )
        })?;
        let lw: LegacyWallet = serde_json::from_str(&data).map_err(|e| {
            tf!(
                "지갑 파일 파싱 실패: {e}",
                "Couldn't parse the wallet file: {e}"
            )
        })?;
        return Ok(WalletStatus {
            state: "legacy".into(),
            address: Some(lw.address),
            backed_up: false,
        });
    }
    Ok(WalletStatus {
        state: "none".into(),
        address: None,
        backed_up: false,
    })
}

/// 새 12단어 니모닉을 생성하고 비번으로 암호화 저장한다.
/// (비번 분실 시 복구 불가 — 시드 백업 흐름이 생성 직후 이어진다.)
#[tauri::command]
pub(crate) fn create_wallet(password: String) -> Result<WalletInfo, String> {
    let password = Zeroizing::new(password); // 사용 후 메모리에서 0으로 덮음
    if enc_path()?.exists() {
        return Err(ts!("이미 지갑이 있습니다", "A wallet already exists").into());
    }
    let mut rng = rand::thread_rng();
    let mnemonic = Mnemonic::<English>::new_with_count(&mut rng, 12).map_err(|e| {
        tf!(
            "니모닉 생성 실패: {e}",
            "Couldn't generate a recovery phrase: {e}"
        )
    })?;
    let phrase = Zeroizing::new(mnemonic.to_phrase());
    let address = derive_address(&phrase)?;

    let w = encrypt_wallet(&phrase, &address, &password)?;
    write_encrypted(&w)?;

    Ok(WalletInfo { address })
}

/// 기존 평문 wallet.json 을 비번으로 암호화한다 (최초 1회). 성공 시 평문 파일 삭제.
#[tauri::command]
pub(crate) fn migrate_wallet(password: String) -> Result<WalletInfo, String> {
    let password = Zeroizing::new(password);
    if enc_path()?.exists() {
        return Err(ts!(
            "이미 암호화된 지갑이 있습니다",
            "An encrypted wallet already exists"
        )
        .into());
    }
    let legacy = legacy_path()?;
    let data = fs::read_to_string(&legacy).map_err(|e| {
        tf!(
            "기존 지갑 읽기 실패: {e}",
            "Couldn't read the existing wallet: {e}"
        )
    })?;
    let lw: LegacyWallet = serde_json::from_str(&data).map_err(|e| {
        tf!(
            "기존 지갑 파싱 실패: {e}",
            "Couldn't parse the existing wallet: {e}"
        )
    })?;
    let mnemonic = Zeroizing::new(lw.mnemonic);

    // 무결성 확인: 저장된 주소가 니모닉에서 실제 파생되는지.
    let derived = derive_address(&mnemonic)?;
    if !derived.eq_ignore_ascii_case(&lw.address) {
        return Err(ts!(
            "기존 지갑 파일이 손상되었습니다 (주소 불일치)",
            "The existing wallet file is damaged (address mismatch)"
        )
        .into());
    }

    let w = encrypt_wallet(&mnemonic, &derived, &password)?;
    write_encrypted(&w)?;

    // 암호화 성공 후에만 평문 파일 제거.
    fs::remove_file(&legacy).map_err(|e| {
        tf!(
            "기존 평문 파일 삭제 실패: {e}",
            "Couldn't delete the old unencrypted file: {e}"
        )
    })?;

    Ok(WalletInfo { address: derived })
}

/// 복구 문구 정규화: 앞뒤·중간 공백과 줄바꿈을 단일 공백으로 모으고 영문 소문자로 바꾼다.
/// (붙여넣을 때 섞여 들어오는 줄바꿈·중복 공백·대문자를 흡수한다. BIP-39 영문 단어는 전부
/// 소문자 ASCII라 소문자화는 유효한 문구를 절대 깨지 않는다 — 비-ASCII 입력은 그대로 두되
/// 어차피 파생 검증에서 거부된다.)
///
/// 결과를 처음부터 `Zeroizing<String>` 버퍼에 직접 쌓는다 — split/join/to_lowercase 가 만드는
/// 중간 String 사본(니모닉 평문이 든)이 제로화 안 된 채 드랍되는 걸 피하기 위함. 용량을 입력
/// 길이로 잡아두면(정규화 결과는 항상 더 짧다) 재할당이 없어 이 버퍼가 유일한 힙 사본이 된다.
fn normalize_mnemonic(phrase: &str) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::with_capacity(phrase.len()));
    for word in phrase.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        for c in word.chars() {
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

/// 가져온 복구 문구를 검증·정규화해 암호화 지갑(backed_up=true)으로 만든다.
/// 순수 함수(디스크 미접근) — 명령은 파일 존재 검사·쓰기만 감싼다. 테스트가 직접 호출한다.
/// 사용자가 12단어를 직접 입력했다는 건 이미 시드를 보유했다는 뜻 → 백업 플로우가 불필요하다.
fn build_imported_wallet(phrase: &str, password: &str) -> Result<EncryptedWallet, String> {
    let normalized = normalize_mnemonic(phrase);
    let count = normalized.split_whitespace().count();
    if !matches!(count, 12 | 15 | 18 | 21 | 24) {
        return Err(tf!(
            "복구 문구는 12·15·18·21·24단어 중 하나여야 해요 (지금 {count}개)",
            "A recovery phrase has 12, 15, 18, 21, or 24 words (this one has {count})"
        ));
    }
    // BIP-39 체크섬·단어 검증 + 주소 파생. 잘못된 단어·순서·체크섬이면 여기서 거부된다.
    // (원시 파생 에러는 내부 정보라 노출하지 않고 사람 말 메시지로 바꾼다.)
    let address = derive_address(&normalized).map_err(|_| {
        ts!(
            "복구 문구가 올바르지 않아요. 단어와 순서를 다시 확인해주세요.",
            "That recovery phrase isn't valid. Check the words and their order."
        )
        .to_string()
    })?;

    let mut w = encrypt_wallet(&normalized, &address, password)?;
    w.backed_up = true;
    Ok(w)
}

/// 기존 복구 문구(BIP-39 12~24단어)를 가져와 비번으로 암호화 저장한다.
/// 다른 지갑에서 옮겨오거나 비번을 잊어 재설정할 때 사용. wallet.enc 가 이미 있으면 거부한다
/// (덮어쓰면 기존 키가 영구 유실되므로 — 가져오려면 사용자가 명시적으로 정리해야 한다).
#[tauri::command]
pub(crate) fn import_wallet(password: String, phrase: String) -> Result<WalletInfo, String> {
    let password = Zeroizing::new(password); // 사용 후 메모리에서 0으로 덮음
    let phrase = Zeroizing::new(phrase);
    if enc_path()?.exists() {
        return Err(ts!("이미 지갑이 있습니다", "A wallet already exists").into());
    }
    let w = build_imported_wallet(&phrase, &password)?;
    let address = w.address.clone();
    write_encrypted(&w)?;
    Ok(WalletInfo { address })
}

/// 비번으로 시드 니모닉(12단어)을 복호화해서 돌려준다. 백업 화면에서만 호출.
/// 비번이 틀리면 복호화 단계에서 거부된다. (자산 영구 손실 방지용 — 비번을 잊어도
/// 이 12단어만 있으면 다른 지갑에서 복구 가능하다.)
#[tauri::command]
pub(crate) fn reveal_mnemonic(password: String) -> Result<Vec<String>, String> {
    let password = Zeroizing::new(password);
    let w = read_encrypted()?;
    let phrase = decrypt_wallet(&w, &password)?;
    Ok(phrase.split_whitespace().map(|s| s.to_string()).collect())
}

/// 사용자가 시드 백업을 마쳤다고 표시한다. 비번은 필요 없다(비밀이 아닌 메타데이터).
#[tauri::command]
pub(crate) fn mark_backed_up() -> Result<(), String> {
    let mut w = read_encrypted()?;
    if !w.backed_up {
        w.backed_up = true;
        write_encrypted(&w)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 잘 알려진 BIP-39 테스트 벡터 (Anvil/Hardhat 기본 니모닉의 0번 계정).
    #[test]
    fn derive_known_vector() {
        let phrase = "test test test test test test test test test test test junk";
        let addr = derive_address(phrase).expect("파생 성공");
        assert_eq!(addr, "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    }

    // 새로 생성한 12단어 니모닉 → 유효한 0x 주소(42자).
    #[test]
    fn generate_and_derive_roundtrip() {
        let mut rng = rand::thread_rng();
        let mnemonic = Mnemonic::<English>::new_with_count(&mut rng, 12).expect("니모닉 생성");
        let phrase = mnemonic.to_phrase();
        assert_eq!(phrase.split_whitespace().count(), 12);
        let addr = derive_address(&phrase).expect("파생 성공");
        assert!(addr.starts_with("0x") && addr.len() == 42);
    }

    // 암호화 → 올바른 비번으로 복호화하면 원본 니모닉이 그대로 나온다.
    #[test]
    fn encrypt_decrypt_roundtrip() {
        let phrase = "test test test test test test test test test test test junk";
        let addr = derive_address(phrase).unwrap();
        let w = encrypt_wallet(phrase, &addr, "비밀번호123").expect("암호화");
        let out = decrypt_wallet(&w, "비밀번호123").expect("복호화");
        assert_eq!(out.as_str(), phrase);
    }

    // 틀린 비번이면 복호화가 실패해야 한다 (GCM 인증 태그).
    #[test]
    fn wrong_password_fails() {
        let phrase = "test test test test test test test test test test test junk";
        let addr = derive_address(phrase).unwrap();
        let w = encrypt_wallet(phrase, &addr, "맞는비번").expect("암호화");
        assert!(decrypt_wallet(&w, "틀린비번").is_err());
    }

    // 복호화한 니모닉을 12단어로 쪼개면 reveal_mnemonic 이 돌려줄 결과가 된다.
    #[test]
    fn reveal_splits_into_twelve_words() {
        let phrase = "test test test test test test test test test test test junk";
        let addr = derive_address(phrase).unwrap();
        let w = encrypt_wallet(phrase, &addr, "pw").unwrap();
        let words: Vec<String> = decrypt_wallet(&w, "pw")
            .unwrap()
            .split_whitespace()
            .map(String::from)
            .collect();
        assert_eq!(words.len(), 12);
        assert_eq!(words[0], "test");
        assert_eq!(words[11], "junk");
    }

    // 새로 암호화한 지갑은 아직 백업 안 된 상태여야 한다.
    #[test]
    fn new_wallet_is_not_backed_up() {
        let w = encrypt_wallet("a b c", "0x0", "pw").unwrap();
        assert!(!w.backed_up);
    }

    // backed_up/kdf 필드가 없는 옛 v2 파일도 읽혀야 한다 (#[serde(default)]).
    // kdf 기본값은 반드시 v2 시절 파라미터(19 MiB/t=2/p=1) — 아니면 기존 지갑이 안 열린다.
    #[test]
    fn legacy_v2_without_backed_up_field_loads() {
        let json =
            r#"{"version":2,"address":"0xabc","salt":"AA==","nonce":"AA==","ciphertext":"AA=="}"#;
        let w: EncryptedWallet = serde_json::from_str(json).expect("파싱 성공");
        assert!(!w.backed_up);
        assert_eq!((w.kdf_m, w.kdf_t, w.kdf_p), (KDF_M_V2, KDF_T_V2, KDF_P_V2));
    }

    // 새로 암호화하면 v3 + 강화 KDF 파라미터여야 한다.
    #[test]
    fn new_wallet_uses_strengthened_kdf() {
        let w = encrypt_wallet("a b c", "0x0", "pw").unwrap();
        assert_eq!(w.version, 3);
        assert_eq!((w.kdf_m, w.kdf_t, w.kdf_p), (KDF_M_V3, KDF_T_V3, KDF_P_V3));
    }

    // 실제 v2 파일 재현: kdf 필드를 JSON에서 지워도(옛 파일 그대로) 복호화돼야 한다.
    #[test]
    fn real_v2_file_without_kdf_fields_decrypts() {
        let phrase = "test test test test test test test test test test test junk";
        let v2 = encrypt_with(phrase, "0x0", "pw", 2, KDF_M_V2, KDF_T_V2, KDF_P_V2).unwrap();
        let mut json: serde_json::Value = serde_json::to_value(&v2).unwrap();
        let obj = json.as_object_mut().unwrap();
        obj.remove("kdf_m");
        obj.remove("kdf_t");
        obj.remove("kdf_p"); // 옛 파일엔 이 필드들이 없다
        let w: EncryptedWallet = serde_json::from_value(json).unwrap();
        assert_eq!(decrypt_wallet(&w, "pw").unwrap().as_str(), phrase);
    }

    // lazy 업그레이드: v2 → v3 사본(같은 비번으로 복호화 가능, backed_up 보존), v3 → None.
    #[test]
    fn upgrade_v2_to_v3_preserves_contents() {
        let phrase = "test test test test test test test test test test test junk";
        let mut v2 = encrypt_with(phrase, "0xAddr", "pw", 2, KDF_M_V2, KDF_T_V2, KDF_P_V2).unwrap();
        v2.backed_up = true;

        let v3 = upgraded_wallet(&v2, phrase, "pw").expect("v2는 업그레이드 대상");
        assert_eq!(v3.version, 3);
        assert_eq!(
            (v3.kdf_m, v3.kdf_t, v3.kdf_p),
            (KDF_M_V3, KDF_T_V3, KDF_P_V3)
        );
        assert_eq!(v3.address, "0xAddr");
        assert!(v3.backed_up); // 백업 표시 보존
        assert_eq!(decrypt_wallet(&v3, "pw").unwrap().as_str(), phrase);

        // 이미 v3면 재암호화하지 않는다.
        assert!(upgraded_wallet(&v3, phrase, "pw").is_none());
    }

    // 정규화: 줄바꿈·중복 공백·대문자를 흡수해 표준 12단어로 정리한다.
    #[test]
    fn normalize_collapses_whitespace_and_lowercases() {
        let out = normalize_mnemonic(
            "  TEST   test\ntest\ttest test test test test test test test JUNK ",
        );
        assert_eq!(out.split_whitespace().count(), 12);
        assert_eq!(
            &*out,
            "test test test test test test test test test test test junk"
        );
    }

    // 가져오기: 지저분한 입력(대문자·여분 공백)도 정규화 후 알려진 벡터로 파생되고,
    // v3 강화 KDF + backed_up=true(시드 직접 보유)로 암호화된다. 복호화하면 정규화된 문구가 나온다.
    #[test]
    fn import_builds_backed_up_v3_from_messy_phrase() {
        let w = build_imported_wallet(
            "  TEST test\ntest test test test test test test test test junk ",
            "pw",
        )
        .expect("정상 문구");
        assert_eq!(w.version, 3);
        assert_eq!((w.kdf_m, w.kdf_t, w.kdf_p), (KDF_M_V3, KDF_T_V3, KDF_P_V3));
        assert!(w.backed_up); // 사용자가 시드를 직접 입력 = 이미 보유
        assert_eq!(w.address, "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        assert_eq!(
            decrypt_wallet(&w, "pw").unwrap().as_str(),
            "test test test test test test test test test test test junk"
        );
    }

    // 체크섬이 안 맞는 12단어(abandon×12)는 거부돼야 한다 — 오타·잘못된 문구 방어.
    #[test]
    fn import_rejects_bad_checksum() {
        let bad = "abandon abandon abandon abandon abandon abandon \
                   abandon abandon abandon abandon abandon abandon";
        assert!(build_imported_wallet(bad, "pw").is_err());
    }

    // 참고: abandon×11 + about 은 체크섬이 맞는 표준 벡터라 가져오기가 성공해야 한다.
    #[test]
    fn import_accepts_valid_all_zero_vector() {
        let ok = "abandon abandon abandon abandon abandon abandon \
                  abandon abandon abandon abandon abandon about";
        let w = build_imported_wallet(ok, "pw").expect("유효한 표준 벡터");
        assert_eq!(w.address, "0x9858EfFD232B4033E47d90003D41EC34EcaEda94");
    }

    // 단어 수가 12/15/18/21/24 가 아니면 거부 (사람 말 메시지).
    #[test]
    fn import_rejects_wrong_word_count() {
        assert!(build_imported_wallet("test test test", "pw").is_err());
        assert!(build_imported_wallet("", "pw").is_err());
    }

    // 같은 입력이라도 매번 솔트/논스가 달라 암호문이 달라야 한다.
    #[test]
    fn ciphertext_is_randomized() {
        let phrase = "test test test test test test test test test test test junk";
        let a = encrypt_wallet(phrase, "0x0", "pw").unwrap();
        let b = encrypt_wallet(phrase, "0x0", "pw").unwrap();
        assert_ne!(a.ciphertext, b.ciphertext);
        assert_ne!(a.salt, b.salt);
    }
}
