impl Expander {
    // ---------- 数字与赋值辅助 ----------

    /// TRIP：数字/尺寸扫描中遇到**条件开始**原语 → 求值并返回 true（TeX
    /// get_x_token 语义：`'\ifnum10=10 12="`——外层 \ifnum 操作数含内层条件，
    /// 内层先求值并输出分支 token）。
    ///
    /// 注意：`\fi`/`\else`/`\or` 是**不可展开**的终结符——TeX scan_int 遇到它们
    /// 直接 back_input 停止扫描（tex.web get_x_token 对 fi_or_else 不展开），
    /// 由外层条件状态机在扫描结束后消费。若在此求值会错位弹栈，如 TRIP L82
    /// `\ifnum'\ifnum10=10 12="\fi`：内层 \fi 必须在 number2 扫描中放回，
    /// 外层 \ifnum 求值为 false 后跳过分支时再闭合。
    fn maybe_eval_cond(&mut self, tok: Token) -> Result<bool> {
        if let Some(op) = self.cond_op(tok) {
            // 仅条件开始（\if*）在数字中先求值；\fi/\else/\or 是不可展开终结符，
            // 放回由外层条件状态机在扫描结束后消费（TeX scan_int back_input
            // 语义）——否则 \numexpr...\else 求值时栈深 0 触发 Extra \else
            // 错乱（etrip L805-873 \1 体 \ifnum 的连锁，l.880 Extra \else）。
            if matches!(op, CondOp::Fi | CondOp::Else | CondOp::Or) {
                return Ok(false);
            }
            if std::env::var("NTEX_COND_TRACE").is_ok() {
                eprintln!("[trace-maybe] 求值条件 {op:?}");
            }
            self.step_conditional(op)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 扫描十进制整数；支持 `\count<idx>` 寄存器引用（M1 简化版）。
    fn scan_number(&mut self) -> Result<i64> {
        // 默认跳过可选 `=`（赋值上下文）；scan_register_index 等内部扫描不跳
        // （tex.web scan_optional_equals 由调用方处理，scan_int 从不跳 `=`）。
        self.scan_number_inner(true)
    }

    /// TeX `scan_int` 核心：读整数。`skip_equals` 控制是否跳过可选赋值符
    /// `=`——tex.web 中 `=` 由调用方的 `scan_optional_equals` 消费（如 `\count0=5`），
    /// `scan_int` 本身不跳；NTex 此前把两者折叠导致 `\setbox=` 漏报 Missing number
    /// （TRIP l.253），现拆出由调用方选择。
    fn scan_number_inner(&mut self, skip_equals: bool) -> Result<i64> {
        self.skip_spaces()?;
        if skip_equals {
            // TeX scan_optional_equals：跳过可选 `=` 赋值符（`\count0=5` 与 `\count0 5` 等价）
            if let Some((tok, _)) = self.fetch()? {
                if tok.charcode() != Some(b'=' as u32) {
                    self.unread(tok);
                }
            }
        }
        let mut neg = false;
        // 负号与未定义 cs 统一循环（TeX get_x_token）：`--\skip90`、
        // `-\mutoglue-\gluetomu9pt` 等逐 token 恢复（未定义 → 报错当 \relax）。
        loop {
            self.skip_spaces()?;
            let Some((tok, _)) = self.fetch()? else { break };
            if tok.charcode() == Some(b'-' as u32) {
                neg = !neg;
                continue;
            }
            // TeX scan_int：正号忽略（`\varunit=+1,001...`，TRIP L160）
            if tok.charcode() == Some(b'+' as u32) {
                continue;
            }
            if let Some(csid) = tok.csid() {
                if matches!(self.eqtb.slot(csid), EqSlot::Undefined) {
                    let _ = self
                        .sink
                        .write16(format!(
                            "! Undefined control sequence.\n\\{}\n",
                            self.intern.name(csid)
                        ));
                    continue;
                }
                // TRIP：条件原语在数字中先求值（TeX get_x_token 嵌套条件）
                if self.maybe_eval_cond(tok)? {
                    continue;
                }
                // tex.web scan_int 符号循环的 get_x_token 语义：宏/可展开原语
                // 展开后重新进入符号处理（trip.tex L103 `\tracingoutput\on`：
                // \on 宏展开为 1 作为参数值——缺此分支则报 Missing number 并把
                // \on 遗留到输入流，L104 \moveleft 连锁错位）。
                let expandable = match self.eqtb.slot(csid).clone() {
                    EqSlot::Macro(m) => !(m.value.protected && self.suppress_expansion > 0),
                    EqSlot::Primitive(p) if p.is_expandable() => true,
                    _ => false,
                };
                if expandable {
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
            self.unread(tok);
            break;
        }
        // 反引号字符码：`<char>（TeX scan_int 的 alphabetic constant，TeXbook p.267）
        if let Some(code) = self.try_scan_backquote()? {
            // TeX scan_int：数字（含反引号常量）后跟随的空格被吞
            self.skip_trailing_spaces()?;
            return Ok(if neg { -code } else { code });
        }
        // 基数前缀：十六进制 `"`（radix 16）与八进制 `'`（radix 8），TeXbook p.267
        if let Some((tok, _)) = self.fetch()? {
            let radix = match (tok.catcode(), tok.charcode()) {
                (Some(Catcode::Other), Some(c)) if c == b'"' as u32 => Some(16u32),
                (Some(Catcode::Other), Some(c)) if c == b'\'' as u32 => Some(8u32),
                _ => None,
            };
            if let Some(base) = radix {
                let mut val: i64 = 0;
                let mut any = false;
                while let Some((t, _)) = self.fetch()? {
                    match radix_digit_value(t, base) {
                        Some(d) => {
                            // 防溢出：达到上限后停止累加（TeX scan_int 钳制语义）
                            if val < i64::MAX / i64::from(base) {
                                val = val * i64::from(base) + i64::from(d);
                            }
                            any = true;
                        }
                        None => {
                            // TRIP：条件原语在数字中先求值（`'\ifnum10=10 12="`）
                            if self.maybe_eval_cond(t)? {
                                continue;
                            }
                            self.unread(t);
                            break;
                        }
                    }
                }
                if !any {
                    // TeX scan_int：基数前缀后无数位 → "Missing number, treated as zero"
                    // 恢复（trip.tex 等；token 已放回，继续后续输入）
                    self.report_missing_number();
                    return Ok(0);
                }
                self.skip_trailing_spaces()?;
                return Ok(if neg { -val } else { val });
            }
            self.unread(tok);
        }
        // 寄存器引用：\count<idx> 或 \count\cs（\newcount 分配的 cs）
        if let Some(csid) = self.peek_csid()? {
            let slot = self.eqtb.slot(csid).clone();
            match slot {
                // e-TeX（M4-5）：\numexpr 可在任意整数上下文求值
                EqSlot::Primitive(Primitive::NumExpr) => {
                    self.fetch()?; // 消费 \numexpr
                    let v = self.eval_int_expression()?;
                    return Ok(if neg { -v } else { v });
                }
                // e-TeX：\dimexpr/\glueexpr/\muexpr 也可在整数上下文求值
                // （etrip L826-828：`\ifnum#4=-\dimexpr-#2sp/#3`、`\glueexpr\muexpr...`）。
                // 结果为 sp 值（dimen）或胶水宽度（glue/mu；\muexpr 注册为 Glueexpr 别名）。
                EqSlot::Primitive(Primitive::Dimexpr) => {
                    self.fetch()?; // 消费 \dimexpr
                    let v = self.eval_dimen_expression()?;
                    return Ok(if neg { -v } else { v });
                }
                EqSlot::Primitive(Primitive::Glueexpr) => {
                    self.fetch()?; // 消费 \glueexpr
                    let g = self.eval_glue_expression(false)?;
                    let v = g.width;
                    return Ok(if neg { -v } else { v });
                }
                EqSlot::Primitive(Primitive::Muexpr) => {
                    self.fetch()?; // 消费 \muexpr
                    let g = self.eval_glue_expression(true)?;
                    let v = g.width;
                    return Ok(if neg { -v } else { v });
                }
                // 内部整数：\catcode<char> → 该字符当前 catcode 值
                EqSlot::Primitive(Primitive::Catcode) => {
                    self.fetch()?; // 消费 \catcode
                    let byte = self.scan_char_code()?;
                    let byte =
                        u8::try_from(byte).map_err(|_| Error::invalid_input("\\catcode 字符码越界"))?;
                    let v = i64::from(self.catcodes.get(byte).as_u8());
                    return Ok(if neg { -v } else { v });
                }
                // 内部只读整数：\badness → 最近盒子的 badness（当前恒 0：
                // 展开侧尚未跟踪盒排版 badness，trip.tex 第 20 行无盒子时为 0）。
                EqSlot::Primitive(Primitive::Badness) => {
                    self.fetch()?; // 消费 \badness
                    return Ok(0);
                }
                // 内部整数：\eTeXversion → 2（e-TeX 版本号，可作数字操作数）
                EqSlot::Primitive(Primitive::ETeXVersion) => {
                    self.fetch()?; // 消费 \eTeXversion
                    return Ok(if neg { -2 } else { 2 });
                }
                // ETRIP 冲刺：\lccode<char>：字符的小写码（数字上下文读取）
                EqSlot::Primitive(Primitive::LcCode) => {
                    self.fetch()?; // 消费 \lccode
                    let byte = self.scan_char_code()?;
                    let byte =
                        u8::try_from(byte).map_err(|_| Error::invalid_input("\\lccode 字符码越界"))?;
                    let v = self.lccodes[byte as usize];
                    return Ok(if neg { -v } else { v });
                }
                // ETRIP 冲刺：TeX/e-TeX 内部整数参数（\interactionmode/\language/\tracing* 等）
                EqSlot::Primitive(p) if int_param_index(p).is_some() => {
                    self.fetch()?; // 消费原语
                    let idx = int_param_index(p).expect("已检查 is_some");
                    let v = self.params.misc[idx];
                    return Ok(if neg { -v } else { v });
                }
                // TRIP：\mag（放大倍数，数字上下文读取；L160 `.5\mag` 等）
                EqSlot::Primitive(Primitive::Mag) => {
                    self.fetch()?; // 消费 \mag
                    let v = self.params.mag;
                    return Ok(if neg { -v } else { v });
                }
                EqSlot::Register(RegKind::Count, idx) => {
                    self.fetch()?; // 消费 cs
                    let v = self.registers.count(idx);
                    return Ok(if neg { -v } else { v });
                }
                EqSlot::Stream(_, n) => {
                    self.fetch()?; // 消费 cs
                    return Ok(if neg { -(n as i64) } else { n as i64 });
                }
                EqSlot::Primitive(Primitive::Count) => {
                    self.fetch()?; // 消费 \count
                    let idx = self.scan_register_index()?;
                    let v = self.registers.count(idx);
                    return Ok(if neg { -v } else { v });
                }
                // \count0=\dimen<idx>：尺寸以 sp 计的整数值（TeX scan_int 可读 \dimen）
                EqSlot::Primitive(Primitive::Dimen) => {
                    self.fetch()?; // 消费 \dimen
                    let idx = self.scan_register_index()?;
                    let v = self.registers.dimen(idx);
                    return Ok(if neg { -v } else { v });
                }
                // ETRIP：`\dimexpr1sp*\skip44` —— 胶水宽度（sp）作整数（TeX scan_int 可读 \skip）
                EqSlot::Primitive(Primitive::Skip) => {
                    self.fetch()?; // 消费 \skip
                    let idx = self.scan_register_index()?;
                    let v = self.registers.skip(idx).width;
                    return Ok(if neg { -v } else { v });
                }
                EqSlot::Primitive(Primitive::Muskip) => {
                    self.fetch()?; // 消费 \muskip
                    let idx = self.scan_register_index()?;
                    let v = self.registers.muskip(idx).width;
                    return Ok(if neg { -v } else { v });
                }
                EqSlot::Register(RegKind::Skip, idx) => {
                    self.fetch()?; // 消费 skipdef'd cs
                    let v = self.registers.skip(idx).width;
                    return Ok(if neg { -v } else { v });
                }
                EqSlot::Register(RegKind::Muskip, idx) => {
                    self.fetch()?; // 消费 muskipdef'd cs
                    let v = self.registers.muskip(idx).width;
                    return Ok(if neg { -v } else { v });
                }
                // \chardef\cs=<num>：数字上下文返回字符码（TeX scan_int）
                EqSlot::Char { charcode, .. } => {
                    self.fetch()?; // 消费 cs
                    let v = charcode as i64;
                    return Ok(if neg { -v } else { v });
                }
                // \inputlineno：当前输入行号（e-TeX；宏展开中为调用处行号——
                // etrip.tex \3 宏的 \typeout{...(l.\number\inputlineno)...} 需要）
                EqSlot::Primitive(Primitive::InputLineNo) => {
                    self.fetch()?; // 消费 \inputlineno
                    return Ok(if neg {
                        -(self.current_line_no() as i64)
                    } else {
                        self.current_line_no() as i64
                    });
                }
                // e-TeX 内部只读整数（数字上下文读取）
                EqSlot::Primitive(Primitive::CurrentGroupLevel) => {
                    self.fetch()?;
                    return Ok(self.group_level as i64);
                }
                // 组类型：sink 跟踪组种类（bottom=0 ... math_left=16）
                EqSlot::Primitive(Primitive::CurrentGroupType) => {
                    self.fetch()?;
                    return Ok(self.query_sink_ref().current_group_type());
                }
                // 最近节点类型：当前列表尾节点类型码（sink 查询；空列表 -1）
                EqSlot::Primitive(Primitive::LastNodeType) => {
                    self.fetch()?;
                    return Ok(self.query_sink_ref().last_node_type());
                }
                // e-TeX 只读整数：条件深度/种类/分支（数字上下文读取）
                EqSlot::Primitive(Primitive::CurrentIfLevel) => {
                    self.fetch()?;
                    return Ok(self.cond_stack.len() as i64);
                }
                EqSlot::Primitive(Primitive::CurrentIfType) => {
                    self.fetch()?;
                    // TeX 语义：`\if*` 遇到即置新类型（参数扫描期间即可读）；
                    // 负号 = `\unless` 前缀
                    return Ok(self.cur_if_type as i64);
                }
                EqSlot::Primitive(Primitive::CurrentIfBranch) => {
                    self.fetch()?;
                    // TeX 语义：0=未决/无、+1=true 分支、-1=false 分支（\else/\or 后）
                    return Ok(self.cur_if_branch as i64);
                }
                // \mathchardef 绑定：数字上下文返回数学字符码（TeX scan_int）
                EqSlot::MathChar(code) => {
                    self.fetch()?; // 消费 cs
                    return Ok(code as i64);
                }
                // ETRIP：\gluestretchorder/\glueshrinkorder<胶水> → 无穷阶（整数上下文）
                EqSlot::Primitive(Primitive::GlueStretchOrder) => {
                    self.fetch()?;
                    let g = self.scan_glue()?;
                    return Ok(if neg {
                        -(g.stretch_order as i64)
                    } else {
                        g.stretch_order as i64
                    });
                }
                EqSlot::Primitive(Primitive::GlueShrinkOrder) => {
                    self.fetch()?;
                    let g = self.scan_glue()?;
                    return Ok(if neg {
                        -(g.shrink_order as i64)
                    } else {
                        g.shrink_order as i64
                    });
                }
                // ETRIP 第二波：\lastpenalty → 当前列表尾 penalty 值（整数上下文）
                EqSlot::Primitive(Primitive::LastPenalty) => {
                    self.fetch()?;
                    let v = self.sink.last_penalty();
                    return Ok(if neg { -v } else { v });
                }
                // ETRIP 第二波：\prevdepth → 上一行 depth（sp；作为整数取其 sp 值）
                EqSlot::Primitive(Primitive::PrevDepth) => {
                    self.fetch()?;
                    let v = self.params.prevdepth;
                    return Ok(if neg { -v } else { v });
                }
                // TRIP：\parshape 在数字上下文返回段落形状行数（tex.web set_shape
                // 内部量；\hangindent- \parshape pt 的整数部分，l.244 误报修复）。
                EqSlot::Primitive(Primitive::Parshape) => {
                    self.fetch()?; // 消费 \parshape
                    let v = self.parshape.len() as i64;
                    return Ok(if neg { -v } else { v });
                }
                // TRIP：显示/页面 dimen 内部量在整数上下文按 sp 读取（tex.web
                // scan_something_internal(int_val)：dimen 转整数；\displayindent 有
                // 参数存储，其余为只读内部量（expander 无排版状态，暂 0——
                // 避免 l.252/253 误报 Missing number）。
                EqSlot::Primitive(Primitive::DisplayIndent) => {
                    self.fetch()?;
                    let v = self.params.displayindent;
                    return Ok(if neg { -v } else { v });
                }
                // TRIP：\mathsurround 是 dimen 参数——整数上下文按 sp 读取
                // （tex.web scan_something_internal(int_val) 的 dimen 转整数）。
                EqSlot::Primitive(Primitive::MathSurround) => {
                    self.fetch()?;
                    let v = self.params.mathsurround;
                    return Ok(if neg { -v } else { v });
                }
                // ETRIP/TRIP：\lastskip → 列表尾 glue 宽度（sp）；\lastkern → 尾 kern
                // 宽度（tex.web scan_something_internal；无则 0。l.305/318 误报修复）。
                EqSlot::Primitive(Primitive::LastSkip) => {
                    self.fetch()?;
                    let v = self.sink.last_skip();
                    return Ok(if neg { -v } else { v });
                }
                EqSlot::Primitive(Primitive::LastKern) => {
                    self.fetch()?;
                    let v = self.sink.last_kern();
                    return Ok(if neg { -v } else { v });
                }
                EqSlot::Primitive(
                    Primitive::DisplayWidth
                        | Primitive::PreDisplaySize
                        | Primitive::PageTotal
                        | Primitive::PageGoal,
                ) => {
                    self.fetch()?;
                    return Ok(0);
                }
                _ => {}
            }
        }
        // 单字符控制符号（`\^^J`、`\@` 等）在数字上下文取其字符码（TeX scan_int：
        // 控制符号名字为单个非字母字符时等价于该字符）。
        if let Some(code) = self.try_control_symbol()? {
            self.skip_trailing_spaces()?;
            return Ok(if neg { -code } else { code });
        }
        let mut val: i64 = 0;
        let mut any = false;
        while let Some((tok, _)) = self.fetch()? {
            // eTeX 表达式分组（{1+}{2*3} 的 {1+}）只属于 \numexpr 表达式因子层
            // （expr.rs expr_factor），scan_int 遇组字符 { 应报 Missing number
            // （tex.web scan_int 无分组分支；TRIP l.106 \number{ 漏报修复）。
            match digit_value(tok) {
                Some(d) => {
                    // TRIP：防 i64 溢出（超大整数钳制——TeX scan_int 同报错钳制）
                    if val < i64::MAX / 10 {
                        val = val * 10 + i64::from(d);
                    }
                    any = true;
                }
                None => {
                    self.unread(tok);
                    break;
                }
            }
        }
        if !any {
            // TeX scan_int：数字缺失 → "Missing number, treated as zero" 恢复
            // （`\countdef\countz` 等，trip.tex L28；token 已放回）
            self.report_missing_number();
            return Ok(0);
        }
        // TeX 规则：数字后跟随的空格被吞掉（实测 pdfTeX `\ifnum3>2 yes` → "yes"）
        self.skip_trailing_spaces()?;
        Ok(if neg { -val } else { val })
    }

    /// 单字符控制符号（非字母名字，如 `\^^J`）→ 字符码；否则不消费并返回 `None`。
    fn try_control_symbol(&mut self) -> Result<Option<i64>> {
        let Some((tok, _)) = self.fetch()? else {
            return Ok(None);
        };
        let Some(csid) = tok.csid() else {
            self.unread(tok);
            return Ok(None);
        };
        let name = self.intern.name(csid);
        if name.len() == 1 && !name.as_bytes()[0].is_ascii_alphabetic() {
            Ok(Some(name.as_bytes()[0] as i64))
        } else {
            self.unread(tok);
            Ok(None)
        }
    }

    /// 若下一 token 是反引号（cat 12、charcode 96），消费并按 TeX 规则返回其后的
    /// 字符码（`{ → 123、`- → 45、`\@ → 64、`\^^@ → 0）；否则不消费并返回 `None`。
    ///
    /// 反引号后可跟任意字符 token（取其字符码），或单字符控制符号（取其字符）；
    /// 控制词（如 `` `\par ``）报 "Improper alphabetic constant"（TeX 同规则）。
    fn try_scan_backquote(&mut self) -> Result<Option<i64>> {
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("扫描到输入末尾"))?
            .0;
        if tok.catcode() != Some(Catcode::Other) || tok.charcode() != Some(b'`' as u32) {
            self.unread(tok);
            return Ok(None);
        }
        let t2 = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("反引号后缺少字符"))?
            .0;
        match t2.kind() {
            TokenKind::Char => Ok(Some(t2.charcode().expect("Char 必有 charcode") as i64)),
            TokenKind::ControlSeq => {
                let name = self.intern.name(t2.csid().expect("ControlSeq 必有 csid"));
                if name.len() == 1 {
                    Ok(Some(name.as_bytes()[0] as i64))
                } else {
                    // TeX：反引号后多字符 cs → "! Improper alphabetic constant."
                    // 恢复插入 \0（TRIP L249 `\delcode`\relax`）。
                    let _ = self.sink.write16(
                        "! Improper alphabetic constant.\n\
                         A one-character control sequence belongs after a ` mark.\n\
                         So I'm essentially inserting \\0 here.\n"
                            .to_string(),
                    );
                    Ok(Some(0))
                }
            }
            _ => Err(Error::invalid_input("反引号后必须是字符或单字符控制序列")),
        }
    }

    /// 扫描字符码（`\catcode`/`\sfcode` 的左操作数，TeX `scan_char_num`）：
    /// 十进制整数或 `` `X `` 反引号形式。
    fn scan_char_code(&mut self) -> Result<i64> {
        self.skip_spaces()?;
        if let Some(code) = self.try_scan_backquote()? {
            return Ok(code);
        }
        self.scan_number()
    }

    /// 跳过前导空格 token（输入耗尽视为合法，返回 Ok）。
    fn skip_spaces(&mut self) -> Result<()> {
        loop {
            let Some((tok, _)) = self.fetch()? else { return Ok(()) };
            if tok.catcode() != Some(Catcode::Space) {
                self.unread(tok);
                return Ok(());
            }
        }
    }

    /// 吞掉数字/尺寸后的尾随空格（输入耗尽时直接返回）。
    fn skip_trailing_spaces(&mut self) -> Result<()> {
        loop {
            match self.fetch()? {
                None => return Ok(()),
                Some((tok, _)) => {
                    if tok.catcode() == Some(Catcode::Space) {
                        continue;
                    }
                    self.unread(tok);
                    return Ok(());
                }
            }
        }
    }

    /// 可选赋值符 `=`（TeX：`=` 在赋值中可省略，如 `\catcode`X 13`），允许前后空格。
    fn expect_equals(&mut self) -> Result<()> {
        self.skip_spaces()?;
        let Some((tok, _)) = self.fetch()? else {
            return Ok(());
        };
        if tok.charcode() == Some(b'=' as u32) {
            return Ok(());
        }
        self.unread(tok);
        Ok(())
    }

    /// 扫描被定义的 csname（`\chardef\cs=...` 等），返回 csid。
    fn scan_cs_ident(&mut self) -> Result<u32> {
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("缺少控制序列"))?
            .0;
        tok.csid().map(Ok).unwrap_or_else(|| {
            // TeX：`\mathchardef A`（非 cs）→ "! Missing control sequence inserted."
            // 恢复（插入 \inaccessible 完成定义；TRIP L298）。**被拒 token 放回输入**
            // （TeX back_input：`<to be read again> {`），参数文本扫描从 `{` 重新开始。
            let _ = self.sink.write16(
                "! Missing control sequence inserted.\n\
                 Please don't say `\\def cs{...}', say `\\def\\cs{...}'.\n\
                 I've inserted an inaccessible control sequence so that your\n\
                 definition will be completed without mixing me up too badly.\n"
                    .to_string(),
            );
            self.unread(tok);
            Ok(self.intern.intern("\u{0}inaccessible"))
        })
    }

    /// 注册 M1 内建原语。
    fn scan_register_target(&mut self, kind: RegKind) -> Result<usize> {
        self.skip_spaces()?;
        let t = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("预期寄存器下标"))?
            .0;
        if let Some(csid) = t.csid() {
            match self.eqtb.slot(csid) {
                EqSlot::Register(k, idx) if *k == kind => Ok(*idx),
                _ => Err(Error::invalid_input(
                    "寄存器未分配（先 \\newcount 等分配）",
                )),
            }
        } else {
            self.unread(t);
            self.scan_register_index()
        }
    }

