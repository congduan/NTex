//! demo 驱动：读 .tex → `Typesetter::typeset_dvi` 排版 → 逐页渲染 PNG。
//!
//! 用法：`ntex-backend <input.tex> [output_prefix] [dpi] [--vello]`
//! 输出：`<prefix>-<页码,01 起>.png`（默认前缀 = 输入文件名去扩展名）。
//! `--vello` 走 GPU 后端（vello/wgpu，无头纹理回读）；缺省软光栅。

use std::path::Path;
use std::process::ExitCode;

use ntex_backend::{Backend, RenderOptions, TinySkiaBackend, VelloBackend};
use ntex_layout::typeset::Typesetter;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args.len() > 5 {
        eprintln!("用法：ntex-backend <input.tex> [output_prefix] [dpi] [--vello]");
        return ExitCode::from(2);
    }
    let use_vello = args.iter().skip(1).any(|a| a == "--vello");
    let positional: Vec<&String> = args[1..].iter().filter(|a| !a.starts_with('-')).collect();
    let tex_path = Path::new(positional[0]);
    let prefix = positional.get(1).map(|s| s.to_string()).unwrap_or_else(|| {
        tex_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "demo".to_owned())
    });
    let dpi: f64 = positional
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(144.0);

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
    let backend_name = if use_vello { "vello(gpu)" } else { "软光栅" };
    let pngs = if use_vello {
        VelloBackend::new().render_pngs(&pages, &fonts, &opts)
    } else {
        TinySkiaBackend.render_pngs(&pages, &fonts, &opts)
    };
    let pngs = match pngs {
        Ok(pngs) => pngs,
        Err(err) => {
            eprintln!("渲染失败（{backend_name}）：{}", err);
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
            "{} → {}（{} 字节，{}，{} 页之 {}）",
            tex_path.display(),
            out_path,
            png.len(),
            backend_name,
            pngs.len(),
            idx + 1
        );
    }
    if pngs.is_empty() {
        eprintln!("没有可渲染的页面（\\shipout 未触发？）");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
