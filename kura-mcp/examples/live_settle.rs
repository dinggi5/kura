// 실 x402 엔드포인트 라이브 정산 드라이버 (개발 12, Session 13).
//
// MCP 도구 x402_fetch 와 동일한 끝과 끝 루프를, Claude Code 재시작 없이 직접 돌리기 위한 예제.
// IPC 파일(~/.jigap/)은 어느 프로세스가 써도 같으므로, 실행 중인 dev 앱(src-tauri, 무변경)이
// 팝업을 띄우고 사람이 비번으로 승인해 서명한다. 비밀은 여기 없다(서명은 GUI 독점).
//
//   1) GET url → 402면 결제 요구 파싱(V1 본문 / V2 payment-required 헤더)
//   2) exact·Base Sepolia USDC 요구 선택 → ~/.jigap 에 x402 서명요청 작성
//   3) dev 앱 팝업 → 사람 비번 승인 → GUI가 EIP-3009 서명 → 결과 파일
//   4) 서명을 X-PAYMENT(서버가 준 version/network 에코)로 붙여 재요청 → 페이실리테이터 정산 → 200
//
// 실행: cargo run --example live_settle --manifest-path kura-mcp/Cargo.toml -- <url>
//       (url 생략 시 https://www.x402.org/protected)

use kura_mcp::{payment, x402};

#[tokio::main]
async fn main() {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://www.x402.org/protected".to_string());
    println!("▶ x402 라이브 정산 대상: {url}\n");

    let client = reqwest::Client::new();

    // 1) 먼저 그냥 GET. 402가 아니면 결제 불필요.
    let resp = client.get(&url).send().await.expect("요청 실패");
    let status = resp.status().as_u16();
    if status != 402 {
        let body = resp.text().await.unwrap_or_default();
        println!("결제 불필요(HTTP {status}). 본문:\n{body}");
        return;
    }

    // 2) 결제 요구 파싱 — V2는 헤더, V1은 본문. 헤더를 본문 소비 전에 챙긴다.
    let pr_header = resp
        .headers()
        .get("payment-required")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let body402 = resp.text().await.unwrap_or_default();
    let required = x402::parse_required(pr_header.as_deref(), &body402).expect("402 파싱 실패");
    let req = x402::pick_requirement(&required).expect("지원 결제 요구 선택 실패");
    let amount = x402::base_units_to_usdc(&req.amount).expect("금액 변환 실패");
    let resource = required.display_resource(&req, &url);
    println!(
        "402 수신 (x402Version={})\n  scheme={} network={}\n  amount={amount} USDC → payTo={}\n  resource={resource}\n",
        required.version, req.scheme, req.network, req.pay_to
    );

    // 3) dev 앱이 켜져 있어야 승인 팝업이 뜬다.
    if !payment::app_alive() {
        eprintln!("✗ dev 앱이 실행 중이 아니에요. `npm run tauri dev` 로 앱을 먼저 켜세요.");
        std::process::exit(1);
    }
    if payment::has_pending() {
        eprintln!("✗ 이미 대기 중인 결제 요청이 있어요. 먼저 처리하세요.");
        std::process::exit(1);
    }

    let memo = {
        let d = required.description(&req);
        if d.is_empty() {
            "x402 실엔드포인트 정산 테스트".to_string()
        } else {
            d
        }
    };
    let id = payment::write_x402_request(req.pay_to.trim(), &amount, &memo, &resource, None)
        .expect("요청 작성 실패");
    println!("→ 지갑 앱에 서명 요청 보냄. 팝업에서 비번으로 승인하세요(최대 5분)…\n");

    let result = match payment::await_result(&id, payment::APPROVAL_TIMEOUT).await {
        Some(r) => r,
        None => {
            payment::cancel_request(&id);
            eprintln!("✗ 승인 시간 초과(5분).");
            std::process::exit(1);
        }
    };
    if result.status != "approved" {
        eprintln!(
            "✗ 승인되지 않음: status={} detail={}",
            result.status, result.detail
        );
        std::process::exit(1);
    }
    let signed = result.x402.expect("승인됐는데 서명 페이로드가 없음");
    println!("✓ 사람 승인 + GUI 서명 완료. X-PAYMENT 조립 후 재요청…\n");

    // 서명 페이로드를 저장 → 이후 GUI 승인 없이 send 포맷만 바꿔가며 재시도(replay) 가능.
    // (validBefore 유효창 동안. 서명은 nonce/value 고정이라 정산 전까지 여러 번 verify 가능.)
    let saved = serde_json::json!({
        "url": url, "version": required.version, "scheme": req.scheme, "network": req.network,
        "signed": signed,
    });
    let _ = std::fs::write("/tmp/x402_replay.json", saved.to_string());
    println!("  (서명 저장: /tmp/x402_replay.json)\n");

    // 4) 결제 헤더(V2=PAYMENT-SIGNATURE / V1=X-PAYMENT) 붙여 재요청 → 페이실리테이터 정산.
    let sub = required
        .build_submission(&req, &signed)
        .expect("결제 헤더 조립 실패");
    if let Ok(d) = base64_decode(&sub.value) {
        println!("  보낸 {}(디코드): {d}\n", sub.header_name);
    }
    let paid = client
        .get(&url)
        .header(sub.header_name, &sub.value)
        .send()
        .await
        .expect("결제 재요청 실패");
    let paid_status = paid.status().as_u16();
    println!("◀ 재요청 응답: HTTP {paid_status}");
    // 실패 진단: 응답 헤더 전부 + 재발급된 payment-required(있으면) 디코드.
    for (k, v) in paid.headers().iter() {
        println!("    [hdr] {k}: {}", v.to_str().unwrap_or("<비ASCII>"));
    }
    let settlement = paid
        .headers()
        .get(sub.response_header)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .unwrap_or_default();
    let re_pr = paid
        .headers()
        .get("payment-required")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    if !settlement.is_empty() {
        println!("  X-PAYMENT-RESPONSE(정산 증빙): {settlement}");
        if let Ok(decoded) = base64_decode(&settlement) {
            println!("    └ 디코드: {decoded}");
        }
        // MCP 도구(x402_fetch)와 동일하게 정산 tx를 기록 → GUI가 내역 "signed"를 "정산됨"으로 갱신.
        if let Some((tx, success)) = x402::parse_settlement(&settlement) {
            match payment::record_settlement(&signed.authorization.nonce, &tx, success) {
                Ok(()) => println!("  정산 기록됨 → GUI 내역 갱신 대상: tx={tx} success={success}"),
                Err(e) => println!("  ⚠ 정산 기록 실패: {e}"),
            }
        }
    }
    if let Some(pr) = re_pr {
        if let Ok(decoded) = base64_decode(&pr) {
            println!("  재발급 payment-required(디코드, 에러 사유 가능): {decoded}");
        }
    }
    let body = paid.text().await.unwrap_or_default();
    println!("  본문:\n{body}");

    if (200..300).contains(&paid_status) {
        println!("\n🎉 라이브 정산 성공 — 실 엔드포인트가 결제를 받고 콘텐츠를 줬습니다.");
    } else {
        println!("\n⚠ 정산 실패(HTTP {paid_status}). 위 본문에서 사유 확인 → send 포맷 점검 필요.");
    }
}

/// 정산 증빙 헤더(base64)를 사람이 읽게 디코드해 본다(실패해도 무해).
fn base64_decode(s: &str) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let bytes = B64.decode(s.trim()).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}
