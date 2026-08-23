// 어댑터 공통 결제 흐름 (개발 22) — MCP(main.rs)와 CLI(bin/kura.rs)가 같은 로직을 공유한다.
//
// 결제·x402 의 보안 민감한 부분(리다이렉트 가드·single-flight·정산 게이팅·승인 대기)을 여기 한 곳에
// 모은다. 두 어댑터가 각자 구현하면 한쪽만 고쳐져 보안이 갈라질 위험이 있다 → 하나의 진실.
//
// 비번은 절대 여기로 들어오지 않는다. 이 모듈은 GUI 에 "요청"만 하고, 서명·전송은 GUI 가
// 사람 승인을 받아 수행한다(payment.rs 의 파일 IPC). 한도·긴급잠금·화이트리스트도 GUI 가 강제한다.

use crate::chain::active_chain;
use crate::{payment, x402};
use crate::{tf, ts};

/// 외부 서버가 거대 본문으로 어댑터 메모리를 채우지 못하게 하는 상한(바이트). 본문을 읽기 전에
/// Content-Length 로 먼저 거른다(버퍼링 후 자르면 메모리 보호가 안 됨).
const MAX_BODY_BYTES: u64 = 4 * 1024 * 1024; // 4 MiB

/// 표시용 본문 문자 상한 — 에이전트/터미널 컨텍스트 보호. 문자 단위라 UTF-8 안전.
const MAX_BODY_CHARS: usize = 100_000;

/// 본문이 너무 길면 잘라서 돌려준다.
fn cap_body(body: String) -> String {
    if body.chars().count() <= MAX_BODY_CHARS {
        return body;
    }
    let mut s: String = body.chars().take(MAX_BODY_CHARS).collect();
    s.push_str(ts!(
        "\n…(본문이 길어 잘렸어요)",
        "\n…(body truncated — it was long)"
    ));
    s
}

/// 응답 본문을 **스트리밍**으로 읽되 누적 바이트가 상한을 넘으면 즉시 중단한다. Content-Length 가
/// 없는(chunked) 응답도 전량 버퍼링하지 않으므로 악성 서버의 메모리 폭주(DoS)를 막는다.
/// 반환: (읽은 바이트, 상한 초과로 잘렸는지). 헤더는 호출자가 먼저 챙긴 뒤 호출한다.
async fn read_limited(mut resp: reqwest::Response) -> Result<(Vec<u8>, bool), String> {
    let cap = MAX_BODY_BYTES as usize;
    let mut buf: Vec<u8> = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| tf!("본문 읽기 실패: {e}", "Couldn't read the body: {e}"))?
    {
        if buf.len() + chunk.len() > cap {
            buf.extend_from_slice(&chunk[..cap - buf.len()]);
            truncated = true;
            break;
        }
        buf.extend_from_slice(&chunk);
    }
    Ok((buf, truncated))
}

/// 응답 본문을 메모리 안전하게 읽어 표시용으로 캡한다(바이트 상한 스트리밍 + 문자 상한).
async fn read_body_capped(resp: reqwest::Response) -> String {
    let (bytes, truncated) = match read_limited(resp).await {
        Ok(v) => v,
        Err(e) => return format!("({e})"),
    };
    let mut s = cap_body(String::from_utf8_lossy(&bytes).into_owned());
    if truncated {
        s.push_str(ts!(
            "\n…(본문이 너무 커서 잘렸어요)",
            "\n…(body truncated — it was too large)"
        ));
    }
    s
}

/// 최초 probe GET 용 HTTP 클라이언트 — 리다이렉트를 따라간다(결제 헤더가 없어 유출 위험 없음 —
/// "리다이렉트 후 402" 같은 정상 케이스를 깨지 않게). 요청당 30초 타임아웃.
fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| {
            tf!(
                "HTTP 클라이언트 생성 실패: {e}",
                "Couldn't create the HTTP client: {e}"
            )
        })
}

/// 결제 재요청 전용 — **리다이렉트를 따라가지 않는다**(`Policy::none`). PAYMENT-SIGNATURE/X-PAYMENT
/// (서명된 EIP-3009 인가)는 reqwest 의 cross-host 민감 헤더 제거 대상이 아니라, 리다이렉트를 따라가면
/// 다른 origin 으로 서명이 유출될 수 있다 → 402 를 낸 바로 그 최종 URL 에만 결제 헤더를 보낸다.
fn http_client_no_redirect() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| {
            tf!(
                "HTTP 클라이언트 생성 실패: {e}",
                "Couldn't create the HTTP client: {e}"
            )
        })
}

