//! TeX 的 16 种 catcode 与默认查表（RFC-1 M1-4：catcode 在 token 生成时固化）。
//!
//! catcode 决定字节如何被切分为 token：控制符（0）、组定界（1/2）、数学切换（3）、
//! 对齐 tab（4）、行尾（5）、参数符（6）、上下标（7/8）、忽略（9）、空格（10）、
//! 字母（11）、其他（12）、active（13）、注释（14）、非法（15）。

/// catcode 值 0..=15。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Catcode {
    Escape = 0,
    BeginGroup = 1,
    EndGroup = 2,
    MathShift = 3,
    AlignmentTab = 4,
    EndOfLine = 5,
    Parameter = 6,
    Superscript = 7,
    Subscript = 8,
    Ignored = 9,
    Space = 10,
    Letter = 11,
    Other = 12,
    Active = 13,
    Comment = 14,
    Invalid = 15,
}

impl Catcode {
    /// 从原始值（0..=15）构造。
    pub const fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Escape,
            1 => Self::BeginGroup,
            2 => Self::EndGroup,
            3 => Self::MathShift,
            4 => Self::AlignmentTab,
            5 => Self::EndOfLine,
            6 => Self::Parameter,
            7 => Self::Superscript,
            8 => Self::Subscript,
            9 => Self::Ignored,
            10 => Self::Space,
            11 => Self::Letter,
            12 => Self::Other,
            13 => Self::Active,
            14 => Self::Comment,
            15 => Self::Invalid,
            _ => return None,
        })
    }

    /// 原始值。
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn is_letter(self) -> bool {
        matches!(self, Self::Letter)
    }

    pub const fn is_space(self) -> bool {
        matches!(self, Self::Space)
    }

    pub const fn is_end_of_line(self) -> bool {
        matches!(self, Self::EndOfLine)
    }
}

/// 8-bit 字节 → catcode 查表（M1 为单表；快照化见 M6+）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatcodeTable([u8; 256]);

impl CatcodeTable {
    /// plain TeX 默认表。
    pub fn new() -> Self {
        let mut t = [Catcode::Other as u8; 256];
        // 空白与行尾
        t[0x09] = Catcode::Space as u8; // tab
        t[0x20] = Catcode::Space as u8; // space
        t[0x0A] = Catcode::EndOfLine as u8; // LF
        t[0x0D] = Catcode::EndOfLine as u8; // CR
                                            // 特殊字符
        t[0x5C] = Catcode::Escape as u8; // '\'
        t[0x7B] = Catcode::BeginGroup as u8; // '{'
        t[0x7D] = Catcode::EndGroup as u8; // '}'
        t[0x24] = Catcode::MathShift as u8; // '$'
        t[0x26] = Catcode::AlignmentTab as u8; // '&'
        t[0x23] = Catcode::Parameter as u8; // '#'
        t[0x5E] = Catcode::Superscript as u8; // '^'
        t[0x5F] = Catcode::Subscript as u8; // '_'
        t[0x25] = Catcode::Comment as u8; // '%'
        t[0x7E] = Catcode::Active as u8; // '~'
        t[0x7F] = Catcode::Invalid as u8; // DEL
                                          // 字母
        for b in b'A'..=b'Z' {
            t[b as usize] = Catcode::Letter as u8;
        }
        for b in b'a'..=b'z' {
            t[b as usize] = Catcode::Letter as u8;
        }
        Self(t)
    }

    /// INITEX（iniTeX / 格式构建态）初始表：tex.web §1273 默认表（含一处引擎偏差）。
    ///
    /// 除 `\`=0、`%`=14、空格=10、**LF=5**、CR=5、DEL=15、字母=11、NUL=9 外
    /// **全部 12**（`^^I` 也是 12——INITEX 不给 tab 任何特殊待遇）。plain 表
    /// （[`Self::new`]）是 plain.tex 装载后的产物，差异（`{`=1 `~`=13 `_`=8 等）
    /// 正是 latex.ltx L98 `\ifnum\catcode`\{=1` 判别"是否已预载格式"的依据。
    ///
    /// **偏差说明**：tex.web 里 INITEX 的 LF 是 12（行结束符由读取层剥掉、再补一个
    /// `\endlinechar`=13/CR）。本引擎的扫描器是字节流直读（`input.rs::scan_token`），
    /// 不剥行尾字节——原始 LF 就是行尾符，`Comment` 跳行、空行→`\par` 都靠
    /// catcode 5 找行尾。若按 tex.web 原样给 LF=12，第一条注释会一路吞到 EOF。
    /// 故此处 LF 与 CR 同为 5（等价于"行尾符必有 catcode 5"的引擎行模型）。
    pub fn initex() -> Self {
        let mut t = [Catcode::Other as u8; 256];
        t[0x00] = Catcode::Ignored as u8; // NUL
        t[0x0A] = Catcode::EndOfLine as u8; // LF（引擎行模型：行尾字节，见上偏差说明）
        t[0x0D] = Catcode::EndOfLine as u8; // CR
        t[0x20] = Catcode::Space as u8; // space
        t[0x25] = Catcode::Comment as u8; // '%'
        t[0x5C] = Catcode::Escape as u8; // '\'
        t[0x7F] = Catcode::Invalid as u8; // DEL
        for b in b'A'..=b'Z' {
            t[b as usize] = Catcode::Letter as u8;
        }
        for b in b'a'..=b'z' {
            t[b as usize] = Catcode::Letter as u8;
        }
        Self(t)
    }

