// M4-5 对齐机制（tex.web §749-823）：\halign/\valign 的 preamble 解析、
// u/v 模板逐单元注入、align_state 平衡计数与 \cr/\crcr/\span/\omit/\noalign 语义。
//
// 状态机映射（tex.web → ntex）：
// - preamble 列表（[glue, alignrecord, glue, ...]）→ `cols: Vec<AlignCol>` +
//   `tabskips: Vec<Glue>`（len = 列数 + 1，tabskips[i] = 第 i 列**前**的胶水，
//   末元素为尾部胶水）；
// - u_part/v_part token 列表 → `InputFrame::AlignU`/`AlignV` 输入帧，
//   耗尽即 tex.web end_token_list 的 u_template（align_state←0）与 endv（fin_col）；
// - align_state（组平衡计数）：preamble = -1000000 + `{}` 深度（只有深度 0 的
//   `&`/`\cr` 结束列）；body 单元 raw 扫描 = 0（`{` +1、`}` -1，减到 -1 =
//   对齐组的 `}`）；模板注入期间 = 1000000；
// - cur_loop（周期 preamble，"Missing #" 的空 u 列起循环复制）→ `loop_col`
//   记起始列 idx + 游标 cursor（扩展列 append 到 cols 尾，游标线性前进，
//   与 tex.web 15640-15647 的列表步进一致）。
//
// sink 事件：`align_preamble_end(tabskips)`（列边界快照）、`align_cell_begin`
// （跨列单元开始 = tex.web init_span 的 push_nest）、`align_cell_end(Tab|Cr,
// span_len)`（fin_col 单元封装时机）、`align_row_end`（fin_row）。
// `\span` 结束的单元不产生 cell_end（与后续单元合并为跨列单元）。
//
// 组模型：对齐组本身 = 静默 eqtb 组（group_level+1，sink 侧组由 align_begin
// 事件开）；每列一个静默单元组（tex.web fin_col 的 unsave + new_save_level，
// 单元内赋值 `\cr` 时回滚）；`\noalign{...}` = 真实组（begin_group，
// layout 组种类 7）。

/// 单元结束符（tex.web extra_info 存的 cur_chr：& 字符码 / span=256 /
/// cr=257 / crcr=258）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellEndKind {
    Tab,
    Span,
    Cr,
    CrCr,
}

/// 命令层分类（tex.web `cur_cmd`/`cur_chr` 二元判据的引擎等价物）。
///
/// TeX 扫描器对齐相关的判据**全部落在命令层**（`cur_cmd=mac_param`、
/// `cur_cmd=tab_mark`、`cur_cmd=left_brace`…）。而 `\let\cs=<字符>` 生成的
/// cs token 在读取时 `cur_cmd:=eq_type(cur_cs)` 就等于那个字符的 catcode，
/// 且 `get_x_token`/`get_token` 都**不会**把它展开成字符——所以
/// "字符命令别名"必须与字符 token 同等识别，否则：
///
/// - `\let\bgroup={`（LaTeX `\@preamble` 的 `\ialign\bgroup`、`\@tabular`
///   的 `\hbox\bgroup`、`\endtabular` 的 `\crcr\egroup…`）→ 误报
///   "Missing { inserted"（2026-09-19 修）；
/// - LaTeX `\@mkpream` 生成 preamble 时 `#` 写作 `\@sharp`
///   （latex.ltx L16813 `\let\@sharp##`）→ 整列落进 u 段，误报
///   "Missing # inserted in alignment preamble"（2026-09-19 修）；
/// - `\let\next=&` 型别名同理。
///
/// **与字符形态的关键差异**（tex.web L7490-7493）：只有字符形态在读取时增减
/// `align_state`（`case cur_cmd of left_brace: incr(align_state)`），cs 形态从
/// token list 读出（走 `t>=cs_token_flag` 分支）**不动** align_state——对齐的
/// 收尾因此靠 align_peek 的命令层判据（L15517 `else if cur_cmd=right_brace then
/// fin_align`），而不是 align_state 变负。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CmdClass {
    /// mac_param（`#`；tex.web L15464）。
    Param,
    /// tab_mark（`&`；tex.web L15466）。
    Tab,
    /// left_brace（`{` / `\bgroup`）。
    BeginBrace,
    /// right_brace（`}` / `\egroup`）。
    EndBrace,
    /// tab_mark + span_code（`\span`）。
    Span,
    /// car_ret + cr_code（`\cr`）。
    Cr,
    /// car_ret + cr_cr_code（`\crcr`）。
    CrCr,
    /// assign_glue + tab_skip_code（`\tabskip`，赋值由主循环执行）。
    TabSkip,
}

/// 一列模板（tex.web alignrecord 的 u_part/v_part）。
#[derive(Debug, Clone, Default)]
struct AlignCol {
    u: Vec<Token>,
    v: Vec<Token>,
}

/// 对齐帧（一个 \halign/\valign；嵌套对齐 = 栈式多帧）。
#[derive(Debug, Clone)]
pub(crate) struct AlignFrame {
    /// true=\halign（行堆叠）；false=\valign（列并排）。
    /// 方向已随 align_begin 事件传给排版器；VM 侧保留用于调试/后续 valign 扩展。
    #[allow(dead_code)]
    is_halign: bool,
    /// 对齐外的 align_state（tex.web push_alignment 保存）。
    saved_align_state: i64,
    /// 组平衡计数（tex.web align_state 语义，见模块注释）。
    align_state: i64,
    phase: AlignPhase,
}

