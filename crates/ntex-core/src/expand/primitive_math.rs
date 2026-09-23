// 数学原语 dispatcher。
//
// 主题：样式（DisplayStyle/...）、分式（Over/Atop/Above*/OverWithDelims/AtopWithDelims）、
// 分组（Left/Right/Middle）、定类原子（MathOrd/.../MathInner/Nonscript）、
// 上下限标志（Limits/NoLimits/DisplayLimits/NoBoundary）、Mu 上下文（MSkip/MKern）、
// 完整原子（MathChar/Delimiter/MathAccent）、公式编号（EqNo/LeqNo）、读音符（Accent）、
// 根式（Radical/Sqrt/VCenter）、盒子位移（MoveLeft/MoveRight/Raise/Lower）、
// 上下标包装（Underline/Overline）。
//
// 维护约定（与 expand/mod.rs 既有 include! 链一致）：
// - 仅 `impl Expander { ... }` 块，无 use / 无模块声明；
// - 入口 `dispatch_math(prim: Primitive)` 由 `primitive.rs` 主 match 委托；
// - 函数 1:1 来自 `exec_primitive` 主 match，零行为变化。

impl Expander {
    /// tex.web scan_math 尾部（L21906-21908）：var 类（class ≥ var_code =
    /// 0x7000）且 cur_fam ∈ 0..15 时，数学字符码的 fam 字段被 cur_fam
    /// **无条件替换**，否则保留码内原值（`(c div 256) mod 16`）。
    /// cur_fam 即内部整数参数 46（`free::int_param_index(\fam)`；数学进入时
    /// 复位 -1，见 param.rs 默认表），letter/other 隐式 mathcode 与
    /// `\mathchar` 显式码共用这一裁决。
    pub(super) fn mathcode_apply_fam(&self, code: u32) -> u32 {
        let cur_fam = self.params.misc[46];
        if (0..16).contains(&cur_fam) {
            (code & !0x0F00) | ((cur_fam as u32) << 8)
        } else {
            code
        }
    }

