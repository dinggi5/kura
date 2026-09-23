// 온체인 송금 + 잔액 조회 (Session 3~6).
// do_send_* 코어(서명자 인자)와 비번 래퍼를 분리 — 자율 승인 경로(session)가 코어를 공유한다.
// 코어가 긴급 잠금·단일/일일 한도·내역·누적 기록을 모두 적용한다.

use crate::i18n::{tf, ts};
use alloy::network::{EthereumWallet, TransactionBuilder};
use alloy::primitives::{
    utils::{format_ether, format_units},
    Address, U256,
};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::TransactionRequest;
use alloy::signers::local::PrivateKeySigner;
use serde::Serialize;
use zeroize::Zeroizing;

use crate::chain::{active_chain, with_pinned_chain, IERC20};
use crate::history::log_attempt;
use crate::limits::{parse_eth_nonneg, parse_usdc_nonneg, refund_spend, reserve_spend};
use crate::lock::read_lock;
use crate::settings::{effective_rpc, read_settings, redact_urls};
use crate::trusted::record_trusted;
use crate::wallet::{active_account_index, unlock_signer, with_pinned_account};
use alloy::sol_types::SolCall;

/// 잔액 — 보기 좋게 다듬기 전의 십진수 문자열.
///
/// `eth` = **네이티브(가스) 토큰 잔액. 그게 USDC 와 다른 자산인 체인에서만 있다** (개발 50).
/// Arc 처럼 네이티브가 곧 USDC 인 체인에선 아예 내보내지 않는다 — 같은 잔액을 18dp 뷰로 한 번 더
/// 담으면 화면이든 AI 든 **같은 돈을 두 번 세기** 때문이다. "보내 놓고 안 보여주기"가 아니라
/// 애초에 안 만드는 쪽을 골랐다(빠뜨리기 쉬운 곳을 구조로 없앤다).
#[derive(Serialize)]
pub(crate) struct Balances {
    #[serde(skip_serializing_if = "Option::is_none")]
    eth: Option<String>,
    usdc: String,
}

/// 체인/RPC 전송 에러를 사람이 읽을 수 있는 한국어로 바꾼다. alloy 의 revert 에러는
/// "server returned an error response: error code 3: execution reverted: ERC20: transfer amount
/// exceeds balance, data: \"0x08c3...\"" 처럼 길고 hex 가 붙어 그대로 보여주면 못 읽는다 →
/// 흔한 원인은 또렷한 안내로 매핑하고, 모르는 건 서버 프리픽스·hex data 노이즈를 떼어 간결하게.
/// (이 메시지는 GUI 승인 모달·거래 내역·CLI/MCP 결과에 그대로 노출된다.)
/// `token` = 호출 문맥("USDC"/"ETH") — 막연한 "exceeds balance" 류를 토큰에 맞게 안내하려고.
pub(crate) fn humanize_chain_error(raw: &str, token: &str) -> String {
    let low = raw.to_lowercase();
    // ERC20 transfer 가 잔액 초과로 revert — USDC 경로에서만 나는 구체 revert 사유.
    if low.contains("transfer amount exceeds balance") {
        return ts!(
            "USDC 잔액이 부족해요. 충전 후 다시 시도하세요.",
            "Not enough USDC. Top up and try again."
        )
        .into();
    }
    // 가스(또는 ETH 송금액) 부족 — 트랜잭션을 낼 가스 토큰이 모자람(토큰 인자와 무관 — 가스 토큰의 문제다).
    // 🔴 가스가 곧 USDC 인 체인(Arc)에선 「ETH가 부족해요」가 틀린 말이다(개발 66, 실물 RPC 하네스에서 발견) —
    // 사용자는 가진 적도 없는 ETH 를 사러 간다.
    if low.contains("insufficient funds") {
        if active_chain().native_is_usdc {
            return ts!(
                "USDC가 부족해요(가스 포함). 이 체인은 가스도 USDC로 내요 — 조금 충전한 뒤 다시 시도하세요.",
                "Not enough USDC (gas included). Gas on this chain is paid in USDC — add a little and try again."
            )
            .into();
        }
        return ts!(
            "ETH가 부족해요(가스 포함). ETH를 조금 충전한 뒤 다시 시도하세요.",
            "Not enough ETH (gas included). Add a little ETH and try again."
        )
        .into();
    }
    // 그 밖의 "exceeds balance" 류는 토큰 문맥에 맞춰 안내(ETH 경로를 USDC 부족으로 오안내 방지).
    if low.contains("exceeds balance") {
        return tf!(
            "{token} 잔액이 부족해요. 충전 후 다시 시도하세요.",
            "Not enough {token}. Top up and try again."
        );
    }
    // execution reverted: <사유> 만 뽑고 뒤의 data hex 는 버린다.
    if let Some(idx) = raw.find("execution reverted:") {
        let after = &raw[idx + "execution reverted:".len()..];
        let reason = after.split(", data:").next().unwrap_or(after).trim();
        if !reason.is_empty() {
            return tf!(
                "전송이 거부됐어요: {reason}",
                "The transfer was rejected: {reason}"
            );
        }
        return ts!(
            "전송이 체인에서 거부됐어요.",
            "The chain rejected the transfer."
        )
        .into();
    }
    // 알 수 없는 에러: 서버 프리픽스·hex data 노이즈 제거 후 간결하게.
    let cleaned = raw
        .split(", data:")
        .next()
        .unwrap_or(raw)
        .replace("server returned an error response: ", "")
        .trim()
        .to_string();
    if cleaned.is_empty() {
        ts!("전송에 실패했어요.", "The transfer failed.").into()
    } else {
        tf!(
            "전송에 실패했어요: {cleaned}",
            "The transfer failed: {cleaned}"
        )
    }
}

/// 지갑 주소의 네이티브(가스용) + USDC(결제용) 잔액을 활성 체인에서 조회한다.
#[tauri::command]
pub(crate) async fn get_balances(addr_hex: String) -> Result<Balances, String> {
    let addr: Address = addr_hex
        .parse()
        .map_err(|e| tf!("주소 파싱 실패: {e}", "Couldn't read that address: {e}"))?;

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

    // ETH와 USDC 잔액을 동시에 조회 (순차 2번 → RPC 왕복 1번 분량).
    // 네이티브가 곧 USDC 인 체인(Arc)에선 네이티브 조회를 **아예 하지 않는다** — 같은 잔액이라
    // 쓸 데가 없고, RPC 왕복도 하나 준다.
    let chain = active_chain();
    let usdc_contract = IERC20::new(chain.usdc_address, &provider);
    let (wei, raw): (Option<U256>, U256) = tokio::try_join!(
        async {
            if chain.native_is_usdc {
                return Ok(None);
            }
            provider.get_balance(addr).await.map(Some).map_err(|e| {
                tf!(
                    "ETH 잔액 조회 실패: {}",
                    "Couldn't read your ETH balance: {}",
                    redact_urls(&e.to_string())
                )
            })
        },
        async {
            usdc_contract.balanceOf(addr).call().await.map_err(|e| {
                tf!(
                    "USDC 잔액 조회 실패: {}",
                    "Couldn't read your USDC balance: {}",
                    redact_urls(&e.to_string())
                )
            })
        },
    )?;

    let eth = wei.map(format_ether);
    let usdc = format_units(raw, chain.usdc_decimals).map_err(|e| {
        tf!(
            "USDC 단위 변환 실패: {e}",
            "Couldn't convert the USDC amount: {e}"
        )
    })?;

    Ok(Balances { eth, usdc })
}

