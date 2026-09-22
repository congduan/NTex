// 排版原语 dispatcher。
//
// 主题：盒子（HBox/VBox/VTop/Par）、胶水（HSkip/VSkip/HFil/HFill/HSS/VFil/VFill/VSS/
// HFilNeg/VFilNeg）、kern、penalty、rule、leaders、control space、indent。
//
// 维护约定（与 expand/mod.rs 既有 include! 链一致）：
// - 仅 `impl Expander { ... }` 块，无 use / 无模块声明；
// - 入口 `dispatch_box(prim: Primitive)` 由 `primitive.rs` 主 match 委托；
// - 函数 1:1 来自 `exec_primitive` 主 match，零行为变化。

impl Expander {
    /// 排版原语 dispatcher：盒子/胶水/kern/penalty/rule/leaders/control space/inf glue。
    pub(super) fn dispatch_box(&mut self, prim: Primitive) -> Result<()> {
        match prim {
            // M3-2 排版原语
            // 盒子：扫描可选 to/spread 规格，直通 sink（排版器解释）。
            Primitive::HBox | Primitive::VBox | Primitive::VTop => {
                let (to, spread) = self.scan_box_spec()?;
                self.sink.box_spec(to, spread)?;
                self.sink.primitive(prim)
            }
            Primitive::Par => {
                // 传 \par 源码行号给排版器（折行警告 `at lines a--b`）
                let ln = self.error_context().map(|(n, _)| n as i64).unwrap_or(0);
                self.sink.paragraph_line(ln)?;
                self.sink.primitive(prim)
            }
            // 带参数扫描的排版原语：扫描在 VM 侧完成，结果交给 sink
            Primitive::HSkip | Primitive::VSkip => {
                let g = self.scan_glue()?;
                if prim == Primitive::VSkip {
                    self.vertical_command_implicit_par()?;
                }
                self.sink.glue(g)
            }
            Primitive::Kern => {
                let w = self.scan_dimen()?;
                self.sink.kern(w)
            }
            Primitive::Penalty => {
                // 记录行号须在 scan_number 之前（数字扫描吞行尾换行 →
                // current_line_no 已前进到下一行）
                let line = self.current_line_no();
                let p = self.scan_number()?;
                // 强制断页（≤ -10000）触发 output 例程：例程组的 entering
                // 行 = 断页行（tex.web：例程在断页 token 处注入）
                if p <= -10_000 {
                    self.output_trigger_line = line;
                }
                self.sink.penalty(p)
            }
            Primitive::HRule | Primitive::VRule => {
                // tex.web L21160/L21162：`\hrule` 只在垂直模式直接落 vlist；
                // 主水平模式经 head_for_vmode → end_graf 先隐式结束段落（规则
                // 落回 vlist，而非塞进行盒内部——此前缺这一步，`A\n\hrule B`
                // 的规则被追加进 B 段的行盒）。`\vrule` 任意模式合法（行内
                // 竖线，L20519 hmode+vrule 臂）。
                if matches!(prim, Primitive::HRule) {
                    self.vertical_command_implicit_par()?;
                }
                let [h, d, w] = self.scan_rule_specs(prim)?;
                self.sink.rule(w, h, d)
            }
            // TRIP 冲刺：\leaders/\cleaders/\xleaders —— 后续盒子（\hbox/\vbox/\hrule/
            // 盒子寄存器）由既有路径扫描，sink 侧挂起为引导符，等 \hskip/\vskip 胶水
            // 组成 Leader 节点（tex.web scan_box(leader_flag+kind) + box_end）。
            Primitive::Leaders | Primitive::Cleaders | Primitive::XLeaders => {
                self.sink.primitive(prim)
            }
            // 段落缩进：直通 sink 由排版器解释
            Primitive::Indent | Primitive::NoIndent => self.sink.primitive(prim),
            // \␣（ex_space）：tex.web 独立命令码（cmd=ex_space≠spacer），不产空格
            // token——此前发 cat10 字符 token 后与真 spacer 在扫描层（数字尾可选
            // 空格站）不可区分，排版层也只能按 spacefactor 折算。直通原语事件，
            // 由排版器按 tex.web 分模式处理（hmode/mmode→append_normal_space、
            // vmode→new_graf 起段）。
            Primitive::ControlSpace => self.sink.primitive(prim),
            // ETRIP 冲刺：无限阶胶水（\hfil/\hfill/\hss/\vfil/\vfill/\vss）
            Primitive::HFil => self.sink.fill_glue(0),
            Primitive::HFill => self.sink.fill_glue(1),
            Primitive::HSS => self.sink.fill_glue(2),
            Primitive::VFil => {
                self.vertical_command_implicit_par()?;
                self.sink.fill_glue(3)
            }
            Primitive::VFill => {
                self.vertical_command_implicit_par()?;
                self.sink.fill_glue(4)
            }
            Primitive::VSS => {
                self.vertical_command_implicit_par()?;
                self.sink.fill_glue(5)
            }
            // TRIP 冲刺：\vfilneg（plain.tex：负 1fil vskip；走 fill_glue kind=6）
            Primitive::VFilNeg => {
                self.vertical_command_implicit_par()?;
                self.sink.fill_glue(6)
            }
            // TRIP 冲刺：\hfilneg（plain.tex：负 1fil hskip；走 fill_glue kind=7）
            Primitive::HFilNeg => self.sink.fill_glue(7),
            // TRIP 冲刺：\/（斜体校正，直通 sink）
            Primitive::ItalicCorrection => self.sink.italic_correction(),
            other => Err(Error::internal(format!(
                "未接入 dispatch_box 的原语 {other:?}"
            ))),
        }
    }

    /// tex.web main_control：垂直命令（\vskip/\vfil/\vfill/\vss/\vfilneg）在
    /// （非受限）水平模式 → 先 end_graf（隐式 \par）再 back_input 重执行。
    ///
    /// demo 差异 #4：`{\boldfont 2. Bulleted Lists}\medskip` 中 \medskip 的
    /// \vskip 直接 append 进水平列表 → 标题行未断、后段并轨同基线。VM 侧
    /// 等效实现：hmode（mode_code 2）先发 \par 事件。受限水平（5）/数学（3/6）
    /// 的报错路径不在本次范围（TRIP 后续）；垂直模式无操作。
    fn vertical_command_implicit_par(&mut self) -> Result<()> {
        if self.sink.mode_code() == 2 {
            let ln = self.error_context().map(|(n, _)| n as i64).unwrap_or(0);
            self.sink.paragraph_line(ln)?;
            self.sink.primitive(Primitive::Par)?;
        }
        Ok(())
    }
}
