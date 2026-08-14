#!/usr/bin/env bash
#
# Kura 배포본 만들기 — 서명(Developer ID) → 공증(notarization) → 스테이플 → 검증
#
#   ./scripts/release.sh                 배포용 DMG 한 개 만들기 (Apple Silicon)
#   ./scripts/release.sh --universal     인텔 맥까지 도는 유니버설 빌드
#   ./scripts/release.sh --no-notarize   서명만 (공증 자격증명 없이 파이프라인 점검용)
#   ./scripts/release.sh --plain-dmg     DMG 안 아이콘 배치 생략 (Finder 권한 없는 환경)
#   ./scripts/release.sh --skip-tests    테스트 건너뛰기 (권장하지 않음)
#
# 1회 설정과 자격증명 만드는 법은 docs/RELEASE.md 를 볼 것.
#
# 공증은 애플 서버에 빌드 결과를 올려 악성코드 스캔을 받는 절차다. 통과하면
# 애플이 "티켓"을 발급하고, staple 이 그 티켓을 DMG 안에 박아 넣는다. 그래야
# 처음 받은 맥이 인터넷 없이도 Gatekeeper 경고 없이 앱을 연다.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# ── 출력 도우미 ──────────────────────────────────────────────────────────────
step() { printf '\n\033[1;34m▸ %s\033[0m\n' "$*"; }
info() { printf '  %s\n' "$*"; }
warn() { printf '\033[1;33m  ! %s\033[0m\n' "$*"; }
die()  { printf '\n\033[1;31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

# ── 인자 ────────────────────────────────────────────────────────────────────
UNIVERSAL=0
NOTARIZE=1
RUN_TESTS=1
ALLOW_DIRTY=0
PLAIN_DMG=0
for arg in "$@"; do
  case "$arg" in
    --universal)   UNIVERSAL=1 ;;
    --no-notarize) NOTARIZE=0 ;;
    --skip-tests)  RUN_TESTS=0 ;;
    --allow-dirty) ALLOW_DIRTY=1 ;;
    --plain-dmg)   PLAIN_DMG=1 ;;
    -h|--help)     sed -n '3,15p' "$0"; exit 0 ;;
    *)             die "모르는 옵션: $arg  (--help 로 사용법 확인)" ;;
  esac
done

