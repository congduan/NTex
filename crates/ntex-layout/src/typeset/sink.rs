impl CoreSink for NodeBuilder {
    // （tokens, take_tokens 走子 trait 默认实现，NodeBuilder 未覆写）
    fn token(&mut self, tok: Token) -> Result<()> {
        // `\leaders` 引导盒子已就位：非胶水 token → TeX "Leaders not followed by
        // proper glue"（空格跳过——TeX "get next non-blank non-relax" 语义）。
        if self.box_state.leaders_box.is_some() {
            if tok.catcode() == Some(ntex_core::Catcode::Space) {
                return Ok(());
            }
            self.report_leaders_misplaced();
        }
        // 空格（cat 10）：垂直/数学模式忽略；水平模式一律转词间空白胶水。
        // tex.web big_switch：`hmode+spacer` 按 spacefactor 分流——sf=1000 →
        // `append_normal_space`，否则 `app_space`——胶水/惩罚节点之后、
        // `\leavevmode` 后的空格在真 TeX 都出胶水（`~`=\leavevmode\nobreak\ 与
        // \@citex 的 `,\penalty\@m\ ` 全靠这一语义）。\␣（ex_space）是独立命令码，
        // 不在此路径（见 primitive() 的 ControlSpace 臂）。
        // 行首/行中连续空格的吞并在扫描器侧完成（input.rs LineStart + 空格折叠），
        // 排版层不再按"上一节点"二次吞并。
        if tok.catcode() == Some(ntex_core::Catcode::Space) {
            match self.mode() {
                Mode::Vertical | Mode::Math | Mode::DisplayMath => {}
                Mode::Horizontal | Mode::RestrictedHorizontal => self.append_space_glue(),
            }
            return Ok(());
        }
        // 数学模式：字符转数学原子（^/_ 挂脚本，字母/其他 → Ord）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_char_tok(tok);
        }
        // M4-7 错误模型：数学模式外遇到 ^/_（cat 7/8）→ TeX "Missing $ inserted"
        // 并插入 $ 恢复（进入数学模式处理脚本；TRIP L263）。
        if matches!(
            tok.catcode(),
            Some(ntex_core::Catcode::Superscript) | Some(ntex_core::Catcode::Subscript)
        ) {
            self.report_error("Missing $ inserted.");
            let _ = self.enter_math(Mode::Math);
            return self.math_char_tok(tok);
        }
        let Some(node) = self.char_node(tok) else {
            return Ok(()); // 控制序列等无可排版语义
        };
        match self.mode() {
            Mode::Vertical => {
                // 垂直模式字符触发段落（TeX new_graf）
                if self.math_state.after_display {
                    // M4-4：显示公式后续文字仍在段内——无 parskip、无缩进（续排）
                    self.lists.push(Vec::new());
                    self.list_modes.push(Mode::Horizontal);
                    self.space_factor = 1000;
                    self.append_char(node);
                } else {
                    // M3-5-2：段落起始追加上下段间距 \parskip（空页上被页面构建器丢弃）
                    self.par_begin(true)?;
                    self.append_char(node);
                }
            }
            Mode::Horizontal | Mode::RestrictedHorizontal => self.append_char(node),
            Mode::Math | Mode::DisplayMath => unreachable!("数学模式已在上面分支返回"),
        }
        Ok(())
    }
    fn group_begin(&mut self, line: u32) -> Result<()> {
        // 显式组种类（\begingroup/\valign/\noalign）优先；否则盒子种类；再否则普通组
        let explicit = self.box_state.pending_kind.take();        let kind = explicit.or_else(|| {
            self.box_state.pending_box.take().map(|pb| match pb {
                // 垂直/内部垂直模式中的 \hbox 是 adjusted hbox group（TeX begin_box 语义）
                PendingBox::HBox => {
                    if self.mode() == Mode::Vertical {
                        GroupKind::AdjustedHBox
                    } else {
                        GroupKind::HBox
                    }
                }
                PendingBox::VBox => GroupKind::VBox,
                PendingBox::VTop => GroupKind::VTop,
            })
        });
        // 盒子/对齐组：复用盒子路径（对齐组按 vbox 打包）。`\noalign` 组也
        // 开垂直列表（M4-5：材料在组结束时原样入对齐流，tex.web no_align
        // 材料直接进对齐 vlist——见 group_end 的 NoAlign 分支）。
        let box_kind = match kind {
            Some(GroupKind::HBox | GroupKind::AdjustedHBox) => Some(PendingBox::HBox),
            Some(GroupKind::VBox | GroupKind::Align | GroupKind::NoAlign) => {
                Some(PendingBox::VBox)
            }
            Some(GroupKind::VTop) => Some(PendingBox::VTop),
            _ => None,
        };
        // `\shipout` 目标 = 紧邻的盒子组（内层盒子不消费该标记）
        let ship = if box_kind.is_some() {
            std::mem::take(&mut self.page_state.shipout_next)
        } else {
            false
        };
        let gkind = kind.unwrap_or_else(|| {
            if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
                GroupKind::Math
            } else {
                GroupKind::Simple
            }
        });
        // `\leaders` 引导盒子：认领到紧邻的盒子组（组结束封装时挂起等胶水）。
        // 内层盒子（如引导 hbox 里的 \vbox）不消费该标记。
        let leaders = if box_kind.is_some() {
            self.box_state.pending_leaders.take()
        } else {
            None
        };
        // `\setbox<n>=<box>` 目标寄存器：认领到**最外层**盒子组（tex.web scan_box
        // box_end 语义）。内层嵌套盒（`\setbox0=\vbox{\hbox{...}}` 的 \hbox）不消费，
        // 否则 target 被第一个内层盒抢走、RHS 的 vbox 无法入寄存器。
        // `\global` 旗标同批认领（tex.web：随赋值入 box_context，嵌套赋值不覆写）。
        let setbox = if box_kind.is_some() {
            self.box_state.setbox_target.take()
        } else {
            None
        };
        let setbox_global = setbox.is_some() && self.box_state.setbox_global;
        // `to`/`spread` 规格：同 setbox 一样认领到紧邻盒子组（嵌套时内层盒的
        // box_spec 调用不得覆写外层已认领的规格）。
        let spec = if box_kind.is_some() {
            self.box_state.pending_box_spec.take()
        } else {
            None
        };
        // 位移前缀（\raise/\lower/\moveleft/\moveright）：认领到紧邻盒子组
        // （tex.web：位移只作用于紧随其后的那个盒子，不向内层嵌套盒泄漏）。
        let shift = if box_kind.is_some() {
            self.box_state
                .pending_shift
                .take()
                .or_else(|| self.box_state.pending_hshift.take())
        } else {
            None
        };
        self.groups.push(GroupCtx {
            kind: gkind,
            box_kind,
            shipout: ship,
            leaders,
            setbox,
            setbox_global,
            spec,
            shift,
            entered_line: line,
            // 组打开时是否数学模式：`\hbox{A}` 数学字段（box 原子）在数学模式
            // 打开、同模式关闭是合法流程；外层组（垂直打开）关闭时若仍处数学
            // 模式才是缺 `$`（etrip L1148 `$\pagediscards}`）。
            entered_math: matches!(self.mode(), Mode::Math | Mode::DisplayMath),
            level: (self.groups.len() + 1) as u32,
        });
        // \tracinggroups（misc 6）：组进入追踪（tex.web begin_group）
        if self.params.misc[6] > 0 {
            let name = gkind.group_name();
            let level = self.groups.len();
            let _ = self.write16(format!(
                "{{entering {name} (level {level}) at line {line}}}\n"
            ));
        }
        self.param_stack.push(self.params);
        // `\parshape` 组作用域（tex.web：par_shape_ptr 属组局部量）——LaTeX
        // `\list` 在环境组内设形状，`\end{quotation}` 后必须还原，否则缩进泄漏
        self.parshape_stack.push(self.parshape.clone());
        // 字体选择组作用域：组开始保存当前字体，组结束恢复
        self.font_stack.push(self.current_font);
        if let Some(k) = box_kind {
            let new_mode = match k {
                PendingBox::HBox => Mode::RestrictedHorizontal,
                PendingBox::VBox | PendingBox::VTop => Mode::Vertical,
            };
            self.lists.push(Vec::new());
            self.list_modes.push(new_mode);
            // \hbox 内容从 spacefactor=1000 开始（tex.web：进入受限水平模式重置）
            if new_mode == Mode::RestrictedHorizontal {
                self.space_factor = 1000;
            }
        } else if gkind == GroupKind::Math {
            // 数学组：`{...}`（含脚本/根式/定类字段）压 math 层。
            // `\begingroup`（SemiSimple）等显式组不压 math 层（tex.web math_group 语义）。
            let field = if let Some(is_sup) = self.math_state.pending_script.take() {
                Some(MathFieldKind::Script(is_sup))
            } else if self.math_state.sqrt_pending {
                self.math_state.sqrt_pending = false;
                Some(MathFieldKind::Sqrt)
            } else if let Some(d) = self.math_state.radical_pending.take() {
                Some(MathFieldKind::Radical(d))
            } else if self.math_state.accent_pending {
                self.math_state.accent_pending = false;
                Some(MathFieldKind::Accent)
            } else if self.math_state.underline_pending {
                self.math_state.underline_pending = false;
                Some(MathFieldKind::Underline)
            } else if self.math_state.overline_pending {
                self.math_state.overline_pending = false;
                Some(MathFieldKind::Overline)
            } else {
                self.math_state.class_pending.take().map(MathFieldKind::Class)
            };
            self.math_state.math.push(MathLevel {
                atoms: Vec::new(),
                field,
                left: None,
                fraction: None,
            });
        }
        Ok(())
    }
    fn group_end(&mut self) -> Result<()> {
        let ctx = self
            .groups
            .pop()
            .ok_or_else(|| Error::internal("group_end 无配对 group_begin"))?;
        // \setbox 组作用域：回滚本组（及更深组）内设置的盒子寄存器
        // （TeX 寄存器组级保存——trip L317 组内 \setbox22 → restoring \box22=void）
        let cur_level = self.groups.len();
        while self
            .box_state.box_saves
            .last()
            .is_some_and(|(lvl, _, _)| *lvl > cur_level)
        {
            let (_, idx, old) = self.box_state.box_saves.pop().expect("last 已检查");
            // 经 write_box 路由：寄存器 255 的回滚落在页面队列队首（trip.tex
            // 第二例程 `\setbox255\copy255` ——「at end of group, \box255 reverts
            // to former value」tex.web eq_restore 语义）
            self.write_box(idx, old);
        }
        // 数学模式关**非数学模式打开的、非 box 的**组：TeX 报 Missing $ inserted
        // 并先关数学再关组（tex.web：数学模式的组结束 → 插 $ 结束数学；etrip
        // L1148 `$\pagediscards}` 的 `}`——恢复后 `}` 关外层 vbox 组）。
        // **排除 MathLeft（16）**：`\right`/`\middle` 关闭 math left group 是
        // 数学模式的正常流程（trip L256 `$\right...`），不是缺 $。
        // **排除数学模式打开的组**（entered_math）：`\hbox{A}` 数学字段
        // （box 原子）同模式关闭合法。
        // **排除 box 组**（box_kind）：`\hbox{\special{...}}` 等 box 内容在
        // 数学模式关闭是 box 原子场景（trip L288），不报缺 $——否则 close_math
        // 提前清空数学层导致后续 Math 组 "无外层 math 层"（Engine error）。
        if ctx.box_kind.is_none()
            && !ctx.entered_math
            && !matches!(ctx.kind, GroupKind::Math | GroupKind::MathLeft)
            && matches!(self.mode(), Mode::Math | Mode::DisplayMath)
        {
            self.report_error("Missing $ inserted.");
            let was_display = self.mode() == Mode::DisplayMath;
            let _ = self.close_math();
            if !was_display && self.mode() == Mode::Horizontal {
                self.close_paragraph();
            }
        }
        // 垂直盒子内容结束时，开放段落先封装（\vbox{a} → vbox[hbox(a)]）。
        // **必须在参数恢复之前**（TeX 语义：段落折行用组内 \hsize——etrip
        // L193 `\vbox{\hsize=0pt...}` 折行用 0pt；恢复后折行会错用外层
        // hsize=469.75pt，行宽/断点全错）。
        if ctx.box_kind.is_some_and(PendingBox::is_vertical) && self.mode() == Mode::Horizontal {
            self.close_paragraph();
        }
        // 盒组封装的 boxmaxdepth 用**组内**值（tex.web：vbox 封装在组恢复前——
        // 与 L635-641 折行用组内 hsize 同理；trip L316 `\vbox to10pt{\boxmaxdepth
        // =-1pt...}` → 恢复后取外层会丢 -1pt 钳制）
        let inner_boxmaxdepth = self.params.boxmaxdepth;
        // 先恢复参数镜像（与 VM 的 save_stack 恢复对齐），随后的缩进/interline 用外层值
        if let Some(prev) = self.param_stack.pop() {
            self.params = prev;
        }
        // `\parshape` 随组恢复（组开始时快照；与参数镜像同序）
        if let Some(p) = self.parshape_stack.pop() {
            self.parshape = p;
        }
        // 字体选择组作用域：组结束恢复进入时的字体（tex.web：cur_font 随组保存）
        if let Some(f) = self.font_stack.pop() {
            self.current_font = f;
        }
        // \tracinggroups（misc 6）：组离开追踪（tex.web end_group；用进入行号）
        if self.params.misc[6] > 0 {
            let _ = self.write16(format!(
                "{{leaving {} (level {}) entered at line {}}}\n",
                ctx.kind.group_name(),
                ctx.level,
                ctx.entered_line
            ));
        }
        // 数学组：内容并入外层（普通组）或作为字段挂到外层 base（^/_ 后组等）。
        // 仅 `{` 数学组（GroupKind::Math）push/pop math 层；`\begingroup`（SemiSimple）
        // 等显式组不新建数学列表（tex.web math_group 语义，TRIP L438 \begingroup）。
        if ctx.kind == GroupKind::Math {
            // tex.web 组与数学层在 save_stack 上同构（math_group 弹出时其数学
            // 层必在栈顶）；本引擎双栈（groups/math）在"强制收数学"恢复
            // （\par 的 Missing $ inserted 等）后会残留迟到的数学组——TRIP
            // L281 `{\above9pt{...` 悬挂组到 L291 `}` 才关闭，此时 math 层
            // 已被收走/只剩一层。真实 TeX 在该处组已平衡，`}` 会被拒为
            // "Extra }, or forgotten \endgroup." 并删除——此处降级对齐：报
            // 同款错误、只收组不并原子，绝不内部致命（任意畸形输入可恢复）。
            let Some(level) = self.math_state.math.pop() else {
                self.report_error("Extra }, or forgotten \\endgroup.");
                return Ok(());
            };
            let Some(parent) = self.math_state.math.last_mut() else {
                self.report_error("Extra }, or forgotten \\endgroup.");
                return Ok(());
            };
            let field_atoms = match level.field {
                Some(MathFieldKind::Script(is_sup)) => {
                    // `x^{...}`：先收组内分式（`x^{a\over b}`），再作为脚本字段挂载
                    let mut lv = level;
                    Self::math_finish_fraction(&mut lv);
                    let field = lv.atoms;
                    if !field.is_empty() {
                        // `x^{}`：空字段合法（TeX 空组字段）
                        Self::math_attach_script(parent, is_sup, field)?;
                    }
                    return Ok(());
                }
                Some(MathFieldKind::Sqrt) => {
                    // `\sqrt{...}`：先收组内分式（`\sqrt{a\over b}`），再作 radicand
                    let mut lv = level;
                    Self::math_finish_fraction(&mut lv);
                    parent.atoms.push(MathAtom::Radical { base: lv.atoms });
                    return Ok(());
                }
                Some(MathFieldKind::Underline) => {
                    // `\underline{...}`：组内容收为 Underline 原子（tex.web math_ac）
                    let mut lv = level;
                    Self::math_finish_fraction(&mut lv);
                    parent
                        .atoms
                        .push(MathAtom::Underline { base: lv.atoms });
                    return Ok(());
                }
                Some(MathFieldKind::Overline) => {
                    // `\overline{...}`：组内容收为 Overline 原子
                    let mut lv = level;
                    Self::math_finish_fraction(&mut lv);
                    parent
                        .atoms
                        .push(MathAtom::Overline { base: lv.atoms });
                    return Ok(());
                }
                Some(MathFieldKind::Radical(delim)) => {
                    // `\radical<delim>{...}`：radicand 同 \sqrt（定界符号暂不参与渲染）
                    let mut lv = level;
                    Self::math_finish_fraction(&mut lv);
                    parent
                        .atoms
                        .push(MathAtom::Radical { base: lv.atoms });
                    let _ = delim;
                    return Ok(());
                }
                Some(MathFieldKind::Class(class)) => {
                    // `\mathbin{...}`：内容作为一个指定类原子
                    let mut lv = level;
                    Self::math_finish_fraction(&mut lv);
                    parent.atoms.push(MathAtom::Classed {
                        class,
                        content: lv.atoms,
                    });
                    return Ok(());
                }
                Some(MathFieldKind::Accent) => {
                    // `\accent<15-bit>{...}`：组内容为 nucleus（被重音）字段，
                    // 与已扫描的重音符字段合并（tex.web math_ac）。
                    let mut lv = level;
                    Self::math_finish_fraction(&mut lv);
                    let accent = parent.atoms.pop().map_or(Vec::new(), |a| vec![a]);
                    parent.atoms.push(MathAtom::Accent {
                        accent,
                        nucleus: lv.atoms,
                    });
                    return Ok(());
                }
                None => {
                    // 普通数学组（tex.web math_group）：组内容存为 Ord noad 的
                    // sub_mlist 核（`{` 开组时 tail 是新鲜 Ord noad，故类恒 Ord，
                    // `{+}` 亦然）；mlist_to_hlist 对 sub_mlist 核 hpack(natural)
                    // （L14848），`^{}` 挂组整体而非末原子。花括号消除特例：
                    // 组恰为一个空脚本的 Ord noad 时核直接取代组（L22339）——
                    // 单个 Ord 字符/嵌套 Ord 组解包，其余（Bin/Frac/Scripts…）
                    // 保持装箱。
                    let mut lv = level;
                    Self::math_finish_fraction(&mut lv);
                    vec![Self::plain_group_atom(lv.atoms)]
                }
            };
            parent.atoms.extend(field_atoms);
            return Ok(());
        }
        match ctx.kind {
            // M4-5 noalign 组：材料原样入对齐流（组列表在 group_begin 开启，
            // tex.web no_align 材料直接进对齐 vlist；不封装盒）
            GroupKind::NoAlign => {
                let nodes = self.lists.pop().unwrap_or_default();
                self.list_modes.pop();
                if let Some((_, ctx)) = self.align_stack.last_mut() {
                    ctx.stream.push(AlignItem::Material(nodes));
                }
            }
            // M4-5 对齐组（tex.web fin_align）：两遍法统一列宽 + 封装
            GroupKind::Align => {
                let nodes = match self.align_stack.pop() {
                    Some((dir, ctx)) => align_fin(self.params, dir, ctx),
                    None => Vec::new(),
                };
                // 对齐组列表（group_begin 的 VBox 路径）弹出——行/材料已走 ctx 流
                self.lists.pop();
                self.list_modes.pop();
                if let Some(outer) = self.lists.last_mut() {
                    outer.extend(nodes);
                }
            }
            GroupKind::HBox | GroupKind::AdjustedHBox | GroupKind::VBox | GroupKind::VTop => {
                if let Some(kind) = ctx.box_kind {
                    let leaders = ctx.leaders;
                    // 归还本组认领的 to/spread 规格，供 package_box 取用
                    // （内层嵌套盒的规格已各自消费，此处放回的是外层的）
                    self.box_state.pending_box_spec = ctx.spec;
                    self.package_box(
                        kind,
                        ctx.shipout,
                        leaders,
                        ctx.setbox.map(|idx| (idx, ctx.setbox_global)),
                        ctx.shift,
                        inner_boxmaxdepth,
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn primitive(&mut self, prim: Primitive) -> Result<()> {
        // TeX box_end leader 分支：盒子后必须是 \hskip/\vskip 胶水，否则
        // "Leaders not followed by proper glue" 报错并丢弃引导盒子。
        if self.box_state.leaders_box.is_some() {
            self.report_leaders_misplaced();
        }
        // tex.web scan_math：待定数学字段遇原语事件（非字符非 {）→ Missing { inserted
        self.check_math_field_break()?;
        match prim {
            Primitive::HBox => self.box_state.pending_box = Some(PendingBox::HBox),
            Primitive::VBox => self.box_state.pending_box = Some(PendingBox::VBox),
            Primitive::VTop => self.box_state.pending_box = Some(PendingBox::VTop),
            // TRIP 冲刺：\leaders/\cleaders/\xleaders —— 引导符，等待其后的盒子
            // （tex.web scan_box(leader_flag+kind)；盒子经 group/rule 路径挂起）。
            Primitive::Leaders => self.box_state.pending_leaders = Some(LeadersKind::Leaders),
            Primitive::Cleaders => self.box_state.pending_leaders = Some(LeadersKind::Cleaders),
            Primitive::XLeaders => self.box_state.pending_leaders = Some(LeadersKind::Xleaders),
            Primitive::Par => {
                match self.mode() {
                    Mode::Horizontal => {
                        self.close_paragraph();
                    }
                    // 垂直模式 \par 无操作；受限水平/数学模式拒绝（TeX 报错恢复，TRIP L210）
                    Mode::Vertical => {}
                    Mode::RestrictedHorizontal => {
                        self.write16(
                            "! You can't use \\par in restricted horizontal mode.\n".to_string(),
                        )?;
                    }
                    Mode::Math | Mode::DisplayMath => {
                        // TeX：数学模式 \par → 报 "Missing $ inserted" 并关数学（当 \par 处理）
                        self.report_error("Missing $ inserted.");
                        let was_display = self.mode() == Mode::DisplayMath;
                        let _ = self.close_math();
                        // 显示数学的公式盒已并入外层垂直列表（TeX 中显示公式不在段落内），
                        // 无需再关段落；行内数学结束时若仍在段落（水平模式）则关闭所在段落
                        // （TRIP L350 `$$` 未闭合段末 \par）。注意：垂直模式开启的行内数学
                        // （`$...$\par`）close_math 后模式已退回 Vertical——此时再关段落会把
                        // 唯一的主列表也弹出（fuzz 命中"列表栈非空"panic，畸形输入不 panic 契约）。
                        if !was_display && self.mode() == Mode::Horizontal {
                            self.close_paragraph();
                        }
                    }
                }
            },
            Primitive::Indent => match self.mode() {
                Mode::Vertical => {
                    // 垂直模式 \indent 强制开段并缩进（TeX：new_graf）
                    self.lists.push(Vec::new());
                    self.list_modes.push(Mode::Horizontal);
                    self.insert_indent();
                }
                Mode::Horizontal | Mode::RestrictedHorizontal => self.insert_indent(),
                Mode::Math | Mode::DisplayMath => {}
            },
            // \␣（ex_space）：tex.web 独立命令码，与 spacer 分路径。
            // hmode/mmode+ex_space → 无条件 append_normal_space（L20054）；
            // vmode+ex_space → back_input + new_graf(true) 起段后重执行（L21107）。
            Primitive::ControlSpace => match self.mode() {
                Mode::Vertical => {
                    self.par_begin(true)?;
                    self.append_normal_space();
                }
                Mode::Horizontal | Mode::RestrictedHorizontal => self.append_normal_space(),
                Mode::Math | Mode::DisplayMath => {
                    // 数学表的胶水是普通（pt）胶水：mlist_to_hlist 只换算
                    // mu_glue（tex.web L14400），原样落 hlist
                    // （GT `\hbox{$A\ B$}` → `.\glue 3.33333 plus …`）。
                    // `\spaceskip` 名标签随 MathAtom::MSkip 无名字段丢弃（宽度不丢）。
                    let (w, s, k, _) = self.normal_space_glue();
                    self.math_push_atom(MathAtom::MSkip {
                        width: w,
                        stretch: s,
                        shrink: k,
                        nonscript: false,
                    })?;
                }
            },
            Primitive::NoIndent => {
                // 垂直模式 \noindent 立即开段（TeX new_graf(0)，缩进 0）——
                // 不只设标志等字符触发：`\vbox{\noindent\hbox{...}}` 的 \hbox
                // 必须在水平模式（否则盒被当 AdjustedHBox 进垂直列表、模式错乱）；
                // 水平模式无操作；数学模式报 Missing $ inserted 并关数学恢复
                // （tex.web：水平命令在数学模式 → 插 $ 结束数学，命令继续执行；
                // etrip L1148 `\noindent$\splitdiscards\noindent$...` 第一处）。
                if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
                    self.report_error("Missing $ inserted.");
                    let was_display = self.mode() == Mode::DisplayMath;
                    let _ = self.close_math();
                    if !was_display && self.mode() == Mode::Horizontal {
                        self.close_paragraph();
                    }
                    // 数学结束后 \noindent 在垂直模式开段（缩进 0）
                    if self.mode() == Mode::Vertical {
                        self.noindent_next = true;
                        self.lists.push(Vec::new());
                        self.list_modes.push(Mode::Horizontal);
                        self.space_factor = 1000;
                        self.insert_indent(); // noindent_next=true → 跳过缩进盒
                    }
                } else if self.mode() == Mode::Vertical {
                    self.noindent_next = true;
                    self.lists.push(Vec::new());
                    self.list_modes.push(Mode::Horizontal);
                    self.space_factor = 1000;
                    self.insert_indent(); // noindent_next=true → 跳过缩进盒
                }
            }
            // M3-5：\shipout 后的下一个盒子封装为页面
            Primitive::ShipOut => self.page_state.shipout_next = true,
            // M4-2：\nonscript 使下一个数学空格在脚本模式丢弃
            Primitive::Nonscript => {
                if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
                    self.math_state.nonscript_pending = true;
                }
            }
            // M4：\mathaccent（\accent 的数学等价，已报错改道）：记录待定重音符
            // 字段，nucleus 字段由后续 token/组补齐（tex.web math_ac）。
            Primitive::MathAccent if matches!(self.mode(), Mode::Math | Mode::DisplayMath) => {
                self.math_state.accent_pending = true;
            }
            // TRIP 冲刺：\accent 在数学模式报错恢复（TRIP L396）
            Primitive::Accent => {
                if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
                    self.write16(
                        "! Please use \\mathaccent for accents in math mode.\n".to_string(),
                    )?;
                    self.math_state.accent_pending = true;
                }
            }
            // TeX \error 原语（tex.web @<Report an improper...@> /
            // error 命令：报告 "! OK." 并进入错误交互——nonstop/batch 模式下
            // 仅转录后继续；trip.tex 多处 \error 依赖此消息，参考 log 14 处）。
            Primitive::Error => {
                let mode = self.params.misc[19]; // interactionmode（0=batch 1=nonstop 2=scroll 3=errorstop）
                self.io_state.transcript.push_str("! OK.\n");
                if mode >= 2 {
                    self.io_state.transcript.push_str("(Please type a command or say \\end)\n");
                }
                if mode == 0 || mode == 1 {
                    self.io_state.transcript.push_str(
                        "This error message was issued in nonstop or batch mode,\n\
                         so I can't continue interacting with you.\n",
                    );
                }
            }
            // 参数扫描型原语经 glue/kern/penalty/rule 事件处理
            _ => {}
        }
        Ok(())
    }
    fn paragraph_line(&mut self, line: i64) -> Result<()> {
        // 记录 \\par 的源码行号：折行警告 `in paragraph at lines a--b` 的结束行
        self.last_par_line = line;
        Ok(())
    }
    fn glue(&mut self, g: Glue) -> Result<()> {
        // `\leaders` 引导盒子已就位：\hskip/\vskip 胶水到来 → 组成 Leader 节点
        // （tex.web box_end leader 分支：append_glue + subtype + leader_ptr）。
        if let Some((kind, box_node)) = self.box_state.leaders_box.take() {
            self.append(Node::Leaders {
                kind,
                inner: Box::new(box_node),
                width: g.width,
                stretch: g.stretch,
                shrink: g.shrink,
            });
            return Ok(());
        }
        // 数学模式 `\hskip`：转数学空格原子（TeX 数学模式 \hskip ≡ \mskip）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            self.math_push_atom(MathAtom::MSkip {
                width: g.width,
                stretch: g.stretch,
                shrink: g.shrink,
                nonscript: false,
            })?;
            return Ok(());
        }
        // 胶水阶必须透传：`skip 0pt plus .0001fil`（LaTeX \@textbottom）丢阶后
        // 变 0.0001pt 有限拉伸，页盒 `\vbox to\@colht` 的胶水设置把拉伸全摊到
        // `\parskip` 等有限拉伸胶上（tex.web scan_glue：plus/minus 各带 glue_ord）。
        self.append(Node::Glue {
            name: None,
            width: g.width,
            stretch: g.stretch,
            shrink: g.shrink,
            stretch_order: g.stretch_order,
            shrink_order: g.shrink_order,
        });
        Ok(())
    }
    fn kern(&mut self, width: i64) -> Result<()> {
        // 数学模式 `\kern`：转数学空格原子（TeX 数学模式 \kern ≡ \mkern）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            self.math_push_atom(MathAtom::MSkip {
                width,
                stretch: 0,
                shrink: 0,
                nonscript: false,
            })?;
            return Ok(());
        }
        self.append(Node::Kern { width });
        Ok(())
    }
    fn penalty(&mut self, penalty: i64) -> Result<()> {
        // 数学模式 `\penalty`：数学列表断行点原子（M4-1——tex.web math list
        // 的 penalty 节点；此前忽略导致公式内断行点缺失）
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_push_atom(MathAtom::Penalty { penalty });
        }
        self.append(Node::Penalty { penalty });
        Ok(())
    }
    fn rule(&mut self, width: i64, height: i64, depth: i64) -> Result<()> {
        // `\leaders\hrule/\vrule`：rule 作引导内容（tex.web scan_box leader 分支
        // 允许 hrule/vrule；宽度保持 NULL_FLAG，showbox 显示 `x*`）。
        if let Some(kind) = self.box_state.pending_leaders.take() {
            self.box_state.leaders_box = Some((kind, Node::Rule { width, height, depth }));
            return Ok(());
        }
        // 数学模式 `\vrule`：数学列表规则原子（M4-1——此前忽略）
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            let width = if width == ntex_core::NULL_FLAG { 0 } else { width };
            return self.math_push_atom(MathAtom::Rule { width, height, depth });
        }
        // 非引导上下文：hrule 未定宽度保持 null 哨兵（tex.web scan_rule_spec
        // L9126 起——`\hrule` 扫描期只定 height=0.4pt/depth=0，width 留
        // null_flag；出货时 `rule_wd:=width(this_box)` 解析为包含盒宽，
        // L12598）。此前简化落 0，DVI/渲染按 width>0 判定跳过不画，
        // `\hrule` 整体消失（resume-plain.tex 节标题横线丢失的根因）。
        self.append(Node::Rule {
            width,
            height,
            depth,
        });
        Ok(())
    }
    fn param_changed(&mut self, kind: ParamKind, value: ParamValue) -> Result<()> {
        self.params.set(kind, value);
        Ok(())
    }
    fn penalty_array_changed(&mut self, kind: u8, values: &[i64]) -> Result<()> {
        if let Some(slot) = self.penalty_arrays.get_mut(kind as usize) {
            *slot = values.to_vec();
        }
        Ok(())
    }
    fn sfcode_changed(&mut self, charcode: u8, value: u32) -> Result<()> {
        self.sfcodes[charcode as usize] = value;
        Ok(())
    }
    /// `\discretionary{pre}{post}{replace}`：断字节点。组内容 token 中字符
    /// 转字符节点（当前字体，维度查字体表），其余忽略（简化）。
    fn discretionary(
        &mut self,
        pre: Vec<Token>,
        post: Vec<Token>,
        replace: Vec<Token>,
    ) -> Result<()> {
        let conv = |toks: &[Token]| -> Vec<Node> {
            toks.iter()
                .filter_map(|t| {
                    t.charcode().map(|c| {
                        let (w, h, d) = self.fonts.metrics(self.current_font, c);
                        Node::Char {
                            font: self.current_font,
                            charcode: c,
                            width: w,
                            height: h,
                            depth: d,
                        }
                    })
                })
                .collect()
        };
        self.append(Node::Discretionary {
            pre: conv(&pre),
            post: conv(&post),
            replace: conv(&replace),
        });
        Ok(())
    }
    /// TeXXeT 方向节点：追加到当前列表（宽度 0 占位）。
    fn direction_node(&mut self, kind: ntex_core::sink::DirectionKind) -> Result<()> {
        self.append(Node::Direction { kind });
        Ok(())
    }
    /// `\begingroup`：下一个组为半简单组（14）。
    fn semisimple_begin(&mut self) -> Result<()> {
        self.box_state.pending_kind = Some(GroupKind::SemiSimple);
        Ok(())
    }
    /// 当前模式名（`\tracingcommands` 追踪；tex.web print_mode 语义）。
    fn mode_name(&self) -> String {
        match self.mode() {
            // tex.web shown_mode：\vbox/\vtop 内容为 internal vertical mode
            // （顶层垂直列表才是 vertical mode）——\tracingcommands 追踪对齐
            Mode::Vertical
                if self.groups.iter().any(|g| {
                    matches!(
                        g.box_kind,
                        Some(crate::typeset::PendingBox::VBox | crate::typeset::PendingBox::VTop)
                    )
                }) =>
            {
                "internal vertical mode".to_string()
            }
            Mode::Vertical => "vertical mode".to_string(),
            Mode::Horizontal => "horizontal mode".to_string(),
            Mode::RestrictedHorizontal => "restricted horizontal mode".to_string(),
            Mode::Math => "math mode".to_string(),
            Mode::DisplayMath => "display math mode".to_string(),
        }
    }
    /// 当前模式码（TeX 模式码：1=垂直、2=水平、3=数学、4=内层垂直、
    /// 5=受限水平、6=显示数学）。
    fn mode_code(&self) -> i64 {
        match self.mode() {
            Mode::Vertical => {
                if self.lists.len() > 1 {
                    4 // 内层垂直（vbox/vtop/vcenter 内容）
                } else {
                    1
                }
            }
            Mode::Horizontal => 2,
            Mode::RestrictedHorizontal => 5,
            Mode::Math => 3,
            Mode::DisplayMath => 6,
        }
    }
    /// 段落即将开始（tex.web `new_graf` 前置查询）：垂直模式且非显示公式续排。
    /// Expander 据此先开段、注入 `\everypar`，再回放触发 token。
    fn par_begin_imminent(&self) -> bool {
        self.mode() == Mode::Vertical && !self.math_state.after_display
    }
    /// `\parshape` 镜像推送（`\list`/quotation 两侧缩进的唯一机制）。
    fn set_parshape(&mut self, shape: &[(i64, i64)]) {
        self.parshape = shape.to_vec();
    }
    /// 开段（tex.web `new_graf`）：`\parskip`（空页上被页面构建器丢弃）→
    /// 新水平列表 → spacefactor 复位 → 缩进盒。
    fn par_begin(&mut self, indented: bool) -> Result<()> {
        if self.page_state.pagination {
            let ps = self.params.parskip;
            self.append(Node::Glue {
            name: None,                            width: ps.width,
                            stretch: ps.stretch,
                            shrink: ps.shrink,
                            stretch_order: ps.stretch_order,
                            shrink_order: ps.shrink_order,
            });
        }
        self.lists.push(Vec::new());
        self.list_modes.push(Mode::Horizontal);
        self.space_factor = 1000; // new_graf：段落开始重置 spacefactor
        if indented {
            self.insert_indent();
        } else {
            // `\noindent` 语义：Expander 拦截路径不经过 NoIndent 原语臂，
            // 在此消费陈旧的 noindent 标志（不落缩进盒）。
            self.noindent_next = false;
        }
        Ok(())
    }
    /// e-TeX `\currentgrouptype`：当前组类型码。
    /// 数学模式：顶组为数学组 → 9，`\left`/`\middle` 组 → 16（math left group），
    /// 否则为 `$` 进入的数学移位组 → 15。
    fn current_group_type(&self) -> i64 {
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return match self.groups.last() {
                Some(g) if g.kind == GroupKind::Math => 9,
                Some(g) if g.kind == GroupKind::MathLeft => 16,
                _ => 15,
            };
        }
        match self.groups.last() {
            Some(g) => g.kind.code(),
            None => 0,
        }
    }
    /// e-TeX `\ifinner`：内部模式为真 —— 行内数学、受限水平、内部垂直；
    /// 外层水平（段落）、主垂直列表、显示数学为假（TeXbook p.209）。
    fn if_inner(&self) -> bool {
        match self.mode() {
            Mode::Math | Mode::RestrictedHorizontal => true,
            Mode::DisplayMath | Mode::Horizontal => false,
            // 垂直模式：主列表（lists[0]）为外层，盒内（vbox/vtop/vcenter）为内部
            Mode::Vertical => self.lists.len() > 1,
        }
    }
    fn transcript(&self) -> &str {
        &self.io_state.transcript
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn as_any_ref(&self) -> &dyn std::any::Any {
        self
    }
}

