use super::*;

    /// 表达式**因子位**的 get_x_token 前瞻：`( <expr> )` 因子臂必须对**展开
    /// 产物**判定（etex.web scan_expr 取 token 循环同为 get_x_token）。expl3
    /// `\int_div_truncate:nn` 让 `\__int_div_truncate:NwNw` 在因子位展开出
    /// `( ... )`；旧实现因子位直读一个 token、非 `(` 即进 scan_int，`(` 落入
    /// 十进制循环 → "Missing number, treated as zero" + "Missing ) inserted"。
    #[test]
    fn expr_factor_expands_before_paren_dispatch() {
        let src = concat!(
            "\\def\\paren{(2+3)} %\n",
            "\\edef\\ga{\\number\\numexpr\\paren*4\\relax} %\n",
            "\\immediate\\write16{GA=\\ga}"
        );
        let (_r, transcript) = run_transcript(src);
        assert!(
            transcript.contains("GA=20"),
            "因子位宏展开产物中的 `(` 应按括号因子处理：{transcript}"
        );
    }

    /// 表达式**运算符前瞻**须跳过空格（`skip_spaces`）——expl3
    /// `\__int_div_truncate:NwNw` 的体含空格：
    /// `( #1#2 ... ( #3#4 - 1 ) / 2 ) / #3#4`。`)` 之后的空格若不跳过，
    /// 乘/除层前瞻把空格当终结符、`/ 2` 残留流中（值 41 + "Missing )"）。
    /// 同时运算符位的 `\if_meaning:w` 条件开始按 get_x_token 就地求值。
    #[test]
    fn expr_operator_peek_skips_spaces_and_evals_cond() {
        // expl3-code l.6652-6673 `\int_div_truncate:nn`/`\__int_div_truncate:NwNw`
        // 机制级复刻（`\__int_sep:` = `\let`，两级别名链同 expl3）。
        //
        // 第二十六刀勘误：本测旧版两处失真致门禁**假阳性**——
        // (a) 缺 `\let\if_meaning:w\ifx` 别名组（§32.5 三件套：初表全 cat12 下
        //     `\if_meaning:w` 未定义，条件体根本不执行，错误形态是
        //     "Undefined control sequence \if_meaning:w"）；
        // (b) 断言 `contains("GA=1")` 被 `GA=10(140-1+-(-1-100-1)/2)/100` 的
        //     **前缀**满足——同构源 /tmp/r25/mech19.tex 在探针路径（Typesetter
        //     + write16）与裸 Expander 均实测 `140(100-1)/2)/100`（真 TeX = 1），
        //     而本测长年绿。现改为输出行**逐行相等**断言。
        let src = concat!(
            "\\catcode`\\_=11 \\catcode`\\:=11 %\n",
            "\\let\\if_meaning:w\\ifx \\let\\else:\\else \\let\\fi:\\fi \\let\\or:\\or %\n",
            "\\let\\texnumD\\number \\let\\iv\\texnumD %\n",
            "\\let\\texnumE\\numexpr \\let\\ev\\texnumE %\n",
            "\\let\\texlet\\let \\let\\sep\\texlet \\let\\eend\\relax %\n",
            "\\long\\def\\auxb#1#2\\sep#3#4\\sep",
            "{\\if_meaning:w0#1 0\\else:(#1#2\\if_meaning:w-#1+\\else:-\\fi:",
            "(\\if_meaning:w-#3-\\fi:#3#4-1)/2)\\fi:/#3#4} %\n",
            "\\edef\\ga{\\iv\\ev\\expandafter\\auxb \\iv\\ev 140 \\expandafter\\sep ",
            "\\iv\\ev 100 \\sep \\eend} %\n",
            "\\immediate\\write16{GA=\\ga}"
        );
        let (_r, transcript) = run_transcript(src);
        let ga = transcript
            .lines()
            .find(|l| l.starts_with("GA="))
            .unwrap_or_else(|| panic!("缺 GA 输出行：{transcript}"));
        assert_eq!(
            ga, "GA=1",
            "含空格/内嵌条件的表达式应得 1（真 TeX 对照）：{transcript}"
        );
        assert!(
            !transcript.contains("Missing )") && !transcript.contains("Missing number"),
            "表达式在空格/条件处提前收口：{transcript}"
        );
    }

    /// 表达式因子位对**已求值条件的 `\fi`** 的消费（第二十六刀主修）。
    ///
    /// `\numexpr 140\if_meaning:w-1+\else:-\fi:(100-1)/2\relax`：`\if_meaning:w`
    /// 在数字扫描的十进制循环就地求值（`\ifx - 1` 假 → skip_ahead 同步吃
    /// `+`/`\else:`、帧留栈等 `\fi`），`-` 落回运算符位、`\fi:` 落到**因子位**。
    /// 旧实现因子位不消费 `\fi`：被当因子放回 scan_number、帧由符号循环弹出、
    /// `( … )` 撞十进制循环报 "Missing number" → 表达式在 `-` 后收 0、外层 `(`
    /// 配不上 `)` 再报 "Missing )" → 值失真为 `140(100-1)/2)/100`。
    /// T1/T2/T3 对照值取自 pdfTeX 1.40.29（TeX Live 2026）实测。
    #[test]
    fn expr_factor_consumes_fi_after_evaluated_cond() {
        let src = concat!(
            // §32.5 三件套：catcode `:`/`_` = 11 + `\else:`/`\fi:` 别名组
            "\\catcode`\\_=11 \\catcode`\\:=11 %\n",
            "\\let\\if_meaning:w\\ifx \\let\\else:\\else \\let\\fi:\\fi %\n",
            "\\immediate\\write16{T1=\\number\\numexpr ",
            "140\\if_meaning:w-1+\\else:-\\fi:(100-1)/2\\relax} %\n",
            "\\immediate\\write16{T2=\\number\\numexpr ",
            "140\\if_meaning:w-1+\\else:-\\fi: 2\\relax} %\n",
            "\\immediate\\write16{T3=\\number\\numexpr",
            "(\\if_meaning:w-1-\\fi: 100-1)/2\\relax}"
        );
        let (_r, transcript) = run_transcript(src);
        for (tag, want) in [("T1=", "90"), ("T2=", "138"), ("T3=", "50")] {
            let line = transcript
                .lines()
                .find(|l| l.starts_with(tag))
                .unwrap_or_else(|| panic!("缺 {tag} 输出行：{transcript}"));
            assert_eq!(line, format!("{tag}{want}"), "真 TeX 对照：{transcript}");
        }
        assert!(!transcript.contains("! "), "零错误契约：{transcript}");
    }

    /// 因子位**游离** `\fi` 的放回契约（TRIP L82 同族，第二十六刀补因子位侧）。
    /// 无帧可归属时 `\fi` 须原样留给外层条件机：本扫描按 Missing number 收场，
    /// 残留的单枚 `\fi` 由主循环报一次 "Extra \fi"（与 HEAD 行为一致）；若因子
    /// 位取 token 自放回（调用方 expr_factor 还会再放回）则 token 翻倍、
    /// "Extra \fi" 变两次。
    #[test]
    fn expr_factor_keeps_free_fi_single() {
        let src = concat!(
            "\\count0=\\numexpr\\fi 7\\relax %\n",
            "\\immediate\\write16{C=\\count0}"
        );
        let (_r, transcript) = run_transcript(src);
        assert!(
            transcript.contains("Missing number"),
            "游离 \\fi 应触发 Missing number 恢复：{transcript}"
        );
        assert!(
            transcript.matches("Extra \\fi").count() <= 1,
            "\\fi 不得翻倍（因子位放回至多一次 Extra \\fi）：{transcript}"
        );
        let line = transcript
            .lines()
            .find(|l| l.starts_with("C="))
            .unwrap_or_else(|| panic!("缺 C= 输出行：{transcript}"));
        assert_eq!(line, "C=0", "Missing number 恢复取 0：{transcript}");
    }
    #[test]
    fn expr_terminator_peek_expands_expandable_token() {
        // 表达式终结符位是 **get_x_token** 位置（etex.web scan_expr 运算符循环
        // `get_x_token; if cur_tok<>plus/minus then back_input`）：可展开 token
        // 展开一次后以其产物首 token 判定"运算符/终结符"，终结符放回。
        //
        // expl3 的 `\int_value:w \__int_eval:w <n> \exp_after:wN \__int_sep:`
        // （`\int_step_function:nnnN`、`\__char_generate_aux:w`）依赖这一步：
        // `\__int_sep:`（\let 别名，不可展开）落为终结符、后续 `\int_value:w`
        // 的求值产物排在其后。get_token 直读则 `\expandafter` 原样放回，调用方
        // 的定界实参扫描把 `\expandafter`+下一值整段吞进同一个实参——expl3-code
        // l.9364 `\char_generate:nn` bootstrap 区 1812 条
        // "Missing = inserted for \ifnum"（`\ifnum` 关系符位读到表达式 `+`）。
        //
        // 原语级最小形：\sep = \let 别名（expl3 \__int_sep: 的真身）。
        // 注意：`%` 后必须真实换行（无换行会把其余输入全部注释掉）。
        let src = concat!(
            "\\def\\body#1{[#1]}%\n",
            "\\let\\sep=\\let%\n",
            "\\def\\stepfun #1#2#3{%\n",
            "\\expandafter\\stepw\\number\\numexpr #1 \\expandafter\\sep%\n",
            "\\number\\numexpr #2 \\expandafter\\sep%\n",
            "\\number\\numexpr #3 \\sep}%\n",
            "\\def\\stepw #1\\sep #2\\sep #3\\sep #4{\\stepn >#1\\sep{#2}{#3}{#4}}%\n",
            "\\def\\stepn #1#2\\sep #3#4#5{%\n",
            "\\ifnum #2 #1 #4 \\relax B\\else%\n",
            "#5{#2}%\n",
            "\\expandafter\\stepn\\expandafter#1\\number\\numexpr #2 + #3 \\sep {#3}{#4}{#5}%\n",
            "\\fi}%\n",
            "\\stepfun {0}{1}{3}\\body%\n",
        );
        assert_eq!(expand(src).unwrap(), "[0][1][2][3]B");
        // 同位直写（无 \expandafter）：`\let` 别名原语作终结符，不展开、放回，
        // 交还调用方的定界实参匹配（expl3 `\__int_sep:` 的用法）
        assert_eq!(
            expand("\\def\\g #1\\sep{[#1]}\\expandafter\\g\\number\\numexpr 1 + 1 \\sep").unwrap(),
            "[2]"
        );
        // 终结符是可展开宏：展开后以其产物首 token 判定（tex.web get_x_token）
        assert_eq!(
            expand("\\def\\plus{+2}\\number\\numexpr 1 \\plus \\relax X").unwrap(),
            "3X"
        );
    }

    #[test]
    fn expr_relax_absorbed_once_at_add_level() {
        // `\relax` 只在**加法层**前瞻吸收一次：乘法层若也吸收，加法层的第二次
        // 前瞻就越过表达式终点、把外侧可展开 token（`\the\skip0`）展开吞进
        // 表达式——`\skip0` 赋值正确而 `\the` 产物 `0pt` 泄漏为排版文本。
        assert_eq!(
            expand(r"\skip0=\glueexpr 1pt plus 2pt - 0.5pt \relax\the\skip0").unwrap(),
            "0.5pt plus 2.0pt"
        );
        // 无运算符同形（乘法层前瞻直接落 `\relax`）
        assert_eq!(
            expand(r"\skip0=\glueexpr 1pt \relax\the\skip0").unwrap(),
            "1.0pt"
        );
        assert_eq!(
            expand(r"\count0=\numexpr 1 + 2 \relax\the\count0").unwrap(),
            "3"
        );
    }

    #[test]
    fn ifnum_gluestretchorder_repro() {
        // ETRIP etrip.tex L938：\ifnum\gluestretchorder#5=#1
        assert_eq!(
            expand("\\ifnum\\gluestretchorder1ptminus0fil=0 yes\\else no\\fi").unwrap(),
            "yes"
        );
    }

    #[test]
    fn expr_cross_group_and_glue_preservation() {
        // ── eTeX 表达式跨组（etrip L880-888 运算符优先级段；scan_number/scan_dimen
        // 组处理 + 正号放回 + glueexpr 优先级修复的回归）──
        // \numexpr{1+}{2*3} = 1 + 2*3 = 7（组内运算符放回、组 } 消费）
        assert_eq!(expand("\\the\\numexpr{1+}{2*3}").unwrap(), "7");
        // \glueexpr{7pt+}{12pt/4} = 7pt + 12pt/4 = 10pt（跨组 + 优先级：/ 绑定项）
        assert_eq!(expand("\\the\\glueexpr{7pt+}{12pt/4}").unwrap(), "10.0pt");
        assert_eq!(
            expand("\\ifdim\\glueexpr{7pt+}{12pt/4}=10pt yes\\else no\\fi").unwrap(),
            "yes"
        );
        // \1 宏实参收集场景（etrip L879）：#3={1+} #4={2*3} 组实参展开后表达式求值
        assert_eq!(
            expand("\\def\\1#1#2#3#4{#1#2#3#4=#2#3(#4)\\else X\\fi}\\1\\ifnum\\numexpr{1+}{2*3}")
                .unwrap(),
            ""
        );
        // ── 胶水寄存器 fil 赋值 + \the 读回（\relax 终止单位扫描，避免 \the 被吞）──
        assert_eq!(
            expand("\\skip43=4pt plus 3fil\\relax\\the\\skip43").unwrap(),
            "4.0pt plus 3.0fil"
        );
        // ── 括号表达式 + 前导量（etrip L869-873 表达式段逐项）──
        assert_eq!(
            expand("\\skip43=4pt plus 3fil\\relax\\the\\glueexpr(\\skip43)+3pt").unwrap(),
            "7.0pt plus 3.0fil"
        );
        assert_eq!(
            expand("\\count43=2\\skip43=4pt plus 3fil\\relax\\the\\dimexpr\\skip43+\\count43pt")
                .unwrap(),
            "6.0pt"
        );
        assert_eq!(
            expand(
                "\\count43=2\\skip43=4pt plus 3fil\\relax\\the\\dimexpr(\\skip43)+(\\count43pt)"
            )
            .unwrap(),
            "6.0pt"
        );
        assert_eq!(
            expand("\\count43=2\\skip43=4pt plus 3fil\\relax\\the\\glueexpr\\skip43/\\count43")
                .unwrap(),
            "2.0pt plus 3.0fil"
        );
        assert_eq!(
            expand("\\muskip43=5mu minus 1mu\\relax\\the\\muexpr(\\muskip43)+3muplus1fill")
                .unwrap(),
            "8.0mu plus 1.0fill minus 1.0mu"
        );
        assert_eq!(
            expand("\\count43=2\\skip43=4pt plus 3fil\\relax\\the\\glueexpr\\skip43*2/3")
                .unwrap(),
            "2.66667pt plus 3.0fil"
        );
        // ── 表达式单独用（can't use 测试）后 \let 定义不受污染（etrip L764-766）──
        // \numexpr 等裸用报 "can't use" 不输出 0（修复前输出 "0" 污染）；
        // \let\9=\relax 正常定义 → \ifx 为 T
        {
            let mut e = Expander::new();
            e.set_sink(Box::new(VecSink::default()));
            e.run_source(
                "\\numexpr \\dimexpr \\glueexpr \\muexpr \\let\\9=\\relax \\ifx\\9\\relax T\\else F\\fi",
            )
            .unwrap();
            let sink = e.take_sink();
            let mut sink = sink;
            let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
            assert_eq!(
                sink.transcript.matches("can't use").count(),
                4,
                "四个表达式原语裸用都应报 can't use：{:?}",
                sink.transcript
            );
            // output 在 VecSink.tokens（take_sink 后 e.output() 为空）
            let out: String = sink
                .tokens
                .iter()
                .filter_map(|t| t.charcode().and_then(char::from_u32))
                .collect();
            assert_eq!(out, "T", "\\9 应保持 \\relax 定义（输出 T 而非 0T）");
        }
        // 宏参数 token 的表达式求值：\def\m#1#2{\the\numexpr#1#2}\m{1+}{2*3}
        // #1=[1,+] #2=[2,*,3] → \numexpr 1+ 2*3 = 7（+/* 是宏参数 token）
        let m1 = expand("\\def\\m#1#2{\\the\\numexpr#1#2}\\m{1+}{2*3}");
                assert_eq!(m1.unwrap(), "7");
        // l.880 精确场景：\1\ifnum\numexpr{1+}{2*3}（\1 体含 \else 分支）
        let m2 = expand("\\def\\1#1#2#3#4{#1#2#3#4=#2#3(#4)\\else X\\fi}\\1\\ifnum\\numexpr{1+}{2*3}");
                assert_eq!(m2.unwrap(), "");
        // \noexpand 定义的 \1（etrip L32）是否干扰后续 \def\1 覆盖
        let m3 = expand(
            "\\def\\noexpand\\1{\\number\\eTeXversion.#1}}\\1}\\def\\1#1#2#3#4{OK}\\1 a b c d",
        );
                assert_eq!(m3.unwrap(), "OK");
        // L32 定义 + l.880 场景组合
        let m4 = expand(
            "\\def\\noexpand\\1{\\number\\eTeXversion.#1}}\\1}\\def\\1#1#2#3#4{#1#2#3#4=#2#3(#4)\\else X\\fi}\\1\\ifnum\\numexpr{1+}{2*3}",
        );
                assert_eq!(m4.unwrap(), "");
        // 实参收集诊断：\1 的 #2#3#4 实际 token（\string 序列化）
                        // \def 定义体内含 \else：是否报 Extra \else（l.880 前的 L1595）
                        {
            let mut e6 = Expander::new();
            e6.set_sink(Box::new(VecSink::default()));
            e6.run_source("\\def\\1#1#2#3#4{#1#2#3#4=#2#3(#4)\\else X\\fi}\\relax")
                .unwrap();
            let sink6 = e6.take_sink();
            let mut sink6 = sink6;
            let sink6 = sink6.as_any_mut().downcast_mut::<VecSink>().unwrap();
                        assert!(
                !sink6.transcript.contains("Extra \\else"),
                "\\def 体内 \\else 不应报 Extra：{:?}",
                sink6.transcript
            );
        }
        // 条件栈非空时 \def 体含 \else：是否报 Extra \else（全量 l.880 前状态）
                        {
            let mut e7 = Expander::new();
            e7.set_sink(Box::new(VecSink::default()));
            e7.run_source(
                "\\iftrue\\relax\\def\\1#1#2#3#4{#1#2#3#4=#2#3(#4)\\else X\\fi}\\fi\\relax",
            )
            .unwrap();
            let sink7 = e7.take_sink();
            let mut sink7 = sink7;
            let sink7 = sink7.as_any_mut().downcast_mut::<VecSink>().unwrap();
                        assert!(
                !sink7.transcript.contains("Extra \\else"),
                "条件内 \\def 体 \\else 不应报 Extra：{:?}",
                sink7.transcript
            );
        }
        // 最小复现：\ifnum 的 RHS \numexpr 后直接跟 \else（\1 体模式）
                        {
            let mut e8 = Expander::new();
            e8.set_sink(Box::new(VecSink::default()));
            e8.run_source("\\ifnum0=\\numexpr1+1\\else X\\fi").unwrap();
            let sink8 = e8.take_sink();
            let mut sink8 = sink8;
            let sink8 = sink8.as_any_mut().downcast_mut::<VecSink>().unwrap();
                        assert!(
                !sink8.transcript.contains("Extra \\else"),
                "\\numexpr 后 \\else 不应报 Extra：{:?}",
                sink8.transcript
            );
        }
        // hex 版本：\ifnum0=\numexpr"3FFFFFFF/"7FFFFFFF\else X\fi（\1 体模式）
        {
            let mut e9 = Expander::new();
            e9.set_sink(Box::new(VecSink::default()));
            e9.run_source("\\ifnum0=\\numexpr\"3FFFFFFF/\"7FFFFFFF\\else X\\fi")
                .unwrap();
            let sink9 = e9.take_sink();
            let mut sink9 = sink9;
            let sink9 = sink9.as_any_mut().downcast_mut::<VecSink>().unwrap();
                        assert!(
                !sink9.transcript.contains("Extra \\else"),
                "hex \\numexpr 后 \\else 不应报 Extra：{:?}",
                sink9.transcript
            );
        }
    }

    #[test]
    fn gluestretchorder_delim_macro_repro() {
        // （调用末尾空格是 #5 的定界符，对应 etrip 中行尾换行→空格）
        let src = "\\def\\1#1#2pt#3#4pt#5 {\\ifnum\\gluestretchorder#5=#1 T\\else F\\fi}\\100pt10pt1ptminus0fil ";
        assert_eq!(expand(src).unwrap(), "T");
        // 换行（行尾）作为 #5 定界：真实 etrip 场景
        let src2 = "\\def\\1#1#2pt#3#4pt#5 {\\ifnum\\gluestretchorder#5=#1 T\\else F\\fi}\\100pt10pt1ptminus0fil\n";
        assert_eq!(expand(src2).unwrap(), "T");
    }

    #[test]
    fn etrip_precedence_section_repro() {
        // etrip L866-891 组合段：前置表达式段（\skip44/\muskip44/\dimen44 赋值）
        // + 运算符优先级段（\def\1 + \1\ifnum\numexpr{1+}{2*3} 等）。
        // 全量上下文 l.880 报 "Missing = inserted for \ifnum"（孤立段复现通过）。
        let Some(src) = read_fixture("fixtures/etrip/etrip.tex") else {
            return;
        };
        let lines: Vec<&str> = src.lines().collect();
        // 前置：\typeout/\error/\empty/\space 宏定义（etrip 顶部）
        let pre = r"\def\empty{} \def\space{ }
\def\typeout{\immediate\write15 }
\def\error#1{\immediate\write15{Bug in your e-TeX implementation!}\immediate\write15 }";
        // 前置状态段（L767-773 \count43/\skip43/\muskip43 赋值）+ L866-891 表达式段 + 优先级段
        let body = format!(
            "{}\n{}\n",
            lines[766..773].join("\n"),
            lines[865..891].join("\n")
        );
        // 二分 2：L700-891 全段（含 \numexpr 裸用段 + parshape + \1 定义）
        let body2 = format!("{}\n", lines[765..891].join("\n"));
        let full2 = format!("{pre}\n{body2}\n");
        let mut e2 = Expander::new();
        e2.set_sink(Box::new(VecSink::default()));
        e2.run_source(&full2).unwrap();
        let sink2 = e2.take_sink();
        let mut sink2 = sink2;
        let sink2 = sink2.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert!(
            !sink2.transcript.contains("Missing = inserted"),
            "L766-891 段不应报 Missing = inserted：{:?}",
            sink2.transcript
        );
        let full = format!("{pre}\n{body}\n");
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source(&full).unwrap();
        let sink = e.take_sink();
        let mut sink = sink;
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert!(
            !sink.transcript.contains("Missing = inserted"),
            "l.880 不应报 Missing = inserted：{:?}",
            sink.transcript
        );
    }

    #[test]
    fn etrip_full_gluestretchorder_section() {
        // 复现 etrip.tex mutoglue 段（L902-963）+ gluestretchorder 段：
        // 前段复杂表达式（\\2=--\\gluetomu--\\glueexpr(...)）可能污染后续 \\ifnum 扫描
        let Some(src) = read_fixture("fixtures/etrip/etrip.tex") else {
            return;
        };
        let lines: Vec<&str> = src.lines().collect();
        // 二分：gluestretchorder 段逐行扩展，定位产生 wrong glue 的 \1 调用
        let end = std::env::var("GSO_END").ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or(963);
        // 末尾补换行：模拟行尾（\1 宏 #5 参数的空格定界符）
        let body = format!("{}\n", lines[928..end].join("\n"));
        let pre = r"\def\empty{} \def\space{ }
\def\typeout#1{\immediate\write15{#1}}
\def\error#1{\immediate\write15{Bug in your e-TeX implementation!}\immediate\write15 }
\chardef\zero=0\chardef\one=1\chardef\two=2
\countdef\ctmp=255 \countdef\cndx=254
\begingroup
\skip1=\mutoglue1muplus-2muminus-3fil
\muskip1=\gluetomu1ptplus-2ptminus-3fil
\skip2=\mutoglue-4muplus5fillminus6filll
\muskip2=\gluetomu-4ptplus5fillminus6filll
";
        let src = format!("{pre}\n{body}\n");
        // 用 VecSink 捕获 write15 转录，确认 wrong glue 是否在 expand 环境复现
        let mut e = Expander::new();
        let sink = VecSink::default();
        e.set_sink(Box::new(sink));
        if let Err(err) = e.run_source(&src) {
            let sink = e.take_sink();
            let mut sink = sink;
            let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
            eprintln!("transcript tail:\n{}", tail(&sink.transcript, 500));
            panic!("运行失败: {err}");
        }
        let sink = e.take_sink();
        let mut sink = sink;
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        eprintln!("transcript:\n{}", sink.transcript);
        assert!(
            !sink.transcript.contains("wrong glue"),
            "不应有 wrong glue 输出"
        );
    }

    #[test]
    fn glueexpr_muexpr_relax_repro() {
        // etrip.tex L949：\glueexpr\mutoglue\muexpr\gluetomu\skip5\9\9 嵌套表达式
        let src = "\\def\\9{\\relax}\\skip5=1ptminus0fil\\ifnum\\gluestretchorder\\glueexpr\\mutoglue\\muexpr\\gluetomu\\skip5\\9\\9=0 T\\else F\\fi";
        assert_eq!(expand(src).unwrap(), "T");
    }

    #[test]
    fn glue_order_value_semantics() {
        // 必修项② 销账锁（2026-09-03）：\gluestretchorder/\glueshrinkorder 值语义。
        // ① 无阶胶水 → 0；\gluestretch/\glueshrink 返回分量 sp 值（\ifdim 尺寸上下文）
        let src = r"\skip7=2pt plus 3pt minus 4pt\relax";
        assert_eq!(expand(&format!("{src}\\number\\gluestretchorder\\skip7")).unwrap(), "0");
        assert_eq!(expand(&format!("{src}\\number\\glueshrinkorder\\skip7")).unwrap(), "0");
        assert_eq!(
            expand(&format!("{src}\\ifdim\\gluestretch\\skip7=3pt T\\else F\\fi")).unwrap(),
            "T"
        );
        assert_eq!(
            expand(&format!("{src}\\ifdim\\glueshrink\\skip7=4pt T\\else F\\fi")).unwrap(),
            "T"
        );
        // ② 阶：fil=1 fill=2 filll=3；0 分量带阶保留（TeX：阶与分量独立存储）
        let src = r"\skip7=1pt plus 0fill minus 0filll\relax";
        assert_eq!(expand(&format!("{src}\\number\\gluestretchorder\\skip7")).unwrap(), "2");
        assert_eq!(expand(&format!("{src}\\number\\glueshrinkorder\\skip7")).unwrap(), "3");
        assert_eq!(
            expand(&format!("{src}\\ifdim\\gluestretch\\skip7=0pt T\\else F\\fi")).unwrap(),
            "T"
        );
        // ③ 负分量 + 阶：minus -3fil → shrink=-3 且阶 fil（参考 etrip.log
        // {into \muskip1=1.0mu plus -2.0mu minus -3.0fil} 同构）
        let src = r"\skip7=1pt plus -2pt minus -3fil\relax";
        assert_eq!(expand(&format!("{src}\\number\\gluestretchorder\\skip7")).unwrap(), "0");
        assert_eq!(expand(&format!("{src}\\number\\glueshrinkorder\\skip7")).unwrap(), "1");
        assert_eq!(
            expand(&format!("{src}\\ifdim\\gluestretch\\skip7=-2pt T\\else F\\fi")).unwrap(),
            "T"
        );
        // ④ 前导量表达式/寄存器链（\1 宏 etrip L937 场景的等价最小式）
        let src = r"\skip5=1ptminus0fil\muskip5=\gluetomu\skip5\relax";
        assert_eq!(
            expand(&format!("{src}\\number\\glueshrinkorder\\mutoglue\\muskip5")).unwrap(),
            "1"
        );
        assert_eq!(
            expand(&format!("{src}\\ifdim\\glueshrink\\mutoglue\\muskip5=0pt T\\else F\\fi"))
                .unwrap(),
            "T"
        );
        // ⑤ 负号前缀：-\gluestretchorder 对阶取负（int 上下文 negate）
        let src = r"\skip7=1pt plus 2fill\relax";
        assert_eq!(
            expand(&format!("{src}\\number-\\gluestretchorder\\skip7")).unwrap(),
            "-2"
        );
    }

    #[test]
    fn glue_order_parsing_and_queries() {
        // 阶后缀解析 + \the\skip 显示（fil/fill/filll；pdfTeX：0 分量省略、
        // 且阶词后的 `\the` 需 `\relax` 隔离避免被 get_x_token 展开吞参数）
        assert_eq!(
            expand("\\skip5=1ptminus0fil\\relax\\the\\skip5").unwrap(),
            "1.0pt"
        );
        assert_eq!(
            expand("\\skip6=1ptplus3fillminus0filll\\relax\\the\\skip6").unwrap(),
            "1.0pt plus 3.0fill"
        );
        // \gluestretchorder/\glueshrinkorder（整数上下文）
        assert_eq!(
            expand("\\skip5=1ptminus0fil\\number\\gluestretchorder\\skip5\\number\\glueshrinkorder\\skip5").unwrap(),
            "01"
        );
        assert_eq!(
            expand("\\skip6=1ptplus3fill\\relax\\number\\gluestretchorder\\skip6").unwrap(),
            "2"
        );
        // \gluestretch/\glueshrink（尺寸上下文）
        assert_eq!(
            expand("\\skip6=1ptplus3fill\\ifdim\\gluestretch\\skip6=3pt yes\\else no\\fi").unwrap(),
            "yes"
        );
        assert_eq!(
            expand("\\skip6=1ptplus3fill\\ifdim\\glueshrink\\skip6=0pt yes\\else no\\fi").unwrap(),
            "yes"
        );
        // skipdef 绑定 cs 也可作为胶水参数
        assert_eq!(
            expand("\\skipdef\\S=7\\skip7=2ptplus1fil\\relax\\number\\gluestretchorder\\S").unwrap(),
            "1"
        );
    }

    #[test]
    fn numexpr_basic_arithmetic() {
        assert_eq!(expand(r"\the\numexpr 2+3*4 \relax").unwrap(), "14");
        assert_eq!(expand(r"\the\numexpr 10/3 \relax").unwrap(), "3");
        assert_eq!(expand(r"\the\numexpr 20-7 \relax").unwrap(), "13");
    }

    #[test]
    fn numexpr_with_register() {
        assert_eq!(
            expand(r"\count0=7\the\numexpr \count0*2 \relax").unwrap(),
            "14"
        );
    }

    #[test]
    fn numexpr_in_ifnum() {
        assert_eq!(
            expand(r"\ifnum\numexpr 2*3 \relax > 5 yes\else no\fi").unwrap(),
            "yes"
        );
    }

    // ---------- M4-5 e-TeX 扩展：\dimexpr/\glueexpr/\ifprimitive/\scantokens ----------

    #[test]
    fn dimexpr_basic_arithmetic() {
        assert_eq!(expand(r"\the\dimexpr 1pt+2pt \relax").unwrap(), "3.0pt");
        assert_eq!(expand(r"\the\dimexpr 10pt-2.5pt \relax").unwrap(), "7.5pt");
        assert_eq!(expand(r"\the\dimexpr -1pt+2pt \relax").unwrap(), "1.0pt");
    }

    #[test]
    fn dimexpr_in_dimen_assignment() {
        // \dimen0=\dimexpr...：scan_dimen 识别 \dimexpr 原语
        assert_eq!(
            expand(r"\dimen0=\dimexpr 1pt+2pt \relax\the\dimen0").unwrap(),
            "3.0pt"
        );
    }

    #[test]
    fn glueexpr_basic_and_last_stretch_wins() {
        // width 求和；stretch/shrink 值求和，无穷阶取最后一个非零分量项的阶
        assert_eq!(
            expand(r"\the\glueexpr 1pt plus 2pt + 3pt minus 1pt \relax").unwrap(),
            "4.0pt plus 2.0pt minus 1.0pt"
        );
        // stretch 值求和（非"最后一个覆盖"）：2pt + 4pt = 6pt
        assert_eq!(
            expand(r"\the\glueexpr 1pt plus 2pt + 3pt plus 4pt \relax").unwrap(),
            "4.0pt plus 6.0pt"
        );
    }

    #[test]
    fn glueexpr_in_skip_assignment() {
        // 减法项：stretch 符号随项翻转
        assert_eq!(
            expand(r"\skip0=\glueexpr 1pt plus 2pt - 0.5pt \relax\the\skip0").unwrap(),
            "0.5pt plus 2.0pt"
        );
    }

    // ==== 临时复现：\muexpr 行为校准（对照 pdfTeX；待并入正式测试后删） ====
    #[test]
    fn tmp_muexpr_repro() {
        for (name, src) in [
            // 前导量合法：mu 上下文 + muexpr 输出 → 无 Incompatible
            ("the_muexpr", r"\muskip43=\muexpr(5muminus1mu)\relax\the\muexpr\muskip43"),
            // 直接赋值 + \the 显示 "5.0mu minus 1.0mu"
            ("muskip_the", r"\muskip43=5mu minus 1mu\the\muskip43"),
            ("quot5c", r#"\def\9{\relax}\ifnum-6=\glueexpr\muexpr32mu/"10000\9/-5 T\else F\fi"#),
            ("quot5d", r#"\def\9{\relax}\ifnum6=\muexpr-\dimexpr32spplus-1muminus-1fil/-5 T\else F\fi"#),
            ("gluetomu_muskip", r"\muskip1=5mu\skip2=\gluetomu\muskip1\the\skip2"),
            // \the\muexpr 直接求值显示 mu 单位
            ("the_muexpr_val", r"\the\muexpr 5mu+3mu"),
            // \skip=\muexpr：Incompatible glue units（pt 上下文遇 mu 胶水）
            ("skip_from_muexpr", r"\skip2=\muexpr5mu\the\skip2"),
            // \muskip=\glueexpr：Incompatible glue units（mu 上下文遇 pt 胶水）
            ("muskip_from_glueexpr", r"\muskip2=\glueexpr5pt\the\muskip2"),
            // \muskip=\skip 前导：Incompatible glue units
            ("muskip_from_skip", r"\skip0=1pt plus 2pt\muskip1=\skip0\the\muskip1"),
            // mu 上下文单位错误：5pt → "(mu inserted)" 恢复 5.0mu
            ("muskip_pt_unit", r"\muskip2=5pt\the\muskip2"),
            // mu 上下文无单位：→ "(mu inserted)" 恢复 5.0mu
            ("muskip_bare", r"\muskip3=5\the\muskip3"),
            // mu 上下文 width 带阶：→ "(mu inserted)"（width 不认 fil）
            ("muskip_width_fil", r"\muskip4=5fil\the\muskip4"),
            // mu 上下文 stretch 带阶：合法（stretch 认 fil）→ "1.0mu plus 1.0fil"
            ("muskip_stretch_fil", r"\muskip5=1mu plus 1fil\the\muskip5"),
            // pt 上下文遇 mu 单位：→ "(pt inserted)" 恢复
            ("skip_mu_unit", r"\skip6=5mu\the\skip6"),
        ] {
            let mut e = Expander::new();
            match e.run_source(src) {
                Ok(_) => {
                    let out: String = e
                        .output()
                        .iter()
                        .map(|t| t.charcode().and_then(char::from_u32).unwrap_or('\u{FFFD}'))
                        .collect();
                    println!("[{name}] OUT={out:?}");
                }
                Err(err) => println!("[{name}] ERR={err:?}"),
            }
            println!("[{name}] TRANSCRIPT={:?}", e.transcript());
        }
    }

    /// ETRIP P0 \muexpr 校准（report_help + math_em）：expander 端两层回归。
    /// ① Incompatible glue units. 后跟 help1 行 "I'm going to assume that
    ///    1mu=1pt when they're mixed."（etrip.tex L900-960 段、参考 tex.web
    ///    L8265-8268 mu_error）。
    /// ② \thinmuskip=\<mu> 与 \the\thinmuskip 显示仍为 "X.0mu"——\muskip 寄存器
    ///    以 mu 数值存，expander 显示不依赖 em（layout 端的 sp 缩放见
    ///    math_em/mu_to_sp，ntex-layout tests 验）。
    #[test]
    fn muexpr_incompatible_help1_and_thinmuskip_display() {
        // ① \skip=\muskip：mu→pt 上下文混用，报 Incompatible 后跟 help1 行。
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source("\\muskip1=5mu\\skip0=\\muskip1\\relax").unwrap();
        let sink = e.take_sink();
        let mut sink = sink;
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert!(
            sink.transcript.contains("Incompatible glue units."),
            "Incompatible 触发：{:?}",
            sink.transcript
        );
        assert!(
            sink.transcript.contains("I'm going to assume that 1mu=1pt when they're mixed."),
            "help1 行未跟随：{:?}",
            sink.transcript
        );

        // ② \thinmuskip=18mu \the\thinmuskip —— 仍显示 "18.0mu"。
        assert_eq!(
            expand("\\thinmuskip=18mu\\the\\thinmuskip").unwrap(),
            "18.0mu",
            "\\thinmuskip \\the 应按 mu 数值原样显示"
        );
        // 整数 mu 显式路径（muskip_params 通路之一）：与 etrip.tex L1018 一致。
        assert_eq!(
            expand("\\thinmuskip=27mu plus 9mu minus 18mu\\the\\thinmuskip").unwrap(),
            "27.0mu plus 9.0mu minus 18.0mu"
        );
        // \muskip 寄存器：以 mu 数值原样存。
        assert_eq!(
            expand("\\muskip5=2.5mu plus 1mu\\the\\muskip5").unwrap(),
            "2.5mu plus 1.0mu"
        );
    }
