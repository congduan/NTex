# M5 阶段六第一刀：InsertRegs Rc 写时复制（检查点捕获成本）

- 日期：2026-10-31
- 基线 HEAD：`a4d5945`（WIP 为上一 cron 轮遗留工作树改动，本轮收口）
- review：CodeBuddy（deepseek-v4.1-flash），结论 **P**；8 项非阻塞建议采纳 6 项
  （坏 intra-doc link / dimens+skips 同值 no-op 断言 / 注释与实测对齐 /
  `from_parts` debug_assert 三连 / 独立测试 mod / 写频措辞），1 项随本刀收口
  （plan.md 三处口径不一致），1 项不适用（KNOWN-SIMPLIFICATIONS 无需登记）。

## 背景

M5 阶段六待办 #1「检查点捕获增量维护」：`IncrementalTypesetter` 每段边界
捕获（`LayoutBoundary::capture` = NodeBuilder 整份克隆）都会深拷贝
`InsertRegs` 三根 32768 槽向量（counts/dimens/skips，≈1.5MB/份）；
RollbackPoint 密度 1/8 时一次编辑仍要 ~20 次捕获。宏体编辑场景加速比被压
在 1.3x 的捕获成本大头之一。

## 修复形态

`InsertRegs` 三根向量改 `Rc<Vec<_>>` 写时复制（模式先例 = M5 阶段五 `BoxFile`）：

- 克隆退化为引用计数（检查点快照零深拷贝）；
- 写侧 setter（`set_count`/`set_dimen`/`set_skip`）= 越界守卫 + **同值 no-op
  短路** + `Rc::make_mut`——仅当快照仍持旧副本时深拷贝一次，随后活体唯一
  持有、连续写零拷贝；
- sink 侧 `count_changed`/`dimen_changed`/`skip_changed`（`sink/page.rs`）改走
  setter（语义等价：越界静默忽略与旧 `get_mut` 守卫一致；`set_count` 后的
  `count_changed` 递推调用在早退路径上仍执行）；
- fmt 快照恢复路径改 `InsertRegs::from_parts`（`debug_assert_eq!` 三连锁槽数）；
- `incremental.rs` 仅诊断插桩增删（`NTEX_M5_DIAG` 计时，从未入库），终态与
  基线一致。

## 实测（release / 同一 120 段基准 `bench_edit_paragraph_vs_full` / 2GB VM）

| 场景 | 基线（stash 对照） | 本刀 | 变化 |
|---|---|---|---|
| A 改正文 1 段 | 4.2x（24.1ms vs 100.5ms） | **5.2x**（16.8ms vs 87.3ms） | +~25% |
| B 改宏体段 | 1.3x（43.6ms vs 58.0ms） | **1.6x**（33.4ms vs 53.5ms；噪声带 1.3–1.6x） | +~25% |

诊断插桩（已剥离）钉出的剩余开销分布（B 场景 35.3ms）：
execute 21.7ms（61 段依赖失效真实重排，合法开销）+ reinject 7.3ms +
boundary 2.1ms + rollback 1.2ms；捕获路径单次 exp_snap ~0.03ms /
layout_snap ~0.01–0.14ms（CoW 生效实证）。**下一刀靶 = 复用段 reinject 的
页面装配重跑**（A 场景 16ms 中占 10.2ms）。

## 回归锁

`page::insert_regs_tests::insert_regs_cow_semantics`：① 同值写 no-op（三根
strong_count 全锁）② CoW 分离（快照持旧值/活体持新值）③ 分离后唯一持有
直写 ④ 越界写静默忽略（在界值不变）⑤ `from_parts` 恢复路径可写可读。

## 门禁（全部亲跑）

- `cargo test -p ntex-core`：603 绿；`cargo test -p ntex-layout`：344 绿（含新锁）
- `cargo clippy -p ntex-layout`：0 警告；`cargo fmt --all --check`：干净
- trip 177B / tripos **12 页 750B 0 错**（TRIP 防回退锁命中）
- 宏包回归 **13/13 PASS**（`scripts/pkg-regression.py`）
- 基准测试 `bench_edit_paragraph_vs_full --ignored`：P（判据为打印非断言）