/// 활성 체인의 USDC 잔액을 **base unit 정수**로 읽는다 (표시용 문자열 X — 비교에 쓰는 값).
/// 자율 승인의 가스 여유분 검사(session.rs)가 쓴다. `get_balances` 와 달리 네이티브는 안 읽는다.
pub(crate) async fn usdc_balance_units(addr: Address) -> Result<U256, String> {
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
    IERC20::new(active_chain().usdc_address, &provider)
        .balanceOf(addr)
        .call()
        .await
        .map_err(|e| {
            tf!(
                "USDC 잔액 조회 실패: {}",
                "Couldn't read your USDC balance: {}",
                redact_urls(&e.to_string())
            )
        })
}

/// 받는 주소 문자열을 파싱한다 (ETH/USDC 송금·x402 서명 공용).
pub(crate) fn parse_to_addr(to: &str) -> Result<Address, String> {
    to.trim().parse().map_err(|e| {
        tf!(
            "받는 주소가 올바르지 않습니다: {e}",
            "That recipient address isn't valid: {e}"
        )
    })
}

// ---------- 전송: 「확실히 안 나감」과 「나갔는지 모름」을 가른다 (개발 66) ----------
//
// 개발 65 까지 송금 세 갈래(ETH·USDC·x402 직접 제출)는 `.send()` 하나로 채우기·서명·제출을 한꺼번에
// 했고, 오류가 나면 전부 「안 나갔다」로 보고 한도를 환불하고 "failed" 로 적었다. 그런데 오류가
// **제출 뒤**에 났다면 — RPC 가 tx 를 받아 퍼뜨린 뒤 응답만 유실됐다면 — 돈은 나갔다. 그걸 실패라고
// 말하면 사람은 다시 누르고 AI 는 다시 요청한다. 새 nonce 로 **한 번 더** 나간다(개발 64 코덱스 P1).
//
// 그래서 둘로 나눈다. ① 채우기·서명(여기까지의 실패는 확실히 안 나감) ② 제출. 서명이 끝난 순간
// **tx 해시를 이미 안다** — 제출 결과를 모르면 그 해시를 들고 「불명」으로 돌려준다.
// 그리고 같은 서명 바이트를 다시 내는 건 **멱등**이다(같은 nonce·같은 해시 — 두 번 나갈 수 없다).
// 그래서 불명이면 한 번 되묻고(해시 조회) 한 번 다시 낸다 — 대부분의 「응답만 유실」은 여기서 풀린다.

/// 채우기(nonce·가스·수수료 조회 + 서명) 상한. 여기서 멈추면 아무것도 안 나갔다 → 확실한 실패.
const FILL_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
/// 제출(eth_sendRawTransaction) 상한. 🔴 이걸 넘기면 「실패」가 아니라 「불명」이다 — 개발 51 이 이 자리에
/// 상한을 **못** 걸었던 이유(상한 = 실패로 둔갑)가 불명 상태가 생기면서 사라졌다.
const SEND_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
/// 불명일 때 해시 되묻기 상한.
const LOOKUP_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// 송금 코어의 실패. 둘을 한 문자열로 뭉치면 호출자가 한도를 환불하고 요청을 되살려 **다시 보낼 수 있게** 된다.
#[derive(Debug)]
pub(crate) enum SendError {
    /// 확실히 아무것도 안 나갔다 — 한도는 환불됐고, 다시 시도해도 된다.
    Failed(String),
    /// 서명한 tx 를 냈는데 받혔는지 모른다. 한도는 **환불하지 않았고**, 다시 보내면 두 번 나갈 수 있다.
    Unknown { hash: String, msg: String },
}

impl From<String> for SendError {
    fn from(e: String) -> Self {
        SendError::Failed(e)
    }
}

impl From<&str> for SendError {
    fn from(e: &str) -> Self {
        SendError::Failed(e.to_string())
    }
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::Failed(m) => f.write_str(m),
            SendError::Unknown { msg, .. } => f.write_str(msg),
        }
    }
}

impl SendError {
    /// 사람이 읽을 한 문장 — 화면에서 직접 보낸 송금(`send_usdc` 커맨드)처럼 문자열 오류만 나갈 수
    /// 있는 자리용. 불명이면 「다시 보내지 말라」가 문장에 들어가야 한다.
    pub(crate) fn into_message(self) -> String {
        match self {
            SendError::Failed(m) => m,
            SendError::Unknown { msg, .. } => msg,
        }
    }

    /// 사람이 읽을 문장(빌림) — 테스트·로그용.
    #[cfg(test)]
    pub(crate) fn contains(&self, pat: &str) -> bool {
        self.to_string().contains(pat)
    }
}

/// 제출 오류의 판정.
#[derive(Debug, PartialEq)]
enum SendFault {
    /// 노드가 받아서 거절했다(JSON-RPC 오류 응답·HTTP 4xx) 또는 보내기 전에 로컬에서 실패 — 안 나갔다.
    Rejected,
    /// 노드가 「이미 가지고 있다」고 답했다 — 같은 tx 가 이미 들어가 있다 = 나갔다.
    AlreadyKnown,
    /// 요청이 노드에 닿았는지·처리됐는지 모른다(연결 끊김·5xx·응답 해석 실패·빈 응답).
    Unknown,
}

