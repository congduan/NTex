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
            return self.math_mode_error("over");
        }
        let level = self
            .math
            .last_mut()
            .ok_or_else(|| Error::internal("\\over 无数学层"))?;
        if level.fraction.is_some() {
            return Err(Error::invalid_input(
                "\\over 歧义（Ambiguous; you need another { and }）",
            ));
        }
        if self.pending_script.is_some() {
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
        self.math.push(MathLevel {
            left: Some(delim),
            ..Default::default()
        });
        Ok(())
    }

    /// e-TeX `\middle<delim>`：在 \left...\right 体内插入定界符原子（类 Inner）。
    fn math_middle(&mut self, delim: Option<u32>) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_mode_error("middle");
        }
        let level = self
            .math
            .last_mut()
            .ok_or_else(|| Error::internal("\\middle 无数学层"))?;
        Self::math_finish_fraction(level);
        level.atoms.push(MathAtom::Middle(delim));
        Ok(())
    }

    /// `\right<delim>`：弹最内层 `\left` 层，内容收为 Delimited 原子并入外层。
    fn math_right(&mut self, delim: Option<u32>) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_mode_error("right");
        }
        let mut level = self
            .math
            .pop()
            .ok_or_else(|| Error::internal("\\right 无数学层"))?;
        let left = level.left.take().ok_or_else(|| {
            Error::invalid_input("\\right 前缺少 \\left（Missing \\left inserted）")
        })?;
        // 先收 \left(...\over...\right) 的分式，再包定界符
        Self::math_finish_fraction(&mut level);
        let parent = self
            .math
            .last_mut()
            .ok_or_else(|| Error::internal("\\right 无外层数学层"))?;
        parent.atoms.push(MathAtom::Delimited {
            left,
            body: level.atoms,
            right: delim,
        });
        Ok(())
    }

    /// `\sqrt`：等待 radicand 字段（下一个原子或组）。
    fn math_sqrt(&mut self) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_mode_error("sqrt");
        }
        // \sqrt 本身不是合法字段开头（tex.web scan_math othercases）
        self.check_math_field_break()?;
        self.sqrt_pending = true;
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
        self.radical_pending = Some(delim.unwrap_or(0));
        Ok(())
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

    /// `\mathord` 等：给下一个字段定类。
    fn math_class(&mut self, class: u8) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_mode_error("mathord");
        }
        // \mathord 等不是合法字段开头（tex.web scan_math othercases；连续
        // `\mathord\mathord x` 第二个在此报 Missing { inserted）
        self.check_math_field_break()?;
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
        self.accent_pending = matches!(self.mode(), Mode::Math | Mode::DisplayMath);
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
            width: 0,
            stretch,
            shrink,
            stretch_order: order,
            shrink_order: order,
        });
        Ok(())
    }

    fn token(&mut self, tok: Token) -> Result<()> {
        // `\leaders` 引导盒子已就位：非胶水 token → TeX "Leaders not followed by
        // proper glue"（空格跳过——TeX "get next non-blank non-relax" 语义）。
        if self.leaders_box.is_some() {
            if tok.catcode() == Some(ntex_core::Catcode::Space) {
                return Ok(());
            }
            self.report_leaders_misplaced();
        }
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

    fn group_begin(&mut self, line: u32) -> Result<()> {
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
        // 盒子/对齐组：复用盒子路径（对齐组按 vbox 打包）。`\noalign` 组只补
        // 组类型（7）不另开列表——其材料沿用对齐组列表（此前行为，ETRIP 简化：
        // 不并入外层垂直列表），避免组栈变化影响排版结果。
        let box_kind = match kind {
            Some(GroupKind::HBox | GroupKind::AdjustedHBox) => Some(PendingBox::HBox),
            Some(GroupKind::VBox | GroupKind::Align) => Some(PendingBox::VBox),
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
        // `\leaders` 引导盒子：认领到紧邻的盒子组（组结束封装时挂起等胶水）。
        // 内层盒子（如引导 hbox 里的 \vbox）不消费该标记。
        let leaders = if box_kind.is_some() {
            self.pending_leaders.take()
        } else {
            None
        };
        // `\setbox<n>=<box>` 目标寄存器：认领到**最外层**盒子组（tex.web scan_box
        // box_end 语义）。内层嵌套盒（`\setbox0=\vbox{\hbox{...}}` 的 \hbox）不消费，
        // 否则 target 被第一个内层盒抢走、RHS 的 vbox 无法入寄存器。
        let setbox = if box_kind.is_some() {
            self.setbox_target.take()
        } else {
            None
        };
        self.groups.push(GroupCtx {
            kind: gkind,
            box_kind,
            shipout: ship,
            leaders,
            setbox,
            entered_line: line,
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
            let field = if let Some(is_sup) = self.pending_script.take() {
                Some(MathFieldKind::Script(is_sup))
            } else if self.sqrt_pending {
                self.sqrt_pending = false;
                Some(MathFieldKind::Sqrt)
            } else if let Some(d) = self.radical_pending.take() {
                Some(MathFieldKind::Radical(d))
            } else if self.accent_pending {
                self.accent_pending = false;
                Some(MathFieldKind::Accent)
            } else {
                self.class_pending.take().map(MathFieldKind::Class)
            };
            self.math.push(MathLevel {
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
        // 垂直盒子内容结束时，开放段落先封装（\vbox{a} → vbox[hbox(a)]）。
        // **必须在参数恢复之前**（TeX 语义：段落折行用组内 \hsize——etrip
        // L193 `\vbox{\hsize=0pt...}` 折行用 0pt；恢复后折行会错用外层
        // hsize=469.75pt，行宽/断点全错）。
        if ctx.box_kind.is_some_and(PendingBox::is_vertical) && self.mode() == Mode::Horizontal {
            self.close_paragraph();
        }
        // 先恢复参数镜像（与 VM 的 save_stack 恢复对齐），随后的缩进/interline 用外层值
        if let Some(prev) = self.param_stack.pop() {
            self.params = prev;
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
                    // 普通数学组：先收组内分式（`{a\over b}`），再并入外层
                    let mut lv = level;
                    Self::math_finish_fraction(&mut lv);
                    lv.atoms
                }
            };
            parent.atoms.extend(field_atoms);
            return Ok(());
        }
        match ctx.kind {
            // noalign 组：只补组类型（\currentgrouptype=7），材料留在对齐组列表
            // （group_begin 未为其开新列表，见上）
            GroupKind::NoAlign => {}
            // 对齐组（tex.web alignment）：列/行盒（\cr 分隔）按方向组装——
            // \halign = vbox of hbox 行（行堆叠）；\valign = hbox of vbox 列（列并排）。
            GroupKind::Align => {
                let dir = self.align_dir.take();
                // 最后一段（未 \cr 的尾部内容）也封装为一列
                if let Some(list) = self.lists.last_mut() {
                    let content = std::mem::take(list);
                    if !content.is_empty() {
                        self.align_columns.push(Node::Box(crate::node::vpack(
                            content,
                            self.params.vsize,
                        )));
                    }
                }
                let columns = std::mem::take(&mut self.align_columns);
                self.lists.pop();
                self.list_modes.pop();
                match dir {
                    Some(AlignDir::Halign) => {
                        // 行堆叠：vbox of 列盒（\halign 数据行 → vbox）
                        let v = crate::node::vpack(columns, self.params.vsize);
                        if let Some(outer) = self.lists.last_mut() {
                            outer.push(Node::Box(v));
                        }
                    }
                    Some(AlignDir::Valign) => {
                        // 列并排：hbox of 列盒（\valign 数据列 → hbox）
                        let h = crate::node::hpack(&columns, self.params.hsize);
                        if let Some(outer) = self.lists.last_mut() {
                            outer.push(Node::Box(h));
                        }
                    }
                    None => {}
                }
            }
            GroupKind::HBox | GroupKind::AdjustedHBox | GroupKind::VBox | GroupKind::VTop => {
                if let Some(kind) = ctx.box_kind {
                    let leaders = ctx.leaders;
                    self.package_box(kind, ctx.shipout, leaders, ctx.setbox);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn primitive(&mut self, prim: Primitive) -> Result<()> {
        // TeX box_end leader 分支：盒子后必须是 \hskip/\vskip 胶水，否则
        // "Leaders not followed by proper glue" 报错并丢弃引导盒子。
        if self.leaders_box.is_some() {
            self.report_leaders_misplaced();
        }
        // tex.web scan_math：待定数学字段遇原语事件（非字符非 {）→ Missing { inserted
        self.check_math_field_break()?;
        match prim {
            Primitive::HBox => self.pending_box = Some(PendingBox::HBox),
            Primitive::VBox => self.pending_box = Some(PendingBox::VBox),
            Primitive::VTop => self.pending_box = Some(PendingBox::VTop),
            // TRIP 冲刺：\leaders/\cleaders/\xleaders —— 引导符，等待其后的盒子
            // （tex.web scan_box(leader_flag+kind)；盒子经 group/rule 路径挂起）。
            Primitive::Leaders => self.pending_leaders = Some(LeadersKind::Leaders),
            Primitive::Cleaders => self.pending_leaders = Some(LeadersKind::Cleaders),
            Primitive::XLeaders => self.pending_leaders = Some(LeadersKind::Xleaders),
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
            Primitive::NoIndent => {
                // 垂直模式 \noindent 立即开段（TeX new_graf(0)，缩进 0）——
                // 不只设标志等字符触发：`\vbox{\noindent\hbox{...}}` 的 \hbox
                // 必须在水平模式（否则盒被当 AdjustedHBox 进垂直列表、模式错乱）；
                // 水平模式无操作。
                if self.mode() == Mode::Vertical {
                    self.noindent_next = true;
                    self.lists.push(Vec::new());
                    self.list_modes.push(Mode::Horizontal);
                    self.space_factor = 1000;
                    self.insert_indent(); // noindent_next=true → 跳过缩进盒
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
            // M4：\mathaccent（\accent 的数学等价，已报错改道）：记录待定重音符
            // 字段，nucleus 字段由后续 token/组补齐（tex.web math_ac）。
            Primitive::MathAccent if matches!(self.mode(), Mode::Math | Mode::DisplayMath) => {
                self.accent_pending = true;
            }
            // TRIP 冲刺：\accent 在数学模式报错恢复（TRIP L396）
            Primitive::Accent => {
                if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
                    self.write16(
                        "! Please use \\mathaccent for accents in math mode.\n".to_string(),
                    )?;
                    self.accent_pending = true;
                }
            }
            // TeX \error 原语（tex.web @<Report an improper...@> /
            // error 命令：报告 "! OK." 并进入错误交互——nonstop/batch 模式下
            // 仅转录后继续；trip.tex 多处 \error 依赖此消息，参考 log 14 处）。
            Primitive::Error => {
                let mode = self.params.misc[19]; // interactionmode（0=batch 1=nonstop 2=scroll 3=errorstop）
                self.transcript.push_str("! OK.\n");
                if mode >= 2 {
                    self.transcript.push_str("(Please type a command or say \\end)\n");
                }
                if mode == 0 || mode == 1 {
                    self.transcript.push_str(
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
            // `\setbox0=\lastbox`：优先取 \lastbox 摘下的盒子
            let b = self
                .lastbox_hold
                .take()
                .or_else(|| self.boxes.get_mut(idx).and_then(|s| s.take()));
            self.boxes[target] = b;
            return Ok(());
        }
        // `\box0` 紧跟在 `\lastbox` 后：取摘下的盒子（TeX 语义）
        let b = self
            .lastbox_hold
            .take()
            .or_else(|| {
                if idx == 255 {
                    self.pending_pages.pop_front()
                } else {
                    self.boxes.get_mut(idx).and_then(|s| s.take())
                }
            });
        let Some(b) = b else {
            // TeX：\box 取 void 盒子 → 空 hbox 节点（tex.web：仍产生节点；TRIP L104 前 \copy200 void）
            self.append(Node::Box(crate::node::BoxNode::new_hbox(Vec::new())));
            return Ok(());
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

    /// 当前模式名（`\tracingcommands` 追踪；tex.web print_mode 语义）。
    fn mode_name(&self) -> String {
        match self.mode() {
            Mode::Vertical => "vertical mode".to_string(),
            Mode::Horizontal => "horizontal mode".to_string(),
            Mode::RestrictedHorizontal => "restricted horizontal mode".to_string(),
            Mode::Math => "math mode".to_string(),
            Mode::DisplayMath => "display math mode".to_string(),
        }
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
        // `\leaders` 引导盒子已就位：\hskip/\vskip 胶水到来 → 组成 Leader 节点
        // （tex.web box_end leader 分支：append_glue + subtype + leader_ptr）。
        if let Some((kind, box_node)) = self.leaders_box.take() {
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
        // `\leaders\hrule/\vrule`：rule 作引导内容（tex.web scan_box leader 分支
        // 允许 hrule/vrule；宽度保持 NULL_FLAG，showbox 显示 `x*`）。
        if let Some(kind) = self.pending_leaders.take() {
            self.leaders_box = Some((kind, Node::Rule { width, height, depth }));
            return Ok(());
        }
        // 数学模式 `\vrule`：M4-1 忽略（规则原子后续补）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Ok(());
        }
        // 非引导上下文：未定宽度（TeX 在 hpack/vpack 解析；NTex 简化落 0）。
        let width = if width == ntex_core::NULL_FLAG { 0 } else { width };
        self.append(Node::Rule { width, height, depth });
        Ok(())
    }

    /// TeXXeT 方向节点：追加到当前列表（宽度 0 占位）。
    fn direction_node(&mut self, kind: ntex_core::sink::DirectionKind) -> Result<()> {
        self.append(Node::Direction { kind });
        Ok(())
    }

    /// `\mark`/`\marks<n>`：mark 节点追加到当前列表（无维度）。
    /// 同步更新当前页 marks_first/marks_bot（class=None 映射到 0，即 \mark=\marks0）。
    fn mark(&mut self, class: Option<i64>, text: String) -> Result<()> {
        // TeX 语义：\mark 等价于 \marks0（class 0）。
        let c = class.unwrap_or(0);
        // marks_first：该 class 在当前页第一次出现时设置。
        self.marks_first.entry(c).or_insert_with(|| text.clone());
        // marks_bot：每次出现都更新（最后一次出现）。
        self.marks_bot.insert(c, text.clone());
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

    // ---- ETRIP 冲刺：e-TeX marks 族查询 ----
    // （注意：轮转在 feed_one 产出页时立即执行，不在查询时修改状态。）
    fn topmarks(&self, class: i64) -> String {
        self.marks_top.get(&class).cloned().unwrap_or_default()
    }
    fn firstmarks(&self, class: i64) -> String {
        self.marks_first.get(&class).cloned().unwrap_or_default()
    }
    fn botmarks(&self, class: i64) -> String {
        self.marks_bot.get(&class).cloned().unwrap_or_default()
    }
    fn splitfirstmarks(&self, class: i64) -> String {
        self.marks_split_first
            .get(&class)
            .cloned()
            .unwrap_or_default()
    }
    fn splittopmarks(&self, class: i64) -> String {
        self.marks_split_top
            .get(&class)
            .cloned()
            .unwrap_or_default()
    }
    fn splitbotmarks(&self, class: i64) -> String {
        self.marks_split_bot
            .get(&class)
            .cloned()
            .unwrap_or_default()
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

    /// 盒子寄存器种类（0=void、1=hbox、2=vbox）；`\ifvoid`/`\ifhbox`/`\ifvbox` 用。
    fn box_register_kind(&self, idx: usize) -> i64 {
        match self.boxes.get(idx).and_then(|s| s.as_ref()) {
            None => 0,
            Some(b) => match b.kind {
                crate::node::BoxKind::HBox => 1,
                crate::node::BoxKind::VBox => 2,
            },
        }
    }

    /// `\lastpenalty`：当前列表尾若是 penalty 节点返回其值，否则 0（TeX 语义）。
    fn last_penalty(&self) -> i64 {
        match self.lists.last().and_then(|l| l.last()) {
            Some(Node::Penalty { penalty }) => *penalty,
            _ => 0,
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
                self.lastbox_hold = Some(b);
            }
        }
        // `\setbox0=\lastbox`：摘下的盒子存入目标寄存器（tex.web last_box →
        // cur_box → set_box 赋值语义）；裸 \lastbox 留给后续 \box 消费。
        if let Some(t) = self.setbox_target.take() {
            if let Some(b) = self.lastbox_hold.take() {
                self.boxes[t] = Some(b);
            }
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

    /// `\copy<n>`：复制盒子寄存器为节点追加到当前列表（原寄存器保留）。
    fn copy_box(&mut self, idx: usize) -> Result<()> {
        let b = if let Some(h) = self.lastbox_hold.take() {
            // `\copy0` 紧跟在 `\lastbox` 后：复制摘下的盒子
            h.clone()
        } else {
            let Some(b) = self.boxes.get(idx).and_then(|s| s.as_ref()) else {
                // TeX：\copy 取 void 盒子 → **空 hbox 节点**（tex.web copy_scan_box：
                // void → null box，仍产生节点触发 freeze/interline；TRIP L104 `\copy200`）
                self.append(Node::Box(crate::node::BoxNode::new_hbox(Vec::new())));
                return Ok(());
            };
            b.clone()
        };
        // `\setbox<n>=\copy<m>`：复制结果存入目标寄存器（\copy 不消耗原盒）
        if let Some(target) = self.setbox_target.take() {
            self.boxes[target] = Some(b);
            return Ok(());
        }
        self.append(Node::Box(b));
        Ok(())
    }

    /// `\unhbox<n>`/`\unhcopy<n>`：hbox 拆开，子节点追加到当前列表。
    /// TeX：void 盒或类型不符 → "! Incompatible list can't be unboxed."
    /// 报错恢复（空操作继续；TRIP L396 `\unhbox234`——234 未设置）。
    fn unhbox(&mut self, idx: usize, copy: bool) -> Result<()> {
        let Ok(b) = self.take_or_clone_box(idx, copy) else {
            self.unbox_error_continue();
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
        let Ok(b) = self.take_or_clone_box(idx, copy) else {
            self.unbox_error_continue();
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

    /// `\wd/\ht/\dp<n>`：盒子寄存器维度（void 为 0）。
    fn box_dim(&self, idx: usize, dim: u8) -> i64 {
        let Some(b) = self.boxes.get(idx).and_then(|s| s.as_ref()) else {
            return 0;
        };
        match dim {
            0 => b.width,
            1 => b.height,
            _ => b.depth,
        }
    }

    /// `\wd/\ht/\dp<n>=<dimen>`：设置盒子寄存器维度。
    fn set_box_dim(&mut self, idx: usize, dim: u8, value: i64) -> Result<()> {
        let Some(b) = self.boxes.get_mut(idx).and_then(|s| s.as_mut()) else {
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

    /// `\showgroups`：把组上下文栈格式化为转录（诊断用）。
    fn showgroups(&mut self) -> Result<()> {
        let mut out = String::from("### begin group\n");
        for (i, g) in self.groups.iter().enumerate() {
            out.push_str(&format!("level {i}: {:?} (code {})\n", g.kind, g.kind.code()));
        }
        out.push_str("### end group\n");
        self.transcript.push_str(&out);
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
        self.transcript.push_str(&out);
        Ok(())
    }

    /// `\begingroup`：下一个组为半简单组（14）。
    fn semisimple_begin(&mut self) -> Result<()> {
        self.pending_kind = Some(GroupKind::SemiSimple);
        Ok(())
    }

    /// `\valign{`/`\halign{`：下一个组为对齐组（6）。
    fn align_begin(&mut self, is_halign: bool) -> Result<()> {
        self.align_dir = Some(if is_halign { AlignDir::Halign } else { AlignDir::Valign });
        self.pending_kind = Some(GroupKind::Align);
        Ok(())
    }

    /// `\noalign{`：下一个组为无对齐组（7）。
    fn noalign_begin(&mut self) -> Result<()> {
        self.pending_kind = Some(GroupKind::NoAlign);
        Ok(())
    }

    /// 输出例程的隐式组：组种类 8（output group，tex.web group_code）。
    fn output_routine_begin(&mut self) -> Result<()> {
        self.pending_kind = Some(GroupKind::Output);
        Ok(())
    }

    /// `\cr`：对齐行/列结束——当前列表内容封装为列盒（tex.web alignment
    /// 数据行边界；此前简化 no-op 导致列内容混在 vbox）。
    fn align_row_end(&mut self) -> Result<()> {
        if self.align_dir.is_none() {
            return Ok(());
        }
        // 数据列内开放的段落先封装（\noindent 开段的列内容——tex.web 列处理
        // 每列一个段落；不 close 则段落悬空、tracingparagraphs 输出丢失）
        if self.mode() == Mode::Horizontal {
            self.close_paragraph();
        }
        let Some(list) = self.lists.last_mut() else {
            return Ok(());
        };
        let content = std::mem::take(list);
        if content.is_empty() {
            return Ok(());
        }
        let v = crate::node::vpack(content, self.params.vsize);
        self.align_columns.push(Node::Box(v));
        Ok(())
    }

    /// `\raise`/`\lower<dimen>`：记录盒子参考点位移（下一个封装盒子生效）。
    fn raise(&mut self, amount: i64) -> Result<()> {
        self.pending_shift = Some(amount);
        Ok(())
    }

    /// `\moveleft<dimen>`：记录盒子水平左移（下一个封装盒子生效；TRIP 冲刺简化）。
    fn move_left(&mut self, amount: i64) -> Result<()> {
        self.pending_hshift = Some(-amount);
        Ok(())
    }

    /// `\moveright<dimen>`：记录盒子水平右移（下一个封装盒子生效；TRIP 冲刺简化）。
    fn move_right(&mut self, amount: i64) -> Result<()> {
        self.pending_hshift = Some(amount);
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
            // TeX：\showbox 空盒 → 显示 void 并恢复（TRIP 中 box 状态差异不致命）
            let out = format!("> \\box{idx}=\nvoid\n! OK.\n");
            self.transcript.push_str(&out);
            return Ok(());
        };
        let mut out = format!("> \\box{idx}=\n");
        showbox_format_box(b, 0, &self.fonts, &self.font_cs_names, &mut out);
        out.push_str("! OK.\n");
        self.transcript.push_str(&out);
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any_ref(&self) -> &dyn std::any::Any {
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

/// sp → pt 字符串（tex.web print_scaled：整数 + 最多 5 位小数去尾 0；
/// 余数 0 显示 `X.0`——如 `4.4`、`1055.44061`、`-1.0`）。
fn showbox_pt(sp: i64) -> String {
    let neg = sp < 0;
    let sp = sp.abs();
    let int = sp / SP_PER_PT;
    let rem = sp % SP_PER_PT;
    let sign = if neg { "-" } else { "" };
    if rem == 0 {
        return format!("{sign}{int}.0");
    }
    let frac = rem * 100_000 / SP_PER_PT;
    let frac_s = format!("{frac:05}").trim_end_matches('0').to_string();
    format!("{sign}{int}.{frac_s}")
}

/// 胶水阶名（tex.web print_glue：1=fil、2=fill、3=filll、0 无）。
fn order_name(order: u8) -> &'static str {
    match order {
        1 => "fil",
        2 => "fill",
        3 => "filll",
        _ => "",
    }
}

fn showbox_format_box(
    b: &BoxNode,
    depth: usize,
    fonts: &Fonts,
    cs_names: &[Option<String>],
    out: &mut String,
) {
    let p = ".".repeat(depth);
    let kind = match b.kind {
        BoxKind::HBox => "hbox",
        BoxKind::VBox => "vbox",
    };
    out.push_str(&format!(
        "{p}\\\\{kind}({}+{})x{}",
        showbox_pt(b.height),
        showbox_pt(b.depth),
        showbox_pt(b.width)
    ));
    // tex.web show_node_list：shift ≠ 0 时追加 ", shifted <dimen>"
    // （\raise/\lower 参考点位移、\vtop 基线移到首行、\moveleft/\moveright 水平位移）
    if b.shift != 0 {
        out.push_str(&format!(", shifted {}", showbox_pt(b.shift)));
    }
    out.push('\n');
    for c in &b.children {
        showbox_format_node(c, depth + 1, fonts, cs_names, out);
    }
}

fn showbox_format_node(
    n: &Node,
    depth: usize,
    fonts: &Fonts,
    cs_names: &[Option<String>],
    out: &mut String,
) {
    let p = ".".repeat(depth + 1);
    match n {
        Node::Box(b) => showbox_format_box(b, depth, fonts, cs_names, out),
        Node::Char {
            font, charcode, ..
        } => {
            // TeX show_node_list：`.<字体名> <字符>`（如 `.\trip 1`，字符直接显示）
            let c = char::from_u32(*charcode)
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("{charcode}"));
            let cs = cs_names
                .get(font.0 as usize)
                .and_then(|n| n.as_ref())
                .cloned()
                .unwrap_or_else(|| fonts.font_name(*font));
            out.push_str(&format!("{p}\\\\{} {c}\n", cs));
        }
        Node::Glue {
            width,
            stretch,
            shrink,
            stretch_order,
            shrink_order,
        } => {
            let mut s = format!("{p}\\glue {}", showbox_pt(*width));
            if *stretch != 0 {
                s.push_str(&format!(
                    " plus {}{}",
                    showbox_pt(*stretch),
                    order_name(*stretch_order)
                ));
            }
            if *shrink != 0 {
                s.push_str(&format!(
                    " minus {}{}",
                    showbox_pt(*shrink),
                    order_name(*shrink_order)
                ));
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
        } => {
            // 未定宽度（NULL_FLAG，如 `\leaders\hrule` 引导）显示 `*`（tex.web print_rule_dimen）
            let w = if *width == ntex_core::NULL_FLAG {
                "*".to_string()
            } else {
                showbox_pt(*width)
            };
            out.push_str(&format!(
                "{p}\\rule({}+{})x{}\n",
                showbox_pt(*height),
                showbox_pt(*depth),
                w
            ));
        }
        Node::Leaders {
            kind,
            width,
            stretch,
            shrink,
            inner,
        } => {
            // TeX show_box：`\{kind} {胶水规格}` + 引导内容作为子节点递归显示
            // （tex.web "Display leaders"：node_list_display(leader_ptr)）。
            let name = match kind {
                LeadersKind::Leaders => "leaders",
                LeadersKind::Cleaders => "cleaders",
                LeadersKind::Xleaders => "xleaders",
            };
            let mut s = format!("{p}\\{name} {}", showbox_pt(*width));
            if *stretch != 0 {
                s.push_str(&format!(" plus {}", showbox_pt(*stretch)));
            }
            if *shrink != 0 {
                s.push_str(&format!(" minus {}", showbox_pt(*shrink)));
            }
            s.push('\n');
            out.push_str(&s);
            showbox_format_node(inner, depth + 1, fonts, cs_names, out);
        }
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
            Some(c) => out.push_str(&format!("{p}\\\\marks{c}{{{text}}}\n")),
            None => out.push_str(&format!("{p}\\\\mark{{{text}}}\n")),
        },
        Node::Ins { class, text } => out.push_str(&format!("{p}\\insert{class} {text}\n")),
        Node::Adjust { text } => out.push_str(&format!("{p}\\vadjust {text}\n")),
        Node::Whatsit { text } => out.push_str(&format!("{p}\\write {text}\n")),
        // 数学边界标记（tex.web math_node）：`.\mathon`；\mathsurround 非 0
        // 时补 `, surrounded X`（参考 etrip.log `\mathon, surrounded 12.3`）
        Node::MathOn { surrounded } | Node::MathOff { surrounded } => {
            let name = if matches!(n, Node::MathOn { .. }) {
                "mathon"
            } else {
                "mathoff"
            };
            if *surrounded != 0 {
                out.push_str(&format!(
                    "{p}\\{name}, surrounded {}\n",
                    showbox_pt(*surrounded)
                ));
            } else {
                out.push_str(&format!("{p}\\{name}\n"));
            }
        }
    }
}
