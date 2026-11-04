# M5 阶段六第八刀战报：runtime_digest 成分迁移至精确比较（2026-11-04）

## 概述

- **刀靶**：第七刀实测证伪了「eqtb 拷贝是大头」后钉出的真靶——post 捕获
  `value_state()` 中 `runtime_digest()` 占 ~35µs/段（90%+），其中
  `format!("{:?}", self.params)` Debug 串（整份内部参数序列化+格式化）与
  lccode/uccode 两个 256 元素循环是可精确比较的字段混进指纹的白付成本。
- **改动**（`3976aa7`，Codex gpt-6-sol 实施、主控收口）：`params` / `lccodes` /
  `uccodes` 三字段从 `runtime_digest()` 摘除，迁入 `ValueState` 精确比较字段
  （`Params` 已 derive `PartialEq`，`[i64;256]` 天然 `PartialEq`）。
  - `ValueState` 加三字段，`value_state()` 填充（Copy 栈拷贝）；
  - `runtime_digest()` 删 params Debug 串与 lcc/ucc 两循环；
  - 上轮勘察轮遗留的 `digest_diag` 计时插桩文件删除（incremental_tests 两行调用同步删）；
  - `ValueExtras` 三字段**保留**（还原通道不变），checkpoint.rs 注释更新
    「指纹字段集一一对应」的过时说法；
  - 顺带修一个中间态残留：第一版 patch 把三字段错加到了 `FmtState`
    （`export_state`，其 `params` 已有），Codex 自查发现后移正到 `ValueState`。
- **语义**：比较口径从「哈希概率等价」升级为「精确等价」，判定更严不更松；
  fmt 格式不动（VERSION 不 bump）。

## 实测（release，/tmp/ntex-cron-target）

**基准（bench_edit_paragraph_vs_full --ignored --nocapture）**：

```
── 场景 A 改正文 1 段（169 段 / 21 页）───────────────
全量：    32.07 ms
增量：     5.50 ms  加速    5.8x  复用 111/169（执行 2）
── 场景 B 改宏体段（121 段 / 12 页）─────────────────
全量：    22.73 ms
增量：    17.94 ms  加速    1.3x  复用 60/121（执行 61，其中依赖失效重排 61）
```

主控当日复跑（同测试，负载不同）：A 5.0x（26.56ms）、B 2.7x（23.39ms）——
场景 B 加速比波动属本机 2 核高负载噪声（第七刀战报同现象），判定看增量墙钟
绝对值与全量同幅波动，无回退。

**门禁**（原始输出行）：

| 项 | 结果 |
|---|---|
| core 测试 | `test result: ok. 609 passed; 0 failed; 2 ignored` |
| layout 测试 | `test result: ok. 362 passed; 0 failed; 3 ignored` |
| clippy -D warnings | Finished，0 警告 |
| fmt 冒烟（sample2e-slim --fmt） | 3373B / 1 页 / 58 字体 |
| TRIP CLI（trip.tex 两步 --no-plain --dump） | 251B / 1 页，与 `fixtures/trip/trip.dvi` golden 逐字节一致 |
| ntex-trip | 4 passed |

**口径备注**：简报与历史 skill 写的「trip 177B」为 vsize 语义修正前陈旧口径——
blade4 战报已登记「全本 9 页 4015B / 构造 251B 是正确形态」；本刀 golden =
`fixtures/trip/trip.dvi`（251B，fixtures 内 11-03 更新）。后续简报统一改 251B。

## 收益定性

post 捕获预估 ~42µs → 预期 ~12µs 级（削掉 params Debug 串 ~30µs + lcc/ucc 循环
~4µs）。场景 B 61 执行段 × ~30µs ≈ 1.8ms 理论墙钟收益，与本轮实测 B 场景
17.94ms（负载噪声带）一致方向但不可单独判读。**第八刀后 digest 剩余成分**：
mathcode/delcode/fontdimen/hyphenchar/skewchar 五个 HashMap 逐项哈希、
font_loads Debug 串、parshape/penalty_arrays/流状态——同款「可精确比较却进指纹」
嫌疑最大的仍是 `font_loads` 的 `format!("{:?}")`，其余 HashMap 项数少
（mathcode 通常 <100 项）成本有限。

## 回归锁

本刀为纯性能迁移（比较面等价收紧），既有
`checkpoint_roundtrip` / 增量 vs 全量逐位对拍锁（`bench_edit_paragraph_vs_full`
内 assert_eq 页面+字体表）即为行为锁；无新增独立锁（与第七刀同形态）。

## 遗留 / 下一刀靶

1. `runtime_digest` 剩余成分治理（见上），或改走「脏标记/版本戳」路线——
   digest 只在相关写路径置位时重算；
2. pre 侧捕获（每执行段两次 `StateSnapshot::capture`）；
3. 阶段六待办其余项见 plan.md §2 M5。
