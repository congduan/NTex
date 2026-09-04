//! # ntex-pdf：DVI → PDF 转换器（正式输出后端，M8）。
//!
//! 取代临时 Helvetica 切片：解析 `ntex-dvi` 产出的 DVI，写出 PDF 1.4——
//! Type1 字体以 PFB 嵌入、绝对坐标定位字符、`re f` 填充规则。
//! 多字体（M8，2026-09-04 验证）：CM 全家族（cmr10/cmbx10/cmss12/cmtt10/
//! cmmi10/cmsy10/cmex10 等多号数）同链嵌入——`/BaseFont`/`/FontDescriptor`
//! 取 PFB 内 `/FontName`（大写），`/FontFile` 原样嵌 PFB；每页 `/Resources
//! /Font` 只列该页实际引用的字体（按 [`DrawOp::Char`] 归集），跨页复用
//! 同一字体字典对象、无资源冲突（demo-multi.tex：6 族 8 页结构抽查全对）。
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
