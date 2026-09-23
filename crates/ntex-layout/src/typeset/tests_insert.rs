use super::*;

    // ---------- 输出例程刀 3（G3a）：insert 结构化——token 体保留 ----------

    /// 真 TeX 探针 1（/tmp/knife3/p1.tex，三参数赋值移到体外——体内的赋值要
    /// 等体在扫描位执行才生效，见 survey §5.bis.4 发现未修 1）：
    /// `\splittopskip=10pt plus2fil \splitmaxdepth=1pt \floatingpenalty=200
    /// \setbox0=\vbox{\insert150{\hbox{FN}}}\showbox0` 的 ins 节点行。
    /// 体按 tex.web `begin_insert_or_adjust` 语义在体内垂直模式**排版**（P0 脚注
    /// 刀）：ins 节点带排好版的 vlist（tex.web `ins_ptr`），showbox 递归出体子树，
    /// natural size = 体自然尺寸（真 TeX 同探针形态：`\insert150, natural size
    /// 6.83331; …` + `.\hbox(...)` 子树；此处 dummy 字体小一个量级）。
    #[test]
    fn insert_body_typeset_and_split_params_preserved() {
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset_dvi(
            r"\splittopskip=10pt plus2fil \splitmaxdepth=1pt \floatingpenalty=200 \setbox0=\vbox{\insert150{\hbox{FN}}}\showbox0\end",
        );
        let t = ts.take_transcript();
        assert!(
            t.contains(
                r"\insert150, natural size 0.11444; split(10.0 plus 2.0fil,1.0); float cost 200"
            ),
            "ins 节点行应按 tex.web show_node 格式带三参数与体自然尺寸：{t:?}"
        );
        // 体是排版结果（行盒 + 字符子树），不再是 token 串
        assert!(t.contains("\\ F"), "体字符 F 应以排版节点出现：{t:?}");
        assert!(t.contains("\\ N"), "体字符 N 应以排版节点出现：{t:?}");
        assert!(
            !t.contains("\\cs"),
            "体不再是 token 串（旧实现以 \\cs 占位显示）：{t:?}"
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

    /// 段中 `\insert`（LaTeX `\footnote` 的真形态）：体在段内排版后，ins 节点
    /// 不进行盒——tex.web line_break 以 adjust_tail 把 ins/mark/adjust 摘出行盒、
    /// 接到行盒之后的竖列表（L12901），fire_up 才投得进 box(class)。
    /// 体留在行盒里时 box150 恒 void（= P0「脚注文本整段消失」）。
    #[test]
    fn insert_inside_paragraph_reaches_register() {
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset_dvi(
            r"text\insert150{\hbox{A}}tail\par\vfill\penalty-10000 \ifvoid150\message{VOID}\else\message{FULL}\fi\end",
        );
        let t = ts.take_transcript();
        assert!(
            t.contains("FULL"),
            "段内 \\insert 的体应经断页投进 box150：{t:?}"
        );
    }

    /// `\unvbox150` 取走累积盒，`\ifvoid150` 复归 void——真 TeX 探针 2 的
    /// `\setbox3=\vbox{\unvbox150}\showbox3`：box3 装的是体 vlist 本身
    /// （fire_up 的 `vpackage(ins_ptr(p))`，卸盒即体内容、无 ins 节点）、150 void。
    #[test]
    fn insert_unvbox_roundtrip_empties_register() {
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset_dvi(
            r"\hbox{X}\insert150{\hbox{A}}\vfill\penalty-10000 \setbox3=\vbox{\unvbox150}\showbox3\ifvoid150\message{VOID}\else\message{FULL}\fi\end",
        );
        let t = ts.take_transcript();
        assert!(
            t.contains("\\ A"),
            "回流内容应是体 vlist（tex.web vpackage(ins_ptr)）进 box3 的盒树：{t:?}"
        );
        assert!(
            !t.contains("\\insert150"),
            "卸盒语义：box3 装体内容，不再有 ins 节点：{t:?}"
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
