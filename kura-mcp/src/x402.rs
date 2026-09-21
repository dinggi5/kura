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

use crate::arc_direct::{DirectProof, NonceBinding, METHOD_CLIENT_BROADCAST};
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

/// 이 요구를 **누가 온체인에 올리는가** (`extra.assetTransferMethod`, x402 제안 #3504).
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum TransferMethod {
    /// 표준 `exact` — 우리는 서명만, 제출·가스는 페이실리테이터. 필드가 없으면 이것(기존 동작).
    Facilitator,
    /// `eip3009-client-broadcast` — **우리가 직접 올리고 우리 USDC 로 가스를 낸다**(개발 64).
    /// 가스가 곧 결제자산인 체인(Arc)에서만 성립한다.
    ClientBroadcast,
}

/// accepts[] 중 우리가 고른 결제 요구 1건. raw = 원본 항목(V2 제출 때 통째로 에코).
pub struct Requirement {
    pub raw: Value,
    pub scheme: String,
    pub network: String,
    /// base unit 금액("10000"). V2 "amount" / V1 "maxAmountRequired" 중 있는 쪽.
    pub amount: String,
    pub pay_to: String,
    /// 이 체인의 USDC 주소(서버가 준 표기 그대로 — nonce 바인딩이 이 문자열을 쓴다).
    pub asset: String,
    /// 누가 올리는가. 갈래가 여기서 갈린다.
    pub method: TransferMethod,
    /// `extra.resource` — **nonce 계산 전용**(표시용 URL 이 아니다, 개발 51).
    pub extra_resource: String,
    /// seed 모드 서버가 준 seed(있으면 그 모드).
    pub seed: Option<String>,
    /// 서버가 미리 게시한 nonce(있으면 우리 계산과 대조한다).
    pub published_nonce: Option<String>,
}

impl Requirement {
    /// nonce 가 묶이는 조각 — **서버가 준 값 그대로**(자세한 이유는 arc_direct 모듈 머리말).
    pub fn binding(&self) -> NonceBinding {
        NonceBinding {
            network: self.network.clone(),
            asset: self.asset.clone(),
            pay_to: self.pay_to.clone(),
            amount: self.amount.clone(),
            resource: self.extra_resource.clone(),
        }
    }
}

