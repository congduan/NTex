//! 输出 sink：VM 与排版器的边界（M3-2）。
//!
//! ntex-core 的展开引擎是纯 token 级 VM（token 进 → token 出）。排版器
//! （ntex-layout 的 typesetter）需要结构化的"排版事件"而非裸 token 流：
//!
//! - [`TokenSink::token`]：普通输出 token（字符、未处理的控制序列）；
//! - [`TokenSink::group_begin`] / [`TokenSink::group_end`]：组定界事件
//!   （`{`/`}`）。VM 内部仍执行组作用域簿记（赋值回滚、`\aftergroup`），
//!   排版器据此构建盒子内容；
//! - [`TokenSink::glue`] / [`TokenSink::kern`] / [`TokenSink::penalty`] /
//!   [`TokenSink::rule`]：排版原语的结果（参数扫描在 VM 侧完成，sink 只收结果）；
//! - [`TokenSink::primitive`]：直通原语（`\hbox`/`\vbox`/`\vtop`/`\par` 等），
//!   VM 不做排版语义，交由 sink 解释。
//!
//! 默认实现 [`VecSink`] 收集 token，行为与 M1/M2 一致（`Expander::output`）。

use crate::eqtb::Primitive;
use crate::error::Result;
use crate::param::{ParamKind, ParamValue};
use crate::register::Glue;
use crate::token::Token;

/// TeXXeT 方向节点种类（e-TeX `\beginL`/`\endL`/`\beginR`/`\endR`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionKind {
    BeginL,
    EndL,
    BeginR,
    EndR,
}

