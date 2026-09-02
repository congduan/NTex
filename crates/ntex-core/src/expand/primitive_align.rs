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
            // \crcr（对齐行结束）与 \-（断字断点）：简化 no-op
            Primitive::CrCr | Primitive::DiscMinus => Ok(()),
            // ETRIP 第二波：\omit（对齐模板跳过；简化为 no-op，由对齐组后续实现语义）
            Primitive::Omit => Ok(()),
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
            other => Err(Error::internal(format!(
                "未接入 dispatch_align 的原语 {other:?}"
            ))),
        }
    }
}
