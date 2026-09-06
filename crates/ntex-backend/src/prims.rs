//! 绘制原语收集：盒树 → 矩形指令（软光栅与 GPU 后端共享的语义层）。
//!
//! 把 [`TinySkiaBackend`](crate::TinySkiaBackend) 与
//! [`VelloBackend`](crate::VelloBackend) 的共同遍历逻辑抽到这里：两后端
//! 收到完全相同的矩形序列，仅光栅化方式不同（整数覆盖盒 vs GPU 亚像素
//! AA），因此位图可做几何差分对照。
//!
//! 坐标语义对照 `ntex_dvi`（页面原点左上、y 向下、sp 单位）：
//!
//! - 页面盒 HBox：基线 = 页参考点；VBox：顶 = 参考点；
//! - 盒参考点由调用方传入（边距），矩形坐标为浮点像素（`sp_to_px` 换算）；
//! - 字符默认占位方框（TFM 只有度量）；`RenderOptions::glyphs` 开启时走
//!   真字形通道（Latin Modern 轮廓，见 `glyphs.rs`），位置/宽度仍按 TFM。

use std::sync::Arc;

use ntex_font::FontMetrics;
use ntex_layout::node::{BoxKind, BoxNode, FontId, GlueOrder, Node, GLUE_ORDER_FIL};

use crate::glyphs::{ot1_to_unicode, GlyphCache, GlyphFont};

use crate::raster::sp_to_px;

/// 占位字符的灰度（浅色示意，非最终墨色；字形渲染属 M9）。
const CHAR_GRAY: u8 = 160;
/// 占位字符方框的底色（浅灰，深灰描边用同色系区分）。
const CHAR_BG_GRAY: u8 = 225;

// —— 调试 overlay 颜色（浅色/细条策略，两后端统一不透明渲染）——
// 约束：每通道 ≤ 240，保证 `Pixmap::pixel_nonwhite`（每通道 < 250 判有墨）
// 对全部 overlay 颜色成立（测试可复用该判定）。
/// 版心（边距）矩形描边：深灰。
const DBG_MARGIN: (u8, u8, u8) = (128, 128, 128);
/// HBox 边界描边：蓝。
const DBG_HBOX: (u8, u8, u8) = (60, 120, 230);
/// VBox 边界描边：紫红。
const DBG_VBOX: (u8, u8, u8) = (200, 60, 200);
/// HBox 基线：青（1px 横线，长 = 盒宽）。
const DBG_BASELINE: (u8, u8, u8) = (0, 180, 180);
/// glue 自然宽色带：浅黄（基线上方 3px / 盒左内侧 3px）。
const DBG_GLUE: (u8, u8, u8) = (240, 230, 120);
/// glue 无穷阶（fil+）：亮绿（替换浅黄带色）。
const DBG_GLUE_FIL: (u8, u8, u8) = (0, 220, 90);
/// glue stretch 指示线：绿（带外缘，长度 = stretch）。
const DBG_GLUE_STRETCH: (u8, u8, u8) = (40, 200, 80);
/// glue shrink 指示线：红（带内缘，长度 = shrink）。
const DBG_GLUE_SHRINK: (u8, u8, u8) = (230, 60, 60);
/// kern 标记：橙（4px 短线）。
const DBG_KERN: (u8, u8, u8) = (240, 130, 0);
/// 可选断点（penalty < 10000）：品红细线。
const DBG_PENALTY: (u8, u8, u8) = (230, 0, 230);
/// 禁断点（penalty ≥ 10000）：深红粗线。
const DBG_PENALTY_INF: (u8, u8, u8) = (150, 0, 0);

/// 渲染配置（两后端共用）。
#[derive(Debug, Clone, PartialEq)]
pub struct RenderOptions {
    /// 渲染 DPI（默认 144，A4 ≈ 1191×1684px）。
    pub dpi: f64,
    /// 页面尺寸（pt）：宽、高（默认 A4，同 ntex-pdf::PdfOptions）。
    pub page_size_pt: (f64, f64),
    /// 页边距（pt）：四边同值（默认 72pt = 1in）。
    pub margin_pt: f64,
    /// 调试 overlay：开启时盒边界/glue/断点标记收集到 [`PagePrims::debug`]
    /// 独立通道，`rects` 不受影响（两后端差分口径不变）。
    pub debug: bool,
    /// 真字形渲染（vello 后端）：字符走 glyphs 通道（Latin Modern 轮廓，
    /// 位置/宽度仍按 TFM），不再产占位方框；字体文件不可用时逐字符回落
    /// 方框。软光栅后端不支持字形通道，强制按关闭处理。默认 false
    /// （保持既有差分/快照口径零变化）。
    pub glyphs: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            dpi: 144.0,
            page_size_pt: (595.276, 841.890),
            margin_pt: 72.0,
            debug: false,
            glyphs: false,
        }
    }
}

/// 校验配置（输入可达路径不 panic；非法配置一律报错）。
pub fn validate_options(opts: &RenderOptions) -> Result<(), String> {
    if !opts.dpi.is_finite() || opts.dpi <= 0.0 {
        return Err(format!("非法 DPI：{}", opts.dpi));
    }
    let (w_pt, h_pt) = opts.page_size_pt;
    if !w_pt.is_finite() || !h_pt.is_finite() || w_pt <= 0.0 || h_pt <= 0.0 {
        return Err(format!("非法页面尺寸：{w_pt}×{h_pt}pt"));
    }
    if !opts.margin_pt.is_finite() || opts.margin_pt < 0.0 {
        return Err(format!("非法页边距：{}", opts.margin_pt));
    }
    Ok(())
}

/// 矩形绘制指令（浮点像素坐标，不透明 RGB；`(x, y)` = 左上角）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectPrim {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub color: (u8, u8, u8),
}

impl RectPrim {
    /// 追加一条矩形指令（非正尺寸静默丢弃，与软光栅 `fill_rect` 语义一致）。
    fn push(out: &mut Vec<Self>, x: f64, y: f64, w: f64, h: f64, color: (u8, u8, u8)) {
        if w > 0.0 && h > 0.0 {
            out.push(Self { x, y, w, h, color });
        }
    }
}

