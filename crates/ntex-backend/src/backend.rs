//! 渲染后端抽象（plan.md §10 M8 / §12 `ntex-backend` 规划）。
//!
//! [`Backend`] 是面向未来后端（WebGPU / WASM Canvas）的稳定接口：输入统一为
//! `Typesetter::typeset_dvi` 产物（`pages: &[BoxNode]` + `fonts: &[FontMetrics]`，
//! 与 `ntex_dvi::write_dvi` 同源），输出统一为逐页 [`Pixmap`]。后端差异
//! （软光栅 / GPU / Canvas）收敛在实现内部，调用方无感知。
//!
//! 坐标语义对照 `ntex_dvi`（页面原点左上、y 向下、sp 单位）：
//!
//! - 页面盒 HBox：基线 = 页参考点；VBox：顶 = 参考点（`vlist` 自行 − height）；
//! - 盒参考点由调用方传入（边距），绘制在像素坐标（`sp_to_px` 换算）；
//! - 字符无字形：TFM 只有度量（M9 字体子系统范围），画占位方框 + 基线标记。

use std::fmt;

use ntex_font::FontMetrics;
use ntex_layout::node::{BoxKind, BoxNode, Node};

use crate::png::{encode_png, PngError};
use crate::raster::{sp_to_px, Pixmap};

/// 占位字符的灰度（浅色示意，非最终墨色；字形渲染属 M9）。
const CHAR_GRAY: u8 = 160;
/// 占位字符方框的底色（浅灰，深灰描边用同色系区分）。
const CHAR_BG_GRAY: u8 = 225;

/// 渲染配置。
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

/// 渲染错误（输入可达路径不 panic；越界坐标一律裁剪而非报错）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError(pub String);

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "渲染失败：{}", self.0)
    }
}

impl std::error::Error for BackendError {}

/// 渲染后端 trait：排版产物 → 逐页像素。
///
/// 页面语义与 `ntex_dvi::write_dvi` 相同：`pages[i]` 是第 i 页的整页盒
/// （HBox 基线 = `page.height`；VBox 顶从参考点 − height 开始）。参考点
/// 原点 = 左边距 + 顶边距（pt 换算 px），y 向下。
pub trait Backend {
    /// 渲染全部页面（页序 = 输入顺序）。
    fn render(
        &self,
        pages: &[BoxNode],
        fonts: &[FontMetrics],
        opts: &RenderOptions,
    ) -> Result<Vec<Pixmap>, BackendError>;

    /// 便捷：渲染全部页面并编码为 PNG 序列。
    fn render_pngs(
        &self,
        pages: &[BoxNode],
        fonts: &[FontMetrics],
        opts: &RenderOptions,
    ) -> Result<Vec<Vec<u8>>, BackendError> {
        self.render(pages, fonts, opts)?
            .iter()
            .map(|pm| encode_png(pm).map_err(|PngError(msg)| BackendError(msg)))
            .collect()
    }
}

/// 软光栅实现（RGBA8 预乘 + 白底；tiny-skia 语义，见 lib.rs 偏差说明）。
#[derive(Debug, Clone, Copy, Default)]
pub struct TinySkiaBackend;

/// pt → sp（配置里的 pt 值转内部 sp 坐标）。
fn pt_to_sp(pt: f64) -> i64 {
    (pt * 65_536.0) as i64
}

impl Backend for TinySkiaBackend {
    fn render(
        &self,
        pages: &[BoxNode],
        _fonts: &[FontMetrics],
        opts: &RenderOptions,
    ) -> Result<Vec<Pixmap>, BackendError> {
        if !opts.dpi.is_finite() || opts.dpi <= 0.0 {
            return Err(BackendError(format!("非法 DPI：{}", opts.dpi)));
        }
        let (w_pt, h_pt) = opts.page_size_pt;
        if !w_pt.is_finite() || !h_pt.is_finite() || w_pt <= 0.0 || h_pt <= 0.0 {
            return Err(BackendError(format!("非法页面尺寸：{w_pt}×{h_pt}pt")));
        }
        if !opts.margin_pt.is_finite() || opts.margin_pt < 0.0 {
            return Err(BackendError(format!("非法页边距：{}", opts.margin_pt)));
        }
        let mut out = Vec::with_capacity(pages.len());
        for page in pages {
            let mut pm = Pixmap::new(
                sp_to_px(pt_to_sp(w_pt), opts.dpi).round() as u32,
                sp_to_px(pt_to_sp(h_pt), opts.dpi).round() as u32,
            );
            pm.fill(255, 255, 255);
            // 页面盒参考点：HBox 基线 = 边距 + 页高；VBox 顶 = 边距。
            let rx = sp_to_px(pt_to_sp(opts.margin_pt), opts.dpi);
            let ry = match page.kind {
                BoxKind::HBox => rx + sp_to_px(page.height, opts.dpi),
                BoxKind::VBox => rx,
            };
            draw_box(&mut pm, page, rx, ry, opts.dpi);
            out.push(pm);
        }
        Ok(out)
    }
}

