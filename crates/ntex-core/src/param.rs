//! 内部参数（M3-2-2/3-3/M3-5/M4-4）：`\parindent`、`\baselineskip`、`\lineskip`、
//! `\lineskiplimit`、`\hsize`、`\tolerance`、`\vsize`、`\topskip`、`\maxdepth`、`\parskip`、
//! 显示数学间距（`\abovedisplayskip` 等 4 个 glue + 前后 penalty）。
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
    /// ETRIP 冲刺：`\parfillskip`：段落末行填充胶水（plain 默认 0pt plus 1fil）。
    ParFillSkip,
    // M4-4 显示数学间距
    /// `\abovedisplayskip`：显示公式上方间距（末行不短时）。
    AboveDisplaySkip,
    /// `\belowdisplayskip`：显示公式下方间距（末行不短时）。
    BelowDisplaySkip,
    /// `\abovedisplayshortskip`：显示公式上方间距（末行短时）。
    AboveDisplayShortSkip,
    /// `\belowdisplayshortskip`：显示公式下方间距（末行短时）。
    BelowDisplayShortSkip,
    /// `\predisplaypenalty`：显示公式前断页惩罚（plain 默认 10000 = 禁断）。
    PreDisplayPenalty,
    /// `\postdisplaypenalty`：显示公式后断页惩罚（plain 默认 0）。
    PostDisplayPenalty,
    // ETRIP 第二波：段落/断页参数（plain 默认）
    /// `\leftskip`：段落左侧悬挂胶水（plain 默认 0pt）。
    LeftSkip,
    /// `\rightskip`：段落右侧悬挂胶水（plain 默认 0pt）。
    RightSkip,
    /// `\prevdepth`：上一行 depth（段落开头 interline glue 用；无前一行 = -1000pt < -1000 表示未定义）。
    PrevDepth,
    /// `\interlinepenalty`：行间断页惩罚（plain 默认 0）。
    InterLinePenalty,
    /// `\clubpenalty`：段首行后断页惩罚（plain 默认 150）。
    ClubPenalty,
    /// `\widowpenalty`：段尾行（寡行）断页惩罚（plain 默认 150）。
    WidowPenalty,
    /// `\displaywidowpenalty`：显示公式前段尾行断页惩罚（plain 默认 50）。
    DisplayWidowPenalty,
    // ETRIP 冲刺：TeX 内部整数参数（非排版参数，仅存储/回读）
    /// `\endlinechar`：行尾字符（TeX initex 默认 13 = CR；-1 表示不追加）。
    EndlineChar,
    /// `\newlinechar`：换行字符（TeX 默认 -1 = 未激活）。
    NewlineChar,
    /// `\defaulthyphenchar`：缺省断字符（TeX initex 默认 45 = `-`）。
    DefaultHyphenChar,
    /// `\defaultskewchar`：缺省 skew 字符（TeX 默认 -1 = 未激活）。
    DefaultSkewChar,
    /// TRIP：`\mag`：放大倍数（TeX initex 默认 1000；TRIP L67 设 2000）。
    Mag,
    /// TeX/e-TeX 内部整数参数（ETRIP 冲刺）：`misc[idx]`（见 [`MISC_INTS`]）。
    MiscInt(usize),
}

/// 参数值：尺寸（dimen）、胶水（glue）或整数（number）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamValue {
    Dimen(i64),
    Glue(Glue),
    Number(i64),
}

/// 内部整数参数总数（TeX/e-TeX 内部整数，ETRIP 冲刺；仅存储/回读）。
/// 下标与 [`crate::expand::int_param_index`] 的映射一致。
pub const MISC_INTS: usize = 33;

