# M6 投机执行 `\output` 例程边界刀（2026-11-02）

## 结论

LaTeX 文档投机面 = 0 的保守口径已拆除。**机制**：投机 fork 实例在 token 边界
**跳过**输出例程注入（`defer_output_injection` 旗标），fork 只收集节点；例程由
主实例在**采纳后自己的断页边界照常注入执行**——例程副作用从不在投机实例内发生，
不存在「例程被投机吸收后采纳」的漂移面。真实 LaTeX 文档（`latex.fmt` + article
结构）测试从「投机面为 0 也行」收紧为 **`adopted > 0` 断言**，投机边界语义由
回归锁锁定。

## 改动清单（4 文件，+33/−20）

| 文件 | 改动 |
|---|---|
| `ntex-core/expand/mod.rs` | `Expander` 加 `defer_output_injection: bool`（默认 false）+ `pub fn set_defer_output_injection`；run 循环 token 边界 `maybe_inject_output()` 门控加 `!defer`；`reset` 归零 |
| `ntex-layout/typeset/speculate.rs` | fork 后 `set_defer_output_injection(true)`；`spec_window_open` 删 `output_routine` 条件；`spec_candidate` 加段首 `$$` 静态预过滤（display-math 段 fork 自旧窗口态会经 file-hook 宏递归，转顺序执行）；SPEC_BLACKLIST 补 `documentclass`（class 加载展开文件探针必 taint） |
| `ntex-layout/typeset/speculate_tests.rs` | `speculative_matches_on_real_latex_document` 收紧：`adopted > 0` + 字体表 Debug 串逐位相等 + 文档注释同步；`speculative_matches_with_output_routine` 语义升级（例程文档可采纳中性段） |
| `docs/pkg-regression-baseline.md` | 元数据刷新（随刀入库） |

## 主控 review 走读（Claude Code review 通道 Bun segfault 挂死 550s，缺席事实登记）

定向 5 项核查全部通过：

1. **defer 旗标恢复完整性** ✅：`fork_isolated`（checkpoint.rs:370）走
   `Self::with_bytecode` 全新构造（defer=false）+ 检查点恢复 + **字段照抄清单不含
   defer**；setter 全仓唯一调用点 speculate.rs:196（fork 实例），fork 实例一次性
   丢弃、无回写主实例路径——「defer 泄进主实例致所有文档不出页」灾难面不存在。
2. **reset 归零充分性** ✅：`Expander::reset`（expand/mod.rs:1545）显式归零；
   投机实例不复用（每段新建）。
3. **`$$` 预过滤** ✅：失败模式 = 段转顺序执行（正确性保守方向）；字面值特判
   限于段级调度层（非引擎状态机），且注释写明机理。
4. **`run_pending_output`（finish 收尾）不加门控** ✅：正确——收尾例程必须跑，
   门控只作用于 token 边界注入点（expand/mod.rs:1825）。
5. **MSRV/clippy/fmt** ✅：无新 API；clippy -D warnings 0、fmt 0。

## 验证基线（原始输出）

- `cargo test -p ntex-core`：**605 passed; 0 failed**（+2 ignored）
- `cargo test -p ntex-layout`：**346 passed; 0 failed**（+3 ignored）
- speculate 过滤：**7 passed; 0 failed**（含收紧后 real_latex_document，
  `assert!(spe.stats().adopted > 0)` 绿）
- TRIP 防回退：trip **177B** 1 页；tripos **750B** 12 页，
  SHA-256 `574045fbe51f4ec48f8de7878d5bd559e5a301b3ce5f5a5afe347222ffe9c150`
  与 `fixtures/trip/tripos.dvi` 基线逐字节相等（`--no-plain --dump` 两步正确姿势）

## 勘察教训（本轮付学费）

**「陈旧调用姿势假复现」**：首轮 trip/tripos 复测漏 `--no-plain`，trip 251B、
tripos 0 页，一度定性「HEAD 既有回归」并准备 53 提交二分——锚点 07a1751 也「坏」
才怀疑姿势（当次提交当天自验过 12 页）。correct 姿势见
math-accent-skew-2026-10-23.md 的警告，**开工复测前先翻该域历史战报的验收命令**，
别凭记忆组命令行。二分两轮白烧（HEAD worktree + 锚点 worktree 编译）。

## 下一刀靶

- M6-3 真并行（`Send` 化：NodeBuilder boxes Arc 化 / Fonts Arc<Mutex> /
  Box<dyn> trait 加 Send）——本刀清障后前置条件不变。
- M6-4/M6-5 窗口宽度自适应 + Amdahl 实测。
