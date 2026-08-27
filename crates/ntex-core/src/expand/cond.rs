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
        if std::env::var("NTEX_COND_TRACE").is_ok() {
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
                    let _ = self.sink.write16("! Extra \\fi.\n".to_string());
                }
                Ok(())
            }
            CondOp::Else => {
                let Some(top) = self.cond_stack.last_mut() else {
                    let _ = self.sink.write16("! Extra \\else.\n".to_string());
                    return Ok(());
                };
                if top.else_seen {
                    let _ = self.sink.write16("! Extra \\else.\n".to_string());
                    return Ok(());
                }
                top.else_seen = true;
                // TeX：\else 后进入 false 分支（\currentifbranch=-1）
                self.cur_if_branch = -1;
                match top.state {
                    CondState::Skipping => {
                        if top.owns_skip {
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
                    let _ = self.sink.write16("! Extra \\or.\n".to_string());
                    return Ok(());
                };
                if !top.is_case {
                    let _ = self.sink.write16("! Extra \\or.\n".to_string());
                    return Ok(());
                }
                match top.state {
                    CondState::Skipping => {
                        if let Some(k) = top.ors_left {
                            if k == 1 {
                                // 跳够 \or：该分支被选中（\currentifbranch=+1）
                                top.state = CondState::Processing;
                                top.ors_left = None;
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
                        saved_if_type: saved_type,
                        saved_if_branch: saved_branch,
                        if_type: code,
                        line: self.error_context().map(|(n, _)| n).unwrap_or(0),
                    });
                    return Ok(());
                }
                let mut truth = self.evaluate_if(op)?;
                if neg {
                    truth = !truth;
                }
                self.cur_if_branch = if truth { 1 } else { -1 };
                if truth {
                    self.cond_stack.push(CondFrame {
                        is_case: false,
                        state: CondState::Processing,
                        owns_skip: false,
                        ors_left: None,
                        else_seen: false,
                        saved_if_type: saved_type,
                        saved_if_branch: saved_branch,
                        if_type: code,
                        line: self.error_context().map(|(n, _)| n).unwrap_or(0),
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
                        saved_if_type: saved_type,
                        saved_if_branch: saved_branch,
                        if_type: code,
                        line: self.error_context().map(|(n, _)| n).unwrap_or(0),
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
                    saved_if_type: saved_type,
                    saved_if_branch: saved_branch,
                    if_type: code,
                    line: self.error_context().map(|(n, _)| n).unwrap_or(0),
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
                            let ln = self.error_context().map(|(n, _)| n).unwrap_or(0);
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
                        saved_if_type: self.cur_if_type,
                        saved_if_branch: self.cur_if_branch,
                        if_type: Self::if_type_code(op),
                        line: self.error_context().map(|(n, _)| n).unwrap_or(0),
                    });
                }
            }
        }
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
                    saved_if_type: saved_type,
                    saved_if_branch: saved_branch,
                    if_type: saved_type,
                    line: self.error_context().map(|(n, _)| n).unwrap_or(0),
                });
                Ok(())
            }
            CondOp::Or => {
                // 非 case 条件的 \or 是错误（TeX：! Extra \or.），保守恢复：
                // 压 Processing 帧让后续 \fi 正常闭合。
                let _ = self.sink.write16("! Extra \\\\or.\n".to_string());
                self.cur_if_branch = -1;
                self.cond_stack.push(CondFrame {
                    is_case: false,
                    state: CondState::Processing,
                    owns_skip: false,
                    ors_left: None,
                    else_seen: true,
                    saved_if_type: saved_type,
                    saved_if_branch: saved_branch,
                    if_type: saved_type,
                    line: self.error_context().map(|(n, _)| n).unwrap_or(0),
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
                let t1 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\if 缺操作数"))?
                    .0;
                let t2 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\if 缺操作数"))?
                    .0;
                Ok(matches!(t1.kind(), TokenKind::Char)
                    && matches!(t2.kind(), TokenKind::Char)
                    && t1 == t2)
            }
            CondOp::IfCat => {
                let t1 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifcat 缺操作数"))?
                    .0;
                let t2 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifcat 缺操作数"))?
                    .0;
                Ok(t1.catcode() == t2.catcode())
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
                if std::env::var("NTEX_IFNUM_TRACE").is_ok() {
                    eprintln!("[trace-ifnum] {a} {rel:?} {b}");
                }
                Ok(compare(a, b, rel))
            }
            CondOp::IfDim => {
                let a = self.scan_dimen()?;
                let rel = self.scan_relation("ifdim")?;
                let b = self.scan_dimen()?;
                if std::env::var("NTEX_IFNUM_TRACE").is_ok() {
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
                        let _ = self.sink.write16("! Bad character code.\n".to_string());
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

    fn scan_relation(&mut self, cond: &str) -> Result<Relation> {
        self.skip_spaces()?;
        let Some((tok, _)) = self.fetch()? else {
            // TeX scan_relation：输入耗尽 → 按 = 恢复
            let _ = self.sink.write16(format!(
                "! Missing = inserted for \\{cond}.\n\
                 I was expecting to see `<', `=', or `>'. Didn't.\n"
            ));
            return Ok(Relation::Eq);
        };
        match tok.charcode() {
            Some(c) if c == b'<' as u32 => Ok(Relation::Lt),
            Some(c) if c == b'=' as u32 => Ok(Relation::Eq),
            Some(c) if c == b'>' as u32 => Ok(Relation::Gt),
            _ => {
                // TeX scan_relation：非关系符 → "Missing = inserted for \<cond>"，
                // token 放回、关系按 = 恢复（TRIP L390 `\ifdim72p\iftrue t1i` 后遇 `1`）
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
                Ok(Relation::Eq)
            }
        }
    }

}
