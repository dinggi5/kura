// ERC-8004(Trustless Agents) **읽기 전용** 조회 (개발 47).
//
// 무엇을 하나: AI 가 "이 서비스는 에이전트 #123 이다"라고 알려주면, 지갑이 **온체인 레지스트리를
// 직접 읽어** 그 번호의 등록 지갑·기재 도메인을 가져와 실제 받는 주소·결제 리소스와 **대조**한다.
// 그 결과(사실)를 결제 요청에 실어 보내면 승인 창이 한 줄로 보여준다.
//
// 왜 이 방향인가 (착수 시 실측으로 뒤집힌 설계 — 개발 47):
//   배포된 IdentityRegistry v2 에는 **주소 → 에이전트 역조회가 없다**(getAgentWallet 은 정방향뿐,
//   agentWallet 을 담은 MetadataSet 이벤트도 indexed 토픽이 아니다). 우회로인 이벤트 스캔도
//   공개 RPC 가 eth_getLogs 를 1만 블록으로 끊어 승인 경로에선 불가능하다. 그래서 "받는 주소만
//   보고 자동 판별"은 원리상 불가 → **주장은 AI 가, 검증은 지갑이** 하는 구조로 뒤집었다.
//
// 지키는 선 (사장 확정):
//   - **온체인 읽기만.** 웹 fetch 는 하지 않는다 — tokenURI 가 http(s) 면 그 **호스트만** 읽고
//     문서를 가져오지 않는다. data: URI 면 등록 JSON 이 온체인에 통째로 들어 있으므로 그대로 파싱한다.
//   - **판정하지 않는다.** "검증됨"·"안전" 같은 말을 만들지 않는다. 등록은 무허가라 누구나
//     아무 도메인이나 적을 수 있고(피드백도 시빌 가능), 이 조회가 주는 건 **일치/다름/모름**뿐이다.
//   - 이름(name)은 여기서 뽑아 **MCP 결과로만** 준다. 승인 창에는 절대 넣지 않는다 —
//     자기신고 이름을 사람 눈앞에 크게 띄우는 순간 그게 사칭의 통로가 된다.

use alloy::primitives::{Address, U256};
use alloy::providers::ProviderBuilder;
use alloy::sol;
use base64::{
    engine::general_purpose::{STANDARD as B64, STANDARD_NO_PAD as B64_NP},
    Engine,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::chain::active_chain;
use crate::wallet::{effective_rpc, jigap_dir, redact_urls};
use crate::{tf, ts};

/// tokenURI / 디코드된 등록 JSON 의 처리 상한(바이트). 체인에서 온 값은 외부 입력이라
/// 파싱 전에 크기부터 끊는다. 정상 등록 문서는 수 KB 다(실측: mainnet #1 = 2,269바이트).
const MAX_URI_BYTES: usize = 64 * 1024;

/// 표시용 이름 상한(문자). MCP 결과로만 나가지만 길이는 여기서 끊는다.
const MAX_NAME_CHARS: usize = 120;

/// 대조에 쓸 서비스 호스트 개수 상한(외부 입력이라 목록 길이를 묶어 둔다).
const MAX_SERVICE_DOMAINS: usize = 16;

/// 조회 전체에 거는 상한. **결제 흐름 앞에 끼는 선택 기능**이라 느린 RPC 가 결제를 무한정
/// 붙잡으면 안 된다 — alloy 기본 HTTP 클라이언트엔 요청 타임아웃이 없어서, 연결은 되고
/// eth_call 이 멎는 RPC 를 만나면 그대로 매달린다(코덱스 개발47 1차 P1).
/// 넘기면 조회만 포기하고 결제는 그대로 진행된다(줄이 안 붙을 뿐).
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

sol! {
    #[sol(rpc)]
    interface IIdentityRegistry {
        function ownerOf(uint256 tokenId) external view returns (address);
        function tokenURI(uint256 tokenId) external view returns (string);
        function getAgentWallet(uint256 agentId) external view returns (address);
    }

    #[sol(rpc)]
    interface IReputationRegistry {
        function getClients(uint256 agentId) external view returns (address[]);
    }
}

/// 레지스트리에서 읽어온 **사실 그대로**. 판정·점수는 없다.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct AgentRecord {
    pub agent_id: u64,
    pub chain_id: u64,
    /// 이 번호가 온체인에 존재하는가(ownerOf 성공).
    pub registered: bool,
    /// NFT 소유자 주소. 미등록이면 빈 문자열.
    pub owner: String,
    /// 등록 지갑(agentWallet). 미설정(0x0)·미등록이면 빈 문자열.
    pub wallet: String,
    /// tokenURI 원문. 웹으로 가져오지 않는다 — 도메인만 읽는다.
    pub token_uri: String,
    /// 표시용 대표 도메인 = `service_domains` 의 첫 값(주장값). 없으면 빈 문자열.
    pub uri_domain: String,
    /// 등록 문서가 밝힌 **서비스 호스트 전부**(주장값). 대조는 이 목록 전체와 한다 —
    /// 등록엔 web·mcp·a2a 가 각각 다른 호스트로 올라오고(web=example.com,
    /// x402=api.example.com), 대표 하나만 보면 **정상 결제가 「다름」으로 찍힌다**
    /// (코덱스 개발47 3차 P2). 헛경고는 이제 자율 결제까지 막으므로 값이 비싸다.
    /// 트레이드오프: 규격 참조용 링크(github.com 등)까지 들어와 일치 판정이 묽어질 수
    /// 있다 — 다만 그건 "그 호스트로 실제 결제가 갈 때"만 성립해 실익이 없다.
    pub service_domains: Vec<String>,
    /// 등록 문서가 스스로 밝힌 이름(data: URI 일 때만). **승인 창엔 쓰지 않는다.**
    pub declared_name: String,
    /// 피드백을 남긴 클라이언트 주소 수. 누구나 남길 수 있다(시빌 가능).
    /// `None` = 못 읽었다(조회 실패) — **0 과 구별한다**. 0 은 "아무도 안 남겼다"는 사실이고,
    /// None 은 "모른다"다. 못 읽은 걸 0 으로 적으면 없는 사실을 지어내는 셈이 된다.
    pub feedback_clients: Option<u32>,
}

