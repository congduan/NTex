/// e-TeX 表达式除法：**四舍五入**到最近整数（ties away from zero），非 TeX 传统截断
/// （etrip "Expr quotient rounding"：`"40000000/"7FFFFFFF`=1）。调用方保证 `d != 0`。
fn expr_quotient_i128(n: i128, d: i128) -> i128 {
    let (an, ad) = (n.abs(), d.abs());
    let q = (an + ad / 2) / ad;
    if (n < 0) != (d < 0) {
        -q
    } else {
        q
    }
}

impl Expander {
    /// `\expandafter a b`：输出 a，再输出 b 的一次展开结果。
    ///
    /// 展开"一次"：宏 → 实参替换后的宏体（不再递归展开）；`\expandafter` → 递归；
    /// `\noexpand` → 标记下一 token；其余原样。
    fn exec_expandafter(&mut self) -> Result<()> {
        let t1 = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\expandafter 后无 token"))?;
        let t2 = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\expandafter 后无第二个 token"))?;
        // 条件终结符（\else/\fi/\or）：TeX expand() 把 fi_or_else 展开为空格
        // （tex.web expand 的 fi_or_else 分支）——消耗该 token 并推进条件机，
        // 不重新输出（否则会被宏实参扫描吞掉，如 `\expandafter\2\fi`）。
        // `\unless`（e-TeX）：先于条件终结符判定——它须**就地拉取**下一个
        // `\if*` 求值（expand_unless_in_place 文档）。
        if self.slot_is_unless(t2.0) && self.expand_unless_in_place()? {
            let seq = vec![t1];
            self.push_frame(InputFrame::TokenList {
                items: Arc::from(seq),
                pos: 0,
            });
            return Ok(());
        }
        if let Some(op) = self.cond_op(t2.0) {
            // tex.web：`\expandafter` 对第二个 token 走 get_x_token → expand，
            // 分支跳过（false 的 \if*、\else/\or 的待弃分支）是**就地**完成的，
            // 随后才把 t1 放回输入。主循环的惰性跳过会在 t1 落回时把它吞掉
            // （\e@alloc 的 `\global\ifnum…\expandafter\chardef\else…\fi`），
            // 故展开上下文里必须急切消费（见 drain_open_skip）。
            let before = self.cond_stack.len();
            self.step_conditional(op, t2.0)?;
            if !matches!(op, CondOp::Fi) {
                let depth = if matches!(op, CondOp::Else | CondOp::Or) {
                    before.saturating_sub(1)
                } else {
                    before
                };
                self.drain_open_skip(depth)?;
            }
            // 前面的 token 照常输出（t1）
            let seq = vec![t1];
            self.push_frame(InputFrame::TokenList {
                items: Arc::from(seq),
                pos: 0,
            });
            return Ok(());
        }
        let mut expansion = Vec::new();
        self.expand_once(t2, &mut expansion)?;
        let mut seq = Vec::with_capacity(1 + expansion.len());
        seq.push(t1);
        seq.extend(expansion);
        self.push_frame(InputFrame::TokenList {
            items: Arc::from(seq),
            pos: 0,
        });
        Ok(())
    }

