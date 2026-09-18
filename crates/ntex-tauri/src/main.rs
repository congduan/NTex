//! # ntex-tauri — Tauri 实时预览壳（M8：wasm 渲染形态）
//!
//! 与 `ntex-studio`（egui/vello 桌面形态）平行的**纯壳**工作台：排版与渲染
//! 全部在前端 WASM 内完成——`ntex-wasm`（B/C 档）的 `compile_document()`
//! 产出页树句柄，`Document::render_page()` 走软光栅出 RGBA「纹理」（真字形
//! 轮廓口径：前端 fetch `ui/fonts/` Latin Modern OTF 经 `set_glyph_font`
//! 注入，未注入字体回落占位方框），JS `putImageData` 上 canvas；Rust 侧
//! **只有一个只读资产命令**（`ntex_latex_bundle`，见下），引擎不进 Tauri
//! 进程（`make tauri` 前置步骤先用 wasm-bindgen 把绑定生成到 `ui/pkg/`）。
//!
//! **唯一的 Rust 侧命令：`ntex_latex_bundle`（2026-09-18，C 档接线）**——
//! LaTeX 要 `.fmt` 快照、`article.cls`/`*.sty`/`*.tfm` 三类文件，而 wasm 无
//! 文件系统，只能由宿主喂。这些资产在仓库 `assets/` 里（实测 821 条 ≈11.7 MB：
//! fmt 6.5 MB、tex 3.7 MB、TFM 1.5 MB），逐个文件走 IPC 要 821 次往返，故这里
//! 在 Rust 侧**打成一个包一次性交给前端**（容器格式见 `crates/ntex-wasm/src/lib.rs` 的
//! `BUNDLE_MAGIC`，两端是同一份契约）；前端拿到后调 `set_bundle` 灌进 wasm。
//! 命令是**只读、无副作用、不碰排版**的——它不违反"引擎不进 Tauri 进程"，
//! 只是把发行资产递过去。
//!
//! 另一个 Rust 侧逻辑是**下载落盘**（PDF 导出）：macOS 的 WKWebView 在没有
//! 下载处理器时对 `<a download>` 的默认策略是 **Cancel**（wry 导航委托的
//! `has_download_handler` 分支），点导出会静默无反应——故窗口改在 `setup`
//! 里经 `WebviewWindowBuilder::from_config` 手工构建并挂 `on_download`。
//! 放行后 wry 自动写「下载」目录（`Finished` 的 path 在 macOS 恒为空，
//! 是 wry 注明的 API 限制，成败经 `success` 布尔回传前端）。
//! 依赖面刻意保持最小：tauri 仅开窗口（wkwebview/WebView2），无插件。
//! 分工对照见 AGENTS.md 工作区结构表与 `crates/ntex-wasm/README.md` 三档路线。

// Windows 发布构建无控制台窗口（dev 构建保留，便于看日志）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use tauri::ipc::Response;
use tauri::webview::DownloadEvent;

/// 发行资产包魔数——**与 `crates/ntex-wasm/src/lib.rs` 的 `BUNDLE_MAGIC` 是同一
/// 契约**（两端改一处必须同步另一处；容器格式在那边的文档注释里）。
const BUNDLE_MAGIC: &[u8] = b"NTEXBND1";

/// 打包好的资产包缓存（一次读盘 ≈12 MB；前端启动只取一次，不该反复打包）。
static BUNDLE: LazyLock<Mutex<Option<Vec<u8>>>> = LazyLock::new(|| Mutex::new(None));

/// 找发行资产根（含 `fmt/latex.fmt` 与 `tex-minimal/tex` 的那层 `assets/`）。
///
/// 顺序与 `ntex-dvi` 的发行搜索链同精神（先环境变量、再可执行文件旁、再入仓
/// 开发树），`NTEX_ASSETS` 可显式覆盖（打包/自定义树用）。
fn assets_root() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("NTEX_ASSETS") {
        candidates.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("assets"));
            candidates.push(dir.join("../../assets"));
        }
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets"));
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("assets"));
    }
    candidates
        .into_iter()
        .find(|root| root.join("fmt/latex.fmt").is_file() && root.join("tex-minimal/tex").is_dir())
}

