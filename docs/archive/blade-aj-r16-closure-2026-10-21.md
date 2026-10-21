# 刀 AJ R16 收口战报：TRIP 嵌套显示数学组恢复语义（306a250）

日期：2026-10-21。commit `306a250`（Codex gpt-6-sol 一轮实施 + 主控收口验收）。
前序：R12（\lastkern 读值链，65bb62e）/ R13（discretionary 尾 kern，f915563）/
R14（组栈诊断，未提交，`/tmp/bladeAJ/r14-report.md`）/ R15（mode 分派 + vtop 组
保护 WIP，Codex 额度中断）/ R16（本刀，续接 R15 WIP 收口）。

## 靶

trip.tex L294-298「外层 display math → \vtop 组 → 内层 $$ display math」三层嵌套：
NTex 在内层 opening `$` 处把 vtop 误作未闭组回收（`Missing } inserted` →
`Too many }'s`），且 closing `$$` 在 mode 漂移后与 par_begin 回放臂自旋挂死
（watchdog 11.5s，`last_tok=J` / `\ifvmode`）。GT（Knuth tex，trip.log
L3744-3775）同一区段零组错配：`\ifvmode` 真支、`$` 在 internal vmode 下开段进
hmode、`}` 闭 vtop、`\hss` 回外层 display。

## 修法（crates/ntex-core/src/expand/mod.rs +107/−15）

1. **math shift 分派按 mode 三路**：`mode∈{1,4}`（垂直/internal 垂直）→
   `par_begin(true)` + everypar 帧回放（tex.web `back_input; new_graf(true)`），
   纯展开 sink 不换 mode 时就地处理防回放死循环；
2. **closing 裁决看组顶不看旗标**：`recover_groups_to_math_scope` 前置撤除，改
   `scope_stack.last() != Some(&true)` 才回收；祖先非数学组（如外层 display 内的
   vtop）归属外层公式不被误吞；
3. **显示数学收尾先恢复外层模式再检查配对 `$`**（mode=6 专用臂）：
   `sink.math_shift(true)` → `next_is_math_shift(true, closing=true)` 按 get_x_token
   展开条件与宏（tex.web after_math 语义；进入侧仍 get_token 不展开）→
   未配对报 `Display math should end with $$` 照收。

## 验收（原始输出 /tmp/bladeAJ/r16-*）

| 项 | 前（b51def0） | 后（306a250） |
|---|---|---|
| trip.tex 全本 | 9 页 / 149 错 / 组错配 3-4-11 | 9 页无挂死 / 组错配 **1-2-7** |
| Missing number | 13 | 13（GT 6，另一发散源，本刀不动） |
| tripos | 13 页 / 804B / 0 错 | **12 页 / 614B / 0 错**（GT 12 页） |
| 窄探针 mini.tex | Too many }'s | 0 错（`\tracinggroups` 三层嵌套同构） |
| ntex-core / layout | 563 / 318 | **565(+2 新锁) / 320** 全绿 |
| 宏包回归 | 13/13 | 13/13（热二进制复跑） |
| clippy / fmt | — | 0 / clean（主控复跑） |

回归锁 2 把（tests_macro.rs）：`math_shift_in_vtop_preserves_enclosing_display_and_box_scopes`、
`nested_display_closing_condition_sees_restored_vtop_mode`；Codex 已在去修复源码上
验证双红（r16-red-*.out）。

## Review 结论（必修项：无 → 放行）

Claude Code 外部 review 通道 300s 超时（exit 124 / 0B，与 R14 同型通道故障），
**按刀 AJ R11 先例由主控亲自完成 review**，缺席事实记入本战报：

- 语义逐条对照 tex.web（vmode 分派 / after_math 组顶裁决 / get_x_token vs
  get_token 两通道）——同构，无「守卫掩盖失同步」残留；
- 挂死面：收尾分支终止条件完备（EOF/`$`/不可展开三出口），无 panic 面；
- MSRV 1.80 / clippy -D warnings / fmt 全过；
- 基线零污染：改动收敛在嵌套显示数学路径。

## 残差与下一刀靶

1. **trip.tex 全本 9 页 vs GT 16 页**：Missing number 13 vs 6 仍为主发散源；
2. **tripos 页内容空**（NTex 每页 0B vs GT 21B+1 rule）：随 b51def0 三提交的
   基线漂移（R15 简报已登记「非本轮责任勿回退」），溯源至
   `apply_layout_defaults` 运行时覆盖快照 misc 槽（plan.md §0.1 第 7 条残留）；
3. `dvicmp.py`（scripts/，本轮入库）可逐页对比 DVI op 构成，已修 postamble
   prev_post 裸地址切片（注释内含 tex.web 指针语义实测口径）。
