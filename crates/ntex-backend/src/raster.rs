//! 软光栅化与像素缓冲（纯 Rust，无第三方依赖）。
//!
//! 像素格式：RGBA8（预乘 alpha，与 tiny-skia `PremultipliedColorU8` 一致），
//! 行主序、每像素 4 字节。矩形填充走整数覆盖盒、裁剪到页面边界；字形轮廓
//! 走 [`Pixmap::fill_polygon`]（扫描线 AA，M8 真字形）。

/// 8 位 RGBA 像素缓冲（预乘 alpha）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pixmap {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Pixmap {
    /// 新建全透明缓冲（维度下限 1×1，避免退化尺寸）。
    ///
    /// 字节数走 `u64` checked 计算：wasm32 的 `usize` 只有 32 位，两个接近
    /// `u32::MAX` 的维度直接相乘会**回绕**（`(2³²-1)² × 4 ≡ 4`）——`data`
    /// 只分到 4 字节、`width/height` 却仍是 42 亿，随后 `fill_rect` 一索引
    /// 就 panic，在 `panic = abort` 下变成无行号的 wasm trap。
    ///
    /// 生产路径已由 [`crate::prims::validate_options`] 的
    /// [`MAX_PAGE_PIXELS`](crate::prims::MAX_PAGE_PIXELS) 先挡；这里的兜底
    /// 针对绕过校验的旁路调用——**宁可退化成 1×1 空图，不可 trap**。
    pub fn new(width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let len = (width as u64)
            .checked_mul(height as u64)
            .and_then(|n| n.checked_mul(4))
            .filter(|&n| n <= usize::MAX as u64);
        match len {
            Some(n) => Self {
                width,
                height,
                data: vec![0; n as usize],
            },
            None => Self {
                width: 1,
                height: 1,
                data: vec![0; 4],
            },
        }
    }

    /// 从紧凑 RGBA8 字节构造（行主序、无 padding；长度必须恰为 w×h×4）。
    ///
    /// 供 GPU 回读路径使用（vello 后端）；长度不符返回 `None`，不 panic。
    pub fn from_rgba(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        if width == 0 || height == 0 || data.len() != width as usize * height as usize * 4 {
            return None;
        }
        Some(Self {
            width,
            height,
            data,
        })
    }

    /// 宽（px）。
    pub fn width(&self) -> u32 {
        self.width
    }

    /// 高（px）。
    pub fn height(&self) -> u32 {
        self.height
    }

    /// 原始字节（RGBA8，行主序）。
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// 整页填充不透明色（白底用）。
    pub fn fill(&mut self, r: u8, g: u8, b: u8) {
        // clippy::chunks_exact_to_as_chunks（Rust 1.98 新增）：常量块大小用 as_chunks_mut
        for px in self.data.as_chunks_mut::<4>().0 {
            px[0] = r;
            px[1] = g;
            px[2] = b;
            px[3] = 255;
        }
    }

    /// 不透明填充色（RGB，alpha 固定 255）。
    pub const OPAQUE: (u8, u8, u8) = (0, 0, 0);

    /// 填充矩形（源坐标可为浮点像素，取覆盖盒，裁剪到页面）。
    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64, color: (u8, u8, u8)) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let x0 = x.floor().max(0.0) as i64;
        let y0 = y.floor().max(0.0) as i64;
        let x1 = ((x + w).ceil() as i64).min(self.width as i64);
        let y1 = ((y + h).ceil() as i64).min(self.height as i64);
        let (r, g, b) = color;
        for py in y0..y1 {
            for px in x0..x1 {
                let idx = (py as usize * self.width as usize + px as usize) * 4;
                self.data[idx] = r;
                self.data[idx + 1] = g;
                self.data[idx + 2] = b;
                self.data[idx + 3] = 255;
            }
        }
    }

    /// 把紧凑 RGB8 图片缩放绘制到页面。采用最近邻采样；坐标与尺寸可为
    /// 浮点并自动裁剪。该路径服务 WASM/Tauri 的 `\includegraphics` 预览，
    /// 输入长度或尺寸非法时安全返回 `false`，不触发越界或分配。
    pub fn blit_rgb_scaled(
        &mut self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        src_width: u32,
        src_height: u32,
        rgb: &[u8],
    ) -> bool {
        let expected = (src_width as u64)
            .checked_mul(src_height as u64)
            .and_then(|n| n.checked_mul(3));
        if src_width == 0
            || src_height == 0
            || !x.is_finite()
            || !y.is_finite()
            || !w.is_finite()
            || !h.is_finite()
            || w <= 0.0
            || h <= 0.0
            || expected != Some(rgb.len() as u64)
        {
            return false;
        }
        let x0 = x.floor().max(0.0) as i64;
        let y0 = y.floor().max(0.0) as i64;
        let x1 = ((x + w).ceil() as i64).min(self.width as i64);
        let y1 = ((y + h).ceil() as i64).min(self.height as i64);
        for py in y0..y1 {
            let sy = (((py as f64 + 0.5 - y) / h) * src_height as f64)
                .floor()
                .clamp(0.0, (src_height - 1) as f64) as usize;
            for px in x0..x1 {
                let sx = (((px as f64 + 0.5 - x) / w) * src_width as f64)
                    .floor()
                    .clamp(0.0, (src_width - 1) as f64) as usize;
                let si = (sy * src_width as usize + sx) * 3;
                let di = (py as usize * self.width as usize + px as usize) * 4;
                self.data[di..di + 3].copy_from_slice(&rgb[si..si + 3]);
                self.data[di + 3] = 255;
            }
        }
        true
    }

    /// 填充水平线段（厚度 1px）。
    pub fn fill_hline(&mut self, y: f64, x0: f64, x1: f64, color: (u8, u8, u8)) {
        let (lo, hi) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        self.fill_rect(lo, y, hi - lo, 1.0, color);
    }

    /// 填充多边形轮廓组（nonzero 绕组；真字形软光栅化用，M8）。
    ///
    /// 输入为页面坐标闭合子路径（`glyphs::GlyphFont::outline_paths` 产出，
    /// y 向下）；抗锯齿 = 每像素行 4 条子扫描线 + 水平向精确覆盖长度，
    /// 覆盖率（0..=1）作 alpha 与既有像素做 source-over 合成（预乘语义）。
    /// 空组/退化轮廓/越界 bbox 一律安全无操作（引擎契约：不 panic）。
    pub fn fill_polygon(&mut self, contours: &[Vec<[f32; 2]>], color: (u8, u8, u8)) {
        // 边收集：(ymin, ymax, x@ymin, dx/dy, 绕组方向)；同时累计 bbox。
        let mut edges: Vec<(f32, f32, f32, f32, i32)> = Vec::new();
        let (mut bx0, mut by0, mut bx1, mut by1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for c in contours {
            if c.len() < 3 {
                continue;
            }
            for i in 0..c.len() {
                let a = c[i];
                let b = c[(i + 1) % c.len()];
                for p in [a, b] {
                    if !p[0].is_finite() || !p[1].is_finite() {
                        return;
                    }
                    bx0 = bx0.min(p[0]);
                    bx1 = bx1.max(p[0]);
                    by0 = by0.min(p[1]);
                    by1 = by1.max(p[1]);
                }
                if a[1] == b[1] {
                    continue; // 水平边不参与扫描线
                }
                let (ymin, ymax, xa, xb) = if a[1] < b[1] {
                    (a[1], b[1], a[0], b[0])
                } else {
                    (b[1], a[1], b[0], a[0])
                };
                let dir = if a[1] < b[1] { 1 } else { -1 };
                edges.push((ymin, ymax, xa, (xb - xa) / (ymax - ymin), dir));
            }
        }
        if edges.is_empty() {
            return;
        }
        // bbox 裁剪到页面（覆盖列区间 [x_lo, x_hi)、行区间 [y_lo, y_hi)）。
        let x_lo = bx0.floor().max(0.0) as u32;
        let y_lo = by0.floor().max(0.0) as u32;
        let x_hi = ((bx1.ceil().max(0.0) as u64).min(self.width as u64)) as u32;
        let y_hi = ((by1.ceil().max(0.0) as u64).min(self.height as u64)) as u32;
        if x_hi <= x_lo || y_hi <= y_lo {
            return;
        }
        let w = self.width as usize;
        let mut cover = vec![0.0f32; (x_hi - x_lo) as usize];
        let mut xs: Vec<(f32, i32)> = Vec::new();
        for py in y_lo..y_hi {
            cover.fill(0.0);
            for k in 0..4u32 {
                let ys = py as f32 + (k as f32 + 0.5) / 4.0;
                xs.clear();
                for &(ymin, ymax, x0, slope, dir) in &edges {
                    if ys >= ymin && ys < ymax {
                        xs.push((x0 + (ys - ymin) * slope, dir));
                    }
                }
                if xs.len() < 2 {
                    continue;
                }
                xs.sort_by(|p, q| p.0.total_cmp(&q.0));
                // nonzero 绕组：winding 0↔非0 转换处开/闭填充区间。
                let (mut winding, mut fill_from) = (0i32, None);
                for &(x, dir) in &xs {
                    let prev = winding;
                    winding += dir;
                    if prev == 0 && winding != 0 {
                        fill_from = Some(x);
                    } else if prev != 0 && winding == 0 {
                        if let Some(a) = fill_from.take() {
                            add_span(&mut cover, x_lo, a, x);
                        }
                    }
                }
            }
            // 覆盖率 → alpha 合成（预乘 source-over；底不透明故 alpha 恒 255）。
            let (cr, cg, cb) = color;
            for (i, &c) in cover.iter().enumerate() {
                if c <= 0.0 {
                    continue;
                }
                let a = (c * 0.25).min(1.0);
                let idx = (py as usize * w + x_lo as usize + i) * 4;
                let d = &mut self.data[idx..idx + 4];
                for (dst, src) in d.iter_mut().zip([cr, cg, cb]) {
                    *dst = (src as f32 * a + *dst as f32 * (1.0 - a)).round() as u8;
                }
                d[3] = 255;
            }
        }
    }

    /// 某像素是否非白（测试用；alpha=0 视为白）。
    pub fn pixel_nonwhite(&self, x: u32, y: u32) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        let idx = (y as usize * self.width as usize + x as usize) * 4;
        let p = &self.data[idx..idx + 4];
        p[3] == 0 || (p[0] < 250 && p[1] < 250 && p[2] < 250)
    }
}

