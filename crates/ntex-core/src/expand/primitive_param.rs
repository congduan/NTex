// 内部参数原语 dispatcher。
//
// 主题：所有 dimen/glue/number 内部参数赋值原语 + `int_param_index` / `interaction_mode_value`
// 守卫分支 + 段落/页面胶水与尺寸补全（HangIndent/.../LastKern）+ e-TeX 惩罚数组
// （InterLinePenalties/...）。
//
// 维护约定（与 expand/mod.rs 既有 include! 链一致）：
// - 仅 `impl Expander { ... }` 块，无 use / 无模块声明；
// - 入口 `dispatch_param(prim: Primitive)` 由 `primitive.rs` 主 match 委托；
// - 函数 1:1 来自 `exec_primitive` 主 match，零行为变化；
// - 守卫分支 `p if int_param_index(p).is_some()` 与
//   `p if interaction_mode_value(p).is_some()` 通过共享的 `p` 兜底 case
//   （编译期不视守卫为覆盖，需显式列出）。

impl Expander {
    /// 内部参数原语 dispatcher：所有 dimen/glue/number 内部参数赋值。
    /// 由 `exec_primitive` 主 match 在覆盖完整 primitive 集合后委托。
    pub(super) fn dispatch_param(&mut self, prim: Primitive) -> Result<()> {
        match prim {
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
                // TeX 赋值语义：`=`(可选) 后跟 <integer>；<integer> 可经宏/可展开
                // 原语产生（trip.tex L103 `\tracingoutput\on`——\on 是宏=1；
                // 2026-09-03 前 \on 未展开直接当"单独出现 no-op"，tracingoutput
                // 永不开启 → TRIP shipout 转录缺失）。展开后重新判定；非数字
                // 且不可展开才是单独出现 no-op（`$\splitdiscards\noindent`）。
                self.skip_spaces()?;
                loop {
                    let Some((tok, _)) = self.fetch()? else {
                        return Ok(()); // EOF：单独出现 no-op
                    };
                    if tok.charcode() == Some(b'=' as u32) {
                        break;
                    }
                    let is_digit = matches!(tok.catcode(), Some(Catcode::Other))
                        && matches!(
                            tok.charcode(),
                            Some(c)
                                if (b'0' as u32..=b'9' as u32).contains(&c)
                                    || c == b'+' as u32
                                    || c == b'-' as u32
                        );
                    if is_digit {
                        self.unread(tok);
                        break;
                    }
                    // cs：可展开（宏/展开原语）→ 展开压栈后重判（TeX get_x_token）
                    if let Some(csid) = tok.csid() {
                        let slot = self.eqtb.slot(csid).clone();
                        // <internal integer> 直接作值起点（tex.web scan_int 的
                        // internal integer 臂：\countdef'd cs、\count<n> 等）。
                        // 不认这个臂 → `\escapechar\m@ne` 被判成"单独出现 no-op"，
                        // \m@ne 回流后当赋值目标吞掉后续 token——plain.tex \newif
                        // 因此把 \m@ne(\count22) 抹成 0，\newinsert 分配器失步。
                        //
                        // `EqSlot::Char`（`\chardef` 定义的 cs）同属这一臂：
                        // tex.web scan_int 的 internal integer 含 \chardef'd cs，
                        // 其取值为字符码（取值实现见 scan.rs 的 `EqSlot::Char` 臂）。
                        // 2026-09-11 修：此前漏掉这一臂 → `\fam\bffam`（plain 的
                        // `\bf`！）被判"单独出现 no-op"，\bffam 回流后被当**字符**
                        // 排版——每个 `\bf` 都往盒里多插一个字符码 6 的节点
                        // （cmr10 char 6 宽 7.22pt，DVI 与真实 TeX 不一致；
                        //  换 Unicode 字体后更直接报 `Missing character: no ^^F`）。
                        let internal_integer = matches!(
                            &slot,
                            EqSlot::Register(..)
                                | EqSlot::Char { .. }
                                | EqSlot::Primitive(
                                    Primitive::Count
                                        | Primitive::Dimen
                                        | Primitive::Skip
                                        | Primitive::Muskip,
                                )
                        );
                        if internal_integer {
                            self.unread(tok);
                            break;
                        }
                        let expandable = match &slot {
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
                                let items: Vec<(Token, bool)> = expansion.into_iter().collect();
                                self.push_frame(InputFrame::TokenList {
                                    items: Arc::from(items),
                                    pos: 0,
                                });
                            }
                            continue;
                        }
                    }
                    self.unread(tok);
                    // 单独出现（无 `=` 非数字）：no-op——TeX 内部整数在主循环/数学
                    // 模式均不读值（TRIP `{\tracingstats}` 追踪后无操作；
                    // ETRIP `$\splitdiscards` 数学模式同样 no-op——参考
                    // showbox27 空数学，l.1148 的 Missing $ inserted 由
                    // `\noindent`/`}` 触发）
                    return Ok(());
                }
                let v = self.scan_number()?;
                // TRIP 语义：\prevgraf 只允许非负（trip L392 `\prevgraf=-1` 报
                // "! Bad \prevgraf (-1)." 恢复为 0；参考 log 对齐）
                if p == Primitive::PrevGraf && v < 0 {
                    self.report_error(&format!("Bad \\prevgraf ({v})."));
                    self.assign_param(ParamKind::MiscInt(idx), ParamValue::Number(0))?;
                } else {
                    self.assign_param(ParamKind::MiscInt(idx), ParamValue::Number(v))?;
                }
                Ok(())
            }

            // ETRIP 冲刺：交互模式命令（\batchmode/\nonstopmode/\scrollmode/\errorstopmode）
            p if interaction_mode_value(p).is_some() => {
                let v = interaction_mode_value(p).expect("已检查 is_some");
                self.assign_param(ParamKind::MiscInt(19), ParamValue::Number(v))
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
            other => Err(Error::internal(format!(
                "未接入 dispatch_param 的原语 {other:?}"
            ))),
        }
    }
}
