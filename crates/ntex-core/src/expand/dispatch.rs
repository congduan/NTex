use super::*;

impl Expander {
    /// 单步处理一个 token；返回 false 表示输入耗尽。
    pub(super) fn process_one(&mut self) -> Result<bool> {
        if bytecode_guard_limit() == 0 {
            return self.process_one_inner();
        }
        // handler 可经 `expand_region` 嵌套调用 process_one；内层必须沿用外层
        // dispatch 的计数，不能重置，否则恰好会漏掉 `\edef` 中的活锁。
        let outer_dispatch = !self.bc_guard_active;
        if outer_dispatch {
            self.bc_guard_active = true;
            self.bc_guard_dispatches = 0;
        }
        let result = self.process_one_inner();
        if outer_dispatch {
            self.bc_guard_active = false;
        }
        result
    }

    /// `process_one` 的实际 dispatcher；护栏 wrapper 使 handler 内的 fetch 也纳入同一计数。
    fn process_one_inner(&mut self) -> Result<bool> {
        match self.fetch()? {
            None if std::mem::take(&mut self.output_boundary_yield) => Ok(true),
            None => Ok(false),
            Some((tok, noexpand)) => {
                // 诊断：记录最近处理的 token（watchdog/单步超时 dump 用；cs 显示真实名字）
                self.last_tok = Some(match tok.csid() {
                    Some(csid) => format!("\\{}", self.intern.name(csid)),
                    None => format!("{tok:?}"),
                });
                // 第十八刀（四）：控制序列真正进入 dispatcher 的现场。`fetch` 内的
                // 实参/定义扫描会吞掉大量 token，却不会再次经过这里；故护栏转储的
                // 尾项就是仍占着 handler、尚未向主循环交还 token 的原语/宏。
                let read_floor = self.read_floor;
                if let (Some(csid), Some(trace)) = (tok.csid(), self.handler_trace.as_mut()) {
                    if trace.len() == BC_GUARD_TRACE_CAP {
                        trace.pop_front();
                    }
                    trace.push_back((csid, read_floor));
                }
                if bytecode_guard_limit() != 0 {
                    if let Some(csid) = tok.csid() {
                        if self.bc_guard_trace.len() == BC_GUARD_TRACE_CAP {
                            self.bc_guard_trace.pop_front();
                        }
                        self.bc_guard_trace.push_back(csid);
                    }
                }
                // M4-5 对齐状态机拦截（tex.web §749-823；详见 align.rs）：
                // preamble 阶段分类收集模板 token；body raw 阶段拦 `&`/`\span`/
                // `\cr`/`\crcr`（Insert v_j）与 `}` 平衡（对齐组闭括号）。
                if !noexpand && self.align_on_token(tok)? {
                    return Ok(true);
                }
                // \tracingcommands（misc 下标 3）：每命令一行 `{模式: 描述}`；
                // 模式只在变化时打印（tex.web show_cur_cmd_chr 的 shown_mode 语义）；
                // 子展开（expand_region：\write 内容等）抑制——TeX 只在主循环追踪。
                // \moveleft/\moveright 的 box 参数（tex.web scan_box）同样抑制：
                // 模式变化 = 盒子组内容开始 → 立即恢复；非组盒子原语（\copy 等）
                // 本次仍抑制、执行后恢复（trace_suppress_defer）。
                if self.pending_box_arg {
                    if self.sink.mode_code() != self.pending_box_arg_mode {
                        self.trace_suppress -= 1;
                        self.pending_box_arg = false;
                    } else if let Some(csid) = tok.csid() {
                        if let EqSlot::Primitive(p) = self.eqtb.slot(csid) {
                            if matches!(
                                p,
                                Primitive::Copy
                                    | Primitive::Box
                                    | Primitive::UnHBox
                                    | Primitive::UnHCopy
                                    | Primitive::UnVBox
                                    | Primitive::UnVCopy
                                    | Primitive::LastBox
                                    | Primitive::VSplit
                            ) {
                                self.pending_box_arg = false;
                                self.trace_suppress_defer = true;
                                // tex.web：`\setbox0=\box1` 无组盒子实参在 box_end
                                // 后回到 prefixed_command `done:` 触发 `\afterassignment`
                                // （组版走 begin_group 的同款触发位）。
                                self.finish_assignment();
                            }
                        }
                    }
                }
                // tex.web pass_text：跳过区（\else/\or 后）的 token 不追踪
                // （show_cur_cmd_chr 只在 main_control/get_x_token 的处理路径）——
                // 仅 \fi（跳过区结束标记）保留追踪（参考 log {\fi} 存在）。
                let skipping = self
                    .cond_stack
                    .last()
                    .map(|c| c.state == CondState::Skipping)
                    .unwrap_or(false);
                let is_fi = matches!(tok.kind(), TokenKind::ControlSeq)
                    && matches!(
                        self.eqtb.slot(tok.csid().expect("ControlSeq 必有 csid")),
                        EqSlot::Primitive(Primitive::Fi)
                    );
                if self.params.misc[3] > 0 && self.trace_suppress == 0 && (!skipping || is_fi) {
                    self.trace_token_now(tok);
                }
                // noexpand（`\noexpand`/`\unexpanded` 输出）：临时不可展开，原样输出。
                // 优先于条件机拦截——`\unexpanded{\ifx...}` 里的条件 token 是数据，
                // 不得 push 条件帧，也不得匹配外层 `\else`/`\fi`。
                if noexpand {
                    self.sink.token(tok)?;
                    return Ok(true);
                }
                // 条件 token（\if*/\\else/\\fi/\\or）优先由条件机处理（无论是否跳过）
                if let Some(op) = self.cond_op(tok) {
                    self.step_conditional(op, tok)?;
                    return Ok(true);
                }
                if self.is_skipping() {
                    // 跳过模式：其余 token 直接丢弃（不展开）。
                    // tex.web get_next 的 forbidden 检查（TRIP L363）：skip 区
                    // 出现 **outer 宏** → `Incomplete \if...; all text was
                    // ignored after line N.` + 插入 \fi 恢复。此前惰性 Skipping
                    // 帧的主循环丢弃无此检查（\ifcase 真分支 skip_ahead 之外的
                    // 另一条 skip 路径），pdfTeX 报 Incomplete 而 NTex 静默。
                    if tok.csid().is_some() && self.is_outer_for_token(tok) {
                        let name = self.intern.name(tok.csid().expect("已判 csid")).to_owned();
                        let ifname = Self::if_type_name(self.cur_if_type).to_owned();
                        let ln = self.error_line_no();
                        let _ = self.sink.write16(format!(
                                    "! Incomplete {ifname}; all text was ignored after line {ln}.\n\
                                     <inserted text>\n                \\fi \n\
                                     <to be read again>\n                   \\{name}\n\
                                     A forbidden control sequence occurred in skipped text.\n\
                                     This kind of error happens when you say `\\if...' and forget\n\
                                     the matching `\\fi'. I've inserted a `\\fi'; this might work.\n"
                                ));
                        // 插入 \fi 闭合全部未决条件帧（对齐 skip_ahead
                        // 的 outer 恢复臂语义）。offending token **放回**
                        // （tex.web back_input，"<to be read again>"）——
                        // skip 结束后它按常规语境执行；preamble 现场它被
                        // 再读时走 aligning 的 Forbidden 恢复（align.rs）。
                        self.cond_stack.clear();
                        self.cur_if_type = 0;
                        self.cur_if_branch = 0;
                        self.unread(tok);
                        return Ok(true);
                    }
                    return Ok(true);
                }
                // list 机制刀：`\everypar` 触发链（tex.web `new_graf`）。
                // LaTeX 的 `\list`→`\item` 依赖段首触发 `\everypar` 清 `\@newlist`；
                // 引擎此前从不触发，`\@trivlist` 逐条报 "missing \item"。
                // tex.web 顺序（`vmode+letter…` 臂）：`back_input; new_graf(true)`
                // ——先开段、注入 `\everypar`、再回放触发 token。不可回放触发
                // token 的 `\indent`/`\noindent`（`start_par` 臂直接消费）。
                // 门禁 `\everypar` 非空：plain/TRIP 从不设它 → 行为逐字节不变。
                if !self.everypar_toks.is_empty() && self.sink.par_begin_imminent() {
                    if let Some((indented, back_input)) = self.par_trigger_kind(tok) {
                        self.sink.par_begin(indented)?;
                        if back_input {
                            self.unread(tok);
                        }
                        // 后压 = 先展开：`\everypar` 在触发 token 之前落列表
                        // （LaTeX 段落钩子机器 `\box_gset_to_last` 取缩进盒时，
                        // 水平列表里必须只有缩进盒）。
                        let items: Vec<(Token, bool)> =
                            self.everypar_toks.iter().map(|t| (*t, false)).collect();
                        self.push_frame(InputFrame::TokenList {
                            items: Arc::from(items),
                            pos: 0,
                        });
                        return Ok(true);
                    }
                }
                self.process_token(tok)?;
                // 非组盒子原语（\copy 等）作为 \moveleft 参数：执行完恢复追踪
                if self.trace_suppress_defer {
                    self.trace_suppress -= 1;
                    self.trace_suppress_defer = false;
                }
                Ok(true)
            }
        }
    }

