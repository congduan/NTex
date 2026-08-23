//! 字节输入 → token 扫描（M1-4：catcode 在 token 生成时固化）。
//!
//! 与 RFC-1 的关系：扫描器按"当前 catcode 表"把字节切分为 token；
//! token 一经生成，其 catcode 即固化，之后修改 catcode 不回写已生成 token。
//!
//! 扫描状态机（对照 tex.web `get_next` 的 `state`：new_line/mid_line/in_space）：
//! - 行首（[`ScanState::LineStart`]）空格忽略；行首行尾 → `\par`（**空行**语义）；
//! - 行中行尾 → 空格；连续空格折叠为单个；行尾后回到行首状态；
//! - 注释（cat 14）吞掉整行（**含**行尾字符，TeX 语义：注释行不产生 token）；
//! - active 字符（cat 13）→ 同名控制序列（可 `\def`、可展开）。

use crate::catcode::{Catcode, CatcodeTable};
use crate::error::{Error, Result};
use crate::intern::InternTable;
use crate::token::Token;

/// `^^` 转义解码（TeXbook p.45）：`pos` 指向第一个 `^`（catcode 7）。
///
/// 命中 `^^` 时消费字节并返回解码后的字符码：
/// - 后随两位十六进制数字（`^^5e`）→ 按十六进制取值；
/// - 否则单字符规则：字符码 <64 加 64，64..=127 减 64（如 `^^A`→1、`^^@`→0、`^^?`→127）。
///
/// 非 `^^` 情形返回 `None` 且不消费任何字节。
fn decode_circumflex(bytes: &[u8], pos: &mut usize, catcodes: &CatcodeTable) -> Option<u8> {
    // 第一个 `^` 必须 catcode 7（superscript），第二个仅按字符码 94 匹配（tex.web 同规则）
    if catcodes.get(*bytes.get(*pos)?) != Catcode::Superscript {
        return None;
    }
    if *bytes.get(*pos + 1)? != b'^' {
        return None;
    }
    let x = *bytes.get(*pos + 2)?;
    let hex = |b: u8| -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    };
    // 十六进制对：^^5e → 0x5E
    if let (Some(hi), Some(&lo)) = (hex(x), bytes.get(*pos + 3)) {
        if let Some(lo) = hex(lo) {
            *pos += 4;
            return Some(hi * 16 + lo);
        }
    }
    // 单字符规则
    *pos += 3;
    Some(if x < 64 {
        x + 64
    } else if x < 128 {
        x - 64
    } else {
        x
    })
}

/// 扫描器行状态（tex.web `get_next` 的 `state`：new_line / mid_line / in_space）。
///
/// 决定两个 TeX 行为：
/// - **空行 → `\par`**：行首（[`ScanState::LineStart`]）遇到行尾（cat 5）产生 `\par`
///   而非空格；
/// - **空格折叠**：行首空格忽略、连续空格合并为单个 token。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanState {
    /// 行首（tex.web new_line）：空格忽略；行尾 → `\par`（空行）。
    #[default]
    LineStart,
    /// 行中（tex.web mid_line）：已产生非空格 token；首个空格 → 空格 token。
    MidLine,
    /// 已产生空格（tex.web in_space）：连续空格忽略；行尾 → 空格 token。
    InSpace,
}

