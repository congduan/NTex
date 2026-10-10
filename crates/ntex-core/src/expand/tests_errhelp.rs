// ---------- M1-13 刀C 回归锁：\errhelp 消费链 + \errmessage 实参报错 ----------

use crate::Expander;

#[test]
fn errmessage_arg_scan_reports_undefined_cs_before_message() {
    // 刀C 主靶：tex.web scan_toks(false,true)——实参**扫描位**就地展开，
    // 实参里的 undefined cs 先报 "! Undefined control sequence." 再报消息
    // 本体（GT nonstopmode 两错序；去 scan.rs 的 Undefined 臂必红）。报错
    // 后继续不脱轨：后续 \errmessage 仍正常出站。
    let mut e = Expander::new();
    e.set_misc_int(crate::param::MISC_INTERACTION_MODE, 1);
    e.run_source(concat!(
        "\\errhelp{Try harder.}\n",
        "\\errmessage{Custom failure: \\foo}\n",
        "\\errmessage{Still alive.}\n",
        "\\end",
    ))
    .unwrap();
    let t = e.transcript().to_string();
    let ucs = t.find("! Undefined control sequence.");
    let msg = t.find("! Custom failure: .");
    let alive = t.find("! Still alive.");
    let (Some(ucs), Some(msg), Some(alive)) = (ucs, msg, alive) else {
        panic!("GT 错误构成缺失（两错 + 续跑站）：{t:?}");
    };
    assert!(ucs < msg, "错序：undefined cs 应先行：{t:?}");
    assert!(msg < alive, "消息错误后未继续处理：{t:?}");
    // 消息文本与 GT 一致：坏 cs 丢弃、前置空格保留（`Custom failure: `）
    assert!(
        t.contains("! Custom failure: ."),
        "消息文本偏差（\\foo 应丢弃、`Custom failure: ` 应保留）：{t:?}"
    );
    // 上下文行号 = 扫描位行（l.2），不得漂到 EOF/锚点行
    let ctx = t[ucs..].find("l.2 ").expect("缺 l.2 上下文行");
    assert!(ctx < msg - ucs, "l.2 上下文应属于 undefined cs 错误块：{t:?}");
}

#[test]
fn errhelp_group_scope_and_errorstop_gated_consumption() {
    // \errhelp 消费（tex.web use_err_help）：仅 errorstopmode(3) 停等打出
    // `? <help>` 块；组内赋值出组恢复（save.rs toks 参数组作用域）——组内
    // boom 用 inner、出组 again 用 Try harder.。nonstop/batch 不打任何
    // help 行（GT nonstopmode 实测无 help 行）。
    const SRC: &str = concat!(
        "\\errhelp{Try harder.}\n",
        "{\\errhelp{inner}\\errmessage{boom}}\n",
        "\\errmessage{again}\n",
        "\\end",
    );
    let mut e = Expander::new();
    e.set_misc_int(crate::param::MISC_INTERACTION_MODE, 3);
    e.run_source(SRC).unwrap();
    let t = e.transcript().to_string();
    let inner = t.find("? inner\n\n");
    let outer = t.find("? Try harder.\n\n");
    let (Some(inner), Some(outer)) = (inner, outer) else {
        panic!("errhelp 停等块缺失：{t:?}");
    };
    assert!(inner < outer, "组作用域恢复错序（inner 应先出、Try harder. 后出）：{t:?}");

    for mode in [0, 1] {
        let mut e2 = Expander::new();
        e2.set_misc_int(crate::param::MISC_INTERACTION_MODE, mode);
        e2.run_source(SRC).unwrap();
        let t2 = e2.transcript().to_string();
        assert!(
            !t2.contains("Try harder.") && !t2.contains("? inner") && !t2.contains("? Try"),
            "mode={mode} 不应打 errhelp 停等块：{t2:?}"
        );
        assert!(
            t2.contains("! boom.") && t2.contains("! again."),
            "mode={mode} 消息本体缺失：{t2:?}"
        );
    }
}
