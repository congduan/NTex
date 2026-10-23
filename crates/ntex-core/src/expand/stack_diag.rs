use super::*;

impl Expander {
    /// 输入栈转储（诊断开关）。
    ///
    /// 触发变量：
    /// - `NTEX_STACK_DUMP`：帧类型直方图 + 栈顶帧签名重复度 + 逐帧内容；
    /// - `NTEX_STACK_DUMP_FRAMES`：仅逐帧内容（单独设置亦生效）；
    /// - `NTEX_STACK_DUMP_TOP=N`：栈顶逐帧转储条数（默认 60）；
    /// - `NTEX_STACK_DUMP_BOTTOM=N`：栈底逐帧转储条数（默认 5；调大可看
    ///   「循环墙从哪一帧开始」——墙的起点在栈底侧）；
    /// - `NTEX_CALL_TRACE=N`：宏调用环形轨迹（N = 显示段数，默认 120；
    ///   未设 = 零开销，见 [`MACRO_TRACE_CAP`]）。
    ///
    /// **两处守卫共用本入口的原因**（2026-09-13 修正）：宏帧守卫
    /// [`Self::call_macro_inner`] 用 `>= MAX_INPUT_STACK`，比 fetch 的统一兜底
    /// （`>`）**先命中**——此前转储只挂在 fetch 上，于是「宏/字节码递归不终止」
    /// 这类最需要现场的爆栈（expl3 载入 l.27200：`\q_stop 递归展开疑似无终止
    /// 条件`）反而永远拿不到转储。两侧改为调用同一函数。
    ///
    /// 转储三件套的分工：类型直方图回答「哪类帧多」；**帧签名重复度**回答
    /// 「哪一族帧在自复制」（循环主体的直接证据）；逐帧详解给出 token 级现场。
    pub(super) fn dump_input_stack(&mut self, reason: &str) {
        let kinds_on = std::env::var_os("NTEX_STACK_DUMP").is_some();
        let frames_on = std::env::var_os("NTEX_STACK_DUMP_FRAMES").is_some();
        if !kinds_on && !frames_on {
            return;
        }
        let n = self.stack.len();
        let top: usize = std::env::var("NTEX_STACK_DUMP_TOP")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(60);
        // 栈底条数：定位「循环从哪一帧开始」要的是**墙的起点**（栈底侧），
        // 只看栈顶一堆同样的帧反而不知道是谁把第一个 `\q_stop` 打进流里的。
        let bottom: usize = std::env::var("NTEX_STACK_DUMP_BOTTOM")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(5);
        if kinds_on {
            let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
            for f in &self.stack {
                *kinds.entry(Self::frame_kind(f)).or_default() += 1;
            }
            let _ = self.sink.write16(format!(
                "[stack-dump] reason={reason} depth={n} kinds={kinds:?}\n"
            ));
            // 栈顶 top 帧的签名重复度：循环主体必在此处高频自复制。
            let sig_n = top.min(n);
            let mut sigs: BTreeMap<String, usize> = BTreeMap::new();
            for f in &self.stack[n - sig_n..] {
                *sigs.entry(self.frame_marker(f)).or_default() += 1;
            }
            let mut ranked: Vec<(String, usize)> = sigs.into_iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let _ = self.sink.write16(format!(
                "[stack-dump] 栈顶 {sig_n} 帧签名重复度（top 8）：\n"
            ));
            for (sig, cnt) in ranked.iter().take(8) {
                let _ = self
                    .sink
                    .write16(format!("[stack-dump]   {cnt:>5} × {sig}\n"));
            }
            // 宏调用轨迹：栈上没有的「已弹出入口帧」只能从这里看出来。
            // 连续同名合并（`\q_stop×4993`），否则表格被同一个名字淹掉。
            if let Some(tr) = &self.macro_trace {
                let mut runs: Vec<(u32, usize)> = Vec::new();
                for &id in tr.iter() {
                    match runs.last_mut() {
                        Some((prev, cnt)) if *prev == id => *cnt += 1,
                        _ => runs.push((id, 1)),
                    }
                }
                let intern = &self.intern;
                let rendered: Vec<String> = runs
                    .iter()
                    .map(|(id, cnt)| Self::fmt_call_run(intern, *id, *cnt))
                    .collect();
                let show: usize = std::env::var("NTEX_CALL_TRACE")
                    .ok()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(120)
                    .max(1);
                let start = rendered.len().saturating_sub(show);
                let _ = self.sink.write16(format!(
                    "[stack-dump] 宏调用轨迹（共 {} 次，旧→新，显示末 {show} 段）：\n[stack-dump]   {}\n",
                    tr.len(),
                    rendered[start..].join(" ")
                ));
            }
        }
        if frames_on {
            let (plan, omitted) = Self::frame_dump_plan(n, top, bottom);
            for slot in plan {
                match slot {
                    Some(idx) => {
                        let _ = self.sink.write16(format!(
                            "[frame {:>5}] {}\n",
                            idx,
                            self.render_frame_head(&self.stack[idx])
                        ));
                    }
                    None => {
                        let _ = self
                            .sink
                            .write16(format!("[frame ...] （省略 {omitted} 帧）\n"));
                    }
                }
            }
        }
    }

