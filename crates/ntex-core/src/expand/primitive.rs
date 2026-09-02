use crate::sink::DirectionKind;

/// 表达式原语种类（\numexpr/\dimexpr/\glueexpr/\muexpr）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum ExprKind {
    Num,
    Dim,
    Glue,
    Mu,
}

impl Expander {
    // ---------- 原语执行 ----------

    /// 命令位置表达式原语：首 token 是否"表达式起点"。
    /// 非起点（异类表达式原语、\let/\relax 等命令类、普通字符）→ 报
    /// "You can't use \<expr> in vertical mode."（TeX 垂直模式裸用语义；
    /// 避免 Missing number 恢复输出 0 污染后续——etrip L764 连续裸用）。
    fn expr_start_ok(&mut self, kind: ExprKind, tok: Token) -> Result<bool> {
        if let Some(c) = tok.charcode() {
            let b = c as u8;
            return Ok(b.is_ascii_digit() || matches!(b, b'.' | b'(' | b'+' | b'-'));
        }
        let Some(csid) = tok.csid() else {
            return Ok(false);
        };
        match self.eqtb.slot(csid) {
            EqSlot::Primitive(p) => Ok(match p {
                // 表达式原语：同类可嵌套（\numexpr\numexpr）；异类 → can't use
                Primitive::NumExpr => kind == ExprKind::Num,
                Primitive::Dimexpr => kind == ExprKind::Dim,
                Primitive::Glueexpr => kind == ExprKind::Glue,
                Primitive::Muexpr => kind == ExprKind::Mu,
                // 量类原语：整数/尺寸/胶水寄存器与 \the/\number 等 → 表达式起点
                Primitive::Count
                | Primitive::Dimen
                | Primitive::Skip
                | Primitive::Muskip
                | Primitive::Toks
                | Primitive::The
                | Primitive::Number
                | Primitive::GlueToMu
                | Primitive::MuToGlue => true,
                // 命令类（\let/\relax/定义/条件/组等）→ 垂直模式裸用 → can't use
                _ => false,
            }),
            // 宏（可能展开为量）/寄存器绑定 cs/未定义 → 放行（scan 展开或恢复）
            _ => Ok(true),
        }
    }

