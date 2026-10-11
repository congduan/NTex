// Toks 参数 / 段落 / 方向 / 收尾 / 盒子尺寸 原语 dispatcher。
//
// 主题：
// - cs 绑定（Chardef/Countdef/Dimendef/Skipdef/Muskipdef/Toksdef/MathCharDef）
// - toks 参数（EveryPar/.../ErrHelp/EveryDisplay/EveryMath/EveryJob）
// - 段落（SpaceFactor/Parshape/ParshapeLength/ParshapeIndent/ParshapeDimen）
// - TeXXeT 方向（BeginL/EndL/BeginR/EndR）
// - 收尾（Dump/ReadLine）
// - 盒子尺寸（Wd/Ht/Dp）
// - PrevDepth（受限模式的特殊处理）
//
// 维护约定（与 expand/mod.rs 既有 include! 链一致）：
// - 仅 `impl Expander { ... }` 块，无 use / 无模块声明；
// - 入口 `dispatch_toks_state(prim: Primitive)` 由 `primitive.rs` 主 match 委托；
// - 函数 1:1 来自 `exec_primitive` 主 match，零行为变化。

impl Expander {
    /// Toks 参数 / 段落 / 方向 / 收尾 / 盒子尺寸原语 dispatcher。
    pub(super) fn dispatch_toks_state(&mut self, prim: Primitive) -> Result<()> {
        match prim {
            // ETRIP 冲刺：\chardef\cs=<num>（cs 绑定字符，cat 12）
            Primitive::Chardef => {
                let csid = self.scan_cs_ident()?;
                // tex.web shorthand_def（L22906）：`define(p,relax,256)`——编号扫描前
                // 把目标**临时**绑成 \relax，编号中途出现 `\p` 即停扫不报
                // 未定义、也不展开旧含义（`\chardef\foo=123\foo`）。GT：
                // `\chardef\gX=12\the\gX` 报 "You can't use `\relax' after
                // \the." 且 \gX=120（ia4/th5 探针 2026-09-15，两引擎一致；
                // 旧注引 `\countdef\x=0\meaning\x` → "\relax" 有误，直接探针
                // 两引擎实为 `\count0`）。临时绑定不消费 \global（tex.web 两次
                // define() 读同一 global_defs）。
                self.set_slot_temp_relax(csid);
                let v = self.scan_number()?;
                let v = u32::try_from(v)
                    .map_err(|_| Error::invalid_input("\\chardef 字符码越界"))?;
                self.set_slot_scoped(csid, EqSlot::Char {
                    catcode: Catcode::Other,
                    charcode: v,
                });
                Ok(())
            }
            // ETRIP 冲刺：\countdef/\dimendef/\skipdef/\muskipdef/\toksdef\cs=<num>（cs 绑定寄存器）
            Primitive::Countdef
            | Primitive::Dimendef
            | Primitive::Skipdef
            | Primitive::Muskipdef
            | Primitive::Toksdef => {
                let csid = self.scan_cs_ident()?;
                // tex.web shorthand_def：编号扫描前临时绑 \relax（同 \chardef 臂注；
                // 不消费 \global）
                self.set_slot_temp_relax(csid);
                let idx = self.scan_number()?;
                // TeX：\countdef\cs=-1 / 32768 → "! Bad register code (-1)."
                // 恢复式（不定义、继续；ETRIP L970 稀疏数组测试的故意用例）。
                // 报错块对齐参考 etrip.log（l.970-973）：! 消息 + <to be read
                // again> + token + l.N 两行 + help 2 行。read-again token 经
                // fetch() 取输入流下一 token——宏体 `#1\1=-1#1\1=32768...`
                // 场景恰为再出现的 `\countdef` 等（与参考一致）；err_snapshot
                // 由 write_error 路径建立，恢复到主循环前校验组/条件栈仍生效。
                if idx < 0 || idx >= REGISTER_COUNT as i64 {
                    self.write_error_help(
                        &format!("Bad register code ({idx})."),
                        "A register number must be between 0 and 32767.\n\
                         I changed this one to zero.\n",
                    );
                    return Ok(());
                }
                let idx = idx as usize;
                let kind = match prim {
                    Primitive::Countdef => RegKind::Count,
                    Primitive::Dimendef => RegKind::Dimen,
                    Primitive::Skipdef => RegKind::Skip,
                    Primitive::Muskipdef => RegKind::Muskip,
                    _ => RegKind::Toks,
                };
                self.set_slot_scoped(csid, EqSlot::Register(kind, idx));
                Ok(())
            }
            // ETRIP 冲刺：\mathchardef\cs=<num>（cs 绑定数学字符码）
            Primitive::MathCharDef => self.exec_mathchardef(),
            // TRIP 冲刺：\everydisplay={<tokens>}（显示数学进入时注入）
            Primitive::EveryDisplay => {
                self.expect_equals()?;
                // tex.web assign_toks L22951：RHS 取 token 用 filler 语义
                // （get_x_token：可展开 filler 展开、跳 spacer/\relax）
                let tok = self
                    .fetch_non_filler()?
                    .ok_or_else(|| Error::invalid_input("everydisplay 缺少 RHS"))?;
                if tok.catcode() == Some(Catcode::BeginGroup) {
                    self.unread(tok);
                    let t = self.scan_group_contents(None)?;
                    self.set_every_scoped(5, t);
                } else if let Some(csid) = tok.csid() {
                    match self.eqtb.slot(csid).clone() {
                        EqSlot::Primitive(_) => {
                            let t = self.the_tokens_after(tok)?;
                            self.set_every_scoped(5, t);
                        }
                        EqSlot::Register(RegKind::Toks, idx) => {
                            let t = self.registers.toks(idx).to_vec();
                            self.set_every_scoped(5, t);
                        }
                        _ => {}
                    }
                }
                self.finish_assignment();
                Ok(())
            }
            // TRIP 冲刺：\spacefactor=<number>（活参数，实时经 sink 赋值；组恢复在排版器侧）
            Primitive::SpaceFactor => {
                let v = self.scan_number()?;
                self.sink.set_space_factor(v)
            }
            // TRIP 冲刺：\everymath={<tokens>}（进入数学模式时注入）
            Primitive::EveryMath => {
                self.expect_equals()?;
                let toks = self.scan_group_contents(Some("everymath"))?;
                self.set_every_scoped(6, toks);
                Ok(())
            }
            // TRIP 冲刺：toks 参数（\everypar/\everyhbox/\everyvbox/\everycr/\errhelp）
            // ——RHS 支持组内容或 toks 参数/寄存器引用（TRIP L140 `\everypar=\errhelp`）
            Primitive::EveryPar
            | Primitive::EveryHBox
            | Primitive::EveryVBox
            | Primitive::EveryCr
            | Primitive::ErrHelp => {
                self.expect_equals()?;
                // tex.web assign_toks L22951：RHS 取 token 用 filler 语义
                // （get_x_token：可展开 filler 展开、跳 spacer/\relax）
                let tok = self
                    .fetch_non_filler()?
                    .ok_or_else(|| Error::invalid_input("toks 参数缺少 RHS"))?;
                let toks = if tok.catcode() == Some(Catcode::BeginGroup) {
                    self.unread(tok);
                    self.scan_group_contents(None)?
                } else if let Some(csid) = tok.csid() {
                    match self.eqtb.slot(csid).clone() {
                        EqSlot::Primitive(_) => self.the_tokens_after(tok)?,
                        EqSlot::Register(RegKind::Toks, idx) => self.registers.toks(idx).to_vec(),
                        _ => Vec::new(),
                    }
                } else {
                    Vec::new()
                };
                match prim {
                    Primitive::EveryPar => self.set_every_scoped(0, toks),
                    Primitive::EveryHBox => self.set_every_scoped(1, toks),
                    Primitive::EveryVBox => self.set_every_scoped(2, toks),
                    Primitive::EveryCr => self.set_every_scoped(3, toks),
                    _ => self.set_every_scoped(4, toks),
                }
                self.finish_assignment();
                Ok(())
            }
            // ETRIP 冲刺：\everyjob=<tokens>（暂映射 toks 寄存器 0）
            Primitive::EveryJob => {
                self.expect_equals()?;
                let val = self.scan_group_contents(Some("everyjob"))?;
                self.assign_toks(0, Arc::from(val));
                Ok(())
            }
            // pdfTeX/e-TeX：\everyeof=<tokens>（文件帧读尽注入；expl3
            // \__sys_get 依赖）。RHS 语义同 \everydisplay（filler + 组/toks 引用）。
            Primitive::EveryEof => {
                self.expect_equals()?;
                let tok = self
                    .fetch_non_filler()?
                    .ok_or_else(|| Error::invalid_input("everyeof 缺少 RHS"))?;
                let toks = if tok.catcode() == Some(Catcode::BeginGroup) {
                    self.unread(tok);
                    self.scan_group_contents(None)?
                } else if let Some(csid) = tok.csid() {
                    match self.eqtb.slot(csid).clone() {
                        EqSlot::Primitive(_) => self.the_tokens_after(tok)?,
                        EqSlot::Register(RegKind::Toks, idx) => {
                            self.registers.toks(idx).to_vec()
                        }
                        _ => Vec::new(),
                    }
                } else {
                    Vec::new()
                };
                self.everyeof_toks = toks;
                self.finish_assignment();
                Ok(())
            }
            // ETRIP 冲刺：\parshape=<n> <indent> <width> ...（段落形状）
            Primitive::Parshape => self.exec_parshape(),
            // ETRIP 冲刺：\parshapelength/indent/dimen 主循环裸用 → 报
            // "You can't use `\parshapelength' in <mode>." 并恢复。恢复**不扫
            // 下标**（宿主 etex 实测：`\parshapelength 2` 报错后 `2` 照常排版
            // 入 hmode；此前报错后 scan_number 会吞掉下标/下一原语——l.701
            // `\parshapelength \parshapeindent \parshapedimen` 三连报错被吞成
            // 一处 Missing number，级联污染 l.702 的 `\def\1#1 {...}` 直到
            // parshape 段 `\edef\2` 撞 `\the` fatal）。下标读取由扫描臂承担
            // （\ifdim/\the/\dimexpr 等语境不走此分支）。
            Primitive::ParshapeLength
            | Primitive::ParshapeIndent
            | Primitive::ParshapeDimen => {
                let name = match prim {
                    Primitive::ParshapeIndent => "parshapeindent",
                    Primitive::ParshapeLength => "parshapelength",
                    _ => "parshapedimen",
                };
                self.write_error_help_no_read_again(
                    &format!("You can't use `\\{name}' in {}.", self.sink.mode_name()),
                    "Sorry, but I'm not programmed to handle this case;\n\
                     I'll just pretend that you didn't ask for it.\n\
                     If you're in the wrong mode, you might be able to\n\
                     return to the right one by typing `I}' or `I$' or `I\\par'.\n",
                );
                Ok(())
            }
            // ETRIP 冲刺：TeXXeT 方向原语 \beginL/\endL/\beginR/\endR。
            // \TeXXeTstate=1：创建方向节点；=0：TeX 报 "Improper \beginL." 后继续。
            Primitive::BeginL
            | Primitive::EndL
            | Primitive::BeginR
            | Primitive::EndR => {
                let state = self.params.misc[20]; // TeXXeTState（free.rs int_param_index 20）
                let name = match prim {
                    Primitive::BeginL => "beginL",
                    Primitive::EndL => "endL",
                    Primitive::BeginR => "beginR",
                    _ => "endR",
                };
                if state == 1 {
                    let kind = match prim {
                        Primitive::BeginL => DirectionKind::BeginL,
                        Primitive::EndL => DirectionKind::EndL,
                        Primitive::BeginR => DirectionKind::BeginR,
                        _ => DirectionKind::EndR,
                    };
                    self.sink.direction_node(kind)
                } else {
                    self.sink.write16(format!("! Improper \\{name}.\n"))
                }
            }
            // ETRIP 冲刺：\dump（initex 收尾）：标记 dumped 并结束作业（驱动负责写 fmt）。
            // M7 fmt 快路径：\dump 是 tex.web 的作业终结点（preserve_tail: end of
            // session）——dumped 后主循环立即停，dump 之后源里剩余 token 一律不执行
            //（此前清栈后继续读，\dump 后的探针语句引发错误链 + dumped 语义被搅）。
            Primitive::Dump => {
                self.dumped = true;
                self.ended = true;
                self.stack.clear();
                self.output_active = false;
                self.cond_stack.clear();
                self.flush_writes()?;
                Ok(())
            }
            // ETRIP 冲刺：\readline<n>to\cs（原始行读取）
            Primitive::ReadLine => self.exec_readline(),
            // ETRIP 第二波：盒子尺寸赋值（\wd/\ht/\dp<n>=<dimen>；无 '=' 时按 TeX 报错）
            Primitive::Wd | Primitive::Ht | Primitive::Dp => {
                let dim: u8 = match prim {
                    Primitive::Wd => 0,
                    Primitive::Ht => 1,
                    _ => 2,
                };
                let idx = self.scan_register_index()?;
                self.expect_equals()?;
                let v = self.scan_dimen()?;
                self.sink.set_box_dim(idx, dim, v)
            }
            other => Err(Error::internal(format!(
                "未接入 dispatch_toks_state 的原语 {other:?}"
            ))),
        }
    }
}

