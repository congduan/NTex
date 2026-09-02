// ---------- 字体相关原语执行器 ----------
//
// 从 expand/primitive.rs 迁出的字体家族方法。沿用项目已有的"include! 分片"模式
// （见 expand/mod.rs 末尾），每片维持独立 `impl Expander { ... }` 块——Rust 允许
// 同一类型的多个 impl 块分散在不同文件，效果等价于单 impl。
//
// 涵盖：\textfont/\scriptfont/\scriptscriptfont 族分配、\patterns/\hyphenation、
// \left/\right/\middle 定界符扫描、\font 加载/\fontname 查询、TFM 字体的
// \fontdimen/\hyphenchar/\skewchar/\delcode/\mathcode/字符度量（\fontcharwd 等）。

impl Expander {
    /// `\textfont<fam>=<fontcs>` 族分配：扫描 fam 号、可选 `=`、字体选择器 cs。
    fn exec_math_font(&mut self, kind: u8) -> Result<()> {
        let fam = self.scan_number()?;
        // TeX：族号越界报 "! Bad number" 钳制（<0 → 0，>15 → 15；TRIP L346
        // `\textfont16=\relax`）
        let fam = if (0..=15).contains(&fam) {
            fam
        } else {
            let _ = self.sink.write16(format!(
                "! Bad number ({}).\n\
                 Since I expected to read a number between 0 and 15,\n\
                 I changed this one to zero.\n",
                fam
            ));
            fam.clamp(0, 15)
        };
        // 可选赋值符 '='
        self.skip_spaces()?;
        let probe = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\textfont 后缺少字体"))?
            .0;
        if probe.charcode() == Some(b'=' as u32) {
            self.skip_spaces()?;
        } else {
            self.unread(probe);
        }
        let (tok, _) = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\textfont 后缺少字体选择器"))?;
        let csid = tok
            .csid()
            .ok_or_else(|| Error::invalid_input("\\textfont 后必须是 \\font 定义的 cs"))?;
        // \scriptfont1=\textfont1：RHS 为另一数学字体族 → 复制其当前字体
        // （TeX：族未赋值时为 nullfont；引擎以 FontId 0 兜底）
        let font = match self.eqtb.slot(csid).clone() {
            EqSlot::Font(f) => f,
            // TRIP：`\textfont1=\font`：`\font` 作当前字体选择器
            EqSlot::Primitive(Primitive::Font) => self.sink.current_font(),
            EqSlot::Primitive(
                Primitive::TextFont | Primitive::ScriptFont | Primitive::ScriptScriptFont,
            ) => {
                let rhs_kind = match self.eqtb.slot(csid) {
                    EqSlot::Primitive(Primitive::TextFont) => 0,
                    EqSlot::Primitive(Primitive::ScriptFont) => 1,
                    _ => 2,
                };
                let rhs_fam = self.scan_number()?;
                let rhs_fam = if (0..=15).contains(&rhs_fam) {
                    rhs_fam
                } else {
                    let _ = self.sink.write16(format!(
                        "! Bad number ({}).\nI changed this one to zero.\n",
                        rhs_fam
                    ));
                    rhs_fam.clamp(0, 15)
                };
                self.math_fonts[rhs_kind][rhs_fam as usize]
            }
            _ => {
                // TeX：\textfont<fam>=<非字体> → 报错恢复（绑定字体 0/nullfont；
                // TRIP L347 `\textfont16=\relax`）
                let _ = self.sink.write16(format!(
                    "! \\textfont 的 \\{} 不是字体选择器。\n",
                    self.intern.name(csid)
                ));
                0
            }
        };
        self.math_fonts[kind as usize][fam as usize] = font;
        self.sink.math_font(kind, fam as u8, font)
    }