    /// 逐帧转储计划：返回 `(帧序号槽位, 中间省略的帧数)`；`None` = 省略标记行。
    ///
    /// 槽位顺序即输出顺序：**栈顶 `top` 帧（降序）+ 省略 + 栈底 `bottom` 帧（升序）**。
    ///
    /// ⚠ 这里是 2026-09-13 修掉的一处**仪器 bug**：原实现取 `stack[..head]`
    /// （栈**底** head 帧）却按 `n-1-i` 标注成「栈顶帧」——**底/顶正好反了**。
    /// 宏递归每层都压在顶上，循环主体在栈顶；看栈底等于只看最外层作业帧，
    /// 必然误判（09-11 那次「4997 个 TokenList 帧」的结论其实是栈底视图）。
    /// 抽成纯函数是为了让这个方向性约定能被单测钉住。
    pub(super) fn frame_dump_plan(
        n: usize,
        top: usize,
        bottom: usize,
    ) -> (Vec<Option<usize>>, usize) {
        let head = n.min(top);
        let tail = bottom.min(n - head);
        let omitted = n - head - tail;
        let mut plan = Vec::with_capacity(head + tail + 1);
        for idx in (n - head..n).rev() {
            plan.push(Some(idx));
        }
        if omitted > 0 {
            plan.push(None);
        }
        for idx in 0..tail {
            plan.push(Some(idx));
        }
        (plan, omitted)
    }

    /// 渲染一段连续同名调用：`\name` / `\name×n`（转储轨迹用）。
    fn fmt_call_run(intern: &InternTable, csid: u32, n: usize) -> String {
        let name = intern.name(csid);
        if n <= 1 {
            format!("\\{name}")
        } else {
            format!("\\{name}×{n}")
        }
    }