/// 递归绘制一个盒子；`(rx, ry)` = 盒参考点（px）：HBox 基线，VBox 顶。
fn draw_box(pm: &mut Pixmap, bx: &BoxNode, rx: f64, ry: f64, dpi: f64) {
    match bx.kind {
        BoxKind::HBox => draw_hlist(pm, bx, rx, ry, dpi),
        BoxKind::VBox => draw_vlist(pm, bx, rx, ry, dpi),
    }
}

/// 水平列表（对照 ntex-dvi `hlist`）：字符/规则坐基线，盒 shift 下移。
fn draw_hlist(pm: &mut Pixmap, bx: &BoxNode, rx: f64, ry: f64, dpi: f64) {
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
                pm.fill_rect(
                    x,
                    top,
                    w,
                    bot - top,
                    CHAR_BG_GRAY,
                    CHAR_BG_GRAY,
                    CHAR_BG_GRAY,
                );
                pm.fill_rect(
                    x,
                    top,
                    w,
                    sp_to_px(*height, dpi),
                    CHAR_GRAY,
                    CHAR_GRAY,
                    CHAR_GRAY,
                );
                pm.fill_hline(ry, x, x + w, CHAR_GRAY, CHAR_GRAY, CHAR_GRAY);
                cur_h += width;
            }
            Node::Rule {
                width,
                height,
                depth,
            } => {
                let x = rx + sp_to_px(cur_h, dpi);
                let top = ry - sp_to_px(*height, dpi);
                pm.fill_rect(
                    x,
                    top,
                    sp_to_px(*width, dpi),
                    sp_to_px(height + depth, dpi),
                    0,
                    0,
                    0,
                );
                cur_h += width;
            }
            Node::Glue { width, .. } | Node::Kern { width } => cur_h += width,
            Node::Box(inner) => {
                let x = rx + sp_to_px(cur_h, dpi);
                let child_ry = ry + sp_to_px(inner.shift, dpi);
                draw_box(pm, inner, x, child_ry, dpi);
                cur_h += inner.width;
            }
            _ => {}
        }
    }
}

/// 垂直列表（对照 ntex-dvi `vlist`）：顶 = 参考点，逐子节点向下推进。
fn draw_vlist(pm: &mut Pixmap, bx: &BoxNode, rx: f64, ry: f64, dpi: f64) {
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
                    BoxKind::HBox => draw_hlist(pm, inner, child_rx, child_ref_y, dpi),
                    BoxKind::VBox => draw_vlist(pm, inner, child_rx, child_ref_y, dpi),
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
                pm.fill_rect(
                    rx,
                    top,
                    sp_to_px(*width, dpi),
                    sp_to_px(height + depth, dpi),
                    0,
                    0,
                    0,
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
    use ntex_layout::node::FontId;
    fn page_with_children(children: Vec<Node>) -> BoxNode {
        BoxNode {
            kind: BoxKind::HBox,
            width: 0,
            height: 0,
            depth: 0,
            shift: 0,
            children,
        }
    }

    fn char_node(w: i64, h: i64, d: i64) -> Node {
        Node::Char {
            font: FontId(0),
            charcode: 65,
            width: w,
            height: h,
            depth: d,
        }
    }

    #[test]
    fn rejects_bad_options() {
        let bad_dpi = RenderOptions {
            dpi: 0.0,
            ..Default::default()
        };
        assert!(TinySkiaBackend.render(&[], &[], &bad_dpi).is_err());
        let bad_size = RenderOptions {
            page_size_pt: (0.0, 100.0),
            ..Default::default()
        };
        assert!(TinySkiaBackend.render(&[], &[], &bad_size).is_err());
    }

    #[test]
    fn empty_pages_render_white() {
        let pms = TinySkiaBackend
            .render(
                &[page_with_children(vec![])],
                &[],
                &RenderOptions::default(),
            )
            .unwrap();
        let pm = &pms[0];
        assert!(!pm.pixel_nonwhite(0, 0));
        assert!(!pm.pixel_nonwhite(pm.width() - 1, pm.height() - 1));
    }

    #[test]
    fn char_placeholder_paints_expected_area() {
        // 1pt 宽/高/深的字符，72dpi：1sp 块 = 1px；边距 72px。
        let page = page_with_children(vec![char_node(65_536, 65_536, 65_536)]);
        let opts = RenderOptions {
            dpi: 72.0,
            ..Default::default()
        };
        let pm = &TinySkiaBackend.render(&[page], &[], &opts).unwrap()[0];
        // 方框从 (72,72) 起占 1px 宽；基线 y = 72（边距 + 页高 0）。
        assert!(pm.pixel_nonwhite(72, 72));
        assert!(!pm.pixel_nonwhite(71, 72));
        assert!(!pm.pixel_nonwhite(74, 74));
    }
}
