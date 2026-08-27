impl Expander {
    // ---------- M3 收尾（RFC-3）：VFS 副作用原语 ----------

    /// `\input<file>`：读文件内容推入 `Source` 输入帧（支持嵌套）。
    ///
    /// 文件名扫描（TeX `scan_file_name`）：`{file}` 花括号形式或普通形式
    /// （cat 11/12 字符，空格终止）。找不到时先试原名、再补 `.tex`。
    fn exec_input(&mut self) -> Result<()> {
        let name = self.scan_file_name()?;
        let mut content = self
            .vfs
            .read(&name)
            .map_err(|e| Error::io("VFS 读取", &name, e))?;
        if content.is_none() {
            let alt = format!("{name}.tex");
            content = self
                .vfs
                .read(&alt)
                .map_err(|e| Error::io("VFS 读取", &alt, e))?;
        }
        match content {
            Some(bytes) => {
                self.stack.push(InputFrame::Source {
                    bytes: Arc::from(bytes),
                    pos: 0,
                    state: ScanState::LineStart,
                });
                Ok(())
            }
            None => Err(Error::invalid_input(format!("找不到文件：{name}"))),
        }
    }

    /// 扫描文件名：`{...}` 或连续 cat 11/12 字符（空格终止）。
    fn scan_file_name(&mut self) -> Result<String> {
        self.skip_spaces()?;
        let first = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("扫描到输入末尾"))?
            .0;
        let mut name = String::new();
        if first.catcode() == Some(Catcode::BeginGroup) {
            // {file}：组内字符原样收集（含空格）
            loop {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("文件名组未闭合"))?
                    .0;
                match t.catcode() {
                    Some(Catcode::EndGroup) => break,
                    Some(Catcode::Letter) | Some(Catcode::Other) | Some(Catcode::Space) => {
                        let ch = t
                            .charcode()
                            .and_then(char::from_u32)
                            .ok_or_else(|| Error::invalid_input("文件名含非法字符"))?;
                        name.push(ch);
                    }
                    _ => return Err(Error::invalid_input("文件名含非法 token")),
                }
            }
        } else {
            self.unread(first);
            while let Some((t, _)) = self.fetch()? {
                // `\jobname`：展开为作业名（TeX 文件名扫描展开 \jobname）
                if let Some(csid) = t.csid() {
                    if self.eqtb.slot(csid) == &EqSlot::Primitive(Primitive::JobName) {
                        name.push_str("texput");
                        continue;
                    }
                    // 可展开项（\romannumeral/\number/\the/宏）：TeX get_x_token
                    // 语义——展开后重新收集（TRIP L94
                    // `\openout10=tr\romannumeral1 \gobble\newcs pos` → 流名
                    // "tripos"：\romannumeral1→"i"、\gobble 吞 \newcs、pos 收集）。
                    let slot = self.eqtb.slot(csid).clone();
                    let expandable = match &slot {
                        EqSlot::Macro(_) => true,
                        EqSlot::Primitive(p) => p.is_expandable(),
                        _ => false,
                    };
                    if expandable {
                        // 级别 2（\tracingcommands2）：tex.web expand() 开头
                        // `if tracing_commands>1 then show_cur_cmd_chr`——展开入口
                        // 也追踪。原语（\romannumeral 等）追踪；宏（\gobble 吞
                        // 参数）参考不追踪（TRIP L94 无 {\gobble}）。
                        if self.params.misc[3] >= 2 && !matches!(slot, EqSlot::Macro(_)) {
                            self.trace_token_now(t);
                        }
                        self.trace_suppress += 1;
                        let mut expansion = Vec::new();
                        let r = self.expand_once((t, false), &mut expansion);
                        let no_progress = expansion.len() == 1 && expansion[0].0 == t;
                        let r = r.and_then(|_| {
                            if no_progress {
                                // expand_once 不识别（\romannumeral 等）：exec
                                // 发射（压帧）后重新收集
                                match slot {
                                    EqSlot::Primitive(p) => self.exec_primitive(p),
                                    _ => Ok(()),
                                }
                            } else {
                                Ok(())
                            }
                        });
                        self.trace_suppress -= 1;
                        r?;
                        if !expansion.is_empty() && !no_progress {
                            let items: Vec<(Token, bool)> = expansion;
                            self.stack.push(InputFrame::TokenList {
                                items: Arc::from(items),
                                pos: 0,
                            });
                        }
                        continue; // 展开结果压帧，重新 fetch 收集
                    }
                }
                match t.catcode() {
                    Some(Catcode::Letter) | Some(Catcode::Other) => {
                        let ch = t
                            .charcode()
                            .and_then(char::from_u32)
                            .ok_or_else(|| Error::invalid_input("文件名含非法字符"))?;
                        name.push(ch);
                    }
                    _ => {
                        self.unread(t);
                        break;
                    }
                }
            }
        }
        if name.is_empty() {
            return Err(Error::invalid_input("缺少文件名"));
        }
        Ok(name)
    }

    /// 扫描流号（0..=max）；越界报 "! Bad number (n)." 并钳制（负数 → 0，
    /// 过大 → max），TeX 错误恢复语义（trip.tex L94 `\openout-'78terminal`
    /// → -7 → 钳 0，文件名 "8terminal"）。
    fn scan_stream_index(&mut self, _what: &str, max: i64) -> Result<usize> {
        let n = self.scan_number()?;
        if !(0..=max).contains(&n) {
            // TeX error() + show_context：`! 消息` + <to be read again> + l.N 两行
            //（TRIP L94 `\openout-'78` 的 Bad number 恢复段逐字对齐）
            self.write_error(&format!("Bad number ({n})."));
            let _ = self.sink.write16(format!(
                "Since I expected to read a number between 0 and {max},\n\
                 I changed this one to zero.\n"
            ));
            return Ok(if n < 0 { 0 } else { max as usize });
        }
        Ok(n as usize)
    }

    /// `\openin<n>=<file>`：文件存在 → 读入内存打开；不存在 → 流保持未打开（不报错）。
    fn exec_openin(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\openin", 15)?;
        self.expect_equals()?;
        let name = self.scan_file_name()?;
        let content = self
            .vfs
            .read(&name)
            .map_err(|e| Error::io("VFS 读取", &name, e))?;
        self.ensure_read_stream(idx);
        self.read_streams[idx] = content.map(|data| ReadStream {
            _path: name,
            data,
            pos: 0,
        });
        Ok(())
    }

    /// `\closein<n>`：关闭读流。
    fn exec_closein(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\closein", 15)?;
        self.ensure_read_stream(idx);
        self.read_streams[idx] = None;
        Ok(())
    }

    /// `\newwrite<cs>` / `\newread<cs>`：分配最小空闲流号，绑定到 cs（流引用）。
    fn exec_new_stream(&mut self, s: StreamKind) -> Result<()> {
        let t = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("流分配后缺少控制序列"))?
            .0;
        let csid = t
            .csid()
            .ok_or_else(|| Error::invalid_input("流分配后必须是控制序列"))?;
        let free = match s {
            // map_or 保持 MSRV 1.80（is_none_or 需 1.82）
            StreamKind::Read => (0..=15)
                .find(|&i| self.read_streams.get(i).map_or(true, |s| s.is_none())),
            StreamKind::Write => (0..=17)
                .find(|&i| self.write_streams.get(i).map_or(true, |s| s.is_none())),
        };
        let n = free.ok_or_else(|| Error::invalid_input("无空闲流号"))?;
        // cs 绑定为流引用（组作用域回滚，独立于 count 槽）
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Eqtb {
                    csid,
                    prev: self.eqtb.slot(csid).clone(),
                },
            ));
        }
        *self.eqtb.slot_mut(csid) = EqSlot::Stream(s, n);
        self.finish_assignment();
        Ok(())
    }

    /// `\read<n> to <cs>`：从流读一行，按当前 catcode 表 token 化，`\def` 赋给 cs。
    fn exec_read(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\read", 15)?;
        self.scan_keyword(|w| w == "to")?;
        self.skip_spaces()?;
        let t = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\read 后缺少控制序列"))?
            .0;
        let csid = t
            .csid()
            .ok_or_else(|| Error::invalid_input("\\read to 后必须是控制序列"))?;
        // 取下一行（到 \n 或文件末尾）
        let line = {
            let Some(stream) = self.read_streams.get_mut(idx).and_then(|s| s.as_mut()) else {
                return Err(Error::invalid_input("\\read 流未打开"));
            };
            if stream.pos >= stream.data.len() {
                return Err(Error::invalid_input("\\read 到文件末尾（EOF）"));
            }
            let start = stream.pos;
            let end = stream.data[start..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|i| start + i)
                .unwrap_or(stream.data.len());
            let line = stream.data[start..end].to_vec();
            stream.pos = if end < stream.data.len() { end + 1 } else { end };
            line
        };
        // token 化（catcode 表；行状态从行首开始——\read 每次读一行）
        let mut pos = 0usize;
        let mut state = ScanState::LineStart;
        let mut toks = Vec::new();
        while let Some(tok) = scan_token(&line, &mut pos, &self.catcodes, &mut self.intern, &mut state)?
        {
            // TeX：\read 的 token 列表禁止 outer 宏（tex.web read_toks）
            self.check_not_outer(tok)?;
            toks.push(tok);
        }
        // \def 语义赋值
        let def = MacroDef {
            params: ParamSpec {
                num_params: 0,
                long: false,
                text: Default::default(),
            },
            body: Arc::from(toks),
            code: None,
            protected: false,
            outer: false,
        };
        self.define_macro_scoped(csid, def);
        Ok(())
    }

    /// `\openout<n>=<file>`：登记写流目标路径（不立即创建文件）。
    fn exec_openout(&mut self) -> Result<()> {
        // TeX：\openout 流号 0..=15（trip.tex L94 报错消息 "between 0 and 15"）
        let idx = self.scan_stream_index("\\openout", 15)?;
        self.expect_equals()?;
        let name = self.scan_file_name()?;
        let immediate = self.take_immediate();
        self.ensure_write_stream(idx);
        self.write_streams[idx] = Some(WriteStream {
            path: Some(name.clone()),
            pending: Vec::new(),
        });
        if immediate {
            // \immediate\openout：立即创建（TeX 语义）
            self.vfs
                .write(&name, b"")
                .map_err(|e| Error::io("VFS 写入", &name, e))?;
        }
        Ok(())
    }

    /// `\closeout<n>`：flush 待写内容并关闭。
    fn exec_closeout(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\closeout", 15)?;
        // \closeout 恒 flush（immediate 前缀对 closeout 无额外效果，两分支等价）
        self.flush_write_stream(idx)?;
        self.ensure_write_stream(idx);
        self.write_streams[idx] = None;
        Ok(())
    }

    /// `\write<n><general text>`：token 列表入队（延迟）或立即展开落盘（`\immediate`）。
    fn exec_write(&mut self) -> Result<()> {
        // TeX 语义：\write 流号 -1..=18（-1 = log-only；16 = 终端；18 = shell）。
        let n = self.scan_number()?;
        if n == 18 {
            return Err(Error::invalid_input("\\write18（shell 转义）暂不支持"));
        }
        let toks = Arc::from(self.scan_general_text()?);
        // 消费 \immediate 前缀（流 15/16 终端写也须消费，避免污染后续 \write）
        let immediate = self.take_immediate();
        if n == -1 {
            // 流 -1：log-only（`\write-1{...}`）。\immediate 立即写 log；否则
            // whatsit 节点 + 延迟到 shipout 边界（TeX 语义，参考 log L44/L58）。
            if immediate {
                let s = self.expand_to_string(&toks)?;
                return self.sink.write16(s);
            }
            let text: String = toks
                .iter()
                .filter_map(|t| t.charcode())
                .filter_map(char::from_u32)
                .collect();
            self.sink.whatsit(text)?;
            self.log_write_pending.push(toks);
            return Ok(());
        }
        if !(0..=17).contains(&n) {
            // TeX：无效流号（如 trip.tex L137 `\write111`）→ 忽略 whatsit
            // （参考 log 显示 `.\write*{\help }`，不报错不写出）。
            let text: String = toks
                .iter()
                .filter_map(|t| t.charcode())
                .filter_map(char::from_u32)
                .collect();
            self.sink.whatsit(text)?;
            return Ok(());
        }
        let idx = n as usize;
        // 流 16 = 终端（TeX：\write16 写终端与日志，无需 \openout）；
        // ETRIP 的 \typeout/\error 用 \write15（同终端；TeX 预留流 15 作 log 输出）
        if idx == 16 || idx == 15 {
            let s = self.expand_to_string(&toks)?;
            // ETRIP 冲刺：记录最近 "Checking ..." 段标题（错误定位用）
            if s.starts_with("Checking ") {
                self.section_label = s.trim().to_owned();
            }
            return self.sink.write16(s);
        }
        self.ensure_write_stream(idx);
        if immediate {
            let s = self.expand_to_string(&toks)?;
            // TeX 语义：\immediate\write 到未打开的流 → 内容静默丢弃（trip.log
            // L431 `\immediate\write10`，流 10 已于 L153 \closeout）。
            let Some(path) = self
                .write_streams
                .get(idx)
                .and_then(|s| s.as_ref())
                .and_then(|st| st.path.clone())
            else {
                return Ok(());
            };
            let mut out = s;
            out.push('\n');
            self.vfs
                .append(&path, out.as_bytes())
                .map_err(|e| Error::io("VFS 写入", &path, e))?;
        } else {
            // 非 \immediate：TeX 在列表中留 whatsit 节点（文本延迟到 shipout 写出）。
            let text: String = toks
                .iter()
                .filter_map(|t| t.charcode())
                .filter_map(char::from_u32)
                .collect();
            self.sink.whatsit(text)?;
            self.write_streams[idx]
                .as_mut()
                .expect("exec_write 已 ensure 流槽")
                .pending
                .push(toks);
        }
        Ok(())
    }

    /// 扫描 `<general text>`：到 `\relax`（无条件）或外层组结束（吸收 `}`）为止。
    fn scan_general_text(&mut self) -> Result<Vec<Token>> {
        self.skip_spaces()?;
        let mut toks = Vec::new();
        let mut depth = 0usize;
        loop {
            let t = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("\\write 文本未闭合"))?
                .0;
            match t.catcode() {
                Some(Catcode::BeginGroup) => {
                    depth += 1;
                    // 最外层 { 是组定界符（被吸收），不计入文本；内层嵌套组保留
                    if depth > 1 {
                        toks.push(t);
                    }
                }
                Some(Catcode::EndGroup) => {
                    if depth == 0 {
                        break; // 组外 }：终止
                    }
                    depth -= 1;
                    if depth == 0 {
                        break; // 匹配到最外层 {：终止并吸收
                    }
                    toks.push(t);
                }
                _ => {
                    // \relax 无条件终止
                    if t.csid()
                        .is_some_and(|c| self.eqtb.slot(c) == &EqSlot::Primitive(Primitive::Relax))
                    {
                        break;
                    }
                    toks.push(t);
                }
            }
        }
        Ok(toks)
    }

    /// 把 token 列表展开成字符串（flush 边界写文件用）：完全展开后
    /// 字符 token → 字节、空格 → ` `；不可展开的 cs → 报错。
    fn expand_to_string(&mut self, toks: &[Token]) -> Result<String> {
        self.debug_expand_caller = "write";
        let expanded = self.expand_region(toks.to_vec())?;
        let mut s = String::new();
        for t in expanded {
            match t.catcode() {
                Some(Catcode::Space) => s.push(' '),
                Some(Catcode::Letter) | Some(Catcode::Other) => {
                    let ch = t
                        .charcode()
                        .and_then(char::from_u32)
                        .ok_or_else(|| Error::invalid_input("\\write 输出含非法字符"))?;
                    s.push(ch);
                }
                // 不可转字符的 token（\protected 宏、原语等）：TeX 语义为丢弃（不写内容）
                _ => {}
            }
        }
        Ok(s)
    }

    /// flush 单个写流：展开全部待写 token 并追加到目标文件（每条后加换行）。
    fn flush_write_stream(&mut self, idx: usize) -> Result<()> {
        let (path, pending) = {
            let Some(stream) = self.write_streams.get_mut(idx).and_then(|s| s.as_mut()) else {
                return Ok(()); // 未打开：无操作
            };
            if stream.pending.is_empty() {
                return Ok(());
            }
            (stream.path.clone(), std::mem::take(&mut stream.pending))
        };
        let Some(path) = path else {
            // TeX 语义：延迟 \write 到未打开的流在 shipout 时被忽略（内容丢弃）。
            return Ok(());
        };
        let mut out = String::new();
        for toks in pending {
            out.push_str(&self.expand_to_string(&toks)?);
            out.push('\n');
        }
        self.vfs
            .append(&path, out.as_bytes())
            .map_err(|e| Error::io("VFS 写入", &path, e))
    }

    /// flush 所有打开且有待写内容的写流（shipout 边界 / `\end` / 排版结束调用）。
    pub fn flush_writes(&mut self) -> Result<()> {
        for i in 0..self.write_streams.len() {
            self.flush_write_stream(i)?;
        }
        // TRIP：`\write-1`（log-only）待写内容展开后写 log（每条后加换行）。
        if !self.log_write_pending.is_empty() {
            let pending = std::mem::take(&mut self.log_write_pending);
            let mut out = String::new();
            for toks in pending {
                out.push_str(&self.expand_to_string(&toks)?);
                out.push('\n');
            }
            self.sink.write16(out)?;
        }
        Ok(())
    }

    /// 消费 `\immediate` 前缀。
    fn take_immediate(&mut self) -> bool {
        let v = self.immediate_pending;
        self.immediate_pending = false;
        v
    }

    /// 确保读流槽存在。
    fn ensure_read_stream(&mut self, idx: usize) {
        while self.read_streams.len() <= idx {
            self.read_streams.push(None);
        }
    }

    /// 确保写流槽存在（未打开时补空槽）。
    fn ensure_write_stream(&mut self, idx: usize) {
        while self.write_streams.len() <= idx {
            self.write_streams.push(None);
        }
        if self.write_streams[idx].is_none() {
            self.write_streams[idx] = Some(WriteStream {
                path: None,
                pending: Vec::new(),
            });
        }
    }

}