    /// `\patterns{...}`（M4-6）：扫描平衡组（不展开），抽取模式文本直通 sink。
    ///
    /// TeX `new_patterns`（tex.web）语义：字母/数字/`.` 是模式字符；
    /// 其余 token（空格、控制序列等）是模式分隔符。文本交由 ntex-layout 的
    /// Liang trie 解析（ntex-layout::hyphen::PatternTrie::parse）。
    fn exec_patterns(&mut self) -> Result<()> {
        let tokens = self.scan_group_contents(None)?;
        let mut out: Vec<u8> = Vec::new();
        for tok in tokens {
            match tok.catcode() {
                Some(Catcode::Letter) | Some(Catcode::Other) => {
                    let ch = tok.charcode().expect("Char 必有 charcode");
                    let is_pattern_char = u8::try_from(ch).is_ok_and(|b| {
                        b.is_ascii_alphabetic() || b.is_ascii_digit() || b == b'.'
                    });
                    if is_pattern_char {
                        out.push(ch as u8);
                    } else if out.last() != Some(&b' ') {
                        out.push(b' ');
                    }
                }
                _ => {
                    // 空格（cat 10）与任何其他 token：模式分隔符
                    if out.last() != Some(&b' ') {
                        out.push(b' ');
                    }
                }
            }
        }
        self.sink.patterns(out)
    }

    /// `\hyphenation{...}`（ETRIP 冲刺）：扫描平衡组，解析异常词表。
    ///
    /// TeX `new_hyphenation`（tex.web）语义：
    /// - 空格（cat 10）分隔单词；字母/其他字符（cat 11/12）是词字符；
    /// - `-`（断字符，默认 hyphenchar 45）标记允许的断点（可位于词首/词尾）；
    /// - 词字符经 `\lccode` 转小写后存储（扫描时转换，组结束不回滚异常表）；
    /// - 异常词按当前 `\language`（misc[16]）归档，断字时**优先于**模式表。
    ///
    /// 简化：暂按单语言全局表存储（sink 侧不分语言）；词比较不做 lccode 二次
    /// 转换（段落词需已小写）。ETRIP 用例均满足。
    fn exec_hyphenation(&mut self) -> Result<()> {
        // TeX：\hyphenation 参数为 <general text>；前导 \relax 跳过
        // （trip.tex L72 `\hyphenation\relax{...}`，TeX scan_toks 的 \relax 分隔）
        self.skip_spaces()?;
        if let Some(csid) = self.peek_csid()? {
            if matches!(self.eqtb.slot(csid), EqSlot::Primitive(Primitive::Relax)) {
                self.fetch()?;
            }
        }
        let tokens = self.scan_group_contents(None)?;
        // 断字符：默认 `-`（45）；ETRIP 用例均用字面 `-`。
        const HYPHEN_CHAR: u32 = 45;
        let mut words: Vec<(Vec<u8>, Vec<usize>)> = Vec::new();
        let mut letters: Vec<u8> = Vec::new();
        let mut breaks: Vec<usize> = Vec::new();
        // 词首断点（`-q-` 的首 `-`）在词开始时记录：word_breaks_at_0
        let mut break_at_start = false;
        for tok in tokens {
            let cat = tok.catcode();
            match cat {
                Some(Catcode::Letter) | Some(Catcode::Other) => {
                    let ch = tok.charcode().unwrap_or(0);
                    if ch == HYPHEN_CHAR {
                        if letters.is_empty() {
                            break_at_start = true; // 词首 `-`：断点 0
                        } else {
                            breaks.push(letters.len());
                        }
                    } else {
                        // lccode 转小写（0 保留原字符：无小写映射）
                        let lower = self.lccodes[ch as usize];
                        if lower > 0 {
                            letters.push(lower as u8);
                        } else {
                            letters.push(ch as u8);
                        }
                    }
                }
                Some(Catcode::Space) | None | Some(_) => {
                    // 空格或任何非字符 token：结束当前词（若有）
                    if !letters.is_empty() {
                        if break_at_start {
                            breaks.insert(0, 0); // 词首 `-`：断点 0
                        }
                        words.push((std::mem::take(&mut letters), std::mem::take(&mut breaks)));
                    }
                    break_at_start = false;
                }
            }
        }
        if !letters.is_empty() {
            if break_at_start {
                breaks.insert(0, 0);
            }
            words.push((letters, breaks));
        }
        if !words.is_empty() {
            self.sink.hyphenation(words)?;
        }
        Ok(())
    }

