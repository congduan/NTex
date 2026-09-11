use crate::sink::DirectionKind;

/// 表达式原语种类（\numexpr/\dimexpr/\glueexpr/\muexpr）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum ExprKind {
    Num,
    Dim,
    Glue,
    Mu,
}

impl Expander {
    /// 命令位置表达式原语：首 token 是否"表达式起点"。
    /// 非起点（异类表达式原语、\let/\relax 等命令类、普通字符）→ 报
    /// "You can't use \<expr> in vertical mode."（TeX 垂直模式裸用语义；
    /// 避免 Missing number 恢复输出 0 污染后续——etrip L764 连续裸用）。
    fn expr_start_ok(&mut self, kind: ExprKind, tok: Token) -> Result<bool> {
        if let Some(c) = tok.charcode() {
            let b = c as u8;
            return Ok(b.is_ascii_digit() || matches!(b, b'.' | b'(' | b'+' | b'-'));
        }
        let Some(csid) = tok.csid() else {
            return Ok(false);
        };
        match self.eqtb.slot(csid) {
            EqSlot::Primitive(p) => Ok(match p {
                // 表达式原语：同类可嵌套（\numexpr\numexpr）；异类 → can't use
                Primitive::NumExpr => kind == ExprKind::Num,
                Primitive::Dimexpr => kind == ExprKind::Dim,
                Primitive::Glueexpr => kind == ExprKind::Glue,
                Primitive::Muexpr => kind == ExprKind::Mu,
                // 量类原语：整数/尺寸/胶水寄存器与 \the/\number 等 → 表达式起点
                Primitive::Count
                | Primitive::Dimen
                | Primitive::Skip
                | Primitive::Muskip
                | Primitive::Toks
                | Primitive::The
                | Primitive::Number
                | Primitive::GlueToMu
                | Primitive::MuToGlue => true,
                // 命令类（\let/\relax/定义/条件/组等）→ 垂直模式裸用 → can't use
                _ => false,
            }),
            // 宏（可能展开为量）/寄存器绑定 cs/未定义 → 放行（scan 展开或恢复）
            _ => Ok(true),
        }
    }

