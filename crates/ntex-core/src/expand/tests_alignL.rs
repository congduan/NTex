use super::*;

// ── 刀L：halign preamble 展开臂（tex.web get_preamble_token 语义）──────────
//
// tex.web L15438-15456：preamble 扫描中 `\span` 由 get_preamble_token 自吞，
// 其后 token 若 cur_cmd>max_command（条件/宏/可展开原语）→ **就地展开**。
// TRIP l.363 `\^^C{{\span\ifcase3 \lo#\cr…}` 依赖此：条件在扫描期求值，
// skip 撞 `\outer` 宏 → check_outer_validity aligning 臂（Runaway preamble?
// + Forbidden + 插 \cr } 收束）。旧实现不展开 → `\ifcase3 \lo` 成字面模板，
// skip 一路吃到 EOF。

/// TRIP l.363 形态的最小复现：preamble 中条件展开 + `\outer` 撞 skip 的
/// Forbidden 恢复链。去 preamble 展开臂必红（无展开 → 无 skip → 无 Forbidden）。
#[test]
fn preamble_cond_expands_and_outer_relief_fires() {
    let (r, t) = run_transcript(concat!(
        "\\catcode`^^C = 6 ",
        "\\let\\^^C=\\halign ",
        "\\outer\\global\\long\\edef\\lo#1#2U3#4#5#6#7#8#8#99#{\\relax} ",
        "\\catcode`U = 12 ",
        "\\tracingcommands=2 ",
        "\\^^C{{\\span\\ifcase3 \\lo#\\cr x\\cr} ",
        "\\end"
    ));
    assert!(r.is_ok(), "TRIP l.363 形态须跑到作业末：{t}");
    // ① 条件在 preamble 扫描期展开（tex.web if_case 的 {case N} 追踪）
    assert!(t.contains("{case 3}"), "条件须在扫描期求值：{t}");
    // ② skip 撞 outer → Incomplete \if（forbidden-\fi 救济）
    assert!(
        t.contains("! Incomplete \\ifcase; all text was ignored after line"),
        "{t}"
    );
    assert!(
        t.contains("A forbidden control sequence occurred in skipped text."),
        "{t}"
    );
    // ③ aligning 臂：Runaway preamble? + Forbidden + \cr } 收束
    assert!(t.contains("Runaway preamble?"), "{t}");
    assert!(
        t.contains("! Forbidden control sequence found while scanning preamble of"),
        "{t}"
    );
    // ④ 旧实现病症不得复发：模板把 `\ifcase3 \lo` 当字面 → 逐列 Only-one-#
    assert!(
        !t.contains("Only one # is allowed per tab"),
        "preamble 条件被当字面模板（未展开）：{t}"
    );
}

/// preamble 展开臂的无害形态：`\span\iffalse …\fi` 展开后模板只剩 `#`，
/// 行内容正常排版（旧实现把 `\iffalse…\fi` 留在模板里，cell 期才求值——
/// 行为碰巧相同，但 `\tracingcommands` 轨迹不同：扫描期无 {false} 线）。
#[test]
fn preamble_cond_expansion_consumes_skipped_text_at_scan_time() {
    let (r, t) = run_transcript(concat!(
        "\\tracingcommands=2 ",
        "\\halign{\\span\\iffalse \\zzz\\fi#\\cr y\\cr} ",
        "\\end"
    ));
    assert!(r.is_ok(), "{t}");
    // `\zzz`（未定义）在扫描期已被 skip 吞掉——若留在模板里，cell 期执行
    // 必报 Undefined control sequence。
    assert!(!t.contains("! Undefined control sequence."), "{t}");
    assert!(t.contains("{false}"), "扫描期求值轨迹缺失：{t}");
}

// ── 刀L 连带收口：参数文本扫描（tex.web scan_toks 参数部 L9357-9379）───────
//
// preamble 展开臂让 trip.tex l.356 的 `\lo` 定义真正执行，暴露旧实现的
// 两个偏差：① 非参数符参数字（trip.tex l.12 `\catcode`U=6`）不计参数序号；
// ② 无 "Parameters must be numbered consecutively"/"You already have nine
// parameters" 恢复臂（GT trip.log L5015/L5023）。oracle（TeX Live tex
// 2026）逐字对拍：错误位置、`<to be read again>` token、help 四行。

#[test]
fn param_text_consecutiveness_and_nine_cap_match_texweb() {
    let (r, t) = run_transcript(concat!(
        "\\catcode`U = 6 ",
        "\\def\\lo#1#2U3#4#5#6#7#8#8#99#{\\relax} ",
        "\\end"
    ));
    assert!(r.is_ok(), "参数文本恢复须可继续作业：{t}");
    // 第二个 `#8`：t=9 期望、读到 8 → consecutively + back_error（8 放回，
    // 槽 9 入文；重读的 8 成字面）
    assert!(
        t.contains("! Parameters must be numbered consecutively."),
        "{t}"
    );
    assert!(
        t.contains("I've inserted the digit you should have used after the #."),
        "{t}"
    );
    // `#9`：t 已满 → 吞 `#` 与后随 `9`
    assert!(t.contains("! You already have nine parameters."), "{t}");
    assert!(
        t.contains("I'm going to ignore the # sign you just used,"),
        "{t}"
    );
    assert!(
        !t.contains("非法参数号"),
        "#0/#x 不得 fatal（tex.web 走 consecutively 恢复）：{t}"
    );
}

/// 参数文本中 `##`/`#0`/`#x` = 参数槽 + 字面（tex.web 无 ## 特例）；
/// oracle：`\def\w##1{\relax}` → 两错 + 2 参；`\def\z#0{\relax}` → 一错 + 1 参 + 字面 0。
#[test]
fn param_text_double_hash_is_slot_plus_literal() {
    let (r, t) = run_transcript("\\def\\w##1{\\relax}\\end");
    assert!(r.is_ok(), "{t}");
    assert_eq!(
        t.matches("! Parameters must be numbered consecutively.").count(),
        2,
        "`##1` = 槽1（##）+ 槽2（#1 越号）：{t}"
    );
    let (r, t) = run_transcript("\\def\\z#0{\\relax}\\z0\\end");
    assert!(r.is_ok(), "{t}");
    assert_eq!(
        t.matches("! Parameters must be numbered consecutively.").count(),
        1,
        "{t}"
    );
}

/// 定义体中的 `\outer` 宏（check_outer_validity defining 臂）：runaway 显示 +
/// Forbidden + 插 `}` 收束（半成品即全身体），offending token 放回重读执行。
/// GT trip.log L5169-5188（trip.tex l.364/365 两个 runaway 定义）。
#[test]
fn outer_macro_in_def_body_ends_definition_and_rereads() {
    let (r, t) = run_transcript(concat!(
        "\\outer\\def\\oa{OUTER} ",
        "\\def\\x#1{[#1\\oa]} ",
        "\\x A ",
        "\\end"
    ));
    assert!(r.is_ok(), "定义须以半成品收束、作业继续：{t}");
    assert!(t.contains("Runaway definition?"), "{t}");
    assert!(
        t.contains("! Forbidden control sequence found while scanning definition of \\x."),
        "{t}"
    );
    assert!(t.contains("<inserted text> \n                }"), "{t}");
    assert!(
        t.contains("I suspect you have forgotten a `}', causing me"),
        "{t}"
    );
    // offending token 重读执行：`\oa` 的体排版出来
    assert!(t.contains("OUTER"), "outer 宏须被重读执行：{t}");
}
