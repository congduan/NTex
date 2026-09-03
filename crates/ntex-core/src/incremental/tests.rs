//! M5 阶段一测试：切段 / 全量 vs 增量逐位一致 / 缓存保留 / 依赖失效 /
//! 畸形输入不 panic / 基准雏形。
//!
//! 测试口径：**增量路径输出 == 同一输入全量 `run_source` 输出**（token 文本），
//! 以及编辑后文档的"增量结果 == 编辑后文档全量重跑"。

use std::time::Instant;

use super::*;
use crate::expand::Expander;

/// 段下标 → 互异的字母串 cs 名后缀（0→"a"、25→"z"、26→"aa"…）。
///
/// 不能直接用下标数字：TeX 控制序列名字只含字母，`\w11` 是 cs `\w` + 字符
/// "11"——那样所有段都引用同一个宏 `\w`，改一处必然全量失效。
fn cs_suffix(i: usize) -> String {
    let mut s = String::new();
    let mut n = i + 1;
    while n > 0 {
        n -= 1;
        s.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
    }
    s.chars().rev().collect()
}

/// 全量基线：同一输入一次 `run_source`（出错也收集已产出的 token）。
fn full_text(src: &str) -> String {
    let mut e = Expander::new();
    let _ = e.run_source(src);
    render_tokens(e.output(), e.intern())
}

/// 增量路径：切段逐段执行，输出按段序拼接。
fn incr_text(src: &str) -> String {
    let mut eng = SegmentEngine::new();
    eng.run(src);
    eng.text()
}

// ─── 段落切分 ───────────────────────────────────────────────────────────────

#[test]
fn segmentize_empty_input() {
    assert!(segmentize("").is_empty());
    assert!(incr_text("").is_empty());
}

#[test]
fn segmentize_single_paragraph_without_par() {
    let src = "just one paragraph, no blank line\n";
    assert_eq!(segmentize(src), vec![src.to_owned()]);
}

#[test]
fn segmentize_multiple_paragraphs() {
    let src = "one\n\ntwo\n\nthree\n";
    let segs = segmentize(src);
    assert_eq!(segs.len(), 3, "三个空行分隔的段落 → 三段：{segs:?}");
    assert_eq!(segs[0], "one\n\n");
    assert_eq!(segs[1], "two\n\n");
    assert_eq!(segs[2], "three\n");
}

#[test]
fn segmentize_concatenation_is_identity() {
    // 拼接不变式：切段不得丢失/改写任何字节（否则 \par 数量会变）
    for src in [
        "",
        "\n",
        "\n\n\n",
        "a",
        "a\n\nb",
        "one\n\ntwo\n\nthree\n",
        "\n\nleading blanks\n\nbody\n",
        "trailing blanks\n\n\n\n",
        "\\def\\x{a\n\nb}\n\nc\n",
        "a % comment\n%\n\nb\n",
        "a\n\\par\nb\n",
        "a\n  \\par  \nb\n",
        "weird \\{ brace \\} and \\% escaped\n\nnext\n",
        "extra close brace }\n\nstill splits\n",
        "no newline at end\n\ntail",
    ] {
        assert_eq!(segmentize(src).concat(), src, "拼接必须还原原文：{src:?}");
    }
}

#[test]
fn segmentize_explicit_par_line_is_boundary() {
    let segs = segmentize("a\n\\par\nb\n");
    assert_eq!(segs.len(), 2, "行首独立 \\par 也是边界：{segs:?}");
    // \paragraph 不是 \par：cs 名按字母序列取
    assert_eq!(segmentize("a\n\\paragraph{x}\nb\n").len(), 1);
}

#[test]
fn segmentize_no_split_inside_group() {
    // 深度 > 0 的空行不切（宏体/盒子参数跨空行）
    let segs = segmentize("\\def\\x{a\n\nb}\n\nc\n");
    assert_eq!(segs.len(), 2, "{segs:?}");
    assert_eq!(segs[0], "\\def\\x{a\n\nb}\n\n");
}

#[test]
fn segmentize_comment_only_line_is_boundary() {
    let segs = segmentize("a\n%\n\nb\n");
    assert!(segs.len() >= 2, "注释行/空行在深度 0 可切：{segs:?}");
}

#[test]
fn segmentize_leading_blank_lines_merge_forward() {
    let segs = segmentize("\n\nhello\n");
    assert_eq!(segs.len(), 1, "文档开头空行并入首段：{segs:?}");
    assert_eq!(segs[0], "\n\nhello\n");
}

// ─── 依赖词法提取 ──────────────────────────────────────────────────────────

