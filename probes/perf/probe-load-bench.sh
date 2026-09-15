#!/usr/bin/env bash
# 第九刀 perf 探针（2026-09-16）：expl3 载入端到端墙钟基准 + 语义门。
#
# 靶子与结论见 docs/expl3-real-scoreboard.md 第九刀节；基准方法：
#   probes/perf/probe-load-bench.sh [binary]
#
# 做什么：
#   1. 从 fixtures/l3kernel/ + probes/fp-load/probe-load.tex 组装临时跑目录
#      （repo 内 fixture 与手工 /tmp/fp9 集逐字节一致，cmp 已验）；
#   2. /usr/bin/time -v 计墙钟 + 峰值内存；
#   3. 校验语义门（见下），过了打印一行 `bench:` 摘要。
#
# 语义门（第九刀验收口径，任何一项不满足即 rc=1）：
#   - 退出码 = 0
#   - stderr 转录含 [LOAD-DONE]（载入走通的唯一信号，\message 发出）
#   - stderr 的 `!` 行数 = 3（第八刀已定残差：Forbidden ^^L ×1 +
#     \unhbox 簇 ×2——Missing number + Incompatible list can't be unboxed；
#     多一条都算新回退）
#   - stdout 落盘行 = DVI 205 字节 / 1 页 / 77 字体
#
# 注意：
#   - NTex 转录走 **stderr**（stdout 只有末行落盘信息），别 grep 锁文件。
#   - probe-load.tex 里 \input 走 --input-path 相对查找，跑目录须扁平拷贝。
#   - 墙钟对机器敏感，跨机比较无意义；同机 before/after 才算数。

set -uo pipefail

BIN=${1:-${NTEX_BIN:-target/debug/ntex-dvi}}
FIX=${NTEX_L3K_FIXTURES:-fixtures/l3kernel}
PROBE=probes/fp-load/probe-load.tex

[ -x "$BIN" ] || { echo "bench: 二进制不可执行: $BIN" >&2; exit 2; }
[ -f "$FIX/expl3-code.tex" ] || { echo "bench: 缺 $FIX/expl3-code.tex" >&2; exit 2; }

RUN=$(mktemp -d)
trap 'rm -rf "$RUN"' EXIT
cp "$FIX"/* "$RUN"/
cp "$PROBE" "$RUN"/

/usr/bin/time -v "$BIN" --input-path "$RUN" "$RUN/probe-load.tex" \
    >"$RUN/out.log" 2>"$RUN/all.log"
rc=$?

elapsed=$(grep -oP 'Elapsed \(wall clock\) time.*: \K.*' "$RUN/all.log")
maxrss=$(grep -oP 'Maximum resident set size .*: \K\d+' "$RUN/all.log")
load_done=$(grep -c '\[LOAD-DONE\]' "$RUN/all.log" || true)
err_lines=$(grep -c '^!' "$RUN/all.log" || true)
dvi_line=$(grep -oP '已写出.*' "$RUN/out.log" | head -1)
dvi_bytes=$(grep -oP '\d+(?= 字节)' "$RUN/out.log" | head -1)
dvi_pages=$(grep -oP '\d+(?= 页)' "$RUN/out.log" | head -1)
dvi_fonts=$(grep -oP '\d+(?= 字体)' "$RUN/out.log" | head -1)

fail=0
[ "$rc" -eq 0 ] || { echo "bench: FAIL rc=$rc（期望 0）"; fail=1; }
[ "$load_done" -eq 1 ] || { echo "bench: FAIL [LOAD-DONE] 次数=$load_done（期望 1）"; fail=1; }
[ "$err_lines" -eq 3 ] || {
    echo "bench: FAIL 载入期 '!' 行=$err_lines（期望 3＝第八刀残差）"
    grep -n '^!' "$RUN/all.log" | sed 's/^/  /'
    fail=1
}
[ "$dvi_bytes:$dvi_pages:$dvi_fonts" = "205:1:77" ] || {
    echo "bench: FAIL DVI 落盘='$dvi_line'（期望 205 字节 / 1 页 / 77 字体）"
    fail=1
}

if [ "$fail" -eq 0 ]; then
    echo "bench: PASS elapsed=$elapsed maxrss=${maxrss}KB err=3(残差) dvi=$dvi_line"
    exit 0
fi
exit 1
