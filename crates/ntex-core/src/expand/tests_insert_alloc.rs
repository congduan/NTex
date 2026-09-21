use super::*;

// ── 输出例程刀 4：\newinsert 分配器 + 三联寄存器（count/dimen/skip） ──
//
// plain.tex 的 \newinsert 是纯宏层分配器：
//   \outer\def\newinsert#1{\global\advance\insc@unt by\m@ne …\allocationnumber=\insc@unt
//     \global\chardef#1=\allocationnumber …}
// 其中 \insc@unt=\count20（INITEX 置 255，向低分配）、\m@ne=\count22=-1。
// 引擎侧要提供的是四面寄存器经 chardef'd cs 间接寻址可读可写。
//
// 本域测试钉住一个曾把 \m@ne(\count22) 抹成 0 的回归：内部整数参数赋值
// （`\escapechar\m@ne`，plain.tex \newif 首行）的值扫描不认 <internal integer>
// 臂时，会把该 cs 判成"单独出现 no-op"回流，随后被当赋值目标吞掉后续 token
// → \count22 被写 0 → \newinsert 步长归零 → \footins/\topins 同落 \insert255
// （G0 仪器在 plain 预载错误清单里首次照见，2026-09-07）。

/// `\newinsert` 宏层形状：countdef 计数器向低步进 + chardef 命名 + 三联读写往返。
#[test]
fn newinsert_allocator_shape_and_triple_roundtrip() {
    let src = concat!(
        "\\catcode`\\@=11 %\n",
        "\\countdef\\insc@unt=20 \\count20=255 %\n",
        "\\countdef\\allocationnumber=21 %\n",
        "\\countdef\\m@ne=22 \\m@ne=-1 %\n",
        "\\def\\newinsert#1{\\global\\advance\\insc@unt by\\m@ne %\n",
        "  \\allocationnumber=\\insc@unt \\global\\chardef#1=\\allocationnumber} %\n",
        "\\newinsert\\foo \\newinsert\\bar %\n",
        "\\count\\foo=3 \\dimen\\foo=5pt \\skip\\foo=2pt plus 1sp %\n",
        "\\message{R:\\number\\foo,\\number\\bar,\\the\\count\\foo,\\the\\dimen\\foo,\\the\\skip\\foo} %\n",
    );
    let (r, t) = run_transcript(src);
    assert!(r.is_ok(), "分配器形状须跑通：{t}");
    assert!(
        t.contains("R:254,253,3,5.0pt,2.0pt plus 0.00002pt"),
        "首次分配 254（255-1）且向低递减、三联读写往返：{t}"
    );
}

/// 回归：`\escapechar\m@ne`（内部整数参数 ← countdef'd cs）须当 <internal integer>
/// 取值，不得把 `\m@ne` 回流成赋值目标（那会把 \count22 写 0）。
#[test]
fn int_param_value_may_be_countdef_cs_without_clobbering_it() {
    let src = concat!(
        "\\catcode`\\@=11 %\n",
        "\\countdef\\m@ne=22 \\m@ne=-1 %\n",
        "\\escapechar\\m@ne %\n",
        "\\message{E:\\the\\escapechar\\space M:\\number\\m@ne} %\n",
    );
    let (r, t) = run_transcript(src);
    assert!(r.is_ok(), "{t}");
    assert!(
        t.contains("E:-1M:-1"),
        "\\escapechar 取 \\m@ne 值且 \\count22 不被抹：{t}"
    );
    assert!(
        !t.contains("Missing number"),
        "值扫描不得落入 Missing number 恢复：{t}"
    );
}

/// 同臂的 `<内部整数寄存器>` 形式：`\escapechar\count0`（`\count<n>` 原语）。
#[test]
fn int_param_value_may_be_count_register_primitive() {
    let src = "\\count0=123 \\escapechar\\count0 \\message{E:\\the\\escapechar}";
    let (r, t) = run_transcript(src);
    assert!(r.is_ok(), "{t}");
    assert!(t.contains("E:123"), "{t}");
}

/// `=` 缺席且后随非值 token：维持 TRIP 需要的"单独出现 no-op"（读值不发生），
/// 本刀的 <internal integer> 臂不得把它吞掉。
#[test]
fn int_param_bare_stay_noop_for_non_value_token() {
    let src = "\\escapechar\\undefinedzz \\message{E:\\the\\escapechar}";
    let (r, t) = run_transcript(src);
    assert!(r.is_ok(), "未定义 cs 恢复继续：{t}");
    assert!(t.contains("E:92"), "默认 92 不变（no-op）：{t}");
}

// ── 2026-09-21：字面常量作内部整数参数的**值**（反引号字母常量/十六进制/八进制） ──
//
// scan_int 的值起点不止数字与 <internal integer>：tex.web 的字面常量（`` `\
// 字母常量、"A 十六进制、'17 八进制）同样合法。预扫描环不认这一族时，整条赋值
// 被判"单独出现 no-op"：值不落 eqtb、反引号回流主循环被当字符排版。
//
// 现场：amsgen.sty \@saveprimitive 的 `\begingroup\escapechar`\\`——amsmath
// 载入 7 次调用各落一个孤立 '`'（OT1 0x60=quoteleft），Transformer 论文
// 首页 7 个竖排 ' + 标题/作者/摘要整体后移（2026-09-21）。