#[test]
fn deps_lexing_separates_reads_and_writes() {
    let deps = snapshot::lex_deps("\\global\\def\\foo{\\bar}\\let\\a=\\b \\count0=1");
    // 定义类命令与其前缀是读；其后的 cs 是写目标
    assert!(deps.read_cs.contains("global"), "{:?}", deps.read_cs);
    assert!(deps.read_cs.contains("def"), "{:?}", deps.read_cs);
    assert!(deps.write_cs.contains("foo"), "{:?}", deps.write_cs);
    // \def 宏体内引用的 \bar 计入读
    assert!(deps.read_cs.contains("bar"), "{:?}", deps.read_cs);
    // \let\a=\b：\a 写、\b 读
    assert!(deps.write_cs.contains("a"), "{:?}", deps.write_cs);
    assert!(deps.read_cs.contains("b"), "{:?}", deps.read_cs);
    // \count 读依赖（寄存器值变化由 ValueState 的版本戳兜底）
    assert!(deps.read_cs.contains("count"), "{:?}", deps.read_cs);
    assert!(!deps.dynamic_cs);
}

#[test]
fn deps_lexing_flags_csname_as_dynamic() {
    let deps = snapshot::lex_deps("x \\csname foo\\endcsname y");
    assert!(deps.dynamic_cs);
}

// ─── 全量 vs 增量：逐位一致 ────────────────────────────────────────────────

#[test]
fn incremental_matches_full_two_segments() {
    // 段 A 定义宏，段 B 使用（plan.md §7 验收口径：增量结果与全量重编逐位一致）
    let doc = "\\def\\hello{World}\n\nHello \\hello.\n\n";
    assert_eq!(incr_text(doc), full_text(doc));
}

#[test]
fn incremental_matches_full_across_many_documents() {
    for doc in [
        "plain text only\n",
        "\\def\\a{A}\n\n\\a \\a \\a.\n\n",
        "\\count0=7\n\nthe count is \\the\\count0.\n\n",
        "{scoped \\def\\a{inner}}\n\nouter \\a\n\n",
        "\\hbox{box content}\n\nafter box\n\n",
        "$x + y$ and $${z}$$\n\nafter math\n\n",
        "\\everypar{[start]}\n\nparagraph text\n\nmore\n\n",
        "\\parshape 1 0pt 10pt\n\nshaped\n\n",
        "\\catcode`\\@=11 \\my@private\n\nafter catcode\n\n",
        "\\toks0={toks body}\n\n\\the\\toks0\n\n",
        "\\font\\tiny=cmr10\n\n\\tiny tiny text\n\n",
        "line one\nline two\n\nline three\n\n\n\nlast\n",
    ] {
        assert_eq!(incr_text(doc), full_text(doc), "增量 != 全量：{doc:?}");
    }
}

// ─── 编辑：缓存保留 ────────────────────────────────────────────────────────

#[test]
fn edit_later_segment_keeps_earlier_cache() {
    let doc = "\\def\\hello{World}\n\nHello \\hello.\n\n";
    let mut eng = SegmentEngine::new();
    eng.run(doc);
    assert_eq!(eng.stats().executed, 2);
    assert_eq!(eng.text(), full_text(doc));

    let edited = "Hello \\hello again.\n\n";
    let results = eng.edit(1, edited).expect("段下标合法");
    // 段 0 未被触碰 → 缓存保留
    assert!(eng.is_cached(0));
    assert!(results[0].from_cache, "未编辑且在前面的段必须复用缓存");
    assert!(!results[1].from_cache);
    assert_eq!(eng.stats().executed, 3, "只重算被编辑的段 1");
    assert_eq!(eng.stats().reused, 0);
    assert_eq!(
        eng.stats().restarts,
        0,
        "段 1 状态中性 → 活状态仍等于 pre(1) → 廉价路径"
    );
    // 正确性：与"编辑后文档全量重跑"逐位一致
    assert_eq!(
        eng.text(),
        full_text("\\def\\hello{World}\n\nHello \\hello again.\n\n")
    );
}

#[test]
fn edit_out_of_range_is_error_not_panic() {
    let mut eng = SegmentEngine::new();
    eng.run("a\n\nb\n\n");
    let err = eng.edit(9, "x\n\n").expect_err("越界必须报错");
    assert!(err.to_string().contains("9"));
}

