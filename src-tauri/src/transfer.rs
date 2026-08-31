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
use crate::wallet::unlock_signer;

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
fn humanize_chain_error(raw: &str, token: &str) -> String {
    let low = raw.to_lowercase();
    // ERC20 transfer 가 잔액 초과로 revert — USDC 경로에서만 나는 구체 revert 사유.
    if low.contains("transfer amount exceeds balance") {
        return ts!(
            "USDC 잔액이 부족해요. 충전 후 다시 시도하세요.",
            "Not enough USDC. Top up and try again."
        )
        .into();
    }
    // 가스(또는 ETH 송금액) 부족 — 트랜잭션을 낼 ETH가 모자람(가스는 항상 ETH라 토큰 무관).
    if low.contains("insufficient funds") {
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

/// 받는 주소 문자열을 파싱한다 (ETH/USDC 송금·x402 서명 공용).
pub(crate) fn parse_to_addr(to: &str) -> Result<Address, String> {
    to.trim().parse().map_err(|e| {
        tf!(
            "받는 주소가 올바르지 않습니다: {e}",
            "That recipient address isn't valid: {e}"
        )
    })
}

/// 서명 가능한(지갑 붙은) provider 를 만든다.
async fn signing_provider(signer: PrivateKeySigner) -> Result<impl Provider, String> {
    let wallet = EthereumWallet::from(signer);
    ProviderBuilder::new()
        .wallet(wallet)
        .connect(&effective_rpc())
        .await
        .map_err(|e| {
            tf!(
                "RPC 연결 실패: {}",
                "Couldn't reach the RPC server: {}",
                redact_urls(&e.to_string())
            )
        })
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
    // 비번 → 서명자. 실패하면(비번 오류 등) 시도로 기록하고 거부.
    let signer = match unlock_signer(&password) {
        Ok(s) => s,
        Err(e) => {
            log_attempt("ETH", to.trim(), amount_eth.trim(), "failed", &e);
            return Err(e);
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
) -> Result<String, String> {
    // 작업 진입 시 체인을 한 번 고정 — 이 송금의 한도·장부·RPC·내역이 모두 같은 체인을 본다.
    with_pinned_chain(
        active_chain().chain_id,
        do_send_eth_inner(signer, to, amount_eth),
    )
    .await
}

async fn do_send_eth_inner(
    signer: &PrivateKeySigner,
    to: String,
    amount_eth: String,
) -> Result<String, String> {
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
        return Err(msg);
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
            return Err(e);
        }
    };

    // 네트워크 전송은 락 밖에서 — 실패하면 예약한 사용액을 환불한다(예약한 날에만).
    let provider = match signing_provider(signer.clone()).await {
        Ok(p) => p,
        Err(e) => {
            refund_spend("ETH", value, reserved_day).await;
            log_attempt("ETH", to, amt, "failed", &e);
            return Err(e);
        }
    };
    let tx = TransactionRequest::default()
        .with_to(to_addr)
        .with_value(value);
    let pending = match provider.send_transaction(tx).await {
        Ok(p) => p,
        Err(e) => {
            refund_spend("ETH", value, reserved_day).await;
            let msg = humanize_chain_error(&redact_urls(&e.to_string()), "ETH");
            log_attempt("ETH", to, amt, "failed", &msg);
            return Err(msg);
        }
    };

    // 전송 성공 → 내역 로그 (누적 사용액은 예약 단계에서 이미 기록됨).
    let hash = pending.tx_hash().to_string();
    log_attempt("ETH", to, amt, "sent", &hash);

    Ok(hash)
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
    let signer = match unlock_signer(&password) {
        Ok(s) => s,
        Err(e) => {
            log_attempt("USDC", to.trim(), amount_usdc.trim(), "failed", &e);
            return Err(e);
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
) -> Result<String, String> {
    // 작업 진입 시 체인 고정 — decimals·USDC 컨트랙트·RPC·한도·장부·내역이 모두 같은 체인.
    with_pinned_chain(
        active_chain().chain_id,
        do_send_usdc_inner(signer, to, amount_usdc),
    )
    .await
}

async fn do_send_usdc_inner(
    signer: &PrivateKeySigner,
    to: String,
    amount_usdc: String,
) -> Result<String, String> {
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
            return Err(e);
        }
    };

    // 네트워크 전송은 락 밖에서 — 실패하면 예약한 사용액을 환불한다(예약한 날에만).
    let provider = match signing_provider(signer.clone()).await {
        Ok(p) => p,
        Err(e) => {
            refund_spend("USDC", value, reserved_day).await;
            log_attempt("USDC", to, amt, "failed", &e);
            return Err(e);
        }
    };
    let usdc = IERC20::new(active_chain().usdc_address, &provider);
    let pending = match usdc.transfer(to_addr, value).send().await {
        Ok(p) => p,
        Err(e) => {
            refund_spend("USDC", value, reserved_day).await;
            let msg = humanize_chain_error(&redact_urls(&e.to_string()), "USDC");
            log_attempt("USDC", to, amt, "failed", &msg);
            return Err(msg);
        }
    };

    // 전송 성공 → 내역 로그 (누적 사용액은 예약 단계에서 이미 기록됨).
    let hash = pending.tx_hash().to_string();
    log_attempt("USDC", to, amt, "sent", &hash);

    Ok(hash)
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
    #[ignore = "네트워크 필요 (Arc 테스트넷 공개 RPC)"]
    async fn arc_native_and_erc20_are_the_same_money() {
        use crate::chain::ARC_TESTNET;
        use alloy::eips::BlockId;
        use alloy::providers::Provider;

        let provider = ProviderBuilder::new()
            .connect(ARC_TESTNET.default_rpc)
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
        let erc20 = IERC20::new(ARC_TESTNET.usdc_address, &provider)
            .balanceOf(addr)
            .block(at)
            .call()
            .await
            .expect("ERC-20 잔액");

        // 18dp → 6dp 는 10^12 로 나눈 몫(내림).
        let scale = U256::from(10u64).pow(U256::from(12u64));
        println!(
            "Arc {addr} @ block {n}: native={native}  erc20={erc20}  native/1e12={}",
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
