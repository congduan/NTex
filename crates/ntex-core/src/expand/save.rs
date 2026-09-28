impl Expander {
    /// 内部参数赋值（组作用域 + sink 镜像通知）。
    /// 值不变不 push（tex.web：\vsize.pt 等读值场景不产生 restoring——
    /// TRIP L152 \vsize.pt\global\vsize=16383.99... 若 push 读值，组结束
    /// restoring 2000 会覆盖 \global 钳制值 16383.99998）。
    fn assign_param(&mut self, kind: ParamKind, value: ParamValue) -> Result<()> {
        let global = self.is_global();
        // \interactionmode（MiscInt 19，e-TeX）：全局参数（组内赋值不恢复——
        // etrip.tex L240 `{\interactionmode=3}` 全局生效）；非法值（<0 或 >3）
        // 钳制保持当前（etrip L236-238 \interactionmode=-1/4 后校验 nonstop 通过）。
        // 注意：`current interactionmode (l.N): X` 是 etrip.tex \3 宏的 \typeout，
        // 引擎不自动显示。
        if kind == ParamKind::MiscInt(19) {
            if let ParamValue::Number(v) = value {
                let cur = self.params.misc[19];
                let clamped = if (0..=3).contains(&v) { v } else { cur };
                if clamped != cur {
                    self.params.set(kind, ParamValue::Number(clamped));
                    self.sink.param_changed(kind, ParamValue::Number(clamped))?;
                }
                // 非法值（<0 或 >3）：e-TeX 报错（etrip L237-238 `\interactionmode=-1/4`
                // → `! Bad interaction mode (-1).` + show_context 两行 + help），
                // 然后钳制保持当前。
                if clamped != v {
                    // 清报错锚点残留（scan_dimen 的锚点会污染 l.N 行号——这里
                    // 是数值扫描上下文，用当前 pos 反推即可）
                    self.error_anchor = None;
                    self.sink.report_error(&format!("Bad interaction mode ({v})."));
                    self.report_error_context();
                    self.sink.write16(
                        "Modes are 0=batch, 1=nonstop, 2=scroll, and\n3=errorstop. Proceed, and I'll ignore this case.\n"
                            .to_owned(),
                    )?;
                }
                self.finish_assignment();
                return Ok(());
            }
        }
        if !global && self.group_level > 0 && self.params.get(kind) != value {
            self.save_stack.push((
                self.group_level,
                SavedValue::Param {
                    kind,
                    prev: self.params.get(kind),
                },
            ));
        }
        // e-TeX \tracingassigns（misc 5）：参数赋值追踪（{changing X=old} 在
        // 赋值前（旧 misc5>0 才输出——\tracingassigns=1 本身只有 into）；
        // {into X=new}/{reassigning X=new} 在赋值后）
        let tracing_assigns = self.params.misc[5] > 0;
        let old = self.params.get(kind);
        if tracing_assigns && old != value {
            let name = param_name(kind);
            if let ParamValue::Number(n) = old {
                let _ = self
                    .sink
                    .write16(format!("{{changing \\{name}={n}}}\n"));
            }
        }
        self.params.set(kind, value);
        self.sink.param_changed(kind, value)?;
        if tracing_assigns || self.params.misc[5] > 0 {
            let name = param_name(kind);
            if let ParamValue::Number(n) = value {
                if old == value {
                    let _ = self
                        .sink
                        .write16(format!("{{reassigning \\{name}={n}}}\n"));
                } else {
                    let _ = self
                        .sink
                        .write16(format!("{{into \\{name}={n}}}\n"));
                }
            }
        }
        self.finish_assignment();
        Ok(())
    }

    /// 扫描 `\hbox`/`\vbox`/`\vtop` 的可选规格：`to <dimen>` 或 `spread <dimen>`。
    /// 返回 `(to, spread)`（单位 sp；无规格 = None/None）。
    fn scan_box_spec(&mut self) -> Result<(Option<i64>, Option<i64>)> {
        if let Some(kw) = self.scan_keyword(|w| w == "to" || w == "spread")? {
            let d = self.scan_dimen()?;
            if kw == "to" {
                Ok((Some(d), None))
            } else {
                Ok((None, Some(d)))
            }
        } else {
            Ok((None, None))
        }
    }

    /// 扫描 `\hrule`/`\vrule` 的可选规格：
    /// `height <dimen> depth <dimen> width <dimen>`（任意顺序、可省略）。
    /// TeX 缺省（tex.web scan_rule_specs）：`\hrule` → height=0.4pt、width=未定；
    /// `\vrule` → width=0.4pt；未定宽度以 [`crate::NULL_FLAG`] 表达（showbox 显示 `*`）。
    /// 返回 `[height, depth, width]`。
    fn scan_rule_specs(&mut self, prim: Primitive) -> Result<[i64; 3]> {
        let mut specs = match prim {
            Primitive::HRule => [26214, 0, crate::NULL_FLAG], // 0.4pt
            _ => [0, 0, 26214], // VRule
        };
        for _ in 0..3 {
            let Some(kw) = self.scan_keyword(|w| matches!(w, "height" | "depth" | "width"))?
            else {
                break;
            };
            let idx = match kw.as_str() {
                "height" => 0,
                "depth" => 1,
                _ => 2,
            };
            specs[idx] = self.scan_dimen()?;
        }
        Ok(specs)
    }

    // ---------- M1-10 寄存器 ----------

    /// `\count/\dimen/\skip/\muskip/\toks` 赋值。
    fn exec_register(&mut self, prim: Primitive) -> Result<()> {
        let kind = match prim {
            Primitive::Count => RegKind::Count,
            Primitive::Dimen => RegKind::Dimen,
            Primitive::Skip => RegKind::Skip,
            Primitive::Muskip => RegKind::Muskip,
            Primitive::Toks => RegKind::Toks,
            _ => unreachable!("exec_register 只处理寄存器原语"),
        };
        let idx = self.scan_register_target(kind)?;
        self.expect_equals()?;
        match prim {
            Primitive::Count => {
                let val = self.scan_number()?;
                self.assign_count(idx, val);
            }
            Primitive::Dimen => {
                let val = self.scan_dimen()?;
                self.assign_dimen(idx, val);
            }
            Primitive::Skip => {
                let val = self.scan_glue()?;
                self.assign_skip(idx, val);
            }
            Primitive::Muskip => {
                // mu 上下文：只认 mu 胶水（\muexpr/\muskip 前导合法；\skip/\glueexpr → Incompatible）
                let val = self.scan_glue_mu()?;
                self.assign_muskip(idx, val);
            }
            Primitive::Toks => {
                // RHS 可为 `{token list}` 或另一 toks 寄存器（内容复制，
                // TRIP L418 `\tokens\toks1`）；见 scan_toks_rhs。
                let val = self.scan_toks_rhs()?;
                self.assign_toks(idx, val);
            }
            _ => unreachable!("exec_register 只处理寄存器原语"),
        }
        Ok(())
    }

    /// `\thinmuskip/\medmuskip/\thickmuskip=<mu glue>`：muskip 寄存器 0/1/2 赋值。
    fn exec_muskip_param(&mut self, idx: usize) -> Result<()> {
        self.expect_equals()?;
        let val = self.scan_glue_mu()?;
        self.assign_muskip(idx, val);
        Ok(())
    }

    /// 扫描寄存器目标：数字下标（`\count0`）或 cs 引用（`\count\foo`，须已分配）。
    fn assign_count(&mut self, idx: usize, val: i64) {
        let global = self.is_global();
        // e-TeX \tracingassigns（misc 5）：\count 寄存器赋值追踪
        if self.params.misc[5] > 0 {
            self.trace_assign_register(
                "count",
                idx,
                global,
                &self.registers.count(idx).to_string(),
                &val.to_string(),
            );
        }
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Count {
                    idx,
                    prev: self.registers.count(idx),
                },
            ));
        }
        self.registers.set_count(idx, val);
        // 输出例程刀 5（G5 页号链）：count0..9 是 shipout 页标签/DVI bop 计数的
        // 取值源（tex.web ship_out L12694-12699 直接读 count(j)），赋值即推送镜像。
        if idx < 10 {
            let _ = self.sink.count_changed(idx, val);
        }
        self.finish_assignment();
    }

    fn assign_dimen(&mut self, idx: usize, val: i64) {
        let global = self.is_global();
        // e-TeX \tracingassigns（misc 5）：\dimen 寄存器赋值追踪
        if self.params.misc[5] > 0 {
            self.trace_assign_register(
                "dimen",
                idx,
                global,
                &format_dimen(self.registers.dimen(idx)),
                &format_dimen(val),
            );
        }
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Dimen {
                    idx,
                    prev: self.registers.dimen(idx),
                },
            ));
        }
        self.registers.set_dimen(idx, val);
        self.finish_assignment();
    }

    fn assign_skip(&mut self, idx: usize, val: Glue) {
        let global = self.is_global();
        // e-TeX \tracingassigns（misc 5）：\skip 寄存器赋值追踪
        if self.params.misc[5] > 0 {
            self.trace_assign_register(
                "skip",
                idx,
                global,
                &format_glue(self.registers.skip(idx)),
                &format_glue(val),
            );
        }
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Skip {
                    idx,
                    prev: self.registers.skip(idx),
                },
            ));
        }
        self.registers.set_skip(idx, val);
        self.finish_assignment();
    }

    fn assign_muskip(&mut self, idx: usize, val: Glue) {
        // \thinmuskip/\medmuskip/\thickmuskip（0/1/2）：通知排版器数学间距参数
        if idx < 3 {
            let _ = self.sink.muskip_param(idx, val);
        }
        let global = self.is_global();
        // e-TeX \tracingassigns（misc 5）：\muskip 寄存器赋值追踪
        if self.params.misc[5] > 0 {
            self.trace_assign_register(
                "muskip",
                idx,
                global,
                &format_mu_glue(self.registers.muskip(idx)),
                &format_mu_glue(val),
            );
        }
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Muskip {
                    idx,
                    prev: self.registers.muskip(idx),
                },
            ));
        }
        self.registers.set_muskip(idx, val);
        self.finish_assignment();
    }

    fn assign_toks(&mut self, idx: usize, val: TokenArray) {
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Toks {
                    idx,
                    prev: self.registers.toks(idx),
                },
            ));
        }
        self.registers.set_toks(idx, val);
        self.finish_assignment();
    }

    /// `\output=<general text>`：设置输出例程 token 列表（M3-5-3）。
    /// 通知排版器：fire_up 改道 box255 + 待执行；组内局部、可 `\global`。
    fn assign_output(&mut self, val: TokenArray) {
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Output {
                    prev: self.output_toks.clone(),
                },
            ));
        }
        self.output_toks = Some(val);
        let _ = self.sink.output_defined(true);
        self.finish_assignment();
    }

    /// `\the<寄存器>`：把寄存器值展开为 token 流。
    ///
    /// tex.web（L9395-9411）：scan_toks 的 xpand 展开器遇 `\the` **不走**
    /// `ins_the_toks`/`ins_list`，而是把 `the_toks` 产物**直接接进正在收集的
    /// token 表**（"Here we insert an entire token list created by |the_toks|
    /// without expanding it further"）——`\edef/\xdef` 体与 `\write/\message`
    /// 的 general text 里，`\the⟨toks⟩` 的内容原样落表：cs 保持冻结的宏
    /// token、组字符不过配平、条件原语不过条件机、`#` 不做参数处理。
    /// 主循环（非收集语境）才走 ins_list，产物照常展开执行（GT pdftex
    /// 1.40.29：`\edef\x{\the\T}` 体存 `\reinstallA` 且定义期不执行；
    /// `\message{[\the\T]}` 打 `\reinstallA `；主循环 `\the\T` 照常执行）。
    /// NTex 对应：展开收集语境（`suppress_expansion > 0`：\edef/\xdef 体、
    /// expand_region（\write/\message/\errmessage）、`\expanded`）帧位带
    /// noexpand 冻结标记——`scan_edef_body` 的 noexpand 裸推臂与
    /// `process_one` 的 noexpand 输出臂恰为 tex.web 的接表语义。
    fn exec_the(&mut self) -> Result<()> {
        let freeze = self.suppress_expansion > 0;
        let tokens = self.the_tokens()?;
        let items: Vec<(Token, bool)> = tokens.into_iter().map(|t| (t, freeze)).collect();
        self.push_frame(InputFrame::TokenList {
            items: Arc::from(items),
            pos: 0,
        });
        Ok(())
    }

    /// 计算 `\the` 的 token 序列（`\count/\dimen/\skip/\toks`）。
    fn the_tokens(&mut self) -> Result<Vec<Token>> {
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\the 后缺少参数"))?
            .0;
        self.the_tokens_after(tok)
    }

    /// `\the` 求值（token 已取出的变体；`\showthe` 复用）。
    fn the_tokens_after(&mut self, tok: Token) -> Result<Vec<Token>> {
        let csid = tok
            .csid()
            .ok_or_else(|| Error::invalid_input("\\the 需要寄存器参数"))?;
        // TeX：`\the` 位置走 get_x_token；宏和可展开原语都先展开（LaTeX
        // `\the\value{section}` 的 `\value` 即宏，展开为 `\c@section` 后才是
        // 内部整数；此前只展开原语，`\section`/`\item`/`\label` 链会在
        // `\the\value{...}` 报 "You can't use \the with this."）。
        let expandable = match self.eqtb.slot(csid) {
            EqSlot::Macro(_) => true,
            EqSlot::Primitive(p) if p.is_expandable() => true,
            _ => false,
        };
        if expandable {
            let mut expansion = Vec::new();
            self.expand_once((tok, false), &mut expansion)?;
            let Some((first, _)) = expansion.first().copied() else {
                return Ok(Vec::new());
            };
            // 展开结果若是普通字符文本（如 `\the\eTeXrevision` → ".6"），
            // 保留整段文本。若首 token 是 cs，则必须把整段回灌输入流再取操作数：
            // `\value{section}` 展开为 `\csname c@section\endcsname`，其中
            // `\csname` 需要从同一展开结果继续读名字和 `\endcsname`。
            if first.csid().is_none() {
                return Ok(expansion.into_iter().map(|(t, _)| t).collect());
            }
            self.push_frame(InputFrame::TokenList {
                items: Arc::from(expansion),
                pos: 0,
            });
            let Some((operand, _)) = self.fetch()? else {
                return Ok(Vec::new());
            };
            return self.the_tokens_after(operand);
        }
        match self.eqtb.slot(csid) {
            EqSlot::Primitive(p) => match p {
                Primitive::Count => {
                    let idx = self.scan_register_index()?;
                    Ok(emit_count(self.registers.count(idx)))
                }
                Primitive::Dimen => {
                    let idx = self.scan_register_index()?;
                    Ok(emit_dimen(self.registers.dimen(idx)))
                }
                Primitive::Skip => {
                    let idx = self.scan_register_index()?;
                    Ok(emit_glue(self.registers.skip(idx)))
                }
                Primitive::Muskip => {
                    let idx = self.scan_register_index()?;
                    Ok(emit_mu_glue(self.registers.muskip(idx)))
                }
                // \the\thinmuskip 等：muskip 寄存器 0/1/2
                Primitive::ThinMuskip => Ok(emit_mu_glue(self.registers.muskip(0))),
                Primitive::MedMuskip => Ok(emit_mu_glue(self.registers.muskip(1))),
                Primitive::ThickMuskip => Ok(emit_mu_glue(self.registers.muskip(2))),
                Primitive::Toks => {
                    let idx = self.scan_register_index()?;
                    // tex.web the_toks（L9395-9411）：`\the⟨toks⟩` 把寄存器内
                    // token 列表**原样接进产物**（app_toks），不转文本。执行/
                    // 冻结由调用方承担：xpand 收集语境（\edef/\message）带
                    // freeze 标志（expr.rs `\the` 冻结位，TRIP L419 参考转录
                    // "不执行"由此而来——冻结 token 经 token_show 串文本，与
                    // 字母 cs 的旧串行化输出逐字一致）；主循环 ins_list 照常
                    // 执行。旧实现把 cs 串行化为 `\`+名字 cat12 字符——对普通
                    // 字母 cs 名重扫描等价、单测全绿，但名字含空格的 cs
                    // （`\csname ...\space\endcsname` 惯用法，latex.ltx
                    // `\SetMathAlphabet@` 的 install 键）重扫描后裂成
                    // `cs(无空格名)+空格字符`，token 身份丢失 → `\in@` 的
                    // 定界匹配永不命中 → amsfonts/amssymb 载入报
                    // "Command `\mathfrak' not defined as a math alphabet."。
                    Ok(self.registers.toks(idx).to_vec())
                }
                // TRIP：\the\textfont/\scriptfont/\scriptscriptfont<n> → 数学族字体
                // （族号越界报 "! Bad number" 钳制 0；expander 侧无 font→cs 名映射，
                // 输出字体 id 文本，值被丢弃的场景足够——L269 `\the\scriptscriptfont-1`）。
                Primitive::TextFont | Primitive::ScriptFont | Primitive::ScriptScriptFont => {
                    let kind = match p {
                        Primitive::TextFont => 0,
                        Primitive::ScriptFont => 1,
                        _ => 2,
                    };
                    let n = self.scan_number()?;
                    if !(0..=15).contains(&n) {
                        let _ = self.sink.write16(format!(
                            "! Bad number ({}).\n\
                             Since I expected to read a number between 0 and 15,\n\
                             I changed this one to zero.\n",
                            n
                        ));
                    }
                    let fam = if (0..=15).contains(&n) { n as usize } else { 0 };
                    let f = self.math_fonts[kind][fam];
                    Ok(emit_count(f as i64))
                }
                Primitive::ParIndent
                | Primitive::BaselineSkip
                | Primitive::LineSkip
                | Primitive::LineSkipLimit
                | Primitive::HSize
                | Primitive::Tolerance
                | Primitive::VSize
                | Primitive::NullDelimiterSpace
                | Primitive::ScriptSpace
                | Primitive::OverfullRule
                | Primitive::VOffset
                | Primitive::HOffset
                | Primitive::TopSkip
                | Primitive::MaxDepth
                | Primitive::ParSkip
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
                | Primitive::HangIndent
                | Primitive::SpaceSkip
                | Primitive::TabSkip
                | Primitive::LastSkip
                | Primitive::SplitTopSkip
                | Primitive::Hfuzz
                | Primitive::Vfuzz
                | Primitive::BoxMaxDepth
                | Primitive::SplitMaxDepth
                | Primitive::EmergencyStretch
                | Primitive::DisplayIndent
                | Primitive::DelimiterShortfall
                | Primitive::MathSurround
                | Primitive::LastKern
                | Primitive::Mag => {
                    let kind = match p {
                        Primitive::ParIndent => ParamKind::ParIndent,
                        Primitive::BaselineSkip => ParamKind::BaselineSkip,
                        Primitive::LineSkip => ParamKind::LineSkip,
                        Primitive::LineSkipLimit => ParamKind::LineSkipLimit,
                        Primitive::HSize => ParamKind::HSize,
                        Primitive::Tolerance => ParamKind::Tolerance,
                        Primitive::VSize => ParamKind::VSize,
                        Primitive::TopSkip => ParamKind::TopSkip,
                        Primitive::MaxDepth => ParamKind::MaxDepth,
                        Primitive::ParSkip => ParamKind::ParSkip,
                        Primitive::AboveDisplaySkip => ParamKind::AboveDisplaySkip,
                        Primitive::BelowDisplaySkip => ParamKind::BelowDisplaySkip,
                        Primitive::AboveDisplayShortSkip => ParamKind::AboveDisplayShortSkip,
                        Primitive::BelowDisplayShortSkip => ParamKind::BelowDisplayShortSkip,
                        Primitive::PreDisplayPenalty => ParamKind::PreDisplayPenalty,
                        Primitive::PostDisplayPenalty => ParamKind::PostDisplayPenalty,
                        Primitive::EndlineChar => ParamKind::EndlineChar,
                        Primitive::NewlineChar => ParamKind::NewlineChar,
                        Primitive::DefaultHyphenChar => ParamKind::DefaultHyphenChar,
                        Primitive::DefaultSkewChar => ParamKind::DefaultSkewChar,
                        Primitive::HangIndent => ParamKind::HangIndent,
                        Primitive::SpaceSkip => ParamKind::SpaceSkip,
                        Primitive::TabSkip => ParamKind::TabSkip,
                        Primitive::LastSkip => ParamKind::LastSkip,
                        Primitive::SplitTopSkip => ParamKind::SplitTopSkip,
                        Primitive::Hfuzz => ParamKind::Hfuzz,
                        Primitive::Vfuzz => ParamKind::Vfuzz,
                        Primitive::BoxMaxDepth => ParamKind::BoxMaxDepth,
                        Primitive::SplitMaxDepth => ParamKind::SplitMaxDepth,
                        Primitive::EmergencyStretch => ParamKind::EmergencyStretch,
                        Primitive::DisplayIndent => ParamKind::DisplayIndent,
                        Primitive::DelimiterShortfall => ParamKind::DelimiterShortfall,
                        Primitive::MathSurround => ParamKind::MathSurround,
                        Primitive::LastKern => ParamKind::LastKern,
                        Primitive::NullDelimiterSpace => ParamKind::NullDelimiterSpace,
                        Primitive::ScriptSpace => ParamKind::ScriptSpace,
                        Primitive::OverfullRule => ParamKind::OverfullRule,
                        Primitive::VOffset => ParamKind::VOffset,
                        Primitive::HOffset => ParamKind::HOffset,
                        _ => ParamKind::Mag,
                    };
                    Ok(match self.params.get(kind) {
                        ParamValue::Dimen(v) => emit_dimen(v),
                        ParamValue::Glue(g) => emit_glue(g),
                        ParamValue::Number(v) => emit_count(v),
                    })
                }
                // ETRIP 冲刺：TeX/e-TeX 内部整数参数（misc 数组）
                p if int_param_index(*p).is_some() => {
                    let idx = int_param_index(*p).expect("已检查 is_some");
                    Ok(emit_count(self.params.misc[idx]))
                }
                // TRIP 冲刺：\the\spacefactor → 活参数实时查询（sink 侧维护；L277/L290/L293）
                Primitive::SpaceFactor => Ok(emit_count(self.query_sink_ref().space_factor())),
                // TRIP：\the\catcode`X → 当前 catcode 值（L295 `\the\catcode`J`）；
                // 刀 4：\utfinputmode=1 时 >255 码位查覆盖表（默认 letter）
                Primitive::Catcode => {
                    let code = self.scan_char_code()?;
                    let utf8 = self.params.misc[crate::param::MISC_UTF_INPUT_MODE] == 1;
                    let in_unicode = utf8
                        && (0..=crate::font::UNICODE_MAX_CHARCODE as i64).contains(&code);
                    if u8::try_from(code).is_err() && !in_unicode {
                        return Err(Error::invalid_input("\\catcode 字符码越界"));
                    }
                    let v = self.catcodes.get_codepoint(code as u32).as_u8();
                    Ok(emit_count(i64::from(v)))
                }
                // ETRIP 冲刺：e-TeX 只读整数（\the/\number 上下文，与 scan_number 对齐）
                Primitive::InputLineNo => Ok(emit_count(self.current_line_no() as i64)),
                Primitive::CurrentGroupLevel => Ok(emit_count(self.group_level as i64)),
                Primitive::CurrentGroupType => Ok(emit_count(self.query_sink_ref().current_group_type())),
                Primitive::LastNodeType => Ok(emit_count(self.query_sink_ref().last_node_type())),
                Primitive::CurrentIfLevel => Ok(emit_count(self.cond_stack.len() as i64)),
                Primitive::CurrentIfType => Ok(emit_count(self.cur_if_type as i64)),
                Primitive::CurrentIfBranch => Ok(emit_count(self.cur_if_branch as i64)),
                // M4-5 e-TeX：\numexpr 表达式、\eTeXversion/\eTeXrevision
                Primitive::NumExpr => Ok(emit_count(self.eval_int_expression()?)),
                Primitive::Dimexpr => Ok(emit_dimen(self.eval_dimen_expression()?)),
                Primitive::Glueexpr => Ok(emit_glue(self.eval_glue_expression(false)?)),
                // \the\muexpr：mu 胶水按 "X.0mu" 显示（对照 etrip.tex \the\muexpr 输出）
                Primitive::Muexpr => Ok(emit_mu_glue(self.eval_glue_expression(true)?)),
                Primitive::ETeXVersion => Ok(emit_count(2)),
                Primitive::ETeXRevision => Ok(".6"
                    .bytes()
                    .map(|b| Token::char(Catcode::Other, u32::from(b)))
                    .collect()),
                // LaTeX 兼容第八刀：pdfTeX 探测原语的 `\the` 读取
                // （\pdftexbanner/\pdfcreationdate 真实 pdfTeX 为可展开字符串量；
                //  \the 路径同样给值，避免 `\the\pdftexbanner` 报 Missing number）
                Primitive::PdfTeXVersion => Ok(emit_count(140)),
                Primitive::PdfTeXRevision => Ok(emit_count(25)),
                Primitive::PdfShellEscape => Ok(emit_count(0)),
                Primitive::PdfElapsedTime => Ok(emit_count(0)),
                Primitive::PdfRandomSeed => {
                    Ok(emit_count(self.params.misc[PDF_RANDOM_SEED_IDX]))
                }
                Primitive::PdfTeXBanner => Ok(pdf_banner_tokens()),
                Primitive::PdfCreationDate => {
                    let p = |q: Primitive| -> i64 {
                        self.params.misc[int_param_index(q).expect("日期时间参数在 misc 表")]
                    };
                    Ok(pdf_creation_date_tokens(
                        p(Primitive::Day),
                        p(Primitive::Month),
                        p(Primitive::Year),
                        p(Primitive::Time),
                    ))
                }
                Primitive::Badness => Ok(emit_count(0)),
                // \the\fontdimen<num><font>：字体参数值（sp）
                Primitive::FontDimen => {
                    let num = self.scan_number()?;
                    let num =
                        u32::try_from(num).map_err(|_| Error::invalid_input("\\fontdimen 参数号越界"))?;
                    let font = self.scan_font_ident()?;
                    // TRIP L404：参数号越界 → 报错并返回 0（TeX "Font \X has only N ..."）
                    if num >= 13 {
                        self.report_error("Font \\FONT? has only 13 fontdimen parameters.");
                        return Ok(emit_dimen(0));
                    }
                    Ok(emit_dimen(self.fontdimen(font, num)))
                }
                // \the\fontcharwd/ht/dp/ic<font><char>：字体字符度量分量（sp）
                //
                // 与 `exec_fontchar_dimen`、`scan_dimen_inner` 同语义：字符码闸门
                // 走 [`Self::fontchar_code`]（按被查字体判上界，M9 中文刀 1）。
                Primitive::FontCharWd
                | Primitive::FontCharHt
                | Primitive::FontCharDp
                | Primitive::FontCharIc => {
                    let component = match p {
                        Primitive::FontCharWd => 0,
                        Primitive::FontCharHt => 1,
                        Primitive::FontCharDp => 2,
                        _ => 3, // FontCharIc
                    };
                    let font = self.scan_font_ident()?;
                    let ch = self.scan_number()?;
                    let Some(ch) = self.fontchar_code(font, ch) else {
                        return Ok(emit_dimen(0));
                    };
                    let m = self.font_loader.char_metric(font, ch);
                    let v = match component {
                        0 => m.map(|x| x.0).unwrap_or(0),
                        1 => m.map(|x| x.1).unwrap_or(0),
                        2 => m.map(|x| x.2).unwrap_or(0),
                        _ => 0,
                    };
                    Ok(emit_dimen(v))
                }
                // \the\hyphenchar<font>：字体断字符（无覆盖 = 默认 45）
                Primitive::HyphenChar => {
                    let font = self.scan_font_ident()?;
                    Ok(emit_count(self.hyphenchars.get(&font).copied().unwrap_or(45)))
                }
                // \the\parshapelength/indent/dimen<n>：段落形状分量（尺寸）
                Primitive::ParshapeLength | Primitive::ParshapeIndent | Primitive::ParshapeDimen => {
                    let kind = match p {
                        Primitive::ParshapeIndent => 0,
                        Primitive::ParshapeLength => 1,
                        _ => 2,
                    };
                    let idx = self.scan_number()?;
                    Ok(emit_dimen(self.parshape_access(idx, kind)))
                }
                // ETRIP 第二波：\the\wd/\the\ht/\the\dp<n>：盒子寄存器尺寸（sp）
                Primitive::Wd | Primitive::Ht | Primitive::Dp => {
                    let dim = match p {
                        Primitive::Wd => 0,
                        Primitive::Ht => 1,
                        _ => 2,
                    };
                    let idx = self.scan_register_index()?;
                    Ok(emit_dimen(self.query_sink_ref().box_dim(idx, dim)))
                }
                // ETRIP 第二波：\the\lastpenalty：当前列表尾 penalty 值（无则 0）
                Primitive::LastPenalty => Ok(emit_count(self.query_sink_ref().last_penalty())),
                // ETRIP 第二波：\the\prevdepth：上一行 depth（sp；未定义 < -1000pt 输出原值）
                Primitive::PrevDepth => Ok(emit_dimen(self.params.prevdepth)),
                // ETRIP 第二波：\the\leftskip/\the\rightskip：段落悬挂胶水
                Primitive::LeftSkip => Ok(emit_glue(self.params.leftskip)),
                Primitive::RightSkip => Ok(emit_glue(self.params.rightskip)),
                // \the\parfillskip：段落末行填充胶水（KOMA setparsizes 的
                // \edef\f@parfillskip{\the\parfillskip} 依赖；载 KOMA 类即触发）
                Primitive::ParFillSkip => Ok(emit_glue(self.params.parfillskip)),
                // ETRIP 第二波：\the\interlinepenalty/\the\clubpenalty/
                // \the\widowpenalty/\the\displaywidowpenalty：行间/孤行/段首断页惩罚
                Primitive::InterLinePenalty => Ok(emit_count(self.params.interlinepenalty)),
                Primitive::ClubPenalty => Ok(emit_count(self.params.clubpenalty)),
                Primitive::WidowPenalty => Ok(emit_count(self.params.widowpenalty)),
                Primitive::DisplayWidowPenalty => Ok(emit_count(self.params.displaywidowpenalty)),
                // ETRIP 第二波：\the\mutoglue<mu 胶水>：mu 胶水转胶水（1mu = 1pt = 65536sp）
                Primitive::MuToGlue => {
                    let g = self.scan_glue_mu()?;
                    Ok(emit_glue(g))
                }
                // ETRIP 第二波：\the\gluetomu<胶水>：胶水转 mu 胶水
                Primitive::GlueToMu => {
                    let g = self.scan_glue()?;
                    Ok(emit_mu_glue(g))
                }
                // \the\delcode<num>：字符定界符码（无覆盖 = 0x500000 默认）
                Primitive::DelCode => {
                    let byte = self.scan_char_code()?;
                    let byte =
                        u8::try_from(byte).map_err(|_| Error::invalid_input("\\delcode 字符码越界"))?;
                    Ok(emit_count(i64::from(
                        self.delcodes.get(&u32::from(byte)).copied().unwrap_or(0x500000),
                    )))
                }
                // TRIP 冲刺：\the\mathcode<num>：字符数学码（无覆盖 = initex 默认）
                Primitive::MathCode => {
                    let byte = self.scan_char_code()?;
                    let byte =
                        u8::try_from(byte).map_err(|_| Error::invalid_input("\\mathcode 字符码越界"))?;
                    Ok(emit_count(i64::from(
                        self.mathcodes.get(&u32::from(byte)).copied().unwrap_or(0x8000),
                    )))
                }
                // \the\lccode<char>：字符的小写码
                Primitive::LcCode => {
                    let byte = self.scan_char_code()?;
                    let byte =
                        u8::try_from(byte).map_err(|_| Error::invalid_input("\\lccode 字符码越界"))?;
                    Ok(emit_count(self.lccodes[byte as usize]))
                }
                // 第二十刀（utf8.def l.148-154 `\uccode`\noexpand\~=\the\uccode`\~`）：
                // `\the` 补 uc/sf code 读臂——tex.web scan_something_internal 的
                // uc_code/sf_code 分支都是合法 `\the` 内部量（cat_code 臂已在
                // 上方 ETRIP 冲刺批次）。此前缺臂使 utf8.def 预载的
                // `\edef\reserved@a{…\the\uccode`\~…}` 落 `_ =>` 兜底错误，
                // 格式引导止步 l.22586（latex.ltx utf8 区，残留 \reserved@a
                // 半载 edef 现场）。
                Primitive::Uccode => {
                    let byte = self.scan_char_code()?;
                    let byte =
                        u8::try_from(byte).map_err(|_| Error::invalid_input("\\uccode 字符码越界"))?;
                    Ok(emit_count(self.uccodes[byte as usize]))
                }
                Primitive::SfCode => {
                    let byte = self.scan_char_code()?;
                    let byte =
                        u8::try_from(byte).map_err(|_| Error::invalid_input("\\sfcode 字符码越界"))?;
                    Ok(emit_count(i64::from(self.sfcodes[byte as usize])))
                }
                // \the\font：当前字体选择器（expander 侧无排版状态——NTex 简化返回
                // 空；trip.tex L30 `\showthe\font`，trip.log 参考为 preload 场景跳过）
                Primitive::Font => Ok(Vec::new()),
                // \\the\\output：输出例程 token 列表（trip.tex L60 `\\message{\\the\\output...}`）
                Primitive::Output => Ok(self.output_toks.clone().unwrap_or_default().to_vec()),
                // \\the\\interlinepenalties<idx> 等：惩罚数组元素（越界 → 0；
                // etrip L1206-1211 `\\the#1-1`/`\\the#10` 稀疏数组读取）
                Primitive::InterLinePenalties
                | Primitive::ClubPenalties
                | Primitive::WidowPenalties
                | Primitive::DisplayWidowPenalties => {
                    let kind: usize = match p {
                        Primitive::InterLinePenalties => 0,
                        Primitive::ClubPenalties => 1,
                        Primitive::WidowPenalties => 2,
                        _ => 3,
                    };
                    let idx = self.scan_number()?;
                    let arr = &self.penalty_arrays[kind];
                    let v = if idx >= 0 && (idx as usize) < arr.len() {
                        arr[idx as usize]
                    } else {
                        0
                    };
                    Ok(emit_count(v))
                }
                // \\the\\pagetotal/\\pagegoal/\\predisplaysize：页面 dimen 只读
                // （expander 无排版状态返回 0）；\\the\\pagestretch 系：页面 glue 参数
                Primitive::PageTotal | Primitive::PageGoal | Primitive::PreDisplaySize => {
                    Ok(emit_dimen(0))
                }
                // \\the\\everypar 等 toks 参数：输出存储的 token 列表
                Primitive::EveryPar
                | Primitive::EveryHBox
                | Primitive::EveryVBox
                | Primitive::EveryCr
                | Primitive::ErrHelp => Ok(match p {
                    Primitive::EveryPar => self.everypar_toks.clone(),
                    Primitive::EveryHBox => self.everyhbox_toks.clone(),
                    Primitive::EveryVBox => self.everyvbox_toks.clone(),
                    Primitive::EveryCr => self.everycr_toks.clone(),
                    _ => self.errhelp_toks.clone(),
                }),
                // \\the\\insertpenalties：int 只读（expander 无排版状态返回 0）
                Primitive::InsertPenalties => Ok(emit_count(0)),
                // TRIP 补全批次：\\the\\skewchar<font>：字体偏斜字符（无覆盖 = -1）
                Primitive::SkewChar => {
                    let font = self.scan_font_ident()?;
                    Ok(emit_count(self.skewchars.get(&font).copied().unwrap_or(-1)))
                }
                // TRIP 补全批次：\\the\\everymath / \\the\\everydisplay：数学注入 token 列表
                Primitive::EveryMath => Ok(self.everymath.clone()),
                // TRIP 补全批次：\\the\\everydisplay：显示数学注入 token 列表
                Primitive::EveryDisplay => Ok(self.everydisplay_toks.clone()),
                // \the\everyjob：toks 参数读回（latex.ltx L727
                // `\everyjob\expandafter{\the\everyjob\the\LaTeXReleaseInfo}` 在
                // scan_left_brace 的 filler 语义下展开时即需读值；NTex 的
                // \everyjob 赋值暂映射 toks 寄存器 0，读回同源）
                Primitive::EveryJob => Ok(self.registers.toks(0).to_vec()),
                // TRIP 补全批次：页面只读内部量（无排版状态返回 0）
                Primitive::DisplayWidth
                | Primitive::PageDepth
                | Primitive::PageFillLStretch
                | Primitive::PageShrink => Ok(emit_dimen(0)),
                Primitive::PageStretch
                | Primitive::PageFilStretch
                | Primitive::PageFillStretch => {
                    let g = match p {
                        Primitive::PageStretch => self.params.pagestretch,
                        Primitive::PageFilStretch => self.params.pagefilstretch,
                        _ => self.params.pagefillstretch,
                    };
                    Ok(emit_glue(g))
                }
                // \the\relax（shorthand_def 预绑目标的扫描中期读）：tex.web
                // @<Complain...@>——"! You can't use `\relax' after \the." +
                // "I'm forgetting what you said and using zero instead."，按
                // int 0 恢复（数字上下文里 "0" 被数字循环吸走，pdfTeX GT
                // mathchar/global 三案 2026-09-14；此前硬 Err 致测试 unwrap 崩）
                Primitive::Relax => {
                    // \the 操作数是 relax 别名（fp 内部 scan mark `\s__fp` 系）
                    // 时几乎必为 expl3 展开机器走错分支，栈/调用轨迹是唯一
                    // 定位手段（NTEX_STACK_DUMP 门控，见 dump_input_stack）。
                    self.dump_input_stack("the-relax");
                    let _ = self
                        .sink
                        .write16("! You can't use `\\relax' after \\the.\nI'm forgetting what you said and using zero instead.\n".to_owned());
                    Ok(emit_count(0))
                }
                _ => Err(Error::invalid_input(
                    "\\the 只支持 \\count\\dimen\\skip\\toks 与内部参数",
                )),
            },
            // \chardef'd cs：\the\x → 字符码
            EqSlot::Char { charcode, .. } => Ok(emit_count(*charcode as i64)),
            // \mathchardef'd cs：\the\x → 数学字符码（十进制）
            EqSlot::MathChar(code) => Ok(emit_count(*code as i64)),
            // 寄存器引用 cs（\countdef\cs=<num> 等）：\the\cs → 寄存器值
            EqSlot::Register(k, idx) => Ok(match k {
                RegKind::Count => emit_count(self.registers.count(*idx)),
                RegKind::Dimen => emit_dimen(self.registers.dimen(*idx)),
                RegKind::Skip => emit_glue(self.registers.skip(*idx)),
                RegKind::Muskip => emit_mu_glue(self.registers.muskip(*idx)),
                RegKind::Toks => self.registers.toks(*idx).to_vec(),
            }),
            // \let 别名：沿链解析
            EqSlot::Alias(target) => {
                let t = Token::control_sequence(*target);
                self.the_tokens_after(t)
            }
            _ => {
                // TeX：\the 对不可用内部量 → 报错恢复（trip.tex L30 `\showthe\pageshrink`——
                // pageshrink 为排版状态量未接线，未定义/不可用恢复为空，不终止）
                let mut msg = "! You can't use \\the with this.\n".to_string();
                if let Some((n, line)) = self.error_context() {
                    msg.push_str(&format!("l.{n} {line}\n"));
                }
                let _ = self.sink.write16(msg);
                Ok(Vec::new())
            }
        }
    }

    /// 组作用域（M1-11）。
    fn begin_group(&mut self) -> Result<()> {
        self.group_level += 1;
        self.scope_stack.push(false);
        // 记录组开始时的条件栈深度：组结束时条件必须回到该深度（跨组开条件 → 错误）
        self.group_cond_depth.push(self.cond_stack.len());
        // M3-2：通知 sink 组开始（排版器据此构建盒子内容）；带进入行号
        // （\\tracinggroups 显示 `{entering X (level N) at line L}`；
        // output 例程组优先用 \\shipout 触发行号）
        let line = if self.output_trigger_line != 0 {
            let l = self.output_trigger_line;
            self.output_trigger_line = 0;
            l
        } else {
            self.current_line_no()
        } as u32;
        self.sink.group_begin(line)?;
        // tex.web prefixed_command `done:`：盒子实参（\setbox0=\vbox{…}、
        // \moveleft20pt\hbox{…}）的 `\afterassignment` 在 `scan_spec` 的
        // `new_save_level`+`scan_left_brace` 推入**盒子组**后、体首 token 前
        // 触发——体的排版在主循环，赋值本身到 `\egroup` 才完成（box_end）。
        // 此前该 token 不在此触发，泄漏到下一个无关赋值（LaTeX \vcenter@text
        // 靠它把 \aftergroup\…@auxii 挂到盒子组边界，再以 \box0 追加回数学
        // 列表；泄漏后 array/cases 的盒子从未回到 display 数学）。
        if self.pending_box_arg {
            self.finish_assignment();
        }
        Ok(())
    }

    fn end_group(&mut self) -> Result<()> {
        if self.group_level == 0 {
            // tex.web 组外闭合语义（pdfTeX 实测 2026-09-12）：
            // - `\endgroup` **原语**在组外 → `! Extra \endgroup.`；
            // - `}` **字符**在组外 → `! Too many }'s.`（TRIP L291）。
            // 二者文本不同但恢复动作相同（忽略并继续）。此前 `\endgroup`
            // 误报 Too many }'s.——语义矩阵 scan_toks_begingroup_counts 红即此。
            // 区分：本次闭合由 exec 原语臂进入（primitive=true）还是字符臂。
            let msg = if self.cur_group_close_via_primitive {
                "Extra \\endgroup."
            } else {
                "Too many }'s."
            };
            self.report_error(msg);
            return Ok(());
        }
        // \setbox/\moveleft 的 box 参数组（\vbox{} 等）在组结束时恢复追踪：
        // 参数组内容可能为空（\setbox255\vbox{}），模式变化检测不触发
        if self.pending_box_arg {
            self.trace_suppress -= 1;
            self.pending_box_arg = false;
            self.trace_suppress_defer = false;
        }
        // 条件栈与组栈相互独立（TeX：条件可跨组，如 `\begingroup\iftrue a\egroup\fi`，
        // ETRIP line 433 的 `\begingroup \iftrue \scantokens... \egroup \fi` 即依赖此语义）。
        let _ = self.group_cond_depth.pop();
        // 恢复本层保存的赋值
        while let Some((level, _)) = self.save_stack.last() {
            if *level != self.group_level {
                break;
            }
            let (_, v) = self.save_stack.pop().expect("last() 已检查非空");
            self.restore(v);
        }
        // 触发 \aftergroup（挂最内层作用域 = 本组；数学层同栈计数）
        let scope = self.scope_stack.len() as u32;
        let tokens: Vec<Token> = self
            .aftergroup
            .iter()
            .filter(|(l, _)| *l == scope)
            .map(|(_, t)| *t)
            .collect();
        self.aftergroup.retain(|(l, _)| *l != scope);
        self.scope_stack.pop();
        if !tokens.is_empty() {
            let items: Vec<(Token, bool)> = tokens.into_iter().map(|t| (t, false)).collect();
            self.push_frame(InputFrame::TokenList {
                items: Arc::from(items),
                pos: 0,
            });
        }
        self.group_level -= 1;
        // M3-2：通知 sink 组结束（排版器封装盒子内容）。
        // 放在 `\aftergroup` 之后：其 token 在组外上下文继续处理，不落入盒子。
        self.sink.group_end()
    }

    /// 数学作用域收尾（tex.web after_math → unsave）：弹出 `$` 开的
    /// math_shift_group 并落 `\aftergroup` token——LaTeX `\frozen@everymath`
    /// 的 `\aftergroup\@ignorefalse` 须在**闭合 `$` 处**执行，而不是泄漏到
    /// 外包盒子/对齐组的行界（`\halign{#\cr $x$ \cr}` 的 `}` 被顶成新行）。
    fn close_math_scope(&mut self) {
        if self.scope_stack.last() != Some(&true) {
            return; // 不平衡（错误恢复路径）：不动 VM 组的槽位
        }
        let scope = self.scope_stack.len() as u32;
        let tokens: Vec<Token> = self
            .aftergroup
            .iter()
            .filter(|(l, _)| *l == scope)
            .map(|(_, t)| *t)
            .collect();
        self.aftergroup.retain(|(l, _)| *l != scope);
        self.scope_stack.pop();
        if !tokens.is_empty() {
            let items: Vec<(Token, bool)> = tokens.into_iter().map(|t| (t, false)).collect();
            self.push_frame(InputFrame::TokenList {
                items: Arc::from(items),
                pos: 0,
            });
        }
    }

    fn restore(&mut self, v: SavedValue) {
        // retain 判定须在恢复前：eqtb 槽当前层级为全局（0）→ 丢弃该陈旧条目
        //（tex.web @<Store save_stack[save_ptr] in eqtb[p], unless eqtb[p]
        // holds a global value@> 的 "retaining" 分支）。
        let retain = matches!(&v, SavedValue::Eqtb { csid, .. } if self.eqtb.level(*csid) == 0);
        // \tracingrestores>0：恢复动作输出 `{restoring ...}` / `{retaining ...}`
        //（tex.web restore_trace + show_eqtb）。先用不可变借用构造消息体
        //（值即写回值），再执行恢复，最后写转录。
        let trace = if self.params.misc[4] > 0 {
            Some(self.restore_trace_body(&v))
        } else {
            None
        };
        match v {
            SavedValue::Eqtb {
                csid,
                prev,
                prev_level,
            } => {
                // 当前层级为全局 → retain（保留组内 \global 赋值，丢弃陈旧
                // save 条目）；否则恢复旧值连同旧层级。
                if !retain {
                    *self.eqtb.slot_mut(csid) = prev;
                    self.eqtb.set_level(csid, prev_level);
                }
            }
            // 组结束回滚同镜像（tex.web：count 寄存器在 eqtb 内，组结束还原旧值）
            SavedValue::Count { idx, prev } => {
                self.registers.set_count(idx, prev);
                if idx < 10 {
                    let _ = self.sink.count_changed(idx, prev);
                }
            }
            SavedValue::Dimen { idx, prev } => self.registers.set_dimen(idx, prev),
            SavedValue::Skip { idx, prev } => self.registers.set_skip(idx, prev),
            SavedValue::Muskip { idx, prev } => self.registers.set_muskip(idx, prev),
            SavedValue::Toks { idx, prev } => self.registers.set_toks(idx, prev),
            SavedValue::Catcode { byte, prev } => self.catcodes.set(byte, prev),
            SavedValue::UnicodeCatcode { cp, prev } => match prev {
                Some(c) => self.catcodes.set_codepoint(cp, c),
                None => self.catcodes.remove_codepoint(cp),
            },
            SavedValue::Param { kind, prev } => self.params.set(kind, prev),
            SavedValue::Sfcode { byte, prev } => {
                self.sfcodes[byte as usize] = prev;
                let _ = self.sink.sfcode_changed(byte, prev);
            }
            SavedValue::Output { prev } => {
                self.output_toks = prev;
                let _ = self.sink.output_defined(self.output_toks.is_some());
            }
            SavedValue::FontDimen { font, num, prev } => match prev {
                Some(v) => {
                    self.fontdimens.insert(font, num, v);
                }
                None => {
                    self.fontdimens.remove(font, num);
                }
            },
            // 当前字体回滚（tex.web cur_font_loc 是 eqtb 字，随组恢复）。
            // 只改 expander 镜像：排版侧 NodeBuilder 有自己的 font_stack
            // （sink.rs group_begin/group_end 同语义），不再发 font_selected
            // 事件，否则会把排版器已经回滚好的字体再改一次。
            SavedValue::CurFont { prev } => self.cur_font = prev,
            SavedValue::DelCode { byte, prev } => match prev {
                Some(v) => {
                    self.delcodes.insert(u32::from(byte), v);
                }
                None => {
                    self.delcodes.remove(&u32::from(byte));
                }
            },
            SavedValue::MathCode { byte, prev } => match prev {
                Some(v) => {
                    self.mathcodes.insert(u32::from(byte), v);
                }
                None => {
                    self.mathcodes.remove(&u32::from(byte));
                }
            },
            SavedValue::LcCode { byte, prev } => self.lccodes[byte as usize] = prev,
            SavedValue::UcCode { byte, prev } => self.uccodes[byte as usize] = prev,
            SavedValue::EveryToks { kind, prev } => {
                *match kind {
                    0 => &mut self.everypar_toks,
                    1 => &mut self.everyhbox_toks,
                    2 => &mut self.everyvbox_toks,
                    3 => &mut self.everycr_toks,
                    4 => &mut self.errhelp_toks,
                    5 => &mut self.everydisplay_toks,
                    _ => &mut self.everymath,
                } = prev;
            }
            SavedValue::PenaltyArray { kind, prev } => {
                if (kind as usize) < self.penalty_arrays.len() {
                    self.penalty_arrays[kind as usize] = prev;
                }
            }
        }
        if let Some(body) = trace {
            // \\tracingassigns 开启时组恢复不打 {restoring}（tex.web：恢复走
            // tracingassigns 的 changing/into 语义或静默；参考 etrip 无恢复行）
            if !body.is_empty() && self.params.misc[5] <= 0 {
                let verb = if retain { "retaining" } else { "restoring" };
                let _ = self.sink.write16(format!("{{{verb} {body}}}\n"));
            }
        }
    }

    // ---------- \tracingrestores 跟踪输出（tex.web restore_trace + show_eqtb） ----------

    /// 构造 `{restoring ...}` 的消息体（不含花括号与换行；tex.web `show_eqtb` 分区格式）。
    fn restore_trace_body(&self, v: &SavedValue) -> String {
        match v {
            SavedValue::Eqtb { csid, prev, .. } => {
                // tex.web restore_trace 用 print_cs（**总是带 escape**，单字符非字母
                // cs 如 \5 也显示 `\5`）——cs_name_display 会返裸字符，曾导致
                // `{restoring 5select font...}` 缺反斜杠；且 `=` 分隔符缺失。
                format!(
                    "{}={}",
                    self.esc(self.intern.name(*csid)),
                    self.slot_display(*csid, prev)
                )
            }
            SavedValue::Count { idx, prev } => format!("{}{}={}", self.esc("count"), idx, prev),
            SavedValue::Dimen { idx, prev } => {
                format!("{}{}={}pt", self.esc("dimen"), idx, format_dimen(*prev))
            }
            SavedValue::Skip { idx, prev } => {
                format!("{}{}={}", self.esc("skip"), idx, format_glue(*prev))
            }
            SavedValue::Muskip { idx, prev } => {
                format!("{}{}={}", self.esc("muskip"), idx, format_mu_glue(*prev))
            }
            SavedValue::Toks { idx, prev } => {
                format!("{}{}={}", self.esc("toks"), idx, self.show_toks(prev))
            }
            SavedValue::Catcode { byte, prev } => {
                format!("{}{}={}", self.esc("catcode"), byte, *prev as u8)
            }
            SavedValue::Param { kind, prev } => {
                format!(
                    "{}={}",
                    self.esc(param_name(*kind)),
                    param_value_display(*prev)
                )
            }
            SavedValue::Sfcode { byte, prev } => format!("{}{}={}", self.esc("sfcode"), byte, prev),
            SavedValue::Output { prev } => match prev {
                Some(t) => format!("{}={{{}}}", self.esc("output"), self.show_toks(t)),
                None => format!("{}=", self.esc("output")),
            },
            SavedValue::LcCode { byte, prev } => {
                format!("{}{}={}", self.esc("lccode"), byte, prev)
            }
            SavedValue::DelCode { byte, prev } => format!(
                "{}{}={}",
                self.esc("delcode"),
                byte,
                prev.unwrap_or(0)
            ),
            SavedValue::MathCode { byte, prev } => format!(
                "{}{}={}",
                self.esc("mathcode"),
                byte,
                prev.unwrap_or(0)
            ),
            // tex.web 恢复跟踪不覆盖：FontDimen/HyphenChar/SkewChar/PenaltyArray
            // （参考 trip.log 无对应 restoring 行）。
            _ => String::new(),
        }
    }

    /// 当前 escape 字符（`\escapechar`，misc[34]）。tex.web print_esc：仅
    /// `0<=esc<256` 才打印转义字符——负数（plain \newif 的 -1）与 256 都不可见。
    fn escape_char_str(&self) -> String {
        let esc = self.params.misc[34];
        if (0..=255).contains(&esc) {
            char::from_u32(esc as u32)
                .map(|c| c.to_string())
                .unwrap_or_default()
        } else {
            String::new()
        }
    }

    /// `print_esc(s)`：escape 字符 + 字符串。
    fn esc(&self, s: &str) -> String {
        format!("{}{}", self.escape_char_str(), s)
    }

    /// `print_cs` 语义：单字符非字母 cs 直接显示字符（无 escape）；
    /// 其余 escape + 名字（tex.web §5598）。
    fn cs_name_display(&self, csid: u32) -> String {
        let name = self.intern.name(csid);
        let b = name.as_bytes();
        if b.len() == 1 && !b[0].is_ascii_alphabetic() {
            name.to_string()
        } else {
            self.esc(name)
        }
    }

    /// cs 名完整显示（tex.web print_cs）：名字中控制字符 → `^^` 记法
    /// （^^@=0、^^A=1…^^_=31、^^?=127、>127 十六进制 `^^XX`）——outer 报错
    /// 消息（`\a^^@^^@a` 等 cat 12 特殊 cs 名）对齐参考。
    fn cs_display_name(&self, csid: u32) -> String {
        let name = self.intern.name(csid);
        let mut body = String::new();
        for &b in name.as_bytes() {
            match b {
                0..=31 => {
                    body.push_str("^^");
                    body.push(char::from(b'@' + b));
                }
                127 => body.push_str("^^?"),
                128..=255 => body.push_str(&format!("^^{b:02X}")),
                _ => body.push(b as char),
            }
        }
        let b = name.as_bytes();
        if b.len() == 1 && !b[0].is_ascii_alphabetic() {
            body
        } else {
            format!("{}{}", self.escape_char_str(), body)
        }
    }

    /// token 列表 → 可见文本（tex.web `show_token_list(..., null, 32)`：
    /// 32 项截断后补 `\ETC.`；字符取字符、cs 带 escape、宏参数 `#n`）。
    fn show_toks(&self, toks: &[Token]) -> String {
        let mut s = String::new();
        for (i, t) in toks.iter().enumerate() {
            if i >= 32 {
                s.push_str(&self.esc("ETC."));
                break;
            }
            match t.kind() {
                TokenKind::Char => {
                    if let Some(ch) = t.charcode().and_then(char::from_u32) {
                        if t.catcode() == Some(Catcode::Parameter) {
                            s.push(ch);
                        }
                        s.push(ch);
                    }
                }
                TokenKind::ControlSeq => {
                    if let Some(csid) = t.csid() {
                        s.push_str(&self.cs_name_display(csid));
                        // tex.web show_token_list：cs 后若非空格 token 则补
                        // 分隔空格（`macro:->\relax `；\detokenize 同规则）——
                        // 宏体尾 cs（后面是组结束/结尾）也补
                        let next_is_space = toks.get(i + 1).is_some_and(|nt| {
                            nt.charcode() == Some(u32::from(b' '))
                        });
                        if !next_is_space {
                            s.push(' ');
                        }
                    }
                }
                TokenKind::MacroParam => {
                    s.push('#');
                    if let Some(n) = t.param_number() {
                        s.push_str(&n.to_string());
                    }
                }
                TokenKind::EndGroup => {}
                TokenKind::EndTemplate => {}
            }
        }
        s
    }

    /// eqtb 槽值显示（tex.web `print_cmd_chr` + 宏体；无结尾点，供 `{restoring ...}`）。
    fn slot_display(&self, _csid: u32, slot: &EqSlot) -> String {
        match slot {
            EqSlot::Undefined => "undefined".to_string(),
            // 原语槽：显示规范名（`{restoring \box=\box}`；`\let\a\else` →
            // `\else`，pdfTeX 对拍 2026-09-11——见 primitive_name 注释）
            EqSlot::Primitive(p) => format!("\\{}", primitive_name(*p)),
            EqSlot::Macro(m) => {
                // tex.web print_cmd_chr `call` 臂（L23707 `print("macro")`）+
                // print_meaning（L6324-6327）：宏打印 = `macro:` 后接
                // **token_show(参数文本)** 再接 `->` + 宏体。参数文本含定界符
                // 与 `#n`（如 `\gdef\if@12{}` 的 `if`、`\def\a#1#2` 的 `#1#2`），
                // 此前只渲染 `#n` → 定界符丢失（`\uppercase{\gdef\if@12{}}`
                // 的 `\meaning\if@` 误报 `macro:->`，pdfTeX 为 `macro:if->`）。
                let params = self.show_toks(&m.value.params.text);
                format!("macro:{params}->{}", self.show_toks(&m.value.body))
            }
            // \let 到字符：`char"XX`（print_esc("char") + print_hex）
            EqSlot::Char { charcode, .. } => {
                format!("{}\\\"{:X}", self.esc("char"), charcode)
            }
            // 字体槽：`select font <名>`（tex.web print_font_identifier——
            // e-TeX \tracingassigns `{changing \6=select font nullfont}`）
            EqSlot::Font(f) => {
                let name = if *f == 0 {
                    "nullfont".to_string()
                } else {
                    self.font_names
                        .get(*f as usize)
                        .and_then(|n| n.clone())
                        .unwrap_or_else(|| f.to_string())
                };
                format!("select font {name}")
            }
            EqSlot::Register(k, n) => format!("{}{}{}", self.esc(reg_kind_name(*k)), n, ""),
            EqSlot::Stream(_, n) => format!("{}{}", self.esc("write"), n),
            EqSlot::MathChar(code) => format!("{}{:X}", self.esc("mathchar\""), code),
            EqSlot::Alias(target) => {
                // \let 别名：沿链解析（防环）后显示目标槽
                let mut id = *target;
                let mut hops = 0;
                while let EqSlot::Alias(t) = self.eqtb.slot(id) {
                    id = *t;
                    hops += 1;
                    if hops > 64 {
                        break;
                    }
                }
                let slot = self.eqtb.slot(id).clone();
                self.slot_display(id, &slot)
            }
        }
    }

    /// 带作用域的宏定义：组内局部保存 + `\afterassignment` 触发。
    fn define_macro_scoped(&mut self, csid: u32, def: MacroDef) {
        let global = self.is_global();
        // e-TeX \tracingassigns（misc 5）：宏定义追踪（define_macro_scoped 不走
        // set_slot_scoped——这里补钩子）
        let tracing = self.params.misc[5] > 0;
        let prev = if tracing {
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
                    prev_level: self.eqtb.level(csid),
                },
            ));
        }
        self.eqtb.define_macro(csid, def);
        self.eq_mark_level(csid, global);
        if tracing {
            let new = self.eqtb.slot(csid).clone();
            self.trace_assign(csid, global, prev.as_ref().expect("tracing 时已存"), &new);
        }
        self.finish_assignment();
    }
    /// cs 槽赋值后的层级登记（tex.web eq_level 最小两档化）：
    /// 全局赋值或底层组（group_level==0，tex.web cur_level=level_one）→ 0；
    /// 组内局部赋值 → 1。restore 侧以"当前层级==0"作 retain 守卫。
    fn eq_mark_level(&mut self, csid: u32, global: bool) {
        self.eqtb
            .set_level(csid, if global || self.group_level == 0 { 0 } else { 1 });
    }

    /// e-TeX \tracingassigns（misc 5）>0：赋值追踪（`{changing X=old}` +
    /// `{into X=new}`，全局为 `{globally changing ...}`；同值重新赋值为
    /// `{reassigning X=new}`——etrip L422-445 \tracingassigns 检查段）。
    fn trace_assign(&mut self, csid: u32, global: bool, prev: &EqSlot, new: &EqSlot) {
        // e-TeX 用 print_esc 显示 cs（\6 带反斜杠——cs_name_display 对单字符
        // 非字母返回裸字符，不适用）
        let name = self.esc(self.intern.name(csid));
        if prev == new {
            let _ = self.sink.write16(format!(
                "{{reassigning {name}={}}}\n",
                self.slot_display(csid, new)
            ));
        } else if global {
            let _ = self.sink.write16(format!(
                "{{globally changing {name}={}}}\n{{into {name}={}}}\n",
                self.slot_display(csid, prev),
                self.slot_display(csid, new)
            ));
        } else {
            let _ = self.sink.write16(format!(
                "{{changing {name}={}}}\n{{into {name}={}}}\n",
                self.slot_display(csid, prev),
                self.slot_display(csid, new)
            ));
        }
    }

    /// 带作用域的 eqtb 槽赋值（`\chardef`/`\countdef` 等；组内局部保存）。
    fn set_slot_scoped(&mut self, csid: u32, slot: EqSlot) {
        let global = self.is_global();
        self.set_slot_scoped_with(csid, slot, global, true);
    }

    /// 显式作用域版本（shorthand_def 两次绑定共用同一裁决时用）。
    /// `fire_after` 控制是否触发 `\afterassignment`——tex.web 的 after_assignment
    /// 在 prefixed_command **整条赋值结束后**触发一次，编号扫描前的临时 `\relax`
    /// 绑定不得提前引燃。
    fn set_slot_scoped_with(&mut self, csid: u32, slot: EqSlot, global: bool, fire_after: bool) {
        // e-TeX \tracingassigns（misc 5）：赋值追踪（changing/into/reassigning）
        if self.params.misc[5] > 0 {
            let prev = self.eqtb.slot(csid).clone();
            self.trace_assign(csid, global, &prev, &slot);
        }
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Eqtb {
                    csid,
                    prev: self.eqtb.slot(csid).clone(),
                    prev_level: self.eqtb.level(csid),
                },
            ));
        }
        *self.eqtb.slot_mut(csid) = slot;
        self.eq_mark_level(csid, global);
        if fire_after {
            self.finish_assignment();
        }
    }

    /// 只读版 `\global` 裁决（不消费旗标）。tex.web 的 `global_defs` 是**读取**
    /// 而非消费：shorthand_def 的临时 `define(p,relax,256)` 与最终 `define(p,a,..)`
    /// 同一裁决、同作用域。此前临时绑定走 `set_slot_scoped` 抢走了 `\global`
    /// 旗标，`\global\chardef\cs=\count15` 的最终绑定落成局部 → `\group_end:`
    /// 回滚成临时 `\relax` → expl3 `\ior_close:N`（`\cs_gset_eq:NN` 后重开流）
    /// 在 CaseFolding/SpecialCasing 段全链 `Missing number` 级联
    /// （2026-09-15 cd2 探针：pdfTeX `[out:\char"41]` vs NTex `[out:\relax]`）。
    fn peek_global(&self) -> bool {
        match self.params.misc[36] {
            n if n < 0 => false, // \globaldefs<0：显式 \global 也被取消
            0 => self.global_pending,
            _ => true, // \globaldefs>0：隐式 \global
        }
    }

    /// shorthand_def 编号扫描前的临时 `\relax` 绑定（tex.web L22906
    /// `define(p,relax,256)`）：**不消费 `\global`**、不引燃 `\afterassignment`
    /// （两者都是 prefixed_command 级语义，属于最终绑定）。
    fn set_slot_temp_relax(&mut self, csid: u32) {
        let global = self.peek_global();
        self.set_slot_scoped_with(csid, EqSlot::Primitive(Primitive::Relax), global, false);
    }

    /// e-TeX `\tracingassigns`：寄存器赋值追踪（`\count17=` 的
    /// changing/into/reassigning；tex.web 对寄存器赋值同样打点）。
    fn trace_assign_register(
        &mut self,
        cmd: &str,
        idx: usize,
        global: bool,
        prev: &str,
        new: &str,
    ) {
        if self.params.misc[5] <= 0 {
            return;
        }
        let name = format!("{}{}", self.esc(cmd), idx);
        if prev == new {
            let _ = self
                .sink
                .write16(format!("{{reassigning {name}={new}}}\n"));
        } else if global {
            let _ = self.sink.write16(format!(
                "{{globally changing {name}={prev}}}\n{{into {name}={new}}}\n"
            ));
        } else {
            let _ = self
                .sink
                .write16(format!("{{changing {name}={prev}}}\n{{into {name}={new}}}\n"));
        }
    }

    /// 消费 `\global` 前缀（每个赋值只消费一次）。
    ///
    /// tex.web `prefixed_command` 的 `\globaldefs` 调整：`>0` 时所有赋值
    /// 隐式全局化；`<0` 时取消显式 `\global`（局部化）。所有赋值路径（def/let/
    /// 寄存器/chardef 族/字体/参数）都经此处取作用域，是唯一收口点。
    fn is_global(&mut self) -> bool {
        let g = self.global_pending;
        self.global_pending = false;
        match self.params.misc[36] {
            n if n < 0 => false, // \globaldefs<0：显式 \global 也被取消
            0 => g,
            _ => true, // \globaldefs>0：隐式 \global
        }
    }

    /// 赋值完成后触发 `\\afterassignment`。
    fn finish_assignment(&mut self) {
        if let Some(tok) = self.afterassignment.take() {
            self.unread(tok);
        }
    }

}

