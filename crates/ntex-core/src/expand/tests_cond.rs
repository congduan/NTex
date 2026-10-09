use super::*;

    #[test]
    fn ifx_noexpand_macro_is_unequal() {        // tex.web l.7506-7516（no_expand_flag=257）：`\noexpand` 标记的 cs 在
        // `\ifx` 比较时，若原含义**可展开**则被替换为 (relax,257)——与原含义
        // 不等；不可展开 cs / 字符 token 含义原样保留（相等）。expl3 全族
        // "宏还是寄存器" 判别 `\exp_after:wN \if_meaning:w \exp_not:N #1 #1`
        // 依赖此语义：宏 → \else 臂（展开一次取值）、寄存器 → \the 臂。
        // 宏（可展开）→ 不等。`\noexpand` 须先经 `\expandafter` 展开（真实
        // TeX 里裸 `\ifx\noexpand\a\a` 的第一操作数是 `\noexpand` 原语本身）
        assert_eq!(
            expand("\\def\\a{XX}\\expandafter\\ifx\\noexpand\\a\\a T\\else F\\fi").unwrap(),
            "F"
        );
        // 不可展开（\let 到 \relax）→ 含义原样保留 → 相等
        assert_eq!(
            expand("\\let\\b\\relax\\expandafter\\ifx\\noexpand\\b\\b T\\else F\\fi").unwrap(),
            "T"
        );
        // 可展开原语（\csname，cur_cmd>max_command）→ 同样被替换 → 不等
        assert_eq!(
            expand("\\expandafter\\ifx\\noexpand\\csname\\csname T\\else F\\fi").unwrap(),
            "F"
        );
        // 不可展开原语（\count，赋值类）→ 含义原样保留 → 相等
        assert_eq!(
            expand("\\expandafter\\ifx\\noexpand\\count\\count T\\else F\\fi").unwrap(),
            "T"
        );
        // 无 \noexpand 时语义不变（宏 vs 宏同义）
        assert_eq!(expand("\\def\\a{XX}\\ifx\\a\\a T\\else F\\fi").unwrap(), "T");
    }

    #[test]
    fn ifx_noexpand_survives_macro_argument_in_expansion() {
        // LaTeX robust/protect 壳会在 \edef/\write 展开区域里经宏实参转交
        // \noexpand 冻结 token；#1 替换时若丢掉冻结位，后续 \ifx 会把宏误判
        // 为自身含义相等，条件分支随之失衡。
        let src = concat!(
            "\\def\\a{XX}",
            "\\def\\wrap#1{\\ifx#1\\a T\\else F\\fi}",
            "\\edef\\x{\\expandafter\\wrap\\expandafter{\\noexpand\\a}}",
            "\\x"
        );
        assert_eq!(expand(src).unwrap(), "F");
    }

    #[test]
    fn if_mode_conditions() {
        // 纯展开轨道 sink 恒为垂直模式：\ifvmode 真、\ifhmode/\ifmmode 假
        assert_eq!(expand("\\ifvmode yes\\else no\\fi").unwrap(), "yes");
        assert_eq!(expand("\\ifhmode yes\\else no\\fi").unwrap(), "no");
        assert_eq!(expand("\\ifmmode yes\\else no\\fi").unwrap(), "no");
        // \unless 交互
        assert_eq!(expand("\\unless\\ifvmode yes\\else no\\fi").unwrap(), "no");
        // 盒子寄存器种类：\ifvoid/\ifhbox/\ifvbox（展开轨道无盒子 → 全 void）
        assert_eq!(expand("\\ifvoid0 yes\\else no\\fi").unwrap(), "yes");
        assert_eq!(expand("\\ifhbox0 yes\\else no\\fi").unwrap(), "no");
        assert_eq!(expand("\\ifvbox0 yes\\else no\\fi").unwrap(), "no");
    }

    // ---------- M1-9 条件原语 ----------

    #[test]
    fn iftrue_else_branch() {
        assert_eq!(expand("\\iftrue yes\\else no\\fi").unwrap(), "yes");
        assert_eq!(expand("\\iffalse yes\\else no\\fi").unwrap(), "no");
    }

    #[test]
    fn if_compares_char_tokens() {
        // 操作数取紧邻两个 token；分支用花括号包裹以避开尾随空格歧义
        assert_eq!(expand("\\if aa{y}\\else n\\fi").unwrap(), "y");
        assert_eq!(expand("\\if ab{y}\\else n\\fi").unwrap(), "n");
    }

    #[test]
    fn ifcat_compares_catcodes() {
        // 'a' cat11 vs '1' cat12 → 不等；'a' vs 'b' → 相等
        assert_eq!(expand("\\ifcat a1{y}\\else n\\fi").unwrap(), "n");
        assert_eq!(expand("\\ifcat ab{y}\\else n\\fi").unwrap(), "y");
    }

    #[test]
    fn ifcat_noexpand_active_char_stays_cat13() {
        // GT pdftex 1.40.29 四例定案（第十二刀）：`\noexpand` 冻结的 active
        // char 在 `\ifcat` 眼里仍是 catcode 13（定义与否无关）；真名 cs 是
        // relax 哨兵（16 类）。latex.ltx l.1398 `\declare@robustcommand` 的
        // auxi/auxiii 分派判别式 `\ifcat\noexpand~\noexpand#1` 依赖此语义：
        // `~` 尚未定义时须判 F（否则全体 `\DeclareRobustCommand` 产物走
        // active char 的 auxi 误路 → NFSS 定义群静默丢失）。
        assert_eq!(
            expand("\\catcode`\\~=13 \\ifcat\\noexpand~\\noexpand\\fooxyz T\\else F\\fi")
                .unwrap()
                .trim(),
            "F"
        );
        // 两 active char（都未定义）→ 相等 → T
        assert_eq!(
            expand("\\catcode`\\~=13 \\ifcat\\noexpand~\\noexpand~ T\\else F\\fi")
                .unwrap()
                .trim(),
            "T"
        );
        // 未定义 active char vs 已定义宏 → F（cat13 vs cs）
        assert_eq!(
            expand("\\catcode`\\~=13 \\def\\mac{xx}\\ifcat\\noexpand~\\noexpand\\mac T\\else F\\fi")
                .unwrap()
                .trim(),
            "F"
        );
    }

    #[test]
    fn ifcat_noexpand_undefined_cs_is_silent_relax() {
        // tex.web no_expand：`\noexpand` 冻结的未定义 cs **不报** Undefined
        // control sequence（当 \relax/哨兵）。第十二刀前 undefined 臂先执行，
        // latex.ltx 每个 `\DeclareRobustCommand` 定义点都误报两条
        // （`\~` + 被定义名）。此处校验判定结果不因报错臂翻面。
        assert_eq!(
            expand("\\expandafter\\ifcat\\noexpand\\undefinedxyz\\noexpand~ T\\else F\\fi")
                .unwrap()
                .trim(),
            "F"
        );
    }

    #[test]
    fn ifnum_with_relations() {
        assert_eq!(expand("\\ifnum3>2 yes\\else no\\fi").unwrap(), "yes");
        assert_eq!(expand("\\ifnum2>3 yes\\else no\\fi").unwrap(), "no");
        assert_eq!(expand("\\ifnum5=5 yes\\else no\\fi").unwrap(), "yes");
        assert_eq!(expand("\\ifnum4<4 yes\\else no\\fi").unwrap(), "no");
    }

    #[test]
    fn ifnum_relation_position_expands_filler() {
        // LaTeX 兼容第十刀：关系符位置 = tex.web `repeat get_x_token until
        // cur_cmd<>spacer`——可展开 filler（`\def\z{=}`）先展开再判 < = >。
        // 修复前 `\z` 不展开 → "Missing = inserted"、`=` 落到右操作数被当垃圾，
        // `0 T` 泄漏为排版文本（docs/archive/latex-feasibility.md §15.5 最小复现）。
        assert_eq!(expand("\\def\\z{=}\\ifnum0\\z 0 T\\else F\\fi").unwrap(), "T");
        assert_eq!(expand("\\def\\z{=}\\ifnum0\\z 1 T\\else F\\fi").unwrap(), "F");
        // < > 同族（filler 两向）
        assert_eq!(expand("\\def\\lt{<}\\ifnum1\\lt 2 T\\else F\\fi").unwrap(), "T");
        assert_eq!(expand("\\def\\lt{<}\\ifnum2\\lt 1 T\\else F\\fi").unwrap(), "F");
        assert_eq!(expand("\\def\\gt{>}\\ifnum2\\gt 1 T\\else F\\fi").unwrap(), "T");
        assert_eq!(expand("\\def\\gt{>}\\ifnum1\\gt 2 T\\else F\\fi").unwrap(), "F");
        // \ifdim 同款
        assert_eq!(
            expand("\\def\\eq{=}\\ifdim1pt\\eq 1pt T\\else F\\fi").unwrap(),
            "T"
        );
        assert_eq!(
            expand("\\def\\eq{=}\\ifdim1pt\\eq 2pt T\\else F\\fi").unwrap(),
            "F"
        );
    }

    #[test]
    fn ifnum_relation_position_evaluates_nested_cond() {
        // expl3-code.tex L193-206 引擎门闩原形：关系符位置是可展开探测链
        // `\expandafter\ifx\csname <引擎标记>\endcsname\relax…=0`。修复前此处报
        // "Missing = inserted for \ifnum"（<to be read again> expandafter），
        // \else/\fi 被悬挂条件机吞掉、后续 \__kernel_primitive:NN 映射表级联。
        // \luatexversion 未定义（引擎栅栏契约）→ \csname 未定义名 = \relax
        // （第六刀）→ \ifx 真、分支产出空 → `=0` 在关系符位就位。
        assert_eq!(
            expand(concat!(
                "\\ifnum0\\expandafter\\ifx\\csname luatexversion\\endcsname\\relax",
                "\\else 1\\fi=0 T\\else F\\fi"
            ))
            .unwrap(),
            "T"
        );
        // 同形反向：\csname 展开为已定义的非 relax 原语 → \ifx 假 → \else 产出
        // `1` → 落"非关系符"臂（token 放回、按 = 恢复）→ 0=1 假
        assert_eq!(
            expand(concat!(
                "\\ifnum0\\expandafter\\ifx\\csname numexpr\\endcsname\\relax",
                "\\else 1\\fi=0 T\\else F\\fi"
            ))
            .unwrap(),
            "F"
        );
        // 条件原语在关系符位置直接求值：真分支产出关系符
        assert_eq!(expand("\\ifnum1\\iftrue=\\fi 1 T\\else F\\fi").unwrap(), "T");
        // 假分支跳过、\else 后产出关系符
        assert_eq!(
            expand("\\ifnum1\\iffalse<\\else=\\fi 1 T\\else F\\fi").unwrap(),
            "T"
        );
    }

    #[test]
    fn ifnum_relation_literal_forms_unchanged() {
        // 零回归护栏：关系符直写形式（TRIP 大量用例面）行为不变
        assert_eq!(expand("\\ifnum3>2 yes\\else no\\fi").unwrap(), "yes");
        assert_eq!(expand("\\ifnum4<4 yes\\else no\\fi").unwrap(), "no");
        assert_eq!(expand("\\ifdim5pt=5pt y\\else n\\fi").unwrap(), "y");
        // 非关系符（数字 char）在关系符位：报 "Missing = inserted"、token 放回、
        // 按 = 恢复（tex.web back_error）——1pt=2pt 假
        assert_eq!(expand("\\ifdim1pt 2pt y\\else n\\fi").unwrap(), "n");
    }

    #[test]
    fn ifdim_with_units() {
        assert_eq!(expand("\\ifdim1pt<2pt yes\\else no\\fi").unwrap(), "yes");
        // pdfTeX 实测：1in=4736286sp；72.27pt 四舍五入后 = 4736287sp ≠ 1in，
        // 72.26999pt = 4736286sp == 1in
        assert_eq!(
            expand("\\ifdim1in=72.27pt yes\\else no\\fi").unwrap(),
            "no"
        );
        assert_eq!(
            expand("\\ifdim1in=72.26999pt yes\\else no\\fi").unwrap(),
            "yes"
        );
    }

    #[test]
    fn ifx_compares_meanings() {
        // 相同定义的宏 → 相等；不同定义 → 不等
        assert_eq!(
            expand("\\def\\a{A}\\def\\b{A}\\ifx\\a\\b yes\\else no\\fi").unwrap(),
            "yes"
        );
        assert_eq!(
            expand("\\def\\a{A}\\def\\b{B}\\ifx\\a\\b yes\\else no\\fi").unwrap(),
            "no"
        );
        // \let 别名解析后与目标同义
        assert_eq!(
            expand("\\def\\a{A}\\let\\c\\a\\ifx\\a\\c yes\\else no\\fi").unwrap(),
            "yes"
        );
    }

    #[test]
    fn ifx_distinguishes_outer() {
        // tex.web：\ifx 比较 eqtb 条目，outer 是 eq_type 的一部分——需区分
        assert_eq!(
            expand("\\outer\\def\\x{A}\\def\\y{A}\\ifx\\x\\y yes\\else no\\fi").unwrap(),
            "no"
        );
        assert_eq!(
            expand("\\outer\\def\\x{A}\\outer\\def\\z{A}\\ifx\\x\\z yes\\else no\\fi").unwrap(),
            "yes"
        );
    }

    // ---------- A7：\ifx 别名环检测（替代 64 跳上限） ----------

    #[test]
    fn ifx_alias_cycle_no_hang() {
        // \let\a\b \let\b\a：值复制语义下两者都未定义 → \ifx 相等（不挂起）。
        // 若未来出现间接 Alias 环（如 .fmt 手工构造），meaning_key 的环检测
        // 视为未定义，同样不挂起（替代旧 64 跳硬上限）。
        assert_eq!(
            expand("\\let\\a\\b\\let\\b\\a\\ifx\\a\\b yes\\else no\\fi").unwrap(),
            "yes"
        );
        // 环 vs 正常宏 → 不等
        assert_eq!(
            expand("\\let\\a\\b\\let\\b\\a\\def\\c{A}\\ifx\\a\\c yes\\else no\\fi").unwrap(),
            "no"
        );
    }

    #[test]
    fn ifodd() {
        assert_eq!(expand("\\ifodd3 yes\\else no\\fi").unwrap(), "yes");
        assert_eq!(expand("\\ifodd4 yes\\else no\\fi").unwrap(), "no");
    }

    #[test]
    fn ifcase_selects_branch() {
        assert_eq!(
            expand("\\ifcase2 zero\\or one\\or two\\or three\\else many\\fi").unwrap(),
            "two"
        );
        assert_eq!(
            expand("\\ifcase0 zero\\or one\\or two\\else many\\fi").unwrap(),
            "zero"
        );
        assert_eq!(
            expand("\\ifcase5 zero\\or one\\else many\\fi").unwrap(),
            "many"
        );
        // 负数：跳过所有 \or 直到 \else（tex.web if_case；\ifcase-1 → else 分支）
        assert_eq!(
            expand("\\ifcase-1 zero\\or one\\or two\\else many\\fi").unwrap(),
            "many"
        );
        assert_eq!(
            expand("\\ifcase-1 a\\or b\\else c\\fi").unwrap(),
            "c"
        );
        // 跳过头（n > \or 数量）：同样落入 \else
        assert_eq!(
            expand("\\ifcase3 a\\or b\\else c\\fi").unwrap(),
            "c"
        );
        // \edef 收集 + \write 展开路径（ETRIP L351 \5 宏模式：\edef\6{\ifcase\lastnodetype...}）。
        // \message 输出走 VecSink.transcript（非 output 通道）
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source(
            "\\edef\\x{\\ifcase-1 char node\\or hlist node\\else empty\\fi}\\message{\\x}",
        )
        .unwrap();
        let sink = e.take_sink();
        let mut sink = sink;
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert!(
            sink.transcript.contains("empty"),
            "\\edef+\\ifcase-1 经 \\message 展开应输出 else 分支，实际 {:?}",
            sink.transcript
        );
    }

    #[test]
    fn conditional_skips_without_expanding() {
        // 跳过分支中的 \def 与嵌套 \if 不得执行/展开
        let src = "\\iffalse \\def\\bad{OOPS}\\bad \\ifnum1=1 hi\\else no\\fi \\else good\\fi";
        assert_eq!(expand(src).unwrap(), "good");
    }

    #[test]
    fn nested_conditionals() {
        let src = "\\iftrue A\\ifnum2>1 B\\else C\\fi D\\else E\\fi";
        assert_eq!(expand(src).unwrap(), "ABD");
        let src2 = "\\iffalse A\\ifnum2>1 B\\else C\\fi D\\else E\\fi";
        assert_eq!(expand(src2).unwrap(), "E");
    }

    #[test]
    fn unbalanced_fi_reports_and_recovers() {
        // TeX 错误恢复（ETRIP 对齐）：多余 \fi/\else/\or 报 "! Extra ..." 消息并继续运行
        let mut e = Expander::new();
        e.run_source("\\fi x").unwrap();
        assert!(e.transcript().contains("! Extra \\fi."));
        let mut e = Expander::new();
        e.run_source("\\else x").unwrap();
        assert!(e.transcript().contains("! Extra \\else."));
    }

    #[test]
    fn if_operand_uses_get_x_token_expansion() {
        // tex.web @<Test if two characters match@>：操作数 get_x_token——宏/可展开
        // 原语先展开一次再比；非字符操作数 cur_chr:=256 哨兵（cs vs cs 恒真、
        // 字符 vs cs 恒假）。expl3 变体生成循环
        // `\if:w #4 \__cs_generate_variant_loop_base:N #2`（expl3-code L2861）的
        // 右操作数是宏调用，未展开时恒假 → 变体串分析全错（报告 §21）。
        // （分支首 token 前的空格属真/假分支文本，比较用 trim）
        assert_eq!(expand(r"\def\A{x}\if x\A T\else F\fi").unwrap().trim(), "T");
        assert_eq!(expand(r"\def\A{x}\if\A x T\else F\fi").unwrap().trim(), "T");
        assert_eq!(expand(r"\def\A{y}\if\A x T\else F\fi").unwrap().trim(), "F");
        assert_eq!(expand(r"\if\relax\relax T\else F\fi").unwrap().trim(), "T");
        assert_eq!(expand(r"\if x\relax T\else F\fi").unwrap().trim(), "F");
        assert_eq!(expand(r"\ifcat\relax\hbox T\else F\fi").unwrap().trim(), "T");
        assert_eq!(
            // 展开为字母 a：letter vs letter 同类
            expand(r"\def\A{a}\ifcat\A b T\else F\fi").unwrap().trim(),
            "T"
        );
        assert_eq!(expand(r"\if ab T\else F\fi").unwrap().trim(), "F");
        assert_eq!(expand(r"\if aa T\else F\fi").unwrap().trim(), "T");
    }

    // ---------- 第十八刀：\if 字符操作数位的嵌套条件 = 就地求值（§24 重开 §22.3） ----------

    /// §22.3 曾按"数据哨兵"裁决（d672423 求值后帧不收口，内层 `\else`/`\fi`
    /// 落到外层条件 → expl3 变体机器 `\if:w 0 <A><C>0` 结构被 A 链的 `\fi:`
    /// 错误收口）。第十八刀定位实害在**帧不收口**而非求值本身，重开为
    /// tex.web get_x_token → expand → conditional 的**求值语义**：操作数位的
    /// `\if*` 完整求值（帧自带收口 + 被弃分支 drain），只让选中分支文本落入
    /// 操作数扫描。四值与 tex.web `@<Test if two characters match@>` 逐点推演
    /// 一致（2026-09-05），expl3 变体机器 l.3245-3395 全绿随之达成（§24）。
    #[test]
    fn if_operand_nested_conditional_evaluates() {
        // V1：嵌套 \if c o 假 → 取 else 支 `n` 作外层右操作数 → n=n 真 → X T
        assert_eq!(
            expand(r"\if n\if c o N\else n\fi X T\else F\fi").unwrap(),
            "X T"
        );
        // V2：嵌套 else 支首 token=m → n≠m 假 → F
        assert_eq!(
            expand(r"\if n\if c o N\else m\fi X T\else F\fi").unwrap(),
            "F"
        );
        // V3：嵌套真支 `N` → n≠N 假 → F
        assert_eq!(
            expand(r"\if n\if c c N\else m\fi n T\else F\fi").unwrap(),
            "F"
        );
        // V4：嵌套无 \else（假 → 空贡献）→ 外层右操作数取 `X` → n≠X 假 → F
        assert_eq!(
            expand(r"\if n\if c o N\fi X T\else F\fi").unwrap(),
            "F"
        );
    }

    /// 第十六刀原始构造（模拟 expl3 `\__cs_generate_variant_loop_base:N`
    /// idiom，expl3-code L2820-2834）的 ground truth 回归锁。`\loopa` 体内
    /// 第二个 `\fi:` 是多余收口，且嵌套真支 `n` 前的空格才是外层操作数——
    /// 真实 TeX 由此报 3 个可恢复错误（`! Extra \fi` → `! Extra \else` →
    /// `! Extra \fi`）并输出 SAMEDIFF；引擎须与之逐点一致（此前测试臆想的
    /// SAME 期望值即本文件 CI 失败的根因）。
    #[test]
    fn if_operand_nested_conditional_degenerate_matches_real_tex() {
        let src = concat!(
            "\\catcode`\\:=11 \\catcode`\\_=11 ",
            "\\let\\if:w\\if \\let\\else:\\else \\let\\fi:\\fi ",
            "\\long\\def\\basea#1{\\if:w c #1 N \\else: \\if:w o #1 n \\else: q\\fi: \\fi:} ",
            "\\long\\def\\loopa#1#2{\\if:w #1 \\basea #2 \\fi: \\fi: SAME\\else: DIFF\\fi:} "
        );
        let (r, t) = run_transcript(&format!("{src}\\loopa n o"));
        assert!(r.is_ok(), "多余收口应可恢复继续：{t}");
        assert!(t.contains("Extra \\fi"), "应报 Extra \\fi：{t}");
        assert!(t.contains("Extra \\else"), "应报 Extra \\else：{t}");
        assert_eq!(expand(&format!("{src}\\loopa n o")).unwrap().trim(), "SAMEDIFF");
        assert_eq!(expand(&format!("{src}\\loopa q q")).unwrap().trim(), "SAMEDIFF");
        assert_eq!(
            expand(&format!("{src}\\loopa n o\\loopa q o"))
                .unwrap()
                .trim(),
            "SAMEDIFFSAMEDIFF"
        );
    }

    #[test]
    fn ifnum_nested_cond_else_digit_joins_left_operand() {
        // expl3 引擎门闩 `\ifnum0\ifx…\else 1\fi=0`：嵌套条件假分支的数字 `1`
        // 必须并入左操作数（scan_number 十进制循环展开 `\expandafter`/可展开项，
        // 条件链就地求值）；否则关系符扫描拿 `1` 报 "Missing = inserted"。
        assert_eq!(
            expand("\\ifnum0\\ifx\\expanded\\relax\\else 1\\fi=0 ABORT\\else OK\\fi").unwrap(),
            "OK"
        );
        // 旧引擎路径（\expanded 未定义，\csname 制造 relax）：内层 \ifx 真、
        // 分支为空 → 0=0 真（\expandafter 先展开 \csname——\ifx 不展开操作数，
        // 与 expl3 引擎门闩同构）
        assert_eq!(
            expand(
                "\\ifnum0\\expandafter\\ifx\\csname nope\\endcsname\\relax\\else 1\\fi=0 \
                 T\\else F\\fi"
            )
            .unwrap(),
            "T"
        );
    }

    #[test]
    fn conditional_inside_group_must_close() {
        // TeX 语义：\if 跨组合法（组结束不要求条件闭合）；输入结束时未闭合
        // 条件按 final_cleanup 报 "! Incomplete \iftrue; ..."（可恢复，不报错）。
        // （旧断言 is_err 是错误语义——TRIP L413 `\iftrue` 开、L424 `\endinput`
        // 结束即依赖此行为。）
        let (r, t) = run_transcript("\\begingroup\\iftrue A\\endgroup");
        assert!(r.is_ok(), "未闭合条件应可恢复：{t}");
        assert!(
            t.contains("! Incomplete \\iftrue; all text was ignored after line 1."),
            "转录：{t}"
        );
    }

    #[test]
    fn incomplete_iffalse_at_eof_recovers() {
        let (r, t) = run_transcript("\\iffalse ignored");
        assert!(r.is_ok(), "未闭合条件 EOF 应可恢复：{t}");
        assert!(
            t.contains("! Incomplete \\iffalse; all text was ignored after line 1."),
            "转录：{t}"
        );
    }

    #[test]
    fn ifx_compares_font_meanings() {
        // GT（pdfTeX -ini 实证）：\font 名字扫描遇 \ifx 会就地求值并把真支收进
        // 名字（\a 变 nullfont）——测试若要比较字体含义，须用 \relax 终止名字
        // （名字扫描对不可展开 CS unread+终止，tex.web scan_file_name done 臂）。
        let out = font_run(r"\font\a=cmr10 \relax\ifx\a\a yes\else no\fi").unwrap().0;
        assert_eq!(out, "yes");
        let out = font_run(r"\font\a=cmr10 \relax\font\b=cmr10 \relax\ifx\a\b yes\else no\fi")
            .unwrap()
            .0;
        assert_eq!(out, "no");
    }

    #[test]
    fn ifdefined_true_and_false() {
        assert_eq!(
            expand(r"\ifdefined\relax yes\else no\fi").unwrap(),
            "yes"
        );
        assert_eq!(
            expand(r"\ifdefined\neverdefinedcs123 yes\else no\fi").unwrap(),
            "no"
        );
    }

    #[test]
    fn nested_cond_in_number_scan() {
        // expl3/latex.ltx L1122 惯用法 `\ifnum0\ifdefined X 1\fi...>0`：数字扫描中
        // 嵌套条件就地求值（tex.web scan_int 的 get_x_token 对 if_test 展开）。
        // 第七刀修复前：十进制数字循环缺条件臂 → \ifdefined 不展开 → "Missing =
        // inserted for \ifnum" 恢复，>0 残留成正文输出。
        // \ifdefined 真 → 贡献数字 1 → 操作数 01=1 >0 真
        assert_eq!(
            expand(r"\ifnum0\ifdefined\relax 1\fi>0 T\else F\fi").unwrap(),
            "T"
        );
        // \ifdefined 假 → 无贡献 → 操作数 0 >0 假
        assert_eq!(
            expand(r"\ifnum0\ifdefined\zzneverdefined 1\fi>0 T\else F\fi").unwrap(),
            "F"
        );
        // 多探针聚合（latex.ltx L1122 引擎检查原形）：两个都真 → 011=11 >0 真
        assert_eq!(
            expand(r"\ifnum0\ifdefined\relax 1\fi\ifdefined\eTeXversion 1\fi>0 T\else F\fi")
                .unwrap(),
            "T"
        );
        // 一真一假 → 01=1 >0 真；全假 → 0 >0 假
        assert_eq!(
            expand(r"\ifnum0\ifdefined\relax 1\fi\ifdefined\qqneverdefined 1\fi>0 T\else F\fi")
                .unwrap(),
            "T"
        );
        assert_eq!(
            expand(r"\ifnum0\ifdefined\qqneverdefined 1\fi\ifdefined\zzneverdefined 1\fi>0 T\else F\fi")
                .unwrap(),
            "F"
        );
        // \else 分支（expl3 变体 `\ifdefined X 1\else 0\fi`）：真条件贡献 1 后 \else
        // 死分支被跳过、\fi 弹帧，数字继续累计
        assert_eq!(
            expand(r"\ifnum0\ifdefined\relax 1\else 0\fi>0 T\else F\fi").unwrap(),
            "T"
        );
        assert_eq!(
            expand(r"\ifnum0\ifdefined\qqneverdefined 1\else 0\fi>0 T\else F\fi").unwrap(),
            "F"
        );
        // 嵌套多层：\ifdefined 真分支里再套一层 \ifdefined（latex.ltx L1125
        // `\ifdefined\luatexversion\ifnum...>94 1\fi\fi` 同构）
        assert_eq!(
            expand(r"\ifnum0\ifdefined\relax\ifdefined\relax 1\fi\fi>0 T\else F\fi").unwrap(),
            "T"
        );
        assert_eq!(
            expand(r"\ifnum0\ifdefined\relax\ifdefined\qqneverdefined 1\fi\fi>0 T\else F\fi")
                .unwrap(),
            "F"
        );
        // 真值聚合精确比较（=11 惯用法）：两探针都真 → 11
        assert_eq!(
            expand(r"\ifnum0\ifdefined\relax 1\fi\ifdefined\eTeXversion 1\fi=11 T\else F\fi")
                .unwrap(),
            "T"
        );
    }

    #[test]
    fn ifcsname_true_and_false() {
        assert_eq!(
            expand(r"\ifcsname relax\endcsname yes\else no\fi").unwrap(),
            "yes"
        );
        assert_eq!(
            expand(r"\ifcsname neverdefinedxyz\endcsname yes\else no\fi").unwrap(),
            "no"
        );
    }

    // ---- 第十一刀：e-TeX `\ifincsname`（GT pdftex 1.40.29 四例对拍） ----

    #[test]
    fn ifincsname_false_outside_csname() {
        // GT A=F：csname 名字扫描外恒假
        assert_eq!(expand(r"\ifincsname yes\else no\fi").unwrap(), "no");
        // GT D（\csname z\endcsname 后 F）：endcsname 闭合即复位
        assert_eq!(
            expand(r"\csname zzneverdefined\endcsname\ifincsname yes\else no\fi").unwrap(),
            "no"
        );
    }

    #[test]
    fn ifincsname_true_during_csname_scan() {
        // GT ONE/TWO=YES：扫描内宏展开任意深度旗标皆真——T 支字符收进名字。
        // 用 \ifcsname 判定收进的是 T 而非 F 支：xF 已造槽时名字若误收 F 会判 yes。
        assert_eq!(
            expand(
                r"\def\q{\ifincsname T\else F\fi}%
\csname xF\endcsname\ifcsname x\q\endcsname yes\else no\fi"
            )
            .unwrap(),
            "no"
        );
        assert_eq!(
            expand(
                r"\def\q{\ifincsname T\else F\fi}%
\def\qq{\q}%
\csname nF\endcsname\ifcsname n\qq\endcsname yes\else no\fi"
            )
            .unwrap(),
            "no"
        );
        // \csname 造出的 relax 槽算已定义：置 xT 后同名探测应判 yes
        assert_eq!(
            expand(
                r"\def\q{\ifincsname T\else F\fi}%
\csname xT\endcsname\ifcsname x\q\endcsname yes\else no\fi"
            )
            .unwrap(),
            "yes"
        );
        // GT IFCS=YES：\ifcsname 自己的名字扫描内旗标亦真
        assert_eq!(
            expand(
                r"\def\q{\ifincsname T\else F\fi}%
\csname aF\endcsname\ifcsname a\q\endcsname yes\else no\fi"
            )
            .unwrap(),
            "no"
        );
    }

    #[test]
    fn ifincsname_robust_body_idiom() {
        // latex.ltx l.1409 \declare@robustcommand@auxii 惯用法（\DeclareRobustCommand
        // 生成的命令体）：csname 外取实体形、csname 扫描内取 \string 形——
        // 这是 2025 版 \IfFileExists/\InputIfFileExists 的入口体。
        let src = r"\long\def\f#1#2{#1}\long\def\s#1#2{#2}%
\def\probe{\ifincsname\expandafter\f\else\expandafter\s\fi{IN}{OUT}}";
        assert_eq!(expand(&format!("{src}\\probe")).unwrap(), "OUT");
        // 扫描内 \probe 展开为 IN → 建 cs "IN"（未定义则造 relax 槽）→ 判 yes
        assert_eq!(
            expand(&format!("{src}\\csname\\probe\\endcsname\\ifcsname IN\\endcsname yes\\else no\\fi"))
                .unwrap(),
            "yes"
        );
    }

    #[test]
    fn unless_reverses_condition() {
        assert_eq!(expand(r"\unless\iftrue yes\else no\fi").unwrap(), "no");
        assert_eq!(expand(r"\unless\iffalse yes\else no\fi").unwrap(), "yes");
    }

    #[test]
    fn ifprimitive_tests_primitive_definition() {
        assert_eq!(
            expand(r"\ifprimitive\relax yes\else no\fi").unwrap(),
            "yes"
        );
        assert_eq!(
            expand(r"\ifprimitive\zzzundef123 yes\else no\fi").unwrap(),
            "no"
        );
        assert_eq!(expand(r"\ifprimitive a yes\else no\fi").unwrap(), "no");
        // \let 到原语：复制含义后即原语 → \ifprimitive 为真（e-TeX 语义）
        assert_eq!(
            expand(r"\let\pr=\relax\ifprimitive\pr yes\else no\fi").unwrap(),
            "yes"
        );
    }

    #[test]
    fn ifcase_ifeof_fi_def_chain() {
        // trip.tex L419：\the\tokens\ifcase1\or\ifeof\fi\def\stopinput{\error\let\input\die}
        // \ifeof 缺流号报 Missing number 后，\fi 应闭合 \ifcase（tex.web pass_text 弹栈顶帧），
        // \def\stopinput 必须执行——否则 L424 \stopinput 报 Undefined。
        let out = expand("\\ifcase1\\or\\ifeof\\fi\\def\\stopinput{OK}\\stopinput").unwrap();
        assert!(out.contains("OK"), "\\stopinput 应已定义：out={out:?}");
        // 完整场景（含 \the\toks 前缀）：toks256 预置内容，\the\tokens 打印后
        // \ifcase 链照常执行、\def\stopinput 必须生效。
        let out2 = expand("\\toksdef\\tokens=256\\tokens={ABC}\\the\\tokens\\ifcase1\\or\\ifeof\\fi\\def\\stopinput{OK}\\stopinput").unwrap();
        assert!(
            out2.contains("OK"),
            "完整场景 \\stopinput 应已定义：out={out2:?}"
        );
        // L354/L417 场景：toks256 含 \a 宏（\ifcat#1\message...\the\tokens 递归）
        let out3 = expand(
            "\\def\\a#1{\\ifcat#1 \\message\\ifx#1 {\\iffalse\\fi\\the\\tokens\\fi\\fi}}\\toksdef\\tokens=256\\tokens={ABC}\\the\\tokens\\ifcase1\\or\\ifeof\\fi\\def\\stopinput{OK}\\stopinput",
        )
        .unwrap();
        assert!(
            out3.contains("OK"),
            "宏场景 \\stopinput 应已定义：out={out3:?}"
        );
        // 真实 toks256 内容（L354）：\a^^@^^@a\par! —— \a 宏调用在 toks 里
        let out4 = expand(
            "\\def\\a#1{\\ifcat#1 \\message\\ifx#1 {\\iffalse\\fi\\the\\tokens\\fi\\fi}}\\toksdef\\tokens=256\\tokens={\\a^^@^^@a\\par!}\\the\\tokens\\ifcase1\\or\\ifeof\\fi\\def\\stopinput{OK}\\stopinput",
        )
        .unwrap();
        assert!(
            out4.contains("OK"),
            "真实 toks 场景 \\\\stopinput 应已定义：out={out4:?}"
        );
    }

    #[test]
    fn nested_cond_in_number_scan_with_newline_indent() {
        // latex.ltx L1122 实际形态：`\ifnum0%` 后换行缩进再接 \ifdefined 探针。
        // 两个修复的协同：① 注释吞行后扫描器状态须回 LineStart（input.rs——TeX
        // new_line 状态忽略行首空格），否则缩进空格产出 token 提前终止 `0`；
        // ② 十进制数字循环条件臂让探针就地求值聚合。两探针都真 → 011=11 >0 真。
        let src = "\\ifnum0%\n  \\ifdefined\\relax 1\\fi\n  \\ifdefined\\eTeXversion 1\\fi\n  >0 T\\else F\\fi";
        assert_eq!(expand(src).unwrap(), "T");
        // 全假 → 0 >0 假
        let src2 = "\\ifnum0%\n  \\ifdefined\\qqneverdefined 1\\fi\n  \\ifdefined\\zzneverdefined 1\\fi\n  >0 T\\else F\\fi";
        assert_eq!(expand(src2).unwrap(), "F");
    }
