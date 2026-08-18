//! 寄存器文件与内部量（M1-10）。
//!
//! - `\count`：256 个整数
//! - `\dimen`：256 个 scaled point（1pt = 65536sp）
//! - `\skip`：256 个胶水（width + stretch + shrink）
//! - `\toks`：256 个 token 列表
//!
//! 单位换算与 `\the` 输出格式均**对照真实 pdfTeX 实测校准**（见函数注释），
//! 精确逐位一致性留待 M3（TRIP）验证。

use crate::macrodef::TokenArray;
use std::sync::Arc;

/// 寄存器个数（TeX 标准：0..=255）。
pub const REGISTER_COUNT: usize = 256;

/// 1 pt = 65536 sp。
pub const SP_PER_PT: i64 = 65_536;

/// 胶水：宽度 + 拉伸 + 收缩（单位 sp）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glue {
    pub width: i64,
    pub stretch: i64,
    pub shrink: i64,
}

impl Glue {
    pub const ZERO: Glue = Glue {
        width: 0,
        stretch: 0,
        shrink: 0,
    };
}

/// 单位 → sp 换算。
///
/// 实测 pdfTeX 输出：`\dimen0=1in\the\dimen0` → "72.26999pt" 等，
/// 反推存储值：pt=65536, sp=1, bp=65781, in=4736286, cm=1864679, mm=186467。
pub fn unit_to_sp(unit: &str) -> Option<i64> {
    match unit {
        "sp" => Some(1),
        "pt" => Some(SP_PER_PT),
        "bp" => Some(65_781),
        "in" => Some(4_736_286),
        "cm" => Some(1_864_679),
        "mm" => Some(186_467),
        _ => None,
    }
}

/// 寄存器类别（`\count`/`\dimen`/`\skip`/`\toks`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegKind {
    Count,
    Dimen,
    Skip,
    Toks,
}

/// 寄存器文件（固定 256 槽，索引越界由调用方保证）。
#[derive(Debug, Clone)]
pub struct Registers {
    counts: [i64; REGISTER_COUNT],
    dimens: [i64; REGISTER_COUNT],
    skips: [Glue; REGISTER_COUNT],
    toks: [TokenArray; REGISTER_COUNT],
}

impl Default for Registers {
    fn default() -> Self {
        Self::new()
    }
}

impl Registers {
    pub fn new() -> Self {
        Self {
            counts: [0; REGISTER_COUNT],
            dimens: [0; REGISTER_COUNT],
            skips: [Glue::ZERO; REGISTER_COUNT],
            toks: std::array::from_fn(|_| Arc::from([])),
        }
    }

    pub fn count(&self, idx: usize) -> i64 {
        self.counts[idx]
    }

    pub fn set_count(&mut self, idx: usize, v: i64) {
        self.counts[idx] = v;
    }

    pub fn dimen(&self, idx: usize) -> i64 {
        self.dimens[idx]
    }

    pub fn set_dimen(&mut self, idx: usize, v: i64) {
        self.dimens[idx] = v;
    }

    pub fn skip(&self, idx: usize) -> Glue {
        self.skips[idx]
    }

    pub fn set_skip(&mut self, idx: usize, v: Glue) {
        self.skips[idx] = v;
    }

    pub fn toks(&self, idx: usize) -> TokenArray {
        self.toks[idx].clone()
    }

    pub fn set_toks(&mut self, idx: usize, v: TokenArray) {
        self.toks[idx] = v;
    }
}

/// 整数 → `\the` 输出（十进制）。
pub fn format_count(v: i64) -> String {
    v.to_string()
}

/// scaled → pt 字符串（对照 pdfTeX 的 print_scaled）：
/// 5 位小数、末位四舍五入、去尾零、至少保留一位小数。
///
/// 实测样例：1pt→"1.0"、2.5pt→"2.5"、-3.125pt→"-3.125"、1in→"72.26999"、1sp→"0.00002"。
pub fn format_dimen(scaled: i64) -> String {
    let mut out = String::new();
    if scaled < 0 {
        out.push('-');
    }
    let mag = scaled.unsigned_abs();
    let n = mag / SP_PER_PT as u64;
    let frac = (mag % SP_PER_PT as u64) as i64;
    out.push_str(&n.to_string());
    out.push('.');
    // round(frac * 100000 / 65536)：用 (frac*200000 + 65536) / 131072 实现四舍五入
    let digits = ((frac as i128 * 200_000 + 65_536) / 131_072) as i64;
    let mut frac_str = format!("{digits:05}");
    while frac_str.ends_with('0') && frac_str.len() > 1 {
        frac_str.pop();
    }
    out.push_str(&frac_str);
    out
}

/// 胶水 → `\the` 输出（"1.0pt plus 2.0pt minus 0.5pt"，零部分省略）。
pub fn format_glue(g: Glue) -> String {
    let mut out = format!("{}pt", format_dimen(g.width));
    if g.stretch != 0 {
        out.push_str(&format!(" plus {}pt", format_dimen(g.stretch)));
    }
    if g.shrink != 0 {
        out.push_str(&format!(" minus {}pt", format_dimen(g.shrink)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_conversions_match_tex() {
        assert_eq!(unit_to_sp("pt"), Some(65_536));
        assert_eq!(unit_to_sp("sp"), Some(1));
        assert_eq!(unit_to_sp("bp"), Some(65_781));
        assert_eq!(unit_to_sp("in"), Some(4_736_286));
        assert_eq!(unit_to_sp("cm"), Some(1_864_679));
        assert_eq!(unit_to_sp("mm"), Some(186_467));
        assert_eq!(unit_to_sp("em"), None);
    }

    #[test]
    fn dimen_formatting_matches_tex() {
        assert_eq!(format_dimen(65_536), "1.0");
        assert_eq!(format_dimen(163_840), "2.5");
        assert_eq!(format_dimen(-204_800), "-3.125");
        assert_eq!(format_dimen(4_736_286), "72.26999");
        assert_eq!(format_dimen(1), "0.00002");
        assert_eq!(format_dimen(0), "0.0");
    }

    #[test]
    fn glue_formatting_matches_tex() {
        let g = Glue {
            width: 65_536,
            stretch: 131_072,
            shrink: 32_768,
        };
        assert_eq!(format_glue(g), "1.0pt plus 2.0pt minus 0.5pt");
        assert_eq!(format_glue(Glue::ZERO), "0.0pt");
    }

    #[test]
    fn registers_default_zeroed() {
        let r = Registers::new();
        assert_eq!(r.count(0), 0);
        assert_eq!(r.dimen(3), 0);
        assert_eq!(r.skip(5), Glue::ZERO);
        assert!(r.toks(7).is_empty());
    }

    #[test]
    fn registers_set_get() {
        let mut r = Registers::new();
        r.set_count(1, 42);
        r.set_dimen(2, 65_536);
        r.set_skip(
            3,
            Glue {
                width: 1,
                stretch: 2,
                shrink: 3,
            },
        );
        assert_eq!(r.count(1), 42);
        assert_eq!(r.dimen(2), 65_536);
        assert_eq!(
            r.skip(3),
            Glue {
                width: 1,
                stretch: 2,
                shrink: 3
            }
        );
    }
}
