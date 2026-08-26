// ---------- 自由函数 ----------

/// TRIP 冲刺：TeX initex 默认 mathcode 表（tex.web `init_math_codes`）：
/// catcode 11/12（letter/other_char）→ `0x7000+码`（class 7 variable、family 0、
/// 字符码本身），其余 → `0x8000`（无效，触发 "Missing character"）。
///
/// initex 初始 catcode：A-Z/a-z = 11；数字/标点/非 ASCII = 12；
/// 排除特殊字符（0=ignored、tab/CR=end_line/space、`\ { } $ # ^ _ ~ %`）。
fn default_mathcodes() -> HashMap<u32, u32> {
    // 非 catcode 11/12 的码点（tex.web `@<Initialize the catcode tables@>`）。
    const NON_LETTER_OTHER: [u32; 13] = [
        0x00, 0x09, 0x0D, 0x20, 0x23, 0x24, 0x25, 0x5C, 0x5E, 0x5F, 0x7B, 0x7D, 0x7E,
    ];
    let mut m = HashMap::with_capacity(256);
    for k in 0..=255u32 {
        let v = if NON_LETTER_OTHER.contains(&k) { 0x8000 } else { 0x7000 + k };
        m.insert(k, v);
    }
    m
}

/// 宏体实参替换：`#n` → 第 n 个实参（整段借用，零拷贝）。
fn materialize(body: &[Token], args: &[TokenArray]) -> Vec<Token> {
    let mut out = Vec::with_capacity(body.len());
    for &t in body {
        if let Some(n) = t.param_number() {
            if let Some(arg) = args.get(n.saturating_sub(1) as usize) {
                out.extend_from_slice(arg);
            }
        } else {
            out.push(t);
        }
    }
    out
}

/// token 是否为参数符 `#`（cat 6）。
fn is_parameter_char(tok: Token) -> bool {
    tok.catcode() == Some(Catcode::Parameter)
}

/// TeX/e-TeX 内部整数参数 → `Params.misc` 数组下标（与 [`param::default_misc`] 对齐）。
fn int_param_index(p: Primitive) -> Option<usize> {
    Some(match p {
        Primitive::TracingStats => 0,
        Primitive::TracingLostChars => 1,
        Primitive::TracingOnline => 2,
        Primitive::TracingCommands => 3,
        Primitive::TracingRestores => 4,
        Primitive::TracingAssigns => 5,
        Primitive::TracingGroups => 6,
        Primitive::TracingIfs => 7,
        Primitive::TracingScantokens => 8,
        Primitive::TracingNesting => 9,
        Primitive::LeftHyphenMin => 10,
        Primitive::RightHyphenMin => 11,
        Primitive::HBadness => 12,
        Primitive::PreTolerance => 13,
        Primitive::ShowBoxDepth => 14,
        Primitive::ShowBoxBreadth => 15,
        Primitive::Language => 16,
        Primitive::SavingHyphCodes => 17,
        Primitive::SavingVDiscards => 18,
        Primitive::InteractionMode => 19,
        Primitive::TeXXeTState => 20,
        Primitive::MathSurround => 21,
        Primitive::LastLineFit => 22,
        Primitive::PredisplayDirection => 23,
        Primitive::EveryEof => 24,
        Primitive::DeadCycles => 25,
        Primitive::TracingMacros => 26,
        Primitive::TracingOutput => 27,
        Primitive::ErrorContextLines => 28,
        Primitive::TracingParagraphs => 29,
        Primitive::PageDiscards => 30,
        Primitive::SplitDiscards => 31,
        Primitive::LostChars => 32,
        // TRIP 冲刺：\delimiterfactor（misc 33；plain 默认 901）
        Primitive::DelimiterFactor => 33,
        // TRIP 冲刺：\escapechar（misc 34；initex 默认 92 = `\`）
        Primitive::EscapeChar => 34,
        // TRIP 冲刺：内部整数参数（misc 35-50；默认值见 param::default_misc）
        Primitive::VBadness => 35,
        Primitive::GlobalDefs => 36,
        Primitive::FloatingPenalty => 37,
        Primitive::LinePenalty => 38,
        Primitive::BinoPenalty => 39,
        Primitive::RelPenalty => 40,
        Primitive::AdjDemerits => 41,
        Primitive::Looseness => 42,
        Primitive::MaxDeadCycles => 43,
        Primitive::HangAfter => 44,
        Primitive::Uchyph => 45,
        Primitive::Fam => 46,
        Primitive::HyphenPenalty => 47,
        Primitive::DoubleHyphenDemerits => 48,
        Primitive::FinalHyphenDemerits => 49,
        Primitive::HoldingInserts => 50,
        _ => return None,
    })
}

/// 交互模式命令 → interactionmode 值（TeX：0=batch,1=nonstop,2=scroll,3=errorstop）。
fn interaction_mode_value(p: Primitive) -> Option<i64> {
    Some(match p {
        Primitive::BatchMode => 0,
        Primitive::NonstopMode => 1,
        Primitive::ScrollMode => 2,
        Primitive::ErrorStopMode => 3,
        _ => return None,
    })
}