impl Expander {
    /// `\every*` 族字段访问（kind：0=everypar 1=everyhbox 2=everyvbox
    /// 3=everycr 4=errhelp 5=everydisplay 6=everymath）。
    pub(crate) fn every_field(&self, kind: u8) -> &Vec<Token> {
        match kind {
            0 => &self.everypar_toks,
            1 => &self.everyhbox_toks,
            2 => &self.everyvbox_toks,
            3 => &self.everycr_toks,
            4 => &self.errhelp_toks,
            5 => &self.everydisplay_toks,
            _ => &self.everymath,
        }
    }

    fn set_every_field(&mut self, kind: u8, v: Vec<Token>) {
        *match kind {
            0 => &mut self.everypar_toks,
            1 => &mut self.everyhbox_toks,
            2 => &mut self.everyvbox_toks,
            3 => &mut self.everycr_toks,
            4 => &mut self.errhelp_toks,
            5 => &mut self.everydisplay_toks,
            _ => &mut self.everymath,
        } = v;
    }

    /// `\every*` 族赋值的组作用域（tex.web：这些是 eqtb toks 槽
    /// `local_base+8` 起，非 `\global` 赋值随组结束恢复）。
    ///
    /// LaTeX 内核 `\@lign`（`\tabskip\z@skip\everycr{}`，注释即
    /// "restore inside \displ@y"）在每个对齐单元组内清空 `\everycr`，
    /// 依赖组结束恢复 `\displ@y` 的值；此前裸字段赋值永久生效。
    fn set_every_scoped(&mut self, kind: u8, v: Vec<Token>) {
        if !self.is_global() {
            let prev = self.every_field(kind).clone();
            self.save_stack
                .push((self.group_level, SavedValue::EveryToks { kind, prev }));
        }
        self.set_every_field(kind, v);
    }
}
