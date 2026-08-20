impl Expander {
    // ---------- 原语执行 ----------

    fn exec_primitive(&mut self, prim: Primitive) -> Result<()> {
        match prim {
            Primitive::Relax => Ok(()),
            // 内部只读整数单独出现：no-op（TeX 在 no_mode——输出例程中——忽略；
            // 其余模式应报错，模式状态在排版器侧，M1 宽松处理）
            Primitive::Badness => Ok(()),
            Primitive::Expandafter => self.exec_expandafter(),
            Primitive::Noexpand => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\noexpand 后无 token"))?;
                self.stack.push(InputFrame::TokenList {
                    items: Arc::from([(t.0, true)]),
                    pos: 0,
                });
                Ok(())
            }
            Primitive::Def => self.exec_def(false),
            Primitive::Edef => self.exec_def(true),
            // \gdef ≡ \global\def；\xdef ≡ \global\edef
            Primitive::Gdef => {
                self.global_pending = true;
                self.exec_def(false)
            }
            Primitive::Xdef => {
                self.global_pending = true;
                self.exec_def(true)
            }
            // \outer：宏定义前缀（TeX 限制宏在实参中出现；当前仅消费，语义后续补）
            Primitive::Outer => {
                self.outer_pending = true;
                Ok(())
            }
            Primitive::Let => self.exec_let(),
            Primitive::Catcode => self.exec_catcode(),
            Primitive::SfCode => self.exec_sfcode(),
            Primitive::LcCode => self.exec_lccode(),
            // ETRIP 冲刺：\advance<寄存器> <增量>（寄存器运算）
            Primitive::Advance => self.exec_advance(),
            Primitive::End => {
                self.stack.clear();
                self.output_active = false;
                // TeX `\end` 收尾：flush 所有延迟写流（final_cleanup 语义）
                self.flush_writes()?;
                Ok(())
            }
            // M1-7 扫描顺序原语
            Primitive::Futurelet => self.exec_futurelet(),
            Primitive::Aftergroup => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\aftergroup 后无 token"))?
                    .0;
                self.aftergroup.push((self.group_level, t));
                Ok(())
            }
            Primitive::Afterassignment => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\afterassignment 后无 token"))?
                    .0;
                self.afterassignment = Some(t);
                Ok(())
            }
            // M1-9 条件原语：由 process_one 拦截，不应到达此处
            Primitive::If
            | Primitive::IfCat
            | Primitive::IfNum
            | Primitive::IfDim
            | Primitive::IfX
            | Primitive::IfOdd
            | Primitive::IfCase
            | Primitive::IfTrue
            | Primitive::IfFalse
            | Primitive::Else
            | Primitive::Fi
            | Primitive::Or => Err(Error::internal("条件原语不应到达 exec_primitive")),
            // M1-10 寄存器
            Primitive::Count
            | Primitive::Dimen
            | Primitive::Skip
            | Primitive::Muskip
            | Primitive::Toks => self.exec_register(prim),
            // ETRIP 冲刺：\thinmuskip/\medmuskip/\thickmuskip（muskip 寄存器 0/1/2）
            Primitive::ThinMuskip => self.exec_muskip_param(0),
            Primitive::MedMuskip => self.exec_muskip_param(1),
            Primitive::ThickMuskip => self.exec_muskip_param(2),
            // ETRIP 冲刺：\advance<寄存器> <增量>
            Primitive::Advance => self.exec_advance(),
            Primitive::The => self.exec_the(),
            Primitive::Global => {
                self.global_pending = true;
                Ok(())
            }
            // M1-11 组
            Primitive::BeginGroup => self.begin_group(),
            Primitive::EndGroup => self.end_group(),
            // M3-2 排版原语
            // 盒子：直通 sink（规格 to/spread 属 M3-2-2，暂拒）
            Primitive::HBox | Primitive::VBox | Primitive::VTop => {
                self.reject_box_spec()?;
                self.sink.primitive(prim)
            }
            Primitive::Par => self.sink.primitive(prim),
            // 带参数扫描的排版原语：扫描在 VM 侧完成，结果交给 sink
            Primitive::HSkip | Primitive::VSkip => {
                let g = self.scan_glue()?;
                self.sink.glue(g)
            }
            Primitive::Kern => {
                let w = self.scan_dimen()?;
                self.sink.kern(w)
            }
            Primitive::Penalty => {
                let p = self.scan_number()?;
                self.sink.penalty(p)
            }
            Primitive::HRule | Primitive::VRule => {
                let [h, d, w] = self.scan_rule_specs()?;
                self.sink.rule(w, h, d)
            }
            // M3-2-2 内部参数赋值
            Primitive::ParIndent | Primitive::LineSkipLimit => {
                let v = self.scan_dimen()?;
                let kind = if prim == Primitive::ParIndent {
                    ParamKind::ParIndent
                } else {
                    ParamKind::LineSkipLimit
                };
                self.assign_param(kind, ParamValue::Dimen(v))
            }
            Primitive::BaselineSkip | Primitive::LineSkip => {
                let g = self.scan_glue()?;
                let kind = if prim == Primitive::BaselineSkip {
                    ParamKind::BaselineSkip
                } else {
                    ParamKind::LineSkip
                };
                self.assign_param(kind, ParamValue::Glue(g))
            }
            // 段落缩进：直通 sink 由排版器解释
            Primitive::Indent | Primitive::NoIndent => self.sink.primitive(prim),
            // M3-3 折行参数
            Primitive::HSize => {
                let v = self.scan_dimen()?;
                self.assign_param(ParamKind::HSize, ParamValue::Dimen(v))
            }
            Primitive::Tolerance => {
                let v = self.scan_number()?;
                self.assign_param(ParamKind::Tolerance, ParamValue::Number(v))
            }
            // M3-5 断页参数
            Primitive::VSize | Primitive::MaxDepth => {
                let v = self.scan_dimen()?;
                let kind = if prim == Primitive::VSize {
                    ParamKind::VSize
                } else {
                    ParamKind::MaxDepth
                };
                self.assign_param(kind, ParamValue::Dimen(v))
            }
            Primitive::TopSkip | Primitive::ParSkip => {
                let g = self.scan_glue()?;
                let kind = if prim == Primitive::TopSkip {
                    ParamKind::TopSkip
                } else {
                    ParamKind::ParSkip
                };
                self.assign_param(kind, ParamValue::Glue(g))
            }
            // M4-4 显示数学间距参数
            Primitive::AboveDisplaySkip
            | Primitive::BelowDisplaySkip
            | Primitive::AboveDisplayShortSkip
            | Primitive::BelowDisplayShortSkip => {
                let g = self.scan_glue()?;
                let kind = match prim {
                    Primitive::AboveDisplaySkip => ParamKind::AboveDisplaySkip,
                    Primitive::BelowDisplaySkip => ParamKind::BelowDisplaySkip,
                    Primitive::AboveDisplayShortSkip => ParamKind::AboveDisplayShortSkip,
                    _ => ParamKind::BelowDisplayShortSkip,
                };
                self.assign_param(kind, ParamValue::Glue(g))
            }
            Primitive::PreDisplayPenalty | Primitive::PostDisplayPenalty => {
                let p = self.scan_number()?;
                let kind = if prim == Primitive::PreDisplayPenalty {
                    ParamKind::PreDisplayPenalty
                } else {
                    ParamKind::PostDisplayPenalty
                };
                self.assign_param(kind, ParamValue::Number(p))
            }
            // ETRIP 冲刺：TeX 内部整数参数
            Primitive::EndlineChar
            | Primitive::NewlineChar
            | Primitive::DefaultHyphenChar
            | Primitive::DefaultSkewChar => {
                let v = self.scan_number()?;
                let kind = match prim {
                    Primitive::EndlineChar => ParamKind::EndlineChar,
                    Primitive::NewlineChar => ParamKind::NewlineChar,
                    Primitive::DefaultHyphenChar => ParamKind::DefaultHyphenChar,
                    _ => ParamKind::DefaultSkewChar,
                };
                self.assign_param(kind, ParamValue::Number(v))
            }
            // ETRIP 冲刺：TeX/e-TeX 内部整数参数（misc 数组，按下标索引）
            p if int_param_index(p).is_some() => {
                let idx = int_param_index(p).expect("已检查 is_some");
                let v = self.scan_number()?;
                self.assign_param(ParamKind::MiscInt(idx), ParamValue::Number(v))
            }
            // ETRIP 冲刺：交互模式命令（\batchmode/\nonstopmode/\scrollmode/\errorstopmode）
            p if interaction_mode_value(p).is_some() => {
                let v = interaction_mode_value(p).expect("已检查 is_some");
                self.assign_param(ParamKind::MiscInt(19), ParamValue::Number(v))
            }
            // ETRIP 冲刺：\chardef\cs=<num>（cs 绑定字符，cat 12）
            Primitive::Chardef => {
                let csid = self.scan_cs_ident()?;
                let v = self.scan_number()?;
                let v = u32::try_from(v)
                    .map_err(|_| Error::invalid_input("\\chardef 字符码越界"))?;
                self.set_slot_scoped(csid, EqSlot::Char {
                    catcode: Catcode::Other,
                    charcode: v,
                });
                Ok(())
            }
            // ETRIP 冲刺：\countdef/\dimendef/\skipdef/\muskipdef/\toksdef\cs=<num>（cs 绑定寄存器）
            Primitive::Countdef
            | Primitive::Dimendef
            | Primitive::Skipdef
            | Primitive::Muskipdef
            | Primitive::Toksdef => {
                let csid = self.scan_cs_ident()?;
                let idx = self.scan_number()?;
                let idx = usize::try_from(idx)
                    .map_err(|_| Error::invalid_input("寄存器下标越界"))?;
                let kind = match prim {
                    Primitive::Countdef => RegKind::Count,
                    Primitive::Dimendef => RegKind::Dimen,
                    Primitive::Skipdef => RegKind::Skip,
                    Primitive::Muskipdef => RegKind::Muskip,
                    _ => RegKind::Toks,
                };
                self.set_slot_scoped(csid, EqSlot::Register(kind, idx));
                Ok(())
            }
            // M3-4 字体
            Primitive::Font => self.exec_font(),
            // ETRIP 冲刺：字体参数 \fontdimen<num><font>=<dimen>
            Primitive::FontDimen => self.exec_fontdimen(),
            // ETRIP 冲刺：\hyphenchar<font>=<int>（字体断字符）
            Primitive::HyphenChar => self.exec_hyphenchar(),
            // ETRIP 冲刺：\delcode<num>=<num>（字符定界符码）
            Primitive::DelCode => self.exec_delcode(),
            // ETRIP 冲刺：终端转录
            Primitive::Message => self.exec_message(),
            Primitive::Show => self.exec_show(),
            Primitive::ShowThe => self.exec_showthe(),
            // M3-5 输出：\shipout 直通 sink（排版器解释：封装下一盒子为页面）
            Primitive::ShipOut => self.sink.primitive(prim),
            // M3-5-3 输出例程：\output=<general text> 存储 token 列表
            Primitive::Output => {
                self.expect_equals()?;
                let val = self.scan_group_contents()?;
                self.assign_output(Arc::from(val));
                Ok(())
            }
            // M3-5-3 盒子寄存器：\box<n> 交给 sink（shipout_next 时封装为页面）
            Primitive::Box => {
                let idx = self.scan_register_index()?;
                self.sink.box_register(idx)
            }
            // M3 收尾（RFC-3）：VFS 副作用原语
            Primitive::Input => self.exec_input(),
            Primitive::OpenIn => self.exec_openin(),
            Primitive::CloseIn => self.exec_closein(),
            Primitive::NewRead => self.exec_new_stream(StreamKind::Read),
            Primitive::Read => self.exec_read(),
            Primitive::NewWrite => self.exec_new_stream(StreamKind::Write),
            Primitive::OpenOut => self.exec_openout(),
            Primitive::CloseOut => self.exec_closeout(),
            Primitive::Write => self.exec_write(),
            Primitive::Immediate => {
                self.immediate_pending = true;
                Ok(())
            }
            // M4-2 数学原语：直通 sink（排版器解释；delimiter 参数在 VM 侧扫描）
            Primitive::DisplayStyle => self.sink.math_style(0),
            Primitive::TextStyle => self.sink.math_style(1),
            Primitive::ScriptStyle => self.sink.math_style(2),
            Primitive::ScriptScriptStyle => self.sink.math_style(3),
            Primitive::Over => self.sink.math_fraction(None),
            Primitive::Atop => self.sink.math_fraction(Some(0)),
            Primitive::Left => {
                let d = self.scan_delimiter()?;
                self.sink.math_left(d)
            }
            Primitive::Right => {
                let d = self.scan_delimiter()?;
                self.sink.math_right(d)
            }
            Primitive::Sqrt => self.sink.math_sqrt(),
            Primitive::MathOrd => self.sink.math_class(0),
            Primitive::MathBin => self.sink.math_class(1),
            Primitive::MathOp => self.sink.math_class(2),
            Primitive::MathRel => self.sink.math_class(3),
            Primitive::MathOpen => self.sink.math_class(4),
            Primitive::MathClose => self.sink.math_class(5),
            Primitive::MathPunct => self.sink.math_class(6),
            Primitive::MathInner => self.sink.math_class(7),
            Primitive::Nonscript => self.sink.primitive(prim),
            // M4-5 e-TeX 展开扩展
            Primitive::Protected => {
                self.protected_pending = true;
                Ok(())
            }
            Primitive::Unless => {
                self.unless_pending = true;
                Ok(())
            }
            // 条件原语由 process_one 拦截
            Primitive::IfDefined | Primitive::IfCsname | Primitive::IfPrimitive => {
                Err(Error::internal("条件原语不应到达 exec_primitive"))
            }
            Primitive::NumExpr => {
                let v = self.eval_int_expression()?;
                self.emit_tokens(emit_count(v))
            }
            // M4-5 e-TeX 扩展：\dimexpr/\glueexpr 可展开求值（\the 上下文由 the_tokens 直接读取）
            Primitive::Dimexpr => {
                let v = self.eval_dimen_expression()?;
                self.emit_tokens(emit_dimen(v))
            }
            Primitive::Glueexpr => {
                let g = self.eval_glue_expression()?;
                self.emit_tokens(emit_glue(g))
            }
            Primitive::Scantokens => self.exec_scantokens(),
            Primitive::Detokenize => self.exec_detokenize(),
            Primitive::Unexpanded => self.exec_unexpanded(),
            // \number<number>：整数十进制展开（TeX 可展开原语）
            Primitive::Number => {
                let v = self.scan_number()?;
                self.emit_tokens(emit_count(v))
            }
            // 内部量：\eTeXversion/\eTeXrevision 可展开（\the 上下文由 the_tokens 读取）
            Primitive::ETeXVersion => self.emit_tokens(emit_count(2)),
            Primitive::ETeXRevision => {
                // e-TeX 2.6：revision 展开为 ".6"（版本号"2.6"的后半段，前导点用于 etrip.tex
                // 的 `\def\1.#1#2\relax` 分隔参数解析）
                self.emit_tokens(
                    ".6"
                        .bytes()
                        .map(|b| Token::char(Catcode::Other, u32::from(b)))
                        .collect(),
                )
            }
            // M4-3 数学字体族：\textfont<fam>=<fontcs>（直通 sink 分配）
            Primitive::TextFont | Primitive::ScriptFont | Primitive::ScriptScriptFont => {
                let kind = match prim {
                    Primitive::TextFont => 0,
                    Primitive::ScriptFont => 1,
                    _ => 2,
                };
                self.exec_math_font(kind)
            }
            // M4-6 断字：\patterns{...}（扫描组 + 直通 sink 文本）
            Primitive::Patterns => self.exec_patterns(),
            // 内部整数参数（\tracingstats 等 25 个）与交互模式命令（\batchmode 等 4 个）
            // 已由上方 int_param_index / interaction_mode_value 守卫分支处理；编译器
            // 不计守卫为覆盖，此处兜底仅满足穷尽性检查（未来新增原语会在此显式报错）。
            _ => Err(Error::internal(
                "未接入 exec_primitive 的原语（内部整数/交互模式应走守卫分支）",
            )),
        }
    }

    /// `\textfont<fam>=<fontcs>` 族分配：扫描 fam 号、可选 `=`、字体选择器 cs。
    fn exec_math_font(&mut self, kind: u8) -> Result<()> {
        let fam = self.scan_number()?;
        if !(0..=15).contains(&fam) {
            return Err(Error::invalid_input("数学字体族号必须为 0..15"));
        }
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
        let font = match self.eqtb.slot(csid) {
            EqSlot::Font(f) => *f,
            _ => {
                return Err(Error::invalid_input(format!(
                    "\\textfont 的 \\{} 不是字体选择器",
                    self.intern.name(csid)
                )));
            }
        };
        self.sink.math_font(kind, fam as u8, font)
    }

    /// `\patterns{...}`（M4-6）：扫描平衡组（不展开），抽取模式文本直通 sink。
    ///
    /// TeX `new_patterns`（tex.web）语义：字母/数字/`.` 是模式字符；
    /// 其余 token（空格、控制序列等）是模式分隔符。文本交由 ntex-layout 的
    /// Liang trie 解析（ntex-layout::hyphen::PatternTrie::parse）。
    fn exec_patterns(&mut self) -> Result<()> {
        let tokens = self.scan_group_contents()?;
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

    /// `\left`/`\right` 的定界符参数：字符 → charcode（`.` 为空定界符）；`\.` → None。
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
                } else {
                    Ok(Some(ch))
                }
            }
            TokenKind::ControlSeq => {
                let name = self.intern.name(tok.csid().expect("ControlSeq 必有 csid"));
                if name == "." {
                    Ok(None)
                } else {
                    Err(Error::invalid_input(format!(
                        "\\left/\\right 定界符暂不支持 \\{name}"
                    )))
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
        let font = self.font_loader.load(&font_name, at, scaled)?;
        // 组作用域 + \global 语义（同 \def）
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Eqtb {
                    csid,
                    prev: self.eqtb.slot(csid).clone(),
                },
            ));
        }
        self.eqtb.set_font(csid, font);
        self.finish_assignment();
        Ok(())
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

    /// 扫描字体标识符（TeX `scan_font_ident`）：`\font` 定义的 cs 或 `\nullfont`。
    fn scan_font_ident(&mut self) -> Result<u32> {
        self.skip_spaces()?;
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("预期字体标识符"))?
            .0;
        let csid = tok
            .csid()
            .ok_or_else(|| Error::invalid_input("预期字体标识符（\\font 定义的 cs 或 \\nullfont）"))?;
        match self.eqtb.slot(csid) {
            EqSlot::Font(f) => Ok(*f),
            _ => Err(Error::invalid_input(
                "预期字体标识符（\\font 定义的 cs 或 \\nullfont）",
            )),
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

    /// 读 fontdimen：覆盖表优先；无覆盖返回 0（TFM 真实参数在排版层，后续接入）。
    fn fontdimen(&self, font: u32, num: u32) -> i64 {
        self.fontdimens.get(&(font, num)).copied().unwrap_or(0)
    }

    // ---------- ETRIP 冲刺：终端转录（\message/\show/\showthe/\write16） ----------

    /// `\message{<general text>}`：展开参数后输出到终端与日志（TeX：不换行）。
    fn exec_message(&mut self) -> Result<()> {
        let toks = self.scan_group_contents()?;
        let s = self.expand_to_string(&toks)?;
        self.sink.message(s)
    }

    /// `\show<token>`：显示下一个 token 的含义（TeX："> \cs=..."，不展开）。
    fn exec_show(&mut self) -> Result<()> {
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\show 后缺少 token"))?
            .0;
        self.sink.show(format!("> {}", self.show_meaning(tok)))
    }

    /// `\showthe<内部量>`：显示内部量当前值（TeX："> \count0=5."）。
    fn exec_showthe(&mut self) -> Result<()> {
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\showthe 后缺少内部量"))?
            .0;
        let csid = tok
            .csid()
            .ok_or_else(|| Error::invalid_input("\\showthe 需要内部量参数"))?;
        let name = self.intern.name(csid).to_owned();
        let value_toks = self.the_tokens_after(tok)?;
        let value = detok_tokens(&value_toks, &self.intern);
        self.sink.show(format!("> \\{name}={value}."))
    }

    /// `\show` 的含义描述（字符 / 控制序列的 eqtb 槽含义）。
    fn show_meaning(&self, tok: Token) -> String {
        match tok.kind() {
            TokenKind::Char => meaning(tok, &self.intern),
            TokenKind::ControlSeq => {
                let csid = tok.csid().expect("ControlSeq 必有 csid");
                let name = self.intern.name(csid).to_owned();
                match self.eqtb.slot(csid).clone() {
                    EqSlot::Undefined => format!("\\{name}=undefined."),
                    EqSlot::Primitive(_) => format!("\\{name}=\\{name}."),
                    EqSlot::Macro(m) => {
                        let params: String = (1..=m.value.params.num_params)
                            .map(|n| format!("#{n}"))
                            .collect();
                        let body = detok_tokens(&m.value.body, &self.intern);
                        format!("\\{name}=macro:{params}->{body}.")
                    }
                    EqSlot::Char { catcode, charcode } => {
                        let ch = char::from_u32(charcode).unwrap_or('\u{FFFD}');
                        let desc = if catcode == Catcode::Letter {
                            format!("the letter {ch}")
                        } else {
                            format!("the character {ch}")
                        };
                        format!("\\{name}={desc}.")
                    }
                    EqSlot::Font(f) => format!("\\{name}=select font {f}."),
                    EqSlot::Register(k, n) => format!("\\{name}=\\{}{}.", reg_kind_name(k), n),
                    EqSlot::Stream(_, n) => format!("\\{name}=write{n}."),
                    EqSlot::Alias(_) => {
                        // \let 别名：沿链解析（防环）后显示目标槽含义
                        let mut id = csid;
                        let mut hops = 0;
                        while let EqSlot::Alias(t) = self.eqtb.slot(id) {
                            id = *t;
                            hops += 1;
                            if hops > 64 {
                                break;
                            }
                        }
                        self.show_meaning(Token::control_sequence(id))
                    }
                }
            }
            _ => String::new(),
        }
    }

    fn exec_catcode(&mut self) -> Result<()> {
        let byte = self.scan_char_code()?;
        let byte = u8::try_from(byte).map_err(|_| Error::invalid_input("\\catcode 字符码越界"))?;
        self.expect_equals()?;
        let code = self.scan_number()?;
        let cat = Catcode::from_u8(
            u8::try_from(code).map_err(|_| Error::invalid_input("catcode 必须在 0..=15"))?,
        )
        .ok_or_else(|| Error::invalid_input("catcode 必须在 0..=15"))?;
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Catcode {
                    byte,
                    prev: self.catcodes.get(byte),
                },
            ));
        }
        self.catcodes.set(byte, cat);
        self.finish_assignment();
        Ok(())
    }

    /// `\sfcode<字符>=<值>`：设置字符的 spacefactor（TeX define_char_code 类）。
    fn exec_sfcode(&mut self) -> Result<()> {
        let byte = self.scan_char_code()?;
        let byte = u8::try_from(byte).map_err(|_| Error::invalid_input("\\sfcode 字符码越界"))?;
        self.expect_equals()?;
        let value = self.scan_number()?;
        let value = u32::try_from(value).map_err(|_| Error::invalid_input("\\sfcode 值越界"))?;
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Sfcode {
                    byte,
                    prev: self.sfcodes[byte as usize],
                },
            ));
        }
        self.sfcodes[byte as usize] = value;
        self.sink.sfcode_changed(byte, value)?;
        self.finish_assignment();
        Ok(())
    }

    /// `\lccode<char>=<num>`：设置字符的小写码（TeX assign_int；etrip 断字用）。
    /// 字符码接受反引号或寄存器值（如 `\lccode\count20=0`，TeX scan_char_num）。
    fn exec_lccode(&mut self) -> Result<()> {
        let byte = self.scan_char_code()?;
        let byte = u8::try_from(byte).map_err(|_| Error::invalid_input("\\lccode 字符码越界"))?;
        self.expect_equals()?;
        let value = self.scan_number()?;
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::LcCode {
                    byte,
                    prev: self.lccodes[byte as usize],
                },
            ));
        }
        self.lccodes[byte as usize] = value;
        self.finish_assignment();
        Ok(())
    }

    /// `\advance<寄存器> <增量>`：寄存器运算（TeX arithmetic；etrip.tex 91 行
    /// `\advance\count20 1`）。目标支持 `\count/\dimen/\skip/\muskip` 寄存器
    /// （数字下标或 `\countdef` 等 cs 绑定）与内部整数参数。
    fn exec_advance(&mut self) -> Result<()> {
        self.skip_spaces()?;
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\advance 后缺少寄存器"))?
            .0;
        let csid = tok
            .csid()
            .ok_or_else(|| Error::invalid_input("\\advance 后必须是寄存器"))?;
        match self.eqtb.slot(csid).clone() {
            EqSlot::Primitive(prim)
                if matches!(
                    prim,
                    Primitive::Count | Primitive::Dimen | Primitive::Skip | Primitive::Muskip
                ) =>
            {
                let kind = match prim {
                    Primitive::Count => RegKind::Count,
                    Primitive::Dimen => RegKind::Dimen,
                    Primitive::Skip => RegKind::Skip,
                    _ => RegKind::Muskip,
                };
                let idx = self.scan_register_index()?;
                self.advance_register(kind, idx)
            }
            EqSlot::Register(kind, idx) => self.advance_register(kind, idx),
            // 内部整数参数（\tracingstats/\language 等）也可 \advance
            EqSlot::Primitive(p) if int_param_index(p).is_some() => {
                let idx = int_param_index(p).expect("已检查 is_some");
                let delta = self.scan_number()?;
                let val = self.params.misc[idx] + delta;
                let global = self.is_global();
                if !global && self.group_level > 0 {
                    self.save_stack.push((
                        self.group_level,
                        SavedValue::Param {
                            kind: ParamKind::MiscInt(idx),
                            prev: self.params.get(ParamKind::MiscInt(idx)),
                        },
                    ));
                }
                self.params.misc[idx] = val;
                self.finish_assignment();
                Ok(())
            }
            _ => Err(Error::invalid_input(
                "\\advance 目标必须是寄存器或内部参数",
            )),
        }
    }

    /// `\advance` 的寄存器增量应用（TeX：`new = old + delta`，胶水逐分量加）。
    fn advance_register(&mut self, kind: RegKind, idx: usize) -> Result<()> {
        match kind {
            RegKind::Count => {
                let delta = self.scan_number()?;
                self.assign_count(idx, self.registers.count(idx) + delta);
            }
            RegKind::Dimen => {
                let delta = self.scan_dimen()?;
                self.assign_dimen(idx, self.registers.dimen(idx) + delta);
            }
            RegKind::Skip => {
                let delta = self.scan_glue()?;
                let old = self.registers.skip(idx);
                self.assign_skip(
                    idx,
                    Glue {
                        width: old.width + delta.width,
                        stretch: old.stretch + delta.stretch,
                        shrink: old.shrink + delta.shrink,
                    },
                );
            }
            RegKind::Muskip => {
                let delta = self.scan_glue()?;
                let old = self.registers.muskip(idx);
                self.assign_muskip(
                    idx,
                    Glue {
                        width: old.width + delta.width,
                        stretch: old.stretch + delta.stretch,
                        shrink: old.shrink + delta.shrink,
                    },
                );
            }
            RegKind::Toks => return Err(Error::invalid_input("\\advance 不支持 \\toks")),
        }
        Ok(())
    }

}