/// 조립된 결제 제출(헤더 이름 + base64 값 + 정산 응답을 읽을 헤더 이름). V1/V2가 다르다.
pub struct Submission {
    pub header_name: &'static str,
    pub value: String,
    pub response_header: &'static str,
    /// 같은 값을 함께 실어 줄 두 번째 헤더(없으면 None). 직접 제출 규격의 서버들이 V2 본문으로
    /// 챌린지를 주면서 헤더는 `X-PAYMENT` 로 읽는 경우가 있어(레퍼런스 서버가 그렇다), 그 갈래에선
    /// 둘 다 보낸다 — 값이 같으므로 양쪽 다 읽는 서버에도 무해하다(상대 구현의 클라이언트도 그렇게 한다).
    pub alt_header: Option<&'static str>,
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

/// **이 요구를 누가 온체인에 올리는가** (개발 64). `extra.assetTransferMethod` 를 본다.
///
/// - 필드 없음 → `Facilitator` (여태 우리가 해 온 것. 대부분의 서버가 여기).
/// - `eip3009-client-broadcast` → `ClientBroadcast` (우리가 직접 올린다).
/// - **그 밖의 값 → `None` = 이 요구는 못 고른다.**
///
/// 마지막 갈래가 이 함수의 존재 이유다. 모르는 방식을 「어차피 exact 니까」 하고 집어 들면,
/// 사람이 승인 창까지 보고 비번을 넣은 뒤 서버가 우리 헤더를 못 읽어 조용히 거절한다 —
/// 개발 50 이 `extra` 도메인 가드로 막았던 바로 그 실패 모드가 **다른 필드로** 다시 열린다.
/// 실제로 개발 64 직전까지 우리는 `eip3009-client-broadcast` 요구(stockwaves.net 실물)를 그대로
/// 골라 서명하고 있었다: 돈은 안 나가지만 일일 한도는 깎이고 내역엔 "signed" 가 남았다.
/// (`native_is_usdc` 를 인자로 받는 이유: 이 판정은 체인마다 답이 달라야 하는데, 단위 테스트는
/// 활성 체인이 Base Sepolia 로 고정돼 있어 «Arc 에선 고른다» 를 검사할 길이 없다. 순수 함수로
/// 두면 양쪽을 다 문다.)
fn transfer_method(entry: &Value, native_is_usdc: bool) -> Option<TransferMethod> {
    let raw = entry
        .get("extra")
        .and_then(|x| str_field(x, "assetTransferMethod"))
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match raw {
        None => Some(TransferMethod::Facilitator),
        Some(m) if m.eq_ignore_ascii_case(METHOD_CLIENT_BROADCAST) => {
            // 🔴 가스가 곧 결제자산인 체인에서만 성립한다(제안의 전제). Base 에서 이걸 고르면
            // 우리 ETH 로 가스를 내야 하는데, x402 경로엔 ETH 회계가 없다 → 고르지 않는다.
            native_is_usdc.then_some(TransferMethod::ClientBroadcast)
        }
        Some(_) => None,
    }
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
            let Some(method) = transfer_method(entry, active_chain().native_is_usdc) else {
                continue; // 우리가 못 내는 방식 — 서명해 봐야 서버가 못 쓴다
            };
            if scheme.eq_ignore_ascii_case(SCHEME)
                && network_supported(network)
                && asset.to_lowercase() == usdc_lower
                && extra_domain_ok(entry)
            {
                let amount = str_field(entry, "amount")
                    .or_else(|| str_field(entry, "maxAmountRequired"))
                    .unwrap_or("")
                    .to_string();
                // 🔴 직접 제출은 **`amount` 필드가 있어야만** 고른다 (개발 64 리뷰 P3). nonce 는
                // 서버가 검증 때 `requirements.amount` 로 다시 만드는데, 우리가 V1 의
                // `maxAmountRequired` 로 계산하면 **그 값이 서로 다르다** — 돈은 나가고 서버는
                // 「이 결제가 아니다」로 거절한다. 서명 갈래는 예전처럼 폴백을 쓴다(돈이 안 나간다).
                if method == TransferMethod::ClientBroadcast && str_field(entry, "amount").is_none()
                {
                    continue;
                }
                let extra = entry.get("extra");
                return Ok(Requirement {
                    raw: entry.clone(),
                    scheme: scheme.to_string(),
                    network: network.to_string(),
                    amount,
                    pay_to: str_field(entry, "payTo").unwrap_or("").to_string(),
                    asset: asset.to_string(),
                    method,
                    // nonce 바인딩은 **서버 문자열 그대로**(없으면 빈 문자열 — 상대 구현도 같은 규칙).
                    extra_resource: extra
                        .and_then(|x| str_field(x, "resource"))
                        .unwrap_or("")
                        .to_string(),
                    seed: extra.and_then(|x| str_field(x, "seed")).map(str::to_string),
                    published_nonce: extra
                        .and_then(|x| str_field(x, "nonce"))
                        .map(str::to_string),
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
                    if transfer_method(e, active_chain().native_is_usdc).is_none() {
                        let m = e
                            .get("extra")
                            .and_then(|x| str_field(x, "assetTransferMethod"))
                            .unwrap_or("?");
                        // 「방식은 아는데 이 체인이 아닌」 경우와 「아예 모르는 방식」을 한 문구로 묶는다 —
                        // 둘 다 사용자가 할 수 있는 일은 없고, 알아야 할 것은 «왜 못 냈나» 뿐이다.
                        tf!(
                            "{base} (전송 방식 {m} — 이 체인에선 우리가 낼 수 없어요)",
                            "{base} (asset transfer method {m} — Kura can't pay that on this chain)"
                        )
                    } else if extra_domain_ok(e) {
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
            alt_header: None,
        })
    }
}

impl PaymentRequired {
    /// **직접 제출**(client-broadcast)의 제출 헤더 — payload 가 서명이 아니라 **우리가 올린 tx 해시**다.
    ///
    /// 모양은 규격(그리고 상대 구현의 `toPaymentHeader`)대로 언제나 V2 다: `{x402Version:2, resource,
    /// accepted, payload}`. 서버가 챌린지를 V1 본문으로 줬더라도 이 갈래는 V2 규격에만 정의돼 있다.
    /// `accepted` 는 서버가 준 요구 raw 를 통째로 에코한다 — 서버가 검증 때 자기 요구사항을 다시 만들어
    /// `required.extra ⊆ accepted.extra` 로 맞춰 보기 때문에, 한 필드라도 빠지면 「맞는 요구가 없다」가 된다.
    pub fn build_direct_submission(
        &self,
        req: &Requirement,
        proof: &DirectProof,
    ) -> Result<Submission, String> {
        let mut body = serde_json::json!({
            "x402Version": 2,
            "accepted": req.raw,
            "payload": proof.payload(),
        });
        // `resource` 는 **있을 때만** 싣는다 — 챌린지에 없으면 키를 통째로 뺀다(상대 구현의
        // `toPaymentHeader` 와 같은 규칙). 명시적 `null` 은 「선택 필드가 없음」과 다른 값이라,
        // 스키마를 엄격히 보는 서버에선 타입 불일치로 튕길 수 있다(코드 리뷰 P2).
        if let Some(resource) = self.raw.get("resource") {
            if !resource.is_null() {
                body["resource"] = resource.clone();
            }
        }
        let bytes = serde_json::to_vec(&body).map_err(|e| {
            tf!(
                "payload 직렬화 실패: {e}",
                "Couldn't serialize the payload: {e}"
            )
        })?;
        Ok(Submission {
            header_name: "PAYMENT-SIGNATURE",
            value: B64.encode(bytes),
            response_header: "PAYMENT-RESPONSE",
            // 레퍼런스 서버(kaditang/x402-arc 의 examples/server.ts)는 챌린지를 V2 로 주면서 헤더는
            // `X-PAYMENT` 로 읽는다 — 상대 구현의 클라이언트도 그래서 둘 다 보낸다. 값이 같다.
            alt_header: Some("X-PAYMENT"),
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

    /// 🔴 개발 64 — **`assetTransferMethod` 를 모르면 고르지 않는다(닫히는 쪽으로 실패).**
    ///
    /// 표본은 stockwaves.net 의 실제 402 챌린지 모양이다(개발 64 에서 받아 확인). scheme·network·
    /// asset·extra 도메인이 **전부 우리와 같다** — 개발 50 가드는 그대로 통과한다. 다른 것은
    /// `assetTransferMethod` 하나뿐이고, 그 하나가 「누가 온체인에 올리나」를 통째로 바꾼다.
    /// 이 가드가 없으면 사람이 비번을 넣어 서명한 뒤 서버가 그 헤더를 못 읽는다(돈은 안 나가고
    /// 일일 한도만 깎인다) — 개발 64 직전까지 실제로 그 상태였다.
    #[test]
    fn unknown_transfer_method_is_not_picked() {
        let body = r#"{"x402Version":2,"accepts":[
          {"scheme":"exact","network":"base-sepolia","amount":"10000",
           "payTo":"0x1111111111111111111111111111111111111111",
           "asset":"0x036CbD53842c5426634e7929541eC2318f3dCF7e",
           "extra":{"name":"USDC","version":"2","assetTransferMethod":"some-future-scheme"}}]}"#;
        let pr = parse_required(None, body).unwrap();
        let err = match pick_requirement(&pr) {
            Ok(_) => panic!("모르는 전송 방식을 골랐다"),
            Err(e) => e,
        };
        // 왜 못 골랐는지가 문구에 남아야 한다 — 「다 맞는데 왜」로 읽히면 안 된다.
        assert!(err.contains("some-future-scheme"), "{err}");
    }

    /// 직접 제출은 **가스가 곧 결제자산인 체인에서만** 고른다. Base 에서 고르면 우리 ETH 로
    /// 가스를 내야 하는데 x402 경로엔 그 회계가 없다 → 체인이 아니면 안 고른다.
    #[test]
    fn client_broadcast_only_where_gas_is_usdc() {
        let entry: Value = serde_json::from_str(
            r#"{"scheme":"exact","network":"eip155:5042","amount":"30000",
                "payTo":"0xDc9F94A8b93F070B58cfa580cbE740d763005FE6",
                "asset":"0x3600000000000000000000000000000000000000",
                "extra":{"name":"USDC","version":"2","assetTransferMethod":"eip3009-client-broadcast"}}"#,
        )
        .unwrap();
        assert_eq!(
            transfer_method(&entry, true),
            Some(TransferMethod::ClientBroadcast)
        );
        assert_eq!(transfer_method(&entry, false), None); // Base 계열 = 안 고른다

        // 대소문자는 흡수한다(프로토콜 문자열이지 서명 도메인이 아니다).
        let upper: Value =
            serde_json::from_str(r#"{"extra":{"assetTransferMethod":"EIP3009-Client-Broadcast"}}"#)
                .unwrap();
        assert_eq!(
            transfer_method(&upper, true),
            Some(TransferMethod::ClientBroadcast)
        );
        // 필드가 없거나 비었으면 예전 그대로(페이실리테이터 정산) — 서버 대다수가 이쪽이다.
        let plain: Value = serde_json::from_str(r#"{"extra":{"name":"USDC"}}"#).unwrap();
        assert_eq!(
            transfer_method(&plain, true),
            Some(TransferMethod::Facilitator)
        );
        let no_extra: Value = serde_json::from_str(r#"{"scheme":"exact"}"#).unwrap();
        assert_eq!(
            transfer_method(&no_extra, false),
            Some(TransferMethod::Facilitator)
        );
        let blank: Value =
            serde_json::from_str(r#"{"extra":{"assetTransferMethod":"  "}}"#).unwrap();
        assert_eq!(
            transfer_method(&blank, true),
            Some(TransferMethod::Facilitator)
        );
    }

    /// 🔴 개발 64 리뷰 — 직접 제출 요구가 **V1 금액 필드만** 갖고 있으면 고르지 않는다.
    /// 서버는 검증 때 `requirements.amount` 로 nonce 를 다시 만드는데 우리가 `maxAmountRequired`
    /// 로 계산하면 값이 갈린다 → **돈은 나가고 서버는 「이 결제가 아니다」**. 서명 갈래는 폴백을
    /// 그대로 쓴다(거절당해도 돈이 안 나간다) — 그 회귀가 안 나게 같이 문다.
    #[test]
    fn client_broadcast_requires_the_v2_amount_field() {
        let entry: Value = serde_json::from_str(
            r#"{"scheme":"exact","network":"eip155:5042","maxAmountRequired":"30000",
                "payTo":"0xDc9F94A8b93F070B58cfa580cbE740d763005FE6",
                "asset":"0x3600000000000000000000000000000000000000",
                "extra":{"assetTransferMethod":"eip3009-client-broadcast"}}"#,
        )
        .unwrap();
        // 방식 판정 자체는 통과한다 — 걸러지는 자리는 `pick_requirement` 다.
        assert_eq!(
            transfer_method(&entry, true),
            Some(TransferMethod::ClientBroadcast)
        );
        let body = format!(r#"{{"x402Version":2,"accepts":[{entry}]}}"#);
        let pr = parse_required(None, &body).unwrap();
        assert!(
            pick_requirement(&pr).is_err(),
            "amount 없는 직접 제출 요구를 골랐다"
        );
        // 서명 갈래(V1 base-sepolia)는 예전처럼 maxAmountRequired 로 통과한다.
        let v1 = r#"{"x402Version":1,"accepts":[
          {"scheme":"exact","network":"base-sepolia","maxAmountRequired":"10000",
           "payTo":"0x1111111111111111111111111111111111111111",
           "asset":"0x036CbD53842c5426634e7929541eC2318f3dCF7e"}]}"#;
        let req = pick_requirement(&parse_required(None, v1).unwrap()).unwrap();
        assert_eq!(req.amount, "10000");
    }

    /// 고른 요구에서 nonce 바인딩 재료가 그대로 나온다 — **서버가 준 문자열 그대로**여야 한다
    /// (한 글자만 다듬어도 서버가 기대하는 nonce 와 갈린다).
    #[test]
    fn requirement_carries_binding_fields() {
        let body = r#"{"x402Version":2,"accepts":[
          {"scheme":"exact","network":"base-sepolia","amount":"10000",
           "payTo":"0x1111111111111111111111111111111111111111",
           "asset":"0x036CbD53842c5426634e7929541eC2318f3dCF7e",
           "extra":{"name":"USDC","version":"2","resource":"https://ex.com/a","nonce":"0xaa"}}]}"#;
        let pr = parse_required(None, body).unwrap();
        let req = pick_requirement(&pr).unwrap();
        assert_eq!(req.method, TransferMethod::Facilitator);
        assert_eq!(req.extra_resource, "https://ex.com/a");
        assert_eq!(req.published_nonce.as_deref(), Some("0xaa"));
        assert!(req.seed.is_none());
        let b = req.binding();
        assert_eq!(b.amount, "10000");
        assert_eq!(b.asset, "0x036CbD53842c5426634e7929541eC2318f3dCF7e"); // 대소문자 그대로
        assert_eq!(b.resource, "https://ex.com/a");
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

    /// 🔴 개발 64 — 직접 제출 헤더는 챌린지에 `resource` 가 **없으면 키를 뺀다**(명시적 null 금지).
    /// 상대 구현의 `toPaymentHeader` 와 같은 규칙이다 — 「선택 필드가 없음」과 「값이 null」은 다른
    /// 값이라, 스키마를 엄격히 보는 서버에선 후자가 타입 불일치로 튕긴다(코드 리뷰 P2).
    #[test]
    fn direct_submission_omits_absent_resource() {
        use crate::arc_direct::DirectProof;
        let proof = || DirectProof {
            transaction: "0xTX".into(),
            client_nonce: Some("aabb".into()),
            seed: None,
            nonce: "0xNONCE".into(),
        };
        // 최상위 resource 가 없는 챌린지
        let bare = r#"{"x402Version":2,"accepts":[
          {"scheme":"exact","network":"base-sepolia","amount":"10000",
           "payTo":"0x1111111111111111111111111111111111111111",
           "asset":"0x036CbD53842c5426634e7929541eC2318f3dCF7e"}]}"#;
        let pr = parse_required(None, bare).unwrap();
        let req = pick_requirement(&pr).unwrap();
        let v = decode(&pr.build_direct_submission(&req, &proof()).unwrap().value);
        assert!(
            v.get("resource").is_none(),
            "resource 키가 null 로 실렸다: {v}"
        );
        assert_eq!(v["payload"]["transaction"], "0xTX");
        assert_eq!(v["x402Version"], 2);

        // 서버가 `"resource": null` 을 **명시적으로** 보낸 경우도 키를 뺀다(2차 리뷰 P3 — 코드는
        // 맞는데 검사가 없었다. 「없음」과 「null」을 다르게 다루는 게 이 함수의 요점이라 둘 다 문다).
        let explicit_null = r#"{"x402Version":2,"resource":null,"accepts":[
          {"scheme":"exact","network":"base-sepolia","amount":"10000",
           "payTo":"0x1111111111111111111111111111111111111111",
           "asset":"0x036CbD53842c5426634e7929541eC2318f3dCF7e"}]}"#;
        let pr = parse_required(None, explicit_null).unwrap();
        let req = pick_requirement(&pr).unwrap();
        let v = decode(&pr.build_direct_submission(&req, &proof()).unwrap().value);
        assert!(
            v.get("resource").is_none(),
            "명시적 null 이 그대로 실렸다: {v}"
        );

