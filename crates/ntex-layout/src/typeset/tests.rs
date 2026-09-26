#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::BoxKind;
    use ntex_core::SP_PER_PT;

    /// TRIP 冲刺调试（临时）：trip.tex 前段逐行二分。
    /// 无断言（只打 DBG 行）。`#[ignore]`：它把进程级 `NTEX_TFM_DIR` 改成
    /// fixtures/trip（无 cmr10），与并行跑的其他真实字体测试（增量排版段测试）
    /// 竞争——先跑到的测试读到被改的 env → 字体加载失败退化 nullfont → 文档
    /// 塌成 1 页（M5 阶段三全量回归实测复现）。调试需要时 `--ignored` 单独跑。
    #[test]
    #[ignore]
    fn dbg_trip_lines() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/trip");
        std::env::set_var("NTEX_TFM_DIR", dir);
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/trip/trip.tex");
        let full = std::fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = full.lines().collect();
        for end in [29, 30] {
            let src = lines[1..end].join("\n");
            let mut ts = Typesetter::with_tfm();
            match ts.typeset(&src) {
                Ok(_) => eprintln!("DBG L2..{end}: ok"),
                Err(e) => {
                    eprintln!("DBG L2..{end}: ERR {e}");
                    break;
                }
            }
        }
        // 隔离：L2..28 状态 + 单独 \toksdef / \def\on
        for (label, extra) in [
            ("toksdef", "\\toksdef\\tokens=256"),
            ("defon", "\\def\\on{1}"),
            ("on+toksdef", "\\def\\on{1} \\toksdef\\tokens=256"),
        ] {
            let src = format!("{}\n{extra}", lines[1..28].join("\n"));
            let mut ts = Typesetter::with_tfm();
            match ts.typeset(&src) {
                Ok(_) => eprintln!("DBG [{}] ok", label),
                Err(e) => eprintln!("DBG [{}] ERR {e}", label),
            }
        }
        // L32 分段隔离
        for (label, src) in [
            ("skip200-plUs", "\\skip200=10pt plUs5fil"),
            ("skip200-ifdim", "\\skip200=10pt plus5fil\\ifdim\\hsize<\\hsize\\fi lllminus 0 fill"),
            ("skip200-full", "\\skip200 = 10pt plUs5fil\\ifdim\\hsize<\\hsize\\fi lllminus 0 fill"),
        ] {
            let mut ts = Typesetter::with_tfm();
            match ts.typeset(src) {
                Ok(_) => eprintln!("DBG [L32-{label}] ok"),
                Err(e) => eprintln!("DBG [L32-{label}] ERR {e}"),
            }
        }
    }

    fn metrics(_font: FontId, ch: u32) -> (i64, i64, i64) {
        // 测试度量：宽 1000sp + 码点，高 6000，深 1500。
        (1000 + i64::from(ch), 6000, 1500)
    }

    fn typeset(text: &str) -> Result<Vec<Node>> {
        Typesetter::with_metrics(metrics).typeset(text)
    }

    fn as_box(n: &Node) -> &BoxNode {
        match n {
            Node::Box(b) => b,
            other => panic!("预期 Box，得到 {other:?}"),
        }
    }

    fn as_char(n: &Node) -> u32 {
        match n {
            Node::Char { charcode, .. } => *charcode,
            other => panic!("预期 Char，得到 {other:?}"),
        }
    }

    #[test]
    fn hbox_of_chars() {
        let main = typeset(r"\hbox{ab}").unwrap();
        assert_eq!(main.len(), 1);
        let b = as_box(&main[0]);
        assert_eq!(b.kind, BoxKind::HBox);
        assert_eq!(b.children.len(), 2);
        // 宽度 = 度量之和（a=1097, b=1098，1000 + 码点）
        assert_eq!(b.width, 1097 + 1098);
        assert_eq!(b.height, 6000);
        assert_eq!(b.depth, 1500);
    }

    #[test]
    fn paragraph_closure_by_par() {
        // 两段之间插入 interline glue：d = 12pt − (depth 1500 + height 6000)
        let main = typeset(r"ab\par cd").unwrap();
        assert_eq!(main.len(), 3);
        let p1 = as_box(&main[0]);
        // 行盒 = [a, b, \parfillskip]（M3-5 对齐 TeX：parfillskip 留在末行）
        assert_eq!(p1.children.len(), 3);
        assert_eq!(as_char(&p1.children[0]), b'a' as u32);
        match &main[1] {
            Node::Glue { width, .. } => {
                assert_eq!(*width, 12 * SP_PER_PT - (1500 + 6000));
            }
            other => panic!("预期 interline Glue，得到 {other:?}"),
        }
        let p2 = as_box(&main[2]);
        assert_eq!(as_char(&p2.children[0]), b'c' as u32);
    }

    #[test]
    fn paragraph_closed_at_eof() {
        let main = typeset("ab").unwrap();
        assert_eq!(main.len(), 1);
        assert_eq!(as_box(&main[0]).children.len(), 3); // a b + \parfillskip
    }

    #[test]
    fn par_after_vertical_inline_math_keeps_main_list() {
        // 回归（fuzz 命中）：垂直模式行内数学 + \par 此前在 close_math 后无条件
        // close_paragraph，把唯一主列表弹空 → append 时空栈 panic。
        let main = typeset("$x$\\par y").unwrap();
        // 垂直模式行内数学关闭后追加 baselineskip glue：公式盒 + 胶水 + 段落盒
        assert_eq!(main.len(), 3);
        assert!(matches!(main[0], Node::Box(_)));
        assert!(matches!(main[1], Node::Glue { .. }));
        let p = as_box(&main[2]);
        assert_eq!(as_char(&p.children[0]), b'y' as u32);
    }

    #[test]
    fn empty_input_gives_empty_main_list() {
        assert!(typeset("").unwrap().is_empty());
    }

    #[test]
    fn vbox_builds_vertical_list() {
        let main = typeset(r"\vbox{\hbox{a}\vskip 10pt\hbox{b}}").unwrap();
        assert_eq!(main.len(), 1);
        let v = as_box(&main[0]);
        assert_eq!(v.kind, BoxKind::VBox);
        // append_to_vlist（tex.web L13315-13327）的 prev_depth 只随盒子更新：
        // \vskip 之后下一个盒子仍补 baselineskip glue（广度 #27 多行 caption
        // 尾行距主根因；GT pdftex 实证 `.glue 10.0` 后紧跟
        // `.glue(\baselineskip) 5.05556`）。
        assert_eq!(v.children.len(), 4, "盒+\\vskip+行间glue+盒：{v:?}");
        assert!(matches!(v.children[0], Node::Box(_)));
        match &v.children[1] {
            Node::Glue { width, .. } => assert_eq!(*width, 10 * SP_PER_PT),
            other => panic!("预期 Glue，得到 {other:?}"),
        }
        assert!(matches!(v.children[2], Node::Glue { .. }), "\\vskip 后仍补行间 glue");
        assert!(matches!(v.children[3], Node::Box(_)));
    }

    #[test]
    fn nested_hbox() {
        let main = typeset(r"\hbox{a\hbox{b}c}").unwrap();
        let outer = as_box(&main[0]);
        assert_eq!(outer.children.len(), 3);
        assert_eq!(as_char(&outer.children[0]), b'a' as u32);
        assert!(matches!(outer.children[1], Node::Box(_)));
        assert_eq!(as_char(&outer.children[2]), b'c' as u32);
    }

    #[test]
    fn scoping_group_does_not_create_box() {
        let main = typeset(r"{\def\x{ab}\x}").unwrap();
        assert_eq!(main.len(), 1);
        assert_eq!(as_box(&main[0]).children.len(), 3); // a b + \parfillskip
    }

    #[test]
    fn kern_penalty_rule_in_hbox() {
        let main = typeset(r"\hbox{a\kern 10pt\penalty -50\hrule height 5pt depth 2pt width 100pt}")
            .unwrap();
        let b = as_box(&main[0]);
        assert_eq!(b.children.len(), 4);
        match &b.children[1] {
            Node::Kern { width } => assert_eq!(*width, 10 * SP_PER_PT),
            other => panic!("预期 Kern，得到 {other:?}"),
        }
        match &b.children[2] {
            Node::Penalty { penalty } => assert_eq!(*penalty, -50),
            other => panic!("预期 Penalty，得到 {other:?}"),
        }
        match &b.children[3] {
            Node::Rule { width, height, depth } => {
                assert_eq!(*width, 100 * SP_PER_PT);
                assert_eq!(*height, 5 * SP_PER_PT);
                assert_eq!(*depth, 2 * SP_PER_PT);
            }
            other => panic!("预期 Rule，得到 {other:?}"),
        }
    }

    #[test]
    fn vskip_appends_to_vertical_list() {
        let main = typeset(r"\vskip 10pt").unwrap();
        assert_eq!(main.len(), 1);
        match &main[0] {
            Node::Glue { width, .. } => assert_eq!(*width, 10 * SP_PER_PT),
            other => panic!("预期 Glue，得到 {other:?}"),
        }
    }

    /// tex.web main_control：垂直命令（\vskip）在（非受限）水平模式 →
    /// end_graf 隐式 \par 后重执行。demo 差异 #4：`{\boldfont 标题}\medskip正文`
    /// —— \medskip 的 vskip 未断标题段 → 标题与正文并轨同基线。
    #[test]
    fn vskip_in_paragraph_ends_it_implicitly() {
        let main = typeset(r"a\vskip 6pt b").unwrap();
        // prev_depth 只随盒子更新（append_to_vlist）：显式 \vskip 不重置行间
        // 关系，末段落盒仍补 baselineskip glue（GT 实证，广度 #27）。
        assert_eq!(main.len(), 4, "隐式 \\par 后应为 段落盒+glue+行间glue+段落盒");
        assert!(matches!(main[0], Node::Box(_)), "首项应为断行后的段落盒");
        match &main[1] {
            Node::Glue { width, .. } => assert_eq!(*width, 6 * SP_PER_PT),
            other => panic!("预期 Glue，得到 {other:?}"),
        }
        assert!(matches!(main[2], Node::Glue { .. }), "\\vskip 后仍补行间 glue");
        assert!(matches!(main[3], Node::Box(_)), "末项应为新起的段落盒");
    }

    /// tex.web L21160/L21162（head_for_vmode）：`\hrule` 只在垂直模式直接落
    /// vlist；主水平模式先隐式 end_graf。回归背景：`A\n\hrule B` 此前把规则
    /// 追加进后段行盒内部（resume-plain.tex 节标题横线现场，2026-09-12）。
    /// 无 width 说明的 hrule 保持 null_flag 哨兵（tex.web scan_rule_spec），
    /// 出货端解析为包含盒宽（L12598 `rule_wd:=width(this_box)`）。
    #[test]
    fn hrule_in_paragraph_ends_it_and_keeps_null_width() {
        let main = typeset(r"a\hrule height 3pt b").unwrap();
        // 结构：段落盒(a) + 规则 + 段落盒(b……含 parfillskip)
        let rule_idx = main
            .iter()
            .position(|n| matches!(n, Node::Rule { .. }))
            .expect("主垂直列表应有独立规则节点");
        assert!(
            main[..rule_idx].iter().all(|n| matches!(n, Node::Box(_))),
            "规则前只应有段落盒（隐式 \\par 生效），得到 {:?}",
            &main[..rule_idx]
        );
        match &main[rule_idx] {
            Node::Rule { width, height, .. } => {
                assert_eq!(*height, 3 * SP_PER_PT);
                assert_eq!(*width, ntex_core::NULL_FLAG, "无 width 说明保持 null 哨兵");
            }
            other => panic!("预期 Rule，得到 {other:?}"),
        }
    }

    /// 同上：\vfill 类无限阶垂直胶水在水平模式同样先隐式 \par（\vfil kind=3）。
    #[test]
    fn vfil_in_paragraph_ends_it_implicitly() {
        let main = typeset(r"a\vfil b").unwrap();
        // 同 \vskip：无限阶显式 glue 也不重置 prev_depth，末段落盒仍补行间 glue。
        assert_eq!(main.len(), 4, "\\vfil 后应为 段落盒+glue+行间glue+段落盒");
        assert!(matches!(main[0], Node::Box(_)));
        assert!(matches!(main[1], Node::Glue { .. }));
        assert!(matches!(main[2], Node::Glue { .. }));
        assert!(matches!(main[3], Node::Box(_)));
    }

    #[test]
    fn vtop_readjusts_height_depth() {
        // tex.web L21083-21087 Readjust：\vtop 的高度取首项高度，depth 相应调整；
        // shift_amount 保持 0（box_context 只由 \raise/\lower/\moveleft/\moveright 设置）
        let main = typeset(r"\vtop{\hbox{a}\hbox{b}}").unwrap();
        let b = as_box(&main[0]);
        assert_eq!(b.kind, BoxKind::VBox);
        assert_eq!(b.shift, 0, "\\vtop 不设置 shift");
        // 首项 hbox{a} 的高度成为 vtop 高度；depth = 原 depth + 原高度 - 首项高
        let first_h = match &b.children[0] {
            Node::Box(inner) => inner.height,
            other => panic!("首项应为 Box：{other:?}"),
        };
        assert_eq!(b.height, first_h);
        assert_eq!(
            b.depth,
            b.depth - b.height + first_h,
            "depth = 原 depth + 原高度 - 首项高（Readjust 后 height 已是 first_h）"
        );
    }

    #[test]
    fn par_in_hbox_is_rejected() {
        // \par 在 \hbox（restricted horizontal mode）：TeX 报错恢复（消息入转录）
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset(r"\hbox{a\par}");
        let t = ts.take_transcript();
        assert!(
            t.contains("You can't use \\par in restricted horizontal mode."),
            "转录应含 par 模式错误：{t}"
        );
    }

    #[test]
    fn box_spec_to_sets_width() {
        // M3-2-2 补齐后：\hbox to <dimen> 撑满到指定宽度（不再拒绝）
        let main = typeset(r"\hbox to 5pt{a}").unwrap();
        let b = as_box(&main[0]);
        assert_eq!(b.width, 5 * SP_PER_PT);
    }

    #[test]
    fn empty_hbox() {
        let main = typeset(r"\hbox{}").unwrap();
        let b = as_box(&main[0]);
        assert!(b.children.is_empty());
        assert_eq!(b.width, 0);
        assert_eq!(b.height, 0);
        assert_eq!(b.depth, 0);
    }

    #[test]
    fn expansion_inside_hbox() {
        // 宏展开、寄存器、\the 在盒子内容里正常工作。
        // GT（pdfTeX 内容流 `[(xy)]TJ … [(1)]TJ`，2026-09-14 主控实拍）：
        // `\count0=7` 的 `7` 完成赋值，`\the\count0` 在 \hbox 构造内取到的是
        // **新值 7 的字符**——但 pdfTeX 把 `\the` 产物排到盒子**外**的页文本流
        // （两个独立 TJ 段），盒内 chars 只有 `xy`。旧期望 `xy7` 系把段外 `1`
        // （`\count0` 展开为数字 `7`? 非——实际段外是 `1`）误并进盒。
        // NTex 与 pdfTeX 一致：盒内 = xy。
        let main = typeset(r"\hbox{\def\x{xy}\x\count0=7\the\count0}").unwrap();
        let b = as_box(&main[0]);
        let chars: Vec<u32> = b.children.iter().map(as_char).collect();
        assert_eq!(chars, vec![b'x' as u32, b'y' as u32]);
    }

    // ---------- M3-2-2 段落：缩进 / 行间胶水 ----------

    fn as_glue_width(n: &Node) -> i64 {
        match n {
            Node::Glue { width, .. } => *width,
            other => panic!("预期 Glue，得到 {other:?}"),
        }
    }

    #[test]
    fn indent_forces_paragraph_with_box() {
        let main = typeset(r"\parindent 20pt\indent a").unwrap();
        assert_eq!(main.len(), 1);
        let para = as_box(&main[0]);
        assert_eq!(para.children.len(), 3); // 缩进盒 + a + \parfillskip
        match &para.children[0] {
            Node::Box(b) => assert_eq!(b.width, 20 * SP_PER_PT),
            other => panic!("预期缩进空盒，得到 {other:?}"),
        }
        assert_eq!(as_char(&para.children[1]), b'a' as u32);
    }

    #[test]
    fn automatic_parindent_on_paragraph_start() {
        let main = typeset(r"\parindent 10pt ab").unwrap();
        let para = as_box(&main[0]);
        assert_eq!(para.children.len(), 4); // 缩进盒 + a + b + \parfillskip
        match &para.children[0] {
            Node::Box(b) => assert_eq!(b.width, 10 * SP_PER_PT),
            other => panic!("预期缩进空盒，得到 {other:?}"),
        }
    }

    #[test]
    fn noindent_suppresses_indent() {
        let main = typeset(r"\parindent 10pt\noindent ab").unwrap();
        let para = as_box(&main[0]);
        assert_eq!(para.children.len(), 3); // a + b + \parfillskip
        assert_eq!(as_char(&para.children[0]), b'a' as u32);
    }

    #[test]
    fn negative_parindent_becomes_kern() {
        let main = typeset(r"\parindent -5pt ab").unwrap();
        let para = as_box(&main[0]);
        match &para.children[0] {
            Node::Kern { width } => assert_eq!(*width, -5 * SP_PER_PT),
            other => panic!("预期 Kern，得到 {other:?}"),
        }
    }

    #[test]
    fn indent_inside_hbox() {
        let main = typeset(r"\parindent 20pt\hbox{\indent a}").unwrap();
        let b = as_box(&main[0]);
        assert_eq!(b.children.len(), 2);
        match &b.children[0] {
            Node::Box(ib) => assert_eq!(ib.width, 20 * SP_PER_PT),
            other => panic!("预期缩进空盒，得到 {other:?}"),
        }
    }

    // ---------- 折行 overfull 判定（tex.web §922-930） ----------

    /// overfull 判定 = 行自然宽超 \hsize 且**收缩不足**（超宽量 > 总可收缩量），
    /// 而非"自然宽 > \hsize"。glue 能收缩压回 \hsize 时（badness 有限）不算
    /// overfull —— 这是 tex.web `line_break` 的语义（demo1-fixed 74 行回归：
    /// `\parindent=0pt` + 断字 discretionary 段落在窄行宽下被误报）。
    #[test]
    fn overfull_only_when_shrink_insufficient() {
        // metrics: 每个字符宽 (1000+charcode)sp；space: 宽 10pt 伸缩 5pt 收缩 3pt。
        // 构造一行：自然宽略超 \hsize，但多个词间 glue 可收缩足够 → 不报 overfull。
        // 取 \hsize 让段落自然宽超过它约 2pt（每词 ~1 字符 + 空格 10pt）。
        // 用两个词 "ab cd"：char a=1061, b=1062, c=1063, d=1064（sp）；
        // 词间 glue 10pt=655360sp。自然宽 ≈ 4×~1062 + 655360 ≈ 4×1062+655360 ≈ 659608sp≈10.06pt。
        // 设 \hsize=8pt=524288sp：自然宽超 ~2pt，而 glue 收缩 3pt 足够 → 不报 overfull。
        let mut ts = Typesetter::with_metrics(metrics).with_space(space);
        ts.typeset(r"\hsize=8pt ab cd").unwrap();
        let t = ts.take_transcript();
        assert!(
            !t.contains("Overfull"),
            "可收缩时不应报 overfull，转录：{t}"
        );
    }

    // ---------- M9 中文刀 5：汉字字间断点（\cjkbreakmode） ----------

    /// `\cjkbreakmode` 决定汉字之间有没有断点。
    ///
    /// 同一段 8 个汉字、`\hsize` 只装得下 5 个（测试度量：宽 = 1000 + 码位，
    /// 「中」= 0x4E2D → 21013sp、「文」= 0x6587 → 26991sp；5 字 117021sp
    /// ≤ 2pt = 131072sp < 6 字 144012sp）：
    /// - **关**（默认，TeX 原语义）——汉字之间既无胶水也无 penalty，断不开，
    ///   整段挤成一行并报 Overfull；
    /// - **开**——字间胶水给出断点，折成两行且不报 Overfull。
    #[test]
    fn cjk_break_mode_controls_han_breakpoints() {
        let src = |mode: u8| {
            format!(
                "\\utfinputmode=1\\cjkbreakmode={mode}\\parindent=0pt\\hsize=2pt \
                 中文中文中文中文\\par"
            )
        };

        let mut ts = Typesetter::with_metrics(metrics);
        let off = ts.typeset(&src(0)).unwrap();
        let off_t = ts.take_transcript();
        assert_eq!(off.len(), 1, "关时汉字断不开，只应有一个行盒：{off:?}");
        assert!(
            off_t.contains("Overfull"),
            "关时装不下的中文行应报 Overfull：{off_t}"
        );

        let mut ts = Typesetter::with_metrics(metrics);
        let on = ts.typeset(&src(1)).unwrap();
        let on_t = ts.take_transcript();
        // 两行 = [行盒, \interlinepenalty, 基线胶水, 行盒]
        assert_eq!(on.len(), 4, "开时应折成两行：{on:?}");
        assert!(matches!(on[0], Node::Box(_)), "首节点应为行盒：{on:?}");
        assert!(matches!(on[3], Node::Box(_)), "末节点应为行盒：{on:?}");
        assert!(
            !on_t.contains("Overfull"),
            "开后字间可断，不应再报 Overfull：{on_t}"
        );
        // 断点落在汉字之间（装得下的第 5 字后）：首行 5 个字形、次行 3 个。
        // 断点处那处字间胶水被**丢弃**（Tex §845：断点自身胶水不入行宽），
        // 行内其余胶水被拉伸到 hsize——首行胶水宽 3513sp ≈ 0.0536pt，
        // 正是「自然宽 117021sp → hsize 131072sp」的余量摊到 4 处字间。
        let counts: Vec<usize> = on
            .iter()
            .filter_map(|n| match n {
                Node::Box(b) => Some(
                    b.children
                        .iter()
                        .filter(|c| matches!(c, Node::Char { .. } | Node::Ligature { .. }))
                        .count(),
                ),
                _ => None,
            })
            .collect();
        assert_eq!(counts, vec![5, 3], "断点应落在第 5 个汉字之后：{on:?}");
        let Node::Box(first) = &on[0] else {
            panic!("首节点应为行盒：{on:?}");
        };
        assert_eq!(first.width, 2 * SP_PER_PT, "行盒应被 hpack 到 \\hsize");
        let stretched: Vec<(i64, i64)> = first
            .children
            .iter()
            .filter_map(|c| match c {
                Node::Glue { width, stretch, .. } => Some((*width, *stretch)),
                _ => None,
            })
            .collect();
        assert_eq!(
            stretched,
            vec![
                (3513, 32768),
                (3513, 32768),
                (3512, 32768),
                (3513, 32768)
            ],
            "行内字间胶水应被拉伸（每处 stretch 0.5pt）：{on:?}"
        );
    }

    /// 宿主级默认（[`Typesetter::set_cjk_break_mode`]，ntex-dvi
    /// `--cjk-fallback`/`--utf8` 的落点）应与源内 `\cjkbreakmode=1` 同效——
    /// 默认值要**同时**落到 expander 与排版器 params 镜像：`close_paragraph`
    /// 读的是镜像，而 `set_misc_int` 不发 param_changed 事件（P0-4 修复：
    /// 只写 expander 时 CLI 开关无声失效）。源内显式 `\cjkbreakmode=0` 后写
    /// 覆盖，仍可关。
    #[test]
    fn host_cjk_break_default_reaches_layout_mirror() {
        let src = "\\utfinputmode=1\\parindent=0pt\\hsize=2pt 中文中文中文中文\\par";

        let mut ts = Typesetter::with_metrics(metrics);
        ts.set_cjk_break_mode(true);
        let on = ts.typeset(src).unwrap();
        let on_t = ts.take_transcript();
        assert!(
            !on_t.contains("Overfull"),
            "宿主默认开：字间可断，不应报 Overfull：{on_t}"
        );
        assert_eq!(on.len(), 4, "宿主默认开：应折成两行：{on:?}");

        // 同一开关下源内显式关：后写覆盖。
        let mut ts = Typesetter::with_metrics(metrics);
        ts.set_cjk_break_mode(true);
        let off = ts.typeset(&format!("\\cjkbreakmode=0{src}")).unwrap();
        let off_t = ts.take_transcript();
        assert_eq!(off.len(), 1, "源内显式关应盖掉宿主默认：{off:?}");
        assert!(
            off_t.contains("Overfull"),
            "源内显式关应回到整段单行 Overfull：{off_t}"
        );

        // 未开默认的其他实例不受影响（引擎默认仍是 0）。
        let mut ts = Typesetter::with_metrics(metrics);
        let plain = ts.typeset(src).unwrap();
        assert_eq!(plain.len(), 1, "未开默认：整段单行：{plain:?}");
        assert!(
            ts.take_transcript().contains("Overfull"),
            "未开默认：应报 Overfull"
        );
    }

    /// 收缩不足：自然宽超 \hsize 且超出量 > 可收缩量 → 报 overfull。
    #[test]
    fn overfull_reported_when_shrink_insufficient() {
        // 单字符 'a' 宽 (1000+97)=1097sp，无可收缩 glue；
        // \hsize 极小（如 0.01pt）→ 自然宽远超 \hsize 且无 glue 可收缩 → overfull。
        let mut ts = Typesetter::with_metrics(metrics).with_space(space);
        ts.typeset(r"\hsize=0.01pt a").unwrap();
        let t = ts.take_transcript();
        assert!(
            t.contains("Overfull"),
            "收缩不足时应报 overfull，转录：{t}"
        );
    }

    #[test]
    fn interline_uses_custom_baselineskip() {
        // \baselineskip 8pt：d = 8pt − (1500 + 6000)
        let main = typeset(r"\baselineskip 8pt ab\par cd").unwrap();
        assert_eq!(main.len(), 3);
        assert_eq!(as_glue_width(&main[1]), 8 * SP_PER_PT - (1500 + 6000));
    }

    #[test]
    fn interline_uses_lineskip_when_below_limit() {
        // d = 1pt − 7500 < 0 < \lineskiplimit 100pt → 用 \lineskip 3pt
        let src = r"\baselineskip 1pt\lineskip 3pt\lineskiplimit 100pt ab\par cd";
        let main = typeset(src).unwrap();
        assert_eq!(main.len(), 3);
        assert_eq!(as_glue_width(&main[1]), 3 * SP_PER_PT);
    }

    #[test]
    fn param_scoped_affects_indent() {
        // 组内 \parindent 30pt 只影响组内段落；组外恢复 20pt
        let src = r"\parindent 20pt{\parindent 30pt ab\par}cd";
        let main = typeset(src).unwrap();
        assert_eq!(main.len(), 3); // 段1 + interline + 段2
        let p1 = as_box(&main[0]);
        match &p1.children[0] {
            Node::Box(b) => assert_eq!(b.width, 30 * SP_PER_PT),
            other => panic!("预期 30pt 缩进，得到 {other:?}"),
        }
        let p2 = as_box(&main[2]);
        match &p2.children[0] {
            Node::Box(b) => assert_eq!(b.width, 20 * SP_PER_PT),
            other => panic!("预期 20pt 缩进，得到 {other:?}"),
        }
    }

    #[test]
    fn vbox_lines_get_interline_glue() {
        let main = typeset(r"\vbox{\hbox{a}\hbox{b}}").unwrap();
        let v = as_box(&main[0]);
        assert_eq!(v.children.len(), 3); // hbox + interline glue + hbox
        assert!(matches!(v.children[1], Node::Glue { .. }));
    }

    // ---------- M3-2-3 词间空白 ----------

    fn space(_font: FontId) -> Glue {
        Glue::new(10 * SP_PER_PT, 5 * SP_PER_PT, 3 * SP_PER_PT)
    }
    fn typeset_spaced(text: &str) -> Result<Vec<Node>> {
        Typesetter::with_metrics(metrics).with_space(space).typeset(text)
    }

    fn spaced_box_children(text: &str) -> Vec<Node> {
        let main = typeset_spaced(text).unwrap();
        as_box(&main[0]).children.clone()
    }

    #[test]
    fn lastskip_reads_only_tail_node() {
        // tex.web L8535 `glue_val: if type(tail)=glue_node then cur_val:=glue_ptr(tail)`
        // ——\lastskip 只认列表尾节点。latex.ltx \sw@slant 的 `\ifdim\lastskip=\z@`
        // 分派依赖它；此前向后回溯到任意 glue 节点，`\textbf` 前词间空格丢失。
        // 尾是 glue：-\lastskip 取到 5pt（回归旧"回溯"实现时这里是 0）
        let kids = spaced_box_children(r"\hbox{a\hskip 5pt\hskip-\lastskip}");
        match &kids[2] {
            Node::Glue { width, .. } => {
                assert_eq!(*width, -5 * SP_PER_PT, "lastskip 应取尾 glue 5pt");
            }
            other => panic!("预期 Glue，得到 {other:?}"),
        }
        // 尾是字符：0，不向列表前段回溯
        let kids = spaced_box_children(r"\hbox{a\hskip 5pt b\hskip\lastskip}");
        match &kids[3] {
            Node::Glue { width, .. } => {
                assert_eq!(*width, 0, "尾非 glue → 0");
            }
            other => panic!("预期 Glue，得到 {other:?}"),
        }
    }

    #[test]
    fn space_becomes_glue() {
        let children = spaced_box_children(r"\hbox{a b}");
        assert_eq!(children.len(), 3);
        match &children[1] {
            Node::Glue {
            name: None,                width,
                stretch,
                shrink,
                ..
            } => {
                assert_eq!(*width, 10 * SP_PER_PT);
                assert_eq!(*stretch, 5 * SP_PER_PT);
                assert_eq!(*shrink, 3 * SP_PER_PT);
            }
            other => panic!("预期词间 Glue，得到 {other:?}"),
        }
        assert_eq!(as_char(&children[2]), b'b' as u32);
    }

    #[test]
    fn consecutive_spaces_collapse() {
        let children = spaced_box_children(r"\hbox{a  b}"); // 两个空格
        assert_eq!(children.len(), 3); // a + glue + b
        assert!(matches!(children[1], Node::Glue { .. }));
    }

    #[test]
    fn space_after_glue_or_penalty_ignored() {
        // 空格吞并在扫描层（数值后单站可选空格 + 连续空格折叠），排版层不动。
        // GT（pdftex/tex `\showbox0`）：`a\hskip 5pt␣␣b` → `.\tenrm a .\glue 5.0
        // .\tenrm b`（3 节点）；`a\penalty -10␣␣b` → a .\penalty -10 .\tenrm b。
        // 而真空格 token 抵达排版层（`~`=\leavevmode\nobreak\ 、\@citex 的
        // `,\penalty\@m\ `）必须出胶水：GT `a~b` → a .\penalty 10000 .\glue 3.33。
        let children = spaced_box_children(r"\hbox{a\hskip 5pt  b}");
        assert_eq!(children.len(), 3); // a + glue(5pt) + b
        match &children[1] {
            Node::Glue { width, .. } => assert_eq!(*width, 5 * SP_PER_PT),
            other => panic!("预期 5pt Glue，得到 {other:?}"),
        }
        let children = spaced_box_children(r"\hbox{a\penalty -10  b}");
        assert_eq!(children.len(), 3); // a + penalty + b
        assert!(matches!(children[1], Node::Penalty { .. }));
    }

    #[test]
    fn space_at_hbox_start_appends_glue() {
        // GT（pdftex/tex `\showbox0`）：`\hbox{ a}` → `.\glue 3.33333 plus …` +
        // `.\tenrm a`——`{` 后扫描器 state=mid_line，空格是 spacer token，
        // tex.web `hmode+spacer` 无条件追加（旧实现按"空列表"吞并，连
        // `\nobreakspace`/`\@citea` 的 `\ ` 一起吞掉 → `~` 零宽、多 key 引用无逗号空格）。
        let children = spaced_box_children(r"\hbox{ a}");
        assert_eq!(children.len(), 2); // glue + a
        assert!(matches!(children[0], Node::Glue { .. }));
        assert_eq!(as_char(&children[1]), b'a' as u32);
    }

    #[test]
    fn tilde_and_citea_space_survive_penalty() {
        // 引用通路依赖：penalty 之后的空格 token 必须出词间胶水（GT `a~b` 4 节点）。
        let children = spaced_box_children(r"\hbox{a\penalty10000\ b}");
        assert_eq!(children.len(), 4); // a + penalty + glue + b
        assert!(matches!(children[1], Node::Penalty { .. }));
        assert!(matches!(children[2], Node::Glue { .. }));
        assert_eq!(as_char(&children[3]), b'b' as u32);
    }

    #[test]
    fn dimen_scan_swallows_trailing_space() {
        // \kern 7pt 后的空格被 dimen 扫描吞掉（TeX 规则），不产生词间胶水
        let children = spaced_box_children(r"\hbox{a\kern 7pt b}");
        assert_eq!(children.len(), 3); // a + kern + b
        assert!(matches!(children[1], Node::Kern { .. }));
    }

    // ---------- \␣（ex_space）：与 spacer 分路径的 tex.web 语义 ----------

    #[test]
    fn control_space_ignores_spacefactor() {
        // tex.web L20054 `hmode+ex_space: goto append_normal_space`——绕过
        // `app_space` 的 spacefactor 折算。GT（tex `\showbox0`）：
        // `\sfcode`A=2000` 后 `A\ B` → `.\glue 3.33333 plus 1.66666 minus 1.11111`
        // （字体胶水原样），而 `A B` → `plus 3.33333 minus 1.66666`（stretch ×2）。
        // 此前 `\ ` 发 cat10 空 token 走 spacer 路径，stretch/shrink 被错折成 1.66499/1.11222。
        let children = spaced_box_children(r"\hbox{\sfcode`a=2000 a\ b}");
        assert_eq!(children.len(), 3); // a + glue + b
        match &children[1] {
            Node::Glue {
                width,
                stretch,
                shrink,
                ..
            } => {
                assert_eq!(*width, 10 * SP_PER_PT, "\\ 宽度不受 spacefactor 影响");
                assert_eq!(*stretch, 5 * SP_PER_PT, "\\ stretch 原样");
                assert_eq!(*shrink, 3 * SP_PER_PT, "\\ shrink 原样");
            }
            other => panic!("预期词间 Glue，得到 {other:?}"),
        }
        // 对照组：真 spacer 走 app_space（sf≥2000 stretch ×= sf/1000、shrink ÷ sf/1000）
        let children = spaced_box_children(r"\hbox{\sfcode`a=2000 a b}");
        match &children[1] {
            Node::Glue {
                stretch,
                shrink,
                ..
            } => {
                assert_eq!(*stretch, 10 * SP_PER_PT, "spacer stretch ×= sf/1000");
                assert_eq!(*shrink, 3 * SP_PER_PT / 2, "spacer shrink ×= 1000/sf");
            }
            other => panic!("预期词间 Glue，得到 {other:?}"),
        }
    }

    #[test]
    fn control_space_uses_spaceskip_param() {
        // tex.web append_normal_space（L20332）：`\spaceskip` 非 zero_glue →
        // 参数胶水（GT：`.\glue(\spaceskip) 5.0`），`\xspaceskip` 不参与
        // （GT `\xspaceskip=9pt\sfcode`A=3000` 后 `A\ B` 仍是 `glue(\spaceskip) 5.0`）。
        let children = spaced_box_children(r"\hbox{\spaceskip=5pt a\ b}");
        assert_eq!(children.len(), 3);
        match &children[1] {
            Node::Glue {
                name,
                width,
                stretch,
                shrink,
                ..
            } => {
                assert_eq!(*name, Some("spaceskip"), "参数胶水带 showbox 来源名");
                assert_eq!(*width, 5 * SP_PER_PT);
                assert_eq!(*stretch, 0);
                assert_eq!(*shrink, 0);
            }
            other => panic!("预期 \\spaceskip Glue，得到 {other:?}"),
        }
        // 对照组：`\spaceskip=0` 落当前字体 font_glue（10/5/3pt）
        let children = spaced_box_children(r"\hbox{a\ b}");
        match &children[1] {
            Node::Glue { name, width, .. } => {
                assert_eq!(*name, None);
                assert_eq!(*width, 10 * SP_PER_PT);
            }
            other => panic!("预期词间 Glue，得到 {other:?}"),
        }
    }

    #[test]
    fn control_space_after_digit_and_adjacent_spaces() {
        // 数字扫描尾的可选空格站不得吞 `\ `（GT `\hbox{2\ 3}` → 2 .\glue 3.33 3）；
        // 显式空格与 `\ ` 相邻各出一个胶水（GT `\hbox{A \ B}` 5 节点）。
        let children = spaced_box_children(r"\hbox{2\ 3}");
        assert_eq!(children.len(), 3);
        assert!(matches!(children[1], Node::Glue { .. }));
        let children = spaced_box_children(r"\hbox{a \ b}");
        assert_eq!(children.len(), 4, "显式空格 + \\ 各出一胶水：{children:?}");
        assert!(matches!(children[1], Node::Glue { .. }));
        assert!(matches!(children[2], Node::Glue { .. }));
    }

    #[test]
    fn control_space_in_vertical_mode_starts_indented_paragraph() {
        // tex.web L21107 `vmode+ex_space → back_input; new_graf(true)`：起段
        // （带缩进）后 `\ ` 的胶水落段首。GT（`\parindent=10pt`，空行后 `\ x`）：
        // 行盒 = 缩进盒 10pt + `.\glue 3.33333 plus …` + `.\tenrm x`。
        // 此前 core 层 `\ ` 发空格 token、排版层垂直模式直接丢弃（无段落、无胶水）。
        let main = typeset_spaced(r"\parindent 65536sp \hsize 30000000sp \ x").unwrap();
        assert_eq!(main.len(), 1, "应起一段");
        let children = as_box(&main[0]).children.clone();
        assert_eq!(
            children.len(),
            4,
            "缩进盒 + \\ 胶水 + x + \\parfillskip：{children:?}"
        );
        assert!(matches!(children[0], Node::Box(_)), "new_graf(true) 落缩进盒");
        assert_eq!(
            as_glue_width(&children[1]),
            10 * SP_PER_PT,
            "\\ 胶水落段首"
        );
        assert_eq!(as_char(&children[2]), b'x' as u32);
    }

    #[test]
    fn control_space_in_math_mode_appends_glue() {
        // tex.web L20054 `mmode+ex_space: goto append_normal_space`：数学模式
        // `\ ` 出普通 pt 胶水（GT `\hbox{$A\ B$}` → `.\teni A .\glue 3.33333 …
        // .\teni B`）；spacer 在数学模式被忽略，`\ ` 不得跟随。
        let main = typeset_spaced(r"$a\ b$").unwrap();
        let children = as_box(&main[0]).children.clone();
        // a + \ 胶水 + b（行尾 \parfillskip 不计）
        let content: Vec<&Node> = children
            .iter()
            .filter(|n| !matches!(n, Node::Glue { stretch_order: 1, .. }))
            .collect();
        assert_eq!(content.len(), 5, "mathon + a + 胶水 + b + mathoff：{children:?}");
        assert_eq!(as_glue_width(content[2]), 10 * SP_PER_PT, "\\ 出普通 pt 胶水");
        assert_eq!(as_char(content[3]), b'b' as u32);
    }

    // ---------- M3-3 Knuth-Plass 段落折行 ----------

    #[test]
    fn paragraph_wraps_at_hsize() {
        // "ab cd" 总宽 5394sp、\hsize 4000sp：断点胶水不入行（tex.web try_break
        // 先于胶水累计调用），首行 "ab" 无内部胶水 → badness 10000（demerits 10⁸），
        // 单行（末行强制断点 d=0）更优 → 只折一行（与 pdfTeX 语义一致）
        let src = r"\hsize 4000sp ab cd";
        let main = Typesetter::with_metrics(metrics)
            .with_space(|_| Glue::new(1000, 500, 300))
            .typeset(src)
            .unwrap();
        let lines: Vec<&Node> = main.iter().filter(|n| matches!(n, Node::Box(_))).collect();
        assert_eq!(lines.len(), 1, "断点胶水不含入行时单行更优：{main:?}");
    }

    #[test]
    fn paragraph_single_line_when_fits() {
        let src = r"ab\par cd";
        let main = typeset(src).unwrap();
        // 默认 \hsize=6.5in 极大：两段各一行，段间 interline glue
        assert_eq!(main.len(), 3);
        assert!(matches!(main[0], Node::Box(_)));
        assert!(matches!(main[1], Node::Glue { .. }));
        assert!(matches!(main[2], Node::Box(_)));
    }

    // ---------- M3-4 TFM（cmr10） ----------

    /// 解析真实 cmr10 度量（无 TeX 安装则 None，测试跳过）。
    fn cmr10_metrics() -> Option<FontMetrics> {
        let path = ntex_font::find_tfm("cmr10")?;
        let bytes = std::fs::read(path).ok()?;
        ntex_font::parse_tfm(&bytes).ok()
    }

    fn tfm_chars(src: &str) -> (Vec<u32>, i64, i64, i64) {
        let mut ts = Typesetter::with_tfm();
        let main = ts.typeset(src).unwrap();
        assert_eq!(main.len(), 1);
        let b = as_box(&main[0]);
        // 行盒含 \parfillskip（M3-5 对齐 TeX）；字符宽度取字符节点之和
        let (chars, width) = chars_width(&b.children);
        (chars, width, b.height, b.depth)
    }

    #[test]
    fn tfm_char_metrics_from_cmr10() {
        let Some(fm) = cmr10_metrics() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let (chars, width, height, depth) = tfm_chars(r"\font\cmr=cmr10\cmr abc");
        assert_eq!(chars, vec![b'a' as u32, b'b' as u32, b'c' as u32]);
        let (wa, ha, da) = fm.char_metrics(b'a' as u32);
        let (wb, hb, db) = fm.char_metrics(b'b' as u32);
        let (wc, hc, dc) = fm.char_metrics(b'c' as u32);
        assert_eq!(width, wa + wb + wc);
        assert_eq!(height, ha.max(hb).max(hc));
        assert_eq!(depth, da.max(db).max(dc));
    }

    /// 行盒 children（\hbox{\cmr ...}；连字/字距/词间距均在此层）。
    fn tfm_line_children(text: &str) -> Vec<Node> {
        let mut ts = Typesetter::with_tfm();
        let main = ts.typeset(text).unwrap();
        assert_eq!(main.len(), 1);
        as_box(&main[0]).children.clone()
    }

    #[test]
    fn tfm_lig_kern_and_sfcode_applied() {
        if cmr10_metrics().is_none() {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        }
        // 连字：f + i → 单字符 12（fi）；v + e → 字距 -18205sp 插入在 'e' 前
        let children = tfm_line_children(r"\font\cmr=cmr10\cmr \hbox{fi ve}");
        match &children[0] {
            Node::Ligature {
                charcode, components, ..
            } => {
                assert_eq!(*charcode, 12, "f+i 应连字为字符 12（fi）");
                assert_eq!(components, b"fi", "连字组成应为 f+i");
            }
            Node::Char { charcode, .. } => assert_eq!(*charcode, 12, "f+i 应连字为字符 12（fi）"),
            other => panic!("预期 Char，得到 {other:?}"),
        }
        assert_eq!(as_char(&children[2]), b'v' as u32);
        assert_eq!(
            children[3],
            Node::Kern { width: -18_205 },
            "v→e 应插入 -18205sp 字距"
        );
        assert_eq!(as_char(&children[4]), b'e' as u32);
        // \sfcode：逗号（sf=1250）后空格 stretch = round(space_stretch × 1250/1000)；
        // cmr10 space_stretch = 109226 sp → 136533
        let children = tfm_line_children(r"\font\cmr=cmr10\cmr \hbox{a, b}");
        match &children[2] {
            Node::Glue { stretch, .. } => {
                assert_eq!(*stretch, 136_533, "逗号后空格 stretch 按 sfcode 放大");
            }
            other => panic!("预期词间 Glue，得到 {other:?}"),
        }
    }

    #[test]
    fn tfm_at_scales_metrics() {
        let Some(fm) = cmr10_metrics() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        // at 12pt：缩放因子 = 12pt / 10pt（= 1.2）
        let scaled = fm.scaled_by(12 * SP_PER_PT, fm.design_size_sp);
        let (chars, width, height, _) = tfm_chars(r"\font\cmr=cmr10 at 12pt\cmr a");
        assert_eq!(chars, vec![b'a' as u32]);
        let (w, h, _) = scaled.char_metrics(b'a' as u32);
        assert_eq!(width, w);
        assert_eq!(height, h);
    }

    #[test]
    fn tfm_scaled_1200_matches() {
        let Some(fm) = cmr10_metrics() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let scaled = fm.scaled_by(1200, 1000);
        let (_, width, _, _) = tfm_chars(r"\font\cmr=cmr10 scaled 1200\cmr a");
        let (w, _, _) = scaled.char_metrics(b'a' as u32);
        assert_eq!(width, w);
    }

    #[test]
    fn tfm_space_glue_from_font_params() {
        let Some(fm) = cmr10_metrics() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let g = fm.space_glue();
        let mut ts = Typesetter::with_tfm();
        let main = ts.typeset(r"\font\cmr=cmr10\hbox{\cmr a b}").unwrap();
        let b = as_box(&main[0]);
        assert_eq!(b.children.len(), 3);
        match &b.children[1] {
            Node::Glue {
            name: None,                width,
                stretch,
                shrink,
                ..
            } => {
                assert_eq!(*width, g.width, "词间距来自字体 space 参数");
                assert_eq!(*stretch, g.stretch);
                assert_eq!(*shrink, g.shrink);
            }
            other => panic!("预期词间 Glue，得到 {other:?}"),
        }
    }

    #[test]
    fn tfm_missing_font_errors() {
        // tex.web \\font 失败 = 报错恢复（绑定字体 0，作业继续），不向排版层传 Err。
        // 断言转录含 "not loadable"（错误面在转录，不在 Result）。
        let mut ts = Typesetter::with_tfm();
        let r = ts.typeset(r"\font\x=definitely_not_a_font");
        assert!(r.is_ok(), "\\font 加载失败应恢复而非致命：{:?}", r.err());
        let transcript = ts.take_transcript();
        assert!(
            transcript.contains("not loadable"),
            "转录应含 not loadable：{transcript}"
        );
    }

    #[test]
    fn tfm_fonts_persist_across_typeset_calls() {
        let Some(fm) = cmr10_metrics() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let mut ts = Typesetter::with_tfm();
        ts.typeset(r"\font\cmr=cmr10").unwrap();
        let (_, width, _, _) = tfm_chars_in(&mut ts, r"\cmr a");
        let (w, _, _) = fm.char_metrics(b'a' as u32);
        assert_eq!(width, w);
    }

    fn tfm_chars_in(ts: &mut Typesetter, src: &str) -> (Vec<u32>, i64, i64, i64) {
        let main = ts.typeset(src).unwrap();
        assert_eq!(main.len(), 1);
        let b = as_box(&main[0]);
        // 行盒含 \parfillskip 胶水（M3-5 对齐 TeX）；字符宽度取字符节点之和
        let (chars, width) = chars_width(&b.children);
        (chars, width, b.height, b.depth)
    }

    /// 行盒内的字符序列与自然宽度（忽略 \parfillskip 等胶水）。
    fn chars_width(children: &[Node]) -> (Vec<u32>, i64) {
        let mut w = 0i64;
        let mut chars = Vec::new();
        for c in children {
            if let Node::Char {
                charcode,
                width,
                ..
            } = c
            {
                chars.push(*charcode);
                w += width;
            }
        }
        (chars, w)
    }

    // ---------- M3-5-3 \output 例程 + box255 ----------

    /// 分页排版（fn 指针度量 + 词间距）；返回 \shipout 页面。
    fn paginated(src: &str) -> Result<Vec<BoxNode>> {
        let mut ts = Typesetter::with_metrics(metrics).with_space(|_| Glue::new(1000, 500, 300));
        ts.typeset_dvi(src).map(|(pages, _)| pages)
    }

    /// 小 \vsize + 窄 \hsize 的填充文本（必然分多页）。
    fn fill_words() -> String {
        let words = [
            "aa", "bb", "cc", "dd", "ee", "ff", "gg", "hh", "ii", "jj", "kk", "ll", "mm", "nn",
            "oo", "pp", "qq", "rr", "ss", "tt", "uu", "vv", "ww", "xx", "yy", "zz",
        ];
        format!(
            r"\vsize 100000sp\hsize 20000sp {}",
            words.join(" ")
        )
    }

    #[test]
    fn clearpage_tail_does_not_ship_blank_page() {
        // LaTeX \clearpage 关键尾巴：\newpage 后空页上出现 \vbox{}\penalty-10001。
        // 空页上的零尺寸空盒不应建立 page_contents，否则会多 ship 一张空白页。
        let pages = paginated(r"X\par\vfil\penalty-10000 \vbox{}\penalty-10001 Y\end").unwrap();
        assert_eq!(pages.len(), 2, "X\\clearpage Y 形态应只产出 X/Y 两页：{pages:?}");

        let mut text = String::new();
        for p in &pages {
            collect_text(p, &mut text);
        }
        assert!(
            text.contains('X') && text.contains('Y'),
            "页面文本应保留 X 与 Y：{text:?}"
        );
    }

    #[test]
    fn zero_empty_box_on_empty_page_is_discarded() {
        let pages = paginated(r"\vbox{}\penalty-10001\end").unwrap();
        assert!(
            pages.is_empty(),
            "空页上的零尺寸空盒 + 强制惩罚不应凭空产页：{pages:?}"
        );
    }

    // ---------- M3 收尾（RFC-3）：VFS 集成 ----------

    fn ts_with_vfs() -> (Typesetter, ntex_io::MemVfs) {
        let mut ts = Typesetter::with_tfm();
        let vfs = ntex_io::MemVfs::new();
        ts.set_vfs(Box::new(vfs));
        let mut vfs = ts.take_vfs();
        let vfs = vfs
            .as_any_mut()
            .downcast_mut::<ntex_io::MemVfs>()
            .expect("MemVfs");
        (ts, vfs.clone())
    }

    #[test]
    fn vfs_input_splits_document() {
        // with_metrics（假字体）而非 with_tfm：测试环境无 TFM 目录，
        // 真实字体加载静默失败会导致字符无节点、页面为空
        let mut ts = Typesetter::with_metrics(metrics).with_space(|_| Glue::new(1000, 500, 300));
        let mut vfs = ntex_io::MemVfs::new();
        vfs.insert("ch1.tex", "Chapter One. ");
        vfs.insert("ch2.tex", "Chapter Two. ");
        ts.set_vfs(Box::new(vfs));
        let (pages, _) = ts.typeset_dvi("\\input{ch1}\\input{ch2}\\end").unwrap();
        assert!(!pages.is_empty(), "\\input 分章文档应产出页面");
        // 页面文本应包含两章内容（合并后页数 ≥ 1，且文本含 Chapter）
        let mut text = String::new();
        for p in &pages {
            collect_text(p, &mut text);
        }
        assert!(text.contains("Chapter"), "页面应含输入文本：{text}");
    }

    #[test]
    fn vfs_write_flushed_on_shipout_boundary() {
        let (mut ts, vfs) = ts_with_vfs();
        ts.set_vfs(Box::new(vfs));
        // \write 延迟 → \shipout 边界 flush（first）→ 再 \write → \end flush（second）
        ts.typeset_dvi(
            "\\newwrite\\aux\\openout\\aux=o.aux\\write\\aux{first}\
             \\shipout\\hbox{A}\\write\\aux{second}\\end",
        )
        .unwrap();
        let mut vfs = ts.take_vfs();
        let vfs = vfs
            .as_any_mut()
            .downcast_mut::<ntex_io::MemVfs>()
            .expect("MemVfs");
        assert_eq!(vfs.get("o.aux"), Some(b"first\nsecond\n".as_slice()));
    }

    #[test]
    fn vfs_read_then_write_roundtrip() {
        let (mut ts, mut vfs) = ts_with_vfs();
        vfs.insert("data.txt", "42\n");
        ts.set_vfs(Box::new(vfs));
        // \read 一行 → \line，\write 回显（延迟，\end flush）
        ts.typeset_dvi(
            "\\newread\\r\\openin\\r=data.txt\\read\\r to \\line\
             \\newwrite\\w\\openout\\w=o.txt\\write\\w{\\line}\\end",
        )
        .unwrap();
        let mut vfs = ts.take_vfs();
        let vfs = vfs
            .as_any_mut()
            .downcast_mut::<ntex_io::MemVfs>()
            .expect("MemVfs");
        // tex.web read_toks（L9471）`\buffer[limit]:=end_line_char` 后 token 化，
        // mid_line+car_ret → 「Finish line, emit a space」：\read 行含尾随空格。
        // pdfTeX 实测（\write\w{\line} 回显）写出 `Hello \n`（带尾空格）。
        assert_eq!(vfs.get("o.txt"), Some(b"42 \n".as_slice()));
    }

    /// 收集盒子树全部文本（集成断言用）。
    fn collect_text(b: &BoxNode, out: &mut String) {
        for c in &b.children {
            match c {
                Node::Char { charcode, .. } => {
                    out.push(char::from_u32(*charcode).unwrap_or('\u{FFFD}'))
                }
                Node::Box(inner) => collect_text(inner, out),
                _ => {}
            }
        }
    }

    #[test]
    fn fmt_snapshot_preserves_typeset_behavior() {
        // preamble（宏集）→ 快照 → 新排版器加载 → 同一文档排版一致
        let preamble = r"\def\emph#1{[#1]}\def\hi{Hi}";
        let doc = r"\emph{Hello} \hi there.";

        let mut ts1 = Typesetter::with_tfm();
        ts1.typeset_dvi(preamble).unwrap();
        let state = ts1.export_state();

        let mut ts2 = Typesetter::with_tfm();
        ts2.import_state(state);
        let (pages2, _) = ts2.typeset_dvi(doc).unwrap();

        let mut ts3 = Typesetter::with_tfm();
        let (pages3, _) = ts3.typeset_dvi(&format!("{preamble} {doc}")).unwrap();

        let text_of = |pages: &[BoxNode]| {
            let mut s = String::new();
            for p in pages {
                collect_text(p, &mut s);
            }
            s
        };
        assert_eq!(
            text_of(&pages2),
            text_of(&pages3),
            "加载 .fmt 快照后排版应与全新排版一致"
        );
    }

    #[test]
    fn fmt_snapshot_reenables_output_routine_in_layout() {
        // \output 已保存在 expander 的 .fmt 状态；导入后新建 NodeBuilder 也必须
        // 知道输出例程已定义，否则页面会绕过 box255，LaTeX 页眉/页脚包装丢失。
        let preamble = r"\font\cmr=cmr10 \cmr \output={\shipout\vbox{\hbox{H}\box255}}";
        let doc = r"Body.";

        let mut ts1 = Typesetter::with_tfm();
        ts1.typeset_dvi(preamble).unwrap();
        let state = ts1.export_state();

        let mut ts2 = Typesetter::with_tfm();
        ts2.import_state(state);
        let (pages, _) = ts2.typeset_dvi(doc).unwrap();

        let mut text = String::new();
        for p in &pages {
            collect_text(p, &mut text);
        }
        assert!(
            text.starts_with('H'),
            "fmt 导入后 \\output 应包住页面，实际文本流：{text:?}"
        );
    }

    // ---------- M4-1 数学模式 ----------

    /// `\vcenter` 按数学轴重分 height/depth（tex.web make_vcenter L14455）。
    /// pdfTeX 实测对照（etex/plain，DVI rule y 差分法）：fam2=cmsy10 轴高
    /// （fontdimen 22）=2.5pt，
    /// `\vcenter{\hbox{\vrule height 20pt depth 4pt width 2pt}}` → 14.5+9.5；
    /// `\vbox` 对照 20+4 不动；`\vcenter{\hbox{$x$}}` 内部 x 上移 0.347pt。
    #[test]
    fn vcenter_splits_height_depth_on_math_axis() {
        // 真 TFM + fam2=cmex10：轴高来自 fontdimen 22
        let mut ts = Typesetter::with_tfm();
        let main = ts
            .typeset(concat!(
                // plain.tex：\textfont2=\tensy（cmsy10，fontdimen 22=2.5pt 轴高；
                // cmex10 仅 13 参数，轴高不在其上）
                r"\font\tensy=cmsy10 \textfont2=\tensy ",
                r"$\vcenter{\hbox{\vrule height 20pt depth 4pt width 2pt}}$",
            ))
            .unwrap();
        let t = ts.take_transcript();
        assert!(!t.contains('!'), "载入期报错：{t}");
        let children: Vec<&Node> = as_box(&main[0])
            .children
            .iter()
            .filter(|n| {
                !matches!(
                    n,
                    Node::MathOn { .. } | Node::MathOff { .. } | Node::Glue { .. }
                )
            })
            .collect();
        assert_eq!(children.len(), 1, "vcenter 盒应单独入行：{children:?}");
        let v = as_box(children[0]);
        assert_eq!(v.kind, BoxKind::VBox);
        // height = axis(2.5pt) + half(24pt) = 14.5pt；depth = 24 − 14.5 = 9.5pt
        assert_eq!(v.height, (145 * SP_PER_PT) / 10, "vcenter 应按轴上移：{v:?}");
        assert_eq!(v.depth, (95 * SP_PER_PT) / 10);

        // 对照：\vbox 不做轴重分
        let main = Typesetter::with_tfm()
            .typeset(r"$\vbox{\hbox{\vrule height 20pt depth 4pt width 2pt}}$")
            .unwrap();
        let children: Vec<&Node> = as_box(&main[0])
            .children
            .iter()
            .filter(|n| {
                !matches!(
                    n,
                    Node::MathOn { .. } | Node::MathOff { .. } | Node::Glue { .. }
                )
            })
            .collect();
        let v = as_box(children[0]);
        assert_eq!((v.height, v.depth), (20 * SP_PER_PT, 4 * SP_PER_PT));
    }

    /// 无 fam2 字体时轴高回退 0（tex.web `mathsy(22)` 字体未加载）：
    /// `\vcenter` 退化为按盒中心分割（ht=dp=half(delta)）。
    #[test]
    fn vcenter_axis_fallback_splits_at_center() {
        let children =
            math_line_children(r"$\vcenter{\hbox{\vrule height 20pt depth 4pt width 2pt}}$");
        let v = as_box(&children[0]);
        assert_eq!(v.kind, BoxKind::VBox);
        assert_eq!(
            (v.height, v.depth),
            (12 * SP_PER_PT, 12 * SP_PER_PT),
            "轴高回退 0 → 中心分割：{v:?}"
        );
    }

    // ---------- D1 数学斜体修正 kern（tex.web §759-762） ----------

    /// 解析真实 cmmi10 度量（无 TeX 安装则 None，测试跳过）。
    fn cmmi10_metrics() -> Option<FontMetrics> {
        let path = ntex_font::find_tfm("cmmi10")?;
        let bytes = std::fs::read(path).ok()?;
        ntex_font::parse_tfm(&bytes).ok()
    }

    /// 数学行盒 children（TFM 实字体；剥 \mathon/\mathoff 边界标记与行尾
    /// parfillskip）。
    fn math_tfm_children(text: &str) -> Vec<Node> {
        let mut ts = Typesetter::with_tfm();
        let main = ts.typeset(text).unwrap();
        assert_eq!(main.len(), 1);
        let mut children: Vec<Node> = as_box(&main[0])
            .children
            .iter()
            .filter(|n| !matches!(n, Node::MathOn { .. } | Node::MathOff { .. }))
            .cloned()
            .collect();
        if matches!(
            children.last(),
            Some(Node::Glue {
                stretch_order: GLUE_ORDER_FIL,
                ..
            })
        ) {
            children.pop();
        }
        children
    }

    /// 段落行盒 children（`$...$` 触发段落 → 主列表单行 hbox）。
    fn math_line_children(text: &str) -> Vec<Node> {
        let main = typeset(text).unwrap();
        assert_eq!(main.len(), 1, "数学公式应封装为单行段落：{text:?}");
        let line = as_box(&main[0]);
        // 去掉行尾 \\parfillskip 胶水与 \\mathon/\\mathoff 边界标记
        // （边界节点是 \\tracinggroups 风格的结构标记，单元测试断言数学内容）
        line.children
            .iter()
            .filter(|n| {
                !matches!(
                    n,
                    Node::Glue { .. } | Node::MathOn { .. } | Node::MathOff { .. }
                )
            })
            .cloned()
            .collect()
    }

    /// 段落行盒全部 children（含 spacing 胶水，排除行尾 \parfillskip；spacing 测试用）。
    fn math_line_all_children(text: &str) -> Vec<Node> {
        let main = typeset(text).unwrap();
        assert_eq!(main.len(), 1, "数学公式应封装为单行段落：{text:?}");
        let mut children: Vec<Node> = as_box(&main[0])
            .children
            .iter()
            // 去掉 \\mathon/\\mathoff 边界标记（结构标记，测试断言数学内容）
            .filter(|n| !matches!(n, Node::MathOn { .. } | Node::MathOff { .. }))
            .cloned()
            .collect();
        // 去掉行尾 \\parfillskip（0pt plus 1fil；hpack 拉伸后宽度非 0，按 fil 阶识别）
        while let Some(Node::Glue {
            name: None,            stretch,
            stretch_order,
            ..
        }) = children.last()
        {
            if *stretch > 0 && *stretch_order == GLUE_ORDER_FIL {
                children.pop();
            } else {
                break;
            }
        }
        children
    }

    // ---------- M4-7 错误模型：数学错误消息 ----------

    /// 断言源码报错且消息含 `expected` 子串。
    fn assert_math_error(src: &str, expected: &str) {
        let err = typeset(src).unwrap_err();
        assert!(
            err.to_string().contains(expected),
            "{src:?} 应报 {expected:?}，实际：{err}"
        );
    }

    /// 断言转录含 `expected`（TeX 恢复式错误：报错后继续，消息入转录）。
    fn assert_math_transcript(src: &str, expected: &str) {
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset(src);
        let t = ts.take_transcript();
        assert!(
            t.contains(expected),
            "{src:?} 转录应含 {expected:?}：{t}"
        );
    }

    #[test]
    fn spacefactor_zero_rejected_without_panic() {
        // TRIP L289 `\spacefactor=0`：tex.web alter_aux 合法范围 1..32767，
        // 越界报 "! Bad space factor (n)." + help，且不改 space_factor
        // （旧实现直接赋 0 → 后续词间空白 xn_over_d(shrink,1000,0) 除零 panic）。
        let mut ts = Typesetter::with_metrics(metrics);
        ts.typeset(r"a\spacefactor=0 b").unwrap();
        let t = ts.take_transcript();
        assert!(
            t.contains("! Bad space factor (0)."),
            "应报 Bad space factor：{t}"
        );
        assert!(
            t.contains("I allow only values in the range 1..32767 here."),
            "应含 help 行：{t}"
        );
        // 越界不生效：后续空格按原 space_factor（1000）处理，不 panic、正常排版
    }

    // ---------- M4-2 数学原语 ----------

    /// 分式盒（`\over`/`\atop`）：vbox = [num 盒, glue, rule?, glue, den 盒]。
    fn fraction_box(text: &str) -> (BoxNode, bool) {
        let children = math_line_children(text);
        assert_eq!(children.len(), 1, "分式应封装为单盒：{text:?}");
        // make_fraction 壳盒 = hpack[null 定界符, 分式 vlist, null 定界符]；取中间 vlist
        let b = as_box(&children[0]);
        assert_eq!(b.kind, BoxKind::HBox, "分式壳是水平盒（tex.web L14659 hpack）");
        assert_eq!(b.children.len(), 3, "[定界符, vlist, 定界符]");
        let v = as_box(&b.children[1]);
        assert_eq!(v.kind, BoxKind::VBox, "分式是垂直堆叠");
        let has_rule = v.children.iter().any(|n| matches!(n, Node::Rule { .. }));
        (v.clone(), has_rule)
    }

    // ---------- M4-6 断字：\patterns ----------

    #[test]
    fn patterns_event_builds_hyphenation_trie() {
        // 端到端：\patterns{...} → sink 事件 → Liang trie（词断点计算）
        let mut b = NodeBuilder::new(Fonts::Fn {
            metrics: |_, _| (0, 0, 0),
            space: |_| Glue::ZERO,
        });
        b.patterns(b".ach4 hy3phen5ation".to_vec()).unwrap();
        assert_eq!(b.patterns.count, 2);
        // hy3phen5ation → hy-phen-ation（断点 2, 6）；.ach4 对 machine 无奇数 gap
        assert_eq!(b.patterns.hyphenate(b"hyphenation"), vec![2, 6]);
        assert_eq!(b.patterns.hyphenate(b"machine"), Vec::<usize>::new());
    }

    #[test]
    fn explicit_discretionary_hyphen_inserts_breakpoints() {
        fn collect<'a>(nodes: &'a [Node], out: &mut Vec<&'a Node>) {
            for n in nodes {
                if let Node::Discretionary { .. } = n {
                    out.push(n);
                }
                if let Node::Box(b) = n {
                    collect(&b.children, out);
                }
            }
        }

        let main = typeset_hyphen(r"\hbox{hy\-phen\-a\-tion}").unwrap();
        let mut discs = Vec::new();
        collect(&main, &mut discs);
        assert_eq!(discs.len(), 3, "\\- 应插入三个 discretionary：{main:?}");
        for disc in discs {
            let Node::Discretionary { pre, post, replace } = disc else {
                unreachable!();
            };
            assert_eq!(post.len(), 0);
            assert_eq!(replace.len(), 0);
            assert_eq!(pre.len(), 1);
            match &pre[0] {
                Node::Char { charcode, .. } => assert_eq!(*charcode, b'-' as u32),
                other => panic!("pre 应为连字符节点：{other:?}"),
            }
        }
    }

    /// 带窄词间胶水（3000/100000/500）的排版器：断字用例需要可拉伸词间空白。
    fn typeset_hyphen(src: &str) -> Result<Vec<Node>> {
        let mut ts = Typesetter::with_metrics(metrics).with_space(|_| Glue::new(3000, 100000, 500));
        ts.typeset(src)
    }

    /// `\lefthyphenmin`/`\righthyphenmin` 过滤（tex.web §924 `found:`）：
    /// 断点 `j` 仅在 `l_hyf <= j <= hn - r_hyf` 时保留。
    ///
    /// 这一条是 pdfTeX 实测锚定的（`\showhyphens`）：
    /// - `\lefthyphenmin=2 \righthyphenmin=3` + `Java`（hn=4）→ 无断点
    ///   （模式表给 2，但 2 号断点右片段只有 2 个字母 < 3 → 被清）；
    /// - `\lefthyphenmin=1 \righthyphenmin=1` + `Java` → `Ja-va`（2 号保留）。
    #[test]
    fn hyphenmin_filters_breaks_by_fragment_size() {
        // 直接构造单词的字母节点（charcode = 小写字母），绕开折行器只看断点。
        let word = |s: &[u8]| -> Vec<Node> {
            s.iter()
                .map(|&c| Node::Char {
                    font: FontId(0),
                    charcode: u32::from(c),
                    width: 1000,
                    height: 6000,
                    depth: 1500,
                })
                .collect()
        };
        // 模式表 ab5c → 唯一断点 2（"ab-cdef"）
        let build = |letters: &[u8], l: i64, r: i64| {
            let mut b = NodeBuilder::new(Fonts::Fn {
                metrics: |_, _| (1000, 6000, 1500),
                space: |_| Glue::ZERO,
            });
            b.patterns(b"ab5c".to_vec()).unwrap();
            b.params.misc[ntex_core::param::MISC_LEFT_HYPHEN_MIN] = l;
            b.params.misc[ntex_core::param::MISC_RIGHT_HYPHEN_MIN] = r;
            b.hyphenate_paragraph(word(letters))
                .iter()
                .filter(|n| matches!(n, Node::Discretionary { .. }))
                .count()
        };
        // hn=6：2 号断点在 [l_hyf, 6-r_hyf] 内与否决定去留
        assert_eq!(build(b"abcdef", 1, 1), 1, "l=1,r=1：2 在 [1,5] 内 → 保留");
        assert_eq!(build(b"abcdef", 2, 3), 1, "l=2,r=3：2 在 [2,3] 内 → 保留");
        assert_eq!(build(b"abcdef", 3, 1), 0, "l=3：左片段 2 < 3 → 清");
        assert_eq!(build(b"abcdef", 1, 5), 0, "r=5：右片段 4 < 5（j<=1）→ 清");
        // 词过短：hn=4 < l_hyf+r_hyf=5 → tex.web `if hn<l_hyf+r_hyf then goto done1`
        assert_eq!(build(b"abcd", 2, 3), 0, "hn < l_hyf+r_hyf：整个词不尝试断字");
    }

    /// 异常词表同样受 `l_hyf`/`r_hyf` 限制（tex.web `found:` 是两条路径的公共
    /// 汇合点，模式表与 `\hyphenation` 都要过同一个清理循环）。
    ///
    /// pdfTeX 实测（`\showhyphens`）：
    /// - `\lefthyphenmin=1 \righthyphenmin=1` + `\hyphenation{-abcd-}` → `abcd`
    ///   （词首 0 与词尾 4 都被清——`norm_min` 保证 `l_hyf >= 1`，词首断点恒不可达）
    /// - 同参数 + `\hyphenation{a-b-c-d-e}` → `a-b-c-d-e`（1..4 全在 [1, 4] 内）
    /// - `\lefthyphenmin=2 \righthyphenmin=3` + `\hyphenation{ab-cde}` → `ab-cde`
    #[test]
    fn hyphenmin_filters_exception_dictionary_breaks() {
        let word = |s: &[u8]| -> Vec<Node> {
            s.iter()
                .map(|&c| Node::Char {
                    font: FontId(0),
                    charcode: u32::from(c),
                    width: 1000,
                    height: 6000,
                    depth: 1500,
                })
                .collect()
        };
        let with_exception = |letters: &[u8], breaks: Vec<usize>, l: i64, r: i64| {
            let mut b = NodeBuilder::new(Fonts::Fn {
                metrics: |_, _| (1000, 6000, 1500),
                space: |_| Glue::ZERO,
            });
            b.hyphenation(vec![(letters.to_vec(), breaks)]).unwrap();
            b.params.misc[ntex_core::param::MISC_LEFT_HYPHEN_MIN] = l;
            b.params.misc[ntex_core::param::MISC_RIGHT_HYPHEN_MIN] = r;
            let out = b.hyphenate_paragraph(word(letters));
            out.iter()
                .enumerate()
                .filter(|(_, n)| matches!(n, Node::Discretionary { .. }))
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        };
        // \hyphenation{-abcd-} → 断点 [0, 4]；l=r=1 时两者都在 [1, 3] 之外 → 全清
        assert_eq!(
            with_exception(b"abcd", vec![0, 4], 1, 1),
            Vec::<usize>::new(),
            "词首/词尾断点应被 l_hyf/r_hyf 清掉"
        );
        // \hyphenation{a-b-c-d-e} → 断点 [1,2,3,4]；l=r=1 → 全保留
        // 输出 = [a,disc,b,disc,c,disc,d,disc,e]：断点 k 落在第 k 个字母之后，
        // 即下标 2k-1（k=1..4）→ [1,3,5,7]
        assert_eq!(
            with_exception(b"abcde", vec![1, 2, 3, 4], 1, 1),
            vec![1, 3, 5, 7],
            "1..4 全在 [l_hyf, hn-r_hyf] 内，应全部保留"
        );
        // \hyphenation{ab-cde} → 断点 [2]；l=2,r=3 时 2 ∈ [2, 5-3] → 保留
        assert_eq!(with_exception(b"abcde", vec![2], 2, 3), vec![2], "2 号断点应保留");

        // 同词表但 r_hyf=4：右片段须 >= 4，2 号断点被清（hn=5 → j <= 1）
        assert_eq!(
            with_exception(b"abcde", vec![2], 2, 4),
            Vec::<usize>::new(),
            "r_hyf=4 时 2 号断点右片段 3 < 4，应被清"
        );
    }

    #[test]
    fn patterns_hyphenates_word_across_lines() {
        // \patterns{ab5c} → "abcdefgh" 断点 2（ab-cdefgh）。
        // 窄 \hsize 下折行唯一可行路径断在词内：
        // 行1 = "m ab-"（discretionary pre 连字符收尾）、行2 = "cdefgh n"。
        let main = typeset_hyphen(r"\patterns{ab5c}\hsize 10700sp m abcdefgh n").unwrap();
        // [行1, interline penalty, interline glue, 行2]（行间惩罚节点 tex.web 语义）
        assert_eq!(main.len(), 4, "应折成两行（行间 penalty + interline glue）：{main:?}");
        let l1 = as_box(&main[0]);
        // [m, 词间 glue, a, b, 连字符]
        assert_eq!(l1.children.len(), 5, "行1 = m ab-：{l1:?}");
        assert_eq!(as_char(&l1.children[0]), b'm' as u32);
        assert!(matches!(l1.children[1], Node::Glue { .. }));
        assert_eq!(as_char(&l1.children[2]), b'a' as u32);
        assert_eq!(as_char(&l1.children[3]), b'b' as u32);
        assert_eq!(as_char(&l1.children[4]), b'-' as u32, "行尾应补连字符");
        assert!(matches!(main[1], Node::Penalty { .. }), "行间应插 interline penalty");
        assert!(matches!(main[2], Node::Glue { .. }), "行间应插 interline glue");
        let l2 = as_box(&main[3]);
        // [c d e f g h, 词间 glue, n, \parfillskip]
        assert_eq!(l2.children.len(), 9, "行2 = cdefgh n：{l2:?}");
        for (k, ch) in (b'c'..=b'h').enumerate() {
            assert_eq!(as_char(&l2.children[k]), ch as u32);
        }
        assert!(matches!(l2.children[6], Node::Glue { .. }));
        assert_eq!(as_char(&l2.children[7]), b'n' as u32);
        assert!(matches!(l2.children[8], Node::Glue { .. }));
    }

    #[test]
    fn without_patterns_word_stays_whole() {
        // 无 \patterns：词内无断点，行超宽且收缩不足 → tex.web artificial
        // demerits 在空格断点逐断点成行（真实 TeX 同为多条 Overfull 行）；
        // 词本身保持完整不断字。
        let main = typeset_spaced(r"\hsize 10700sp m abcdefgh n").unwrap();
        assert_eq!(main.len(), 4, "行1(m abcdefgh 过满) + 惩罚/胶 + 行2(n)：{main:?}");
        let l1 = as_box(&main[0]);
        // 行1 = [m, glue, a..h]（10 节点，字母连续不断字）
        assert_eq!(l1.children.len(), 10, "行1 = m abcdefgh：{l1:?}");
        assert_eq!(as_char(&l1.children[0]), b'm' as u32);
        for (k, ch) in (b'a'..=b'h').enumerate() {
            assert_eq!(as_char(&l1.children[k + 2]), ch as u32, "字母不断字");
        }
        let l2 = as_box(&main[3]);
        assert_eq!(as_char(&l2.children[0]), b'n' as u32, "行2 = n");
    }

    // ---------- ETRIP 冲刺：e-TeX marks 族状态语义 ----------

    /// 无字体依赖的 NodeBuilder（marks 状态在 NodeBuilder 上）。
    fn marks_builder() -> NodeBuilder {
        NodeBuilder::new(Fonts::Fn {
            metrics: |_, _| (0, 0, 0),
            space: |_| Glue::ZERO,
        })
    }

    #[test]
    fn marks_record_first_and_bot() {
        let mut b = marks_builder();
        b.mark(Some(1), "first".into()).unwrap();
        b.mark(Some(1), "second".into()).unwrap();
        b.mark(Some(1), "third".into()).unwrap();
        // first = 首次出现；bot = 末次出现；top 初始为空
        assert_eq!(b.firstmarks(1), "first");
        assert_eq!(b.botmarks(1), "third");
        assert_eq!(b.topmarks(1), "");
        // 未出现的 class 查询为空
        assert_eq!(b.firstmarks(2), "");
        assert_eq!(b.botmarks(2), "");
        assert_eq!(b.topmarks(2), "");
        // split* 接口预留（\vsplit 未实现）→ 恒空
        assert_eq!(b.splitfirstmarks(1), "");
        assert_eq!(b.splittopmarks(1), "");
        assert_eq!(b.splitbotmarks(1), "");
    }

    #[test]
    fn mark_aliases_class_zero() {
        // \mark ≡ \marks0（class=None 映射 0）
        let mut b = marks_builder();
        b.mark(None, "aliased".into()).unwrap();
        assert_eq!(b.firstmarks(0), "aliased");
        assert_eq!(b.botmarks(0), "aliased");
        // 显式 \marks0 与 \mark 共享 class 0 状态
        b.mark(Some(0), "explicit".into()).unwrap();
        assert_eq!(b.firstmarks(0), "aliased", "first 保持首次出现");
        assert_eq!(b.botmarks(0), "explicit", "bot 更新为末次出现");
    }

    #[test]
    fn rotate_marks_inherits_bot_as_top() {
        let mut b = marks_builder();
        b.mark(Some(1), "p1-first".into()).unwrap();
        b.mark(Some(1), "p1-bot".into()).unwrap();
        b.rotate_marks();
        // 断页轮转：top = 旧 bot；first 清空；bot = top（继承）
        assert_eq!(b.topmarks(1), "p1-bot");
        assert_eq!(b.firstmarks(1), "");
        assert_eq!(b.botmarks(1), "p1-bot");
        // 新页新 marks 重新记录
        b.mark(Some(1), "p2-a".into()).unwrap();
        b.mark(Some(1), "p2-b".into()).unwrap();
        assert_eq!(b.firstmarks(1), "p2-a");
        assert_eq!(b.botmarks(1), "p2-b");
        assert_eq!(b.topmarks(1), "p1-bot", "top 保持上一页继承值");
        // 再次轮转
        b.rotate_marks();
        assert_eq!(b.topmarks(1), "p2-b");
        assert_eq!(b.firstmarks(1), "");
        assert_eq!(b.botmarks(1), "p2-b");
    }

    #[test]
    fn marks_query_empty_without_state() {
        // 从未 mark 过的 class：全部查询为空串（不 panic）
        let b = marks_builder();
        assert_eq!(b.topmarks(0), "");
        assert_eq!(b.firstmarks(0), "");
        assert_eq!(b.botmarks(0), "");
        assert_eq!(b.splitfirstmarks(0), "");
        assert_eq!(b.splittopmarks(0), "");
        assert_eq!(b.splitbotmarks(0), "");
    }

    /// 递归收集所有字符节点（数学内容保留断言用）。
    fn collect_chars(nodes: &[Node], out: &mut Vec<u32>) {
        for n in nodes {
            match n {
                Node::Char { charcode, .. } => out.push(*charcode),
                Node::Box(b) => collect_chars(&b.children, out),
                _ => {}
            }
        }
    }

    #[test]
    fn paragraph_lines_include_left_right_skip() {
        // tex.web：折行每行行首 \leftskip、行尾 \rightskip（参考行结构
        // `.\glue(\leftskip) 3.0 ... .\glue(\rightskip) 0.0`——此前行盒
        // 只有内容，- 侧 125+ 行 glue(leftskip/rightskip) 缺失）
        let main = typeset(r"\leftskip 3pt a\par b").unwrap();
        assert!(main.len() >= 2, "两行：{main:?}");
        let l1 = as_box(&main[0]);
        let has_ls = matches!(
            l1.children.first(),
            Some(Node::Glue { width, .. }) if *width == 3 * SP_PER_PT
        );
        assert!(has_ls, "行 1 行首缺 \\leftskip 3pt glue：{:?}", l1.children.first());
        let has_rs = matches!(
            l1.children.last(),
            Some(Node::Glue { .. })
        );
        assert!(has_rs, "行 1 行尾缺 \\rightskip glue：{:?}", l1.children.last());
        // 行 2 同
        let l2 = as_box(&main[2]);
        let has_ls2 = matches!(
            l2.children.first(),
            Some(Node::Glue { width, .. }) if *width == 3 * SP_PER_PT
        );
        assert!(has_ls2, "行 2 行首缺 \\leftskip glue");
    }

    #[test]
    fn mathchar_produces_char_node() {
        // tex.web：\mathchar"322D（数学 15-bit：class 3/fam 2/char "2D）在数学中
        // 是原子；"2D = '-' 字符必须出现在渲染输出。此前 scan_number 即丢。
        let main = typeset(r#"\hbox{$\mathchar"322D$}"#).unwrap();
        let mut chars = Vec::new();
        collect_chars(&main, &mut chars);
        assert!(
            chars.contains(&0x2D),
            "mathchar 字符 - 丢失: {chars:?}"
        );
    }

    // ---------- 输出例程刀 1：\outputpenalty + \deadcycles ----------
    //
    // tex.web fire_up `@<Set the value of |output_penalty|@>`：最佳断点是惩罚
    // 节点 → \outputpenalty := 其惩罚；否则（胶水/kern 自然断页）→ inf_penalty
    // (10000)。latex.ltx 罚分协议六档（docs/archive/output-routine-survey.md §2.2bis）
    // 全靠它分派，档位差 ±1 就换路，不能模糊化。
    //
    // 探针用 \showthe（转录专用、不进节点流）。勘察报告刀 1 原规格的
    // `\typeout{p=\the\outputpenalty}`（=\immediate\write16）在 NTex 会被
    // 既有缺陷卡死：\write 的 whatsit 节点在例程内落入主列表 → 永久冲页循环
    // （真实 TeX 同探针正常，已单独勘误）。

    /// NTex \showthe 行 → 值（`> \outputpenalty=-10000.` → `-10000`）。
    fn showthe_values(t: &str, name: &str) -> Vec<String> {
        let head = format!("> \\{name}=");
        t.lines()
            .filter(|l| l.starts_with(&head))
            .map(|l| l[head.len()..].trim_end_matches('.').to_string())
            .collect()
    }

    fn probe_transcript(src: &str) -> String {
        let mut ts =
            Typesetter::with_metrics(metrics).with_space(|_| Glue::new(1000, 500, 300));
        // \vsize 取大值（真 TeX 探针同款）：页未满，断页由惩罚/eject 触发，
        // 与真 TeX 对拍（TinyTeX plain 同源探针，§5.2 勘误附录）。
        let _ = ts.typeset_dvi(&format!(
            r"\vsize 40000000sp\hsize 20000000sp \output={{\showthe\outputpenalty\shipout\box255}} {src}"
        ));
        ts.take_transcript()
    }

    // ---------- 输出例程刀 2：box255 寄存器化 ----------
    //
    // tex.web 裁决：box(255) 就是普通盒子寄存器——fire_up
    // @<Break the current page at node |p|, put it in box~255...@> 直接
    // `box(255):=vpackage(link(page_head),best_size,exactly,page_max_depth)`
    // （同层裸写），例程负责消费，例程结束 @<Ensure that box 255 is empty
    // after output@> 检查。NTex 例程延迟到 token 边界注入 → pending_pages
    // 队列即 255 的物理存储、队首即寄存器内容（PAGE_BOX 注释）；本刀把
    // \box/\copy/\unhbox/\unvbox/\vsplit/\ifvoid/\ifhbox/\ifvbox/\ht/\wd/\dp/
    // \setbox/\showbox 全部寄存器访问路径统一路由（此前只有 \box255 感知队列，
    // `\unvbox\@cclv`×2、`\vsplit\@cclv to\z@`×1、`\ifvoid\@cclv`、`\ht\@cclv`
    // 恒 void——latex.ltx `\@doclearpage`/`\@specialoutput` 必炸）。
    //
    // 真 TeX 对照（TinyTeX 2026 plain，/tmp/knife2/p*.tex；探针与真实 TeX
    // 同源，例程内经 \showthe 转录——刀 1 既定纪律，\write 在例程内挂死是
    // 既有缺陷）：
    //   P1 页在：`\ifvoid255`=N、`\ifvbox255`=Y、`\ifhbox255`=N、
    //            `\ht255`=30.0pt、`\dp255`=1.94444pt、`\wd255`=100.0pt(=\hsize)
    //   P2 `\setbox2=\vbox{\unvbox255}` → shipout 盒树子节点与直通
    //      `\shipout\box255` 逐项同形
    //   P3 `\setbox0=\vsplit255 to 10pt` → `\ht0`=10.0pt（exactly）、余量留 255
    //   P4 `\setbox255=\vbox{\box255\vfil}` → `\ht255`=31.94444pt
    //      （=30.0+1.94444 自然高；latex.ltx L20914 同形）
    //   P5 void 255：`\ifvoid255`=Y、`\ht255`=0.0pt、`\vsplit255 to 10pt` 静默
    //      （box0 void、255 保持 void、无错误信息）
    //
    // 例程体末尾 `\relax`：引擎例程帧末 token 的参数扫描前瞻会提前触发
    // end_group（既有 quirk，expand/mod.rs 禁碰）——`...\shipout\box<n>` 裸尾
    // 会让本例程 `\setbox` 的组级回滚先于 ship 落地（预存问题，与本刀无关）。

    /// NTex \showthe 行 → 数值（`> \dimen=30.0pt.` → `30.0`；\showthe 印 cs 名
    /// 不带寄存器号，故每个探针借道一个 \dimen/\count 寄存器、按序断言）。
    fn showthe_dimens(t: &str, name: &str) -> Vec<f64> {
        let head = format!("> \\{name}=");
        t.lines()
            .filter(|l| l.starts_with(&head))
            .map(|l| {
                l[head.len()..]
                    .trim_end_matches('.')
                    .trim_end_matches("pt")
                    .trim_end_matches('.')
                    .parse::<f64>()
                    .unwrap_or(f64::NAN)
            })
            .collect()
    }

    // ── 按域拆分的测试子模块（R2 纯移动，helper 留主文件走 super::* 链） ──
    mod tests_box { include!("tests_box.rs"); }
    mod tests_page { include!("tests_page.rs"); }
    mod tests_insert { include!("tests_insert.rs"); }
    mod tests_math { include!("tests_math.rs"); }
    mod tests_align { include!("tests_align.rs"); }
    mod tests_plain_format { include!("tests_plain_format.rs"); }
}
