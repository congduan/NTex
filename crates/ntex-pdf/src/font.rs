//! Helvetica 度量（Adobe core14 标准字体，无需嵌入）。
//!
//! 临时切片用：M3-4 TFM 解析后由真实字体表替换。
//! 宽度表为 Helvetica.afm 的 ASCII 32..=126 值（单位 1/1000 em）。

use ntex_core::register::{Glue, SP_PER_PT};
use ntex_layout::{FontId, MetricsFn, SpaceFn};

/// 默认字号（pt）。
pub const FONT_SIZE_PT: i64 = 10;

/// 行距（pt）：切片阶段行基线间距。
pub const LINE_ADVANCE_PT: f64 = 12.0;

/// 上伸部（cap height / ascender，1/1000 em）。
const ASCENDER: i64 = 718;
/// 下伸部（descender，1/1000 em）。
const DESCENDER: i64 = 207;

/// ASCII 32..=126 的 Helvetica 宽度（单位 1/1000 em）。
const WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 222, 333, 333, 389, 584, 278, 333, 278, 278, //  32..47
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, //  48..63
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, //  64..79
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, //  80..95
    222, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, //  96..111
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584, //    112..126
];

/// 字符宽度（1/1000 em）；范围外默认 556。
pub fn width_afm(charcode: u32) -> i64 {
    match charcode {
        32..=126 => i64::from(WIDTHS[(charcode - 32) as usize]),
        _ => 556,
    }
}

/// 字符宽度（sp，按 `FONT_SIZE_PT`）。
pub fn char_width_sp(charcode: u32) -> i64 {
    width_afm(charcode) * FONT_SIZE_PT * SP_PER_PT / 1000
}

/// 字符宽度（pt）。
pub fn char_width_pt(charcode: u32) -> f64 {
    width_afm(charcode) as f64 * FONT_SIZE_PT as f64 / 1000.0
}

/// 字符度量：宽 = Helvetica 宽度；高/深 = 上伸/下伸（切片近似，所有字符一致）。
pub fn helvetica_metrics(_font: FontId, charcode: u32) -> (i64, i64, i64) {
    (
        char_width_sp(charcode),
        ASCENDER * FONT_SIZE_PT * SP_PER_PT / 1000,
        DESCENDER * FONT_SIZE_PT * SP_PER_PT / 1000,
    )
}

/// 词间空白：space = 278/1000em，stretch = 1/2 space，shrink = 1/3 space。
pub fn helvetica_space(_font: FontId) -> Glue {
    let space = 278 * FONT_SIZE_PT * SP_PER_PT / 1000;
    Glue {
        width: space,
        stretch: space / 2,
        shrink: space / 3,
    }
}

/// 切片用度量/空白组合（M3-4 TFM 前）。
pub fn slice_metrics() -> (MetricsFn, SpaceFn) {
    (helvetica_metrics, helvetica_space)
}