    /// 栈帧类型名（转储直方图用）。
    pub(super) fn frame_kind(f: &InputFrame) -> &'static str {
        match f {
            InputFrame::Source { .. } => "Source",
            InputFrame::Macro { .. } => "Macro",
            InputFrame::Bytecode { .. } => "Bytecode",
            InputFrame::TokenList { .. } => "TokenList",
            InputFrame::MacroArg { .. } => "MacroArg",
            InputFrame::One { .. } => "One",
            InputFrame::OutputRoutine { .. } => "OutputRoutine",
            InputFrame::AlignU { .. } => "AlignU",
            InputFrame::AlignV { .. } => "AlignV",
        }
    }

    /// 帧签名（转储分组用）：**类型 + 头部 token，不含进度**——同一宏体的不同
    /// 进度应归为同一族，重复度才看得出「谁在自复制」。
    pub(super) fn frame_marker(&self, f: &InputFrame) -> String {
        let (kind, desc) = match f {
            InputFrame::Source { .. } => ("Source", String::new()),
            InputFrame::Macro { body, .. } => ("Macro", self.render_token_head(body, 8)),
            InputFrame::Bytecode { code, .. } => ("Bytecode", self.render_bytecode_head(code, 8)),
            InputFrame::TokenList { items, .. } => ("TokenList", self.render_pairs_head(items, 8)),
            InputFrame::MacroArg { items, .. } => ("MacroArg", self.render_pairs_head(items, 8)),
            InputFrame::One { tok, .. } => ("One", self.render_token(*tok)),
            InputFrame::OutputRoutine { items, .. } => {
                ("OutputRoutine", self.render_pairs_head(items, 8))
            }
            InputFrame::AlignU { items, .. } => ("AlignU", self.render_token_head(items, 8)),
            InputFrame::AlignV { items, .. } => ("AlignV", self.render_token_head(items, 8)),
        };
        if desc.is_empty() {
            kind.to_owned()
        } else {
            format!("{kind}[{desc}]")
        }
    }

    /// 逐帧详解（转储用）：类型 + 进度 + 头部内容。
    ///
    /// 帧内容**从头**显示（不看 pos）——已消费帧的内容同样是「谁在循环展开」
    /// 的证据。此前实现漏了 [`InputFrame::Macro`]/`MacroArg`/`AlignU`/`AlignV`/
    /// `OutputRoutine` 五个帧型（落进 `Other`），而宏递归爆栈现场恰恰全是
    /// Macro 帧，等于转储在最需要的地方失明。
    pub(super) fn render_frame_head(&self, f: &InputFrame) -> String {
        match f {
            InputFrame::Source { pos, bytes, .. } => format!("Source[pos={pos}/{}]", bytes.len()),
            InputFrame::Macro { body, pos, args } => format!(
                "{} args={}",
                self.render_seq_frame("Macro", body, *pos),
                args.len()
            ),
            InputFrame::Bytecode { code, pc, args } => format!(
                "Bytecode[pc={pc}/{} args={}] head={} at={}",
                code.len(),
                args.len(),
                self.render_bytecode_span(code, 0, 8),
                self.render_bytecode_span(code, *pc, 8)
            ),
            InputFrame::TokenList { items, pos } => {
                self.render_pair_frame("TokenList", items, *pos)
            }
            InputFrame::MacroArg { items, pos } => self.render_pair_frame("MacroArg", items, *pos),
            InputFrame::One { tok, .. } => format!("One[{}]", self.render_token(*tok)),
            InputFrame::OutputRoutine { items, pos } => {
                self.render_pair_frame("OutputRoutine", items, *pos)
            }
            InputFrame::AlignU { items, pos } => self.render_seq_frame("AlignU", items, *pos),
            InputFrame::AlignV { items, pos } => self.render_seq_frame("AlignV", items, *pos),
        }
    }

    /// token 序列帧的通用渲染：`Kind[rem=r/total] head=… at=…`。
    ///
    /// `head` 用来看「这一帧是什么」（帧从头显示）；**`at` 才是当前现场**——
    /// 长宏体/长实参（本例 439 token 的 MacroArg）上只看 head 会完全失明：
    /// 正被展开的 token 在 pos 处，不在帧头。
    fn render_seq_frame(&self, kind: &str, items: &[Token], pos: usize) -> String {
        format!(
            "{kind}[rem={}/{}] head={} at={}",
            items.len().saturating_sub(pos),
            items.len(),
            self.render_token_span(items, 0, 6),
            self.render_token_span(items, pos, 14)
        )
    }

    /// `(token, noexpand)` 序列帧的通用渲染（TokenList/OutputRoutine）。
    fn render_pair_frame(&self, kind: &str, items: &[(Token, bool)], pos: usize) -> String {
        format!(
            "{kind}[rem={}/{}] head={} at={}",
            items.len().saturating_sub(pos),
            items.len(),
            self.render_pairs_span(items, 0, 6),
            self.render_pairs_span(items, pos, 14)
        )
    }

    /// 渲染 token 序列头部（转储用）；超出 `max` 的以 `…(+k)` 计数收尾。
    fn render_token_head(&self, items: &[Token], max: usize) -> String {
        self.render_token_span(items, 0, max)
    }

    /// 渲染 token 序列从 `from` 起的 `max` 个 token（转储用）。
    pub(super) fn render_token_span(&self, items: &[Token], from: usize, max: usize) -> String {
        let from = from.min(items.len());
        let rest = items.len() - from;
        let take = rest.min(max);
        let mut parts: Vec<String> = Vec::with_capacity(take + 1);
        for t in &items[from..from + take] {
            parts.push(self.render_token(*t));
        }
        if rest > take {
            parts.push(format!("…(+{})", rest - take));
        }
        parts.join(" ")
    }

    /// 渲染 `(token, noexpand)` 序列头部（TokenList/OutputRoutine 帧转储用）。
    /// `noexpand` 标记以 `~` 前缀表示（`\noexpand` 冻结的 token）。
    fn render_pairs_head(&self, items: &[(Token, bool)], max: usize) -> String {
        self.render_pairs_span(items, 0, max)
    }

    /// 渲染 `(token, noexpand)` 序列从 `from` 起的 `max` 个元素。
    fn render_pairs_span(&self, items: &[(Token, bool)], from: usize, max: usize) -> String {
        let from = from.min(items.len());
        let rest = items.len() - from;
        let take = rest.min(max);
        let mut parts: Vec<String> = Vec::with_capacity(take + 1);
        for (t, ne) in &items[from..from + take] {
            let s = self.render_token(*t);
            parts.push(if *ne { format!("~{s}") } else { s });
        }
        if rest > take {
            parts.push(format!("…(+{})", rest - take));
        }
        parts.join(" ")
    }

    /// 渲染字节码帧头部（转储用）：token 原值按 `\name`，`#n` 实参，`end` 结束。
    fn render_bytecode_head(&self, code: &Bytecode, max: usize) -> String {
        self.render_bytecode_span(code, 0, max)
    }

    /// 渲染字节码从第 `from` 个字起的 `max` 条指令（转储用；`from` 通常取 pc，
    /// 因为待执行指令从 pc 开始，头部已执行部分不是现场）。
    fn render_bytecode_span(&self, code: &Bytecode, from: usize, max: usize) -> String {
        let words = code.words();
        let from = from.min(words.len());
        let rest = words.len() - from;
        let take = rest.min(max);
        let mut parts: Vec<String> = Vec::with_capacity(take + 1);
        for &w in &words[from..from + take] {
            parts.push(match Instruction::decode(w) {
                Instruction::Emit { token } => self.render_token(token),
                Instruction::EmitArg { n } => format!("#{n}"),
                Instruction::End => "end".to_owned(),
            });
        }
        if rest > take {
            parts.push(format!("…(+{})", rest - take));
        }
        parts.join(" ")
    }

    /// 字节码护栏转储：最多三个活动字节码帧，逐帧列出 pc 邻域反汇编与近期 cs 环。
    pub(super) fn dump_bytecode_guard(&self, dispatches: u64, limit: u64) {
        eprintln!("[bc-guard] dispatch 超限：连续 {dispatches} 次未返回 token（limit={limit}）");
        for (ordinal, (code, pc)) in self
            .stack
            .iter()
            .rev()
            .filter_map(|frame| match frame {
                InputFrame::Bytecode { code, pc, .. } => Some((code, *pc)),
                _ => None,
            })
            .take(3)
            .enumerate()
        {
            let start = pc.saturating_sub(8);
            let end = (pc + 9).min(code.len());
            let disasm = (start..end)
                .map(|at| {
                    let marker = if at == pc { '>' } else { ' ' };
                    let word = code.words()[at];
                    let text = match Instruction::decode(word) {
                        Instruction::Emit { token } => self.render_token(token),
                        Instruction::EmitArg { n } => format!("#{n}"),
                        Instruction::End => "end".to_owned(),
                    };
                    format!("{marker}{at:>4}: {text} [{word:#018x}]")
                })
                .collect::<Vec<_>>()
                .join("; ");
            eprintln!(
                "[bc-guard] frame#{ordinal} pc={pc}/{}: {disasm}",
                code.len()
            );
        }
        let recent = self
            .bc_guard_trace
            .iter()
            .map(|&csid| format!("\\{}", self.intern.name(csid)))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("[bc-guard] recent-cs: {recent}");
        if let Some(trace) = &self.handler_trace {
            let recent = trace
                .iter()
                .map(|&(csid, floor)| format!("\\{}(floor={floor})", self.intern.name(csid)))
                .collect::<Vec<_>>()
                .join(" ");
            eprintln!("[bc-guard] handler-trace: {recent}");
        }
        eprintln!("[bc-guard] stack: {}", self.debug_stack_summary());
    }

    /// 单个 token 的可读渲染（转储用）：`\name` / 可打印字符 / `#n`；
    /// 空白显示为 `␣`，其余控制字符显示为 `^XX`。
    pub(super) fn render_token(&self, t: Token) -> String {
        match t.kind() {
            TokenKind::ControlSeq => match t.csid() {
                Some(id) => format!("\\{}", self.intern.name(id)),
                None => "\\?".to_owned(),
            },
            TokenKind::MacroParam => match t.param_number() {
                Some(n) => format!("#{n}"),
                None => "#?".to_owned(),
            },
            TokenKind::EndGroup => "}".to_owned(),
            TokenKind::EndTemplate => String::new(),
            TokenKind::Char => match t.charcode() {
                Some(c) => {
                    let ch = char::from_u32(c).unwrap_or('\u{FFFD}');
                    if ch == ' ' {
                        "␣".to_owned()
                    } else if ch.is_ascii_graphic() {
                        ch.to_string()
                    } else {
                        format!("^{:02X}", c & 0x7F)
                    }
                }
                None => "?".to_owned(),
            },
        }
    }
}
