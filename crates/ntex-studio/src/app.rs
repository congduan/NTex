//! Studio 应用状态与 UI（左编辑 / 右预览 / 底部状态栏）。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use eframe::egui::{CentralPanel, Panel};
use eframe::egui_wgpu::RenderState;
use ntex_backend::prims::collect_page;
use ntex_backend::{build_scene, RenderOptions};
use ntex_core::incremental::segmentize;
use ntex_layout::typeset::{CompileOutput, IncrementalTypesetter};
use vello::Scene;

use crate::editor;
use crate::engine::Setup;
use crate::render::{compose_display, GpuState, VelloPaint};

/// 编辑停止后延迟重排的防抖窗口。
const DEBOUNCE: Duration = Duration::from_millis(250);
/// 显示缩放上下限（相对"适配"基准的倍数）。
const ZOOM_MIN: f32 = 0.05;
const ZOOM_MAX: f32 = 16.0;
/// 预览区深色底（vello base_color 与之衔接）。
const CANVAS_BG: egui::Color32 = egui::Color32::from_gray(24);

/// 一页的 GPU 侧编码产物。
struct PageScene {
    scene: Scene,
    w: u32,
    h: u32,
}

pub struct Studio {
    /// eframe wgpu 渲染状态（含共享 device/queue；GPU 不可用时为 None，
    /// 应用退化为纯编辑器 + 提示）。
    render: Option<RenderState>,

    /// 发行资产与字体（启动时安装一次；LaTeX 是否可用由它决定）。
    setup: Setup,
    /// `\input` 相对路径可见的目录（打开文件所在目录；无文件则空）。
    input_dirs: Vec<PathBuf>,

    // —— 编辑侧 ——
    source: String,
    file_note: Option<String>,
    /// 文本/参数已变更，待防抖窗口结束后重排。
    dirty: bool,
    last_edit: Instant,

    // —— 排版侧 ——
    /// 常驻增量排版器：段级缓存跨防抖重排复用（含 `Rc` 非 Send，随 UI 线程
    /// 同步编译，不可移后台线程）。**模式（LaTeX/plain）切换时整体重建**——
    /// 两种格式的初始状态互斥（`.fmt` 导入 vs plain 预载），叠加会污染。
    inc: IncrementalTypesetter,
    /// 当前增量器所处的口径（true = LaTeX）。源码特征变化时重建增量器。
    inc_latex: bool,
    /// 最近一次成功编译对应的段列表（增量 diff 基准；失败置 None 强制全量）。
    seg_snapshot: Option<Vec<String>>,
    /// 最近一次成功排版的页面输出（源码未变、仅 dpi/调试/字形参数变更时
    /// 免引擎重排，直接重编码 Scene；与 seg_snapshot 同生共死）。
    last_output: Option<CompileOutput>,
    pages: Vec<PageScene>,
    page: usize,
    dpi: f64,
    debug: bool,
    /// 真字形渲染（Latin Modern 轮廓；关闭回落占位方框口径）。
    glyphs_on: bool,
    /// 字形字体解析缓存（跨重排复用）。
    glyph_cache: ntex_backend::glyphs::GlyphCache,
    status: String,
    err: Option<String>,
    /// 最近一次编译的转录（log 面板内容；失败时含「首现场」`!` 行）。
    log: String,
    log_open: bool,

    // —— 视图侧 ——
    zoom: f32,
    /// 用户平移（逻辑点；None = 居中）。
    pan: Option<(f32, f32)>,
    /// 拖拽/缩放后关闭，重置/翻页后恢复（保持居中适配）。
    auto_fit: bool,
}