impl FontSink for NodeBuilder {
    fn font_selected(&mut self, font: u32) -> Result<()> {
        // fn 指针模式恒为 FontId(0)；TFM 模式更新当前字体
        self.current_font = FontId(font);
        Ok(())
    }
    fn font_defined(&mut self, font: u32, cs_name: &str) -> Result<()> {
        if self.font_cs_names.len() <= font as usize {
            self.font_cs_names.resize(font as usize + 1, None);
        }
        self.font_cs_names[font as usize] = Some(cs_name.to_string());
        Ok(())
    }
    fn current_font(&self) -> u32 {
        self.current_font.0
    }
    /// `\spacefactor` 实时查询（活参数：随字符/句号由排版器调整）。
    fn space_factor(&self) -> i64 {
        self.space_factor
    }
    /// `\spacefactor=<number>` 赋值（组作用域恢复由 save/restore 处理）。
    /// TeX 只允许 1..32767：越界 `int_error("Bad space factor")` 报错恢复、不赋值
    /// （tex.web L23220-23228；TRIP L289 `\showbox0\spacefactor=0`——否则后续
    ///  `app_space` 中 `xn_over_d(shrink, 1000, sf)` 除零）。
    fn set_space_factor(&mut self, v: i64) -> Result<()> {
        if !(1..=32767).contains(&v) {
            // 参考 log 格式（int_error 带值 + help 行，TRIP L289）
            let _ = self.write16(format!(
                "! Bad space factor ({}).\n\
                 I allow only values in the range 1..32767 here.\n",
                v
            ));
        } else {
            self.space_factor = v;
        }
        Ok(())
    }
    /// `\/`：斜体校正（水平模式发 kern / 数学模式斜体校正原子 / 垂直模式报错）。
    fn italic_correction(&mut self) -> Result<()> {
        // 数学模式：斜体校正原子（当前无字体斜体校正度量，宽度 0）
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_push_atom(MathAtom::MSkip {
                width: 0,
                stretch: 0,
                shrink: 0,
                nonscript: false,
            });
        }
        // 垂直模式：TeX 报 "You can't use `\/' in vertical mode"
        if matches!(self.mode(), Mode::Vertical) {
            return self.math_mode_error("/");
        }
        // 水平/受限水平：斜体校正 kern（无字体度量数据，宽度 0 不输出；TeX 同理）
        self.append(Node::Kern { width: 0 });
        Ok(())
    }
}

