use super::*;

    #[test]
    fn input_reads_file_from_vfs() {
        let mut vfs = MemVfs::new();
        vfs.insert("ch1.tex", "Chapter One");
        let (out, _) = expand_vfs("\\input{ch1}", vfs).unwrap();
        assert_eq!(out, "Chapter One");
    }

    #[test]
    fn input_falls_back_to_tex_extension() {
        let mut vfs = MemVfs::new();
        vfs.insert("ch2.tex", "Ch2");
        let (out, _) = expand_vfs("\\input ch2", vfs).unwrap();
        assert_eq!(out, "Ch2");
    }

    #[test]
    fn input_nests_and_returns() {
        let mut vfs = MemVfs::new();
        vfs.insert("a.tex", "A\\input{b}B");
        vfs.insert("b.tex", "X");
        let (out, _) = expand_vfs("\\input{a}", vfs).unwrap();
        assert_eq!(out, "AXB");
    }

    #[test]
    fn input_missing_file_errors() {
        let vfs = MemVfs::new();
        assert!(expand_vfs("\\input{nope}", vfs).is_err());
    }

    #[test]
    fn end_inside_nested_input_terminates_without_stack_overflow() {
        // tex.web final_cleanup：`\end` 是终结信号——无论嵌套多深都必须结束
        // 作业（A1.undevicies 爆栈修复：此前嵌套 \input 内 \end 导致输入栈
        // 无限增长至 5001 帧爆栈，expl3 LVT 111 例 STACK-END 的根因）。
        let mut vfs = MemVfs::new();
        vfs.insert("inner.tex", "X\\end");
        let (out, _) = expand_vfs("\\input{inner}", vfs).unwrap();
        // `\end` 后的 token 不再处理（外层无后续 token 场景）
        assert_eq!(out, "X");
    }

    #[test]
    fn end_in_macro_body_inside_nested_input_terminates() {
        // 更贴近 LVT harness 场景：嵌套 input 内宏展开触发 \end
        let mut vfs = MemVfs::new();
        vfs.insert("shim.tex", "\\def\\myend{\\end}\\input{test}\\myend");
        vfs.insert("test.tex", "OK");
        let (out, _) = expand_vfs("\\input{shim}", vfs).unwrap();
        assert!(out.contains("OK"));
    }

    #[test]
    fn immediate_write_appends() {
        let vfs = MemVfs::new();
        let (_, vfs) = expand_vfs(
            "\\newwrite\\f\\immediate\\openout\\f=out.txt\\immediate\\write\\f{abc}\\closeout\\f",
            vfs,
        )
        .unwrap();
        assert_eq!(vfs.get("out.txt"), Some(b"abc\n".as_slice()));
    }

    #[test]
    fn write_defers_until_end() {
        let vfs = MemVfs::new();
        // 无 \immediate：\write 入队，\end 收尾统一 flush
        let (_, vfs) = expand_vfs(
            "\\newwrite\\f\\openout\\f=out.txt\\write\\f{abc}\\write\\f{def}\\end",
            vfs,
        )
        .unwrap();
        assert_eq!(vfs.get("out.txt"), Some(b"abc\ndef\n".as_slice()));
    }

    #[test]
    fn write_expands_the_at_write_time() {
        let vfs = MemVfs::new();
        // \write 时展开 \the\count0 与宏（TeX 语义：写文件时展开）
        let (_, vfs) = expand_vfs(
            "\\count0=42\\def\\mark{X}\\newwrite\\f\\openout\\f=o.txt\\write\\f{\\the\\count0\\mark}\\end",
            vfs,
        )
        .unwrap();
        assert_eq!(vfs.get("o.txt"), Some(b"42X\n".as_slice()));
    }

    #[test]
    fn write_to_unopened_stream_goes_to_transcript() {
        // tex.web write_out：「write to the terminal if file isn't open」——
        // 未 `\openout` 的 0..15 流（以及流号 >15 的钳制 j=16）在 `\immediate` 下
        // 落终端+log，非丢弃（LaTeX `\write\@unused`/ETRIP `\immediate\write15`
        // 靠这条）。
        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        e.run_source("\\immediate\\write0{abc}").unwrap();
        assert_eq!(e.transcript(), "abc\n");
        // \write16（j=16）同样落终端+log
        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        e.run_source("\\immediate\\write16{term}").unwrap();
        assert_eq!(e.transcript(), "term\n");
        // 非 immediate：延迟写在无 shipout 时忽略（tex.web：whatsit 随页面才执行）
        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        e.run_source("\\write0{abc}").unwrap();
        assert_eq!(e.transcript(), "");
    }

    #[test]
    fn negative_write_stream_is_log_only() {
        // `\write-1`（LaTeX `\wlog`）→ 仅 log（引擎：转录）；j=17 钳制分支。
        // 负流号为既有延迟路径（shipout/结束边界 flush），`\end` 触发 flush。
        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        e.run_source("\\write-1{log only}\\immediate\\write-100000{x}\\end").unwrap();
        assert_eq!(e.transcript(), "x\nlog only\n");
    }

    #[test]
    fn write15_after_openout15_lands_in_file() {
        // LaTeX 内核探测（latex.ltx L176-178）：流 15 是合法文件流——
        // `\immediate\openout15` + `\write15` + `\closeout15` 必须落盘。
        let vfs = MemVfs::new();
        let (_, vfs) = expand_vfs(
            "\\immediate\\openout15=t.aux \\immediate\\write15{hello}\\immediate\\closeout15 \\end",
            vfs,
        )
        .unwrap();
        assert_eq!(vfs.get("t.aux"), Some(b"hello\n".as_slice()));
    }

    #[test]
    fn openout_truncates_stale_file() {
        // tex.web `a_open_out`：\openout 语义为覆盖（首次实际写出清空残留）
        let mut vfs = MemVfs::new();
        vfs.insert("o.aux", "STALE\n");
        let (_, vfs) = expand_vfs(
            "\\newwrite\\f\\openout\\f=o.aux\\write\\f{new}\\closeout\\f",
            vfs,
        )
        .unwrap();
        assert_eq!(vfs.get("o.aux"), Some(b"new\n".as_slice()));
    }

    #[test]
    fn openout_closeout_creates_empty_file() {
        // \openout + \closeout（无 \write）：文件仍产生（tex.web a_open_out→a_close）
        let vfs = MemVfs::new();
        let (_, vfs) = expand_vfs("\\newwrite\\f\\openout\\f=empty.aux\\closeout\\f", vfs).unwrap();
        assert_eq!(vfs.get("empty.aux"), Some(b"".as_slice()));
    }

    #[test]
    fn typeout_via_write17_reaches_transcript() {
        // LaTeX `\typeout` = `\immediate\write17`（latex.ltx L129）→ 终端+log
        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        e.run_source("\\immediate\\write17{Hello world.}").unwrap();
        assert_eq!(e.transcript(), "Hello world.\n");
    }

    #[test]
    fn write_stringification_prints_all_character_catcodes() {
        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        e.run_source("\\catcode`\\_=8 \\catcode`\\^=7 \\catcode`\\#=6 \\immediate\\write16{a_b^c#d{e}}")
            .unwrap();
        assert_eq!(e.transcript(), "a_b^c#d{e}\n");

        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        e.run_source("\\catcode`\\_=8 \\message{a_b}").unwrap();
        assert_eq!(e.transcript(), "a_b");

        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        e.run_source("\\catcode`\\_=8 \\def\\m{a_b}\\show\\m").unwrap();
        assert!(
            e.transcript().contains("->a_b."),
            "show transcript={:?}",
            e.transcript()
        );
    }

    #[derive(Debug, Default)]
    struct SpecialCaptureSink {
        inner: VecSink,
        specials: Vec<String>,
    }

    impl crate::sink::CoreSink for SpecialCaptureSink {
        fn token(&mut self, tok: Token) -> Result<()> {
            self.inner.token(tok)
        }
        fn transcript(&self) -> &str {
            self.inner.transcript()
        }
        fn tokens(&self) -> &[Token] {
            self.inner.tokens()
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
        fn as_any_ref(&self) -> &dyn std::any::Any {
            self
        }
    }

    impl crate::sink::IoSink for SpecialCaptureSink {
        fn message(&mut self, text: String) -> Result<()> {
            self.inner.message(text)
        }
        fn show(&mut self, text: String) -> Result<()> {
            self.inner.show(text)
        }
        fn write16(&mut self, text: String) -> Result<()> {
            self.inner.write16(text)
        }
        fn special(&mut self, text: String) -> Result<()> {
            self.specials.push(text);
            Ok(())
        }
    }

    impl crate::sink::FontSink for SpecialCaptureSink {}
    impl crate::sink::MathSink for SpecialCaptureSink {}
    impl crate::sink::BoxSink for SpecialCaptureSink {}
    impl crate::sink::AlignSink for SpecialCaptureSink {}
    impl crate::sink::PageSink for SpecialCaptureSink {}
    impl crate::sink::TokenSink for SpecialCaptureSink {}

    #[test]
    fn special_stringification_prints_subscript_character() {
        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        e.set_sink(Box::new(SpecialCaptureSink::default()));
        e.run_source("\\catcode`\\_=8 \\special{a_b}").unwrap();
        let sink = e.take_sink();
        let sink = sink
            .as_any_ref()
            .downcast_ref::<SpecialCaptureSink>()
            .unwrap();
        assert_eq!(sink.specials, vec!["a_b".to_string()]);
    }

    #[test]
    fn openout_jobname_uses_host_job_name() {
        // latex.ltx L9652 `\immediate\openout\@mainaux\jobname.aux`：名字扫描里的
        // `\jobname` 必须展开成当前作业名，否则 aux **写**路落 texput.aux，而
        // **读**路（`\InputIfFileExists{\jobname.aux}` 走普通展开）拿真名——
        // 两路文件名错位 ⇒ `\newlabel` 永远读不回 ⇒ 正文 `\ref` 全数 `??`。
        let mut e = Expander::new();
        e.set_job_name("paper-main");
        e.set_vfs(Box::new(MemVfs::new()));
        e.run_source(
            "\\immediate\\openout7=\\jobname.aux\\relax\
             \\immediate\\write7{\\string\\newlabel{x}{{1}{1}}}\
             \\immediate\\closeout7",
        )
        .unwrap();
        let mut vfs = e.take_vfs();
        let vfs = vfs.as_any_mut().downcast_mut::<MemVfs>().unwrap();
        assert_eq!(
            vfs.get("paper-main.aux"),
            Some(b"\\newlabel{x}{{1}{1}}\n".as_slice())
        );
    }

    #[test]
    fn openout_relax_terminator_keeps_name_and_token() {
        // tex.web scan_file_name：非名字字符终止收集并 back_input——名字保留已收集
        // 部分，`\relax` 留在流里照常执行（TRIP L94 同款恢复语义）。
        let vfs = MemVfs::new();
        let (_, vfs) = expand_vfs(
            "\\newwrite\\f\\openout\\f=r.aux\\relax\\write\\f{v}\\closeout\\f \\end",
            vfs,
        )
        .unwrap();
        assert_eq!(vfs.get("r.aux"), Some(b"v\n".as_slice()));
    }

    #[test]
    fn quoted_file_name_is_unquoted() {
        // web2c：带引号文件名剥引号（latex.ltx `\openin\@inputcheck"#1" `）
        let mut vfs = MemVfs::new();
        vfs.insert("q.tex", "Q");
        // 名字后的终止空格留在输入流（引擎既有的文件名扫描偏差）→ 输出 "Q "
        let (out, _) = expand_vfs("\\input\"q\" ", vfs).unwrap();
        assert_eq!(out, "Q ");
        let mut vfs = MemVfs::new();
        vfs.insert("data.txt", "Hello\n");
        // 名字终止空格留在输入流（既有文件名扫描偏差）→ 前导空格。
        // pdfTeX 对拍：引号名后的终止空格确实留在流（\setbox0=\hbox{\input"q" A}
        // 的 box 内容含 `glue 3.33333` + `A`）；\read 行尾空格同上（read_toks）。
        let (out, _) = expand_vfs(
            "\\newread\\r\\openin\\r=\"data.txt\" \\read\\r to \\l\\l",
            vfs,
        )
        .unwrap();
        assert_eq!(out, " Hello ");
    }

    #[test]
    fn openin_chardef_stream_cs_expands_to_number() {
        // latex.ltx `\newread` 最终经 `\chardef` 绑定流 cs；`\openin` 的流号
        // 扫描必须走 scan_int 语义，把 cs 取值为流号，而不是把 cs 名字符当
        // 文件名前缀吞入（曾见 `@inputcheck` + `texsys.aux` 污染）。
        let mut vfs = MemVfs::new();
        vfs.insert("x.tex", "OK\n");
        vfs.insert("scx.tex", "BAD\n");
        let (out, _) = expand_vfs(
            "\\chardef\\sc=3\\openin\\sc=x.tex \\read\\sc to \\line\\line",
            vfs,
        )
        .unwrap();
        assert_eq!(out, " OK ");
    }

    #[test]
    fn openin_chardef_stream_cs_reads_immediate_write() {
        // mini latex_probe 同款：先写 texsys.aux，再用 chardef'd cs 作为 openin/read
        // 流号读回。另放入污染文件，若流 cs 名字符混进文件名会读到 BAD。
        let mut vfs = MemVfs::new();
        vfs.insert("sctexsys.aux", "BAD\n");
        let (out, _) = expand_vfs(
            concat!(
                "\\immediate\\openout15=texsys.aux",
                "\\immediate\\write15{OK}",
                "\\immediate\\closeout15",
                "\\chardef\\sc=3",
                "\\openin\\sc texsys.aux ",
                "\\read\\sc to \\line\\line",
            ),
            vfs,
        )
        .unwrap();
        assert_eq!(out, " OK ");
    }

    #[test]
    fn newread_stream_cs_reads_immediate_write() {
        let vfs = MemVfs::new();
        let (out, _) = expand_vfs(
            concat!(
                "\\immediate\\openout15=texsys.aux",
                "\\immediate\\write15{OK}",
                "\\immediate\\closeout15",
                "\\newread\\r",
                "\\openin\\r texsys.aux ",
                "\\ifeof\\r NO\\else\\read\\r to \\line\\line\\fi",
            ),
            vfs,
        )
        .unwrap();
        assert_eq!(out, " OK ");
    }

    #[test]
    fn braced_file_name_expands_macros() {
        // tex.web scan_file_name 的循环顶是 get_x_token（L10210）：花括号组名
        // 里的宏**在名字扫描内展开**。真现场 graphics.sty `\Gin@getbase` →
        // `\IfFileExists{\Gin@base#1}` —— 组内 `\Gin@base` 是 cs；旧实现裸收
        // token，报「文件名含非法 token：cs \Gin@base」致命（\includegraphics
        // 全线不可用）。组内空格照旧入名（与旧 braced 分支一致）。
        let mut vfs = MemVfs::new();
        vfs.insert("Figures/fig.txt", "X");
        let (out, _) = expand_vfs("\\def\\Gin@base{Figures/fig}\\input{\\Gin@base.txt}", vfs)
            .unwrap();
        assert_eq!(out, "X");
    }

    #[test]
    fn input_missing_file_reports_tex_error() {
        // tex.web prompt_file_name：`! I can't find file \`x'.` + 交互式替换文件名
        // 询问 + batchmode 致命（引擎无交互层 → 终止）
        let vfs = MemVfs::new();
        let err = expand_vfs("\\input{nope}", vfs).unwrap_err();
        assert!(err.to_string().contains("nope"), "错误信息：{err}");
    }

    #[test]
    fn input_missing_file_transcript_block() {
        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        assert!(e.run_source("\\input nope.tex").is_err());
        let t = e.transcript();
        assert!(t.contains("! I can't find file `nope.tex'."), "{t}");
        assert!(t.contains("Please type another input file name"), "{t}");
        assert!(t.contains("! Emergency stop."), "{t}");
        assert!(t.contains("*** (job aborted, file error in nonstop mode)"), "{t}");
    }

    #[test]
    fn read_line_defines_cs() {
        let mut vfs = MemVfs::new();
        vfs.insert("data.txt", "Hello\nWorld\n");
        let (out, _) = expand_vfs(
            "\\newread\\r\\openin\\r=data.txt\\read\\r to \\line\\line",
            vfs,
        )
        .unwrap();
        // tex.web read_toks（L9471）：`buffer[limit]:=end_line_char` 后再 token 化，
        // mid_line 状态遇行尾字符 → 「Finish line, emit a space」——\read 行**含
        // 尾随空格**。pdfTeX 实测（/tmp/rdchk rt.tex）：`\meaning\line` =
        // `macro:->Hello `；`\endlinechar=-1` 时 = `macro:->World`。
        assert_eq!(out, "Hello ");
    }

    #[test]
    fn read_eof_closes_stream_assigns_empty() {
        // 2026-09-15 第七刀更正：tex.web read_toks（L9510-9517）EOF 读不报错
        // （a_close + read_open:=closed + 赋空表），旧期望"报错"系本引擎私设。
        let mut vfs = MemVfs::new();
        vfs.insert("empty.txt", "");
        let (out, _) = expand_vfs(
            "\\newread\\r\\openin\\r=empty.txt\\read\\r to \\line\\ifeof\\r T\\else F\\fi\\end",
            vfs,
        )
        .unwrap();
        assert_eq!(out, "T");
    }

    #[test]
    fn read_at_eof_closes_stream_no_error() {
        // tex.web read_toks（L9510-9517）：input_ln 失败 → a_close +
        // read_open:=closed，不报错、赋空表，`\ifeof` 随之为真。
        // l3kernel `\__ior_map_variable_loop` 的 `\if_eof:w` 收束靠此语义。
        let mut vfs = MemVfs::new();
        vfs.insert("data.txt", "one\ntwo\n");
        let (out, _) = expand_vfs(
            concat!(
                "\\newread\\r\\openin\\r=data.txt",
                "\\read\\r to \\a\\read\\r to \\b",
                "\\ifeof\\r T\\else F\\fi",
                // 第三次读触发 EOF 臂：不报错、撤流条目（⇔ closed）、赋空表
                "\\read\\r to \\c\\ifeof\\r T\\else F\\fi",
                "\\def\\empty{}\\ifx\\c\\empty E\\else N\\fi",
                "\\end",
            ),
            vfs,
        )
        .unwrap();
        assert_eq!(out, "FTE");

        // 已 closed 的流再 \read：tex.web 转终端输入，nonstop/batch 禁止交互
        // → pdfTeX GT 致命（第十八刀，见 fatal_closed_read_stream）——作业
        // 终止而非静默吞。（读 1 取行、读 2 触发 EOF 臂撤流条目，读 3 才是
        // closed 流再读。）
        let mut vfs = MemVfs::new();
        vfs.insert("data.txt", "one\n");
        let err = expand_vfs(
            "\\newread\\r\\openin\\r=data.txt\\read\\r to \\a\\read\\r to \\b\\read\\r to \\c\\end",
            vfs,
        )
        .unwrap_err();
        assert!(
            err
                .to_string()
                .contains("cannot \\read from terminal in nonstop modes"),
            "closed 流再读应报 GT 文本，实得：{err}"
        );
    }

    #[test]
    fn ior_read_no_trailing_newline_last_line_counts() {
        // tex.web input_ln：无换行符尾行仍是一次成功读；read_open 要到下一次
        // input_ln 失败才 closed。因此读完尾行后的 \ifeof 仍为假，第四次读才 EOF。
        let mut vfs = MemVfs::new();
        vfs.insert("data.txt", "one\ntwo\nthree");
        let (out, _) = expand_vfs(
            concat!(
                "\\newread\\r\\openin\\r=data.txt\\endlinechar=-1",
                "\\read\\r to \\a\\ifeof\\r T\\else F\\fi\\a,",
                "\\read\\r to \\b\\ifeof\\r T\\else F\\fi\\b,",
                "\\read\\r to \\c\\ifeof\\r T\\else F\\fi\\c,",
                "\\read\\r to \\d\\ifeof\\r T\\else F\\fi",
                "\\def\\empty{}\\ifx\\d\\empty E\\else N\\fi\\end",
            ),
            vfs,
        )
        .unwrap();
        assert_eq!(out, "Fone,Ftwo,Fthree,TE");
    }

    #[test]
    fn input_stack_tailrecursion_macro_loop_35k_lines() {
        // tex.web macro_call: a macro whose active branch ends in a self call must
        // not keep one input frame per iteration. The expl3 UnicodeData loader uses
        // this shape via \__ior_map_inline_loop over 34931 lines.
        // This do-while probe counts the EOF-closing empty read too; keep 34930
        // content lines to lock the externally observed LINES:34931 criterion.
        let mut vfs = MemVfs::new();
        let mut data = String::new();
        for i in 0..34_930 {
            if i != 0 {
                data.push('\n');
            }
            data.push_str(&format!("{i:04X}; NAME"));
        }
        vfs.insert("UnicodeData.txt", data);
        let (out, _) = expand_vfs(
            concat!(
                "\\newread\\uin",
                "\\openin\\uin=UnicodeData.txt\\relax",
                "\\def\\rdloop{\\ifeof\\uin\\relax\\else",
                "\\begingroup\\endlinechar=-1 \\readline\\uin to \\uline\\endgroup",
                "\\advance\\count0 by 1 \\rdloop\\fi}",
                "\\rdloop LINES:\\the\\count0\\end",
            ),
            vfs,
        )
        .unwrap();
        assert_eq!(out, "LINES:34931");
    }

    #[test]
    fn read_from_unopened_stream_is_emergency_stop_not_invalid_input() {
        // 第十八刀回归锁：未开流 \read 逐字对齐 pdfTeX GT（TinyTeX 实测转录
        // 三行顺序），且不得回退成旧的引擎内部"流未打开"消息。
        let mut e = Expander::new();
        e.set_misc_int(crate::param::MISC_INTERACTION_MODE, 1);
        let err = e.run_source("\\read15 to \\x").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("cannot \\read from terminal in nonstop modes"),
            "应报 GT 文本，实得：{msg}"
        );
        assert!(!msg.contains("流未打开"), "旧消息不得回归：{msg}");
        let t = e.transcript();
        let emergency = t.find("! Emergency stop.");
        let read = t.find("<read 15>");
        let abort = t.find("*** (cannot \\read from terminal in nonstop modes)");
        let (Some(emergency), Some(read), Some(abort)) = (emergency, read, abort) else {
            panic!("GT 转录三行缺失：{t:?}");
        };
        assert!(emergency < read && read < abort, "GT 顺序错乱：{t:?}");
    }

    #[test]
    fn readline_unopened_stream_aligns_read_semantics() {
        // \readline 与 \read 同站（tex.web read_toks）：未开流 → 同一 GT
        // 致命臂，非 \readline 私设消息。
        let mut e = Expander::new();
        e.set_misc_int(crate::param::MISC_INTERACTION_MODE, 1);
        let err = e.run_source("\\readline15 to \\x").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("cannot \\read from terminal in nonstop modes"),
            "应报 GT 文本，实得：{msg}"
        );
        assert!(!msg.contains("流未打开"), "旧消息不得回归：{msg}");
        let t = e.transcript();
        assert!(t.contains("! Emergency stop."), "转录缺 Emergency stop：{t:?}");
        assert!(t.contains("<read 15>"), "转录缺 <read 15>：{t:?}");
        assert!(
            t.contains("*** (cannot \\read from terminal in nonstop modes)"),
            "转录缺 *** 段：{t:?}"
        );
    }

    #[test]
    fn readline_at_eof_assigns_empty_and_closes_stream_no_error() {
        // \readline EOF 臂与 \read 同语义（第十八刀）：不报错、赋空表、
        // `\ifeof` 为真（流撤条目 ⇔ closed）。旧实现此站私设 fatal，靠本测
        // 锁住不回退。
        let mut vfs = MemVfs::new();
        vfs.insert("empty.txt", "");
        let (out, _) = expand_vfs(
            concat!(
                "\\newread\\r\\openin\\r=empty.txt\\readline\\r to \\line",
                "\\ifeof\\r T\\else F\\fi",
                "\\def\\empty{}\\ifx\\line\\empty E\\else N\\fi\\end",
            ),
            vfs,
        )
        .unwrap();
        assert_eq!(out, "TE");
    }

    #[test]
    fn readline_reads_raw_line() {
        // \endlinechar=13（默认）：行尾附加 ^^M
        let mut vfs = MemVfs::new();
        vfs.insert("data.txt", "Hello World\nnext\n");
        let (out, _) = expand_vfs(
            "\\newread\\r\\openin\\r=data.txt\\readline\\r to \\line\\line",
            vfs,
        )
        .unwrap();
        assert_eq!(out, "Hello World\r");
        // \endlinechar=-1：不附加
        let mut vfs = MemVfs::new();
        vfs.insert("data.txt", "Hello\n");
        let (out, _) = expand_vfs(
            "\\newread\\r\\openin\\r=data.txt\\endlinechar=-1\\readline\\r to \\line\\line",
            vfs,
        )
        .unwrap();
        assert_eq!(out, "Hello");
    }

    #[test]
    fn write18_shell_escape_rejected() {
        let vfs = MemVfs::new();
        assert!(expand_vfs("\\write18{echo hi}\\end", vfs).is_err());
    }
