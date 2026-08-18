//! PDF 写出器（M8 输出后端）：DVI 页面 → PDF 1.4 文档。
//!
//! - 每页一个内容流：同行字符按基线聚成一个 `TJ` 数组（同 dvipdfmx 的整行串，
//!   词内间隙为 0，避免 poppler 等提取器在逐字符定位下误判出幻影空格）；
//!   词间空隙插入空格字形并用调整量把后续字符精确落在 TFM 位置；
//! - 规则用 `re f` 填充矩形；
//! - 字体：Type1 嵌入（PFB 流，见 [`crate::type1`]），`/Encoding` 不指定——
//!   查看器用字体程序内建编码（cmr10 的 TeX 编码 = DVI 字符码，天然一致）；
//! - 坐标转换：DVI 原点在页左上、y 向下（sp）；PDF 原点在左下、y 向上（pt），
//!   字符参考点为基线（DVI 的 v 即基线）。

use std::io::{self, Write};

use crate::dvi::{DrawOp, Dvi};
use crate::type1::load_pfb;

/// DVI 单位：1pt = 65536sp（mag=1000）。
const SP_PER_PT: f64 = 65_536.0;

/// 转换参数。
pub struct PdfOptions {
    /// 页面尺寸（pt）：宽、高。
    pub page_size: (f64, f64),
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            page_size: (595.276, 841.890), // A4
        }
    }
}

/// 把解析出的 DVI 写成 PDF 字节。
pub fn write_pdf(dvi: &Dvi, opts: &PdfOptions) -> io::Result<Vec<u8>> {
    let (_, h_pt) = opts.page_size;
    let mut contents: Vec<Vec<u8>> = Vec::with_capacity(dvi.pages.len());
    for page in &dvi.pages {
        let mut c = Vec::new();
        // 当前行 run：连续、同字体、同基线的字符（(font, code, x_pt, y_pt)）
        let mut run: Vec<(u32, u8, f64, f64)> = Vec::new();
        for op in &page.ops {
            match op {
                DrawOp::Char { font, code, h, v } => {
                    let x = *h as f64 / SP_PER_PT;
                    let y = h_pt - *v as f64 / SP_PER_PT;
                    match run.last() {
                        Some(&(pf, _, _, py)) if pf != *font || (py - y).abs() > 0.001 => {
                            emit_line(&mut c, dvi, &run)?;
                            run.clear();
                        }
                        _ => {}
                    }
                    run.push((*font, *code, x, y));
                }
                DrawOp::Rule { h, v, width, height } => {
                    emit_line(&mut c, dvi, &run)?;
                    run.clear();
                    let x = *h as f64 / SP_PER_PT;
                    let w = *width as f64 / SP_PER_PT;
                    let y = h_pt - *v as f64 / SP_PER_PT;
                    let hgt = *height as f64 / SP_PER_PT;
                    if w > 0.0 && hgt > 0.0 {
                        writeln!(c, "{:.4} {:.4} {:.4} {:.4} re f", x, y, w, hgt)?;
                    }
                }
            }
        }
        emit_line(&mut c, dvi, &run)?;
        contents.push(c);
    }

    build_document(dvi, &contents, opts)
}

/// 写一行字符（一个 `TJ` 数组）。
///
/// `Tm` 定基线起点；逐字符用字体宽度推进。每个转移都发一个调整量
/// （`TJ` 数字：正数左移、负数右移，单位 1/1000 em），把下一字符精确落到
/// 其 TFM 位置；词间空隙以一个大调整量表达（poppler 等提取器据此识别为
/// 空格，同 dvipdfmx 的做法）。
fn emit_line(c: &mut Vec<u8>, dvi: &Dvi, run: &[(u32, u8, f64, f64)]) -> io::Result<()> {
    if run.is_empty() {
        return Ok(());
    }
    let (font, _, x0, y0) = run[0];
    let fm = &dvi.fonts[font as usize];
    let size = font_size_pt(fm);
    write!(
        c,
        "BT /F{} {:.6} Tf 1 0 0 1 {:.4} {:.4} Tm [({})",
        font + 1,
        size,
        x0,
        y0,
        escape_byte(run[0].1)
    )?;
    let mut prev_code = run[0].1;
    let mut prev_x = x0;
    for &(f, code, x, _) in &run[1..] {
        debug_assert_eq!(f, font, "run 内字体应一致");
        let w_prev = fm.char_metrics(prev_code as u32).0 as f64 / SP_PER_PT;
        let delta = x - prev_x;
        let adj = (w_prev - delta) * 1000.0 / size;
        if adj.abs() > 0.01 {
            write!(c, " {:.2}", adj)?;
        }
        write!(c, "({})", escape_byte(code))?;
        prev_code = code;
        prev_x = x;
    }
    writeln!(c, "] TJ ET")?;
    Ok(())
}