impl MathSink for NodeBuilder {
    /// 数学移位（`$`，cat 3）：VM 已 peek 出 `display`（连续 `$$`）。
    /// - Math：结束行内公式；
    /// - DisplayMath：`$$` 结束显示公式，单 `$` 报错（TeX "Display math should end with $$"）；
    /// - 非数学模式：display → 显示数学（M4-4：收尾段落/开段，公式作垂直元素），否则行内数学。
    fn math_shift(&mut self, display: bool) -> Result<()> {
        match self.mode() {
            Mode::Math => {
                if display {
                    // 数学模式内 `$$`（如行内数学未关时紧接的 `$$`）：tex.web
                    // 报 Missing $ inserted + 结束当前数学 + 开显示数学
                    // （expander 已消费第二个 `$`，这里连续切换；参考 trip
                    // L261 前 `$\x` 残留场景）。受限水平（\halign 模板等）下
                    // 数学内 $$ 结束后续接**普通**数学（tex.web mode<0 语义，
                    // sink 受限水平分支同款）。
                    self.report_error("Missing $ inserted.");
                    self.close_math()?;
                    if self.mode() == Mode::RestrictedHorizontal {
                        self.enter_math(Mode::Math)
                    } else {
                        self.enter_display_math()
                    }
                } else {
                    self.close_math()
                }
            }
            Mode::DisplayMath => {
                if display {
                    self.close_math()
                } else {
                    // TeX：单 `$` 结束显示数学 → 报错但恢复（该 `$` 按 `$$` 处理，
                    // 关闭公式；TRIP L206 `$$\eqno^{}$`）。
                    self.report_error("Display math should end with $$.");
                    self.close_math()
                }
            }
            Mode::Vertical => {
                if display {
                    // M4-4 显示数学：垂直模式 = TeX new_graf 开段（parskip），公式作段首
                    // 垂直元素（predisplaypenalty + abovedisplayskip + 公式盒 + 下间距）。
                    // 垂直列表为空（文档开头）时不加 parskip。
                    if self.page_state.pagination && !self.lists.last().is_some_and(Vec::is_empty) {
                        let ps = self.params.parskip;
                        self.append(Node::Glue {
            name: None,                            width: ps.width,
                            stretch: ps.stretch,
                            shrink: ps.shrink,
                            stretch_order: ps.stretch_order,
                            shrink_order: ps.shrink_order,
                        });
                    }
                    // tex.web：段首 `$$` 走 head=tail 臂（`\noindent$$`），w := -max_dimen
                    // → close_math 裁决取长 skip。
                    self.math_state.predisplay_size = -ntex_core::register::MAX_DIMEN;
                    self.enter_display_math()
                } else {
                    // 行内数学：开段（TeX new_graf）
                    if self.page_state.pagination {
                        let ps = self.params.parskip;
                        self.append(Node::Glue {
            name: None,                            width: ps.width,
                            stretch: ps.stretch,
                            shrink: ps.shrink,
                            stretch_order: ps.stretch_order,
                            shrink_order: ps.shrink_order,
                        });
                    }
                    self.lists.push(Vec::new());
                    self.list_modes.push(Mode::Horizontal);
                    self.space_factor = 1000; // new_graf：段落开始重置 spacefactor
                    self.insert_indent();
                    self.enter_math(Mode::Math)
                }
            }
            Mode::Horizontal => {
                if display {
                    // M4-4 显示数学：TeX $$ 在水平模式先 \par 收尾段落，公式作垂直元素。
                    // pre_display_size（tex.web §1193）：末行非空 = 2em + 末行可见材料
                    // 自然宽（close_paragraph 返回值即末行自然宽，\parfillskip 自然宽 0）；
                    // 列表为空 = -max_dimen。长/短 skip 的裁决在 close_math 退出时做
                    // （那时公式自然宽才可知）。
                    let last_natural = self.close_paragraph();
                    self.math_state.predisplay_size = match last_natural {
                        Some(w) => w + 2 * self.fonts.font_param(self.current_font, 6),
                        None => -ntex_core::register::MAX_DIMEN,
                    };
                    self.enter_display_math()
                } else {
                    self.enter_math(Mode::Math)
                }
            }
            Mode::RestrictedHorizontal => {
                // tex.web init_math：`$$` 只有在 mode>0 时才进入显示数学（
                // `if (cur_cmd=math_shift) and (mode>0)`）。受限水平模式的
                // mode=-hmode<0，故两个 $ 各自进出**普通**数学——不报任何错。
                // 此前 NTex 自创 "Display math in restricted mode." 并在进入
                // 行内数学后丢失第二个 $，导致组/模式错位（TRIP L210
                // `\hbox{$$}$\par}` 之后整段落在受限水平模式）。
                let _ = display;
                self.enter_math(Mode::Math)
            }
        }
    }
    /// tex.web init_math `mode>0`：`$$` 进显示数学仅限垂直/普通水平模式；
    /// 受限水平（\hbox/\halign 模板）下第二个 `$` 由 VM back_input，`$$`
    /// 退化为两次独立进出（TRIP L210/L340）。
    fn math_display_allowed(&self) -> bool {
        matches!(self.mode(), Mode::Vertical | Mode::Horizontal)
    }
    /// tex.web after_math：只有**显示**数学收尾要求配对 `$`（无则报
    /// "Display math should end with $$" 照收）；行内数学收尾不 peek——
    /// 紧随的 `$` 落回水平/垂直模式由 init_math 重新判定。
    fn math_close_consumes_dollar(&self) -> bool {
        matches!(self.mode(), Mode::DisplayMath)
    }
    /// 数学样式原语：数学模式内 push 样式原子（影响后续字阶与 spacing）。
    fn math_style(&mut self, style: u8) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            // tex.web math_style：非数学模式报错恢复（样式忽略）——
            // 与 math_class 同族（原为"简化忽略"，报错缺失已补）
            let name = match style {
                0 => "displaystyle",
                1 => "textstyle",
                2 => "scriptstyle",
                _ => "scriptscriptstyle",
            };
            return self.math_mode_error(name);
        }
        let s = match style {
            0 => MathStyle::Display,
            1 => MathStyle::Text,
            2 => MathStyle::Script,
            _ => MathStyle::ScriptScript,
        };
        self.math_push_atom(MathAtom::Style(s))
    }
    /// `\over`/`\atop`/`\above`：numerator 已收集（当前 math 层），等待 denominator。
    fn math_fraction(&mut self, thickness: Option<i64>) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_mode_error("over");
        }
        let level = self
            .math_state.math
            .last_mut()
            .ok_or_else(|| Error::internal("\\over 无数学层"))?;
        if level.fraction.is_some() {
            // TeX 恢复式（参考 trip l.257 `\left.A\over A\abovewithdelims.?\right(`：
            // 同层已有 fraction 时报 Ambiguous 并**丢弃新 fraction 命令**，
            // 保持原 fraction——不中断）。
            self.write16("! Ambiguous; you need another { and }.\n".to_string())?;
            return Ok(());
        }
        if self.math_state.pending_script.is_some() {
            return Err(Error::invalid_input("\\over 前不能有未挂脚本（Missing { inserted）"));
        }
        let num = std::mem::take(&mut level.atoms);
        level.fraction = Some(FractionPending { thickness, num });
        Ok(())
    }
    /// `\left<delim>`：压一层数学层（定界符记在层上），`\right` 时收为 Delimited。
    /// 嵌套 `\left...\right` 由层栈天然支持（ETRIP \middle 测试含深层嵌套）。
    fn math_left(&mut self, delim: Option<u32>) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_mode_error("left");
        }
        self.math_state.math.push(MathLevel {
            left: Some(delim),
            ..Default::default()
        });
        Ok(())
    }
    /// `\left`/`\middle` 的 math left group（16）挂起标记：随后的 begin_group 消费。
    fn math_left_begin(&mut self) -> Result<()> {
        self.box_state.pending_kind = Some(GroupKind::MathLeft);
        Ok(())
    }
    /// `\right<delim>`：弹最内层 `\left` 层，内容收为 Delimited 原子并入外层。
    fn math_right(&mut self, delim: Option<u32>) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_mode_error("right");
        }
        let mut level = self
            .math_state.math
            .pop()
            .ok_or_else(|| Error::internal("\\right 无数学层"))?;
        let Some(left) = level.left.take() else {
            // TeX：\right 前缺 \left → 恢复式 "Extra \right."（不中断；expander
            // 侧 math_left_depth 保护后 math_right 只接收有配对的情况，但直接
            // typesetter 路径（测试）仍可达——参考 trip.log L256 双错误恢复）
            self.write16("! Extra \\right.\n".to_string())?;
            return Ok(());
        };
        // 先收 \left(...\over...\right) 的分式，再包定界符
        Self::math_finish_fraction(&mut level);
        let parent = self
            .math_state.math
            .last_mut()
            .ok_or_else(|| Error::internal("\\right 无外层数学层"))?;
        parent.atoms.push(MathAtom::Delimited {
            left,
            body: level.atoms,
            right: delim,
        });
        Ok(())
    }
    /// e-TeX `\middle<delim>`：在 \left...\right 体内插入定界符原子（类 Inner）。
    fn math_middle(&mut self, delim: Option<u32>) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_mode_error("middle");
        }
        let level = self
            .math_state.math
            .last_mut()
            .ok_or_else(|| Error::internal("\\middle 无数学层"))?;
        Self::math_finish_fraction(level);
        level.atoms.push(MathAtom::Middle(delim));
        Ok(())
    }
    /// `\sqrt`：等待 radicand 字段（下一个原子或组）。
    fn math_sqrt(&mut self) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_mode_error("sqrt");
        }
        // \sqrt 本身不是合法字段开头（tex.web scan_math othercases）
        self.check_math_field_break()?;
        self.math_state.sqrt_pending = true;
        Ok(())
    }
    /// `\radical<delimiter><math field>`：根式原子（\sqrt 底层，带定界符号；TRIP L412）。
    fn math_radical(&mut self, delim: Option<u32>) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_mode_error("radical");
        }
        // \radical 不是合法字段开头（tex.web scan_math othercases；TRIP L272
        // `\mathord \radical` 在此报 Missing { inserted）
        self.check_math_field_break()?;
        self.math_state.radical_pending = Some(delim.unwrap_or(0));
        Ok(())
    }
    /// `\mathord` 等：给下一个字段定类。
    fn math_class(&mut self, class: u8) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_mode_error("mathord");
        }
        // \mathord 等不是合法字段开头（tex.web scan_math othercases；连续
        // `\mathord\mathord x` 第二个在此报 Missing { inserted）
        self.check_math_field_break()?;
        // 入参是命令编号（1=Bin/2=Op/7=Inner），须用 class_of_cmd 而非
        // mathcode 体系的 class_of（两体系 Bin/Op 互换）
        self.math_state.class_pending = Some(Self::class_of_cmd(class));

        Ok(())
    }
    /// `\accent`（数学模式）：TeX 报错改道为 `\mathaccent`（tex.web math_ac），
    /// <15-bit number> + nucleus 字段继续扫描（TRIP L396 `\accent\x\vfill`）。
    /// 水平/受限水平模式的 `\accent` 是合法文本重音——只在数学模式报错改道。
    fn math_accent(&mut self, plain: bool) -> Result<()> {
        // \mathaccent 不是合法字段开头（tex.web scan_math othercases）
        self.check_math_field_break()?;
        if plain && matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            self.write16(
                "! Please use \\mathaccent for accents in math mode.\n".to_string(),
            )?;
        }
        self.math_state.accent_pending = matches!(self.mode(), Mode::Math | Mode::DisplayMath);
        Ok(())
    }
    /// `\underline`：等待字段组（数学模式；组开收为 Underline 原子）。
    fn math_underline(&mut self) -> Result<()> {
        self.check_math_field_break()?;
        self.math_state.underline_pending = matches!(self.mode(), Mode::Math | Mode::DisplayMath);
        Ok(())
    }
    /// `\mathchar<15-bit>`：完整数学字符原子（tex.web math_char）——
    /// 数学模式直接产出 Char 原子（class=n>>12&7、fam=n>>8&15、charcode=n&255）；
    /// 文本模式同 `\char` 输出低 8 位字符。此前 scan_number 即丢。
    fn math_char_full(&mut self, n: u32) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.token(ntex_core::Token::char(
                ntex_core::Catcode::Other,
                n & 0xFF,
            ));
        }
        let class = Self::class_of(((n >> 12) & 7) as u8);
        let fam = ((n >> 8) & 0xF) as u8;
        let charcode = n & 0xFF;
        self.math_push_atom(MathAtom::Char(MathChar { class, fam, charcode }))
    }
    /// `\overline`：等待字段组（数学模式；组开收为 Overline 原子）。
    fn math_overline(&mut self) -> Result<()> {
        self.check_math_field_break()?;
        self.math_state.overline_pending = matches!(self.mode(), Mode::Math | Mode::DisplayMath);
        Ok(())
    }
    /// 数学字体族分配（`\textfont<fam>=<fontcs>` 等；M4-3）。
    fn math_font(&mut self, kind: u8, fam: u8, font: u32) -> Result<()> {
        if let Some(slot) = self.math_state.math_fonts.get_mut(fam as usize) {
            slot[kind as usize] = Some(FontId(font));
        }
        Ok(())
    }
    fn muskip_param(&mut self, idx: usize, glue: ntex_core::Glue) -> Result<()> {
        // \thinmuskip/\medmuskip/\thickmuskip：存 mu 参数（math_to_hlist 读）
        // 标位 muskip_is_mu[idx] 同时置 true——muskip_param 触发自 expander 端的
        // `\thinmuskip=<mu glue>` 路径，width/stretch/shrink 始终以 mu 数值存。
        if idx < 3 {
            self.math_state.muskip_params[idx] = glue;
            self.math_state.muskip_is_mu[idx] = true;
        }
        Ok(())
    }
}

