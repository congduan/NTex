# M5 阶段六第九刀战报：font_loads/name 表/math_fonts 迁 ValueState 精确比较（2026-11-04）

## 概述

- **刀靶**：第八刀战报点名的 digest 剩余成分中「可精确比较却进指纹」嫌疑最大的
  `font_loads`（`format!("{:?}", ...)` Debug 串格式化）——连同同函数内同款的
  `font_names` / `font_cs_names`（逐项 digest）与 `math_fonts`（48 元素逐 digest）
  一并迁移。
- **改动**（`b3c8a7a`，Codex gpt-6-sol 实施→中途额度耗尽退出，主控接管收口+review）：
  - `ValueState` 加四字段（`font_loads`/`font_names`/`font_cs_names`/`math_fonts`），
    `value_state()` 填充；
  - `runtime_digest()` 删对应四段（Debug 串 + 两个逐项循环）；
  - `ValueExtras` 四字段保留（还原通道不变）；checkpoint.rs 三处注释同步；
  - 新回归锁 `font_tables_are_compared_exactly_outside_runtime_digest`：
    四字段各自改动 → `runtime_digest()` 不变 + `value_state()` 必变（迁移正确性双向断言）。
- **语义**：比较口径只收紧不放松（精确等价 ⊇ 哈希等价）；`FontLoad` 是
  `(String, Option<i64>, Option<i64>)` 私有 type alias，纯数据天然 PartialEq，
  底层类型全 pub 无 E0446。fmt 格式不动。

## 委派过程（Codex 额度耗尽首次实战）

Codex gpt-6-sol 按简报完成全部三文件改动 + 回归锁后，在跑验证阶段 workspace
额度耗尽退出（`ERROR: Your workspace is out of credits`，exit 1，37,876 tokens）。
**无 commit**。按接管流程：git status 清点工作树（三文件改动完整、含回归锁）→
主控走读 diff 全量 → 亲跑全量验证 → 主控 commit。**review 通道缺席事实记录在案**
（同刀AJ R11 先例：不能因通道挂了就跳过审查，主控亲自完成 diff 审查替代）。

## 实测（release，/tmp/ntex-cron-target，本机高负载噪声带）

基线（刀后主控当日实测，同噪声环境）：A 4.2x（增量 7.71ms）/ B 1.3x（15.86ms）。

刀后多轮复跑（全量基线波动 20.6→94ms，2 核负载噪声主导）：

```
增量场景 A 最小 7.05ms（3 轮：8.06 / 8.18 / 7.05，加速 8-10.8x）
增量场景 B 21-23ms（基线 15.86ms，与全量同幅波动）
```

场景 A（执行段 2 个 × capture 2 次）增量墙钟可观测下降（7.71→7.05ms 最小值）；
场景 B 增量绝对值高于基线但与全量波动同幅（47.71ms 全量 vs 基线 20.63ms），
负载噪声主导不可单独判读——按第八刀口径，判定看增量墙钟 + 全量同幅波动，无回退。

**理论收益定性**：font_loads Debug 串在 169 段文档 ≈ 55 字体 × 每段 2 次捕获；
digest 剩余成分中 HashMap 逐项哈希（mathcode/delcode/fontdimens/hyphenchars/
skewchars）与 parshape/penalty_arrays/流位置仍在指纹内。

## 门禁（主控亲测原始输出）

| 项 | 结果 |
|---|---|
| cargo test -p ntex-core | `610 passed; 0 failed`（基线 609 + 1 新锁） |
| cargo test -p ntex-layout | `362 passed; 0 failed` |
| clippy --workspace --exclude ntex-tauri -D warnings | Finished，0 警告 |
| cargo fmt --all --check | 干净 |
| TRIP：`ntex-dvi trip.tex`（plain 自动预载） | 251B / 1 页，`cmp` 与 fixtures/trip/trip.dvi golden 逐字节一致 |
| tripos：`--fmt trip.fmt` | 750B / 12 页，逐字节一致 |
| cargo test -p ntex-trip | 4 passed |
| fmt 冒烟：latex-sample2e-slim --fmt | 3373B / 1 页 / 58 字体（= 基线） |

**TRIP 验证口径勘误（本次实测澄清）**：blade8 战报写的「trip.tex 两步 --no-plain
--dump → 251B」实为**两件事混写**——`--no-plain --dump` 产 fmt 后跑正文是全本
4015B/9 页；251B/1 页 golden 来自**直接跑**（plain 自动预载，无 --fmt）。
第九刀两形态均验证：全本 4015B/9 页 + 直接跑 251B golden 逐字节一致。

## 回归锁

新锁 `font_tables_are_compared_exactly_outside_runtime_digest`（expand/tests.rs）：
四字段各自 push/改动 → digest 恒等 + ValueState 不等——锁「迁移不回退」。
行为锁沿用 checkpoint_roundtrip + 增量 vs 全量逐位对拍。

## 遗留 / 下一刀靶

1. `runtime_digest` 剩余成分（五 HashMap 逐项哈希 / parshape / penalty_arrays /
   流状态）——fontdimens 在 expl3 重载文档可达数万项，clone 换哈希可能反亏，
   **建议改走脏标记/版本戳路线**（写侧置位 + 首次读算一次）或字段 CoW 化 +
   `Rc::ptr_eq` 短路（blade7 战报的①②选项）；
2. record_post 恒真捕获（每执行段 1 次，capture 内部仍有 eqtb dirty 扫描）；
3. 阶段六待办其余项见 plan.md §2 M5（副作用边界 / Expand 纯函数化 / 槽级归因）。
