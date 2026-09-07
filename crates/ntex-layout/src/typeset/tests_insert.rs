use super::*;

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
