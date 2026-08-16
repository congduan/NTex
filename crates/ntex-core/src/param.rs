//! 内部参数（M3-2-2/3-3）：`\parindent`、`\baselineskip`、`\lineskip`、
//! `\lineskiplimit`、`\hsize`、`\tolerance`。
//!
//! TeX 的内部参数存储在 eqtb；这里用独立结构体持有。赋值走组作用域
//! （`SavedValue::Param`），值变化经 [`TokenSink::param_changed`] 事件
//! 镜像给排版器（ntex-layout），排版器据此计算段落缩进、interline glue 与折行。

use crate::register::{Glue, SP_PER_PT};

/// 参数种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    ParIndent,
    BaselineSkip,
    LineSkip,
    LineSkipLimit,
    /// `\hsize`：行目标宽度（折行用）。
    HSize,
    /// `\tolerance`：可接受的最大 badness（折行用）。
    Tolerance,
}

/// 参数值：尺寸（dimen）、胶水（glue）或整数（number）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamValue {
    Dimen(i64),
    Glue(Glue),
    Number(i64),
}

/// 内部参数集合（单位 sp）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    /// `\parindent`：段落首行缩进（可为负）。
    pub parindent: i64,
    /// `\baselineskip`：行间胶水。
    pub baselineskip: Glue,
    /// `\lineskip`：行间距小于 `\lineskiplimit` 时使用的胶水。
    pub lineskip: Glue,
    /// `\lineskiplimit`：行间胶水切换阈值。
    pub lineskiplimit: i64,
    /// `\hsize`：行目标宽度（TeX initex 默认 6.5in）。
    pub hsize: i64,
    /// `\tolerance`：可接受最大 badness（TeX initex 默认 10000）。
    pub tolerance: i64,
}

impl Default for Params {
    /// TeX initex 默认值：`\parindent=0`、`\baselineskip=12pt`、
    /// `\lineskip=0`、`\lineskiplimit=0`、`\hsize=6.5in`、`\tolerance=10000`。
    fn default() -> Self {
        Self {
            parindent: 0,
            baselineskip: Glue {
                width: 12 * SP_PER_PT,
                stretch: 0,
                shrink: 0,
            },
            lineskip: Glue {
                width: 0,
                stretch: 0,
                shrink: 0,
            },
            lineskiplimit: 0,
            // 6.5in = 13/2 × 4_736_286 sp
            hsize: 13 * 4_736_286 / 2,
            tolerance: 10_000,
        }
    }
}

impl Params {
    pub fn get(&self, kind: ParamKind) -> ParamValue {
        match kind {
            ParamKind::ParIndent => ParamValue::Dimen(self.parindent),
            ParamKind::BaselineSkip => ParamValue::Glue(self.baselineskip),
            ParamKind::LineSkip => ParamValue::Glue(self.lineskip),
            ParamKind::LineSkipLimit => ParamValue::Dimen(self.lineskiplimit),
            ParamKind::HSize => ParamValue::Dimen(self.hsize),
            ParamKind::Tolerance => ParamValue::Number(self.tolerance),
        }
    }

    pub fn set(&mut self, kind: ParamKind, value: ParamValue) {
        match (kind, value) {
            (ParamKind::ParIndent, ParamValue::Dimen(v)) => self.parindent = v,
            (ParamKind::BaselineSkip, ParamValue::Glue(g)) => self.baselineskip = g,
            (ParamKind::LineSkip, ParamValue::Glue(g)) => self.lineskip = g,
            (ParamKind::LineSkipLimit, ParamValue::Dimen(v)) => self.lineskiplimit = v,
            (ParamKind::HSize, ParamValue::Dimen(v)) => self.hsize = v,
            (ParamKind::Tolerance, ParamValue::Number(v)) => self.tolerance = v,
            // 类型不匹配忽略（VM 侧保证参数种类与值类型匹配）
            _ => {}
        }
    }
}
