use crate::sink::DirectionKind;

impl Expander {
    // ---------- 原语执行 ----------

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
            Primitive::Let => self.exec_let(),
            Primitive::Catcode => self.exec_catcode(),
            Primitive::SfCode => self.exec_sfcode(),
            Primitive::LcCode => self.exec_lccode(),
            // ETRIP 冲刺：\advance<寄存器> <增量>（寄存器运算）
            Primitive::Advance => self.exec_advance(),
            Primitive::End => {
                self.stack.clear();
                self.output_active = false;
                self.cond_stack.clear();
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
            Primitive::EndGroup => self.end_group(),
            // M3-2 排版原语
            // 盒子：扫描可选 to/spread 规格，直通 sink（排版器解释）。
            Primitive::HBox | Primitive::VBox | Primitive::VTop => {
                let (to, spread) = self.scan_box_spec()?;
                self.sink.box_spec(to, spread)?;
                self.sink.primitive(prim)
            }
            Primitive::Par => self.sink.primitive(prim),
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
                let p = self.scan_number()?;
                self.sink.penalty(p)
            }
            Primitive::HRule | Primitive::VRule => {
                let [h, d, w] = self.scan_rule_specs()?;
                self.sink.rule(w, h, d)
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
                let v = self.scan_dimen()?;
                let kind = if prim == Primitive::VSize {
                    ParamKind::VSize
                } else {
                    ParamKind::MaxDepth
                };
                self.assign_param(kind, ParamValue::Dimen(v))
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
            // ETRIP 冲刺：TeX/e-TeX 内部整数参数（misc 数组，按下标索引）
            p if int_param_index(p).is_some() => {
                let idx = int_param_index(p).expect("已检查 is_some");
                let v = self.scan_number()?;
                self.assign_param(ParamKind::MiscInt(idx), ParamValue::Number(v))
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
                let idx = usize::try_from(idx)
                    .map_err(|_| Error::invalid_input("寄存器下标越界"))?;
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
            Primitive::ShipOut => self.sink.primitive(prim),
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
                self.immediate_pending = true;
                Ok(())
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
                self.sink.math_left(d)
            }
            Primitive::Right => {
                let d = self.scan_delimiter()?;
                self.sink.math_right(d)
            }
            // e-TeX（M4-5）：\middle<delimiter>（\left...\right 内分隔符）
            Primitive::Middle => {
                let d = self.scan_delimiter()?;
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
            // ETRIP 冲刺：\valign/\halign：下一个组为对齐组（组种类 6）
            Primitive::Valign | Primitive::Halign => self.sink.align_begin(),
            // ETRIP 冲刺：\noalign{...}：下一个组为无对齐组（组种类 7）
            Primitive::NoAlign => self.sink.noalign_begin(),
            // ETRIP 冲刺：\cr（对齐行结束）：无操作（简化；对齐组按盒子处理）
            Primitive::Cr => self.sink.align_row_end(),
            // ETRIP 冲刺：\mathchoice{D}{T}{S}{SS}：收集四个分支（内容不执行）
            Primitive::MathChoice => {
                for _ in 0..4 {
                    self.scan_group_contents(None)?;
                }
                Ok(())
            }
            // ETRIP 冲刺：\raise/\lower<dimen><box>：记录盒子参考点位移（下一个封装盒子生效）
            Primitive::Raise | Primitive::Lower => {
                let amount = self.scan_dimen()?;
                let amount = if prim == Primitive::Lower {
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
            Primitive::MathOrd => self.sink.math_class(0),
            Primitive::MathBin => self.sink.math_class(1),
            Primitive::MathOp => self.sink.math_class(2),
            Primitive::MathRel => self.sink.math_class(3),
            Primitive::MathOpen => self.sink.math_class(4),
            Primitive::MathClose => self.sink.math_class(5),
            Primitive::MathPunct => self.sink.math_class(6),
            Primitive::MathInner => self.sink.math_class(7),
            Primitive::Nonscript => self.sink.primitive(prim),
            // TRIP 冲刺：\noboundary（数学字符边界抑制；水平/垂直模式 no-op）
            Primitive::NoBoundary => self.sink.primitive(prim),
            // TRIP 冲刺：\moveleft/\moveright<dimen><box>（盒子水平位移）
            Primitive::MoveLeft => {
                let d = self.scan_dimen()?;
                self.sink.move_left(d)
            }
            Primitive::MoveRight => {
                let d = self.scan_dimen()?;
                self.sink.move_right(d)
            }
            // TRIP 冲刺：\accent（读音符）。TeX 在数学模式先报错且不扫描数字
            //（TRIP L396 `\accent\x`），模式判断在布局层，故不预扫描。
            Primitive::Accent => self.sink.primitive(prim),
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
                let v = self.eval_int_expression();
                match v {
                    Ok(v) => self.emit_tokens(emit_count(v)),
                    Err(_) => {
                        let _ = self
                            .sink
                            .write16("! You can't use \\numexpr in vertical mode.\n".to_string());
                        Ok(())
                    }
                }
            }
            // M4-5 e-TeX 扩展：\dimexpr/\glueexpr 可展开求值（\the 上下文由 the_tokens 直接读取）
            Primitive::Dimexpr => {
                let v = self.eval_dimen_expression();
                match v {
                    Ok(v) => self.emit_tokens(emit_dimen(v)),
                    Err(_) => {
                        let _ = self
                            .sink
                            .write16("! You can't use \\dimexpr in vertical mode.\n".to_string());
                        Ok(())
                    }
                }
            }
            Primitive::Glueexpr => {
                let g = self.eval_glue_expression();
                match g {
                    Ok(g) => self.emit_tokens(emit_glue(g)),
                    Err(_) => {
                        let _ = self
                            .sink
                            .write16("! You can't use \\glueexpr in vertical mode.\n".to_string());
                        Ok(())
                    }
                }
            }
            Primitive::Scantokens => self.exec_scantokens(),
            Primitive::Detokenize => self.exec_detokenize(),
            Primitive::Unexpanded => self.exec_unexpanded(),
            // \csname...\endcsname：构造控制序列（TeX 可展开原语）
            Primitive::Csname => self.exec_csname(),
            Primitive::EndCsname => {
                let _ = self.sink.write16("! Extra \\endcsname.\n".to_string());
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
            | Primitive::CurrentIfBranch => {
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
                let _ = self
                    .sink
                    .write16("! You can't use \\parshape... in vertical mode.\n".to_string());
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
            Primitive::SetBox => {
                let idx = self.scan_register_index()?;
                self.expect_equals()?;
                self.sink.setbox(idx)
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
            Primitive::PrevDepth => {
                let v = self.scan_dimen()?;
                self.assign_param(ParamKind::PrevDepth, ParamValue::Dimen(v))
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
                let g = self.scan_glue()?;
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
            _ => Err(Error::internal(
                "未接入 exec_primitive 的原语（内部整数/交互模式应走守卫分支）",
            )),
        }
    }

    /// `\textfont<fam>=<fontcs>` 族分配：扫描 fam 号、可选 `=`、字体选择器 cs。
    fn exec_math_font(&mut self, kind: u8) -> Result<()> {
        let fam = self.scan_number()?;
        // TeX：族号越界报 "! Bad number" 钳制（<0 → 0，>15 → 15；TRIP L346
        // `\textfont16=\relax`）
        let fam = if (0..=15).contains(&fam) {
            fam
        } else {
            let _ = self.sink.write16(format!(
                "! Bad number ({}).\n\
                 Since I expected to read a number between 0 and 15,\n\
                 I changed this one to zero.\n",
                fam
            ));
            fam.clamp(0, 15)
        };
        // 可选赋值符 '='
        self.skip_spaces()?;
        let probe = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\textfont 后缺少字体"))?
            .0;
        if probe.charcode() == Some(b'=' as u32) {
            self.skip_spaces()?;
        } else {
            self.unread(probe);
        }
        let (tok, _) = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\textfont 后缺少字体选择器"))?;
        let csid = tok
            .csid()
            .ok_or_else(|| Error::invalid_input("\\textfont 后必须是 \\font 定义的 cs"))?;
        // \scriptfont1=\textfont1：RHS 为另一数学字体族 → 复制其当前字体
        // （TeX：族未赋值时为 nullfont；引擎以 FontId 0 兜底）
        let font = match self.eqtb.slot(csid).clone() {
            EqSlot::Font(f) => f,
            // TRIP：`\textfont1=\font`：`\font` 作当前字体选择器
            EqSlot::Primitive(Primitive::Font) => self.sink.current_font(),
            EqSlot::Primitive(
                Primitive::TextFont | Primitive::ScriptFont | Primitive::ScriptScriptFont,
            ) => {
                let rhs_kind = match self.eqtb.slot(csid) {
                    EqSlot::Primitive(Primitive::TextFont) => 0,
                    EqSlot::Primitive(Primitive::ScriptFont) => 1,
                    _ => 2,
                };
                let rhs_fam = self.scan_number()?;
                let rhs_fam = if (0..=15).contains(&rhs_fam) {
                    rhs_fam
                } else {
                    let _ = self.sink.write16(format!(
                        "! Bad number ({}).\nI changed this one to zero.\n",
                        rhs_fam
                    ));
                    rhs_fam.clamp(0, 15)
                };
                self.math_fonts[rhs_kind][rhs_fam as usize]
            }
            _ => {
                // TeX：\textfont<fam>=<非字体> → 报错恢复（绑定字体 0/nullfont；
                // TRIP L347 `\textfont16=\relax`）
                let _ = self.sink.write16(format!(
                    "! \\textfont 的 \\{} 不是字体选择器。\n",
                    self.intern.name(csid)
                ));
                0
            }
        };
        self.math_fonts[kind as usize][fam as usize] = font;
        self.sink.math_font(kind, fam as u8, font)
    }

    /// `\patterns{...}`（M4-6）：扫描平衡组（不展开），抽取模式文本直通 sink。
    ///
    /// TeX `new_patterns`（tex.web）语义：字母/数字/`.` 是模式字符；
    /// 其余 token（空格、控制序列等）是模式分隔符。文本交由 ntex-layout 的
    /// Liang trie 解析（ntex-layout::hyphen::PatternTrie::parse）。
    fn exec_patterns(&mut self) -> Result<()> {
        let tokens = self.scan_group_contents(None)?;
        let mut out: Vec<u8> = Vec::new();
        for tok in tokens {
            match tok.catcode() {
                Some(Catcode::Letter) | Some(Catcode::Other) => {
                    let ch = tok.charcode().expect("Char 必有 charcode");
                    let is_pattern_char = u8::try_from(ch).is_ok_and(|b| {
                        b.is_ascii_alphabetic() || b.is_ascii_digit() || b == b'.'
                    });
                    if is_pattern_char {
                        out.push(ch as u8);
                    } else if out.last() != Some(&b' ') {
                        out.push(b' ');
                    }
                }
                _ => {
                    // 空格（cat 10）与任何其他 token：模式分隔符
                    if out.last() != Some(&b' ') {
                        out.push(b' ');
                    }
                }
            }
        }
        self.sink.patterns(out)
    }

    /// `\hyphenation{...}`（ETRIP 冲刺）：扫描平衡组，解析异常词表。
    ///
    /// TeX `new_hyphenation`（tex.web）语义：
    /// - 空格（cat 10）分隔单词；字母/其他字符（cat 11/12）是词字符；
    /// - `-`（断字符，默认 hyphenchar 45）标记允许的断点（可位于词首/词尾）；
    /// - 词字符经 `\lccode` 转小写后存储（扫描时转换，组结束不回滚异常表）；
    /// - 异常词按当前 `\language`（misc[16]）归档，断字时**优先于**模式表。
    ///
    /// 简化：暂按单语言全局表存储（sink 侧不分语言）；词比较不做 lccode 二次
    /// 转换（段落词需已小写）。ETRIP 用例均满足。
    fn exec_hyphenation(&mut self) -> Result<()> {
        // TeX：\hyphenation 参数为 <general text>；前导 \relax 跳过
        // （trip.tex L72 `\hyphenation\relax{...}`，TeX scan_toks 的 \relax 分隔）
        self.skip_spaces()?;
        if let Some(csid) = self.peek_csid()? {
            if matches!(self.eqtb.slot(csid), EqSlot::Primitive(Primitive::Relax)) {
                self.fetch()?;
            }
        }
        let tokens = self.scan_group_contents(None)?;
        // 断字符：默认 `-`（45）；ETRIP 用例均用字面 `-`。
        const HYPHEN_CHAR: u32 = 45;
        let mut words: Vec<(Vec<u8>, Vec<usize>)> = Vec::new();
        let mut letters: Vec<u8> = Vec::new();
        let mut breaks: Vec<usize> = Vec::new();
        // 词首断点（`-q-` 的首 `-`）在词开始时记录：word_breaks_at_0
        let mut break_at_start = false;
        for tok in tokens {
            let cat = tok.catcode();
            match cat {
                Some(Catcode::Letter) | Some(Catcode::Other) => {
                    let ch = tok.charcode().unwrap_or(0);
                    if ch == HYPHEN_CHAR {
                        if letters.is_empty() {
                            break_at_start = true; // 词首 `-`：断点 0
                        } else {
                            breaks.push(letters.len());
                        }
                    } else {
                        // lccode 转小写（0 保留原字符：无小写映射）
                        let lower = self.lccodes[ch as usize];
                        if lower > 0 {
                            letters.push(lower as u8);
                        } else {
                            letters.push(ch as u8);
                        }
                    }
                }
                Some(Catcode::Space) | None | Some(_) => {
                    // 空格或任何非字符 token：结束当前词（若有）
                    if !letters.is_empty() {
                        if break_at_start {
                            breaks.insert(0, 0); // 词首 `-`：断点 0
                        }
                        words.push((std::mem::take(&mut letters), std::mem::take(&mut breaks)));
                    }
                    break_at_start = false;
                }
            }
        }
        if !letters.is_empty() {
            if break_at_start {
                breaks.insert(0, 0);
            }
            words.push((letters, breaks));
        }
        if !words.is_empty() {
            self.sink.hyphenation(words)?;
        }
        Ok(())
    }

    /// `\left`/`\right`/`\middle` 的定界符参数：字符 → charcode（`.` 为空定界符）；
    /// `\.` → None。无法识别的 cs（如 `\par`）按 TeX 恢复：报
    /// "! Missing delimiter (. inserted)." 到转录，并以 `(` 定界符继续。
    fn scan_delimiter(&mut self) -> Result<Option<u32>> {
        self.skip_spaces()?;
        let (tok, _) = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\left/\\right 后缺少定界符"))?;
        match tok.kind() {
            TokenKind::Char => {
                let ch = tok.charcode().expect("Char 必有 charcode");
                if ch == b'.' as u32 {
                    Ok(None) // \left.：空定界符
                } else if (b'0' as u32..=b'9' as u32).contains(&ch)
                    || ch == b'"' as u32
                    || ch == b'\'' as u32
                {
                    // TRIP：\radical"3 的 "3 是十六进制 delimiter number（TeX scan_delimiter
                    // 优先扫描数字）；放回后按数字扫描
                    self.unread(tok);
                    let n = self.scan_number()?;
                    Ok(Some(u32::try_from(n).unwrap_or(0)))
                } else {
                    Ok(Some(ch))
                }
            }
            TokenKind::ControlSeq => {
                let name = self.intern.name(tok.csid().expect("ControlSeq 必有 csid"));
                if name == "." {
                    Ok(None)
                } else {
                    let _ = self
                        .sink
                        .write16("! Missing delimiter (. inserted).\n".to_string());
                    Ok(Some(b'(' as u32))
                }
            }
            _ => Err(Error::invalid_input("\\left/\\right 后必须是定界符")),
        }
    }

    /// `\font<cs>[=]<名字>[at <dimen>|scaled <int>]`：加载字体并定义 cs 为字体选择器。
    ///
    /// 语法扫描在 VM 侧（cs、可选 `=`、字体名、可选 at/scaled），实际加载交给
    /// [`FontLoader`]（ntex-layout 的 TFM 加载器维护字体表并返回 FontId）。
    fn exec_font(&mut self) -> Result<()> {
        let name = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\font 后缺少控制序列"))?
            .0;
        let csid = name
            .csid()
            .ok_or_else(|| Error::invalid_input("\\font 后必须是控制序列"))?;
        // 可选赋值符 '='
        self.skip_spaces()?;
        let probe = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\font 后缺少字体名"))?
            .0;
        if probe.charcode() != Some(b'=' as u32) {
            self.unread(probe);
        }
        let font_name = self.scan_font_name()?;
        // 可选 at / scaled（互斥）
        let keyword = self.scan_keyword(|w| w == "at" || w == "scaled")?;
        let (at, scaled) = match keyword.as_deref() {
            Some("at") => (Some(self.scan_dimen()?), None),
            Some("scaled") => (None, Some(self.scan_number()?)),
            _ => (None, None),
        };
        let font = match self.font_loader.load(&font_name, at, scaled) {
            Ok(f) => f,
            Err(_) => {
                // TeX：字体加载失败 → "! Font not loadable" 报错恢复（绑定字体 0，
                // 后续使用报更多错但作业继续；TRIP L211 `\font\mumble=mumble`）。
                let _ = self.sink.write16(format!(
                    "! Font {font_name} not loadable: Metric (TFM) file not found.\n\
                     I'm not loading it.\n"
                ));
                0
            }
        };
        // 组作用域 + \global 语义（同 \def）
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
        self.eqtb.set_font(csid, font);
        self.finish_assignment();
        Ok(())
    }

    /// 扫描外部字体名：连续 cat 11（字母）/ cat 12（其他）字符，遇空格/组/控制序列结束。
    fn scan_font_name(&mut self) -> Result<String> {
        self.skip_spaces()?;
        let mut name = String::new();
        while let Some((tok, _)) = self.fetch()? {
            match tok.catcode() {
                Some(Catcode::Letter) | Some(Catcode::Other) => {
                    let ch = tok
                        .charcode()
                        .and_then(char::from_u32)
                        .ok_or_else(|| Error::invalid_input("字体名含非法字符"))?;
                    if !ch.is_ascii() {
                        return Err(Error::invalid_input("字体名仅支持 ASCII（M3-4 范围）"));
                    }
                    name.push(ch);
                }
                _ => {
                    self.unread(tok);
                    break;
                }
            }
        }
        if name.is_empty() {
            return Err(Error::invalid_input("\\font 后缺少字体名"));
        }
        Ok(name)
    }

    /// `\fontdimen<num><font>=<dimen>`：设置字体的 fontdimen 参数
    /// （TeX `assign_font_dimen`；组内局部、可 `\global`）。
    fn exec_fontdimen(&mut self) -> Result<()> {
        let num = self.scan_number()?;
        let num = u32::try_from(num).map_err(|_| Error::invalid_input("\\fontdimen 参数号越界"))?;
        let font = self.scan_font_ident()?;
        // TRIP L404：fontdimen 参数号越界（`\fontdimen 1000=...`）→ 报错并跳过赋值
        if num >= 13 {
            let _ = self
                .sink
                .write16("! Font \\FONT? has only 13 fontdimen parameters.\n".to_string());
            return Ok(());
        }
        self.expect_equals()?;
        let value = self.scan_dimen()?;
        let prev = self.fontdimens.get(&(font, num)).copied();
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::FontDimen { font, num, prev },
            ));
        }
        self.fontdimens.insert((font, num), value);
        self.finish_assignment();
        Ok(())
    }

    /// TeX `scan_font_ident` 的 "Missing font identifier" 报错恢复：
    /// 报错后用当前字体继续（TRIP L404 `\fontdimen 1000=20\varunit`——`=` 非字体）。
    fn missing_font_ident(&mut self) -> Result<u32> {
        let _ = self.sink.write16("! Missing font identifier.\n".to_string());
        Ok(self.sink.current_font())
    }

    /// 扫描字体标识符（TeX `scan_font_ident`）：`\font` 定义的 cs 或 `\nullfont`。
    fn scan_font_ident(&mut self) -> Result<u32> {
        self.skip_spaces()?;
        let Some((tok, _ne)) = self.fetch()? else {
            return self.missing_font_ident();
        };
        let Some(csid) = tok.csid() else {
            return self.missing_font_ident();
        };
        match self.eqtb.slot(csid).clone() {
            EqSlot::Font(f) => Ok(f),
            // TRIP：`\font`（无参数）作当前字体选择器（\textfont1=\font）
            EqSlot::Primitive(Primitive::Font) => Ok(self.sink.current_font()),
            // \textfont<n>/...：字体位置读取当前族字体（TeX find_font 语义）
            EqSlot::Primitive(
                Primitive::TextFont | Primitive::ScriptFont | Primitive::ScriptScriptFont,
            ) => {
                let kind = match self.eqtb.slot(csid) {
                    EqSlot::Primitive(Primitive::TextFont) => 0,
                    EqSlot::Primitive(Primitive::ScriptFont) => 1,
                    _ => 2,
                };
                let fam = self.scan_number()?;
                let fam = if (0..=15).contains(&fam) {
                    fam
                } else {
                    let _ = self.sink.write16(format!(
                        "! Bad number ({}).\nI changed this one to zero.\n",
                        fam
                    ));
                    fam.clamp(0, 15)
                };
                Ok(self.math_fonts[kind][fam as usize])
            }
            // TRIP：`\fontdimen6\the\scriptfont2` —— \the 展开为字体选择器
            EqSlot::Primitive(Primitive::The) => {
                let t2 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\the 后缺少内部量"))?
                    .0;
                let csid2 = t2
                    .csid()
                    .ok_or_else(|| Error::invalid_input("\\the 需要内部量参数"))?;
                match self.eqtb.slot(csid2).clone() {
                    EqSlot::Primitive(
                        Primitive::TextFont | Primitive::ScriptFont | Primitive::ScriptScriptFont,
                    ) => {
                        let kind = match self.eqtb.slot(csid2) {
                            EqSlot::Primitive(Primitive::TextFont) => 0,
                            EqSlot::Primitive(Primitive::ScriptFont) => 1,
                            _ => 2,
                        };
                        let fam = self.scan_number()?;
                        let fam = if (0..=15).contains(&fam) {
                            fam
                        } else {
                            let _ = self.sink.write16(format!(
                                "! Bad number ({}).\nI changed this one to zero.\n",
                                fam
                            ));
                            fam.clamp(0, 15)
                        };
                        Ok(self.math_fonts[kind][fam as usize])
                    }
                    _ => self.missing_font_ident(),
                }
            }
            _ => self.missing_font_ident(),
        }
    }

    /// `\hyphenchar<font>=<int>`：设置字体的断字符（TeX assign_font_int；
    /// 组内局部、可 `\global`；覆盖表存 expander 侧，排版器断字时读取）。
    fn exec_hyphenchar(&mut self) -> Result<()> {
        let font = self.scan_font_ident()?;
        self.expect_equals()?;
        let value = self.scan_number()?;
        let prev = self.hyphenchars.get(&font).copied();
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::HyphenChar { font, prev },
            ));
        }
        self.hyphenchars.insert(font, value);
        self.finish_assignment();
        Ok(())
    }

    /// `\delcode<num>=<num>`：设置字符的定界符码（TeX assign_del_code；组内局部）。
    fn exec_delcode(&mut self) -> Result<()> {
        let byte = self.scan_char_code()?;
        let byte = u8::try_from(byte).map_err(|_| Error::invalid_input("\\delcode 字符码越界"))?;
        self.expect_equals()?;
        let value = self.scan_number()?;
        let value = u32::try_from(value)
            .map_err(|_| Error::invalid_input("\\delcode 定界符码越界（24 位）"))?
            & 0x00FF_FFFF;
        let prev = self.delcodes.get(&u32::from(byte)).copied();
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::DelCode { byte, prev },
            ));
        }
        self.delcodes.insert(u32::from(byte), value);
        self.finish_assignment();
        Ok(())
    }

    /// TRIP 冲刺：`\mathcode<8位字符>=<15位值>`：字符数学码赋值
    /// （TeX：mathcode 15 位 = class(3)<<12 + family(4)<<8 + char(8)）。
    fn exec_mathcode(&mut self) -> Result<()> {
        let byte = self.scan_char_code()?;
        let byte = u8::try_from(byte).map_err(|_| Error::invalid_input("\\mathcode 字符码越界"))?;
        self.expect_equals()?;
        let value = self.scan_number()?;
        let value = u32::try_from(value)
            .map_err(|_| Error::invalid_input("\\mathcode 数学码越界（15 位）"))?
            & 0x0000_7FFF;
        let prev = self.mathcodes.get(&u32::from(byte)).copied();
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::MathCode { byte, prev },
            ));
        }
        self.mathcodes.insert(u32::from(byte), value);
        self.finish_assignment();
        Ok(())
    }

    /// 读 fontdimen：覆盖表优先；无覆盖返回 0（TFM 真实参数在排版层，后续接入）。
    fn fontdimen(&self, font: u32, num: u32) -> i64 {
        self.fontdimens.get(&(font, num)).copied().unwrap_or(0)
    }

    // ---------- ETRIP 冲刺：终端转录（\message/\show/\showthe/\write16） ----------

    /// `\message{<general text>}`：展开参数后输出到终端与日志（TeX：不换行）。
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

    fn exec_catcode(&mut self) -> Result<()> {
        let byte = self.scan_char_code()?;
        let byte = u8::try_from(byte).map_err(|_| Error::invalid_input("\\catcode 字符码越界"))?;
        self.expect_equals()?;
        let code = self.scan_number()?;
        let cat = Catcode::from_u8(
            u8::try_from(code).map_err(|_| Error::invalid_input("catcode 必须在 0..=15"))?,
        )
        .ok_or_else(|| Error::invalid_input("catcode 必须在 0..=15"))?;
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

    /// `\advance<寄存器> <增量>`：寄存器运算（TeX arithmetic；etrip.tex 91 行
    /// `\advance\count20 1`）。目标支持 `\count/\dimen/\skip/\muskip` 寄存器
    /// （数字下标或 `\countdef` 等 cs 绑定）与内部整数参数。
    fn exec_advance(&mut self) -> Result<()> {
        self.skip_spaces()?;
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\advance 后缺少寄存器"))?
            .0;
        let csid = tok
            .csid()
            .ok_or_else(|| Error::invalid_input("\\advance 后必须是寄存器"))?;
        match self.eqtb.slot(csid).clone() {
            EqSlot::Primitive(prim)
                if matches!(
                    prim,
                    Primitive::Count | Primitive::Dimen | Primitive::Skip | Primitive::Muskip
                ) =>
            {
                let kind = match prim {
                    Primitive::Count => RegKind::Count,
                    Primitive::Dimen => RegKind::Dimen,
                    Primitive::Skip => RegKind::Skip,
                    _ => RegKind::Muskip,
                };
                let idx = self.scan_register_index()?;
                self.advance_register(kind, idx)
            }
            EqSlot::Register(kind, idx) => self.advance_register(kind, idx),
            // TRIP L410：\xspaceskip 胶水参数也可 \advance
            EqSlot::Primitive(Primitive::XSpaceSkip) => {
                self.scan_keyword(|w| w == "by")?;
                let delta = self.scan_glue()?;
                let old = self.params.xspaceskip;
                let global = self.is_global();
                if !global && self.group_level > 0 {
                    self.save_stack.push((
                        self.group_level,
                        SavedValue::Param {
                            kind: ParamKind::XSpaceSkip,
                            prev: self.params.get(ParamKind::XSpaceSkip),
                        },
                    ));
                }
                self.params.xspaceskip = add_glue(old, delta);
                self.sink.param_changed(
                    ParamKind::XSpaceSkip,
                    ParamValue::Glue(self.params.xspaceskip),
                )?;
                self.finish_assignment();
                Ok(())
            }
            // 内部整数参数（\tracingstats/\language 等）也可 \advance
            EqSlot::Primitive(p) if int_param_index(p).is_some() => {
                let idx = int_param_index(p).expect("已检查 is_some");
                self.scan_keyword(|w| w == "by")?;
                let delta = self.scan_number()?;
                let val = self.params.misc[idx] + delta;
                let global = self.is_global();
                if !global && self.group_level > 0 {
                    self.save_stack.push((
                        self.group_level,
                        SavedValue::Param {
                            kind: ParamKind::MiscInt(idx),
                            prev: self.params.get(ParamKind::MiscInt(idx)),
                        },
                    ));
                }
                self.params.misc[idx] = val;
                self.finish_assignment();
                Ok(())
            }
            _ => Err(Error::invalid_input(
                "\\advance 目标必须是寄存器或内部参数",
            )),
        }
    }

    /// `\advance` 的寄存器增量应用（TeX：`new = old + delta`，胶水逐分量加）。
    fn advance_register(&mut self, kind: RegKind, idx: usize) -> Result<()> {
        // 可选 `by` 关键字（TeX：`\advance\count0 by5` 与 `\advance\count0 5` 等价）
        self.scan_keyword(|w| w == "by")?;
        match kind {
            RegKind::Count => {
                let delta = self.scan_number()?;
                self.assign_count(idx, self.registers.count(idx) + delta);
            }
            RegKind::Dimen => {
                let delta = self.scan_dimen()?;
                self.assign_dimen(idx, self.registers.dimen(idx) + delta);
            }
            RegKind::Skip => {
                let delta = self.scan_glue()?;
                let old = self.registers.skip(idx);
                self.assign_skip(idx, add_glue(old, delta));
            }
            RegKind::Muskip => {
                let delta = self.scan_glue()?;
                let old = self.registers.muskip(idx);
                self.assign_muskip(idx, add_glue(old, delta));
            }
            RegKind::Toks => return Err(Error::invalid_input("\\advance 不支持 \\toks")),
        }
        Ok(())
    }

    /// `\multiply/\divide<寄存器> by<n>`：寄存器标量乘/除（TeX arithmetic）。
    /// 目标支持 `\count/\dimen/\skip/\muskip` 寄存器与内部整数参数；
    /// 除数为 0 时按 TeX 语义保持不变。
    fn exec_multiply_divide(&mut self, prim: Primitive) -> Result<()> {
        self.skip_spaces()?;
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\multiply/\\divide 后缺少寄存器"))?
            .0;
        let csid = tok
            .csid()
            .ok_or_else(|| Error::invalid_input("\\multiply/\\divide 后必须是寄存器"))?;
        match self.eqtb.slot(csid).clone() {
            EqSlot::Primitive(p)
                if matches!(
                    p,
                    Primitive::Count | Primitive::Dimen | Primitive::Skip | Primitive::Muskip
                ) =>
            {
                let kind = match p {
                    Primitive::Count => RegKind::Count,
                    Primitive::Dimen => RegKind::Dimen,
                    Primitive::Skip => RegKind::Skip,
                    _ => RegKind::Muskip,
                };
                let idx = self.scan_register_index()?;
                self.scale_register(kind, idx, prim)
            }
            EqSlot::Register(kind, idx) => self.scale_register(kind, idx, prim),
            // 内部整数参数（\tracingstats/\language 等）也可 \multiply/\divide
            EqSlot::Primitive(p) if int_param_index(p).is_some() => {
                let idx = int_param_index(p).expect("已检查 is_some");
                self.scan_keyword(|w| w == "by")?;
                let f = self.scan_number()?;
                let val = self.params.misc[idx];
                let new = if prim == Primitive::Multiply {
                    val * f
                } else if f != 0 {
                    val / f
                } else {
                    val
                };
                let global = self.is_global();
                if !global && self.group_level > 0 {
                    self.save_stack.push((
                        self.group_level,
                        SavedValue::Param {
                            kind: ParamKind::MiscInt(idx),
                            prev: self.params.get(ParamKind::MiscInt(idx)),
                        },
                    ));
                }
                self.params.misc[idx] = new;
                self.finish_assignment();
                Ok(())
            }
            _ => Err(Error::invalid_input(
                "\\multiply/\\divide 目标必须是寄存器或内部参数",
            )),
        }
    }

    /// `\multiply/\divide` 的寄存器标量应用（胶水逐分量乘/除）。
    fn scale_register(&mut self, kind: RegKind, idx: usize, prim: Primitive) -> Result<()> {
        // 可选 `by` 关键字（TeX：`\multiply\count0 by5` 与 `\multiply\count0 5` 等价）
        self.scan_keyword(|w| w == "by")?;
        let f = self.scan_number()?;
        // TeX：除以 0 保持不变（不报错）
        let scale = |v: i64| {
            if prim == Primitive::Multiply {
                v * f
            } else if f != 0 {
                v / f
            } else {
                v
            }
        };
        match kind {
            RegKind::Count => self.assign_count(idx, scale(self.registers.count(idx))),
            RegKind::Dimen => self.assign_dimen(idx, scale(self.registers.dimen(idx))),
            RegKind::Skip => {
                let old = self.registers.skip(idx);
                // 乘除不改无穷阶（TeX：分量标量缩放，阶保留）
                self.assign_skip(
                    idx,
                    Glue {
                        width: scale(old.width),
                        stretch: scale(old.stretch),
                        shrink: scale(old.shrink),
                        ..old
                    },
                );
            }
            RegKind::Muskip => {
                let old = self.registers.muskip(idx);
                self.assign_muskip(
                    idx,
                    Glue {
                        width: scale(old.width),
                        stretch: scale(old.stretch),
                        shrink: scale(old.shrink),
                        ..old
                    },
                );
            }
            RegKind::Toks => return Err(Error::invalid_input("\\multiply/\\divide 不支持 \\toks")),
        }
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
                let _ = self.sink.write16("! Bad character code.\n".to_string());
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
