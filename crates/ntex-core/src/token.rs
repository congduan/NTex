//! Token：8 字节 tagged union（RFC-1 §3）。
//!
//! 位布局（u64，tag 在高 4 bit，载荷 60 bit）：
//!
//! | tag | 载荷 | 说明 |
//! |---|---|---|
//! | 0 `Char` | catcode(4b)<<21 | charcode(21b) | 25 bit，压缩进低 60 bit |
//! | 1 `ControlSeq` | csid(32b) | InternTable 下标 |
//! | 2 `MacroParam` | num(4b) | `#1..#9` |
//! | 3 `EndGroup` | — | 内部控制标记 |
//!
//! 设计要点（RFC-1）：token 无堆指针、可自由复制共享；控制序列的"定义"不放在
//! token 里（查 eqtb 槽）；catcode 在 token 生成时固化。

use std::fmt;

use crate::catcode::Catcode;
use crate::intern::InternTable;

/// charcode 上限（21 bit，覆盖 Unicode 全量）。
pub const MAX_CHARCODE: u32 = (1 << 21) - 1;

/// active 字符槽名前缀（input.rs Active 臂 intern 时加在原字符前，保证
/// active 槽与同名普通 cs 槽隔离）。取回原字符一律经 [`Token::active_charcode`]。
pub const ACTIVE_SLOT_PREFIX: &str = "\u{0}A";

const TAG_SHIFT: u32 = 60;
const CHAR_SHIFT: u32 = 21;
const CHARCODE_MASK: u64 = MAX_CHARCODE as u64;
const CSID_MASK: u64 = u32::MAX as u64;
const PARAM_MASK: u64 = 0xF;
/// ControlSeq 载荷的 active 字符标志（csid 32 bit 之上的首个空位）。
///
/// tex.web 的 active char token = `cs_token_flag + eqtb active 区槽位`
/// （active 区在 single_base 之前），与命名 cs 同属 cs token 但**结构可分**
/// ——`\lowercase`/`\uppercase`（shift_case，tex.web §1288）按此区分：
/// active 字符施 uccode/lccode 表、同名单字符 cs 不施。本引擎 csid 只是
/// InternTable 下标、无 active 区，用标志位补回表示层差异（token 表示层
/// 另一刀，2026-09-08；仅生成位 input.rs 与消费位 case_convert_tokens
/// 使用，`\if`/`\ifx`/eqtb 查找仍只看 csid）。
const CS_ACTIVE_FLAG: u64 = 1 << 32;

/// token 类别（tag 值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TokenKind {
    Char = 0,
    ControlSeq = 1,
    MacroParam = 2,
    EndGroup = 3,
    /// 对齐 v 模板尾哨兵（tex.web `end_template_token`，L15497：扫描到的 v_j
    /// 模板以显式 `\endtemplate` 收尾）。只在 `InputFrame::AlignV` 帧内出现，
    /// 永不出自用户输入。
    EndTemplate = 4,
}

/// 8 字节 token。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Token(u64);

impl Token {
    /// 字符 token（catcode 固化在生成时）。
    pub const fn char(cat: Catcode, ch: u32) -> Self {
        debug_assert!(ch <= MAX_CHARCODE, "charcode 越界");
        Self(((cat as u8 as u64) << CHAR_SHIFT) | (ch as u64 & CHARCODE_MASK))
    }

    /// 控制序列 token（csid = InternTable 下标）。
    pub const fn control_sequence(csid: u32) -> Self {
        Self(((TokenKind::ControlSeq as u64) << TAG_SHIFT) | (csid as u64 & CSID_MASK))
    }

    /// active 字符 token（cat 13 → 同名 cs + active 标志；tex.web
    /// `cs_token_flag + eqtb active 区槽位` 的引擎等价表示）。
    pub const fn active_sequence(csid: u32) -> Self {
        Self(
            ((TokenKind::ControlSeq as u64) << TAG_SHIFT)
                | CS_ACTIVE_FLAG
                | (csid as u64 & CSID_MASK),
        )
    }