# ── 0. 자격증명 불러오기 ────────────────────────────────────────────────────
# scripts/release.env 는 .gitignore 로 커밋에서 제외된다. 없으면 이미 export 된
# 환경변수를 그대로 쓴다(CI 용).
ENV_FILE="$REPO_ROOT/scripts/release.env"
if [[ -f "$ENV_FILE" ]]; then
  perms="$(stat -f '%Lp' "$ENV_FILE")"
  # 정식 배포(공증)에서는 경고로 넘기지 않는다. 이 파일에는 앱 전용 암호나 API 키 위치가
  # 들어 있고, 그게 새면 "내 이름으로 서명된 악성 앱"이 가능해진다 — 이 지갑에서 제일
  # 아픈 시나리오다. 점검용(--no-notarize)에서는 경고로 둔다.
  # 600 만 통과시키면 400(읽기 전용)처럼 **더 엄격한** 설정까지 막는다. 실제 조건은
  # "그룹·다른 사용자가 못 본다" 이므로 하위 6비트가 0인지로 검사한다.
  if (( 8#$perms & 077 )); then
    if [[ $NOTARIZE -eq 1 ]]; then
      die "$ENV_FILE 권한이 $perms 다. 다른 로컬 계정이 읽을 수 있는 자격증명으로는 정식 배포를 만들지 않는다:  chmod 600 $ENV_FILE"
    fi
    warn "$ENV_FILE 권한이 $perms 다. chmod 600 을 권함"
  fi
  . "$ENV_FILE"
  info "자격증명: scripts/release.env"
else
  info "scripts/release.env 없음 — 환경변수를 그대로 사용"
fi

# 읽은 값은 CRED_* 셸 변수로 옮기고 환경에서는 지운다. 이유 두 가지:
#  1) 호출한 셸에 같은 이름이 이미 export 돼 있으면, source 로 새 값을 대입해도
#     export 속성이 남는다 → cargo test·npx tsc·의존성 빌드 스크립트까지 앱 암호를
#     물려받는다. 대입만으로는 격리가 안 되고 환경에서 빼야 한다.
#  2) 같은 이유로 --no-notarize 인데 환경에 자격증명이 남아 있으면 Tauri 가 멋대로
#     공증을 시도한다. 여기서 지워 두면 그 경로가 애초에 없다.
# 필요한 곳(빌드 한 번, notarytool 호출)에만 아래에서 다시 실어 준다.
CRED_IDENTITY="${APPLE_SIGNING_IDENTITY:-}"
CRED_API_KEY="${APPLE_API_KEY:-}"
CRED_API_ISSUER="${APPLE_API_ISSUER:-}"
CRED_API_KEY_PATH="${APPLE_API_KEY_PATH:-}"
CRED_APPLE_ID="${APPLE_ID:-}"
CRED_PASSWORD="${APPLE_PASSWORD:-}"
CRED_TEAM_ID="${APPLE_TEAM_ID:-}"
unset APPLE_SIGNING_IDENTITY APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH \
      APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID

# 실패한 채로 끝나면 배포하면 안 되는 DMG 가 정상 릴리스와 같은 이름·자리에 남는다.
# 나중에(또는 다른 사람이) 그걸 완성본으로 착각하지 않게 이름을 바꿔 둔다.
# 특정 경로 하나가 아니라 출력 폴더를 훑는 이유: 빌드가 DMG 를 만든 *뒤* 죽으면
# DMG_PATH 를 잡기 전이라, 경로 변수에 기대면 정작 위험한 경우를 놓친다.
PROBE_DIR=""
NOTARY_ERR=""
BUNDLE_DIR=""
OWNS_OUTPUT=0
RELEASE_OK=0
cleanup() {
  if [[ -n "$PROBE_DIR" ]]; then rm -rf "$PROBE_DIR"; fi
  if [[ -n "$NOTARY_ERR" ]]; then rm -f "$NOTARY_ERR"; fi
  # OWNS_OUTPUT 이 서야 폴더를 건드린다. 그 전에 죽으면 폴더 안은 지난 실행이 남긴
  # 것들이라, 멀쩡히 성공했던 배포본을 실패본으로 잘못 표시하게 된다.
  if [[ $RELEASE_OK -eq 1 || $OWNS_OUTPUT -eq 0 || ! -d "$BUNDLE_DIR/dmg" ]]; then return 0; fi
  local f dest
  for f in "$BUNDLE_DIR"/dmg/*.dmg; do
    [[ -f "$f" ]] || continue
    # rw.*.dmg 는 bundle_dmg.sh 가 남긴 중간 이미지고, 이미 표시해 둔 건 건드리지 않는다.
    case "${f##*/}" in rw.*|*.FAILED.dmg) continue ;; esac
    dest="${f%.dmg}.FAILED.dmg"
    # -n 으로 앞선 실패본을 덮어쓰지 않는다. 다만 BSD mv 는 대상이 있어도 0 을 주므로
    # 옮겨졌는지는 원본이 사라졌는지로 판단한다. 어느 쪽이든 경고는 반드시 남긴다.
    if mv -n "$f" "$dest" 2>/dev/null && [[ ! -f "$f" ]]; then
      warn "실패한 산출물을 $dest 로 옮겼다 — 배포하면 안 된다"
    else
      warn "실패한 산출물이 $f 에 그대로 있다 — 배포하면 안 된다"
    fi
  done
}
trap cleanup EXIT

# ── 1. 사전 점검 ────────────────────────────────────────────────────────────
step "사전 점검"

[[ "$(uname -s)" == "Darwin" ]] || die "macOS 에서만 돈다"

# 기본 빌드는 호스트 아키텍처 그대로 나온다. README 와 Homebrew 캐스크는 aarch64/arm64 만
# 약속하므로(캐스크 depends_on arch: :arm64), 인텔 맥에서 낸 산출물은 이름만 맞고 아무 데서도
# 안 걸린 채 배포될 수 있다. 인텔까지 덮으려면 --universal 이 따로 있다.
if [[ $UNIVERSAL -eq 0 && "$(uname -m)" != "arm64" ]]; then
  die "여기는 $(uname -m) 다. 배포본은 Apple Silicon(arm64) 에서 빌드해야 한다 (인텔까지 덮으려면 --universal)"
fi
command -v xcrun >/dev/null || die "xcrun 이 없다. Xcode 또는 Command Line Tools 설치 필요"
xcrun notarytool --version >/dev/null 2>&1 || die "notarytool 이 없다. Xcode 14 이상 필요"

# 서명 인증서 — 이름 자체는 커밋하지 않는다(실명이 들어간다). 환경변수로만 받는다.
[[ -n "$CRED_IDENTITY" ]] || die \
  "APPLE_SIGNING_IDENTITY 가 비어 있다. scripts/release.env 에 Developer ID Application 인증서 이름을 넣을 것 (docs/RELEASE.md 참고)"

IDENTITIES="$(security find-identity -v -p codesigning)"
grep -Fq "$CRED_IDENTITY" <<<"$IDENTITIES" || die \
  "키체인에서 인증서를 못 찾음: $CRED_IDENTITY"$'\n'"현재 쓸 수 있는 인증서:"$'\n'"$IDENTITIES"
[[ "$CRED_IDENTITY" == "Developer ID Application:"* ]] || die \
  "배포용 서명에는 'Developer ID Application' 인증서가 필요하다. 'Apple Development' 는 내 맥에서 돌려보는 용도라 남에게 주면 Gatekeeper 가 막는다"
info "서명 인증서 확인됨"

# 공증을 받으려면 서명에 애플 타임스탬프 서버가 찍은 '보안 타임스탬프'가 있어야 한다.
# codesign 은 이걸 인증서 종류·환경에 따라 알아서 붙이는데, man 페이지가 "system-specific
# default behavior ... may result in some but not all code signatures being timestamped"
# 라고 못 박은 대로 보장이 아니다. 실측: Apple Development 인증서로 서명하면 안 붙는다
# (`Signed Time=` 만 나오고 `Timestamp=` 가 없다).
# 몇 분짜리 빌드를 다 돌린 뒤 공증에서 거부당하는 대신 여기서 1초 만에 확인한다.
if [[ $NOTARIZE -eq 1 ]]; then
  PROBE_DIR="$(mktemp -d)"
  cp /usr/bin/true "$PROBE_DIR/probe"
  # codesign 이 남긴 말을 버리면 안 된다 — 키체인 잠김·개인키 없음·타임스탬프 서버
  # 불통이 전부 여기서 갈리는데, 이유 없이 "실패했다"만 나오면 손쓸 데가 없다.
  if ! PROBE_ERR="$(codesign --force --options runtime -s "$CRED_IDENTITY" "$PROBE_DIR/probe" 2>&1)"; then
    warn "codesign 이 남긴 말:"
    printf '%s\n' "$PROBE_ERR" | sed 's/^/  /'
    die "시험 서명이 실패했다. 키체인이 잠겨 있거나(로그인 키체인 잠금 해제), 인증서에 개인키가 없거나, 키체인 접근 허용 팝업을 놓쳤을 수 있다"
  fi
  grep -q '^Timestamp=' <<<"$(codesign -dvvv "$PROBE_DIR/probe" 2>&1)" || die \
"이 인증서로 서명하면 보안 타임스탬프가 안 붙는다 (Signed Time 만 있고 Timestamp 가 없음).
   그대로 올리면 애플이 공증을 거부한다.
   - timestamp.apple.com 에 닿는지 (VPN·프록시·방화벽) 확인할 것
   - 'Apple Development' 인증서로는 원래 안 붙는다. 'Developer ID Application' 인지 확인할 것"
  info "보안 타임스탬프 확인됨"
fi

# DMG 안 아이콘 배치는 bundle_dmg.sh 가 Finder 에게 AppleScript 로 시킨다. 그 권한이
# 없으면 빌드를 다 마친 뒤에야 "error running bundle_dmg.sh" 라는 알아보기 힘든 에러로
# 죽는다(실측). 여기서 미리 물어본다 — 처음이면 허용 팝업이 뜬다.
#
# CI 가 이미 환경에 있으면 Tauri 는 어차피 그 단계를 건너뛴다. 그 상태에서 Finder 권한을
# 요구하면 실제로는 만들 수 있는 빌드를 거짓으로 막게 되고, 반대로 "권한 확인됨"을
# 찍어 놓고 정작 꾸미기는 생략되는 엇갈림도 생긴다 → --plain-dmg 와 같이 취급한다.
if [[ $PLAIN_DMG -eq 0 && -n "${CI:-}" ]]; then
  PLAIN_DMG=1
  info "CI 환경변수가 이미 설정돼 있다 — --plain-dmg 와 같이 동작한다"
fi

if [[ $PLAIN_DMG -eq 0 ]]; then
  osascript -e 'tell application "Finder" to return name of startup disk' >/dev/null 2>&1 || die \
"Finder 제어 권한이 없어 DMG 만드는 마지막 단계에서 실패한다.
   - 방금 뜬 '제어하려고 합니다' 팝업이 있으면 '승인' 을 누르고 다시 실행할 것
   - 이미 거부했다면: 시스템 설정 → 개인정보 보호 및 보안 → 자동화 → (지금 쓰는 터미널 앱) → Finder 켜기
   - 창 배치를 포기하고 그냥 만들려면: ./scripts/release.sh --plain-dmg"
  info "Finder 제어 권한 확인됨"
else
  warn "DMG 안 아이콘 배치를 건너뛴다 (설치 동작은 같고 모양만 밋밋해진다)"
fi

# 공증 자격증명 — App Store Connect API 키(권장) 또는 앱 암호, 둘 중 하나 완비.
NOTARY_AUTH=()
if [[ $NOTARIZE -eq 1 ]]; then
  if [[ -n "$CRED_API_KEY" || -n "$CRED_API_ISSUER" || -n "$CRED_API_KEY_PATH" ]]; then
    [[ -n "$CRED_API_KEY" && -n "$CRED_API_ISSUER" && -n "$CRED_API_KEY_PATH" ]] || die \
      "API 키 방식은 APPLE_API_KEY(키 ID)·APPLE_API_ISSUER·APPLE_API_KEY_PATH 세 개가 다 있어야 한다"
    [[ -r "$CRED_API_KEY_PATH" ]] || die "API 키 파일을 못 읽음: $CRED_API_KEY_PATH"
    NOTARY_AUTH=(--key "$CRED_API_KEY_PATH" --key-id "$CRED_API_KEY" --issuer "$CRED_API_ISSUER")
    info "공증 자격증명: App Store Connect API 키"
  elif [[ -n "$CRED_APPLE_ID" || -n "$CRED_PASSWORD" || -n "$CRED_TEAM_ID" ]]; then
    [[ -n "$CRED_APPLE_ID" && -n "$CRED_PASSWORD" && -n "$CRED_TEAM_ID" ]] || die \
      "앱 암호 방식은 APPLE_ID·APPLE_PASSWORD·APPLE_TEAM_ID 세 개가 다 있어야 한다"
    NOTARY_AUTH=(--apple-id "$CRED_APPLE_ID" --password "$CRED_PASSWORD" --team-id "$CRED_TEAM_ID")
    info "공증 자격증명: 앱 전용 암호"
    warn "앱 암호는 notarytool 실행 중 프로세스 인자로 잠깐 노출된다. 여러 사람이 쓰는 맥이면 API 키 방식을 쓸 것"
  else
    die "공증 자격증명이 없다. docs/RELEASE.md 를 보고 scripts/release.env 를 채울 것 (점검만 하려면 --no-notarize)"
  fi
else
  warn "--no-notarize: 서명만 한다. 이 결과물은 남에게 주면 Gatekeeper 가 막는다"
fi

# 버전이 세 파일에서 갈리면 배포본과 업데이트 판단이 어긋난다.
VERSION_CONF="$(python3 -c 'import json;print(json.load(open("src-tauri/tauri.conf.json"))["version"])')"
VERSION_PKG="$(python3 -c 'import json;print(json.load(open("package.json"))["version"])')"
VERSION_CARGO="$(awk '/^\[package\]/{p=1;next}/^\[/{p=0}p&&/^version *=/{gsub(/[" ]/,"");sub(/^version=/,"");print;exit}' src-tauri/Cargo.toml)"
[[ "$VERSION_CONF" == "$VERSION_PKG" && "$VERSION_CONF" == "$VERSION_CARGO" ]] || die \
  "버전 불일치 — tauri.conf.json=$VERSION_CONF / package.json=$VERSION_PKG / Cargo.toml=$VERSION_CARGO"