/// 内部整数参数默认值（TeX initex/plain 默认）。
pub fn default_misc() -> [i64; MISC_INTS] {
    [
        0,    // 0 TracingStats
        1,    // 1 TracingLostChars（initex 默认 1）
        0,    // 2 TracingOnline
        0,    // 3 TracingCommands
        0,    // 4 TracingRestores
        0,    // 5 TracingAssigns
        0,    // 6 TracingGroups
        0,    // 7 TracingIfs
        0,    // 8 TracingScantokens
        0,    // 9 TracingNesting
        2,    // 10 LeftHyphenMin（plain 默认 2）
        3,    // 11 RightHyphenMin（plain 默认 3）
        1000, // 12 HBadness（plain 默认 1000）
        100,  // 13 PreTolerance（plain 默认 100）
        0,    // 14 ShowBoxDepth
        5,    // 15 ShowBoxBreadth
        0,    // 16 Language
        0,    // 17 SavingHyphCodes
        0,    // 18 SavingVDiscards
        0,    // 19 InteractionMode（驱动以 batchmode 启动）
        0,    // 20 TeXXeTState
        0,    // 21 MathSurround
        0,    // 22 LastLineFit
        0,    // 23 PredisplayDirection
        -1,   // 24 EveryEof（-1 = 无）
        0,    // 25 DeadCycles（输出例程循环计数）
        0,    // 26 TracingMacros
        0,    // 27 TracingOutput
        100,  // 28 ErrorContextLines（plain 默认 100）
        0,    // 29 TracingParagraphs（折行追踪；plain 默认 0）
        0,    // 30 PageDiscards（e-TeX：保存页面丢弃物；0=不保存）
        0,    // 31 SplitDiscards（e-TeX：保存 vsplit 丢弃物；0=不保存）
        2,    // 32 LostChars（e-TeX：丢失字符提示级；plain 默认 2 = 计数）
    ]
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
    /// `\parfillskip`：段落末行填充胶水（plain 默认 0pt plus 1fil；fil 阶隐含）。
    pub parfillskip: Glue,
    // M4-4 显示数学间距（plain 默认）
    /// `\abovedisplayskip`（plain 默认 12pt plus 3pt minus 9pt）。
    pub abovedisplayskip: Glue,
    /// `\belowdisplayskip`（plain 默认 12pt plus 3pt minus 9pt）。
    pub belowdisplayskip: Glue,
    /// `\abovedisplayshortskip`（plain 默认 0pt plus 3pt）。
    pub abovedisplayshortskip: Glue,
    /// `\belowdisplayshortskip`（plain 默认 7pt plus 3pt minus 4pt）。
    pub belowdisplayshortskip: Glue,
    /// `\predisplaypenalty`（plain 默认 10000）。
    pub predisplaypenalty: i64,
    /// `\postdisplaypenalty`（plain 默认 0）。
    pub postdisplaypenalty: i64,
    // ETRIP 第二波：段落/断页参数
    /// `\leftskip`（plain 默认 0pt）。
    pub leftskip: Glue,
    /// `\rightskip`（plain 默认 0pt）。
    pub rightskip: Glue,
    /// `\prevdepth`（initex 默认 -1000pt < -1000 = 未定义；单位 sp）。
    pub prevdepth: i64,
    /// `\interlinepenalty`（plain 默认 0）。
    pub interlinepenalty: i64,
    /// `\clubpenalty`（plain 默认 150）。
    pub clubpenalty: i64,
    /// `\widowpenalty`（plain 默认 150）。
    pub widowpenalty: i64,
    /// `\displaywidowpenalty`（plain 默认 50）。
    pub displaywidowpenalty: i64,
    /// `\endlinechar`（TeX initex 默认 13）。
    pub endlinechar: i64,
    /// `\newlinechar`（TeX 默认 -1 = 未激活）。
    pub newlinechar: i64,
    /// `\defaulthyphenchar`（TeX initex 默认 45 = `-`）。
    pub defaulthyphenchar: i64,
    /// `\defaultskewchar`（TeX 默认 -1 = 未激活）。
    pub defaultskewchar: i64,
    /// TRIP：`\mag`：放大倍数（TeX initex 默认 1000）。
    pub mag: i64,
    /// TeX/e-TeX 内部整数参数（ETRIP 冲刺；下标见 [`MISC_INTS`]）。
    pub misc: [i64; MISC_INTS],
}

