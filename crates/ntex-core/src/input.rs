//! 字节输入 → token 扫描（M1-4：catcode 在 token 生成时固化）。
//!
//! 与 RFC-1 的关系：扫描器按"当前 catcode 表"把字节切分为 token；
//! token 一经生成，其 catcode 即固化，之后修改 catcode 不回写已生成 token。
//!
//! 扫描状态机（对照 tex.web `get_next` 的 `state`：new_line/mid_line/in_space）：
//! - **物理行边界 = LF 字节**（按字节身份识别，与 catcode(0x0A) 无关——tex.web
//!   行尾字节是按位置写入的 `end_line_char`，OS 换行字节不进 buffer）；
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

/// 扫描器行状态（tex.web `get_next` 的 `state`：new_line / mid_line / skip_blanks）。
///
/// 决定两个 TeX 行为：
/// - **空行 → `\par`**：行首（[`ScanState::LineStart`]）遇到行尾（cat 5）产生 `\par`
///   而非空格；
/// - **空格折叠**：行首空格忽略、连续空格合并为单个 token；
/// - **skip_blanks 吸收**：[`ScanState::InSpace`] 即 tex.web `skip_blanks`——
///   控制词/空格控制符号后进入，后随空格**按读取时的 catcode** 逐个忽略
///   （`skip_blanks+spacer` 在「忽略字符」表里），行尾不产 token
///   （`skip_blanks+car_ret → Finish line, goto switch`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanState {
    /// 行首（tex.web new_line）：空格忽略；行尾 → `\par`（空行）。
    #[default]
    LineStart,
    /// 行中（tex.web mid_line）：已产生非空格 token；首个空格 → 空格 token。
    MidLine,
    /// tex.web skip_blanks：已产生空格或刚读控制词/空格控制符号——后随空格
    /// 忽略（catcode 读取时裁决），行尾不产 token。
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
    endlinechar: i64,
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
                // 物理行尾（LF 字节）按字节身份终止丢弃——tex.web 注释丢弃是
                // 按位置的（`loc:=limit+1`），catcode(0x0A) 被 `\catcode`\^^J=…`
                // 改写后注释不得吞过物理行尾（否则一条注释吞掉余下整个文件）。
                while *pos < bytes.len()
                    && bytes[*pos] != b'\n'
                    && !catcodes.get(bytes[*pos]).is_end_of_line()
                {
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
                    // tex.web L7417 `if cat=letter then state:=skip_blanks`：控制词后
                    // 的空格吸收是**惰性**的 skip_blanks 状态（本引擎 InSpace），
                    // 不是读词时就地吞字节。就地吞掉会用**控制词当时的 catcode**
                    // 裁决后随空格；而 TeX 是取下一个 token 时再查表——两者在
                    // 「控制词与后随空格之间发生 `\catcode` 改写」时分歧：
                    //   `\ExplSyntaxOn` 下 cat 32 = Ignored(9)，读得 `\X` 后体内
                    //   `\char_set_catcode_space:n{32}` 改为 Space(10)，再回来读
                    //   行上那个空格——TeX 走 `skip_blanks+spacer`（忽略），旧实现
                    //   已把 state 置回 MidLine → 产空格 token → 定界实参扫描
                    //   "Use of \X doesn't match its definition"（ctex ctexopts.cfg
                    //   `\GetIdInfo`/`\ProvidesExplFile` 自递归根因）。
                    // 物理行尾（LF 字节）由 EndOfLine 臂处理：InSpace 状态下不产
                    // token 且行尾回 LineStart（tex.web `skip_blanks+car_ret` →
                    // "Finish line, goto switch" → 下一行 new_line）——catcode(0x0A)
                    // 被改写为非 cat-5 后（latex.ltx L299 `\catcode`\^^J=\active`）
                    // 行尾字节同样不会漏到主分派。
                    *state = ScanState::InSpace;
                    return Ok(Some(Token::control_sequence(csid)));
                }
                // 控制符号：单个任意非字母字符（含空格、`\` 自身、^^/UTF-8 解码字符）。
                // tex.web L7418-7420：`else if cat=spacer then state:=skip_blanks
                // else state:=mid_line`——`\ `（名字是空格字符的控制符号）同样进
                // skip_blanks（`\ x` 里 `\ ` 后的空格被忽略，真 TeX 只出一个空格）；
                // 其余控制符号（`\%`/`\$`/`\{`…）回 mid_line，后随空格仍有效。
                let csid = intern.intern(&first_char.unwrap_or('\u{FFFD}').to_string());
                *state = match first_char.and_then(|c| u8::try_from(c).ok()) {
                    Some(b) if catcodes.get(b).is_space() => ScanState::InSpace,
                    _ => ScanState::MidLine,
                };
                return Ok(Some(Token::control_sequence(csid)));
            }
            _ => {
                // 字符三路解码 ^^ 转义：catcode 7 的 ^ 后随 ^ → 解码为单个字符 token
                let ch: u32;
                let mut cat = cat;
                if b == b'\n' {
                    // 物理行边界按**字节身份**走行尾，与其 catcode 无关。
                    //
                    // tex.web 锚点（get_next）：行尾字节是按**位置**写入的
                    // `end_line_char`——`@<Read next line of file...@>` 末尾
                    // `if end_line_char_inactive then decr(limit)
                    //  else buffer[limit]:=end_line_char`（L7578-7579），OS 换行
                    // 字节根本不进 buffer；行尾 token 由 end_line_char 的 catcode
                    // 分派（INITEX 下 13→cat 5 → mid_line+car_ret「emit a space」）。
                    // 用户 `\catcode`\^^J=…` 改的是 char **10** 而非
                    // end_line_char(13)，故行尾分派不受影响——latex.ltx L299
                    // `\catcode`\^^J=\active` 正依赖这一点。
                    //
                    // 本引擎行模型以文件内 LF 字节为物理边界（`line_starts` 同按
                    // 字节身份切行），该字节即 tex.web end_line_char 的**位置**
                    // 等价物；若按其可变 catcode 分派，改写 catcode(0x0A) 的代码
                    // 会把每个物理行尾当**数据**扫出：
                    //   - active（latex.ltx L299）→ 未定义 active char token →
                    //     主循环 "! Undefined control sequence."（首现场 l.301，
                    //     `\edef\reserved@a{\expandafter\reserved@a\string^^J\@@}`
                    //     的 TeX 版本嗅探；pdfTeX 对照 INITEX 同段 0 错误）；
                    //   - cat 12（expl3-code L24551 `\char_set_catcode_other:N
                    //     \^^J`）→ char-10 token 静默混入 token 流。
                    cat = Catcode::EndOfLine;
                }
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
                        // tex.web `skip_blanks+car_ret → @<Finish line, |goto switch|@>`
                        //（L7332）：skip_blanks（本引擎 InSpace）状态下行尾**不产
                        // token**，只回 new_line——`\message{x \ny}` 真 TeX 只出一个
                        // 空格（空格 token 已在 mid_line+spacer 臂产出），旧实现再补
                        // 一个行尾空格 token 成两个。
                        if *state == ScanState::InSpace {
                            *state = ScanState::LineStart;
                            continue;
                        }
                        *state = ScanState::LineStart;
                        // 行尾插入字符按其**当前 catcode** 处理（tex.web
                        // end_line_char 语义；本引擎行模型硬编码行尾插入 char 32）。
                        // expl3 把 cat 32 设 Ignored(9)：行尾空格被忽略 → 行边界
                        // 消失——l3names `\def\__kernel_primitive:NN #1#2{` 的参数
                        // 文本因此不含尾随空格；否则 #2 变"空格定界"实参、实参扫描
                        // 一路吞到首个 `}`（l.398/l.819 级联的第三根因，报告 §18）。
                        // cat 32 维持 Space(10) 时行为不变（plain/TRIP/LaTeX）。
                        //
                        // M9 中文刀 7（beamer 活锁根因修复）：行尾插入字符的
                        // **字符码取 `\endlinechar` 参数值**（tex.web
                        // `buffer[limit]:=end_line_char`，L7578-7579），不再硬编码
                        // 32。`\catcode`\^^M=12` 类用法（beamer 逐行消费器
                        // `#1^^M` 定界、`\uppercase` 内活字符构造）要求行尾
                        // token 的字符码 = endlinechar（13）且 catcode 按
                        // **该码位查当前表**——cat 12 时产出数据字符可作宏
                        // 参数定界符。endlinechar < 0（tex.web
                        // end_line_char_inactive）→ 不产出任何 token。
                        // 默认 endlinechar=13 且 cat 13=EndOfLine(5) 时维持
                        // 现行"空格语义"（plain/TRIP/LaTeX 逐字节不变）。
                        if !(0..=0xFF).contains(&endlinechar) {
                            // 不追加：行尾不产 token（继续扫下一物理行）
                            continue;
                        }
                        let elc = endlinechar as u8;
                        match catcodes.get(elc) {
                            Catcode::EndOfLine => {
                                // 行尾字符自身是 cat 5 → 维持空格语义
                                //（字符码仍用 32：定界空格匹配，历史口径）
                                match catcodes.get(b' ') {
                                    Catcode::Ignored => continue,
                                    Catcode::Space => {
                                        return Ok(Some(Token::char(Catcode::Space, b' ' as u32)))
                                    }
                                    other => return Ok(Some(Token::char(other, b' ' as u32))),
                                }
                            }
                            Catcode::Ignored => continue,
                            Catcode::Space => {
                                // tex.web：cat 10（spacer）一律产出**规范空格
                                // token**（`cur_chr:=" "`，与字符码无关）——
                                // `@<Finish line, emit a space@>` 与
                                // `mid_line+spacer` 两臂都是 `cur_chr:=" "`。
                                // 此前用 endlinechar 码位（13），`\@sptoken`
                                // （cat10/char32）的 `\ifx` 判等失配：
                                // `\ProvidesFile` 的 `\@ifnextchar[` 拿不到
                                // `[`、多行可选实参泄漏成正文（l.45 级联）。
                                // 非 mid_line（skip_blanks/new_line+spacer）
                                // 不产 token（tex.web 同名臂忽略）。
                                return Ok(Some(Token::char(Catcode::Space, b' ' as u32)));
                            }
                            other => return Ok(Some(Token::char(other, elc as u32))),
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
                        //
                        // 槽隔离（\u{0}A 前缀，2026-09-18 latex.ltx L15916
                        // `{\catcode`\_=\active \gdef_{\_}}` 自噬根因）：
                        // active char 的 csid **不得**与同名 cs 共槽——真 TeX
                        // 里 active char 住 eqtb active 区（hash 区之下），
                        // 与命名 cs 槽互不可见。旧实现 intern(字符本身)，
                        // `_` 的 active 槽与 cs `\_` 同槽：latex.ltx 用 gdef
                        // 写 active `_` 时把 L10248 的 robust `\_` 覆盖成
                        // 自引用体 → `\textunderscore` 展开撞回 active `_`
                        // 每步自推（small2e 正文 `\_` 5300 万步死循环）。
                        // 前缀 `\u{0}` 不可入 token 流，天然防碰撞（同
                        // `"\u{0}inaccessible"` 先例）；名字显示/trace 取
                        // 首字符后仍输出原字符。
                        *state = ScanState::MidLine;
                        let name = char::from_u32(ch).unwrap_or('\u{FFFD}').to_string();
                        let csid = intern.intern(&format!("\u{0}A{name}"));
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
            13,
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
            13,
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
            13,
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
            13,
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
            13,
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
        assert!(scan_token(
            &bytes,
            &mut pos,
            &catcodes,
            &mut intern,
            &mut state,
            false,
            13
        )
        .is_err());
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
            13,
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
            13,
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

    // ---------- 物理行边界按字节身份（catcode(0x0A) 改写不影响行尾） ----------
    //
    // tex.web get_next：行尾字节是按**位置**写入的 end_line_char
    // （`@<Read next line of file...@>` 末尾 `buffer[limit]:=end_line_char`，
    // L7578-7579），OS 换行字节不进 buffer；`\catcode`\^^J=…` 改的是 char 10
    // 而非 end_line_char(13)，行尾分派（INITEX cat 5 → space/\par）不受影响。
    // pdfTeX ground truth（2026-09-12，INITEX + `\catcode`\^=7`）：
    // latex.ltx L299-302 的 TeX 版本嗅探段（含 `\catcode`\^^J=\active`）执行
    // **0 错误**、`\reserved@a` 成为空宏；本引擎修复前每个物理行尾被扫成
    // 未定义 active char token → 逐行 "! Undefined control sequence."。
    //
    // 对照（latex.ltx L299 之后改写 catcode(0x0A) 的第二处）：
    // expl3-code L24551 `\char_set_catcode_other:N \^^J`（组内）——行尾按
    // catcode 会产出 char-10 **数据** token 静默混入 token 流。

    /// 自定义 catcode 表版 `scan_all`（返回 intern 以便核对 active 名）。
    fn scan_all_with(catcodes: CatcodeTable, src: &str) -> (Vec<Token>, InternTable) {
        let mut intern = InternTable::new();
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
            13,
        )
        .unwrap()
        {
            out.push(t);
        }
        (out, intern)
    }

    #[test]
    fn active_newline_keeps_line_ends_as_line_ends() {
        // `\catcode`\^^J=\active`（latex.ltx L299）后：物理行尾仍是行尾
        // （空格 token，且下一行行首空格照常忽略），**不得**产出 active char
        // token——修复前行尾字节按 catcode 走 active 臂。
        let mut catcodes = CatcodeTable::new();
        catcodes.set(0x0A, Catcode::Active);
        let (toks, _) = scan_all_with(catcodes, "a\n  b\n");
        // a <行尾空格> b <行尾空格>（行首空格被忽略、无任何 active token）
        assert_eq!(toks.len(), 4, "tokens: {toks:?}");
        assert_eq!(toks[0].charcode(), Some(b'a' as u32));
        assert_eq!(toks[1].catcode(), Some(Catcode::Space), "行尾 → 空格");
        assert_eq!(toks[2].charcode(), Some(b'b' as u32), "行首空格被忽略");
        assert_eq!(toks[3].catcode(), Some(Catcode::Space));
        assert!(
            toks.iter().all(|t| !t.is_active()),
            "行尾不得产出 active char token：{toks:?}"
        );
    }

    #[test]
    fn decoded_circumflex_newline_is_active_data() {
        // ^^J 的**解码产物**是数据：catcode(0x0A)=Active 时 `^^J` → active char
        // token（`\string^^J` 的实参、latex.ltx L301 版本嗅探依赖此形态）；
        // 同行随后的物理行尾仍是行尾（空格）——二者不得混淆。
        let mut catcodes = CatcodeTable::new();
        catcodes.set(0x0A, Catcode::Active);
        let (toks, intern) = scan_all_with(catcodes, "^^J\n");
        assert_eq!(toks.len(), 2, "tokens: {toks:?}");
        assert!(toks[0].is_active(), "解码 ^^J → active char：{toks:?}");
        // active 槽名带 `\u{0}A` 前缀（槽隔离，见 Active 臂注释），前缀后是原字符
        assert_eq!(intern.name(toks[0].csid().unwrap()), "\u{0}A\n");
        assert_eq!(toks[1].catcode(), Some(Catcode::Space), "物理行尾 → 空格");
    }

    #[test]
    fn comment_stops_at_physical_line_end_when_newline_recoded() {
        // catcode(0x0A) 非 cat 5（expl3-code L24551 设 cat 12）时注释仍止于
        // 物理行尾——否则一条注释吞掉余下整个文件。
        let mut catcodes = CatcodeTable::new();
        catcodes.set(0x0A, Catcode::Other);
        let (toks, _) = scan_all_with(catcodes, "a% junk\nb\n");
        assert_eq!(toks.len(), 3, "tokens: {toks:?}");
        assert_eq!(toks[0].charcode(), Some(b'a' as u32));
        assert_eq!(toks[1].charcode(), Some(b'b' as u32));
        assert_eq!(toks[2].catcode(), Some(Catcode::Space));
    }

    #[test]
    fn control_word_trailing_newline_yields_no_token_when_recoded() {
        // 控制词行尾：tex.web `skip_blanks+car_ret` → 无 token；catcode(0x0A)
        // 被改写后行尾字节不得漏到主分派（修复前 → active char token）。
        let mut catcodes = CatcodeTable::new();
        catcodes.set(0x0A, Catcode::Active);
        let (toks, _) = scan_all_with(catcodes, "\\foo\nb\n");
        assert_eq!(toks.len(), 3, "tokens: {toks:?}");
        assert_eq!(toks[0].kind(), TokenKind::ControlSeq);
        assert_eq!(toks[1].charcode(), Some(b'b' as u32));
        assert_eq!(toks[2].catcode(), Some(Catcode::Space));
        assert!(toks.iter().all(|t| !t.is_active()), "tokens: {toks:?}");
    }
}