// ---------- \tracingrestores 显示辅助（自由函数） ----------

/// 内部参数显示名（tex.web `print_param`/`print_length_param`/`print_skip_param`）。
fn param_name(kind: ParamKind) -> &'static str {
    use ParamKind::*;
    match kind {
        ParIndent => "parindent",
        BaselineSkip => "baselineskip",
        LineSkip => "lineskip",
        LineSkipLimit => "lineskiplimit",
        HSize => "hsize",
        Tolerance => "tolerance",
        VSize => "vsize",
        TopSkip => "topskip",
        MaxDepth => "maxdepth",
        ParSkip => "parskip",
        ParFillSkip => "parfillskip",
        XSpaceSkip => "xspaceskip",
        AboveDisplaySkip => "abovedisplayskip",
        BelowDisplaySkip => "belowdisplayskip",
        AboveDisplayShortSkip => "abovedisplayshortskip",
        BelowDisplayShortSkip => "belowdisplayshortskip",
        PreDisplayPenalty => "predisplaypenalty",
        PostDisplayPenalty => "postdisplaypenalty",
        LeftSkip => "leftskip",
        RightSkip => "rightskip",
        PrevDepth => "prevdepth",
        InterLinePenalty => "interlinepenalty",
        ClubPenalty => "clubpenalty",
        WidowPenalty => "widowpenalty",
        DisplayWidowPenalty => "displaywidowpenalty",
        HangIndent => "hangindent",
        SpaceSkip => "spaceskip",
        TabSkip => "tabskip",
        LastSkip => "lastskip",
        Hfuzz => "hfuzz",
        Vfuzz => "vfuzz",
        BoxMaxDepth => "boxmaxdepth",
        SplitMaxDepth => "splitmaxdepth",
        SplitTopSkip => "splittopskip",
        EmergencyStretch => "emergencystretch",
        DisplayIndent => "displayindent",
        DelimiterShortfall => "delimitershortfall",
        MathSurround => "mathsurround",
        LastKern => "lastkern",
        PageStretch => "pagestretch",
        PageFilStretch => "pagefilstretch",
        PageFillStretch => "pagefillstretch",
        EndlineChar => "endlinechar",
        NewlineChar => "newlinechar",
        DefaultHyphenChar => "defaulthyphenchar",
        DefaultSkewChar => "defaultskewchar",
        Mag => "mag",
        NullDelimiterSpace => "nulldelimiterspace",
        ScriptSpace => "scriptspace",
        OverfullRule => "overfullrule",
        VOffset => "voffset",
        HOffset => "hoffset",
        MiscInt(idx) => misc_int_name(idx),
    }
}

