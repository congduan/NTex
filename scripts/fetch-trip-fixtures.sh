#!/usr/bin/env bash
# 获取 TRIP 一致性测试 fixtures（trip.tex / trip.typ / trip.log）。
#
# 策略（尽力而为，失败不致命——框架在 fixtures 缺失时返回"跳过"）：
# 1. trip.tex 优先用系统 TeX 的 `kpsewhich` 定位；
# 2. 其余从 CTAN / TeX Live 源码镜像下载。
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${NTEX_FIXTURES_DIR:-$REPO_ROOT/fixtures/trip}"
mkdir -p "$TARGET"

fetch() { # url -> dest（成功返回 0）
  echo "  下载 $2 <- $1"
  curl -fsSL --retry 3 --connect-timeout 10 -o "$2" "$1"
}

fetch_first_of() { # dest, url...
  local dest="$1"; shift
  for url in "$@"; do
    if fetch "$url" "$dest" 2>/dev/null; then
      echo "  已获取：$dest"
      return 0
    fi
  done
  echo "  警告：未能获取 $dest" >&2
  return 1
}

echo "目标目录：$TARGET"

# trip.tex
if command -v kpsewhich >/dev/null 2>&1 && kpsewhich trip.tex >/dev/null 2>&1; then
  cp "$(kpsewhich trip.tex)" "$TARGET/trip.tex"
  echo "trip.tex <- kpsewhich"
else
  fetch_first_of "$TARGET/trip.tex" \
    "https://mirrors.ctan.org/macros/plain/base/trip.tex" || true
fi

# trip.typ / trip.log（参考输出，来自 TeX Live 源码树的 web2c 测试）
fetch_first_of "$TARGET/trip.typ" \
  "https://raw.githubusercontent.com/TeX-Live/texlive-source/trunk/texk/web2c/trip.typ" \
  "https://raw.githubusercontent.com/TeX-Live/texlive-source/master/texk/web2c/trip.typ" || true

fetch_first_of "$TARGET/trip.log" \
  "https://raw.githubusercontent.com/TeX-Live/texlive-source/trunk/texk/web2c/trip.log" \
  "https://raw.githubusercontent.com/TeX-Live/texlive-source/master/texk/web2c/trip.log" || true

echo "---- fixtures ----"
ls -la "$TARGET"
if [ -s "$TARGET/trip.tex" ] && [ -s "$TARGET/trip.typ" ] && [ -s "$TARGET/trip.log" ]; then
  echo "TRIP fixtures 就绪。"
else
  echo "TRIP fixtures 不完整：请检查网络或手动放置 trip.tex/trip.typ/trip.log 到 $TARGET" >&2
fi
