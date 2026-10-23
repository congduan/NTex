# SavedValue 与寄存器赋值路径重构（2026-10-23）

基线 `a4b4815`。本刀严格限定在 `expand` 的保存项、赋值流水线和参数映射；`docs/pkg-regression-baseline.md` 开工前已有未提交改动，未触碰；`input.rs` 和 stash@{0}–{3} 未触碰。

## 分步提交

| 提交 | 内容 | 核查 |
|---|---|---|
| `e16bc56` | 五种 `SavedValue` 寄存器项并为 `Register { kind, idx, prev: RegValue, retained }`；保存、失效、恢复、retaining 转录归为一条路径 | core 580、layout 334 全绿 |
| `8fc62b1` | 五个 `assign_*` 进入统一 `assign_register`，保留 muskip 的赋值前 sink 通知时序；`misc[5]` 裸读收为 `tracing_assigns()` | core 580、layout 334 全绿 |
| `761ad90` | 唯一行为变更：`\toks` 赋值补 `\tracingassigns` 转录，测试锁定 `into`、`reassigning`、全局赋值 | core 581、layout 334 全绿 |
| `4482097` | `param_kind_of` 作为 `Primitive→ParamKind` 唯一映射，删除 `save.rs` 的重复表 | core 581、layout 334 全绿 |

指定 retaining 测试中 `count_local_and_global_scoping`、`global_prefix_through_conditional_assign_target`、`named_interline_glue_counts_toward_page_goal` 在 R1 全量日志中均为 `ok`。`global_param_survives_group_after_local_touch` 这个精确测试名在当前仓库中不存在，故不虚报已执行。四轮完整日志在 `/tmp/refactor-{r1,r2,toks,r3}-test.log`；代码相对基线净减少约 153 行。`cargo clippy -p ntex-core -p ntex-layout --all-targets -- -D warnings` 通过，记录 `/tmp/bladeAJ/refactor-clippy.out`。

## 全量实测

当前 release 二进制由 `CARGO_TARGET_DIR=/tmp/refactor-target cargo build --release -p ntex-dvi` 构建。逐项结果与原始输出在 `/tmp/bladeAJ/refactor-final/`；两遍文档由 `/tmp/bladeAJ/refactor-verify.py` 顺序执行，汇总 `summary.json`。

| 项目 | 本刀实测 | 判定 |
|---|---:|---|
| TRIP 构造 | 177B、1 页 | 与基线一致；构造期 5 条预期诊断 |
| tripos | 750B、12 页、0 错 | 与任务门槛一致 |
| Transformer 真文档 `/tmp/real-doc/technical-report.tex` | 53 页、0 错 | 与当前基线一致 |
| T1 amsldoc | 44 页、1 错（`Multicol Error`） | 任务容许的 44 页/2 错以内 |
| T2 testmath | 40 页、0 错 | 与任务门槛一致 |
| T3 fntguide | 49 页、0 错 | `plan.md` 开工最新基线为 49 页；任务所列 50 页是更早口径 |
| 宏包矩阵 | 13/13；footmisc 负例 PASS | 全绿，`--no-report` 未改用户基线文档 |
| Transformer PDF 页底 | 53 页中 0 页文字底边 >752pt；最大 701.43pt | 无回退 |
| R18 公式 | 10 处均 x0=203.2pt、编号 x1=484.4pt | 无回退 |

PDF 扫描脚本和原始行见 `/tmp/bladeAJ/refactor-geometry.py`、`refactor-final/geometry.out`。第一次验证脚本漏放 `refs.bbl`，且误写 `makeindex` 路径，使 Transformer 返回缺 `refs.bbl`、三文档跑到 T1 第一遍便中断；修正工作目录和命令后完整重跑，上表仅取成功的第二次记录。

为直接核查零语义变更，另从开工提交 `a4b4815` 在隔离工作树构建 release 二进制，并在同一工作目录、同一输入/辅助文件、同一输出路径下与本刀二进制相继运行。tripos、Transformer、T1、T2、T3 五份 DVI 的 SHA-256 逐份相等，错误数也分别相等（0/0/1/0/0）；原始判据、两侧 DVI 与输出见 `/tmp/bladeAJ/refactor-final/differential/`、`differential.out`。这一对拍排除了首轮文档数字与旧战报不同是由本刀代码引起；唯一预期差异仍是单独锁住的 `\toks` 赋值 tracing 行。

Claude review 按要求只试一次：`timeout 45 claude --model sonnet -p`，退出码 124，stdout/stderr 均 0B。短简报与原始记录为 `/tmp/bladeAJ/refactor-review-brief.md`、`refactor-claude-review.{out,err,exit}`；外部审查无结论，以四轮测试、clippy 和全量实测为依据。

## 后续

R4 的 `expand/mod.rs`、`typeset/sink.rs` 拆分未执行。本刀已经完成优先的 R1–R3 和全量门禁；R4 需逐文件移动并在每个文件后完整复测，单列下刀以保留易二分的提交边界。T1 的 `Multicol Error`、T3 与 GT 的页差属于原有语义战役，本刀未修改。
