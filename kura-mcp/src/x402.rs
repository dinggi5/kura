// x402 HTTP 결제 루프 (개발 11, Session 12 / V2 호환 개발 12, Session 13).
//
// x402 = HTTP 402 Payment Required 위에 올린 결제 프로토콜. 흐름:
//   1. 리소스 GET → 서버가 402 + 결제 요구(accepts[]: 얼마를 누구에게 어느 토큰으로) 반환
//   2. 우리가 지원하는 요구(exact·Base Sepolia USDC)를 골라
//      GUI에 "서명 요청" → 사람이 비번 승인 → EIP-3009 인가 서명을 받는다
//   3. 그 서명을 결제 헤더(base64 JSON)로 만들어 같은 URL을 재요청
//   4. 서버(+페이실리테이터)가 검증·온체인 정산 후 200 + 콘텐츠 반환
//
// 비밀은 여기 없다. 서명은 GUI 프로세스만(payment IPC), MCP는 HTTP·헤더 조립만 한다.
//
// V1 ↔ V2 차이 (실 www.x402.org 는 V2 — 개발 12에서 라이브 정산 검증 완료):
//   - 결제 요구 위치:  V1 = 응답 본문(JSON)        / V2 = `payment-required` 헤더(base64 JSON)
//   - 버전 필드:       V1 = x402Version:1          / V2 = x402Version:2
//   - 네트워크 표기:   V1 = "base-sepolia"         / V2 = "eip155:84532" (CAIP-2)
//   - 금액 필드:       V1 = maxAmountRequired      / V2 = amount
//   - 리소스/설명:     V1 = 요구별 문자열 필드     / V2 = 최상위 resource{url,description} 객체
//   - 제출 헤더:       V1 = `X-PAYMENT`            / V2 = `PAYMENT-SIGNATURE`
//   - 제출 payload:    V1 = {x402Version,scheme,network,payload}
//                      V2 = {x402Version,resource,accepted(선택한 요구 전체),payload}
//   - 정산 응답 헤더:  V1 = `X-PAYMENT-RESPONSE`   / V2 = `PAYMENT-RESPONSE`
// 자산(USDC 0x036C…)·서명(EIP-3009)·payload{signature,authorization} 구조는 양쪽 동일.
// V2 제출은 서버가 준 resource/accepted 를 raw 그대로 에코한다(extra·maxTimeoutSeconds 등 미지 필드 무손실).

use crate::tf;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chain::active_chain;

/// 우리가 지원하는 결제 스킴 (체인 무관 프로토콜 값).
pub const SCHEME: &str = "exact";