/// 反引号字母常量作值：值生效、且回流字符不得进入输出流。
#[test]
fn int_param_value_may_be_backquote_alphabetic_constant() {
    let src = "\\escapechar`\\\\ \\message{E:\\the\\escapechar}\n";
    let (r, t) = run_transcript(src);
    assert!(r.is_ok(), "{t}");
    assert!(t.contains("E:92"), "`\\\\ 须作反斜杠字符码 92：{t}");
    assert_eq!(
        expand("\\escapechar`\\\\").unwrap(),
        "",
        "反引号不得回流成排版字符（LaTeX 首页孤立 ' 的根源）"
    );
}

/// 十六进制 / 八进制前缀同属字面常量起点。
#[test]
fn int_param_value_may_be_hex_or_octal_constant() {
    let src = "\\escapechar\"A \\message{H:\\the\\escapechar}\\escapechar'17 \\message{O:\\the\\escapechar}\n";
    let (r, t) = run_transcript(src);
    assert!(r.is_ok(), "{t}");
    assert!(t.contains("H:10"), "\"A 须作 16#A=10：{t}");
    assert!(t.contains("O:15"), "'17 须作 8#17=15：{t}");
}

/// `\newinsert` 分配的号与 `\insert<n>`/`\ifvoid` 打通（盒寄存器面）。
#[test]
fn allocated_number_reaches_insert_and_void_test() {
    let src = concat!(
        "\\catcode`\\@=11 %\n",
        "\\countdef\\insc@unt=20 \\count20=255 %\n",
        "\\countdef\\m@ne=22 \\m@ne=-1 %\n",
        "\\def\\newinsert#1{\\global\\advance\\insc@unt by\\m@ne %\n",
        "  \\global\\chardef#1=\\insc@unt} %\n",
        "\\newinsert\\fa %\n",
        "\\message{V:\\ifvoid\\fa void\\else nonvoid\\fi\\space N:\\number\\fa} %\n",
    );
    let (r, t) = run_transcript(src);
    assert!(r.is_ok(), "{t}");
    assert!(t.contains("V:voidN:254"), "分配号 254 直接可用：{t}");
}

// ── 2026-09-11：`\chardef` 定义的 cs 作内部整数参数的**值**（与上文 \m@ne 同源） ──
//
// 上文钉的是 `\countdef` 走 `EqSlot::Register` 臂；本组钉 `\chardef` 走
// `EqSlot::Char` 臂。两者同属 tex.web scan_int 的 <internal integer>，
// `internal_integer` 判定漏掉 `EqSlot::Char` 时的症状不是"值错"而是
// **多排版一个字符**：`\fam\bffam`（plain.tex `\def\bf{\fam\bffam\tenbf}`）
// 被判"单独出现 no-op"→ `\bffam` 回流 → 主循环把它当字符 6 送进盒树。
//
// 影响面：**每个 `\bf` 都多插一个字符节点**——
//   - cmr10 下 char 6 宽 7.22222pt（`\showbox` 实测），DVI 与真实 TeX 不一致；
//   - 换 Unicode 正文字体（中文场景）后直接报
//     `Missing character: There is no ^^F in font <name>!`。
// 现场：Tauri 渲染 resume-plain.tex（2026-09-11）。

/// `\chardef` cs 作参数值：值生效、且**不得**作为字符进入输出流。
#[test]
fn chardef_cs_is_a_parameter_value_not_a_character() {
    // 值生效：\the\fam 回读为 5（chardef 的字符码）。
    let (r, t) = run_transcript("\\chardef\\five=5 \\fam\\five \\message{F:\\the\\fam}\n");
    assert!(r.is_ok(), "{t}");
    assert!(t.contains("F:5"), "chardef'd cs 应作参数值取出：{t}");

    // 且不得被排版：输出为空（修复前会输出字符 '5'）。
    assert_eq!(
        expand("\\chardef\\five=5 \\fam\\five").unwrap(),
        "",
        "\\fam\\<chardef'd cs> 不得把该 cs 当字符排版"
    );

    // 对照组（本就正确，锁住不回归）：宏作值（trip.tex `\tracingoutput\on` 形态）
    // 与字面数字。注意源码里不留尾随空格——空格本身是会被排版出去的字符，
    // 会让"输出应为空"的判据失真（`\chardef` 那条的数字扫描顺带吃掉分隔空格，
    // 宏那条不会，两者不可共用同一写法）。
    assert_eq!(expand("\\fam5").unwrap(), "");
    assert_eq!(expand("\\def\\five{5}\\fam\\five").unwrap(), "");
    assert_eq!(expand("\\chardef\\five=5\\fam\\five").unwrap(), "");
    // 对照：chardef'd cs 直接出现在正文里**应当**被排版（catcode 12 字符）。
    // 取值是 chardef 给定的**字符码**（`\chardef\cs=65` → 字符 'A'），
    // 不是数值文本 `65`——别把这条与"作参数值"混淆。
    assert_eq!(expand("\\chardef\\letter=65\\letter").unwrap(), "A");
}