    /// 主分发：原 `exec_primitive` ~1400 行大 match 按主题拆为 7 个 dispatcher
    /// （详见同名 include! 分片文件）。简单 case（单行委托、单 token 控制）
    /// 保留在主 match；条件原语由 `process_one` 拦截，此处仅 Err 兜底。
    ///
    /// 主题 dispatcher 守卫由 `free.rs` 主题分类函数判断：
    /// `is_box_prim / is_param_prim / is_math_prim / is_expandable_prim /
    ///  is_align_prim / is_toks_state_prim / is_io_prim`。
    ///
    /// 新增原语流程：(1) Primitive 枚举加变体；(2) 选择归属主题；
    /// (3) 加入对应 `is_xxx_prim` matches! 列表；(4) 在该主题 dispatcher
    /// 函数加 case body。
    fn exec_primitive(&mut self, prim: Primitive) -> Result<()> {
        match prim {
            // ---- 控制流 / 前缀 ----
            Primitive::Relax => Ok(()),
            // 内部只读整数单独出现：no-op（TeX 在 no_mode——输出例程中——忽略；
            // 其余模式应报错，模式状态在排版器侧，M1 宽松处理）
            Primitive::Badness => Ok(()),
            Primitive::Expandafter => self.exec_expandafter(),
            Primitive::Noexpand => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\noexpand 后无 token"))?;
                self.push_frame(InputFrame::One {
                    tok: t.0,
                    noexpand: true,
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
            // \long：宏定义前缀（允许参数中含 \par；同 \outer 仅置前缀）
            Primitive::Long => {
                self.long_pending = true;
                Ok(())
            }
            Primitive::Let => self.exec_let(),
            // ---- 组 / 全局 / 收尾 ----
            Primitive::End => {
                if std::env::var_os("NTEX_END_DBG").is_some() {
                    eprintln!("[END] 执行 \\end（栈深 {}）", self.stack.len());
                }
                self.stack.clear();
                self.output_active = false;
                self.ended = true;
                // tex.web final_cleanup：\end 也报未闭合条件（\endinput 同款；
                // trip L442 \if 在 \write 组内被 \end 中断 → if 442 incomplete）
                self.report_incomplete_conditions();
                // TeX `\end` 收尾：flush 所有延迟写流（final_cleanup 语义）
                self.flush_writes()?;
                Ok(())
            }
            Primitive::Global => {
                self.global_pending = true;
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
            // M1-11 组
            Primitive::BeginGroup => {
                // \begingroup：半简单组（currentgrouptype=14）
                self.sink.semisimple_begin()?;
                self.begin_group()
            }
            Primitive::EndGroup => self.end_group(),
            // ---- 条件原语（process_one 拦截，不应到达此处） ----
            Primitive::If
            | Primitive::IfCat
            | Primitive::IfNum
            | Primitive::IfDim
            | Primitive::IfX
            | Primitive::IfOdd
            | Primitive::IfCase
            | Primitive::IfTrue
            | Primitive::IfFalse
            | Primitive::IfInner
            | Primitive::IfVMode
            | Primitive::IfHMode
            | Primitive::IfMMode
            | Primitive::IfEof
            | Primitive::IfVoid
            | Primitive::IfHBox
            | Primitive::IfVBox
            | Primitive::Else
            | Primitive::Fi
            | Primitive::Or => Err(Error::internal("条件原语不应到达 exec_primitive")),
            Primitive::IfDefined | Primitive::IfCsname | Primitive::IfPrimitive => {
                Err(Error::internal("条件原语不应到达 exec_primitive"))
            }
            Primitive::IfFontChar => Err(Error::internal("\\iffontchar 不应到达 exec_primitive")),
            // ---- 寄存器（保留 exec_register/the/muskip_param 助手路径） ----
            Primitive::Count
            | Primitive::Dimen
            | Primitive::Skip
            | Primitive::Muskip
            | Primitive::Toks => self.exec_register(prim),
            // ETRIP 冲刺：\thinmuskip/\medmuskip/\thickmuskip（muskip 寄存器 0/1/2）
            Primitive::ThinMuskip => self.exec_muskip_param(0),
            Primitive::MedMuskip => self.exec_muskip_param(1),
            Primitive::ThickMuskip => self.exec_muskip_param(2),
            Primitive::The => self.exec_the(),
            // ---- 字符代码与寄存器算术（已有助手方法，保留委托） ----
            Primitive::Catcode => self.exec_catcode(),
            Primitive::SfCode => self.exec_sfcode(),
            Primitive::LcCode => self.exec_lccode(),
            Primitive::Uccode => self.exec_uccode(),
            Primitive::Advance => self.exec_advance(),
            Primitive::Multiply | Primitive::Divide => self.exec_multiply_divide(prim),
            // TRIP 纯 VM 可展开原语（exec 实现保留在 primitive_codes.rs；
            // is_expandable() 为真，\edef/\write 上下文同样经 exec_primitive 到达）
            Primitive::RomanNumeral => self.exec_roman_numeral(),
            Primitive::Char => self.exec_char(),
            Primitive::Uppercase => self.exec_uppercase(),
            Primitive::Lowercase => self.exec_lowercase(),
            // pdfTeX \expanded：组内容按 \edef 语义全展开后放回输入（exec/展开
            // 上下文共用 scan_expanded_group；见 expr.rs exec_expanded）
            Primitive::Expanded => self.exec_expanded(),
            Primitive::EndInput => self.exec_endinput(),
            Primitive::Ignorespaces => self.exec_ignorespaces(),
            // ---- 字体（M3-4，已有助手方法，保留委托） ----
            Primitive::Font => self.exec_font(),
            Primitive::FontDimen => self.exec_fontdimen(),
            Primitive::HyphenChar => self.exec_hyphenchar(),
            Primitive::DelCode => self.exec_delcode(),
            Primitive::MathCode => self.exec_mathcode(),
            Primitive::SkewChar => self.exec_skewchar(),
            Primitive::FontName => self.exec_fontname(),
            Primitive::TextFont | Primitive::ScriptFont | Primitive::ScriptScriptFont => {
                let kind = match prim {
                    Primitive::TextFont => 0,
                    Primitive::ScriptFont => 1,
                    _ => 2,
                };
                self.exec_math_font(kind)
            }
            Primitive::FontCharWd
            | Primitive::FontCharHt
            | Primitive::FontCharDp
            | Primitive::FontCharIc => self.exec_fontchar_dimen(prim),
            // LaTeX 兼容第八刀：\pdfsetrandomseed<number>（写 misc 64 种子状态；
            // \pdfrandomseed 读、\pdfuniformdeviate 推进——pdfTeX 随机源种子接口）
            Primitive::PdfSetRandomSeed => {
                let v = self.scan_number()?;
                self.params.misc[PDF_RANDOM_SEED_IDX] = v;
                Ok(())
            }
            // M4-6 断字
            Primitive::Patterns => self.exec_patterns(),
            Primitive::Hyphenation => self.exec_hyphenation(),
            // ---- 主题 dispatcher 委托（详见同名分片文件） ----
            p if is_box_prim(p) => self.dispatch_box(p),
            p if is_param_prim(p) => self.dispatch_param(p),
            p if is_math_prim(p) => self.dispatch_math(p),
            p if is_expandable_prim(p) => self.dispatch_expandable(p),
            p if is_align_prim(p) => self.dispatch_align(p),
            p if is_toks_state_prim(p) => self.dispatch_toks_state(p),
            p if is_io_prim(p) => self.dispatch_io(p),
            // ---- 兜底（满足穷尽性检查；新原语需先登记到对应主题集） ----
            other => Err(Error::internal(format!(
                "未接入 exec_primitive 的原语 {other:?}（检查 7 个主题 dispatcher 守卫）"
            ))),
        }
    }

    fn exec_message(&mut self) -> Result<()> {
        let toks = self.scan_group_contents(None)?;
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
        let Some(csid) = tok.csid() else {
            // TeX：非内部量 → "! You can't use `X' after \the." + 恢复显示 0
            // （参考 log L216 `\showthe$`）。
            let what = tok
                .charcode()
                .and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_else(|| "token".to_owned());
            self.sink
                .write16(format!("! You can't use `{what}' after \\the.\n"))?;
            return self.sink.show("> 0.".to_owned());
        };
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
                    // tex.web show_eqtb：原语槽打印规范名（`\let\a\else` →
                    // `\a=\else.`，非 `\a=\a.`——见 primitive_name 注释）
                    EqSlot::Primitive(p) => format!("\\{name}=\\{}.", primitive_name(p)),
                    EqSlot::Macro(m) => {
                        let params: String = (1..=m.value.params.num_params)
                            .map(|n| format!("#{n}"))
                            .collect();
                        let body = detok_tokens(&m.value.body, &self.intern);
                        // 同 meaning_text：protected 前缀（tex.web print_meaning）
                        let head = if m.value.protected {
                            "\\protected macro:"
                        } else {
                            "macro:"
                        };
                        format!("\\{name}={head}{params}->{body}.")
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
                    // \mathchardef 绑定：TeX 显示为 \mathchar"XXXX（十六进制）
                    EqSlot::MathChar(code) => format!("\\{name}=\\mathchar\"{code:X}."),
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
    fn exec_endinput(&mut self) -> Result<()> {
        for i in (0..self.stack.len()).rev() {
            if matches!(self.stack[i], InputFrame::Source { .. }) {
                self.stack.truncate(i);
                if self
                    .stack
                    .iter()
                    .all(|f| !matches!(f, InputFrame::Source { .. }))
                {
                    self.ended = true;
                }
                break;
            }
        }
        Ok(())
    }

    /// TRIP：`\ignorespaces`：跳过后续空格 token（tex.web；trip.tex l.315）。
    fn exec_ignorespaces(&mut self) -> Result<()> {
        // TeX get_x_token 语义：跳过后续空格，且**展开宏**（\space 等）——
        // 展开结果若为空格继续跳。参考 log 中 \ignorespaces\space\space 的
        // 空格被吸收不打印 blank space（tracingcommands）；NTex 此前只跳
        // 空格 token，\space 展开的空格漏到主循环多打印（TRIP diff 之一）。
        loop {
            let Some((tok, _)) = self.fetch()? else {
                return Ok(());
            };
            match tok.catcode() {
                Some(Catcode::Space) => continue,
                _ => {
                    if let Some(csid) = tok.csid() {
                        if let EqSlot::Macro(def) = self.eqtb.slot(csid).clone() {
                            self.call_macro(csid, def.value.clone())?;
                            continue;
                        }
                    }
                    self.unread(tok);
                    return Ok(());
                }
            }
        }
    }

    /// TRIP：`\uccode<char>=<num>`：设置字符的大写码（tex.web assign_int；
    /// trip.tex l.211 `\uccode`m=`A`）。语义与 `\lccode` 一致。
    fn exec_uccode(&mut self) -> Result<()> {
        let mut byte = self.scan_char_code()?;
        if !(0..=255).contains(&byte) {
            let mut msg = "! Improper \\uccode.\n".to_string();
            if let Some((n, line)) = self.error_context() {
                msg.push_str(&format!("l.{n} {line}\n"));
            }
            let _ = self.sink.write16(msg);
            byte = 0;
        }
        let byte = byte as u8;
        self.expect_equals()?;
        let value = self.scan_number()?;
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::LcCode {
                    byte,
                    prev: self.uccodes[byte as usize],
                },
            ));
        }
        self.uccodes[byte as usize] = value;
        self.finish_assignment();
        Ok(())
    }

    /// `\meaning<token>`（可展开）：token 的含义文本（TeX 格式，无尾随句点）。
    fn exec_meaning(&mut self) -> Result<()> {
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\meaning 后缺少 token"))?
            .0;
        let text = self.meaning_text(tok);
        self.emit_tokens(
            text.bytes()
                .map(|b| Token::char(Catcode::Other, u32::from(b)))
                .collect(),
        )
    }

    /// `\meaning` 的含义描述（字符 / 控制序列的 eqtb 槽含义；TeX `\meaning` 格式）。
    fn meaning_text(&self, tok: Token) -> String {
        match tok.kind() {
            TokenKind::Char => meaning(tok, &self.intern),
            TokenKind::ControlSeq => {
                let csid = tok.csid().expect("ControlSeq 必有 csid");
                let name = self.intern.name(csid).to_owned();
                match self.eqtb.slot(csid).clone() {
                    EqSlot::Undefined => "undefined".to_owned(),
                    // tex.web print_cmd_chr：原语槽打印规范名（`\meaning\a`，
                    // `\let\a\else` → `\else`，非 `\a`——见 primitive_name 注释）
                    EqSlot::Primitive(p) => format!("\\{}", primitive_name(p)),
                    EqSlot::Macro(m) => {
                        // tex.web print_cmd_chr `call` 臂（L23707 `print("macro")`）
                        // + print_meaning（L6324-6327）：`macro:` 后接
                        // **token_show(参数文本)** 再接 `->` + 宏体。参数文本含
                        // 定界符与 `#n`（`\gdef\if@12{}` 的 `if`、`\def\a#1#2`
                        // 的 `#1#2`）——此前只渲染 `#n`，定界符丢失：
                        // `\uppercase{\gdef\if@12{}}` 的 `\meaning\if@` 误报
                        // `macro:->`（pdfTeX 为 `macro:if->`）。
                        let params = detok_tokens(&m.value.params.text, &self.intern);
                        let body = detok_tokens(&m.value.body, &self.intern);
                        // tex.web print_meaning（e-TeX）：protected 宏前缀
                        // `\protected`（长貌 `\long` 前缀需 MacroDef 记录 long
                        // 位，暂缺，维持现状）。expl3 `\cs_generate_variant`
                        // 的变体构造器选型（`\__cs_generate_variant:N`：
                        // meaning 前缀含 "pr" → `\cs_new_protected:Npe`，否则
                        // `\cs_new:Npe`）读的正是此前缀——缺它则全部变体
                        // 降级为非保护宏：`\tl_const:Ne` 在 `\expanded` 内被
                        // 展开，`\char_generate:nn` 查表守卫 `\exp_not:N \or:`
                        // 残留进 `\c__char_*_tl`，latex.ltx l.9386 触发
                        // Illegal parameter number。
                        let head = if m.value.protected {
                            "\\protected macro:"
                        } else {
                            "macro:"
                        };
                        format!("{head}{params}->{body}")
                    }
                    EqSlot::Char { catcode, charcode } => {
                        meaning(Token::char(catcode, charcode), &self.intern)
                    }
                    EqSlot::Font(_) => format!("select font {name}"),
                    EqSlot::Register(k, n) => format!("\\{}{}", reg_kind_name(k), n),
                    EqSlot::Stream(_, n) => format!("write{n}"),
                    EqSlot::MathChar(code) => format!("\\mathchar\"{code:X}"),
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
                        self.meaning_text(Token::control_sequence(id))
                    }
                }
            }
            _ => String::new(),
        }
    }

    /// `\mathchardef\cs=<num>`：绑定 cs 为数学字符（类<<15 | 族<<8 | 字符）。
    /// 越界值（<0 或 >32767）报 "! Bad mathchar code." 且不改变绑定（TeX 语义）。
    fn exec_mathchardef(&mut self) -> Result<()> {
        let csid = self.scan_cs_ident()?;
        self.expect_equals()?;
        let v = self.scan_number()?;
        if !(0..=0x7FFF).contains(&v) {
            let _ = self.sink.write16(format!("! Bad mathchar code ({v})."));
            return Ok(());
        }
        self.set_slot_scoped(csid, EqSlot::MathChar(v as u32));
        self.finish_assignment();
        Ok(())
    }

    /// `\showtokens{<general text>}`：展开后显示 token 列表（TeX："> <tokens>." + 换行）。
    fn exec_showtokens(&mut self) -> Result<()> {
        // TeX `<general text>`：`\showtokens\expandafter{#1}` 组前先展开可展开项
        let toks = self.scan_group_contents_expanding()?;
        let text = self.expand_to_string(&toks)?;
        self.sink.show(format!("> {text}."))
    }

    /// `\fontcharwd/ht/dp/ic<font><char>` 的字符码闸门：上界按**被查字体**判定
    /// （M9 中文刀 1）。
    ///
    /// - 8-bit 字体（TFM）→ 255：tex.web §1108 的 `! Bad character code (N).`
    ///   是硬口径，`reference/trip/tripin.log` 与 `fixtures/etrip/etrip.log`
    ///   均有参考块，**不许放宽**；
    /// - Unicode 直映字体（OpenType）→ 0x10FFFF：对齐 XeTeX，使
    ///   `\fontcharwd\zh"4E2D` 可用。
    ///
    /// **`\fontchar*` 有三个入口，必须共用本函数**（历史上只放宽了其中一处，
    /// 造成同一语义两套口径）：
    /// 1. [`Self::exec_fontchar_dimen`]——独立展开（`\fontcharwd\zh"4E2D`）；
    /// 2. `scan_dimen_inner` 的尺寸上下文臂（`\dimen0=\fontcharwd\zh"4E2D`）；
    /// 3. `the_tokens_after` 的 `\the` 臂（`\the\fontcharwd\zh"4E2D`）。
    ///
    /// 漏改的后果是隐蔽的：`\iffontchar` 判定「字体里有」，而 `\fontcharwd`
    /// 报「Bad character code」。回归锁见
    /// `crates/ntex-layout/tests/cjk_charcode.rs::otf_font_char_queries_share_unicode_range`。
    ///
    /// 越界时自行报错并返回 `None`，调用方沿用各自既有的 0 值恢复路径。
    fn fontchar_code(&mut self, font: u32, ch: i64) -> Option<u32> {
        let limit = i64::from(self.font_loader.char_code_limit(font));
        if !(0..=limit).contains(&ch) {
            self.report_error("Bad character code.");
            return None;
        }
        u32::try_from(ch).ok()
    }

    /// `\fontcharwd/ht/dp/ic<font><char>`：查询字体字符度量分量（sp）并展开为维度。
    /// 参数非法（字体标识符/字符码越界）→ 报 "! Bad character code." 并恢复
    /// （TeX 对 `\fontcharwd \fontcharht ...` 裸用同样报错继续）。
    ///
    /// 字符码上限按被查字体判定（M9 中文刀 1），闸门见 [`Self::fontchar_code`]。
    fn exec_fontchar_dimen(&mut self, prim: Primitive) -> Result<()> {
        let component = match prim {
            Primitive::FontCharWd => 0,
            Primitive::FontCharHt => 1,
            Primitive::FontCharDp => 2,
            _ => 3, // FontCharIc（italic correction：TFM 无此字段，恒 0）
        };
        let scanned = (|| -> Result<(u32, i64)> {
            let font = self.scan_font_ident()?;
            let ch = self.scan_number()?;
            Ok((font, ch))
        })();
        let (font, ch) = match scanned {
            Ok(v) => v,
            Err(_) => {
                self.report_error("Bad character code.");
                return Ok(());
            }
        };
        let Some(ch) = self.fontchar_code(font, ch) else {
            return Ok(());
        };
        let m = self.font_loader.char_metric(font, ch);
        let v = match component {
            0 => m.map(|x| x.0).unwrap_or(0),
            1 => m.map(|x| x.1).unwrap_or(0),
            _ => 0,
        };
        self.emit_tokens(emit_dimen(v))
    }

    /// `\showifs`：显示当前条件嵌套状态（e-TeX 诊断原语；简化格式）。
    fn exec_showifs(&mut self) -> Result<()> {
        let depth = self.cond_stack.len();
        self.sink
            .show(format!("{depth} conditionals are open (level \\currentiflevel)"))
    }

    /// `\parshape=<n> <indent1> <width1> ...`：设置段落形状（n≤0 清空）。
    fn exec_parshape(&mut self) -> Result<()> {
        let n = self.scan_number()?; // 可选 `=`
        if n <= 0 {
            self.parshape = Vec::new();
            return Ok(());
        }
        let mut shape = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let indent = self.scan_dimen()?;
            let width = self.scan_dimen()?;
            shape.push((indent, width));
        }
        self.parshape = shape;
        Ok(())
    }

