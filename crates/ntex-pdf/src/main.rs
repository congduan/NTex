//! `ntex-pdf <input.dvi> [output.pdf] [-p <W>x<H>]`：DVI → PDF 转换（正式后端）。
//!
//! 页面尺寸以 pt 给出（如 `-p 595.28x841.89`），缺省 A4。DVI 本身不含页面
//! 尺寸信息，需显式指定（与 dvipdfmx 的 `-p` 同理）。

use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法：ntex-pdf <input.dvi> [output.pdf] [-p <W>x<H>]");
        return ExitCode::FAILURE;
    }
    let input = &args[1];
    let mut output = format!("{}.pdf", input.trim_end_matches(".dvi"));
    let mut opts = ntex_pdf::PdfOptions::default();

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "-p" => {
                let Some(spec) = args.get(i + 1) else {
                    eprintln!("-p 后缺少页面尺寸");
                    return ExitCode::FAILURE;
                };
                match parse_papersize(spec) {
                    Some((w, h)) => opts.page_size = (w, h),
                    None => {
                        eprintln!("页面尺寸格式非法：{spec}（应为 <W>x<H>，单位 pt）");
                        return ExitCode::FAILURE;
                    }
                }
                i += 2;
            }
            // 非选项参数：输出文件名（跳过 -p 的值）
            other if !other.starts_with('-') => {
                output = other.to_owned();
                i += 1;
            }
            _ => i += 1,
        }
    }

    let dvi_bytes = match fs::read(input) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("读取 {input} 失败：{e}");
            return ExitCode::FAILURE;
        }
    };
    match ntex_pdf::convert(&dvi_bytes, &opts) {
        Ok(pdf) => match fs::write(&output, &pdf) {
            Ok(()) => {
                let n_pages = pdf
                    .windows(12)
                    .filter(|w| w == b"/Type /Page ")
                    .count();
                println!("已写出 {output}（{} 字节，{} 页）", pdf.len(), n_pages);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("写出失败：{e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("转换失败：{e}");
            ExitCode::FAILURE
        }
    }
}

/// 解析 `<W>x<H>`（pt，浮点）。
fn parse_papersize(spec: &str) -> Option<(f64, f64)> {
    let (w, h) = spec.split_once(['x', 'X'])?;
    Some((w.parse().ok()?, h.parse().ok()?))
}
