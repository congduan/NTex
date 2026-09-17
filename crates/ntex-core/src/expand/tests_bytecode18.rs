use super::*;

// M1-9 / M2，第十八刀：条件尾递归与跨帧实参扫描（RFC-4 双轨等价）。
// 数字上界后必须用空格结束 scan_int；否则 get_x_token 提前展开
// \expandafter\iter\fi，Evaluating 条件插入的 \relax/\fi 尚未消费，
// pdfTeX 同样 O(n) 输入栈增长。不能把这类活帧当作泄漏删除。
// 调用点也须分隔：concat!("\\iter", "DONE") 会变成未定义的 \\iterDONE。

/// 长程循环：expl3 \tl_map_inline 机制同构（break 炸弹 + 跨帧 cs 定界），2 万次迭代。
///
/// \mapbreak#1#2#3\breakpoint#4#5 = \prg_map_break:Nn 同款签名；
/// \one 的调用点在循环体内被反复执行——每一轮都走
/// Bytecode(调用者) → Bytecode(one) → Bytecode(mapbreak) 三层帧链。
#[test]
fn bytecode_long_loop_map_break_no_stack_growth() {
    let src = concat!(
        "\\def\\useiii#1#2#3{}",
        "\\let\\breakpoint\\useiii",
        "\\def\\mapbreak#1#2#3\\breakpoint#4#5{",
        "#5\\ifx#1#4\\expandafter\\useiii\\fi\\mapbreak#1{#2}}",
        "\\def\\one{{[\\mapbreak\\one{X}BODY\\breakpoint\\one{END}]}}",
        "\\def\\clmap{\\one\\one\\one}",
        "\\count0=0",
        "\\def\\iter{\\clmap\\advance\\count0 by 1 \\ifnum\\count0<20000 \\expandafter\\iter\\fi}",
        "\\iter ",
        "DONE"
    );
    let bc = run_track18(src, true);
    let ip = run_track18(src, false);
    assert_eq!(bc, ip, "双轨分叉：长程 break 炸弹循环");
    assert_eq!(bc, format!("{}DONE", "[END]".repeat(60_000)));
    assert!(
        bc.ends_with("DONE"),
        "循环应完整跑完：…{}",
        &bc[bc.len().saturating_sub(40)..]
    );
}

/// 输出例程式重发（纯尾递归内宏调用链）+ 跨帧定界实参：每轮实参扫描须把
/// 调用者 Bytecode 帧内剩余 token 吸干后越过帧边界继续扫。
#[test]
fn bytecode_loop_cross_frame_delim_arg() {
    let src = concat!(
        "\\def\\useiii#1#2#3{}",
        "\\let\\breakpoint\\useiii",
        "\\def\\mapbreak#1#2#3\\breakpoint#4#5{[#3][#5]}",
        "\\def\\issuer{\\mapbreak\\foo{}BODY}",
        "\\def\\caller{\\issuer\\breakpoint\\foo{TAIL}AFTER}",
        "\\count0=0",
        "\\def\\iter{\\caller\\advance\\count0 by 1 \\ifnum\\count0<20000 \\expandafter\\iter\\fi}",
        "\\iter ",
        "DONE"
    );
    let bc = run_track18(src, true);
    let ip = run_track18(src, false);
    assert_eq!(bc, ip, "双轨分叉：跨帧 cs 定界长程循环");
    assert_eq!(bc, format!("{}DONE", "[BODY][TAIL]AFTER".repeat(20_000)));
    assert!(bc.ends_with("DONE"), "循环应完整跑完");
}

/// 单轮三层 break 炸弹（tests_break17 同款 + 字节码轨道直跑）——回归锚点。
#[test]
fn bytecode_single_break_bomb() {
    let src = concat!(
        "\\def\\useii#1#2{}\\def\\useiii#1#2#3{}",
        "\\let\\breakpoint\\useii",
        "\\def\\mapbreak#1#2#3\\breakpoint#4#5{",
        "#5\\ifx#1#4\\expandafter\\useiii\\fi\\mapbreak#1{#2}}",
        "\\def\\clbreak{\\mapbreak\\clbreak{}}",
        "\\def\\lvlthree{\\lvlbody}",
        "\\def\\lvlbody{X\\clbreak}",
        "\\def\\clmap{\\lvlthree\\breakpoint\\clbreak{END}}",
        "\\clmap AFTER"
    );
    let bc = run_track18(src, true);
    assert_eq!(bc, run_track18(src, false));
    assert_eq!(bc, "XENDAFTER", "单轮 break 炸弹（第十七刀语义锁）");
}

/// 双轨运行 + 输出收集（tests.rs 的 expand 双轨对拍取自同款）。
fn run_track18(src: &str, use_bytecode: bool) -> String {
    let mut e = if use_bytecode {
        Expander::new()
    } else {
        Expander::new_interpreter()
    };
    e.feed_source(src);
    let mut peak = 0;
    let mut steps = 0;
    while e
        .process_one()
        .unwrap_or_else(|err| panic!("track(use_bytecode={use_bytecode}) err: {err:?}"))
    {
        steps += 1;
        peak = peak.max(e.stack.len());
        assert!(peak <= 64, "尾递归栈深应有界：track={use_bytecode}, peak={peak}");
        assert!(steps <= 5_000_000, "尾递归应在有限步数内结束");
    }
    assert!(e.cond_stack.is_empty(), "条件帧应全部闭合");
    println!("track={use_bytecode}, steps={steps}, peak={peak}");
    e.output()
        .iter()
        .map(|t| t.charcode().and_then(char::from_u32).unwrap_or('\u{FFFD}'))
        .collect()
}
