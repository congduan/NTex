# ctexart mark 文本泄漏：WIP 诊断（2026-11-03）

**状态：未闭环，不提交。** 本轮修通 mark token 保真后，`ctex-mark` 泄漏消失；`ctexart-zh` 转为执行页眉中的 `\MakeUppercase`，暴露独立的 CJK active UTF-8 展开失败，尚未达到零错与 PDF 验收。工作树保留 WIP。

## 根因链

- `reference/tex.web` 的 `make_mark`（约 L21237）为 `scan_toks(false,true)`：扫描时展开，产物仍是带 catcode、控制序列与组界的 token 列表。
- 原引擎 `expand/primitive_align.rs` 用 `scan_group_contents(None)`（体内不展开），随后 `expand_to_string` 把 mark 列表压成 `String`；layout 的 marks 状态亦为 `String`；`expand/expr.rs`、`primitive_align.rs` 查询时逐字节重造 catcode 12 字符。因此 LaTeX 内核 mark 值 `\__mark_value:nn{...}{...}` 在页眉从控制序列退化为可见文字。`ctexart` 默认页眉查询 marks，article 判别探针没有走同一页眉通路。
- WIP 修改：mark 使用 `scan_left_brace` + 展开体扫描，按 token 储存；sink、页状态、节点和 split marks 传递 token；六类查询按原 token 回流。showbox 的文字仅作诊断投影。引擎内没有按具体 mark csname 特判。
- 新暴露阻塞：`\MakeUppercase[]{中}` 在 `ctexart` 下**不经过任何 mark**即报 `Missing number`×2、`File ended while scanning definition`×3，未产出页；`\edef\myresult{\text_expand:n{中}}` 也同类失败，而直接 `\edef\myresult{中}` 能得到 `\CTEX@char@nnn {228}{184}{173}`。故 `ctexart-zh` 的 6 错不是 mark token 丢失，而是此前字符串化让页眉宏根本没有执行。下一步应沿 `l3text` 的 `text_expand:n` × CJK.sty active 字节宏 × 条件/数字扫描通路做 pdfTeX 对拍；不得绕开 `\MakeUppercase` 或特判 mark 值。

## 探针配方与原始输出

在 `/home/ubuntu/NTex`：`CARGO_TARGET_DIR=/tmp/ntex-cron-target cargo build -q -p ntex-dvi`，然后复制二进制到 `target/debug/ntex-dvi`。CLI 探针均在 `/tmp/deep-probe`；格式为 `assets/fmt/latex.fmt`。

`bash /tmp/deep-probe/run3.sh`（当前 WIP 二进制）：

```text
== art-mark rc=0 errs=0
X text.

1

== ctex-mark rc=0 errs=0
右

1
中文正文。

== art-ctex rc=0 errs=0
Plain body.

1

My Sect
Section text.
```

同一份 `ctex-mark.tex` 用 `PATH=$HOME/.local/bin:$PATH xelatex -interaction=nonstopmode`，再 `pdftotext`，实测也是：

```text
右

1
中文正文。
```

因此简报中“GT 只有 `1/中文正文。`”被本轮独立实测证伪；显式 `\markboth{左}{右}` 的页眉含“右”，WIP 输出与 GT 文本同构。

`ctexart-zh.tex`：

```text
rc=1 errors=6
! Unbalanced output routine.
! Missing number, treated as zero.
! Missing number, treated as zero.
! File ended while scanning definition.
! File ended while scanning definition.
! File ended while scanning definition.
未产出页面（源码缺少 \shipout）
```

隔离探针 `/tmp/deep-probe/case-cjk.tex` 为 `ctexart` 下直接 `\MakeUppercase[]{第一节}`，不含 mark，仍是 `Missing number`×2 + `File ended while scanning definition`×3，未产出页。`/tmp/deep-probe/text-expand-cjk.tex` 的 `\edef\myresult{\text_expand:n{中}}` 同类失败；pdfTeX 对拍（`fontset=none`）显示 `\myresult` 为 `\CTEX@char@nnn@n {{228}{184}{173}}`，无同类错误。

为排除字节码分叉，临时将 `Expander::new` 改成 interpreter 构建独立二进制 `/tmp/ntex-mark-interpreter-dvi`，同一 `case-cjk.tex` 仍为 `rc=1 errors=5`、同样两类错误；随后恢复源码并重建正常二进制。故该阻塞不局限于字节码通路。

再缩小：article 下 `\edef\R{\text_expand:n{\iftrue A\fi}}\show\R`（`/tmp/deep-probe/tx-truefi.tex`）已是 `errors=3`、无页；pdfTeX 同源输出 `> \R=macro:`、`->A.`，零错。真/假分支及有无 `\else` 四案全同。`NTEX_COND_TRACE=1` 在此看到退出时 6 层未闭条件，均来自同一行，故下一步可先修 `text_expand:n` 的条件展开时序，再回测 CJK，不必从 UTF-8 字节宏开查。

`art-sec.tex` 复跑：`已写出 art-sec.dvi（354 字节，1 页，43 字体）`，`pdftotext` 仍含 `Plain start. / 1 / My Section / Section body text. / 1`，零错误。

## 门禁与回归锁

- `CARGO_TARGET_DIR=/tmp/ntex-cron-target cargo test -q -p ntex-core -p ntex-layout`：`606 passed; 0 failed; 2 ignored`（core）；`360 passed; 0 failed; 3 ignored`（layout）；其余测试目标分别 `4/4`、`10/10`、`4/4`（1 ignored）、`3/3`。
- 新 `mark_query_keeps_control_sequence_tokens_out_of_text`：正常 `1 passed`；临时恢复旧的 String→catcode-12 查询后，原始失败行为 `left: "\\payload " / right: "SAFE"`，恢复修复后 `1 passed`。
- `CARGO_TARGET_DIR=/tmp/ntex-cron-target cargo clippy -q -p ntex-core -p ntex-layout -p ntex-dvi --all-targets -- -D warnings`：退出 0、无输出。
- `bash scripts/asset-load-audit.sh /tmp/ntex-cron-target/debug/ntex-dvi`：`=== 汇总：PASS=27 ENGINE-GATE=3 ERROR=0 TIMEOUT=0 （共 30 项）===`。此审计及 ctex-mark 均从发行 `latex.fmt` 快路径启动，格式可用，未触碰 `assets/fmt`。
- `/tmp/ntex-mark-trip`：`trip.tex --no-plain --dump trip.fmt` → `已写出 trip.fmt（2247214 字节，格式快照）`；`tripos.tex --fmt trip.fmt` → `已写出 tripos.dvi（750 字节，12 页，5 字体）`，`rg -c '^!' tripos.log` 无匹配；SHA-256 `574045fbe51f4ec48f8de7878d5bd559e5a301b3ce5f5a5afe347222ffe9c150`，与仓库归档基线相同。
- 已运行 `cargo fmt --all`；最终 WIP 收尾仍需再次执行格式、测试与 clippy（当前各项测试已针对本轮语义改动通过）。

## 下一步

1. 先用 `article` 的 `tx-truefi.tex` 对拍 `\text_expand:n` 内 `\iftrue…\fi` 的展开时序：检查 `\exp:w`/`\expandafter` 处理条件后的栈收口及 numeric scanner 取 token 时机；恢复到 pdfTeX 的零错与 `\R=macro:->A`。再回测 CJK active 字节宏。
2. 让直接 `\MakeUppercase[]{第一节}` 零错，再跑 `ctexart-zh.tex` 的 DVI/PDF 和文本层对拍。
3. 达成全部验收后才提交；本轮不 push，也不提交半成品。
