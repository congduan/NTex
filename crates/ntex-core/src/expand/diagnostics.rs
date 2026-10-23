use super::*;

impl Expander {
    /// TeX final_cleanup：未闭合条件 → 可恢复转录消息
    /// "(end occurred when \ifxxx on line N was incomplete)"
    /// （GT trip.log 尾部 L442/419/413 同款；tex.web final_cleanup 在 \end 后
    /// 按条件栈 LIFO 逐层弹出的警告。中途中止的 skip/EOF 恢复才用
    /// "! Incomplete ...; all text was ignored" 形——两形不可混用。）
    pub(super) fn report_incomplete_conditions(&mut self) {
        if self.cond_stack.is_empty() {
            return;
        }
        #[cfg(debug_assertions)]
        {
            let frames: Vec<String> = self
                .cond_stack
                .iter()
                .map(|f| {
                    format!(
                        "{{is_case={} state={:?} owns_skip={} else={} line={}}}",
                        f.is_case, f.state, f.owns_skip, f.else_seen, f.line
                    )
                })
                .collect();
            eprintln!(
                "[debug] 条件未闭合: depth={} frames={:?}",
                self.cond_stack.len(),
                frames
            );
        }
        for f in self.cond_stack.iter().rev() {
            let _ = self.sink.write16(format!(
                "(end occurred when {} on line {} was incomplete)\n",
                Self::if_type_name(f.if_type).trim_start_matches('\\'),
                f.line
            ));
        }
        self.cond_stack.clear();
    }

    /// A3：当前源码上下文——从输入栈找最近的 [`InputFrame::Source`] 帧，
    /// 当前输入行号（纯 pos 计算——不经过 error_context 的 error_anchor 锚点，
    /// 锚点是 clamp_dimen 报错回溯用，会残留污染 \inputlineno 的值）。
    pub(super) fn current_line_no(&self) -> usize {
        for frame in self.stack.iter().rev() {
            if let InputFrame::Source {
                line_starts, pos, ..
            } = frame
            {
                // 预建行索引二分（原实现每帧从头数 \n——组事件高频调用下的 O(pos) 热点）。
                return line_starts
                    .partition_point(|&s| (s as usize) <= *pos)
                    .max(1);
            }
        }
        0
    }

    /// `\endinput` 行归属判据：**已交付的最后一个字节**（`pos-1`）所在行的
    /// 0 基行号（`pos == 0` 即帧首、尚无已交付字节 → 第 0 行）。
    ///
    /// 同一行内任意 `pos` 同值，且「行尾已交付、pos 恰停在下一行起点」时也
    /// 仍属**上一行**——故扫描前后比较即可判定「是否越过行尾」：
    /// - 相等：本轮 token 属本行（含行尾 token 本身）；
    /// - 变大：扫描器吞掉行尾（InSpace/注释臂 `continue`）后扫到了下一行。
    pub(super) fn line_read_index(line_starts: &[u32], pos: usize) -> usize {
        line_starts
            .partition_point(|&s| (s as usize) <= pos.saturating_sub(1))
            .saturating_sub(1)
    }

    /// 行定位（`line_starts` 二分）：返回 (行号, 行起始偏移, 行结束偏移)。
    ///
    /// 行结束偏移不含行尾 `\n`（与逐字节扫描一致）。替代原 `error_context`/
    /// `error_context_pos` 的两次 O(pos) 全文扫描——条件帧入栈（\if 高频）也取
    /// 行号，长文档上线性扫描整体 O(n²)（0b2fc9b 对 `current_line_no` 同类修复
    /// 的补全）。
    pub(super) fn locate_line(
        bytes: &[u8],
        line_starts: &[u32],
        end: usize,
    ) -> (usize, usize, usize) {
        let line_no = line_starts.partition_point(|&s| (s as usize) <= end).max(1);
        let line_start = line_starts[line_no - 1] as usize;
        // 下一行起始 = 本行行尾 `\n` 下标 + 1（末行无下一行 → 全文长度）
        let line_end = match line_starts.get(line_no) {
            Some(&s) => (s as usize).saturating_sub(1).min(bytes.len()),
            None => bytes.len(),
        };
        (line_no, line_start, line_end)
    }