/// 一页的绘制指令集（`width/height` 为页面像素尺寸，下限 1×1）。
///
/// `debug` 是独立 overlay 通道（仅 [`RenderOptions::debug`] 开启时填充），
/// 后端在 `rects` 之后绘制；`rects` 始终与 overlay 无关的纯内容指令。
/// `glyphs` 为真字形通道（仅 [`RenderOptions::glyphs`] 开启时填充，vello
/// 后端绘制；`glyph_fonts` 为页内去重的字体表，`GlyphPrim::font` 下标引用）。
pub struct PagePrims {
    pub width: u32,
    pub height: u32,
    pub rects: Vec<RectPrim>,
    pub glyphs: Vec<GlyphPrim>,
    pub glyph_fonts: Vec<Arc<GlyphFont>>,
    pub debug: Vec<RectPrim>,
}

/// 真字形绘制指令：`(x, y)` = 基线原点（y 向下，同 rect 坐标系），
/// `size` = em 的像素数（TFM 实际字号按 dpi 换算），`gid`/`font` 定位轮廓。
#[derive(Debug, Clone, Copy)]
pub struct GlyphPrim {
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub gid: u32,
    pub font: u16,
}

/// 字形通道收集上下文（贯穿 collect 递归；`on = false` 时全部为空操作，
/// 字符走占位方框口径）。
pub(crate) struct GlyphCtx<'a> {
    on: bool,
    dpi: f64,
    /// 排版输出的字体表（`FontId` 下标引用；提供名字与实际字号）。
    metrics: &'a [FontMetrics],
    cache: &'a mut GlyphCache,
    out: &'a mut Vec<GlyphPrim>,
    fonts_out: &'a mut Vec<Arc<GlyphFont>>,
}

impl GlyphCtx<'_> {
    /// 尝试把字符收成字形指令；成功返回 true（调用方跳过占位方框），
    /// 字体/字形/编码任一环节缺失返回 false（回落方框，不报错）。
    fn push_char(&mut self, font: FontId, charcode: u32, x: f64, y: f64) -> bool {
        if !self.on {
            return false;
        }
        let Some(slot) = u8::try_from(charcode).ok().and_then(ot1_to_unicode) else {
            return false; // OT1 之外/之上的编码位（如 T1 高位区）暂无映射
        };
        let Some(fm) = self.metrics.get(font.0 as usize) else {
            return false;
        };
        let gf = match self.cache.resolve(&fm.name) {
            Some(gf) => gf,
            None => return false, // 环境无字体文件（kpsewhich 未命中）
        };
        let Some(gid) = gf.glyph_id(slot) else {
            return false; // 该字体无此字形（如 lmroman 无希腊区）
        };
        // em 像素 = 实际字号（design × scale / 2^20）按 dpi 换算。
        let em_sp = fm.design_size_sp.saturating_mul(fm.scale) >> 20;
        let idx = match self.fonts_out.iter().position(|f| Arc::ptr_eq(f, &gf)) {
            Some(i) => i,
            None => {
                self.fonts_out.push(gf);
                self.fonts_out.len() - 1
            }
        };
        let idx = u16::try_from(idx).unwrap_or(u16::MAX); // 页内字体数上限防御
        self.out.push(GlyphPrim {
            x,
            y,
            size: sp_to_px(em_sp, self.dpi),
            gid,
            font: idx,
        });
        true
    }
}

/// 调试通道输出（`None` = 关闭，跳过全部标记逻辑）。
type DebugOut<'a> = Option<&'a mut Vec<RectPrim>>;

/// 调试通道追加一条矩形（关闭时为空操作）。
fn dbg_push(dbg: &mut DebugOut, x: f64, y: f64, w: f64, h: f64, color: (u8, u8, u8)) {
    if let Some(v) = dbg.as_deref_mut() {
        RectPrim::push(v, x, y, w, h, color);
    }
}

/// 空心描边（4 条 1px 矩形；宽高下限 1px）。
fn dbg_stroke(dbg: &mut DebugOut, x: f64, y: f64, w: f64, h: f64, color: (u8, u8, u8)) {
    let (w, h) = (w.max(1.0), h.max(1.0));
    dbg_push(dbg, x, y, w, 1.0, color);
    dbg_push(dbg, x, y + h - 1.0, w, 1.0, color);
    dbg_push(dbg, x, y, 1.0, h, color);
    dbg_push(dbg, x + w - 1.0, y, 1.0, h, color);
}

/// 水平 glue 标记：自然宽带（基线上方 3px）+ stretch/shrink 指示线。
fn dbg_glue_h(
    dbg: &mut DebugOut,
    x: f64,
    ry: f64,
    w: f64,
    stretch: f64,
    shrink: f64,
    order: GlueOrder,
) {
    let band = if order >= GLUE_ORDER_FIL {
        DBG_GLUE_FIL
    } else {
        DBG_GLUE
    };
    dbg_push(dbg, x, ry - 4.0, w, 3.0, band);
    dbg_push(dbg, x, ry - 5.0, stretch, 1.0, DBG_GLUE_STRETCH);
    dbg_push(dbg, x, ry - 1.0, shrink, 1.0, DBG_GLUE_SHRINK);
}

/// 引导区域宽度（tex.web `leader_wt`）：无延伸时取自然宽（此时退化为画一次），
// 有 stretch 取 `stretch`（fill/filll 视作按传入的拉伸后宽度），负值钳 0。
fn leaders_region_width(width: i64, stretch: i64) -> i64 {
    if stretch > 0 {
        stretch
    } else {
        width.max(0)
    }
}

/// 引导内容自然宽（tex.web `leader_lr`）：box 取宽，rule 取宽。
/// 引导内容自然高（垂直 leaders 用）：box 取 height+depth，rule 同理。
fn leader_unit_height(inner: &Node) -> i64 {
    match inner {
        Node::Box(b) => b.height + b.depth,
        Node::Rule { height, depth, .. } => height + depth,
        _ => 0,
    }
}

