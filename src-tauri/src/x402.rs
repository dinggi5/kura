// x402 EIP-3009 오프체인 서명 (Session 11).
//
// x402의 "exact" 스킴은 온체인 transfer()가 아니라 EIP-3009 transferWithAuthorization 을
// "오프체인 EIP-712 서명"만 한다. 실제 정산(온체인 제출·가스)은 페이실리테이터가 한다.
// → 우리 지갑은 ETH 가스가 없어도 결제할 수 있고, 키 접근(서명)은 오직 이 GUI 프로세스만.
//
// 보안: 서명도 "내 USDC를 빼갈 권한"을 주는 행위다 → 송금과 똑같이 긴급 잠금·단일/일일
// 한도를 적용하고, 서명 시점에 누적 사용액에 기록한다(보수적: 서명=인출 권한 부여).

use crate::i18n::{tf, ts};
use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::OsRng;
use alloy::primitives::{Address, B256, U256};
use alloy::signers::local::PrivateKeySigner;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::chain::{active_chain, with_pinned_chain, TransferWithAuthorization};
use crate::history::log_attempt;
use crate::limits::{parse_usdc_nonneg, refund_spend, reserve_spend};
use crate::lock::read_lock;
use crate::settings::read_settings;
use crate::store::now_secs;
use crate::transfer::parse_to_addr;
use crate::trusted::record_trusted;
use crate::wallet::{active_account_index, unlock_signer, with_pinned_account};

/// x402 결제 페이로드의 authorization 부분 (서명된 EIP-3009 인가). 비밀 없음.
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct X402Authorization {
    pub(crate) from: String,
    pub(crate) to: String,
    /// base unit 정수 문자열 (USDC 6 decimals → "10000" = 0.01 USDC).
    pub(crate) value: String,
    #[serde(rename = "validAfter")]
    pub(crate) valid_after: String,
    #[serde(rename = "validBefore")]
    pub(crate) valid_before: String,
    /// 32바이트 랜덤 (재생 방지). "0x..".
    pub(crate) nonce: String,
}

/// x402 "exact" 결제 페이로드 중 지갑이 만드는 부분 = 서명 + 인가.
/// x402Version/scheme/network 같은 프로토콜 메타데이터는 호출자(MCP)가 덧붙인다.
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct X402Payment {
    /// 65바이트 r||s||v 서명 ("0x..").
    pub(crate) signature: String,
    pub(crate) authorization: X402Authorization,
}

/// 주어진 서명자로 EIP-3009 인가를 서명한다 (순수 암호 연산 — 잠금/한도 검사 없음, 테스트용).
/// validAfter=0(즉시 유효), validBefore=now+valid_secs, nonce=랜덤 32바이트.
async fn sign_authorization(
    signer: &PrivateKeySigner,
    to: Address,
    value: U256,
    valid_secs: u64,
) -> Result<X402Payment, String> {
    use alloy::signers::Signer;
    use alloy::sol_types::{eip712_domain, SolStruct};

    let chain = active_chain();
    let domain = eip712_domain! {
        name: chain.usdc_eip712_name,
        version: chain.usdc_eip712_version,
        chain_id: chain.chain_id,
        verifying_contract: chain.usdc_address,
    };

    let mut nonce_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = B256::from(nonce_bytes);

    let valid_after = U256::ZERO;
    let valid_before = U256::from(now_secs().saturating_add(valid_secs));

    let auth = TransferWithAuthorization {
        from: signer.address(),
        to,
        value,
        validAfter: valid_after,
        validBefore: valid_before,
        nonce,
    };

    // EIP-712 다이제스트 = domainSeparator ⊕ structHash. 이걸 서명한다.
    let hash = auth.eip712_signing_hash(&domain);
    let sig = signer
        .sign_hash(&hash)
        .await
        .map_err(|e| tf!("서명 실패: {e}", "Couldn't sign: {e}"))?;

    // 65바이트 r||s||v (v=27/28) — EIP-3009 ecrecover 가 기대하는 형식.
    let mut sig_bytes = [0u8; 65];
    sig_bytes[..32].copy_from_slice(&sig.r().to_be_bytes::<32>());
    sig_bytes[32..64].copy_from_slice(&sig.s().to_be_bytes::<32>());
    sig_bytes[64] = 27 + sig.v() as u8;

    Ok(X402Payment {
        signature: format!("0x{}", alloy::hex::encode(sig_bytes)),
        authorization: X402Authorization {
            from: signer.address().to_string(),
            to: to.to_string(),
            value: value.to_string(),
            valid_after: valid_after.to_string(),
            valid_before: valid_before.to_string(),
            nonce: nonce.to_string(),
        },
    })
}

