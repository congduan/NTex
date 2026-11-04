# M5 阶段六第七刀战报：eqtb 槽向量 Rc 写时复制（2026-11-04）

## 概述

- **刀靶**：`Expander::capture_checkpoint` 的 `eqtb: self.eqtb.slots().to_vec()`（段边界
  post 快照捕获整份深拷贝）与 `restore_checkpoint` 的 `cp.eqtb.clone()`。
- **改动**：`Eqtb.slots: Vec<EqSlot>` → `Rc<Vec<EqSlot>>`；写路径（`ensure` /
  `slot_mut`）`Rc::make_mut`；`slots()` 签名不变（`&[EqSlot]`），调用方零改动；
  检查点字段 `EngineCheckpoint.eqtb` 改 `Rc<Vec<EqSlot>>`，捕获 = `Rc::clone`，
  还原 = `replace_slots_shared`（共享，下次写 make_mut 自动分离）。`levels` 仍为
  裸 `Vec<u8>`，还原语义保持"全 0"。
- **语义**：零变更。只把"拷贝"换成"引用计数"，**未做**任何"相等⇒跳过还原"的优化；
  `define_macro` 的"内容不变不 bump 版本"原样保留；fmt 序列化格式不动（VERSION 不 bump）。

## 结论先说：刀靶前提被证伪（证伪有功）

简报的事实依据是「post 捕获 32.6µs/段，根因 = `eqtb.slots().to_vec()` 整份深拷贝」。
本机实测该场景（`bench_edit_paragraph_vs_full` 场景 B，61 执行段）**eqtb 只有 465 槽**，
`to_vec` 分量实测 **≈6.8µs/次捕获**（instrumented，649 次捕获平均，含编译期小状态；
post 段上约 3–4µs），不是 32.6µs。post 捕获的真大头是 `value_state()` 里的
**`runtime_digest()`**（FNV 指纹逐字段重算，占 `value_state` 的 90%+，约 35µs/段）。

因此本刀的真实收益 = 削掉 eqtb 分量（3–7µs），**达不到 <10µs 的目标**；<10µs 需要下一刀
动 `runtime_digest`（增量维护 / 脏标记 / 或把指纹覆盖字段也 CoW 化），而那在 expand/mod.rs
与 incremental 的领地之外，本刀未越界。

## 实测（release，HEAD `81def5f` 对照 worktree 交错复跑）

本机 `nproc=2`、实测期间 load average **14.4**（≈7x 超订），墙钟不可判读；分相计时
（`NTEX_INCR_PROFILE=1`，相位 3 = post snapshot，61 次调用）取多次复跑的**最小值**：

| post 捕获（µs/段，ns÷61） | HEAD | 本刀 |
|---|---|---|
| 第 1 组 3 轮 | 45.1 / 50.9 / 48.3 | 42.1 / 63.5† / 43.0 |
| 第 2 组 5 轮 | 49.6 / 51.2 / 46.5 / 90.1† / 48.9 | 125.7† / 123.2† / 49.5 / 42.1 / 67.4 |
| 第 3 组 7 轮 | 47.7 / 46.7 / 186.8† / 46.8 / 46.0 / 115.0† / 77.7 | 48.4 / 50.2 / 45.3 / 46.0 / 43.5 / 44.7 / 113.5† |

† = 被同机其它负载打断的样本。**最小值 45.1 → 42.1µs（−6.6%）**，与本刀应削掉的
eqtb 分量（3–4µs）一致。

组件级 instrumented 实测（release，measure 后已还原）：

| 分量 | HEAD | 本刀 |
|---|---|---|
| eqtb 捕获（465 槽） | 6.8µs（`to_vec` + 465 次 Arc 计数） | **0.11µs**（`Rc::clone`） |
| `value_state()`（含 `runtime_digest`） | ~35µs | ~35µs（未触及） |
| `ValueExtras` 克隆 | ~4µs | ~4µs（未触及） |
| `ControlState` 克隆 | ~0.05µs | ~0.05µs（未触及） |

墙钟（同一交错运行，最小值）：场景 A 增量 7.44ms → 7.41ms（全量 51.3→51.6ms，
加速比 6.5–7.0x 不变）；场景 B 增量 22.63ms → 23.15ms（在 7x 超订噪声内，判不可分辨）。
**无回退**；但也**没有**可判读的墙钟收益——post 相位只占场景 B 墙钟 61×45µs ≈ 2.7ms/23ms。

## 安全性前提与审计

