# M5 阶段六第二刀：IoState 诊断域惰性化（2026-10-31）

## 背景

M5 阶段六第一刀（InsertRegs CoW，`fc4b800`）把改正文加速比推到 5.2x 后，主控
cron 轮接手时发现工作树有一份未提交 WIP：`IoState.last_appended` 惰性化——
`NodeBuilder::append` 热路径上**每节点**执行 `format!("{node:?}")`（String 堆
分配 + 全字段 Debug 格式化），只为喂 20 万节点才打印一次的 layout watchdog。
长文档（1 万+ 节点）每次排版为从未消费的字符串付费。

## 改动（3 文件 +70/−8，WIP 复活 + review 修复）

- `typeset/mod.rs`：
  - `IoState.last_appended: String` → `last_char: Option<(FontId, u32)>`（Char
    节点采样 font+charcode 两标量）+ `last_tag: i8`（`node_type_code()`，e-TeX
    `\lastnodetype` 码 0..=13，-1=尚未追加）；
  - **watchdog 打印块从 push 之前移到之后**（review F1：push 前打印时
    `top_len−1` 指向的是上一个节点，`last_char` 与 `last_rank` 错一位）；
  - `push_node`（行间惩罚入口，不经 `append`）同步维护两标量，防两入口口径分叉。
- `typeset/incremental.rs`：快照豁免清单 doc 同步（`nodes_appended`/`last_char`/
  `last_tag` 为惰性诊断标量，不进快照——SideEffects 是复用闸门，诊断字段绝不能
  进，否则增量命中率归零）。
- `typeset/incremental_tests.rs`：回归锁
  `io_state_diagnostic_fields_stay_lazy_scalars`——空态 tag=-1 / 非 Char 不污染
  last_char 且 tag=12（kern）/ Char 采样+tag=0 / `size_of::<IoState>() <= 48`
  封顶（64 位实测 48B；旧 String 形态 56B 必红；32 位 40B 不误伤）。

## Review（CodeBuddy deepseek-v4.1-flash，P-with-fixes）

| 项 | 内容 | 处置 |
|---|---|---|
| F1 | watchdog 打印在 push 前，`last_rank` 差一位 | ✅ 打印块移到 push 后 |
| F2 | 只采样 Char，纯 Kern/Glue 挂死失去定位（watchdog 立项场景恰是 `\halign` 无限增长） | ✅ 加 `last_tag`（复用既有 `node_type_code()`，零成本） |
| F3 | incremental.rs 快照清单注释重复词 + 漏新字段 | ✅ 改为 `nodes_appended`/`last_char`/`last_tag` |
| F4 | 测试注释尺寸算术错（12B 误算 16B；旧形态 56B 非 72B）+ rank 零覆盖 | ✅ 注释修正；rank 藏在 20 万次才触发的 eprintln 里，提函数抽测收益低、由 F1 位置修正 + 语义注释覆盖（登记残留） |
| F5 | plan.md 未登记 | ✅ 结论行 + 本战报 |

review 逐条核码裁决：F1 实锤（打印块确在 `record_main_node`/`push` 之前）；
F2 采纳（`last_tag` 方案比 `last_char_at` 更直接）；快照面判断（诊断字段不进
side_effects 是正确设计）reviewer 与主控一致。

## 实测（release / bench_edit_paragraph_vs_full / 2GB VM）

| 场景 | 前值（第一刀后） | 本刀 | 备注 |
|---|---|---|---|
| A 改正文 1 段（169 段/21 页） | 5.2x | **5.7x~6.5x**（多轮） | 复用 111/169 |
| B 改宏体段（121 段/12 页） | 1.6x | ~1.1x | 61 段合法重排主导（21ms+），字符串开销占比小；1.6x 与 1.1x 均在噪声带内，无实质变化 |

## 验证链（提交前全跑）

- `cargo test -q -p ntex-layout`：345+4+10+4+3 全绿（含新锁）；
- `cargo clippy -p ntex-layout --all-targets -- -D warnings`：0 warning；
- TRIP 防回退：trip.dvi 177B / tripos.dvi 750B·12页·0错，SHA-256 与
  `fixtures/trip/` 基线逐字节相等；
- 宏包回归 13/13（⚠ 复跑后 `git checkout -- docs/pkg-regression-baseline.md`
  防元数据脏改动）；
- `cargo fmt --all`（WIP 与 review 修复带出的既有 fmt 风格差异一并消掉）。

## 残留与下一刀

- `last_rank` 无直接单测（藏在 20 万节点阈值后）；如后续再动 watchdog，先提
  `fn watchdog_line()` 抽测。
- M5 阶段六下一刀靶不变：**复用段 reinject 的页面装配重跑**（A 场景 16ms 中占
  10ms，`incremental.rs` edit 循环 append 路径）+ expand 侧 `StateSnapshot::capture`
  每段两次 ~30µs。