impl BoxSink for NodeBuilder {
    /// `\setbox<n>=<box>`（ETRIP）：记录目标寄存器；后续封装的盒子存入该槽。
    fn setbox(&mut self, idx: usize, global: bool) -> Result<()> {
        self.box_state.setbox_target = Some(idx);
        self.box_state.setbox_global = global;
        Ok(())
    }
    /// `\hbox to/spread <dimen>`（ETRIP）：记录盒子规格，随下一个盒子组生效。
    fn box_spec(&mut self, to: Option<i64>, spread: Option<i64>) -> Result<()> {
        self.box_state.pending_box_spec = Some((to, spread));
        Ok(())
    }
    /// 无限阶胶水（ETRIP）：`\hfil`=0/`\hfill`=1/`\hss`=2/`\vfil`=3/`\vfill`=4/`\vss`=5。
    /// 方向不符当前模式的原语忽略（TeX 语义：如水平模式中的 `\vfil` 无效）。
    /// stretch/shrink 按 TeX 存 **1pt = 65536sp**（`0pt plus 1fil` 的 stretch=65536，
    /// fil 阶下 gs 按比例缩放、布局不变，showbox 显示 `plus 1.0fil` 对齐参考）。
    fn fill_glue(&mut self, kind: u8) -> Result<()> {
        let one = ntex_core::register::SP_PER_PT;
        let (horizontal, stretch, shrink, order) = match kind {
            0 => (true, one, 0, GLUE_ORDER_FIL),      // \hfil  0pt plus 1fil
            1 => (true, one, 0, GLUE_ORDER_FILL),     // \hfill 0pt plus 1fill
            2 => (true, one, one, GLUE_ORDER_FIL),    // \hss   0pt plus 1fil minus 1fil
            3 => (false, one, 0, GLUE_ORDER_FIL),     // \vfil
            4 => (false, one, 0, GLUE_ORDER_FILL),    // \vfill
            5 => (false, one, one, GLUE_ORDER_FIL),   // \vss
            6 => (false, -one, 0, GLUE_ORDER_FIL),    // \vfilneg（负 1fil）
            7 => (true, -one, 0, GLUE_ORDER_FIL),     // \hfilneg（负 1fil）
            _ => return Err(Error::internal("非法 fill 胶水种类")),
        };
        let in_horizontal =
            matches!(self.mode(), Mode::Horizontal | Mode::RestrictedHorizontal);
        if horizontal != in_horizontal {
            return Ok(()); // 方向不符：忽略
        }
        // \vfill 等不是合法数学字段开头（tex.web scan_math othercases；TRIP L396
        // `\accent\x\vfill` 在 \vfill 处报 Missing { inserted）
        self.check_math_field_break()?;
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Ok(()); // 数学模式中 fill 胶水无效果
        }
        self.append(Node::Glue {
            name: None,            width: 0,
            stretch,
            shrink,
            stretch_order: order,
            shrink_order: order,
        });
        Ok(())
    }
    /// `\vsplit<n> to/spread <dimen>`（ETRIP）：拆分盒子寄存器 n 顶部。
    /// 寄存器 n 保留余量；结果按 `\setbox` 目标路由，否则追加。
    /// 寄存器 255 = 待输出页队首（[PAGE_BOX]）：latex.ltx `\@doclearpage` 的
    /// `\vsplit\@cclv to\z@` 从页顶取走 discardables。
    /// tex.web vsplit @<Dispense with trivial cases...@>：void 盒 → 结果 void、
    /// 静默（"The extracted box is void if and only if the original box was
    /// void"）；真 TeX 实测 `\vsplit255 to 10pt` void → `\ifvoid0`=Y、`\ht0`=0、
    /// 255 保持 void、无错误信息。
    fn vsplit(&mut self, idx: usize, to: Option<i64>, spread: Option<i64>) -> Result<()> {
        let Some(b) = self.take_box_at(idx) else {
            // void 盒：cur_box=null（tex.web）→ `\setbox` 目标存空、裸调用不产节点
            if let Some(t) = self.box_state.setbox_target.take() {
                self.store_box(t, None);
            }
            return Ok(());
        };
        let natural = b.height + b.depth;
        let target = match (to, spread) {
            (Some(t), _) => t,
            (_, Some(s)) => natural + s,
            _ => natural,
        };
        let (result, remainder) = split_vbox(b, target);
        // 余量写回寄存器：tex.web `box(n):=vpack(q,natural)`；若 q 为空则原盒
        // 变 void。LaTeX mark 代码依赖 `\vsplit <box> to \maxdimen` 后
        // `\ifvoid<box>` 为真来终止递归。
        let remainder = if remainder.children.is_empty() {
            None
        } else {
            Some(remainder)
        };
        self.write_box(idx, remainder);
        if let Some(t) = self.box_state.setbox_target.take() {
            self.store_box(t, Some(result));
        } else {
            self.append(Node::Box(result));
        }
        Ok(())
    }
    /// `\box<n>`（M3-5-3）：取出盒子寄存器；`\shipout` 前缀时封装为页面，
    /// 否则作为节点追加到当前列表。void 盒子报错（TeX "Box n is void"）。
    /// 寄存器 255 = 待输出例程处理页面的队首（[PAGE_BOX]，tex.web `box(255)`）。
    fn box_register(&mut self, idx: usize) -> Result<()> {
        // `\setbox5=\box3`：把寄存器 3 移入目标 5（\box3 变 void；tex.web set_box 赋值语义）
        if let Some(target) = self.box_state.setbox_target.take() {
            // `\setbox0=\lastbox`：优先取 \lastbox 摘下的盒子
            let b = self.box_state.lastbox_hold.take().or_else(|| self.take_box_at(idx));
            self.store_box(target, b);
            return Ok(());
        }
        // `\box0` 紧跟在 `\lastbox` 后：取摘下的盒子（TeX 语义）
        let b = self.box_state.lastbox_hold.take().or_else(|| self.take_box_at(idx));
        let Some(b) = b else {
            // TeX：\box 取 void 盒子 → 空 hbox 节点（tex.web：仍产生节点；TRIP L104 前 \copy200 void）
            self.append(Node::Box(crate::node::BoxNode::new_hbox(Vec::new())));
            return Ok(());
        };
        if self.page_state.shipout_next {
            self.page_state.shipout_next = false;
            self.ship_page(b);
        } else {
            self.append(Node::Box(b));
        }
        Ok(())
    }
    /// 盒子寄存器种类（0=void、1=hbox、2=vbox）；`\ifvoid`/`\ifhbox`/`\ifvbox` 用。
    /// 寄存器 255 = 待输出页队首（[PAGE_BOX]）：页面为 vbox → `\ifvbox255`=真、
    /// 页队列空 → `\ifvoid255`=真（真 TeX 实测：页在 `VB:Y|HB:N|VOID:N`，
    /// 页尽 `VOID:Y`）。
    fn box_register_kind(&self, idx: usize) -> i64 {
        match self.box_view(idx) {
            None => 0,
            Some(b) => match b.kind {
                crate::node::BoxKind::HBox => 1,
                crate::node::BoxKind::VBox => 2,
            },
        }
    }
    /// `\wd/\ht/\dp<n>`：盒子寄存器维度（void 为 0）。
    /// 寄存器 255 = 待输出页队首（[PAGE_BOX]）：latex.ltx `\@specialoutput` 的
    /// `\@pageht \ht\@holdpg` 型查询与页尺寸记账（`\ht\@cclv`）同语义。
    fn box_dim(&self, idx: usize, dim: u8) -> i64 {
        let Some(b) = self.box_view(idx) else {
            return 0;
        };
        match dim {
            0 => b.width,
            1 => b.height,
            _ => b.depth,
        }
    }
    /// `\wd/\ht/\dp<n>=<dimen>`：设置盒子寄存器维度。
    /// 255 → 改待输出页队首（`\dp\@cclv=...` 型改写落在页面上）；void 忽略。
    fn set_box_dim(&mut self, idx: usize, dim: u8, value: i64) -> Result<()> {
        let idx255 = idx == PAGE_BOX;
        let slot = if idx255 {
            self.page_state.pending_pages.front_mut()
        } else {
            self.boxes_mut().get_mut(idx).and_then(|s| s.as_mut())
        };
        let Some(b) = slot else {
            // void 盒子无维度可设：忽略（TeX 恢复语义，TRIP halign 模板场景）
            return Ok(());
        };
        match dim {
            0 => b.width = value,
            1 => b.height = value,
            _ => b.depth = value,
        }
        Ok(())
    }
    /// `\copy<n>`：复制盒子寄存器为节点追加到当前列表（原寄存器保留）。
    /// 寄存器 255 = 待输出页队首（[PAGE_BOX]）——trip.tex 第二例程
    /// `\setbox255\copy255` 即复制当前页。
    fn copy_box(&mut self, idx: usize) -> Result<()> {
        let b = if let Some(h) = self.box_state.lastbox_hold.take() {
            // `\copy0` 紧跟在 `\lastbox` 后：复制摘下的盒子
            h.clone()
        } else {
            let Some(b) = self.box_view(idx) else {
                // TeX：\copy 取 void 盒子 → **空 hbox 节点**（tex.web copy_scan_box：
                // void → null box，仍产生节点触发 freeze/interline；TRIP L104 `\copy200`）
                self.append(Node::Box(crate::node::BoxNode::new_hbox(Vec::new())));
                return Ok(());
            };
            b.clone()
        };
        // `\setbox<n>=\copy<m>`：复制结果存入目标寄存器（\copy 不消耗原盒）
        if let Some(target) = self.box_state.setbox_target.take() {
            self.store_box(target, Some(b));
            return Ok(());
        }
        // `\shipout\copy<n>`（tex.web scan_box 复制语义；现代 LaTeX shipout 包装
        // `\tex_shipout:D \box_use:N \l_shipout_box`，`\box_use:N`≡`\copy`——
        // ship 副本、寄存器保留给 shipout/after 钩子）。不认此臂则页永不 ship：
        // shipout_next 泄漏毒化后续轮次 → "Output loop---100 consecutive dead cycles"。
        if self.page_state.shipout_next {
            self.page_state.shipout_next = false;
            self.ship_page(b);
            return Ok(());
        }
        self.append(Node::Box(b));
        Ok(())
    }
    /// `\unhbox<n>`/`\unhcopy<n>`：hbox 拆开，子节点追加到当前列表。
    /// tex.web unpackage：`if p=null then return`——void 盒**静默无操作**（真
    /// pdflatex 的 `\unvbox\@begindvibox`（恒 void）不报错为证）；类型/模式
    /// 不符才报 "! Incompatible list can't be unboxed."（TRIP L396 实为
    /// `\unhcopy3` 在 math 模式遇 vlist，非 voidness）。
    fn unhbox(&mut self, idx: usize, copy: bool) -> Result<()> {
        let Some(b) = (if copy {
            self.box_view(idx).cloned()
        } else {
            self.take_box_at(idx)
        }) else {
            return Ok(());
        };
        match b.kind {
            BoxKind::HBox => {
                for c in b.children {
                    self.append(c);
                }
                Ok(())
            }
            BoxKind::VBox => {
                self.unbox_error_continue();
                Ok(())
            }
        }
    }
    /// `\unvbox<n>`/`\unvcopy<n>`：vbox 拆开，子节点追加到当前列表。
    fn unvbox(&mut self, idx: usize, copy: bool) -> Result<()> {
        let Some(b) = (if copy {
            self.box_view(idx).cloned()
        } else {
            self.take_box_at(idx)
        }) else {
            return Ok(());
        };
        match b.kind {
            BoxKind::VBox => {
                for c in b.children {
                    self.append(c);
                }
                Ok(())
            }
            BoxKind::HBox => {
                self.unbox_error_continue();
                Ok(())
            }
        }
    }
    /// `\lastbox`：摘下当前列表尾的盒子节点（无则无操作）；
    /// 摘下的盒子由下一个 `\box`/`\copy` 取用（见 [`Self::box_register`]）。
    fn lastbox(&mut self) -> Result<()> {
        let Some(list) = self.lists.last_mut() else {
            return Ok(());
        };
        if let Some(Node::Box(_)) = list.last() {
            if let Some(Node::Box(b)) = list.pop() {
                self.box_state.lastbox_hold = Some(b);
            }
        }
        // `\setbox0=\lastbox`：摘下的盒子存入目标寄存器（tex.web last_box →
        // cur_box → set_box 赋值语义）；裸 \lastbox 留给后续 \box 消费。
        // 无盒可摘 → cur_box=null → `\setbox` 目标**清空**（tex.web
        // `\setbox\thr@@\lastbox` 置 box3 void）——amsmath `\measure@` 的
        // 列宽循环 `\ifhbox\thr@@ ... \repeat` 依赖此终止；旧实现保持
        // box3 旧值 → 恒真死循环。
        if let Some(t) = self.box_state.setbox_target.take() {
            let b = self.box_state.lastbox_hold.take();
            self.store_box(t, b);
        }
        Ok(())
    }
    /// `\unskip`：移除当前列表尾部的 glue 节点（无则无操作）。
    fn unskip(&mut self) -> Result<()> {
        if let Some(list) = self.lists.last_mut() {
            while let Some(Node::Glue { .. } | Node::Leaders { .. }) = list.last() {
                list.pop();
            }
        }
        Ok(())
    }
    /// `\unpenalty`：移除当前列表尾部的 penalty 节点（无则无操作）。
    fn unpenalty(&mut self) -> Result<()> {
        if let Some(list) = self.lists.last_mut() {
            while let Some(Node::Penalty { .. }) = list.last() {
                list.pop();
            }
        }
        Ok(())
    }
    /// `\unkern`：移除当前列表尾部的 kern 节点（无则无操作；TRIP L189）。
    fn unkern(&mut self) -> Result<()> {
        if let Some(list) = self.lists.last_mut() {
            while let Some(Node::Kern { .. }) = list.last() {
                list.pop();
            }
        }
        Ok(())
    }
    /// e-TeX `\lastnodetype`：当前列表尾节点类型码（空列表 -1）。
    ///
    /// `\noalign{...}` 组内：TeX 把 `\noalign` 材料并入对齐所在的垂直列表，
    /// 其尾节点是刚 `\cr` 完成的行/列盒——TeX 的 unset node（e-TeX 码 14，
    /// `\noalign` 只能跟在 `\cr` 后，行盒必已存在）。本引擎行/列盒暂存于
    /// [`Self::align_columns`]、对齐列表在 `\cr` 时已取空，读取时补此映射
    /// （不改节点生成；列表已有节点时仍按列表尾报码）。
    fn last_node_type(&self) -> i64 {
        if let Some(n) = self.lists.last().and_then(|l| l.last()) {
            return n.node_type_code();
        }
        if matches!(self.groups.last().map(|g| g.kind), Some(GroupKind::NoAlign)) {
            return 14; // unset node（\cr 完成的行/列盒）
        }
        -1
    }
    /// `\lastpenalty`：当前列表尾若是 penalty 节点返回其值，否则 0（TeX 语义）。
    fn last_penalty(&self) -> i64 {
        match self.lists.last().and_then(|l| l.last()) {
            Some(Node::Penalty { penalty }) => *penalty,
            _ => 0,
        }
    }
    /// e-TeX `\lastskip`：当前列表最后 glue 节点的宽度（sp；无 glue 节点 → 0）。
    /// tex.web：lastskip 只认 glue_node（leaders 是独立节点类型，不计入）。
    fn last_skip(&self) -> i64 {
        if let Some(list) = self.lists.last() {
            for n in list.iter().rev() {
                if let Node::Glue { width, .. } = n {
                    return *width;
                }
            }
        }
        0
    }
    /// e-TeX `\lastkern`：当前列表最后 kern 节点的宽度（sp；无 kern 节点 → 0）。
    fn last_kern(&self) -> i64 {
        if let Some(list) = self.lists.last() {
            for n in list.iter().rev() {
                if let Node::Kern { width } = n {
                    return *width;
                }
            }
        }
        0
    }
    /// `\raise`/`\lower<dimen>`：记录盒子参考点位移（下一个封装盒子生效）。
    fn raise(&mut self, amount: i64) -> Result<()> {
        self.box_state.pending_shift = Some(amount);
        Ok(())
    }
    /// `\moveleft<dimen>`：记录盒子水平左移（下一个封装盒子生效；TRIP 冲刺简化）。
    fn move_left(&mut self, amount: i64) -> Result<()> {
        self.box_state.pending_hshift = Some(-amount);
        Ok(())
    }
    /// `\moveright<dimen>`：记录盒子水平右移（下一个封装盒子生效；TRIP 冲刺简化）。
    fn move_right(&mut self, amount: i64) -> Result<()> {
        self.box_state.pending_hshift = Some(amount);
        Ok(())
    }
    /// `\showbox<n>`：把盒子寄存器内容格式化到转录（TeX show_box 风格）。
    fn showbox(&mut self, idx: usize) -> Result<()> {
        let Some(b) = self.box_view(idx) else {
            // TeX：\showbox 空盒 → 显示 void 并恢复（TRIP 中 box 状态差异不致命）
            let out = format!("> \\box{idx}=\nvoid\n! OK.\n");
            self.io_state.transcript.push_str(&out);
            return Ok(());
        };
        let mut out = format!("> \\box{idx}=\n");
        showbox_format_box(b, 0, &self.fonts, &self.font_cs_names, &mut out);
        out.push_str("! OK.\n");
        self.io_state.transcript.push_str(&out);
        Ok(())
    }
}