/// 결제(송금) 요청 결과 — 사람 승인 팝업에서 사용자가 응답한 결과.
pub struct PayOutcome {
    /// approved | rejected | failed.
    pub status: String,
    pub tx_hash: String,
    pub detail: String,
    /// tx 해시가 있을 때의 익스플로러 링크(없으면 빈 문자열).
    pub explorer: String,
}

/// 결제(송금)를 사용자에게 요청한다. 앱이 켜져 있고 대기 중인 요청이 없어야 한다.
/// 토큰 검증 → 앱 생존/single-flight 확인 → 요청 파일 작성 → 최대 5분 승인 대기.
///
/// `Err` = 요청 자체를 띄울 수 없음(잘못된 토큰·앱 꺼짐·이미 대기 중·시간 초과).
/// `Ok`  = 사용자가 응답함(status 가 approved/rejected/failed 중 하나).
pub async fn run_payment(
    token: &str,
    to: &str,
    amount: &str,
    memo: &str,
) -> Result<PayOutcome, String> {
    let token = token.trim().to_uppercase();
    if token != "USDC" && token != "ETH" {
        return Err(ts!(
            "token은 USDC 또는 ETH 여야 합니다",
            "token must be either USDC or ETH"
        )
        .into());
    }
    // 앱이 안 켜져 있으면 승인할 사람이 없다 → 즉시 안내(5분 대기 안 함).
    if !payment::app_alive() {
        return Err(ts!(
            "지갑 앱이 실행 중이 아니에요. 앱을 켠 뒤 다시 시도하세요.",
            "The wallet app isn't running. Open it and try again."
        )
        .into());
    }
    // single-flight: 이미 대기 중인 요청이 있으면 거절.
    if payment::has_pending() {
        return Err(ts!(
            "이미 승인 대기 중인 결제가 있어요. 먼저 처리한 뒤 다시 요청하세요.",
            "A payment is already waiting for approval. Let the user handle it, then ask again."
        )
        .into());
    }

    let id = payment::write_request(&token, to.trim(), amount.trim(), memo.trim())?;

    match payment::await_result(&id, payment::APPROVAL_TIMEOUT).await {
        Some(r) => {
            let explorer = if r.tx_hash.is_empty() {
                String::new()
            } else {
                format!("{}{}", active_chain().explorer_tx_prefix, r.tx_hash)
            };
            Ok(PayOutcome {
                status: r.status,
                tx_hash: r.tx_hash,
                detail: r.detail,
                explorer,
            })
        }
        None => {
            payment::cancel_request(&id);
            Err(ts!(
                "승인 시간 초과(5분). 사용자가 응답하지 않았어요.",
                "Approval timed out after 5 minutes — the user didn't respond."
            )
            .into())
        }
    }
}

/// x402 리소스 가져오기 결과.
pub enum X402Outcome {
    /// 402 가 아니었다 — 결제 불필요, 본문 그대로.
    NotPaid { http_status: u16, body: String },
    /// 결제가 필요했지만 사용자가 승인하지 않았다(rejected | failed).
    Declined { status: String, detail: String },
    /// 결제하고 콘텐츠를 받았다.
    Paid {
        http_status: u16,
        /// 2xx(정산 성공) 여부. false 면 결제 헤더는 보냈으나 정산 단계에서 실패.
        ok: bool,
        amount: String,
        pay_to: String,
        resource: String,
        /// 정산 증빙 헤더(base64). 없으면 빈 문자열.
        settlement: String,
        body: String,
    },
}

