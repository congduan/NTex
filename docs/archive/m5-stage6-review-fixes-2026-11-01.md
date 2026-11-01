# M5 阶段六 review 收口：P1-1 控制态中性口径 + error_anchor 生命周期根治（2026-11-01）

- 基线：`805a7f0`（第三刀 Rc 共享已合入）
- 提交：`f055f0c` fix: P1-1（CodeBuddy 实施 + 主控复测）→ `f4b1ae1` docs: → `5890b81` fix: error_anchor（CodeBuddy 实施 + 主控复测）→ `6ec2421` docs:
- 引擎分工：CodeBuddy 实施 → Codex review（P-with-fixes）→ CodeBuddy 修复 → 主控独立复测全链

## 一、P1-1：同步态复用回滚点 Rc 共享的控制态洞

**问题**（Codex review 发现，主控走读确认）：`snapshot_states_equal` 只比 ValueState + eqtb，
`ControlState` 不在比较面。`\tracingcommands` 只改 `shown_trace_mode` 的段被判中性 →
同步态复用跳过 `restore_to` → 活控制态停旧值，而回滚点仍共享旧 `record_post` →
下一轮 edit 回滚恢复出旧段的转录模式记忆，转录与全量不一致。

**修法**（简报方案 b）：`snapshot_states_equal` 增加控制态维度
（`ControlState::output_visible_equal`，checkpoint.rs），比较面 = `shown_trace_mode` /
`trace_suppress` / `trace_suppress_defer` / `output_trigger_line` / `output_prev_count` /
`err_snapshot` / `section_label` / `ended`。

**排除面与理由**（实施者实证修正 review 两处偏差）：
- `\show` 不改 `shown_trace_mode`（唯一写点 `trace_token_now`，mod.rs:1856）——review 的
  `\show` 例子不成立，探针改用 `\tracingcommands`；
- `last_tok` 每 token 必变，纳入即全段非中性（复用与投机全灭）；
- `error_anchor` 粘滞，纳入实测 bench 7.9x→3.1x——已排除，转根治（见下节）；
- 结构帧由 `boundary_is_clean` 闸挡。

**回归锁**：`trace_control_state_rollback_matches_full_transcript`（incremental_tests.rs，
两轮 edit：改正文触发同步态复用 → 改探针段触发回滚，断言探针转录尾块与全量逐字节一致）。
主控亲测 RED（摘控制态比较 → `{vertical mode: the letter z}` vs `{the letter z}` 漂移）→ GREEN。

## 二、error_anchor 生命周期根治（review 阻塞项）

**问题**（Codex review 发现 + 主控核实）：`error_anchor` 唯一写点 `scan_dimen_body` 入口
（scan.rs），每次值扫描无条件写、成功路径不清除 → 粘滞。`error_context` 对任何报错
`error_anchor.unwrap_or(*pos)` 优先读锚点 → 后续无关报错的 `l.N` 被拉回旧行（**全量亦错**，
增量下「段 1 中性复用跳过扫描 + 段 2 报错」使行号与全量漂移）。

**修法改道**（执行者实证否决简报的「body Ok 即清」形态）：`clamp_dimen` 在 inner 返回
**Ok 之后**才执行，Ok 即清会弄丢 `! Dimension too large.` 的值行回溯（实测 l.1→l.2）。
终形态 = **锚点作用域 = 一次完整值扫描（含 clamp）**：`scan_dimen` / `scan_dimen_mu` /
glue plus-minus 三处保存外层锚点、扫描结束（成功/失败）恢复（顶层即 None）。
全部 `scan_dimen_inner` 调用点均走包装（grep 核实无绕过路径）。save.rs:28 的冗余清理随删。

**回归锁三把**：
- `successful_dimen_scan_does_not_leave_stale_error_anchor`（全量粘滞）
- `stale_error_anchor_does_not_leak_across_segments`（SegmentEngine 增量 vs 全量转录逐字一致，
  RED 现场实测：全量 `l.1 \count1=\x` vs 增量 `l.2 \fi`）
- `dimension_too_large_reports_value_line`（clamp 合法用途不回退）

**语义收益**：粘滞旧行号在全量模式下本来就是错的（真 TeX 的 `l.N` 停在报错现场）——
本刀同时修正全量与增量两侧语义，而非只为增量打补丁。

## 三、验证（主控独立复测，全部亲跑）

| 项 | 结果 |
|---|---|
| `cargo test -p ntex-core -q` | 606 passed / 0 failed / 2 ignored |
| `cargo test -p ntex-layout -q` | 346+4+10+4+3 全绿 |
| 三新锁 + P1-1 锁单跑 | 全 GREEN |
| clippy `--workspace --all-targets -D warnings` | 0 |
| `cargo fmt --all -- --check` | OK |
| bench 场景 A（release，CodeBuddy 3 跑） | 7.9–8.3x 稳态（≥7x 口径不劣化） |
| TRIP：`trip.tex --no-plain --dump trip.fmt` → `tripos.tex --fmt trip.fmt --no-plain` | 177B/1 页 + 750B/12 页，tripos.dvi SHA-256 `574045fbe51f4ec4…` 与历史基线逐字节相同 |
| 宏包回归 `pkg-regression.py` | PASS 13/13 |

**坑速记**：TRIP 防回退线漏 `--no-plain` 会得到 251B/0 页假回退（plain 预载进快照）——
正典命令 `ntex-dvi trip.tex --no-plain --dump trip.fmt` → `ntex-dvi tripos.tex --fmt trip.fmt --no-plain`。

## 四、后续（未改，登记）

1. `SegmentEngine`（ntex-core/incremental/engine.rs:437）内联的旧中性判据（value + eqtb）
   与 `snapshot_states_equal` 双轨——error_anchor 根治后转录漂移洞已消，但双轨仍是
   「相等判定字段集 ⊆ 还原写回字段集」铁律的隐患，建议另刀一行收编。
2. `IncrementalTypesetter::transcript` 文档「单调累积」措辞与实现（edit 截回边界）不符，
   文档级修正。
3. review 第 3 项裁决存档：投机判定（speculate.rs:214）变严 = 保守方向（采纳率降），
   语义安全；锚点根治后「投机回放不推进主引擎控制态」缺口一并消除。
