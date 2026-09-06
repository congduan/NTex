//! # ntex-tauri — Tauri 实时预览壳（M8：wasm 渲染形态）
//!
//! 与 `ntex-studio`（egui/vello 桌面形态）平行的**纯壳**工作台：排版与渲染
//! 全部在前端 WASM 内完成——`ntex-wasm`（B 档第一刀）的 `compile_document()`
//! 产出页树句柄，`Document::render_page()` 走软光栅出 RGBA「纹理」，JS
//! `putImageData` 上 canvas；Rust 侧零命令、零 IPC（引擎不进 Tauri 进程，
//! `make tauri` 前置步骤先用 wasm-bindgen 把绑定生成到 `ui/pkg/`）。
//! 依赖面刻意保持最小：tauri 仅开窗口（wkwebview/WebView2），无插件。
//! 分工对照见 AGENTS.md 工作区结构表与 `crates/ntex-wasm/README.md` 三档路线。

// Windows 发布构建无控制台窗口（dev 构建保留，便于看日志）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("NTex Studio（Tauri）启动失败");
}
