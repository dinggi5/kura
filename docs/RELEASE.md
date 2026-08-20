# 배포본 만들기 — 서명과 공증

Kura 를 남에게 줄 수 있는 `.dmg` 로 만드는 절차.

## 왜 이게 필요한가

macOS 는 인터넷에서 받은 앱을 **Gatekeeper** 로 검사한다. 애플이 발급한
**Developer ID** 인증서로 서명돼 있고, 애플 서버에 올려 악성코드 스캔을 통과
(**공증, notarization**)한 앱만 그냥 열린다. 둘 중 하나라도 없으면 받는 사람은
"확인되지 않은 개발자" 경고를 보고, macOS Sequoia 부터는 우클릭 → 열기 우회도
막혀서 **시스템 설정까지 들어가 수동으로 허용**해야 한다.

돈을 다루는 앱이 "보안 경고를 뚫으세요"라고 안내하는 건 신뢰상 최악이라,
직접 배포(DMG)든 Homebrew 든 서명·공증은 사실상 필수다.
(본인이 소스로 직접 빌드해 쓸 때는 필요 없다.)

**공증 ≠ App Store 심사.** 공증은 자동 악성코드 스캔이라 사람 심사가 없고,
암호화폐 지갑을 막는 App Store 규정 3.1.5 도 적용되지 않는다. 그래서 개인
Apple Developer 계정(₩129,000/년)으로 충분하다.

---

## 1회 설정

한 번만 해두면 이후로는 `./scripts/release.sh` 한 줄이면 된다.

### ① Developer ID Application 인증서 만들기

지금 맥에 있는 인증서를 먼저 확인한다.

```bash
security find-identity -v -p codesigning
```

`Developer ID Application: …` 로 시작하는 줄이 있으면 이 단계는 건너뛴다.
`Apple Development: …` 밖에 없다면 그건 **내 맥에서 돌려보는 용도**라 배포에는 못 쓴다.

Xcode 로 만드는 게 제일 간단하다 (개인키가 로그인 키체인에 바로 들어간다).

1. Xcode 실행 → 메뉴 **Xcode → Settings…** (⌘,)
2. **Accounts** 탭 → 왼쪽에서 본인 Apple ID 선택 → 오른쪽 아래 **Manage Certificates…**
3. 왼쪽 아래 **+** → **Developer ID Application** 선택
4. 창을 닫고 위 `security find-identity` 를 다시 실행해 새 줄이 생겼는지 확인

> ⚠️ **개인키를 잃으면 그 인증서로 다시 서명할 수 없다.** 맥을 옮기거나 초기화하기
> 전에 백업해 둘 것: **키체인 접근** 앱 → *내 인증서* → 해당 인증서 우클릭 →
> **내보내기** → `.p12` 로 저장(암호 걸림). 이 `.p12` 는 리포에 넣지 말 것
> (`.gitignore` 가 이미 막고 있다).

> ℹ️ Developer ID Application 인증서는 계정당 **최대 5개**, 유효기간 5년이다.
> 필요 이상으로 만들지 말 것.

### ② 공증용 자격증명 만들기 (App Store Connect API 키 — 권장)

비밀번호를 어디에도 타이핑하지 않고, 나중에 언제든 취소할 수 있어서 이 방식이 낫다.

