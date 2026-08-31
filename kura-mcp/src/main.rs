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
// 신원 조회 1개 — 읽기 전용, 온체인만 (개발 47):
//   - lookup_agent      : ERC-8004 레지스트리에서 에이전트 번호의 등록 지갑·기재 도메인을 읽는다.
//
// 핵심 보안: 비번은 절대 MCP/채팅에 노출되지 않는다. MCP는 결제를 "요청"만 하고, 서명·전송은
// GUI 앱이 사람 승인을 받아 수행한다(파일 기반 IPC). 한도·긴급잠금도 GUI가 강제한다.

use kura_mcp::flow::{self, X402Outcome, X402Result};
use kura_mcp::{erc8004, payment, wallet};

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
    /// Optional: the recipient's ERC-8004 agent number, if you know it from the service's own
    /// documentation or agent card. The wallet reads that agent's on-chain record and tells the
    /// user whether the address you are paying is the one registered for that agent. A mismatch
    /// is the useful part — a match is not proof of anything, since anyone can register.
    /// Leave it out if you don't know it; a wrong number just produces a "no such agent" note.
    #[serde(default)]
    agent_id: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct X402Args {
    /// URL of the paid resource to fetch (http/https).
    url: String,
    /// What the payment is for — the user reads this in the approval window, so fill it in.
    #[serde(default)]
    memo: Option<String>,
    /// Optional: the seller's ERC-8004 agent number, if you know it from the service's own
    /// documentation or agent card. The wallet then reads that agent's on-chain record and shows
    /// the user whether the payment address and the resource domain match what is registered.
    /// Leave it out if you don't know it — a wrong number just produces a "no such agent" note.
    #[serde(default)]
    agent_id: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AgentArgs {
    /// The agent's ERC-8004 number (agentId — the Identity Registry NFT token id).
    agent_id: u64,
    /// Optional: an address to compare against the agent's registered wallet.
    #[serde(default)]
    pay_to: Option<String>,
    /// Optional: a resource URL whose domain is compared with the domain listed on-chain.
    #[serde(default)]
    resource: Option<String>,
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
        description = "Reads the wallet's USDC (for payments) and gas-token balances on the active network (Base mainnet, Base Sepolia, or Arc testnet, per the user's setting). The `eth` field is the gas balance and is ABSENT on chains where gas is paid in USDC itself (Arc) — there the USDC balance already covers gas, so never add the two together. Errors if there is no wallet."
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
        (what the payment is for — the user reads it to decide, so always fill it in), and optionally \
        agent_id (the recipient's ERC-8004 number, if a service told you one — the wallet then shows the \
        user whether the address matches that agent's registered wallet). Per-payment and daily \
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
            args.agent_id,
            // MCP 는 도구 호출당 응답이 한 번뿐이라 "기다리는 중"을 중간에 알릴 상대가 없다.
            || {},
        )
        .await
        .map_err(|e| McpError::internal_error(e, None))?;
        let mut body = serde_json::json!({
            "status": out.status,   // approved | rejected | failed
            "tx_hash": out.tx_hash,
            "detail": out.detail,
            "explorer": out.explorer,
        });
        // 대조 결과는 **있을 때만** 붙인다 — 번호를 안 준 결제의 응답은 예전과 완전히 같다.
        if let Some(a) = out.agent {
            body["agent"] = serde_json::to_value(a).unwrap_or(serde_json::Value::Null);
        }
        if !out.agent_note.is_empty() {
            body["agent_note"] = serde_json::Value::String(out.agent_note);
        }
        json_result(&body)
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
        let out = flow::run_x402(args.url.trim(), args.memo.as_deref(), args.agent_id)
            .await
            .map_err(|e| McpError::internal_error(e, None))?;
        let X402Result {
            outcome,
            agent,
            agent_note,
        } = out;
        let mut body = match outcome {
            X402Outcome::NotPaid { http_status, body } => serde_json::json!({
                "paid": false,
                "status": "ok",
                "http_status": http_status,
                "body": body,
            }),
            X402Outcome::Declined { status, detail } => serde_json::json!({
                "paid": false,
                "status": status,   // rejected | failed
                "detail": detail,
            }),
            X402Outcome::Paid {
                http_status,
                ok,
                amount,
                pay_to,
                resource,
                settlement,
                body,
            } => serde_json::json!({
                "paid": true,
                "status": if ok { "ok" } else { "settlement_failed" },
                "http_status": http_status,
                "amount": amount,
                "asset": "USDC",
                "pay_to": pay_to,
                "resource": resource,
                "settlement": settlement,   // X-PAYMENT-RESPONSE (base64) — 정산 증빙
                "body": body,
            }),
        };
        // 대조 결과는 사람(승인 창)과 AI 가 같이 본다 — 여기선 사실만, 판정은 없다.
        if let Some(a) = agent {
            body["agent"] = serde_json::to_value(a).unwrap_or(serde_json::Value::Null);
            body["agent_caution"] = serde_json::json!(CAUTION);
        }
        if !agent_note.is_empty() {
            body["agent_lookup_note"] = serde_json::json!(agent_note);
        }
        json_result(&body)
    }

    #[tool(
        description = "Reads an agent's ERC-8004 record from the registry on the active Base network \
        (read-only, on-chain only — the wallet never fetches the agent's website). Give it agent_id, the \
        agent's number in the Identity Registry. Returns: registered, owner, wallet (the registered \
        agentWallet), token_uri and the uri_domain read from it, declared_name (what the record calls \
        itself), and feedback_clients (how many addresses left feedback). Pass pay_to and/or resource to \
        also get a comparison: whether the address equals the registered wallet and whether the resource's \
        domain equals the domain listed on-chain. IMPORTANT: registration is permissionless — anyone can \
        register any name, domain, or wallet, and anyone can leave feedback. Being registered is NOT proof \
        of safety. Only a mismatch is a strong signal, and only when the agent number came from a source \
        you trust (the service's own docs), not from the payment response itself."
    )]
    async fn lookup_agent(
        &self,
        Parameters(args): Parameters<AgentArgs>,
    ) -> Result<CallToolResult, McpError> {
        if !erc8004::lookup_enabled() {
            return Err(McpError::internal_error(
                "The user has turned agent identity lookup off in the wallet settings. Ask them to \
                 turn it back on (Settings → Network) if they want it.",
                None,
            ));
        }
        let rec = erc8004::lookup(args.agent_id)
            .await
            .map_err(|e| McpError::internal_error(e, None))?;
        let mut body = serde_json::to_value(&rec).unwrap_or(serde_json::Value::Null);
        let pay_to = args.pay_to.unwrap_or_default();
        let resource = args.resource.unwrap_or_default();
        if !pay_to.trim().is_empty() || !resource.trim().is_empty() {
            let trust = erc8004::trust_from(&rec, pay_to.trim(), resource.trim());
            body["comparison"] = serde_json::to_value(trust).unwrap_or(serde_json::Value::Null);
        }
        body["caution"] = serde_json::json!(CAUTION);
        json_result(&body)
    }
}

/// 조회 결과에 항상 함께 나가는 경고 — 등록은 무허가라 "등록됨"이 안전을 뜻하지 않는다.
/// 읽는 쪽이 모델이라, 이 한 줄이 없으면 «온체인에 있으니 믿을 만하다»로 넘어가기 쉽다.
const CAUTION: &str = "ERC-8004 registration is permissionless: anyone can register any name, \
domain, or wallet, and anyone can leave feedback. Registration is not proof of safety — treat \
these as claims. A mismatch is meaningful; a match only means the claim is self-consistent.";

#[tool_handler]
impl ServerHandler for WalletServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Kura — a local Ethereum wallet for AI agents (on Base mainnet, Base Sepolia, or Arc \
                 testnet, per the user's setting; check get_balances/get_wallet_status for the current \
                 balance. On mainnet these are real funds). Balance, address, and history are read-only. To pay, call \
                 request_payment: the wallet app opens an approval window — by default a human must approve \
                 with their password, and only when the user has turned autopay on is it approved \
                 automatically, within that limit. Never ask for or accept a password in chat or over \
                 MCP. lookup_agent reads an ERC-8004 agent record on-chain (read-only); registration \
                 there is permissionless, so it is a claim, not proof of safety."
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
