# 今日四刀测试补强（2026-10-23）

本刀只增加测试、战报与进度结论；未修改引擎语义。GT 使用本机真 pdfTeX 1.40.29，原始探针和输出保留在 `/tmp/ntex-test-hardening-gt/`。

| 新增测试 | GT 取证输出 | 锁定内容 |
|---|---|---|
| `meaning_long_prefix_matches_pdftex_and_preserves_patchability` 新断言 | `prefix.tex` / `prefix.log`（MEANING、PERMUTED） | 三前缀定义顺序变化后，meaning 仍按 protected、long、outer 输出；已有原顺序断言继续保留。 |
| `show_and_tracingassigns_print_all_macro_prefixes` | `prefix.tex` / `prefix.log`（`> \f=`、`{into \g=}`） | `show_meaning` 与 `slot_display` 两条独立显示路径。 |
| `character_alias_direct_and_noexpand_boundary` | `alias.tex` / `alias.log`（SELF、FROZEN_CHARS、DIRECT、LITERAL、FROZEN_SELF、CSALIAS） | 字符别名直接参与 `\ifx`/`\if`、冻结后仍为字符；宏别名冻结后仍走哨兵。 |
| `character_alias_group_restore_and_global_override` | `alias.tex` / `alias.log`（LOCAL、RESTORED、GLOBAL） | 局部 `\let` 出组还原与 `\global\let` 保留。 |
| `ctex_fontset_none_currently_still_selects_dvi_branch` | `ctex-fontset-none.tex` / `ctex-fontset-none.log` | 当前扫描器对 `fontset=none` 仍选 DVI；窄回归现状。 |
| `ctex_class_in_top_level_input_should_select_dvi_branch`（ignored） | `ctex-wrapper.tex`、`body.tex` / `ctex-wrapper.log` | 真 pdfTeX 可从顶层 `\input` 载入 ctexart；当前快扫漏检，期望断言留待另刀启用。 |
| `math_accent_skew_shift_matches_pdftex` | `accent.tex` / `accent.log`（两条 shifted） | `\hat{a}` 0.14294pt≈9368sp、`\hat{abc}` 4.45255pt≈291802sp；允许 1sp。 |

判别力：前缀断言与 3fd1477 修前输出对照，旧 `meaning_text`、`show_meaning`、`slot_display` 均漏 long/outer，三前缀期望必红（见 `/home/ubuntu/ntex-biblatex-review.md` 第四节的修前实跑）。字符别名的 `FROZEN_CHARS=F` 与 5184b3e 修前 `get_x_char_operand` 对照：旧分支把两个字符别名都归为 relax/256，结果是 T；临时屏蔽修复分支的定向测试日志见 `alias-mutation.log`。改动已还原。显式运行 ignored 用例，`ctex_needs_dvi_output(wrapper)` 如预期变红，见 `ctex-ignored-proof.log`。

门禁原始输出：`core-tests.log`、`layout-tests.log`、`dvi-tests.log`、`instrument.log`；TRIP 的 `trip-build/construct.log` 和 `tripos-run/tripos.log`。core 586 绿、layout 336 绿、DVI CLI 3 绿/1 ignored、仪器 14/14；trip 构造 177B，tripos 12 页/750B、SHA-256 `574045fbe51f4ec48f8de7878d5bd559e5a301b3ce5f5a5afe347222ffe9c150`。

## 发现区

未发现新的引擎语义 bug。顶层 `\input` 漏检和 `fontset=none` 的现行分支已按任务要求记录，留给单独修复刀。
