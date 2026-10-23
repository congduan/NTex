use super::*;

    #[test]
    fn vtop_first_paragraph_has_no_leading_parskip() {
        let nodes = typeset(r"\parskip=0pt plus1pt \setbox0=\vtop{a}\copy0").unwrap();
        let b = as_box(nodes.last().unwrap());
        assert!(matches!(b.children.first(), Some(Node::Box(_))), "{b:?}");
        assert!(b.height > 0, "vtop takes its first line's height: {b:?}");
    }

    #[test]
    fn internal_vtop_with_alignment_cell_list_omits_first_parskip() {
        // Alignment cells can expose a single local list while retaining the
        // surrounding vtop group. This reproduces the p{} tabular cell state.
        let mut b = NodeBuilder::with_pagination(Fonts::Fn {
            metrics: |_, _| (0, 0, 0),
            space: |_| Glue::ZERO,
        }, true);
        b.box_state.pending_box = Some(PendingBox::VTop);
        b.group_begin(0).unwrap();
        b.list_modes.truncate(1);
        b.lists.truncate(1);
        b.par_begin(false).unwrap();
        assert_eq!(b.io_state.nodes_appended, 0, "first parskip must be omitted");
    }

    #[test]
    fn alignment_cell_restores_local_box_but_keeps_global_assignment() {
        fn chars(nodes: &[Node], out: &mut Vec<u32>) {
            for node in nodes {
                match node {
                    Node::Char { charcode, .. } => out.push(*charcode),
                    Node::Box(b) => chars(&b.children, out),
                    _ => {}
                }
            }
        }
        for (source, expected) in [
            (r"\setbox0=\hbox{A}\halign{#&#\cr \setbox0=\hbox{B}x&\copy0\cr}\end", "xA"),
            (r"\setbox0=\hbox{A}\halign{#&#\cr \global\setbox0=\hbox{G}x&\copy0\cr}\end", "xG"),
        ] {
            let nodes = typeset(source).unwrap();
            let mut actual = Vec::new();
            chars(&nodes, &mut actual);
            assert_eq!(actual, expected.bytes().map(u32::from).collect::<Vec<_>>(), "{source}");
        }
    }

    #[test]
    fn global_setbox_cancels_prior_group_restore() {
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        ts.typeset(r"\setbox0=\hbox{A}{\setbox0=\hbox{B}\global\setbox0=\hbox{C}}\ifvoid0\message{LOST}\else\message{KEPT}\fi\box0\end").unwrap();
        let t = ts.take_transcript();
        assert!(t.contains("KEPT"), "global box must survive group end: {t}");
        assert!(!t.contains("LOST"), "stale local save must not restore: {t}");
    }

    #[test]
    fn math_unbox_checks_void_before_mode_and_preserves_incompatible_box() {
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        ts.typeset_dvi(r"\setbox234=\hbox{A}$\unhbox233\unhbox234$\ifvoid234\message{LOST}\else\message{KEPT}\fi\end").unwrap();
        let t = ts.take_transcript();
        assert_eq!(t.matches("! Incompatible list can't be unboxed.").count(), 1, "{t}");
        assert!(t.contains("KEPT"), "incompatible unbox must leave register intact: {t}");
    }

    #[test]
    fn unclosed_box_group_is_rejected() {
        // 刀K：EOF 残组走 final_cleanup 救济（警告进转录 + 就地丢弃），
        // 作业不再拒收（TRIP/ETRIP 现场：\end 被上游错误恢复吞掉时残组只能
        // 靠救济收尾）。严格拒收仍保留在 incremental 路径与 pending_box 分支。
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        ts.typeset(r"\hbox{a").unwrap();
        let t = ts.take_transcript();
        assert!(
            t.contains("(end occurred inside a group at level 1)"),
            "残组应警告收尾：{t}"
        );
    }

    #[test]
    fn end_inside_unclosed_group_and_cond_completes() {
        // 刀K 回归锁 1：显式 \end 时未闭合组/条件 → final_cleanup 救济
        // （警告 + 正常收尾），不 fatal。去救济臂必红。
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        ts.typeset(r"\begingroup\iftrue A\end").unwrap();
        let t = ts.take_transcript();
        assert!(
            t.contains("(end occurred inside a group at level 1)"),
            "转录：{t}"
        );
        assert!(
            t.contains("(end occurred when iftrue on line 1 was incomplete)"),
            "转录：{t}"
        );
    }

    #[test]
    fn eof_inside_unclosed_group_and_cond_completes() {
        // 刀K 回归锁 2：\end 被吞（TRIP L363 runaway skip / ETRIP 现场）→
        // 纯 EOF 残组/残条件同样救济收尾，不 fatal。去救济臂必红。
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        ts.typeset(r"\begingroup\iftrue A").unwrap();
        let t = ts.take_transcript();
        assert!(
            t.contains("(end occurred inside a group at level 1)"),
            "转录：{t}"
        );
        assert!(
            t.contains("(end occurred when iftrue on line 1 was incomplete)"),
            "转录：{t}"
        );
    }

    #[test]
    fn box_register_void_errors() {
        // \box255 在无页面（void）时取用 → 报错
        assert!(
            paginated(r"\shipout\box255").is_err(),
            "void \\box255 应报错"
        );
    }

    #[test]
    fn box255_void_reads_void_and_vsplit_is_silent() {
        // P5 对照 + 简报探针第 3/4 条：页队列空时 255 与普通 void 寄存器同语义；
        // tex.web vsplit @<Dispense with trivial cases of void or bad boxes@>：
        // void → 结果 void、静默（改动前此处报 "\vsplit 盒子为空（void）" 硬错）。
        let mut ts =
            Typesetter::with_metrics(metrics).plain_params().with_space(|_| Glue::new(1000, 500, 300));
        let res = ts.typeset_dvi(concat!(
            r"\output={\shipout\box255} ",
            r"\setbox0=\vsplit255 to 655360sp ",
            r"\dimen0=\ht0\showthe\dimen0 ",
            r"\ifvoid0\count0=1\else\count0=0\fi\showthe\count0 ",
            r"\ifvoid255\count1=1\else\count1=0\fi\showthe\count1 ",
            r"aa bb cc\par dd ee ff\par\end",
        ));
        let t = ts.take_transcript();
        let pages = res.expect("void \\vsplit255 应静默而非硬错").0;
        assert_eq!(pages.len(), 1, "后续材料照常出页：{t:?}");
        assert_eq!(showthe_dimens(&t, "dimen"), vec![0.0], "\\ht0 应为 0：{t:?}");
        assert_eq!(
            showthe_values(&t, "count"),
            vec!["1", "1"],
            "box0 与 255 均应 void：{t:?}"
        );
    }

    #[test]
    fn box_register_dimensions_expand_inside_message() {
        // `\message` 的 fully-expanded text 会临时把输出 sink 换成 VecSink；
        // 只读内部量仍必须查询真实排版 sink。此前 `\the\wd0/\ht0/\dp0`
        // 在该路径恒读 VecSink 默认 0。
        let mut ts =
            Typesetter::with_metrics(metrics).plain_params().with_space(|_| Glue::new(1000, 500, 300));
        ts.typeset_dvi(
            r"\setbox0=\hbox{abc}\message{WD=[\the\wd0] HT=[\the\ht0] DP=[\the\dp0]}\end",
        )
        .ok();
        let t = ts.take_transcript();
        assert!(
            t.contains("WD=[0.05026pt] HT=[0.09155pt] DP=[0.02289pt]"),
            r"\message 内 \the\wd0/\ht0/\dp0 应读取真实盒寄存器维度：{t:?}"
        );
    }

    #[test]
    fn box255_page_reads_dims_kind_and_vsplit() {
        // P1/P3 对照（简报探针第 1/2/4 条）：页在 → 非 void 的 vbox，\ht/\wd 可读；
        // `\vsplit255 to 10pt` 从页顶切出恰好 10pt（结果 ht+dp = to 值，
        // tex.web vpackage(exactly)），余量留在 255。latex.ltx `\@doclearpage`
        // 的 `\setbox\@tempboxa\vsplit\@cclv to\z@` 走的正是这条路。
        let mut ts =
            Typesetter::with_metrics(metrics).plain_params().with_space(|_| Glue::new(1000, 500, 300));
        ts.typeset_dvi(concat!(
            r"\vsize 2000000sp\hsize 10000000sp ",
            r"\output={\dimen0=\ht255\showthe\dimen0\dimen1=\wd255\showthe\dimen1",
            r"\ifvoid255\count0=1\else\count0=0\fi\showthe\count0",
            r"\ifvbox255\count1=1\else\count1=0\fi\showthe\count1",
            r"\setbox0=\vsplit255 to 655360sp\dimen2=\ht0\dimen3=\dp0",
            r"\showthe\dimen2\showthe\dimen3",
            r"\ifvoid255\count2=1\else\count2=0\fi\showthe\count2",
            r"\shipout\box255\relax} ",
            r"aa bb cc\par dd ee ff\par\end",
        ))
        .unwrap();
        let t = ts.take_transcript();
        let dims = showthe_dimens(&t, "dimen");
        let counts = showthe_values(&t, "count");
        assert_eq!(counts.len(), 3, "应有三组判型探针：{t:?}");
        assert_eq!(
            &counts[..2],
            &["0", "1"],
            r"\ifvoid255 假（页在）、\ifvbox255 真（页面是 vbox）：{t:?}"
        );
        assert_eq!(counts[2], "0", "vsplit 后余量留在 255（非 void）：{t:?}");
        assert!(
            dims.len() >= 2 && dims[0] > 0.0 && dims[1] > 0.0,
            r"页在时 \ht255/\wd255 应 >0（真 TeX：ht=30.0pt、wd=100.0pt=\hsize）：{dims:?} {t:?}"
        );
        let (ht, dp) = (dims[2], dims[3]);
        assert!(
            (ht + dp - 10.0).abs() < 1e-4,
            r"\vsplit255 to 10pt 的结果应为恰好 10pt（真 TeX SPLIT-HT:10.0pt）：ht={ht} dp={dp}：{t:?}"
        );
    }

    #[test]
    fn vsplit_all_leaves_source_void() {
        // LaTeX mark 提取用 `\vsplit<box> to \maxdimen` 抽空临时页盒后，
        // 依赖原盒变 void 作为递归终止条件；空 vbox 不等价于 void。
        let mut ts =
            Typesetter::with_metrics(metrics).plain_params().with_space(|_| Glue::new(1000, 500, 300));
        ts.typeset_dvi(concat!(
            r"\vsize 2000000sp\hsize 10000000sp ",
            r"\output={\setbox0=\vsplit255 to 1073741823sp ",
            r"\ifvoid255\count0=1\else\count0=0\fi\showthe\count0",
            r"\shipout\box0\relax} ",
            r"aa bb cc\par\end",
        ))
        .unwrap();
        let t = ts.take_transcript();
        assert_eq!(
            showthe_values(&t, "count"),
            vec!["1"],
            r"\vsplit 抽空后源盒应为 void：{t:?}"
        );
    }

    #[test]
    fn box255_unvbox_preserves_page_children() {
        // P2 对照（latex.ltx `\@specialoutput` 的 `\global\setbox\@holdpg
        // \vbox{\unvbox\@cclv}` 同形）：页内容经 \unvbox255 原样回流用户 vbox
        // （真 TeX 盒树：`\vbox(22.0+1.94444)x100.0` 内子节点与直通
        // `\shipout\box255` 逐项同形；改动前恒报
        // "Incompatible list can't be unboxed."——255 不在寄存器文件）。
        let src = r"\vsize 2000000sp\hsize 10000000sp aa bb cc\par dd ee ff\par\end";
        let default = paginated(src).unwrap();
        let with = paginated(&format!(
            r"\output={{\setbox2=\vbox{{\unvbox255}}\shipout\box2\relax}} {src}"
        ))
        .unwrap();
        assert_eq!(with.len(), default.len(), "例程不改变页数");
        for (p, d) in with.iter().zip(&default) {
            // tex.web vpackage `list_ptr(r):=p` 整表保留（含段首 \parskip 前导
            // glue，真 GT ship 盒首子 `.\glue 16.0` 同理）——两侧原始 children 直比。
            assert_eq!(&p.children, &d.children, "\\unvbox255 应原样回流页内容");
        }
    }

    #[test]
    fn box255_setbox_stores_back_into_page_queue() {
        // P4 对照（latex.ltx L20914 `\setbox\@cclv\vbox{\box\@cclv\vfil}` 同形）：
        // 体内 \box255 弹出原页 → 追加 \vfil → 结果存回 255（write_box 替换队首）
        // → \shipout\box255 输出改写后页面。
        // 组级语义逐事件对齐 tex.web：take 是裸写（不入 save stack）、\setbox 记
        // eq_save（例程组结束时回滚到 null——页已 ship、队列空 → 回滚为无操作）。
        let src = r"\vsize 2000000sp\hsize 10000000sp aa bb cc\par dd ee ff\par\end";
        let default = paginated(src).unwrap();
        let with = paginated(&format!(
            r"\output={{\setbox255=\vbox{{\box255\vfil}}\shipout\box255\relax}} {src}"
        ))
        .unwrap();
        assert_eq!(with.len(), default.len(), "例程不改变页数");
        for (p, d) in with.iter().zip(&default) {
            // 改写后页面 = vbox{ 原页整体 , \vfil }（真 TeX NEW-HT:31.94444pt
            // = 30.0 + 1.94444 —— 原页作为整体盒嵌套，高度为其自然高）
            assert_eq!(p.children.len(), 2, "应为 [原页, vfil]：{p:?}");
            assert_eq!(&p.children[0], &Node::Box(d.clone()), "原页应整体嵌套");
            match p.children.last() {
                Some(Node::Glue {
                    stretch_order: GLUE_ORDER_FIL,
                    ..
                }) => {}
                other => panic!("末尾应为 \\vfil 胶水，得到 {other:?}"),
            }
        }
    }

    #[test]
    fn box255_copy_reads_page_queue() {
        // \copy255 走队列（trip.tex 第二例程 `\setbox255\copy255` 的通路）：
        // 复制当前页输出 —— 与默认直通逐页一致（改动前 \copy255 恒 void →
        // 空 hbox，页被吞）。
        let src = r"\vsize 2000000sp\hsize 10000000sp aa bb cc\par dd ee ff\par\end";
        let default = paginated(src).unwrap();
        let with = paginated(&format!(
            r"\output={{\setbox1=\copy255\shipout\box1\relax}} {src}"
        ))
        .unwrap();
        assert_eq!(with.len(), default.len(), "例程不改变页数");
        assert_eq!(with, default, "\\copy255 应复制当前页并原样输出");
    }