    /// 扫描寄存器下标（eTeX 0..=32767；越界报 "! Bad register code." 并钳到 0，
    /// etrip "Checking sparse arrays" 段：`\countdef\cs=32768` / `=-1`）。
    fn scan_register_index(&mut self) -> Result<usize> {
        // tex.web scan_register_code → scan_int：不跳 `=`（`\setbox=` 的 `=` 不是
        // 合法下标，应报 Missing number；赋值符由调用方 expect_equals 消费）。
        let n = self.scan_number_inner(false)?;
        if !(0..REGISTER_COUNT as i64).contains(&n) {
            let _ = self.sink.write16(format!(
                "! Bad register code ({}).\n\
                 A register number must be between 0 and 32767.\n\
                 I changed this one to zero.\n",
                n
            ));
            return Ok(0);
        }
        Ok(n as usize)
    }

    /// `\toks<n>` 赋值 RHS（TeX `toks_register` 分支，tex.web L22945-22979）：
    /// RHS 可为 `{<token list>}`（scan_toks），也可为另一个 toks 寄存器
    /// （`\toks<n>` 或 `\toksdef` 命名的 cs，如 TRIP L418 `\tokens\toks1`，
    /// 无 `=` 经 `scan_optional_equals`）——此时**内容复制**；空源寄存器 →
    /// 空 token 列表（TeX 定义为 undefined_cs/null，`\the` 输出空，等价）。
    fn scan_toks_rhs(&mut self) -> Result<TokenArray> {
        loop {
            self.skip_spaces()?;
            let fetched = self.fetch()?;
            let Some((tok, _)) = fetched else {
                // 输入耗尽：TeX 报 "Missing { inserted" 后以空 token list 收尾
                return Ok(Arc::from(Vec::<Token>::new()));
            };
            // TeX `@<Get the next non-blank non-relax non-call token@>`：
            // `\relax` 分隔跳过（`\hyphenation\relax{...}` 既有处理同款）。
            if let Some(csid) = tok.csid() {
                if matches!(self.eqtb.slot(csid), EqSlot::Primitive(Primitive::Relax)) {
                    continue;
                }
            }
            // 组开始 → scan_toks 收集（本实现用 scan_group_contents，
            // 它入口自行 fetch 组开始 token，故先放回）
            if tok.catcode() == Some(Catcode::BeginGroup) {
                self.unread(tok);
                let val = self.scan_group_contents(Some("toks"))?;
                return Ok(Arc::from(val));
            }
            // toks 寄存器内容复制
            if let Some(idx) = self.toks_rhs_index(tok)? {
                return Ok(self.registers.toks(idx));
            }
            return Err(Error::invalid_input(
                "\\toks 赋值 RHS 需为 {token list} 或 toks 寄存器",
            ));
        }
    }

