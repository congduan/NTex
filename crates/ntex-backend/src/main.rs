//! demo 驱动：读 .tex → `Typesetter::typeset_dvi` 排版 → 逐页渲染 PNG。
//!
//! 用法：`ntex-backend <input.tex> [output_prefix] [dpi] [--vello] [--glyphs]
//! [--debug] [--no-glyphs] [--input-path <dir>]... [--quiet]`
//! 输出：`<prefix>-<页码,01 起>.png`（默认前缀 = 输入文件名去扩展名）；
//! `--debug` 时文件名带 `-debug` 后缀，并叠加排版调试 overlay
//! （盒边界/glue/断点标记）。
//! `--vello` 走 GPU 后端（vello/wgpu，无头纹理回读）；缺省软光栅。
//! 真字形通道（字符走字体轮廓 + `fill_polygon`，非占位方框）**两个后端都支持**：
//! vello 下默认开启，软光栅下经 `--glyphs` 显式开启（默认关，保持既有
//! 方框差分/快照口径零变化）；`--no-glyphs` 无条件回落方框。
//! `--input-path <dir>`（可重复，先加先试）：`\input` 文件解析的搜索路径
//! （TEXINPUTS 语义最小子集，格式预载 G1）。缺省只有 cwd。
//! `--quiet`：关掉 stderr 转录（`\message`/`\show`/`\write16`/错误恢复文本，
//! 格式预载 G0）。默认开——静默是最大的测量陷阱。

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
            "用法：ntex-backend <input.tex> [output_prefix] [dpi] [--vello] [--glyphs] [--debug] [--no-glyphs] [--input-path <dir>]... [--quiet]"
        );
        return ExitCode::from(2);
    }
    // flag 解析：`--input-path` 吃一个值；未知 `-` 前缀参数报错；positional 1..=3 个。
    let mut positional: Vec<&String> = Vec::new();
    let mut input_paths: Vec<String> = Vec::new();
    let mut use_vello = false;
    let mut debug = false;
    let mut no_glyphs = false;
    let mut glyphs_flag = false;
    let mut quiet = false;
    // LaTeX 格式快照（--fmt x.fmt）：载入后不再预载 plain（两者互斥）。
    let mut load_fmt: Option<String> = None;
    // CJK 字体回落名（如 FandolSong-Regular）：码位 >0xFF 的字符走它，
    // 与 wasm 前端 set_fallback_font 同一引擎通路（182a113）。
    let mut cjk_fallback: Option<String> = None;
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--vello" => use_vello = true,
            "--glyphs" => glyphs_flag = true,
            "--debug" => debug = true,
            "--no-glyphs" => no_glyphs = true,
            "--quiet" => quiet = true,
            "--fmt" => match it.next() {
                Some(p) => load_fmt = Some(p.clone()),
                None => {
                    eprintln!("--fmt 需要一个 .fmt 输入路径参数");
                    return ExitCode::from(2);
                }
            },
            "--cjk-fallback" => match it.next() {
                Some(p) => cjk_fallback = Some(p.clone()),
                None => {
                    eprintln!("--cjk-fallback 需要一个字体名参数（如 FandolSong-Regular）");
                    return ExitCode::from(2);
                }
            },
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
    if positional.is_empty() || positional.len() > 3 {
        eprintln!(
            "用法：ntex-backend <input.tex> [output_prefix] [dpi] [--vello] [--debug] [--no-glyphs] [--input-path <dir>]... [--quiet]"
        );
        return ExitCode::from(2);
    }
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
    let mut ts = Typesetter::with_tfm();
    // CJK 字体回落（码位 >0xFF 的字符走它；须在排版前注入）。
    if let Some(name) = &cjk_fallback {
        ts.set_fallback_font(Some(name.clone()));
    }
    // 格式预载（G2(a)/G4）：与 ntex-dvi 驱动同路径——内嵌 plain 兜底 +
    // 启动预载（等价源首行 `\input plain`）。缺此则 `\hsize`/`\baselineskip`
    // 等 plain 定义全缺，corpus 样例产出空页（survey §5.bis #6）。
    ts.use_embedded_format();
    // --fmt x.fmt：载入 LaTeX 格式快照（fmt 优先于 plain 预载，两者互斥；
    // 与 ntex-dvi 同口径）。
    if let Some(fmt_path) = &load_fmt {
        let data = match std::fs::read(fmt_path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("读取 {fmt_path} 失败：{e}");
                return ExitCode::from(2);
            }
        };
        let state = match ntex_format::load(&mut &data[..]) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("解析 {fmt_path} 失败：{e}");
                return ExitCode::from(2);
            }
        };
        ts.import_state(state);
    } else {
        ts.set_preload_plain(true);
    }
    if !input_paths.is_empty() || load_fmt.is_some() {
        // 发行版默认搜索链（与 ntex-dvi default_vfs 同口径）：显式 --input-path
        // 排最前，随后入仓 tex-minimal。--fmt（LaTeX）时 latex.ltx 的
        // \input/\usepackage 需要它能找到 article.cls 等资产文件。
        let mut vfs = ntex_io::SearchPathVfs::new(Box::new(ntex_io::LocalVfs));
        for p in &input_paths {
            vfs.push_path(p.clone());
        }
        let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/tex-minimal/tex");
        if bundled.exists() {
            vfs.push_path(bundled.to_string_lossy().into_owned());
            vfs.push_path(bundled.join("latex/base").to_string_lossy().into_owned());
        }
        ts.set_vfs(Box::new(ntex_io::KpsewhichVfs::new(Box::new(vfs))));
        // set_vfs 会替换 VFS → 重新包一层内嵌兜底（幂等由 installed 标志保证）
        ts.use_embedded_format();
    }
    let outcome = ts.typeset_dvi(&source);
    // G0：转录透传（成功/失败两条路都取；失败时 finish 未走，转录仍在 sink）。
    let transcript = ts.take_transcript();
    if !quiet && !transcript.is_empty() {
        let mut t = transcript;
        if !t.ends_with('\n') {
            t.push('\n');
        }
        eprint!("{t}");
    }
    let (pages, fonts) = match outcome {
        Ok(out) => out,
        Err(err) => {
            eprintln!("排版失败：{}", err);
            return ExitCode::from(1);
        }
    };
    let opts = RenderOptions {
        dpi,
        debug,
        // 真字形通道：两个后端都实现（软光栅走 `fill_polygon`）。
        // vello 默认开（历史口径）；软光栅默认关，经 `--glyphs` 显式开启，
        // 以保持既有方框差分/快照口径零变化；`--no-glyphs` 无条件回落方框。
        glyphs: !no_glyphs && (use_vello || glyphs_flag),
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