    /// 由扫描位置反推 (行号, 行内容)。宏展开中的错误回退到最近的源文件行
    /// （TeX `l.N` 上下文行语义）。
    pub(super) fn error_context(&self) -> Option<(usize, String)> {
        for frame in self.stack.iter().rev() {
            if let InputFrame::Source {
                bytes,
                line_starts,
                pos,
                ..
            } = frame
            {
                let bytes: &[u8] = bytes;
                // 报错锚点（clamp_dimen 值扫描报错）：pos 已推进到报错后的行，
                // 用值扫描起始位置回溯（tex.web l.N 上下文停在值所在行）。
                // 只读（&self）；消费由各报错出口负责（undef 路径自行清除，
                // report_error_context 出口见下）。
                let end = self.error_anchor.unwrap_or(*pos).min(bytes.len());
                let (line_no, line_start, line_end) = Self::locate_line(bytes, line_starts, end);
                let line = String::from_utf8_lossy(&bytes[line_start..line_end]).into_owned();
                return Some((line_no, line));
            }
        }
        None
    }

    /// A3：把错误上下文行（`l.N <行内容>`）写入转录（TeX error() 的上下文行）。
    /// 尾 \n 由 write16 自动追加（与 write_error 的 `l.N\n\n` 块分隔一致）。
    pub(super) fn report_error_context(&mut self) {
        if let Some((n, line)) = self.error_context() {
            let _ = self.sink.write16(format!("l.{n} {line}\n"));
        }
        // 锚点一次性使用（报错上下文输出后清除——后续报错用当前 pos）
        self.error_anchor = None;
    }

    /// 仅取错误上下文行号（不构建行内容字符串）。
    ///
    /// 条件帧入栈（每次 `\if*`）只记行号供 `! Incomplete \ifxxx ... after line N`
    /// 用——此前走 [`Self::error_context`] 会把**整行**做 UTF-8 转换（长行样张上
    /// 每 `\if` 一次百 KB 级拷贝）。行号计算与 `error_context` 完全一致（同一
    /// `error_anchor` 回溯语义），逐位等价。
    pub(super) fn error_line_no(&self) -> usize {
        for frame in self.stack.iter().rev() {
            if let InputFrame::Source {
                bytes,
                line_starts,
                pos,
                ..
            } = frame
            {
                let end = self.error_anchor.unwrap_or(*pos).min(bytes.len());
                let (line_no, _, _) = Self::locate_line(bytes, line_starts, end);
                return line_no;
            }
        }
        0
    }

    /// 错误上下文带行内错误位置（Source 帧 pos 相对行首的偏移）。
    fn error_context_pos(&self) -> Option<(usize, String, usize)> {
        for frame in self.stack.iter().rev() {
            if let InputFrame::Source {
                bytes,
                line_starts,
                pos,
                ..
            } = frame
            {
                let bytes: &[u8] = bytes;
                let end = (*pos).min(bytes.len());
                let (line_no, line_start, line_end) = Self::locate_line(bytes, line_starts, end);
                let line = String::from_utf8_lossy(&bytes[line_start..line_end]).into_owned();
                let pos = end.saturating_sub(line_start).min(line.len());
                return Some((line_no, line, pos));
            }
        }
        None
    }

    /// TeX 错误恢复模式统一入口：`! 消息` 写入转录 + `l.N 上下文行`（tex.web
    /// error()：报错后继续执行，不终止作业）。恢复动作由调用方决定（钳制/插入/
    /// 忽略）；`<to be read again>` 与 help 行由 write_error 等完整格式路径输出。
    pub(super) fn report_error(&mut self, msg: &str) {
        if self.err_snapshot.is_none() {
            self.err_snapshot = Some((self.group_level, self.cond_stack.len()));
        }
        self.sink.report_error(msg);
        self.report_error_context();
    }

    /// ETRIP P0 \muexpr 校准：错误帮助行（tex.web help1..help5）转发到 sink。
    /// mu_error 等场合在 `! <msg>` 后追加一行提示（如 etrip.tex L906："I'm going to
    /// assume that 1mu=1pt when they're mixed."），与参考 log 字面对齐。
    pub(super) fn report_help(&mut self, text: &str) {
        self.sink.report_help(text);
    }

    /// M1-13 刀C：报错（无 `<to be read again>` 段）+ interaction 门控 help——
    /// tex.web error() 的 help 行只在 errorstopmode 停等时打出（TinyTeX 实测：
    /// nonstopmode 转录无 help 行）。引擎无交互停等，errorstopmode(3) 按
    /// 「用户在 `? ` 提示后敲 h」模拟成 `? <help>` 块（help 文本接在提示符
    /// 同行、块尾空行），与 GT errorstopmode 实测逐字对齐。既有
    /// write_error_help* 调用面的无条件 help 本刀不动，新报错站一律走此门控。
    pub(super) fn write_error_stop_aware(&mut self, msg: &str, help: &str) {
        self.write_error_help_no_read_again(msg, "");
        if !help.is_empty() && self.params.misc[crate::param::MISC_INTERACTION_MODE] == 3 {
            let _ = self.sink.write16(format!("? {help}\n\n"));
        }
    }

