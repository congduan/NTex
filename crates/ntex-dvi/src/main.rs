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
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Debug)]
struct AssetTfmSource {
    roots: Vec<PathBuf>,
}

impl ntex_layout::TfmSource for AssetTfmSource {
    fn tfm_bytes(&mut self, name: &str) -> Option<Vec<u8>> {
        let file = format!("{name}.tfm");
        // 直查各 root；未命中再扫一层子目录（assets/tfm/ec 等 T1 字体族目录）
        for root in &self.roots {
            let direct = root.join(&file);
            if direct.exists() {
                return fs::read(direct).ok();
            }
        }
        for root in &self.roots {
            let Ok(rd) = fs::read_dir(root) else {
                continue;
            };
            for entry in rd.flatten() {
                if entry.path().is_dir() {
                    let p = entry.path().join(&file);
                    if p.exists() {
                        return fs::read(p).ok();
                    }
                }
            }
        }
        None
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let mut positional: Vec<&String> = Vec::new();
    let mut input_paths: Vec<String> = Vec::new();
    let mut quiet = false;
    let mut no_plain = false;
    let mut dump_fmt: Option<String> = None;
    let mut load_fmt: Option<String> = None;
    let mut generate_fmt: Option<String> = None;
    // CJK 字体回落名（如 FandolSong-Regular）：码位 >0xFF 的字符走它，
    // 与 wasm 前端 set_fallback_font 同一引擎通路（182a113）。
    let mut cjk_fallback: Option<String> = None;
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--quiet" => quiet = true,
            "--no-plain" => no_plain = true,
            "--cjk-fallback" => match it.next() {
                Some(p) => cjk_fallback = Some(p.clone()),
                None => {
                    eprintln!("--cjk-fallback 需要一个字体名参数（如 FandolSong-Regular）");
                    return ExitCode::from(2);
                }
            },
            "--dump" => match it.next() {
                Some(p) => dump_fmt = Some(p.clone()),
                None => {
                    eprintln!("--dump 需要一个 .fmt 输出路径参数");
                    return ExitCode::from(2);
                }
            },
            "--generate-fmt" => match it.next() {
                Some(p) => generate_fmt = Some(p.clone()),
                None => {
                    eprintln!("--generate-fmt 需要一个 .fmt 输出路径参数");
                    return ExitCode::from(2);
                }
            },
            "--fmt" => match it.next() {
                Some(p) => load_fmt = Some(p.clone()),
                None => {
                    eprintln!("--fmt 需要一个 .fmt 输入路径参数");
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
    if generate_fmt.is_some() {
        if !positional.is_empty() {
            eprintln!("--generate-fmt 不接受输入文件参数");
            return ExitCode::from(2);
        }
        return generate_latex_fmt(
            generate_fmt.as_deref().unwrap_or_default(),
            &input_paths,
            quiet,
        );
    }
    if positional.is_empty() || positional.len() > 2 {
        eprintln!("{}", usage());
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
    let job_name = PathBuf::from(input)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("texput")
        .to_string();
    ts.set_job_name(job_name);
    install_distribution_tfm_source();
    if let Some(name) = &cjk_fallback {
        ts.set_fallback_font(Some(name.clone()));
    }
    if no_plain {
        // `--no-plain` 语义 = 纯 iniTeX 起点（无预载格式）：catcode 表换用
        // tex.web §1273 INITEX 初表（`{`=12、NUL=9 …）。此前只跳过预载而
        // 保留 plain 表，与 pdfTeX `-ini` 不对齐（`\the\catcode0` 应为 9
        // 却是 12），导致 init 语义域的对拍失真——见 scripts/instrument-check.py。
        ts = ts.initex();
    }
    // M7 fmt 快路径第一步：`--fmt x.fmt` 载入预存格式（expl3/latex.ltx 全量
    // 状态毫秒级恢复，载入不再每次 62s）。fmt 优先于 plain 预载——两者互斥
    // （fmt 已含目标格式全量状态，再叠 plain 会污染）。
    if let Some(fmt_path) = &load_fmt {
        let resolved = match resolve_format_path(fmt_path) {
            Some(p) => p,
            None => {
                eprintln!("找不到格式文件 {fmt_path}（已查 cwd、可执行文件同目录和 assets/fmt）");
                return ExitCode::FAILURE;
            }
        };
        let data = match fs::read(&resolved) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("读取 {} 失败：{e}", resolved.display());
                return ExitCode::FAILURE;
            }
        };
        let state = match ntex_format::load(&mut &data[..]) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("解析 {} 失败：{e}", resolved.display());
                return ExitCode::FAILURE;
            }
        };
        ts.import_state(state);
    }
    {
        // 第二十四刀：发行版默认搜索链。显式 --input-path 仍排最前；
        // 随后接便携 tex/、用户扩展、TEXINPUTS、入仓 tex-minimal、TinyTeX
        // 探测路径，最后再由 ntex-io 的 kpsewhich 层兜底。
        ts.set_vfs(default_vfs(&input_paths));
        ts.use_embedded_format();
    }
    if no_plain && load_fmt.is_none() {
        // --no-plain 且未显式给 fmt：纯 iniTeX 起点，不注入任何格式。
    } else if load_fmt.is_none() {
        // 格式自动检测（M9 中文刀 6，与 ntex-tauri ui/main.js::applyMode 同口径）：
        // 源含 \documentclass / \usepackage / \begin{document} 任一特征 →
        // LaTeX（载入 assets/fmt/latex.fmt）；否则 plain 预载（G2(a)，
        // 等价源首行 `\input plain`）。显式 --fmt 始终优先；--no-plain
        // 无特征时保持 plain 语义受控关闭。
        if looks_like_latex(&text) {
            if let Some(fmt_path) = resolve_format_path("latex.fmt") {
                match fs::read(&fmt_path) {
                    Ok(data) => match ntex_format::load(&mut &data[..]) {
                        Ok(mut state) => {
                            if !quiet {
                                eprintln!("[auto] 检测到 LaTeX 特征，载入 {}", fmt_path.display());
                            }
                            // pdflatex 语义（图片管线 Step A）：pdflatex 在 fmt 生成期
                            // 已置 \pdfoutput=1（misc 63，下标见 ntex-core
                            // free::int_param_index），graphics.cfg 据此选 pdftex.def，
                            // 图的自然尺寸走 `\pdfximage` 引擎通路。NTex 的 fmt 由
                            // latex.ltx 生成、无法在其 \dump 前注入（expl3 backend
                            // 会跟着翻成 pdftex 而 NTex 无 l3backend-pdftex.def），
                            // 故在 fmt 载入后、用户源前补种子。plain/iniTeX 作业
                            // 不经过此分支，\pdfoutput 默认 0 的 pdfTeX 原义不变。
                            // NTex 实际仍输出 DVI：该参数只作驱动选择信号。
                            state.params.misc[63] = 1;
                            ts.import_state(state);
                        }
                        Err(e) => {
                            eprintln!("[auto] 解析 {} 失败：{e}（回退 plain）", fmt_path.display());
                            ts.set_preload_plain(true);
                        }
                    },
                    Err(e) => {
                        eprintln!("[auto] 读取 {} 失败：{e}（回退 plain）", fmt_path.display());
                        ts.set_preload_plain(true);
                    }
                }
            } else {
                if !quiet {
                    eprintln!("[auto] 检测到 LaTeX 特征但找不到 latex.fmt（回退 plain）");
                }
                ts.set_preload_plain(true);
            }
        } else {
            ts.set_preload_plain(true);
        }
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
    // M7 fmt 快路径：`--dump x.fmt` 把（含 `\dump` 后的）全量状态存盘。
    // 语义对齐 tex.web：`\dump` 只在 INITEX 合法；这里宽松处理——若引擎报
    // dumped=false 则拒绝保存（防止把半载状态当格式分发）。
    if let Some(fmt_path) = &dump_fmt {
        if !ts.dumped() {
            eprintln!("--dump：源未执行 \\dump（或非 \\dump 收尾），拒绝保存不完整格式");
            return ExitCode::FAILURE;
        }
        let state = ts.export_state();
        let mut buf = Vec::new();
        if let Err(e) = ntex_format::save(&mut buf, &state) {
            eprintln!("序列化 {fmt_path} 失败：{e}");
            return ExitCode::FAILURE;
        }
        match fs::write(fmt_path, &buf) {
            Ok(()) => println!("已写出 {fmt_path}（{} 字节，格式快照）", buf.len()),
            Err(e) => {
                eprintln!("写出 {fmt_path} 失败：{e}");
                return ExitCode::FAILURE;
            }
        }
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
    let dvi = ntex_dvi::write_dvi_with_counts(&pages, ts.shipped_page_counts(), &fonts);
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

fn usage() -> &'static str {
    "用法：ntex-dvi <input.tex> [output.dvi] [--fmt <latex.fmt>] [--input-path <dir>]... [--no-plain] [--cjk-fallback <字体名>] [--quiet]\n\
     或：ntex-dvi --generate-fmt <output.fmt> [--input-path <dir>]... [--quiet]"
}

fn default_vfs(input_paths: &[String]) -> Box<dyn ntex_io::Vfs> {
    let mut vfs = ntex_io::SearchPathVfs::new(Box::new(ntex_io::LocalVfs));
    let mut paths = Vec::new();
    paths.extend(input_paths.iter().map(PathBuf::from));

    if let Some(exe_dir) = executable_dir() {
        add_tex_tree_paths(&mut paths, exe_dir.join("tex"));
    }
    if let Ok(home) = std::env::var("HOME") {
        add_tex_tree_paths(&mut paths, PathBuf::from(home).join(".ntex/tex"));
    }
    if let Ok(texinputs) = std::env::var("TEXINPUTS") {
        for p in texinputs.split(':').filter(|p| !p.is_empty()) {
            paths.push(PathBuf::from(p));
        }
    }
    if let Some(root) = bundled_tex_root() {
        add_tex_tree_paths(&mut paths, root);
    }
    if let Ok(home) = std::env::var("HOME") {
        let tiny = PathBuf::from(home).join(".TinyTeX/texmf-dist/tex");
        add_tex_tree_paths(&mut paths, tiny);
    }

    for p in paths {
        vfs.push_path(p.to_string_lossy().into_owned());
    }
    Box::new(ntex_io::KpsewhichVfs::new(Box::new(vfs)))
}

fn add_tex_tree_paths(out: &mut Vec<PathBuf>, root: PathBuf) {
    out.push(root.clone());
    out.push(root.join("latex/base"));
    push_children(out, &root.join("latex"));
    push_children(out, &root.join("generic"));
}

fn push_children(out: &mut Vec<PathBuf>, dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.push(path);
        }
    }
}

