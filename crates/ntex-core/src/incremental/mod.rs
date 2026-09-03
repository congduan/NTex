//! # M5 增量计算 —— 阶段二：可回滚段快照 + 单路径 edit（plan.md §7）
//!
//! M5 完整形态是求值图 `Expand(source, snapshot) → (tokens, snapshot')` 纯函数化，
//! 加上依赖追踪、失效传播、Box 级缓存复用。阶段一打通**切段 → 带快照执行 →
//! 缓存 → 编辑重算 → 与全量逐位一致**的骨架；阶段二把快照升级为**可还原的
//! 完整检查点**（`expand::EngineCheckpoint`），`edit` 收敛为**一条路径**：
//! 回滚到段前 → 重放，删除"从全新引擎重建状态链"的保底路径。基准数字见
//! `tests::bench_edit_one_segment_vs_full`（场景 A/B 从 39/107ms 降到 5/11ms）。
//!
//! ## 阶段二边界（离 M5 目标 ≥100x 还差什么）
//!
//! - **链偏差全量扫描**：每次执行段后重算一次"活状态 vs 旧缓存链"的偏差
//!   （值指纹 + eqtb 全槽比较）。要做偏差集的增量维护（赋值入口挂记录或
//!   COW 影子表）才能免掉这次扫描。
//! - **值状态槽级归因**：寄存器/参数/catcode/编码表变化仍按"全局失效"处理
//!   （[`expand::ValueState`] 整体比较）；`\csname` 动态构造的 cs 名按"读任意
//!   槽"保守失效。均未归因到槽。
//! - **副作用边界**：`\write`/`\openout`/`\openin` 读位置等副作用随段重算重复
//!   提交（RFC-3 把副作用移到 shipout 后才能干净重算）。
//! - **文本级切段**：[`segment::segmentize`] 按空行 / 行首 `\par`（深度 0）切分，
//!   不感知 catcode 与展开。跨段构造（未闭合 `\if`/`{`/`$`、跨段参数扫描）的
//!   语义本就不可切段——脏边界禁止复用缓存（保守正确）；检查点携带控制状态，
//!   悬挂态下的**回滚重算**仍与全量一致（见 `tests::edit_inside_unclosed_group_rolls_back_control_state`）。
//! - **错误消息行号**：切段后段内行号从 1 重计，转录（transcript）不参与
//!   逐位一致比较；比较口径是输出 token 流（`render_tokens`）。

pub mod engine;
pub mod segment;
pub mod snapshot;

pub use engine::{render_tokens, SegmentEngine, SegmentResult, SegmentStats};
pub use segment::segmentize;
pub use snapshot::SegmentDeps;

#[cfg(test)]
mod tests;
