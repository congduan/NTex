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
/// 十六进制对仅限小写 `a`-`f` 与 `0`-`9`（pdfTeX 实测：`^^Ab`→字符1+'b'、
/// `^^A0`→字符1+'0'，大写 A-F 不参与配对）；`^^a`（后随非 hex）→ 97-64=33。
///
/// UTF-8 多字节序列解码（M9 中文刀 2：`\utfinputmode=1` 时启用）。
///
/// `pos` 指向待读字节；仅当该字节 ≥ 0x80（UTF-8 领先字节/孤儿续字节）时尝试解码，
/// 返回 `(码位, 消费字节数)`。解码用 `std::str::from_utf8` 按前缀长度 2..=4 逐步
/// 尝试（合法 UTF-8 最长 4 字节；std 保证不产出代理区/超 21-bit 的值）。
/// 解码失败返回 `None` 且不消费——调用方落回单字节处理（按 8-bit catcode 表，
/// 通常得到 cat 12 token 或 cat 15 可恢复错误，与字节模式一致）。
fn decode_utf8_char(bytes: &[u8], pos: usize) -> Option<(u32, usize)> {
    let b = *bytes.get(pos)?;
    if b < 0x80 {
        return None;
    }
    for len in 2..=4usize {
        let Some(slice) = bytes.get(pos..pos + len) else {
            break; // 尾部截断：不再有更长的合法前缀
        };
        if let Ok(s) = std::str::from_utf8(slice) {
            let cp = s.chars().next()? as u32;
            return Some((cp, len));
        }
    }
    None
}

/// `^^` 转义解码（TeXbook p.45）：`pos` 指向第一个 `^`（catcode 7）。
///
/// 命中 `^^` 时消费字节并返回解码后的字符码：
/// - 后随两位十六进制数字（`^^5e`）→ 按十六进制取值；
/// - 否则单字符规则：字符码 <64 加 64，64..=127 减 64（如 `^^A`→1、`^^@`→0、`^^?`→127）。
///
/// 十六进制对仅限小写 `a`-`f` 与 `0`-`9`（pdfTeX 实测：`^^Ab`→字符1+'b'、
/// `^^A0`→字符1+'0'，大写 A-F 不参与配对）；`^^a`（后随非 hex）→ 97-64=33。
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

