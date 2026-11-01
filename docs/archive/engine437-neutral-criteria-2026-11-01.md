# SegmentEngine 中性判据对齐 + transcript 文档修正（2026-11-01）

性质：M5 阶段六第三刀 review 遗留两靶的收口刀（-expand 半边镜像）。HEAD 起点 `1ec59ff`。

## 靶 1：SegmentEngine（engine.rs ~L454）内联旧中性判据

第三刀 P1-1 修复给排版层管线（ntex-layout `snapshot_states_equal`）补了**对输出可见的
控制态**比较面，但 ntex-core 侧 `SegmentEngine::execute_segment` 仍内联旧两维判据
（`slots_semantically_equal` + `ValueState` 相等）——同类漂移仍在：只改
`ControlState` 输出可见字段（`\tracingcommands` 改 `shown_trace_mode`、`\end` 置
`ended`）的段被判中性 → 链偏差下复用跳过执行且不还原 post → 回滚点携带旧控制态，
后续回滚/复用转录漂移。

修法：判据改调 `snapshot_states_equal`（值态 + eqtb 逐槽 + `ControlState::
output_visible_equal`），与排版层管线同源。比较面不含 `last_tok`（每 token 必变，
全比杀死复用）与 `error_anchor`（P1-1 后续已改为扫描内有效、段边界恒 `None`），
不会误伤既有复用；新判据只会把原判中性的段改判非中性（少复用不多复用），无反向回归。

回归锁：`end_segment_control_state_rejected_on_chain_delta`（incremental/tests.rs）——
`\def\a{X}` / `\end` / `B` 三段，edit 第 0 段后断言 `\end` 段 `!from_cache`。
**实证「去修复必红」**：临时还原 engine.rs 旧判据（git stash 单文件）该测试红，
恢复后全绿。

## 靶 2：`transcript()`「单调累积」文档不实

`transcript()` 实为**各段缓存转录按段序拼接**（`execute_segment` 每段设新 sink、
转录存段缓存），edit 回滚后编辑段及其后各段重算/复用、前缀段缓存不动——拼接结果
是「当前链上各段最近一次执行的转录」，不是首轮 `run` 的原样累积。旧注释声称
「更早的转录字节仍在 sink 缓冲里」「回归测试只能比较尾块」与实现不符（既有测试
`edit_rollback_then_earlier_edit_matches_full` 就是增量 vs 全量**整体**对比）。
文档按实现重写（Codex review 需修项）。

## Codex review（gpt-6-sol，只读）结论与裁决

**P-with-fixes**。逐条裁决：
1. 比较面无遗漏无过比（checkpoint.rs 排除项有据）——采纳，无需改；
2. 判据只会更严不会反向——与主控走读一致；
3. 测试能锁旧判据，但「下一轮 edit 后转录漂移」的声称超出了测试实际断言——
   **采纳**，注释收窄为「旧 post 留链上、后续回滚/复用即漂移」（漂移断言已由
   排版层姊妹锁 `trace_control_state_rollback_matches_full_transcript` 覆盖）；
4. transcript 注释与实现不符——**采纳（唯一需修项）**，按段缓存实现重写；
5. MSRV/clippy——主控侧实测：clippy -D warnings rc=0（当前工具链），无新 API。

## 门禁（原始输出）

- `cargo test -q -p ntex-core`：**607 passed; 0 failed; 2 ignored**（11.63s）
- `cargo test -q -p ntex-layout`：**346 passed** + 4 + 10 + 4 + 3（0 failed）
- `cargo clippy -q -p ntex-core --all-targets -- -D warnings`：rc=0
- `cargo fmt --all -- --check`：ok
- TRIP 防回退（当前 HEAD 重建二进制，正确姿势 `--no-plain --dump`）：
  trip.dvi **177B / 1 页**；tripos.dvi **750B / 12 页 / 5 字体**，
  MD5 `92100387e70e2695eb267f28400091c1` 与基线逐字节一致。

## 遗留

- `docs/pkg-regression-baseline.md` 元数据刷新（日期/HEAD/二进制 mtime 17:15）
  随本轮入库，为上一轮宏包跑分的合法基线刷新，数字无实质变化。
- 阶段六余项见 plan.md §2 M5「阶段六待办」（结构性 owned 拷贝两靶未动）。