    /// 数学原语 dispatcher。
    pub(super) fn dispatch_math(&mut self, prim: Primitive) -> Result<()> {
        match prim {
            // M4-2 数学原语：直通 sink（排版器解释；delimiter 参数在 VM 侧扫描）
            Primitive::DisplayStyle => self.sink.math_style(0),
            Primitive::TextStyle => self.sink.math_style(1),
            Primitive::ScriptStyle => self.sink.math_style(2),
            Primitive::ScriptScriptStyle => self.sink.math_style(3),
            Primitive::Over => self.sink.math_fraction(None),
            Primitive::Atop => self.sink.math_fraction(Some(0)),
            Primitive::Left => {
                let d = self.scan_delimiter()?;
                // math left group（16）：\left 打开真实组（tex.web new_math_left_group；
                // \currentgrouptype 检查 + \tracinggroups 显示）。数学层（math_left/
                // math_right 的 Delimited 物化）由 layout 侧另行维护，互不干扰。
                self.sink.math_left_begin()?;
                self.begin_group()?;
                self.math_left_depth += 1;
                self.sink.math_left(d)
            }
            Primitive::Right => {
                let d = self.scan_delimiter()?;
                // 配对 \\left 存在才关组（\\right 前缺 \\left → TeX 报
                // "! Extra \\right." 恢复并丢弃——trip L256 `$\\right\\relax`，
                // 参考 log 双错误：Missing delimiter (. inserted) + Extra \\right.；
                // 不报 Error 中断（引擎契约：畸形输入不 panic））。
                if self.math_left_depth > 0 {
                    self.math_left_depth -= 1;
                    self.end_group()?;
                    self.sink.math_right(d)
                } else {
                    self.write_error("Extra \\right.");
                    Ok(())
                }
            }
            // e-TeX（M4-5）：\middle<delimiter>（\left...\right 内分隔符）
            Primitive::Middle => {
                let d = self.scan_delimiter()?;
                // \middle 关闭当前 \left 组并开新组（参考 log：leaving entered at
                // 前一行 + entering 本行成对；组类型仍为 math left group 16）
                if self.math_left_depth > 0 {
                    self.math_left_depth -= 1;
                    self.end_group()?;
                }
                self.sink.math_left_begin()?;
                self.begin_group()?;
                self.math_left_depth += 1;
                self.sink.math_middle(d)
            }
            // tex.web L20859-20861：hmove/vmove 扫描 dimen 后按 chr_code 取正负
            // 传入 scan_box（`if t=0 then scan_box(cur_val) else scan_box(-cur_val)`）。
            // box_context 最终整体写入 shift_amount（L20894），showbox 显示该值。
            // 参考实测（trip.log L401）：\lower2pt → shifted 2.0（正值，非直觉的 -2）。
            Primitive::Raise | Primitive::Lower => {
                let amount = self.scan_dimen()?;
                // lower(chr=0) → +amount；raise(chr=1) → -amount（tex.web t=0/1 规则）
                let amount = if prim == Primitive::Raise {
                    -amount
                } else {
                    amount
                };
                self.sink.raise(amount)
            }
            // TRIP 冲刺：\moveleft/\moveright<dimen><box>（盒子水平位移）
            // tex.web L20859-20861 符号规则：moveleft(chr=1)→+cur_val、
            // moveright(chr=0)→-cur_val（box_context 直写 shift_amount；
            // trip.log 参考实测 \moveleft20pt\copy200 → shifted -20.0，
            // \moveright20pt\hbox → shifted 20.0——hmove 的位移最终以
            // "盒内内容的坐标偏移"形式表现，与 vmove 方向约定相反）。
            // tex.web：\moveleft/\moveright 仅垂直模式合法（vmode），数学模式
            // 报 "You can't use ..." 且**不扫参数**（l.395 误报修复：此前先 scan_dimen
            // 导致 \lastbox 被当尺寸报 Missing number）。水平模式（2/5）暂不拦截——
            // NTex 的 \vskip 尚未实现"水平模式隐式结束段落"（l.316 场景），
            // 拦截会暴露该系统性缺口，待 \vskip 修好后再补。
            Primitive::MoveLeft | Primitive::MoveRight => {
                let mode = self.sink.mode_code();
                if matches!(mode, 3 | 6) {
                    let name = if prim == Primitive::MoveLeft {
                        "moveleft"
                    } else {
                        "moveright"
                    };
                    let what = "math mode";
                    self.write_error(&format!("You can't use `\\{name}' in {what}."));
                    let _ = self.sink.write16(
                        "Sorry, but I'm not programmed to handle this case;\n\
                         I'll just pretend that you didn't ask for it.\n\
                         If you're in the wrong mode, you might be able to\n\
                         return to the right one by typing `I}' or `I$' or `I\\par'.\n"
                            .to_string(),
                    );
                    return Ok(());
                }
                let d = self.scan_dimen()?;
                // tex.web scan_box：box 参数 token 不追踪（\moveleft20pt\copy200 的
                // {\copy}、\moveright20pt\hbox{ 的 {\hbox}/{ 均不输出）
                self.trace_suppress += 1;
                self.pending_box_arg = true;
                self.pending_box_arg_mode = self.sink.mode_code();
                if prim == Primitive::MoveLeft {
                    self.sink.move_left(-d)
                } else {
                    self.sink.move_right(d)
                }
            }
            // TRIP 冲刺：\\accent（读音符）。TeX 在数学模式先报错且**继续**扫描
            // <15-bit number> 与 nucleus 字段（tex.web math_ac；TRIP L396
            // `\\accent\\x\\vfill` 报错改道后还要求 `{`），故数字预扫描必须保留。
            // 顺序对齐 tex.web math_ac：先 Complain（sink 报 Please use）再
            // scan_fifteen_bit_int——`\\x` 必须被数字扫描消费（参考 log 报完
            // Please use 后 <to be read again> 是 \\vfill 而非 \\x）。
            Primitive::Accent => {
                self.sink.math_accent(true)?;
                let _ = self.scan_number()?;
                Ok(())
            }
            Primitive::Sqrt => self.sink.math_sqrt(),
            // \vcenter<box>：数学垂直居中盒（tex.web mmode+vcenter L22112：
            // scan_spec(vcenter_group) 后进内层竖模式收集，组尾 vpack 成盒并
            // 作为 vcenter_noad 的核——mlist_to_hlist 的 make_vcenter（L14455）
            // 按数学轴重分 height/depth）。此处保留 VCenter 身份发给排版层；
            // 收集通路与 \vbox 同构（排版层 PendingBox::VCenter 复用 vbox 组）。
            Primitive::VCenter => {
                let (to, spread) = self.scan_box_spec()?;
                self.sink.box_spec(to, spread)?;
                self.sink.primitive(Primitive::VCenter)
            }
            // TeX math_comp（tex.web L22019）：\mathord 等“定类原语”之后必有
            // <math field>；非 `{` 的 token 经 scan_math → scan_left_brace 报
            // "Missing { inserted" 放回重扫（TRIP L272 `\mathord \radical`）。
            Primitive::MathOrd => self.sink.math_class(0),
            Primitive::MathBin => self.sink.math_class(1),
            Primitive::MathOp => self.sink.math_class(2),
            Primitive::MathRel => self.sink.math_class(3),
            Primitive::MathOpen => self.sink.math_class(4),
            Primitive::MathClose => self.sink.math_class(5),
            Primitive::MathPunct => self.sink.math_class(6),
            Primitive::MathInner => self.sink.math_class(7),
            Primitive::Nonscript => self.sink.primitive(prim),
            // TRIP 冲刺：\limits/\nolimits/\displaylimits（\mathop 后置上下限标志；
            // 布局层数学原子按 `\mathop` 标志处理；单独出现时 TeX 报
            // "Limit controls must follow a math operator"——差异待 etrip.log 校准）
            Primitive::Limits | Primitive::NoLimits | Primitive::DisplayLimits => {
                self.sink.primitive(prim)
            }
            // TRIP 冲刺：\noboundary（数学字符边界抑制；水平/垂直模式 no-op）
            Primitive::NoBoundary => self.sink.primitive(prim),
            // TRIP 冲刺：数学原语（存在性测试；简化实现"消费参数"——
            // 数学列表节点由排版器处理，expander 侧跳过）。
            // tex.web：\mskip/\mkern 是 **mu 上下文**（scan_glue_mu/scan_dimen_mu，
            // 只认 "mu" 单位）——用 pt 上下文扫描会把 `\mkern-9mu` 报 Illegal
            // unit（8d0a71c muskip 参数化后暴露；trip L262/L275）。
            Primitive::MSkip => {
                let _ = self.scan_glue_mu()?;
                Ok(())
            }
            Primitive::MKern => {
                let _ = self.scan_dimen_mu()?;
                Ok(())
            }
            Primitive::MathAccent => {
                let _ = self.scan_number()?;
                self.sink.math_accent(false)?;
                Ok(())
            }
            // tex.web math_char：\mathchar<15-bit> 是完整数学字符原子（数学中
            // 直接产出 Char——此前 scan_number 即丢）；\delimiter<27-bit> 为
            // 定界符（Delimited 场景经 \left/\right 扫描；裸用暂简化）
            Primitive::MathChar => {
                // tex.web math_char_num 分支与 letter/other 同流到 scan_math 尾部：
                // 15 位码同样吃 `\fam` 替换（`\fam0 \mathchar"7101 x` → fam0 正体）
                let n = self.scan_number()? as u32;
                let n = if n >= 0x7000 { self.mathcode_apply_fam(n) } else { n };
                self.sink.math_char_full(n)
            }
            Primitive::Delimiter => {
                let _ = self.scan_number()?;
                Ok(())
            }
            // \eqno/\leqno：显示数学内是公式编号分隔符 no-op；非数学模式 TeX
            // 报错 + pretend 恢复（trip l.254 `\eqno` horizontal 报错对齐参考）
            Primitive::EqNo | Primitive::LeqNo => {
                let mode = self.sink.mode_code();
                if matches!(mode, 3 | 6) {
                    Ok(())
                } else {
                    // tex.web print_mode：\halign 行 mode=hmode（正）报
                    // "horizontal mode"（trip l.254 参考）；\hbox 内容
                    // mode=-hmode 才报 "restricted horizontal mode"
                    let what = if !self.align_frames.is_empty()
                        && matches!(self.sink.mode_code(), 2 | 5)
                    {
                        "horizontal mode".to_string()
                    } else {
                        self.sink.mode_name()
                    };
                    self.write_error(&format!(
                        "You can't use \\{} in {what}.",
                        if matches!(prim, Primitive::EqNo) {
                            "eqno"
                        } else {
                            "leqno"
                        }
                    ));
                    let _ = self.sink.write16(
                        "Sorry, but I'm not programmed to handle this case;\n\
                         I'll just pretend that you didn't ask for it.\n\
                         If you're in the wrong mode, you might be able to\n\
                         return to the right one by typing `I}' or `I$' or `I\\par'.\n"
                            .to_string(),
                    );
                    Ok(())
                }
            }
            // tex.web math_fraction：\abovewithdelims<delim1><delim2><dimen>——
            // 先扫两个定界符再扫厚度（TRIP l.257 漏报修复；原顺序 dimen 在前
            // 导致参数错位）。**必须调 sink.math_fraction**（\over/\atop 已有；
            // 此前 4 个 withdelims/above 只扫参数不挂 fraction → 分子分母被
            // 混收当前层，l.276 \abovewithdelims(.2pt 后数学状态崩）。
            Primitive::AboveWithDelims => {
                let _ = self.scan_delimiter()?;
                let _ = self.scan_delimiter()?;
                let thickness = self.scan_dimen()?;
                self.sink.math_fraction(Some(thickness))
            }
            // TRIP 冲刺：\above<dimen>（分数）与 \atopwithdelims<delim><delim>（带定界分数）
            Primitive::Above => {
                let thickness = self.scan_dimen()?;
                self.sink.math_fraction(Some(thickness))
            }
            Primitive::AtopWithDelims => {
                let _ = self.scan_delimiter()?;
                let _ = self.scan_delimiter()?;
                self.sink.math_fraction(Some(0))
            }
            Primitive::OverWithDelims => {
                let _ = self.scan_delimiter()?;
                let _ = self.scan_delimiter()?;
                self.sink.math_fraction(None)
            }
            // tex.web math_ac：\underline/\overline 是数学前缀——字段经主循环组
            // 机制收集（sink pending → 组开 field → 组关原子）；此前 scan_group_contents
            // 收集即丢（KNOWN-SIMPLIFICATIONS §1——内容不进数学列表）
            Primitive::Underline => self.sink.math_underline(),
            Primitive::Overline => self.sink.math_overline(),
            // TRIP 冲刺：\radical<delimiter><math field>（根式原子，\sqrt 底层）
            Primitive::Radical => {
                let delim = self.scan_delimiter()?;
                self.sink.math_radical(delim)
            }
            other => Err(Error::internal(format!(
                "未接入 dispatch_math 的原语 {other:?}"
            ))),
        }
    }
}