/// sp → 像素换算（`1pt = 65536sp`，`px = pt × dpi / 72`）。
///
/// 中间量走 f64：i64 sp 上限 ~9.2e18 仍在 f64 精度量级，dpi 合法范围
/// （~1..=2400）内误差远小于 1px。
pub fn sp_to_px(sp: i64, dpi: f64) -> f64 {
    sp as f64 / 65_536.0 * dpi / 72.0
}

/// 将填充区间 `[a, b)`（页面绝对坐标）的水平覆盖长度累加进行覆盖表
/// （`cover` 下标 = 像素 x − `ox`；与像素列求交，越界部分自然丢弃）。
fn add_span(cover: &mut [f32], ox: u32, a: f32, b: f32) {
    let ox = ox as f32;
    let (a, b) = (a - ox, b - ox);
    // 部分有序显式比较：b ≤ a 或含 NaN（不可比）一律跳过。
    if b.partial_cmp(&a) != Some(std::cmp::Ordering::Greater) {
        return;
    }
    let i1 = (b.ceil().max(0.0)) as usize;
    let mut i = (a.floor().max(0.0)) as usize;
    while i < i1 && i < cover.len() {
        let l = a.max(i as f32);
        let r = b.min((i + 1) as f32);
        if r > l {
            cover[i] += r - l;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sp_to_px_matches_reference() {
        // 65536sp = 1pt = 1px@72dpi；1pt@300dpi ≈ 4.1667px。
        assert!((sp_to_px(65_536, 72.0) - 1.0).abs() < 1e-12);
        assert!((sp_to_px(65_536, 300.0) - 300.0 / 72.0).abs() < 1e-12);
        assert!((sp_to_px(65_536 * 10, 144.0) - 20.0).abs() < 1e-12);
    }

    #[test]
    fn fill_rect_clips_and_marks() {
        let mut pm = Pixmap::new(10, 10);
        pm.fill(255, 255, 255);
        pm.fill_rect(-3.0, -3.0, 6.0, 6.0, (0, 0, 0));
        assert!(pm.pixel_nonwhite(0, 0));
        assert!(!pm.pixel_nonwhite(3, 0));
    }

    #[test]
    fn pixmap_new_does_not_wrap_on_huge_dims() {
        // 溢出兜底：u32::MAX × u32::MAX × 4 在 32 位 usize 上回绕成 4，
        // 若不兜底就是"data 4 字节 / width 42 亿"→ 索引越界 panic → wasm trap。
        let pm = Pixmap::new(u32::MAX, u32::MAX);
        assert_eq!((pm.width(), pm.height()), (1, 1));
        assert_eq!(pm.data().len(), 4);
        // 退化尺寸下填充/索引仍然安全（不 panic）。
        let mut pm = Pixmap::new(u32::MAX, u32::MAX);
        pm.fill(255, 255, 255);
        pm.fill_rect(0.0, 0.0, 10.0, 10.0, (0, 0, 0));
        assert!(pm.pixel_nonwhite(0, 0));
    }

    #[test]
    fn fill_polygon_triangle_interior_only() {
        let mut pm = Pixmap::new(20, 20);
        pm.fill(255, 255, 255);
        // 直角三角形 (2,2)-(12,2)-(2,12)：内部有墨，外部白。
        pm.fill_polygon(&[vec![[2.0, 2.0], [12.0, 2.0], [2.0, 12.0]]], (0, 0, 0));
        assert!(pm.pixel_nonwhite(3, 3));
        assert!(pm.pixel_nonwhite(6, 4)); // 斜边内侧
        assert!(!pm.pixel_nonwhite(15, 15));
        assert!(!pm.pixel_nonwhite(1, 1)); // 顶点外
    }

    #[test]
    fn fill_polygon_nonzero_hole() {
        // 外方顺时针 + 内方逆时针（绕组相消）：中心为孔，环带有墨。
        let mut pm = Pixmap::new(20, 20);
        pm.fill(255, 255, 255);
        pm.fill_polygon(
            &[
                vec![[2.0, 2.0], [14.0, 2.0], [14.0, 14.0], [2.0, 14.0]],
                vec![[6.0, 6.0], [6.0, 10.0], [10.0, 10.0], [10.0, 6.0]],
            ],
            (0, 0, 0),
        );
        assert!(pm.pixel_nonwhite(4, 4)); // 环带内
        assert!(!pm.pixel_nonwhite(8, 8)); // 孔内（绕组 0）
    }

    #[test]
    fn fill_polygon_degenerate_inputs_noop() {
        let mut pm = Pixmap::new(8, 8);
        pm.fill(255, 255, 255);
        pm.fill_polygon(&[], (0, 0, 0)); // 空组
        pm.fill_polygon(&[vec![[1.0, 1.0], [2.0, 2.0]]], (0, 0, 0)); // 退化段
        pm.fill_polygon(&[vec![[f32::NAN, 0.0], [4.0, 0.0], [4.0, 4.0]]], (0, 0, 0)); // NaN
        assert!(!pm.pixel_nonwhite(3, 3));
    }
}
