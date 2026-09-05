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
//! - 字符无字形：TFM 只有度量（M9 字体子系统范围），画占位方框 + 基线标记。

use ntex_layout::node::{BoxKind, BoxNode, GlueOrder, Node, GLUE_ORDER_FIL};

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
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            dpi: 144.0,
            page_size_pt: (595.276, 841.890),
            margin_pt: 72.0,
            debug: false,
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
pub struct PagePrims {
    pub width: u32,
    pub height: u32,
    pub rects: Vec<RectPrim>,
    pub debug: Vec<RectPrim>,
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
pub fn collect_page(page: &BoxNode, opts: &RenderOptions) -> PagePrims {
    let (w_pt, h_pt) = opts.page_size_pt;
    let mut rects = Vec::new();
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
    collect_box(page, rx, ry, opts.dpi, &mut rects, &mut dbg);
    PagePrims {
        width: page_w_px.round() as u32,
        height: page_h_px.round() as u32,
        rects,
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
            collect_hlist(bx, rx, ry, dpi, out, dbg);
        }
        BoxKind::VBox => {
            dbg_stroke(dbg, rx, ry, w_px, h_px + d_px, DBG_VBOX);
            collect_vlist(bx, rx, ry, dpi, out, dbg);
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
    dbg: &mut DebugOut,
) {
    let mut cur_h = 0i64;
    for node in &bx.children {
        match node {
            Node::Char {
                width,
                height,
                depth,
                ..
            }
            | Node::Ligature {
                width,
                height,
                depth,
                ..
            } => {
                let x = rx + sp_to_px(cur_h, dpi);
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
                collect_box(inner, x, child_ry, dpi, out, dbg);
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
                collect_box(inner, child_rx, child_ref_y, dpi, out, dbg);
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
    fn validate_options_rejects_bad() {
        assert!(validate_options(&RenderOptions::default()).is_ok());
        let bad = |dpi, size, margin| RenderOptions {
            dpi,
            page_size_pt: size,
            margin_pt: margin,
            debug: false,
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
        let prims = collect_page(&page, &opts);
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
        let prims = collect_page(&page, &opts);
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
                &RenderOptions {
                    dpi: 72.0,
                    debug,
                    ..Default::default()
                },
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
            &RenderOptions {
                dpi: 72.0,
                debug: true,
                ..Default::default()
            },
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
            &RenderOptions {
                dpi: 72.0,
                debug: true,
                ..Default::default()
            },
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
        let inf = collect_page(&mk(10_000), &opts(true));
        let inf = inf
            .debug
            .iter()
            .find(|r| r.color == DBG_PENALTY_INF)
            .unwrap();
        // 高度有 4px 下限（空行盒保护）。
        assert!((inf.w - 2.0).abs() < 1e-9 && (inf.h - 4.0).abs() < 1e-9);
        let opt = collect_page(&mk(-50), &opts(true));
        let opt = opt.debug.iter().find(|r| r.color == DBG_PENALTY).unwrap();
        assert!((opt.w - 1.0).abs() < 1e-9);
    }
}
