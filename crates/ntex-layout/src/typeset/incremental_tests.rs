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

    /// 按段下标**降序**在原文上替换多段（先换靠后的段不动靠前段的偏移；管线
    /// 段列表保持固定切分，故以原文段界为基准的降序替换即参考文档）。
    fn replace_segments(doc: &str, segments: &[String], edits: &[(usize, String)]) -> String {
        let mut ordered: Vec<(usize, &String)> = edits.iter().map(|(k, t)| (*k, t)).collect();
        ordered.sort_by_key(|a| std::cmp::Reverse(a.0));
        let mut current = doc.to_string();
        for (k, t) in ordered {
            current = replace_segment(&current, segments, k, t);
        }
        current
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

    /// M5 阶段五基准（#[ignore]：release 下运行）——
    /// `cargo test --release -p ntex-layout --lib bench_edit_paragraph_vs_full -- --ignored --nocapture`
    ///
    /// 多页真实排版文档，两类编辑各测一轮（增量 vs 全量耗时与加速比、复用段数）：
    /// - 场景 A（改正文 1 段）：状态中性路径，阶段三已覆盖，作对照；
    /// - 场景 B（改宏体段）：阶段三在状态偏差下其后各段**整体重排**（复用 0、
    ///   耗时 ≈ 全量），阶段四依赖判定后仅引用该宏的段重排——提升即阶段四交付；
    ///   阶段五再把判定链开销压到墙钟转正（增量 < 全量）。
    ///
    /// 实测（release，120 段文档 / 2GB VM，2026-09-03）：
    /// - 阶段四：A 全量 56 ms / 增量 201 ms（0.3x，复用 107/169）；B 全量 33 ms /
    ///   增量 164 ms（0.2x，复用 60/121）——判定链每段整份状态拷贝支配。
    /// - 阶段五：A 全量 58 ms / **增量 13.6 ms（4.3x）**，复用 111/169（执行 2）；
    ///   B 全量 35 ms / **增量 30.3 ms（1.1x）**，复用 60/121（执行 61）。
    ///
    /// 阶段五的削减点：盒子寄存器文件 `Rc` 共享 + 写时复制（32768 槽不再随
    /// 边界克隆/副作用对齐/闸比较每段复制）、链偏差跨段携带（逐段全量扫描 →
    /// 执行段后一次）、段边界两档化（每段只留副作用字段快照，回滚点按 1/8 密度）。
    /// B 的剩余大头是执行段本身的排版 + 两次 expand 检查点捕获（见模块头
    /// "仍存边界"）。
    #[test]
    #[ignore]
    fn bench_edit_paragraph_vs_full() {
        let n_para = 120;

        // ---- 场景 A：改正文 1 段（状态中性） --------------------------------
        {
            let doc = build_doc(n_para);
            let mid = n_para / 2;
            let (probe, _) = pipeline_compile(&doc);
            let segments = probe.segments().to_vec();
            let new_text = format!("{}\n", body(mid, true));
            let doc_edited = replace_segment(&doc, &segments, mid, &new_text);
            // 基线：编辑后文档全新排版（现有全量路径）
            let t = Instant::now();
            let reference = {
                let mut ts = Typesetter::with_tfm_paginated();
                let (pages, fonts) = ts.typeset_dvi(&doc_edited).expect("全量排版");
                (pages, fonts)
            };
            let full = t.elapsed();
            // 增量：编译基线文档 → 编辑中段 → 增量重生
            let (mut it, _) = pipeline_compile(&doc);
            let seg_count = it.segments().len();
            let t = Instant::now();
            let out = it.edit(mid, &new_text).expect("增量编辑");
            let inc = t.elapsed();
            assert_eq!(&out.pages, &reference.0, "基准口径 A：增量 != 全量（页面）");
            assert_eq!(&out.fonts, &reference.1, "基准口径 A：增量 != 全量（字体表）");
            let n_pages = out.pages.len();
            let ratio = full.as_secs_f64() / inc.as_secs_f64().max(1e-9);
            println!("── 场景 A 改正文 1 段（{seg_count} 段 / {n_pages} 页）───────────────");
            println!("全量：{:>9.2} ms", full.as_secs_f64() * 1e3);
            println!(
                "增量：{:>9.2} ms  加速 {:>6.1}x  复用 {}/{}（执行 {}）",
                inc.as_secs_f64() * 1e3,
                ratio,
                it.stats().reused,
                seg_count,
                it.stats().executed
            );
        }

        // ---- 场景 B：改宏体段（依赖判定：仅引用段重排） ----------------------
        {
            let macro_doc = build_macro_doc(n_para);
            let (probe, _) = pipeline_compile(&macro_doc);
            let segments = probe.segments().to_vec();
            let new_preamble = segments[0].replace(GREET_VARIANTS[0], GREET_VARIANTS[1]);
            assert_ne!(new_preamble, segments[0], "导言应含 \\greet 宏体待改");
            let doc_edited = replace_segment(&macro_doc, &segments, 0, &new_preamble);
            let t = Instant::now();
            let reference = {
                let mut ts = Typesetter::with_tfm_paginated();
                let (pages, fonts) = ts.typeset_dvi(&doc_edited).expect("全量排版");
                (pages, fonts)
            };
            let full = t.elapsed();
            let (mut it, _) = pipeline_compile(&macro_doc);
            let seg_count = it.segments().len();
            let t = Instant::now();
            let out = it.edit(0, &new_preamble).expect("增量编辑宏体段");
            let inc = t.elapsed();
            assert_eq!(&out.pages, &reference.0, "基准口径 B：增量 != 全量（页面）");
            assert_eq!(&out.fonts, &reference.1, "基准口径 B：增量 != 全量（字体表）");
            let n_pages = out.pages.len();
            let ratio = full.as_secs_f64() / inc.as_secs_f64().max(1e-9);
            let rejected = it.last_rejects().iter().filter(|r| r.is_some()).count();
            println!("── 场景 B 改宏体段（{seg_count} 段 / {n_pages} 页）─────────────────");
            println!("全量：{:>9.2} ms", full.as_secs_f64() * 1e3);
            println!(
                "增量：{:>9.2} ms  加速 {:>6.1}x  复用 {}/{}（执行 {}，其中依赖失效重排 {}）",
                inc.as_secs_f64() * 1e3,
                ratio,
                it.stats().reused,
                seg_count,
                it.stats().executed,
                rejected
            );
            println!(
                "阶段三对照：改宏体 = 状态偏差 → 其后各段整体重排（复用 0 / 执行 {}、耗时 ≈ 全量 \
                 {:.2} ms）；阶段四依赖判定后仅读闭包触及被改宏槽的段重排（{} 段）。",
                seg_count - 1,
                full.as_secs_f64() * 1e3,
                rejected
            );
            println!(
                "剩余开销（阶段五未动，见模块头\"仍存边界\"）：执行段的排版本身 + 每段两次 \
                 expand 检查点捕获与词法依赖提取、注入节点的页面装配重跑。"
            );
        }
        println!("──────────────────────────────────────────────────");
    }


    // ---------- M5 阶段四：排版层依赖判定（宏体编辑省重排） ----------

    /// `\greet` 宏体的两个版本（等长：改宏体尽量不动段内折行形状，把重排归因
    /// 收敛到依赖判定本身）。
    const GREET_VARIANTS: [&str; 2] = [
        "Hello world from the greet macro.",
        "Greetings from the revised macro.",
    ];
    /// `\outer` 宏体的两个版本（宏引宏：`\outer` 体内引用 `\greet`）。
    const OUTER_VARIANTS: [&str; 2] = ["outer wraps it", "outer winds it"];

    /// 宏体编辑场景文档：导言（宏定义段）+ 引用段（`\greet` / `\outer`）+
    /// 纯正文段，多页真实排版。改宏体 = eqtb 宏槽偏差（阶段三收益归零的场景），
    /// 是依赖判定的目标场景。
    fn build_macro_doc(n_para: usize) -> String {
        let mut s = String::new();
        s.push_str("\\hsize 300pt\n\\vsize 320pt\n\\parindent 20pt\n");
        s.push_str(&font_preamble());
        s.push_str(&format!("\\def\\greet{{{}}}\n", GREET_VARIANTS[0]));
        s.push_str(&format!(
            "\\def\\outer{{\\greet\\ {}\\ \\greet}}\n",
            OUTER_VARIANTS[0]
        ));
        s.push('\n');
        for i in 0..n_para {
            match i % 4 {
                0 => s.push_str("\\greet\\ This paragraph uses the greet macro.\n\n"),
                1 => s.push_str("\\outer\\ This paragraph uses the outer macro.\n\n"),
                _ => {
                    s.push_str(&body(i, false));
                    s.push_str("\n\n");
                }
            }
        }
        s
    }

    /// 改宏体段（状态偏差）：引用该宏的段**必**被判依赖失效重排；未引用的纯
    /// 正文段复用缓存节点流；三重口径逐位一致。
    #[test]
    fn edit_macro_body_retypesets_only_referencing_segments() {
        let doc = build_macro_doc(24);
        let (mut it, dvi0) = pipeline_compile(&doc);
        assert_eq!(dvi0, full_path(&doc), "口径 3：管线全量 != 引擎全量");
        let segments = it.segments().to_vec();
        let pre = segments[0].clone();
        let new_preamble = pre.replace(GREET_VARIANTS[0], GREET_VARIANTS[1]);
        assert_ne!(new_preamble, pre, "导言应含 \\greet 宏体待改");
        let out = it.edit(0, &new_preamble).expect("编辑宏体段");
        let doc_edited = replace_segment(&doc, &segments, 0, &new_preamble);
        assert_doc_eq(
            &out,
            &full_path(&doc_edited),
            "口径 1：增量 != 全量（改 \\greet 宏体）".to_string(),
        );
        let (_, dvi_full) = pipeline_compile(&doc_edited);
        assert_doc_eq(
            &out,
            &dvi_full,
            "口径 2：增量 != 管线全量（改 \\greet 宏体）".to_string(),
        );

        // 双口径：引用段依赖失效（读闭包触及被改宏槽，判定必拒）；未引用正文段
        // 至少一个复用（stats + last_rejects 两口径互证）。
        let rejects = it.last_rejects();
        let mut cited = 0;
        let mut body_reused = 0;
        for (k, seg) in segments.iter().enumerate() {
            if seg.contains("\\def") {
                continue;
            }
            if seg.contains("\\greet") {
                cited += 1;
                assert!(
                    rejects[k].is_some(),
                    "引用 \\greet 的段 {k} 应被判重排（实际 {:?}）",
                    rejects[k]
                );
            } else if !seg.contains("\\outer") && rejects[k].is_none() {
                body_reused += 1;
            }
        }
        assert!(cited >= 2, "引用 \\greet 的段样本不足（{cited}）");
        assert!(body_reused > 0, "未引用正文段应复用缓存（reused={}）", it.stats().reused);
    }

    /// 嵌套宏（宏引宏）依赖闭包：改 `\outer` 宏体 → 闭包含 `\outer` 的段重排；
    /// 只引用 `\greet` 的段**不得**因 `\outer` 变化被判依赖失效（允许因排版
    /// 上下文漂移自愈重排——拒绝原因只能是上下文类，不许误报依赖）。
    #[test]
    fn edit_nested_macro_invalidates_only_closure_dependents() {
        let doc = build_macro_doc(24);
        let (mut it, _) = pipeline_compile(&doc);
        let segments = it.segments().to_vec();
        let pre = segments[0].clone();
        let new_preamble = pre.replace(OUTER_VARIANTS[0], OUTER_VARIANTS[1]);
        assert_ne!(new_preamble, pre, "导言应含 \\outer 宏体待改");
        let out = it.edit(0, &new_preamble).expect("编辑嵌套宏体段");
        let doc_edited = replace_segment(&doc, &segments, 0, &new_preamble);
        assert_doc_eq(
            &out,
            &full_path(&doc_edited),
            "嵌套宏体编辑：增量 != 全量".to_string(),
        );
        let rejects = it.last_rejects();
        let mut outer_users = 0;
        let mut greet_only = 0;
        for (k, seg) in segments.iter().enumerate() {
            if seg.contains("\\def") {
                continue;
            }
            if seg.contains("\\outer") {
                outer_users += 1;
                assert!(
                    rejects[k].is_some(),
                    "引用 \\outer 的段 {k} 应被判重排（实际 {:?}）",
                    rejects[k]
                );
            } else if seg.contains("\\greet") {
                greet_only += 1;
                if let Some(r) = rejects[k] {
                    assert!(
                        !r.contains("读依赖闭包"),
                        "只引用 \\greet 的段 {k} 不得因 \\outer 变化被判依赖失效（{r}）"
                    );
                }
            }
        }
        assert!(outer_users >= 2 && greet_only >= 2, "样本不足（{outer_users}/{greet_only}）");
    }

    /// 宏体编辑 + 连续编辑（含编辑"被复用过的正文段"）：偏差路径复用后段边界
    /// 已刷成活状态，后续 `edit` 的回滚点必须是新状态——否则回滚丢掉宏体新值、
    /// 文档退回旧宏输出（阶段四回归锁定）。
    #[test]
    fn macro_edit_then_edit_reused_segment_matches_full() {
        let doc = build_macro_doc(24);
        let (mut it, dvi0) = pipeline_compile(&doc);
        assert_eq!(dvi0, full_path(&doc), "口径 3：管线全量 != 引擎全量");
        let segments = it.segments().to_vec();
        let preamble = segments[0].clone();
        let greet_new = preamble.replace(GREET_VARIANTS[0], GREET_VARIANTS[1]);
        // ① 改 \greet 宏体（偏差路径；其后正文段走依赖判定复用）
        let out1 = it.edit(0, &greet_new).expect("编辑宏体段");
        let doc1 = replace_segment(&doc, &segments, 0, &greet_new);
        assert_doc_eq(&out1, &full_path(&doc1), "① 改宏体：增量 != 全量".to_string());
        // ② 编辑一个被复用过的纯正文段（回滚点 = 偏差路径刷新过的边界）
        let k = segments
            .iter()
            .position(|s| {
                !s.contains("\\def") && !s.contains("\\greet") && !s.contains("\\outer")
            })
            .expect("存在纯正文段");
        let new_text = format!("{}\n", body(k, true));
        let out2 = it.edit(k, &new_text).expect("编辑被复用过的正文段");
        let doc2 =
            replace_segments(&doc, &segments, &[(0, greet_new.clone()), (k, new_text.clone())]);
        assert_doc_eq(
            &out2,
            &full_path(&doc2),
            "② 再编辑正文段：增量 != 全量（宏体新值须保留）".to_string(),
        );
        // ③ 还原宏体：应回到"基线 + ② 的正文编辑"
        let out3 = it.edit(0, &preamble).expect("还原宏体段");
        let doc3 = replace_segments(&doc, &segments, &[(0, preamble.clone()), (k, new_text)]);
        assert_doc_eq(&out3, &full_path(&doc3), "③ 还原宏体：增量 != 全量".to_string());
    }

    /// 连续宏体编辑：改 `\greet` → 改 `\outer` → 还原，每步与全量逐位一致
    /// （偏差通道与同步通道在同一管线上交替使用）。
    #[test]
    fn consecutive_macro_edits_match_full() {
        let doc = build_macro_doc(20);
        let (mut it, _) = pipeline_compile(&doc);
        let segments = it.segments().to_vec();
        let preamble = segments[0].clone();
        let edits: Vec<(usize, String)> = vec![
            (
                0,
                preamble.replace(GREET_VARIANTS[0], GREET_VARIANTS[1]),
            ),
            (
                0,
                preamble.replace(OUTER_VARIANTS[0], OUTER_VARIANTS[1]),
            ),
            (
                0,
                preamble
                    .replace(GREET_VARIANTS[0], GREET_VARIANTS[1])
                    .replace(OUTER_VARIANTS[0], OUTER_VARIANTS[1]),
            ),
        ];
        let mut applied: Vec<(usize, String)> = Vec::new();
        for (round, (k, text)) in edits.into_iter().enumerate() {
            let out = it.edit(k, &text).expect("连续宏体编辑");
            applied.push((k, text));
            let current = replace_segments(&doc, &segments, &applied);
            assert_doc_eq(
                &out,
                &full_path(&current),
                format!("连续宏体编辑第 {round} 步：增量 != 全量"),
            );
        }
        // 还原全部 → 基线
        let out = it.edit(0, &preamble).expect("还原宏体段");
        assert_doc_eq(&out, &full_path(&doc), "还原后应回到基线".to_string());
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