/// 引导符收集（tex.web `hlist_out` §623-629 leader 分支 / TeXbook Ch.21）。
///
/// 布局层经 `hpack` 把 glue_set 烘焙进胶水宽度，故引导区域宽 = 本节点 glue 的
/// 拉伸后宽度（无 stretch 时为自然宽）。三种对齐：
///
/// - `Leaders`：重复单元按周期在区域内**取整对齐网格**，首单元起点 ≥ 区域起点，
///   两端允许不满格留白；
/// - `Cleaders`：取整份居中，两侧留白相等；
/// - `Xleaders`：单元与间隙交替均分剩余空间（首尾必贴区域两端，间隙 ≥ 0）。
///
/// 单元 ≤ 0 或区域装不下一个完整单元时不绘制（tex.web `finite_shrink` 守卫：
/// 防止死循环）。绘制把 inner 物化为既有 Rect/Glyph 指令：box 递归
/// `collect_box`，rule 直接 `RectPrim::push`。
// 装配参数即 TeX 排版语义的自然元组（尺寸/伸缩/坐标系/输出上下文），打包成
// 结构体反而割裂调用点；此处豁免参数数检查。
#[allow(clippy::too_many_arguments)]
fn collect_leaders(
    kind: ntex_layout::node::LeadersKind,
    inner: &Node,
    width: i64,
    stretch: i64,
    cur: i64,
    rx: f64,
    ry: f64,
    dpi: f64,
    out: &mut Vec<RectPrim>,
    g: &mut GlyphCtx,
    dbg: &mut DebugOut,
) {
    let region = leaders_region_width(width, stretch);
    if region <= 0 {
        return;
    }
    let x0 = rx + sp_to_px(cur, dpi);
    let region_px = sp_to_px(region, dpi);
    match inner {
        Node::Box(b) => {
            let unit = b.width;
            if unit <= 0 || sp_to_px(unit, dpi) <= 0.0 {
                return;
            }
            let unit_px = sp_to_px(unit, dpi);
            let count = (region_px / unit_px).floor() as i64;
            if count <= 0 {
                return;
            }
            let gap_px = region_px - count as f64 * unit_px;
            let start_px = match kind {
                ntex_layout::node::LeadersKind::Leaders => x0,
                ntex_layout::node::LeadersKind::Cleaders => x0 + gap_px / 2.0,
                ntex_layout::node::LeadersKind::Xleaders => {
                    let step = if count > 1 {
                        gap_px / (count + 1) as f64
                    } else {
                        0.0
                    };
                    let mut cx = x0;
                    for _ in 0..count {
                        draw_leader_unit(inner, cx, ry, dpi, out, g, dbg);
                        cx += unit_px + step;
                    }
                    return;
                }
            };
            for i in 0..count {
                let ux = start_px + i as f64 * unit_px;
                draw_leader_unit(inner, ux, ry, dpi, out, g, dbg);
            }
        }
        Node::Rule {
            width: rw,
            height,
            depth,
        } => {
            // 布局简化：未定宽度（`\leaders\hrule` 后接非 1fil glue）经 rule
            // 扫描路径落成 i32::MIN 哨兵——渲染层视作非法，跳过不画。
            if *rw <= 0 || *rw <= i32::MIN as i64 {
                return;
            }
            let unit_px = sp_to_px(*rw, dpi);
            let count = (region_px / unit_px).floor() as i64;
            if count <= 0 {
                return;
            }
            let gap_px = region_px - count as f64 * unit_px;
            let start_px = match kind {
                ntex_layout::node::LeadersKind::Leaders => x0,
                ntex_layout::node::LeadersKind::Cleaders => x0 + gap_px / 2.0,
                ntex_layout::node::LeadersKind::Xleaders => {
                    let step = if count > 1 {
                        gap_px / (count + 1) as f64
                    } else {
                        0.0
                    };
                    let top = ry - sp_to_px(*height, dpi);
                    let h_px = sp_to_px(height + depth, dpi);
                    let mut cx = x0;
                    for _ in 0..count {
                        RectPrim::push(out, cx, top, unit_px, h_px, (0, 0, 0));
                        cx += unit_px + step;
                    }
                    return;
                }
            };
            let top = ry - sp_to_px(*height, dpi);
            let h_px = sp_to_px(height + depth, dpi);
            for i in 0..count {
                RectPrim::push(
                    out,
                    start_px + i as f64 * unit_px,
                    top,
                    unit_px,
                    h_px,
                    (0, 0, 0),
                );
            }
        }
        _ => {}
    }
}

/// 画一个引导单元：box 递归走 [`collect_box`]（复用字符占位/字形通道），
// 其它类型无独立绘制形态（rule 在调用方直接物化）。
fn draw_leader_unit(
    inner: &Node,
    x: f64,
    ry: f64,
    dpi: f64,
    out: &mut Vec<RectPrim>,
    g: &mut GlyphCtx,
    dbg: &mut DebugOut,
) {
    if let Node::Box(b) = inner {
        collect_box(b, x, ry + sp_to_px(b.shift, dpi), dpi, out, g, dbg);
    }
}

/// 垂直引导单元：box 以「参考点 + height」递归（对照 vlist 的盒子推进）。
fn draw_leader_unit_v(
    inner: &Node,
    rx: f64,
    top: f64,
    dpi: f64,
    out: &mut Vec<RectPrim>,
    g: &mut GlyphCtx,
    dbg: &mut DebugOut,
) {
    if let Node::Box(b) = inner {
        collect_box(
            b,
            rx + sp_to_px(b.shift, dpi),
            top + sp_to_px(b.height, dpi),
            dpi,
            out,
            g,
            dbg,
        );
    }
}

