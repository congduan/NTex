// IO / 输出 / 盒子操作 / 诊断 原语 dispatcher。
//
// 主题：
// - IO 流（Input/OpenIn/CloseIn/NewRead/Read/NewWrite/OpenOut/CloseOut/Write/Immediate）
// - 输出（ShipOut/Output/Box/SetBox/VSplit）
// - 盒子操作（Copy/UnHBox/UnHCopy/UnVBox/UnVCopy/LastBox/UnSkip/UnPenalty/Unkern）
// - 页面只读内部量（DisplayWidth/PageDepth/PageFillLStretch/PageShrink/NullFont）
// - 诊断与显示（ShowBox/ShowGroups/ShowLists/Message/Show/ShowThe/ShowTokens/
//   ShowIfs/ErrMessage/Error/VarUnit）
//
// 维护约定（与 expand/mod.rs 既有 include! 链一致）：
// - 仅 `impl Expander { ... }` 块，无 use / 无模块声明；
// - 入口 `dispatch_io(prim: Primitive)` 由 `primitive.rs` 主 match 委托；
// - 函数 1:1 来自 `exec_primitive` 主 match，零行为变化。

impl Expander {
    /// IO / 输出 / 盒子操作 / 诊断原语 dispatcher。
    pub(super) fn dispatch_io(&mut self, prim: Primitive) -> Result<()> {
        match prim {
            // M3 收尾（RFC-3）：VFS 副作用原语
            Primitive::Input => self.exec_input(),
            Primitive::OpenIn => self.exec_openin(),
            Primitive::CloseIn => self.exec_closein(),
            Primitive::NewRead => self.exec_new_stream(StreamKind::Read),
            Primitive::Read => self.exec_read(),
            Primitive::NewWrite => self.exec_new_stream(StreamKind::Write),
            Primitive::OpenOut => self.exec_openout(),
            Primitive::CloseOut => self.exec_closeout(),
            Primitive::Write => self.exec_write(),
            Primitive::Immediate => {
                // tex.web `<Implement \immediate>`：立即取下一个 token；是
                // \write/\openout/\closeout → 设前缀后执行（立即写）；否则放回、
                // 前缀无效（不残留——旧实现残留标志导致 TRIP L2
                // `\immediate\catcode` 污染 L93 `\write-1` 被误判立即写，
                // "log file only" 提前输出）。
                self.immediate_pending = false;
                let Some((next, _)) = self.fetch()? else {
                    return Err(Error::invalid_input("\\immediate 后无 token"));
                };
                let is_write_family = next.csid().is_some_and(|c| {
                    matches!(
                        self.eqtb.slot(c),
                        EqSlot::Primitive(
                            Primitive::Write | Primitive::OpenOut | Primitive::CloseOut
                        )
                    )
                });
                if is_write_family {
                    self.immediate_pending = true;
                    self.process_token(next)
                } else {
                    self.unread(next);
                    Ok(())
                }
            }
            // M3-5 输出：\shipout 直通 sink（排版器解释：封装下一盒子为页面）
            Primitive::ShipOut => {
                // 记录触发行号（output 例程组 entering 行 = shipout 行）
                self.output_trigger_line = self.current_line_no();
                self.sink.primitive(prim)
            }
            // M3-5-3 输出例程：\output=<general text> 存储 token 列表。
            // RHS 与 toks 寄存器同族（tex.web `toks_register,assign_toks` 共用
            // 分支 L22945-22979）：`{token list}` 之外也接受另一 toks 寄存器/
            // `\output`（内容复制，`\output\pr@output`）——统一走 scan_toks_rhs。
            Primitive::Output => {
                self.expect_equals()?;
                let val = self.scan_toks_rhs()?;
                self.assign_output(val);
                Ok(())
            }
            // M3-5-3 盒子寄存器：\box<n> 交给 sink（shipout_next 时封装为页面）
            Primitive::Box => {
                let idx = self.scan_register_index()?;
                self.sink.box_register(idx)
            }
            // ETRIP 冲刺：\setbox<n>=<box>（盒子寄存器赋值：通知 sink 存入寄存器）
            // box 参数（\vbox{}/\box255 等）同 \moveleft：scan_box 扫描不追踪
            Primitive::SetBox => {
                let idx = self.scan_register_index()?;
                self.expect_equals()?;
                self.trace_suppress += 1;
                self.pending_box_arg = true;
                self.pending_box_arg_mode = self.sink.mode_code();
                let global = self.global_pending;
                self.global_pending = false;
                self.sink.setbox(idx, global)
            }
            // ETRIP 冲刺：\showbox<n>：显示盒子寄存器内容（sink 格式化到转录）
            Primitive::ShowBox => {
                let idx = self.scan_register_index()?;
                self.sink.showbox(idx)
            }
            // ETRIP 第二波：诊断原语（\showgroups/\showlists）
            Primitive::ShowGroups => self.sink.showgroups(),
            Primitive::ShowLists => self.sink.showlists(),
            // ETRIP 第二波：盒子复制/拆包原语（\copy/\unhbox/\unvbox/\unhcopy/\unvcopy/\lastbox）
            Primitive::Copy => {
                let idx = self.scan_register_index()?;
                self.sink.copy_box(idx)
            }
            Primitive::UnHBox | Primitive::UnHCopy => {
                let idx = self.scan_register_index()?;
                self.sink.unhbox(idx, prim == Primitive::UnHCopy)
            }
            Primitive::UnVBox | Primitive::UnVCopy => {
                let idx = self.scan_register_index()?;
                self.sink.unvbox(idx, prim == Primitive::UnVCopy)
            }
            Primitive::LastBox => self.sink.lastbox(),
            // ETRIP 第二波：列表尾操作（\unskip/\unpenalty）
            Primitive::UnSkip => self.sink.unskip(),
            Primitive::UnPenalty => self.sink.unpenalty(),
            // TRIP 冲刺：\unkern：移除当前列表尾的 kern 节点（TRIP L189）
            Primitive::Unkern => self.sink.unkern(),
            // ETRIP 冲刺：\vsplit<n> to/spread <dimen>（纵向拆分盒子寄存器）
            Primitive::VSplit => {
                let idx = self.scan_register_index()?;
                self.skip_spaces()?;
                let mut to = None;
                let mut spread = None;
                if let Some(kw) = self.scan_keyword(|w| w == "to" || w == "spread")? {
                    let d = self.scan_dimen()?;
                    if kw == "to" {
                        to = Some(d);
                    } else {
                        spread = Some(d);
                    }
                }
                self.sink.vsplit(idx, to, spread)
            }
            // TRIP 补全批次：页面只读内部量（\the 查询在 save.rs 返回 0）
            Primitive::DisplayWidth
            | Primitive::PageDepth
            | Primitive::PageFillLStretch
            | Primitive::PageShrink
            | Primitive::NullFont => Ok(()),
            // ETRIP 冲刺：终端转录
            Primitive::Message => self.exec_message(),
            Primitive::Show => self.exec_show(),
            Primitive::ShowThe => self.exec_showthe(),
            Primitive::ShowTokens => self.exec_showtokens(),
            // ETRIP 冲刺：\showifs（显示当前条件嵌套；诊断原语）
            Primitive::ShowIfs => self.exec_showifs(),
            // TRIP 冲刺：\error（plain.tex 宏：errmessage；TRIP 分支中不执行）
            Primitive::Error => self.sink.primitive(prim),
            // TRIP 冲刺：\varunit 用作字体单位（plain.tex 字体）；no-op 原语
            Primitive::VarUnit => Ok(()),
            other => Err(Error::internal(format!(
                "未接入 dispatch_io 的原语 {other:?}"
            ))),
        }
    }
}