    /// 判断 RHS token 是否为 toks 寄存器（`\toks<n>` 或 `\toksdef` cs），
    /// 返回寄存器下标；否则返回 None。
    fn toks_rhs_index(&mut self, tok: Token) -> Result<Option<usize>> {
        let Some(csid) = tok.csid() else { return Ok(None) };
        match self.eqtb.slot(csid) {
            EqSlot::Register(RegKind::Toks, idx) => Ok(Some(*idx)),
            EqSlot::Primitive(Primitive::Toks) => {
                // `\toks<n>`：token 形式即 `\toks`+`1`（\toks 原语后跟数字），
                // 原语 token 已被 fetch，直接扫描其后的寄存器下标即可——
                // 不可 unread，否则 scan_number 会 fetch 到 `\toks` 本身
                // 报 "Missing number" 返回 0，数字与后续 token 全部错位。
                Ok(Some(self.scan_register_index()?))
            }
            _ => Ok(None),
        }
    }

    /// 扫描平衡花括号内的 token 列表（`\toks0={...}` 用）。
    ///
    /// TeX `scan_toks(macro, xpand)` 恢复语义：
    /// - **输入耗尽未配平** → 转录报告 "Runaway text?" 并以隐含 `}` 收尾返回已收集
    ///   tokens（可恢复，不报错；TeX runaway）；
    /// - `forbidden` 为 `Some(cs 名)` 时（`\toks`/`\output`/`\every...` 赋值上下文），
    ///   实参中出现的 **outer 宏** → forbidden：报 "Runaway text?" + "! Forbidden
    ///   control sequence found while scanning text of \X."，插入 `}` 结束扫描、
    ///   offending cs 放回输入流（TRIP L354 `\tokens{\a^^@^^@a\par!`）。
    fn scan_group_contents(&mut self, forbidden: Option<&str>) -> Result<Vec<Token>> {
        self.skip_spaces()?; // TeX scan_general_text：跳过 = 后的空格再读组（etrip L1178 \output = {）
        let fetched = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("扫描到输入末尾"))?
            .0;
        let open = self.resolve_group_char(fetched);
        if open.catcode() != Some(Catcode::BeginGroup) {
            // TeX scan_left_brace（tex.web L692-696）：非 { → 报
            // "Missing { inserted."（token 放回、隐含 `{` 恢复继续），
            // 不再硬错误（trip.tex L396 `\accent\x\vfill` 等 20 处依赖此恢复）
            self.unread(open);
            let _ = self.sink.write16(
                "! Missing { inserted.\n\
                 A left brace was mandatory here, so I've put one in.\n\
                 You might want to delete and/or insert some corrections\n\
                 so that I will find a matching right brace soon.\n\
                 (If you're confused by all this, try typing `I}' now.)\n"
                    .to_string(),
            );
            self.report_error_context();
        }
        let mut tokens = Vec::new();
        let mut depth = 0usize;
        loop {
            let Some((fetched, _)) = self.fetch()? else {
                // 输入耗尽未配平：TeX "Runaway text?" 恢复（补隐含 }）
                let _ = self.sink.write16("Runaway text?\n".to_owned());
                return Ok(tokens);
            };
            let t = self.resolve_group_char(fetched);
            // outer 宏 forbidden（仅 \toks 类赋值上下文）
            if let Some(name) = forbidden {
                if let Some(csid) = t.csid() {
                    if let EqSlot::Macro(m) = self.eqtb.slot(csid) {
                        if m.value.outer {
                            let csname = self.intern.name(csid);
                            let _ = self.sink.write16(format!(
                                "Runaway text?\n\
                                 ! Forbidden control sequence found while scanning text of \\{name}.\n\
                                 <inserted text>\n                }}\n\
                                 <to be read again>\n                   \\{csname}\n"
                            ));
                            self.unread(t);
                            // TeX 语义：Forbidden 时报错并放弃整个赋值
                            // （scan_toks 返回空列表），否则部分写入 toks 寄存器会
                            // 在 `\the\tokens` 时被重新展开造成死循环（TRIP L417）。
                            return Ok(Vec::new());
                        }
                    }
                }
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
        Ok(tokens)
    }

    /// TeX `\mathchoice` 分支扫描（tex.web `scan_left_brace` + `scan_balanced_group`
    /// 语义）：每个分支强制以 `{` 开头（跳过前导空格）；非 `{` → 报
    /// "Missing { inserted." 并把 token 放回、隐含插入 `{` 后收集到下一个 `}`
    /// （`}` 消费），与 build_choices 逐分支划分一致（TRIP L438
    /// `\mathchoice{}a}{A|{}}{\mathchoice}` → 分支 `{}`/`a`/`A|{}`/`\mathchoice`）。
    /// 返回收集到的分支 token（内容不执行）。
    fn scan_mathchoice_branch(&mut self) -> Result<Vec<Token>> {
        self.skip_spaces()?;
        let fetched = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\mathchoice 分支扫描到输入末尾"))?
            .0;
        let t = self.resolve_group_char(fetched);
        if t.catcode() == Some(Catcode::BeginGroup) {
            self.unread(t);
            return self.scan_group_contents(None);
        }
        // 非 `{`：TeX scan_left_brace 报 "Missing { inserted."，token 放回、隐含 `{`
        self.unread(t);
        let _ = self.sink.write16(
            "! Missing { inserted.\n\
             A left brace was mandatory here, so I've put one in.\n\
             You might want to delete and/or insert some corrections\n\
             so that I will find a matching right brace soon.\n\
             (If you're confused by all this, try typing `I}' now.)\n"
                .to_string(),
        );
        self.report_error_context();
        // 隐含 `{` 后按平衡组收集到下一个 `}`（`}` 消费）
        let mut tokens = Vec::new();
        let mut depth = 0usize;
        loop {
            let Some((fetched, _)) = self.fetch()? else {
                let _ = self.sink.write16("Runaway text?\n".to_owned());
                return Ok(tokens);
            };
            let tt = self.resolve_group_char(fetched);
            match tt.catcode() {
                Some(Catcode::BeginGroup) => {
                    depth += 1;
                    tokens.push(tt);
                }
                Some(Catcode::EndGroup) => {
                    if depth == 0 {
                        return Ok(tokens);
                    }
                    depth -= 1;
                    tokens.push(tt);
                }
                _ => tokens.push(tt),
            }
        }
    }

    /// `\let\bgroup={`/`\let\egroup=}` 别名解析：绑定为组定界符字符的 cs → 底层字符 token。
    fn resolve_group_char(&self, tok: Token) -> Token {
        let Some(csid) = tok.csid() else {
            return tok;
        };
        if let EqSlot::Char { catcode, charcode } = self.eqtb.slot(csid) {
            if matches!(catcode, Catcode::BeginGroup | Catcode::EndGroup) {
                return Token::char(*catcode, *charcode);
            }
        }
        tok
    }

    /// TeX `<general text>` 扫描（`\unexpanded`/`\detokenize` 参数）：
    /// 先展开可展开项（`\expandafter`/宏/可展开原语）；组开始 `{` 后按平衡组
    /// 收集（组内不展开）。general text 语义：不可展开 token 原样收集，
    /// `\relax` 或外层 `}` 终止；平衡组在输入耗尽时补 `}` 收尾（TeX 语义，
    /// 如 `\unexpanded\expandafter{\1}` 中 `\1` 展开含不平衡花括号）。
    fn scan_group_contents_expanding(&mut self) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();
        loop {
            let open = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("扫描到输入末尾"))?
                .0;
            let open = self.resolve_group_char(open);
            // 组开始：转平衡组收集
            if open.catcode() == Some(Catcode::BeginGroup) {
                self.unread(open);
                break;
            }
            let Some(csid) = open.csid() else {
                tokens.push(open);
                continue;
            };
            match self.eqtb.slot(csid).clone() {
                // \let\bgroup={`：cs 绑定为组定界符 → 展开成该字符
                EqSlot::Char {
                    catcode: Catcode::BeginGroup,
                    charcode,
                } => {
                    self.unread(Token::char(Catcode::BeginGroup, charcode));
                    break;
                }
                // \relax：general text 终止（TeX scan_general_text）
                EqSlot::Primitive(Primitive::Relax) => return Ok(tokens),
                EqSlot::Alias(_) => {
                    // 沿别名链解引用（\let\bgroup={ 是 Char 不会到这；\let\1=\5 会）。
                    // 若 unread 原 alias token 再 continue 会无限循环。
                    let mut id = csid;
                    let mut depth = 0;
                    while let EqSlot::Alias(t) = self.eqtb.slot(id) {
                        id = *t;
                        depth += 1;
                        if depth > 100 {
                            return Err(Error::invalid_input("\\let 别名环"));
                        }
                    }
                    self.unread(Token::control_sequence(id));
                    continue;
                }
                // protected 宏在展开抑制上下文（\edef/\write）不展开 → 视为不可展开
                EqSlot::Macro(m)
                    if !(m.value.protected && self.suppress_expansion > 0) =>
                {
                    let mut expansion = Vec::new();
                    self.expand_once((open, false), &mut expansion)?;
                    let items: Vec<(Token, bool)> = expansion.into_iter().collect();
                    self.stack.push(InputFrame::TokenList {
                        items: Arc::from(items),
                        pos: 0,
                    });
                    continue;
                }
                EqSlot::Primitive(p) if p.is_expandable() => {
                    let mut expansion = Vec::new();
                    self.expand_once((open, false), &mut expansion)?;
                    let items: Vec<(Token, bool)> = expansion.into_iter().collect();
                    self.stack.push(InputFrame::TokenList {
                        items: Arc::from(items),
                        pos: 0,
                    });
                    continue;
                }
                // 不可展开 cs：原样收集（general text 语义）
                _ => {
                    tokens.push(open);
                    continue;
                }
            }
        }
        // 平衡组收集：先消费组开始 `{`（定界符，不计入内容），收集到匹配的 `}`。
        // EOF 容忍（TeX 输入耗尽时补 } 收尾）。
        if self.fetch()?.is_none() {
            return Ok(tokens);
        }
        let mut depth = 0usize;
        while let Some((fetched, _)) = self.fetch()? {
            let t = self.resolve_group_char(fetched);
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
        Ok(tokens)
    }

