#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::BoxKind;
    use ntex_core::SP_PER_PT;
    use ntex_core::TokenSink;

    /// TRIP 冲刺调试（临时）：trip.tex 前段逐行二分。
    #[test]
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
        assert_eq!(v.children.len(), 3);
        assert!(matches!(v.children[0], Node::Box(_)));
        match &v.children[1] {
            Node::Glue { width, .. } => assert_eq!(*width, 10 * SP_PER_PT),
            other => panic!("预期 Glue，得到 {other:?}"),
        }
        assert!(matches!(v.children[2], Node::Box(_)));
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
                width,
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
                width,
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

    /// 段落行盒 children（`$...$` 触发段落 → 主列表单行 hbox）。
    fn math_line_children(text: &str) -> Vec<Node> {
        let main = typeset(text).unwrap();
        assert_eq!(main.len(), 1, "数学公式应封装为单行段落：{text:?}");
        let line = as_box(&main[0]);
        // 去掉行尾 \parfillskip 胶水
        line.children
            .iter()
            .filter(|n| !matches!(n, Node::Glue { .. }))
            .cloned()
            .collect()
    }

    /// 段落行盒全部 children（含 spacing 胶水，排除行尾 \parfillskip；spacing 测试用）。
    fn math_line_all_children(text: &str) -> Vec<Node> {
        let main = typeset(text).unwrap();
        assert_eq!(main.len(), 1, "数学公式应封装为单行段落：{text:?}");
        let mut children = as_box(&main[0]).children.clone();
        // 去掉行尾 \parfillskip（0pt plus 1fil；hpack 拉伸后宽度非 0，按 fil 阶识别）
        while let Some(Node::Glue {
            stretch,
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
        // fn 指针模式 x_height=0 → 上标提升量 0（M4-3 用 fontdimen 精化）
        assert_eq!(sup.shift, 0);
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
        assert_eq!(zw, xn_over_d(1000 + b'z' as i64, 5, 10));
    }

    #[test]
    fn math_display_formula() {
        // $$x$$（垂直模式，无前驱）：predisplaypenalty + abovedisplayskip +
        // 居中公式盒 + belowdisplayskip + postdisplaypenalty
        let main = typeset(r"$$x$$").unwrap();
        assert_eq!(main.len(), 5, "显示公式 = 前后 penalty + 上下间距 + 公式盒：{main:?}");
        match &main[0] {
            Node::Penalty { penalty } => assert_eq!(*penalty, 10_000, "predisplaypenalty 默认 10000"),
            other => panic!("预期 predisplaypenalty，得到 {other:?}"),
        }
        match &main[1] {
            Node::Glue { width, stretch, shrink, .. } => {
                assert_eq!(*width, 12 * SP_PER_PT, "abovedisplayskip");
                assert_eq!(*stretch, 3 * SP_PER_PT);
                assert_eq!(*shrink, 9 * SP_PER_PT);
            }
            other => panic!("预期 abovedisplayskip，得到 {other:?}"),
        }
        let boxed = as_box(&main[2]);
        // 公式盒 = \hbox to \hsize 居中（两侧 \hfil），中为 x
        assert_eq!(boxed.width, 13 * 4_736_286 / 2, "公式盒宽 = \\hsize");
        assert_eq!(boxed.children.len(), 3, "hfil + x + hfil");
        assert_eq!(as_char(&boxed.children[1]), b'x' as u32);
        match &main[3] {
            Node::Glue { width, .. } => assert_eq!(*width, 12 * SP_PER_PT, "belowdisplayskip"),
            other => panic!("预期 belowdisplayskip，得到 {other:?}"),
        }
        match &main[4] {
            Node::Penalty { penalty } => assert_eq!(*penalty, 0, "postdisplaypenalty 默认 0"),
            other => panic!("预期 postdisplaypenalty，得到 {other:?}"),
        }
    }

    #[test]
    fn math_display_short_skip_after_short_line() {
        // 段中 $$：前段末行自然宽度 < \hsize → 短间距
        // （abovedisplayshortskip = 0pt plus 3pt、belowdisplayshortskip = 7pt plus 3pt minus 4pt）
        let main = typeset(r"\hsize 10000sp a $$x$$").unwrap();
        // [行(a), penalty, above-glue, 公式盒, below-glue, penalty]
        assert_eq!(main.len(), 6, "段中短行公式：{main:?}");
        match &main[2] {
            Node::Glue { width, stretch, shrink, .. } => {
                assert_eq!(*width, 0, "abovedisplayshortskip 宽 0");
                assert_eq!(*stretch, 3 * SP_PER_PT);
                assert_eq!(*shrink, 0);
            }
            other => panic!("预期 abovedisplayshortskip，得到 {other:?}"),
        }
        match &main[4] {
            Node::Glue { width, .. } => assert_eq!(*width, 7 * SP_PER_PT, "belowdisplayshortskip"),
            other => panic!("预期 belowdisplayshortskip，得到 {other:?}"),
        }
    }

    #[test]
    fn math_display_long_skip_after_full_line() {
        // 段中 $$：前段末行自然宽度 ≥ \hsize（过满）→ 长间距（abovedisplayskip）
        let main = typeset(r"\hsize 500sp a $$x$$").unwrap();
        assert_eq!(main.len(), 6, "段中满行公式：{main:?}");
        match &main[2] {
            Node::Glue { width, .. } => assert_eq!(*width, 12 * SP_PER_PT, "abovedisplayskip"),
            other => panic!("预期 abovedisplayskip，得到 {other:?}"),
        }
    }

    #[test]
    fn math_display_skips_configurable() {
        // \abovedisplayskip/\belowdisplayskip 可赋值（无 = 形式，同现有测试风格）
        let main = typeset(r"\abovedisplayskip 5pt\belowdisplayskip 3pt$$x$$").unwrap();
        match &main[1] {
            Node::Glue { width, .. } => assert_eq!(*width, 5 * SP_PER_PT),
            other => panic!("abovedisplayskip 应生效：{other:?}"),
        }
        match &main[3] {
            Node::Glue { width, .. } => assert_eq!(*width, 3 * SP_PER_PT),
            other => panic!("belowdisplayskip 应生效：{other:?}"),
        }
    }

    #[test]
    fn math_display_paragraph_continues_after_formula() {
        // 段中公式：ab $$x$$ cd → 行(ab) + 公式垂直元素 + 行(cd)，续排无 parskip/缩进
        let main = typeset(r"ab $$x$$ cd").unwrap();
        assert_eq!(main.len(), 7, "公式前后文字各成行：{main:?}");
        let l1 = as_box(&main[0]);
        assert_eq!(as_char(&l1.children[0]), b'a' as u32);
        assert!(matches!(main[1], Node::Penalty { .. }));
        assert!(matches!(main[2], Node::Glue { .. }));
        assert!(matches!(main[3], Node::Box(_)), "公式盒");
        assert!(matches!(main[4], Node::Glue { .. }));
        assert!(matches!(main[5], Node::Penalty { .. }));
        let l3 = as_box(&main[6]);
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
        // \right 前缺少 \left：Err（math_right 检查）；\left 未配对（$ 关数学时）：
        // TeX "Extra } or forgotten \right." 恢复自动闭合（参考 trip.log L299 附近）
        assert_math_error(r"$\right)$", "\\right 前缺少 \\left（Missing \\left inserted）");
        assert_math_transcript(r"$\left(x$", "Extra } or forgotten \\right.");
    }

    #[test]
    fn math_over_ambiguous_message() {
        assert_math_error(r"$a\over b\over c$", "\\over 歧义（Ambiguous; you need another { and }）");
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
        // 定界符 + body + 定界符
        assert_eq!(children.len(), 3);
        assert_eq!(as_char(&children[0]), b'(' as u32);
        assert_eq!(as_char(&children[1]), b'x' as u32);
        assert_eq!(as_char(&children[2]), b')' as u32);
    }

    #[test]
    fn math_left_right_dot_empty_delims() {
        let children = math_line_children(r"$\left.x\right.$");
        assert_eq!(children.len(), 1, "空定界符不产生字符");
        assert_eq!(as_char(&children[0]), b'x' as u32);
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
        assert!(typeset(r"$a\over b\over c$").is_err(), "连续 \\over 应歧义报错");
    }

    #[test]
    fn math_left_without_right_rejected() {
        assert!(typeset(r"$\left(x$").is_err(), "\\left 必须配 \\right");
    }

    #[test]
    fn math_right_without_left_rejected() {
        assert!(typeset(r"$x\right)$").is_err(), "\\right 前必须有 \\left");
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
        // \textfont0=\twelve（12pt）→ $x$ 的 x 用族 0 的 12pt 字体度量
        let mut ts = Typesetter::with_tfm();
        let main = ts
            .typeset(r"\font\tenrm=cmr10\font\twelve=cmr10 at 12pt\textfont0=\twelve\tenrm $x$")
            .unwrap();
        let line = as_box(&main[0]);
        let x = &line.children[0];
        let (w10, _, _) = fm.char_metrics(b'x' as u32);
        let w12 = xn_over_d(w10, 12 * SP_PER_PT, 10 * SP_PER_PT);
        assert_eq!(x.dimensions().width, w12, "族 0 字体应为 12pt cmr10");
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
}