impl AlignSink for NodeBuilder {
    /// `\valign{`/`\halign{`：下一个组为对齐组（6）。M4-5：建排版上下文
    /// （两遍法；`to`/`spread` 规格取 box_spec 槽）。
    fn align_begin(&mut self, is_halign: bool) -> Result<()> {
        let dir = if is_halign { AlignDir::Halign } else { AlignDir::Valign };
        let (to, spread) = self.box_state.pending_box_spec.take().unwrap_or((None, None));
        self.align_stack.push((
            dir,
            AlignCtx {
                tabskips: Vec::new(),
                stream: Vec::new(),
                cur_cells: Vec::new(),
                cur_col: 0,
                to,
                spread,
            },
        ));
        self.box_state.pending_kind = Some(GroupKind::Align);
        Ok(())
    }
    /// M4-5 `\cr`（对齐行/列结束，tex.web fin_row）：当前行单元入流
    /// （原始列表，fin_align 统一封装；空行跳过——tex.web 空行盒高 0）。
    fn align_row_end(&mut self) -> Result<()> {
        let Some((_, ctx)) = &mut self.align_stack.last_mut() else {
            return Ok(());
        };
        let cells = std::mem::take(&mut ctx.cur_cells);
        ctx.cur_col = 0;
        if cells.is_empty() {
            return Ok(());
        }
        ctx.stream.push(AlignItem::Row(cells));
        Ok(())
    }
    /// M4-5 对齐 preamble 结束：记录列边界 tabskip 快照（len = 列数 + 1）。
    fn align_preamble_end(&mut self, tabskips: Vec<ntex_core::Glue>) -> Result<()> {
        if let Some((_, ctx)) = self.align_stack.last_mut() {
            ctx.tabskips = tabskips;
        }
        Ok(())
    }
    /// M4-5 对齐单元开始（tex.web init_span 的 push_nest）：压入单元内容
    /// 列表——\halign 受限水平（v 模板通常以 \hfil 收尾）；\valign 垂直。
    fn align_cell_begin(&mut self) -> Result<()> {
        self.lists.push(Vec::new());
        let mode = match self.align_stack.last() {
            Some((AlignDir::Valign, _)) => Mode::Vertical,
            _ => Mode::RestrictedHorizontal,
        };
        self.list_modes.push(mode);
        Ok(())
    }
    /// M4-5 对齐单元结束（tex.web fin_col 的单元封装时机）：单元列表出栈，
    /// 原始节点攒入当前行（封装延迟到 fin_align 统一列宽）。`&`（Tab）推进
    /// 列指针（跨列单元按 span_len 累计）；`\cr`（Cr）的行收集由
    /// [`Self::align_row_end`] 完成。
    fn align_cell_end(&mut self, end: ntex_core::sink::AlignCellEnd, span_len: u16) -> Result<()> {
        // \valign 单元（垂直列表）内开段时先收段
        if self.mode() == Mode::Horizontal {
            self.close_paragraph();
        }
        let nodes = self.lists.pop().unwrap_or_default();
        self.list_modes.pop();
        let Some((_, ctx)) = &mut self.align_stack.last_mut() else {
            return Ok(());
        };
        let start = ctx.cur_col;
        ctx.cur_cells.push(AlignCellBox { start_col: start, span_len, nodes });
        if matches!(end, ntex_core::sink::AlignCellEnd::Tab) {
            ctx.cur_col += span_len as usize;
        }
        Ok(())
    }
    /// `\noalign{`：下一个组为无对齐组（7）。
    fn noalign_begin(&mut self) -> Result<()> {
        self.box_state.pending_kind = Some(GroupKind::NoAlign);
        Ok(())
    }
}