    /// M1-13 刀C：`\errhelp` 消费侧——toks 内容按 token_show 语义串化（tex.web
    /// issue_message 把 err_help toks 经 tokens_to_string 交给 error() 打印，
    /// 打印不展开：存的是扫描期 token、显示即字面）。字符印字面、幸存 cs 按
    /// detokenize 语义印名（escape 字符 + 名 + 控制词尾空格）。
    pub(super) fn errhelp_text(&self) -> String {
        let mut s = String::new();
        for t in &self.errhelp_toks {
            match t.catcode() {
                Some(_) => {
                    if let Some(ch) = t.charcode().and_then(char::from_u32) {
                        s.push(ch);
                    }
                }
                _ => {
                    if let Some(ch) = t.active_charcode(&self.intern) {
                        s.push(ch);
                    } else if let Some(csid) = t.csid() {
                        s.push_str(&self.escape_char_str());
                        let name = self.intern.name(csid);
                        s.push_str(name);
                        if name.bytes().next().is_some_and(|c| c.is_ascii_alphabetic()) {
                            s.push(' ');
                        }
                    }
                }
            }
        }
        s
    }

    /// NTEX_SANITY_CHECK 设施：错误恢复后状态完整性校验。
    /// 在主循环每次迭代开始调用——若存在待校验快照（刚发生过错误恢复），
    /// 对比当前 (组级, 条件栈深) 与错误前：组级偏离 >1 或条件栈偏离 >1 视为
    /// 恢复路径写坏了状态（内部 bug），NTEX_SANITY_CHECK=1 时打警告。
    /// 快照一律清除（一次错误只校验一次）。
    pub(super) fn sanity_check_after_error(&mut self) {
        let Some((g0, c0)) = self.err_snapshot.take() else {
            return;
        };
        let dg = self.group_level.abs_diff(g0);
        let dc = (self.cond_stack.len() as i64 - c0 as i64).abs();
        if diag_enabled("NTEX_SANITY_CHECK") && (dg > 1 || dc > 1) {
            eprintln!(
                "[sanity] 错误恢复后状态偏离：组级 {g0}->{} (Δ{dg})，条件栈 {c0}->{} (Δ{dc}) last_tok={:?}",
                self.group_level,
                self.cond_stack.len(),
                self.last_tok
            );
        }
    }

    /// 错误上下文 token 的简单显示（TeX show_token_list：字符直接显示、cs 显示 `\名`）。
    fn trace_tok_simple(&self, tok: Token) -> String {
        if let Some(csid) = tok.csid() {
            format!("\\{}", self.intern.name(csid))
        } else if let Some(ch) = tok.charcode() {
            char::from_u32(ch).unwrap_or('?').to_string()
        } else {
            "?".to_string()
        }
    }

    /// 统一错误消息输出（TeX error()/show_context 语义）：`! 消息` 后接
    /// `<to be read again>` 段（peek 输入流下一个 token，18 列）与 `l.N`
    /// 行上下文两行显示（位置前内容；n 空格 + 位置后 ≤trick 字符 + `...`）。
    /// TRIP 对齐：参考 log L9-11 的 `<to be read again>` / l.94 段。
    ///
    /// 该格式逐字对齐 web2c TRIP 参考输出（error_line=79 的 show_context 伪打印）。
    pub(super) fn write_error(&mut self, msg: &str) {
        self.write_error_impl(msg, true, None);
    }

    /// 报错 + help 一体输出（无 `<to be read again>`，TeX error() 语义：报错
    /// 发生时无 back_input 待读 token——主循环"裸用内部量"类错误，如 etrip
    /// gluestretchorder 段 l.932/l.933 的 4 个 "You can't use ..."）。help
    /// 紧随 l.N 上下文行后、错误块尾空行由 write16 追加的 \n 产生（块结构：
    /// ! 消息 / l.N 两行 / help 4 行 / 空行——与参考逐行一致）。扫描参数失败
    /// 的报错才带 read-again（write_error，如 Missing number / Bad register
    /// code）。
    pub(super) fn write_error_help_no_read_again(&mut self, msg: &str, help: &str) {
        self.write_error_impl(msg, false, Some(help));
    }

