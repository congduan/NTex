//! `ntex-dvi <input.tex> [output.dvi] [--input-path <dir>]... [--quiet]`：
//! 排版（TFM 模式）并写出 DVI（M3-5）。
//!
//! 输入为纯 TeX 源码；`\font\cs=<名字>` 加载 TFM（cmr10 等），
//! `\shipout<hbox|vbox>` 产出页面。输出供 dvipdfmx/xdvi 等驱动渲染。
//!
//! `--input-path <dir>`（可重复，先加先试）：`\input` 文件解析的搜索路径
//! （TEXINPUTS 语义最小子集，格式预载 G1）。缺省只有 cwd。
//! `--quiet`：关掉 stderr 转录（`\message`/`\show`/`\write16`/错误恢复文本，
//! 格式预载 G0）。默认开——静默是当前最大的测量陷阱（plain-format-survey §2.4）。

use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let mut positional: Vec<&String> = Vec::new();
    let mut input_paths: Vec<String> = Vec::new();
    let mut quiet = false;
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--quiet" => quiet = true,
            "--input-path" => match it.next() {
                Some(p) => input_paths.push(p.clone()),
                None => {
                    eprintln!("--input-path 需要一个目录参数");
                    return ExitCode::from(2);
                }
            },
            other if other.starts_with('-') => {
                eprintln!("未知参数：{other}");
                return ExitCode::from(2);
            }
            _ => positional.push(a),
        }
    }
    if positional.is_empty() || positional.len() > 2 {
        eprintln!("用法：ntex-dvi <input.tex> [output.dvi] [--input-path <dir>]... [--quiet]");
        return ExitCode::from(2);
    }
    let input = positional[0];
    let output = positional
        .get(1)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}.dvi", input.trim_end_matches(".tex")));
    let text = match fs::read_to_string(input) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("读取 {input} 失败：{e}");
            return ExitCode::FAILURE;
        }
    };
    let mut ts = ntex_layout::typeset::Typesetter::with_tfm();
    if !input_paths.is_empty() {
        let mut vfs = ntex_io::SearchPathVfs::new(Box::new(ntex_io::LocalVfs));
        for p in &input_paths {
            vfs.push_path(p);
        }
        ts.set_vfs(Box::new(vfs));
    }
    let outcome = ts.typeset_dvi(&text);
    // G0：转录透传。成功/失败两条路都取（失败时 finish 未走，转录仍在 sink）。
    let transcript = ts.take_transcript();
    if !quiet && !transcript.is_empty() {
        let mut t = transcript;
        if !t.ends_with('\n') {
            t.push('\n');
        }
        eprint!("{t}");
    }
    let (pages, fonts) = match outcome {
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
