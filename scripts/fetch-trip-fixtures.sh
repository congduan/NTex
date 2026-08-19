#!/usr/bin/env bash
# 获取 TRIP / ETRIP 一致性测试 fixtures。
#
# - TRIP（Knuth 官方）：trip.tex / trip.typ / trip.log —— CTAN knuth dist
#   （raw.githubusercontent 在部分网络不可达，改用 CTAN 与 jsdelivr CDN）。
# - ETRIP（e-TeX）：etrip.tex / etrip.log —— TeX Live 源码 texk/web2c/etexdir/etrip/。
#
# 策略（尽力而为，失败不致命——框架在 fixtures 缺失时返回"跳过"）。
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${NTEX_FIXTURES_DIR:-$REPO_ROOT/fixtures}"
TL="https://cdn.jsdelivr.net/gh/TeX-Live/texlive-source@trunk"

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

# ---------- TRIP ----------
mkdir -p "$TARGET/trip"
echo "== TRIP fixtures =="
if command -v kpsewhich >/dev/null 2>&1 && kpsewhich trip.tex >/dev/null 2>&1; then
  cp "$(kpsewhich trip.tex)" "$TARGET/trip/trip.tex"
  echo "trip.tex <- kpsewhich"
else
  fetch_first_of "$TARGET/trip/trip.tex" \
    "https://mirrors.ctan.org/systems/knuth/dist/tex/trip.tex" \
    "https://mirrors.ctan.org/macros/plain/base/trip.tex" || true
fi
fetch_first_of "$TARGET/trip/trip.typ" \
  "https://mirrors.ctan.org/systems/knuth/dist/tex/trip.typ" \
  "$TL/texk/web2c/triptrap/trip.typ" || true
fetch_first_of "$TARGET/trip/trip.log" \
  "https://mirrors.ctan.org/systems/knuth/dist/tex/trip.log" \
  "$TL/texk/web2c/triptrap/trip.log" || true

# ---------- ETRIP ----------
mkdir -p "$TARGET/etrip"
echo "== ETRIP fixtures =="
fetch_first_of "$TARGET/etrip/etrip.tex" \
  "$TL/texk/web2c/etexdir/etrip/etrip.tex" || true
fetch_first_of "$TARGET/etrip/etrip.log" \
  "$TL/texk/web2c/etexdir/etrip/etrip.log" || true

echo "---- fixtures ----"
for dir in trip etrip; do
  echo "[$dir]"
  ls -la "$TARGET/$dir"
done

trip_ok=1
[ -s "$TARGET/trip/trip.tex" ] && [ -s "$TARGET/trip/trip.typ" ] && [ -s "$TARGET/trip/trip.log" ] || trip_ok=0
etrip_ok=1
[ -s "$TARGET/etrip/etrip.tex" ] && [ -s "$TARGET/etrip/etrip.log" ] || etrip_ok=0

[ "$trip_ok" = 1 ] && echo "TRIP fixtures 就绪。" || echo "TRIP fixtures 不完整。" >&2
[ "$etrip_ok" = 1 ] && echo "ETRIP fixtures 就绪。" || echo "ETRIP fixtures 不完整。" >&2
