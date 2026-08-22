//! 宏体字节码（M2-1/2）：定长指令 IR、编译器、反汇编。
//!
//! RFC-4 的落地子集：
//! - 每条指令编码为单个 u64（tag 高 4 bit；`Emit` 的 tag 为 0，**内联 8B token 原值**，
//!   RFC-1 布局零解包，`.fmt` 序列化友好）；
//! - **常量条件折叠**：平衡的 `\iftrue`/`\iffalse..\else..\fi` 在编译期直接求值，
//!   死分支不发射；其余 `\if*`（操作数来自运行期输入流）保持发射为控制序列 token，
//!   由运行期条件机（M1-9）处理——保证惰性跳过语义不变；
//! - 未平衡的条件不折叠（保持跨宏 `\if` 的运行期扫描语义）；
//! - 双轨等价框架（M2-7）：解释器（TokenArray）与字节码（[`Bytecode`]）逐 token 输出一致，
//!   由 ntex-core 测试套件全量覆盖。

use crate::eqtb::{EqSlot, Eqtb, Primitive};
use crate::intern::InternTable;
use crate::token::{meaning, Token, TokenKind};
use std::sync::Arc;

/// token/指令 tag 高位偏移（token 自身 tag 占高 4 bit，见 RFC-1 §3）。
pub const TAG_SHIFT: u32 = 60;

/// 运行期指令 tag：`Emit` 内联 token 原值（tag 即 token 自身 tag 0..=3）。
pub const EMIT_ARG_TAG: u64 = 4;
pub const END_TAG: u64 = 5;

/// 字节码指令（定长，编码为 u64）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    /// 发射一个 token（8B 原值内联）。
    Emit { token: Token },
    /// 发射第 n 个实参（`#n`，1..=9）。
    EmitArg { n: u8 },
    /// 宏体结束。
    End,
}

impl Instruction {
    /// 编码为单个 u64 字。
    ///
    /// 标签分配避免与 token 自身 tag（0..=3：Char/CS/MacroParam/EndGroup）冲突：
    /// `Emit` 直接内联 token 原值（高 4 bit 即 token tag），`EmitArg`/`End` 用 4/5。
    pub fn encode(self) -> u64 {
        match self {
            Instruction::Emit { token } => token.raw(),
            Instruction::EmitArg { n } => (4u64 << TAG_SHIFT) | u64::from(n),
            Instruction::End => 5u64 << TAG_SHIFT,
        }
    }

    /// 从 u64 字解码。
    pub fn decode(word: u64) -> Self {
        match word >> TAG_SHIFT {
            0..=3 => Instruction::Emit {
                token: Token::from_raw(word),
            },
            4 => Instruction::EmitArg {
                n: (word & 0xF) as u8,
            },
            _ => Instruction::End,
        }
    }
}

/// 宏体字节码：**定长 u64 原始字序列**（M2-6 补课）。
///
/// 运行时直接按 `word >> TAG_SHIFT` 分发（零解包）：tag 0..=3 即内联 token 原值，
/// `EmitArg`/`End` 分别以 `EMIT_ARG_TAG`/`END_TAG` 标记。`Instruction` 枚举仅保留为
/// 构造/反汇编/测试的便利视图（编码与字表示严格等价，见 [`Instruction::encode`]）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bytecode {
    words: Arc<[u64]>,
}

