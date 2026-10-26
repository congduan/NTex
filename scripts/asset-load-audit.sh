#!/bin/bash
# 资产审计 CLI 矩阵——30 个宏包/文档类入口最小加载+出页（发行资产树 latex.fmt）。
# 对照 docs/archive/default-tex-assets-2026-10-20.md 的 WASM 审计表（CLI 通道版本）。
# 判据：PASS = rc==0 且 0 错且 DVI 落盘；ENGINE-GATE = 首错是 XeTeX/LuaTeX 引擎要求
# （pdfTeX 语义下 by-design 拒绝，与真 pdfLaTeX 同构）；否则 ERROR。
# 用法：bash scripts/asset-load-audit.sh [二进制路径] [输出目录]
set -u
REPO="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${1:-$REPO/target/debug/ntex-dvi}"
# cd 到输出目录后相对路径失效（2026-10-27 实测 30 项全 ERROR 假信号），统一转绝对路径。
case "$BIN" in /*) ;; *) BIN="$PWD/${BIN#./}" ;; esac
OUT="${2:-/tmp/ntex-asset-audit}"
FMT="$REPO/assets/fmt/latex.fmt"
mkdir -p "$OUT"
cd "$OUT"

declare -a PKGS=(amsthm mathtools siunitx enumitem fancyhdr titlesec setspace parskip
  float caption subcaption makecell tabularray hyperref cleveref natbib biblatex
  listings fancyvrb minted tikz pgfplots ctex xeCJK babel fontspec unicode-math
  standalone)
declare -a CLASSES=(beamer ctexart)

pass=0; gate=0; err=0; to=0
echo "=== 资产加载审计（CLI 通道，二进制=$BIN，fmt=$(md5sum "$FMT" | cut -c1-8)）==="
printf "%-14s %-11s %-6s %s\n" "入口" "结果" "页数" "首错/备注"

run_one() {
  local name="$1" src="$2"
  : > "$name.aux"
  rm -f "$name.dvi"
  timeout 120 "$BIN" "$name.tex" --fmt "$FMT" > "${name}_tr.txt" 2>&1
  local rc=$?
  local errs; errs=$(grep -ac '^!' "${name}_tr.txt")
  local pages=""
  [ -f "$name.dvi" ] && pages=$(grep -aoE '[0-9]+ 字节' "${name}_tr.txt" | head -1)
  local first; first=$(grep -a '^!' "${name}_tr.txt" | head -1)
  local first_show; first_show=$(printf '%s' "$first" | cut -c1-70)
  local verdict
  if [ $rc -eq 124 ]; then verdict="TIMEOUT"; to=$((to+1));
  elif [ "$rc" -eq 0 ] && [ "$errs" -eq 0 ] && [ -f "$name.dvi" ]; then verdict="PASS"; pass=$((pass+1));
  elif echo "$first" | grep -qE "(requires? (either )?(XeTeX|LuaTeX)|Cannot be run with pdftex)"; then
    verdict="ENGINE-GATE"; gate=$((gate+1));
  else verdict="ERROR"; err=$((err+1)); fi
  printf "%-14s %-11s %-6s %s\n" "$name" "$verdict" "$pages" "$first_show"
}

for p in "${PKGS[@]}"; do
  printf '\\documentclass{article}\n\\usepackage{%s}\n\\begin{document}\nAsset probe.\n\\end{document}\n' "$p" > "$p.tex"
  run_one "$p" "$p.tex"
done
for c in "${CLASSES[@]}"; do
  printf '\\documentclass{%s}\n\\begin{document}\nAsset probe.\n\\end{document}\n' "$c" > "$c.tex"
  run_one "$c" "$c.tex"
done

echo "=== 汇总：PASS=$pass ENGINE-GATE=$gate ERROR=$err TIMEOUT=$to （共 $((pass+gate+err+to)) 项）==="
