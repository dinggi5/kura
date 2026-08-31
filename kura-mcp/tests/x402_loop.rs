// x402 전체 HTTP 루프 통합 테스트 (개발 11, Session 12 / V2 추가 개발 12, Session 13).
//
// 로컬 목 x402 서버를 띄우고, 실제 reqwest 로 끝과 끝을 검증한다:
//   GET → 402(결제 요구) → [지갑이 서명] → 결제 헤더로 재요청 → 목 서버가 EIP-3009 서명을
//   복구해 서명자가 authorization.from 과 일치하는지 확인 → 200 + 콘텐츠.
//
// 두 형태를 모두 돌린다:
//   V1: 요구=본문, network="base-sepolia", maxAmountRequired, 제출=X-PAYMENT, 정산=X-PAYMENT-RESPONSE
//   V2: 요구=payment-required 헤더, network="eip155:84532", amount, 최상위 resource + solana 옵션,
//       제출=PAYMENT-SIGNATURE({x402Version,resource,accepted,payload}), 정산=PAYMENT-RESPONSE
// 우리 파서/조립기(build_submission)가 양쪽과 맞물리는지 결정적으로 증명한다. (실 www.x402.org V2
// 정산은 개발 12에서 온체인 status 0x1 로 별도 라이브 검증함.)
//
// GUI/IPC(사람 승인)만 빠진 형태다 — 서명이 진짜 USDC(Base Sepolia) 도메인으로 만들어지므로 실제
// 페이실리테이터 정산과 동등하다(서명 검증 통과 = 온체인에서도 통과).

use alloy::primitives::{address, Address, B256, U256};
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use alloy::sol_types::{eip712_domain, SolStruct};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::str::FromStr;

// Base Sepolia USDC — src-tauri/lib.rs 와 동일 상수 (서명·복구가 같은 도메인이어야 함).
const USDC_ADDRESS: Address = address!("0x036CbD53842c5426634e7929541eC2318f3dCF7e");
const CHAIN_ID: u64 = 84_532;
const USDC_NAME: &str = "USDC";
const USDC_VERSION: &str = "2";

sol! {
    struct TransferWithAuthorization {
        address from;
        address to;
        uint256 value;
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
    }
}

const PAY_TO: &str = "0x209693Bc6afc0C5328bA36FaF03C514EF312287C";
const MAX_AMOUNT: &str = "10000"; // 0.01 USDC (base units)
const PAID_BODY: &str = "{\"secret\":\"프리미엄 데이터 42\"}";

/// 목 서버가 흉내낼 x402 버전.
#[derive(Clone, Copy)]
enum Mode {
    V1, // 본문 JSON, network="base-sepolia", maxAmountRequired, 제출=X-PAYMENT
    V2, // payment-required 헤더(base64), eip155:84532, amount, 최상위 resource + solana, 제출=PAYMENT-SIGNATURE
}

