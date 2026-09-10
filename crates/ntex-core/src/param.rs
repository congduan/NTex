//! 内部参数（M3-2-2/3-3/M3-5/M4-4）：`\parindent`、`\baselineskip`、`\lineskip`、
//! `\lineskiplimit`、`\hsize`、`\tolerance`、`\vsize`、`\topskip`、`\maxdepth`、`\parskip`、
//! 显示数学间距（`\abovedisplayskip` 等 4 个 glue + 前后 penalty）。
//!
//! TeX 的内部参数存储在 eqtb；这里用独立结构体持有。赋值走组作用域
//! （`SavedValue::Param`），值变化经 [`CoreSink::param_changed`] 事件
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
    /// TRIP 冲刺：`\xspaceskip`：句后空格胶水（plain 默认 0pt；L410 `\advance\xspaceskip by-\xspaceskip`）。
    XSpaceSkip,
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
    // TRIP 冲刺：补充标准参数
    /// `\hangindent`：TRIP 冲刺补充标准参数。
    HangIndent,
    /// `\spaceskip`：TRIP 冲刺补充标准参数。
    SpaceSkip,
    /// `\tabskip`：TRIP 冲刺补充标准参数。
    TabSkip,
    /// `\lastskip`：TRIP 冲刺补充标准参数。
    LastSkip,
    /// `\hfuzz`：TRIP 冲刺补充标准参数。
    Hfuzz,
    /// `\vfuzz`：TRIP 冲刺补充标准参数。
    Vfuzz,
    /// `\boxmaxdepth`：TRIP 冲刺补充标准参数。
    BoxMaxDepth,
    /// `\splitmaxdepth`：TRIP 冲刺补充标准参数。
    SplitMaxDepth,
    /// `\splittopskip`：TRIP 冲刺补充标准参数。
    SplitTopSkip,
    /// `\emergencystretch`：TRIP 冲刺补充标准参数。
    EmergencyStretch,
    /// `\displayindent`：TRIP 冲刺补充标准参数。
    DisplayIndent,
    /// `\delimitershortfall`：TRIP 冲刺补充标准参数。
    DelimiterShortfall,
    /// `\mathsurround`：数学公式周围水平间距（TeX **dimen** 参数，非整数；
    /// plain 默认 0pt。此前误登记为 int 导致 `\mathsurround.11em` 报 Missing number）。
    MathSurround,
    /// `\lastkern`：TRIP 冲刺补充标准参数。
    LastKern,
    /// `\pagestretch` 等：页面胶水内部量（TRIP L249 可写）。
    PageStretch,
    PageFilStretch,
    PageFillStretch,
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
    // TRIP 冲刺：TeX initex 预定义 dimen 内部参数（数学/排版）
    /// `\nulldelimiterspace`：空定界符占位宽度（TeX initex 默认 1.2pt）。
    NullDelimiterSpace,
    /// `\scriptspace`：上下标与主符号间距（TeX initex 默认 0.5pt）。
    ScriptSpace,
    /// `\overfullrule`：超满提示条宽度（TeX initex 默认 5pt；0 关闭提示）。
    OverfullRule,
    /// `\voffset`：整页纵向偏移（TeX initex 默认 0pt）。
    VOffset,
    /// `\hoffset`：整页横向偏移（TeX initex 默认 0pt）。
    HOffset,
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

/// 内部整数参数总数（TeX/e-TeX 内部整数，ETRIP/TRIP 冲刺；仅存储/回读）。
/// 下标与 [`crate::expand::int_param_index`] 的映射一致。
pub const MISC_INTS: usize = 66;

/// `\utfinputmode` 在 [`Params::misc`] 中的下标（M9 中文刀 2）：
/// 源文件输入编码开关，0 = bytes（默认）、非 0 = UTF-8 解码。
pub const MISC_UTF_INPUT_MODE: usize = 65;

/// 系统时间 → (日, 月, 年, 自午夜分钟数)（tex.web `date_and_time`；\day/\month/\year/\time）。
/// 公历转换用 Howard Hinnant 的 days-from-civil 逆算法（无外部依赖）。
fn system_date_time() -> [i64; 4] {
    // WASM（M8-A 骨架线）：wasm32-unknown-unknown 的 std 无 OS 时钟（SystemTime::now()
    // 直接 panic），而 js-sys Date 会把 wasm-bindgen 系依赖引进 ntex-core——取固定值。
    // 偏差：wasm 侧 \day/\month/\year/\time 不反映宿主时钟，恒为 1970-01-01 00:00
    // （换确定性：同输入同输出）；接真实时钟留给 B 档（js-sys 只进 ntex-wasm）。
    #[cfg(target_arch = "wasm32")]
    let secs: i64 = 0;
    #[cfg(not(target_arch = "wasm32"))]
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let minutes = secs.rem_euclid(86_400) / 60;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    [d, m, y, minutes]
}