    /// 扫描一个尺寸：数字（含小数）+ 可选单位；支持 `\dimen<idx>` 引用。
    ///
    /// 换算对照 pdfTeX：`scaled = (int + frac/10^k) * unit_sp`（逐项截断）。
    fn scan_dimen(&mut self) -> Result<i64> {
        let (v, _) = self.scan_dimen_inner(false, false)?;
        Ok(self.clamp_dimen(v))
    }

    /// mu 上下文尺寸扫描（`\muskip`/`\muexpr`/`\mskip` 项）：只认 "mu" 单位，
    /// 其他单位（含无单位、fil 阶）→ "! Illegal unit of measure (mu inserted)."
    /// （tex.web L8987-8997 语义）。
    fn scan_dimen_mu(&mut self) -> Result<i64> {
        let (v, _) = self.scan_dimen_inner(true, false)?;
        Ok(self.clamp_dimen(v))
    }

    /// 单位非法错误（tex.web L8990-8998）：mu 上下文只认 "mu" 单位，
    /// 其他单位词/无单位 → "(mu inserted)" 恢复（值按 mu、词放回）。
    /// help 行（tex.web L8992-8995）在 etrip.log 收尾比对阶段统一补。
    fn report_bad_unit(&mut self, mu: bool) {
        if mu {
            let _ = self.sink.write16(
                "! Illegal unit of measure (mu inserted).\nThe unit of measurement in math glue must be mu.\n"
                    .to_string(),
            );
        } else {
            self.report_error("Illegal unit of measure (pt inserted).");
        }
    }