- **共享安全**：检查点持 `Rc` 旧副本，活引擎首个 eqtb 写经 `Rc::make_mut` 分离；
  还原后引擎与检查点重新共享，下次写再分离。同阶段五 `BoxFile` / 第一刀 InsertRegs /
  第四刀 shipped 页面的模式。
- **`Eqtb::clone` 审计**（要求 6）：全仓 grep `Eqtb` 的 `.clone()` 只有
  `restore_checkpoint` 一处（已改 `Rc::clone`）；`#[derive(Clone)]` 仍在，但**无调用点**
  （`Expander.eqtb` 从不 clone）。`import_state` 是 owned 替换（`self.eqtb = eqtb`）；
  `fork_isolated` 走 `capture_checkpoint` → `restore_checkpoint`，因此 fork 后两侧共享
  `Rc`，各边首次写 `make_mut` 分离，语义与深拷贝 fork 等价（回归锁 `fork_isolated_eqtb_cow_isolation` 锁证）。
  跨 crate 无泄漏：`EngineCheckpoint.eqtb` 是 `pub(crate)`，ntex-layout 只经 `StateSnapshot` 句柄。
- **线程前提**：M6 投机是 `fork_isolated` fork 式顺序执行（`speculate.rs` 注释明示
  `Send` 化是 M6-3），当前无真线程，`Rc` 安全；设计注释已注明未来升 `Arc` + 锁的路径。
- **wasm**：`Rc` 与平台无关，`cargo check -p ntex-core --target wasm32-unknown-unknown` 零警告。

## 回归锁（`crates/ntex-core/src/expand/checkpoint.rs::cow_tests`）

1. `checkpoint_capture_shares_eqtb_slots`：捕获后 `Rc::ptr_eq` + `strong_count ≥ 2`；
   捕获后写 eqtb → 检查点独占（count == 1）且槽值未被污染；还原回捕获值并重新共享。
2. `fork_isolated_eqtb_cow_isolation`：fork 后两侧 `ptr_eq`（共享）；fork 侧 `\def` 生效于
   fork 自己、主实例槽不变、intern 独立；主实例再写仍与 fork 独立。

**去修复必红（手工实证后已还原）**：
- 把捕获换回 `slots().to_vec().into()` → 两锁都红（`捕获必须共享槽向量（深拷贝 = 本刀修复被撤销）`，
  以及 fork 的 `fork 应共享主实例的槽向量`）。
- 把 `Eqtb::ensure` 的 `Rc::make_mut` 换成 `Rc::get_mut(...).expect(...)` → 两锁都红
  （`共享槽向量被就地写`），证明写路径确实命中共享态、锁对"忘记分离"敏感。

## 门禁原始输出

```
cargo test -p ntex-core
test result: ok. 609 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out

cargo test -p ntex-layout --lib
test result: ok. 362 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out
cargo test -p ntex-layout incremental
test result: ok. 28 passed; 0 failed; 2 ignored; 0 measured; 335 filtered out

cargo clippy -p ntex-core -p ntex-layout --all-targets -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s)

cargo fmt --all --check → 无输出
cargo check -p ntex-core --target wasm32-unknown-unknown → Finished, 0 警告

fmt 冒烟：ntex-dvi samples/latex-sample2e-slim.tex --fmt assets/fmt/latex.fmt
→ 已写出 /tmp/smoke7.dvi（3373 字节，1 页，58 字体）

TRIP：ntex-dvi fixtures/trip/trip.tex → 251 字节 1 页，与 golden fixtures/trip/trip.dvi
逐字节相同（cmp 无差异）；cargo test -p ntex-trip → 4 passed

宏包回归：PASS 13/13（负例 footmisc PASS）
```

tripos 未产出页（`源码缺少 \shipout`）——本机按裸 `tripos.tex` 直接跑不进门；
`cargo test -p ntex-trip` 为简报允许的替代口径，已跑。

## 下一刀建议

`value_state()` 的 `runtime_digest()`：每段捕获重算一次 FNV（覆盖 everypar/mathcode/
lccode/parshape/penalty/fontdimen/字体表/流位置/参数 Debug 串…，~35µs/段）。三选一：
① 写侧脏标记 + 首次读算一次；② 把覆盖字段本身 CoW 化后按 `Rc::ptr_eq` 短路（`EqSlot`
已有同款快照快路径）；③ 把 digest 从"每次捕获"改为"段边界增量化"。任一都能把 post
从 45µs 压到 10µs 以下，是本阶段 `<10µs` 目标的真正落点。
