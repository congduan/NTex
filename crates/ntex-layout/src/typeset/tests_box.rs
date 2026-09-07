use super::*;

    #[test]
    fn unclosed_box_group_is_rejected() {
        assert!(typeset(r"\hbox{a").is_err());
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
            Typesetter::with_metrics(metrics).with_space(|_| Glue::new(1000, 500, 300));
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
    fn box255_page_reads_dims_kind_and_vsplit() {
        // P1/P3 对照（简报探针第 1/2/4 条）：页在 → 非 void 的 vbox，\ht/\wd 可读；
        // `\vsplit255 to 10pt` 从页顶切出恰好 10pt（结果 ht+dp = to 值，
        // tex.web vpackage(exactly)），余量留在 255。latex.ltx `\@doclearpage`
        // 的 `\setbox\@tempboxa\vsplit\@cclv to\z@` 走的正是这条路。
        let mut ts =
            Typesetter::with_metrics(metrics).with_space(|_| Glue::new(1000, 500, 300));
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
            assert_eq!(
                &p.children,
                strip_leading_discardables(&d.children),
                "\\unvbox255 应原样回流页内容"
            );
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
