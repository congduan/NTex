//! 软光栅化与像素缓冲（纯 Rust，无第三方依赖）。
//!
//! 像素格式：RGBA8（预乘 alpha，与 tiny-skia `PremultipliedColorU8` 一致），
//! 行主序、每像素 4 字节。填充走整数覆盖盒，裁剪到页面边界；亚像素 AA
//! 属 M9 字形渲染范围。

/// 8 位 RGBA 像素缓冲（预乘 alpha）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pixmap {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Pixmap {
    /// 新建全透明缓冲（维度下限 1×1，避免退化尺寸）。
    pub fn new(width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        Self {
            width,
            height,
            data: vec![0; width as usize * height as usize * 4],
        }
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
        for px in self.data.chunks_exact_mut(4) {
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

    /// 填充水平线段（厚度 1px）。
    pub fn fill_hline(&mut self, y: f64, x0: f64, x1: f64, color: (u8, u8, u8)) {
        let (lo, hi) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        self.fill_rect(lo, y, hi - lo, 1.0, color);
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
}
