# l3 消息机 `\iow_wrap` 扫描中断修复（2026-10-23）

## 结论与复现

`origin/main` 与开工 HEAD 均为 `bb15860`。`xecjk-critical-wrap.tex` 在修复前报
`File ended while scanning use of \__iow_wrap_next_line:w`；真 pdfLaTeX 将同一
critical 消息完整排版后给出预期的 xeCJK/XeTeX 能力诊断。修复后 critical-wrap
只报这 **1 条预期 critical 消息**，没有包装器或扫描器错误；direct-wrap 仍以
0 错、1 页 DVI 结束。原样 xeCJK 探针也越过消息包装，先报告预期 critical 消息，
其后仍有 XeTeX/UTF-8 能力边界错误，不属于本刀闭环目标。

独立于消息机的最小 l3 调用序列是 `\int_set:Nn \l_iow_line_count_int {76}`
接一条含 `Critical Package xeCJK Error:` 前缀、`\\` 换行和较长第二行的
`\iow_wrap:nenN {...} { } { } \iow_log:n`。同样文本在默认 78 列或删去
换行时正常；76 列、前缀和换行的组合在旧引擎中重现扫描 EOF。

```tex
\documentclass{article}
\ExplSyntaxOn
\int_set:Nn \l_iow_line_count_int { 76 }
\iow_wrap:nenN { Critical~Package~xeCJK~Error:~The~xeCJK~package~requires~XeTeX~to~function.\\\\ You~must~change~your~typesetting~engine~to~"xelatex"~instead~of~plain~"latex"~or~"pdflatex"~or~"lualatex".\\ Loading~xeCJK~will~abort! } { } { } \iow_log:n
\ExplSyntaxOff
\begin{document}OK\end{document}
```

## 实际根因

被怀疑丢失的 `\s__iow_stop` **已产生**，其身份和 catcode 没有失配。
插桩实测首个 `\__iow_wrap_next_line:w` 成功扫到该定界符。下一轮
`\__iow_wrap_line_end:NnnnnnnnN` 的 `#9` 是
`\__iow_wrap_end_chunk:w`，应走非空格分支；NTex 却让
`\token_if_eq_charcode:NNTF \c_space_token #9` 判真，遂提前调用
`\__iow_wrap_line_end:nw` 并吃掉尾哨兵。后续 `next_line` 从文本中段继续扫描，
越过包装宏、消息机尾部直至文件 EOF。

真正偏差在 `expand/cond.rs::get_x_char_operand` 的 `\noexpand` 臂：
它把所有冻结的非 active 控制序列归成 relax/256 哨兵。
`\c_space_token` 是以 `\let` 指向空格字符的控制序列，本身不可展开；
`\noexpand` 后在 `\if`/`\ifcat` 中仍须保留字符码和 catcode。
pdfLaTeX 裸形对照：`\token_if_eq_charcode:NNTF \c_space_token
\__iow_test_end:w` 得 `F`，旧 NTex 得 `T`，修复后得 `F`。
另以 `\let\bg={`、`\let\dot=.` 对照 pdfTeX，分别锁定 char alias
与宏比较为假、other 字符 alias 与字面字符比较为真。

修复仅在 `noexpand` 操作数分支对 `EqSlot::Char` 返回原字符含义。
`expand/tests_cond.rs::noexpand_char_alias_keeps_character_in_if_operands`
是去修复必红的 core 单测；`scripts/xecjk-survey.sh` 现在断言三探针结果，
旧引擎的 critical-wrap 扫描 EOF 会使脚本失败。未改 fmt codec 或发行 fmt。

## 门禁与原始输出

所有命令使用 `CARGO_TARGET_DIR=/tmp/biblatex-target`。宏包脚本固定读取
`target/debug/ntex-dvi`，已从该 target 复制同一新构建二进制；运行时加
`--no-report`，未写动在途的 `docs/pkg-regression-baseline.md`。

| 门禁 | 实测 | 原始输出 |
|---|---|---|
| 三探针 `scripts/xecjk-survey.sh` | 原样 xeCJK exit 1、5 条后续边界诊断；critical-wrap exit 1、**仅预期 critical 1 条**；direct-wrap exit 0、0 错、DVI 有效；脚本断言 exit 0 | `/tmp/iow-survey-locked.out`、`/tmp/iow-survey-locked/*.log` |
| `cargo test -p ntex-core -p ntex-layout` | core **583 passed**（新增 1 锁）、layout **335 passed**，其余集成测试全绿 | `/tmp/iow-gates/core-layout.log` |
| `make instrument-check` | **14/14** | `/tmp/iow-gates/instrument.log` |
| TRIP 构造 `--no-plain --dump` | **177B**、1 页 | `/tmp/iow-gates/trip/trip-build.log` |
| tripos 载入新生成 fmt | **12 页、750B、0 错**；DVI SHA-256 `574045fb…` 与旧基线逐字节一致 | `/tmp/iow-gates/tripos/tripos.log` |
| `scripts/pkg-regression.py --no-report` | **13/13 PASS**，footmisc 负例 PASS | `/tmp/iow-gates/pkg-regression.log` |
| T2 testmath 两遍、显式 latex.fmt | **40 页、114154B、0 错**；SHA-256 `520d1392…` 与旧基线逐字节一致 | `/tmp/iow-gates/t2/pass1.log`、`pass2.log` |
| T3 fntguide 两遍、显式 latex.fmt | **49 页、148528B、0 错**；SHA-256 `a11b97b…` 与旧 ctex 门禁的同口径产物逐字节一致 | `/tmp/iow-gates/t3/pass1.log`、`pass2.log` |

T3 的 148528B 与 biblatex 报告所列 148526B 相差 2B，原因是输入 `.toc`
状态不同：那次 pass2 从 3182B `.toc` 读取，本轮及 ctex 常规门禁从
3184B `.toc` 读取。本轮与 ctex 常规门禁的 DVI SHA-256 完全相同；
`.tex`/最终 `.aux`/最终 `.toc` 与 biblatex 运行目录逐字节相同。
`git diff --check`、`bash -n scripts/xecjk-survey.sh` 均通过。
全仓 `cargo fmt --check` 因既有 `crates/ntex-dvi/src/main.rs:632` 格式差异返回 1；
本刀未修改该文件，输出在 `/tmp/iow-gates/fmt-check.log`。
