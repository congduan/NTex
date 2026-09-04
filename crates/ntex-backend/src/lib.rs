//! # ntex-backend：渲染后端（plan.md §10 M8「Skia 渲染后端」条目）。
//!
//! 职责：把排版产物（`Typesetter::typeset_dvi` 的 `(pages, fonts)`，同源于
//! `ntex_dvi::write_dvi` 的输入）渲染为像素缓冲，并提供 PNG 导出。
//!
//! ## 与 plan 的偏差（重要）
//!
//! plan.md M8 条目选型为 tiny-skia。本 crate 首版落地时构建环境**离线**且
//! 本地 cargo 缓存无 tiny-skia（网络受限），故位图/PNG 编码为**纯 Rust 自研**
//! （zlib deflate + PNG chunk，见 [`png`]）；对外仍以 [`Backend`] trait 抽象，
//! [`TinySkiaBackend`] 仅是当前实现名（语义对齐 tiny-skia：RGBA8 预乘 sRGB
//! 软光栅）。待环境可联网，把 `tiny_skia` 作为 dev/实现替换即可，trait 不变。
//!
//! ## 坐标语义（对照 ntex-dvi / ntex-pdf）
//!
//! - 源单位 sp：`1pt = 65536sp`；像素 = `sp / 65_536 * dpi / 72`；
//! - 页面原点左上、y 向下（同 DVI）；盒子/规则的参考点语义与 `ntex_dvi`
//!   的 `hlist`/`vlist` 完全一致（见 [`raster`]）；
//! - **字符无字形**：TFM 只有度量，无轮廓（M9 字体子系统范围）。字符按
//!   任务简报画占位方框 + 基线标记示意，非最终排版效果。

#![deny(unsafe_code)]

pub mod png;
pub mod raster;
pub use raster::Pixmap;

mod backend;

pub use backend::{Backend, BackendError, RenderOptions, TinySkiaBackend};
