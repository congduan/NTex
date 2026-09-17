use super::*;

// ── 第十七刀取证：cs 定界实参 + 跨帧匹配（\prg_map_break:Nn break 炸弹）──────
//
// expl3 \prg_map_break:Nn（expl3-code l.2477）的纯宏 break 机制：
//   \cs_new:Npn \prg_map_break:Nn #1#2#3 \prg_break_point:Nn #4#5 {...}
//   \cs_new_eq:NN \prg_break_point:Nn \use_ii:nn
// #3 的定界符是 **cs token**，且往往不在发出 break 的那一帧——tex.web macro_call
// 的定界扫描 pop 帧续扫（token 链跨层），此处对齐。

/// cs 定界实参：同帧内匹配（基准）。
#[test]
fn cs_delimited_arg_same_frame() {
    let src = "\\def\\useii#1#2{}\
               \\let\\breakpoint\\useii\
               \\def\\a#1\\breakpoint{[#1]}\
               \\a XY\\breakpoint Z";
    let out = expand(src).unwrap_or_else(|e| panic!("err: {e:?}"));
    // GT（pdfTeX 实证，内容流 [(\[XY\]Z)]TJ）：#2 无定界收集吞前导空格
    assert_eq!(out, "[XY]Z", "同帧 cs 定界实参");
}

/// cs 定界实参跨帧：定界符在**调用者**帧，扫描须 pop 帧续扫。
#[test]
fn cs_delimited_arg_crosses_frame() {
    // \issuer 的帧在 BODY 后耗尽；\breakpoint 在 caller 帧里
    let src = "\\def\\useii#1#2{}\
               \\let\\breakpoint\\useii\
               \\def\\mapbreak#1#2#3\\breakpoint#4#5{[#3][#5]}\
               \\def\\issuer{\\mapbreak\\foo{}BODY}\
               \\def\\caller{\\issuer\\breakpoint\\foo{TAIL}AFTER}\
               \\caller";
    let out = expand(src).unwrap_or_else(|e| panic!("err: {e:?}"));
    assert_eq!(out, "[BODY][TAIL]AFTER", "跨帧 cs 定界实参");
}

// ── \prg_map_break:Nn 全结构（expl3-code l.2476-2484 逐 token 转写）─────────

/// break 炸弹：break 从 3 层嵌套帧深处发出，\prg_break_point:Nn 标记在
/// 最外层帧——#3 定界扫描须吸收全部中间帧残留并终止（tex.web pop 续扫语义）。
#[test]
fn prg_map_break_bomb_three_frames() {
    let src = concat!(
        "\\def\\useii#1#2{}\\def\\useiii#1#2#3{}",
        "\\let\\breakpoint\\useii",
        // \prg_map_break:Nn
        "\\def\\mapbreak#1#2#3\\breakpoint#4#5{",
        "#5\\ifx#1#4\\expandafter\\useiii\\fi\\mapbreak#1{#2}}",
        // \prg_break_point:Nn \clbreak {} 标记 + break 发出点在 3 层深处
        "\\def\\clbreak{\\mapbreak\\clbreak{}}",
        "\\def\\lvlthree{\\lvlbody}",
        "\\def\\lvlbody{X\\clbreak}",
        "\\def\\clmap{\\lvlthree\\breakpoint\\clbreak{END}}",
        "\\clmap AFTER"
    );
    let out = expand(src).unwrap_or_else(|e| panic!("err: {e:?}"));
    assert_eq!(out, "XENDAFTER", "break 炸弹三帧 unwind");
}

/// 不等名 break：嵌套 map 外层继续扫（#1≠#4 → 重发 \prg_map_break:Nn）。
#[test]
fn prg_map_break_nested_outer_breakpoint() {
    let src = concat!(
        "\\def\\useii#1#2{}\\def\\useiii#1#2#3{}",
        "\\let\\breakpoint\\useii",
        "\\def\\mapbreak#1#2#3\\breakpoint#4#5{",
        "#5\\ifx#1#4\\expandafter\\useiii\\fi\\mapbreak#1{#2}}",
        "\\def\\inner{\\mapbreak\\innerbrk{}}",
        "\\def\\innerbrk{}",
        // break 发出点在深层；第一个 \breakpoint 标记名不同（≠#1）→ 继续向外扫
        "\\def\\go{\\deep\\breakpoint\\otherbrk{SKIP}\\breakpoint\\innerbrk{END}}",
        "\\def\\deep{\\lvlbody}",
        "\\def\\lvlbody{X\\inner}",
        "\\go AFTER"
    );
    let out = expand(src).unwrap_or_else(|e| panic!("err: {e:?}"));
    // GT（pdfTeX 内容流 [(XSKIPEND)28(AFTER)]TJ）：#5 无条件输出——第一标记
    // 名不等时 {SKIP} 照样吐出，#1≠#4 只驱动 \mapbreak 重发 break 向外扫。
    assert_eq!(out, "XSKIPENDAFTER", "嵌套 break 逐层 unwind");
}