    /// 展开单个 token 一次，结果追加到 `out`。
    fn expand_once(&mut self, item: (Token, bool), out: &mut Vec<(Token, bool)>) -> Result<()> {
        if item.1 {
            // 已被 \noexpand 标记：不展开
            out.push(item);
            return Ok(());
        }
        let tok = item.0;
        if let Some(csid) = tok.csid() {
            match self.eqtb.slot(csid).clone() {
                EqSlot::Alias(ref target) => {
                    // tex.web：`\let` 在 eqtb 层**复制含义**（eq_type/eq_chr/equivalency
                    // 整体搬），别名即原义——展开 `\A` 就是展开 `\B` 的含义。此处必须
                    // 沿别名链解引用到最终含义再分派（宏/可展开原语 → 就地展开），
                    // 不得把目标 token 原样回填：expl3 全篇 `\cs_new_eq:NN` 两级别名链
                    // （`\__int_eval:w → \tex_numexpr:D → \numexpr`、
                    //  `\int_value:w → \tex_number:D → \number`、`\__int_sep: →
                    // \tex_let:D → \let`）依赖此语义——`\exp_after:wN X \int_value:w`
                    // （l3int `\int_div_truncate:nn`，l.6652）的 `\number` 若不就地
                    // 求值，`\expandafter` 的展开产物只剩 `\__int_sep:`，外层表达式
                    // 在终结符处提前收口、`\__int_div_truncate:NwNw` 的体整段残留
                    // （`\c_sys_engine_version_str` = `140(100-1)/2)/100\__int_eval_end:`
                    //   失真，l.8073 起 cascading 到 l.9386/l.9468 区）。
                    let target = *target;
                    let mut id = target;
                    let mut depth = 0;
                    while let EqSlot::Alias(next) = self.eqtb.slot(id) {
                        id = *next;
                        depth += 1;
                        if depth > 100 {
                            return Err(Error::invalid_input("\\let 别名环"));
                        }
                    }
                    match self.eqtb.slot(id).clone() {
                        EqSlot::Macro(_) | EqSlot::Primitive(_) => {
                            self.expand_once((Token::control_sequence(id), item.1), out)?
                        }
                        _ => out.push((Token::control_sequence(id), item.1)),
                    }
                }
                EqSlot::Macro(m) => {
                    let def = m.value.clone();
                    // 0 参数宏也必须走 collect_args：参数文本可能是**纯定界串**
                    // （`\def\X\fi:\use:n{...}`），调用点须匹配并吞掉（tex.web
                    // macro_call `if info(r)<>end_match_token`；与 call_macro 同一
                    // 契约）。expl3 条件生成器 fast form 的 `\__prg_T_true:w`/
                    // `\__prg_F_true:w`/`\__prg_TF_true:w`/`\__prg_p_true:w` 即此
                    // 形态：`\edef`/f 型展开（\exp:w）里跳过匹配会把体首 `\fi:`
                    // 泄给条件机——帧被体内 `\fi:` 提前弹掉，随后定界串里的 `\fi:`
                    // 再来一次即 "! Extra \fi."（expl3-code l.7934 起 \str_const:Ne
                    // 区级联，\str_case 全线 extra-} 失衡即源于此）。
                    let args = self.collect_args(csid, &def)?;
                    let materialized = materialize(&def.body, &args);
                    out.extend(materialized.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::Expandafter) => {
                    let a = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("\\expandafter 链中断"))?;
                    let b = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("\\expandafter 链中断"))?;
                    out.push(a);
                    // \unless：与 exec_expandafter 同理——就地拉取下一个 \if*
                    // 求值（\str_tail:n 的 `\expandafter\X\reverse_if:N\if…`）。
                    if self.slot_is_unless(b.0) && self.expand_unless_in_place()? {
                        return Ok(());
                    }
                    // \else/\fi/\or：TeX expand() 的 fi_or_else 分支（展开为空格并推进条件机）；
                    // 开着的跳过区同样就地消费（见 exec_expandafter 的说明）
                    if let Some(op) = self.cond_op(b.0) {
                        let before = self.cond_stack.len();
                        self.step_conditional(op, b.0)?;
                        if !matches!(op, CondOp::Fi) {
                            let depth = if matches!(op, CondOp::Else | CondOp::Or) {
                                before.saturating_sub(1)
                            } else {
                                before
                            };
                            self.drain_open_skip(depth)?;
                        }
                    } else {
                        self.expand_once(b, out)?;
                    }
                }
                EqSlot::Primitive(Primitive::Noexpand) => {
                    let t = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("\\noexpand 后无 token"))?;
                    out.push((t.0, true));
                }
                EqSlot::Primitive(Primitive::The) => {
                    // `\the` 冻结位（tex.web L9395-9411：scan_toks 的 xpand
                    // 展开器把 `the_toks` 产物直接接进收集表，"without
                    // expanding it further"）。展开收集语境（suppress>0：
                    // \edef/\xdef 体、`\expanded`）产物不再展开——`\g@addto@macro`
                    // 的 `\xdef#1{\the\toks@}` 惯用法（latex.ltx l.12705 NFSS
                    // 钩子链）依赖此语义；主循环 ins_list 路径照常展开。
                    // 详见 exec_the（save.rs）注解。
                    let freeze = self.suppress_expansion > 0;
                    let tokens = self.the_tokens()?;
                    out.extend(tokens.into_iter().map(|t| (t, freeze)));
                }
                // M4-5 e-TeX/可展开原语（与 `is_expandable()` 对齐）：\number/\unexpanded/
                // \detokenize/\eTeXversion/\eTeXrevision。此前落入 `_` 分支被当作不可展开
                // 原样保留，导致 `\expandafter\1\eTeXrevision` 把未展开的 \eTeXrevision
                // 当作实参（ETRIP 版本检查 `2..6` 错误即由此而来）。
                EqSlot::Primitive(Primitive::Number) => {
                    let v = self.scan_number()?;
                    out.extend(emit_count(v).into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::ETeXVersion) => {
                    out.extend(emit_count(2).into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::ETeXRevision) => {
                    out.extend(
                        ".6"
                            .bytes()
                            .map(|b| (Token::char(Catcode::Other, u32::from(b)), false)),
                    );
                }
                EqSlot::Primitive(Primitive::Unexpanded) => {
                    // \unexpanded{...}：组内容原样保留（noexpand 标记）
                    let toks = self.scan_group_contents_expanding()?;
                    out.extend(toks.into_iter().map(|t| (t, true)));
                }
                EqSlot::Primitive(Primitive::Detokenize) => {
                    // \detokenize{...}：组内容转回字符 token（cat 12 其他字符）
                    let toks = self.scan_group_contents_expanding()?;
                    let mut detok = Vec::new();
                    let esc = self.params.misc[34];
                    for t in toks {
                        detokenize_token(t, &self.intern, esc, &mut detok);
                    }
                    out.extend(detok.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::Expanded) => {
                    // pdfTeX \expanded{...}：组内容按 \edef 语义全展开（结果已
                    // 无可展开项，标记 false 直接入 out；\edef 扫描器会原样吸收）
                    let toks = self.scan_expanded_group()?;
                    out.extend(toks.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::String_) => {
                    // \string<token>：token 转文本（字符序列）。用 string_token
                    // （tex.web sprint_cs 语义：控制词后**不**补空格——\detokenize
                    // 才补。expl3 cs_split/cs_to_str 依赖无空格签名）。转义字符按
                    // \escapechar（tex.web print_esc：0..=255 才打印）——
                    // plain \newif 的 \string\iffoo 在 \escapechar=-1 下须无前导 \。
                    let t = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("\\string 后无 token"))?
                        .0;
                    let mut buf = Vec::new();
                    string_token(t, &self.intern, self.params.misc[34], &mut buf);
                    out.extend(buf.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::Meaning) => {
                    // \meaning<token>：token 含义文本（与 exec_meaning 对齐）。
                    // 此前缺失：\meaning 在 is_expandable() 中但此处落入 `_` 分支被原样
                    // 保留，`\the\meaning\cs` 使 the_tokens_after 对 \meaning 无限递归
                    // → 输入栈溢出（fuzz 命中，畸形输入不 panic 契约违约）。
                    // 空格 cat 10（str_char_token 重扫描语义）：edef 内
                    // `\Ifstrstart{\meaning #1}{…}` 的定界匹配含空格
                    // （scrbase `\Ifisinteger` 族）——全 Other 时 KOMA 节键全灭。
                    let t = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("\\meaning 后无 token"))?
                        .0;
                    let text = self.meaning_text(t);
                    out.extend(text.bytes().map(|b| (str_char_token(b), false)));
                }
                EqSlot::Primitive(Primitive::JobName) => {
                    // \jobname：作业名（与 exec 对齐：恒 "texput"）。
                    // 此前缺失：同 \meaning，`\the\jobname` 会无限递归。
                    out.extend(
                        "texput"
                            .bytes()
                            .map(|b| (Token::char(Catcode::Other, u32::from(b)), false)),
                    );
                }
                EqSlot::Primitive(Primitive::Csname) => {
                    // \csname...\endcsname：名字扫描 → 控制序列 token（TeX expand() 语义）
                    let name = self.scan_csname()?;
                    let csid = self.intern.intern(&name);
                    if diag_enabled("NTEX_IFX_TRACE") {
                        eprintln!("[trace-csname-the] line={} 制造: {name}", self.current_line_no());
                    }
                    // TeX eq_define(cs,relax,256)：未定义名先变 \relax 同义再放回
                    self.csname_define_relax(csid);
                    out.push((Token::control_sequence(csid), false));
                }
                // ETRIP 冲刺：e-TeX marks 族查询（可展开，返回字符 token 文本）
                // \topmarks<n> / \firstmarks<n> / \botmarks<n> / \splitfirstmarks<n> /
                // \splittopmarks<n> / \splitbotmarks<n>：扫描 class 号，向 sink 查询，
                // 返回内容转为字符 token（空内容输出空）。
                EqSlot::Primitive(p @ (Primitive::TopMarks
                    | Primitive::FirstMarks
                    | Primitive::BotMarks
                    | Primitive::SplitFirstMarks
                    | Primitive::SplitTopMarks
                    | Primitive::SplitBotMarks)) => {
                    let class = self.scan_number()?;
                    let text = match p {
                        Primitive::TopMarks => self.sink.topmarks(class),
                        Primitive::FirstMarks => self.sink.firstmarks(class),
                        Primitive::BotMarks => self.sink.botmarks(class),
                        Primitive::SplitFirstMarks => self.sink.splitfirstmarks(class),
                        Primitive::SplitTopMarks => self.sink.splittopmarks(class),
                        Primitive::SplitBotMarks => self.sink.splitbotmarks(class),
                        _ => unreachable!("marks 族已在上层 match 穷举"),
                    };
                    out.extend(text.bytes().map(|b| {
                        let cat = if b == b' ' {
                            Catcode::Space
                        } else {
                            Catcode::Other
                        };
                        (Token::char(cat, u32::from(b)), false)
                    }));
                }
                // TRIP 补全批次：TeX 版 marks（\topmark 等，class 0 不扫描）
                EqSlot::Primitive(p @ (Primitive::TopMark
                    | Primitive::FirstMark
                    | Primitive::BotMark
                    | Primitive::SplitFirstMark
                    | Primitive::SplitBotMark)) => {
                    let text = match p {
                        Primitive::TopMark => self.sink.topmarks(0),
                        Primitive::FirstMark => self.sink.firstmarks(0),
                        Primitive::BotMark => self.sink.botmarks(0),
                        Primitive::SplitFirstMark => self.sink.splitfirstmarks(0),
                        _ => self.sink.splitbotmarks(0),
                    };
                    out.extend(text.bytes().map(|b| {
                        let cat = if b == b' ' {
                            Catcode::Space
                        } else {
                            Catcode::Other
                        };
                        (Token::char(cat, u32::from(b)), false)
                    }));
                }
                // fuzz 挂死修复（2026-08-28）：以下原语在 is_expandable() 白名单中，
                // 但此前此处无分支 → 落 `_` 原样保留。扫描循环（scan_dimen_inner/
                // scan_number 的"可展开 → 展开后重试"）撞上它们时展开结果仍是自己，
                // 无限空转零消费（`\box\muexpr\romannumeral` 触发；内存随 TokenList
                // 帧无限 push 爆涨 → OOM）。与 exec_* 共用 helper，语义一致。
                EqSlot::Primitive(Primitive::RomanNumeral) => {
                    let toks = self.roman_numeral_tokens()?;
                    out.extend(toks.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::Char) => {
                    let toks = self.char_tokens()?;
                    out.extend(toks.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::Uppercase) => {
                    let toks = self.case_convert_tokens(true)?;
                    out.extend(toks.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::Lowercase) => {
                    let toks = self.case_convert_tokens(false)?;
                    out.extend(toks.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::EndInput) => {
                    // \endinput 展开语义 = 执行语义（弹 Source 帧/置 ended），无 token 输出
                    self.exec_endinput()?;
                }
                EqSlot::Primitive(Primitive::Ignorespaces) => {
                    // \ignorespaces 展开语义 = 执行语义（跳空格），无 token 输出
                    self.exec_ignorespaces()?;
                }
                EqSlot::Primitive(Primitive::FontName) => {
                    let font = self.scan_font_ident()?;
                    let name = self
                        .font_names
                        .get(font as usize)
                        .and_then(|n| n.clone())
                        .unwrap_or_default();
                    out.extend(
                        name.bytes()
                            .map(|b| {
                                let cat = if b == b' ' {
                                    Catcode::Space
                                } else {
                                    Catcode::Other
                                };
                                (Token::char(cat, u32::from(b)), false)
                            }),
                    );
                }
                // LaTeX 兼容第八刀：pdfTeX 可展开族（is_expandable_prim 白名单成员，
                // 此处必须有分支——否则 `_` 原样保留触发"展开后重试"空转，
                // 见上方 fuzz 挂死修复注释）。语义与 dispatch_expandable 对齐。
                EqSlot::Primitive(Primitive::Unless) => {
                    // e-TeX：`\unless` 可展开（f/e 型实参扫描同 \edef 语义）——
                    // 就地拉取下一个 `\if*` 取反求值（expand_unless_in_place
                    // 文档）。u3 探针：`\edef\zz{\reverse_if:N \if_charcode:w
                    // a a X\else Y\fi}` 应得 `Y`（unless 取反：`\if_charcode`
                    // 真 → 假 → 走 \else 支），而非未展开残留 `\reverse_if:NX`。
                    if !self.expand_unless_in_place()? {
                        out.push((tok, false));
                    }
                }
                EqSlot::Primitive(Primitive::PdfTeXVersion) => {
                    out.extend(emit_count(140).into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::PdfTeXRevision) => {
                    out.extend(emit_count(25).into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::PdfShellEscape) => {
                    out.extend(emit_count(0).into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::PdfElapsedTime) => {
                    out.extend(emit_count(0).into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::PdfRandomSeed) => {
                    let v = self.params.misc[PDF_RANDOM_SEED_IDX];
                    out.extend(emit_count(v).into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::PdfTeXBanner) => {
                    out.extend(pdf_banner_tokens().into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::PdfCreationDate) => {
                    let p = |q: Primitive| {
                        self.params.misc[int_param_index(q).expect("日期时间参数在 misc 表")]
                    };
                    let toks = pdf_creation_date_tokens(
                        p(Primitive::Day),
                        p(Primitive::Month),
                        p(Primitive::Year),
                        p(Primitive::Time),
                    );
                    out.extend(toks.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::PdfStrCmp) => {
                    let a = self.scan_group_contents_xpand(true)?;
                    let b = self.scan_group_contents_xpand(true)?;
                    let v = pdf_strcmp_value(&a, &b, &self.intern, self.params.misc[34]);
                    out.extend(emit_count(v).into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::PdfFileSize) => {
                    let name = self.scan_file_name()?;
                    if let Some(bytes) = self
                        .vfs
                        .read(&name)
                        .map_err(|e| Error::io("VFS 读取", &name, e))?
                    {
                        out.extend(
                            emit_count(bytes.len() as i64)
                                .into_iter()
                                .map(|t| (t, false)),
                        );
                    }
                }
                EqSlot::Primitive(Primitive::PdfUniformDeviate) => {
                    let n = self.scan_number()?;
                    let state = self.params.misc[PDF_RANDOM_SEED_IDX] as u32;
                    let next = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    self.params.misc[PDF_RANDOM_SEED_IDX] = i64::from(next);
                    let r = if n <= 0 { 0 } else { i64::from((next >> 8) % (n as u32)) };
                    out.extend(emit_count(r).into_iter().map(|t| (t, false)));
                }
                _ => {
                    // 未定义/不可展开原语：原样保留
                    out.push((tok, false));
                }
            }
        } else {
            out.push((tok, false));
        }
        Ok(())
    }

    // ---------- M4-5 e-TeX 展开扩展 ----------

    /// 把 token 序列压入输入流（可展开项将被展开）。
    fn emit_tokens(&mut self, tokens: Vec<Token>) -> Result<()> {
        let items: Vec<(Token, bool)> = tokens.into_iter().map(|t| (t, false)).collect();
        self.push_frame(InputFrame::TokenList {
            items: Arc::from(items),
            pos: 0,
        });
        Ok(())
    }

    /// `\numexpr` 整数表达式：`<term> (('+'|'-'|'*'|'/') <term>)*`。
    ///
    /// eTeX 语义：中间量用 i128（`mult_and_add` 64 位中间等价），**仅最终结果**
    /// 超出 ±0x7FFFFFFF 才报 "! Arithmetic overflow." 并取 0（etrip "Expr fraction
    /// rounding"：`"7FFFFFFE*"7FFFFFFE/"7FFFFFFD` 的中间乘积 2^62 不得误判溢出）。
    fn eval_int_expression(&mut self) -> Result<i64> {
        let mut value = self.expr_mul_term()?;
        while let Some(op) = self.peek_int_op(true)? {
            if op != b'+' && op != b'-' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            let rhs = self.expr_mul_term()?;
            value = if op == b'+' { value + rhs } else { value - rhs };
        }
        if value > i128::from(MAX_INT) || value < -i128::from(MAX_INT) {
            self.report_error("Arithmetic overflow.");
            return Ok(0);
        }
        Ok(value as i64)
    }

    /// 乘法项：`factor (('*'|'/') factor)*`（i128 中间量，见 [`Self::eval_int_expression`]）。
    fn expr_mul_term(&mut self) -> Result<i128> {
        let mut value = i128::from(self.expr_factor()?);
        while let Some(op) = self.peek_int_op(false)? {
            if op != b'*' && op != b'/' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            let rhs = i128::from(self.expr_factor()?);
            value = if op == b'*' {
                value * rhs
            } else if rhs == 0 {
                // eTeX：除零 → "! Arithmetic overflow."，结果 0（etrip L789-791）
                self.report_error("Arithmetic overflow.");
                0
            } else {
                expr_quotient_i128(value, rhs)
            };
        }
        Ok(value)
    }

    /// 表达式因子位的 get_x_token 前瞻：把可展开项（宏/`\number`/`\romannumeral`/
    /// `\expandafter` 等）就地展开、条件开始就地步进、跳过区就地推进，直到
    /// 不可展开 token 放回并返回——etex.web scan_expr 的取 token 循环同为
    /// get_x_token，`( <expr> )` 因子臂因此必须对**展开产物**判定。
    ///
    /// expl3 `\int_div_truncate:nn`（l.6652）的
    /// `\exp_after:wN \__int_div_truncate:NwNw \int_value:w ...` 让
    /// `\__int_div_truncate:NwNw` 在因子位展开出 `( ... )`（l.6670）——缺此前瞻
    /// 则 `(` 落进 scan_int 的十进制循环报 "Missing number, treated as zero"，
    /// 收尾再报 "Missing ) inserted for expression"（l.8023
    /// `\c_sys_engine_version_str` 的 pdftex 分支，2+1 条 + 下游级联）。
    fn expr_peek_factor_token(&mut self) -> Result<Option<Token>> {
        loop {
            self.skip_spaces()?;
            let Some((tok, _)) = self.fetch()? else {
                return Ok(None);
            };
            // 条件机跳过区：同 scan_int 符号循环（scan.rs 同款臂）——
            // `\else` 之后假分支 token 不得被表达式吸收。
            if self.is_skipping() {
                if let Some(op) = self.cond_op(tok) {
                    self.step_conditional(op, tok)?;
                }
                continue;
            }
            if let Some(csid) = tok.csid() {
                if matches!(self.eqtb.slot(csid), EqSlot::Undefined) {
                    let _ = self.sink.write16(format!(
                        "! Undefined control sequence.\n\\{}\n",
                        self.intern.name(csid)
                    ));
                    continue;
                }
                if self.maybe_eval_cond(tok)? {
                    continue;
                }
                // tex.web expand 的 fi_or_else 臂（§9897）：因子位的 get_x_token
                // 对 `\else`/`\fi`/`\or` 同样经 expand → conditional() 处理——栈顶
                // 帧求值中（if_limit=if_code）由 insert_relax 门拦截（token 放回 +
                // frozen `\relax` 前插）；帧已完成求值则就地闭合/转臂。
                //
                // expl3 `\__int_div_truncate:NwNw`（l.6660-6674）的体在表达式里嵌
                //   `#1#2 \if_meaning:w - #1 + \else: - \fi: ( … ) / 2`
                // ——`\if_meaning:w` 由数字扫描的十进制循环就地求值（假分支同步
                // skip_ahead、帧留栈等 `\fi`），其 `\fi:` 随即落到**因子位**：
                // 不消费则被当因子放回 scan_number，帧由符号循环弹出、`( … )` 撞
                // 十进制循环报 "Missing number" → 表达式在 `-` 后收 0、外层 `(` 配
                // 不上 `)` 再报 "Missing )"，值失真为 `140(100-1)/2)/100`
                // （l.8023 `\c_sys_engine_version_str` pdftex 分支 +
                // `\int_div_truncate` 全线）。游离终结符（无帧可归属）维持放回
                // （TRIP L82 游离 `\fi` 契约）。
                if let Some(op) = self.cond_op(tok) {
                    if matches!(op, CondOp::Fi | CondOp::Else | CondOp::Or) {
                        // 游离终结符按因子位契约原样返回（调用方 expr_factor 负责
                        // unread——此处不得自放回，否则 token 翻倍）。
                        if self.cond_stack.is_empty() {
                            return Ok(Some(tok));
                        }
                        self.step_conditional(op, tok)?;
                        continue;
                    }
                }
                // 别名即原义：宏别名按目标含义展开（tex.web get_x_token；Alias 槽
                // 只指向宏/未定义，见 scan.rs deref_alias_chain）。
                let expandable = match self.eqtb.slot(self.deref_alias_chain(csid)).clone() {
                    EqSlot::Macro(_) => true,
                    EqSlot::Primitive(p) if p.is_expandable() => true,
                    _ => false,
                };
                if expandable {
                    let mut expansion = Vec::new();
                    self.expand_once((tok, false), &mut expansion)?;
                    if !expansion.is_empty() {
                        self.push_frame(InputFrame::TokenList {
                            items: Arc::from(expansion),
                            pos: 0,
                        });
                    }
                    continue;
                }
            }
            return Ok(Some(tok));
        }
    }

    /// 整数因子：`(` <表达式> `)`（TeX 括号子表达式）或 [`Self::scan_number`]。
    fn expr_factor(&mut self) -> Result<i64> {
        let Some(t) = self.expr_peek_factor_token()? else {
            return Err(Error::invalid_input("\\numexpr 表达式未闭合"));
        };
        if t.charcode() == Some(b'(' as u32) {
            let v = self.eval_int_expression()?;
            let close = self.fetch()?;
            match close {
                Some((c, _)) if c.charcode() == Some(b')' as u32) => {}
                _ => {
                    // TeX 恢复：报 "Missing ) inserted" 并插入 `)` 继续
                    if let Some((c, _)) = close {
                        self.unread(c);
                    }
                    self.report_error("Missing ) inserted for expression.");
                }
            }
            return Ok(v);
        }
        // eTeX 表达式分组：{1+}{2*3} 的 {1+} 组（组只是分隔；组内扫描因子后，
        // 剩余 token 放回给表达式循环——etrip L880）。scan_int 本身不处理组
        // （{ 在普通整数上下文报 Missing number，TRIP l.106）。
        if t.catcode() == Some(Catcode::BeginGroup) {
            let v = self.scan_number()?;
            let mut pending: Vec<Token> = Vec::new();
            while let Some((c, _)) = self.fetch()? {
                if c.catcode() == Some(Catcode::EndGroup) {
                    break;
                }
                pending.push(c);
            }
            for tok in pending.into_iter().rev() {
                self.unread(tok);
            }
            return Ok(v);
        }
        self.unread(t);
        self.scan_number()
    }

    /// 取下一个整数运算符（`+ - * /`）或 `\relax`（结束符，吸收）；其余 token 放回。
    ///
    /// `absorb_relax` 只在**加法层**（eval_*_expression 的运算符循环）为真：
    /// 本引擎的乘/加两级各做一次前瞻（etex.web scan_expr 是单层循环单次前瞻），
    /// 若乘法层也吸收 `\relax`，加法层的第二次前瞻就越过了表达式终点、把外侧
    /// token（如 `\glueexpr 1pt \relax\the\skip0` 的 `\the`）展开吞掉。
    ///
    /// 前瞻是 **get_x_token** 语义（etex.web scan_expr 运算符循环
    /// `get_x_token; if cur_tok<>plus/minus then back_input`）：可展开 token
    /// 先展开一次，以展开产物的**首 token** 判定运算符/终结符；判为终结符时
    /// 该首 token 放回，其余展开产物仍在输入流中（顺序不变）。
    ///
    /// expl3 的 `\int_value:w \__int_eval:w <n> \exp_after:wN \__int_sep:`（
    /// `\int_step_function:nnnN`、`\__char_generate_aux:w` 等）正依赖这一步把
    /// `\expandafter` 在前瞻里就地消化——`\__int_sep:`（`\let`，不可展开）落为
    /// 终结符、后续 `\int_value:w` 的求值产物排在其后。若按 get_token 直读，
    /// `\expandafter` 被原样放回，调用方的定界实参扫描便把 `\expandafter` 与
    /// 下一值整段吞进同一个实参（expl3-code l.9364 `\char_generate:nn`
    /// bootstrap 区 1812 条 "Missing = inserted for \ifnum" 的根因：`\ifnum`
    /// 关系符位读到表达式里的 `+`）。
    fn peek_int_op(&mut self, absorb_relax: bool) -> Result<Option<u8>> {
        loop {
            self.skip_spaces()?;
            let Some((tok, noexpand)) = self.fetch()? else {
                return Ok(None);
            };
            // \relax 终止表达式：`\relax` 原语、`\let\9=\relax` 别名，或
            // `\def\9{\relax}` 宏（etrip 大量用 `\9` 收尾——942 行是宏定义而非 \let）。
            if let Some(id) = tok.csid() {
                let mut cur = id;
                let mut depth = 0;
                let mut is_relax = false;
                loop {
                    match self.eqtb.slot(cur) {
                        EqSlot::Alias(t) => {
                            cur = *t;
                            depth += 1;
                            if depth > 100 {
                                break;
                            }
                        }
                        EqSlot::Macro(m) => {
                            // 宏体为单个 `\relax`（如 `\def\9{\relax}`）→ 等价终止符
                            is_relax = m.value.body.len() == 1
                                && self.is_relax_token(&m.value.body[0]);
                            break;
                        }
                        EqSlot::Primitive(Primitive::Relax) => {
                            is_relax = true;
                            break;
                        }
                        _ => break,
                    }
                }
                if is_relax {
                    if !absorb_relax {
                        self.unread(tok);
                    }
                    return Ok(None); // \relax 吸收
                }
                // 条件开始（`\if*`）在运算符位**就地求值**（tex.web get_x_token →
                // expand → conditional()：条件连同操作数就地消费、不产 token，取
                // token 循环随后拿分支首 token）。expl3 `\__int_div_truncate:NwNw`
                // 的体（l.6660）在表达式里嵌
                //   `#1#2 \if_meaning:w - #1 + \else: - \fi: ( ... ) / 2`
                // ——运算符位遇 `\if_meaning:w` 必须求值并落到分支 token（`-`），
                // 否则表达式在条件处提前收口、`( ... ) / 2` 残留流中
                // （`\c_sys_engine_version_str` 的 "Missing ) inserted" +
                // `\int_div_truncate` 全线 140(100-1)/2)/100 失真）。
                // `\else`/`\fi`/`\or` 仍不在此臂：它们属外层条件机，放回由
                // scan_int 的条件臂/主循环消费（TRIP L82 游离 `\fi` 契约）。
                if self.cond_op(tok).is_some() && self.maybe_eval_cond(tok)? {
                    continue;
                }
                // fi_or_else 臂：运算符位的 get_x_token 对 `\else`/`\fi`/`\or`
                // 同样经 expand → conditional()（§9897）——帧求值中由 insert_relax
                // 门拦截，帧已完成则就地闭合/转臂后继续前瞻。expl3
                // `\__int_div_truncate:NwNw` 体内嵌
                //   `… \if_meaning:w - #1 + \else: - \fi: ( … ) / 2`
                // 的 `\fi:` 若在此被当终结符放回，表达式提前收口、`( … ) / 2`
                // 残留流中（同 expr_peek_factor_token 臂注）。游离终结符（无帧可
                // 归属）维持"放回 + 表达式收口"（TRIP L82 契约）。
                if let Some(op) = self.cond_op(tok) {
                    if matches!(op, CondOp::Fi | CondOp::Else | CondOp::Or) {
                        if self.cond_stack.is_empty() {
                            self.unread(tok);
                            return Ok(None);
                        }
                        self.step_conditional(op, tok)?;
                        continue;
                    }
                }
                // get_x_token 展开臂：宏（非 protected 抑制面）与可展开原语展开
                // 一次后重探。`\noexpand` 冻结的 token 不展开（e-TeX 语义）。
                if !noexpand {
                    let expandable =
                        match self.eqtb.slot(self.deref_alias_chain(id)).clone() {
                            EqSlot::Macro(m) => {
                                !(m.value.protected && self.suppress_expansion > 0)
                            }
                            EqSlot::Primitive(p) => p.is_expandable(),
                            _ => false,
                        };
                    if expandable {
                        let mut expansion = Vec::new();
                        self.expand_once((tok, false), &mut expansion)?;
                        if !expansion.is_empty() {
                            self.push_frame(InputFrame::TokenList {
                                items: Arc::from(expansion),
                                pos: 0,
                            });
                        }
                        continue;
                    }
                }
            }
            if tok.catcode() == Some(Catcode::Other) {
                if let Some(ch) = tok.charcode() {
                    if matches!(ch, 0x2B | 0x2D | 0x2A | 0x2F) {
                        // + - * /
                        return Ok(Some(ch as u8));
                    }
                }
            }
            self.unread(tok);
            return Ok(None);
        }
    }

    /// token 是否等价于 `\relax`（原语或经 `\let` 别名链指向 `\relax`）。
    fn is_relax_token(&self, tok: &Token) -> bool {
        let Some(id) = tok.csid() else {
            return false;
        };
        let mut cur = id;
        let mut depth = 0;
        loop {
            match self.eqtb.slot(cur) {
                EqSlot::Alias(t) => {
                    cur = *t;
                    depth += 1;
                    if depth > 100 {
                        return false;
                    }
                }
                EqSlot::Primitive(Primitive::Relax) => return true,
                _ => return false,
            }
        }
    }

    /// dimen/glue 表达式 `*`/`/` 的 number 因子：`( <int expr> )` 或 [`Self::scan_number`]
    /// （etrip L854：`\dimexpr(#3sp)*(#4)/(#5)` 的括号乘数）。
    fn expr_number_factor(&mut self) -> Result<i128> {
        let Some(t) = self.expr_peek_factor_token()? else {
            return Err(Error::invalid_input("表达式缺少数字因子"));
        };
        if t.charcode() == Some(b'(' as u32) {
            let v = self.eval_int_expression()?;
            let close = self.fetch()?;
            match close {
                Some((c, _)) if c.charcode() == Some(b')' as u32) => {}
                _ => {
                    if let Some((c, _)) = close {
                        self.unread(c);
                    }
                    self.report_error("Missing ) inserted for expression.");
                }
            }
            return Ok(i128::from(v));
        }
        self.unread(t);
        Ok(i128::from(self.scan_number()?))
    }

    /// `\dimexpr` 尺寸表达式：`<dimen> (('+'|'-') <dimen>)*`（e-TeX 文法子集：
    /// 每项为 [`Self::scan_dimen`] 可识别的尺寸；支持 `( <expr> )` 括号；
    /// `\relax` 或不可识别 token 结束，后者放回）。
    fn eval_dimen_expression(&mut self) -> Result<i64> {
        let mut value = i128::from(self.dimen_expr_term()?);
        while let Some(op) = self.peek_int_op(true)? {
            if op != b'+' && op != b'-' && op != b'*' && op != b'/' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            if op == b'*' || op == b'/' {
                // e-TeX：`<dimen> * <number>` 与 `<dimen> / <number>`（etrip L780/785）
                let rhs = self.expr_number_factor()?;
                value = if op == b'*' {
                    value * rhs
                } else if rhs == 0 {
                    self.report_error("Arithmetic overflow.");
                    0
                } else {
                    expr_quotient_i128(value, rhs)
                };
            } else {
                let rhs = i128::from(self.dimen_expr_term()?);
                value = if op == b'+' { value + rhs } else { value - rhs };
            }
        }
        // 仅最终结果超限才报（中间量 i128 不逐项检查，与 eTeX 一致）
        if value > i128::from(MAX_DIMEN) || value < -i128::from(MAX_DIMEN) {
            self.report_error("Arithmetic overflow.");
            return Ok(0);
        }
        Ok(value as i64)
    }

    /// 尺寸表达式项：`( <expr> )` 括号或 [`Self::scan_dimen`]。
    fn dimen_expr_term(&mut self) -> Result<i64> {
        let Some(t) = self.expr_peek_factor_token()? else {
            return Err(Error::invalid_input("\\dimexpr 表达式未闭合"));
        };
        if t.charcode() == Some(b'(' as u32) {
            let v = self.eval_dimen_expression()?;
            let close = self.fetch()?;
            match close {
                Some((c, _)) if c.charcode() == Some(b')' as u32) => {}
                _ => {
                    if let Some((c, _)) = close {
                        self.unread(c);
                    }
                    self.report_error("Missing ) inserted for expression.");
                }
            }
            return Ok(v);
        }
        // eTeX 表达式分组：{7pt+}{12pt/4} 的 {7pt+} 组（etrip L884-888）。
        if t.catcode() == Some(Catcode::BeginGroup) {
            let v = self.scan_dimen()?;
            let mut pending: Vec<Token> = Vec::new();
            while let Some((c, _)) = self.fetch()? {
                if c.catcode() == Some(Catcode::EndGroup) {
                    break;
                }
                pending.push(c);
            }
            for tok in pending.into_iter().rev() {
                self.unread(tok);
            }
            return Ok(v);
        }
        self.unread(t);
        self.scan_dimen()
    }

    /// `\glueexpr`/`\muexpr` 胶水表达式：`<glue> (('+'|'-'|'*'|'/') <glue>)`。
    /// width 逐项求和；stretch/shrink **值求和**，其**无穷阶 = 最后一个非零分量项**
    /// 的阶（无则 NORMAL；etrip L800/L950：`\skip90+0pt` 保留 1fil、`\skip5+0pt` 清 0）。
    fn eval_glue_expression(&mut self, mu: bool) -> Result<Glue> {
        let first = self.glue_expr_mul_term(mu)?;
        let mut width = i128::from(first.width);
        let mut stretch = first.stretch;
        let mut shrink = first.shrink;
        let mut stretch_order = if first.stretch != 0 {
            first.stretch_order
        } else {
            0
        };
        let mut shrink_order = if first.shrink != 0 {
            first.shrink_order
        } else {
            0
        };
        let mut has_op = false;
        while let Some(op) = self.peek_int_op(true)? {
            if op != b'+' && op != b'-' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            has_op = true;
            let term = self.glue_expr_mul_term(mu)?;
            if op == b'+' {
                width += i128::from(term.width);
                stretch += term.stretch;
                shrink += term.shrink;
            } else {
                width -= i128::from(term.width);
                stretch -= term.stretch;
                shrink -= term.shrink;
            }
            if term.stretch != 0 {
                stretch_order = term.stretch_order;
            }
            if term.shrink != 0 {
                shrink_order = term.shrink_order;
            }
        }
        // 仅一项（无任何运算符）：保留 first 的完整 order（0 值分量的阶也保留，
        // etrip L949：`\glueexpr\mutoglue\muexpr\gluetomu\skip5` 的 0shrink 保留 fil）
        if !has_op {
            stretch_order = first.stretch_order;
            shrink_order = first.shrink_order;
        }
        // 仅最终宽度超限才报（中间量 i128，与 eTeX 一致）
        if width > i128::from(MAX_DIMEN) || width < -i128::from(MAX_DIMEN) {
            self.report_error("Arithmetic overflow.");
            width = 0;
        }
        Ok(Glue {
            width: width as i64,
            stretch,
            shrink,
            stretch_order,
            shrink_order,
        })
    }

    /// 胶水乘法项：`<胶水项> (('*'|'/') <factor>)*`——`*/` 优先级高于 `+ -`，
    /// 作用于**本项**（`7pt+12pt/4` = 7pt + (12pt/4)，etrip L888）。
    fn glue_expr_mul_term(&mut self, mu: bool) -> Result<Glue> {
        let mut g = self.glue_expr_term(mu)?;
        while let Some(op) = self.peek_int_op(false)? {
            if op != b'*' && op != b'/' {
                self.unread(Token::char(Catcode::Other, op as u32));
                break;
            }
            let rhs = self.expr_number_factor()?;
            let w = if op == b'*' {
                i128::from(g.width) * rhs
            } else if rhs == 0 {
                self.report_error("Arithmetic overflow.");
                0
            } else {
                expr_quotient_i128(i128::from(g.width), rhs)
            };
            g.width = w as i64;
        }
        Ok(g)
    }

    /// 胶水表达式项：`( <expr> )` 括号或 [`Self::scan_glue`]/[`Self::scan_glue_mu`]。
    /// 括号内嵌套同一单位上下文（`\muexpr(5muminus1mu)`、`\glueexpr(\muexpr...)` 由
    /// 前导量分支报 "Incompatible glue units"）。
    fn glue_expr_term(&mut self, mu: bool) -> Result<Glue> {
        let Some(t) = self.expr_peek_factor_token()? else {
            return Err(Error::invalid_input("\\glueexpr 表达式未闭合"));
        };
        if t.charcode() == Some(b'(' as u32) {
            let v = self.eval_glue_expression(mu)?;
            let close = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("\\glueexpr 括号未闭合"))?
                .0;
            if close.charcode() != Some(b')' as u32) {
                self.unread(close);
                self.report_error("Missing ) inserted for expression.");
            }
            return Ok(v);
        }
        // eTeX 表达式分组：{...} 组（组内胶水+运算符，etrip 同 dimen 因子）。
        if t.catcode() == Some(Catcode::BeginGroup) {
            let v = if mu {
                self.scan_glue_mu()?
            } else {
                self.scan_glue()?
            };
            let mut pending: Vec<Token> = Vec::new();
            while let Some((c, _)) = self.fetch()? {
                if c.catcode() == Some(Catcode::EndGroup) {
                    break;
                }
                pending.push(c);
            }
            for tok in pending.into_iter().rev() {
                self.unread(tok);
            }
            return Ok(v);
        }
        self.unread(t);
        if mu {
            self.scan_glue_mu()
        } else {
            self.scan_glue()
        }
    }

    /// `\detokenize{...}`：组内容转字符 token 流（字符 catcode 12、空格 10、
    /// 控制序列 → `\名字` 文本），作为输入继续处理。
    fn exec_detokenize(&mut self) -> Result<()> {
        let toks = self.scan_group_contents_expanding()?;
        let mut out = Vec::new();
        let esc = self.params.misc[34];
        for t in toks {
            detokenize_token(t, &self.intern, esc, &mut out);
        }
        self.emit_tokens(out)
    }

    /// `\unexpanded{...}`：组内容作为 token 流输出。
    /// - 展开上下文（`\edef`/`\write`）：标记 noexpand，内容不再展开（e-TeX 语义）；
    /// - 主循环执行：正常执行（`\unexpanded{\def\1{...}}` 中 `\def` 生效）。
    fn exec_unexpanded(&mut self) -> Result<()> {
        let toks = self.scan_group_contents_expanding()?;
        let flag = self.expand_only;
        let items: Vec<(Token, bool)> = toks.into_iter().map(|t| (t, flag)).collect();
        self.push_frame(InputFrame::TokenList {
            items: Arc::from(items),
            pos: 0,
        });
        Ok(())
    }

    /// pdfTeX `\expanded{<balanced text>}`（可展开）：组内容按 `\edef` 语义
    /// **全展开**后放回输入流继续处理（`\edef\1{...}` 的体内联版）。
    /// 主循环执行与展开上下文（`\edef`/`\write`/外层 `\expanded`）同路径。
    fn exec_expanded(&mut self) -> Result<()> {
        let toks = self.scan_expanded_group()?;
        self.emit_tokens(toks)
    }

    /// `\expanded` 实参扫描：scan_left_brace（filler 语义——跳空格/`\relax`、
    /// 展开可展开项）消费强制 `{` 后，组内容按 `scan_toks(macro_def=false,
    /// xpand=true)` 扫描（protected 宏抑制展开——pdfTeX 语义同 `\edef`；
    /// 可展开项展开后压帧递归、条件即时求值）。与 `\edef` 体的关键差异：
    /// **不做参数 `#` 处理**（tex.web scan_toks macro_def=false——字面 `#`
    /// 原样收集、不报 Illegal parameter number；expl3-code l.9356-9372 经
    /// `\lowercase` 构造 catcode 查表时 `#` 进 `\expanded` 实参即依赖此）。
    fn scan_expanded_group(&mut self) -> Result<Vec<Token>> {
        self.scan_left_brace()?;
        self.suppress_expansion += 1;
        // macro_def=false：不报 IPN，def_name 不再使用（此前传空串使消息
        // 缺 "of \X" 段——现参数 `#` 检查整体关闭）。
        let scanned = self.scan_edef_body("", false);
        self.suppress_expansion -= 1;
        scanned
    }

    /// `\scantokens{...}`（M4-5 e-TeX）：组内容 detokenize 为文本后按**当前**
    /// catcode 重新扫描（eTeX 语义：等价于从字符串 `\input`）。
    /// 参数为 `<general text>`：先展开可展开项（`\scantokens\expandafter{\1}`）。
    fn exec_scantokens(&mut self) -> Result<()> {
        let toks = self.scan_group_contents_expanding()?;
        let mut text: Vec<Token> = Vec::new();
        let esc = self.params.misc[34];
        for t in toks {
            detokenize_token(t, &self.intern, esc, &mut text);
        }
        let bytes: Vec<u8> = text
            .iter()
            .filter_map(|t| t.charcode().and_then(|c| u8::try_from(c).ok()))
            .collect();
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

    /// `\csname<name>\endcsname`：扫描名字，构造控制序列 token 并放回输入流
    /// （TeX expand() 语义：结果是可执行 token，主循环继续处理）。
    fn exec_csname(&mut self) -> Result<()> {
        let name = self.scan_csname()?;
        if diag_enabled("NTEX_IFX_TRACE") {
            eprintln!("[trace-csname] 制造: {name}");
        }
        let csid = self.intern.intern(&name);
        // TeX eq_define(cs,relax,256)：未定义名先变 \relax 同义再放回
        self.csname_define_relax(csid);
        let tok = Token::control_sequence(csid);
        self.push_frame(InputFrame::One {
            tok,
            noexpand: false,
        });
        Ok(())
    }

    /// `\csname` 对**未定义名**的制造语义（tex.web L7753-7754）：
    /// `eq_define(cur_cs,relax,256)`——该 cs 变为与 `\relax` 原语同义
    /// （`\ifx\csname x\endcsname\relax` 为真、执行 no-op、`\ifdefined` 为真、
    /// `\meaning` 不再显示 undefined）。只作用于"制造时槽为 Undefined"的名字；
    /// 已定义（宏/原语/\let）不重定义。
    ///
    /// 组作用域：TeX 2.9 起该定义为**局部**（tex.web 版本注记 "Version 2.9
    /// made \csname\endcsname's relax local"）——组内制造在组末恢复未定义
    /// （与 \def 同走 save_stack）。制造发生在 expand() 中、**非赋值命令**：
    /// 不消费 `\global` 前缀、不触发 `\afterassignment`、不受 \globaldefs
    /// 影响，故不走 `set_slot_scoped`（其 is_global 会错吞 \global）。
    ///
    /// 记录偏差：
    /// 1. 引擎存为 `EqSlot::Primitive(Relax)`（与 `\relax` 原语同槽）。\ifx/
    ///    执行/\ifdefined 与 TeX 一致；唯 e-TeX `\ifprimitive` 对制造出的
    ///    relax 会误判为真（TeX 中它是 eq_define 产物、非原语）。
    /// 2. \meaning 对 Primitive 槽按**当前 cs 名**显示 `\名`（primitive.rs
    ///    meaning_text 既有约定）——\csname 制造出的 relax 显示 `\nope`，
    ///    而 TeX 按含义输出 "relax"。非本刀引入（\relax 原语因名恰为 relax
    ///    故表面一致）；留待 \meaning 语义刀。
    fn csname_define_relax(&mut self, csid: u32) {
        if !matches!(self.eqtb.slot(csid), EqSlot::Undefined) {
            return;
        }
        if self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Eqtb {
                    csid,
                    prev: EqSlot::Undefined,
                    prev_level: self.eqtb.level(csid),
                },
            ));
        }
        *self.eqtb.slot_mut(csid) = EqSlot::Primitive(Primitive::Relax);
        self.eq_mark_level(csid, false);
    }

    /// `\csname` 名字扫描（`\csname`/`\ifcsname` 用）：收集直到 `\endcsname` 的名字字符。
    /// get_x_token 语义：宏/可展开原语在名字中先展开一次；`\endcsname` 终止；
    /// 其余不可展开控制序列报错（TeX "Missing endcsname inserted"）。
    ///
    /// e-TeX `\ifincsname` 旗标由本层进出维持：保存旧值再置真（嵌套 csname 的
    /// 内层出口不得清掉外层旗标），错误路径（`?` 传播）同样还原。
    fn scan_csname(&mut self) -> Result<String> {
        let saved = std::mem::replace(&mut self.name_in_progress, true);
        let r = self.scan_csname_body();
        self.name_in_progress = saved;
        r
    }

    /// scan_csname 本体（`name_in_progress` 旗标由包装层管理）。
    fn scan_csname_body(&mut self) -> Result<String> {
        let mut name = String::new();
        loop {
            let tok = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("\\csname 未闭合（缺少 \\endcsname）"))?
                .0;
            if let Some(csid) = tok.csid() {
                // TeX scan_csname 终止判定按**含义**（tex.web `cur_cmd=end_csname`）：
                // cs 名不必是 "endcsname"——expl3 通篇用 `\cs_end:`（L1494
                // `\let\cs_end:\tex_endcsname:D`，槽 = EndCsname 原语）闭合
                // `\csname`。旧实现只认名为 "endcsname" 的 token → `\cs_end:`
                // 落入不可展开臂报 Missing endcsname（L1907 起每处
                // `\csname...\cs_end:` 级联）。沿别名链解引用后判槽。
                let mut id = csid;
                let mut hops = 0;
                while let EqSlot::Alias(next) = self.eqtb.slot(id) {
                    id = *next;
                    hops += 1;
                    if hops > 100 {
                        return Err(Error::invalid_input("\\let 别名环"));
                    }
                }
                if self.eqtb.slot(id) == &EqSlot::Primitive(Primitive::EndCsname) {
                    break;
                }
                // 条件原语在名字扫描内**就地求值**（tex.web scan_csname 的循环逐
                // token 走 `get_x_token` → expand 的 if_test/fi_or_else 分支）：
                // 真支字符收进名字、假支就地跳过。缺此臂时条件 token 落入不可展开
                // 臂报 "Missing endcsname inserted"（expl3-code l.6838
                // `\__int_compare:NNw` 分派名构造
                // `\use:c { __int_compare_ \token_to_str:N #1
                //  \if_meaning:w = #2 = \fi: :NNw }`——`\int_compare:n` 每次比较
                // 都走；l.3347 `\__quark_if_empty_if:o` 展开成不闭合的
                // `\if_meaning:w \q_nil … \q_nil` 留在流里等扫描器执行）。语义与
                // expr.rs `\expandafter` 臂同款：step_conditional + drain_open_skip
                // （不可用 maybe_eval_cond——此处是展开位置，get_x_token 本身）。
                if let Some(op) = self.cond_op(tok) {
                    let before = self.cond_stack.len();
                    self.step_conditional(op, tok)?;
                    if !matches!(op, CondOp::Fi) {
                        let depth = if matches!(op, CondOp::Else | CondOp::Or) {
                            before.saturating_sub(1)
                        } else {
                            before
                        };
                        self.drain_open_skip(depth)?;
                    }
                    continue;
                }
                match self.eqtb.slot(csid).clone() {
                    EqSlot::Macro(m) => {
                        // 0 参数宏同样须匹配纯定界串参数文本（tex.web macro_call
                        // `if info(r)<>end_match_token`）——\csname 名字扫描里展开
                        // 可展开宏时漏匹配会把定界串（如 fast-form 条件的
                        // `\fi: \use_none:n`）泄进名字文本 → "Missing endcsname
                        // inserted" 级联。
                        let args = self.collect_args(csid, &m.value)?;
                        let body = materialize(&m.value.body, &args);
                        let seq: Vec<(Token, bool)> =
                            body.into_iter().map(|t| (t, false)).collect();
                        self.push_frame(InputFrame::TokenList {
                            items: Arc::from(seq),
                            pos: 0,
                        });
                        continue;
                    }
                    EqSlot::Primitive(p) if p.is_expandable() => {
                        let mut out = Vec::new();
                        self.expand_once((tok, false), &mut out)?;
                        let seq: Vec<(Token, bool)> = out;
                        self.push_frame(InputFrame::TokenList {
                            items: Arc::from(seq),
                            pos: 0,
                        });
                        continue;
                    }
                    // TeX `scan_csname`（tex.web L19368-19379）：不可展开控制序列 →
                    // 报 "Missing endcsname inserted"，放回该 cs，插入 `\endcsname`
                    // 结束名字扫描（可恢复，不中断引擎；TRIP L428 `\csname^^Mendcsname=\^^@`）。
                    _ => {
                        let csname = self.intern.name(csid);
                        let _ = self.sink.write16(format!(
                            "! Missing endcsname inserted.\n<to be read again>\n \\{csname}\n"
                        ));
                        self.unread(tok);
                        return Ok(name);
                    }
                }
            }
            if let Some(ch) = tok.charcode().and_then(char::from_u32) {
                name.push(ch);
            }
        }
        Ok(name)
    }

}
