impl Expander {
    fn register_builtins(&mut self) {
        const BUILTINS: [(&str, Primitive); 178] = [
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
            // M4-5 e-TeX 展开扩展
            ("protected", Primitive::Protected),
            ("ifdefined", Primitive::IfDefined),
            ("ifcsname", Primitive::IfCsname),
            ("unless", Primitive::Unless),
            ("numexpr", Primitive::NumExpr),
            ("detokenize", Primitive::Detokenize),
            ("unexpanded", Primitive::Unexpanded),
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
            ("ifprimitive", Primitive::IfPrimitive),
            ("scantokens", Primitive::Scantokens),
            // ETRIP 冲刺：TeX 内部整数参数
            ("endlinechar", Primitive::EndlineChar),
            ("newlinechar", Primitive::NewlineChar),
            ("defaulthyphenchar", Primitive::DefaultHyphenChar),
            ("defaultskewchar", Primitive::DefaultSkewChar),
            // ETRIP 冲刺：宏定义前缀与变体
            ("outer", Primitive::Outer),
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
            // ETRIP 冲刺：\vsplit<n> to/spread <dimen>（纵向拆分盒子寄存器）
            ("vsplit", Primitive::VSplit),
            // ETRIP 冲刺：\everyjob=<tokens>（作业开始 token 表；暂映射到 toks 0）
            ("everyjob", Primitive::EveryJob),
            // ETRIP 冲刺：\dump（initex 收尾：写 fmt + 结束作业）
            ("dump", Primitive::Dump),
        ];
        for (name, prim) in BUILTINS {
            let csid = self.intern.intern(name);
            self.eqtb.set_primitive(csid, prim);
        }
        // \nullfont：内建空字体（TeX 的 null font，无字符；固定字体槽 0）
        let nullfont = self.intern.intern("nullfont");
        self.eqtb.set_font(nullfont, 0);
    }
}
