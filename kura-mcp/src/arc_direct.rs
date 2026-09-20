// x402 「직접 제출」 갈래 — 가스가 곧 결제자산인 체인(Arc)에서 페이실리테이터 없이 결제한다 (개발 64).
//
// 표준 `exact` 스킴은 우리가 EIP-3009 인가에 **서명만** 하고, 페이실리테이터가 그걸 온체인에 올리며
// 가스를 대신 낸다. Arc 엔 그 역할을 하는 페이실리테이터가 없었다(개발 50·62 가 「서명 경로만」으로
// 남겨 둔 이유). 그런데 Arc 는 **가스가 USDC 자체**라, 구매자가 자기 인가를 직접 올리면 된다 —
// 결제액과 가스가 같은 잔액에서 나가므로 누가 가스를 대 줄 이유가 애초에 없다.
//
// 그 규격이 x402 제안 #3504 의 `assetTransferMethod: "eip3009-client-broadcast"` 이고, 서버 쪽
// 구현체(kaditang/x402-arc, MIT)가 Arc 메인넷에서 돌고 있다. 흐름이 표준과 뒤집힌다:
//   표준:      서명 → 헤더로 제출 → 서버/페이실리테이터가 정산(그쪽 가스)
//   직접 제출: 서명 → **우리가 브로드캐스트**(우리 가스) → 영수증이 나온 뒤 **tx 해시를 증거로** 제출
//
// 🔴 **이 갈래에선 「x402 는 우리 가스가 안 나간다」는 가정이 깨진다.** 그 가정은 세 곳에 박혀 있었고
// (자율 경로의 가스 여유분 제외·승인 창 잔액 검사·그 주석) 개발 64 에서 전부 갈래를 나눴다.
// 여기서 만드는 요청의 kind 가 `x402-direct` 인 것이 그 갈림길이다 — GUI 는 이걸 **송금과 같은 것**
// (한도·잠금·가스 여유분·내역 "sent")으로 다루고, `x402`(서명만)와 다른 경로로 보낸다.
//
// ── nonce: 제안 원안이 틀렸고, 고친 쪽이 실물이다 ────────────────────────────────────────────
// 원안은 nonce 를 **요구사항 해시**로만 만들었다. EIP-3009 nonce 는 1회용이라(토큰이
// `authorizationState(authorizer, nonce)` 를 기록한다) 그러면 같은 구매자가 같은 리소스를 같은 값에
// **평생 한 번만** 살 수 있다. 우리가 낸 제안의 결함이고, 답글에서 인정했다
// (메모리 `reference_x402_arc_proposal`).
//
// 실제로 도는 규격은 **구매자가 신선도를 댄다**: `nonce = sha256("arc-nonce-v2|" ‖ canonical(binding) ‖ "|" ‖ clientNonce)`.
// binding(network·asset·payTo·amount·resource)은 **서버의 요구사항**에서 재구성되므로, 다른 값·다른
// 리소스로 한 결제는 서버가 기대하는 nonce 를 만들지 못한다. clientNonce 는 `extra` 가 아니라
// **payload** 로 간다 — `extra` 에 넣으면 `required.extra ⊆ accepted.extra` 매칭이 모든 유효 결제를
// 튕긴다(서버가 검증 때 요구사항을 다시 만들기 때문).
//
// 서버가 `extra.seed` 를 준 경우(seed 모드)는 접두어가 `arc-nonce-v1` 이고 seed 를 그대로 되돌려준다.
// 두 모드 다 우리는 **받은 값으로 계산만** 한다 — 서버가 `extra.nonce` 를 같이 줬다면 우리 계산과
// 같은지 대조하고, 다르면 결제하지 않는다(그 챌린지가 주장하는 것과 다른 결제가 된다).

use alloy::primitives::B256;
use alloy::providers::{Provider, ProviderBuilder};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::Duration;

use crate::tf;
use crate::wallet::{effective_rpc, redact_urls};