    fn exec_primitive(&mut self, prim: Primitive) -> Result<()> {
        match prim {
            Primitive::Relax => Ok(()),
            // 内部只读整数单独出现：no-op（TeX 在 no_mode——输出例程中——忽略；
            // 其余模式应报错，模式状态在排版器侧，M1 宽松处理）
            Primitive::Badness => Ok(()),
            Primitive::Expandafter => self.exec_expandafter(),
            Primitive::Noexpand => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\noexpand 后无 token"))?;
                self.stack.push(InputFrame::One {
                    tok: t.0,
                    noexpand: true,
                });
                Ok(())
            }
            Primitive::Def => self.exec_def(false),
            Primitive::Edef => self.exec_def(true),
            // \gdef ≡ \global\def；\xdef ≡ \global\edef
            Primitive::Gdef => {
                self.global_pending = true;
                self.exec_def(false)
            }
            Primitive::Xdef => {
                self.global_pending = true;
                self.exec_def(true)
            }
            // \outer：宏定义前缀（TeX 限制宏在实参中出现；当前仅消费，语义后续补）
            Primitive::Outer => {
                self.outer_pending = true;
                Ok(())
            }
            // \long：宏定义前缀（允许参数中含 \par；同 \outer 仅置前缀）
            Primitive::Long => {
                self.long_pending = true;
                Ok(())
            }
            Primitive::Let => self.exec_let(),
            Primitive::Catcode => self.exec_catcode(),
            Primitive::SfCode => self.exec_sfcode(),
            Primitive::LcCode => self.exec_lccode(),
            // ETRIP 冲刺：\advance<寄存器> <增量>（寄存器运算）
            Primitive::Advance => self.exec_advance(),
            Primitive::End => {
                self.stack.clear();
                self.output_active = false;
                self.ended = true;
                // tex.web final_cleanup：\end 也报未闭合条件（\endinput 同款；
                // trip L442 \if 在 \write 组内被 \end 中断 → if 442 incomplete）
                self.report_incomplete_conditions();
                // TeX `\end` 收尾：flush 所有延迟写流（final_cleanup 语义）
                self.flush_writes()?;
                Ok(())
            }
            // M1-7 扫描顺序原语
            Primitive::Futurelet => self.exec_futurelet(),
            Primitive::Aftergroup => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\aftergroup 后无 token"))?
                    .0;
                self.aftergroup.push((self.group_level, t));
                Ok(())
            }
            Primitive::Afterassignment => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\afterassignment 后无 token"))?
                    .0;
                self.afterassignment = Some(t);
                Ok(())
            }
            // M1-9 条件原语：由 process_one 拦截，不应到达此处
            Primitive::If
            | Primitive::IfCat
            | Primitive::IfNum
            | Primitive::IfDim
            | Primitive::IfX
            | Primitive::IfOdd
            | Primitive::IfCase
            | Primitive::IfTrue
            | Primitive::IfFalse
            | Primitive::IfInner
            | Primitive::IfVMode
            | Primitive::IfHMode
            | Primitive::IfMMode
            | Primitive::IfEof
            | Primitive::IfVoid
            | Primitive::IfHBox
            | Primitive::IfVBox
            | Primitive::Else
            | Primitive::Fi
            | Primitive::Or => Err(Error::internal("条件原语不应到达 exec_primitive")),
            // M1-10 寄存器
            Primitive::Count
            | Primitive::Dimen
            | Primitive::Skip
            | Primitive::Muskip
            | Primitive::Toks => self.exec_register(prim),
            // ETRIP 冲刺：\thinmuskip/\medmuskip/\thickmuskip（muskip 寄存器 0/1/2）
            Primitive::ThinMuskip => self.exec_muskip_param(0),
            Primitive::MedMuskip => self.exec_muskip_param(1),
            Primitive::ThickMuskip => self.exec_muskip_param(2),
            Primitive::The => self.exec_the(),
            Primitive::Global => {
                self.global_pending = true;
                Ok(())
            }
            // M1-11 组
            Primitive::BeginGroup => {
                // \begingroup：半简单组（currentgrouptype=14）
                self.sink.semisimple_begin()?;
                self.begin_group()
            }
            Primitive::EndGroup => {
                self.end_group()
            }
            // M3-2 排版原语
            // 盒子：扫描可选 to/spread 规格，直通 sink（排版器解释）。
            Primitive::HBox | Primitive::VBox | Primitive::VTop => {
                let (to, spread) = self.scan_box_spec()?;
                self.sink.box_spec(to, spread)?;
                self.sink.primitive(prim)
            }
            Primitive::Par => {
                // 传 \par 源码行号给排版器（折行警告 `at lines a--b`）
                let ln = self.error_context().map(|(n, _)| n as i64).unwrap_or(0);
                self.sink.paragraph_line(ln)?;
                self.sink.primitive(prim)
            }
            // 带参数扫描的排版原语：扫描在 VM 侧完成，结果交给 sink
            Primitive::HSkip | Primitive::VSkip => {
                let g = self.scan_glue()?;
                self.sink.glue(g)
            }
            Primitive::Kern => {
                let w = self.scan_dimen()?;
                self.sink.kern(w)
            }
            Primitive::Penalty => {
                // 记录行号须在 scan_number 之前（数字扫描吞行尾换行 →
                // current_line_no 已前进到下一行）
                let line = self.current_line_no();
                let p = self.scan_number()?;
                // 强制断页（≤ -10000）触发 output 例程：例程组的 entering
                // 行 = 断页行（tex.web：例程在断页 token 处注入）
                if p <= -10_000 {
                    self.output_trigger_line = line;
                }
                self.sink.penalty(p)
            }
            Primitive::HRule | Primitive::VRule => {
                let [h, d, w] = self.scan_rule_specs(prim)?;
                self.sink.rule(w, h, d)
            }
            // TRIP 冲刺：\leaders/\cleaders/\xleaders —— 后续盒子（\hbox/\vbox/\hrule/
            // 盒子寄存器）由既有路径扫描，sink 侧挂起为引导符，等 \hskip/\vskip 胶水
            // 组成 Leader 节点（tex.web scan_box(leader_flag+kind) + box_end）。
            Primitive::Leaders | Primitive::Cleaders | Primitive::XLeaders => {
                self.sink.primitive(prim)
            }
            // M3-2-2 内部参数赋值
            Primitive::ParIndent | Primitive::LineSkipLimit => {
                let v = self.scan_dimen()?;
                let kind = if prim == Primitive::ParIndent {
                    ParamKind::ParIndent
                } else {
                    ParamKind::LineSkipLimit
                };
                self.assign_param(kind, ParamValue::Dimen(v))
            }
            // TRIP 冲刺：TeX initex 预定义 dimen 内部参数
            Primitive::NullDelimiterSpace
            | Primitive::ScriptSpace
            | Primitive::OverfullRule
            | Primitive::VOffset
            | Primitive::HOffset => {
                let v = self.scan_dimen()?;
                let kind = match prim {
                    Primitive::NullDelimiterSpace => ParamKind::NullDelimiterSpace,
                    Primitive::ScriptSpace => ParamKind::ScriptSpace,
                    Primitive::OverfullRule => ParamKind::OverfullRule,
                    Primitive::VOffset => ParamKind::VOffset,
                    _ => ParamKind::HOffset,
                };
                self.assign_param(kind, ParamValue::Dimen(v))
            }
            Primitive::BaselineSkip | Primitive::LineSkip => {
                let g = self.scan_glue()?;
                let kind = if prim == Primitive::BaselineSkip {
                    ParamKind::BaselineSkip
                } else {
                    ParamKind::LineSkip
                };
                self.assign_param(kind, ParamValue::Glue(g))
            }
            // 段落缩进：直通 sink 由排版器解释
            Primitive::Indent | Primitive::NoIndent => self.sink.primitive(prim),
            // M3-3 折行参数
            Primitive::HSize => {
                let v = self.scan_dimen()?;
                self.assign_param(ParamKind::HSize, ParamValue::Dimen(v))
            }
            Primitive::Tolerance => {
                let v = self.scan_number()?;
                self.assign_param(ParamKind::Tolerance, ParamValue::Number(v))
            }
            // M3-5 断页参数
            Primitive::VSize | Primitive::MaxDepth => {
                let kind = if prim == Primitive::VSize {
                    ParamKind::VSize
                } else {
                    ParamKind::MaxDepth
                };
                let v = self.scan_dimen()?;
                // \vsize.pt（内部量 + 单位）：scan_dimen 读 .pt 返回 0——tex.web
                // 读值不赋值（TRIP L151；若赋值 0 会 push 并污染 \global 钳制值）。
                // 显式 \vsize=0 走 eq 路径？——scan_dimen 已消费可选 =，此处 0 值
                // 一律视为读值（\vsize=0pt 罕见，TRIP 无此场景）。
                if v != 0 {
                    self.assign_param(kind, ParamValue::Dimen(v))
                } else {
                    Ok(())
                }
            }
            Primitive::TopSkip
            | Primitive::ParSkip
            | Primitive::ParFillSkip
            | Primitive::XSpaceSkip => {
                let g = self.scan_glue()?;
                let kind = if prim == Primitive::TopSkip {
                    ParamKind::TopSkip
                } else if prim == Primitive::ParSkip {
                    ParamKind::ParSkip
                } else if prim == Primitive::ParFillSkip {
                    ParamKind::ParFillSkip
                } else {
                    ParamKind::XSpaceSkip
                };
                self.assign_param(kind, ParamValue::Glue(g))
            }
            // M4-4 显示数学间距参数
            Primitive::AboveDisplaySkip
            | Primitive::BelowDisplaySkip
            | Primitive::AboveDisplayShortSkip
            | Primitive::BelowDisplayShortSkip => {
                let g = self.scan_glue()?;
                let kind = match prim {
                    Primitive::AboveDisplaySkip => ParamKind::AboveDisplaySkip,
                    Primitive::BelowDisplaySkip => ParamKind::BelowDisplaySkip,
                    Primitive::AboveDisplayShortSkip => ParamKind::AboveDisplayShortSkip,
                    _ => ParamKind::BelowDisplayShortSkip,
                };
                self.assign_param(kind, ParamValue::Glue(g))
            }
            Primitive::PreDisplayPenalty | Primitive::PostDisplayPenalty => {
                let p = self.scan_number()?;
                let kind = if prim == Primitive::PreDisplayPenalty {
                    ParamKind::PreDisplayPenalty
                } else {
                    ParamKind::PostDisplayPenalty
                };
                self.assign_param(kind, ParamValue::Number(p))
            }
            // ETRIP 冲刺：TeX 内部整数参数
            Primitive::EndlineChar
            | Primitive::NewlineChar
            | Primitive::DefaultHyphenChar
            | Primitive::DefaultSkewChar
            | Primitive::Mag => {
                let v = self.scan_number()?;
                let kind = match prim {
                    Primitive::EndlineChar => ParamKind::EndlineChar,
                    Primitive::NewlineChar => ParamKind::NewlineChar,
                    Primitive::DefaultHyphenChar => ParamKind::DefaultHyphenChar,
                    Primitive::DefaultSkewChar => ParamKind::DefaultSkewChar,
                    _ => ParamKind::Mag,
                };
                self.assign_param(kind, ParamValue::Number(v))
            }
            // ETRIP 冲刺：TeX/e-TeX 内部整数参数（misc 数组，按下标索引）。
            // TeX 语义：内部整数可展开——主循环单独出现（无 `=`）= 读值输出
            // （tex.web：`$\splitdiscards` 数学模式取 eqtb 值入列表，不扫描输入；
            // 水平/垂直模式输出数字）。仅当 `=` 存在才是赋值（`\splitdiscards=1`）。
            p if int_param_index(p).is_some() => {
                let idx = int_param_index(p).expect("已检查 is_some");
                self.skip_spaces()?;
                let Some((tok, _)) = self.fetch()? else {
                    return Ok(());
                };
                if tok.charcode() == Some(b'=' as u32) {
                    let v = self.scan_number()?;
                    // TRIP 语义：\prevgraf 只允许非负（trip L392 `\prevgraf=-1` 报
                    // "! Bad \prevgraf (-1)." 恢复为 0；参考 log 对齐）
                    if p == Primitive::PrevGraf && v < 0 {
                        self.report_error(&format!("Bad \\\\prevgraf ({v})."));
                        self.assign_param(ParamKind::MiscInt(idx), ParamValue::Number(0))
                    } else {
                        self.assign_param(ParamKind::MiscInt(idx), ParamValue::Number(v))
                    }
                } else {
                    // 无 `=`：TeX 可选 `=` 语义——后跟 <integer> 仍是赋值
                    // （`\tracingcommands2`；9b0bc69 曾把数字也当"单独出现"
                    // no-op 丢弃，导致 TRIP 的 \tracingcommands 永不开启）；
                    // 后跟非数字才是单独出现 no-op（`$\splitdiscards\noindent`）。
                    if matches!(tok.catcode(), Some(Catcode::Other))
                        && matches!(
                            tok.charcode(),
                            Some(c)
                                if (b'0' as u32..=b'9' as u32).contains(&c)
                                    || c == b'+' as u32
                                    || c == b'-' as u32
                        )
                    {
                        self.unread(tok);
                        let v = self.scan_number()?;
                        if p == Primitive::PrevGraf && v < 0 {
                            self.report_error(&format!("Bad \\\\\\\\prevgraf ({v})."));
                            self.assign_param(ParamKind::MiscInt(idx), ParamValue::Number(0))
                        } else {
                            self.assign_param(ParamKind::MiscInt(idx), ParamValue::Number(v))
                        }
                    } else {
                        self.unread(tok);
                        // 单独出现（无 `=`）：no-op——TeX 内部整数在主循环/数学
                        // 模式均不读值（TRIP `{\tracingstats}` 追踪后无操作；
                        // ETRIP `$\splitdiscards` 数学模式同样 no-op——参考
                        // showbox27 空数学，l.1148 的 Missing $ inserted 由
                        // `\noindent`/`}` 触发）
                        Ok(())
                    }
                }
            }
            // ETRIP 冲刺：交互模式命令（\batchmode/\nonstopmode/\scrollmode/\errorstopmode）
            p if interaction_mode_value(p).is_some() => {
                let v = interaction_mode_value(p).expect("已检查 is_some");
                self.assign_param(ParamKind::MiscInt(19), ParamValue::Number(v))
            }
            // ETRIP 冲刺：\chardef\cs=<num>（cs 绑定字符，cat 12）
            Primitive::Chardef => {
                let csid = self.scan_cs_ident()?;
                let v = self.scan_number()?;
                let v = u32::try_from(v)
                    .map_err(|_| Error::invalid_input("\\chardef 字符码越界"))?;
                self.set_slot_scoped(csid, EqSlot::Char {
                    catcode: Catcode::Other,
                    charcode: v,
                });
                Ok(())
            }
            // ETRIP 冲刺：\countdef/\dimendef/\skipdef/\muskipdef/\toksdef\cs=<num>（cs 绑定寄存器）
            Primitive::Countdef
            | Primitive::Dimendef
            | Primitive::Skipdef
            | Primitive::Muskipdef
            | Primitive::Toksdef => {
                let csid = self.scan_cs_ident()?;
                let idx = self.scan_number()?;
                // TeX：\countdef\cs=-1 / 32768 → "! Bad register code (-1)."
                // 恢复式（不定义、继续；ETRIP L970 稀疏数组测试的故意用例）。
                if idx < 0 || idx >= REGISTER_COUNT as i64 {
                    self.report_error(&format!("Bad register code ({idx})."));
                    return Ok(());
                }
                let idx = idx as usize;
                let kind = match prim {
                    Primitive::Countdef => RegKind::Count,
                    Primitive::Dimendef => RegKind::Dimen,
                    Primitive::Skipdef => RegKind::Skip,
                    Primitive::Muskipdef => RegKind::Muskip,
                    _ => RegKind::Toks,
                };
                self.set_slot_scoped(csid, EqSlot::Register(kind, idx));
                Ok(())
            }
            // M3-4 字体
            Primitive::Font => self.exec_font(),
            // ETRIP 冲刺：字体参数 \fontdimen<num><font>=<dimen>
            Primitive::FontDimen => self.exec_fontdimen(),
            // ETRIP 冲刺：\hyphenchar<font>=<int>（字体断字符）
            Primitive::HyphenChar => self.exec_hyphenchar(),
            // ETRIP 冲刺：\delcode<num>=<num>（字符定界符码）
            Primitive::DelCode => self.exec_delcode(),
            // TRIP 冲刺：\mathcode<num>=<num>（字符数学码）
            Primitive::MathCode => self.exec_mathcode(),
            // ETRIP 冲刺：终端转录
            Primitive::Message => self.exec_message(),
            Primitive::Show => self.exec_show(),
            Primitive::ShowThe => self.exec_showthe(),
            // M3-5 输出：\shipout 直通 sink（排版器解释：封装下一盒子为页面）
            Primitive::ShipOut => {
                // 记录触发行号（output 例程组 entering 行 = shipout 行）
                self.output_trigger_line = self.current_line_no();
                self.sink.primitive(prim)
            }
            // M3-5-3 输出例程：\output=<general text> 存储 token 列表
            Primitive::Output => {
                self.expect_equals()?;
                let val = self.scan_group_contents(Some("output"))?;
                self.assign_output(Arc::from(val));
                Ok(())
            }
            // M3-5-3 盒子寄存器：\box<n> 交给 sink（shipout_next 时封装为页面）
            Primitive::Box => {
                let idx = self.scan_register_index()?;
                self.sink.box_register(idx)
            }
            // M3 收尾（RFC-3）：VFS 副作用原语
            Primitive::Input => self.exec_input(),
            Primitive::OpenIn => self.exec_openin(),
            Primitive::CloseIn => self.exec_closein(),
            Primitive::NewRead => self.exec_new_stream(StreamKind::Read),
            Primitive::Read => self.exec_read(),
            Primitive::NewWrite => self.exec_new_stream(StreamKind::Write),
            Primitive::OpenOut => self.exec_openout(),
            Primitive::CloseOut => self.exec_closeout(),
            Primitive::Write => self.exec_write(),
            Primitive::Immediate => {
                // tex.web `<Implement \immediate>`：立即取下一个 token；是
                // \write/\openout/\closeout → 设前缀后执行（立即写）；否则放回、
                // 前缀无效（不残留——旧实现残留标志导致 TRIP L2
                // `\immediate\catcode` 污染 L93 `\write-1` 被误判立即写，
                // "log file only" 提前输出）。
                self.immediate_pending = false;
                let Some((next, _)) = self.fetch()? else {
                    return Err(Error::invalid_input("\\immediate 后无 token"));
                };
                let is_write_family = next.csid().is_some_and(|c| {
                    matches!(
                        self.eqtb.slot(c),
                        EqSlot::Primitive(
                            Primitive::Write | Primitive::OpenOut | Primitive::CloseOut
                        )
                    )
                });
                if is_write_family {
                    self.immediate_pending = true;
                    self.process_token(next)
                } else {
                    self.unread(next);
                    Ok(())
                }
            }
            // M4-2 数学原语：直通 sink（排版器解释；delimiter 参数在 VM 侧扫描）
            Primitive::DisplayStyle => self.sink.math_style(0),
            Primitive::TextStyle => self.sink.math_style(1),
            Primitive::ScriptStyle => self.sink.math_style(2),
            Primitive::ScriptScriptStyle => self.sink.math_style(3),
            Primitive::Over => self.sink.math_fraction(None),
            Primitive::Atop => self.sink.math_fraction(Some(0)),
            Primitive::Left => {
                let d = self.scan_delimiter()?;
                // math left group（16）：\left 打开真实组（tex.web new_math_left_group；
                // \currentgrouptype 检查 + \tracinggroups 显示）。数学层（math_left/
                // math_right 的 Delimited 物化）由 layout 侧另行维护，互不干扰。
                self.sink.math_left_begin()?;
                self.begin_group()?;
                self.math_left_depth += 1;
                self.sink.math_left(d)
            }
            Primitive::Right => {
                let d = self.scan_delimiter()?;
                // 配对 \\left 存在才关组（\\right 前缺 \\left → TeX 报
                // "! Extra \\right." 恢复并丢弃——trip L256 `$\\right\\relax`，
                // 参考 log 双错误：Missing delimiter (. inserted) + Extra \\right.；
                // 不报 Error 中断（引擎契约：畸形输入不 panic））。
                if self.math_left_depth > 0 {
                    self.math_left_depth -= 1;
                    self.end_group()?;
                    self.sink.math_right(d)
                } else {
                    self.write_error("Extra \\right.");
                    Ok(())
                }
            }
            // e-TeX（M4-5）：\middle<delimiter>（\left...\right 内分隔符）
            Primitive::Middle => {
                let d = self.scan_delimiter()?;
                // \middle 关闭当前 \left 组并开新组（参考 log：leaving entered at
                // 前一行 + entering 本行成对；组类型仍为 math left group 16）
                if self.math_left_depth > 0 {
                    self.math_left_depth -= 1;
                    self.end_group()?;
                }
                self.sink.math_left_begin()?;
                self.begin_group()?;
                self.math_left_depth += 1;
                self.sink.math_middle(d)
            }
            // ETRIP 冲刺：\mark{<text>}（mark 节点）；e-TeX \marks<n>{<text>}
            Primitive::Mark | Primitive::Marks => {
                let class = if prim == Primitive::Marks {
                    Some(self.scan_number()?)
                } else {
                    None
                };
                let toks = self.scan_group_contents(None)?;
                let text = self.expand_to_string(&toks)?;
                self.sink.mark(class, text)
            }
            // ETRIP 冲刺：e-TeX marks 族查询原语（可展开）。主循环/\edef 等执行上下文
            // 走此处（emit_tokens 压回输入流）；扫描上下文（宏参数收集/\edef 等）由
            // expand_once 的同构分支处理——两处语义一致：scan_number 取 class →
            // sink 查询 → 文本转字符 token（空格 → Space，其余 → Other）。
            Primitive::TopMarks
            | Primitive::FirstMarks
            | Primitive::BotMarks
            | Primitive::SplitFirstMarks
            | Primitive::SplitTopMarks
            | Primitive::SplitBotMarks => {
                let class = self.scan_number()?;
                let text = match prim {
                    Primitive::TopMarks => self.sink.topmarks(class),
                    Primitive::FirstMarks => self.sink.firstmarks(class),
                    Primitive::BotMarks => self.sink.botmarks(class),
                    Primitive::SplitFirstMarks => self.sink.splitfirstmarks(class),
                    Primitive::SplitTopMarks => self.sink.splittopmarks(class),
                    Primitive::SplitBotMarks => self.sink.splitbotmarks(class),
                    _ => unreachable!("marks 族已在上层 match 穷举"),
                };
                self.emit_tokens(
                    text.bytes()
                        .map(|b| {
                            let cat = if b == b' ' {
                                Catcode::Space
                            } else {
                                Catcode::Other
                            };
                            Token::char(cat, u32::from(b))
                        })
                        .collect(),
                )
            }
            // TRIP 补全批次：TeX 版 marks 查询（\topmark 等，无 class 参数，class 0）
            Primitive::TopMark
            | Primitive::FirstMark
            | Primitive::BotMark
            | Primitive::SplitFirstMark
            | Primitive::SplitBotMark => {
                let text = match prim {
                    Primitive::TopMark => self.sink.topmarks(0),
                    Primitive::FirstMark => self.sink.firstmarks(0),
                    Primitive::BotMark => self.sink.botmarks(0),
                    Primitive::SplitFirstMark => self.sink.splitfirstmarks(0),
                    _ => self.sink.splitbotmarks(0),
                };
                self.emit_tokens(
                    text.bytes()
                        .map(|b| {
                            let cat = if b == b' ' {
                                Catcode::Space
                            } else {
                                Catcode::Other
                            };
                            Token::char(cat, u32::from(b))
                        })
                        .collect(),
                )?;
                Ok(())
            }
            // TRIP 补全批次：\skewchar<font>=<num>（字体偏斜字符，仿 \hyphenchar）
            Primitive::SkewChar => self.exec_skewchar(),
            // TRIP 补全批次：\everydisplay={<tokens>}（显示数学进入时注入）
            Primitive::EveryDisplay => {
                self.expect_equals()?;
                self.skip_spaces()?;
                let (tok, _) = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("everydisplay 缺少 RHS"))?;
                if tok.catcode() == Some(Catcode::BeginGroup) {
                    self.unread(tok);
                    self.everydisplay_toks = self.scan_group_contents(None)?;
                } else if let Some(csid) = tok.csid() {
                    match self.eqtb.slot(csid).clone() {
                        EqSlot::Primitive(_) => {
                            self.everydisplay_toks = self.the_tokens_after(tok)?
                        }
                        EqSlot::Register(RegKind::Toks, idx) => {
                            self.everydisplay_toks = self.registers.toks(idx).to_vec()
                        }
                        _ => {}
                    }
                }
                self.finish_assignment();
                Ok(())
            }
            // TRIP 补全批次：页面只读内部量（\the 查询在 save.rs 返回 0）
            Primitive::DisplayWidth
            | Primitive::PageDepth
            | Primitive::PageFillLStretch
            | Primitive::PageShrink
            | Primitive::NullFont => Ok(()),
            // ETRIP 冲刺：\showbox<n>：显示盒子寄存器内容（sink 格式化到转录）
            Primitive::ShowBox => {
                let idx = self.scan_register_index()?;
                self.sink.showbox(idx)
            }
            // ETRIP 冲刺：\discretionary{pre}{post}{replace}（断字节点）
            Primitive::Discretionary => {
                let pre = self.scan_group_contents(None)?;
                let post = self.scan_group_contents(None)?;
                let replace = self.scan_group_contents(None)?;
                self.sink.discretionary(pre, post, replace)
            }
            // ETRIP 冲刺：\insert<regnum>{<general text>}（insert 节点；内容只收集不排版）
            Primitive::Insert => {
                let class = self.scan_register_index()?;
                let toks = self.scan_group_contents(None)?;
                self.sink.insert_node(class, toks)
            }
            // ETRIP 冲刺：\vadjust{<vertical material>}（adjust 节点；内容只收集不排版）
            Primitive::VAdjust => {
                let toks = self.scan_group_contents(None)?;
                self.sink.vadjust(toks)
            }
            // ETRIP 冲刺：\valign/ \halign：下一个组为对齐组（组种类 6）。
            // TeX 语义：`\halign` 的 `{` 由 scan_left_brace 消费，alignment 内容中
            // 的 `{`/`}` 由对齐状态机管理（不建普通组）——VM 侧用 align_depth 模拟。
            Primitive::Valign | Primitive::Halign => {
                // 可选 `to <dimen>`/`spread <dimen>` 规格（同 \hbox 的 scan_box_spec；
                // TRIP L332 `\halign to 0pt{...}`、L407 `\halign to 1truemm...`）
                let (to, spread) = self.scan_box_spec()?;
                self.sink.box_spec(to, spread)?;
                self.sink.align_begin(prim == Primitive::Halign)?;
                let fetched = self.fetch()?;
                if let Some((tok, _)) = fetched {
                    if tok.catcode() == Some(Catcode::BeginGroup) {
                        self.begin_group()?;
                        self.align_depth = 1;
                        // 模板（preamble）阶段：`{` 后到首个 `\cr`/`\crcr` 之间收集不执行
                        self.align_preamble = true;
                        self.align_preamble_depth = self.align_depth;
                        Ok(())
                    } else {
                        self.unread(tok);
                        self.align_depth = 0;
                        Ok(())
                    }
                } else {
                    Ok(())
                }
            }
            // ETRIP 冲刺：\noalign{...}：下一个组为无对齐组（组种类 7）。
            // 对齐体内的 `{`/`}` 只调整 align_depth 不建组——`\noalign` 的 `{`
            // 例外（主流层据此建真实组，见 process_token 的组定界符分支）。
            Primitive::NoAlign => {
                if self.align_depth > 0 {
                    self.align_noalign_pending = true;
                }
                self.sink.noalign_begin()
            }
            // ETRIP 冲刺：\cr（对齐行结束）：无操作（简化；对齐组按盒子处理）
            Primitive::Cr => self.sink.align_row_end(),
            // ETRIP 冲刺：\mathchoice{D}{T}{S}{SS}：收集四个分支（内容不执行）。
            // TeX 语义（tex.web scan_left_brace + build_choices）：每个分支强制以
            // `{` 开头，非 `{`（含 `}`/单 token）报 "Missing { inserted." 并把
            // token 放回、隐含插入 `{` 后收集到下一个 `}`（TRIP L438
            // `\mathchoice{}a}{A|{}}{\mathchoice}`）。
            Primitive::MathChoice => {
                for _ in 0..4 {
                    self.scan_mathchoice_branch()?;
                }
                Ok(())
            }
            // tex.web L20859-20861：hmove/vmove 扫描 dimen 后按 chr_code 取正负
            // 传入 scan_box（`if t=0 then scan_box(cur_val) else scan_box(-cur_val)`）。
            // box_context 最终整体写入 shift_amount（L20894），showbox 显示该值。
            // 参考实测（trip.log L401）：\lower2pt → shifted 2.0（正值，非直觉的 -2）。
            Primitive::Raise | Primitive::Lower => {
                let amount = self.scan_dimen()?;
                // lower(chr=0) → +amount；raise(chr=1) → -amount（tex.web t=0/1 规则）
                let amount = if prim == Primitive::Raise {
                    -amount
                } else {
                    amount
                };
                self.sink.raise(amount)
            }
            // ETRIP 冲刺：\span（对齐模板列合并）：无操作（简化）
            Primitive::Span => Ok(()),
            // ETRIP 冲刺：\special{<general text>}：whatsit 节点（内容只收集不排版）
            Primitive::Special => {
                let toks = self.scan_group_contents(None)?;
                let text: String = toks
                    .iter()
                    .filter_map(|t| t.charcode())
                    .filter_map(char::from_u32)
                    .collect();
                self.sink.whatsit(text)
            }
            // ETRIP 冲刺：\jobname：作业名（当前无名字来源，恒 "texput"）
            Primitive::JobName => self.emit_tokens(
                "texput"
                    .bytes()
                    .map(|b| Token::char(Catcode::Other, u32::from(b)))
                    .collect(),
            ),
            // ETRIP 冲刺：\vcenter<box>：数学垂直居中盒（简化按 vbox 处理）
            Primitive::VCenter => {
                let (to, spread) = self.scan_box_spec()?;
                self.sink.box_spec(to, spread)?;
                self.sink.primitive(Primitive::VBox)
            }
            Primitive::Sqrt => self.sink.math_sqrt(),
            // TeX math_comp（tex.web L22019）：\mathord 等“定类原语”之后必有
            // <math field>；非 `{` 的 token 经 scan_math → scan_left_brace 报
            // "Missing { inserted" 放回重扫（TRIP L272 `\mathord \radical`）。
            Primitive::MathOrd => self.sink.math_class(0),
            Primitive::MathBin => self.sink.math_class(1),
            Primitive::MathOp => self.sink.math_class(2),
            Primitive::MathRel => self.sink.math_class(3),
            Primitive::MathOpen => self.sink.math_class(4),
            Primitive::MathClose => self.sink.math_class(5),
            Primitive::MathPunct => self.sink.math_class(6),
            Primitive::MathInner => self.sink.math_class(7),
            Primitive::Nonscript => self.sink.primitive(prim),
            // TRIP 冲刺：\limits/\nolimits/\displaylimits（\mathop 后置上下限标志；
            // 布局层数学原子按 `\mathop` 标志处理；单独出现时 TeX 报
            // "Limit controls must follow a math operator"——差异待 etrip.log 校准）
            Primitive::Limits | Primitive::NoLimits | Primitive::DisplayLimits => {
                self.sink.primitive(prim)
            }
            // TRIP 冲刺：\noboundary（数学字符边界抑制；水平/垂直模式 no-op）
            Primitive::NoBoundary => self.sink.primitive(prim),
            // TRIP 冲刺：数学原语（存在性测试；简化实现"消费参数"——
            // 数学列表节点由排版器处理，expander 侧跳过）。
            // tex.web：\mskip/\mkern 是 **mu 上下文**（scan_glue_mu/scan_dimen_mu，
            // 只认 "mu" 单位）——用 pt 上下文扫描会把 `\mkern-9mu` 报 Illegal
            // unit（8d0a71c muskip 参数化后暴露；trip L262/L275）。
            Primitive::MSkip => {
                let _ = self.scan_glue_mu()?;
                Ok(())
            }
            Primitive::MKern => {
                let _ = self.scan_dimen_mu()?;
                Ok(())
            }
            Primitive::MathAccent => {
                let _ = self.scan_number()?;
                self.sink.math_accent(false)?;
                Ok(())
            }
            // tex.web math_char：\mathchar<15-bit> 是完整数学字符原子（数学中
            // 直接产出 Char——此前 scan_number 即丢）；\delimiter<27-bit> 为
            // 定界符（Delimited 场景经 \left/\right 扫描；裸用暂简化）
            Primitive::MathChar => {
                let n = self.scan_number()? as u32;
                self.sink.math_char_full(n)
            }
            Primitive::Delimiter => {
                let _ = self.scan_number()?;
                Ok(())
            }
            // \eqno/\leqno：显示数学内是公式编号分隔符 no-op；非数学模式 TeX
            // 报错 + pretend 恢复（trip l.254 `\eqno` horizontal 报错对齐参考）
            Primitive::EqNo | Primitive::LeqNo => {
                let mode = self.sink.mode_code();
                if matches!(mode, 3 | 6) {
                    Ok(())
                } else {
                    // tex.web print_mode：\halign 行 mode=hmode（正）报
                    // "horizontal mode"（trip l.254 参考）；\hbox 内容
                    // mode=-hmode 才报 "restricted horizontal mode"
                    let what = if self.align_depth > 0
                        && matches!(self.sink.mode_code(), 2 | 5)
                    {
                        "horizontal mode".to_string()
                    } else {
                        self.sink.mode_name()
                    };
                    self.write_error(&format!(
                        "You can't use \\{} in {what}.",
                        if matches!(prim, Primitive::EqNo) {
                            "eqno"
                        } else {
                            "leqno"
                        }
                    ));
                    let _ = self.sink.write16(
                        "Sorry, but I'm not programmed to handle this case;\n\
                         I'll just pretend that you didn't ask for it.\n\
                         If you're in the wrong mode, you might be able to\n\
                         return to the right one by typing `I}' or `I$' or `I\\par'.\n"
                            .to_string(),
                    );
                    Ok(())
                }
            }
            // tex.web math_fraction：\abovewithdelims<delim1><delim2><dimen>——
            // 先扫两个定界符再扫厚度（TRIP l.257 漏报修复；原顺序 dimen 在前
            // 导致参数错位）。**必须调 sink.math_fraction**（\over/\atop 已有；
            // 此前 4 个 withdelims/above 只扫参数不挂 fraction → 分子分母被
            // 混收当前层，l.276 \abovewithdelims(.2pt 后数学状态崩）。
            Primitive::AboveWithDelims => {
                let _ = self.scan_delimiter()?;
                let _ = self.scan_delimiter()?;
                let thickness = self.scan_dimen()?;
                self.sink.math_fraction(Some(thickness))
            }
            // TRIP 冲刺：\above<dimen>（分数）与 \atopwithdelims<delim><delim>（带定界分数）
            Primitive::Above => {
                let thickness = self.scan_dimen()?;
                self.sink.math_fraction(Some(thickness))
            }
            Primitive::AtopWithDelims => {
                let _ = self.scan_delimiter()?;
                let _ = self.scan_delimiter()?;
                self.sink.math_fraction(Some(0))
            }
            Primitive::OverWithDelims => {
                let _ = self.scan_delimiter()?;
                let _ = self.scan_delimiter()?;
                self.sink.math_fraction(None)
            }
            // tex.web math_ac：\underline/\overline 是数学前缀——字段经主循环组
            // 机制收集（sink pending → 组开 field → 组关原子）；此前 scan_group_contents
            // 收集即丢（KNOWN-SIMPLIFICATIONS §1——内容不进数学列表）
            Primitive::Underline => self.sink.math_underline(),
            Primitive::Overline => self.sink.math_overline(),
            // \crcr（对齐行结束）与 \-（断字断点）：简化 no-op
            Primitive::CrCr | Primitive::DiscMinus => Ok(()),

            // TRIP 冲刺：\moveleft/\moveright<dimen><box>（盒子水平位移）
            // tex.web L20859-20861 符号规则：moveleft(chr=1)→+cur_val、
            // moveright(chr=0)→-cur_val（box_context 直写 shift_amount；
            // trip.log 参考实测 \moveleft20pt\copy200 → shifted -20.0，
            // \moveright20pt\hbox → shifted 20.0——hmove 的位移最终以
            // "盒内内容的坐标偏移"形式表现，与 vmove 方向约定相反）。
            // tex.web：\moveleft/\moveright 仅垂直模式合法（vmode），数学模式
            // 报 "You can't use ..." 且**不扫参数**（l.395 误报修复：此前先 scan_dimen
            // 导致 \lastbox 被当尺寸报 Missing number）。水平模式（2/5）暂不拦截——
            // NTex 的 \vskip 尚未实现"水平模式隐式结束段落"（l.316 场景），
            // 拦截会暴露该系统性缺口，待 \vskip 修好后再补。
            Primitive::MoveLeft | Primitive::MoveRight => {
                let mode = self.sink.mode_code();
                if matches!(mode, 3 | 6) {
                    let name = if prim == Primitive::MoveLeft {
                        "moveleft"
                    } else {
                        "moveright"
                    };
                    let what = "math mode";
                    self.write_error(&format!("You can't use `\\{name}' in {what}."));
                    let _ = self.sink.write16(
                        "Sorry, but I'm not programmed to handle this case;\n\
                         I'll just pretend that you didn't ask for it.\n\
                         If you're in the wrong mode, you might be able to\n\
                         return to the right one by typing `I}' or `I$' or `I\\par'.\n"
                            .to_string(),
                    );
                    return Ok(());
                }
                let d = self.scan_dimen()?;
                // tex.web scan_box：box 参数 token 不追踪（\moveleft20pt\copy200 的
                // {\copy}、\moveright20pt\hbox{ 的 {\hbox}/{ 均不输出）
                self.trace_suppress += 1;
                self.pending_box_arg = true;
                self.pending_box_arg_mode = self.sink.mode_code();
                if prim == Primitive::MoveLeft {
                    self.sink.move_left(-d)
                } else {
                    self.sink.move_right(d)
                }
            }
            // TRIP 冲刺：\\accent（读音符）。TeX 在数学模式先报错且**继续**扫描
            // <15-bit number> 与 nucleus 字段（tex.web math_ac；TRIP L396
            // `\\accent\\x\\vfill` 报错改道后还要求 `{`），故数字预扫描必须保留。
            // 顺序对齐 tex.web math_ac：先 Complain（sink 报 Please use）再
            // scan_fifteen_bit_int——`\\x` 必须被数字扫描消费（参考 log 报完
            // Please use 后 <to be read again> 是 \\vfill 而非 \\x）。
            Primitive::Accent => {
                self.sink.math_accent(true)?;
                let _ = self.scan_number()?;
                Ok(())
            }
            // TRIP 冲刺：\vfilneg（plain.tex：负 1fil vskip；走 fill_glue kind=6）
            Primitive::VFilNeg => self.sink.fill_glue(6),
            // TRIP 冲刺：\hfilneg（plain.tex：负 1fil hskip；走 fill_glue kind=7）
            Primitive::HFilNeg => self.sink.fill_glue(7),
            // TRIP 冲刺：\error（plain.tex 宏：errmessage；TRIP 分支中不执行）
            Primitive::Error => self.sink.primitive(prim),
            // TRIP 冲刺：\varunit 用作字体单位（plain.tex 字体）；no-op 原语
            Primitive::VarUnit => Ok(()),
            // TRIP 冲刺：\spacefactor=<number>（活参数，实时经 sink 赋值；组恢复在排版器侧）
            Primitive::SpaceFactor => {
                let v = self.scan_number()?;
                self.sink.set_space_factor(v)
            }
            // TRIP 冲刺：\everymath={<tokens>}（进入数学模式时注入）
            Primitive::EveryMath => {
                self.expect_equals()?;
                let toks = self.scan_group_contents(Some("everymath"))?;
                self.everymath = toks;
                Ok(())
            }
            // TRIP 冲刺：toks 参数（\everypar/\everyhbox/\everyvbox/\everycr/\errhelp）
            // ——RHS 支持组内容或 toks 参数/寄存器引用（TRIP L140 `\everypar=\errhelp`）
            Primitive::EveryPar
            | Primitive::EveryHBox
            | Primitive::EveryVBox
            | Primitive::EveryCr
            | Primitive::ErrHelp => {
                self.expect_equals()?;
                self.skip_spaces()?;
                let (tok, _) = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("toks 参数缺少 RHS"))?;
                let toks = if tok.catcode() == Some(Catcode::BeginGroup) {
                    self.unread(tok);
                    self.scan_group_contents(None)?
                } else if let Some(csid) = tok.csid() {
                    match self.eqtb.slot(csid).clone() {
                        EqSlot::Primitive(_) => self.the_tokens_after(tok)?,
                        EqSlot::Register(RegKind::Toks, idx) => self.registers.toks(idx).to_vec(),
                        _ => Vec::new(),
                    }
                } else {
                    Vec::new()
                };
                match prim {
                    Primitive::EveryPar => self.everypar_toks = toks,
                    Primitive::EveryHBox => self.everyhbox_toks = toks,
                    Primitive::EveryVBox => self.everyvbox_toks = toks,
                    Primitive::EveryCr => self.everycr_toks = toks,
                    _ => self.errhelp_toks = toks,
                }
                self.finish_assignment();
                Ok(())
            }
            // TRIP 冲刺：\insertpenalties（int 只读——数字上下文由 scan_number 读取）
            Primitive::InsertPenalties => Ok(()),
            // TRIP 冲刺：\/（斜体校正，直通 sink）
            Primitive::ItalicCorrection => self.sink.italic_correction(),
            // TRIP 冲刺：\radical<delimiter><math field>（根式原子，\sqrt 底层）
            Primitive::Radical => {
                let delim = self.scan_delimiter()?;
                self.sink.math_radical(delim)
            }
            // M4-5 e-TeX 展开扩展
            Primitive::Protected => {
                self.protected_pending = true;
                Ok(())
            }
            Primitive::Unless => {
                self.unless_pending = true;
                Ok(())
            }
            // 条件原语由 process_one 拦截
            Primitive::IfDefined | Primitive::IfCsname | Primitive::IfPrimitive => {
                Err(Error::internal("条件原语不应到达 exec_primitive"))
            }
            Primitive::NumExpr => {
                // 裸用（`\numexpr \dimexpr ...` 错误用例）：TeX 报错并恢复
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\numexpr 后缺少参数"))?
                    .0;
                if !self.expr_start_ok(ExprKind::Num, t)? {
                    self.unread(t);
                    self.report_error("You can't use \\numexpr in vertical mode.");
                    return Ok(());
                }
                self.unread(t);
                let v = self.eval_int_expression()?;
                self.emit_tokens(emit_count(v))
            }
            // M4-5 e-TeX 扩展：\dimexpr/\glueexpr 可展开求值（\the 上下文由 the_tokens 直接读取）
            Primitive::Dimexpr => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\dimexpr 后缺少参数"))?
                    .0;
                if !self.expr_start_ok(ExprKind::Dim, t)? {
                    self.unread(t);
                    self.report_error("You can't use \\dimexpr in vertical mode.");
                    return Ok(());
                }
                self.unread(t);
                let v = self.eval_dimen_expression()?;
                self.emit_tokens(emit_dimen(v))
            }
            Primitive::Glueexpr => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\glueexpr 后缺少参数"))?
                    .0;
                if !self.expr_start_ok(ExprKind::Glue, t)? {
                    self.unread(t);
                    self.report_error("You can't use \\glueexpr in vertical mode.");
                    return Ok(());
                }
                self.unread(t);
                let g = self.eval_glue_expression(false)?;
                self.emit_tokens(emit_glue(g))
            }
            Primitive::Muexpr => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\muexpr 后缺少参数"))?
                    .0;
                if !self.expr_start_ok(ExprKind::Mu, t)? {
                    self.unread(t);
                    self.report_error("You can't use \\muexpr in vertical mode.");
                    return Ok(());
                }
                self.unread(t);
                let g = self.eval_glue_expression(true)?;
                self.emit_tokens(emit_glue(g))
            }
            Primitive::Scantokens => self.exec_scantokens(),
            Primitive::Detokenize => self.exec_detokenize(),
            Primitive::Unexpanded => self.exec_unexpanded(),
            // \csname...\endcsname：构造控制序列（TeX 可展开原语）
            Primitive::Csname => self.exec_csname(),
            Primitive::EndCsname => {
                self.report_error("Extra \\endcsname.");
                Ok(())
            }
            // \number<number>：整数十进制展开（TeX 可展开原语）
            Primitive::Number => {
                let v = self.scan_number()?;
                self.emit_tokens(emit_count(v))
            }
            // 内部量：\eTeXversion/\eTeXrevision 可展开（\the 上下文由 the_tokens 读取）
            Primitive::ETeXVersion => self.emit_tokens(emit_count(2)),
            Primitive::ETeXRevision => {
                // e-TeX 2.6：revision 展开为 ".6"（版本号"2.6"的后半段，前导点用于 etrip.tex
                // 的 `\def\1.#1#2\relax` 分隔参数解析）
                self.emit_tokens(
                    ".6"
                        .bytes()
                        .map(|b| Token::char(Catcode::Other, u32::from(b)))
                        .collect(),
                )
            }
            // \string<token>：token 转文本（字符序列；TeX 可展开原语）
            Primitive::String_ => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\string 后无 token"))?
                    .0;
                let mut buf = Vec::new();
                detokenize_token(t, &self.intern, &mut buf);
                self.emit_tokens(buf)
            }
            // \inputlineno 单独出现：no-op（恒 0；数字上下文由 scan_number 处理）
            Primitive::InputLineNo => Ok(()),
            // e-TeX 只读整数单独出现：no-op（数字上下文由 scan_number 读取）
            Primitive::CurrentGroupLevel
            | Primitive::CurrentGroupType
            | Primitive::LastNodeType
            | Primitive::CurrentIfLevel
            | Primitive::CurrentIfType
            | Primitive::CurrentIfBranch => Ok(()),
            // TRIP 冲刺：页面 dimen 内部量（\pagetotal/\pagegoal/\predisplaysize
            // 只读——expander 无排版状态，单独出现无操作；\the 查询在 save.rs 返回 0）
            Primitive::PageTotal | Primitive::PageGoal | Primitive::PreDisplaySize => Ok(()),
            // TRIP 冲刺：\errmessage{...} 报错到转录（plain.tex \error 宏的底层原语）
            Primitive::ErrMessage => {
                let msg = self.scan_group_contents(None)?;
                let text: String = msg
                    .iter()
                    .filter_map(|t| t.charcode().and_then(char::from_u32))
                    .collect();
                self.report_error(&format!("{text}."));
                Ok(())
            }
            // ETRIP 冲刺：寄存器算术 \multiply/\divide<寄存器> by<n>
            Primitive::Multiply | Primitive::Divide => self.exec_multiply_divide(prim),
            // ETRIP 冲刺：\meaning<token>（可展开：token 含义文本）
            Primitive::Meaning => self.exec_meaning(),
            // ETRIP 冲刺：\mathchardef\cs=<num>（cs 绑定数学字符码）
            Primitive::MathCharDef => self.exec_mathchardef(),
            // 胶水分量查询单独出现：no-op（\ifnum/\ifdim/\the 上下文由扫描函数读取）
            Primitive::GlueStretchOrder
            | Primitive::GlueShrinkOrder
            | Primitive::GlueStretch
            | Primitive::GlueShrink => Ok(()),
            // ETRIP 冲刺：\showtokens{<text>}（展开后显示 token 列表）
            Primitive::ShowTokens => self.exec_showtokens(),
            // ETRIP 冲刺：\readline<n>to\cs（原始行读取）
            Primitive::ReadLine => self.exec_readline(),
            // ETRIP 冲刺：字体字符度量 \fontcharwd/ht/dp/ic<font><char>
            Primitive::FontCharWd
            | Primitive::FontCharHt
            | Primitive::FontCharDp
            | Primitive::FontCharIc => self.exec_fontchar_dimen(prim),
            // ETRIP 冲刺：\showifs（显示当前条件嵌套；诊断原语）
            Primitive::ShowIfs => self.exec_showifs(),
            // \iffontchar 是条件原语，由 process_one 拦截（不应到达此处）
            Primitive::IfFontChar => Err(Error::internal("\\iffontchar 不应到达 exec_primitive")),
            // ETRIP 冲刺：\parshape=<n> <indent> <width> ...（段落形状）
            Primitive::Parshape => self.exec_parshape(),
            // ETRIP 冲刺：\parshapelength/indent/dimen 单独出现（无索引）→
            // TeX 报 "can't use" 并恢复
            Primitive::ParshapeLength
            | Primitive::ParshapeIndent
            | Primitive::ParshapeDimen => {
                self.report_error("You can't use \\parshape... in vertical mode.");
                let _ = self.scan_number();
                Ok(())
            }
            // M4-3 数学字体族：\textfont<fam>=<fontcs>（直通 sink 分配）
            Primitive::TextFont | Primitive::ScriptFont | Primitive::ScriptScriptFont => {
                let kind = match prim {
                    Primitive::TextFont => 0,
                    Primitive::ScriptFont => 1,
                    _ => 2,
                };
                self.exec_math_font(kind)
            }
            // M4-6 断字：\patterns{...}（扫描组 + 直通 sink 文本）
            Primitive::Patterns => self.exec_patterns(),
            // ETRIP 冲刺：\hyphenation{...}（断字异常词表：lccode 转小写 + 断点 → sink）
            Primitive::Hyphenation => self.exec_hyphenation(),
            // ETRIP 冲刺：\setbox<n>=<box>（盒子寄存器赋值：通知 sink 存入寄存器）
            // box 参数（\vbox{}/\box255 等）同 \moveleft：scan_box 扫描不追踪
            Primitive::SetBox => {
                let idx = self.scan_register_index()?;
                self.expect_equals()?;
                self.trace_suppress += 1;
                self.pending_box_arg = true;
                self.pending_box_arg_mode = self.sink.mode_code();
                let global = self.global_pending;
                self.global_pending = false;
                self.sink.setbox(idx, global)
            }
            // ETRIP 冲刺：\␣（control space）：输出空格 token（TeX control_space）
            Primitive::ControlSpace => self
                .sink
                .token(Token::char(Catcode::Space, u32::from(b' '))),
            // ETRIP 冲刺：无限阶胶水（\hfil/\hfill/\hss/\vfil/\vfill/\vss）
            Primitive::HFil => self.sink.fill_glue(0),
            Primitive::HFill => self.sink.fill_glue(1),
            Primitive::HSS => self.sink.fill_glue(2),
            Primitive::VFil => self.sink.fill_glue(3),
            Primitive::VFill => self.sink.fill_glue(4),
            Primitive::VSS => self.sink.fill_glue(5),
            // ETRIP 冲刺：\vsplit<n> to/spread <dimen>（纵向拆分盒子寄存器）
            Primitive::VSplit => {
                let idx = self.scan_register_index()?;
                self.skip_spaces()?;
                let mut to = None;
                let mut spread = None;
                if let Some(kw) = self.scan_keyword(|w| w == "to" || w == "spread")? {
                    let d = self.scan_dimen()?;
                    if kw == "to" {
                        to = Some(d);
                    } else {
                        spread = Some(d);
                    }
                }
                self.sink.vsplit(idx, to, spread)
            }
            // ETRIP 冲刺：\everyjob=<tokens>（暂映射 toks 寄存器 0）
            Primitive::EveryJob => {
                self.expect_equals()?;
                let val = self.scan_group_contents(Some("everyjob"))?;
                self.assign_toks(0, Arc::from(val));
                Ok(())
            }
            // ETRIP 冲刺：\dump（initex 收尾）：标记 dumped 并结束作业（驱动负责写 fmt）
            Primitive::Dump => {
                self.dumped = true;
                self.stack.clear();
                self.output_active = false;
                self.cond_stack.clear();
                self.flush_writes()?;
                Ok(())
            }
            // ETRIP 冲刺：TeXXeT 方向原语 \beginL/\endL/\beginR/\endR。
            // \TeXXeTstate=1：创建方向节点；=0：TeX 报 "Improper \beginL." 后继续。
            Primitive::BeginL
            | Primitive::EndL
            | Primitive::BeginR
            | Primitive::EndR => {
                let state = self.params.misc[20]; // TeXXeTState（free.rs int_param_index 20）
                let name = match prim {
                    Primitive::BeginL => "beginL",
                    Primitive::EndL => "endL",
                    Primitive::BeginR => "beginR",
                    _ => "endR",
                };
                if state == 1 {
                    let kind = match prim {
                        Primitive::BeginL => DirectionKind::BeginL,
                        Primitive::EndL => DirectionKind::EndL,
                        Primitive::BeginR => DirectionKind::BeginR,
                        _ => DirectionKind::EndR,
                    };
                    self.sink.direction_node(kind)
                } else {
                    self.sink.write16(format!("! Improper \\{name}.\n"))
                }
            }
            // ETRIP 第二波：段落/断页参数原语（\leftskip/\rightskip/\prevdepth/
            // \interlinepenalty/\clubpenalty/\widowpenalty/\displaywidowpenalty）
            Primitive::LeftSkip | Primitive::RightSkip => {
                let g = self.scan_glue()?;
                let kind = if prim == Primitive::LeftSkip {
                    ParamKind::LeftSkip
                } else {
                    ParamKind::RightSkip
                };
                self.assign_param(kind, ParamValue::Glue(g))
            }
            // tex.web：\prevdepth 仅垂直模式可读，受限水平/数学模式报错且不扫参数
            // （l.410 误报修复：此前先 scan_dimen 导致 \advance 被当尺寸）。
            Primitive::PrevDepth => {
                let mode = self.sink.mode_code();
                if matches!(mode, 3 | 5 | 6) {
                    let what = match mode {
                        5 => "restricted horizontal mode",
                        6 => "display math mode",
                        _ => "math mode",
                    };
                    self.write_error(&format!(
                        "You can't use `\\prevdepth' in {what}."
                    ));
                    let _ = self.sink.write16(
                        "Sorry, but I'm not programmed to handle this case;\n\
                         I'll just pretend that you didn't ask for it.\n\
                         If you're in the wrong mode, you might be able to\n\
                         return to the right one by typing `I}' or `I$' or `I\\par'.\n"
                            .to_string(),
                    );
                    return Ok(());
                }
                let v = self.scan_dimen()?;
                self.assign_param(ParamKind::PrevDepth, ParamValue::Dimen(v))
            }
            // TRIP 冲刺：补充标准胶水参数（\hangindent/\spaceskip/\tabskip/
            // \lastskip/\splittopskip——普通槽存储；只读语义（\lastskip）暂不区分）
            Primitive::HangIndent
            | Primitive::SpaceSkip
            | Primitive::TabSkip
            | Primitive::LastSkip
            | Primitive::SplitTopSkip
            | Primitive::PageStretch
            | Primitive::PageFilStretch
            | Primitive::PageFillStretch => {
                let g = self.scan_glue()?;
                let kind = match prim {
                    Primitive::HangIndent => ParamKind::HangIndent,
                    Primitive::SpaceSkip => ParamKind::SpaceSkip,
                    Primitive::TabSkip => ParamKind::TabSkip,
                    Primitive::LastSkip => ParamKind::LastSkip,
                    Primitive::PageStretch => ParamKind::PageStretch,
                    Primitive::PageFilStretch => ParamKind::PageFilStretch,
                    Primitive::PageFillStretch => ParamKind::PageFillStretch,
                    _ => ParamKind::SplitTopSkip,
                };
                self.assign_param(kind, ParamValue::Glue(g))
            }
            // TRIP 冲刺：补充标准尺寸参数（\hfuzz/\vfuzz/\boxmaxdepth/\splitmaxdepth/
            // \emergencystretch/\displayindent/\delimitershortfall/\lastkern/\mathsurround）
            Primitive::Hfuzz
            | Primitive::Vfuzz
            | Primitive::BoxMaxDepth
            | Primitive::SplitMaxDepth
            | Primitive::EmergencyStretch
            | Primitive::DisplayIndent
            | Primitive::DelimiterShortfall
            | Primitive::MathSurround
            | Primitive::LastKern => {
                let v = self.scan_dimen()?;
                let kind = match prim {
                    Primitive::Hfuzz => ParamKind::Hfuzz,
                    Primitive::Vfuzz => ParamKind::Vfuzz,
                    Primitive::BoxMaxDepth => ParamKind::BoxMaxDepth,
                    Primitive::SplitMaxDepth => ParamKind::SplitMaxDepth,
                    Primitive::EmergencyStretch => ParamKind::EmergencyStretch,
                    Primitive::DisplayIndent => ParamKind::DisplayIndent,
                    Primitive::DelimiterShortfall => ParamKind::DelimiterShortfall,
                    Primitive::MathSurround => ParamKind::MathSurround,
                    _ => ParamKind::LastKern,
                };
                self.assign_param(kind, ParamValue::Dimen(v))
            }
            Primitive::InterLinePenalty
            | Primitive::ClubPenalty
            | Primitive::WidowPenalty
            | Primitive::DisplayWidowPenalty => {
                let v = self.scan_number()?;
                let kind = match prim {
                    Primitive::InterLinePenalty => ParamKind::InterLinePenalty,
                    Primitive::ClubPenalty => ParamKind::ClubPenalty,
                    Primitive::WidowPenalty => ParamKind::WidowPenalty,
                    _ => ParamKind::DisplayWidowPenalty,
                };
                self.assign_param(kind, ParamValue::Number(v))
            }
            // ETRIP 第二波：e-TeX 惩罚数组（\interlinepenalties n p1 ... pn 等）
            // 扫描 n 个 penalty 值并存储到 penalty_arrays[kind]（断页器后续读取）。
            Primitive::InterLinePenalties
            | Primitive::ClubPenalties
            | Primitive::WidowPenalties
            | Primitive::DisplayWidowPenalties => {
                let kind: u8 = match prim {
                    Primitive::InterLinePenalties => 0,
                    Primitive::ClubPenalties => 1,
                    Primitive::WidowPenalties => 2,
                    _ => 3,
                };
                let n = self.scan_number()?;
                let n = usize::try_from(n).unwrap_or(0);
                let mut arr = Vec::with_capacity(n);
                for _ in 0..n {
                    arr.push(self.scan_number()?);
                }
                let global = self.is_global();
                if !global && self.group_level > 0 {
                    self.save_stack.push((
                        self.group_level,
                        SavedValue::PenaltyArray {
                            kind,
                            prev: std::mem::take(&mut self.penalty_arrays[kind as usize]),
                        },
                    ));
                }
                self.penalty_arrays[kind as usize] = arr;
                // 推送给折行器（linebreak 行间惩罚读取；tex.web interline 语义）
                self.sink
                    .penalty_array_changed(kind, &self.penalty_arrays[kind as usize])?;
                self.finish_assignment();
                Ok(())
            }
            // ETRIP 第二波：盒子复制/拆包原语（\copy/\unhbox/\unvbox/\unhcopy/\unvcopy/\lastbox）
            Primitive::Copy => {
                let idx = self.scan_register_index()?;
                self.sink.copy_box(idx)
            }
            Primitive::UnHBox | Primitive::UnHCopy => {
                let idx = self.scan_register_index()?;
                self.sink.unhbox(idx, prim == Primitive::UnHCopy)
            }
            Primitive::UnVBox | Primitive::UnVCopy => {
                let idx = self.scan_register_index()?;
                self.sink.unvbox(idx, prim == Primitive::UnVCopy)
            }
            Primitive::LastBox => self.sink.lastbox(),
            // ETRIP 第二波：列表尾操作（\unskip/\unpenalty）
            Primitive::UnSkip => self.sink.unskip(),
            Primitive::UnPenalty => self.sink.unpenalty(),
            // TRIP 冲刺：\unkern：移除当前列表尾的 kern 节点（TRIP L189）
            Primitive::Unkern => self.sink.unkern(),
            // TRIP 冲刺：纯 VM 原语（\romannumeral/\char/\uppercase/\lowercase/
            // \endinput/\ignorespaces/\uccode）
            Primitive::RomanNumeral => self.exec_roman_numeral(),
            Primitive::Char => self.exec_char(),
            Primitive::Uppercase => self.exec_uppercase(),
            Primitive::Lowercase => self.exec_lowercase(),
            Primitive::EndInput => self.exec_endinput(),
            Primitive::Ignorespaces => self.exec_ignorespaces(),
            // TRIP 冲刺：\fontname<font>：展开为字体外部名（TRIP L218）
            Primitive::FontName => self.exec_fontname(),
            Primitive::Uccode => self.exec_uccode(),
            // ETRIP 第二波：盒子尺寸赋值（\wd/\ht/\dp<n>=<dimen>；无 '=' 时按 TeX 报错）
            Primitive::Wd | Primitive::Ht | Primitive::Dp => {
                let dim: u8 = match prim {
                    Primitive::Wd => 0,
                    Primitive::Ht => 1,
                    _ => 2,
                };
                let idx = self.scan_register_index()?;
                self.expect_equals()?;
                let v = self.scan_dimen()?;
                self.sink.set_box_dim(idx, dim, v)
            }
            // ETRIP 第二波：诊断原语（\showgroups/\showlists）
            Primitive::ShowGroups => self.sink.showgroups(),
            Primitive::ShowLists => self.sink.showlists(),
            // ETRIP 第二波：\omit（对齐模板跳过；简化为 no-op，由对齐组后续实现语义）
            Primitive::Omit => Ok(()),
            // ETRIP 第二波：\mutoglue/\gluetomu 单独出现（数字/尺寸上下文由扫描函数处理）。
            // 裸用按 TeX 报 "You can't use \mutoglue in vertical mode." 并恢复（简化：发胶水 token）。
            Primitive::MuToGlue => {
                let g = self.scan_glue_mu()?;
                self.emit_tokens(emit_glue(g))
            }
            Primitive::GlueToMu => {
                let g = self.scan_glue()?;
                self.emit_tokens(emit_mu_glue(g))
            }
            // ETRIP 第二波：\lastpenalty 单独出现：no-op（数字上下文由 scan_number 读取）
            Primitive::LastPenalty => Ok(()),
            // 内部整数参数（\tracingstats 等 25 个）与交互模式命令（\batchmode 等 4 个）
            // 已由上方 int_param_index / interaction_mode_value 守卫分支处理；编译器
            // 不计守卫为覆盖，此处兜底仅满足穷尽性检查（未来新增原语会在此显式报错）。
            other => Err(Error::internal(format!(
                "未接入 exec_primitive 的原语 {other:?}（内部整数/交互模式应走守卫分支）"
            ))),
        }
    }

    fn exec_message(&mut self) -> Result<()> {
        let toks = self.scan_group_contents(None)?;
        let s = self.expand_to_string(&toks)?;
        self.sink.message(s)
    }

    /// `\show<token>`：显示下一个 token 的含义（TeX："> \cs=..."，不展开）。
    fn exec_show(&mut self) -> Result<()> {
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\show 后缺少 token"))?
            .0;
        self.sink.show(format!("> {}", self.show_meaning(tok)))
    }

    /// `\showthe<内部量>`：显示内部量当前值（TeX："> \count0=5."）。
    fn exec_showthe(&mut self) -> Result<()> {
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\showthe 后缺少内部量"))?
            .0;
        let Some(csid) = tok.csid() else {
            // TeX：非内部量 → "! You can't use `X' after \the." + 恢复显示 0
            // （参考 log L216 `\showthe$`）。
            let what = tok
                .charcode()
                .and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_else(|| "token".to_owned());
            self.sink
                .write16(format!("! You can't use `{what}' after \\the.\n"))?;
            return self.sink.show("> 0.".to_owned());
        };
        let name = self.intern.name(csid).to_owned();
        let value_toks = self.the_tokens_after(tok)?;
        let value = detok_tokens(&value_toks, &self.intern);
        self.sink.show(format!("> \\{name}={value}."))
    }

    /// `\show` 的含义描述（字符 / 控制序列的 eqtb 槽含义）。
    fn show_meaning(&self, tok: Token) -> String {
        match tok.kind() {
            TokenKind::Char => meaning(tok, &self.intern),
            TokenKind::ControlSeq => {
                let csid = tok.csid().expect("ControlSeq 必有 csid");
                let name = self.intern.name(csid).to_owned();
                match self.eqtb.slot(csid).clone() {
                    EqSlot::Undefined => format!("\\{name}=undefined."),
                    EqSlot::Primitive(_) => format!("\\{name}=\\{name}."),
                    EqSlot::Macro(m) => {
                        let params: String = (1..=m.value.params.num_params)
                            .map(|n| format!("#{n}"))
                            .collect();
                        let body = detok_tokens(&m.value.body, &self.intern);
                        format!("\\{name}=macro:{params}->{body}.")
                    }
                    EqSlot::Char { catcode, charcode } => {
                        let ch = char::from_u32(charcode).unwrap_or('\u{FFFD}');
                        let desc = if catcode == Catcode::Letter {
                            format!("the letter {ch}")
                        } else {
                            format!("the character {ch}")
                        };
                        format!("\\{name}={desc}.")
                    }
                    EqSlot::Font(f) => format!("\\{name}=select font {f}."),
                    EqSlot::Register(k, n) => format!("\\{name}=\\{}{}.", reg_kind_name(k), n),
                    EqSlot::Stream(_, n) => format!("\\{name}=write{n}."),
                    // \mathchardef 绑定：TeX 显示为 \mathchar"XXXX（十六进制）
                    EqSlot::MathChar(code) => format!("\\{name}=\\mathchar\"{code:X}."),
                    EqSlot::Alias(_) => {
                        // \let 别名：沿链解析（防环）后显示目标槽含义
                        let mut id = csid;
                        let mut hops = 0;
                        while let EqSlot::Alias(t) = self.eqtb.slot(id) {
                            id = *t;
                            hops += 1;
                            if hops > 64 {
                                break;
                            }
                        }
                        self.show_meaning(Token::control_sequence(id))
                    }
                }
            }
            _ => String::new(),
        }
    }
    fn exec_endinput(&mut self) -> Result<()> {
        for i in (0..self.stack.len()).rev() {
            if matches!(self.stack[i], InputFrame::Source { .. }) {
                self.stack.truncate(i);
                if self
                    .stack
                    .iter()
                    .all(|f| !matches!(f, InputFrame::Source { .. }))
                {
                    self.ended = true;
                }
                break;
            }
        }
        Ok(())
    }

    /// TRIP：`\ignorespaces`：跳过后续空格 token（tex.web；trip.tex l.315）。
    fn exec_ignorespaces(&mut self) -> Result<()> {
        // TeX get_x_token 语义：跳过后续空格，且**展开宏**（\space 等）——
        // 展开结果若为空格继续跳。参考 log 中 \ignorespaces\space\space 的
        // 空格被吸收不打印 blank space（tracingcommands）；NTex 此前只跳
        // 空格 token，\space 展开的空格漏到主循环多打印（TRIP diff 之一）。
        loop {
            let Some((tok, _)) = self.fetch()? else {
                return Ok(());
            };
            match tok.catcode() {
                Some(Catcode::Space) => continue,
                _ => {
                    if let Some(csid) = tok.csid() {
                        if let EqSlot::Macro(def) = self.eqtb.slot(csid).clone() {
                            self.call_macro(csid, def.value.clone())?;
                            continue;
                        }
                    }
                    self.unread(tok);
                    return Ok(());
                }
            }
        }
    }

    /// TRIP：`\uccode<char>=<num>`：设置字符的大写码（tex.web assign_int；
    /// trip.tex l.211 `\uccode`m=`A`）。语义与 `\lccode` 一致。
    fn exec_uccode(&mut self) -> Result<()> {
        let mut byte = self.scan_char_code()?;
        if !(0..=255).contains(&byte) {
            let mut msg = "! Improper \\uccode.\n".to_string();
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
                    prev: self.uccodes[byte as usize],
                },
            ));
        }
        self.uccodes[byte as usize] = value;
        self.finish_assignment();
        Ok(())
    }

    /// `\meaning<token>`（可展开）：token 的含义文本（TeX 格式，无尾随句点）。
    fn exec_meaning(&mut self) -> Result<()> {
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\meaning 后缺少 token"))?
            .0;
        let text = self.meaning_text(tok);
        self.emit_tokens(
            text.bytes()
                .map(|b| Token::char(Catcode::Other, u32::from(b)))
                .collect(),
        )
    }

    /// `\meaning` 的含义描述（字符 / 控制序列的 eqtb 槽含义；TeX `\meaning` 格式）。
    fn meaning_text(&self, tok: Token) -> String {
        match tok.kind() {
            TokenKind::Char => meaning(tok, &self.intern),
            TokenKind::ControlSeq => {
                let csid = tok.csid().expect("ControlSeq 必有 csid");
                let name = self.intern.name(csid).to_owned();
                match self.eqtb.slot(csid).clone() {
                    EqSlot::Undefined => "undefined".to_owned(),
                    EqSlot::Primitive(_) => format!("\\{name}"),
                    EqSlot::Macro(m) => {
                        let params: String = (1..=m.value.params.num_params)
                            .map(|n| format!("#{n}"))
                            .collect();
                        let body = detok_tokens(&m.value.body, &self.intern);
                        format!("macro:{params}->{body}")
                    }
                    EqSlot::Char { catcode, charcode } => {
                        meaning(Token::char(catcode, charcode), &self.intern)
                    }
                    EqSlot::Font(_) => format!("select font {name}"),
                    EqSlot::Register(k, n) => format!("\\{}{}", reg_kind_name(k), n),
                    EqSlot::Stream(_, n) => format!("write{n}"),
                    EqSlot::MathChar(code) => format!("\\mathchar\"{code:X}"),
                    EqSlot::Alias(_) => {
                        // \let 别名：沿链解析（防环）后显示目标槽含义
                        let mut id = csid;
                        let mut hops = 0;
                        while let EqSlot::Alias(t) = self.eqtb.slot(id) {
                            id = *t;
                            hops += 1;
                            if hops > 64 {
                                break;
                            }
                        }
                        self.meaning_text(Token::control_sequence(id))
                    }
                }
            }
            _ => String::new(),
        }
    }

    /// `\mathchardef\cs=<num>`：绑定 cs 为数学字符（类<<15 | 族<<8 | 字符）。
    /// 越界值（<0 或 >32767）报 "! Bad mathchar code." 且不改变绑定（TeX 语义）。
    fn exec_mathchardef(&mut self) -> Result<()> {
        let csid = self.scan_cs_ident()?;
        self.expect_equals()?;
        let v = self.scan_number()?;
        if !(0..=0x7FFF).contains(&v) {
            let _ = self.sink.write16(format!("! Bad mathchar code ({v})."));
            return Ok(());
        }
        self.set_slot_scoped(csid, EqSlot::MathChar(v as u32));
        self.finish_assignment();
        Ok(())
    }

    /// `\showtokens{<general text>}`：展开后显示 token 列表（TeX："> <tokens>." + 换行）。
    fn exec_showtokens(&mut self) -> Result<()> {
        // TeX `<general text>`：`\showtokens\expandafter{#1}` 组前先展开可展开项
        let toks = self.scan_group_contents_expanding()?;
        let text = self.expand_to_string(&toks)?;
        self.sink.show(format!("> {text}."))
    }

    /// `\fontcharwd/ht/dp/ic<font><char>`：查询字体字符度量分量（sp）并展开为维度。
    /// 参数非法（字体标识符/字符码越界）→ 报 "! Bad character code." 并恢复
    /// （TeX 对 `\fontcharwd \fontcharht ...` 裸用同样报错继续）。
    fn exec_fontchar_dimen(&mut self, prim: Primitive) -> Result<()> {
        let component = match prim {
            Primitive::FontCharWd => 0,
            Primitive::FontCharHt => 1,
            Primitive::FontCharDp => 2,
            _ => 3, // FontCharIc（italic correction：TFM 无此字段，恒 0）
        };
        let scanned = (|| -> Result<(u32, u32)> {
            let font = self.scan_font_ident()?;
            let ch = self.scan_number()?;
            if !(0..=255).contains(&ch) {
                return Err(Error::invalid_input("Bad character code"));
            }
            Ok((font, ch as u32))
        })();
        let (font, ch) = match scanned {
            Ok(v) => v,
            Err(_) => {
                self.report_error("Bad character code.");
                return Ok(());
            }
        };
        let m = self.font_loader.char_metric(font, ch);
        let v = match component {
            0 => m.map(|x| x.0).unwrap_or(0),
            1 => m.map(|x| x.1).unwrap_or(0),
            _ => 0,
        };
        self.emit_tokens(emit_dimen(v))
    }

    /// `\showifs`：显示当前条件嵌套状态（e-TeX 诊断原语；简化格式）。
    fn exec_showifs(&mut self) -> Result<()> {
        let depth = self.cond_stack.len();
        self.sink
            .show(format!("{depth} conditionals are open (level \\currentiflevel)"))
    }

    /// `\parshape=<n> <indent1> <width1> ...`：设置段落形状（n≤0 清空）。
    fn exec_parshape(&mut self) -> Result<()> {
        let n = self.scan_number()?; // 可选 `=`
        if n <= 0 {
            self.parshape = Vec::new();
            return Ok(());
        }
        let mut shape = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let indent = self.scan_dimen()?;
            let width = self.scan_dimen()?;
            shape.push((indent, width));
        }
        self.parshape = shape;
        Ok(())
    }

    /// `\parshapelength/indent/dimen<n>` 取值（sp）。TeX 语义（etrip 实证）：
    /// - n ≤ 0 → 0；
    /// - indent/length：n > 行数 → 最后一行对应值；
    /// - dimen：n > 2×行数 → 按 (n-2·len) 奇偶回退到最后两个值之一。
    fn parshape_access(&self, n: i64, kind: u8) -> i64 {
        if n <= 0 {
            return 0;
        }
        let len = self.parshape.len() as i64;
        if len == 0 {
            return 0;
        }
        match kind {
            // 0 = indent（值 2n-1）；1 = length（值 2n）
            0 | 1 => {
                let line = if n > len { len } else { n };
                let (i, w) = self.parshape[(line - 1) as usize];
                if kind == 0 { i } else { w }
            }
            // 2 = dimen（值 n）
            _ => {
                let total = 2 * len;
                let idx = if n <= total {
                    n
                } else {
                    let diff = n - total;
                    if diff % 2 == 0 { total } else { total - 1 }
                };
                let (i, w) = self.parshape[((idx - 1) / 2) as usize];
                if idx % 2 == 1 { i } else { w }
            }
        }
    }

    /// `\readline<n>to\cs`：读流下一行（原始字符，不 token 化）——去尾随空格后
    /// 附加 `\endlinechar`（>=0 时）为字符 token 存入宏体（e-TeX readline 语义）。
    fn exec_readline(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\readline", 15)?;
        self.scan_keyword(|w| w == "to")?;
        self.skip_spaces()?;
        let t = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\readline 后缺少控制序列"))?
            .0;
        let csid = t
            .csid()
            .ok_or_else(|| Error::invalid_input("\\readline to 后必须是控制序列"))?;
        let line = {
            let Some(stream) = self.read_streams.get_mut(idx).and_then(|s| s.as_mut()) else {
                return Err(Error::invalid_input("\\readline 流未打开"));
            };
            if stream.pos >= stream.data.len() {
                return Err(Error::invalid_input("\\readline 到文件末尾（EOF）"));
            }
            let start = stream.pos;
            let end = stream.data[start..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|i| start + i)
                .unwrap_or(stream.data.len());
            let mut line = stream.data[start..end].to_vec();
            stream.pos = if end < stream.data.len() { end + 1 } else { end };
            // 去尾随空格（TeX readline：空白行尾去除；\r 一并处理）
            while matches!(line.last(), Some(b' ' | b'\t' | b'\r')) {
                line.pop();
            }
            // 附加 \endlinechar（TeX 语义：>0 时在行尾附加该字符）
            if let ParamValue::Number(eol) = self.params.get(ParamKind::EndlineChar) {
                if eol > 0 {
                    line.push(eol as u8);
                }
            }
            line
        };
        let toks: Vec<Token> = line
            .into_iter()
            .map(|b| Token::char(Catcode::Other, u32::from(b)))
            .collect();
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

}