/// 递归收集待打包文件。
///
/// 键的取法按 kind 分两种（**踩过**：TFM 用带扩展名的文件名会在 wasm 侧查不中）：
/// - `kind 0`（TeX 文件）用**带扩展名的主名**（`article.cls`）——`\input`/
///   `\usepackage` 就是按这个名字找的；
/// - `kind 1`（TFM 度量）用**去掉扩展名的字干**（`cmbx12`）——`TfmSource::tfm_bytes`
///   拿到的是裸字体名。
fn collect_assets(
    dir: &Path,
    kind: u8,
    ext: Option<&str>,
    seen: &mut HashSet<(u8, String)>,
    out: &mut Vec<(u8, String, PathBuf)>,
) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("读取目录 {} 失败：{e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_assets(&path, kind, ext, seen, out)?;
            continue;
        }
        if let Some(ext) = ext {
            if path.extension().and_then(|s| s.to_str()) != Some(ext) {
                continue;
            }
        }
        let key = match kind {
            1 => path.file_stem().and_then(|s| s.to_str()),
            _ => path.file_name().and_then(|s| s.to_str()),
        };
        let Some(name) = key else {
            continue;
        };
        // 主名去重：`tex/latex/base` 与 `tex/latex/l3kernel` 下有同名副本
        // （expl3-code.tex 1.4 MB ×2 等），wasm 侧 MemVfs 按键覆盖（后写赢），
        // 先去重省掉 ~1.5 MB 重复传输，语义不变。
        if seen.insert((kind, name.to_owned())) {
            out.push((kind, name.to_owned(), path));
        }
    }
    Ok(())
}

