#!/usr/bin/env bash
# expl3 条件分支探针（一刀定性）：expl3 `:TF` 是否只执行选中支？
#
# 用法：bash scripts/expl3-cond-probe.sh
#
# 依赖 /tmp/l3kernel-tests（scripts/lvt-run.py --fetch 抓取）。
# 复用 lvt-run.py 的 harness 机制（shim + regression-test.tex）。

set -uo pipefail
REPO=/home/ubuntu/NTex
NTEX=$REPO/target/debug/ntex-dvi
SHIM=$REPO/scripts/lvt/lvt-shim.tex
TF=/tmp/l3kernel-tests/testfiles
PROBE=$REPO/probes/expl3-conditional-branch.tex

[ -x "$NTEX" ] || { echo "缺 $NTEX，先 cargo build -p ntex-dvi"; exit 1; }
[ -d "$TF" ]   || { echo "缺 $TF，先 python3 scripts/lvt-run.py --fetch"; exit 1; }

WD=$(mktemp -d /tmp/expl3cond-XXXX)
trap 'rm -rf "$WD"' EXIT
cp "$TF"/regression-test.tex "$TF"/regression-test.cfg "$WD"/ 2>/dev/null
cp "$SHIM" "$WD"/lvt-shim.tex
cp "$PROBE" "$WD"/t.lvt
printf '\\def\\LVTFILE{t.lvt}\n\\input %s/lvt-shim\n' "$WD" > "$WD"/run.tex

export NTEX_TFM_DIR=${NTEX_TFM_DIR:-$HOME/.ntex-fonts}
OUT=$("$NTEX" --input-path "$WD" "$WD"/run.tex 2>&1)

echo "──── 探针输出（关键行）────"
echo "$OUT" | grep -aE "^===|TRUE-executed|FALSE-executed|^T$|^F$|GOT-TRUE|GOT-FALSE|TRUEBRANCH|FALSEBRANCH" 
echo
echo "──── 判读 ────"
chk() { # $1=label $2=must_only $3=must_absent
  if echo "$OUT" | grep -qa "$2" && ! echo "$OUT" | grep -qa "$3"; then
    echo "  ✓ $1"
  else
    echo "  ✗ $1  —— 两分支都执行（缺陷）"
  fi
}
chk "A 裸 TeX \\ifnum 只走 true 支" "TRUE-executed" "FALSE-executed"
chk "B \\bool_if:NTF 只走选中支"     "^T$"          "^F$"
chk "C \\int_compare:nNnTF 只走 false 支" "GOT-FALSE" "GOT-TRUE"
chk "D \\sys_if_engine_luatex:TF 只走 false 支" "FALSEBRANCH" "TRUEBRANCH"