/// 垂直 glue 标记：自然宽竖带（盒左内侧 3px）+ stretch 指示线。
fn dbg_glue_v(dbg: &mut DebugOut, rx: f64, y: f64, h: f64, stretch: f64, order: GlueOrder) {
    let band = if order >= GLUE_ORDER_FIL {
        DBG_GLUE_FIL
    } else {
        DBG_GLUE
    };
    dbg_push(dbg, rx, y, 3.0, h, band);
    dbg_push(dbg, rx - 1.0, y, 1.0, stretch, DBG_GLUE_STRETCH);
}

/// pt → sp（配置里的 pt 值转内部 sp 坐标）。
fn pt_to_sp(pt: f64) -> i64 {
    (pt * 65_536.0) as i64
}

/// 收集一页的矩形指令（页面白底清屏由各后端自行负责）。
///
/// `fonts` = 排版输出的字体表（字形通道用；`cache` 跨页/跨渲染复用解析结果）。
pub fn collect_page(
    page: &BoxNode,
    fonts: &[FontMetrics],
    opts: &RenderOptions,
    cache: &mut GlyphCache,
) -> PagePrims {
    let (w_pt, h_pt) = opts.page_size_pt;
    let mut rects = Vec::new();
    let mut glyphs = Vec::new();
    let mut glyph_fonts = Vec::new();
    let mut debug = Vec::new();
    let rx = sp_to_px(pt_to_sp(opts.margin_pt), opts.dpi);
    // 页面盒参考点：HBox 基线 = 边距 + 页高；VBox 顶 = 边距。
    let ry = match page.kind {
        BoxKind::HBox => rx + sp_to_px(page.height, opts.dpi),
        BoxKind::VBox => rx,
    };
    let mut dbg: DebugOut = if opts.debug { Some(&mut debug) } else { None };
    // 版心（边距矩形）描边。
    let (page_w_px, page_h_px) = (
        sp_to_px(pt_to_sp(w_pt), opts.dpi),
        sp_to_px(pt_to_sp(h_pt), opts.dpi),
    );
    dbg_stroke(
        &mut dbg,
        rx,
        rx,
        page_w_px - 2.0 * rx,
        page_h_px - 2.0 * rx,
        DBG_MARGIN,
    );
    let mut g = GlyphCtx {
        on: opts.glyphs,
        dpi: opts.dpi,
        metrics: fonts,
        cache,
        out: &mut glyphs,
        fonts_out: &mut glyph_fonts,
    };
    collect_box(page, rx, ry, opts.dpi, &mut rects, &mut g, &mut dbg);
    PagePrims {
        width: page_w_px.round() as u32,
        height: page_h_px.round() as u32,
        rects,
        glyphs,
        glyph_fonts,
        debug,
    }
}

/// 递归收集一个盒子；`(rx, ry)` = 盒参考点（px）：HBox 基线，VBox 顶。
fn collect_box(
    bx: &BoxNode,
    rx: f64,
    ry: f64,
    dpi: f64,
    out: &mut Vec<RectPrim>,
    g: &mut GlyphCtx,
    dbg: &mut DebugOut,
) {
    // 盒边界描边（HBox 蓝 / VBox 紫红）+ HBox 基线（青）。
    let (w_px, h_px, d_px) = (
        sp_to_px(bx.width, dpi),
        sp_to_px(bx.height, dpi),
        sp_to_px(bx.depth, dpi),
    );
    match bx.kind {
        BoxKind::HBox => {
            dbg_stroke(dbg, rx, ry - h_px, w_px, h_px + d_px, DBG_HBOX);
            dbg_push(dbg, rx, ry, w_px, 1.0, DBG_BASELINE);
            collect_hlist(bx, rx, ry, dpi, out, g, dbg);
        }
        BoxKind::VBox => {
            dbg_stroke(dbg, rx, ry, w_px, h_px + d_px, DBG_VBOX);
            collect_vlist(bx, rx, ry, dpi, out, g, dbg);
        }
    }
}

/// 水平列表（对照 ntex-dvi `hlist`）：字符/规则坐基线，盒 shift 下移。
fn collect_hlist(
    bx: &BoxNode,
    rx: f64,
    ry: f64,
    dpi: f64,
    out: &mut Vec<RectPrim>,
    g: &mut GlyphCtx,
    dbg: &mut DebugOut,
) {
    let mut cur_h = 0i64;
    for node in &bx.children {
        match node {
            Node::Char {
                font,
                charcode,
                width,
                height,
                depth,
            }
            | Node::Ligature {
                font,
                charcode,
                width,
                height,
                depth,
                ..
            } => {
                let x = rx + sp_to_px(cur_h, dpi);
                // 真字形通道优先（OT1→Unicode→LM 轮廓）；任一环节缺失
                // 回落占位方框口径（与既有渲染一致）。
                if g.push_char(*font, *charcode, x, ry) {
                    cur_h += width;
                    continue;
                }
                let w = sp_to_px(*width, dpi);
                let top = ry - sp_to_px(*height, dpi);
                let bot = ry + sp_to_px(*depth, dpi);
                // 占位示意：浅灰外框（height+depth 全高）+ 深灰上标区 + 基线段。
                RectPrim::push(
                    out,
                    x,
                    top,
                    w,
                    bot - top,
                    (CHAR_BG_GRAY, CHAR_BG_GRAY, CHAR_BG_GRAY),
                );
                RectPrim::push(
                    out,
                    x,
                    top,
                    w,
                    sp_to_px(*height, dpi),
                    (CHAR_GRAY, CHAR_GRAY, CHAR_GRAY),
                );
                RectPrim::push(out, x, ry, w, 1.0, (CHAR_GRAY, CHAR_GRAY, CHAR_GRAY));
                cur_h += width;
            }
            Node::Rule {
                width,
                height,
                depth,
            } => {
                let x = rx + sp_to_px(cur_h, dpi);
                let top = ry - sp_to_px(*height, dpi);
                RectPrim::push(
                    out,
                    x,
                    top,
                    sp_to_px(*width, dpi),
                    sp_to_px(height + depth, dpi),
                    (0, 0, 0),
                );
                cur_h += width;
            }
            Node::Glue {
                width,
                stretch,
                shrink,
                stretch_order,
                ..
            } => {
                dbg_glue_h(
                    dbg,
                    rx + sp_to_px(cur_h, dpi),
                    ry,
                    sp_to_px(*width, dpi),
                    sp_to_px(*stretch, dpi),
                    sp_to_px(*shrink, dpi),
                    *stretch_order,
                );
                cur_h += width;
            }
            Node::Leaders {
                kind,
                inner,
                width,
                stretch,
                ..
            } => {
                collect_leaders(
                    *kind, inner, *width, *stretch, cur_h, rx, ry, dpi, out, g, dbg,
                );
                cur_h += leaders_region_width(*width, *stretch);
            }
            Node::Kern { width } => {
                dbg_push(dbg, rx + sp_to_px(cur_h, dpi), ry - 2.0, 1.0, 4.0, DBG_KERN);
                cur_h += width;
            }
            Node::Penalty { penalty } => {
                // 断点竖线贯穿盒高深区；禁断（≥10000）深红加粗。
                let x = rx + sp_to_px(cur_h, dpi);
                let top = ry - sp_to_px(bx.height, dpi);
                let h = sp_to_px(bx.height + bx.depth, dpi).max(4.0);
                let color = if *penalty >= 10_000 {
                    DBG_PENALTY_INF
                } else {
                    DBG_PENALTY
                };
                dbg_push(
                    dbg,
                    x,
                    top,
                    if *penalty >= 10_000 { 2.0 } else { 1.0 },
                    h,
                    color,
                );
            }
            Node::Box(inner) => {
                let x = rx + sp_to_px(cur_h, dpi);
                let child_ry = ry + sp_to_px(inner.shift, dpi);
                collect_box(inner, x, child_ry, dpi, out, g, dbg);
                cur_h += inner.width;
            }
            _ => {}
        }
    }
}

