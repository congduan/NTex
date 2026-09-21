// 对齐与特殊节点原语 dispatcher。
//
// 主题：对齐（Valign/Halign/NoAlign/Cr/CrCr/MathChoice/DiscMinus/Span/Omit）、
// marks 族（Mark/Marks/TopMarks/.../TopMark/...）、特殊节点
// （Special/Discretionary/Insert/VAdjust）。
//
// 维护约定（与 expand/mod.rs 既有 include! 链一致）：
// - 仅 `impl Expander { ... }` 块，无 use / 无模块声明；
// - 入口 `dispatch_align(prim: Primitive)` 由 `primitive.rs` 主 match 委托；
// - 函数 1:1 来自 `exec_primitive` 主 match，零行为变化。

impl Expander {
    /// 对齐与特殊节点原语 dispatcher。
    pub(super) fn dispatch_align(&mut self, prim: Primitive) -> Result<()> {
        match prim {
            // M4-5 对齐：\halign/\valign（tex.web init_align + scan_spec）。
            // scan_box_spec 取 `to`/`spread`；`{` 由 scan_left_brace 消费并开
            // 对齐帧（preamble 扫描；见 align.rs）。sink 侧 align_begin 设
            // pending_kind=Align，随 `{`…… 实际由 align_start 静默开组、
            // 组关闭走 end_group（layout 在 group_end(Align) 做 fin_align）。
            Primitive::Valign | Primitive::Halign => {
                // 可选 `to <dimen>`/`spread <dimen>` 规格（同 \hbox 的 scan_box_spec；
                // TRIP L332 `\halign to 0pt{...}`、L407 `\halign to 1truemm...`）
                let (to, spread) = self.scan_box_spec()?;
                self.sink.box_spec(to, spread)?;
                let is_h = prim == Primitive::Halign;
                self.sink.align_begin(is_h)?;
                // `{`（tex.web scan_spec 的 scan_left_brace：缺失报
                // "Missing { inserted"）
                self.align_scan_left_brace()?;
                self.align_start(is_h)
            }
            // M4-5：\noalign 到达 dispatcher = 非行边界（合法位由
            // align_peek_next 消费；tex.web no_align case 的错误路径）
            Primitive::NoAlign => {
                self.write_error_help(
                    "Misplaced \\noalign.",
                    "\\noalign only allowed right between rows of an alignment.\n",
                );
                Ok(())
            }
            // M4-5：\cr 到达 dispatcher = 模板注入阶段或非对齐上下文
            // （raw 扫描的 \cr 由 align_on_token 拦截做 Insert v_j）
            Primitive::Cr => {
                self.write_error_help(
                    "Misplaced \\cr.",
                    "I'm guessing that you meant to end an alignment.\n\
                     Sorry... The \\cr that I just found was not preceded by\n\
                     an appropriate \\halign or \\valign.\n",
                );
                Ok(())
            }
            // M4-5：\span 到达 dispatcher = 模板注入阶段或非对齐上下文
            // （raw 扫描的 \span 由 align_on_token 拦截）
            Primitive::Span => {
                self.write_error_help(
                    "Misplaced \\span.",
                    "I'm guessing that you meant to end an alignment.\n\
                     Sorry... The \\span that I just found was not in an\n\
                     appropriate \\halign or \\valign.\n",
                );
                Ok(())
            }
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
            // ETRIP 冲刺：\special{<general text>}：whatsit 节点（内容只收集不排版）。
            // 走 `special()` 通道与延迟 \write 的 whatsit 分型：前者 shipout 经
            // DVI xxx 落后端，后者写流不进 DVI（tex.web 两套出口）。
            Primitive::Special => {
                let toks = self.scan_group_contents(None)?;
                let text: String = toks
                    .iter()
                    .filter_map(|t| t.charcode())
                    .filter_map(char::from_u32)
                    .collect();
                self.sink.special(text)
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
            // M4-5：\crcr 到达 dispatcher = Misplaced（raw 的 \crcr 由
            // align_on_token 拦截；行边界冗余 \crcr 由 align_peek_next 忽略）
            Primitive::CrCr => {
                self.write_error_help(
                    "Misplaced \\crcr.",
                    "I'm guessing that you meant to end an alignment.\n\
                     Sorry... The \\crcr that I just found was not preceded by\n\
                     an appropriate \\halign or \\valign.\n",
                );
                Ok(())
            }
            // \-（断字断点）：简化 no-op
            Primitive::DiscMinus => Ok(()),
            // M4-5：\omit 到达 dispatcher = 非列首（合法位由 align_init_col
            // 的 peek 消费；tex.web omit case）
            Primitive::Omit => {
                self.write_error_help(
                    "Misplaced \\omit.",
                    "I expect to see \\omit only after tab marks or the\n\
                     cr of an alignment. Presumably, I just found one\n\
                     somewhere else.\n",
                );
                Ok(())
            }
            // ETRIP 冲刺：\mark{<text>}（mark 节点）；e-TeX \marks<n>{<text>}
            Primitive::Mark | Primitive::Marks => {
                let class = if prim == Primitive::Marks {
                    let mut class = self.scan_number()?;
                    // e-TeX：marks class 走 register-code 语义（etrip.tex L208
                    // `\marks-1{-1}\marks32768{32768}`）：越界报 "! Bad register
                    // code (N)." 并钳 0（同 countdef 五连——write_error_help 错误块：
                    // ! 消息 / read-again token（数字后下一个 token `{`）/ l.N 两行 /
                    // help 2 行，逐行对齐参考 etrip.log l.153-167）。
                    if !(0..REGISTER_COUNT as i64).contains(&class) {
                        self.write_error_help(
                            &format!("Bad register code ({class})."),
                            "A register number must be between 0 and 32767.\n\
                             I changed this one to zero.\n",
                        );
                        class = 0;
                    }
                    Some(class)
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
            other => Err(Error::internal(format!(
                "未接入 dispatch_align 的原语 {other:?}"
            ))),
        }
    }
}