    /// 当前 token 是否会开段（tex.web `vmode` 水平材料触发集的子集）：
    /// 返回 `(indented, back_input)`——字母/其他字符开段并把触发 token 压回
    /// 输入（`back_input` 臂）；`\indent`/`\noindent`（`start_par` 臂）开段并
    /// **直接消费**，不回放（否则水平模式 `\indent` 再落一个缩进盒）。
    /// `\let` 别名经 eqtb 槽判定，同样命中。
    ///
    /// `vmode+un_hbox` 臂（tex.web L21104-21111 `back_input; new_graf(true)`）：
    /// `\leavevmode`（=`\unhbox\voidb@x`）赖此进入水平模式——LaTeX `\@tabular`
    /// 的 `\leavevmode\hbox\bgroup`、`\@maketitle` 作者块的 tabular 居中都踩
    /// 这里；此前垂直模式 `\unhbox` 直接拆包，盒落进竖列表 → 永不居中。
    fn par_trigger_kind(&self, tok: Token) -> Option<(bool, bool)> {
        match tok.kind() {
            TokenKind::Char => match tok.catcode() {
                Some(Catcode::Letter) | Some(Catcode::Other) => Some((true, true)),
                _ => None,
            },
            TokenKind::ControlSeq => {
                let csid = tok.csid()?;
                match self.eqtb.slot(csid) {
                    EqSlot::Primitive(Primitive::Indent) => Some((true, false)),
                    EqSlot::Primitive(Primitive::NoIndent) => Some((false, false)),
                    EqSlot::Primitive(Primitive::UnHBox)
                    | EqSlot::Primitive(Primitive::UnHCopy) => Some((true, true)),
                    EqSlot::Primitive(
                        Primitive::HSkip
                        | Primitive::HFil
                        | Primitive::HFill
                        | Primitive::HSS
                        | Primitive::HFilNeg,
                    ) => Some((true, true)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// 处理单个 token（展开宏/原语，其余输出）。
    pub(super) fn process_token(&mut self, tok: Token) -> Result<()> {
        // 注意：此处不加 `ended` 拦截——`\end` 收尾时要 flush 延迟写流
        // （`flush_writes` → `expand_to_string` → 本函数），拦截会丢 write
        // 内容。终结由 `process_one` 顶部检查保证。
        self.cur_cs = tok.csid();
        if self.expand_only {
            return self.process_expand_only(tok);
        }
        match tok.kind() {
            TokenKind::ControlSeq => {
                let csid = tok.csid().expect("ControlSeq 必有 csid");
                let action = match self.eqtb.slot(csid) {
                    // M1-13 错误恢复（ETRIP）：未定义 cs 报 "! Undefined control
                    // sequence." 到转录并**当 \relax 继续**（TeX 错误恢复；上下文行
                    // "l.N …" 留 M1-13 后续细化）。
                    EqSlot::Undefined => SlotAction::Undefined(self.intern.name(csid).to_owned()),
                    EqSlot::Alias(target) => SlotAction::Alias(*target),
                    EqSlot::Char { catcode, charcode } => SlotAction::Char {
                        catcode: *catcode,
                        charcode: *charcode,
                    },
                    // \countdef\cs 等绑定的寄存器 cs：执行位置为赋值（`\cs=<值>`，
                    // TeX 中 `=` 可选）；非赋值上下文（\the/\advance/\ifnum 等）由
                    // 各扫描函数处理，不会到达此处。
                    EqSlot::Register(kind, idx) => SlotAction::Register(*kind, *idx),
                    EqSlot::Stream(..) => SlotAction::Stream,
                    EqSlot::MathChar(code) => SlotAction::MathChar(*code),
                    EqSlot::Macro(m) => SlotAction::Macro(m.value.clone()),
                    EqSlot::Font(font) => SlotAction::Font(*font),
                    EqSlot::Primitive(p) => SlotAction::Primitive(*p),
                };
                match action {
                    SlotAction::Undefined(name) => {
                        // A3：TeX 错误格式 `! 消息` + 上下文行 `l.N <行内容>`；
                        // 未定义 cs 当 \relax 继续（TeX 错误恢复）。
                        let mut msg = format!("! Undefined control sequence.\n\\{name}\n");
                        if let Some((n, line)) = self.error_context() {
                            msg.push_str(&format!("l.{n} {line}\n"));
                        }
                        // 锚点一次性使用（本出口不经 report_error_context；残留
                        // 锚点会把后续所有 `l.N` 上下文拉回旧行）
                        self.error_anchor = None;
                        let _ = self.sink.write16(msg);
                        Ok(())
                    }
                    SlotAction::Alias(target) => {
                        self.process_token(Token::control_sequence(target))
                    }
                    // \let\cs=<字符>：等价于该字符（\bgroup/\egroup 等组定界也生效）
                    SlotAction::Char { catcode, charcode } => {
                        let c = Token::char(catcode, charcode);
                        match catcode {
                            Catcode::BeginGroup => self.begin_group(),
                            Catcode::EndGroup => {
                                // `}` 字符闭合（tex.web handle_right_brace）：
                                // 关组 + noalign 组恢复（裁决键=分派时的组级）
                                self.close_group_and_resume_align()
                            }
                            Catcode::AlignmentTab => {
                                self.misplaced_alignment_tab();
                                Ok(())
                            }
                            _ => self.sink.token(c),
                        }
                    }
                    SlotAction::Register(kind, idx) => {
                        self.skip_spaces()?;
                        self.expect_equals()?;
                        match kind {
                            RegKind::Count => {
                                let val = self.scan_number()?;
                                self.assign_count(idx, val);
                            }
                            RegKind::Dimen => {
                                let val = self.scan_dimen()?;
                                self.assign_dimen(idx, val);
                            }
                            RegKind::Skip => {
                                let val = self.scan_glue()?;
                                self.assign_skip(idx, val);
                            }
                            RegKind::Muskip => {
                                let val = self.scan_glue_mu()?;
                                self.assign_muskip(idx, val);
                            }
                            RegKind::Toks => {
                                // RHS 可为 `{token list}` 或另一 toks 寄存器（内容复制，
                                // TRIP L418 `\tokens\toks1`）——统一走 scan_toks_rhs；
                                // 旧的 scan_register_index 路径会把 `\toks1` 的 `\toks`
                                // 原语误判为数字（Missing number）后再丢回主循环。
                                let val = self.scan_toks_rhs()?;
                                self.assign_toks(idx, val);
                            }
                        }
                        Ok(())
                    }
                    SlotAction::Stream => Err(Error::invalid_input(
                        "流引用不能直接使用（需在 \\read/\\write 等扫描上下文中）",
                    )),
                    // ETRIP 冲刺：\mathchardef\cs=<num> 绑定的 cs 执行时推送
                    // 完整数学字符原子（tex.web main_control `math_given`：
                    // class<<12 | fam<<8 | char 全量进 mlist——demo 差异 #3：
                    // \sum="1350 的 fam3（cmex10 大算符）此前被丢成 fam0 文本
                    // 字符）。数学模式外退回 \char 文本行为（TeX 语义应为
                    // "Missing $ inserted" 报错，ETRIP 后续对齐）。
                    SlotAction::MathChar(code) => {
                        if self.sink.math_code_applies()
                            && (self.in_math || self.sink.math_shift_will_close())
                        {
                            self.sink.math_char_full(code)
                        } else {
                            self.sink.token(Token::char(Catcode::Other, code & 0xFF))
                        }
                    }
                    SlotAction::Macro(def) => {
                        // tex.web `check_outer_validity`（L7149-7170）——**唯一**判据是
                        // `scanner_status <> normal`：
                        //
                        //   begin if scanner_status<>normal then
                        //     begin deletions_allowed:=false;
                        //     @<Back up an outer control sequence so that it can be reread@>;
                        //     if scanner_status>skipping then
                        //       @<Tell the user what has run away and try to recover@>
                        //     else  begin print_err("Incomplete "); ... end;
                        //     deletions_allowed:=true;
                        //     end;
                        //   end;
                        //
                        // ⚠ 旧实现用「输入栈含 Macro 帧」当代理判据——**错的**：
                        // 栈含 Macro 帧 ≠ `scanner_status<>normal`。宏实参扫描用
                        // `get_token`（L7714 取 token 前临时置 `normal`），此时栈里
                        // 明明有 Macro 帧但 `scanner_status=normal`，pdfTeX 不报错。
                        // 实证：plain `^^L` 为 active char + `\outer\def^^L{\par}`，
                        // expl3-code.tex L9320 `\char_set_catcode_active:N \^^L`
                        // 取实参即触发误报 → `Extra \or` 224 条（43.8%）。
                        // （2026-09-13 再定性：该误报的真正机制是 active char 槽
                        // 与同名单字符 cs 槽共享——见 `MacroDef::active_slot`，
                        // 判据改为「token 形式 ↔ 槽形式一致」。）
                        if def.outer
                            && (tok.is_active() == def.active_slot)
                            && self.scanner_status != ScannerStatus::Normal
                        {
                            // active char token 不报（tex.web：active char 的
                            // cur_cmd=active 走 active 臂；plain \outer\def^^L
                            // 的槽残留 outer 不影响 ^^L active 使用——expl3
                            // L9320-9321 依赖此语义重定义 ^^L）
                            if std::env::var_os("NTEX_OUTER_SITE").is_some() {
                                eprintln!(
                                    "[OUTER-EXPAND] cs={} status={:?}",
                                    self.intern.name(csid),
                                    self.scanner_status
                                );
                            }
                            let name = match self.warning_index {
                                Some(cs) => self.cs_display_name(cs),
                                None => self.cs_display_name(csid),
                            };
                            // `scanner_status > skipping` → Runaway/Forbidden 恢复分支；
                            // `= skipping` → Incomplete \if 分支（L7157-7166）。
                            if self.scanner_status > ScannerStatus::Skipping {
                                match self.scanner_status {
                                    ScannerStatus::Matching => {
                                        let _ = self.sink.write16(format!(
                                            "! Forbidden control sequence found while scanning use of {name}.\\n\\
                                             <inserted text> \\n                \\\\par \\n\\
                                             I suspect you have forgotten a `}}', causing me\\n\\
                                             to read past where you wanted me to stop.\\n\\
                                             I'll try to recover; but if the error is serious,\\n\\
                                             you'd better type `E' or `X' now and fix your file.\\n"
                                        ));
                                    }
                                    ScannerStatus::Absorbing => {
                                        let _ = self.sink.write16(format!(
                                            "! Forbidden control sequence found while scanning text of {name}.\\n\\
                                             <inserted text> \\n                }} \\n\\
                                             I suspect you have forgotten a `}}', causing me\\n\\
                                             to read past where you wanted me to stop.\\n\\
                                             I'll try to recover; but if the error is serious,\\n\\
                                             you'd better type `E' or `X' now and fix your file.\\n"
                                        ));
                                    }
                                    _ => {
                                        // Defining：tex.web L6617 case 只有
                                        // defining/alignment 的措辞，其余同 Forbidden。
                                        let _ = self.sink.write16(format!(
                                            "! Forbidden control sequence found while scanning definition of {name}.\\n"
                                        ));
                                    }
                                }
                            } else {
                                let _ = self.sink.write16(
                                    "! Incomplete \\\\if; all text was ignored after line.\\n\\
                                     A forbidden control sequence occurred in skipped text.\\n"
                                        .to_string(),
                                );
                            }
                            return Ok(());
                        }
                        // e-TeX（M4-5）：protected 宏在展开抑制上下文（\edef/\write 等）
                        // 不展开，原样输出。
                        if def.protected && self.suppress_expansion > 0 {
                            self.sink.token(Token::control_sequence(csid))?;
                            return Ok(());
                        }
                        self.call_macro(csid, def)
                    }
                    SlotAction::Font(font) => {
                        // tex.web：字体 cs 执行即 `define(cur_font_loc,…)` +
                        // 推 sink（排版器靠该事件切换当前字体）。cur_font 是
                        // eqtb 字，**随组保存/恢复**（组内换字体只在本组生效；
                        // em/ex 内部单位读它，泄漏会让组外 `\kern-.36em` 等
                        // 用错字体算——`\LaTeX` 徽标现场）。
                        if !self.is_global() && self.group_level > 0 {
                            self.save_stack.push((
                                self.group_level,
                                SavedValue::CurFont {
                                    prev: self.cur_font,
                                },
                            ));
                        }
                        self.cur_font = font;
                        self.sink.font_selected(font)
                    }
                    SlotAction::Primitive(p) => {
                        // 诊断（NTEX_TRACE_EXEC=1）：打印每个执行的原语，定位挂死点
                        if diag_enabled("NTEX_TRACE_EXEC") {
                            eprintln!("[trace-exec] {p:?}");
                        }
                        if diag_enabled("NTEX_TRACE_STACK") {
                            eprintln!(
                                "[trace-stack] {p:?} last_tok={:?} stack={}",
                                self.last_tok,
                                self.debug_stack_summary()
                            );
                        }
                        self.exec_primitive(p)
                    }
                }
            }
            TokenKind::Char => {
                // 数学移位（$，cat 3）：peek 下一个 token 判定 `$$`（显示数学），
                // 交给 sink 按自身模式决定进出（M4-1）。
                if tok.catcode() == Some(Catcode::MathShift) {
                    // tex.web dispatches math_shift by the current mode. An
                    // enclosing display can still be open while a vbox is in
                    // internal vertical mode; its group must survive this `$`.
                    if matches!(self.sink.mode_code(), 1 | 4) {
                        // vmode+math_shift: back_input; new_graf(true). The
                        // same token is then handled by hmode+math_shift.
                        self.sink.par_begin(true)?;
                        // Pure expansion sinks do not implement paragraph
                        // mode. In that case process the shift here instead
                        // of replaying it forever in vertical mode.
                        if !matches!(self.sink.mode_code(), 1 | 4) {
                            self.unread(tok);
                            if !self.everypar_toks.is_empty() {
                                let items = self
                                    .everypar_toks
                                    .iter()
                                    .map(|t| (*t, false))
                                    .collect::<Vec<_>>();
                                self.push_frame(InputFrame::TokenList {
                                    items: Arc::from(items),
                                    pos: 0,
                                });
                            }
                            return Ok(());
                        }
                    }
                    // tex.web init_math 的进出裁决在 **mode**（mmode=收），
                    // 不在旗标——核心 in_math 在 `\[\halign{…$x$…}\]` 一类
                    // 结构里与排版层脱钩（显示数学开过即恒 true，单元首 `$`
                    // 被误判为收，everymath 注入错位到第二个 `$` 之后）。
                    let entering = !self.sink.math_shift_will_close();
                    if !entering && self.scope_stack.last() != Some(&true) {
                        // A closing shift in actual math mode cannot cross an
                        // unfinished ordinary group. A box enclosing an inner
                        // display is in vertical mode here and never enters
                        // this branch.
                        self.recover_groups_to_math_scope()?;
                    }
                    // tex.web init_math：`$$` 进显示数学要求 `mode>0`（垂直/
                    // 普通水平）；受限水平（\hbox/\halign 模板，mode<0）下
                    // peek 到的第二个 `$` 必须 back_input——本 `$` 单独进普通
                    // 数学，第二个 `$` 随后在数学模式下作为独立 token 一记
                    // 闭合（TRIP L210 `\hbox{$$}$`、L340 单元内 `$$`：参考日志
                    // 两行 `{math mode: math shift character $}` + restoring，
                    // 且数学必须关上——否则对齐/数学组悬挂到文档尾致命）。
                    // 数学内（entering=false）是**闭合**语义，与进入侧不对称：
                    // tex.web mmode+math_shift → after_math——显示数学
                    // （mode=+mmode）收尾必 "Check that another $ follows"
                    // （吃掉配对 `$`，否则报 "Display math should end with $$"
                    // 照收）；行内数学（mode=-mmode）走 Finish math in text，
                    // **不 peek**——紧随的 `$` 落回水平模式由 init_math 重新
                    // 判定（`$x$$y$` = `$x$`+`$y$`，real TeX 如此）。360c342
                    // 把闭合侧一律拦成"只 peek 不消费"，`$$x$$` 的第二个 `$`
                    // 被 back_input 后在非数学态重开一个永不闭合的行内公式 →
                    // "数学模式未闭合"。
                    let consume_for_display = if entering {
                        self.sink.math_display_allowed()
                    } else {
                        self.sink.math_close_consumes_dollar()
                    };
                    if !entering && self.sink.mode_code() == 6 {
                        // finish_display restores the enclosing mode before
                        // tex.web's get_x_token check for the second `$`.
                        // A conditional here must observe that restored mode.
                        self.in_math = false;
                        let r = self.sink.math_shift(true);
                        self.in_math = self.sink.math_shift_will_close();
                        let matched = self.next_is_math_shift(true, true)?;
                        if !matched {
                            self.write_error("Display math should end with $$.");
                        }
                        self.close_math_scope();
                        if self.scope_stack.last() == Some(&true) {
                            self.close_math_scopes_if_sink_out()?;
                        }
                        return r;
                    }
                    let display = self.next_is_math_shift(consume_for_display, !entering)?;
                    self.in_math = entering;
                    // tex.web init_math `@<Go into display math mode@>`（L21774）：
                    // `eq_word_define(dimen_base+display_width_code,l)`，其中
                    // `l:=hsize`（「Calculate the length l and the shift s」的
                    // 悬挂缩进摊派臂未实现 → l 恒 = hsize、s = 0）。这是**全局**
                    // 写（不进 save_stack），`$$` 体内宏读 `\displaywidth` 即得行宽。
                    // 此前该量是"只读内部量、恒按 0 读"，LaTeX `\displaylines`
                    // 与 amsmath `multline`/`gather` 的 `\hbox to\displaywidth`
                    // 因此全部排成 0 宽行盒（公式溢出/对齐全错）。
                    let enters_display = display
                        && if entering {
                            self.sink.math_display_allowed()
                        } else {
                            // 行内数学内遇 `$$`：sink 报错后转进显示数学；
                            // 已在 DisplayMath 是**收尾**，不重设。
                            self.sink.mode_code() == 3
                        };
                    if enters_display {
                        let l = self.params.hsize;
                        // The VM's displaywidth is global, but the layout mirror
                        // can have been restored by a surrounding group. Re-send
                        // the value on every display entry, even if the VM value
                        // already equals hsize (tex.web init_math eq_word_define).
                        self.params.displaywidth = l;
                        self.sink
                            .param_changed(ParamKind::DisplayWidth, ParamValue::Dimen(l))?;
                    }
                    // tex.web init_math：`$` 即 `new_save_level(math_shift_group)`
                    // ——数学层是 save group，先入作用域栈，`\everymath` 体里的
                    // `\aftergroup` 才挂到本层（而非外包盒子/对齐组）。
                    if entering {
                        self.scope_stack.push(true);
                        self.scope_silent.push(false);
                    }
                    // tex.web init_math: inline math runs \everymath, while
                    // display math runs \everydisplay. AMS uses the latter to
                    // set \if@display before the first token of an equation.
                    //
                    // LaTeX fmt 兼容：发行快照可能来自修复前引擎，`\let\frozen@everymath
                    // \everymath` 没有保住 primitive toks 参数，导致 LaTeX 写入 primitive
                    // `\everymath` 的 `\check@mathfonts` 钩子为空。若当前快照已有
                    // LaTeX 的 `\check@mathfonts` 且 primitive everymath 为空，补注入该
                    // 钩子，避免 NFSS 数学尺寸宏 `\tf@size/\sf@size/\ssf@size` 未初始化。
                    if entering {
                        // tex.web init_math（L21721 普通数学 / L21773 显示数学两路）：
                        // 进入公式即 eq_word_define(cur_fam, -1)——每个公式开头
                        // `\fam` 都是 -1（无字族替换），显式 `\fam<n>` 只在公式内生效
                        self.params.misc[46] = -1;
                        let hook = if enters_display {
                            &self.everydisplay_toks
                        } else {
                            &self.everymath
                        };
                        let mut items = hook.iter().map(|t| (*t, false)).collect::<Vec<_>>();
                        if items.is_empty() && !enters_display {
                            if let Some(csid) = self.intern.lookup("check@mathfonts") {
                                if !matches!(self.eqtb.slot(csid), EqSlot::Undefined) {
                                    items.push((Token::control_sequence(csid), false));
                                }
                            }
                        }
                        if !items.is_empty() {
                            self.push_frame(InputFrame::TokenList {
                                items: Arc::from(items),
                                pos: 0,
                            });
                        }
                    }
                    if entering {
                        return self.sink.math_shift(display);
                    }
                    // tex.web after_math → unsave：数学作用域先关，其
                    // `\aftergroup` token 随后在外层模式继续处理。
                    let r = self.sink.math_shift(display);
                    // Closing an inline formula inside a box can restore an
                    // enclosing display. Follow the actual TeX mode.
                    self.in_math = self.sink.math_shift_will_close();
                    self.close_math_scope();
                    // A non-math group above an older math scope belongs to
                    // that outer formula (e.g. a vtop in display math).
                    if self.scope_stack.last() == Some(&true) {
                        self.close_math_scopes_if_sink_out()?;
                    }
                    return r;
                }
                // 数学模式普通字符（letter/other，cat 11/12）→ 查 \mathcode 表
                // 改道为完整数学字符原子（tex.web 主控制数学分支：普通字符=
                // 隐式 mathcode 查表；class<<12 | fam<<8 | char）。demo 差异 #1：
                // $E=mc^2$ 字母默认 fam1（cmmi 斜体，initex 表 0x7100+码），
                // 此前布局侧硬编码 fam0（cmr 正体）。`^`/`_`（cat 7/8）、空格
                // （cat 10）、组定界（cat 1/2）不走此路。
                if (self.in_math || self.sink.math_shift_will_close())
                    && self.sink.math_code_applies()
                    && matches!(tok.catcode(), Some(Catcode::Letter | Catcode::Other))
                {
                    let ch = tok.charcode().unwrap_or(0);
                    let mut code = self.mathcodes.get(&ch).copied().unwrap_or(0x8000);
                    // tex.web scan_math（L21906-21908）：class ≥ var_code(0x7000)
                    // 且 cur_fam ∈ 0..15 时 **无条件替换** fam 字段——`\rm`/`\it`
                    // （plain `\def\rm{\fam\z@\tenrm}`）正是靠这一条把字母改道到
                    // 正体/斜体字族。数学进入时 cur_fam 复位 -1（L21721），故只在
                    // 显式 `\fam<n>` 后生效；\mathcode 自带的 fam 字段被抹掉。
                    if code >= 0x7000 {
                        code = self.mathcode_apply_fam(code);
                    }
                    return self.sink.math_char_full(code);
                }
                if tok.catcode() == Some(Catcode::AlignmentTab) {
                    self.misplaced_alignment_tab();
                    return Ok(());
                }
                if std::env::var_os("NTEX_HOOK_TRACE").is_some() {
                    if let Some(ch) = tok.charcode().and_then(char::from_u32) {
                        if matches!(ch, 'p' | 'a' | 'r' | '/' | ',' | '0') {
                            let calls = self
                                .macro_trace
                                .as_ref()
                                .map(|tr| {
                                    tr.iter()
                                        .rev()
                                        .take(24)
                                        .rev()
                                        .map(|id| format!("\\{}", self.intern.name(*id)))
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                })
                                .unwrap_or_default();
                            eprintln!(
                                "[hook-char] ch={ch:?} line={} last={:?} stack={} calls={}",
                                self.error_line_no(),
                                self.last_tok,
                                self.debug_stack_summary(),
                                calls
                            );
                        }
                    }
                }
                // 组定界符（cat 1/2）在主流层建立/结束组（M1-11）。
                // 对齐上下文（`\halign`/`\valign`）的 `{`/`}` 平衡计数、
                // `\noalign` 组的建立与行边界裁决已在 align_on_token 处理；
                // 此处建立/结束普通组，`}` 关组后按 tex.web L21663 恢复
                // 悬置的 noalign 组（close_group_and_resume_align）。
                match tok.catcode() {
                    Some(Catcode::BeginGroup) => self.begin_group(),
                    Some(Catcode::EndGroup) => self.close_group_and_resume_align(),
                    _ => self.sink.token(tok),
                }
            }
            _ => self.sink.token(tok),
        }
    }
    /// 展开上下文（TeX `expand()`，`\edef`/`\xdef`/`\write`）：只展开可展开项——
    /// 宏、可展开原语（`\the`/`\expandafter`/`\noexpand`/`\number`/`\unexpanded`/
    /// `\detokenize`/`\eTeXversion`/`\eTeXrevision`）；条件由 process_one 拦截。
    /// 不可展开原语、未定义 cs、字符、组定界、宏参数一律原样保留（不执行、不建组）。
    fn process_expand_only(&mut self, tok: Token) -> Result<()> {
        // TeX expand() 语义：条件原语在展开上下文（\edef/\write/\message 参数
        // 收集）中**求值**（tex.web expand 的 if_test/if_case 分支）——`\ifcase`
        // 在 \edef 里执行、\x 收集的是选中分支文本（etrip \5 宏
        // `\edef\6{\ifcase\lastnodetype...}`：\6 = else 分支 "empty"）。
        // 此前原样保留导致 \6 含未求值条件、\typeout 展开时再遇 expand_only
        // 仍不执行 → 输出空（ETRIP L351 `last node type (l.351): ` 缺 empty）。
        if let Some(op) = self.cond_op(tok) {
            return self.step_conditional(op, tok);
        }
        // 条件跳过区（\\ifcase-1 的 \\or 段等）：token 丢弃不收集
        // （TeX expand 的 pass_text 语义；主循环 process_one 同样先查 is_skipping）
        if self.is_skipping() {
            return Ok(());
        }
        match tok.kind() {
            TokenKind::ControlSeq => {
                let csid = tok.csid().expect("ControlSeq 必有 csid");
                match self.eqtb.slot(csid).clone() {
                    EqSlot::Macro(m) => {
                        if m.value.protected && self.suppress_expansion > 0 {
                            return self.sink.token(tok);
                        }
                        // tex.web `@<Tell the user what has run away...@>`（L7184-7200）：
                        // `\edef` 展开上下文里的 outer 宏同样是**可恢复错误**
                        // （`error`，非 fatal）——`scanner_status=absorbing` 分支
                        // （L7219-7222）打印 `... while scanning text of \X` +
                        // **插入 `}`** 后继续。
                        //
                        // ⚠ 实测（2026-09-13 再定性）：`plain` 的 `^^L` 是
                        // `\outer\def^^L{\par}`（写进 tex.web active 槽），cs 形式
                        // `\^^L` 的槽是 undefined——expl3 L9320 靠这一点不报。
                        // 判据按「token 形式 ↔ 槽形式一致」（`is_outer_for_token`）。
                        if self.is_outer_for_token(tok) {
                            let name = self.intern.name(csid).to_owned();
                            let _ = self.sink.write16(format!(
                                "! Forbidden control sequence found while scanning text of \\{name}.\\n\\
                                 <inserted text> \\n                }} \\n\\
                                 I suspect you have forgotten a `}}', causing me\\n\\
                                 to read past where you wanted me to stop.\\n\\
                                 I'll try to recover; but if the error is serious,\\n\\
                                 you'd better type `E' or `X' now and fix your file.\\n"
                            ));
                            // 恢复：按空展开继续（不中断作业）
                            return Ok(());
                        }
                        self.call_macro(csid, m.value.clone())
                    }
                    EqSlot::Primitive(p) if p.is_expandable() => self.exec_primitive(p),
                    _ => self.sink.token(tok),
                }
            }
            _ => self.sink.token(tok),
        }
    }

    /// 展开宏调用：收集实参，压入字节码（M2）或宏体输入帧。
    pub(super) fn call_macro(&mut self, csid: u32, def: Arc<MacroDef>) -> Result<()> {
        // tex.web macro_call（L7968-7975）：
        //   save_scanner_status:=scanner_status; save_warning_index:=warning_index;
        //   warning_index:=cur_cs; ...
        //   @<Scan the parameters...@> = begin scanner_status:=matching; unbalance:=0; ...
        //   exit: scanner_status:=save_scanner_status; warning_index:=save_warning_index;
        // 即**扫实参期间** `scanner_status=matching`——outer 宏此刻才被禁。
        //
        // ⚠ 实参扫描本身用 `get_token`，且 `get_token` 内部（L7714）会临时
        // `save_scanner_status; scanner_status:=normal; get_token; 恢复`——
        // 故「取单个 token」不触发 outer 检查；只有扫描过程里的**宏展开**才看到
        // `matching`。这正是 pdfTeX 对 `\csca:N \^^L`（`` `#1 `` 取实参）不报错
        // 的原因（外层 normal 未被 brief `get_token` 时期的恢复覆盖）。
        let save_status = self.scanner_status;
        let save_warning = self.warning_index;
        let save_arg_recovery = self.arg_scan_recovered;
        self.warning_index = Some(csid);
        self.scanner_status = ScannerStatus::Matching;
        self.arg_scan_recovered = false;
        let r = self.call_macro_inner(csid, def);
        self.scanner_status = save_status;
        self.warning_index = save_warning;
        self.arg_scan_recovered = save_arg_recovery;
        r
    }

    /// `call_macro` 主体（`scanner_status` 由调用方设/恢复）。
    fn call_macro_inner(&mut self, csid: u32, def: Arc<MacroDef>) -> Result<()> {
        // 诊断轨迹（`NTEX_CALL_TRACE`）：在守卫**之前**记录，溢出那一次调用
        // 也要在内——「谁调用了爆栈的宏」的答案就在它前面几条。
        if let Some(t) = self.macro_trace.as_mut() {
            if t.len() >= MACRO_TRACE_CAP {
                t.pop_front();
            }
            t.push_back(csid);
        }
        self.diag_trace(format!(
            "CALL \\{} depth={}",
            self.intern.name(csid),
            self.stack.len()
        ));
        // 0 参数宏也走 collect_args：参数文本可能是**纯定界串**（`\def\X\fi:\use:n{...}`），
        // 调用点须匹配并吞掉（tex.web macro_call `if info(r)<>end_match_token`）——
        // 见 collect_args n==0 臂注释。
        let args = self.collect_args(csid, &def)?;
        if self.arg_scan_recovered {
            // tex.web macro_call 的 `long_state=outer_call`/Paragraph-ended 恢复
            // 直接跳到调用结束：宏体不能在残缺实参上继续展开，否则会再次读取
            // 已回推的恢复材料而重放同一残流。
            return Ok(());
        }
        // TeX 输入栈上限（tex.web `stack_size`）：宏递归展开无终止条件时以此
        // 报错终止，而非耗尽内存。GT 锚（pdfTeX，35k 行裸 `\rdloop\fi` 循环）：
        // `TeX capacity exceeded, sorry [input stack size = 10000].`——tex.web
        // macro_call 并无 conserve-stack 语义，调用者帧照常入栈；只有
        // `\expandafter\loop\fi` 形态（展开位急切闭合条件，真宏调用前 `\fi`
        // 已消费）才不按行数涨栈。绝不可在此偷收束调用者尾部的 `\fi`：那是
        // 数据，可能是正被扫描的定界实参（\robust@command@act@do，刀A）。
        if self.stack.len() >= MAX_INPUT_STACK {
            let name = self.intern.name(csid).to_owned();
            let _ = self.sink.write16(format!(
                "TeX capacity exceeded, sorry [input stack size = {MAX_INPUT_STACK}].\n"
            ));
            self.report_error_context();
            // 现场转储必须紧邻致命报错做掉：宏/字节码递归爆栈的现场只在栈帧里
            // （此类循环中报错行号是失真的——见 2026-09-11/09-13 两次排查结论）。
            self.dump_input_stack("call-macro");
            return Err(Error::invalid_input(format!(
                "输入栈超限（{MAX_INPUT_STACK} 帧）——宏 \\{name} 递归展开疑似无终止条件"
            )));
        }
        // M2 双轨：字节码优先（未编译则回退解释器轨道）
        if self.use_bytecode {
            if let Some(code) = &def.code {
                self.push_frame(InputFrame::Bytecode {
                    code: code.clone(),
                    pc: 0,
                    args,
                });
                return Ok(());
            }
        }
        self.push_frame(InputFrame::Macro {
            body: def.body.clone(),
            pos: 0,
            args,
        });
        Ok(())
    }

    // ---------- 输入获取 ----------

    /// 探测下一个 token 是否为数学移位（`$$` 检测）：
    /// `consume_for_display`（tex.web `mode>0`，由调用方按 sink 模式判定）为
    /// true 时——是 → 消费该 `$`（显示数学成立）；否则（含受限水平下 peek
    /// 到 `$`）→ 放回（tex.web init_math `back_input`），返回 false。
    /// 显示数学收尾在排版器恢复外层模式后探测，按 get_x_token 展开条件和宏；
    /// 进入侧只取原 token。非 `$` 一律放回。输入耗尽返回 false。
    ///
    fn next_is_math_shift(&mut self, consume_for_display: bool, closing: bool) -> Result<bool> {
        loop {
            let Some((tok, ne)) = self.fetch()? else {
                return Ok(false);
            };
            // tex.web after_math checks the second display delimiter with
            // get_x_token. In particular, an \ifvmode between the two dollars
            // can expand to the matching `$` while the display's vertical mode
            // is still current. init_math, by contrast, uses get_token.
            if closing && consume_for_display && !ne {
                if let Some(op) = self.cond_op(tok) {
                    if !matches!(op, CondOp::Fi | CondOp::Else | CondOp::Or)
                        || !self.cond_stack.is_empty()
                    {
                        self.step_conditional(op, tok)?;
                        continue;
                    }
                } else if self.is_skipping() {
                    continue;
                }
                if let Some(csid) = tok.csid() {
                    let slot = self.eqtb.slot(self.deref_alias_chain(csid)).clone();
                    if match slot {
                        EqSlot::Macro(_) => true,
                        EqSlot::Primitive(p) => p.is_expandable(),
                        _ => false,
                    } {
                        self.push_expansion(tok, ne)?;
                        continue;
                    }
                }
            }
            if tok.catcode() == Some(Catcode::MathShift) && consume_for_display {
                return Ok(true);
            }
            self.push_frame(InputFrame::One { tok, noexpand: ne });
            return Ok(false);
        }
    }
}