/// `extra.assetTransferMethod` 가 이 값이면 「구매자가 직접 올린다」 (x402 제안 #3504).
pub const METHOD_CLIENT_BROADCAST: &str = "eip3009-client-broadcast";

/// nonce 가 묶이는 요구사항 조각. **서버가 준 값 그대로** 채운다 — 서버도 검증 때 자기 요구사항으로
/// 같은 것을 만들기 때문에, 한 글자라도 우리 식으로 다듬으면 nonce 가 갈린다.
pub struct NonceBinding {
    pub network: String,
    pub asset: String,
    pub pay_to: String,
    /// base unit 문자열("30000") — 십진 변환 전 값.
    pub amount: String,
    /// `extra.resource` (없으면 빈 문자열). **표시용 URL 이 아니다** — 표시는 우리가 실제로 요청한
    /// 최종 URL 이고(개발 51), 이건 nonce 계산에만 쓰는 서버의 문자열이다.
    pub resource: String,
}

/// 정규형 — 주소만 소문자로(EVM 주소는 대소문자 무의미하고 체크섬 표기가 클라이언트마다 다르다),
/// 나머지는 그대로. **키 순서가 값을 바꾸지 못하게** 객체가 아니라 고정 순서 배열로 직렬화한다.
/// (JS 쪽 `JSON.stringify([...])` 와 바이트가 같아야 한다 — 아래 골든 벡터 테스트가 그걸 문다.)
fn canonical(b: &NonceBinding) -> String {
    serde_json::Value::Array(vec![
        Value::String(b.network.clone()),
        Value::String(b.asset.to_lowercase()),
        Value::String(b.pay_to.to_lowercase()),
        Value::String(b.amount.clone()),
        Value::String(b.resource.clone()),
    ])
    .to_string()
}

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("0x{}", alloy::hex::encode(h.finalize()))
}

/// 기본 모드 — 구매자가 신선도를 댄다. `clientNonce` 는 4~32바이트 hex.
pub fn client_nonce_digest(client_nonce: &str, b: &NonceBinding) -> String {
    sha256_hex(&format!(
        "arc-nonce-v2|{}|{}",
        canonical(b),
        client_nonce.to_lowercase()
    ))
}

/// seed 모드 — 서버가 챌린지마다 발급한 seed 를 그대로 묶는다.
pub fn seed_nonce_digest(seed: &str, b: &NonceBinding) -> String {
    sha256_hex(&format!("arc-nonce-v1|{}|{}", canonical(b), seed))
}

/// 결제마다 새로 뽑는 구매자 신선도 값(16바이트 hex). **이게 같은 값으로 두 번 나가면 두 번째
/// 결제는 체인에서 revert 한다**(EIP-3009 nonce 재사용) → 운영체제 엔트로피에서 뽑는다.
pub fn new_client_nonce() -> String {
    alloy::hex::encode(&B256::random().0[..16])
}

/// 이번 결제의 신선도 값과 nonce 를 한 번에 정한다 — **서버가 seed 를 줬으면 seed 모드, 아니면
/// 기본(클라이언트 nonce) 모드.** 반환 = (clientNonce, seed, nonce).
///
/// 흐름(flow.rs)과 하네스가 **같은 함수**를 쓰라고 빼 뒀다. 이 판정을 테스트가 따로 베껴 쓰면
/// 「테스트는 초록인데 실제 배선은 다른 값을 만드는」 상태가 만들어진다(개발 63 이 실측으로 겪은 것).
pub fn fresh_nonce(
    seed: Option<&str>,
    b: &NonceBinding,
) -> (Option<String>, Option<String>, String) {
    match seed {
        Some(seed) => {
            let n = seed_nonce_digest(seed, b);
            (None, Some(seed.to_string()), n)
        }
        None => {
            let cn = new_client_nonce();
            let n = client_nonce_digest(&cn, b);
            (Some(cn), None, n)
        }
    }
}

