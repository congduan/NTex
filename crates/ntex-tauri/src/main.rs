//! # ntex-tauri — Tauri 实时预览壳（M8：wasm 渲染形态）
//!
//! 与 `ntex-studio`（egui/vello 桌面形态）平行的**纯壳**工作台：排版与渲染
//! 全部在前端 WASM 内完成——`ntex-wasm`（B 档）的 `compile_document()`
//! 产出页树句柄，`Document::render_page()` 走软光栅出 RGBA「纹理」（真字形
//! 轮廓口径：前端 fetch `ui/fonts/` Latin Modern OTF 经 `set_glyph_font`
//! 注入，未注入字体回落占位方框），JS `putImageData` 上 canvas；Rust 侧
//! 零命令、零 IPC（引擎不进 Tauri 进程，`make tauri` 前置步骤先用
//! wasm-bindgen 把绑定生成到 `ui/pkg/`）。
//!
//! 唯一的 Rust 侧逻辑是**下载落盘**（PDF 导出）：macOS 的 WKWebView 在没有
//! 下载处理器时对 `<a download>` 的默认策略是 **Cancel**（wry 导航委托的
//! `has_download_handler` 分支），点导出会静默无反应——故窗口改在 `setup`
//! 里经 `WebviewWindowBuilder::from_config` 手工构建并挂 `on_download`。
//! 放行后 wry 自动写「下载」目录（`Finished` 的 path 在 macOS 恒为空，
//! 是 wry 注明的 API 限制，成败经 `success` 布尔回传前端）。
//! 依赖面刻意保持最小：tauri 仅开窗口（wkwebview/WebView2），无插件。
//! 分工对照见 AGENTS.md 工作区结构表与 `crates/ntex-wasm/README.md` 三档路线。

// Windows 发布构建无控制台窗口（dev 构建保留，便于看日志）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::webview::DownloadEvent;

fn main() {
    tauri::Builder::default()
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