#[derive(Debug, Clone)]
enum AlignPhase {
    /// preamble 扫描中（tex.web scanner_status=aligning）。
    Preamble {
        cols: Vec<AlignCol>,
        /// 列边界胶水快照（len = cols.len() + 1）。
        tabskips: Vec<Glue>,
        /// 当前列 u 段收集（`#` 之前）。
        cur_u: Vec<Token>,
        /// 当前列 v 段收集（`#` 之后）。
        cur_v: Vec<Token>,
        /// 当前列已遇 `#`（u→v 切换）。
        seen_hash: bool,
        /// 周期 preamble 起始列（首个 u 空的 `&` 列；tex.web cur_loop）。
        loop_col: Option<usize>,
        /// preamble 内 `{}` 平衡（深度 0 的 &/\cr 才结束列；tex.web
        /// align_state=-1000000+depth 的词法行为）。
        brace_depth: i64,
        /// `\span` 后一个 token 需展开一次（tex.web get_preamble_token 的
        /// span 循环）。
        span_pending: bool,
    },
    /// body 扫描中（模板逐单元注入）。
    Body {
        cols: Vec<AlignCol>,
        tabskips: Vec<Glue>,
        /// 周期扩展游标（起始列 idx, 下一个待复制列 idx）。
        loop_col: Option<(usize, usize)>,
        /// 当前列 idx。
        cur_col: usize,
        /// 当前跨列单元起始列（tex.web cur_span）。
        span_start: usize,
        /// 单元结束符（v 模板注入时记录；V 帧耗尽时消费）。
        pending_end: Option<CellEndKind>,
        /// 本单元 \omit（v 模板省略；tex.web extra_info 的 omit 标记）。
        omit: bool,
        /// 行边界（\cr 后；\noalign 合法位）。
        row_open: bool,
        /// \noalign 组的配对组级（begin_group 后的 group_level）。
        noalign_level: Option<u32>,
    },
}

impl Expander {
    // ---------- M4-5 对齐：帧生命周期 ----------

