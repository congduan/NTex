// 可展开原语 dispatcher。
//
// 主题：表达式原语（NumExpr/Dimexpr/Glueexpr/Muexpr）+ 整数与版本
// （Number/ETeXVersion/ETeXRevision）+ 文本转换（String_/Meaning/JobName/
// Csname/EndCsname）+ e-TeX 控制（Protected/Unless/Scantokens/Detokenize/
// Unexpanded）+ 只读整数与胶水分量（InputLineNo/CurrentGroupLevel/.../
// PageTotal/.../GlueStretchOrder/.../LastPenalty/InsertPenalties）+ Mu↔Glue
// 转换（MuToGlue/GlueToMu）+ 报错（ErrMessage）。
//
// 维护约定（与 expand/mod.rs 既有 include! 链一致）：
// - 仅 `impl Expander { ... }` 块，无 use / 无模块声明；
// - 入口 `dispatch_expandable(prim: Primitive)` 由 `primitive.rs` 主 match 委托；
// - 函数 1:1 来自 `exec_primitive` 主 match，零行为变化。

impl Expander {
    /// 可展开原语 dispatcher。
    pub(super) fn dispatch_expandable(&mut self, prim: Primitive) -> Result<()> {
        match prim {
            Primitive::NumExpr => {
                // 裸用（`\numexpr \dimexpr ...` 错误用例）：TeX 报错并恢复
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\numexpr 后缺少参数"))?
                    .0;
                if !self.expr_start_ok(ExprKind::Num, t)? {
                    self.unread(t);
                    self.report_error("You can't use \\numexpr in vertical mode.");
                    return Ok(());
                }
                self.unread(t);
                let v = self.eval_int_expression()?;
                self.emit_tokens(emit_count(v))
            }
            // M4-5 e-TeX 扩展：\dimexpr/\glueexpr 可展开求值（\the 上下文由 the_tokens 直接读取）
            Primitive::Dimexpr => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\dimexpr 后缺少参数"))?
                    .0;
                if !self.expr_start_ok(ExprKind::Dim, t)? {
                    self.unread(t);
                    self.report_error("You can't use \\dimexpr in vertical mode.");
                    return Ok(());
                }
                self.unread(t);
                let v = self.eval_dimen_expression()?;
                self.emit_tokens(emit_dimen(v))
            }
            Primitive::Glueexpr => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\glueexpr 后缺少参数"))?
                    .0;
                if !self.expr_start_ok(ExprKind::Glue, t)? {
                    self.unread(t);
                    self.report_error("You can't use \\glueexpr in vertical mode.");
                    return Ok(());
                }
                self.unread(t);
                let g = self.eval_glue_expression(false)?;
                self.emit_tokens(emit_glue(g))
            }
            Primitive::Muexpr => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\muexpr 后缺少参数"))?
                    .0;
                if !self.expr_start_ok(ExprKind::Mu, t)? {
                    self.unread(t);
                    self.report_error("You can't use \\muexpr in vertical mode.");
                    return Ok(());
                }
                self.unread(t);
                let g = self.eval_glue_expression(true)?;
                self.emit_tokens(emit_glue(g))
            }
            // \csname...\endcsname：构造控制序列（TeX 可展开原语）
            Primitive::Csname => self.exec_csname(),
            Primitive::EndCsname => {
                self.report_error("Extra \\endcsname.");
                Ok(())
            }
            // \number<number>：整数十进制展开（TeX 可展开原语）
            Primitive::Number => {
                let v = self.scan_number()?;
                self.emit_tokens(emit_count(v))
            }
            // 内部量：\eTeXversion/\eTeXrevision 可展开（\the 上下文由 the_tokens 读取）
            Primitive::ETeXVersion => self.emit_tokens(emit_count(2)),
            Primitive::ETeXRevision => {
                // e-TeX 2.6：revision 展开为 ".6"（版本号"2.6"的后半段，前导点用于 etrip.tex
                // 的 `\def\1.#1#2\relax` 分隔参数解析）
                self.emit_tokens(
                    ".6"
                        .bytes()
                        .map(|b| Token::char(Catcode::Other, u32::from(b)))
                        .collect(),
                )
            }
            // \string<token>：token 转文本（字符序列；TeX 可展开原语）
            Primitive::String_ => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\string 后无 token"))?
                    .0;
                let mut buf = Vec::new();
                detokenize_token(t, &self.intern, &mut buf);
                self.emit_tokens(buf)
            }
            // \inputlineno 单独出现：no-op（恒 0；数字上下文由 scan_number 处理）
            Primitive::InputLineNo => Ok(()),
            // e-TeX 只读整数单独出现：no-op（数字上下文由 scan_number 读取）
            Primitive::CurrentGroupLevel
            | Primitive::CurrentGroupType
            | Primitive::LastNodeType
            | Primitive::CurrentIfLevel
            | Primitive::CurrentIfType
            | Primitive::CurrentIfBranch => Ok(()),
            // TRIP 冲刺：页面 dimen 内部量（\pagetotal/\pagegoal/\predisplaysize
            // 只读——expander 无排版状态，单独出现无操作；\the 查询在 save.rs 返回 0）
            Primitive::PageTotal | Primitive::PageGoal | Primitive::PreDisplaySize => Ok(()),
            // ETRIP 冲刺：\errmessage{...} 报错到转录（plain.tex \error 宏的底层原语）
            Primitive::ErrMessage => {
                let msg = self.scan_group_contents(None)?;
                let text: String = msg
                    .iter()
                    .filter_map(|t| t.charcode().and_then(char::from_u32))
                    .collect();
                self.report_error(&format!("{text}."));
                Ok(())
            }
            // ETRIP 冲刺：\meaning<token>（可展开：token 含义文本）
            Primitive::Meaning => self.exec_meaning(),
            // e-TeX 胶水分量查询原语裸用（\gluestretchorder/\glueshrinkorder/
            // \gluestretch/\glueshrink）：TeX 报 "You can't use `\x' in <mode>
            // mode." 并恢复、不扫参数——etrip gluestretchorder 段 l.932/l.933
            // 各 2 个模式错（垂直模式裸用）。数值/\the/\ifnum 上下文由 scan.rs
            // /save.rs 先行处理，不达此分支；报错无 `<to be read again>` 段。
            Primitive::GlueStretchOrder
            | Primitive::GlueShrinkOrder
            | Primitive::GlueStretch
            | Primitive::GlueShrink => {
                let name = match prim {
                    Primitive::GlueStretchOrder => "gluestretchorder",
                    Primitive::GlueShrinkOrder => "glueshrinkorder",
                    Primitive::GlueStretch => "gluestretch",
                    _ => "glueshrink",
                };
                self.write_error_help_no_read_again(
                    &format!(
                        "You can't use `\\{name}' in {}.", self.sink.mode_name()
                    ),
                    "Sorry, but I'm not programmed to handle this case;\n\
                     I'll just pretend that you didn't ask for it.\n\
                     If you're in the wrong mode, you might be able to\n\
                     return to the right one by typing `I}' or `I$' or `I\\par'.\n",
                );
                Ok(())
            }
            // ETRIP 第二波：\mutoglue/\gluetomu 裸用（非胶水/数字上下文）——
            // TeX 报 "You can't use `\mutoglue' in vertical mode." 并恢复
            // （etrip L905：不扫参数；此前简化实现直接 scan 参数会吞掉后续
            // 输入，污染下一行 \skip1= 等赋值）。胶水上下文由 scan_glue_inner
            // 前导量分支先行处理，不达此分支。
            Primitive::MuToGlue | Primitive::GlueToMu => {
                let name = if matches!(prim, Primitive::MuToGlue) {
                    "mutoglue"
                } else {
                    "gluetomu"
                };
                self.write_error_help_no_read_again(
                    &format!(
                        "You can't use `\\{name}' in {}.",
                        self.sink.mode_name()
                    ),
                    "Sorry, but I'm not programmed to handle this case;\n\
                     I'll just pretend that you didn't ask for it.\n\
                     If you're in the wrong mode, you might be able to\n\
                     return to the right one by typing `I}' or `I$' or `I\\par'.\n",
                );
                Ok(())
            }
            // ETRIP 第二波：\lastpenalty 单独出现：no-op（数字上下文由 scan_number 读取）
            Primitive::LastPenalty => Ok(()),
            // ETRIP 冲刺：\jobname：作业名（当前无名字来源，恒 "texput"）
            Primitive::JobName => self.emit_tokens(
                "texput"
                    .bytes()
                    .map(|b| Token::char(Catcode::Other, u32::from(b)))
                    .collect(),
            ),
            // M4-5 e-TeX 展开扩展
            Primitive::Protected => {
                self.protected_pending = true;
                Ok(())
            }
            Primitive::Unless => {
                self.unless_pending = true;
                Ok(())
            }
            Primitive::Scantokens => self.exec_scantokens(),
            Primitive::Detokenize => self.exec_detokenize(),
            Primitive::Unexpanded => self.exec_unexpanded(),
            // TRIP 冲刺：\insertpenalties（int 只读——数字上下文由 scan_number 读取）
            Primitive::InsertPenalties => Ok(()),
            other => Err(Error::internal(format!(
                "未接入 dispatch_expandable 的原语 {other:?}"
            ))),
        }
    }
}
