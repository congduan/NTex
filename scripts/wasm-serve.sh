#!/usr/bin/env bash
# ntex-wasm 快速启动：构建 WASM → 生成绑定 → 起 HTTP server 演示页。
# 用法：
#   scripts/wasm-serve.sh            # 全流程（构建+绑定+serve，端口 8000）
#   scripts/wasm-serve.sh --no-build # 跳过构建（已构建过，直接 serve）
#   scripts/wasm-serve.sh --port 9000
# 依赖：rustup（wasm32 target）、wasm-bindgen-cli（版本须与 Cargo.lock 的
# wasm-bindgen 一致，见 crates/ntex-wasm/README.md）。构建产物不入库。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE="$ROOT/crates/ntex-wasm"
PORT=8000
BUILD=1
while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-build) BUILD=0; shift ;;
    --port) PORT="$2"; shift 2 ;;
    *) echo "未知参数: $1" >&2; exit 2 ;;
  esac
done

export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
command -v cargo >/dev/null || { echo "缺 cargo（~/.cargo/bin）" >&2; exit 2; }

if [[ $BUILD -eq 1 ]]; then
  rustup target list --installed | grep -q wasm32-unknown-unknown || {
    echo "==> 安装 wasm32 target"; rustup target add wasm32-unknown-unknown
  }
  echo "==> 构建 ntex-wasm（wasm32 release，首次约 1-3 分钟）"
  (cd "$ROOT" && cargo build -p ntex-wasm --target wasm32-unknown-unknown --release)

  WASM="$ROOT/target/wasm32-unknown-unknown/release/ntex_wasm.wasm"
  [[ -f "$WASM" ]] || { echo "构建产物缺失: $WASM" >&2; exit 1; }

  if ! command -v wasm-bindgen >/dev/null; then
    VER=$(grep -A1 '^name = "wasm-bindgen"' "$ROOT/Cargo.lock" | grep version | head -1 | cut -d'"' -f2)
    echo "缺 wasm-bindgen-cli。安装（版本必须 = $VER）：" >&2
    echo "  cargo install wasm-bindgen-cli --version $VER" >&2
    exit 2
  fi
  echo "==> 生成 JS 绑定 → www/pkg/"
  wasm-bindgen --out-dir "$CRATE/www/pkg" --target web "$WASM"
fi

echo "==> HTTP server: http://localhost:$PORT （Ctrl-C 停止）"
cd "$CRATE/www"
exec python3 -m http.server "$PORT"
