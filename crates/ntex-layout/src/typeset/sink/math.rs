use super::*;

impl MathSink for NodeBuilder {
    fn math_choice_begin(&mut self) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_mode_error("mathchoice");
        }
        self.math_state.choices.push(MathChoicePending {
            branches: Vec::with_capacity(4),
            awaiting_group: true,
        });
        Ok(())
    }
    /// \mathcode 改道只认**真实列表模式**（tex.web mmode+letter/other）：
    /// 数学内文本盒（`\hbox{…}`/`\mbox`/`\text`）里是正文字符。
    fn math_code_applies(&self) -> bool {
        matches!(self.mode(), Mode::Math | Mode::DisplayMath)
    }
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
                            name: None,
                            width: ps.width,
                            stretch: ps.stretch,
                            shrink: ps.shrink,
                            stretch_order: ps.stretch_order,
                            shrink_order: ps.shrink_order,
                        });
                    }
                    // tex.web：垂直模式 `$$` 不是直接进显示——`vmode+math_shift` 先
                    // `back_input; new_graf(true)`（parskip + 缩进盒开段），init_math
                    // 才在水平模式见到非空 hlist → line_break 对「只含缩进盒的段落」
                    // 折行：垂直列表落一个高 0 深 0 的空行盒（与上一行之间按
                    // \baselineskip-\prevdepth 插行间 glue），w = 缩进盒宽 + 2em
                    // （`\noindent$$` 无缩进盒 → head=tail 臂 → -max_dimen）。
                    // 段末公式与上文之间 ~\baselineskip 的空隙 + \predisplaysize
                    // 语义都由此而来；直接 -max_dimen 会让公式顶到上一行上。
                    self.lists.push(Vec::new());
                    self.list_modes.push(Mode::Horizontal);
                    self.space_factor = 1000; // new_graf：段落开始重置 spacefactor
                    self.insert_indent();
                    let last_natural = self.close_paragraph();
                    self.math_state.predisplay_size = match last_natural {
                        Some(w) => w + 2 * self.fonts.font_param(self.current_font, 6),
                        None => -ntex_core::register::MAX_DIMEN,
                    };
                    self.enter_display_math()
                } else {
                    // 行内数学：开段（TeX new_graf）
                    if self.page_state.pagination {
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
    /// 这个 `$` 是收还是开：数学层在栈上（Math/DisplayMath）= 收，否则 = 开。
    /// 与 `math_shift` 的模式分派同一真源。
    fn math_shift_will_close(&self) -> bool {
        matches!(self.mode(), Mode::Math | Mode::DisplayMath)
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
            .math_state
            .math
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
            return Err(Error::invalid_input(
                "\\over 前不能有未挂脚本（Missing { inserted）",
            ));
        }
        let num = std::mem::take(&mut level.atoms);
        let delims = std::mem::take(&mut level.frac_delims);
        level.fraction = Some(FractionPending {
            thickness,
            num,
            delims,
        });
        Ok(())
    }
    /// `\overwithdelims` 系定界符：tex.web 在 fraction 节点左/右字段（make_fraction
    /// 包在分式两侧）。存当前数学层，`math_fraction` 打包时并入。
    fn math_fraction_delims(&mut self, left: Option<u32>, right: Option<u32>) -> Result<()> {
        if let Some(level) = self.math_state.math.last_mut() {
            level.frac_delims = (left, right);
        }
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
            .math_state
            .math
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
            .math_state
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
    /// e-TeX `\middle<delim>`：在 \left...\right 体内插入定界符原子（类 Inner）。
    fn math_middle(&mut self, delim: Option<u32>) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_mode_error("middle");
        }
        let level = self
            .math_state
            .math
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
    /// `\limits`/`\nolimits`/`\displaylimits`（tex.web math_limit_switch）：
    /// 改写当前数学层**最后一个原子**（须为 Op 字符原子）的上下限摆放方式。
    /// mode：0=displaylimits/normal、1=limits、2=nolimits。
    /// 最后原子不是 Op 时报 tex.web 同名错误（"Limit controls must follow a
    /// math operator."）；非数学模式无害忽略（单独 `\limits` 的错误口径待
    /// etrip 校准，见 primitive_math.rs 原注释）。
    fn math_limit_switch(&mut self, mode: u8) -> Result<()> {
        let Some(level) = self.math_state.math.last_mut() else {
            return Ok(());
        };
        let after_op = matches!(level.atoms.last(), Some(MathAtom::Char(mc)) if mc.class == MathClass::Op)
            || matches!(
                level.atoms.last(),
                Some(MathAtom::Classed {
                    class: MathClass::Op,
                    ..
                })
            );
        if !after_op {
            return self.write16("! Limit controls must follow a math operator.\n".to_owned());
        }
        match level.atoms.last_mut() {
            Some(MathAtom::Char(mc)) => mc.limits = mode,
            Some(MathAtom::Classed { limits, .. }) => *limits = mode,
            _ => unreachable!("op_noad checked above"),
        }
        Ok(())
    }
    /// `\accent`（数学模式）：TeX 报错改道为 `\mathaccent`（tex.web math_ac），
    /// <15-bit number> + nucleus 字段继续扫描（TRIP L396 `\accent\x\vfill`）。
    /// 水平/受限水平模式的 `\accent` 是合法文本重音——只在数学模式报错改道。
    fn math_accent(&mut self, plain: bool) -> Result<()> {
        // \mathaccent 不是合法字段开头（tex.web scan_math othercases）
        self.check_math_field_break()?;
        if plain && matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            self.write16("! Please use \\mathaccent for accents in math mode.\n".to_string())?;
        }
        self.math_state.accent_pending = matches!(self.mode(), Mode::Math | Mode::DisplayMath);
        Ok(())
    }
    fn math_accent_char(&mut self, code: u32) -> Result<()> {
        if self.math_state.accent_pending {
            let accent = MathAtom::Char(MathChar {
                class: MathClass::Ord,
                fam: ((code >> 8) & 15) as u8,
                charcode: code & 255,
                limits: 0,
            });
            if let Some(level) = self.math_state.math.last_mut() {
                level.atoms.push(MathAtom::Accent {
                    accent: vec![accent],
                    nucleus: Vec::new(),
                });
            }
        }
        Ok(())
    }
    /// `\underline`：等待字段组（数学模式；组开收为 Underline 原子）。
    fn math_underline(&mut self) -> Result<()> {
        self.check_math_field_break()?;
        self.math_state.underline_pending = matches!(self.mode(), Mode::Math | Mode::DisplayMath);
        Ok(())
    }
    /// `\eqno`/`\leqno`（tex.web start_eq_no → after_math）：当前层已收原子
    /// 是公式 mlist（p），切出去暂存；其后材料进新列表，close_math 时
    /// hpack natural 成编号盒 a（`leqno=true` → 编号在左）。
    fn math_eqno(&mut self, leqno: bool) -> Result<()> {
        let Some(level) = self.math_state.math.last_mut() else {
            return Ok(());
        };
        let atoms = std::mem::take(&mut level.atoms);
        self.math_state.eqno_formula = Some(atoms);
        self.math_state.eqno_side = Some(leqno);
        Ok(())
    }
    /// `\mathchar<15-bit>`：完整数学字符原子（tex.web math_char）——
    /// 数学模式直接产出 Char 原子（class=n>>12&7、fam=n>>8&15、charcode=n&255）；
    /// 文本模式同 `\char` 输出低 8 位字符。此前 scan_number 即丢。
    fn math_char_full(&mut self, n: u32) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.token(ntex_core::Token::char(ntex_core::Catcode::Other, n & 0xFF));
        }
        let class = Self::class_of(((n >> 12) & 7) as u8);
        let fam = ((n >> 8) & 0xF) as u8;
        let charcode = n & 0xFF;
        self.math_push_atom(MathAtom::Char(MathChar {
            class,
            fam,
            charcode,
            limits: 0,
        }))
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
