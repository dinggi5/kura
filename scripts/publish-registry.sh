#!/usr/bin/env bash
# MCP 공식 레지스트리 발행 (개발 37).
#
#   ./scripts/publish-registry.sh            → server.json 갱신 + 검증 + 발행
#   ./scripts/publish-registry.sh --dry-run  → 발행만 빼고 전부 (로그인 불필요)
#
# server.json 은 릴리스된 .mcpb 를 가리킨다 — 로컬 빌드물이 아니라 **GitHub 릴리스에
# 실제로 올라간 자산**을 내려받아 sha256 을 계산한다. 클라이언트가 받게 될 바로 그
# 바이트를 재는 것이고, 로컬에 뭐가 남아 있든 결과가 갈릴 수 없다.
#
# 순서: 릴리스(release.sh --publish)가 끝난 뒤에 돌린다. 자산이 없으면 여기서 죽는다.
# 발행 계정: 네임스페이스 io.github.dinggi5 는 GitHub 로그인(dinggi5)으로 증명한다.
# 로그인은 **이 스크립트가 발행 직전에 직접** 한다 (개발 44) — 토큰 수명이 5분뿐이라
# 사람이 먼저 로그인해 두면 그사이 다른 걸 하다 만료된다(개발 40·43 이 연속으로 밟았다).
set -euo pipefail

cd "$(dirname "$0")/.."

ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
info() { printf '  %s\n' "$*"; }
die()  { printf '\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

# 발행은 취소 불가능한 원격 부작용이다 — 오타난 플래그가 조용히 무시된 채 발행으로
# 흘러가지 않게, 모르는 인자는 전부 거부한다 (코덱스 1차 P1).
DRY_RUN=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    *) die "모르는 인자: $arg  (지원: --dry-run)" ;;
  esac
done

REGISTRY_NAME="io.github.dinggi5/kura"
REPO="dinggi5/kura"

command -v mcp-publisher >/dev/null || die \
  "mcp-publisher 가 없다. 설치할 것:  brew install mcp-publisher"
[[ -f server.json ]] || die "server.json 이 없다 (레포 루트에서 관리한다)"

# ── 버전 = mcpb/manifest.json (릴리스 검사 여섯 곳과 같은 정본 계열) ──────────
VERSION="$(python3 -c 'import json;print(json.load(open("mcpb/manifest.json"))["version"])')"
ASSET_URL="https://github.com/$REPO/releases/download/v$VERSION/kura-$VERSION.mcpb"

# ── 아이콘 드리프트 가드 ────────────────────────────────────────────────────
# server.json 의 아이콘은 GitHub Pages(docs/icon.png)를 가리킨다. 정본은 앱 아이콘
# 하나(src-tauri/icons/icon.png)다 — 앱 아이콘만 바뀌고 레지스트리 아이콘이 옛것으로
# 남는 드리프트를 바이트 비교로 막는다.
cmp -s docs/icon.png src-tauri/icons/icon.png || die \
  "docs/icon.png 이 앱 아이콘과 다르다. 갱신할 것:  cp src-tauri/icons/icon.png docs/icon.png"
ok "아이콘 동기화 확인 (docs/icon.png = 앱 아이콘)"

# ── 릴리스 자산을 내려받아 sha256 계산 ─────────────────────────────────────
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
curl -fsSL "$ASSET_URL" -o "$TMP_DIR/kura.mcpb" || die \
  "릴리스 자산을 받지 못했다: $ASSET_URL
  v$VERSION 릴리스가 아직 없으면 먼저 배포할 것:  ./scripts/release.sh --publish"
SHA256="$(shasum -a 256 "$TMP_DIR/kura.mcpb" | cut -d' ' -f1)"
ok "릴리스 자산 sha256: $SHA256"

# ── server.json 의 릴리스 종속 필드 갱신 (메타데이터는 파일에 있는 그대로) ──
python3 - "$VERSION" "$ASSET_URL" "$SHA256" <<'EOF'
import json, sys
version, url, sha = sys.argv[1:4]
with open("server.json") as f:
    doc = json.load(f)
doc["version"] = version
pkgs = doc.get("packages", [])
assert len(pkgs) == 1 and pkgs[0].get("registryType") == "mcpb", \
    "server.json 의 packages 는 mcpb 하나여야 한다"
pkgs[0]["identifier"] = url
pkgs[0]["version"] = version
pkgs[0]["fileSha256"] = sha
with open("server.json", "w") as f:
    json.dump(doc, f, ensure_ascii=False, indent=2)
    f.write("\n")
EOF
if git diff --quiet -- server.json; then
  ok "server.json 최신 (v$VERSION)"
else
  info "server.json 갱신됨 (v$VERSION) — 발행 후 커밋할 것:  git add server.json"
fi

# ── 로컬 검증 → 발행 ───────────────────────────────────────────────────────
mcp-publisher validate server.json || die "server.json 검증 실패"
ok "server.json 스키마 검증 통과"

if [[ "$DRY_RUN" == "1" ]]; then
  ok "dry-run 끝 (발행 안 함)"
  exit 0
fi

