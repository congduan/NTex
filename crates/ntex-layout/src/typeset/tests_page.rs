use super::*;

    #[test]
    fn tracingoutput_transcribes_shipped_box_tree() {
        // \tracingoutput=1（trip.tex L103 语义，值可经宏：\tracingoutput\on）：
        // 每次 shipout 转录 "Completed box being shipped out [页号]" + showbox 树
        // （tex.web ship_out L12687-12691）。2026-09-03 实现——TRIP 语义 diff
        // 大块转录缺失的修复。
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        let _ = ts.typeset_dvi(r"\def\on{1}\tracingoutput\on\shipout\hbox{aa}\end");
        let t = ts.take_transcript();
        assert!(
            t.contains("Completed box being shipped out"),
            "tracingoutput 应转录 shipout 标题：{t:?}"
        );
        assert!(
            t.contains(r"\hbox"),
            "转录应含 showbox 同款盒树：{t:?}"
        );
        // tracingoutput=0（默认）不转录
        let mut ts2 = Typesetter::with_metrics(metrics).plain_params();
        let _ = ts2.typeset_dvi(r"\shipout\hbox{aa}\end");
        let t2 = ts2.take_transcript();
        assert!(
            !t2.contains("Completed box being shipped out"),
            "tracingoutput=0 不应转录：{t2:?}"
        );
    }

    #[test]
    fn output_routine_shipout_box255_equals_default() {
        let src = fill_words();
        let default = paginated(&src).unwrap();
        assert!(
            default.len() >= 2,
            "小 \\vsize 应分多页，实际 {}",
            default.len()
        );
        // \output={\shipout\box255} ≡ 默认 shipout
        let with = paginated(&format!(r"\output={{\shipout\box255}} {src}")).unwrap();
        assert_eq!(with, default, r"\output={{\shipout\box255}} 应与默认完全一致");
    }

    #[test]
    fn empty_output_uses_default_routine() {
        let src = format!(r"\output={{}} {}", fill_words());
        let pages = paginated(&src).unwrap();
        assert_eq!(pages, paginated(&fill_words()).unwrap(), "tex.web assign_toks: 空列表回退默认输出");
    }

    #[test]
    fn nonshipping_output_reports_box255_and_retries_end_until_dead_cycle_gate() {
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        let pages = ts
            .typeset_dvi(
                r"\vsize=100pt \maxdeadcycles=3 \output={\relax} \hbox{x}\vfil\penalty-10000 \end",
            )
            .unwrap()
            .0;
        let transcript = ts.take_transcript();
        assert_eq!(
            transcript.matches("Output routine didn't use all of \\box255.").count(),
            3,
            "{transcript}"
        );
        assert!(transcript.contains("Output loop---3 consecutive dead cycles."), "{transcript}");
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn pagegoal_reads_empty_and_frozen_page_state() {
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        ts.typeset_dvi(r"\vsize=100pt \showthe\pagegoal \hbox{x}\showthe\pagegoal\end")
            .unwrap();
        assert_eq!(
            showthe_values(&ts.take_transcript(), "pagegoal"),
            vec!["16383.99998pt", "100.0pt"],
        );

        let mut output = Typesetter::with_metrics(metrics).plain_params();
        output
            .typeset_dvi(
                r"\vsize=100pt \output={\showthe\pagegoal\shipout\box255} \hbox{x}\vfil\penalty-10000\end",
            )
            .unwrap();
        assert_eq!(showthe_values(&output.take_transcript(), "pagegoal"), vec!["100.0pt"]);
    }

    #[test]
    fn first_insert_correction_glue_reduces_pagegoal() {
        fn page_height(skip: &str) -> i64 {
            let mut ts = Typesetter::with_metrics(metrics).plain_params();
            ts.typeset_dvi(&format!(
                r"\vsize=100pt \skip200={skip} \count200=1000 \dimen200=1000pt \hbox{{x}}\insert200{{\hbox{{y}}}}\vfil\penalty-10000\end"
            ))
            .unwrap()
            .0[0]
                .height
        }
        // tex.web Create a page insertion node: correction glue is charged
        // once, before the insertion is fitted or split.
        assert_eq!(page_height("0pt") - page_height("10pt"), 10 * 65_536);
    }

    #[test]
    fn holdinginserts_keeps_insert_out_of_box_register() {
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        ts.typeset_dvi(
            r"\vsize=100pt \holdinginserts=1 \insert200{\hbox{x}}\hbox{y}\vfil\penalty-10000 \ifvoid200\message{BOX=VOID}\else\message{BOX=FILLED}\fi\end",
        )
        .unwrap();
        assert!(ts.take_transcript().contains("BOX=VOID"));
    }

    #[test]
    fn holdinginserts_preserves_full_insert_in_page_box() {
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        ts.typeset_dvi(
            r"\vsize=1pt \holdinginserts=1 \count200=1000 \dimen200=0pt \skip200=10pt
              \output={\showbox255\shipout\box255}
              \insert200{\hbox{x}\penalty999\hbox{y}}\hbox{z}\vfil\penalty-10000\end",
        )
        .unwrap();
        let transcript = ts.take_transcript();
        assert!(transcript.contains("..\\penalty 999"), "{transcript}");
        assert!(transcript.matches("..\\hbox").count() >= 2, "{transcript}");
    }

    #[test]
    fn adjusted_hbox_moves_vadjust_to_vertical_list() {
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        ts.typeset_dvi(r"\setbox0=\vbox{\hbox{\vadjust{\penalty999}}}\showbox0\end")
            .unwrap();
        let transcript = ts.take_transcript();
        assert!(transcript.contains(".\\penalty 999"), "{transcript}");
        assert!(!transcript.contains("..\\penalty 999"), "{transcript}");
    }

    #[test]
    fn output_routine_custom_header() {
        let src = fill_words();
        let default = paginated(&src).unwrap();
        // 例程给每页包一个页眉：\output={\shipout\vbox{\hbox{Header}\box255}}
        let with =
            paginated(&format!(r"\output={{\shipout\vbox{{\hbox{{Header}}\box255}}}} {src}"))
                .unwrap();
        assert_eq!(with.len(), default.len(), "例程不改变页数");
        for (p, d) in with.iter().zip(&default) {
            assert_eq!(p.children.len(), 2, "页面应包 Header + 原页：{p:?}");
            match &p.children[0] {
                Node::Box(h) => {
                    assert_eq!(h.children.len(), 6, "Header 为 6 字符 hbox");
                }
                other => panic!("首子节点应为 Header hbox，得到 {other:?}"),
            }
            assert_eq!(&p.children[1], &Node::Box(d.clone()), "box255 应为原页面");
        }
    }

    #[test]
    fn output_local_restores_after_group() {
        let src = fill_words();
        let default = paginated(&src).unwrap();
        // 组内定义 \output，组结束恢复未定义 → 行为与默认一致（直通 shipout）
        let with = paginated(&format!(r"{{\output={{\shipout\box255}}}} {src}")).unwrap();
        assert_eq!(with, default, "组结束应恢复未定义 \\output");
    }

    #[test]
    fn outputpenalty_six_tier_protocol() {
        // §2.2bis 罚分协议表逐行：-\@M/-\@Mi/-\@Mii/-\@Miii/-\@Miv/-\@MM
        // （宏展开等价：\newpage=\par\vfil\penalty-10000、
        //   \clearpage=...\penalty-10001、\supereject=\par\penalty-20000）。
        // 源码与 TinyTeX plain 对拍探针逐字符一致（/tmp/ors-work-d1/*.tex）：
        // 惩罚前须 \par 进垂直模式（否则与真 TeX 一样落在段内水平列表）。
        let cases: [(&str, &str); 6] = [
            (r"A\par\vfil\penalty-10000 B\par\end", "-10000"),  // \newpage
            (r"A\par\vbox{}\penalty-10001 B\par\end", "-10001"), // \clearpage
            (r"A\par\penalty-10002 B\par\end", "-10002"),        // 行内 float
            (r"A\par\penalty-10003 B\par\end", "-10003"),        // 垂直 float
            (r"A\par\penalty-10004 B\par\end", "-10004"),        // \end@float 强制页
            (r"A\par\penalty-20000 B\par\end", "-20000"),        // \supereject
        ];
        for (src, want) in cases {
            let got = showthe_values(&probe_transcript(src), "outputpenalty");
            assert!(
                got.iter().any(|v| v == want),
                "{src} 应报 {want}，实际 {got:?}"
            );
        }
    }

    #[test]
    fn outputpenalty_glue_break_is_inf_penalty() {
        // 页满在胶水处自然断页：断点非惩罚节点 → inf_penalty(10000)
        // （tex.web：`type(best_page_break)<>penalty_node → \outputpenalty:=10000`）。
        let mut ts =
            Typesetter::with_metrics(metrics).plain_params().with_space(|_| Glue::new(1000, 500, 300));
        let _ = ts.typeset_dvi(&format!(
            r"\vsize 100000sp\hsize 20000sp \output={{\showthe\outputpenalty\shipout\box255}} {}",
            fill_words()
        ));
        let got = showthe_values(&ts.take_transcript(), "outputpenalty");
        assert!(got.len() >= 2, "应产出多页：{got:?}");
        // GT（pdftex 同构文档实测 26 页 = [150, 10000×21, 150, 10000]）：胶水
        // 自然断页 = inf_penalty(10000)，但**末段的 widow-150 是合法点火**
        // （club/widow 实装后 GT 自己也有）——旧锁「第 2 页起全 10000」写于
        // club/widow 实装前（绿锁≠对，GT 证伪后按真值重钉）。
        // 断言三件：①首页 club=150（首行边界）；②中间页 ∈ {10000, 150}
        // （10000=胶水自然断 / 150=widow 段边界，打包粒度决定 widow 落点）；
        // ③末页=10000（its_all_over eject 页，断点=合成 penalty(0) 前的胶水）。
        assert_eq!(got[0], "150", "首页应为行间 clubpenalty 断点：{got:?}");
        for v in &got[1..got.len() - 1] {
            assert!(
                v == "10000" || v == "150",
                "自然断页 \\outputpenalty 应为 10000（widow 段可为 150）：{got:?}"
            );
        }
        assert_eq!(
            got.last(),
            Some(&"10000".to_string()),
            "末页应为 eject（inf_penalty）：{got:?}"
        );
    }

    #[test]
    fn outputpenalty_persists_until_next_fire_up() {
        // tex.web 无"例程结束后重置"：值保持到下一次 fire_up（\end 冲页用
        // eject 惩罚 -'10000000000 → 末页 -1073741824，与真 TeX 一致）。
        let got = showthe_values(&probe_transcript(r"A\par\vfil\penalty-10000 B\end"), "outputpenalty");
        assert_eq!(
            got,
            vec!["-10000", "-1073741824"],
            "例程后 \\outputpenalty 应保持，末页为 eject 惩罚：{got:?}"
        );
    }

    #[test]
    fn setbox255_outside_output_does_not_reinject_outputpenalty() {
        // TRIP L112 形态：输出例程外的 \setbox255 只是普通盒寄存器赋值。
        // 若把它当作待输出页面排队，会用上一轮 \outputpenalty=-10000
        // 立刻再点火，凭空 ship 一个空页。
        let mut ts =
            Typesetter::with_metrics(metrics).plain_params().with_space(|_| Glue::new(1000, 500, 300));
        let pages = ts
            .typeset_dvi(concat!(
                r"\vsize 40000000sp\hsize 20000000sp ",
                r"\output={\showthe\outputpenalty\shipout\box255} ",
                r"A\par\vfil\penalty-10000 ",
                r"\setbox255=\vbox{} ",
                r"B\end",
            ))
            .unwrap()
            .0;
        let vals = showthe_values(&ts.take_transcript(), "outputpenalty");
        assert_eq!(
            vals,
            vec!["-10000", "-1073741824"],
            "例程外 \\setbox255 不得用旧 \\outputpenalty 重新点火：{vals:?}"
        );
        assert_eq!(pages.len(), 2, "不应额外 ship 空页：{pages:?}");
    }

    #[test]
    fn deadcycles_counts_and_resets_per_page() {
        // tex.web fire_up incr(dead_cycles)、ship_out 清零：每页例程内读到的
        // 都是 1（连续死循环数只统计"例程没 ship"的轮次）。
        let mut ts =
            Typesetter::with_metrics(metrics).plain_params().with_space(|_| Glue::new(1000, 500, 300));
        let _ = ts.typeset_dvi(&format!(
            r"\vsize 100000sp\hsize 20000sp \output={{\showthe\deadcycles\shipout\box255}} {}",
            fill_words()
        ));
        let got = showthe_values(&ts.take_transcript(), "deadcycles");
        assert!(got.len() >= 2, "应产出多页：{got:?}");
        for v in &got {
            assert_eq!(v, "1", "每页例程内 \\deadcycles 应为 1（ship 后清零）：{got:?}");
        }
    }

    #[test]
    fn shipout_of_constructed_box_resets_deadcycles() {
        let mut ts =
            Typesetter::with_metrics(metrics).plain_params().with_space(|_| Glue::new(1000, 500, 300));
        let _ = ts.typeset_dvi(&format!(
            r"\vsize 100000sp\hsize 20000sp \maxdeadcycles=1 \output={{\showthe\deadcycles\shipout\vbox{{\box255}}}} {}",
            fill_words()
        ));
        let t = ts.take_transcript();
        let cycles = showthe_values(&t, "deadcycles");
        assert!(cycles.len() >= 2, "{t}");
        assert!(cycles.iter().all(|v| v == "1"), "{t}");
        assert!(!t.contains("Output loop---"), "{t}");
    }

    #[test]
    fn deadcycles_gate_blocks_routine_and_ships_default() {
        // tex.web fire_up：dead_cycles >= max_dead_cycles → "Output loop---N
        // consecutive dead cycles" 错 + 默认输出（直接 ship box255，例程不跑）。
        let mut ts =
            Typesetter::with_metrics(metrics).plain_params().with_space(|_| Glue::new(1000, 500, 300));
        let pages = ts
            .typeset_dvi(concat!(
                r"\vsize 100000sp\hsize 20000sp \maxdeadcycles=0 ",
                r"\output={\showthe\deadcycles\shipout\box255} ",
                r"A\par\vfil\penalty-10000 B\end",
            ))
            .map(|(p, _)| p)
            .expect("死循环保护应转默认输出而非失败");
        let t = ts.take_transcript();
        assert!(
            t.contains("Output loop---0 consecutive dead cycles."),
            "应报 Output loop 错误：{t:?}"
        );
        assert!(
            t.contains("increase \\maxdeadcycles"),
            "应带 tex.web help3 文本：{t:?}"
        );
        assert!(
            showthe_values(&t, "deadcycles").is_empty(),
            "用户例程不应再执行：{t:?}"
        );
        // 真 TeX 对拍（deadgate.tex，TinyTeX plain）：同源探针 "Output written
        // on deadgate.dvi (2 pages, 240 bytes)" —— 例程页 + \end 冲页。
        assert_eq!(pages.len(), 2, "页面应由默认输出例程照常 ship：{pages:?}");
    }

    #[test]
    fn deadcycles_is_assignable_internal_int() {
        // \deadcycles/\maxdeadcycles 走内部量语义（可 \the 可赋值；latex.ltx
        // L20608 \maxdeadcycles=100、\enddocument 的 \deadcycles\z@）。
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        let _ = ts.typeset_dvi(r"\deadcycles=7 \maxdeadcycles=100 \showthe\deadcycles\showthe\maxdeadcycles\end");
        let t = ts.take_transcript();
        assert_eq!(showthe_values(&t, "deadcycles"), vec!["7"]);
        assert_eq!(showthe_values(&t, "maxdeadcycles"), vec!["100"]);
    }

    #[test]
    fn deadcycles_assignment_is_global_but_maxdeadcycles_is_scoped() {
        // tex.web stores dead_cycles as page-builder state, so even an explicit
        // grouped assignment survives; max_dead_cycles is the normal scoped int.
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        let _ = ts.typeset_dvi(
            r"\deadcycles=1 \maxdeadcycles=1 {\deadcycles=2 \maxdeadcycles=2}\showthe\deadcycles\showthe\maxdeadcycles\end",
        );
        let t = ts.take_transcript();
        assert_eq!(showthe_values(&t, "deadcycles"), vec!["2"]);
        assert_eq!(showthe_values(&t, "maxdeadcycles"), vec!["1"]);
    }

    #[test]
    fn grouped_nonshipping_output_can_trip_deadcycles_gate() {
        let mut ts =
            Typesetter::with_metrics(metrics).plain_params().with_space(|_| Glue::new(1000, 500, 300));
        let pages = ts
            .typeset_dvi(concat!(
                r"\hsize 20000sp \vsize 100000sp \maxdeadcycles=3 ",
                r"A\par {\output{\relax}\deadcycles=2\penalty-10000} ",
                r"\output={\showthe\deadcycles\shipout\box255} ",
                r"B\par\vfil\penalty-10000\end",
            ))
            .map(|(p, _)| p)
            .expect("deadcycles gate should force ship instead of looping");
        let t = ts.take_transcript();
        assert!(
            t.contains("Output loop---3 consecutive dead cycles."),
            "grouped nonshipping routine should leave \\deadcycles at 3: {t:?}"
        );
        assert!(
            showthe_values(&t, "deadcycles").is_empty(),
            "forced default output must bypass the user routine: {t:?}"
        );
        assert!(!pages.is_empty(), "forced default output should ship a page");
    }

    #[test]
    fn unvbox_without_shipout_reaches_deadcycles_gate() {
        let mut ts =
            Typesetter::with_metrics(metrics).plain_params().with_space(|_| Glue::new(1000, 500, 300));
        let pages = ts
            .typeset_dvi(concat!(
                r"\hsize 20000sp \vsize 100000sp \maxdeadcycles=3 ",
                r"\output={\showthe\deadcycles\unvbox255\end} ",
                r"A\par\vfil\penalty-10000\end",
            ))
            .map(|(p, _)| p)
            .expect("deadcycles gate should force a default shipout");
        let t = ts.take_transcript();
        assert_eq!(
            showthe_values(&t, "deadcycles"),
            vec!["1", "2", "3"],
            "routine should run until the configured gate: {t:?}"
        );
        assert!(
            t.contains("Output loop---3 consecutive dead cycles."),
            "gate should report the TeX output-loop message: {t:?}"
        );
        assert_eq!(pages.len(), 1, "forced default output should ship exactly once");
    }

    #[test]
    fn resumed_contribution_reenters_output_before_outer_input() {
        // TRIP 末段控制流：`\unvbox255` 把页内容回投后，恢复点必须先让
        // 回投贡献重新触发输出例程；外层输入里的后续赋值不能抢先污染页标签。
        let mut ts =
            Typesetter::with_metrics(metrics).plain_params().with_space(|_| Glue::new(1000, 500, 300));
        let pages = ts
            .typeset_dvi(concat!(
                r"\hsize 20000sp \vsize 100000sp \maxdeadcycles=1 ",
                r"\output={\unvbox255} ",
                r"A\par\vfil\penalty-10000 ",
                r"\count0=7 \end",
            ))
            .map(|(p, _)| p)
            .expect("resumed contribution should be handled before final input");
        let counts = ts.shipped_page_counts();
        assert_eq!(pages.len(), 1, "deadcycles gate should ship the resumed page");
        assert_eq!(
            counts[0][0], 0,
            "resumed page must ship before the outer \\count0 assignment: {counts:?}"
        );
    }

    #[test]
    fn end_inside_output_routine_is_recoverable() {
        // TRIP 末段形态：输出例程中 `\unvbox255\end\rb`。`\end` 在输出例程的
        // internal vertical mode 里是可恢复错误，不能直接清栈结束作业；后续 `\rb`
        // 仍应被读取，最终由 deadcycles gate 强制 ship box255。
        let mut ts =
            Typesetter::with_metrics(metrics).plain_params().with_space(|_| Glue::new(1000, 500, 300));
        let pages = ts
            .typeset_dvi(concat!(
                r"\hsize 20000sp \vsize 100000sp \maxdeadcycles=2 ",
                r"\def\rb{}}",
                r"\output={\showthe\deadcycles\unvbox255\end\rb} ",
                r"A\par\vfil\penalty-10000\end",
            ))
            .map(|(p, _)| p)
            .expect("output routine recovery should not become an Engine error");
        let t = ts.take_transcript();
        assert!(
            t.contains("You can't use `end' in internal vertical mode."),
            "output-routine \\end should be reported as a recoverable mode error: {t:?}"
        );
        assert!(
            t.contains("Output loop---2 consecutive dead cycles."),
            "recovered routine should still reach the deadcycles gate: {t:?}"
        );
        assert_eq!(pages.len(), 1, "forced default output should ship the pending box255");
    }

    // ---------- 输出例程刀 5：页号链（\count0 页标签 + DVI bop 计数） ----------
    //
    // tex.web ship_out L12694-12699：`print_char("["); j:=9;
    // while (count(j)=0)and(j>0) do decr(j); for k:=0 to j do print_int(count(k))
    // ...`——count0 起逐段点分、**遇 0 截断**；全零 j 停在 0 → 恒打 `[0]`。
    // TRIP 参考 log L42/L602：`[0.0.0.0.11]`、`[-5000.0.0.0.11.53110374]`
    // （负值照打）。真 TeX 对拍探针见 /tmp/ors-work-k5/（TinyTeX plain）。

    #[test]
    fn page_label_truncates_at_first_zero_tail() {
        let c = |v: [i64; 10]| {
            let mut a = [0i64; 10];
            a[..v.len()].copy_from_slice(&v);
            format_page_label(&a)
        };
        assert_eq!(c([1, 0, 0, 0, 0, 0, 0, 0, 0, 0]), "[1]", "count1 起全零→只打 count0");
        assert_eq!(c([5, 7, 0, 0, 0, 0, 0, 0, 0, 0]), "[5.7]");
        assert_eq!(c([0, 0, 0, 0, 11, 0, 0, 0, 0, 0]), "[0.0.0.0.11]", "TRIP L42");
        assert_eq!(
            c([-5000, 0, 0, 0, 11, 53110374, 0, 0, 0, 0]),
            "[-5000.0.0.0.11.53110374]",
            "TRIP L602：负值照打，中段零保留"
        );
        assert_eq!(c([0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), "[0]", "全零特例：恒打 [0]");
        assert_eq!(c([2, 0, 4, 0, 0, 0, 0, 0, 0, 0]), "[2.0.4]", "尾零截断，非尾零保留");
    }

    #[test]
    fn count0_feeds_shipout_page_label() {
        // 单页：\count0=5 \count1=7 → 标题行 `... [5.7]`（与 TinyTeX plain 逐字符），
        // 且页级快照随页走（DVI bop 计数取值源）。
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        let _ = ts.typeset_dvi(r"\tracingoutput=1 \count0=5 \count1=7 \shipout\hbox{aa}\end");
        let t = ts.take_transcript();
        assert!(
            t.contains("Completed box being shipped out [5.7]"),
            "页标签应为 count0.count1 尾零截断：{t:?}"
        );
        let mut ts2 = Typesetter::with_metrics(metrics).plain_params();
        let (pages, _) =
            ts2.typeset_dvi(r"\count0=5 \count1=7 \shipout\hbox{aa}\end").unwrap();
        assert_eq!(pages.len(), 1);
        let mut want = [0i64; 10];
        want[0] = 5;
        want[1] = 7;
        assert_eq!(
            ts2.shipped_page_counts(),
            &[want][..],
            "shipout 边界的 \\count0..9 快照应随页携带"
        );
    }

    #[test]
    fn count0_advances_between_pages() {
        // 两页：例程 `\global\advance\count0 by 1` 复刻 plain `\advancepageno`
        // （\countdef\pageno=0；例程体在组内，须 \global——真 TeX 同探针
        // 非 global 时两页都是 [5.7]），第二页标签 [6.7]——真 TeX 对拍
        // [5.7]/[6.7]（/tmp/ors-work-k5/k5-p2.tex）。
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        let _ = ts.typeset_dvi(
            r"\vsize 40000000sp\hsize 20000000sp \count0=5 \count1=7 \tracingoutput=1
              \output={\shipout\box255 \global\advance\count0 by 1}
              aa\par\penalty-10000 bb\par\end",
        );
        let t = ts.take_transcript();
        assert!(
            t.contains("Completed box being shipped out [5.7]"),
            "第一页 [5.7]：{t:?}"
        );
        assert!(
            t.contains("Completed box being shipped out [6.7]"),
            "第二页 count0 已 +1 → [6.7]：{t:?}"
        );
        let counts: Vec<i64> = ts
            .shipped_page_counts()
            .iter()
            .flat_map(|c| c[..2].to_vec())
            .collect();
        assert_eq!(counts, vec![5, 7, 6, 7], "两页快照 = count0 递增、count1 不变");
    }

    #[test]
    fn page_label_defaults_to_zero() {
        // 未设 \count（INITEX 初表全零）→ `[0]`：tex.web 全零特例 j 停在 0。
        // 旧实现的 `[0.0.0.0.1]`（ship_seq 占位）即由此纠正。
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        let _ = ts.typeset_dvi(r"\tracingoutput=1 \shipout\hbox{aa}\end");
        let t = ts.take_transcript();
        assert!(
            t.contains("Completed box being shipped out [0]\n"),
            "全零 count 应打 [0]：{t:?}"
        );
    }

    #[test]
    fn count0_group_rollback_restores_mirror() {
        // 组内赋值组外回滚（tex.web：count 寄存器在 eqtb 内，组结束还原）：
        // 镜像须随引擎还原，否则 shipout 标签读到已回滚的值。
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        let _ = ts
            .typeset_dvi(r"\tracingoutput=1 \count0=1 {\count0=9} \shipout\hbox{aa}\end");
        let t = ts.take_transcript();
        assert!(
            t.contains("Completed box being shipped out [1]"),
            "组内 \\count0=9 回滚后应仍为 1：{t:?}"
        );
    }
