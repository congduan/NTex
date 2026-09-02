// ---------- 字符代码原语执行器 ----------

// 从 expand/primitive.rs 迁出的字符 / 代码映射家族方法（catcode / sfcode / lccode
// / roman / char / uppercase / lowercase / case_convert 及其 _tokens 辅助）。
// 沿用项目已有的 "include! 分片" 模式（见 expand/mod.rs 末尾），每片维持独立
// `impl Expander { ... }` 块——Rust 允许同一类型的多个 impl 块分散在不同
// 文件，效果等价于单 impl。

impl Expander {
    fn exec_catcode(&mut self) -> Result<()> {
        let byte = self.scan_char_code()?;
        let byte = u8::try_from(byte).map_err(|_| Error::invalid_input("\\catcode 字符码越界"))?;
        self.expect_equals()?;
        let code = self.scan_number()?;
        let cat = match Catcode::from_u8(u8::try_from(code).unwrap_or(u8::MAX)) {
            Some(c) => c,
            // TeX assign_catcode（tex.web L3736-3744）：超 0..=15 → "Invalid code" 恢复，
            // 跳过赋值不中断（TRIP L429 `\catcode`\qq1qM=13` 中 scan_int 取 `\1`=49）。
            None => {
                let _ = self.sink.write16(format!(
                    "! Invalid code ({}), should be in the range 0..15.\n\
                     <to be read again> \nI didn't change it.\n",
                    code
                ));
                return Ok(());
            }
        };
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Catcode {
                    byte,
                    prev: self.catcodes.get(byte),
                },
            ));
        }
        self.catcodes.set(byte, cat);
        self.finish_assignment();
        Ok(())
    }

    /// `\sfcode<字符>=<值>`：设置字符的 spacefactor（TeX define_char_code 类）。
    fn exec_sfcode(&mut self) -> Result<()> {
        let byte = self.scan_char_code()?;
        let byte = u8::try_from(byte).map_err(|_| Error::invalid_input("\\sfcode 字符码越界"))?;
        self.expect_equals()?;
        let value = self.scan_number()?;
        let value = u32::try_from(value).map_err(|_| Error::invalid_input("\\sfcode 值越界"))?;
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Sfcode {
                    byte,
                    prev: self.sfcodes[byte as usize],
                },
            ));
        }
        self.sfcodes[byte as usize] = value;
        self.sink.sfcode_changed(byte, value)?;
        self.finish_assignment();
        Ok(())
    }

    /// `\lccode<char>=<num>`：设置字符的小写码（TeX assign_int；etrip 断字用）。
    /// 字符码接受反引号或寄存器值（如 `\lccode\count20=0`，TeX scan_char_num）。
    fn exec_lccode(&mut self) -> Result<()> {
        let mut byte = self.scan_char_code()?;
        if !(0..=255).contains(&byte) {
            // TeX scan_char_num：越界报 "Improper \lccode" 并钳制为 0（恢复继续，
            // trip.tex L26 `\lccode256-0`——TRIP 冲刺卡点）。
            let mut msg = "! Improper \\lccode.\n".to_string();
            if let Some((n, line)) = self.error_context() {
                msg.push_str(&format!("l.{n} {line}\n"));
            }
            let _ = self.sink.write16(msg);
            byte = 0;
        }
        let byte = byte as u8;
        self.expect_equals()?;
        let value = self.scan_number()?;
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::LcCode {
                    byte,
                    prev: self.lccodes[byte as usize],
                },
            ));
        }
        self.lccodes[byte as usize] = value;
        self.finish_assignment();
        Ok(())
    }

    /// TRIP：`\romannumeral<number>`：数字 → 小写罗马数字文本（tex.web
    /// `print_roman_numeral`；l.94 `\romannumeral1 \gobble`）。非正数 → 空；
    /// >4999 → "! Roman numeral too large." 并截断为 4999。输出字符为 other。
    fn exec_roman_numeral(&mut self) -> Result<()> {
        let out = self.roman_numeral_tokens()?;
        self.emit_tokens(out)
    }

    /// `\romannumeral` 展开 token 计算（exec 与 expand_once 共用；n≤0 → 空列表）。
    fn roman_numeral_tokens(&mut self) -> Result<Vec<Token>> {
        let mut n = self.scan_number()?;
        if n > 4999 {
            let mut msg = "! Roman numeral too large.\n".to_string();
            if let Some((ln, line)) = self.error_context() {
                msg.push_str(&format!("l.{ln} {line}\n"));
            }
            let _ = self.sink.write16(msg);
            n = 4999;
        }
        let mut out = Vec::new();
        if n > 0 {
            const TABLES: [(&str, i64); 13] = [
                ("m", 1000), ("cm", 900), ("d", 500), ("cd", 400), ("c", 100),
                ("xc", 90), ("l", 50), ("xl", 40), ("x", 10), ("ix", 9),
                ("v", 5), ("iv", 4), ("i", 1),
            ];
            let mut roman = String::new();
            for (sym, val) in TABLES {
                while n >= val {
                    roman.push_str(sym);
                    n -= val;
                }
            }
            for b in roman.bytes() {
                out.push(Token::char(Catcode::Other, u32::from(b)));
            }
        }
        Ok(out)
    }

    /// TRIP：`\char<num>`：字符码 → 输出 other 字符 token（tex.web scan_char_num；
    /// trip.tex l.195 `A /A\char`A`）。越界 → "! Bad character code (..)." 钳制 0。
    fn exec_char(&mut self) -> Result<()> {
        let toks = self.char_tokens()?;
        self.emit_tokens(toks)
    }

    /// `\char<num>` 展开 token 计算（exec 与 expand_once 共用）。
    fn char_tokens(&mut self) -> Result<Vec<Token>> {
        let n = self.scan_number()?;
        if !(0..=255).contains(&n) {
            let mut msg = format!("! Bad character code ({n}).\n");
            if let Some((ln, line)) = self.error_context() {
                msg.push_str(&format!("l.{ln} {line}\n"));
            }
            let _ = self.sink.write16(msg);
            return Ok(vec![Token::char(Catcode::Other, 0)]);
        }
        Ok(vec![Token::char(Catcode::Other, n as u32)])
    }
    /// TRIP：`\uppercase<general text>`：展开扫描 general text 后，按 `\uccode` 表
    /// 转换字符 token（tex.web upper_case；trip.tex l.96/97/338）。可展开项
    /// （如 `\number`）在扫描时展开，不可展开原语/组定界原样保留。
    fn exec_uppercase(&mut self) -> Result<()> {
        self.case_convert(true)
    }

    /// TRIP：`\lowercase<general text>`：同 `\uppercase`，按 `\lccode` 表转换。
    fn exec_lowercase(&mut self) -> Result<()> {
        self.case_convert(false)
    }

    fn case_convert(&mut self, upper: bool) -> Result<()> {
        let out = self.case_convert_tokens(upper)?;
        self.emit_tokens(out)
    }

    /// `\uppercase`/`\lowercase` 展开 token 计算（exec 与 expand_once 共用）。
    fn case_convert_tokens(&mut self, upper: bool) -> Result<Vec<Token>> {
        let toks = self.scan_general_text()?;
        let table = if upper { &self.uccodes } else { &self.lccodes };
        let mut out = Vec::with_capacity(toks.len());
        for tok in toks {
            if let (Some(ch), Some(cc)) = (tok.charcode(), tok.catcode()) {
                if matches!(cc, Catcode::Letter | Catcode::Other) && ch <= 0xff {
                    let nv = table[ch as usize];
                    if nv != 0 && nv != ch as i64 {
                        out.push(Token::char(cc, nv as u32));
                        continue;
                    }
                }
            }
            out.push(tok);
        }
        Ok(out)
    }

}
