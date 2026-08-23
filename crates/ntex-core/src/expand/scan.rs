impl Expander {
    // ---------- 数字与赋值辅助 ----------

    /// 扫描十进制整数；支持 `\count<idx>` 寄存器引用（M1 简化版）。
    fn scan_number(&mut self) -> Result<i64> {
        self.skip_spaces()?;
        // TeX scan_int：跳过可选 `=` 赋值符（`\count0=5` 与 `\count0 5` 等价）
        if let Some((tok, _)) = self.fetch()? {
            if tok.charcode() != Some(b'=' as u32) {
                self.unread(tok);
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
                            val = val * i64::from(base) + i64::from(d);
                            any = true;
                        }
                        None => {
                            self.unread(t);
                            break;
                        }
                    }
                }
                if !any {
                    return Err(Error::invalid_input("预期数字"));
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
                    self.fetch()?; // 消费 \glueexpr/\muexpr
                    let g = self.eval_glue_expression()?;
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
                // \inputlineno：当前输入行号（行号跟踪后续迭代补，恒 0）
                EqSlot::Primitive(Primitive::InputLineNo) => {
                    self.fetch()?; // 消费 \inputlineno
                    return Ok(0);
                }
                // e-TeX 内部只读整数（数字上下文读取）
                EqSlot::Primitive(Primitive::CurrentGroupLevel) => {
                    self.fetch()?;
                    return Ok(self.group_level as i64);
                }
                // 组类型：sink 跟踪组种类（bottom=0 ... math_left=16）
                EqSlot::Primitive(Primitive::CurrentGroupType) => {
                    self.fetch()?;
                    return Ok(self.sink.current_group_type());
                }
                // 最近节点类型：当前列表尾节点类型码（sink 查询；空列表 -1）
                EqSlot::Primitive(Primitive::LastNodeType) => {
                    self.fetch()?;
                    return Ok(self.sink.last_node_type());
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
            match digit_value(tok) {
                Some(d) => {
                    val = val * 10 + i64::from(d);
                    any = true;
                }
                None => {
                    self.unread(tok);
                    break;
                }
            }
        }
        if !any {
            return Err(Error::invalid_input("预期数字"));
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
                    Err(Error::invalid_input("Improper alphabetic constant"))
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
        tok.csid()
            .ok_or_else(|| Error::invalid_input("此处必须是控制序列"))
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
        let n = self.scan_number()?;
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

    /// 扫描平衡花括号内的 token 列表（`\toks0={...}` 用）。
    fn scan_group_contents(&mut self) -> Result<Vec<Token>> {
        let fetched = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("扫描到输入末尾"))?
            .0;
        let open = self.resolve_group_char(fetched);
        if open.catcode() != Some(Catcode::BeginGroup) {
            return Err(Error::invalid_input("预期 {（组开始）"));
        }
        let mut tokens = Vec::new();
        let mut depth = 0usize;
        loop {
            let fetched = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("组未闭合"))?
                .0;
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
        loop {
            let Some((fetched, _)) = self.fetch()? else {
                break;
            };
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
        let (v, _) = self.scan_dimen_inner()?;
        Ok(self.clamp_dimen(v))
    }

    /// TeX scan_dimen 末尾的尺寸钳制：|v| > 0x3FFFFFFF →
    /// "! Dimension too large." 并钳到 ±MAX_DIMEN（如 `\dimen45=\skip44` 读超大胶水）。
    fn clamp_dimen(&mut self, v: i64) -> i64 {
        if v > MAX_DIMEN {
            let _ = self.sink.write16("! Dimension too large.\n".to_string());
            MAX_DIMEN
        } else if v < -MAX_DIMEN {
            let _ = self.sink.write16("! Dimension too large.\n".to_string());
            -MAX_DIMEN
        } else {
            v
        }
    }

    /// 尺寸扫描（含胶水无穷阶）：返回 `(值, 阶)`。`scan_dimen` 丢弃阶；
    /// `scan_glue` 的 plus/minus 值用它取阶（TeX：`1pt plus 3fill`）。
    fn scan_dimen_inner(&mut self) -> Result<(i64, u8)> {
        self.skip_spaces()?;
        // TeX scan_dimen：跳过可选 `=` 赋值符（`\hsize=5in` 与 `\hsize 5in` 等价）
        if let Some((tok, _)) = self.fetch()? {
            if tok.charcode() != Some(b'=' as u32) {
                self.unread(tok);
            }
        }
        // 连续负号循环（TeX 表达式 `--\skip90` 等）+ 未定义 cs 跳过
        // （`-\mutoglue-\gluetomu9pt`，报错当 \relax 继续）：奇偶决定符号。
        let mut neg = false;
        loop {
            self.skip_spaces()?;
            let Some(tok) = self.fetch()?.map(|t| t.0) else {
                break;
            };
            if tok.charcode() == Some(b'-' as u32) {
                neg = !neg;
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
            }
            self.unread(tok);
            break;
        }
        // TeX get_x_token 语义：展开可展开 cs（`\ifdim\csname fontcharwd\endcsname...`）。
        // 未定义 cs 报 "! Undefined control sequence." 并当 \relax 继续。
        // 展开结果压回输入流顶，循环直至不可展开项或数量原语。
        loop {
            let Some(csid) = self.peek_csid()? else {
                break;
            };
            if matches!(self.eqtb.slot(csid), EqSlot::Undefined) {
                self.fetch()?; // 消费未定义 cs
                let _ = self
                    .sink
                    .write16(format!(
                        "! Undefined control sequence.\n\\{}\n",
                        self.intern.name(csid)
                    ));
                continue;
            }
            let expandable = match self.eqtb.slot(csid).clone() {
                EqSlot::Macro(m) => !(m.value.protected && self.suppress_expansion > 0),
                EqSlot::Primitive(p) if p.is_expandable() => true,
                _ => false,
            };
            if !expandable {
                break;
            }
            let (tok, ne) = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("扫描尺寸时输入耗尽"))?;
            let mut expansion = Vec::new();
            self.expand_once((tok, ne), &mut expansion)?;
            if expansion.is_empty() {
                continue;
            }
            self.stack.push(InputFrame::TokenList {
                items: Arc::from(expansion),
                pos: 0,
            });
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
                self.fetch()?; // 消费 \glueexpr/\muexpr
                let g = self.eval_glue_expression()?;
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
                    let _ = self.sink.write16("! Bad character code.\n".to_string());
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
            // ETRIP 第二波：\mutoglue<mu 胶水> / \gluetomu<胶水> → 胶水宽度（尺寸上下文）
            // 转换为胶水后取 width 分量（1mu = 1pt = 65536sp，数值不变）。
            if let EqSlot::Primitive(Primitive::MuToGlue | Primitive::GlueToMu) =
                self.eqtb.slot(csid)
            {
                self.fetch()?; // 消费 \mutoglue/\gluetomu
                let g = self.scan_glue()?;
                return Ok((if neg { -g.width } else { g.width }, 0));
            }
        }
        // 基数前缀：十六进制 `"` / 八进制 `'`（TeX scan_dimen 同 scan_int，TeXbook p.267）
        let mut radix_val: Option<i64> = None;
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
                    return Err(Error::invalid_input("预期尺寸数字"));
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
                        if matches!(p, Primitive::Count | Primitive::NumExpr)
                            || int_param_index(*p).is_some()
                );
            }
            if number_cs {
                int_part = self.scan_number()?;
                any = true;
            } else {
                while let Some((tok, _)) = self.fetch()? {
                    if let Some(d) = digit_value(tok) {
                        if saw_dot {
                            frac = frac * 10 + i64::from(d);
                            frac_len += 1;
                        } else {
                            int_part = int_part * 10 + i64::from(d);
                        }
                        any = true;
                    } else if tok.charcode() == Some(b'.' as u32) && !saw_dot {
                        saw_dot = true;
                    } else {
                        self.unread(tok);
                        break;
                    }
                }
            }
        }
        if !any {
            return Err(Error::invalid_input("预期尺寸数字"));
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
        const UNITS: &[&str] = &["sp", "pt", "bp", "in", "cm", "mm", "mu"];
        const ORDER_WORDS: &[&str] = &["fil", "fill", "filll"];
        let mut unit_tokens: Vec<(Token, char)> = Vec::new();
        while let Some((tok, _)) = self.fetch()? {
            let Some(ch) = tok.charcode().and_then(char::from_u32) else {
                self.unread(tok);
                break;
            };
            if !ch.is_ascii_alphabetic() {
                self.unread(tok);
                break;
            }
            unit_tokens.push((tok, ch));
        }
        let word: String = unit_tokens.iter().map(|(_, c)| c).collect();
        // 最长完整候选前缀（单位优先于阶词；同长按出现顺序取首个）
        let mut best: Option<(&str, usize)> = None; // (词, 长度)
        for u in UNITS.iter().chain(ORDER_WORDS.iter()) {
            if word.starts_with(u) && best.map_or(true, |(_, l)| u.len() > l) {
                best = Some((u, u.len()));
            }
        }
        let (unit, consumed, order) = match best {
            Some((u, len)) if ORDER_WORDS.contains(&u) => {
                // 阶词：消费，尺寸按 pt
                let order = match u {
                    "fil" => crate::register::order::FIL,
                    "fill" => crate::register::order::FILL,
                    "filll" => crate::register::order::FILLL,
                    _ => 0,
                };
                ("pt".to_owned(), len, order)
            }
            Some((u, len)) => (u.to_owned(), len, 0),
            None if unit_tokens.is_empty() => ("pt".to_owned(), 0, 0),
            None => (String::new(), 0, 0), // 未知单位：整体放回并报错
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
        if unit.is_empty() {
            return Err(Error::invalid_input(format!("未知单位：{word}")));
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

    /// 扫描胶水：可选前导胶水量（`\glueexpr`/`\skip<idx>`/`\muskip<idx>`/skipdef cs）
    /// 或 width + 可选 `plus <dimen>[fil]` / `minus <dimen>[fil]`。
    /// 非 plus/minus 字母（如正文）原样放回（TeX `scan_keyword` 语义）。
    fn scan_glue(&mut self) -> Result<Glue> {
        // M4-5 e-TeX：\glueexpr 可在任意胶水上下文求值
        self.skip_spaces()?;
        if let Some(csid) = self.peek_csid()? {
            match self.eqtb.slot(csid).clone() {
                EqSlot::Primitive(Primitive::Glueexpr) => {
                    self.fetch()?; // 消费 \glueexpr
                    return self.eval_glue_expression();
                }
                // ETRIP：`\hskip\skip5` 等 —— 前导胶水寄存器整体引用
                EqSlot::Primitive(Primitive::Skip) => {
                    self.fetch()?;
                    let idx = self.scan_register_index()?;
                    return Ok(self.registers.skip(idx));
                }
                EqSlot::Primitive(Primitive::Muskip) => {
                    self.fetch()?;
                    let idx = self.scan_register_index()?;
                    return Ok(self.registers.muskip(idx));
                }
                // ETRIP 第二波：\mutoglue<mu 胶水> / \gluetomu<胶水> → 胶水整体引用
                // （1mu = 1pt = 65536sp，数值不变；仅单位语义转换）
                EqSlot::Primitive(Primitive::MuToGlue | Primitive::GlueToMu) => {
                    self.fetch()?; // 消费 \mutoglue/\gluetomu
                    return self.scan_glue();
                }
                EqSlot::Register(kind, idx) => {
                    // skipdef/muskipdef 绑定的寄存器 cs
                    self.fetch()?;
                    return Ok(match kind {
                        RegKind::Skip => self.registers.skip(idx),
                        RegKind::Muskip => self.registers.muskip(idx),
                        _ => {
                            return Err(Error::invalid_input(
                                "胶水上下文需要 \\skip/\\muskip 寄存器",
                            ))
                        }
                    });
                }
                _ => {}
            }
        }
        let width = self.scan_dimen()?;
        let mut stretch = 0i64;
        let mut shrink = 0i64;
        let mut stretch_order = 0u8;
        let mut shrink_order = 0u8;
        for _ in 0..2 {
            let Some(word) = self.scan_keyword(|w| w == "plus" || w == "minus")? else {
                break;
            };
            // 值 + 无穷阶（scan_dimen_inner 消费 fil/fill/filll 阶后缀）
            let (d, order) = self.scan_dimen_inner()?;
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
        if is_kw(&word) {
            return Ok(Some(word));
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
