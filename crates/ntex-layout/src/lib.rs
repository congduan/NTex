//! # ntex-layout
//!
//! NTex 排版核心（M3）：节点模型、盒子维度、折行、断页。
//!
//! M3-1 交付：
//! - [`node`]：排版节点模型（Char/HBox/VBox/Rule/Glue/Kern/Penalty/Leaders）
//!   与盒子维度计算（`hbox_dimensions` / `vbox_dimensions`）。
//!
//! M3-2 交付：
//! - [`typeset`]：排版器（模式状态机 + token→节点构建）。

#![deny(unsafe_code)]

pub mod linebreak;
pub mod node;
pub mod typeset;

pub use linebreak::{badness, collect_breakpoints, BreakPoint};
pub use node::{
    hbox_dimensions, vbox_dimensions, BoxDimensions, BoxKind, BoxNode, FontId, LeadersKind, Node,
};
pub use typeset::{MetricsFn, SpaceFn, Typesetter};