        // 있으면 그대로 에코한다
        let with_res = r#"{"x402Version":2,"resource":{"url":"https://ex.com/a"},"accepts":[
          {"scheme":"exact","network":"base-sepolia","amount":"10000",
           "payTo":"0x1111111111111111111111111111111111111111",
           "asset":"0x036CbD53842c5426634e7929541eC2318f3dCF7e"}]}"#;
        let pr = parse_required(None, with_res).unwrap();
        let req = pick_requirement(&pr).unwrap();
        let v = decode(&pr.build_direct_submission(&req, &proof()).unwrap().value);
        assert_eq!(v["resource"]["url"], "https://ex.com/a");
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

/// 🔴 **교차 구현 하네스** (개발 64) — 상대 구현(kaditang/x402-arc)이 만든 진짜 402 챌린지를 읽어
/// **우리가 낼 제출 헤더**를 만들어 파일로 내보낸다. 그 헤더를 상대 구현의 검증기(`ArcLocalFacilitator`)에
/// 그대로 먹여 통과하는지 보는 것이 이 하네스의 목적이다.
///
/// 왜 테스트로 두나: 여기서 도는 것이 **실제 경로 그 자체**여야 의미가 있다 — `pick_requirement` →
/// `Requirement::binding` → `arc_direct::fresh_nonce` → `build_direct_submission` 는 flow.rs 가 부르는
/// 함수와 한 글자도 다르지 않다(HTTP·GUI 만 빠진다). 테스트가 판정을 베껴 쓰면 「초록인데 실물은
/// 다른 값」이 된다(개발 63).
///
/// 실행: `HARNESS_DIR=… KURA_CHAIN_ID=5042002 cargo test --ignored direct_submission_for_harness`
#[cfg(test)]
mod harness {
    use super::*;
    use crate::arc_direct;