/// 서버가 우리가 계산한 것과 **다른** nonce 를 게시했으면 결제하지 않는다. 그 값은 이 챌린지가
/// 무엇에 대한 결제인지를 묶는 값이라, 다르다는 건 챌린지가 주장하는 것과 다른 결제라는 뜻이다.
/// (kadi 클라이언트도 같은 자리에서 같은 이유로 거절한다.)
pub fn published_nonce_ok(published: Option<&str>, derived: &str) -> bool {
    match published {
        None => true,
        Some(p) => p.trim().eq_ignore_ascii_case(derived),
    }
}

/// 직접 제출의 증거 — 우리가 올린 tx 와, 서버가 nonce 를 재구성하는 데 필요한 값.
/// `x402Version:2` 제출의 `payload` 자리에 그대로 들어간다.
pub struct DirectProof {
    pub transaction: String,
    /// 기본 모드에서만 — 서버가 이것으로 nonce 를 다시 만든다.
    pub client_nonce: Option<String>,
    /// seed 모드에서만 — 받은 seed 를 에코.
    pub seed: Option<String>,
    /// 우리가 쓴 nonce. 서버는 자기 계산과 대조만 한다(신뢰하지 않는다).
    pub nonce: String,
}

impl DirectProof {
    pub fn payload(&self) -> Value {
        let mut v = serde_json::json!({
            "transaction": self.transaction,
            "nonce": self.nonce,
        });
        let map = v.as_object_mut().expect("json object");
        if let Some(cn) = &self.client_nonce {
            map.insert("clientNonce".into(), Value::String(cn.clone()));
        }
        if let Some(seed) = &self.seed {
            map.insert("seed".into(), Value::String(seed.clone()));
        }
        v
    }
}

/// 브로드캐스트한 tx 의 결말.
#[derive(PartialEq, Debug)]
pub enum ReceiptOutcome {
    /// 채굴됐고 성공(status 0x1). 서버에 증거로 낼 수 있다.
    Mined,
    /// 채굴됐는데 revert(status 0x0) — 가스만 나갔다. 서버에 내 봐야 거절된다.
    Reverted,
    /// 제한 시간 안에 안 잡혔다. **실패가 아니다** — 나중에 채굴될 수 있다.
    Pending,
}

