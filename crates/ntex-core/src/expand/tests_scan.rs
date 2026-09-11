use super::*;

    /// 数字扫描**符号循环**的 `\else`/`\fi`/`\or` 须按 tex.web expand 的
    /// fi_or_else 臂（§9897 @<Terminate the current conditional and skip to
    /// \fi@>）推进条件机：栈顶帧已完成求值（if_limit=else_code）→ `\fi` 就地
    /// 弹帧、扫描继续；栈顶帧求值中（if_limit=if_code）→ insert_relax 门
    /// （token 放回 + 前插 `\relax`）；无帧 → 维持旧"放回 + Missing number"
    /// 恢复。expl3-code l.1888-1893 `\cs_to_str:N` 是依赖此语义的真实样例：
    ///
    /// ```tex
    /// \cs_gset:Npn \cs_to_str:N { \tex_romannumeral:D
    ///   \if:w \token_to_str:N \ \__cs_to_str:w \fi:
    ///   \exp_after:wN \__cs_to_str:N \token_to_str:N }
    /// \cs_gset:Npn \__cs_to_str:N #1 { \c_zero_int }
    /// \cs_gset:Npn \__cs_to_str:w #1 \__cs_to_str:N
    ///   { - \int_value:w \fi: \exp_after:wN \c_zero_int }
    /// ```
    ///
    /// `\if:w` 真臂里 `\number` 的嵌套数字扫描在符号循环遇到 `\fi:`（闭合
    /// `\if:w` 帧后须继续到 `\expandafter`/`\c_zero_int` 取 0 终止）；旧实现
    /// 符号循环把 `\fi:` 放回，落入十进制数字循环错位弹帧 → Missing number
    /// + `\string` 产物错位（expl3 sys/bool 区 40 条 Missing number 同族）。
    #[test]
    fn number_scan_sign_loop_steps_fi_of_completed_frame() {
        // l3kernel `\cs_to_str:N` 机制级复刻（`\if:w X` 取 cat 12 vs cat 12 同真）。
        let src = concat!(
            "\\catcode`\\_=11 \\catcode`\\:=11 %\n",
            "\\let\\fi:\\fi \\let\\if:w\\if %\n",
            "\\chardef\\c_zero_int=0 %\n",
            "\\long\\def\\__cs_to_str:N#1{\\c_zero_int} %\n",
            "\\long\\def\\__cs_to_str:w#1\\__cs_to_str:N",
            "{-\\number\\fi:\\expandafter\\c_zero_int} %\n",
            "\\def\\cs_to_str:N#1{\\romannumeral",
            "\\if:w -\\__cs_to_str:w\\fi:",
            "\\expandafter\\__cs_to_str:N\\string#1} %\n",
            "\\edef\\t{\\cs_to_str:N\\abc}\\t"
        );
        let (r, transcript) = run_transcript(src);
        r.unwrap();
        assert!(
            !transcript.contains("Missing number"),
            "数字扫描符号循环未推进条件机：{transcript}"
        );
        assert!(
            transcript.matches("Extra \\fi").count() <= 1,
            "\\fi 不得翻倍（因子位放回至多一次 Extra \\fi）：{transcript}"
        );
    }

    /// 符号循环遇**无帧可归属**的游离 `\else`/`\fi` 维持旧放回语义（TRIP L82
    /// `\ifnum'\ifnum10=10 12="\fi` 契约：radix 循环/符号循环的游离终结符须
    /// 留给外层条件机闭合，Missing number 恢复后由 skip_ahead 消费）。
    #[test]
    fn number_scan_sign_loop_keeps_free_fi_for_outer_machine() {
        // 外层 \ifnum 操作数扫描中（栈顶帧 Evaluating=insert_relax 门）遇 `\fi`：
        // token 放回、本扫描按 Missing number 收场，`\fi` 由外层 skip_ahead 闭合。
        let src = concat!(
            "\\count0=\\number\\fi 7 %\n",
            "\\count1=5 "
        );
        let (_r, transcript) = run_transcript(src);
        assert!(
            transcript.contains("Missing number"),
            "游离 \\fi 应触发 Missing number 恢复：{transcript}"
        );
    }

    /// 第二十五刀：数字扫描**符号循环的 is_skipping 臂**——条件机处于跳过区
    /// （`\else` 之后）时，嵌套 romannumeral（`\exp_after:wN X \exp:w` 的
    /// f-前瞻）不得展开假分支的宏。tex.web 的 get_x_token 在 fi_or_else 臂内
    /// **同步** `while cur_chr<>fi_code do pass_text` 后弹帧，假分支 token 到不了
    /// 数字扫描的取 token 位；本引擎是惰性 Skipping 帧，数字扫描必须自己问
    /// is_skipping()（tex.web pass_text：只计数 `\if*`、只推进 `\else`/`\fi`）。
    ///
    /// 2026 l3kernel 生成条件体 **normal 臂**（l3basics `\@@_generate_p_form`：
    /// `#8 = \use_i_ii:nnn` 路径）：
    /// ```tex
    /// \cs_new:Npn \foo_p:n #1 { <test> \prg_return_true: \else:
    ///   \prg_return_false: \fi: \exp_end: \c_true_bool \c_false_bool }
    /// ```
    /// `\prg_return_true:` = `\exp_after:wN \use_i:nn \exp:w`——`\exp:w` 在
    /// `\number` 的嵌套数字扫描里展开，遇 `\else:`（真分支 → else 臂）应就地
    /// 跳过假分支、`\fi:` 弹帧，再以 `\exp_end:`（chardef 0）收口，最后
    /// `\use_i:nn` 取 `\c_true_bool`(=1)/`\c_false_bool`(=0) 交回外层扫描。
    /// 缺 is_skipping 臂则假分支的 `\prg_return_false:` 被展开，其 `\exp:w`
    /// 吃掉 `\exp_end:`、`\use_ii:nn` 反手吞掉真臂两个 bool 常量 → 真臂取 0
    /// + 残留 token 级联（expl3 l.7952 区 34 条 `\use_ii:nn extra }` 主簇）。
    #[test]
    fn number_scan_skips_false_branch_of_nested_romannumeral() {
        let src = concat!(
            "\\catcode`\\_=11 \\catcode`\\:=11 %\n",
            "\\chardef\\exp_end:=0 \\chardef\\c_true_bool=1 \\chardef\\c_false_bool=0 %\n",
            "\\long\\def\\use_i:nn#1#2{#1}\\long\\def\\use_ii:nn#1#2{#2} %\n",
            "\\def\\prg_return_true:{\\expandafter\\use_i:nn\\romannumeral} %\n",
            "\\def\\prg_return_false:{\\expandafter\\use_ii:nn\\romannumeral} %\n",
            // normal 臂：真/假两支
            "\\edef\\ga{\\number\\ifnum1=1\\prg_return_true:\\else\\prg_return_false:\\fi",
            "\\exp_end:\\c_true_bool\\c_false_bool} %\n",
            "\\edef\\gb{\\number\\ifnum1=2\\prg_return_true:\\else\\prg_return_false:\\fi",
            "\\exp_end:\\c_true_bool\\c_false_bool} %\n",
            // fast 臂（`\__prg_p_true:w` 形态）作对照
            "\\def\\prgp_true:w#1\\fi\\c_false_bool{\\fi\\c_true_bool} %\n",
            "\\edef\\gc{\\number\\ifnum1=1\\prgp_true:w\\fi\\c_false_bool} %\n",
            "\\immediate\\write16{GA=\\ga,GB=\\gb,GC=\\gc}"
        );
        let (_r, transcript) = run_transcript(src);
        assert!(
            transcript.contains("GA=1,GB=0,GC=1"),
            "normal 臂真支应取 \\c_true_bool(1)：{transcript}"
        );
        assert!(
            !transcript.contains("Missing number") && !transcript.contains("Extra"),
            "真臂时序偏差（假分支宏被展开）：{transcript}"
        );
    }

    /// 数字扫描的**别名即原义**（tex.web §24.4：`\let` 在 eqtb 层复制含义）。
    /// 本引擎 Alias 槽保留于宏/未定义目标（`let_to`），数字扫描的符号循环
    /// expandable 检查与内部量分派、表达式因子/运算符位展开检查此前都不追链：
    /// `\number\宏别名` 落成 Missing number 取 0、别名 token 留流。
    #[test]
    fn number_scan_derefs_alias_to_target_meaning() {
        let src = concat!(
            "\\def\\mymac{140} %\n",
            "\\let\\alias\\mymac %\n",
            "\\immediate\\write16{AL=\\number\\alias} %\n",
            "\\def\\tmpa{5} \\let\\iv\\tmpa %\n",
            "\\immediate\\write16{EV=\\number\\numexpr 1+\\iv\\relax}"
        );
        let (_r, transcript) = run_transcript(src);
        for (tag, want) in [("AL=", "140"), ("EV=", "6")] {
            let line = transcript
                .lines()
                .find(|l| l.starts_with(tag))
                .unwrap_or_else(|| panic!("缺 {tag} 输出行：{transcript}"));
            assert_eq!(line, format!("{tag}{want}"), "真 TeX 对照：{transcript}");
        }
        assert!(
            !transcript.contains("Missing number"),
            "宏别名不得落成 Missing number：{transcript}"
        );
    }

    #[test]
    fn catcode_change_affects_later_input() {
        // \catcode92=12 后 `\` 变为普通字符。
        // 用 \relax 隔离数字与后续输入（数字扫描会预读紧邻 token，与真实 TeX 一致）。
        assert_eq!(expand("\\catcode92=12\\relax\\abc").unwrap(), "\\abc");
    }

    #[test]
    fn catcode_backquote_syntax() {
        // trip.tex 开头：\catcode `{ = 1（反引号字符码）
        assert_eq!(expand("\\catcode`{=1\\relax").unwrap(), "");
        // \catcode `$ = 3 {\catcode`$13 ...}：省略 `=` 的赋值
        assert_eq!(expand("\\catcode`$13\\relax").unwrap(), "");
        // 控制符号：\catcode`\@ = 15
        assert_eq!(expand("\\catcode`\\@=15\\relax").unwrap(), "");
        // ^^ 转义：\catcode `^^A = 8（另一写法）
        assert_eq!(expand("\\catcode`^^A=0008\\relax").unwrap(), "");
        // \sfcode 同样支持反引号
        assert_eq!(expand("\\sfcode`x=1000\\relax").unwrap(), "");
    }

    #[test]
    fn catcode_backquote_circumflex_control_symbol() {
        // \catcode `\^^@ = 11：^^@ 解码为字符码 0，控制符号名 "\0"
        assert_eq!(expand("\\catcode`\\^^@=11\\relax").unwrap(), "");
        // 读回：\catcode 0 现在是 11（letter）
        assert_eq!(
            expand("\\catcode`\\^^@=11\\relax\\ifnum\\catcode`\\^^@=11 yes\\else no\\fi").unwrap(),
            "yes"
        );
    }

    #[test]
    fn catcode_internal_int_reads_catcode() {
        // \catcode `U = \catcode`#：把 U 的 catcode 设为 # 的当前 catcode（默认 6）
        assert_eq!(
            expand("\\catcode`U=\\catcode`#\\relax\\ifnum\\catcode`U=6 yes\\else no\\fi").unwrap(),
            "yes"
        );
    }

    #[test]
    fn delcode_assign_and_the() {
        // etrip.tex 第 73 行：\delcode`\[="161361（hex 字符码 + hex 值）；\the 读回
        assert_eq!(
            expand("\\delcode`\\[=\"161361\\relax\\the\\delcode`\\[").unwrap(),
            "1446753"
        );
        // 未赋值字符的默认 delcode = 0x500000
        assert_eq!(expand("\\the\\delcode`x").unwrap(), "5242880");
    }

    #[test]
    fn group_delimiter_alias_is_data_in_token_list_scan() {
        // 第二十七轮（tex.web scan_toks/scan_general_text）：token 列表里的组
        // 判定看 **cur_tok**——`\let\bg={` 型 cs 经 get_token 仍是 cs token
        // （组性只体现在 cur_cmd=eq_type），不得归一成字符 token。此前 e 型体
        // （expl3 `\use:e` 生成条件体）把 `\c_group_begin_token` 归一成字面
        // `{`，组深 +1 永不闭合 → 组平衡崩塌：expl3 `\token_if_*` 生成条件区
        // 每条定义 1×Missing number + 2×`\use_ii:nn extra }`（57 错主簇）。
        // 开括号位（scan_left_brace 的 cur_cmd 判定）归一不受影响——真 TeX
        // `\let\bgroup={\everyjob\bgroup y` 合法收 `y`，但 `\egroup` 不配平。
        assert_eq!(
            expand("\\let\\bg={\\def\\f#1{[#1]}\\edef\\x{\\f\\bg}\\meaning\\x}").unwrap(),
            "macro:->[\\bg]"
        );
        // 对照：真 TeX 数字上下文 `\let` 别名不是内部量（char_given 才是）——
        // `\ifnum\bg=1` 报 Missing number（本测只锁 token 位数据性，数字位
        // 内部量分派的 char_given/let 区分是下一轮靶）。
    }

    #[test]
    fn scan_left_brace_expandable_filler() {
        // tex.web scan_left_brace（L8194-8206）：toks 值扫描的 `{` 入口是
        // get_x_token（可展开 filler）——latex.ltx L727
        // `\everyjob\expandafter{\the\everyjob\the\LaTeXReleaseInfo}` 的最小复现
        // （`\expandafter` 展开把 `{` 压回，`\string` 随之展开成字符 token）
        assert_eq!(
            expand("\\everyjob\\expandafter{\\string x}\\the\\toks0").unwrap(),
            "x"
        );
        // 宏 filler + `\let\bgroup={` 别名作组定界（tex.web scan_left_brace
        // 只认 cur_cmd=left_brace；本引擎经 resolve_group_char 归一）
        assert_eq!(
            expand(
                "\\let\\bgroup={\\let\\egroup=}\
                 \\def\\f{\\bgroup y\\egroup}\\everyjob\\f\\the\\toks0"
            )
            .unwrap(),
            "y"
        );
        // spacer 与 \relax 跳过（tex.web L8210 `until (cur_cmd<>spacer)
        // and (cur_cmd<>relax)`）
        assert_eq!(expand("\\toks1=\\relax  {z}\\the\\toks1").unwrap(), "z");
        // toks 寄存器 RHS 复制（tex.web <If the right-hand side is a token
        // parameter or token register>）不受 filler 语义影响
        assert_eq!(
            expand("\\toks1={a}\\toks2=\\toks1\\the\\toks2").unwrap(),
            "a"
        );
        // 非 `{`：报 "Missing { inserted." 后 token 放回照常收集（TeX 恢复语义，
        // TRIP L438 `\mathchoice{}a}{...}` 同路径）
        let mut e = Expander::new();
        e.run_source("\\everyjob q").unwrap();
        assert!(
            e.transcript().contains("! Missing { inserted."),
            "{}",
            e.transcript()
        );
    }

    #[test]
    fn lccode_assign_and_read() {
        // etrip.tex 88 行：\lccode`A=`a；数字上下文读回
        assert_eq!(
            expand("\\lccode`A=`a\\relax\\ifnum\\lccode`A=`a yes\\else no\\fi").unwrap(),
            "yes"
        );
        // 寄存器值作字符码：\lccode\count20=0（etrip.tex 91 行）
        assert_eq!(
            expand("\\count20=65\\lccode\\count20=0\\relax\\ifnum\\lccode`A=0 yes\\else no\\fi").unwrap(),
            "yes"
        );
        // \the 读回（字母常量后跟空格：tex.web @<Scan an optional space@> 把空格
        // 吞掉，\the 在赋值完成后才求值）
        assert_eq!(expand("\\lccode`B=`b \\the\\lccode`B").unwrap(), "98");
        // 字母常量后**紧跟** \the：tex.web @<Scan an optional space@> 是
        // `get_x_token; if cur_cmd<>spacer then back_input`——get_x_token 会展开
        // `\the`（convert > max_command），展开产物留在流里（back_input 只放回
        // 当前 token），此刻赋值尚未发生 → 读到旧值 0。expl3 f 型展开
        // （`\exp:w \exp_end_continue_f:w`）正依赖此"字母常量后继续展开"语义。
        assert_eq!(expand("\\lccode`B=`b\\the\\lccode`B").unwrap(), "0");
        // 组作用域回滚
        assert_eq!(
            expand("\\lccode`C=1{\\lccode`C=2}\\the\\lccode`C").unwrap(),
            "1"
        );
    }

    #[test]
    fn lowercase_converts_all_char_catcodes() {
        // tex.web change_case：判据是"字符 token"，与 catcode 无关。
        // 只认 Letter|Other 曾把 expl3 `\char_generate:nn` 查表的 `^^@`
        // 非 Letter/Other 臂改得半残（cat 6 臂残留 char 0 → latex.ltx
        // l.9386 Illegal parameter number 级联源之一）。
        // cat 6 参数符：## 在 general text 扫描存两个 # token，lccode 35→65
        // 转换后 `\if` 按字符码比对命中 A。（cat 13 active char 在本引擎
        // 以 cs 形式表示，属 token 表示层另一刀，不在本判据覆盖内。）
        let cat6 = expand(
            "\\lccode`\\#=65\\relax\\lowercase{\\toks0{##}}\\if A\\the\\toks0 YES\\else NO\\fi",
        )
        .unwrap();
        assert!(cat6.contains("YES") && !cat6.contains("NO"), "{cat6:?}");
    }

    #[test]
    fn mathchardef_binds_cs() {
        // \the\cs 返回十进制数学字符码
        assert_eq!(expand("\\mathchardef\\x=100\\the\\x").unwrap(), "100");
        // \number\cs（数字上下文）
        assert_eq!(expand("\\mathchardef\\x=32767\\number\\x").unwrap(), "32767");
        // \meaning\cs → \mathchar"XXXX（十六进制）
        assert_eq!(expand("\\mathchardef\\x=100\\meaning\\x").unwrap(), "\\mathchar\"64");
        // 越界：报 "! Bad mathchar code." 且不改变绑定（cs 保持未定义）
        let mut e = Expander::new();
        e.run_source("\\mathchardef\\x=-1\\mathchardef\\y=32768\\mathchardef\\z=5\\the\\z")
            .unwrap();
        assert_eq!(e.transcript(), "! Bad mathchar code (-1).\n! Bad mathchar code (32768).\n");
        // 越界不改绑定，合法值仍可用
        assert_eq!(expand("\\mathchardef\\z=5\\the\\z").unwrap(), "5");
    }

    #[test]
    fn catcode_backquote_control_word() {
        // q=letter(11) 时 \qq 是多字符控制词：反引号报 "Improper alphabetic
        // constant" 恢复、q 保持 letter（真实 TeX 同；控制符号语义需先
        // `\catcode`q=7`（TRIP L428）使 \qq 成单字符 cs）。scan_int 在 `\` 处停，
        // 无遗留文本。
        assert_eq!(expand("\\catcode`\\qq1\\the\\catcode`q").unwrap(), "11");
    }

    // ---------- LaTeX 兼容第十二刀：l.398 阻塞点根因链（报告 §18） ----------

    #[test]
    fn lowercase_group_contains_relax_meaning_cs_as_data() {
        // scan_general_text（\lowercase/\write 参数）：`\relax` 终止只限**未进
        // 平衡组**时；组内与 `\relax` 同义的 cs（`\csname` 制造）是普通数据。
        // expl3-code L205 `\lowercase{\endgroup\def\PackageError#1...}` 的
        // `\PackageError`（L199 \csname 刚制造为 relax）曾被截断 → `\def` 后接
        // 字面 `#` → Missing control sequence 级联（l.398 现场，报告 §18）。
        let (r, t) = run_transcript(concat!(
            "\\csname PkgErr\\endcsname ",
            "\\lowercase{\\endgroup\\def\\PkgErr#1#2{ok:#1:#2}} ",
            "\\PkgErr{A}{B}"
        ));
        assert!(r.is_ok(), "应可恢复运行");
        assert!(
            !t.contains("Missing control sequence"),
            "组内 relax 同义 cs 不应截断 general text：{t}"
        );
    }

    #[test]
    fn endline_space_ignored_when_char32_ignored() {
        // expl3 语法（cat 32=9 ignore）：行尾插入的 char-32 被忽略 → 行边界消失。
        // 旧扫描器无条件在行尾制造 cat-10 空格 → `\def\kp#1#2{` 参数文本带尾随
        // 空格，使 #2 变"空格定界"实参、实参扫描一路吞到首个 `}`（l.398 级联的
        // 第三根因，报告 §18）。
        let src = concat!(
            "\\catcode32=9 \\endlinechar=32\n",
            "\\def\\a#1#2{[#1][#2]}\n",
            "\\a X Y"
        );
        assert_eq!(expand(src).unwrap(), "[X][Y]");
    }

    #[test]
    fn patterns_reads_group_and_forwards_text() {
        let text = pattern_run(r"\patterns{.ach4 .ad4 % 注释换行
ab5c}").unwrap();
        // 字母/数字/`.` 保留；空格/% 注释/换行折叠为分隔空格
        assert_eq!(text, b".ach4 .ad4 ab5c");
    }

    #[test]
    fn patterns_multi_and_unclosed_group_errors() {
        let mut e = Expander::new();
        let sink = EventSink::default();
        e.set_sink(Box::new(sink));
        e.run_source(r"\patterns{ab5c xy7z}").unwrap();
        let mut sink = e.take_sink();
        let sink = sink.as_any_mut().downcast_mut::<EventSink>().unwrap();
        assert_eq!(sink.patterns, vec![b"ab5c xy7z".to_vec()]);
        // 未闭合组 → "Runaway text?" 报错恢复继续（真实 TeX INITEX 同）
        let mut e = Expander::new();
        assert!(e.run_source(r"\patterns{ab5c").is_ok());
    }

    #[test]
    fn scantokens_rescans_text_with_current_catcodes() {
        // 组内容 detokenize 后按当前 catcode 重新扫描（等价于从字符串 \input）
        assert_eq!(expand(r"\def\x{abc}\scantokens{\x}").unwrap(), "abc");
        // 扫描过程中定义并展开宏
        assert_eq!(expand(r"\scantokens{a\def\y{b}\y}").unwrap(), "ab");
    }

    /// 刀29（LaTeX 兼容战役）：tex.web scan_glue S=scan_dimen 分支——level=glue_val
    /// 时内部 dimen 转零阶胶水（width=值，stretch/shrink=0，plus/minus 照常可扫）。
    /// expl3 依赖此臂：`\skip_const:Nn \c_zero_skip {\c_zero_dim}`（latex.ltx
    /// l.13899 停点根因）。
    #[test]
    fn glue_scan_accepts_internal_dimen() {
        // 基本转换：\skip1=\dimen0 → width=5pt、零阶
        assert_eq!(
            expand(r"\dimen0=5pt\skip1=\dimen0\the\skip1").unwrap(),
            "5.0pt"
        );
        // 前导负号作用于整个胶水（width 取负，stretch/shrink 本就为 0）
        assert_eq!(
            expand(r"\dimen0=3pt\skip1=-\dimen0\the\skip1").unwrap(),
            "-3.0pt"
        );
        // 内部 dimen 后 plus/minus 照常扫描
        assert_eq!(
            expand(r"\dimen0=1pt\skip2=\dimen0 plus 2pt\the\skip2").unwrap(),
            "1.0pt plus 2.0pt"
        );
        // dimendef 绑定的 cs 同样走 Register(Dimen) 臂
        assert_eq!(
            expand(r"\dimendef\zd=3\dimen3=7pt\skip4=\zd\the\skip4").unwrap(),
            "7.0pt"
        );
    }

    /// 刀29：mu 上下文遇内部 dimen → "Incompatible glue units"（按 1mu=1pt 继续，
    /// tex.web mu 分支同款），赋值成功不中断。
    #[test]
    fn muskip_scan_internal_dimen_incompatible_units() {
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source(r"\dimen0=2pt\muskip0=\dimen0\the\muskip0").unwrap();
        // 按 1mu=1pt 恢复：赋值成功，宽度保留（output 先于 take_sink 取）
        let out: String = e
            .output()
            .iter()
            .map(|t| t.charcode().and_then(char::from_u32).unwrap_or('\u{FFFD}'))
            .collect();
        let mut sink = e.take_sink();
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert!(
            sink.transcript.contains("Incompatible glue units"),
            "mu 上下文遇 dimen 应报 Incompatible glue units：{}",
            sink.transcript
        );
        assert_eq!(out, "2.0mu", "1mu=1pt 恢复口径");
    }

