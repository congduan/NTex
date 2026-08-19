//! eqtb（等价表）槽：控制序列的运行时含义（RFC-1 §5）。
//!
//! 设计要点：
//! - token 只含 csid，等价关系一律查 eqtb 槽；
//! - 宏定义槽版本化（[`Versioned`]），供 M5 增量依赖追踪；
//! - `\let` 走 [`EqSlot::Alias`] 间接（不复制宏体）。

use std::sync::Arc;

use crate::catcode::Catcode;
use crate::macrodef::MacroDef;
use crate::version::{Version, Versioned};

/// M1 原语集（随里程碑扩充）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Primitive {
    Def = 1,
    Edef,
    Gdef,
    Let,
    Relax,
    Expandafter,
    Noexpand,
    Catcode,
    End,
    // M1-7 扫描顺序原语
    Futurelet,
    Aftergroup,
    Afterassignment,
    // M1-9 条件原语
    If,
    IfCat,
    IfNum,
    IfDim,
    IfX,
    IfOdd,
    IfCase,
    IfTrue,
    IfFalse,
    Else,
    Fi,
    Or,
    // M1-10 寄存器
    Count,
    Dimen,
    Skip,
    Toks,
    The,
    Global,
    // M1-11 组
    BeginGroup,
    EndGroup,
    // M3-2 排版原语：参数扫描在 VM 侧（结果经 sink 输出）；
    // hbox/vbox/vtop/par 直通 sink 由排版器解释。
    HBox,
    VBox,
    VTop,
    HSkip,
    VSkip,
    Kern,
    Penalty,
    HRule,
    VRule,
    Par,
    // M3-2-2 内部参数与段落
    ParIndent,
    BaselineSkip,
    LineSkip,
    LineSkipLimit,
    Indent,
    NoIndent,
    // M3-3 折行参数
    HSize,
    Tolerance,
    // M3-4 字体：\font<cs>=<名字>[at/scaled]
    Font,
    // M3-5 输出：\shipout<box>（直通 sink，由排版器封装页面）
    ShipOut,
    // M3-5 断页参数：\vsize/\topskip/\maxdepth/\parskip
    VSize,
    TopSkip,
    MaxDepth,
    ParSkip,
    // M3-4 词间距：\sfcode<字符>=<spacefactor>
    SfCode,
    // M3-5-3 输出例程：\output=<general text>（token 列表存储）；\box<n>（盒子寄存器）
    Output,
    Box,
    // M3 收尾（RFC-3）：VFS 副作用原语
    Input,
    OpenIn,
    CloseIn,
    NewRead,
    Read,
    NewWrite,
    OpenOut,
    CloseOut,
    Write,
    Immediate,
    // M4-2 数学原语（直通 sink，由排版器解释）
    DisplayStyle,
    TextStyle,
    ScriptStyle,
    ScriptScriptStyle,
    Over,
    Atop,
    Left,
    Right,
    Sqrt,
    MathOrd,
    MathBin,
    MathOp,
    MathRel,
    MathOpen,
    MathClose,
    MathPunct,
    MathInner,
    Nonscript,
    // M4-5 e-TeX 展开扩展
    Protected,
    IfDefined,
    IfCsname,
    Unless,
    NumExpr,
    Detokenize,
    Unexpanded,
    ETeXVersion,
    ETeXRevision,
    // M4-3 数学字体族
    TextFont,
    ScriptFont,
    ScriptScriptFont,
    // M4-6 断字：\patterns 模式表
    Patterns,
}

impl Primitive {
    /// 参与展开的原语（其余为不可展开，直接执行）。
    pub fn is_expandable(self) -> bool {
        matches!(self, Self::Expandafter | Self::Noexpand | Self::The)
    }

