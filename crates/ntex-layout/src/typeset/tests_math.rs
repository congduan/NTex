use super::*;

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
        // tex.web make_scripts：sub+sup 同挂一个 vpack 组合盒（sup 盒 + kern + sub 盒）
        let children = math_line_children(r"$x_1^2$");
        assert_eq!(children.len(), 2, "x + 组合盒");
        assert_eq!(as_char(&children[0]), b'x' as u32);
        let combo = as_box(&children[1]);
        assert_eq!(combo.kind, BoxKind::VBox);
        assert_eq!(combo.children.len(), 3, "sup 盒 + kern + sub 盒");
        assert_eq!(as_char(&as_box(&combo.children[0]).children[0]), b'2' as u32);
        assert!(matches!(combo.children[1], Node::Kern { .. }));
        assert_eq!(as_char(&as_box(&combo.children[2]).children[0]), b'1' as u32);
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
        // tex.web sub_mlist 核 → hpack 成单独 hbox（GT：{ab} 组盒宽 = ab）
        let a = math_line_children(r"${ab}$");
        assert_eq!(a.len(), 1, "组收成单个 Ord 原子盒");
        let g = as_box(&a[0]);
        assert_eq!(g.kind, BoxKind::HBox);
        assert_eq!(as_char(&g.children[0]), b'a' as u32);
        assert_eq!(as_char(&g.children[1]), b'b' as u32);
        // 裸 ab 不装箱
        let b = math_line_children(r"$ab$");
        assert_eq!(b.len(), 2);
        assert_eq!(as_char(&b[0]), b'a' as u32);
        assert_eq!(as_char(&b[1]), b'b' as u32);
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
        // 公式盒 = \hbox to \hsize 居中（两侧 \hfil），中为 x。
        // 30_785_863 = 469.75499pt（tex.web 1in 常量；pdfTeX `HS=[\the\hsize]`
        // 实测同值，2026-09-14）——旧值 30_785_859 是 6.5in 换算的圆整残差。
        assert_eq!(boxed.width, 30_785_863, "公式盒宽 = \\hsize");
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
        // 组 = sub_mlist 核：分式壳盒（null 定界符 + vlist）先并入组盒（hpack），
        // 再进外层横列表
        let a = math_line_children(r"${a\over b}$");
        assert_eq!(a.len(), 1);
        let g = as_box(&a[0]);
        assert_eq!(g.kind, BoxKind::HBox, "组内分式收进组盒");
        assert_eq!(g.children.len(), 1);
        let shell = as_box(&g.children[0]);
        assert_eq!(shell.kind, BoxKind::HBox, "make_fraction 壳 = hpack[定界符, v, 定界符]");
        let b = math_line_children(r"$a\over b$");
        assert_eq!(b.len(), 1, "裸分式直接进外层");
        let frac = as_box(&b[0]);
        assert_eq!(frac.kind, BoxKind::HBox);
        assert_eq!(frac.children.len(), 3, "null 定界符 + vlist + null 定界符");
        let core_v = as_box(&shell.children[1]);
        assert_eq!(core_v.children, as_box(&frac.children[1]).children, "两种写法分式体一致");
    }

    #[test]
    fn math_fraction_with_scripts_in_sup() {
        // x^{a\over b}：上标字段内含分式
        let children = math_line_children(r"$x^{a\over b}$");
        assert_eq!(children.len(), 2);
        let sup = as_box(&children[1]);
        // 上标字段 = make_fraction 壳盒 [null 定界符, vlist, null 定界符]
        assert_eq!(sup.children.len(), 3, "上标字段 = 壳盒三件");
        assert!(matches!(sup.children[1], Node::Box(_)), "中间为分式 vlist");
        let frac = as_box(&sup.children[1]);
        assert_eq!(frac.kind, BoxKind::VBox);
        assert!(frac.children.iter().any(|n| matches!(n, Node::Rule { .. })));
    }

    #[test]
    fn math_sqrt_radical_box() {
        // tex.web make_radical：外层 hbox = [√ 字形盒(shifted), 覆盖线 vbox]
        let children = math_line_children(r"$\sqrt{x}$");
        assert_eq!(children.len(), 1);
        let b = as_box(&children[0]);
        assert_eq!(b.kind, BoxKind::HBox);
        assert_eq!(b.children.len(), 2, "[√ 字形盒, 根号体 vbox]");
        let over = as_box(&b.children[1]);
        assert_eq!(over.kind, BoxKind::VBox);
        // [kern, 横线 rule, kern, 内容盒]
        assert!(matches!(over.children[1], Node::Rule { .. }));
        let base = as_box(&over.children[3]);
        assert_eq!(as_char(&base.children[0]), b'x' as u32);
    }

    #[test]
    fn math_sqrt_single_atom() {
        let children = math_line_children(r"$\sqrt x$");
        assert_eq!(children.len(), 1);
        assert_eq!(as_box(&children[0]).kind, BoxKind::HBox);
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
    fn math_penalty_inline_between_chars() {
        // 债务表项 1 的验收探针：`$a\penalty100 b$`——penalty 落在 a/b 之间
        // （tex.web math list 的 penalty 节点，公式内断行候选）
        let children = math_line_children(r"$a\penalty100 b$");
        let kinds: Vec<&str> = children
            .iter()
            .map(|n| match n {
                Node::Char { .. } => "char",
                Node::Penalty { .. } => "penalty",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["char", "penalty", "char"],
            "penalty 应在 a/b 之间：{children:?}"
        );
        let pd = children[1].dimensions();
        assert_eq!((pd.width, pd.height, pd.depth), (0, 0, 0));
        match &children[1] {
            Node::Penalty { penalty } => assert_eq!(*penalty, 100),
            other => panic!("预期 penalty 节点：{other:?}"),
        }
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