impl PageSink for NodeBuilder {
    fn output_defined(&mut self, defined: bool) -> Result<()> {
        self.page_state.output_defined = defined;
        if !defined {
            // 例程恢复未定义：未处理页面无法再经例程产出，直接丢弃（TeX 语义）
            self.page_state.pending_pages.clear();
        }
        Ok(())
    }
    fn output_pending(&self) -> bool {
        !self.page_state.pending_pages.is_empty()
    }
    fn take_output_pending(&mut self) -> bool {
        !self.page_state.pending_pages.is_empty()
    }
    fn output_pending_count(&self) -> usize {
        self.page_state.pending_pages.len()
    }
    fn discard_pending_pages(&mut self) {
        self.page_state.pending_pages.clear();
    }
    fn output_break_penalty(&mut self) -> Option<i64> {
        self.page_state.page.take_output_penalty()
    }
    fn take_page_shipped(&mut self) -> bool {
        std::mem::take(&mut self.page_state.page_shipped)
    }
    fn default_output_routine(&mut self) {
        // tex.web @<Perform the default output routine@>：待处理页面不经用户
        // 例程直接 shipout（dead cycles 分支——`\output` 例程从不 ship 时）。
        while let Some(p) = self.page_state.pending_pages.pop_front() {
            self.ship_page(p);
        }
    }
    /// `\count<n>` 赋值镜像（输出例程刀 5 页号链）：shipout 页标签与 DVI bop
    /// 计数的取值源（tex.web ship_out L12694 直接读 count(j)）。
    fn count_changed(&mut self, idx: usize, value: i64) -> Result<()> {
        self.count_changed(idx, value);
        Ok(())
    }
    // ---- ETRIP 冲刺：e-TeX marks 族查询 ----
    // （注意：轮转在 feed_one 产出页时立即执行，不在查询时修改状态。）
    fn topmarks(&self, class: i64) -> String {
        self.page_state.marks_top.get(&class).cloned().unwrap_or_default()
    }
    fn firstmarks(&self, class: i64) -> String {
        self.page_state.marks_first.get(&class).cloned().unwrap_or_default()
    }
    fn botmarks(&self, class: i64) -> String {
        self.page_state.marks_bot.get(&class).cloned().unwrap_or_default()
    }
    fn splitfirstmarks(&self, class: i64) -> String {
        self.page_state.marks_split_first
            .get(&class)
            .cloned()
            .unwrap_or_default()
    }
    fn splittopmarks(&self, class: i64) -> String {
        self.page_state.marks_split_top
            .get(&class)
            .cloned()
            .unwrap_or_default()
    }
    fn splitbotmarks(&self, class: i64) -> String {
        self.page_state.marks_split_bot
            .get(&class)
            .cloned()
            .unwrap_or_default()
    }
    /// `\insert<num>{...}`：insert 节点追加到当前列表（无维度；体 token 串无损保留）。
    ///
    /// 三参数取扫描点的参数镜像——tex.web 在 insert_group 收口（`}` 处）读
    /// `\splittopskip`/`\splitmaxdepth`/`\floatingpenalty`，NTex 体不在扫描位
    /// 执行，组体内的同名赋值不生效（见 survey §5.bis.4 发现未修 1）。
    /// `\insert255` 按 tex.web 报错并改道 0（box 255 是页面寄存器）——否则
    /// 刀 3 的 fire_up 累积会写穿 [`PAGE_BOX`] 页队列。
    fn insert_node(&mut self, class: usize, toks: Vec<Token>) -> Result<()> {
        let mut class = class;
        if class == PAGE_BOX {
            self.report_error("You can't \\insert255.");
            self.report_help("I'm changing to \\insert0; box 255 is special.");
            class = 0;
        }
        self.append(Node::Ins {
            class,
            body: toks,
            split_top_skip: self.params.splittopskip,
            split_max_depth: self.params.splitmaxdepth,
            float_cost: self.params.misc[37], // \floatingpenalty
        });
        Ok(())
    }
    /// `\vadjust{...}`：adjust 节点追加到当前列表（无维度）。
    fn vadjust(&mut self, toks: Vec<Token>) -> Result<()> {
        self.append(Node::Adjust {
            text: toks_to_text(&toks),
        });
        Ok(())
    }
    /// 输出例程的隐式组：组种类 8（output group，tex.web group_code）。
    fn output_routine_begin(&mut self) -> Result<()> {
        self.box_state.pending_kind = Some(GroupKind::Output);
        Ok(())
    }
    /// `\mark`/`\marks<n>`：mark 节点追加到当前列表（无维度）。
    /// 同步更新当前页 marks_first/marks_bot（class=None 映射到 0，即 \mark=\marks0）。
    fn mark(&mut self, class: Option<i64>, text: String) -> Result<()> {
        // TeX 语义：\mark 等价于 \marks0（class 0）。
        let c = class.unwrap_or(0);
        // marks_first：该 class 在当前页第一次出现时设置。
        self.page_state.marks_first.entry(c).or_insert_with(|| text.clone());
        // marks_bot：每次出现都更新（最后一次出现）。
        self.page_state.marks_bot.insert(c, text.clone());
        self.append(Node::Mark { class, text });
        Ok(())
    }
    fn take_write_flush_pending(&mut self) -> bool {
        let v = self.page_state.write_flush_pending;
        self.page_state.write_flush_pending = false;
        v
    }
}