/// x402 결제용 EIP-3009 인가를 서명한다 (온체인 전송 X — 페이실리테이터가 정산).
/// 송금과 동일하게 긴급 잠금·단일/일일 한도를 적용하고, 서명 시점에 누적 사용액에 기록한다.
/// (활성 체인의 USDC 전용. network/asset 은 앱 설정과 동일하게 고정.)
#[tauri::command]
pub(crate) async fn sign_x402_payment(
    password: String,
    to: String,
    amount_usdc: String,
    valid_secs: Option<u64>,
) -> Result<X402Payment, String> {
    let password = Zeroizing::new(password);
    // 진입 시 계정을 한 번 고정 (개발 54) — 서명 키(from)와 내역이 같은 계정을 본다.
    with_pinned_account(
        active_account_index(),
        sign_x402_pinned(password, to, amount_usdc, valid_secs),
    )
    .await
}

async fn sign_x402_pinned(
    password: Zeroizing<String>,
    to: String,
    amount_usdc: String,
    valid_secs: Option<u64>,
) -> Result<X402Payment, String> {
    let signer = match unlock_signer(&password) {
        Ok(s) => s,
        Err(e) => {
            log_attempt("USDC", to.trim(), amount_usdc.trim(), "failed", &e);
            return Err(e);
        }
    };
    let to_addr = to.clone();
    let payment = do_sign_x402(&signer, to, amount_usdc, valid_secs).await?;
    record_trusted(&to_addr); // 비번(사람) 승인 성공 = 신뢰 주소 학습
    Ok(payment)
}

/// x402 서명 코어 — 서명자가 이미 있는 상태에서 실행한다(비번 래퍼와 자율 승인 경로가 공유).
pub(crate) async fn do_sign_x402(
    signer: &PrivateKeySigner,
    to: String,
    amount_usdc: String,
    valid_secs: Option<u64>,
) -> Result<X402Payment, String> {
    // 작업 진입 시 체인·계정 고정 — EIP-712 도메인(체인ID·USDC)·한도·장부·내역이 모두 같은 체인·계정.
    with_pinned_chain(
        active_chain().chain_id,
        with_pinned_account(
            active_account_index(),
            do_sign_x402_inner(signer, to, amount_usdc, valid_secs),
        ),
    )
    .await
}

