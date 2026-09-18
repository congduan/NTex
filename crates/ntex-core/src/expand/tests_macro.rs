use super::*;

    #[test]
    fn simple_def_and_use() {
        assert_eq!(expand("\\def\\foo{Hello}\\foo").unwrap(), "Hello");
    }

    #[test]
    fn macro_with_argument() {
        assert_eq!(
            expand("\\def\\greet#1{Hi #1!}\\greet{World}").unwrap(),
            "Hi World!"
        );
    }

    #[test]
    fn macro_with_two_arguments() {
        assert_eq!(
            expand("\\def\\pair#1#2{[#1:#2]}\\pair{x}{y}").unwrap(),
            "[x:y]"
        );
    }

    #[test]
    fn edef_expands_at_definition() {
        assert_eq!(expand("\\def\\a{1}\\edef\\y{\\a2}\\y").unwrap(), "12");
    }

    #[test]
    fn edef_keeps_primitive_tokens() {
        // TeX expand() 语义：\edef 不执行不可展开原语，保留在宏体
        assert_eq!(expand("\\edef\\x{\\def\\y{Z}\\y}\\x").unwrap(), "Z");
        // 保留组定界（花括号是宏体结构的一部分）
        assert_eq!(
            expand("\\edef\\x{a{b}}\\expandafter\\detokenize\\expandafter{\\x}").unwrap(),
            "a{b}"
        );
        // 未定义 cs 保留不报错
        assert_eq!(
            expand("\\edef\\x{a\\undefinedcs}\\expandafter\\detokenize\\expandafter{\\x}").unwrap(),
            "a\\undefinedcs "
        );
    }

    #[test]
    fn number_primitive_expands() {
        assert_eq!(expand("\\count0=5\\number\\count0").unwrap(), "");
        // \edef 中 \number 展开（ETRIP \def\2{\number\eTeXversion\eTeXrevision} 模式）
        assert_eq!(expand("\\count0=5\\edef\\x{\\number\\count0}\\x").unwrap(), "5");
        assert_eq!(expand("\\number-7").unwrap(), "-7");
    }

    // ── 第十三刀：`\the⟨toks⟩` 在展开收集语境的冻结语义（tex.web L9395-9411）──
    // scan_toks 的 xpand 展开器遇 `\the` 把产物**直接接进收集表**（"without
    // expanding it further"）；主循环 ins_list 路径照常展开执行。

    #[test]
    fn the_toks_frozen_in_edef_deferred_execution() {
        // 任务最小复现（GT pdftex 1.40.29：DEF-DONE → R-EXEC → CALL-DONE）：
        // `\the\T` 产物在 \edef 期冻结为宏 token，调用 `\x` 时才执行。
        // 修复前签名：R-EXEC 提前于 DEF-DONE（体被内联展开）。
        let (r, t) = run_transcript(
            "\\catcode`\\@=11 \n\\toksdef\\T=0\n\\def\\reinstallA{\\message{R-EXEC}}\n\
             \\T={\\reinstallA}\n\\edef\\x{\\the\\T}\\message{DEF-DONE}\\x\\message{CALL-DONE}",
        );
        assert!(r.is_ok(), "{r:?}");
        let def_done = t.find("DEF-DONE").expect("DEF-DONE 应在转录中");
        assert!(
            !t[..def_done].contains("R-EXEC"),
            "R-EXEC 提前于 DEF-DONE（\\the 产物在 edef 被再展开）：{t}"
        );
        assert!(t.contains("CALL-DONE"), "转录：{t}");
    }

    #[test]
    fn the_toks_gaddto_macro_body_stays_frozen() {
        // latex.ltx l.12705 NFSS 钩子链惯用法：`\xdef#1{\the\toks@}` 的体必须
        // 保宏 token 原样（GT pdftex：`macro:->BASE\reinstall@nfss@defs `；
        // 修复前体被内联展开成 `\message{R-IN}`，NFSS 钩子链整段失效）。
        let (r, t) = run_transcript(
            "\\catcode`\\@=11 \n\\toksdef\\toks@=0\n\\def\\reinstall@nfss@defs{\\message{R-IN}}\n\
             \\def\\g@addto@macro#1#2{\\begingroup \\toks@\\expandafter{#1#2}\\xdef#1{\\the\\toks@}\\endgroup}\n\
             \\def\\kb{BASE}\n\\g@addto@macro\\kb{\\reinstall@nfss@defs}\n\
             \\message{M1:[\\meaning\\kb]}",
        );
        assert!(r.is_ok(), "{r:?}");
        assert!(
            t.contains("macro:->BASE\\reinstall@nfss@defs"),
            "\\g@addto@macro 体被内联展开：{t}"
        );
        let m1 = t.find("M1:").expect("M1 应在转录中");
        assert!(!t[..m1].contains("R-IN"), "R-IN 提前于 M1（体被展开执行）：{t}");
    }

    #[test]
    fn the_toks_frozen_in_message_context() {
        // GT：`\message{[\the\T]}` 打 `\reinstallA `（cs 冻结、不执行）。
        // 修复前：内联执行后转录含 R-EXEC、消息体为空。
        let (r, t) = run_transcript(
            "\\catcode`\\@=11 \n\\toksdef\\T=0\n\\def\\reinstallA{\\message{R-EXEC}}\n\
             \\T={\\reinstallA}\n\\message{MSG:[\\the\\T]}",
        );
        assert!(r.is_ok(), "{r:?}");
        assert!(t.contains("MSG:[\\reinstallA ]"), "转录：{t}");
        assert!(!t.contains("R-EXEC"), "\\the 产物在 message 展开被执行：{t}");
    }

    #[test]
    fn the_toks_still_executes_in_main_loop() {
        // 主循环（非收集语境）`\the\T` 产物照常执行（GT：EXEC-A）——冻结只在
        // 展开收集语境，不得砸掉横/竖模式执行语义（第十二刀语义零回退面）。
        assert_eq!(expand("\\toksdef\\T=0\\def\\A{X}\\T={\\A}\\the\\T").unwrap(), "X");
    }

    #[test]
    fn the_toks_frozen_group_chars_preserved() {
        // GT：`\T={{x}}` + `\edef\y{a\the\T b}` → `\y=macro:->a{x}b`——冻结
        // token 的组字符原样落表（tex.web 接表语义：不过配平、不建组）。
        assert_eq!(
            expand(
                "\\toksdef\\T=0\\T={{x}}\\edef\\y{a\\the\\T b}\
                 \\expandafter\\detokenize\\expandafter{\\y}"
            )
            .unwrap(),
            "a{x}b"
        );
    }

    #[test]
    fn let_alias() {
        assert_eq!(expand("\\def\\a{XY}\\let\\b\\a\\b").unwrap(), "XY");
    }

    #[test]
    fn outer_prefix_parses() {
        assert_eq!(expand("\\outer\\def\\foo{XY}\\foo").unwrap(), "XY");
        assert_eq!(expand("\\outer\\gdef\\foo{XY}\\foo").unwrap(), "XY");
        assert_eq!(expand("\\outer\\global\\edef\\foo{XY}\\foo").unwrap(), "XY");
    }

    #[test]
    fn xdef_expands_body_at_definition() {
        // \xdef ≡ \global\edef：定义时展开宏体
        assert_eq!(expand("\\def\\a{1}\\xdef\\b{\\a2}\\def\\a{3}\\b").unwrap(), "12");
        // 全局：组内 \xdef 在组外可见
        assert_eq!(
            expand("{\\def\\a{1}\\xdef\\b{\\a2}}\\b").unwrap(),
            "12"
        );
    }

    #[test]
    fn let_primitive_survives_redefinition() {
        // \let\PAR=\par 后 \PAR 应可用（TRIP L404；L418 报 Undefined 是 bug 线索）
        assert_eq!(
            expand("\\let\\PAR=\\par\\PAR x").unwrap(),
            "x",
            "\\PAR 应别名 \\par 原语"
        );
        // TRIP L392-404 组结构：{ 组 + \c}（\c 未定义）+ \let\c\b + 条件 + \let\PAR
        // 参考里 \c} 的 } 闭合 L392 组 → \let\PAR 顶层定义；NTex 曾组级=1 泄漏
        assert_eq!(
            expand("{\\c}\\let\\PAR=\\par\\PAR x").unwrap(),
            "x",
            "最小 c-闭组（未定义）场景 PAR 应可用"
        );
        // \\if 11 后的空格：参考 log 同样输出 {blank space  }（TeX 语义：
        // get_x_token 跳过 \\if 后的空格，但 11 与 A 之间的空格在输入流照常处理）
        assert_eq!(expand(r"{\if11 A\else B\fi}").unwrap(), " A");
        // \\PAR 别名链在条件/组后仍成立（\\ifx 验证）
        for (name, pre) in [
            ("纯条件", r"{\if 11 A\else B\fi}"),
            ("纯组", r"{A}"),
        ] {
            assert!(
                expand(&format!(r"{pre}\let\PAR=\par\ifx\PAR\par yes\else no\fi"))
                    .unwrap()
                    .contains("yes"),
                "{name} 后 PAR 应别名 par"
            );
        }
        let out392 = expand(
            "{\\if 11 \\prevgraf=-1\\if 0123\\error\\else\\relax\\fi\\else\\error\\fi\\c}\\let\\c\\b \\ifx\\a\\ifx.\\else\\error\\fi\\fi\\let\\PAR=\\par\\gdef\\par{\\relax\\PAR}\\PAR x",
        )
        .unwrap();
        // \\if 11 前导空格是 TeX 语义（参考同样输出）；\\PAR 应正常展开为段落结束
        assert!(
            out392.ends_with("x"),
            "L392-404 组结构后 PAR 应可用：{out392:?}"
        );
        // TRIP L397-418 模拟：\def 错误 + \let\c + 条件 + \let\PAR + \gdef\par
        assert_eq!(
            expand(
                "\\def\\a}{\\let\\a\\xyzzy\\csname a\\endcsname}\\def\\a{ab\\par\\c}\\def\\b{ab*\\par\\c}\\let\\c\\b \\ifx\\a\\ifx.\\else\\expandafter\\ifx\\b\\ifinner\\error\\else\\relax\\fi\\else\\error\\fi\\fi\\let\\PAR=\\par\\gdef\\par{\\relax\\PAR}\\PAR x"
            )
            .unwrap(),
            "x",
            "TRIP 序列后 \\PAR 应可用"
        );
        // TRIP L6-10（\par 覆盖+恢复）+ L397-418 序列：\PAR 应可用
        assert_eq!(
            expand(
                "\\let\\paR=\\par\\outer\\xdef\\par{\\catcode`\\%14}\\let\\par=\\paR\\def\\a}{\\let\\a\\xyzzy\\csname a\\endcsname}\\def\\a{ab\\par\\c}\\def\\b{ab*\\par\\c}\\let\\c\\b \\ifx\\a\\ifx.\\else\\expandafter\\ifx\\b\\ifinner\\error\\else\\relax\\fi\\else\\error\\fi\\fi\\let\\PAR=\\par\\gdef\\par{\\relax\\PAR}\\PAR x"
            )
            .unwrap(),
            "x",
            "L6-10 + L397-418 序列后 \\PAR 应可用"
        );
        // trip.tex 第 6-10 行：\let\paR=\par → 重定义 \par → \let\par=\paR 恢复
        assert_eq!(
            expand(
                "\\let\\paR=\\par\\outer\\xdef\\par{\\catcode`\\%14}\\let\\par=\\paR\\relax"
            )
            .unwrap(),
            ""
        );
    }

    #[test]
    fn let_to_char() {
        assert_eq!(expand("\\let\\X=x\\X").unwrap(), "x");
    }

    #[test]
    fn let_missing_control_sequence_recovers() {
        // tex.web 的 \let 左侧不是控制序列时插入 \inaccessible，拒绝的字符
        // 回推给主循环；不得把可恢复的 TeX 输入错误升级为 Rust Err。
        // 被拒的 `a` 会被回推，继而成为 \inaccessible 的右值；`=` 随后作为
        // 普通字符输出（与 TeX 的 back_input 恢复顺序一致）。
        assert_eq!(expand("\\let a=xZ").unwrap(), "=xZ");
    }

    #[test]
    fn macro_definition_balances_non_brace_group_character() {
        // expl3 `tl-analysis` 临时将 ^^@ 设为组开始符，再把它放进宏定义体。
        // 定义扫描必须按 token 的实际 catcode 计深，不能吞掉其后的定义/全文。
        assert_eq!(
            expand("\\catcode`\\^^@=1 \\def\\a{^^@}}\\catcode`\\^^@=12 X").unwrap(),
            "X"
        );
    }

    #[test]
    fn expandafter_classic() {
        // 经典：\expandafter\def\expandafter\x\expandafter{\b} 使 \x = \b 的展开。
        // 注：\b 中 \a 与 C 之间的空格在扫描时被控制词吞掉，故为 "ABC"（与真实 TeX 一致）。
        let src =
            "\\def\\a{B}\\def\\b{A\\a C}\\expandafter\\def\\expandafter\\x\\expandafter{\\b}\\x";
        assert_eq!(expand(src).unwrap(), "ABC");
    }

    #[test]
    fn noexpand_defers_expansion() {
        // \edef 时 \noexpand\y 使 \y 保持为 token；\z 使用时 \y 才展开
        let src = "\\def\\y{YY}\\def\\x{A\\noexpand\\y B}\\edef\\z{\\x}\\z";
        assert_eq!(expand(src).unwrap(), "AYYB");
    }

    #[test]
    fn noexpand_survives_argument_handoff_inside_edef() {
        // pdfTeX GT（2026-09-18）：
        // \def\a{A}\def\p{\noexpand\a}\def\y#1{#1}
        // \edef\z{\expandafter\y\p} → \meaning\z = macro:->\a
        //
        // \expandafter 先把 \p 展开成带 noexpand 标记的 \a，再交给 \y 收作实参。
        // 实参帧若丢掉这个一次性冻结位，\a 会在同一个 \edef 区域内误展开成 A。
        let src = "\\def\\a{A}\\def\\p{\\noexpand\\a}\\def\\y#1{#1}\\edef\\z{\\expandafter\\y\\p}\\meaning\\z";
        assert_eq!(expand(src).unwrap(), "macro:->\\a");
    }

    #[test]
    fn noexpanded_the_survives_write_argument_handoff() {
        fn run(src: &str, bytecode: bool) -> String {
            let mut e = if bytecode {
                Expander::new()
            } else {
                Expander::new_interpreter()
            };
            e.run_source(src).unwrap();
            e.transcript().to_owned()
        }

        // 同上，但冻结对象换成 \the：LaTeX \protect 链会让 \the<counter>
        // 经宏实参转交到 \write 展开区域。冻结位丢失时会误执行 \the 并在真实
        // \section/\item/\label 链上报 "You can't use \the with this."。
        let src = "\\count0=3\\def\\p{\\noexpand\\the\\count0}\\def\\y#1{#1}\\immediate\\write16{W=\\expandafter\\y\\p}";
        let bytecode = run(src, true);
        let interp = run(src, false);
        assert_eq!(bytecode, interp);
        assert!(bytecode.contains("W=\\the \\count 0"), "{bytecode}");
        assert!(!bytecode.contains("You can't use \\the"), "{bytecode}");
    }

    #[test]
    fn the_expands_value_macro_to_counter_register() {
        // LaTeX 结构宏常用 `\the\value{section}`；`\value` 本身是宏，展开后
        // 才得到 countdef'd `\c@section` 内部整数。`\the` 扫操作数必须走
        // get_x_token 语义，不能只展开可展开原语。
        let src = concat!(
            "\\catcode`\\@=11 ",
            "\\countdef\\c@section=0 ",
            "\\count0=7 ",
            "\\def\\value#1{\\csname c@#1\\endcsname}",
            "\\the\\value{section}"
        );
        assert_eq!(expand(src).unwrap(), "7");
    }

    #[test]
    fn meaning_expands_to_meaning_text() {
        // 宏：macro:->body（无尾随句点）
        assert_eq!(expand("\\def\\x{a}\\meaning\\x").unwrap(), "macro:->a");
        // 原语：\relax
        assert_eq!(expand("\\meaning\\relax").unwrap(), "\\relax");
        // \countdef 绑定：\count0
        assert_eq!(expand("\\countdef\\x=0\\meaning\\x").unwrap(), "\\relax");
        // 未定义 cs：undefined
        assert_eq!(expand("\\meaning\\undefinedcs").unwrap(), "undefined");
    }

    #[test]
    fn meaning_protected_prefix() {
        // tex.web print_meaning（e-TeX）：protected 宏 `\protected macro:`。
        // expl3 `\cs_generate_variant`（\__cs_generate_variant:N 读 meaning
        // 前缀选 `\cs_new_protected:Npe`/`\cs_new:Npe`）依赖此前缀：缺它则
        // 全部变体降级非保护，`\tl_const:Ne` 在 `\expanded` 内被展开，
        // latex.ltx l.9386 `\char_generate:nn` 查表守卫残留 → Illegal
        // parameter number（第二十八刀）。
        assert_eq!(
            expand("\\protected\\def\\x{a}\\meaning\\x").unwrap(),
            "\\protected macro:->a"
        );
        // 非保护宏无前缀
        assert_eq!(expand("\\def\\y{b}\\meaning\\y").unwrap(), "macro:->b");
    }

    #[test]
    fn nested_braces_in_argument() {
        // 实参内层花括号在主流层建立组（不输出），与真实 TeX 一致
        assert_eq!(expand("\\def\\wrap#1{[#1]}\\wrap{a{b}c}").unwrap(), "[abc]");
    }

    #[test]
    fn empty_argument() {
        assert_eq!(expand("\\def\\wrap#1{[#1]}\\wrap{}").unwrap(), "[]");
    }

    // ---------- M1-7 扫描顺序原语 ----------

    #[test]
    fn futurelet_captures_next_token() {
        // \futurelet\next\relax a → \next := 字符 'a'（\let 语义），\relax 与 a 照常处理；
        // 用 \ifx 与另一个 \let 到 'a' 的控制序列比较（\ifx CS vs 裸字符必为假）。
        let src =
            "\\futurelet\\next\\relax a\\let\\expected a\\ifx\\next\\expected yes\\else no\\fi";
        assert_eq!(expand(src).unwrap(), "ayes");
    }

    #[test]
    fn futurelet_missing_control_sequence_recovers() {
        // 与 \let 同款 missing-control-sequence 恢复：被拒 token 回推后成为
        // \inaccessible 的观察 token，后续输入仍可继续处理。
        assert_eq!(expand("\\futurelet aXY").unwrap(), "aXY");
    }

    #[test]
    fn aftergroup_inserts_token_at_group_end() {
        let src = "\\def\\X{Z}\\begingroup\\aftergroup\\X\\endgroup";
        assert_eq!(expand(src).unwrap(), "Z");
    }

    #[test]
    fn afterassignment_inserts_token_after_assignment() {
        let src = "\\def\\X{done}\\afterassignment\\X\\count0=5";
        assert_eq!(expand(src).unwrap(), "done");
    }

    #[test]
    fn afterassignment_survives_group_scope() {
        // \afterassignment 触发时已出组，赋值本身组内局部
        let src = "\\count0=1\\def\\X{a}\\begingroup\\afterassignment\\X\\count0=2\\endgroup\\the\\count0";
        assert_eq!(expand(src).unwrap(), "a1");
    }

    #[test]
    fn nested_param_in_macro_arg_repro() {
        // ETRIP etrip.tex L1038：\def\1#1{\2{3210#1}}，\11 调用时 #1 应被替换
        // 为实参 1（\2 实参 = 32101），而非保持 macro_param 导致 3210+Param(1)
        // 错位（\ifvbox 读到 Param → Missing number）。
        // 对照：字母宏名（应正常）
        // 旧期望里的 -0.27779 是盒内容 GT 通道的 italic-correction kern 残留；
        // 本引擎 output() 只收字符（pdfTeX GT gtf/gf b13/b14：字符流 "ARG=abc"）
        assert_eq!(expand("\\def\\a#1{ARG=#1}\\a{abc}").unwrap(), "ARG=abc");
        // 最小链拆解：\2 单独调用（数字 cs）
        assert_eq!(expand("\\def\\2#1{ARG=#1}\\2{abc}").unwrap(), "ARG=abc");
        // 最小链拆解：\1 单独定义+调用（无嵌套）
        assert_eq!(expand("\\def\\1#1{X#1}\\1{5}").unwrap(), "X5");
        // 直接输出实参内容：期望 ARG=32101（实参 3210 后跟 \1 的实参 1）
        // 注意：% 后必须真实换行（Rust `%\` 续行会把 % 注释吞到 EOF！）
        let src = "\\def\\2#1{ARG=#1}%\n\\def\\1#1{\\2{3210#1}}%\n\\1{5}";
        assert_eq!(expand(src).unwrap(), "ARG=32105");
        // \11 词法：\1 + 数字 1（catcode 12 → 单字符 cs）
        let src2 = "\\def\\2#1{ARG=#1}%\n\\def\\1#1{\\2{3210#1}}%\n\\11";
        assert_eq!(expand(src2).unwrap(), "ARG=32101");
        // e-TeX 大寄存器号（etrip L1039 \setbox32101；e-Trip 扩展 32768 寄存器）：
        // \ifhbox32101 不应 Missing number / Bad register code。
        // 注：VecSink 环境 setbox 是 no-op（box 未设置 → \ifhbox 假、\hbox{X} 输出 X），
        // 这里只断言无 Missing number 报错（真实 sink 由 etrip 段验证）。
        let bigsrc = "\\setbox32101=\\hbox{X}\\ifhbox32101 A\\else B\\fi";
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source(bigsrc).unwrap();
        let sink = e.take_sink();
        let mut sink = sink;
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert!(
            !sink.transcript.contains("Missing number"),
            "大寄存器号不应 Missing number：{}",
            sink.transcript
        );

        // etrip.tex L1020-1050 段复现（box 寄存器测试：\setbox32101 + \11 调用）
        let Some(full) = read_fixture("fixtures/etrip/etrip.tex") else {
            return;
        };
        let lines: Vec<&str> = full.lines().collect();
        let pre = r"\def\typeout{\immediate\write15 }
\def\error#1{\immediate\write15{Bug in your e-TeX implementation!}\immediate\write15 }";
        let body = format!("{}\n", lines[1019..1050].join("\n"));
        let src = format!("{pre}\n{body}\n");
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        // 运行验证不 panic（错误细节由 ETRIP 全量覆盖）
        let _ = e.run_source(&src);
        // 数字 cs 词法：\22000 应为 \2 + 2000（数字 catcode 12 → 单字符 cs）
        assert_eq!(expand("\\def\\2{X}\\22000").unwrap(), "X2000");
        // \the\count2000（原语 \count + 数字，etrip \the\22000 模式）
        assert_eq!(expand("\\count2000=5\\the\\count2000").unwrap(), "");
        // \write15 内容走转录（VecSink）而非 output；流 15 未打开 → 需 \immediate
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source("\\count2000=5\\immediate\\write15{\\the\\count2000}").unwrap();
        let sink = e.take_sink();
        let mut sink = sink;
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert_eq!(
            sink.transcript, "5\n",
            "\\write15 内 \\the\\count2000 应输出 5：{:?}",
            sink.transcript
        );
        // \advance/\multiply/\divide 目标支持宏别名展开（etrip \edef\2{\csname
        // count\endcsname} 模式：\advance\2200by3 = \advance\count200by3）。
        // pdfTeX GT（r.tex R1）：amount 数字循环尾 get_x_token 就地展开
        // \the\count200，旧值 "5" 折入 3 → by35 → count200=40，字符流空；
        // 尾部再加 [\the\count200] 锁值 → "[40]"（裸形断 "" 太弱）。早前
        // tr.tex 的 "8" 是把 \the 藏进 \write16 实参（数字循环外）的探针形，
        // 不是本输入形的真值。原形 \count2000 需 e-TeX 32768 寄存器扩展，
        // 本引擎 256 槽未实现，见 scoreboard 遗留清单。
        assert_eq!(
            expand("\\count200=5\\edef\\2{\\csname count\\endcsname}\\advance\\2200by3\\the\\count200[\\the\\count200]")
                .unwrap(),
            "[40]"
        );
        // multiply/divide 走同一条别名目标 + amount 数字循环折叠：
        // pdfTeX GT（s.tex N2/N3）\multiply 旧值 "5" 折入 3 → by35 → 175；
        // \divide 12/35 圆整 → 0。原形 \count2000（pdftex N4=15）需
        // e-TeX 32768 寄存器扩展，本引擎 256 槽未实现，见 scoreboard 遗留清单。
        assert_eq!(
            expand("\\count200=5\\edef\\2{\\csname count\\endcsname}\\multiply\\2200by3\\the\\count200[\\the\\count200]")
                .unwrap(),
            "[175]"
        );
        assert_eq!(
            expand("\\count200=12\\edef\\2{\\csname count\\endcsname}\\divide\\2200by5\\the\\count200[\\the\\count200]")
                .unwrap(),
            "[0]"
        );
    }

    // ---------- A4：\outer 语义 ----------

    #[test]
    fn outer_macro_normal_use_ok() {
        // outer 宏在正常展开上下文可用
        assert_eq!(expand("\\outer\\def\\x{A}\\x").unwrap(), "A");
    }

    #[test]
    fn outer_forbidden_in_macro_argument() {
        // tex.web `@<Tell the user what has run away...@>`（L7184-7200）：outer 宏
        // 出现在参数上下文是**可恢复错误**（`error`，非 fatal）——打印
        // `Forbidden control sequence found while scanning use of \X` +
        // 插入 `\par` 恢复，**作业继续**。
        //
        // pdfTeX ground truth（2026-09-11 实测）：
        //   `\outer\def\O{a}\def\f#1{[#1]}\f\O\message{[B]}`
        //   → 报 Forbidden + **继续**执行 `\message{[B]}`。
        //
        // ⚠ 用 run_transcript（单轨）而非 expand()：expand() 做双轨对照
        // （字节码 vs 解释器），恢复语义只在生产轨（字节码）实现。
        let (r, t) = run_transcript("\\def\\a#1{#1}\\outer\\def\\x{A}\\a\\x\\message{[B]}");
        assert!(r.is_ok(), "outer 恢复后作业应继续：{t}");
        assert!(
            t.contains("Forbidden control sequence"),
            "须报 Forbidden（诊断保真）：{t}"
        );
        assert!(
            t.contains("use of \\a"),
            "错误须指明 warning_index（tex.web `sprint_cs`: 正在扫的宏 \\a，非 outer 宏 \\x）：{t}"
        );
        // 组实参内同样报 forbidden 但**不中断**作业
        let (r2, t2) = run_transcript("\\def\\a#1{#1}\\outer\\def\\x{A}\\a{\\x}\\message{[C]}");
        assert!(r2.is_ok(), "组实参内 outer 恢复后应继续：{t2}");
        assert!(t2.contains("Forbidden control sequence"), "{t2}");
    }

    #[test]
    fn outer_forbidden_in_edef() {
        // \edef 展开上下文中 outer 宏 → forbidden
        let e = expand("\\outer\\def\\x{A}\\edef\\y{\\x}");
        assert!(e.is_err(), "\\edef 中 outer 宏应报错");
        assert!(e.unwrap_err().to_string().contains("forbidden"));
    }

    #[test]
    fn the_in_edef_expands() {
        assert_eq!(
            expand("\\count0=7\\edef\\x{\\the\\count0}\\x").unwrap(),
            "7"
        );
    }

    #[test]
    fn the_meaning_expands_without_recursion() {
        // 回归（fuzz 命中）：\the\meaning 此前 expand_once 缺失 Meaning 分支，\meaning 被原样
        // 保留 → the_tokens_after 无限递归 → 栈溢出（畸形输入不 panic 契约违约）。
        assert_eq!(
            expand("\\the\\meaning\\undefinedcs").unwrap(),
            "undefined"
        );
    }

    #[test]
    fn the_jobname_expands_without_recursion() {
        // 回归（fuzz 命中）：\the\jobname 此前同 \meaning 无限递归。
        assert_eq!(expand("\\the\\jobname").unwrap(), "texput");
    }

    #[test]
    fn romannumeral_expands_to_roman_digits() {
        assert_eq!(expand("\\romannumeral 14").unwrap(), "xiv");
        assert_eq!(expand("\\romannumeral 0").unwrap(), "");
    }

    // ---------- M1-11 组与作用域 ----------

    #[test]
    fn local_def_restored_at_group_end() {
        let src = "\\def\\a{X}\\begingroup\\def\\a{Y}\\a\\endgroup\\a";
        assert_eq!(expand(src).unwrap(), "YX");
    }

    #[test]
    fn global_def_persists() {
        let src = "\\def\\a{X}\\begingroup\\global\\def\\a{Y}\\a\\endgroup\\a";
        assert_eq!(expand(src).unwrap(), "YY");
    }

    #[test]
    fn gdef_is_global() {
        let src = "\\def\\a{X}\\begingroup\\gdef\\a{Y}\\a\\endgroup\\a";
        assert_eq!(expand(src).unwrap(), "YY");
    }

    // ─── unsave 的 retaining 分支（tex.web @<Store save_stack[save_ptr] in
    // eqtb[p], unless eqtb[p] holds a global value@>）────────────────────────
    // 组内"先局部触碰（压 save 条目）、后 \global 赋值"：组末恢复必须跳过该
    // 陈旧条目，保留全局值。expl3 全线依赖：expl3.ltx l.23-30 的
    // `\csname c__kernel_expl_date_tl\endcsname`（局部制造 relax）+
    // `\global\let` 守卫、l.3195 起的 `\exp_args:*` 变体生成器
    // （`\group_begin: … \cs_if_free:cT` 的 c 型 \csname 制造 + `\cs_gset`
    // 全局定义 + `\group_end:`）——此前组末把全局定义回滚成局部触碰前的值，
    // 变体名整体失效（expl3-code l.3202 区 Undefined control sequence 级联，
    // 最终 `\char_set_catcode_group_end:N` 缺失 → `^^@` catcode 复位失败 →
    // regex 替换宏定义体 runaway）。

    #[test]
    fn global_def_survives_group_after_local_touch() {
        // \csname 制造（局部 relax）+ \global\def：组末 retain
        assert_eq!(
            expand(
                "\\begingroup\\expandafter\\let\\csname gz\\endcsname\\relax\\global\\def\\gz{G}\\endgroup\\ifdefined\\gz KEEP\\else LOST\\fi"
            )
            .unwrap(),
            "KEEP"
        );
        // 全局值不被局部触碰前的旧值覆盖
        assert_eq!(
            expand("\\begingroup\\def\\gv{L}\\global\\def\\gv{G}\\endgroup\\gv").unwrap(),
            "G"
        );
    }

    #[test]
    fn global_let_survives_group_after_csname_relax() {
        // expl3.ltx l.23-30 原型（_ 为字母的语法态）：组内 \csname 制造 relax
        // （局部，TeX 2.9）→ \global\let → 组末 retain。注意组外检查须保持
        // _ 为字母，否则 \c__kt 被读作控制词 \c + 字符。
        assert_eq!(
            expand(
                "\\catcode`\\_=11 \\begingroup\\expandafter\\ifx\\csname c__kt\\endcsname\\relax\\global\\let\\c__kt\\relax\\fi\\endgroup\\ifdefined\\c__kt KEEP\\else LOST\\fi"
            )
            .unwrap(),
            "KEEP"
        );
    }

    // ─── \global 前缀与赋值之间的可展开序列（LaTeX 兼容第三刀）─────────────
    // tex.web：前缀只置标志（prefixed_command 的前缀循环），赋值发生在主循环
    // 后续 token——前缀与赋值原语之间可以隔条件、\expandafter 链、宏展开。
    // latex.ltx L488 `\newbox\voidb@x` → `\e@alloc` 的
    // `\global#2#6\allocationnumber`（#2 = `\ifnum…\expandafter\chardef\else…\fi`）
    // 即依赖此语义。

    #[test]
    fn global_prefix_through_conditional_assign_target() {
        // 条件选择的赋值目标（\newbox/\e@alloc 的真实形态）
        assert_eq!(
            expand("\\global\\ifnum1=1\\chardef\\x=3\\else\\chardef\\x=4\\fi\\the\\x").unwrap(),
            ""
        );
        // else 分支同样到达赋值
        assert_eq!(
            expand("\\countdef\\n=0 \\n=7 \\global\\ifnum\\n>8\\chardef\\x=3\\else\\chardef\\x=4\\fi\\the\\x")
                .unwrap(),
            // false → \else 分支 \chardef 扫描遇 \fi 触发 insert_relax，
            // pdfTeX 实测（gtf/gf b17）：\x 未绑定成 char_given，\the 报
            // "You can't use \relax after \the" → 盒空
            ""
        );
        // \global 确实全局：组外可见
        assert_eq!(
            expand("\\begingroup\\global\\ifnum1=1\\count0=5\\else\\relax\\fi\\endgroup\\the\\count0")
                .unwrap(),
            "5"
        );
    }

    #[test]
    fn global_prefix_through_expandable_chain() {
        // \expandafter\chardef\csname…（\e@alloc@chardef 分支的形态）
        assert_eq!(
            expand("\\global\\expandafter\\chardef\\csname y\\endcsname=5\\the\\y").unwrap(),
            ""
        );
        // \relax 在前缀与赋值之间（tex.web 前缀循环跳过 \relax）
        assert_eq!(expand("\\global\\relax\\chardef\\r=9\\the\\r").unwrap(), "");
        // 前缀 → 宏展开 → 赋值（TeX：前缀标志不随宏展开丢失）
        assert_eq!(expand("\\def\\z{\\chardef\\q=7}\\global\\z\\the\\q").unwrap(), "");
    }

    #[test]
    fn expandafer_before_conditional_terminator_keeps_first_token() {
        // `\expandafter` 的第二 token 是 \else：tex.web expand() 的 fi_or_else
        // 处理是急切的（pass_text 消费到配对 \fi 后弹帧），t1 放回时**不得**被
        // 惰性跳过区吞掉——否则 \chardef 消失、赋值目标落空（latex.ltx L488）。
        // 两条引擎在该形上都走错误恢复、字符流皆空（pdfTeX 实测
        // "Missing control sequence inserted"：\chardef 落在 \else 后被就地
        // 执行吃掉 \relax\fi；本引擎 \chardef 落空 → \a 未定义）。旧期望 "B"
        // 是盒内容 GT 通道 kern 残留前的手推值。
        assert_eq!(
            expand("\\ifnum1=1\\expandafter\\chardef\\else\\relax\\fi\\a 1\\the\\a").unwrap(),
            ""
        );
        // \edef 展开上下文：\chardef 作为数据进入宏体。旧期望的
        // "-0.27779"/"3.33333 plus..." 是盒内容 GT 通道 kern/glue 残留——字符流
        // 就是宏体两 token。pdfTeX GT（tp.tex T4 \write16 同形）字符串为
        // "macro:->\chardef \relax"（pdftex 的 \meaning 在 token 间插空格，
        // 本引擎 \meaning 无分隔空格——格式器遗留偏差，非吸收语义差异）。
        assert_eq!(
            expand("\\edef\\b{\\ifnum1=1\\expandafter\\chardef\\else\\relax\\fi}\\meaning\\b").unwrap(),
            "macro:->\\chardef\\relax"
        );
        // 嵌套条件：内层 \else 在 \chardef 的目标扫描位被就地展开（fi_or_else
        // 急切处理）→ 内层条件闭合并跳过 \relax\fi，\chardef 落空报
        // "Missing control sequence inserted"（\inaccessible 兜底）→ \a 从未
        // 绑定，\the\a 级联 Undefined control sequence → 字符流空。
        // pdfTeX GT（s.tex N1，同一串错误级联、零排版输出）；旧期望 "2" 是
        // \else 不展开（内层 \\fi 闭条件）的手推值。
        assert_eq!(
            expand(
                "\\ifnum1=1\\ifnum1=1\\expandafter\\chardef\\else\\relax\\fi\\a 2\\else\\relax\\fi\\the\\a"
            )
            .unwrap(),
            ""
        );
    }

    // ─── 实参位置的条件终结符是数据（LaTeX 兼容第十一刀）─────────────────
    // tex.web 宏实参扫描（scan_toks macro=true）用 get_token：不展开、不推进
    // 条件机。`\else`/`\fi`/`\or`/`\if*` 在实参位置一律是数据 token——expl3 的
    // `\__kernel_primitive:NN \else \tex_else:D` 别名表依赖此语义。

    #[test]
    fn arg_cond_terminators_are_data_undelimited() {
        // 无分隔实参：`\else`/`\fi`/`\or` 收进 #1，不交条件机（不报 Extra）
        assert_eq!(
            expand("\\def\\f#1{[\\string#1]}\\edef\\a{\\f\\else\\f\\fi\\f\\or}\\a").unwrap(),
            // tex.web \string 用 sprint_cs（控制序列名后**不**补空格；
            // 补空格是 \detokenize 的 e-TeX 语义）。expl3 cs_to_str 依赖此。
            "[\\else][\\fi][\\or]"
        );
        // 实参扫描不在跳过区里运行：`\f` 的 #1=`\fi` 数据展开后由主循环闭合
        // 外层 \iftrue 帧（`\fi` 在宏体里被消费，`[]x` 全部排出）
        assert_eq!(
            expand("\\def\\f#1{[#1]}\\iftrue\\f\\fi x").unwrap(),
            "[]x"
        );
        // `\if*` 作实参数据（既有行为不回归）：#1=`\iftrue`
        assert_eq!(
            expand("\\def\\f#1{[\\string#1]}\\edef\\a{\\f\\iftrue}\\a").unwrap(),
            "[\\iftrue]"
        );
    }

    #[test]
    fn arg_cond_terminators_are_data_delimited() {
        // 分隔实参：`\else`/`\fi`/`\or` 是数据（不因 arg_cond==0 交条件机）
        assert_eq!(
            expand("\\def\\f#1!{[\\string#1]}\\edef\\a{\\f\\else!\\f\\fi!\\f\\or!}\\a").unwrap(),
            "[\\else][\\fi][\\or]"
        );
        // e-TeX 条件全集（旧白名单缺的 9 个）同样作数据收进实参
        assert_eq!(
            expand(concat!(
                "\\def\\f#1!{[\\string#1]}\\edef\\a{",
                "\\f\\ifeof!\\f\\ifvoid!\\f\\ifhbox!\\f\\ifvbox!\\f\\ifinner!",
                "\\f\\ifvmode!\\f\\ifhmode!\\f\\ifmmode!\\f\\iffontchar!}\\a"
            ))
            .unwrap(),
            "[\\ifeof][\\ifvoid][\\ifhbox][\\ifvbox][\\ifinner][\\ifvmode][\\ifhmode][\\ifmmode][\\iffontchar]"
        );
    }

    #[test]
    fn arg_cond_terminators_build_primitive_alias_expl3() {
        // expl3 `\__kernel_primitive:NN` 语义：`\else` 作 #1 数据被 `\let` 消费，
        // 建立原语别名——别名随后在条件结构里当终结符使用。
        assert_eq!(
            expand(concat!(
                "\\long\\def\\kp#1#2{\\global\\let#2#1}",
                "\\kp\\else\\myelse\\kp\\fi\\myfi\\kp\\or\\myor",
                "\\edef\\n{\\ifcase3\\myor a\\myelse y\\myfi}\\n"
            ))
            .unwrap(),
            "y"
        );
        // 旧注释引用的 `\expandafter\2\fi` 惯用法不经实参扫描：`\fi` 由
        // \expandafter 的展开位置消费，`\2` 的实参从其后取
        assert_eq!(
            expand("\\def\\tw#1{[#1]}\\edef\\a{\\iftrue\\expandafter\\tw\\fi\\iftrue b\\else c\\fi}\\a")
                .unwrap(),
            // #1=\iftrue（数据），在 \edef 的展开位置被重新求值（帧 F2 开启）
            "[]b"
        );
    }

    // ─── 分隔实参的整组贡献（LaTeX 兼容第十三刀：l3prg w 尾参）─────────────
    // tex.web macro_call：定界参数内的 `{…}` 平衡组**整体**作实参数据贡献，
    // 组内 token 只配对、不匹配定界符、`}` 不触发 "extra }"——expl3 的 w 尾参
    // `\tl_if_empty:nF {#8} {…}`（l3prg `\__prg_generate_conditional:NNnnnnNw`
    // 体）即此形。旧实现扁平扫描把组内首个 `}` 当深度 0 的额外 `}` → 报错截断
    // → 尾参被就地执行（undefined-cs 级联 + 栈超限，报告 §19）。

    #[test]
    fn delimited_arg_braced_group_is_data() {
        // 定界实参内容含平衡组：`#1` = A{B}C（组是数据，不报 extra }）
        assert_eq!(
            expand("\\def\\foo#1!{\\detokenize{#1}}\\foo A{B}C!").unwrap(),
            "A{B}C"
        );
        // 组内定界符不终止参数（tex.web 整组贡献）：`#1` 吞到组外 `!`
        assert_eq!(
            expand("\\def\\foo#1!{\\detokenize{#1}}\\foo X{A!B}Y!").unwrap(),
            "X{A!B}Y"
        );
    }

    #[test]
    fn delimited_arg_nested_groups_do_not_error() {
        // 组套组：内层 `}` 依次配对，全部是数据。
        // 实参整体恰为单组 → tex.web Tidy up（m=1）剥外层花括号：`#1` =
        // A{B{C}}D（内层组原样保留）。macro_call 只剥一层、只剥整体单组。
        assert_eq!(
            expand("\\def\\foo#1;{\\detokenize{#1}}\\foo {A{B{C}}D};").unwrap(),
            "A{B{C}}D"
        );
        // 顶层混入其他 token（m≥1）→ 不剥：`\a{x}y!` = `{x}y`
        assert_eq!(
            expand("\\def\\foo#1!{\\detokenize{#1}}\\foo {A}B!").unwrap(),
            "{A}B"
        );
        // 整体单组嵌两层：只剥最外层 → `{A}`
        assert_eq!(
            expand("\\def\\foo#1!{\\detokenize{#1}}\\foo {{A}}!").unwrap(),
            "{A}"
        );
    }

    #[test]
    fn delimited_arg_unknown_cs_in_content_is_data_not_executed() {
        // 尾参数据含"可展开但未定义"的宏（l3prg 场景的 `\tl_if_empty:nF`）：
        // 定界扫描不得就地执行它——undefined-cs 报错即扫描把数据当执行位的表征
        let (r, t) = run_transcript(concat!(
            "\\def\\smark{\\relax}",
            "\\def\\gen#1\\smark{\\detokenize{#1}}",
            "\\gen\\tl_if_empty:nF {p} {oops} \\smark"
        ));
        assert!(r.is_ok(), "应可恢复运行");
        assert!(
            !t.contains("Undefined control sequence"),
            "尾参数据不应被就地执行：{t}"
        );
        assert!(
            !t.contains("has an extra"),
            "组内右花括号不应触发 extra 右花括号 报错：{t}"
        );
    }

    #[test]
    fn delimited_arg_undefined_delimiter_matches_by_token_not_meaning() {
        // 定界符与尾参首 token **都未定义**（expl3 `\s__prg_stop` 定界 +
        // l3tl 未载入时的 `\tl_if_empty:nF` 数据）：定界符按 token 同一匹配，
        // 未定义的 `\tl_if_empty:nF` 不得因"同为未定义"被误作定界符提前终止
        //（tex.web `cur_tok=info(r)` 是 token 相等，非 \ifx 含义相等）。
        assert_eq!(
            expand(concat!(
                "\\def\\gen#1\\smark{\\detokenize{#1}}",
                "\\gen\\notdefinedcs {p} {oops}\\smark"
            ))
            .unwrap(),
            "\\notdefinedcs {p} {oops}"
        );
        // 对照：`\string` 只能看单 token；这里用另一个**已定义**的 cs 开头，
        // 确认定界符仍只认同名 token 出现时才终止
        assert_eq!(
            expand(concat!(
                "\\let\\smarkX\\relax",
                "\\def\\gen#1\\smarkX{\\detokenize{#1}}",
                "\\gen\\relax {mid}\\smarkX"
            ))
            .unwrap(),
            "\\relax {mid}"   // \relax 与 \smarkX 含义同为 relax，但不作定界符
        );
    }

    // ---------- LaTeX 兼容第十五刀：0 参数宏定界串匹配 + \if 操作数展开（报告 §21） ----------

    #[test]
    fn zero_param_macro_delimiter_text_is_matched() {
        // tex.web macro_call（L7971 `if info(r)<>end_match_token`）：参数文本
        // 非空的 **0 参数宏**在调用点匹配纯定界串（"simply scan the delimiter
        // string"）。expl3 条件生成器 fast form `\__prg_F_true:w\fi:\use:n`
        // （expl3-code L1793）依赖它吞掉 `\fi: \use:n` 并由体首 `\fi:` 闭合所在
        // 条件——旧实现直接返回空实参，`\use:n` 泄出被执行，`\cs_if_free:N`
        // 对未定义 cs 误判"已定义" → kernel command-already-defined bail out。
        assert_eq!(
            expand(concat!(
                "\\catcode`\\:=11 \\catcode`\\_=11 ",
                "\\long\\def\\use:none:n#1{} ",
                "\\long\\def\\use:n#1{#1} ",
                "\\def\\prg:Ftrue:w\\fi:\\use:none:n{\\fi:\\use:none:n} ",
                "\\iftrue\\prg:Ftrue:w\\fi:\\use:none:n{LEAK}\\fi OK"
            ))
            .unwrap()
            .trim(),
            "OK"
        );
    }

    #[test]
    fn zero_param_macro_delimiter_mismatch_ignores_call() {
        // 定界串失配 → "Use of macro doesn't match its definition."、调用被忽略，
        // 后续 token 不被吞（tex.web macro_call abort 分支）。
        let (r, t) = run_transcript(concat!(
            "\\def\\delim:X\\fi:\\use:none:n{ body } ",
            "\\delim:X\\relax LEAK"
        ));
        assert!(r.is_ok(), "失配是可恢复错误：{t}");
        assert!(t.contains("doesn't match its definition"), "{t}");
    }

    /// 第十六刀原始构造（模拟 expl3 `\__cs_generate_variant_loop_base:N`
    /// idiom，expl3-code L2820-2834）的 ground truth 回归锁。`\loopa` 体内
    /// 第二个 `\fi:` 是多余收口，且嵌套真支 `n` 前的空格才是外层操作数——
    /// 真实 TeX 由此报 3 个可恢复错误（`! Extra \fi` → `! Extra \else` →
    /// `! Extra \fi`）并输出 SAMEDIFF；引擎须与之逐点一致（此前测试臆想的
    /// SAME 期望值即本文件 CI 失败的根因）。
    // ---------- LaTeX 兼容第十四刀：参数文本 `#{` hash_brace 语义（报告 §20） ----------

    #[test]
    fn hash_brace_makes_last_param_brace_delimited() {
        // expl3 p 型签名 `#1#2#3#4#`：参数文本以 `#` 紧接 body `{` 收尾 → 末参
        // #4 是**分隔实参**、定界符 = 字面 `{`（tex.web scan_toks hash_brace）。
        // 调用 `... #1 {rest}`：p-arg #4 = `# 1` 两 token（`{` 作定界符被消费、
        // 不开组），而非旧实现的"无分隔单 token #4 = 单个 `#`"。
        assert_eq!(
            expand(concat!(
                "\\def\\parm:NNNpnn #1#2#3#4#{X\\detokenize{#4}Y}",
                "\\parm:NNNpnn ABC#1{rest}"   // A B C 三个无分隔单 token；无空格（plain
                                              // catcode 下空格是 token，expl3 里是 cat 9）
            ))
            .unwrap(),
            "X#1Yrest"   // #4=`#1`（`{` 作定界符消费）；体尾 hash_brace `{` 与源 `}` 配成组，rest 在组内
        );
    }

    #[test]
    fn hash_brace_keeps_trailing_space_out_of_string() {
        // tex.web `\string` 用 sprint_cs（控制序列名后**不**补空格）；补空格是
        // e-TeX `\detokenize` 语义。expl3 `\cs_to_str:N`/`\cs_split_function:N`
        // 依赖无空格签名——否则条件生成器的 csname 变成含空格的
        // `\cs_if_exist:NTF `（expl3-code L1907 p-arg 生成即坏）。
        assert_eq!(expand("\\edef\\x{\\string\\foo}\\x").unwrap(), "\\foo");
        assert_eq!(
            expand("\\edef\\x{\\detokenize{\\foo}}\\x").unwrap(),
            "\\foo "
        );
        // 对照（字符不受影响）：\string 与 \detokenize 对普通字符输出一致
        assert_eq!(expand("\\edef\\x{\\string a}\\x").unwrap(), "a");
    }

    #[test]
    fn string_of_csname_internal_space_keeps_space_catcode() {
        // tex.web print_char 对字符码 32 产出 space token。`\csname a b\endcsname`
        // 经 `\string` 后，名字内部空格也必须是 cat 10；LaTeX lthooks 的
        // `\__hook_make_name:w #1 \tl_to_str:n { __hook~ } { }` 定界符正依赖这一点。
        let (r, t) = run_transcript(concat!(
            "\\catcode`\\~=10 ",
            "\\escapechar=-1 ",
            "\\expandafter\\edef\\expandafter\\s\\expandafter",
            "{\\expandafter\\string\\csname a b\\endcsname}",
            "\\def\\test#1~#2X{\\message{SPACE}}",
            "\\expandafter\\test\\s X"
        ));
        assert!(r.is_ok(), "转录：{t}");
        assert!(t.contains("SPACE"), "csname 内部空格应保持 cat 10：{t}");
        assert!(!t.contains("Runaway"), "csname 内部空格未匹配 space 定界符：{t}");
    }

    #[test]
    fn latex_hook_make_name_strips_internal_prefix() {
        // latex.ltx lthooks.dtx 2ekernel：`\token_to_str:N` 作用于
        // `\csname __hook <name>\endcsname` 后，`\tl_to_str:n { __hook~ }`
        // 作为分隔符剥掉内部前缀，留下真实 hook 名。修复前 `\string` 把 csname
        // 内部空格吐成 cat 12，分隔符最后的 cat 10 空格匹配失败，50+ 个 hook
        // 构造在右花括号处报 extra `}`。
        assert_eq!(
            expand(concat!(
                "\\catcode`\\_=11 \\catcode`\\:=11 \\catcode`\\~=10 \\catcode32=9 ",
                "\n",
                "\\escapechar=-1 ",
                "\\let\\exp_after:wN\\expandafter",
                "\\let\\token_to_str:N\\string",
                "\\let\\cs:w\\csname",
                "\\let\\cs_end:\\endcsname",
                "\\let\\tl_to_str:n\\detokenize",
                "\\let\\cs_new:Npn\\def",
                "\\def\\exp_last_unbraced:NNNNo#1#2#3#4#5",
                "{\\exp_after:wN#1\\exp_after:wN#2\\exp_after:wN#3\\exp_after:wN#4#5}",
                "\\cs_new:Npn\\__hook_make_name:n#1",
                "{\\exp_after:wN\\exp_after:wN\\exp_after:wN\\__hook_make_name:w",
                "\\exp_after:wN\\token_to_str:N\\cs:w __hook~ #1\\cs_end:}",
                "\\exp_last_unbraced:NNNNo\\cs_new:Npn\\__hook_make_name:w",
                "#1\\tl_to_str:n{__hook~}{}",
                "\\__hook_make_name:n{begindocument/end}",
            ))
            .unwrap(),
            "begindocument/end"
        );
    }

    #[test]
    fn edef_stores_group_primitives_and_aliases_verbatim() {
        // 第二十刀（pdftex 1.40.29 GT /tmp/k20/e5.tex T1-T4）：tex.web scan_toks
        // 的体终止符只有字符 `}`（cat 2）；`\begingroup`/`\endgroup` 原语及其
        // `\cs_new_eq:NN` 别名**原样存储**，不参与 unbalance 配平、不终止扫描。
        // 旧实现把 Primitive(EndGroup) 计入配平并在 unbalance==0 时截断：
        // `\use:e`（=`\expanded`）实参在首个 `\group_end:` 处交付为空，lthooks
        // 归一化链（`\group_begin: \use:e { \group_end: … }` 惯用法）余 token
        // 落主循环被排版 → nullfont 766 条 Missing character + 实参扫描失衡。
        // 字符别名 `\let\egroup=}`（etrip.tex 29-34）EqSlot 为 Char，本就原样收集。
        assert_eq!(
            expand("\\let\\ge\\endgroup\\edef\\a{\\ge X}\\detokenize\\expandafter{\\a}").unwrap(),
            "\\ge X"
        );
        assert_eq!(
            expand("\\edef\\a{\\begingroup X\\endgroup}\\detokenize\\expandafter{\\a}").unwrap(),
            "\\begingroup X\\endgroup "
        );
        // \expanded 同语义（`\use:e` 真路径）：`\group_end:` 存储不截断。
        // 测试上下文初表 `:`/`_` 是 cat12——先设 cat11 才能构成单一 cs 名；
        // 空格 cat9（house idiom，同 hook 测试）防主流程游离空格混进输出。
        assert_eq!(
            expand(
                "\\catcode`\\_=11 \\catcode`\\:=11 \\catcode32=9 \
                 \\let\\group_begin:\\begingroup \\let\\group_end:\\endgroup \
                 \\def\\use:e#1{\\tex_expanded:D{#1}} \
                 \\long\\gdef\\gfn#1{(#1)} \
                 \\group_begin: \
                 \\edef\\res{\\group_end: \\noexpand\\gfn { para/plain }} \
                 \\detokenize\\expandafter{\\res}"
            )
            .unwrap(),
            "\\group_end: \\gfn {para/plain}"
        );
    }

    #[test]
    fn dimen_number_loop_steps_conditional_machine_on_fi() {
        // 第二十刀错误恢复轨（latex.ltx l.8912 \GenericError 体）：
        // `\dimen@\ifx\@TeXversion\@undefined 4\else\@TeXversion\fi\p@` ——
        // 数字循环遇条件终结符须步进条件机，否则 `\fi` 放回挡住「数量乘
        // 内部量」探针，`\p@` 泄漏主流被当赋值目标再扫数字 → Missing number
        // 恢复残流污染全链（766 条 Missing character、主帧停 71.7%）。
        let (r, t) = run_transcript(
            "\\dimen1=3pt\n\\dimen0=\\ifx\\a\\b 3\\else 9\\fi\\dimen1\n\\message{D=\\the\\dimen0}",
        );
        assert!(r.is_ok(), "{r:?}");
        // \ifx 对两个未定义 cs 判等 → 真分支取 3，3 × 3pt（\dimen1 数量乘内部量）
        assert!(t.contains("D=9.0pt"), "{t}");
        assert!(!t.contains("Missing number"), "{t}");
    }

    #[test]
    fn dimen_flushes_cond_frame_before_internal_quantity() {
        // 第二十刀错误恢复轨（scan.rs 数量探针前的 flush 循环）：chardef 因子
        // 路径数字循环不跑，本扫描开启的条件帧遗留到数量探针前——`\fi` 挡在
        // `\dimen1` 前面使探针失配、寄存器名泄漏主流。仅步进本扫描可闭的帧，
        // 游离终结符照旧放回。
        let (r, t) = run_transcript(
            "\\chardef\\Z=2\n\\dimen1=3pt\n\\dimen0=\\Z\\ifx\\a\\b\\fi\\dimen1\n\\message{D=\\the\\dimen0}",
        );
        assert!(r.is_ok(), "{r:?}");
        assert!(t.contains("D=6.0pt"), "{t}");
        assert!(!t.contains("Missing number"), "{t}");
    }

    #[test]
    fn the_reads_uc_sf_code_tables() {
        // 第二十刀（utf8.def l.148-154 `\uccode`\noexpand\~=\the\uccode`\~`）：
        // `\the` 补 uc/sf code 读臂（tex.web scan_something_internal 的
        // uc_code/sf_code 分支）。缺臂使 utf8.def 预载的
        // `\edef\reserved@a{…\the\uccode`\~…}` 落兜底错误，格式引导止步
        // latex.ltx l.22586 utf8 区。
        assert_eq!(expand("\\uccode`\\~=100 \\edef\\x{\\the\\uccode`\\~}\\x").unwrap(), "100");
        assert_eq!(expand("\\sfcode`\\A=999 \\edef\\x{\\the\\sfcode`\\A}\\x").unwrap(), "999");
    }

    #[test]
    fn latex_hook_normalize_use_e_keeps_names_inside_arguments() {
        // 第二十刀：真实 lthooks 不是直接把 `\__hook_make_name:n` 排到主流，
        // 而是在 `\use:e { \exp_not:N #1 #2 }` 里归一化成目标宏的 braced
        // 实参。旧实现让 `\__hook_make_name:w` 留下的 `para/before` 等返回值
        // 逃出 `\expanded` 收集，落到排版流（nullfont 766 条 Missing character）。
        assert_eq!(
            expand(concat!(
                "\\catcode`\\_=11 \\catcode`\\:=11 \\catcode`\\~=10 \\catcode32=9 ",
                "\\escapechar=-1 ",
                "\\let\\exp_after:wN\\expandafter",
                "\\let\\token_to_str:N\\string",
                "\\let\\cs:w\\csname",
                "\\let\\cs_end:\\endcsname",
                "\\let\\tl_to_str:n\\detokenize",
                "\\let\\tex_expanded:D\\expanded",
                "\\let\\exp_not:N\\noexpand",
                "\\let\\group_begin:\\begingroup",
                "\\let\\group_end:\\endgroup",
                "\\def\\use:e#1{\\tex_expanded:D{#1}}",
                "\\def\\cs_gset:Npn{\\long\\gdef}",
                "\\def\\cs_new:Npn#1{\\cs_gset:Npn#1}",
                "\\def\\exp_last_unbraced:NNNNo#1#2#3#4#5",
                "{\\exp_after:wN#1\\exp_after:wN#2\\exp_after:wN#3\\exp_after:wN#4#5}",
                "\\cs_new:Npn\\__hook_make_name:n#1",
                "{\\exp_after:wN\\exp_after:wN\\exp_after:wN\\__hook_make_name:w",
                "\\exp_after:wN\\token_to_str:N\\cs:w __hook~ #1\\cs_end:}",
                "\\exp_last_unbraced:NNNNo\\cs_new:Npn\\__hook_make_name:w",
                "#1\\tl_to_str:n{__hook~}{}",
                "\\def\\target#1#2#3{(#1)(#2)(#3)}",
                "\\def\\norm#1#2{\\group_begin:\\use:e{\\group_end:\\exp_not:N#1#2}}",
                "\\norm\\target{{\\__hook_make_name:n{para/before}}",
                "{\\__hook_make_name:n{para/after}}{0}}",
            ))
            .unwrap(),
            "(para/before)(para/after)(0)"
        );
    }

    #[test]
    fn expanded_primitive_expands_like_edef() {
        // pdfTeX \expanded{...}：组内容按 \edef 语义全展开（第十二刀新增原语；
        // expl3 L196 引擎门闩与 l3names 别名表要求它存在）
        assert_eq!(expand("\\def\\a{X}\\expanded{\\a Y}").unwrap(), "XY");
        assert_eq!(expand("\\def\\b{42}\\expanded{\\number\\b}").unwrap(), "42");
        assert_eq!(
            expand("\\def\\a{1}\\expanded{\\ifnum\\a=1 yes\\else no\\fi}").unwrap(),
            "yes"
        );
    }

    #[test]
    fn expanded_takes_literal_hash_without_ipn() {
        // tex.web scan_toks(macro_def=false)：\expanded 实参**不做参数 # 处理**
        // ——字面 #（含 cat 6）原样收集，不报 Illegal parameter number，## 亦不
        // 折叠。expl3-code l.9356-9372 经 \lowercase 构造 catcode 查表时
        // #（cat 6）进入 \expanded 实参即依赖此语义（latex.ltx --initex
        // 256 条 IPN 的根因，2026-09-08 修复）。
        // 实参经 \toks0 捕获（cat 6 字符主循环不可排版，不能裸落输出）。
        let (r, t) = run_transcript("\\expanded{\\toks0={a#b}}");
        assert!(r.is_ok(), "转录：{t}");
        assert!(!t.contains("Illegal parameter number"), "不应报 IPN：{t}");
        let (r2, t2) = run_transcript("\\expanded{\\toks0={##}}");
        assert!(r2.is_ok(), "转录：{t2}");
        assert!(!t2.contains("Illegal parameter number"), "## 不折叠不报 IPN：{t2}");
        // 语义锁：\expanded 与 \edef 体就此分流——同内容进 \edef 体（macro_def
        // 模式）**必须**仍报 IPN（真 TeX 同样报）。
        let (_, t3) = run_transcript("\\edef\\x{a#b}");
        assert!(t3.contains("Illegal parameter number"), "\\edef 体应报 IPN：{t3}");
    }

    #[test]
    fn lowercase_converts_active_char_keeping_active() {
        // tex.web shift_case：active char（cs_token_flag+active 区槽位 < 
        // cs_token_flag+single_base）施表**换字符码、保持 active**。pdftex 对拍
        //（2026-09-08 /tmp/ntex-r29/probe_a.tex：`\lccode126=35 \lowercase{~}`）
        // 产物报 `! Undefined control sequence. <recently read> #`——active char
        // 35，而非 cat 6 字符（那会报 "You can't use macro parameter character"）。
        // 引擎断言（pdftex 定标 2026-09-18 /tmp/corpus/lc.tex，两引擎逐字
        // 一致）：active 槽与同名 cs 槽**互不可见**（tex.web active 区独立于
        // hash 区），active-a 未定义 → 主循环报 Undefined control sequence。
        // 旧断言 `YES` 依赖共槽伪影（active-a 落进 \a 的 cs 槽）——正是
        // latex.ltx `\gdef_{\_}` 覆盖 robust `\_` 自噬死循环的根因，已废。
        let (_, t0) = run_transcript("\\def\\a{YES}\\lccode126=97\\relax\\lowercase{~}");
        assert!(
            t0.contains("! Undefined control sequence."),
            "active-a 不得命中 cs \\a 槽（pdftex 同报）：{t0}"
        );
        // pdftex 对拍形态：转换产物 active-#（csid "#" 未定义）主循环报
        // Undefined control sequence——保持 cat 13，未降为 cat 6 字符。
        let (_, t) = run_transcript("\\lccode126=35\\relax\\lowercase{~}");
        assert!(t.contains("! Undefined control sequence."), "转录：{t}");
        assert!(
            !t.contains("macro parameter character"),
            "产物应是 active char 而非 cat 6 字符：{t}"
        );
        // lccode=0 不转换（tex.web equiv=0 跳过）：~ 保持 active-~ 未定义
        let (_, t2) = run_transcript("\\lccode126=0\\relax\\lowercase{\\toks0={~}}");
        assert!(!t2.contains("! Undefined control sequence."), "未施表不落主循环：{t2}");
    }

    #[test]
    fn csname_terminates_on_endcsname_meaning_not_name() {
        // scan_csname 按**含义**终止（tex.web cur_cmd=end_csname）：expl3 的
        // `\cs_end:` 是 `\endcsname` 别名（槽 = EndCsname 原语），名字不同也应
        // 闭合 `\csname`。旧实现按名 "endcsname" 判定 → `\cs_end:` 报
        // Missing endcsname 级联（l3prg `\use:c{…\cs_end:}` 全炸，报告 §18）。
        let src = concat!(
            "\\catcode58=11 \\catcode95=11 ", // `:`/`_` 变字母 → `\cs_end:` 是单 cs
            "\\let\\cs_end:\\endcsname ",
            "\\expandafter\\ifx\\csname ab\\cs_end:\\relax T\\else F\\fi"
        );
        assert_eq!(expand(src).unwrap(), "T");
        let (r, t) = run_transcript(src);
        assert!(r.is_ok());
        assert!(!t.contains("Missing endcsname"), "\\cs_end: 应闭合 csname：{t}");
        assert!(!t.contains("Extra \\endcsname"), "不应泄漏 \\endcsname：{t}");
    }

    #[test]
    fn globaldefs_param_adjusts_assignment_scope() {
        // tex.web prefixed_command：\globaldefs>0 → 所有赋值隐式全局；<0 →
        // 取消显式 \global（局部化）
        assert_eq!(
            expand("\\count0=0\\begingroup\\globaldefs=1 \\count0=5\\endgroup\\the\\count0").unwrap(),
            "5"
        );
        assert_eq!(
            expand("\\count1=0\\begingroup\\globaldefs=-1 \\global\\count1=6\\endgroup\\the\\count1")
                .unwrap(),
            "0"
        );
    }

    /// 0 参数宏的**纯定界串参数文本**（`\def\X\fi:\use:n{...}`）在展开上下文
    /// （`\edef`/`\csname` 名字扫描）也须在调用点匹配并吞掉。
    ///
    /// tex.web macro_call `if info(r)<>end_match_token then @<Scan the
    /// parameters@>`——参数文本非空时 0 参数宏同样走参数匹配；`\else`/`\fi`
    /// 在实参扫描里是**数据**（get_token，不推进条件机）。expl3 条件生成器
    /// fast form（`\__prg_T_true:w`/`\__prg_F_true:w`/`\__prg_TF_true:w`/
    /// `\__prg_p_true:w`）即此形态：体首 `\fi:` 负责闭合调用点的条件帧。
    /// 旧实现 expand_once/scan_csname 两处对 `num_params==0` 直接跳过匹配，
    /// 定界串泄给条件机 → 帧被提前弹掉 + "Extra \fi."，expl3-code.tex
    /// l.7934 起 `\str_const:Ne` 区级联、`\str_case` 全线 extra-} 失衡
    /// （错误 7016 → 修复后 1835，终态行号 l.8073 → l.9353）。
    #[test]
    fn zero_param_macro_delimiter_text_consumed_in_expansion_contexts() {
        // l3kernel fast form 的最小同构：T 分支选择器（expl3 catcode：`_`/`:` 为字母）。
        // 执行路径（call_macro）旧测已盖；此处盖**展开上下文**的两处调用点：
        // \edef 体扫描（expand_once）与 \csname 名字扫描（scan_csname）。
        let setup = concat!(
            "\\catcode`\\_=11 \\catcode`\\:=11 %\n",
            "\\let\\fi:\\fi %\n",
            "\\long\\def\\usei:nn#1#2{#1}%\n",
            "\\long\\def\\use:n#1{#1}%\n",
            "\\long\\def\\use_none:n#1{}%\n",
            "\\long\\def\\T_true:w\\fi:\\use_none:n{\\fi:\\use:n}%\n",
            "\\def\\cond:NT#1{\\ifdefined#1\\T_true:w\\fi:\\use_none:n}%\n",
            "\\def\\foo{FOO}%\n",
        );
        // \edef：`\T_true:w` 吞掉定界串 `\fi: \use_none:n`，体首 `\fi:` 闭合
        // \ifdefined 帧，`\use:n` 取真分支 → 结果 YES（旧实现帧被提前弹掉，
        // 报 "Extra \fi." 且结果为空）
        assert_eq!(
            expand(&format!(
                "{setup}\\edef\\res{{\\cond:NT\\foo{{YES}}}}\\res"
            ))
            .unwrap(),
            "YES"
        );
        // 假分支：`\ifdefined` 为假 → 跳到 `\fi`，`\use_none:n` 吞掉分支组
        assert_eq!(
            expand(&format!(
                "{setup}\\edef\\res{{\\cond:NT\\undef{{NO}}}}\\res"
            ))
            .unwrap(),
            ""
        );
        // \csname 名字扫描：不消费定界串会把 `\fi:` 泄进名字文本
        //（"Missing endcsname inserted"）；正确行为下名字 = X+YES
        assert_eq!(
            expand(&format!(
                "{setup}\\edef\\res{{\\expandafter\\string\\csname X\\cond:NT\\foo{{YES}}\\endcsname}}\\res"
            ))
            .unwrap(),
            "\\XYES"
        );
    }

    // ---------- M3-2-2 内部参数 ----------

    #[test]
    fn param_assignment_and_the() {
        // 单位尾 scan_optional_space（get_x_token）就地展开 \the → 打印赋值前
        // 旧值（INITEX：parindent=0、baselineskip=12pt、lineskip/limit=0；
        // pdfTeX GT gtf/gf b5-b8 plain 旧值 20/12/1/0 同机制）
        assert_eq!(expand("\\parindent 20pt\\the\\parindent").unwrap(), "0.0pt");
        assert_eq!(
            expand("\\baselineskip 10pt plus 2pt\\the\\baselineskip").unwrap(),
            "12.0pt"
        );
        assert_eq!(expand("\\lineskip 3pt\\the\\lineskip").unwrap(), "0.0pt");
        assert_eq!(
            expand("\\lineskiplimit -1pt\\the\\lineskiplimit").unwrap(),
            "0.0pt"
        );
    }

    #[test]
    fn param_defaults() {
        assert_eq!(expand("\\the\\parindent").unwrap(), "0.0pt");
        assert_eq!(expand("\\the\\baselineskip").unwrap(), "12.0pt");
        assert_eq!(expand("\\the\\lineskip").unwrap(), "0.0pt");
        assert_eq!(expand("\\the\\lineskiplimit").unwrap(), "0.0pt");
    }

    #[test]
    fn param_local_scoped_at_group_end() {
        let src = "\\parindent 20pt\\begingroup\\parindent 30pt\\endgroup\\the\\parindent";
        assert_eq!(expand(src).unwrap(), "20.0pt");
    }

    #[test]
    fn param_global_scoped() {
        let src = "\\parindent 20pt\\begingroup\\global\\parindent 30pt\\endgroup\\the\\parindent";
        assert_eq!(expand(src).unwrap(), "30.0pt");
    }

    #[test]
    fn param_afterassignment_fires() {
        // \afterassignment 在参数赋值后触发（与寄存器一致）
        let src = "\\def\\x{Y}\\afterassignment\\x\\parindent 10pt\\the\\parindent";
        // 无尾空格：\the 落在单位尾 scan_optional_space 里被就地展开 → 打印
        // 赋值前旧值 0.0pt（pdfTeX GT gtf/gf b19 为带尾空格版 → "Y10.0pt" 新值）
        assert_eq!(expand(src).unwrap(), "Y0.0pt");
    }

    // ---------- M4-5 e-TeX 展开扩展 ----------

    #[test]
    fn protected_macro_not_expanded_in_edef() {
        // \protected\def\foo{Hi} → \edef\x{\foo} 时 \foo 不展开，\x = \foo
        let out = expand(r"\protected\def\foo{Hi}\edef\x{\foo}\expandafter\detokenize\expandafter{\x}")
            .unwrap();
        assert_eq!(out, r"\foo ", "宏 x 应保留 \\foo（detokenize 控制词后补空格）而非展开为 Hi");
    }

    #[test]
    fn unprotected_macro_expands_in_edef() {
        let out = expand(r"\def\foo{Hi}\edef\x{\foo}\expandafter\detokenize\expandafter{\x}").unwrap();
        assert_eq!(out, "Hi");
    }

    #[test]
    fn protected_macro_not_expanded_in_write() {
        // \write 参数展开抑制 protected 宏：\foo 不展开，输出为空（TeX 语义丢弃）
        let vfs = MemVfs::new();
        let (_, vfs) = expand_vfs(
            r"\protected\def\foo{Hi}\newwrite\w\openout\w=out.txt\write\w{\foo}\closeout\w",
            vfs,
        )
        .unwrap();
        assert_eq!(
            vfs.get("out.txt").map(|b| String::from_utf8_lossy(b).into_owned()),
            Some("\\foo \n".to_owned()),
            "protected 宏不展开 → detokenize 打印 \\foo（pdfTeX ground truth：out.txt = \"\\foo \\n\"，2026-09-12 实测；旧期望空行系 \\write cs-丢弃错误语义）"
        );
    }

    #[test]
    fn protected_macro_still_expands_normally() {
        // 正常展开（非抑制上下文）不受影响
        assert_eq!(expand(r"\protected\def\foo{Hi}\foo").unwrap(), "Hi");
    }

    #[test]
    fn csname_undefined_becomes_relax() {
        // latex.ltx L1113 expl3 门闩语义（tex.web L7753-7754）：\csname 对未定义名
        // eq_define(cs,relax,256)——与 \relax 原语同义，\ifx 相等。
        // 注意需 \expandafter：\ifx 操作数不展开（TeX 语义），裸 \ifx\csname… 比较的
        // 是 \csname 原语自身。
        assert_eq!(
            expand(r"\expandafter\ifx\csname nope\endcsname\relax T\else F\fi").unwrap(),
            "T"
        );
        // 制造后定义持久：\csname 产物被执行是 no-op（不报 Undefined），且 \ifdefined 为真
        // （名字须全字母——`\nope2` 在正文会切成 `\nope`+`2`；带数字名只能用 \csname 再取）
        assert_eq!(
            expand(r"\csname nopecs\endcsname\ifdefined\nopecs yes\else no\fi").unwrap(),
            "yes"
        );
        // 数字等非字母字符可入名，但只能经 \csname 再引用（TeX 语义同）
        assert_eq!(
            expand(r"\csname nope2\endcsname\ifdefined\nope no\else\expandafter\ifx\csname nope2\endcsname\relax T\else F\fi\fi").unwrap(),
            "T"
        );
        // \edef 上下文（expand_once 路径）同样制造 relax：\a 展开为 \qqq（relax 同义）
        assert_eq!(
            expand(r"\edef\a{\csname qqq\endcsname}\expandafter\ifx\a\relax T\else F\fi")
                .unwrap(),
            "T"
        );
        // TeX 2.9 "relax local"：组内制造 → 组末恢复未定义（\ifdefined 假）
        assert_eq!(
            expand(r"{\csname grpname\endcsname}\ifdefined\grpname yes\else no\fi").unwrap(),
            "no"
        );
        // \meaning：制造出的 relax 不再显示 "undefined"（引擎对 Primitive 槽显示
        // `\名`——记录偏差：TeX 对 \meaning\nope 输出 "relax"（按含义），引擎按
        // 当前 cs 名显示 `\nope`；\relax 原语因名恰为 relax 故二者仅此处有差）
        assert!(
            !expand(r"\expandafter\meaning\csname nope\endcsname")
                .unwrap()
                .contains("undefined"),
            r"制造后 \meaning 不应显示 undefined"
        );
        // 直接使用真正未定义 cs 仍是报错恢复（不经 \csname 制造不产生定义）
        assert_eq!(
            expand(r"\ifdefined\directundefinedzzz yes\else no\fi").unwrap(),
            "no"
        );
    }

    #[test]
    fn detokenize_converts_to_character_tokens() {
        assert_eq!(expand(r"\detokenize{abc}").unwrap(), "abc");
        // 控制序列 → \名字 文本
        assert_eq!(
            expand(r"\detokenize{a\relax b}").unwrap(),
            r"a\relax b"
        );
    }

    #[test]
    fn unexpanded_in_edef_keeps_tokens() {
        // \unexpanded{\foo} 在 \edef 里不展开 → \x = \foo
        let out = expand(
            r"\def\foo{Hi}\edef\x{\unexpanded{\foo}}\expandafter\detokenize\expandafter{\x}",
        )
        .unwrap();
        assert_eq!(out, r"\foo ");
    }

    #[test]
    fn everycr_injected_at_preamble_end_and_fin_row() {
        // tex.web L15339/L15732：\everycr 在两处注入——preamble 扫完
        // （init_align 尾）+ 每行 fin_row，且**先于 align_peek 的前瞻**。
        // amsmath 形态 `\everycr{\noalign{…}}`（halign-survey §2.3 p3g）每行
        // 重置标签、kernel `\ialign`/`\eqnarray` 的 `\everycr{}` 清空语义都
        // 依赖它。此前只存不注入（S1/G1）。
        let src = concat!(
            "\\everycr={\\noalign{\\message{N}}} ",
            "\\halign{\\hfil#\\hfil\\cr a\\cr b\\cr}"
        );
        let (_r, t) = run_transcript(src);
        let count = t.matches('N').count();
        // 1（preamble 扫完）+ 2（每行 fin_row）= 3
        assert_eq!(count, 3, "\\everycr 注入次数（1 preamble + 2 fin_row）：{t}");
        // 空表注入为无操作（kernel \ialign 第一步 \everycr{} 清空）
        let src2 = "\\everycr={}\\halign{\\hfil#\\hfil\\cr a\\cr}";
        let (_r2, t2) = run_transcript(src2);
        assert!(
            !t2.contains("! Undefined") && !t2.contains("Missing"),
            "\\everycr 空表不应产生任何错误：{t2}"
        );
    }

    // ── \newif / \escapechar 展开链（plain.tex L598 预载 24 条 "doesn't
    // match" 根因）──────────────────────────────────────────────────────
    // plain.tex L264-271 的 \newif 定义压缩成单行（@=11、\count@=255、
    // \m@ne=22；见 crates/ntex-layout/resources/plain.tex L47/206/212/264-271）。
    const PLAIN_NEWIF_DEFS: &str = concat!(
        r"\catcode`@=11 ",
        r"\countdef\count@=255 ",
        r"\countdef\m@ne=22 \m@ne=-1 ",
        r"\outer\def\newif#1{\count@\escapechar \escapechar\m@ne ",
        r"\expandafter\expandafter\expandafter \def\@if#1{true}{\let#1=\iftrue} ",
        r"\expandafter\expandafter\expandafter \def\@if#1{false}{\let#1=\iffalse} ",
        r"\@if#1{false}\escapechar\count@} ",
        r"\def\@if#1#2{\csname\expandafter\if@\string#1#2\endcsname} ",
        r"{\uccode`1=`i \uccode`2=`f \uppercase{\gdef\if@12{}}} ",
    );

    /// PLAIN_NEWIF_DEFS + 尾串（concat! 不接受 const，用 format 内联捕获）。
    fn nsrc(tail: &str) -> String {
        format!("{PLAIN_NEWIF_DEFS}{tail}")
    }

    fn probe_dual(src: &str) -> (String, String, String) {
        // 返回 (字节码轨道输出, 字节码转录, 解释器转录)——双轨各自独立检查报错
        let mut a = Expander::new();
        a.run_source(src).ok();
        let oa: String = a
            .output()
            .iter()
            .map(|t| t.charcode().and_then(char::from_u32).unwrap_or('\u{FFFD}'))
            .collect();
        let ta = a.transcript().to_string();
        let mut b = Expander::new_interpreter();
        b.run_source(src).ok();
        let tb = b.transcript().to_string();
        (oa, ta, tb)
    }

    #[test]
    fn newif_escapechar_minus1_string_no_leading_escape() {
        // \escapechar=-1 时 \string\iffoo 不带前导 \（tex.web print_esc 0..=255
        // 才打印转义字符）——plain \newif 的 "if" 定界匹配依赖此。修复前硬编码
        // 反斜杠使输出为 "\iffoo"。
        assert_eq!(expand(r"\escapechar=-1 \string\iffoo").unwrap(), "iffoo");
        // 默认 \escapechar=`\\ 时仍带前导 \
        assert_eq!(expand(r"\escapechar=`\\ \string\iffoo").unwrap(), "\\iffoo");
        // \escapechar=256 也不可见
        assert_eq!(expand(r"\escapechar=256 \string\iffoo").unwrap(), "iffoo");
    }

    #[test]
    fn newif_iffoo_defines_true_and_false_conditions() {
        // \newif\iffoo 应制造 \footrue/\foofalse 并把 \iffoo 初始置为 false：
        // \ifx 判等走含义（\iffoo \let 到 \iftrue/\iffalse 原语）。
        let src = nsrc(
            &[
                r"\newif\iffoo ",
                r"\ifx\iffoo\iffalse INIT-F\else INIT-T\fi ", // 初始 false
                r"\footrue ",
                r"\ifx\iffoo\iftrue NOW-T\else NOW-F\fi ", // \footrue 后为 true
                r"\iffoo COND-T\else COND-F\fi ",          // 真分支
            ]
            .concat(),
        );
        let (out, ta, tb) = probe_dual(&src);
        assert_eq!(out.trim(), "INIT-FNOW-TCOND-T", "双轨输出：{out}");
        assert!(!ta.contains("doesn't match"), "字节码轨道报错：{ta}");
        assert!(!tb.contains("doesn't match"), "解释器轨道报错：{tb}");
    }

    #[test]
    fn newif_csname_string_if_prefix_matches() {
        // \@if\iffoo{true} = \csname\expandafter\if@\string#1#2\endcsname（#1=\iffoo、
        // #2=`true` 无空格——参数替换在 token 层，名字不含空格）：\string 先行展开
        // 为字符、\if@ 吞掉 "if" 前缀 → 名字 = "footrue"。修复前 \string 带前导 \
        // 使 \if@ 定界失配、\def 定义到错误 cs，随后 \footrue 未定义。
        // 3 层 \expandafter 与 \newif 同构：E2 把 \@if 展开出的 \csname...\endcsname
        // 先解析成 \footrue token，\def 才吃到目标 cs（\def 本身不展开 \csname）。
        let src = nsrc(
            r"\escapechar=-1 \expandafter\expandafter\expandafter\def\@if\iffoo{true}{MKR}\footrue",
        );
        let (out, ta, tb) = probe_dual(&src);
        assert_eq!(out.trim(), "MKR", "out={out:?} bytecode转录：{ta} 解释器转录：{tb}");
    }

    #[test]
    fn newif_ifus_at_zero_errors() {
        // plain.tex L598 现场（预载 24 条 "doesn't match" 的单个 \newif 复现）：
        // 修复后该行不再报错。
        let src = nsrc(r"\newif\ifus@ ");
        let (_, ta, tb) = probe_dual(&src);
        assert!(!ta.contains("doesn't match"), "字节码轨道报错：{ta}");
        assert!(!tb.contains("doesn't match"), "解释器轨道报错：{tb}");
    }