impl Studio {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        source: String,
        file_note: Option<String>,
    ) -> Self {
        // egui 内置字体不含 CJK 字形，先注册系统字体回落（状态栏/编辑器中文、
        // ◀/▶ 按钮符号都依赖它），GPU 有无两条路径均需生效。
        install_cjk_fonts(&cc.egui_ctx);
        // 发行资产 + 字体（进程级注册表，必须在 UI 线程安装：TFM 字节源是
        // thread_local，编译跑在哪个线程就要在哪个线程注册）。
        let setup = Setup::install();
        // GPU 侧常驻资源注入 egui renderer 的回调资源仓（一次性）。
        // Mutex 包装：vello Renderer 内含 RefCell 非 Sync，而回调资源仓
        // 在 native 下要求 Send+Sync（渲染阶段单线程持锁，无争用）。
        let render = cc.wgpu_render_state.clone();
        if let Some(rs) = &render {
            match GpuState::new(&rs.device, rs.target_format) {
                Ok(gpu) => {
                    rs.renderer
                        .write()
                        .callback_resources
                        .insert(std::sync::Mutex::new(gpu));
                }
                Err(err) => {
                    eprintln!("ntex-studio：GPU 初始化失败：{err}（预览不可用）");
                    return Self::without_gpu(source, file_note, setup, err);
                }
            }
        }
        let input_dirs = input_dirs_of(file_note.as_deref());
        // 首排口径由源码特征决定（与后续每次编译的分派同一判据）。
        let inc_latex = setup.latex_ready() && ntex_layout::typeset::looks_like_latex(&source);
        let inc = setup.typesetter(inc_latex, &input_dirs);
        let mut app = Self {
            render,
            setup,
            input_dirs,
            source,
            file_note,
            dirty: true,
            last_edit: Instant::now() - DEBOUNCE, // 启动即先排一次
            inc,
            inc_latex,
            seg_snapshot: None,
            last_output: None,
            pages: Vec::new(),
            page: 0,
            dpi: 144.0,
            debug: false,
            glyphs_on: true,
            glyph_cache: ntex_backend::glyphs::GlyphCache::new(),
            status: "Ready".to_owned(),
            err: None,
            log: String::new(),
            log_open: false,
            zoom: 1.0,
            pan: None,
            auto_fit: true,
        };
        app.compile_now();
        app
    }

    fn without_gpu(source: String, file_note: Option<String>, setup: Setup, err: String) -> Self {
        let input_dirs = input_dirs_of(file_note.as_deref());
        let inc_latex = setup.latex_ready() && ntex_layout::typeset::looks_like_latex(&source);
        let inc = setup.typesetter(inc_latex, &input_dirs);
        Self {
            render: None,
            setup,
            input_dirs,
            source,
            file_note,
            dirty: false,
            last_edit: Instant::now(),
            inc,
            inc_latex,
            seg_snapshot: None,
            last_output: None,
            pages: Vec::new(),
            page: 0,
            dpi: 144.0,
            debug: false,
            glyphs_on: true,
            glyph_cache: ntex_backend::glyphs::GlyphCache::new(),
            status: String::new(),
            err: Some(format!("GPU unavailable, preview disabled: {err}")),
            log: String::new(),
            log_open: false,
            zoom: 1.0,
            pan: None,
            auto_fit: true,
        }
    }

    /// 同步重排（三路分派）：
    ///
    /// - 恰一段替换（段数不变）→ `IncrementalTypesetter::edit(k)` 增量：未变段
    ///   注入缓存节点流（行盒免重排），仅重跑页面装配；
    /// - 源码未变（dpi/调试/字形参数触发）→ 免引擎，复用上次页面输出仅重编码；
    /// - 其余（首排/段增删/多段改/上次失败）→ `compile` 全量重建缓存。
    ///
    /// 同步执行（增量排版器含 `Rc` 非 Send，随 UI 线程；demo 级毫秒量级阻塞
    /// 一帧可接受，更大文档后续再考虑后台线程）。
    fn compile_now(&mut self) {
        let started = Instant::now();
        // 模式分派（与 Tauri 前端 `main.js::applyMode` 同判据、同语义）：源码含
        // LaTeX 特征且发行资产可用 → LaTeX 口径（`latex.fmt`）；否则 plain 子集
        // （预载内嵌 plain.tex）。**口径变化必须重建增量器**——`.fmt` 导入与
        // plain 预载是互斥的两种初始状态，且旧段缓存/边界检查点属于旧格式。
        let want_latex =
            self.setup.latex_ready() && ntex_layout::typeset::looks_like_latex(&self.source);
        if want_latex != self.inc_latex {
            self.inc = self.setup.typesetter(want_latex, &self.input_dirs);
            self.inc_latex = want_latex;
            self.seg_snapshot = None;
            self.last_output = None;
        }
        let new_segs = segmentize(&self.source);
        // LaTeX 口径**只用全量重编译**（对齐 Tauri 前端：每次
        // `compile_document()` 都是新建文档句柄，没有增量接口）。这不是保守
        // 起见，而是已定位的缺陷：fmt 状态下的增量 `edit` 重放会把 `\@filef@und`
        // 的替换文本还原成含 NUL 前缀的坏值，随后 `\input` 以 `"\0article.cls"`
        // 读文件失败（现场见 ntex-layout `incremental_tests` 的 `#[ignore]`
        // 用例 `latex_fmt_import_and_incremental_edit`）。修复前 LaTeX 不启用
        // 段级增量；plain 口径（既有逐位一致测试覆盖）不受影响。
        let edited = if self.inc_latex {
            None
        } else {
            match &self.seg_snapshot {
                Some(old) => edited_segment(old, &new_segs),
                None => None,
            }
        };
        // 「源码未变」→ 免引擎复用上次页面输出（仅渲染参数变化时走这条，
        // 两种口径都保留：它不依赖段增量）。
        let attempted = if edited.is_none() && self.seg_snapshot.as_deref() == Some(&new_segs[..]) {
            Ok((
                Recompile::Reuse,
                self.last_output
                    .clone()
                    .expect("段快照与输出存档同生共死（零变化必有存档）"),
            ))
        } else {
            match edited {
                Some(k) => self.inc.edit(k, &new_segs[k]).map(|o| (Recompile::Edit, o)),
                None => self.inc.compile(&self.source).map(|o| (Recompile::Full, o)),
            }
            .map_err(|e| e.to_string())
        };
        // 转录（log）：成功/失败都取——失败时它是唯一能定位「首现场」的地方
        // （与 wasm 侧把 `!` 行拼进错误消息同口径）。
        self.log = self.inc.transcript().to_owned();
        match attempted {
            Ok((plan, out)) => {
                let stats = self.inc.stats();
                self.rebuild_scenes(&out);
                self.seg_snapshot = Some(new_segs);
                self.last_output = Some(out);
                let mode = match plan {
                    Recompile::Reuse => "params-only re-encode".to_owned(),
                    Recompile::Edit => {
                        format!(
                            "incremental {} recomputed / {} reused",
                            stats.executed, stats.reused
                        )
                    }
                    Recompile::Full => "full".to_owned(),
                };
                self.status = format!(
                    "{} · {} pages · {} · {} ms",
                    self.mode_label(),
                    self.pages.len(),
                    mode,
                    started.elapsed().as_millis()
                );
                // 字形回落的字体提示（环境缺文件，逐字符方框口径）。
                if self.glyphs_on {
                    let missing: Vec<_> = self.glyph_cache.missing().collect();
                    if !missing.is_empty() {
                        self.status.push_str(&format!(
                            " (glyph fallback: no font for {})",
                            missing.join(", ")
                        ));
                    }
                }
                self.err = None;
            }
            Err(err) => {
                // 编译失败保留旧页面（预览不闪空）；引擎内部状态已随编辑推进，
                // 丢弃增量快照，下次必全量重建（正确性优先）。
                self.seg_snapshot = None;
                self.last_output = None;
                self.err = Some(with_first_error_site(err, &self.log));
            }
        }
        self.dirty = false;
    }

    /// 当前口径标签（状态栏；对齐 Tauri 前端的 `#mode-chip`）。
    fn mode_label(&self) -> &'static str {
        if self.inc_latex {
            "LaTeX"
        } else {
            "plain"
        }
    }

    /// 页面节点 → collect_page + vello Scene（字形字体解析缓存跨重排借出/归还）。
    fn rebuild_scenes(&mut self, out: &CompileOutput) {
        let opts = RenderOptions {
            dpi: self.dpi,
            debug: self.debug,
            glyphs: self.glyphs_on,
            ..RenderOptions::default()
        };
        let mut cache = std::mem::take(&mut self.glyph_cache);
        self.pages = out
            .pages
            .iter()
            .map(|p| {
                let prims = collect_page(p, &out.fonts, &opts, &mut cache);
                let (w, h) = (prims.width, prims.height);
                PageScene {
                    scene: build_scene(&prims),
                    w,
                    h,
                }
            })
            .collect();
        self.glyph_cache = cache;
        self.page = self.page.min(self.pages.len().saturating_sub(1));
    }

    /// 当前页显示参数：(scale, offset)——页面像素 → 显示物理像素的仿射。
    /// `zoom` 是相对"适配窗口"基准的倍数；`pan = None` 表示居中。
    fn view(&self, area: egui::Rect, ppp: f32) -> (f64, (f64, f64)) {
        let (aw, ah) = (
            area.width() as f64 * ppp as f64,
            area.height() as f64 * ppp as f64,
        );
        let Some(p) = self.pages.get(self.page) else {
            return (1.0, (0.0, 0.0));
        };
        let fit = (aw / p.w.max(1) as f64).min(ah / p.h.max(1) as f64);
        let scale = fit * self.zoom as f64;
        let offset = match self.pan {
            Some((x, y)) => (x as f64 * ppp as f64, y as f64 * ppp as f64),
            None => (
                (aw - p.w as f64 * scale) / 2.0,
                (ah - p.h as f64 * scale) / 2.0,
            ),
        };
        (scale, offset)
    }
}

