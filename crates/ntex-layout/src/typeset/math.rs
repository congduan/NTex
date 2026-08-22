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
            width: above.width,
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
        // 公式末尾收尾：未闭合 \left 报错；待定分式收尾（TeX 允许空分母）
        if level.left.is_some() {
            return Err(Error::invalid_input("\\left 后缺少 \\right（Extra } or forgotten \\right）"));
        }
        Self::math_finish_fraction(&mut level);
        let nodes = self.math_to_hlist(&level.atoms, style);
        if was_display {
            // 公式盒 = `\hbox to \hsize`（两侧 \hfil 居中；displaywidth≈\hsize）
            let mut line: Vec<Node> = Vec::with_capacity(nodes.len() + 2);
            line.push(Node::Glue {
                width: 0,
                stretch: 1,
                shrink: 0,
                stretch_order: GLUE_ORDER_FIL,
                shrink_order: 0,
            });
            line.extend(nodes);
            line.push(Node::Glue {
                width: 0,
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
                width: below.width,
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
        // `\mathbin` 等单原子字段：`\mathbin+`（Char 改类，其余包 Classed）
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
        let level = self
            .math
            .last_mut()
            .ok_or_else(|| Error::internal("数学原子无数学层"))?;
        if let Some(is_sup) = self.pending_script.take() {
            let mut base = level.atoms.pop().ok_or_else(|| {
                Error::invalid_input("数学模式中 ^/_ 前缺少原子（Missing { inserted）")
            })?;
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
    fn math_attach_script(
        parent: &mut MathLevel,
        is_sup: bool,
        field: Vec<MathAtom>,
    ) -> Result<()> {
        let mut base = parent.atoms.pop().ok_or_else(|| {
            Error::invalid_input("数学模式中 ^/_ 前缺少原子（Missing { inserted）")
        })?;
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
            MathAtom::Box(_) => Some(MathClass::Ord),
            MathAtom::MSkip { .. } | MathAtom::Style(_) => None,
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
                        let quad = self.fonts.quad(self.current_font);
                        let (w, st, sh) = muskip(code, quad);
                        out.push(Node::Glue {
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
                let mut out = self.math_atom_nodes(base, style);
                let s_style = style.next();
                // 上标：内容打包为 hbox，shift 上移（hlist 内 Box.shift 为垂直位移）
                if let Some(sup_atoms) = sup {
                    let nodes = self.math_to_hlist(sup_atoms, s_style);
                    let mut b = BoxNode::new_hbox(nodes);
                    b.shift = -self.script_rise(style);
                    out.push(Node::Box(b));
                }
                if let Some(sub_atoms) = sub {
                    let nodes = self.math_to_hlist(sub_atoms, s_style);
                    let mut b = BoxNode::new_hbox(nodes);
                    b.shift = self.script_drop();
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
            MathAtom::Delimited { left, body, right } => {
                let mut out = Vec::new();
                if let Some(d) = left {
                    out.extend(self.delim_nodes(*d, style));
                }
                out.extend(self.math_to_hlist(body, style));
                if let Some(d) = right {
                    out.extend(self.delim_nodes(*d, style));
                }
                out
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
                        width: *width,
                        stretch: *stretch,
                        shrink: *shrink,
                        stretch_order: 0,
                        shrink_order: 0,
                    }]
                }
            }
            MathAtom::Box(b) => vec![Node::Box(b.clone())],
            MathAtom::Style(_) => unreachable!("Style 原子在 math_to_hlist 循环中处理"),
        }
    }

    /// 分式 → 节点（M4-2 简化：分子/分式线/分母垂直堆叠；M4-3 用 fontdimen 精化）。
    fn fraction_nodes(
        &self,
        num: &[MathAtom],
        den: &[MathAtom],
        thickness: Option<i64>,
        style: MathStyle,
    ) -> Vec<Node> {
        let num_b = BoxNode::new_hbox(self.math_to_hlist(num, style));
        let den_b = BoxNode::new_hbox(self.math_to_hlist(den, style));
        let width = num_b.width.max(den_b.width);
        let num_height = num_b.height;
        let t = thickness.unwrap_or(SP_PER_PT * 2 / 5); // 默认分式线 0.4pt
        let gap = 2 * SP_PER_PT; // 分子/分母与线的间隙（M4-3 用 fontdimen num1 等）
        let mut children = Vec::new();
        children.push(Node::Box(num_b));
        // 垂直间隙（vbox 内 x 不推进；宽度 0 避免抬高 vbox 总宽）
        children.push(Node::Glue {
            width: 0,
            stretch: 0,
            shrink: 0,
            stretch_order: 0,
            shrink_order: 0,
        });
        if t > 0 {
            children.push(Node::Rule {
                width,
                height: t,
                depth: 0,
            });
        }
        children.push(Node::Glue {
            width: 0,
            stretch: 0,
            shrink: 0,
            stretch_order: 0,
            shrink_order: 0,
        });
        children.push(Node::Box(den_b));
        let mut b = BoxNode::new_vbox(children);
        // 参考点 = 分式线：顶部（num 顶）到线 = num 高 + gap + t/2（hlist 内 shift 为垂直位移）
        b.shift = num_height + gap + t / 2;
        vec![Node::Box(b)]
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
                width: 0,
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
    fn script_rise(&self, style: MathStyle) -> i64 {
        let sup1 = self.fonts.font_param(self.current_font, 11);
        if sup1 != 0 {
            return sup1;
        }
        let xh = self.fonts.x_height(self.current_font);
        let (num, den) = style.scale();
        xn_over_d(xh, num, den)
    }

    /// 下标下降量（M4-1 近似：0.5pt；M4-3 用 fontdimen sub_drop）。
    fn script_drop(&self) -> i64 {
        0
    }
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
        MathClass::Ord => 0,
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

/// muskip 宽度（1mu = quad/18）：thin=3mu、med=4mu±2mu∓4mu、thick=5mu±5mu。
fn muskip(code: SpacingCode, quad: i64) -> (i64, i64, i64) {
    let mu = quad / 18;
    match code {
        SpacingCode::Thin => (3 * mu, 0, 0),
        SpacingCode::Med => (4 * mu, 2 * mu, 4 * mu),
        SpacingCode::Thick => (5 * mu, 5 * mu, 0),
        _ => (0, 0, 0),
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

