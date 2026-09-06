impl NodeBuilder {
    // ---------- M4-1 数学模式 ----------

    /// 数学专用原语在非数学模式：TeX 报 "You can't use \x in <mode> mode." 并恢复
    /// （ETRIP：错误入转录继续，不再致命终止）。
    fn math_mode_error(&mut self, prim: &str) -> Result<()> {
        let mode = match self.mode() {
            Mode::Vertical => "vertical",
            Mode::Horizontal => "horizontal",
            Mode::RestrictedHorizontal => "restricted horizontal",
            Mode::Math => "math",
            Mode::DisplayMath => "display math",
        };
        self.write16(format!("! You can't use \\{prim} in {mode} mode.\n"))?;
        Ok(())
    }

    /// 进入数学模式：压数学层 + 占位列表层（公式节点经 close_math 落回上层列表）。
    /// `mode` 为 Math（行内，textstyle）或 DisplayMath（显示，displaystyle）。
    fn enter_math(&mut self, mode: Mode) -> Result<()> {
        let style = match mode {
            Mode::DisplayMath => MathStyle::Display,
            _ => MathStyle::Text,
        };
        self.math.push(MathLevel::default());
        self.math_style = style;
        self.lists.push(Vec::new());
        self.list_modes.push(mode);
        Ok(())
    }

    /// M4-4 进入显示数学：先插 `\predisplaypenalty` + `\abovedisplayskip`（或短变体），
    /// 再进入 DisplayMath 模式（公式原子收集，退出时经 [`Self::close_math`] 落垂直列表）。
    fn enter_display_math(&mut self) -> Result<()> {
        self.append(Node::Penalty {
            penalty: self.params.predisplaypenalty,
        });
        let above = if self.display_short {
            self.params.abovedisplayshortskip
        } else {
            self.params.abovedisplayskip
        };
        self.append(Node::Glue {
            name: None,            width: above.width,
            stretch: above.stretch,
            shrink: above.shrink,
            stretch_order: 0,
            shrink_order: 0,
        });
        self.enter_math(Mode::DisplayMath)
    }

    /// 退出数学模式：数学列表转节点追加到上层列表。
    /// 显示公式（M4-4）：收为 `\hbox to \hsize` 居中盒 + `\belowdisplayskip` +
    /// `\postdisplaypenalty`（垂直元素）；行内公式：节点直通当前列表。
    fn close_math(&mut self) -> Result<()> {
        if self.pending_script.is_some() {
            return Err(Error::invalid_input(
                "数学模式中 ^/_ 后缺少上标/下标（Missing { inserted）",
            ));
        }
        let mut level = self
            .math
            .pop()
            .ok_or_else(|| Error::internal("close_math 无数学层"))?;
        let style = self.math_style;
        let was_display = self.list_modes.pop() == Some(Mode::DisplayMath);
        self.lists.pop();
        // 公式末尾收尾：未闭合 \left 报错恢复（TeX "Extra } or forgotten \right."，
        // 自动闭合；TRIP L298 `\left(\over\left(...`）；待定分式收尾（TeX 允许空分母）
        if level.left.is_some() {
            self.report_error("Extra } or forgotten \\right.");
            level.left = None;
        }
        Self::math_finish_fraction(&mut level);
        let mut nodes = self.math_to_hlist(&level.atoms, style);
        // 行内数学边界标记（tex.web math_node）：`$` 进入/退出插 \\mathon/\\mathoff
        // （无维度；showbox 显示 `..\\mathon`。显示数学的公式盒内 TeX 同样有
        // math_node——本引擎显示公式走 `\\hbox to \\hsize` 盒，先只做行内）。
        let ms = self.params.mathsurround;
        if !was_display {
            nodes.insert(0, Node::MathOn { surrounded: ms });
            nodes.push(Node::MathOff { surrounded: ms });
        }
        if was_display {
            // 公式盒 = `\hbox to \hsize`（两侧 \hfil 居中；displaywidth≈\hsize）
            let mut line: Vec<Node> = Vec::with_capacity(nodes.len() + 2);
            line.push(Node::Glue {
            name: None,                width: 0,
                stretch: 1,
                shrink: 0,
                stretch_order: GLUE_ORDER_FIL,
                shrink_order: 0,
            });
            line.extend(nodes);
            line.push(Node::Glue {
            name: None,                width: 0,
                stretch: 1,
                shrink: 0,
                stretch_order: GLUE_ORDER_FIL,
                shrink_order: 0,
            });
            self.append(Node::Box(hpack(&line, self.params.hsize)));
            // \belowdisplayskip（或短变体）+ \postdisplaypenalty
            let below = if self.display_short {
                self.params.belowdisplayshortskip
            } else {
                self.params.belowdisplayskip
            };
            self.append(Node::Glue {
            name: None,                width: below.width,
                stretch: below.stretch,
                shrink: below.shrink,
                stretch_order: 0,
                shrink_order: 0,
            });
            self.append(Node::Penalty {
                penalty: self.params.postdisplaypenalty,
            });
            // 后续文字续排：无 parskip/缩进（TeX 公式仍在段内）
            self.after_display = true;
        } else {
            for n in nodes {
                self.append(n);
            }
        }
        Ok(())
    }

    /// 数学模式字符：`^`/`_` 设待挂脚本；字母/其他 → Ord 原子（M4-2 原子类化）。
    fn math_char_tok(&mut self, tok: Token) -> Result<()> {
        let (Some(cat), Some(ch)) = (tok.catcode(), tok.charcode()) else {
            return Ok(());
        };
        // 字符属于 scan_math 的单字符字段：重音符 nucleus 在重音符字段之后到来
        // （`\mathaccent 16 x`），经 math_push_atom 挂为 nucleus 原子（不报错）。
        match cat {
            ntex_core::Catcode::Superscript => {
                self.pending_script = Some(true);
                Ok(())
            }
            ntex_core::Catcode::Subscript => {
                self.pending_script = Some(false);
                Ok(())
            }
            ntex_core::Catcode::Letter | ntex_core::Catcode::Other => self
                .math_push_atom(MathAtom::Char(MathChar {
                    class: MathClass::Ord,
                    fam: 0,
                    charcode: ch,
                })),
            _ => Ok(()), // active 等已在展开侧处理；其余忽略
        }
    }

    /// 追加原子到当前数学层；处理待定字段（`\sqrt`/`\mathbin`/`\nonscript`）与
    /// 脚本挂载（`x^2`/`x_i`/`x_i^2`）。
    fn math_push_atom(&mut self, atom: MathAtom) -> Result<()> {
        // `\sqrt` 单原子字段：`\sqrt x`
        if self.sqrt_pending {
            self.sqrt_pending = false;
            let level = self
                .math
                .last_mut()
                .ok_or_else(|| Error::internal("数学原子无数学层"))?;
            level.atoms.push(MathAtom::Radical { base: vec![atom] });
            return Ok(());
        }
        // `\radical<delim>` 单原子字段：`\radical"3 x`（TRIP L412 everymath 注入路径）
        if let Some(_delim) = self.radical_pending.take() {
            let level = self
                .math
                .last_mut()
                .ok_or_else(|| Error::internal("数学原子无数学层"))?;
            level.atoms.push(MathAtom::Radical { base: vec![atom] });
            return Ok(());
        }
        // `\accent`/`\mathaccent` 单原子 nucleus 字段：第一个原子充当重音符，
        // 第二个原子是被重音内容（tex.web math_ac：accent 字段在前）。
        if self.accent_pending {
            if let Some(level) = self.math.last_mut() {
                if let Some(MathAtom::Accent { nucleus, .. }) = level.atoms.last_mut() {
                    nucleus.push(atom);
                    self.accent_pending = false;
                    return Ok(());
                }
            }
        }
        // `\mathbin` 等单原子字段：`\mathbin+`（Char 改类，其余包 Classed）。
        // 单字符是合法字段（tex.web scan_math letter 分支：`\mathord x` 不报错）；
        // 非字符非 { token 的 Missing { inserted 由 check_math_field_break 负责
        //（原语事件入口）。
        if let Some(class) = self.class_pending.take() {
            let atom = match atom {
                MathAtom::Char(mut mc) => {
                    mc.class = class;
                    MathAtom::Char(mc)
                }
                other => MathAtom::Classed {
                    class,
                    content: vec![other],
                },
            };
            return self.math_push_atom_raw(atom);
        }
        // `\nonscript`：下一个数学空格标记为脚本模式丢弃
        let atom = match atom {
            MathAtom::MSkip {
                width,
                stretch,
                shrink,
                ..
            } if self.nonscript_pending => MathAtom::MSkip {
                width,
                stretch,
                shrink,
                nonscript: true,
            },
            other => other,
        };
        self.nonscript_pending = false;
        self.math_push_atom_raw(atom)
    }

    /// 原始追加（含脚本挂载）：`x^2`/`x_i`/`x_i^2`。
    fn math_push_atom_raw(&mut self, atom: MathAtom) -> Result<()> {
        // 先探测原子缺失（报错写 transcript 需 &mut self，避免与 level 借用冲突）
        if self.pending_script.is_some()
            && self.math.last().is_some_and(|l| l.atoms.is_empty())
        {
            // TeX：^/_ 前无原子 → "Missing { inserted" 恢复（插入空原子；TRIP L263）
            self.report_error("Missing { inserted.");
        }
        // 重音符 nucleus 字段在重音符之后（`\accent\x`）：TeX scan_math 对非字符
        // token 报 "Missing { inserted" 放回重扫（trip.tex L396）。
        if self.accent_pending
            && self
                .math
                .last()
                .is_some_and(|l| {
                    matches!(l.atoms.last(), Some(MathAtom::Accent { nucleus, .. }) if nucleus.is_empty())
                })
        {
            self.report_error("Missing { inserted.");
        }
        let level = self
            .math
            .last_mut()
            .ok_or_else(|| Error::internal("数学原子无数学层"))?;
        if let Some(is_sup) = self.pending_script.take() {
            let mut base = match level.atoms.pop() {
                Some(b) => b,
                None => MathAtom::Classed {
                    class: MathClass::Ord,
                    content: Vec::new(),
                },
            };
            if let MathAtom::Scripts { sub, sup, .. } = &mut base {
                if is_sup {
                    if sup.is_some() {
                        return Err(Error::invalid_input("双重上标（Double superscript）"));
                    }
                    *sup = Some(vec![atom]);
                } else {
                    if sub.is_some() {
                        return Err(Error::invalid_input("双重下标（Double subscript）"));
                    }
                    *sub = Some(vec![atom]);
                }
                level.atoms.push(base);
            } else {
                let (sub, sup) = if is_sup {
                    (None, Some(vec![atom]))
                } else {
                    (Some(vec![atom]), None)
                };
                level.atoms.push(MathAtom::Scripts {
                    base: Box::new(base),
                    sub,
                    sup,
                });
            }
        } else {
            level.atoms.push(atom);
        }
        Ok(())
    }

    /// 收尾本层待定分式：numerator 已存，当前层 atoms 作为 denominator 打包
    /// （TeX fin_mlist）。
    fn math_finish_fraction(level: &mut MathLevel) {
        if let Some(fp) = level.fraction.take() {
            let den = std::mem::take(&mut level.atoms);
            level.atoms.push(MathAtom::Fraction {
                num: fp.num,
                den,
                thickness: fp.thickness,
            });
        }
    }

    /// 脚本字段组结束（`x^{...}`/`x_{...}`）：字段挂到外层末尾原子。
    /// 无原子可挂（`x^{}` 前空）→ TeX "Missing { inserted" 语义：插入空原子恢复。
    fn math_attach_script(
        parent: &mut MathLevel,
        is_sup: bool,
        field: Vec<MathAtom>,
    ) -> Result<()> {
        let mut base = parent.atoms.pop().unwrap_or(MathAtom::Classed {
            class: MathClass::Ord,
            content: Vec::new(),
        });
        if let MathAtom::Scripts { sub, sup, .. } = &mut base {
            if is_sup {
                if sup.is_some() {
                    return Err(Error::invalid_input("双重上标（Double superscript）"));
                }
                *sup = Some(field);
            } else {
                if sub.is_some() {
                    return Err(Error::invalid_input("双重下标（Double subscript）"));
                }
                *sub = Some(field);
            }
            parent.atoms.push(base);
        } else {
            let (sub, sup) = if is_sup {
                (None, Some(field))
            } else {
                (Some(field), None)
            };
            parent.atoms.push(MathAtom::Scripts {
                base: Box::new(base),
                sub,
                sup,
            });
        }
        Ok(())
    }

    /// 原子类别（spacing 表用；脚本原子取 base 的类）。
    fn math_class(atom: &MathAtom) -> Option<MathClass> {
        match atom {
            MathAtom::Char(mc) => Some(mc.class),
            MathAtom::Scripts { base, .. } => Self::math_class(base),
            MathAtom::Classed { class, .. } => Some(*class),
            MathAtom::Fraction { .. } => Some(MathClass::Inner),
            MathAtom::Delimited { .. } => Some(MathClass::Inner),
            MathAtom::Middle(_) => Some(MathClass::Inner),
            MathAtom::Radical { .. } => Some(MathClass::Ord),
            MathAtom::Underline { .. } | MathAtom::Overline { .. } => Some(MathClass::Ord),
            MathAtom::Box(_) => Some(MathClass::Ord),
            MathAtom::Accent { .. } => Some(MathClass::Ord),
            MathAtom::MSkip { .. }
            | MathAtom::Style(_)
            | MathAtom::Penalty { .. }
            | MathAtom::Rule { .. } => None,
        }
    }

    /// 数学列表 → 水平节点（M4-1：spacing 胶水 + 字符 + 上下标盒）。
    fn math_to_hlist(&self, atoms: &[MathAtom], style: MathStyle) -> Vec<Node> {
        let mut out = Vec::new();
        let mut style = style;
        let mut prev: Option<MathClass> = None;
        for atom in atoms {
            // 样式切换原子就地生效（影响后续原子字阶与 spacing）
            if let MathAtom::Style(s) = atom {
                style = *s;
                continue;
            }
            let cur = Self::math_class(atom);
            if let (Some(p), Some(c)) = (prev, cur) {
                match spacing_code(p, c, style) {
                    SpacingCode::None | SpacingCode::Tight => {}
                    code => {
                        // ETRIP P0 \muexpr 校准：muskip_params[i] 字段以 mu 数值存
                        // （默认 thin=3mu 在 cmr10 → 实际 sp ≈ 1.6667pt；etrip 在
                        //  smalltrip=5pt → 18mu 实际 sp = 5pt，对齐 etrip.log）。
                        // math_to_hlist 按当前 style 的 family-2 em/18 缩放到 sp。
                        let idx = code_idx(code);
                        let g = self.muskip_params[idx];
                        let (w, st, sh) = if self.muskip_is_mu[idx] {
                            let em = self.math_em(style);
                            (
                                mu_to_sp(g.width, em),
                                mu_to_sp(g.stretch, em),
                                mu_to_sp(g.shrink, em),
                            )
                        } else {
                            (g.width, g.stretch, g.shrink)
                        };
                        out.push(Node::Glue {
                            // showbox 显示 \glue(\thinmuskip) 等（tex.web 来源名）
                            name: Some(match code {
                                SpacingCode::Thin => "thinmuskip",
                                SpacingCode::Med => "medmuskip",
                                SpacingCode::Thick => "thickmuskip",
                                _ => "muskip",
                            }),
                            width: w,
                            stretch: st,
                            shrink: sh,
                            stretch_order: 0,
                            shrink_order: 0,
                        });
                    }
                }
            }
            if cur.is_some() {
                prev = cur;
            }
            out.extend(self.math_atom_nodes(atom, style));
        }
        out
    }

    /// 原子 → 节点（M4-1/2：Char/Scripts/Fraction/Radical/Delimited/Classed/MSkip/Box）。
    fn math_atom_nodes(&self, atom: &MathAtom, style: MathStyle) -> Vec<Node> {
        match atom {
            MathAtom::Char(mc) => {
                // Op 大算符：vcenter（+display 变体放大），tex.web make_op
                // 对所有 Op 字符原子生效（demo1 差异 #3：display Σ 未放大居中）
                if mc.class == MathClass::Op {
                    return self.op_nodes(mc, style, None, None);
                }
                let (font, num, den) = self.math_char_font(mc, style);
                let (w, h, d) = self.math_metrics(font, mc.charcode, num, den);
                vec![Node::Char {
                    font,
                    charcode: mc.charcode,
                    width: w,
                    height: h,
                    depth: d,
                }]
            }
            MathAtom::Scripts { base, sub, sup } => {
                // display 大算符带上下标：limits 堆叠（tex.web make_op subtype
                // =limits 默认于 display；`\sum_{n=1}^{\infty}` 上下限居中于 Σ）
                if let MathAtom::Char(mc) = base.as_ref() {
                    if mc.class == MathClass::Op && style == MathStyle::Display {
                        return self.op_nodes(mc, style, sub.as_deref(), sup.as_deref());
                    }
                }
                let mut out = self.math_atom_nodes(base, style);
                let s_style = style.next();
                // 上标：内容打包为 hbox，shift 上移（hlist 内 Box.shift 为垂直位移）。
                // tex.web make_scripts 对 sub/sup 盒都执行 width(x)+=script_space
                // （盒宽加大但字形不移动）；分式规则宽 = max(分子,分母盒宽)，
                // demo1 对照：官方 `{1\over n^2}` 规则宽 687373 vs 修前 654605，
                // 差 32768 = \scriptspace(0.5pt)。
                // 垂直位移按 make_scripts 公式（见 script_shifts）；
                // sub+sup 同时出现的清空/合并盒（4×rule_thickness 判定 + vpack）
                // 本引擎仍用两个独立盒表达，未覆盖。
                let (mut shift_up, mut shift_down) = self.script_base_shifts(&out, style);
                if let Some(sup_atoms) = sup {
                    let nodes = self.math_to_hlist(sup_atoms, s_style);
                    let mut b = BoxNode::new_hbox(nodes);
                    b.width += self.params.scriptspace;
                    let kind = style.size_kind();
                    // clr：display→sup1（mathsy 13），其余（本引擎无 cramped 样式）
                    // →sup2（mathsy 14）；再与 depth(sup)+x_height/4 取大
                    let clr = if style == MathStyle::Display {
                        self.mathsy_param(kind, 13)
                    } else {
                        self.mathsy_param(kind, 14)
                    };
                    if shift_up < clr {
                        shift_up = clr;
                    }
                    let clr2 = b.depth + self.mathsy_x_height(kind) / 4;
                    if shift_up < clr2 {
                        shift_up = clr2;
                    }
                    b.shift = -shift_up;
                    out.push(Node::Box(b));
                }
                if let Some(sub_atoms) = sub {
                    let nodes = self.math_to_hlist(sub_atoms, s_style);
                    let mut b = BoxNode::new_hbox(nodes);
                    b.width += self.params.scriptspace;
                    let kind = style.size_kind();
                    // 无上标时下限 sub1（mathsy 16），且不低于 height(sub)-4/5·x_height
                    let sub1 = self.mathsy_param(kind, 16);
                    if shift_down < sub1 {
                        shift_down = sub1;
                    }
                    let clr = b.height - (self.mathsy_x_height(kind) * 4) / 5;
                    if shift_down < clr {
                        shift_down = clr;
                    }
                    b.shift = shift_down;
                    out.push(Node::Box(b));
                }
                out
            }
            MathAtom::Fraction {
                num,
                den,
                thickness,
            } => self.fraction_nodes(num, den, *thickness, style),
            MathAtom::Radical { base } => self.radical_nodes(base, style),
            // \underline/\overline：M4-2 简化——内容直接输出（底线/顶线渲染 M4-3）
            MathAtom::Underline { base } | MathAtom::Overline { base } => {
                self.math_to_hlist(base, style)
            }
            MathAtom::Delimited { left, body, right } => {
                let mut inner = Vec::new();
                if let Some(d) = left {
                    inner.extend(self.delim_nodes(*d, style));
                }
                inner.extend(self.math_to_hlist(body, style));
                if let Some(d) = right {
                    inner.extend(self.delim_nodes(*d, style));
                }
                // \\left...\\right 物化为单个 hbox（tex.web：定界符与内容同盒，
                // 盒高由定界符撑起——参考 etrip `\\hbox(17.0+3.00002)x23.9999`
                // 含 [ 定界符盒 + 内容 + ] 定界符）。自然宽打包（excess=0 →
                // 盒内 \\thinmuskip 等 glue 不烘焙，showbox 保留原值）。
                let w = hbox_dimensions(&inner).width;
                vec![Node::Box(hpack(&inner, w))]
            }
            // e-TeX \middle：定界符原子（类 Inner，同 \left/\right 的分隔符排版）
            MathAtom::Middle(d) => {
                if let Some(d) = d {
                    self.delim_nodes(*d, style)
                } else {
                    Vec::new()
                }
            }
            MathAtom::Classed { content, .. } => self.math_to_hlist(content, style),
            MathAtom::Accent { accent, nucleus } => self
                .math_to_hlist(accent, style)
                .into_iter()
                .chain(self.math_to_hlist(nucleus, style))
                .collect(),
            MathAtom::MSkip {
                width,
                stretch,
                shrink,
                nonscript,
            } => {
                if *nonscript
                    && matches!(style, MathStyle::Script | MathStyle::ScriptScript)
                {
                    Vec::new()
                } else {
                    vec![Node::Glue {
            name: None,                        width: *width,
                        stretch: *stretch,
                        shrink: *shrink,
                        stretch_order: 0,
                        shrink_order: 0,
                    }]
                }
            }
            MathAtom::Box(b) => vec![Node::Box(b.clone())],
            MathAtom::Penalty { penalty } => vec![Node::Penalty { penalty: *penalty }],
            MathAtom::Rule { width, height, depth } => vec![Node::Rule {
                width: *width,
                height: *height,
                depth: *depth,
            }],
            MathAtom::Style(_) => unreachable!("Style 原子在 math_to_hlist 循环中处理"),
        }
    }

    /// 分式 → 节点（tex.web §1184 `make_fraction` 精确垂直几何；2026-09-06
    /// demo1 差异 #3 收尾：此前分式盒挂公式基线下，现按 num1/denom1/axis 重排
    /// ——参考点构造使分式线落在基线上方 axis 处；num/den rebox 等宽居中）。
    /// `\atop`（thickness=0）与 `\over`（None=默认厚度 mathex(8)）均覆盖；
    /// 外壳 null delimiter（两侧各 \nulldelimiterspace=1.2pt）暂略。
    fn fraction_nodes(
        &self,
        num: &[MathAtom],
        den: &[MathAtom],
        thickness: Option<i64>,
        style: MathStyle,
    ) -> Vec<Node> {
        // 分子/分母字阶：display→text、其余降一级（tex.web num_style/denom_style）
        let sub_style = match style {
            MathStyle::Display => MathStyle::Text,
            s => s.next(),
        };
        let mut num_b = BoxNode::new_hbox(self.math_to_hlist(num, sub_style));
        let mut den_b = BoxNode::new_hbox(self.math_to_hlist(den, sub_style));
        // num1/num2/num3/denom1/denom2 = mathsy(8..12)（tex.web @d L13817-13821）
        let kind = style.size_kind();
        let fam2 = self
            .math_fonts
            .get(2)
            .and_then(|s| s.get(kind))
            .copied()
            .flatten();
        let fp = |idx: usize| match fam2 {
            Some(f) => self.fonts.font_param(f, idx),
            None => 0,
        };
        // 默认分式线厚度 = mathex(8)（tex.web @d L13841，非 fam2！）
        let fam3 = self
            .math_fonts
            .get(3)
            .and_then(|s| s.get(kind))
            .copied()
            .flatten();
        let default_t = match fam3 {
            Some(f) => self.fonts.font_param(f, 8),
            None => 2 * SP_PER_PT / 5,
        };
        let t = thickness.unwrap_or(default_t);
        let axis = self.axis_height(style);
        let display = style == MathStyle::Display;
        // 初始 shift_up/shift_down（display 用 num1/denom1；否则 num2/num3/denom2）
        let (mut shift_up, mut shift_down) = if display {
            (fp(8), fp(11))
        } else {
            (if t != 0 { fp(9) } else { fp(10) }, fp(12))
        };
        let width = num_b.width.max(den_b.width);
        Self::rebox_centered(&mut num_b, width);
        Self::rebox_centered(&mut den_b, width);
        let (num_h, num_d, den_h, den_d) = (num_b.height, num_b.depth, den_b.height, den_b.depth);
        let mut children: Vec<Node> = Vec::new();
        if t > 0 {
            // 有分式线：clr = display ? 3t : t；间隙不足则同时外推
            let clr = if display { 3 * t } else { t };
            let delta = t / 2;
            let delta1 = clr - ((shift_up - num_d) - (axis + delta));
            let delta2 = clr - ((axis - delta) - (den_h - shift_down));
            if delta1 > 0 {
                shift_up += delta1;
            }
            if delta2 > 0 {
                shift_down += delta2;
            }
            children.push(Node::Box(num_b));
            children.push(Node::Kern {
                width: (shift_up - num_d) - (axis + delta),
            });
            children.push(Node::Rule {
                width,
                height: t,
                depth: 0,
            });
            children.push(Node::Kern {
                width: (axis - delta) - (den_h - shift_down),
            });
            children.push(Node::Box(den_b));
        } else {
            // \atop：clr 按默认厚度计（tex.web：7×/3× default_rule_thickness）
            let clr = if display {
                7 * default_t
            } else {
                3 * default_t
            };
            let delta = (clr - ((shift_up - num_d) - (den_h - shift_down))) / 2;
            if delta > 0 {
                shift_up += delta;
                shift_down += delta;
            }
            children.push(Node::Box(num_b));
            children.push(Node::Kern {
                width: (shift_up - num_d) - (den_h - shift_down),
            });
            children.push(Node::Box(den_b));
        }
        let mut v = BoxNode::new_vbox(children);
        v.width = width;
        // height(v)=shift_up+h(num)、depth(v)=d(den)+shift_down（tex.web）→
        // 分式线中心恰好落在参考点上方 axis 处（vcenter 于数学轴）
        v.height = shift_up + num_h;
        v.depth = den_d + shift_down;
        vec![Node::Box(v)]
    }

    /// 数学轴高度（tex.web `mathsy(22)`：fam 2 当前字阶字体 fontdimen 22；
    /// 字体未加载回退 0）。
    fn axis_height(&self, style: MathStyle) -> i64 {
        let kind = style.size_kind();
        let font = self
            .math_fonts
            .get(2)
            .and_then(|s| s.get(kind))
            .copied()
            .flatten();
        match font {
            Some(f) => self.fonts.font_param(f, 22),
            None => 0,
        }
    }

    /// `big_op_spacing1..5`（tex.web `mathex(9..13)`：fam 3 字体 fontdimen
    /// 9–13，大算符上下限最小间隙与盒顶/底 clearance）。
    fn big_op_spacings(&self, style: MathStyle) -> [i64; 5] {
        let kind = style.size_kind();
        let font = self
            .math_fonts
            .get(3)
            .and_then(|s| s.get(kind))
            .copied()
            .flatten();
        match font {
            Some(f) => [
                self.fonts.font_param(f, 9),
                self.fonts.font_param(f, 10),
                self.fonts.font_param(f, 11),
                self.fonts.font_param(f, 12),
                self.fonts.font_param(f, 13),
            ],
            None => [0; 5],
        }
    }

    /// hbox 撑宽居中（tex.web `rebox`）：两侧对称 kern（引擎 DVI 端无 glue
    /// set，故用 kern 而非 TeX 的 fil glue，坐标等价）。0 宽侧不插 kern。
    fn rebox_centered(b: &mut BoxNode, width: i64) {
        if width > b.width {
            let left = (width - b.width) / 2;
            if left > 0 {
                b.children.insert(0, Node::Kern { width: left });
            }
            let right = width - b.width - left;
            if right > 0 {
                b.children.push(Node::Kern { width: right });
            }
            b.width = width;
        }
    }

    /// Op 大算符布局（tex.web §1185 make_op）：
    /// - display 时沿 next_larger 放大一步（`\sum="1350` → cmex10 80→88）；
    /// - 字符盒 vcenter：`shift = half(h−d) − axis_height`（hlist 中正=下移）；
    /// - display 带 sup/sub 时上下限堆叠（limits；真实 TeX showbox 验证：
    ///   `[kern(bos5), sup, kern(shift_up), op, kern(shift_down), sub, kern(bos5)]`，
    ///   `shift_up = max(bos3 − d(sup), bos1)`，`shift_down = max(bos4 − h(sub), bos2)`，
    ///   vbox 参考点 = op 行盒基线，height/depth 手工按段累加）。
    fn op_nodes(
        &self,
        mc: &MathChar,
        style: MathStyle,
        sub: Option<&[MathAtom]>,
        sup: Option<&[MathAtom]>,
    ) -> Vec<Node> {
        let (font, num, den) = self.math_char_font(mc, style);
        let mut ch = mc.charcode;
        // display 变体放大（tex.web：cur_style < text_style 且 char_tag=list_tag）
        if style == MathStyle::Display {
            if let Some(larger) = self.fonts.next_larger(font, ch) {
                if self.fonts.char_exists(font, larger) {
                    ch = larger;
                }
            }
        }
        let (w, h, d) = self.math_metrics(font, ch, num, den);
        let axis = self.axis_height(style);
        // vcenter（cmex10 大算符基线在设计上偏离中心，如 'X' h=1.0/d=15.0 → shift≈−9.5pt）
        let shift = (h - d) / 2 - axis;
        let mut sigma = BoxNode::new_hbox(vec![Node::Char {
            font,
            charcode: ch,
            width: w,
            height: h,
            depth: d,
        }]);
        sigma.shift = shift;
        if sub.is_none() && sup.is_none() {
            return vec![Node::Box(sigma)];
        }
        let [bos1, bos2, bos3, bos4, bos5] = self.big_op_spacings(style);
        // op 行盒：Σ 盒 shift 后的实际占位（引擎 hbox_dimensions 不计子盒 shift，手工设）
        let op_h = h - shift;
        let op_d = d + shift;
        let s_style = style.next();
        let mut sup_b = sup
            .filter(|a| !a.is_empty())
            .map(|a| BoxNode::new_hbox(self.math_to_hlist(a, s_style)));
        let mut sub_b = sub
            .filter(|a| !a.is_empty())
            .map(|a| BoxNode::new_hbox(self.math_to_hlist(a, s_style)));
        let width = [Some(w), sup_b.as_ref().map(|b| b.width), sub_b.as_ref().map(|b| b.width)]
            .into_iter()
            .flatten()
            .max()
            .unwrap_or(0);
        // op 行盒（Σ 盒居中撑宽，h/d 按 shift 后实际占位）
        let left = (width - w) / 2;
        let mut op_row = BoxNode::new_hbox(vec![
            Node::Kern { width: left },
            Node::Box(sigma),
            Node::Kern {
                width: width - w - left,
            },
        ]);
        op_row.width = width;
        op_row.height = op_h;
        op_row.depth = op_d;
        // limits vbox（参考点 = op 行盒基线）
        let mut children: Vec<Node> = Vec::new();
        let mut v_height = op_h;
        let mut v_depth = op_d;
        if let Some(x) = sup_b.as_mut() {
            Self::rebox_centered(x, width);
            let shift_up = (bos3 - x.depth).max(bos1);
            children.push(Node::Kern { width: bos5 });
            children.push(Node::Box(x.clone()));
            children.push(Node::Kern { width: shift_up });
            v_height += bos5 + x.height + x.depth + shift_up;
        }
        children.push(Node::Box(op_row));
        if let Some(z) = sub_b.as_mut() {
            Self::rebox_centered(z, width);
            let shift_down = (bos4 - z.height).max(bos2);
            children.push(Node::Kern { width: shift_down });
            children.push(Node::Box(z.clone()));
            children.push(Node::Kern { width: bos5 });
            v_depth += shift_down + z.height + z.depth + bos5;
        }
        let mut v = BoxNode::new_vbox(children);
        v.width = width;
        v.height = v_height;
        v.depth = v_depth;
        vec![Node::Box(v)]
    }

    /// 根式 → 节点（M4-2 简化：内容上方画分式线式横线；M4-3 换 cmex10 根号）。
    fn radical_nodes(&self, base: &[MathAtom], style: MathStyle) -> Vec<Node> {
        let base_b = BoxNode::new_hbox(self.math_to_hlist(base, style));
        let base_height = base_b.height;
        let t = 2 * SP_PER_PT / 5; // 0.4pt
        let gap = SP_PER_PT; // 内容上缘到线的间隙
        let children = vec![
            Node::Rule {
                width: base_b.width,
                height: t,
                depth: 0,
            },
            Node::Glue {
            name: None,                width: 0,
                stretch: 0,
                shrink: 0,
                stretch_order: 0,
                shrink_order: 0,
            },
            Node::Box(base_b),
        ];
        let mut b = BoxNode::new_vbox(children);
        // 参考点 = 内容基线：线在基线上方 base 高 + gap + t 处（shift 为负 = 上移）
        b.shift = -(base_height + gap + t);
        vec![Node::Box(b)]
    }

    /// 定界符字符节点（当前字体 + 字阶缩放；M4-3 换 cmex10 变体伸缩）。
    fn delim_nodes(&self, d: u32, style: MathStyle) -> Vec<Node> {
        let (num, den) = style.scale();
        let (w, h, dd) = self.math_metrics(self.current_font, d, num, den);
        vec![Node::Char {
            font: self.current_font,
            charcode: d,
            width: w,
            height: h,
            depth: dd,
        }]
    }

    /// 按字阶缩放字符度量（M4-1 比例近似；M4-3 换真实 scriptfont）。
    fn math_metrics(&self, font: FontId, ch: u32, num: i64, den: i64) -> (i64, i64, i64) {
        let (w, h, d) = self.fonts.metrics(font, ch);
        if num == den {
            return (w, h, d);
        }
        (
            xn_over_d(w, num, den),
            xn_over_d(h, num, den),
            xn_over_d(d, num, den),
        )
    }

    /// 数学字符的字体（M4-3）：族+字阶查 `\textfont` 表；未分配回退当前字体+比例缩放。
    fn math_char_font(&self, mc: &MathChar, style: MathStyle) -> (FontId, i64, i64) {
        let kind = match style {
            MathStyle::Display | MathStyle::Text => 0,
            MathStyle::Script => 1,
            _ => 2,
        };
        if let Some(Some(f)) = self.math_fonts.get(mc.fam as usize).map(|s| s[kind]) {
            (f, 1, 1) // 族字体已按字阶设计字号，不缩放
        } else {
            let (num, den) = style.scale();
            (self.current_font, num, den)
        }
    }

    /// 上标提升量（M4-3）：fontdimen sup1（参数 11，无上标时 sup2/3）；回退 x_height×字阶。
    /// family-2（math symbols）字体在指定字阶槽的 fontdimen（tex.web
    /// `mathsy(n)`；缺字体/缺参数回 0，与 TeX nullfont 参数为 0 同义）。
    fn mathsy_param(&self, kind: usize, idx: usize) -> i64 {
        self.math_fonts
            .get(2)
            .and_then(|s| s.get(kind).copied().flatten())
            .map(|f| self.fonts.font_param(f, idx))
            .unwrap_or(0)
    }

    /// family-2 字体的 x_height（tex.web `math_x_height(cur_size)`）。
    fn mathsy_x_height(&self, kind: usize) -> i64 {
        self.math_fonts
            .get(2)
            .and_then(|s| s.get(kind).copied().flatten())
            .map(|f| self.fonts.x_height(f))
            .unwrap_or(0)
    }

    /// tex.web §14884 make_scripts 开头的 nucleus 基准位移：
    /// nucleus 是单字符节点 → (0,0)；否则 hpack(natural) 后
    /// `shift_up = height - sup_drop(t)`、`shift_down = depth + sub_drop(t)`，
    /// 其中 t = 脚本字阶（cur_style<script_style → script_size，否则
    /// script_script_size；对应 fontdimen 18/19）。
    fn script_base_shifts(&self, base_nodes: &[Node], style: MathStyle) -> (i64, i64) {
        if base_nodes.len() == 1 && matches!(base_nodes[0], Node::Char { .. }) {
            return (0, 0);
        }
        let w = hbox_dimensions(base_nodes).width;
        let packed = hpack(base_nodes, w);
        // sup_drop/sub_drop 的字阶：display/text 用 script 槽，script 系用
        // scriptscript 槽（tex.web `t:=script_size/script_script_size`）
        let t_kind = match style {
            MathStyle::Display | MathStyle::Text => 1,
            MathStyle::Script | MathStyle::ScriptScript => 2,
        };
        let sup_drop = self.mathsy_param(t_kind, 18);
        let sub_drop = self.mathsy_param(t_kind, 19);
        (packed.height - sup_drop, packed.depth + sub_drop)
    }

    /// ETRIP P0 \muexpr 校准：取当前 style 的 1em（sp）—— tex.web §685 ÷18 即 1mu。
    /// 顺序取 family 2 该 style 的字体 fontdimen 6（quad，TeXbook 附录 G）。
    /// 回退链：family 2 字体 → current_font fontdimen 6 → 10pt（em=10pt 默认值，
    /// 对照 plain TeX 隐含 cmr10 设计字号，与 etrip 期望 cmr10 × 小字体族差距可接受）。
    fn math_em(&self, style: MathStyle) -> i64 {
        let kind = match style {
            MathStyle::Display | MathStyle::Text => 0,
            MathStyle::Script => 1,
            _ => 2,
        };
        let quad = if let Some(f) = self
            .math_fonts
            .get(2)
            .and_then(|s| s.get(kind).copied().flatten())
        {
            self.fonts.font_param(f, 6)
        } else {
            self.fonts.font_param(self.current_font, 6)
        };
        if quad != 0 {
            quad
        } else {
            // 无任何 fontdimen 6（如 fn 指针占位 + 当前字体未加载 TFM）：
            // 退回 plain TeX 默认 10pt em，对齐真实 TeX 在 plain plain 下的隐含值。
            10 * SP_PER_PT
        }
    }
}