impl Bytecode {
    /// 原始指令字序列（只读）。
    pub fn words(&self) -> &[u64] {
        &self.words
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// 编码为 u64 字数组（`.fmt` 序列化 / 往返测试；即内部表示的视图拷贝）。
    pub fn to_words(&self) -> Vec<u64> {
        self.words.to_vec()
    }

    /// 从 u64 字数组解码。
    pub fn from_words(words: &[u64]) -> Self {
        Self {
            words: Arc::from(words),
        }
    }

    /// 反汇编为可读文本（每行 `pc: mnemonic operands`）。
    pub fn disassemble(&self, intern: &InternTable) -> String {
        let mut out = String::new();
        for (pc, &word) in self.words.iter().enumerate() {
            match word >> TAG_SHIFT {
                0..=3 => {
                    let token = Token::from_raw(word);
                    let desc = match token.kind() {
                        TokenKind::ControlSeq => meaning(token, intern),
                        _ => format!("{token:?}"),
                    };
                    out.push_str(&format!("{pc:>3}: emit {desc}\n"));
                }
                4 => out.push_str(&format!("{pc:>3}: arg {}\n", word & 0xF)),
                _ => out.push_str(&format!("{pc:>3}: end\n")),
            }
        }
        out
    }
}

/// 编译宏体为字节码（发射 token + 常量条件折叠）。
pub fn compile(body: &[Token], eqtb: &Eqtb) -> Bytecode {
    let mut out = Vec::new();
    let mut from = 0usize;
    compile_slice(body, &mut from, body.len(), &mut out, eqtb);
    out.push(Instruction::End.encode());
    Bytecode {
        words: Arc::from(out),
    }
}

/// 编译 `[from, to)` 区间（递归；`\iftrue/\iffalse` 折叠）。
fn compile_slice(tokens: &[Token], from: &mut usize, to: usize, out: &mut Vec<u64>, eqtb: &Eqtb) {
    while *from < to {
        let tok = tokens[*from];
        if let Some(const_val) = const_if_value(tok, eqtb) {
            if let Some((else_i, fi_i)) = find_matching_else_fi(tokens, *from, to, eqtb) {
                if const_val {
                    // \iftrue：发射 then 分支，跳过 else 分支（死代码）
                    *from += 1;
                    compile_slice(tokens, from, else_i.unwrap_or(fi_i), out, eqtb);
                    *from = fi_i + 1;
                } else {
                    // \iffalse：跳过 then 分支，发射 else 分支
                    *from = else_i.map_or(fi_i, |e| e + 1);
                    if else_i.is_some() {
                        compile_slice(tokens, from, fi_i, out, eqtb);
                    }
                    *from = fi_i + 1;
                }
                continue;
            }
        }
        match tok.kind() {
            TokenKind::MacroParam => {
                let n = tok.param_number().unwrap_or(1);
                out.push(Instruction::EmitArg { n }.encode());
            }
            _ => out.push(Instruction::Emit { token: tok }.encode()),
        }
        *from += 1;
    }
}

/// `\iftrue`/`\iffalse` 的常量值（其余返回 None）。
fn const_if_value(tok: Token, eqtb: &Eqtb) -> Option<bool> {
    let csid = tok.csid()?;
    match eqtb.slot(csid) {
        EqSlot::Primitive(Primitive::IfTrue) => Some(true),
        EqSlot::Primitive(Primitive::IfFalse) => Some(false),
        _ => None,
    }
}

/// 查找与 `if_idx` 平衡匹配的 `\else`/`\fi`。
///
/// 返回 `(else 下标或 None, fi 下标)`；区间内未闭合返回 `None`（不折叠）。
fn find_matching_else_fi(
    tokens: &[Token],
    if_idx: usize,
    to: usize,
    eqtb: &Eqtb,
) -> Option<(Option<usize>, usize)> {
    let mut depth = 1usize;
    let mut else_i = None;
    let mut i = if_idx + 1;
    while i < to {
        let tok = tokens[i];
        if let Some(csid) = tok.csid() {
            match eqtb.slot(csid) {
                EqSlot::Primitive(
                    Primitive::If
                    | Primitive::IfCat
                    | Primitive::IfNum
                    | Primitive::IfDim
                    | Primitive::IfX
                    | Primitive::IfOdd
                    | Primitive::IfCase
                    | Primitive::IfTrue
                    | Primitive::IfFalse,
                ) => depth += 1,
                EqSlot::Primitive(Primitive::Fi) => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((else_i, i));
                    }
                }
                EqSlot::Primitive(Primitive::Else) => {
                    if depth == 1 {
                        if else_i.is_some() {
                            return None; // 双 \else：不折叠，留给运行期报错
                        }
                        else_i = Some(i);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catcode::Catcode;
    use crate::expand::Expander;

    fn tokens(src: &str) -> Vec<Token> {
        // 复用展开器构造 token 序列（`_` 是 cat 8，控制词名需纯字母）
        let mut e = Expander::new_interpreter();
        e.run_source(&format!("\\def\\texbody{{{src}}}")).unwrap();
        let def = e.eqtb().slot(e.intern().lookup("texbody").unwrap()).clone();
        match def {
            EqSlot::Macro(m) => m.value.body.to_vec(),
            _ => panic!("构造失败"),
        }
    }

    #[test]
    fn encode_decode_round_trip() {
        let e = Expander::new();
        let body = tokens("A\\foo#1 B");
        let bc = compile(&body, e.eqtb());
        let words = bc.to_words();
        let decoded = Bytecode::from_words(&words);
        assert_eq!(decoded, bc);
    }

    #[test]
    fn instructions_are_fixed_width() {
        // 每个指令编码为单个 u64
        assert_eq!(Instruction::End.encode().to_be_bytes().len(), 8);
        let t = Token::char(Catcode::Letter, b'A' as u32);
        let w = Instruction::Emit { token: t }.encode();
        assert_eq!(Instruction::decode(w), Instruction::Emit { token: t });
    }

    #[test]
    fn compile_emits_arg_slots() {
        let e = Expander::new();
        let body = tokens("A#1B#2C");
        let bc = compile(&body, e.eqtb());
        let words = [
            Token::char(Catcode::Letter, b'A' as u32),
            Token::char(Catcode::Letter, b'B' as u32),
            Token::char(Catcode::Letter, b'C' as u32),
        ];
        assert_eq!(
            bc.to_words(),
            [
                Instruction::Emit { token: words[0] }.encode(),
                Instruction::EmitArg { n: 1 }.encode(),
                Instruction::Emit { token: words[1] }.encode(),
                Instruction::EmitArg { n: 2 }.encode(),
                Instruction::Emit { token: words[2] }.encode(),
                Instruction::End.encode(),
            ]
        );
    }

    #[test]
    fn const_iftrue_folds_else_branch() {
        let e = Expander::new();
        let body = tokens("\\iftrue A\\else B\\fi C");
        let bc = compile(&body, e.eqtb());
        let text = bc.disassemble(e.intern());
        assert!(
            text.contains("emit Char(cat=Letter,ch=65)"),
            "应含 A：\n{text}"
        );
        assert!(!text.contains("ch=66)"), "else 分支 B 应被折叠：\n{text}");
        assert!(text.contains("ch=67)"), "应含 C：\n{text}");
    }

    #[test]
    fn const_iffalse_folds_then_branch() {
        let e = Expander::new();
        let body = tokens("\\iffalse A\\else B\\fi C");
        let bc = compile(&body, e.eqtb());
        let text = bc.disassemble(e.intern());
        assert!(!text.contains("ch=65)"), "then 分支 A 应被折叠：\n{text}");
        assert!(text.contains("ch=66)"), "应含 B：\n{text}");
        assert!(text.contains("ch=67)"), "应含 C：\n{text}");
    }

    #[test]
    fn unbalanced_if_not_folded() {
        let e = Expander::new();
        // \iftrue 无 \fi → 不折叠，原样发射
        let body = tokens("\\iftrue A");
        let bc = compile(&body, e.eqtb());
        let text = bc.disassemble(e.intern());
        assert!(text.contains("\\iftrue"), "未平衡条件不应折叠：\n{text}");
    }

    #[test]
    fn nested_const_folds() {
        let e = Expander::new();
        // 外层 \iftrue → then = "A" + 内层 \iffalse→"Y" + "B"；外层 else "C" 为死代码
        let body = tokens("\\iftrue A\\iffalse X\\else Y\\fi B\\else C\\fi");
        let bc = compile(&body, e.eqtb());
        let text = bc.disassemble(e.intern());
        assert!(text.contains("ch=65)"), "A 应保留：\n{text}");
        assert!(text.contains("ch=66)"), "B 应保留：\n{text}");
        assert!(
            text.contains("ch=89)"),
            "Y 应保留（内层 \\iffalse 的 else）：\n{text}"
        );
        assert!(!text.contains("ch=88)"), "X 应折叠：\n{text}");
        assert!(
            !text.contains("ch=67)"),
            "C 应折叠（外层 else 死代码）：\n{text}"
        );
    }

    #[test]
    fn nonconst_if_kept_as_token() {
        let e = Expander::new();
        let body = tokens("\\ifnum1>0 A\\else B\\fi");
        let bc = compile(&body, e.eqtb());
        let text = bc.disassemble(e.intern());
        // \ifnum 操作数来自运行期，保持为 token（不解体）
        assert!(text.contains("\\ifnum"), "\\ifnum 应原样发射：\n{text}");
    }
}
