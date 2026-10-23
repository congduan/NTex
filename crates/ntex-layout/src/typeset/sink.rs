use super::*;

/// Read a literal `\kern` dimension in a discretionary sublist. TeX builds
/// these sublists in restricted horizontal mode; this covers direct numeric
/// `pt`/`sp` kerns while the surrounding token-list conversion is still a
/// restricted implementation.
fn parse_discretionary_kern(toks: &[Token]) -> Option<(i64, usize)> {
    let mut i = 0;
    while toks.get(i)?.charcode() == Some(u32::from(b' ')) {
        i += 1;
    }
    let negative = matches!(toks.get(i)?.charcode(), Some(45));
    if negative || toks.get(i)?.charcode() == Some(u32::from(b'+')) {
        i += 1;
    }
    let start = i;
    let mut whole = 0_i64;
    while let Some(c @ 48..=57) = toks.get(i).and_then(|t| t.charcode()) {
        whole = whole.checked_mul(10)?.checked_add(i64::from(c - 48))?;
        i += 1;
    }
    if i == start {
        return None;
    }
    let unit = [toks.get(i)?.charcode()?, toks.get(i + 1)?.charcode()?];
    let scale = match unit {
        [112, 116] => ntex_core::SP_PER_PT,
        [115, 112] => 1,
        _ => return None,
    };
    let width = whole.checked_mul(scale)?;
    Some((if negative { -width } else { width }, i + 2))
}

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
        let explicit = self.box_state.pending_kind.take();
        let mut pb_vcenter = false;
        let kind = explicit.or_else(|| {
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
                // \vcenter 组复用 vbox 收集（tex.web mmode+vcenter 同进内层竖
                // 模式）；种类暂存，封装时按数学轴重分 height/depth。
                PendingBox::VCenter => {
                    pb_vcenter = true;
                    GroupKind::VBox
                }
                PendingBox::VTop => GroupKind::VTop,
            })
        });
        // 盒子/对齐组：复用盒子路径（对齐组按 vbox 打包）。`\noalign` 组也
        // 开垂直列表（M4-5：材料在组结束时原样入对齐流，tex.web no_align
        // 材料直接进对齐 vlist——见 group_end 的 NoAlign 分支）。
        // `\insert`/`\vadjust` 组（tex.web begin_insert_or_adjust 的
        // push_nest+mode:=-vmode）
        // 同样走内部垂直列表，但**不是盒子组**——`\shipout`/`\setbox`/`to`/
        // `spread`/位移前缀都不得被它认领。
        let is_insert_or_adjust = matches!(kind, Some(GroupKind::Insert | GroupKind::VAdjust));
        let box_kind = match kind {
            Some(GroupKind::HBox | GroupKind::AdjustedHBox) => Some(PendingBox::HBox),
            // 仅显式 \vcenter 保留 VCenter 种类（显式 VBox 组/对齐组不受影响）
            Some(GroupKind::VBox) if pb_vcenter => Some(PendingBox::VCenter),
            Some(GroupKind::VBox | GroupKind::Align | GroupKind::NoAlign) => Some(PendingBox::VBox),
            Some(GroupKind::Insert | GroupKind::VAdjust) => Some(PendingBox::VBox),
            Some(GroupKind::VTop) => Some(PendingBox::VTop),
            _ => None,
        };
        let box_group = box_kind.is_some() && !is_insert_or_adjust;
        // `\shipout` 目标 = 紧邻的盒子组（内层盒子不消费该标记）
        let ship = if box_group {
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
        let leaders = if box_group {
            self.box_state.pending_leaders.take()
        } else {
            None
        };
        // `\setbox<n>=<box>` 目标寄存器：认领到**最外层**盒子组（tex.web scan_box
        // box_end 语义）。内层嵌套盒（`\setbox0=\vbox{\hbox{...}}` 的 \hbox）不消费，
        // 否则 target 被第一个内层盒抢走、RHS 的 vbox 无法入寄存器。
        // `\global` 旗标同批认领（tex.web：随赋值入 box_context，嵌套赋值不覆写）。
        let setbox = if box_group {
            self.box_state.setbox_target.take()
        } else {
            None
        };
        let setbox_global = setbox.is_some() && self.box_state.setbox_global;
        // `to`/`spread` 规格：同 setbox 一样认领到紧邻盒子组（嵌套时内层盒的
        // box_spec 调用不得覆写外层已认领的规格）。
        let spec = if box_group {
            self.box_state.pending_box_spec.take()
        } else {
            None
        };
        // 位移前缀（\raise/\lower/\moveleft/\moveright）：认领到紧邻盒子组
        // （tex.web：位移只作用于紧随其后的那个盒子，不向内层嵌套盒泄漏）。
        let shift = if box_group {
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
            insert_class: if kind == Some(GroupKind::Insert) {
                std::mem::take(&mut self.box_state.pending_insert_class)
            } else {
                0
            },
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
                PendingBox::VBox | PendingBox::VTop | PendingBox::VCenter => Mode::Vertical,
            };
            self.lists.push(Vec::new());
            self.list_modes.push(new_mode);
            // \hbox 内容从 spacefactor=1000 开始（tex.web：进入受限水平模式重置）
            if new_mode == Mode::RestrictedHorizontal {
                self.space_factor = 1000;
            }
        } else if gkind == GroupKind::Output {
            // tex.web output routine runs in its own internal vertical list.
            // NTex may inject it while the interrupted input is still in hmode;
            // without this nest, output-routine material can close or pollute the
            // interrupted paragraph, turning later hmode penalties into vmode ones.
            self.page_state.in_output_group = true;
            self.lists.push(Vec::new());
            self.list_modes.push(Mode::Vertical);
        } else if gkind == GroupKind::Math {
            // 数学组：`{...}`（含脚本/根式/定类字段）压 math 层。
            // `\begingroup`（SemiSimple）等显式组不压 math 层（tex.web math_group 语义）。
            let field = if self
                .math_state
                .choices
                .last()
                .is_some_and(|c| c.awaiting_group)
            {
                self.math_state.choices.last_mut().unwrap().awaiting_group = false;
                Some(MathFieldKind::Choice)
            } else if let Some(is_sup) = self.math_state.pending_script.take() {
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
                self.math_state
                    .class_pending
                    .take()
                    .map(MathFieldKind::Class)
            };
            self.math_state.math.push(MathLevel {
                atoms: Vec::new(),
                field,
                left: None,
                fraction: None,
                frac_delims: (None, None),
            });
        }
        Ok(())
    }
    fn group_end(&mut self) -> Result<()> {
        let Some(ctx) = self.groups.pop() else {
            self.report_error("Extra }, or forgotten \\endgroup.");
            return Ok(());
        };
        // \setbox 组作用域：回滚本组（及更深组）内设置的盒子寄存器
        // （TeX 寄存器组级保存——trip L317 组内 \setbox22 → restoring \box22=void）
        let cur_level = self.groups.len();
        while self
            .box_state
            .box_saves
            .last()
            .is_some_and(|(lvl, _, _)| *lvl > cur_level)
        {
            let (_, idx, old) = self.box_state.box_saves.pop().expect("last 已检查");
            if idx == 0 && std::env::var_os("AVD_BOX_DEBUG").is_some() {
                eprintln!(
                    "AVD_RESTORE0 level={} old_present={}",
                    cur_level,
                    old.is_some()
                );
            }
            // 经 write_box 路由：寄存器 255 的回滚落在页面队列队首（trip.tex
            // 第二例程 `\setbox255\copy255` ——「at end of group, \box255 reverts
            // to former value」tex.web eq_restore 语义）
            self.write_box(idx, old);
            if idx == PAGE_BOX {
                // A local assignment such as TRIP's `\setbox255=\copy255` can be
                // split or unboxed inside the output group and then restored here.
                // The net effect is that box255 was not consumed; do not let the
                // transient `\vsplit255` keep reinjecting the same pending page.
                self.page_state.output_consumed_count = 0;
            }
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
        // insert_group 收口三参数（tex.web：q/d/f 在 unsave **前**读组内值）
        let insert_params = (
            self.params.splittopskip,
            self.params.splitmaxdepth,
            self.params.misc[37],
        );
        // 盒组封装的 boxmaxdepth 用**组内**值（tex.web：vbox 封装在组恢复前——
        // 与 L635-641 折行用组内 hsize 同理；trip L316 `\vbox to10pt{\boxmaxdepth
        // =-1pt...}` → 恢复后取外层会丢 -1pt 钳制）
        let inner_boxmaxdepth = self.params.boxmaxdepth;
        // 先恢复参数镜像（与 VM 的 save_stack 恢复对齐），随后的缩进/interline 用外层值
        if let Some(prev) = self.param_stack.pop() {
            let mut restored = prev;
            if ctx.kind == GroupKind::Output {
                // Output routines commonly flip \holdinginserts globally before
                // returning held-over material. The VM keeps that assignment, but
                // the layout-side param_stack only has coarse group snapshots, so
                // preserve the live mirror across the output group boundary.
                restored.misc[50] = self.params.misc[50];
            }
            self.params = restored;
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
            if level.field == Some(MathFieldKind::Choice) {
                let mut level = level;
                Self::math_finish_fraction(&mut level);
                let choice = self
                    .math_state
                    .choices
                    .last_mut()
                    .ok_or_else(|| Error::internal("mathchoice 分支无挂起选择"))?;
                choice.branches.push(level.atoms);
                if choice.branches.len() == 4 {
                    let choice = self.math_state.choices.pop().unwrap();
                    let branches: [Vec<MathAtom>; 4] = choice.branches.try_into().unwrap();
                    self.math_push_atom(MathAtom::Choice(Box::new(branches)))?;
                } else {
                    choice.awaiting_group = true;
                }
                return Ok(());
            }
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
                    parent.atoms.push(MathAtom::Radical {
                        base: lv.atoms,
                        delim: SQRT_DELIM_CODE,
                    });
                    return Ok(());
                }
                Some(MathFieldKind::Underline) => {
                    // `\underline{...}`：组内容收为 Underline 原子（tex.web math_ac）
                    let mut lv = level;
                    Self::math_finish_fraction(&mut lv);
                    parent.atoms.push(MathAtom::Underline { base: lv.atoms });
                    return Ok(());
                }
                Some(MathFieldKind::Overline) => {
                    // `\overline{...}`：组内容收为 Overline 原子
                    let mut lv = level;
                    Self::math_finish_fraction(&mut lv);
                    parent.atoms.push(MathAtom::Overline { base: lv.atoms });
                    return Ok(());
                }
                Some(MathFieldKind::Radical(delim)) => {
                    // `\radical<delim>{...}`：radicand 同 \sqrt，定界符码进原子
                    // （27 位码驱动 var_delimiter 取根号变体；0 = null delimiter）
                    let mut lv = level;
                    Self::math_finish_fraction(&mut lv);
                    parent.atoms.push(MathAtom::Radical {
                        base: lv.atoms,
                        delim,
                    });
                    return Ok(());
                }
                Some(MathFieldKind::Class(class)) => {
                    // `\mathbin{...}`：内容作为一个指定类原子
                    let mut lv = level;
                    Self::math_finish_fraction(&mut lv);
                    parent.atoms.push(MathAtom::Classed {
                        class,
                        content: lv.atoms,
                        limits: 0,
                    });
                    return Ok(());
                }
                Some(MathFieldKind::Accent) => {
                    // `\accent<15-bit>{...}`：组内容为 nucleus（被重音）字段，
                    // 与已扫描的重音符字段合并（tex.web math_ac）。
                    let mut lv = level;
                    Self::math_finish_fraction(&mut lv);
                    if let Some(MathAtom::Accent { nucleus, .. }) = parent.atoms.last_mut() {
                        *nucleus = lv.atoms;
                    }
                    return Ok(());
                }
                Some(MathFieldKind::Choice) => unreachable!("choice handled above"),
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
            // tex.web insert_group：end_graf（上方 box_kind 臂已做）→
            // `p:=vpack(link(head),natural)` → tail_append(ins_node)。类号取
            // group_begin 认领的 `saved(0)`；三参数取 unsave 前的组内值。
            GroupKind::Insert => {
                let (split_top_skip, split_max_depth, float_cost) = insert_params;
                let children = self.lists.pop().unwrap_or_default();
                self.list_modes.pop();
                let natural = vbox_dimensions(&children);
                let body = vpack(children, natural.height + natural.depth, inner_boxmaxdepth);
                self.append(Node::Ins {
                    class: ctx.insert_class,
                    body,
                    split_top_skip,
                    split_max_depth,
                    float_cost,
                });
            }
            GroupKind::VAdjust => {
                let children = self.lists.pop().unwrap_or_default();
                self.list_modes.pop();
                let natural = vbox_dimensions(&children);
                let body = vpack(children, natural.height + natural.depth, inner_boxmaxdepth);
                self.append(Node::Adjust { body });
            }
            GroupKind::HBox | GroupKind::AdjustedHBox | GroupKind::VBox | GroupKind::VTop => {
                if let Some(kind) = ctx.box_kind {
                    let leaders = ctx.leaders;
                    // 归还本组认领的 to/spread 规格，供 package_box 取用
                    // （内层嵌套盒的规格已各自消费，此处放回的是外层的）
                    self.box_state.pending_box_spec = ctx.spec;
                    self.package_box(
                        kind,
                        ctx.kind == GroupKind::AdjustedHBox,
                        ctx.shipout,
                        leaders,
                        ctx.setbox.map(|idx| (idx, ctx.setbox_global)),
                        ctx.shift,
                        inner_boxmaxdepth,
                    );
                }
            }
            GroupKind::Output => {
                let nodes = self.lists.pop().unwrap_or_default();
                self.list_modes.pop();
                let returned = !nodes.is_empty();
                self.lists[0].extend(nodes);
                self.page_state.in_output_group = false;
                if returned {
                    self.resume_after_output_routine();
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
            // \vcenter（tex.web mmode+vcenter）：收集同 vbox，封装按数学轴重分
            Primitive::VCenter => self.box_state.pending_box = Some(PendingBox::VCenter),
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
                    // 显示公式之后的显式 \par 终止续排语境。否则跨过定理环境
                    // 的 \endtrivlist，下一段的 \everypar 被误抑制，amsthm 的
                    // deferred heading 留下 \@newlist=true，收尾误报 missing \item。
                    Mode::Vertical => self.math_state.after_display = false,
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
            }
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
                        stretch_order: 0,
                        shrink_order: 0,
                        nonscript: false,
                        mu: false,
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
                    self.io_state
                        .transcript
                        .push_str("(Please type a command or say \\end)\n");
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
                stretch_order: g.stretch_order,
                shrink_order: g.shrink_order,
            });
            return Ok(());
        }
        // 数学模式 `\hskip`：转数学空格原子（TeX 数学模式 \hskip ≡ \mskip）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            self.math_push_atom(MathAtom::MSkip {
                width: g.width,
                stretch: g.stretch,
                shrink: g.shrink,
                stretch_order: g.stretch_order,
                shrink_order: g.shrink_order,
                nonscript: false,
                mu: false,
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
        // 数学模式 `\kern`：转数学空格原子（TeX 数学模式 \kern 是 **pt** 胶，
        // mlist_to_hlist 原样落 hlist——与 \mkern 的 mu 胶不同，见 mu_glue）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            self.math_push_atom(MathAtom::MSkip {
                width,
                stretch: 0,
                shrink: 0,
                stretch_order: 0,
                shrink_order: 0,
                nonscript: false,
                mu: false,
            })?;
            return Ok(());
        }
        self.append(Node::Kern { width });
        Ok(())
    }
    /// `\mskip`/`\mkern`（mu 单位）：数学空格原子（mu 胶），mlist_to_hlist
    /// 按当前 style 的 em/18 换算（tex.web new_mu_glue + math_glue）。
    fn mu_glue(&mut self, g: Glue) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            // tex.web：非数学模式 \mskip 报 "You can't use \mskip in vertical mode"
            // 系语义（收窄风险，先按 pt 胶落列表与旧径一致）
            return self.glue(g);
        }
        self.math_push_atom(MathAtom::MSkip {
            width: g.width,
            stretch: g.stretch,
            shrink: g.shrink,
            stretch_order: g.stretch_order,
            shrink_order: g.shrink_order,
            nonscript: false,
            mu: true,
        })
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
            self.box_state.leaders_box = Some((
                kind,
                Node::Rule {
                    width,
                    height,
                    depth,
                },
            ));
            return Ok(());
        }
        // 数学模式 `\vrule`：数学列表规则原子（M4-1——此前忽略）
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            let width = if width == ntex_core::NULL_FLAG {
                0
            } else {
                width
            };
            return self.math_push_atom(MathAtom::Rule {
                width,
                height,
                depth,
            });
        }
        // 非引导上下文：hrule 未定宽度保持 null 哨兵（tex.web scan_rule_spec
        // L9126 起——`\hrule` 扫描期只定 height=0.4pt/depth=0，width 留
        // null_flag；出货时 `rule_wd:=width(this_box)` 解析为包含盒宽，
        // L12598）。此前简化落 0，DVI/渲染按 width>0 判定跳过不画，
        // `\hrule` 整体消失（resume-plain.tex 节标题横线丢失的根因）。
        //
        // `\vrule` 的对称面（2026-10-13）：扫描期 width=0.4pt、height/depth
        // 留 null_flag，出货时按 hlist_out L12422 解析为**包含盒**高深。
        // 唯一例外是**垂直模式**：tex.web L21105 的 `vmode+vrule` 走
        // head_for_vmode 的 `back_input; new_graf`（先开段落，规则最终落
        // hlist），本引擎宽容地直接落 vlist——那里没有包含盒可解析，按 0
        // 落节点（保持旧行为，且不把 -2^30 喂进 vpack/page 的纵向归并）。
        let vertical = self.mode() == Mode::Vertical;
        let fix = |v: i64| {
            if vertical && v == ntex_core::NULL_FLAG {
                0
            } else {
                v
            }
        };
        self.append(Node::Rule {
            width,
            height: fix(height),
            depth: fix(depth),
        });
        Ok(())
    }
    fn param_changed(&mut self, kind: ParamKind, value: ParamValue) -> Result<()> {
        self.params.set(kind, value);
        if kind == ParamKind::VSize {
            if let ParamValue::Dimen(v) = value {
                // 事件面第二通道：组尾回滚只改镜像，见 PageBuilder::vsize_live
                self.page_state.page.note_vsize(v);
            }
        }
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
    /// 转字符节点，并保留直接写出的 `\kern`（当前字体，维度查字体表）。
    fn discretionary(
        &mut self,
        pre: Vec<Token>,
        post: Vec<Token>,
        replace: Vec<Token>,
        kern_csid: Option<u32>,
    ) -> Result<()> {
        let conv = |toks: &[Token]| -> Vec<Node> {
            let mut result = Vec::new();
            let mut i = 0;
            while i < toks.len() {
                if kern_csid.is_some() && toks[i].csid() == kern_csid {
                    if let Some((width, consumed)) = parse_discretionary_kern(&toks[i + 1..]) {
                        result.push(Node::Kern { width });
                        i += consumed + 1;
                        continue;
                    }
                }
                if let Some(c) = toks[i].charcode() {
                    let (w, h, d) = self.fonts.metrics(self.current_font, c);
                    result.push(Node::Char {
                        font: self.current_font,
                        charcode: c,
                        width: w,
                        height: h,
                        depth: d,
                    });
                }
                i += 1;
            }
            result
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
                        Some(
                            crate::typeset::PendingBox::VBox
                                | crate::typeset::PendingBox::VTop
                                | crate::typeset::PendingBox::VCenter
                        )
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
        // TeX discards parskip at the top of an internal vertical list. Keep
        // the outer contribution list's existing path: speculative segments
        // observe a snapshot of that list and the page builder discards its
        // leading glue when appropriate.
        // Alignment cells can temporarily expose their own list as the only
        // list. A surrounding vbox/vtop group still makes this internal
        // vertical mode, where TeX omits the first parskip.
        let in_vertical_box = self
            .groups
            .iter()
            .rev()
            .any(|g| matches!(g.kind, GroupKind::VBox | GroupKind::VTop));
        let internal_top = (self.list_modes.len() > 1 || in_vertical_box)
            && self.lists.last().is_some_and(Vec::is_empty);
        if self.page_state.pagination && !internal_top {
            let ps = self.params.parskip;
            self.append(Node::Glue {
                name: None,
                width: ps.width,
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
impl TokenSink for NodeBuilder {}

/// M4-5 fin_align（tex.web §784-823 简化数学）：两遍法——
/// 1) 列宽 w_c = max(单列 span 单元自然宽)；跨列单元按跨度升序，
///    需求超区间和时差额均摊（span_widths 精神）；
/// 2) `to <dimen>` 摊派：自然总宽不足目标 → 差额均摊各列；
/// 3) 单元 hpack 到跨度宽（区间列宽和 + 中间 tabskip 自然宽），
///    行 = hpack([g_0, cell_0, g_1, ..., g_n]) 自然宽；
/// 4) \halign → 行盒直接入外层竖列表；\valign → 列盒直接入外层水平列表，
///    行高/深度按 halign 列宽数学的对偶口径统一。
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
            // \valign 是 \halign 的对偶：`\cr` 分隔出来的是一列，`&`
            // 分隔的是该列内的各行。第一遍量各行的最大纵向占据，第二遍把
            // 每列打成 vbox，并在列内按 preamble 保留 \tabskip 胶水。
            let mut row_extents = vec![0i64; n];
            let mut span_extents: Vec<(usize, usize, i64)> = Vec::new();
            for item in &ctx.stream {
                if let AlignItem::Row(cells) = item {
                    for c in cells {
                        let nat = crate::node::vbox_dimensions(&c.nodes).total();
                        if c.span_len == 1 && c.start_col < n {
                            row_extents[c.start_col] = row_extents[c.start_col].max(nat);
                        } else if c.span_len > 1 {
                            span_extents.push((c.start_col, c.span_len as usize, nat));
                        }
                    }
                }
            }
            span_extents.sort_by_key(|(_, sp, _)| *sp);
            for (s, sp, need) in span_extents {
                if s + sp > n {
                    continue;
                }
                let have: i64 = (s..s + sp).map(|i| row_extents[i]).sum::<i64>()
                    + ((s + 1)..s + sp).map(glue_w).sum::<i64>();
                if need > have {
                    let diff = need - have;
                    let each = diff / sp as i64;
                    let mut rem = diff % sp as i64;
                    for h in row_extents.iter_mut().take(s + sp).skip(s) {
                        let extra = if rem > 0 {
                            rem -= 1;
                            1
                        } else {
                            0
                        };
                        *h += each + extra;
                    }
                }
            }
            let target: Option<i64> = match (ctx.to, ctx.spread) {
                (Some(t), _) => Some(t),
                (None, Some(sp)) => {
                    Some(row_extents.iter().sum::<i64>() + (0..=n).map(glue_w).sum::<i64>() + sp)
                }
                (None, None) => None,
            };
            let total = target.unwrap_or_else(|| {
                row_extents.iter().sum::<i64>() + (0..=n).map(glue_w).sum::<i64>()
            });
            let mut cols: Vec<Node> = Vec::new();
            for item in ctx.stream {
                match item {
                    AlignItem::Row(cells) => {
                        let col_width = cells
                            .iter()
                            .map(|c| crate::node::vbox_dimensions(&c.nodes).width)
                            .max()
                            .unwrap_or(0);
                        let mut nodes: Vec<Node> = vec![align_tabskip_node(ctx.tabskips.first())];
                        for c in cells {
                            let end = (c.start_col + c.span_len as usize).min(n);
                            let span_h: i64 =
                                (c.start_col..end).map(|i| row_extents[i]).sum::<i64>()
                                    + ((c.start_col + 1)..end).map(glue_w).sum::<i64>();
                            let mut b = crate::node::vpack(c.nodes, span_h, params.boxmaxdepth);
                            b.width = col_width;
                            nodes.push(Node::Box(b));
                            nodes.push(align_tabskip_node(ctx.tabskips.get(end)));
                        }
                        cols.push(Node::Box(crate::node::vpack(
                            nodes,
                            total,
                            params.boxmaxdepth,
                        )));
                    }
                    AlignItem::Material(ns) => cols.extend(ns),
                }
            }
            cols
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
                        let extra = if rem > 0 {
                            rem -= 1;
                            1
                        } else {
                            0
                        };
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
                                let nodes: Vec<Node> =
                                    std::iter::once(align_tabskip_node(ctx.tabskips.first()))
                                        .chain(cells.iter().map(|c| {
                                            let end = (c.start_col + c.span_len as usize).min(n);
                                            let span_w: i64 = (c.start_col..end)
                                                .map(|i| w[i])
                                                .sum::<i64>()
                                                + ((c.start_col + 1)..end).map(glue_w).sum::<i64>();
                                            Node::Box(crate::node::hpack(&c.nodes, span_w))
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
            let total =
                target.unwrap_or_else(|| w.iter().sum::<i64>() + (0..=n).map(glue_w).sum::<i64>());
            let mut rows: Vec<Node> = Vec::new();
            for item in ctx.stream {
                match item {
                    AlignItem::Row(cells) => {
                        let mut nodes: Vec<Node> = vec![align_tabskip_node(ctx.tabskips.first())];
                        for c in cells {
                            let end = (c.start_col + c.span_len as usize).min(n);
                            let span_w: i64 = (c.start_col..end).map(|i| w[i]).sum::<i64>()
                                + ((c.start_col + 1)..end).map(glue_w).sum::<i64>();
                            nodes.push(Node::Box(crate::node::hpack(&c.nodes, span_w)));
                            nodes.push(align_tabskip_node(ctx.tabskips.get(end)));
                        }
                        // 差额按行内 tabskip 胶水的 stretch/shrink 摊派
                        // （hpack 4 阶语义）
                        let mut row_box = crate::node::hpack(&nodes, total);
                        resolve_row_rule_dims(&mut row_box);
                        rows.push(Node::Box(row_box));
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
                        let (g, name) = if d < params.lineskiplimit {
                            (params.lineskip, "lineskip")
                        } else {
                            let mut g = params.baselineskip;
                            g.width = d;
                            (g, "baselineskip")
                        };
                        out.push(Node::Glue {
                            name: Some(name),
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

/// 对齐行盒里 null 高深规则的**行盒**解析（tex.web `hlist_out` L12422 的
/// 「包含盒」在 `\halign` 行里就是行盒本身）。
///
/// tex.web 的 halign 行是**单层**盒：preamble 项（`|` 生成的 `\vrule`、LaTeX
/// 每行的 `\@arstrut`）与各单元内容同挂行盒，故行内每个 vrule 都按行盒高深
/// 出货（表格 `|c|c|` 的竖线 = 12pt = 含 strut 的行高）。本引擎为了列宽摊派
/// 把每个单元先 `hpack` 成盒（多一层嵌套），单元内的 null 高深规则会退化成
/// **单元内容盒**高深（6.94444pt = 文本字高），表格右边框随之短一截、还错位。
///
/// 此处把行盒直接单元盒的直接子规则就地实体化为行盒高深——与 tex.web 同口径；
/// 单元内容**自己嵌套**的盒（用户 `\hbox{\vrule}` 等）不在此列，其 null 高深
/// 仍按各自包含盒在出货端解析（那条路没变）。
fn resolve_row_rule_dims(row: &mut crate::node::BoxNode) {
    let (row_h, row_d) = (row.height, row.depth);
    for child in row.children.iter_mut() {
        let Node::Box(cell) = child else { continue };
        for n in cell.children.iter_mut() {
            if let Node::Rule { height, depth, .. } = n {
                if *height == ntex_core::NULL_FLAG {
                    *height = row_h;
                }
                if *depth == ntex_core::NULL_FLAG {
                    *depth = row_d;
                }
            }
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

mod align;
mod boxes;
mod font;
mod io;
mod math;
mod page;