    /// 报错 + help 一体输出（带 `<to be read again>`，扫描参数失败且报错后
    /// 输入流仍有待处理 token 时——etrip sparse arrays 段 l.970-973 的
    /// "Bad register code" 块即此类：read-again token 由 fetch() 取输入流
    /// 下一 token（宏体 `#1\1=-1#1...` 场景恰为再次出现的 `\countdef` 等，
    /// 与参考 `<to be read again>` 后的 token 一致）。块结构与
    /// write_error_help_no_read_again 相同，只是 error 后先输出 read-again 段）。
    pub(super) fn write_error_help(&mut self, msg: &str, help: &str) {
        self.write_error_impl(msg, true, Some(help));
    }

    pub(super) fn misplaced_alignment_tab(&mut self) {
        self.write_error_help_no_read_again(
            "Misplaced alignment tab character &.",
            "I can't figure out why you would want to use a tab mark\n\
             here. If you just want an ampersand, the remedy is\n\
             simple: Just type `I\\&' now. But if some right brace\n\
             up above has ended a previous alignment prematurely,\n\
             you're probably due for more error messages, and you\n\
             might try typing `S' now just to see what is salvageable.\n",
        );
    }

    /// 报错 + `<inserted text>` 段 + `<to be read again>` + help 一体输出
    /// （check_outer_validity defining/aligning 臂的恢复块形态；插入 token
    /// 已由调用方 unread，read-again 段由输入流 peek 显示它）。
    pub(super) fn write_error_insert_help(&mut self, msg: &str, inserted: &str, help: &str) {
        self.write_error_impl_ins(msg, true, Some(inserted), Some(help));
    }

    /// write_error 实现：read_again=true 时先输出 `<to be read again>` 段
    /// （TRIP 对齐：扫描参数失败时错误恢复后将被读取的 token）。
    fn write_error_impl(&mut self, msg: &str, read_again: bool, help: Option<&str>) {
        self.write_error_impl_ins(msg, read_again, None, help);
    }

    /// 带插入段变体：`<inserted text>` 段位于 `! 消息` 与 `<to be read again>`
    /// 之间（tex.web check_outer_validity 的 defining/aligning 恢复块：
    /// ins_error 的 inserted token 先于 read-again 上下文显示——trip.log
    /// L5172-5173 `<inserted text> }` → `<to be read again> \a^^@^^@a`）。
    fn write_error_impl_ins(
        &mut self,
        msg: &str,
        read_again: bool,
        inserted: Option<&str>,
        help: Option<&str>,
    ) {
        let mut s = format!("! {msg}\n");
        if let Some(txt) = inserted {
            s.push_str("<inserted text> \n");
            s.push_str(txt);
        }
        // <to be read again>：错误恢复后将被读取的 token（peek 输入流下一个）；
        // 第二行 = 描述宽度（19：`<to be read again> `）空格 + token 直接显示。
        if read_again {
            if let Some((tok, _)) = self.fetch().ok().flatten() {
                let desc = self.trace_tok_simple(tok);
                s.push_str("<to be read again> \n");
                s.push_str(&format!("{}{}\n", " ".repeat(19), desc));
                self.unread(tok);
            }
        }
        // l.N 行上下文（两行：位置前内容 + n 空格 + 位置后字符）
        if let Some((n, line, byte_pos)) = self.error_context_pos() {
            // **口径陷阱（2026-09-19 现场）**：`error_context_pos` 给的是源内的
            // **字节**偏移（源按字节读取），而 tex.web 的 trick_buf /
            // half_error_line 全按**字符**计数。两套口径混用——旧代码把字节
            // 偏移与描述字符数相加、再拿结果当字节下标切 `&str`——会在含多字节
            // 字符（中文）的行上把裁剪点算进字符内部：`&before[start..]` 撞 UTF-8
            // 边界直接 panic，而 wasm 的 `panic = "abort"` 把它退化成一句
            // 无位置信息的 `RuntimeError: unreachable`（Tauri 工作台整窗报
            // 「加载失败」，排查成本极高）。
            // 故此处一律换算成**字符下标**再裁剪：ASCII 源两套口径恒等，
            // TRIP/ETRIP 逐字对齐不受影响（口径只在多字节输入下才有分歧）。
            let chars: Vec<char> = line.chars().collect();
            let pos = line[..snap_char_boundary(&line, byte_pos)]
                .chars()
                .count()
                .min(chars.len());
            // tex.web @<Show the context...@>：第一行超 half_error_line 时左侧
            // 裁剪（`...` + 尾部）。参考 TRIP log 反推 half_error_line=32
            // （l.253：l=6,k=58 → 显示尾部 23 字符；l.2：l=4,k=29 → 尾部 25）。
            let l_desc = format!("l.{n} ");
            let l_len = l_desc.chars().count();
            let k = l_len + pos;
            const HALF_ERROR_LINE: usize = 32;
            let before: String = chars[..pos].iter().collect();
            let (prefix, before_shown) = if l_len + k > HALF_ERROR_LINE {
                // tex.web：trick_buf[(l+k-h+3)..k-1]，trick_buf 前 l_len 字符是描述
                // → before 起点 = k - h + 3（l.2 参考：k=33 → before[4..]="case..."）
                let start = k.saturating_sub(HALF_ERROR_LINE - 3).min(pos);
                ("...", chars[start..pos].iter().collect::<String>())
            } else {
                ("", before)
            };
            let l1 = format!("{l_desc}{prefix}{before_shown}");
            s.push_str(&l1);
            s.push('\n');
            // 第二行上限：TeX trick_count = first_count+1+error_line-half_error_line
            //（TRIP 参考 L94：pos=11 → 42 字符）
            let max_after = pos + 1 + 79 - 48;
            let after: String = chars[pos..].iter().take(max_after).collect();
            let suffix = if chars.len().saturating_sub(pos) > max_after {
                "..."
            } else {
                ""
            };
            s.push_str(&format!(
                "{}{}{}\n",
                " ".repeat(l1.chars().count()),
                after,
                suffix
            ));
        }
        // help1..6：紧随 l.N 上下文行（TeX error() 在上下文行后打印帮助文本，
        // 无额外空行；块间空行由 write16 尾部追加的 \n 产生）
        if let Some(h) = help {
            s.push_str(h);
        }
        let _ = self.sink.write16(s);
    }