/// 字体的实际字号（pt）：design × scale / 2^20（`scaled_by` 后的 scale 字段）。
fn font_size_pt(fm: &ntex_font::FontMetrics) -> f64 {
    let sp = (fm.design_size_sp as i128 * fm.scale as i128) >> 20;
    sp as f64 / SP_PER_PT
}

/// PDF 字符串转义（字节直出；`( ) \` 与 <0x20、>=0x7F 转八进制）。
fn escape_byte(b: u8) -> String {
    match b {
        b'(' => "\\(".to_owned(),
        b')' => "\\)".to_owned(),
        b'\\' => "\\\\".to_owned(),
        c if !(0x20..0x7F).contains(&c) => format!("\\{:03o}", c),
        c => (c as char).to_string(),
    }
}

/// 组装 PDF 文档。
///
/// 对象布局：1 Catalog、2 Pages、每页 (Page, Contents) 两对象、
/// 每个唯一字体 (Font dict, FontDescriptor, FontFile 流) 三对象。
fn build_document(
    dvi: &Dvi,
    contents: &[Vec<u8>],
    opts: &PdfOptions,
) -> io::Result<Vec<u8>> {
    // 字体去重：name → (FontName, PFA)。找不到 PFA 时退化为不嵌入的空字典。
    let mut uniq: Vec<(&str, (String, Vec<u8>))> = Vec::new();
    for name in &dvi.font_names {
        if uniq.iter().any(|(n, _)| *n == name.as_str()) {
            continue;
        }
        let entry = match load_pfb(name) {
            Ok(f) => (f.name, f.pfb),
            Err(e) => {
                eprintln!("警告：{e}（以不嵌入方式引用字体）");
                (name.to_ascii_uppercase(), Vec::new())
            }
        };
        uniq.push((name.as_str(), entry));
    }
    // DVI 字体号 → PDF 字体字典对象号
    let n_pages = contents.len();
    let font_base = 3 + 2 * n_pages;
    let font_obj: Vec<u32> = dvi
        .font_names
        .iter()
        .map(|name| {
            let idx = uniq
                .iter()
                .position(|(n, _)| *n == name.as_str())
                .expect("字体名已注册");
            (font_base + 3 * idx) as u32
        })
        .collect();

    let (w_pt, h_pt) = opts.page_size;
    let mut buf = Vec::new();
    let mut offsets = Vec::new();
    let obj = |buf: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(buf.len());
        buf.extend_from_slice(body);
        buf.push(b'\n');
    };

    buf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    obj(&mut buf, &mut offsets, b"1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj");

    let mut kids = String::new();
    for i in 0..n_pages {
        if i > 0 {
            kids.push(' ');
        }
        kids.push_str(&format!("{} 0 R", 3 + 2 * i));
    }
    obj(
        &mut buf,
        &mut offsets,
        format!("2 0 obj << /Type /Pages /Kids [{kids}] /Count {n_pages} >> endobj").as_bytes(),
    );

    // 页面 + 内容流
    for (i, c) in contents.iter().enumerate() {
        let page_obj = 3 + 2 * i;
        let content_obj = 4 + 2 * i;
        let mut res = String::new();
        for (idx, fobj) in font_obj.iter().enumerate() {
            res.push_str(&format!("/F{} {fobj} 0 R ", idx + 1));
        }
        obj(
            &mut buf,
            &mut offsets,
            format!(
                "{page_obj} 0 obj << /Type /Page /Parent 2 0 R \
                 /MediaBox [0 0 {w_pt:.4} {h_pt:.4}] \
                 /Resources << /Font << {res}>> >> /Contents {content_obj} 0 R >> endobj"
            )
            .as_bytes(),
        );
        let mut body = format!("{content_obj} 0 obj << /Length {} >>\nstream\n", c.len())
            .into_bytes();
        body.extend_from_slice(c);
        body.extend_from_slice(b"\nendstream\nendobj");
        obj(&mut buf, &mut offsets, &body);
    }

    // 字体对象（去重）
    for (idx, (font_name, (_, pfa))) in uniq.iter().enumerate() {
        let dict_obj = font_base + 3 * idx;
        let desc_obj = dict_obj + 1;
        let file_obj = dict_obj + 2;
        if pfa.is_empty() {
            obj(
                &mut buf,
                &mut offsets,
                format!(
                    "{dict_obj} 0 obj << /Type /Font /Subtype /Type1 /BaseFont /{font_name} >> endobj"
                )
                .as_bytes(),
            );
            continue;
        }
        obj(
            &mut buf,
            &mut offsets,
            format!(
                "{dict_obj} 0 obj << /Type /Font /Subtype /Type1 /BaseFont /{font_name} \
                 /FontDescriptor {desc_obj} 0 R >> endobj"
            )
            .as_bytes(),
        );
        obj(
            &mut buf,
            &mut offsets,
            format!(
                "{desc_obj} 0 obj << /Type /FontDescriptor /FontName /{font_name} \
                 /Flags 4 /ItalicAngle 0 /Ascent 0 /Descent 0 /CapHeight 0 /StemV 0 \
                 /FontFile {file_obj} 0 R >> endobj"
            )
            .as_bytes(),
        );
        let mut body = format!("{file_obj} 0 obj << /Length {} >>\nstream\n", pfa.len())
            .into_bytes();
        body.extend_from_slice(pfa);
        body.extend_from_slice(b"\nendstream\nendobj");
        obj(&mut buf, &mut offsets, &body);
    }

    // xref / trailer
    let xref_pos = buf.len();
    buf.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
    buf.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        buf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    buf.extend_from_slice(
        format!(
            "trailer << /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
            offsets.len() + 1
        )
        .as_bytes(),
    );
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dvi::Page;
    use ntex_font::parse_tfm;

    #[test]
    fn escapes_pdf_string_bytes() {
        assert_eq!(escape_byte(b'('), "\\(");
        assert_eq!(escape_byte(b')'), "\\)");
        assert_eq!(escape_byte(b'\\'), "\\\\");
        assert_eq!(escape_byte(b'a'), "a");
        assert_eq!(escape_byte(0x00), "\\000");
        assert_eq!(escape_byte(0xFF), "\\377");
    }

    /// 用真实 cmr10 度量构造 Dvi（10pt、未缩放），供内容流测试。
    fn test_dvi() -> Option<Dvi> {
        let path = ntex_font::find_tfm("cmr10")?;
        let bytes = std::fs::read(path).ok()?;
        let mut fm = parse_tfm(&bytes).ok()?;
        fm.name = "cmr10".to_owned();
        fm.design_size_sp = 655_360;
        fm.scale = 1 << 20; // 设计字号 10pt，未缩放
        Some(Dvi {
            pages: Vec::new(),
            fonts: vec![fm],
            font_names: vec!["cmr10".to_owned()],
        })
    }

    #[test]
    fn run_groups_chars_and_adjusts_word_gaps() {
        let Some(dvi) = test_dvi() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let fm = &dvi.fonts[0];
        let w_t = fm.char_metrics(b'T' as u32).0 as f64 / SP_PER_PT;
        let w_h = fm.char_metrics(b'h' as u32).0 as f64 / SP_PER_PT;
        // 一行：T h 空格 h（词距 5pt）——词距以负调整量表达
        let run1 = vec![
            (0, b'T', 0.0, 10.0),
            (0, b'h', w_t, 10.0),
            (0, b'h', w_t + w_h + 5.0, 10.0),
        ];
        let mut c = Vec::new();
        emit_line(&mut c, &dvi, &run1).unwrap();
        let s = String::from_utf8(c).unwrap();
        assert!(s.starts_with("BT /F1 10.000000 Tf 1 0 0 1 0.0000 10.0000 Tm ["));
        // 词内相邻字符无调整量（delta == 字符宽），词距产生负调整量 -500
        assert!(s.contains("(T)(h)"), "词内不应有调整量：{s}");
        assert!(s.contains(" -500.00"), "词距应有 -500.00 调整量：{s}");
        assert!(s.ends_with("] TJ ET\n"), "{s}");
    }

    #[test]
    fn write_pdf_splits_lines_by_baseline_and_rules() {
        let Some(mut dvi) = test_dvi() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let fm = &dvi.fonts[0];
        let w_t = fm.char_metrics(b'T' as u32).0 as f64 / SP_PER_PT;
        let page = Page {
            ops: vec![
                DrawOp::Char {
                    font: 0,
                    code: b'T',
                    h: 0,
                    v: 0,
                },
                DrawOp::Rule {
                    h: 100,
                    v: 0,
                    width: 200,
                    height: 100,
                },
                DrawOp::Char {
                    font: 0,
                    code: b'T',
                    h: 0,
                    v: 655_360, // 下一行
                },
            ],
        };
        dvi.pages = vec![page];
        let pdf = write_pdf(&dvi, &PdfOptions {
            page_size: (100.0, 20.0),
        })
        .unwrap();
        let s = String::from_utf8_lossy(&pdf);
        // 两条 TJ 行（规则前后各一）+ 一条规则
        assert_eq!(s.matches("] TJ ET").count(), 2, "{s}");
        assert!(s.contains("re f\n"));
        // 第二行基线 = 页高 - v = 20 - 10 = 10pt
        assert!(s.contains("Tm [("), "{s}");
        let _ = w_t;
    }
}