/// 从字节流扫描下一个 token；输入耗尽返回 `None`。
///
/// `pos` 指向下一个待读字节；`intern` 用于驻留控制词/控制符号/active 名字；
/// `state` 为扫描器行状态（每行行尾自动回到 [`ScanState::LineStart`]）。
pub fn scan_token(
    bytes: &[u8],
    pos: &mut usize,
    catcodes: &CatcodeTable,
    intern: &mut InternTable,
    state: &mut ScanState,
) -> Result<Option<Token>> {
    loop {
        let Some(&b) = bytes.get(*pos) else {
            return Ok(None);
        };
        let cat = catcodes.get(b);
        match cat {
            Catcode::Comment => {
                // 注释吞掉整行（**含**行尾字符）：不产生 token，状态不变
                // （tex.web：注释行不触发空行判定，如 `a%c\nb` → "ab"）。
                while *pos < bytes.len() && !catcodes.get(bytes[*pos]).is_end_of_line() {
                    *pos += 1;
                }
                *pos += 1; // 越过行尾字符（若存在）
                continue;
            }
            Catcode::Ignored => {
                *pos += 1;
                continue;
            }
            Catcode::Invalid => {
                *pos += 1;
                return Err(Error::invalid_input(format!("非法字符 0x{b:02X}")));
            }
            Catcode::Escape => {
                *pos += 1;
                let Some(&c0) = bytes.get(*pos) else {
                    return Err(Error::invalid_input("输入以反斜杠结束"));
                };
                // ^^ 转义（\^^@、\^^? 等）：控制符号名取解码后的字符
                let mut first = c0;
                let mut advanced = false;
                if c0 == b'^'
                    && catcodes.get(c0) == Catcode::Superscript
                    && bytes.get(*pos + 1) == Some(&b'^')
                {
                    if let Some(d) = decode_circumflex(bytes, pos, catcodes) {
                        first = d;
                        advanced = true;
                    }
                }
                if catcodes.get(first).is_letter() {
                    // 控制词：首字符（可能为 ^^ 解码）+ 连续字母（cat 11）
                    if !advanced {
                        *pos += 1; // 首字符未解码：越过它再读后续字母
                    }
                    let mut name = vec![first];
                    let mut i = *pos;
                    while i < bytes.len() && catcodes.get(bytes[i]).is_letter() {
                        name.push(bytes[i]);
                        i += 1;
                    }
                    *pos = i;
                    let name = std::str::from_utf8(&name)
                        .map_err(|_| Error::invalid_input("控制词含非 UTF-8 字节"))?;
                    let csid = intern.intern(name);
                    // TeX：控制词后跟随的空格被吞掉（含行尾转换的空格）
                    while *pos < bytes.len() {
                        let c2 = catcodes.get(bytes[*pos]);
                        if !(c2.is_space() || c2.is_end_of_line()) {
                            break;
                        }
                        *pos += 1;
                    }
                    *state = ScanState::MidLine;
                    return Ok(Some(Token::control_sequence(csid)));
                }
                // 控制符号：单个任意非字母字符（含空格、`\` 自身、^^ 解码字符）
                if !advanced {
                    *pos += 1;
                }
                let csid = intern.intern(&char::from(first).to_string());
                *state = ScanState::MidLine;
                return Ok(Some(Token::control_sequence(csid)));
            }
            _ => {
                // ^^ 转义：catcode 7 的 ^ 后随 ^ → 解码为单个字符 token
                let mut ch = b;
                let mut cat = cat;
                if cat == Catcode::Superscript && b == b'^' && bytes.get(*pos + 1) == Some(&b'^') {
                    if let Some(d) = decode_circumflex(bytes, pos, catcodes) {
                        ch = d;
                        cat = catcodes.get(d);
                    } else {
                        *pos += 1; // 非 ^^（如行尾）：按单字符处理
                    }
                } else {
                    *pos += 1;
                }
                // 行状态机（tex.web get_next）：空格折叠 + 空行 → \par。
                // 注意 `^^M`（cat 5）也在此路径解码为行尾，走同样判定。
                match cat {
                    Catcode::EndOfLine => {
                        // 空行（行首行尾）→ `\par`；否则行尾 → 空格（charcode 用 32：
                        // 定界符匹配——分隔实参 `#5 ` 的定界空格是 char 32，若保留 LF 将失配）
                        if *state == ScanState::LineStart {
                            let csid = intern.intern("par");
                            return Ok(Some(Token::control_sequence(csid)));
                        }
                        *state = ScanState::LineStart;
                        return Ok(Some(Token::char(Catcode::Space, b' ' as u32)));
                    }
                    Catcode::Space => {
                        // 行首空格忽略；连续空格合并；行中首个空格 → 空格 token
                        if *state == ScanState::MidLine {
                            *state = ScanState::InSpace;
                            return Ok(Some(Token::char(Catcode::Space, b' ' as u32)));
                        }
                        continue; // LineStart/InSpace：跳过，状态不变
                    }
                    Catcode::Active => {
                        // active 字符视作同名控制序列
                        *state = ScanState::MidLine;
                        let csid = intern.intern(&char::from(ch).to_string());
                        return Ok(Some(Token::control_sequence(csid)));
                    }
                    _ => {
                        *state = ScanState::MidLine;
                        return Ok(Some(Token::char(cat, ch as u32)));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::TokenKind;

    fn scan_all(src: &str) -> Vec<Token> {
        let mut intern = InternTable::new();
        let catcodes = CatcodeTable::new();
        let mut pos = 0usize;
        let mut state = ScanState::LineStart;
        let mut out = Vec::new();
        while let Some(t) =
            scan_token(src.as_bytes(), &mut pos, &catcodes, &mut intern, &mut state).unwrap()
        {
            out.push(t);
        }
        out
    }

    fn assert_kind_seq(src: &str, expected: &[TokenKind]) {
        let toks = scan_all(src);
        let kinds: Vec<_> = toks.iter().map(|t| t.kind()).collect();
        assert_eq!(kinds, expected, "tokens: {toks:?}");
    }

    #[test]
    fn scans_control_word_and_group() {
        let toks = scan_all("\\foo{a}");
        assert_kind_seq(
            "\\foo{a}",
            &[
                TokenKind::ControlSeq,
                TokenKind::Char,
                TokenKind::Char,
                TokenKind::Char,
            ],
        );
        // 校验细节：\foo、{、a、}
        assert_eq!(toks[0].csid(), Some(0)); // "foo"
        assert_eq!(toks[1].catcode(), Some(Catcode::BeginGroup));
        assert_eq!(toks[2].charcode(), Some(b'a' as u32));
        assert_eq!(toks[3].catcode(), Some(Catcode::EndGroup));
    }

    #[test]
    fn control_word_swallows_following_spaces() {
        // "\\foo bar" → \foo + b,a,r（空格被吞）
        let toks = scan_all("\\foo bar");
        assert_eq!(toks.len(), 4);
        assert!(toks.iter().skip(1).all(|t| t.kind() == TokenKind::Char));
        assert_eq!(toks[1].charcode(), Some(b'b' as u32));
    }

    #[test]
    fn control_symbol_single_char() {
        // "\." → cs(".")；"\\" → cs("\")
        let toks = scan_all("\\.");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind(), TokenKind::ControlSeq);

        let toks = scan_all("\\\\");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind(), TokenKind::ControlSeq);
    }

    #[test]
    fn comment_swallows_whole_line() {
        // 注释吞掉整行（含行尾）："% comment\nabc" → a,b,c（无行尾空格）
        let toks = scan_all("% comment\nabc");
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[0].charcode(), Some(b'a' as u32));
        assert_eq!(toks[1].charcode(), Some(b'b' as u32));
        assert_eq!(toks[2].charcode(), Some(b'c' as u32));
    }

    #[test]
    fn newline_becomes_space() {
        let toks = scan_all("a\nb");
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[1].catcode(), Some(Catcode::Space));
    }

    // ---------- 空行 → \par（A2；tex.web get_next 状态机） ----------

    #[test]
    fn blank_line_becomes_par() {
        // a\n\nb → a <空格> \par b（第二个 \n 在行首 → \par）
        let mut intern = InternTable::new();
        let catcodes = CatcodeTable::new();
        let mut pos = 0usize;
        let mut state = ScanState::LineStart;
        let mut toks = Vec::new();
        while let Some(t) =
            scan_token(b"a\n\nb", &mut pos, &catcodes, &mut intern, &mut state).unwrap()
        {
            toks.push(t);
        }
        assert_eq!(toks.len(), 4);
        assert_eq!(toks[0].charcode(), Some(b'a' as u32));
        assert_eq!(toks[1].catcode(), Some(Catcode::Space));
        assert_eq!(toks[2].kind(), TokenKind::ControlSeq);
        assert_eq!(intern.name(toks[2].csid().unwrap()), "par");
        assert_eq!(toks[3].charcode(), Some(b'b' as u32));
    }

    #[test]
    fn blank_line_with_leading_spaces_becomes_par() {
        // 行首空格忽略：a\n   \nb → a <空格> \par b
        let toks = scan_all("a\n   \nb");
        assert_eq!(toks.len(), 4);
        assert_eq!(toks[0].charcode(), Some(b'a' as u32));
        assert_eq!(toks[1].catcode(), Some(Catcode::Space));
        assert_eq!(toks[2].kind(), TokenKind::ControlSeq); // \par
        assert_eq!(toks[3].charcode(), Some(b'b' as u32));
    }

    #[test]
    fn blank_line_at_eof() {
        // 尾部空行：a\n\n → a <空格> \par
        let toks = scan_all("a\n\n");
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[2].kind(), TokenKind::ControlSeq); // \par
    }

    #[test]
    fn leading_spaces_ignored() {
        // 行首空格不产生 token
        let toks = scan_all("  a");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].charcode(), Some(b'a' as u32));
    }

    #[test]
    fn consecutive_spaces_folded() {
        // 连续空格合并为单个空格 token（tex.web in_space）
        let toks = scan_all("a   b");
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[1].catcode(), Some(Catcode::Space));
    }

    #[test]
    fn space_then_newline_yields_two_spaces() {
        // "a \n b"：行中空格 + 行尾空格 → 两个空格 token（TeX 语义）
        let toks = scan_all("a \n b");
        assert_eq!(toks.len(), 4); // a, sp, sp, b
        assert_eq!(toks[1].catcode(), Some(Catcode::Space));
        assert_eq!(toks[2].catcode(), Some(Catcode::Space));
    }

    #[test]
    fn comment_line_produces_no_par() {
        // 注释行不产生 \par（注释吞掉含行尾）：a\n%c\nb → a <空格> b
        let toks = scan_all("a\n%comment\nb");
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[0].charcode(), Some(b'a' as u32));
        assert_eq!(toks[1].catcode(), Some(Catcode::Space));
        assert_eq!(toks[2].charcode(), Some(b'b' as u32));
    }

    #[test]
    fn trailing_comment_swallows_newline() {
        // a%comment\nb → ab（注释吞掉行尾，无空格）
        let toks = scan_all("a%comment\nb");
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].charcode(), Some(b'a' as u32));
        assert_eq!(toks[1].charcode(), Some(b'b' as u32));
    }

    #[test]
    fn comment_line_then_blank_line_produces_par() {
        // 注释行后真空行：%c\n\nb → \par b
        let toks = scan_all("%c\n\nb");
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].kind(), TokenKind::ControlSeq); // \par
        assert_eq!(toks[1].charcode(), Some(b'b' as u32));
    }

    #[test]
    fn active_char_becomes_control_seq() {
        let toks = scan_all("~");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind(), TokenKind::ControlSeq);
    }

    #[test]
    fn circumflex_escape_decodes_char() {
        // ^^A → 字符码 1（A=65 − 64）
        let toks = scan_all("^^A");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind(), TokenKind::Char);
        assert_eq!(toks[0].charcode(), Some(1));
        // ^^@ → 字符码 0（@=64 − 64）
        let toks = scan_all("^^@");
        assert_eq!(toks[0].charcode(), Some(0));
        // ^^? → 字符码 127（?=63 + 64）
        let toks = scan_all("^^?");
        assert_eq!(toks[0].charcode(), Some(127));
        // 十六进制对：^^5e → 0x5E
        let toks = scan_all("^^5e");
        assert_eq!(toks[0].charcode(), Some(0x5E));
        // 单个 ^ 不转义
        let toks = scan_all("^A");
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].charcode(), Some(b'^' as u32));
    }

    #[test]
    fn circumflex_escape_after_escape_char() {
        let mut intern = InternTable::new();
        let catcodes = CatcodeTable::new();
        // \^^@ → 控制符号，名字为字符码 0
        let mut pos = 0usize;
        let mut state = ScanState::LineStart;
        let tok = scan_token(b"\\^^@", &mut pos, &catcodes, &mut intern, &mut state)
            .unwrap()
            .unwrap();
        assert_eq!(tok.kind(), TokenKind::ControlSeq);
        let csid = tok.csid().unwrap();
        assert_eq!(intern.name(csid), "\0");
        // \^^? → 控制符号，名字为字符码 127
        let mut pos = 0usize;
        let mut state = ScanState::LineStart;
        let tok = scan_token(b"\\^^?", &mut pos, &catcodes, &mut intern, &mut state)
            .unwrap()
            .unwrap();
        assert_eq!(intern.name(tok.csid().unwrap()), "\u{7f}");
        // \^^A：A 解码为 1，非字母 → 控制符号
        let mut pos = 0usize;
        let mut state = ScanState::LineStart;
        let tok = scan_token(b"\\^^A", &mut pos, &catcodes, &mut intern, &mut state)
            .unwrap()
            .unwrap();
        assert_eq!(intern.name(tok.csid().unwrap()), "\u{1}");
    }

    #[test]
    fn end_of_input_returns_none() {
        let toks = scan_all("");
        assert!(toks.is_empty());
    }

    #[test]
    fn invalid_byte_errors() {
        let mut intern = InternTable::new();
        let catcodes = CatcodeTable::new();
        let mut pos = 0usize;
        let mut state = ScanState::LineStart;
        let bytes = [0x7F];
        assert!(scan_token(&bytes, &mut pos, &catcodes, &mut intern, &mut state).is_err());
    }
}