/// 목 x402 서버. 한 커넥션당 요청 1건 처리하고 Connection: close 로 닫는다(2회 accept).
/// 반환: (bound_url, 검증결과 채널).
fn spawn_mock(mode: Mode) -> (String, std::sync::mpsc::Receiver<Result<Address, String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}/data");
    let (tx, rx) = std::sync::mpsc::channel();
    let asset = format!("{USDC_ADDRESS:?}");
    let res_url = url.clone();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut s = match stream {
                Ok(s) => s,
                Err(_) => break,
            };
            let req = read_http_request(&mut s);
            // 제출 헤더: V2=payment-signature / V1=x-payment.
            let submit = header_value(&req, "payment-signature")
                .map(|h| (h, true))
                .or_else(|| header_value(&req, "x-payment").map(|h| (h, false)));

            match submit {
                // 1차 요청: 결제 헤더 없음 → 402 + 결제 요구 (모드별 형태).
                None => match mode {
                    Mode::V1 => {
                        let body = format!(
                            r#"{{"x402Version":1,"accepts":[{{"scheme":"exact","network":"base-sepolia","maxAmountRequired":"{MAX_AMOUNT}","resource":"{res_url}","description":"테스트 데이터","payTo":"{PAY_TO}","asset":"{asset}","maxTimeoutSeconds":60,"extra":{{"name":"USDC","version":"2"}}}}]}}"#
                        );
                        write_402(&mut s, &body, None);
                    }
                    Mode::V2 => {
                        // 실 엔드포인트처럼: 요구는 payment-required 헤더(base64), 본문은 빈 객체.
                        // solana 옵션을 함께 제시 → 우리 파서가 EVM(eip155:84532)을 골라야 한다.
                        let pr = format!(
                            r#"{{"x402Version":2,"error":"Payment required","resource":{{"url":"{res_url}","description":"테스트 데이터 V2","mimeType":""}},"accepts":[{{"scheme":"exact","network":"solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1","amount":"{MAX_AMOUNT}","asset":"4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU","payTo":"CKsol","maxTimeoutSeconds":300}},{{"scheme":"exact","network":"eip155:84532","amount":"{MAX_AMOUNT}","asset":"{asset}","payTo":"{PAY_TO}","maxTimeoutSeconds":300,"extra":{{"name":"USDC","version":"2"}}}}]}}"#
                        );
                        write_402(&mut s, "{}", Some(&B64.encode(pr)));
                    }
                },
                // 2차 요청: 결제 헤더 검증 → 서명자 복구 후 200(정산 응답 헤더 포함) 또는 402.
                Some((header, is_v2)) => {
                    match verify_payment(&header) {
                        Ok(signer) => {
                            let settle_hdr = if is_v2 {
                                "PAYMENT-RESPONSE"
                            } else {
                                "X-PAYMENT-RESPONSE"
                            };
                            // base64({"settled":true})
                            write_response(
                                &mut s,
                                200,
                                "OK",
                                PAID_BODY,
                                settle_hdr,
                                "eyJzZXR0bGVkIjp0cnVlfQ==",
                            );
                            let _ = tx.send(Ok(signer));
                            break;
                        }
                        Err(e) => {
                            write_response(
                                &mut s,
                                402,
                                "Payment Required",
                                "{\"error\":\"invalid\"}",
                                "",
                                "",
                            );
                            let _ = tx.send(Err(e));
                            break;
                        }
                    }
                }
            }
        }
    });

    (url, rx)
}

/// 헤더 끝(\r\n\r\n)까지 읽어 요청 텍스트를 돌려준다(GET이라 바디 없음).
fn read_http_request(s: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = s.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// 대소문자 무시 헤더 조회.
fn header_value(req: &str, name: &str) -> Option<String> {
    req.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim().eq_ignore_ascii_case(name)).then(|| v.trim().to_string())
    })
}

