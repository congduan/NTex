// ---------- 测试分片：incremental_tests.rs（M5 阶段三：端到端增量排版） ----------
//
// 铁律：编辑 1 段后的增量 DVI 与全量 DVI **逐位一致**。比较口径三重：
// 1. 管线增量 `edit` == 现有全量路径 `Typesetter::typeset_dvi`（字节级口径见 ntex-dvi 测试）；
// 2. 管线增量 `edit` == 管线全新 `compile`（编辑后文档）；
// 3. 管线 `compile` == 现有全量路径（管线自身未偏离引擎语义）。
// 文档为真实排版（TFM 度量 + 行内/显示公式 + 列表 + 多页断页）。

#[cfg(test)]
mod incremental_tests {
    use crate::typeset::{IncrementalTypesetter, Typesetter};
    use std::time::Instant;

    /// 定位含 cmr10.tfm 的目录：NTEX_TFM_DIR 已指到即用；否则试 ~/.ntex-fonts。
    /// 找不到时不覆盖环境（文档退化为 nullfont 全零度量，逐位一致仍成立）。
    fn ensure_tfm_dir() {
        if std::env::var("NTEX_TFM_DIR")
            .ok()
            .is_some_and(|d| std::path::Path::new(&d).join("cmr10.tfm").exists())
        {
            return;
        }
        let home = std::env::var("HOME").unwrap_or_default();
        let p = std::path::PathBuf::from(home).join(".ntex-fonts");
        if p.join("cmr10.tfm").exists() {
            std::env::set_var("NTEX_TFM_DIR", &p);
        }
    }

    /// cmr10 可用时的导言（真实度量 + 数学字体族）；不可用时返回空串
    /// （字符用 nullfont 全零度量，管线/全量同样退化，比较口径不变）。
    fn font_preamble() -> String {
        ensure_tfm_dir();
        let dir = std::env::var("NTEX_TFM_DIR").unwrap_or_default();
        if !dir.is_empty() && std::path::Path::new(&dir).join("cmr10.tfm").exists() {
            "\\font\\tenrm=cmr10\n\\tenrm\n\\textfont0=\\tenrm \\scriptfont0=\\tenrm \
             \\scriptscriptfont0=\\tenrm\n"
                .to_string()
        } else {
            String::new()
        }
    }

    /// 词表（真实排版的填充文本；确定性）。
    const WORDS: [&str; 24] = [
        "the", "engine", "keeps", "checkpoint", "each", "paragraph", "boundary", "editing",
        "segment", "only", "retypesets", "affected", "lines", "reassembles", "pages", "glue",
        "penalty", "breakpoint", "boxes", "baseline", "measure", "width", "height", "depth",
    ];

    /// 第 `i` 段正文（确定性伪随机组句；`revise` 注入标记词）。
    fn body(i: usize, revise: bool) -> String {
        let mut s = String::new();
        for sent in 0..4 {
            for w in 0..10 {
                let word = WORDS[(i * 7 + sent * 3 + w * 5) % WORDS.len()];
                s.push_str(word);
                s.push(' ');
            }
            if revise && sent == 1 {
                s.push_str("REVISED ");
            }
            s.pop();
            s.push_str(". ");
        }
        s
    }

    /// 生成多页真实排版文档：`n_para` 个正文段，穿插行内公式/显示公式/列表。
    fn build_doc(n_para: usize) -> String {
        let mut s = String::new();
        s.push_str("\\hsize 300pt\n\\vsize 320pt\n\\parindent 20pt\n");
        s.push_str(&font_preamble());
        s.push_str("\\def\\topic#1{\\par\\vskip 6pt\\noindent #1\\par}\n");
        s.push('\n');
        for i in 0..n_para {
            match i % 7 {
                // 列表段：3 个条目（hangindent 条目排版）
                3 => {
                    for k in 0..3 {
                        s.push_str(&format!(
                            "\\hangindent 20pt\\hangafter 1\\noindent --{} item {} {}\n\n",
                            WORDS[(i + k) % WORDS.len()],
                            i,
                            k
                        ));
                    }
                }
                // 显示公式段：公式独立成行（上下间距 + 公式盒参与断页）
                5 => {
                    s.push_str(&body(i, false));
                    s.push_str("\n$${a \\over b} + x_i = y_i$$\n");
                    s.push_str(&body(i + 1, false));
                    s.push_str("\n\n");
                }
                // 行内公式段
                2 => {
                    s.push_str(&body(i, false));
                    s.push_str(" Inline math $x_i + y_i = z_i$ ends here.\n\n");
                }
                _ => {
                    s.push_str(&body(i, false));
                    s.push_str("\n\n");
                }
            }
            if i % 9 == 0 {
                s.push_str(&format!("\\topic{{Section {}}}\n\n", i / 9));
            }
        }
        s
    }

