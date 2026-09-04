//! demo 驱动：读 .tex → `Typesetter::typeset_dvi` 排版 → 逐页渲染 PNG。
//!
//! 用法：`ntex-backend <input.tex> [output_prefix] [dpi]`
//! 输出：`<prefix>-<页码,01 起>.png`（默认前缀 = 输入文件名去扩展名）。

use std::path::Path;
use std::process::ExitCode;

use ntex_backend::{Backend, RenderOptions, TinySkiaBackend};
use ntex_layout::typeset::Typesetter;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args.len() > 4 {
        eprintln!("用法：ntex-backend <input.tex> [output_prefix] [dpi]");
        return ExitCode::from(2);
    }
    let tex_path = Path::new(&args[1]);
    let prefix = args.get(2).map(|s| s.to_owned()).unwrap_or_else(|| {
        tex_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "demo".to_owned())
    });
    let dpi: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(144.0);

    let source = match std::fs::read_to_string(tex_path) {
        Ok(src) => src,
        Err(err) => {
            eprintln!("读取 {} 失败：{}", tex_path.display(), err);
            return ExitCode::from(1);
        }
    };
    let (pages, fonts) = match Typesetter::with_tfm().typeset_dvi(&source) {
        Ok(out) => out,
        Err(err) => {
            eprintln!("排版失败：{}", err);
            return ExitCode::from(1);
        }
    };
    let opts = RenderOptions {
        dpi,
        ..RenderOptions::default()
    };
    let pngs = match TinySkiaBackend.render_pngs(&pages, &fonts, &opts) {
        Ok(pngs) => pngs,
        Err(err) => {
            eprintln!("渲染失败：{}", err);
            return ExitCode::from(1);
        }
    };
    for (idx, png) in pngs.iter().enumerate() {
        let out_path = format!("{prefix}-{:02}.png", idx + 1);
        if let Err(err) = std::fs::write(&out_path, png) {
            eprintln!("写 {} 失败：{}", out_path, err);
            return ExitCode::from(1);
        }
        println!(
            "{} → {}（{} 字节）",
            tex_path.display(),
            out_path,
            png.len()
        );
    }
    if pngs.is_empty() {
        eprintln!("没有可渲染的页面（\\shipout 未触发？）");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
