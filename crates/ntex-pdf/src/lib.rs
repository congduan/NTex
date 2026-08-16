//! # ntex-pdf：最小 PDF 输出（临时切片）。
//!
//! 目的：尽早让"一张能看的 PDF"跑通，验证 M1~M3 流水线（token → 节点树）。
//! 临时手段（后续替换）：
//! - 字体：Helvetica 标准字体（免嵌入）+ 硬编码宽度表（[`font`]）；
//! - 折行：贪心（`ntex_layout::linebreak::simple_lines`），M3-3 Knuth-Plass 替换；
//! - 度量：上伸/下伸近似，M3-4 TFM 替换。
//!
//! 用法：`ntex-pdf <input.tex> [output.pdf]`（或库 API [`render`]）。

pub mod font;

use std::io;

use ntex_core::SP_PER_PT;
use ntex_layout::node::{BoxKind, Node};
use ntex_layout::typeset::Typesetter;

pub use font::{helvetica_metrics, helvetica_space, slice_metrics};

/// 页面尺寸（A4，pt）。
pub const PAGE_WIDTH_PT: f64 = 595.28;
pub const PAGE_HEIGHT_PT: f64 = 841.89;
/// 页边距（pt）。
pub const MARGIN_PT: f64 = 72.0;

/// 排版源码并写出 PDF。
pub fn render(text: &str) -> io::Result<Vec<u8>> {
    let (metrics, space) = slice_metrics();
    let mut ts = Typesetter::with_metrics(metrics).with_space(space);
    let main = ts
        .typeset(text)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    write_pdf(&main)
}

/// 把主垂直列表写成 PDF 字节。
pub fn write_pdf(main: &[Node]) -> io::Result<Vec<u8>> {
    let content = build_content(main);
    build_document(&content)
}

/// 生成内容流（文本与规则）。
fn build_content(main: &[Node]) -> String {
    let max_width = (PAGE_WIDTH_PT - 2.0 * MARGIN_PT) * SP_PER_PT as f64;
    let mut out = String::new();
    // 当前基线 y（PDF 坐标，向上为正）；规则等用 y 作参考点
    let mut y = PAGE_HEIGHT_PT - MARGIN_PT;
    render_vlist(main, &mut out, &mut y, max_width as i64);
    out
}

/// 渲染垂直列表（递归处理 vbox 内容）。
/// 段落已由排版器（Knuth-Plass）拆成行 hbox，此处每个 hbox 渲染为一行。
fn render_vlist(nodes: &[Node], out: &mut String, y: &mut f64, _max_width: i64) {
    for node in nodes {
        match node {
            Node::Box(b) if b.kind == BoxKind::HBox => {
                let mut x = MARGIN_PT;
                for n in &b.children {
                    match n {
                        Node::Char { charcode, .. } => {
                            let s = escape_char(*charcode);
                            out.push_str(&format!(
                                "BT /F1 {} Tf {x:.3} {:.3} Td ({s}) Tj ET\n",
                                font::FONT_SIZE_PT,
                                y
                            ));
                            x += font::char_width_pt(*charcode);
                        }
                        Node::Glue { width, .. } | Node::Kern { width } => {
                            x += *width as f64 / SP_PER_PT as f64;
                        }
                        // 行内盒（缩进盒等）：按其宽度推进
                        Node::Box(inner) => {
                            x += inner.width as f64 / SP_PER_PT as f64;
                        }
                        Node::Rule { width, height, depth } => {
                            draw_rule(
                                out,
                                x,
                                *y,
                                *width as f64 / SP_PER_PT as f64,
                                *height as f64 / SP_PER_PT as f64,
                                *depth as f64 / SP_PER_PT as f64,
                            );
                            x += *width as f64 / SP_PER_PT as f64;
                        }
                        _ => {}
                    }
                }
                *y -= font::LINE_ADVANCE_PT;
            }
            Node::Box(b) if b.kind == BoxKind::VBox => {
                // vbox 内容在当前位置渲染；基线推进由内部行/胶水完成（切片近似）
                render_vlist(&b.children, out, y, _max_width);
            }
            // 垂直列表里的胶水/字距：基线推进
            Node::Glue { width, .. } | Node::Kern { width } => {
                *y -= *width as f64 / SP_PER_PT as f64;
            }
            // 垂直规则：横贯文本区，参考点在顶部
            Node::Rule { width, height, depth } => {
                let w = if *width <= 0 {
                    PAGE_WIDTH_PT - 2.0 * MARGIN_PT
                } else {
                    *width as f64 / SP_PER_PT as f64
                };
                draw_rule(
                    out,
                    MARGIN_PT,
                    *y,
                    w,
                    *height as f64 / SP_PER_PT as f64,
                    *depth as f64 / SP_PER_PT as f64,
                );
            }
            _ => {}
        }
    }
}