    /// 参考输出：页面列表 + 字体表——`write_dvi(pages, fonts)` 的全部输入，
    /// 二者一致 ⇒ DVI 字节一致（确定性纯函数）。字节级铁律另由 ntex-dvi 的
    /// `tests/incremental_dvi.rs` 直接比较 DVI 字节（dev 依赖环使本模块不能
    /// 链接 ntex-dvi：lib 会被构建两次、类型不互通）。
    type Doc = (Vec<crate::node::BoxNode>, Vec<ntex_font::FontMetrics>);

    /// 节点层逐位一致断言。
    fn assert_doc_eq(out: &super::CompileOutput, reference: &Doc, ctx: String) {
        assert_eq!(
            &out.pages, &reference.0,
            "{ctx}: 页面列表不一致（增量 != 全量）"
        );
        assert_eq!(&out.fonts, &reference.1, "{ctx}: 字体表不一致");
    }

    /// 现有全量路径（`Typesetter::typeset_dvi`）。
    fn full_path(src: &str) -> Doc {
        let mut ts = Typesetter::with_tfm_paginated();
        let (pages, fonts) = ts.typeset_dvi(src).expect("全量排版");
        assert!(pages.len() >= 2, "文档应产出 2 页以上，实际 {} 页", pages.len());
        (pages, fonts)
    }

    /// 管线全量 `compile`。
    fn pipeline_compile(src: &str) -> (IncrementalTypesetter, Doc) {
        let mut it = IncrementalTypesetter::with_tfm_paginated();
        let out = it.compile(src).expect("管线编译");
        assert!(
            out.pages.len() >= 2,
            "文档应产出 2 页以上，实际 {}",
            out.pages.len()
        );
        (it, (out.pages, out.fonts))
    }

    /// 段 `k` 在文档中的字节区间（各段拼接与原文逐字节一致——segmentize 不变式）。
    fn segment_span(segments: &[String], k: usize) -> std::ops::Range<usize> {
        let mut start = 0;
        for (i, s) in segments.iter().enumerate() {
            if i == k {
                return start..start + s.len();
            }
            start += s.len();
        }
        panic!("段下标 {k} 越界（共 {} 段）", segments.len());
    }

    /// 把文档第 `k` 段替换为 `new_text`（编辑后参考文档）。
    fn replace_segment(doc: &str, segments: &[String], k: usize, new_text: &str) -> String {
        let span = segment_span(segments, k);
        let mut out = String::with_capacity(doc.len());
        out.push_str(&doc[..span.start]);
        out.push_str(new_text);
        out.push_str(&doc[span.end..]);
        out
    }

    /// 编辑第 `k` 段（改正文措辞，状态中性）：三重口径逐位一致 + 复用发生。
    fn assert_edit_matches_full(n_para: usize, k: usize) {
        let doc = build_doc(n_para);
        let (mut it, dvi0) = pipeline_compile(&doc);
        assert_eq!(
            dvi0,
            full_path(&doc),
            "口径 3：管线全量 != 引擎全量（{n_para} 段文档）"
        );
        let segments = it.segments().to_vec();
        let seg_count = segments.len();
        // 编辑后文本：同段长度的正文 + REVISED 标记（状态中性，页面断点可能后移）
        let new_text = format!("{}\n", body(k, true));
        let out = it.edit(k, &new_text).expect("编辑合法段");
        let doc_edited = replace_segment(&doc, &segments, k, &new_text);
        assert_doc_eq(
            &out,
            &full_path(&doc_edited),
            format!("口径 1：增量 != 全量（编辑第 {k}/{seg_count} 段）"),
        );
        let (_, dvi_full) = pipeline_compile(&doc_edited);
        assert_doc_eq(
            &out,
            &dvi_full,
            format!("口径 2：增量 != 管线全量（编辑第 {k}/{seg_count} 段）"),
        );
        assert!(
            it.stats().reused > 0,
            "改正文编辑应复用其后各段缓存（实际 reused={}）",
            it.stats().reused
        );
    }