#[test]
fn long_document_edit_reuses_most_segments() {
    // 文档形态：段 0 = 宏定义段（有状态副作用），段 1..=n = 纯正文段（状态中性）。
    // 只改一个宏体 → 宏定义段必算，正文段读依赖未变 + 状态中性 → 全部复用。
    let n = 20usize;
    let defs = |k: Option<usize>| {
        (0..n)
            .map(|i| match k {
                Some(k) if i == k => format!("\\def\\w{}{{word-{k}-EDITED}}", cs_suffix(i)),
                _ => format!("\\def\\w{}{{word-{i}}}", cs_suffix(i)),
            })
            .collect::<String>()
    };
    let mut doc = format!("{}\n\n", defs(None));
    for i in 0..n {
        doc.push_str(&format!("Paragraph {i} says \\w{}.\n\n", cs_suffix(i)));
    }
    let mut eng = SegmentEngine::new();
    eng.run(&doc);
    assert_eq!(eng.segments().len(), n + 1);
    assert_eq!(eng.stats().executed, n + 1);
    assert_eq!(eng.text(), full_text(&doc));

    let k = 7;
    let edited = format!("{}\n\n", defs(Some(k)));
    let results = eng.edit(0, &edited).expect("段下标合法");
    let reused = results.iter().filter(|r| r.from_cache).count();
    let rejects = eng.last_rejects();
    let reasons: Vec<(usize, &str)> = (0..n + 1)
        .filter_map(|i| rejects[i].map(|r| (i, r)))
        .collect();
    // 段 k+1（正文段 k）引用被改宏 → 必重算；其余正文段全部复用
    assert_eq!(
        reused,
        n - 1,
        "只改一个宏体：仅引用它的正文段重算（拒绝原因 {reasons:?}）"
    );
    assert_eq!(
        rejects[k + 1],
        Some("读依赖闭包触及变化槽（该段引用的宏被改动）")
    );
    // 首轮 n+1 段全跑 + 编辑轮只重算 2 段（宏定义段 + 引用被改宏的正文段 k）
    assert_eq!(
        eng.stats().executed,
        n + 3,
        "编辑轮只重算宏定义段与其依赖段"
    );

    // 逐位一致：与编辑后文档全量重跑比较
    let mut edited_doc = doc.clone();
    let end = doc
        .match_indices("\n\n")
        .next()
        .map_or(doc.len(), |(p, _)| p + 2);
    edited_doc.replace_range(0..end, &edited);
    let mut ref_eng = SegmentEngine::new();
    ref_eng.run(&edited_doc);
    assert_eq!(eng.text(), ref_eng.text(), "增量结果必须与全量重跑逐位一致");
    assert!(eng.text().contains("word-7-EDITED"));
}

// ─── 编辑：依赖失效 ────────────────────────────────────────────────────────

#[test]
fn edit_macro_definition_invalidates_dependent_segment() {
    let doc = "\\def\\greet{Hi}\n\n\\greet there.\n\n";
    let mut eng = SegmentEngine::new();
    eng.run(doc);
    assert_eq!(eng.text(), full_text(doc));

    let results = eng.edit(0, "\\def\\greet{Hey}\n\n").expect("段下标合法");
    // 段 1 引用了被改动的宏 → 依赖失效，必须重算
    assert!(!results[1].from_cache, "依赖被改宏的后续段必须重算");
    assert_eq!(
        eng.text(),
        full_text("\\def\\greet{Hey}\n\n\\greet there.\n\n")
    );
}

#[test]
fn edit_unrelated_definition_keeps_independent_segment() {
    // 段 0 定义 \foo 与 \bar；段 1 只引用 \bar → 只改 \foo 不应使段 1 失效
    let doc = "\\def\\foo{FOO}\\def\\bar{BAR}\n\nuse \\bar.\n\n";
    let mut eng = SegmentEngine::new();
    eng.run(doc);
    assert_eq!(eng.text(), full_text(doc));

    let results = eng
        .edit(0, "\\def\\foo{FOO-EDITED}\\def\\bar{BAR}\n\n")
        .expect("段下标合法");
    assert!(results[1].from_cache, "未引用被改宏的后续段应复用缓存");
    assert_eq!(
        eng.text(),
        full_text("\\def\\foo{FOO-EDITED}\\def\\bar{BAR}\n\nuse \\bar.\n\n")
    );
}

#[test]
fn edit_related_definition_does_recompute() {
    // 与上一测试对照：改的正是被引用的 \bar → 必须重算
    let doc = "\\def\\foo{FOO}\\def\\bar{BAR}\n\nuse \\bar.\n\n";
    let mut eng = SegmentEngine::new();
    eng.run(doc);
    let results = eng
        .edit(0, "\\def\\foo{FOO}\\def\\bar{BAR-EDITED}\n\n")
        .expect("段下标合法");
    assert!(!results[1].from_cache, "被引用宏变化必须失效重算");
    assert!(eng.text().contains("BAR-EDITED"));
}

#[test]
fn csname_dependent_segment_is_conservatively_invalidated() {
    // \csname 可动态构造 cs 名 → 读依赖闭包不可靠 → 保守失效
    let doc = "\\def\\foo{FOO}\n\nx \\csname foo\\endcsname\n\n";
    let mut eng = SegmentEngine::new();
    eng.run(doc);
    assert_eq!(eng.text(), full_text(doc));
    let results = eng
        .edit(0, "\\def\\foo{FOO-EDITED}\n\n")
        .expect("段下标合法");
    assert!(!results[1].from_cache, "含 \\csname 的段必须保守重算");
    assert_eq!(
        eng.text(),
        full_text("\\def\\foo{FOO-EDITED}\n\nx \\csname foo\\endcsname\n\n")
    );
}

#[test]
fn register_write_invalidates_following_segments() {
    // 寄存器值变化走 ValueState 版本戳 → 全局失效（阶段一不做槽级归因）
    let doc = "\\count0=1\n\nthe count is \\the\\count0.\n\n";
    let mut eng = SegmentEngine::new();
    eng.run(doc);
    assert_eq!(eng.text(), full_text(doc));
    let results = eng.edit(0, "\\count0=2\n\n").expect("段下标合法");
    assert!(!results[1].from_cache, "寄存器变化必须失效后续段");
    assert_eq!(
        eng.text(),
        full_text("\\count0=2\n\nthe count is \\the\\count0.\n\n")
    );
}