/// GUI가 서명해 돌려준 결제 인가(비밀 없음). sign_x402_payment 의 반환과 동일 형태.
#[derive(Serialize, Deserialize, Clone)]
pub struct X402Authorization {
    pub from: String,
    pub to: String,
    pub value: String,
    #[serde(rename = "validAfter")]
    pub valid_after: String,
    #[serde(rename = "validBefore")]
    pub valid_before: String,
    pub nonce: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct X402Payment {
    pub signature: String,
    pub authorization: X402Authorization,
}

/// 402 결제 요구 전체(원본 JSON 보존). version 만 미리 뽑아두고, 나머지는 raw 로 다룬다 —
/// 서버마다 다른 필드(extra/maxTimeoutSeconds/mimeType 등)를 잃지 않고 V2 제출 때 그대로 에코하려고.
pub struct PaymentRequired {
    raw: Value,
    /// 보낼 결제 헤더에 그대로 되돌려줄 버전. 없으면 1(V1).
    pub version: u8,
}

/// accepts[] 중 우리가 고른 결제 요구 1건. raw = 원본 항목(V2 제출 때 통째로 에코).
pub struct Requirement {
    pub raw: Value,
    pub scheme: String,
    pub network: String,
    /// base unit 금액("10000"). V2 "amount" / V1 "maxAmountRequired" 중 있는 쪽.
    pub amount: String,
    pub pay_to: String,
}

/// 조립된 결제 제출(헤더 이름 + base64 값 + 정산 응답을 읽을 헤더 이름). V1/V2가 다르다.
pub struct Submission {
    pub header_name: &'static str,
    pub value: String,
    pub response_header: &'static str,
}

/// 네트워크 표기가 우리가 지원하는 활성 체인인지 (V1 단축명/V2 CAIP-2 둘 다 허용).
/// V1 단축명이 없는 체인(Arc)은 CAIP-2 로만 매칭한다. 빈 표기는 항상 불일치 — `network` 를
/// 아예 안 준 요구가 "빈 문자열끼리 같다"로 통과하면 체인 검사가 통째로 무력해진다.
fn network_supported(raw: &str) -> bool {
    let chain = active_chain();
    let n = raw.trim();
    if n.is_empty() {
        return false;
    }
    chain
        .x402_network_v1
        .is_some_and(|v1| n.eq_ignore_ascii_case(v1))
        || n.eq_ignore_ascii_case(chain.x402_network_caip2)
}

/// **서버가 지정한 서명 도메인이 우리가 실제로 서명할 도메인과 같은가** (개발 50).
///
/// x402 요구의 `extra` 는 "이 EIP-712 도메인에 서명하라"는 지시다. 우리는 언제나 **활성 체인의
/// USDC(EIP-3009) 도메인**에만 서명하는데, 지금까지는 scheme·network·asset 세 개만 보고 골라서
/// **다른 도메인을 요구하는 서버의 요구도 "지원함"으로 집어 들었다**. 그러면 사람이 승인 창까지 보고
/// 비번을 넣은 뒤, 서버가 검증에 실패해 조용히 거절된다 — 최악의 실패 모드(돈은 안 나가지만 사용자는
/// 왜 안 되는지 모른다).
///
/// 실물 예 (개발 50, Circle Gateway 테스트넷 페이실리테이터 `/v1/x402/supported` 실응답):
/// Arc·Base Sepolia 등에서 `scheme:"exact"`, 우리와 **같은 USDC 주소**로 제시하면서
/// `extra:{name:"GatewayWalletBatched", version:"1", verifyingContract:"0x0077…19b9"}` 를 준다.
/// 앞의 세 필드만 보면 정확히 통과하는 요구다 → 이 가드가 없으면 그대로 잘못 서명한다.
///
/// 판정은 **있는 필드만** 본다(없으면 우리 기본 도메인이라는 뜻으로 받아들인다) — 여태 잘 돌던
/// `extra` 없는 서버·`extra` 에 다른 것만 담은 서버를 새로 깨뜨리지 않으려고.
fn extra_domain_ok(entry: &Value) -> bool {
    let Some(extra) = entry.get("extra") else {
        return true;
    };
    let chain = active_chain();
    // name/version 은 EIP-712 도메인 문자열이라 대소문자까지 정확히 같아야 서명이 맞는다
    // ("USDC" vs "USD Coin" 이 체인마다 다른 것과 같은 이유 — 한 글자 다르면 다른 도메인이다).
    if let Some(name) = str_field(extra, "name") {
        if name != chain.usdc_eip712_name {
            return false;
        }
    }
    if let Some(version) = str_field(extra, "version") {
        if version != chain.usdc_eip712_version {
            return false;
        }
    }
    // 주소만 체크섬 대소문자를 흡수해 비교한다.
    if let Some(vc) = str_field(extra, "verifyingContract") {
        if vc.trim().to_lowercase() != chain.usdc_address.to_string().to_lowercase() {
            return false;
        }
    }
    true
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// 402 응답에서 결제 요구를 추출한다. V2는 `payment-required` 헤더(base64 JSON)를,
/// 없으면 V1처럼 응답 본문(JSON)을 파싱한다.
pub fn parse_required(header: Option<&str>, body: &str) -> Result<PaymentRequired, String> {
    let raw: Value = match header.map(str::trim).filter(|s| !s.is_empty()) {
        Some(h) => {
            let bytes = B64.decode(h).map_err(|e| {
                tf!(
                    "payment-required 헤더 base64 디코드 실패: {e}",
                    "Couldn't base64-decode the payment-required header: {e}"
                )
            })?;
            serde_json::from_slice(&bytes).map_err(|e| {
                tf!(
                    "payment-required 헤더 JSON 파싱 실패: {e}",
                    "Couldn't parse the payment-required header as JSON: {e}"
                )
            })?
        }
        None => serde_json::from_str(body).map_err(|e| {
            tf!(
                "402 본문 파싱 실패: {e}",
                "Couldn't parse the 402 body: {e}"
            )
        })?,
    };
    let version = raw.get("x402Version").and_then(Value::as_u64).unwrap_or(1) as u8;
    Ok(PaymentRequired { raw, version })
}

/// accepts[] 중 우리가 처리할 수 있는 요구(exact·Base Sepolia·USDC)를 고른다.
/// 대소문자·체크섬·네트워크 표기(V1/V2) 차이를 흡수해 비교한다. (예: solana 옵션은 건너뜀)
pub fn pick_requirement(pr: &PaymentRequired) -> Result<Requirement, String> {
    let usdc_lower = active_chain().usdc_address.to_string().to_lowercase();
    let accepts = pr.raw.get("accepts").and_then(Value::as_array);
    if let Some(list) = accepts {
        for entry in list {
            let scheme = str_field(entry, "scheme").unwrap_or("");
            let network = str_field(entry, "network").unwrap_or("");
            let asset = str_field(entry, "asset").unwrap_or("");
            if scheme.eq_ignore_ascii_case(SCHEME)
                && network_supported(network)
                && asset.to_lowercase() == usdc_lower
                && extra_domain_ok(entry)
            {
                let amount = str_field(entry, "amount")
                    .or_else(|| str_field(entry, "maxAmountRequired"))
                    .unwrap_or("")
                    .to_string();
                return Ok(Requirement {
                    raw: entry.clone(),
                    scheme: scheme.to_string(),
                    network: network.to_string(),
                    amount,
                    pay_to: str_field(entry, "payTo").unwrap_or("").to_string(),
                });
            }
        }
    }
    // 제시 목록에 **왜 못 골랐는지**가 드러나게 적는다. 특히 scheme/network/asset 이 전부 맞는데
    // extra 도메인만 다른 경우(Circle Gateway 등)는 세 값만 찍으면 "맞는데 왜 안 되지"로 읽힌다.
    let offered: Vec<String> = accepts
        .map(|l| {
            l.iter()
                .map(|e| {
                    let base = format!(
                        "{}/{}/{}",
                        str_field(e, "scheme").unwrap_or("?"),
                        str_field(e, "network").unwrap_or("?"),
                        str_field(e, "asset").unwrap_or("?")
                    );
                    if extra_domain_ok(e) {
                        base
                    } else {
                        let name = e
                            .get("extra")
                            .and_then(|x| str_field(x, "name"))
                            .unwrap_or("?");
                        tf!(
                            "{base} (서명 도메인이 {name} — 우리는 USDC 에 서명해요)",
                            "{base} (asks to sign the {name} domain — Kura signs USDC itself)"
                        )
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let chain = active_chain();
    Err(tf!(
        "지원하는 결제 요구가 없어요. 우리는 exact 스킴 · {} · 그 체인의 USDC · USDC 자체 서명(EIP-3009)만 지원합니다. 서버 제시: [{}]",
        "No supported payment requirement. Kura supports the exact scheme on {} with that chain's USDC, signed against USDC itself (EIP-3009). Server offered: [{}]",
        chain.x402_network_caip2,
        offered.join(", ")
    ))
}

impl PaymentRequired {
    /// 결제 사유 후보(없으면 빈 문자열): 요구별 설명(V1) > 최상위 설명(V2).
    pub fn description(&self, req: &Requirement) -> String {
        if let Some(s) = str_field(&req.raw, "description") {
            if !s.trim().is_empty() {
                return s.trim().to_string();
            }
        }
        self.raw
            .get("resource")
            .and_then(|r| r.get("description"))
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }

    /// 서명된 결제를 제출 헤더로 조립한다. 버전에 따라 헤더 이름과 payload 구조가 다르다.
    ///   V2: PAYMENT-SIGNATURE = base64({x402Version, resource, accepted=요구 raw, payload})
    ///   V1: X-PAYMENT         = base64({x402Version, scheme, network, payload})
    pub fn build_submission(
        &self,
        req: &Requirement,
        payment: &X402Payment,
    ) -> Result<Submission, String> {
        let (json, header_name, response_header) = if self.version >= 2 {
            let body = serde_json::json!({
                "x402Version": self.version,
                "resource": self.raw.get("resource").cloned().unwrap_or(Value::Null),
                "accepted": req.raw,
                "payload": payment,
            });
            (body, "PAYMENT-SIGNATURE", "PAYMENT-RESPONSE")
        } else {
            let body = serde_json::json!({
                "x402Version": self.version,
                "scheme": req.scheme,
                "network": req.network,
                "payload": payment,
            });
            (body, "X-PAYMENT", "X-PAYMENT-RESPONSE")
        };
        let bytes = serde_json::to_vec(&json).map_err(|e| {
            tf!(
                "payload 직렬화 실패: {e}",
                "Couldn't serialize the payload: {e}"
            )
        })?;
        Ok(Submission {
            header_name,
            value: B64.encode(bytes),
            response_header,
        })
    }
}

/// base unit 정수 문자열("10000")을 USDC 십진 문자열("0.01")로 — 사람 표시·송금 한도 검사용.
pub fn base_units_to_usdc(base: &str) -> Result<String, String> {
    let dec = active_chain().usdc_decimals as usize;
    let scale = 10u128.pow(dec as u32);
    let n: u128 = base.trim().parse().map_err(|_| {
        tf!(
            "금액 형식 오류: {base}",
            "That amount isn't a valid number: {base}"
        )
    })?;
    let whole = n / scale;
    let frac = n % scale;
    if frac == 0 {
        Ok(whole.to_string())
    } else {
        let s = format!("{whole}.{frac:0width$}", width = dec);
        Ok(s.trim_end_matches('0').trim_end_matches('.').to_string())
    }
}

/// 정산 응답(PAYMENT-RESPONSE / X-PAYMENT-RESPONSE, base64 JSON)에서 정산 tx 해시와 성공여부를
/// 뽑는다. 필드명은 구현마다 "transaction"(V2 표준) 또는 "txHash"(V1)일 수 있어 둘 다 본다.
/// tx 해시를 못 찾으면 None(표시할 게 없음).
pub fn parse_settlement(b64: &str) -> Option<(String, bool)> {
    let bytes = B64.decode(b64.trim()).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    let tx = v
        .get("transaction")
        .or_else(|| v.get("txHash"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let success = v.get("success").and_then(Value::as_bool).unwrap_or(true);
    Some((tx, success))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// V1 응답 본문 (Session 12 형태): 본문 JSON + base-sepolia + maxAmountRequired + 요구별 resource.
    fn sample_v1_body() -> &'static str {
        r#"{
          "x402Version": 1,
          "accepts": [
            {
              "scheme": "exact",
              "network": "base-sepolia",
              "maxAmountRequired": "10000",
              "resource": "https://example.com/data",
              "description": "프리미엄 데이터",
              "payTo": "0x1111111111111111111111111111111111111111",
              "asset": "0x036CbD53842c5426634e7929541eC2318f3dCF7e",
              "maxTimeoutSeconds": 60,
              "extra": { "name": "USDC", "version": "2" }
            }
          ]
        }"#
    }

    /// V2 응답 (실 www.x402.org/protected 실측 형태): eip155:84532 + amount + 최상위 resource + solana 옵션 동봉.
    fn sample_v2_json() -> &'static str {
        r#"{
          "x402Version": 2,
          "error": "Payment required",
          "resource": { "url": "https://www.x402.org/protected", "description": "Access to protected content", "mimeType": "" },
          "accepts": [
            { "scheme": "exact", "network": "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1", "amount": "10000",
              "asset": "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU", "payTo": "CKPKsol", "maxTimeoutSeconds": 300 },
            { "scheme": "exact", "network": "eip155:84532", "amount": "10000",
              "asset": "0x036CbD53842c5426634e7929541eC2318f3dCF7e",
              "payTo": "0x209693Bc6afc0C5328bA36FaF03C514EF312287C", "maxTimeoutSeconds": 300,
              "extra": { "name": "USDC", "version": "2" } }
          ]
        }"#
    }

    fn sample_payment() -> X402Payment {
        X402Payment {
            signature: "0xabcd".into(),
            authorization: X402Authorization {
                from: "0xaaa".into(),
                to: "0xbbb".into(),
                value: "10000".into(),
                valid_after: "0".into(),
                valid_before: "9999999999".into(),
                nonce: "0x1234".into(),
            },
        }
    }

    fn decode(value: &str) -> Value {
        serde_json::from_slice(&B64.decode(value).unwrap()).unwrap()
    }

    /// 🔴 개발 50 — **서버가 지정한 서명 도메인이 우리 것과 다르면 고르지 않는다.**
    /// 표본은 Circle Gateway 테스트넷 페이실리테이터의 실응답 형태다(`/v1/x402/supported`):
    /// scheme·network·asset 은 우리와 정확히 같고 `extra` 만 GatewayWallet 도메인을 가리킨다.
    /// 가드가 없으면 사람이 비번까지 넣은 뒤 서버가 조용히 거절한다.
    #[test]
    fn reject_requirement_asking_for_another_signing_domain() {
        let body = r#"{
          "x402Version": 2,
          "accepts": [
            { "scheme": "exact", "network": "eip155:84532", "amount": "10000",
              "asset": "0x036CbD53842c5426634e7929541eC2318f3dCF7e",
              "payTo": "0x1111111111111111111111111111111111111111",
              "extra": { "name": "GatewayWalletBatched", "version": "1",
                         "verifyingContract": "0x0077777d7eba4688bdef3e311b846f25870a19b9" } }
          ]
        }"#;
        let pr = parse_required(None, body).unwrap();
        let err = match pick_requirement(&pr) {
            Ok(_) => panic!("Gateway 도메인 요구를 골랐다 — 가드가 안 걸렸다"),
            Err(e) => e,
        };
        // 왜 못 골랐는지가 문구에 남아야 한다 — 세 값만 찍으면 "다 맞는데 왜"로 읽힌다.
        assert!(err.contains("GatewayWalletBatched"), "{err}");
    }

    /// extra 가 아예 없거나 우리 도메인과 같으면 예전처럼 통과한다(회귀 방지 — 대부분의 서버가 이쪽).
    #[test]
    fn extra_absent_or_matching_still_passes() {
        let no_extra = r#"{"x402Version":1,"accepts":[
          {"scheme":"exact","network":"base-sepolia","maxAmountRequired":"1",
           "payTo":"0x1111111111111111111111111111111111111111",
           "asset":"0x036CbD53842c5426634e7929541eC2318f3dCF7e"}]}"#;
        assert!(pick_requirement(&parse_required(None, no_extra).unwrap()).is_ok());
        // 체크섬 대소문자만 다른 verifyingContract 는 같은 주소다 — 주소만 대소문자를 흡수한다.
        let checksum = r#"{"x402Version":1,"accepts":[
          {"scheme":"exact","network":"base-sepolia","maxAmountRequired":"1",
           "payTo":"0x1111111111111111111111111111111111111111",
           "asset":"0x036CbD53842c5426634e7929541eC2318f3dCF7e",
           "extra":{"name":"USDC","version":"2",
                    "verifyingContract":"0x036cbd53842c5426634e7929541ec2318f3dcf7e"}}]}"#;
        assert!(pick_requirement(&parse_required(None, checksum).unwrap()).is_ok());
    }