    #[test]
    fn pipeline_compile_matches_full_path() {
        let doc = build_doc(30);
        let (_, dvi) = pipeline_compile(&doc);
        assert_eq!(dvi, full_path(&doc), "口径 3：管线全量 != 引擎全量");
    }

    #[test]
    fn edit_first_macro_section_matches_full() {
        // 首段（导言/宏定义段）：改 \hsize —— 状态偏差，其后保守全量重排，
        // 但仍须与全量逐位一致。段 0 是整个导言（\hsize/\font/\def 连成一段），
        // 编辑须保留字体与 \topic 定义、只改 \hsize——删掉它们文档退化为
        // nullfont，全量参考本身不足 2 页，断言口径失效。
        let doc = build_doc(24);
        let (mut it, _) = pipeline_compile(&doc);
        let segments = it.segments().to_vec();
        let original_preamble = segments[0].clone();
        let new_preamble = original_preamble.replace("\\hsize 300pt", "\\hsize 280pt");
        assert_ne!(new_preamble, original_preamble, "导言应含 \\hsize 300pt 待改");
        let out = it.edit(0, &new_preamble).expect("编辑段 0");
        let doc_edited = replace_segment(&doc, &segments, 0, &new_preamble);
        assert_doc_eq(&out, &full_path(&doc_edited), "首段宏定义编辑：增量 != 全量".to_string());
    }

    #[test]
    fn edit_first_body_paragraph_matches_full() {
        assert_edit_matches_full(30, 2);
    }

    #[test]
    fn edit_middle_paragraph_matches_full() {
        assert_edit_matches_full(30, 12);
    }

    #[test]
    fn edit_last_paragraph_matches_full() {
        let doc = build_doc(30);
        let (mut it, _) = pipeline_compile(&doc);
        let segments = it.segments().to_vec();
        let last = segments.len() - 1;
        let new_text = format!("{}\n", body(last, true));
        let out = it.edit(last, &new_text).expect("编辑末段");
        let doc_edited = replace_segment(&doc, &segments, last, &new_text);
        assert_doc_eq(&out, &full_path(&doc_edited), "末段编辑：增量 != 全量".to_string());
    }

    #[test]
    fn edit_math_and_list_paragraphs_match_full() {
        // 含公式/列表的段（build_doc 里 i%7 == 2/3/5 的段）逐个编辑并还原
        let doc = build_doc(30);
        let (mut it, _) = pipeline_compile(&doc);
        let segments = it.segments().to_vec();
        let mut checked = 0;
        for k in 0..segments.len() {
            let seg = segments[k].clone();
            let is_math = seg.contains('$');
            let is_list = seg.contains("\\hangindent");
            if !is_math && !is_list {
                continue;
            }
            let new_text = if is_list {
                format!(
                    "\\hangindent 20pt\\hangafter 1\\noindent --{} item {} 9\n",
                    WORDS[k % WORDS.len()],
                    k
                )
            } else {
                format!("{} Revised math $p_q + r_s = t_u$ here.\n", body(k, true))
            };
            let out = it.edit(k, &new_text).expect("编辑公式/列表段");
            let doc_edited = replace_segment(&doc, &segments, k, &new_text);
            assert_doc_eq(
                &out,
                &full_path(&doc_edited),
                format!("公式/列表段编辑（段 {k}）：增量 != 全量"),
            );
            // 还原回基线文本：应回到基线（复用链反转方向后再用的检验）
            let back = it.edit(k, &seg).expect("还原段");
            assert_doc_eq(&back, &full_path(&doc), format!("还原后应回到基线（段 {k}）"));
            checked += 1;
        }
        assert!(checked >= 3, "应至少覆盖 3 个公式/列表段，实际 {checked}");
    }

