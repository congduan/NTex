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

#[test]
fn fmt_reload_then_run_etrip_preamble() {
    // 与驱动一致：TFM 目录 + 完整文件 → dump → 载入 → 重跑
    std::env::set_var(
        "NTEX_TFM_DIR",
        "/Users/congduan/Desktop/code/_vibe_coding_/NTex/fixtures/etrip",
    );
    let src =
        std::fs::read("/Users/congduan/Desktop/code/_vibe_coding_/NTex/fixtures/etrip/etrip.tex")
            .unwrap();

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
    let tr4 = ts4.take_transcript();
    eprintln!(
        "pass2(fullfmt fullsrc): {:?} tr_len={} tr_head={:?} tr_tail={:?}",
        r3.as_ref().err(),
        tr4.len(),
        &tr4.chars().take(120).collect::<String>(),
        &tr4.chars()
            .rev()
            .take(300)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
    );

    // 只跑测试体（line 119+）
    let mut ts5 = ntex_layout::Typesetter::with_tfm();
    let state5 = ntex_format::load(&mut &buf[..]).unwrap();
    ts5.import_state(state5);
    let body: Vec<u8> = lines[118..].join(&b'\n');
    let r5 = ts5.typeset_bytes(body);
    eprintln!(
        "pass2(body-only): {:?} transcript_head={:?}",
        r5.as_ref().err(),
        &ts5.take_transcript().chars().take(80).collect::<String>()
    );

    // 二分 body 定位下一个失败点
    for (tag, end) in [
        ("to399", 399usize),
        ("to414", 414usize),
        ("to470", 470usize),
    ] {
        let mut t = ntex_layout::Typesetter::with_tfm();
        t.import_state(ntex_format::load(&mut &buf[..]).unwrap());
        let seg: Vec<u8> = lines[118..end].join(&b'\n');
        let rr = t.typeset_bytes(seg);
        let tr = t.take_transcript();
        eprintln!(
            "seg {tag}: {:?} tr_tail={:?}",
            rr.as_ref().err(),
            &tr.chars()
                .rev()
                .take(60)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        );
    }

    // 诊断：fmt 载入后 e-TeX 原语是否保持原语身份
    let mut ts6 = ntex_layout::Typesetter::with_tfm();
    let state6 = ntex_format::load(&mut &buf[..]).unwrap();
    ts6.import_state(state6);
    let d = ts6.typeset_bytes(r"\message{D2: \ifx\unexpanded\relax UNEXP-REL\else UNEXP-OK\fi|\ifx\ifdefined\relax IFDEF-REL\else IFDEF-OK\fi|\ifx\eTeXversion\relax ETV-REL\else ETV-OK\fi}");
    eprintln!(
        "diag(primitives): {:?} transcript={:?}",
        d.as_ref().err(),
        ts6.take_transcript()
    );
}
