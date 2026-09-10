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
    ///
    /// 合法上界**由当前字体决定**（[`FontLoader::char_code_limit`]，M9 中文刀 1）：
    /// 8-bit TFM 字体 255——tex.web §1108 的 `! Bad character code (N).` 是
    /// TRIP/ETRIP 硬口径（`reference/trip/tripin.log`、`fixtures/etrip/etrip.log`
    /// 均含 `(256)` 参考块），不可放宽；Unicode 直映字体（OpenType）0x10FFFF，
    /// 对齐 XeTeX，使 `\char"4E00` 可排汉字。无字体（nullfont）时按 255。
    fn char_tokens(&mut self) -> Result<Vec<Token>> {
        let n = self.scan_number()?;
        let font = self.sink.current_font();
        let limit = i64::from(self.font_loader.char_code_limit(font));
        if !(0..=limit).contains(&n) {
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
            // active 字符 token（本引擎编码为带标志 cs token）：tex.web
            // shift_case 的判据 `cur_tok<cs_token_flag+single_base` 含 active
            // char——施表**换字符码、保持 active**。实测 pdftex（2026-09-08
            // 对拍 /tmp/ntex-r29/probe_a.tex：`\lccode126=35 \lowercase{~}`）
            // 产物报 `! Undefined control sequence. <recently read> #`——
            // 即 active char 35（若是 cat 6 字符会报 "You can't use macro
            // parameter character"）。转换 = 新字符码 intern 为名 + active 标志。
            if tok.is_active() {
                if let Some(csid) = tok.csid() {
                    let code = self.intern.name(csid).chars().next().map(|c| c as u32);
                    if let Some(code) = code.filter(|&c| c <= 0xff) {
                        let nv = table[code as usize];
                        if nv != 0 && nv != code as i64 {
                            if let Some(nc) = char::from_u32(nv as u32) {
                                let ncsid = self.intern.intern(&nc.to_string());
                                out.push(Token::active_sequence(ncsid));
                                continue;
                            }
                        }
                    }
                }
                out.push(tok);
                continue;
            }
            if let (Some(ch), Some(cc)) = (tok.charcode(), tok.catcode()) {
                // tex.web change_case（@1288）：判据是"字符 token"
                // （`info(p)<cs_token_flag+single_base`，含 active char），
                // **与 catcode 无关**——Letter/Other 之外，space（cat 10）、
                // `#`（cat 6）、`$`/`&`/`^`/`_`/`{`/`}` 等一律施 lccode/uccode。
                // 只认 Letter|Other 曾把 expl3 `\char_generate:nn` 查表
                // （l.9331-9352 的 `^^@` 全 catcode 臂）改得半残：cat 6 臂
                // 残留 char 0，l.9386 `\tl_const:Ne \c_catcode_other_space_tl`
                // 触发 Illegal parameter number。
                if ch <= 0xff {
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