async fn do_sign_x402_inner(
    signer: &PrivateKeySigner,
    to: String,
    amount_usdc: String,
    valid_secs: Option<u64>,
) -> Result<X402Payment, String> {
    let dec = active_chain().usdc_decimals;
    let amt = amount_usdc.trim();
    let value: U256 = parse_usdc_nonneg(amt, dec)?;
    if value.is_zero() {
        return Err(ts!(
            "0보다 큰 금액을 입력하세요",
            "Enter an amount greater than 0"
        )
        .into());
    }
    let to_addr = parse_to_addr(&to)?;
    let to = to.trim();

    // 긴급 잠금: 켜져 있으면 결제를 가장 먼저 차단한다 (비상 스위치).
    if read_lock() {
        log_attempt(
            "USDC",
            to,
            amt,
            "blocked",
            ts!("긴급 잠금 (x402 서명)", "Emergency lock (x402 signature)"),
        );
        return Err(ts!(
            "긴급 잠금이 켜져 있어 결제가 차단됐어요. 해제 후 다시 시도하세요.",
            "Emergency lock is on, so the payment was blocked. Turn it off and try again."
        )
        .into());
    }

    // 한도 검사 + 예약 (송금과 동일 — 서명도 USDC 인출 권한 부여라 누적에 보수적으로 선반영).
    // 락은 빠른 파일 I/O 구간만 잡는다. 한도 초과면 여기서 거부.
    let settings = read_settings();
    let single: U256 = parse_usdc_nonneg(&settings.single_usdc, dec)?;
    let daily: U256 = parse_usdc_nonneg(&settings.daily_usdc, dec)?;
    let reserved_day = match reserve_spend("USDC", value, single, daily, dec).await {
        Ok(d) => d,
        Err(e) => {
            log_attempt("USDC", to, amt, "blocked", &e);
            return Err(e);
        }
    };

    // 서명만 하므로 RPC/가스 불필요. 서명자는 호출자가 넘긴다(비번 래퍼 또는 자율 세션 키).
    // 서명 실패 시 예약한 사용액을 환불한다(예약한 날에만).
    let valid = valid_secs.unwrap_or(600); // 기본 10분 유효
    let payment = match sign_authorization(signer, to_addr, value, valid).await {
        Ok(p) => p,
        Err(e) => {
            refund_spend("USDC", value, reserved_day).await;
            log_attempt("USDC", to, amt, "failed", &e);
            return Err(e);
        }
    };

    // 서명 성공 → 내역 로그(status "signed", detail=nonce; 누적은 예약 단계에서 기록됨).
    log_attempt("USDC", to, amt, "signed", &payment.authorization.nonce);

    Ok(payment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::IERC20;
    use alloy::primitives::address;
    use alloy::providers::ProviderBuilder;

    // [x402-1] EIP-3009 타입해시가 표준 상수와 일치 → 우리 구조체 정의(필드·순서·타입)가 정확하다.
    // (오프라인. keccak256("TransferWithAuthorization(address from,address to,uint256 value,
    //  uint256 validAfter,uint256 validBefore,bytes32 nonce)") = Circle FiatToken 의 TYPEHASH.)
    #[test]
    fn x402_eip3009_type_hash_matches_standard() {
        use alloy::primitives::{b256, keccak256};
        use alloy::sol_types::SolStruct;
        let expected = b256!("0x7c7c6cdb67a18743f49ec6fa9b35f50d52ed05cbed4cc592e13b44501c1a2267");
        let got = keccak256(TransferWithAuthorization::eip712_encode_type().as_bytes());
        assert_eq!(got, expected);
    }

    // [x402-2] 서명 → 같은 EIP-712 다이제스트로 복구하면 서명자 주소가 나온다 (서명 파이프라인 유효).
    // 오프라인. Anvil 0번 키로 서명하고 ecrecover 로 from 을 되찾는다.
    #[tokio::test]
    async fn x402_signature_recovers_to_signer() {
        use alloy::sol_types::{eip712_domain, SolStruct};

        let signer: PrivateKeySigner =
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
                .parse()
                .unwrap();
        let to = address!("0x209693Bc6afc0C5328bA36FaF03C514EF312287C");
        let value = U256::from(10_000u64); // 0.01 USDC
        let payment = sign_authorization(&signer, to, value, 600).await.unwrap();

        // authorization 직렬화 필드 점검.
        assert_eq!(payment.authorization.from, signer.address().to_string());
        assert_eq!(payment.authorization.value, "10000");
        assert_eq!(payment.authorization.valid_after, "0");
        assert!(payment.signature.starts_with("0x"));
        assert_eq!(payment.signature.len(), 2 + 130); // 0x + 65바이트 hex

        // 서명에서 서명자 복구 → from 과 같아야 한다.
        let chain = active_chain();
        let domain = eip712_domain! {
            name: chain.usdc_eip712_name,
            version: chain.usdc_eip712_version,
            chain_id: chain.chain_id,
            verifying_contract: chain.usdc_address,
        };
        let nonce: B256 = payment.authorization.nonce.parse().unwrap();
        let valid_before: U256 = payment.authorization.valid_before.parse().unwrap();
        let auth = TransferWithAuthorization {
            from: signer.address(),
            to,
            value,
            validAfter: U256::ZERO,
            validBefore: valid_before,
            nonce,
        };
        let hash = auth.eip712_signing_hash(&domain);
        let sig: alloy::primitives::Signature = payment.signature.parse().unwrap();
        let recovered = sig.recover_address_from_prehash(&hash).unwrap();
        assert_eq!(recovered, signer.address());
    }

    /// 🔴 [x402-4] 개발 50 — **Arc 에서 우리 서명이 실제로 정산될까**를 체인에 직접 물어본다(돈 0원).
    ///
    /// 도메인 일치(x402-3)는 "봉투 주소가 맞다"까지고, 이건 **컨트랙트가 그 서명을 받아 실행까지
    /// 하는가**를 본다. 방법: 아무 키나 하나 만들어 x402 와 같은 경로로 EIP-3009 인가를 서명하고,
    /// `transferWithAuthorization` 에 실어 `eth_call` 로 **시뮬레이션**한다. eth_call 은 상태를 안 바꾸고
    /// 가스도 안 쓰므로 잔액 0인 새 키로 안전하다. 금액 0 이면 잔액 검사에 안 걸려 **서명 검증만**
    /// 통과 여부를 가른다 → 성공 = 페이실리테이터가 이 서명을 그대로 정산할 수 있다.
    /// (개발 50 시점엔 Arc 용 페이실리테이터가 없어 진짜 왕복은 못 한다 — 우리 쪽 준비 완료의 증거다.)
    #[tokio::test]
    #[ignore = "네트워크 필요 — Arc 테스트넷에 eth_call 로 정산 시뮬레이션"]
    // sol! 이 만드는 호출 함수는 인자가 9개다 — EIP-3009 시그니처가 원래 그렇다.
    #[allow(clippy::too_many_arguments)]
    async fn x402_signature_would_settle_on_arc() {
        use crate::chain::{TransferWithAuthorization, ARC_TESTNET};
        use alloy::signers::local::PrivateKeySigner;
        use alloy::signers::Signer;
        use alloy::sol;
        use alloy::sol_types::{eip712_domain, Eip712Domain, SolStruct};

        sol! {
            #[sol(rpc)]
            interface IEIP3009 {
                function transferWithAuthorization(
                    address from, address to, uint256 value,
                    uint256 validAfter, uint256 validBefore, bytes32 nonce,
                    uint8 v, bytes32 r, bytes32 s
                ) external;
            }
        }

        let chain = ARC_TESTNET;
        let signer = PrivateKeySigner::random();
        let domain: Eip712Domain = eip712_domain! {
            name: chain.usdc_eip712_name,
            version: chain.usdc_eip712_version,
            chain_id: chain.chain_id,
            verifying_contract: chain.usdc_address,
        };

        let mut nonce_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut nonce_bytes);
        let auth = TransferWithAuthorization {
            from: signer.address(),
            to: Address::from([0x11u8; 20]),
            value: U256::ZERO, // 0 = 잔액 검사를 안 타고 서명 검증만 남는다
            validAfter: U256::ZERO,
            validBefore: U256::from(now_secs() + 3600),
            nonce: B256::from(nonce_bytes),
        };

        let provider = ProviderBuilder::new()
            .connect(chain.default_rpc)
            .await
            .expect("Arc RPC 연결");
        let usdc = IEIP3009::new(chain.usdc_address, &provider);

        // 시뮬레이션 한 번 = (서명 도메인, 기대 결과) 한 쌍.
        let sim = |dom: Eip712Domain| {
            let usdc = &usdc;
            let signer = &signer;
            let auth = auth.clone();
            async move {
                let sig = signer
                    .sign_hash(&auth.eip712_signing_hash(&dom))
                    .await
                    .expect("서명");
                usdc.transferWithAuthorization(
                    auth.from,
                    auth.to,
                    auth.value,
                    auth.validAfter,
                    auth.validBefore,
                    auth.nonce,
                    27 + sig.v() as u8,
                    B256::from(sig.r().to_be_bytes::<32>()),
                    B256::from(sig.s().to_be_bytes::<32>()),
                )
                .call()
                .await
                .map(|_| ()) // 반환값은 볼 게 없다 — Ok/Err 만 본다(그리고 Debug 로 찍기 위함).
            }
        };

        let ours = sim(domain).await;
        println!("Arc 정산 시뮬레이션 (우리 도메인): {ours:?}");
        assert!(
            ours.is_ok(),
            "Arc USDC 가 우리 EIP-3009 서명을 거부했다 = x402 정산이 안 된다: {ours:?}"
        );

        // 대조군 — 버전 한 글자만 틀린 도메인. 이게 없으면 위 성공이 "아무 서명이나 받는다"는
        // 뜻일 수도 있어 증거가 못 된다(개발 12 의 «통과하지만 아무것도 안 보는 검사» 방지).
        let wrong: Eip712Domain = eip712_domain! {
            name: chain.usdc_eip712_name,
            version: "1", // 실제는 "2"
            chain_id: chain.chain_id,
            verifying_contract: chain.usdc_address,
        };
        let bad = sim(wrong).await;
        println!("대조군 (틀린 도메인): {bad:?}");
        assert!(
            bad.is_err(),
            "틀린 도메인 서명이 통과했다 — 위 성공은 증거가 못 된다"
        );
    }

    // [x402-3] 우리가 만든 EIP-712 도메인 세퍼레이터가 온체인 USDC.DOMAIN_SEPARATOR() 와 일치
    // (네트워크 필요). → name/version/chainId/컨트랙트가 정확 → 우리 서명을 USDC 가 그대로 받아준다.
    // 타입해시(x402-1) + 도메인(x402-3) + 복구(x402-2)가 다 맞으면 온체인 정산과 수학적으로 동등.
    //
    // **양 체인 모두** 검증한다(설정과 무관, 명시 const). 특히 메인넷 USDC 의 도메인 name 은 Sepolia
    // ("USDC")와 달리 "USD Coin" 이라, 이 테스트가 메인넷 진입 전 그 값이 맞는지 확정하는 게이트다.
    #[tokio::test]
    #[ignore = "네트워크 필요 (Base Sepolia + Base 메인넷 공개 RPC)"]
    async fn x402_domain_matches_usdc_onchain() {
        use crate::chain::{ARC_TESTNET, BASE_MAINNET, BASE_SEPOLIA};
        use alloy::sol_types::{eip712_domain, Eip712Domain};

        // Arc 도 같이 본다 (개발 50). 이 테스트가 Arc 에서 통과한다 = **우리가 만드는 서명 도메인이
        // Arc USDC 컨트랙트가 기대하는 것과 바이트 단위로 같다** = 정산해 줄 서버만 나타나면 그대로 선다.
        for chain in [BASE_SEPOLIA, BASE_MAINNET, ARC_TESTNET] {
            let domain: Eip712Domain = eip712_domain! {
                name: chain.usdc_eip712_name,
                version: chain.usdc_eip712_version,
                chain_id: chain.chain_id,
                verifying_contract: chain.usdc_address,
            };
            let local = domain.separator();

            let provider = ProviderBuilder::new()
                .connect(chain.default_rpc)
                .await
                .expect("RPC 연결");
            let usdc = IERC20::new(chain.usdc_address, &provider);
            let onchain = usdc
                .DOMAIN_SEPARATOR()
                .call()
                .await
                .expect("DOMAIN_SEPARATOR 조회");
            println!(
                "chain {} : local = {local}  onchain = {onchain}",
                chain.chain_id
            );
            assert_eq!(local, onchain, "체인 {} 도메인 불일치", chain.chain_id);
        }
    }
}