/// ETRIP P0 \muexpr 校准：mu 数值 → sp（tex.web §685：1mu = em/18）。
/// `mu_emu` 即 NTex 现有约定下 Glie 字段以 N×65536 存的"伪 mu"数值（N=1 ≈ 1pt）。
/// 若 mu=0 直接返回 0（避免 fallback em 0 时空转）。
///
/// 算术对齐 tex.web `math_glue`/`mu_mult`（§14096-14114）：先取
/// `cur_mu = em/18`（**整数除法截断**），再 `mu_mult(x) = n*x + xn_over_d(x, f, 65536)`
/// （n = cur_mu div 65536、f = cur_mu mod 65536）。截断不可省：demo1 对照
/// TinyTeX 实测 `\thickmuskip=5mu`（em=cmex10 quad=10pt）官方 DVI `right182040`
/// = 5 × 36408；精确除法会得 182044（差 4 sp，DVI 逐指令对比可见）。
fn mu_to_sp(mu_emu: i64, em_sp: i64) -> i64 {
    if mu_emu == 0 || em_sp == 0 {
        return mu_emu;
    }
    let cur_mu = em_sp / 18;
    let n = cur_mu / SP_PER_PT;
    let f = cur_mu - n * SP_PER_PT;
    n.saturating_mul(mu_emu) + xn_over_d(mu_emu, f, SP_PER_PT)
}

