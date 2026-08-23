# Kura (蔵 — 보물을 지키는 곳간)

**AI 에이전트를 위한 로컬 EVM 지갑.**
Mac에서 Claude 같은 AI가 인터넷 결제(x402)를 할 때, 사람이 비밀번호로 승인하는 지갑이에요.

- 사람용 지갑(메타마스크 등)이 아니라 **AI 에이전트용**
- 클라우드 SaaS 아님 — **로컬에서만** 돌아가요. 키는 이 컴퓨터를 떠나지 않아요.
- AI가 결제를 **요청** → 사람이 **비밀번호로 승인** → 실행
- 체인은 **Base**(이더리움 L2, 수수료 거의 0). 기본은 메인넷 — 연습용 테스트넷(Base Sepolia)으로 설정에서 전환할 수 있어요

---

## 한눈에 보기

```
[ AI 앱 (Claude Code / 데스크톱) ]
        │  MCP로 "결제해줘" 요청
        ▼
[ Kura 데스크톱 앱 ]  ← 승인 팝업 → 사람이 비밀번호 입력
        │  서명 (키는 이 앱 안에서만)
        ▼
[ Base 체인 / x402 페이실리테이터 ]  ← 실제 결제
```

비밀번호는 **Kura 앱의 입력칸에서만** 받아요. 채팅창·MCP·설정 파일 어디에도 들어가지 않아요.

---

## 설치

**Apple Silicon Mac (macOS 11 이상)**

### (A) Homebrew

```bash
brew install --cask dinggi5/tap/kura
```

### (B) DMG 직접 내려받기

