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

1. **사전 점검** — 인증서·자격증명·버전 일치(`tauri.conf.json`/`package.json`/`Cargo.toml`)·커밋 안 된 변경 확인
2. **시험 서명** — 작은 파일을 실제로 한 번 서명해서 **보안 타임스탬프**가 붙는지 확인.
   이게 없으면 애플이 공증을 거부하는데, 빌드가 다 끝난 뒤에 알면 시간이 아깝다
3. **테스트** — `src-tauri` + `kura-mcp` 의 러스트 테스트와 타입 검사
4. **빌드·서명** — Tauri 가 하드닝 런타임으로 앱을 서명하고, 애플에 올려 공증받고, 티켓을 앱에 박는다(staple)
5. **DMG 공증** — Tauri 는 앱까지만 공증하므로 DMG 는 스크립트가 따로 올려 티켓을 박는다
6. **검증** — `codesign` / `stapler` / `spctl` 로 실제 Gatekeeper 판정을 확인
7. **결과** — DMG 경로와 `sha256`(Homebrew Cask 에 넣을 값) 출력

공증은 애플 서버 응답을 기다리므로 **처음엔 5~15분** 걸릴 수 있다. 그동안 맥이
잠들지 않게 두는 게 좋다.

### 옵션

| 옵션 | 언제 |
|---|---|
| `--universal` | 인텔 맥까지 지원. 먼저 `rustup target add x86_64-apple-darwin` 필요. 기본은 Apple Silicon 전용(주 사용자층과 일치, 용량 절반) |
| `--no-notarize` | 자격증명 없이 서명 경로만 점검. **결과물을 배포하면 안 된다** |
| `--plain-dmg` | Finder 제어 권한을 못 주는 환경(원격 접속·CI)에서. DMG 안 아이콘 배치를 건너뛴다 — 설치 동작은 같고 모양만 밋밋해진다 |
| `--skip-tests` | 테스트 건너뛰기. 급할 때만 |
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
# 1. 버전을 세 곳에서 올린다 (release.sh 사전 점검이 불일치를 잡는다)
#    tauri.conf.json / package.json / src-tauri/Cargo.toml
# 2. 커밋 (작업 트리가 깨끗해야 사전 점검을 통과한다)
git commit -am "v0.1.1"
# 3. 서명·공증 빌드
./scripts/release.sh
# 4~5. 태그와 릴리스 — release.sh 가 마지막에 **커밋 SHA 를 박아서** 명령을 찍어 준다
#      (git tag / git push origin <태그> / gh release create). 손으로 `git tag v0.1.1` 을
#      치지 말 것(아래 이유).
#      ⚠️ 빌드한 커밋이 origin/main 에 아직 없으면 스크립트가 경고를 낸다. 그때는
#      `git push origin main` 을 **먼저** 하고 나서 태그를 민다 — 태그만 밀면 릴리스는
#      생기지만 그 커밋이 브랜치 어디에도 없다.
```

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

Cask 에서 조심할 것 두 가지는 파일 자체에 주석으로 박아 뒀다 — `launchctl` 라벨이 번들
ID 가 아니라 `Kura` 라는 것과, `zap` 에 `~/.jigap` 를 **의도적으로 넣지 않았다**는 것.

## 아직 안 한 것

- **자동 업데이트(Tauri updater)** — DMG 로 받은 사용자는 아직 수동으로 새 버전을 받아야 한다.
  README "업데이트" 절에 Watch → Releases 를 안내해 뒀지만 임시방편이다
- **재현 가능 빌드** — 태그를 찍고 `npm ci` 로 빌드하면 소스는 같지만 바이트 단위로는 다르다
- **다른 맥에서 실제로 열어보기** — 같은 맥에서 quarantine 을 붙여 재현한 판정까지가 한계
- **`--universal`** — x86_64 타깃 미설치라 사전 점검에서 멈춘다
- **DMG 배경 디자인** — 지금은 Tauri 기본 레이아웃