/// 数学间距（TeXbook 附录 G 规则 18；text/script 模式；display 对 op 修正）。
/// 行 = 左原子类、列 = 右原子类；0 无 / 1 thin / 2 med / 3 thick / 4 *（紧排）。
fn spacing_code(l: MathClass, r: MathClass, style: MathStyle) -> SpacingCode {
    const TABLE: [[u8; 8]; 8] = [
        //          ord op  bin rel open close punct inner
        /*ord*/    [0, 1, 2, 3, 0, 0, 0, 1],
        /*op*/     [1, 1, 4, 3, 0, 0, 0, 1],
        /*bin*/    [2, 2, 4, 4, 2, 2, 2, 2],
        /*rel*/    [3, 3, 4, 0, 3, 3, 3, 3],
        /*open*/   [0, 0, 4, 0, 0, 0, 0, 0],
        /*close*/  [0, 1, 2, 3, 0, 0, 0, 1],
        /*punct*/  [1, 1, 4, 1, 1, 1, 1, 1],
        /*inner*/  [1, 1, 2, 3, 1, 0, 1, 1],
    ];
    let idx = |c: MathClass| match c {
        MathClass::Ord | MathClass::Var => 0,
        MathClass::Op => 1,
        MathClass::Bin => 2,
        MathClass::Rel => 3,
        MathClass::Open => 4,
        MathClass::Close => 5,
        MathClass::Punct => 6,
        MathClass::Inner => 7,
    };
    let raw = TABLE[idx(l)][idx(r)];
    // display 模式（TeXbook p.170）：op 前后的 thin(1) 升为 thick(3)。
    if style == MathStyle::Display
        && raw == 1
        && (l == MathClass::Op || r == MathClass::Op)
    {
        return SpacingCode::Thick;
    }
    match raw {
        0 => SpacingCode::None,
        1 => SpacingCode::Thin,
        2 => SpacingCode::Med,
        3 => SpacingCode::Thick,
        _ => SpacingCode::Tight,
    }
}

/// SpacingCode → muskip 参数表索引（0=thin 1=med 2=thick）。
fn code_idx(code: SpacingCode) -> usize {
    match code {
        SpacingCode::Thin => 0,
        SpacingCode::Med => 1,
        SpacingCode::Thick => 2,
        _ => 0,
    }
}


/// `xn_over_d`（tex.web）：t×n/d 四舍五入（负值按远离零）。
fn xn_over_d(t: i64, n: i64, d: i64) -> i64 {
    if t >= 0 {
        (t * n + d / 2) / d
    } else {
        -(((-t) * n + d / 2) / d)
    }
}
