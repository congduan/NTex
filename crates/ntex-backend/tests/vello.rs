//! vello GPU 后端差分测试：与软光栅同输入渲染，矩形几何应一致。
//!
//! 两后端共享 `prims` 遍历，唯一差异是光栅化（GPU 亚像素 AA vs 整数
//! 覆盖盒），故断言避开边缘 1-2px，只比对「内部有墨 / 远端无墨」。
//! 无 GPU 适配器的环境（CI 容器等）自动跳过。

use ntex_backend::raster::sp_to_px;
use ntex_backend::{Backend, Pixmap, RenderOptions, TinySkiaBackend, VelloBackend};
use ntex_layout::node::{BoxKind, BoxNode, Node};

const SP_PER_PT: i64 = 65_536;

fn render_page(children: Vec<Node>, dpi: f64) -> Result<Pixmap, ntex_backend::BackendError> {
    let page = BoxNode {
        kind: BoxKind::HBox,
        width: 0,
        height: 0,
        depth: 0,
        shift: 0,
        children,
    };
    Ok(VelloBackend::new()
        .render(
            &[page],
            &[],
            &RenderOptions {
                dpi,
                ..Default::default()
            },
        )?
        .remove(0))
}

fn region_has_ink(pm: &Pixmap, x0: u32, y0: u32, x1: u32, y1: u32) -> bool {
    (y0..y1).any(|y| (x0..x1).any(|x| pm.pixel_nonwhite(x, y)))
}

#[test]
fn vello_rule_geometry_matches_soft_raster() {
    // 10pt kern + 2×20pt rule，72dpi：黑矩形 x∈[82,84)、y∈[52,72)。
    let children = vec![
        Node::Kern {
            width: 10 * SP_PER_PT,
        },
        Node::Rule {
            width: 2 * SP_PER_PT,
            height: 20 * SP_PER_PT,
            depth: 0,
        },
    ];
    let gpu = render_page(children.clone(), 72.0);
    if gpu.is_err() {
        eprintln!("跳过（GPU 不可用）：{}", gpu.err().unwrap());
        return;
    }
    let pm = gpu.unwrap();
    assert_eq!(pm.width(), 595);
    assert_eq!(pm.height(), 842);
    // 内部有墨（避开 AA 边缘 1px）。
    assert!(region_has_ink(&pm, 82, 53, 84, 71));
    // 远端无墨（左右各留 2px 余量）。
    assert!(!region_has_ink(&pm, 60, 40, 80, 80));
    assert!(!region_has_ink(&pm, 86, 40, 120, 80));

    // 与软光栅差分：同一输入的墨区域行/列范围应一致（±2px 容差）。
    let soft = TinySkiaBackend
        .render(
            &[BoxNode {
                kind: BoxKind::HBox,
                width: 0,
                height: 0,
                depth: 0,
                shift: 0,
                children,
            }],
            &[],
            &RenderOptions {
                dpi: 72.0,
                ..Default::default()
            },
        )
        .unwrap()
        .remove(0);
    let bbox = |pm: &Pixmap| (ink_cols(pm), ink_rows(pm));
    let (gc, gr) = bbox(&pm);
    let (sc, sr) = bbox(&soft);
    let close = |a: (u32, u32), b: (u32, u32)| (a.0.abs_diff(b.0) <= 2) && (a.1.abs_diff(b.1) <= 2);
    assert!(close(gc, sc), "列范围 {gc:?} vs {sc:?}");
    assert!(close(gr, sr), "行范围 {gr:?} vs {sr:?}");
}

#[test]
fn vello_multi_page_order_and_png_magic() {
    let page = BoxNode {
        kind: BoxKind::VBox,
        width: 0,
        height: 0,
        depth: 0,
        shift: 0,
        children: vec![Node::Rule {
            width: 4 * SP_PER_PT,
            height: 2 * SP_PER_PT,
            depth: 0,
        }],
    };
    let pages = vec![page.clone(), page];
    let pngs = VelloBackend::new().render_pngs(
        &pages,
        &[],
        &RenderOptions {
            dpi: 72.0,
            ..Default::default()
        },
    );
    match pngs {
        Ok(pngs) => {
            assert_eq!(pngs.len(), 2);
            for png in &pngs {
                assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
            }
        }
        Err(e) => eprintln!("跳过（GPU 不可用）：{e}"),
    }
}

#[test]
fn vello_rejects_bad_options() {
    let bad = RenderOptions {
        dpi: 0.0,
        ..Default::default()
    };
    // 配置校验在 GPU 初始化之前，无 GPU 环境也应报错而非跳过。
    let err = VelloBackend::new().render(&[], &[], &bad);
    assert!(err.is_err());
    assert!(err.unwrap_err().0.contains("DPI"));
}

/// 有墨列范围（首个/末个非白列）。
fn ink_cols(pm: &Pixmap) -> (u32, u32) {
    let first = (0..pm.width()).find(|&x| (0..pm.height()).any(|y| pm.pixel_nonwhite(x, y)));
    let last = (0..pm.width())
        .rev()
        .find(|&x| (0..pm.height()).any(|y| pm.pixel_nonwhite(x, y)));
    (first.unwrap_or(0), last.unwrap_or(0))
}

/// 有墨行范围（首个/末个非白行）。
fn ink_rows(pm: &Pixmap) -> (u32, u32) {
    let first = (0..pm.height()).find(|&y| (0..pm.width()).any(|x| pm.pixel_nonwhite(x, y)));
    let last = (0..pm.height())
        .rev()
        .find(|&y| (0..pm.width()).any(|x| pm.pixel_nonwhite(x, y)));
    (first.unwrap_or(0), last.unwrap_or(0))
}

#[test]
fn sp_to_px_consistency() {
    assert!((sp_to_px(SP_PER_PT, 72.0) - 1.0).abs() < 1e-12);
}