    /// network 를 아예 안 준 요구가 통과하면 체인 검사가 통째로 무력해진다 (Option 전환 시 실수하기 쉬운 곳).
    #[test]
    fn empty_network_is_not_supported() {
        assert!(!network_supported(""));
        assert!(!network_supported("   "));
        assert!(network_supported("base-sepolia")); // 테스트 기본 체인
        assert!(network_supported("eip155:84532"));
    }

    /// V1: 본문 파싱(헤더 없음) + 요구 선택 + 표시 정보.
    #[test]
    fn v1_body_parse_and_pick() {
        let pr = parse_required(None, sample_v1_body()).unwrap();
        assert_eq!(pr.version, 1);
        let req = pick_requirement(&pr).unwrap();
        assert_eq!(req.scheme, "exact");
        assert_eq!(req.amount, "10000");
        assert_eq!(req.pay_to, "0x1111111111111111111111111111111111111111");
        assert_eq!(pr.description(&req), "프리미엄 데이터");
        // 요구의 `resource` 문자열은 **읽지 않는다**(개발 51) — 승인 창에 보이는 URL 은 우리가
        // 실제로 요청한 최종 URL 이다. 서버 주장값을 표시에 쓰면 신뢰 도메인 사칭이 된다.
    }

    /// V2: payment-required 헤더(base64)에서 파싱 + eip155:84532 요구 선택(solana는 건너뜀).
    #[test]
    fn v2_header_parse_and_pick() {
        let header = B64.encode(sample_v2_json());
        let pr = parse_required(Some(&header), "").unwrap();
        assert_eq!(pr.version, 2);
        let req = pick_requirement(&pr).unwrap();
        assert_eq!(req.network, "eip155:84532"); // solana 가 아니라 EVM 을 골라야 한다
        assert_eq!(req.amount, "10000"); // "amount" 필드도 읽힌다
        assert_eq!(req.pay_to, "0x209693Bc6afc0C5328bA36FaF03C514EF312287C");
        assert_eq!(pr.description(&req), "Access to protected content");
    }

