//! # ntex-backend：渲染后端（plan.md §10 M8 渲染后端条目）。
//!
//! 职责：把排版产物（`Typesetter::typeset_dvi` 的 `(pages, fonts)`，同源于
//! `ntex_dvi::write_dvi` 的输入）渲染为像素缓冲，并提供 PNG 导出。
//!
//! ## 后端实现
//!
//! - [`TinySkiaBackend`]：软光栅（纯 Rust 自研，见下方偏差说明）；
//! - [`VelloBackend`]：vello 0.10 / wgpu 29 GPU 路径（compute 管线 + area AA），
//!   无头渲染到 Rgba8Unorm 纹理后回读；GPU 不可用时报错，可回落软光栅。
//!
//! 两后端共享 [`prims`] 的「盒树 → 矩形指令」遍历，几何完全一致，
//! 位图可做差分对照（tests/vello.rs）；字形轮廓（M9 后）也将在该层
//! 扩展为曲线指令，两后端同步受益。
//!
//! ## 调试 overlay
//!
//! [`RenderOptions::debug`] 开启时，[`prims`] 额外产出独立 `debug` 通道：
//! 版心与盒边界描边（HBox 蓝 / VBox 紫红）、基线（青）、glue 自然宽带 +
//! stretch/shrink 指示线（fil 阶亮绿）、kern 橙线、penalty 断点标记
//! （禁断深红粗线）。后端在内容之后绘制该通道，`rects` 不受影响，
//! 正常渲染与差分口径零变化。
//!
//! ## 与 plan 的偏差（历史）
//!
//! plan.md M8 条目选型为 tiny-skia。软光栅首版落地时构建环境**离线**且
//! 本地 cargo 缓存无 tiny-skia（网络受限），故位图/PNG 编码为**纯 Rust
//! 自研**（zlib deflate + PNG chunk，见 [`png`]）；对外仍以 [`Backend`]
//! trait 抽象，[`TinySkiaBackend`] 仅是当前实现名（语义对齐 tiny-skia：
//! RGBA8 预乘 sRGB 软光栅）。
//!
//! ## 坐标语义（对照 ntex-dvi / ntex-pdf）
//!
//! - 源单位 sp：`1pt = 65536sp`；像素 = `sp / 65_536 * dpi / 72`；
//! - 页面原点左上、y 向下（同 DVI）；盒子/规则的参考点语义与 `ntex_dvi`
//!   的 `hlist`/`vlist` 完全一致（见 [`prims`]）；
//! - **字符无字形**：TFM 只有度量，无轮廓（M9 字体子系统范围）。字符按
//!   任务简报画占位方框 + 基线标记示意，非最终排版效果。

#![deny(unsafe_code)]

pub mod png;
pub mod prims;
pub mod raster;
pub use prims::RenderOptions;
pub use raster::Pixmap;

mod backend;
mod vello;

pub use backend::{Backend, BackendError, RectPrim, TinySkiaBackend};
pub use vello::VelloBackend;
