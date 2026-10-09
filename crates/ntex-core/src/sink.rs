//! 输出 sink：VM 与排版器的边界（M3-2）。
//!
//! ntex-core 的展开引擎是纯 token 级 VM（token 进 → token 出）。排版器
//! （ntex-layout 的 typesetter）需要结构化的"排版事件"而非裸 token 流。
//!
//! 事件按域拆成七个子 trait（R1 债务治理；[`TokenSink`] 是它们的组合视图）：
//!
//! - [`CoreSink`]：主循环基本事件——[`CoreSink::token`]（普通输出 token，
//!   字符/未处理控制序列）、[`CoreSink::group_begin`]/[`CoreSink::group_end`]
//!   （组定界；VM 内部仍执行组作用域簿记与 `\aftergroup`）、
//!   [`CoreSink::glue`]/[`CoreSink::kern`]/[`CoreSink::penalty`]/[`CoreSink::rule`]
//!   （排版原语结果，参数扫描在 VM 侧完成）、[`CoreSink::primitive`]
//!   （直通原语，VM 不做排版语义，交由 sink 解释）；
//! - [`FontSink`]/[`MathSink`]/[`BoxSink`]/[`AlignSink`]/[`PageSink`]/[`IoSink`]：
//!   字体、数学、盒寄存器与列表尾、对齐、页与输出例程、IO 各域。
//!
//! 默认实现 [`VecSink`] 收集 token，行为与 M1/M2 一致（`Expander::output`）；
//! 除 [`CoreSink`] 外各域全走子 trait 默认 no-op。

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

/// 对齐单元结束方式（tex.web `extra_info` 存的 `cur_chr`）。
/// `\span` 结束的单元不产生独立 cell_end（与后续单元合并为跨列单元）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignCellEnd {
    /// `&`（cat-4 字符）结束：普通换列。
    Tab,
    /// `\cr`/`\crcr` 结束：行末（随后 `align_row_end`）。
    Cr,
}

