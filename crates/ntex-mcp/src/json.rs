//! 最小 JSON 值模型 + 解析/序列化（M9 形态①）。
//!
//! MCP 协议最小实现约定不引第三方 SDK：JSON-RPC 2.0 帧只需对象/数组/字符串/
//! 数字/布尔/null 的解析与序列化。数字统一按 f64 携带；JSON-RPC 的 id 只使用
//! 字符串或整数（规范允许），其余类型按协议错误处理。
//!
//! RFC 8259 对齐点：字符串转义覆盖引号、反斜杠与 0x00-0x1F 控制字符（b/f/n/r/t
//! 短转义及 uXXXX）；解析接受全部合法转义与代理对。

use std::collections::BTreeMap;
use std::fmt;

/// JSON 值（对象用 BTreeMap：键序稳定输出，够用且免依赖）。
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    /// 构造对象（键值列表；BTreeMap 负责去重与键序）。
    pub fn object(fields: Vec<(&str, Json)>) -> Json {
        Json::Object(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }

    /// 取对象字段（不存在返回 Null，调用方免判型分支）。
    pub fn get(&self, key: &str) -> &Json {
        match self {
            Json::Object(map) => map.get(key).unwrap_or(&Json::Null),
            _ => &Json::Null,
        }
    }

    /// JSON 字符串序列化（含引号与转义）。
    pub fn escape_string(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for ch in s.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\u{08}' => out.push_str("\\b"),
                '\u{0C}' => out.push_str("\\f"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                // 非 BMP 字符：JSON 文本无原生表示，按规范拆为代理对 u 转义。
                c if (c as u32) > 0xFFFF => {
                    let code = c as u32 - 0x1_0000;
                    let hi = 0xD800 + (code >> 10);
                    let lo = 0xDC00 + (code & 0x3FF);
                    out.push_str(&format!("\\u{:04x}\\u{:04x}", hi, lo));
                }
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Json::Null => f.write_str("null"),
            Json::Bool(true) => f.write_str("true"),
            Json::Bool(false) => f.write_str("false"),
            Json::Number(n) => {
                // JSON 禁 NaN/Inf；整值输出整型字面量（id 场景友好）。
                if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
                    write!(f, "{}", *n as i64)
                } else if n.is_finite() {
                    write!(f, "{n}")
                } else {
                    f.write_str("null")
                }
            }
            Json::String(s) => f.write_str(&Json::escape_string(s)),
            Json::Array(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("]")
            }
            Json::Object(map) => {
                f.write_str("{")?;
                for (i, (key, value)) in map.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{}:{value}", Json::escape_string(key))?;
                }
                f.write_str("}")
            }
        }
    }
}

/// JSON 解析错误（带字节偏移，便于定位畸形输入）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JSON 解析失败（偏移 {}）：{}", self.offset, self.message)
    }
}

impl std::error::Error for ParseError {}

