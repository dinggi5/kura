// 로컬 목 x402 서버 (개발 11, Session 12) — 라이브 데모/수동 검증용.
//
// 실제 x402 유료 엔드포인트를 흉내낸다. 페이실리테이터 없이 우리가 직접 EIP-3009 서명을
// 복구·검증하므로, "지갑이 만든 서명이 온체인에서도 통과할지"를 로컬에서 결정적으로 확인할 수 있다.
//
//   1) GET /article (X-PAYMENT 없음)      → 402 + accepts (exact·base-sepolia·USDC)
//   2) GET /article (X-PAYMENT: base64)   → 서명 복구 → from 일치하면 200 + 콘텐츠, 아니면 402
//
// 실행:   cargo run --example mock_x402_server --manifest-path kura-mcp/Cargo.toml
// 호출:   MCP 도구 x402_fetch 에 url=http://127.0.0.1:4021/article 전달
//         → 지갑 앱 팝업 → 비번 승인 → 서명 → 200 콘텐츠 반환.

use alloy::primitives::{address, Address, B256, U256};
use alloy::sol;
use alloy::sol_types::{eip712_domain, SolStruct};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::str::FromStr;

// Base Sepolia USDC — 지갑(src-tauri)과 같은 도메인이어야 서명이 맞물린다.
const USDC_ADDRESS: Address = address!("0x036CbD53842c5426634e7929541eC2318f3dCF7e");
const CHAIN_ID: u64 = 84_532;
const USDC_NAME: &str = "USDC";
const USDC_VERSION: &str = "2";

// 이 목 서버가 받겠다고 요구하는 결제 (아무 테스트 주소·소액).
const PAY_TO: &str = "0x209693Bc6afc0C5328bA36FaF03C514EF312287C";
const MAX_AMOUNT: &str = "10000"; // 0.01 USDC
const PORT: u16 = 4021;
const PAID_BODY: &str =
    r#"{"article":"x402로 잠긴 프리미엄 글","body":"결제가 확인됐습니다. 비밀 콘텐츠 🎉"}"#;

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

fn main() {
    let listener = TcpListener::bind(("127.0.0.1", PORT)).expect("포트 바인드 실패");
    let asset = format!("{USDC_ADDRESS:?}");
    println!("🔒 목 x402 서버: http://127.0.0.1:{PORT}/article");
    println!("   요구: {MAX_AMOUNT} (0.01 USDC) → {PAY_TO}");
    println!("   x402_fetch 도구로 위 URL을 호출하세요. Ctrl+C 로 종료.\n");

    for stream in listener.incoming() {
        let Ok(mut s) = stream else { continue };
        let req = read_http_request(&mut s);
        match header_value(&req, "x-payment") {
            None => {
                let body = format!(
                    r#"{{"x402Version":1,"accepts":[{{"scheme":"exact","network":"base-sepolia","maxAmountRequired":"{MAX_AMOUNT}","resource":"http://127.0.0.1:{PORT}/article","description":"프리미엄 글 1회 열람","payTo":"{PAY_TO}","asset":"{asset}","maxTimeoutSeconds":120,"extra":{{"name":"USDC","version":"2"}}}}]}}"#
                );
                println!("→ 402 Payment Required (결제 요구 전송)");
                write_response(&mut s, 402, "Payment Required", &body, "");
            }
            Some(header) => match verify_payment(&header) {
                Ok(signer) => {
                    println!("✅ 서명 검증 통과 — 서명자 {signer} → 200 콘텐츠 반환");
                    write_response(&mut s, 200, "OK", PAID_BODY, "eyJzZXR0bGVkIjp0cnVlfQ==");
                }
                Err(e) => {
                    println!("❌ 서명 검증 실패: {e} → 402");
                    let body = format!(r#"{{"error":"{e}"}}"#);
                    write_response(&mut s, 402, "Payment Required", &body, "");
                }
            },
        }
    }
}

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

fn header_value(req: &str, name: &str) -> Option<String> {
    req.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim().eq_ignore_ascii_case(name)).then(|| v.trim().to_string())
    })
}

fn write_response(s: &mut TcpStream, code: u16, reason: &str, body: &str, settle: &str) {
    let extra = if settle.is_empty() {
        String::new()
    } else {
        format!("X-PAYMENT-RESPONSE: {settle}\r\n")
    };
    let resp = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = s.write_all(resp.as_bytes());
    let _ = s.flush();
}

/// X-PAYMENT(base64 JSON)에서 EIP-3009 서명자를 복구하고 액수·수취인을 검증한다.
fn verify_payment(header: &str) -> Result<Address, String> {
    let raw = B64.decode(header.trim()).map_err(|e| format!("base64: {e}"))?;
    let v: serde_json::Value = serde_json::from_slice(&raw).map_err(|e| format!("json: {e}"))?;
    if v["scheme"] != "exact" || v["network"] != "base-sepolia" {
        return Err("scheme/network 불일치".into());
    }
    let auth = &v["payload"]["authorization"];
    let sig_hex = v["payload"]["signature"].as_str().ok_or("서명 없음")?;
    let field = |k: &str| -> Result<String, String> {
        auth[k]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("{k} 없음"))
    };
    let from = Address::from_str(&field("from")?).map_err(|e| e.to_string())?;
    let to = Address::from_str(&field("to")?).map_err(|e| e.to_string())?;
    let value = U256::from_str(&field("value")?).map_err(|e| e.to_string())?;
    let valid_after = U256::from_str(&field("validAfter")?).map_err(|e| e.to_string())?;
    let valid_before = U256::from_str(&field("validBefore")?).map_err(|e| e.to_string())?;
    let nonce = B256::from_str(&field("nonce")?).map_err(|e| e.to_string())?;

    if value != U256::from_str(MAX_AMOUNT).unwrap() {
        return Err("요구 금액과 불일치".into());
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
