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
            // tex.web：`\expandafter` 对第二个 token 走 get_x_token → expand，
            // 分支跳过（false 的 \if*、\else/\or 的待弃分支）是**就地**完成的，
            // 随后才把 t1 放回输入。主循环的惰性跳过会在 t1 落回时把它吞掉
            // （\e@alloc 的 `\global\ifnum…\expandafter\chardef\else…\fi`），
            // 故展开上下文里必须急切消费（见 drain_open_skip）。
            let before = self.cond_stack.len();
            self.step_conditional(op)?;
            if !matches!(op, CondOp::Fi) {
                let depth = if matches!(op, CondOp::Else | CondOp::Or) {
                    before.saturating_sub(1)
                } else {
                    before
                };
                self.drain_open_skip(depth)?;
            }
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
                        self.collect_args(csid, &def)?
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
                    // \else/\fi/\or：TeX expand() 的 fi_or_else 分支（展开为空格并推进条件机）；
                    // 开着的跳过区同样就地消费（见 exec_expandafter 的说明）
                    if let Some(op) = self.cond_op(b.0) {
                        let before = self.cond_stack.len();
                        self.step_conditional(op)?;
                        if !matches!(op, CondOp::Fi) {
                            let depth = if matches!(op, CondOp::Else | CondOp::Or) {
                                before.saturating_sub(1)
                            } else {
                                before
                            };
                            self.drain_open_skip(depth)?;
                        }
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
                EqSlot::Primitive(Primitive::Meaning) => {
                    // \meaning<token>：token 含义文本（与 exec_meaning 对齐）。
                    // 此前缺失：\meaning 在 is_expandable() 中但此处落入 `_` 分支被原样
                    // 保留，`\the\meaning\cs` 使 the_tokens_after 对 \meaning 无限递归
                    // → 输入栈溢出（fuzz 命中，畸形输入不 panic 契约违约）。
                    let t = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("\\meaning 后无 token"))?
                        .0;
                    let text = self.meaning_text(t);
                    out.extend(
                        text.bytes()
                            .map(|b| (Token::char(Catcode::Other, u32::from(b)), false)),
                    );
                }
                EqSlot::Primitive(Primitive::JobName) => {
                    // \jobname：作业名（与 exec 对齐：恒 "texput"）。
                    // 此前缺失：同 \meaning，`\the\jobname` 会无限递归。
                    out.extend(
                        "texput"
                            .bytes()
                            .map(|b| (Token::char(Catcode::Other, u32::from(b)), false)),
                    );
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
                // TRIP 补全批次：TeX 版 marks（\topmark 等，class 0 不扫描）
                EqSlot::Primitive(p @ (Primitive::TopMark
                    | Primitive::FirstMark
                    | Primitive::BotMark
                    | Primitive::SplitFirstMark
                    | Primitive::SplitBotMark)) => {
                    let text = match p {
                        Primitive::TopMark => self.sink.topmarks(0),
                        Primitive::FirstMark => self.sink.firstmarks(0),
                        Primitive::BotMark => self.sink.botmarks(0),
                        Primitive::SplitFirstMark => self.sink.splitfirstmarks(0),
                        _ => self.sink.splitbotmarks(0),
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
                // fuzz 挂死修复（2026-08-28）：以下原语在 is_expandable() 白名单中，
                // 但此前此处无分支 → 落 `_` 原样保留。扫描循环（scan_dimen_inner/
                // scan_number 的"可展开 → 展开后重试"）撞上它们时展开结果仍是自己，
                // 无限空转零消费（`\box\muexpr\romannumeral` 触发；内存随 TokenList
                // 帧无限 push 爆涨 → OOM）。与 exec_* 共用 helper，语义一致。
                EqSlot::Primitive(Primitive::RomanNumeral) => {
                    let toks = self.roman_numeral_tokens()?;
                    out.extend(toks.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::Char) => {
                    let toks = self.char_tokens()?;
                    out.extend(toks.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::Uppercase) => {
                    let toks = self.case_convert_tokens(true)?;
                    out.extend(toks.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::Lowercase) => {
                    let toks = self.case_convert_tokens(false)?;
                    out.extend(toks.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::EndInput) => {
                    // \endinput 展开语义 = 执行语义（弹 Source 帧/置 ended），无 token 输出
                    self.exec_endinput()?;
                }
                EqSlot::Primitive(Primitive::Ignorespaces) => {
                    // \ignorespaces 展开语义 = 执行语义（跳空格），无 token 输出
                    self.exec_ignorespaces()?;
                }
                EqSlot::Primitive(Primitive::FontName) => {
                    let font = self.scan_font_ident()?;
                    let name = self
                        .font_names
                        .get(font as usize)
                        .and_then(|n| n.clone())
                        .unwrap_or_default();
                    out.extend(
                        name.bytes()
                            .map(|b| (Token::char(Catcode::Other, u32::from(b)), false)),
                    );
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
            self.report_error("Arithmetic overflow.");
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
                self.report_error("Arithmetic overflow.");
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
                    self.report_error("Missing ) inserted for expression.");
                }
            }
            return Ok(v);
        }
        // eTeX 表达式分组：{1+}{2*3} 的 {1+} 组（组只是分隔；组内扫描因子后，
        // 剩余 token 放回给表达式循环——etrip L880）。scan_int 本身不处理组
        // （{ 在普通整数上下文报 Missing number，TRIP l.106）。
        if t.catcode() == Some(Catcode::BeginGroup) {
            let v = self.scan_number()?;
            let mut pending: Vec<Token> = Vec::new();
            while let Some((c, _)) = self.fetch()? {
                if c.catcode() == Some(Catcode::EndGroup) {
                    break;
                }
                pending.push(c);
            }
            for tok in pending.into_iter().rev() {
                self.unread(tok);
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
                    self.report_error("Missing ) inserted for expression.");
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
                    self.report_error("Arithmetic overflow.");
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
            self.report_error("Arithmetic overflow.");
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
                    self.report_error("Missing ) inserted for expression.");
                }
            }
            return Ok(v);
        }
        // eTeX 表达式分组：{7pt+}{12pt/4} 的 {7pt+} 组（etrip L884-888）。
        if t.catcode() == Some(Catcode::BeginGroup) {
            let v = self.scan_dimen()?;
            let mut pending: Vec<Token> = Vec::new();
            while let Some((c, _)) = self.fetch()? {
                if c.catcode() == Some(Catcode::EndGroup) {
                    break;
                }
                pending.push(c);
            }
            for tok in pending.into_iter().rev() {
                self.unread(tok);
            }
            return Ok(v);
        }
        self.unread(t);
        self.scan_dimen()
    }

    /// `\glueexpr`/`\muexpr` 胶水表达式：`<glue> (('+'|'-'|'*'|'/') <glue>)`。
    /// width 逐项求和；stretch/shrink **值求和**，其**无穷阶 = 最后一个非零分量项**
    /// 的阶（无则 NORMAL；etrip L800/L950：`\skip90+0pt` 保留 1fil、`\skip5+0pt` 清 0）。
    fn eval_glue_expression(&mut self, mu: bool) -> Result<Glue> {
        let first = self.glue_expr_mul_term(mu)?;
        let mut width = i128::from(first.width);
        let mut stretch = first.stretch;
        let mut shrink = first.shrink;
        let mut stretch_order = if first.stretch != 0 {
            first.stretch_order
        } else {
            0
        };
        let mut shrink_order = if first.shrink != 0 {
            first.shrink_order
        } else {
            0
        };
        let mut has_op = false;
        while let Some(op) = self.peek_int_op()? {
            if op != b'+' && op != b'-' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            has_op = true;
            let term = self.glue_expr_mul_term(mu)?;
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
        // 仅一项（无任何运算符）：保留 first 的完整 order（0 值分量的阶也保留，
        // etrip L949：`\glueexpr\mutoglue\muexpr\gluetomu\skip5` 的 0shrink 保留 fil）
        if !has_op {
            stretch_order = first.stretch_order;
            shrink_order = first.shrink_order;
        }
        // 仅最终宽度超限才报（中间量 i128，与 eTeX 一致）
        if width > i128::from(MAX_DIMEN) || width < -i128::from(MAX_DIMEN) {
            self.report_error("Arithmetic overflow.");
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

    /// 胶水乘法项：`<胶水项> (('*'|'/') <factor>)*`——`*/` 优先级高于 `+ -`，
    /// 作用于**本项**（`7pt+12pt/4` = 7pt + (12pt/4)，etrip L888）。
    fn glue_expr_mul_term(&mut self, mu: bool) -> Result<Glue> {
        let mut g = self.glue_expr_term(mu)?;
        while let Some(op) = self.peek_int_op()? {
            if op != b'*' && op != b'/' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            let rhs = self.expr_number_factor()?;
            let w = if op == b'*' {
                i128::from(g.width) * rhs
            } else if rhs == 0 {
                self.report_error("Arithmetic overflow.");
                0
            } else {
                expr_quotient_i128(i128::from(g.width), rhs)
            };
            g.width = w as i64;
        }
        Ok(g)
    }

    /// 胶水表达式项：`( <expr> )` 括号或 [`Self::scan_glue`]/[`Self::scan_glue_mu`]。
    /// 括号内嵌套同一单位上下文（`\muexpr(5muminus1mu)`、`\glueexpr(\muexpr...)` 由
    /// 前导量分支报 "Incompatible glue units"）。
    fn glue_expr_term(&mut self, mu: bool) -> Result<Glue> {
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\glueexpr 表达式未闭合"))?;
        let (t, _) = tok;
        if t.charcode() == Some(b'(' as u32) {
            let v = self.eval_glue_expression(mu)?;
            let close = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("\\glueexpr 括号未闭合"))?
                .0;
            if close.charcode() != Some(b')' as u32) {
                self.unread(close);
                self.report_error("Missing ) inserted for expression.");
            }
            return Ok(v);
        }
        // eTeX 表达式分组：{...} 组（组内胶水+运算符，etrip 同 dimen 因子）。
        if t.catcode() == Some(Catcode::BeginGroup) {
            let v = if mu {
                self.scan_glue_mu()?
            } else {
                self.scan_glue()?
            };
            let mut pending: Vec<Token> = Vec::new();
            while let Some((c, _)) = self.fetch()? {
                if c.catcode() == Some(Catcode::EndGroup) {
                    break;
                }
                pending.push(c);
            }
            for tok in pending.into_iter().rev() {
                self.unread(tok);
            }
            return Ok(v);
        }
        self.unread(t);
        if mu {
            self.scan_glue_mu()
        } else {
            self.scan_glue()
        }
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
        let bytes = Arc::from(bytes);
        self.stack.push(InputFrame::Source {
            line_starts: Arc::from(crate::input::line_starts(&bytes)),
            bytes,
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
        self.stack.push(InputFrame::One {
            tok,
            noexpand: false,
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
                            self.collect_args(csid, &m.value)?
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
                    // TeX `scan_csname`（tex.web L19368-19379）：不可展开控制序列 →
                    // 报 "Missing endcsname inserted"，放回该 cs，插入 `\endcsname`
                    // 结束名字扫描（可恢复，不中断引擎；TRIP L428 `\csname^^Mendcsname=\^^@`）。
                    _ => {
                        let csname = self.intern.name(csid);
                        let _ = self.sink.write16(format!(
                            "! Missing endcsname inserted.\n<to be read again>\n \\{csname}\n"
                        ));
                        self.unread(tok);
                        return Ok(name);
                    }
                }
            }
            if let Some(ch) = tok.charcode().and_then(char::from_u32) {
                name.push(ch);
            }
        }
        Ok(name)
    }

}