/// 字符 token 是否为十进制数字；返回数字值。
fn digit_value(tok: Token) -> Option<u8> {
    let ch = tok.charcode()? as u8;
    if ch.is_ascii_digit() {
        Some(ch - b'0')
    } else {
        None
    }
}

/// 指定基数下的数字值（`radix_digit_value`）：0-9、a-f/A-F，超基数返回 `None`。
fn radix_digit_value(tok: Token, base: u32) -> Option<u32> {
    let c = tok.charcode()?;
    let d = match c {
        0x30..=0x39 => c - 0x30,
        0x61..=0x66 => c - 0x61 + 10,
        0x41..=0x46 => c - 0x41 + 10,
        _ => return None,
    };
    if d < base {
        Some(d)
    } else {
        None
    }
}

/// 整数 → `\the` token 序列（十进制字符，cat 12）。
fn emit_count(v: i64) -> Vec<Token> {
    format_count(v)
        .bytes()
        .map(|b| Token::char(Catcode::Other, u32::from(b)))
        .collect()
}

/// 尺寸 → `\the` token 序列（如 "2.5pt"）。
fn emit_dimen(v: i64) -> Vec<Token> {
    let mut s = format_dimen(v);
    s.push_str("pt");
    s.bytes()
        .map(|b| Token::char(Catcode::Other, u32::from(b)))
        .collect()
}

/// 胶水 → `\the` token 序列（如 "1.0pt plus 2.0pt minus 0.5pt"）。
fn emit_glue(g: Glue) -> Vec<Token> {
    format_glue(g)
        .bytes()
        .map(|b| Token::char(Catcode::Other, u32::from(b)))
        .collect()
}

/// mu 胶水 → `\the` token 序列（如 "1.0mu plus 2.0mu minus 0.5mu"）。
fn emit_mu_glue(g: Glue) -> Vec<Token> {
    format_mu_glue(g)
        .bytes()
        .map(|b| Token::char(Catcode::Other, u32::from(b)))
        .collect()
}

/// 寄存器种类名（`\show` 显示用）。
fn reg_kind_name(k: RegKind) -> &'static str {
    match k {
        RegKind::Count => "count",
        RegKind::Dimen => "dimen",
        RegKind::Skip => "skip",
        RegKind::Muskip => "muskip",
        RegKind::Toks => "toks",
    }
}

/// token 列表 → 文本（`\show` 宏体/`\showthe` 值显示用）：
/// 字符取字符、控制序列 → `\名字`、宏参数 → `#n`。
fn detok_tokens(toks: &[Token], intern: &InternTable) -> String {
    let mut s = String::new();
    for t in toks {
        match t.kind() {
            TokenKind::Char => {
                if let Some(ch) = t.charcode().and_then(char::from_u32) {
                    s.push(ch);
                }
            }
            TokenKind::ControlSeq => {
                s.push('\\');
                s.push_str(intern.name(t.csid().expect("ControlSeq 必有 csid")));
            }
            TokenKind::MacroParam => {
                s.push('#');
                s.push_str(&t.param_number().expect("MacroParam 必有参数号").to_string());
            }
            TokenKind::EndGroup => {}
        }
    }
    s
}

/// 按关系符比较两个内部量。
fn compare(a: i64, b: i64, rel: Relation) -> bool {
    match rel {
        Relation::Lt => a < b,
        Relation::Eq => a == b,
        Relation::Gt => a > b,
    }
}

/// `\detokenize` 单 token 转换：字符 → catcode 12（空格 10）；控制序列 → `\名字`。
/// e-TeX：控制词（名字以字母开头）后补一个空格分隔符（TeX `\detokenize{a\relax b}`
/// 输出 "a\relax b"——控制词后的空格 token 已被扫描吞掉）。
fn detokenize_token(tok: Token, intern: &InternTable, out: &mut Vec<Token>) {
    match tok.kind() {
        TokenKind::Char => {
            let ch = tok.charcode().expect("Char 必有 charcode");
            let cat = if ch == b' ' as u32 {
                Catcode::Space
            } else {
                Catcode::Other
            };
            out.push(Token::char(cat, ch));
        }
        TokenKind::ControlSeq => {
            let name = intern.name(tok.csid().expect("ControlSeq 必有 csid"));
            out.push(Token::char(Catcode::Other, u32::from(b'\\')));
            for b in name.bytes() {
                out.push(Token::char(Catcode::Other, u32::from(b)));
            }
            if name.bytes().next().is_some_and(|c| c.is_ascii_alphabetic()) {
                out.push(Token::char(Catcode::Space, u32::from(b' ')));
            }
        }
        TokenKind::MacroParam => {
            let n = tok.param_number().unwrap_or(0);
            out.push(Token::char(Catcode::Other, u32::from(b'#')));
            out.push(Token::char(Catcode::Other, u32::from(b'0' + n)));
        }
        TokenKind::EndGroup => out.push(Token::char(Catcode::Other, u32::from(b'}'))),
    }
}
