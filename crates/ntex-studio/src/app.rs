//! Studio 应用状态与 UI（左编辑 / 右预览 / 底部状态栏）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use eframe::egui::{CentralPanel, Panel};
use eframe::egui_wgpu::RenderState;
use ntex_backend::prims::collect_page;
use ntex_backend::{build_scene, RenderOptions};
use ntex_layout::typeset::Typesetter;
use vello::Scene;

use crate::editor;
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

    // —— 编辑侧 ——
    source: String,
    file_note: Option<String>,
    /// 文本/参数已变更，待防抖窗口结束后重排。
    dirty: bool,
    last_edit: Instant,

    // —— 排版侧 ——
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
                    return Self::without_gpu(source, file_note, err);
                }
            }
        }
        let mut app = Self {
            render,
            source,
            file_note,
            dirty: true,
            last_edit: Instant::now() - DEBOUNCE, // 启动即先排一次
            pages: Vec::new(),
            page: 0,
            dpi: 144.0,
            debug: false,
            glyphs_on: true,
            glyph_cache: ntex_backend::glyphs::GlyphCache::new(),
            status: "就绪".to_owned(),
            err: None,
            zoom: 1.0,
            pan: None,
            auto_fit: true,
        };
        app.compile_now();
        app
    }

    fn without_gpu(source: String, file_note: Option<String>, err: String) -> Self {
        Self {
            render: None,
            source,
            file_note,
            dirty: false,
            last_edit: Instant::now(),
            pages: Vec::new(),
            page: 0,
            dpi: 144.0,
            debug: false,
            glyphs_on: true,
            glyph_cache: ntex_backend::glyphs::GlyphCache::new(),
            status: String::new(),
            err: Some(format!("GPU 不可用，预览已停用：{err}")),
            zoom: 1.0,
            pan: None,
            auto_fit: true,
        }
    }

    /// 同步重排：源码 → 页盒树 → prims → vello Scene（demo 级文档毫秒量级，
    /// 阻塞一帧可接受；更大文档后续再考虑后台线程 + 增量）。
    fn compile_now(&mut self) {
        let started = Instant::now();
        match Typesetter::with_tfm().typeset_dvi(&self.source) {
            Ok((pages, fonts)) => {
                let opts = RenderOptions {
                    dpi: self.dpi,
                    debug: self.debug,
                    glyphs: self.glyphs_on,
                    ..RenderOptions::default()
                };
                // 字形字体解析缓存借出参与页收集，结束后归还（跨重排复用，
                // 编辑防抖周期内不重复 kpsewhich/解析）。
                let mut cache = std::mem::take(&mut self.glyph_cache);
                self.pages = pages
                    .iter()
                    .map(|p| {
                        let prims = collect_page(p, &fonts, &opts, &mut cache);
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
                self.status = format!(
                    "{} 页 · 排版+编码 {} ms",
                    self.pages.len(),
                    started.elapsed().as_millis()
                );
                // 字形回落的字体提示（环境缺文件，逐字符方框口径）。
                if self.glyphs_on {
                    let missing: Vec<_> = self.glyph_cache.missing().collect();
                    if !missing.is_empty() {
                        self.status
                            .push_str(&format!("（字形回落：{} 未找到字体）", missing.join("、")));
                    }
                }
                self.err = None;
            }
            Err(err) => {
                // 编译失败保留旧页面（预览不闪空），错误进状态栏。
                self.err = Some(err.to_string());
            }
        }
        self.dirty = false;
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
                ui.label(&self.status);
                if let Some(file) = &self.file_note {
                    ui.separator();
                    ui.weak(file);
                }
                ui.separator();
                let n = self.pages.len();
                ui.label(format!("页 {}/{}", self.page + 1, n.max(1)));
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
                ui.label(format!("缩放 {:.0}%", self.zoom * 100.0));
                if ui.button("适配 (0)").clicked() {
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
                let dbg_changed = ui.checkbox(&mut self.debug, "调试 overlay").changed();
                let glyph_changed = ui.checkbox(&mut self.glyphs_on, "字形").changed();
                if dpi_changed || dbg_changed || glyph_changed {
                    self.dirty = true;
                    self.last_edit = Instant::now();
                }
                if let Some(err) = &self.err {
                    ui.separator();
                    ui.colored_label(egui::Color32::LIGHT_RED, egui::RichText::new(err).small());
                }
            });
        });

        // —— 左：源码编辑 ——
        Panel::left("editor")
            .default_size(480.0)
            .resizable(true)
            .show(ui, |ui| {
                let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                    let job = editor::tex_layout(buf.as_str(), wrap_width);
                    ui.ctx().fonts_mut(|f| f.layout_job(job))
                };
                let res = ui.add(
                    egui::TextEdit::multiline(&mut self.source)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        // 尽量高，近似填满面板（egui 以行数定高）。
                        .desired_rows(200)
                        .layouter(&mut layouter),
                );
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
                let drag = ui.input(|i| i.pointer.primary_down().then(|| i.pointer.delta()));
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
                                "GPU 预览不可用（wgpu 初始化失败）"
                            } else {
                                "（无页面）"
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