/// VM 排版事件消费者。
///
/// 要求 `Debug`（`Expander` 派生 Debug）。`\edef` 区域需要取回临时收集的
/// token，通过 [`TokenSink::take_tokens`]（默认 None，由 [`VecSink`] 实现）。
///
/// 事件方法返回 `Result`：排版器（ntex-layout）可拒绝非法输入（如
/// `\par` 出现在受限水平模式、盒子组未闭合）。
pub trait TokenSink: std::fmt::Debug {
    /// 输出 token（字符、未处理的控制序列等）。
    fn token(&mut self, tok: Token) -> Result<()>;
    /// 数学移位（`$`，cat 3；M4-1）：进出数学模式。
    /// `display`：VM 检测到连续 `$$`（进入显示数学/结束显示数学用；行内进出忽略）。
    fn math_shift(&mut self, _display: bool) -> Result<()> {
        Ok(())
    }
    /// 数学样式原语（`\displaystyle`=0/`\textstyle`=1/`\scriptstyle`=2/`\scriptscriptstyle`=3；M4-2）。
    fn math_style(&mut self, _style: u8) -> Result<()> {
        Ok(())
    }
    /// 分式原语（`\over`=None 默认厚度 / `\atop`=Some(0) / `\above`=显式；M4-2）。
    fn math_fraction(&mut self, _thickness: Option<i64>) -> Result<()> {
        Ok(())
    }
    /// `\left<delimiter>`：`None` = `\left.`（空定界符；M4-2）。
    fn math_left(&mut self, _delim: Option<u32>) -> Result<()> {
        Ok(())
    }
    /// `\left` 建 math left group（16）前的挂起标记：下一个组为数学定界组。
    /// expander 在 `\left`/`\middle` 时调用（随后 begin_group 消费）；布局侧
    /// 置 pending_kind，使 currentgrouptype 返回 16（ETRIP L356 检查）。
    fn math_left_begin(&mut self) -> Result<()> {
        Ok(())
    }
    /// `\right<delimiter>`：`None` = `\right.`（M4-2）。
    fn math_right(&mut self, _delim: Option<u32>) -> Result<()> {
        Ok(())
    }
    /// e-TeX `\middle<delimiter>`（\left...\right 内分隔符；M4-5）。
    fn math_middle(&mut self, _delim: Option<u32>) -> Result<()> {
        Ok(())
    }
    /// `\sqrt`：根式（M4-2）。
    fn math_sqrt(&mut self) -> Result<()> {
        Ok(())
    }
    /// `\radical<delimiter><math field>`：根式原子（\sqrt 底层，带定界符号；TRIP L412）。
    fn math_radical(&mut self, _delim: Option<u32>) -> Result<()> {
        Ok(())
    }
    /// `\spacefactor` 实时查询（活参数，由排版器维护；TRIP L277 `\showthe\spacefactor`）。
    fn space_factor(&self) -> i64 {
        1000
    }
    /// `\spacefactor=<number>` 赋值（组作用域恢复由排版器负责；TRIP L209/L288）。
    fn set_space_factor(&mut self, _v: i64) -> Result<()> {
        Ok(())
    }
    /// `\/`：斜体校正（水平模式发 kern / 数学模式斜体校正原子 / 垂直模式报错；TRIP L410/L412）。
    fn italic_correction(&mut self) -> Result<()> {
        Ok(())
    }
    /// `\mathord`=0/`\mathbin`=1/`\mathop`=2/`\mathrel`=3/`\mathopen`=4/
    /// `\mathclose`=5/`\mathpunct`=6/`\mathinner`=7：给下一字段定类（M4-2）。
    fn math_class(&mut self, _class: u8) -> Result<()> {
        Ok(())
    }
    /// `\accent`/`\mathaccent`（M4）：`plain` 为 true 时表示 `\accent` 在数学模式
    /// 被改道为 `\mathaccent`（tex.web math_ac；已由 sink 报告改道消息）。
    /// nucleus 字段的 `{` 检查与扫描由排版器按后续 token/组完成。
    fn math_accent(&mut self, _plain: bool) -> Result<()> {
        Ok(())
    }
    /// `\underline`：等待字段组（tex.web math_ac——内容收为 Underline 原子）。
    fn math_underline(&mut self) -> Result<()> {
        Ok(())
    }
    /// `\mathchar<15-bit>`：完整数学字符原子（tex.web math_char——类/族/码）。
    fn math_char_full(&mut self, _n: u32) -> Result<()> {
        Ok(())
    }
    /// `\overline`：等待字段组（内容收为 Overline 原子）。
    fn math_overline(&mut self) -> Result<()> {
        Ok(())
    }
    /// 数学字体族分配（M4-3）：`\textfont<fam>=<fontcs>` 等。
    /// `kind`：0=text、1=script、2=scriptscript；`fam` 0-15。
    fn math_font(&mut self, _kind: u8, _fam: u8, _font: u32) -> Result<()> {
        Ok(())
    }
    /// 断字模式表（M4-6）：`\patterns{...}` 的原始文本字节
    /// （字母/数字/`.` 及空格分隔符；由 ntex-layout 的 Liang trie 解析）。
    fn patterns(&mut self, _patterns: Vec<u8>) -> Result<()> {
        Ok(())
    }
    /// 断字异常词表（ETRIP）：`\hyphenation{...}` 解析结果。
    /// `words[i] = (小写字母序列, 允许断点位置)`；断点 0 = 词首、len = 词尾。
    /// 断字时异常词优先于模式表。
    fn hyphenation(&mut self, _words: Vec<(Vec<u8>, Vec<usize>)>) -> Result<()> {
        Ok(())
    }
    /// `\setbox<n>=<box>`（ETRIP）：下一个封装的盒子存入寄存器 n（sink 侧实现）。
    fn setbox(&mut self, _idx: usize, _global: bool) -> Result<()> {
        Ok(())
    }
    /// `\hbox to/spread <dimen>`（ETRIP）：记录当前盒子规格（to = 精确目标宽/高，
    /// spread = 在自然尺寸上增减；单位 sp）。随下一个盒子组生效。
    fn box_spec(&mut self, _to: Option<i64>, _spread: Option<i64>) -> Result<()> {
        Ok(())
    }
    /// 无限阶胶水（ETRIP）：`\hfil`=0/`\hfill`=1/`\hss`=2/`\vfil`=3/`\vfill`=4/`\vss`=5
    /// （方向不符的模式忽略；排版器侧换算 order）。
    fn fill_glue(&mut self, _kind: u8) -> Result<()> {
        Ok(())
    }
    /// `\vsplit<n> to/spread <dimen>`（ETRIP）：纵向拆分盒子寄存器 n 的顶部，
    /// 寄存器 n 保留余量，结果盒子按 `\setbox` 目标路由或追加。
    fn vsplit(&mut self, _idx: usize, _to: Option<i64>, _spread: Option<i64>) -> Result<()> {
        Ok(())
    }
    /// 组开始（`{`）：VM 已完成组作用域簿记。
    fn group_begin(&mut self, _line: u32) -> Result<()> {
        Ok(())
    }
    /// 组结束（`}`）：VM 已完成赋值回滚与 `\aftergroup` 处理。
    fn group_end(&mut self) -> Result<()> {
        Ok(())
    }
    /// 直通原语：VM 不做排版语义，交由 sink 解释。
    fn primitive(&mut self, _prim: Primitive) -> Result<()> {
        Ok(())
    }
    /// `\par` 的源码行号（折行警告 `in paragraph at lines a--b` 用；
    /// 默认 no-op，排版器实现记录）。
    fn paragraph_line(&mut self, _line: i64) -> Result<()> {
        Ok(())
    }
    /// 胶水（`\hskip`/`\vskip` 的扫描结果）。
    fn glue(&mut self, _g: Glue) -> Result<()> {
        Ok(())
    }
    /// 字距（`\kern` 的扫描结果）。
    fn kern(&mut self, _width: i64) -> Result<()> {
        Ok(())
    }
    /// 惩罚（`\penalty` 的扫描结果）。
    fn penalty(&mut self, _penalty: i64) -> Result<()> {
        Ok(())
    }
    /// 规则（`\hrule`/`\vrule` 的扫描结果；width/height/depth 单位 sp）。
    fn rule(&mut self, _width: i64, _height: i64, _depth: i64) -> Result<()> {
        Ok(())
    }
    /// 内部参数变化（`\parindent`/`\baselineskip`/`\lineskip`/`\lineskiplimit` 赋值）。
    fn param_changed(&mut self, _kind: ParamKind, _value: ParamValue) -> Result<()> {
        Ok(())
    }
    /// e-TeX 惩罚数组变化（`\interlinepenalties`/`\clubpenalties`/`\widowpenalties`/
    /// `\displaywidowpenalties` 赋值；kind 0-3 与 expander 的 penalty_arrays 下标一致）。
    /// 折行器在行间插入惩罚节点时读取（tex.web 语义：数组按索引、超出用末值）。
    fn penalty_array_changed(&mut self, _kind: u8, _values: &[i64]) -> Result<()> {
        Ok(())
    }
    /// `\sfcode<字符>=<值>` 赋值（M3-4 词间距 spacefactor 表）。
    fn sfcode_changed(&mut self, _charcode: u8, _value: u32) -> Result<()> {
        Ok(())
    }
    /// `\output` 例程定义状态变化（M3-5-3）：true 时 fire_up 改道 box255 + 待执行，
    /// false 时直通 shipout（默认）。
    fn output_defined(&mut self, _defined: bool) -> Result<()> {
        Ok(())
    }
    /// 是否有待执行的输出例程（fire_up 已把页面放入 box255）。
    fn output_pending(&self) -> bool {
        false
    }
    /// 清除待执行标记并返回是否曾有（expander 注入输出例程 token 时调用）。
    fn take_output_pending(&mut self) -> bool {
        false
    }
    /// 待输出例程处理的页面数量（expander 按进度判断例程是否消费了 box255）。
    fn output_pending_count(&self) -> usize {
        0
    }
    /// 丢弃所有待输出例程处理的页面（例程不消费 box255 时）。
    fn discard_pending_pages(&mut self) {}
    /// `\box<n>`：取盒子寄存器（`\shipout` 前缀时封装为页面，否则作为节点追加）。
    fn box_register(&mut self, _idx: usize) -> Result<()> {
        Ok(())
    }
    /// 字体选择器执行（M3-4）：`\font` 定义的 cs 被使用，设置当前字体。
    fn font_selected(&mut self, _font: u32) -> Result<()> {
        Ok(())
    }
    /// `\font<cs>=<name>` 定义完成（ETRIP showbox）：登记 FontId → cs 名
    /// （tex.web 字体标识显示用 cs 名，如 `.\trip 1`；csid 的 intern 表在
    /// expander 侧，故由 expander 解析名字后推送）。
    fn font_defined(&mut self, _font: u32, _cs_name: &str) -> Result<()> {
        Ok(())
    }
    /// 当前字体（TRIP：`\textfont1=\font` 中 `\font` 作当前字体选择器）。
    fn current_font(&self) -> u32 {
        0
    }
    /// 当前模式名（`\tracingcommands` 追踪输出用；tex.web print_mode 语义，
    /// 如 "vertical mode"/"restricted horizontal mode"）。
    fn mode_name(&self) -> String {
        "no mode".to_string()
    }
    /// RFC-3：页面真正输出（`\shipout` 边界）时置位；Expander 在 token 边界
    /// 检查并 flush 延迟写流。默认 sink（纯展开轨道）不置位 → 无副作用。
    fn take_write_flush_pending(&mut self) -> bool {
        false
    }
    // ETRIP 冲刺：终端转录（\message/\show/\showthe/\write16）
    /// `\message{...}`：输出文本到终端与日志（TeX 语义：不换行）。
    fn message(&mut self, _text: String) -> Result<()> {
        Ok(())
    }
    /// `\show`/`\showthe`：输出 meaning 行（TeX："> ..."，带换行）。
    fn show(&mut self, _text: String) -> Result<()> {
        Ok(())
    }
    /// `\write16{...}`：写终端（流 16 = 终端，带换行）。
    fn write16(&mut self, _text: String) -> Result<()> {
        Ok(())
    }
    /// TeX 错误恢复模式统一入口：`! 消息` 写入转录（与既有散落调用格式
    /// 一致）；恢复动作由调用方决定（钳制/插入/忽略——TeX error() 语义：
    /// 报错后继续执行，不终止作业）。
    fn report_error(&mut self, msg: &str) {
        let _ = self.write16(format!("! {msg}\n"));
    }
    /// TeXXeT 方向节点（`\beginL`/`\endL`/`\beginR`/`\endR`；\TeXXeTstate=1 时）。
    fn direction_node(&mut self, _kind: DirectionKind) -> Result<()> {
        Ok(())
    }
    /// `\mark`/`\marks<n>`：mark 节点（class：`\marks` 的寄存器号，`\mark` 为 None）。
    fn mark(&mut self, _class: Option<i64>, _text: String) -> Result<()> {
        Ok(())
    }
    /// `\showbox<n>`：把盒子寄存器内容格式化到转录（TeX show_box 风格）。
    fn showbox(&mut self, _idx: usize) -> Result<()> {
        Ok(())
    }
    /// `\discretionary{pre}{post}{replace}`：断字节点（组内容 token 由排版器转节点）。
    fn discretionary(
        &mut self,
        _pre: Vec<Token>,
        _post: Vec<Token>,
        _replace: Vec<Token>,
    ) -> Result<()> {
        Ok(())
    }
    /// `\insert<num>{<general text>}`：insert 节点（无维度；内容只收集不排版）。
    fn insert_node(&mut self, _class: usize, _toks: Vec<Token>) -> Result<()> {
        Ok(())
    }
    /// `\vadjust{<vertical material>}`：adjust 节点（无维度）。
    fn vadjust(&mut self, _toks: Vec<Token>) -> Result<()> {
        Ok(())
    }
    /// `\write<n>{...}`（非 \immediate）：whatsit 节点（无维度）。
    fn whatsit(&mut self, _text: String) -> Result<()> {
        Ok(())
    }
    /// e-TeX marks 族查询：`\topmarks<n>`（继承自上一页的 botmarks；初始为空）。
    fn topmarks(&self, _class: i64) -> String {
        String::new()
    }
    /// e-TeX marks 族查询：`\firstmarks<n>`（当前页第一个出现的 marks<n>）。
    fn firstmarks(&self, _class: i64) -> String {
        String::new()
    }
    /// e-TeX marks 族查询：`\botmarks<n>`（当前页最后一个出现的 marks<n>）。
    fn botmarks(&self, _class: i64) -> String {
        String::new()
    }
    /// e-TeX marks 族查询：`\splitfirstmarks<n>`（\vsplit 拆出盒的第一个 marks）。
    fn splitfirstmarks(&self, _class: i64) -> String {
        String::new()
    }
    /// e-TeX marks 族查询：`\splittopmarks<n>`（\vsplit 前继承的 topmarks）。
    fn splittopmarks(&self, _class: i64) -> String {
        String::new()
    }
    /// e-TeX marks 族查询：`\splitbotmarks<n>`（\vsplit 拆出盒的最后 marks）。
    fn splitbotmarks(&self, _class: i64) -> String {
        String::new()
    }
    /// e-TeX `\lastnodetype`：当前列表尾节点类型码（空列表 -1）。
    fn last_node_type(&self) -> i64 {
        -1
    }
    /// ETRIP 第二波：`\lastpenalty`：当前列表尾若是 penalty 节点返回其值，否则 0。
    fn last_penalty(&self) -> i64 {
        0
    }
    /// ETRIP/TRIP：`\lastskip`：当前列表尾 glue 节点宽度（sp；无则 0）。
    fn last_skip(&self) -> i64 {
        0
    }
    /// ETRIP/TRIP：`\lastkern`：当前列表尾 kern 节点宽度（sp；无则 0）。
    fn last_kern(&self) -> i64 {
        0
    }
    /// ETRIP 第二波：`\lastbox`：从当前列表尾移除盒子节点。
    /// 若 `\setbox<n>=` 待赋值目标存在，移除的盒子存入该寄存器；否则作为节点
    /// 拆开并入当前列表（TeX lastbox 语义）。无尾盒子节点时空操作（垂直模式无意义）。
    fn lastbox(&mut self) -> Result<()> {
        Ok(())
    }
    /// ETRIP 第二波：`\unskip`：移除当前列表尾的 glue 节点（无则无操作）。
    fn unskip(&mut self) -> Result<()> {
        Ok(())
    }
    /// ETRIP 第二波：`\unpenalty`：移除当前列表尾的 penalty 节点（无则无操作）。
    fn unpenalty(&mut self) -> Result<()> {
        Ok(())
    }
    /// TRIP 冲刺：`\unkern`：移除当前列表尾的 kern 节点（无则无操作）。
    fn unkern(&mut self) -> Result<()> {
        Ok(())
    }
    /// ETRIP 第二波：`\copy<n>`：取出盒子寄存器 n 的内容（保留寄存器），
    /// 作为节点追加或封装为页面（`\shipout` 前缀时）。
    fn copy_box(&mut self, _idx: usize) -> Result<()> {
        Ok(())
    }
    /// ETRIP 第二波：`\unhbox<n>` / `\unhcopy<n>`：拆开 hbox 寄存器 n 的内容，
    /// 子节点追加到当前水平列表。`copy=true` 保留寄存器（`\unhcopy`）。
    fn unhbox(&mut self, _idx: usize, _copy: bool) -> Result<()> {
        Ok(())
    }
    /// ETRIP 第二波：`\unvbox<n>` / `\unvcopy<n>`：拆开 vbox 寄存器 n 的内容，
    /// 子节点追加到当前垂直列表。`copy=true` 保留寄存器（`\unvcopy`）。
    fn unvbox(&mut self, _idx: usize, _copy: bool) -> Result<()> {
        Ok(())
    }
    /// ETRIP 第二波：`\wd/\ht/\dp<n>`：盒子寄存器 n 的尺寸（sp）。
    /// `dim`：0=width、1=height、2=depth；void 盒子返回 0。
    fn box_dim(&self, _idx: usize, _dim: u8) -> i64 {
        0
    }
    /// ETRIP 第二波：`\wd/\ht/\dp<n>=<dimen>`：设置盒子寄存器 n 的尺寸。
    /// void 盒子报错（TeX "Cannot \wd a void box"）；越界钳制。
    fn set_box_dim(&mut self, _idx: usize, _dim: u8, _value: i64) -> Result<()> {
        Ok(())
    }
    /// ETRIP 第二波：`\showgroups`：转储组栈状态到终端/日志（诊断原语）。
    fn showgroups(&mut self) -> Result<()> {
        Ok(())
    }
    /// ETRIP 第二波：`\showlists`：转储节点列表栈状态到终端/日志（诊断原语）。
    fn showlists(&mut self) -> Result<()> {
        Ok(())
    }
    /// e-TeX `\currentgrouptype`：当前组类型码（bottom=0 ... math_left=16）。
    fn current_group_type(&self) -> i64 {
        0
    }
    /// e-TeX `\ifinner`：当前是否内部模式（数学/受限水平/内层垂直）。
    fn if_inner(&self) -> bool {
        false
    }
    /// 当前模式码（TeX 模式码：1=垂直、2=水平、3=数学、4=内层垂直、
    /// 5=受限水平、6=显示数学）；`\ifvmode`/`\ifhmode`/`\ifmmode` 用。
    fn mode_code(&self) -> i64 {
        1
    }
    /// 盒子寄存器种类（0=void、1=hbox、2=vbox）；`\ifvoid`/`\ifhbox`/`\ifvbox` 用。
    fn box_register_kind(&self, _idx: usize) -> i64 {
        0
    }
    /// `\begingroup`：下一个组为半简单组（currentgrouptype=14）。
    fn semisimple_begin(&mut self) -> Result<()> {
        Ok(())
    }
    /// `\valign{`/`\halign{`：下一个组为对齐组（currentgrouptype=6）。
    /// `is_halign`：true=\halign（行堆叠），false=\valign（列并排）。
    fn align_begin(&mut self, _is_halign: bool) -> Result<()> {
        Ok(())
    }
    /// `\noalign{`：下一个组为无对齐组（currentgrouptype=7）。
    fn noalign_begin(&mut self) -> Result<()> {
        Ok(())
    }
    /// 输出例程的隐式组（tex.web：例程在 group_code=output_group 组内执行，
    /// currentgrouptype=8）。VM 在注入例程 token 前调用。
    fn output_routine_begin(&mut self) -> Result<()> {
        Ok(())
    }
    /// `\cr`（对齐行结束）：无操作（ETRIP 简化）。
    fn align_row_end(&mut self) -> Result<()> {
        Ok(())
    }
    /// `\thinmuskip/\medmuskip/\thickmuskip`（muskip 寄存器 0/1/2）赋值：
    /// 数学间距参数（math_to_hlist 读；默认 thin=3mu/med=4mu/thick=5mu）。
    fn muskip_param(&mut self, _idx: usize, _glue: Glue) -> Result<()> {
        Ok(())
    }
    /// `\raise`/`\lower<dimen>`：记录盒子参考点位移（下一个封装盒子生效）。
    fn raise(&mut self, _amount: i64) -> Result<()> {
        Ok(())
    }
    /// `\moveleft<dimen>`：记录盒子水平左移（TRIP 冲刺；默认 no-op）。
    fn move_left(&mut self, _amount: i64) -> Result<()> {
        Ok(())
    }
    /// `\moveright<dimen>`：记录盒子水平右移（TRIP 冲刺；默认 no-op）。
    fn move_right(&mut self, _amount: i64) -> Result<()> {
        Ok(())
    }
    /// 累积的终端转录文本（默认空；收集型 sink 实现）。
    fn transcript(&self) -> &str {
        ""
    }
    /// 已收集的输出 token（默认空；测试与 `Expander::output` 用）。
    fn tokens(&self) -> &[Token] {
        &[]
    }
    /// 取回本体收集的 token（仅收集型 sink 实现；`\edef` 区域用）。
    fn take_tokens(self: Box<Self>) -> Option<Vec<Token>> {
        None
    }
    /// 类型擦除互转：排版器（ntex-layout）在运行结束后取回其 NodeBuilder。
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
    /// 类型擦除只读互转（export_state 读当前字体等）。
    fn as_any_ref(&self) -> &dyn std::any::Any;
}

/// 默认 sink：收集 token 流（等价于 M1/M2 的 `output: Vec<Token>`）。
#[derive(Debug, Default)]
pub struct VecSink {
    pub tokens: Vec<Token>,
    /// 终端转录累积（\message/\show/\write16）。
    pub transcript: String,
}

impl TokenSink for VecSink {
    fn token(&mut self, tok: Token) -> Result<()> {
        self.tokens.push(tok);
        Ok(())
    }

    fn message(&mut self, text: String) -> Result<()> {
        self.transcript.push_str(&text);
        Ok(())
    }

    fn show(&mut self, text: String) -> Result<()> {
        self.transcript.push_str(&text);
        self.transcript.push('\n');
        Ok(())
    }

    fn write16(&mut self, text: String) -> Result<()> {
        self.transcript.push_str(&text);
        self.transcript.push('\n');
        Ok(())
    }

    fn transcript(&self) -> &str {
        &self.transcript
    }

    fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    fn take_tokens(self: Box<Self>) -> Option<Vec<Token>> {
        Some(self.tokens)
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any_ref(&self) -> &dyn std::any::Any {
        self
    }
}
