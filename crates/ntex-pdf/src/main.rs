//! `ntex-pdf <input.tex> [output.pdf]`：最小 PDF 渲染（临时切片）。

use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法：ntex-pdf <input.tex> [output.pdf]");
        return ExitCode::FAILURE;
    }
    let input = &args[1];
    let output = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| format!("{}.pdf", input.trim_end_matches(".tex")));
    let text = match fs::read_to_string(input) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("读取 {input} 失败：{e}");
            return ExitCode::FAILURE;
        }
    };
    match ntex_pdf::render(&text) {
        Ok(bytes) => match fs::write(&output, &bytes) {
            Ok(()) => {
                println!("已写出 {output}（{} 字节）", bytes.len());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("写出失败：{e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("排版失败：{e}");
            ExitCode::FAILURE
        }
    }
}