info "버전 $VERSION_CONF (세 파일 일치)"

# 더러운 작업 트리에서 낸 배포본은 어떤 소스로 만든 건지 나중에 되짚을 수 없다.
IS_DIRTY=0
if [[ -n "$(git status --porcelain)" ]]; then
  if [[ $ALLOW_DIRTY -eq 1 ]]; then
    IS_DIRTY=1
    warn "커밋 안 된 변경이 있는 채로 빌드한다 (--allow-dirty)"
  else
    git status --short
    die "커밋 안 된 변경이 있다. 커밋하고 다시 돌리거나 --allow-dirty 를 붙일 것"
  fi
fi
GIT_SHA="$(git rev-parse --short HEAD)"
HEAD_BEFORE="$(git rev-parse HEAD)"   # 빌드가 끝난 뒤 같은 커밋인지 대조할 기준
info "커밋 $GIT_SHA"

# 태그 v<버전> 이 이 배포본의 소스와 맞는지.
#
# README 는 "git checkout v0.1.0 으로 배포본과 같은 소스를 받는다"고 약속한다.
# 이 약속이 깨지는 길이 셋인데, 셋 다 나온 DMG 는 서명·공증 검사를 멀쩡히 통과하므로
# 어디서도 안 걸린다 → 게이트로 막는다.
#
#  ① 태그를 먼저 찍고 그 뒤 커밋이 얹힌 채로 빌드    → 아래 태그≠HEAD 검사
#  ② 커밋 안 된 변경이 섞여 들어감(--allow-dirty)     → 아래 dirty 검사
#  ③ 빌드한 커밋이 아닌 다른 커밋에 나중에 태그를 찍음 → 마지막에 SHA 를 박은
#     `git tag` 명령을 그대로 찍어 준다(사람이 커밋을 다시 고를 일이 없게)
#
# 태그가 아직 없는 건 정상이다 — 권장 순서가 빌드 → 검증 → 그 커밋에 태그다.
VERSION_TAG="v$VERSION_CONF"
if git rev-parse -q --verify "refs/tags/$VERSION_TAG" >/dev/null; then
  TAG_SHA="$(git rev-parse "$VERSION_TAG^{commit}")"
  HEAD_SHA="$(git rev-parse HEAD)"
  if [[ "$TAG_SHA" != "$HEAD_SHA" ]]; then
    die "태그 $VERSION_TAG 가 HEAD 가 아닌 커밋($(git rev-parse --short "$TAG_SHA"))을 가리킨다.
     이대로 배포하면 태그를 받아 빌드한 사람이 다른 소스를 얻는다.
     그 커밋을 체크아웃해 빌드하거나, 태그를 이 커밋으로 옮길 것."
  fi
  # 태그가 HEAD 를 가리켜도 트리가 더러우면 DMG 안에는 태그에 없는 코드가 들어간다.
  # "태그 = HEAD" 만 보고 통과시키면 게이트가 오히려 거짓 보증을 서는 꼴이 된다.
  [[ $IS_DIRTY -eq 0 ]] || die \
    "태그 $VERSION_TAG 가 있는데 커밋 안 된 변경이 있다. --allow-dirty 로는 정식 배포를 만들 수 없다.
     이 DMG 에는 태그에 없는 코드가 들어가는데 서명·공증은 그대로 통과한다."
  info "태그 $VERSION_TAG = HEAD (트리 깨끗)"
