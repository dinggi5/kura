// Kura MCP 어댑터.
//
// AI 에이전트(Claude Code 등)가 stdio MCP로 로컬 지갑을 다룬다.
// 읽기 도구 3개 — 비번 불필요, 읽기 전용 (개발 8, Session 9):
//   - get_wallet_status : 지갑 상태 + 주소
//   - get_balances      : ETH(가스) + USDC(결제) 잔액
//   - get_history       : 최근 거래 내역
// 결제 도구 2개 — 사람 승인 필요:
//   - request_payment   : 송금을 "요청" (개발 9, Session 10). GUI 팝업 → 비번 승인 → 온체인 전송.
//   - x402_fetch        : x402 유료 리소스를 가져온다 (개발 11, Session 12). 402 → 서명 승인 → 재요청.
//
// 핵심 보안: 비번은 절대 MCP/채팅에 노출되지 않는다. MCP는 결제를 "요청"만 하고, 서명·전송은
// GUI 앱이 사람 승인을 받아 수행한다(파일 기반 IPC). 한도·긴급잠금도 GUI가 강제한다.

use kura_mcp::flow::{self, X402Outcome};
use kura_mcp::{payment, wallet};

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Clone)]
struct WalletServer {
    tool_router: ToolRouter<WalletServer>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct HistoryArgs {
    /// How many recent entries to return (default 20, max 200).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PayArgs {
    /// Recipient address (42 characters, starting with 0x).
    to: String,
    /// Amount as a decimal string, for example "1.5".
    amount: String,
    /// Token: "USDC" (default) or "ETH".
    #[serde(default)]
    token: Option<String>,
    /// What the payment is for — the user reads this in the approval window, so fill it in.
    #[serde(default)]
    memo: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct X402Args {
    /// URL of the paid resource to fetch (http/https).
    url: String,
    /// What the payment is for — the user reads this in the approval window, so fill it in.
    #[serde(default)]
    memo: Option<String>,
}

/// JSON 직렬화 결과를 MCP 텍스트 콘텐츠로 감싼다.
fn json_result<T: serde::Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(value).map_err(|e| {
        McpError::internal_error(format!("Couldn't serialize the result: {e}"), None)
    })?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

#[tool_router]
impl WalletServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Returns the wallet's state and address. state is encrypted (normal), legacy, or none. No password needed — read only."
    )]
    async fn get_wallet_status(&self) -> Result<CallToolResult, McpError> {
        let status = wallet::wallet_status().map_err(|e| McpError::internal_error(e, None))?;
        json_result(&status)
    }

    #[tool(
        description = "Reads the wallet's ETH (for gas) and USDC (for payments) balances on the active network (Base — testnet or mainnet, per the user's setting). Errors if there is no wallet."
    )]
    async fn get_balances(&self) -> Result<CallToolResult, McpError> {
        let status = wallet::wallet_status().map_err(|e| McpError::internal_error(e, None))?;
        let addr = status
            .address
            .ok_or_else(|| McpError::internal_error("There is no wallet yet", None))?;
        let balances = wallet::get_balances(&addr)
            .await
            .map_err(|e| McpError::internal_error(e, None))?;
        json_result(&balances)
    }

    #[tool(
        description = "Returns recent transaction attempts, newest first. status is one of sent, blocked, failed, \
        signed (x402 signed, awaiting settlement), settled (x402 settled, settle_tx is the settlement tx), \
        or settle_failed. Use limit to cap how many come back (default 20)."
    )]
    async fn get_history(
        &self,
        Parameters(args): Parameters<HistoryArgs>,
    ) -> Result<CallToolResult, McpError> {
        let mut list = wallet::read_history();
        let limit = args.limit.unwrap_or(20);
        list.truncate(limit);
        json_result(&list)
    }

    #[tool(
        description = "Asks the user to make a payment. The wallet app opens an approval window, and the \
        payment is only sent once the user approves it with their password (it waits up to 5 minutes). The one \
        exception is autopay, which the user turns on themselves — only then can a payment be approved \
        automatically, and only within an unlocked session, a small limit, and a trusted address. \
        Arguments: to (recipient address), amount (decimal string), token (USDC by default, or ETH), memo \
        (what the payment is for — the user reads it to decide, so always fill it in). Per-payment and daily \
        limits and the emergency lock are enforced by the app. Never send a password as an argument — the \
        user types it in the app. Returns: status (approved/rejected/failed), tx_hash, and an explorer link."
    )]
    async fn request_payment(
        &self,
        Parameters(args): Parameters<PayArgs>,
    ) -> Result<CallToolResult, McpError> {
        let token = args.token.as_deref().unwrap_or("USDC");
        let out = flow::run_payment(
            token,
            args.to.trim(),
            args.amount.trim(),
            args.memo.as_deref().unwrap_or("").trim(),
        )
        .await
        .map_err(|e| McpError::internal_error(e, None))?;
        json_result(&serde_json::json!({
            "status": out.status,   // approved | rejected | failed
            "tx_hash": out.tx_hash,
            "detail": out.detail,
            "explorer": out.explorer,
        }))
    }

    #[tool(
        description = "Fetches an x402 paid resource (a URL). It GETs the URL first; if the server answers \
        402 Payment Required, it asks the user to approve the required payment (exact scheme, the active Base \
        network, USDC) in the wallet app, builds an EIP-3009 signature, and re-requests the same URL with an \
        X-PAYMENT header to return the content. If no payment is required (no 402), it just returns the body. \
        Approval works exactly as in request_payment (password by default; automatic only when the user has \
        turned autopay on), and the app enforces per-payment and daily limits and the emergency lock. Never \
        send a password as an argument. Arguments: url (required), memo (what the payment is for — the user \
        reads it to decide). Returns: paid, status, http_status, body, and amount/pay_to/settlement when paid."
    )]
    async fn x402_fetch(
        &self,
        Parameters(args): Parameters<X402Args>,
    ) -> Result<CallToolResult, McpError> {
        let out = flow::run_x402(args.url.trim(), args.memo.as_deref())
            .await
            .map_err(|e| McpError::internal_error(e, None))?;
        match out {
            X402Outcome::NotPaid { http_status, body } => json_result(&serde_json::json!({
                "paid": false,
                "status": "ok",
                "http_status": http_status,
                "body": body,
            })),
            X402Outcome::Declined { status, detail } => json_result(&serde_json::json!({
                "paid": false,
                "status": status,   // rejected | failed
                "detail": detail,
            })),
            X402Outcome::Paid {
                http_status,
                ok,
                amount,
                pay_to,
                resource,
                settlement,
                body,
            } => json_result(&serde_json::json!({
                "paid": true,
                "status": if ok { "ok" } else { "settlement_failed" },
                "http_status": http_status,
                "amount": amount,
                "asset": "USDC",
                "pay_to": pay_to,
                "resource": resource,
                "settlement": settlement,   // X-PAYMENT-RESPONSE (base64) — 정산 증빙
                "body": body,
            })),
        }
    }
}

