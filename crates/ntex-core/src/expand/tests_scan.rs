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
            // 平衡形式（真/假支各带 \exp_end: 终结符）：romannumeral 吞掉
            // \exp_end:(0) 产空，\use_i/ii:nn 取 #1=\c_true_bool/#2=\exp_end:
            // → 真支 yield \c_true_bool、假支 yield \c_false_bool
            "\\edef\\ga{\\number\\ifnum1=1\\prg_return_true:\\exp_end:\\c_true_bool\\exp_end:",
            "\\else\\prg_return_false:\\exp_end:\\c_false_bool\\exp_end:\\fi} %\n",
            "\\edef\\gb{\\number\\ifnum1=2\\prg_return_true:\\exp_end:\\c_true_bool\\exp_end:",
            "\\else\\prg_return_false:\\exp_end:\\c_false_bool\\exp_end:\\fi} %\n",
            // fast 臂（`\__prg_p_true:w` 形态）在 pdfTeX 本身就报 Missing
            // number（gi.log 2026-09-14：GC=0\relax \c_true_bool）——通道噪声
            // 不入判据
            "\\immediate\\write16{GA=\\ga,GB=\\gb}"
        );
        let (_r, transcript) = run_transcript(src);
        assert!(
            transcript.contains("GA=1,GB=0"),
            "平衡形式：真支取 \\c_true_bool(1)、假支取 \\c_false_bool(0)：{transcript}"
        );
        assert!(
            !transcript.contains("Missing number") && !transcript.contains("Extra"),
            "真臂时序偏差（假分支宏被展开）：{transcript}"
        );
    }

    /// 第二十六刀：字母常量探测位于数字扫描的可选分支，EOF 只表示“没有
    /// 反引号可读”，不得升级成 fatal。真实墙面是 expl3-code 载入末端的
    /// `\romannumeral`/f 型展开前瞻刚好读到输入边界，旧实现把
    /// `try_scan_backquote` 的探测 EOF 报成“扫描到输入末尾”。
    #[test]
    fn backquote_probe_at_expansion_boundary_eof_is_recoverable() {
        let (r, transcript) = run_transcript("\\def\\n{}\\count0=\\n");
        r.unwrap();
        assert!(
            transcript.contains("Missing number"),
            "EOF 数字扫描仍应走 Missing number 恢复：{transcript}"
        );
        assert!(
            !transcript.contains("扫描到输入末尾"),
            "反引号可选探测不得把 EOF 升级成 fatal：{transcript}"
        );
    }

    #[test]
    fn dangling_backquote_at_eof_is_missing_number_zero() {
        let (r, transcript) = run_transcript("\\count0=`");
        r.unwrap();
        assert!(
            transcript.contains("Missing number"),
            "反引号后 EOF 按 TeX 错误恢复插入 0：{transcript}"
        );
        assert!(
            !transcript.contains("扫描到输入末尾") && !transcript.contains("反引号后缺少字符"),
            "反引号 EOF 不应 fatal：{transcript}"
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
            "macro:->[\\bg ]"
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
        // （`\expandafter` 展开把 `{` 压回，`\string` 随之展开成字符 token）。
        // 赋值目标用 \toks0 而非 \everyjob：真实 TeX 里 \everyjob 不落 toks0，
        // （pdfTeX GT /tmp/gt8/t2 box9 = "x"）
        assert_eq!(
            expand("\\toks0\\expandafter{\\string x}\\the\\toks0").unwrap(),
            "x"
        );
        assert_eq!(
            expand("\\toks0{A}\\toks0\\expandafter{\\the\\toks0 B}\\the\\toks0").unwrap(),
            "AB"
        );
        // 宏 filler 作 `{` 入口：scan_left_brace 的 get_x_token 展开 \f，
        // 体循环按 cur_tok（自然字符 token catcode 1/2）记 unbalance 收口。
        // 注意不能用 `\def\f{\bgroup y\egroup}` 别名定界——tex.web L9368
        // `cur_tok<right_brace_limit` 只对字符 token 生效，cs 别名 \egroup
        // 不收口（pdfTeX 实测 runaway，/tmp/gt8/t4）；真实 TeX 永不终止。
        // `\def\f{{y}}` 体里是自然花括号，pdfTeX GT /tmp/gt8/t8 box12 = "y"。
        assert_eq!(
            expand("\\def\\f{{y}}\\toks0\\f\\the\\toks0").unwrap(),
            "y"
        );
        // spacer 与 \relax 跳过（tex.web L8210 `until (cur_cmd<>spacer)
        // and (cur_cmd<>relax)`）
        assert_eq!(expand("\\toks1=\\relax  {z}\\the\\toks1").unwrap(), "z");
        // toks 寄存器 RHS 复制（tex.web <If the right-hand side is a token
        // parameter or token register>）不受 filler 语义影响
        assert_eq!(
            expand("\\toks1={a}\\toks2=\\toks1\\the\\toks2").unwrap(),
            ""
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
            // 旧期望的 -0.27779 是盒内容 GT 通道 kern 残留；字符流就是 "yes"
            // （pdfTeX GT gtf/gf b11 同值）
            "yes"
        );
        // 寄存器值作字符码：\lccode\count20=0（etrip.tex 91 行）；内部量做
        // 字符码走 scan_int 内部量臂（无数字循环尾），\relax 后照常读回
        // （旧期望的 "-0.27779" 是盒内容 GT 通道 kern 残留；字符流就是 "yes"，
        // pdfTeX GT gtf/gf b11 同族）
        assert_eq!(
            expand("\\count20=65\\lccode\\count20=0\\relax\\ifnum\\lccode`A=0 yes\\else no\\fi").unwrap(),
            "yes"
        );
        // \the 读回（字母常量后跟空格：tex.web @<Scan an optional space@> 把空格
        // 吞掉，\the 在赋值完成后才求值）。选 `C`：预载 lccode`C=99（第十刀
        // tex.web §191 INITEX 初表）≠ 赋值目标 98，读回 "98" 即证赋值已落地。
        assert_eq!(expand("\\lccode`C=`b \\the\\lccode`C").unwrap(), "98");
        // 字母常量后**紧跟** \the：tex.web @<Scan an optional space@> 是
        // `get_x_token; if cur_cmd<>spacer then back_input`——get_x_token 把
        // `\the` 就地展开（convert > max_command），读到的是**赋值前**的旧值
        // （第十刀后 INITEX 预载 lccode`C=99 → 字符流 "99"）；expl3 f 型展开
        // （`\exp:w \exp_end_continue_f:w`）正依赖此"字母常量后继续展开"语义。
        // （旧期望 "0" 是全零初表的假象——tex.web §191 本就预载 lccode[A-Z]=+@'40。）
        assert_eq!(expand("\\lccode`C=`b\\the\\lccode`C").unwrap(), "99");
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
        assert_eq!(expand("\\mathchardef\\x=100\\the\\x").unwrap(), "");
        // \number\cs（数字上下文）：\number 在 mathchardef 值数字循环里被
        // 就地展开，\x 尚未绑定 → Missing number→0 折入 → 327670 越界报
        // "Bad mathchar code"（pdfTeX GT gtf/gf b12：盒空 + 该错误）
        assert_eq!(expand("\\mathchardef\\x=32767\\number\\x").unwrap(), "");
        // \meaning\cs：数字 100 后 \meaning 被循环尾展开，\x 尚未绑定 → \relax
        // 字符流排出（pdfTeX GT a68）；旧期望 \mathchar"64 是绑定完成后的语义
        assert_eq!(expand("\\mathchardef\\x=100\\meaning\\x").unwrap(), "\\relax");
        // 越界：报 "! Bad mathchar code." 且不改变绑定（cs 保持未定义）。
        // 第三个 \mathchardef\z=5 的值数字循环就地展开 \the\z：\z 尚未绑定 →
        // "You can't use `\relax' after \the" + 按零续扫，"0" 折入值 → \z=50
        // （与上行 b12 "Bad mathchar 327670" 同族机制）
        let mut e = Expander::new();
        e.run_source("\\mathchardef\\x=-1\\mathchardef\\y=32768\\mathchardef\\z=5\\the\\z")
            .unwrap();
        assert_eq!(
            e.transcript(),
            "! Bad mathchar code (-1).\n! Bad mathchar code (32768).\n\
             ! You can't use `\\relax' after \\the.\n\
             I'm forgetting what you said and using zero instead.\n\n"
        );
        // 越界不改绑定，合法值仍可用
        assert_eq!(expand("\\mathchardef\\z=5\\the\\z").unwrap(), "");
    }

    #[test]
    fn catcode_backquote_control_word() {
        // q=letter(11) 时 \qq 是多字符控制词：反引号报 "Improper alphabetic
        // constant" 恢复、q 保持 letter（真实 TeX 同；控制符号语义需先
        // `\catcode`q=7`（TRIP L428）使 \qq 成单字符 cs）。scan_int 在 `\` 处停，
        // 无遗留文本。
        assert_eq!(expand("\\catcode`\\qq1\\the\\catcode`q").unwrap(), "");
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
        // 基本转换：\skip1=\dimen0 → width=5pt、零阶。注意 \the 落在寄存器
        // 下标数字循环里被就地展开：\skip1 的 "0"（\the\skip1 旧值首字符）
        // 折入下标、其余 ".0pt" 照排——pdfTeX GT（pe 盒0/gtf 同族）同值，
        // 赋值实际落入下标折入后的寄存器，此处只锁字符流。
        assert_eq!(
            expand(r"\dimen0=5pt\skip1=\dimen0\the\skip1").unwrap(),
            ".0pt"
        );
        // 前导负号作用于整个胶水（width 取负，stretch/shrink 本就为 0）
        assert_eq!(
            expand(r"\dimen0=3pt\skip1=-\dimen0\the\skip1").unwrap(),
            ".0pt"
        );
        // 内部 dimen 后 plus/minus 照常扫描。\the 落在 stretch 尾部
        // scan_optional_space（get_x_token 展开后放回首 token）→ 整串 "0.0pt"
        // 照排（**无**首字符折入：折入只发生在寄存器下标数字循环）
        // —— pdfTeX GT（y.tex P3 盒内 "0.0pt"）。
        assert_eq!(
            expand(r"\dimen0=1pt\skip2=\dimen0 plus 2pt\the\skip2").unwrap(),
            "0.0pt"
        );
        // dimendef 绑定的 cs 同样走 Register(Dimen) 臂。\the 落在 scan_glue 的
        // plus/minus 关键字扫描（tex.web scan_keyword=get_x_token）里被就地
        // 展开 → 打印赋值前旧值 "0.0pt"（pdfTeX GT y.tex P4 盒内同值；cs 即
        // 寄存器本体，无下标数字循环可折）
        assert_eq!(
            expand(r"\dimendef\zd=3\dimen3=7pt\skip4=\zd\the\skip4").unwrap(),
            "0.0pt"
        );
    }

    /// 第二十一刀（count 系数 × dimen 内部量，tex.web scan_glue S=scan_int
    /// 分支 L9094 `if cur_val_level=int_val then scan_dimen(mu,false,true)`）：
    /// countdef'd cs 在胶水语境回 int_val → 落穿 scan_dimen，与后续 dimendef'd
    /// cs 结成乘积进 skip。multirow L171
    /// `\addtolength\multirow@dima{\multirow@cntb\bigstrutjot}` 的承重臂。
    /// GT 真值（TinyTeX pdftex g1.tex 实测）：10pt 基数、系数 2、5pt 内部量
    /// → advance 后 `\the\myskip` = `20.0pt`。此前落硬错误
    /// 「胶水上下文需要 \skip/\muskip 寄存器」 fatal。
    #[test]
    fn advance_skip_with_count_times_dimen_internal() {
        let src = concat!(
            "\\countdef\\mycnt=5 \\mycnt=2 %\n",
            "\\dimendef\\mydim=6 \\mydim=5pt %\n",
            "\\skipdef\\myskip=7 \\myskip=10pt %\n",
            "\\advance\\myskip \\mycnt\\mydim %\n",
            "\\message{A=[\\the\\myskip]}"
        );
        let (r, transcript) = run_transcript(src);
        r.unwrap();
        assert!(
            !transcript.contains("胶水上下文需要"),
            "count 寄存器系数不得落硬错误臂：{transcript}"
        );
        assert!(
            !transcript.contains("Missing number"),
            "countdef'd cs 须被数字臂认作系数：{transcript}"
        );
        assert!(
            transcript.contains("A=[20.0pt]"),
            "GT 期望 10pt+2×5pt=20.0pt：{transcript}"
        );
    }

    /// 负系数变体：GT（TinyTeX pdflatex g2.tex，LaTeX `\setlength` 实测）
    /// `\setlength\myskip{-\mycnt\mydim}`（\mycnt=3、\mydim=5pt）→
    /// `-15.0pt`。NTex 单元锁走同语义的直接赋值形态（LaTeX 括号组路径已由
    /// b15 `\setlength\multirow@dima{2\ht\@arstrutbox}` 双侧验证）。
    #[test]
    fn setlength_skip_with_negative_count_factor() {
        let src = concat!(
            "\\countdef\\mycnt=5 \\mycnt=3 %\n",
            "\\dimendef\\mydim=6 \\mydim=5pt %\n",
            "\\skipdef\\myskip=7 %\n",
            "\\myskip=-\\mycnt\\mydim %\n",
            "\\message{D=[\\the\\myskip]}"
        );
        let (r, transcript) = run_transcript(src);
        r.unwrap();
        assert!(
            !transcript.contains("Missing number"),
            "负系数亦不得报 Missing number：{transcript}"
        );
        assert!(
            transcript.contains("D=[-15.0pt]"),
            "GT 期望 -3×5pt=-15.0pt：{transcript}"
        );
    }

    /// multirow 原文场景模拟（GT TinyTeX pdftex g1.tex F 实测）：`\skb=1pt`、
    /// `\cntb=4`、`\bigstrutjot=3pt`，连续两次 `\advance\skb \cntb\jot` →
    /// `25.0pt`（1+12+12）。锁「多轮连续乘积 advance」不串值。
    #[test]
    fn advance_skip_repeated_count_factor_multirow_shape() {
        let src = concat!(
            "\\countdef\\cntb=5 \\cntb=4 %\n",
            "\\dimendef\\jot=6 \\jot=3pt %\n",
            "\\skipdef\\skb=7 \\skb=1pt %\n",
            "\\advance\\skb \\cntb\\jot %\n",
            "\\advance\\skb \\cntb\\jot %\n",
            "\\message{F=[\\the\\skb]}"
        );
        let (r, transcript) = run_transcript(src);
        r.unwrap();
        assert!(
            transcript.contains("F=[25.0pt]"),
            "GT 期望 1pt+2×(4×3pt)=25.0pt：{transcript}"
        );
    }

    /// 第二十一刀：tex.web scan_glue 的 assign_glue 内部量分支。
    /// `\baselineskip` 等胶水参数在胶水上下文须复制完整 glue（三分量），
    /// 不能退化为 scan_dimen 的 width。LaTeX `\set@fontsize` 会执行
    /// `\baselineskip\f@linespread\baselineskip`，size10.clo 随后又有
    /// `\belowdisplayskip\abovedisplayskip`。
    #[test]
    fn glue_scan_accepts_internal_glue_params() {
        let src = concat!(
            "\\baselineskip=12pt plus 2pt minus 3pt ",
            "\\skip0=\\baselineskip ",
            "\\message{B=\\the\\skip0} ",
            "\\abovedisplayskip=10pt plus 2pt minus 5pt ",
            "\\belowdisplayskip\\abovedisplayskip ",
            "\\message{D=\\the\\belowdisplayskip}"
        );
        let (r, transcript) = run_transcript(src);
        r.unwrap();
        assert!(
            !transcript.contains("Missing number"),
            "胶水参数作为内部胶水量不应报 Missing number：{transcript}"
        );
        assert!(
            transcript.contains("B=12.0pt plus 2.0pt minus 3.0pt"),
            "baselineskip 应完整复制到 skip 寄存器：{transcript}"
        );
        assert!(
            transcript.contains("D=10.0pt plus 2.0pt minus 5.0pt"),
            "胶水参数互赋值应保留 stretch/shrink：{transcript}"
        );
    }

    /// 第二十一刀：scan_dimen 的 `<factor><internal dimen>` 分支也要认内部参数。
    /// LaTeX `\set@fontsize` 会把 `\f@linespread=1` 乘到 `\baselineskip` 上，
    /// 实际 token 形态是 `\baselineskip 1\baselineskip`。
    #[test]
    fn dimen_scan_multiplies_internal_glue_param_width() {
        let src = concat!(
            "\\baselineskip=12pt plus 2pt minus 3pt ",
            "\\dimen0=1\\baselineskip ",
            "\\message{D=\\the\\dimen0}"
        );
        let (r, transcript) = run_transcript(src);
        r.unwrap();
        assert!(
            !transcript.contains("Missing number"),
            "数量乘胶水参数 width 不应报 Missing number：{transcript}"
        );
        assert!(
            transcript.contains("D=12.0pt"),
            "1\\baselineskip 应取 width 分量：{transcript}"
        );
    }

    /// 第二十一刀：scan_int 读取内部胶水参数时取 width 的 sp 整数。
    /// `size10.clo` 用 `\divide\@tempdima\baselineskip` 把文本高度转成行数。
    #[test]
    fn number_scan_reads_internal_glue_param_width() {
        let src = concat!(
            "\\baselineskip=10pt ",
            "\\dimen0=25pt ",
            "\\divide\\dimen0\\baselineskip ",
            "\\message{D=\\the\\dimen0}"
        );
        let (r, transcript) = run_transcript(src);
        r.unwrap();
        assert!(
            !transcript.contains("Missing number"),
            "胶水参数作为整数因子应取 width：{transcript}"
        );
        assert!(
            transcript.contains("D=0.00003pt"),
            "pdfTeX GT：10pt 在数字上下文为 655360，25pt/655360=0.00003pt：{transcript}"
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
        // 下标数字循环吸收 \the\muskip0 的 "0"（下标不变）、".0mu" 照排
        // ——pdfTeX GT（pe 盒2）同值；宽度保留口径另见 transcript 断言
        assert_eq!(out, ".0mu", "1mu=1pt 恢复口径：\"0\" 折入下标后余 .0mu");
    }

    /// 第十九刀：mu 参数（\thinmuskip 等参数原语形态）在尺寸上下文是内部量——
    /// `\kern+\thinmuskip` 不再报 Missing number；按 GT 报 "Incompatible glue
    /// units." + help 行 "1mu=1pt" 后以 mu 数值当 pt 继续（plain 形态，恒 pt 语境）。
    #[test]
    fn kern_plus_muskip_param_is_internal_dimen_with_incompatible_note() {
        let mut e = Expander::new();
        let kerns = Rc::new(RefCell::new(Vec::new()));
        e.set_sink(Box::new(KernCaptureSink::new(kerns.clone())));
        e.run_source("\\thinmuskip=3mu\\relax").unwrap();
        e.feed_source("X\\kern+\\thinmuskip Y");
        e.run().unwrap();
        let kerns = kerns.take();
        assert_eq!(
            kerns.as_slice(),
            &[196_608],
            "3mu 应按 1mu=1pt 作 3sp 尺寸：{:?}",
            kerns
        );
        assert!(
            e.transcript().contains("Incompatible glue units"),
            "尺寸语境读 mu 参数应报 Incompatible glue units：{}",
            e.transcript()
        );
        assert!(
            e.transcript().contains("1mu=1pt"),
            "应带 help 行：{}",
            e.transcript()
        );
        assert!(
            !e.transcript().contains("Missing number"),
            "缺臂的 Missing number 应消除：{}",
            e.transcript()
        );
    }

    /// 第十九刀：LaTeX `\,` 文本模式链的最小模拟 `\kern+\medmuskip`——
    /// 0 错通排（此前正号 + mu 参数 → "Missing number … +" 级联）。
    #[test]
    fn latex_thinspace_text_mode_no_missing_number() {
        let mut e = Expander::new();
        let kerns = Rc::new(RefCell::new(Vec::new()));
        e.set_sink(Box::new(KernCaptureSink::new(kerns.clone())));
        e.run_source("\\medmuskip=2mu plus1mu X\\kern+\\medmuskip Y").unwrap();
        let kerns = kerns.take();
        assert_eq!(
            kerns.as_slice(),
            &[131_072],
            "2mu → 2sp：{:?}",
            kerns
        );
        assert!(
            !e.transcript().contains("Missing number"),
            "\\kern+\\medmuskip 应 0 错：{}",
            e.transcript()
        );
    }

    /// 记录 kern 事件宽度的 sink（`\kern` 走 CoreSink::kern 事件；VecSink 同款
    /// 结构 + Rc<RefCell> 外泄记录，tests.rs EventSink / tests_io_write
    /// SpecialCaptureSink 的最小组合）。
    #[derive(Debug, Default)]
    struct KernCaptureSink {
        inner: VecSink,
        kerns: Rc<RefCell<Vec<i64>>>,
    }

    impl KernCaptureSink {
        fn new(kerns: Rc<RefCell<Vec<i64>>>) -> Self {
            Self {
                inner: VecSink::default(),
                kerns,
            }
        }
    }

    impl CoreSink for KernCaptureSink {
        fn token(&mut self, tok: Token) -> Result<()> {
            self.inner.token(tok)
        }
        fn transcript(&self) -> &str {
            self.inner.transcript()
        }
        fn tokens(&self) -> &[Token] {
            self.inner.tokens()
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
        fn as_any_ref(&self) -> &dyn std::any::Any {
            &self.kerns
        }
        fn kern(&mut self, width: i64) -> Result<()> {
            self.kerns.borrow_mut().push(width);
            Ok(())
        }
    }

    impl FontSink for KernCaptureSink {}
    impl MathSink for KernCaptureSink {}
    impl BoxSink for KernCaptureSink {}
    impl AlignSink for KernCaptureSink {}
    impl PageSink for KernCaptureSink {}
    impl IoSink for KernCaptureSink {
        fn message(&mut self, text: String) -> Result<()> {
            self.inner.message(text)
        }
        fn show(&mut self, text: String) -> Result<()> {
            self.inner.show(text)
        }
        fn write16(&mut self, text: String) -> Result<()> {
            self.inner.write16(text)
        }
    }
    impl TokenSink for KernCaptureSink {}

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

/// §41：`\uppercase` 内 `\gdef` 的参数文本按 `\uccode` 转换（语义锁）。
///
/// **ground truth（pdfTeX 实测 /tmp/ifxcheck.tex）**：
/// `{\uccode`1=`i \uccode`2=`f \uppercase{\gdef\ifxx12{}}}` 后
/// `\meaning\ifxx` = `macro:if->`——数字字符 `1``2` 被转成 `if` 并**作为
/// 参数文本**（定界符）存入宏定义。
///
/// **修复前的真缺陷在显示层**：`\meaning` 只渲染 `#n` 而丢定界符 → 误报
/// `macro:->`，掩盖了"参数文本其实正确"的实情，导致 §38/§39 把根因误判为
/// `\uppercase` 语义问题。修复 = `meaning_text`/`slot_display` 改渲染
/// `params.text`（tex.web `print_meaning` 的 `token_show(参数文本)`）。
#[cfg(test)]
mod uppercase_param_text {
    use super::*;

    /// 参数文本 = `if`（非 `#n`）→ `\meaning` 应含 `macro:if->`。
    #[test]
    fn uppercase_gdef_param_text_is_converted() {
        let out = expand(
            "\\uccode`1=`i \\uccode`2=`f              \\uppercase{\\gdef\\ifxx12{}}              \\meaning\\ifxx",
        )
        .unwrap();
        assert_eq!(out.trim(), "macro:if->");
    }

    /// 对照：普通 `#n` 参数文本仍正常渲染（不得被新实现破坏）。
    #[test]
    fn normal_params_still_rendered() {
        assert_eq!(expand("\\def\\a#1#2{#2#1}\\meaning\\a").unwrap(), "macro:#1#2->#2#1");
    }

    /// 对照：定界符 + `#n` 混合（`\def\a,#1;{...}`）。
    #[test]
    fn mixed_delimiter_and_params_rendered() {
        assert_eq!(
            expand("\\def\\a,#1;{#1}\\meaning\\a").unwrap(),
            "macro:,#1;->#1"
        );
    }
}

/// `\string` 对 **active char** 输出裸字符（不补 escapechar 前缀）。
///
/// tex.web `conv_toks`（L9280-9281）：
/// ```pascal
/// string_code: if cur_cs<>0 then sprint_cs(cur_cs) else print_char(cur_chr);
/// ```
/// 判据是 **`cur_cs≠0`**；active char 的 `cur_cs=0` → 走 `print_char` 裸字符路径。
///
/// 本引擎把 active char 编码为「带 `CS_ACTIVE_FLAG` 的 csid token」，`string_token`
/// 曾对 `TokenKind::ControlSeq` **无条件**加 escapechar 前缀 → `\string^^J`
/// 输出 `\<换行>` 而非裸换行。多出的 `\` 污染下游 token 流（latex.ltx L301
/// TeX 版本嗅探 `\expandafter\reserved@a\string^^J\@@` 的首现场错误即此形态）。
#[cfg(test)]
mod string_active_char {
    use super::*;

    /// active char 的 `\string` 产物不含 escapechar 前缀。
    #[test]
    fn string_of_active_char_has_no_escape_prefix() {
        // `^^J` 设为 active 后 `\string^^J` 应输出单个 char（码 10），
        // 而非 `\` + char。判据：产物长度 1 且码为 10。
        // active 化后 `^^J` 会把后续内容也吞作 active char，故用组隔离 + `\relax` 收口
        let out = expand("\\catcode`\\^^J\\active {\\string^^J}\\relax").unwrap();
        assert_eq!(out.chars().count(), 1, "应输出单字符（裸换行）：{out:?}");
        assert_eq!(out.chars().next().unwrap() as u32, 10, "{out:?}");
    }

    /// 对照：普通控制序列的 `\string` **仍带** escapechar 前缀。
    #[test]
    fn string_of_control_seq_keeps_escape_prefix() {
        assert_eq!(expand("\\string\\relax").unwrap(), "\\relax");
    }

    /// 对照：普通字符 token 的 `\string` 输出裸字符。
    #[test]
    fn string_of_char_token_is_bare() {
        assert_eq!(expand("\\string x").unwrap(), "x");
    }
}

/// latex.ltx L299-302 TeX 版本嗅探段端到端（首现场 l.301 的机制级最小复现）。
///
/// ```tex
/// {\catcode`\^^J=\active
///    \def\reserved@a#1#2\@@{\if#1\string^3\fi}
///    \edef\reserved@a{\expandafter\reserved@a\string^^J\@@}
///    \ifx\reserved@a\@empty\else\gdef\@TeXversion{3}\fi}
/// ```
///
/// pdfTeX ground truth（2026-09-12 实测，pdfTeX 3.141592653 -ini，catcode 先行
/// 归位 `{`=1 `}`=2 `#`=6 `^`=7 `@`=11 + `\chardef\active=13`）：整段执行
/// **0 错误**，`\show\reserved@a` → `> \reserved@a=macro:` + `->.`（空宏——
/// `\string^^J`（active）产出裸 char 10 ≠ `\string^` 的 char 94，`\if` 假、
/// 分支被跳过）。
///
/// 修复前本引擎逐物理行尾报 "! Undefined control sequence."（未定义 active
/// char token）：行尾字节按其**可变 catcode** 分派——`\catcode`\^^J=\active`
/// 改写 catcode(0x0A) 后每个行尾被当数据扫成 active char。tex.web get_next
/// 的行尾字节是按位置写入的 `end_line_char`（L7578-7579
/// `buffer[limit]:=end_line_char`），OS 换行字节不进 buffer，故 char 10 的
/// catcode 改写**不可能**触及行尾分派——物理行边界须按字节身份识别。
#[cfg(test)]
mod latex_ltx_l301_sniff {
    use super::*;

    /// `\edef` + `\expandafter\string<active char>`：无 Undefined 错误、
    /// `\reserved@a` 为空宏（与 pdfTeX 逐字节一致）。
    #[test]
    fn l301_sniff_runs_without_undefined_cs() {
        let src = concat!(
            "\\catcode`\\@=11 \\chardef\\active=13 %\n",
            "\\catcode`\\^^J=\\active %\n",
            "\\def\\reserved@a#1#2\\@@{\\if#1\\string^3\\fi}%\n",
            "\\edef\\reserved@a{\\expandafter\\reserved@a\\string^^J\\@@}%\n",
            "\\meaning\\reserved@a"
        );
        let (r, transcript) = run_transcript(src);
        r.unwrap();
        assert!(
            !transcript.contains("Undefined control sequence"),
            "行尾被当数据扫成未定义 active char：{transcript}"
        );
        // `expand` 输出即 `\meaning` 文本：空宏（pdfTeX `->.` 对照）
        assert_eq!(expand(src).unwrap(), "macro:->");
    }

    /// 对照（v_f 探针形态）：`\catcode`\^^J=\active` 之后的多行源码，
    /// 每个物理行尾都不得产出 active char token（修复前 l.4 起逐行报错）。
    #[test]
    fn line_ends_stay_silent_after_active_newline() {
        let src = "\\chardef\\active=13 %\n\\catcode`\\^^J=\\active %\n\\relax\n\\relax\n";
        let (r, transcript) = run_transcript(src);
        r.unwrap();
        assert!(
            !transcript.contains("Undefined control sequence"),
            "行尾泄漏未定义 active char：{transcript}"
        );
    }
}

/// §A1.septies 探针：char 10 token 在宏实参传递中的存活性。
///
/// §A1.septies 探针：可展开原语在**宏实参位置**是否被展开。
///
/// 宏实参扫描**不展开**可展开原语（tex.web `get_token` 语义）。
///
/// pdfTeX ground truth（2026-09-11 实测）：
/// ```tex
/// \def\showit#1{[#1]}
/// \message{[A]}\showit\char65\message{[B]}
/// ```
/// → `[A]` + `! Missing number, treated as zero.` + `[B]`
/// 即 `#1` = `\char` 单 token，`65` 留在外面；`\char` 执行时读不到数字。
///
/// **NTex 已知差异**：不报 `Missing number`，静默产出 `\0` 并把 `65` 留作字面。
/// 归类「错误报告面缺失」（不影响排版结果，影响诊断保真）。
/// 见 docs/archive/latex-feasibility.md §A1.octies。
#[cfg(test)]
mod arg_scan_does_not_expand {
    use super::*;

    /// 实参只吃到 `\char` 单 token（不跨 token 取参数）。
    #[test]
    fn undelimited_arg_takes_only_the_primitive_token() {
        let out = expand("\\def\\showit#1{[#1]}\\showit\\char65").unwrap();
        // 关键：`65` 留在实参**之外**（不作 `\char` 的参数）
        assert!(out.contains("65"), "65 应留在实参外：{out:?}");
        // 已知差异：NTex 静默（pdfTeX 报 Missing number）——此处不断言错误，
        // 只锁「65 未被 `\\char` 消费」这一结构性事实。
    }

    /// 对照：组形式实参把 `\char65` 整体交给 `#1`，`\char` 正常取参。
    #[test]
    fn braced_arg_lets_char_read_its_number() {
        assert_eq!(expand("\\def\\showit#1{[#1]}\\showit{\\char65}").unwrap(), "[A]");
    }

    /// 对照：`\number`/`\romannumeral` 在实参位置同样不展开。
    #[test]
    fn other_expandables_also_not_expanded_in_arg() {
        let n = expand("\\def\\s#1{[#1]}\\s\\number65").unwrap();
        assert!(n.contains("65"), "65 应留在实参外：{n:?}");
        let r = expand("\\def\\s#1{[#1]}\\s\\romannumeral5").unwrap();
        assert!(r.contains('5'), "5 应留在实参外：{r:?}");
    }
}

/// 内部整数读臂：`\hyphenchar`/`\skewchar` 在数字上下文走 tex.web
/// scan_something_internal 的 `@<Fetch a font integer@>`（L8552-8557）——
/// `scan_font_ident` 后取 hyphen_char[f]/skew_char[f]。expl3 intarray 的
/// pdfTeX 模拟里 `\__intarray_count:w` 即字体 `\hyphenchar`，`\number` 读
/// 数组长度全走此臂；缺臂时 `\number\hyphenchar\font` 报
/// `! Missing number, treated as zero.`，expl3-code l.23381
/// `\cctab_const:Nn` 现场级联出 2489× Missing endcsname（本刀震中真根因）。
/// pdfTeX 对照：`\the\hyphenchar\nullfont` → `-`（45 = \defaulthyphenchar）、
/// `\the\skewchar\nullfont` → -1（\defaultskewchar），赋值后读回同源。
#[test]
fn number_scan_hyphenchar_skewchar_font_integer() {
    // 未赋值回退默认（与 `\the` 臂同约定；tex.web 建字体时逐字体初始化 L11210）
    assert_eq!(expand("\\number\\hyphenchar\\nullfont").unwrap(), "45");
    assert_eq!(expand("\\number\\skewchar\\nullfont").unwrap(), "-1");
    // 赋值后数字通道读回（exec_hyphenchar 写、scan_number 读，同键）
    assert_eq!(
        expand("\\hyphenchar\\nullfont=`a \\number\\hyphenchar\\nullfont").unwrap(),
        "97"
    );
    assert_eq!(
        expand("\\skewchar\\nullfont=48 \\number\\skewchar\\nullfont").unwrap(),
        "48"
    );
    // \ifnum 操作数位同臂（expl3 intarray 越界检查惯用法
    // `\if_int_compare:w \__intarray_count:w #1 < ... `）
    assert_eq!(expand("\\ifnum\\skewchar\\nullfont<0 F\\else T\\fi").unwrap(), "F");
    let (_, transcript) = run_transcript("\\ifnum\\skewchar\\nullfont<0 F\\else T\\fi");
    assert!(
        !transcript.contains("Missing number"),
        "数字上下文缺 \\skewchar 臂：{transcript}"
    );
}

/// 刀E：scan_dimen「数量×内部量」乘法的 tex.web 舍入语义。
///
/// tex.web L8930-8951：`⟨factor⟩⟨internal dimen⟩` 的小数先经 round_decimals
/// （L2189-2198：自最低位逐位 `(a+dig·2^17) div 10` 进位、末位 `(a+1) div 2`
/// 正确舍入）打包成 2^16 定点 f，再按 xn_over_d（L2299-2322：32768 分割的
/// 1.5 精度乘法）乘内部量。`.6\p@` = 39322sp（`\the` → "0.6pt"）；此前的
/// `(int·10^k+frac)·q/10^k` 一步整除截成 39321sp（`\the` → "0.59999pt"）
/// ——xcolor `\rshift@`/`\lshift@` 定点小数族的地基偏差（刀E）。
///
/// pdfTeX GT：`\dimen0=0.6\p@ \typeout{\the\dimen0}` → `0.6pt`。
#[test]
fn scan_dimen_quantity_internal_rounding() {
    let pre = "\\catcode`\\@=11 \\dimendef\\p@=11 \\p@=65536sp ";
    assert_eq!(
        expand(&format!("{pre}\\dimen0=0.6\\p@ \\the\\dimen0")).unwrap(),
        "0.6pt"
    );
    // 逐位对照（GT sp：3932 / 32768 / 393216 / 58982）
    assert_eq!(
        expand(&format!("{pre}\\dimen0=0.06\\p@ \\the\\dimen0")).unwrap(),
        "0.06pt"
    );
    assert_eq!(
        expand(&format!("{pre}\\dimen0=0.5\\p@ \\the\\dimen0")).unwrap(),
        "0.5pt"
    );
    assert_eq!(
        expand(&format!("{pre}\\dimen0=6.00\\p@ \\the\\dimen0")).unwrap(),
        "6.0pt"
    );
    assert_eq!(
        expand(&format!("{pre}\\dimen0=0.9\\p@ \\the\\dimen0")).unwrap(),
        "0.9pt"
    );
}

/// 刀E 端到端：xcolor `\rshift\dimen@`（定点小数右移一位）机制级复刻。
///
/// xcolor.sty L410-421：`\def\rshift#1{#1\expandafter\rshift@\the#1}` +
/// catcode-PT/lowercase 定义的 `\rshift@##1.##2PT{\rshift@@##1\relax##2\p@}`。
/// 展开链把 `\expandafter`/`\rshift@@` 落进数字位（tex.web 数字循环
/// get_x_token 语义）：60pt 现场缺展开臂时扫描早断，残流 `0\p@` 泄主输入
/// ——`\p@` 被当赋值目标吞后续展开报 Missing number 且被恢复赋 0pt，
/// 随后 `\dimen@=0.6\p@` 全链归零（LaTeX 层即 "Missing \begin{document}"
/// 误报 + AA-RS2 0.0pt，刀E/E2 同根）。
///
/// pdflatex+真 xcolor GT：`RS1:6.0pt`、`RS2:0.06pt`、零错误。
#[test]
fn xcolor_rshift_delimited_fixedpoint() {
    let src = concat!(
        "\\catcode`\\@=11 \\catcode`\\#=6 %\n",
        "\\begingroup\\catcode`P=12 \\catcode`T=12 %\n",
        "\\lowercase{\\def\\@@tmp{%\n",
        "  \\def\\rshift@##1.##2PT{\\rshift@@##1\\relax##2\\p@}%\n",
        "  \\def\\lshift@##1.##2##3PT{##1##2\\ifnum0##3>\\z@.##3\\fi\\p@}}} %\n",
        "\\expandafter\\endgroup\\@@tmp %\n",
        "\\def\\rshift@@#1#2{\\ifx#2\\relax.#1\\else#1\\expandafter\\rshift@@\\expandafter#2\\fi} %\n",
        "\\def\\rshift#1{#1\\expandafter\\rshift@\\the#1} %\n",
        "\\def\\lshift#1{#1\\expandafter\\lshift@\\the#1} %\n",
        "\\dimendef\\da=0 \\dimendef\\p@=11 \\p@=65536sp %\n",
        "\\countdef\\z@=0 %\n",
        "\\da=60\\p@ \\rshift\\da \\immediate\\write16{RS1:\\the\\da} %\n",
        "\\da=0.6\\p@ \\rshift\\da \\immediate\\write16{RS2:\\the\\da} %\n",
        "\\da=0.6\\p@ \\lshift\\da \\immediate\\write16{LS1:\\the\\da} %\n",
    );
    let (r, transcript) = run_transcript(src);
    if let Err(e) = &r {
        panic!(
            "运行失败 {e}；转录尾：{}",
            &transcript[transcript.len().saturating_sub(600)..]
        );
    }
    assert!(transcript.contains("RS1:6.0pt"), "60pt 右移：{transcript}");
    assert!(transcript.contains("RS2:0.06pt"), "0.6pt 右移：{transcript}");
    assert!(transcript.contains("LS1:6.0pt"), "0.6pt 左移：{transcript}");
    assert!(
        !transcript.contains("Missing number"),
        "数字位展开链缺臂→残流泄主输入（刀E/E2 同根）：{transcript}"
    );
}

/// 刀E2 锁：preamble 里 `\rshift\dimen@` 曾把残流（`0\p@`）泄进主输入——
/// `0` 排版起段（LaTeX `\everypar{\@nodocument}` → "Missing \begin{document}"），
/// `\p@` 被当赋值目标吞 `\typeout` 展开报 Missing number 且恢复赋 0pt，
/// 后续 `0.6\p@` 全链归零（e2t.tex AA-RS2 0.0pt 现场）。
/// 修复后数字循环就地展开展开链，无残流——`\everypar` 哨兵不得触发。
#[test]
fn preamble_rshift_no_paragraph_leak() {
    let src = concat!(
        "\\catcode`\\@=11 \\catcode`\\#=6 %\n",
        "\\def\\sentry{SENTRY-FIRED} %\n",
        "\\everypar{\\sentry} %\n",
        "\\begingroup\\catcode`P=12 \\catcode`T=12 %\n",
        "\\lowercase{\\def\\@@tmp{\\def\\rshift@##1.##2PT{\\rshift@@##1\\relax##2\\p@}}} %\n",
        "\\expandafter\\endgroup\\@@tmp %\n",
        "\\def\\rshift@@#1#2{\\ifx#2\\relax.#1\\else#1\\expandafter\\rshift@@\\expandafter#2\\fi} %\n",
        "\\def\\rshift#1{#1\\expandafter\\rshift@\\the#1} %\n",
        "\\dimendef\\dimen@=0 \\dimendef\\p@=11 \\p@=65536sp %\n",
        "\\dimen@=60\\p@ \\rshift\\dimen@ \\immediate\\write16{RS:\\the\\dimen@} %\n",
        "\\dimen@=0.6\\p@ \\rshift\\dimen@ \\immediate\\write16{RS2:\\the\\dimen@} %\n",
    );
    let (r, transcript) = run_transcript(src);
    r.unwrap();
    assert!(transcript.contains("RS:6.0pt"), "60pt 右移：{transcript}");
    assert!(transcript.contains("RS2:0.06pt"), "0.6pt 右移：{transcript}");
    assert!(
        !transcript.contains("SENTRY-FIRED"),
        "残流泄主输入起段（\\everypar 哨兵触发）＝preamble 误报 Missing \\begin{{document}} 的引擎级机制：{transcript}"
    );
    assert!(
        !transcript.contains("Missing number"),
        "数字位展开链缺臂→\\p@ 被当赋值目标：{transcript}"
    );
}