    /// TeX scan_dimen 末尾的尺寸钳制：|v| > 0x3FFFFFFF →
    /// "! Dimension too large." 并钳到 ±MAX_DIMEN（如 `\dimen45=\skip44` 读超大胶水）。
    /// help 行（tex.web L8992-8995）在 etrip.log 收尾比对阶段统一补。
    fn clamp_dimen(&mut self, v: i64) -> i64 {
        let too_large = |e: &mut Self| {
            e.report_error("Dimension too large.");
            let _ = e.sink.write16(
                "I can't work with sizes bigger than about 19 feet.\n\
                 Continue and I'll use the largest value I can.\n"
                    .to_string(),
            );
        };
        if v > MAX_DIMEN {
            too_large(self);
            MAX_DIMEN
        } else if v < -MAX_DIMEN {
            too_large(self);
            -MAX_DIMEN
        } else {
            v
        }
    }

    /// 尺寸扫描（含胶水无穷阶）：返回 `(值, 阶)`。`scan_dimen` 丢弃阶；
    /// `scan_glue` 的 plus/minus 值用它取阶（TeX：`1pt plus 3fill`）。
    ///
    /// 参数（tex.web scan_dimen 语义）：
    /// - `mu`：mu 上下文——合法单位仅 "mu"（其他单位/无单位/fil 阶 → "(mu inserted)"）；
    ///   pt 上下文中 "mu" 单位不合法（→ "(pt inserted)"）。
    /// - `inf`：是否允许 fil/fill/filll 阶词（glue 的 width 不允许，stretch/shrink 允许）。
    fn scan_dimen_inner(&mut self, mu: bool, inf: bool) -> Result<(i64, u8)> {
        self.skip_spaces()?;
        // 报错锚点：值扫描起始位置（clamp_dimen 报错时 pos 已推进——回溯用）
        for frame in self.stack.iter().rev() {
            if let InputFrame::Source { pos, .. } = frame {
                self.error_anchor = Some(*pos);
                break;
            }
        }
        // TeX scan_dimen：跳过可选 `=` 赋值符（`\hsize=5in` 与 `\hsize 5in` 等价）
        if let Some((tok, _)) = self.fetch()? {
            if tok.charcode() != Some(b'=' as u32) {
                self.unread(tok);
            }
        }
        // 连续负号循环（TeX 表达式 `--\skip90` 等）+ 未定义 cs 跳过
        // （`-\mutoglue-\gluetomu9pt`，报错当 \relax 继续）：奇偶决定符号。
        // TeX get_x_token 语义：可展开 cs 展开后压回输入流顶并**重新进入符号处理**
        // （`\t` 展开 `-.01001010pt` 以 `-` 开头，TRIP L161）。
        let mut neg = false;
        loop {
            self.skip_spaces()?;
            let Some((tok, ne)) = self.fetch()? else {
                break;
            };
            if tok.charcode() == Some(b'-' as u32) {
                neg = !neg;
                continue;
            }
            // TeX scan_dimen：正号忽略（`\varunit=+1,001...`，TRIP L160）。
            // 但 \glueexpr 表达式里 {7pt+} 的 + 是运算符（+ 后非数字）——放回由
            // 表达式循环（peek_int_op）处理；仅 + 后跟数字时才是正号（etrip L888
            // `\glueexpr{7pt+}{12pt/4}` = 7pt + 12pt/4）。
            if tok.charcode() == Some(b'+' as u32) {
                let next = self.fetch()?;
                match next {
                    Some((n, _)) if n.charcode().is_some_and(|c| (c as u8).is_ascii_digit()) => {
                        continue;
                    }
                    _ => {
                        if let Some((n, _)) = next {
                            self.unread(n);
                        }
                        self.unread(tok);
                        break;
                    }
                }
            }
            if let Some(csid) = tok.csid() {
                if matches!(self.eqtb.slot(csid), EqSlot::Undefined) {
                    let _ = self
                        .sink
                        .write16(format!(
                            "! Undefined control sequence.\n\\{}\n",
                            self.intern.name(csid)
                        ));
                    continue;
                }
                // TRIP：条件原语在尺寸中先求值（TeX get_x_token 嵌套条件）
                if self.maybe_eval_cond(tok)? {
                    continue;
                }
                // 可展开 cs（宏/可展开原语）：展开**当前** token（第一次 fetch 的），
                // 结果压栈后重新符号处理（\t 展开 `-.01001010pt` 以 `-` 开头，TRIP L161）。
                let expandable = match self.eqtb.slot(csid).clone() {
                    EqSlot::Macro(m) => !(m.value.protected && self.suppress_expansion > 0),
                    EqSlot::Primitive(p) if p.is_expandable() => true,
                    _ => false,
                };
                if expandable {
                    let mut expansion = Vec::new();
                    self.expand_once((tok, ne), &mut expansion)?;
                    if !expansion.is_empty() {
                        self.stack.push(InputFrame::TokenList {
                            items: Arc::from(expansion),
                            pos: 0,
                        });
                    }
                    continue;
                }
            }
            self.unread(tok);
            break;
        }
        // 寄存器引用：\dimen<idx>
        if let Some(csid) = self.peek_csid()? {
            if let EqSlot::Primitive(Primitive::Dimen) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \dimen
                let idx = self.scan_register_index()?;
                let v = self.registers.dimen(idx);
                return Ok((if neg { -v } else { v }, 0));
            }
            // ETRIP：`\dimen45=\skip44` —— 胶水寄存器的宽度作尺寸（TeX scan_dimen 语义）
            if let EqSlot::Primitive(Primitive::Skip) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \skip
                let idx = self.scan_register_index()?;
                let v = self.registers.skip(idx).width;
                return Ok((if neg { -v } else { v }, 0));
            }
            if let EqSlot::Primitive(Primitive::Muskip) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \muskip
                let idx = self.scan_register_index()?;
                let v = self.registers.muskip(idx).width;
                return Ok((if neg { -v } else { v }, 0));
            }
            if let EqSlot::Register(RegKind::Skip, idx) = self.eqtb.slot(csid).clone() {
                self.fetch()?; // 消费 skipdef'd cs
                let v = self.registers.skip(idx).width;
                return Ok((if neg { -v } else { v }, 0));
            }
            if let EqSlot::Register(RegKind::Muskip, idx) = self.eqtb.slot(csid).clone() {
                self.fetch()?; // 消费 muskipdef'd cs
                let v = self.registers.muskip(idx).width;
                return Ok((if neg { -v } else { v }, 0));
            }
            // M4-5 e-TeX：\dimexpr 可在任意尺寸上下文求值
            if let EqSlot::Primitive(Primitive::Dimexpr) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \dimexpr
                let v = self.eval_dimen_expression()?;
                return Ok((if neg { -v } else { v }, 0));
            }
            // e-TeX：\glueexpr/\muexpr 宽度可在尺寸上下文求值（etrip L888 `\ifdim\glueexpr...`）
            if let EqSlot::Primitive(Primitive::Glueexpr) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \glueexpr
                let g = self.eval_glue_expression(false)?;
                let v = g.width;
                return Ok((if neg { -v } else { v }, 0));
            }
            if let EqSlot::Primitive(Primitive::Muexpr) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \muexpr
                let g = self.eval_glue_expression(true)?;
                let v = g.width;
                return Ok((if neg { -v } else { v }, 0));
            }
            // ETRIP 冲刺：\fontdimen<num><font> 可在任意尺寸上下文读取
            if let EqSlot::Primitive(Primitive::FontDimen) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \fontdimen
                let num = self.scan_number()?;
                let num =
                    u32::try_from(num).map_err(|_| Error::invalid_input("\\fontdimen 参数号越界"))?;
                let font = self.scan_font_ident()?;
                let v = self.fontdimen(font, num);
                return Ok((if neg { -v } else { v }, 0));
            }
            // ETRIP 冲刺：\gluestretch/\glueshrink<胶水> → 胶水分量（尺寸上下文）
            if let EqSlot::Primitive(Primitive::GlueStretch) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \gluestretch
                let g = self.scan_glue()?;
                return Ok((if neg { -g.stretch } else { g.stretch }, 0));
            }
            if let EqSlot::Primitive(Primitive::GlueShrink) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \glueshrink
                let g = self.scan_glue()?;
                return Ok((if neg { -g.shrink } else { g.shrink }, 0));
            }
            // ETRIP 冲刺：\fontcharwd/ht/dp/ic<font><char> → 字符度量分量（尺寸上下文）
            if let EqSlot::Primitive(
                Primitive::FontCharWd | Primitive::FontCharHt | Primitive::FontCharDp | Primitive::FontCharIc,
            ) = self.eqtb.slot(csid)
            {
                let component = match self.eqtb.slot(csid) {
                    EqSlot::Primitive(Primitive::FontCharWd) => 0,
                    EqSlot::Primitive(Primitive::FontCharHt) => 1,
                    EqSlot::Primitive(Primitive::FontCharDp) => 2,
                    _ => 3, // FontCharIc
                };
                self.fetch()?; // 消费 \fontchar*
                let font = self.scan_font_ident()?;
                let ch = self.scan_number()?;
                if !(0..=255).contains(&ch) {
                    self.report_error("Bad character code.");
                    return Ok((0, 0));
                }
                let m = self.font_loader.char_metric(font, ch as u32);
                let v = match component {
                    0 => m.map(|x| x.0).unwrap_or(0),
                    1 => m.map(|x| x.1).unwrap_or(0),
                    2 => m.map(|x| x.2).unwrap_or(0),
                    _ => 0,
                };
                return Ok((if neg { -v } else { v }, 0));
            }
            // ETRIP 冲刺：\parshapelength/indent/dimen<n> → 段落形状分量（尺寸上下文）
            if let EqSlot::Primitive(
                Primitive::ParshapeLength | Primitive::ParshapeIndent | Primitive::ParshapeDimen,
            ) = self.eqtb.slot(csid)
            {
                let kind = match self.eqtb.slot(csid) {
                    EqSlot::Primitive(Primitive::ParshapeIndent) => 0,
                    EqSlot::Primitive(Primitive::ParshapeLength) => 1,
                    _ => 2,
                };
                self.fetch()?; // 消费 \parshape*
                let idx = self.scan_number()?;
                let v = self.parshape_access(idx, kind);
                return Ok((if neg { -v } else { v }, 0));
            }
            // ETRIP 第二波：\wd/\ht/\dp<n> → 盒子寄存器尺寸（尺寸上下文）
            if let EqSlot::Primitive(Primitive::Wd | Primitive::Ht | Primitive::Dp) =
                self.eqtb.slot(csid)
            {
                let dim = match self.eqtb.slot(csid) {
                    EqSlot::Primitive(Primitive::Wd) => 0,
                    EqSlot::Primitive(Primitive::Ht) => 1,
                    _ => 2,
                };
                self.fetch()?; // 消费 \wd/\ht/\dp
                let idx = self.scan_register_index()?;
                let v = self.sink.box_dim(idx, dim);
                return Ok((if neg { -v } else { v }, 0));
            }
            // ETRIP 第二波：\prevdepth → 上一行 depth（尺寸上下文）
            if let EqSlot::Primitive(Primitive::PrevDepth) = self.eqtb.slot(csid) {
                self.fetch()?;
                let v = self.params.prevdepth;
                return Ok((if neg { -v } else { v }, 0));
            }
            // TRIP 冲刺：dimen 内部参数作尺寸（\ifdim\hsize<\hsize、\the\hsize 等）
            if matches!(
                self.eqtb.slot(csid),
                EqSlot::Primitive(
                    Primitive::HSize
                        | Primitive::ParIndent
                        | Primitive::VSize
                        | Primitive::MaxDepth
                        | Primitive::LineSkipLimit
                        // TRIP：\displayindent 同为 dimen 参数（l.251/252 误报修复）
                        | Primitive::DisplayIndent
                        // TRIP：\mathsurround 是 dimen 参数（l.260 `.11em` 赋值）
                        | Primitive::MathSurround
                )
            ) {
                self.fetch()?;
                let v = match self.eqtb.slot(csid) {
                    EqSlot::Primitive(Primitive::HSize) => self.params.hsize,
                    EqSlot::Primitive(Primitive::ParIndent) => self.params.parindent,
                    EqSlot::Primitive(Primitive::VSize) => self.params.vsize,
                    EqSlot::Primitive(Primitive::MaxDepth) => self.params.maxdepth,
                    EqSlot::Primitive(Primitive::DisplayIndent) => self.params.displayindent,
                    EqSlot::Primitive(Primitive::MathSurround) => self.params.mathsurround,
                    _ => self.params.lineskiplimit,
                };
                return Ok((if neg { -v } else { v }, 0));
            }
            // TRIP：只读显示/页面内部量作尺寸（\predisplaysize/\displaywidth/\pagetotal/
            // \pagegoal）——expander 无排版状态，暂按 0 读，避免 l.253 误报 Missing number。
            if matches!(
                self.eqtb.slot(csid),
                EqSlot::Primitive(
                    Primitive::DisplayWidth
                        | Primitive::PreDisplaySize
                        | Primitive::PageTotal
                        | Primitive::PageGoal
                )
            ) {
                self.fetch()?;
                return Ok((0, 0));
            }
            // ETRIP/TRIP：\lastskip/\lastkern 作尺寸（sp 值；无则 0）。
            if let EqSlot::Primitive(Primitive::LastSkip) = self.eqtb.slot(csid) {
                self.fetch()?;
                let v = self.sink.last_skip();
                return Ok((if neg { -v } else { v }, 0));
            }
            if let EqSlot::Primitive(Primitive::LastKern) = self.eqtb.slot(csid) {
                self.fetch()?;
                let v = self.sink.last_kern();
                return Ok((if neg { -v } else { v }, 0));
            }
            // TRIP 冲刺：glue 内部参数作尺寸（宽度分量）——`minus\baselineskip` 等
            if matches!(
                self.eqtb.slot(csid),
                EqSlot::Primitive(
                    Primitive::BaselineSkip
                        | Primitive::LineSkip
                        | Primitive::ParSkip
                        | Primitive::ParFillSkip
                        | Primitive::TopSkip
                        | Primitive::XSpaceSkip
                )
            ) {
                self.fetch()?;
                let g = match self.eqtb.slot(csid) {
                    EqSlot::Primitive(Primitive::BaselineSkip) => self.params.baselineskip,
                    EqSlot::Primitive(Primitive::LineSkip) => self.params.lineskip,
                    EqSlot::Primitive(Primitive::ParSkip) => self.params.parskip,
                    EqSlot::Primitive(Primitive::ParFillSkip) => self.params.parfillskip,
                    EqSlot::Primitive(Primitive::XSpaceSkip) => self.params.xspaceskip,
                    _ => self.params.topskip,
                };
                return Ok((if neg { -g.width } else { g.width }, 0));
            }
            // ETRIP 第二波：\mutoglue<mu 胶水> / \gluetomu<胶水> → 胶水宽度（尺寸上下文），
            // 转换为胶水后取 width 分量（1mu = 1pt = 65536sp，数值不变）。
            if let EqSlot::Primitive(Primitive::MuToGlue) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \mutoglue：输入 mu 胶水
                let g = self.scan_glue_mu()?;
                return Ok((if neg { -g.width } else { g.width }, 0));
            }
            if let EqSlot::Primitive(Primitive::GlueToMu) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \gluetomu：输入 pt 胶水
                let g = self.scan_glue()?;
                return Ok((if neg { -g.width } else { g.width }, 0));
            }
        }
        // 基数前缀：十六进制 `"` / 八进制 `'`（TeX scan_dimen 同 scan_int，TeXbook p.267）
        let mut radix_val: Option<i64> = None;
        // TRIP：反引号字符常量（TeX scan_dimen 的 alphabetic constant）：`<char> →
        // 字符码作整数部分，后随单位正常扫描（trip.tex L83 `\ifdim1,0pt<`^^Abpt`：
        // `` ` `` + ^^A(字符1) → 1，单位 bpt 取最长前缀 bp，剩余 `t` 留在流中）。
        if let Some(code) = self.try_scan_backquote()? {
            radix_val = Some(code);
        }
        if let Some((tok, _)) = self.fetch()? {
            let base = match (tok.catcode(), tok.charcode()) {
                (Some(Catcode::Other), Some(c)) if c == b'"' as u32 => Some(16u32),
                (Some(Catcode::Other), Some(c)) if c == b'\'' as u32 => Some(8u32),
                _ => None,
            };
            if let Some(base) = base {
                let mut val: i64 = 0;
                let mut any_radix = false;
                while let Some((t, _)) = self.fetch()? {
                    match radix_digit_value(t, base) {
                        Some(d) => {
                            val = val * i64::from(base) + i64::from(d);
                            any_radix = true;
                        }
                        None => {
                            self.unread(t);
                            break;
                        }
                    }
                }
                if !any_radix {
                    // TeX：基数前缀无数位 → "Missing number" 恢复 0（\leftskip \parshape pt）
                    self.report_missing_number();
                    return Ok((0, 0));
                }
                radix_val = Some(val);
            } else {
                self.unread(tok);
            }
        }
        // 数字：整数部分 + 可选小数
        let mut int_part: i64 = 0;
        let mut frac: i64 = 0;
        let mut frac_len: u32 = 0;
        let mut any = false;
        let mut saw_dot = false;
        if let Some(rv) = radix_val {
            int_part = rv;
            any = true;
        } else {
            // 数字部分可为寄存器/内部整数：`\count43pt`（TeX scan_dimen 的
            // <number><unit>，etrip L869 `\dimexpr\skip43+\count43pt`）。
            // 符号已由上方 multi-minus 处理，此处只取数值。
            let mut number_cs = false;
            if let Some(csid) = self.peek_csid()? {
                let slot = self.eqtb.slot(csid).clone();
                number_cs = matches!(
                    &slot,
                    EqSlot::Register(RegKind::Count, _) | EqSlot::Char { .. }
                ) || matches!(
                    &slot,
                    EqSlot::Primitive(p)
                        if matches!(
                            p,
                            Primitive::Count
                                | Primitive::NumExpr
                                // TRIP：内部整数读取原语作数字（\catcode`\} 等）
                                | Primitive::Catcode
                                | Primitive::LcCode
                                | Primitive::Badness
                                | Primitive::ETeXVersion
                                | Primitive::ETeXRevision
                                | Primitive::InputLineNo
                                // TRIP：\parshape 作 dimen 的整数部分（\hangindent- \parshape pt）
                                | Primitive::Parshape
                                | Primitive::CurrentGroupLevel
                                | Primitive::CurrentGroupType
                                | Primitive::LastNodeType
                                | Primitive::CurrentIfLevel
                                | Primitive::CurrentIfType
                                | Primitive::CurrentIfBranch
                        ) || int_param_index(*p).is_some()
                );
            }
            if number_cs {
                int_part = self.scan_number()?;
                any = true;
            } else {
                while let Some((tok, _)) = self.fetch()? {
                    // eTeX 表达式分组（{7pt+}{12pt/4} 的 {7pt+}）只属于 \dimexpr 因子层
                    // （expr.rs dimen_expr_term），scan_dimen 遇组字符 { 应报
                    // Missing number（tex.web scan_dimen 无分组分支）。
                    if let Some(d) = digit_value(tok) {
                        if saw_dot {
                            // TRIP：`16383.99999237060546875pt` 17 位小数——防 i64
                            // 溢出，超限位截断（TeX scan_dimen 定点累加同效）。
                            if frac < i64::MAX / 10 {
                                frac = frac * 10 + i64::from(d);
                                frac_len += 1;
                            }
                        } else if int_part < i64::MAX / 10 {
                            int_part = int_part * 10 + i64::from(d);
                        }
                        any = true;
                    } else if matches!(tok.charcode(), Some(c) if c == b'.' as u32 || c == b',' as u32)
                        && !saw_dot
                    {
                        // TeX scan_dimen：`.` 与 `,` 均可作小数点（trip.tex L40 `,0015...in`）；
                        // 无整数部分也合法（`.5in`、`.pt` → 0pt，TRIP L151 `\vsize.pt`）
                        saw_dot = true;
                        any = true;
                    } else {
                        self.unread(tok);
                        break;
                    }
                }
            }
        }
        if !any {
            // TeX：尺寸数字缺失 → "Missing number" 恢复 0（\leftskip \parshape pt plus...）
            self.report_missing_number();
            return Ok((0, 0));
        }
        // <整数>[<小数>]<内部尺寸量>：`11\parshapedimen4` = 11 × 4pt、
        // `2\fontdimen6\font` 等（TeX scan_dimen 的数量乘内部量）。
        if let Some(csid) = self.peek_csid()? {
            let quantity = match self.eqtb.slot(csid).clone() {
                EqSlot::Primitive(
                    Primitive::ParshapeLength | Primitive::ParshapeIndent | Primitive::ParshapeDimen,
                ) => {
                    let kind = match self.eqtb.slot(csid) {
                        EqSlot::Primitive(Primitive::ParshapeIndent) => 0,
                        EqSlot::Primitive(Primitive::ParshapeLength) => 1,
                        _ => 2,
                    };
                    self.fetch()?; // 消费 \parshape*
                    let idx = self.scan_number()?;
                    Some(self.parshape_access(idx, kind))
                }
                EqSlot::Primitive(Primitive::FontDimen) => {
                    self.fetch()?; // 消费 \fontdimen
                    let num = self.scan_number()?;
                    let font = self.scan_font_ident()?;
                    Some(self.fontdimen(font, num as u32))
                }
                EqSlot::Primitive(Primitive::Dimen) => {
                    self.fetch()?; // 消费 \dimen
                    let idx = self.scan_register_index()?;
                    Some(self.registers.dimen(idx))
                }
                EqSlot::Register(RegKind::Dimen, idx) => {
                    self.fetch()?; // 消费 \dimendef'd cs
                    Some(self.registers.dimen(idx))
                }
                // TRIP：内部整数作数量乘子（TeX scan_dimen `<factor><internal integer>`：
                // 值×65536sp 作 dimen；L160 `\ifdim.5\mag>0cc0` → .5×2000pt）
                EqSlot::Primitive(Primitive::Mag) => {
                    self.fetch()?; // 消费 \mag
                    Some(self.params.mag * SP_PER_PT)
                }
                // TRIP：盒子尺寸作数量乘子（`4\wd4` = 4×盒 4 宽度、`2\dp3` = 2×盒 3
                // 深度；tex.web scan_dimen `<factor><internal dimen>`，l.332/333 误报修复）
                EqSlot::Primitive(Primitive::Wd | Primitive::Ht | Primitive::Dp) => {
                    let dim = match self.eqtb.slot(csid) {
                        EqSlot::Primitive(Primitive::Wd) => 0,
                        EqSlot::Primitive(Primitive::Ht) => 1,
                        _ => 2,
                    };
                    self.fetch()?; // 消费 \wd/\ht/\dp
                    let idx = self.scan_register_index()?;
                    Some(self.sink.box_dim(idx, dim))
                }
                _ => None,
            };
            if let Some(q) = quantity {
                let denom = 10i128.pow(frac_len);
                let v = (i128::from(int_part) * denom + i128::from(frac)) * i128::from(q) / denom;
                let v = i64::try_from(v).unwrap_or(i64::MAX);
                return Ok((if neg { -v } else { v }, 0));
            }
        }
        // 单位/阶后缀：连续字母，取**最长**已知单位或 fil/fill/filll 阶前缀
        // （TeX scan_keyword 逐个字母匹配的等价：`1ptminus0fil` → "pt" + 放回 "minus"；
        // `0fillminus0filll` → 阶词 "fill" 被消费并回传，放回 "minus"）。
        const UNITS: &[&str] = &["sp", "pt", "bp", "in", "cm", "mm", "pc", "cc", "dd", "mu"];
        const ORDER_WORDS: &[&str] = &["fil", "fill", "filll"];
        let mut unit_tokens: Vec<(Token, char)> = Vec::new();
        while let Some((tok, _)) = self.fetch()? {
            // TeX scan_keyword 逐字符 get_x_token：单位字母可经展开产生，
            // 如 TRIP L390 `72p\iftrue t1i` → `p` 后 \iftrue 展开取真分支 `t`
            // 组成 "pt"（\iftrue 被求值消费，`t1i` 中 `t` 匹配单位、`1` 放回）。
            if let Some(csid) = tok.csid() {
                let slot = self.eqtb.slot(csid).clone();
                let expandable = match slot {
                    EqSlot::Macro(m) => !(m.value.protected && self.suppress_expansion > 0),
                    EqSlot::Primitive(p) => p.is_expandable(),
                    _ => false,
                };
                if expandable {
                    let mut expansion = Vec::new();
                    self.expand_once((tok, false), &mut expansion)?;
                    // TeX scan_keyword 逐字符语义：展开结果**第一个 token 是字母**
                    // 才并入单位词（`p\iftrue t1i` → `\iftrue` 展开为字母 `t`
                    // 组成 "pt"）；否则（如 `2.5pt\the\dimen0` 的 `\the` 展开为
                    // 数字 `0`）放回**展开结果的第一个 token**（cs 本身已消费其参数，
                    // 不能放回 cs 否则主循环重复执行报"缺少参数"），结束单位扫描。
                    let first_is_letter = expansion
                        .first()
                        .is_some_and(|(t, _)| t.catcode() == Some(Catcode::Letter));
                    if !first_is_letter {
                        // 展开结果整体放回（TeX get_x_token：`\the` 等被展开后其
                        // 输出全部进入流，如 `0fil\the\count7` → 展开 "77" 保留为
                        // 文本；只放回首个 token 会丢失其余输出）。
                        let items: Vec<(Token, bool)> = expansion.into_iter().collect();
                        self.stack.push(InputFrame::TokenList {
                            items: Arc::from(items),
                            pos: 0,
                        });
                        break;
                    }
                    let items: Vec<(Token, bool)> = expansion.into_iter().collect();
                    self.stack.push(InputFrame::TokenList {
                        items: Arc::from(items),
                        pos: 0,
                    });
                    continue;
                }
            }
            let Some(ch) = tok.charcode().and_then(char::from_u32) else {
                self.unread(tok);
                break;
            };
            if !ch.is_ascii_alphabetic() {
                self.unread(tok);
                break;
            }
            unit_tokens.push((tok, ch));
            // TeX scan_keyword：单位词一旦完整（且不可能再扩展成更长单位，
            // 如 "pt"；"fil"/"fill" 可能是 "filll" 前缀故不在此列）立即停止
            // 收集，不再 fetch 后续 token —— 保证 `2.5pt\the\dimen0` 中
            // `\the` 不被单位扫描消费（修复 save.rs exec_register 错位）。
            let w: String = unit_tokens.iter().map(|(_, c)| c).collect();
            if UNITS.contains(&w.as_str()) {
                break;
            }
        }
        let word: String = unit_tokens.iter().map(|(_, c)| c).collect();
        // 最长完整候选前缀（单位优先于阶词；同长按出现顺序取首个）
        let mut best: Option<(&str, usize)> = None; // (词, 长度)
        for u in UNITS.iter().chain(ORDER_WORDS.iter()) {
            if word.starts_with(u) && best.map_or(true, |(_, l)| u.len() > l) {
                best = Some((u, u.len()));
            }
        }
        // TeX `true<unit>`：绝对单位（忽略放大系数；NTex 无放大系数，等同 `<unit>`）。
        // 需在 "truedd"/"truept" 等整体匹配失败时剥离 "true" 前缀后按单位匹配
        // （TRIP L331 `\halign spread-12.truedd{...}`——参考不报 "Illegal unit"）。
        if let Some(rest) = word.strip_prefix("true") {
            if !rest.is_empty() {
                for u in UNITS.iter() {
                    if rest.starts_with(u) && best.map_or(true, |(_, l)| u.len() + 4 > l) {
                        best = Some((u, u.len() + 4)); // consumed 含 "true"
                    }
                }
            }
        }
        let (unit, consumed, order) = match best {
            Some((u, len)) if ORDER_WORDS.contains(&u) => {
                // 阶词：仅 stretch/shrink 上下文（inf=true）消费（tex.web L8932 `if inf`）；
                // width（inf=false）与 mu 上下文不认阶 → 报单位错、整词放回
                // （实测：`\muskip1=5fil` "(mu inserted)"、`\skip1=5fil` "(pt inserted)"）。
                if !inf {
                    self.report_bad_unit(mu);
                    let unit = if mu { "mu" } else { "pt" };
                    (unit.to_owned(), 0, 0)
                } else {
                    let order = match u {
                        "fil" => crate::register::order::FIL,
                        "fill" => crate::register::order::FILL,
                        "filll" => crate::register::order::FILLL,
                        _ => 0,
                    };
                    ("pt".to_owned(), len, order)
                }
            }
            Some((u, len)) => {
                // mu 上下文只认 "mu"（其他单位词 → "(mu inserted)"、词放回、值按 mu）；
                // pt 上下文 "mu" 单位不合法（→ "(pt inserted)"、词放回、值按 pt）。
                if u == "mu" && !mu {
                    self.report_bad_unit(mu);
                    ("pt".to_owned(), 0, 0)
                } else if u != "mu" && mu {
                    self.report_bad_unit(mu);
                    ("mu".to_owned(), 0, 0)
                } else {
                    (u.to_owned(), len, 0)
                }
            }
            None if unit_tokens.is_empty() => {
                // mu 上下文无单位同样报错并按 mu 恢复（tex.web L8990 无条件报错）
                if mu {
                    self.report_bad_unit(mu);
                    ("mu".to_owned(), 0, 0)
                } else {
                    ("pt".to_owned(), 0, 0)
                }
            }
            None => {
                if mu {
                    // mu 上下文：任何非 "mu" 字母词 → "(mu inserted)"
                    self.report_bad_unit(mu);
                    ("mu".to_owned(), 0, 0)
                } else {
                    // TeX scan_dimen：字母串匹配不到完整单位 → "Illegal unit of measure
                    // (pt inserted)" 恢复：整词放回、值按 pt 计（TRIP L390 `\ifdim72p...`）
                    self.report_bad_unit(mu);
                    ("pt".to_owned(), 0, 0)
                }
            }
        };
        // 放回未消费的字母（[consumed..]）
        if consumed < unit_tokens.len() {
            let back: Vec<(Token, bool)> = unit_tokens[consumed..]
                .iter()
                .map(|(t, _)| (*t, false))
                .collect();
            self.stack.push(InputFrame::TokenList {
                items: Arc::from(back),
                pos: 0,
            });
        }
        // 整数部分 + 四舍五入的小数部分（pdfTeX 实测：3.6pt→235930、0.0001pt→7，
        // 即 round(frac × 65536 / 10^k)）；i128 防溢出。
        let num_pt: i128 = if frac_len == 0 {
            i128::from(int_part) * i128::from(SP_PER_PT)
        } else {
            let denom = 10i128.pow(frac_len);
            let frac_sp = (i128::from(frac) * i128::from(SP_PER_PT) + denom / 2) / denom;
            i128::from(int_part) * i128::from(SP_PER_PT) + frac_sp
        };
        let scaled: i128 = if unit == "mu" {
            // mu 单位：1mu = 65536 单位（pdfTeX 实测 \mutoglue/\gluetomu 1:1，无 quad 换算）
            num_pt
        } else {
            let unit_sp =
                unit_to_sp(&unit).ok_or_else(|| Error::invalid_input(format!("未知单位：{unit}")))?;
            num_pt * i128::from(unit_sp) / i128::from(SP_PER_PT)
        };
        let scaled = if neg { -scaled } else { scaled };
        let scaled = i64::try_from(scaled).map_err(|_| Error::invalid_input("尺寸溢出"))?;
        // TeX 规则：尺寸后跟随的空格被吞掉
        self.skip_trailing_spaces()?;
        Ok((scaled, order))
    }

    /// "! Incompatible glue units."（tex.web mu_error，L8265-8268）：
    /// glue 与 mu 胶水混用（`\skip=\muskip`、`\muskip=\skip`、`\glueexpr` 嵌 `\muexpr` 等）。
    /// 恢复：按 1mu=1pt 换算继续（数值不变，仅单位语义标记）。
    fn report_incompatible_glue_units(&mut self) {
        self.report_error("Incompatible glue units.");
    }

    /// 扫描胶水（非 mu 上下文）：`\hskip`/`\vskip`/`\skip<idx>=`/`\glueexpr` 项等。
    fn scan_glue(&mut self) -> Result<Glue> {
        self.scan_glue_inner(false)
    }

    /// 扫描胶水（mu 上下文）：`\muskip<idx>=`/`\mskip`/`\muexpr` 项。
    /// mu 上下文只认 mu 胶水（`\skip` 前导/`\glueexpr`/`\gluetomu` 输出 → "Incompatible glue units"）。
    fn scan_glue_mu(&mut self) -> Result<Glue> {
        self.scan_glue_inner(true)
    }

    /// 胶水扫描公共实现。可选前导胶水量（`\glueexpr`/`\muexpr`/`\skip<idx>`/`\muskip<idx>`/
    /// skipdef/muskipdef cs/`\mutoglue`/`\gluetomu`）或 width + 可选 `plus/minus <dimen>[fil]`。
    /// 前导量单位与目标 mu 标志不匹配时报 "Incompatible glue units"（按 1:1 继续）。
    fn scan_glue_inner(&mut self, mu: bool) -> Result<Glue> {
        // M4-5 e-TeX：\glueexpr/\muexpr 可在任意胶水上下文求值
        self.skip_spaces()?;
        if let Some(csid) = self.peek_csid()? {
            match self.eqtb.slot(csid).clone() {
                EqSlot::Primitive(Primitive::Glueexpr) => {
                    self.fetch()?; // 消费 \glueexpr
                    let g = self.eval_glue_expression(false)?;
                    if mu {
                        // \muskip=\glueexpr：glueexpr 输出 pt 胶水 → 目标 mu → Incompatible
                        self.report_incompatible_glue_units();
                    }
                    return Ok(g);
                }
                EqSlot::Primitive(Primitive::Muexpr) => {
                    self.fetch()?; // 消费 \muexpr
                    let g = self.eval_glue_expression(true)?;
                    if !mu {
                        // \skip=\muexpr：muexpr 输出 mu 胶水 → 目标 pt → Incompatible
                        self.report_incompatible_glue_units();
                    }
                    return Ok(g);
                }
                // ETRIP：`\hskip\skip5` 等 —— 前导胶水寄存器整体引用
                EqSlot::Primitive(Primitive::Skip) => {
                    self.fetch()?;
                    let idx = self.scan_register_index()?;
                    let g = self.registers.skip(idx);
                    if mu {
                        self.report_incompatible_glue_units();
                    }
                    return Ok(g);
                }
                EqSlot::Primitive(Primitive::Muskip) => {
                    self.fetch()?;
                    let idx = self.scan_register_index()?;
                    let g = self.registers.muskip(idx);
                    if !mu {
                        self.report_incompatible_glue_units();
                    }
                    return Ok(g);
                }
                // ETRIP 第二波：\mutoglue<mu 胶水> → pt 胶水、\gluetomu<胶水> → mu 胶水
                // （1mu = 1pt = 65536sp，数值不变；仅单位语义转换）
                EqSlot::Primitive(Primitive::MuToGlue) => {
                    self.fetch()?; // 消费 \mutoglue
                    let g = self.scan_glue_inner(true)?; // 输入：mu 上下文
                    if mu {
                        self.report_incompatible_glue_units(); // 输出 pt
                    }
                    return Ok(g);
                }
                EqSlot::Primitive(Primitive::GlueToMu) => {
                    self.fetch()?; // 消费 \gluetomu
                    let g = self.scan_glue_inner(false)?; // 输入：pt 上下文
                    if !mu {
                        self.report_incompatible_glue_units(); // 输出 mu
                    }
                    return Ok(g);
                }
                EqSlot::Register(kind, idx) => {
                    // skipdef/muskipdef 绑定的寄存器 cs
                    self.fetch()?;
                    let g = match kind {
                        RegKind::Skip => self.registers.skip(idx),
                        RegKind::Muskip => self.registers.muskip(idx),
                        _ => {
                            return Err(Error::invalid_input(
                                "胶水上下文需要 \\skip/\\muskip 寄存器",
                            ))
                        }
                    };
                    if mu != matches!(kind, RegKind::Muskip) {
                        self.report_incompatible_glue_units();
                    }
                    return Ok(g);
                }
                _ => {}
            }
        }
        // width：mu 上下文只认 "mu" 单位（scan_dimen_mu）
        let width = if mu {
            self.scan_dimen_mu()?
        } else {
            self.scan_dimen()?
        };
        let mut stretch = 0i64;
        let mut shrink = 0i64;
        let mut stretch_order = 0u8;
        let mut shrink_order = 0u8;
        for _ in 0..2 {
            let Some(word) = self.scan_keyword(|w| w == "plus" || w == "minus")? else {
                break;
            };
            // 值 + 无穷阶：stretch/shrink 允许 fil 阶（inf=true，tex.web scan_glue）
            let (d, order) = self.scan_dimen_inner(mu, true)?;
            if word == "plus" {
                stretch = d;
                stretch_order = order;
            } else {
                shrink = d;
                shrink_order = order;
            }
        }
        Ok(Glue {
            width,
            stretch,
            shrink,
            stretch_order,
            shrink_order,
        })
    }

    /// 跳过空格后读取一个裸字母词（TeX `scan_keyword` 语义：`\hskip 5pt plus 2pt`
    /// 中的 `plus`、`\hrule height 1pt` 中的 `height` 都是裸字母词）。
    /// `is_kw` 判定是否为关键字；非关键字时字母原样放回（保持顺序）并返回 None。
    fn scan_keyword(&mut self, is_kw: impl Fn(&str) -> bool) -> Result<Option<String>> {
        self.skip_spaces()?;
        let mut word = String::new();
        let mut letters: Vec<(Token, bool)> = Vec::new();
        while let Some((tok, _)) = self.fetch()? {
            if let Some(ch) = tok.charcode().and_then(char::from_u32) {
                if ch.is_ascii_alphabetic() {
                    word.push(ch);
                    letters.push((tok, false));
                    continue;
                }
            }
            self.unread(tok);
            break;
        }
        if word.is_empty() {
            return Ok(None);
        }
        // TeX scan_keyword：关键字大小写不敏感（`plUs`/`lllminus` = plus/minus）
        let lower = word.to_ascii_lowercase();
        if is_kw(&lower) {
            return Ok(Some(lower));
        }
        self.stack.push(InputFrame::TokenList {
            items: Arc::from(letters),
            pos: 0,
        });
        Ok(None)
    }

    /// 窥视下一个 token 是否为控制序列（fetch + unread）。
    fn peek_csid(&mut self) -> Result<Option<u32>> {
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("扫描到输入末尾"))?
            .0;
        let csid = tok.csid();
        self.unread(tok);
        Ok(csid)
    }

}
