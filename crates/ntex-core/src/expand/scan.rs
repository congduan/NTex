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
        if let Some(tok) = self.fetch()?.map(|t| t.0) {
            if tok.charcode() == Some(b'-' as u32) {
                neg = true;
            } else {
                self.unread(tok);
            }
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

    /// 扫描寄存器下标（0..=255）。
    fn scan_register_index(&mut self) -> Result<usize> {
        let n = self.scan_number()?;
        if !(0..REGISTER_COUNT as i64).contains(&n) {
            return Err(Error::invalid_input(format!("寄存器下标越界：{n}")));
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
        self.skip_spaces()?;
        // TeX scan_dimen：跳过可选 `=` 赋值符（`\hsize=5in` 与 `\hsize 5in` 等价）
        if let Some((tok, _)) = self.fetch()? {
            if tok.charcode() != Some(b'=' as u32) {
                self.unread(tok);
            }
        }
        let mut neg = false;
        if let Some(tok) = self.fetch()?.map(|t| t.0) {
            if tok.charcode() == Some(b'-' as u32) {
                neg = true;
            } else {
                self.unread(tok);
            }
        }
        // 寄存器引用：\dimen<idx>
        if let Some(csid) = self.peek_csid()? {
            if let EqSlot::Primitive(Primitive::Dimen) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \dimen
                let idx = self.scan_register_index()?;
                let v = self.registers.dimen(idx);
                return Ok(if neg { -v } else { v });
            }
            // M4-5 e-TeX：\dimexpr 可在任意尺寸上下文求值
            if let EqSlot::Primitive(Primitive::Dimexpr) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \dimexpr
                let v = self.eval_dimen_expression()?;
                return Ok(if neg { -v } else { v });
            }
            // ETRIP 冲刺：\fontdimen<num><font> 可在任意尺寸上下文读取
            if let EqSlot::Primitive(Primitive::FontDimen) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \fontdimen
                let num = self.scan_number()?;
                let num =
                    u32::try_from(num).map_err(|_| Error::invalid_input("\\fontdimen 参数号越界"))?;
                let font = self.scan_font_ident()?;
                let v = self.fontdimen(font, num);
                return Ok(if neg { -v } else { v });
            }
        }
        // 数字：整数部分 + 可选小数
        let mut int_part: i64 = 0;
        let mut frac: i64 = 0;
        let mut frac_len: u32 = 0;
        let mut any = false;
        let mut saw_dot = false;
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
        if !any {
            return Err(Error::invalid_input("预期尺寸数字"));
        }
        // 单位：连续字母（缺省 pt）
        let mut unit_tokens = Vec::new();
        while let Some((tok, _)) = self.fetch()? {
            if let Some(ch) = tok.charcode().and_then(char::from_u32) {
                if ch.is_ascii_alphabetic() {
                    unit_tokens.push(ch);
                    continue;
                }
            }
            self.unread(tok);
            break;
        }
        let unit = if unit_tokens.is_empty() {
            "pt".to_owned()
        } else {
            unit_tokens.into_iter().collect()
        };
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
        Ok(scaled)
    }

    /// 扫描胶水：width + 可选 `plus <dimen>` / `minus <dimen>`。
    /// 非 plus/minus 字母（如正文）原样放回（TeX `scan_keyword` 语义）。
    fn scan_glue(&mut self) -> Result<Glue> {
        // M4-5 e-TeX：\glueexpr 可在任意胶水上下文求值
        self.skip_spaces()?;
        if let Some(csid) = self.peek_csid()? {
            if let EqSlot::Primitive(Primitive::Glueexpr) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \glueexpr
                return self.eval_glue_expression();
            }
        }
        let width = self.scan_dimen()?;
        let mut stretch = 0i64;
        let mut shrink = 0i64;
        for _ in 0..2 {
            let Some(word) = self.scan_keyword(|w| w == "plus" || w == "minus")? else {
                break;
            };
            if word == "plus" {
                stretch = self.scan_dimen()?;
            } else {
                shrink = self.scan_dimen()?;
            }
        }
        Ok(Glue {
            width,
            stretch,
            shrink,
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
