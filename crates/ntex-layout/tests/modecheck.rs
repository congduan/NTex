//! `$$` 模式判定（tex.web init_math 的 `mode>0` 门）回归测试。
//!
//! TRIP L210 `\hbox{$$}$\par}` 依赖：受限水平模式（mode=-hmode<0）中 `$$`
//! 不进入显示数学，而是退化为两个独立的 `$` 各自进出普通数学——不报任何错。
//! NTex 此前自创 "Display math in restricted mode." 并在 peek 时消费第二个 `$`，
//! 导致组/模式错位。

use ntex_layout::Typesetter;

/// 取转录（终端+日志文本）。
fn transcript(src: &str) -> String {
    let mut ts = Typesetter::with_tfm_paginated();
    let _ = ts.typeset_bytes(src.as_bytes());
    ts.take_transcript()
}

fn assert_contains(needle: &str, hay: &str, what: &str) {
    assert!(hay.contains(needle), "{what}\n--- 实际转录 ---\n{hay}");
}

#[test]
fn double_dollar_in_restricted_horizontal_enters_plain_math_without_error() {
    let t = transcript("\\tracingcommands=1\n\\hbox{$$} then\n");
    assert_contains(
        "{restricted horizontal mode: math shift character $}",
        &t,
        "第一个 $ 在受限水平模式进入普通数学（tex.web init_math：mode>0 不成立）",
    );
    assert!(
        !t.contains("Display math in restricted mode"),
        "tex.web 无此错误，不应自创\n--- 实际转录 ---\n{t}"
    );
    // `$`#2 在数学模式被追踪（back_input 放回后作为闭合 `$` 收掉普通数学——
    // trip.log L1832 同款）；`}` 须已回到受限水平模式（trip.log L1834），即
    // 数学必须真正关上：闭合侧 peek 到的 `$` 一律消费（tex.web after_math
    // "Check that another $ follows"），否则放回的 `$` 会在非数学态重开一个
    // 永不闭合的行内公式（`$$x$$` 报"数学模式未闭合"）。
    assert_contains(
        "{math mode: math shift character $}",
        &t,
        "第二个 $ 在数学模式作为闭合 `$` 被追踪",
    );
    assert_contains(
        "{restricted horizontal mode: end-group character }}",
        &t,
        "`}` 回到受限水平模式（数学已由第二个 $ 关上）",
    );
}

#[test]
fn double_dollar_in_horizontal_enters_display_math() {
    let t = transcript("\\tracingcommands=1\ntext $$x$$ after\n");
    assert_contains(
        "{display math mode: the letter x}",
        &t,
        "水平模式（mode>0）的 `$$` 进入显示数学",
    );
}

#[test]
fn dollar_par_in_restricted_horizontal_reports_missing_dollar() {
    // `\hbox{$$}$\par`：末尾 $ 开启行内数学，`\par` 在数学模式 → tex.web
    // insert_dollar_sign 报 "Missing $ inserted."（TRIP L210 参考行为）。
    let t = transcript("\\tracingcommands=1\n\\hbox{$$}$\\par\n");
    assert_contains(
        "! Missing $ inserted.",
        &t,
        "数学模式中的 \\par 应报 Missing $ inserted.",
    );
}