fi

# ── 2. 테스트 ───────────────────────────────────────────────────────────────
if [[ $RUN_TESTS -eq 1 ]]; then
  step "테스트"
  (cd src-tauri && cargo test --quiet)
  (cd kura-mcp && cargo test --quiet)
  npx tsc --noEmit
  info "통과"
else
  warn "--skip-tests: 테스트를 건너뛴다"
fi

# ── 3. 빌드 (서명 + 앱 공증 + 앱 스테이플까지 Tauri 가 처리) ────────────────
step "빌드 · 서명"

BUILD_ARGS=(--bundles app,dmg)
BUNDLE_DIR="src-tauri/target/release/bundle"
if [[ $UNIVERSAL -eq 1 ]]; then
  # 인텔 호스트에는 x86_64 타깃이 기본으로 있으니 그것만 보면 통과시켜 놓고, 정작 없는
  # aarch64 때문에 빌드 한참 뒤에 죽는다. 유니버설은 두 쪽이 다 있어야 한다.
  INSTALLED_TARGETS="$(rustup target list --installed)"
  for t in x86_64-apple-darwin aarch64-apple-darwin; do
    grep -q "^$t\$" <<<"$INSTALLED_TARGETS" || die \
      "유니버설 빌드에는 $t 타깃이 필요하다:  rustup target add $t"
  done
  BUILD_ARGS+=(--target universal-apple-darwin)
  BUNDLE_DIR="src-tauri/target/universal-apple-darwin/release/bundle"
