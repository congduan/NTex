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
    fn quoted_file_name_is_unquoted() {
        // web2c：带引号文件名剥引号（latex.ltx `\openin\@inputcheck"#1" `）
        let mut vfs = MemVfs::new();
        vfs.insert("q.tex", "Q");
        // 名字后的终止空格留在输入流（引擎既有的文件名扫描偏差）→ 输出 "Q "
        let (out, _) = expand_vfs("\\input\"q\" ", vfs).unwrap();
        assert_eq!(out, "Q ");
        let mut vfs = MemVfs::new();
        vfs.insert("data.txt", "Hello\n");
        // 名字终止空格留在输入流（既有文件名扫描偏差）→ 前导空格
        let (out, _) = expand_vfs(
            "\\newread\\r\\openin\\r=\"data.txt\" \\read\\r to \\l\\l",
            vfs,
        )
        .unwrap();
        assert_eq!(out, " Hello");
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
        assert_eq!(out, "Hello");
    }

    #[test]
    fn read_eof_errors() {
        let mut vfs = MemVfs::new();
        vfs.insert("empty.txt", "");
        assert!(
            expand_vfs("\\newread\\r\\openin\\r=empty.txt\\read\\r to \\line", vfs)
                .is_err()
        );
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
