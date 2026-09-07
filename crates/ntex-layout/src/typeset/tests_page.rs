use super::*;

    #[test]
    fn tracingoutput_transcribes_shipped_box_tree() {
        // \tracingoutput=1（trip.tex L103 语义，值可经宏：\tracingoutput\on）：
        // 每次 shipout 转录 "Completed box being shipped out [页号]" + showbox 树
        // （tex.web ship_out L12687-12691）。2026-09-03 实现——TRIP 语义 diff
        // 大块转录缺失的修复。
        let mut ts = Typesetter::with_metrics(metrics);
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
        let mut ts2 = Typesetter::with_metrics(metrics);
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
    fn output_routine_empty_swallows_pages() {
        let src = format!(r"\output={{}} {}", fill_words());
        let pages = paginated(&src).unwrap();
        assert!(pages.is_empty(), "空 \\output 例程应吞掉所有页面");
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
            Typesetter::with_metrics(metrics).with_space(|_| Glue::new(1000, 500, 300));
        let _ = ts.typeset_dvi(&format!(
            r"\vsize 100000sp\hsize 20000sp \output={{\showthe\outputpenalty\shipout\box255}} {}",
            fill_words()
        ));
        let got = showthe_values(&ts.take_transcript(), "outputpenalty");
        assert!(got.len() >= 2, "应产出多页：{got:?}");
        // 首页断点读 0（待主线核：极小 \vsize 下首例程注入早于惩罚写定的疑似
        // 时序差，见 docs/output-routine-survey.md §5.2 勘误附录）；第 2 页起
        // 与真 TeX 一致：胶水自然断页 = inf_penalty(10000)。
        for v in &got[1..] {
            assert_eq!(v, "10000", "胶水自然断页 \\outputpenalty 应为 10000：{got:?}");
        }
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
    fn deadcycles_counts_and_resets_per_page() {
        // tex.web fire_up incr(dead_cycles)、ship_out 清零：每页例程内读到的
        // 都是 1（连续死循环数只统计"例程没 ship"的轮次）。
        let mut ts =
            Typesetter::with_metrics(metrics).with_space(|_| Glue::new(1000, 500, 300));
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
    fn deadcycles_gate_blocks_routine_and_ships_default() {
        // tex.web fire_up：dead_cycles >= max_dead_cycles → "Output loop---N
        // consecutive dead cycles" 错 + 默认输出（直接 ship box255，例程不跑）。
        let mut ts =
            Typesetter::with_metrics(metrics).with_space(|_| Glue::new(1000, 500, 300));
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
            t.contains("Output loop---0 consecutive dead cycles"),
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
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset_dvi(r"\deadcycles=7 \maxdeadcycles=100 \showthe\deadcycles\showthe\maxdeadcycles\end");
        let t = ts.take_transcript();
        assert_eq!(showthe_values(&t, "deadcycles"), vec!["7"]);
        assert_eq!(showthe_values(&t, "maxdeadcycles"), vec!["100"]);
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
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset_dvi(r"\tracingoutput=1 \count0=5 \count1=7 \shipout\hbox{aa}\end");
        let t = ts.take_transcript();
        assert!(
            t.contains("Completed box being shipped out [5.7]"),
            "页标签应为 count0.count1 尾零截断：{t:?}"
        );
        let mut ts2 = Typesetter::with_metrics(metrics);
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
        let mut ts = Typesetter::with_metrics(metrics);
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
        let mut ts = Typesetter::with_metrics(metrics);
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
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts
            .typeset_dvi(r"\tracingoutput=1 \count0=1 {\count0=9} \shipout\hbox{aa}\end");
        let t = ts.take_transcript();
        assert!(
            t.contains("Completed box being shipped out [1]"),
            "组内 \\count0=9 回滚后应仍为 1：{t:?}"
        );
    }