    /// 原始值（`.fmt` 快照序列化；变体自 1 起连续）。
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// 从原始值恢复（`.fmt` 快照反序列化）。
    pub const fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::Def,
            2 => Self::Edef,
            3 => Self::Gdef,
            4 => Self::Let,
            5 => Self::Relax,
            6 => Self::Expandafter,
            7 => Self::Noexpand,
            8 => Self::Catcode,
            9 => Self::End,
            10 => Self::Futurelet,
            11 => Self::Aftergroup,
            12 => Self::Afterassignment,
            13 => Self::If,
            14 => Self::IfCat,
            15 => Self::IfNum,
            16 => Self::IfDim,
            17 => Self::IfX,
            18 => Self::IfOdd,
            19 => Self::IfCase,
            20 => Self::IfTrue,
            21 => Self::IfFalse,
            22 => Self::Else,
            23 => Self::Fi,
            24 => Self::Or,
            25 => Self::Count,
            26 => Self::Dimen,
            27 => Self::Skip,
            28 => Self::Toks,
            29 => Self::The,
            30 => Self::Global,
            31 => Self::BeginGroup,
            32 => Self::EndGroup,
            33 => Self::HBox,
            34 => Self::VBox,
            35 => Self::VTop,
            36 => Self::HSkip,
            37 => Self::VSkip,
            38 => Self::Kern,
            39 => Self::Penalty,
            40 => Self::HRule,
            41 => Self::VRule,
            42 => Self::Par,
            43 => Self::ParIndent,
            44 => Self::BaselineSkip,
            45 => Self::LineSkip,
            46 => Self::LineSkipLimit,
            47 => Self::Indent,
            48 => Self::NoIndent,
            49 => Self::HSize,
            50 => Self::Tolerance,
            51 => Self::Font,
            52 => Self::ShipOut,
            53 => Self::VSize,
            54 => Self::TopSkip,
            55 => Self::MaxDepth,
            56 => Self::ParSkip,
            57 => Self::SfCode,
            58 => Self::Output,
            59 => Self::Box,
            60 => Self::Input,
            61 => Self::OpenIn,
            62 => Self::CloseIn,
            63 => Self::NewRead,
            64 => Self::Read,
            65 => Self::NewWrite,
            66 => Self::OpenOut,
            67 => Self::CloseOut,
            68 => Self::Write,
            69 => Self::Immediate,
            70 => Self::DisplayStyle,
            71 => Self::TextStyle,
            72 => Self::ScriptStyle,
            73 => Self::ScriptScriptStyle,
            74 => Self::Over,
            75 => Self::Atop,
            76 => Self::Left,
            77 => Self::Right,
            78 => Self::Sqrt,
            79 => Self::MathOrd,
            80 => Self::MathBin,
            81 => Self::MathOp,
            82 => Self::MathRel,
            83 => Self::MathOpen,
            84 => Self::MathClose,
            85 => Self::MathPunct,
            86 => Self::MathInner,
            87 => Self::Nonscript,
            88 => Self::Protected,
            89 => Self::IfDefined,
            90 => Self::IfCsname,
            91 => Self::Unless,
            92 => Self::NumExpr,
            93 => Self::Detokenize,
            94 => Self::Unexpanded,
            95 => Self::ETeXVersion,
            96 => Self::ETeXRevision,
            97 => Self::TextFont,
            98 => Self::ScriptFont,
            99 => Self::ScriptScriptFont,
            100 => Self::Patterns,
            _ => return None,
        })
    }
}

/// 控制序列的等价槽。
#[derive(Debug, Clone, PartialEq)]
pub enum EqSlot {
    /// 未定义。
    Undefined,
    /// 宏定义（版本化）。
    Macro(Versioned<Arc<MacroDef>>),
    /// 内建原语。
    Primitive(Primitive),
    /// `\let` 别名（cs → cs）。
    Alias(u32),
    /// `\let` 到字符（cs 等价于某字符 token，保留其 catcode）。
    Char { catcode: Catcode, charcode: u32 },
    /// 字体选择器（M3-4）：`\font\cs=cmr10` 定义；执行时设置当前字体。
    Font(u32),
    /// 寄存器引用（M3 收尾）：`\newcount\cs` 等把 cs 绑定到某寄存器槽。
    Register(crate::register::RegKind, usize),
    /// 流引用（RFC-3）：`\newwrite`/`\newread` 分配的流号（独立于 count 槽）。
    Stream(StreamKind, usize),
}

/// 流类别（RFC-3）：读流（`\openin`/`\read`）与写流（`\openout`/`\write`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Read,
    Write,
}

/// eqtb：按 csid 索引的等价槽数组。
#[derive(Debug, Clone, Default)]
pub struct Eqtb {
    slots: Vec<EqSlot>,
}

impl Eqtb {
    pub fn new() -> Self {
        Self::default()
    }