/// 승인 창에 실어 보내는 **대조 결과**. 각 항목은 판정이 아니라 비교의 결과다.
/// `wallet_check`  = match | differs | unset | unknown
/// `domain_check`  = match | differs | unknown
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct AgentTrust {
    pub agent_id: u64,
    pub chain_id: u64,
    pub registered: bool,
    pub wallet: String,
    pub wallet_check: String,
    pub uri_domain: String,
    /// **실제로 결제 헤더를 보내는 URL** 의 호스트. 서버가 402 본문에 적어 보낸 주장값이
    /// 아니다 — 그걸 쓰면 공격자의 주장을 레지스트리와 맞춰 보는 꼴이 된다(2차 P1).
    pub resource_domain: String,
    pub domain_check: String,
    /// `None` = 못 읽음(0 과 구별 — AgentRecord 쪽 주석 참고).
    pub feedback_clients: Option<u32>,
}

/// settings.json 에서 ERC-8004 조회 스위치만 읽는 가벼운 뷰(다른 필드 무시).
/// GUI 와 같은 파일을 공유한다 — wallet.rs 의 RpcSettings 와 같은 패턴.
#[derive(Deserialize)]
struct LookupSel {
    #[serde(default = "yes")]
    agent_lookup: bool,
}

fn yes() -> bool {
    true
}

/// 사용자가 ERC-8004 조회를 켜 뒀는가. **기본 켜짐** — 새 바깥 연결을 여는 게 아니라
/// 이미 잔액을 읽고 있는 그 RPC 로 읽기 한 번을 더 하는 것이라, 끄는 쪽이 명시적 선택이다.
/// 파일이 없거나 못 읽으면 켜짐으로 본다(기능이 조용히 사라지는 것보다 낫다).
pub fn lookup_enabled() -> bool {
    let Ok(dir) = jigap_dir() else {
        return true;
    };
    std::fs::read_to_string(dir.join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<LookupSel>(&s).ok())
        .map(|s| s.agent_lookup)
        .unwrap_or(true)
}

/// 컨트랙트 호출이 **revert** 로 끝났는가(= 그런 토큰이 없다) — 네트워크 실패와 갈라야 한다.
/// 네트워크 실패를 "없음"으로 표시하면 멀쩡한 에이전트에 헛경고가 뜬다(치명적 오탐).
fn is_revert(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("revert") || e.contains("execution error")
}

