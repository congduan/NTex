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
pub use typeset::{IncrementalStats, IncrementalTypesetter, MetricsFn, SpaceFn, Typesetter};
// 规则未定宽度哨兵（tex.web `null_flag`——`\hrule` 无 width 说明时保持到
// vlist 出货/渲染，再解析为包含盒宽，见 `typeset/sink.rs` rule 分支注释）。
pub use ntex_core::NULL_FLAG;
// M8-A WASM 骨架线：TFM 字节源缝（宿主注入 TFM 字节；默认空 → native 走文件系统不变，
// 详见 typeset/wasm_fonts.rs 模块注释）。M9 中文刀 1 追加 OpenType 字节缝
// （`TfmSource::otf_bytes` / `registered_otf_bytes`，CJK 字体注入）。
pub use typeset::{
    clear_tfm_source, registered_otf_bytes, registered_tfm_bytes, set_tfm_source, TfmSource,
};