fi

# 자격증명은 이 빌드 한 번에만 넘긴다(위 0번 주석 참고). 사전 점검에서 한 세트가
# 온전한지 이미 확인했으므로 여기서는 있는 쪽을 그대로 싣는다.
BUILD_ENV=("APPLE_SIGNING_IDENTITY=$CRED_IDENTITY")
if [[ $NOTARIZE -eq 1 ]]; then
  if [[ -n "$CRED_API_KEY" ]]; then
    BUILD_ENV+=("APPLE_API_KEY=$CRED_API_KEY" "APPLE_API_ISSUER=$CRED_API_ISSUER" "APPLE_API_KEY_PATH=$CRED_API_KEY_PATH")
  else
    BUILD_ENV+=("APPLE_ID=$CRED_APPLE_ID" "APPLE_PASSWORD=$CRED_PASSWORD" "APPLE_TEAM_ID=$CRED_TEAM_ID")
  fi
fi
if [[ $PLAIN_DMG -eq 1 ]]; then
  BUILD_ENV+=("CI=true")   # bundle_dmg.sh 의 Finder 단계를 건너뛰게 하는 Tauri 쪽 스위치
fi

rm -rf "$BUNDLE_DIR/macos" "$BUNDLE_DIR/dmg"
OWNS_OUTPUT=1   # 폴더를 비웠으니 이제부터 그 안에 있는 건 전부 이번 실행이 만든 것
env "${BUILD_ENV[@]}" npm run tauri build -- "${BUILD_ARGS[@]}"

# 빌드 전에 한 번 본 것으로는 부족하다. 빌드·공증은 몇 분씩 걸리고 그 사이에 편집기가
# 저장을 하거나 cargo 가 lockfile 을 갱신할 수 있는데, 그러면 "태그 = 배포본 소스" 라는
# 약속이 깨진 채로 서명·공증은 멀쩡히 통과한다(그 조합이 이 게이트의 존재 이유였다).
#
# 트리가 더러워지는 것만 보면 부족하다 — 빌드 도중 다른 창에서 커밋하거나 체크아웃하면
# 트리는 깨끗한데 HEAD 가 옮겨간다. 그러면 마지막에 찍어 주는 태그 명령은 빌드 전에
# 잡아 둔 GIT_SHA 를 가리키므로, 태그와 산출물이 조용히 갈린다.
# 명령 치환은 조건문 안에 두면 실패해도 빈 문자열로 통과하니 상태를 따로 받는다.
STATUS_AFTER="$(git status --porcelain)" || die "빌드 후 git status 가 실패했다. 트리 상태를 확인하지 못했으므로 이 산출물은 보증할 수 없다"
HEAD_AFTER="$(git rev-parse HEAD)" || die "빌드 후 HEAD 를 읽지 못했다"
[[ "$HEAD_AFTER" == "$HEAD_BEFORE" ]] || die \
  "빌드 도중 HEAD 가 $GIT_SHA → $(git rev-parse --short HEAD) 로 바뀌었다. 이 DMG 가 어느 커밋에서 나온 건지 보증할 수 없다"