/// URL 에서 호스트만 뽑는다(소문자, 포트·userinfo 제거, 선행 `www.` 제거).
/// 스킴이 없으면 None — "도메인처럼 생긴 문자열"을 도메인으로 받아주지 않는다.
pub fn host_of(url: &str) -> Option<String> {
    let s = url.trim();
    let (_, rest) = s.split_once("://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    // 공백이 낀 authority = 깨진 URL. trim 으로 살려내면 "https:// evil.com" 이 evil.com 으로
    // 통과해, 사람이 읽은 문자열과 우리가 대조한 도메인이 갈린다 → 그냥 "모름"으로 둔다.
    if authority.chars().any(char::is_whitespace) {
        return None;
    }
    // userinfo@host 형태에서 호스트만 (`@` 뒤). `user@pass@host` 는 마지막 `@` 기준.
    let hostport = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    // IPv6 리터럴은 대괄호까지 통째로, 그 외는 첫 `:` 앞까지(포트 제거).
    let host = match hostport.find(']') {
        Some(close) => &hostport[..=close],
        None => hostport.split(':').next().unwrap_or(hostport),
    };
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '[' | ']' | ':'))
    {
        return None;
    }
    let host = match host.strip_prefix("www.") {
        Some(h) => h.to_string(),
        None => host,
    };
    (!host.is_empty()).then_some(host)
}

