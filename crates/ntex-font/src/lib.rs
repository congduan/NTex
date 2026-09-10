//! # ntex-font
//!
//! NTex 字体层（M3-4）：TFM/OFM 解析与字体度量。
//!
//! - [`otf`]：OTF/TTF 静态解析（M9 §11.0：cmap/hmtx/轮廓，Unicode 语义）；
//!   [`otf::build_metrics`] 把 OpenType 字体构建为引擎侧 [`FontMetrics`]
//!   （`unicode_native`，CJK 直映），[`otf::find_otf`] 按名字定位字体文件
//! - [`tfm`]：TFM（TeX Font Metrics）解析（TeXbook 附录 F），产出 [`FontMetrics`]
//!   （字符 width/height/depth + 字体参数，单位 sp）；[`FontMetrics::scaled_by`]
//!   按 `at`/`scaled` 规格缩放；[`tfm::find_tfm`] 按名字查找 TFM 文件。

#![deny(unsafe_code)]

pub mod otf;
pub mod tfm;

pub use otf::{build_metrics, find_otf, OtfError, OtfFont};
pub use tfm::{find_tfm, parse_tfm, FontMetrics, LigKern, LigKernStep, TfmError};