    /// `\left`/`\right`/`\middle` 的定界符参数：字符 → charcode（`.` 为空定界符）；
    /// `\.` → None。无法识别的 cs（如 `\par`）按 TeX 恢复：报
    /// "! Missing delimiter (. inserted)." 到转录，并以 `(` 定界符继续。
    fn scan_delimiter(&mut self) -> Result<Option<u32>> {
        self.skip_spaces()?;
        let (tok, _) = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\left/\\right 后缺少定界符"))?;
        match tok.kind() {
            TokenKind::Char => {
                let ch = tok.charcode().expect("Char 必有 charcode");
                if ch == b'.' as u32 {
                    Ok(None) // \left.：空定界符
                } else if (b'0' as u32..=b'9' as u32).contains(&ch)
                    || ch == b'"' as u32
                    || ch == b'\'' as u32
                {
                    // TRIP：\radical"3 的 "3 是十六进制 delimiter number（TeX scan_delimiter
                    // 优先扫描数字）；放回后按数字扫描
                    self.unread(tok);
                    let n = self.scan_number()?;
                    Ok(Some(u32::try_from(n).unwrap_or(0)))
                } else {
                    Ok(Some(ch))
                }
            }
            TokenKind::ControlSeq => {
                let name = self.intern.name(tok.csid().expect("ControlSeq 必有 csid"));
                if name == "." {
                    Ok(None)
                } else {
                    self.report_error("Missing delimiter (. inserted).");
                    Ok(Some(b'(' as u32))
                }
            }
            _ => Err(Error::invalid_input("\\left/\\right 后必须是定界符")),
        }
    }

