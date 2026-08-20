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
                    self.expand_once(b, out)?;
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
                    let toks = self.scan_group_contents()?;
                    out.extend(toks.into_iter().map(|t| (t, true)));
                }
                EqSlot::Primitive(Primitive::Detokenize) => {
                    // \detokenize{...}：组内容转回字符 token（cat 12 其他字符）
                    let toks = self.scan_group_contents()?;
                    let mut detok = Vec::new();
                    for t in toks {
                        detokenize_token(t, &self.intern, &mut detok);
                    }
                    out.extend(detok.into_iter().map(|t| (t, false)));
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
    fn eval_int_expression(&mut self) -> Result<i64> {
        let mut value = self.expr_mul_term()?;
        loop {
            let Some(op) = self.peek_int_op()? else { break };
            if op != b'+' && op != b'-' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            let rhs = self.expr_mul_term()?;
            value = if op == b'+' { value + rhs } else { value - rhs };
        }
        Ok(value)
    }

    /// 乘法项：`factor (('*'|'/') factor)*`。
    fn expr_mul_term(&mut self) -> Result<i64> {
        let mut value = self.scan_number()?;
        loop {
            let Some(op) = self.peek_int_op()? else { break };
            if op != b'*' && op != b'/' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            let rhs = self.scan_number()?;
            value = if op == b'*' { value * rhs } else { value / rhs };
        }
        Ok(value)
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
    /// 每项为 [`Self::scan_dimen`] 可识别的尺寸；`\relax` 或不可识别 token 结束，后者放回）。
    fn eval_dimen_expression(&mut self) -> Result<i64> {
        let mut value = self.scan_dimen()?;
        loop {
            let Some(op) = self.peek_int_op()? else { break };
            if op != b'+' && op != b'-' {
                // * / 不是尺寸运算符：放回结束
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            let rhs = self.scan_dimen()?;
            value = if op == b'+' { value + rhs } else { value - rhs };
        }
        Ok(value)
    }

    /// `\glueexpr` 胶水表达式：`<glue> (('+'|'-') <glue>)*`。
    /// width 逐项求和；stretch/shrink 取**最后一个**非零项（含符号，eTeX 语义）。
    fn eval_glue_expression(&mut self) -> Result<Glue> {
        let mut result = self.scan_glue()?;
        loop {
            let Some(op) = self.peek_int_op()? else { break };
            if op != b'+' && op != b'-' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            let term = self.scan_glue()?;
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

    /// `\detokenize{...}`：组内容转字符 token 流（字符 catcode 12、空格 10、
    /// 控制序列 → `\名字` 文本），作为输入继续处理。
    fn exec_detokenize(&mut self) -> Result<()> {
        let toks = self.scan_group_contents()?;
        let mut out = Vec::new();
        for t in toks {
            detokenize_token(t, &self.intern, &mut out);
        }
        self.emit_tokens(out)
    }

    /// `\unexpanded{...}`：组内容作为 noexpand token 流输出（不展开、保留 catcode）。
    fn exec_unexpanded(&mut self) -> Result<()> {
        let toks = self.scan_group_contents()?;
        let items: Vec<(Token, bool)> = toks.into_iter().map(|t| (t, true)).collect();
        self.stack.push(InputFrame::TokenList {
            items: Arc::from(items),
            pos: 0,
        });
        Ok(())
    }

    /// `\scantokens{...}`（M4-5 e-TeX）：组内容 detokenize 为文本后按**当前**
    /// catcode 重新扫描（eTeX 语义：等价于从字符串 `\input`）。
    fn exec_scantokens(&mut self) -> Result<()> {
        let toks = self.scan_group_contents()?;
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

    /// `\csname` 名字扫描（`\ifcsname` 用）：收集直到 `\endcsname` 的名字字符。
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
                return Err(Error::invalid_input("\\csname 名字中含控制序列"));
            }
            if let Some(ch) = tok.charcode().and_then(char::from_u32) {
                name.push(ch);
            }
        }
        Ok(name)
    }

}