/// 퍼센트 인코딩(`%7B` → `{`)을 푼다. 바이트로 돌려주고 UTF-8 판정은 호출자가 한다.
///
/// **`+` 를 공백으로 바꾸지 않는다** — 그건 form 인코딩(`application/x-www-form-urlencoded`)
/// 규칙이고, `data:` URI 본문은 RFC 2397 의 URI 인코딩이라 `+` 는 글자 그대로다. 바꾸면
/// base64 를 본문에 담은 JSON 문자열이 조용히 망가진다.
///
/// 깨진 시퀀스(`%`, `%A`, `%zz`)는 **원문 그대로 남긴다**. 조용히 다른 글자로 바꾸느니 남겨서
/// 뒤의 JSON 파싱이 실패하게 하는 쪽이 낫다 — 여기서 만들어 낸 글자가 도메인 대조에 쓰이면
/// 「등록 문서가 이렇게 말했다」가 거짓이 된다.
fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hi = (b[i + 1] as char).to_digit(16);
            let lo = (b[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// `data:` URI 의 본문을 문자열로 꺼낸다(base64 면 디코드). 웹 접속 없음 — 값이 URI 안에 다 있다.
fn decode_data_uri(uri: &str) -> Option<String> {
    let (meta, payload) = uri.split_once(',')?;
    if !meta.to_ascii_lowercase().contains(";base64") {
        // base64 가 아니면 본문은 **퍼센트 인코딩**이다(RFC 2397). 예전엔 원문을 그대로 돌려줘서
        // `data:application/json,%7B...%7D` 같은 등록 문서가 JSON 파싱에 실패했고, 도메인이
        // 「모름」으로 떨어져 승인 창에 대조 줄이 안 붙었다(개발 47 이월).
        return String::from_utf8(percent_decode(payload)).ok();
    }
    let cleaned: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = B64
        .decode(&cleaned)
        .ok()
        .or_else(|| B64_NP.decode(&cleaned).ok())?;
    if bytes.len() > MAX_URI_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// 등록 JSON(EIP-8004 registration)의 `services[]` 에서 http(s) 호스트를 **전부** 뽑는다.
/// name="web" 을 앞에 둔다(표시용 대표가 되도록). 중복 제거, 개수 상한.
fn service_domains_from_registration(json: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(services) = v.get("services").and_then(|s| s.as_array()) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    let mut push = |h: String| {
        if out.len() < MAX_SERVICE_DOMAINS && !out.contains(&h) {
            out.push(h);
        }
    };
    for web_only in [true, false] {
        for s in services {
            let name = s.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if web_only != name.eq_ignore_ascii_case("web") {
                continue;
            }
            let ep = s.get("endpoint").and_then(|e| e.as_str()).unwrap_or("");
            let low = ep.to_ascii_lowercase();
            if low.starts_with("http://") || low.starts_with("https://") {
                if let Some(h) = host_of(ep) {
                    push(h);
                }
            }
        }
    }
    out
}

/// tokenURI 에서 **기재 서비스 도메인들**을 읽는다.
///
/// - `data:` — 등록 JSON 이 온체인에 통째로 있다 → 파싱해서 서비스 호스트를 전부 쓴다.
/// - `http(s)` — **빈 목록**을 돌려준다. 그 호스트는 "등록 문서가 어디 저장돼 있나"일
///   뿐이지 에이전트의 서비스 도메인이 아니다(실측: 여러 에이전트가
///   `marketplace.olas.network/...` 에 문서를 올려 둔다). 문서를 웹으로 가져오지 않기로
///   한 이상 서비스 도메인은 알 수 없다 → **모름으로 남긴다**. 저장소 호스트를 도메인이라
///   우기면 정상 결제가 「다름」으로 찍힌다(코덱스 개발47 3차 P2).
/// - 그 외(ipfs: 등) — 빈 목록.
pub fn domains_from_token_uri(uri: &str) -> Vec<String> {
    let s = uri.trim();
    if s.is_empty() || s.len() > MAX_URI_BYTES {
        return Vec::new();
    }
    if !s.to_ascii_lowercase().starts_with("data:") {
        return Vec::new();
    }
    decode_data_uri(s)
        .map(|json| service_domains_from_registration(&json))
        .unwrap_or_default()
}

/// 등록 문서가 밝힌 이름(data: URI 일 때만). 제어문자를 걷어내고 길이를 끊는다.
/// **승인 창에는 쓰지 않는다** — MCP 결과에서 AI 가 참고할 용도.
pub fn name_from_token_uri(uri: &str) -> Option<String> {
    let s = uri.trim();
    if s.is_empty() || s.len() > MAX_URI_BYTES || !s.to_ascii_lowercase().starts_with("data:") {
        return None;
    }
    let json = decode_data_uri(s)?;
    let v: serde_json::Value = serde_json::from_str(&json).ok()?;
    let name = v.get("name")?.as_str()?;
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    let cleaned = cleaned.trim().to_string();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// 두 주소 문자열이 같은가(체크섬 대소문자 무시). 빈 값은 같지 않다고 본다.
fn same_addr(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    !a.is_empty() && !b.is_empty() && a.eq_ignore_ascii_case(b)
}

/// 읽어온 기록을 실제 결제와 **대조**한다 — 순수 함수(테스트 대상).
/// `resource` 에는 **실제 요청 URL**(402 를 낸 최종 URL)을 넘긴다. 서버가 주장한 리소스
/// 문자열을 넘기면 안 된다 — 대조가 공격자의 자기신고를 검사하는 일이 되어 버린다.
pub fn trust_from(rec: &AgentRecord, pay_to: &str, resource: &str) -> AgentTrust {
    let wallet_check = if !rec.registered {
        "unknown"
    } else if rec.wallet.is_empty() {
        "unset"
    } else if same_addr(&rec.wallet, pay_to) {
        "match"
    } else {
        "differs"
    };

    let resource_domain = host_of(resource).unwrap_or_default();
    let domain_check =
        if !rec.registered || rec.service_domains.is_empty() || resource_domain.is_empty() {
            "unknown"
        } else if rec.service_domains.contains(&resource_domain) {
            "match"
        } else {
            "differs"
        };

    AgentTrust {
        agent_id: rec.agent_id,
        chain_id: rec.chain_id,
        registered: rec.registered,
        wallet: rec.wallet.clone(),
        wallet_check: wallet_check.to_string(),
        uri_domain: rec.uri_domain.clone(),
        resource_domain,
        domain_check: domain_check.to_string(),
        feedback_clients: rec.feedback_clients,
    }
}

/// 활성 체인의 레지스트리에서 에이전트 기록을 읽는다(읽기 전용 RPC 호출 4번).
/// 레지스트리가 없는 체인이면 Err — 호출자가 "이 네트워크엔 없음"으로 안내한다.
pub async fn lookup(agent_id: u64) -> Result<AgentRecord, String> {
    tokio::time::timeout(LOOKUP_TIMEOUT, lookup_inner(agent_id))
        .await
        .map_err(|_| {
            ts!(
                "에이전트 조회가 시간 안에 끝나지 않았어요(10초).",
                "The agent lookup didn't finish in time (10s)."
            )
            .to_string()
        })?
}

async fn lookup_inner(agent_id: u64) -> Result<AgentRecord, String> {
    let chain = active_chain();
    let identity = chain.erc8004_identity.ok_or_else(|| {
        ts!(
            "이 네트워크에는 ERC-8004 레지스트리가 없어요.",
            "There is no ERC-8004 registry on this network."
        )
        .to_string()
    })?;

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

    let id = U256::from(agent_id);
    let registry = IIdentityRegistry::new(identity, &provider);

    // ownerOf 가 revert = 그런 번호가 없다(미등록). 그 외 실패는 네트워크 문제 → 에러로 올린다.
    let owner = match registry.ownerOf(id).call().await {
        Ok(a) => a,
        Err(e) => {
            let msg = e.to_string();
            if is_revert(&msg) {
                return Ok(AgentRecord {
                    agent_id,
                    chain_id: chain.chain_id,
                    registered: false,
                    owner: String::new(),
                    wallet: String::new(),
                    token_uri: String::new(),
                    uri_domain: String::new(),
                    service_domains: Vec::new(),
                    declared_name: String::new(),
                    feedback_clients: None,
                });
            }
            return Err(tf!(
                "에이전트 조회 실패: {}",
                "Couldn't read the agent record: {}",
                redact_urls(&msg)
            ));
        }
    };

    // 나머지 셋은 동시에. 각각의 revert 는 "그 값이 없다"로 접는다(전체를 실패시키지 않는다).
    let reputation = chain.erc8004_reputation;
    let (uri, wallet, clients) = tokio::join!(
        async { registry.tokenURI(id).call().await },
        async { registry.getAgentWallet(id).call().await },
        async {
            match reputation {
                Some(rep) => IReputationRegistry::new(rep, &provider)
                    .getClients(id)
                    .call()
                    .await
                    .map(|v| v.len()),
                None => Ok(0),
            }
        }
    );

    // ⚠️ 일시적 RPC 실패를 **빈 값으로 접지 않는다**(코덱스 개발47 1차 P1). 접으면
    // "등록 지갑 없음"·"도메인 모름"이라는 **사실처럼 보이는 거짓**이 승인 창에 뜨고,
    // 하필 그게 주소 불일치 경고를 덮어버린다. 등록된 에이전트라면 이 둘은 revert 하지
    // 않으므로(미설정은 0x0 을 돌려준다 — Sepolia #1 실측), 여기서의 에러 = 통신 실패다.
    let token_uri = uri.map_err(|e| {
        tf!(
            "에이전트 URI 조회 실패: {}",
            "Couldn't read the agent URI: {}",
            redact_urls(&e.to_string())
        )
    })?;
    let wallet_addr = wallet.map_err(|e| {
        tf!(
            "등록 지갑 조회 실패: {}",
            "Couldn't read the registered wallet: {}",
            redact_urls(&e.to_string())
        )
    })?;
    let wallet = if wallet_addr == Address::ZERO {
        String::new() // 미설정 — 이건 진짜 "없음"이다(0x0 을 읽어냈다)
    } else {
        wallet_addr.to_string()
    };
    // 피드백은 못 읽어도 신원·대조는 살아 있으므로 조회를 실패시키지 않는다 — 대신 0 이 아니라
    // "모름"으로 남긴다.
    let feedback_clients = clients.ok().map(|n| n.min(u32::MAX as usize) as u32);

    let service_domains = domains_from_token_uri(&token_uri);
    Ok(AgentRecord {
        agent_id,
        chain_id: chain.chain_id,
        registered: true,
        owner: owner.to_string(),
        uri_domain: service_domains.first().cloned().unwrap_or_default(),
        service_domains,
        declared_name: name_from_token_uri(&token_uri).unwrap_or_default(),
        token_uri,
        wallet,
        feedback_clients,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_of_reads_the_host_only() {
        assert_eq!(host_of("https://api.example.com/x/y?z=1").as_deref(), Some("api.example.com"));
        assert_eq!(host_of("http://Example.COM").as_deref(), Some("example.com"));
        assert_eq!(host_of("https://www.example.com/").as_deref(), Some("example.com"));
        assert_eq!(host_of("https://example.com:8443/p").as_deref(), Some("example.com"));
        assert_eq!(host_of("https://user:pw@example.com/p").as_deref(), Some("example.com"));
        assert_eq!(host_of("https://example.com./").as_deref(), Some("example.com")); // 루트 점 제거
        assert_eq!(host_of("https://[::1]:8545/").as_deref(), Some("[::1]"));
    }

    /// 스킴 없는 값·빈 값·공백 섞인 값은 도메인으로 받아주지 않는다.
    #[test]
    fn host_of_rejects_non_urls() {
        assert_eq!(host_of("example.com"), None);
        assert_eq!(host_of(""), None);
        assert_eq!(host_of("https://"), None);
        assert_eq!(host_of("https:// spaced.com"), None);
    }

    /// http(s) tokenURI 의 호스트는 **문서가 놓인 곳**이지 에이전트의 서비스 도메인이 아니다.
    /// 실측: 여러 에이전트가 marketplace.olas.network 에 문서를 올려 둔다 — 그걸 도메인이라
    /// 우기면 그 마켓플레이스에서 산 정상 결제가 전부 「다름」이 된다(3차 P2).
    #[test]
    fn https_token_uri_yields_no_claimed_domain() {
        assert!(
            domains_from_token_uri("https://marketplace.olas.network/erc8004/base/ai-agents/5")
                .is_empty()
        );
        // ipfs 등 해석 못 하는 스킴·빈 값도 마찬가지로 "모름".
        assert!(domains_from_token_uri("ipfs://bafy…").is_empty());
        assert!(domains_from_token_uri("").is_empty());
    }

    /// data: URI = 등록 JSON 이 온체인에 통째로 있다 → 웹 접속 없이 서비스 호스트를 읽는다.
    /// (실측 형태: Base 메인넷 #1 ClawNews)
    #[test]
    fn domain_from_data_uri_registration() {
        let json = r#"{"name":"ClawNews","services":[
            {"name":"OASF","endpoint":"https://github.com/agntcy/oasf/"},
            {"name":"web","endpoint":"https://clawnews.io"}]}"#;
        let uri = format!("data:application/json;base64,{}", B64.encode(json));
        // web 이 대표(첫 값)로 오되, 나머지 http(s) 호스트도 대조 대상으로 남는다.
        assert_eq!(domains_from_token_uri(&uri), vec!["clawnews.io", "github.com"]);
        assert_eq!(name_from_token_uri(&uri).as_deref(), Some("ClawNews"));
    }

    /// web 항목이 없으면 http(s) 엔드포인트들이 그대로 목록이 된다(ipfs 등은 건너뜀).
    #[test]
    fn domains_skip_non_http_endpoints() {
        let json = r#"{"services":[{"name":"a2a","endpoint":"ipfs://x"},
            {"name":"mcp","endpoint":"https://mcp.example.com/sse"}]}"#;
        let uri = format!("data:application/json;base64,{}", B64.encode(json));
        assert_eq!(domains_from_token_uri(&uri), vec!["mcp.example.com"]);
    }

    /// 실제 결제가 가는 곳은 web 이 아니라 x402/mcp 엔드포인트인 경우가 흔하다 —
    /// 대표 하나만 보면 정상 결제가 「다름」으로 찍히므로 **목록 전체**와 대조한다(3차 P2).
    #[test]
    fn trust_matches_any_registered_endpoint() {
        let json = r#"{"services":[{"name":"web","endpoint":"https://example.com"},
            {"name":"x402","endpoint":"https://api.example.com/pay"}]}"#;
        let uri = format!("data:application/json;base64,{}", B64.encode(json));
        let domains = domains_from_token_uri(&uri);
        assert_eq!(domains, vec!["example.com", "api.example.com"]);

        let mut r = rec("0x1", "example.com");
        r.service_domains = domains;
        // web 이 아닌 엔드포인트로 결제해도 일치.
        assert_eq!(
            trust_from(&r, "0x1", "https://api.example.com/pay/9").domain_check,
            "match"
        );
        // 목록에 없는 곳이면 다름.
        assert_eq!(
            trust_from(&r, "0x1", "https://evil.example/pay").domain_check,
            "differs"
        );
    }

    /// 깨진 base64·JSON 아님·services 없음 → 조용히 "모름"(패닉·오탐 금지).
    #[test]
    fn broken_data_uri_is_unknown() {
        assert!(domains_from_token_uri("data:application/json;base64,!!!!").is_empty());
        assert!(domains_from_token_uri("data:text/plain,hello").is_empty());
        let uri = format!("data:application/json;base64,{}", B64.encode("{\"name\":\"x\"}"));
        assert!(domains_from_token_uri(&uri).is_empty());
        assert_eq!(name_from_token_uri(&uri).as_deref(), Some("x"));
    }

    /// base64 가 아닌 `data:` URI 는 **퍼센트 인코딩**이다 (개발 47 이월).
    /// 안 풀면 JSON 파싱이 실패해 도메인이 통째로 「모름」이 된다.
    #[test]
    fn percent_encoded_data_uri_is_decoded() {
        let json = r#"{"name":"a b","services":[{"name":"web","endpoint":"https://api.example.com/x"}]}"#;
        // RFC 3986 예약문자를 인코딩한 형태(실제 온체인 표본이 이렇게 온다).
        let encoded: String = json
            .chars()
            .map(|c| match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' | ':' => {
                    c.to_string()
                }
                _ => format!("%{:02X}", c as u32),
            })
            .collect();
        let uri = format!("data:application/json,{encoded}");
        assert_eq!(domains_from_token_uri(&uri), vec!["api.example.com"]);
        assert_eq!(name_from_token_uri(&uri).as_deref(), Some("a b"));
    }

    /// 대조군 — 인코딩 규칙을 잘못 적용하면 값이 조용히 달라진다.
    /// `+` 는 공백이 **아니고**(form 인코딩 규칙을 쓰면 안 된다), 깨진 `%` 는 원문으로 남는다.
    #[test]
    fn percent_decode_does_not_invent_characters() {
        let uri = r#"data:application/json,{"name":"a+b"}"#;
        assert_eq!(name_from_token_uri(uri).as_deref(), Some("a+b"));
        // 깨진 시퀀스는 그대로 → JSON 은 여전히 유효하고 글자가 바뀌지 않는다.
        let uri = r#"data:application/json,{"name":"100%"}"#;
        assert_eq!(name_from_token_uri(uri).as_deref(), Some("100%"));
        let uri = r##"data:application/json,{"name":"%zz"}"##;
        assert_eq!(name_from_token_uri(uri).as_deref(), Some("%zz"));
    }

    /// 이름은 제어문자를 걷어내고 길이를 끊는다(외부 입력).
    #[test]
    fn declared_name_is_sanitized() {
        let json = serde_json::json!({ "name": format!("A\nB{}", "x".repeat(300)) }).to_string();
        let uri = format!("data:application/json;base64,{}", B64.encode(&json));
        let name = name_from_token_uri(&uri).expect("이름");
        assert!(!name.contains('\n'));
        assert_eq!(name.chars().count(), MAX_NAME_CHARS);
    }

    fn rec(wallet: &str, domain: &str) -> AgentRecord {
        AgentRecord {
            agent_id: 123,
            chain_id: 8453,
            registered: true,
            owner: "0xowner".into(),
            wallet: wallet.into(),
            token_uri: String::new(),
            uri_domain: domain.into(),
            service_domains: if domain.is_empty() {
                Vec::new()
            } else {
                vec![domain.to_string()]
            },
            declared_name: String::new(),
            feedback_clients: Some(7),
        }
    }

    /// 핵심 대조: 등록 지갑 == 받는 주소, 기재 도메인 == 리소스 도메인.
    #[test]
    fn trust_marks_match() {
        let t = trust_from(
            &rec("0xAbC0000000000000000000000000000000000001", "api.example.com"),
            "0xabc0000000000000000000000000000000000001", // 체크섬 대소문자만 다름
            "https://api.example.com/paid/thing",
        );
        assert_eq!(t.wallet_check, "match");
        assert_eq!(t.domain_check, "match");
        assert_eq!(t.resource_domain, "api.example.com");
        assert_eq!(t.feedback_clients, Some(7));
    }

    /// 주소 바꿔치기 = 이 기능이 실제로 잡아야 하는 것.
    #[test]
    fn trust_marks_wallet_swap() {
        let t = trust_from(
            &rec("0x1111111111111111111111111111111111111111", "api.example.com"),
            "0x2222222222222222222222222222222222222222",
            "https://api.example.com/x",
        );
        assert_eq!(t.wallet_check, "differs");
        assert_eq!(t.domain_check, "match");
    }

    /// 기재 도메인이 결제 리소스와 다른 경우.
    #[test]
    fn trust_marks_domain_mismatch() {
        let t = trust_from(
            &rec("0x1111111111111111111111111111111111111111", "other.example"),
            "0x1111111111111111111111111111111111111111",
            "https://api.example.com/x",
        );
        assert_eq!(t.wallet_check, "match");
        assert_eq!(t.domain_check, "differs");
    }

    /// 값이 없으면 "다름"이 아니라 "모름"이다 — 없는 걸 경고로 만들지 않는다.
    #[test]
    fn missing_values_are_unknown_not_differs() {
        let t = trust_from(&rec("", ""), "0x1", "https://api.example.com/x");
        assert_eq!(t.wallet_check, "unset");
        assert_eq!(t.domain_check, "unknown");

        // 리소스 URL 을 모를 때(직접 송금 등)도 도메인은 "모름".
        let t2 = trust_from(&rec("0x1", "api.example.com"), "0x1", "");
        assert_eq!(t2.domain_check, "unknown");
    }

    /// 미등록 번호: 지갑·도메인 비교 자체가 성립하지 않는다.
    #[test]
    fn unregistered_agent_has_no_comparisons() {
        let mut r = rec("", "");
        r.registered = false;
        let t = trust_from(&r, "0x1", "https://x.example/y");
        assert!(!t.registered);
        assert_eq!(t.wallet_check, "unknown");
        assert_eq!(t.domain_check, "unknown");
    }

    /// 실물 조회 — 네트워크가 필요해 기본 제외. 체인은 KURA_CHAIN_ID 로 고정한다
    /// (테스트는 사용자의 settings.json 을 안 따르지만, 환경변수는 그보다 우선한다).
    ///   KURA_CHAIN_ID=8453 cargo test -p kura-mcp -- --ignored --nocapture
    ///
    /// 대상: Base 메인넷 #1 = "ClawNews" (개발 47 착수 실측). 등록 지갑이 owner 와 같고,
    /// tokenURI 가 data: URI 라 **웹 접속 없이** 등록 JSON 에서 도메인이 나온다.
    #[tokio::test]
    #[ignore = "네트워크 필요 — Base 메인넷 레지스트리 실조회"]
    async fn live_lookup_base_mainnet_agent_one() {
        let rec = lookup(1).await.expect("조회");
        assert!(rec.registered);
        assert_eq!(rec.chain_id, 8453, "KURA_CHAIN_ID=8453 로 실행할 것");
        assert!(rec.token_uri.starts_with("data:"), "{}", rec.token_uri);
        assert_eq!(rec.uri_domain, "clawnews.io");
        assert!(rec.service_domains.contains(&"clawnews.io".to_string()));
        assert_eq!(rec.declared_name, "ClawNews");
        assert!(!rec.wallet.is_empty());
        assert!(rec.feedback_clients.unwrap_or(0) > 0);

        // 같은 주소로 결제하면 "일치", 딴 주소면 "다름" — 이 기능이 실제로 하는 일.
        let ok = trust_from(&rec, &rec.wallet, "https://clawnews.io/paid");
        assert_eq!(ok.wallet_check, "match");
        assert_eq!(ok.domain_check, "match");
        let swapped = trust_from(
            &rec,
            "0x000000000000000000000000000000000000dEaD",
            "https://evil.example/paid",
        );
        assert_eq!(swapped.wallet_check, "differs");
        assert_eq!(swapped.domain_check, "differs");
    }

    /// 없는 번호는 revert → registered=false 로 접힌다(에러가 아니다).
    #[tokio::test]
    #[ignore = "네트워크 필요 — Base 메인넷 레지스트리 실조회"]
    async fn live_lookup_missing_agent_is_not_an_error() {
        let rec = lookup(999_999_999).await.expect("조회는 성공해야 한다");
        assert!(!rec.registered);
        assert!(rec.wallet.is_empty());
    }

    /// revert(없는 토큰)와 네트워크 실패를 갈라야 한다 — 네트워크 실패를 "없음"으로
    /// 표시하면 멀쩡한 에이전트에 헛경고가 뜬다.
    #[test]
    fn revert_is_distinguished_from_network_failure() {
        assert!(is_revert("server returned an error response: execution reverted"));
        assert!(is_revert("Execution Reverted"));
        assert!(!is_revert("over rate limit"));
        assert!(!is_revert("error sending request for url"));
        assert!(!is_revert("connection closed before message completed"));
    }
}
