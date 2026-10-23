use super::*;

impl Expander {
    /// 取下一个 token；返回 `(token, noexpand)`。输入耗尽或越过读取下限返回 None。
    pub(super) fn fetch(&mut self) -> Result<Option<(Token, bool)>> {
        let guard_limit = bytecode_guard_limit();
        let mut dispatches = 0u64;
        loop {
            if guard_limit != 0 {
                dispatches += 1;
                if self.bc_guard_active {
                    self.bc_guard_dispatches += 1;
                    dispatches = self.bc_guard_dispatches;
                }
                if dispatches > guard_limit {
                    self.dump_bytecode_guard(dispatches, guard_limit);
                    return Err(Error::invalid_input(format!(
                        "字节码 dispatch 超限（{dispatches} 次未返回 token；NTEX_BC_GUARD={guard_limit}）"
                    )));
                }
            }
            // 输入栈深度兜底上限（tex.web `stack_size` 语义）：[`Self::call_macro`]
            // 只盖宏帧，TokenList/Source 等帧的循环注入同样能把栈撑爆——此处统一
            // 兜底（每次压帧后必经 fetch，故为全帧型的唯一收口点）。
            if self.stack.len() > MAX_INPUT_STACK {
                self.dump_input_stack("fetch");
                return Err(Error::invalid_input(format!(
                    "输入栈超限（{} 帧 > {MAX_INPUT_STACK}）——展开/参数扫描疑似无终止条件",
                    self.stack.len()
                )));
            }
            // 不允许读取位于子展开边界（read_floor）以下的帧
            if self.stack.len() <= self.read_floor {
                return Ok(None);
            }
            let Some(frame) = self.stack.last_mut() else {
                return Ok(None);
            };
            match frame {
                InputFrame::Source {
                    bytes,
                    pos,
                    state,
                    eof_mark,
                    line_starts,
                    ..
                } => {
                    // `\endinput` force_eof（tex.web）：扫描越过截断点 =
                    // 文件提前结束。弹帧 + 注入 `\everyeof`（pdfTeX every_eof
                    // 扩展；expl3 \__sys_get 依赖）。
                    if let Some(mark) = *eof_mark {
                        if *pos >= mark {
                            self.stack.pop();
                            // tex.web：文件关闭即消费 force_eof（一次性旗标——
                            // 哪个文件先到行尾哪个关，其余文件幸存）。TRIP l.424：
                            // tripos 关闭时旗标熄灭，trip.tex 本体的 l.424 行尾
                            // 不得再触发。
                            self.force_eof = false;
                            let toks = std::mem::take(&mut self.everyeof_toks);
                            if !toks.is_empty() {
                                let seq: Vec<(Token, bool)> =
                                    toks.into_iter().map(|t| (t, false)).collect();
                                self.push_frame(InputFrame::TokenList {
                                    items: Arc::from(seq),
                                    pos: 0,
                                });
                            } else if self.scanner_status != ScannerStatus::Normal {
                                return Ok(None);
                            }
                            continue;
                        }
                    }
                    // 刀 N：`\endinput` force_eof 消费点——tex.web
                    // @<Read next line of file@>（L7566）/ @<Finish line@>：
                    // 旗标置位后**当前行**余下 token 照常交付；一旦要取下一行
                    // （行尾 / 读尽）即 `print_char(")"); force_eof:=false;
                    // end_file_reading`——一次性**全局**旗标，哪个文件先到行尾
                    // 哪个关，其余文件幸存（TRIP l.424：tripos 关闭时旗标熄灭，
                    // trip.tex 本体 l.424 行尾不得再触发）。三处收口，缺一即
                    // 泄漏给父帧：
                    //   ① 取词前 pos 已落在行边界（行尾 token 已交付）→ 关本帧；
                    //   ② 扫描期**越过行尾**（InSpace/注释臂 `continue` 吞掉行尾
                    //      后继续扫到下一行）→ 丢弃该 token、关本帧；
                    //   ③ 自然读尽（`Ok(None)`）→ 关本帧。
                    // 三者统一经 eof_mark 弹帧臂收场（`\everyeof` 注入 +
                    // scanner_status 门 + 熄旗标一处复用）。
                    if self.force_eof {
                        let cur_pos = *pos;
                        // pos=0（帧首行未读）豁免：tex.web start_input 读入首行，
                        // 旗标只在**第二次**取行时消费——新文件至少交付首行。
                        // 行归属判据见 [`Self::line_read_index`]：pos 恰在行边界
                        // （= 下一行起点）时 `line_starts[idx] == pos`，即本行读尽。
                        if cur_pos > 0 {
                            let idx = Self::line_read_index(line_starts, cur_pos);
                            let line_end = match line_starts.get(idx + 1) {
                                Some(&s) => s as usize,
                                None => bytes.len(),
                            };
                            if cur_pos >= line_end {
                                self.force_eof = false;
                                *eof_mark = Some(cur_pos);
                                continue;
                            }
                        }
                    }
                    // ② 越过行尾：记录取词前的行索引，扫描返回后若该索引变大 =
                    // 扫描器吞掉行尾继续扫到了下一行——本 token 属下一行，不得交付。
                    let eof_line_before = Self::line_read_index(line_starts, *pos);
                    match scan_token(
                        bytes,
                        pos,
                        &self.catcodes,
                        &mut self.intern,
                        state,
                        // M9 中文刀 2：\utfinputmode≠0 → 源码按 UTF-8 解码
                        //（只作用于字节→token 入口；宏体/实参 token 流不受影响，
                        // TRIP/ETRIP 默认 bytes 模式零影响）
                        self.params.misc[crate::param::MISC_UTF_INPUT_MODE] != 0,
                        // M9 中文刀 7：\endlinechar 动态传入（beamer 逐行消费器
                        // 的 #1^^M 定界依赖行尾 token 字符码/猫码随参数变化）
                        self.params.endlinechar,
                    ) {
                        Ok(Some(tok)) => {
                            if self.force_eof
                                && Self::line_read_index(line_starts, *pos) != eof_line_before
                            {
                                // 丢弃该 token（tex.web 关文件后不会回到本文件，
                                // 本行余下未读 token 一并作废）——走 eof_mark 臂
                                // 收场：弹帧 + 熄旗标 + 注入 `\everyeof`。
                                self.force_eof = false;
                                *eof_mark = Some(*pos);
                                continue;
                            }
                            return Ok(Some((tok, false)));
                        }
                        Ok(None) => {
                            // ③ 自然读尽 = 「取下一行」失败：本帧就是该消费旗标的
                            // 文件（tex.web：`print_char(")"); force_eof:=false;
                            // end_file_reading`）。不熄旗标则泄漏给**父帧**，父帧
                            // 在其下一行边界被 eof_mark 剪死——现场是 LaTeX 类
                            // 文件 `\input{size10.clo}` 之后整段类定义蒸发
                            // （size10.clo 末行 `\endinput` + 尾注释，扫描器在
                            // InSpace 状态一路扫到 EOF，行边界检查从未命中）。
                            self.force_eof = false;
                            self.stack.pop();
                            // 自然读尽同样注入 `\everyeof`（pdfTeX every_eof
                            // 语义：文件结束边界统一注入，无论正常 EOF 还是
                            // \endinput 截断；expl3 \__sys_get 依赖）。
                            let toks = std::mem::take(&mut self.everyeof_toks);
                            if !toks.is_empty() {
                                let seq: Vec<(Token, bool)> =
                                    toks.into_iter().map(|t| (t, false)).collect();
                                self.push_frame(InputFrame::TokenList {
                                    items: Arc::from(seq),
                                    pos: 0,
                                });
                            } else if self.scanner_status != ScannerStatus::Normal {
                                // tex.web `check_outer_validity` / `macro_call`：
                                // 宏实参、定义体等扫描语境中，**当前文件**耗尽即
                                // 是 runaway 边界；不能静默弹回父输入继续吞 token。
                                // 典型形态：`\input child`，child 内 `\act` 的定界
                                // 参数未闭合。GT 在 child EOF 处报
                                // "File ended while scanning use of \act."，插入
                                // `\par` 后继续读父文件，从而还能执行后续 \shipout。
                                return Ok(None);
                            }
                            continue;
                        }
                        // M1-13 错误恢复（TRIP L351）：cat 15 非法字符 → TeX
                        // "Text line contains an invalid character." + 跳过该字符继续
                        // （tex.web get_next invalid_char；scan_token 已消费该字节）。
                        Err(Error::InvalidCharacter { .. }) => {
                            let mut msg =
                                "! Text line contains an invalid character.\n".to_string();
                            if let Some((n, line)) = self.error_context() {
                                msg.push_str(&format!("l.{n} {line}\n"));
                            }
                            msg.push_str("A funny symbol that I can't read has just been input.\n");
                            msg.push_str("Continue, and I'll forget that it ever happened.\n");
                            let _ = self.sink.write16(msg);
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                }
                InputFrame::Macro { body, pos, args } => {
                    if *pos >= body.len() {
                        self.stack.pop();
                        continue;
                    }
                    let tok = body[*pos];
                    *pos += 1;
                    // 宏参数替换：#n → 实参（压帧展开）
                    if let Some(n) = tok.param_number() {
                        let arg = args
                            .get((n.saturating_sub(1)) as usize)
                            .cloned()
                            .unwrap_or_default();
                        if arg.is_empty() {
                            continue;
                        }
                        self.push_frame(InputFrame::MacroArg { items: arg, pos: 0 });
                        continue;
                    }
                    return Ok(Some((tok, false)));
                }
                InputFrame::Bytecode { code, pc, args } => {
                    if *pc >= code.len() {
                        self.stack.pop();
                        continue;
                    }
                    // M2-6：按 u64 原始字直接分发（零解包）。tag 0..=3 = token 原值
                    // 内联（RFC-1 布局），EMIT_ARG_TAG/END_TAG 见 bytecode.rs。
                    let word = code.words()[*pc];
                    match word >> crate::bytecode::TAG_SHIFT {
                        0..=3 => {
                            *pc += 1;
                            return Ok(Some((Token::from_raw(word), false)));
                        }
                        crate::bytecode::EMIT_ARG_TAG => {
                            *pc += 1;
                            let n = (word & 0xF) as u8;
                            let arg = args
                                .get(n.saturating_sub(1) as usize)
                                .cloned()
                                .unwrap_or_default();
                            if arg.is_empty() {
                                continue;
                            }
                            self.push_frame(InputFrame::MacroArg { items: arg, pos: 0 });
                            continue;
                        }
                        _ => {
                            // End
                            self.stack.pop();
                            continue;
                        }
                    }
                }
                InputFrame::TokenList { items, pos } => {
                    if *pos >= items.len() {
                        self.stack.pop();
                        continue;
                    }
                    let item = items[*pos];
                    *pos += 1;
                    return Ok(Some(item));
                }
                InputFrame::MacroArg { items, pos } => {
                    if *pos >= items.len() {
                        self.stack.pop();
                        continue;
                    }
                    let (tok, noexpand) = items[*pos];
                    *pos += 1;
                    return Ok(Some((tok, noexpand)));
                }
                InputFrame::One { tok, noexpand } => {
                    // 先拷贝（结束字段借用）再弹帧（&mut stack）
                    let (t, ne) = (*tok, *noexpand);
                    self.stack.pop();
                    return Ok(Some((t, ne)));
                }
                InputFrame::OutputRoutine { items, pos } => {
                    if *pos >= items.len() {
                        // 例程帧耗尽：关隐式组（恢复例程内赋值），复位输出例程
                        // 激活标志（可再次注入）。剩余待处理页面是否丢弃由
                        // maybe_inject_output 按进度判断（例程未取用 box255 时），
                        // 不在帧弹出时处理——例程末 token 的参数扫描（如 \box255
                        // 数字）会 fetch 到帧外。
                        self.stack.pop();
                        if self.output_group_level == Some(self.group_level) {
                            self.end_group()?;
                        } else {
                            self.sink.report_error("Unbalanced output routine.");
                            self.sink.report_help(
                                "Your sneaky output routine has problematic {'s and/or }'s.\n\
                                 I can't handle that very well; good luck.",
                            );
                        }
                        self.output_active = false;
                        self.output_group_level = None;
                        self.output_boundary_yield = true;
                        return Ok(None);
                    }
                    let item = items[*pos];
                    *pos += 1;
                    return Ok(Some(item));
                }
                // M4-5 对齐模板帧耗尽 hook（tex.web end_token_list 的 u/v 分支）
                InputFrame::AlignU { items, pos } => {
                    if *pos >= items.len() {
                        self.stack.pop();
                        self.align_u_exhausted();
                        continue;
                    }
                    let tok = items[*pos];
                    *pos += 1;
                    return Ok(Some((tok, false)));
                }
                InputFrame::AlignV { items, pos } => {
                    if *pos >= items.len() {
                        self.stack.pop();
                        self.align_fin_col()?;
                        continue;
                    }
                    let tok = items[*pos];
                    *pos += 1;
                    return Ok(Some((tok, false)));
                }
            }
        }
    }

    /// 帧是否「已耗尽」：下一个 [`Self::fetch`] 对它的唯一动作就是 `pop`
    /// （tex.web `loc=null`，module 6638 的定义）。
    ///
    /// **只有游标型帧（`pos`/`pc` 推进式）参与判定**。`One` 虽小却是
    /// 「弹出并返回 token」型——它在栈上时 token **尚未被消费**，永远不算
    /// 耗尽（`fetch` 的 One 臂 `pop` 与 `return Ok(Some(...))` 同时发生）。
    fn frame_depleted(f: &InputFrame) -> bool {
        match f {
            InputFrame::Bytecode { code, pc, .. } => {
                // bytecode 体末尾恒有一个 `End` 指令占位，而 tex.web 的 token
                // 链表以 `null` 收尾——故「已耗尽」= 越过 `words()` 尾 **或**
                // 游标正指 `End` 终止字（与 fetch 的 `_ =>` 臂同判据）。
                *pc >= code.len()
                    || code.words()[*pc] >> crate::bytecode::TAG_SHIFT
                        > crate::bytecode::EMIT_ARG_TAG
            }
            InputFrame::Macro { body, pos, .. } => *pos >= body.len(),
            InputFrame::TokenList { items, pos } => *pos >= items.len(),
            InputFrame::MacroArg { items, pos } => *pos >= items.len(),
            // 以下帧型**不参与** drain，理由各不相同（对照 fetch 各臂）：
            // - `Source`：tex.web 的循环条件 `state=token_list` 同样排除文件层；
            //   其弹出还要注入 `\everyeof`（非纯 pop）；
            // - `One`：见上，在栈上即未消费；
            // - `OutputRoutine`：弹出带 `end_group` + 复位输出例程激活标志；
            // - `AlignU`/`AlignV`：弹出带 `align_u_exhausted`/`align_fin_col`
            //   钩子（tex.web 只排除 v_template，此处从保守侧一并对齐排除）。
            InputFrame::Source { .. }
            | InputFrame::One { .. }
            | InputFrame::OutputRoutine { .. }
            | InputFrame::AlignU { .. }
            | InputFrame::AlignV { .. } => false,
        }
    }

    /// 清掉栈顶**已耗尽**的纯帧（tex.web `end_token_list` 的 "conserve stack
    /// space" 步骤）。
    ///
    /// tex.web module 7978（`macro_call` 的 `@<Feed the macro body and its
    /// parameters to the scanner@>`）与 module 7025（`back_input`）在**压新层
    /// 之前**都执行同一循环，原文附注即点明其目的：
    ///
    /// ```text
    /// while (state=token_list)and(loc=null)and(token_type<>v_template) do
    ///   end_token_list; {conserve stack space}
    /// ```
    ///
    /// > "Then a user macro that ends with a call to itself will not require
    /// >  unbounded stack space."
    ///
    /// 缺此步骤时，**尾递归宏**（自调用是该帧最后一串 token）每轮都把已耗尽的
    /// 调用者帧留在栈上：[`Self::fetch`] 只在帧成为**栈顶**时才弹它，而下一轮的
    /// 帧立刻压在其上——调用者永远等不到再次成为栈顶。栈深随迭代数线性增长。
    ///
    /// [实测 2026-09-15] 这是 expl3 载入在 l.36005 撞墙（`TeX capacity exceeded
    /// [input stack size]`）的**根因**，且与 expl3 无关——纯 plain TeX 复现：
    ///
    /// | 用例 | NTex（修复前）| pdfTeX（GT）|
    /// |---|---|---|
    /// | `\def\step{...\read...\step}` 逐行读完 UnicodeData.txt（34k 行）| 输入栈超限 | 正常读完 |
    /// | `\int_step_inline:nn {5000}{\relax}`（expl3 已载入）| 输入栈超限 | 正常（20000 亦无事）|
    ///
    /// 全栈转储佐证：5001 帧中 **4932 帧**是 `\__ior_map_variable_loop:NNNn`
    /// （每读一行漏 1 帧）、35 帧是 `\__int_step:Nw`，其余为作业帧。
    ///
    /// `read_floor` 是子展开（`\edef`/`\expanded`/`\csname`）边界，其下的帧
    /// 属外层上下文，不得越界——与 [`Self::fetch`] 的 `stack.len() <= read_floor`
    /// 同口径（fetch 允许弹到恰好 `read_floor`，本循环同）。
    pub(super) fn drain_depleted_frames(&mut self) {
        while self.stack.len() > self.read_floor {
            match self.stack.last() {
                Some(f) if Self::frame_depleted(f) => {
                    self.stack.pop();
                }
                _ => break,
            }
        }
    }

    /// 压入输入帧（全帧型统一入口；TEMP DEBUG 挂钩巨型 TokenList 定位）。
    ///
    /// 压栈前先跑 [`Self::drain_depleted_frames`]：tex.web 把该步骤放在
    /// `macro_call`/`back_input` 两处，这里收到唯一入口——不变量是「栈顶已耗尽
    /// 的纯帧对后续任何读取都不可见」，故在各调用点均等价（见该函数文档）。
    pub(super) fn push_frame(&mut self, f: InputFrame) {
        self.drain_depleted_frames();
        #[cfg(not(target_arch = "wasm32"))]
        if diag_enabled("NTEX_BIGLIST_TRACE") {
            if let InputFrame::TokenList { items, .. } = &f {
                if items.len() > 300_000 {
                    let head: Vec<String> = items
                        .iter()
                        .take(40)
                        .map(|t| match t.0.csid() {
                            Some(id) => format!("\\{}", self.intern.name(id)),
                            None => format!("c{}", t.0.charcode().unwrap_or(9999)),
                        })
                        .collect();
                    eprintln!(
                        "[bigpush] {} tok last_tok={:?} head={:?}",
                        items.len(),
                        self.last_tok,
                        head
                    );
                }
            }
        }
        self.stack.push(f);
        // 结构化 trace（JSONL）：记录每次压帧——定位「栈为什么膨胀」的关键。
        if crate::expand::trace::enabled() {
            let (tok, frame) = match self.stack.last() {
                Some(fr) => {
                    let desc = match fr {
                        InputFrame::TokenList { items, .. } => {
                            format!("TokenList({}tok)", items.len())
                        }
                        InputFrame::MacroArg { items, .. } => {
                            format!("MacroArg({}tok)", items.len())
                        }
                        InputFrame::Macro { body, .. } => {
                            let name = body
                                .first()
                                .and_then(|t| t.csid())
                                .map(|id| self.intern.name(id).to_string())
                                .unwrap_or_default();
                            format!("Macro({},{}tok)", name, body.len())
                        }
                        InputFrame::One { .. } => "One".to_string(),
                        InputFrame::OutputRoutine { items, .. } => {
                            format!("OutputRoutine({}tok)", items.len())
                        }
                        InputFrame::Source { bytes, .. } => {
                            format!("Source({}B)", bytes.len())
                        }
                        InputFrame::Bytecode { .. } => "Bytecode".to_string(),
                        InputFrame::AlignU { items, .. } => {
                            format!("AlignU({}tok)", items.len())
                        }
                        InputFrame::AlignV { items, .. } => {
                            format!("AlignV({}tok)", items.len())
                        }
                    };
                    (self.last_tok.clone(), desc)
                }
                None => (None, String::new()),
            };
            crate::expand::trace::event(
                self.steps,
                crate::expand::trace::KIND_PUSH,
                tok.as_deref(),
                None,
                self.stack.len(),
                Some(&frame),
                None,
            );
        }
    }

    /// 把 token 放回输入流（等价于压入单元素 token 列表帧）。
    pub(super) fn unread(&mut self, tok: Token) {
        self.push_frame(InputFrame::TokenList {
            items: Arc::from([(tok, false)]),
            pos: 0,
        });
    }
}
