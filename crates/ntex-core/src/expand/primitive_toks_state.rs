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
                let idx = self.scan_number()?;
                // TeX：\countdef\cs=-1 / 32768 → "! Bad register code (-1)."
                // 恢复式（不定义、继续；ETRIP L970 稀疏数组测试的故意用例）。
                if idx < 0 || idx >= REGISTER_COUNT as i64 {
                    self.report_error(&format!("Bad register code ({idx})."));
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
                self.skip_spaces()?;
                let (tok, _) = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("everydisplay 缺少 RHS"))?;
                if tok.catcode() == Some(Catcode::BeginGroup) {
                    self.unread(tok);
                    self.everydisplay_toks = self.scan_group_contents(None)?;
                } else if let Some(csid) = tok.csid() {
                    match self.eqtb.slot(csid).clone() {
                        EqSlot::Primitive(_) => {
                            self.everydisplay_toks = self.the_tokens_after(tok)?
                        }
                        EqSlot::Register(RegKind::Toks, idx) => {
                            self.everydisplay_toks = self.registers.toks(idx).to_vec()
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
                self.everymath = toks;
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
                self.skip_spaces()?;
                let (tok, _) = self
                    .fetch()?
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
                    Primitive::EveryPar => self.everypar_toks = toks,
                    Primitive::EveryHBox => self.everyhbox_toks = toks,
                    Primitive::EveryVBox => self.everyvbox_toks = toks,
                    Primitive::EveryCr => self.everycr_toks = toks,
                    _ => self.errhelp_toks = toks,
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
            // ETRIP 冲刺：\parshape=<n> <indent> <width> ...（段落形状）
            Primitive::Parshape => self.exec_parshape(),
            // ETRIP 冲刺：\parshapelength/indent/dimen 单独出现（无索引）→
            // TeX 报 "can't use" 并恢复
            Primitive::ParshapeLength
            | Primitive::ParshapeIndent
            | Primitive::ParshapeDimen => {
                self.report_error("You can't use \\parshape... in vertical mode.");
                let _ = self.scan_number();
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
            // ETRIP 冲刺：\dump（initex 收尾）：标记 dumped 并结束作业（驱动负责写 fmt）
            Primitive::Dump => {
                self.dumped = true;
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
