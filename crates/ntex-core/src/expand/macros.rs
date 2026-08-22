impl Expander {
    // ---------- 宏调用与实参 ----------

    /// 收集宏的全部实参（M1-8 分隔参数）。
    ///
    /// 按 TeX scan_args 语义处理参数文本 `P_1 #1 P_2 #2 ... P_n P_{n+1}`：
    /// - 先匹配前导定界符 P_1（须与输入开头逐 token 相同）；
    /// - 对每个 `#k`：若其后定界符 P_{k+1} 为空 → 无分隔参数（单个 token 或组）；
    ///   否则为分隔参数，收集到 P_{k+1} 在输入中完整出现为止（定界符被消费）。
    fn collect_args(&mut self, def: &MacroDef) -> Result<Vec<TokenArray>> {
        let n = def.params.num_params as usize;
        if n == 0 {
            return Ok(Vec::new());
        }
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
        self.match_input_delim(&segments[0])?;
        for k in 0..n {
            let delim = &segments[k + 1]; // P_{k+2}：紧跟在 #(k+1) 后的定界符
            let arg = if delim.is_empty() {
                self.collect_undelimited_arg(def.params.long)?
            } else {
                self.collect_delimited_arg(delim, def.params.long)?
            };
            args.push(arg);
        }
        Ok(args)
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

    /// 定界符 token 等价比较：字符按 (catcode, char)；控制序列按含义（\ifx 语义）。
    fn delim_token_eq(&self, a: Token, b: Token) -> bool {
        match (a.kind(), b.kind()) {
            (TokenKind::Char, TokenKind::Char) => a == b,
            (TokenKind::ControlSeq, TokenKind::ControlSeq) => {
                self.meaning_key(a.csid().expect("ControlSeq 必有 csid"))
                    == self.meaning_key(b.csid().expect("ControlSeq 必有 csid"))
            }
            _ => false,
        }
    }

    /// 收集一个分隔实参：读入 token 直到定界符序列在输入中完整匹配（后缀匹配）。
    fn collect_delimited_arg(&mut self, delim: &[Token], long: bool) -> Result<TokenArray> {
        let mut buf: Vec<Token> = Vec::new();
        // 参数内未闭合 `\if*` 数：TeX scan_toks 跟踪实参内条件配对
        let mut arg_cond = 0usize;
        loop {
            let tok = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("分隔实参扫描到输入末尾（定界符未出现）"))?
                .0;
            if !long && self.is_par_token(tok) {
                return Err(Error::invalid_input("参数包含 \\par（宏未声明 \\long）"));
            }
            // 实参内条件：开 `\if*` 作数据并计数；闭合 token 先匹配参数内条件，
            // 无匹配（arg_cond==0）时是**外层**条件的 `\else/\fi/\or` → 交条件机，
            // 不作为实参（同无分隔实参的修复）。
            if let Some(op) = self.cond_op(tok) {
                if matches!(
                    op,
                    CondOp::If
                        | CondOp::IfCat
                        | CondOp::IfNum
                        | CondOp::IfDim
                        | CondOp::IfX
                        | CondOp::IfOdd
                        | CondOp::IfCase
                        | CondOp::IfTrue
                        | CondOp::IfFalse
                        | CondOp::IfDefined
                        | CondOp::IfCsname
                        | CondOp::IfPrimitive
                ) {
                    arg_cond += 1;
                } else if arg_cond > 0 {
                    if op == CondOp::Fi {
                        arg_cond -= 1;
                    }
                } else {
                    self.step_conditional(op)?;
                    continue;
                }
            }
            buf.push(tok);
            if self.suffix_matches_delim(&buf, delim) {
                buf.truncate(buf.len() - delim.len());
                break;
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
    fn collect_undelimited_arg(&mut self, long: bool) -> Result<TokenArray> {
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
        // TeX scan_args：实参扫描遇**外层**条件 token（\else/\fi/\or，非实参内
        // 嵌套条件）时先由条件机处理，不作为实参（`\expandafter\2\fi` 惯用法：
        // \fi 闭合外层 \ifx 后，\2 的实参是 \fi 之后的 token）。若将 \fi 当作
        // 实参，条件帧永不弹出 → 递归宏无限循环。
        if let Some(op) = self.cond_op(tok) {
            match op {
                CondOp::Else | CondOp::Fi | CondOp::Or => {
                    self.step_conditional(op)?;
                    // 继续扫描实参（\fi 已消费）
                    return self.collect_undelimited_arg(long);
                }
                _ => {} // \if*：实参数据（TeX 参数内开条件作数据）
            }
        }
        match tok.catcode() {
            Some(Catcode::BeginGroup) => {
                let mut tokens = Vec::new();
                let mut depth = 0usize;
                loop {
                    let t = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("实参组未闭合"))?
                        .0;
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
            _ => {
                if !long && self.is_par_token(tok) {
                    return Err(Error::invalid_input("参数包含 \\par（宏未声明 \\long）"));
                }
                Ok(Arc::from([tok]))
            }
        }
    }

    /// 判断 token 是否为 `\par`（M1：cat 5 字符或名为 "par" 的控制序列）。
    fn is_par_token(&self, tok: Token) -> bool {
        tok.catcode() == Some(Catcode::EndOfLine)
            || tok.csid().is_some_and(|id| self.intern.name(id) == "par")
    }

    fn exec_def(&mut self, expand_body: bool) -> Result<()> {
        let name = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\def 后缺少控制序列"))?
            .0;
        let csid = name
            .csid()
            .ok_or_else(|| Error::invalid_input("\\def 后必须是控制序列"))?;

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
        // `\outer` 前缀仅消费（outer 语义限制后续迭代补）。
        let protected = std::mem::take(&mut self.protected_pending);
        let _ = std::mem::take(&mut self.outer_pending);
        let mut def = MacroDef {
            params: ParamSpec {
                num_params,
                long: false,
                text: param_text,
            },
            body,
            code: None,
            protected,
        };
        // M2：编译期预编译字节码（常量条件折叠等），解释器轨道不编译
        if self.use_bytecode {
            def.code = Some(Arc::new(compile(&def.body, &self.eqtb)));
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
                _ => out.push(tok),
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
                EqSlot::Macro(_) => {
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
        // \edef/\xdef/\write：TeX expand() 语义——只展开可展开项，
        // 不可展开原语/未定义 cs/字符/组定界原样保留（不执行、不建组）
        self.expand_only = true;
        // 区域输出重定向到临时 VecSink（M3-2：sink 替代 output 字段）
        let saved = std::mem::replace(&mut self.sink, Box::new(VecSink::default()));
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
            if self.cond_stack.len() != cond_depth {
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
            let temp = std::mem::replace(&mut self.sink, saved);
            Ok(temp.take_tokens().expect("expand_region 安装了 VecSink"))
        })();
        // 统一恢复（错误路径下 sink 保持区域 VecSink，引擎随之终止）
        self.suppress_expansion -= 1;
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
            self.skip_spaces()?;
            self.fetch()?
                .ok_or_else(|| Error::invalid_input("\\let 后缺少被别名 token"))?
                .0
        } else {
            self.unread(probe);
            self.fetch()?
                .ok_or_else(|| Error::invalid_input("\\let 后缺少被别名 token"))?
                .0
        };

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