// ─── 跨段构造：脏边界禁用缓存复用 ──────────────────────────────────────────

#[test]
fn unclean_boundary_disables_cache_reuse() {
    // 未闭合 `{` 跨段：边界不干净 → 之后一律重算（不复用缓存）
    let doc = "\\def\\x{A}\n\n{\\x\n\nmore \\x.\n\n";
    let mut eng = SegmentEngine::new();
    eng.run(doc);
    assert!(!eng.expander().boundary_is_clean(), "未闭合组 → 脏边界");
    let results = eng.edit(0, "\\def\\x{B}\n\n{\\x\n\n").expect("段下标合法");
    assert!(!results[1].from_cache, "脏边界后禁止复用缓存");
}

#[test]
fn unclosed_input_scan_divergence_is_documented_not_asserted() {
    // 未闭合 \if 的"跳过分支"会跨段吞输入——切段语义本就不保证与全量一致
    // （阶段二引擎级 checkpoint 取代文本切段）。此处只锁"不 panic、结果自洽"。
    let doc = "\\def\\x{A}\n\n\\iftrue skipped\n\nuses \\x.\n\n";
    let mut eng = SegmentEngine::new();
    let results = eng.run(doc);
    assert_eq!(results.len(), 3);
    let _ = eng.text();
}

// ─── 畸形输入契约：不 panic ────────────────────────────────────────────────

#[test]
fn malformed_input_no_panic_and_matches_full() {
    // 段内自闭合的畸形（错误恢复不吞跨段输入）→ 增量与全量逐位一致
    for doc in [
        "\\undefinedcs here\n\ntext\n\n",
        "bad % unterminated comment\n\ntext\n\n",
        "{unclosed brace\n\ntext\n\n",
        "$x\n\ny$\n\n",
        "\\hbox{unbalanced\n\ntext\n\n",
        "\\relax^^ff\n\ntext\n\n",
        "\\par\\par\\par\n\ntext\n\n",
        "\\def\\foo{}\n\n\\foo \\foo\n\n",
        "\\catcode`\\A=13 \\A\n\ntext\n\n",
        "\\lccode`\\a=`\\b\n\ntext\n\n",
    ] {
        assert_eq!(
            incr_text(doc),
            full_text(doc),
            "畸形输入下增量 != 全量：{doc:?}"
        );
    }
}

#[test]
fn malformed_input_across_segments_no_panic() {
    // 跨段吞输入的畸形（\if 跳过/数字扫描/\csname/\def 体扫描）→ 只锁不 panic
    for doc in [
        "",
        "\n\n\n",
        "\\iftrue x\n\ny\n\n",
        "\\count0=\n\n5\n\n",
        "\\csname junk\n\ny\n\n",
        "\\def\\foo{\n\n}\n\n",
        "\\message{open\n\n}\n\n",
        "\\uppercase{\\a\n\n}\n\n",
        "\\end\n\nafter end\n\n",
        "\\input{nonexistent-file}\n\ntext\n\n",
    ] {
        let _ = incr_text(doc);
    }
}

// ─── 基准（M5 阶段二：可回滚段快照 + 单路径 edit）─────────────────────────
// 场景 A（改正文中段，状态中性）与场景 B（改宏体，引入状态偏差）各测一次。
// 实测（501 段，2 核受限 VM，dev profile，多次取值 4.9~5.3 / 10.6~11.2ms）：
//   阶段一（0099b1f，两条路径 + 全状态逐段判定）：全量 294.1ms；场景 A 39.0ms
//   （7.6x）；场景 B 107.2ms（2.7x，保底路径重建状态链）。
//   阶段二（回滚到段前 + 链偏差判定）：全量 ~275-295ms（持平）；场景 A ~5ms
//   （~56x）；场景 B ~11ms（~26x）；restarts = 0（不再重建引擎）。
// 剩余开销：每次执行段后一次链偏差全量扫描（value_state 指纹 + eqtb 全槽比较）、
// 每段一次 run()（含看门狗线程）、每段边界两份完整检查点的捕获。离 100x 还差：
// 偏差集的增量维护（免全量扫描）、副作用边界（RFC-3）、值状态槽级归因。

