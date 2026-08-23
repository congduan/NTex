/// e-TeX 表达式除法：**四舍五入**到最近整数（ties away from zero），非 TeX 传统截断
/// （etrip "Expr quotient rounding"：`"40000000/"7FFFFFFF`=1）。调用方保证 `d != 0`。
fn expr_quotient_i128(n: i128, d: i128) -> i128 {
    let (an, ad) = (n.abs(), d.abs());
    let q = (an + ad / 2) / ad;
    if (n < 0) != (d < 0) {
        -q
    } else {
        q
    }
}

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
                // ETRIP 冲刺：e-TeX marks 族查询（可展开，返回字符 token 文本）
                // \topmarks<n> / \firstmarks<n> / \botmarks<n> / \splitfirstmarks<n> /
                // \splittopmarks<n> / \splitbotmarks<n>：扫描 class 号，向 sink 查询，
                // 返回内容转为字符 token（空内容输出空）。
                EqSlot::Primitive(p @ (Primitive::TopMarks
                    | Primitive::FirstMarks
                    | Primitive::BotMarks
                    | Primitive::SplitFirstMarks
                    | Primitive::SplitTopMarks
                    | Primitive::SplitBotMarks)) => {
                    let class = self.scan_number()?;
                    let text = match p {
                        Primitive::TopMarks => self.sink.topmarks(class),
                        Primitive::FirstMarks => self.sink.firstmarks(class),
                        Primitive::BotMarks => self.sink.botmarks(class),
                        Primitive::SplitFirstMarks => self.sink.splitfirstmarks(class),
                        Primitive::SplitTopMarks => self.sink.splittopmarks(class),
                        Primitive::SplitBotMarks => self.sink.splitbotmarks(class),
                        _ => unreachable!("marks 族已在上层 match 穷举"),
                    };
                    out.extend(text.bytes().map(|b| {
                        let cat = if b == b' ' {
                            Catcode::Space
                        } else {
                            Catcode::Other
                        };
                        (Token::char(cat, u32::from(b)), false)
                    }));
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

    /// `\numexpr` 整数表达式：`<term> (('+'|'-'|'*'|'/') <term>)*`。
    ///
    /// eTeX 语义：中间量用 i128（`mult_and_add` 64 位中间等价），**仅最终结果**
    /// 超出 ±0x7FFFFFFF 才报 "! Arithmetic overflow." 并取 0（etrip "Expr fraction
    /// rounding"：`"7FFFFFFE*"7FFFFFFE/"7FFFFFFD` 的中间乘积 2^62 不得误判溢出）。
    fn eval_int_expression(&mut self) -> Result<i64> {
        let mut value = self.expr_mul_term()?;
        while let Some(op) = self.peek_int_op()? {
            if op != b'+' && op != b'-' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            let rhs = self.expr_mul_term()?;
            value = if op == b'+' { value + rhs } else { value - rhs };
        }
        if value > i128::from(MAX_INT) || value < -i128::from(MAX_INT) {
            let _ = self.sink.write16("! Arithmetic overflow.\n".to_string());
            return Ok(0);
        }
        Ok(value as i64)
    }

    /// 乘法项：`factor (('*'|'/') factor)*`（i128 中间量，见 [`Self::eval_int_expression`]）。
    fn expr_mul_term(&mut self) -> Result<i128> {
        let mut value = i128::from(self.expr_factor()?);
        while let Some(op) = self.peek_int_op()? {
            if op != b'*' && op != b'/' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            let rhs = i128::from(self.expr_factor()?);
            value = if op == b'*' {
                value * rhs
            } else if rhs == 0 {
                // eTeX：除零 → "! Arithmetic overflow."，结果 0（etrip L789-791）
                let _ = self.sink.write16("! Arithmetic overflow.\n".to_string());
                0
            } else {
                expr_quotient_i128(value, rhs)
            };
        }
        Ok(value)
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
        // \relax 终止表达式：`\relax` 原语、`\let\9=\relax` 别名，或
        // `\def\9{\relax}` 宏（etrip 大量用 `\9` 收尾——942 行是宏定义而非 \let）。
        if let Some(id) = tok.csid() {
            let mut cur = id;
            let mut depth = 0;
            let mut is_relax = false;
            loop {
                match self.eqtb.slot(cur) {
                    EqSlot::Alias(t) => {
                        cur = *t;
                        depth += 1;
                        if depth > 100 {
                            break;
                        }
                    }
                    EqSlot::Macro(m) => {
                        // 宏体为单个 `\relax`（如 `\def\9{\relax}`）→ 等价终止符
                        is_relax = m.value.body.len() == 1
                            && self.is_relax_token(&m.value.body[0]);
                        break;
                    }
                    EqSlot::Primitive(Primitive::Relax) => {
                        is_relax = true;
                        break;
                    }
                    _ => break,
                }
            }
            if is_relax {
                return Ok(None); // \relax 吸收
            }
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

    /// token 是否等价于 `\relax`（原语或经 `\let` 别名链指向 `\relax`）。
    fn is_relax_token(&self, tok: &Token) -> bool {
        let Some(id) = tok.csid() else {
            return false;
        };
        let mut cur = id;
        let mut depth = 0;
        loop {
            match self.eqtb.slot(cur) {
                EqSlot::Alias(t) => {
                    cur = *t;
                    depth += 1;
                    if depth > 100 {
                        return false;
                    }
                }
                EqSlot::Primitive(Primitive::Relax) => return true,
                _ => return false,
            }
        }
    }

    /// dimen/glue 表达式 `*`/`/` 的 number 因子：`( <int expr> )` 或 [`Self::scan_number`]
    /// （etrip L854：`\dimexpr(#3sp)*(#4)/(#5)` 的括号乘数）。
    fn expr_number_factor(&mut self) -> Result<i128> {
        self.skip_spaces()?;
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("表达式缺少数字因子"))?;
        let (t, _) = tok;
        if t.charcode() == Some(b'(' as u32) {
            let v = self.eval_int_expression()?;
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
            return Ok(i128::from(v));
        }
        self.unread(t);
        Ok(i128::from(self.scan_number()?))
    }

    /// `\dimexpr` 尺寸表达式：`<dimen> (('+'|'-') <dimen>)*`（e-TeX 文法子集：
    /// 每项为 [`Self::scan_dimen`] 可识别的尺寸；支持 `( <expr> )` 括号；
    /// `\relax` 或不可识别 token 结束，后者放回）。
    fn eval_dimen_expression(&mut self) -> Result<i64> {
        let mut value = i128::from(self.dimen_expr_term()?);
        while let Some(op) = self.peek_int_op()? {
            if op != b'+' && op != b'-' && op != b'*' && op != b'/' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            if op == b'*' || op == b'/' {
                // e-TeX：`<dimen> * <number>` 与 `<dimen> / <number>`（etrip L780/785）
                let rhs = self.expr_number_factor()?;
                value = if op == b'*' {
                    value * rhs
                } else if rhs == 0 {
                    let _ = self.sink.write16("! Arithmetic overflow.\n".to_string());
                    0
                } else {
                    expr_quotient_i128(value, rhs)
                };
            } else {
                let rhs = i128::from(self.dimen_expr_term()?);
                value = if op == b'+' { value + rhs } else { value - rhs };
            }
        }
        // 仅最终结果超限才报（中间量 i128 不逐项检查，与 eTeX 一致）
        if value > i128::from(MAX_DIMEN) || value < -i128::from(MAX_DIMEN) {
            let _ = self.sink.write16("! Arithmetic overflow.\n".to_string());
            return Ok(0);
        }
        Ok(value as i64)
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
    /// width 逐项求和；stretch/shrink **值求和**，其**无穷阶 = 最后一个非零分量项**
    /// 的阶（无则 NORMAL；etrip L800/L950：`\skip90+0pt` 保留 1fil、`\skip5+0pt` 清 0）。
    /// `*`/`/` 为 `<glue width> * <number>` 标量运算（etrip L872-873）。
    /// 支持 `( <expr> )` 括号（`\muexpr(5muminus1mu)`）。
    fn eval_glue_expression(&mut self) -> Result<Glue> {
        let first = self.glue_expr_term()?;
        let mut width = i128::from(first.width);
        let mut stretch = first.stretch;
        let mut shrink = first.shrink;
        let mut stretch_order = if first.stretch != 0 { first.stretch_order } else { 0 };
        let mut shrink_order = if first.shrink != 0 { first.shrink_order } else { 0 };
        while let Some(op) = self.peek_int_op()? {
            if op != b'+' && op != b'-' && op != b'*' && op != b'/' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            if op == b'*' || op == b'/' {
                let rhs = self.expr_number_factor()?;
                width = if op == b'*' {
                    width * rhs
                } else if rhs == 0 {
                    let _ = self.sink.write16("! Arithmetic overflow.\n".to_string());
                    0
                } else {
                    expr_quotient_i128(width, rhs)
                };
                continue;
            }
            let term = self.glue_expr_term()?;
            if op == b'+' {
                width += i128::from(term.width);
                stretch += term.stretch;
                shrink += term.shrink;
            } else {
                width -= i128::from(term.width);
                stretch -= term.stretch;
                shrink -= term.shrink;
            }
            if term.stretch != 0 {
                stretch_order = term.stretch_order;
            }
            if term.shrink != 0 {
                shrink_order = term.shrink_order;
            }
        }
        // 仅最终宽度超限才报（中间量 i128，与 eTeX 一致）
        if width > i128::from(MAX_DIMEN) || width < -i128::from(MAX_DIMEN) {
            let _ = self.sink.write16("! Arithmetic overflow.\n".to_string());
            width = 0;
        }
        Ok(Glue {
            width: width as i64,
            stretch,
            shrink,
            stretch_order,
            shrink_order,
        })
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
            state: ScanState::LineStart,
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