impl IoSink for NodeBuilder {
    // （report_error, report_help 走子 trait 默认实现，NodeBuilder 未覆写）
    /// `\patterns{...}`（M4-6）：解析文本为 Liang trie（后续段落折行按需断字）。
    fn patterns(&mut self, patterns: Vec<u8>) -> Result<()> {
        self.patterns = PatternTrie::parse(&patterns);
        Ok(())
    }
    /// `\hyphenation{...}`（ETRIP）：追加异常词表（小写字母 + 允许断点）。
    fn hyphenation(&mut self, words: Vec<(Vec<u8>, Vec<usize>)>) -> Result<()> {
        self.hyph_exceptions.extend(words);
        Ok(())
    }
    // ETRIP 冲刺：终端转录（\message/\show/\showthe/\write16）
    fn message(&mut self, text: String) -> Result<()> {
        self.io_state.transcript.push_str(&text);
        Ok(())
    }
    fn show(&mut self, text: String) -> Result<()> {
        self.io_state.transcript.push_str(&text);
        self.io_state.transcript.push('\n');
        Ok(())
    }
    /// `\showbox<n>`：把盒子寄存器内容格式化到转录（TeX show_box 风格）。
    fn write16(&mut self, text: String) -> Result<()> {
        self.io_state.transcript.push_str(&text);
        self.io_state.transcript.push('\n');
        Ok(())
    }
    /// `\showgroups`：把组上下文栈格式化为转录（诊断用）。
    fn showgroups(&mut self) -> Result<()> {
        let mut out = String::from("### begin group\n");
        for (i, g) in self.groups.iter().enumerate() {
            out.push_str(&format!("level {i}: {:?} (code {})\n", g.kind, g.kind.code()));
        }
        out.push_str("### end group\n");
        self.io_state.transcript.push_str(&out);
        Ok(())
    }
    /// `\showlists`：把当前列表简化为转录（诊断用；盒子内容递归展示）。
    fn showlists(&mut self) -> Result<()> {
        let mut out = String::from("### begin list\n");
        for (li, list) in self.lists.iter().enumerate() {
            out.push_str(&format!(
                "### list {li} (mode {:?}, {} nodes)\n",
                self.list_modes.get(li),
                list.len()
            ));
            for n in list {
                showbox_format_node(n, 1, &self.fonts, &self.font_cs_names, &mut out);
            }
        }
        out.push_str("### end list\n");
        self.io_state.transcript.push_str(&out);
        Ok(())
    }
    /// `\write<n>{...}`（非 \immediate）：whatsit 节点追加到当前列表（无维度）。
    fn whatsit(&mut self, text: String) -> Result<()> {
        self.append(Node::Whatsit {
            text,
            special: false,
        });
        Ok(())
    }
    /// `\special{...}`：whatsit 节点（`special: true`——shipout 经 DVI xxx
    /// 落后端；`\pdfrefximage` 的图片载荷同走此通道）。
    fn special(&mut self, text: String) -> Result<()> {
        self.append(Node::Whatsit {
            text,
            special: true,
        });
        Ok(())
    }
}