    /// 宏参数 token `#n`（n ∈ 1..=9）。
    pub const fn macro_param(n: u8) -> Self {
        debug_assert!(n <= 9, "宏参数号必须 <= 9");
        Self(((TokenKind::MacroParam as u64) << TAG_SHIFT) | (n as u64 & PARAM_MASK))
    }

    /// 内部控制标记 token。
    pub const fn end_group() -> Self {
        Self((TokenKind::EndGroup as u64) << TAG_SHIFT)
    }

    /// 对齐 v 模板尾哨兵（tex.web `end_template_token`）。
    ///
    /// v_j 模板末尾放一枚哨兵，使 `\hskip\tabcolsep` 之后的 plus/minus 关键字
    /// 前瞻（scan_glue 内部 `scan_keyword`）读到的是**可退回的 token**——否则
    /// 帧耗尽在探针里就触发 fin_col，尾段胶水落进下一单元（tex.web 用
    /// expand L7785 的 `end_template→frozen_endv` 换形达成同一效果：endv 只在
    /// 真正到达主循环时收列）。
    pub const fn end_template() -> Self {
        Self((TokenKind::EndTemplate as u64) << TAG_SHIFT)
    }

    /// 是否 v 模板尾哨兵。
    pub const fn is_end_template(self) -> bool {
        matches!(self.kind(), TokenKind::EndTemplate)
    }

    /// 由原始 u64 构造（供测试与序列化）。
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// 原始 u64。
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// token 类别。
    pub const fn kind(self) -> TokenKind {
        match (self.0 >> TAG_SHIFT) as u8 {
            0 => TokenKind::Char,
            1 => TokenKind::ControlSeq,
            2 => TokenKind::MacroParam,
            4 => TokenKind::EndTemplate,
            _ => TokenKind::EndGroup,
        }
    }

    /// 字符 token 的 catcode（仅 `Char`）。
    pub const fn catcode(self) -> Option<Catcode> {
        match self.kind() {
            TokenKind::Char => Catcode::from_u8(((self.0 >> CHAR_SHIFT) & 0xF) as u8),
            _ => None,
        }
    }

    /// 字符 token 的字符码（仅 `Char`）。
    pub const fn charcode(self) -> Option<u32> {
        match self.kind() {
            TokenKind::Char => Some((self.0 & CHARCODE_MASK) as u32),
            _ => None,
        }
    }

    /// 控制序列 token 的 csid（仅 `ControlSeq`）。
    pub const fn csid(self) -> Option<u32> {
        match self.kind() {
            TokenKind::ControlSeq => Some((self.0 & CSID_MASK) as u32),
            _ => None,
        }
    }

    /// 是否 active 字符 token（`ControlSeq` + active 标志；见 [`Self::active_sequence`]）。
    pub const fn is_active(self) -> bool {
        matches!(self.kind(), TokenKind::ControlSeq) && (self.0 & CS_ACTIVE_FLAG) != 0
    }

    /// active 字符 token 的原字符码。active 槽名带 `\u{0}A` 前缀（input.rs
    /// Active 臂，槽隔离），原字符在前缀之后——所有「从 token 取回字符」的
    /// 消费位（`\string`/`\detokenize`/`\show`/`\write` 构串）必须经此取码，
    /// 直接取 `name.chars().next()` 会拿到 NUL（2026-09-19 beamer 消费器
    /// 第二层活锁：`\xdef\beamer@masterdecode` 的 `\string|` 产出 NUL，
    /// `all|stop:0|` 变 `all\0stop\00\0`，decode 永不命中 `|` 定界 →
    /// `\beamer@doifnotinframe` 不被改写 → startcomment 逐行吞文件）。
    pub fn active_charcode(&self, intern: &InternTable) -> Option<char> {
        if !self.is_active() {
            return None;
        }
        intern
            .name(self.csid()?)
            .strip_prefix(ACTIVE_SLOT_PREFIX)
            .and_then(|s| s.chars().next())
    }