    #[test]
    fn consecutive_edits_match_full() {
        let doc = build_doc(26);
        let (mut it, _) = pipeline_compile(&doc);
        let segments = it.segments().to_vec();
        // 累积编辑的参考文档 = 原文按**原始段界**替换各被编辑段后的拼接（管线段
        // 列表保持固定切分、只换段文本，故逐段拼接即参考文档）。不能对逐次编辑的
        // `current` 串按原始段 span 顺序替换——前一次替换改变段长后，后续段的字节
        // 偏移已漂移，顺序替换会写到错位处、污染参考。
        let mut edits: Vec<(usize, String)> = Vec::new();
        for k in [4usize, 5, 6, 11, 3, 20] {
            if k >= segments.len() {
                continue;
            }
            let new_text = format!("{}\n", body(k + 100, true));
            let out = it.edit(k, &new_text).expect("连续编辑");
            edits.push((k, new_text));
            // 从原文重建：被编辑段 span 互不相交，按段下标**降序**替换即可（先换
            // 靠后的段不动靠前段的偏移）。注意不能 `iter().rev()`——那是反转编辑
            // 顺序（编辑可非单调，如 4,5,6,11,3），须按段下标排序降序。
            let mut current = doc.clone();
            let mut ordered: Vec<(usize, &String)> = edits.iter().map(|(k, t)| (*k, t)).collect();
            ordered.sort_by_key(|a| std::cmp::Reverse(a.0));
            for (kk, tt) in ordered {
                current = replace_segment(&current, &segments, kk, tt);
            }
            assert_doc_eq(
                &out,
                &full_path(&current),
                format!("连续编辑第 {k} 段后：增量 != 全量"),
            );
        }
    }

    #[test]
    fn repeated_same_edit_is_stable() {
        let doc = build_doc(18);
        let (mut it, dvi0) = pipeline_compile(&doc);
        let segments = it.segments().to_vec();
        let k = 3;
        let original = segments[k].clone();
        let new_text = format!("{}\n", body(k, true));
        let doc_edited = replace_segment(&doc, &segments, k, &new_text);
        let dvi_edited_full = full_path(&doc_edited);
        for round in 0..3 {
            let out = it.edit(k, &new_text).expect("重复编辑");
            assert_doc_eq(&out, &dvi_edited_full, format!("第 {round} 轮重复编辑偏离全量"));
        }
        // 还原回原文：应回到基线
        let back = it.edit(k, &original).expect("还原段");
        assert_doc_eq(&back, &dvi0, "还原后应回到基线".to_string());
    }

    #[test]
    fn errors_do_not_panic_and_pipeline_stays_usable() {
        let doc = build_doc(12);
        let (mut it, _) = pipeline_compile(&doc);
        let segments = it.segments().to_vec();
        // 越界 / 未编译：报错不 panic
        assert!(it.edit(999, "x").is_err(), "越界编辑应报错");
        let mut fresh = IncrementalTypesetter::with_tfm_paginated();
        assert!(fresh.edit(0, "x").is_err(), "未编译编辑应报错");
        // 错误段（双重上标 $x_i^2^3$ 的数学错误）→ 报错，管线随后仍可继续编辑。
        // 注：\undefinedcs 在本引擎是软错误（写转录、当 \relax 继续，不终止），
        // \font 指向不存在的字体也是软错误（引擎延迟加载/加载失败不终止）——
        // 不能作为"执行报错"样本；让 run() 返回 Err 的是扫描级错误（如双重上标）。
        let broken = "This paragraph has $x_i^2^3$ inside.\n".to_string();
        if it.edit(5, &broken).is_err() {
            let good = format!("{}\n", body(5, true));
            let out = it.edit(5, &good).expect("错误恢复后可继续编辑");
            let doc_edited = replace_segment(&doc, &segments, 5, &good);
            assert_doc_eq(&out, &full_path(&doc_edited), "错误恢复后：增量 != 全量".to_string());
        } else {
            panic!("双重上标应使该段执行报错");
        }
    }