/// alloy 의 전송 오류를 셋으로 가른다 (순수 — 테스트용).
///
/// 기준은 「노드가 이 요청을 처리했다는 증거가 있는가」다. JSON-RPC 오류 응답은 노드가 읽고 거절한 것이고,
/// 4xx 는 요청을 처리하기 전에 막은 것이다(요율 제한·인증). 5xx 는 앞단(게이트웨이)이 뒤로 넘긴 뒤 났을 수
/// 있어 모른다. 연결 수준 오류(`Custom`)는 연결 전 실패일 수도 있지만 우리가 가를 수 없다 — 모르는 쪽으로
/// 접는다. 불명은 되묻기·재제출로 대부분 풀리므로 이 보수가 비싸지 않다.
fn classify_send_error(e: &alloy::providers::transport::TransportError) -> SendFault {
    use alloy::providers::transport::{RpcError, TransportErrorKind};
    match e {
        RpcError::ErrorResp(p) => {
            let m = p.message.to_lowercase();
            // geth/reth/erigon "already known", nethermind "AlreadyKnown", besu "Known transaction".
            if m.contains("already known")
                || m.contains("alreadyknown")
                || m.contains("known transaction")
            {
                SendFault::AlreadyKnown
            } else {
                SendFault::Rejected
            }
        }
        RpcError::SerError(_) | RpcError::LocalUsageError(_) | RpcError::UnsupportedFeature(_) => {
            SendFault::Rejected
        }
        RpcError::Transport(TransportErrorKind::HttpError(h)) if (400..500).contains(&h.status) => {
            SendFault::Rejected
        }
        _ => SendFault::Unknown,
    }
}

/// 트랜잭션을 채워 서명하고 제출한다. 반환: 확실히 받혔으면 해시, 아니면 `SendError`.
///
/// 호출자는 `Box::pin` 으로 부른다 — alloy 제공자의 채우기 future 가 깊어서, 승인 경로(체인·계정 고정이
/// 몇 겹 둘러싼)에 그대로 넣으면 컴파일러의 타입 깊이 한도를 넘는다(개발 66 실측).
///
/// 한도 환불·내역 기록은 **호출자**가 한다(갈래마다 내역 문구가 다르다) — 이 함수는 판정만 정확히 돌려준다.
/// `token` = 오류 문구 문맥(`humanize_chain_error`).
pub(crate) async fn broadcast(
    signer: &PrivateKeySigner,
    tx: TransactionRequest,
    token: &str,
) -> Result<String, SendError> {
    broadcast_via(&effective_rpc(), signer, tx, token).await
}

/// `broadcast` 의 속알맹이 — RPC 주소를 받는다. 테스트가 **가짜 RPC**(응답을 일부러 끊는)로 불명 갈래를
/// 밟으려고 갈라 뒀다 — 실제 노드로는 「받았는데 응답만 유실」을 만들 수 없다.
async fn broadcast_via(
    rpc: &str,
    signer: &PrivateKeySigner,
    tx: TransactionRequest,
    token: &str,
) -> Result<String, SendError> {
    use alloy::network::eip2718::Encodable2718;

    // 제공자를 여기서 만든다 — 구체 타입이어야 채우기(`fill`)를 따로 부를 수 있다. (예전의
    // `signing_provider` 는 `impl Provider` 를 돌려줘 `.send()` 한 방밖에 못 했다 — 개발 66 에 지웠다.)
    let provider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer.clone()))
        .connect(rpc)
        .await
        .map_err(|e| {
            SendError::Failed(tf!(
                "RPC 연결 실패: {}",
                "Couldn't reach the RPC server: {}",
                redact_urls(&e.to_string())
            ))
        })?;
    let humanize =
        |e: &dyn std::fmt::Display| humanize_chain_error(&redact_urls(&e.to_string()), token);

    // 🔴 **체인 ID 는 우리가 정한다** (개발 66, 코덱스 P0). 비워 두면 alloy 가 **RPC 에 물어** 채우고 그 값으로
    // 서명한다 — 설정은 테스트넷인데 사용자 RPC 주소가 메인넷이면 **메인넷용 서명**이 나가 진짜 돈이 움직인다.
    // 선택한 체인으로 못박으면(EIP-155) 엉뚱한 체인의 노드는 이 tx 를 받지 않는다(확실한 실패로 끝난다).
    let mut tx = tx;
    if tx.chain_id.is_none() {
        tx.set_chain_id(active_chain().chain_id);
    }
    // ① 채우기 + 서명. 가스 추정이 revert(잔액 부족 등)를 여기서 잡는다 — 전부 「안 나감」.
    let filled = match tokio::time::timeout(FILL_WAIT, provider.fill(tx)).await {
        Ok(Ok(f)) => f,
        Ok(Err(e)) => return Err(SendError::Failed(humanize(&e))),
        Err(_) => {
            return Err(SendError::Failed(
                ts!(
                "RPC 가 응답하지 않아 전송을 준비하지 못했어요. 아무것도 보내지 않았습니다.",
                "The RPC server didn't answer, so the transfer wasn't prepared. Nothing was sent."
            )
                .into(),
            ))
        }
    };
    let envelope = filled.try_into_envelope().map_err(|_| {
        SendError::Failed(
            ts!(
                "트랜잭션에 서명하지 못했어요. 아무것도 보내지 않았습니다.",
                "Couldn't sign the transaction. Nothing was sent."
            )
            .into(),
        )
    })?;
    let hash = envelope.tx_hash().to_string();
    let raw = envelope.encoded_2718();

    // ② 제출.
    let why = match tokio::time::timeout(SEND_WAIT, provider.send_raw_transaction(&raw)).await {
        Ok(Ok(_)) => return Ok(hash),
        Ok(Err(e)) => match classify_send_error(&e) {
            SendFault::AlreadyKnown => return Ok(hash),
            SendFault::Rejected => return Err(SendError::Failed(humanize(&e))),
            SendFault::Unknown => redact_urls(&e.to_string()),
        },
        Err(_) => ts!("제출 응답 시간 초과", "no reply to the submission").to_string(),
    };

    // ③ 불명 — 되묻고, 한 번 다시 낸다(같은 바이트라 두 번 나갈 수 없다).
    if let Ok(Ok(Some(_))) = tokio::time::timeout(
        LOOKUP_WAIT,
        provider.get_transaction_by_hash(envelope.tx_hash().to_owned()),
    )
    .await
    {
        return Ok(hash);
    }
    match tokio::time::timeout(SEND_WAIT, provider.send_raw_transaction(&raw)).await {
        Ok(Ok(_)) => return Ok(hash),
        Ok(Err(e)) if classify_send_error(&e) == SendFault::AlreadyKnown => return Ok(hash),
        _ => {}
    }
    Err(SendError::Unknown {
        msg: unknown_send_message(&hash, &why),
        hash,
    })
}

/// 불명일 때 사람·AI 에게 나가는 문장 — **다시 보내지 말라**가 핵심이다.
pub(crate) fn unknown_send_message(hash: &str, why: &str) -> String {
    tf!(
        "전송이 체인에 들어갔는지 확인하지 못했어요({why}). tx {hash} — 나갔을 수 있으니 **다시 보내기 전에** 익스플로러나 내역에서 먼저 확인하세요.",
        "Couldn't confirm whether the transfer reached the chain ({why}). tx {hash} — it may have gone through, so check the explorer or history **before sending again**."
    )
}

