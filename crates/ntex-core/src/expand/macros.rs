impl Expander {
    // ---------- 宏调用与实参 ----------

    /// 收集宏的全部实参（M1-8 分隔参数）。
    ///
    /// 按 TeX scan_args 语义处理参数文本 `P_1 #1 P_2 #2 ... P_n P_{n+1}`：
    /// - 先匹配前导定界符 P_1（须与输入开头逐 token 相同）；
    /// - 对每个 `#k`：若其后定界符 P_{k+1} 为空 → 无分隔参数（单个 token 或组）；
    ///   否则为分隔参数，收集到 P_{k+1} 在输入中完整出现为止（定界符被消费）。
    fn collect_args(&mut self, csid: u32, def: &MacroDef) -> Result<Vec<TokenArray>> {
        let n = def.params.num_params as usize;
        if n == 0 {
            return Ok(Vec::new());
        }
        let name = self.intern.name(csid).to_owned();
        // 按 #n 参数 token 分段：segments[0]=P_1（#1 前），segments[k]=P_{k+1}（#k 后）
        let mut segments: Vec<Vec<Token>> = vec![Vec::new()];
        for t in def.params.text.iter() {
            if t.param_number().is_some() {
                segments.push(Vec::new());
            } else {
                segments
                    .last_mut()
                    .expect("segments 非空")
                    .push(*t);
            }
        }
        debug_assert_eq!(segments.len(), n + 1, "参数文本分段应与参数个数一致");

        let mut args = Vec::with_capacity(n);
        // 前导定界符 P_1
        if self.match_input_delim(&segments[0]).is_err() {
            // TeX：宏调用与定义不匹配 → "Use of \X doesn't match its definition."
            // 报错恢复（忽略该宏调用，按无参展开；TRIP L332 `\t2` 等）
            let _ = self.sink.write16(
                "! Use of macro doesn't match its definition.\n\
                 The macro here has not been followed by the required stuff,\n\
                 so I'm ignoring it.\n"
                    .to_string(),
            );
            return Ok(Vec::new());
        }
        for k in 0..n {
            let delim = &segments[k + 1]; // P_{k+2}：紧跟在 #(k+1) 后的定界符
            let arg = if delim.is_empty() {
                self.collect_undelimited_arg(def.params.long, &name)?
            } else {
                self.collect_delimited_arg(delim, def.params.long, &name)?
            };
            args.push(arg);
        }
        Ok(args)
    }

    /// TeX "Paragraph ended" 恢复辅助：丢弃当前行中 `\par` 之后的 token，
    /// 直到行尾（EOL）或下一个 `\par`（放回保留，供后续分隔符/主循环使用）。
    fn skip_to_line_end_after_par(&mut self) -> Result<()> {
        loop {
            let Some((tok, _)) = self.fetch()? else { return Ok(()) };
            if self.is_par_token(tok) {
                self.unread(tok);
                return Ok(());
            }
            if tok.catcode() == Some(Catcode::EndOfLine) {
                return Ok(());
            }
        }
    }

    /// TeX：non-long 宏参数扫描中遇 `\par` → "Paragraph ended before \<name>
    /// was complete." 恢复：报错 + 跳过本行剩余 + `\par` 放回（TRIP L357）。
    fn recover_par_in_argument(&mut self, name: &str, tok: Token) -> Result<()> {
        let _ = self.sink.write16(format!(
            "! Paragraph ended before \\{name} was complete.\n\
             <to be read again>\n                   \\par\n"
        ));
        // TeX error() 的上下文行：`l.2 \a\par`（真实 TeX 同格式）
        self.report_error_context();
        self.skip_to_line_end_after_par()?;
        self.unread(tok);
        Ok(())
    }

    /// 逐 token 匹配输入与定界符序列（用于前导定界符 P_1）。
    /// 失配 → 报 "宏调用与定义不匹配"。
    fn match_input_delim(&mut self, delim: &[Token]) -> Result<()> {
        for want in delim {
            let tok = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("宏参数定界符匹配到输入末尾"))?
                .0;
            if !self.delim_token_eq(tok, *want) {
                return Err(Error::invalid_input(
                    "宏调用与定义不匹配（参数定界符不一致）",
                ));
            }
        }
        Ok(())
    }

    /// 定界符 token 等价比较：字符按 (catcode, char)；控制序列按 **token 同一**
    /// （cs 名，tex.web macro_call `cur_tok=info(r)` 的 token 相等）。**不按含义**
    /// ——expl3 的定界/quark token 常留未定义（`\s__prg_stop`/`\q__prg_recursion_
    /// tail` 全篇无定义），而尾参数据里同是未定义的 `\tl_if_empty:nF`（l3tl 未
    /// 载入）若按含义比较会**误作定界符**提前终止 → 尾参泄出被就地执行。真实 TeX
    /// 两个不同名的未定义 cs 是不同 token，不定界。
    fn delim_token_eq(&self, a: Token, b: Token) -> bool {
        match (a.kind(), b.kind()) {
            (TokenKind::Char, TokenKind::Char) => a == b,
            (TokenKind::ControlSeq, TokenKind::ControlSeq) => a.csid() == b.csid(),
            _ => false,
        }
    }

    /// 收集一个分隔实参：读入 token 直到定界符序列在输入中完整匹配（后缀匹配）。
    fn collect_delimited_arg(
        &mut self,
        delim: &[Token],
        long: bool,
        name: &str,
    ) -> Result<TokenArray> {
        if diag_enabled("NTEX_COND_TRACE") {
            let d: String = delim
                .iter()
                .filter_map(|t| t.charcode())
                .filter_map(char::from_u32)
                .collect();
            eprintln!("[trace-arg] 定界符 {:?} n={}", d, delim.len());
        }
        let mut buf: Vec<Token> = Vec::new();
        let mut depth = 0usize; // 平衡组深度：`{…}` 组整组贡献，组内 token 不定界
        loop {
            let tok = match self.fetch()? {
                Some(t) => t.0,
                None => {
                    // TeX：定界参数扫描到输入末尾 → "Runaway argument?" +
                    // "! Paragraph ended before \<name> was complete." 恢复
                    // （trip.log L6557-6560）：`\par` 插入输入流终止参数，
                    // 返回已收集内容（可恢复，不中断；TRIP L431 `\l}`）。
                    let _ = self.sink.write16(format!(
                        "Runaway argument?\n\
                         ! Paragraph ended before \\{name} was complete.\n\
                         <to be read again>\n                   \\par\n"
                    ));
                    let par = Token::control_sequence(self.intern.intern("par"));
                    self.unread(par);
                    return Ok(Arc::from(buf));
                }
            };
            // 实参位置的条件 token（`\if*`/`\else`/`\fi`/`\or`）一律是**数据**
            //（见 collect_undelimited_arg 的归属说明）。
            //
            // TeX：分隔实参内的 outer 宏 → forbidden
            self.check_not_outer(tok)?;
            match tok.catcode() {
                Some(Catcode::BeginGroup) => {
                    // tex.web macro_call "Contribute an entire group"：`{` 起的
                    // 平衡组整体作为实参数据贡献。组内 token 只做配对，不匹配
                    // 定界符（expl3 w 尾参 `\tl_if_empty:nF {#8} {…}` 的组内容
                    // 是数据，其内部定界符不终止参数——l3prg 条件生成器阻塞）。
                    depth += 1;
                    buf.push(tok);
                    continue;
                }
                Some(Catcode::EndGroup) if depth > 0 => {
                    depth -= 1;
                    buf.push(tok);
                    continue;
                }
                _ => {}
            }
            // 至此 depth == 0 的 token（组内非定界符 token 也落此，但 depth>0，
            // 下面各检查对组内 token 只看 non-long `\par`——tex.web 整组贡献
            // 的循环同样禁止 non-long 参数内任意深度 `\par`）。
            buf.push(tok);
            // 分隔符匹配优先：`\par` 作为定界符时合法（TRIP L354 `\a#1\par#2` 调
            // `\a\par!` → `#1` 空、`#2`=`!`；non-long 参数扫描的 Paragraph ended
            // 检查须在分隔符匹配之后，否则定界符 `\par` 被误报）。只在 depth==0
            // 匹配——组内的同形 token 是数据。
            if depth == 0 && self.suffix_matches_delim(&buf, delim) {
                buf.truncate(buf.len() - delim.len());
                break;
            }
            // TeX scan_macro_arg：定界参数扫描遇 `}`（end_group，非定界符）→
            // "! Argument of \X has an extra }." 恢复（trip.log L6541）：long 宏
            // 参数补 `\par` 终止并放回 `}`；non-long 宏同 "Paragraph ended"。
            // depth>0 的 `}` 已在上方配对，到这里的只可能是 depth==0 的额外 `}`。
            if tok.catcode() == Some(Catcode::EndGroup) {
                buf.pop();
                let _ = self.sink.write16(format!(
                    "! Argument of \\{name} has an extra }}.\n\
                     I've run across a `}}' that doesn't seem to match anything.\n\
                     For example, `\\def\\a#1{{...}}' and `\\a}}' would produce\n\
                     this error. If you simply proceed now, the `\\par' that\n\
                     I've just inserted will cause me to report a runaway\n\
                     argument that might be the root of the problem. But if\n\
                     your `}}' was spurious, just type `2' and it will go away.\n"
                ));
                self.unread(tok);
                if long {
                    buf.push(Token::control_sequence(self.intern.intern("par")));
                    return Ok(Arc::from(buf));
                }
                self.recover_par_in_argument(name, tok)?;
                return Ok(Arc::from(buf));
            }
            // non-long 参数中 `\par`（非定界符位置）→ "Paragraph ended"（含组内）
            if !long && self.is_par_token(tok) {
                buf.pop();
                self.recover_par_in_argument(name, tok)?;
                return Ok(Arc::from(buf));
            }
        }
        Ok(Arc::from(buf))
    }

    /// 检查 `buf` 尾部是否与定界符逐 token 相同。
    fn suffix_matches_delim(&self, buf: &[Token], delim: &[Token]) -> bool {
        if buf.len() < delim.len() {
            return false;
        }
        let start = buf.len() - delim.len();
        buf[start..]
            .iter()
            .zip(delim)
            .all(|(&a, &b)| self.delim_token_eq(a, b))
    }

    /// 收集一个无分隔实参：
    /// 跳过前导空格；`{...}` 取组内容（去外层花括号），否则取单个 token。
    fn collect_undelimited_arg(&mut self, long: bool, name: &str) -> Result<TokenArray> {
        // 跳过前导空格
        loop {
            let tok = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("实参扫描到输入末尾"))?
                .0;
            if tok.catcode() != Some(Catcode::Space) {
                self.unread(tok);
                break;
            }
        }
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("实参扫描到输入末尾"))?
            .0;
        // 归属判定（LaTeX 兼容第十一刀）：实参位置的条件终结符
        // `\else`/`\fi`/`\or` **一律是数据**，不交条件机。tex.web 的宏实参扫描
        // （scan_toks(macro=true)，383-389）取 token 用的是 `get_token`——它只做
        // get_next + 词法包装，**既不展开、也不推进条件机**；`\else`/`\fi` 在
        // tex.web 里被条件机消费的唯一位点是 `expand`（get_x_token）的
        // fi_or_else 分支（@<Terminate the current conditional…@>），而实参扫描
        // 不属展开位置。故"它闭合的是不是扫描前已开的帧"这个问题在真实 TeX 里
        // 答案恒为否——该帧的 `\else`/`\fi` 在输入流的更后面，等宏展开完由主循环
        // （tex.web main_control → get_next）消费。explore3 的别名表
        // `\__kernel_primitive:NN \else \tex_else:D` 即依赖此语义：`\else` 作为
        // `#1` 数据被 `\tex_let:D #2 #1` 消费，建立原语别名。
        //
        // 旧实现在此 step_conditional，产生两类偏差：① 外层无帧时误报
        // `! Extra \else.`/`! Extra \fi.`；② 有帧时翻转/弹出外层帧并把该 token
        // 丢掉，实参错位一格（#1 吃到 `#2` 位置的内容，expl3 别名表连锁错位）。
        // 旧注释引的 `\expandafter\2\fi` 惯用法实际不经实参扫描——`\expandafter`
        // 对第二 token 走 expand（expr.rs 的 fi_or_else 臂：step_conditional +
        // drain_open_skip），`\fi` 在 `\2` 的实参扫描开始前就已被消费。
        //
        // 不变量（惰性跳过模型）：实参扫描不会在条件跳过区里运行——
        // `process_one` 对跳过区的非条件 token 直接丢弃（不派发宏调用），
        // skip_ahead/drain_open_skip/扫描循环的 is_skipping 臂同样只推进条件机
        // 不展开。故此处无需 is_skipping 臂（与 scan.rs 数字循环、scan_relation
        // 不同：那些是展开位置）。
        match tok.catcode() {
            Some(Catcode::BeginGroup) => {
                let mut tokens = Vec::new();
                let mut depth = 0usize;
                loop {
                    let t = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("实参组未闭合"))?
                        .0;
                    // TeX scan_toks(macro=true)：non-long 宏参数中任意深度的
                    // `\par` 都触发 "Paragraph ended"（TRIP L357 `\b{\par`）
                    if !long && self.is_par_token(t) {
                        self.recover_par_in_argument(name, t)?;
                        return Ok(Arc::from(tokens));
                    }
                    // TeX：组实参内的 outer 宏同样 forbidden
                    self.check_not_outer(t)?;
                    match t.catcode() {
                        Some(Catcode::BeginGroup) => {
                            depth += 1;
                            tokens.push(t);
                        }
                        Some(Catcode::EndGroup) => {
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                            tokens.push(t);
                        }
                        _ => tokens.push(t),
                    }
                }
                Ok(Arc::from(tokens))
            }
            Some(Catcode::EndGroup) => {
                // TeX scan_arg：无分隔实参遇 `}` → "! Argument of \X has an
                // extra }." 报错恢复（trip.log L6541），`}` 放回输入流供
                // 定界符扫描/主循环，实参为空（可恢复，不中断）。
                let _ = self.sink.write16(format!(
                    "! Argument of \\{name} has an extra }}.\n\
                     <to be read again>\n                   }}\n"
                ));
                self.unread(tok);
                Ok(Arc::from([]))
            }
            _ => {
                if !long && self.is_par_token(tok) {
                    self.recover_par_in_argument(name, tok)?;
                    return Ok(Arc::from([]));
                }
                // TeX：单 token 实参为 outer 宏 → forbidden
                self.check_not_outer(tok)?;
                Ok(Arc::from([tok]))
            }
        }
    }

    /// 判断 token 是否为 `\par`（M1：cat 5 字符或名为 "par" 的控制序列）。
    fn is_par_token(&self, tok: Token) -> bool {
        tok.catcode() == Some(Catcode::EndOfLine)
            || tok.csid().is_some_and(|id| self.intern.name(id) == "par")
    }

    /// TeX：outer 宏禁止出现在宏实参 / `\edef` / general text / `\read` 的 token
    /// 列表中（tex.web `forbidden`：`\outer` 宏只能在正常展开上下文使用）。
    /// 非 outer 宏或非宏 token 直接通过。
    fn check_not_outer(&self, tok: Token) -> Result<()> {
        let Some(csid) = tok.csid() else {
            return Ok(());
        };
        if let EqSlot::Macro(m) = self.eqtb.slot(csid) {
            if m.value.outer {
                return Err(Error::invalid_input(format!(
                    "forbidden control sequence \\{}（outer 宏禁止出现在参数/展开上下文）",
                    self.intern.name(csid)
                )));
            }
        }
        Ok(())
    }

    fn exec_def(&mut self, expand_body: bool) -> Result<()> {
        let name = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\def 后缺少控制序列"))?
            .0;
        let csid = name.csid().map(Ok).unwrap_or_else(|| {
            // TeX：`\def{...}`（非 cs）→ "! Missing control sequence inserted."
            // 恢复（插入 \inaccessible；TRIP L347 `\outer\def{}?`）。**offending
            // token 必须放回输入流**（tex.web back_input：`<to be read again> {`），
            // 否则参数文本扫描会吞掉它后面的整段文本当作参数文本。
            let _ = self.sink.write16(
                "! Missing control sequence inserted.\n\
                 Please don't say `\\def cs{...}', say `\\def\\cs{...}'.\n\
                 I've inserted an inaccessible control sequence so that your\n\
                 definition will be completed without mixing me up too badly.\n"
                    .to_string(),
            );
            self.unread(name);
            Ok(self.intern.intern("\u{0}inaccessible"))
        })?;

        let (num_params, param_text) = self.scan_parameter_text()?;
        // 取错误消息本体再补定义上下文，避免 "非法输入：非法输入：" 双前缀
        let cs_name = self.intern.name(csid).to_owned();
        let ctx = |e: Error| {
            let msg = match &e {
                Error::InvalidInput { message } => message.clone(),
                other => other.to_string(),
            };
            Error::invalid_input(format!("{msg}（定义 \\{cs_name} 的替换文本时）"))
        };
        let body: TokenArray = if expand_body {
            // e-TeX（ETRIP）：\edef/\xdef 体 = TeX scan_toks(macro_def, xpand)——
            // 扫描时即展开可展开项、组深含 \begingroup/\endgroup、条件即时求值；
            // 输入耗尽未配平 → "Runaway definition" 转录报告并以 } 收尾（可恢复）。
            self.suppress_expansion += 1;
            let scanned = self.scan_edef_body().map_err(ctx);
            self.suppress_expansion -= 1;
            Arc::from(scanned?)
        } else {
            Arc::from(self.scan_balanced_text().map_err(ctx)?)
        };

        // e-TeX（M4-5）：`\protected` 前缀标记宏（`\edef`/`\write` 等上下文不展开）；
        // `\outer` 前缀标记宏（禁止出现在实参/展开上下文/general text/`\read` 中）；
        // `\long` 前缀标记宏（参数允许含 `\par`）。
        let protected = std::mem::take(&mut self.protected_pending);
        let outer = std::mem::take(&mut self.outer_pending);
        let long = std::mem::take(&mut self.long_pending);
        let mut def = MacroDef {
            params: ParamSpec {
                num_params,
                long,
                text: param_text,
            },
            body,
            code: None,
            protected,
            outer,
        };
        // M2：编译期预编译字节码，解释器轨道不编译
        if self.use_bytecode {
            def.code = Some(Arc::new(compile(&def.body)));
        }
        self.define_macro_scoped(csid, def);
        Ok(())
    }

    /// 扫描参数文本直到 `{`，返回 (参数个数, 参数文本 token 数组)。
    /// 参数文本含 `#n` 参数 token 与定界符 token（M1-8 实参收集按此分段匹配）；
    /// `##` → 字面 `#`（跳过一个 #，文本中保留一个）。
    fn scan_parameter_text(&mut self) -> Result<(u8, TokenArray)> {
        let mut num = 0u8;
        let mut text = Vec::new();
        loop {
            let tok = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("\\def 参数文本未闭合（缺少 {）"))?
                .0;
            match tok.catcode() {
                Some(Catcode::BeginGroup) => break,
                // TeX scan_toks macro_def（tex.web L22945 `if cur_chr=...end_group`）：
                // 参数文本遇 `}` → 报 "Missing { inserted."（该 } 是 body 的开始括号
                // 缺失——`}` 放回作为隐含 `{` 后的 body 首内容起点，body 为空）。
                // trip.tex L397 `\def\a}{\let\a\xyzzy...` 依赖此恢复。
                Some(Catcode::EndGroup) => {
                    self.unread(tok);
                    let _ = self.sink.write16(
                        "! Missing { inserted.\n\
                         A left brace was mandatory here, so I've put one in.\n\
                         You might want to delete and/or insert some corrections\n\
                         so that I will find a matching right brace soon.\n\
                         (If you're confused by all this, try typing `I}' now.)\n"
                            .to_string(),
                    );
                    self.report_error_context();
                    break;
                }
                _ if is_parameter_char(tok) => {
                    let next = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("参数文本中 # 后无 token"))?
                        .0;
                    if let Some(d) = digit_value(next) {
                        if d == 0 {
                            return Err(Error::invalid_input("非法参数号 #0"));
                        }
                        num = num.max(d);
                        // 参数文本中 #n → MacroParam token（与宏体中的参数槽一致）
                        text.push(Token::macro_param(d));
                    } else if is_parameter_char(next) {
                        // ## → 字面 #：文本中保留一个 #
                        text.push(tok);
                    } else if next.catcode() == Some(Catcode::BeginGroup) {
                        // TeX：`#{` → 丢弃 `#`，`{` 作参数文本终止符被消费（**不**计入
                        // 定界符——pdfTeX 实测 `\t120100101001001{\relax}` 参数
                        // = `01001010`、剩余 `{\relax }` 含 `{`，TRIP L159/L161）。
                        break;
                    } else {
                        return Err(Error::invalid_input("参数文本中 # 后必须跟数字或 #"));
                    }
                }
                _ => text.push(tok),
            }
        }
        Ok((num, Arc::from(text)))
    }

    /// 扫描平衡花括号内的替换文本；`#n` → 参数槽 token，`##` → 字面 `#`。
    fn scan_balanced_text(&mut self) -> Result<Vec<Token>> {
        let mut out = Vec::new();
        let mut depth = 0usize;
        loop {
            let tok = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("替换文本未闭合（缺少 }）"))?
                .0;
            match tok.catcode() {
                Some(Catcode::EndGroup) => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    out.push(tok);
                }
                Some(Catcode::BeginGroup) => {
                    depth += 1;
                    out.push(tok);
                }
                _ if is_parameter_char(tok) => {
                    let next = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("替换文本中 # 后无 token"))?
                        .0;
                    if let Some(d) = digit_value(next) {
                        out.push(Token::macro_param(d));
                    } else if is_parameter_char(next) {
                        out.push(Token::char(Catcode::Parameter, b'#' as u32));
                    } else {
                        return Err(Error::invalid_input("替换文本中 # 后必须跟数字或 #"));
                    }
                }
                _ => {
                    // \outer 禁止出现在 \def/\edef 体（tex.web scan_toks
                    // macro_def 的 forbidden 检查）——报 Forbidden + 跳过
                    // （不收入体；TeX 为 Runaway definition + 插入 } 截断，
                    // 简化先对齐主消息）。展开期不再重复报（体里无 outer）。
                    if let Some(csid) = tok.csid() {
                        if matches!(
                            self.eqtb.slot(csid),
                            EqSlot::Macro(m) if m.value.outer
                        ) {
                            let name = self.cs_display_name(csid);
                            self.write_error(&format!(
                                "Forbidden control sequence found while scanning definition of {name}."
                            ));
                            continue;
                        }
                    }
                    out.push(tok);
                }
            }
        }
        Ok(out)
    }

    /// e-TeX（ETRIP）：`\edef`/`\xdef` 体 = TeX `scan_toks(macro_def, xpand=true)`。
    ///
    /// 与 [`scan_balanced_text`] 的差异：
    /// - **扫描时即展开**可展开项（宏/可展开原语/`\expandafter` 链），展开结果
    ///   压帧重新进入本扫描（递归语义），不再"先扫后展"两步；
    /// - **组深度计入 `\begingroup`/`\endgroup`**（TeX macro_def 模式组定界），
    ///   `\begingroup...\endgroup` 内的 `}` 不再误关宏体；
    /// - **条件原语即时求值**（`\iftrue` 等走 `cond_op`/`step_conditional`，
    ///   跳过分支的 token 直接丢弃，与 `process_one` 一致）；
    /// - **输入耗尽未配平** → 转录报告 "Runaway definition?" 并以 `}` 收尾
    ///   （可恢复，TeX 语义，不报错）；
    /// - `\edef` 上下文（`suppress_expansion > 0`）：protected 宏不展开，原样收入。
    fn scan_edef_body(&mut self) -> Result<Vec<Token>> {
        let mut out = Vec::new();
        let mut depth = 0usize;
        let mut runaway = false;
        'scan: loop {
            let Some((tok, noexpand)) = self.fetch()? else {
                runaway = depth > 0;
                break 'scan;
            };
            if noexpand {
                out.push(tok);
                continue;
            }
            // 条件原语：即时求值（优先级与 process_one 相同）
            if let Some(op) = self.cond_op(tok) {
                self.step_conditional(op)?;
                continue;
            }
            if self.is_skipping() {
                continue;
            }
            // 组定界：{ } 与 \begingroup/\endgroup（TeX macro_def 模式组定界）
            match tok.catcode() {
                Some(Catcode::EndGroup) => {
                    if depth == 0 {
                        break 'scan; // 外层 }：宏体结束（不收入体）
                    }
                    depth -= 1;
                    out.push(tok);
                    continue;
                }
                Some(Catcode::BeginGroup) => {
                    depth += 1;
                    out.push(tok);
                    continue;
                }
                _ => {}
            }
            let Some(csid) = tok.csid() else {
                // 字符 token：参数 # 处理（同 scan_balanced_text）
                if is_parameter_char(tok) {
                    let next = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("替换文本中 # 后无 token"))?
                        .0;
                    if let Some(d) = digit_value(next) {
                        out.push(Token::macro_param(d));
                    } else if is_parameter_char(next) {
                        out.push(Token::char(Catcode::Parameter, b'#' as u32));
                    } else {
                        return Err(Error::invalid_input("替换文本中 # 后必须跟数字或 #"));
                    }
                } else {
                    out.push(tok);
                }
                continue;
            };
            match self.eqtb.slot(csid).clone() {
                // \begingroup/\endgroup：TeX macro_def 模式组定界。
                // 注意：`\let\egroup=}` 是**字符别名**，tex.web scan_toks 只对
                // 原语等价（equiv=end_group）计数，字符别名不计数（etrip.tex
                // 29-34 行版本宏惯用 `\egroup` 于 \edef 体内即依赖此语义）→
                // 落入 `_` 原样收集，不改深度。
                EqSlot::Primitive(Primitive::BeginGroup) => {
                    depth += 1;
                    out.push(tok);
                }
                EqSlot::Primitive(Primitive::EndGroup) => {
                    if depth == 0 {
                        break 'scan;
                    }
                    depth -= 1;
                    out.push(tok);
                }
                // protected 宏在展开抑制上下文（\edef/\write）不展开 → 原样收入
                EqSlot::Macro(m) if m.value.protected && self.suppress_expansion > 0 => {
                    out.push(tok);
                }
                // 可展开项（宏/可展开原语）：展开后压帧，重新进入本扫描
                EqSlot::Macro(m) => {
                    // TeX：\edef 中 outer 宏 → forbidden
                    if m.value.outer {
                        return Err(Error::invalid_input(format!(
                            "forbidden control sequence \\{}（outer 宏禁止出现在 \\edef 展开上下文）",
                            self.intern.name(csid)
                        )));
                    }
                    let mut expansion = Vec::new();
                    self.expand_once((tok, noexpand), &mut expansion)?;
                    if expansion.is_empty() {
                        continue;
                    }
                    self.stack.push(InputFrame::TokenList {
                        items: Arc::from(expansion),
                        pos: 0,
                    });
                }
                EqSlot::Primitive(p) if p.is_expandable() => {
                    let mut expansion = Vec::new();
                    self.expand_once((tok, noexpand), &mut expansion)?;
                    // expand_once 不识别但声明可展开的原语（\uppercase/\lowercase/
                    // \char/\romannumeral 等）：原样返回自身——若压帧会无限循环
                    // （TRIP L338 `\edef\A{\uppercase{...}}` 曾因此 OOM 挂死）。
                    // TeX scan_toks 语义：展开不成则**执行**（扫描参数 + 发射结果）。
                    if expansion.len() == 1 && expansion[0].0 == tok {
                        self.exec_primitive(p)?;
                        continue;
                    }
                    if expansion.is_empty() {
                        continue;
                    }
                    self.stack.push(InputFrame::TokenList {
                        items: Arc::from(expansion),
                        pos: 0,
                    });
                }
                _ => out.push(tok),
            }
        }
        if runaway {
            let _ = self.sink.write16("Runaway definition?\n".to_owned());
        }
        Ok(out)
    }

    /// 把 token 列表放到输入流顶并全展开（`\edef` 用），返回展开结果。
    ///
    /// 通过临时提升 `read_floor` 划定区域边界，防止越过该区域读取外层输入；
    /// 区域内条件必须闭合（回到进入时的条件栈深度）。
    fn expand_region(&mut self, tokens: Vec<Token>) -> Result<Vec<Token>> {
        let saved_floor = self.read_floor;
        let depth = self.stack.len();
        let cond_depth = self.cond_stack.len();
        self.read_floor = depth;
        // e-TeX（M4-5）：\edef/\write 等展开上下文抑制 protected 宏展开
        self.suppress_expansion += 1;
        // TeX：\tracingcommands 只在 main_control 主循环追踪——区域展开抑制
        self.trace_suppress += 1;
        // \edef/\xdef/\write：TeX expand() 语义——只展开可展开项，
        // 不可展开原语/未定义 cs/字符/组定界原样保留（不执行、不建组）
        self.expand_only = true;
        // 区域输出重定向到临时 VecSink（M3-2：sink 替代 output 字段）；
        // 内部量查询（\currentgrouptype/\lastnodetype）转发到原 sink
        let saved = std::mem::replace(&mut self.sink, Box::new(VecSink::default()));
        self.query_sink = Some(saved);
        let outcome = (|| -> Result<Vec<Token>> {
            #[cfg(debug_assertions)]
            let shown: Vec<String> = tokens.iter().take(24).map(|t| match t.csid() {
                Some(c) => format!("\\{}", self.intern.name(c)),
                None => format!("{:?}({:?})", t.charcode(), t.catcode()),
            }).collect();
            let items: Vec<(Token, bool)> = tokens.into_iter().map(|t| (t, false)).collect();
            self.stack.push(InputFrame::TokenList {
                items: Arc::from(items),
                pos: 0,
            });
            while self.process_one()? {}
            // TeX 允许条件帧跨 \message/\write 参数边界（\ifx 在参数内求值、
            // \fi 在括号外闭合），故只对"过度闭合"（深度低于入口）报错；
            // 遗留的帧交由外层主循环正常闭合。
            if self.cond_stack.len() < cond_depth {
                #[cfg(debug_assertions)]
                {
                    let frames: Vec<String> = self
                        .cond_stack
                        .iter()
                        .map(|f| format!("{{is_case={} state={:?} owns_skip={} else={}}}",
                            f.is_case, f.state, f.owns_skip, f.else_seen))
                        .collect();
                    eprintln!("[debug] expand_region 条件未闭合: caller={} cond_depth={} len={} frames={:?}",
                        self.debug_expand_caller, cond_depth, self.cond_stack.len(), frames);
                    eprintln!("[debug]   edef tokens head: {:?}", shown);
                }
                return Err(Error::invalid_input("条件未闭合（缺少 \\fi）"));
            }
            let temp = std::mem::replace(
                &mut self.sink,
                self.query_sink
                    .take()
                    .expect("expand_region 设置了 query_sink"),
            );
            Ok(temp.take_tokens().expect("expand_region 安装了 VecSink"))
        })();
        // 统一恢复（错误路径下 sink 保持区域 VecSink，引擎随之终止）
        self.suppress_expansion -= 1;
        self.trace_suppress -= 1;
        self.expand_only = false;
        self.read_floor = saved_floor;
        outcome
    }

    /// `\let\cs<token>`：cs 别名到控制序列或等价于字符。
    /// TeX 语义：`=` 是可选赋值符（`\let\cs=x` 等价 `\let\cs x`）。
    fn exec_let(&mut self) -> Result<()> {
        let name = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\let 后缺少控制序列"))?
            .0;
        let csid = name
            .csid()
            .ok_or_else(|| Error::invalid_input("\\let 后必须是控制序列"))?;

        // 可选空格 + 可选 '='
        self.skip_spaces()?;
        let probe = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\let 后缺少被别名 token"))?
            .0;
        let rhs = if probe.charcode() == Some(b'=' as u32) {
            // TeX `\let` 的 '=' 分支：`get_token` 读一个 token；若它是空格
            // （cat 10）则再读一个——只跳**恰好一个**空格，源 token 不跳过空白
            // （TRIP L416 `\test. \show\test` 中 `\let\test= ` 后 `\test`
            //  必须别名到空格 token，而非 `\show`）。
            let mut t = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("\\let 后缺少被别名 token"))?
                .0;
            if t.catcode() == Some(Catcode::Space) {
                t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\let 后缺少被别名 token"))?
                    .0;
            }
            t
        } else {
            self.unread(probe);
            self.fetch()?
                .ok_or_else(|| Error::invalid_input("\\let 后缺少被别名 token"))?
                .0
        };

        let global = self.is_global();
        // e-TeX \tracingassigns（misc 5）：\let 赋值追踪（changing/into/reassigning）
        let prev_trace = if self.params.misc[5] > 0 {
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
        match rhs.kind() {
            TokenKind::ControlSeq => {
                let target = rhs.csid().expect("ControlSeq 必有 csid");
                // TeX 语义：\let 复制右侧**当前**含义（不随重定义漂移）——原语/字符/
                // 字体/寄存器/宏直接复制值（宏 Arc 共享，bump 后不漂移）；Alias 链压缩。
                match self.eqtb.slot(target).clone() {
                    EqSlot::Alias(t2) => self.eqtb.alias(csid, t2),
                    other => *self.eqtb.slot_mut(csid) = other,
                }
            }
            TokenKind::Char => {
                let catcode = rhs.catcode().expect("Char 必有 catcode");
                let charcode = rhs.charcode().expect("Char 必有 charcode");
                self.eqtb.char_alias(csid, catcode, charcode);
            }
            _ => return Err(Error::invalid_input("\\let 仅支持控制序列或字符")),
        }
        // \tracingassigns：\let 赋值后打点（prev 在赋值前已存）
        if let Some(prev) = prev_trace {
            let new = self.eqtb.slot(csid).clone();
            self.trace_assign(csid, global, &prev, &new);
        }
        self.finish_assignment();
        Ok(())
    }

    // ---------- M1-7 扫描顺序原语 ----------

    /// `\futurelet\cs T1 T2`：\cs ← \let T2（不展开），T1、T2 继续正常处理。
    fn exec_futurelet(&mut self) -> Result<()> {
        let name = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\futurelet 后缺少控制序列"))?
            .0;
        let csid = name
            .csid()
            .ok_or_else(|| Error::invalid_input("\\futurelet 后必须是控制序列"))?;
        let t1 = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\futurelet 后缺少 token"))?
            .0;
        let t2 = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\futurelet 后缺少被观察 token"))?
            .0;
        self.let_to(csid, t2);
        let items: Vec<(Token, bool)> = vec![(t1, false), (t2, false)];
        self.stack.push(InputFrame::TokenList {
            items: Arc::from(items),
            pos: 0,
        });
        Ok(())
    }

    /// `\let` 语义：cs 等价于 token（控制序列别名 / 字符等价）。
    fn let_to(&mut self, csid: u32, rhs: Token) {
        match rhs.kind() {
            TokenKind::ControlSeq => {
                let target = rhs.csid().expect("ControlSeq 必有 csid");
                // TeX `\let` 复制右侧**当前**含义（而非 csid 引用）：
                // 原语/字符/字体/寄存器等不可变含义直接复制，重定义后别名不漂移
                // （trip.tex `\let\paR=\par` → 重定义 `\par` → `\let\par=\paR` 恢复）。
                match self.eqtb.slot(target).clone() {
                    EqSlot::Alias(t2) => self.eqtb.alias(csid, t2),
                    EqSlot::Primitive(_)
                    | EqSlot::Char { .. }
                    | EqSlot::Font(_)
                    | EqSlot::Register(..)
                    | EqSlot::Stream(..) => {
                        let slot = self.eqtb.slot(target).clone();
                        *self.eqtb.slot_mut(csid) = slot;
                    }
                    // 宏/未定义：保持间接引用（宏不复制宏体，M1 简化）
                    _ => self.eqtb.alias(csid, target),
                }
            }
            TokenKind::Char => {
                let catcode = rhs.catcode().expect("Char 必有 catcode");
                let charcode = rhs.charcode().expect("Char 必有 charcode");
                self.eqtb.char_alias(csid, catcode, charcode);
            }
            _ => {} // 非字符/控制序列：忽略（TeX 报错，M1 宽松）
        }
    }

}