    /// M5 阶段三基准（#[ignore]：release 下运行）——
    /// `cargo test --release -p ntex-layout --lib bench_edit_paragraph_vs_full -- --ignored --nocapture`
    ///
    /// 多页真实排版文档（正文 + 行内/显示公式 + 列表），改中段 1 段：
    /// 增量 vs 全量耗时与加速比；复用段数与剩余开销一并打印。
    #[test]
    #[ignore]
    fn bench_edit_paragraph_vs_full() {
        let n_para = 120;
        let doc = build_doc(n_para);
        let mid = n_para / 2;
        // 编辑后参考文档（与增量路径同一替换口径）
        let (probe, _) = pipeline_compile(&doc);
        let segments = probe.segments().to_vec();
        let new_text = format!("{}\n", body(mid, true));
        let doc_edited = replace_segment(&doc, &segments, mid, &new_text);

        // 基线：编辑后文档全新排版（现有全量路径）
        let t = Instant::now();
        let (pages_full, fonts_full) = {
            let mut ts = Typesetter::with_tfm_paginated();
            ts.typeset_dvi(&doc_edited).expect("全量排版")
        };
        let full_elapsed = t.elapsed();
        let reference = (pages_full, fonts_full);

        // 增量：编译基线文档 → 编辑中段 → 增量重生
        let (mut it, _) = pipeline_compile(&doc);
        let seg_count = it.segments().len();
        let t = Instant::now();
        let out = it.edit(mid, &new_text).expect("增量编辑");
        let inc_elapsed = t.elapsed();
        assert_eq!(&out.pages, &reference.0, "基准口径：增量 != 全量（页面）");
        assert_eq!(&out.fonts, &reference.1, "基准口径：增量 != 全量（字体表）");

        let n_pages = out.pages.len();
        let ratio = full_elapsed.as_secs_f64() / inc_elapsed.as_secs_f64().max(1e-9);
        println!("── M5 阶段三基准（端到端增量 vs 全量，{seg_count} 段 / {n_pages} 页）──────");
        println!(
            "全量（编辑后文档全新排版）      : {:>9.2} ms",
            full_elapsed.as_secs_f64() * 1e3
        );
        println!(
            "增量（编辑中段 1 段）          : {:>9.2} ms  加速 {:>6.1}x  复用 {}/{}",
            inc_elapsed.as_secs_f64() * 1e3,
            ratio,
            it.stats().reused,
            seg_count
        );
        println!(
            "剩余开销：复用判定逐段全状态比较（值指纹 + eqtb 全槽）、每段一次边界快照克隆、\
             页面装配自编辑段起重跑；宏体编辑（状态偏差）其后各段整体重排。"
        );
        println!("──────────────────────────────────────────────────");
    }


    #[test]
    fn dbg_first_diff() {
        let doc = build_doc(6);
        let mut ts = Typesetter::with_tfm_paginated();
        let (fp, _) = ts.typeset_dvi(&doc).expect("全量");
        let mut it = IncrementalTypesetter::with_tfm_paginated();
        let out = it.compile(&doc).expect("管线");
        println!("pages: full={} pipe={}", fp.len(), out.pages.len());
        for (pi, (a, b)) in fp.iter().zip(out.pages.iter()).enumerate() {
            if a == b {
                continue;
            }
            println!("page {pi} differs: dims a=({} {} {}) b=({} {} {})",
                a.width, a.height, a.depth, b.width, b.height, b.depth);
            let ca = sig_list(&a.children, 3);
            let cb = sig_list(&b.children, 3);
            println!("  counts {} vs {}", ca.len(), cb.len());
            for i in 0..ca.len().max(cb.len()) {
                let x = ca.get(i).map(String::as_str).unwrap_or("<none>");
                let y = cb.get(i).map(String::as_str).unwrap_or("<none>");
                if x != y {
                    println!("  first diff at child {i}:\n    full={x}\n    pipe={y}");
                    return;
                }
            }
            return;
        }
    }

    fn sig_list(list: &[crate::node::Node], depth: usize) -> Vec<String> {
        list.iter().enumerate().map(|(i, n)| sig_node(i, n, depth)).collect()
    }

    fn sig_node(i: usize, n: &crate::node::Node, depth: usize) -> String {
        use crate::node::Node::*;
        let head = match n {
            Box(b) => format!("{i}:Box({},{} {},{})", b.width, b.height, b.depth, b.shift),
            Rule { width, height, depth } => format!("{i}:Rule({width},{height},{depth})"),
            Char { charcode, width, .. } => format!("{i}:Char({} {})", charcode, width),
            Ligature { charcode, .. } => format!("{i}:Lig({charcode})"),
            Glue { width, stretch, shrink, stretch_order, .. } => {
                format!("{i}:Glue({width}+{stretch}/{shrink}@{stretch_order})")
            }
            Kern { width } => format!("{i}:Kern({width})"),
            Penalty { penalty } => format!("{i}:Pen({penalty})"),
            Mark { class, text } => format!("{i}:Mark({class:?},{text})"),
            other => format!("{i}:{other:?}"),
        };
        if let Box(b) = n {
            if depth > 0 && !b.children.is_empty() {
                let kids = sig_list(&b.children, depth - 1).join(" | ");
                return format!("{head} [{kids}]");
            }
        }
        head
    }
}