/// 参数值显示（`{restoring \lineskip=0.0pt plus 40.0pt}` 等；`\the` 同格式）。
fn param_value_display(v: ParamValue) -> String {
    match v {
        ParamValue::Number(n) => n.to_string(),
        ParamValue::Dimen(d) => format!("{}pt", format_dimen(d)),
        ParamValue::Glue(g) => format_glue(g),
    }
}

/// 内部整数参数（misc 数组）显示名（与 [`crate::expand::int_param_index`] 反向）。
fn misc_int_name(idx: usize) -> &'static str {
    match idx {
        0 => "tracingstats",
        1 => "tracinglostchars",
        2 => "tracingonline",
        3 => "tracingcommands",
        4 => "tracingrestores",
        5 => "tracingassigns",
        6 => "tracinggroups",
        7 => "tracingifs",
        8 => "tracingscantokens",
        9 => "tracingnesting",
        10 => "lefthyphenmin",
        11 => "righthyphenmin",
        12 => "hbadness",
        13 => "pretolerance",
        14 => "showboxdepth",
        15 => "showboxbreadth",
        16 => "language",
        17 => "savinghyphcodes",
        18 => "savingvdiscards",
        19 => "interactionmode",
        20 => "texxetstate",
        22 => "lastlinefit",
        23 => "predisplaydirection",
        // 24 曾映射 everyeof，已改 toks 字段语义（占位保留）
        25 => "deadcycles",
        26 => "tracingmacros",
        27 => "tracingoutput",
        28 => "errorcontextlines",
        29 => "tracingparagraphs",
        30 => "pagediscards",
        31 => "splitdiscards",
        32 => "lostchars",
        33 => "delimiterfactor",
        34 => "escapechar",
        35 => "vbadness",
        36 => "globaldefs",
        37 => "floatingpenalty",
        38 => "linepenalty",
        39 => "binoppenalty",
        40 => "relpenalty",
        41 => "adjdemerits",
        42 => "looseness",
        43 => "maxdeadcycles",
        44 => "hangafter",
        45 => "uchyph",
        46 => "fam",
        47 => "hyphenpenalty",
        48 => "doublehyphendemerits",
        49 => "finalhyphendemerits",
        50 => "holdinginserts",
        51 => "prevgraf",
        52 => "insertpenalties",
        53 => "day",
        54 => "month",
        55 => "year",
        56 => "time",
        57 => "brokenpenalty",
        58 => "exhyphenpenalty",
        59 => "tracingpages",
        60 => "parshape",
        // LaTeX 兼容第八刀：pdfTeX 原语状态（misc 63/64）
        63 => "pdfoutput",
        64 => "pdfrandomseed",
        // 图片管线 Step A：\pdflastximage（misc 67）
        67 => "pdflastximage",
        _ => "?",
    }
}