    /// 确保 csid 槽存在（自动扩展为 `Undefined`）。
    fn ensure(&mut self, csid: u32) {
        while (self.slots.len() as u32) <= csid {
            self.slots.push(EqSlot::Undefined);
        }
    }

    /// 读取槽（越界视为未定义）。
    pub fn slot(&self, csid: u32) -> &EqSlot {
        self.slots.get(csid as usize).unwrap_or(&EqSlot::Undefined)
    }

    /// 可变槽（自动扩展）。
    pub fn slot_mut(&mut self, csid: u32) -> &mut EqSlot {
        self.ensure(csid);
        &mut self.slots[csid as usize]
    }

    /// 定义宏：已有宏槽则 bump 版本（保持版本单调），否则新建。
    pub fn define_macro(&mut self, csid: u32, def: MacroDef) {
        let slot = self.slot_mut(csid);
        match slot {
            EqSlot::Macro(v) => v.bump(Arc::new(def)),
            _ => *slot = EqSlot::Macro(Versioned::new(Arc::new(def))),
        }
    }

    /// 注册原语。
    pub fn set_primitive(&mut self, csid: u32, prim: Primitive) {
        *self.slot_mut(csid) = EqSlot::Primitive(prim);
    }

    /// `\let\cs\other`：cs → Alias(other)。
    pub fn alias(&mut self, csid: u32, target: u32) {
        *self.slot_mut(csid) = EqSlot::Alias(target);
    }

    /// `\let\cs=x`：cs 等价于字符 token。
    pub fn char_alias(&mut self, csid: u32, catcode: Catcode, charcode: u32) {
        *self.slot_mut(csid) = EqSlot::Char { catcode, charcode };
    }

    /// `\font\cs=<名字>`：cs 定义为字体选择器。
    pub fn set_font(&mut self, csid: u32, font: u32) {
        *self.slot_mut(csid) = EqSlot::Font(font);
    }

    /// 槽版本（供 M5 依赖追踪；Undefined 返回 None）。
    pub fn version_of(&self, csid: u32) -> Option<Version> {
        match self.slot(csid) {
            EqSlot::Macro(v) => Some(v.version),
            _ => None,
        }
    }

    /// 全部槽（`.fmt` 快照：导出用）。
    pub fn slots(&self) -> &[EqSlot] {
        &self.slots
    }

    /// 整体替换槽（`.fmt` 快照：加载用）。
    pub fn replace_slots(&mut self, slots: Vec<EqSlot>) {
        self.slots = slots;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catcode::Catcode;
    use crate::token::Token;

    fn tiny_def() -> MacroDef {
        MacroDef {
            params: crate::macrodef::ParamSpec {
                num_params: 0,
                long: false,
                delimiter: None,
            },
            body: Arc::from([Token::char(Catcode::Letter, b'A' as u32)]),
            code: None,
            protected: false,
        }
    }

    #[test]
    fn undefined_by_default() {
        let e = Eqtb::new();
        assert_eq!(e.slot(100), &EqSlot::Undefined);
    }

    #[test]
    fn define_macro_bumps_version() {
        let mut e = Eqtb::new();
        e.define_macro(1, tiny_def());
        let v1 = e.version_of(1).unwrap();
        e.define_macro(1, tiny_def());
        let v2 = e.version_of(1).unwrap();
        assert!(v2 > v1);
    }

    #[test]
    fn alias_redirects() {
        let mut e = Eqtb::new();
        e.alias(2, 3);
        assert_eq!(e.slot(2), &EqSlot::Alias(3));
    }

    #[test]
    fn char_alias_stores_catcode() {
        let mut e = Eqtb::new();
        e.char_alias(4, Catcode::Other, b'x' as u32);
        assert_eq!(
            e.slot(4),
            &EqSlot::Char {
                catcode: Catcode::Other,
                charcode: b'x' as u32
            }
        );
    }

    #[test]
    fn ensure_grows_to_csid() {
        let mut e = Eqtb::new();
        e.set_primitive(50, Primitive::Relax);
        assert_eq!(e.slot(50), &EqSlot::Primitive(Primitive::Relax));
        // 中间槽保持 Undefined
        assert_eq!(e.slot(49), &EqSlot::Undefined);
    }

    #[test]
    fn set_font_slot() {
        let mut e = Eqtb::new();
        e.set_font(7, 3);
        assert_eq!(e.slot(7), &EqSlot::Font(3));
    }
}
