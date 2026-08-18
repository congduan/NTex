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
}

impl TokenSink for VecSink {
    fn token(&mut self, tok: Token) -> Result<()> {
        self.tokens.push(tok);
        Ok(())
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