/// 解析一段 JSON 文本（允许首尾空白）。
pub fn parse(text: &str) -> Result<Json, ParseError> {
    let mut p = Parser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    p.skip_ws();
    let value = p.parse_value()?;
    p.skip_ws();
    if p.pos != p.bytes.len() {
        return Err(p.err("尾部存在多余内容"));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn err(&self, message: &str) -> ParseError {
        ParseError {
            offset: self.pos,
            message: message.to_owned(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, b: u8) -> Result<(), ParseError> {
        if self.peek() == Some(b) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(&format!("期望 '{}'", b as char)))
        }
    }

    fn parse_value(&mut self) -> Result<Json, ParseError> {
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => Ok(Json::String(self.parse_string()?)),
            Some(b't') => self.parse_literal("true", Json::Bool(true)),
            Some(b'f') => self.parse_literal("false", Json::Bool(false)),
            Some(b'n') => self.parse_literal("null", Json::Null),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.parse_number(),
            _ => Err(self.err("期望 JSON 值")),
        }
    }

    fn parse_literal(&mut self, lit: &str, value: Json) -> Result<Json, ParseError> {
        if self.bytes[self.pos..].starts_with(lit.as_bytes()) {
            self.pos += lit.len();
            Ok(value)
        } else {
            Err(self.err("字面量不完整"))
        }
    }

    fn parse_object(&mut self) -> Result<Json, ParseError> {
        self.expect(b'{')?;
        let mut map = BTreeMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(map));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let value = self.parse_value()?;
            map.insert(key, value);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(map));
                }
                _ => return Err(self.err("对象缺少 ',' 或 '}'")),
            }
        }
    }

    fn parse_array(&mut self) -> Result<Json, ParseError> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.err("数组缺少 ',' 或 ']'")),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, ParseError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let Some(b) = self.peek() else {
                return Err(self.err("字符串未闭合"));
            };
            self.pos += 1;
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    let Some(esc) = self.peek() else {
                        return Err(self.err("转义序列不完整"));
                    };
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.parse_hex4()?;
                            let ch = if (0xD800..0xDC00).contains(&hi) {
                                // 代理对：必须紧跟低半区 u 转义。
                                if self.bytes[self.pos..].starts_with(b"\\u") {
                                    self.pos += 2;
                                    let lo = self.parse_hex4()?;
                                    if !(0xDC00..0xE000).contains(&lo) {
                                        return Err(self.err("非法低位代理"));
                                    }
                                    let code = 0x1_0000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                                    char::from_u32(code).ok_or_else(|| self.err("非法码点"))?
                                } else {
                                    return Err(self.err("孤立高位代理"));
                                }
                            } else {
                                char::from_u32(hi).ok_or_else(|| self.err("非法码点"))?
                            };
                            out.push(ch);
                        }
                        _ => return Err(self.err("非法转义字符")),
                    }
                }
                0x00..=0x1F => return Err(self.err("字符串含未转义控制字符")),
                _ => {
                    let start = self.pos - 1;
                    let end = start + utf8_len(b);
                    if end > self.bytes.len() {
                        return Err(self.err("UTF-8 序列不完整"));
                    }
                    self.pos = end;
                    match std::str::from_utf8(&self.bytes[start..end]) {
                        Ok(s) => out.push_str(s),
                        Err(_) => return Err(self.err("非法 UTF-8 序列")),
                    }
                }
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, ParseError> {
        if self.pos + 4 > self.bytes.len() {
            return Err(self.err("u 转义不完整"));
        }
        let mut v: u32 = 0;
        for _ in 0..4 {
            let b = self.bytes[self.pos];
            let digit = match b {
                b'0'..=b'9' => u32::from(b - b'0'),
                b'a'..=b'f' => u32::from(b - b'a') + 10,
                b'A'..=b'F' => u32::from(b - b'A') + 10,
                _ => return Err(self.err("u 转义需 4 位十六进制")),
            };
            self.pos += 1;
            v = v * 16 + digit;
        }
        Ok(v)
    }

    fn parse_number(&mut self) -> Result<Json, ParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        // 整数部分：单个 0，或非零开头的数字串。
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(c) if c.is_ascii_digit() => {
                while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.err("数字格式非法")),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.err("小数点后需数字"));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.err("指数部分需数字"));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| self.err("数字字节非法"))?;
        text.parse::<f64>()
            .map(Json::Number)
            .map_err(|_| self.err("数字超出范围"))
    }
}

/// 由首字节推 UTF-8 序列长度（输入本为 &str，仅用于字节回抄定位）。
fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_object_array_string() {
        let src = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":7,",
            "\"params\":{\"n\":-1.5,\"a\":[true,null,\"x\"]}}"
        );
        let value = parse(src).expect("解析");
        assert_eq!(value.get("jsonrpc"), &Json::String("2.0".into()));
        assert_eq!(value.get("id"), &Json::Number(7.0));
        let rendered = value.to_string();
        let again = parse(&rendered).expect("再解析");
        assert_eq!(value, again);
    }

    #[test]
    fn escapes_and_unicode() {
        let value = Json::String("quote\"back\\slash\n\u{1F600}".into());
        let text = value.to_string();
        assert!(text.contains("\\n"), "newline short escape: {text}");
        assert!(text.contains("\\u"), "hex escape: {text}");
        assert_eq!(parse(&text).expect("parse"), value);
    }

    #[test]
    fn rejects_malformed() {
        let cases = ["{", "[1,]", "{\"a\":}", "nul", "1.2.3", "\"\\ud800\""];
        for src in cases {
            assert!(parse(src).is_err(), "应拒绝：{src}");
        }
    }
}