/// 预建行起始偏移表（第 k 项 = 第 k+1 行的字节偏移；恒含 0）。
///
/// `current_line_no` 等按 `pos` 二分求行号，替代原来每次从头线性数 `\n`
/// （后者在每组事件/报错都触发时是 O(pos) 热点——组密集文档整体 O(n²)）。
pub fn line_starts(bytes: &[u8]) -> Vec<u32> {
    let mut v = Vec::with_capacity(bytes.len() / 16 + 1);
    v.push(0);
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            v.push((i + 1) as u32);
        }
    }
    v
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
///
/// `utf8_input`（M9 中文刀 2）：`\utfinputmode=1` 时源码按 UTF-8 解码——领先字节
/// ≥0x80 的多字节序列合并为单个 21-bit 字符 token（>255 码位 catcode 默认
/// letter，见 [`CatcodeTable::get_codepoint`]），控制词/控制符号名可含非 ASCII
/// 字母。默认 `false`（bytes）：逐字节语义与 TRIP/ETRIP 口径完全一致，该分支
/// 一个字节都不会碰到。
pub fn scan_token(
    bytes: &[u8],
    pos: &mut usize,
    catcodes: &CatcodeTable,
    intern: &mut InternTable,
    state: &mut ScanState,
    utf8_input: bool,
) -> Result<Option<Token>> {
    loop {
        let Some(&b) = bytes.get(*pos) else {
            return Ok(None);
        };
        let cat = catcodes.get(b);
        match cat {
            Catcode::Comment => {
                // 注释吞掉整行（**含**行尾字符）：不产生 token
                // （tex.web：注释行不触发空行判定，如 `a%c\nb` → "ab"）。
                while *pos < bytes.len() && !catcodes.get(bytes[*pos]).is_end_of_line() {
                    *pos += 1;
                }
                *pos += 1; // 越过行尾字符（若存在）
                           // tex.web get_next：行尾（注释 `@<Finish line,|goto switch|@>` 后
                           // `loc>limit`）→ `state:=new_line`（L7277）——下一行**行首**空格在
                           // new_line 状态被忽略（`new_line+spacer` 属"被忽略字符"，L7310）。
                           // 此前状态不重置：`\ifnum0%` 后换行缩进的空格被当作行中空格产出
                           // token，终止外层数字扫描（latex.ltx L1122 引擎检查 `\ifnum0%` +
                           // 缩进 `\ifdefined` 探针恒报 "Missing = inserted" 的根因）。重置为
                           // LineStart 后行首空格忽略、探针直接续接数字。
                *state = ScanState::LineStart;
                continue;
            }
            Catcode::Ignored => {
                *pos += 1;
                continue;
            }
            Catcode::Invalid => {
                // TeX（tex.web get_next invalid_char）：报 "! Text line contains an
                // invalid character." 并**跳过该字符继续**（不产生 token）；错误消息
                // 由调用方（fetch）写入转录，此处仅消费字节并返回可恢复错误。
                *pos += 1;
                return Err(Error::invalid_character(b));
            }
            Catcode::Escape => {
                *pos += 1;
                let Some(&c0) = bytes.get(*pos) else {
                    return Err(Error::invalid_input("输入以反斜杠结束"));
                };
                // 首字符三路解码：^^ 转义（\^^@、\^^? 等）/ UTF-8 多字节
                // （utf8 模式，M9 中文刀 2：`\中文` 可作控制词/控制符号名）/ 单字节
                let mut first_cp: u32 = c0 as u32;
                let mut first_len: usize = 1;
                if c0 == b'^'
                    && catcodes.get(c0) == Catcode::Superscript
                    && bytes.get(*pos + 1) == Some(&b'^')
                {
                    if let Some(d) = decode_circumflex(bytes, pos, catcodes) {
                        first_cp = d as u32;
                        first_len = 0; // decode_circumflex 已消费字节
                    }
                } else if utf8_input && c0 >= 0x80 {
                    if let Some((cp, len)) = decode_utf8_char(bytes, *pos) {
                        first_cp = cp;
                        first_len = len;
                    }
                }
                *pos += first_len;
                let first_char = char::from_u32(first_cp);
                if catcodes.get_codepoint(first_cp).is_letter() {
                    // 控制词：首字符 + 连续字母（cat 11；utf8 模式下非 ASCII
                    // letter 同样并入——`\中文` → 控制词 "中文"）
                    let mut name = first_char.unwrap_or('\u{FFFD}').to_string().into_bytes();
                    let mut i = *pos;
                    while i < bytes.len() {
                        let b = bytes[i];
                        if catcodes.get(b).is_letter() {
                            name.push(b);
                            i += 1;
                        } else if utf8_input && b >= 0x80 {
                            // UTF-8 多字节字母（M9 中文刀 2）
                            match decode_utf8_char(bytes, i) {
                                Some((cp, len)) if catcodes.get_codepoint(cp).is_letter() => {
                                    let c = char::from_u32(cp).unwrap_or('\u{FFFD}');
                                    let mut tmp = [0u8; 4];
                                    name.extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
                                    i += len;
                                }
                                _ => break,
                            }
                        } else if b == b'^'
                            && catcodes.get(b) == Catcode::Superscript
                            && bytes.get(i + 1) == Some(&b'^')
                        {
                            // ^^ 转义：解码后若 cat 11（letter）则并入控制词名
                            let mut dpos = i;
                            if let Some(d) = decode_circumflex(bytes, &mut dpos, catcodes) {
                                if catcodes.get(d).is_letter() {
                                    name.push(d);
                                    i = dpos;
                                    continue;
                                }
                            }
                            break;
                        } else {
                            break;
                        }
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
                // 控制符号：单个任意非字母字符（含空格、`\` 自身、^^/UTF-8 解码字符）
                let csid = intern.intern(&first_char.unwrap_or('\u{FFFD}').to_string());
                *state = ScanState::MidLine;
                return Ok(Some(Token::control_sequence(csid)));
            }
            _ => {
                // 字符三路解码 ^^ 转义：catcode 7 的 ^ 后随 ^ → 解码为单个字符 token
                let ch: u32;
                let mut cat = cat;
                if utf8_input && b >= 0x80 {
                    // UTF-8 多字节（M9 中文刀 2）：领先字节/孤儿续字节 ≥0x80 →
                    // 尝试整体解码为单个 21-bit 字符 token；失败（孤儿续字节/
                    // 尾部截断）落回单字节——按 8-bit 表处理，行为与 bytes 模式一致
                    match decode_utf8_char(bytes, *pos) {
                        Some((cp, len)) => {
                            *pos += len;
                            ch = cp;
                            cat = catcodes.get_codepoint(cp);
                        }
                        None => {
                            *pos += 1;
                            ch = b as u32;
                        }
                    }
                } else if cat == Catcode::Superscript
                    && b == b'^'
                    && bytes.get(*pos + 1) == Some(&b'^')
                {
                    if let Some(d) = decode_circumflex(bytes, pos, catcodes) {
                        ch = d as u32;
                        cat = catcodes.get(d);
                    } else {
                        *pos += 1; // 非 ^^（如行尾）：按单字符处理
                        ch = b as u32;
                    }
                } else {
                    *pos += 1;
                    ch = b as u32;
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
                        // 行尾插入字符按其**当前 catcode** 处理（tex.web
                        // end_line_char 语义；本引擎行模型硬编码行尾插入 char 32）。
                        // expl3 把 cat 32 设 Ignored(9)：行尾空格被忽略 → 行边界
                        // 消失——l3names `\def\__kernel_primitive:NN #1#2{` 的参数
                        // 文本因此不含尾随空格；否则 #2 变"空格定界"实参、实参扫描
                        // 一路吞到首个 `}`（l.398/l.819 级联的第三根因，报告 §18）。
                        // cat 32 维持 Space(10) 时行为不变（plain/TRIP/LaTeX）。
                        match catcodes.get(b' ') {
                            Catcode::Ignored => continue,
                            Catcode::Space => {
                                return Ok(Some(Token::char(Catcode::Space, b' ' as u32)))
                            }
                            // 罕见：cat 32 被设为其他 catcode → 按该 catcode 产出
                            other => return Ok(Some(Token::char(other, b' ' as u32))),
                        }
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
                        // active 字符视作同名控制序列；带 active 标志
                        // （tex.web：active char 是 cs token，但位于 eqtb
                        // active 区、结构上与命名 cs 可分——\lowercase/
                        // \uppercase 的 change_case 语义依赖该区分）
                        *state = ScanState::MidLine;
                        let csid =
                            intern.intern(&char::from_u32(ch).unwrap_or('\u{FFFD}').to_string());
                        return Ok(Some(Token::active_sequence(csid)));
                    }
                    _ => {
                        *state = ScanState::MidLine;
                        return Ok(Some(Token::char(cat, ch)));
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
        while let Some(t) = scan_token(
            src.as_bytes(),
            &mut pos,
            &catcodes,
            &mut intern,
            &mut state,
            false,
        )
        .unwrap()
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
        while let Some(t) = scan_token(
            b"a\n\nb",
            &mut pos,
            &catcodes,
            &mut intern,
            &mut state,
            false,
        )
        .unwrap()
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
    fn comment_resets_to_line_start_ignoring_next_indent() {
        // tex.web get_next：注释吞行后 state:=new_line（L7277），下一行**行首**
        // 空格被忽略（new_line+spacer 属被忽略字符，L7310）。此前状态不重置导致
        // `\ifnum0%` 后换行缩进的空格被当行中空格产出 → 终止外层数字扫描
        // （latex.ltx L1122 引擎检查报 "Missing = inserted" 的根因）。
        // a%c\n   b → a,b（缩进空格不产出 token）
        let toks = scan_all("a%c\n   b");
        assert_eq!(toks.len(), 2, "tokens: {toks:?}");
        assert_eq!(toks[0].charcode(), Some(b'a' as u32));
        assert_eq!(toks[1].charcode(), Some(b'b' as u32));
        // 注释行后下一行即空行（行首行尾）→ 空行仍 \par（a%c\n   \nb → a \par b）
        let toks = scan_all("a%c\n   \nb");
        assert_eq!(toks.len(), 3, "tokens: {toks:?}");
        assert_eq!(toks[0].charcode(), Some(b'a' as u32));
        assert_eq!(toks[1].kind(), TokenKind::ControlSeq); // \par
        assert_eq!(toks[2].charcode(), Some(b'b' as u32));
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
        let tok = scan_token(
            b"\\^^@",
            &mut pos,
            &catcodes,
            &mut intern,
            &mut state,
            false,
        )
        .unwrap()
        .unwrap();
        assert_eq!(tok.kind(), TokenKind::ControlSeq);
        let csid = tok.csid().unwrap();
        assert_eq!(intern.name(csid), "\0");
        // \^^? → 控制符号，名字为字符码 127
        let mut pos = 0usize;
        let mut state = ScanState::LineStart;
        let tok = scan_token(
            b"\\^^?",
            &mut pos,
            &catcodes,
            &mut intern,
            &mut state,
            false,
        )
        .unwrap()
        .unwrap();
        assert_eq!(intern.name(tok.csid().unwrap()), "\u{7f}");
        // \^^A：A 解码为 1，非字母 → 控制符号
        let mut pos = 0usize;
        let mut state = ScanState::LineStart;
        let tok = scan_token(
            b"\\^^A",
            &mut pos,
            &catcodes,
            &mut intern,
            &mut state,
            false,
        )
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
        assert!(scan_token(&bytes, &mut pos, &catcodes, &mut intern, &mut state, false).is_err());
    }

    // ---------- M9 中文刀 2：UTF-8 输入模式（\utfinputmode=1） ----------

    /// 指定编码模式扫描全部 token。
    fn scan_all_mode(src: &str, utf8: bool) -> Vec<Token> {
        let mut intern = InternTable::new();
        let catcodes = CatcodeTable::new();
        let mut pos = 0usize;
        let mut state = ScanState::LineStart;
        let mut out = Vec::new();
        while let Some(t) = scan_token(
            src.as_bytes(),
            &mut pos,
            &catcodes,
            &mut intern,
            &mut state,
            utf8,
        )
        .unwrap()
        {
            out.push(t);
        }
        out
    }

    #[test]
    fn utf8_mode_merges_multibyte_into_single_token() {
        // utf8 模式：「中文」→ 两个 21-bit 字符 token，catcode letter（XeTeX 惯例）
        let toks = scan_all_mode("中文", true);
        assert_eq!(toks.len(), 2, "tokens: {toks:?}");
        assert_eq!(toks[0].charcode(), Some(0x4E2D));
        assert_eq!(toks[0].catcode(), Some(Catcode::Letter));
        assert_eq!(toks[1].charcode(), Some(0x6587));
        assert_eq!(toks[1].catcode(), Some(Catcode::Letter));
    }

    #[test]
    fn bytes_mode_keeps_single_byte_semantics() {
        // 默认 bytes 模式（TRIP/ETRIP 口径）：同一输入按 8-bit 切成单字节 token
        let toks = scan_all_mode("中", false);
        assert_eq!(
            toks.iter().map(|t| t.charcode()).collect::<Vec<_>>(),
            vec![Some(0xE4), Some(0xB8), Some(0xAD)]
        );
    }

    #[test]
    fn utf8_mode_mixed_ascii_and_cjk() {
        // 混排：行状态机行为与纯 ASCII 一致（行中空格产出空格 token）
        let toks = scan_all_mode("a中 b", true);
        assert_eq!(
            toks.iter().map(|t| t.charcode()).collect::<Vec<_>>(),
            vec![
                Some(b'a' as u32),
                Some(0x4E2D),
                Some(b' ' as u32),
                Some(b'b' as u32)
            ]
        );
    }

    #[test]
    fn utf8_mode_cjk_control_word() {
        // `\中文 x` → 控制词 "中文" + 字符 x（后续空格被吞）
        let mut intern = InternTable::new();
        let catcodes = CatcodeTable::new();
        let mut pos = 0usize;
        let mut state = ScanState::LineStart;
        let mut toks = Vec::new();
        while let Some(t) = scan_token(
            "\\中文 x".as_bytes(),
            &mut pos,
            &catcodes,
            &mut intern,
            &mut state,
            true,
        )
        .unwrap()
        {
            toks.push(t);
        }
        assert_eq!(toks.len(), 2, "tokens: {toks:?}");
        assert_eq!(toks[0].kind(), TokenKind::ControlSeq);
        assert_eq!(intern.name(toks[0].csid().unwrap()), "中文");
        assert_eq!(toks[1].charcode(), Some(b'x' as u32));
    }

    #[test]
    fn utf8_mode_orphan_continuation_falls_back_to_byte() {
        // 孤儿续字节 0x80：解码失败落回单字节（cat 12），不 panic、不吞后续输入
        let toks = scan_all_mode("A\u{80}B", true);
        assert_eq!(
            toks.iter().map(|t| t.charcode()).collect::<Vec<_>>(),
            vec![Some(b'A' as u32), Some(0x80), Some(0x42)]
        );
    }

    #[test]
    fn utf8_mode_comment_and_newline_unchanged() {
        // 注释/行尾仍是字节级语义（% 与 \n 是 ASCII）：a%中\nb → a b
        let toks = scan_all_mode("a%中\nb", true);
        assert_eq!(
            toks.iter().map(|t| t.charcode()).collect::<Vec<_>>(),
            vec![Some(b'a' as u32), Some(b'b' as u32)]
        );
    }

    #[test]
    fn utf8_mode_circumflex_still_works() {
        // ^^ 转义在 utf8 模式不受影响（ASCII 字节级规则优先）
        let toks = scan_all_mode("^^5e", true);
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].charcode(), Some(0x5E));
    }
}
