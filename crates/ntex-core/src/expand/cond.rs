impl Expander {
    // ---------- M1-9 条件 ----------

    fn cond_op(&self, tok: Token) -> Option<CondOp> {
        let csid = tok.csid()?;
        match self.eqtb.slot(csid) {
            EqSlot::Primitive(p) => CondOp::from_prim(*p),
            _ => None,
        }
    }

    /// 是否处于跳过模式（最内层条件帧在跳过）。
    fn is_skipping(&self) -> bool {
        self.cond_stack
            .last()
            .is_some_and(|f| f.state == CondState::Skipping)
    }

    /// 条件状态机单步推进。
    fn step_conditional(&mut self, op: CondOp) -> Result<()> {
        if diag_enabled("NTEX_COND_TRACE") {
            let frames: Vec<String> = self
                .cond_stack
                .iter()
                .map(|f| {
                    format!(
                        "{}{}{}",
                        if f.is_case { "C" } else { "I" },
                        match f.state {
                            CondState::Processing => "P",
                            CondState::Skipping => "S",
                        },
                        if f.else_seen { "e" } else { "-" }
                    )
                })
                .collect();
            eprintln!(
                "[trace-cond] op={:?} 前栈深={} 组级={} [{}]",
                op,
                self.cond_stack.len(),
                self.group_level,
                frames.join(" ")
            );
        }
        match op {
            CondOp::Fi => {
                if let Some(frame) = self.cond_stack.pop() {
                    // 恢复外层条件的类型/分支（TeX：\fi 弹出后 cur_if 回到外层）
                    self.cur_if_type = frame.saved_if_type;
                    self.cur_if_branch = frame.saved_if_branch;
                } else {
                    // TeX 错误恢复：`! Extra \fi.` —— 记录消息并继续（ETRIP 的
                    // \scantokens 恶魔测试会故意制造多余 \fi/\else）。
                    self.report_error("Extra \\fi.");
                }
                Ok(())
            }
            CondOp::Else => {
                let Some(top) = self.cond_stack.last_mut() else {
                    self.report_error("Extra \\else.");
                    return Ok(());
                };
                if top.else_seen {
                    self.report_error("Extra \\else.");
                    return Ok(());
                }
                top.else_seen = true;
                // TeX：\\else 后进入 false 分支（\\currentifbranch=-1）
                self.cur_if_branch = -1;
                match top.state {
                    CondState::Skipping => {
                        // \\ifcase 的 \\else 是兜底落点，但仅当分支尚未选中：
                        // - n<0（负数跳过，ors_left=None）或 n>0 未跳够 → 进入 else 分支
                        // - 已选中分支（case_selected）后遇多余 \\or 转 Skipping → \\else
                        //   是选中分支后的内容，保持跳过
                        // 嵌套在被跳过区域里的非 case 条件（is_case=false）保持 Skipping。
                        if top.is_case && !top.case_selected {
                            top.state = CondState::Processing;
                        }
                    }
                    CondState::Processing => {
                        top.state = CondState::Skipping;
                        top.owns_skip = false;
                    }
                }
                Ok(())
            }
            CondOp::Or => {
                let Some(top) = self.cond_stack.last_mut() else {
                    self.report_error("Extra \\or.");
                    return Ok(());
                };
                if !top.is_case {
                    self.report_error("Extra \\or.");
                    return Ok(());
                }
                match top.state {
                    CondState::Skipping => {
                        if let Some(k) = top.ors_left {
                            if k == 1 {
                                // 跳够 \\or：该分支被选中（\\currentifbranch=+1）
                                top.state = CondState::Processing;
                                top.ors_left = None;
                                top.case_selected = true;
                                self.cur_if_branch = 1;
                            } else {
                                top.ors_left = Some(k - 1);
                            }
                        }
                    }
                    CondState::Processing => {
                        top.state = CondState::Skipping;
                        top.owns_skip = false;
                        self.cur_if_branch = -1;
                    }
                }
                Ok(())
            }
            CondOp::If
            | CondOp::IfCat
            | CondOp::IfNum
            | CondOp::IfDim
            | CondOp::IfX
            | CondOp::IfOdd
            | CondOp::IfTrue
            | CondOp::IfFalse
            | CondOp::IfDefined
            | CondOp::IfCsname
            | CondOp::IfPrimitive
            | CondOp::IfInner
            | CondOp::IfVMode
            | CondOp::IfHMode
            | CondOp::IfMMode
            | CondOp::IfEof
            | CondOp::IfVoid
            | CondOp::IfHBox
            | CondOp::IfVBox
            | CondOp::IfFontChar => {
                // e-TeX（M4-5）：`\unless` 取反下一个条件（类型码同时取负）
                let neg = std::mem::take(&mut self.unless_pending);
                let code = Self::if_type_code(op) * if neg { -1 } else { 1 };
                // TeX：`\if*` 遇到即置新类型（参数扫描期间 branch=0）
                let saved_type = self.cur_if_type;
                let saved_branch = self.cur_if_branch;
                self.cur_if_type = code;
                self.cur_if_branch = 0;
                if self.is_skipping() {
                    // 惰性：不评估测试，仅计数（未走的分支中的宏不被展开）
                    self.cond_stack.push(CondFrame {
                        is_case: false,
                        state: CondState::Skipping,
                        owns_skip: false,
                        ors_left: None,
                        else_seen: false,
                        case_selected: false,
saved_if_type: saved_type,
                        saved_if_branch: saved_branch,
                        if_type: code,
                        line: self.error_line_no(),
                    });
                    return Ok(());
                }
                // TeX 错误恢复：条件求值失败（Arithmetic overflow / Missing number
                // 等）后按 false 继续——条件帧照常建立（truth=false 走 skip_ahead），
                // 否则后续 \else/\fi 找不到帧全部错乱（etrip L805-873 \1 体
                // \ifnum 的 overflow 连锁 → l.880 Extra \else + Missing =）。
                let mut truth = self.evaluate_if(op).unwrap_or_default();
                if neg {
                    truth = !truth;
                }
                self.cur_if_branch = if truth { 1 } else { -1 };
                // tex.web：\if 求值后打印 {true}/{false}（tracing_commands>0；
                // 跳过区的 \if 惰性分支不评估不打印——上方 return 已处理）
                if self.params.misc[3] > 0 && self.trace_suppress == 0 {
                    let _ = self
                        .sink
                        .write16(if truth { "{true}".into() } else { "{false}".into() });
                }
                if truth {
                    self.cond_stack.push(CondFrame {
                        is_case: false,
                        state: CondState::Processing,
                        owns_skip: false,
                        ors_left: None,
                        else_seen: false,
                        case_selected: false,
saved_if_type: saved_type,
                        saved_if_branch: saved_branch,
                        if_type: code,
                        line: self.error_line_no(),
                    });
                } else {
                    // TeX：false 条件不压帧，立即 `skip_ahead` 到匹配的
                    // \else/\or/\fi（tex.web P27）。若等外层主循环跳过，扫描
                    // 内部的条件（TRIP L82 `\scriptspace...\ifnum'\ifnum10=10 12="\fi`）
                    // 已错位——\fi 必须闭合栈顶的内层帧（内层 \ifnum），
                    // 而本条件的跳过恰好消费该 \fi 结束。
                    self.skip_ahead(saved_type, saved_branch)?;
                }
                Ok(())
            }
            CondOp::IfCase => {
                let neg = std::mem::take(&mut self.unless_pending);
                let code = Self::if_type_code(op) * if neg { -1 } else { 1 };
                let saved_type = self.cur_if_type;
                let saved_branch = self.cur_if_branch;
                self.cur_if_type = code;
                self.cur_if_branch = 0;
                if self.is_skipping() {
                    self.cond_stack.push(CondFrame {
                        is_case: true,
                        state: CondState::Skipping,
                        owns_skip: false,
                        ors_left: None,
                        else_seen: false,
                        case_selected: false,
saved_if_type: saved_type,
                        saved_if_branch: saved_branch,
                        if_type: code,
                        line: self.error_line_no(),
                    });
                    return Ok(());
                }
                let n = self.scan_number()?;
                // TeX：n<0 时跳过所有 \or 直到 \else（\ifcase-1 → else 分支）
                self.cur_if_branch = if n == 0 { 1 } else { -1 };
                self.cond_stack.push(CondFrame {
                    is_case: true,
                    state: if n == 0 {
                        CondState::Processing
                    } else {
                        CondState::Skipping
                    },
                    owns_skip: n > 0,
                    ors_left: (n > 0).then_some(n as usize),
                    else_seen: false,
                    // n==0：case 0 直接选中（遇多余 \or/\else 保持跳过）
                    case_selected: n == 0,
saved_if_type: saved_type,
                    saved_if_branch: saved_branch,
                    if_type: code,
                    line: self.error_line_no(),
                });
                Ok(())
            }
        }
    }

    /// TeX `skip_ahead`：false 条件求值后立即跳到匹配的 `\else`/`\or`/`\fi`。
    /// 本条件不压帧（TeX 语义），期间：
    /// - 普通 token 丢弃（不展开）；
    /// - 嵌套 `\if*` 惰性计数（压 Skipping 帧，`\fi` 时弹出）；
    /// - 嵌套的 `\else`/`\or` 不计数（TeX skip_ahead 同样忽略）；
    /// - 到达本层级的 `\fi` → 结束（本条件直接闭合，无帧）；
    /// - 到达本层级的 `\else` → 进入 else 分支（压 Processing 帧）。
    fn skip_ahead(&mut self, saved_type: i32, saved_branch: i32) -> Result<()> {
        let target = self.cond_stack.len();
        loop {
            let Some((tok, _ne)) = self.fetch()? else {
                return Err(Error::invalid_input("\\if 缺少 \\fi"));
            };
            let Some(op) = self.cond_op(tok) else {
                // TeX：跳过 text 中出现 outer 宏 → "Incomplete \if...; all text
                // was ignored after line N." + "A forbidden control sequence
                // occurred in skipped text."（tex.web `get_next` 的 forbidden 检查）。
                // 插入 `\fi` 结束跳过、offending cs 放回输入流（TRIP L363
                // `\^^C{{ \span\ifcase3 \lo...`：\lo 为 \outer，触发本恢复）。
                if let Some(csid) = tok.csid() {
                    if let EqSlot::Macro(m) = self.eqtb.slot(csid) {
                        if m.value.outer {
                            let name = self.intern.name(csid).to_owned();
                            let ln = self.error_line_no();
                            let _ = self.sink.write16(format!(
                                "! Incomplete \\if; all text was ignored after line {ln}.\n\
                                 <inserted text>\n                \\fi \n\
                                 <to be read again>\n                   \\{name}\n\
                                 A forbidden control sequence occurred in skipped text.\n\
                                 This kind of error happens when you say `\\if...' and forget\n\
                                 the matching `\\fi'. I've inserted a `\\fi'; this might work.\n"
                            ));
                            self.unread(tok);
                            while self.cond_stack.len() > target {
                                self.cond_stack.pop();
                            }
                            self.cur_if_type = saved_type;
                            self.cur_if_branch = saved_branch;
                            return Ok(());
                        }
                    }
                }
                continue; // 普通 token 丢弃
            };
            match op {
                CondOp::Fi => {
                    if self.cond_stack.len() == target {
                        // 本条件的 \fi：直接闭合，无帧，恢复外层类型
                        self.cur_if_type = saved_type;
                        self.cur_if_branch = saved_branch;
                        return Ok(());
                    }
                    self.cond_stack.pop(); // 嵌套条件闭合
                }
                CondOp::Else | CondOp::Or => {
                    if self.cond_stack.len() == target {
                        // 本条件的 \else/\or：进入该分支
                        return self.enter_skipped_branch(op, saved_type, saved_branch);
                    }
                    // 嵌套条件的 \else/\or：TeX skip_ahead 忽略，不计数
                }
                _ => {
                    // 嵌套条件开始：惰性计数（跳过中不评估测试）
                    self.cond_stack.push(CondFrame {
                        is_case: matches!(op, CondOp::IfCase),
                        state: CondState::Skipping,
                        owns_skip: false,
                        ors_left: None,
                        else_seen: false,
                        case_selected: false,
saved_if_type: self.cur_if_type,
                        saved_if_branch: self.cur_if_branch,
                        if_type: Self::if_type_code(op),
                        line: self.error_line_no(),
                    });
                }
            }
        }
    }

    /// 展开上下文里"开着的跳过区"的急切消费（tex.web `expand`：`\else`/`\or`
    /// 的 `fi_or_else` 处理是 `while cur_chr<>fi_code do pass_text; pop`，
    /// false 的 `\if*` 也由 `conditional` 就地 `pass_text`——分支从不滞留）。
    ///
    /// 主循环对"待丢弃的分支"是**惰性**跳过（`process_one` 逐 token 丢弃、
    /// `\fi` 到来才弹帧），在逐 token 路径上与 tex.web 等价。但 `\expandafter`
    /// 会把 token 推回输入流——若推回时条件帧仍处于 Skipping，推回的 token
    /// 会被惰性跳过区吞掉（latex.ltx L488 `\newbox` → `\e@alloc` 的
    /// `\global\ifnum…\expandafter\chardef\else…\fi\voidb@x\allocationnumber`
    /// 即此：`\chardef` 被吞、`\voidb@x` 落空成 undefined）。展开上下文
    /// （`exec_expandafter` / `expand_once` 的 Expandafter 臂）在推进条件机后
    /// 用本函数把仍开着的跳过区就地消费，对齐 tex.web 的急切语义。
    ///
    /// `depth` 是刚进入 Skipping 的条件帧下标（调用前的 `cond_stack.len()`，
    /// `\else`/`\or` 为 len-1）。消费直到该帧**离开 Skipping**：`\fi` 弹出它，
    /// 或 `\ifcase` 的 `\or` 选中分支（回到 Processing——选中分支必须保持活）。
    /// 期间：普通 token 丢弃（不展开、不执行、不报错——pass_text 语义）；
    /// 嵌套 `\if*` 经条件机压惰性 Skipping 帧（等价 pass_text 的 `incr(l)`）；
    /// 本层级的 `\else`/`\or` 继续跳（`while cur_chr<>fi_code`）。输入耗尽不报
    /// 错——帧保持 Skipping，交给 `\end`/EOF 的 `Incomplete \if` 收口（与主
    /// 循环惰性路径一致）。
    fn drain_open_skip(&mut self, depth: usize) -> Result<()> {
        while self
            .cond_stack
            .get(depth)
            .is_some_and(|f| f.state == CondState::Skipping)
        {
            let Some((tok, noexpand)) = self.fetch()? else {
                return Ok(());
            };
            if noexpand {
                continue;
            }
            if let Some(op) = self.cond_op(tok) {
                self.step_conditional(op)?;
            }
        }
        Ok(())
    }

    /// 跳过结束于本条件的 `\else`/`\or` 时进入该分支。
    fn enter_skipped_branch(
        &mut self,
        op: CondOp,
        saved_type: i32,
        saved_branch: i32,
    ) -> Result<()> {
        match op {
            CondOp::Else => {
                // 进入 else 分支：压 Processing 帧（等价于 \else 状态机转换）
                self.cur_if_branch = -1;
                self.cond_stack.push(CondFrame {
                    is_case: false,
                    state: CondState::Processing,
                    owns_skip: false,
                    ors_left: None,
                    else_seen: true,
                    case_selected: false,
                    saved_if_type: saved_type,
                    saved_if_branch: saved_branch,
                    if_type: saved_type,
                    line: self.error_line_no(),
                });
                Ok(())
            }
            CondOp::Or => {
                // 非 case 条件的 \or 是错误（TeX：! Extra \or.），保守恢复：
                // 压 Processing 帧让后续 \fi 正常闭合。
                self.report_error("Extra \\\\or.");
                self.cur_if_branch = -1;
                self.cond_stack.push(CondFrame {
                    is_case: false,
                    state: CondState::Processing,
                    owns_skip: false,
                    ors_left: None,
                    else_seen: true,
                    case_selected: false,
                    saved_if_type: saved_type,
                    saved_if_branch: saved_branch,
                    if_type: saved_type,
                    line: self.error_line_no(),
                });
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// TeX 条件类型码（`\currentiftype` 用）：0=无、1=\if、2=\ifcat、3=\ifnum、
    /// 4=\ifdim、5=\ifodd、6=\ifvmode、7=\ifhmode、8=\ifmmode、9=\ifinner、
    /// 10=\ifvoid、11=\ifhbox、12=\ifvbox、13=\ifx、14=\ifeof、15=\iftrue、
    /// 16=\iffalse、17=\ifcase、18=\ifdefined、19=\ifcsname、20=\iffontchar、
    /// 21=\ifprimitive。
    fn if_type_code(op: CondOp) -> i32 {
        match op {
            CondOp::If => 1,
            CondOp::IfCat => 2,
            CondOp::IfNum => 3,
            CondOp::IfDim => 4,
            CondOp::IfOdd => 5,
            CondOp::IfVMode => 6,
            CondOp::IfHMode => 7,
            CondOp::IfMMode => 8,
            CondOp::IfInner => 9,
            CondOp::IfVoid => 10,
            CondOp::IfHBox => 11,
            CondOp::IfVBox => 12,
            CondOp::IfX => 13,
            CondOp::IfEof => 14,
            CondOp::IfTrue => 15,
            CondOp::IfFalse => 16,
            CondOp::IfCase => 17,
            CondOp::IfDefined => 18,
            CondOp::IfCsname => 19,
            CondOp::IfFontChar => 20,
            CondOp::IfPrimitive => 21,
            _ => 0,
        }
    }

    /// 条件类型码 → TeX 显示名（`Incomplete \ifxxx` 消息；tex.web if_type_name 语义）。
    fn if_type_name(code: i32) -> &'static str {
        match code.abs() {
            1 => "\\if",
            2 => "\\ifcat",
            3 => "\\ifnum",
            4 => "\\ifdim",
            5 => "\\ifodd",
            6 => "\\ifvmode",
            7 => "\\ifhmode",
            8 => "\\ifmmode",
            9 => "\\ifinner",
            10 => "\\ifvoid",
            11 => "\\ifhbox",
            12 => "\\ifvbox",
            13 => "\\ifx",
            14 => "\\ifeof",
            15 => "\\iftrue",
            16 => "\\iffalse",
            17 => "\\ifcase",
            18 => "\\ifdefined",
            19 => "\\ifcsname",
            20 => "\\iffontchar",
            21 => "\\ifprimitive",
            _ => "\\if",
        }
    }

    /// 评估条件测试（跳过模式下不会被调用）。
    fn evaluate_if(&mut self, op: CondOp) -> Result<bool> {
        match op {
            CondOp::IfTrue => Ok(true),
            CondOp::IfFalse => Ok(false),
            CondOp::If => {
                let (c1, _) = self.get_x_char_operand("\\if")?;
                let (c2, _) = self.get_x_char_operand("\\if")?;
                // tex.web：非字符操作数 cur_chr:=256 哨兵——cs vs cs 恒真、
                // 字符 vs cs 恒假
                Ok(c1 == c2)
            }
            CondOp::IfCat => {
                let (_, k1) = self.get_x_char_operand("\\ifcat")?;
                let (_, k2) = self.get_x_char_operand("\\ifcat")?;
                // tex.web：非字符操作数 cur_cmd:=relax 哨兵——cs vs cs 恒真
                Ok(k1 == k2)
            }
            CondOp::IfX => {
                let t1 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifx 缺操作数"))?
                    .0;
                let t2 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifx 缺操作数"))?
                    .0;
                Ok(self.ifx_equal(t1, t2))
            }
            CondOp::IfNum => {
                let a = self.scan_number()?;
                let rel = self.scan_relation("ifnum")?;
                let b = self.scan_number()?;
                if diag_enabled("NTEX_IFNUM_TRACE") {
                    eprintln!("[trace-ifnum] {a} {rel:?} {b}");
                }
                Ok(compare(a, b, rel))
            }
            CondOp::IfDim => {
                let a = self.scan_dimen()?;
                let rel = self.scan_relation("ifdim")?;
                let b = self.scan_dimen()?;
                if diag_enabled("NTEX_IFNUM_TRACE") {
                    eprintln!("[trace-ifdim] {a} {rel:?} {b}");
                }
                Ok(compare(a, b, rel))
            }
            CondOp::IfOdd => Ok(self.scan_number()? % 2 != 0),
            // e-TeX（M4-5）
            CondOp::IfDefined => {
                let tok = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifdefined 缺操作数"))?
                    .0;
                match tok.kind() {
                    TokenKind::ControlSeq => {
                        let csid = tok.csid().expect("ControlSeq 必有 csid");
                        Ok(!matches!(self.eqtb.slot(csid), EqSlot::Undefined))
                    }
                    _ => Ok(false),
                }
            }
            // e-TeX（M4-5）：\ifinner —— 当前模式为内部（数学/受限水平/内层垂直）
            CondOp::IfInner => Ok(self.sink.if_inner()),
            // ETRIP 冲刺：\ifvmode/\ifhmode/\ifmmode —— 当前模式族
            // （TeX 模式码：1=垂直、2=水平、3=数学、4=内层垂直、5=受限水平、6=显示数学）
            CondOp::IfVMode => Ok(matches!(self.sink.mode_code(), 1 | 4)),
            CondOp::IfHMode => Ok(matches!(self.sink.mode_code(), 2 | 5)),
            CondOp::IfMMode => Ok(matches!(self.sink.mode_code(), 3 | 6)),
            // ETRIP 冲刺：\ifeof<流> —— 读流未打开或已到末尾为真
            CondOp::IfEof => {
                let idx = self.scan_stream_index("\\ifeof", 15)?;
                Ok(self.read_streams.get(idx).map_or(true, |s| match s {
                    None => true,
                    Some(rs) => rs.pos >= rs.data.len(),
                }))
            }
            // ETRIP 冲刺：\ifvoid/\ifhbox/\ifvbox<寄存器> —— 盒子寄存器种类
            // （sink 查询：0=void、1=hbox、2=vbox）
            CondOp::IfVoid => {
                let idx = self.scan_register_index()?;
                Ok(self.sink.box_register_kind(idx) == 0)
            }
            CondOp::IfHBox => {
                let idx = self.scan_register_index()?;
                Ok(self.sink.box_register_kind(idx) == 1)
            }
            CondOp::IfVBox => {
                let idx = self.scan_register_index()?;
                Ok(self.sink.box_register_kind(idx) == 2)
            }
            // ETRIP 冲刺：\iffontchar<font><char> —— 字体含该字符为真。
            // 参数缺失/非法（如 `\iffontchar \else \fi`）报错恢复取假；
            // 字符码越界（<0 或 >255）报 "! Bad character code." 并取假。
            CondOp::IfFontChar => {
                let scanned = (|| -> Result<(u32, u32)> {
                    let font = self.scan_font_ident()?;
                    let ch = self.scan_number()?;
                    if !(0..=255).contains(&ch) {
                        return Err(Error::invalid_input("Bad character code"));
                    }
                    Ok((font, ch as u32))
                })();
                let (font, ch) = match scanned {
                    Ok(v) => v,
                    Err(_) => {
                        self.report_error("Bad character code.");
                        return Ok(false);
                    }
                };
                Ok(self
                    .font_loader
                    .char_metric(font, ch)
                    .is_some())
            }
            CondOp::IfCsname => {
                let name = self.scan_csname()?;
                Ok(matches!(
                    self.intern.lookup(&name),
                    Some(id) if !matches!(self.eqtb.slot(id), EqSlot::Undefined)
                ))
            }
            // e-TeX（M4-5）：\ifprimitive <cs> —— cs 的 eqtb 槽是内建原语
            CondOp::IfPrimitive => {
                let tok = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifprimitive 缺操作数"))?
                    .0;
                match tok.kind() {
                    TokenKind::ControlSeq => {
                        let csid = tok.csid().expect("ControlSeq 必有 csid");
                        Ok(matches!(self.eqtb.slot(csid), EqSlot::Primitive(_)))
                    }
                    _ => Ok(false),
                }
            }
            CondOp::IfCase | CondOp::Else | CondOp::Fi | CondOp::Or => {
                unreachable!("step_conditional 已分流")
            }
        }
    }

    /// `\ifx`：字符按 (catcode,char)；控制序列按含义（解析别名）；其余 false。
    fn ifx_equal(&self, t1: Token, t2: Token) -> bool {
        match (t1.kind(), t2.kind()) {
            (TokenKind::Char, TokenKind::Char) => t1 == t2,
            (TokenKind::ControlSeq, TokenKind::ControlSeq) => {
                self.meaning_key(t1.csid().expect("ControlSeq 必有 csid"))
                    == self.meaning_key(t2.csid().expect("ControlSeq 必有 csid"))
            }
            _ => false,
        }
    }

    /// 控制序列的含义键（沿 Alias 链解析；环检测替代固定跳数上限）。
    fn meaning_key(&self, csid: u32) -> MeaningKey {
        let mut id = csid;
        // A7：别名链可成环（`\let\a\b\let\b\a` 在间接别名模型下 → a↔b 互指）。
        // 无环链长至多 = csid 总数（每个 csid 至多出现一次，有界）；遇环视为
        // 未定义（环无确定含义）。替代此前的 64 跳硬上限（超长链误判）。
        let mut seen: Vec<u32> = Vec::with_capacity(8);
        while let EqSlot::Alias(target) = self.eqtb.slot(id) {
            if seen.contains(&id) {
                return MeaningKey::Undefined;
            }
            seen.push(id);
            id = *target;
        }
        match self.eqtb.slot(id).clone() {
            EqSlot::Undefined => MeaningKey::Undefined,
            EqSlot::Macro(m) => MeaningKey::Macro {
                def: m.value.clone(),
                outer: m.value.outer,
            },
            EqSlot::Primitive(p) => MeaningKey::Primitive(p),
            EqSlot::Char { catcode, charcode } => MeaningKey::Char { catcode, charcode },
            EqSlot::Alias(t) => MeaningKey::Alias(t),
            EqSlot::Font(font) => MeaningKey::Font(font),
            EqSlot::Register(k, n) => MeaningKey::Register(k, n),
            EqSlot::Stream(k, n) => MeaningKey::Stream(k, n),
            EqSlot::MathChar(code) => MeaningKey::MathChar(code),
        }
    }

    /// `\if`/`\ifcat` 操作数取 token（tex.web `get_x_token_or_active_char`，
    /// @<Test if two characters match@> 的取数臂）。
    ///
    /// - **get_x_token 展开语义**：宏（非 protected）/可展开原语展开一次后重取；
    ///   未定义 cs 报 "! Undefined control sequence." 当 `\relax`（非字符）返回；
    ///   条件原语推进条件机（step + drain，同 [`Self::scan_relation`] 的臂）。
    /// - `\noexpand` 冻结的 cs 按 tex.web 当 **active char**（cat 13），字符码取
    ///   cs 编号哨兵（256+csid，与真实字符码 0..=255 不相交）。
    /// - 返回 `(字符码, 类码)`；**非字符操作数两者均为 `None`**——tex.web
    ///   cur_cmd:=relax / cur_chr:=256 哨兵：cs vs cs 恒真、字符 vs cs 恒假。
    ///
    /// LaTeX 兼容第十五刀：此前 `\if` 直接取字面 token 且要求两侧都是字符 token、
    /// `\ifcat` 取字面 token——`\if:w #4 \__cs_generate_variant_loop_base:N #2`
    /// （expl3-code.tex l.2861 变体生成循环）右操作数是宏调用，未展开时按
    /// "非字符"判假 → 变体串分析全错 → `invalid-variant`/`Extra \fi` 级联
    /// （expl3 l.3245-3324，见报告 §21.3）。
    fn get_x_char_operand(&mut self, cond: &str) -> Result<(Option<u32>, Option<Catcode>)> {
        loop {
            let Some((tok, noexpand)) = self.fetch()? else {
                return Err(Error::invalid_input(format!(
                    "\\.if 操作数扫描到输入末尾（\\{cond}）"
                )));
            };
            // 跳过区惰性消费（同 scan_relation）：操作数位置的内层条件被拒分支
            // （`\ifx…\else x\fi`）由本循环吞掉，不得当操作数
            if self.is_skipping() {
                if let Some(op) = self.cond_op(tok) {
                    self.step_conditional(op)?;
                }
                continue;
            }
            if let Some(csid) = tok.csid() {
                let expandable = match self.eqtb.slot(csid).clone() {
                    EqSlot::Undefined => {
                        let _ = self.sink.write16(format!(
                            "! Undefined control sequence.\n\\{}\n",
                            self.intern.name(csid)
                        ));
                        // tex.web get_x_token 错误恢复：当 \relax → 非字符
                        return Ok((None, None));
                    }
                    EqSlot::Macro(m) => !(m.value.protected && self.suppress_expansion > 0),
                    EqSlot::Primitive(p) if p.is_expandable() => true,
                    _ => false,
                };
                if expandable && !noexpand {
                    let mut expansion = Vec::new();
                    self.expand_once((tok, false), &mut expansion)?;
                    if !expansion.is_empty() {
                        self.stack.push(InputFrame::TokenList {
                            items: Arc::from(expansion),
                            pos: 0,
                        });
                    }
                    continue;
                }
                if noexpand {
                    // \noexpand 冻结 cs → active char（cat 13），码取 cs 哨兵
                    return Ok((Some(256 + csid), Some(Catcode::Active)));
                }
                // 不可展开 cs（\relax、\hbox、字符型 cs …）→ 非字符
                return Ok((None, None));
            }
            return Ok((tok.charcode(), tok.catcode()));
        }
    }

    /// 关系符扫描（tex.web @<Test relation between integers or dimensions@> 取
    /// token 用的是 `repeat get_x_token until cur_cmd<>spacer`）：**关系符位置的
    /// 可展开 filler 先展开再判 `<`/`=`/`>`**。
    ///
    /// LaTeX 兼容第十刀：此前直接取字面 token，`\def\z{=}\ifnum0\z 0` 中 `\z`
    /// 不展开 → "Missing = inserted"、`\z` 放回后 `=` 落到右操作数被当垃圾，
    /// `0 T` 泄漏为排版文本。expl3-code.tex L193-206 引擎门闩
    /// `\ifnum0%\expandafter\ifx…=0`（§15.5）同根因——关系符位置是一整段
    /// 可展开探测链（`\expandafter`+嵌套 `\ifx`）。与第七/八刀（scan_number
    /// 数字循环的展开/条件聚合）同族、位置不同，此处按 tex.web 另有
    /// `\if*`/`\else`/`\fi` 条件机推进（step + drain）与未定义 cs 报错当
    /// relax 两臂。
    ///
    /// `\relax`/寄存器/字符等不可展开项落"非关系符"臂放回（tex.web
    /// back_error；TRIP L390 `\ifdim72p\iftrue` 的直写关系符形式不受影响）。
    fn scan_relation(&mut self, cond: &str) -> Result<Relation> {
        loop {
            self.skip_spaces()?;
            let Some((tok, noexpand)) = self.fetch()? else {
                // TeX scan_relation：输入耗尽 → 按 = 恢复
                let _ = self.sink.write16(format!(
                    "! Missing = inserted for \\{cond}.\n\
                     I was expecting to see `<', `=', or `>'. Didn't.\n"
                ));
                return Ok(Relation::Eq);
            };
            // get_x_token 展开语义（同 scan_number 符号循环）：宏（非
            // protected）/可展开原语展开后重取；未定义 cs 报错当 \relax 继续循环
            // （tex.web get_x_token 错误恢复）。`\noexpand` 冻结的 token 不展开。
            //
            // 跳过区惰性消费（同 scan_number 数字循环的 is_skipping 臂）：关系符
            // 位置的内层条件被拒分支（`\ifx…\else 1\fi` 的 `1\fi`）由本循环吞掉，
            // 不得当关系符/右操作数——否则 `\else` 翻 Skipping 后 `1` 报
            // "Missing ="、`\fi` 被右操作数数字循环吞掉（b 取 0 而非 1）。
            if self.is_skipping() {
                if let Some(op) = self.cond_op(tok) {
                    self.step_conditional(op)?;
                }
                continue;
            }
            if let Some(csid) = tok.csid() {
                let expandable = match self.eqtb.slot(csid).clone() {
                    EqSlot::Undefined => {
                        let _ = self.sink.write16(format!(
                            "! Undefined control sequence.\n\\{}\n",
                            self.intern.name(csid)
                        ));
                        true
                    }
                    EqSlot::Macro(m) => !(m.value.protected && self.suppress_expansion > 0),
                    EqSlot::Primitive(p) if p.is_expandable() => true,
                    _ => false,
                };
                if expandable && !noexpand {
                    let mut expansion = Vec::new();
                    self.expand_once((tok, false), &mut expansion)?;
                    if !expansion.is_empty() {
                        self.stack.push(InputFrame::TokenList {
                            items: Arc::from(expansion),
                            pos: 0,
                        });
                    }
                    continue;
                }
            }
            // 条件原语：get_x_token 求值语义。tex.web 关系符取 token 的
            // `repeat get_x_token until cur_cmd<>spacer` 对 if_test **与**
            // fi_or_else 都走 expand（max_command<cur_cmd<call）——`\ifx…\relax
            // \else 1\fi=0` 中 `\else` 不落"非关系符"臂，而是翻转分支帧后跳到
            // \fi 弹帧（tex.web @<Terminate the current conditional…@>）。故此处
            // 与 expr.rs \expandafter 臂同款：step_conditional + drain_open_skip。
            // 注意不可用 maybe_eval_cond（数字扫描版刻意对 \fi/\else/\or 返回
            // false 放回外层——TRIP L82 十六进制循环的 \fi 属外层未决条件），
            // 关系符位是展开位置、语义就是 get_x_token 本身。
            if let Some(op) = self.cond_op(tok) {
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
                continue;
            }
            match tok.charcode() {
                Some(c) if c == b'<' as u32 => return Ok(Relation::Lt),
                Some(c) if c == b'=' as u32 => return Ok(Relation::Eq),
                Some(c) if c == b'>' as u32 => return Ok(Relation::Gt),
                _ => {
                    // 非关系符 → "Missing = inserted for \<cond>"，token 放回、
                    // 关系按 = 恢复（tex.web back_error）
                    let name = tok
                        .csid()
                        .map(|id| self.intern.name(id).to_string())
                        .unwrap_or_else(|| format!("{:?}", tok));
                    let _ = self.sink.write16(format!(
                        "! Missing = inserted for \\{cond}.\n\
                         <to be read again>\n                   {name}\n\
                         I was expecting to see `<', `=', or `>'. Didn't.\n"
                    ));
                    self.unread(tok);
                    return Ok(Relation::Eq);
                }
            }
        }
    }

}
