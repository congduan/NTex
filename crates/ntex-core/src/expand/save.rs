impl Expander {
    /// 内部参数赋值（组作用域 + sink 镜像通知）。
    fn assign_param(&mut self, kind: ParamKind, value: ParamValue) -> Result<()> {
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Param {
                    kind,
                    prev: self.params.get(kind),
                },
            ));
        }
        self.params.set(kind, value);
        self.sink.param_changed(kind, value)?;
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
    /// `height <dimen> depth <dimen> width <dimen>`（任意顺序、可省略，缺省 0）。
    /// 返回 `[height, depth, width]`。
    fn scan_rule_specs(&mut self) -> Result<[i64; 3]> {
        let mut specs = [0i64; 3];
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

    /// `\def`/`\edef`：扫描控制序列名 + 参数文本 + 替换文本并定义。
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
                let val = self.scan_glue()?;
                self.assign_muskip(idx, val);
            }
            Primitive::Toks => {
                let val = self.scan_group_contents()?;
                self.assign_toks(idx, Arc::from(val));
            }
            _ => unreachable!("exec_register 只处理寄存器原语"),
        }
        Ok(())
    }

    /// `\thinmuskip/\medmuskip/\thickmuskip=<mu glue>`：muskip 寄存器 0/1/2 赋值。
    fn exec_muskip_param(&mut self, idx: usize) -> Result<()> {
        self.expect_equals()?;
        let val = self.scan_glue()?;
        self.assign_muskip(idx, val);
        Ok(())
    }

    /// 扫描寄存器目标：数字下标（`\count0`）或 cs 引用（`\count\foo`，须已分配）。
    fn assign_count(&mut self, idx: usize, val: i64) {
        let global = self.is_global();
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
        self.finish_assignment();
    }

    fn assign_dimen(&mut self, idx: usize, val: i64) {
        let global = self.is_global();
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
        let global = self.is_global();
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
    fn exec_the(&mut self) -> Result<()> {
        let tokens = self.the_tokens()?;
        let items: Vec<(Token, bool)> = tokens.into_iter().map(|t| (t, false)).collect();
        self.stack.push(InputFrame::TokenList {
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
                    Ok(self.registers.toks(idx).to_vec())
                }
                Primitive::ParIndent
                | Primitive::BaselineSkip
                | Primitive::LineSkip
                | Primitive::LineSkipLimit
                | Primitive::HSize
                | Primitive::Tolerance
                | Primitive::VSize
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
                | Primitive::DefaultSkewChar => {
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
                        _ => unreachable!("\\the 参数匹配已穷举"),
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
                // M4-5 e-TeX：\numexpr 表达式、\eTeXversion/\eTeXrevision
                Primitive::NumExpr => Ok(emit_count(self.eval_int_expression()?)),
                Primitive::Dimexpr => Ok(emit_dimen(self.eval_dimen_expression()?)),
                Primitive::Glueexpr => Ok(emit_glue(self.eval_glue_expression()?)),
                Primitive::ETeXVersion => Ok(emit_count(2)),
                Primitive::ETeXRevision => Ok(".6"
                    .bytes()
                    .map(|b| Token::char(Catcode::Other, u32::from(b)))
                    .collect()),
                Primitive::Badness => Ok(emit_count(0)),
                // \the\fontdimen<num><font>：字体参数值（sp）
                Primitive::FontDimen => {
                    let num = self.scan_number()?;
                    let num =
                        u32::try_from(num).map_err(|_| Error::invalid_input("\\fontdimen 参数号越界"))?;
                    let font = self.scan_font_ident()?;
                    Ok(emit_dimen(self.fontdimen(font, num)))
                }
                // \the\hyphenchar<font>：字体断字符（无覆盖 = 默认 45）
                Primitive::HyphenChar => {
                    let font = self.scan_font_ident()?;
                    Ok(emit_count(self.hyphenchars.get(&font).copied().unwrap_or(45)))
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
                // \the\lccode<char>：字符的小写码
                Primitive::LcCode => {
                    let byte = self.scan_char_code()?;
                    let byte =
                        u8::try_from(byte).map_err(|_| Error::invalid_input("\\lccode 字符码越界"))?;
                    Ok(emit_count(self.lccodes[byte as usize]))
                }
                _ => Err(Error::invalid_input(
                    "\\the 只支持 \\count\\dimen\\skip\\toks 与内部参数",
                )),
            },
            // \chardef'd cs：\the\x → 字符码
            EqSlot::Char { charcode, .. } => Ok(emit_count(*charcode as i64)),
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
            _ => Err(Error::invalid_input("\\the 需要寄存器参数")),
        }
    }

    /// 组作用域（M1-11）。
    fn begin_group(&mut self) -> Result<()> {
        self.group_level += 1;
        // 记录组开始时的条件栈深度：组结束时条件必须回到该深度（跨组开条件 → 错误）
        self.group_cond_depth.push(self.cond_stack.len());
        // M3-2：通知 sink 组开始（排版器据此构建盒子内容）
        self.sink.group_begin()
    }

    fn end_group(&mut self) -> Result<()> {
        if self.group_level == 0 {
            return Err(Error::invalid_input("多余的 }"));
        }
        let cond_depth = self
            .group_cond_depth
            .pop()
            .expect("begin_group 与 end_group 必须配对");
        if self.cond_stack.len() != cond_depth {
            return Err(Error::invalid_input("组内条件未闭合（缺少 \\fi）"));
        }
        // 恢复本层保存的赋值
        while let Some((level, _)) = self.save_stack.last() {
            if *level != self.group_level {
                break;
            }
            let (_, v) = self.save_stack.pop().expect("last() 已检查非空");
            self.restore(v);
        }
        // 触发 \aftergroup
        let tokens: Vec<Token> = self
            .aftergroup
            .iter()
            .filter(|(l, _)| *l == self.group_level)
            .map(|(_, t)| *t)
            .collect();
        self.aftergroup.retain(|(l, _)| *l != self.group_level);
        if !tokens.is_empty() {
            let items: Vec<(Token, bool)> = tokens.into_iter().map(|t| (t, false)).collect();
            self.stack.push(InputFrame::TokenList {
                items: Arc::from(items),
                pos: 0,
            });
        }
        self.group_level -= 1;
        // M3-2：通知 sink 组结束（排版器封装盒子内容）。
        // 放在 `\aftergroup` 之后：其 token 在组外上下文继续处理，不落入盒子。
        self.sink.group_end()
    }

    fn restore(&mut self, v: SavedValue) {
        match v {
            SavedValue::Eqtb { csid, prev } => {
                *self.eqtb.slot_mut(csid) = prev;
            }
            SavedValue::Count { idx, prev } => self.registers.set_count(idx, prev),
            SavedValue::Dimen { idx, prev } => self.registers.set_dimen(idx, prev),
            SavedValue::Skip { idx, prev } => self.registers.set_skip(idx, prev),
            SavedValue::Muskip { idx, prev } => self.registers.set_muskip(idx, prev),
            SavedValue::Toks { idx, prev } => self.registers.set_toks(idx, prev),
            SavedValue::Catcode { byte, prev } => self.catcodes.set(byte, prev),
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
                    self.fontdimens.insert((font, num), v);
                }
                None => {
                    self.fontdimens.remove(&(font, num));
                }
            },
            SavedValue::HyphenChar { font, prev } => match prev {
                Some(v) => {
                    self.hyphenchars.insert(font, v);
                }
                None => {
                    self.hyphenchars.remove(&font);
                }
            },
            SavedValue::DelCode { byte, prev } => match prev {
                Some(v) => {
                    self.delcodes.insert(u32::from(byte), v);
                }
                None => {
                    self.delcodes.remove(&u32::from(byte));
                }
            },
            SavedValue::LcCode { byte, prev } => self.lccodes[byte as usize] = prev,
        }
    }

    /// 带作用域的宏定义：组内局部保存 + `\afterassignment` 触发。
    fn define_macro_scoped(&mut self, csid: u32, def: MacroDef) {
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
        self.eqtb.define_macro(csid, def);
        self.finish_assignment();
    }

    /// 带作用域的 eqtb 槽赋值（`\chardef`/`\countdef` 等；组内局部保存）。
    fn set_slot_scoped(&mut self, csid: u32, slot: EqSlot) {
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
        *self.eqtb.slot_mut(csid) = slot;
        self.finish_assignment();
    }

    /// 消费 `\global` 前缀（每个赋值只消费一次）。
    fn is_global(&mut self) -> bool {
        let g = self.global_pending;
        self.global_pending = false;
        g
    }

    /// 赋值完成后触发 `\afterassignment`。
    fn finish_assignment(&mut self) {
        if let Some(tok) = self.afterassignment.take() {
            self.unread(tok);
        }
    }

}