#[test]
#[ignore = "基准（M5 阶段二）：cargo test -p ntex-core --lib incremental -- --ignored --nocapture"]
fn bench_edit_one_segment_vs_full() {
    let n_body = 500usize;
    let defs = (0..n_body)
        .map(|i| format!("\\def\\w{}{{word-{i}}}", cs_suffix(i)))
        .collect::<String>();
    let mut doc = format!("{defs}\n\n");
    for i in 0..n_body {
        doc.push_str(&format!("Paragraph {i} mentions \\w{}.\n\n", cs_suffix(i)));
    }
    let n_seg = n_body + 1;

    // 基线：全量（全新引擎逐段执行）
    let t = Instant::now();
    let mut full = SegmentEngine::new();
    let results = full.run(&doc);
    let full_elapsed = t.elapsed();
    assert_eq!(results.len(), n_seg);

    // 场景 A：改正文中段的措辞（状态中性 → 重放后零偏差，其后各段免判定复用）
    let mid = n_body / 2;
    let para_a = format!(
        "Paragraph {mid} mentions \\w{} (revised).\n\n",
        cs_suffix(mid)
    );
    let t = Instant::now();
    let res_a = full.edit(mid + 1, &para_a).expect("段下标合法");
    let elapsed_a = t.elapsed();
    let reused_a = res_a.iter().filter(|r| r.from_cache).count();
    // 正确性：增量 == 编辑后文档全量重跑（逐位一致）
    let mut doc_a = doc.clone();
    let start_a = doc
        .match_indices("\n\n")
        .nth(mid)
        .map_or(doc.len(), |(p, _)| p + 2);
    let end_a = doc
        .match_indices("\n\n")
        .nth(mid + 1)
        .map_or(doc.len(), |(p, _)| p + 2);
    doc_a.replace_range(start_a..end_a, &para_a);
    let mut refull_a = SegmentEngine::new();
    refull_a.run(&doc_a);
    assert_eq!(full.text(), refull_a.text(), "场景 A：增量 != 全量");

    // 场景 B：改宏定义段里的一个宏体（编辑引入状态偏差 → 回滚到段 0 前重放，
    // 后续段按链偏差依赖判定复用）
    let defs_b = (0..n_body)
        .map(|i| match i == mid {
            true => format!("\\def\\w{}{{word-{mid}-REVISED}}", cs_suffix(i)),
            false => format!("\\def\\w{}{{word-{i}}}", cs_suffix(i)),
        })
        .collect::<String>();
    let para_b = format!("{defs_b}\n\n");
    let t = Instant::now();
    let res_b = full.edit(0, &para_b).expect("段下标合法");
    let elapsed_b = t.elapsed();
    let reused_b = res_b.iter().filter(|r| r.from_cache).count();
    // 场景 B 的参考文档建立在场景 A 之后的文档上（编辑是累积的）
    let mut doc_b = doc_a.clone();
    let end_b = doc_a
        .match_indices("\n\n")
        .next()
        .map_or(doc_a.len(), |(p, _)| p + 2);
    doc_b.replace_range(0..end_b, &para_b);
    let mut refull_b = SegmentEngine::new();
    refull_b.run(&doc_b);
    assert_eq!(full.text(), refull_b.text(), "场景 B：增量 != 全量");

    let ratio = |a: std::time::Duration, b: std::time::Duration| {
        b.as_secs_f64().max(1e-9) / a.as_secs_f64().max(1e-9)
    };
    println!("── M5 阶段二基准（段级增量 vs 全量，{n_seg} 段）─────────────");
    println!(
        "全量（全新引擎逐段执行）      : {:>9.2} ms",
        full_elapsed.as_secs_f64() * 1e3
    );
    println!(
        "场景 A 改正文 1 段（零偏差复用）: {:>9.2} ms  加速 {:>6.1}x  复用 {reused_a}/{n_seg}",
        elapsed_a.as_secs_f64() * 1e3,
        ratio(elapsed_a, full_elapsed)
    );
    println!(
        "场景 B 改宏体（回滚+依赖判定） : {:>9.2} ms  加速 {:>6.1}x  复用 {reused_b}/{n_seg}",
        elapsed_b.as_secs_f64() * 1e3,
        ratio(elapsed_b, full_elapsed)
    );
    println!("引擎统计                      : {:?}", full.stats());
    println!("剩余开销：每次执行段后一次链偏差全量扫描（指纹 + eqtb 全槽）、每段一次 run()；");
    println!("离 100x：偏差集增量维护、副作用边界（RFC-3）、值状态槽级归因。");
    println!("────────────────────────────────────────────────────────────");
    assert!(reused_a >= n_seg - 2, "场景 A 应几乎全部复用：{reused_a}");
    assert!(reused_b >= n_seg - 2, "场景 B 应几乎全部复用：{reused_b}");
    // 单路径：基准里的两类编辑都不再重建引擎（阶段一场景 B 走保底路径 restarts=1）
    assert_eq!(full.stats().restarts, 0, "编辑不得重建引擎");
}

#[test]
fn non_letter_macro_name_invalidation() {
    // 词法依赖对非字母宏名（\foo@bar，@ 经 \catcode 11 为 letter）必须**超近似**：
    // 文本层无法知 catcode——字母前缀 \foo 与整段名 \foo@bar 双记，只多不少。
    // 回归：修复前 lex_deps 只收字母前缀 \foo，编辑 \foo@bar 定义后引用段
    // 错误复用缓存输出 old（增量 != 全量，违反逐位一致铁律）；tmp 测试实证后
    // lex_deps 保守延伸定界符修复（2026-09-03 验收）。
    let mut eng = SegmentEngine::new();
    let doc = r"\catcode`@=11\relax
\def\foo@bar{old}

\foo@bar
";
    eng.run(doc);
    assert_eq!(
        eng.text().trim(),
        "old",
        "基线输出应为 old,实际 {:?}",
        eng.text()
    );
    eng.edit(
        0,
        r"\catcode`@=11\relax
\def\foo@bar{new}",
    )
    .unwrap();
    let after = eng.text();
    assert_eq!(
        after.trim(),
        "new",
        "编辑宏体后末段应失效重算输出 new(若输出 old = 词法漏记 foo@bar 依赖)"
    );
}