/// 组合 trait 落名：NodeBuilder 经 `include!` 入 mod.rs，
/// 排版器以 `Box<dyn TokenSink>` 挂进 Expander。
impl TokenSink for NodeBuilder {}

/// M4-5 fin_align（tex.web §784-823 简化数学）：两遍法——
/// 1) 列宽 w_c = max(单列 span 单元自然宽)；跨列单元按跨度升序，
///    需求超区间和时差额均摊（span_widths 精神）；
/// 2) `to <dimen>` 摊派：自然总宽不足目标 → 差额均摊各列；
/// 3) 单元 hpack 到跨度宽（区间列宽和 + 中间 tabskip 自然宽），
///    行 = hpack([g_0, cell_0, g_1, ..., g_n]) 自然宽；
/// 4) \halign → vbox of 行盒；\valign → 简化列并排（hbox of 列盒，
///    不做行高数学——valign 用例罕见，后续校准）。
///
/// 自由函数（非 TokenSink 事件）：由 `group_end` 的 Align 分支在对齐组
/// 结束时调用，行间 interline 胶水按 `params` 的 \baselineskip 族计算。
fn align_fin(params: ntex_core::param::Params, dir: AlignDir, mut ctx: AlignCtx) -> Vec<Node> {
    // 尾行未 \cr（对齐组 `}` 前隐含收行）
    if !ctx.cur_cells.is_empty() {
        let cells = std::mem::take(&mut ctx.cur_cells);
        ctx.stream.push(AlignItem::Row(cells));
    }
    let n = ctx.tabskips.len().saturating_sub(1);
    let glue_w = |i: usize| ctx.tabskips.get(i).map(|g| g.width).unwrap_or(0);
    match dir {
        AlignDir::Valign => {
            // 简化：数据列（\cr 分隔）各自 vbox 自然高并排；noalign 材料
            // 为水平材料原样混入
            let mut cols: Vec<Node> = Vec::new();
            for item in ctx.stream {
                match item {
                    AlignItem::Row(cells) => {
                        let mut col: Vec<Node> = Vec::new();
                        for c in cells {
                            let nat = crate::node::vbox_dimensions(&c.nodes);
                            col.push(Node::Box(crate::node::vpack(
                                c.nodes,
                                nat.height + nat.depth,
                                params.boxmaxdepth,
                            )));
                        }
                        let nat = crate::node::hbox_dimensions(&col).width;
                        cols.push(Node::Box(crate::node::hpack(&col, nat)));
                    }
                    AlignItem::Material(ns) => cols.extend(ns),
                }
            }
            let nat = crate::node::hbox_dimensions(&cols).width;
            vec![Node::Box(crate::node::hpack(&cols, nat))]
        }
        AlignDir::Halign => {
            // 列宽（第一遍）
            let mut w = vec![0i64; n];
            let mut spans: Vec<(usize, usize, i64)> = Vec::new();
            for item in &ctx.stream {
                if let AlignItem::Row(cells) = item {
                    for c in cells {
                        let nat = crate::node::hbox_dimensions(&c.nodes).width;
                        if c.span_len == 1 && c.start_col < n {
                            w[c.start_col] = w[c.start_col].max(nat);
                        } else if c.span_len > 1 {
                            spans.push((c.start_col, c.span_len as usize, nat));
                        }
                    }
                }
            }
            // 跨列需求按跨度升序摊派（tex.web span_widths 精神）
            spans.sort_by_key(|(_, sp, _)| *sp);
            for (s, sp, need) in spans {
                if s + sp > n {
                    continue;
                }
                let have: i64 = (s..s + sp).map(|i| w[i]).sum::<i64>()
                    + ((s + 1)..s + sp).map(glue_w).sum::<i64>();
                if need > have {
                    let diff = need - have;
                    let each = diff / sp as i64;
                    let mut rem = diff % sp as i64;
                    for wi in w.iter_mut().take(s + sp).skip(s) {
                        let extra = if rem > 0 { rem -= 1; 1 } else { 0 };
                        *wi += each + extra;
                    }
                }
            }
            // 目标宽（tex.web fin_align）：`to` → 锁定值；`spread` → 最宽行
            // 自然宽 + 增量；无规格 → 各行保持自己的自然宽。**列宽不因
            // to/spread 改变**（保持第一遍的自然最大值）——差额由行级
            // hpack 的 glue set 摊给行内 tabskip 胶水（4 阶拉伸/收缩，
            // align_tabskip_node 已保留 stretch/shrink 字段）。旧实现把
            // to 差额均摊进列宽、丢弃 spread（S2/S7），2026-09-08 对齐
            // tex.web fin_align 真语义。
            let target: Option<i64> = match (ctx.to, ctx.spread) {
                (Some(t), _) => Some(t),
                (None, Some(sp)) => {
                    let max_nat = ctx
                        .stream
                        .iter()
                        .filter_map(|it| match it {
                            AlignItem::Row(cells) => {
                                let nodes: Vec<Node> = std::iter::once(
                                    align_tabskip_node(ctx.tabskips.first()),
                                )
                                .chain(cells.iter().map(|c| {
                                    let end =
                                        (c.start_col + c.span_len as usize).min(n);
                                    let span_w: i64 = (c.start_col..end)
                                        .map(|i| w[i])
                                        .sum::<i64>()
                                        + ((c.start_col + 1)..end)
                                            .map(glue_w)
                                            .sum::<i64>();
                                    Node::Box(crate::node::hpack(
                                        &c.nodes, span_w,
                                    ))
                                }))
                                .collect();
                                Some(crate::node::hbox_dimensions(&nodes).width)
                            }
                            AlignItem::Material(_) => None,
                        })
                        .max()
                        .unwrap_or(0);
                    Some(max_nat + sp)
                }
                (None, None) => None,
            };
            // 行封装（第二遍）。tex.web：行宽 = Σ列宽 + Σtabskip 自然宽——
            // **所有行等宽**（unset box 设列宽后逐行封装；GT
            // `\vbox{\halign{#\cr a\cr b\cr}}` 两行均 5.55557 宽）。旧实现
            // 无规格时各行保持自然宽，amsmath `\measure@` 的 `\wd\@ne`
            // （末行宽 = 全对齐宽）随之塌掉。to/spread 规格优先。
            let total = target.unwrap_or_else(|| {
                w.iter().sum::<i64>() + (0..=n).map(glue_w).sum::<i64>()
            });
            let mut rows: Vec<Node> = Vec::new();
            for item in ctx.stream {
                match item {
                    AlignItem::Row(cells) => {
                        let mut nodes: Vec<Node> =
                            vec![align_tabskip_node(ctx.tabskips.first())];
                        for c in cells {
                            let end = (c.start_col + c.span_len as usize).min(n);
                            let span_w: i64 = (c.start_col..end).map(|i| w[i]).sum::<i64>()
                                + ((c.start_col + 1)..end).map(glue_w).sum::<i64>();
                            nodes.push(Node::Box(crate::node::hpack(&c.nodes, span_w)));
                            nodes.push(align_tabskip_node(ctx.tabskips.get(end)));
                        }
                        // 差额按行内 tabskip 胶水的 stretch/shrink 摊派
                        // （hpack 4 阶语义）
                        rows.push(Node::Box(crate::node::hpack(&nodes, total)));
                    }
                    AlignItem::Material(ns) => rows.extend(ns),
                }
            }
            // tex.web fin_align「Insert the current list into its environment」
            // （L15989）：行盒**直接拼进外层竖列表**，不打成单个 vbox——
            // `\vbox{\halign{...}}` 的内容就是各行行盒（GT showbox：vbox 直接
            // 下挂 hbox），amsmath `\measure@` 的
            // `\setbox\z@\vbox{\unvbox\z@ \unpenalty \global\setbox\@ne\lastbox}`
            // 依赖 `\lastbox` 取到**末行行盒**；行盒若被 vpack 包成单盒，
            // 后续 `\unhbox\@ne` 即报 Incompatible list 且列宽量测全失。
            // 行间 interline 胶水 = fin_row 的 append_to_vlist（行进对齐自己
            // 的竖列表，prev_depth 从 null 起，行间不穿透外层列表的胶水）。
            let mut out: Vec<Node> = Vec::with_capacity(rows.len());
            let mut prev_depth: Option<i64> = None;
            for node in rows {
                if let Node::Box(b) = &node {
                    if let Some(pd) = prev_depth {
                        let d = params.baselineskip.width - (pd + b.height);
                        let g = if d < params.lineskiplimit {
                            params.lineskip
                        } else {
                            let mut g = params.baselineskip;
                            g.width = d;
                            g
                        };
                        out.push(Node::Glue {
                            name: None,
                            width: g.width,
                            stretch: g.stretch,
                            shrink: g.shrink,
                            stretch_order: g.stretch_order,
                            shrink_order: g.shrink_order,
                        });
                    }
                    prev_depth = Some(b.depth);
                }
                out.push(node);
            }
            out
        }
    }
}

/// M4-5：`\tabskip` 胶水快照 → 水平列表胶水节点（None = 对齐已无 preamble，
/// 兜底零胶水）。
fn align_tabskip_node(g: Option<&ntex_core::Glue>) -> Node {
    match g {
        Some(g) => Node::Glue {
            name: None,
            width: g.width,
            stretch: g.stretch,
            shrink: g.shrink,
            stretch_order: g.stretch_order,
            shrink_order: g.shrink_order,
        },
        None => Node::Glue {
            name: None,
            width: 0,
            stretch: 0,
            shrink: 0,
            stretch_order: 0,
            shrink_order: 0,
        },
    }
}