fn executable_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

fn install_distribution_tfm_source() {
    let mut roots = Vec::new();
    if let Some(exe_dir) = executable_dir() {
        roots.push(exe_dir.join("tfm"));
    }
    if let Ok(home) = std::env::var("HOME") {
        roots.push(PathBuf::from(home).join(".ntex/tfm"));
    }
    if let Some(root) = bundled_tfm_root() {
        roots.push(root);
    }
    ntex_layout::set_tfm_source(Box::new(AssetTfmSource { roots }));
}

fn bundled_tex_root() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/tex-minimal/tex");
    root.exists().then_some(root)
}

fn bundled_tfm_root() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/tfm");
    root.exists().then_some(root)
}

fn bundled_fmt_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fmt");
    dir.exists().then_some(dir)
}

/// LaTeX 源特征检测（M9 中文刀 6）：**实现已上移到
/// `ntex_layout::typeset::looks_like_latex`**（与 ntex-studio 工作台共用同一
/// 份实现，避免两处口径漂移——JS 侧 `ntex-tauri/ui/main.js::looksLikeLatex`
/// 是容器边界外的同口径副本）。检测前剥行注释，与 `applyMode` 同规则。
fn looks_like_latex(src: &str) -> bool {
    ntex_layout::typeset::looks_like_latex(src)
}

