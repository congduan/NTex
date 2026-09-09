// ---------- 自由函数 ----------

/// TRIP 冲刺：TeX initex 默认 mathcode 表（tex.web `init_math_codes`；
/// plain.tex L49-51 记载）：**字母 A-Z/a-z → `0x7100+码`**（class 7
/// variable、**family 1**——数学斜体 cmmi，demo `$E=mc^2$` 斜体来源）、
/// 数字/其余 catcode 11/12 → `0x7000+码`（class 7 variable、family 0），
/// 非 11/12 码点 → `0x8000`（无效，触发 "Missing character"）。
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
        let v = if NON_LETTER_OTHER.contains(&k) {
            0x8000
        } else if matches!(k, 0x41..=0x5A | 0x61..=0x7A) {
            0x7100 + k
        } else {
            0x7000 + k
        };
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
        // 21 曾是 \mathsurround——已改为 dimen 参数（param.rs MathSurround），
        // 槽位保留占位，避免扰动后续整数参数下标。
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
        // TRIP 冲刺：\prevgraf（上一段落行数，只读内部量；plain 默认 0）
        Primitive::PrevGraf => 51,
        Primitive::InsertPenalties => 52,
        // TRIP 补全批次：日期时间与参数类（53-62）
        Primitive::Day => 53,
        Primitive::Month => 54,
        Primitive::Year => 55,
        Primitive::Time => 56,
        Primitive::BrokenPenalty => 57,
        Primitive::ExHyphenPenalty => 58,
        Primitive::TracingPages => 59,
        Primitive::Pausing => 60,
        Primitive::SetLanguage => 61,
        Primitive::OutputPenalty => 62,
        // LaTeX 兼容第八刀：\pdfoutput（misc 63，pdfTeX 默认 0 = DVI 模式）。
        // 可写内部整数参数——expl3 `\c_sys_output_str`/`\c_sys_engine_format_str`
        // 以 `\tex_pdfoutput:D` 作数字读取，graphicx/hyperref 以 `\pdfoutput=` 赋值。
        // （\pdfrandomseed 用 misc 64 但**不**入此表：pdfTeX 中它只读，
        //  写入走 \pdfsetrandomseed。）
        Primitive::PdfOutput => 63,
        // M9 中文刀 2：\utfinputmode（misc 65；源文件输入编码开关，0=bytes 1=utf8）
        Primitive::UtfInputMode => crate::param::MISC_UTF_INPUT_MODE,
        _ => return None,
    })
}

/// pdfTeX 兼容原语的只读整数/种子值（misc 64 = \pdfrandomseed 状态）。
pub(crate) const PDF_RANDOM_SEED_IDX: usize = 64;

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