/// 垂直列表（对照 ntex-dvi `vlist`）：顶 = 参考点，逐子节点向下推进。
fn collect_vlist(
    bx: &BoxNode,
    rx: f64,
    ry: f64,
    dpi: f64,
    out: &mut Vec<RectPrim>,
    g: &mut GlyphCtx,
    dbg: &mut DebugOut,
) {
    let mut cur_v = 0i64;
    // ntex-dvi vlist：参考点即页顶（cur_v -= height 回 0 的等价形式）；
    // 本函数直接以 ry = 顶、cur_v 从 0 推进。
    for node in &bx.children {
        match node {
            Node::Box(inner) => {
                // 先推进 height，再以「参考点 + height」为子盒基线（HBox）
                // 或子盒顶（VBox）。统一走 collect_box：子盒也描边/画基线。
                cur_v += inner.height;
                let child_ref_y = ry + sp_to_px(cur_v, dpi);
                let child_rx = rx + sp_to_px(inner.shift, dpi);
                collect_box(inner, child_rx, child_ref_y, dpi, out, g, dbg);
                cur_v += inner.depth;
            }
            Node::Glue {
                width,
                stretch,
                stretch_order,
                ..
            } => {
                dbg_glue_v(
                    dbg,
                    rx,
                    ry + sp_to_px(cur_v, dpi),
                    sp_to_px(*width, dpi),
                    sp_to_px(*stretch, dpi),
                    *stretch_order,
                );
                cur_v += width;
            }
            Node::Leaders {
                kind,
                inner,
                width,
                stretch,
                ..
            } => {
                // 垂直列表里的 leaders：区域高度取拉伸后 glue（对齐 vbox 打包
                // 烘焙语义）；单元高度 = inner height+depth，纵向逐格铺放。
                let region = leaders_region_width(*width, *stretch);
                if region > 0 {
                    let unit = leader_unit_height(inner);
                    if unit > 0 {
                        let y0 = ry + sp_to_px(cur_v, dpi);
                        let region_px = sp_to_px(region, dpi);
                        let unit_px = sp_to_px(unit, dpi);
                        let count = (region_px / unit_px).floor() as i64;
                        if count > 0 {
                            let gap_px = region_px - count as f64 * unit_px;
                            let start_y = match kind {
                                ntex_layout::node::LeadersKind::Leaders => y0,
                                ntex_layout::node::LeadersKind::Cleaders => y0 + gap_px / 2.0,
                                ntex_layout::node::LeadersKind::Xleaders => {
                                    let step = if count > 1 {
                                        gap_px / (count + 1) as f64
                                    } else {
                                        0.0
                                    };
                                    let mut cy = y0;
                                    for _ in 0..count {
                                        draw_leader_unit_v(inner, rx, cy, dpi, out, g, dbg);
                                        cy += unit_px + step;
                                    }
                                    cur_v += region;
                                    continue;
                                }
                            };
                            for i in 0..count {
                                draw_leader_unit_v(
                                    inner,
                                    rx,
                                    start_y + i as f64 * unit_px,
                                    dpi,
                                    out,
                                    g,
                                    dbg,
                                );
                            }
                        }
                    }
                }
                cur_v += region;
            }
            Node::Kern { width } => {
                dbg_push(dbg, rx, ry + sp_to_px(cur_v, dpi), 4.0, 1.0, DBG_KERN);
                cur_v += width;
            }
            Node::Penalty { penalty } => {
                // 断页点横线；禁断（≥10000）深红加粗。
                let y = ry + sp_to_px(cur_v, dpi);
                let w = sp_to_px(bx.width, dpi).max(4.0);
                let (h, color) = if *penalty >= 10_000 {
                    (2.0, DBG_PENALTY_INF)
                } else {
                    (1.0, DBG_PENALTY)
                };
                dbg_push(dbg, rx, y, w, h, color);
            }
            Node::Rule {
                width,
                height,
                depth,
            } => {
                let top = ry + sp_to_px(cur_v, dpi);
                RectPrim::push(
                    out,
                    rx,
                    top,
                    sp_to_px(*width, dpi),
                    sp_to_px(height + depth, dpi),
                    (0, 0, 0),
                );
                cur_v += height + depth;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaders_grid_alignment_counts_and_phase() {
        let inner = Node::Box(BoxNode {
            kind: BoxKind::HBox,
            width: 10 * 65_536,
            height: 2 * 65_536,
            depth: 0,
            shift: 0,
            children: vec![Node::Rule {
                width: 2 * 65_536,
                height: 2 * 65_536,
                depth: 0,
            }],
        });
        let ld = Node::Leaders {
            kind: ntex_layout::node::LeadersKind::Leaders,
            inner: Box::new(inner),
            width: 0,
            stretch: 100 * 65_536,
            shrink: 0,
        };
        let page = BoxNode {
            kind: BoxKind::HBox,
            width: 100 * 65_536,
            height: 2 * 65_536,
            depth: 0,
            shift: 0,
            children: vec![ld],
        };
        let opts = RenderOptions {
            dpi: 72.0,
            ..Default::default()
        };
        let prims = collect_page(&page, &[], &opts, &mut GlyphCache::new());
        // 100pt 区域装 10pt 单元 → 10 份；内盒 rule 居中（单元左端 +4pt）。
        assert_eq!(prims.rects.len(), 10);
        let xs: Vec<f64> = prims.rects.iter().map(|r| r.x).collect();
        // 首单元对齐网格起点 x0=72（72dpi 1pt=1px）；单元周期 10px。
        assert!((xs[0] - 72.0).abs() < 1e-9);
        for (i, pair) in xs.windows(2).enumerate() {
            assert!(
                (pair[1] - pair[0] - 10.0).abs() < 1e-9,
                "pair {i}: {pair:?}"
            );
        }
        // rule 尺寸一致（2×2pt）。
        assert!(prims
            .rects
            .iter()
            .all(|r| (r.w - 2.0).abs() < 1e-9 && (r.h - 2.0).abs() < 1e-9));
    }

    #[test]
    fn leaders_grid_shifts_to_glue_start() {
        // 区域 25pt，单元 10pt → 2 份贴齐网格（Leaders 原点对齐），尾差 5pt 留白。
        let ld = Node::Leaders {
            kind: ntex_layout::node::LeadersKind::Leaders,
            inner: Box::new(Node::Box(BoxNode {
                kind: BoxKind::HBox,
                width: 10 * 65_536,
                height: 65_536,
                depth: 0,
                shift: 0,
                children: vec![Node::Rule {
                    width: 65_536,
                    height: 65_536,
                    depth: 0,
                }],
            })),
            width: 0,
            stretch: 25 * 65_536,
            shrink: 0,
        };
        let page = BoxNode {
            kind: BoxKind::HBox,
            width: 25 * 65_536,
            height: 65_536,
            depth: 0,
            shift: 0,
            children: vec![ld],
        };
        let opts = RenderOptions {
            dpi: 72.0,
            ..Default::default()
        };
        let prims = collect_page(&page, &[], &opts, &mut GlyphCache::new());
        assert_eq!(prims.rects.len(), 2);
        let xs: Vec<f64> = prims.rects.iter().map(|r| r.x).collect();
        assert!((xs[0] - 72.0).abs() < 1e-9);
        assert!((xs[1] - 82.0).abs() < 1e-9); // 72 + 10pt
    }

    #[test]
    fn cleaders_centers_with_equal_gaps() {
        // 区域 25pt，单元 10pt → 2 份居中：两侧各 2.5pt 留白。
        let ld = Node::Leaders {
            kind: ntex_layout::node::LeadersKind::Cleaders,
            inner: Box::new(Node::Box(BoxNode {
                kind: BoxKind::HBox,
                width: 10 * 65_536,
                height: 65_536,
                depth: 0,
                shift: 0,
                children: vec![Node::Rule {
                    width: 65_536,
                    height: 65_536,
                    depth: 0,
                }],
            })),
            width: 0,
            stretch: 25 * 65_536,
            shrink: 0,
        };
        let page = BoxNode {
            kind: BoxKind::HBox,
            width: 25 * 65_536,
            height: 65_536,
            depth: 0,
            shift: 0,
            children: vec![ld],
        };
        let opts = RenderOptions {
            dpi: 72.0,
            ..Default::default()
        };
        let prims = collect_page(&page, &[], &opts, &mut GlyphCache::new());
        assert_eq!(prims.rects.len(), 2);
        let xs: Vec<f64> = prims.rects.iter().map(|r| r.x).collect();
        assert!((xs[0] - 74.5).abs() < 1e-9); // 72 + 2.5pt
        assert!((xs[1] - 84.5).abs() < 1e-9); // +10pt
    }

    #[test]
    fn xleaders_equalizes_gaps() {
        // 区域 25pt，单元 10pt → 2 份 + 3 个等间隙（25−20)/3 ≈ 1.667pt。
        let ld = Node::Leaders {
            kind: ntex_layout::node::LeadersKind::Xleaders,
            inner: Box::new(Node::Box(BoxNode {
                kind: BoxKind::HBox,
                width: 10 * 65_536,
                height: 65_536,
                depth: 0,
                shift: 0,
                children: vec![Node::Rule {
                    width: 65_536,
                    height: 65_536,
                    depth: 0,
                }],
            })),
            width: 0,
            stretch: 25 * 65_536,
            shrink: 0,
        };
        let page = BoxNode {
            kind: BoxKind::HBox,
            width: 25 * 65_536,
            height: 65_536,
            depth: 0,
            shift: 0,
            children: vec![ld],
        };
        let opts = RenderOptions {
            dpi: 72.0,
            ..Default::default()
        };
        let prims = collect_page(&page, &[], &opts, &mut GlyphCache::new());
        assert_eq!(prims.rects.len(), 2);
        let xs: Vec<f64> = prims.rects.iter().map(|r| r.x).collect();
        assert!((xs[0] - 72.0).abs() < 1e-9);
        let step = 10.0 + 5.0 / 3.0; // 单元 + 间隙（(25−20)/3 ≈ 1.667px）
        assert!((xs[1] - (72.0 + step)).abs() < 1e-9);
    }

    #[test]
    fn leaders_rule_units_materialize_rects() {
        // inner = Rule：\leaders\hrule 一串矩形（3 份 4pt 宽、间隔 1pt）。
        let ld = Node::Leaders {
            kind: ntex_layout::node::LeadersKind::Leaders,
            inner: Box::new(Node::Rule {
                width: 4 * 65_536,
                height: 65_536,
                depth: 0,
            }),
            width: 0,
            stretch: 15 * 65_536,
            shrink: 0,
        };
        let page = BoxNode {
            kind: BoxKind::HBox,
            width: 15 * 65_536,
            height: 65_536,
            depth: 0,
            shift: 0,
            children: vec![ld],
        };
        let opts = RenderOptions {
            dpi: 72.0,
            ..Default::default()
        };
        let prims = collect_page(&page, &[], &opts, &mut GlyphCache::new());
        assert_eq!(prims.rects.len(), 3);
        let xs: Vec<f64> = prims.rects.iter().map(|r| r.x).collect();
        assert!((xs[0] - 72.0).abs() < 1e-9);
        // rule 单元 4pt：x2 = x1 + 4（rule 连排， Leaders 不加间隔）。
        assert!((xs[1] - 76.0).abs() < 1e-9);
        assert!((xs[2] - 80.0).abs() < 1e-9);
        assert!(prims.rects.iter().all(|r| (r.w - 4.0).abs() < 1e-9));
    }

    #[test]
    fn leaders_narrower_than_unit_draws_nothing() {
        // 区域 9pt < 单元 10pt → 零绘制（三种 kind 均不越界）。
        let mk = |kind| Node::Leaders {
            kind,
            inner: Box::new(Node::Box(BoxNode {
                kind: BoxKind::HBox,
                width: 10 * 65_536,
                height: 65_536,
                depth: 0,
                shift: 0,
                children: vec![],
            })),
            width: 0,
            stretch: 9 * 65_536,
            shrink: 0,
        };
        for kind in [
            ntex_layout::node::LeadersKind::Leaders,
            ntex_layout::node::LeadersKind::Cleaders,
            ntex_layout::node::LeadersKind::Xleaders,
        ] {
            let page = BoxNode {
                kind: BoxKind::HBox,
                width: 9 * 65_536,
                height: 65_536,
                depth: 0,
                shift: 0,
                children: vec![mk(kind)],
            };
            let opts = RenderOptions {
                dpi: 72.0,
                ..Default::default()
            };
            let prims = collect_page(&page, &[], &opts, &mut GlyphCache::new());
            assert!(prims.rects.is_empty(), "kind {kind:?} 不应绘制");
        }
    }

    #[test]
    fn leaders_char_box_glyph_and_fallback_paths() {
        // inner 含字符：字形关 → 仅占位方框（3 条矩形）；字形开 + 无字体表 →
        // 同样回落方框（不 panic）；两状态都出墨。
        let mk = || Node::Leaders {
            kind: ntex_layout::node::LeadersKind::Leaders,
            inner: Box::new(Node::Box(BoxNode {
                kind: BoxKind::HBox,
                width: 10 * 65_536,
                height: 65_536,
                depth: 0,
                shift: 0,
                children: vec![Node::Char {
                    font: FontId(0),
                    charcode: b'.' as u32,
                    width: 65_536,
                    height: 65_536,
                    depth: 0,
                }],
            })),
            width: 0,
            stretch: 10 * 65_536,
            shrink: 0,
        };
        let page = BoxNode {
            kind: BoxKind::HBox,
            width: 10 * 65_536,
            height: 65_536,
            depth: 0,
            shift: 0,
            children: vec![mk()],
        };
        let base = RenderOptions {
            dpi: 72.0,
            ..Default::default()
        };
        let off = collect_page(&page, &[], &base, &mut GlyphCache::new());
        assert_eq!(off.rects.len(), 3);
        assert!(off.glyphs.is_empty());
        let on_opts = RenderOptions {
            glyphs: true,
            ..base.clone()
        };
        let on = collect_page(&page, &[], &on_opts, &mut GlyphCache::new());
        assert_eq!(on.rects.len(), 3);
    }

    #[test]
    fn validate_options_rejects_bad() {
        assert!(validate_options(&RenderOptions::default()).is_ok());
        let bad = |dpi, size, margin| RenderOptions {
            dpi,
            page_size_pt: size,
            margin_pt: margin,
            debug: false,
            glyphs: false,
        };
        assert!(validate_options(&bad(0.0, (100.0, 100.0), 72.0)).is_err());
        assert!(validate_options(&bad(72.0, (0.0, 100.0), 72.0)).is_err());
        assert!(validate_options(&bad(72.0, (100.0, 100.0), -1.0)).is_err());
    }

    #[test]
    fn collect_page_rule_geometry() {
        // 10pt kern + 2×2pt rule：黑矩形应在 (margin+10pt, margin) 处、2×2pt 大小。
        let page = BoxNode {
            kind: BoxKind::HBox,
            width: 12 * 65_536,
            height: 2 * 65_536,
            depth: 0,
            shift: 0,
            children: vec![
                Node::Kern { width: 10 * 65_536 },
                Node::Rule {
                    width: 2 * 65_536,
                    height: 2 * 65_536,
                    depth: 0,
                },
            ],
        };
        let opts = RenderOptions {
            dpi: 72.0,
            ..Default::default()
        };
        let prims = collect_page(&page, &[], &opts, &mut GlyphCache::new());
        assert_eq!(prims.width, 595);
        assert_eq!(prims.height, 842);
        assert_eq!(prims.rects.len(), 1);
        let r = prims.rects[0];
        assert_eq!(r.color, (0, 0, 0));
        assert!((r.x - 82.0).abs() < 1e-9, "x={}", r.x);
        // 基线 ry = 边距 72 + 页高 2 = 74；rule 顶 = 74 − 高 2 = 72。
        assert!((r.y - 72.0).abs() < 1e-9, "y={}", r.y);
        assert!((r.w - 2.0).abs() < 1e-9 && (r.h - 2.0).abs() < 1e-9);
    }

    #[test]
    fn collect_page_char_placeholder_shape() {
        // 1×1×1pt 字符：外框 + 上区 + 基线段共 3 条矩形。
        let page = BoxNode {
            kind: BoxKind::HBox,
            width: 65_536,
            height: 65_536,
            depth: 65_536,
            shift: 0,
            children: vec![Node::Char {
                font: ntex_layout::node::FontId(0),
                charcode: 65,
                width: 65_536,
                height: 65_536,
                depth: 65_536,
            }],
        };
        let opts = RenderOptions {
            dpi: 72.0,
            ..Default::default()
        };
        let prims = collect_page(&page, &[], &opts, &mut GlyphCache::new());
        assert_eq!(prims.rects.len(), 3);
        // 基线 y = margin 72 + 页高 1 = 73；上区 [72,73)、基线段 y=73 h=1。
        let baseline = prims.rects[2];
        assert!((baseline.y - 73.0).abs() < 1e-9 && (baseline.h - 1.0).abs() < 1e-9);
    }

    #[test]
    fn debug_overlay_is_separate_channel() {
        // 开/关 debug：rects 必须完全一致（差分口径不变）；开启时 debug 非空。
        let page = BoxNode {
            kind: BoxKind::HBox,
            width: 65_536,
            height: 65_536,
            depth: 0,
            shift: 0,
            children: vec![
                Node::Char {
                    font: ntex_layout::node::FontId(0),
                    charcode: 65,
                    width: 65_536,
                    height: 65_536,
                    depth: 0,
                },
                Node::Glue {
                    name: None,
                    width: 2 * 65_536,
                    stretch: 65_536,
                    shrink: 0,
                    stretch_order: 0,
                    shrink_order: 0,
                },
                Node::Penalty { penalty: -100 },
            ],
        };
        let collect = |debug| {
            collect_page(
                &page,
                &[],
                &RenderOptions {
                    dpi: 72.0,
                    debug,
                    ..Default::default()
                },
                &mut GlyphCache::new(),
            )
        };
        let off = collect(false);
        let on = collect(true);
        assert!(off.debug.is_empty());
        assert_eq!(off.rects, on.rects, "overlay 不得影响内容通道");
        assert!(!on.debug.is_empty());
        // 颜色覆盖：版心描边 + HBox 描边/基线 + glue 带 + stretch 线 + penalty 线。
        let colors: Vec<_> = on.debug.iter().map(|r| r.color).collect();
        for expect in [
            DBG_MARGIN,
            DBG_HBOX,
            DBG_BASELINE,
            DBG_GLUE,
            DBG_GLUE_STRETCH,
            DBG_PENALTY,
        ] {
            assert!(colors.contains(&expect), "缺颜色 {expect:?}");
        }
    }

    #[test]
    fn debug_glue_band_geometry() {
        // 10pt 宽 glue（stretch 4pt），72dpi：带 x∈[72,82)、y∈[ry-4,ry-1)，
        // stretch 线 y=ry-5、w=4；ry = 72（边距，页高 0）。
        let page = BoxNode {
            kind: BoxKind::HBox,
            width: 10 * 65_536,
            height: 0,
            depth: 0,
            shift: 0,
            children: vec![Node::Glue {
                name: None,
                width: 10 * 65_536,
                stretch: 4 * 65_536,
                shrink: 0,
                stretch_order: 0,
                shrink_order: 0,
            }],
        };
        let prims = collect_page(
            &page,
            &[],
            &RenderOptions {
                dpi: 72.0,
                debug: true,
                ..Default::default()
            },
            &mut GlyphCache::new(),
        );
        let band = prims.debug.iter().find(|r| r.color == DBG_GLUE).unwrap();
        assert!((band.x - 72.0).abs() < 1e-9 && (band.w - 10.0).abs() < 1e-9);
        assert!((band.y - 68.0).abs() < 1e-9 && (band.h - 3.0).abs() < 1e-9);
        let stretch = prims
            .debug
            .iter()
            .find(|r| r.color == DBG_GLUE_STRETCH)
            .unwrap();
        assert!((stretch.y - 67.0).abs() < 1e-9 && (stretch.w - 4.0).abs() < 1e-9);
        // fil 阶：带色换亮绿。
        let fil = BoxNode {
            kind: BoxKind::HBox,
            width: 0,
            height: 0,
            depth: 0,
            shift: 0,
            children: vec![Node::Glue {
                name: None,
                width: 0,
                stretch: 65_536,
                shrink: 0,
                stretch_order: 1,
                shrink_order: 0,
            }],
        };
        let prims = collect_page(
            &fil,
            &[],
            &RenderOptions {
                dpi: 72.0,
                debug: true,
                ..Default::default()
            },
            &mut GlyphCache::new(),
        );
        assert!(
            !prims.debug.iter().any(|r| r.color == DBG_GLUE),
            "fil 阶不带浅黄带"
        );
    }

    #[test]
    fn debug_penalty_inf_thicker() {
        // 禁断点（penalty ≥ 10000）用深红粗线，可选断点用品红细线。
        let mk = |penalty| BoxNode {
            kind: BoxKind::HBox,
            width: 0,
            height: 2 * 65_536,
            depth: 65_536,
            shift: 0,
            children: vec![Node::Penalty { penalty }],
        };
        let opts = |debug| RenderOptions {
            dpi: 72.0,
            debug,
            ..Default::default()
        };
        let inf = collect_page(&mk(10_000), &[], &opts(true), &mut GlyphCache::new());
        let inf = inf
            .debug
            .iter()
            .find(|r| r.color == DBG_PENALTY_INF)
            .unwrap();
        // 高度有 4px 下限（空行盒保护）。
        assert!((inf.w - 2.0).abs() < 1e-9 && (inf.h - 4.0).abs() < 1e-9);
        let opt = collect_page(&mk(-50), &[], &opts(true), &mut GlyphCache::new());
        let opt = opt.debug.iter().find(|r| r.color == DBG_PENALTY).unwrap();
        assert!((opt.w - 1.0).abs() < 1e-9);
    }
}
