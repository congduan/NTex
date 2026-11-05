# M5 阶段六第十一刀：Registers restore_dirty 去整份重分配（撤销活脏键 + 影子表重放）

> 战报索引：plan.md §2 M5 阶段六注；commit `bb45c12`（2026-11-05）。
> 本文件由次轮 cron（同日）补写归档——上一轮提交了代码但未及落 plan.md 结论与战报，
> 本轮 A/B 复测补齐数据后归档，代码零改动。

## 靶形与修法

`Registers::restore_dirty`（段级回滚的寄存器还原）此前 `*self = Self::new()` 整份
重分配 32768×5 槽的五个大数组（counts/dimens/skips/muskip/toks，~1.2MB 级），
即便本段只写过几个槽。修法利用「未写槽恒为零值」不变量（`ValueState` 比较口径的
前提）+「影子表只在此处整体替换」的所有权事实：

1. **撤销活脏键**：先遍历**活**影子表键集，把这些槽归零（不碰其余槽——它们本来就
   是零值）；2. **影子表重放**：直接替换 `self.dirty` 并把快照值写进槽（绕过
   `set_*` 的 `insert` 键重插——值与影子表同源，直接写数组即可）。

开销从「与寄存器文件总量成正比」降为「与脏键数成正比」。

## A/B 实测（本轮 cron 交错复测补齐，SegmentEngine 场景 A）

基准 = 500 段宏定义 + 500 段正文（与既有场景 A 探针同构），两代二进制交错
2 组 × 每组 3 轮，`SegmentEngine::edit(250, …)` 单段改正文：

| 项 | 父提交 `1fcb06d` | HEAD `bb45c12` | 改善 |
|---|---|---|---|
| 场景 A edit 墙钟 | 3.85–4.07ms（中位 ~3.9） | 0.70–0.75ms（中位 ~0.71） | **~5.4×** |
| 全量 run（噪声对照） | 148–149ms | 149–154ms | 持平（噪声带，确认非机器漂移） |
| 复用段数 | 500/501 | 500/501 | 语义一致 |

第一轮粗测（非交错）曾显示 5.6ms vs 0.74ms（~7.5×）与全量 276 vs 162ms 的
机器漂移——**按交错对照口径采信 ~5.4×**，全量持平排除了负载噪声解释。

## 回归锁

`registers_restore_dirty_matches_fresh_replay`（撤销+重放与「从全零寄存器重建」
精确一致，含 toks/Glue order 变体）+ `registers_restore_dirty_replaces_shadow_table`
（影子表整体替换语义）。

## 门禁（本轮 cron 复测，HEAD `bb45c12`）

- cargo test ntex-core **619 绿** + ntex-layout **362 绿**；
- trip 构造页 251B 与 `fixtures/trip/trip.dvi` golden 逐字节一致；
- tripos `--fmt trip-ntex.fmt`（**`--no-plain --dump` 两步**，plain 预载会毒化快照
  产「0 错 0 页」假形态——本轮再次踩中，blade9 勘误口径持续有效）750B/12 页与
  fixture 逐字节一致；
- 宏包回归 **13/13 PASS**；资产审计 27 PASS / 3 ENGINE-GATE / 0 ERROR；
- 工作区遗留勘察探针已剥离（上一轮 cron 的场景 A 计时插桩，数据已消费）。

## 阶段六余项（不变）

digest 剩余成分（五 HashMap 逐项哈希 / parshape / penalty_arrays / 流状态——
fontdimens 大表 clone 换哈希可能反亏，建议脏标记或 CoW+ptr_eq 路线）、
record_post 恒真捕获、副作用边界（`\output` 回放 / `\write` 流 / `\input` 注入 /
marks 保守全量重排）、`Expand(source, snapshot)` 纯函数化收尾。
