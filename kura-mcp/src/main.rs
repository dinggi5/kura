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
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
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
    /// 최근 몇 건을 가져올지 (기본 20, 최대 200).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PayArgs {
    /// 받는 주소 (0x로 시작하는 42자).
    to: String,
    /// 금액 (십진수 문자열, 예: "1.5").
    amount: String,
    /// 토큰: "USDC"(기본) 또는 "ETH".
    #[serde(default)]
    token: Option<String>,
    /// 무엇에 대한 결제인지 — 사용자가 승인 팝업에서 보고 판단한다(있는 게 좋다).
    #[serde(default)]
    memo: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct X402Args {
    /// 가져올 유료 리소스의 URL (http/https).
    url: String,
    /// 무엇에 대한 결제인지 — 사용자가 승인 팝업에서 보고 판단한다. 채워주는 게 좋다.
    #[serde(default)]
    memo: Option<String>,
}

/// JSON 직렬화 결과를 MCP 텍스트 콘텐츠로 감싼다.
fn json_result<T: serde::Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| McpError::internal_error(format!("직렬화 실패: {e}"), None))?;
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
        description = "지갑 상태와 주소를 반환한다. state는 encrypted(정상)/legacy/none. 비번 불필요(읽기 전용)."
    )]
    async fn get_wallet_status(&self) -> Result<CallToolResult, McpError> {
        let status = wallet::wallet_status().map_err(|e| McpError::internal_error(e, None))?;
        json_result(&status)
    }

    #[tool(
        description = "지갑의 ETH(가스용)와 USDC(결제용) 잔액을 활성 네트워크(Base — 사용자 설정에 따라 테스트넷/메인넷)에서 조회한다. 지갑이 없으면 오류."
    )]
    async fn get_balances(&self) -> Result<CallToolResult, McpError> {
        let status = wallet::wallet_status().map_err(|e| McpError::internal_error(e, None))?;
        let addr = status
            .address
            .ok_or_else(|| McpError::internal_error("지갑이 아직 없습니다", None))?;
        let balances = wallet::get_balances(&addr)
            .await
            .map_err(|e| McpError::internal_error(e, None))?;
        json_result(&balances)
    }

    #[tool(
        description = "최근 거래 시도 내역을 최신순으로 반환한다. status는 sent(전송됨)/blocked(차단)/failed(실패)/\
        signed(x402 서명·정산 대기)/settled(x402 정산됨, settle_tx=정산 tx)/settle_failed(정산 실패). \
        limit으로 개수 제한 가능(기본 20)."
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
        description = "사용자에게 결제(송금)를 요청한다. 지갑 앱이 승인 팝업을 띄우고, 사용자가 \
        비밀번호로 승인해야만 실제로 전송된다(최대 5분 대기, 사람 승인 없이는 절대 전송되지 않음). \
        인자: to(받는 주소), amount(금액 십진수 문자열), token(USDC 기본/ETH), memo(결제 사유 — 사용자가 \
        보고 판단하니 꼭 채워라). 단일/일일 한도와 긴급잠금은 앱이 강제한다. 비밀번호는 절대 인자로 \
        보내지 마라 — 앱에서만 입력한다. 반환: status(approved/rejected/failed) + tx_hash + explorer 링크."
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
        description = "x402 유료 리소스(URL)를 가져온다. 먼저 GET 해보고 402 Payment Required 가 \
        오면, 서버가 요구한 결제(exact·활성 Base 네트워크·USDC)를 지갑 앱 팝업으로 사용자에게 승인받아 \
        EIP-3009 서명을 만들고, X-PAYMENT 헤더를 붙여 같은 URL을 재요청해 콘텐츠를 돌려준다. \
        결제가 필요 없으면(402가 아니면) 그대로 본문을 반환한다. 사람 승인(비번) 없이는 절대 결제되지 \
        않으며, 단일/일일 한도·긴급잠금은 앱이 강제한다. 비밀번호는 절대 인자로 보내지 마라. \
        인자: url(필수), memo(결제 사유 — 사용자가 보고 판단하니 채워라). \
        반환: paid 여부, status, http_status, 본문(body), 결제 시 amount/pay_to/settlement."
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
                "Kura — 로컬 AI 에이전트 이더리움 지갑(Base — 사용자 설정에 따라 테스트넷/메인넷; \
                 get_balances/get_wallet_status 로 현재 잔액을 확인하라. 메인넷이면 실제 자금이다). \
                 잔액/주소/거래내역을 읽기 전용으로 조회한다. 결제는 request_payment로 \
                 '요청'하면 지갑 앱이 사용자 승인 팝업을 띄운다 — 사람이 비밀번호로 승인해야만 \
                 전송된다(비밀번호는 절대 채팅/MCP에 입력하지 말 것)."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
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