    /// 命令层分类（tex.web 读 token 时 `cur_cmd:=eq_type`/catcode 的等价判定）。
    /// 显式字符 token 与 `\let\cs=<字符>` 型 cs token 归入同一类，原语类
    /// （`\span`/`\cr`/`\crcr`/`\tabskip`）按 `Primitive` 归位；其余返回 `None`
    /// （= 普通 token，按内容存入模板 / 交给主循环）。详见 [`CmdClass`]。
    fn cmd_class(&self, tok: Token) -> Option<CmdClass> {
        let char_class = |c: &Catcode| match c {
            Catcode::Parameter => Some(CmdClass::Param),
            Catcode::AlignmentTab => Some(CmdClass::Tab),
            Catcode::BeginGroup => Some(CmdClass::BeginBrace),
            Catcode::EndGroup => Some(CmdClass::EndBrace),
            _ => None,
        };
        match tok.kind() {
            TokenKind::Char => char_class(&tok.catcode()?),
            TokenKind::ControlSeq => {
                let csid = tok.csid()?;
                match self.eqtb.slot(csid) {
                    EqSlot::Char { catcode, .. } => char_class(catcode),
                    EqSlot::Primitive(p) => match p {
                        Primitive::Span => Some(CmdClass::Span),
                        Primitive::Cr => Some(CmdClass::Cr),
                        Primitive::CrCr => Some(CmdClass::CrCr),
                        Primitive::TabSkip => Some(CmdClass::TabSkip),
                        _ => None,
                    },
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// `\halign`/`\valign` 启动（`{` 已由 dispatcher 消费；tex.web init_align
    /// + push_alignment + Scan the preamble）。sink 侧 align_begin 已发。
    fn align_start(&mut self, is_halign: bool) -> Result<()> {
        let saved_align_state = self
            .align_frames
            .last()
            .map(|f| f.align_state)
            .unwrap_or(0);
        let tabskip = self.current_tabskip();
        self.align_frames.push(AlignFrame {
            is_halign,
            saved_align_state,
            align_state: -1000000,
            phase: AlignPhase::Preamble {
                cols: Vec::new(),
                tabskips: vec![tabskip],
                cur_u: Vec::new(),
                cur_v: Vec::new(),
                seen_hash: false,
                loop_col: None,
                brace_depth: 0,
                span_pending: false,
            },
        });
        // 对齐 eqtb 组（tex.web scan_spec 的 new_save_level）。VM 侧静默开组，
        // sink 侧此时消费 align_begin 留下的 pending_kind=Align **立即开组**
        // ——对齐的 `{` 已被 align_scan_left_brace 消费，不会再有主循环的
        // group_begin 事件来认领；若不发，无内层 `{}` 的对齐（`\halign{#\cr}`）
        // 在 align_finish→end_group 时 group_end 无配对，有内层 `{}` 时首个
        // 内层组还会错认领 Align 种类（TRIP 回归 001e0bc 的根因）。
        let line = self.current_line_no() as u32;
        self.sink.group_begin(line)?;
        self.begin_silent_group();
        Ok(())
    }

    /// 对齐收尾（对齐组的 `}`；tex.web fin_align + pop_alignment）。
    /// sink 侧组关闭由 end_group 的 group_end 事件完成（layout 在该事件做
    /// fin_align 数学：列宽分配 + 单元封装 + 行堆叠）。
    fn align_finish(&mut self) -> Result<()> {
        if let Some(frame) = self.align_frames.pop() {
            self.align_state = frame.saved_align_state;
        }
        self.end_group()
    }

    /// 当前 `\tabskip` 胶水快照（preamble 列边界用）。
    fn current_tabskip(&self) -> Glue {
        match self.params.get(ParamKind::TabSkip) {
            ParamValue::Glue(g) => g,
            _ => Glue::ZERO,
        }
    }

    /// `\everycr` 注入（tex.web L15339/L15732：`begin_token_list(every_cr,
    /// every_cr_text)`）。两处注入点：① preamble 扫完（init_align 尾）；
    /// ② 每行 fin_row。注入后 token 在输入流顶，**先于 align_peek 的前瞻**
    /// 被读（amsmath `\everycr{\noalign{…}}` 每行重置标签、kernel `\ialign`/
    /// `\eqnarray` 的 `\everycr{}` 清空语义都依赖它）。此前"只存不注入"
    /// （S1/G1，halign-survey §3.2）。
    fn align_inject_everycr(&mut self) -> Result<()> {
        let toks = self.everycr_toks.clone();
        if toks.is_empty() {
            return Ok(());
        }
        let items: Vec<(Token, bool)> = toks.into_iter().map(|t| (t, false)).collect();
        self.push_frame(InputFrame::TokenList {
            items: Arc::from(items),
            pos: 0,
        });
        Ok(())
    }

    /// 静默组（eqtb 作用域，不发 sink 组事件）：对齐组与对齐单元组用
    /// （sink 侧组由 align_begin/align_cell_begin 事件分别管理）。
    fn begin_silent_group(&mut self) {
        self.group_level += 1;
        self.scope_stack.push(false);
        self.group_cond_depth.push(self.cond_stack.len());
    }

    /// 静默组收尾（恢复 save_stack + 条件深度 + \aftergroup；无 sink 事件）。
    fn end_silent_group(&mut self) {
        let _ = self.group_cond_depth.pop();
        while let Some((level, _)) = self.save_stack.last() {
            if *level != self.group_level {
                break;
            }
            let (_, v) = self.save_stack.pop().expect("last() 已检查非空");
            self.restore(v);
        }
        let scope = self.scope_stack.len() as u32;
        let tokens: Vec<Token> = self
            .aftergroup
            .iter()
            .filter(|(l, _)| *l == scope)
            .map(|(_, t)| *t)
            .collect();
        self.aftergroup.retain(|(l, _)| *l != scope);
        self.scope_stack.pop();
        if !tokens.is_empty() {
            let items: Vec<(Token, bool)> = tokens.into_iter().map(|t| (t, false)).collect();
            self.push_frame(InputFrame::TokenList {
                items: Arc::from(items),
                pos: 0,
            });
        }
        self.group_level -= 1;
    }

    // ---------- M4-5 对齐：主循环拦截 ----------

    /// 对齐状态机拦截（process_one 在 fetch 后调用；返回 true = token 已消费）。
    /// preamble 阶段分类收集；body raw 阶段拦 `&`/`\span`/`\cr`/`\crcr`
    ///（Insert v_j）与 `}` 平衡（对齐组闭括号判定）。
    fn align_on_token(&mut self, tok: Token) -> Result<bool> {
        // v 模板尾哨兵抵达主循环 ≙ tex.web expand L7785 的
        // `end_template → frozen_endv` 换形：此刻才收列（fin_col）。
        if tok.is_end_template() {
            self.align_fin_col()?;
            return Ok(true);
        }
        let Some(frame) = self.align_frames.last_mut() else {
            return Ok(false);
        };
        if diag_enabled("NTEX_ALIGN_TRACE") {
            let ph = match &frame.phase {
                AlignPhase::Preamble {
                    brace_depth,
                    seen_hash,
                    ..
                } => format!("pre d={} h={}", brace_depth, seen_hash),
                AlignPhase::Body { cur_col, .. } => format!("body col={}", cur_col),
            };
            let name = tok
                .csid()
                .map(|csid| self.intern.name(csid).to_string())
                .unwrap_or_default();
            eprintln!("[trace-align] tok={:?} \\{} {}", tok, name, ph);
        }
        match &mut frame.phase {
            AlignPhase::Preamble { .. } => self.align_preamble_step(tok),
            AlignPhase::Body { .. } => self.align_body_step(tok),
        }
    }

    /// preamble 单步（tex.web get_preamble_token + u_j/v_j 扫描）。
    /// "The preamble is copied directly"：宏/可展开原语**不展开**（`\span` 后
    /// 一次例外），`\tabskip` 赋值执行，`#` 是 u→v 分界。
    fn align_preamble_step(&mut self, tok: Token) -> Result<bool> {
        // 借用拆分：先取决策数据，再分步变更
        let (depth, seen_hash, span_pending) = match &self.align_frames.last().unwrap().phase {
            AlignPhase::Preamble {
                brace_depth,
                seen_hash,
                span_pending,
                ..
            } => (*brace_depth, *seen_hash, *span_pending),
            _ => unreachable!("调用方已确保 Preamble"),
        };
        // \span 后的可展开 token：展开一次（tex.web 15442-15447）
        if span_pending {
            if let Some(csid) = tok.csid() {
                if let Some(op) = self.cond_op(tok) {
                    self.set_preamble_span_pending(false);
                    self.step_conditional(op, tok)?;
                    return Ok(true);
                }
                match self.eqtb.slot(csid).clone() {
                    EqSlot::Macro(m) => {
                        self.set_preamble_span_pending(false);
                        return self.call_macro(csid, m.value.clone()).map(|_| true);
                    }
                    EqSlot::Alias(target) => {
                        self.set_preamble_span_pending(false);
                        self.unread(Token::control_sequence(target));
                        return Ok(true);
                    }
                    EqSlot::Primitive(p) if p.is_expandable() => {
                        self.set_preamble_span_pending(false);
                        return self.exec_primitive(p).map(|_| true);
                    }
                    _ => {}
                }
            }
            self.set_preamble_span_pending(false);
        }
        // 命令层分类（tex.web cur_cmd 判据）：显式字符 token 取 catcode，
        // `\let\cs=<字符>` 型 cs token 取槽内 catcode——LaTeX `\@mkpream`
        // 生成的 preamble 里 `#` 写作 `\@sharp`（latex.ltx L16813
        // `\let\@sharp##`），只按字符 token 判会整列落进 u 段并报
        // "Missing # inserted in alignment preamble"。
        match self.cmd_class(tok) {
            // `#`：u→v 分界（tex.web mac_param L15464；不存入模板）
            Some(CmdClass::Param) => {
                if seen_hash {
                    // tex.web 15484 段："Only one # is allowed per tab"
                    let _ = self.sink.write16(
                        "! Only one # is allowed per tab.\n\
                         There should be exactly one # between &'s when an\n\
                         \\halign or \\valign is being set up. In this case you\n\
                         had more than one, so I kept the first.\n"
                            .to_string(),
                    );
                } else {
                    self.preamble_set_hash();
                }
                Ok(true)
            }
            // `&`：深度 0 结束当前列（u 段 → 进 v 段；v 段 → 列完成），
            // 否则存
            Some(CmdClass::Tab) => {
                if depth == 0 {
                    if seen_hash {
                        self.preamble_col_done(false)?;
                    } else {
                        self.preamble_u_ended(tok);
                    }
                    Ok(true)
                } else {
                    self.preamble_store(tok);
                    Ok(true)
                }
            }
            // `{`/`}`：只有**字符 token** 调深度。tex.web 的 align_state 加减
            // 只发生在两处字符通路：get_next 的 `mid_line+left_brace` 臂
            // （L7335-7341，文件/行扫描）与 get_token 的 token 表分支
            // （L7492-7493，`t<cs_token_flag` 才进 case）；cs token 分支只做
            // outer_call 校验、**不碰 align_state**。故 `\let\bgroup={` 型
            // cs（eq_type=left_brace）在 preamble 里是纯数据——GT pdftex
            // 实证 `\halign{\bgroup&#\egroup\cr}` 首错是
            // "Missing # inserted in alignment preamble"（`&` 仍按深度 0
            // 终结 u 段）。分类仍走 cmd_class（cur_cmd 层面，tab/param/cr
            // 的 cs 形态命中不变），只是深度记账收窄到字符形态。
            Some(CmdClass::BeginBrace) => {
                if tok.kind() == TokenKind::Char {
                    self.preamble_adjust_depth(1);
                }
                self.preamble_store(tok);
                Ok(true)
            }
            Some(CmdClass::EndBrace) => {
                if tok.kind() == TokenKind::Char {
                    self.preamble_adjust_depth(-1);
                }
                self.preamble_store(tok);
                Ok(true)
            }
            // \span：吸收，下一 token 展开一次（不存 \span 本身）
            Some(CmdClass::Span) => {
                self.set_preamble_span_pending(true);
                Ok(true)
            }
            // \cr/\crcr（tex.web 15465 的 `cur_cmd<=car_ret` 段）
            Some(CmdClass::Cr | CmdClass::CrCr) => {
                if depth == 0 {
                    if seen_hash {
                        self.preamble_col_done(true)?;
                    } else {
                        self.preamble_u_ended(tok);
                    }
                } else {
                    self.preamble_store(tok);
                }
                Ok(true)
            }
            // \tabskip 赋值执行（tex.web assign_glue+tab_skip_code；
            // \global 前缀 token 存模板、\tabskip 本身执行）
            Some(CmdClass::TabSkip) => Ok(false),
            None => {
                // u 段开头空格丢弃（tex.web "Spaces are eliminated from the
                // beginning of a template"）；其余（含普通 cs）原样存
                if tok.kind() == TokenKind::Char
                    && tok.catcode() == Some(Catcode::Space)
                    && !seen_hash
                    && self.preamble_u_is_empty()
                {
                    return Ok(true);
                }
                self.preamble_store(tok);
                Ok(true)
            }
        }
    }

    /// body 单步：raw（align_state==0）时拦 `&`/`\span`/`\cr`/`\crcr`（Insert
    /// v_j）；`{`/`}` 平衡计数（减到 -1 = 对齐组闭括号 → fin_align）。
    fn align_body_step(&mut self, tok: Token) -> Result<bool> {
        let raw = match self.align_frames.last() {
            Some(AlignFrame {
                align_state, ..
            }) => *align_state == 0,
            None => false,
        };
        if raw {
            let is_tab = tok.kind() == TokenKind::Char && tok.catcode() == Some(Catcode::AlignmentTab);
            let is_align_delim = is_tab
                || (tok.csid().is_some_and(|csid| {
                    matches!(
                        self.eqtb.slot(csid),
                        EqSlot::Primitive(Primitive::Span | Primitive::Cr | Primitive::CrCr)
                    )
                }));
            if is_align_delim {
                let kind = if is_tab {
                    CellEndKind::Tab
                } else {
                    let csid = tok.csid().expect("is_align_delim 已确认");
                    match self.eqtb.slot(csid) {
                        EqSlot::Primitive(Primitive::Span) => CellEndKind::Span,
                        EqSlot::Primitive(Primitive::Cr) => CellEndKind::Cr,
                        _ => CellEndKind::CrCr,
                    }
                };
                self.align_insert_v(kind);
                return Ok(true);
            }
        }
        match tok.kind() {
            TokenKind::Char => match tok.catcode() {
                Some(Catcode::BeginGroup) => {
                    if let Some(frame) = self.align_frames.last_mut() {
                        frame.align_state += 1;
                    }
                    Ok(false)
                }
                Some(Catcode::EndGroup) => {
                    let Some(frame) = self.align_frames.last_mut() else {
                        return Ok(false);
                    };
                    frame.align_state -= 1;
                    if frame.align_state < 0 {
                        // 对齐组的 `}`（raw 平衡 -1 或行边界 peek 到）
                        self.align_finish()
                            .map(|_| true)
                    } else {
                        // \noalign 组配对 `}`：关组后回到行边界 peek
                        //（token 已消费，不再落 Char 分支的 end_group）
                        let closes_noalign = matches!(
                            &frame.phase,
                            AlignPhase::Body {
                                noalign_level: Some(l),
                                ..
                            } if *l == self.group_level
                        );
                        if closes_noalign {
                            if let Some(AlignFrame {
                                phase: AlignPhase::Body { noalign_level, .. },
                                ..
                            }) = self.align_frames.last_mut()
                            {
                                *noalign_level = None;
                            }
                            self.end_group()?;
                            self.align_peek_next()?;
                            return Ok(true);
                        }
                        Ok(false)
                    }
                }
                _ => Ok(false),
            },
            _ => Ok(false),
        }
    }

    // ---------- M4-5 对齐：preamble 内部 ----------

    fn set_preamble_span_pending(&mut self, v: bool) {
        if let Some(AlignFrame {
            phase: AlignPhase::Preamble { span_pending, .. },
            ..
        }) = self.align_frames.last_mut()
        {
            *span_pending = v;
        }
    }

    fn preamble_set_hash(&mut self) {
        if let Some(AlignFrame {
            phase: AlignPhase::Preamble { seen_hash, .. },
            ..
        }) = self.align_frames.last_mut()
        {
            *seen_hash = true;
        }
    }

    fn preamble_u_is_empty(&self) -> bool {
        match self.align_frames.last() {
            Some(AlignFrame {
                phase: AlignPhase::Preamble { cur_u, .. },
                ..
            }) => cur_u.is_empty(),
            _ => true,
        }
    }

    fn preamble_adjust_depth(&mut self, delta: i64) {
        if let Some(frame) = self.align_frames.last_mut() {
            if let AlignPhase::Preamble { brace_depth, .. } = &mut frame.phase {
                *brace_depth += delta;
            }
        }
    }

    /// preamble token 存入当前段（u 或 v）。
    fn preamble_store(&mut self, tok: Token) {
        if let Some(AlignFrame {
            phase: AlignPhase::Preamble {
                seen_hash,
                cur_u,
                cur_v,
                ..
            },
            ..
        }) = self.align_frames.last_mut()
        {
            if *seen_hash {
                cur_v.push(tok);
            } else {
                cur_u.push(tok);
            }
        }
    }

    /// u 段遇深度 0 的 `&`/`\cr`/`\crcr`（tex.web 15465-15474）：
    /// - u 空 + 无 loop + `&` → 周期 preamble 标记（cur_loop）；
    /// - 否则未遇 `#` → "Missing # inserted in alignment preamble"
    ///   （按有 # 恢复，v 为空）；
    /// - 分界 token 压回（back_error 语义），由 v 段读回后真正结束列。
    fn preamble_u_ended(&mut self, delim: Token) {
        let (u_empty, seen_hash, loop_is_none, is_tab) = match &self.align_frames.last() {
            Some(AlignFrame {
                phase: AlignPhase::Preamble {
                    cur_u,
                    seen_hash,
                    loop_col,
                    ..
                },
                ..
            }) => (cur_u.is_empty(), *seen_hash, loop_col.is_none(), delim.catcode() == Some(Catcode::AlignmentTab)),
            _ => return,
        };
        if u_empty && loop_is_none && is_tab {
            // 周期 preamble 起始列 = 即将 push 的当前列 idx。
            // tex.web 15469-15471：**吸收**该 `&`（cur_loop:=cur_align 后 u 扫描
            // 继续，token 丢弃、不建列、不切 v 段）——amsmath `\align@preamble`
            // 以 `&` 开头（`\halign{\span\align@preamble\crcr}`），若在此建列会
            // 凭空多出第 0 列（u=v=空），全体单元格右移一格：首格脱离
            // `$\displaystyle{…}` 模板（\mathrm 报 "allowed only in math
            // mode"）、行尾 `\add@amps` 读到的 `\column@` 偏大（少吐 `&` 填充，
            // `\math@cr@@@align` 的 `\omit` 落在格内 = Misplaced \omit）。
            let cols_len = match &self.align_frames.last() {
                Some(AlignFrame {
                    phase: AlignPhase::Preamble { cols, .. },
                    ..
                }) => cols.len(),
                _ => 0,
            };
            if let Some(AlignFrame {
                phase: AlignPhase::Preamble { loop_col, .. },
                ..
            }) = self.align_frames.last_mut()
            {
                *loop_col = Some(cols_len);
            }
            return;
        } else if !seen_hash {
            let _ = self.sink.write16(
                "! Missing # inserted in alignment preamble.\n\
                 There should be exactly one # between &'s, when an\n\
                 \\halign or \\valign is being set up. In this case you had\n\
                 none, so I've put one in; maybe that will work.\n"
                    .to_string(),
            );
        }
        // 进 v 段；压回 delim（v 段读回 → 结束列）
        if let Some(AlignFrame {
            phase: AlignPhase::Preamble { seen_hash, .. },
            ..
        }) = self.align_frames.last_mut()
        {
            *seen_hash = true;
        }
        self.unread(delim);
    }

    /// v 段遇深度 0 的 `&`/`\cr`/`\crcr`：当前列完成（tex.web done2 +
    /// append alignrecord）。`&` → 下一列；`\cr`/`\crcr` → preamble 结束
    ///（尾部胶水 + align_preamble_end + 行边界 peek）。
    fn preamble_col_done(&mut self, delim_is_cr: bool) -> Result<()> {
        let tabskip = self.current_tabskip();
        // 步骤一：完成当前列（u/v 移入 cols + 胶水快照）
        let body_ready = {
            let Some(AlignFrame { phase, .. }) = self.align_frames.last_mut() else {
                return Ok(());
            };
            let AlignPhase::Preamble {
                cols,
                cur_u,
                cur_v,
                tabskips,
                seen_hash,
                ..
            } = phase
            else {
                return Ok(());
            };
            let col = AlignCol {
                u: std::mem::take(cur_u),
                v: std::mem::take(cur_v),
            };
            cols.push(col);
            tabskips.push(tabskip);
            // 新列的 u 段扫描从头开始（tex.web 每列的 u-scan 是独立 loop，
            // `#` 判据按列重置）。不复位 → 第二列的 `#` 被视为"列内第二个 #"
            // 误报 "Only one # is allowed per tab"，且该列 u 段空、整列落进
            // v 段（LaTeX `\@sharp` 的两列 tabular 必踩）。
            if !delim_is_cr {
                *seen_hash = false;
            }
            delim_is_cr
        };
        if !body_ready {
            return Ok(());
        }
        // 步骤二（\cr）：preamble 完成 → Body 态
        let (cols, tabskips, loop_pair) = {
            let Some(AlignFrame { phase, .. }) = self.align_frames.last_mut() else {
                return Ok(());
            };
            let AlignPhase::Preamble {
                cols,
                tabskips,
                loop_col,
                seen_hash,
                ..
            } = phase
            else {
                return Ok(());
            };
            let _ = seen_hash;
            (
                std::mem::take(cols),
                std::mem::take(tabskips),
                loop_col.map(|s| (s, s)),
            )
        };
        let snapshot = tabskips.clone();
        if let Some(AlignFrame { phase, .. }) = self.align_frames.last_mut() {
            *phase = AlignPhase::Body {
                cols,
                tabskips,
                loop_col: loop_pair,
                cur_col: 0,
                span_start: 0,
                pending_end: None,
                omit: false,
                row_open: true,
                noalign_level: None,
            };
        }
        self.sink.align_preamble_end(snapshot)?;
        // tex.web L15339：preamble 扫完注入 \everycr（先于 align_peek）
        self.align_inject_everycr()?;
        self.align_peek_next()?;
        Ok(())
    }

    // ---------- M4-5 对齐：body 内部 ----------

    /// Insert v_j（tex.web get_next 的 v_template 插入）：记录结束符 + 压 V
    /// 模板帧（\omit 单元 V 为空）+ align_state←1000000。
    fn align_insert_v(&mut self, kind: CellEndKind) {
        let (v, omit) = match self.align_frames.last_mut() {
            Some(AlignFrame {
                phase:
                    AlignPhase::Body {
                        cols,
                        cur_col,
                        pending_end,
                        omit,
                        ..
                    },
                ..
            }) => {
                *pending_end = Some(kind);
                let v = if *omit {
                    Vec::new()
                } else {
                    cols.get(*cur_col).map(|c| c.v.clone()).unwrap_or_default()
                };
                (v, *omit)
            }
            _ => return,
        };
        let _ = omit;
        // tex.web L15497：扫描到的 v_j 模板以显式 `\endtemplate` 收尾；哨兵
        // 让 plus/minus 等关键字前瞻读到可退回的 token，收列只在哨兵真正
        // 抵达主循环（align_on_token）时发生。
        let mut v = v;
        v.push(Token::end_template());
        self.push_frame(InputFrame::AlignV {
            items: TokenArray::from(v),
            pos: 0,
        });
        if let Some(frame) = self.align_frames.last_mut() {
            frame.align_state = 1000000;
        }
    }

    /// U 模板帧耗尽（tex.web end_token_list 的 u_template 分支）：raw 扫描开始。
    fn align_u_exhausted(&mut self) {
        if let Some(frame) = self.align_frames.last_mut() {
            frame.align_state = 0;
        }
    }

    /// V 模板帧耗尽 = \endtemplate/endv（tex.web do_endv → fin_col）：
    /// 按结束符封装单元/推进列/结束行。
    fn align_fin_col(&mut self) -> Result<()> {
        let kind = match self.align_frames.last_mut() {
            Some(AlignFrame {
                phase: AlignPhase::Body { pending_end, .. },
                ..
            }) => pending_end.take(),
            _ => None,
        };
        let Some(kind) = kind else {
            return Ok(());
        };
        match kind {
            // 跨列单元中间段（tex.web fin_col 的 `extra_info=span_code` 分支：
            // **跳过** `unsave; new_save_level(align_group)` 与 `init_span(p)`）
            // ——不关 sink 单元、不开新组，只推进列指针 + 新列模板。
            CellEndKind::Span => {
                if self.align_col_will_overflow() {
                    self.align_extra_tab_as_cr()?;
                } else {
                    self.align_advance_col(false);
                    let peeked = self.align_fetch_significant()?;
                    if let Some(t) = peeked {
                        self.align_init_col(t, false)?;
                    }
                }
            }
            CellEndKind::Tab => {
                if self.align_col_will_overflow() {
                    self.align_extra_tab_as_cr()?;
                } else {
                    self.align_close_cell(AlignCellEnd::Tab)?;
                    self.align_advance_col(true);
                    let peeked = self.align_fetch_significant()?;
                    if let Some(t) = peeked {
                        self.align_init_col(t, true)?;
                    }
                }
            }
            CellEndKind::Cr | CellEndKind::CrCr => {
                self.align_close_cell(AlignCellEnd::Cr)?;
                self.align_end_row_and_peek()?;
            }
        }
        Ok(())
    }

    /// 越界判定：preamble 列表耗尽且无周期列（tex.web fin_col 的
    /// `<If the preamble list has been traversed…>`：`p=null` 且
    /// `extra_info(cur_align)<cr_code` 且 `cur_loop=null`）。
    fn align_col_will_overflow(&self) -> bool {
        match self.align_frames.last() {
            Some(AlignFrame {
                phase:
                    AlignPhase::Body {
                        cols,
                        loop_col,
                        cur_col,
                        ..
                    },
                ..
            }) => *cur_col + 1 >= cols.len() && loop_col.is_none(),
            _ => false,
        }
    }

    /// 越界 `&`/`\span` = `\cr`（tex.web 同一节：报
    /// "Extra alignment tab has been changed to \cr" 后
    /// `extra_info(cur_align):=cr_code` → fin_col 返回 true → 行结束，
    /// 余下 token 由 align_peek 起新行的首列续排）。此前实现只报错却把
    /// 内容钳进末列（单行多单元），与 GT 行数/行内容都不符。
    fn align_extra_tab_as_cr(&mut self) -> Result<()> {
        let _ = self.sink.write16(
            "! Extra alignment tab has been changed to \\cr.\n\
             You have given more \\span or & marks than there were\n\
             in the preamble to the \\halign or \\valign now in progress.\n\
             So I'll assume that you meant to type \\cr instead.\n"
                .to_string(),
        );
        self.align_close_cell(AlignCellEnd::Cr)?;
        self.align_end_row_and_peek()
    }

    /// 行收尾三连（tex.web fin_row：行盒入竖列 → 注入 \everycr → align_peek）。
    fn align_end_row_and_peek(&mut self) -> Result<()> {
        self.sink.align_row_end()?;
        self.align_set_row_open(true);
        // tex.web L15732：fin_row 尾注入 \everycr（先于 align_peek）
        self.align_inject_everycr()?;
        self.align_peek_next()
    }

    /// 单元封装（sink 事件 + 单元 eqtb 组收尾）。span_len = 跨列数。
    fn align_close_cell(&mut self, end: AlignCellEnd) -> Result<()> {
        let span_len = match self.align_frames.last() {
            Some(AlignFrame {
                phase: AlignPhase::Body {
                    cur_col, span_start, ..
                },
                ..
            }) => (*cur_col - *span_start + 1) as u16,
            _ => 1,
        };
        self.end_silent_group();
        self.sink.align_cell_end(end, span_len)
    }

    /// 列指针推进（cur_col+1）；越界时周期扩展（tex.web "Lengthen the
    /// preamble periodically"）。无周期列的越界**不会走到这里**——调用方
    /// （align_fin_col）已先经 align_col_will_overflow 分流到
    /// align_extra_tab_as_cr（越界 `&`/`\span` 按 `\cr` 收行）；此处兜底
    /// 钳末列只为不 panic。
    ///
    /// `new_unit`：是否开始新**单元**（非 `\span`）。跨列单元（`\span`）的
    /// 起始列由 tex.web `cur_span` 记着，且 fin_col 的 span 分支**不**调
    /// `init_span(p)`——故 span 续列不得重置 `span_start`，否则单元收尾时
    /// `span_len = cur_col - span_start + 1` 恒为 1，跨列宽度/列数全丢。
    fn align_advance_col(&mut self, new_unit: bool) {
        let Some(AlignFrame {
            phase: AlignPhase::Body { cols, tabskips, loop_col, cur_col, span_start, .. },
            ..
        }) = self.align_frames.last_mut()
        else {
            return;
        };
        let next = *cur_col + 1;
        if next >= cols.len() {
            if let Some((_start, cursor)) = loop_col.as_mut() {
                // 周期复制：新列 = cols[cursor] 的模板；其后胶水 = 该列后胶水
                let src = (*cursor).min(cols.len().saturating_sub(1));
                let col = cols[src].clone();
                let glue = tabskips.get(src + 1).copied().unwrap_or(Glue::ZERO);
                cols.push(col);
                tabskips.push(glue);
                *cursor += 1;
            }
        }
        *cur_col = (*cur_col + 1).min(cols.len().saturating_sub(1));
        if new_unit {
            *span_start = *cur_col;
        }
    }

    fn align_set_row_open(&mut self, v: bool) {
        if let Some(AlignFrame {
            phase: AlignPhase::Body { row_open, .. },
            ..
        }) = self.align_frames.last_mut()
        {
            *row_open = v;
        }
    }

    /// 新列启动（tex.web init_col）：sink 单元开始（init_span 的 push_nest
    /// 时机）+ 单元 eqtb 组 + \omit 判定 / U 模板帧注入。
    ///
    /// `new_unit`：`\span` 续列时为 false——tex.web fin_col 的 span 分支跳过
    /// `unsave; new_save_level(align_group)` 与 `init_span(p)`，故既不 push_nest
    /// 也不开新组；不照做会让每个 `\span` 泄漏一个 eqtb 组，
    /// `\omit\span\omit`（LaTeX `\multispan`/`\multicolumn`）最终以
    /// "(end occurred inside a group)" / "ended by \document" 收场。
    fn align_init_col(&mut self, peeked: Token, new_unit: bool) -> Result<()> {
        if new_unit {
            self.sink.align_cell_begin()?;
            self.begin_silent_group();
        }
        let is_omit = peeked
            .csid()
            .is_some_and(|csid| matches!(self.eqtb.slot(csid), EqSlot::Primitive(Primitive::Omit)));
        if is_omit {
            if let Some(AlignFrame {
                phase: AlignPhase::Body { omit, .. },
                align_state,
                ..
            }) = self.align_frames.last_mut()
            {
                *omit = true;
                *align_state = 0;
            }
        } else {
            if let Some(AlignFrame {
                phase: AlignPhase::Body { omit, .. },
                ..
            }) = self.align_frames.last_mut()
            {
                *omit = false;
            }
            self.unread(peeked);
            let u = match self.align_frames.last() {
                Some(AlignFrame {
                    phase: AlignPhase::Body { cols, cur_col, .. },
                    ..
                }) => cols.get(*cur_col).map(|c| c.u.clone()).unwrap_or_default(),
                _ => Vec::new(),
            };
            self.push_frame(InputFrame::AlignU {
                items: TokenArray::from(u),
                pos: 0,
            });
            if let Some(frame) = self.align_frames.last_mut() {
                frame.align_state = 1000000;
            }
        }
        Ok(())
    }

    /// 取下一"非空白非宏调用"token（tex.web "Get the next non-blank non-call
    /// token"）：跳空格、展开宏/别名。None = 输入耗尽（对齐未闭合）。
    fn align_fetch_significant(&mut self) -> Result<Option<Token>> {
        loop {
            let Some((tok, noexpand)) = self.fetch()? else {
                return Ok(None);
            };
            if noexpand {
                return Ok(Some(tok));
            }
            if tok.kind() == TokenKind::Char && tok.catcode() == Some(Catcode::Space) {
                continue;
            }
            if let Some(csid) = tok.csid() {
                match self.eqtb.slot(csid).clone() {
                    EqSlot::Macro(m) => {
                        self.call_macro(csid, m.value.clone())?;
                        continue;
                    }
                    EqSlot::Alias(target) => {
                        self.unread(Token::control_sequence(target));
                        continue;
                    }
                    // tex.web get_x_token：可展开原语就地展开（`\romannumeral`/
                    // `\csname`/`\the`…）。LaTeX `\end{tabular}` 的展开链
                    // `\end → \romannumeral\ifx…\z@\end<space> → \csname
                    // endtabular\endcsname → \crcr` 全程可展开，peek 必须能穿过，
                    // 否则 `\crcr\egroup`（\endtabular 的对齐收尾）读不到，
                    // `\romannumeral` 本身被当成新行首列 → 凭空多一行。
                    EqSlot::Primitive(p) if p.is_expandable() => {
                        self.exec_primitive(p)?;
                        continue;
                    }
                    _ => {}
                }
            }
            return Ok(Some(tok));
        }
    }

    /// 行边界 peek（tex.web align_peek）：`\noalign{`（真实组）→ 开组；
    /// `}` → 对齐收尾；冗余 `\crcr` → 忽略；其他 → 新行首列（unread +
    /// init_row + init_col）。
    fn align_peek_next(&mut self) -> Result<()> {
        loop {
            // tex.web L15509：align_peek 入口复位 align_state:=1000000
            // （restart 标签首句）。preamble 完成路径下 align_state 残留
            // -1000000 哨兵，若不复位，注入的 `\noalign{…}` 的 `}` 会被
            // EndGroup 分支减到 <0 误判为对齐闭括号（2026-09-08 everycr
            // 注入刀引入对拍时暴露；fin_row 路径此前恒 1000000 未踩到）。
            if let Some(frame) = self.align_frames.last_mut() {
                frame.align_state = 1000000;
            }
            let Some(tok) = self.align_fetch_significant()? else {
                // 输入耗尽（\end inside \halign）——交由主循环收尾报错
                return Ok(());
            };
            // \noalign（行边界合法位已在 dispatch 层保证；tex.web no_align
            // + scan_left_brace）：真实组（layout 组种类 7）
            if tok
                .csid()
                .is_some_and(|csid| matches!(self.eqtb.slot(csid), EqSlot::Primitive(Primitive::NoAlign)))
            {
                self.align_scan_left_brace()?;
                self.sink.noalign_begin()?;
                self.begin_group()?;
                if let Some(AlignFrame {
                    phase: AlignPhase::Body { noalign_level, .. },
                    ..
                }) = self.align_frames.last_mut()
                {
                    *noalign_level = Some(self.group_level);
                }
                return Ok(());
            }
            // `}` / `\egroup`：对齐组闭括号（tex.web L15517 命令层判据
            // `else if cur_cmd=right_brace then fin_align`——LaTeX
            // `\endtabular` 的 `\crcr\egroup…` 走这条；cs 形态不动
            // align_state，故不能靠平衡计数收尾）；align_state 保持 1000000
            // 不减——tex.web fin_align 前 align_state 不动。
            if self.cmd_class(tok) == Some(CmdClass::EndBrace) {
                return self.align_finish();
            }
            // 冗余 \crcr：忽略（tex.web align_peek 的 cr_cr_code restart）
            if tok.csid().is_some_and(|csid| {
                matches!(self.eqtb.slot(csid), EqSlot::Primitive(Primitive::CrCr))
            }) {
                continue;
            }
            // 新行：列指针复位 + 行开 + 首列启动
            if let Some(AlignFrame {
                phase: AlignPhase::Body {
                    cur_col, span_start, row_open, ..
                },
                ..
            }) = self.align_frames.last_mut()
            {
                *cur_col = 0;
                *span_start = 0;
                *row_open = false;
            }
            return self.align_init_col(tok, true);
        }
    }

    /// 消费对齐组起始 `{`（tex.web scan_left_brace L8194-8206：
    /// `<Get the next non-blank non-relax non-call token>` = get_x_token 展开
    /// 宏/可展开原语，跳空白与 `\relax`，然后**命令层**判 `cur_cmd=left_brace`）。
    /// 故 `\halign\bgroup`（LaTeX `\@preamble` 的 `\ialign\bgroup`）合法：
    /// `\bgroup` 的 eq_type 就是 left_brace，get_x_token 不会把它展开成 `{`。
    /// 非 `{` 报 "Missing { inserted" 并放回 token——组内容照常继续，相当于
    /// 插入了隐含 `{`。
    fn align_scan_left_brace(&mut self) -> Result<()> {
        loop {
            let Some((tok, noexpand)) = self.fetch()? else {
                return Ok(());
            };
            // get_x_token：可展开项就地展开一次（`\@halignto` 为空宏、
            // `\let` 别名跟随目标；`\noexpand` 前缀抑制展开）。
            if !noexpand {
                if let Some(csid) = tok.csid() {
                    match self.eqtb.slot(csid).clone() {
                        EqSlot::Macro(m) => {
                            self.call_macro(csid, m.value.clone())?;
                            continue;
                        }
                        EqSlot::Alias(target) => {
                            self.unread(Token::control_sequence(target));
                            continue;
                        }
                        EqSlot::Primitive(p) if p.is_expandable() => {
                            self.exec_primitive(p)?;
                            continue;
                        }
                        // non-relax（tex.web L8208-8210 的 until 条件）
                        EqSlot::Primitive(Primitive::Relax) => continue,
                        _ => {}
                    }
                }
            }
            if tok.kind() == TokenKind::Char && tok.catcode() == Some(Catcode::Space) {
                continue;
            }
            if self.cmd_class(tok) == Some(CmdClass::BeginBrace) {
                return Ok(());
            }
            self.unread(tok);
            // tex.web L8199-8202 的 help4（此前误用了 scan_toks 的
            // "Where was the left brace?" 文案，见 §9350）。
            self.write_error_help(
                "Missing { inserted",
                "A left brace was mandatory here, so I've put one in.\n\
                 You might want to delete and/or insert some corrections\n\
                 so that I will find a matching right brace soon.\n\
                 (If you're confused by all this, try typing `I}' now.)\n",
            );
            return Ok(());
        }
    }
}
