//! 内部参数（M3-2-2/3-3/M3-5）：`\parindent`、`\baselineskip`、`\lineskip`、
//! `\lineskiplimit`、`\hsize`、`\tolerance`、`\vsize`、`\topskip`、`\maxdepth`、`\parskip`。
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
    /// `\vsize`：页目标高度（断页 DP 用，M3-5）。
    VSize,
    /// `\topskip`：每页首行顶部的胶水（M3-5）。
    TopSkip,
    /// `\maxdepth`：页面最后盒子的最大深度（M3-5）。
    MaxDepth,
    /// `\parskip`：段落之间的胶水（M3-5）。
    ParSkip,
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
    /// `\vsize`：页目标高度（plain 默认 643.20255pt）。
    pub vsize: i64,
    /// `\topskip`：页首行顶部胶水（plain 默认 10pt）。
    pub topskip: Glue,
    /// `\maxdepth`：页面最后盒子最大深度（plain 默认 4pt）。
    pub maxdepth: i64,
    /// `\parskip`：段落间胶水（plain 默认 0pt plus 1pt）。
    pub parskip: Glue,
}

impl Default for Params {
    /// TeX initex/plain 默认值：`\parindent=0`、`\baselineskip=12pt`、
    /// `\lineskip=0`、`\lineskiplimit=0`、`\hsize=6.5in`、`\tolerance=10000`、
    /// `\vsize=643.20255pt`、`\topskip=10pt`、`\maxdepth=4pt`、`\parskip=0pt plus 1pt`。
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
            // plain \vsize：643.20255pt × 2^16（TeX 内部存为 scaled 四舍五入）
            vsize: 42_152_922,
            topskip: Glue {
                width: 10 * SP_PER_PT,
                stretch: 0,
                shrink: 0,
            },
            maxdepth: 4 * SP_PER_PT,
            parskip: Glue {
                width: 0,
                stretch: SP_PER_PT,
                shrink: 0,
            },
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
            ParamKind::VSize => ParamValue::Dimen(self.vsize),
            ParamKind::TopSkip => ParamValue::Glue(self.topskip),
            ParamKind::MaxDepth => ParamValue::Dimen(self.maxdepth),
            ParamKind::ParSkip => ParamValue::Glue(self.parskip),
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
            (ParamKind::VSize, ParamValue::Dimen(v)) => self.vsize = v,
            (ParamKind::TopSkip, ParamValue::Glue(g)) => self.topskip = g,
            (ParamKind::MaxDepth, ParamValue::Dimen(v)) => self.maxdepth = v,
            (ParamKind::ParSkip, ParamValue::Glue(g)) => self.parskip = g,
            // 类型不匹配忽略（VM 侧保证参数种类与值类型匹配）
            _ => {}
        }
    }
}
