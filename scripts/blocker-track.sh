#!/usr/bin/env bash
# 阻塞点单调性看板：把 latex_probe 的输出变成「历史对比 + 单调性判定」。
#
# ## 为什么需要它
#
# 项目纪律（见 docs/archive/latex-feasibility.md §F）：
#   **进度判定用「阻塞点位置单调前移」，不用「跑完/错误计数」**
# —— 引擎的错误恢复会让「跑完」和错误计数全部失真。
#
# 但人工比对违背纪律且易漏。本脚本自动：
#   1. 跑 latex_probe 拿「最远到达行 + 首错签名」
#   2. 追加到历史 TSV
#   3. 与上一条对比，判定：单调前移 / 停滞 / **回退（告警）** / **签名突变（告警）**
#
# ## 用法
#
#     scripts/blocker-track.sh                 # 默认跑 /tmp/r27b/latex.ltx
#     scripts/blocker-track.sh path/to/x.ltx   # 指定源
#     scripts/blocker-track.sh --show          # 只看历史，不重跑
#
# 历史文件：docs/blocker-history.tsv（建议入库，作为战役仪表盘）
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HIST="$REPO/docs/blocker-history.tsv"
# 默认源优先仓库内 fixtures（2026-09-13 入库，防 /tmp/r27b 随整机重启丢失）；
# 显式传参或 /tmp 备料树存在时仍可覆盖（旧口径不变）。
SRC_DEFAULT="/tmp/r27b/latex.ltx"
if [[ ! -f "$SRC_DEFAULT" && -f "$REPO/fixtures/latex2e/latex.ltx" ]]; then
    SRC_DEFAULT="$REPO/fixtures/latex2e/latex.ltx"
fi
PROBE_LOG="$(mktemp -t blocker-probe-XXXX.log)"

if [[ "${1:-}" == "--show" ]]; then
    if [[ -f "$HIST" ]]; then
        column -t -s $'\t' "$HIST" 2>/dev/null || cat "$HIST"
    else
        echo "（无历史；先跑一次 scripts/blocker-track.sh）"
    fi
    exit 0
fi

SRC="${1:-$SRC_DEFAULT}"
if [[ ! -f "$SRC" ]]; then
    echo "❌ 找不到源：$SRC" >&2
    echo "   备料树重建见 docs/archive/latex-feasibility.md §E（/tmp/r27b 勿重下）" >&2
    exit 2
fi

export PATH="$HOME/.cargo/bin:$PATH"
export NTEX_TFM_DIR="${NTEX_TFM_DIR:-$HOME/.ntex-fonts}"

