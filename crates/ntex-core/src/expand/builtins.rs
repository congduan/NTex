/// 内建原语注册表：`名字 → Primitive` 的单一事实源。
///
/// 425 项（`\muexpr` 为 `Glueexpr` 的别名，故比 `Primitive::ALL` 多一项；
/// TRIP 冲刺补充 13 个标准参数原语；LaTeX 兼容第八刀补 pdfTeX 引擎探测/兼容族
/// 12 项；M9 中文刀 2 补 \utfinputmode；图片管线 Step A 补 pdfTeX 图片族 3 项，
/// Step B 补 PDF 变换栈 \pdfsave/\pdfsetmatrix/\pdfrestore 3 项；
/// ctex 第二墙补 catcode table 三原语），
/// 供 `register_builtins` 注册与 `tests.rs` 的一致性测试共用。
pub(crate) const BUILTINS: [(&str, Primitive); 425] = [
    ("def", Primitive::Def),
    ("edef", Primitive::Edef),
    ("gdef", Primitive::Gdef),
    ("let", Primitive::Let),
    ("relax", Primitive::Relax),
    ("expandafter", Primitive::Expandafter),
    ("noexpand", Primitive::Noexpand),
    ("catcode", Primitive::Catcode),
    ("end", Primitive::End),
    // M1-7
    ("futurelet", Primitive::Futurelet),
    ("aftergroup", Primitive::Aftergroup),
    ("afterassignment", Primitive::Afterassignment),
    // M1-9
    ("if", Primitive::If),
    ("ifcat", Primitive::IfCat),
    ("ifnum", Primitive::IfNum),
    ("ifdim", Primitive::IfDim),
    ("ifx", Primitive::IfX),
    ("ifodd", Primitive::IfOdd),
    ("ifcase", Primitive::IfCase),
    ("iftrue", Primitive::IfTrue),
    ("iffalse", Primitive::IfFalse),
    ("else", Primitive::Else),
    ("fi", Primitive::Fi),
    ("or", Primitive::Or),
    // M1-10
    ("count", Primitive::Count),
    ("dimen", Primitive::Dimen),
    ("skip", Primitive::Skip),
    ("toks", Primitive::Toks),
    ("the", Primitive::The),
    ("global", Primitive::Global),
    // M1-11
    ("begingroup", Primitive::BeginGroup),
    ("endgroup", Primitive::EndGroup),
    // M3-2 排版原语
    ("hbox", Primitive::HBox),
    ("vbox", Primitive::VBox),
    ("vtop", Primitive::VTop),
    ("hskip", Primitive::HSkip),
    ("vskip", Primitive::VSkip),
    ("kern", Primitive::Kern),
    ("penalty", Primitive::Penalty),
    ("hrule", Primitive::HRule),
    ("vrule", Primitive::VRule),
    ("par", Primitive::Par),
    // M3-2-2 内部参数与段落
    ("parindent", Primitive::ParIndent),
    ("baselineskip", Primitive::BaselineSkip),
    ("lineskip", Primitive::LineSkip),
    ("lineskiplimit", Primitive::LineSkipLimit),
    ("indent", Primitive::Indent),
    ("noindent", Primitive::NoIndent),
    // M3-3 折行参数
    ("hsize", Primitive::HSize),
    ("tolerance", Primitive::Tolerance),
    // M3-4 字体
    ("font", Primitive::Font),
    // M3-5 输出
    ("shipout", Primitive::ShipOut),
    // M3-5 断页参数
    ("vsize", Primitive::VSize),
    ("topskip", Primitive::TopSkip),
    ("maxdepth", Primitive::MaxDepth),
    ("parskip", Primitive::ParSkip),
    // M3-4 词间距
    ("sfcode", Primitive::SfCode),
    // M3-5-3 输出例程
    ("output", Primitive::Output),
    ("box", Primitive::Box),
    // M3 收尾（RFC-3）：VFS 副作用原语
    ("input", Primitive::Input),
    ("openin", Primitive::OpenIn),
    ("closein", Primitive::CloseIn),
    ("newread", Primitive::NewRead),
    ("read", Primitive::Read),
    ("newwrite", Primitive::NewWrite),
    ("openout", Primitive::OpenOut),
    ("closeout", Primitive::CloseOut),
    ("write", Primitive::Write),
    ("immediate", Primitive::Immediate),
    // M4-2 数学原语
    ("displaystyle", Primitive::DisplayStyle),
    ("textstyle", Primitive::TextStyle),
    ("scriptstyle", Primitive::ScriptStyle),
    ("scriptscriptstyle", Primitive::ScriptScriptStyle),
    ("over", Primitive::Over),
    ("atop", Primitive::Atop),
    ("left", Primitive::Left),
    ("right", Primitive::Right),
    ("sqrt", Primitive::Sqrt),
    ("mathord", Primitive::MathOrd),
    ("mathbin", Primitive::MathBin),
    ("mathop", Primitive::MathOp),
    ("mathrel", Primitive::MathRel),
    ("mathopen", Primitive::MathOpen),
    ("mathclose", Primitive::MathClose),
    ("mathpunct", Primitive::MathPunct),
    ("mathinner", Primitive::MathInner),
    ("nonscript", Primitive::Nonscript),
    ("limits", Primitive::Limits),
    ("nolimits", Primitive::NoLimits),
    ("displaylimits", Primitive::DisplayLimits),
    // M4-5 e-TeX 展开扩展
    ("protected", Primitive::Protected),
    ("ifdefined", Primitive::IfDefined),
    ("ifcsname", Primitive::IfCsname),
    ("ifincsname", Primitive::IfInCsname),
    ("unless", Primitive::Unless),
    ("numexpr", Primitive::NumExpr),
    ("detokenize", Primitive::Detokenize),
    ("unexpanded", Primitive::Unexpanded),
    // pdfTeX（LaTeX 兼容第九刀一族）：\expanded{...} 按 \edef 语义全展开
    ("expanded", Primitive::Expanded),
    ("eTeXversion", Primitive::ETeXVersion),
    ("eTeXrevision", Primitive::ETeXRevision),
    // M4-3 数学字体族
    ("textfont", Primitive::TextFont),
    ("scriptfont", Primitive::ScriptFont),
    ("scriptscriptfont", Primitive::ScriptScriptFont),
    // M4-6 断字：\patterns 模式表
    ("patterns", Primitive::Patterns),
    // M4-4 显示数学间距参数
    ("abovedisplayskip", Primitive::AboveDisplaySkip),
    ("belowdisplayskip", Primitive::BelowDisplaySkip),
    ("abovedisplayshortskip", Primitive::AboveDisplayShortSkip),
    ("belowdisplayshortskip", Primitive::BelowDisplayShortSkip),
    ("predisplaypenalty", Primitive::PreDisplayPenalty),
    ("postdisplaypenalty", Primitive::PostDisplayPenalty),
    // M4-5 e-TeX 扩展
    ("dimexpr", Primitive::Dimexpr),
    ("glueexpr", Primitive::Glueexpr),
    // ETRIP 冲刺：\muexpr（独立原语，mu 单位表达式；\the 显示 "X.0mu"）
    ("muexpr", Primitive::Muexpr),
    ("ifprimitive", Primitive::IfPrimitive),
    ("scantokens", Primitive::Scantokens),
    // ETRIP 冲刺：TeX 内部整数参数
    ("endlinechar", Primitive::EndlineChar),
    ("newlinechar", Primitive::NewlineChar),
    ("defaulthyphenchar", Primitive::DefaultHyphenChar),
    ("defaultskewchar", Primitive::DefaultSkewChar),
    // ETRIP 冲刺：宏定义前缀与变体
    ("outer", Primitive::Outer),
    ("long", Primitive::Long),
    ("xdef", Primitive::Xdef),
    // ETRIP 冲刺：内部只读整数
    ("badness", Primitive::Badness),
    // ETRIP 冲刺：字体参数
    ("fontdimen", Primitive::FontDimen),
    // ETRIP 冲刺：终端转录原语
    ("message", Primitive::Message),
    ("show", Primitive::Show),
    ("showthe", Primitive::ShowThe),
    // ETRIP 冲刺：\number 可展开原语
    ("number", Primitive::Number),
    // ETRIP 冲刺：TeX/e-TeX 内部整数参数（misc 数组）
    ("tracingstats", Primitive::TracingStats),
    ("tracinglostchars", Primitive::TracingLostChars),
    ("tracingonline", Primitive::TracingOnline),
    ("tracingcommands", Primitive::TracingCommands),
    ("tracingrestores", Primitive::TracingRestores),
    ("tracingassigns", Primitive::TracingAssigns),
    ("tracinggroups", Primitive::TracingGroups),
    ("tracingifs", Primitive::TracingIfs),
    ("tracingscantokens", Primitive::TracingScantokens),
    ("tracingnesting", Primitive::TracingNesting),
    ("lefthyphenmin", Primitive::LeftHyphenMin),
    ("righthyphenmin", Primitive::RightHyphenMin),
    ("hbadness", Primitive::HBadness),
    ("pretolerance", Primitive::PreTolerance),
    ("showboxdepth", Primitive::ShowBoxDepth),
    ("showboxbreadth", Primitive::ShowBoxBreadth),
    ("language", Primitive::Language),
    ("savinghyphcodes", Primitive::SavingHyphCodes),
    ("savingvdiscards", Primitive::SavingVDiscards),
    ("interactionmode", Primitive::InteractionMode),
    ("TeXXeTstate", Primitive::TeXXeTState),
    ("mathsurround", Primitive::MathSurround),
    ("lastlinefit", Primitive::LastLineFit),
    ("predisplaydirection", Primitive::PredisplayDirection),
    ("everyeof", Primitive::EveryEof),
    // TRIP 冲刺：内部整数参数（misc 下标 35-50）
    ("vbadness", Primitive::VBadness),
    ("globaldefs", Primitive::GlobalDefs),
    ("floatingpenalty", Primitive::FloatingPenalty),
    ("linepenalty", Primitive::LinePenalty),
    ("binoppenalty", Primitive::BinoPenalty),
    ("relpenalty", Primitive::RelPenalty),
    ("adjdemerits", Primitive::AdjDemerits),
    ("looseness", Primitive::Looseness),
    ("maxdeadcycles", Primitive::MaxDeadCycles),
    ("hangafter", Primitive::HangAfter),
    ("uchyph", Primitive::Uchyph),
    ("fam", Primitive::Fam),
    ("hyphenpenalty", Primitive::HyphenPenalty),
    ("doublehyphendemerits", Primitive::DoubleHyphenDemerits),
    ("finalhyphendemerits", Primitive::FinalHyphenDemerits),
    ("holdinginserts", Primitive::HoldingInserts),
    // TRIP 冲刺：可展开原语（\romannumeral/\char/\uppercase/\lowercase/\endinput/\ignorespaces/\fontname）
    ("romannumeral", Primitive::RomanNumeral),
    ("char", Primitive::Char),
    ("uppercase", Primitive::Uppercase),
    ("lowercase", Primitive::Lowercase),
    ("endinput", Primitive::EndInput),
    ("ignorespaces", Primitive::Ignorespaces),
    ("fontname", Primitive::FontName),
    // TRIP 冲刺：不可展开原语
    ("uccode", Primitive::Uccode),
    ("cleaders", Primitive::Cleaders),
    ("xleaders", Primitive::XLeaders),
    ("leaders", Primitive::Leaders),
    ("unkern", Primitive::Unkern),
    // ETRIP 冲刺：交互模式命令
    ("batchmode", Primitive::BatchMode),
    ("nonstopmode", Primitive::NonstopMode),
    ("scrollmode", Primitive::ScrollMode),
    ("errorstopmode", Primitive::ErrorStopMode),
    // ETRIP 冲刺：\chardef/\countdef/\dimendef/\skipdef/\toksdef
    ("chardef", Primitive::Chardef),
    ("countdef", Primitive::Countdef),
    ("dimendef", Primitive::Dimendef),
    ("skipdef", Primitive::Skipdef),
    ("toksdef", Primitive::Toksdef),
    // ETRIP 冲刺：\hyphenchar<font>=<int>
    ("hyphenchar", Primitive::HyphenChar),
    // ETRIP 冲刺：\delcode<num>=<num>（字符定界符码）
    ("delcode", Primitive::DelCode),
    // ETRIP 冲刺：\muskip/\muskipdef（mu 胶量寄存器）
    ("muskip", Primitive::Muskip),
    ("muskipdef", Primitive::Muskipdef),
    // ETRIP 冲刺：\thinmuskip/\medmuskip/\thickmuskip（muskip 寄存器 0/1/2）
    ("thinmuskip", Primitive::ThinMuskip),
    ("medmuskip", Primitive::MedMuskip),
    ("thickmuskip", Primitive::ThickMuskip),
    // ETRIP 冲刺：\lccode<char>=<num>（小写码表，断字用）
    ("lccode", Primitive::LcCode),
    // ETRIP 冲刺：\advance<寄存器> <增量>（寄存器运算）
    ("advance", Primitive::Advance),
    // ETRIP 冲刺：\hyphenation{...}（断字异常词表）
    ("hyphenation", Primitive::Hyphenation),
    // ETRIP 冲刺：\setbox<n>=<box>（盒子寄存器赋值）
    ("setbox", Primitive::SetBox),
    // ETRIP 冲刺：\parfillskip（段落末行填充胶水）
    ("parfillskip", Primitive::ParFillSkip),
    // ETRIP 冲刺：\␣（control space：输出空格）
    (" ", Primitive::ControlSpace),
    // ETRIP 冲刺：无限阶胶水
    ("hfil", Primitive::HFil),
    ("hfill", Primitive::HFill),
    ("hss", Primitive::HSS),
    ("vfil", Primitive::VFil),
    ("vfill", Primitive::VFill),
    ("vss", Primitive::VSS),
    // TRIP 冲刺：\moveleft/\moveright<dimen><box>（盒子水平位移）
    ("moveleft", Primitive::MoveLeft),
    ("moveright", Primitive::MoveRight),
    // TRIP 冲刺：\accent<8-bit number><字符>（读音符）
    ("accent", Primitive::Accent),
    // TRIP 冲刺：\vfilneg（plain.tex 宏：负 1fil vskip）
    ("vfilneg", Primitive::VFilNeg),
    // TRIP 冲刺：\hfilneg（plain.tex 宏：负 1fil hskip）
    ("hfilneg", Primitive::HFilNeg),
    // TRIP 冲刺：\error（plain.tex 宏：errmessage）
    ("error", Primitive::Error),
    // TRIP 冲刺：\varunit（plain.tex 字体标识符；TRIP L404 引用）
    ("varunit", Primitive::VarUnit),
    // TRIP 冲刺：\xspaceskip（plain.tex 胶水参数；TRIP L410 `\advance\xspaceskip`）
    ("xspaceskip", Primitive::XSpaceSkip),
    // TRIP 冲刺：\spacefactor（活空间因子，L209/L288 赋值、L277 读取）
    ("spacefactor", Primitive::SpaceFactor),
    // TRIP 冲刺：\everymath（数学模式进入注入 token 列表，L411）
    ("everymath", Primitive::EveryMath),
    // TRIP 冲刺：\/（斜体校正，控制符号；L410/L412）
    ("/", Primitive::ItalicCorrection),
    // TRIP 冲刺：\radical<delimiter><math field>（根式原子，L412 everymath 注入）
    ("radical", Primitive::Radical),
    // TRIP 冲刺：\delimiterfactor（内部整数参数；L412 赋值 1600）
    ("delimiterfactor", Primitive::DelimiterFactor),
    // TRIP 冲刺：\escapechar（内部整数参数；控制序列显示用 escape 字符，默认 `\`）
    ("escapechar", Primitive::EscapeChar),
    // TRIP 冲刺：\prevgraf（上一段落行数，只读内部量）
    ("prevgraf", Primitive::PrevGraf),
    // TRIP 冲刺：\errmessage{...}（报错到转录）
    ("errmessage", Primitive::ErrMessage),
    // TRIP 冲刺：页面内部量（\pagetotal/\pagegoal/\predisplaysize 只读；\pagestretch 系可写）
    ("pagetotal", Primitive::PageTotal),
    ("pagegoal", Primitive::PageGoal),
    ("predisplaysize", Primitive::PreDisplaySize),
    ("pagestretch", Primitive::PageStretch),
    ("pagefilstretch", Primitive::PageFilStretch),
    ("pagefillstretch", Primitive::PageFillStretch),
    // TRIP 冲刺：数学原语（存在性测试；简化实现）
    ("mskip", Primitive::MSkip),
    ("mkern", Primitive::MKern),
    ("mathaccent", Primitive::MathAccent),
    ("mathchar", Primitive::MathChar),
    ("delimiter", Primitive::Delimiter),
    ("eqno", Primitive::EqNo),
    ("leqno", Primitive::LeqNo),
    ("abovewithdelims", Primitive::AboveWithDelims),
    ("overwithdelims", Primitive::OverWithDelims),
    ("underline", Primitive::Underline),
    ("overline", Primitive::Overline),
    ("crcr", Primitive::CrCr),
    ("-", Primitive::DiscMinus),
    // TRIP 冲刺：toks 参数与 \insertpenalties
    ("everypar", Primitive::EveryPar),
    ("everyhbox", Primitive::EveryHBox),
    ("everyvbox", Primitive::EveryVBox),
    ("everycr", Primitive::EveryCr),
    ("errhelp", Primitive::ErrHelp),
    ("insertpenalties", Primitive::InsertPenalties),
    ("above", Primitive::Above),
    ("atopwithdelims", Primitive::AtopWithDelims),
    // TRIP 补全批次：tex.web 标准原语 22 个
    ("day", Primitive::Day),
    ("month", Primitive::Month),
    ("year", Primitive::Year),
    ("time", Primitive::Time),
    ("topmark", Primitive::TopMark),
    ("firstmark", Primitive::FirstMark),
    ("botmark", Primitive::BotMark),
    ("splitfirstmark", Primitive::SplitFirstMark),
    ("splitbotmark", Primitive::SplitBotMark),
    ("brokenpenalty", Primitive::BrokenPenalty),
    ("exhyphenpenalty", Primitive::ExHyphenPenalty),
    ("tracingpages", Primitive::TracingPages),
    ("pausing", Primitive::Pausing),
    ("setlanguage", Primitive::SetLanguage),
    ("skewchar", Primitive::SkewChar),
    ("displaywidth", Primitive::DisplayWidth),
    ("everydisplay", Primitive::EveryDisplay),
    ("nullfont", Primitive::NullFont),
    ("outputpenalty", Primitive::OutputPenalty),
    ("pagedepth", Primitive::PageDepth),
    ("pagefilllstretch", Primitive::PageFillLStretch),
    ("pageshrink", Primitive::PageShrink),
    // ETRIP 冲刺：\vsplit<n> to/spread <dimen>（纵向拆分盒子寄存器）
    ("vsplit", Primitive::VSplit),
    // ETRIP 冲刺：\everyjob=<tokens>（作业开始 token 表；暂映射到 toks 0）
    ("everyjob", Primitive::EveryJob),
    // ETRIP 冲刺：\dump（initex 收尾：写 fmt + 结束作业）
    ("dump", Primitive::Dump),
    // M7 fmt：expl3 \__kernel_primitive:NN \dump \tex_dump:D 把原语改名
    // 载入（expl3-code l.339）——latex.ltx 末尾的 \dump 经此路径触发。
    // 缺此注册时 \dump 变 undefined → fmt dump 永远不触发。
    ("tex_dump", Primitive::Dump),
    // ETRIP 冲刺：TeXXeT 方向原语（\TeXXeTstate=1 时创建方向节点）
    ("beginL", Primitive::BeginL),
    ("endL", Primitive::EndL),
    ("beginR", Primitive::BeginR),
    ("endR", Primitive::EndR),
    // e-TeX（M4-5）：\middle<delimiter>（\left...\right 内分隔符）
    ("middle", Primitive::Middle),
    // ETRIP 冲刺：\mark<general text>（mark 节点）；e-TeX \marks<n><general text>
    ("mark", Primitive::Mark),
    ("marks", Primitive::Marks),
    // ETRIP 冲刺：\showbox<n>（显示盒子寄存器内容）
    ("showbox", Primitive::ShowBox),
    // ETRIP 冲刺：\inputlineno（当前输入行号；只读整数）与 \string<token>
    ("inputlineno", Primitive::InputLineNo),
    ("string", Primitive::String_),
    // ETRIP 冲刺：e-TeX 内部只读整数
    ("currentgrouplevel", Primitive::CurrentGroupLevel),
    ("currentgrouptype", Primitive::CurrentGroupType),
    ("lastnodetype", Primitive::LastNodeType),
    // ETRIP 冲刺：\discretionary{pre}{post}{replace}
    ("discretionary", Primitive::Discretionary),
    // ETRIP 冲刺：\insert<num>{<general text>}（insert 节点）与 \vadjust{...}（adjust 节点）
    ("insert", Primitive::Insert),
    ("vadjust", Primitive::VAdjust),
    // ETRIP 冲刺：对齐原语（组种类 6/7）与 \mathchoice{}{}{}{}
    ("valign", Primitive::Valign),
    ("halign", Primitive::Halign),
    ("cr", Primitive::Cr),
    ("noalign", Primitive::NoAlign),
    ("mathchoice", Primitive::MathChoice),
    // ETRIP 冲刺：输出例程/追踪/错误上下文
    ("deadcycles", Primitive::DeadCycles),
    ("tracingmacros", Primitive::TracingMacros),
    ("tracingoutput", Primitive::TracingOutput),
    ("errorcontextlines", Primitive::ErrorContextLines),
    // ETRIP 冲刺：\raise/\lower/\span/\special/\jobname/\vcenter
    ("raise", Primitive::Raise),
    ("lower", Primitive::Lower),
    ("span", Primitive::Span),
    ("special", Primitive::Special),
    ("jobname", Primitive::JobName),
    ("vcenter", Primitive::VCenter),
    // ETRIP 冲刺：\ifinner（内部模式条件）
    ("ifinner", Primitive::IfInner),
    // ETRIP 冲刺：\csname...\endcsname（构造控制序列名）
    ("csname", Primitive::Csname),
    ("endcsname", Primitive::EndCsname),
    // ETRIP 冲刺：模式/盒子/EOF 条件原语
    ("ifvmode", Primitive::IfVMode),
    ("ifhmode", Primitive::IfHMode),
    ("ifmmode", Primitive::IfMMode),
    ("ifeof", Primitive::IfEof),
    ("ifvoid", Primitive::IfVoid),
    ("ifhbox", Primitive::IfHBox),
    ("ifvbox", Primitive::IfVBox),
    // ETRIP 冲刺：寄存器算术
    ("multiply", Primitive::Multiply),
    ("divide", Primitive::Divide),
    // ETRIP 冲刺：e-TeX 只读整数
    ("currentiflevel", Primitive::CurrentIfLevel),
    ("currentiftype", Primitive::CurrentIfType),
    ("currentifbranch", Primitive::CurrentIfBranch),
    // ETRIP 冲刺：\meaning 与 \mathchardef
    ("meaning", Primitive::Meaning),
    ("mathchardef", Primitive::MathCharDef),
    // ETRIP 冲刺：胶水分量查询
    ("gluestretchorder", Primitive::GlueStretchOrder),
    ("glueshrinkorder", Primitive::GlueShrinkOrder),
    ("gluestretch", Primitive::GlueStretch),
    ("glueshrink", Primitive::GlueShrink),
    // ETRIP 冲刺：\showtokens 与 \readline
    ("showtokens", Primitive::ShowTokens),
    ("readline", Primitive::ReadLine),
    // ETRIP 冲刺：字体字符查询与 \showifs
    ("iffontchar", Primitive::IfFontChar),
    ("fontcharwd", Primitive::FontCharWd),
    ("fontcharht", Primitive::FontCharHt),
    ("fontchardp", Primitive::FontCharDp),
    ("fontcharic", Primitive::FontCharIc),
    ("showifs", Primitive::ShowIfs),
    // ETRIP 冲刺：段落形状
    ("parshape", Primitive::Parshape),
    ("parshapelength", Primitive::ParshapeLength),
    ("parshapeindent", Primitive::ParshapeIndent),
    ("parshapedimen", Primitive::ParshapeDimen),
    // ETRIP 冲刺：e-TeX marks 族查询（可展开，返回字符 token 文本）
    ("topmarks", Primitive::TopMarks),
    ("firstmarks", Primitive::FirstMarks),
    ("botmarks", Primitive::BotMarks),
    ("splitfirstmarks", Primitive::SplitFirstMarks),
    ("splittopmarks", Primitive::SplitTopMarks),
    ("splitbotmarks", Primitive::SplitBotMarks),
    // ETRIP 第二波：e-TeX mu 转换原语（可展开）
    ("mutoglue", Primitive::MuToGlue),
    ("gluetomu", Primitive::GlueToMu),
    // ETRIP 第二波：e-TeX 惩罚数组（\interlinepenalties 等）
    ("interlinepenalties", Primitive::InterLinePenalties),
    ("clubpenalties", Primitive::ClubPenalties),
    ("widowpenalties", Primitive::WidowPenalties),
    ("displaywidowpenalties", Primitive::DisplayWidowPenalties),
    // ETRIP 第二波：e-TeX 丢弃物控制整数（misc 数组）
    ("pagediscards", Primitive::PageDiscards),
    ("splitdiscards", Primitive::SplitDiscards),
    ("lostchars", Primitive::LostChars),
    // ETRIP 第二波：盒子复制/拆包原语
    ("copy", Primitive::Copy),
    ("unhbox", Primitive::UnHBox),
    ("unvbox", Primitive::UnVBox),
    ("unhcopy", Primitive::UnHCopy),
    ("unvcopy", Primitive::UnVCopy),
    ("lastbox", Primitive::LastBox),
    // ETRIP 第二波：盒子尺寸查询/赋值
    ("wd", Primitive::Wd),
    ("ht", Primitive::Ht),
    ("dp", Primitive::Dp),
    // ETRIP 第二波：段落/断页参数原语
    ("leftskip", Primitive::LeftSkip),
    ("rightskip", Primitive::RightSkip),
    ("prevdepth", Primitive::PrevDepth),
    ("interlinepenalty", Primitive::InterLinePenalty),
    ("clubpenalty", Primitive::ClubPenalty),
    ("widowpenalty", Primitive::WidowPenalty),
    ("displaywidowpenalty", Primitive::DisplayWidowPenalty),
    // TRIP 冲刺：补充标准参数原语
    ("hangindent", Primitive::HangIndent),
    ("spaceskip", Primitive::SpaceSkip),
    ("tabskip", Primitive::TabSkip),
    ("lastskip", Primitive::LastSkip),
    ("hfuzz", Primitive::Hfuzz),
    ("vfuzz", Primitive::Vfuzz),
    ("boxmaxdepth", Primitive::BoxMaxDepth),
    ("splitmaxdepth", Primitive::SplitMaxDepth),
    ("splittopskip", Primitive::SplitTopSkip),
    ("emergencystretch", Primitive::EmergencyStretch),
    ("displayindent", Primitive::DisplayIndent),
    ("delimitershortfall", Primitive::DelimiterShortfall),
    ("lastkern", Primitive::LastKern),
    // ETRIP 第二波：列表尾操作
    ("unskip", Primitive::UnSkip),
    ("lastpenalty", Primitive::LastPenalty),
    ("unpenalty", Primitive::UnPenalty),
    // ETRIP 第二波：诊断原语
    ("showgroups", Primitive::ShowGroups),
    ("showlists", Primitive::ShowLists),
    // ETRIP 第二波：折行追踪整数参数
    ("tracingparagraphs", Primitive::TracingParagraphs),
    // ETRIP 第二波：对齐模板跳过
    ("omit", Primitive::Omit),
    // TRIP：放大倍数整数参数
    ("mag", Primitive::Mag),
    // TRIP 冲刺：dimen 内部参数（initex 预定义；排版器经 param_changed 镜像）
    ("nulldelimiterspace", Primitive::NullDelimiterSpace),
    ("scriptspace", Primitive::ScriptSpace),
    ("overfullrule", Primitive::OverfullRule),
    ("voffset", Primitive::VOffset),
    ("hoffset", Primitive::HOffset),
    // TRIP 冲刺：\mathcode<num>=<num>（字符数学码表）
    ("mathcode", Primitive::MathCode),
    // TRIP 冲刺：\noboundary（数学字符边界抑制；直通 sink）
    ("noboundary", Primitive::NoBoundary),
    // LaTeX 兼容第八刀：pdfTeX 引擎探测/兼容原语族（语义边界见
    // eqtb/primitive.rs 该族注释与 docs/archive/latex-feasibility.md §15）。
    // 版本值对齐 pdfTeX 1.40.25（TeX Live 2024–2025 世代；
    // latex.ltx engine-check 只要求 v1.40，l3kernel 仅探测存在性）。
    ("pdftexversion", Primitive::PdfTeXVersion),
    ("pdftexrevision", Primitive::PdfTeXRevision),
    ("pdftexbanner", Primitive::PdfTeXBanner),
    // \pdfoutput 默认 0 = DVI 模式（pdfTeX 默认即 0；NTex 输出 DVI，
    // 非 0 值可赋但无 PDF 后端承接——偏差记录在报告 §15.3）
    ("pdfoutput", Primitive::PdfOutput),
    ("pdfshellescape", Primitive::PdfShellEscape),
    ("pdfelapsedtime", Primitive::PdfElapsedTime),
    ("pdfrandomseed", Primitive::PdfRandomSeed),
    ("pdfsetrandomseed", Primitive::PdfSetRandomSeed),
    ("pdfuniformdeviate", Primitive::PdfUniformDeviate),
    // l3kernel 无条件 `\cs_new_eq:NN \__str_if_eq:nn \tex_strcmp:D`
    // （expl3-code L5129）→ \pdfstrcmp 必须可展开，否则 expl3 全篇
    // 字符串比较在首次调用时报"未定义控制序列"
    ("pdfstrcmp", Primitive::PdfStrCmp),
    // `\cs_new_eq:NN \__file_size:n \tex_filesize:D`（L12679）→
    // \pdffilesize 缺失则 \file_full_name:n 的存在性探测瘫痪
    ("pdffilesize", Primitive::PdfFileSize),
    ("pdfcreationdate", Primitive::PdfCreationDate),
    // M9 中文刀 2：源文件输入编码开关（0=bytes 默认，1=UTF-8 解码；
    // 只影响字节→token 入口，TRIP/ETRIP 的 8-bit 口径零影响）
    ("utfinputmode", Primitive::UtfInputMode),
    // M9 中文刀 5：汉字字间断点开关（0=关 默认，非 0=开）。默认关是
    // 硬口径——断点会改变折行结果，TRIP/ETRIP/expl3 必须零影响。
    ("cjkbreakmode", Primitive::CjkBreakMode),
    // \includegraphics 图片管线 Step A：pdfTeX 图片三原语（pdftex.def
    // 的 \Gread@png/\Gread@pdf 唯一尺寸来源；语义见 eqtb/primitive.rs）
    ("pdfximage", Primitive::PdfXImage),
    ("pdflastximage", Primitive::PdfLastXImage),
    ("pdfrefximage", Primitive::PdfRefXImage),
    // 图片管线 Step B：PDF 变换栈（pdftex.def \Gscale@start/\Gscale@end
    // 的底层原语；语义见 eqtb/primitive.rs PdfSave 注释块）
    ("pdfsave", Primitive::PdfSave),
    ("pdfsetmatrix", Primitive::PdfSetMatrix),
    ("pdfrestore", Primitive::PdfRestore),
    ("catcodetable", Primitive::CatcodeTable),
    ("initcatcodetable", Primitive::InitCatcodeTable),
    ("savecatcodetable", Primitive::SaveCatcodeTable),
];