/// 비번으로 키를 복호화해 활성 체인에서 ETH(가스 토큰)를 송금한다. tx 해시를 돌려준다.
/// 가스가 곧 USDC 인 체인(Arc)에선 이 경로가 막혀 있다 — do_send_eth_inner 주석 참고.
#[tauri::command]
pub(crate) async fn send_eth(
    password: String,
    to: String,
    amount_eth: String,
) -> Result<String, String> {
    let password = Zeroizing::new(password);
    // 진입 시 계정을 한 번 고정 (개발 54) — 비번 검증·서명·내역이 모두 같은 계정을 본다.
    with_pinned_account(
        active_account_index(),
        send_eth_pinned(password, to, amount_eth),
    )
    .await
    .map_err(SendError::into_message)
}

/// `send_eth` 와 같되 **실패의 종류를 보존한다**(개발 66) — 결제 승인(`approve_payment`)이 쓴다.
/// 화면의 보내기 버튼은 문자열 오류만 받지만, 승인 흐름은 「불명」을 결과로 MCP 에 넘겨야 한다.
pub(crate) async fn send_eth_checked(
    password: String,
    to: String,
    amount_eth: String,
) -> Result<String, SendError> {
    let password = Zeroizing::new(password);
    with_pinned_account(
        active_account_index(),
        send_eth_pinned(password, to, amount_eth),
    )
    .await
}

async fn send_eth_pinned(
    password: Zeroizing<String>,
    to: String,
    amount_eth: String,
) -> Result<String, SendError> {
    // 비번 → 서명자. 실패하면(비번 오류 등) 시도로 기록하고 거부.
    let signer = match unlock_signer(&password) {
        Ok(s) => s,
        Err(e) => {
            log_attempt("ETH", to.trim(), amount_eth.trim(), "failed", &e);
            return Err(e.into());
        }
    };
    let to_addr = to.clone();
    let hash = do_send_eth(&signer, to, amount_eth).await?;
    record_trusted(&to_addr); // 비번(사람) 승인 성공 = 신뢰 주소 학습
    Ok(hash)
}

/// ETH 송금 코어 — 서명자가 이미 있는 상태에서 실행한다(비번 래퍼와 자율 승인 경로가 공유).
/// 긴급 잠금·단일/일일 한도·내역·누적 기록을 모두 적용한다.
pub(crate) async fn do_send_eth(
    signer: &PrivateKeySigner,
    to: String,
    amount_eth: String,
) -> Result<String, SendError> {
    // 작업 진입 시 체인·계정을 한 번 고정 — 이 송금의 한도·장부·RPC·내역이 모두 같은 체인·계정을 본다.
    with_pinned_chain(
        active_chain().chain_id,
        with_pinned_account(
            active_account_index(),
            do_send_eth_inner(signer, to, amount_eth),
        ),
    )
    .await
}

async fn do_send_eth_inner(
    signer: &PrivateKeySigner,
    to: String,
    amount_eth: String,
) -> Result<String, SendError> {
    let amt = amount_eth.trim();
    // 🔴 네이티브가 곧 USDC 인 체인(Arc)에선 네이티브 송금 경로를 닫는다 (개발 50).
    // 여기서 보내는 "1"은 1 ETH 가 아니라 **1 USDC 를 18dp 로** 옮기는 것이라, 6dp 로 세는 한도·
    // 오늘 사용액·내역과 전부 어긋난다(같은 돈이 두 장부에 다르게 남는다). 같은 일을 USDC 송금이
    // 이미 정확히 해 주므로 막는 게 기능 상실이 아니다.
    if active_chain().native_is_usdc {
        let msg = ts!(
            "이 체인은 가스도 USDC로 내요. ETH 송금 대신 USDC로 보내세요.",
            "On this chain gas is paid in USDC — send USDC instead of ETH."
        )
        .to_string();
        log_attempt("ETH", to.trim(), amt, "failed", &msg);
        return Err(msg.into());
    }
    let value = parse_eth_nonneg(amt)?;
    if value.is_zero() {
        return Err(ts!(
            "0보다 큰 금액을 입력하세요",
            "Enter an amount greater than 0"
        )
        .into());
    }
    let to_addr = parse_to_addr(&to)?;
    let to = to.trim();

    // 긴급 잠금: 켜져 있으면 모든 송금을 가장 먼저 차단한다 (비상 스위치).
    if read_lock() {
        log_attempt(
            "ETH",
            to,
            amt,
            "blocked",
            ts!("긴급 잠금", "Emergency lock"),
        );
        return Err(ts!(
            "긴급 잠금이 켜져 있어 송금이 차단됐어요. 해제 후 다시 시도하세요.",
            "Emergency lock is on, so the payment was blocked. Turn it off and try again."
        )
        .into());
    }

    // 단일 + 일일 누적 한도 검사 + 예약(낙관적 선반영). 락은 이 빠른 파일 I/O 구간만 잡는다
    // (느린 RPC 가 모든 결제를 전역 정지시키지 않게). 한도 초과면 여기서 거부.
    let settings = read_settings();
    let single = parse_eth_nonneg(&settings.single_eth)?;
    let daily = parse_eth_nonneg(&settings.daily_eth)?;
    let reserved_day = match reserve_spend("ETH", value, single, daily, 18).await {
        Ok(d) => d,
        Err(e) => {
            log_attempt("ETH", to, amt, "blocked", &e);
            return Err(e.into());
        }
    };

    // 네트워크 전송은 락 밖에서 — 실패하면 예약한 사용액을 환불한다(예약한 날에만).
    let tx = TransactionRequest::default()
        .with_to(to_addr)
        .with_value(value);
    let hash = settle_broadcast(
        Box::pin(broadcast(signer, tx, "ETH")).await,
        "ETH",
        to,
        amt,
        value,
        reserved_day,
    )
    .await?;
    Ok(hash)
}