// ─── M5 阶段二：检查点回滚 ─────────────────────────────────────────────────
//
// 阶段二把 `edit` 从"两条路径"（活状态未推进 → 前缀复用；已推进 → 全新引擎
// 重建）改为**一条路径**：引擎整体回滚到段前快照，再从该段起重放。回滚的
// 正确性 = 检查点字段完整 + 还原逐位复原，以下测试分别锁：

/// 编辑序列的逐位一致校验：每次编辑后，增量输出 == "累积编辑后的文档全量重跑"。
///
/// 参考文档 = 各段源码（含替换后的）按序拼接——与 `edit` 替换单段源码的口径
/// 一致（段数不变）。
fn assert_edits_match_full(doc: &str, edits: &[(usize, &str)]) {
    let mut eng = SegmentEngine::new();
    eng.run(doc);
    let mut segs = segmentize(doc);
    for &(idx, src) in edits {
        assert!(
            idx < segs.len(),
            "段下标 {idx} 越界（共 {} 段）",
            segs.len()
        );
        segs[idx] = src.to_owned();
        eng.edit(idx, src).expect("段下标合法");
        let reference = segs.concat();
        let mut full = SegmentEngine::new();
        full.run(&reference);
        assert_eq!(
            eng.text(),
            full.text(),
            "编辑段 {idx} 后 增量 != 全量（编辑序列 {edits:?}）"
        );
        assert_eq!(
            eng.segments().concat(),
            reference,
            "段列表拼接必须与参考文档一致"
        );
    }
}

#[test]
fn checkpoint_roundtrip_restores_value_state() {
    // 检查点完整性（回滚健全性的根）：对每类状态改动做
    // 捕获 → 用"泥沙"改动污染全部状态类别 → 还原 → value_state/eqtb 必须逐位
    // 复原。`value_state` 的 `runtime_digest` 覆盖 catcode/sfcode/寄存器以外的
    // 全部指纹字段——检查点漏掉任何一个字段，还原后指纹就对不上。
    use crate::expand::Expander;
    let snippets = [
        "\\count0=7 \\dimen1=9pt \\skip2=3pt plus 1fil \\toks4={toks body}",
        "\\catcode`\\@=11 \\sfcode`\\.=3000",
        "\\lccode`\\a=`\\b \\uccode`\\x=`\\Y \\mathcode`\\+=1234 \\delcode`\\|=5678",
        "\\everypar{[p]} \\everymath{[m]} \\everyhbox{[h]} \\everyvbox{[v]}",
        "\\everycr{[c]} \\everydisplay{[d]} \\errhelp{[e]}",
        "\\hsize=100pt \\tolerance=99 \\parindent=1pt \\vsize=500pt \\topskip=12pt",
        "\\parshape 2 0pt 10pt 1pt 9pt \\interlinepenalties 2 10 20",
        "\\thinmuskip=3mu \\output{\\relax}",
    ];
    // 泥沙：把每个状态类别都改成"捕获值以外的值"（与 snippets 中的值均不同）
    let mud = "\\count0=1 \\dimen1=2pt \\skip2=4pt \\toks4={m} \\catcode`\\@=12 \
               \\sfcode`\\.=2000 \\lccode`\\a=`\\z \\uccode`\\x=`\\W \\mathcode`\\+=999 \
               \\delcode`\\|=111 \\everypar{Q} \\everymath{Q} \\everyhbox{Q} \\everyvbox{Q} \
               \\everycr{Q} \\everydisplay{Q} \\errhelp{Q} \\hsize=1pt \\tolerance=1 \
               \\parindent=2pt \\vsize=3pt \\topskip=4pt \\parshape 1 2pt 3pt \
               \\interlinepenalties 1 7 \\thinmuskip=5mu \\output{}";
    for src in snippets {
        let mut e = Expander::new();
        let _ = e.run_source(src);
        let value = e.value_state();
        let eqtb = e.eqtb().slots().to_vec();
        let cp = e.capture_checkpoint();
        let _ = e.run_source(mud);
        e.restore_checkpoint(&cp);
        assert_eq!(
            e.value_state(),
            value,
            "检查点还原不完整（值状态/指纹不符）：{src}"
        );
        assert!(e.eqtb().slots() == eqtb, "检查点还原不完整（eqtb）：{src}");
    }
}

#[test]
fn edit_stateful_segment_rollback_matches_full() {
    // 编辑带状态副作用的段（寄存器赋值）：回滚到段前后重放，后续段必须读到
    // 新值。阶段一这条路径要重建整个引擎（501 段文档全量重放）。
    assert_edits_match_full(
        "\\def\\pre{P}\n\n\\count0=1 \\pre\n\nthe count is \\the\\count0.\n\n",
        &[(1, "\\count0=42 \\pre\n\n")],
    );
    // 寄存器赋值段不动、只改它前面的正文：段 1 的副作用必须原样重现
    assert_edits_match_full(
        "hello\n\n\\count0=3\n\nvalue is \\the\\count0.\n\n",
        &[(0, "hello world\n\n")],
    );
}

