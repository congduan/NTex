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
        let b = as_box(&children[0]);
        assert_eq!(b.kind, BoxKind::VBox, "分式是垂直堆叠");
        let has_rule = b.children.iter().any(|n| matches!(n, Node::Rule { .. }));
        (b.clone(), has_rule)
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
    // ── 按域拆分的测试子模块（R2 纯移动，helper 留主文件走 super::* 链） ──
    mod tests_box { include!("tests_box.rs"); }
    mod tests_page { include!("tests_page.rs"); }
    mod tests_insert { include!("tests_insert.rs"); }
    mod tests_math { include!("tests_math.rs"); }
    mod tests_align { include!("tests_align.rs"); }
}
