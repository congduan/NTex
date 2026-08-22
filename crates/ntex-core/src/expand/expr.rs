impl Expander {
    /// `\expandafter a b`：输出 a，再输出 b 的一次展开结果。
    ///
    /// 展开"一次"：宏 → 实参替换后的宏体（不再递归展开）；`\expandafter` → 递归；
    /// `\noexpand` → 标记下一 token；其余原样。
    fn exec_expandafter(&mut self) -> Result<()> {
        let t1 = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\expandafter 后无 token"))?;
        let t2 = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\expandafter 后无第二个 token"))?;
        // 条件终结符（\else/\fi/\or）：TeX expand() 把 fi_or_else 展开为空格
        // （tex.web expand 的 fi_or_else 分支）——消耗该 token 并推进条件机，
        // 不重新输出（否则会被宏实参扫描吞掉，如 `\expandafter\2\fi`）。
        if let Some(op) = self.cond_op(t2.0) {
            self.step_conditional(op)?;
            // 前面的 token 照常输出（t1）
            let seq = vec![t1];
            self.stack.push(InputFrame::TokenList {
                items: Arc::from(seq),
                pos: 0,
            });
            return Ok(());
        }
        let mut expansion = Vec::new();
        self.expand_once(t2, &mut expansion)?;
        let mut seq = Vec::with_capacity(1 + expansion.len());
        seq.push(t1);
        seq.extend(expansion);
        self.stack.push(InputFrame::TokenList {
            items: Arc::from(seq),
            pos: 0,
        });
        Ok(())
    }

    /// 展开单个 token 一次，结果追加到 `out`。
    fn expand_once(&mut self, item: (Token, bool), out: &mut Vec<(Token, bool)>) -> Result<()> {
        if item.1 {
            // 已被 \noexpand 标记：不展开
            out.push(item);
            return Ok(());
        }
        let tok = item.0;
        if let Some(csid) = tok.csid() {
            match self.eqtb.slot(csid).clone() {
                EqSlot::Alias(target) => {
                    out.push((Token::control_sequence(target), false));
                }
                EqSlot::Macro(m) => {
                    let def = m.value.clone();
                    let args = if def.params.num_params > 0 {
                        self.collect_args(&def)?
                    } else {
                        Vec::new()
                    };
                    let materialized = materialize(&def.body, &args);
                    out.extend(materialized.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::Expandafter) => {
                    let a = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("\\expandafter 链中断"))?;
                    let b = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("\\expandafter 链中断"))?;
                    out.push(a);
                    // \else/\fi/\or：TeX expand() 的 fi_or_else 分支（展开为空格并推进条件机）
                    if let Some(op) = self.cond_op(b.0) {
                        self.step_conditional(op)?;
                    } else {
                        self.expand_once(b, out)?;
                    }
                }
                EqSlot::Primitive(Primitive::Noexpand) => {
                    let t = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("\\noexpand 后无 token"))?;
                    out.push((t.0, true));
                }
                EqSlot::Primitive(Primitive::The) => {
                    let tokens = self.the_tokens()?;
                    out.extend(tokens.into_iter().map(|t| (t, false)));
                }
                // M4-5 e-TeX/可展开原语（与 `is_expandable()` 对齐）：\number/\unexpanded/
                // \detokenize/\eTeXversion/\eTeXrevision。此前落入 `_` 分支被当作不可展开
                // 原样保留，导致 `\expandafter\1\eTeXrevision` 把未展开的 \eTeXrevision
                // 当作实参（ETRIP 版本检查 `2..6` 错误即由此而来）。
                EqSlot::Primitive(Primitive::Number) => {
                    let v = self.scan_number()?;
                    out.extend(emit_count(v).into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::ETeXVersion) => {
                    out.extend(emit_count(2).into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::ETeXRevision) => {
                    out.extend(
                        ".6"
                            .bytes()
                            .map(|b| (Token::char(Catcode::Other, u32::from(b)), false)),
                    );
                }
                EqSlot::Primitive(Primitive::Unexpanded) => {
                    // \unexpanded{...}：组内容原样保留（noexpand 标记）
                    let toks = self.scan_group_contents_expanding()?;
                    out.extend(toks.into_iter().map(|t| (t, true)));
                }
                EqSlot::Primitive(Primitive::Detokenize) => {
                    // \detokenize{...}：组内容转回字符 token（cat 12 其他字符）
                    let toks = self.scan_group_contents_expanding()?;
                    let mut detok = Vec::new();
                    for t in toks {
                        detokenize_token(t, &self.intern, &mut detok);
                    }
                    out.extend(detok.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::String_) => {
                    // \string<token>：token 转文本（字符序列）
                    let t = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("\\string 后无 token"))?
                        .0;
                    let mut buf = Vec::new();
                    detokenize_token(t, &self.intern, &mut buf);
                    out.extend(buf.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::Csname) => {
                    // \csname...\endcsname：名字扫描 → 控制序列 token（TeX expand() 语义）
                    let name = self.scan_csname()?;
                    let csid = self.intern.intern(&name);
                    out.push((Token::control_sequence(csid), false));
                }
                _ => {
                    // 未定义/不可展开原语：原样保留
                    out.push((tok, false));
                }
            }
        } else {
            out.push((tok, false));
        }
        Ok(())
    }

    // ---------- M4-5 e-TeX 展开扩展 ----------

    /// 把 token 序列压入输入流（可展开项将被展开）。
    fn emit_tokens(&mut self, tokens: Vec<Token>) -> Result<()> {
        let items: Vec<(Token, bool)> = tokens.into_iter().map(|t| (t, false)).collect();
        self.stack.push(InputFrame::TokenList {
            items: Arc::from(items),
            pos: 0,
        });
        Ok(())
    }

    /// `\numexpr` 整数表达式求值：`expr := term (('+'|'-') term)*`、
    /// `term := factor (('*'|'/') factor)*`（TeX：* / 优先，左结合，截断除法）。
    /// 以 `\relax` 或不可识别 token 结束（后者放回）。
    /// `\numexpr` 整数表达式：`<term> (('+'|'-'|'*'|'/') <term>)*`。
    fn eval_int_expression(&mut self) -> Result<i64> {
        let mut value = self.expr_mul_term()?;
        while let Some(op) = self.peek_int_op()? {
            if op != b'+' && op != b'-' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            let rhs = self.expr_mul_term()?;
            value = if op == b'+' { value + rhs } else { value - rhs };
            value = self.expr_overflow_check(value, MAX_INT);
        }
        Ok(value)
    }

    /// 乘法项：`factor (('*'|'/') factor)*`。
    fn expr_mul_term(&mut self) -> Result<i64> {
        let mut value = self.expr_factor()?;
        while let Some(op) = self.peek_int_op()? {
            if op != b'*' && op != b'/' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            let rhs = self.expr_factor()?;
            value = if op == b'*' {
                value * rhs
            } else if rhs == 0 {
                // eTeX：除零 → "! Arithmetic overflow."，结果 0（etrip L789-791）
                let _ = self.sink.write16("! Arithmetic overflow.\n".to_string());
                0
            } else {
                self.expr_quotient(value, rhs)
            };
            value = self.expr_overflow_check(value, MAX_INT);
        }
        Ok(value)
    }

    /// 表达式运算溢出检查：|v| 超限则报 "! Arithmetic overflow." 并归 0（eTeX 语义，
    /// 结果 0 由 `{reassigning \count44=0}` 等可见）。`limit` 为正限值。
    fn expr_overflow_check(&mut self, v: i64, limit: i64) -> i64 {
        if v > limit || v < -limit {
            let _ = self.sink.write16("! Arithmetic overflow.\n".to_string());
            0
        } else {
            v
        }
    }

    /// e-TeX 表达式除法：**四舍五入**到最近整数（ties away from zero），
    /// 非 TeX 传统截断（etrip "Expr quotient rounding" 段：`"40000000/"7FFFFFFF`=1）。
    /// 调用方保证 `d != 0`。
    fn expr_quotient(&self, n: i64, d: i64) -> i64 {
        let an = n.unsigned_abs();
        let ad = d.unsigned_abs();
        let q = (an + ad / 2) / ad;
        let q = q as i64;
        if (n < 0) != (d < 0) {
            -q
        } else {
            q
        }
    }

    /// 整数因子：`(` <表达式> `)`（TeX 括号子表达式）或 [`Self::scan_number`]。
    fn expr_factor(&mut self) -> Result<i64> {
        self.skip_spaces()?;
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\numexpr 表达式未闭合"))?;
        let (t, _) = tok;
        if t.charcode() == Some(b'(' as u32) {
            let v = self.eval_int_expression()?;
            let close = self.fetch()?;
            match close {
                Some((c, _)) if c.charcode() == Some(b')' as u32) => {}
                _ => {
                    // TeX 恢复：报 "Missing ) inserted" 并插入 `)` 继续
                    if let Some((c, _)) = close {
                        self.unread(c);
                    }
                    let _ = self
                        .sink
                        .write16("! Missing ) inserted for expression.\n".to_string());
                }
            }
            return Ok(v);
        }
        self.unread(t);
        self.scan_number()
    }

    /// 取下一个整数运算符（`+ - * /`）或 `\relax`（结束符，吸收）；其余 token 放回。
    fn peek_int_op(&mut self) -> Result<Option<u8>> {
        let Some((tok, _)) = self.fetch()? else { return Ok(None) };
        if tok.csid().is_some_and(|id| self.intern.name(id) == "relax") {
            return Ok(None); // \relax 吸收
        }
        if tok.catcode() == Some(Catcode::Other) {
            if let Some(ch) = tok.charcode() {
                if matches!(ch, 0x2B | 0x2D | 0x2A | 0x2F) {
                    // + - * /
                    return Ok(Some(ch as u8));
                }
            }
        }
        self.unread(tok);
        Ok(None)
    }

    /// `\dimexpr` 尺寸表达式：`<dimen> (('+'|'-') <dimen>)*`（e-TeX 文法子集：
    /// 每项为 [`Self::scan_dimen`] 可识别的尺寸；支持 `( <expr> )` 括号；
    /// `\relax` 或不可识别 token 结束，后者放回）。
    fn eval_dimen_expression(&mut self) -> Result<i64> {
        let mut value = self.dimen_expr_term()?;
        while let Some(op) = self.peek_int_op()? {
            if op != b'+' && op != b'-' && op != b'*' && op != b'/' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            if op == b'*' || op == b'/' {
                // e-TeX：`<dimen> * <number>` 与 `<dimen> / <number>`（etrip L780/785）
                let rhs = self.scan_number()?;
                value = if op == b'*' {
                    value * rhs
                } else if rhs == 0 {
                    let _ = self.sink.write16("! Arithmetic overflow.\n".to_string());
                    0
                } else {
                    self.expr_quotient(value, rhs)
                };
            } else {
                let rhs = self.dimen_expr_term()?;
                value = if op == b'+' { value + rhs } else { value - rhs };
            }
            value = self.expr_overflow_check(value, MAX_DIMEN);
        }
        Ok(value)
    }

    /// 尺寸表达式项：`( <expr> )` 括号或 [`Self::scan_dimen`]。
    fn dimen_expr_term(&mut self) -> Result<i64> {
        self.skip_spaces()?;
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\dimexpr 表达式未闭合"))?;
        let (t, _) = tok;
        if t.charcode() == Some(b'(' as u32) {
            let v = self.eval_dimen_expression()?;
            let close = self.fetch()?;
            match close {
                Some((c, _)) if c.charcode() == Some(b')' as u32) => {}
                _ => {
                    if let Some((c, _)) = close {
                        self.unread(c);
                    }
                    let _ = self
                        .sink
                        .write16("! Missing ) inserted for expression.\n".to_string());
                }
            }
            return Ok(v);
        }
        self.unread(t);
        self.scan_dimen()
    }

    /// `\glueexpr` 胶水表达式：`<glue> (('+'|'-'|'*'|'/') <glue>)`。
    /// width 逐项求和；stretch/shrink 取**最后一个**非零项（含符号，eTeX 语义）。
    /// `*`/`/` 为 `<glue width> * <number>` 标量运算（etrip L872-873）。
    /// 支持 `( <expr> )` 括号（`\muexpr(5muminus1mu)`）。
    fn eval_glue_expression(&mut self) -> Result<Glue> {
        let mut result = self.glue_expr_term()?;
        while let Some(op) = self.peek_int_op()? {
            if op != b'+' && op != b'-' && op != b'*' && op != b'/' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            if op == b'*' || op == b'/' {
                let rhs = self.scan_number()?;
                if op == b'*' {
                    result.width = self.expr_overflow_check(result.width * rhs, MAX_DIMEN);
                } else if rhs == 0 {
                    let _ = self.sink.write16("! Arithmetic overflow.\n".to_string());
                    result.width = 0;
                } else {
                    result.width =
                        self.expr_overflow_check(self.expr_quotient(result.width, rhs), MAX_DIMEN);
                }
                continue;
            }
            let term = self.glue_expr_term()?;
            if op == b'+' {
                result.width += term.width;
                if term.stretch != 0 {
                    result.stretch = term.stretch;
                }
                if term.shrink != 0 {
                    result.shrink = term.shrink;
                }
            } else {
                result.width -= term.width;
                if term.stretch != 0 {
                    result.stretch = -term.stretch;
                }
                if term.shrink != 0 {
                    result.shrink = -term.shrink;
                }
            }
        }
        Ok(result)
    }

    /// 胶水表达式项：`( <expr> )` 括号或 [`Self::scan_glue`]。
    fn glue_expr_term(&mut self) -> Result<Glue> {
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\glueexpr 表达式未闭合"))?;
        let (t, _) = tok;
        if t.charcode() == Some(b'(' as u32) {
            let v = self.eval_glue_expression()?;
            let close = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("\\glueexpr 括号未闭合"))?
                .0;
            if close.charcode() != Some(b')' as u32) {
                self.unread(close);
                let _ = self
                    .sink
                    .write16("! Missing ) inserted for expression.\n".to_string());
            }
            return Ok(v);
        }
        self.unread(t);
        self.scan_glue()
    }

    /// `\detokenize{...}`：组内容转字符 token 流（字符 catcode 12、空格 10、
    /// 控制序列 → `\名字` 文本），作为输入继续处理。
    fn exec_detokenize(&mut self) -> Result<()> {
        let toks = self.scan_group_contents_expanding()?;
        let mut out = Vec::new();
        for t in toks {
            detokenize_token(t, &self.intern, &mut out);
        }
        self.emit_tokens(out)
    }

    /// `\unexpanded{...}`：组内容作为 token 流输出。
    /// - 展开上下文（`\edef`/`\write`）：标记 noexpand，内容不再展开（e-TeX 语义）；
    /// - 主循环执行：正常执行（`\unexpanded{\def\1{...}}` 中 `\def` 生效）。
    fn exec_unexpanded(&mut self) -> Result<()> {
        let toks = self.scan_group_contents_expanding()?;
        let flag = self.expand_only;
        let items: Vec<(Token, bool)> = toks.into_iter().map(|t| (t, flag)).collect();
        self.stack.push(InputFrame::TokenList {
            items: Arc::from(items),
            pos: 0,
        });
        Ok(())
    }

    /// `\scantokens{...}`（M4-5 e-TeX）：组内容 detokenize 为文本后按**当前**
    /// catcode 重新扫描（eTeX 语义：等价于从字符串 `\input`）。
    /// 参数为 `<general text>`：先展开可展开项（`\scantokens\expandafter{\1}`）。
    fn exec_scantokens(&mut self) -> Result<()> {
        let toks = self.scan_group_contents_expanding()?;
        let mut text: Vec<Token> = Vec::new();
        for t in toks {
            detokenize_token(t, &self.intern, &mut text);
        }
        let bytes: Vec<u8> = text
            .iter()
            .filter_map(|t| t.charcode().and_then(|c| u8::try_from(c).ok()))
            .collect();
        self.stack.push(InputFrame::Source {
            bytes: Arc::from(bytes),
            pos: 0,
        });
        Ok(())
    }

    /// `\csname<name>\endcsname`：扫描名字，构造控制序列 token 并放回输入流
    /// （TeX expand() 语义：结果是可执行 token，主循环继续处理）。
    fn exec_csname(&mut self) -> Result<()> {
        let name = self.scan_csname()?;
        let csid = self.intern.intern(&name);
        let tok = Token::control_sequence(csid);
        self.stack.push(InputFrame::TokenList {
            items: Arc::from([(tok, false)]),
            pos: 0,
        });
        Ok(())
    }

    /// `\csname` 名字扫描（`\csname`/`\ifcsname` 用）：收集直到 `\endcsname` 的名字字符。
    /// get_x_token 语义：宏/可展开原语在名字中先展开一次；`\endcsname` 终止；
    /// 其余不可展开控制序列报错（TeX "Missing endcsname inserted"）。
    fn scan_csname(&mut self) -> Result<String> {
        let mut name = String::new();
        loop {
            let tok = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("\\csname 未闭合（缺少 \\endcsname）"))?
                .0;
            if let Some(csid) = tok.csid() {
                if self.intern.name(csid) == "endcsname" {
                    break;
                }
                match self.eqtb.slot(csid).clone() {
                    EqSlot::Macro(m) => {
                        let args = if m.value.params.num_params > 0 {
                            self.collect_args(&m.value)?
                        } else {
                            Vec::new()
                        };
                        let body = materialize(&m.value.body, &args);
                        let seq: Vec<(Token, bool)> =
                            body.into_iter().map(|t| (t, false)).collect();
                        self.stack.push(InputFrame::TokenList {
                            items: Arc::from(seq),
                            pos: 0,
                        });
                        continue;
                    }
                    EqSlot::Primitive(p) if p.is_expandable() => {
                        let mut out = Vec::new();
                        self.expand_once((tok, false), &mut out)?;
                        let seq: Vec<(Token, bool)> = out;
                        self.stack.push(InputFrame::TokenList {
                            items: Arc::from(seq),
                            pos: 0,
                        });
                        continue;
                    }
                    _ => return Err(Error::invalid_input("\\csname 名字中含控制序列")),
                }
            }
            if let Some(ch) = tok.charcode().and_then(char::from_u32) {
                name.push(ch);
            }
        }
        Ok(name)
    }

}
