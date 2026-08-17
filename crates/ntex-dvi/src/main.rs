//! `ntex-dvi <input.tex> [output.dvi]`：排版（TFM 模式）并写出 DVI（M3-5）。
//!
//! 输入为纯 TeX 源码；`\font\cs=<名字>` 加载 TFM（cmr10 等），
//! `\shipout<hbox|vbox>` 产出页面。输出供 dvipdfmx/xdvi 等驱动渲染。

use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法：ntex-dvi <input.tex> [output.dvi]");
        return ExitCode::FAILURE;
    }
    let input = &args[1];
    let output = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| format!("{}.dvi", input.trim_end_matches(".tex")));
    let text = match fs::read_to_string(input) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("读取 {input} 失败：{e}");
            return ExitCode::FAILURE;
        }
    };
    let mut ts = ntex_layout::typeset::Typesetter::with_tfm();
    let (pages, fonts) = match ts.typeset_dvi(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("排版失败：{e}");
            return ExitCode::FAILURE;
        }
    };
    if pages.is_empty() {
        eprintln!("未产出页面（源码缺少 \\shipout）");
        return ExitCode::FAILURE;
    }
    let dvi = ntex_dvi::write_dvi(&pages, &fonts);
    match fs::write(&output, &dvi) {
        Ok(()) => {
            println!(
                "已写出 {output}（{} 字节，{} 页，{} 字体）",
                dvi.len(),
                pages.len(),
                fonts.len()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("写出失败：{e}");
            ExitCode::FAILURE
        }
    }
}