# 같은 이름+버전은 레지스트리가 재발행을 거부한다. 지난 실행이 발행까지 하고 확인만
# 실패했으면, 무조건 다시 발행하려는 스크립트는 자기 검증 경로에 영영 못 간다
# (코덱스 1차 P2) — 그래서 발행 전에 원격을 먼저 본다.
LOOKUP="https://registry.modelcontextprotocol.io/v0.1/servers/${REGISTRY_NAME/\//%2F}/versions/$VERSION"
fetch_remote_sha() {
  # 미발행(404)이든 일시 장애든 빈 문자열 — 어느 쪽이어도 발행 시도로 진행하면 된다.
  curl -fsSL "$LOOKUP" 2>/dev/null | python3 -c '
import json, sys
doc = json.load(sys.stdin)
srv = doc.get("server", doc)
print(srv["packages"][0]["fileSha256"])' 2>/dev/null
}

REMOTE_SHA="$(fetch_remote_sha || true)"
if [[ -n "$REMOTE_SHA" ]]; then
  [[ "$REMOTE_SHA" == "$SHA256" ]] || die \
    "v$VERSION 은 이미 발행돼 있는데 sha256 이 다르다 — 원격 $REMOTE_SHA / 로컬 $SHA256
  릴리스 자산이 발행 후에 바뀌었다는 뜻이다. 버전을 올려 다시 릴리스할 것."
  ok "이미 발행돼 있다: $REGISTRY_NAME v$VERSION (sha256 일치) — 발행 건너뜀"
  exit 0
fi

# ── 로그인 — 발행 직전에, 스크립트가 직접 (개발 44) ────────────────────────
# 토큰은 JWT 고 수명이 5분이다(exp - iat 실측). 개발 40 이 "발행 직전에 로그인"으로
# 정리했는데 개발 43 이 또 밟았다 — 로그인해 두고 그사이 앱 확인을 하나 끼웠더니 만료.
# 사람이 두 명령 사이에 아무것도 안 끼우기를 바라는 대신, 여기서 붙여 버린다.
#
# 만료를 "발행이 실패하면 안다"로 두지 않고 **미리 재는** 이유: 발행은 되돌릴 수 없는
# 원격 부작용이라, 만료된 토큰으로 절반쯤 나가는 것보다 쏘기 전에 아는 게 낫다.
TOKEN_FILE="${XDG_CONFIG_HOME:-$HOME/.config}/mcp-publisher/token.json"

# 남은 수명(초). 파일이 없거나 모양이 다르면 0 = "없는 것으로 친다" → 로그인시킨다.
token_seconds_left() {
  [[ -f "$TOKEN_FILE" ]] || { echo 0; return; }
  python3 - "$TOKEN_FILE" <<'EOF' 2>/dev/null || echo 0
import base64, json, sys, time
try:
    tok = json.load(open(sys.argv[1]))["token"]
    payload = tok.split(".")[1]
    payload += "=" * (-len(payload) % 4)          # base64url 패딩 복원
    exp = json.loads(base64.urlsafe_b64decode(payload))["exp"]
except Exception:
    print(0)
else:
    print(max(0, int(exp - time.time())))
EOF
}

# 발행 왕복(네트워크 + 사후 검증)에 쓸 여유. 5분짜리 토큰에서 2분은 넉넉한 마진이다.
NEED_SECONDS=120
LEFT="$(token_seconds_left)"
if [[ "$LEFT" -lt "$NEED_SECONDS" ]]; then
  if [[ "$LEFT" -eq 0 ]]; then
    info "레지스트리 로그인이 없거나 만료됐다 — 지금 로그인한다 (브라우저에서 dinggi5 로 인증)"
  else
    info "토큰이 ${LEFT}초 뒤 만료된다 — 발행 전에 다시 로그인한다"
  fi
  mcp-publisher login github || die "로그인 실패. 브라우저에서 dinggi5 로 인증할 것"
  LEFT="$(token_seconds_left)"
  [[ "$LEFT" -ge "$NEED_SECONDS" ]] || die \
    "로그인은 끝났는데 토큰 수명이 ${LEFT}초뿐이다 — 발행을 시작하지 않는다. 다시 돌릴 것"
fi
ok "레지스트리 토큰 유효 (${LEFT}초 남음)"

mcp-publisher publish server.json || die \
  "발행 실패. 토큰이 만료됐으면 다시 돌릴 것 (이 스크립트가 로그인부터 다시 한다)"

# ── 사후 검증: 레지스트리가 실제로 이 버전을 광고하는지 (우리 손 밖 API 기준) ──
REMOTE_SHA="$(fetch_remote_sha || true)"
[[ -n "$REMOTE_SHA" ]] || die "발행 확인 실패: $LOOKUP  (다시 돌리면 확인만 재시도한다)"
[[ "$REMOTE_SHA" == "$SHA256" ]] || die \
  "레지스트리의 sha256 이 우리가 잰 값과 다르다 — 원격 $REMOTE_SHA / 로컬 $SHA256"
ok "레지스트리 발행 확인: $REGISTRY_NAME v$VERSION (sha256 일치)"
info "목록 확인: https://registry.modelcontextprotocol.io/v0.1/servers?search=$REGISTRY_NAME"