impl eframe::App for Studio {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // —— 防抖重排 ——
        if self.dirty && self.last_edit.elapsed() >= DEBOUNCE {
            self.compile_now();
        }

        // —— 底部状态栏（先于中央面板分配空间）——
        Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                // 口径徽标（对齐 Tauri 前端的 #mode-chip）：LaTeX 资产不可用时
                // 恒为 plain——鼠标悬停给出原因（资产/字体缺失提示）。
                let (label, color, tip) = if self.inc_latex {
                    (
                        "LaTeX",
                        egui::Color32::from_rgb(120, 180, 255),
                        "LaTeX 口径：套用发行 latex.fmt（\\documentclass/\\section 可用）",
                    )
                } else if self.setup.latex_ready() {
                    (
                        "plain",
                        egui::Color32::GRAY,
                        "plain 口径：未检测到 LaTeX 特征（\\documentclass 等触发切换）",
                    )
                } else {
                    (
                        "plain",
                        egui::Color32::from_rgb(220, 160, 90),
                        "LaTeX 资产不可用，只能用 plain 子集",
                    )
                };
                let mut tip = tip.to_owned();
                if !self.setup.fonts.is_empty() {
                    tip.push_str(&format!("\n已注入真字形字体 {} 个", self.setup.fonts.len()));
                }
                if !self.setup.note.is_empty() {
                    tip.push('\n');
                    tip.push_str(&self.setup.note);
                }
                ui.label(egui::RichText::new(label).strong().color(color))
                    .on_hover_text(tip);
                ui.separator();
                ui.label(&self.status);
                if let Some(file) = &self.file_note {
                    ui.separator();
                    ui.weak(file);
                }
                ui.separator();
                let n = self.pages.len();
                ui.label(format!("Page {}/{}", self.page + 1, n.max(1)));
                if ui
                    .add_enabled(self.page > 0, egui::Button::new("◀"))
                    .clicked()
                {
                    self.page -= 1;
                    self.pan = None;
                    self.auto_fit = true;
                }
                if ui
                    .add_enabled(self.page + 1 < n, egui::Button::new("▶"))
                    .clicked()
                {
                    self.page += 1;
                    self.pan = None;
                    self.auto_fit = true;
                }
                ui.separator();
                ui.label(format!("Zoom {:.0}%", self.zoom * 100.0));
                if ui.button("Fit (0)").clicked() {
                    self.zoom = 1.0;
                    self.pan = None;
                    self.auto_fit = true;
                }
                ui.separator();
                let dpi_changed = ui
                    .add(
                        egui::DragValue::new(&mut self.dpi)
                            .range(72.0..=600.0)
                            .suffix(" dpi"),
                    )
                    .changed();
                let dbg_changed = ui.checkbox(&mut self.debug, "Debug overlay").changed();
                let glyph_changed = ui.checkbox(&mut self.glyphs_on, "Glyphs").changed();
                if dpi_changed || dbg_changed || glyph_changed {
                    self.dirty = true;
                    self.last_edit = Instant::now();
                }
                if let Some(err) = &self.err {
                    ui.separator();
                    ui.colored_label(egui::Color32::LIGHT_RED, egui::RichText::new(err).small());
                }
                // log 开关（对齐 Tauri 前端的 log 面板）：转录是 TeX 报错定位的
                // 唯一现场（`!` 行 + 上下文），默认收起不占预览高度。
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .selectable_label(self.log_open, "Log")
                        .on_hover_text("TeX 转录（\\message/\\show/错误上下文）")
                        .clicked()
                    {
                        self.log_open = !self.log_open;
                    }
                });
            });
        });

        // —— log 面板（可折叠；显示最近一次编译的转录）——
        if self.log_open {
            Panel::bottom("log")
                .resizable(true)
                .default_size(180.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false; 2])
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            let text = if self.log.is_empty() {
                                "（转录为空）"
                            } else {
                                self.log.as_str()
                            };
                            ui.add(
                                egui::Label::new(egui::RichText::new(text).monospace().small())
                                    .wrap(),
                            );
                        });
                });
        }

        // —— 左：源码编辑 ——
        Panel::left("editor")
            .default_size(480.0)
            .resizable(true)
            .show(ui, |ui| {
                let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                    let job = editor::tex_layout(buf.as_str(), wrap_width);
                    ui.ctx().fonts_mut(|f| f.layout_job(job))
                };
                // egui 0.35 的 multiline TextEdit 不再内置滚动（desired_rows 仅为
                // 最小高度），需外层 ScrollArea 提供视口与滚动；TextEdit 自然高度
                // 随内容增长，宽度 auto_shrink(false) 填满面板。
                let res = egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.source)
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .layouter(&mut layouter),
                        )
                    })
                    .inner;
                if res.changed() {
                    self.dirty = true;
                    self.last_edit = Instant::now();
                }
            });

        // —— 右：GPU 预览 ——
        CentralPanel::default()
            .frame(egui::Frame::default().fill(CANVAS_BG))
            .show(ui, |ui| {
                let area = ui.available_rect_before_wrap();
                let ppp = ui.ctx().pixels_per_point();

                // 滚轮缩放（以指针为锚点，仅指针在预览区内时）；主键拖拽平移。
                let pointer = ui.input(|i| i.pointer.latest_pos());
                let over_preview = pointer.is_some_and(|pt| area.contains(pt));
                let scroll = if over_preview {
                    ui.input(|i| i.smooth_scroll_delta.y)
                } else {
                    0.0
                };
                if scroll != 0.0 {
                    let factor = (scroll / 240.0).exp();
                    self.zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
                    self.pan = match (self.pan, pointer) {
                        (Some((x, y)), Some(pt)) => {
                            // 保持指针下的内容点不动：pan' = m - (m - pan)·k
                            let (mx, my) = (pt.x - area.left(), pt.y - area.top());
                            Some((mx - (mx - x) * factor, my - (my - y) * factor))
                        }
                        (Some(p), None) => Some(p),
                        (None, _) => None,
                    };
                    self.auto_fit = false;
                }
                // 主键拖拽平移：需按下且起点在预览区内才接管（egui 输入是全局的，
                // 编辑器里选择文字的 primary_down 不得移动预览页面）；
                // 起点在预览内时即使拖出边界也继续跟随，避免拖拽中途丢失。
                let drag = ui.input(|i| {
                    let origin_in_preview = i
                        .pointer
                        .press_origin()
                        .is_some_and(|origin| area.contains(origin));
                    (i.pointer.primary_down() && (origin_in_preview || over_preview))
                        .then(|| i.pointer.delta())
                });
                if let Some(d) = drag.filter(|d| *d != egui::Vec2::ZERO) {
                    self.pan = Some(match self.pan {
                        Some((x, y)) => (x + d.x, y + d.y),
                        // 从居中态进入平移：把当前居中偏移物化成 pan（逻辑点）。
                        None => {
                            let (_, off) = self.view(area, ppp);
                            (
                                (off.0 / ppp as f64 + d.x as f64) as f32,
                                (off.1 / ppp as f64 + d.y as f64) as f32,
                            )
                        }
                    });
                    self.auto_fit = false;
                }
                if self.auto_fit {
                    self.pan = None;
                }

                if self.pages.get(self.page).is_none() {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new(if self.render.is_none() {
                                "GPU preview unavailable (wgpu init failed)"
                            } else {
                                "(no pages)"
                            })
                            .color(egui::Color32::GRAY),
                        );
                    });
                    return;
                }

                let page = &self.pages[self.page];
                let (scale, offset) = self.view(area, ppp);
                // 页面投影用 egui 原生绘制（视觉层），内容由 vello 回调绘制。
                let page_rect = egui::Rect::from_min_size(
                    area.min
                        + egui::vec2(
                            (offset.0 / ppp as f64) as f32,
                            (offset.1 / ppp as f64) as f32,
                        ),
                    egui::vec2(
                        (page.w as f64 * scale / ppp as f64) as f32,
                        (page.h as f64 * scale / ppp as f64) as f32,
                    ),
                );
                let painter = ui.painter_at(area);
                painter.rect_filled(
                    page_rect.expand(6.0),
                    10.0,
                    egui::Color32::from_black_alpha(120),
                );

                // 显示 Scene：白纸 + base + 仿射（每帧合成，append 数千 rect 是微秒级）。
                let display =
                    compose_display(&page.scene, scale, offset, (page.w as f64, page.h as f64));
                let vp_px = [
                    (area.width() * ppp).ceil().max(1.0) as u32,
                    (area.height() * ppp).ceil().max(1.0) as u32,
                ];
                let callback = eframe::egui_wgpu::Callback::new_paint_callback(
                    area,
                    VelloPaint {
                        scene: Arc::new(display),
                        viewport_px: vp_px,
                    },
                );
                painter.add(egui::Shape::Callback(callback));

                // 防抖窗口内保持重绘（egui 默认空闲不重绘）。
                if self.dirty {
                    ui.ctx().request_repaint_after(DEBOUNCE);
                }
            });
    }
}