fn resolve_format_path(name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(name);
    if p.exists() {
        return Some(p);
    }
    let mut dirs = Vec::new();
    if let Some(exe_dir) = executable_dir() {
        dirs.push(exe_dir.clone());
        dirs.push(exe_dir.join("fmt"));
    }
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(home).join(".ntex/fmt"));
    }
    if let Some(dir) = bundled_fmt_dir() {
        dirs.push(dir);
    }
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.exists())
}

fn generate_latex_fmt(output: &str, input_paths: &[String], quiet: bool) -> ExitCode {
    let Some(init_path) = resolve_format_path("ltxinit.tex") else {
        eprintln!("找不到 ltxinit.tex（已查 cwd、可执行文件同目录和 assets/fmt）");
        return ExitCode::FAILURE;
    };
    let source = match fs::read_to_string(&init_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("读取 {} 失败：{e}", init_path.display());
            return ExitCode::FAILURE;
        }
    };
    let mut ts = ntex_layout::typeset::Typesetter::with_tfm().initex();
    install_distribution_tfm_source();
    ts.set_vfs(default_vfs(input_paths));
    ts.use_embedded_format();
    let outcome = ts.typeset_dvi(&source);
    let transcript = ts.take_transcript();
    if !quiet && !transcript.is_empty() {
        let mut t = transcript;
        if !t.ends_with('\n') {
            t.push('\n');
        }
        eprint!("{t}");
    }
    if let Err(e) = outcome {
        if !ts.dumped() {
            eprintln!("生成 fmt 失败：{e}");
            return ExitCode::FAILURE;
        }
    }
    if !ts.dumped() {
        eprintln!("生成 fmt 失败：ltxinit.tex 未执行 \\dump");
        return ExitCode::FAILURE;
    }
    let state = ts.export_state();
    let mut buf = Vec::new();
    if let Err(e) = ntex_format::save(&mut buf, &state) {
        eprintln!("序列化 {output} 失败：{e}");
        return ExitCode::FAILURE;
    }
    match fs::write(output, &buf) {
        Ok(()) => {
            println!("已写出 {output}（{} 字节，格式快照）", buf.len());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("写出 {output} 失败：{e}");
            ExitCode::FAILURE
        }
    }
}
