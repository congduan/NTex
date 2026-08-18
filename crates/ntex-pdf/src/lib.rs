//! # ntex-pdf：DVI → PDF 转换器（正式输出后端，M8）。
//!
//! 取代临时 Helvetica 切片：解析 `ntex-dvi` 产出的 DVI，写出 PDF 1.4——
//! Type1 字体（cmr10 等）以 PFA 嵌入、绝对坐标定位字符、`re f` 填充规则。
//!
//! 用法：`ntex-pdf <input.dvi> [output.pdf] [-p <W>x<H>]`（页面尺寸 pt，默认 A4）；
//! 或库 API [`convert`]。

pub mod dvi;
pub mod pdf;
pub mod type1;

use std::io;

pub use dvi::{parse as parse_dvi, DrawOp, Dvi, Page};
pub use pdf::{write_pdf, PdfOptions};

/// 读入 DVI 字节并写出 PDF（页面尺寸默认 A4）。
pub fn convert(dvi_bytes: &[u8], opts: &PdfOptions) -> io::Result<Vec<u8>> {
    let dvi = dvi::parse(dvi_bytes)?;
    pdf::write_pdf(&dvi, opts)
}