/// 内部整数参数默认值（TeX initex/plain 默认）。
pub fn default_misc() -> [i64; MISC_INTS] {
    // 日期时间（tex.web date_and_time）：系统时钟 → (日, 月, 年, 分钟)
    let dt = system_date_time();
    [
        0,     // 0 TracingStats
        1,     // 1 TracingLostChars（initex 默认 1）
        0,     // 2 TracingOnline
        0,     // 3 TracingCommands
        0,     // 4 TracingRestores
        0,     // 5 TracingAssigns
        0,     // 6 TracingGroups
        0,     // 7 TracingIfs
        0,     // 8 TracingScantokens
        0,     // 9 TracingNesting
        2,     // 10 LeftHyphenMin（plain 默认 2）
        3,     // 11 RightHyphenMin（plain 默认 3）
        1000,  // 12 HBadness（plain 默认 1000）
        100,   // 13 PreTolerance（plain 默认 100）
        0,     // 14 ShowBoxDepth
        5,     // 15 ShowBoxBreadth
        0,     // 16 Language
        0,     // 17 SavingHyphCodes
        0,     // 18 SavingVDiscards
        0,     // 19 InteractionMode（驱动以 batchmode 启动）
        0,     // 20 TeXXeTState
        0,     // 21 (unused；曾是 \mathsurround，已改 dimen 参数)
        0,     // 22 LastLineFit
        0,     // 23 PredisplayDirection
        -1,    // 24 EveryEof（-1 = 无）
        0,     // 25 DeadCycles（输出例程循环计数）
        0,     // 26 TracingMacros
        0,     // 27 TracingOutput
        100,   // 28 ErrorContextLines（plain 默认 100）
        0,     // 29 TracingParagraphs（折行追踪；plain 默认 0）
        0,     // 30 PageDiscards（e-TeX：保存页面丢弃物；0=不保存）
        0,     // 31 SplitDiscards（e-TeX：保存 vsplit 丢弃物；0=不保存）
        2,     // 32 LostChars（e-TeX：丢失字符提示级；plain 默认 2 = 计数）
        901,   // 33 DelimiterFactor（plain 默认 901；delimiter 缩放因子）
        92,    // 34 EscapeChar（initex 默认 92 = `\`；控制序列显示用 escape 字符）
        1000,  // 35 VBadness（plain 默认 1000）
        0,     // 36 GlobalDefs（plain 默认 0）
        0,     // 37 FloatingPenalty（plain 默认 0）
        10,    // 38 LinePenalty（plain 默认 10）
        700,   // 39 BinoPenalty（plain 默认 700）
        500,   // 40 RelPenalty（plain 默认 500）
        10000, // 41 AdjDemerits（plain 默认 10000）
        0,     // 42 Looseness（plain 默认 0）
        25,    // 43 MaxDeadCycles（plain 默认 25）
        1,     // 44 HangAfter（plain 默认 1）
        1,     // 45 Uchyph（plain 默认 1）
        -1,    // 46 Fam（plain 默认 -1）
        50,    // 47 HyphenPenalty（plain 默认 50）
        10000, // 48 DoubleHyphenDemerits（plain 默认 10000）
        5000,  // 49 FinalHyphenDemerits（plain 默认 5000）
        0,     // 50 HoldingInserts（plain 默认 0）
        0,     // 51 PrevGraf（只读内部量：上一段落行数；plain 默认 0）
        0,     // 52 InsertPenalties（只读内部量：插入惩罚；plain 默认 0）
        // 53-56：日期时间（TeX initex 启动时设为系统时间 date_and_time）
        dt[0], // 53 Day
        dt[1], // 54 Month
        dt[2], // 55 Year
        dt[3], // 56 Time（自午夜分钟数）
        0,     // 57 BrokenPenalty（断行惩罚；plain 默认 0）
        0,     // 58 ExHyphenPenalty（显式连字符惩罚；plain 默认 0）
        0,     // 59 TracingPages（断页追踪开关；plain 默认 0）
        0,     // 60 Pausing（交互暂停开关；plain 默认 0）
        0,     // 61 SetLanguage（当前语言；plain 默认 0）
        0,     // 62 OutputPenalty（\\output 时惩罚；plain 默认 0）
        // LaTeX 兼容第八刀：pdfTeX 原语状态
        0, // 63 \pdfoutput（pdfTeX 默认 0 = DVI 模式；NTex 亦输出 DVI）
        1, // 64 \pdfrandomseed（随机种子状态；\pdfsetrandomseed 写、
        //        \pdfuniformdeviate 推进。pdfTeX 出厂种子非 0，取 1 避免首个
        //        随机数序列恒 0——LCG 平凡不动点）
        // M9 中文刀 2：\utfinputmode（源文件输入编码开关；initex 默认 0 = bytes——
        // 8-bit 逐字节语义原样保留，TRIP/ETRIP 口径零影响；置 1 后源码按 UTF-8
        // 解码，多字节序列合并为单个 21-bit 字符 token，>255 码位默认 letter）
        0,
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
    /// `\xspaceskip`：句后空格胶水（plain 默认 0pt）。
    pub xspaceskip: Glue,
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
    /// TRIP 冲刺补充：`\hangindent`。
    pub hangindent: Glue,
    /// TRIP 冲刺补充：`\spaceskip`。
    pub spaceskip: Glue,
    /// TRIP 冲刺补充：`\tabskip`。
    pub tabskip: Glue,
    /// TRIP 冲刺补充：`\lastskip`。
    pub lastskip: Glue,
    /// TRIP 冲刺补充：`\hfuzz`。
    pub hfuzz: i64,
    /// TRIP 冲刺补充：`\vfuzz`。
    pub vfuzz: i64,
    /// TRIP 冲刺补充：`\boxmaxdepth`。
    pub boxmaxdepth: i64,
    /// TRIP 冲刺补充：`\splitmaxdepth`。
    pub splitmaxdepth: i64,
    /// TRIP 冲刺补充：`\splittopskip`。
    pub splittopskip: Glue,
    /// TRIP 冲刺补充：`\emergencystretch`。
    pub emergencystretch: i64,
    /// TRIP 冲刺补充：`\displayindent`。
    pub displayindent: i64,
    /// TRIP 冲刺补充：`\delimitershortfall`。
    pub delimitershortfall: i64,
    /// `\mathsurround`：数学公式周围水平间距（dimen；plain 默认 0pt）。
    pub mathsurround: i64,
    /// TRIP 冲刺补充：`\lastkern`。
    pub lastkern: i64,
    /// TRIP 冲刺补充：`\pagestretch`（页面自然胶水）。
    pub pagestretch: Glue,
    /// TRIP 冲刺补充：`\pagefilstretch`。
    pub pagefilstretch: Glue,
    /// TRIP 冲刺补充：`\pagefillstretch`。
    pub pagefillstretch: Glue,
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
    /// `\nulldelimiterspace`（TeX initex 默认 1.2pt = 78643sp）。
    pub nulldelimiterspace: i64,
    /// `\scriptspace`（TeX initex 默认 0.5pt）。
    pub scriptspace: i64,
    /// `\overfullrule`（TeX initex 默认 5pt）。
    pub overfullrule: i64,
    /// `\voffset`（TeX initex 默认 0pt）。
    pub voffset: i64,
    /// `\hoffset`（TeX initex 默认 0pt）。
    pub hoffset: i64,
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
            xspaceskip: Glue::new(0, 0, 0),
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
            hangindent: Glue::ZERO,
            spaceskip: Glue::ZERO,
            tabskip: Glue::ZERO,
            lastskip: Glue::ZERO,
            hfuzz: SP_PER_PT / 10,
            vfuzz: SP_PER_PT / 10,
            boxmaxdepth: 16_384 * SP_PER_PT,
            splitmaxdepth: 16_384 * SP_PER_PT,
            splittopskip: Glue::new(10 * SP_PER_PT, 0, 0),
            emergencystretch: 0,
            displayindent: 0,
            delimitershortfall: 5 * SP_PER_PT,
            mathsurround: 0,
            lastkern: 0,
            pagestretch: Glue::new(0, 0, 0),
            pagefilstretch: Glue::new(0, 0, 0),
            pagefillstretch: Glue::new(0, 0, 0),
            // TeX 内部整数参数（initex 默认）
            endlinechar: 13,
            newlinechar: -1,
            defaulthyphenchar: 45,
            defaultskewchar: -1,
            mag: 1000,
            // TRIP 冲刺：TeX initex 默认（TeXbook 附录 D）
            nulldelimiterspace: 78_643, // 1.2pt
            scriptspace: 32_768,        // 0.5pt
            overfullrule: 327_680,      // 5pt
            voffset: 0,
            hoffset: 0,
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
            ParamKind::XSpaceSkip => ParamValue::Glue(self.xspaceskip),
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
            ParamKind::HangIndent => ParamValue::Glue(self.hangindent),
            ParamKind::SpaceSkip => ParamValue::Glue(self.spaceskip),
            ParamKind::TabSkip => ParamValue::Glue(self.tabskip),
            ParamKind::LastSkip => ParamValue::Glue(self.lastskip),
            ParamKind::Hfuzz => ParamValue::Dimen(self.hfuzz),
            ParamKind::Vfuzz => ParamValue::Dimen(self.vfuzz),
            ParamKind::BoxMaxDepth => ParamValue::Dimen(self.boxmaxdepth),
            ParamKind::SplitMaxDepth => ParamValue::Dimen(self.splitmaxdepth),
            ParamKind::SplitTopSkip => ParamValue::Glue(self.splittopskip),
            ParamKind::EmergencyStretch => ParamValue::Dimen(self.emergencystretch),
            ParamKind::DisplayIndent => ParamValue::Dimen(self.displayindent),
            ParamKind::DelimiterShortfall => ParamValue::Dimen(self.delimitershortfall),
            ParamKind::MathSurround => ParamValue::Dimen(self.mathsurround),
            ParamKind::LastKern => ParamValue::Dimen(self.lastkern),
            ParamKind::PageStretch => ParamValue::Glue(self.pagestretch),
            ParamKind::PageFilStretch => ParamValue::Glue(self.pagefilstretch),
            ParamKind::PageFillStretch => ParamValue::Glue(self.pagefillstretch),
            ParamKind::EndlineChar => ParamValue::Number(self.endlinechar),
            ParamKind::NewlineChar => ParamValue::Number(self.newlinechar),
            ParamKind::DefaultHyphenChar => ParamValue::Number(self.defaulthyphenchar),
            ParamKind::DefaultSkewChar => ParamValue::Number(self.defaultskewchar),
            ParamKind::Mag => ParamValue::Number(self.mag),
            ParamKind::NullDelimiterSpace => ParamValue::Dimen(self.nulldelimiterspace),
            ParamKind::ScriptSpace => ParamValue::Dimen(self.scriptspace),
            ParamKind::OverfullRule => ParamValue::Dimen(self.overfullrule),
            ParamKind::VOffset => ParamValue::Dimen(self.voffset),
            ParamKind::HOffset => ParamValue::Dimen(self.hoffset),
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
            (ParamKind::XSpaceSkip, ParamValue::Glue(g)) => self.xspaceskip = g,
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
            (ParamKind::HangIndent, ParamValue::Glue(v)) => self.hangindent = v,
            (ParamKind::SpaceSkip, ParamValue::Glue(v)) => self.spaceskip = v,
            (ParamKind::TabSkip, ParamValue::Glue(v)) => self.tabskip = v,
            (ParamKind::LastSkip, ParamValue::Glue(v)) => self.lastskip = v,
            (ParamKind::Hfuzz, ParamValue::Dimen(v)) => self.hfuzz = v,
            (ParamKind::Vfuzz, ParamValue::Dimen(v)) => self.vfuzz = v,
            (ParamKind::BoxMaxDepth, ParamValue::Dimen(v)) => self.boxmaxdepth = v,
            (ParamKind::SplitMaxDepth, ParamValue::Dimen(v)) => self.splitmaxdepth = v,
            (ParamKind::SplitTopSkip, ParamValue::Glue(v)) => self.splittopskip = v,
            (ParamKind::EmergencyStretch, ParamValue::Dimen(v)) => self.emergencystretch = v,
            (ParamKind::DisplayIndent, ParamValue::Dimen(v)) => self.displayindent = v,
            (ParamKind::DelimiterShortfall, ParamValue::Dimen(v)) => self.delimitershortfall = v,
            (ParamKind::MathSurround, ParamValue::Dimen(v)) => self.mathsurround = v,
            (ParamKind::LastKern, ParamValue::Dimen(v)) => self.lastkern = v,
            (ParamKind::PageStretch, ParamValue::Glue(g)) => self.pagestretch = g,
            (ParamKind::PageFilStretch, ParamValue::Glue(g)) => self.pagefilstretch = g,
            (ParamKind::PageFillStretch, ParamValue::Glue(g)) => self.pagefillstretch = g,
            (ParamKind::EndlineChar, ParamValue::Number(v)) => self.endlinechar = v,
            (ParamKind::NewlineChar, ParamValue::Number(v)) => self.newlinechar = v,
            (ParamKind::DefaultHyphenChar, ParamValue::Number(v)) => self.defaulthyphenchar = v,
            (ParamKind::DefaultSkewChar, ParamValue::Number(v)) => self.defaultskewchar = v,
            (ParamKind::Mag, ParamValue::Number(v)) => self.mag = v,
            (ParamKind::NullDelimiterSpace, ParamValue::Dimen(v)) => self.nulldelimiterspace = v,
            (ParamKind::ScriptSpace, ParamValue::Dimen(v)) => self.scriptspace = v,
            (ParamKind::OverfullRule, ParamValue::Dimen(v)) => self.overfullrule = v,
            (ParamKind::VOffset, ParamValue::Dimen(v)) => self.voffset = v,
            (ParamKind::HOffset, ParamValue::Dimen(v)) => self.hoffset = v,
            (ParamKind::MiscInt(idx), ParamValue::Number(v)) => self.misc[idx] = v,
            // 类型不匹配忽略（VM 侧保证参数种类与值类型匹配）
            _ => {}
        }
    }
}
