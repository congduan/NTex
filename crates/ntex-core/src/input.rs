//! 字节输入 → token 扫描（M1-4：catcode 在 token 生成时固化）。
//!
//! 与 RFC-1 的关系：扫描器按"当前 catcode 表"把字节切分为 token；
//! token 一经生成，其 catcode 即固化，之后修改 catcode 不回写已生成 token。
//!
//! M1 简化（TRIP 冲刺时修正）：
//! - 行尾（cat 5）→ 空格 token（blank line → `\par` 语义未实现）；
//! - active 字符（cat 13）→ 同名控制序列（可 `\def`、可展开）。

use crate::catcode::{Catcode, CatcodeTable};
use crate::error::{Error, Result};
use crate::intern::InternTable;
use crate::token::Token;

/// 从字节流扫描下一个 token；输入耗尽返回 `None`。
///
/// `pos` 指向下一个待读字节；`intern` 用于驻留控制词/控制符号/active 名字。
pub fn scan_token(
    bytes: &[u8],
    pos: &mut usize,
    catcodes: &CatcodeTable,
    intern: &mut InternTable,
) -> Result<Option<Token>> {
    loop {
        let Some(&b) = bytes.get(*pos) else {
            return Ok(None);
        };
        let cat = catcodes.get(b);
        match cat {
            Catcode::Comment => {
                // 跳过注释直到行尾（行尾字符留到下一轮 → 空格 token）
                while *pos < bytes.len() && !catcodes.get(bytes[*pos]).is_end_of_line() {
                    *pos += 1;
                }
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
                let Some(&c) = bytes.get(*pos) else {
                    return Err(Error::invalid_input("输入以反斜杠结束"));
                };
                if catcodes.get(c).is_letter() {
                    // 控制词：连续字母（cat 11）
                    let start = *pos;
                    while *pos < bytes.len() && catcodes.get(bytes[*pos]).is_letter() {
                        *pos += 1;
                    }
                    let name = std::str::from_utf8(&bytes[start..*pos])
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
                    return Ok(Some(Token::control_sequence(csid)));
                }
                // 控制符号：单个任意非字母字符（含空格、`\` 自身）
                *pos += 1;
                let csid = intern.intern(&char::from(c).to_string());
                return Ok(Some(Token::control_sequence(csid)));
            }
            _ => {
                *pos += 1;
                // 行尾 → 空格（M1 简化）
                let cat = if cat == Catcode::EndOfLine {
                    Catcode::Space
                } else {
                    cat
                };
                let tok = if cat == Catcode::Active {
                    // active 字符视作同名控制序列
                    let csid = intern.intern(&char::from(b).to_string());
                    Token::control_sequence(csid)
                } else {
                    Token::char(cat, b as u32)
                };
                return Ok(Some(tok));
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
        let mut out = Vec::new();
        while let Some(t) = scan_token(src.as_bytes(), &mut pos, &catcodes, &mut intern).unwrap() {
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
    fn comment_skips_to_end_of_line() {
        let toks = scan_all("% comment\nabc");
        // 注释被跳过，行尾 → 空格，然后 a,b,c
        assert_eq!(toks.len(), 4);
        assert_eq!(toks[0].catcode(), Some(Catcode::Space));
        assert_eq!(toks[1].charcode(), Some(b'a' as u32));
    }

    #[test]
    fn newline_becomes_space() {
        let toks = scan_all("a\nb");
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[1].catcode(), Some(Catcode::Space));
    }

    #[test]
    fn active_char_becomes_control_seq() {
        let toks = scan_all("~");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind(), TokenKind::ControlSeq);
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
        let bytes = [0x7F];
        assert!(scan_token(&bytes, &mut pos, &catcodes, &mut intern).is_err());
    }
}
