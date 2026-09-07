#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::BoxKind;
    use ntex_core::SP_PER_PT;
    use ntex_core::TokenSink;

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
        // tex.web：\vskip 不改写 prev_depth，故 \hbox{b} 落盒仍按前驱盒插行间
        // glue（d = baselineskip 12pt - depth 1500 - height 6000 = 778932）
        assert_eq!(v.children.len(), 4, "盒+\\vskip+行间glue+盒：{v:?}");
        assert!(matches!(v.children[0], Node::Box(_)));
        match &v.children[1] {
            Node::Glue { width, .. } => assert_eq!(*width, 10 * SP_PER_PT),
            other => panic!("预期 Glue，得到 {other:?}"),
        }
        match &v.children[2] {
            Node::Glue { width, .. } => assert_eq!(*width, 12 * SP_PER_PT - 1500 - 6000, "行间 glue"),
            other => panic!("预期行间 Glue，得到 {other:?}"),
        }
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
        // tex.web：\vskip 不改写 prev_depth → 末段落盒落盒仍插行间 glue
        // （baselineskip 12pt - depth 1500 - height 6000 = 778932）
        assert_eq!(main.len(), 4, "隐式 \\par 后应为 段落盒+glue+行间glue+段落盒");
        assert!(matches!(main[0], Node::Box(_)), "首项应为断行后的段落盒");
        match &main[1] {
            Node::Glue { width, .. } => assert_eq!(*width, 6 * SP_PER_PT),
            other => panic!("预期 Glue，得到 {other:?}"),
        }
        match &main[2] {
            Node::Glue { width, .. } => assert_eq!(*width, 12 * SP_PER_PT - 1500 - 6000, "行间 glue"),
            other => panic!("预期行间 Glue，得到 {other:?}"),
        }
        assert!(matches!(main[3], Node::Box(_)), "末项应为新起的段落盒");
    }

    /// 同上：\vfill 类无限阶垂直胶水在水平模式同样先隐式 \par（\vfil kind=3）。
    #[test]
    fn vfil_in_paragraph_ends_it_implicitly() {
        let main = typeset(r"a\vfil b").unwrap();
        // 行间 glue（tex.web append_to_vlist）：baselineskip 12pt - depth 1500
        // - height 6000 = 778932（\vfil 不改写 prev_depth）
        assert_eq!(main.len(), 4, "\\vfil 后应为 段落盒+glue+行间glue+段落盒");
        assert!(matches!(main[0], Node::Box(_)));
        assert!(matches!(main[1], Node::Glue { .. }));
        match &main[2] {
            Node::Glue { width, .. } => assert_eq!(*width, 12 * SP_PER_PT - 1500 - 6000),
            other => panic!("预期行间 Glue，得到 {other:?}"),
        }
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
    fn unclosed_box_group_is_rejected() {
        assert!(typeset(r"\hbox{a").is_err());
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
        // 宏展开、寄存器、\the 在盒子内容里正常工作
        let main = typeset(r"\hbox{\def\x{xy}\x\count0=7\the\count0}").unwrap();
        let b = as_box(&main[0]);
        let chars: Vec<u32> = b.children.iter().map(as_char).collect();
        assert_eq!(chars, vec![b'x' as u32, b'y' as u32, b'7' as u32]);
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
        let children = spaced_box_children(r"\hbox{a\hskip 5pt  b}");
        assert_eq!(children.len(), 3); // a + glue(5pt) + b（空格被忽略）
        match &children[1] {
            Node::Glue { width, .. } => assert_eq!(*width, 5 * SP_PER_PT),
            other => panic!("预期 5pt Glue，得到 {other:?}"),
        }
        let children = spaced_box_children(r"\hbox{a\penalty -10  b}");
        assert_eq!(children.len(), 3); // a + penalty + b
        assert!(matches!(children[1], Node::Penalty { .. }));
    }

    #[test]
    fn space_at_hbox_start_ignored() {
        let children = spaced_box_children(r"\hbox{ a}");
        assert_eq!(children.len(), 1);
        assert_eq!(as_char(&children[0]), b'a' as u32);
    }

    #[test]
    fn dimen_scan_swallows_trailing_space() {
        // \kern 7pt 后的空格被 dimen 扫描吞掉（TeX 规则），不产生词间胶水
        let children = spaced_box_children(r"\hbox{a\kern 7pt b}");
        assert_eq!(children.len(), 3); // a + kern + b
        assert!(matches!(children[1], Node::Kern { .. }));
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
        let mut ts = Typesetter::with_tfm();
        assert!(ts.typeset(r"\font\x=definitely_not_a_font").is_err());
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
    fn box_register_void_errors() {
        // \box255 在无页面（void）时取用 → 报错
        assert!(
            paginated(r"\shipout\box255").is_err(),
            "void \\box255 应报错"
        );
    }

    #[test]
    fn output_local_restores_after_group() {
        let src = fill_words();
        let default = paginated(&src).unwrap();
        // 组内定义 \output，组结束恢复未定义 → 行为与默认一致（直通 shipout）
        let with = paginated(&format!(r"{{\output={{\shipout\box255}}}} {src}")).unwrap();
        assert_eq!(with, default, "组结束应恢复未定义 \\output");
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

    // ---------- 输出例程刀 3（G3a）：insert 结构化——token 体保留 ----------

    /// 真 TeX 探针 1（/tmp/knife3/p1.tex，三参数赋值移到体外——体内的赋值要
    /// 等体在扫描位执行才生效，见 survey §5.bis.4 发现未修 1）：
    /// `\splittopskip=10pt plus2fil \splitmaxdepth=1pt \floatingpenalty=200
    /// \setbox0=\vbox{\insert150{\hbox{FN}}}\showbox0` 的 ins 节点行。
    /// 真 TeX 同探针：`\insert150, natural size 6.83331; split(10.0 plus 2.0fil,1.0);
    /// float cost 200` + `.\hbox(...)` 体子树；NTex 体未排版 → natural size 0.0
    /// 占位、体以 token 串显示（三参数与脚注文本都在）。
    #[test]
    fn insert_body_tokens_and_split_params_preserved() {
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset_dvi(
            r"\splittopskip=10pt plus2fil \splitmaxdepth=1pt \floatingpenalty=200 \setbox0=\vbox{\insert150{\hbox{FN}}}\showbox0\end",
        );
        let t = ts.take_transcript();
        assert!(
            t.contains(
                r"\insert150, natural size 0.0; split(10.0 plus 2.0fil,1.0); float cost 200"
            ),
            "ins 节点行应按 tex.web show_node 格式带三参数：{t:?}"
        );
        assert!(t.contains("FN"), "体 token 应读得出脚注文本：{t:?}");
        // 此前 toks_to_text 把 cs 全丢——`\hbox` 的组结构现在仍在体 token 串里
        assert!(
            t.contains("\\cs"),
            "体 token 串应保留 cs（\\cs<下标> 占位显示）：{t:?}"
        );
    }

    /// 真 TeX 探针 2（/tmp/knife3/p2.tex）：断页（fire_up）后插入体进入
    /// `box(class)` → `\ifvoid150` 为假。真 TeX：`FULL`。
    #[test]
    fn insert_register_nonvoid_after_page_break() {
        let mut ts = Typesetter::with_metrics(metrics);
        // 真 TeX 探针 2（p4.tex）：\penalty-10000 与 \ifvoid 紧邻时条件在数字
        // 扫描的前瞻位求值（真 TeX 同款：p3.tex 紧邻 → VOID、p4.tex 带空格 →
        // FULL），故探针用空格隔开。
        let _ = ts.typeset_dvi(
            r"\hbox{X}\insert150{\hbox{A}}\vfill\penalty-10000 \ifvoid150\message{VOID}\else\message{FULL}\fi\end",
        );
        let t = ts.take_transcript();
        assert!(t.contains("FULL"), "断页后 \\ifvoid150 应为假：{t:?}");
        assert!(!t.contains("VOID"), "不应走 void 臂：{t:?}");
    }

    /// `\unvbox150` 取走累积盒（ins 节点回流），`\ifvoid150` 复归 void——
    /// 真 TeX 探针 2 的 `\setbox3=\vbox{\unvbox150}\showbox3`：box3 非空、150 void。
    #[test]
    fn insert_unvbox_roundtrip_empties_register() {
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset_dvi(
            r"\hbox{X}\insert150{\hbox{A}}\vfill\penalty-10000 \setbox3=\vbox{\unvbox150}\showbox3\ifvoid150\message{VOID}\else\message{FULL}\fi\end",
        );
        let t = ts.take_transcript();
        assert!(
            t.contains("\\insert150"),
            "回流内容应带着 ins 节点进 box3 的盒树：{t:?}"
        );
        assert!(
            t.contains("VOID"),
            "取走语义：\\unvbox 后 150 应复归 void：{t:?}"
        );
    }

    /// 断页产出的页面盒树**不含** ins 节点（tex.web fire_up 删除 ins_node——
    /// `\insert` 体已进 box(class)，脚注由输出例程回流）。盒组内（无 fire_up）
    /// 的 ins 节点原地保留（探针 1 的 `\showbox0` 路径）。
    #[test]
    fn page_break_moves_insert_out_of_page_box() {
        let pages = paginated(r"\hbox{X}\insert150{\hbox{A}}\vfill\penalty-10000\end").unwrap();
        assert_eq!(pages.len(), 1, "单页");
        assert!(
            pages[0]
                .children
                .iter()
                .all(|n| !matches!(n, Node::Ins { .. })),
            "页盒树不应含 ins 节点：{:?}",
            pages[0].children
        );
        // 盒组内不动：ins 节点留在 vbox 0
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset_dvi(r"\setbox0=\vbox{\insert150{\hbox{A}}}\showbox0\end");
        assert!(
            ts.take_transcript().contains(r"\insert150"),
            "盒组内 ins 节点应保留："
        );
    }

    /// `\insert255`：tex.web begin_insert_or_adjust 报错改道 0
    /// （"I'm changing to \insert0; box 255 is special."）——255 是页面寄存器，
    /// 放行会写穿页队列。
    #[test]
    fn insert255_redirects_to_zero_with_error() {
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset_dvi(
            r"\hbox{X}\insert255{\hbox{A}}\vfill\penalty-10000 \ifvoid0\message{VOID}\else\message{FULL}\fi\end",
        );
        let t = ts.take_transcript();
        assert!(t.contains("You can't \\insert255."), "应报错：{t:?}");
        assert!(
            t.contains("I'm changing to \\insert0; box 255 is special."),
            "应带 help 行：{t:?}"
        );
        assert!(t.contains("FULL"), "改道后 \\ifvoid0 应为假：{t:?}");
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
        assert_eq!(vfs.get("o.txt"), Some(b"42\n".as_slice()));
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

    // ---------- M4-1 数学模式 ----------

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

    #[test]
    fn math_italic_correction_kern_after_ord_char() {
        if cmmi10_metrics().is_none() {
            eprintln!("未找到 cmmi10.tfm，跳过");
            return;
        }
        // 官方对照（tex \tracingoutput showbox）：`...\tenmi E \kern0.57637`
        // （0.57637pt = 37773sp = char_italic(cmmi10, E)，fix_word 截断取整）
        let children = math_tfm_children(r#"\font\tenmi=cmmi10\textfont1=\tenmi\mathcode`\E="0145 $E$"#);
        assert_eq!(children.len(), 2, "E 后应跟斜体修正 kern：{children:?}");
        assert_eq!(as_char(&children[0]), b'E' as u32);
        assert_eq!(children[1], Node::Kern { width: 37_773 });
        // italic=0 的字符不追加（官方 `mc^2`：c 后无 kern，right294003 全为 sup 盒宽）
        let plain = math_tfm_children(r#"\font\tenmi=cmmi10\textfont1=\tenmi\mathcode`\m="016D $m$"#);
        assert_eq!(plain.len(), 1, "m（italic=0）不应有 kern：{plain:?}");
        assert_eq!(as_char(&plain[0]), b'm' as u32);
    }

    #[test]
    fn math_italic_kern_sup_only_but_not_sub_only() {
        if cmmi10_metrics().is_none() {
            eprintln!("未找到 cmmi10.tfm，跳过");
            return;
        }
        // 官方对照：`f^2` → `\tenmi f \kern1.0764 \hbox`（70543sp）；
        // `f_2` → `\tenmi f \hbox(...)`（无 kern，delta 转 make_scripts 偏移）
        let sup = math_tfm_children(r#"\font\tenmi=cmmi10\textfont1=\tenmi\mathcode`\f="0166 $f^2$"#);
        assert_eq!(as_char(&sup[0]), b'f' as u32);
        assert_eq!(sup[1], Node::Kern { width: 70_543 }, "仅上标仍追加斜体 kern");
        assert!(matches!(sup[2], Node::Box(_)));
        let sub = math_tfm_children(r#"\font\tenmi=cmmi10\textfont1=\tenmi\mathcode`\f="0166 $f_2$"#);
        assert_eq!(as_char(&sub[0]), b'f' as u32);
        assert!(matches!(sub[1], Node::Box(_)), "带下标时 delta 交脚本偏移，不落 kern：{sub:?}");
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

    #[test]
    fn math_inline_formula_in_paragraph() {
        let children = math_line_children(r"$x$");
        assert_eq!(children.len(), 1);
        assert_eq!(as_char(&children[0]), b'x' as u32);
    }

    #[test]
    fn math_superscript_builds_script_box() {
        let children = math_line_children(r"$x^2$");
        assert_eq!(children.len(), 2, "x 后应挂上标盒");
        assert_eq!(as_char(&children[0]), b'x' as u32);
        let sup = as_box(&children[1]);
        assert_eq!(sup.kind, BoxKind::HBox);
        assert_eq!(sup.children.len(), 1);
        assert_eq!(as_char(&sup.children[0]), b'2' as u32);
        // 脚本字阶缩放：宽 7/10 × (1000+50)；高 7/10 × 6000
        assert_eq!(sup.children[0].dimensions().width, xn_over_d(1050, 7, 10));
        assert_eq!(sup.children[0].dimensions().height, 4200);
        // tex.web make_scripts：shift_up ≥ depth(sup 盒)+x_height/4。fn 指针模式
        // family-2 无字体（sup2=0、x_height=0）→ 退为 depth(sup)=1050
        assert_eq!(sup.shift, -1050);
    }

    #[test]
    fn math_sub_and_superscript_both() {
        let children = math_line_children(r"$x_1^2$");
        assert_eq!(children.len(), 3, "x + 上标盒 + 下标盒");
        assert_eq!(as_char(&children[0]), b'x' as u32);
        assert_eq!(as_char(&as_box(&children[1]).children[0]), b'2' as u32);
        assert_eq!(as_char(&as_box(&children[2]).children[0]), b'1' as u32);
    }

    #[test]
    fn math_sub_then_sup_attach_to_same_base() {
        // x_1^2 与 x^2_1 等价：同一 base 双侧脚本
        let a = math_line_children(r"$x_1^2$");
        let b = math_line_children(r"$x^2_1$");
        assert_eq!(a, b);
    }

    #[test]
    fn math_group_script_field() {
        let children = math_line_children(r"$x^{ab}$");
        assert_eq!(children.len(), 2);
        let sup = as_box(&children[1]);
        assert_eq!(sup.children.len(), 2);
        assert_eq!(as_char(&sup.children[0]), b'a' as u32);
        assert_eq!(as_char(&sup.children[1]), b'b' as u32);
    }

    #[test]
    fn math_group_splices_into_list() {
        // {ab} 数学组直接并入外层（等价 ab）
        let a = math_line_children(r"${ab}$");
        let b = math_line_children(r"$ab$");
        assert_eq!(a, b);
    }

    #[test]
    fn math_ignores_spaces() {
        let children = math_line_children(r"$a b$");
        assert_eq!(children.len(), 2, "数学模式空格应忽略");
        assert_eq!(as_char(&children[0]), b'a' as u32);
        assert_eq!(as_char(&children[1]), b'b' as u32);
    }

    #[test]
    fn math_nested_scripts_use_scriptscript_scale() {
        // x_{y^z}：z 为第三级脚本（5/10 缩放）
        let children = math_line_children(r"$x_{y^z}$");
        assert_eq!(children.len(), 2);
        let sub = as_box(&children[1]);
        assert_eq!(sub.children.len(), 2, "y + z 上标盒");
        let z = &sub.children[1];
        let zw = z.dimensions().width;
        // 嵌套上标盒同样是 script 盒：宽含 \scriptspace（tex.web make_scripts）
        assert_eq!(zw, xn_over_d(1000 + b'z' as i64, 5, 10) + 32768);
    }

    #[test]
    fn math_display_formula() {
        // $$x$$（垂直模式，无前驱）：predisplaypenalty + 上间距 + 居中公式盒 +
        // postdisplaypenalty + 下间距（tex.web finish_display 顺序：postdisplaypenalty
        // 在下间距**之前**）。
        // 空段（tex.web head=tail 臂）pre_display_size = -max_dimen，d+s > 它 →
        // 短间距（abovedisplayshortskip = 0pt plus 3pt / belowdisplayshortskip =
        // 7pt plus 3pt minus 4pt）。
        let main = typeset(r"$$x$$").unwrap();
        assert_eq!(main.len(), 5, "显示公式 = 前后 penalty + 上下间距 + 公式盒：{main:?}");
        match &main[0] {
            Node::Penalty { penalty } => assert_eq!(*penalty, 10_000, "predisplaypenalty 默认 10000"),
            other => panic!("预期 predisplaypenalty，得到 {other:?}"),
        }
        match &main[1] {
            Node::Glue { width, stretch, shrink, .. } => {
                assert_eq!(*width, 0, "abovedisplayshortskip 宽 0（短间距）");
                assert_eq!(*stretch, 3 * SP_PER_PT);
                assert_eq!(*shrink, 0);
            }
            other => panic!("预期 abovedisplayshortskip，得到 {other:?}"),
        }
        let boxed = as_box(&main[2]);
        // 公式盒 = \hbox to \hsize 居中（两侧 \hfil），中为 x
        assert_eq!(boxed.width, 13 * 4_736_286 / 2, "公式盒宽 = \\hsize");
        assert_eq!(boxed.children.len(), 3, "hfil + x + hfil");
        assert_eq!(as_char(&boxed.children[1]), b'x' as u32);
        match &main[3] {
            Node::Penalty { penalty } => assert_eq!(*penalty, 0, "postdisplaypenalty 默认 0"),
            other => panic!("预期 postdisplaypenalty，得到 {other:?}"),
        }
        match &main[4] {
            Node::Glue { width, .. } => assert_eq!(*width, 7 * SP_PER_PT, "belowdisplayshortskip"),
            other => panic!("预期 belowdisplayshortskip，得到 {other:?}"),
        }
    }

    #[test]
    fn math_display_short_skip_after_short_line() {
        // 段中 $$：d+s = half(\displaywidth-公式宽) > pre_display_size（末行自然宽
        // + 2em）→ 短间距（abovedisplayshortskip = 0pt plus 3pt、
        // belowdisplayshortskip = 7pt plus 3pt minus 4pt）。
        // 10000sp 下 d = half(10000-1120) = 4440 > 1097 = 末行宽(a) → 短间距。
        let main = typeset(r"\hsize 10000sp a $$x$$").unwrap();
        // [行(a), penalty, 上短间距, 行间glue, 公式盒, penalty, 下短间距]
        assert_eq!(main.len(), 7, "段中短行公式：{main:?}");
        match &main[2] {
            Node::Glue { width, stretch, shrink, .. } => {
                assert_eq!(*width, 0, "abovedisplayshortskip 宽 0");
                assert_eq!(*stretch, 3 * SP_PER_PT);
                assert_eq!(*shrink, 0);
            }
            other => panic!("预期 abovedisplayshortskip，得到 {other:?}"),
        }
        match &main[6] {
            Node::Glue { width, .. } => assert_eq!(*width, 7 * SP_PER_PT, "belowdisplayshortskip"),
            other => panic!("预期 belowdisplayshortskip，得到 {other:?}"),
        }
        // tex.web append_to_vlist：公式盒落盒前按 prev_depth 插行间 glue
        // （baselineskip 12pt - depth 1500 - height 6000 = 778932）
        match &main[3] {
            Node::Glue { width, .. } => assert_eq!(*width, 12 * SP_PER_PT - 1500 - 6000, "行间 glue"),
            other => panic!("预期行间 Glue，得到 {other:?}"),
        }
    }

    #[test]
    fn math_display_long_skip_after_full_line() {
        // 段中 $$：\hsize 极窄 → d = half(500-1120) = -310 <= pre_display_size
        // （clearance 不足）→ 长间距（abovedisplayskip 12pt）。
        let main = typeset(r"\hsize 500sp a $$x$$").unwrap();
        assert_eq!(main.len(), 7, "段中满行公式：{main:?}");
        match &main[2] {
            Node::Glue { width, .. } => assert_eq!(*width, 12 * SP_PER_PT, "abovedisplayskip"),
            other => panic!("预期 abovedisplayskip，得到 {other:?}"),
        }
    }

    #[test]
    fn math_display_skips_configurable() {
        // \abovedisplayskip/\belowdisplayskip 可赋值（无 = 形式，同现有测试风格）。
        // 公式盒取超宽（\hbox to 30000sp）→ d < 0 <= pre_display_size → 长间距被选中。
        let main =
            typeset(r"\abovedisplayskip 5pt\belowdisplayskip 3pt\hsize 500sp a $$x$$").unwrap();
        match &main[2] {
            Node::Glue { width, .. } => assert_eq!(*width, 5 * SP_PER_PT),
            other => panic!("abovedisplayskip 应生效：{other:?}"),
        }
        match &main[6] {
            Node::Glue { width, .. } => assert_eq!(*width, 3 * SP_PER_PT),
            other => panic!("belowdisplayskip 应生效：{other:?}"),
        }
    }

    #[test]
    fn math_display_paragraph_continues_after_formula() {
        // 段中公式：ab $$x$$ cd → 行(ab) + 公式垂直元素 + 行(cd)，续排无 parskip/缩进
        let main = typeset(r"ab $$x$$ cd").unwrap();
        // [行(ab), penalty, 上间距, 行间glue, 公式盒, penalty, 下间距, 行间glue, 行(cd)]
        assert_eq!(main.len(), 9, "公式前后文字各成行：{main:?}");
        let l1 = as_box(&main[0]);
        assert_eq!(as_char(&l1.children[0]), b'a' as u32);
        assert!(matches!(main[1], Node::Penalty { .. }));
        assert!(matches!(main[2], Node::Glue { .. }));
        assert!(matches!(main[4], Node::Box(_)), "公式盒");
        assert!(matches!(main[5], Node::Penalty { .. }), "postdisplaypenalty 先于下间距");
        assert!(matches!(main[6], Node::Glue { .. }));
        let l3 = as_box(&main[8]);
        assert_eq!(as_char(&l3.children[0]), b'c' as u32, "公式后续文字续排");
        assert_eq!(as_char(&l3.children[1]), b'd' as u32);
    }

    #[test]
    fn math_display_paginated_smoke() {
        // 分页模式：显示公式序列（penalty/glue/box/glue/penalty）正常入页
        let pages = paginated(r"$$\hbox{ab}$$").unwrap();
        assert_eq!(pages.len(), 1, "小公式应单页：{pages:?}");
        // 页面 = [公式盒, belowdisplayskip, postdisplaypenalty]（页首可丢弃节点被丢弃）
        assert!(pages[0].children.iter().any(|n| matches!(n, Node::Box(_))), "页面含公式盒");
    }

    #[test]
    fn math_display_inside_hbox_falls_back_to_inline_math() {
        // $$ 在 \hbox（受限水平模式，mode=-hmode<0）内：tex.web init_math 的
        // `if (cur_cmd=math_shift) and (mode>0)` 不成立 → back_input 放回第二个
        // `$`，按**普通**数学进出，不报任何错。（TeX 无
        // "Display math in restricted mode." 这一错误；trip.log L210 附近为证。）
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset(r"\hbox{$$x$$}");
        let t = ts.take_transcript();
        assert!(
            !t.contains("Display math in restricted mode"),
            "tex.web 无此错误，不应自创：{t}"
        );
        // 不进入显示数学（否则盒子内容会作为独立公式垂直元素出现）
        assert!(
            !t.contains("display math"),
            "受限水平模式不应进入显示数学：{t}"
        );
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
    fn math_caret_outside_math_rejected() {
        // 数学模式外 ^/_（cat 7/8）：TeX "Missing $ inserted" 报错并插入 $ 恢复
        //（参考 trip.log L5330/L5384；恢复式，消息入转录而非 Err）
        assert_math_transcript(r"a^b", "Missing $ inserted");
        assert_math_transcript(r"x_2", "Missing $ inserted");
        assert_math_transcript(r"\hbox{a^b}", "Missing $ inserted");
    }

    #[test]
    fn math_double_superscript_message() {
        assert_math_error(r"$x^2^3$", "双重上标（Double superscript）");
    }

    #[test]
    fn math_missing_base_message() {
        // ^/_ 前无原子：TeX "Missing { inserted" 恢复（参考 trip.log L2851/L5344）
        assert_math_transcript(r"$^2$", "Missing { inserted");
        assert_math_transcript(r"$_2$", "Missing { inserted");
    }

    #[test]
    fn math_field_requires_left_brace_message() {
        // math 原子字段位置的非字符非 { token：TeX scan_math → scan_left_brace 报
        // "Missing { inserted" 放回重扫（恢复式；trip.tex L272/L375/L396）。
        // \\mathord 后随 \\radical（trip.log L2851；`"161` 是 16 进制 delimiter 数字）
        assert_math_transcript(r#"$\mathord\radical"161$"#, "Missing { inserted");
        // ^ 后随 \\leaders（trip.log L5344）；后续内容继续排版
        assert_math_transcript(r"$^\leaders\vrule$", "Missing { inserted");
        // \\accent 报错改道 \\mathaccent 后，nucleus 字段同样要求 {（trip.log
        // L5663 `\accent\x\vfill`：\x 被 15-bit 数字扫描消费，\vfill 处报错）
        assert_math_transcript(r"$\accent 100 \vfill$", "Missing { inserted");
        // 恢复式语义：报错后剩余 token 仍正常产出节点
        let main = typeset(r"$\mathord x$").unwrap();
        assert!(!main.is_empty(), "报错后应继续排版：{main:?}");
    }

    #[test]
    fn math_single_char_field_is_legal() {
        // 单字符是合法 math 字段（tex.web scan_math letter/other_char 分支）：
        // `\mathord x` / `\mathaccent 16 x` / `\sqrt x` 都不报 Missing { inserted
        for src in [r"$\mathord x$", r"$\mathaccent 16 x$", r"$\sqrt x$"] {
            let mut ts = Typesetter::with_metrics(metrics);
            ts.typeset(src).unwrap();
            assert!(
                !ts.take_transcript().contains("Missing {"),
                "合法单字符字段不应报错：{src}"
            );
        }
    }

    #[test]
    fn math_accent_scans_nucleus_field() {
        // `\mathaccent <15-bit> {<field>}`：重音符 + nucleus 字段正常扫描，不报错
        let mut ts = Typesetter::with_metrics(metrics);
        ts.typeset(r"$\mathaccent 7161 {a}{b}$").unwrap();
        assert!(!ts.take_transcript().contains("Missing {"), "合法字段不应报错");
    }

    #[test]
    fn math_left_right_message() {
        // \\right 前缺少 \\left：恢复式错误 "Extra \\right."（TeX 语义，参考
        // trip.log L256 `$\\right\\relax` 双错误恢复；不中断）；\\left 未配对
        // （$ 关数学时）：TeX "Extra } or forgotten \\right." 恢复自动闭合
        assert_math_transcript(r"$\right)$", "Extra \\right.");
        assert_math_transcript(r"$\left(x$", "Extra } or forgotten \\right.");
    }

    #[test]
    fn math_over_ambiguous_message() {
        // TeX 恢复式（参考 trip l.257 同层嵌套 fraction 报 Ambiguous 后继续）：
        // 消息入转录，不中断
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset(r"$a\over b\over c$");
        let t = ts.take_transcript();
        assert!(
            t.contains("Ambiguous; you need another { and }"),
            "转录应含 Ambiguous：{t}"
        );
    }

    #[test]
    fn math_display_end_message() {
        // 显示数学以单 $ 结束：TeX "Display math should end with $$." 恢复
        //（参考 trip.log L1761/L4004；该 $ 按 $$ 处理关闭公式）
        assert_math_transcript(r"$$x$", "Display math should end with $$.");
    }

    #[test]
    fn math_unclosed_message() {
        assert_math_error(r"$x", "数学模式未闭合（缺少 $）");
    }

    #[test]
    fn math_primitive_outside_math_message() {
        // 数学专用原语在文本模式使用 → TeX 报错并恢复（消息入转录，继续执行）
        let mut ts = Typesetter::with_metrics(metrics);
        ts.typeset(r"\over b \sqrt{x}").unwrap();
        let t = ts.take_transcript();
        assert!(t.contains("! You can't use \\over in vertical mode."), "转录应含 over 错误：{t}");
        assert!(t.contains("! You can't use \\sqrt in horizontal mode."), "转录应含 sqrt 错误：{t}");
    }

    #[test]
    fn math_display_requires_double_dollar_end() {
        // $$x$ 的 $ 按 $$ 处理关闭公式（TeX 恢复语义），公式正常产出
        assert_math_transcript(r"$$x$", "Display math should end with $$.");
        let main = typeset(r"$$x$").unwrap();
        assert!(!main.is_empty(), "公式应产出：{main:?}");
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

    #[test]
    fn math_unclosed_formula_rejected() {
        let err = typeset(r"$x").unwrap_err();
        assert!(
            err.to_string().contains("数学模式未闭合"),
            "未闭合公式应报错：{err}"
        );
    }

    #[test]
    fn math_script_without_base_rejected() {
        // ^/_ 前缺原子：TeX "Missing { inserted" 恢复（不终止；参考 trip.log L2851）
        assert_math_transcript(r"$^2$", "Missing { inserted");
        assert_math_transcript(r"$_2$", "Missing { inserted");
    }

    #[test]
    fn math_double_superscript_rejected() {
        assert!(typeset(r"$x^2^3$").is_err(), "双重上标应报错");
    }

    #[test]
    fn math_empty_inline_is_fine() {
        // 空行内公式 $ $ 不产生节点
        let main = typeset(r"a $ $ b").unwrap();
        let line = as_box(&main[0]);
        let chars: Vec<u32> = line
            .children
            .iter()
            .filter_map(|n| match n {
                Node::Char { charcode, .. } => Some(*charcode),
                _ => None,
            })
            .collect();
        assert_eq!(chars, vec![b'a' as u32, b'b' as u32]);
    }

    // ---------- M4-2 数学原语 ----------

    /// 分式盒（`\over`/`\atop`）：vbox = [num 盒, glue, rule?, glue, den 盒]。
    fn fraction_box(text: &str) -> (BoxNode, bool) {
        let children = math_line_children(text);
        assert_eq!(children.len(), 1, "分式应封装为单盒：{text:?}");
        let b = as_box(&children[0]);
        assert_eq!(b.kind, BoxKind::VBox, "分式是垂直堆叠");
        let has_rule = b.children.iter().any(|n| matches!(n, Node::Rule { .. }));
        (b.clone(), has_rule)
    }

    #[test]
    fn math_fraction_over_builds_vbox() {
        let (b, has_rule) = fraction_box(r"$a\over b$");
        assert!(has_rule, "\\over 应画分式线");
        // num 盒含 a、den 盒含 b
        assert_eq!(b.children.len(), 5);
        let num = as_box(&b.children[0]);
        assert_eq!(as_char(&num.children[0]), b'a' as u32);
        let den = as_box(&b.children[4]);
        assert_eq!(as_char(&den.children[0]), b'b' as u32);
    }

    #[test]
    fn math_atop_has_no_rule() {
        let (_, has_rule) = fraction_box(r"$a\atop b$");
        assert!(!has_rule, "\\atop 无线");
    }

    #[test]
    fn math_fraction_in_group() {
        let a = math_line_children(r"${a\over b}$");
        let b = math_line_children(r"$a\over b$");
        assert_eq!(a, b, "组内分式应并入外层");
    }

    #[test]
    fn math_fraction_with_scripts_in_sup() {
        // x^{a\over b}：上标字段内含分式
        let children = math_line_children(r"$x^{a\over b}$");
        assert_eq!(children.len(), 2);
        let sup = as_box(&children[1]);
        assert_eq!(sup.children.len(), 1, "上标字段 = 单个分式盒");
        assert!(matches!(sup.children[0], Node::Box(_)), "上标内应为分式盒");
        let frac = as_box(&sup.children[0]);
        assert_eq!(frac.kind, BoxKind::VBox);
        assert!(frac.children.iter().any(|n| matches!(n, Node::Rule { .. })));
    }

    #[test]
    fn math_sqrt_radical_box() {
        let children = math_line_children(r"$\sqrt{x}$");
        assert_eq!(children.len(), 1);
        let b = as_box(&children[0]);
        assert_eq!(b.kind, BoxKind::VBox);
        // [横线 rule, glue, 内容盒]
        assert!(matches!(b.children[0], Node::Rule { .. }));
        let base = as_box(&b.children[2]);
        assert_eq!(as_char(&base.children[0]), b'x' as u32);
    }

    #[test]
    fn math_sqrt_single_atom() {
        let children = math_line_children(r"$\sqrt x$");
        assert_eq!(children.len(), 1);
        assert_eq!(as_box(&children[0]).kind, BoxKind::VBox);
    }

    #[test]
    fn math_left_right_delimited() {
        let children = math_line_children(r"$\left(x\right)$");
        // \\left...\\right 物化为单个 hbox（tex.web：定界符与内容同盒）
        assert_eq!(children.len(), 1, "\\left(\\right) 应封装为单个 hbox");
        let b = as_box(&children[0]);
        assert_eq!(b.children.len(), 3, "盒内 = 定界符 + body + 定界符");
        assert_eq!(as_char(&b.children[0]), b'(' as u32);
        assert_eq!(as_char(&b.children[1]), b'x' as u32);
        assert_eq!(as_char(&b.children[2]), b')' as u32);
    }

    #[test]
    fn math_left_right_dot_empty_delims() {
        let children = math_line_children(r"$\left.x\right.$");
        // 空定界符不产生字符；\left.\right. 仍封装为单个 hbox（盒内仅 body）
        assert_eq!(children.len(), 1, "\\left.\\right. 封装为单个 hbox");
        let b = as_box(&children[0]);
        assert_eq!(b.children.len(), 1, "盒内仅 body（空定界符无字符）");
        assert_eq!(as_char(&b.children[0]), b'x' as u32);
    }

    #[test]
    fn math_style_affects_scale() {
        let children = math_line_children(r"$x{\scriptstyle y}$");
        assert_eq!(children.len(), 2);
        // y 在 script 样式：宽度 7/10 × (1000+121)
        assert_eq!(children[1].dimensions().width, xn_over_d(1121, 7, 10));
    }

    #[test]
    fn math_bin_class_inserts_medskip() {
        // 默认 `+` 是 Ord → 无间距；\mathbin+ → Bin → 两侧 medmuskip
        // （fn 指针模式 quad=0 → 胶水宽 0，但 Glue 节点仍插入）
        let plain = math_line_all_children(r"$a+b$");
        let bin = math_line_all_children(r"$a\mathbin+b$");
        assert_eq!(plain.len(), 3, "全 Ord 无胶水");
        assert_eq!(bin.len(), 5, "\\mathbin+ 两侧插入 medmuskip 胶水");
        assert!(matches!(bin[1], Node::Glue { .. }));
        assert!(matches!(bin[3], Node::Glue { .. }));
        // \mathbin{+} 组形式等价
        let bin_group = math_line_all_children(r"$a\mathbin{+}b$");
        assert_eq!(bin_group.len(), 5);
        assert!(matches!(bin_group[1], Node::Glue { .. }));
    }

    #[test]
    fn math_rel_class_inserts_thickmuskip() {
        let rel = math_line_all_children(r"$a\mathrel=b$");
        assert_eq!(rel.len(), 5, "ord+rel+ord → 两侧 thickmuskip");
        assert!(matches!(rel[1], Node::Glue { .. }));
        assert!(matches!(rel[3], Node::Glue { .. }));
    }

    /// ETRIP P0 \muexpr 校准：layout 端 muskip_params 按 mu 数值存，
    /// math_to_hlist 内部按当前 style 的 family-2 em/18 转 sp（tex.web
    /// `math_glue`/`mu_mult`：cur_mu = em/18 **整数截断**，再 round(x·cur_mu/65536)）。
    /// 测试用 `with_metrics`（fn 指针模式，font_param 全 0 → math_em fallback
    /// 10pt = 655360 sp → cur_mu = 36408），验证 `\thinmuskip=18mu` 触发 Bin 后
    /// medmuskip 节点 width = 18 × 36408 = 655344 sp。
    /// 截断值有 TinyTeX 实测铁证：plain 下 `\thickmuskip=5mu`（em=10pt）官方 DVI
    /// `E = mc^2` 产物为 `right182040` = 5 × 36408（精确除法得 182044，不匹配）。
    #[test]
    fn math_thinmuskip_em_scaled_in_layout() {
        // \thinmuskip=18mu → muskip_params[0].width = 18 * 65536；
        // 但 Bin 触发的是 medmuskip（idx=1）——为对照, 同时重设 medmuskip=18mu plus 3.6mu
        // 这样 medmuskip 节点 width 与 stretch 都可验证。
        let src = r"\thinmuskip=18mu\medmuskip=18mu plus 3.6mu$a\mathbin+b$";
        let main = typeset(src).unwrap();
        let line = as_box(&main[0]);
        // 行盒 children：mathon, a, Gl_prespacing, mathbin_node, Gl_postspacing, b, mathoff, parfillskip
        // 找 Bin 附近的两个 medmuskip Glue 节点
        let mut inserts = 0;
        let mut found_width = 0i64;
        let mut found_stretch = 0i64;
        for n in &line.children {
            if let Node::Glue {
                name: Some("medmuskip"),
                width,
                stretch,
                stretch_order,
                ..
            } = n
            {
                assert_eq!(*stretch_order, 0, "medmuskip stretch 阶应为 0");
                inserts += 1;
                found_width = *width;
                found_stretch = *stretch;
            }
        }
        assert_eq!(inserts, 2, "Bin 两侧应插 medmuskip 各一：{line:?}");
        // tex.web mu_mult：cur_mu = 655360/18 = 36408（截断）；18mu = 18 × 36408
        // = 655344 sp = 9.99976pt（TeXbook 的 mu 换算本就有截断误差，非精确 10pt）。
        assert_eq!(
            found_width, 655344,
            "medmuskip 实际 sp 应为 tex.web mu_mult:18mu × cur_mu(36408)"
        );
        // 3.6mu plus：mu 存 235930（3.6 × 65536 = 235929.6 四舍五入）
        // → round(235930 × 36408 / 65536) = 131069 sp
        assert_eq!(
            found_stretch, 131069,
            "medmuskip stretch 应按 tex.web mu_mult 转 sp:3.6mu × cur_mu(36408)"
        );
    }

    /// P1（demo1 对照）：行内公式 `E = mc^2` 的 `=` 两侧须插入 thickmuskip，
    /// 宽度按 tex.web `math_glue`/`mu_mult`（cur_mu = em/18 截断）换算。
    /// TinyTeX 实测（plain 格式，cmex10 quad=10pt → cur_mu=36408）：
    /// `=` 两侧 `right219813`（= E 斜体修正 0.57637pt + 5mu）/`right182040`（= 5mu）。
    /// 此前引擎 mu→sp 用精确除法得 182044，且 muskip 寄存器缺省为 0（INITEX 语义）
    /// 导致胶水宽 0、DVI 中完全无间距。
    #[test]
    fn math_thickmuskip_inserted_around_rel_with_texweb_mu_mult() {
        let src = r"\thickmuskip=5mu plus 5mu$a\mathrel=b\mathrel=c$";
        let main = typeset(src).unwrap();
        let line = as_box(&main[0]);
        let mut found = Vec::new();
        for n in &line.children {
            if let Node::Glue {
                name: Some("thickmuskip"),
                width,
                stretch,
                ..
            } = n
            {
                found.push((*width, *stretch));
            }
        }
        assert_eq!(found.len(), 4, "rel 原子两侧各插 thickmuskip：{line:?}");
        // 5mu → 5 × 36408 = 182040（TinyTeX 官方 DVI 实测值）
        assert!(
            found.iter().all(|&(w, s)| w == 182040 && s == 182040),
            "thickmuskip 应为 182040sp（5mu × cur_mu 36408）：{found:?}"
        );
    }

    /// P2（demo1 对照）：上标抬升量按 tex.web make_scripts —— clr 取
    /// family-2（math symbols）字体的 sup1/sup2（fontdimen 13/14），再与
    /// depth(sup 盒)+x_height/4 竞争；nucleus 为单字符时基准 0。
    /// TinyTeX 实测：plain 下 `mc^2` 的 `2` → `down-237825` =
    /// -\sup2（cmex10 fontdimen 14 = 0.362890em × 10pt）。修前用
    /// current_font 的 fontdimen 11（denom1，cmr10 无 → 回退 x_height
    /// 4.30554pt = 282168）——机制错位。
    #[test]
    fn math_sup_shift_uses_mathsy_sup2() {
        let mut ts = Typesetter::with_tfm();
        let src = concat!(
            "\\font\\tenrm=cmr10 \\font\\tenmi=cmmi10 \\font\\tensy=cmsy10 \\font\\tenex=cmex10 ",
            "\\textfont0=\\tenrm \\textfont1=\\tenmi \\textfont2=\\tensy \\textfont3=\\tenex ",
            "\\tenrm $mc^2$"
        );
        let main = ts.typeset(src).unwrap();
        let line = as_box(&main[0]);
        let shifts: Vec<i64> = line
            .children
            .iter()
            .filter_map(|n| match n {
                Node::Box(b) => Some(b.shift),
                _ => None,
            })
            .collect();
        assert_eq!(shifts, vec![-237825], "上标盒 shift 应为 -\\sup2：{line:?}");
    }

    /// P4（demo1 对照）：sub/sup 盒宽须加 \scriptspace（tex.web make_scripts
    /// `width(x):=width(x)+script_space`，盒宽加大但字形不移动）。
    /// TinyTeX 实测铁证：`{1\over n^2}` 分式规则宽 = max(分子,分母盒宽)，
    /// 官方 DVI `putrule w=687373`（分母 n² 盒含 0.5pt scriptspace），
    /// 修前 654605 = 687373-32768。`{π^2\over 6}` 同样 623731 vs 590963。
    #[test]
    fn math_script_box_width_includes_scriptspace() {
        let children = math_line_all_children(r"$x^2$");
        // children：[Char x, Box(sup)]（mathon/mathoff/parfillskip 已剥离）
        let sup = children
            .iter()
            .find_map(|n| match n {
                Node::Box(b) => Some(b),
                _ => None,
            })
            .expect("上标应打包为 hbox");
        // 测试度量：'2' 在 scriptscript 阶宽度缩放为 735；盒宽须再加 \scriptspace=32768
        let inner = match sup.children.last() {
            Some(Node::Char { width, .. }) => *width,
            other => panic!("上标盒内应为字形节点：{other:?}"),
        };
        assert_eq!(
            sup.width,
            inner + 32768,
            "sup 盒宽应含 \\scriptspace：{children:?}"
        );
        // shift（垂直位移）不受 scriptspace 影响：符号在盒内不移动
        assert_eq!(sup.children.len(), 1, "盒内仍只有一个字形节点");
    }

    #[test]
    fn math_over_outside_math_rejected() {
        // TeX 报错并恢复：消息入转录，执行继续
        let mut ts = Typesetter::with_metrics(metrics);
        ts.typeset(r"a\over b").unwrap();
        let t = ts.take_transcript();
        assert!(t.contains("! You can't use \\over in"), "转录应含 over 模式错误：{t}");
    }

    #[test]
    fn math_over_ambiguous_rejected() {
        // TeX 恢复式（参考 trip l.257）：连续 \over 报 Ambiguous 后继续，
        // 不中断（原实现硬错误 Err，已按参考改为恢复）
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset(r"$a\over b\over c$");
        let t = ts.take_transcript();
        assert!(
            t.contains("Ambiguous; you need another { and }"),
            "转录应含 Ambiguous：{t}"
        );
    }

    #[test]
    fn math_left_without_right_rejected() {
        assert!(typeset(r"$\left(x$").is_err(), "\\left 必须配 \\right");
    }

    #[test]
    fn math_right_without_left_rejected() {
        // TeX 语义：\\right 前无 \\left → 恢复式 "Extra \\right."（不中断，
        // 参考 trip.log L256；旧实现是硬错误，已按参考改为恢复）
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset(r"$x\right)$");
        let t = ts.take_transcript();
        assert!(
            t.contains("Extra \\right."),
            "\\right 前无 \\left 应报 Extra \\right：{t}"
        );
    }

    #[test]
    fn math_over_empty_denominator_is_fine() {
        // TeX 允许空分母：$a\over$ → 分式盒（只有分子）
        let (_, has_rule) = fraction_box(r"$a\over$");
        assert!(has_rule);
    }

    // ---------- M4-3 数学字体族 + fontdimen ----------

    #[test]
    fn math_textfont_family_uses_family_font() {
        let Some(fm) = cmr10_metrics() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        // \textfont1=\twelve（12pt）→ $x$ 的 x 走 fam 1（字母 mathcode x+"7100"），
        // 用族 1 的 12pt 字体度量（修正后语义：字母默认 cmmi 斜体族）
        let mut ts = Typesetter::with_tfm();
        let main = ts
            .typeset(r"\font\tenrm=cmr10\font\twelve=cmr10 at 12pt\textfont1=\twelve\tenrm $x$")
            .unwrap();
        let line = as_box(&main[0]);
        // 行盒含 \mathon/\mathoff 边界节点（tex.web math_node）与行尾 \parfillskip：
        // 过滤后 children[0] 即公式首原子（惯例同 math_line_children）
        let x = line
            .children
            .iter()
            .find(|n| matches!(n, Node::Char { charcode: c, .. } if *c == b'x' as u32))
            .expect("公式内应有字符 x");
        let (w10, _, _) = fm.char_metrics(b'x' as u32);
        let w12 = xn_over_d(w10, 12 * SP_PER_PT, 10 * SP_PER_PT);
        assert_eq!(x.dimensions().width, w12, "族 1 字体应为 12pt cmr10");
    }

    #[test]
    fn math_sup_rise_uses_fontdimen_sup1() {
        let Some(fm) = cmr10_metrics() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        // cmr10 的 sup1 = 参数 11（font_params[10]）；上标提升量应取该值
        let sup1 = fm.font_params.get(10).copied().unwrap_or(0);
        if sup1 == 0 {
            eprintln!("cmr10 无 sup1 参数，跳过");
            return;
        }
        let mut ts = Typesetter::with_tfm();
        let main = ts.typeset(r"\font\tenrm=cmr10\tenrm $x^2$").unwrap();
        let line = as_box(&main[0]);
        let sup = as_box(&line.children[1]);
        assert_eq!(sup.shift, -sup1, "上标提升量应为 fontdimen sup1");
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

    /// 带窄词间胶水（3000/100000/500）的排版器：断字用例需要可拉伸词间空白。
    fn typeset_hyphen(src: &str) -> Result<Vec<Node>> {
        let mut ts = Typesetter::with_metrics(metrics).with_space(|_| Glue::new(3000, 100000, 500));
        ts.typeset(src)
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
        // 无 \patterns：词内无断点 → 整段一条过满行（末尾强制断点），不断字
        let main = typeset_spaced(r"\hsize 10700sp m abcdefgh n").unwrap();
        assert_eq!(main.len(), 1, "无模式表：整段一条过满行（不断字）：{main:?}");
        let l1 = as_box(&main[0]);
        // [m, glue, a..h, glue, n, \parfillskip]（13 节点，字母连续）
        assert_eq!(l1.children.len(), 13, "行1 = m abcdefgh n：{l1:?}");
        assert_eq!(as_char(&l1.children[0]), b'm' as u32);
        for (k, ch) in (b'a'..=b'h').enumerate() {
            assert_eq!(as_char(&l1.children[k + 2]), ch as u32, "字母不断字");
        }
        assert_eq!(as_char(&l1.children[11]), b'n' as u32);
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

    // ---- 数学原语内容保留（KNOWN-SIMPLIFICATIONS §1 存在性测试）----

    #[test]
    fn math_underline_keeps_content() {
        // tex.web math_ac：\underline 是 Under 原子——内容必须保留在数学列表。
        // 此前 scan_group_contents(None) 收集即丢（primitive.rs:860）。
        let main = typeset(r"\hbox{$\underline{A}$}").unwrap();
        let mut chars = Vec::new();
        collect_chars(&main, &mut chars);
        assert!(
            chars.contains(&(b'A' as u32)),
            "underline 内容 A 丢失: {chars:?}"
        );
    }

    #[test]
    fn math_overline_keeps_content() {
        let main = typeset(r"\hbox{$\overline{B}$}").unwrap();
        let mut chars = Vec::new();
        collect_chars(&main, &mut chars);
        assert!(
            chars.contains(&(b'B' as u32)),
            "overline 内容 B 丢失: {chars:?}"
        );
    }

    #[test]
    fn math_penalty_kept_in_formula() {
        // tex.web：数学模式 \penalty 是断行点——必须出现在公式输出（M4-1；
        // 此前 sink 数学模式直接忽略）。"2D = '-' 字符必须出现在渲染输出。此前 scan_number 即丢。
        let main = typeset(r"\hbox{$\penalty-50 x$}").unwrap();
        let mut has_penalty = false;
        fn walk(ns: &[Node], found: &mut bool) {
            for n in ns {
                if matches!(n, Node::Penalty { penalty: -50 } if !*found) {
                    *found = true;
                }
                if let Node::Box(b) = n {
                    walk(&b.children, found);
                }
            }
        }
        walk(&main, &mut has_penalty);
        assert!(has_penalty, "数学内 \\penalty-50 丢失：{main:?}");
    }

    #[test]
    fn math_vrule_kept_in_formula() {
        // tex.web：数学模式 \vrule 是规则原子（M4-1；此前忽略）
        let main = typeset(r"\hbox{$\vrule width 5pt x$}").unwrap();
        let mut has_rule = false;
        fn walk2(ns: &[Node], found: &mut bool) {
            for n in ns {
                if matches!(n, Node::Rule { .. }) {
                    *found = true;
                }
                if let Node::Box(b) = n {
                    walk2(&b.children, found);
                }
            }
        }
        walk2(&main, &mut has_rule);
        assert!(has_rule, "数学内 \\vrule 丢失：{main:?}");
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
    // (10000)。latex.ltx 罚分协议六档（docs/output-routine-survey.md §2.2bis）
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

    /// NTex vpack 的占位简化（node.rs vpack）：打包时剥前导 discardable 节点。
    /// `\vbox{\unvbox255}` 的对照基准须先剥默认页的前导 glue 再比较。
    fn strip_leading_discardables(children: &[Node]) -> &[Node] {
        let n = children.iter().take_while(|c| c.is_discardable()).count();
        &children[n..]
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

}