/// `\uppercase`/`\lowercase` 语义锁（tex.web `shift_case` @23609）：
/// 判据 `t < cs_token_flag + single_base` —— **只对单字符 token（char / active
/// char）施表，多字母控制序列一律原样保留**。plain.tex L271
/// `{\uccode`1=`i \uccode`2=`f \uppercase{\gdef\if@if{}}}` 依赖数字字符
/// `1`/`2` 被转成 `i`/`f`（不是动 cs）。
#[cfg(test)]
mod case_convert_semantics {
    use super::*;

    /// plain.tex L271 复现：`\uppercase` 内数字字符按 `\uccode` 转字母，
    /// 使 `\if@if` 名字成立（G4 靶的最小用例）。
    #[test]
    fn uppercase_maps_digits_per_uccode_in_definition_body() {
        let out = expand(
            "\\uccode`1=`i \\uccode`2=`f \\uppercase{\\gdef\\if@if{}}             \\ifx\\if@if\\undefined NO\\else YES\\fi",
        )
        .unwrap();
        assert!(out.contains("YES"), "\\if@if 未定义：{out}");
    }

    /// 多字母控制序列不受施表影响（tex.web：`t >= cs_token_flag + single_base`）。
    #[test]
    fn uppercase_keeps_multiletter_cs() {
        assert_eq!(expand("\\def\\maxdimen{MD}\\uppercase{\\maxdimen}").unwrap(), "MD");
    }

    /// 字符 token 按 `\uccode` 表转换。
    #[test]
    fn uppercase_maps_char_token() {
        assert_eq!(expand("\\uccode`1=`i \\uppercase{1}").unwrap(), "i");
    }

    /// `\lowercase` 同构：字符 token 按 `\lccode` 表转换。
    #[test]
    fn lowercase_maps_char_token() {
        assert_eq!(expand("\\lccode`A=`a \\lowercase{A}").unwrap(), "a");
    }
}