    /// 헤더가 있으면 본문보다 헤더를 우선한다.
    #[test]
    fn header_takes_precedence_over_body() {
        let header = B64.encode(sample_v2_json());
        let pr = parse_required(Some(&header), "not json").unwrap();
        assert_eq!(pr.version, 2);
    }

    /// 체크섬(대문자 섞인) asset 주소도 매칭돼야 한다.
    #[test]
    fn pick_matches_checksummed_asset() {
        let pr = parse_required(None, sample_v1_body()).unwrap();
        assert!(pick_requirement(&pr).is_ok());
    }

    /// 지원하지 않는 네트워크(base 메인넷)면 거른다.
    #[test]
    fn reject_unsupported_network() {
        let body = r#"{"x402Version":1,"accepts":[
          {"scheme":"exact","network":"base","maxAmountRequired":"10000",
           "payTo":"0x1","asset":"0x036cbd53842c5426634e7929541ec2318f3dcf7e"}]}"#;
        let pr = parse_required(None, body).unwrap();
        assert!(pick_requirement(&pr).is_err());
    }

    /// solana만 제시되면(EVM 없음) 거른다.
    #[test]
    fn reject_solana_only() {
        let body = r#"{"x402Version":2,"accepts":[
          {"scheme":"exact","network":"solana:Et","amount":"10000",
           "payTo":"CK","asset":"4zMMC"}]}"#;
        let pr = parse_required(None, body).unwrap();
        assert!(pick_requirement(&pr).is_err());
    }

    /// 정산 응답 파싱: V2 "transaction" / V1 "txHash" 둘 다, success 기본 true, tx 없으면 None.
    #[test]
    fn settlement_parse() {
        let v2 =
            B64.encode(r#"{"success":true,"transaction":"0xSETTLE","network":"eip155:84532"}"#);
        assert_eq!(parse_settlement(&v2), Some(("0xSETTLE".into(), true)));
        let v1 = B64.encode(r#"{"success":false,"txHash":"0xT1"}"#);
        assert_eq!(parse_settlement(&v1), Some(("0xT1".into(), false)));
        // success 필드 없으면 true 로 본다(정산 응답 헤더가 왔다는 건 보통 성공).
        let no_succ = B64.encode(r#"{"transaction":"0xT2"}"#);
        assert_eq!(parse_settlement(&no_succ), Some(("0xT2".into(), true)));
        // tx 없음 → None
        let no_tx = B64.encode(r#"{"success":true}"#);
        assert_eq!(parse_settlement(&no_tx), None);
        // 깨진 base64 → None
        assert_eq!(parse_settlement("!!notb64!!"), None);
    }

    /// base unit → USDC 십진 변환.
    #[test]
    fn base_units_format() {
        assert_eq!(base_units_to_usdc("10000").unwrap(), "0.01");
        assert_eq!(base_units_to_usdc("1000000").unwrap(), "1");
        assert_eq!(base_units_to_usdc("1500000").unwrap(), "1.5");
        assert_eq!(base_units_to_usdc("1").unwrap(), "0.000001");
        assert_eq!(base_units_to_usdc("0").unwrap(), "0");
    }

    /// V1 제출: X-PAYMENT 헤더 + {x402Version:1, scheme, network, payload}.
    #[test]
    fn submission_v1() {
        let pr = parse_required(None, sample_v1_body()).unwrap();
        let req = pick_requirement(&pr).unwrap();
        let sub = pr.build_submission(&req, &sample_payment()).unwrap();
        assert_eq!(sub.header_name, "X-PAYMENT");
        assert_eq!(sub.response_header, "X-PAYMENT-RESPONSE");
        let v = decode(&sub.value);
        assert_eq!(v["x402Version"], 1);
        assert_eq!(v["scheme"], "exact");
        assert_eq!(v["network"], "base-sepolia");
        assert_eq!(v["payload"]["authorization"]["value"], "10000");
        assert!(v.get("accepted").is_none()); // V1엔 accepted 없음
    }

    /// V2 제출: PAYMENT-SIGNATURE 헤더 + {x402Version:2, resource, accepted(요구 전체), payload}.
    #[test]
    fn submission_v2() {
        let header = B64.encode(sample_v2_json());
        let pr = parse_required(Some(&header), "").unwrap();
        let req = pick_requirement(&pr).unwrap();
        let sub = pr.build_submission(&req, &sample_payment()).unwrap();
        assert_eq!(sub.header_name, "PAYMENT-SIGNATURE");
        assert_eq!(sub.response_header, "PAYMENT-RESPONSE");
        let v = decode(&sub.value);
        assert_eq!(v["x402Version"], 2);
        // 최상위 resource 를 그대로 에코.
        assert_eq!(v["resource"]["url"], "https://www.x402.org/protected");
        // 선택한 요구(EVM)를 통째로 에코 — extra 등 미지 필드도 보존.
        assert_eq!(v["accepted"]["network"], "eip155:84532");
        assert_eq!(v["accepted"]["amount"], "10000");
        assert_eq!(v["accepted"]["extra"]["version"], "2");
        assert_eq!(v["payload"]["signature"], "0xabcd");
        assert_eq!(v["payload"]["authorization"]["value"], "10000");
    }
}
