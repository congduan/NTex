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
            loop {
                let Some((t, _)) = self.fetch()? else { break };
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

    /// 扫描流号（0..=max）。
    fn scan_stream_index(&mut self, what: &str, max: i64) -> Result<usize> {
        let n = self.scan_number()?;
        if !(0..=max).contains(&n) {
            return Err(Error::invalid_input(format!("{what} 流号越界：{n}")));
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
            StreamKind::Read => (0..=15)
                .find(|&i| self.read_streams.get(i).is_none_or(|s| s.is_none())),
            StreamKind::Write => (0..=17)
                .find(|&i| self.write_streams.get(i).is_none_or(|s| s.is_none())),
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
        // token 化（catcode 表）
        let mut pos = 0usize;
        let mut toks = Vec::new();
        while let Some(tok) = scan_token(&line, &mut pos, &self.catcodes, &mut self.intern)? {
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
        };
        self.define_macro_scoped(csid, def);
        Ok(())
    }

    /// `\openout<n>=<file>`：登记写流目标路径（不立即创建文件）。
    fn exec_openout(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\openout", 17)?;
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
        let idx = self.scan_stream_index("\\closeout", 17)?;
        let immediate = self.take_immediate();
        if !immediate {
            self.flush_write_stream(idx)?;
        } else {
            self.flush_write_stream(idx)?;
        }
        self.ensure_write_stream(idx);
        self.write_streams[idx] = None;
        Ok(())
    }

    /// `\write<n><general text>`：token 列表入队（延迟）或立即展开落盘（`\immediate`）。
    fn exec_write(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\write", 18)?;
        if idx == 18 {
            return Err(Error::invalid_input("\\write18（shell 转义）暂不支持"));
        }
        let toks = Arc::from(self.scan_general_text()?);
        // 流 16 = 终端（TeX：\write16 写终端与日志，无需 \openout）
        if idx == 16 {
            let s = self.expand_to_string(&toks)?;
            return self.sink.write16(s);
        }
        self.ensure_write_stream(idx);
        if self.take_immediate() {
            let s = self.expand_to_string(&toks)?;
            let path = self
                .write_streams
                .get(idx)
                .and_then(|s| s.as_ref())
                .and_then(|st| st.path.clone())
                .ok_or_else(|| Error::invalid_input("\\write 到未打开的流"))?;
            let mut out = s;
            out.push('\n');
            self.vfs
                .append(&path, out.as_bytes())
                .map_err(|e| Error::io("VFS 写入", &path, e))?;
        } else {
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
                    toks.push(t);
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
            return Err(Error::invalid_input("\\write 到未打开的流"));
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