    /// 查询某字节的 catcode。
    pub fn get(&self, byte: u8) -> Catcode {
        Catcode::from_u8(self.0[byte as usize]).expect("catcode 表只允许 0..=15")
    }

    /// 查询某 Unicode 码位的 catcode（M9 中文刀 2：UTF-8 输入模式专用）。
    ///
    /// ≤255 走 8-bit 表（与字节模式同源，ASCII 行为不变）；>255 按 XeTeX
    /// 惯例默认 **letter**（CJK/扩展文字直接可排版）。8-bit 表保持 `[u8; 256]`
    /// 不动——`\catcode` 对 >255 码位的赋值扩展留待后续刀（A5 建议方向）。
    pub fn get_codepoint(&self, cp: u32) -> Catcode {
        if cp <= 0xFF {
            self.get(cp as u8)
        } else {
            Catcode::Letter
        }
    }

    /// 修改某字节的 catcode（`\catcode` 原语入口）。
    pub fn set(&mut self, byte: u8, cat: Catcode) {
        self.0[byte as usize] = cat.as_u8();
    }

    /// 原始字节表（`.fmt` 快照导出）。
    pub fn raw(&self) -> &[u8; 256] {
        &self.0
    }

    /// 从原始字节表重建（`.fmt` 快照导入）。
    pub fn from_raw(raw: [u8; 256]) -> Self {
        Self(raw)
    }
}

impl Default for CatcodeTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_table_specials() {
        let t = CatcodeTable::new();
        assert_eq!(t.get(b'\\'), Catcode::Escape);
        assert_eq!(t.get(b'{'), Catcode::BeginGroup);
        assert_eq!(t.get(b'}'), Catcode::EndGroup);
        assert_eq!(t.get(b'$'), Catcode::MathShift);
        assert_eq!(t.get(b'&'), Catcode::AlignmentTab);
        assert_eq!(t.get(b'#'), Catcode::Parameter);
        assert_eq!(t.get(b'^'), Catcode::Superscript);
        assert_eq!(t.get(b'_'), Catcode::Subscript);
        assert_eq!(t.get(b'%'), Catcode::Comment);
        assert_eq!(t.get(b'~'), Catcode::Active);
        assert_eq!(t.get(0x0A), Catcode::EndOfLine);
        assert_eq!(t.get(b' '), Catcode::Space);
        assert_eq!(t.get(0x7F), Catcode::Invalid);
    }

    #[test]
    fn default_table_letters_and_others() {
        let t = CatcodeTable::new();
        assert_eq!(t.get(b'a'), Catcode::Letter);
        assert_eq!(t.get(b'Z'), Catcode::Letter);
        assert_eq!(t.get(b'0'), Catcode::Other);
        assert_eq!(t.get(b'.'), Catcode::Other);
    }

    #[test]
    fn initex_table_is_tex_web_1273() {
        let t = CatcodeTable::initex();
        assert_eq!(t.get(b'\\'), Catcode::Escape);
        assert_eq!(t.get(b'%'), Catcode::Comment);
        assert_eq!(t.get(b' '), Catcode::Space);
        assert_eq!(t.get(0x0D), Catcode::EndOfLine);
        assert_eq!(t.get(0x7F), Catcode::Invalid);
        assert_eq!(t.get(0x00), Catcode::Ignored);
        assert_eq!(t.get(b'a'), Catcode::Letter);
        assert_eq!(t.get(b'Z'), Catcode::Letter);
        // INITEX 里这些都不是特殊字符（plain 表才是）；LF 例外——见
        // [`CatcodeTable::initex`] 的引擎行模型偏差说明。
        assert_eq!(t.get(b'\n'), Catcode::EndOfLine);
        for &b in b"{}$&#^_~\t" {
            assert_eq!(t.get(b), Catcode::Other, "byte 0x{b:02X}");
        }
    }

    #[test]
    fn set_changes_lookup() {
        let mut t = CatcodeTable::new();
        assert_eq!(t.get(b'_'), Catcode::Subscript);
        t.set(b'_', Catcode::Letter);
        assert_eq!(t.get(b'_'), Catcode::Letter);
    }

    #[test]
    fn catcode_round_trip() {
        for v in 0..=15u8 {
            let c = Catcode::from_u8(v).unwrap();
            assert_eq!(c.as_u8(), v);
        }
        assert!(Catcode::from_u8(16).is_none());
    }
}