/// 402 응답을 쓴다. pr_header 가 Some 이면 payment-required 헤더로 결제 요구를 보낸다(V2).
fn write_402(s: &mut TcpStream, body: &str, pr_header: Option<&str>) {
    let extra = pr_header
        .map(|h| format!("payment-required: {h}\r\n"))
        .unwrap_or_default();
    let resp = format!(
        "HTTP/1.1 402 Payment Required\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = s.write_all(resp.as_bytes());
    let _ = s.flush();
}

/// 일반 응답. settle_hdr/settle 이 비어있지 않으면 정산 증빙 헤더를 붙인다(헤더 이름은 버전별로 다름).
fn write_response(
    s: &mut TcpStream,
    code: u16,
    reason: &str,
    body: &str,
    settle_hdr: &str,
    settle: &str,
) {
    let extra = if settle.is_empty() || settle_hdr.is_empty() {
        String::new()
    } else {
        format!("{settle_hdr}: {settle}\r\n")
    };
    let resp = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = s.write_all(resp.as_bytes());
    let _ = s.flush();
}

/// 결제 헤더(base64 JSON)를 복호화해 EIP-3009 서명자를 복구하고, value/payTo/network 를 검증한다.
/// V1({scheme,network,payload}) · V2({accepted,payload}) 두 형태 모두 받는다.
fn verify_payment(header: &str) -> Result<Address, String> {
    let raw = B64
        .decode(header.trim())
        .map_err(|e| format!("base64: {e}"))?;
    let v: serde_json::Value = serde_json::from_slice(&raw).map_err(|e| format!("json: {e}"))?;

    // scheme/network 는 V1=최상위, V2=accepted 안.
    let (scheme, net) = if v.get("accepted").is_some() {
        (
            v["accepted"]["scheme"].as_str(),
            v["accepted"]["network"].as_str(),
        )
    } else {
        (v["scheme"].as_str(), v["network"].as_str())
    };
    if scheme != Some("exact") {
        return Err("scheme 불일치".into());
    }
    let net = net.unwrap_or_default();
    if net != "base-sepolia" && net != "eip155:84532" {
        return Err(format!("network 불일치: {net}"));
    }

    let auth = &v["payload"]["authorization"];
    let sig_hex = v["payload"]["signature"].as_str().ok_or("서명 없음")?;

    let from =
        Address::from_str(auth["from"].as_str().ok_or("from 없음")?).map_err(|e| e.to_string())?;
    let to = Address::from_str(auth["to"].as_str().ok_or("to 없음")?).map_err(|e| e.to_string())?;
    let value =
        U256::from_str(auth["value"].as_str().ok_or("value 없음")?).map_err(|e| e.to_string())?;
    let valid_after = U256::from_str(auth["validAfter"].as_str().ok_or("validAfter 없음")?)
        .map_err(|e| e.to_string())?;
    let valid_before = U256::from_str(auth["validBefore"].as_str().ok_or("validBefore 없음")?)
        .map_err(|e| e.to_string())?;
    let nonce =
        B256::from_str(auth["nonce"].as_str().ok_or("nonce 없음")?).map_err(|e| e.to_string())?;

    // 서버가 요구한 결제 조건과 일치하는지(액수·수취인).
    if value != U256::from_str(MAX_AMOUNT).unwrap() {
        return Err("금액 불일치".into());
    }
    if to != Address::from_str(PAY_TO).unwrap() {
        return Err("수취인 불일치".into());
    }

    let domain = eip712_domain! {
        name: USDC_NAME,
        version: USDC_VERSION,
        chain_id: CHAIN_ID,
        verifying_contract: USDC_ADDRESS,
    };
    let order = TransferWithAuthorization {
        from,
        to,
        value,
        validAfter: valid_after,
        validBefore: valid_before,
        nonce,
    };
    let hash = order.eip712_signing_hash(&domain);
    let sig = alloy::primitives::Signature::from_str(sig_hex).map_err(|e| e.to_string())?;
    let recovered = sig
        .recover_address_from_prehash(&hash)
        .map_err(|e| e.to_string())?;
    if recovered != from {
        return Err("서명자 ≠ from (위조)".into());
    }
    Ok(recovered)
}

/// 지갑 역할: 고른 요구에 맞춰 EIP-3009 인가를 서명한다(GUI의 sign_authorization 와 동일 로직).
fn sign_for(
    signer: &PrivateKeySigner,
    req: &kura_mcp::x402::Requirement,
) -> kura_mcp::x402::X402Payment {
    use alloy::signers::SignerSync;
    let to = Address::from_str(&req.pay_to).unwrap();
    let value = U256::from_str(&req.amount).unwrap();
    let valid_before = U256::from(2_000_000_000u64);
    let nonce = B256::from(U256::from(12345u64));

    let domain = eip712_domain! {
        name: USDC_NAME,
        version: USDC_VERSION,
        chain_id: CHAIN_ID,
        verifying_contract: USDC_ADDRESS,
    };
    let order = TransferWithAuthorization {
        from: signer.address(),
        to,
        value,
        validAfter: U256::ZERO,
        validBefore: valid_before,
        nonce,
    };
    let hash = order.eip712_signing_hash(&domain);
    let sig = signer.sign_hash_sync(&hash).unwrap();
    let mut sig_bytes = [0u8; 65];
    sig_bytes[..32].copy_from_slice(&sig.r().to_be_bytes::<32>());
    sig_bytes[32..64].copy_from_slice(&sig.s().to_be_bytes::<32>());
    sig_bytes[64] = 27 + sig.v() as u8;

    kura_mcp::x402::X402Payment {
        signature: format!("0x{}", alloy::hex::encode(sig_bytes)),
        authorization: kura_mcp::x402::X402Authorization {
            from: signer.address().to_string(),
            to: req.pay_to.clone(),
            value: req.amount.clone(),
            valid_after: "0".to_string(),
            valid_before: valid_before.to_string(),
            nonce: nonce.to_string(),
        },
    }
}

/// 끝과 끝 루프를 한 번 돌린다(모드별). 서버가 복구한 서명자를 돌려준다.
async fn drive_loop(mode: Mode) -> Address {
    use kura_mcp::x402;
    // 이 테스트의 목/상수는 Base Sepolia 기준 → 활성 체인을 고정한다(사용자의 라이브 settings.json
    // 이 메인넷이어도 결정론적이게). 모든 테스트가 같은 값을 쓰므로 병렬 실행에도 안전하다.
    std::env::set_var("KURA_CHAIN_ID", "84532");
    let (url, rx) = spawn_mock(mode);
    let client = reqwest::Client::new();

    // 1) 첫 GET → 402.
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 402, "결제 전엔 402여야 한다");
    // V2는 payment-required 헤더, V1은 본문에서 요구를 읽는다.
    let pr_header = resp
        .headers()
        .get("payment-required")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let body = resp.text().await.unwrap();
    let required = x402::parse_required(pr_header.as_deref(), &body).expect("결제 요구 파싱");

    // 2) 우리가 처리 가능한 요구 선택(exact·Base Sepolia·USDC). V2면 solana 말고 eip155 를 골라야.
    let req = x402::pick_requirement(&required).expect("USDC 요구를 골라야 한다");
    assert_eq!(req.amount, MAX_AMOUNT);

    // 3) 지갑이 서명 → 결제 헤더 조립(버전별 헤더 이름·payload).
    let signer: PrivateKeySigner =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
            .parse()
            .unwrap();
    let payment = sign_for(&signer, &req);
    let sub = required.build_submission(&req, &payment).unwrap();

    // 4) 결제 헤더 붙여 재요청 → 200 + 콘텐츠.
    let paid = client
        .get(&url)
        .header(sub.header_name, &sub.value)
        .send()
        .await
        .unwrap();
    assert_eq!(paid.status().as_u16(), 200, "서명 검증 후 200이어야 한다");
    assert!(
        paid.headers().get(sub.response_header).is_some(),
        "정산 증빙 헤더({})가 있어야 한다",
        sub.response_header
    );
    let content = paid.text().await.unwrap();
    assert_eq!(content, PAID_BODY);

    // 5) 서버가 복구한 서명자 = 우리 지갑 주소 (= 온체인 정산도 통과).
    let recovered = rx.recv().unwrap().expect("서명 검증 통과");
    assert_eq!(recovered, signer.address());
    recovered
}