/// 제출 결과를 내역·한도에 반영한다 — 송금 세 갈래(ETH·USDC·x402 직접 제출)가 **이 함수 하나**를 쓴다
/// (개발 64·65 의 교훈: 같은 규칙을 갈래마다 두면 한쪽이 뒤처진다).
///   받힘 → 내역 "sent" + 해시(누적 사용액은 예약 단계에서 이미 기록됨)
///   확실한 실패 → 한도 환불 + 내역 "failed"
///   불명 → 🔴 **환불하지 않는다**(나갔을 수 있다 — 한도를 되돌리면 한도 밖으로 한 번 더 나갈 길이 된다)
///          + 내역 "unknown" + 해시 + 사람에게 알림(창이 없는 자율 경로도 있어서 알림이 유일한 통로다).
pub(crate) async fn settle_broadcast(
    sent: Result<String, SendError>,
    token: &str,
    to: &str,
    amt: &str,
    value: U256,
    reserved_day: u64,
) -> Result<String, SendError> {
    match sent {
        Ok(hash) => {
            log_attempt(token, to, amt, "sent", &hash);
            Ok(hash)
        }
        Err(SendError::Failed(msg)) => {
            refund_spend(token, value, reserved_day).await;
            log_attempt(token, to, amt, "failed", &msg);
            Err(SendError::Failed(msg))
        }
        Err(SendError::Unknown { hash, msg }) => {
            log_attempt(token, to, amt, "unknown", &hash);
            crate::notify::show_notification(
                ts!("전송 확인 필요", "Transfer needs checking"),
                ts!(
                    "보낸 결제가 체인에 들어갔는지 확인하지 못했어요. 다시 보내기 전에 내역을 먼저 확인하세요.",
                    "A payment's arrival on-chain couldn't be confirmed. Check the history before sending again."
                ),
            );
            Err(SendError::Unknown { hash, msg })
        }
    }
}

/// 비번으로 키를 복호화해 활성 체인에서 USDC(ERC20)를 송금한다. tx 해시를 돌려준다.
/// (가스는 ETH로 지불되므로 ETH 잔액도 약간 필요하다.)
#[tauri::command]
pub(crate) async fn send_usdc(
    password: String,
    to: String,
    amount_usdc: String,
) -> Result<String, String> {
    let password = Zeroizing::new(password);
    // 진입 시 계정을 한 번 고정 (개발 54) — send_eth 와 같은 이유.
    with_pinned_account(
        active_account_index(),
        send_usdc_pinned(password, to, amount_usdc),
    )
    .await
    .map_err(SendError::into_message)
}

/// `send_usdc` 와 같되 **실패의 종류를 보존한다**(개발 66) — 결제 승인(`approve_payment`)이 쓴다.
/// 화면의 보내기 버튼은 문자열 오류만 받지만, 승인 흐름은 「불명」을 결과로 MCP 에 넘겨야 한다.
pub(crate) async fn send_usdc_checked(
    password: String,
    to: String,
    amount_usdc: String,
) -> Result<String, SendError> {
    let password = Zeroizing::new(password);
    with_pinned_account(
        active_account_index(),
        send_usdc_pinned(password, to, amount_usdc),
    )
    .await
}

async fn send_usdc_pinned(
    password: Zeroizing<String>,
    to: String,
    amount_usdc: String,
) -> Result<String, SendError> {
    let signer = match unlock_signer(&password) {
        Ok(s) => s,
        Err(e) => {
            log_attempt("USDC", to.trim(), amount_usdc.trim(), "failed", &e);
            return Err(e.into());
        }
    };
    let to_addr = to.clone();
    let hash = do_send_usdc(&signer, to, amount_usdc).await?;
    record_trusted(&to_addr); // 비번(사람) 승인 성공 = 신뢰 주소 학습
    Ok(hash)
}

/// USDC 송금 코어 — 서명자가 이미 있는 상태에서 실행한다(비번 래퍼와 자율 승인 경로가 공유).
pub(crate) async fn do_send_usdc(
    signer: &PrivateKeySigner,
    to: String,
    amount_usdc: String,
) -> Result<String, SendError> {
    // 작업 진입 시 체인·계정 고정 — decimals·USDC 컨트랙트·RPC·한도·장부·내역이 모두 같은 체인·계정.
    with_pinned_chain(
        active_chain().chain_id,
        with_pinned_account(
            active_account_index(),
            do_send_usdc_inner(signer, to, amount_usdc),
        ),
    )
    .await
}

