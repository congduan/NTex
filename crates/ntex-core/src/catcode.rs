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

/// 8-bit 字节 → catcode 查表 + >255 码位覆盖表（M9 中文刀 2/4）。
///
/// `bytes` 是 8-bit 主表（TRIP/ETRIP 口径，`[u8; 256]` 原样）；`unicode_cats`
/// 是 **>255 码位的赋值覆盖**（A5 全量收口，刀 4）：`\utfinputmode=1` 下
/// `\catcode`，=13 可把全角逗号设为 active，`get_codepoint` 查表优先于
/// 默认 letter。未覆盖的 >255 码位仍按 XeTeX 惯例默认 letter。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatcodeTable {
    bytes: [u8; 256],
    unicode_cats: std::collections::BTreeMap<u32, u8>,
}

impl CatcodeTable {
    /// plain TeX 默认表。
    pub fn new() -> Self {
        Self::from_bytes(Self::plain_bytes())
    }

    /// INITEX（iniTeX / 格式构建态）初始表：tex.web §1273 默认表。
    ///
    /// 除 `\`=0、`%`=14、空格=10、CR=5、DEL=15、字母=11、NUL=9 外
    /// **全部 12**（`^^I` 也是 12——INITEX 不给 tab 任何特殊待遇；LF=12 同）。
    /// plain 表（[`Self::new`]）是 plain.tex 装载后的产物，差异（`{`=1 `~`=13
    /// `_`=8 等）正是 latex.ltx L98 `\ifnum\catcode`\{=1` 判别"是否已预载格式"
    /// 的依据。
    ///
    /// **行模型说明**：tex.web 里行结束符由读取层剥掉、再按位置补一个
    /// `\endlinechar`=13/CR（INITEX 给它 cat 5）。本引擎的扫描器是字节流直读
    /// （`input.rs::scan_token`），物理行边界按**字节身份**（`b == b'\n'`）识别、
    /// 与 catcode(0x0A) 无关——该位置判定才是"行尾符"的载体。表项 catcode(0x0A)
    /// 保持 tex.web INITEX 的 12（other）：行中 `^^J` 解码产物是**普通 cat 12
    /// 字符 token**（可作宏定界符、进 `\message` 等，pdfTeX 对拍一致）；若把它
    /// 设成 5，`\__iow_wrap_fix_newline:w #1 ^^J #2 ^^J` 这类 chr(10) 定界符在
    /// 定义位变 `\par`、调用位被吞（expl3 iow_wrap 全线失配，fp 载入探针
    /// 3+2+2+3 错的根因）。
    pub fn initex() -> Self {
        let mut t = [Catcode::Other as u8; 256];
        t[0x00] = Catcode::Ignored as u8; // NUL
        t[0x0D] = Catcode::EndOfLine as u8; // CR（endlinechar=13 的 cat）
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
        Self::from_bytes(t)
    }

    /// plain TeX 默认 8-bit 表体（[`Self::new`] 的内容，独立成函数便于重建）。
    fn plain_bytes() -> [u8; 256] {
        let mut t = [Catcode::Other as u8; 256];
        // 空白与行尾（plain.tex 不改 chr(10)：INITEX 的 cat 12 原样继承；
        // 物理行尾由扫描器按字节身份判定，见 initex 的行模型说明）
        t[0x09] = Catcode::Space as u8; // tab
        t[0x20] = Catcode::Space as u8; // space
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
        t
    }

    /// 由 8-bit 表体构造（覆盖表为空）。
    fn from_bytes(bytes: [u8; 256]) -> Self {
        Self {
            bytes,
            unicode_cats: std::collections::BTreeMap::new(),
        }
    }

    /// 查询某字节的 catcode。
    pub fn get(&self, byte: u8) -> Catcode {
        Catcode::from_u8(self.bytes[byte as usize]).expect("catcode 表只允许 0..=15")
    }

    /// 查询某 Unicode 码位的 catcode（M9 中文刀 2：UTF-8 输入模式专用）。
    ///
    /// ≤255 走 8-bit 表（与字节模式同源，ASCII 行为不变）；>255 先查刀 4
    /// 的 `\catcode` 赋值覆盖表（`\utfinputmode=1` 下 `\catcode`，=13 可赋），
    /// 未覆盖按 XeTeX 惯例默认 **letter**（CJK/扩展文字直接可排版）。
    /// 8-bit 主表保持 `[u8; 256]` 原样——TRIP/ETRIP 口径不受影响。
    pub fn get_codepoint(&self, cp: u32) -> Catcode {
        if cp <= 0xFF {
            self.get(cp as u8)
        } else if let Some(&v) = self.unicode_cats.get(&cp) {
            Catcode::from_u8(v).expect("覆盖表只允许 0..=15")
        } else {
            Catcode::Letter
        }
    }

