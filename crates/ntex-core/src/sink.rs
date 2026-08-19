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
    /// `\right<delimiter>`：`None` = `\right.`（M4-2）。
    fn math_right(&mut self, _delim: Option<u32>) -> Result<()> {
        Ok(())
    }
    /// `\sqrt`：根式（M4-2）。
    fn math_sqrt(&mut self) -> Result<()> {
        Ok(())
    }
    /// `\mathord`=0/`\mathbin`=1/`\mathop`=2/`\mathrel`=3/`\mathopen`=4/
    /// `\mathclose`=5/`\mathpunct`=6/`\mathinner`=7：给下一字段定类（M4-2）。
    fn math_class(&mut self, _class: u8) -> Result<()> {
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
    /// 组开始（`{`）：VM 已完成组作用域簿记。
    fn group_begin(&mut self) -> Result<()> {
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
}