async fn do_send_usdc_inner(
    signer: &PrivateKeySigner,
    to: String,
    amount_usdc: String,
) -> Result<String, SendError> {
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

    // 긴급 잠금: 켜져 있으면 모든 송금을 가장 먼저 차단한다 (비상 스위치).
    if read_lock() {
        log_attempt(
            "USDC",
            to,
            amt,
            "blocked",
            ts!("긴급 잠금", "Emergency lock"),
        );
        return Err(ts!(
            "긴급 잠금이 켜져 있어 송금이 차단됐어요. 해제 후 다시 시도하세요.",
            "Emergency lock is on, so the payment was blocked. Turn it off and try again."
        )
        .into());
    }

    // 한도 검사 + 예약 (do_send_eth 와 동일 — 락은 빠른 파일 I/O 만, 네트워크 전송은 락 밖).
    let settings = read_settings();
    let single: U256 = parse_usdc_nonneg(&settings.single_usdc, dec)?;
    let daily: U256 = parse_usdc_nonneg(&settings.daily_usdc, dec)?;
    let reserved_day = match reserve_spend("USDC", value, single, daily, dec).await {
        Ok(d) => d,
        Err(e) => {
            log_attempt("USDC", to, amt, "blocked", &e);
            return Err(e.into());
        }
    };

    // 네트워크 전송은 락 밖에서 — 실패하면 예약한 사용액을 환불한다(예약한 날에만, settle_broadcast).
    let tx = TransactionRequest::default()
        .with_to(active_chain().usdc_address)
        .with_input(
            IERC20::transferCall {
                to: to_addr,
                amount: value,
            }
            .abi_encode(),
        );
    settle_broadcast(
        Box::pin(broadcast(signer, tx, "USDC")).await,
        "USDC",
        to,
        amt,
        value,
        reserved_day,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    // USDC 송금 calldata 가 표준 ERC20 transfer(address,uint256) 인코딩과 일치해야 한다.
    // (네트워크/잔액 없이 ABI 인코딩만 검증.)
    #[test]
    fn usdc_transfer_calldata_is_standard() {
        use alloy::sol_types::SolCall;
        let to = address!("0x00000000000000000000000000000000000000Ad");
        let call = IERC20::transferCall {
            to,
            amount: U256::from(1_000_000u64), // 1 USDC (6 decimals)
        };
        let data = call.abi_encode();
        // transfer(address,uint256) 셀렉터 = 0xa9059cbb
        assert_eq!(&data[..4], &[0xa9, 0x05, 0x9c, 0xbb]);
        // 인자 2개(주소+금액) = 64바이트 → 셀렉터 포함 68바이트
        assert_eq!(data.len(), 68);
    }

    /// 🔴 **제출 오류의 판정** (개발 66, 개발 64 「다음」 3번) — 노드가 읽고 거절한 것만 「안 나감」이다.
    /// 연결·5xx·해석 실패는 「모름」 — 예전엔 이것도 전부 「안 나감」이라 한도를 환불하고 다시 보낼 수 있었다.
    #[test]
    fn send_errors_split_rejected_from_unknown() {
        use alloy::providers::transport::{RpcError, TransportError, TransportErrorKind};
        let resp = |msg: &str| -> TransportError {
            RpcError::ErrorResp(
                serde_json::from_str(&format!(r#"{{"code":-32000,"message":"{msg}"}}"#)).unwrap(),
            )
        };
        // 노드가 받아서 거절 — 안 나갔다.
        assert_eq!(
            classify_send_error(&resp("nonce too low")),
            SendFault::Rejected
        );
        assert_eq!(
            classify_send_error(&resp("insufficient funds for gas * price + value")),
            SendFault::Rejected
        );
        // 이미 가지고 있다 — 같은 tx 가 들어가 있다 = 나갔다(클라이언트마다 문구가 다르다).
        for m in ["already known", "AlreadyKnown", "Known transaction: 0xabc"] {
            assert_eq!(
                classify_send_error(&resp(m)),
                SendFault::AlreadyKnown,
                "{m}"
            );
        }
        // HTTP 4xx = 처리 전에 막힘(요율 제한·인증) / 5xx = 앞단 뒤에서 났을 수 있어 모름.
        assert_eq!(
            classify_send_error(&TransportErrorKind::http_error(429, String::new())),
            SendFault::Rejected
        );
        assert_eq!(
            classify_send_error(&TransportErrorKind::http_error(502, String::new())),
            SendFault::Unknown
        );
        // 연결 끊김·빈 응답·해석 실패 — 모른다.
        assert_eq!(
            classify_send_error(&TransportErrorKind::custom_str("connection reset")),
            SendFault::Unknown
        );
        assert_eq!(classify_send_error(&RpcError::NullResp), SendFault::Unknown);
    }

    // ── 가짜 RPC: 「받았는데 응답만 유실」을 만든다 (개발 66) ─────────────────────────────────
    //
    // 실제 노드로는 불명 갈래를 못 밟는다. std 스레드 하나로 JSON-RPC 를 흉내 내고, 제출
    // (eth_sendRawTransaction)에만 대본대로 반응한다: 연결을 그냥 끊기 / HTTP 오류 / JSON-RPC 오류 / 성공.
    // 채우기에 필요한 조회(체인 id·nonce·가스)는 그럴듯한 값으로 답한다.

    #[derive(Clone, Copy)]
    enum Act {
        /// 요청을 다 읽고 **응답 없이** 연결을 닫는다 = 노드가 받았는지 모르는 상태.
        Drop,
        Http(u16),
        RpcErr(&'static str),
        Ok,
    }

    struct FakeRpc {
        url: String,
        /// 받은 제출들의 raw tx(hex) — 재제출이 **같은 바이트**인지 본다.
        raws: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    fn fake_rpc(script: Vec<Act>) -> FakeRpc {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let raws = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let raws2 = raws.clone();
        let mut script: std::collections::VecDeque<Act> = script.into();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut st) = stream else { continue };
                // 헤더 → Content-Length → 본문.
                let mut buf = Vec::new();
                let mut byte = [0u8; 1];
                while !buf.ends_with(b"\r\n\r\n") {
                    if st.read(&mut byte).unwrap_or(0) == 0 {
                        break;
                    }
                    buf.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&buf).to_lowercase();
                let len: usize = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                let mut body = vec![0u8; len];
                let _ = st.read_exact(&mut body);
                let req: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                let id = req["id"].clone();
                let method = req["method"].as_str().unwrap_or("").to_string();
                let ok = |result: serde_json::Value| serde_json::json!({"jsonrpc":"2.0","id":id,"result":result});
                let reply: Option<(u16, serde_json::Value)> = match method.as_str() {
                    "eth_chainId" => Some((200, ok("0x4cf2c2".into()))),
                    "eth_getTransactionCount" => Some((200, ok("0x7".into()))),
                    "eth_estimateGas" => Some((200, ok("0x5208".into()))),
                    "eth_gasPrice" | "eth_maxPriorityFeePerGas" => {
                        Some((200, ok("0x3b9aca00".into())))
                    }
                    "eth_feeHistory" => Some((
                        200,
                        ok(serde_json::json!({
                            "oldestBlock":"0x1","baseFeePerGas":["0x3b9aca00","0x3b9aca00"],
                            "gasUsedRatio":[0.5],"reward":[["0x3b9aca00"]]
                        })),
                    )),
                    "eth_getBlockByNumber" => Some((200, ok(serde_json::Value::Null))),
                    // 되묻기 — 항상 「모른다」. 재제출이 풀어 주는지 보려고.
                    "eth_getTransactionByHash" => Some((200, ok(serde_json::Value::Null))),
                    "eth_sendRawTransaction" => {
                        let raw = req["params"][0].as_str().unwrap_or("").to_string();
                        raws2.lock().unwrap().push(raw.clone());
                        match script.pop_front().unwrap_or(Act::Drop) {
                            Act::Drop => None,
                            Act::Http(code) => Some((code, serde_json::json!({}))),
                            Act::RpcErr(m) => Some((
                                200,
                                serde_json::json!({"jsonrpc":"2.0","id":id,
                                    "error":{"code":-32000,"message":m}}),
                            )),
                            Act::Ok => {
                                let bytes = alloy::hex::decode(&raw).unwrap();
                                let h = alloy::primitives::keccak256(bytes).to_string();
                                Some((200, ok(h.into())))
                            }
                        }
                    }
                    other => Some((
                        200,
                        serde_json::json!({"jsonrpc":"2.0","id":id,
                            "error":{"code":-32601,"message":format!("fake: {other}")}}),
                    )),
                };
                if let Some((code, v)) = reply {
                    let b = v.to_string();
                    let _ = write!(
                        st,
                        "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}",
                        b.len()
                    );
                }
                // Drop 이면 아무것도 안 쓰고 닫는다.
            }
        });
        FakeRpc { url, raws }
    }

    fn native_tx() -> TransactionRequest {
        TransactionRequest::default()
            .with_to(Address::from([0x22u8; 20]))
            .with_value(U256::from(1u64))
    }

    /// 제출 두 번이 **같은 바이트**였고, 돌려준 해시가 그 바이트의 keccak 인가 — 「재제출은 멱등」의 증거.
    fn assert_same_raw(f: &FakeRpc, hash: &str, sends: usize) {
        let raws = f.raws.lock().unwrap().clone();
        assert_eq!(raws.len(), sends, "제출 횟수");
        assert!(
            raws.windows(2).all(|w| w[0] == w[1]),
            "재제출은 같은 서명 바이트여야 한다"
        );
        let h = alloy::primitives::keccak256(alloy::hex::decode(&raws[0]).unwrap()).to_string();
        assert_eq!(h, hash, "해시는 제출한 바이트의 것이어야 한다");
    }

    /// 🔴 응답만 유실 → 다시 냈더니 「already known」 = **나갔다**(Sent). 예전엔 첫 오류에서 「실패」라
    /// 적고 한도를 환불했다 — 사람이 다시 누르면 새 nonce 로 두 번째가 나갔다.
    #[tokio::test]
    async fn lost_reply_then_already_known_is_sent() {
        let f = fake_rpc(vec![Act::Drop, Act::RpcErr("already known")]);
        let signer = PrivateKeySigner::random();
        let hash = broadcast_via(&f.url, &signer, native_tx(), "USDC")
            .await
            .expect("already known 은 받힌 것이다");
        assert_same_raw(&f, &hash, 2);
    }

    /// 응답이 두 번 다 유실 → **불명**(해시는 안다). 확실한 실패가 아니다.
    #[tokio::test]
    async fn lost_twice_is_unknown_with_the_hash() {
        let f = fake_rpc(vec![Act::Drop, Act::Drop]);
        let signer = PrivateKeySigner::random();
        match broadcast_via(&f.url, &signer, native_tx(), "USDC").await {
            Err(SendError::Unknown { hash, msg }) => {
                assert!(msg.contains(&hash), "{msg}");
                assert_same_raw(&f, &hash, 2);
            }
            other => panic!("불명이어야 한다: {other:?}"),
        }
    }

    /// 앞단 5xx 는 모름 → 재제출이 성공하면 나간 것.
    #[tokio::test]
    async fn gateway_error_then_ok_is_sent() {
        let f = fake_rpc(vec![Act::Http(502), Act::Ok]);
        let signer = PrivateKeySigner::random();
        let hash = broadcast_via(&f.url, &signer, native_tx(), "USDC")
            .await
            .unwrap();
        assert_same_raw(&f, &hash, 2);
    }

    /// 노드가 읽고 거절 → 확실한 실패, **다시 내지 않는다**(제출 1회).
    #[tokio::test]
    async fn node_rejection_is_final_and_not_resent() {
        let f = fake_rpc(vec![Act::RpcErr("nonce too low")]);
        let signer = PrivateKeySigner::random();
        match broadcast_via(&f.url, &signer, native_tx(), "USDC").await {
            Err(SendError::Failed(_)) => {}
            other => panic!("확실한 실패여야 한다: {other:?}"),
        }
        assert_eq!(f.raws.lock().unwrap().len(), 1);
    }

    /// 🔴 RPC 가 다른 체인을 말해도 서명은 **선택한 체인**으로 한다(개발 66, 코덱스 P0). 가짜 RPC 는 Arc 테스트넷
    /// (0x4cf2c2)이라고 답하고, 활성 체인은 Base Sepolia 로 고정한다 → 서명된 tx 의 체인 ID 는 84532 여야 한다.
    #[tokio::test]
    async fn signs_for_the_selected_chain_not_the_rpcs() {
        use alloy::consensus::{Transaction, TxEnvelope};
        use alloy::network::eip2718::Decodable2718;
        let f = fake_rpc(vec![Act::Ok]);
        let signer = PrivateKeySigner::random();
        let base = crate::chain::BASE_SEPOLIA.chain_id;
        with_pinned_chain(base, broadcast_via(&f.url, &signer, native_tx(), "USDC"))
            .await
            .unwrap();
        let raw = alloy::hex::decode(&f.raws.lock().unwrap()[0]).unwrap();
        let env = TxEnvelope::decode_2718(&mut raw.as_slice()).unwrap();
        assert_eq!(env.chain_id(), Some(base));
    }

    /// 한 번에 받히면 그대로 — 제출 1회.
    #[tokio::test]
    async fn plain_success_sends_once() {
        let f = fake_rpc(vec![Act::Ok]);
        let signer = PrivateKeySigner::random();
        let hash = broadcast_via(&f.url, &signer, native_tx(), "USDC")
            .await
            .unwrap();
        assert_same_raw(&f, &hash, 1);
    }

    /// 🔴 **실물 RPC 로 새 전송 함수를 밟는다 — 돈 0원** (개발 66). 잔액 0인 새 키라 체인은 반드시 거절한다.
    /// 판정이 「확실한 실패」로 나와야 한다: ① USDC 송금은 **채우기(가스 추정)** 에서 revert 로 막히고
    /// ② 0원 네이티브 전송은 채우기·서명을 **통과해** 해시까지 만든 뒤 **제출**에서 「가스 부족」으로 막힌다
    /// — 즉 ②는 fill → 서명 → 해시 → eth_sendRawTransaction → JSON-RPC 오류 판정까지 한 줄을 다 지난다.
    /// 어느 쪽도 불명으로 나오면 안 된다(그러면 사람에게 「나갔을 수 있다」는 거짓 경고가 뜬다).
    /// ⚠️ `effective_rpc` 가 실제 ~/.jigap/settings.json 의 사용자 RPC 를 읽는다(읽기만 한다).
    #[tokio::test]
    #[ignore = "네트워크 필요 — Arc 테스트넷 RPC (잔액 0 키, 돈 안 듦)"]
    async fn broadcast_from_an_empty_key_is_a_definite_failure() {
        use crate::chain::ARC_TESTNET;
        let signer = PrivateKeySigner::random();
        let to = Address::from([0x11u8; 20]);
        with_pinned_chain(ARC_TESTNET.chain_id, async {
            // ① USDC 0.01 — 가스 추정이 revert.
            let tx = TransactionRequest::default()
                .with_to(ARC_TESTNET.usdc_address)
                .with_input(
                    IERC20::transferCall {
                        to,
                        amount: U256::from(10_000u64),
                    }
                    .abi_encode(),
                );
            match broadcast(&signer, tx, "USDC").await {
                Err(SendError::Failed(m)) => println!("① Failed: {m}"),
                other => panic!("① 확실한 실패여야 한다: {other:?}"),
            }
            // ② 0원 네이티브 — 서명·해시까지 가고 제출에서 거절.
            let tx = TransactionRequest::default()
                .with_to(to)
                .with_value(U256::ZERO);
            match broadcast(&signer, tx, "USDC").await {
                Err(SendError::Failed(m)) => println!("② Failed: {m}"),
                other => panic!("② 확실한 실패여야 한다: {other:?}"),
            }
        })
        .await;
    }

    /// 가스 부족 문구는 **가스 토큰**을 말한다 — Arc 에선 USDC(개발 66).
    #[tokio::test]
    async fn insufficient_gas_names_the_gas_token() {
        let raw = "insufficient funds for gas * price + value: have 0 want 1";
        // 체인은 명시적으로 고정한다 — 고정하지 않은 테스트의 활성 체인은 **실제 settings.json** 을 따라간다.
        let base = with_pinned_chain(crate::chain::BASE_SEPOLIA.chain_id, async {
            humanize_chain_error(raw, "USDC")
        })
        .await;
        assert!(base.contains("ETH"), "{base}");
        let arc = with_pinned_chain(crate::chain::ARC_TESTNET.chain_id, async {
            humanize_chain_error(raw, "USDC")
        })
        .await;
        assert!(arc.contains("USDC") && !arc.contains("ETH"), "{arc}");
    }

    /// 불명 문구는 tx 해시와 「다시 보내기 전에 확인」을 싣는다 — 화면 송금은 이 문장만 보여 줄 수 있다.
    #[test]
    fn unknown_message_carries_hash_and_warning() {
        let m = unknown_send_message("0xabc", "제출 응답 시간 초과");
        assert!(m.contains("0xabc") && m.contains("다시 보내기 전에"), "{m}");
        let e = SendError::Unknown {
            hash: "0xabc".into(),
            msg: m.clone(),
        };
        assert_eq!(e.into_message(), m);
    }

    // USDC 금액 파싱: 6 decimals 로 정확히 변환돼야 한다 (음수 거부 헬퍼 경유).
    #[test]
    fn usdc_amount_parses_with_6_decimals() {
        let v: U256 = parse_usdc_nonneg("1.5", 6).unwrap();
        assert_eq!(v, U256::from(1_500_000u64));
    }

    // 날것 revert 에러를 사람이 읽을 수 있는 한국어로 매핑하고, hex data 노이즈를 떼어낸다.
    #[test]
    fn humanize_maps_common_chain_errors() {
        let raw = "server returned an error response: error code 3: execution reverted: \
                   ERC20: transfer amount exceeds balance, data: \"0x08c379a0...\"";
        assert_eq!(
            humanize_chain_error(raw, "USDC"),
            "USDC 잔액이 부족해요. 충전 후 다시 시도하세요."
        );
        assert!(
            humanize_chain_error("insufficient funds for gas * price + value", "ETH")
                .starts_with("ETH가 부족해요")
        );
        // 토큰 문맥: ETH 경로의 막연한 "exceeds balance" 는 ETH 부족으로(USDC 오안내 방지).
        assert_eq!(
            humanize_chain_error("transaction cost exceeds balance", "ETH"),
            "ETH 잔액이 부족해요. 충전 후 다시 시도하세요."
        );
        // 알 수 없는 revert: 사유만 남기고 hex data 는 버린다.
        let other = "execution reverted: Pausable: paused, data: \"0xdeadbeef\"";
        assert_eq!(
            humanize_chain_error(other, "USDC"),
            "전송이 거부됐어요: Pausable: paused"
        );
        // 완전 미지: 서버 프리픽스 제거 + 간결화.
        assert_eq!(
            humanize_chain_error("server returned an error response: nonce too low", "ETH"),
            "전송에 실패했어요: nonce too low"
        );
    }

    /// 🔴 개발 50 — **Arc 의 네이티브 잔액과 ERC-20 잔액이 "같은 돈"인지** 체인에 확인한다.
    ///
    /// Arc UI 결정(가스 줄 감춤·ETH 탭 제거·네이티브 송금 차단)이 통째로 이 사실 하나 위에 서 있다.
    /// 만약 둘이 **다른 자산**이라면 우리는 진짜 가스 잔액을 숨겨 사용자가 송금을 못 하게 만든 셈이다.
    /// 검증하는 식: `balanceOf(a)` == `floor(eth_getBalance(a) / 10^12)`
    /// (6dp 뷰는 18dp 를 자른다 — 실측 예: 253271474403192451 → 253271).
    /// 두 조회를 **같은 블록에 고정**해서 그 사이 입금이 들어와도 흔들리지 않게 한다.
    /// 잔액이 0인 주소는 이 식이 그냥 성립하므로, 최근 블록에서 **실제로 움직인 주소**를 골라 쓴다.
    #[tokio::test]
    #[ignore = "네트워크 필요 (Arc 테스트넷·메인넷 공개 RPC)"]
    async fn arc_native_and_erc20_are_the_same_money() {
        use crate::chain::{ARC_MAINNET, ARC_TESTNET};
        // 메인넷도 같은 질문이다(개발 62) — UI 결정이 체인 하나가 아니라 «Arc 라는 설계» 위에 서 있으니
        // 두 체인 모두에서 성립해야 한다.
        for chain in [ARC_TESTNET, ARC_MAINNET] {
            arc_same_money_on(chain).await;
        }
    }

    async fn arc_same_money_on(chain: crate::chain::ChainConfig) {
        use alloy::eips::BlockId;
        use alloy::providers::Provider;

        let provider = ProviderBuilder::new()
            .connect(chain.default_rpc)
            .await
            .expect("Arc RPC 연결");
        let n = provider.get_block_number().await.expect("블록 번호");
        let at = BlockId::number(n);
        let block = provider
            .get_block(at)
            .full()
            .await
            .expect("블록 조회")
            .expect("블록 존재");
        let addr = block
            .transactions
            .txns()
            .next()
            .map(|t| t.inner.signer())
            .expect("이 블록에 트랜잭션이 있어야 표본이 된다");

        let native = provider
            .get_balance(addr)
            .block_id(at)
            .await
            .expect("네이티브 잔액");
        let erc20 = IERC20::new(chain.usdc_address, &provider)
            .balanceOf(addr)
            .block(at)
            .call()
            .await
            .expect("ERC-20 잔액");

        // 18dp → 6dp 는 10^12 로 나눈 몫(내림).
        let scale = U256::from(10u64).pow(U256::from(12u64));
        println!(
            "Arc {} {addr} @ block {n}: native={native}  erc20={erc20}  native/1e12={}",
            chain.chain_id,
            native / scale
        );
        assert!(
            !native.is_zero(),
            "표본 주소의 잔액이 0이라 아무것도 못 본다"
        );
        assert_eq!(
            erc20,
            native / scale,
            "Arc 의 네이티브 잔액과 ERC-20 잔액이 같은 돈이 아니다 — 가스 줄을 감춘 판단이 틀렸다는 뜻"
        );
    }

    // 실제 Base Sepolia RPC로 잔액 조회 (네트워크 필요).
    #[tokio::test]
    #[ignore = "네트워크 필요 (Base Sepolia 공개 RPC)"]
    async fn live_balance_query() {
        let b = get_balances("0x8b7ba5077d261739f5FeBB31B10167671e590161".into())
            .await
            .expect("잔액 조회 성공");
        println!("ETH = {:?}  USDC = {}", b.eth, b.usdc);
        assert!(b.eth.as_deref().unwrap().parse::<f64>().is_ok());
        assert!(b.usdc.parse::<f64>().is_ok());
    }
}
