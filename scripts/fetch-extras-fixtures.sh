#!/usr/bin/env bash
# fixtures/pdftex — NTex 补充对照 fixtures（与 fetch-trip-fixtures.sh 并列）
#
# 当前入库（2026-09-03）：
#   pdftex/expanded.tex  pdftex 官方 `\expanded` 原语 12 个端到端测试
#                         （David Carlisle / Bruno Le Floch, 2018, Public Domain）
#   pdftex/expanded.txt  与上面一对的 36 行期望输出基线（pdftex --recorder 格式）
#
# 抓取来源：TeX-Live GitHub `texlive-source@trunk/texk/web2c/pdftexdir/tests`
# （NTex 环境下国内 `mirrors.ctan.org` 镜像被劫持到被墙节点，raw github / jsdelivr 直连可用）

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${NTEX_FIXTURES_DIR:-$REPO_ROOT/fixtures}"
JS="https://cdn.jsdelivr.net/gh/TeX-Live/texlive-source@trunk"
RAW="https://raw.githubusercontent.com/TeX-Live/texlive-source/trunk"

mkdir -p "$TARGET/pdftex"
echo "== pdftex 补充对照（expanded 原语）=="

for entry in \
  "expanded.tex|pdftex expanded.tex|David Carlisle/Bruno Le Floch 2018 \\\\expanded 端到端测试" \
  "expanded.txt|expanded.txt 期望基线|pdftex --recorder 格式 36 行期望输出"; do
  IFS='|' read -r name label desc <<<"$entry"
  ok=0
  for base in "$JS" "$RAW"; do
    url="$base/texk/web2c/pdftexdir/tests/$name"
    if curl -fsSL --retry 2 --max-time 25 -o "$TARGET/pdftex/$name" "$url" 2>/dev/null; then
      size=$(wc -c <"$TARGET/pdftex/$name" | tr -d ' ')
      echo "  ✓ $name ($size 字节) [$desc]"
      ok=1; break
    fi
  done
  if [ "$ok" = 0 ]; then
    echo "  ✗ $name 拿不到（保留旧版本如有）"
  fi
done

echo
echo "---- pdftex 状态 ----"
ls -la "$TARGET/pdftex"
echo
echo "完成。"
