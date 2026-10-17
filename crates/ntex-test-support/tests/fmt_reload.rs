//! 临时诊断：fmt 往返后二次运行（模拟驱动 pass 2）。

#[test]
fn fmt_reload_minimal() {
    let mut ts = ntex_layout::Typesetter::with_tfm();
    ts.typeset_bytes(r"\def\foo{FOO}\message{defs done;}")
        .unwrap();
    let mut buf = Vec::new();
    ntex_format::save(&mut buf, &ts.export_state()).unwrap();
    let mut ts2 = ntex_layout::Typesetter::with_tfm();
    let state = ntex_format::load(&mut &buf[..]).unwrap();
    ts2.import_state(state);
    let r = ts2.typeset_bytes(r"\ifx\foo\undefined NO\else YES\fi");
    eprintln!(
        "minimal: {:?} transcript={:?}",
        r.as_ref().err(),
        ts2.take_transcript()
    );
    // 空 fmt 直接跑
    let mut ts3 = ntex_layout::Typesetter::with_tfm();
    let r3 = ts3.typeset_bytes(r"\message{plain}\relax");
    eprintln!(
        "plain: {:?} transcript={:?}",
        r3.as_ref().err(),
        ts3.take_transcript()
    );
}

/// `.fmt` 必须保留当前字体：TRIP 的初始化段用 `\trip` 选中字体后 `\dump`，
/// 第二遍会跳过定义段，后续字符只能从快照恢复该选择。
#[test]
fn fmt_reload_preserves_current_font() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/trip");
    std::env::set_var("NTEX_TFM_DIR", dir);

    let mut ts = ntex_layout::Typesetter::with_tfm();
    ts.typeset_bytes(r"\font\trip=trip \trip\dump").unwrap();
    let state = ts.export_state();
    assert_ne!(
        state.current_font, 0,
        "\\dump 前选中的字体不得退回 nullfont"
    );

    let mut buf = Vec::new();
    ntex_format::save(&mut buf, &state).unwrap();
    let state = ntex_format::load(&mut &buf[..]).unwrap();
    assert_eq!(state.current_font, 1, "fmt 编解码不得丢失当前字体");
}

#[test]
fn trip_initex_dump_preserves_selected_font() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/trip");
    std::env::set_var("NTEX_TFM_DIR", dir);
    let src = std::fs::read(format!("{dir}/trip.tex")).unwrap();

    let mut ts = ntex_layout::Typesetter::with_tfm_paginated();
    ts.typeset_bytes(src).unwrap();
    assert!(ts.dumped());
    assert_ne!(
        ts.export_state().current_font,
        0,
        "TRIP initex 的 \\trip 选择必须进入 fmt"
    );
}

#[test]
fn fmt_reload_then_run_etrip_preamble() {
    // 与驱动一致：TFM 目录 + 完整文件 → dump → 载入 → 重跑
    // fixtures 路径按 CARGO_MANIFEST_DIR 定位（硬编码 /Users/... 仅原作者机器可用）
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/etrip");
    std::env::set_var("NTEX_TFM_DIR", dir);
    let src = std::fs::read(format!("{dir}/etrip.tex")).unwrap();

    let mut ts = ntex_layout::Typesetter::with_tfm();
    let r1 = ts.typeset_bytes(src.clone());
    eprintln!(
        "pass1(full): {:?} dumped={}",
        r1.as_ref().err(),
        ts.dumped()
    );
    eprintln!("pass1 transcript: {}", ts.take_transcript());
    let mut buf = Vec::new();
    ntex_format::save(&mut buf, &ts.export_state()).unwrap();
    eprintln!("fmt bytes: {}", buf.len());

    let lines: Vec<&[u8]> = src.split(|&b| b == b'\n').collect();

    // 完整 fmt + 截断源（60 行）：若仍失败 → fmt 状态坏
    let mut ts2 = ntex_layout::Typesetter::with_tfm();
    let state = ntex_format::load(&mut &buf[..]).unwrap();
    ts2.import_state(state);
    let head60: Vec<u8> = lines[..60].join(&b'\n');
    let r2a = ts2.typeset_bytes(head60);
    eprintln!(
        "pass2(fullfmt @60): {:?} transcript={:?}",
        r2a.as_ref().err(),
        ts2.take_transcript()
    );

    // 完整 fmt + 完整源
    let mut ts4 = ntex_layout::Typesetter::with_tfm();
    let state4 = ntex_format::load(&mut &buf[..]).unwrap();
    ts4.import_state(state4);
    let r3 = ts4.typeset_bytes(src.clone());
    eprintln!(
        "pass2(fullfmt @full): {:?} transcript={:?}",
        r3.as_ref().err(),
        ts4.take_transcript()
    );
}