/// 영수증이 나올 때까지 기다린다. 서버(kadi 기본값)는 **최소 1 확인**을 요구하므로, 브로드캐스트
/// 직후 바로 재요청하면 「아직 안 끝났다」로 거절된다 → 여기서 기다린 뒤 제출한다.
///
/// 🔴 **시간 초과를 「실패」로 바꾸지 않는다**(개발 51 이 세 라운드로 치른 것). 돈은 이미 체인에
/// 나갔고, 여기서 실패라고 말하면 AI 가 **다시 결제한다**. 못 잡으면 `Pending` 으로 돌려주고,
/// 호출자가 「결제는 나갔다, tx 는 이것이다」를 그대로 알린다.
pub async fn wait_for_receipt(tx_hash: &str, timeout: Duration) -> Result<ReceiptOutcome, String> {
    let hash: B256 = tx_hash
        .trim()
        .parse()
        .map_err(|e| tf!("tx 해시 파싱 실패: {e}", "Couldn't read the tx hash: {e}"))?;
    let provider = ProviderBuilder::new()
        .connect(&effective_rpc())
        .await
        .map_err(|e| {
            tf!(
                "RPC 연결 실패: {}",
                "Couldn't reach the RPC server: {}",
                redact_urls(&e.to_string())
            )
        })?;
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        // 조회 실패(일시적 RPC 오류)는 즉시 포기할 사유가 아니다 — 남은 시간 동안 다시 묻는다.
        if let Ok(Some(receipt)) = provider.get_transaction_receipt(hash).await {
            return Ok(if receipt.status() {
                ReceiptOutcome::Mined
            } else {
                ReceiptOutcome::Reverted
            });
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(ReceiptOutcome::Pending);
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
}

/// 영수증을 못 잡았을 때 AI 에게 그대로 전할 문구 — **재시도하지 말라**가 핵심이다.
pub fn pending_notice(tx: &str, explorer: &str) -> String {
    let link = if explorer.is_empty() {
        String::new()
    } else {
        format!(" ({explorer})")
    };
    tf!(
        "결제는 체인에 이미 올라갔어요(tx {tx}{link}). 다만 확인이 늦어 서버에 증거를 내지 못했습니다. \
         **다시 요청하면 또 결제됩니다** — 잠시 뒤 같은 URL 을 다시 요청하기 전에 사용자에게 알리세요.",
        "The payment is already on-chain (tx {tx}{link}), but the receipt didn't confirm in time, so the \
         server wasn't given the proof. **Asking again will pay again** — tell the user before retrying."
    )
}

/// 서버에 낼 게 없을 때(revert) — 가스는 나갔고 결제는 안 됐다.
pub fn reverted_notice(tx: &str) -> String {
    tf!(
        "결제 트랜잭션이 체인에서 실패했어요(tx {tx}). 결제액은 나가지 않았지만 가스는 소모됐습니다.",
        "The payment transaction reverted on-chain (tx {tx}). The amount wasn't paid, but gas was spent."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stockwaves() -> NonceBinding {
        NonceBinding {
            network: "eip155:5042".into(),
            asset: "0x3600000000000000000000000000000000000000".into(),
            pay_to: "0xDc9F94A8b93F070B58cfa580cbE740d763005FE6".into(),
            amount: "30000".into(),
            resource: "https://stockwaves.net/api/xstock/health".into(),
        }
    }

    /// 🔴 **골든 벡터 — 실제 구현체(kaditang/x402-arc)의 `clientNonceFor` 가 낸 값이다.**
    /// 개발 64 에서 그 리포를 받아 `node`(24, TS 직접 실행)로 돌려 뽑았다. 우리 러스트 계산이
    /// 한 바이트라도 다르면 서버가 기대하는 nonce 와 달라져 **모든 결제가 거절된다** — 그런데
    /// 자체 테스트만 있으면 「우리끼리 일관」될 뿐이라 그걸 못 잡는다. 상대 구현의 출력을 박아 둔다.
    ///
    /// 세 번째 벡터는 JSON 이스케이프(`"` 와 `\`)가 양쪽에서 같은지를 문다 — 정규형이
    /// `JSON.stringify` 와 바이트가 같아야 하는 자리라, URL 에 따옴표가 섞이면 갈릴 수 있다.
    #[test]
    fn client_nonce_matches_reference_implementation() {
        assert_eq!(
            client_nonce_digest("0123456789abcdef0123456789abcdef", &stockwaves()),
            "0x82fc16dbda29c8d2e5305f4226e34d1c253abaca86030b6acaf681a081397e3b"
        );
        let arc_testnet = NonceBinding {
            network: "eip155:5042002".into(),
            asset: "0x3600000000000000000000000000000000000000".into(),
            pay_to: "0x8b7ba5077d261739f5FeBB31B10167671e590161".into(),
            amount: "10000".into(),
            resource: String::new(), // extra.resource 가 없는 서버 = 빈 문자열(양쪽 같은 규칙)
        };
        assert_eq!(
            client_nonce_digest("deadbeef", &arc_testnet),
            "0xf7bbab7d93119ca67eeb65308cac208c091e46cf25e533721ec1b190ae320e61"
        );
        let quoted = NonceBinding {
            network: "eip155:5042".into(),
            asset: "0xAAAA000000000000000000000000000000000000".into(),
            pay_to: "0xbbbb000000000000000000000000000000000000".into(),
            amount: "1".into(),
            resource: r#"https://ex.com/a?b=1&c="q"\z"#.into(),
        };
        assert_eq!(
            client_nonce_digest("AABBCCDDEEFF0011", &quoted),
            "0x9d48be5bfa29ee7eca4077f1d832013a9156bb304599f5d77b57a0d05c6d49ab"
        );
    }

    /// seed 모드도 같은 출처의 골든 벡터로 문다(접두어가 `arc-nonce-v1` 로 다르다).
    #[test]
    fn seed_nonce_matches_reference_implementation() {
        assert_eq!(
            seed_nonce_digest("v1.123.aa.bb", &stockwaves()),
            "0x23eb1ed49bb30fe900e0beaa392235b74d19cd20d0b079d45cc8bbc270c697c0"
        );
    }

    /// 대문자로 온 clientNonce 는 소문자와 같은 nonce 를 내야 한다(상대 구현이 소문자로 접는다).
    /// 반대로 **binding 의 amount·resource 는 대소문자를 안 접는다** — 접으면 다른 결제가 된다.
    #[test]
    fn case_rules_match_reference() {
        let b = stockwaves();
        assert_eq!(
            client_nonce_digest("AABB", &b),
            client_nonce_digest("aabb", &b)
        );
        // 주소는 체크섬 표기를 흡수한다.
        let lowered = NonceBinding {
            pay_to: b.pay_to.to_lowercase(),
            ..stockwaves()
        };
        assert_eq!(
            client_nonce_digest("aabb", &b),
            client_nonce_digest("aabb", &lowered)
        );
        // 리소스 한 글자만 달라도 다른 nonce (다른 엔드포인트의 결제로 재사용 불가).
        let other = NonceBinding {
            resource: "https://stockwaves.net/api/xstock/healthz".into(),
            ..stockwaves()
        };
        assert_ne!(
            client_nonce_digest("aabb", &b),
            client_nonce_digest("aabb", &other)
        );
        // 금액이 다르면 다른 nonce (싼 결제로 비싼 걸 사지 못한다).
        let pricey = NonceBinding {
            amount: "300000".into(),
            ..stockwaves()
        };
        assert_ne!(
            client_nonce_digest("aabb", &b),
            client_nonce_digest("aabb", &pricey)
        );
    }

    /// 매번 다른 값이어야 한다 — 같으면 두 번째 결제가 체인에서 revert 한다.
    #[test]
    fn client_nonce_is_fresh_each_time() {
        let a = new_client_nonce();
        let b = new_client_nonce();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32); // 16바이트 hex
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// 서버가 게시한 nonce 와 우리 계산이 다르면 결제하지 않는다(대소문자·공백은 흡수).
    #[test]
    fn published_nonce_guard() {
        let derived = "0xabc123";
        assert!(published_nonce_ok(None, derived)); // 안 준 서버가 대다수 — 예전대로 진행
        assert!(published_nonce_ok(Some("0xABC123"), derived));
        assert!(published_nonce_ok(Some("  0xabc123 "), derived));
        assert!(!published_nonce_ok(Some("0xdeadbeef"), derived));
    }

    /// payload 모양 — 기본 모드는 clientNonce, seed 모드는 seed. 둘이 섞이면 안 된다.
    #[test]
    fn proof_payload_shape() {
        let p = DirectProof {
            transaction: "0xTX".into(),
            client_nonce: Some("aabb".into()),
            seed: None,
            nonce: "0xNONCE".into(),
        };
        let v = p.payload();
        assert_eq!(v["transaction"], "0xTX");
        assert_eq!(v["clientNonce"], "aabb");
        assert_eq!(v["nonce"], "0xNONCE");
        assert!(v.get("seed").is_none());

        let s = DirectProof {
            transaction: "0xTX".into(),
            client_nonce: None,
            seed: Some("v1.1.a.b".into()),
            nonce: "0xNONCE".into(),
        };
        let v = s.payload();
        assert_eq!(v["seed"], "v1.1.a.b");
        assert!(v.get("clientNonce").is_none());
    }
}
