impl Expander {
    // ---------- M3 收尾（RFC-3）：VFS 副作用原语 ----------

    /// `\input<file>`：读文件内容推入 `Source` 输入帧（支持嵌套）。
    ///
    /// 缺文件走 tex.web `prompt_file_name`（s="input file name"）的致命分支：
    /// `! I can't find file \`x'.` + show_context（e=".tex"）+ "Please type another
    /// input file name"，随后 `interaction < scroll_mode` → `fatal_error`。
    /// 引擎无交互层（无法交互询问替代文件名），一律按致命分支：报错进转录，
    /// `run()` 返回 `Err` 终止（`\@input`/`\IfFileExists` 类守卫探测走 `\openin`，
    /// 不经此处，见 latex.ltx L9889）。
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
                let bytes = Arc::from(bytes);
                self.push_frame(InputFrame::Source {
                    line_starts: Arc::from(crate::input::line_starts(&bytes)),
                    bytes,
                    pos: 0,
                    state: ScanState::LineStart,
                    eof_mark: None,
                });
                Ok(())
            }
            None => {
                self.write_error_help_no_read_again(
                    &format!("I can't find file `{name}'."),
                    "Please type another input file name\n",
                );
                self.sink.write16("! Emergency stop.".to_string())?;
                self.sink
                    .write16("*** (job aborted, file error in nonstop mode)".to_string())?;
                Err(Error::invalid_input(format!("找不到文件：{name}")))
            }
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
        } else if first.charcode() == Some(34) {
            // "file name"：web2c 对带引号文件名剥引号（kpathsea quote_name 语义，
            // 引号内空格保留）。latex.ltx 的 \IfFileExists/\@partaux 即此形式：
            // `\openin\@inputcheck"#1" `、`\immediate\openout\@partaux "#1.aux"`。
            loop {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("文件名引号未闭合"))?
                    .0;
                if t.charcode() == Some(34) {
                    break;
                }
                let ch = t
                    .charcode()
                    .and_then(char::from_u32)
                    .ok_or_else(|| Error::invalid_input("文件名含非法字符"))?;
                name.push(ch);
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
                            self.push_frame(InputFrame::TokenList {
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
                    // tex.web `scan_file_name`（L10210）：判据是
                    //   `if (cur_cmd>other_char)or(cur_chr>255) then back_input; goto done`
                    // 即**只要不是「命令」就按字符值 `cur_chr` 收集**，catcode 不参与
                    // 判定。`more_name`（L10023）只在**空格**处返回 false。
                    // 故下标 `_`(cat 8)、上标 `^`(cat 7)、参数 `#`(cat 6) 等
                    // **都算文件名字符**。
                    //
                    // ⚠ 实测缺此分支的后果：路径含 `_` 时文件名被截断（
                    // `/tmp/lvt-u_u/x` → 只扫到 `/tmp/lvt-u`）→
                    // `! 非法输入：找不到文件`。在 l3kernel 测试里表现为
                    // **随机的假 CRASH**（tempfile.mkdtemp 随机生成含 `_` 的目录名，
                    // 同一用例时通时不通），曾误导定位多轮（2026-09-11）。
                    Some(c) if !matches!(c, Catcode::Space) => {
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
            // tex.web：写流只有 0..15（16/17 是流号钳制哨兵，恒不可打开）；
            // LaTeX 的 `\newwrite` 同为 `\sixt@@n` 上限（latex.ltx L367）。
            StreamKind::Write => (0..=15)
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
                    prev_level: self.eqtb.level(csid),
                },
            ));
        }
        *self.eqtb.slot_mut(csid) = EqSlot::Stream(s, n);
        self.eq_mark_level(csid, global);
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
        // token 化（catcode 表；行状态从行首开始——\read 每次读一行；
        // M9 中文刀 2：\utfinputmode≠0 时该行同样按 UTF-8 解码）
        let mut pos = 0usize;
        let mut state = ScanState::LineStart;
        let mut toks = Vec::new();
        while let Some(tok) = scan_token(
            &line,
            &mut pos,
            &self.catcodes,
            &mut self.intern,
            &mut state,
            self.params.misc[crate::param::MISC_UTF_INPUT_MODE] != 0,
        )? {
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
            active_slot: false,
        };
        self.define_macro_scoped(csid, def);
        Ok(())
    }

    /// `\openout<n>=<file>`：登记写流目标路径。
    ///
    /// tex.web `open_out_file`：流号 0..=15；非 `\immediate` 的 `\openout` 是
    /// whatsit 节点（延迟到 shipout 才 `a_open_out`，页面被丢弃则文件不产生），
    /// 故此处只登记路径，截断/创建推迟到首次实际写出（`open_if_needed`）。
    /// `\immediate\openout` 立即创建（截断）。
    fn exec_openout(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\openout", 15)?;
        self.expect_equals()?;
        let name = self.scan_file_name()?;
        let immediate = self.take_immediate();
        while self.write_streams.len() <= idx {
            self.write_streams.push(None);
        }
        self.write_streams[idx] = Some(WriteStream {
            path: Some(name.clone()),
            created: false,
            pending: Vec::new(),
        });
        if immediate {
            self.open_if_needed(idx)?;
        }
        Ok(())
    }

    /// `\closeout<n>`：flush 待写内容并关闭。
    fn exec_closeout(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\closeout", 15)?;
        // \closeout 恒 flush（immediate 前缀对 closeout 无额外效果，两分支等价）。
        // 流未 `\openout`（无槽/无路径）→ no-op（tex.web：write_open[j]:=false）。
        self.flush_write_stream(idx)?;
        self.open_if_needed(idx)?;
        if let Some(slot) = self.write_streams.get_mut(idx) {
            *slot = None;
        }
        Ok(())
    }

    /// `\write<n><general text>`：token 列表入队（延迟）或立即展开落盘（`\immediate`）。
    ///
    /// tex.web `write_out`（§24847）：流号先钳制 `j`（负→17、>15→16，16/17 恒
    /// `write_open=false`），已 `\openout` 的 0..15 写文件；否则
    /// - `j=17`（负流号，`\write-1`/`\wlog`）→ 仅 log；
    /// - `j=16`（>15，LaTeX `\typeout`=`\write17`）**与未打开的 0..15** →
    ///   终端+log（tex.web「write to the terminal if file isn't open」——ETRIP 的
    ///   `\immediate\write15`、LaTeX 的 `\write\@unused` 都靠这条进转录）。
    fn exec_write(&mut self) -> Result<()> {
        let n = self.scan_number()?;
        if n == 18 {
            return Err(Error::invalid_input("\\write18（shell 转义）暂不支持"));
        }
        let toks: TokenArray = Arc::from(self.scan_general_text()?);
        // 消费 \immediate 前缀（终端/文件写都须消费，避免污染后续 \write）
        let immediate = self.take_immediate();
        // 已打开的文件流（0..=15 且 `\openout` 过）→ 写文件
        let open_idx = if (0..=15).contains(&n) {
            self.write_streams
                .get(n as usize)
                .and_then(|s| s.as_ref())
                .filter(|st| st.path.is_some())
                .map(|_| n as usize)
        } else {
            None
        };
        let Some(idx) = open_idx else {
            // 未打开/非文件流：`\immediate` 立即写转录（tex.web：未打开流 →
            // 「write to the terminal」，LaTeX 的 `\typeout`=`\immediate\write17`、
            // `\GenericError` 的 `\immediate\write\@unused` 都靠这条）；否则只留
            // whatsit 节点（box 追踪显示用）。
            let text: String = toks
                .iter()
                .filter_map(|t| t.charcode())
                .filter_map(char::from_u32)
                .collect();
            self.sink.whatsit(text)?;
            if immediate {
                // ETRIP 冲刺：记录最近 "Checking ..." 段标题（错误定位用）
                let s = self.expand_to_string(&toks)?;
                if s.starts_with("Checking ") {
                    self.section_label = s.trim().to_owned();
                }
                self.sink.write16(s)?;
            } else if n < 0 {
                // 负流号（j=17，log-only）：延迟到 shipout/结束边界写 log——
                // 既有路径（TRIP L441 `\write-100000` 的参考输出 `write->…` 已验证）
                self.log_write_pending.push(toks);
            }
            // 其余非 immediate（未打开的 0..15、流号 >15）：保持既有「无目标文件的
            // 延迟写忽略」简化。tex.web 在 shipout 的 write_out 会跳过 leaders 内的
            // whatsit（`if not doing_leaders`）；引擎 flush 不感知 leaders（whatsit
            // 节点只存文本），入队会把 TRIP L137 `\write111{\help}`（leaders 内、
            // `\help` 未定义）展开 → 运行中止。
            return Ok(());
        };
        if immediate {
            let s = self.expand_to_string(&toks)?;
            self.open_if_needed(idx)?;
            let mut out = s;
            out.push('\n');
            let path = self.write_streams[idx]
                .as_ref()
                .and_then(|st| st.path.clone())
                .expect("open_idx 已判定流已打开");
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
                .expect("open_idx 已判定流已打开")
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
                    // \relax 终止 general text —— **仅限未进入平衡组时**（TeX
                    // scan_toks 的 general text 语义：`\relax` 只作 `<filler>`
                    // 出现在必选 `{` 之前；平衡组内容一律是数据，含与 `\relax`
                    // 同义的 cs——`\csname` 制造出的 relax 在 `\lowercase{...}`
                    // 组内是普通 token）。旧实现无条件终止，expl3-code L205
                    // `\lowercase{\endgroup\def\PackageError#1...}` 中
                    // `\PackageError`（L199 `\csname` 刚制造为 relax）把组
                    // 扫描截断在 `\def` 后 → `\def` 后接字面 `#` → Missing
                    // control sequence 级联（报告 §18）。depth==0 只在开组
                    // `{` 之前出现（组闭合即 break），故该分支保留既有
                    // "裸 general text 遇 \relax 终止"行为。
                    if depth == 0
                        && t.csid().is_some_and(|c| {
                            self.eqtb.slot(c) == &EqSlot::Primitive(Primitive::Relax)
                        })
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
    /// 字符 token → 字节、空格 → ` `；不可展开的 cs → detokenize 语义打印
    /// （e-TeX `write_out` 用 `token_show`：escape 字符 + 名字，控制词补尾
    /// 空格——pdfTeX 实测 `\write\w{\foo}`（\foo protected）→ out.txt =
    /// `"\foo \n"`；旧实现丢弃不可展开 cs，protected 宏静默消失）。
    ///
    /// 偏差（有意，范围外）：tex.web `token_show` 对字符 token 一律印其字符，
    /// 此处非字母/其他 catcode 字符（math shift 等）仍丢弃——共享此函数的
    /// `\message`/`\show`/`\special` 输出须保持不变。副作用：引擎行模型
    /// LF=5（§latex-feasibility A1 偏差）把 `^^J` 归并为空格 token，故
    /// `\write{a^^Jb}` 写出空格而非 LF；latex.ltx L177 的 texsys.aux 探测
    /// 因此带尾空格（其 `\ifx` 对比失败 → 非致命 "BAD: old file" 噪声）。
    fn expand_to_string(&mut self, toks: &[Token]) -> Result<String> {
        self.debug_expand_caller = "write";
        let expanded = self.expand_region(toks.to_vec())?;
        let mut s = String::new();
        for t in expanded {
            match t.catcode() {
                Some(Catcode::Space) => s.push(' '),
                // 组定界字符：write 输出**字面** `{`/`}`（tex.web token_show：
                // 字符 token 一律印其字符，含组字符——pdfTeX 实测
                // `\write{A{B}C}` → `A{B}C`；expl3 消息组大量依赖。此前落
                // `_` 臂被丢弃，`\foo` 展开含组的 write 全部丢花括号）。
                Some(Catcode::BeginGroup) => s.push('{'),
                Some(Catcode::EndGroup) => s.push('}'),
                Some(Catcode::Letter) | Some(Catcode::Other) => {
                    let ch = t
                        .charcode()
                        .and_then(char::from_u32)
                        .ok_or_else(|| Error::invalid_input("\\write 输出含非法字符"))?;
                    s.push(ch);
                }
                // 不可展开 token：**已定义** cs 按 detokenize 语义打印（escape
                // 字符 + 名，控制词补尾空格，同 `detokenize_token`；pdfTeX 实测
                // 2026-09-12：`\write\w{\foo}`（protected）→ `\foo \n`，
                // `\immediate\write16{C=\count0}` → `C=\count 0`，
                // `{R=\relax X}` → `R=\relax X`）；**undefined cs 维持丢弃**——
                // tex.web 在 write 展开阶段报 Undefined control sequence 后恢复
                // 跳过该 token（输出面不含），本引擎展开层静默保留至构串（既有
                // 偏差），丢弃即对齐 pdfTeX 恢复后的输出面。其余 token 维持丢弃
                _ => {
                    if t.kind() == TokenKind::ControlSeq {
                        let csid = t.csid().expect("ControlSeq 必有 csid");
                        if self.eqtb.slot(csid) != &EqSlot::Undefined {
                            let name = self.intern.name(csid);
                            s.push_str(&self.escape_char_str());
                            s.push_str(name);
                            if name.bytes().next().is_some_and(|c| c.is_ascii_alphabetic()) {
                                s.push(' ');
                            }
                        }
                    }
                }
            }
        }
        Ok(s)
    }

    /// 打开（截断）写流目标文件：RFC-3 §4.4，`\openout` 语义为覆盖——首次实际
    /// 写出（`\immediate\write`/flush 边界/`\closeout`）清空目标，其后追加。
    fn open_if_needed(&mut self, idx: usize) -> Result<()> {
        let Some(stream) = self.write_streams.get_mut(idx).and_then(|s| s.as_mut()) else {
            return Ok(()); // 流未打开（无 `\openout`）：无操作
        };
        if stream.created {
            return Ok(());
        }
        let Some(path) = stream.path.clone() else {
            return Ok(());
        };
        self.vfs
            .write(&path, b"")
            .map_err(|e| Error::io("VFS 写入", &path, e))?;
        if let Some(stream) = self.write_streams.get_mut(idx).and_then(|s| s.as_mut()) {
            stream.created = true;
        }
        Ok(())
    }

    /// flush 单个写流：展开全部待写 token 并追加到目标文件（每条后加换行）。
    fn flush_write_stream(&mut self, idx: usize) -> Result<()> {
        let has_pending = self.write_streams
            .get(idx)
            .and_then(|s| s.as_ref())
            .map(|st| st.path.is_some() && !st.pending.is_empty())
            .unwrap_or(false);
        if !has_pending {
            return Ok(()); // 流未打开或无待写内容：无操作
        }
        let pending = {
            let stream = self.write_streams[idx]
                .as_mut()
                .expect("has_pending 已判定");
            std::mem::take(&mut stream.pending)
        };
        self.open_if_needed(idx)?;
        let path = self.write_streams[idx]
            .as_ref()
            .and_then(|st| st.path.clone())
            .expect("has_pending 已判定流已打开");
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
        // 转录流（`\write-1` log-only、未打开流、流号 >15）待写内容：
        // 每条一行（tex.web write_out 是 token_show + print_ln——每条恰一个换行，
        // 收尾换行由 write16 追加）。
        if !self.log_write_pending.is_empty() {
            let pending = std::mem::take(&mut self.log_write_pending);
            let mut out = String::new();
            for toks in pending {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&self.expand_to_string(&toks)?);
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

}