if [[ $IS_DIRTY -eq 0 && -n "$STATUS_AFTER" ]]; then
  git status --short
  die "빌드 도중에 작업 트리가 바뀌었다. 이 DMG 가 어느 소스에서 나온 건지 보증할 수 없다"
fi

APP_PATH="$BUNDLE_DIR/macos/Kura.app"
[[ -d "$APP_PATH" ]] || die "앱이 안 나왔다: $APP_PATH"
DMG_PATH="$(find "$BUNDLE_DIR/dmg" -maxdepth 1 -name '*.dmg' -print -quit 2>/dev/null || true)"
[[ -n "$DMG_PATH" && -f "$DMG_PATH" ]] || die "DMG 가 안 나왔다: $BUNDLE_DIR/dmg"
info "앱 $APP_PATH"
info "DMG $DMG_PATH"

# ── 4. 앱 검증 ──────────────────────────────────────────────────────────────
step "앱 검증"

codesign --verify --deep --strict --verbose=2 "$APP_PATH" 2>&1 | sed 's/^/  /' \
  || die "코드 서명 검증 실패 — 위 메시지 참고"

# 서명 정보는 한 번만 읽어 변수에 담는다. `codesign … | grep -q` 로 쓰면 grep 이 첫 매치에
# 파이프를 닫아 codesign 이 SIGPIPE 로 죽고, pipefail 때문에 멀쩡한 서명이 실패로 둔갑한다.
APP_SIG="$(codesign -dvvv "$APP_PATH" 2>&1)"

# 하드닝 런타임이 안 켜지면 공증이 거부된다. CodeDirectory flags 에 runtime 이 있어야 함.
grep -q 'flags=.*runtime' <<<"$APP_SIG" || die \
  "하드닝 런타임이 안 켜졌다 (tauri.conf.json 의 bundle.macOS.hardenedRuntime 확인)"
info "하드닝 런타임 켜짐"

# 여기까지의 검사는 "유효한 Developer ID 로 서명됐고 공증됐다"까지만 본다. 그런데 README
# 와 SECURITY.md 는 받는 사람에게 **특정 값**을 확인하라고 시킨다(팀 74ZAMXKVXN, 번들 ID
# com.dinggi5.kura). 다른 팀 인증서와 그 팀 자격증명이 환경에 들어와 있으면 지금 검사는
# 전부 통과하는데 사용자 쪽 확인은 실패한다 → 문서가 약속하는 값을 여기서 그대로 검사한다.
# (개발 26 "검증 안 되는 약속은 문서가 아니라 게이트로 둔다"와 같은 계열)
EXPECT_TEAM_ID="74ZAMXKVXN"
EXPECT_BUNDLE_ID="com.dinggi5.kura"
APP_TEAM="$(sed -n 's/^TeamIdentifier=//p' <<<"$APP_SIG" | head -1)"
APP_IDENT="$(sed -n 's/^Identifier=//p' <<<"$APP_SIG" | head -1)"
[[ "$APP_TEAM" == "$EXPECT_TEAM_ID" ]] || die \
  "팀 ID 가 다르다: $APP_TEAM (기대 $EXPECT_TEAM_ID). README·SECURITY.md 가 사용자에게 알려주는 값과 어긋난다"
[[ "$APP_IDENT" == "$EXPECT_BUNDLE_ID" ]] || die \
  "번들 ID 가 다르다: $APP_IDENT (기대 $EXPECT_BUNDLE_ID)"

# 앱 안의 버전이 파일명·태그·캐스크와 갈리면, 사용자가 설정 → 정보에서 보는 값이 배포 번호와
# 어긋난다. 파일명은 Tauri 가 붙여 주지만 Info.plist 는 별개다.
APP_EXEC="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$APP_PATH/Contents/Info.plist" 2>/dev/null || true)"
APP_VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP_PATH/Contents/Info.plist" 2>/dev/null || true)"
[[ "$APP_VERSION" == "$VERSION_CONF" ]] || die \
  "앱 Info.plist 버전이 $APP_VERSION 인데 빌드 버전은 $VERSION_CONF 다"
[[ -n "$APP_EXEC" && -f "$APP_PATH/Contents/MacOS/$APP_EXEC" ]] || die \
  "앱 실행 파일을 못 찾음 (CFBundleExecutable=$APP_EXEC)"

# 캐스크가 depends_on arch: :arm64 로 약속하는 값. 유니버설이면 두 아키텍처가 다 있어야 한다.
APP_ARCHS="$(lipo -archs "$APP_PATH/Contents/MacOS/$APP_EXEC" 2>/dev/null || true)"
if [[ $UNIVERSAL -eq 1 ]]; then
  [[ "$APP_ARCHS" == *arm64* && "$APP_ARCHS" == *x86_64* ]] || die \
    "유니버설 빌드인데 아키텍처가 '$APP_ARCHS' 다 (arm64 와 x86_64 둘 다 필요)"
else
  [[ "$APP_ARCHS" == "arm64" ]] || die \
    "기본 빌드는 arm64 단일이어야 하는데 '$APP_ARCHS' 다"