/// `Primitive → 规范名`（`BUILTINS` 反查；tests.rs
/// `primitive_builtins_cover_enum` 保证全变体覆盖且名字唯一）。
///
/// tex.web `show_eqtb`/`print_cmd_chr` 语义：原语槽打印的是**原语的规范名**，
/// 不是被查询 cs 自己的名字——`\let\a\else` 后 `\show\a` 须为 `\a=\else.`、
/// `\meaning\a` 须为 `\else`、`\tracingassigns` 须打 `{into \a=\else}`
/// （pdfTeX 对拍 2026-09-11）。此前渲染成被查询 cs 名（`\a=\a.`），诊断
/// 通道输出假信息，直接促成「\global\let 别名自指」的误判。
pub(crate) fn primitive_name(prim: Primitive) -> &'static str {
    for &(name, p) in BUILTINS.iter() {
        if p == prim {
            return name;
        }
    }
    unreachable!("BUILTINS 覆盖全部 Primitive 变体（tests.rs primitive_builtins_cover_enum）")
}

impl Expander {
    fn register_builtins(&mut self) {
        for (name, prim) in BUILTINS {
            let csid = self.intern.intern(name);
            self.eqtb.set_primitive(csid, prim);
        }
        // \nullfont：内建空字体（TeX 的 null font，无字符；固定字体槽 0）
        let nullfont = self.intern.intern("nullfont");
        self.eqtb.set_font(nullfont, 0);
        // 图片管线 Step A 配套：\pdfpagewidth/\pdfpageheight——pdfTeX 内部
        // dimen，PDF 模式下 latex.ltx \begin{document} 以 \paperwidth/
        // \paperheight 写入。NTex 输出 DVI（页尺寸由 DVI 驱动决定），这里只
        // 提供"可写可读"的存储：绑到寄存器表尾部两个实际分配永不触及的槽，
        // 赋值/数字读取/\the 全走既有寄存器机器，不新增 ParamKind（那会波及
        // 排版器参数面与 param_changed 镜像）。
        for (name, idx) in [("pdfpagewidth", 32767usize), ("pdfpageheight", 32766)] {
            let csid = self.intern.intern(name);
            self.eqtb
                .set_register(csid, crate::register::RegKind::Dimen, idx);
        }
    }
}