impl Default for Params {
    /// TeX initex/plain 默认值：`\parindent=0`、`\baselineskip=12pt`、
    /// `\lineskip=0`、`\lineskiplimit=0`、`\hsize=6.5in`、`\tolerance=200`（TeXbook：
    /// initex 默认 200——A1 修复，此前 10000 使折行 active 集永不淘汰 → O(n²)）、
    /// `\vsize=643.20255pt`、`\topskip=10pt`、`\maxdepth=4pt`、`\parskip=0pt plus 1pt`。
    fn default() -> Self {
        Self {
            parindent: 0,
            baselineskip: Glue::new(12 * SP_PER_PT, 0, 0),
            lineskip: Glue::new(0, 0, 0),
            lineskiplimit: 0,
            // 6.5in = 13/2 × 4_736_286 sp
            hsize: 13 * 4_736_286 / 2,
            tolerance: 200,
            // plain \vsize：643.20255pt × 2^16（TeX 内部存为 scaled 四舍五入）
            vsize: 42_152_922,
            topskip: Glue::new(10 * SP_PER_PT, 0, 0),
            maxdepth: 4 * SP_PER_PT,
            parskip: Glue::new(0, SP_PER_PT, 0),
            parfillskip: Glue::new(0, 1, 0), // 1fil（布局侧隐含 fil 阶）
            // M4-4 显示数学间距（plain：TeXbook p.189）
            abovedisplayskip: Glue::new(12 * SP_PER_PT, 3 * SP_PER_PT, 9 * SP_PER_PT),
            belowdisplayskip: Glue::new(12 * SP_PER_PT, 3 * SP_PER_PT, 9 * SP_PER_PT),
            abovedisplayshortskip: Glue::new(0, 3 * SP_PER_PT, 0),
            belowdisplayshortskip: Glue::new(7 * SP_PER_PT, 3 * SP_PER_PT, 4 * SP_PER_PT),
            predisplaypenalty: 10_000,
            postdisplaypenalty: 0,
            // ETRIP 第二波：段落/断页参数（plain 默认）
            leftskip: Glue::ZERO,
            rightskip: Glue::ZERO,
            prevdepth: -1_000 * SP_PER_PT - 1, // < -1000pt = 未定义
            interlinepenalty: 0,
            clubpenalty: 150,
            widowpenalty: 150,
            displaywidowpenalty: 50,
            // TeX 内部整数参数（initex 默认）
            endlinechar: 13,
            newlinechar: -1,
            defaulthyphenchar: 45,
            defaultskewchar: -1,
            mag: 1000,
            misc: default_misc(),
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
            ParamKind::ParFillSkip => ParamValue::Glue(self.parfillskip),
            ParamKind::AboveDisplaySkip => ParamValue::Glue(self.abovedisplayskip),
            ParamKind::BelowDisplaySkip => ParamValue::Glue(self.belowdisplayskip),
            ParamKind::AboveDisplayShortSkip => ParamValue::Glue(self.abovedisplayshortskip),
            ParamKind::BelowDisplayShortSkip => ParamValue::Glue(self.belowdisplayshortskip),
            ParamKind::PreDisplayPenalty => ParamValue::Number(self.predisplaypenalty),
            ParamKind::PostDisplayPenalty => ParamValue::Number(self.postdisplaypenalty),
            ParamKind::LeftSkip => ParamValue::Glue(self.leftskip),
            ParamKind::RightSkip => ParamValue::Glue(self.rightskip),
            ParamKind::PrevDepth => ParamValue::Dimen(self.prevdepth),
            ParamKind::InterLinePenalty => ParamValue::Number(self.interlinepenalty),
            ParamKind::ClubPenalty => ParamValue::Number(self.clubpenalty),
            ParamKind::WidowPenalty => ParamValue::Number(self.widowpenalty),
            ParamKind::DisplayWidowPenalty => ParamValue::Number(self.displaywidowpenalty),
            ParamKind::EndlineChar => ParamValue::Number(self.endlinechar),
            ParamKind::NewlineChar => ParamValue::Number(self.newlinechar),
            ParamKind::DefaultHyphenChar => ParamValue::Number(self.defaulthyphenchar),
            ParamKind::DefaultSkewChar => ParamValue::Number(self.defaultskewchar),
            ParamKind::Mag => ParamValue::Number(self.mag),
            ParamKind::MiscInt(idx) => ParamValue::Number(self.misc[idx]),
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
            (ParamKind::ParFillSkip, ParamValue::Glue(g)) => self.parfillskip = g,
            (ParamKind::AboveDisplaySkip, ParamValue::Glue(g)) => self.abovedisplayskip = g,
            (ParamKind::BelowDisplaySkip, ParamValue::Glue(g)) => self.belowdisplayskip = g,
            (ParamKind::AboveDisplayShortSkip, ParamValue::Glue(g)) => {
                self.abovedisplayshortskip = g
            }
            (ParamKind::BelowDisplayShortSkip, ParamValue::Glue(g)) => {
                self.belowdisplayshortskip = g
            }
            (ParamKind::PreDisplayPenalty, ParamValue::Number(v)) => self.predisplaypenalty = v,
            (ParamKind::PostDisplayPenalty, ParamValue::Number(v)) => self.postdisplaypenalty = v,
            (ParamKind::LeftSkip, ParamValue::Glue(g)) => self.leftskip = g,
            (ParamKind::RightSkip, ParamValue::Glue(g)) => self.rightskip = g,
            (ParamKind::PrevDepth, ParamValue::Dimen(v)) => self.prevdepth = v,
            (ParamKind::InterLinePenalty, ParamValue::Number(v)) => self.interlinepenalty = v,
            (ParamKind::ClubPenalty, ParamValue::Number(v)) => self.clubpenalty = v,
            (ParamKind::WidowPenalty, ParamValue::Number(v)) => self.widowpenalty = v,
            (ParamKind::DisplayWidowPenalty, ParamValue::Number(v)) => self.displaywidowpenalty = v,
            (ParamKind::EndlineChar, ParamValue::Number(v)) => self.endlinechar = v,
            (ParamKind::NewlineChar, ParamValue::Number(v)) => self.newlinechar = v,
            (ParamKind::DefaultHyphenChar, ParamValue::Number(v)) => self.defaulthyphenchar = v,
            (ParamKind::DefaultSkewChar, ParamValue::Number(v)) => self.defaultskewchar = v,
            (ParamKind::Mag, ParamValue::Number(v)) => self.mag = v,
            (ParamKind::MiscInt(idx), ParamValue::Number(v)) => self.misc[idx] = v,
            // 类型不匹配忽略（VM 侧保证参数种类与值类型匹配）
            _ => {}
        }
    }
}