fi
info "신원 확인: 팀 $APP_TEAM · $APP_IDENT · $APP_VERSION · $APP_ARCHS"

if [[ $NOTARIZE -eq 1 ]]; then
  # 사전 점검에서 시험 서명으로 확인했지만, 실제 산출물에서도 확인한다.
  grep -q '^Timestamp=' <<<"$APP_SIG" \
    || die "앱 서명에 보안 타임스탬프가 없다 (Signed Time 만 있음)"
  info "보안 타임스탬프 확인됨"

  xcrun stapler validate "$APP_PATH" >/dev/null 2>&1 \
    || die "앱에 공증 티켓이 안 붙었다. 위 빌드 로그에서 notarytool 출력 확인"
  info "앱 공증 티켓 확인"
  spctl -a -t exec -vvv "$APP_PATH" 2>&1 | sed 's/^/  /' \
    || die "Gatekeeper 가 앱을 거부했다 — 위 사유 참고"
fi

# ── 5. DMG 공증 ─────────────────────────────────────────────────────────────
# Tauri 는 앱까지만 공증한다. DMG 는 여기서 따로 올려야 받는 사람이 DMG 를
# 여는 순간부터 경고를 안 본다.
if [[ $NOTARIZE -eq 1 ]]; then
  step "DMG 공증 (애플 서버 응답까지 몇 분 걸린다)"

  # --output-format json 은 결과를 stdout 으로만 낸다(진행 상황은 stderr). 진단 메시지가
  # JSON 에 섞이지 않게 따로 받아 둬야, 제출 자체가 실패했을 때 사용자에게 보여줄 게 남는다.
  NOTARY_ERR="$(mktemp)"
  if ! SUBMIT_JSON="$(xcrun notarytool submit "$DMG_PATH" "${NOTARY_AUTH[@]}" --wait --output-format json 2>"$NOTARY_ERR")"; then
    warn "notarytool 제출이 실패했다. 도구가 남긴 말:"
    sed 's/^/  /' "$NOTARY_ERR"
    rm -f "$NOTARY_ERR"
    die "공증 제출 실패 — 네트워크와 자격증명을 확인할 것"
  fi
  rm -f "$NOTARY_ERR"

  STATUS="$(python3 -c 'import json,sys;print(json.load(sys.stdin).get("status",""))' <<<"$SUBMIT_JSON" 2>/dev/null || true)"
  SUBMIT_ID="$(python3 -c 'import json,sys;print(json.load(sys.stdin).get("id",""))' <<<"$SUBMIT_JSON" 2>/dev/null || true)"
  if [[ -z "$STATUS" ]]; then
    warn "notarytool 응답을 해석하지 못했다. 받은 그대로:"
    printf '%s\n' "$SUBMIT_JSON" | sed 's/^/  /'
    die "공증 결과 확인 실패"
  fi
  info "제출 $SUBMIT_ID → $STATUS"

  if [[ "$STATUS" != "Accepted" ]]; then
    warn "공증 거부됨. 애플이 보낸 사유:"
    xcrun notarytool log "$SUBMIT_ID" "${NOTARY_AUTH[@]}" 2>&1 | sed 's/^/  /' || true
    die "공증 실패 ($STATUS)"
  fi

  xcrun stapler staple "$DMG_PATH" 2>&1 | sed 's/^/  /' || die "DMG 에 티켓을 못 박았다"
  xcrun stapler validate "$DMG_PATH" >/dev/null || die "DMG 스테이플 검증 실패"
  info "DMG 티켓 확인"

  # 받는 사람의 맥이 실제로 통과시키는지 — Gatekeeper 판단을 그대로 물어본다.
  spctl -a -t open --context context:primary-signature -vvv "$DMG_PATH" 2>&1 | sed 's/^/  /' \
    || die "Gatekeeper 가 DMG 를 거부했다 — 위 사유 참고"
fi

# ── 6. 결과 ─────────────────────────────────────────────────────────────────
step "완료"
SHA256="$(shasum -a 256 "$DMG_PATH" | awk '{print $1}')"
SIZE="$(du -h "$DMG_PATH" | awk '{print $1}')"
cat <<EOF
  파일    $DMG_PATH
  크기    $SIZE
  버전    $VERSION_CONF ($GIT_SHA)
  sha256  $SHA256

  Homebrew Cask 나 릴리스 노트에 넣을 체크섬이 위 sha256 이다.
EOF
[[ $NOTARIZE -eq 1 ]] || warn "공증을 건너뛴 결과물이다. 배포하지 말 것"