1. [appstoreconnect.apple.com](https://appstoreconnect.apple.com) 로그인
2. **사용자 및 액세스(Users and Access)** → **통합(Integrations)** 탭 →
   **App Store Connect API** → **팀 키(Team Keys)**
3. **+** → 이름 `Kura Notarize`, 역할(Access)은 **Developer** → 생성
4. **`.p8` 파일은 딱 한 번만 받을 수 있다.** 바로 다운로드할 것
5. 같은 화면의 **키 ID**(짧은 문자열)와 맨 위의 **발급자 ID(Issuer ID)**(긴 UUID)를 복사해 둔다

받은 파일을 제자리에 옮기고 권한을 잠근다 (`AuthKey_…` 부분은 실제 파일명으로):

```bash
mkdir -p ~/.appstoreconnect/private_keys && mv ~/Downloads/AuthKey_*.p8 ~/.appstoreconnect/private_keys/ && chmod 600 ~/.appstoreconnect/private_keys/*.p8 && ls -l ~/.appstoreconnect/private_keys/
```

<details>
<summary>대신 앱 전용 암호를 쓰고 싶다면</summary>

[appleid.apple.com](https://appleid.apple.com) → **로그인 및 보안** → **앱 암호** →
`Kura Notarize` 로 하나 생성. 계정 비밀번호가 아니라 여기서 만든 16자리다.
`release.env` 의 방법 B 세 줄(`APPLE_ID`·`APPLE_PASSWORD`·`APPLE_TEAM_ID`)을 채우고
방법 A 세 줄은 지운다. 팀 ID 는
[developer.apple.com/account](https://developer.apple.com/account) → *Membership details* 에 있다.
(`Developer ID Application: 이름 (XXXXXXXXXX)` 의 괄호 안 값과 같은 값이다. 단
`Apple Development:` 인증서의 괄호 안은 팀 ID 가 아니니 그걸 보고 베끼면 안 된다.)

이 방식은 공증하는 동안 암호가 프로세스 인자로 잠깐 노출된다. 혼자 쓰는 맥이면
실질적 문제는 없지만, 그래서 API 키를 권한다.
</details>

### ③ `release.env` 채우기

```bash
cp scripts/release.env.example scripts/release.env && chmod 600 scripts/release.env && open -e scripts/release.env
```

열린 파일에 ①의 인증서 이름과 ②의 키 ID·발급자 ID·`.p8` 경로를 적고 저장한다.
인증서 이름은 `security find-identity` 출력의 큰따옴표 안 내용을 **그대로** 복사한다.

이 파일은 `.gitignore` 에 들어 있어 커밋되지 않는다. 개인 계정이라 서명 이름에
법적 실명이 들어가기 때문에 일부러 리포 밖에 두는 것이다.

**워크트리에서 빌드해도 된다(개발 32).** 이 파일은 커밋에 없으니 워크트리에는 구조상
절대 생기지 않는데, 이 프로젝트는 워크트리에서 빌드하는 게 기본 흐름이다. `release.sh`
는 워크트리에 파일이 없으면 **메인 워킹트리**(`git rev-parse --git-common-dir` 의 부모)
의 `scripts/release.env` 를 읽는다. 즉 위 명령은 **메인 워킹트리에서 한 번만** 하면 된다.
심링크로 때우지 말 것 — 권한 검사가 `stat -f '%Lp'` 로 링크 자신의 모드(777)를 보므로
정식 배포가 게이트에 막힌다.

### ④ 업데이트 서명 키 만들기 (개발 31 — 1회, 그리고 다시 못 만든다)

인앱 업데이트는 minisign 서명이 맞아야만 설치된다. 키는 **한 번 만들고 영원히 같은 걸
쓴다** — 바꾸면 그 전 버전을 쓰는 사람들은 새 업데이트를 설치할 수 없다(그들 앱에는 옛
공개키가 박혀 있으니까). 잃어버려도 마찬가지다.

```bash
npm run tauri signer generate -- -w ~/.tauri/kura-updater.key
chmod 600 ~/.tauri/kura-updater.key
```

암호를 물으면 **강한 것으로 정해 비밀번호 관리자에 넣는다.**

- 개인키 `~/.tauri/kura-updater.key` + 그 암호 → **이 둘을 잃으면 자동 업데이트는 끝난다.**
  백업은 비밀번호 관리자나 오프라인 매체에. 클라우드 동기화 폴더·리포에는 두지 않는다.
- 공개키 `~/.tauri/kura-updater.key.pub` → 파일 내용을 그대로 `src-tauri/tauri.conf.json`
  의 `plugins.updater.pubkey` 에 넣는다(자리표시자 `REPLACE_WITH_UPDATER_PUBLIC_KEY` 를 교체).
  공개키는 공개돼도 안전하다 — 앱에 박혀서 배포된다.

`scripts/release.env` 에 아래를 추가한다:

```bash
TAURI_SIGNING_PRIVATE_KEY_PATH="$HOME/.tauri/kura-updater.key"
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="위에서 정한 암호"
```

> 🔴 **이 키는 Developer ID 인증서만큼, 어떤 면에선 그보다 중요하다.** 인증서가 새면
> "내 이름으로 서명된 새 악성 앱"이 만들어지지만, 이 키가 새면 **이미 사람들이 믿고
> 설치해 둔 지갑들**에 임의 코드를 밀어넣을 수 있다. `SECURITY.md` 의 "업데이트" 절에
> 사용자에게 약속한 내용이 있으니, 키 취급을 바꾸면 그 문서도 같이 고칠 것.

`release.sh` 가 사전 점검에서 공개키가 자리표시자인지 보고, 빌드 뒤에는 나온 서명이
**앱에 박은 그 공개키로 만들어졌는지** 키 ID 로 대조한다. 어긋나면 배포가 멈춘다 —
이 실패는 안 막으면 배포가 다 끝난 뒤 "다음 버전을 아무도 설치 못 한다"로만 드러난다.

---

## 배포본 만들기

```bash
./scripts/release.sh
```

**처음 실행하면 두 개의 팝업이 뜬다. 둘 다 허용해야 한다.**

- *"…이(가) Finder을(를) 제어하려고 합니다"* — DMG 안 아이콘 배치를 Finder 에게
  시키는 단계다. 거부하면 빌드를 다 마친 뒤에야 `error running bundle_dmg.sh` 로 죽는다.
  스크립트는 이걸 **빌드 전에** 미리 확인해서 몇 분을 날리지 않게 해 둔다.
- 키체인이 서명 키를 쓰겠다고 묻는 팝업 — *항상 허용* 을 누르면 다음부터 안 묻는다.

스크립트가 하는 일:

1. **사전 점검** — 인증서·자격증명·버전 일치(`tauri.conf.json`/`package.json`/두 `Cargo.toml`/`mcpb/manifest.json`)·커밋 안 된 변경 확인
2. **시험 서명** — 작은 파일을 실제로 한 번 서명해서 **보안 타임스탬프**가 붙는지 확인.
   이게 없으면 애플이 공증을 거부하는데, 빌드가 다 끝난 뒤에 알면 시간이 아깝다
3. **테스트** — `src-tauri` + `kura-mcp` 의 러스트 테스트와 타입 검사
4. **빌드·서명** — 사이드카(kura-mcp·kura-cli)를 자격증명 없이 먼저 만들고, Tauri 가 하드닝 런타임으로 앱(사이드카 포함)을 서명하고, 애플에 올려 공증받고, 티켓을 앱에 박는다(staple). 앱 안의 사이드카는 서명·아키텍처·버전에 더해 **실제 MCP 핸드셰이크**까지 확인한다(개발 34)
5. **업데이트 산출물 검증**(개발 31) — `Kura.app.tar.gz` 의 서명 키 ID 가 앱에 박은 공개키와
   같은지 대조하고, **tar 를 풀어서 안에 든 앱**의 서명·팀 ID·번들 ID·버전·공증 티켓까지 본다.
   DMG 만 검사하면 인앱 업데이트로 나가는 파일은 아무도 안 본 채 배포된다
6. **DMG 공증** — Tauri 는 앱까지만 공증하므로 DMG 는 스크립트가 따로 올려 티켓을 박는다
7. **검증** — `codesign` / `stapler` / `spctl` 로 실제 Gatekeeper 판정을 확인
8. **`latest.json` 생성** — 앱이 물어보는 유일한 파일. 방금 검증한 산출물에서 버전·URL·서명을
   그대로 만들어 낸다(손으로 쓰면 하나 틀려도 증상 없이 업데이트만 조용히 안 된다)
9. **Claude 데스크톱 확장** — `scripts/build-mcpb.sh` 로 `kura-<버전>.mcpb` 를 만든다. 실행 파일 없는 런처라 공증 대상이 아니다(개발 34)
10. **결과** — DMG 경로와 `sha256`(Homebrew Cask 에 넣을 값), 업데이트 3종·확장 경로 출력

공증은 애플 서버 응답을 기다리므로 **처음엔 5~15분** 걸릴 수 있다. 그동안 맥이
잠들지 않게 두는 게 좋다.

### 옵션

| 옵션 | 언제 |
|---|---|
| `--universal` | 인텔 맥까지 지원. 먼저 `rustup target add x86_64-apple-darwin` 필요. 기본은 Apple Silicon 전용(주 사용자층과 일치, 용량 절반) |
| `--no-notarize` | 자격증명 없이 서명 경로만 점검. **결과물을 배포하면 안 된다** |
| `--plain-dmg` | Finder 제어 권한을 못 주는 환경(원격 접속·CI)에서. DMG 안 아이콘 배치를 건너뛴다 — 설치 동작은 같고 모양만 밋밋해진다 |
| `--skip-tests` | 테스트 건너뛰기. 급할 때만 (`--publish` 와는 같이 못 쓴다) |
| `--publish` | 빌드·검증이 통과하면 태그·릴리스·캐스크까지 (아래 "새 버전 내기"). `--universal`·`--no-notarize`·`--skip-tests`·`--allow-dirty` 와는 같이 못 쓴다 |
| `--yes` | `--publish` 의 확인 프롬프트 생략. 사람이 안 보는 자리에서만 |
| `--replace-assets` | 릴리스에 이미 올라간 자산을 이번 빌드로 덮어쓴다. **릴리스가 만들어진 뒤 실패해서 다시 돌릴 때만** (아래) |
| `--allow-dirty` | 커밋 안 된 변경이 있어도 진행. 어떤 소스로 만든 배포본인지 추적이 안 되므로 비권장 |

---

## 잘 됐는지 확인하는 법

```bash
spctl -a -t open --context context:primary-signature -vvv src-tauri/target/release/bundle/dmg/Kura_*_aarch64.dmg
```

`accepted` 와 `source=Notarized Developer ID` 가 나오면 남의 맥에서도 경고 없이 열린다.
(`--universal` 로 만들었으면 파일 이름이 `…_universal.dmg` 다. 헷갈리면 스크립트가
마지막에 출력한 **파일** 경로를 그대로 쓰면 된다.)

받은 사람 입장을 그대로 재현하려면, DMG 를 다른 폴더로 복사한 뒤 격리 속성을 붙여
열어 본다(브라우저로 내려받은 파일과 같은 상태가 된다).

```bash
cp src-tauri/target/release/bundle/dmg/Kura_*_aarch64.dmg /tmp/kura-test.dmg && xattr -w com.apple.quarantine "0081;00000000;Safari;" /tmp/kura-test.dmg && open /tmp/kura-test.dmg
```

---

## 문제가 생기면

**`키체인에서 인증서를 못 찾음`**
`release.env` 의 `APPLE_SIGNING_IDENTITY` 가 `security find-identity -v -p codesigning`
출력의 큰따옴표 안 문자열과 글자 하나까지 같아야 한다.

**공증이 `Invalid` 로 거부됨**
스크립트가 애플이 보낸 사유를 그대로 출력한다. 흔한 원인은 하드닝 런타임 누락,
서명 안 된 실행 파일이 앱 안에 섞여 있음, 안전한 타임스탬프 누락(빌드 중 네트워크 끊김)이다.
나중에 다시 보려면 제출 ID 로 조회한다.

```bash
xcrun notarytool log "제출ID" --key ~/.appstoreconnect/private_keys/AuthKey_키ID.p8 --key-id "키ID" --issuer "발급자ID"
```

**`이 인증서로 서명하면 보안 타임스탬프가 안 붙는다`**
`codesign` 은 이 타임스탬프를 인증서 종류와 환경에 따라 알아서 붙이는데, man 페이지가
*"some but not all code signatures"* 라고 적어 둔 대로 보장이 아니다. 실제로
`Apple Development` 인증서로 서명하면 안 붙는다(`Signed Time` 만 남는다). 배포용
`Developer ID Application` 인증서를 쓰고 있는지, `timestamp.apple.com` 을 VPN·프록시가
막고 있지 않은지 확인한다.

**`error running bundle_dmg.sh`**
Finder 제어 권한이 없어 DMG 꾸미기 단계에서 죽은 것이다. 위 팝업을 허용하거나
`--plain-dmg` 로 그 단계를 건너뛴다.

**공증은 통과했는데 `spctl` 이 거부**
staple 부터 의심하되 단정하지는 말 것 — 스테이플이 없어도 인터넷이 되면 Gatekeeper 가
애플 서버에서 티켓을 찾아 통과시키므로, 거부의 원인은 서명·정책 쪽일 수도 있다.
`spctl` 출력의 사유 줄과, 스크립트가 어느 단계까지 갔는지를 같이 본다.
`codesign`·`spctl` 검사만으로는 완전하지 않으니, 최종 확인은 격리 속성이 붙은
실제 배포본(내려받은 그대로)을 열어 보는 것이다.

---

## 내보내기 (개발 29에서 깔았다)

배포 채널은 **GitHub Releases** 와 **Homebrew tap([dinggi5/homebrew-tap](https://github.com/dinggi5/homebrew-tap))**
두 곳뿐이다. 그 외의 경로로 도는 Kura 는 우리 것이 아니다 — `SECURITY.md` 에 그렇게 적어 뒀다.

새 버전을 낼 때:

```bash
# 1. 버전을 네 곳에서 올린다 (release.sh 사전 점검이 불일치를 잡는다)
#    tauri.conf.json / package.json / src-tauri/Cargo.toml / kura-mcp/Cargo.toml
# 2. 릴리스 노트를 쓴다 — docs/release-notes/v<버전>.md (아래 "릴리스 노트 파일")
# 3. 커밋 (작업 트리가 깨끗해야 사전 점검을 통과한다)
#    ⚠️ 새 릴리스 노트는 추적 안 되는 파일이라 -am 으로는 안 들어간다. add 를 먼저.
git add docs/release-notes/v0.1.2.md && git commit -am "v0.1.2"
# 4. main 에 올린다 — 태그가 가리키는 커밋이 브랜치에 있어야 한다
git checkout main && git merge --ff-only <그 커밋> && git push origin main
# 5. 빌드·검증·배포를 한 번에 (개발 33)
./scripts/release.sh --publish
```

`--publish` 는 빌드·서명·공증·검증이 **전부 통과한 뒤에만** 배포 단계로 넘어가고,
넘어가기 전에 무엇을 할지 보여주고 `yes` 를 받는다. 하는 일:

| 단계 | 하는 것 | 이미 돼 있으면 |
|---|---|---|
| 태그 | `git tag v<버전> <빌드한 커밋>` → 그 태그 하나만 푸시 | 건너뛴다 (원격 태그가 **다른** 커밋이면 멈춘다) |
| 릴리스 | `gh release create` 로 자산 5종(DMG · tar · sig · latest.json · .mcpb) 업로드 | 빠진 자산만 올린다 |
| 재검증 | 올라간 자산 **다섯 개를 다 다시 받아** 로컬 산출물과 바이트 대조. 업데이트 엔드포인트가 이 버전을 광고할 때까지 최대 5번 확인(끝내 다르면 멈춘다) | — |
| 캐스크 | tap 의 `Casks/kura.rb` 에 version·sha256 반영, `brew audit` 통과 후 커밋·푸시 | 커밋할 것 없음 (안 밀린 커밋이 남아 있으면 민다) |

배포 전에 먼저 막는 것들: `--no-notarize`/`--skip-tests`/`--allow-dirty`/`--universal` 과의 조합,
릴리스 노트 파일 없음, gh 미로그인, origin·tap 의 fetch/push URL 이 우리 리포가 아닌 경우,
tap 에 안 밀린 커밋, `tauri.conf.json` 의 업데이트 엔드포인트가 다른 리포를 가리키는 경우,
캐스크 버전이 **내려가는** 배포(옛 태그를 다시 빌드했을 때 — 원격 tap 기준으로 빌드 전에 본다).

자동화해도 **사람이 계속 하는 것 셋**: 버전 올리기 · 릴리스 노트 쓰기 · 키체인
「항상 허용」 누르기. 앞의 둘은 판단이고, 셋째는 헤드리스면 서명이 그냥 실패한다.

🔴 **릴리스가 만들어진 뒤에 실패하면 "그냥 다시 돌리기"가 안 된다.** 재실행은 새로 빌드하는데,
같은 커밋이라도 코드서명 타임스탬프·공증 티켓 때문에 **바이트가 달라진다**(재현 가능 빌드가
아니다 — 아래 "아직 안 한 것"). 그래서 재검증 단계에서 "올라간 자산이 방금 만든 것과 다르다"로
멈춘다. 그때 스크립트가 두 갈래를 명령까지 찍어 준다:

1. `./scripts/release.sh --publish --replace-assets` — 자산 다섯 개를 **같은 실행 안에서**
   이번 빌드로 덮어쓰고 그대로 진행한다. (그냥 `--publish` 로 다시 돌리면 또 새로 빌드해서
   또 같은 자리에서 막힌다 — 바이트가 매번 다르니 영원히 안 끝난다.)
2. 이미 올라간 것을 그대로 둔다 — 이 스크립트로 캐스크를 갱신하지 말고, 릴리스에서 받은
   DMG 의 해시로 손수 갱신한다.

⚠️ `--replace-assets` 는 `gh … --clobber` 로 **기존 자산을 지운 뒤** 올린다. 올리다 실패하면
그 자산은 사라지므로(릴리스 페이지 확인 필요), 복구 상황에서만 쓴다.

릴리스 **전에** 끊긴 경우(태그까지만 됐다든지)는 그냥 다시 돌리면 된다.

🔴 **이걸 GitHub Actions 로 옮기지 않는다.** Developer ID 인증서와 **업데이트 서명
개인키**를 CI 시크릿에 올려야 하는데, 그 키가 새면 이미 깔린 지갑에 임의 코드를
밀어넣을 수 있다. 키는 이 맥을 안 떠난다.

배포까지 안 하고 산출물만 만들려면 `--publish` 없이 돌린다. 그러면 예전처럼 태그·릴리스
명령을 **커밋 SHA 를 박아서** 찍어 주므로 손으로 이어서 할 수 있다. 어느 쪽이든
손으로 `git tag v0.1.2` 를 치지 말 것(아래 이유).

**태그를 빌드 전이 아니라 빌드 뒤에, 그것도 SHA 를 박아서 찍는 이유**: README 는
"`git checkout v0.1.0` 하면 배포본과 같은 소스"라고 약속한다. 이 약속이 깨지는 길이
셋인데, **셋 다 나온 DMG 는 서명·공증 검사를 멀쩡히 통과해서** 어디서도 안 걸린다.

| 깨지는 길 | 막는 것 |
|---|---|
| 태그를 먼저 찍고, 그 뒤 커밋이 얹힌 채로 빌드 | `release.sh` 가 태그≠HEAD 면 멈춘다 |
| 커밋 안 된 변경이 섞여 들어감(`--allow-dirty`) | 태그가 있는데 트리가 더러우면 멈춘다. 태그가 없어도 결과에 "태그도 릴리스도 하지 말 것" 경고 |
| 빌드한 커밋이 아닌 커밋에 나중에 태그를 찍음 | 결과 출력이 `git tag v0.1.1 <그 커밋>` 을 통째로 찍어 준다 |

세 번째가 제일 조용하다 — 빌드하고 며칠 뒤에 태그를 찍으면 그때의 HEAD 에 붙는다.

Release 노트에는 **sha256 을 반드시 적는다.** 받는 사람이 전송 중 손상을 확인하는 값이다.
(위조까지 걸러내는 건 서명·공증이지 이 해시가 아니다 — `README.md` 에 그렇게 구분해 뒀다.)

### 업데이트 자산 3종 (개발 31)

`release.sh` 가 찍어 주는 `gh release create` 명령에는 DMG 말고 **세 개가 더** 들어 있다.
셋 다 올려야 한다:

| 파일 | 없으면 |
|---|---|
| `Kura.app.tar.gz` | 설치가 404 로 죽는다 |
| `Kura.app.tar.gz.sig` | (참고용. 실제 검증에 쓰는 서명은 `latest.json` 안에 들어간다) |
| `latest.json` | **기존 사용자가 새 버전이 나온 걸 영영 모른다** |

앱이 물어보는 주소는 `releases/latest/download/latest.json` 하나다. 이건 **가장 최신
정식 릴리스**(프리릴리스 제외)를 가리키므로, 프리릴리스로 올리면 아무한테도 안 간다.

### 릴리스 노트 파일

`docs/release-notes/v<버전>.md` 를 만들어 두면 그 내용이 `latest.json` 의 `notes` 로 들어가고,
사용자의 **설치 승인 화면에 그대로 뜬다.** 없으면 릴리스 페이지를 보라는 일반 안내만 뜬다
(스크립트가 경고한다).

지갑에 새 코드를 넣을지 판단하는 유일한 근거라, 짧아도 무엇이 바뀌는지는 적는다.

**마크다운으로 렌더되지 않는다** — 설정 화면은 이 글을 그대로(`whitespace-pre-wrap`)
찍는다. `**굵게**` 는 별표가 그대로 보이므로 평문으로 쓴다. 문단은 한 줄로 길게 쓰고
빈 줄로 나눈다(카드 폭에 맞춰 알아서 접힌다 — 직접 접으면 폭이 좁을 때 이상하게 끊긴다).

같은 글이 **GitHub 릴리스 본문으로도 올라간다(개발 32)**. `release.sh` 가 노트 + sha256
을 합친 `release-body.md` 를 산출물 폴더에 만들고, 마지막에 찍어 주는 `gh release create`
명령이 `--notes-file` 로 그걸 가리킨다. 웹에서도 앱과 같은 글을 보게 하려는 것이다.

### 🔴 캐스크에 `auto_updates true` 를 넣는 시점

앱이 스스로 업데이트하게 됐으니 캐스크에도 그 사실을 적어야 한다. 그러면 `brew upgrade`
가 Kura 를 건드리지 않고, **캐스크의 `uninstall launchctl:` 이 안 돌아서 자동 시작이 안
지워진다**(개발 30 P2 의 근본 해결).

**다만 0.1.1 캐스크에 같이 넣으면 안 된다.** `auto_updates true` 가 있으면 brew 는
0.1.0 → 0.1.1 업그레이드 자체를 건너뛰는데, 0.1.0 에는 업데이트 기능이 없어서
그 사용자들은 어느 경로로도 새 버전을 못 받는다.

순서:

1. **0.1.1 캐스크**: 버전·sha256 만 갱신 (`auto_updates` 없이) → 기존 사용자가 brew 로 올라온다
2. **0.1.2 캐스크**부터: `auto_updates true` 를 추가 → 이후로는 인앱 업데이트가 담당

**이 순서는 개발 33 부터 `--publish` 안의 게이트다.** 0.1.1 이하를 내면서 캐스크에
`auto_updates` 가 있으면 멈추고, 0.1.2 이상이면 없을 때 자동으로 넣는다. 문서에만
적어 두면 "다음 배포 때 넣자"가 그대로 잊힌다 — 개발 30 의 "검증 안 되는 약속은
문서가 아니라 게이트로" 와 같은 자리다.

`auto_updates true` 를 넣은 뒤에도 캐스크의 버전·sha256 은 계속 갱신한다 —
새로 설치하는 사람은 캐스크로 받기 때문이다.

### ⚠️ `release.env` 는 dotenv 가 아니라 셸 코드다

`release.sh` 가 `source` 로 읽는다. 즉 값 안의 `$`, 백틱, `$(…)` 가 **확장된다** —
암호 관리자가 만든 암호에 그런 문자가 있으면 조용히 다른 문자열이 되거나, 최악에는
그 자리에서 명령이 돈다. 값은 **작은따옴표**로 감쌀 것:

```bash
TAURI_SIGNING_PRIVATE_KEY_PASSWORD='p$w0rd`with$(weird)chars'
```

(작은따옴표 안에 작은따옴표가 있으면 `'\''` 로 끊어 넣는다.)

### 🔴 Cask 해시를 갱신하기 전에

Homebrew Cask 의 `sha256` 은 "이 파일을 설치해도 된다"는 **우리 쪽 보증**이다. 해시만
바꿔 올리면 잘못된 인증서로 서명된 앱도, 서명이 깨진 앱도 그대로 배포된다. 그래서
**올릴 파일 자체에** 아래를 돌려 통과한 뒤에만 갱신한다:

```bash
DMG=<올릴.dmg>
shasum -a 256 "$DMG"                       # Cask 에 넣을 값
spctl -a -t open --context context:primary-signature -vv "$DMG"
#   → accepted / source=Notarized Developer ID

MP=$(hdiutil attach "$DMG" -nobrowse -readonly | grep -o '/Volumes/.*')

# 서명이 온전한지 — 실제 검증은 이 줄이 한다 (아래 -dv 는 보여주기만 한다)
codesign --verify --deep --strict --verbose=2 "$MP/Kura.app"
#   → satisfies its Designated Requirement

# 누가 서명했는지·어떤 조건으로 서명됐는지
codesign -dv --verbose=2 "$MP/Kura.app" 2>&1 | grep -E "TeamIdentifier|flags|Timestamp="
#   → TeamIdentifier=74ZAMXKVXN / flags=0x10000(runtime) / Timestamp= 있음

spctl -a -vv "$MP/Kura.app"                # 앱 자체에 대한 Gatekeeper 판정
stapler validate "$MP/Kura.app"            # 공증 티켓이 파일에 박혀 있는지
hdiutil detach "$MP" -quiet
```

⚠️ **`codesign -dv` 는 검증이 아니라 출력이다.** 서명 정보를 보여줄 뿐, 파일이 서명 이후
변조됐는지는 안 본다. 그걸 보는 건 `--verify --deep --strict` 다. 둘을 섞어 쓰면
"확인했다"는 착각만 남는다.

`stapler validate` 도 **티켓이 박혀 있는지**를 볼 뿐, 오프라인 맥에서 실제로 열리는지를
시험하지는 않는다(그건 아래 "아직 안 한 것"에 남아 있다).

`release.sh` 가 빌드 직후 같은 검증을 하지만, **Cask 에 올리는 파일은 Release 에서 다시
받은 파일**이라 한 번 더 본다. 업로드가 깨졌는지도 여기서 걸린다.
`--publish` 는 이 중 **해시 대조를 자동으로 한다** — 릴리스에서 DMG 를 다시 받아
로컬 빌드본과 대조하고, 다르면 캐스크를 건드리지 않고 멈춘다. 서명·공증 쪽 확인까지
직접 보고 싶으면 위 명령들을 그대로 돌리면 된다(빌드 직후 스크립트가 같은 검사를 했다).

Cask 에서 조심할 것 두 가지는 파일 자체에 주석으로 박아 뒀다 — `launchctl` 라벨이 번들
ID 가 아니라 `Kura` 라는 것과, `zap` 에 `~/.jigap` 를 **의도적으로 넣지 않았다**는 것.

## 아직 안 한 것

- **재현 가능 빌드** — 태그를 찍고 `npm ci` 로 빌드하면 소스는 같지만 바이트 단위로는 다르다
- **다른 맥에서 실제로 열어보기** — 같은 맥에서 quarantine 을 붙여 재현한 판정까지가 한계
- **`--universal`** — x86_64 타깃 미설치라 사전 점검에서 멈춘다
- **DMG 배경 디자인** — 지금은 Tauri 기본 레이아웃
