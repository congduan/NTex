/// 原语变体单一事实源：由 [`define_primitives!`] 生成枚举、编号映射与可展开判定。
///
/// 语法（`EXPANDABLE:` 列表置于最前并用方括号包裹，规避宏匹配器对
/// `expr` 片段后继 token 的限制与 ident 列表歧义）：
/// ```text
/// define_primitives! {
///     (可选枚举文档注释)
///     EXPANDABLE: [可展开原语列表],   // 与变体列表同义，拼错由编译器兜底
///     变体列表（可带 `= 初始编号`，默认自前一变体 +1；编号自 1 起连续），
/// }
/// ```
/// - `from_u16` 从枚举声明自动推导编号，杜绝手写表与枚举错位（历史教训：
///   `HFilNeg`/`Error`/`VarUnit` 的 from_u16 编号曾漂移）；
/// - `EXPANDABLE:` 列表中的标识符若拼错，展开为 `Self::X` 时由编译器报错兜底；
/// - 新增原语只需在变体列表登记（并按需加入 `EXPANDABLE:`），
///   注册表与一致性测试分别见 `crates/ntex-core/src/expand/builtins.rs`
///   与 `expand/tests.rs` 的 `primitive_*` 测试。
macro_rules! define_primitives {
    (
        $(#[$enum_meta:meta])*
        EXPANDABLE: [$($expandable:ident),* $(,)?]
        $(
            $variant:ident $(= $init:expr)?
        ),+ $(,)?
    ) => {
        $(#[$enum_meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(u16)]
        pub enum Primitive {
            $($variant $(= $init)?,)+
        }

        impl Primitive {
            /// 全部变体（定义顺序；builtins 注册与一致性测试共用）。
            pub const ALL: &[Primitive] = &[$(Self::$variant,)+];

            /// 参与展开的原语（其余为不可展开，直接执行）。
            ///
            /// TeX 可展开集：`\expandafter`/`\noexpand`/`\the`/`\number`、
            /// e-TeX `\unexpanded`/`\detokenize`/`\eTeXversion`/`\eTeXrevision`；
            /// 条件原语由 process_one 单独拦截（\edef 中同样展开）。
            pub fn is_expandable(self) -> bool {
                matches!(self, $(Self::$expandable)|+)
            }

            /// 原始值（`.fmt` 快照序列化；变体自 1 起连续）。
            pub const fn as_u16(self) -> u16 {
                self as u16
            }

            /// 从原始值恢复（`.fmt` 快照反序列化）。
            pub const fn from_u16(v: u16) -> Option<Self> {
                Some(match v {
                    $(x if x == Self::$variant as u16 => Self::$variant,)+
                    _ => return None,
                })
            }
        }
    };
}

define_primitives! {
    /// M1 原语集（随里程碑扩充）。
    EXPANDABLE: [
        Expandafter,
        Noexpand,
        The,
        Number,
        Unexpanded,
        Detokenize,
        // \eTeXrevision 可展开（→".6"，etrip 版本宏习语 `\number\eTeXversion\eTeXrevision`）；
        // \eTeXversion 是内部整数（非可展开），`\the\eTeXversion` 由 the_tokens_after 直读。
        ETeXRevision,
        String_,
        JobName,
        Csname,
        // \meaning<token>（TeX 可展开原语）
        Meaning,
        // e-TeX marks 族查询（可展开：返回字符 token 文本）
        TopMarks,
        FirstMarks,
        BotMarks,
        SplitFirstMarks,
        SplitTopMarks,
        SplitBotMarks,
        // TRIP 冲刺：可展开原语（\romannumeral/\char/\uppercase/\lowercase/\endinput/\ignorespaces/\fontname）
        RomanNumeral,
        Char,
        Uppercase,
        Lowercase,
        EndInput,
        Ignorespaces,
        FontName,
    ]
    // 变体列表：自 1 起连续编号（0 为 eqtb 槽 Undefined 哨兵）
    Def = 1,
    Edef,
    Gdef,
    Let,
    Relax,
    Expandafter,
    Noexpand,
    Catcode,
    End,
    // M1-7 扫描顺序原语
    Futurelet,
    Aftergroup,
    Afterassignment,
    // M1-9 条件原语
    If,
    IfCat,
    IfNum,
    IfDim,
    IfX,
    IfOdd,
    IfCase,
    IfTrue,
    IfFalse,
    Else,
    Fi,
    Or,
    // M1-10 寄存器
    Count,
    Dimen,
    Skip,
    Toks,
    The,
    Global,
    // M1-11 组
    BeginGroup,
    EndGroup,
    // M3-2 排版原语：参数扫描在 VM 侧（结果经 sink 输出）；
    // hbox/vbox/vtop/par 直通 sink 由排版器解释。
    HBox,
    VBox,
    VTop,
    HSkip,
    VSkip,
    Kern,
    Penalty,
    HRule,
    VRule,
    Par,
    // M3-2-2 内部参数与段落
    ParIndent,
    BaselineSkip,
    LineSkip,
    LineSkipLimit,
    Indent,
    NoIndent,
    // M3-3 折行参数
    HSize,
    Tolerance,
    // M3-4 字体：\font<cs>=<名字>[at/scaled]
    Font,
    // M3-5 输出：\shipout<box>（直通 sink，由排版器封装页面）
    ShipOut,
    // M3-5 断页参数：\vsize/\topskip/\maxdepth/\parskip
    VSize,
    TopSkip,
    MaxDepth,
    ParSkip,
    // M3-4 词间距：\sfcode<字符>=<spacefactor>
    SfCode,
    // M3-5-3 输出例程：\output=<general text>（token 列表存储）；\box<n>（盒子寄存器）
    Output,
    Box,
    // M3 收尾（RFC-3）：VFS 副作用原语
    Input,
    OpenIn,
    CloseIn,
    NewRead,
    Read,
    NewWrite,
    OpenOut,
    CloseOut,
    Write,
    Immediate,
    // M4-2 数学原语（直通 sink，由排版器解释）
    DisplayStyle,
    TextStyle,
    ScriptStyle,
    ScriptScriptStyle,
    Over,
    Atop,
    Left,
    Right,
    Sqrt,
    MathOrd,
    MathBin,
    MathOp,
    MathRel,
    MathOpen,
    MathClose,
    MathPunct,
    MathInner,
    Nonscript,
    // TRIP 冲刺：\limits/\nolimits/\displaylimits（mathop 后置上下限标志；单独出现报错）
    Limits,
    NoLimits,
    DisplayLimits,
    // M4-5 e-TeX 展开扩展
    Protected,
    IfDefined,
    IfCsname,
    Unless,
    NumExpr,
    Detokenize,
    Unexpanded,
    ETeXVersion,
    ETeXRevision,
    // M4-3 数学字体族
    TextFont,
    ScriptFont,
    ScriptScriptFont,
    // M4-6 断字：\patterns 模式表
    Patterns,
    // M4-4 显示数学间距参数
    AboveDisplaySkip,
    BelowDisplaySkip,
    AboveDisplayShortSkip,
    BelowDisplayShortSkip,
    PreDisplayPenalty,
    PostDisplayPenalty,
    // M4-5 e-TeX 扩展：\dimexpr/\glueexpr/\ifprimitive/\scantokens
    Dimexpr,
    Glueexpr,
    Muexpr,
    IfPrimitive,
    Scantokens,
    // ETRIP 冲刺：TeX 内部整数参数
    EndlineChar,
    NewlineChar,
    DefaultHyphenChar,
    DefaultSkewChar,
    // ETRIP 冲刺：宏定义前缀与变体
    Outer,
    Long,
    Xdef,
    // ETRIP 冲刺：内部只读整数
    Badness,
    // ETRIP 冲刺：字体参数
    FontDimen,
    // ETRIP 冲刺：终端转录原语
    Message,
    Show,
    ShowThe,
    // ETRIP 冲刺：\number 可展开原语
    Number,
    // ETRIP 冲刺：TeX/e-TeX 内部整数参数（值存 Params.misc，按下标索引）
    TracingStats,
    TracingLostChars,
    TracingOnline,
    TracingCommands,
    TracingRestores,
    TracingAssigns,
    TracingGroups,
    TracingIfs,
    TracingScantokens,
    TracingNesting,
    LeftHyphenMin,
    RightHyphenMin,
    HBadness,
    PreTolerance,
    ShowBoxDepth,
    ShowBoxBreadth,
    Language,
    SavingHyphCodes,
    SavingVDiscards,
    InteractionMode,
    TeXXeTState,
    MathSurround,
    LastLineFit,
    PredisplayDirection,
    EveryEof,
    // TRIP 冲刺：内部整数参数（misc 下标 35-50，见 int_param_index/param::default_misc）
    VBadness,
    GlobalDefs,
    FloatingPenalty,
    LinePenalty,
    BinoPenalty,
    RelPenalty,
    AdjDemerits,
    Looseness,
    MaxDeadCycles,
    HangAfter,
    Uchyph,
    Fam,
    HyphenPenalty,
    DoubleHyphenDemerits,
    FinalHyphenDemerits,
    HoldingInserts,
    // ETRIP 冲刺：交互模式命令（\batchmode/\nonstopmode/\scrollmode/\errorstopmode）
    BatchMode,
    NonstopMode,
    ScrollMode,
    ErrorStopMode,
    // ETRIP 冲刺：\chardef/\countdef/\dimendef/\skipdef/\toksdef（cs 绑定字符/寄存器）
    Chardef,
    Countdef,
    Dimendef,
    Skipdef,
    Toksdef,
    // ETRIP 冲刺：\hyphenchar<font>=<int>（字体断字符）
    HyphenChar,
    // ETRIP 冲刺：\delcode<num>=<num>（字符定界符码）
    DelCode,
    // ETRIP 冲刺：\muskip/\muskipdef（mu 胶量寄存器；1mu = 65536 单位）
    Muskip,
    Muskipdef,
    // ETRIP 冲刺：\thinmuskip/\medmuskip/\thickmuskip（muskip 寄存器 0/1/2）
    ThinMuskip,
    MedMuskip,
    ThickMuskip,
    // ETRIP 冲刺：\lccode<char>=<num>（小写码表）
    LcCode,
    // ETRIP 冲刺：\advance<寄存器> <增量>（寄存器运算）
    Advance,
    // ETRIP 冲刺：\hyphenation{...}（断字异常词表；按语言存，断字时优先于 patterns）
    Hyphenation,
    // ETRIP 冲刺：\setbox<n>=<box>（盒子寄存器赋值）
    SetBox,
    // ETRIP 冲刺：\parfillskip（段落末行填充胶水）
    ParFillSkip,
    // ETRIP 冲刺：\␣（control space，输出一个空格 token）
    ControlSpace,
    // ETRIP 冲刺：无限阶胶水 \hfil/\hfill/\hss/\vfil/\vfill/\vss
    HFil,
    HFill,
    HSS,
    VFil,
    VFill,
    VSS,
    // ETRIP 冲刺：\vsplit<n> to/spread <dimen>（纵向拆分盒子寄存器）
    VSplit,
    // ETRIP 冲刺：\everyjob=<tokens>（作业开始 token 表；暂映射 toks 0）
    EveryJob,
    // ETRIP 冲刺：\dump（initex 收尾：写 fmt + 结束作业）
    Dump,
    // ETRIP 冲刺：TeXXeT 方向原语 \beginL/\endL/\beginR/\endR（方向节点）
    BeginL,
    EndL,
    BeginR,
    EndR,
    // e-TeX（M4-5）：\middle<delimiter>（\left...\right 内分隔符）
    Middle,
    // ETRIP 冲刺：\mark<general text>（mark 节点）与 e-TeX \marks<n><general text>
    Mark,
    Marks,
    // ETRIP 冲刺：\showbox<n>（显示盒子寄存器内容）
    ShowBox,
    // ETRIP 冲刺：\inputlineno（当前输入行号；只读整数）与 \string<token>（token 转文本）
    InputLineNo,
    String_,
    // ETRIP 冲刺：e-TeX 内部只读整数 \currentgrouplevel/\currentgrouptype/\lastnodetype
    CurrentGroupLevel,
    CurrentGroupType,
    LastNodeType,
    // ETRIP 冲刺：\discretionary{pre}{post}{replace}（断字节点）
    Discretionary,
    // ETRIP 冲刺：\insert<num>{<general text>}（insert 节点）与 \vadjust{...}（adjust 节点）
    Insert,
    VAdjust,
    // ETRIP 冲刺：对齐 \valign/\halign/\cr/\noalign 与 \mathchoice{}{}{}{}
    Valign,
    Halign,
    Cr,
    NoAlign,
    MathChoice,
    // ETRIP 冲刺：输出例程 \deadcycles；\tracingmacros/\tracingoutput/\errorcontextlines
    DeadCycles,
    TracingMacros,
    TracingOutput,
    ErrorContextLines,
    // ETRIP 冲刺：\raise/\lower<dimen><box>（盒子参考点位移）与 \span（对齐模板）
    Raise,
    Lower,
    Span,
    // ETRIP 冲刺：\special{<general text>}（whatsit 节点）与 \jobname（作业名）
    Special,
    JobName,
    // ETRIP 冲刺：\vcenter<box>（数学垂直居中盒，简化按 vbox）
    VCenter,
    // ETRIP 冲刺：\ifinner（内部垂直/受限水平/数学模式为真；TeX 条件原语）
    IfInner,
    // ETRIP 冲刺：\csname...\endcsname（构造控制序列名，可展开）
    Csname,
    EndCsname,
    // ETRIP 冲刺：模式/盒子/EOF 条件原语（\ifvmode/\ifhmode/\ifmmode/\ifeof/\ifvoid/\ifhbox/\ifvbox）
    IfVMode,
    IfHMode,
    IfMMode,
    IfEof,
    IfVoid,
    IfHBox,
    IfVBox,
    // ETRIP 冲刺：寄存器算术（\multiply/\divide）
    Multiply,
    Divide,
    // ETRIP 冲刺：e-TeX 只读整数（\currentiflevel/\currentiftype/\currentifbranch）
    CurrentIfLevel,
    CurrentIfType,
    CurrentIfBranch,
    // ETRIP 冲刺：\meaning<token>（可展开：token 含义文本）与 \mathchardef\cs=<num>
    Meaning,
    MathCharDef,
    // ETRIP 冲刺：胶水分量查询（\gluestretchorder/\glueshrinkorder 整数上下文、
    // \gluestretch/\glueshrink 尺寸上下文）
    GlueStretchOrder,
    GlueShrinkOrder,
    GlueStretch,
    GlueShrink,
    // ETRIP 冲刺：\showtokens{<text>}（显示展开后的 token 列表）与 \readline<n>to\cs
    ShowTokens,
    ReadLine,
    // ETRIP 冲刺：字体字符查询（\iffontchar 条件 + \fontcharwd/ht/dp/ic 度量）与
    // \showifs（显示条件嵌套）
    IfFontChar,
    FontCharWd,
    FontCharHt,
    FontCharDp,
    FontCharIc,
    ShowIfs,
    // ETRIP 冲刺：段落形状（\parshape 赋值 + \parshapelength/indent/dimen 读取）
    Parshape,
    ParshapeLength,
    ParshapeIndent,
    ParshapeDimen,
    // ETRIP 冲刺：e-TeX marks 族查询（可展开，返回字符 token 文本）
    TopMarks,
    FirstMarks,
    BotMarks,
    SplitFirstMarks,
    SplitTopMarks,
    SplitBotMarks,
    // TRIP 冲刺：可展开原语（\romannumeral/\char/\uppercase/\lowercase/\endinput/\ignorespaces/\fontname）
    RomanNumeral,
    Char,
    Uppercase,
    Lowercase,
    EndInput,
    Ignorespaces,
    FontName,
    // TRIP 冲刺：不可展开原语（\uccode 字符码表/\cleaders/\xleaders/\unkern）
    Uccode,
    Cleaders,
    XLeaders,
    Unkern,
    // ETRIP 第二波：e-TeX mu 转换原语（可展开，参数为 mu/skip 胶水，输出胶水/mu 胶水 token）
    MuToGlue,
    GlueToMu,
    // ETRIP 第二波：e-TeX 惩罚数组（\interlinepenalties n p1 p2 ... pn 等）
    InterLinePenalties,
    ClubPenalties,
    WidowPenalties,
    DisplayWidowPenalties,
    // ETRIP 第二波：e-TeX 丢弃物控制整数（misc 数组）
    PageDiscards,
    SplitDiscards,
    LostChars,
    // ETRIP 第二波：盒子复制/拆包原语（\copy/\unhbox/\unvbox/\unhcopy/\unvcopy/\lastbox）
    Copy,
    UnHBox,
    UnVBox,
    UnHCopy,
    UnVCopy,
    LastBox,
    // ETRIP 第二波：盒子尺寸查询/赋值（\wd/\ht/\dp，可展开读尺寸）
    Wd,
    Ht,
    Dp,
    // ETRIP 第二波：段落/断页参数原语（胶水/尺寸/整数参数）
    LeftSkip,
    RightSkip,
    PrevDepth,
    InterLinePenalty,
    ClubPenalty,
    WidowPenalty,
    DisplayWidowPenalty,
    // ETRIP 第二波：列表尾操作（\unskip/\lastpenalty/\unpenalty；\lastpenalty 可展开读整数）
    UnSkip,
    LastPenalty,
    UnPenalty,
    // ETRIP 第二波：诊断原语（\showgroups/\showlists）
    ShowGroups,
    ShowLists,
    // ETRIP 第二波：折行追踪整数参数（misc 数组）
    TracingParagraphs,
    // ETRIP 第二波：对齐模板跳过（\omit；简化为 no-op，由对齐组后续实现语义）
    Omit,
    // TRIP：\mag（放大倍数整数参数；L67 赋值 2000，L160 `.5\mag` 作 dimen 乘子）
    Mag,
    // TRIP 冲刺：dimen 内部参数（initex 预定义）
    NullDelimiterSpace,
    ScriptSpace,
    OverfullRule,
    VOffset,
    HOffset,
    // TRIP 冲刺：\mathcode（字符数学码表）
    MathCode,
    // TRIP 冲刺：\noboundary（数学字符边界抑制；水平/垂直模式 no-op）
    NoBoundary,
    // TRIP 冲刺：\moveleft/\moveright<dimen><box>（盒子水平位移）
    MoveLeft,
    MoveRight,
    // TRIP 冲刺：\accent<8-bit number><字符>（读音符；数学模式报错恢复）
    Accent,
    // TRIP 冲刺：\vfilneg（plain.tex：负 1fil 的 vskip）
    VFilNeg,
    // TRIP 冲刺：\hfilneg（plain.tex：负 1fil 的 hskip）
    HFilNeg,
    // TRIP 冲刺：\error（plain.tex 宏：errmessage；TRIP L402 分支不执行）
    Error,
    // TRIP 冲刺：\varunit（plain.tex 字体标识符；TRIP L404 `\fontdimen1000=20\varunit`）
    VarUnit,
    // TRIP 冲刺：\xspaceskip（plain.tex 胶水参数；TRIP L410 `\advance\xspaceskip by-\xspaceskip`）
    XSpaceSkip,
    // TRIP 冲刺：\spacefactor（活空间因子；由排版器维护，赋值/读取实时经 sink）
    SpaceFactor,
    // TRIP 冲刺：\everymath（进入数学模式时注入的 token 列表；VM 存储）
    EveryMath,
    // TRIP 冲刺：\/（斜体校正；水平模式发 kern，数学模式为斜体校正原子）
    ItalicCorrection,
    // TRIP 冲刺：\radical<delimiter><math field>（根式原子，\sqrt 底层，带定界符号）
    Radical,
    // TRIP 冲刺：\delimiterfactor（内部整数参数，misc 数组；delimiter 缩放因子，默认 901）
    DelimiterFactor,
    // TRIP 冲刺：\escapechar（内部整数参数，misc 数组；控制序列显示用 escape 字符，默认 `\`）
    EscapeChar,
}