# 태그는 "방금 검증한 그 커밋"에 찍혀야 한다. 사람이 나중에 손으로 `git tag v0.1.1` 을
# 치면 그때의 HEAD 에 붙어서, 빌드한 커밋과 조용히 어긋날 수 있다 → 커밋을 박아서 준다.
#
# 🔴 `--tags` 를 쓰지 않는다(개발 30 에서 실제로 사고가 났다). 그 플래그는 로컬 태그를
# 전부 밀어서, 공개할 생각이 없던 태그(개발 29 가 남긴 스쿼시 전 원본 히스토리
# pre-squash-dev29 등)까지 태그가 가리키는 커밋 전체와 함께 공개 리포로 올라간다.
# 원격 태그를 지워도 이미 올라간 객체는 한동안 SHA 로 접근 가능하다 → 애초에 안 민다.
# 릴리스에 필요한 건 방금 찍은 버전 태그 하나뿐이므로 그것만 이름으로 지정한다.
if [[ $IS_DIRTY -eq 1 ]]; then
  warn "커밋 안 된 변경이 섞인 결과물이다. 태그도 릴리스도 하지 말 것"
elif [[ $NOTARIZE -eq 0 ]]; then
  # 공증 안 한 결과물에 릴리스 명령을 찍어 주면, 경고를 흘려보고 그대로 붙여넣게 된다.
  # 이 모드는 파이프라인 점검용이므로 릴리스로 가는 길 자체를 안 보여준다.
  warn "--no-notarize 결과물이라 릴리스 명령을 출력하지 않는다"
else
  # 빌드한 커밋이 아직 origin/main 에 없으면, 태그만 밀 경우 태그가 가리키는 커밋을
  # 브랜치에서 볼 수 없고, main 을 같이 밀면 태그에 없는 후속 커밋까지 나간다.
  # 어느 쪽이든 사람이 알고 골라야 하므로, 상태를 말해 주고 명령을 나눠서 준다.
  if ! git fetch -q origin main 2>/dev/null; then
    warn "origin/main 을 새로 못 받아왔다(네트워크·권한?). 아래 판단은 마지막으로 받아온 정보 기준이다"
  fi
  ORIGIN_MAIN="$(git rev-parse -q --verify origin/main || true)"
  HEAD_FULL="$(git rev-parse HEAD)"
  # 같은 커밋인지가 아니라 **origin/main 에 들어 있는지**를 본다. main 이 이 커밋보다
  # 앞서 있는 건 정상(옛 태그를 다시 빌드하는 경우 등)인데, 같은지만 보면 그때마다
  # "main 에 올려라"는 오탐이 난다.
  if [[ -n "$ORIGIN_MAIN" ]] && ! git merge-base --is-ancestor "$HEAD_FULL" "$ORIGIN_MAIN"; then
    warn "빌드한 커밋($GIT_SHA)이 아직 origin/main 에 없다. 태그만 밀면 릴리스는 생기지만"
    warn "그 커밋이 브랜치 어디에도 없다 — 먼저 main 에 올리고 나서 태그를 밀 것."
    # 워크트리·기능 브랜치에서 빌드하는 게 이 프로젝트의 기본 흐름이라, 여기서 그냥
    # "git push origin main" 이라고 하면 로컬 main(= 이 커밋이 없는 브랜치)을 밀게 된다.
    CUR_BRANCH="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo HEAD)"
    if [[ "$CUR_BRANCH" == "main" ]]; then
      warn "  git push origin main"
    else
      warn "  지금 브랜치는 '$CUR_BRANCH' 다. main 에 머지한 뒤 밀 것:"
      warn "    git -C \"\$(git rev-parse --show-toplevel)\" checkout main && git merge --ff-only $GIT_SHA && git push origin main"
      warn "  (그냥 'git push origin main' 은 이 커밋이 없는 로컬 main 을 민다)"
    fi
  fi
  if ! git rev-parse -q --verify "refs/tags/$VERSION_TAG" >/dev/null; then
    # 🔴 `--tags` 를 쓰지 않는다(개발 30 에서 실제 사고). 그 플래그는 로컬 태그를 전부 밀어서,
    # 공개할 생각이 없던 태그(스쿼시 전 원본 히스토리 등)까지 그 커밋 전체와 함께 올라간다.
    # 원격 태그를 지워도 이미 올라간 객체는 한동안 SHA 로 접근 가능하다 → 애초에 안 민다.
    cat <<EOF

  이 배포본을 릴리스하려면, 방금 빌드한 바로 그 커밋에 태그를 찍는다:

    git tag $VERSION_TAG $GIT_SHA
    git push origin $VERSION_TAG
    gh release create $VERSION_TAG "$DMG_PATH" --title "Kura $VERSION_TAG" --notes "sha256: $SHA256"
EOF
  else
    # 태그가 이미 있으면(= 위 게이트에서 태그 = HEAD 로 확인된 상태) 안내가 통째로
    # 사라져서, 정작 배포하려는 사람이 다음 명령을 못 받는다.
    cat <<EOF

  태그 $VERSION_TAG 는 이미 이 커밋($GIT_SHA)에 있다. 남은 단계:

    git push origin $VERSION_TAG
    gh release create $VERSION_TAG "$DMG_PATH" --title "Kura $VERSION_TAG" --notes "sha256: $SHA256"
EOF
  fi
fi

RELEASE_OK=1   # 여기까지 왔으면 산출물을 남긴다 (cleanup 이 이름을 안 바꾼다)
