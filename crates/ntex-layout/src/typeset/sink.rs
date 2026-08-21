impl TokenSink for NodeBuilder {
    /// 数学移位（`$`，cat 3）：VM 已 peek 出 `display`（连续 `$$`）。
    /// - Math：结束行内公式；
    /// - DisplayMath：`$$` 结束显示公式，单 `$` 报错（TeX "Display math should end with $$"）；
    /// - 非数学模式：display → 显示数学（M4-4：收尾段落/开段，公式作垂直元素），否则行内数学。
    fn math_shift(&mut self, display: bool) -> Result<()> {
        match self.mode() {
            Mode::Math => self.close_math(),
            Mode::DisplayMath => {
                if display {
                    self.close_math()
                } else {
                    Err(Error::invalid_input("Display math should end with $$."))
                }
            }
            Mode::Vertical => {
                if display {
                    // M4-4 显示数学：垂直模式 = TeX new_graf 开段（parskip），公式作段首
                    // 垂直元素（predisplaypenalty + abovedisplayskip + 公式盒 + 下间距）。
                    // 垂直列表为空（文档开头）时不加 parskip。
                    if self.pagination && !self.lists.last().is_some_and(Vec::is_empty) {
                        let ps = self.params.parskip;
                        self.append(Node::Glue {
                            width: ps.width,
                            stretch: ps.stretch,
                            shrink: ps.shrink,
                            stretch_order: 0,
                            shrink_order: 0,
                        });
                    }
                    self.enter_display_math()
                } else {
                    // 行内数学：开段（TeX new_graf）
                    if self.pagination {
                        let ps = self.params.parskip;
                        self.append(Node::Glue {
                            width: ps.width,
                            stretch: ps.stretch,
                            shrink: ps.shrink,
                            stretch_order: 0,
                            shrink_order: 0,
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
                    // short 判定：末行自然宽度（未拉伸）< \displaywidth（≈\hsize）。
                    let last_natural = self.close_paragraph();
                    self.display_short = last_natural.is_some_and(|w| w < self.params.hsize);
                    self.enter_display_math()
                } else {
                    self.enter_math(Mode::Math)
                }
            }
            Mode::RestrictedHorizontal => {
                if display {
                    Err(Error::invalid_input(
                        "显示数学不允许出现在 \\hbox 内（restricted horizontal mode）",
                    ))
                } else {
                    self.enter_math(Mode::Math)
                }
            }
        }
    }

    /// 数学样式原语：数学模式内 push 样式原子（影响后续字阶与 spacing）。
    fn math_style(&mut self, style: u8) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Ok(()); // TeX 报错，简化忽略（非数学模式样式无意义）
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
            return Err(Error::invalid_input("\\over 只能在数学模式使用"));
        }
        if self.fraction_pending.is_some() {
            return Err(Error::invalid_input(
                "\\over 歧义（Ambiguous; you need another { and }）",
            ));
        }
        if self.pending_script.is_some() {
            return Err(Error::invalid_input("\\over 前不能有未挂脚本（Missing { inserted）"));
        }
        let level = self
            .math
            .last_mut()
            .ok_or_else(|| Error::internal("\\over 无数学层"))?;
        let num = std::mem::take(&mut level.atoms);
        self.fraction_pending = Some(FractionPending { thickness, num });
        Ok(())
    }

    /// `\left<delim>`：记录定界符，等待 `\right`（嵌套暂不支持）。
    fn math_left(&mut self, delim: Option<u32>) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Err(Error::invalid_input("\\left 只能在数学模式使用"));
        }
        if self.left_pending.is_some() {
            return Err(Error::invalid_input("\\left 不能嵌套（Extra \\left）"));
        }
        self.left_pending = Some(delim);
        Ok(())
    }

    /// e-TeX `\middle<delim>`：在 \left...\right 体内插入定界符原子（类 Inner）。
    fn math_middle(&mut self, delim: Option<u32>) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Err(Error::invalid_input("\\middle 只能在数学模式使用"));
        }
        let level = self
            .math
            .last_mut()
            .ok_or_else(|| Error::internal("\\middle 无数学层"))?;
        Self::math_finish_fraction(&mut self.fraction_pending, level);
        level.atoms.push(MathAtom::Middle(delim));
        Ok(())
    }

    /// `\right<delim>`：当前 math 层内容收为 \left...\right 的 body。
    fn math_right(&mut self, delim: Option<u32>) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Err(Error::invalid_input("\\right 只能在数学模式使用"));
        }
        let left = self
            .left_pending
            .take()
            .ok_or_else(|| Error::invalid_input("\\right 前缺少 \\left（Missing \\left inserted）"))?;
        let level = self
            .math
            .last_mut()
            .ok_or_else(|| Error::internal("\\right 无数学层"))?;
        // 先收 \left(...\over...\right) 的分式
        Self::math_finish_fraction(&mut self.fraction_pending, level);
        let body = std::mem::take(&mut level.atoms);
        level.atoms.push(MathAtom::Delimited {
            left,
            body,
            right: delim,
        });
        Ok(())
    }

    /// `\sqrt`：等待 radicand 字段（下一个原子或组）。
    fn math_sqrt(&mut self) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Err(Error::invalid_input("\\sqrt 只能在数学模式使用"));
        }
        self.sqrt_pending = true;
        Ok(())
    }

    /// `\mathord` 等：给下一个字段定类。
    fn math_class(&mut self, class: u8) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Err(Error::invalid_input("\\mathord 等只能在数学模式使用"));
        }
        self.class_pending = Some(match class {
            0 => MathClass::Ord,
            1 => MathClass::Bin,
            2 => MathClass::Op,
            3 => MathClass::Rel,
            4 => MathClass::Open,
            5 => MathClass::Close,
            6 => MathClass::Punct,
            _ => MathClass::Inner,
        });
        Ok(())
    }

    /// 数学字体族分配（`\textfont<fam>=<fontcs>` 等；M4-3）。
    fn math_font(&mut self, kind: u8, fam: u8, font: u32) -> Result<()> {
        if let Some(slot) = self.math_fonts.get_mut(fam as usize) {
            slot[kind as usize] = Some(FontId(font));
        }
        Ok(())
    }

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

    /// `\setbox<n>=<box>`（ETRIP）：记录目标寄存器；后续封装的盒子存入该槽。
    fn setbox(&mut self, idx: usize) -> Result<()> {
        self.setbox_target = Some(idx);
        Ok(())
    }

    /// `\hbox to/spread <dimen>`（ETRIP）：记录盒子规格，随下一个盒子组生效。
    fn box_spec(&mut self, to: Option<i64>, spread: Option<i64>) -> Result<()> {
        self.pending_box_spec = Some((to, spread));
        Ok(())
    }

    /// `\vsplit<n> to/spread <dimen>`（ETRIP）：拆分盒子寄存器 n 顶部。
    /// 寄存器 n 保留余量；结果按 `\setbox` 目标路由，否则追加。
    fn vsplit(&mut self, idx: usize, to: Option<i64>, spread: Option<i64>) -> Result<()> {
        let b = self
            .boxes
            .get_mut(idx)
            .and_then(|s| s.take())
            .ok_or_else(|| Error::invalid_input("\\vsplit 盒子为空（void）"))?;
        let natural = b.height + b.depth;
        let target = match (to, spread) {
            (Some(t), _) => t,
            (_, Some(s)) => natural + s,
            _ => natural,
        };
        let (result, remainder) = split_vbox(b, target);
        self.boxes[idx] = Some(remainder);
        if let Some(t) = self.setbox_target.take() {
            self.boxes[t] = Some(result);
        } else {
            self.append(Node::Box(result));
        }
        Ok(())
    }

    /// 无限阶胶水（ETRIP）：`\hfil`=0/`\hfill`=1/`\hss`=2/`\vfil`=3/`\vfill`=4/`\vss`=5。
    /// 方向不符当前模式的原语忽略（TeX 语义：如水平模式中的 `\vfil` 无效）。
    fn fill_glue(&mut self, kind: u8) -> Result<()> {
        let (horizontal, stretch, shrink, order) = match kind {
            0 => (true, 1, 0, GLUE_ORDER_FIL),   // \hfil  0pt plus 1fil
            1 => (true, 1, 0, GLUE_ORDER_FILL),  // \hfill 0pt plus 1fill
            2 => (true, 1, 1, GLUE_ORDER_FIL),   // \hss   0pt plus 1fil minus 1fil
            3 => (false, 1, 0, GLUE_ORDER_FIL),  // \vfil
            4 => (false, 1, 0, GLUE_ORDER_FILL), // \vfill
            5 => (false, 1, 1, GLUE_ORDER_FIL),  // \vss
            _ => return Err(Error::internal("非法 fill 胶水种类")),
        };
        let in_horizontal =
            matches!(self.mode(), Mode::Horizontal | Mode::RestrictedHorizontal);
        if horizontal != in_horizontal {
            return Ok(()); // 方向不符：忽略
        }
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Ok(()); // 数学模式中 fill 胶水无效果
        }
        self.append(Node::Glue {
            width: 0,
            stretch,
            shrink,
            stretch_order: order,
            shrink_order: order,
        });
        Ok(())
    }

    fn token(&mut self, tok: Token) -> Result<()> {
        // 空格（cat 10）：垂直/数学模式忽略；水平模式转词间空白胶水
        // （行首或胶水/惩罚之后忽略，TeX spacer 语义）。
        if tok.catcode() == Some(ntex_core::Catcode::Space) {
            match self.mode() {
                Mode::Vertical | Mode::Math | Mode::DisplayMath => {}
                Mode::Horizontal | Mode::RestrictedHorizontal => {
                    let ignorable = match self.lists.last().and_then(|l| l.last()) {
                        None => true,
                        Some(Node::Glue { .. } | Node::Penalty { .. }) => true,
                        Some(_) => false,
                    };
                    if !ignorable {
                        self.append_space_glue();
                    }
                }
            }
            return Ok(());
        }
        // 数学模式：字符转数学原子（^/_ 挂脚本，字母/其他 → Ord）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_char_tok(tok);
        }
        // M4-7 错误模型：数学模式外遇到 ^/_（cat 7/8）→ TeX "Missing $ inserted"，
        // 而非静默渲染为字面字符（TeX 会插入 $ 恢复；我们直接报错）。
        if matches!(
            tok.catcode(),
            Some(ntex_core::Catcode::Superscript) | Some(ntex_core::Catcode::Subscript)
        ) {
            return Err(Error::invalid_input(
                "Missing $ inserted（^/_ 只能在数学模式内使用）",
            ));
        }
        let Some(node) = self.char_node(tok) else {
            return Ok(()); // 控制序列等无可排版语义
        };
        match self.mode() {
            Mode::Vertical => {
                // 垂直模式字符触发段落（TeX new_graf）
                if self.after_display {
                    // M4-4：显示公式后续文字仍在段内——无 parskip、无缩进（续排）
                    self.after_display = false;
                    self.lists.push(Vec::new());
                    self.list_modes.push(Mode::Horizontal);
                    self.space_factor = 1000;
                    self.append_char(node);
                } else {
                    // M3-5-2：段落起始追加上下段间距 \parskip（空页上被页面构建器丢弃）
                    if self.pagination {
                        let ps = self.params.parskip;
                        self.append(Node::Glue {
                            width: ps.width,
                            stretch: ps.stretch,
                            shrink: ps.shrink,
                            stretch_order: 0,
                            shrink_order: 0,
                        });
                    }
                    self.lists.push(Vec::new());
                    self.list_modes.push(Mode::Horizontal);
                    self.space_factor = 1000; // new_graf：段落开始重置 spacefactor
                    self.insert_indent();
                    self.append_char(node);
                }
            }
            Mode::Horizontal | Mode::RestrictedHorizontal => self.append_char(node),
            Mode::Math | Mode::DisplayMath => unreachable!("数学模式已在上面分支返回"),
        }
        Ok(())
    }

    fn group_begin(&mut self) -> Result<()> {
        // 显式组种类（\begingroup/\valign/\noalign）优先；否则盒子种类；再否则普通组
        let explicit = self.pending_kind.take();
        let kind = explicit.or_else(|| {
            self.pending_box.take().map(|pb| match pb {
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
        // 盒子/对齐组：复用盒子路径（对齐组按 vbox 打包，noalign 组结束时丢弃）
        let box_kind = match kind {
            Some(GroupKind::HBox | GroupKind::AdjustedHBox) => Some(PendingBox::HBox),
            Some(GroupKind::VBox | GroupKind::Align | GroupKind::NoAlign) => Some(PendingBox::VBox),
            Some(GroupKind::VTop) => Some(PendingBox::VTop),
            _ => None,
        };
        // `\shipout` 目标 = 紧邻的盒子组（内层盒子不消费该标记）
        let ship = if box_kind.is_some() {
            std::mem::take(&mut self.shipout_next)
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
        self.groups.push(GroupCtx {
            kind: gkind,
            box_kind,
            shipout: ship,
        });
        self.param_stack.push(self.params);
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
        } else if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            // 数学组：`{...}`（含脚本/根式/定类字段）压 math 层。
            let field = if let Some(is_sup) = self.pending_script.take() {
                Some(MathFieldKind::Script(is_sup))
            } else if self.sqrt_pending {
                self.sqrt_pending = false;
                Some(MathFieldKind::Sqrt)
            } else {
                self.class_pending.take().map(MathFieldKind::Class)
            };
            self.math.push(MathLevel {
                atoms: Vec::new(),
                field,
            });
        }
        Ok(())
    }

    fn group_end(&mut self) -> Result<()> {
        let ctx = self
            .groups
            .pop()
            .ok_or_else(|| Error::internal("group_end 无配对 group_begin"))?;
        // 先恢复参数镜像（与 VM 的 save_stack 恢复对齐），随后的缩进/interline 用外层值
        if let Some(prev) = self.param_stack.pop() {
            self.params = prev;
        }
        // 数学组：内容并入外层（普通组）或作为字段挂到外层 base（^/_ 后组等）。
        if ctx.box_kind.is_none() && matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            let level = self
                .math
                .pop()
                .ok_or_else(|| Error::internal("数学组结束无配对 math 层"))?;
            let parent = self
                .math
                .last_mut()
                .ok_or_else(|| Error::internal("数学组结束无外层 math 层"))?;
            let field_atoms = match level.field {
                Some(MathFieldKind::Script(is_sup)) => {
                    // `x^{...}`：先收组内分式（`x^{a\over b}`），再作为脚本字段挂载
                    let mut lv = level;
                    Self::math_finish_fraction(&mut self.fraction_pending, &mut lv);
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
                    Self::math_finish_fraction(&mut self.fraction_pending, &mut lv);
                    parent.atoms.push(MathAtom::Radical { base: lv.atoms });
                    return Ok(());
                }
                Some(MathFieldKind::Class(class)) => {
                    // `\mathbin{...}`：内容作为一个指定类原子
                    let mut lv = level;
                    Self::math_finish_fraction(&mut self.fraction_pending, &mut lv);
                    parent.atoms.push(MathAtom::Classed {
                        class,
                        content: lv.atoms,
                    });
                    return Ok(());
                }
                None => {
                    // 普通数学组：先收组内分式（`{a\over b}`），再并入外层
                    let mut lv = level;
                    Self::math_finish_fraction(&mut self.fraction_pending, &mut lv);
                    lv.atoms
                }
            };
            parent.atoms.extend(field_atoms);
            return Ok(());
        }
        // 垂直盒子内容结束时，开放段落先封装（\vbox{a} → vbox[hbox(a)]）
        if ctx.box_kind.is_some_and(PendingBox::is_vertical) && self.mode() == Mode::Horizontal {
            self.close_paragraph();
        }
        match ctx.kind {
            // noalign 组：对齐行间材料在 TeX 中并入外层垂直列表；ETRIP 简化丢弃
            GroupKind::NoAlign => {
                self.lists.pop();
                self.list_modes.pop();
            }
            // 对齐组：按 vbox 打包（行内容简化合并；ETRIP 仅需组种类正确）
            GroupKind::Align
            | GroupKind::HBox
            | GroupKind::AdjustedHBox
            | GroupKind::VBox
            | GroupKind::VTop => {
                if let Some(kind) = ctx.box_kind {
                    self.package_box(kind, ctx.shipout);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn primitive(&mut self, prim: Primitive) -> Result<()> {
        match prim {
            Primitive::HBox => self.pending_box = Some(PendingBox::HBox),
            Primitive::VBox => self.pending_box = Some(PendingBox::VBox),
            Primitive::VTop => self.pending_box = Some(PendingBox::VTop),
            Primitive::Par => match self.mode() {
                Mode::Horizontal => {
                    self.close_paragraph();
                }
                // 垂直模式 \par 无操作；受限水平/数学模式拒绝
                Mode::Vertical => {}
                Mode::RestrictedHorizontal => {
                    return Err(Error::invalid_input(
                        "\\par 不允许出现在受限水平模式（\\hbox 内）",
                    ));
                }
                Mode::Math | Mode::DisplayMath => {
                    return Err(Error::invalid_input(
                        "\\par 不允许出现在数学模式（\\par 应在 $ 外）",
                    ));
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
            Primitive::NoIndent => {
                // 垂直模式：下一个段落不缩进；水平模式无操作
                if self.mode() == Mode::Vertical {
                    self.noindent_next = true;
                }
            }
            // M3-5：\shipout 后的下一个盒子封装为页面
            Primitive::ShipOut => self.shipout_next = true,
            // M4-2：\nonscript 使下一个数学空格在脚本模式丢弃
            Primitive::Nonscript => {
                if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
                    self.nonscript_pending = true;
                }
            }
            // 参数扫描型原语经 glue/kern/penalty/rule 事件处理
            _ => {}
        }
        Ok(())
    }

    fn param_changed(&mut self, kind: ParamKind, value: ParamValue) -> Result<()> {
        self.params.set(kind, value);
        Ok(())
    }

    fn sfcode_changed(&mut self, charcode: u8, value: u32) -> Result<()> {
        self.sfcodes[charcode as usize] = value;
        Ok(())
    }

    fn output_defined(&mut self, defined: bool) -> Result<()> {
        self.output_defined = defined;
        if !defined {
            // 例程恢复未定义：未处理页面无法再经例程产出，直接丢弃（TeX 语义）
            self.pending_pages.clear();
        }
        Ok(())
    }

    fn output_pending(&self) -> bool {
        !self.pending_pages.is_empty()
    }

    fn take_output_pending(&mut self) -> bool {
        !self.pending_pages.is_empty()
    }

    fn output_pending_count(&self) -> usize {
        self.pending_pages.len()
    }

    fn discard_pending_pages(&mut self) {
        self.pending_pages.clear();
    }

    /// `\box<n>`（M3-5-3）：取出盒子寄存器；`\shipout` 前缀时封装为页面，
    /// 否则作为节点追加到当前列表。void 盒子报错（TeX "Box n is void"）。
    /// box255 = 待输出例程处理页面的队首。
    fn box_register(&mut self, idx: usize) -> Result<()> {
        // `\setbox5=\box3`：把寄存器 3 移入目标 5（\box3 变 void；tex.web set_box 赋值语义）
        if let Some(target) = self.setbox_target.take() {
            let b = self.boxes.get_mut(idx).and_then(|s| s.take());
            self.boxes[target] = b;
            return Ok(());
        }
        let b = if idx == 255 {
            self.pending_pages.pop_front()
        } else {
            self.boxes.get_mut(idx).and_then(|s| s.take())
        };
        let Some(b) = b else {
            return Err(Error::invalid_input(format!("盒子 {idx} 为空（void）")));
        };
        if self.shipout_next {
            self.shipout_next = false;
            self.shipped.push(b);
            self.write_flush_pending = true;
        } else {
            self.append(Node::Box(b));
        }
        Ok(())
    }

    fn font_selected(&mut self, font: u32) -> Result<()> {
        // fn 指针模式恒为 FontId(0)；TFM 模式更新当前字体
        self.current_font = FontId(font);
        Ok(())
    }

    fn take_write_flush_pending(&mut self) -> bool {
        let v = self.write_flush_pending;
        self.write_flush_pending = false;
        v
    }

    // ETRIP 冲刺：终端转录（\message/\show/\showthe/\write16）
    fn message(&mut self, text: String) -> Result<()> {
        self.transcript.push_str(&text);
        Ok(())
    }

    fn show(&mut self, text: String) -> Result<()> {
        self.transcript.push_str(&text);
        self.transcript.push('\n');
        Ok(())
    }

    fn write16(&mut self, text: String) -> Result<()> {
        self.transcript.push_str(&text);
        self.transcript.push('\n');
        Ok(())
    }

    fn transcript(&self) -> &str {
        &self.transcript
    }

    fn glue(&mut self, g: Glue) -> Result<()> {
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
        self.append(Node::Glue {
            width: g.width,
            stretch: g.stretch,
            shrink: g.shrink,
            stretch_order: 0,
            shrink_order: 0,
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
        // 数学模式 `\penalty`：M4-1 忽略（数学断行点后续补）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Ok(());
        }
        self.append(Node::Penalty { penalty });
        Ok(())
    }

    fn rule(&mut self, width: i64, height: i64, depth: i64) -> Result<()> {
        // 数学模式 `\vrule`：M4-1 忽略（规则原子后续补）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Ok(());
        }
        self.append(Node::Rule { width, height, depth });
        Ok(())
    }

    /// TeXXeT 方向节点：追加到当前列表（宽度 0 占位）。
    fn direction_node(&mut self, kind: ntex_core::sink::DirectionKind) -> Result<()> {
        self.append(Node::Direction { kind });
        Ok(())
    }

    /// `\mark`/`\marks<n>`：mark 节点追加到当前列表（无维度）。
    fn mark(&mut self, class: Option<i64>, text: String) -> Result<()> {
        self.append(Node::Mark { class, text });
        Ok(())
    }

    /// `\insert<num>{...}`：insert 节点追加到当前列表（无维度；内容只收集不排版）。
    fn insert_node(&mut self, class: usize, toks: Vec<Token>) -> Result<()> {
        self.append(Node::Ins {
            class,
            text: toks_to_text(&toks),
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

    /// `\write<n>{...}`（非 \immediate）：whatsit 节点追加到当前列表（无维度）。
    fn whatsit(&mut self, text: String) -> Result<()> {
        self.append(Node::Whatsit { text });
        Ok(())
    }

    /// e-TeX `\lastnodetype`：当前列表尾节点类型码（空列表 -1）。
    fn last_node_type(&self) -> i64 {
        match self.lists.last().and_then(|l| l.last()) {
            None => -1,
            Some(n) => n.node_type_code(),
        }
    }

    /// e-TeX `\currentgrouptype`：当前组类型码。
    /// 数学模式：顶组为数学组 → 9，否则为 `$` 进入的数学移位组 → 15。
    fn current_group_type(&self) -> i64 {
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return match self.groups.last() {
                Some(g) if g.kind == GroupKind::Math => 9,
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

    /// `\begingroup`：下一个组为半简单组（14）。
    fn semisimple_begin(&mut self) -> Result<()> {
        self.pending_kind = Some(GroupKind::SemiSimple);
        Ok(())
    }

    /// `\valign{`/`\halign{`：下一个组为对齐组（6）。
    fn align_begin(&mut self) -> Result<()> {
        self.pending_kind = Some(GroupKind::Align);
        Ok(())
    }

    /// `\noalign{`：下一个组为无对齐组（7）。
    fn noalign_begin(&mut self) -> Result<()> {
        self.pending_kind = Some(GroupKind::NoAlign);
        Ok(())
    }

    /// `\cr`：对齐行结束（简化无操作）。
    fn align_row_end(&mut self) -> Result<()> {
        Ok(())
    }

    /// `\raise`/`\lower<dimen>`：记录盒子参考点位移（下一个封装盒子生效）。
    fn raise(&mut self, amount: i64) -> Result<()> {
        self.pending_shift = Some(amount);
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

    /// `\showbox<n>`：把盒子寄存器内容格式化到转录（TeX show_box 风格）。
    fn showbox(&mut self, idx: usize) -> Result<()> {
        let Some(b) = self.boxes.get(idx).and_then(|s| s.as_ref()) else {
            return Err(Error::invalid_input(format!("\\showbox{idx}: 盒子为空（void）")));
        };
        let mut out = format!("> \\box{idx}=\n");
        showbox_format_box(b, 0, &mut out);
        out.push_str("! OK.\n");
        self.transcript.push_str(&out);
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ---------- \showbox 格式化（TeX show_box 风格） ----------

/// 组内字符 token → 文本（insert/adjust 节点内容；cs 等忽略）。
fn toks_to_text(toks: &[Token]) -> String {
    toks.iter()
        .filter_map(|t| t.charcode())
        .filter_map(char::from_u32)
        .collect()
}

/// sp → pt 字符串（固定 1 位小数）。
fn showbox_pt(sp: i64) -> String {
    format!("{:.1}", sp as f64 / SP_PER_PT as f64)
}

fn showbox_format_box(b: &BoxNode, depth: usize, out: &mut String) {
    let p = ".".repeat(depth);
    let kind = match b.kind {
        BoxKind::HBox => "hbox",
        BoxKind::VBox => "vbox",
    };
    out.push_str(&format!(
        "{p}\\{kind}({}+{})x{}\n",
        showbox_pt(b.height),
        showbox_pt(b.depth),
        showbox_pt(b.width)
    ));
    for c in &b.children {
        showbox_format_node(c, depth + 1, out);
    }
}

fn showbox_format_node(n: &Node, depth: usize, out: &mut String) {
    let p = ".".repeat(depth + 1);
    match n {
        Node::Box(b) => showbox_format_box(b, depth, out),
        Node::Char { charcode, .. } => {
            let c = char::from_u32(*charcode)
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("{charcode}"));
            out.push_str(&format!("{p}\\font {c}\n"));
        }
        Node::Glue {
            width,
            stretch,
            shrink,
            ..
        } => {
            let mut s = format!("{p}\\glue {}", showbox_pt(*width));
            if *stretch != 0 {
                s.push_str(&format!(" plus {}", showbox_pt(*stretch)));
            }
            if *shrink != 0 {
                s.push_str(&format!(" minus {}", showbox_pt(*shrink)));
            }
            s.push('\n');
            out.push_str(&s);
        }
        Node::Kern { width } => out.push_str(&format!("{p}\\kern {}\n", showbox_pt(*width))),
        Node::Penalty { penalty } => out.push_str(&format!("{p}\\penalty {}\n", penalty)),
        Node::Rule {
            width,
            height,
            depth,
        } => out.push_str(&format!(
            "{p}\\rule({}+{})x{}\n",
            showbox_pt(*height),
            showbox_pt(*depth),
            showbox_pt(*width)
        )),
        Node::Leaders { .. } => out.push_str(&format!("{p}\\leaders\n")),
        Node::Discretionary { .. } => out.push_str(&format!("{p}\\discretionary\n")),
        Node::Direction { kind } => {
            let name = match kind {
                ntex_core::sink::DirectionKind::BeginL => "beginL",
                ntex_core::sink::DirectionKind::EndL => "endL",
                ntex_core::sink::DirectionKind::BeginR => "beginR",
                ntex_core::sink::DirectionKind::EndR => "endR",
            };
            out.push_str(&format!("{p}\\{name}\n"));
        }
        Node::Mark { class, text } => match class {
            Some(c) => out.push_str(&format!("{p}\\marks{c} {text}\n")),
            None => out.push_str(&format!("{p}\\mark {text}\n")),
        },
        Node::Ins { class, text } => out.push_str(&format!("{p}\\insert{class} {text}\n")),
        Node::Adjust { text } => out.push_str(&format!("{p}\\vadjust {text}\n")),
        Node::Whatsit { text } => out.push_str(&format!("{p}\\write {text}\n")),
    }
}