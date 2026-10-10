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

    /// 脚注类 insert 参与断页计账：`\box255` 的正文页高要扣掉
    /// `\skip<class>` 与 insert 体自然高，输出例程随后可 `\unvbox<class>`
    /// 把脚注排到页底。
    #[test]
    fn insert_footnote_reaches_page_bottom_and_reserves_space() {
        let mut ts = Typesetter::with_metrics(metrics).with_space(|_| Glue::new(1000, 500, 300));
        let (pages, _) = ts
            .typeset_dvi(
                r"\count150=1000 \dimen150=50pt \skip150=12pt
                  \vsize=100pt \hsize=200pt
                  \output={\dimen0=\ht255 \showthe\dimen0
                    \shipout\vbox to\vsize{\unvbox255\vfil
                      \ifvoid150\else\vskip\skip150\hrule\unvbox150\fi}}
                  \hbox{BODY}\insert150{\hbox{FN}}\vfill\penalty-10000\end",
            )
            .unwrap();
        assert_eq!(pages.len(), 1, "应输出单页：{pages:?}");
        let t = ts.take_transcript();
        let ht255 = showthe_values(&t, "dimen");
        assert_eq!(ht255.len(), 1, "输出例程应展示一次 box255 高度：{t:?}");
        assert_ne!(
            ht255[0], "100.0pt",
            "box255 不能占满 vsize，否则脚注没有参与断页让位：{t:?}"
        );
        let mut text = String::new();
        collect_text(&pages[0], &mut text);
        assert!(text.contains("BODY"), "正文应在输出页：{text:?}");
        assert!(text.contains("FN"), "脚注体应由输出例程回流到页底：{text:?}");
    }

    // ---------- 刀 B：长脚注 split/holdover 前置——plain \vfootnote 通路 ------
    //
    // lf3.tex（长脚注 2 页样张）在 NTex 全军覆没的真根因不在 page.rs 拆分臂
    // （该探针 insert 高 < 剩余页高，GT 也不走 split），而在脚注宏机制两处
    // 扫描缺陷：① `\footstrut`=`\vbox to\splittopskip{}` 双报 Missing number
    // （胶参数在尺寸上下文无 glue_val→width 降级臂）；② `\fo@t` 的
    // `\ifcat\bgroup\noexpand\next` 误判真 → 误入 `\f@@t` 分支 → `\@foot`
    // 的 `\egroup` 永不执行 → 组泄漏、整篇无页。两测去修复必红。

    /// 根因①：胶水内部参数在**尺寸**上下文按 tex.web
    /// scan_something_internal 的 `while cur_val_level>level` 转换臂取宽度分量，
    /// 不报 Missing number（plain.tex `\footstrut` 即 `\vbox to\splittopskip{}`）。
    #[test]
    fn glue_param_in_dimen_context_takes_width() {
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset_dvi(
            r"\splittopskip=7pt \setbox0=\vbox to\splittopskip{\hbox{X}}\ifdim\ht0>6.9pt\message{HTOK}\else\message{HTBAD}\fi\end",
        );
        let t = ts.take_transcript();
        assert!(
            !t.contains("Missing number"),
            "胶参数在尺寸上下文应取宽度分量而非报 Missing number：{t:?}"
        );
        assert!(t.contains("HTOK"), "\\vbox to\\splittopskip 应取 7pt 为高：{t:?}");
        assert!(!t.contains("HTBAD"), "高度不应落 0：{t:?}");
    }

    /// 根因②：`\let` 到字符的 cs 在 `\if`/`\ifcat` 操作数位按 tex.web
    /// get_x_token 呈现其字符含义（cur_cmd/cur_chr 即 eq_type/equiv）——
    /// `\ifcat\bgroup\noexpand\next`（\next=字符 Z）判假（cat1 vs 非字符哨兵）、
    /// `\if\bgroup{` 判真。此前一刀切非字符哨兵 → `\ifcat` 恒真 → 误支。
    #[test]
    fn let_to_char_cs_is_a_character_in_if_operands() {
        let mut ts = Typesetter::with_metrics(metrics);
        let _ = ts.typeset_dvi(
            r"\let\bgroup={ \let\next=Z\ifcat\bgroup\noexpand\next\message{CATTRUE}\else\message{CATFALSE}\fi \let\bg={ \if\bg{\message{CHREQ}\fi\end",
        );
        let t = ts.take_transcript();
        assert!(
            t.contains("CATFALSE"),
            "cat1（\\bgroup）vs noexpand 冻结非字符哨兵应判假：{t:?}"
        );
        assert!(!t.contains("CATTRUE"), "不应误支：{t:?}");
        assert!(t.contains("CHREQ"), "\\if 对 let-to-char 与字面同字符应判真：{t:?}");
    }

    /// plain `\vfootnote` 骨架端到端：`\insert\bgroup …\futurelet\next\fo@t`
    /// 分派脚注文本首 token、`\@foot` 闭合 insert 组，脚注体经断页投进
    /// box(class)、出页不中断（组泄漏 = 无页）。去两修复之一必红
    /// （① 载体 `\vbox to\splittopskip` 的 \footstrut 同款未在此复刻，
    /// 该测锁的是②的组闭合链 + insert 落页）。
    #[test]
    fn vfootnote_dispatch_closes_insert_group_and_ships_page() {
        let mut ts = Typesetter::with_metrics(metrics);
        let (pages, _) = ts
            .typeset_dvi(
                r"\catcode`\@=11 \let\bgroup={ \let\egroup=} \def\myfoot{\egroup}%
                  \def\f@t#1{#1\myfoot}%
                  \def\fo@t{\ifcat\bgroup\noexpand\next \let\next\BADBRANCH\else\let\next\f@t\fi \next}%
                  \vsize=100pt \hsize=200pt
                  \hbox{BODY}\insert254\bgroup FN\futurelet\next\fo@t Ztail\par
                  \vfill\penalty-10000 \ifvoid254\message{VOID}\else\message{FULL}\fi\end",
            )
            .unwrap();
        let t = ts.take_transcript();
        assert!(
            !t.contains("Undefined control sequence"),
            "\\fo@t 应走 \\f@t 臂（\\BADBRANCH 不该被执行）：{t:?}"
        );
        assert_eq!(pages.len(), 1, "组应闭合、断页应出页：{pages:?}");
        assert!(t.contains("FULL"), "组闭合后脚注体应投进 box254：{t:?}");
        assert!(!t.contains("VOID"), "不应走 void 臂：{t:?}");
    }
