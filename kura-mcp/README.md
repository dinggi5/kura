# kura-mcp

**Kura 어댑터 크레이트** — AI 에이전트(MCP)와 사람(CLI)이 로컬 지갑을 조회·결제한다.

이 프로젝트의 존재 이유("AI가 호출하는 로컬 지갑")로 들어가는 어댑터 계층이다. Tauri
앱(`../src-tauri`)과 분리된 독립 크레이트로, `~/.jigap` 파일을 읽고 활성 Base 네트워크
RPC로 잔액을 조회하며, 결제는 GUI 앱에 "요청"만 한다. **비밀(니모닉/키)은 절대 건드리지
않는다** — 서명·전송은 키를 쥔 GUI 앱이 사람 승인을 받아 수행한다(파일 기반 IPC).

## 두 바이너리, 한 lib

같은 lib(`wallet`/`flow`/`payment`/`x402`/`chain`)을 공유하므로 결제·보안 로직이 한 벌이다.

| 바이너리 | 대상 | 진입점 |
|---------|------|--------|
| `kura-mcp` | AI 에이전트 (stdio MCP 서버) | `src/main.rs` |
| `kura` | 사람·스크립트 (터미널 CLI) | `src/bin/kura.rs` |

`default-run = "kura-mcp"` 이므로 `.mcp.json` 의 `cargo run`(--bin 없음)은 MCP 서버를 띄운다.

## 기능 (MCP 도구 ↔ CLI 명령)

| 기능 | MCP 도구 | CLI 명령 | 비번 |
|------|----------|----------|------|
| 지갑 상태·주소 | `get_wallet_status` | `kura status` | 불필요 |
| USDC·가스 잔액 | `get_balances` | `kura balance` | 불필요 |
| 거래 내역 | `get_history` | `kura history [--limit N]` | 불필요 |
| 송금 요청 | `request_payment` | `kura pay <주소> <금액> [--token USDC\|ETH] [--memo "사유"]` | **GUI 승인** |
| x402 유료 리소스 | `x402_fetch` | `kura fetch <URL> [--memo "사유"]` | **GUI 승인** |
| 에이전트 신원 조회 | `lookup_agent` | (CLI 없음 — `kura fetch --agent N` 으로 대조) | 불필요 |

읽기 명령은 즉시. 결제 명령(`pay`/`fetch`)은 지갑 앱이 팝업으로 사람 승인을 받아야만
실행된다(최대 5분 대기). 단일/일일 한도·긴급잠금·화이트리스트는 GUI 가 강제한다.
**비밀번호는 절대 MCP/CLI/인자로 받지 않는다 — GUI 입력칸에서만.**

## CLI 사용

```bash
# 빌드 (릴리즈)
cargo build --release --bin kura       # → target/release/kura

# PATH 에 올리기 (선택)
ln -sf "$(pwd)/target/release/kura" /usr/local/bin/kura

# 읽기 — 즉시
kura status
kura balance
kura history --limit 10
kura balance --json                    # 스크립트용 JSON (MCP 와 동일 형태)

# 결제 — 지갑 앱이 떠 있어야 하고, 팝업에서 비번 승인 필요
kura pay 0xRecipient... 1.5 --memo "데이터 API"
kura fetch https://example.com/paid --memo "리포트 1건"
kura fetch https://example.com/paid --agent 1   # ERC-8004 번호를 함께 넘겨 승인 창에서 대조
```

`--json` 은 모든 명령에서 기계가 읽는 출력을 준다(읽기는 구조체, 결제는 MCP 와 동일 JSON).
실패 시 종료코드 1.

### 체인 고정 (`KURA_CHAIN_ID`)

평소엔 GUI 와 공유하는 `settings.json` 의 `chain_id` 로 활성 체인을 정한다. 테스트·스크립트가
라이브 설정에 의존하지 않게 하려면 환경변수로 체인을 고정할 수 있다:

```bash
KURA_CHAIN_ID=84532 kura balance       # 강제로 Base Sepolia 에서 조회
KURA_CHAIN_ID=5042002 kura balance    # Arc 테스트넷 (가스도 USDC — 잔액에 ETH 줄이 없다)
```

지원 값은 `8453`(Base 메인넷) · `84532`(Base Sepolia) · `5042002`(Arc 테스트넷) 셋뿐이고,
그 밖의 값은 **조용히 폴백하지 않고 즉시 종료**한다(오타가 실돈 체인으로 도는 것 방지).

GUI 와 다른 체인을 가리켜도, 결제 요청에 각인된 chain_id 를 GUI 가 승인 시 대조해 거부하므로
(개발 20 가드) 잘못된 체인으로 송금되지 않는다.

이 환경변수로 **설정과 다른 체인**을 고정하면 `settings.json` 의 `rpc_url` 은 그 체인의
엔드포인트가 아니므로 쓰지 않고, 고정한 체인의 공개 RPC 로 폴백한다(개발 49). 커스텀 RPC 를
쓰는 사람이 이 변수로 체인을 갈아탔을 때 잔액 조회가 `returned no data ("0x")` 로 죽던 걸 막는다.

## Claude Code에 연결 (MCP)

레포 루트의 [`.mcp.json`](../.mcp.json)에 이미 등록돼 있다. 프로젝트 디렉터리에서 Claude
Code를 실행하면 `kura` 서버가 자동으로 뜬다(첫 실행 시 `cargo` 빌드). 도구를 바꾸면 **Claude
Code 재시작**해야 새 바이너리가 로드된다.

수동 등록:

```bash
claude mcp add kura -- cargo run --quiet --manifest-path ./kura-mcp/Cargo.toml
```

## 검증

```bash
cargo test                  # 단위(비밀 차단 계약·파서·CLI 파싱 등) + x402 끝과끝 통합
cargo clippy --all-targets  # 경고 0
```

## 메모

- `RPC_URL` / `USDC_ADDRESS` 등 체인 상수는 `src-tauri` 와 **의도적 중복**(공유 크레이트를 안
  만드는 정책 — Tauri 빌드 위험 0). `chain.rs` 가 두 크레이트 평행 사본.
- HTTP·결제 오케스트레이션(리다이렉트 가드·정산 게이팅·single-flight)은 `flow.rs` 한 곳에
  모아 MCP·CLI 두 어댑터가 공유한다(로직 분기 방지).
- MCP SDK: [`rmcp`](https://crates.io/crates/rmcp) 0.16 (stdio 전송, 매크로 기반).
