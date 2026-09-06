//! demo 驱动：读 .tex → `Typesetter::typeset_dvi` 排版 → 逐页渲染 PNG。
//!
//! 用法：`ntex-backend <input.tex> [output_prefix] [dpi] [--vello] [--debug]
//! [--no-glyphs]`
//! 输出：`<prefix>-<页码,01 起>.png`（默认前缀 = 输入文件名去扩展名）；
//! `--debug` 时文件名带 `-debug` 后缀，并叠加排版调试 overlay
//! （盒边界/glue/断点标记）。
//! `--vello` 走 GPU 后端（vello/wgpu，无头纹理回读）；缺省软光栅。
//! vello 后端默认渲染真字形（Latin Modern，kpsewhich/texlive 定位）；
//! `--no-glyphs` 回落占位方框口径（软光栅恒为方框口径）。

use std::path::Path;
use std::process::ExitCode;

#[cfg(feature = "vello")]
use ntex_backend::VelloBackend;
use ntex_backend::{Backend, RenderOptions, TinySkiaBackend};
use ntex_layout::typeset::Typesetter;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "用法：ntex-backend <input.tex> [output_prefix] [dpi] [--vello] [--debug] [--no-glyphs]"
        );
        return ExitCode::from(2);
    }
    // flag 解析：未知 `-` 前缀参数报错；positional 1..=3 个。
    for a in &args[1..] {
        match a.as_str() {
            "--vello" | "--debug" | "--no-glyphs" => {}
            other if other.starts_with('-') => {
                eprintln!("未知参数：{other}");
                return ExitCode::from(2);
            }
            _ => {}
        }
    }
    let positional: Vec<&String> = args[1..].iter().filter(|a| !a.starts_with('-')).collect();
    if positional.len() > 3 {
        eprintln!(
            "用法：ntex-backend <input.tex> [output_prefix] [dpi] [--vello] [--debug] [--no-glyphs]"
        );
        return ExitCode::from(2);
    }
    let use_vello = args.iter().skip(1).any(|a| a == "--vello");
    let debug = args.iter().skip(1).any(|a| a == "--debug");
    let no_glyphs = args.iter().skip(1).any(|a| a == "--no-glyphs");
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
        debug,
        // 真字形仅 vello 支持（软光栅内部强制回落方框口径）。
        glyphs: use_vello && !no_glyphs,
        ..RenderOptions::default()
    };
    let backend_name = if use_vello { "vello(gpu)" } else { "软光栅" };
    let pngs = if use_vello {
        #[cfg(feature = "vello")]
        {
            VelloBackend::new().render_pngs(&pages, &fonts, &opts)
        }
        // no-default-features 构建（wasm 薄壳链验证等）：GPU 路径未编译进。
        #[cfg(not(feature = "vello"))]
        {
            let _ = &opts;
            eprintln!("--vello 需要 vello feature（default 开启；本构建为纯软光栅）");
            return ExitCode::from(2);
        }
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
        let suffix = if debug { "-debug" } else { "" };
        let out_path = format!("{prefix}-{:02}{suffix}.png", idx + 1);
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
