// ---------- 寄存器操作原语执行器 ----------
//
// 从 expand/primitive.rs 迁出的寄存器算术家族方法。沿用项目已有的"include! 分片"模式
// （见 expand/mod.rs 末尾），每片维持独立 `impl Expander { ... }` 块——Rust 允许
// 同一类型的多个 impl 块分散在不同文件，效果等价于单 impl。
//
// 涵盖：\advance/\multiply/\divide 的目标获取（fetch_register_target，含宏/\let
// 别名跟随的 TeX get_x_token 语义），增量应用（advance_register），标量应用
// （scale_register）；G3 起目标集合扩到内部参数族（advance_param/scale_param，
// 表见 free.rs::param_kind_of——tex.web do_register_command 的 assign_* 区）。

impl Expander {
    /// 读取寄存器运算目标（`\advance/\multiply/\divide` 共用）：TeX get_x_token
    /// 语义——宏别名（etrip `\edef\2{\csname count\endcsname}` 使 `\advance\22000by…`
    /// 即 `\advance\count2000by…`）与 `\let` 别名须展开/跟随到不可展开目标。
    fn fetch_register_target(&mut self, prim_name: &str) -> Result<u32> {
        let mut tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input(format!("{prim_name} 后缺少寄存器")))?
            .0;
        loop {
            let Some(csid) = tok.csid() else {
                return Err(Error::invalid_input(format!(
                    "{prim_name} 后必须是寄存器"
                )));
            };
            match self.eqtb.slot(csid).clone() {
                // 宏别名：展开后压帧，继续读下一个 token（展开结果可能仍是宏）
                EqSlot::Macro(m) if !(m.value.protected && self.suppress_expansion > 0) => {
                    let mut expansion = Vec::new();
                    self.expand_once((tok, false), &mut expansion)?;
                    self.stack.push(InputFrame::TokenList {
                        items: Arc::from(expansion),
                        pos: 0,
                    });
                    tok = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input(format!("{prim_name} 后缺少寄存器")))?
                        .0;
                }
                // \let 别名：跟随目标
                EqSlot::Alias(target) => tok = Token::control_sequence(target),
                _ => return Ok(csid),
            }
        }
    }

    /// `\advance<寄存器> <增量>`：寄存器运算（TeX arithmetic；etrip.tex 91 行
    /// `\advance\count20 1`）。目标支持 `\count/`\dimen/`\skip/`\muskip` 寄存器
    /// （数字下标或 `\countdef` 等 cs 绑定）与内部整数参数。
    fn exec_advance(&mut self) -> Result<()> {
        self.skip_spaces()?;
        let csid = self.fetch_register_target("\\advance")?;
        match self.eqtb.slot(csid).clone() {
            EqSlot::Primitive(prim)
                if matches!(
                    prim,
                    Primitive::Count | Primitive::Dimen | Primitive::Skip | Primitive::Muskip
                ) =>
            {
                let kind = match prim {
                    Primitive::Count => RegKind::Count,
                    Primitive::Dimen => RegKind::Dimen,
                    Primitive::Skip => RegKind::Skip,
                    _ => RegKind::Muskip,
                };
                let idx = self.scan_register_index()?;
                self.advance_register(kind, idx)
            }
            EqSlot::Register(kind, idx) => self.advance_register(kind, idx),
            // TRIP L410：\xspaceskip 胶水参数也可 \advance
            EqSlot::Primitive(Primitive::XSpaceSkip) => {
                self.scan_keyword(|w| w == "by")?;
                let delta = self.scan_glue()?;
                let old = self.params.xspaceskip;
                let global = self.is_global();
                if !global && self.group_level > 0 {
                    self.save_stack.push((
                        self.group_level,
                        SavedValue::Param {
                            kind: ParamKind::XSpaceSkip,
                            prev: self.params.get(ParamKind::XSpaceSkip),
                        },
                    ));
                }
                self.params.xspaceskip = add_glue(old, delta);
                self.sink.param_changed(
                    ParamKind::XSpaceSkip,
                    ParamValue::Glue(self.params.xspaceskip),
                )?;
                self.finish_assignment();
                Ok(())
            }
            // TRIP L434：\advance\prevdepth —— restricted horizontal mode 下
            // prevdepth 不可赋值，TeX 报错恢复（trip.log L6604-6607），不扫描增量。
            EqSlot::Primitive(Primitive::PrevDepth) => {
                let _ = self.sink.write16(
                    "! You can't use `\\prevdepth' after \\advance.\n\
                     I'm forgetting what you said and not changing anything.\n"
                        .to_string(),
                );
                self.finish_assignment();
                Ok(())
            }
            // 内部整数参数（\tracingstats/\language 等）也可 \advance
            EqSlot::Primitive(p) if int_param_index(p).is_some() => {
                let idx = int_param_index(p).expect("已检查 is_some");
                self.scan_keyword(|w| w == "by")?;
                let delta = self.scan_number()?;
                let val = self.params.misc[idx] + delta;
                let global = self.is_global();
                if !global && self.group_level > 0 {
                    self.save_stack.push((
                        self.group_level,
                        SavedValue::Param {
                            kind: ParamKind::MiscInt(idx),
                            prev: self.params.get(ParamKind::MiscInt(idx)),
                        },
                    ));
                }
                self.params.misc[idx] = val;
                self.finish_assignment();
                Ok(())
            }
            // G3（tex.web do_register_command）：内部参数族目标——\hsize/\vsize/
            // \voffset 等页面参数、\baselineskip/\parskip 等胶参数、\tolerance 等
            // 都在 assign_int/assign_dimen/assign_glue 区。增量按参数值类扫描，
            // 写通道复用 `\hsize=…` 的 assign_param（组作用域保存 + sink 镜像 +
            // tracingassigns），增量通道与赋值通道不另起炉灶。
            EqSlot::Primitive(p) if param_kind_of(p).is_some() => {
                self.advance_param(param_kind_of(p).expect("已检查 is_some"))
            }
            // \thinmuskip/\medmuskip/\thickmuskip：muskip 寄存器 0/1/2（与既有
            // 赋值 exec_muskip_param 同槽），增量走 mu 胶通道
            EqSlot::Primitive(p)
                if matches!(
                    p,
                    Primitive::ThinMuskip | Primitive::MedMuskip | Primitive::ThickMuskip
                ) =>
            {
                let idx = match p {
                    Primitive::ThinMuskip => 0,
                    Primitive::MedMuskip => 1,
                    _ => 2,
                };
                self.advance_register(RegKind::Muskip, idx)
            }
            _ => Err(Error::invalid_input(
                "\\advance 目标必须是寄存器或内部参数",
            )),
        }
    }

    /// `\advance<内部参数> <增量>`（G3）：按参数值类扫描增量（dimen/glue/number），
    /// 经 assign_param 落账。旧值在增量扫描后读取（tex.web do_register_command
    /// 同序——增量展开中若含对同一参数的赋值，以赋值后的值为基）。
    fn advance_param(&mut self, kind: ParamKind) -> Result<()> {
        self.scan_keyword(|w| w == "by")?;
        let delta = match self.params.get(kind) {
            ParamValue::Dimen(_) => ParamValue::Dimen(self.scan_dimen()?),
            ParamValue::Number(_) => ParamValue::Number(self.scan_number()?),
            ParamValue::Glue(_) => ParamValue::Glue(self.scan_glue()?),
        };
        let new = match (self.params.get(kind), delta) {
            (ParamValue::Dimen(v), ParamValue::Dimen(d)) => ParamValue::Dimen(v + d),
            (ParamValue::Number(v), ParamValue::Number(d)) => ParamValue::Number(v + d),
            (ParamValue::Glue(g), ParamValue::Glue(d)) => ParamValue::Glue(add_glue(g, d)),
            _ => return Err(Error::internal("参数值类在增量扫描中漂移")),
        };
        self.assign_param(kind, new)
    }

    /// `\advance` 的寄存器增量应用（TeX：`new = old + delta`，胶水逐分量加）。
    fn advance_register(&mut self, kind: RegKind, idx: usize) -> Result<()> {
        // 可选 `by` 关键字（TeX：`\advance\count0 by5` 与 `\advance\count0 5` 等价）
        self.scan_keyword(|w| w == "by")?;
        match kind {
            RegKind::Count => {
                let delta = self.scan_number()?;
                self.assign_count(idx, self.registers.count(idx) + delta);
            }
            RegKind::Dimen => {
                let delta = self.scan_dimen()?;
                self.assign_dimen(idx, self.registers.dimen(idx) + delta);
            }
            RegKind::Skip => {
                let delta = self.scan_glue()?;
                let old = self.registers.skip(idx);
                self.assign_skip(idx, add_glue(old, delta));
            }
            RegKind::Muskip => {
                let delta = self.scan_glue_mu()?;
                let old = self.registers.muskip(idx);
                self.assign_muskip(idx, add_glue(old, delta));
            }
            RegKind::Toks => return Err(Error::invalid_input("\\advance 不支持 \\toks")),
        }
        Ok(())
    }

    /// `\multiply/\divide<寄存器> by<n>`：寄存器标量乘/除（TeX arithmetic）。
    /// 目标支持 `\count/\dimen/\skip/\muskip` 寄存器与内部整数参数；
    /// 除数为 0 时按 TeX 语义保持不变。
    fn exec_multiply_divide(&mut self, prim: Primitive) -> Result<()> {
        self.skip_spaces()?;
        let csid = self.fetch_register_target("\\multiply/\\divide")?;
        match self.eqtb.slot(csid).clone() {
            EqSlot::Primitive(p)
                if matches!(
                    p,
                    Primitive::Count | Primitive::Dimen | Primitive::Skip | Primitive::Muskip
                ) =>
            {
                let kind = match p {
                    Primitive::Count => RegKind::Count,
                    Primitive::Dimen => RegKind::Dimen,
                    Primitive::Skip => RegKind::Skip,
                    _ => RegKind::Muskip,
                };
                let idx = self.scan_register_index()?;
                self.scale_register(kind, idx, prim)
            }
            EqSlot::Register(kind, idx) => self.scale_register(kind, idx, prim),
            // 内部整数参数（\tracingstats/\language 等）也可 \multiply/\divide
            EqSlot::Primitive(p) if int_param_index(p).is_some() => {
                let idx = int_param_index(p).expect("已检查 is_some");
                self.scan_keyword(|w| w == "by")?;
                let f = self.scan_number()?;
                let val = self.params.misc[idx];
                let new = if prim == Primitive::Multiply {
                    val * f
                } else if f != 0 {
                    val / f
                } else {
                    val
                };
                let global = self.is_global();
                if !global && self.group_level > 0 {
                    self.save_stack.push((
                        self.group_level,
                        SavedValue::Param {
                            kind: ParamKind::MiscInt(idx),
                            prev: self.params.get(ParamKind::MiscInt(idx)),
                        },
                    ));
                }
                self.params.misc[idx] = new;
                self.finish_assignment();
                Ok(())
            }
            // G3（tex.web do_register_command）：内部参数族目标，乘除与增量同集合
            EqSlot::Primitive(p) if param_kind_of(p).is_some() => {
                self.scale_param(param_kind_of(p).expect("已检查 is_some"), prim)
            }
            // \thinmuskip/\medmuskip/\thickmuskip：muskip 寄存器 0/1/2，mu 胶通道
            EqSlot::Primitive(p)
                if matches!(
                    p,
                    Primitive::ThinMuskip | Primitive::MedMuskip | Primitive::ThickMuskip
                ) =>
            {
                let idx = match p {
                    Primitive::ThinMuskip => 0,
                    Primitive::MedMuskip => 1,
                    _ => 2,
                };
                self.scale_register(RegKind::Muskip, idx, prim)
            }
            _ => Err(Error::invalid_input(
                "\\multiply/\\divide 目标必须是寄存器或内部参数",
            )),
        }
    }

    /// `\multiply/\divide<内部参数> by<n>`（G3）：标量缩放与 [`Self::scale_register`]
    /// 同式（胶水逐分量、除 0 保持不变），经 assign_param 落账。
    fn scale_param(&mut self, kind: ParamKind, prim: Primitive) -> Result<()> {
        self.scan_keyword(|w| w == "by")?;
        let f = self.scan_number()?;
        // TeX：除以 0 保持不变（不报错）
        let scale = |v: i64| {
            if prim == Primitive::Multiply {
                v * f
            } else if f != 0 {
                v / f
            } else {
                v
            }
        };
        let new = match self.params.get(kind) {
            ParamValue::Dimen(v) => ParamValue::Dimen(scale(v)),
            ParamValue::Number(v) => ParamValue::Number(scale(v)),
            // 乘除不改无穷阶（分量标量缩放，阶保留）
            ParamValue::Glue(g) => ParamValue::Glue(Glue {
                width: scale(g.width),
                stretch: scale(g.stretch),
                shrink: scale(g.shrink),
                ..g
            }),
        };
        self.assign_param(kind, new)
    }

    /// `\multiply/\divide` 的寄存器标量应用（胶水逐分量乘/除）。
    fn scale_register(&mut self, kind: RegKind, idx: usize, prim: Primitive) -> Result<()> {
        // 可选 `by` 关键字（TeX：`\multiply\count0 by5` 与 `\multiply\count0 5` 等价）
        self.scan_keyword(|w| w == "by")?;
        let f = self.scan_number()?;
        // TeX：除以 0 保持不变（不报错）
        let scale = |v: i64| {
            if prim == Primitive::Multiply {
                v * f
            } else if f != 0 {
                v / f
            } else {
                v
            }
        };
        match kind {
            RegKind::Count => self.assign_count(idx, scale(self.registers.count(idx))),
            RegKind::Dimen => self.assign_dimen(idx, scale(self.registers.dimen(idx))),
            RegKind::Skip => {
                let old = self.registers.skip(idx);
                // 乘除不改无穷阶（TeX：分量标量缩放，阶保留）
                self.assign_skip(
                    idx,
                    Glue {
                        width: scale(old.width),
                        stretch: scale(old.stretch),
                        shrink: scale(old.shrink),
                        ..old
                    },
                );
            }
            RegKind::Muskip => {
                let old = self.registers.muskip(idx);
                self.assign_muskip(
                    idx,
                    Glue {
                        width: scale(old.width),
                        stretch: scale(old.stretch),
                        shrink: scale(old.shrink),
                        ..old
                    },
                );
            }
            RegKind::Toks => return Err(Error::invalid_input("\\multiply/\\divide 不支持 \\toks")),
        }
        Ok(())
    }
}
