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
        // \the 读回
        assert_eq!(expand("\\lccode`B=`b\\the\\lccode`B").unwrap(), "98");
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
        // \write16：写终端（带换行）
        let mut e = Expander::new();
        e.run_source("\\write16{hi}").unwrap();
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
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/etrip/etrip.tex"
        );
        let full = std::fs::read_to_string(path).unwrap();
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
        // \write15 内容走转录（VecSink）而非 output：
        let mut e = Expander::new();
        e.set_sink(Box::new(VecSink::default()));
        e.run_source("\\count2000=5\\write15{\\the\\count2000}").unwrap();
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
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/etrip/etrip.tex");
        let src = std::fs::read_to_string(path).unwrap();
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
        assert_eq!(
            sink.transcript,
            "! Bad register code (-1).\n\nl.1 \\countdef\\1=-1 \\countdef\\1=32768 \\countdef\\1=0 \\countdef\\1=32767 \\relax\n\n! Bad register code (32768).\n\nl.1 \\countdef\\1=-1 \\countdef\\1=32768 \\countdef\\1=0 \\countdef\\1=32767 \\relax\n\n",
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
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/etrip/etrip.tex");
        let src = std::fs::read_to_string(path).unwrap();
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
        let src = "\\skip5=1ptminus0fil\\muskip5=\\gluetomu\\skip5\\ifnum\\glueshrinkorder\\mutoglue\\muskip5=1 T\\else F\\fi";
        assert_eq!(expand(src).unwrap(), "T");
        // etrip.tex L948 完整宏场景：\100pt10pt\mutoglue\muskip5
        let src3 = "\\def\\1#1#2pt#3#4pt#5 {\\ifnum\\glueshrinkorder#5=#3 T\\else F\\fi}\\skip5=1ptminus0fil\\muskip5=\\gluetomu\\skip5\\100pt10pt\\mutoglue\\muskip5 ";
        assert_eq!(expand(src3).unwrap(), "T");
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

    /// TRIP 冲刺调试（临时）。
    #[test]
    fn dbg_trip_l26_mathchardef() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/trip/trip.tex");
        let full = std::fs::read_to_string(path).unwrap();
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
    fn write_to_unopened_stream_is_ignored() {
        // TeX 语义：延迟 \write 到未打开流在 shipout/flush 时被忽略（内容丢弃，不报错）
        let vfs = MemVfs::new();
        assert!(expand_vfs("\\write0{abc}\\end", vfs).is_ok());
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
}