/// x402 유료 리소스를 가져온다. GET → 402 면 결제 요구 파싱 → GUI 승인(서명) → 결제 헤더로 재요청.
///
/// `memo` = 호출자가 준 결제 사유(없으면 서버 설명으로 폴백). `Err` = 진행 불가(요청/파싱 실패·
/// 앱 꺼짐·대기 중·시간 초과·서명 누락·재요청 실패).
pub async fn run_x402(url: &str, memo: Option<&str>) -> Result<X402Outcome, String> {
    let url = url.trim().to_string();
    let client = http_client()?;

    // 1) 먼저 그냥 가져와 본다. 402가 아니면 결제 불필요 → 본문 그대로 반환.
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| tf!("요청 실패: {e}", "The request failed: {e}"))?;
    let http_status = resp.status().as_u16();
    if http_status != 402 {
        let body = read_body_capped(resp).await;
        return Ok(X402Outcome::NotPaid { http_status, body });
    }

    // probe 가 리다이렉트를 따라갔을 수 있으니 402 를 실제로 낸 "최종" URL 을 잡아둔다 —
    // 결제 재요청(서명 헤더 포함)은 이 최종 URL 에만 보낸다(원본 URL 로 보내면 다시 리다이렉트).
    let final_url = resp.url().clone();

    // 2) 결제 요구 파싱 → 우리가 처리 가능한 요구 선택.
    // V2는 요구가 `payment-required` 헤더(base64)에, V1은 본문에 온다. 본문을 소비하기 전에 헤더 먼저.
    let pr_header = resp
        .headers()
        .get("payment-required")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    // 402 본문도 스트리밍 상한으로 읽는다(Content-Length 없는 거대 응답 차단). 잘렸으면 = 비정상.
    let (body402_bytes, truncated) = read_limited(resp).await?;
    if truncated {
        return Err(ts!(
            "402 응답 본문이 너무 큽니다",
            "The 402 response body is too large"
        )
        .into());
    }
    let body402 = String::from_utf8_lossy(&body402_bytes).into_owned();
    let required = x402::parse_required(pr_header.as_deref(), &body402).map_err(|e| {
        tf!(
            "402 형식 파싱 실패: {e}",
            "Couldn't parse the 402 response: {e}"
        )
    })?;
    let req = x402::pick_requirement(&required)?;
    let amount_usdc = x402::base_units_to_usdc(&req.amount)?;
    let resource = required.display_resource(&req, &url);
    // 팝업에 보일 사유: 호출자 memo > 서버 description(V1 요구별/V2 최상위) > 빈 값.
    let memo = memo
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| {
            let d = required.description(&req);
            (!d.is_empty()).then_some(d)
        })
        .unwrap_or_default();

    // 3) 승인할 앱이 켜져 있는지 + single-flight.
    if !payment::app_alive() {
        return Err(ts!(
            "지갑 앱이 실행 중이 아니에요. 앱을 켠 뒤 다시 시도하세요.",
            "The wallet app isn't running. Open it and try again."
        )
        .into());
    }
    if payment::has_pending() {
        return Err(ts!(
            "이미 승인 대기 중인 결제가 있어요. 먼저 처리한 뒤 다시 요청하세요.",
            "A payment is already waiting for approval. Let the user handle it, then ask again."
        )
        .into());
    }

    // 4) GUI에 서명 요청 → 사람 승인 → 서명 페이로드 수신.
    let id = payment::write_x402_request(req.pay_to.trim(), &amount_usdc, &memo, &resource)?;
    let result = match payment::await_result(&id, payment::APPROVAL_TIMEOUT).await {
        Some(r) => r,
        None => {
            payment::cancel_request(&id);
            return Err(ts!(
                "승인 시간 초과(5분). 사용자가 응답하지 않았어요.",
                "Approval timed out after 5 minutes — the user didn't respond."
            )
            .into());
        }
    };
    if result.status != "approved" {
        return Ok(X402Outcome::Declined {
            status: result.status, // rejected | failed
            detail: result.detail,
        });
    }
    let payment = result.x402.ok_or(ts!(
        "승인됐지만 서명 페이로드가 비어 있어요",
        "Approved, but the signature payload is empty"
    ))?;

    // 5) 결제 헤더를 붙여 재요청 → 콘텐츠 수신.
    // V2면 PAYMENT-SIGNATURE(+ resource/accepted 에코), V1이면 X-PAYMENT. 정산 응답 헤더 이름도 버전별.
    let sub = required.build_submission(&req, &payment)?;
    // 결제 헤더(서명된 인가)는 리다이렉트를 따라가지 않는 클라이언트로 최종 URL 에만 보낸다.
    let pay_client = http_client_no_redirect()?;
    let paid_resp = pay_client
        .get(final_url)
        .header(sub.header_name, &sub.value)
        .send()
        .await
        .map_err(|e| tf!("결제 재요청 실패: {e}", "The paid re-request failed: {e}"))?;
    let paid_status = paid_resp.status().as_u16();
    let settlement = paid_resp
        .headers()
        .get(sub.response_header)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .unwrap_or_default();
    let body = read_body_capped(paid_resp).await;

    // 정산 tx 해시를 뽑아 GUI 내역에 반영되도록 기록(nonce 로 "signed" 항목과 매칭).
    // **2xx(실제 정산 성공) 응답일 때만** 기록한다 — 비-2xx 응답에 위조 PAYMENT-RESPONSE 헤더를
    // 실어 보내 GUI 내역을 가짜 "정산됨"으로 오염시키는 걸 막는다.
    let ok = (200..300).contains(&paid_status);
    if ok && !settlement.is_empty() {
        if let Some((tx, success)) = x402::parse_settlement(&settlement) {
            let _ = payment::record_settlement(&payment.authorization.nonce, &tx, success);
        }
    }

    Ok(X402Outcome::Paid {
        http_status: paid_status,
        ok,
        amount: amount_usdc,
        pay_to: req.pay_to,
        resource,
        settlement,
        body,
    })
}