    /// `\font<cs>[=]<名字>[at <dimen>|scaled <int>]`：加载字体并定义 cs 为字体选择器。
    ///
    /// 语法扫描在 VM 侧（cs、可选 `=`、字体名、可选 at/scaled），实际加载交给
    /// [`FontLoader`]（ntex-layout 的 TFM 加载器维护字体表并返回 FontId）。
    fn exec_font(&mut self) -> Result<()> {
        let name = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\font 后缺少控制序列"))?
            .0;
        let csid = name
            .csid()
            .ok_or_else(|| Error::invalid_input("\\font 后必须是控制序列"))?;
        // 可选赋值符 '='
        self.skip_spaces()?;
        let probe = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\font 后缺少字体名"))?
            .0;
        if probe.charcode() != Some(b'=' as u32) {
            self.unread(probe);
        }
        let font_name = self.scan_font_name()?;
        // 可选 at / scaled（互斥）
        let keyword = self.scan_keyword(|w| w == "at" || w == "scaled")?;
        let (at, scaled) = match keyword.as_deref() {
            Some("at") => (Some(self.scan_dimen()?), None),
            Some("scaled") => (None, Some(self.scan_number()?)),
            _ => (None, None),
        };
        let font = match self.font_loader.load(&font_name, at, scaled) {
            Ok(f) => f,
            Err(_) => {
                // TeX：字体加载失败 → "! Font not loadable" 报错恢复（绑定字体 0，
                // 后续使用报更多错但作业继续；TRIP L211 `\font\mumble=mumble`）。
                let _ = self.sink.write16(format!(
                    "! Font {font_name} not loadable: Metric (TFM) file not found.\n\
                     I'm not loading it.\n"
                ));
                0
            }
        };
        // `\fontname` 查询登记：FontId → 外部名（失败加载绑定 0 也登记，保留名字）
        if self.font_names.len() <= font as usize {
            self.font_names.resize(font as usize + 1, None);
        }
        // at 规格并入名字（e-TeX \tracingassigns 显示 `select font etrip at 11.0pt`）
        self.font_names[font as usize] = Some(match at {
            Some(a) => format!(
                "{font_name} at {}pt",
                crate::register::format_dimen(a)
            ),
            None => font_name.clone(),
        });
        // .fmt 序列化用：FontId → (外部名, at, scaled)，pass2 恢复字体表
        if self.font_loads.len() <= font as usize {
            self.font_loads.resize(font as usize + 1, None);
        }
        self.font_loads[font as usize] = Some((font_name.clone(), at, scaled));
        // 组作用域 + \global 语义（同 \def）
        let global = self.is_global();
        // e-TeX \tracingassigns（misc 5）：字体赋值追踪——tex.web \font 先绑定
        // nullfont 再加载实际字体（etrip 显示 `undefined→nullfont→etrip` 两步）
        let tracing = self.params.misc[5] > 0;
        let prev0 = if tracing {
            Some(self.eqtb.slot(csid).clone())
        } else {
            None
        };
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Eqtb {
                    csid,
                    prev: self.eqtb.slot(csid).clone(),
                },
            ));
        }
        self.eqtb.set_font(csid, 0);
        if tracing {
            self.trace_assign(csid, global, prev0.as_ref().expect("tracing 时已存"), &EqSlot::Font(0));
        }
        if font != 0 {
            if tracing {
                let prev = self.eqtb.slot(csid).clone();
                self.trace_assign(csid, global, &prev, &EqSlot::Font(font));
            }
            self.eqtb.set_font(csid, font);
            // ETRIP showbox：登记 FontId → cs 名（tex.web 字体标识显示用 cs 名）
            let cs_name = self.intern.name(csid).to_string();
            if self.font_cs_names.len() <= font as usize {
                self.font_cs_names.resize(font as usize + 1, None);
            }
            self.font_cs_names[font as usize] = Some(cs_name.clone());
            self.sink.font_defined(font, &cs_name)?;
        }
        self.finish_assignment();
        Ok(())
    }

    /// `\fontname<font>`：展开为字体外部名（cat 12 字符 token；tex.web 可展开原语，
    /// `\font` 加载时登记的 [`Expander::font_names`] 查询；TRIP L218 `\fontname\ip`）。
    fn exec_fontname(&mut self) -> Result<()> {
        let font = self.scan_font_ident()?;
        let name = self
            .font_names
            .get(font as usize)
            .and_then(|n| n.clone())
            .unwrap_or_default();
        self.emit_tokens(
            name.bytes()
                .map(|b| Token::char(Catcode::Other, u32::from(b)))
                .collect(),
        )
    }

    /// 扫描外部字体名：连续 cat 11（字母）/ cat 12（其他）字符，遇空格/组/控制序列结束。
    fn scan_font_name(&mut self) -> Result<String> {
        self.skip_spaces()?;
        let mut name = String::new();
        while let Some((tok, _)) = self.fetch()? {
            match tok.catcode() {
                Some(Catcode::Letter) | Some(Catcode::Other) => {
                    let ch = tok
                        .charcode()
                        .and_then(char::from_u32)
                        .ok_or_else(|| Error::invalid_input("字体名含非法字符"))?;
                    if !ch.is_ascii() {
                        return Err(Error::invalid_input("字体名仅支持 ASCII（M3-4 范围）"));
                    }
                    name.push(ch);
                }
                _ => {
                    self.unread(tok);
                    break;
                }
            }
        }
        if name.is_empty() {
            return Err(Error::invalid_input("\\font 后缺少字体名"));
        }
        Ok(name)
    }

    /// `\fontdimen<num><font>=<dimen>`：设置字体的 fontdimen 参数
    /// （TeX `assign_font_dimen`；组内局部、可 `\global`）。
    fn exec_fontdimen(&mut self) -> Result<()> {
        let num = self.scan_number()?;
        let num = u32::try_from(num).map_err(|_| Error::invalid_input("\\fontdimen 参数号越界"))?;
        let font = self.scan_font_ident()?;
        // TRIP L404：fontdimen 参数号越界（`\fontdimen 1000=...`）→ 报错并跳过赋值
        if num >= 13 {
            self.report_error("Font \\FONT? has only 13 fontdimen parameters.");
            // TeX 恢复：消费 `= <dimen>`（`20\varunit`），不改变字体参数；后续
            // `\showthe\fontdimen1000\trip\let\PAR=\par` 正常继续（trip.log L5831）。
            if self.expect_equals().is_ok() {
                let _ = self.scan_dimen();
            }
            return Ok(());
        }
        self.expect_equals()?;
        let value = self.scan_dimen()?;
        let prev = self.fontdimens.get(&(font, num)).copied();
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::FontDimen { font, num, prev },
            ));
        }
        self.fontdimens.insert((font, num), value);
        self.finish_assignment();
        Ok(())
    }

    /// TeX `scan_font_ident` 的 "Missing font identifier" 报错恢复：
    /// 报错后用当前字体继续（TRIP L404 `\fontdimen 1000=20\varunit`——`=` 非字体）。
    fn missing_font_ident(&mut self) -> Result<u32> {
        self.report_error("Missing font identifier.");
        Ok(self.sink.current_font())
    }

    /// 扫描字体标识符（TeX `scan_font_ident`）：`\font` 定义的 cs 或 `\nullfont`。
    fn scan_font_ident(&mut self) -> Result<u32> {
        self.skip_spaces()?;
        let Some((tok, _ne)) = self.fetch()? else {
            return self.missing_font_ident();
        };
        let Some(csid) = tok.csid() else {
            // TeX scan_font_ident：非字体 cs 报错后 **放回** token（TRIP L404
            // `\fontdimen 1000=20\varunit` —— `=` 放回，供错误恢复跳过赋值）。
            self.unread(tok);
            return self.missing_font_ident();
        };
        match self.eqtb.slot(csid).clone() {
            EqSlot::Font(f) => Ok(f),
            // TRIP：`\font`（无参数）作当前字体选择器（\textfont1=\font）
            EqSlot::Primitive(Primitive::Font) => Ok(self.sink.current_font()),
            // TRIP 补全批次：\\nullfont（预定义空字体，id 0）
            EqSlot::Primitive(Primitive::NullFont) => Ok(0),
            // \textfont<n>/...：字体位置读取当前族字体（TeX find_font 语义）
            EqSlot::Primitive(
                Primitive::TextFont | Primitive::ScriptFont | Primitive::ScriptScriptFont,
            ) => {
                let kind = match self.eqtb.slot(csid) {
                    EqSlot::Primitive(Primitive::TextFont) => 0,
                    EqSlot::Primitive(Primitive::ScriptFont) => 1,
                    _ => 2,
                };
                let fam = self.scan_number()?;
                let fam = if (0..=15).contains(&fam) {
                    fam
                } else {
                    let _ = self.sink.write16(format!(
                        "! Bad number ({}).\nI changed this one to zero.\n",
                        fam
                    ));
                    fam.clamp(0, 15)
                };
                Ok(self.math_fonts[kind][fam as usize])
            }
            // TRIP：`\fontdimen6\the\scriptfont2` —— \the 展开为字体选择器
            EqSlot::Primitive(Primitive::The) => {
                let t2 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\the 后缺少内部量"))?
                    .0;
                let csid2 = t2
                    .csid()
                    .ok_or_else(|| Error::invalid_input("\\the 需要内部量参数"))?;
                match self.eqtb.slot(csid2).clone() {
                    EqSlot::Primitive(
                        Primitive::TextFont | Primitive::ScriptFont | Primitive::ScriptScriptFont,
                    ) => {
                        let kind = match self.eqtb.slot(csid2) {
                            EqSlot::Primitive(Primitive::TextFont) => 0,
                            EqSlot::Primitive(Primitive::ScriptFont) => 1,
                            _ => 2,
                        };
                        let fam = self.scan_number()?;
                        let fam = if (0..=15).contains(&fam) {
                            fam
                        } else {
                            let _ = self.sink.write16(format!(
                                "! Bad number ({}).\nI changed this one to zero.\n",
                                fam
                            ));
                            fam.clamp(0, 15)
                        };
                        Ok(self.math_fonts[kind][fam as usize])
                    }
                    _ => self.missing_font_ident(),
                }
            }
            _ => self.missing_font_ident(),
        }
    }

    /// `\hyphenchar<font>=<int>`：设置字体的断字符（TeX assign_font_int；
    /// 组内局部、可 `\global`；覆盖表存 expander 侧，排版器断字时读取）。
    fn exec_hyphenchar(&mut self) -> Result<()> {
        let font = self.scan_font_ident()?;
        self.expect_equals()?;
        let value = self.scan_number()?;
        let prev = self.hyphenchars.get(&font).copied();
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::HyphenChar { font, prev },
            ));
        }
        self.hyphenchars.insert(font, value);
        self.finish_assignment();
        Ok(())
    }

    /// TRIP 补全批次：`\skewchar<font>=<num>`：设置字体偏斜字符（仿 \hyphenchar）。
    fn exec_skewchar(&mut self) -> Result<()> {
        let font = self.scan_font_ident()?;
        self.expect_equals()?;
        let value = self.scan_number()?;
        let prev = self.skewchars.get(&font).copied();
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::SkewChar { font, prev },
            ));
        }
        self.skewchars.insert(font, value);
        self.finish_assignment();
        Ok(())
    }

    /// `\delcode<num>=<num>`：设置字符的定界符码（TeX assign_del_code；组内局部）。
    fn exec_delcode(&mut self) -> Result<()> {
        let byte = self.scan_char_code()?;
        let byte = u8::try_from(byte).map_err(|_| Error::invalid_input("\\delcode 字符码越界"))?;
        self.expect_equals()?;
        let value = self.scan_number()?;
        let value = u32::try_from(value)
            .map_err(|_| Error::invalid_input("\\delcode 定界符码越界（24 位）"))?
            & 0x00FF_FFFF;
        let prev = self.delcodes.get(&u32::from(byte)).copied();
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::DelCode { byte, prev },
            ));
        }
        self.delcodes.insert(u32::from(byte), value);
        self.finish_assignment();
        Ok(())
    }

    /// TRIP 冲刺：`\mathcode<8位字符>=<15位值>`：字符数学码赋值
    /// （TeX：mathcode 15 位 = class(3)<<12 + family(4)<<8 + char(8)）。
    fn exec_mathcode(&mut self) -> Result<()> {
        let byte = self.scan_char_code()?;
        let byte = u8::try_from(byte).map_err(|_| Error::invalid_input("\\mathcode 字符码越界"))?;
        self.expect_equals()?;
        let value = self.scan_number()?;
        let value = u32::try_from(value)
            .map_err(|_| Error::invalid_input("\\mathcode 数学码越界（15 位）"))?
            & 0x0000_7FFF;
        let prev = self.mathcodes.get(&u32::from(byte)).copied();
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::MathCode { byte, prev },
            ));
        }
        self.mathcodes.insert(u32::from(byte), value);
        self.finish_assignment();
        Ok(())
    }

    /// 读取字体参数（fontdimen 数值查询；与 [`Self::exec_fontdimen`] 配对）。
    fn fontdimen(&self, font: u32, num: u32) -> i64 {
        self.fontdimens.get(&(font, num)).copied().unwrap_or(0)
    }
}
