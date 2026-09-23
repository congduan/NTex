/// 字符斜体修正（tex.web `char_italic(f)(q)`；fn 指针占位一律 0）。
/// impl 分片落在本文件：mod.rs 正由 box/insert 侧刀线并行修改，领地隔离。
impl Fonts {
    fn char_italic(&self, font: FontId, charcode: u32) -> i64 {
        match self {
            Fonts::Fn { .. } => 0,
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .map(|fm| fm.char_italic(charcode))
                .unwrap_or(0),
        }
    }
}

/// tex.web `half(x)`：奇数取 (x+1) div 2（Pascal div 向零截断），偶数取 x div 2。
fn half(x: i64) -> i64 {
    if x % 2 != 0 {
        (x + 1) / 2
    } else {
        x / 2
    }
}

impl NodeBuilder {
    // ---------- M4-1 数学模式 ----------
    /// tex.web `new_param_glue`：按参数 glue 追加节点（display 上下间距用）。
    fn append_param_glue(&mut self, g: ntex_core::Glue) {
        self.append(Node::Glue {
            name: None,
            width: g.width,
            stretch: g.stretch,
            shrink: g.shrink,
            stretch_order: g.stretch_order,
            shrink_order: g.shrink_order,
        });
    }

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
        self.math_state.math.push(MathLevel::default());
        self.math_state.math_style = style;
        self.lists.push(Vec::new());
        self.list_modes.push(mode);
        Ok(())
    }

    /// M4-4 进入显示数学：只切模式（公式原子收集，退出时经 [`Self::close_math`]
    /// 按 tex.web finish_display 的顺序落垂直列表）。
    /// tex.web 的 `\predisplaypenalty` + 上间距**不在进入时**追加——那是
    /// finish_display（退出）里的 `@<Append the glue or equation number preceding
    /// the display@>`：长短 skip 的裁决要用公式自然宽（退出时才可知）。
    fn enter_display_math(&mut self) -> Result<()> {
        self.enter_math(Mode::DisplayMath)
    }

    /// 退出数学模式：数学列表转节点追加到上层列表。
    /// 显示公式（M4-4）：收为 `\hbox to \hsize` 居中盒 + `\belowdisplayskip` +
    /// `\postdisplaypenalty`（垂直元素）；行内公式：节点直通当前列表。
    fn close_math(&mut self) -> Result<()> {
        if self.math_state.pending_script.is_some() {
            return Err(Error::invalid_input(
                "数学模式中 ^/_ 后缺少上标/下标（Missing { inserted）",
            ));
        }
        let mut level = self
            .math_state.math
            .pop()
            .ok_or_else(|| Error::internal("close_math 无数学层"))?;
        let style = self.math_state.math_style;
        let was_display = self.list_modes.pop() == Some(Mode::DisplayMath);
        // display 内的对齐材料（amsmath align*/`\eqalignno` 的行盒）：
        // tex.web fin_align「Finish an alignment in a display」（L22622）把
        // 行盒直接拼进外层竖列表。NTex 的 DisplayMath 模式列表即此容器——
        // `$$` 关闭时拼进主竖列表，否则整段对齐内容随 pop 丢弃（align*
        // 单元格 DVI 为空的根因）。
        let display_material = self.lists.pop().unwrap_or_default();
        // 公式末尾收尾：未闭合 \left 报错恢复（TeX "Extra } or forgotten \right."，
        // 自动闭合；TRIP L298 `\left(\over\left(...`）；待定分式收尾（TeX 允许空分母）
        if level.left.is_some() {
            self.report_error("Extra } or forgotten \\right.");
            level.left = None;
        }
        Self::math_finish_fraction(&mut level);
        let mut nodes = self.math_to_hlist(&level.atoms, style);
        // tex.web after_math（L22421-22433）：`\eqno`/`\leqno` 后的材料是独立
        // mlist，`cur_style:=text_style` 转 hlist 后 hpack natural 成编号盒
        // a（e=width(a)）。math_eqno 事件时已把公式原子切去 eqno_formula，
        // 这里 pop 出的 level.atoms 是编号材料。
        let mut eqno_nodes: Vec<Node> = Vec::new();
        let mut leqno = false;
        if let Some(leq) = self.math_state.eqno_side.take() {
            leqno = leq;
            if let Some(formula) = self.math_state.eqno_formula.take() {
                nodes = self.math_to_hlist(&formula, style);
            }
            eqno_nodes = self.math_to_hlist(&level.atoms, MathStyle::Text);
        }
        // 行内数学边界标记（tex.web math_node）：`$` 进入/退出插 \\mathon/\\mathoff
        // （无维度；showbox 显示 `..\\mathon`。显示数学的公式盒内 TeX 同样有
        // math_node——本引擎显示公式走 `\\hbox to \\hsize` 盒，先只做行内）。
        let ms = self.params.mathsurround;
        if !was_display {
            nodes.insert(0, Node::MathOn { surrounded: ms });
            nodes.push(Node::MathOff { surrounded: ms });
        }
        if was_display {
            // tex.web finish_display：公式先收为自然宽盒，取 z=\displaywidth（≈\hsize）、
            // s=\displayindent、d=half(z-公式自然宽)，再按 d+s 与 \predisplaysize 的
            // 比较裁决长/短 display skip（短行 + 窄公式 → 短 skip）。
            let formula_width = hbox_dimensions(&nodes).width;
            let z = self.params.hsize;
            let s = self.params.displayindent;
            // tex.web @<Determine the displacement...@>（L22578）：d=half(z-w)；
            // 有编号且公式离编号太近（d<2e）时公式左移 d=half(z-w-e)。
            let e = hbox_dimensions(&eqno_nodes).width;
            let mut d = half(z - formula_width);
            if e > 0 && d < 2 * e {
                d = half(z - formula_width - e);
            }
            // tex.web：`(d+s<=pre_display_size) or l` → 长 skip（ clearance 不足），
            // 否则短 skip；leqno（l=true）恒取长 skip。
            let long = d + s <= self.math_state.predisplay_size || leqno;
            let above = if long {
                self.params.abovedisplayskip
            } else {
                self.params.abovedisplayshortskip
            };
            let below = if long {
                self.params.belowdisplayskip
            } else {
                self.params.belowdisplayshortskip
            };
            // 顺序照 tex.web：penalty(\predisplaypenalty) → 上 glue → 公式盒 →
            // penalty(\postdisplaypenalty) → 下 glue。
            self.append(Node::Penalty {
                penalty: self.params.predisplaypenalty,
            });
            self.append_param_glue(above);
            // 公式盒 = `\hbox to \hsize`（两侧 \hfil 居中；displaywidth≈\hsize）。
            // 公式为空但 display 内有对齐材料（amsmath align*：公式内容全在
            // 对齐行里）时跳过空盒——tex.web 对齐显示无公式盒，空 `\hbox to
            // \hsize` 只会多出一段空白行距；TRIP 的空显示（无对齐材料）仍照常
            // 产盒（行为不变）。
            if e > 0 {
                // tex.web @<Append the display and perhaps also the equation
                // number@>（L22606）：行 = 公式 + kern(z-w-e-d) + 编号盒，
                // hpack(natural) 后 shift=s+d——编号右缘落在 displaywidth
                // 右缘（eqno）/左缘（leqno：行序反过来且 d 归 0）。
                // 不做 @<Squeeze...@>（公式挤窄）臂：公式过宽时 kern 为负，
                // 与 tex.web 同样放行。
                let line = if leqno {
                    let mut l = eqno_nodes;
                    l.push(Node::Kern {
                        width: z - formula_width - e - d,
                    });
                    l.extend(nodes);
                    d = 0;
                    l
                } else {
                    let mut l = nodes;
                    l.push(Node::Kern {
                        width: z - formula_width - e - d,
                    });
                    l.extend(eqno_nodes);
                    l
                };
                let mut b = hpack(&line, hbox_dimensions(&line).width);
                b.shift = s + d;
                self.push_box_crossing_glue(Node::Box(b));
            } else if !nodes.is_empty() || display_material.is_empty() {
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
                // tex.web finish_display 用 append_to_vlist(b) 落公式盒：先按
                // prev_depth 插行间 glue（baselineskip/lineskip），再落盒——
                // 公式前后的 12pt 行距由此而来。走 `append` 会漏掉这段 glue
                // （P5：display 前垂直跳缺 interline glue）。
                self.push_box_crossing_glue(Node::Box(hpack(&line, self.params.hsize)));
            }
            // 对齐行盒拼接（tex.web L22622 `link(tail):=p`；行间 interline
            // 胶水已由 align_fin 按 append_to_vlist 语义生成，此处原样拼接）。
            for n in display_material {
                self.append(n);
            }
            self.append(Node::Penalty {
                penalty: self.params.postdisplaypenalty,
            });
            self.append_param_glue(below);
            // 后续文字续排：无 parskip/缩进（TeX 公式仍在段内）
            self.math_state.after_display = true;
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
                self.math_state.pending_script = Some(true);
                Ok(())
            }
            ntex_core::Catcode::Subscript => {
                self.math_state.pending_script = Some(false);
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
        if self.math_state.sqrt_pending {
            self.math_state.sqrt_pending = false;
            let level = self
                .math_state.math
                .last_mut()
                .ok_or_else(|| Error::internal("数学原子无数学层"))?;
            level.atoms.push(MathAtom::Radical {
                base: vec![atom],
                delim: SQRT_DELIM_CODE,
            });
            return Ok(());
        }
        // `\radical<delim>` 单原子字段：`\radical"3 x`（TRIP L412 everymath 注入路径）
        if let Some(delim) = self.math_state.radical_pending.take() {
            let level = self
                .math_state.math
                .last_mut()
                .ok_or_else(|| Error::internal("数学原子无数学层"))?;
            level.atoms.push(MathAtom::Radical {
                base: vec![atom],
                delim,
            });
            return Ok(());
        }
        // `\accent`/`\mathaccent` 单原子 nucleus 字段：第一个原子充当重音符，
        // 第二个原子是被重音内容（tex.web math_ac：accent 字段在前）。
        if self.math_state.accent_pending {
            if let Some(level) = self.math_state.math.last_mut() {
                if let Some(MathAtom::Accent { nucleus, .. }) = level.atoms.last_mut() {
                    nucleus.push(atom);
                    self.math_state.accent_pending = false;
                    return Ok(());
                }
            }
        }
        // `\mathbin` 等单原子字段：`\mathbin+`（Char 改类，其余包 Classed）。
        // 单字符是合法字段（tex.web scan_math letter 分支：`\mathord x` 不报错）；
        // 非字符非 { token 的 Missing { inserted 由 check_math_field_break 负责
        //（原语事件入口）。
        if let Some(class) = self.math_state.class_pending.take() {
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
                mu,
                ..
            } if self.math_state.nonscript_pending => MathAtom::MSkip {
                width,
                stretch,
                shrink,
                nonscript: true,
                mu,
            },
            other => other,
        };
        self.math_state.nonscript_pending = false;
        self.math_push_atom_raw(atom)
    }

    /// 原始追加（含脚本挂载）：`x^2`/`x_i`/`x_i^2`。
    fn math_push_atom_raw(&mut self, atom: MathAtom) -> Result<()> {
        // 先探测原子缺失（报错写 transcript 需 &mut self，避免与 level 借用冲突）
        if self.math_state.pending_script.is_some()
            && self.math_state.math.last().is_some_and(|l| l.atoms.is_empty())
        {
            // TeX：^/_ 前无原子 → "Missing { inserted" 恢复（插入空原子；TRIP L263）
            self.report_error("Missing { inserted.");
        }
        // 重音符 nucleus 字段在重音符之后（`\accent\x`）：TeX scan_math 对非字符
        // token 报 "Missing { inserted" 放回重扫（trip.tex L396）。
        if self.math_state.accent_pending
            && self
                .math_state.math
                .last()
                .is_some_and(|l| {
                    matches!(l.atoms.last(), Some(MathAtom::Accent { nucleus, .. }) if nucleus.is_empty())
                })
        {
            self.report_error("Missing { inserted.");
        }
        let level = self
            .math_state.math
            .last_mut()
            .ok_or_else(|| Error::internal("数学原子无数学层"))?;
        if let Some(is_sup) = self.math_state.pending_script.take() {
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
            let frac = MathAtom::Fraction {
                num: fp.num,
                den,
                thickness: fp.thickness,
            };
            // tex.web make_fraction：withdelims 系把定界符（var_delimiter 节点）
            // 包在分式两侧——复用 \left...\right 的 Delimited 装配径
            if fp.delims.0.is_none() && fp.delims.1.is_none() {
                level.atoms.push(frac);
            } else {
                level.atoms.push(MathAtom::Delimited {
                    left: fp.delims.0,
                    body: vec![frac],
                    right: fp.delims.1,
                });
            }
        }
    }

    /// 普通数学组收口原子（tex.web L22330 math_group：核存 sub_mlist 后的花括号
    /// 消除特例——组恰为一个**空脚本的 Ord noad** 时 `mem[saved(0)]:=mem[nucleus(p)]`
    /// 核直接取代组）：单个 Ord 字符原样返回、单个 Ord Classed 解包取其内容，
    /// 其余（含空组、Bin/Frac/Scripts 等非 Ord noad）装箱为 Ord Classed。
    fn plain_group_atom(mut atoms: Vec<MathAtom>) -> MathAtom {
        if atoms.len() == 1 {
            match atoms.pop() {
                Some(MathAtom::Char(mc)) if mc.class == MathClass::Ord => {
                    return MathAtom::Char(mc);
                }
                Some(MathAtom::Classed {
                    class: MathClass::Ord,
                    content,
                }) => {
                    return MathAtom::Classed {
                        class: MathClass::Ord,
                        content,
                    };
                }
                Some(other) => {
                    atoms.push(other);
                }
                None => {}
            }
        }
        MathAtom::Classed {
            class: MathClass::Ord,
            content: atoms,
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

    /// tex.web mlist_to_hlist 第一遍（L14323-14349）：不在二元语境的 Bin 转
    /// Ord。r_type 初值 op_noad ⇒ 列表开头的 Bin 也转（前导负号不吃
    /// medmuskip）；glue/kern/penalty/style 节点跳过不改语境；Rel/Close/Punct
    /// （含右定界符）触发对其前一 noad 的追溯转换。左定界符（tex.web
    /// left_noad）计入前向语境但不算追溯触发位。返回逐原子有效类。
    fn effective_classes(atoms: &[MathAtom]) -> Vec<Option<MathClass>> {
        let mut eff: Vec<Option<MathClass>> = atoms.iter().map(Self::math_class).collect();
        let mut last_noad: Option<usize> = None;
        for i in 0..eff.len() {
            if eff[i].is_none() {
                continue;
            }
            let ctx = last_noad.map_or(true, |j| {
                matches!(
                    eff[j],
                    Some(MathClass::Bin | MathClass::Op | MathClass::Rel | MathClass::Open | MathClass::Punct)
                )
            });
            if ctx && eff[i] == Some(MathClass::Bin) {
                eff[i] = Some(MathClass::Ord);
            }
            if matches!(eff[i], Some(MathClass::Rel | MathClass::Close | MathClass::Punct)) {
                if let Some(j) = last_noad {
                    if eff[j] == Some(MathClass::Bin) {
                        eff[j] = Some(MathClass::Ord);
                    }
                }
            }
            last_noad = Some(i);
        }
        eff
    }

    /// 数学列表 → 水平节点（M4-1：spacing 胶水 + 字符 + 上下标盒）。
    fn math_to_hlist(&self, atoms: &[MathAtom], style: MathStyle) -> Vec<Node> {
        let mut out = Vec::new();
        let mut style = style;
        let eff = Self::effective_classes(atoms);
        let mut prev: Option<MathClass> = None;
        for (idx, atom) in atoms.iter().enumerate() {
            // 样式切换原子就地生效（影响后续原子字阶与 spacing）
            if let MathAtom::Style(s) = atom {
                style = *s;
                continue;
            }
            let cur = eff[idx];
            if let (Some(p), Some(c)) = (prev, cur) {
                let code = spacing_code(p, c, style);
                // 脚本系风格（tex.web L15077 spacing 解码）：仅 "2"（无条件
                // thin）保留，条件系 "1"/"3"/"4"（thin/med/thick）一律归零
                // ——GT `x_i^2` 的下标 `i=1` 在 script 风格零胶水
                let suppress = style.size_kind() >= 1 && !matches!(code, SpacingCode::Thin);
                match code {
                    SpacingCode::None | SpacingCode::Tight => {}
                    _ if suppress => {}
                    code => {
                        // ETRIP P0 \muexpr 校准：muskip_params[i] 字段以 mu 数值存
                        // （默认 thin=3mu 在 cmr10 → 实际 sp ≈ 1.6667pt；etrip 在
                        //  smalltrip=5pt → 18mu 实际 sp = 5pt，对齐 etrip.log）。
                        // math_to_hlist 按当前 style 的 family-2 em/18 缩放到 sp。
                        let idx = code_idx(code);
                        let g = self.math_state.muskip_params[idx];
                        let (w, st, sh) = if self.math_state.muskip_is_mu[idx] {
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
                            stretch_order: g.stretch_order,
                            shrink_order: g.shrink_order,
                        });
                    }
                }
            }
            if cur.is_some() {
                prev = cur;
            }
            out.extend(self.math_atom_nodes(atom, style));
            // 斜体修正（tex.web §759-762）：紧邻后随原子作 make_ord 判据
            if let MathAtom::Char(mc) = atom {
                if let Some(kern) = self.italic_kern_after(mc, style, false, atoms.get(idx + 1)) {
                    out.push(kern);
                }
            }
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
                // 大算符带上下标：limits 堆叠（tex.web make_op subtype normal
                // 且 cur_style<text_style（display 两态）→ limits；
                // `\sum_{n=1}^{\infty}` 上下限居中于 Σ）
                if let MathAtom::Char(mc) = base.as_ref() {
                    if mc.class == MathClass::Op && style.code() < 2 {
                        return self.op_nodes(mc, style, sub.as_deref(), sup.as_deref());
                    }
                }
                let mut out = self.math_atom_nodes(base, style);
                // 斜体修正 delta（tex.web L14855-14866 核字符翻译）：delta :=
                // char_italic；sub 为空时 delta≥0 转**核后 kern** 且 delta 清零
                // （L14864），sub 在场时 kern 不落、delta 交 make_scripts 作
                // 上标盒水平偏移（combo 臂）。Op 核的 delta 走 make_op（此处
                // 仅 display 外落到此臂，保持 0）。
                let mut delta = 0;
                if let MathAtom::Char(mc) = base.as_ref() {
                    if sub.is_some() {
                        let (font, num, den) = self.math_char_font(mc, style);
                        let it = self.fonts.char_italic(font, mc.charcode);
                        delta = if num == den { it } else { xn_over_d(it, num, den) };
                    } else if let Some(kern) = self.italic_kern_after(mc, style, false, None) {
                        out.push(kern);
                    }
                }
                // tex.web make_scripts L14884：脚本体清理盒 + 垂直清位；
                // sub/sup 同时在场 → 单 vpack 合并盒（combo），不是并排两盒。
                let (mut shift_up, mut shift_down) = self.script_base_shifts(&out, style);
                let kind = style.size_kind();
                let xh = self.mathsy_x_height(kind);
                // 上标清理盒（@<Construct a superscript box |x|@>）
                let sup_box = sup.as_ref().map(|sup_atoms| {
                    let mut b = self.math_clean_box(sup_atoms, style.sup_style());
                    b.width += self.params.scriptspace;
                    // clr：cramped→sup3、display→sup1、其余→sup2（tex.web L14929）
                    let clr = if style.is_cramped() {
                        self.mathsy_param(kind, 15)
                    } else if style.code() < 2 {
                        self.mathsy_param(kind, 13)
                    } else {
                        self.mathsy_param(kind, 14)
                    };
                    if shift_up < clr {
                        shift_up = clr;
                    }
                    let clr2 = b.depth + xh / 4;
                    if shift_up < clr2 {
                        shift_up = clr2;
                    }
                    b
                });
                // 下标清理盒（@<Construct a subscript box |x|@>）
                let sub_box = sub.as_ref().map(|sub_atoms| {
                    let mut b = self.math_clean_box(sub_atoms, style.sub_style());
                    b.width += self.params.scriptspace;
                    b
                });
                match (sup_box, sub_box) {
                    (Some(mut x), Some(y)) => {
                        // combo 臂（@<Construct a sub/superscript combination box
                        // |x|...@> L14940）：下限 sub2，4×rule_thickness 间隙不足
                        // 时下标下移、必要时整体上移（4/5·x_height 上限），最终
                        // vpack[sup(+delta 偏移), kern, sub]、盒 shift=shift_down。
                        let sub2 = self.mathsy_param(kind, 17);
                        if shift_down < sub2 {
                            shift_down = sub2;
                        }
                        let rt = self.math_rule_thickness(kind);
                        let mut clr =
                            4 * rt - ((shift_up - x.depth) - (y.height - shift_down));
                        if clr > 0 {
                            shift_down += clr;
                            clr = (xh * 4) / 5 - (shift_up - x.depth);
                            if clr > 0 {
                                shift_up += clr;
                                shift_down -= clr;
                            }
                        }
                        x.shift = delta; // vlist 内 Box.shift = 水平偏移
                        let kern = (shift_up - x.depth) - (y.height - shift_down);
                        let children = vec![
                            Node::Box(x),
                            Node::Kern { width: kern },
                            Node::Box(y),
                        ];
                        // 自然装（tex.web `vpack(x,natural)`）：目标高传自然总高，
                        // diff=0 保持自然 height/depth；参考点在末盒基线。
                        let dims = vbox_dimensions(&children);
                        let mut b = vpack(children, dims.height + dims.depth, i64::MAX);
                        b.shift = shift_down;
                        out.push(Node::Box(b));
                    }
                    (Some(mut x), None) => {
                        x.shift = -shift_up;
                        out.push(Node::Box(x));
                    }
                    (None, Some(mut y)) => {
                        // 无上标：下限 sub1（mathsy 16），且不低于
                        // height(sub)-4/5·x_height（下标盒构造末尾）
                        let sub1 = self.mathsy_param(kind, 16);
                        if shift_down < sub1 {
                            shift_down = sub1;
                        }
                        let clr = y.height - (xh * 4) / 5;
                        if shift_down < clr {
                            shift_down = clr;
                        }
                        y.shift = shift_down;
                        out.push(Node::Box(y));
                    }
                    (None, None) => {}
                }
                out
            }
            MathAtom::Fraction {
                num,
                den,
                thickness,
            } => self.fraction_nodes(num, den, *thickness, style),
            MathAtom::Radical { base, delim } => self.radical_nodes(base, *delim, style),
            // \underline/\overline：M4-2 简化——内容直接输出（底线/顶线渲染
            // M4-3）；核取 cramped_style（tex.web make_underline/make_overline）
            MathAtom::Underline { base } | MathAtom::Overline { base } => {
                self.math_to_hlist(base, style.cramped())
            }
            MathAtom::Delimited { left, body, right } => {
                let body_nodes = self.math_to_hlist(body, style);
                // tex.web var_delimiter 的 needed = 括起内容的高+深（先排内容再选字形）
                let dims = hbox_dimensions(&body_nodes);
                let need = dims.height + dims.depth;
                let mut inner = Vec::new();
                if let Some(d) = left {
                    inner.extend(self.delim_nodes_sized(*d, style, Some(need)));
                }
                inner.extend(body_nodes);
                if let Some(d) = right {
                    inner.extend(self.delim_nodes_sized(*d, style, Some(need)));
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
            // tex.web L14848：核 sub_mlist → 递归转换后 hpack(natural)——组是
            // 一个盒，脚本 shift 取盒高（GT \box0：`.\hbox(6.94444+0.83333)`）。
            MathAtom::Classed { content, .. } => {
                let nodes = self.math_to_hlist(content, style);
                let w = hbox_dimensions(&nodes).width;
                vec![Node::Box(hpack(&nodes, w))]
            }
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
                mu,
            } => {
                if *nonscript && style.size_kind() >= 1 {
                    Vec::new()
                } else {
                    // mu 胶在 mlist_to_hlist 按 em/18 换算（tex.web math_glue；
                    // 截断对齐 math_glue/mu_mult，见 mu_to_sp 注释）；pt 胶原样落
                    let (w, st, sh) = if *mu {
                        let em = self.math_em(style);
                        (
                            mu_to_sp(*width, em),
                            mu_to_sp(*stretch, em),
                            mu_to_sp(*shrink, em),
                        )
                    } else {
                        (*width, *stretch, *shrink)
                    };
                    vec![Node::Glue {
                        name: None,
                        width: w,
                        stretch: st,
                        shrink: sh,
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
        // 分子/分母样式（tex.web L13858-13859 num_style/denom_style）：降一级
        // 字阶，分子保持压性、分母恒压
        let mut num_b = BoxNode::new_hbox(self.math_to_hlist(num, style.numerator_style()));
        let mut den_b = BoxNode::new_hbox(self.math_to_hlist(den, style.denominator_style()));
        // num1/num2/num3/denom1/denom2 = mathsy(8..12)（tex.web @d L13817-13821）
        let kind = style.size_kind();
        let fam2 = self
            .math_state.math_fonts
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
            .math_state.math_fonts
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
        let display = style.is_display();
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
        // tex.web make_fraction 尾段（L14656-14660）：new_hlist = hpack[左定界符,
        // v, 右定界符]。定界符域为空时 var_delimiter 走 null 分支（L14108-14112）：
        // 空盒宽 \nulldelimiterspace，shift = −axis_height（GT 实证
        // `.\hbox(0.0+0.0)x1.2, shifted -2.5`）
        let null_delim = move || BoxNode {
            kind: BoxKind::HBox,
            width: self.params.nulldelimiterspace,
            height: 0,
            depth: 0,
            shift: -axis,
            children: Vec::new(),
        };
        let kids = vec![
            Node::Box(null_delim()),
            Node::Box(v),
            Node::Box(null_delim()),
        ];
        vec![Node::Box(BoxNode::new_hbox(kids))]
    }

    /// 数学轴高度（tex.web `mathsy(22)`：fam 2 当前字阶字体 fontdimen 22；
    /// 字体未加载回退 0）。
    fn axis_height(&self, style: MathStyle) -> i64 {
        let kind = style.size_kind();
        let font = self
            .math_state.math_fonts
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
            .math_state.math_fonts
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
        if style.is_display() {
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
        // 上下限字阶（tex.web sup_style/sub_style）：降一级、上标保持压性
        let sup_style = style.sup_style();
        let sub_style = style.sub_style();
        let mut sup_b = sup
            .filter(|a| !a.is_empty())
            .map(|a| BoxNode::new_hbox(self.math_to_hlist(a, sup_style)));
        let mut sub_b = sub
            .filter(|a| !a.is_empty())
            .map(|a| BoxNode::new_hbox(self.math_to_hlist(a, sub_style)));
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

    /// 根式 → 节点（tex.web make_radical L14476）：x = clean_box(核,
    /// cramped_style)；clr = display ? drt+x_height/4 : drt+drt/4；
    /// y = var_delimiter(根号域, h(x)+d(x)+clr+drt)；y 深度过盈时
    /// clr += half(delta)；y shift = −(h(x)+clr)；末盒 = hpack[y,
    /// overbar(x, clr, height(y))]（GT `\sqrt b`：外盒 9.32217+1.07779 =
    /// 根号盒 0.39998+9.6 @shift −8.52222 与竖排 [kern.39998, rule.39998,
    /// kern1.57777, b 6.94444] 的 hpack 归并）。
    fn radical_nodes(&self, base: &[MathAtom], delim: u32, style: MathStyle) -> Vec<Node> {
        let x = self.math_clean_box(base, style.cramped());
        let kind = style.size_kind();
        let drt = self.math_rule_thickness(kind);
        let mut clr = if style.code() < 2 {
            drt + self.mathsy_x_height(kind) / 4
        } else {
            drt + drt / 4
        };
        let mut y = self.radical_delimiter(delim, kind, x.height + x.depth + clr + drt);
        let delta = y.depth - (x.height + x.depth + clr);
        if delta > 0 {
            clr += half(delta);
        }
        y.shift = -(x.height + clr);
        let over = self.overbar_box(x, clr, y.height);
        let children = vec![Node::Box(y), Node::Box(over)];
        let w = hbox_dimensions(&children).width;
        vec![Node::Box(hpack(&children, w))]
    }

    /// tex.web overbar（L14560）：vpack[kern t, rule t(宽=内容宽), kern k, 盒]。
    fn overbar_box(&self, b: BoxNode, k: i64, t: i64) -> BoxNode {
        let children = vec![
            Node::Kern { width: t },
            Node::Rule {
                width: b.width,
                height: t,
                depth: 0,
            },
            Node::Kern { width: k },
            Node::Box(b),
        ];
        let dims = vbox_dimensions(&children);
        vpack(children, dims.height + dims.depth, i64::MAX)
    }

    /// tex.web var_delimiter（L13880 起）按定界符码取根号变体：码拆
    /// small=(fam 2,'p')/large=(fam 3,'p')（\sqrt 默认码 = plain.tex
    /// `\radical"270370`），small 取 fam 的当前字阶字体，h+d ≥ v 即停，
    /// 不足则落到 large（tex.web best-so-far：large_attempt 后必返回某变体，
    /// 单字形近似——字阶内 char list 逐级放大与 extensible 拼接仍属 M4-3）。
    /// 码 0（或两变体均 (fam 0, char 0)，即 tex.web `(z<>0)or(x<>min_quarterword)`
    /// 排除位）走 null 分支：空盒宽 \nulldelimiterspace（L14108-14112）。
    fn radical_delimiter(&self, delim: u32, kind: usize, v: i64) -> BoxNode {
        let small_fam = ((delim >> 20) & 0xF) as usize;
        let small_char = (delim >> 12) & 0xFF;
        let large_fam = ((delim >> 8) & 0xF) as usize;
        let large_char = delim & 0xFF;
        let mut best: Option<(FontId, u32, i64, i64, i64)> = None;
        for (fam, ch) in [(small_fam, small_char), (large_fam, large_char)] {
            if fam == 0 && ch == 0 {
                continue; // null delimiter 域
            }
            if let Some(f) = self
                .math_state
                .math_fonts
                .get(fam)
                .and_then(|s| s.get(kind).copied().flatten())
            {
                let (w, h, d) = self.math_metrics(f, ch, 1, 1);
                if h + d >= v {
                    return Self::delim_char_box(f, ch, w, h, d);
                }
                best = Some((f, ch, w, h, d));
            }
        }
        if let Some((f, ch, w, h, d)) = best {
            return Self::delim_char_box(f, ch, w, h, d);
        }
        // null 分支：宽 \nulldelimiterspace 的空盒（同 make_fraction 的外壳）
        BoxNode {
            kind: BoxKind::HBox,
            width: self.params.nulldelimiterspace,
            height: 0,
            depth: 0,
            shift: 0,
            children: Vec::new(),
        }
    }

    /// var_delimiter char_box（tex.web L13958）：单字形盒，宽含斜体修正，
    /// 高深取字形度量。
    fn delim_char_box(f: FontId, ch: u32, w: i64, h: i64, d: i64) -> BoxNode {
        BoxNode::new_hbox(vec![Node::Char {
            font: f,
            charcode: ch,
            width: w,
            height: h,
            depth: d,
        }])
    }

    /// 定界符字符节点（当前字体 + 字阶缩放；M4-3 换 cmex10 变体伸缩）。
    fn delim_nodes(&self, d: u32, style: MathStyle) -> Vec<Node> {
        self.delim_nodes_sized(d, style, None)
    }

    /// tex.web var_delimiter（L1184）的两档近似（不含 cmex 扩展拼接）：
    /// 27 位定界码拆 small（fam=(d/@"4000000) mod 16、char=(d/@"10000) mod 256）/
    /// large（fam=(d/256) mod 16、char=d mod 256）；needed=Some(内容高+深) 时
    /// small 字形不够高即取 large（`\binom`/`\left(\frac..` 落 cmex 大字，
    /// `\left(x` 落正文字体小括号）。needed=None 维持小字形。
    fn delim_nodes_sized(&self, d: u32, style: MathStyle, needed: Option<i64>) -> Vec<Node> {
        let kind = style.size_kind();
        let pick = |s: &Self, fam: u32, ch: u32| -> Option<(FontId, u32, i64, i64, i64)> {
            let font = s
                .math_state
                .math_fonts
                .get(fam as usize)
                .and_then(|t| t.get(kind).copied().flatten())?;
            let (w, h, dd) = s.fonts.metrics(font, ch);
            if w == 0 && h == 0 && dd == 0 {
                return None;
            }
            Some((font, ch, w, h, dd))
        };
        let small_fam = (d >> 22) & 0xF;
        let small_char = (d >> 16) & 0xFF;
        let large_fam = (d >> 8) & 0xF;
        let large_char = d & 0xFF;
        let mut chosen = pick(self, small_fam, small_char);
        if let Some(need) = needed {
            if let Some((_, _, _, h, dd)) = chosen {
                if h + dd < need {
                    chosen = pick(self, large_fam, large_char).or(chosen);
                }
            }
        }
        let (font, ch, w, h, dd) = chosen.unwrap_or_else(|| {
            let (wn, dn) = style.scale();
            let (w, h, dd) = self.math_metrics(self.current_font, small_char, wn, dn);
            (self.current_font, small_char, w, h, dd)
        });
        vec![Node::Char {
            font,
            charcode: ch,
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
        let kind = style.size_kind();
        if let Some(Some(f)) = self.math_state.math_fonts.get(mc.fam as usize).map(|s| s[kind]) {
            (f, 1, 1) // 族字体已按字阶设计字号，不缩放
        } else {
            let (num, den) = style.scale();
            (self.current_font, num, den)
        }
    }

    /// tex.web §759-762（mlist_to_hlist 第三遍 `@<Create a character node
    /// |p| for |nucleus(q)|...@>`）：math_char 核取字后 `delta=char_italic`；
    /// 核**无下标**且 delta≠0 时在字符后追加显式 kern（有下标时 delta 交
    /// make_scripts 作 sub/sup 水平偏移，不落 kern 节点；demo1 实证：官方
    /// `E =` 左侧 right219813 = kern37773 + thickmuskip182040，而 `c^2` 因
    /// cmmi10 'c' italic=0 无 kern、sup 盒宽 261235+scriptspace=294003）。
    /// make_ord（§14759）转成的 math_text_char：核无脚本、紧邻后随同族
    /// math_char 简单 noad（ord..punct，不含 inner）→ 正文字体（space≠0）
    /// 词中不加斜体修正。Op 大算符走 make_op：delta 进 vcenter/脚本偏移，
    /// 同样不落 kern。`has_sub`=本原子带下标；`next`=紧邻下一原子。
    fn italic_kern_after(
        &self,
        mc: &MathChar,
        style: MathStyle,
        has_sub: bool,
        next: Option<&MathAtom>,
    ) -> Option<Node> {
        if mc.class == MathClass::Op || has_sub {
            return None;
        }
        // make_ord 只挂在 ord_noad 上（§14419）：斜体修正本身对所有简单 noad
        // 的 math_char 核生效，text 字体抑制仅 Ord/Var 核。
        if matches!(mc.class, MathClass::Ord | MathClass::Var) {
            if let Some(MathAtom::Char(nc)) = next {
                if nc.fam == mc.fam
                    && nc.class != MathClass::Inner
                    && self.fonts.space(self.math_char_font(mc, style).0).width != 0
                {
                    return None;
                }
            }
        }
        let (font, num, den) = self.math_char_font(mc, style);
        let it = self.fonts.char_italic(font, mc.charcode);
        if it == 0 {
            return None;
        }
        let it = if num == den { it } else { xn_over_d(it, num, den) };
        Some(Node::Kern { width: it })
    }

    /// tex.web clean_box（L14173）：字段内容转 hlist 后 hpack(natural)，
    /// 再过 @<Simplify a trivial box@>：单字符后跟**孤立 kern**（恰两节点）
    /// 时删 kern——脚本字段的斜体修正已由核的 delta/kern 表达，盒内不重复
    /// （GT 实证：`x_i^j` 上标盒 `.\seveni j` 后无 kern）。\ scriptspace 由
    /// 调用方（make_scripts）追加，clean_box 本身不碰宽度。
    fn math_clean_box(&self, atoms: &[MathAtom], style: MathStyle) -> BoxNode {
        let mut nodes = self.math_to_hlist(atoms, style);
        // tex.web clean_box "it's already clean"：结果恰为单个无移位盒时原样
        // 复用（如 sup 字段含分式壳盒，不另套一层 hbox）。
        if let [Node::Box(b)] = &nodes[..] {
            if b.shift == 0 {
                return b.clone();
            }
        }
        // 否则 hpack 先按 [char, italic kern] 自然宽装盒，随后 "Simplify a
        // trivial box" 才剥掉 kern——宽度已烙进盒里，不再重算。
        let w = hbox_dimensions(&nodes).width;
        if nodes.len() == 2
            && matches!(nodes[0], Node::Char { .. })
            && matches!(nodes[1], Node::Kern { .. })
        {
            nodes.truncate(1);
        }
        hpack(&nodes, w)
    }

    /// 上标提升量（M4-3）：fontdimen sup1（参数 11，无上标时 sup2/3）；回退 x_height×字阶。
    /// family-2（math symbols）字体在指定字阶槽的 fontdimen（tex.web
    /// `mathsy(n)`；缺字体/缺参数回 0，与 TeX nullfont 参数为 0 同义）。
    fn mathsy_param(&self, kind: usize, idx: usize) -> i64 {
        self.math_state.math_fonts
            .get(2)
            .and_then(|s| s.get(kind).copied().flatten())
            .map(|f| self.fonts.font_param(f, idx))
            .unwrap_or(0)
    }

    /// family-2 字体的 x_height（tex.web `math_x_height(cur_size)`）。
    fn mathsy_x_height(&self, kind: usize) -> i64 {
        self.math_state.math_fonts
            .get(2)
            .and_then(|s| s.get(kind).copied().flatten())
            .map(|f| self.fonts.x_height(f))
            .unwrap_or(0)
    }

    /// family-3（extension）字体该字阶的 fontdimen 8（tex.web
    /// `rule_thickness(cur_size)`：根式横线、combo 脚本 4t 间隙、分式线默认厚）。
    fn math_rule_thickness(&self, kind: usize) -> i64 {
        self.math_state.math_fonts
            .get(3)
            .and_then(|s| s.get(kind).copied().flatten())
            .map(|f| self.fonts.font_param(f, 8))
            .unwrap_or_else(|| 2 * SP_PER_PT / 5)
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
        // sup_drop/sub_drop 的字阶（tex.web `t:=script_size`，cur_style≥
        // script_style 才取 script_script_size）：display/text 两态用 script
        // 槽，script 系用 scriptscript 槽
        let t_kind = style.size_kind().max(1);
        let sup_drop = self.mathsy_param(t_kind, 18);
        let sub_drop = self.mathsy_param(t_kind, 19);
        (packed.height - sup_drop, packed.depth + sub_drop)
    }

    /// ETRIP P0 \muexpr 校准：取当前 style 的 1em（sp）—— tex.web §685 ÷18 即 1mu。
    /// 顺序取 family 2 该 style 的字体 fontdimen 6（quad，TeXbook 附录 G）。
    /// 回退链：family 2 字体 → current_font fontdimen 6 → 10pt（em=10pt 默认值，
    /// 对照 plain TeX 隐含 cmr10 设计字号，与 etrip 期望 cmr10 × 小字体族差距可接受）。
    fn math_em(&self, style: MathStyle) -> i64 {
        let kind = style.size_kind();
        let quad = if let Some(f) = self
            .math_state.math_fonts
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
