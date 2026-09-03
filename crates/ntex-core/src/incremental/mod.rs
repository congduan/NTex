//! # M5 增量计算 —— 阶段一 POC：段级重算 + 依赖追踪雏形（plan.md §7）
//!
//! M5 完整形态是求值图 `Expand(source, snapshot) → (tokens, snapshot')` 纯函数化，
//! 加上依赖追踪、失效传播、Box 级缓存复用。本模块先打通
//! **切段 → 带快照执行 → 缓存 → 编辑重算 → 与全量逐位一致**的骨架与正确性，
//! 不求性能（基准数字见 `tests::bench_edit_one_segment_vs_full`）。
//!
//! ## 阶段一边界（与 M5 完整版的差距 = 阶段二待办）
//!
//! - **文本级切段**：[`segment::segmentize`] 按空行 / 行首 `\par`（深度 0）切分，
//!   不感知 catcode 与展开。跨段构造（未闭合 `\if`/`{`/`$`、跨段参数扫描）的
//!   语义本就不可切段——[`Expander::boundary_is_clean`] 检出脏边界后该段之后
//!   禁止复用缓存（保守正确），不做段间状态回滚。
//! - **持久引擎顺序重放**：段在同一个 `Expander` 上顺序执行，不是
//!   `Expand(source, snapshot) → snapshot'` 纯函数（无状态复刻/回滚）。因此
//!   `\write`/`\openout`/`\openin` 读位置等**副作用**会随段重算重复提交
//!   （RFC-3 副作用边界移到 shipout 后才能干净重算——阶段二）。
//! - **宏槽级依赖**：寄存器/参数/catcode/编码表变化按"全局失效"处理
//!   （`ValueState` 整体比较）；`\csname` 动态构造的 cs 名按"读任意槽"保守失效；
//!   均未做槽级归因。`ValueState` 只读探针见 `expand::Expander::value_state`。
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
