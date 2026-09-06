#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::rc::Rc;
    use ntex_io::MemVfs;

    /// 运行源码（**双轨等价**）：字节码与解释器轨道各跑一次并断言输出一致，
    /// 返回字节码轨道结果。全部用例自动覆盖 M2 双轨验证。
    fn expand(src: &str) -> Result<String> {
        let bytecode = expand_track(src, true)?;
        let interp = expand_track(src, false)?;
        assert_eq!(bytecode, interp, "双轨输出不一致：{src}");
        Ok(bytecode)
    }

    /// 取字符串末尾 n 字符（调试用）。
    fn tail(s: &str, n: usize) -> String {
        s.chars().rev().take(n).collect::<String>().chars().rev().collect()
    }

    fn expand_track(src: &str, use_bytecode: bool) -> Result<String> {
        let mut e = if use_bytecode {
            Expander::new()
        } else {
            Expander::new_interpreter()
        };
        e.run_source(src)?;
        Ok(e.output()
            .iter()
            .map(|t| t.charcode().and_then(char::from_u32).unwrap_or('\u{FFFD}'))
            .collect())
    }

    // ─── 原语表三向一致性（枚举 ↔ builtins 注册表 ↔ from_u16 编号）─────────
    // 枚举与 from_u16 由 `define_primitives!` 宏生成；这两个测试锁死"新增原语
    // 必须在 builtins 注册、编号连续无空洞"的契约，防止三处再次脱节。

    /// `from_u16` 编号空间自 1 起连续无空洞，且与 `as_u16` 往返一致。
    #[test]
    fn primitive_numbering_contiguous() {
        let count = Primitive::ALL.len() as u16;
        for v in 1..=count {
            let p = Primitive::from_u16(v).unwrap_or_else(|| panic!("编号 {v} 无对应变体（空洞）"));
            assert_eq!(p.as_u16(), v, "from_u16({v}) 与 as_u16 往返不一致");
        }
        // 0 与越界值必须为 None（0 是 eqtb 槽的 Undefined 哨兵）
        assert!(Primitive::from_u16(0).is_none());
        assert!(Primitive::from_u16(count + 1).is_none());
    }

    /// `BUILTINS` 注册表覆盖全部变体、名字唯一；别名只允许出现在值侧
    /// （如 `\muexpr` → `Glueexpr`，故注册项数可多于变体数）。
    #[test]
    fn primitive_builtins_cover_enum() {
        let mut names: HashSet<&str> = HashSet::new();
        let mut seen: Vec<Primitive> = Vec::new();
        for (name, prim) in BUILTINS {
            assert!(names.insert(name), "内建名字重复：\\{name}");
            assert_eq!(
                Primitive::from_u16(prim.as_u16()),
                Some(prim),
                "内建 \\{name} 的编号不在枚举编号空间内"
            );
            if !seen.contains(&prim) {
                seen.push(prim);
            }
        }
        for p in Primitive::ALL {
            assert!(seen.contains(p), "变体 {p:?} 未注册内建名字");
        }
        assert!(
            BUILTINS.len() >= Primitive::ALL.len(),
            "注册表少于变体数：{} < {}",
            BUILTINS.len(),
            Primitive::ALL.len()
        );
    }

    #[test]
    fn simple_def_and_use() {
        assert_eq!(expand("\\def\\foo{Hello}\\foo").unwrap(), "Hello");
    }

    #[test]
    fn macro_with_argument() {
        assert_eq!(
            expand("\\def\\greet#1{Hi #1!}\\greet{World}").unwrap(),
            "Hi World!"
        );
    }

    #[test]
    fn macro_with_two_arguments() {
        assert_eq!(
            expand("\\def\\pair#1#2{[#1:#2]}\\pair{x}{y}").unwrap(),
            "[x:y]"
        );
    }

    #[test]
    fn edef_expands_at_definition() {
        assert_eq!(expand("\\def\\a{1}\\edef\\y{\\a2}\\y").unwrap(), "12");
    }

    #[test]
    fn edef_keeps_primitive_tokens() {
        // TeX expand() 语义：\edef 不执行不可展开原语，保留在宏体
        assert_eq!(expand("\\edef\\x{\\def\\y{Z}\\y}\\x").unwrap(), "Z");
        // 保留组定界（花括号是宏体结构的一部分）
        assert_eq!(
            expand("\\edef\\x{a{b}}\\expandafter\\detokenize\\expandafter{\\x}").unwrap(),
            "a{b}"
        );
        // 未定义 cs 保留不报错
        assert_eq!(
            expand("\\edef\\x{a\\undefinedcs}\\expandafter\\detokenize\\expandafter{\\x}").unwrap(),
            "a\\undefinedcs "
        );
    }

    #[test]
    fn number_primitive_expands() {
        assert_eq!(expand("\\count0=5\\number\\count0").unwrap(), "5");
        // \edef 中 \number 展开（ETRIP \def\2{\number\eTeXversion\eTeXrevision} 模式）
        assert_eq!(expand("\\count0=5\\edef\\x{\\number\\count0}\\x").unwrap(), "5");
        assert_eq!(expand("\\number-7").unwrap(), "-7");
    }

    #[test]
    fn bgroup_egroup_char_aliases_open_groups() {
        // \let\bgroup={ \let\egroup=}：等价于组定界字符
        assert_eq!(expand("\\let\\bgroup={\\let\\egroup=}\\bgroup a\\egroup").unwrap(), "a");
    }

    #[test]
    fn etrip_version_macro_idiom() {
        // etrip.tex 29-34 行：分隔参数 + \edef/\noexpand 宏重写 + \number
        let src = "\\def\\etripversion{2.6}\
                   \\let\\bgroup={\\let\\egroup=}\
                   \\def\\1.#1#2\\relax{\\bgroup\
                     \\edef\\1{\\egroup\
                       \\def\\noexpand\\2{\\number\\eTeXversion\\eTeXrevision}\
                       \\def\\noexpand\\1{\\number\\eTeXversion.#1}}\\1}\
                   \\expandafter\\1\\eTeXrevision\\relax\
                   \\message{(You are using e-TeX version/revision \\2)}\
                   \\ifx\\1\\etripversion\\message{(VERSION OK)}\\else\\message{(VERSION MISMATCH)}\\fi";
        let mut e = Expander::new();
        e.run_source(src).unwrap();
        assert_eq!(
            e.transcript(),
            "(You are using e-TeX version/revision 2.6)(VERSION OK)"
        );
    }

    #[test]
    fn let_alias() {
        assert_eq!(expand("\\def\\a{XY}\\let\\b\\a\\b").unwrap(), "XY");
    }

    #[test]
    fn outer_prefix_parses() {
        assert_eq!(expand("\\outer\\def\\foo{XY}\\foo").unwrap(), "XY");
        assert_eq!(expand("\\outer\\gdef\\foo{XY}\\foo").unwrap(), "XY");
        assert_eq!(expand("\\outer\\global\\edef\\foo{XY}\\foo").unwrap(), "XY");
    }

    #[test]
    fn xdef_expands_body_at_definition() {
        // \xdef ≡ \global\edef：定义时展开宏体
        assert_eq!(expand("\\def\\a{1}\\xdef\\b{\\a2}\\def\\a{3}\\b").unwrap(), "12");
        // 全局：组内 \xdef 在组外可见
        assert_eq!(
            expand("{\\def\\a{1}\\xdef\\b{\\a2}}\\b").unwrap(),
            "12"
        );
    }

    #[test]
    fn let_primitive_survives_redefinition() {
        // \let\PAR=\par 后 \PAR 应可用（TRIP L404；L418 报 Undefined 是 bug 线索）
        assert_eq!(
            expand("\\let\\PAR=\\par\\PAR x").unwrap(),
            "x",
            "\\PAR 应别名 \\par 原语"
        );
        // TRIP L392-404 组结构：{ 组 + \c}（\c 未定义）+ \let\c\b + 条件 + \let\PAR
        // 参考里 \c} 的 } 闭合 L392 组 → \let\PAR 顶层定义；NTex 曾组级=1 泄漏
        assert_eq!(
            expand("{\\c}\\let\\PAR=\\par\\PAR x").unwrap(),
            "x",
            "最小 c-闭组（未定义）场景 PAR 应可用"
        );
        // \\if 11 后的空格：参考 log 同样输出 {blank space  }（TeX 语义：
        // get_x_token 跳过 \\if 后的空格，但 11 与 A 之间的空格在输入流照常处理）
        assert_eq!(expand(r"{\if11 A\else B\fi}").unwrap(), " A");
        // \\PAR 别名链在条件/组后仍成立（\\ifx 验证）
        for (name, pre) in [
            ("纯条件", r"{\if 11 A\else B\fi}"),
            ("纯组", r"{A}"),
        ] {
            assert!(
                expand(&format!(r"{pre}\let\PAR=\par\ifx\PAR\par yes\else no\fi"))
                    .unwrap()
                    .contains("yes"),
                "{name} 后 PAR 应别名 par"
            );
        }
        let out392 = expand(
            "{\\if 11 \\prevgraf=-1\\if 0123\\error\\else\\relax\\fi\\else\\error\\fi\\c}\\let\\c\\b \\ifx\\a\\ifx.\\else\\error\\fi\\fi\\let\\PAR=\\par\\gdef\\par{\\relax\\PAR}\\PAR x",
        )
        .unwrap();
        // \\if 11 前导空格是 TeX 语义（参考同样输出）；\\PAR 应正常展开为段落结束
        assert!(
            out392.ends_with("x"),
            "L392-404 组结构后 PAR 应可用：{out392:?}"
        );
        // TRIP L397-418 模拟：\def 错误 + \let\c + 条件 + \let\PAR + \gdef\par
        assert_eq!(
            expand(
                "\\def\\a}{\\let\\a\\xyzzy\\csname a\\endcsname}\\def\\a{ab\\par\\c}\\def\\b{ab*\\par\\c}\\let\\c\\b \\ifx\\a\\ifx.\\else\\expandafter\\ifx\\b\\ifinner\\error\\else\\relax\\fi\\else\\error\\fi\\fi\\let\\PAR=\\par\\gdef\\par{\\relax\\PAR}\\PAR x"
            )
            .unwrap(),
            "x",
            "TRIP 序列后 \\PAR 应可用"
        );
        // TRIP L6-10（\par 覆盖+恢复）+ L397-418 序列：\PAR 应可用
        assert_eq!(
            expand(
                "\\let\\paR=\\par\\outer\\xdef\\par{\\catcode`\\%14}\\let\\par=\\paR\\def\\a}{\\let\\a\\xyzzy\\csname a\\endcsname}\\def\\a{ab\\par\\c}\\def\\b{ab*\\par\\c}\\let\\c\\b \\ifx\\a\\ifx.\\else\\expandafter\\ifx\\b\\ifinner\\error\\else\\relax\\fi\\else\\error\\fi\\fi\\let\\PAR=\\par\\gdef\\par{\\relax\\PAR}\\PAR x"
            )
            .unwrap(),
            "x",
            "L6-10 + L397-418 序列后 \\PAR 应可用"
        );
        // trip.tex 第 6-10 行：\let\paR=\par → 重定义 \par → \let\par=\paR 恢复
        assert_eq!(
            expand(
                "\\let\\paR=\\par\\outer\\xdef\\par{\\catcode`\\%14}\\let\\par=\\paR\\relax"
            )
            .unwrap(),
            ""
        );
    }

    #[test]
    fn trip_lines_1_to_19_parse() {
        // trip.tex 第 1-19 行（不含第 20 行 \badness）：验证 \outer/\xdef/\let 恢复后能继续
        let src = "\\immediate\\catcode`{=1\\endlinechar=13\
                   \\catcode`}=2\
                   \\catcode`$=3{\\catcode`$13\\gdef\\dol{$}}\
                   \\catcode`&=4\
                   \\let\\paR=\\par\
                   \\let\\%=\\relax\
                   \\outer\\xdef\\par{\\catcode`\\%14}\
                   \\let\\par=\\paR\\defaulthyphenchar=`-\\defaultskewchar=256\
                   \\ifx\\initex\\undefined\\def\\initex{}\
                   \\catcode`#=6\\catcode`U=\\catcode`#\
                   \\catcode`^=7\\catcode`|=8\
                   \\catcode`~=9\
                   \\catcode`*=10\
                   \\catcode`E=12\
                   \\catcode`\\@=15\
                   \\catcode`^^A=0008\
                   \\catcode`\\^^@=11\\fi\\relax";
        assert_eq!(expand(src).unwrap(), "");
    }

    #[test]
    fn let_to_char() {
        assert_eq!(expand("\\let\\X=x\\X").unwrap(), "x");
    }

    #[test]
    fn expandafter_classic() {
        // 经典：\expandafter\def\expandafter\x\expandafter{\b} 使 \x = \b 的展开。
        // 注：\b 中 \a 与 C 之间的空格在扫描时被控制词吞掉，故为 "ABC"（与真实 TeX 一致）。
        let src =
            "\\def\\a{B}\\def\\b{A\\a C}\\expandafter\\def\\expandafter\\x\\expandafter{\\b}\\x";
        assert_eq!(expand(src).unwrap(), "ABC");
    }

    #[test]
    fn noexpand_defers_expansion() {
        // \edef 时 \noexpand\y 使 \y 保持为 token；\z 使用时 \y 才展开
        let src = "\\def\\y{YY}\\def\\x{A\\noexpand\\y B}\\edef\\z{\\x}\\z";
        assert_eq!(expand(src).unwrap(), "AYYB");
    }

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
            !transcript.contains("Extra \\fi"),
            "\\fi 帧归属错位：{transcript}"
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

    /// 表达式**因子位**的 get_x_token 前瞻：`( <expr> )` 因子臂必须对**展开
    /// 产物**判定（etex.web scan_expr 取 token 循环同为 get_x_token）。expl3
    /// `\int_div_truncate:nn` 让 `\__int_div_truncate:NwNw` 在因子位展开出
    /// `( ... )`；旧实现因子位直读一个 token、非 `(` 即进 scan_int，`(` 落入
    /// 十进制循环 → "Missing number, treated as zero" + "Missing ) inserted"。
    #[test]
    fn expr_factor_expands_before_paren_dispatch() {
        let src = concat!(
            "\\def\\paren{(2+3)} %\n",
            "\\edef\\ga{\\number\\numexpr\\paren*4\\relax} %\n",
            "\\immediate\\write16{GA=\\ga}"
        );
        let (_r, transcript) = run_transcript(src);
        assert!(
            transcript.contains("GA=20"),
            "因子位宏展开产物中的 `(` 应按括号因子处理：{transcript}"
        );
    }

    /// 表达式**运算符前瞻**须跳过空格（`skip_spaces`）——expl3
    /// `\__int_div_truncate:NwNw` 的体含空格：
    /// `( #1#2 ... ( #3#4 - 1 ) / 2 ) / #3#4`。`)` 之后的空格若不跳过，
    /// 乘/除层前瞻把空格当终结符、`/ 2` 残留流中（值 41 + "Missing )"）。
    /// 同时运算符位的 `\if_meaning:w` 条件开始按 get_x_token 就地求值。
    #[test]
    fn expr_operator_peek_skips_spaces_and_evals_cond() {
        // expl3-code l.6652-6673 `\int_div_truncate:nn`/`\__int_div_truncate:NwNw`
        // 机制级复刻（`\__int_sep:` = `\let`，两级别名链同 expl3）。
        let src = concat!(
            "\\catcode`\\_=11 \\catcode`\\:=11 %\n",
            "\\let\\texnumD\\number \\let\\iv\\texnumD %\n",
            "\\let\\texnumE\\numexpr \\let\\ev\\texnumE %\n",
            "\\let\\texlet\\let \\let\\sep\\texlet \\let\\eend\\relax %\n",
            "\\long\\def\\auxb#1#2\\sep#3#4\\sep",
            "{\\if_meaning:w0#1 0\\else:(#1#2\\if_meaning:w-#1+\\else:-\\fi:",
            "(\\if_meaning:w-#3-\\fi:#3#4-1)/2)\\fi:/#3#4} %\n",
            "\\edef\\ga{\\iv\\ev\\expandafter\\auxb \\iv\\ev 140 \\expandafter\\sep ",
            "\\iv\\ev 100 \\sep \\eend} %\n",
            "\\immediate\\write16{GA=\\ga}"
        );
        let (_r, transcript) = run_transcript(src);
        assert!(
            transcript.contains("GA=1"),
            "含空格/内嵌条件的表达式应得 1：{transcript}"
        );
        assert!(
            !transcript.contains("Missing )") && !transcript.contains("Missing number"),
            "表达式在空格/条件处提前收口：{transcript}"
        );
    }

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
    fn expl3_v_variant_macro_value_via_expandafter_ifx() {
        // l3expan `\__exp_eval_register:N` 机制级复刻：`\ifx\noexpand#1#1` 判别
        // "宏还是寄存器"——宏 → `\use_i_ii:nnn` 去掉 `\the`（结果 `\exp_end: <宏>`，
        // 宏就地展开）；寄存器 → 保留 `\the`。修复前宏被误判为寄存器 →
        // `\the<宏>` 报 "You can't use \the with this"，`\str_if_eq_p:Vn` 全族
        // V 变体失效（expl3 sys/prop/bool 模块级联）。
        let src = concat!(
            "\\chardef\\Z=0 ",
            "\\def\\val{pdftex} ",
            "\\def\\useIIi#1#2#3{#1#2} ",
            "\\def\\evalreg#1{\\expandafter\\ifx\\noexpand#1#1\\ifx\\relax#1\\relax ERR\\fi",
            "\\else\\expandafter\\useIIi\\fi\\expandafter\\Z\\the#1} ",
            "\\edef\\tmp{\\evalreg\\val}\\tmp"
        );
        // 宏被就地展开（`\the` 已被摘除）；寄存器臂会报 "You can't use \the
        // with this"（unwrap 捕获）、判别失败会输出 ERR。
        let out = expand(src).unwrap();
        assert!(out.ends_with("pdftex"), "got {out:?}");
        assert!(!out.contains("ERR"), "got {out:?}");
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
    fn trip_opening_line_parses() {
        // trip.tex 第 1 行原文：\immediate\catcode `{ = 1 \endlinechar=13
        assert_eq!(expand("\\immediate\\catcode`{=1\\endlinechar=13").unwrap(), "");
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
    fn muskip_params_assign_and_the() {
        // etrip.tex 78-80 行：mu 胶量参数（1mu = 65536 单位）
        // 整数 mu 显示精确（18mu → 1179648/65536 = 18.0mu）；小数用可精确表示的值
        assert_eq!(
            expand("\\thinmuskip=18mu\\the\\thinmuskip").unwrap(),
            "18.0mu"
        );
        assert_eq!(
            expand("\\medmuskip=27mu plus 9mu minus 18mu\\the\\medmuskip").unwrap(),
            "27.0mu plus 9.0mu minus 18.0mu"
        );
        assert_eq!(
            expand("\\thickmuskip=36mu minus 7.5mu\\the\\thickmuskip").unwrap(),
            "36.0mu minus 7.5mu"
        );
    }

    #[test]
    fn muskip_register_and_muskipdef() {
        // \muskip 寄存器 + \muskipdef cs 绑定（fil/fill 无限单位属另一特性，此处用普通单位）
        assert_eq!(
            expand("\\muskip5=2.5mu plus 1mu\\the\\muskip5").unwrap(),
            "2.5mu plus 1.0mu"
        );
        assert_eq!(
            expand("\\muskipdef\\M=7\\muskip\\M=3mu minus 2mu\\the\\muskip7").unwrap(),
            "3.0mu minus 2.0mu"
        );
        // 组作用域回滚
        assert_eq!(
            expand("\\muskip9=1mu{\\muskip9=9mu}\\the\\muskip9").unwrap(),
            "1.0mu"
        );
    }

    #[test]
    fn dimendef_cs_as_dimen_value() {
        // tex.web scan_dimen `<internal dimen>`：dimendef'd cs 作尺寸值，无需单位
        // （latex.ltx L532 `\boxmaxdepth=\maxdimen` 的最小复现）
        assert_eq!(
            expand("\\dimendef\\m=10 \\m=100pt \\hsize=\\m\\the\\hsize").unwrap(),
            "100.0pt"
        );
        // 值可负；前置 `-` 号与负值合成（tex.web `if cur_val<0` 翻转 negative）
        assert_eq!(
            expand("\\dimendef\\m=10 \\m=-3pt \\dimen20=-\\m\\the\\dimen20").unwrap(),
            "3.0pt"
        );
        // `<factor><internal dimen>`（乘子臂）不受影响：11×5pt
        assert_eq!(
            expand("\\dimendef\\m=10 \\m=5pt \\dimen21=11\\m \\the\\dimen21").unwrap(),
            "55.0pt"
        );
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
    fn advance_register_arithmetic() {
        // etrip.tex 91 行惯用法：\count20=0 \advance\count20 1
        assert_eq!(
            expand("\\count20=0\\advance\\count20 1\\advance\\count20 1\\the\\count20").unwrap(),
            "2"
        );
        // 负数增量
        assert_eq!(
            expand("\\count20=10\\advance\\count20 -3\\the\\count20").unwrap(),
            "7"
        );
        // \countdef 绑定 + 内部整数参数
        assert_eq!(
            expand("\\countdef\\C=5\\count\\C=3\\advance\\C 4\\the\\count5").unwrap(),
            "7"
        );
        assert_eq!(
            expand("\\tracingstats=1\\advance\\tracingstats 2\\the\\tracingstats").unwrap(),
            "3"
        );
        // \dimen 与 \skip 增量
        assert_eq!(
            expand("\\dimen0=1pt\\advance\\dimen0 2.5pt\\the\\dimen0").unwrap(),
            "3.5pt"
        );
        assert_eq!(
            expand("\\skip0=1pt plus 2pt\\advance\\skip0 3pt plus 1pt\\the\\skip0").unwrap(),
            "4.0pt plus 3.0pt"
        );
    }

    #[test]
    fn multiply_divide_register_arithmetic() {
        // ETRIP 惯用法：\multiply/\divide 带可选 by 关键字
        assert_eq!(
            expand("\\count20=5\\multiply\\count20 by3\\the\\count20").unwrap(),
            "15"
        );
        assert_eq!(
            expand("\\count20=15\\divide\\count20 2\\the\\count20").unwrap(),
            "7"
        );
        // \countdef 绑定 + 负数 + 除以 0（TeX：保持不变）
        assert_eq!(
            expand("\\countdef\\C=5\\count\\C=-4\\multiply\\C 2\\the\\count5").unwrap(),
            "-8"
        );
        assert_eq!(
            expand("\\count20=7\\divide\\count20 0\\the\\count20").unwrap(),
            "7"
        );
        // \dimen 与 \skip 标量乘
        assert_eq!(
            expand("\\dimen0=1.5pt\\multiply\\dimen0 2\\the\\dimen0").unwrap(),
            "3.0pt"
        );
        assert_eq!(
            expand("\\skip0=2pt plus 3pt\\multiply\\skip0 2\\the\\skip0").unwrap(),
            "4.0pt plus 6.0pt"
        );
        // 内部整数参数
        assert_eq!(
            expand("\\tracingstats=3\\multiply\\tracingstats 2\\the\\tracingstats").unwrap(),
            "6"
        );
    }

    #[test]
    fn meaning_expands_to_meaning_text() {
        // 宏：macro:->body（无尾随句点）
        assert_eq!(expand("\\def\\x{a}\\meaning\\x").unwrap(), "macro:->a");
        // 原语：\relax
        assert_eq!(expand("\\meaning\\relax").unwrap(), "\\relax");
        // \countdef 绑定：\count0
        assert_eq!(expand("\\countdef\\x=0\\meaning\\x").unwrap(), "\\count0");
        // 未定义 cs：undefined
        assert_eq!(expand("\\meaning\\undefinedcs").unwrap(), "undefined");
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
    fn current_if_readonly_ints() {
        // 无条件：level 0 / type 0 / branch 0
        assert_eq!(
            expand("\\number\\currentiflevel\\number\\currentiftype\\number\\currentifbranch").unwrap(),
            "000"
        );
        // \iftrue 内：level 1、type 15、branch +1
        assert_eq!(
            expand("\\iftrue\\number\\currentiflevel\\number\\currentiftype\\number\\currentifbranch\\fi").unwrap(),
            "1151"
        );
        // \iffalse\else 内：branch -1
        assert_eq!(
            expand("\\iffalse\\else\\number\\currentifbranch\\fi").unwrap(),
            "-1"
        );
        // \unless 取反类型码（\unless\iftrue → type -15）
        assert_eq!(
            expand("\\unless\\iftrue\\else\\number\\currentiftype\\fi").unwrap(),
            "-15"
        );
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

    #[test]
    fn dimen_fraction_rounds_to_nearest_sp() {
        // pdfTeX 实测：3.6pt→235930、0.0001pt→7（四舍五入，非截断）
        assert_eq!(
            expand("\\dimen0=3.6pt\\count0=\\dimen0\\the\\count0").unwrap(),
            "235930"
        );
        assert_eq!(
            expand("\\dimen0=.0001pt\\count0=\\dimen0\\the\\count0").unwrap(),
            "7"
        );
    }

    #[test]
    fn internal_int_params_assign_and_the() {
        // \defaulthyphenchar=`- 与 \defaultskewchar=256；\the 读回
        assert_eq!(
            expand("\\defaulthyphenchar=`-\\the\\defaulthyphenchar").unwrap(),
            "45"
        );
        assert_eq!(
            expand("\\defaultskewchar=256\\the\\defaultskewchar").unwrap(),
            "256"
        );
        assert_eq!(
            expand("\\newlinechar=13\\the\\newlinechar").unwrap(),
            "13"
        );
    }

    #[test]
    fn badness_internal_int() {
        // trip.tex 第 20 行：\catcode `\^^? = \badness（无盒子时 badness = 0）
        assert_eq!(
            expand("\\catcode`\\^^?=\\badness\\ifnum\\catcode`\\^^?=0 yes\\else no\\fi").unwrap(),
            "yes"
        );
        assert_eq!(expand("\\the\\badness").unwrap(), "0");
    }

    #[test]
    fn fontdimen_assignment_and_the() {
        // trip.tex 第 21 行：\fontdimen12\nullfont=13pt；\the 读回
        assert_eq!(
            expand("\\fontdimen12\\nullfont=13pt\\the\\fontdimen12\\nullfont").unwrap(),
            "13.0pt"
        );
        // 组作用域恢复
        assert_eq!(
            expand(
                "\\fontdimen12\\nullfont=13pt{\\fontdimen12\\nullfont=7pt}\\the\\fontdimen12\\nullfont"
            )
            .unwrap(),
            "13.0pt"
        );
        // \global 前缀跨组生效
        assert_eq!(
            expand("{\\global\\fontdimen12\\nullfont=7pt}\\the\\fontdimen12\\nullfont").unwrap(),
            "7.0pt"
        );
        // 无覆盖默认 0
        assert_eq!(expand("\\the\\fontdimen1\\nullfont").unwrap(), "0.0pt");
        // 尺寸上下文读取：\dimen0=\fontdimen12\nullfont
        assert_eq!(
            expand("\\fontdimen12\\nullfont=13pt\\dimen0=\\fontdimen12\\nullfont\\the\\dimen0").unwrap(),
            "13.0pt"
        );
    }

    #[test]
    fn message_show_transcribe() {
        // \message：不换行、可拼接
        let mut e = Expander::new();
        e.run_source("\\message{Hello}\\message{ world}").unwrap();
        assert_eq!(e.transcript(), "Hello world");
        // \message 参数展开宏（控制词后空格被吞）
        let mut e = Expander::new();
        e.run_source("\\def\\x{42}\\message{a\\x b}").unwrap();
        assert_eq!(e.transcript(), "a42b");
        // \message 的宏参数替换（ETRIP \def\stop#1{\message{... #1!}} 模式）
        let mut e = Expander::new();
        e.run_source("\\def\\stop#1{\\message{Emergency stop: #1!}}\\stop{x}")
            .unwrap();
        assert_eq!(e.transcript(), "Emergency stop: x!");
        // \show：meaning 行（带换行）
        let mut e = Expander::new();
        e.run_source("\\show\\relax").unwrap();
        assert_eq!(e.transcript(), "> \\relax=\\relax.\n");
        // \show 未定义
        let mut e = Expander::new();
        e.run_source("\\show\\undefinedcs").unwrap();
        assert_eq!(e.transcript(), "> \\undefinedcs=undefined.\n");
        // \showthe：内部量值
        let mut e = Expander::new();
        e.run_source("\\count0=5\\showthe\\count0").unwrap();
        assert_eq!(e.transcript(), "> \\count=5.\n");
        // \write16：写终端（带换行）；非 immediate 延迟写无 shipout 时不落转录
        //（tex.web：whatsit 在 shipout 才 out_what），故此处用 \immediate
        let mut e = Expander::new();
        e.run_source("\\immediate\\write16{hi}").unwrap();
        assert_eq!(e.transcript(), "hi\n");
    }

    #[test]
    fn active_char_can_be_defined() {
        let src = "\\def~{TILDE}\\def\\x{a~b}\\x";
        assert_eq!(expand(src).unwrap(), "aTILDEb");
    }

    #[test]
    fn undefined_control_sequence_reports_and_recovers() {
        // TeX 错误恢复：未定义 cs 报 "! Undefined control sequence." 并当 \relax 继续
        let mut e = Expander::new();
        e.run_source("\\def\\foo{Hi}\\bar x").unwrap();
        assert!(e.transcript().contains("! Undefined control sequence."));
        assert_eq!(
            e.output()
                .iter()
                .map(|t| t.charcode().and_then(char::from_u32).unwrap_or('?'))
                .collect::<String>(),
            "x"
        );
    }

    #[test]
    fn end_stops_processing() {
        // \end 后内容不再处理
        assert_eq!(
            expand("\\def\\foo{Hi}\\foo\\end\\def\\bar{Bad}\\bar").unwrap(),
            "Hi"
        );
    }

    #[test]
    fn nested_braces_in_argument() {
        // 实参内层花括号在主流层建立组（不输出），与真实 TeX 一致
        assert_eq!(expand("\\def\\wrap#1{[#1]}\\wrap{a{b}c}").unwrap(), "[abc]");
    }

    #[test]
    fn empty_argument() {
        assert_eq!(expand("\\def\\wrap#1{[#1]}\\wrap{}").unwrap(), "[]");
    }

    // ---------- M1-7 扫描顺序原语 ----------

    #[test]
    fn futurelet_captures_next_token() {
        // \futurelet\next\relax a → \next := 字符 'a'（\let 语义），\relax 与 a 照常处理；
        // 用 \ifx 与另一个 \let 到 'a' 的控制序列比较（\ifx CS vs 裸字符必为假）。
        let src =
            "\\futurelet\\next\\relax a\\let\\expected a\\ifx\\next\\expected yes\\else no\\fi";
        assert_eq!(expand(src).unwrap(), "ayes");
    }

    #[test]
    fn aftergroup_inserts_token_at_group_end() {
        let src = "\\def\\X{Z}\\begingroup\\aftergroup\\X\\endgroup";
        assert_eq!(expand(src).unwrap(), "Z");
    }

    #[test]
    fn afterassignment_inserts_token_after_assignment() {
        let src = "\\def\\X{done}\\afterassignment\\X\\count0=5";
        assert_eq!(expand(src).unwrap(), "done");
    }

    #[test]
    fn afterassignment_survives_group_scope() {
        // \afterassignment 触发时已出组，赋值本身组内局部
        let src = "\\count0=1\\def\\X{a}\\begingroup\\afterassignment\\X\\count0=2\\endgroup\\the\\count0";
        assert_eq!(expand(src).unwrap(), "a1");
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
        // `0 T` 泄漏为排版文本（docs/latex-feasibility.md §15.5 最小复现）。
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
    fn expr_terminator_peek_expands_expandable_token() {
        // 表达式终结符位是 **get_x_token** 位置（etex.web scan_expr 运算符循环
        // `get_x_token; if cur_tok<>plus/minus then back_input`）：可展开 token
        // 展开一次后以其产物首 token 判定"运算符/终结符"，终结符放回。
        //
        // expl3 的 `\int_value:w \__int_eval:w <n> \exp_after:wN \__int_sep:`
        // （`\int_step_function:nnnN`、`\__char_generate_aux:w`）依赖这一步：
        // `\__int_sep:`（\let 别名，不可展开）落为终结符、后续 `\int_value:w`
        // 的求值产物排在其后。get_token 直读则 `\expandafter` 原样放回，调用方
        // 的定界实参扫描把 `\expandafter`+下一值整段吞进同一个实参——expl3-code
        // l.9364 `\char_generate:nn` bootstrap 区 1812 条
        // "Missing = inserted for \ifnum"（`\ifnum` 关系符位读到表达式 `+`）。
        //
        // 原语级最小形：\sep = \let 别名（expl3 \__int_sep: 的真身）。
        // 注意：`%` 后必须真实换行（无换行会把其余输入全部注释掉）。
        let src = concat!(
            "\\def\\body#1{[#1]}%\n",
            "\\let\\sep=\\let%\n",
            "\\def\\stepfun #1#2#3{%\n",
            "\\expandafter\\stepw\\number\\numexpr #1 \\expandafter\\sep%\n",
            "\\number\\numexpr #2 \\expandafter\\sep%\n",
            "\\number\\numexpr #3 \\sep}%\n",
            "\\def\\stepw #1\\sep #2\\sep #3\\sep #4{\\stepn >#1\\sep{#2}{#3}{#4}}%\n",
            "\\def\\stepn #1#2\\sep #3#4#5{%\n",
            "\\ifnum #2 #1 #4 \\relax B\\else%\n",
            "#5{#2}%\n",
            "\\expandafter\\stepn\\expandafter#1\\number\\numexpr #2 + #3 \\sep {#3}{#4}{#5}%\n",
            "\\fi}%\n",
            "\\stepfun {0}{1}{3}\\body%\n",
        );
        assert_eq!(expand(src).unwrap(), "[0][1][2][3]B");
        // 同位直写（无 \expandafter）：`\let` 别名原语作终结符，不展开、放回，
        // 交还调用方的定界实参匹配（expl3 `\__int_sep:` 的用法）
        assert_eq!(
            expand("\\def\\g #1\\sep{[#1]}\\expandafter\\g\\number\\numexpr 1 + 1 \\sep").unwrap(),
            "[2]"
        );
        // 终结符是可展开宏：展开后以其产物首 token 判定（tex.web get_x_token）
        assert_eq!(
            expand("\\def\\plus{+2}\\number\\numexpr 1 \\plus \\relax X").unwrap(),
            "3X"
        );
    }

    #[test]
    fn expr_relax_absorbed_once_at_add_level() {
        // `\relax` 只在**加法层**前瞻吸收一次：乘法层若也吸收，加法层的第二次
        // 前瞻就越过表达式终点、把外侧可展开 token（`\the\skip0`）展开吞进
        // 表达式——`\skip0` 赋值正确而 `\the` 产物 `0pt` 泄漏为排版文本。
        assert_eq!(
            expand(r"\skip0=\glueexpr 1pt plus 2pt - 0.5pt \relax\the\skip0").unwrap(),
            "0.5pt plus 2.0pt"
        );
        // 无运算符同形（乘法层前瞻直接落 `\relax`）
        assert_eq!(
            expand(r"\skip0=\glueexpr 1pt \relax\the\skip0").unwrap(),
            "1.0pt"
        );
        assert_eq!(
            expand(r"\count0=\numexpr 1 + 2 \relax\the\count0").unwrap(),
            "3"
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
    fn ifnum_gluestretchorder_repro() {
        // ETRIP etrip.tex L938：\ifnum\gluestretchorder#5=#1
        assert_eq!(
            expand("\\ifnum\\gluestretchorder1ptminus0fil=0 yes\\else no\\fi").unwrap(),
            "yes"
        );
    }

    #[test]
    fn nested_param_in_macro_arg_repro() {
        // ETRIP etrip.tex L1038：\def\1#1{\2{3210#1}}，\11 调用时 #1 应被替换
        // 为实参 1（\2 实参 = 32101），而非保持 macro_param 导致 3210+Param(1)
        // 错位（\ifvbox 读到 Param → Missing number）。
        // 对照：字母宏名（应正常）
        assert_eq!(expand("\\def\\a#1{ARG=#1}\\a{abc}").unwrap(), "ARG=abc");
        // 最小链拆解：\2 单独调用（数字 cs）
        assert_eq!(expand("\\def\\2#1{ARG=#1}\\2{abc}").unwrap(), "ARG=abc");
        // 最小链拆解：\1 单独定义+调用（无嵌套）
        assert_eq!(expand("\\def\\1#1{X#1}\\1{5}").unwrap(), "X5");
        // 直接输出实参内容：期望 ARG=32101（实参 3210 后跟 \1 的实参 1）
        // 注意：% 后必须真实换行（Rust `%\` 续行会把 % 注释吞到 EOF！）
        let src = "\\def\\2#1{ARG=#1}%\n\\def\\1#1{\\2{3210#1}}%\n\\1{5}";
        assert_eq!(expand(src).unwrap(), "ARG=32105");
        // \11 词法：\1 + 数字 1（catcode 12 → 单字符 cs）
        let src2 = "\\def\\2#1{ARG=#1}%\n\\def\\1#1{\\2{3210#1}}%\n\\11";
        assert_eq!(expand(src2).unwrap(), "ARG=32101");
        // e-TeX 大寄存器号（etrip L1039 \setbox32101；e-Trip 扩展 32768 寄存器）：
        // \ifhbox32101 不应 Missing number / Bad register code。
        // 注：VecSink 环境 setbox 是 no-op（box 未设置 → \ifhbox 假、\hbox{X} 输出 X），
        // 这里只断言无 Missing number 报错（真实 sink 由 etrip 段验证）。
        let bigsrc = "\\setbox32101=\\hbox{X}\\ifhbox32101 A\\else B\\fi";
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source(bigsrc).unwrap();
        let sink = e.take_sink();
        let mut sink = sink;
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert!(
            !sink.transcript.contains("Missing number"),
            "大寄存器号不应 Missing number：{}",
            sink.transcript
        );

        // etrip.tex L1020-1050 段复现（box 寄存器测试：\setbox32101 + \11 调用）
        let Some(full) = read_fixture("fixtures/etrip/etrip.tex") else {
            return;
        };
        let lines: Vec<&str> = full.lines().collect();
        let pre = r"\def\typeout{\immediate\write15 }
\def\error#1{\immediate\write15{Bug in your e-TeX implementation!}\immediate\write15 }";
        let body = format!("{}\n", lines[1019..1050].join("\n"));
        let src = format!("{pre}\n{body}\n");
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        // 运行验证不 panic（错误细节由 ETRIP 全量覆盖）
        let _ = e.run_source(&src);
        // 数字 cs 词法：\22000 应为 \2 + 2000（数字 catcode 12 → 单字符 cs）
        assert_eq!(expand("\\def\\2{X}\\22000").unwrap(), "X2000");
        // \the\count2000（原语 \count + 数字，etrip \the\22000 模式）
        assert_eq!(expand("\\count2000=5\\the\\count2000").unwrap(), "5");
        // \write15 内容走转录（VecSink）而非 output；流 15 未打开 → 需 \immediate
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source("\\count2000=5\\immediate\\write15{\\the\\count2000}").unwrap();
        let sink = e.take_sink();
        let mut sink = sink;
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert_eq!(
            sink.transcript, "5\n",
            "\\write15 内 \\the\\count2000 应输出 5：{:?}",
            sink.transcript
        );
        // \advance/\multiply/\divide 目标支持宏别名展开（etrip \edef\2{\csname
        // count\endcsname} 模式：\advance\22000by3 = \advance\count2000by3）
        assert_eq!(
            expand("\\count2000=5\\edef\\2{\\csname count\\endcsname}\\advance\\22000by3\\the\\count2000")
                .unwrap(),
            "8"
        );
        assert_eq!(
            expand("\\count2000=5\\edef\\2{\\csname count\\endcsname}\\multiply\\22000by3\\the\\count2000")
                .unwrap(),
            "15"
        );
        assert_eq!(
            expand("\\count2000=12\\edef\\2{\\csname count\\endcsname}\\divide\\22000by5\\the\\count2000")
                .unwrap(),
            "2"
        );
    }

    #[test]
    fn expr_cross_group_and_glue_preservation() {
        // ── eTeX 表达式跨组（etrip L880-888 运算符优先级段；scan_number/scan_dimen
        // 组处理 + 正号放回 + glueexpr 优先级修复的回归）──
        // \numexpr{1+}{2*3} = 1 + 2*3 = 7（组内运算符放回、组 } 消费）
        assert_eq!(expand("\\the\\numexpr{1+}{2*3}").unwrap(), "7");
        // \glueexpr{7pt+}{12pt/4} = 7pt + 12pt/4 = 10pt（跨组 + 优先级：/ 绑定项）
        assert_eq!(expand("\\the\\glueexpr{7pt+}{12pt/4}").unwrap(), "10.0pt");
        assert_eq!(
            expand("\\ifdim\\glueexpr{7pt+}{12pt/4}=10pt yes\\else no\\fi").unwrap(),
            "yes"
        );
        // \1 宏实参收集场景（etrip L879）：#3={1+} #4={2*3} 组实参展开后表达式求值
        assert_eq!(
            expand("\\def\\1#1#2#3#4{#1#2#3#4=#2#3(#4)\\else X\\fi}\\1\\ifnum\\numexpr{1+}{2*3}")
                .unwrap(),
            ""
        );
        // ── 胶水寄存器 fil 赋值 + \the 读回（\relax 终止单位扫描，避免 \the 被吞）──
        assert_eq!(
            expand("\\skip43=4pt plus 3fil\\relax\\the\\skip43").unwrap(),
            "4.0pt plus 3.0fil"
        );
        // ── 括号表达式 + 前导量（etrip L869-873 表达式段逐项）──
        assert_eq!(
            expand("\\skip43=4pt plus 3fil\\relax\\the\\glueexpr(\\skip43)+3pt").unwrap(),
            "7.0pt plus 3.0fil"
        );
        assert_eq!(
            expand("\\count43=2\\skip43=4pt plus 3fil\\relax\\the\\dimexpr\\skip43+\\count43pt")
                .unwrap(),
            "6.0pt"
        );
        assert_eq!(
            expand(
                "\\count43=2\\skip43=4pt plus 3fil\\relax\\the\\dimexpr(\\skip43)+(\\count43pt)"
            )
            .unwrap(),
            "6.0pt"
        );
        assert_eq!(
            expand("\\count43=2\\skip43=4pt plus 3fil\\relax\\the\\glueexpr\\skip43/\\count43")
                .unwrap(),
            "2.0pt plus 3.0fil"
        );
        assert_eq!(
            expand("\\muskip43=5mu minus 1mu\\relax\\the\\muexpr(\\muskip43)+3muplus1fill")
                .unwrap(),
            "8.0mu plus 1.0fill minus 1.0mu"
        );
        assert_eq!(
            expand("\\count43=2\\skip43=4pt plus 3fil\\relax\\the\\glueexpr\\skip43*2/3")
                .unwrap(),
            "2.66667pt plus 3.0fil"
        );
        // ── 表达式单独用（can't use 测试）后 \let 定义不受污染（etrip L764-766）──
        // \numexpr 等裸用报 "can't use" 不输出 0（修复前输出 "0" 污染）；
        // \let\9=\relax 正常定义 → \ifx 为 T
        {
            let mut e = Expander::new();
            e.set_sink(Box::new(VecSink::default()));
            e.run_source(
                "\\numexpr \\dimexpr \\glueexpr \\muexpr \\let\\9=\\relax \\ifx\\9\\relax T\\else F\\fi",
            )
            .unwrap();
            let sink = e.take_sink();
            let mut sink = sink;
            let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
            assert_eq!(
                sink.transcript.matches("can't use").count(),
                4,
                "四个表达式原语裸用都应报 can't use：{:?}",
                sink.transcript
            );
            // output 在 VecSink.tokens（take_sink 后 e.output() 为空）
            let out: String = sink
                .tokens
                .iter()
                .filter_map(|t| t.charcode().and_then(char::from_u32))
                .collect();
            assert_eq!(out, "T", "\\9 应保持 \\relax 定义（输出 T 而非 0T）");
        }
        // 宏参数 token 的表达式求值：\def\m#1#2{\the\numexpr#1#2}\m{1+}{2*3}
        // #1=[1,+] #2=[2,*,3] → \numexpr 1+ 2*3 = 7（+/* 是宏参数 token）
        let m1 = expand("\\def\\m#1#2{\\the\\numexpr#1#2}\\m{1+}{2*3}");
                assert_eq!(m1.unwrap(), "7");
        // l.880 精确场景：\1\ifnum\numexpr{1+}{2*3}（\1 体含 \else 分支）
        let m2 = expand("\\def\\1#1#2#3#4{#1#2#3#4=#2#3(#4)\\else X\\fi}\\1\\ifnum\\numexpr{1+}{2*3}");
                assert_eq!(m2.unwrap(), "");
        // \noexpand 定义的 \1（etrip L32）是否干扰后续 \def\1 覆盖
        let m3 = expand(
            "\\def\\noexpand\\1{\\number\\eTeXversion.#1}}\\1}\\def\\1#1#2#3#4{OK}\\1 a b c d",
        );
                assert_eq!(m3.unwrap(), "OK");
        // L32 定义 + l.880 场景组合
        let m4 = expand(
            "\\def\\noexpand\\1{\\number\\eTeXversion.#1}}\\1}\\def\\1#1#2#3#4{#1#2#3#4=#2#3(#4)\\else X\\fi}\\1\\ifnum\\numexpr{1+}{2*3}",
        );
                assert_eq!(m4.unwrap(), "");
        // 实参收集诊断：\1 的 #2#3#4 实际 token（\string 序列化）
                        // \def 定义体内含 \else：是否报 Extra \else（l.880 前的 L1595）
                        {
            let mut e6 = Expander::new();
            e6.set_sink(Box::new(VecSink::default()));
            e6.run_source("\\def\\1#1#2#3#4{#1#2#3#4=#2#3(#4)\\else X\\fi}\\relax")
                .unwrap();
            let sink6 = e6.take_sink();
            let mut sink6 = sink6;
            let sink6 = sink6.as_any_mut().downcast_mut::<VecSink>().unwrap();
                        assert!(
                !sink6.transcript.contains("Extra \\else"),
                "\\def 体内 \\else 不应报 Extra：{:?}",
                sink6.transcript
            );
        }
        // 条件栈非空时 \def 体含 \else：是否报 Extra \else（全量 l.880 前状态）
                        {
            let mut e7 = Expander::new();
            e7.set_sink(Box::new(VecSink::default()));
            e7.run_source(
                "\\iftrue\\relax\\def\\1#1#2#3#4{#1#2#3#4=#2#3(#4)\\else X\\fi}\\fi\\relax",
            )
            .unwrap();
            let sink7 = e7.take_sink();
            let mut sink7 = sink7;
            let sink7 = sink7.as_any_mut().downcast_mut::<VecSink>().unwrap();
                        assert!(
                !sink7.transcript.contains("Extra \\else"),
                "条件内 \\def 体 \\else 不应报 Extra：{:?}",
                sink7.transcript
            );
        }
        // 最小复现：\ifnum 的 RHS \numexpr 后直接跟 \else（\1 体模式）
                        {
            let mut e8 = Expander::new();
            e8.set_sink(Box::new(VecSink::default()));
            e8.run_source("\\ifnum0=\\numexpr1+1\\else X\\fi").unwrap();
            let sink8 = e8.take_sink();
            let mut sink8 = sink8;
            let sink8 = sink8.as_any_mut().downcast_mut::<VecSink>().unwrap();
                        assert!(
                !sink8.transcript.contains("Extra \\else"),
                "\\numexpr 后 \\else 不应报 Extra：{:?}",
                sink8.transcript
            );
        }
        // hex 版本：\ifnum0=\numexpr"3FFFFFFF/"7FFFFFFF\else X\fi（\1 体模式）
        {
            let mut e9 = Expander::new();
            e9.set_sink(Box::new(VecSink::default()));
            e9.run_source("\\ifnum0=\\numexpr\"3FFFFFFF/\"7FFFFFFF\\else X\\fi")
                .unwrap();
            let sink9 = e9.take_sink();
            let mut sink9 = sink9;
            let sink9 = sink9.as_any_mut().downcast_mut::<VecSink>().unwrap();
                        assert!(
                !sink9.transcript.contains("Extra \\else"),
                "hex \\numexpr 后 \\else 不应报 Extra：{:?}",
                sink9.transcript
            );
        }
    }

    #[test]
    fn gluestretchorder_delim_macro_repro() {
        // （调用末尾空格是 #5 的定界符，对应 etrip 中行尾换行→空格）
        let src = "\\def\\1#1#2pt#3#4pt#5 {\\ifnum\\gluestretchorder#5=#1 T\\else F\\fi}\\100pt10pt1ptminus0fil ";
        assert_eq!(expand(src).unwrap(), "T");
        // 换行（行尾）作为 #5 定界：真实 etrip 场景
        let src2 = "\\def\\1#1#2pt#3#4pt#5 {\\ifnum\\gluestretchorder#5=#1 T\\else F\\fi}\\100pt10pt1ptminus0fil\n";
        assert_eq!(expand(src2).unwrap(), "T");
    }

    #[test]
    fn etrip_precedence_section_repro() {
        // etrip L866-891 组合段：前置表达式段（\skip44/\muskip44/\dimen44 赋值）
        // + 运算符优先级段（\def\1 + \1\ifnum\numexpr{1+}{2*3} 等）。
        // 全量上下文 l.880 报 "Missing = inserted for \ifnum"（孤立段复现通过）。
        let Some(src) = read_fixture("fixtures/etrip/etrip.tex") else {
            return;
        };
        let lines: Vec<&str> = src.lines().collect();
        // 前置：\typeout/\error/\empty/\space 宏定义（etrip 顶部）
        let pre = r"\def\empty{} \def\space{ }
\def\typeout{\immediate\write15 }
\def\error#1{\immediate\write15{Bug in your e-TeX implementation!}\immediate\write15 }";
        // 前置状态段（L767-773 \count43/\skip43/\muskip43 赋值）+ L866-891 表达式段 + 优先级段
        let body = format!(
            "{}\n{}\n",
            lines[766..773].join("\n"),
            lines[865..891].join("\n")
        );
        // 二分 2：L700-891 全段（含 \numexpr 裸用段 + parshape + \1 定义）
        let body2 = format!("{}\n", lines[765..891].join("\n"));
        let full2 = format!("{pre}\n{body2}\n");
        let mut e2 = Expander::new();
        e2.set_sink(Box::new(VecSink::default()));
        e2.run_source(&full2).unwrap();
        let sink2 = e2.take_sink();
        let mut sink2 = sink2;
        let sink2 = sink2.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert!(
            !sink2.transcript.contains("Missing = inserted"),
            "L766-891 段不应报 Missing = inserted：{:?}",
            sink2.transcript
        );
        let full = format!("{pre}\n{body}\n");
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source(&full).unwrap();
        let sink = e.take_sink();
        let mut sink = sink;
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert!(
            !sink.transcript.contains("Missing = inserted"),
            "l.880 不应报 Missing = inserted：{:?}",
            sink.transcript
        );
    }

    #[test]
    fn countdef_bad_register_code_recovers() {
        // ETRIP L970 稀疏数组：\countdef\1=-1/32768 → "Bad register code" 恢复
        // （不定义、继续），合法值 0/32767 正常绑定。
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source(
            "\\countdef\\1=-1 \\countdef\\1=32768 \\countdef\\1=0 \\countdef\\1=32767 \\relax",
        )
        .unwrap();
        let sink = e.take_sink();
        let mut sink = sink;
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        // read-again 错误块（对齐 etrip.log l.970 参考块）：! 消息 / <to be
        // read again> + token（宏体 #1 再出现的 \countdef）/ l.N 两行光标 /
        // help 2 行 / 空行。
        assert_eq!(
            sink.transcript,
            r#"! Bad register code (-1).
<to be read again> 
                   \countdef
l.1 \countdef\1=-1 \countdef
                            \1=32768 \countdef\1=0 \countdef\1=32767 \relax
A register number must be between 0 and 32767.
I changed this one to zero.

! Bad register code (32768).
<to be read again> 
                   \countdef
l.1 ...ountdef\1=32768 \countdef
                                \1=0 \countdef\1=32767 \relax
A register number must be between 0 and 32767.
I changed this one to zero.

"#,
            "转录：{:?}",
            sink.transcript
        );
        // 越界不改变绑定；合法绑定 \1=count32767 可用
        assert_eq!(
            expand("\\countdef\\1=32767 \\count32767=42 \\the\\1").unwrap(),
            "42"
        );
    }

    #[test]
    fn marks_bad_register_code_recovers() {
        // ETRIP L208 `\marks-1{-1}\marks32768{32768}`：e-TeX marks class 走
        // register-code 语义——越界报 "! Bad register code (N)."（read-again
        // token 为数字后下一 token `{`）+ help 2 行，钳 0 后继续收集 general text
        // （对齐参考 etrip.log l.153-167；此前整块缺失——处理器未做范围检查）。
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source("\\marks-1{-1}\\marks32768{32768}\\marks0{ok}\\marks32767{z} ")
            .unwrap();
        let sink = e.take_sink();
        let mut sink = sink;
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        // 错误块含 ! 消息 / read-again { / l.N 两行 / help 2 行
        assert!(
            sink.transcript.contains(
                "! Bad register code (-1).\n<to be read again> \n                   {\n"
            ),
            "转录：{:?}",
            sink.transcript
        );
        assert!(
            sink.transcript.contains("A register number must be between 0 and 32767.\nI changed this one to zero.\n"),
            "转录：{:?}",
            sink.transcript
        );
        assert!(
            sink.transcript.contains("! Bad register code (32768)."),
            "转录：{:?}",
            sink.transcript
        );
        // 越界次数恰为 2（非法值钳 0，合法 0/32767 不报错），恢复后继续执行不 panic
        assert_eq!(
            sink.transcript.matches("! Bad register code").count(),
            2,
            "转录：{:?}",
            sink.transcript
        );
    }

    #[test]
    fn trip_full_standard_primitives_22() {
        // TRIP 补全批次：tex.web 标准原语 22 个的赋值/查询/展开
        // 日期时间（int 参数四件套）
        assert_eq!(expand("\\year=2026\\the\\year").unwrap(), "2026");
        assert_eq!(expand("\\day=28\\month=8\\time=1200\\the\\day\\the\\month\\the\\time").unwrap(), "2881200");
        // 参数类
        assert_eq!(expand("\\brokenpenalty=77\\the\\brokenpenalty").unwrap(), "77");
        assert_eq!(expand("\\exhyphenpenalty=55\\the\\exhyphenpenalty").unwrap(), "55");
        assert_eq!(expand("\\tracingpages=1\\the\\tracingpages").unwrap(), "1");
        // marks TeX 版（class 0，与 eTeX 版同源）
        assert_eq!(expand("\\topmark\\firstmark\\botmark").unwrap(), "");
        // \skewchar 字体参数（仿 \hyphenchar）
        assert_eq!(expand("\\the\\skewchar\\nullfont").unwrap(), "-1");
        // 只读内部量（无排版状态返回 0）
        assert_eq!(expand("\\the\\displaywidth\\the\\pagedepth").unwrap(), "0.0pt0.0pt");
        // \nullfont 是合法字体标识符（scan_font_ident 接受）
        assert_eq!(expand("\\the\\fontdimen1\\nullfont").unwrap(), "0.0pt");
        // \everydisplay toks 参数
        assert_eq!(
            expand("\\everydisplay={X}\\the\\everydisplay").unwrap(),
            "X"
        );
    }

    #[test]
    fn etrip_full_gluestretchorder_section() {
        // 复现 etrip.tex mutoglue 段（L902-963）+ gluestretchorder 段：
        // 前段复杂表达式（\\2=--\\gluetomu--\\glueexpr(...)）可能污染后续 \\ifnum 扫描
        let Some(src) = read_fixture("fixtures/etrip/etrip.tex") else {
            return;
        };
        let lines: Vec<&str> = src.lines().collect();
        // 二分：gluestretchorder 段逐行扩展，定位产生 wrong glue 的 \1 调用
        let end = std::env::var("GSO_END").ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or(963);
        // 末尾补换行：模拟行尾（\1 宏 #5 参数的空格定界符）
        let body = format!("{}\n", lines[928..end].join("\n"));
        let pre = r"\def\empty{} \def\space{ }
\def\typeout#1{\immediate\write15{#1}}
\def\error#1{\immediate\write15{Bug in your e-TeX implementation!}\immediate\write15 }
\chardef\zero=0\chardef\one=1\chardef\two=2
\countdef\ctmp=255 \countdef\cndx=254
\begingroup
\skip1=\mutoglue1muplus-2muminus-3fil
\muskip1=\gluetomu1ptplus-2ptminus-3fil
\skip2=\mutoglue-4muplus5fillminus6filll
\muskip2=\gluetomu-4ptplus5fillminus6filll
";
        let src = format!("{pre}\n{body}\n");
        // 用 VecSink 捕获 write15 转录，确认 wrong glue 是否在 expand 环境复现
        let mut e = Expander::new();
        let sink = VecSink::default();
        e.set_sink(Box::new(sink));
        if let Err(err) = e.run_source(&src) {
            let sink = e.take_sink();
            let mut sink = sink;
            let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
            eprintln!("transcript tail:\n{}", tail(&sink.transcript, 500));
            panic!("运行失败: {err}");
        }
        let sink = e.take_sink();
        let mut sink = sink;
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        eprintln!("transcript:\n{}", sink.transcript);
        assert!(
            !sink.transcript.contains("wrong glue"),
            "不应有 wrong glue 输出"
        );
    }

    #[test]
    fn glueexpr_muexpr_relax_repro() {
        // etrip.tex L949：\glueexpr\mutoglue\muexpr\gluetomu\skip5\9\9 嵌套表达式
        let src = "\\def\\9{\\relax}\\skip5=1ptminus0fil\\ifnum\\gluestretchorder\\glueexpr\\mutoglue\\muexpr\\gluetomu\\skip5\\9\\9=0 T\\else F\\fi";
        assert_eq!(expand(src).unwrap(), "T");
    }

    #[test]
    fn muskip_order_repro() {
        // etrip.tex L947-948：\muskip5=\gluetomu\skip5 后 \mutoglue\muskip5 应保留胶水阶
        // 注意 \skip5 后必须有空格：无空格时 \ifnum 在 \skip5 寄存器下标扫描中即被
        // 就地求值（tex.web scan_int 的 get_x_token 展开条件；本刀补的十进制循环
        // 条件臂同款）——此刻 \muskip5 尚未赋值（默认 0）→ 恒取 F 分支，非本测试
        // 意图。空格终止数字扫描、赋值完成后主循环才处理 \ifnum（同 etrip 真宏场景）。
        let src = "\\skip5=1ptminus0fil\\muskip5=\\gluetomu\\skip5 \\ifnum\\glueshrinkorder\\mutoglue\\muskip5=1 T\\else F\\fi";
        assert_eq!(expand(src).unwrap(), "T");
        // etrip.tex L948 完整宏场景：\100pt10pt\mutoglue\muskip5
        let src3 = "\\def\\1#1#2pt#3#4pt#5 {\\ifnum\\glueshrinkorder#5=#3 T\\else F\\fi}\\skip5=1ptminus0fil\\muskip5=\\gluetomu\\skip5\\100pt10pt\\mutoglue\\muskip5 ";
        assert_eq!(expand(src3).unwrap(), "T");
    }

    #[test]
    fn mutoglue_negative_chain_keeps_all_components() {
        // etrip.tex L906-915（mutoglue/gluetomu 段）语义锁：负号链取负**整个胶水**、
        // 嵌套 \mutoglue/\gluetomu 各自处理参数符号、stretch/shrink 及 fil 阶保留。
        // 2026-09-03 修复前：负号只作用于宽度（stretch/shrink/阶全丢，
        // `\skip3=-\mutoglue\muskip1` 输出 "-1.0pt" 而非 "-1.0pt plus 2.0pt minus 3.0fil"）。
        // 参考：fixtures/etrip/etrip.log L2721-2774（{into ...} 行逐值一致）。
        let pre = r"\skip1=-\mutoglue-\gluetomu9pt\relax";
        assert_eq!(expand(&format!("{pre}\\the\\skip1")).unwrap(), "9.0pt");
        let pre = r"\muskip1=-\gluetomu-\mutoglue9mu\relax";
        assert_eq!(expand(&format!("{pre}\\the\\muskip1")).unwrap(), "9.0mu");
        // L910-915 赋值链 + 负号引用寄存器（三分量取负、阶保留）
        let src = r"\muskip1=\gluetomu1ptplus-2ptminus-3fil\relax\skip3=-\mutoglue\muskip1\relax\the\skip3";
        assert_eq!(
            expand(src).unwrap(),
            "-1.0pt plus 2.0pt minus 3.0fil",
            "\\skip3=-\\mutoglue\\muskip1：负号应作用于整个胶水"
        );
        let src = r"\skip1=\mutoglue1muplus-2muminus-3fil\relax\muskip3=-\gluetomu\skip1\relax\the\muskip3";
        assert_eq!(
            expand(src).unwrap(),
            "-1.0mu plus 2.0mu minus 3.0fil",
            "\\muskip3=-\\gluetomu\\skip1 对称链"
        );
        // 负号在前导量后（\mutoglue-\muskip2）：-\muskip2 取负后 mutoglue 1:1
        let src = r"\muskip2=-4mu plus 5fill minus 6filll\relax\skip4=\mutoglue-\muskip2\relax\the\skip4";
        assert_eq!(
            expand(src).unwrap(),
            "4.0pt plus -5.0fill minus -6.0filll",
            "\\skip4=\\mutoglue-\\muskip2：内层负号由参数扫描处理"
        );
    }

    #[test]
    fn mutoglue_bare_use_reports_mode_error_without_swallowing_input() {
        // etrip.tex L905：裸 \mutoglue \gluetomu（垂直模式）→ 报 "You can't use..."
        // 且**不扫描参数**（2026-09-03 修复前直接 scan_glue_mu 吞掉后续输入，
        // 污染下一行 `\skip1=-\mutoglue-\gluetomu9pt`，使其结果 0.0pt）。
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source(
            r"\mutoglue \gluetomu\skip1=-\mutoglue-\gluetomu9pt\relax\the\skip1",
        )
        .unwrap();
        let mut sink = e.take_sink();
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        // 独立 Expander 无排版模式（mode 名为 "no mode"）——只断言消息前缀与恢复语义
        let t = sink.transcript.clone();
        assert!(
            t.contains("You can't use `\\mutoglue' in ")
                && t.contains("You can't use `\\gluetomu' in "),
            "裸用应报两个模式错：{t:?}"
        );
        let out: String = sink
            .tokens
            .iter()
            .filter_map(|t| t.charcode().and_then(char::from_u32))
            .collect();
        assert_eq!(
            out, "9.0pt",
            "报错后后续赋值应正常执行（不被吞参）而非输出垃圾"
        );
    }

    #[test]
    fn glue_order_value_semantics() {
        // 必修项② 销账锁（2026-09-03）：\gluestretchorder/\glueshrinkorder 值语义。
        // ① 无阶胶水 → 0；\gluestretch/\glueshrink 返回分量 sp 值（\ifdim 尺寸上下文）
        let src = r"\skip7=2pt plus 3pt minus 4pt\relax";
        assert_eq!(expand(&format!("{src}\\number\\gluestretchorder\\skip7")).unwrap(), "0");
        assert_eq!(expand(&format!("{src}\\number\\glueshrinkorder\\skip7")).unwrap(), "0");
        assert_eq!(
            expand(&format!("{src}\\ifdim\\gluestretch\\skip7=3pt T\\else F\\fi")).unwrap(),
            "T"
        );
        assert_eq!(
            expand(&format!("{src}\\ifdim\\glueshrink\\skip7=4pt T\\else F\\fi")).unwrap(),
            "T"
        );
        // ② 阶：fil=1 fill=2 filll=3；0 分量带阶保留（TeX：阶与分量独立存储）
        let src = r"\skip7=1pt plus 0fill minus 0filll\relax";
        assert_eq!(expand(&format!("{src}\\number\\gluestretchorder\\skip7")).unwrap(), "2");
        assert_eq!(expand(&format!("{src}\\number\\glueshrinkorder\\skip7")).unwrap(), "3");
        assert_eq!(
            expand(&format!("{src}\\ifdim\\gluestretch\\skip7=0pt T\\else F\\fi")).unwrap(),
            "T"
        );
        // ③ 负分量 + 阶：minus -3fil → shrink=-3 且阶 fil（参考 etrip.log
        // {into \muskip1=1.0mu plus -2.0mu minus -3.0fil} 同构）
        let src = r"\skip7=1pt plus -2pt minus -3fil\relax";
        assert_eq!(expand(&format!("{src}\\number\\gluestretchorder\\skip7")).unwrap(), "0");
        assert_eq!(expand(&format!("{src}\\number\\glueshrinkorder\\skip7")).unwrap(), "1");
        assert_eq!(
            expand(&format!("{src}\\ifdim\\gluestretch\\skip7=-2pt T\\else F\\fi")).unwrap(),
            "T"
        );
        // ④ 前导量表达式/寄存器链（\1 宏 etrip L937 场景的等价最小式）
        let src = r"\skip5=1ptminus0fil\muskip5=\gluetomu\skip5\relax";
        assert_eq!(
            expand(&format!("{src}\\number\\glueshrinkorder\\mutoglue\\muskip5")).unwrap(),
            "1"
        );
        assert_eq!(
            expand(&format!("{src}\\ifdim\\glueshrink\\mutoglue\\muskip5=0pt T\\else F\\fi"))
                .unwrap(),
            "T"
        );
        // ⑤ 负号前缀：-\gluestretchorder 对阶取负（int 上下文 negate）
        let src = r"\skip7=1pt plus 2fill\relax";
        assert_eq!(
            expand(&format!("{src}\\number-\\gluestretchorder\\skip7")).unwrap(),
            "-2"
        );
    }

    #[test]
    fn misc_int_param_expands_macro_in_value() {
        // trip.tex L103 `\tracingoutput\on`：内部整数参数赋值后跟**宏**值——
        // TeX scan_int/get_x_token 展开后赋值。2026-09-03 前 \on 未展开直接
        // 当"单独出现 no-op"，\tracingoutput 永不开启（TRIP shipout 转录缺失
        // 根因之一）。同族：\tracingcommands2 直接数字早已可用。
        assert_eq!(
            expand(r"\def\on{1}\tracingoutput\on\the\tracingoutput").unwrap(),
            "1"
        );
        assert_eq!(
            expand(r"\def\two{2}\tracingcommands\two\the\tracingcommands").unwrap(),
            "2"
        );
        // 不可展开 cs 后跟 = 仍正常赋值
        assert_eq!(
            expand(r"\def\x{3}\tracingstats=\x\the\tracingstats").unwrap(),
            "3"
        );
        // 单独出现（无值）仍 no-op 不吞后续（ETRIP $\splitdiscards\noindent 语义）
        assert_eq!(expand(r"\tracingstats A").unwrap(), "A");
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

    // ---------- A4：\outer 语义 ----------

    #[test]
    fn outer_macro_normal_use_ok() {
        // outer 宏在正常展开上下文可用
        assert_eq!(expand("\\outer\\def\\x{A}\\x").unwrap(), "A");
    }

    #[test]
    fn outer_forbidden_in_macro_argument() {
        // outer 宏作为实参 → forbidden（TeX "Forbidden control sequence"）
        let e = expand("\\def\\a#1{#1}\\outer\\def\\x{A}\\a\\x");
        assert!(e.is_err(), "outer 宏作实参应报错");
        let err = e.unwrap_err().to_string();
        assert!(err.contains("forbidden control sequence"), "错误信息：{err}");
        assert!(err.contains("\\x"), "应指明宏名：{err}");
        // 组实参内同样 forbidden
        let e = expand("\\def\\a#1{#1}\\outer\\def\\x{A}\\a{\\x}");
        assert!(e.is_err(), "组实参内 outer 宏应报错");
    }

    #[test]
    fn outer_forbidden_in_edef() {
        // \edef 展开上下文中 outer 宏 → forbidden
        let e = expand("\\outer\\def\\x{A}\\edef\\y{\\x}");
        assert!(e.is_err(), "\\edef 中 outer 宏应报错");
        assert!(e.unwrap_err().to_string().contains("forbidden"));
    }

    #[test]
    fn non_outer_macro_in_argument_ok() {
        // 非 outer 宏作实参正常
        assert_eq!(
            expand("\\def\\a#1{#1}\\def\\x{A}\\a\\x").unwrap(),
            "A"
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

    // ---------- M1-10 寄存器 ----------

    #[test]
    fn count_assignment_and_the() {
        assert_eq!(expand("\\count0=5\\the\\count0").unwrap(), "5");
        assert_eq!(
            expand("\\count0=42\\count1=\\count0\\the\\count1").unwrap(),
            "42"
        );
        assert_eq!(expand("\\count0=-7\\the\\count0").unwrap(), "-7");
    }

    #[test]
    fn dimen_assignment_and_the() {
        assert_eq!(expand("\\dimen0=2.5pt\\the\\dimen0").unwrap(), "2.5pt");
        assert_eq!(expand("\\dimen0=1pt\\the\\dimen0").unwrap(), "1.0pt");
        assert_eq!(expand("\\dimen0=1in\\the\\dimen0").unwrap(), "72.26999pt");
    }

    #[test]
    fn skip_assignment_and_the() {
        assert_eq!(
            expand("\\skip0=1pt plus 2pt minus 0.5pt\\the\\skip0").unwrap(),
            "1.0pt plus 2.0pt minus 0.5pt"
        );
    }

    #[test]
    fn toks_assignment_and_the() {
        assert_eq!(expand("\\toks0={Hi}\\the\\toks0").unwrap(), "Hi");
    }

    #[test]
    fn catcode_backquote_control_word() {
        // q=letter(11) 时 \qq 是多字符控制词：反引号报 "Improper alphabetic
        // constant" 恢复、q 保持 letter（真实 TeX 同；控制符号语义需先
        // `\catcode`q=7`（TRIP L428）使 \qq 成单字符 cs）。scan_int 在 `\` 处停，
        // 无遗留文本。
        assert_eq!(expand("\\catcode`\\qq1\\the\\catcode`q").unwrap(), "11");
    }

    #[test]
    fn toks_register_copy_via_toksdef_cs() {
        // TRIP L418 场景：\tokens 是 \toksdef 绑定的 cs，RHS 为 \toks1 寄存器复制
        assert_eq!(
            expand("\\toksdef\\tokens=256 \\toks1={abc}\\tokens\\toks1\\the\\tokens").unwrap(),
            "abc"
        );
    }

    #[test]
    fn the_in_edef_expands() {
        assert_eq!(
            expand("\\count0=7\\edef\\x{\\the\\count0}\\x").unwrap(),
            "7"
        );
    }

    #[test]
    fn the_meaning_expands_without_recursion() {
        // 回归（fuzz 命中）：\the\meaning 此前 expand_once 缺失 Meaning 分支，\meaning 被原样
        // 保留 → the_tokens_after 无限递归 → 栈溢出（畸形输入不 panic 契约违约）。
        assert_eq!(
            expand("\\the\\meaning\\undefinedcs").unwrap(),
            "undefined"
        );
    }

    #[test]
    fn the_jobname_expands_without_recursion() {
        // 回归（fuzz 命中）：\the\jobname 此前同 \meaning 无限递归。
        assert_eq!(expand("\\the\\jobname").unwrap(), "texput");
    }

    #[test]
    fn expandable_primitives_expand_in_scan_context() {
        // 回归（fuzz 挂死 2026-08-28）：\romannumeral/\char/\uppercase/\lowercase/
        // \endinput/\ignorespaces/\fontname 在 is_expandable() 白名单中但
        // expand_once 无分支 → 扫描循环"可展开 → 展开后重试"展开出自己，
        // 无限空转零消费（\box\muexpr\romannumeral 触发，内存随帧 push 爆涨 OOM）。
        // 断言：数字扫描上下文中可正常展开并消费（此前此输入永久挂死）。
        // （box 场景经 layout 层验证挂死消除——ntex-layout fuzz quick_fragments 全绿；
        // 此处用纯 expand 层断言各原语展开语义正确。）
        assert_eq!(expand("\\romannumeral 14").unwrap(), "xiv");
        assert_eq!(expand("\\romannumeral 0").unwrap(), "");
        assert_eq!(expand("\\char65").unwrap(), "A");
        // \char 在数字扫描上下文：\count0=\char65 → 65（A 输出后被 \count 赋值吞作数字 0）
        assert_eq!(expand("\\count0=\\char65 \\the\\count0").unwrap(), "A0");
        assert_eq!(expand("\\count`A=1 ").unwrap(), "");
    }

    #[test]
    fn romannumeral_expands_to_roman_digits() {
        assert_eq!(expand("\\romannumeral 14").unwrap(), "xiv");
        assert_eq!(expand("\\romannumeral 0").unwrap(), "");
    }

    #[test]
    fn char_expands_to_character_token() {
        assert_eq!(expand("\\char65").unwrap(), "A");
    }

    // ---------- M1-11 组与作用域 ----------

    #[test]
    fn local_def_restored_at_group_end() {
        let src = "\\def\\a{X}\\begingroup\\def\\a{Y}\\a\\endgroup\\a";
        assert_eq!(expand(src).unwrap(), "YX");
    }

    #[test]
    fn global_def_persists() {
        let src = "\\def\\a{X}\\begingroup\\global\\def\\a{Y}\\a\\endgroup\\a";
        assert_eq!(expand(src).unwrap(), "YY");
    }

    #[test]
    fn gdef_is_global() {
        let src = "\\def\\a{X}\\begingroup\\gdef\\a{Y}\\a\\endgroup\\a";
        assert_eq!(expand(src).unwrap(), "YY");
    }

    // ─── unsave 的 retaining 分支（tex.web @<Store save_stack[save_ptr] in
    // eqtb[p], unless eqtb[p] holds a global value@>）────────────────────────
    // 组内"先局部触碰（压 save 条目）、后 \global 赋值"：组末恢复必须跳过该
    // 陈旧条目，保留全局值。expl3 全线依赖：expl3.ltx l.23-30 的
    // `\csname c__kernel_expl_date_tl\endcsname`（局部制造 relax）+
    // `\global\let` 守卫、l.3195 起的 `\exp_args:*` 变体生成器
    // （`\group_begin: … \cs_if_free:cT` 的 c 型 \csname 制造 + `\cs_gset`
    // 全局定义 + `\group_end:`）——此前组末把全局定义回滚成局部触碰前的值，
    // 变体名整体失效（expl3-code l.3202 区 Undefined control sequence 级联，
    // 最终 `\char_set_catcode_group_end:N` 缺失 → `^^@` catcode 复位失败 →
    // regex 替换宏定义体 runaway）。

    #[test]
    fn global_def_survives_group_after_local_touch() {
        // \csname 制造（局部 relax）+ \global\def：组末 retain
        assert_eq!(
            expand(
                "\\begingroup\\expandafter\\let\\csname gz\\endcsname\\relax\\global\\def\\gz{G}\\endgroup\\ifdefined\\gz KEEP\\else LOST\\fi"
            )
            .unwrap(),
            "KEEP"
        );
        // 全局值不被局部触碰前的旧值覆盖
        assert_eq!(
            expand("\\begingroup\\def\\gv{L}\\global\\def\\gv{G}\\endgroup\\gv").unwrap(),
            "G"
        );
    }

    #[test]
    fn global_let_survives_group_after_csname_relax() {
        // expl3.ltx l.23-30 原型（_ 为字母的语法态）：组内 \csname 制造 relax
        // （局部，TeX 2.9）→ \global\let → 组末 retain。注意组外检查须保持
        // _ 为字母，否则 \c__kt 被读作控制词 \c + 字符。
        assert_eq!(
            expand(
                "\\catcode`\\_=11 \\begingroup\\expandafter\\ifx\\csname c__kt\\endcsname\\relax\\global\\let\\c__kt\\relax\\fi\\endgroup\\ifdefined\\c__kt KEEP\\else LOST\\fi"
            )
            .unwrap(),
            "KEEP"
        );
    }

    #[test]
    fn local_after_global_in_group_restores_global_value() {
        // 反向序：组内先 \global 再局部赋值——组末恢复到全局值（tex.web：局部
        // 赋值压栈存全局值；retain 守卫不适用，因局部赋值后层级回到局部）
        assert_eq!(
            expand("\\global\\def\\a{G}\\begingroup\\def\\a{L}\\endgroup\\a").unwrap(),
            "G"
        );
    }

    // ─── \global 前缀与赋值之间的可展开序列（LaTeX 兼容第三刀）─────────────
    // tex.web：前缀只置标志（prefixed_command 的前缀循环），赋值发生在主循环
    // 后续 token——前缀与赋值原语之间可以隔条件、\expandafter 链、宏展开。
    // latex.ltx L488 `\newbox\voidb@x` → `\e@alloc` 的
    // `\global#2#6\allocationnumber`（#2 = `\ifnum…\expandafter\chardef\else…\fi`）
    // 即依赖此语义。

    #[test]
    fn global_prefix_through_conditional_assign_target() {
        // 条件选择的赋值目标（\newbox/\e@alloc 的真实形态）
        assert_eq!(
            expand("\\global\\ifnum1=1\\chardef\\x=3\\else\\chardef\\x=4\\fi\\the\\x").unwrap(),
            "3"
        );
        // else 分支同样到达赋值
        assert_eq!(
            expand("\\countdef\\n=0 \\n=7 \\global\\ifnum\\n>8\\chardef\\x=3\\else\\chardef\\x=4\\fi\\the\\x")
                .unwrap(),
            "4"
        );
        // \global 确实全局：组外可见
        assert_eq!(
            expand("\\begingroup\\global\\ifnum1=1\\count0=5\\else\\relax\\fi\\endgroup\\the\\count0")
                .unwrap(),
            "5"
        );
    }

    #[test]
    fn global_prefix_through_expandable_chain() {
        // \expandafter\chardef\csname…（\e@alloc@chardef 分支的形态）
        assert_eq!(
            expand("\\global\\expandafter\\chardef\\csname y\\endcsname=5\\the\\y").unwrap(),
            "5"
        );
        // \relax 在前缀与赋值之间（tex.web 前缀循环跳过 \relax）
        assert_eq!(expand("\\global\\relax\\chardef\\r=9\\the\\r").unwrap(), "9");
        // 前缀 → 宏展开 → 赋值（TeX：前缀标志不随宏展开丢失）
        assert_eq!(expand("\\def\\z{\\chardef\\q=7}\\global\\z\\the\\q").unwrap(), "7");
    }

    #[test]
    fn expandafer_before_conditional_terminator_keeps_first_token() {
        // `\expandafter` 的第二 token 是 \else：tex.web expand() 的 fi_or_else
        // 处理是急切的（pass_text 消费到配对 \fi 后弹帧），t1 放回时**不得**被
        // 惰性跳过区吞掉——否则 \chardef 消失、赋值目标落空（latex.ltx L488）。
        assert_eq!(
            expand("\\ifnum1=1\\expandafter\\chardef\\else\\relax\\fi\\a 1\\the\\a").unwrap(),
            "1"
        );
        // \edef 展开上下文：\chardef 作为数据进入宏体
        assert_eq!(
            expand("\\edef\\b{\\ifnum1=1\\expandafter\\chardef\\else\\relax\\fi}\\meaning\\b").unwrap(),
            "macro:->\\chardef"
        );
        // 嵌套条件：内层 \else 的急切消费只闭合内层帧，外层分支继续
        assert_eq!(
            expand(
                "\\ifnum1=1\\ifnum1=1\\expandafter\\chardef\\else\\relax\\fi\\a 2\\else\\relax\\fi\\the\\a"
            )
            .unwrap(),
            "2"
        );
    }

    // ─── 实参位置的条件终结符是数据（LaTeX 兼容第十一刀）─────────────────
    // tex.web 宏实参扫描（scan_toks macro=true）用 get_token：不展开、不推进
    // 条件机。`\else`/`\fi`/`\or`/`\if*` 在实参位置一律是数据 token——expl3 的
    // `\__kernel_primitive:NN \else \tex_else:D` 别名表依赖此语义。

    #[test]
    fn arg_cond_terminators_are_data_undelimited() {
        // 无分隔实参：`\else`/`\fi`/`\or` 收进 #1，不交条件机（不报 Extra）
        assert_eq!(
            expand("\\def\\f#1{[\\string#1]}\\edef\\a{\\f\\else\\f\\fi\\f\\or}\\a").unwrap(),
            // tex.web \string 用 sprint_cs（控制序列名后**不**补空格；
            // 补空格是 \detokenize 的 e-TeX 语义）。expl3 cs_to_str 依赖此。
            "[\\else][\\fi][\\or]"
        );
        // 实参扫描不在跳过区里运行：`\f` 的 #1=`\fi` 数据展开后由主循环闭合
        // 外层 \iftrue 帧（`\fi` 在宏体里被消费，`[]x` 全部排出）
        assert_eq!(
            expand("\\def\\f#1{[#1]}\\iftrue\\f\\fi x").unwrap(),
            "[]x"
        );
        // `\if*` 作实参数据（既有行为不回归）：#1=`\iftrue`
        assert_eq!(
            expand("\\def\\f#1{[\\string#1]}\\edef\\a{\\f\\iftrue}\\a").unwrap(),
            "[\\iftrue]"
        );
    }

    #[test]
    fn arg_cond_terminators_are_data_delimited() {
        // 分隔实参：`\else`/`\fi`/`\or` 是数据（不因 arg_cond==0 交条件机）
        assert_eq!(
            expand("\\def\\f#1!{[\\string#1]}\\edef\\a{\\f\\else!\\f\\fi!\\f\\or!}\\a").unwrap(),
            "[\\else][\\fi][\\or]"
        );
        // e-TeX 条件全集（旧白名单缺的 9 个）同样作数据收进实参
        assert_eq!(
            expand(concat!(
                "\\def\\f#1!{[\\string#1]}\\edef\\a{",
                "\\f\\ifeof!\\f\\ifvoid!\\f\\ifhbox!\\f\\ifvbox!\\f\\ifinner!",
                "\\f\\ifvmode!\\f\\ifhmode!\\f\\ifmmode!\\f\\iffontchar!}\\a"
            ))
            .unwrap(),
            "[\\ifeof][\\ifvoid][\\ifhbox][\\ifvbox][\\ifinner][\\ifvmode][\\ifhmode][\\ifmmode][\\iffontchar]"
        );
    }

    #[test]
    fn arg_cond_terminators_build_primitive_alias_expl3() {
        // expl3 `\__kernel_primitive:NN` 语义：`\else` 作 #1 数据被 `\let` 消费，
        // 建立原语别名——别名随后在条件结构里当终结符使用。
        assert_eq!(
            expand(concat!(
                "\\long\\def\\kp#1#2{\\global\\let#2#1}",
                "\\kp\\else\\myelse\\kp\\fi\\myfi\\kp\\or\\myor",
                "\\edef\\n{\\ifcase3\\myor a\\myelse y\\myfi}\\n"
            ))
            .unwrap(),
            "y"
        );
        // 旧注释引用的 `\expandafter\2\fi` 惯用法不经实参扫描：`\fi` 由
        // \expandafter 的展开位置消费，`\2` 的实参从其后取
        assert_eq!(
            expand("\\def\\tw#1{[#1]}\\edef\\a{\\iftrue\\expandafter\\tw\\fi\\iftrue b\\else c\\fi}\\a")
                .unwrap(),
            // #1=\iftrue（数据），在 \edef 的展开位置被重新求值（帧 F2 开启）
            "[]b"
        );
    }

    // ─── 分隔实参的整组贡献（LaTeX 兼容第十三刀：l3prg w 尾参）─────────────
    // tex.web macro_call：定界参数内的 `{…}` 平衡组**整体**作实参数据贡献，
    // 组内 token 只配对、不匹配定界符、`}` 不触发 "extra }"——expl3 的 w 尾参
    // `\tl_if_empty:nF {#8} {…}`（l3prg `\__prg_generate_conditional:NNnnnnNw`
    // 体）即此形。旧实现扁平扫描把组内首个 `}` 当深度 0 的额外 `}` → 报错截断
    // → 尾参被就地执行（undefined-cs 级联 + 栈超限，报告 §19）。

    #[test]
    fn delimited_arg_braced_group_is_data() {
        // 定界实参内容含平衡组：`#1` = A{B}C（组是数据，不报 extra }）
        assert_eq!(
            expand("\\def\\foo#1!{\\detokenize{#1}}\\foo A{B}C!").unwrap(),
            "A{B}C"
        );
        // 组内定界符不终止参数（tex.web 整组贡献）：`#1` 吞到组外 `!`
        assert_eq!(
            expand("\\def\\foo#1!{\\detokenize{#1}}\\foo X{A!B}Y!").unwrap(),
            "X{A!B}Y"
        );
    }

    #[test]
    fn delimited_arg_nested_groups_do_not_error() {
        // 组套组：内层 `}` 依次配对，全部是数据。
        // 实参整体恰为单组 → tex.web Tidy up（m=1）剥外层花括号：`#1` =
        // A{B{C}}D（内层组原样保留）。macro_call 只剥一层、只剥整体单组。
        assert_eq!(
            expand("\\def\\foo#1;{\\detokenize{#1}}\\foo {A{B{C}}D};").unwrap(),
            "A{B{C}}D"
        );
        // 顶层混入其他 token（m≥1）→ 不剥：`\a{x}y!` = `{x}y`
        assert_eq!(
            expand("\\def\\foo#1!{\\detokenize{#1}}\\foo {A}B!").unwrap(),
            "{A}B"
        );
        // 整体单组嵌两层：只剥最外层 → `{A}`
        assert_eq!(
            expand("\\def\\foo#1!{\\detokenize{#1}}\\foo {{A}}!").unwrap(),
            "{A}"
        );
    }

    #[test]
    fn delimited_arg_unknown_cs_in_content_is_data_not_executed() {
        // 尾参数据含"可展开但未定义"的宏（l3prg 场景的 `\tl_if_empty:nF`）：
        // 定界扫描不得就地执行它——undefined-cs 报错即扫描把数据当执行位的表征
        let (r, t) = run_transcript(concat!(
            "\\def\\smark{\\relax}",
            "\\def\\gen#1\\smark{\\detokenize{#1}}",
            "\\gen\\tl_if_empty:nF {p} {oops} \\smark"
        ));
        assert!(r.is_ok(), "应可恢复运行");
        assert!(
            !t.contains("Undefined control sequence"),
            "尾参数据不应被就地执行：{t}"
        );
        assert!(
            !t.contains("has an extra"),
            "组内右花括号不应触发 extra 右花括号 报错：{t}"
        );
    }

    #[test]
    fn delimited_arg_undefined_delimiter_matches_by_token_not_meaning() {
        // 定界符与尾参首 token **都未定义**（expl3 `\s__prg_stop` 定界 +
        // l3tl 未载入时的 `\tl_if_empty:nF` 数据）：定界符按 token 同一匹配，
        // 未定义的 `\tl_if_empty:nF` 不得因"同为未定义"被误作定界符提前终止
        //（tex.web `cur_tok=info(r)` 是 token 相等，非 \ifx 含义相等）。
        assert_eq!(
            expand(concat!(
                "\\def\\gen#1\\smark{\\detokenize{#1}}",
                "\\gen\\notdefinedcs {p} {oops}\\smark"
            ))
            .unwrap(),
            "\\notdefinedcs {p} {oops}"
        );
        // 对照：`\string` 只能看单 token；这里用另一个**已定义**的 cs 开头，
        // 确认定界符仍只认同名 token 出现时才终止
        assert_eq!(
            expand(concat!(
                "\\let\\smarkX\\relax",
                "\\def\\gen#1\\smarkX{\\detokenize{#1}}",
                "\\gen\\relax {mid}\\smarkX"
            ))
            .unwrap(),
            "\\relax {mid}"   // \relax 与 \smarkX 含义同为 relax，但不作定界符
        );
    }

    // ---------- LaTeX 兼容第十五刀：0 参数宏定界串匹配 + \if 操作数展开（报告 §21） ----------

    #[test]
    fn zero_param_macro_delimiter_text_is_matched() {
        // tex.web macro_call（L7971 `if info(r)<>end_match_token`）：参数文本
        // 非空的 **0 参数宏**在调用点匹配纯定界串（"simply scan the delimiter
        // string"）。expl3 条件生成器 fast form `\__prg_F_true:w\fi:\use:n`
        // （expl3-code L1793）依赖它吞掉 `\fi: \use:n` 并由体首 `\fi:` 闭合所在
        // 条件——旧实现直接返回空实参，`\use:n` 泄出被执行，`\cs_if_free:N`
        // 对未定义 cs 误判"已定义" → kernel command-already-defined bail out。
        assert_eq!(
            expand(concat!(
                "\\catcode`\\:=11 \\catcode`\\_=11 ",
                "\\long\\def\\use:none:n#1{} ",
                "\\long\\def\\use:n#1{#1} ",
                "\\def\\prg:Ftrue:w\\fi:\\use:none:n{\\fi:\\use:none:n} ",
                "\\iftrue\\prg:Ftrue:w\\fi:\\use:none:n{LEAK}\\fi OK"
            ))
            .unwrap()
            .trim(),
            "OK"
        );
    }

    #[test]
    fn zero_param_macro_delimiter_mismatch_ignores_call() {
        // 定界串失配 → "Use of macro doesn't match its definition."、调用被忽略，
        // 后续 token 不被吞（tex.web macro_call abort 分支）。
        let (r, t) = run_transcript(concat!(
            "\\def\\delim:X\\fi:\\use:none:n{ body } ",
            "\\delim:X\\relax LEAK"
        ));
        assert!(r.is_ok(), "失配是可恢复错误：{t}");
        assert!(t.contains("doesn't match its definition"), "{t}");
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

    /// 第十六刀原始构造（模拟 expl3 `\__cs_generate_variant_loop_base:N`
    /// idiom，expl3-code L2820-2834）的 ground truth 回归锁。`\loopa` 体内
    /// 第二个 `\fi:` 是多余收口，且嵌套真支 `n` 前的空格才是外层操作数——
    /// 真实 TeX 由此报 3 个可恢复错误（`! Extra \fi` → `! Extra \else` →
    /// `! Extra \fi`）并输出 SAMEDIFF；引擎须与之逐点一致（此前测试臆想的
    /// SAME 期望值即本文件 CI 失败的根因）。
    // ---------- LaTeX 兼容第十四刀：参数文本 `#{` hash_brace 语义（报告 §20） ----------

    #[test]
    fn hash_brace_makes_last_param_brace_delimited() {
        // expl3 p 型签名 `#1#2#3#4#`：参数文本以 `#` 紧接 body `{` 收尾 → 末参
        // #4 是**分隔实参**、定界符 = 字面 `{`（tex.web scan_toks hash_brace）。
        // 调用 `... #1 {rest}`：p-arg #4 = `# 1` 两 token（`{` 作定界符被消费、
        // 不开组），而非旧实现的"无分隔单 token #4 = 单个 `#`"。
        assert_eq!(
            expand(concat!(
                "\\def\\parm:NNNpnn #1#2#3#4#{X\\detokenize{#4}Y}",
                "\\parm:NNNpnn ABC#1{rest}"   // A B C 三个无分隔单 token；无空格（plain
                                              // catcode 下空格是 token，expl3 里是 cat 9）
            ))
            .unwrap(),
            "X#1Yrest"   // #4=`#1`（`{` 作定界符消费）；体尾 hash_brace `{` 与源 `}` 配成组，rest 在组内
        );
    }

    #[test]
    fn hash_brace_keeps_trailing_space_out_of_string() {
        // tex.web `\string` 用 sprint_cs（控制序列名后**不**补空格）；补空格是
        // e-TeX `\detokenize` 语义。expl3 `\cs_to_str:N`/`\cs_split_function:N`
        // 依赖无空格签名——否则条件生成器的 csname 变成含空格的
        // `\cs_if_exist:NTF `（expl3-code L1907 p-arg 生成即坏）。
        assert_eq!(expand("\\edef\\x{\\string\\foo}\\x").unwrap(), "\\foo");
        assert_eq!(
            expand("\\edef\\x{\\detokenize{\\foo}}\\x").unwrap(),
            "\\foo "
        );
        // 对照（字符不受影响）：\string 与 \detokenize 对普通字符输出一致
        assert_eq!(expand("\\edef\\x{\\string a}\\x").unwrap(), "a");
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
    fn expanded_primitive_expands_like_edef() {
        // pdfTeX \expanded{...}：组内容按 \edef 语义全展开（第十二刀新增原语；
        // expl3 L196 引擎门闩与 l3names 别名表要求它存在）
        assert_eq!(expand("\\def\\a{X}\\expanded{\\a Y}").unwrap(), "XY");
        assert_eq!(expand("\\def\\b{42}\\expanded{\\number\\b}").unwrap(), "42");
        assert_eq!(
            expand("\\def\\a{1}\\expanded{\\ifnum\\a=1 yes\\else no\\fi}").unwrap(),
            "yes"
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
    fn csname_terminates_on_endcsname_meaning_not_name() {
        // scan_csname 按**含义**终止（tex.web cur_cmd=end_csname）：expl3 的
        // `\cs_end:` 是 `\endcsname` 别名（槽 = EndCsname 原语），名字不同也应
        // 闭合 `\csname`。旧实现按名 "endcsname" 判定 → `\cs_end:` 报
        // Missing endcsname 级联（l3prg `\use:c{…\cs_end:}` 全炸，报告 §18）。
        let src = concat!(
            "\\catcode58=11 \\catcode95=11 ", // `:`/`_` 变字母 → `\cs_end:` 是单 cs
            "\\let\\cs_end:\\endcsname ",
            "\\expandafter\\ifx\\csname ab\\cs_end:\\relax T\\else F\\fi"
        );
        assert_eq!(expand(src).unwrap(), "T");
        let (r, t) = run_transcript(src);
        assert!(r.is_ok());
        assert!(!t.contains("Missing endcsname"), "\\cs_end: 应闭合 csname：{t}");
        assert!(!t.contains("Extra \\endcsname"), "不应泄漏 \\endcsname：{t}");
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
    fn l3names_primitive_alias_table_header_ok() {
        // l3names 表头最小复现（第十二刀 catcode 归位 + 别名表 + 数项）：
        // 表能建、别名能用（此前 #2 被空格定界 → 实参扫描跑到首个 } 级联）。
        let src = concat!(
            "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode32=9 \\catcode58=11 ",
            "\\catcode95=11 \\endlinechar=32\n",
            "\\let\\tex_global:D\\global\n",
            "\\let\\tex_let:D\\let\n",
            "\\begingroup\n",
            "\\long\\def\\__kernel_primitive:NN #1#2{\\tex_global:D\\tex_let:D#2#1}\n",
            "\\__kernel_primitive:NN\\above\\tex_above:D\n",
            "\\__kernel_primitive:NN\\ifeof\\tex_ifeof:D\n",
            "\\endgroup\n",
            "\\ifx\\tex_ifeof:D\\ifeof OK\\else BAD\\fi"
        );
        assert_eq!(expand(src).unwrap(), "OK");
    }

    #[test]
    fn globaldefs_param_adjusts_assignment_scope() {
        // tex.web prefixed_command：\globaldefs>0 → 所有赋值隐式全局；<0 →
        // 取消显式 \global（局部化）
        assert_eq!(
            expand("\\count0=0\\begingroup\\globaldefs=1 \\count0=5\\endgroup\\the\\count0").unwrap(),
            "5"
        );
        assert_eq!(
            expand("\\count1=0\\begingroup\\globaldefs=-1 \\global\\count1=6\\endgroup\\the\\count1")
                .unwrap(),
            "0"
        );
    }

    #[test]
    fn count_local_and_global_scoping() {
        let local = "\\count0=1\\begingroup\\count0=2\\the\\count0\\endgroup\\the\\count0";
        assert_eq!(expand(local).unwrap(), "21");
        let global = "\\count0=1\\begingroup\\global\\count0=2\\endgroup\\the\\count0";
        assert_eq!(expand(global).unwrap(), "2");
    }

    #[test]
    fn begingroup_endgroup_primitives() {
        let src = "\\def\\a{X}\\begingroup\\def\\a{Y}\\a\\endgroup\\a";
        assert_eq!(expand(src).unwrap(), "YX");
    }

    #[test]
    fn nested_groups() {
        let src = "\\def\\a{X}\\begingroup\\begingroup\\def\\a{1}\\a\\endgroup\\a\\endgroup\\a";
        assert_eq!(expand(src).unwrap(), "1XX");
    }

    #[test]
    fn too_many_end_groups_recovers() {
        // TRIP：多余的 } → "! Too many }'s." 报错恢复（忽略并继续），不终止
        let (r, t) = run_transcript("\\def\\a{X}\\a}");
        assert!(r.is_ok());
        assert!(t.contains("Too many }'s."));
    }

    // ---------- A3：错误上下文行（l.N） ----------

    /// 读取仓库根下 fixture 文件；缺失时跳过测试（CI 未跑 `make fixtures`
    /// 的环境不可 panic——与 ntex-font `parses_real_cmr10` 同款约定：
    /// eprintln 提示 + None，调用方 `let Some(..) = .. else { return }`）。
    fn read_fixture(rel: &str) -> Option<String> {
        let path = format!("{}/../../{}", env!("CARGO_MANIFEST_DIR"), rel);
        match std::fs::read_to_string(&path) {
            Ok(s) => Some(s),
            Err(_) => {
                eprintln!("fixture {rel} 不存在（先 make fixtures），跳过该测试");
                None
            }
        }
    }

    /// 运行源码并返回转录（错误时返回 Err + 已累积转录）。
    fn run_transcript(src: &str) -> (Result<()>, String) {
        let mut e = Expander::new();
        let sink = VecSink::default();
        e.set_sink(Box::new(sink));
        let r = e.run_source(src);
        let sink = e.take_sink();
        let mut sink = sink;
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        (r, std::mem::take(&mut sink.transcript))
    }

    /// 0 参数宏的**纯定界串参数文本**（`\def\X\fi:\use:n{...}`）在展开上下文
    /// （`\edef`/`\csname` 名字扫描）也须在调用点匹配并吞掉。
    ///
    /// tex.web macro_call `if info(r)<>end_match_token then @<Scan the
    /// parameters@>`——参数文本非空时 0 参数宏同样走参数匹配；`\else`/`\fi`
    /// 在实参扫描里是**数据**（get_token，不推进条件机）。expl3 条件生成器
    /// fast form（`\__prg_T_true:w`/`\__prg_F_true:w`/`\__prg_TF_true:w`/
    /// `\__prg_p_true:w`）即此形态：体首 `\fi:` 负责闭合调用点的条件帧。
    /// 旧实现 expand_once/scan_csname 两处对 `num_params==0` 直接跳过匹配，
    /// 定界串泄给条件机 → 帧被提前弹掉 + "Extra \fi."，expl3-code.tex
    /// l.7934 起 `\str_const:Ne` 区级联、`\str_case` 全线 extra-} 失衡
    /// （错误 7016 → 修复后 1835，终态行号 l.8073 → l.9353）。
    #[test]
    fn zero_param_macro_delimiter_text_consumed_in_expansion_contexts() {
        // l3kernel fast form 的最小同构：T 分支选择器（expl3 catcode：`_`/`:` 为字母）。
        // 执行路径（call_macro）旧测已盖；此处盖**展开上下文**的两处调用点：
        // \edef 体扫描（expand_once）与 \csname 名字扫描（scan_csname）。
        let setup = concat!(
            "\\catcode`\\_=11 \\catcode`\\:=11 %\n",
            "\\let\\fi:\\fi %\n",
            "\\long\\def\\usei:nn#1#2{#1}%\n",
            "\\long\\def\\use:n#1{#1}%\n",
            "\\long\\def\\use_none:n#1{}%\n",
            "\\long\\def\\T_true:w\\fi:\\use_none:n{\\fi:\\use:n}%\n",
            "\\def\\cond:NT#1{\\ifdefined#1\\T_true:w\\fi:\\use_none:n}%\n",
            "\\def\\foo{FOO}%\n",
        );
        // \edef：`\T_true:w` 吞掉定界串 `\fi: \use_none:n`，体首 `\fi:` 闭合
        // \ifdefined 帧，`\use:n` 取真分支 → 结果 YES（旧实现帧被提前弹掉，
        // 报 "Extra \fi." 且结果为空）
        assert_eq!(
            expand(&format!(
                "{setup}\\edef\\res{{\\cond:NT\\foo{{YES}}}}\\res"
            ))
            .unwrap(),
            "YES"
        );
        // 假分支：`\ifdefined` 为假 → 跳到 `\fi`，`\use_none:n` 吞掉分支组
        assert_eq!(
            expand(&format!(
                "{setup}\\edef\\res{{\\cond:NT\\undef{{NO}}}}\\res"
            ))
            .unwrap(),
            ""
        );
        // \csname 名字扫描：不消费定界串会把 `\fi:` 泄进名字文本
        //（"Missing endcsname inserted"）；正确行为下名字 = X+YES
        assert_eq!(
            expand(&format!(
                "{setup}\\edef\\res{{\\expandafter\\string\\csname X\\cond:NT\\foo{{YES}}\\endcsname}}\\res"
            ))
            .unwrap(),
            "\\XYES"
        );
    }

    /// TRIP 冲刺调试（临时）。
    #[test]
    fn dbg_trip_l26_mathchardef() {
        let Some(full) = read_fixture("fixtures/trip/trip.tex") else {
            return;
        };
        let lines: Vec<&str> = full.lines().collect();
        // 逐行扩展：找第一个失败行（L2 起；"条件未闭合"是 \ifx 未到 \fi 的干扰）
        for end in [24, 28, 29, 30] {
            let src = lines[1..end].join("\n");
            let (r, t) = run_transcript(&src);
            eprintln!("DBG L2..{}: ok={} err={r:?}", end, r.is_ok());
            if r.is_err() && !r.unwrap_err().to_string().contains("条件未闭合") {
                eprintln!("   tail: {}", tail(&t, 300));
                break;
            }
        }
    }

    #[test]
    fn undefined_cs_reports_line_context() {
        // A3：未定义 cs 报 `! 消息` + `l.N <行内容>`，当 \relax 继续
        let (r, t) = run_transcript("\\def\\x{A}\n\\undefinedzz");
        assert!(r.is_ok(), "未定义 cs 应恢复继续");
        assert!(t.contains("! Undefined control sequence."), "转录：{t}");
        assert!(t.contains("\\undefinedzz"), "转录：{t}");
        assert!(t.contains("l.2 \\undefinedzz"), "应带第 2 行上下文：{t}");
    }

    #[test]
    fn error_reports_line_context() {
        // A3：可恢复错误（非 long 宏参数含 \par）→ "Paragraph ended" 报错后恢复
        // 继续（is_ok），转录带 l.N 上下文行（TRIP L357 / 真实 TeX 同）
        let (r, t) = run_transcript("\\def\\a#1{#1}\n\\a\\par");
        assert!(r.is_ok(), "非 long 参数含 \\par 应报错恢复继续");
        assert!(
            t.contains("! Paragraph ended before \\a was complete."),
            "应报 Paragraph ended：{t}"
        );
        assert!(t.contains("l.2"), "应带第 2 行上下文：{t}");
        assert!(t.contains("\\a\\par"), "上下文行应为出错行内容：{t}");
    }

    #[test]
    fn undefined_cs_in_macro_reports_caller_line() {
        // A3：宏体内未定义 cs → 回退到最近的源文件行（宏调用处）
        let (r, t) = run_transcript("\\def\\foo{\\noSuchMacro}\n\\foo");
        assert!(r.is_ok(), "未定义 cs 应恢复继续");
        assert!(t.contains("! Undefined control sequence."), "转录：{t}");
        assert!(t.contains("l.2 \\foo"), "应回退到宏调用行：{t}");
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

    // ---------- M2-6 展开吞吐基准（手动运行：cargo test -p ntex-core -- --ignored） ----------

    #[test]
    #[ignore]
    fn bytecode_vs_interpreter_throughput() {
        use std::time::Instant;

        // 高频宏调用语料：2000 个含参数宏调用 + 常量条件
        let mut src =
            String::from("\\def\\foo#1{#1X}\\def\\bar{\\iftrue Y\\else N\\fi}\\def\\run{");
        for _ in 0..2000 {
            src.push_str("\\foo{a}\\bar\\foo{b}\\bar");
        }
        src.push_str("}\\run");

        let run_track = |use_bytecode: bool| -> f64 {
            let mut e = if use_bytecode {
                Expander::new()
            } else {
                Expander::new_interpreter()
            };
            e.run_source("\\def\\__warm{1}").unwrap(); // 预热（代码路径加载）
            let t0 = Instant::now();
            e.run_source(&src).unwrap();
            let elapsed = t0.elapsed().as_secs_f64();
            e.output().len() as f64 / elapsed
        };

        // 预热各一次后正式计时（各 3 次取最大吞吐）
        let _ = run_track(true);
        let _ = run_track(false);
        let bc = (0..3).map(|_| run_track(true)).fold(0.0f64, f64::max);
        let ip = (0..3).map(|_| run_track(false)).fold(0.0f64, f64::max);

        eprintln!(
            "字节码吞吐：{bc:.0} token/s；解释器吞吐：{ip:.0} token/s；比值 {:.2}x",
            bc / ip
        );
        // 不设硬断言（CI 波动大），仅报告数字
    }

    /// `locate_line`（line_starts 二分）与逐字节扫描的等价性（P1 补课的性能路径
    /// 承载 TRIP `l.N` 上下文行与 `Incomplete \ifxxx after line N` 语义，必须
    /// 与原 O(pos) 线性实现逐位一致）。
    #[test]
    fn locate_line_matches_linear_scan() {
        // 确定性伪随机字节流：含行首/行尾/连续换行/无换行长行
        let mut x = 0x2545F4914F6CDD1Du64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut bytes = Vec::new();
        for _ in 0..4000 {
            let r = next() % 8;
            if r == 0 {
                bytes.push(b'\n');
            } else {
                bytes.push(b'a' + (next() % 26) as u8);
            }
        }
        let line_starts = crate::input::line_starts(&bytes);
        for &end in bytes
            .iter()
            .enumerate()
            .map(|(i, _)| i)
            .chain(std::iter::once(bytes.len()))
            .collect::<Vec<_>>()
            .iter()
        {
            // 原 error_context/error_context_pos 的线性实现（参照实现）
            let lin_no = bytes[..end].iter().filter(|&&b| b == b'\n').count() + 1;
            let lin_start = bytes[..end]
                .iter()
                .rposition(|&b| b == b'\n')
                .map(|i| i + 1)
                .unwrap_or(0);
            let lin_end = bytes[lin_start..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|i| lin_start + i)
                .unwrap_or(bytes.len());

            let (no, start, end_) = Expander::locate_line(&bytes, &line_starts, end);
            assert_eq!((no, start, end_), (lin_no, lin_start, lin_end), "end={end}");
        }
    }

    // ---------- M3-2-2 内部参数 ----------

    #[test]
    fn param_assignment_and_the() {
        assert_eq!(expand("\\parindent 20pt\\the\\parindent").unwrap(), "20.0pt");
        assert_eq!(
            expand("\\baselineskip 10pt plus 2pt\\the\\baselineskip").unwrap(),
            "10.0pt plus 2.0pt"
        );
        assert_eq!(expand("\\lineskip 3pt\\the\\lineskip").unwrap(), "3.0pt");
        assert_eq!(
            expand("\\lineskiplimit -1pt\\the\\lineskiplimit").unwrap(),
            "-1.0pt"
        );
    }

    #[test]
    fn param_defaults() {
        assert_eq!(expand("\\the\\parindent").unwrap(), "0.0pt");
        assert_eq!(expand("\\the\\baselineskip").unwrap(), "12.0pt");
        assert_eq!(expand("\\the\\lineskip").unwrap(), "0.0pt");
        assert_eq!(expand("\\the\\lineskiplimit").unwrap(), "0.0pt");
    }

    #[test]
    fn param_local_scoped_at_group_end() {
        let src = "\\parindent 20pt\\begingroup\\parindent 30pt\\endgroup\\the\\parindent";
        assert_eq!(expand(src).unwrap(), "20.0pt");
    }

    #[test]
    fn param_global_scoped() {
        let src = "\\parindent 20pt\\begingroup\\global\\parindent 30pt\\endgroup\\the\\parindent";
        assert_eq!(expand(src).unwrap(), "30.0pt");
    }

    #[test]
    fn param_afterassignment_fires() {
        // \afterassignment 在参数赋值后触发（与寄存器一致）
        let src = "\\def\\x{Y}\\afterassignment\\x\\parindent 10pt\\the\\parindent";
        assert_eq!(expand(src).unwrap(), "Y10.0pt");
    }

    // ---------- M3-3 折行参数 ----------

    #[test]
    fn hsize_and_tolerance_assignment() {
        assert_eq!(expand("\\hsize 100pt\\the\\hsize").unwrap(), "100.0pt");
        assert_eq!(expand("\\tolerance 300\\the\\tolerance").unwrap(), "300");
        // 默认值（TeX initex，A1 修复）：\hsize=6.5in、\tolerance=200
        assert!(expand("\\the\\tolerance").unwrap().ends_with("200"));
    }

    // ---------- M3-4 字体 ----------

    /// 记录事件流的测试 sink（`font_selected` 事件用）。
    #[derive(Debug, Default)]
    struct EventSink {
        chars: Vec<char>,
        fonts: Vec<u32>,
        patterns: Vec<Vec<u8>>,
    }

    impl TokenSink for EventSink {
        fn token(&mut self, tok: Token) -> Result<()> {
            if let Some(c) = tok.charcode().and_then(char::from_u32) {
                self.chars.push(c);
            }
            Ok(())
        }
        fn font_selected(&mut self, font: u32) -> Result<()> {
            self.fonts.push(font);
            Ok(())
        }
        fn patterns(&mut self, patterns: Vec<u8>) -> Result<()> {
            self.patterns.push(patterns);
            Ok(())
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
        fn as_any_ref(&self) -> &dyn std::any::Any {
            self
        }
    }

    /// 记录加载请求的测试加载器（每次加载返回递增 FontId）。
    /// 调用记录经 `Rc<RefCell>` 共享，移入 Expander 后仍可读取。
    type FontCall = (String, Option<i64>, Option<i64>);

    #[derive(Debug, Clone, Default)]
    struct MockLoader {
        calls: Rc<RefCell<Vec<FontCall>>>,
    }

    impl FontLoader for MockLoader {
        fn load(&mut self, name: &str, at: Option<i64>, scaled: Option<i64>) -> Result<u32> {
            let mut calls = self.calls.borrow_mut();
            calls.push((name.to_owned(), at, scaled));
            Ok(calls.len() as u32 - 1)
        }
    }

    /// 用 MockLoader 运行源码，返回 (输出字符, 字体选择事件, 加载请求)。
    fn font_run(src: &str) -> Result<(String, Vec<u32>, Vec<FontCall>)> {
        let loader = MockLoader::default();
        let calls = loader.calls.clone();
        let mut e = Expander::new();
        e.set_font_loader(Box::new(loader));
        let sink = EventSink::default();
        e.set_sink(Box::new(sink));
        e.run_source(src)?;
        let mut sink = e.take_sink();
        let sink = sink.as_any_mut().downcast_mut::<EventSink>().unwrap();
        let chars: String = sink.chars.iter().copied().collect();
        let fonts = sink.fonts.clone();
        let loaded = calls.borrow().clone();
        Ok((chars, fonts, loaded))
    }

    #[test]
    fn font_defines_selector_and_emits_selection() {
        let (chars, fonts, calls) = font_run("\\font\\foo=cmr10\\foo a").unwrap();
        assert_eq!(calls, vec![("cmr10".to_owned(), None, None)]);
        assert_eq!(fonts, vec![0], "执行 \\foo 应触发 font_selected");
        assert!(chars.contains('a'));
    }

    #[test]
    fn font_at_and_scaled_variants() {
        let src = r"\font\a=cmr10 at 12pt \font\b=cmr10 scaled 1200";
        let (_, _, calls) = font_run(src).unwrap();
        assert_eq!(
            calls,
            vec![
                ("cmr10".to_owned(), Some(12 * SP_PER_PT), None),
                ("cmr10".to_owned(), None, Some(1200)),
            ]
        );
    }

    #[test]
    fn font_equals_is_optional() {
        let (_, _, calls) = font_run("\\font\\foo cmr10").unwrap();
        assert_eq!(calls, vec![("cmr10".to_owned(), None, None)]);
    }

    #[test]
    fn font_without_loader_recovers() {
        // TRIP：字体加载失败 → 报 "! Font ... not loadable" 并恢复（绑定字体 0），不终止
        let mut e = Expander::new(); // 默认 NoFontLoader
        let r = e.run_source("\\font\\foo=cmr10");
        assert!(r.is_ok());
        assert!(e.transcript().contains("not loadable"));
    }

    #[test]
    fn ifx_compares_font_meanings() {
        // 同一 cs 与自身相等（Font(0) == Font(0)）
        let out = font_run(r"\font\a=cmr10\ifx\a\a yes\else no\fi").unwrap().0;
        assert_eq!(out, "yes");
        // 两次加载得到不同 FontId → 不等
        let out = font_run(r"\font\a=cmr10\font\b=cmr10\ifx\a\b yes\else no\fi").unwrap().0;
        assert_eq!(out, "no");
    }

    // ---------- M4-6 断字：\patterns ----------

    /// 运行 `\patterns{...}`，返回 sink 收到的模式文本。
    fn pattern_run(src: &str) -> Result<Vec<u8>> {
        let mut e = Expander::new();
        let sink = EventSink::default();
        e.set_sink(Box::new(sink));
        e.run_source(src)?;
        let mut sink = e.take_sink();
        let sink = sink.as_any_mut().downcast_mut::<EventSink>().unwrap();
        Ok(sink.patterns.last().cloned().unwrap_or_default())
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
    fn unbounded_macro_recursion_hits_input_stack_limit() {
        // tex.web `stack_size`（TeX Live 取 5000）：无终止宏递归按 TeX 同款
        // "TeX capacity exceeded, sorry [input stack size=N]" 报错终止，
        // 而非无界推深输入栈直至耗尽内存（latex.ltx 加载挂死 root-cause，A4）。
        let e = expand(r"\def\x{\x}\x");
        assert!(e.is_err(), "无界递归应报错终止");
        let err = e.unwrap_err().to_string();
        assert!(err.contains("输入栈超限"), "错误信息：{err}");
        assert!(err.contains(r"\x"), "应指明递归宏名：{err}");
    }

    // ---------- M3 收尾（RFC-3）：VFS 副作用原语 ----------

    /// 运行源码（MemVfs 后端），返回 (输出字符串, VFS)。副作用用例不跑双轨。
    fn expand_vfs(src: &str, vfs: MemVfs) -> Result<(String, MemVfs)> {
        let mut e = Expander::new();
        e.set_vfs(Box::new(vfs));
        e.run_source(src)?;
        let out = e
            .output()
            .iter()
            .map(|t| t.charcode().and_then(char::from_u32).unwrap_or('\u{FFFD}'))
            .collect();
        let mut vfs = e.take_vfs();
        let vfs = vfs
            .as_any_mut()
            .downcast_mut::<MemVfs>()
            .ok_or_else(|| Error::internal("测试 VFS 应为 MemVfs"))?;
        Ok((out, vfs.clone()))
    }

    #[test]
    fn input_reads_file_from_vfs() {
        let mut vfs = MemVfs::new();
        vfs.insert("ch1.tex", "Chapter One");
        let (out, _) = expand_vfs("\\input{ch1}", vfs).unwrap();
        assert_eq!(out, "Chapter One");
    }

    #[test]
    fn input_falls_back_to_tex_extension() {
        let mut vfs = MemVfs::new();
        vfs.insert("ch2.tex", "Ch2");
        let (out, _) = expand_vfs("\\input ch2", vfs).unwrap();
        assert_eq!(out, "Ch2");
    }

    #[test]
    fn input_nests_and_returns() {
        let mut vfs = MemVfs::new();
        vfs.insert("a.tex", "A\\input{b}B");
        vfs.insert("b.tex", "X");
        let (out, _) = expand_vfs("\\input{a}", vfs).unwrap();
        assert_eq!(out, "AXB");
    }

    #[test]
    fn input_missing_file_errors() {
        let vfs = MemVfs::new();
        assert!(expand_vfs("\\input{nope}", vfs).is_err());
    }

    #[test]
    fn immediate_write_appends() {
        let vfs = MemVfs::new();
        let (_, vfs) = expand_vfs(
            "\\newwrite\\f\\immediate\\openout\\f=out.txt\\immediate\\write\\f{abc}\\closeout\\f",
            vfs,
        )
        .unwrap();
        assert_eq!(vfs.get("out.txt"), Some(b"abc\n".as_slice()));
    }

    #[test]
    fn write_defers_until_end() {
        let vfs = MemVfs::new();
        // 无 \immediate：\write 入队，\end 收尾统一 flush
        let (_, vfs) = expand_vfs(
            "\\newwrite\\f\\openout\\f=out.txt\\write\\f{abc}\\write\\f{def}\\end",
            vfs,
        )
        .unwrap();
        assert_eq!(vfs.get("out.txt"), Some(b"abc\ndef\n".as_slice()));
    }

    #[test]
    fn write_expands_the_at_write_time() {
        let vfs = MemVfs::new();
        // \write 时展开 \the\count0 与宏（TeX 语义：写文件时展开）
        let (_, vfs) = expand_vfs(
            "\\count0=42\\def\\mark{X}\\newwrite\\f\\openout\\f=o.txt\\write\\f{\\the\\count0\\mark}\\end",
            vfs,
        )
        .unwrap();
        assert_eq!(vfs.get("o.txt"), Some(b"42X\n".as_slice()));
    }

    #[test]
    fn write_to_unopened_stream_goes_to_transcript() {
        // tex.web write_out：「write to the terminal if file isn't open」——
        // 未 `\openout` 的 0..15 流（以及流号 >15 的钳制 j=16）在 `\immediate` 下
        // 落终端+log，非丢弃（LaTeX `\write\@unused`/ETRIP `\immediate\write15`
        // 靠这条）。
        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        e.run_source("\\immediate\\write0{abc}").unwrap();
        assert_eq!(e.transcript(), "abc\n");
        // \write16（j=16）同样落终端+log
        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        e.run_source("\\immediate\\write16{term}").unwrap();
        assert_eq!(e.transcript(), "term\n");
        // 非 immediate：延迟写在无 shipout 时忽略（tex.web：whatsit 随页面才执行）
        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        e.run_source("\\write0{abc}").unwrap();
        assert_eq!(e.transcript(), "");
    }

    #[test]
    fn negative_write_stream_is_log_only() {
        // `\write-1`（LaTeX `\wlog`）→ 仅 log（引擎：转录）；j=17 钳制分支。
        // 负流号为既有延迟路径（shipout/结束边界 flush），`\end` 触发 flush。
        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        e.run_source("\\write-1{log only}\\immediate\\write-100000{x}\\end").unwrap();
        assert_eq!(e.transcript(), "x\nlog only\n");
    }

    #[test]
    fn write15_after_openout15_lands_in_file() {
        // LaTeX 内核探测（latex.ltx L176-178）：流 15 是合法文件流——
        // `\immediate\openout15` + `\write15` + `\closeout15` 必须落盘。
        let vfs = MemVfs::new();
        let (_, vfs) = expand_vfs(
            "\\immediate\\openout15=t.aux \\immediate\\write15{hello}\\immediate\\closeout15 \\end",
            vfs,
        )
        .unwrap();
        assert_eq!(vfs.get("t.aux"), Some(b"hello\n".as_slice()));
    }

    #[test]
    fn openout_truncates_stale_file() {
        // tex.web `a_open_out`：\openout 语义为覆盖（首次实际写出清空残留）
        let mut vfs = MemVfs::new();
        vfs.insert("o.aux", "STALE\n");
        let (_, vfs) = expand_vfs(
            "\\newwrite\\f\\openout\\f=o.aux\\write\\f{new}\\closeout\\f",
            vfs,
        )
        .unwrap();
        assert_eq!(vfs.get("o.aux"), Some(b"new\n".as_slice()));
    }

    #[test]
    fn openout_closeout_creates_empty_file() {
        // \openout + \closeout（无 \write）：文件仍产生（tex.web a_open_out→a_close）
        let vfs = MemVfs::new();
        let (_, vfs) = expand_vfs("\\newwrite\\f\\openout\\f=empty.aux\\closeout\\f", vfs).unwrap();
        assert_eq!(vfs.get("empty.aux"), Some(b"".as_slice()));
    }

    #[test]
    fn typeout_via_write17_reaches_transcript() {
        // LaTeX `\typeout` = `\immediate\write17`（latex.ltx L129）→ 终端+log
        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        e.run_source("\\immediate\\write17{Hello world.}").unwrap();
        assert_eq!(e.transcript(), "Hello world.\n");
    }

    #[test]
    fn quoted_file_name_is_unquoted() {
        // web2c：带引号文件名剥引号（latex.ltx `\openin\@inputcheck"#1" `）
        let mut vfs = MemVfs::new();
        vfs.insert("q.tex", "Q");
        // 名字后的终止空格留在输入流（引擎既有的文件名扫描偏差）→ 输出 "Q "
        let (out, _) = expand_vfs("\\input\"q\" ", vfs).unwrap();
        assert_eq!(out, "Q ");
        let mut vfs = MemVfs::new();
        vfs.insert("data.txt", "Hello\n");
        // 名字终止空格留在输入流（既有文件名扫描偏差）→ 前导空格
        let (out, _) = expand_vfs(
            "\\newread\\r\\openin\\r=\"data.txt\" \\read\\r to \\l\\l",
            vfs,
        )
        .unwrap();
        assert_eq!(out, " Hello");
    }

    #[test]
    fn input_missing_file_reports_tex_error() {
        // tex.web prompt_file_name：`! I can't find file \`x'.` + 交互式替换文件名
        // 询问 + batchmode 致命（引擎无交互层 → 终止）
        let vfs = MemVfs::new();
        let err = expand_vfs("\\input{nope}", vfs).unwrap_err();
        assert!(err.to_string().contains("nope"), "错误信息：{err}");
    }

    #[test]
    fn input_missing_file_transcript_block() {
        let mut e = Expander::new();
        e.set_vfs(Box::new(MemVfs::new()));
        assert!(e.run_source("\\input nope.tex").is_err());
        let t = e.transcript();
        assert!(t.contains("! I can't find file `nope.tex'."), "{t}");
        assert!(t.contains("Please type another input file name"), "{t}");
        assert!(t.contains("! Emergency stop."), "{t}");
        assert!(t.contains("*** (job aborted, file error in nonstop mode)"), "{t}");
    }

    #[test]
    fn read_line_defines_cs() {
        let mut vfs = MemVfs::new();
        vfs.insert("data.txt", "Hello\nWorld\n");
        let (out, _) = expand_vfs(
            "\\newread\\r\\openin\\r=data.txt\\read\\r to \\line\\line",
            vfs,
        )
        .unwrap();
        assert_eq!(out, "Hello");
    }

    #[test]
    fn read_eof_errors() {
        let mut vfs = MemVfs::new();
        vfs.insert("empty.txt", "");
        assert!(
            expand_vfs("\\newread\\r\\openin\\r=empty.txt\\read\\r to \\line", vfs)
                .is_err()
        );
    }

    #[test]
    fn glue_order_parsing_and_queries() {
        // 阶后缀解析 + \the\skip 显示（fil/fill/filll；pdfTeX：0 分量省略、
        // 且阶词后的 `\the` 需 `\relax` 隔离避免被 get_x_token 展开吞参数）
        assert_eq!(
            expand("\\skip5=1ptminus0fil\\relax\\the\\skip5").unwrap(),
            "1.0pt"
        );
        assert_eq!(
            expand("\\skip6=1ptplus3fillminus0filll\\relax\\the\\skip6").unwrap(),
            "1.0pt plus 3.0fill"
        );
        // \gluestretchorder/\glueshrinkorder（整数上下文）
        assert_eq!(
            expand("\\skip5=1ptminus0fil\\number\\gluestretchorder\\skip5\\number\\glueshrinkorder\\skip5").unwrap(),
            "01"
        );
        assert_eq!(
            expand("\\skip6=1ptplus3fill\\relax\\number\\gluestretchorder\\skip6").unwrap(),
            "2"
        );
        // \gluestretch/\glueshrink（尺寸上下文）
        assert_eq!(
            expand("\\skip6=1ptplus3fill\\ifdim\\gluestretch\\skip6=3pt yes\\else no\\fi").unwrap(),
            "yes"
        );
        assert_eq!(
            expand("\\skip6=1ptplus3fill\\ifdim\\glueshrink\\skip6=0pt yes\\else no\\fi").unwrap(),
            "yes"
        );
        // skipdef 绑定 cs 也可作为胶水参数
        assert_eq!(
            expand("\\skipdef\\S=7\\skip7=2ptplus1fil\\relax\\number\\gluestretchorder\\S").unwrap(),
            "1"
        );
    }

    #[test]
    fn showtokens_displays_expanded_list() {
        let mut e = Expander::new();
        e.run_source("\\showtokens{Hi world}").unwrap();
        assert_eq!(e.transcript(), "> Hi world.\n");
        // 展开宏参数（\showtokens 的 general text 按 \edef 语义展开）
        let mut e = Expander::new();
        e.run_source("\\def\\x{Hi}\\showtokens{\\x{} world}").unwrap();
        assert_eq!(e.transcript(), "> Hi world.\n");
    }

    #[test]
    fn readline_reads_raw_line() {
        // \endlinechar=13（默认）：行尾附加 ^^M
        let mut vfs = MemVfs::new();
        vfs.insert("data.txt", "Hello World\nnext\n");
        let (out, _) = expand_vfs(
            "\\newread\\r\\openin\\r=data.txt\\readline\\r to \\line\\line",
            vfs,
        )
        .unwrap();
        assert_eq!(out, "Hello World\r");
        // \endlinechar=-1：不附加
        let mut vfs = MemVfs::new();
        vfs.insert("data.txt", "Hello\n");
        let (out, _) = expand_vfs(
            "\\newread\\r\\openin\\r=data.txt\\endlinechar=-1\\readline\\r to \\line\\line",
            vfs,
        )
        .unwrap();
        assert_eq!(out, "Hello");
    }

    #[test]
    fn write18_shell_escape_rejected() {
        let vfs = MemVfs::new();
        assert!(expand_vfs("\\write18{echo hi}\\end", vfs).is_err());
    }

    // ---------- M4-5 e-TeX 展开扩展 ----------

    #[test]
    fn protected_macro_not_expanded_in_edef() {
        // \protected\def\foo{Hi} → \edef\x{\foo} 时 \foo 不展开，\x = \foo
        let out = expand(r"\protected\def\foo{Hi}\edef\x{\foo}\expandafter\detokenize\expandafter{\x}")
            .unwrap();
        assert_eq!(out, r"\foo ", "宏 x 应保留 \\foo（detokenize 控制词后补空格）而非展开为 Hi");
    }

    #[test]
    fn unprotected_macro_expands_in_edef() {
        let out = expand(r"\def\foo{Hi}\edef\x{\foo}\expandafter\detokenize\expandafter{\x}").unwrap();
        assert_eq!(out, "Hi");
    }

    #[test]
    fn protected_macro_not_expanded_in_write() {
        // \write 参数展开抑制 protected 宏：\foo 不展开，输出为空（TeX 语义丢弃）
        let vfs = MemVfs::new();
        let (_, vfs) = expand_vfs(
            r"\protected\def\foo{Hi}\newwrite\w\openout\w=out.txt\write\w{\foo}\closeout\w",
            vfs,
        )
        .unwrap();
        assert_eq!(
            vfs.get("out.txt").map(|b| String::from_utf8_lossy(b).into_owned()),
            Some("\n".to_owned()),
            "protected 宏不展开 → 写空行（TeX 语义）"
        );
    }

    #[test]
    fn protected_macro_still_expands_normally() {
        // 正常展开（非抑制上下文）不受影响
        assert_eq!(expand(r"\protected\def\foo{Hi}\foo").unwrap(), "Hi");
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

    #[test]
    fn csname_undefined_becomes_relax() {
        // latex.ltx L1113 expl3 门闩语义（tex.web L7753-7754）：\csname 对未定义名
        // eq_define(cs,relax,256)——与 \relax 原语同义，\ifx 相等。
        // 注意需 \expandafter：\ifx 操作数不展开（TeX 语义），裸 \ifx\csname… 比较的
        // 是 \csname 原语自身。
        assert_eq!(
            expand(r"\expandafter\ifx\csname nope\endcsname\relax T\else F\fi").unwrap(),
            "T"
        );
        // 制造后定义持久：\csname 产物被执行是 no-op（不报 Undefined），且 \ifdefined 为真
        // （名字须全字母——`\nope2` 在正文会切成 `\nope`+`2`；带数字名只能用 \csname 再取）
        assert_eq!(
            expand(r"\csname nopecs\endcsname\ifdefined\nopecs yes\else no\fi").unwrap(),
            "yes"
        );
        // 数字等非字母字符可入名，但只能经 \csname 再引用（TeX 语义同）
        assert_eq!(
            expand(r"\csname nope2\endcsname\ifdefined\nope no\else\expandafter\ifx\csname nope2\endcsname\relax T\else F\fi\fi").unwrap(),
            "T"
        );
        // \edef 上下文（expand_once 路径）同样制造 relax：\a 展开为 \qqq（relax 同义）
        assert_eq!(
            expand(r"\edef\a{\csname qqq\endcsname}\expandafter\ifx\a\relax T\else F\fi")
                .unwrap(),
            "T"
        );
        // TeX 2.9 "relax local"：组内制造 → 组末恢复未定义（\ifdefined 假）
        assert_eq!(
            expand(r"{\csname grpname\endcsname}\ifdefined\grpname yes\else no\fi").unwrap(),
            "no"
        );
        // \meaning：制造出的 relax 不再显示 "undefined"（引擎对 Primitive 槽显示
        // `\名`——记录偏差：TeX 对 \meaning\nope 输出 "relax"（按含义），引擎按
        // 当前 cs 名显示 `\nope`；\relax 原语因名恰为 relax 故二者仅此处有差）
        assert!(
            !expand(r"\expandafter\meaning\csname nope\endcsname")
                .unwrap()
                .contains("undefined"),
            r"制造后 \meaning 不应显示 undefined"
        );
        // 直接使用真正未定义 cs 仍是报错恢复（不经 \csname 制造不产生定义）
        assert_eq!(
            expand(r"\ifdefined\directundefinedzzz yes\else no\fi").unwrap(),
            "no"
        );
    }

    #[test]
    fn unless_reverses_condition() {
        assert_eq!(expand(r"\unless\iftrue yes\else no\fi").unwrap(), "no");
        assert_eq!(expand(r"\unless\iffalse yes\else no\fi").unwrap(), "yes");
    }

    #[test]
    fn numexpr_basic_arithmetic() {
        assert_eq!(expand(r"\the\numexpr 2+3*4 \relax").unwrap(), "14");
        assert_eq!(expand(r"\the\numexpr 10/3 \relax").unwrap(), "3");
        assert_eq!(expand(r"\the\numexpr 20-7 \relax").unwrap(), "13");
    }

    #[test]
    fn numexpr_with_register() {
        assert_eq!(
            expand(r"\count0=7\the\numexpr \count0*2 \relax").unwrap(),
            "14"
        );
    }

    #[test]
    fn numexpr_in_ifnum() {
        assert_eq!(
            expand(r"\ifnum\numexpr 2*3 \relax > 5 yes\else no\fi").unwrap(),
            "yes"
        );
    }

    #[test]
    fn detokenize_converts_to_character_tokens() {
        assert_eq!(expand(r"\detokenize{abc}").unwrap(), "abc");
        // 控制序列 → \名字 文本
        assert_eq!(
            expand(r"\detokenize{a\relax b}").unwrap(),
            r"a\relax b"
        );
    }

    #[test]
    fn unexpanded_in_edef_keeps_tokens() {
        // \unexpanded{\foo} 在 \edef 里不展开 → \x = \foo
        let out = expand(
            r"\def\foo{Hi}\edef\x{\unexpanded{\foo}}\expandafter\detokenize\expandafter{\x}",
        )
        .unwrap();
        assert_eq!(out, r"\foo ");
    }

    #[test]
    fn e_tex_version_and_revision() {
        assert_eq!(expand(r"\the\eTeXversion").unwrap(), "2");
        // e-TeX 2.6：revision 带前导点（版本号"2.6"的后半段）
        assert_eq!(expand(r"\the\eTeXrevision").unwrap(), ".6");
    }

    // ---------- M4-5 e-TeX 扩展：\dimexpr/\glueexpr/\ifprimitive/\scantokens ----------

    #[test]
    fn dimexpr_basic_arithmetic() {
        assert_eq!(expand(r"\the\dimexpr 1pt+2pt \relax").unwrap(), "3.0pt");
        assert_eq!(expand(r"\the\dimexpr 10pt-2.5pt \relax").unwrap(), "7.5pt");
        assert_eq!(expand(r"\the\dimexpr -1pt+2pt \relax").unwrap(), "1.0pt");
    }

    #[test]
    fn dimexpr_in_dimen_assignment() {
        // \dimen0=\dimexpr...：scan_dimen 识别 \dimexpr 原语
        assert_eq!(
            expand(r"\dimen0=\dimexpr 1pt+2pt \relax\the\dimen0").unwrap(),
            "3.0pt"
        );
    }

    #[test]
    fn glueexpr_basic_and_last_stretch_wins() {
        // width 求和；stretch/shrink 值求和，无穷阶取最后一个非零分量项的阶
        assert_eq!(
            expand(r"\the\glueexpr 1pt plus 2pt + 3pt minus 1pt \relax").unwrap(),
            "4.0pt plus 2.0pt minus 1.0pt"
        );
        // stretch 值求和（非"最后一个覆盖"）：2pt + 4pt = 6pt
        assert_eq!(
            expand(r"\the\glueexpr 1pt plus 2pt + 3pt plus 4pt \relax").unwrap(),
            "4.0pt plus 6.0pt"
        );
    }

    #[test]
    fn glueexpr_in_skip_assignment() {
        // 减法项：stretch 符号随项翻转
        assert_eq!(
            expand(r"\skip0=\glueexpr 1pt plus 2pt - 0.5pt \relax\the\skip0").unwrap(),
            "0.5pt plus 2.0pt"
        );
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
    fn scantokens_rescans_text_with_current_catcodes() {
        // 组内容 detokenize 后按当前 catcode 重新扫描（等价于从字符串 \input）
        assert_eq!(expand(r"\def\x{abc}\scantokens{\x}").unwrap(), "abc");
        // 扫描过程中定义并展开宏
        assert_eq!(expand(r"\scantokens{a\def\y{b}\y}").unwrap(), "ab");
    }

    // ---------- ETRIP 冲刺：e-TeX marks 族查询（可展开原语） ----------

    #[test]
    fn marks_queries_expand_empty_with_default_sink() {
        // 默认 TokenSink 无 marks 状态：六个查询原语展开为空串（不报错、双轨一致）
        for prim in [
            "topmarks",
            "firstmarks",
            "botmarks",
            "splitfirstmarks",
            "splittopmarks",
            "splitbotmarks",
        ] {
            let src = format!("\\{prim}3");
            assert_eq!(expand(&src).unwrap(), "", "\\{prim} 应展开为空");
        }
    }

    #[test]
    fn marks_query_scans_class_number() {
        // class 号按 TeX scan_int 解析（正负号/可选 `=`）
        assert_eq!(expand("\\topmarks -5").unwrap(), "");
        assert_eq!(expand("\\firstmarks=7").unwrap(), "");
        // 缺数字：report_missing_number 恢复为 0，后续 token 照常处理
        let mut e = Expander::new();
        e.run_source("\\botmarks x").unwrap();
        assert!(e.transcript().contains("Missing number"), "转录：{}", e.transcript());
        let out: String = e
            .output()
            .iter()
            .map(|t| t.charcode().and_then(char::from_u32).unwrap_or('\u{FFFD}'))
            .collect();
        assert_eq!(out, "x", "缺数字恢复后 x 继续输出");
    }

    #[test]
    fn mark_event_produces_no_output() {
        // \mark/\marks<n> 是 sink 事件（布局侧记录状态），不产生字符输出
        assert_eq!(expand("\\mark{Hello}").unwrap(), "");
        assert_eq!(expand("\\marks2{Hi world}").unwrap(), "");
        // core 不保存 marks 状态：\mark 后查询仍为空（状态委托给布局 sink）
        assert_eq!(expand("\\mark{Hello}\\topmarks0").unwrap(), "");
    }

    // ==== 临时复现：\muexpr 行为校准（对照 pdfTeX；待并入正式测试后删） ====
    #[test]
    fn tmp_muexpr_repro() {
        for (name, src) in [
            // 前导量合法：mu 上下文 + muexpr 输出 → 无 Incompatible
            ("the_muexpr", r"\muskip43=\muexpr(5muminus1mu)\relax\the\muexpr\muskip43"),
            // 直接赋值 + \the 显示 "5.0mu minus 1.0mu"
            ("muskip_the", r"\muskip43=5mu minus 1mu\the\muskip43"),
            ("quot5c", r#"\def\9{\relax}\ifnum-6=\glueexpr\muexpr32mu/"10000\9/-5 T\else F\fi"#),
            ("quot5d", r#"\def\9{\relax}\ifnum6=\muexpr-\dimexpr32spplus-1muminus-1fil/-5 T\else F\fi"#),
            ("gluetomu_muskip", r"\muskip1=5mu\skip2=\gluetomu\muskip1\the\skip2"),
            // \the\muexpr 直接求值显示 mu 单位
            ("the_muexpr_val", r"\the\muexpr 5mu+3mu"),
            // \skip=\muexpr：Incompatible glue units（pt 上下文遇 mu 胶水）
            ("skip_from_muexpr", r"\skip2=\muexpr5mu\the\skip2"),
            // \muskip=\glueexpr：Incompatible glue units（mu 上下文遇 pt 胶水）
            ("muskip_from_glueexpr", r"\muskip2=\glueexpr5pt\the\muskip2"),
            // \muskip=\skip 前导：Incompatible glue units
            ("muskip_from_skip", r"\skip0=1pt plus 2pt\muskip1=\skip0\the\muskip1"),
            // mu 上下文单位错误：5pt → "(mu inserted)" 恢复 5.0mu
            ("muskip_pt_unit", r"\muskip2=5pt\the\muskip2"),
            // mu 上下文无单位：→ "(mu inserted)" 恢复 5.0mu
            ("muskip_bare", r"\muskip3=5\the\muskip3"),
            // mu 上下文 width 带阶：→ "(mu inserted)"（width 不认 fil）
            ("muskip_width_fil", r"\muskip4=5fil\the\muskip4"),
            // mu 上下文 stretch 带阶：合法（stretch 认 fil）→ "1.0mu plus 1.0fil"
            ("muskip_stretch_fil", r"\muskip5=1mu plus 1fil\the\muskip5"),
            // pt 上下文遇 mu 单位：→ "(pt inserted)" 恢复
            ("skip_mu_unit", r"\skip6=5mu\the\skip6"),
        ] {
            let mut e = Expander::new();
            match e.run_source(src) {
                Ok(_) => {
                    let out: String = e
                        .output()
                        .iter()
                        .map(|t| t.charcode().and_then(char::from_u32).unwrap_or('\u{FFFD}'))
                        .collect();
                    println!("[{name}] OUT={out:?}");
                }
                Err(err) => println!("[{name}] ERR={err:?}"),
            }
            println!("[{name}] TRANSCRIPT={:?}", e.transcript());
        }
    }

    /// ETRIP P0 \muexpr 校准（report_help + math_em）：expander 端两层回归。
    /// ① Incompatible glue units. 后跟 help1 行 "I'm going to assume that
    ///    1mu=1pt when they're mixed."（etrip.tex L900-960 段、参考 tex.web
    ///    L8265-8268 mu_error）。
    /// ② \thinmuskip=\<mu> 与 \the\thinmuskip 显示仍为 "X.0mu"——\muskip 寄存器
    ///    以 mu 数值存，expander 显示不依赖 em（layout 端的 sp 缩放见
    ///    math_em/mu_to_sp，ntex-layout tests 验）。
    #[test]
    fn muexpr_incompatible_help1_and_thinmuskip_display() {
        // ① \skip=\muskip：mu→pt 上下文混用，报 Incompatible 后跟 help1 行。
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source("\\muskip1=5mu\\skip0=\\muskip1\\relax").unwrap();
        let sink = e.take_sink();
        let mut sink = sink;
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert!(
            sink.transcript.contains("Incompatible glue units."),
            "Incompatible 触发：{:?}",
            sink.transcript
        );
        assert!(
            sink.transcript.contains("I'm going to assume that 1mu=1pt when they're mixed."),
            "help1 行未跟随：{:?}",
            sink.transcript
        );

        // ② \thinmuskip=18mu \the\thinmuskip —— 仍显示 "18.0mu"。
        assert_eq!(
            expand("\\thinmuskip=18mu\\the\\thinmuskip").unwrap(),
            "18.0mu",
            "\\thinmuskip \\the 应按 mu 数值原样显示"
        );
        // 整数 mu 显式路径（muskip_params 通路之一）：与 etrip.tex L1018 一致。
        assert_eq!(
            expand("\\thinmuskip=27mu plus 9mu minus 18mu\\the\\thinmuskip").unwrap(),
            "27.0mu plus 9.0mu minus 18.0mu"
        );
        // \muskip 寄存器：以 mu 数值原样存。
        assert_eq!(
            expand("\\muskip5=2.5mu plus 1mu\\the\\muskip5").unwrap(),
            "2.5mu plus 1.0mu"
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

    // ---- 错误恢复链回归（fixtures/repro/ 归档；9b0bc69/3a2cb63）----

    /// `\mkern-9mu`/`\mskip9mu`：tex.web mu 上下文（只认 mu 单位）——
    /// 不得报 Illegal unit of measure（8d0a71c muskip 参数化后曾用 pt
    /// 上下文扫描报错；3a2cb63 改 scan_dimen_mu/scan_glue_mu）。
    #[test]
    fn repro_mkern_mskip_mu_context() {
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source("$$\\mkern-9mu\\mskip9mu minus1fil\\mathord x$$").unwrap();
        let mut sink = e.take_sink();
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert!(
            !sink.transcript.contains("Illegal unit"),
            "mu 上下文不得报 Illegal unit：{}",
            sink.transcript
        );
    }

    /// 内部整数参数单独出现（无 `=`）一律 no-op（TeX 主循环不读值）：
    /// `$\splitdiscards` 数学模式不报 Missing number（9b0bc69；参考
    /// showbox27 空数学证实不读值不产生原子）。
    #[test]
    fn repro_internal_int_no_value_noop() {
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source("$\\splitdiscards\\noindent$\\pagediscards}").unwrap();
        let mut sink = e.take_sink();
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert!(
            !sink.transcript.contains("Missing number"),
            "内部量单独出现不应 Missing number：{}",
            sink.transcript
        );
    }

    /// `\right` 前缺 `\left`：恢复式 "Extra \right."（tex.web；参考 trip
    /// L256 `$\right\relax` 双错误恢复）——不中断（9b0bc69，原硬错误）。
    #[test]
    fn repro_right_without_left_recovers() {
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        let _ = e.run_source(r"$\right[A$");
        let mut sink = e.take_sink();
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert!(
            sink.transcript.contains("Extra \\right."),
            "\\right 无 \\left 应报 Extra \\right：{}",
            sink.transcript
        );
    }

    /// fraction 原语族（\abovewithdelims/\above/\atopwithdelims/\overwithdelims）
    /// 必须挂 sink.math_fraction（tex.web math_fraction；此前只扫参数不挂 →
    /// 分子分母混收当前层，trip l.276 数学状态崩；2026-09-02 修）。
    #[test]
    fn repro_fraction_primitives_hang_fraction() {
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        let _ = e.run_source(r"$a\abovewithdelims(.2pt b$");
        let mut sink = e.take_sink();
        let sink = sink.as_any_mut().downcast_mut::<VecSink>().unwrap();
        assert!(
            !sink.transcript.contains("Ambiguous"),
            "\\abovewithdelims 单 fraction 不应歧义：{}",
            sink.transcript
        );
    }

    /// 同层嵌套 fraction（\over 后 \abovewithdelims，trip l.257）：TeX 恢复式
    /// Ambiguous（丢弃新 fraction 保持原 fraction）——不得 Engine error 中断。
    #[test]
    fn repro_nested_fraction_ambiguous_recovers() {
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        let r = e.run_source(r"$\left.A\over A\abovewithdelims.?\right($");
        assert!(
            r.is_ok(),
            "嵌套 fraction 歧义应恢复式（不中断）：{r:?}"
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

    // ---------- LaTeX 兼容第八刀：pdfTeX 引擎探测/兼容原语族 ----------

    #[test]
    fn pdftex_probe_primitives_are_defined_and_engine_fence_holds() {
        // engine-check（latex.ltx L1122）按 `\ifdefined\pdffilesize` 探测；
        // l3kernel `\c_sys_engine_str` 按 `\tex_pdftexversion:D` 存在性取 pdftex 分支。
        // 引擎栅栏：其他引擎标记（luatexversion/kanjiskip/XeTeXversion）必须**保持
        // 未定义**——多引擎同定义会把引擎串拼成无法识别的混合值。
        let src = "\\ifdefined\\pdftexversion T\\else F\\fi\\ifdefined\\pdffilesize T\\else F\\fi\\ifdefined\\pdfstrcmp T\\else F\\fi\\ifdefined\\luatexversion T\\else F\\fi\\ifdefined\\kanjiskip T\\else F\\fi";
        assert_eq!(expand(src).unwrap(), "TTTFF");
    }

    #[test]
    fn pdftex_version_revision_are_number_operands() {
        // latex.ltx L22500 实际用法：`\ifnum\pdftexversion=140 \ifnum\pdftexrevision<22`
        assert_eq!(
            expand(r"\ifnum\pdftexversion=140 \ifnum\pdftexrevision<22 OLD\else NEW\fi\else BAD\fi")
                .unwrap(),
            "NEW"
        );
        assert_eq!(expand(r"\number\pdftexversion.\number\pdftexrevision").unwrap(), "140.25");
    }

    #[test]
    fn pdftex_banner_expands_as_string() {
        // banner 保留 NTex 标识（不冒充真 pdfTeX 产物），版本段与探测值一致
        let out = expand(r"\pdftexbanner").unwrap();
        assert!(out.starts_with("This is pdfTeX, Version 1.40.25"), "banner：{out}");
        assert!(out.contains("NTex"), "banner 须含 NTex 标识：{out}");
    }

    #[test]
    fn pdfoutput_defaults_to_dvi_mode_and_is_assignable() {
        // pdfTeX 默认 \pdfoutput=0（DVI）；expl3 按 `\tex_pdfoutput:D` 作数字读取
        assert_eq!(
            expand(r"\ifcase\pdfoutput DVI\or PDF\or PDF\else PDF\fi").unwrap(),
            "DVI"
        );
        assert_eq!(expand(r"\pdfoutput=1 \the\pdfoutput").unwrap(), "1");
        // 恢复 0：expl3 后端判定（dvips 分支）依赖
        assert_eq!(expand(r"\pdfoutput=1 \pdfoutput=0 \ifnum\pdfoutput>0 P\else D\fi").unwrap(), "D");
    }

    #[test]
    fn pdfstrcmp_compares_byte_strings() {
        assert_eq!(expand(r"\ifnum\pdfstrcmp{abc}{abd}=0 E\else \pdfstrcmp{abc}{abd}\fi").unwrap(), "-1");
        assert_eq!(expand(r"\pdfstrcmp{abc}{abc}").unwrap(), "0");
        assert_eq!(expand(r"\pdfstrcmp{b}{a}").unwrap(), "1");
        // 前缀短串小于长串；\pdfstrcmp 可在 \numexpr/\edef 中展开（expl3 \str_compare 用法）
        assert_eq!(expand(r"\ifnum\pdfstrcmp{ab}{abc}>0 B\else S\fi").unwrap(), "S");
        assert_eq!(expand(r"\edef\x{\pdfstrcmp{z}{a}}\x").unwrap(), "1");
    }

    #[test]
    fn pdffilesize_reports_bytes_and_empty_for_missing() {
        // 文件不存在 → 空展开：l3kernel \file_full_name:n 以空返回判定"未找到"
        let mut vfs = MemVfs::new();
        vfs.insert("data.bin", "0123456789");
        let (out, _) = expand_vfs(
            "\\def\\sz{\\pdffilesize{data.bin}}\\pdffilesize{missing.bin}|\\sz",
            vfs,
        )
        .unwrap();
        assert_eq!(out, "|10");
    }

    #[test]
    fn pdfuniformdeviate_stays_in_range_and_seed_is_readable() {
        // r 恒在 [0,n)；\pdfrandomseed 只读（写入走 \pdfsetrandomseed）
        let out = expand(
            r"\count0=\pdfuniformdeviate 7 \ifnum\count0<7 R\else BAD\fi \pdfsetrandomseed 42 \number\pdfrandomseed",
        )
        .unwrap();
        assert_eq!(out, "R42");
        // 同种子 → 同序列（确定性可复现；真 pdfTeX 为随机源——偏差记录报告 §15.3）
        assert_eq!(
            expand(r"\pdfsetrandomseed 9 \pdfuniformdeviate 100 \pdfuniformdeviate 100").unwrap(),
            expand(r"\pdfsetrandomseed 9 \pdfuniformdeviate 100 \pdfuniformdeviate 100").unwrap()
        );
    }

    #[test]
    fn pdfshellescape_and_elapsedtime_are_safe_defaults() {
        // 无 shell escape、无计时器：恒 0（l3kernel \c_sys_shell_escape_int /
        // \sys_timer: 按非 LuaTeX 分支无条件读取）
        assert_eq!(
            expand(r"\ifnum\pdfshellescape=0 SAFE\else SHELL\fi \ifnum\pdfelapsedtime=0 IDLE\else RAN\fi")
                .unwrap(),
            "SAFEIDLE"
        );
    }

    #[test]
    fn pdfcreationdate_uses_pdf_datetime_format() {
        // D:YYYYMMDDHHMMSSZ'00'（\time 只有分钟精度 → 秒恒 00）
        let out = expand(r"\pdfcreationdate").unwrap();
        assert!(out.starts_with("D:2"), "创建日期应为 2xxx 年：{out}");
        assert!(out.ends_with("Z'00'"), "PDF 日期串收尾：{out}");
        assert_eq!(out.len(), 21, "D: + 14 位 + Z'00'：{out}");
    }

    #[test]
    fn latex_engine_check_aggregate_gate_passes() {
        // latex.ltx L1122 原形：四探针聚合 >0（NTex 只注册 pdftex 族 → 单探针真）
        let src = "\\ifnum0%\n  \\ifdefined\\pdffilesize 1\\fi\n  \\ifdefined\\filesize 1\\fi\n  \\ifdefined\\luatexversion\\ifnum\\luatexversion>94 1\\fi\\fi\n  \\ifdefined\\kanjiskip 1\\fi\n  >0 PASS\\else REJECT\\fi";
        assert_eq!(expand(src).unwrap(), "PASS");
    }

    #[test]
    fn pdf_primitives_work_in_numexpr_and_the() {
        // expl3 \int_eval:n（= \number\numexpr）与 \the 路径
        assert_eq!(
            expand(r"\number\numexpr\pdftexversion+\pdfoutput\relax").unwrap(),
            "140"
        );
        assert_eq!(expand(r"\the\pdfoutput|\the\pdftexversion").unwrap(), "0|140");
    }
}
