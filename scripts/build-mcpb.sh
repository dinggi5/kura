#!/usr/bin/env bash
# Claude 데스크톱 확장(.mcpb) 만들기 (개발 34).
#
#   ./scripts/build-mcpb.sh              → src-tauri/target/mcpb/kura-<버전>.mcpb
#
# 번들에는 **실행 파일이 없다**. manifest 와 런처 스크립트(mcpb/server/kura-mcp)뿐이고,
# 런처는 설치된 Kura.app 안의 서명·공증된 kura-mcp 를 서명 확인 후 exec 한다.
# 그래서 이 산출물은 서명·공증 대상이 아니다 — 아래 "맥오 금지" 게이트가 그 전제를 지킨다.
#
# 개발 35: 같은 파일을 src-tauri/resources/kura.mcpb 로도 복사한다 — 앱 Resources 에
# 동봉돼 "AI 연결" 화면의 'Claude 데스크톱에 연결' 버튼이 이 파일을 연다. 그래서 이
# 스크립트는 tauri.conf.json 의 beforeBuild/DevCommand 에서도 불리고, 사이드카 스크립트와
# 같은 신선도 검사 + STRICT 게이트를 갖는다(자격증명 실린 빌드 안에서 npx 가 새로 돌지 않게).
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

MCPB_FILE="$OUT_DIR/kura-$VERSION_CONF.mcpb"
# 앱 Resources 동봉본(개발 35). tauri.conf.json bundle.resources 가 이 고정 이름을 집는다.
RESOURCE_FILE="src-tauri/resources/kura.mcpb"

# ── 신선도 검사 (사이드카 스크립트와 같은 이유) ─────────────────────────────
# 이 스크립트는 beforeBuild/DevCommand 에서도 불린다. release.sh 는 자격증명을 싣기
# **전에** 직접 부르고, 빌드 안쪽 호출은 여기서 아무것도 안 하고 끝나야 한다 —
# 자격증명 실린 환경에서 npx(임의 노드 코드)가 새로 돌지 않게.
# 이 스크립트 자신과 package-lock.json 도 소스다(코덱스 개발35 3차): 패킹 방식이나
# mcpb 패커 버전이 바뀌어도 입력 파일들 mtime 은 그대로라 옛 산출물이 "최신"으로 통과한다.
SOURCES=("$SRC_DIR/manifest.json" "$SRC_DIR/server/kura-mcp" \
  "src-tauri/icons/icon.png" "src-tauri/tauri.conf.json" \
  "scripts/build-mcpb.sh" "package-lock.json")
needs_build() {
  local out src
  for out in "$MCPB_FILE" "$RESOURCE_FILE"; do
    [[ -f "$out" ]] || return 0
    for src in "${SOURCES[@]}"; do
      [[ "$out" -nt "$src" ]] || return 0
    done
  done
  # mtime 만으로는 부족하다: src-tauri/build.rs 가 신규 체크아웃 방어로 만드는
  # **빈 자리표시자**는 방금 생긴 파일이라 mtime 검사를 통과한다(실측). 두 산출물이
  # 같은 바이트일 때만 최신으로 친다 — 자리표시자든 어긋난 사본이든 여기서 걸린다.
  cmp -s "$MCPB_FILE" "$RESOURCE_FILE" || return 0
  return 1
}
if [[ "${KURA_SIDECARS_STRICT:-0}" == "1" ]]; then
  if needs_build; then
    die "확장(.mcpb)이 최신이 아닌데 STRICT 모드다.
  자격증명이 실린 빌드 안에서 도구를 돌리지 않으려고 막아 둔 것이다.
  release.sh 가 빌드 전에 부르는 곳에서 실패했을 가능성이 크다 — 위 로그를 확인할 것."
  fi
  ok "확장 최신 (STRICT: 빌드 안 함)"
  exit 0
fi
if ! needs_build; then
  ok "확장 최신  $MCPB_FILE"
  exit 0
fi

rm -rf "$STAGE"
mkdir -p "$STAGE/server"
cp "$SRC_DIR/manifest.json" "$STAGE/manifest.json"
cp "$SRC_DIR/server/kura-mcp" "$STAGE/server/kura-mcp"
chmod +x "$STAGE/server/kura-mcp"
# 아이콘은 앱 아이콘 하나를 정본으로 쓴다. 리포에 사본을 두면 앱 아이콘만 바뀌고
# 확장 아이콘은 옛것으로 남는 드리프트가 생긴다. (512×512 PNG 요구)
cp src-tauri/icons/icon.png "$STAGE/icon.png"

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

# 앱 Resources 동봉본 — 릴리스 자산과 **같은 파일의 바이트 사본**이어야 한다.
# 두 번 pack 하면 zip 타임스탬프 때문에 해시가 갈려서, 릴리스 본문의 sha256 으로
# 동봉본을 검증할 수 없게 된다.
mkdir -p "$(dirname "$RESOURCE_FILE")"
rm -f "$RESOURCE_FILE"
cp "$MCPB_FILE" "$RESOURCE_FILE"

ok "$MCPB_FILE  $(du -h "$MCPB_FILE" | cut -f1)"
ok "$RESOURCE_FILE (앱 Resources 동봉본)"
info "설치: Finder 에서 더블클릭하거나 Claude 데스크톱 설정 → 확장에서 파일 선택"
