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

use ntex_layout::node::{BoxKind, BoxNode, Node};

use crate::raster::sp_to_px;

/// 占位字符的灰度（浅色示意，非最终墨色；字形渲染属 M9）。
const CHAR_GRAY: u8 = 160;
/// 占位字符方框的底色（浅灰，深灰描边用同色系区分）。
const CHAR_BG_GRAY: u8 = 225;

/// 渲染配置（两后端共用）。
#[derive(Debug, Clone, PartialEq)]
pub struct RenderOptions {
    /// 渲染 DPI（默认 144，A4 ≈ 1191×1684px）。
    pub dpi: f64,
    /// 页面尺寸（pt）：宽、高（默认 A4，同 ntex-pdf::PdfOptions）。
    pub page_size_pt: (f64, f64),
    /// 页边距（pt）：四边同值（默认 72pt = 1in）。
    pub margin_pt: f64,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            dpi: 144.0,
            page_size_pt: (595.276, 841.890),
            margin_pt: 72.0,
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
pub struct PagePrims {
    pub width: u32,
    pub height: u32,
    pub rects: Vec<RectPrim>,
}

/// pt → sp（配置里的 pt 值转内部 sp 坐标）。
fn pt_to_sp(pt: f64) -> i64 {
    (pt * 65_536.0) as i64
}

/// 收集一页的矩形指令（页面白底清屏由各后端自行负责）。
pub fn collect_page(page: &BoxNode, opts: &RenderOptions) -> PagePrims {
    let (w_pt, h_pt) = opts.page_size_pt;
    let mut rects = Vec::new();
    let rx = sp_to_px(pt_to_sp(opts.margin_pt), opts.dpi);
    // 页面盒参考点：HBox 基线 = 边距 + 页高；VBox 顶 = 边距。
    let ry = match page.kind {
        BoxKind::HBox => rx + sp_to_px(page.height, opts.dpi),
        BoxKind::VBox => rx,
    };
    collect_box(page, rx, ry, opts.dpi, &mut rects);
    PagePrims {
        width: sp_to_px(pt_to_sp(w_pt), opts.dpi).round() as u32,
        height: sp_to_px(pt_to_sp(h_pt), opts.dpi).round() as u32,
        rects,
    }
}

/// 递归收集一个盒子；`(rx, ry)` = 盒参考点（px）：HBox 基线，VBox 顶。
fn collect_box(bx: &BoxNode, rx: f64, ry: f64, dpi: f64, out: &mut Vec<RectPrim>) {
    match bx.kind {
        BoxKind::HBox => collect_hlist(bx, rx, ry, dpi, out),
        BoxKind::VBox => collect_vlist(bx, rx, ry, dpi, out),
    }
}

/// 水平列表（对照 ntex-dvi `hlist`）：字符/规则坐基线，盒 shift 下移。
fn collect_hlist(bx: &BoxNode, rx: f64, ry: f64, dpi: f64, out: &mut Vec<RectPrim>) {
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
            Node::Glue { width, .. } | Node::Kern { width } => cur_h += width,
            Node::Box(inner) => {
                let x = rx + sp_to_px(cur_h, dpi);
                let child_ry = ry + sp_to_px(inner.shift, dpi);
                collect_box(inner, x, child_ry, dpi, out);
                cur_h += inner.width;
            }
            _ => {}
        }
    }
}

/// 垂直列表（对照 ntex-dvi `vlist`）：顶 = 参考点，逐子节点向下推进。
fn collect_vlist(bx: &BoxNode, rx: f64, ry: f64, dpi: f64, out: &mut Vec<RectPrim>) {
    let mut cur_v = 0i64;
    // ntex-dvi vlist：参考点即页顶（cur_v -= height 回 0 的等价形式）；
    // 本函数直接以 ry = 顶、cur_v 从 0 推进。
    for node in &bx.children {
        match node {
            Node::Box(inner) => {
                // 先推进 height，再以「参考点 + height」为子盒基线（HBox）
                // 或子盒顶（VBox）。
                cur_v += inner.height;
                let child_ref_y = ry + sp_to_px(cur_v, dpi);
                let child_rx = rx + sp_to_px(inner.shift, dpi);
                match inner.kind {
                    BoxKind::HBox => collect_hlist(inner, child_rx, child_ref_y, dpi, out),
                    BoxKind::VBox => collect_vlist(inner, child_rx, child_ref_y, dpi, out),
                }
                cur_v += inner.depth;
            }
            Node::Glue { width, .. } | Node::Kern { width } => cur_v += width,
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
}