    /// 修改某字节的 catcode（`\catcode` 原语入口，≤255）。
    pub fn set(&mut self, byte: u8, cat: Catcode) {
        self.bytes[byte as usize] = cat.as_u8();
    }

    /// 设置 >255 码位的 catcode 覆盖（刀 4：`\catcode`，=13）。
    ///
    /// 仅 `\utfinputmode=1` 下可达（调用方 gate）；≤255 请走 [`Self::set`]。
    pub fn set_codepoint(&mut self, cp: u32, cat: Catcode) {
        self.unicode_cats.insert(cp, cat.as_u8());
    }

    /// 查询 >255 码位是否被显式赋值过（组回滚用；`None` = 默认 letter）。
    pub fn unicode_codepoint(&self, cp: u32) -> Option<Catcode> {
        self.unicode_cats
            .get(&cp)
            .and_then(|&v| Catcode::from_u8(v))
    }

    /// 撤销 >255 码位的覆盖（组回滚 / `\global` 恢复默认 letter）。
    pub fn remove_codepoint(&mut self, cp: u32) {
        self.unicode_cats.remove(&cp);
    }

    /// 覆盖表只读视图（`.fmt` 快照导出用；`(码位, catcode 值)` 升序）。
    pub fn unicode_overrides(&self) -> impl Iterator<Item = (u32, u8)> + '_ {
        self.unicode_cats.iter().map(|(&cp, &v)| (cp, v))
    }

    /// 原始字节表（`.fmt` 快照导出）。
    pub fn raw(&self) -> &[u8; 256] {
        &self.bytes
    }

    /// 从原始字节表重建（`.fmt` 快照导入；覆盖表为空，加载后需回填）。
    pub fn from_raw(raw: [u8; 256]) -> Self {
        Self::from_bytes(raw)
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
        // plain.tex 不改 chr(10)：INITEX 的 cat 12 原样继承（物理行尾由扫描器
        // 按字节身份判定；行中 ^^J 解码产物是普通 cat 12 字符 token）
        assert_eq!(t.get(0x0A), Catcode::Other);
        assert_eq!(t.get(0x0D), Catcode::EndOfLine);
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
        // INITEX 里这些都不是特殊字符（plain 表才是）；LF=12 同 tex.web §1273——
        // 行界由扫描器按字节身份判定（[`CatcodeTable::initex`] 行模型说明）。
        assert_eq!(t.get(b'\n'), Catcode::Other);
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

    #[test]
    fn unicode_overlay_assign_and_default() {
        let mut t = CatcodeTable::new();
        // 未覆盖：默认 letter（刀 2 XeTeX 惯例）
        assert_eq!(t.get_codepoint(0x4E2D), Catcode::Letter);
        assert_eq!(t.unicode_codepoint(0x4E2D), None);
        // 赋 active 后查表优先（刀 4：\catcode`，=13 的底层）
        t.set_codepoint(0xFF0C, Catcode::Active);
        assert_eq!(t.get_codepoint(0xFF0C), Catcode::Active);
        assert_eq!(t.unicode_codepoint(0xFF0C), Some(Catcode::Active));
        // 8-bit 主表不受覆盖表影响
        assert_eq!(t.get(b'a'), Catcode::Letter);
        // 撤销覆盖 → 回默认 letter（组回滚 / \global 语义）
        t.remove_codepoint(0xFF0C);
        assert_eq!(t.get_codepoint(0xFF0C), Catcode::Letter);
    }

    #[test]
    fn unicode_overlay_does_not_leak_into_byte_range() {
        let mut t = CatcodeTable::new();
        t.set_codepoint(0x4E2D, Catcode::Active);
        // ≤255 恒走 8-bit 表，覆盖表不拦路（TRIP 口径）
        assert_eq!(t.get_codepoint(0x41), Catcode::Letter);
        assert_eq!(t.get_codepoint(0x7F), Catcode::Invalid);
    }

    #[test]
    fn unicode_override_iteration_is_sorted() {
        let mut t = CatcodeTable::new();
        t.set_codepoint(0xFF0C, Catcode::Active);
        t.set_codepoint(0x4E2D, Catcode::Letter);
        let v: Vec<(u32, u8)> = t.unicode_overrides().collect();
        assert_eq!(v, vec![(0x4E2D, 11), (0xFF0C, 13)]);
    }
}
