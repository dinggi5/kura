#!/usr/bin/env bash
# Claude 데스크톱 확장(.mcpb) 만들기 (개발 34).
#
#   ./scripts/build-mcpb.sh              → src-tauri/target/mcpb/kura-<버전>.mcpb
#
# 번들에는 **실행 파일이 없다**. manifest 와 런처 스크립트(mcpb/server/kura-mcp)뿐이고,
# 런처는 설치된 Kura.app 안의 서명·공증된 kura-mcp 를 서명 확인 후 exec 한다.
# 그래서 이 산출물은 서명·공증 대상이 아니다 — 아래 "맥오 금지" 게이트가 그 전제를 지킨다.
set -euo pipefail

cd "$(dirname "$0")/.."

ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
info() { printf '  %s\n' "$*"; }
die()  { printf '\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

SRC_DIR="mcpb"
OUT_DIR="src-tauri/target/mcpb"
STAGE="$OUT_DIR/stage"

command -v npx >/dev/null || die "npx 가 없다 (Node 필요)"
[[ -f "$SRC_DIR/manifest.json" ]] || die "$SRC_DIR/manifest.json 이 없다"
[[ -x "$SRC_DIR/server/kura-mcp" ]] || die \
  "$SRC_DIR/server/kura-mcp 에 실행 비트가 없다:  chmod +x $SRC_DIR/server/kura-mcp"

VERSION_CONF="$(python3 -c 'import json;print(json.load(open("src-tauri/tauri.conf.json"))["version"])')"
VERSION_MCPB="$(python3 -c 'import json;print(json.load(open("mcpb/manifest.json"))["version"])')"
# 🔴 버전을 보는 다섯째 파일이다(개발 32 의 네 곳 + 여기). 갈리면 사용자가 받은 확장이
# 어느 앱 버전을 위한 건지 알 수 없게 된다. release.sh 사전 점검에도 같은 검사가 있다.
[[ "$VERSION_CONF" == "$VERSION_MCPB" ]] || die \
  "버전 불일치 — tauri.conf.json=$VERSION_CONF / mcpb/manifest.json=$VERSION_MCPB"

rm -rf "$STAGE"
mkdir -p "$STAGE/server"
cp "$SRC_DIR/manifest.json" "$STAGE/manifest.json"
cp "$SRC_DIR/server/kura-mcp" "$STAGE/server/kura-mcp"
chmod +x "$STAGE/server/kura-mcp"
# 아이콘은 앱 아이콘 하나를 정본으로 쓴다. 리포에 사본을 두면 앱 아이콘만 바뀌고
# 확장 아이콘은 옛것으로 남는 드리프트가 생긴다. (512×512 PNG 요구)
cp src-tauri/icons/icon.png "$STAGE/icon.png"

MCPB_FILE="$OUT_DIR/kura-$VERSION_CONF.mcpb"
rm -f "$MCPB_FILE"
npx --no-install mcpb pack "$STAGE" "$MCPB_FILE" >/dev/null \
  || die "mcpb pack 실패 (자세히 보려면: npx mcpb pack $STAGE)"
[[ -f "$MCPB_FILE" ]] || die "번들이 안 나왔다: $MCPB_FILE"

# ── 나온 물건을 열어서 확인한다 ──────────────────────────────────────────────
LIST="$(unzip -Z1 "$MCPB_FILE")" || die "번들을 열지 못했다"
for want in manifest.json server/kura-mcp icon.png; do
  grep -Fxq "$want" <<<"$LIST" || die "번들에 $want 가 없다"
done

# 🔴 맥오 금지. 실행 파일이 들어가는 순간 이 번들은 서명·공증 대상이 되는데,
# 맨 바이너리에는 공증 티켓을 스테이플할 수 없어서 조용히 "온라인일 때만 되는" 배포가 된다.
VERIFY_DIR="$OUT_DIR/verify"
rm -rf "$VERIFY_DIR"
mkdir -p "$VERIFY_DIR"
unzip -qq "$MCPB_FILE" -d "$VERIFY_DIR" || die "번들 압축을 풀지 못했다"
while IFS= read -r f; do
  if file -b "$f" | grep -q 'Mach-O'; then
    die "번들에 실행 파일이 들어 있다: ${f#"$VERIFY_DIR/"}
  이 번들은 서명·공증을 안 하는 전제로 만들어졌다. 설계를 바꿀 거면 release.sh 도 같이 볼 것."
  fi
done < <(find "$VERIFY_DIR" -type f)

# 🔴 zip 이 실행 비트를 보존하는지 본다. 여기가 깨지면 Claude 데스크톱이 확장을 풀어도
# 런처를 실행하지 못해 "서버 시작 실패"만 뜬다 — 우리 코드는 한 줄도 안 돌아 보고도 없다.
[[ -x "$VERIFY_DIR/server/kura-mcp" ]] || die \
  "압축을 푸니 런처에 실행 비트가 없다. mcpb 가 파일 모드를 잃은 것이다 — 이대로 배포하면 안 된다."

# 런처가 가리키는 이름이 실제 사이드카 이름과 같은지. 둘이 갈리면 앱은 멀쩡한데
# 확장만 "앱을 찾지 못했습니다"로 죽는다.
grep -q 'Contents/MacOS/kura-mcp' "$VERIFY_DIR/server/kura-mcp" || die \
  "런처가 Contents/MacOS/kura-mcp 를 가리키지 않는다"
python3 -c '
import json,sys
c=json.load(open("src-tauri/tauri.conf.json"))
b=c.get("bundle",{}).get("externalBin",[])
assert "binaries/kura-mcp" in b, f"tauri.conf.json 의 externalBin 에 binaries/kura-mcp 가 없다: {b}"
' || die "앱이 kura-mcp 사이드카를 넣지 않는다 — 확장이 가리킬 바이너리가 안 생긴다"

rm -rf "$VERIFY_DIR"
ok "$MCPB_FILE  $(du -h "$MCPB_FILE" | cut -f1)"
info "설치: Finder 에서 더블클릭하거나 Claude 데스크톱 설정 → 확장에서 파일 선택"
