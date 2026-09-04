//! 渲染后端抽象（plan.md §10 M8 / §12 `ntex-backend` 规划）。
//!
//! [`Backend`] 是面向各光栅化实现的稳定接口：输入统一为
//! `Typesetter::typeset_dvi` 产物（`pages: &[BoxNode]` + `fonts: &[FontMetrics]`，
//! 与 `ntex_dvi::write_dvi` 同源），输出统一为逐页 [`Pixmap`]。后端差异
//! （软光栅 / GPU）收敛在实现内部，调用方无感知。
//!
//! 盒树遍历与坐标换算在 [`crate::prims`] 共享（两后端几何一致，可差分）；
//! 坐标语义对照 `ntex_dvi`（页面原点左上、y 向下、sp 单位）：
//!
//! - 页面盒 HBox：基线 = 页参考点；VBox：顶 = 参考点（`vlist` 自行 − height）；
//! - 字符无字形：TFM 只有度量（M9 字体子系统范围），画占位方框 + 基线标记。

use std::fmt;

use ntex_font::FontMetrics;
use ntex_layout::node::BoxNode;

use crate::png::{encode_png, PngError};
use crate::prims::{collect_page, validate_options, RenderOptions};
use crate::raster::Pixmap;

pub use crate::prims::RectPrim;

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

impl Backend for TinySkiaBackend {
    fn render(
        &self,
        pages: &[BoxNode],
        _fonts: &[FontMetrics],
        opts: &RenderOptions,
    ) -> Result<Vec<Pixmap>, BackendError> {
        validate_options(opts).map_err(BackendError)?;
        pages
            .iter()
            .map(|page| {
                let prims = collect_page(page, opts);
                let mut pm = Pixmap::new(prims.width, prims.height);
                pm.fill(255, 255, 255);
                for r in &prims.rects {
                    pm.fill_rect(r.x, r.y, r.w, r.h, r.color);
                }
                Ok(pm)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ntex_layout::node::{BoxKind, FontId, Node};

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
