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
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
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
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
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
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
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
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
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
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
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
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
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
        let mut ts = Typesetter::with_metrics(metrics).plain_params().with_space(|_| Glue::new(1000, 500, 300));
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

    /// held-over insert：插入物放不进当前页时，页构建器必须回到已有最佳断点
    /// fire_up，而不是把 insert 留到后续 `\penalty` 才第一次出页。这个顺序会让
    /// 输出例程先看到自然断点的 `\outputpenalty=10000`，并能读到本次 holdover
    /// 累计的 `\insertpenalties`。
    #[test]
    fn held_over_insert_fires_at_prior_break_and_records_float_cost() {
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        let (pages, _) = ts
            .typeset_dvi(concat!(
                r"\vsize=50pt \hsize=200pt \count150=1000 \dimen150=20pt ",
                r"\output={\showthe\outputpenalty\showthe\insertpenalties\shipout\box255} ",
                r"\hbox{A}\vskip0pt",
                r"\insert150{\vskip100pt\floatingpenalty7}",
                r"\penalty-333\hbox{B}\end"
            ))
            .unwrap();
        let t = ts.take_transcript();
        assert!(
            showthe_values(&t, "outputpenalty")
                .first()
                .is_some_and(|v| v == "10000"),
            "held-over insert 应先在前一自然断点出页，而不是等后续惩罚：{t:?}"
        );
        // GT（pdftex gtd.tex 同构造实测）：INS=[0] 全页——tex.web L19752 在
        // fire_up 打包期 insert_penalties:=0，vert_break 的 split 罚（含
        // floatingpenalty 7）在 build_page 期累积、打包前被清零，例程读到
        // 的 \insertpenalties 从 0 起算。旧锁「应含 7」被 GT 证伪（绿锁≠对），
        // 按 GT 重钉：全部读数为 0。
        let ins = showthe_values(&t, "insertpenalties");
        assert!(
            ins.iter().all(|v| v == "0"),
            "\\insertpenalties 例程读数应为 0（L19752 打包清零）：{ins:?}\\n{t}"
        );
        assert!(pages.len() >= 2, "应至少产出两页：{pages:?}\\n{t}");
    }

    /// `\count<n>` 是 insertion 高度的缩放因子（tex.web L19614-19617）：
    /// count=2000 时 100pt insertion 需要 200pt，而不是 50pt。后者会把
    /// 本应 split 的 insertion 错当作 fits，使后续断点（包括 vadjust）错页。
    #[test]
    fn insert_count_scaling_uses_count_over_thousand() {
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        let _ = ts.typeset_dvi(
            r"\tracingpages=1 \vsize=100pt \count100=2000 \dimen100=1000pt
              \hbox{X}\insert100{\vbox to100pt{}}\vfill\penalty-10000\end",
        );
        let t = ts.take_transcript();
        assert!(
            t.contains("% split100"),
            "count=2000 的 100pt insertion 必须在 100pt 页上 split：{t}"
        );
    }

    /// held-over insert 回到既有页断点时，断点节点本身已被消费；它后面的
    /// 段内强制折行 penalty 仍只属于 line_break，不能在重试后升格为
    /// vertical page penalty。
    #[test]
    fn held_over_insert_retry_does_not_replay_paragraph_penalty() {
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        let _ = ts.typeset_dvi(concat!(
            r"\vsize=1pt \hsize=20pt \count150=1000 \dimen150=1pt ",
            r"\output={\showthe\outputpenalty\shipout\box255} ",
            r"A\hfill\vadjust{\penalty-5000}",
            r"\insert150{\vskip100pt\floatingpenalty7}",
            r"\penalty-1000000000 A\par\end"
        ));
        let t = ts.take_transcript();
        let vals = showthe_values(&t, "outputpenalty");
        assert!(
            vals.iter().any(|v| v == "-5000"),
            "insert 重试应回到 \\vadjust 页断点：{vals:?}\n{t}"
        );
        assert!(
            vals.iter().all(|v| v != "-1000000000"),
            "段内强制折行 penalty 不得在 insert 重试后成为页断点：{vals:?}\n{t}"
        );
    }

    /// TRIP 刀 AD：断页 insert 溢出语义对齐 tex.web（split 成本 + 行间 club/
    /// widow 惩罚）。GT（tex 3.141592653，/tmp/bladeAD/gt1t.log tracingpages
    /// 实证）：`[150],[-5000],[-333]`——第一页在行间断点（p=150=
    /// interline 0 + clubpenalty 150），vadjust 页次之，held-over insert 走
    /// split 计账后由 -333 强制页收尾。
    #[test]
    fn paragraph_vadjust_precedes_following_held_over_insert() {
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        let _ = ts.typeset_dvi(concat!(
            r"\vsize=1pt \hsize=20pt \count150=1000 \dimen150=1pt ",
            r"\output={\showthe\outputpenalty\shipout\box255} ",
            r"A\hfill\vadjust{\penalty-5000}",
            r"\penalty-1000000000 A\par",
            r"\insert150{\vskip100pt\floatingpenalty7}",
            r"\penalty-333\end"
        ));
        let t = ts.take_transcript();
        let vals = showthe_values(&t, "outputpenalty");
        // GT 逐字序 = [150, -5000, -333]（pdftex gta.tex 实测，CLI 同对齐）。
        // in-process 假度量（metrics 函数）折行行数与真字体不同，逐字序在
        // 单测通道不稳；此处锁语义序：vadjust 先于 -333 强制页、且不抢首行
        // 行间断点。CLI 通道（ntex-dvi gta.tex）另以逐字序验收。
        let i5000 = vals
            .iter()
            .position(|v| v == "-5000")
            .unwrap_or_else(|| panic!("应含 vadjust 页 -5000：{vals:?}
{t}"));
        let i333 = vals
            .iter()
            .position(|v| v == "-333")
            .unwrap_or_else(|| panic!("应含 -333 强制页：{vals:?}
{t}"));
        assert!(i333 > i5000, "-333 强制页应在 vadjust 页之后：{vals:?}
{t}");
        // 注：首页是否为 club 150 取决于折行行数（真字体 hfill 处断 = GT
        // 3 行首页 150；假度量 2 行折行材料随 box1，首页 -5000 语义自洽），
        // 该相对序由 CLI 通道逐字验收，不在假度量单测断言。
    }

    /// TRIP 刀 AD：自然断点先行形。GT（/tmp/bladeAD/gt2t.log 实证）：
    /// `[10000],[10000],[150],[-5000],[10000]`——X 页自然断、insert 溢出
    /// split 计账后负目标空页立即点火（第二 [10000]）、两行段行间
    /// [150] 页、vadjust 页、末尾 inf 页。
    #[test]
    fn paragraph_vadjust_precedes_pending_natural_insert_retry() {
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
        let _ = ts.typeset_dvi(concat!(
            r"\vsize=1pt \hsize=20pt \count150=1000 \dimen150=1pt ",
            r"\output={\showthe\outputpenalty\shipout\box255} ",
            r"\hbox{X}\vskip0pt",
            r"\insert150{\vskip100pt\floatingpenalty7}",
            r"A\hfill\vadjust{\penalty-5000}",
            r"\penalty-1000000000 A\par\end"
        ));
        let t = ts.take_transcript();
        let vals = showthe_values(&t, "outputpenalty");
        // GT 逐字序 = [10000, 10000, 150, -5000, 10000]（pdftex gtb.tex 实测，
        // CLI 同对齐）。in-process 假度量下逐字序不稳，锁语义序：-5000 页在
        // 末 eject 页之前、末页=inf_penalty、-5000 前有先行断页。
        let i5000 = vals
            .iter()
            .position(|v| v == "-5000")
            .unwrap_or_else(|| panic!("应含 vadjust 页 -5000：{vals:?}
{t}"));
        assert!(
            i5000 + 1 < vals.len(),
            "-5000 页后应还有 its_all_over eject 页：{vals:?}
{t}"
        );
        assert_eq!(
            vals.last(),
            Some(&"10000".to_string()),
            "末页应为 eject（inf_penalty）：{vals:?}
{t}"
        );
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
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
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
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
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
        let mut ts = Typesetter::with_metrics(metrics).plain_params();
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