#[tool_handler]
impl ServerHandler for WalletServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Kura — a local Ethereum wallet for AI agents (on Base — testnet or mainnet, per the \
                 user's setting; check get_balances/get_wallet_status for the current balance. On mainnet \
                 these are real funds). Balance, address, and history are read-only. To pay, call \
                 request_payment: the wallet app opens an approval window — by default a human must approve \
                 with their password, and only when the user has turned autopay on is it approved \
                 automatically, within that limit. Never ask for or accept a password in chat or over MCP."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            // 기본값은 SDK 자신의 이름("rmcp 0.16.0")이라 MCP 앱의 서버 목록에 그렇게 뜬다.
            // 사용자가 보는 이름이고, 확장 목록에서 어느 버전이 도는지도 여기로 드러난다.
            server_info: Implementation {
                name: "kura".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = WalletServer::new().serve(stdio()).await?;

    // 연결한 AI 클라이언트 이름(예: "claude-code")을 알아내, GUI가 "연결됨" 배지를 띄울 수
    // 있게 주기적으로 생존 하트비트를 쓴다. 종료 시엔 지워서 GUI가 즉시 "연결 안 됨"으로 본다.
    let client = service
        .peer_info()
        .map(|i| i.client_info.name.clone())
        .unwrap_or_default();
    let beat = tokio::spawn(async move {
        loop {
            let _ = payment::write_mcp_heartbeat(&client);
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });

    service.waiting().await?;
    beat.abort();
    payment::clear_mcp_heartbeat();
    Ok(())
}