    /// TeX scan_int 缺数恢复：`! Missing number, treated as zero.` + `<to be read
    /// again>` + l.N 上下文 + 3 行 help（tex.web error() + @<Report an improper...@>；
    /// 参考 trip.log 各 Missing number 段逐字对齐——l.253/l.419 等格式差即源于
    /// 此前缺失这些行）。
    pub(super) fn report_missing_number(&mut self) {
        // 第二十刀定位开关：NTEX_NUM_TRACE——现场打出输入栈摘要 + 宏调用链 +
        // 最近 token，用于锁定「数字扫描读到哪个 token、从哪条宏链进来」。
        if std::env::var_os("NTEX_NUM_TRACE").is_some() {
            fn show(intern: &crate::intern::InternTable, t: &Token) -> String {
                if let Some(csid) = t.csid() {
                    format!("\\{}", intern.name(csid))
                } else if let Some(n) = t.param_number() {
                    format!("#{n}")
                } else if let Some(ch) = t.charcode().and_then(char::from_u32) {
                    format!("{ch:?}/cc{:?}", t.catcode())
                } else {
                    format!("{t:?}")
                }
            }
            let mut frames = Vec::new();
            for f in self.stack.iter().rev().take(4) {
                match f {
                    InputFrame::TokenList { items, pos } => {
                        let next = items[*pos..]
                            .iter()
                            .take(10)
                            .map(|(t, _)| show(&self.intern, t))
                            .collect::<Vec<_>>()
                            .join(" ");
                        let done = items[..*pos]
                            .iter()
                            .rev()
                            .take(4)
                            .map(|(t, _)| show(&self.intern, t))
                            .collect::<Vec<_>>()
                            .join(" ");
                        frames.push(format!(
                            "{}tok@{} next=[{next}] done=[{done}]",
                            items.len(),
                            pos
                        ));
                    }
                    InputFrame::MacroArg { items, pos } => {
                        let next = items[*pos..]
                            .iter()
                            .take(10)
                            .map(|(t, _)| show(&self.intern, t))
                            .collect::<Vec<_>>()
                            .join(" ");
                        frames.push(format!("MacroArg {}tok@{} next=[{next}]", items.len(), pos));
                    }
                    InputFrame::Source { bytes, pos, .. } => {
                        let off = (*pos).min(bytes.len());
                        let ctx = String::from_utf8_lossy(&bytes[off..(off + 80).min(bytes.len())])
                            .replace('\n', "⏎");
                        frames.push(format!("source@{} [{ctx}]", pos));
                    }
                    other => frames.push(format!("{other:?}")),
                }
            }
            eprintln!(
                "[missing-number] line={} last_tok={:?}\n  frames(外→内): {}",
                self.error_line_no(),
                self.last_tok,
                frames.join("\n  ")
            );
        }
        self.write_error("Missing number, treated as zero.");
        let _ = self.sink.write16(
            "A number should have been here; I inserted `0'.\n\
             (If you can't figure out why I needed to see a number,\n\
             look up `weird error' in the index to The TeXbook.)\n"
                .to_string(),
        );
    }
}
