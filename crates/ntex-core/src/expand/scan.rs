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
            if diag_enabled("NTEX_COND_TRACE") {
                eprintln!("[trace-maybe] 求值条件 {op:?}");
            }
            self.step_conditional(op, tok)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 沿 `\let` 别名链解引用到末端槽（tex.web 口径：`\let` 在 eqtb 层复制含义，
    /// 别名即原义）。本引擎的 [`crate::eqtb::EqSlot::Alias`] 只在目标是**宏/未
    /// 定义** cs 时保留（`let_to`：原语/寄存器/字符等在定义时压缩为直接含义），
    /// 故各取 token 位在判定"是否可展开 / 是否内部量"前必须先追链，否则
    /// `\number\宏别名` 落成 Missing number（tex.web 的 scan_int 经 get_x_token
    /// 展开宏）。带环保护（上限 100，同 fetch_non_filler /
    /// scan_group_contents_expanding 的同名守卫）。
    pub(crate) fn deref_alias_chain(&self, csid: u32) -> u32 {
        let mut id = csid;
        for _ in 0..100 {
            match self.eqtb.slot(id) {
                EqSlot::Alias(next) => id = *next,
                _ => return id,
            }
        }
        id
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
            // 条件机跳过区（tex.web pass_text 语义）：fi_or_else 臂在 expand 内
            // **同步** `while cur_chr<>fi_code do pass_text` 后弹帧——假分支 token
            // 根本到不了数字扫描的后续取 token 位。本引擎把跳过留给惰性
            // Skipping 帧（主循环逐 token 丢弃），所以数字扫描的每个取 token 位
            // 都必须先问 is_skipping()（十进制循环既有同款臂，见下方数字分支）。
            // 缺此臂时嵌套 romannumeral（`\exp_after:wN X \exp:w` 的 f-前瞻）会把
            // 假分支的宏就地展开：2026 l3kernel 生成条件体 normal 臂
            // `<test> \prg_return_true: \else: \prg_return_false: \fi: \exp_end:
            // \c_true_bool \c_false_bool` 中 `\prg_return_false:` 的
            // `\exp_after:wN\use_ii:nn\exp:w` 被展开，其 `\exp:w` 又把
            // `\exp_end:`（chardef 0）当数吃掉、`\use_ii:nn` 反手吞掉真臂的
            // `\c_true_bool`/`\c_false_bool`——真臂取到 0、残留 token 级联成
            // `\use_ii:nn extra }` 主簇（expl3 l.7952 区 34 条）。
            if self.is_skipping() {
                if let Some(op) = self.cond_op(tok) {
                    self.step_conditional(op, tok)?;
                }
                continue;
            }
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
                // 别名即原义：宏别名须按目标含义展开（tex.web scan_int 符号循环
                // 的 get_x_token；Alias 槽只指向宏/未定义，见 deref_alias_chain）。
                let expandable = match self.eqtb.slot(self.deref_alias_chain(csid)).clone() {
                    EqSlot::Macro(_) => true,
                    EqSlot::Primitive(p) if p.is_expandable() => true,
                    _ => false,
                };
                // tex.web expand 的 fi_or_else 臂（§9897 @<Terminate the current
                // conditional...@>）：符号循环的 get_x_token 对 `\else`/`\fi`/`\or`
                // 同样经 expand 处理——栈顶帧 Evaluating（if_limit=if_code，外层
                // 条件操作数扫描中）走 insert_relax 门（token 放回 + 前插
                // frozen `\relax`，本扫描按 Missing number 收场、`\fi` 留给外层
                // 条件机闭合）；栈顶帧已完成求值（if_limit=else_code）则就地
                // 跳过/弹帧。expl3 生成条件体的 test 段自带 `\else:`/`\fi:`
                // （`\cs_if_exist_p:N` 的
                //   `\if_meaning:w #1\scan_stop: \use_i:nnnn \else: \fi: \if_cs_exist:N #1`），
                // 在 `\number<谓词>` 中求值时这些终结符从符号循环到达——放回
                // 则落入十进制数字循环错位弹帧，`\use:n` 级联 Missing number
                // （l3kernel l.7952 sys 区 40 条的根因）。仅当无帧可归属时维持
                // 旧"放回 + Missing number"恢复（游终结符，行为不变）。
                if let Some(op) = self.cond_op(tok) {
                    if matches!(op, CondOp::Fi | CondOp::Else | CondOp::Or)
                        && self.cond_stack.is_empty()
                    {
                        self.unread(tok);
                        break;
                    }
                    self.step_conditional(op, tok)?;
                    continue;
                }
                // tex.web scan_int 符号循环的 get_x_token 语义：宏/可展开原语
                // 展开后重新进入符号处理（trip.tex L103 `\tracingoutput\on`：
                // \on 宏展开为 1 作为参数值——缺此分支则报 Missing number 并把
                // \on 遗留到输入流，L104 \moveleft 连锁错位）。
                //
                // e-TeX \protected 抑制面（etex-manual "Protected macros … are
                // not expanded **when building an expanded token list**"）只盖
                // token 列表吸收（\edef/\write/\message 的 xpand 循环，见
                // scan_edef_body 的 protected 臂）——**数值扫描不是列构建**：
                // get_x_token 无 protected 门，expl3 的
                // `\exp:w \exp_end_continue_f:w <stuff>`（l3expan 全族的
                // romannumeral 技巧，\exp_end_continue_f:w 是 protected 宏，
                // expl3-code L2792）在 \expanded/f 型实参内依赖此语义展开成
                // char-0 供 scan_int 取 0；此前的 suppress_expansion 门在
                // \expanded 内把它挡成不可展开 → "Missing number, treated as
                // zero"，quark 模块 `\__quark_module_name:N` 全灭（expl3
                // l.3782 起 invalid-function bail out，§24）。
                if expandable {
                    let mut expansion = Vec::new();
                    self.expand_once((tok, false), &mut expansion)?;
                    if !expansion.is_empty() {
                        self.push_frame(InputFrame::TokenList {
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
            // 字母常量之后 TeX **仍继续展开**（l3expan 对 `\exp_end_continue_f:w`
            // 的注释："after a character code, TeX will still look for further
            // digits, so full expansion continues until an unexpandable token is
            // found"）。逐 token `get_x_token`（可展开项就地展开、条件就地步进），
            // 直到不可展开 token 放回——展开产物**不折入数值**（`\lccode`B=`b\the
            // \lccode`B`=98 etrip 同款：数字放回照常输出）。expl3 全族 f 型展开
            // （`\exp:w \exp_end_continue_f:w <stuff>` = `\romannumeral` `^^@，即
            // 字符码 0）依赖此语义：哨兵展开成 0 后继续前瞻、把 <stuff> 展开到位，
            // 与 `\exp_end:`（chardef 0，内部整数立即收尾不前瞻）相区分。quark
            // 模块 `\__quark_module_name:N`（expl3-code L3505）的
            // `\exp_last_unbraced:Nf \__quark_module_name:w { \cs_to_str:N #1 }`
            // 正是靠它把 `\cs_to_str:N` 的展开产物（字符）真正交到参数匹配手里；
            // 缺此前瞻则 `\cs_to_str:N #1` 以裸 token 进入 `##1`，`:`/`_` 两个
            // 分割点全部落空 → 模块名取到原名 → invalid-function bail out（§24.3）。
            let val: i64 = code;
            while let Some((tok, _)) = self.fetch()? {
                if let Some(csid) = tok.csid() {
                    let expandable =
                        match self.eqtb.slot(self.deref_alias_chain(csid)).clone() {
                            EqSlot::Macro(_) => true,
                            EqSlot::Primitive(p) if p.is_expandable() => true,
                            _ => false,
                        };
                    if expandable {
                        let mut expansion = Vec::new();
                        self.expand_once((tok, false), &mut expansion)?;
                        if !expansion.is_empty() {
                            self.push_frame(InputFrame::TokenList {
                                items: Arc::from(expansion),
                                pos: 0,
                            });
                        }
                        continue;
                    }
                }
                if let Some(op) = self.cond_op(tok) {
                    if matches!(op, CondOp::Fi | CondOp::Else | CondOp::Or)
                        && self.cond_stack.is_empty()
                    {
                        self.unread(tok);
                        break;
                    }
                    self.step_conditional(op, tok)?;
                    continue;
                }
                self.unread(tok);
                break;
            }
            // TeX scan_int：数字（含反引号常量）后跟随的空格被吞
            self.skip_trailing_spaces()?;
            return Ok(if neg { -val } else { val });
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
            // 别名即原义（tex.web §24.4：`\let` 复制含义）：分派前沿链解引用——
            // expl3 全篇 `\cs_new_eq:NN` 别名（`\__int_eval:w → \tex_numexpr:D`
            // 等）以 Alias 槽落 eqtb 时，`\number\别名` 须直达目标的内部量臂，
            // 否则落空（旧 `_ => {}`）→ Missing number 取 0、原 token 留流
            // （l.8073 表达式失真的机制级复刻 mech19 同款）。
            let csid = self.deref_alias_chain(csid);
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
                // 内部整数：\fontdimen<num><font> → 该字体参数值（sp）
                // tex.web scan_int 的 scan_something_internal 臂——expl3
                // intarray（\__intarray_entry:w = \tex_fontdimen:D）在 \ifnum/
                // \numexpr 里读项值全走此路径（expl3-code L14928）。
                EqSlot::Primitive(Primitive::FontDimen) => {
                    self.fetch()?; // 消费 \fontdimen
                    let num = self.scan_number()?;
                    let num =
                        u32::try_from(num).map_err(|_| Error::invalid_input("\\fontdimen 参数号越界"))?;
                    let font = self.scan_font_ident()?;
                    let v = self.fontdimen(font, num);
                    return Ok(if neg { -v } else { v });
                }
                // 内部整数：\hyphenchar<font> / \skewchar<font> → 该字体的断字
                // 字符/skew 字符。tex.web scan_something_internal 的
                // `@<Fetch a font integer@>`（L8552-8557）：scan_font_ident 后取
                // hyphen_char[f]/skew_char[f]；scan_int 的 <internal integer> 臂
                // （§445 cur_cmd∈[min_internal,max_internal]）同样路由至此。
                // expl3 intarray 的 pdfTeX 模拟里 \__intarray_count:w = 字体
                // \hyphenchar，\number 读数组长度全走此臂
                // （expl3-code L15576/L15591/L15601）。
                // 偏差：tex.web 建字体时按 default_hyphen_char/default_skew_char
                // 逐字体初始化（L11210）；引擎以 HashMap 惰性覆盖、未覆盖回退常量
                // 默认（与 `\the` 臂同约定）。
                EqSlot::Primitive(Primitive::HyphenChar) => {
                    self.fetch()?; // 消费 \hyphenchar
                    let font = self.scan_font_ident()?;
                    let v = self.hyphenchars.get(&font).copied().unwrap_or(45);
                    return Ok(if neg { -v } else { v });
                }
                EqSlot::Primitive(Primitive::SkewChar) => {
                    self.fetch()?; // 消费 \skewchar
                    let font = self.scan_font_ident()?;
                    let v = self.skewchars.get(&font).copied().unwrap_or(-1);
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
                // LaTeX 兼容第八刀：pdfTeX 探测原语只读整数
                // （\pdftexversion/\pdftexrevision 对齐 pdfTeX 1.40.25；
                // \pdfshellescape=0（无 shell escape）；\pdfelapsedtime=0
                // （无计时器，恒 0——偏差记录报告 §15.3）。
                // latex.ltx L1122 engine-check 与 L22500 `\ifnum\pdftexrevision<22`
                // 均以数字操作数身份读取）
                EqSlot::Primitive(Primitive::PdfTeXVersion) => {
                    self.fetch()?;
                    return Ok(if neg { -140 } else { 140 });
                }
                EqSlot::Primitive(Primitive::PdfTeXRevision) => {
                    self.fetch()?;
                    return Ok(if neg { -25 } else { 25 });
                }
                EqSlot::Primitive(Primitive::PdfShellEscape) => {
                    self.fetch()?;
                    return Ok(0);
                }
                EqSlot::Primitive(Primitive::PdfElapsedTime) => {
                    self.fetch()?;
                    return Ok(0);
                }
                // \pdfrandomseed：只读（misc 64，经 \pdfsetrandomseed 写）
                EqSlot::Primitive(Primitive::PdfRandomSeed) => {
                    self.fetch()?;
                    let v = self.params.misc[PDF_RANDOM_SEED_IDX];
                    return Ok(if neg { -v } else { v });
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
                // ETRIP 冲刺：TeX/e-TeX 内部整数参数（\interactionmode/\language/\tracing* 等）。
                // 值域两处合一：int_param_index 覆盖 misc 索引区；param_kind_of 的
                // Number 值参数（\endlinechar/\newlinechar/\parindent 类之外的纯整数
                // 参数）同臂——tex.web scan_something_internal 对 assign_int 区全认，
                // G3 起读写两侧共用 param_kind_of 一张表，读侧不得自持索引表漏项
                // （expl3 `\tex_endlinechar:D` 读臂缺此臂时落 Missing number，
                // `\__cctab_gset:n` 的 `\fontdimen257<font> \tex_endlinechar:D
                // \c__intarray_sp_dim` 现场级联）。
                EqSlot::Primitive(p)
                    if int_param_index(p).is_some()
                        || matches!(
                            param_kind_of(p).map(|k| self.params.get(k)),
                            Some(ParamValue::Number(_))
                        ) =>
                {
                    self.fetch()?; // 消费原语
                    if let Some(idx) = int_param_index(p) {
                        let v = self.params.misc[idx];
                        return Ok(if neg { -v } else { v });
                    }
                    let v = match param_kind_of(p).map(|k| self.params.get(k)) {
                        Some(ParamValue::Number(v)) => v,
                        _ => 0,
                    };
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
                // dimendef'd cs（如 plain 的 \z@=\dimen12）数字上下文返回 sp 值：
                // tex.web scan_something_internal `register:` 分支对四种寄存器统一
                // 取 cur_val，随后 while cur_val_level>level 降级（dimen→int 数值直传）。
                // 此前只认 Count 别名，\fam\z@（plain.tex \rm 定义）报 Missing number。
                EqSlot::Register(RegKind::Dimen, idx) => {
                    self.fetch()?; // 消费 dimendef'd cs
                    let v = self.registers.dimen(idx);
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
            // 真条件的 `\else` 死分支（step_conditional 已把最内层帧翻到 Skipping）：
            // 分支内 token 丢弃不累计（主循环惰性跳过同款），仅条件 token 仍走状态机
            // 推进（`\fi` 弹帧后回到累计）。
            if self.is_skipping() {
                if let Some(op) = self.cond_op(tok) {
                    self.step_conditional(op, tok)?;
                }
                continue;
            }
            // eTeX 表达式分组（{1+}{2*3} 的 {1+}）只属于 \numexpr 表达式因子层
            // （expr.rs expr_factor），scan_int 遇组字符 { 应报 Missing number
            // （tex.web scan_int 无分组分支；TRIP l.106 \number{ 漏报修复）。
            match digit_value(tok) {
                Some(d) => {
                    // tex.web scan_int @<Scan a decimal constant@>：
                    // 超 0x7FFFFFFF → "Number too big"（**一次性**，报错后
                    // goto done 停止数字扫描并钳制——pdfTeX 实测
                    // \count0=99999999999999 只报 1 次，值=2147483647）。
                    // 此前两处偏差：阈值用 i64::MAX（永不触发）+ 修复初期
                    // 每位数字重复报错（未停止扫描）。
                    if val > (0x7FFF_FFFF - i64::from(d)) / 10 {
                        self.report_error("Number too big.");
                        val = 0x7FFF_FFFF;
                        any = true;
                        // 报错即终止数字累计（tex.web goto done），后续
                        // token 留给调用方（\write 文本等）。
                        self.skip_trailing_spaces()?;
                        break;
                    }
                    val = val * 10 + i64::from(d);
                    any = true;
                }
                None => {
                    // expl3 惯用法 `\ifnum0\ifdefined X 1\fi...>0`（latex.ltx L1122、
                    // expl3 全篇）：十进制数字循环遇条件 token 一律步进条件机（tex.web
                    // scan_int 的 get_x_token 对 if_test 与 fi_or_else 都 expand：
                    // If* 就地求值真则产 1 继续累计、假则 skip_ahead 跳过；`\fi`/
                    // `\else`/`\or` 闭合的是**本数字扫描期间**开启的条件帧——如
                    // `\ifdefined` 真分支的 `1\fi`，放回会让外层 scan_relation 误报
                    // "Missing = inserted for \ifnum"（第七轮 L1122 偏差即此）。
                    // 与符号/基数循环不同：那里 \fi 等属外层求值的未决终结符须放回
                    // （TRIP L82 `\ifnum'\ifnum10=10 12="\fi` 的 \fi 在 hex 循环里
                    // 由外层 skip_ahead 闭合）。无帧可闭的游离终结符维持原放回语义
                    // （"Missing number" 恢复，行为不变）。
                    if let Some(op) = self.cond_op(tok) {
                        if matches!(op, CondOp::Fi | CondOp::Else | CondOp::Or)
                            && self.cond_stack.is_empty()
                        {
                            self.unread(tok);
                            break;
                        }
                        self.step_conditional(op, tok)?;
                        continue;
                    }
                    // get_x_token 展开语义的**窄子集**：仅当数字中途的
                    // `\expandafter` 揭示的是**条件开始**（\if*）才展开。
                    // expl3 引擎门闩 `\ifnum0\expandafter\ifx\csname …=0`：
                    // `0` 后 `\expandafter\ifx…`——不展开则左操作数停在 0、
                    // 关系符扫描拿到内层 `\ifx` 假分支（\else 后）的活跃数字
                    // 1，报 "Missing = inserted for \ifnum"（第十二轮）；展开后
                    // 条件链在数字循环内就地求值、分支数字 1 继续累计（01=1）。
                    // 限条件开始：`\ifnum1=1\expandafter\chardef\else…` 右操作数
                    // 后是已完成数外的 `\expandafter`（指向 \chardef 非条件）——
                    // 展开会把帧外 \else 急切消费致结构错乱（回归）。其他可展开
                    // 项（`\number`/`\the`/宏）保持旧行为（停在它们处放回，
                    // 全展开会把 `\count0=5\number\count0` 后续 `\number` 吸入
                    // 当前数，与既有语义/测试相悖；报告 §18 偏差记录）。
                    if let Some(csid) = tok.csid() {
                        if matches!(
                            self.eqtb.slot(csid),
                            EqSlot::Primitive(Primitive::Expandafter)
                        ) {
                            let reveals_cond_start = match self.fetch()? {
                                Some((nt, _)) => {
                                    let c = nt.csid().is_some_and(|nid| {
                                        matches!(
                                            self.eqtb.slot(nid),
                                            EqSlot::Primitive(p)
                                                if matches!(
                                                    CondOp::from_prim(*p),
                                                    Some(
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
                                                            | CondOp::IfInner
                                                            | CondOp::IfVMode
                                                            | CondOp::IfHMode
                                                            | CondOp::IfMMode
                                                            | CondOp::IfEof
                                                            | CondOp::IfVoid
                                                            | CondOp::IfHBox
                                                            | CondOp::IfVBox
                                                            | CondOp::IfFontChar
                                                    )
                                                )
                                        )
                                    });
                                    self.unread(nt);
                                    c
                                }
                                None => false,
                            };
                            if reveals_cond_start {
                                let mut expansion = Vec::new();
                                self.expand_once((tok, false), &mut expansion)?;
                                if !expansion.is_empty() {
                                    self.push_frame(InputFrame::TokenList {
                                        items: Arc::from(expansion),
                                        pos: 0,
                                    });
                                }
                                continue;
                            }
                        }
                    }
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
                    // cs 别名到空格字符（`\let\exp_stop_f: ~`，l3expan.dtx:1093
                    // `\use:nn{\cs_new_eq:NN\exp_stop_f:}{~}`）→ **等价空格终结符**
                    //（tex.web scan_int `.10`：get_token 返回 cmd=space 即消散；
                    // cs 的 cmd 经 eqtb 查得）。expl3 `\exp_stop_f:` 贴数字尾
                    // （fp 区 `\__fp_int_eval:w <n> \exp_stop_f: = ...` 万级出现）
                    // 不解引用则落 `\ifnum` 关系符位 → "Missing = inserted"
                    // 1500 次主簇（2026-09-12 定性）。
                    if tok.catcode() == Some(Catcode::Space) {
                        continue;
                    }
                    if let Some(csid) = tok.csid() {
                        match self.resolve_slot(csid) {
                            Some(EqSlot::Char {
                                catcode: Catcode::Space,
                                ..
                            }) => continue,
                            // \let\sp=\space 型原语别名（Primitive::ControlSpace）
                            Some(EqSlot::Primitive(Primitive::ControlSpace)) => continue,
                            _ => {}
                        }
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
                _ => {
                    // tex.web do_register_command：寄存器号走 scan_eight_bit_int
                    // （eTeX 扩到 15 位）= 完整 scan_int 的**值**语义——
                    // `\count\count1`（TRIP L336 \xx 体 `\global\count\count1=`）
                    // 以 count1 的当前值 2 为下标；内部整数/可展开 token 皆可，
                    // 非数字 cs 报 Missing number 恢复而非致命错误。
                    self.unread(t);
                    self.scan_register_index()
                }
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
        // tex.web assign_toks（L22951）：scan_optional_equals 后同款 filler
        // `@<Get the next non-blank non-relax non-call token@>`（get_x_token：
        // 可展开 filler 展开、跳 spacer/\relax）。filler 处理已内聚到
        // fetch_non_filler，此处单遍即可（clippy：no never_loop）。
        let Some(tok) = self.fetch_non_filler()? else {
            // 输入耗尽：TeX 报 "Missing { inserted" 后以空 token list 收尾
            return Ok(Arc::from(Vec::<Token>::new()));
        };
        // 组开始 → scan_toks 收集（本实现用 scan_group_contents，
        // 它入口自行 fetch 组开始 token，故先放回）
        if tok.catcode() == Some(Catcode::BeginGroup) {
            self.unread(tok);
            let val = self.scan_group_contents(Some("tokens"))?;
            return Ok(Arc::from(val));
        }
        // toks 寄存器内容复制
        if let Some(idx) = self.toks_rhs_index(tok)? {
            return Ok(self.registers.toks(idx));
        }
        Err(Error::invalid_input(
            "\\toks 赋值 RHS 需为 {token list} 或 toks 寄存器",
        ))
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

    /// tex.web `@<Get the next non-blank non-relax non-call token@>`
    /// （tex.web L8208-8210）：`repeat get_x_token until (cur_cmd<>spacer)
    /// and (cur_cmd<>relax)`——filler 位置的取 token 循环。
    ///
    /// - 可展开 token（`\expandafter`/宏/`\the`/`\csname`/...）先展开再重判
    ///   （get_x_token 语义；`\everyjob\expandafter{...}` 的 `{` 由
    ///   `\expandafter` 压回）；
    /// - spacer（cat 10）与 `\relax`（含 `\let` 链上的别名）跳过；
    /// - 被 `\noexpand` 冻结的 token（fetch 返回 `ne=true`）：本轮**不展开**
    ///   （tex.web \noexpand 一次性闩锁——get_x_token 直接返回它；否则
    ///   expand_once 会把冻结 token 原样压回造成无限重取），但仍做
    ///   spacer/\relax 判定；
    /// - `\let\bgroup={` 类组定界别名经 [`Self::resolve_group_char`] 归一；
    /// - 返回值**已被消费**，调用方按需放回。
    ///
    /// 偏差（记录）：tex.web get_x_token 对条件原语（`\ifnum` 等）同样展开；
    /// 本引擎 `is_expandable()` 白名单不含条件原语（`maybe_eval_cond` 是数字
    /// 扫描的专用臂），此处条件 token 走"不可展开 → Missing {"恢复路径。
    fn fetch_non_filler(&mut self) -> Result<Option<Token>> {
        loop {
            let Some((tok, ne)) = self.fetch()? else {
                return Ok(None);
            };
            let t = self.resolve_group_char(tok);
            if t.catcode() == Some(Catcode::Space) {
                continue; // spacer：filler 循环跳过
            }
            if let Some(csid) = t.csid() {
                // 别名链解引用（tex.web get_x_token 直接取 eq_type/eq_value，
                // 别名无独立语义）；带环保护（上限 100，同
                // scan_group_contents_expanding）。
                let mut id = csid;
                let mut hops = 0usize;
                while let EqSlot::Alias(next) = self.eqtb.slot(id) {
                    id = *next;
                    hops += 1;
                    if hops > 100 {
                        return Err(Error::invalid_input("\\let 别名环"));
                    }
                }
                match self.eqtb.slot(id).clone() {
                    // \relax：filler 循环明确跳过（tex.web L8210）；\noexpand\relax
                    // 同样跳过（cur_cmd 仍是 relax）
                    EqSlot::Primitive(Primitive::Relax) => continue,
                    // 被 \noexpand 冻结：不展开，直接作为 filler 结果返回
                    // （TeX get_x_token 的 noexpand 分支）
                    _ if ne => {}
                    // protected 宏在展开抑制上下文（\edef/\write）不展开 → 视为不可展开
                    EqSlot::Macro(m) if !(m.value.protected && self.suppress_expansion > 0) => {
                        self.push_expansion(t, ne)?;
                        continue;
                    }
                    EqSlot::Primitive(p) if p.is_expandable() => {
                        self.push_expansion(t, ne)?;
                        continue;
                    }
                    _ => {}
                }
            }
            return Ok(Some(t));
        }
    }

    /// 把展开结果压回输入栈（filler 循环用；与既有展开点同构）。
    fn push_expansion(&mut self, tok: Token, noexpand: bool) -> Result<()> {
        let mut expansion = Vec::new();
        self.expand_once((tok, noexpand), &mut expansion)?;
        let items: Vec<(Token, bool)> = expansion.into_iter().collect();
        self.push_frame(InputFrame::TokenList {
            items: Arc::from(items),
            pos: 0,
        });
        Ok(())
    }

    /// tex.web `scan_left_brace`（L8194-8206）：一切"必选 `{`"值扫描的入口——
    /// `scan_toks` 的非宏定义臂（toks 寄存器 / `\every*` / `\output` /
    /// `\message` / `\mark`）、`\insert`/`\discretionary`/`\mathchoice`/
    /// `\noalign`/`\hyphenation`/`\patterns`。取 token 用 filler 语义
    /// （[`Self::fetch_non_filler`]）；非 `{` → "! Missing { inserted" +
    /// back_error（token 放回）+ 隐含插入 `{`（tex.web `incr(align_state)`，
    /// 本引擎组配平由收集循环的 depth 承担）。
    fn scan_left_brace(&mut self) -> Result<()> {
        let Some(t) = self.fetch_non_filler()? else {
            // 输入耗尽：与既有入口同款硬错误（tex.web EOF 处 cur_cmd=0
            // ≠ left_brace，同样报 Missing { 但恢复继续）
            return Err(Error::invalid_input("扫描到输入末尾"));
        };
        if t.catcode() != Some(Catcode::BeginGroup) {
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
        }
        Ok(())
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
        // TeX scan_toks 非宏定义臂入口 = scan_left_brace（tex.web L9329）：
        // filler 语义（get_x_token 展开至 `{`，跳 spacer/\relax），非 { →
        // "Missing { inserted."（token 放回、隐含 `{` 恢复继续），不再硬错误
        // （trip.tex L396 `\accent\x\vfill` 等 20 处依赖此恢复）
        self.scan_left_brace()?;
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
                            let csname = self.cs_display_name(csid);
                            let _ = self.sink.write16(format!(
                                "Runaway text?\n\
                                 ! Forbidden control sequence found while scanning text of \\{name}.\n\
                                 <inserted text>\n                }}\n\
                                 <to be read again>\n                   {csname}\n"
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
        // tex.web math_choice（L22152/22171）：push_math(math_choice_group) 后
        // scan_left_brace——入口与一般值扫描共用（filler 语义）；非 `{` 的
        // 错误恢复（token 放回、隐含 `{`、收集到下一个 `}`）同 scan_toks 臂
        // （TRIP L438 `\mathchoice{}a}{A|{}}{\mathchoice}` 分支 `a`/`A|{}`/
        // `\mathchoice`）。scan_group_contents 入口已含 scan_left_brace，
        // 勿在此再调（否则空分支 `{}` 的 `}` 被当作开组 token 多报一次
        // "Missing { inserted."）。
        self.scan_group_contents(None)
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
            // tex.web scan_general_text → scan_toks：token 位的组判定看 **cur_tok**
            //（`cur_tok<right_brace_limit`）；`\let\bg={` 型 cs 经 get_token 仍是
            // cs token（组性只体现在 cur_cmd=eq_type），不得在此归一成字符 token——
            // 否则 e 型体（\use:e 的体经本扫描）里 `\c_group_begin_token` 撑起
            // 永不闭合的组深 → 组平衡崩塌（expl3 `\token_if_*` 生成条件区
            // 32×extra-} + 25×Missing number，第二十七轮）。组定界别名只在
            // **开括号位**归一（下方 match 臂 = tex.web scan_left_brace 的
            // cur_cmd 判定）。
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
                    self.push_frame(InputFrame::TokenList {
                        items: Arc::from(items),
                        pos: 0,
                    });
                    continue;
                }
                EqSlot::Primitive(p) if p.is_expandable() => {
                    let mut expansion = Vec::new();
                    self.expand_once((open, false), &mut expansion)?;
                    let items: Vec<(Token, bool)> = expansion.into_iter().collect();
                    self.push_frame(InputFrame::TokenList {
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
            // cur_tok 判定（tex.web scan_toks）：组定界别名 cs 在组内是数据
            //（`\def\f#1{[#1]}\edef\x{\f\bg}` 真 TeX 存 `\bg`）。
            let t = fetched;
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
                        self.push_frame(InputFrame::TokenList {
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
            // tex.web `<internal dimen>`（scan_dimen 开头 `cur_cmd∈[min_internal,max_internal]`
            // → `scan_something_internal(dimen_val)`，`cur_val_level=dimen_val` → `goto
            // attach_sign`）：dimendef'd cs 的值**就是**尺寸，无需单位；值可负（与前置
            // `-` 号合成，tex.web 由 `if cur_val<0` 翻转 negative 等价实现）。
            // latex.ltx L532 `\boxmaxdepth=\maxdimen`（\maxdimen=\dimendef'd）依赖此臂。
            if let EqSlot::Register(RegKind::Dimen, idx) = self.eqtb.slot(csid).clone() {
                self.fetch()?; // 消费 dimendef'd cs
                let v = self.registers.dimen(idx);
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
            //
            // 与 [`exec_fontchar_dimen`](Self::exec_fontchar_dimen)、`the_tokens_after`
            // 是**同一语义的三个入口**：字符码闸门一律走 [`Self::fontchar_code`]
            // （按被查字体判上界，M9 中文刀 1）。任一处写死 255 都会让
            // `\dimen0=\fontcharwd\zh"4E2D` 报「Bad character code」。
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
                let Some(ch) = self.fontchar_code(font, ch) else {
                    return Ok((0, 0));
                };
                let m = self.font_loader.char_metric(font, ch);
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
            // dimen 内部参数作尺寸（\ifdim\hsize<\hsize、\the\hsize 等；G3 起与
            // \advance 目标集合共用 free.rs::param_kind_of 一张表——tex.web
            // scan_dimen 对 assign_dimen 区全认：\voffset/\scriptspace/\hangindent/
            // \overfullrule/… 不再报 Missing number，letterformat 的
            // `\advance\vsize by-\voffset` 增量扫描走这里）。只取 Dimen 值类：
            // 胶参数由下方胶臂按宽度分量、整数参数走数字通道；\lastkern 下方有
            // 专属臂读 sink 实时值，须排除在此臂外。
            let dim_param = match self.eqtb.slot(csid) {
                EqSlot::Primitive(p) if *p != Primitive::LastKern => param_kind_of(*p).and_then(
                    |k| match self.params.get(k) {
                        ParamValue::Dimen(v) => Some(v),
                        _ => None,
                    },
                ),
                _ => None,
            };
            if let Some(v) = dim_param {
                self.fetch()?;
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
                // 别名即原义（tex.web §24.4）：expl3 全篇 `\cs_new_eq:NN` 别名
                // （`\tex_endlinechar:D → \endlinechar` 等）以 Alias 槽落 eqtb，
                // 判定前须追链，否则 `<dimen>` 数字部分漏认 → `! Missing number`
                // （`\__cctab_gset:n` 的 `\fontdimen257<font> \tex_endlinechar:D
                // \c__intarray_sp_dim` 现场即此）。scan_number 分派侧同款追链。
                let slot = self.eqtb.slot(self.deref_alias_chain(csid)).clone();
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
                                // LaTeX 兼容第八刀：pdfTeX 探测整数作数字操作数
                                // （latex.ltx `\ifnum\pdftexversion=140`；\pdfoutput
                                //  已由下方 int_param_index 分支覆盖）
                                | Primitive::PdfTeXVersion
                                | Primitive::PdfTeXRevision
                                | Primitive::PdfShellEscape
                                | Primitive::PdfElapsedTime
                                | Primitive::PdfRandomSeed
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
                            // Number 值内部参数同数字（与 scan_number 分派臂同一判据，
                            // 探针勿自持第二张表——\tex_endlinechar:D 即漏项反例）
                            || matches!(
                                param_kind_of(*p).map(|k| self.params.get(k)),
                                Some(ParamValue::Number(_))
                            )
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
        const UNITS: &[&str] = &[
            "sp", "pt", "bp", "in", "cm", "mm", "pc", "cc", "dd", "mu", "em", "ex",
        ];
        const ORDER_WORDS: &[&str] = &["fil", "fill", "filll"];
        let mut unit_tokens: Vec<(Token, char)> = Vec::new();
        while let Some((tok, _)) = self.fetch()? {
            // TeX scan_keyword 逐字符 get_x_token：单位字母可经展开产生，
            // 如 TRIP L390 `72p\iftrue t1i` → `p` 后 \iftrue 展开取真分支 `t`
            // 组成 "pt"（\iftrue 被求值消费，`t1i` 中 `t` 匹配单位、`1` 放回）。
            if let Some(csid) = tok.csid() {
                let slot = self.eqtb.slot(csid).clone();
                // 单位词扫描同为数值扫描（scan_keyword 的 get_x_token）：
                // 非 token 列构建，protected 宏照常展开（见 scan_number_inner 注）
                let expandable = match slot {
                    EqSlot::Macro(_) => true,
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
                        self.push_frame(InputFrame::TokenList {
                            items: Arc::from(items),
                            pos: 0,
                        });
                        break;
                    }
                    let items: Vec<(Token, bool)> = expansion.into_iter().collect();
                    self.push_frame(InputFrame::TokenList {
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
            self.push_frame(InputFrame::TokenList {
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
        } else if unit == "em" || unit == "ex" {
            // TeX 内部单位（tex.web scan_dimen）：em = quad(cur_font)（fontdimen 6）、
            // ex = x_height(cur_font)（fontdimen 5）。字体参数缺失（nullfont/
            // 无加载器）按 0 计（tex.web nullfont quad/x_height = 0 同义）。
            let font = self.sink.current_font();
            let param = if unit == "em" { 6 } else { 5 };
            let unit_sp = i128::from(self.font_loader.font_param(font, param).unwrap_or(0));
            num_pt * unit_sp / i128::from(SP_PER_PT)
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
    /// ETRIP P0 校准：与参考 etrip.log 对齐——除 `!` 消息外追加 help1 行
    /// "I'm going to assume that 1mu=1pt when they're mixed."。恢复：按 1mu=1pt 换算继续
    /// （数值不变，仅单位语义标记——本任务不动此数值路径，后续若需按 em/18 修正时可单独立项）。
    fn report_incompatible_glue_units(&mut self) {
        self.report_error("Incompatible glue units.");
        self.report_help("I'm going to assume that 1mu=1pt when they're mixed.");
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
        // tex.web scan_glue：胶水层先处理前导符号（@<Get the next non-blank
        // non-sign token>）——负号作用于**整个胶水**（内部量前导三分量取负；
        // 裸尺寸仅宽度，见下）。不能交给 scan_dimen：否则 `-\muskip1` /
        // `-\mutoglue-\gluetomu9pt`（etrip L906-911）的 stretch/shrink/阶丢失。
        // 递归层（\mutoglue/\gluetomu 参数）各自处理参数的符号。
        self.skip_spaces()?;
        let mut neg = false;
        loop {
            self.skip_spaces()?;
            let Some((tok, _)) = self.fetch()? else {
                break;
            };
            match tok.charcode() {
                Some(v) if v == b'-' as u32 => neg = !neg,
                Some(v) if v == b'+' as u32 => {}
                _ => {
                    self.unread(tok);
                    break;
                }
            }
        }
        // M4-5 e-TeX：\glueexpr/\muexpr 可在任意胶水上下文求值
        let mut width_internal: Option<i64> = None;
        if let Some(csid) = self.peek_csid()? {
            match self.eqtb.slot(csid).clone() {
                EqSlot::Primitive(Primitive::Glueexpr) => {
                    self.fetch()?; // 消费 \glueexpr
                    let g = self.eval_glue_expression(false)?;
                    if mu {
                        // \muskip=\glueexpr：glueexpr 输出 pt 胶水 → 目标 mu → Incompatible
                        self.report_incompatible_glue_units();
                    }
                    return Ok(if neg { g.negated() } else { g });
                }
                EqSlot::Primitive(Primitive::Muexpr) => {
                    self.fetch()?; // 消费 \muexpr
                    let g = self.eval_glue_expression(true)?;
                    if !mu {
                        // \skip=\muexpr：muexpr 输出 mu 胶水 → 目标 pt → Incompatible
                        self.report_incompatible_glue_units();
                    }
                    return Ok(if neg { g.negated() } else { g });
                }
                // ETRIP：`\hskip\skip5` 等 —— 前导胶水寄存器整体引用
                EqSlot::Primitive(Primitive::Skip) => {
                    self.fetch()?;
                    let idx = self.scan_register_index()?;
                    let g = self.registers.skip(idx);
                    if mu {
                        self.report_incompatible_glue_units();
                    }
                    return Ok(if neg { g.negated() } else { g });
                }
                EqSlot::Primitive(Primitive::Muskip) => {
                    self.fetch()?;
                    let idx = self.scan_register_index()?;
                    let g = self.registers.muskip(idx);
                    if !mu {
                        self.report_incompatible_glue_units();
                    }
                    return Ok(if neg { g.negated() } else { g });
                }
                // ETRIP 第二波：\mutoglue<mu 胶水> → pt 胶水、\gluetomu<胶水> → mu 胶水
                // （1mu = 1pt = 65536sp，数值不变；仅单位语义转换）
                EqSlot::Primitive(Primitive::MuToGlue) => {
                    self.fetch()?; // 消费 \mutoglue
                    let g = self.scan_glue_inner(true)?; // 输入：mu 上下文
                    if mu {
                        self.report_incompatible_glue_units(); // 输出 pt
                    }
                    return Ok(if neg { g.negated() } else { g });
                }
                EqSlot::Primitive(Primitive::GlueToMu) => {
                    self.fetch()?; // 消费 \gluetomu
                    let g = self.scan_glue_inner(false)?; // 输入：pt 上下文
                    if !mu {
                        self.report_incompatible_glue_units(); // 输出 mu
                    }
                    return Ok(if neg { g.negated() } else { g });
                }
                // tex.web scan_glue（S=scan_dimen 分支）：`\hskip\dimen0` ——
                // `\dimen` 寄存器访问原语 + 下标，语义同下方 Register(Dimen) 臂
                EqSlot::Primitive(Primitive::Dimen) => {
                    self.fetch()?;
                    let idx = self.scan_register_index()?;
                    let d = self.registers.dimen(idx);
                    width_internal = Some(if neg { -d } else { d });
                    if mu {
                        self.report_incompatible_glue_units();
                    }
                }
                EqSlot::Register(kind, idx) => {
                    // skipdef/muskipdef 绑定的寄存器 cs
                    self.fetch()?;
                    match kind {
                        RegKind::Skip => {
                            let g = self.registers.skip(idx);
                            if mu {
                                self.report_incompatible_glue_units();
                            }
                            return Ok(if neg { g.negated() } else { g });
                        }
                        RegKind::Muskip => {
                            let g = self.registers.muskip(idx);
                            if !mu {
                                self.report_incompatible_glue_units();
                            }
                            return Ok(if neg { g.negated() } else { g });
                        }
                        // tex.web scan_glue（S=scan_dimen 分支）：level=glue_val 时内部
                        // dimen 转零阶胶水（width=值，stretch/shrink=0，plus/minus 照常可扫）；
                        // mu 上下文非 mu 内部量 → "Incompatible glue units"（按 1mu=1pt 继续）。
                        // LaTeX/expl3 依赖此臂：`\skip_const:Nn \c_zero_skip {\c_zero_dim}`。
                        RegKind::Dimen => {
                            let d = self.registers.dimen(idx);
                            width_internal = Some(if neg { -d } else { d });
                            if mu {
                                self.report_incompatible_glue_units();
                            }
                        }
                        _ => {
                            return Err(Error::invalid_input(
                                "胶水上下文需要 \\skip/\\muskip 寄存器",
                            ))
                        }
                    }
                }
                _ => {}
            }
        }
        // width：mu 上下文只认 "mu" 单位（scan_dimen_mu）。
        // 内部 dimen 量（Register(Dimen) 臂）已转为宽度，跳过宽度扫描直接进 plus/minus。
        // 裸尺寸路径的负号只作用于宽度（tex.web scan_glue 非内部量分支：
        // scan_dimen 后 `if negative then negate(cur_val)`；plus/minus 不受影响）
        let width = match width_internal {
            // 内部 dimen 路径的负号已在各自臂内应用于整个胶水，此处不再取反
            Some(d) => d,
            None if mu => {
                let w = self.scan_dimen_mu()?;
                if neg { -w } else { w }
            }
            None => {
                let w = self.scan_dimen()?;
                if neg { -w } else { w }
            }
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
        self.push_frame(InputFrame::TokenList {
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