#[test]
fn sync_reuse_of_stateful_segment_restores_its_effect() {
    // 同步态复用有状态副作用的段：跳过执行必须把它写的值"补上"（还原其 post
    // 检查点），否则后续段读到旧值。阶段一无状态回滚，只能把非中性段整体拒算。
    let doc = "hello\n\n\\count0=3\n\nvalue is \\the\\count0.\n\n";
    let mut eng = SegmentEngine::new();
    eng.run(doc);
    let results = eng.edit(0, "hello world\n\n").expect("段下标合法");
    // 正文编辑不引入状态偏差 → 段 1、2 同步复用（含有副作用的段 1）
    assert!(results[1].from_cache, "零偏差时有副作用段也可复用");
    assert!(results[2].from_cache);
    assert_eq!(eng.stats().reused, 2);
    assert!(
        eng.text().contains("value is 3."),
        "复用段的状态副作用必须生效：{:?}",
        eng.text()
    );
}

#[test]
fn edit_catcode_rollback_matches_full() {
    // \catcode 属值状态：编辑改动 catcode 的段，回滚必须把它还原，后续段的
    // token 化才能复现全量结果。
    assert_edits_match_full(
        "\\catcode`\\@=11 \\def\\my@macro{PRIVATE}\n\n\\my@macro\n\n",
        &[(0, "\\catcode`\\@=11 \\def\\my@macro{CHANGED}\n\n")],
    );
    assert_edits_match_full(
        "\\catcode`\\@=11 \\def\\my@macro{PRIVATE}\n\n\\my@macro\n\n",
        &[(1, "\\my@macro plus \\my@macro\n\n")],
    );
}

#[test]
fn edit_rollback_then_earlier_edit_matches_full() {
    // 连续多次编辑，且后一次编辑的段在更前面（k 之后又 j<k）：每次都要回滚到
    // 对应段执行前，任何一次状态没回滚干净都会在结果里留痕。
    assert_edits_match_full(
        "\\def\\a{1}\n\n\\def\\b{\\a}\n\n\\count0=5 \\b\n\nvalue \\the\\count0.\n\n",
        &[
            (3, "\\count0=7 \\b\n\n"),
            (1, "\\def\\b{\\a TWO}\n\n"),
            (0, "\\def\\a{ONE}\n\n"),
            (2, "\\count0=8 \\b \\count1=9\n\n"),
        ],
    );
}

#[test]
fn edits_never_restart_engine() {
    // 单路径化：改正文/改宏体/改 catcode/改寄存器都不再重建引擎（restarts 恒 0）。
    let doc = "\\def\\a{A}\n\n\\count0=1\n\n\\catcode`\\@=11\n\nuse \\a \\the\\count0.\n\n";
    let mut eng = SegmentEngine::new();
    eng.run(doc);
    for (idx, src) in [
        (3usize, "use \\a \\the\\count0 again.\n\n"),
        (0, "\\def\\a{B}\n\n"),
        (1, "\\count0=2\n\n"),
        (2, "\\catcode`\\@=12\n\n"),
    ] {
        eng.edit(idx, src).expect("段下标合法");
        assert_eq!(eng.stats().restarts, 0, "编辑段 {idx} 走了重建路径");
    }
}

#[test]
fn edit_keeps_injected_vfs() {
    // 单路径化的另一面：编辑不再 new 一个引擎，用户注入的环境（自定义 VFS、
    // 预载 `.fmt`）在编辑后仍然有效。阶段一保底路径会把它们一起丢掉。
    let mut vfs = ntex_io::MemVfs::new();
    vfs.insert("chap.tex", "chapter body");
    let mut eng = SegmentEngine::new();
    eng.expander_mut().set_vfs(Box::new(vfs));
    let doc = "\\def\\part{ONE}\n\n\\input{chap}\n\n\\part\n\n";
    eng.run(doc);
    assert!(eng.text().contains("chapter body"), "{:?}", eng.text());
    eng.edit(0, "\\def\\part{TWO}\n\n").expect("段下标合法");
    assert!(
        eng.text().contains("chapter body"),
        "编辑后 VFS 失效 = 引擎被重建：{:?}",
        eng.text()
    );
    // 引擎对象未被替换（仍是注入 MemVfs 的那个）
    let mut taken = eng.expander_mut().take_vfs();
    let vfs = taken
        .as_any_mut()
        .downcast_mut::<ntex_io::MemVfs>()
        .expect("VFS 变回默认 LocalVfs = 走了重建路径");
    assert!(vfs.get("chap.tex").is_some());
}

