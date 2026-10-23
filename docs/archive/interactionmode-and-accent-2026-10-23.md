# 交互旗标与 accent 非零 kern（2026-10-23）

基线 `origin/main=3bec8be`；本刀取证依据 `/home/ubuntu/ntex-testhard-review.md` 的 F4/F1。工作区原有 `docs/pkg-regression-baseline.md` 改动未碰；stash@{0}–{3} 未动。

## F4：预存 CLI 红灯

开工原样复跑 `transcript_cli::interaction_flag_sets_error_mode`，确定性失败。旧测试写入 `\error`，但当前 CLI 的 plain 环境并未定义该控制序列，转录实际是 `! Undefined control sequence.`，因此原断言未能判定 `--interaction` 是否落到参数。`main.rs:90/159` 的两种参数语法均能解析，`Typesetter::set_interaction_mode` 写入 core 的 `misc[19]`。plain 路径实际读回为 1。

真实断点在 `main.rs`：原先 `set_interaction_mode` 位于 `ts.import_state(state)` 之前；显式或自动载入 `.fmt` 时，快照中的 `params.misc[19]` 覆盖 CLI 设置。现把设置移到全部格式导入之后、`typeset_dvi` 之前。这样启动旗标是本次作业初值，用户源码里的显式赋值仍可覆盖。未改 fmt 数据结构或 codec，无需 bump VERSION 或重生成资产。

CLI 端到端锁现用 `\message{MODE=\the\interactionmode}` 和 `\errmessage{boom}`：等号语法验证 `nonstopmode→1` 及不进入交互停等，空格语法验证 `batchmode→0`。另加载发行 `latex.fmt`，断言 `FMTMODE=1`，专门锁住导入后的优先级。M1-13 `errhelp` 两例单独复跑也通过。

## F1：数学重音位移

在既有 `math_accent_skew_shift_matches_pdftex` 中补 `\hat{f}` 与 `\hat{A}`，沿用 cmr10/cmmi10、`\skewchar\teni=127` 配置。pdfTeX GT 在 `/tmp/review-gt-verify/skew-f.log` 与 `skew-A.log` 分别打印 `shifted 2.15285`、`2.63893`，对应 141089sp、172945sp；关闭 skew 后分别为 0.48613pt、1.25pt。新用例测试通过，能识别 `lig_kern` 查找退化为零。既有 `abc` 1sp `half()` 舍入差仍保留公差，并修正注释说明。

## 验收

所有构建与测试使用 `CARGO_TARGET_DIR=/tmp/biblatex-target`。宏包脚本固定读取仓库 `target/debug/ntex-dvi`，运行前将当前构建二进制复制到该路径，脚本以 `--no-report` 运行，未写基线文档。

| 门禁 | 结果 | 原始记录 |
|---|---|---|
| core/layout lib | 586/336 passed | `/tmp/ntex-interaction-core-layout.log` |
| DVI transcript CLI | 6 passed、1 ignored | `/tmp/ntex-interaction-cli.log` |
| M1-13 errhelp | 2 passed | `/tmp/ntex-interaction-errhelp.log` |
| instrument-check | 14/14 | `/tmp/ntex-interaction-instrument.log` |
| TRIP `--no-plain --dump trip.fmt` | 构造 177B；tripos 12 页/750B/0 错，SHA-256 `574045fbe51f4ec48f8de7878d5bd559e5a301b3ce5f5a5afe347222ffe9c150` | `/tmp/ntex-interaction-trip/{trip-build,tripos}.log` |
| 宏包回归 `--no-report` | 13/13 PASS，footmisc 负例 PASS | `/tmp/ntex-interaction-pkgreg.log` |

本刀运行 `cargo fmt --all -- --check` 时发现 `main.rs` 另有预存格式差异；本次新增的 CLI 测试文件已单独 `rustfmt`，`git diff --check` 通过。