/// 把发行资产打成 C 档资产包（容器契约见 `crates/ntex-wasm` 的 `BUNDLE_MAGIC`）。
///
/// 条目三段：`tex-minimal/tex/**`（键 = 文件主名，kind 0）、`tfm/**/*.tfm`
/// （kind 1）、`fmt/latex.fmt`（kind 2）。TFM 只在资产树里取，不取内嵌 48 件——
/// 内嵌那些在 wasm 侧本来就查得到，重复传输没有意义。
fn build_latex_bundle(root: &Path) -> Result<Vec<u8>, String> {
    let mut files: Vec<(u8, String, PathBuf)> = Vec::new();
    let mut seen: HashSet<(u8, String)> = HashSet::new();
    collect_assets(
        &root.join("tex-minimal/tex"),
        0,
        None,
        &mut seen,
        &mut files,
    )?;
    collect_assets(&root.join("tfm"), 1, Some("tfm"), &mut seen, &mut files)?;
    let fmt_path = root.join("fmt/latex.fmt");
    let fmt =
        std::fs::read(&fmt_path).map_err(|e| format!("读取 {} 失败：{e}", fmt_path.display()))?;

    let mut out: Vec<u8> = Vec::with_capacity(fmt.len() + 4 * 1024 * 1024);
    out.extend_from_slice(BUNDLE_MAGIC);
    out.extend_from_slice(&((files.len() as u64 + 1) as u32).to_le_bytes());
    for (kind, name, path) in &files {
        let bytes =
            std::fs::read(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
        push_bundle_entry(&mut out, *kind, name, &bytes);
    }
    push_bundle_entry(&mut out, 2, "latex.fmt", &fmt);
    Ok(out)
}

/// 写入一个 bundle 条目（`kind | u32 name_len | name | u32 data_len | data`）。
fn push_bundle_entry(out: &mut Vec<u8>, kind: u8, name: &str, data: &[u8]) {
    out.push(kind);
    out.extend_from_slice(&(name.len() as u32).to_le_bytes());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
}

/// 把仓库发行资产打成 C 档资产包交给前端（**只读通道**；不碰排版、不写盘）。
///
/// 返回 `Response` 而非 `Vec<u8>`：包是 10 MB 级，走 JSON 数字数组序列化会膨胀
/// 数倍且慢，`Response` 直接以原始字节（JS 侧 `ArrayBuffer`）送达。
///
/// 失败即 `Err(String)`（前端把消息显示在错误条上）：找不到资产根、读盘失败、
/// 或文件被换掉（读到一半消失）都会明确报出来，不静默给个空包。
#[tauri::command]
fn ntex_latex_bundle() -> Result<Response, String> {
    let mut slot = BUNDLE
        .lock()
        .map_err(|_| "资产包缓存锁失效（上次 panic 污染）".to_owned())?;
    if slot.is_none() {
        let root = assets_root().ok_or_else(|| {
            "找不到发行资产目录（需含 fmt/latex.fmt 与 tex-minimal/tex；可用 NTEX_ASSETS 指定）"
                .to_owned()
        })?;
        *slot = Some(build_latex_bundle(&root)?);
    }
    let bytes = slot.clone().unwrap_or_default();
    Ok(Response::new(bytes))
}

fn main() {
    // dev 辅助（无 GUI）：`ntex-tauri --dump-bundle <path.bin>` 把资产包写盘后退出。
    // 用途是**无头验证**——前端拿到的到底是哪一版资产、包有多大、能不能被
    // `ntex-wasm` 的解析器吃下，都能在不开窗口的情况下查清（Node 驱动
    // `ui/pkg/` 复现前端渲染时尤其有用）。
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--dump-bundle") {
        let Some(path) = args.get(2) else {
            eprintln!("用法：ntex-tauri --dump-bundle <输出路径.bin>");
            std::process::exit(2);
        };
        let root = match assets_root() {
            Some(r) => r,
            None => {
                eprintln!("找不到发行资产目录（需含 fmt/latex.fmt 与 tex-minimal/tex）");
                std::process::exit(1);
            }
        };
        match build_latex_bundle(&root) {
            Ok(bytes) => match std::fs::write(path, &bytes) {
                Ok(()) => println!(
                    "已写出 {path}（{} 字节资产包，资产根 {}）",
                    bytes.len(),
                    root.display()
                ),
                Err(e) => {
                    eprintln!("写出 {path} 失败：{e}");
                    std::process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("打包失败：{e}");
                std::process::exit(1);
            }
        }
        return;
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![ntex_latex_bundle])
        .setup(|app| {
            // 窗口在配置里声明（"create": false）——on_download 只存在于
            // Builder 上，窗口建好后补挂不上，必须在构建前接好。
            let cfg = &app.config().app.windows[0];
            tauri::WebviewWindowBuilder::from_config(app.handle(), cfg)?
                .on_download(|webview, event| match event {
                    DownloadEvent::Requested { url, destination } => {
                        // 本工作台唯一的下载来源是导出按钮（blob URL）；
                        // 其余导航类下载一律拒绝，不给恶意内容落盘的口子。
                        if url.scheme() != "blob" {
                            return false;
                        }
                        // WKWebView 对 blob 下载的建议文件名不可靠（常无扩展名），
                        // 缺扩展名时按导出语义命名；目录保持 wry 预填的「下载」，
                        // 冲突加序号（wry 自己的去重发生在 handler 之前，
                        // 改名后须自查）。
                        if destination.extension().is_none() {
                            let dir = destination
                                .parent()
                                .map(std::path::Path::to_path_buf)
                                .unwrap_or_default();
                            let mut n = 1;
                            let mut path = dir.join("ntex.pdf");
                            while path.exists() {
                                n += 1;
                                path = dir.join(format!("ntex ({n}).pdf"));
                            }
                            *destination = path;
                        }
                        true
                    }
                    DownloadEvent::Finished { success, .. } => {
                        // 成败回传前端更新状态栏（eval 是发后即忘，不引入命令/IPC）。
                        let _ = webview.eval(format!(
                            "window.__downloadDone && window.__downloadDone({success})"
                        ));
                        true
                    }
                    // DownloadEvent 是 #[non_exhaustive]：未来版本新增事件时
                    // 显式拒绝，不放过任何未经审查的落盘请求。
                    _ => false,
                })
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("NTex Studio（Tauri）启动失败");
}
