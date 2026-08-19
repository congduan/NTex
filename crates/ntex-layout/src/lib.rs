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
//!
//! M3-4 交付：
//! - [`typeset::Typesetter::with_tfm`]：TFM 字体模式——`\font\cs=cmr10` 经
//!   `ntex-font` 解析真实度量，字符维度/词间空白来自 TFM。
//!
//! M3-5 交付：
//! - [`page`]：断页 DP（TeX page builder 子集）——`\vsize` 自动分页，
//!   `fire_up` 打包页面、余下退回贡献，`\end` 冲页不产生多余空页。

#![deny(unsafe_code)]

pub mod hyphen;
pub mod linebreak;
pub mod node;
pub mod page;
pub mod typeset;

pub use linebreak::{badness, knuth_plass};
pub use node::{
    hbox_dimensions, hpack, vbox_dimensions, BoxDimensions, BoxKind, BoxNode, FontId, GlueOrder,
    LeadersKind, Node, GLUE_ORDER_FIL,
};
pub use typeset::{MetricsFn, SpaceFn, Typesetter};