/// 为 egui 注册 CJK 回落字体。egui 内置字体无 CJK 字形，编辑器中的中文
/// 内容、引擎输出的中文错误信息（及 ◀/▶ 按钮符号）缺回落时显示为方框；追加到 Proportional 与
/// Monospace 两族末位——拉丁字符仍用内置字体，仅 CJK/符号回落。按平台
/// 候选系统字体逐个尝试，全部缺失则维持默认（不视为错误）。
fn install_cjk_fonts(ctx: &egui::Context) {
    let candidates = [
        // macOS
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        // Linux
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        // Windows
        "C:\\Windows\\Fonts\\msyh.ttc",
    ];
    let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("cjk".into(), Arc::new(egui::FontData::from_owned(bytes)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("cjk".into());
    }
    ctx.set_fonts(fonts);
}

/// 打开文件所在目录（`\input` 相对路径可见；无文件 / 取不到父目录 → 空）。
fn input_dirs_of(file_note: Option<&str>) -> Vec<PathBuf> {
    file_note
        .map(PathBuf::from)
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .filter(|p| !p.as_os_str().is_empty())
        .into_iter()
        .collect()
}

/// 把转录里的 TeX「首现场」（第一个 `!` 行）拼进错误消息。
///
/// 与 wasm 侧 `compile_pipeline_assets` 同口径：宿主只看得到一句
/// `InvalidInput`，真因（`File 'xxx.sty' not found.`）只在转录里——不拼出来
/// 用户无从下手。转录里没有 `!` 行时原样返回错误。
fn with_first_error_site(err: String, transcript: &str) -> String {
    match transcript
        .lines()
        .find(|l| l.trim_start().starts_with('!'))
        .map(str::trim)
    {
        Some(line) if !err.contains(line) => format!("{err}（首现场：{line}）"),
        _ => err,
    }
}

/// 重排分派口径（状态栏展示）。
#[derive(Clone, Copy)]
enum Recompile {
    /// 源码未变（仅渲染参数），免引擎复用上次输出。
    Reuse,
    /// 恰一段替换，增量重放。
    Edit,
    /// 全量编译（首排/段结构变化/多段改/失败重建）。
    Full,
}

/// 段 diff → 增量计划：`Some(k)` = 段数不变且恰好段 k 被替换（可走增量
/// `edit(k)`：未变段注入缓存节点流，行盒免重排）；`None` = 其余（多段变化/
/// 段数增删/完全相同——完全相同时调用方另行判定走"免引擎"复用，见
/// `Studio::compile_now`）。
fn edited_segment(old: &[String], new: &[String]) -> Option<usize> {
    if old.len() != new.len() {
        return None;
    }
    let mut diff = old
        .iter()
        .zip(new.iter())
        .enumerate()
        .filter_map(|(i, (a, b))| (a != b).then_some(i));
    let first = diff.next()?;
    diff.next().is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use super::edited_segment;

    fn segs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    /// 完全相同 → None（调用方以"零变化"另行走免引擎复用）。
    #[test]
    fn identical_is_none() {
        let a = segs(&["a\n", "b\n"]);
        assert_eq!(edited_segment(&a, &a), None);
        assert_eq!(edited_segment(&[], &[]), None);
    }

    #[test]
    fn single_replacement_finds_index() {
        let old = segs(&["a\n", "b\n", "c\n"]);
        let new = segs(&["a\n", "B\n", "c\n"]);
        assert_eq!(edited_segment(&old, &new), Some(1));
    }

    /// 多段变化 / 段数增删 → None（全量）。
    #[test]
    fn multi_change_or_len_change_is_none() {
        let old = segs(&["a\n", "b\n"]);
        assert_eq!(edited_segment(&old, &segs(&["A\n", "B\n"])), None);
        assert_eq!(edited_segment(&old, &segs(&["a\n"])), None);
        assert_eq!(edited_segment(&old, &segs(&["a\n", "b\n", "c\n"])), None);
    }
}
