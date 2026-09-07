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