    /// 宏参数号（仅 `MacroParam`）。
    pub const fn param_number(self) -> Option<u8> {
        match self.kind() {
            TokenKind::MacroParam => Some((self.0 & PARAM_MASK) as u8),
            _ => None,
        }
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind() {
            TokenKind::Char => write!(
                f,
                "Char(cat={:?},ch={})",
                self.catcode().unwrap(),
                self.charcode().unwrap()
            ),
            TokenKind::ControlSeq => write!(f, "CS({})", self.csid().unwrap()),
            TokenKind::MacroParam => write!(f, "Param({})", self.param_number().unwrap()),
            TokenKind::EndGroup => write!(f, "EndGroup"),
            TokenKind::EndTemplate => write!(f, "EndTemplate"),
        }
    }
}

/// 类 TeX 的 `\meaning`/`\show` 风格描述（M1 基础版，后续随原语完善）。
pub fn meaning(tok: Token, intern: &InternTable) -> String {
    match tok.kind() {
        TokenKind::Char => {
            let cat = tok.catcode().unwrap();
            let ch = tok.charcode().unwrap();
            let chr = char::from_u32(ch).unwrap_or('\u{FFFD}');
            match cat {
                Catcode::Space => "blank space  ".to_owned(),
                Catcode::Letter => format!("the letter {chr}"),
                Catcode::Other => format!("the character {chr}"),
                Catcode::BeginGroup => "begin-group character {".to_owned(),
                Catcode::EndGroup => "end-group character }".to_owned(),
                Catcode::Escape => "escape character \\".to_owned(),
                Catcode::Parameter => "parameter character #".to_owned(),
                Catcode::Active => format!("active character {chr}"),
                Catcode::MathShift => "math shift character $".to_owned(),
                _ => format!("char(cat={cat:?},ch={ch})"),
            }
        }
        TokenKind::ControlSeq => format!("\\{}", intern.name(tok.csid().unwrap())),
        TokenKind::MacroParam => format!("#{}", tok.param_number().unwrap()),
        TokenKind::EndGroup => "end-group character }".to_owned(),
        TokenKind::EndTemplate => "end-template".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_round_trip() {
        let t = Token::char(Catcode::Letter, b'A' as u32);
        assert_eq!(t.kind(), TokenKind::Char);
        assert_eq!(t.catcode(), Some(Catcode::Letter));
        assert_eq!(t.charcode(), Some(b'A' as u32));
    }

    #[test]
    fn char_layout_is_compact() {
        // 25 bit 载荷应落在低 25 位内（tag 0）。
        let t = Token::char(Catcode::Space, b' ' as u32);
        assert_eq!(t.raw() >> 60, 0);
        assert!(t.raw() <= (MAX_CHARCODE as u64) << CHAR_SHIFT);
    }

    #[test]
    fn max_charcode_fits() {
        let t = Token::char(Catcode::Other, MAX_CHARCODE);
        assert_eq!(t.charcode(), Some(MAX_CHARCODE));
    }

    #[test]
    fn control_seq_round_trip() {
        let t = Token::control_sequence(42);
        assert_eq!(t.kind(), TokenKind::ControlSeq);
        assert_eq!(t.csid(), Some(42));
        assert_eq!(t.catcode(), None);
        assert_eq!(t.charcode(), None);
    }

    #[test]
    fn macro_param_round_trip() {
        let t = Token::macro_param(9);
        assert_eq!(t.kind(), TokenKind::MacroParam);
        assert_eq!(t.param_number(), Some(9));
    }

    #[test]
    fn end_group_kind() {
        assert_eq!(Token::end_group().kind(), TokenKind::EndGroup);
    }

    #[test]
    fn from_raw_preserves_bits() {
        let t = Token::control_sequence(7);
        assert_eq!(Token::from_raw(t.raw()), t);
    }

    #[test]
    fn meaning_basic() {
        let mut intern = InternTable::new();
        let csid = intern.intern("foo");
        assert_eq!(meaning(Token::control_sequence(csid), &intern), "\\foo");
        assert_eq!(
            meaning(Token::char(Catcode::Letter, b'A' as u32), &intern),
            "the letter A"
        );
        assert_eq!(meaning(Token::macro_param(2), &intern), "#2");
    }
}
