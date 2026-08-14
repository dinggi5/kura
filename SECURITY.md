# 보안

Kura는 개인키를 다루는 지갑이에요. 취약점을 발견하셨다면 이슈로 올리기 전에 아래로 먼저 알려주시면 고맙겠어요.

## 신고하는 법

1. **GitHub 비공개 신고 (권장)** — 저장소의 **Security → Report a vulnerability**. 저장소 주인만 볼 수 있어요.
2. 그게 안 되면 커밋 작성자 주소(`dinggi5@proton.me`)로 메일.

혼자 만드는 프로젝트라 24시간 대응은 못 해요. **일주일 안에 답장**하는 걸 목표로 하고, 답이 없으면 다시 한 번 보내주세요.

고쳐지기 전까지 자세한 내용을 공개하지 말아주세요. 고친 뒤에는 원하시면 릴리스 노트에 이름을 올려드려요(금전 보상은 없어요).

## 지원 버전

| 버전 | 지원 |
|---|---|
| 최신 릴리스 | ✅ |
| 그 이전 | ❌ — 최신으로 올려주세요 |

0.1.x 초기 버전이라 보안 수정은 **최신 릴리스에만** 반영해요. 아직 앱 안에 자동 업데이트가 없어서, Homebrew를 쓰거나 [Releases](https://github.com/dinggi5/kura/releases)를 Watch 해두시길 권해요.

## 관심 있는 취약점

- 개인키·복구 문구가 암호화된 파일(`~/.jigap/wallet.enc`) 밖으로 새는 경로
- 비밀번호 없이 서명·송금이 되는 경로
- MCP 어댑터로 **사람 승인 없이** 결제가 나가는 경로 (승인 팝업 우회, 요청 위조)
- 거래 한도·긴급 잠금·화이트리스트를 우회하는 방법
- 승인 팝업이 **실제와 다른 금액·주소**를 보여주게 만드는 방법
- 배포본 무결성 (서명·공증·업데이트 경로)

## 여기서 다루지 않는 것

아래는 Kura가 **막지 못해요.**

- **이미 뚫린 맥.** 키로거·화면 녹화·임의 코드 실행이 가능한 상태면 비밀번호를 입력하는 순간 끝이에요. 로컬 지갑의 근본 한계예요.
- **비밀번호를 잊었는데 12단어도 없는 경우.** 복구 수단이 없어요. 설계가 그래요 — 되살릴 방법이 있으면 남도 되살릴 수 있으니까요.
- **사용자가 승인한 결제.** 팝업에 뜬 금액·주소를 확인하고 비밀번호를 넣는 건 최종 결정이에요. AI가 속아서 요청했더라도 승인은 사람이 해요.
- **RPC 서버와 체인이 보는 것.** 잔액 조회·송금은 설정에서 고른 RPC 서버로 나가요 — 그쪽은 IP와 주소를 볼 수 있어요. 거래는 공개 블록체인에 영구히 남고요. 키가 로컬에 있다는 것과 활동이 비공개라는 건 다른 얘기예요.
- **다른 지갑·거래소·x402 상대방의 문제.** Kura 밖의 일이에요.

## 배포본이 진짜인지 확인

설치한 앱은 이걸로 확인할 수 있어요:

```bash
codesign --verify --deep --strict /Applications/Kura.app   # 서명 후 변조 여부 (실제 검증)
codesign -dv --verbose=2 /Applications/Kura.app 2>&1 | grep -E "TeamIdentifier|Authority=Developer ID"
spctl -a -vv /Applications/Kura.app
```

첫 줄이 조용히 끝나야(출력 없음 = 통과) 하고, **팀 ID는 `74ZAMXKVXN`**, 판정은 `accepted / source=Notarized Developer ID` 여야 해요. 다르면 쓰지 말고 신고해주세요.

`codesign -dv` 는 서명 정보를 **보여주기만** 해요. 변조 여부를 실제로 검사하는 건 `--verify` 쪽이에요.

배포는 [GitHub Releases](https://github.com/dinggi5/kura/releases)와 Homebrew tap(`dinggi5/tap`) **두 곳뿐**이에요. 다른 데서 받은 Kura는 저와 무관해요.

---

## English

Kura is a wallet that handles private keys. If you find a vulnerability, please tell me privately before opening an issue.

Report via **Security → Report a vulnerability** on this repository, or email `dinggi5@proton.me`. This is a solo project — expect a reply within a week, and please ping again if you don't hear back. Please hold public disclosure until a fix ships. Credit in release notes if you'd like it; no bug bounty.

Security fixes land on the **latest release only** (0.1.x).

**Most interesting:** private key or recovery phrase escaping `~/.jigap/wallet.enc`; signing or sending without the password; the MCP adapter paying without human approval; bypassing spend limits, emergency lock, or the allowlist; making the approval dialog display an amount or address different from what gets signed; distribution integrity.

**Out of scope:** an already-compromised Mac; forgotten password with no 12-word backup (there is no recovery path by design); payments the user approved; what the RPC provider and the public chain can observe; third-party wallets, exchanges, or x402 counterparties.

**Verify a build:** Team ID must be `74ZAMXKVXN` and `spctl -a -vv` must report `accepted / source=Notarized Developer ID`. Kura ships only via GitHub Releases and the `dinggi5/tap` Homebrew tap.
