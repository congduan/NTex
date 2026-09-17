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
        // 第二十刀定位开关（NTEX_ARG_DUMP=1）：调用瞬间 dump 参数文本
        //（cs/char/param 分类）。零成本门控，排查定界/非定界实参失配时开。
        if std::env::var_os("NTEX_ARG_DUMP").is_some() {
            let pt: Vec<String> = def
                .params
                .text
                .iter()
                .map(|t| {
                    if let Some(csid) = t.csid() {
                        format!("\\{}", self.intern.name(csid))
                    } else if let Some(d) = t.param_number() {
                        format!("#{d}")
                    } else {
                        format!(
                            "{:?}/cc{:?}",
                            t.charcode().and_then(char::from_u32),
                            t.catcode()
                        )
                    }
                })
                .collect();
            eprintln!(
                "[arg-dump] \\{} n={n} text={pt:?} body0={:?}",
                self.intern.name(csid),
                def.body.first().map(|t| {
                    if let Some(d) = t.param_number() {
                        format!("#{d}")
                    } else if let Some(csid) = t.csid() {
                        format!("\\{}", self.intern.name(csid))
                    } else {
                        format!("{:?}", t.charcode().and_then(char::from_u32))
                    }
                })
            );
        }
        self.diag_trace(format!(
            "ARGS \\{} n={n} floor={}",
            self.intern.name(csid),
            self.read_floor
        ));
        if n == 0 {
            // tex.web macro_call（L7971）：`if info(r)<>end_match_token then
            // @<Scan the parameters...@>`——参数文本非空时**即使 0 参数**也须在
            // 调用点匹配定界串（"@<Scan a parameter...@> 中 |s=null| 的
            // "simply scan the delimiter string" 分支），匹配失败报
            // "Use of \X doesn't match its definition." 并忽略该调用。
            // `\def\X\fi:\use:n{...}`（tex.web 0 参数宏的参数文本 = 纯定界串，
            // expl3 条件生成器 fast form 的 `\__prg_F_true:w`/`\__prg_TF_true:w`
            // /`\__prg_p_true:w` 即此形态）依赖它吞掉 `\fi: \use:n` 并由体首
            // `\fi:` 闭合所在条件。旧实现直接返回空实参，`\use:n` 泄出被执行：
            // `\cs_if_free:N` 对未定义 cs 误判"已定义" → expl3 kernel
            // `command-already-defined` bail out（expl3-code.tex l.2056
            // `\__kernel_chk_if_free_cs:N`，latex.ltx --initex l.398 终态）。
            if !def.params.text.is_empty()
                && self.match_input_delim(&def.params.text).is_err()
            {
                let name = self.intern.name(csid).to_owned();
                if std::env::var_os("NTEX_DELIM_DBG").is_some() {
                    let pt: Vec<String> = def
                        .params
                        .text
                        .iter()
                        .map(|t| {
                            if let Some(c) = t.csid() {
                                format!("\\{}", self.intern.name(c))
                            } else {
                                format!("{:?}", t.charcode().and_then(char::from_u32))
                            }
                        })
                        .collect();
                    eprintln!(
                        "[delim-mismatch] csid={csid} name={name:?} params_text={pt:?}"
                    );
                    // 调用者：warning_index = call_macro 设置的「正在扫的宏」
                    let caller = self
                        .warning_index
                        .map(|c| format!("\\{}", self.intern.name(c)))
                        .unwrap_or_else(|| "(none)".into());
                    eprintln!("[delim-mismatch] caller={caller}");
                    let body6: Vec<String> = def
                        .body
                        .iter()
                        .take(6)
                        .map(|t| {
                            if let Some(c) = t.csid() {
                                format!("\\{}", self.intern.name(c))
                            } else {
                                format!("{:?}", t.charcode().and_then(char::from_u32))
                            }
                        })
                        .collect();
                    eprintln!("[delim-mismatch] body6={body6:?}");
                    for (fi, fr) in self.stack.iter().enumerate().rev().take(4) {
                        if let crate::expand::InputFrame::TokenList { items, pos } = fr {
                            let toks: Vec<String> = items
                                .iter()
                                .skip(*pos)
                                .take(4)
                                .map(|(t, _)| {
                                    if let Some(c) = t.csid() {
                                        format!("\\{}", self.intern.name(c))
                                    } else {
                                        format!(
                                            "{:?}",
                                            t.charcode().and_then(char::from_u32)
                                        )
                                    }
                                })
                                .collect();
                            eprintln!("[delim-mismatch] frame[{fi}]@{pos}={toks:?}");
                        }
                    }
                    let kinds: Vec<&str> = self
                        .stack
                        .iter()
                        .rev()
                        .take(4)
                        .map(|f| match f {
                            crate::expand::InputFrame::Macro { .. } => "Macro",
                            crate::expand::InputFrame::TokenList { .. } => "TokList",
                            crate::expand::InputFrame::Source { .. } => "Src",
                            crate::expand::InputFrame::Bytecode { .. } => "Bc",
                            _ => "?",
                        })
                        .collect();
                    eprintln!("[delim-mismatch] stack(顶→底)={kinds:?}");
                    // 失配现场：peek 输入流下一个 token（不动流）
                    if let Ok(Some((nt, _))) = self.fetch() {
                        let d = if let Some(c) = nt.csid() {
                            format!("cs:{}", self.intern.name(c))
                        } else {
                            format!("char:{:?} cat:{:?}", nt.charcode(), nt.catcode())
                        };
                        eprintln!("[delim-mismatch] next_tok={d}");
                        self.unread(nt);
                    }
                }
                let _ = self.sink.write16(format!(
                    "! Use of \\{name} doesn't match its definition.\n\
                     The macro here has not been followed by the required stuff,\n\
                     so I'm ignoring it.\n"
                ));
            }
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
            let _ = self.sink.write16(format!(
                "! Use of \\{name} doesn't match its definition.\n\
                 The macro here has not been followed by the required stuff,\n\
                 so I'm ignoring it.\n"
            ));
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
            if self.arg_scan_recovered {
                // Paragraph-ended/extra-} 恢复已插入终止材料；tex.web 不会
                // 继续把它当作下一个参数的开头扫描。
                return Ok(Vec::new());
            }
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
        self.arg_scan_recovered = true;
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

    /// tex.web `macro_call` 的「额外右花括号」恢复：保留被拒的 `}` 供后续
    /// 主输入处理，却必须在它**上方**插入真正的 `\par` 来终止当前 non-long
    /// 实参扫描。此前复用了 [`Self::recover_par_in_argument`]，把同一枚 `}`
    /// 回推后又当作恢复材料读取；定界实参的残流遂可从 `\edef` 扫描器重放，
    /// 在 LaTeX NFSS `#1<#2>` 链形成 handler 内 fetch 自旋。
    fn recover_extra_end_group_in_argument(&mut self, tok: Token) {
        self.arg_scan_recovered = true;
        self.unread(tok);
        let par = Token::control_sequence(self.intern.intern("par"));
        self.unread(par);
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
        // TEMP DEBUG（挂死定位）
        let mut guard: u64 = 0;
        // 第十八刀（二）诊断护栏：默认仍取历史 4000 万上限；仅显式设置
        // NTEX_DELIM_GUARD 后才收紧，以便把单步活锁转成可定位的错误现场。
        // 实验已证实 latex.ltx 88.4% 主墙不在此路径；保留作今后定界实参回归诊断。
        let guard_limit = std::env::var("NTEX_DELIM_GUARD")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&v| v > 0)
            .unwrap_or(40_000_000);
        let dbg_on = std::env::var("NTEX_DELIM_DBG").is_ok();
        self.diag_trace(format!(
            "DELIM-BEGIN \\{name} dlen={} long={long} floor={}",
            delim.len(),
            self.read_floor
        ));
        let entry_last = self.last_tok.clone();
        let entry_stack = if dbg_on {
            self.debug_stack_summary()
        } else {
            String::new()
        };
        loop {
            guard += 1;
            if guard % 1_000_000 == 0 {
                self.diag_trace(format!(
                    "DELIM-PROG \\{name} guard={guard} buf={} depth={depth} floor={}",
                    buf.len(),
                    self.read_floor
                ));
            }
            if dbg_on && guard == 1_000_000 {
                let show = |t: &Token| match t.csid() {
                    Some(id) => format!("\\{}", self.intern.name(id)),
                    None => format!("c{}", t.charcode().unwrap_or(9999)),
                };
                let head: Vec<String> = buf.iter().take(32).map(show).collect();
                eprintln!(
                    "[delim-dbg] 失控定界实参 name={name} 达到 1M：buf.len={blen} depth={depth}\n  入口 last_tok={entry_last:?}\n  入口栈={entry_stack}\n  头32=[{head:?}]",
                    blen = buf.len()
                );
            }
            if guard > guard_limit {
                self.diag_trace(format!(
                    "DELIM-ABORT \\{name} guard={guard} limit={guard_limit} buf={} depth={depth} floor={}",
                    buf.len(),
                    self.read_floor
                ));
                eprintln!(
                    "[delim-guard] 超限退出 name={name} guard={guard} limit={guard_limit} buf.len={} depth={depth} floor={} stack={}",
                    buf.len(),
                    self.read_floor,
                    self.debug_stack_summary()
                );
                return Err(Error::invalid_input(format!(
                    "定界实参收集无终止（\\{name}，{} token）",
                    buf.len()
                )));
            }
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
            // TeX：实参位置的控制序列是 outer 宏 → Forbidden。
            // ⚠ 仅限**控制序列**（`should_check_outer`）：active char 取作实参
            // 不报（pdfTeX 对拍 `\csca:N \^^L` = 0 错误）。
            if let Err(e) = self.check_not_outer(tok) {
                if self.recover_forbidden_outer(&e) {
                    return Ok(Arc::from([]));
                }
                return Err(e);
            }
            match tok.catcode() {
                Some(Catcode::BeginGroup) => {
                    // TeX macro_call：定界符匹配**优先于**组贡献（`cur_tok=info(r)`
                    // 先于 "Contribute an entire group"）。`#{` 型参数文本把末位 `{`
                    // 存为定界符（scan_parameter_text hash_brace，expl3 p 型签名）：
                    // depth==0 的输入 `{` 若构成完整定界符后缀 → 作定界符消费（不开
                    // 组），否则按 tex.web "Contribute an entire group" 整组贡献——
                    // 组内 token 只配对、不匹配定界符（l3prg w 尾参整组贡献依赖）。
                    buf.push(tok);
                    if depth == 0 && self.suffix_matches_delim(&buf, delim) {
                        buf.truncate(buf.len() - delim.len());
                        break;
                    }
                    depth += 1;
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
                if long {
                    self.unread(tok);
                    buf.push(Token::control_sequence(self.intern.intern("par")));
                    return Ok(Arc::from(buf));
                }
                self.recover_extra_end_group_in_argument(tok);
                return Ok(Arc::from(buf));
            }
            // non-long 参数中 `\par`（非定界符位置）→ "Paragraph ended"（含组内）
            if !long && self.is_par_token(tok) {
                buf.pop();
                self.recover_par_in_argument(name, tok)?;
                return Ok(Arc::from(buf));
            }
        }
        Ok(Arc::from(strip_single_group(buf)))
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
    ///
    /// tex.web 语义注记（store_arg 裁决）：macro_call 对**无分隔**实参——
    /// 无论单 token 还是组——`m` 恒为 1（首轮即 `goto found`），故组实参在
    /// `@<Tidy up the parameter just scanned@>` 处**必被剥去外层花括号**
    /// （"If the parameter consists of a single group enclosed in braces, we
    /// must strip off the enclosing braces."）。存储进 param_stack 的值不含
    /// 花括号，宏体回填（begin_token_list(parameter)）也不再有剥组环节。
    /// TRIP log 的 45 条 `#N<-…` 追踪（tracing_macros）无一含花括号，与此一致。
    /// **不存在**"连组存储、使用时剥组"的两级模型。
    /// tex.web `@<Report a runaway argument and abort@>`（L8090+）：宏实参扫描
    /// 遇**输入耗尽**时报 `Runaway argument` 并**按空实参恢复**，作业继续。
    ///
    /// ⚠ 这是可恢复错误，不是致命错。pdfTeX 实测（2026-09-11）：
    /// ```tex
    /// \catcode`\~=\active \def~#1{[TIE:#1]}
    /// \immediate\write128{~}          % ~ 需 #1 而输入已尽
    /// \immediate\write128{[AFTER]}
    /// ```
    /// pdfTeX 输出 `Runaway argument` 后**继续执行**并打出 `[AFTER]`；
    /// 旧 NTex 报致命 `实参扫描到输入末尾` → **整个作业终止**，导致 l3kernel
    /// 8 例 CRASH（m3fp-logic004/m3int001/m3int003/m3prg001/m3skip002/
    /// m3skip006/m3tl002/m3tlist002 —— harness 把 `~` 定义为
    /// `\def~#1{\accent"7E #1}`，凡 `~` 出现在 write/参数组末尾即触发）。
    fn recover_runaway_arg(&mut self, name: &str) -> Result<TokenArray> {
        let _ = self.sink.write16(format!(
            "Runaway argument?\n\\{name}\n! File ended while scanning use of \\{name}.\n"
        ));
        Ok(Arc::from([]))
    }

    fn collect_undelimited_arg(&mut self, long: bool, name: &str) -> Result<TokenArray> {
        self.diag_trace(format!("UNDELIM \\{name} long={long} floor={}", self.read_floor));
        // 跳过前导空格
        loop {
            let Some(tok) = self.fetch()?.map(|p| p.0) else {
                return self.recover_runaway_arg(name);
            };
            if tok.catcode() != Some(Catcode::Space) {
                self.unread(tok);
                break;
            }
        }
        // tex.web macro_call：**无定界实参的首 token** 经 get_token 的
        // check_outer_validity——outer 宏在此报 Forbidden（pdfTeX 对拍
        // `\outer\def\x{A}\def\a#1{#1}\a\x` → Forbidden + 作业继续）。
        // 仅 Matching（宏实参扫描）+ 首 token 触发；恢复 = 插入 \par + 空实参续跑。
        // ⚠ 不可放宽到续 token/组内（2026-09-12 实测：expl3 载入 +231 Undefined，
        // \newif 类真 outer 宏在实参续位合法出现）。
        let Some(tok) = self.fetch()?.map(|p| p.0) else {
            return self.recover_runaway_arg(name);
        };
        if self.scanner_status == ScannerStatus::Matching {
            if let Some(csid) = tok.csid() {
                if self.is_outer_for_token(tok) {
                    let wname = match self.warning_index {
                        Some(cs) => self.cs_display_name(cs),
                        None => self.cs_display_name(csid),
                    };
                    let _ = self.sink.write16(format!(
                        "! Forbidden control sequence found while scanning use of {wname}.\n\
                         <inserted text> \n                \\par \n"
                    ));
                    let par = Token::control_sequence(self.intern.intern("par"));
                    self.unread(par);
                    return Ok(Arc::from([]));
                }
            }
        }
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
                    let t = match self.fetch()? {
                        Some(p) => p.0,
                        None => {
                            // 第二十刀定位开关（NTEX_ARG_EOF=1）：点名实参组扫描
                            // 逃逸到输入末尾的宏与当前行号。
                            if std::env::var_os("NTEX_ARG_EOF").is_some() {
                                let mut fs = Vec::new();
                                for f in self.stack.iter().rev().take(5) {
                                    use crate::expand::InputFrame;
                                    fs.push(match f {
                                        InputFrame::Source { pos, bytes, .. } => {
                                            format!("Src({pos}/{})", bytes.len())
                                        }
                                        InputFrame::Macro { body, pos, .. } => {
                                            format!("Mac({pos}/{})", body.len())
                                        }
                                        InputFrame::Bytecode { pc, .. } => {
                                            format!("Bc({pc})")
                                        }
                                        InputFrame::TokenList { items, pos }
                                        | InputFrame::OutputRoutine { items, pos } => {
                                            format!("Tl({pos}/{})", items.len())
                                        }
                                        InputFrame::MacroArg { items, pos } => {
                                            format!("Arg({pos}/{})", items.len())
                                        }
                                        InputFrame::One { .. } => "One".to_string(),
                                        _ => "?".to_string(),
                                    });
                                }
                                let got: Vec<String> = tokens
                                    .iter()
                                    .take(10)
                                    .map(|t: &Token| {
                                        if let Some(c) = t.csid() {
                                            format!("\\{}", self.intern.name(c))
                                        } else {
                                            format!(
                                                "{:?}/cc{:?}",
                                                t.charcode().and_then(char::from_u32),
                                                t.catcode()
                                            )
                                        }
                                    })
                                    .collect();
                                eprintln!(
                                    "[arg-eof] line={} macro=\\{name} depth={depth} \
                                     floor={} stack=[{}] got={got:?} (+{})",
                                    self.current_line_no(),
                                    self.read_floor,
                                    fs.join(" "),
                                    tokens.len().saturating_sub(10)
                                );
                            }
                            return Err(Error::invalid_input("实参组未闭合"));
                        }
                    };
                    // tex.web scan_toks 组贡献循环同样经 get_token 的
                    // check_outer_validity（pdfTeX 对拍 `\a{\x}` outer 组内
                    // → Forbidden + 作业继续）。
                    if self.scanner_status == ScannerStatus::Matching {
                        if let Some(acid) = t.csid() {
                            // tex.web L7398/L7433：active char 与 cs 各查**自己的
                            // 槽**——plain `\outer\def^^L{\par}` 写的是 active 槽，
                            // cs 形式 `\^^L` 的实参（expl3 L9320）不报（见
                            // `is_outer_for_token`）
                            if self.is_outer_for_token(t) {
                                let wname = match self.warning_index {
                                    Some(cs) => self.cs_display_name(cs),
                                    None => self.cs_display_name(acid),
                                };
                                let _ = self.sink.write16(format!(
                                    "! Forbidden control sequence found while scanning use of {wname}.\n\
                                     <inserted text> \n                \\par \n"
                                ));
                                return Ok(Arc::from(tokens));
                            }
                        }
                    }
                    // TeX scan_toks(macro=true)：non-long 宏参数中任意深度的
                    // `\par` 都触发 "Paragraph ended"（TRIP L357 `\b{\par`）
                    if !long && self.is_par_token(t) {
                        self.recover_par_in_argument(name, t)?;
                        return Ok(Arc::from(tokens));
                    }
                    // TeX：组实参内的**控制序列** outer 宏同样 forbidden
                    //（active char 不在其列，见 should_check_outer）。
                    if let Err(e) = self.check_not_outer(t) {
                        if self.recover_forbidden_outer(&e) {
                            return Ok(Arc::from(tokens));
                        }
                        return Err(e);
                    }
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
                self.recover_extra_end_group_in_argument(tok);
                Ok(Arc::from([]))
            }
            _ => {
                if !long && self.is_par_token(tok) {
                    self.recover_par_in_argument(name, tok)?;
                    return Ok(Arc::from([]));
                }
                // TeX：单 token 实参为 **outer 控制序列** → forbidden。
                // ⚠ active char 豁免（`should_check_outer`）：pdfTeX 对
                // `\csca:N \^^L` 报 0 错误，对 `\a\x`（\x 是普通 cs）报
                // Forbidden——差异在「取到的是 active char 而非控制序列」。
                if let Err(e) = self.check_not_outer(tok) {
                    if self.recover_forbidden_outer(&e) {
                        return Ok(Arc::from([tok]));
                    }
                    return Err(e);
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

    /// TeX：outer 宏禁止出现在宏实参 / `\edef` / general text / `\read` 的 token
    /// 列表中（tex.web `forbidden`：`\outer` 宏只能在正常展开上下文使用）。
    /// 非 outer 宏或非宏 token 直接通过。
    /// tex.web `@<Tell the user what has run away...@>` 的恢复动作
    /// （`scanner_status=matching`，L7210-7212）：打印
    /// `Forbidden control sequence found while scanning use of \X.` + help，
    /// **插入 `\par`**（`info(p):=par_token; long_state:=outer_call`），
    /// 当前实参扫描按空实参收场并**不中断作业**。
    ///
    /// 返回 true 表示「已按 outer 恢复处理」（调用方应 return 空实参）。
    fn recover_forbidden_outer(&mut self, e: &Error) -> bool {
        let Error::RecoverableOuter { name } = e else {
            return false;
        };
        let _ = self.sink.write16(format!(
            "! Forbidden control sequence found while scanning use of \\{name}.\n\
             <inserted text> \n                \\par \n\
             I suspect you have forgotten a `}}', causing me\n\
             to read past where you wanted me to stop.\n\
             I'll try to recover; but if the error is serious,\n\
             you'd better type `E' or `X' now and fix your file.\n"
        ));
        true
    }

    /// tex.web `@<Tell the user what has run away...@>`（L7184-7200）：**outer 宏
    /// 出现在参数/展开上下文是「可恢复错误」**（`error`，非 `fatal_error`），
    /// 恢复材料按 `scanner_status` 选择（L7206-7223）：`matching`（宏实参扫描）
    /// → 插入 `\par`；`absorbing`（general text）→ 插入 `}`。
    ///
    /// ⚠ pdfTeX ground truth（2026-09-11 实测）：
    /// ```tex
    /// \outer\def\O{outer-macro}
    /// \def\f#1{[#1]}
    /// \f\O                     % outer 宏作实参
    /// \message{[B]}
    /// ```
    /// pdfTeX：`[A]` + `Forbidden control sequence` + **`[B]`**（继续）✅
    /// 旧 NTex：返回致命 `Error::invalid_input` → **作业终止** ❌
    /// （l3kernel `m3fp-parse002`/`m3regex005` 即此因）。
    /// outer 判据（tex.web 双槽语义的压缩表示，2026-09-13 定性）：
    /// 只有「**token 形式 ↔ 槽的写入形式一致**」时槽里的 outer 才对该 token 可见。
    ///
    /// tex.web（L242 `single_base=active_base+256`）里 active char 与同名单字符
    /// cs 是**两个槽**：plain.tex L20 `\outer\def^^L{\par}` 只写 active 槽，cs 槽
    /// `\^^L` 保持 undefined。pdfTeX ground truth（2026-09-13 实测）：
    ///
    /// | 输入 | pdfTeX |
    /// |---|---|
    /// | `\outer\def\O{a}\def\f#1{[#1]}\f\O`（cs 形式、cs 写入）| Forbidden ✅ 报 |
    /// | `\outer\def^^L{\par}\def\g#1{[#1]}\g^^L`（active 形式、active 写入）| Forbidden ✅ 报 |
    /// | `\def\h#1{[#1]}\h\^^L`（cs 形式、槽是 active 写入）| **不报**（后续用到才 Undefined）|
    ///
    /// ⚠ 旧实现用「名字是否单字符」豁免（`should_check_outer`）——两处都错：
    /// 单字符 **cs** 写入的 outer（`\O`）被漏报，active 写入被误继承给 cs 形式
    /// （expl3 L9320 `\char_set_catcode_active:N \^^L` → Forbidden 级联 →
    /// lvt 187/187 STACK 首错）。
    fn is_outer_for_token(&self, tok: Token) -> bool {
        let Some(csid) = tok.csid() else {
            return false;
        };
        matches!(self.eqtb.slot(csid), EqSlot::Macro(m) if m.value.outer && m.value.active_slot == tok.is_active())
    }

    fn check_not_outer(&self, tok: Token) -> Result<()> {
        if self.is_outer_for_token(tok) {
            let csid = tok.csid().expect("is_outer_for_token 已判 csid");
            return Err(Error::recoverable_outer(
                self.intern.name(csid).to_owned(),
            ));
        }
        Ok(())
    }

    fn exec_def(&mut self, expand_body: bool) -> Result<()> {
        let name = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\def 后缺少控制序列"))?
            .0;
        // tex.web：`\def` 目标若是 **active char token**，定义写进 `active_base+c`
        // 槽（与同名单字符 cs 槽互不可见）——见 [`MacroDef::active_slot`]。
        let active_slot = name.is_active();
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

        let (num_params, param_text, hash_brace) = self.scan_parameter_text()?;
        // 取错误消息本体再补定义上下文，避免 "非法输入：非法输入：" 双前缀
        let cs_name = self.intern.name(csid).to_owned();
        let ctx = |e: Error| {
            let msg = match &e {
                Error::InvalidInput { message } => message.clone(),
                other => other.to_string(),
            };
            Error::invalid_input(format!("{msg}（定义 \\{cs_name} 的替换文本时）"))
        };
        let mut body_toks: Vec<Token> = if expand_body {
            // e-TeX（ETRIP）：\edef/\xdef 体 = TeX scan_toks(macro_def, xpand)——
            // 扫描时即展开可展开项、组深含 \begingroup/\endgroup、条件即时求值；
            // 输入耗尽未配平 → "Runaway definition" 转录报告并以 } 收尾（可恢复）。
            self.suppress_expansion += 1;
            let scanned = self.scan_edef_body(&cs_name, true).map_err(ctx);
            self.suppress_expansion -= 1;
            scanned?
        } else {
            self.scan_balanced_text(&cs_name).map_err(ctx)?
        };
        // TeX scan_toks（macro_def）hash_brace：参数文本以 `#{` 收尾时，`{` 被存为
        // 末参定界符，同时把同一 `{` 追加到宏体 token 列**末尾**。调用时该定界符
        // `{` 从输入消费作定界（不开组），源 `{...}` 组的 `}` 改由这枚体尾 `{`
        // 配对——expl3 p 型签名（`#1#2#3#4#`，p 实参 = 到下一组为止的参数文本）即
        // 依赖此语义；TRIP L161 `\t120100101001001{\relax}` 的 `{\relax}` 组同样由
        // 它重开（trip.log 实测 `#1<-01001010` 后接 begin-group）。
        if let Some(hb) = hash_brace {
            body_toks.push(hb);
        }
        let body: TokenArray = Arc::from(body_toks);

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
            active_slot,
        };
        if diag_enabled("NTEX_IFX_TRACE") && cs_name.contains("cs_replacement_spec") {
            let body_dbg: Vec<String> = def.body.iter().map(|t| format!("{t:?}")).collect();
            eprintln!(
                "[trace-def] line={} \\{} params={num_params} body={}",
                self.current_line_no(),
                cs_name,
                body_dbg.join(",")
            );
        }
        if std::env::var_os("NTEX_HOOK_TRACE").is_some() && cs_name == "__hook_make_name:w" {
            let show = |t: &Token| {
                if let Some(csid) = t.csid() {
                    format!("\\{}", self.intern.name(csid))
                } else if let Some(n) = t.param_number() {
                    format!("#{n}")
                } else if let Some(ch) = t.charcode().and_then(char::from_u32) {
                    format!("{:?}/{:?}", ch, t.catcode())
                } else {
                    format!("{t:?}")
                }
            };
            let params = def.params.text.iter().map(show).collect::<Vec<_>>().join(" ");
            let body = def.body.iter().map(show).collect::<Vec<_>>().join(" ");
            eprintln!(
                "[hook-def] line={} params={} text=[{}] body=[{}] long={} protected={}",
                self.current_line_no(),
                def.params.num_params,
                params,
                body,
                def.params.long,
                def.protected
            );
        }
        // M2：编译期预编译字节码，解释器轨道不编译
        if self.use_bytecode {
            def.code = Some(Arc::new(compile(&def.body)));
        }
        self.define_macro_scoped(csid, def);
        Ok(())
    }

    /// 扫描参数文本直到 `{`，返回 (参数个数, 参数文本 token 数组, hash_brace)。
    /// 参数文本含 `#n` 参数 token 与定界符 token（M1-8 实参收集按此分段匹配）；
    /// `##` → 字面 `#`（跳过一个 #，文本中保留一个）。第三元组项 = 参数文本以
    /// `#{` 收尾时需追加到宏体末尾的 `{` token（tex.web scan_toks hash_brace，
    /// 无则 `None`）。
    fn scan_parameter_text(&mut self) -> Result<(u8, TokenArray, Option<Token>)> {
        let mut num = 0u8;
        let mut text = Vec::new();
        let mut hash_brace: Option<Token> = None;
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
                        // TeX scan_toks macro_def hash_brace（tex.web `#{` 分支）：
                        // 参数文本以 `#` 紧接 body 的 `{` 收尾 → `#` 丢弃（非定界符），
                        // 该 `{` **计入**末参定界符（末参 = 分隔实参，定界符末 token
                        // 即此 `{`）；宏体末尾须另补一枚同一 `{`（hash_brace），调用时
                        // 与源组 `}` 配对。expl3 p 型签名（`NNNpnn` 的 `#1#2#3#4#`）
                        // 的 p 实参即"到下一个 `{` 组为止"——p-arg = 用户参数文本
                        // `#1`，表单清单 `{p,T,F,TF}` 的 `{` 是定界符而非组开。
                        // TRIP L159 `\def\t12#101001#{-.#1pt}` 同此：末参定界符 =
                        // `01001{`，调用 `\t120100101001001{\relax}` 实参 `01001010`
                        // （trip.log L1043-1044，含定界符消费、体尾 `{` 重开组）。
                        text.push(next);
                        hash_brace = Some(next);
                        break;
                    } else {
                        return Err(Error::invalid_input("参数文本中 # 后必须跟数字或 #"));
                    }
                }
                _ => text.push(tok),
            }
        }
        Ok((num, Arc::from(text), hash_brace))
    }

    /// 扫描平衡花括号内的替换文本；`#n` → 参数槽 token，`##` → 字面 `#`。
    fn scan_balanced_text(&mut self, def_name: &str) -> Result<Vec<Token>> {
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
                        // tex.web @<Look for parameter number or ##@>（L9416-9423）：
                        // "Illegal parameter number in definition of \<cs>" **可恢复**
                        // ——back_error 把越界 token 放回输入流、`cur_tok:=s` 存回
                        // 字面 `#` 后继续扫描。旧实现 fatal 中止，latex.ltx l.10163
                        // 区（l3prop 生成器体的 `#` 接非数字）被整体截断。
                        self.unread(next);
                        let def = if def_name.is_empty() {
                            String::new()
                        } else {
                            format!(" \\{def_name}")
                        };
                        self.write_error_help(
                            &format!("Illegal parameter number in definition of{def}."),
                            "You meant to type ## instead of #, right?\n\
                             Or maybe a } was forgotten somewhere earlier, and things\n\
                             are all screwed up? I'm going to assume that you meant ##.\n",
                        );
                        out.push(tok);
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

    /// e-TeX（ETRIP）：`\edef`/`\xdef` 体与 pdfTeX `\expanded` 实参 = TeX
    /// `scan_toks(macro_def, xpand=true)` 的两种形态，由 `in_definition` 区分。
    ///
    /// 共同点（xpand=true）：
    /// - **扫描时即展开**可展开项（宏/可展开原语/`\expandafter` 链），展开结果
    ///   压帧重新进入本扫描（递归语义），不再"先扫后展"两步；
    /// - **组深度计入 `\begingroup`/`\endgroup`**（TeX macro_def 模式组定界），
    ///   `\begingroup...\endgroup` 内的 `}` 不再误关宏体；
    /// - **条件原语即时求值**（`\iftrue` 等走 `cond_op`/`step_conditional`，
    ///   跳过分支的 token 直接丢弃，与 `process_one` 一致）；
    /// - **输入耗尽未配平** → 转录报告 "Runaway definition?" 并以 `}` 收尾
    ///   （可恢复，TeX 语义，不报错）；
    /// - 展开抑制上下文（`suppress_expansion > 0`）：protected 宏不展开，原样收入。
    ///
    /// 差异（macro_def 位，tex.web scan_toks L9405 区）：
    /// - `in_definition=true`（`\edef`/`\xdef`）：字符 token 走参数 `#` 处理
    ///   ——`#<数字>` → macro_param、`##` → 字面 `#`、越界 → IPN（可恢复）；
    /// - `in_definition=false`（`\expanded`，tex.web scan_toks(false,true)）：
    ///   **不做参数 `#` 处理**——字面 `#`（含 cat 6）原样收集、不报
    ///   Illegal parameter number。expl3-code l.9356-9372 经 `\lowercase`
    ///   构造 catcode 查表时 `#`（cat 6）进入 `\expanded` 实参即依赖此语义
    ///   （latex.ltx --initex 256 条 IPN 的根因，2026-09-08 修复）。
    fn scan_edef_body(&mut self, def_name: &str, in_definition: bool) -> Result<Vec<Token>> {
        let mut out = Vec::new();
        // tex.web scan_toks（L9394）：`unbalance:=1` 起始——首个 `{` 已被
        // scan_left_brace/参数部消费，体扫描从「已开一个未配平组」起算。
        // NTex 由调用方 scan_left_brace 消费 `{`（或调用点等价保证），此处
        // 起点 1 与之对齐。此前的 `depth:=0` + 首 `{` 自计使配平判定整体
        // 偏移 1：skip 区吞掉体边界 `}` 后，`\fi:` 闭合 skip，后续源码的
        // `}` 触发 `depth==0 → break` —— 宏体「正常」提前闭合，**Runaway
        // 判定被整个绕过**，残留条件帧由 expand_region 守卫误报「缺少 \fi」
        // （expl3-code L25851 `{ \if_false: } \fi:` 空组惯用法现场，
        // 187 例全 STACK 的根因；2026-09-12 trace 定性）。
        let mut unbalance = 1usize;
        let mut runaway = false;
        'scan: loop {
            let Some((tok, noexpand)) = self.fetch()? else {
                runaway = unbalance > 0;
                break 'scan;
            };
            if noexpand {
                out.push(tok);
                continue;
            }
            // 条件原语：即时求值（优先级与 process_one 相同）
            if let Some(op) = self.cond_op(tok) {
                if std::env::var_os("NTEX_EDEF_COND_TRACE").is_some() {
                    let nm = match tok.csid() {
                        Some(id) => self.intern.name(id).to_owned(),
                        None => format!("{tok:?}"),
                    };
                    eprintln!(
                        "[edef-cond] line={} tok=\\{} op={op:?} skip_before={}",
                        self.current_line_no(),
                        nm,
                        self.is_skipping()
                    );
                }
                self.step_conditional(op, tok)?;
                continue;
            }
            if self.is_skipping() {
                if std::env::var_os("NTEX_EDEF_COND_TRACE").is_some() {
                    let nm = match tok.csid() {
                        Some(id) => format!("\\{}", self.intern.name(id)),
                        None => format!("{tok:?}"),
                    };
                    eprintln!("[edef-skip] line={} swallowed={nm}", self.current_line_no());
                }
                continue;
            }
            // 组定界：{ } 与 \begingroup/\endgroup（TeX macro_def 模式组定界）。
            // unbalance 语义（tex.web L9394 `unbalance:=1`）：体扫描开始时已有
            // 一个未配平组；`}` 使 unbalance 归 0 即体结束。skip 区的组定界
            // token 已在上方 is_skipping 臂消费（不入体、**同样维护配平**——
            // tex.web 的 pass_text 在 get_token 层之下，此处对齐手段是把
            // skip 吞入也计入 unbalance，否则空组惯用法 `{ \if_false: } \fi:`
            // 的 `}` 丢失配平 → 体吞掉后续全部源码）。
            match tok.catcode() {
                Some(Catcode::EndGroup) => {
                    unbalance -= 1;
                    if unbalance == 0 {
                        break 'scan; // 外层 }：宏体结束（不收入体）
                    }
                    out.push(tok);
                    continue;
                }
                Some(Catcode::BeginGroup) => {
                    unbalance += 1;
                    out.push(tok);
                    continue;
                }
                _ => {}
            }
            let Some(csid) = tok.csid() else {
                // 字符 token：仅 \edef/\xdef 体（macro_def 模式）做参数 # 处理；
                // \expanded 实参（macro_def=false）字面 # 原样收集（见函数头注释）
                if in_definition && is_parameter_char(tok) {
                    let next = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("替换文本中 # 后无 token"))?
                        .0;
                    if let Some(d) = digit_value(next) {
                        out.push(Token::macro_param(d));
                    } else if is_parameter_char(next) {
                        out.push(Token::char(Catcode::Parameter, b'#' as u32));
                    } else {
                        // 同 scan_balanced_text：tex.web L9416-9423 可恢复语义
                        self.unread(next);
                        let def = if def_name.is_empty() {
                            String::new()
                        } else {
                            format!(" \\{def_name}")
                        };
                        self.write_error_help(
                            &format!("Illegal parameter number in definition of{def}."),
                            "You meant to type ## instead of #, right?\n\
                             Or maybe a } was forgotten somewhere earlier, and things\n\
                             are all screwed up? I'm going to assume that you meant ##.\n",
                        );
                        out.push(tok);
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
                    self.push_frame(InputFrame::TokenList {
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
                    self.push_frame(InputFrame::TokenList {
                        items: Arc::from(expansion),
                        pos: 0,
                    });
                }
                // 第二十刀（l.9114 hook 名泄露根因）：`\begingroup`/`\endgroup`
                // 原语及其 `\cs_new_eq:NN` 别名（expl3 `\group_begin:`/`\group_end:`）
                // 在宏体/`\expanded` 实参扫描中**原样存储，不参与 unbalance 配平、
                // 不终止扫描**。tex.web scan_toks 的体终止符只有字符 `}`（cat 2）；
                // pdftex 1.40.29 实测（/tmp/k20/e5.tex）：`\edef\a{\endgroup XXX}` →
                // `macro:->\endgroup XXX`，`\let\ge\endgroup` 别名同存储。旧实现把
                // Primitive(EndGroup) 计入配平并在 unbalance==0 时 break——`\use:e`
                // (=`\expanded`) 实参在首个 `\group_end:` 处截断，lthooks 归一化链
                // （`\__hook_normalize_hook_args_aux:Nn` 的
                // `\group_begin: \use:e { \group_end: … }` 惯用法）交付为空，余
                // token 落主循环：hook 名字符被排版（Missing character 洪水）+
                // 实参扫描失衡（「实参组未闭合」fatal，latex.ltx l.9114 停点）。
                // 字符别名 `\let\egroup=}`（etrip.tex 29-34）EqSlot 为 Char，本就
                // 落此臂原样收集。
                _ => out.push(tok),
            }
        }
        if runaway {
            let _ = self.sink.write16("Runaway definition?\n".to_owned());
            // tex.web @<Report an runaway definition...@>（scan_toks found 之后的
            // file_end 路径）：`! File ended while scanning definition of \foo.`
            // 可恢复——插 } 收尾后作业继续（pdfTeX 实测 2026-09-12）。
            let def = if def_name.is_empty() {
                String::new()
            } else {
                format!(" of \\{def_name}")
            };
            self.write_error(&format!("File ended while scanning definition{def}."));
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
            self.push_frame(InputFrame::TokenList {
                items: Arc::from(items),
                pos: 0,
            });
            // 子展开护栏：与主循环 max_steps 同源（expand_only 区域内的
            // process_one 递归不经过主循环步数检查，latex.ltx l.16900
            // \@preamble \\edef 曾在此无限循环 900s）。
            let mut region_steps: u64 = 0;
            // 第十八刀（三）：区域展开的每轮都是完整 dispatch（fetch →
            // process_token）。它会在 handler 内反复交还 token，因而不同于
            // fetch 内自旋，必须在此另计；否则主循环 steps 停住而护栏失明。
            let bc_guard_limit = bytecode_guard_limit();
            let depth_floor = depth + 8; // 正常 \\edef 嵌套极浅；+8 为嵌套 \\edef 余量
            while self.process_one()? {
                region_steps += 1;
                self.region_steps = self.region_steps.saturating_add(1);
                if bc_guard_limit != 0 && region_steps > bc_guard_limit {
                    self.dump_bytecode_guard(region_steps, bc_guard_limit);
                    return Err(Error::invalid_input(format!(
                        "字节码区域 dispatch 超限（{region_steps} 步；NTEX_BC_GUARD={bc_guard_limit}）"
                    )));
                }
                if region_steps > max_steps() || self.stack.len() > depth_floor + 4096 {
                    return Err(Error::invalid_input(format!(
                        "区域展开步数/栈深超限（\\edef/\\write 内疑似死循环）；步 {} 栈深 {}（入口 {}）",
                        region_steps,
                        self.stack.len(),
                        depth
                    )));
                }
            }
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
        // tex.web prefixed_command 的 define 分支经 scan_optional_equals 前先取
        // 左侧控制序列；非控制序列不是引擎致命错误，而是 "Missing control
        // sequence inserted" 后以 \inaccessible 续跑。复用定义/寄存器绑定的
        // scan_cs_ident，保证错误恢复会回推被拒 token（真实 latex.ltx 的错误
        // 恢复链会走到这一分支）。
        let csid = self.scan_cs_ident()?;

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
        if diag_enabled("NTEX_IFX_TRACE") {
            let n = |t: Token| match t.csid() {
                Some(id) => self.intern.name(id).to_owned(),
                None => format!("{t:?}"),
            };
            let nm = self.intern.name(csid);
            if nm.contains("replacement_spec") {
                eprintln!("[trace-let] line={} \\{} = {}", self.current_line_no(), nm, n(rhs));
            }
        }
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
                    prev_level: self.eqtb.level(csid),
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
        self.eq_mark_level(csid, global);
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
        // 与 \let/\def 一致：左侧不是控制序列时，TeX 插入 \inaccessible
        // 并回推原 token 继续恢复；不能把格式加载中的可恢复错误升级为 Rust Err。
        let csid = self.scan_cs_ident()?;
        let t1 = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\futurelet 后缺少 token"))?
            .0;
        let t2 = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\futurelet 后缺少被观察 token"))?
            .0;
        // 作用域与 \let 同（tex.web prefix 循环后同一 let 赋值路径）：组内局部
        // 保存 + 层级登记——futurelet 不登记会让"局部值 + 全局层级"破坏
        // unsave 的 retain 守卫（expl3 \@ifnextchar 高频使用）。
        let global = self.is_global();
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
        self.let_to(csid, t2);
        self.eq_mark_level(csid, global);
        let items: Vec<(Token, bool)> = vec![(t1, false), (t2, false)];
        self.push_frame(InputFrame::TokenList {
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

/// tex.web macro_call `@<Tidy up the parameter just scanned, and tuck it away@>`：
/// 整个定界实参**恰为一个组**时剥去外层花括号。
///
/// tex.web 的判定是 `(m=1) and (info(p)<right_brace_limit)`——`m` 计主循环
/// 顶层"贡献单元"数（单 token 记 1，整组也只记 1，定界符前缀逐 token 记），
/// `p` 是首存 token；`rbrace_ptr:=p` 后 `link(rbrace_ptr):=null` 丢弃配对
/// `}`、再释放首 `{`。等价于：buf 非空、首 token 是 `{`、其配对 `}` 恰为
/// buf 末 token（无其他顶层 token 混入 ⇔ m=1）。
///
/// 注意与无分隔实参的差异：无分隔实参 `m` 恒为 1，故**组实参必剥组**；
/// 定界实参只有整体恰为单组才剥——`\def\a#1!` 下 `\a{x}!` 得 `x`，
/// `\a{x}y!` 得 `{x}y`，`\a{{x}}!` 得 `{x}`（tex.web 只剥一层）。
/// 仅正常 found 路径剥：runaway / extra-} / Paragraph ended 恢复路径在
/// tex.web 里直接 `pstack[n]:=link(temp_head)`，保留原样。
fn strip_single_group(buf: Vec<Token>) -> Vec<Token> {
    if buf.len() < 2 || buf[0].catcode() != Some(Catcode::BeginGroup) {
        return buf;
    }
    let mut depth = 0usize;
    let mut close = None;
    for (i, t) in buf.iter().enumerate() {
        match t.catcode() {
            Some(Catcode::BeginGroup) => depth += 1,
            Some(Catcode::EndGroup) => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    match close {
        Some(i) if i == buf.len() - 1 => Vec::from(&buf[1..i]),
        _ => buf,
    }
}