/// 画填充矩形：`x` 为左缘，参考点 `y` 在规则顶（高向下伸、深继续向下）。单位 pt。
fn draw_rule(out: &mut String, x: f64, y: f64, w: f64, h: f64, d: f64) {
    out.push_str(&format!("{x:.3} {:.3} {w:.3} {:.3} re f\n", y - h - d, h + d));
}

/// PDF 字符串转义（WinAnsi 字节直出，控制字符转八进制）。
fn escape_char(charcode: u32) -> String {
    match charcode {
        0x28 => "\\(".to_owned(),  // (
        0x29 => "\\)".to_owned(),  // )
        0x5c => "\\\\".to_owned(), // \
        c if c < 0x20 || c == 0x7f => format!("\\{:03o}", c),
        c => (c as u8 as char).to_string(),
    }
}

/// 组装最小 PDF 文档（单页 A4，Helvetica 10pt，WinAnsi）。
fn build_document(content: &str) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut offsets = Vec::new();

    let obj = |buf: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &str| {
        offsets.push(buf.len());
        buf.extend_from_slice(body.as_bytes());
        buf.extend_from_slice(b"\n");
    };

    buf.extend_from_slice(b"%PDF-1.4\n");
    obj(&mut buf, &mut offsets, "1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj");
    obj(
        &mut buf,
        &mut offsets,
        "2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj",
    );
    obj(
        &mut buf,
        &mut offsets,
        &format!(
            "3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_WIDTH_PT} {PAGE_HEIGHT_PT}] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >> endobj"
        ),
    );
    obj(
        &mut buf,
        &mut offsets,
        "4 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >> endobj",
    );
    let stream = format!("5 0 obj << /Length {} >>\nstream\n{content}endstream\nendobj", content.len());
    obj(&mut buf, &mut offsets, &stream);

    // xref 表
    let xref_pos = buf.len();
    buf.extend_from_slice(&format!("xref\n0 {}\n", offsets.len() + 1).into_bytes());
    buf.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        buf.extend_from_slice(&format!("{off:010} 00000 n \n").into_bytes());
    }
    buf.extend_from_slice(&format!("trailer << /Size {} /Root 1 0 R >>\n", offsets.len() + 1).into_bytes());
    buf.extend_from_slice(&format!("startxref\n{xref_pos}\n%%EOF\n").into_bytes());
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_pdf() {
        let text = "Hello, world. This is the first NTex PDF.";
        let pdf = render(text).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.windows(5).any(|w| w == b"%%EOF"));
        // 内容流里应有字体与逐字符文本
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Helvetica"));
        assert!(content.contains("(H)"));
        assert!(content.contains("(w)"));
    }

    #[test]
    fn escapes_special_chars() {
        assert_eq!(escape_char(b'(' as u32), "\\(");
        assert_eq!(escape_char(b')' as u32), "\\)");
        assert_eq!(escape_char(b'\\' as u32), "\\\\");
        assert_eq!(escape_char(b'a' as u32), "a");
    }
}
