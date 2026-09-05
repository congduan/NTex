//! ntex-studio：实时预览工作台（左 TeX 编辑 / 右 vello GPU 渲染）。
//!
//! 架构（plan.md M8 渲染后端的 GUI 延伸，M5+ 实时预览器的先行形态）：
//! - 引擎复用：`Typesetter::with_tfm().typeset_dvi(&str)` 同进程排版，
//!   `ntex_backend::prims::collect_page` + `build_scene` 把页面盒树编码为
//!   vello Scene（与无头回读管路同一事实源）；
//! - GPU 复用：eframe(wgpu) 共享 device/queue 上创建常驻 `vello::Renderer`，
//!   经 `egui_wgpu` 回调三段式（prepare 渲离屏纹理 / paint blit 上屏）实现
//!   表面直绘——缩放平移只改 Scene 仿射变换，矢量重光栅化，免重排版；
//! - 编译防抖：文本变更 250ms 后同步重排（demo 级文档毫秒量级），
//!   引擎契约不 panic，错误进状态栏。
//!
//! 运行：`cargo run -p ntex-studio [文件.tex]`（TFM 查找依赖运行目录，
//! 请在仓库根或其他引擎可用的目录启动）。

#![deny(unsafe_code)]

mod app;
mod editor;
mod render;

use std::path::Path;

/// 自带的默认文档（engine 可全量排版的英文示例；中文 TFM 字体 M9 前不可用）。
const DEFAULT_TEX: &str = r"\tolerance 10000
\parindent 20pt
\parskip 6pt plus 2pt
\vsize 240pt
\hsize 350pt
\font\cmr=cmr10
\cmr This is the first paragraph of a richer test document. It is long enough to wrap around into several lines, and it deliberately uses words that exercise the newly implemented features: ligatures like first, office, efficient, and flame (the f-i, f-l and f-f pairs become single glyphs); kerning pairs such as the v in several, the w in two words, and the n in into; and the spacefactor adjustments after commas, semicolons, and periods.
\par
\cmr The second paragraph shows automatic pagination: edit this text on the left and watch the page rebuild in real time on the right. When the vertical list overflows the page builder fires at the best feasible break, ships it out, and starts a fresh one.
\par
\cmr Finally, the third paragraph demonstrates the interplay of parskip, baselineskip, and lineskip between paragraphs and lines. Interline glue keeps a constant baseline distance, while parskip adds space with stretch between the paragraphs themselves, giving the document a more relaxed, readable rhythm.
\end
";

fn main() -> eframe::Result<()> {
    // CLI：可选一个 .tex 文件作为初始内容（GUI 工具，非法用法直接打印提示退出）。
    let args: Vec<String> = std::env::args().collect();
    let (source, file_note) = match args.get(1).map(|s| s.as_str()) {
        None => (DEFAULT_TEX.to_owned(), None),
        Some(path) => match std::fs::read_to_string(Path::new(path)) {
            Ok(text) => (text, Some(path.to_owned())),
            Err(err) => {
                eprintln!("读取 {path} 失败：{err}，使用内置示例文档");
                (DEFAULT_TEX.to_owned(), None)
            }
        },
    };

    let opts = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1320.0, 860.0])
            .with_title("NTex Studio"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "NTex Studio",
        opts,
        Box::new(move |cc| eframe::Result::Ok(Box::new(app::Studio::new(cc, source, file_note)))),
    )
}