    /// `\parshapelength/indent/dimen<n>` 取值（sp）。TeX 语义（etrip 实证）：
    /// - n ≤ 0 → 0；
    /// - indent/length：n > 行数 → 最后一行对应值；
    /// - dimen：n > 2×行数 → 按 (n-2·len) 奇偶回退到最后两个值之一。
    fn parshape_access(&self, n: i64, kind: u8) -> i64 {
        if n <= 0 {
            return 0;
        }
        let len = self.parshape.len() as i64;
        if len == 0 {
            return 0;
        }
        match kind {
            // 0 = indent（值 2n-1）；1 = length（值 2n）
            0 | 1 => {
                let line = if n > len { len } else { n };
                let (i, w) = self.parshape[(line - 1) as usize];
                if kind == 0 { i } else { w }
            }
            // 2 = dimen（值 n）
            _ => {
                let total = 2 * len;
                let idx = if n <= total {
                    n
                } else {
                    let diff = n - total;
                    if diff % 2 == 0 { total } else { total - 1 }
                };
                let (i, w) = self.parshape[((idx - 1) / 2) as usize];
                if idx % 2 == 1 { i } else { w }
            }
        }
    }

    /// `\readline<n>to\cs`：读流下一行（原始字符，不 token 化）——去尾随空格后
    /// 附加 `\endlinechar`（>=0 时）为字符 token 存入宏体（e-TeX readline 语义）。
    fn exec_readline(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\readline", 15)?;
        self.scan_keyword(|w| w == "to")?;
        self.skip_spaces()?;
        let t = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\readline 后缺少控制序列"))?
            .0;
        let csid = t
            .csid()
            .ok_or_else(|| Error::invalid_input("\\readline to 后必须是控制序列"))?;
        let line = {
            let Some(stream) = self.read_streams.get_mut(idx).and_then(|s| s.as_mut()) else {
                return Err(Error::invalid_input("\\readline 流未打开"));
            };
            if stream.pos >= stream.data.len() {
                return Err(Error::invalid_input("\\readline 到文件末尾（EOF）"));
            }
            let start = stream.pos;
            let end = stream.data[start..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|i| start + i)
                .unwrap_or(stream.data.len());
            let mut line = stream.data[start..end].to_vec();
            stream.pos = if end < stream.data.len() { end + 1 } else { end };
            // 去尾随空格（TeX readline：空白行尾去除；\r 一并处理）
            while matches!(line.last(), Some(b' ' | b'\t' | b'\r')) {
                line.pop();
            }
            // 附加 \endlinechar（TeX 语义：>0 时在行尾附加该字符）
            if let ParamValue::Number(eol) = self.params.get(ParamKind::EndlineChar) {
                if eol > 0 {
                    line.push(eol as u8);
                }
            }
            line
        };
        let toks: Vec<Token> = line
            .into_iter()
            .map(|b| Token::char(Catcode::Other, u32::from(b)))
            .collect();
        let def = MacroDef {
            params: ParamSpec {
                num_params: 0,
                long: false,
                text: Default::default(),
            },
            body: Arc::from(toks),
            code: None,
            protected: false,
            outer: false,
        };
        self.define_macro_scoped(csid, def);
        Ok(())
    }
}
