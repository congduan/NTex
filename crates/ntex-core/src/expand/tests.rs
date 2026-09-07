#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::rc::Rc;
    use ntex_io::MemVfs;
    use crate::sink::{AlignSink, BoxSink, CoreSink, FontSink, IoSink, MathSink, PageSink};

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
    fn trip_opening_line_parses() {
        // trip.tex 第 1 行原文：\immediate\catcode `{ = 1 \endlinechar=13
        assert_eq!(expand("\\immediate\\catcode`{=1\\endlinechar=13").unwrap(), "");
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
    fn advance_param_page_arithmetic() {
        // G3（tex.web do_register_command）：page 参数是 \advance 合法目标
        // （survey §2.3 的独立引擎缺口；plain letterformat.tex 死点形态）
        assert_eq!(
            expand("\\hsize=100pt\\advance\\hsize by 10pt\\the\\hsize").unwrap(),
            "110.0pt"
        );
        // \vsize 同通道；负号紧贴 by（无空格）
        assert_eq!(
            expand("\\vsize=100pt\\voffset=24pt\\advance\\vsize by-\\voffset\\the\\vsize").unwrap(),
            "76.0pt"
        );
        // 内部量做增量（\parindent → \hsize）
        assert_eq!(
            expand("\\parindent=10pt\\hsize=100pt\\advance\\hsize by\\parindent\\the\\hsize").unwrap(),
            "110.0pt"
        );
        // 胶参数：逐分量加
        assert_eq!(
            expand("\\baselineskip=12pt plus 2pt\\advance\\baselineskip 3pt\\the\\baselineskip")
                .unwrap(),
            "15.0pt plus 2.0pt"
        );
        // 整数参数
        assert_eq!(
            expand("\\tolerance=100\\advance\\tolerance 100\\the\\tolerance").unwrap(),
            "200"
        );
        // 组作用域：组内增量出组恢复（走 assign_param 的 save 通道）
        assert_eq!(
            expand("\\hsize=100pt{\\advance\\hsize by10pt}\\the\\hsize").unwrap(),
            "100.0pt"
        );
        // mu 胶参数（muskip 0/1/2 槽）
        assert_eq!(
            expand("\\thinmuskip=3mu\\advance\\thinmuskip by 1mu\\the\\thinmuskip").unwrap(),
            "4.0mu"
        );
    }

    #[test]
    fn multiply_divide_param_arithmetic() {
        // G3：乘除同通道（tex.web do_register_command 同一目标集合）
        assert_eq!(
            expand("\\hsize=100pt\\multiply\\hsize by2\\divide\\hsize by4\\the\\hsize").unwrap(),
            "50.0pt"
        );
        // 寄存器路径不受影响（回归钉：by 关键字形态）
        assert_eq!(
            expand("\\count0=5\\advance\\count0 by1\\the\\count0").unwrap(),
            "6"
        );
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

    /// em/ex 内部单位（tex.web scan_dimen）：em = quad(cur_font)（fontdimen 6）、
    /// ex = x_height(cur_font)（fontdimen 5）。修复前 em/ex 不在单位表，
    /// `1.5em` 的 "em" 字母会泄漏回排版流（demo 列表 "emem" 字面字符 bug）。
    #[test]
    fn dimen_em_ex_internal_units() {
        let run = |src: &str| -> Result<String> {
            let mut e = Expander::new();
            e.set_font_loader(Box::new(MockLoader::default()));
            e.run_source(src)?;
            Ok(e.output()
                .iter()
                .map(|t| t.charcode().and_then(char::from_u32).unwrap_or('\u{FFFD}'))
                .collect())
        };
        // 1.5em = 1.5 × 10pt(655360sp) = 983040sp；且 "em" 不泄漏
        assert_eq!(
            run("\\dimen0=1.5em\\count0=\\dimen0\\the\\count0").unwrap(),
            "983040"
        );
        // 1ex = 4.3pt = 281744sp
        assert_eq!(
            run("\\dimen0=1ex\\count0=\\dimen0\\the\\count0").unwrap(),
            "281744"
        );
        // 无字体加载器 → 参数缺失按 0 计，"ex" 同样不泄漏
        assert_eq!(
            expand("\\dimen0=1ex\\count0=\\dimen0\\the\\count0").unwrap(),
            "0"
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
    fn non_outer_macro_in_argument_ok() {
        // 非 outer 宏作实参正常
        assert_eq!(
            expand("\\def\\a#1{#1}\\def\\x{A}\\a\\x").unwrap(),
            "A"
        );
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
    fn toks_register_copy_via_toksdef_cs() {
        // TRIP L418 场景：\tokens 是 \toksdef 绑定的 cs，RHS 为 \toks1 寄存器复制
        assert_eq!(
            expand("\\toksdef\\tokens=256 \\toks1={abc}\\tokens\\toks1\\the\\tokens").unwrap(),
            "abc"
        );
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
    fn char_expands_to_character_token() {
        assert_eq!(expand("\\char65").unwrap(), "A");
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

    impl CoreSink for EventSink {
        fn token(&mut self, tok: Token) -> Result<()> {
            if let Some(c) = tok.charcode().and_then(char::from_u32) {
                self.chars.push(c);
            }
            Ok(())
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
        fn as_any_ref(&self) -> &dyn std::any::Any {
            self
        }
    }
    impl FontSink for EventSink {
        fn font_selected(&mut self, font: u32) -> Result<()> {
            self.fonts.push(font);
            Ok(())
        }
    }
    impl IoSink for EventSink {
        fn patterns(&mut self, patterns: Vec<u8>) -> Result<()> {
            self.patterns.push(patterns);
            Ok(())
        }
    }
    impl TokenSink for EventSink {}
    impl MathSink for EventSink {}
    impl BoxSink for EventSink {}
    impl AlignSink for EventSink {}
    impl PageSink for EventSink {}

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

        /// em/ex 内部单位测试用（cmr10 量级固定值，忽略 font 参数——
        /// EventSink 的 current_font 恒 0）：quad(6)=655360sp(10pt)、
        /// x_height(5)=281744sp(4.3pt)。
        fn font_param(&mut self, _font: u32, param: usize) -> Option<i64> {
            match param {
                5 => Some(281_744),
                6 => Some(655_360),
                _ => None,
            }
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
    fn e_tex_version_and_revision() {
        assert_eq!(expand(r"\the\eTeXversion").unwrap(), "2");
        // e-TeX 2.6：revision 带前导点（版本号"2.6"的后半段）
        assert_eq!(expand(r"\the\eTeXrevision").unwrap(), ".6");
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

    // ── 按域拆分的测试子模块（R2 纯移动，helper 留主文件走 super::* 链） ──
    mod tests_macro { include!("tests_macro.rs"); }
    mod tests_scan { include!("tests_scan.rs"); }
    mod tests_expr { include!("tests_expr.rs"); }
    mod tests_cond { include!("tests_cond.rs"); }
    mod tests_io_write { include!("tests_io_write.rs"); }
    mod tests_insert_alloc { include!("tests_insert_alloc.rs"); }
}