/// V1(본문 기반, X-PAYMENT) 전체 루프.
#[tokio::test]
async fn x402_v1_loop_signs_and_settles() {
    drive_loop(Mode::V1).await;
}

/// V2(payment-required 헤더, eip155:84532, solana 옵션 동봉, PAYMENT-SIGNATURE) 전체 루프 — 실 엔드포인트 형태.
#[tokio::test]
async fn x402_v2_loop_signs_and_settles() {
    drive_loop(Mode::V2).await;
}

/// [라이브·네트워크] 실 www.x402.org/protected(V2)의 402를 우리 파서가 그대로 처리하는지.
/// 키·돈 불필요(파싱만). 기본 `cargo test`에선 ignored — 명시 실행:
///   cargo test --test x402_loop live_real_endpoint_v2_parse -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn live_real_endpoint_v2_parse() {
    use kura_mcp::x402;
    std::env::set_var("KURA_CHAIN_ID", "84532"); // 실 엔드포인트는 base-sepolia 요구 → 활성 체인 고정
    let url = "https://www.x402.org/protected";
    let client = reqwest::Client::new();
    let resp = client.get(url).send().await.expect("실 엔드포인트 GET");
    assert_eq!(resp.status().as_u16(), 402, "유료 리소스는 402여야 한다");

    let h = resp
        .headers()
        .get("payment-required")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    assert!(
        h.is_some(),
        "실 엔드포인트는 payment-required 헤더로 요구를 준다(V2)"
    );

    let body = resp.text().await.unwrap();
    let required = x402::parse_required(h.as_deref(), &body).expect("V2 헤더 파싱");
    assert_eq!(required.version, 2, "x402Version=2");

    let req = x402::pick_requirement(&required).expect("eip155:84532 USDC 요구 선택");
    assert_eq!(req.network, "eip155:84532");
    let amount = x402::base_units_to_usdc(&req.amount).unwrap();
    // 리소스 URL 은 **우리가 요청한 URL** 이다(개발 51) — 서버가 402 에 적어 보낸 주장값이 아니다.
    println!("LIVE V2 ✓  amount={amount} USDC  payTo={}  resource={url}", req.pay_to);
}