/// 主循环基本事件：token 流、组定界、直通原语、水平材料节点、
/// 参数/内部量赋值推送、模式查询与转录/类型擦除。
pub trait CoreSink {
    /// 输出 token（字符、未处理的控制序列等）。
    fn token(&mut self, tok: Token) -> Result<()>;
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
    /// 数学胶（`\mskip`/`\mkern` 的 mu 单位扫描结果；tex.web `new_mu_glue`）。
    /// mu 值以 NTex 约定的伪 mu（N×65536）存 width/stretch/shrink，排版层在
    /// mlist_to_hlist 时按当前 style 的 em/18 换算（tex.web `math_glue`）。
    /// 默认 no-op（仅排版引擎侧覆写）。
    fn mu_glue(&mut self, _g: Glue) -> Result<()> {
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
    /// `\discretionary{pre}{post}{replace}`：断字节点（组内容 token 由排版器转节点）。
    fn discretionary(
        &mut self,
        _pre: Vec<Token>,
        _post: Vec<Token>,
        _replace: Vec<Token>,
    ) -> Result<()> {
        Ok(())
    }
    /// TeXXeT 方向节点（`\beginL`/`\endL`/`\beginR`/`\endR`；\TeXXeTstate=1 时）。
    fn direction_node(&mut self, _kind: DirectionKind) -> Result<()> {
        Ok(())
    }
    /// `\begingroup`：下一个组为半简单组（currentgrouptype=14）。
    fn semisimple_begin(&mut self) -> Result<()> {
        Ok(())
    }
    /// 当前模式名（`\tracingcommands` 追踪输出用；tex.web print_mode 语义，
    /// 如 "vertical mode"/"restricted horizontal mode"）。
    fn mode_name(&self) -> String {
        "no mode".to_string()
    }
    /// 当前模式码（TeX 模式码：1=垂直、2=水平、3=数学、4=内层垂直、
    /// 5=受限水平、6=显示数学）；`\ifvmode`/`\ifhmode`/`\ifmmode` 用。
    fn mode_code(&self) -> i64 {
        1
    }
    /// 段落即将开始（tex.web `new_graf` 前置查询）：当前处于垂直模式、且下一个
    /// 水平材料 token 会开段。显示公式续排（`after_display`）不算——该路径无
    /// parskip/缩进、tex.web 也不触发 `\everypar`。
    ///
    /// list 机制刀：Expander 在派发水平材料 token 前查询；true 时先
    /// [`CoreSink::par_begin`] 开段、注入 `\everypar` token 列表，再把触发
    /// token 压回输入（tex.web `back_input; new_graf(true)` 的顺序）。
    /// `\everypar` 必须先于触发 token 展开：LaTeX 段落钩子机器
    /// （`\g__para_standard_everypar_tl`）用 `\box_gset_to_last` 取走缩进盒，
    /// 此刻水平列表里必须只有缩进盒。
    fn par_begin_imminent(&self) -> bool {
        false
    }
    /// 开段（tex.web `new_graf`）：`\parskip`、模式切水平、spacefactor 复位、
    /// 缩进盒（`indented=false` 即 `\noindent` 语义，不落缩进盒）。触发 token
    /// 由 Expander 决定消费或压回输入。
    fn par_begin(&mut self, _indented: bool) -> Result<()> {
        Ok(())
    }
    /// `\parshape` 表镜像推送（`[(indent, width)]`，sp；空表 = 无形状）。
    /// 排版器折行/行盒装配按 tex.web §16742/§17425 取逐行宽与左缩进——
    /// LaTeX `\list` 的 `\parshape \@ne \@totalleftmargin \linewidth` 是
    /// quotation/abstract 等 list 环境两侧缩进的唯一机制。
    fn set_parshape(&mut self, _shape: &[(i64, i64)]) {}
    /// e-TeX `\currentgrouptype`：当前组类型码（bottom=0 ... math_left=16）。
    fn current_group_type(&self) -> i64 {
        0
    }
    /// e-TeX `\ifinner`：当前是否内部模式（数学/受限水平/内层垂直）。
    fn if_inner(&self) -> bool {
        false
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

/// 字体域：字体选择与定义、当前字体、spacefactor 与斜体校正。
pub trait FontSink {
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
    /// `\hyphenchar<font>=<int>` 赋值（字体断字符；负值禁用自动断字）。
    fn hyphen_char_changed(&mut self, _font: u32, _value: i64) -> Result<()> {
        Ok(())
    }
    /// 当前字体（TRIP：`\textfont1=\font` 中 `\font` 作当前字体选择器）。
    fn current_font(&self) -> u32 {
        0
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
}

/// 数学域：进出数学模式、样式/分式/定界/根式/类/重音/字段、
/// 数学字体族与 muskip 参数。
pub trait MathSink {
    /// 数学移位（`$`，cat 3；M4-1）：进出数学模式。
    /// `display`：VM 检测到连续 `$$`（进入显示数学/结束显示数学用；行内进出忽略）。
    fn math_shift(&mut self, _display: bool) -> Result<()> {
        Ok(())
    }
    /// `$$` 是否允许进**显示**数学（tex.web init_math 的 `mode>0` 判定：
    /// 垂直/普通水平为 true；受限水平（\hbox/\halign 模板）为 false——
    /// 此时第二个 `$` 由 VM back_input，`$$` 退化为两次独立的一进一出）。
    /// 默认 true（纯展开轨道无模式概念，不影响既有行为）。
    fn math_display_allowed(&self) -> bool {
        true
    }
    /// 这个 `$` 是**收**数学还是**开**数学（tex.web：由 `cur_list.mode_field`
    /// 唯一裁决——mmode/+mmode 为收，其余为开）。核心侧 `in_math` 是 `$` 翻转
    /// 旗标，在 `\[\halign{…$x$…}\]` 一类"显示数学内嵌对齐单元"结构里与
    /// 排版层真实模式脱钩（显示数学开过之后旗标恒 true，单元首 `$` 被误判为
    /// 收），故 `$` 的进出裁决必须问排版层。默认 false（纯展开轨道无模式
    /// 概念，保持既有"进"语义）。
    fn math_shift_will_close(&self) -> bool {
        false
    }
    /// **收**数学时是否要求配对的第二个 `$`（tex.web mmode+math_shift →
    /// after_math：显示数学（mode=+mmode）收尾必 "Check that another $
    /// follows"——有则一并消费、无则报 "Display math should end with $$"
    /// 照收；行内数学（mode=-mmode）走 Finish math in text，**不 peek**，
    /// 随后的 `$` 由水平模式按 init_math 重新判定）。默认 true（纯展开轨道
    /// 无模式概念，保持"peek 到即消费"的既有行为）。
    fn math_close_consumes_dollar(&self) -> bool {
        true
    }
    /// 数学码表（\mathcode）是否适用于当前字符 token：tex.web 按**模式**分流
    /// （mmode+letter/other 才查表，L21845）。核心侧 `in_math` 是 `$` 翻转旗标，
    /// 数学内文本盒（`\hbox{…}`/`\mbox{…}`/`\text{…}`）不清它——若照搬旗标，
    /// 盒内文本字符也会被改道成数学字符（`.`→cmmi 槽 0x3A 而非 cmr 0x2E），
    /// 目录点线（\@dottedtocline 的 `\hbox{$…\hbox{.}…$}`）全排成错字形。
    /// 排版层按真实列表模式裁决；默认 true（纯展开轨道无模式概念，不影响既有行为）。
    fn math_code_applies(&self) -> bool {
        true
    }
    /// 数学样式原语（`\displaystyle`=0/`\textstyle`=1/`\scriptstyle`=2/`\scriptscriptstyle`=3；M4-2）。
    fn math_style(&mut self, _style: u8) -> Result<()> {
        Ok(())
    }
    /// 分式原语（`\over`=None 默认厚度 / `\atop`=Some(0) / `\above`=显式；M4-2）。
    fn math_fraction(&mut self, _thickness: Option<i64>) -> Result<()> {
        Ok(())
    }
    /// 分式定界符（`\overwithdelims`/`\abovewithdelims`/`\atopwithdelims` 在
    /// `math_fraction` 前扫得的一对定界符；tex.web math_fraction 的 left/right
    /// delimiter 字段，make_fraction 包在分式两侧）。默认 no-op。
    fn math_fraction_delims(&mut self, _left: Option<u32>, _right: Option<u32>) -> Result<()> {
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
    /// `\eqno`/`\leqno`（显示数学内）：编号材料从此处起**独立成列表**收集
    /// （tex.web start_eq_no 进普通数学模式，after_math 把它 hpack natural
    /// 成编号盒 a；`leqno=true` 表示编号在左）。
    fn math_eqno(&mut self, _leqno: bool) -> Result<()> {
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
    /// `\mathord`=0/`\mathbin`=1/`\mathop`=2/`\mathrel`=3/`\mathopen`=4/
    /// `\mathclose`=5/`\mathpunct`=6/`\mathinner`=7：给下一字段定类（M4-2）。
    fn math_class(&mut self, _class: u8) -> Result<()> {
        Ok(())
    }
    /// `\displaylimits`(0)/`\limits`(1)/`\nolimits`(2)（tex.web limit_switch →
    /// `math_limit_switch`）：改写**当前数学层最后一个原子**（须为 Op）的
    /// 上下限摆放方式。Op 的默认 subtype 是 0=normal（display 堆叠），
    /// `\int`/`\oint` 定义为 `\intop\nolimits`（上下标走普通脚本位）。
    fn math_limit_switch(&mut self, _mode: u8) -> Result<()> {
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
    /// `\thinmuskip/\medmuskip/\thickmuskip`（muskip 寄存器 0/1/2）赋值：
    /// 数学间距参数（math_to_hlist 读；默认 thin=3mu/med=4mu/thick=5mu）。
    fn muskip_param(&mut self, _idx: usize, _glue: Glue) -> Result<()> {
        Ok(())
    }
}

/// 盒域：盒寄存器（setbox/copy/unh/unv/尺寸）、列表尾操作
/// （\lastbox/\unskip/\unpenalty/\unkern 与 \last* 查询）、盒子位移。
pub trait BoxSink {
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
    /// `\box<n>`：取盒子寄存器（`\shipout` 前缀时封装为页面，否则作为节点追加）。
    fn box_register(&mut self, _idx: usize) -> Result<()> {
        Ok(())
    }
    /// 盒子寄存器种类（0=void、1=hbox、2=vbox）；`\ifvoid`/`\ifhbox`/`\ifvbox` 用。
    fn box_register_kind(&self, _idx: usize) -> i64 {
        0
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
    /// e-TeX `\lastnodetype`：当前列表尾节点类型码（空列表 -1）。
    fn last_node_type(&self) -> i64 {
        -1
    }
    /// ETRIP 第二波：`\lastpenalty`：当前列表尾若是 penalty 节点返回其值，否则 0。
    fn last_penalty(&self) -> i64 {
        0
    }
    /// `\lastskip` 作胶水量（tex.web L8535 `glue_val: if type(tail)=glue_node then
    /// cur_val:=glue_ptr(tail)`）：返回**当前列表尾**的完整 glue 节点（含
    /// stretch/shrink），尾节点不是 glue → None。`\skip@=\lastskip`（latex.ltx
    /// `\sw@slant`）依赖此语义；此前被通用胶参数臂抢走 → 读到从未更新的
    /// param 镜像 0，`\textbf` 前词间空格丢失。
    fn last_glue(&self) -> Option<Glue> {
        None
    }
    /// ETRIP/TRIP：`\lastskip`：当前列表尾 glue 节点宽度（sp；无则 0）。
    fn last_skip(&self) -> i64 {
        0
    }
    /// ETRIP/TRIP：`\lastkern`：当前列表尾 kern 节点宽度（sp；无则 0）。
    fn last_kern(&self) -> i64 {
        0
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
    /// `\showbox<n>`：把盒子寄存器内容格式化到转录（TeX show_box 风格）。
    fn showbox(&mut self, _idx: usize) -> Result<()> {
        Ok(())
    }
}

/// 对齐域：\halign/\valign 的 preamble、单元与行边界（halign 战役落点）。
pub trait AlignSink {
    /// `\valign{`/`\halign{`：下一个组为对齐组（currentgrouptype=6）。
    /// `is_halign`：true=\halign（行堆叠），false=\valign（列并排）。
    fn align_begin(&mut self, _is_halign: bool) -> Result<()> {
        Ok(())
    }
    /// `\cr`（对齐行结束）：排版器封装当前行（tex.web fin_row）。
    fn align_row_end(&mut self) -> Result<()> {
        Ok(())
    }
    /// 对齐 preamble 结束（preamble 尾部 `\cr` 后）：给出各列边界的
    /// `\tabskip` 胶水快照（`tabskips.len() == 列数 + 1`，含首尾）。
    /// tex.web：preamble 列表结构为 [glue, alignrecord, glue, ... , glue]。
    fn align_preamble_end(&mut self, _tabskips: Vec<Glue>) -> Result<()> {
        Ok(())
    }
    /// 对齐单元开始（u 模板注入前；tex.web init_col/init_span：push_nest）。
    /// 排版器压入单元内容列表（\halign → 受限水平；\valign → 内部垂直）。
    fn align_cell_begin(&mut self) -> Result<()> {
        Ok(())
    }
    /// 对齐单元结束（v 模板执行完，tex.web fin_col 的单元封装时机）。
    /// `end`：`&`（Tab）或 `\cr`（Cr，随后必有 align_row_end）；
    /// `span_len`：本单元跨列数（`\span` 合并单元 ≥1，tex.web cur_span 机制）。
    fn align_cell_end(&mut self, _end: AlignCellEnd, _span_len: u16) -> Result<()> {
        Ok(())
    }
    /// `\noalign{`：下一个组为无对齐组（currentgrouptype=7）。
    fn noalign_begin(&mut self) -> Result<()> {
        Ok(())
    }
}

/// 页域：输出例程战刀 1-5 的全部事件（例程点火/shipout/dead cycles）、
/// 页号链镜像、e-TeX marks 族与 insert/vadjust 页面材料。
pub trait PageSink {
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
    /// 自上次查询以来输出例程是否显式消费过 `\box255`（或同槽的 `\vsplit`/清空）。
    fn output_consumed(&mut self) -> bool {
        false
    }
    /// 丢弃所有待输出例程处理的页面（例程不消费 box255 时）。
    fn discard_pending_pages(&mut self) {}
    /// `fire_up` 记录的最佳断点惩罚（输出例程刀 1，tex.web
    /// `@<Set the value of |output_penalty|@>`）。`None` = 最佳断点非惩罚节点
    /// （胶水/kern 自然断页）→ 引擎侧写 `\outputpenalty := inf_penalty`(10000)。
    /// 在例程点火（`maybe_inject_output`）前查询并全局写入 `\outputpenalty`。
    fn output_break_penalty(&mut self) -> Option<i64> {
        None
    }
    /// 自上次查询以来是否真正 shipout 过页面（`\shipout\box255` 例程产出，或
    /// 无例程直通）。输出例程刀 1：`dead_cycles` 清零依据（tex.web ship_out
    /// L12707 `dead_cycles:=0`）——查询即取走（take 语义）。
    fn take_page_shipped(&mut self) -> bool {
        false
    }
    /// tex.web `@<Perform the default output routine@>`（dead cycles 分支）：
    /// 待处理页面不经用户例程直接 shipout（`\output` 例程从不 ship 时，
    /// `dead_cycles >= max_dead_cycles` 触发）。
    fn default_output_routine(&mut self) {}
    /// `\count<n>=<值>`（含 `\advance`/组内回滚还原；所有下标都会推送）：
    /// 页号链镜像——tex.web `ship_out` L12694 在 shipout 边界**直接读 count(j)**
    /// 打页标签、写 DVI bop 的 10 计数字，排版器侧无从反查引擎寄存器，故与
    /// [`TokenSink::param_changed`] 同款赋值即推送。
    fn count_changed(&mut self, _idx: usize, _value: i64) -> Result<()> {
        Ok(())
    }
    /// `\dimen<n>=<值>`（含 `\advance`/组内回滚还原）：页面构建器用 insert
    /// 三联寄存器中的 `\dimen<class>` 作为本页该类插入物上限。
    fn dimen_changed(&mut self, _idx: usize, _value: i64) -> Result<()> {
        Ok(())
    }
    /// `\skip<n>=<值>`（含 `\advance`/组内回滚还原）：页面构建器用 insert
    /// 三联寄存器中的 `\skip<class>` 作为该类插入物首次出现的页内间距。
    fn skip_changed(&mut self, _idx: usize, _value: Glue) -> Result<()> {
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
    /// `\insert<num>{`：下一个组为 insert 组（tex.web insert_group=11；组体在
    /// 内部垂直模式由主循环排版，`}` 处 vpack(natural) 挂 ins_node）。
    fn insert_begin(&mut self, _class: usize) -> Result<()> {
        Ok(())
    }
    /// `\vadjust{<vertical material>}`：adjust 节点（无维度）。
    fn vadjust(&mut self, _toks: Vec<Token>) -> Result<()> {
        Ok(())
    }
    /// 输出例程的隐式组（tex.web：例程在 group_code=output_group 组内执行，
    /// currentgrouptype=8）。VM 在注入例程 token 前调用。
    fn output_routine_begin(&mut self) -> Result<()> {
        Ok(())
    }
    /// `\mark`/`\marks<n>`：mark 节点（class：`\marks` 的寄存器号，`\mark` 为 None）。
    fn mark(&mut self, _class: Option<i64>, _text: String) -> Result<()> {
        Ok(())
    }
    /// RFC-3：页面真正输出（`\shipout` 边界）时置位；Expander 在 token 边界
    /// 检查并 flush 延迟写流。默认 sink（纯展开轨道）不置位 → 无副作用。
    fn take_write_flush_pending(&mut self) -> bool {
        false
    }
}

/// IO 域：断字表摄取、终端转录（\message/\show/\write16 与错误恢复文本）、
/// 诊断转储与延迟写 whatsit。
pub trait IoSink {
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
    /// TeX `help1..6`：紧随 `! 消息` 一行的解释文本（无 `! ` 前缀）。
    /// tex.web L8265-8268 mu_error：! Incompatible glue units. 后跟 help1 行。
    fn report_help(&mut self, text: &str) {
        let _ = self.write16(format!("{text}\n"));
    }
    /// ETRIP 第二波：`\showgroups`：转储组栈状态到终端/日志（诊断原语）。
    fn showgroups(&mut self) -> Result<()> {
        Ok(())
    }
    /// ETRIP 第二波：`\showlists`：转储节点列表栈状态到终端/日志（诊断原语）。
    fn showlists(&mut self) -> Result<()> {
        Ok(())
    }
    /// `\write<n>{...}`（非 \immediate）：whatsit 节点（无维度）。
    fn whatsit(&mut self, _text: String) -> Result<()> {
        Ok(())
    }
    /// `\special{...}`（图片管线 Step B 起也与 `\pdfrefximage` 载荷共用）：
    /// whatsit 节点，shipout 时经 DVI xxx 指令落墨到后端（tex.web `ship_out`
    /// 的 `dvi_special`）；与延迟 `\write` 的 whatsit（shipout 写流、不进
    /// DVI）在节点上分型，见 [`crate::sink::IoSink::whatsit`]。
    fn special(&mut self, _text: String) -> Result<()> {
        Ok(())
    }
}

/// VM 排版事件消费者（组合 trait）：按域拆分的七个子 trait 的聚合视图。
///
/// 拆分动机（R1 债务治理）：单接口 107 个方法令输出例程战/数学战/对齐战
/// 每刀都要挤同一个 trait 定义与同一个 impl 块（并行领地冲突根源）。拆分后
/// 各域只动自己的子 trait + 自己的 impl 块。
///
/// expander 侧仍是 `Box<dyn TokenSink>`：子 trait 方法经超trait vtable
/// 直接可达（`self.sink.math_fraction(..)` 无需改动）；需要子 trait 对象时
/// 用 trait upcast（Rust 1.86+）。
///
/// 要求 `Debug`（`Expander` 派生 Debug）。事件方法返回 `Result`：排版器
/// （ntex-layout）可拒绝非法输入（如 `\par` 出现在受限水平模式、盒子组
/// 未闭合）。`\edef` 区域经 [`CoreSink::take_tokens`] 取回临时收集的
/// token（默认 None，由 [`VecSink`] 实现）。
pub trait TokenSink:
    CoreSink + FontSink + MathSink + BoxSink + AlignSink + PageSink + IoSink + std::fmt::Debug
{
}

/// 默认 sink：收集 token 流（等价于 M1/M2 的 `output: Vec<Token>`）。
#[derive(Debug, Default)]
pub struct VecSink {
    pub tokens: Vec<Token>,
    /// 终端转录累积（\message/\show/\write16）。
    pub transcript: String,
}

impl CoreSink for VecSink {
    fn token(&mut self, tok: Token) -> Result<()> {
        self.tokens.push(tok);
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

impl IoSink for VecSink {
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
}

/// 组合 trait 落名：VecSink 需要显式实现才能作 `dyn TokenSink` 用。
impl TokenSink for VecSink {}

// VecSink 是纯展开轨道的收集 sink：数学/字体/盒/对齐/页五域不维护状态，
// 全部走各子 trait 的默认 no-op 实现（语义与拆分前一致）。
impl FontSink for VecSink {}
impl MathSink for VecSink {}
impl BoxSink for VecSink {}
impl AlignSink for VecSink {}
impl PageSink for VecSink {}