echo "== 跑 latex_probe（--initex）…"
# ⚠ latex_probe 的 `\input` 文件查找依赖 `--input-path`：源在 /tmp 备料树时
# 用源所在目录；在仓库 fixtures 时带上 fixtures/latex2e（2026-09-13 接线）。
SRC_DIR="$(dirname "$SRC")"
INPUT_PATH_ARGS=(--input-path "$SRC_DIR")
# fixtures 路线补兄弟目录：latex2e 的载入闭包要跨目录取 l3kernel 的
# expl3-code.tex / UnicodeData.txt 等件（2026-10-02：explN-code 找不到
# 假性回退 38446→200 的根因，缺件引擎早退伪影，非引擎回归）。
if [[ "$SRC" == "$REPO"/* ]]; then
    INPUT_PATH_ARGS+=(--input-path "$REPO/fixtures/l3kernel")
fi
timeout 600 cargo run -q --release -p ntex-test-support --example latex_probe -- \
    "$SRC" --initex "${INPUT_PATH_ARGS[@]}" > "$PROBE_LOG" 2>&1
probe_rc=$?

# ── 抽取信号 ────────────────────────────────────────────────────────────
# (a) 最远到达行：转录里最后一个 `l.NNNNN`（probe 转录行有 `| ` 前缀）
furthest=$(grep -aoE '(^|[[:space:]|])l\.[0-9]+' "$PROBE_LOG" \
    | grep -oE '[0-9]+' | sort -n | tail -1 || true)
furthest="${furthest:-0}"

# (b) 首错签名：第一条 `! ...` 错误（规范化：去数字、去具体标识符尾巴）
first_err=$(grep -am1 '^! ' "$PROBE_LOG" | sed -E 's/[0-9]+/N/g' | cut -c1-60 || true)
if [[ -z "$first_err" ]]; then
    # probe 的致命错误走 `== pass1 ERROR:` 行而非 `! `
    first_err=$(grep -am1 'pass1 ERROR' "$PROBE_LOG" \
        | sed -E 's/^== pass1 ERROR: //; s/[0-9]+/N/g' | cut -c1-60 || true)
fi
first_err="${first_err:-<none>}"

# (c) 终止方式：pass1 ERROR / dumped / 超时
if [[ $probe_rc -eq 124 ]]; then
    term="TIMEOUT"
elif grep -qa 'pass1 ERROR' "$PROBE_LOG"; then
    term="ERROR"
elif grep -qa 'dumped: true' "$PROBE_LOG"; then
    term="DUMPED"
else
    term="END"
fi

# (d) 栈超限专项（本战役的主要卡点形态）
stack_hit=$(grep -ac '输入栈超限' "$PROBE_LOG" || true)
stack_hit="${stack_hit:-0}"

ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
head_sha="$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo '?')"

echo
echo "== 本轮信号"
printf '   最远行        : %s\n' "$furthest"
printf '   首错签名      : %s\n' "$first_err"
printf '   终止方式      : %s\n' "$term"
printf '   栈超限命中    : %s\n' "$stack_hit"
printf '   HEAD          : %s\n' "$head_sha"

# ── 与上一条历史对比 ────────────────────────────────────────────────────
prev_line=""
[[ -f "$HIST" ]] && prev_line="$(tail -1 "$HIST")"

verdict="FIRST"
if [[ -n "$prev_line" ]]; then
    IFS=$'\t' read -r _p_ts _p_sha p_far p_err p_term p_stack <<< "$prev_line"
    prev_far="${p_far:-0}"
    echo
    echo "== 对比上一轮（$p_far 行 / ${p_err}）"
    if [[ "$furthest" -gt "$prev_far" ]]; then
        verdict="FORWARD"
        echo "   ✅ 单调前移（$prev_far → $furthest）——健康"
    elif [[ "$furthest" -lt "$prev_far" ]]; then
        verdict="REGRESSION"
        echo "   🔴 **回退**（$prev_far → $furthest）——告警：改动引入回归，先查 diff"
    else
        if [[ "$first_err" != "$p_err" ]]; then
            verdict="SIGMA-CHANGE"
            echo "   ⚠ **签名突变**（同位置但错误类型变了）"
            echo "      旧: $p_err"
            echo "      新: $first_err"
            echo "      提示：可能是「根因被推翻」或修复改判——勿当作停滞"
        else
            verdict="STALL"
            echo "   ⏸ 停滞（位置与签名均未变）"
        fi
    fi
fi

# ── 追加历史 ────────────────────────────────────────────────────────────
if [[ ! -f "$HIST" ]]; then
    printf 'ts\thead\tfurthest_line\tfirst_error\tterminal\tstack_overflow\tverdict\n' > "$HIST"
fi
printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$ts" "$head_sha" "$furthest" "$first_err" "$term" "$stack_hit" "$verdict" >> "$HIST"
echo
echo "== 已追加 → $HIST（verdict=$verdict）"
echo "   查看全部：scripts/blocker-track.sh --show"

rm -f "$PROBE_LOG"
# 回退与签名突变的非零退出，便于 CI/脚本捕获
case "$verdict" in
    REGRESSION|SIGMA-CHANGE) exit 1 ;;
    *) exit 0 ;;
esac