[Releases](https://github.com/dinggi5/kura/releases/latest)에서 `Kura_<버전>_aarch64.dmg`를 받아 열고, Kura를 `Applications` 폴더로 끌어다 놓으세요.

열쇠를 맡기는 앱이니, **받은 앱이 진짜 내가 만든 것인지 확인하는 법**은 [SECURITY.md](SECURITY.md#배포본이-진짜인지-확인)에 명령까지 적어 뒀어요.

### (C) 소스에서 빌드

사전 준비: [Rust](https://rustup.rs), [Node.js](https://nodejs.org)(20.19+ 또는 22.12+ — Vite 7 요구), macOS.

```bash
git clone https://github.com/dinggi5/kura.git
cd kura
git checkout v0.1.1   # 배포본과 같은 소스. 빼면 개발 중인 최신 코드가 받아져요
npm ci                # package-lock.json 그대로 설치 (install 은 버전이 올라갈 수 있어요)

# 개발 모드로 바로 실행
npm run tauri dev

# 앱으로 빌드해 /Applications에 설치
npm run tauri build -- --bundles app --no-sign
ditto src-tauri/target/release/bundle/macos/Kura.app /Applications/Kura.app
open /Applications/Kura.app
```

### 업데이트

**앱이 스스로 업데이트해요.**  
**설정 → 정보**에서 버전과 바뀐 내용을 확인하고 **지금 설치하고 다시 시작**을 누르세요.

- 확인만 자동이고 **설치는 항상 직접 눌러야** 해요.
- 받은 파일은 서명이 맞는지 검사한 뒤에만 설치돼요(서명 검증은 끌 수 없어요). 자세한 건 [SECURITY.md](SECURITY.md)의 "업데이트" 절에 적어 뒀어요.
- 자동 업데이트 확인 통신이 싫으면 **설정 → 정보 → 시작할 때 확인**을 끄세요.

### 지우기

⚠️ **앱을 지워도 지갑은 지워지지 않아요.** 키·설정·거래 내역은 앱 바깥의 `~/.jigap` 폴더에 있어서, 앱만 지우면 그대로 남아요 — 실수로 자산을 날리지 않게 일부러 그렇게 뒀어요(`brew uninstall --zap`으로도 안 지워요).

```bash
# 앱만 제거 (지갑은 남음 — 나중에 다시 설치하면 그대로 이어서 써요)
brew uninstall --cask dinggi5/tap/kura   # 또는 /Applications/Kura.app 을 휴지통으로
```

이 맥에서 **지갑까지 완전히 지우려면**, 순서를 꼭 지키세요:

1. **12단어 복구 문구가 손에 있는지 먼저 확인하세요.** 앱의 헤더 열쇠 버튼에서 다시 볼 수 있어요.
2. 잔액이 남아 있다면 다른 지갑으로 옮기세요.
3. **Kura를 완전히 종료하세요** (메뉴바 아이콘 우클릭 → 종료). 켜져 있으면 몇 초마다 `~/.jigap`에 상태 파일을 쓰기 때문에, 지운 폴더가 곧바로 다시 생겨요.
4. Kura MCP를 붙여 둔 AI 도구(Claude Code 등)도 종료하세요.
5. 자동 시작을 켜 뒀다면 꺼 두거나 `rm ~/Library/LaunchAgents/Kura.plist` (앱을 휴지통으로 지우면 이 파일은 남아요. `brew uninstall --cask` 로 지우면 같이 내려가요).
6. 그다음에 `rm -rf ~/.jigap`

12단어 없이 마지막 `rm -rf ~/.jigap` 를 하면 **그 지갑의 자산은 누구도 되찾을 수 없어요.**

---

## 첫 실행

> **Kura는 메뉴바에 살아요.** 실행하면 화면 위쪽 메뉴바에 열쇠구멍 아이콘(◉)이 생기고, 창은 그 아이콘을 눌렀을 때만 아래로 펼쳐져요. 다른 곳을 클릭하면 닫히고, 앱은 백그라운드에 남아 AI 결제 요청을 계속 기다려요. 완전히 끄려면 아이콘을 **우클릭 → 종료**. (지갑이 아직 없는 첫 실행에는 창이 저절로 열려요.)

1. **지갑 만들기** — 송금할 때마다 입력할 비밀번호(8자 이상)를 정해요. 이 비번으로 키가 암호화돼 저장돼요(`~/.jigap/wallet.enc`).
2. **시드 백업** — 12개 단어가 나와요. 이게 **자산의 진짜 소유 증명**이라 종이나 비밀번호 관리자에 적어두세요. 비밀번호를 잊어도 12단어만 있으면 되찾을 수 있어요: 앱을 종료하고 **12단어가 손에 있는지 먼저 확인한 뒤**, `~/.jigap/wallet.enc`를 지우고 앱을 다시 실행하면 첫 화면의 **가져오기**로 복구돼요(다른 표준 BIP-39 지갑에 넣어도 돼요). 단, 12단어 없이 이 파일을 지우면 자산을 영영 잃어요.
3. **환영 투어** — 충전·AI 연결·안전장치를 짚어줘요. 헤더의 **ⓘ 도움말**에서 언제든 다시 볼 수 있어요.

---

## 돈 채우기 (USDC)

결제하려면 지갑에 **USDC**(디지털 달러)가 있어야 해요.

- 앱에서 **받기**를 누르면 주소와 QR이 나와요.
- 거래소·다른 지갑에서 보낼 땐 **반드시 Base 네트워크**를 고르세요. 다른 네트워크로 보내면 자금을 잃어요.
- **메인넷(기본)** 은 실제 돈이에요 — 운영 예산만 소액 충전하세요. 먼저 연습하고 싶으면 설정에서 **테스트넷**으로 바꾸면 받기 화면의 Faucet 버튼으로 공짜 테스트 코인을 받을 수 있어요.
- **ETH는 선택이에요** — x402 결제는 수수료를 페이실리테이터가 대신 내서 ETH가 0이어도 돼요. 앱의 **보내기**로 직접 송금할 때만 가스용 ETH가 조금 필요해요(Base 수수료는 1센트 안팎). 충전은 USDC만 하면 충분해요.

---

## AI(Claude) 연결하기

AI 앱의 **MCP 설정**에 Kura 서버를 등록하면 연결돼요. 연결되면 메인 화면에 **"Claude 연결됨"** 배지가 떠요. 0.1.2부터는 **앱 안에 MCP 서버가 들어 있어서** 소스 클론이나 Rust 설치 없이 연결할 수 있어요.

### 제일 쉬운 길 — 앱 안 "AI 연결" 화면

메인 화면 위쪽의 **"AI 연결 안 됨" 배지를 누르면** 연결 화면이 열려요.

- **Claude 데스크톱** — "연결" 버튼을 누르면 Claude에 확장 설치 창이 떠요. '설치'만 누르면 끝.
- **Claude Code** — 버튼 한 번으로 등록돼요(`claude mcp add`를 앱이 대신 실행). 다음 `claude` 실행부터 어느 폴더에서든 연결돼요.

아래는 같은 일을 손으로 하는 방법이에요.

### Claude 데스크톱 — 확장 파일 하나로 끝

[릴리스 페이지](https://github.com/dinggi5/kura/releases/latest)에서 `kura-<버전>.mcpb`를 받아 **더블클릭**하면 Claude 데스크톱이 설치를 물어봐요. (또는 Claude 설정 → 확장 → 파일 선택.)

확장 안에는 실행 파일이 없어요 — 설치된 Kura.app 안의 서명·공증된 MCP 서버를 **서명 확인 후** 실행하는 런처만 들어 있어요. 그래서 Kura 앱을 먼저 설치해야 해요.

### Claude Code · 다른 MCP 앱

앱 안의 바이너리를 절대경로로 등록하면 돼요:

```bash
claude mcp add --scope user kura -- /Applications/Kura.app/Contents/MacOS/kura-mcp
```

(`--scope user`는 "이 폴더만"이 아니라 어디서든 쓰이게 하는 옵션이에요. 한 프로젝트에서만 쓸 거면 빼도 돼요.)

다른 MCP 앱은 설정에 아래처럼 적어요:

```json
{
  "mcpServers": {
    "kura": {
      "command": "/Applications/Kura.app/Contents/MacOS/kura-mcp"
    }
  }
}
```

### 소스에서 개발 중이라면

리포 루트의 [`.mcp.json`](.mcp.json)이 이미 들어 있어요. 이 폴더에서 `claude`를 실행하면 자동으로 잡혀요(처음엔 서버를 쓸지 묻는데 **승인**하면 돼요). 이 경로는 `cargo run`으로 매번 새로 빌드하는 개발용이에요 — 설치해서 쓰실 거라면 위의 앱 경로 등록이 맞아요.

AI가 쓸 수 있는 도구: `get_wallet_status` · `get_balances` · `get_history`(읽기 전용) · `request_payment`(결제 요청 → 앱 승인 팝업) · `x402_fetch`(402 결제가 걸린 URL 호출).

> MCP 도구를 바꾸면 **AI 앱을 재시작**해야 반영돼요(서버가 세션 시작 때 1회 로드).

---

## 결제는 이렇게 흘러가요

1. AI가 결제를 요청해요.
2. Kura 창이 떠서 **얼마를, 어디로** 보내는지 보여줘요(창이 숨어 있어도 자동으로 떠요).
3. 비밀번호를 넣고 승인해요.
4. 결제가 실행되고 결과가 AI에게 돌아가요. **5분 안에 승인 안 하면 자동 거부.**

---

## 보안 모델

- **비밀번호 승인** — 기본값은 매 결제마다 비밀번호 승인이에요(자율 결제를 켜면 아래 조건에서만 예외). 키는 암호화(Argon2id + AES-256-GCM)돼 저장되고, 결제할 때만 복호화해 바로 지워요(자율 결제 세션을 켜면 잠금 해제 동안에만 메모리에 보관).
- **한도** — 한 번에 / 하루에 얼마까지 보낼지 설정에서 정해요(기본 단일 5 · 일일 20 USDC). 넘으면 막혀요.
- **긴급 잠금** — 헤더 방패 버튼을 켜면 모든 송금이 즉시 차단돼요.
- **신뢰 주소 · 자율 결제** — 비번 없이 자동 승인하려면 *세션 잠금 해제 + 소액 한도 + 신뢰 주소* 셋 다 맞아야 해요. 그 외엔 항상 비번.
- **거래 내역** — 보낸 송금·차단된 시도·서명을 전부 기록해요.
- **로컬 우선** — 키(`~/.jigap/`)는 리포 밖에 있고 git에 올라가지 않아요.
- **분석·폰트 CDN 없음** — 앱이 스스로 인터넷에 연결하는 건 셋뿐이에요. ①잔액 조회·송금에 쓰는 RPC 서버 ②AI가 요청한 x402 주소 ③업데이트 확인(깃허브). 사용 기록은 어디로도 보내지 않고, 글꼴도 앱 안에 들어 있어서 오프라인에서도 화면은 그대로 떠요.
- **업데이트 확인은 끌 수 있어요** — 앱을 켤 때 깃허브에 새 버전이 있는지 물어봐요(그쪽엔 접속 IP와 현재 버전이 남아요). **설정 → 정보 → 시작할 때 확인**에서 끄면 이 통신도 없어져요. 확인을 켜 두든 끄든 **설치는 직접 눌러야** 진행돼요 — 몰래 바뀌는 일은 없어요.

---

## 개발자용

CLI `kura`로도 같은 지갑을 다룰 수 있어요(MCP 서버와 코어 로직 공유). 설치된 앱에 이미 들어 있어서, PATH에 한 줄 걸면 바로 써요:

```bash
sudo mkdir -p /usr/local/bin
sudo ln -sf /Applications/Kura.app/Contents/MacOS/kura-cli /usr/local/bin/kura
kura status
```

소스에서 돌리려면:

```bash
cargo run --manifest-path ./kura-mcp/Cargo.toml --bin kura -- status
cargo run --manifest-path ./kura-mcp/Cargo.toml --bin kura -- balance
cargo run --manifest-path ./kura-mcp/Cargo.toml --bin kura -- history --limit 10
cargo run --manifest-path ./kura-mcp/Cargo.toml --bin kura -- pay <주소> <금액> --token usdc
cargo run --manifest-path ./kura-mcp/Cargo.toml --bin kura -- fetch <URL>
```

`pay`/`fetch`는 **비번을 CLI로 받지 않아요** — Kura 앱이 떠 있어야 하고, 승인 팝업에서 사람이 비번을 넣어요.

테스트:

```bash
(cd src-tauri && cargo test)        # 백엔드(지갑·암호화·한도·송금)
(cd kura-mcp && cargo test)         # MCP/CLI 어댑터
npx tsc --noEmit && npx vite build  # 프론트
```

배포본(서명·공증된 DMG)을 만드는 절차는 **[docs/RELEASE.md](docs/RELEASE.md)** 에 있어요. Apple Developer 계정과 1회 설정이 필요하고, 그다음부터는 한 줄이에요:

```bash
./scripts/release.sh
```

### 기술 스택

| | |
|---|---|
| 데스크톱 | Tauri (Rust + 웹 프론트) |
| 프론트 | React + Tailwind CSS, Framer Motion, Lucide, Pretendard |
| 체인 | Base / alloy-rs |
| 결제 | x402 (EIP-3009 오프체인 서명) |
| AI 연결 | MCP 서버 (rmcp) |

구조: `[Rust 코어(src-tauri)] ← MCP / CLI 어댑터(kura-mcp)`. 키 접근(서명)은 GUI 프로세스만 — 그게 최종 방어선이에요.

---

## 라이선스 · 상태

**[MIT 라이선스](LICENSE)** — 열쇠와 돈을 다루는 코드라, 직접 읽고 검증하고 고쳐 쓸 수 있어야 한다고 봐요.

취약점을 발견하셨다면 이슈로 올리기 전에 **[SECURITY.md](SECURITY.md)** 의 비공개 경로로 알려주세요. 같은 문서에 **Kura가 막지 못하는 것**도 적어뒀어요.

함께 담긴 [Pretendard](https://github.com/orioncactus/pretendard) 글꼴은 **SIL Open Font License 1.1**이에요([전문](public/fonts/LICENSE-Pretendard.txt)).

1순위 사용자는 본인 — Apple Silicon Mac에서 로컬 LLM/Claude로 결제하는 한국 개발자·창작자를 위한 지갑이에요. 아직 초기 버전이라, 메인넷에는 잃어도 되는 만큼만 넣으시고 — 익숙하지 않으면 설정에서 테스트넷으로 바꿔 먼저 연습해 보세요.
