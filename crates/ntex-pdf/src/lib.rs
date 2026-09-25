//! # ntex-pdf：DVI → PDF 转换器（正式输出后端，M8）。
//!
//! 取代临时 Helvetica 切片：解析 `ntex-dvi` 产出的 DVI，写出 PDF 1.4——
//! TFM 8-bit 字体可复用预览 OTF（slot→CID 的 Type0/CFF）或以 Type1 PFB
//! 嵌入；字符绝对定位，规则以 `re f` 填充。
//! Unicode 直映字体（中文 Fandol 等，M9）走 Type0/CIDFontType0，内容流里
//! 字符以两字节十六进制串写出，**串值 = 字体 CFF charset 里的真 CID**（中文
//! Fandol 即 Adobe-GB1 CID，见 [`cid`]；写 Unicode 会让查看器 CID→字形
//! 查表落空、整页空白——2026-09-13 修复）；嵌入取 sfnt 内**裸 CFF 表**
//! （`/Subtype /CIDFontType0C`，见 [`cid::bare_cff`]；整包 OTTO 以
//! `/OpenType` 嵌入时 poppler/CoreGraphics 会把同一 CID 解析到错误字形）。
//! Type1（[`type1`]）除 PFB 外还写 `/Widths`（TFM 同源，缺它查看器推进
//! 为 0、整行字形叠架）与 `/Encoding /Differences`（PFB 内建编码，缺它
//! CoreGraphics 按 StandardEncoding 兜底、控制区字形画成 notdef）。
//!
//! 多字体（M8，2026-09-04 验证）：CM 全家族（cmr10/cmbx10/cmss12/cmtt10/
//! cmmi10/cmsy10/cmex10 等多号数）同链嵌入——`/BaseFont`/`/FontDescriptor`
//! 取 PFB 内 `/FontName`（大写），`/FontFile` 原样嵌 PFB；每页 `/Resources
//! /Font` 只列该页实际引用的字体（按 [`DrawOp::Char`] 归集），跨页复用
//! 同一字体字典对象、无资源冲突（demo-multi.tex：6 族 8 页结构抽查全对）。
//!
//! 用法：`ntex-pdf <input.dvi> [output.pdf] [-p <W>x<H>]`（页面尺寸 pt，默认 A4）；
//! 或库 API [`convert`]。

pub mod cid;
pub mod dvi;
pub mod image;
pub mod otf;
pub mod pdf;
pub mod type1;

use std::io;

pub use dvi::{parse as parse_dvi, DrawOp, Dvi, Page};
pub use pdf::{clear_image_bytes, register_image_bytes, write_pdf, PdfOptions};

/// 读入 DVI 字节并写出 PDF（页面尺寸默认 A4）。
pub fn convert(dvi_bytes: &[u8], opts: &PdfOptions) -> io::Result<Vec<u8>> {
    let dvi = dvi::parse(dvi_bytes)?;
    pdf::write_pdf(&dvi, opts)
}
