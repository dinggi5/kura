#!/usr/bin/env bash
# kura-mcp 크레이트의 두 바이너리를 Tauri 사이드카(externalBin)로 넣을 자리에 놓는다 (개발 34).
#
#   ./scripts/build-sidecars.sh                     호스트 아키텍처만
#   ./scripts/build-sidecars.sh aarch64-apple-darwin x86_64-apple-darwin
#                                                   유니버설 빌드용 (두 타깃 다)
#
# 왜 이게 필요한가:
#   DMG·brew 로 앱만 받은 사람은 지금까지 MCP 를 아예 붙일 수 없었다. README 의 등록 예시가
#   `cargo run --manifest-path ./kura-mcp/Cargo.toml` 이라 **소스 클론 + Rust 툴체인**이
#   있어야 했기 때문이다. 앱 번들 안에 바이너리를 넣으면 앱과 함께 서명·공증·스테이플되고,
#   등록은 절대경로 한 줄로 끝난다.
#
# Tauri 사이드카 규칙(v2): 설정의 `binaries/kura-mcp` 는 파일 이름이
# `binaries/kura-mcp-<타깃트리플>` 이어야 하고, 번들에 들어갈 때 트리플이 떨어져서
# `Kura.app/Contents/MacOS/kura-mcp` 로 놓인다.
#
# 🔴 CLI 는 `kura` 가 아니라 `kura-cli` 로 넣는다. 앱 본체 실행파일이 이미
# `Contents/MacOS/kura` 라서(=src-tauri 크레이트 이름) 같은 이름을 쓰면 번들 안에서
# 앱 자신을 덮어쓴다. PATH 에 올릴 때 `kura` 라는 이름을 주는 건 심링크가 할 일이다.
set -euo pipefail

cd "$(dirname "$0")/.."

ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
info() { printf '  %s\n' "$*"; }
die()  { printf '\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

OUT_DIR="src-tauri/binaries"
# 왼쪽 = cargo 가 만드는 이름, 오른쪽 = 번들에 들어갈 이름.
BINS=("kura-mcp:kura-mcp" "kura:kura-cli")

TARGETS=("$@")
if [[ ${#TARGETS[@]} -eq 0 ]]; then
  HOST_TRIPLE="$(rustc -vV | awk '/^host:/{print $2}')" \
    || die "rustc 로 호스트 타깃을 알아내지 못했다"
  [[ -n "$HOST_TRIPLE" ]] || die "rustc -vV 에 host 줄이 없다"
  TARGETS=("$HOST_TRIPLE")
fi

# 소스 중 가장 최근에 바뀐 시각. 이것보다 모든 산출물이 새것이면 cargo 를 아예 안 부른다.
#
# 왜 굳이 건너뛰나: 이 스크립트는 tauri.conf.json 의 beforeBuildCommand 에서도 불린다.
# 즉 release.sh 의 **자격증명이 실린 빌드 안에서** 한 번 더 불린다는 뜻이고, 거기서 cargo 가
# 새로 돌면 업데이트 서명 개인키·애플 앱 암호를 kura-mcp 의존성 빌드 스크립트까지 물려받는다
# (개발 30 보류 P1 이 그대로 넓어진다). release.sh 는 자격증명을 싣기 **전에** 이 스크립트를
# 직접 부르고, 그때 만들어 둔 산출물 덕에 빌드 안쪽 호출은 아무것도 안 하고 끝난다.
SRC_NEWEST=""
while IFS= read -r f; do
  [[ -z "$SRC_NEWEST" || "$f" -nt "$SRC_NEWEST" ]] && SRC_NEWEST="$f"
done < <(find kura-mcp/src kura-mcp/Cargo.toml kura-mcp/Cargo.lock -type f)
[[ -n "$SRC_NEWEST" ]] || die "kura-mcp 소스를 찾지 못했다 (리포 루트에서 도는 게 맞는지 확인)"

needs_build() {
  local t out
  for t in "${TARGETS[@]}"; do
    for pair in "${BINS[@]}"; do
      out="$OUT_DIR/${pair#*:}-$t"
      [[ -f "$out" && "$out" -nt "$SRC_NEWEST" ]] || return 0
    done
  done
  return 1
}

# STRICT(=release.sh) 에서는 만들지 않는다. 여기서 만들어야 하는 상황 자체가
# "자격증명이 실린 빌드 안에서 cargo 가 돈다"는 뜻이라 멈추는 게 맞다.
if [[ "${KURA_SIDECARS_STRICT:-0}" == "1" ]]; then
  if needs_build; then
    die "사이드카가 최신이 아닌데 STRICT 모드다.
  자격증명이 실린 빌드 안에서 cargo 를 돌리지 않으려고 막아 둔 것이다.
  release.sh 가 빌드 전에 부르는 곳에서 실패했을 가능성이 크다 — 위 로그를 확인할 것."
  fi
  ok "사이드카 최신 (STRICT: 빌드 안 함)"
  exit 0
fi

if ! needs_build; then
  ok "사이드카 최신 (${TARGETS[*]})"
  exit 0
fi

mkdir -p "$OUT_DIR"
INSTALLED_TARGETS="$(rustup target list --installed 2>/dev/null || true)"

for t in "${TARGETS[@]}"; do
  if [[ -n "$INSTALLED_TARGETS" ]] && ! grep -qx "$t" <<<"$INSTALLED_TARGETS"; then
    die "타깃 $t 가 설치돼 있지 않다:  rustup target add $t"
  fi
  info "빌드 $t"
  # 호스트와 같은 타깃이면 --target 을 안 붙인다. 붙이면 cargo 가 target/<트리플>/ 아래에
  # 의존성 트리를 **한 벌 더** 만든다(alloy·aws-lc 까지 다시 컴파일 = 수 GB).
  # 개발 34 에서 실제로 이것 때문에 디스크가 찼다.
  TARGET_ARGS=()
  REL_DIR="kura-mcp/target/release"
  if [[ "$t" != "$(rustc -vV | awk '/^host:/{print $2}')" ]]; then
    TARGET_ARGS=(--target "$t")
    REL_DIR="kura-mcp/target/$t/release"
  fi
  # --locked: Cargo.lock 을 고치지 않는다. 배포본에 들어가는 바이너리라
  # 빌드가 의존성 버전을 조용히 올리면 안 된다.
  cargo build --release --locked --manifest-path kura-mcp/Cargo.toml \
    "${TARGET_ARGS[@]+"${TARGET_ARGS[@]}"}" --bin kura-mcp --bin kura \
    || die "cargo 빌드 실패 ($t)"

  for pair in "${BINS[@]}"; do
    src="$REL_DIR/${pair%%:*}"
    out="$OUT_DIR/${pair#*:}-$t"
    [[ -f "$src" ]] || die "cargo 가 만든 파일이 없다: $src"
    # 덮어쓰기가 아니라 지우고 복사한다. 실행 중인 바이너리를 덮어쓰면
    # 그 프로세스가 죽는데(ETXTBSY 가 아니라 조용한 크래시), MCP 서버는 늘 떠 있다.
    rm -f "$out"
    cp "$src" "$out"
    chmod +x "$out"
    ok "$(basename "$out")  $(du -h "$out" | cut -f1)"
  done
done