    #[test]
    #[ignore = "하네스 — HARNESS_DIR 이 필요하다(상대 구현과 교차 검증)"]
    fn direct_submission_for_harness() {
        let dir = std::env::var("HARNESS_DIR").expect("HARNESS_DIR");
        let body =
            std::fs::read_to_string(format!("{dir}/challenge.json")).expect("challenge.json");
        let pr = parse_required(None, &body).expect("402 파싱");
        let req = pick_requirement(&pr).expect("요구 선택");
        assert_eq!(
            req.method,
            TransferMethod::ClientBroadcast,
            "직접 제출 요구로 안 읽혔다"
        );
        let (client_nonce, seed, nonce) =
            arc_direct::fresh_nonce(req.seed.as_deref(), &req.binding());
        // tx 해시는 하네스가 정한다(체인 조회는 노드 쪽 스텁이 답한다).
        let tx = std::env::var("HARNESS_TX").expect("HARNESS_TX");
        let sub = pr
            .build_direct_submission(
                &req,
                &arc_direct::DirectProof {
                    transaction: tx,
                    client_nonce,
                    seed,
                    nonce: nonce.clone(),
                },
            )
            .expect("제출 조립");
        std::fs::write(format!("{dir}/header.b64"), &sub.value).expect("header 쓰기");
        std::fs::write(
            format!("{dir}/derived.json"),
            serde_json::json!({
                "nonce": nonce,
                "header_name": sub.header_name,
                "alt_header": sub.alt_header,
                "amount": req.amount,
                "pay_to": req.pay_to,
            })
            .to_string(),
        )
        .expect("derived 쓰기");
    }
}