#[test]
fn edit_inside_unclosed_group_rolls_back_control_state() {
    // 跨段构造（悬挂的数学模式 `$`）：段 1 进入数学态未退出，段 2 在悬挂态下
    // 执行。编辑段 2 → 回滚必须把悬挂的控制状态一并还原（控制状态进检查点），
    // 重放语义才与全量一致。未闭合 `{` 同理，只是它同时改变切段深度（后段并入）。
    let doc = "\\def\\v{V}\n\n$\\v\n\ninside \\v.\n\n";
    let mut eng = SegmentEngine::new();
    eng.run(doc);
    assert_eq!(eng.segments().len(), 3, "{:?}", eng.segments());
    assert!(
        !eng.expander().boundary_is_clean(),
        "悬挂 $ → 脏边界（前置条件）"
    );
    eng.edit(2, "inside EDITED \\v.\n\n").expect("段下标合法");
    let mut segs = segmentize(doc);
    segs[2] = "inside EDITED \\v.\n\n".to_owned();
    let mut full = SegmentEngine::new();
    full.run(&segs.concat());
    assert_eq!(eng.text(), full.text(), "悬挂态内的段编辑后 增量 != 全量");

    // 未闭合 `{`：悬挂组与其后内容并入同段，编辑该段 = 回滚到干净边界后重算
    let doc2 = "\\def\\v{V}\n\n{\\v\n\ninside \\v.\n\n";
    let mut eng2 = SegmentEngine::new();
    eng2.run(doc2);
    assert!(!eng2.expander().boundary_is_clean());
    eng2.edit(1, "{\\v DEEPER\n\ninside EDITED \\v.\n\n")
        .expect("段下标合法");
    let mut segs2 = segmentize(doc2);
    segs2[1] = "{\\v DEEPER\n\ninside EDITED \\v.\n\n".to_owned();
    let mut full2 = SegmentEngine::new();
    full2.run(&segs2.concat());
    assert_eq!(eng2.text(), full2.text(), "悬挂组内编辑后 增量 != 全量");
}

#[test]
fn edit_sweep_all_segments_matches_full() {
    // 系统化抽测：对文档每个段 × 每类编辑（正文/宏体/寄存器赋值/catcode/\everypar）
    // 各做一次单段编辑，增量必须与"编辑后文档全量重跑"逐位一致，且都不走重建。
    let base = [
        "\\def\\w{one}",
        "\\count0=5",
        "\\catcode`\\@=11",
        "plain text \\w",
        "count is \\the\\count0",
        "\\def\\w{two} \\w",
    ];
    let variants = [
        "\\def\\w{EDITED}",
        "\\count0=9",
        "\\catcode`\\@=12",
        "EDITED text",
        "x",
        "\\everypar{[p]}",
    ];
    for idx in 0..base.len() {
        for variant in variants {
            // 段体各带一个空行边界，保证切段结果 = base 的每行一段
            let mut segs = base.iter().map(|s| format!("{s}\n\n")).collect::<Vec<_>>();
            segs[idx] = format!("{variant}\n\n");
            let doc = base.iter().map(|s| format!("{s}\n\n")).collect::<String>();
            let mut eng = SegmentEngine::new();
            eng.run(&doc);
            assert_eq!(eng.segments().len(), base.len(), "前置条件：切段数");
            eng.edit(idx, &segs[idx]).expect("段下标合法");
            let mut full = SegmentEngine::new();
            full.run(&segs.concat());
            assert_eq!(
                eng.text(),
                full.text(),
                "段 {idx} 编辑为 {variant:?} 后 增量 != 全量"
            );
            assert_eq!(eng.stats().restarts, 0, "段 {idx} 走了重建路径");
        }
    }
}

#[test]
fn edit_segments_never_run_before_is_error_free() {
    // 编辑从未执行到的段（前一轮在更早的段出错终止）：回滚目标退到最近的更早
    // 缓存段；错误段本身未修复时缓存错误原样复现，修复后后续段能正常跑完。
    let doc = "\\def\\ok{fine}\n\n\\input{no-such-file}\n\ntail \\ok.\n\n";
    let mut eng = SegmentEngine::new();
    let results = eng.run(doc);
    assert_eq!(results.len(), 2, "出错段之后的段不产出");
    assert!(results[1].error.is_some(), "前置条件：段 1 出错");
    assert!(!eng.is_cached(2), "段 2 从未执行 → 无缓存");
    // 编辑其后从未跑过的段：重放须从回滚目标起补跑中间段（输出不得缺段）
    let results = eng.edit(2, "tail EDITED \\ok.\n\n").expect("编辑未跑段");
    assert_eq!(results.len(), 2, "出错段仍终止后续段");
    let mut segs = segmentize(doc);
    segs[2] = "tail EDITED \\ok.\n\n".to_owned();
    let mut full = SegmentEngine::new();
    let reference = full.run(&segs.concat());
    assert_eq!(reference.len(), 2, "全量同样停在出错段");
    assert_eq!(eng.text(), full.text(), "编辑未跑段后 增量 != 全量");
    // 修复出错段：其后从未跑过的段随重放执行
    let results = eng
        .edit(1, "\\def\\fixed{fixed} \\fixed\n\n")
        .expect("编辑失败段");
    assert_eq!(results.len(), 3, "错误修复后后续段应产出");
    assert_eq!(
        eng.text(),
        full_text("\\def\\ok{fine}\n\n\\def\\fixed{fixed} \\fixed\n\ntail EDITED \\ok.\n\n"),
    );
}
