//! `ntex-dvi <input.tex> [output.dvi] [--input-path <dir>]... [--quiet]`：
//! 排版（TFM 模式）并写出 DVI（M3-5）。
//!
//! 输入为纯 TeX 源码；`\font\cs=<名字>` 加载 TFM（cmr10 等），
//! `\shipout<hbox|vbox>` 产出页面。输出供 dvipdfmx/xdvi 等驱动渲染。
//!
//! `--input-path <dir>`（可重复，先加先试）：`\input` 文件解析的搜索路径
//! （TEXINPUTS 语义最小子集，格式预载 G1）。缺省只有 cwd。
//! plain 格式预载（格式预载 G2(a)）：默认开——启动时先跑内嵌 plain.tex
//! （等价源首行 `\input plain`），`\input plain`/`\input hyphen` 在本地
//! 文件落空时改读内嵌资源（无 TinyTeX 树也能跑 plain 文档）。
//! `--no-plain`：关掉预载（回 INITEX 裸表；内嵌文件兜底仍在）。
//! `--quiet`：关掉 stderr 转录（`\message`/`\show`/`\write16`/错误恢复文本，
//! 格式预载 G0）。默认开——静默是当前最大的测量陷阱（plain-format-survey §2.4）。

use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let mut positional: Vec<&String> = Vec::new();
    let mut input_paths: Vec<String> = Vec::new();
    let mut quiet = false;
    let mut no_plain = false;
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--quiet" => quiet = true,
            "--no-plain" => no_plain = true,
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
        eprintln!(
            "用法：ntex-dvi <input.tex> [output.dvi] [--input-path <dir>]... [--no-plain] [--quiet]"
        );
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
    if no_plain {
        // `--no-plain` 语义 = 纯 iniTeX 起点（无预载格式）：catcode 表换用
        // tex.web §1273 INITEX 初表（`{`=12、NUL=9 …）。此前只跳过预载而
        // 保留 plain 表，与 pdfTeX `-ini` 不对齐（`\the\catcode0` 应为 9
        // 却是 12），导致 init 语义域的对拍失真——见 scripts/instrument-check.py。
        ts = ts.initex();
    }
    {
        // G1 搜索路径 + G2(a) 内嵌格式文件兜底：组合成 Local → 搜索前缀 → 内嵌
        // 三层（内嵌只答 plain.tex/hyphen.tex，且仅在前两层全落空时命中）。
        let mut base: Box<dyn ntex_io::Vfs> = Box::new(ntex_io::LocalVfs);
        if !input_paths.is_empty() {
            let mut vfs = ntex_io::SearchPathVfs::new(base);
            for p in &input_paths {
                vfs.push_path(p);
            }
            base = Box::new(vfs);
        }
        ts.set_vfs(base);
        ts.use_embedded_format();
    }
    if !no_plain {
        // G2(a)：启动预载（等价源首行 `\input plain`）。
        ts.set_preload_plain(true);
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