/// 内部参数原语 → [`ParamKind`]（G3：`\advance/\multiply/\divide` 目标判定与
/// `\the` 读取共用的一张表）。tex.web `do_register_command` 的目标集合是
/// assign_int/assign_dimen/assign_glue/assign_mu_glue 四个 eqtb 区——`\hsize/
/// \vsize/\voffset` 等页面参数、`\baselineskip/\parskip` 等胶参数、`\tolerance`
/// 等整数参数都在其列；内部整数（misc 数组）走 [`int_param_index`]，
/// `\xspaceskip`/`\prevdepth` 在 exec_advance 有专属臂（TRIP L410/L434），
/// 均不入此表。
fn param_kind_of(p: Primitive) -> Option<ParamKind> {
    Some(match p {
        Primitive::ParIndent => ParamKind::ParIndent,
        Primitive::LineSkipLimit => ParamKind::LineSkipLimit,
        Primitive::BaselineSkip => ParamKind::BaselineSkip,
        Primitive::LineSkip => ParamKind::LineSkip,
        Primitive::HSize => ParamKind::HSize,
        Primitive::Tolerance => ParamKind::Tolerance,
        Primitive::VSize => ParamKind::VSize,
        Primitive::TopSkip => ParamKind::TopSkip,
        Primitive::MaxDepth => ParamKind::MaxDepth,
        Primitive::ParSkip => ParamKind::ParSkip,
        Primitive::ParFillSkip => ParamKind::ParFillSkip,
        Primitive::AboveDisplaySkip => ParamKind::AboveDisplaySkip,
        Primitive::BelowDisplaySkip => ParamKind::BelowDisplaySkip,
        Primitive::AboveDisplayShortSkip => ParamKind::AboveDisplayShortSkip,
        Primitive::BelowDisplayShortSkip => ParamKind::BelowDisplayShortSkip,
        Primitive::PreDisplayPenalty => ParamKind::PreDisplayPenalty,
        Primitive::PostDisplayPenalty => ParamKind::PostDisplayPenalty,
        Primitive::LeftSkip => ParamKind::LeftSkip,
        Primitive::RightSkip => ParamKind::RightSkip,
        Primitive::HangIndent => ParamKind::HangIndent,
        Primitive::SpaceSkip => ParamKind::SpaceSkip,
        Primitive::TabSkip => ParamKind::TabSkip,
        Primitive::LastSkip => ParamKind::LastSkip,
        Primitive::SplitTopSkip => ParamKind::SplitTopSkip,
        Primitive::PageStretch => ParamKind::PageStretch,
        Primitive::PageFilStretch => ParamKind::PageFilStretch,
        Primitive::PageFillStretch => ParamKind::PageFillStretch,
        Primitive::Hfuzz => ParamKind::Hfuzz,
        Primitive::Vfuzz => ParamKind::Vfuzz,
        Primitive::BoxMaxDepth => ParamKind::BoxMaxDepth,
        Primitive::SplitMaxDepth => ParamKind::SplitMaxDepth,
        Primitive::EmergencyStretch => ParamKind::EmergencyStretch,
        Primitive::DisplayIndent => ParamKind::DisplayIndent,
        Primitive::DelimiterShortfall => ParamKind::DelimiterShortfall,
        Primitive::MathSurround => ParamKind::MathSurround,
        Primitive::LastKern => ParamKind::LastKern,
        Primitive::InterLinePenalty => ParamKind::InterLinePenalty,
        Primitive::ClubPenalty => ParamKind::ClubPenalty,
        Primitive::WidowPenalty => ParamKind::WidowPenalty,
        Primitive::DisplayWidowPenalty => ParamKind::DisplayWidowPenalty,
        Primitive::NullDelimiterSpace => ParamKind::NullDelimiterSpace,
        Primitive::ScriptSpace => ParamKind::ScriptSpace,
        Primitive::OverfullRule => ParamKind::OverfullRule,
        Primitive::VOffset => ParamKind::VOffset,
        Primitive::HOffset => ParamKind::HOffset,
        Primitive::EndlineChar => ParamKind::EndlineChar,
        Primitive::NewlineChar => ParamKind::NewlineChar,
        Primitive::DefaultHyphenChar => ParamKind::DefaultHyphenChar,
        Primitive::DefaultSkewChar => ParamKind::DefaultSkewChar,
        Primitive::Mag => ParamKind::Mag,
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

/// 文本 → 字符 token 序列（cat 12；pdfTeX 可展开字符串量的输出形式）。
fn pdf_text_tokens(s: &str) -> Vec<Token> {
    s.bytes()
        .map(|b| Token::char(Catcode::Other, u32::from(b)))
        .collect()
}

/// `\pdftexbanner`：pdfTeX 兼容层的 banner。
///
/// 版本段与 [\pdfTeX 探测原语值](Primitive::PdfTeXVersion) 对齐（1.40.25），
/// 但保留 "NTex" 标识：banner 会进日志/PDF 元数据，不冒充真 pdfTeX 产物。
pub(crate) const PDF_BANNER: &str = "This is pdfTeX, Version 1.40.25 (NTex pdfTeX compatibility layer)";

fn pdf_banner_tokens() -> Vec<Token> {
    pdf_text_tokens(PDF_BANNER)
}

/// `\pdfcreationdate` → `D:YYYYMMDDHHMMSSZ'00'`（pdfTeX PDF 时间串格式）。
///
/// 从 TeX 日期参数（\day/\month/\year/\time，misc 53-56）取值：引擎内已按
/// 系统时钟初始化。`\time` 只有分钟精度，秒恒写 `00`（偏差见报告 §15.3）。
fn pdf_creation_date_tokens(day: i64, month: i64, year: i64, minutes: i64) -> Vec<Token> {
    let hour = minutes / 60;
    let min = minutes % 60;
    let year = year.clamp(0, 9999);
    pdf_text_tokens(&format!(
        "D:{year:04}{month:02}{day:02}{hour:02}{min:02}00Z'00'"
    ))
}

/// `\pdfstrcmp`：两 token 串的字符串比较（detokenize 同规则转字节）→ -1/0/1。
fn pdf_strcmp_value(a: &[Token], b: &[Token], intern: &InternTable, esc: i64) -> i64 {
    let sa = pdf_detokenize_bytes(a, intern, esc);
    let sb = pdf_detokenize_bytes(b, intern, esc);
    match sa.cmp(&sb) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// token 串 → 字节串（与 `\detokenize` 同一转换，再取字符码）。
fn pdf_detokenize_bytes(toks: &[Token], intern: &InternTable, esc: i64) -> Vec<u8> {
    let mut text = Vec::new();
    for t in toks {
        detokenize_token(*t, intern, esc, &mut text);
    }
    text.iter()
        .filter_map(|t| t.charcode().and_then(|c| u8::try_from(c).ok()))
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
///
/// 转义字符取 `\escapechar`（`esc`）：tex.web `print_esc` 只在 `0<=esc<256` 时打印
/// 转义字符（负数 / 256 / >255 一律不可见）——plain `\newif` 的
/// `\expandafter\if@\string\iffoo` 依赖 `\escapechar=-1` 时 `\string` 不带前导 `\`
/// （`\if@` 的 "if" 定界才匹配得上）。旧实现硬编码 `\` 使 -1 失效。
fn detokenize_token(tok: Token, intern: &InternTable, esc: i64, out: &mut Vec<Token>) {
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
            if (0..=255).contains(&esc) {
                out.push(Token::char(Catcode::Other, esc as u32));
            }
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

/// `\string` 单 token 转换：与 [`detokenize_token`] 同一转换，但控制序列名后
/// **不**补尾随空格——tex.web `convert` 的 `string_code` 用 `sprint_cs`
/// （"never prints a space after the control sequence"，L5622-5627），只有
/// e-TeX `\detokenize` 才补空格。expl3 `\cs_to_str:N` 依赖此：`\string\cs_if_exist:N`
/// = `\cs_if_exist:N`（无空格），cs_split 的签名组才是干净 `{N}`；若误带空格，
/// p 型条件生成器的 csname `\cs_if_exist:NTF` 会变成含空格的 `\cs_if_exist:N TF`，
/// 后续 `\cs_if_exist:NTF` 全部 Undefined control sequence。
///
/// 转义字符同 [`detokenize_token`]：`esc`（`\escapechar`）仅在 `0..=255` 内打印。
fn string_token(tok: Token, intern: &InternTable, esc: i64, out: &mut Vec<Token>) {
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
            if (0..=255).contains(&esc) {
                out.push(Token::char(Catcode::Other, esc as u32));
            }
            for b in name.bytes() {
                out.push(Token::char(Catcode::Other, u32::from(b)));
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
/// 主题分类：原语是否属于"排版"主题（dispatch_box）。
/// 含盒子/胶水/kern/penalty/rule/leaders/control space/inf glue/italic correction。
fn is_box_prim(p: Primitive) -> bool {
    matches!(
        p,
        Primitive::HBox
            | Primitive::VBox
            | Primitive::VTop
            | Primitive::Par
            | Primitive::HSkip
            | Primitive::VSkip
            | Primitive::Kern
            | Primitive::Penalty
            | Primitive::HRule
            | Primitive::VRule
            | Primitive::Leaders
            | Primitive::Cleaders
            | Primitive::XLeaders
            | Primitive::Indent
            | Primitive::NoIndent
            | Primitive::ControlSpace
            | Primitive::HFil
            | Primitive::HFill
            | Primitive::HSS
            | Primitive::VFil
            | Primitive::VFill
            | Primitive::VSS
            | Primitive::VFilNeg
            | Primitive::HFilNeg
            | Primitive::ItalicCorrection
    )
}

/// 主题分类：原语是否属于"内部参数"主题（dispatch_param）。
fn is_param_prim(p: Primitive) -> bool {
    matches!(
        p,
        Primitive::ParIndent
            | Primitive::LineSkipLimit
            | Primitive::NullDelimiterSpace
            | Primitive::ScriptSpace
            | Primitive::OverfullRule
            | Primitive::VOffset
            | Primitive::HOffset
            | Primitive::BaselineSkip
            | Primitive::LineSkip
            | Primitive::HSize
            | Primitive::Tolerance
            | Primitive::VSize
            | Primitive::MaxDepth
            | Primitive::TopSkip
            | Primitive::ParSkip
            | Primitive::ParFillSkip
            | Primitive::XSpaceSkip
            | Primitive::AboveDisplaySkip
            | Primitive::BelowDisplaySkip
            | Primitive::AboveDisplayShortSkip
            | Primitive::BelowDisplayShortSkip
            | Primitive::PreDisplayPenalty
            | Primitive::PostDisplayPenalty
            | Primitive::EndlineChar
            | Primitive::NewlineChar
            | Primitive::DefaultHyphenChar
            | Primitive::DefaultSkewChar
            | Primitive::Mag
            | Primitive::LeftSkip
            | Primitive::RightSkip
            | Primitive::PrevDepth
            | Primitive::HangIndent
            | Primitive::SpaceSkip
            | Primitive::TabSkip
            | Primitive::LastSkip
            | Primitive::SplitTopSkip
            | Primitive::PageStretch
            | Primitive::PageFilStretch
            | Primitive::PageFillStretch
            | Primitive::Hfuzz
            | Primitive::Vfuzz
            | Primitive::BoxMaxDepth
            | Primitive::SplitMaxDepth
            | Primitive::EmergencyStretch
            | Primitive::DisplayIndent
            | Primitive::DelimiterShortfall
            | Primitive::MathSurround
            | Primitive::LastKern
            | Primitive::InterLinePenalty
            | Primitive::ClubPenalty
            | Primitive::WidowPenalty
            | Primitive::DisplayWidowPenalty
            | Primitive::InterLinePenalties
            | Primitive::ClubPenalties
            | Primitive::WidowPenalties
            | Primitive::DisplayWidowPenalties
    ) || int_param_index(p).is_some()
        || interaction_mode_value(p).is_some()
}

/// 主题分类：原语是否属于"数学"主题（dispatch_math）。
fn is_math_prim(p: Primitive) -> bool {
    matches!(
        p,
        Primitive::DisplayStyle
            | Primitive::TextStyle
            | Primitive::ScriptStyle
            | Primitive::ScriptScriptStyle
            | Primitive::Over
            | Primitive::Atop
            | Primitive::Left
            | Primitive::Right
            | Primitive::Middle
            | Primitive::Above
            | Primitive::AboveWithDelims
            | Primitive::AtopWithDelims
            | Primitive::OverWithDelims
            | Primitive::Underline
            | Primitive::Overline
            | Primitive::Raise
            | Primitive::Lower
            | Primitive::MoveLeft
            | Primitive::MoveRight
            | Primitive::Accent
            | Primitive::Sqrt
            | Primitive::VCenter
            | Primitive::MathOrd
            | Primitive::MathBin
            | Primitive::MathOp
            | Primitive::MathRel
            | Primitive::MathOpen
            | Primitive::MathClose
            | Primitive::MathPunct
            | Primitive::MathInner
            | Primitive::Nonscript
            | Primitive::Limits
            | Primitive::NoLimits
            | Primitive::DisplayLimits
            | Primitive::NoBoundary
            | Primitive::MSkip
            | Primitive::MKern
            | Primitive::MathAccent
            | Primitive::MathChar
            | Primitive::Delimiter
            | Primitive::EqNo
            | Primitive::LeqNo
            | Primitive::Radical
    )
}

/// 主题分类：原语是否属于"可展开"主题（dispatch_expandable）。
fn is_expandable_prim(p: Primitive) -> bool {
    matches!(
        p,
        Primitive::NumExpr
            | Primitive::Dimexpr
            | Primitive::Glueexpr
            | Primitive::Muexpr
            | Primitive::Number
            | Primitive::ETeXVersion
            | Primitive::ETeXRevision
            // LaTeX 兼容第八刀：pdfTeX 可展开族（带参的 PdfStrCmp/PdfFileSize/
            // PdfUniformDeviate 与字符串量 PdfTeXBanner/PdfCreationDate；
            // 只读整数 PdfTeXVersion 等仿 \eTeXversion 单独出现时展开为数字——
            // 否则 exec 主循环报"未接入 dispatcher"，\numexpr 语境不可用）。
            // 注意：进此白名单的原语必须在 expr.rs expand_once 与
            // primitive_expand.rs dispatch_expandable 都有分支——否则
            // "可展开 → 展开后重试"循环空转（见 expr.rs fuzz 挂死修复注释）。
            | Primitive::PdfTeXVersion
            | Primitive::PdfTeXRevision
            | Primitive::PdfShellEscape
            | Primitive::PdfElapsedTime
            | Primitive::PdfRandomSeed
            | Primitive::PdfTeXBanner
            | Primitive::PdfCreationDate
            | Primitive::PdfStrCmp
            | Primitive::PdfFileSize
            | Primitive::PdfUniformDeviate
            | Primitive::String_
            | Primitive::InputLineNo
            | Primitive::CurrentGroupLevel
            | Primitive::CurrentGroupType
            | Primitive::LastNodeType
            | Primitive::CurrentIfLevel
            | Primitive::CurrentIfType
            | Primitive::CurrentIfBranch
            | Primitive::PageTotal
            | Primitive::PageGoal
            | Primitive::PreDisplaySize
            | Primitive::InsertPenalties
            | Primitive::GlueStretchOrder
            | Primitive::GlueShrinkOrder
            | Primitive::GlueStretch
            | Primitive::GlueShrink
            | Primitive::LastPenalty
            | Primitive::MuToGlue
            | Primitive::GlueToMu
            | Primitive::Meaning
            | Primitive::JobName
            | Primitive::Csname
            | Primitive::EndCsname
            | Primitive::Protected
            | Primitive::Unless
            | Primitive::Scantokens
            | Primitive::Detokenize
            | Primitive::Unexpanded
            | Primitive::ErrMessage
    )
}

/// 主题分类：原语是否属于"对齐与特殊节点"主题（dispatch_align）。
fn is_align_prim(p: Primitive) -> bool {
    matches!(
        p,
        Primitive::Valign
            | Primitive::Halign
            | Primitive::NoAlign
            | Primitive::Cr
            | Primitive::CrCr
            | Primitive::MathChoice
            | Primitive::DiscMinus
            | Primitive::Span
            | Primitive::Omit
            | Primitive::Special
            | Primitive::Discretionary
            | Primitive::Insert
            | Primitive::VAdjust
            | Primitive::Mark
            | Primitive::Marks
            | Primitive::TopMarks
            | Primitive::FirstMarks
            | Primitive::BotMarks
            | Primitive::SplitFirstMarks
            | Primitive::SplitTopMarks
            | Primitive::SplitBotMarks
            | Primitive::TopMark
            | Primitive::FirstMark
            | Primitive::BotMark
            | Primitive::SplitFirstMark
            | Primitive::SplitBotMark
    )
}

/// 主题分类：原语是否属于"toks 参数 / 段落 / 方向 / 收尾 / 盒子尺寸"主题（dispatch_toks_state）。
fn is_toks_state_prim(p: Primitive) -> bool {
    matches!(
        p,
        Primitive::Chardef
            | Primitive::Countdef
            | Primitive::Dimendef
            | Primitive::Skipdef
            | Primitive::Muskipdef
            | Primitive::Toksdef
            | Primitive::MathCharDef
            | Primitive::EveryDisplay
            | Primitive::EveryMath
            | Primitive::EveryPar
            | Primitive::EveryHBox
            | Primitive::EveryVBox
            | Primitive::EveryCr
            | Primitive::ErrHelp
            | Primitive::EveryJob
            | Primitive::SpaceFactor
            | Primitive::Parshape
            | Primitive::ParshapeLength
            | Primitive::ParshapeIndent
            | Primitive::ParshapeDimen
            | Primitive::BeginL
            | Primitive::EndL
            | Primitive::BeginR
            | Primitive::EndR
            | Primitive::Dump
            | Primitive::ReadLine
            | Primitive::Wd
            | Primitive::Ht
            | Primitive::Dp
    )
}

/// 主题分类：原语是否属于"IO / 输出 / 盒子操作 / 诊断"主题（dispatch_io）。
fn is_io_prim(p: Primitive) -> bool {
    matches!(
        p,
        Primitive::Input
            | Primitive::OpenIn
            | Primitive::CloseIn
            | Primitive::NewRead
            | Primitive::Read
            | Primitive::NewWrite
            | Primitive::OpenOut
            | Primitive::CloseOut
            | Primitive::Write
            | Primitive::Immediate
            | Primitive::ShipOut
            | Primitive::Output
            | Primitive::Box
            | Primitive::SetBox
            | Primitive::Copy
            | Primitive::UnHBox
            | Primitive::UnHCopy
            | Primitive::UnVBox
            | Primitive::UnVCopy
            | Primitive::LastBox
            | Primitive::UnSkip
            | Primitive::UnPenalty
            | Primitive::Unkern
            | Primitive::VSplit
            | Primitive::DisplayWidth
            | Primitive::PageDepth
            | Primitive::PageFillLStretch
            | Primitive::PageShrink
            | Primitive::NullFont
            | Primitive::ShowBox
            | Primitive::ShowGroups
            | Primitive::ShowLists
            | Primitive::Message
            | Primitive::Show
            | Primitive::ShowThe
            | Primitive::ShowTokens
            | Primitive::ShowIfs
            | Primitive::Error
            | Primitive::VarUnit
    )
}
