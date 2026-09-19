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
        // 并行测试竞态防护（2026-09-12）：set_var 在多线程下是数据竞争
        //（Rust 2024 起标 unsafe）——13 例增量测试并行偶发 0 页即此因
        //（NTEX_TFM_DIR 被并发写坏 → 字体度量退化 → 分页数翻转）。
        // OnceLock 只初始化一次；首次成功后再无 set_var 调用。
        static TFM_DIR: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        if TFM_DIR.get().is_some() {
            return;
        }
        if std::env::var("NTEX_TFM_DIR")
            .ok()
            .is_some_and(|d| std::path::Path::new(&d).join("cmr10.tfm").exists())
        {
            let _ = TFM_DIR.set(());
            return;
        }
        let home = std::env::var("HOME").unwrap_or_default();
        let p = std::path::PathBuf::from(home).join(".ntex-fonts");
        if p.join("cmr10.tfm").exists() {
            std::env::set_var("NTEX_TFM_DIR", &p);
            let _ = TFM_DIR.set(());
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
    // ---------- R1b 前置钉子：SideEffects 镜像 roundtrip ----------
    //
    // side_effects() 的**字段完整性**由编译器保证（SideEffects 结构体字面量
    // 缺字段即编译错）；restore_side_effects 的**遗漏 / 串线**（写了错的来源
    // 字段、或干脆漏写一个字段）编译器抓不到，只能运行时断言。本测试把全部
    // 镜像字段涂成 A 值 → 捕获 → 涂成 B 值 → 恢复，逐字段断言回到 A 值。
    // 铁律：SideEffects 字段增减时，A/B 两组涂值与断言同步增减。

    use crate::node::{BoxNode, FontId, LeadersKind, Node};
    use crate::hyphen::PatternTrie;
    use ntex_core::Glue;

    fn mirror_builder() -> super::NodeBuilder {
        super::NodeBuilder::new(super::Fonts::Fn {
            metrics: |_, _| (0, 0, 0),
            space: |_| Glue::ZERO,
        })
    }

    /// 镜像字段涂成 A 值（全部非默认、彼此可辨）。
    fn paint_a(b: &mut super::NodeBuilder) {
        b.box_state.pending_box = Some(super::PendingBox::VTop);
        b.box_state.pending_kind = Some(super::GroupKind::Output);
        b.box_state.pending_shift = Some(11);
        b.box_state.pending_hshift = Some(33);
        b.box_state.pending_leaders = Some(LeadersKind::Xleaders);
        b.box_state.leaders_box = Some((LeadersKind::Leaders, Node::Kern { width: 7 }));
        b.params = super::Params { parindent: 101, ..Default::default() };
        b.penalty_arrays = [vec![1], vec![2], vec![3], vec![4]];
        b.param_stack = vec![
            super::Params { parindent: 201, ..Default::default() },
            super::Params { parindent: 202, ..Default::default() },
        ];
        b.sfcodes = [0; 256];
        b.sfcodes[b'a' as usize] = 3000;
        b.sfcodes[b'b' as usize] = 3001;
        b.font_stack = vec![FontId(1)];
        b.space_factor = 1001;
        b.noindent_next = true;
        b.align_stack = vec![(super::AlignDir::Halign, super::AlignCtx {
            tabskips: vec![Glue::ZERO],
            stream: vec![],
            cur_cells: vec![],
            cur_col: 1,
            to: Some(5),
            spread: None,
        })];
        b.last_par_line = 55;
        b.font_cs_names = vec![None, Some("tenrm".into())];
        b.current_font = FontId(3);
        b.page_state.shipout_next = true;
        b.page_state.ship_seq = 5;
        b.page_state.page_counts = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        b.box_state.boxes = std::rc::Rc::new(vec![Some(BoxNode::new_hbox(vec![])), None, None, None]);
        b.box_state.box_saves = vec![(0, 1, None)];
        b.page_state.output_defined = true;
        b.page_state.pending_pages = [BoxNode::new_vbox(vec![])].into();
        b.page_state.write_flush_pending = true;
        b.page_state.page_shipped = true;
        b.math_state.math_style = super::MathStyle::Script;
        b.math_state.pending_script = Some(true);
        b.math_state.sqrt_pending = true;
        b.math_state.radical_pending = Some(7);
        b.math_state.class_pending = Some(super::MathClass::Bin);
        b.math_state.accent_pending = true;
        b.math_state.underline_pending = true;
        b.math_state.overline_pending = true;
        b.math_state.nonscript_pending = true;
        b.math_state.math_fonts = vec![[Some(FontId(1)); 3]; 16];
        b.patterns = PatternTrie::parse(b"ab1c");
        b.hyph_exceptions = vec![(b"abc".to_vec(), vec![0, 2])];
        b.box_state.setbox_target = Some(4);
        b.box_state.setbox_global = true;
        b.box_state.pending_box_spec = Some((Some(10), None));
        b.math_state.predisplay_size = 77;
        b.math_state.after_display = true;
        b.math_state.muskip_params = [
            Glue { width: 196608, ..Glue::ZERO },
            Glue { width: 262144, ..Glue::ZERO },
            Glue { width: 327680, ..Glue::ZERO },
        ];
        b.math_state.muskip_is_mu = [true, false, true];
        b.page_state.marks_top = [(1, "top-a".to_string())].into_iter().collect();
        b.page_state.marks_first = [(1, "first-a".to_string())].into_iter().collect();
        b.page_state.marks_bot = [(1, "bot-a".to_string())].into_iter().collect();
        b.page_state.marks_split_top = [(2, "stop-a".to_string())].into_iter().collect();
        b.page_state.marks_split_first = [(2, "sfirst-a".to_string())].into_iter().collect();
        b.page_state.marks_split_bot = [(2, "sbot-a".to_string())].into_iter().collect();
        b.box_state.lastbox_hold = Some(BoxNode::new_hbox(vec![]));
    }

    /// 镜像字段涂成 B 值（与 A 值逐一不同）。
    fn paint_b(b: &mut super::NodeBuilder) {
        b.box_state.pending_box = Some(super::PendingBox::HBox);
        b.box_state.pending_kind = Some(super::GroupKind::Align);
        b.box_state.pending_shift = Some(-22);
        b.box_state.pending_hshift = Some(-44);
        b.box_state.pending_leaders = Some(LeadersKind::Cleaders);
        b.box_state.leaders_box = Some((LeadersKind::Cleaders, Node::Kern { width: 9 }));
        b.params = super::Params { parindent: 102, ..Default::default() };
        b.penalty_arrays = [vec![11], vec![12], vec![13], vec![14]];
        b.param_stack = vec![super::Params { parindent: 203, ..Default::default() }];
        b.sfcodes = [0; 256];
        b.sfcodes[b'a' as usize] = 4000;
        b.sfcodes[b'b' as usize] = 4001;
        b.font_stack = vec![FontId(1), FontId(2)];
        b.space_factor = 2002;
        b.noindent_next = false;
        b.align_stack = vec![(super::AlignDir::Valign, super::AlignCtx {
            tabskips: vec![],
            stream: vec![],
            cur_cells: vec![],
            cur_col: 2,
            to: None,
            spread: None,
        })];
        b.last_par_line = 66;
        b.font_cs_names = vec![Some("trip".into())];
        b.current_font = FontId(4);
        b.page_state.shipout_next = false;
        b.page_state.ship_seq = 6;
        b.page_state.page_counts = [10, 9, 8, 7, 6, 5, 4, 3, 2, 1];
        b.box_state.boxes = std::rc::Rc::new(vec![None; 6]);
        b.box_state.box_saves = vec![(0, 1, None), (1, 2, None)];
        b.page_state.output_defined = false;
        b.page_state.pending_pages = [BoxNode::new_vbox(vec![]), BoxNode::new_vbox(vec![])].into();
        b.page_state.write_flush_pending = false;
        b.page_state.page_shipped = false;
        b.math_state.math_style = super::MathStyle::ScriptScript;
        b.math_state.pending_script = Some(false);
        b.math_state.sqrt_pending = false;
        b.math_state.radical_pending = Some(8);
        b.math_state.class_pending = Some(super::MathClass::Rel);
        b.math_state.accent_pending = false;
        b.math_state.underline_pending = false;
        b.math_state.overline_pending = false;
        b.math_state.nonscript_pending = false;
        b.math_state.math_fonts = vec![[Some(FontId(2)); 3]; 16];
        b.patterns = PatternTrie::parse(b"xy2z");
        b.hyph_exceptions = vec![(b"xy".to_vec(), vec![1])];
        b.box_state.setbox_target = Some(5);
        b.box_state.setbox_global = false;
        b.box_state.pending_box_spec = Some((None, Some(20)));
        b.math_state.predisplay_size = 88;
        b.math_state.after_display = false;
        b.math_state.muskip_params = [
            Glue { width: 1, ..Glue::ZERO },
            Glue { width: 2, ..Glue::ZERO },
            Glue { width: 3, ..Glue::ZERO },
        ];
        b.math_state.muskip_is_mu = [false, true, false];
        b.page_state.marks_top = [(1, "top-b".to_string())].into_iter().collect();
        b.page_state.marks_first = [(1, "first-b".to_string())].into_iter().collect();
        b.page_state.marks_bot = [(1, "bot-b".to_string())].into_iter().collect();
        b.page_state.marks_split_top = [(2, "stop-b".to_string())].into_iter().collect();
        b.page_state.marks_split_first = [(2, "sfirst-b".to_string())].into_iter().collect();
        b.page_state.marks_split_bot = [(2, "sbot-b".to_string())].into_iter().collect();
        b.box_state.lastbox_hold = Some(BoxNode::new_vbox(vec![]));
    }

    /// 捕获 → 涂异 → 恢复：52 个镜像字段逐位复原（缺一即败）。
    #[test]
    fn side_effects_roundtrip_restores_every_field() {
        let mut b = mirror_builder();
        paint_a(&mut b);
        let se = b.side_effects();
        paint_b(&mut b);
        b.restore_side_effects(&se);

        // 盒封装待定
        assert_eq!(b.box_state.pending_box, Some(super::PendingBox::VTop));
        assert_eq!(b.box_state.pending_kind, Some(super::GroupKind::Output));
        assert_eq!(b.box_state.pending_shift, Some(11));
        assert_eq!(b.box_state.pending_hshift, Some(33));
        assert_eq!(b.box_state.pending_leaders, Some(LeadersKind::Xleaders));
        assert_eq!(
            b.box_state.leaders_box,
            Some((LeadersKind::Leaders, Node::Kern { width: 7 }))
        );
        // 参数 / 字体镜像
        assert_eq!(b.params.parindent, 101);
        assert_eq!(b.penalty_arrays, [vec![1], vec![2], vec![3], vec![4]]);
        assert_eq!(b.param_stack.len(), 2);
        assert_eq!(b.param_stack[1].parindent, 202);
        assert_eq!(b.sfcodes[b'a' as usize], 3000);
        assert_eq!(b.sfcodes[b'b' as usize], 3001);
        assert_eq!(b.font_stack, vec![FontId(1)]);
        assert_eq!(b.space_factor, 1001);
        assert!(b.noindent_next);
        assert_eq!(b.align_stack.len(), 1);
        assert_eq!(b.align_stack[0].0, super::AlignDir::Halign);
        assert_eq!(b.align_stack[0].1.cur_col, 1);
        assert_eq!(b.last_par_line, 55);
        assert_eq!(b.font_cs_names, vec![None, Some("tenrm".to_string())]);
        assert_eq!(b.current_font, FontId(3));
        // 页面 / shipout
        assert!(b.page_state.shipout_next);
        assert_eq!(b.page_state.ship_seq, 5);
        assert_eq!(b.page_state.page_counts, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(b.box_state.boxes.len(), 4);
        assert!(b.box_state.boxes[0].is_some());
        assert_eq!(b.box_state.box_saves.len(), 1);
        assert!(b.page_state.output_defined);
        assert_eq!(b.page_state.pending_pages.len(), 1);
        assert!(b.page_state.write_flush_pending);
        assert!(b.page_state.page_shipped);
        // 数学
        assert_eq!(b.math_state.math_style, super::MathStyle::Script);
        assert_eq!(b.math_state.pending_script, Some(true));
        assert!(b.math_state.sqrt_pending);
        assert_eq!(b.math_state.radical_pending, Some(7));
        assert_eq!(b.math_state.class_pending, Some(super::MathClass::Bin));
        assert!(b.math_state.accent_pending);
        assert!(b.math_state.underline_pending);
        assert!(b.math_state.overline_pending);
        assert!(b.math_state.nonscript_pending);
        assert_eq!(b.math_state.math_fonts[0], [Some(FontId(1)); 3]);
        assert!(!b.patterns.is_empty());
        assert_eq!(b.patterns.count, PatternTrie::parse(b"ab1c").count);
        assert_eq!(b.hyph_exceptions, vec![(b"abc".to_vec(), vec![0, 2])]);
        assert_eq!(b.box_state.setbox_target, Some(4));
        assert!(b.box_state.setbox_global);
        assert_eq!(b.box_state.pending_box_spec, Some((Some(10), None)));
        assert_eq!(b.math_state.predisplay_size, 77);
        assert!(b.math_state.after_display);
        assert_eq!(b.math_state.muskip_params[0].width, 196608);
        assert_eq!(b.math_state.muskip_params[2].width, 327680);
        assert_eq!(b.math_state.muskip_is_mu, [true, false, true]);
        // marks
        assert_eq!(b.page_state.marks_top.get(&1).map(String::as_str), Some("top-a"));
        assert_eq!(b.page_state.marks_first.get(&1).map(String::as_str), Some("first-a"));
        assert_eq!(b.page_state.marks_bot.get(&1).map(String::as_str), Some("bot-a"));
        assert_eq!(b.page_state.marks_split_top.get(&2).map(String::as_str), Some("stop-a"));
        assert_eq!(b.page_state.marks_split_first.get(&2).map(String::as_str), Some("sfirst-a"));
        assert_eq!(b.page_state.marks_split_bot.get(&2).map(String::as_str), Some("sbot-a"));
        assert!(b.box_state.lastbox_hold.is_some());
        assert_eq!(b.box_state.lastbox_hold.as_ref().map(|x| x.kind), b.box_state.boxes[0].as_ref().map(|x| x.kind));

        // 全结构一致（BoxFile 走指针相等快路径：restore 后共享同一 Rc）
        assert_eq!(b.side_effects(), se);
    }

    /// 长驻增量器连续 `compile` 两份作业：第二份不得因上一份 `\end` 残留的
    /// `ended` 标志而空转——`run` 主循环见 `ended` 立即停（tex.web final_cleanup），
    /// 第二份文档的段全部被跳过 → **恒 0 页**（2026-09-19 现场：ntex-studio 每次
    /// 编辑重编译都会走 `compile`，预览会突然空白）。
    ///
    /// 注意本文件其它用例的文档**不带 `\end`**（靠输入耗尽收尾），故此前未覆盖
    /// 这条路径；studio 的样张（`samples/demo.tex` 等）与真实 LaTeX 源码都以
    /// `\end` 结尾。
    #[test]
    fn recompile_on_same_engine_after_end_is_not_empty() {
        ensure_tfm_dir();
        let src = format!(
            "\\hsize 300pt\n\\vsize 200pt\n{}A paragraph long enough to occupy a line or two \
             of the page.\n\\par\n\\end\n",
            font_preamble()
        );
        let mut inc = IncrementalTypesetter::with_tfm_paginated();
        let first = inc.compile(&src).expect("首次编译");
        assert!(!first.pages.is_empty(), "首次编译应有页面");
        let second = inc.compile(&src).expect("二次编译（同一实例）");
        assert_eq!(
            second.pages.len(),
            first.pages.len(),
            "二次编译页数应与首次一致（`\\end` 残留会让主循环空转 → 0 页）"
        );
        // 三次也稳（`ended` 复位是幂等的，且段缓存/边界重建不影响结果）。
        let third = inc.compile(&src).expect("三次编译");
        assert_eq!(third.pages.len(), first.pages.len(), "三次编译页数应稳定");
    }

    /// C 档 LaTeX（2026-09-19，ntex-studio 对齐 Tauri）：`import_state`
    /// （发行 `assets/fmt/latex.fmt`）+ TeX 文件搜索链（发行 `tex-minimal` 树）
    /// 让增量器按 LaTeX 口径排版——**0 页曾是接通前的现场**（`\documentclass`
    /// 未定义、静默排空）。
    ///
    /// 资产缺失即跳过：本用例验证的是"增量器 × fmt 状态"的组合，不是资产分发
    /// 本身（分发由 ntex-tauri 的资产包与 `ntex-dvi --fmt` 覆盖）。
    #[test]
    fn latex_fmt_compile_produces_pages() {
        let Some((root, state)) = latex_assets() else {
            return;
        };
        let src = "\\documentclass{article}\n\\begin{document}\n\\section{Hello}\n\
                   A paragraph of body text.\n\\end{document}\n";
        let mut inc = latex_inc(&root, state);
        let out = inc.compile(src).expect("LaTeX 口径编译（fmt + TeX 树）");
        assert!(
            !out.pages.is_empty(),
            "LaTeX 文档应产出页面（0 页 = 实际按 plain 子集排了：\\documentclass 未定义）"
        );
        // 字体表应含 LaTeX 字体块（cmr10 + 12pt 族的 cmbx12/cmr17 等之一）。
        let names: Vec<&str> = out.fonts.iter().map(|f| f.name.as_str()).collect();
        assert!(
            names.iter().any(|n| n.starts_with("cmr") || n.starts_with("cmbx")),
            "LaTeX 文档应装载 CM 家族字体，实际：{names:?}"
        );
    }

    /// **已知缺陷（2026-09-19，未修）**：fmt 状态下的增量 `edit` 不可用。
    ///
    /// 现场：`import_state(latex.fmt)` 后 `compile` → `edit(k, ..)` 报
    /// `InvalidInput { message: "VFS 读取 失败：\0article.cls（定义 \@filef@und
    /// 的替换文本时）" }` —— 重放（回滚到段前检查点 + 重新执行）把 LaTeX 的
    /// `\@filef@und` 替换文本还原成**含 NUL 前缀**的形态，随后 `\input` 拿它当
    /// 文件名去读 VFS（首次全量路径同一字体/文件读的是裸名 `article.cls`，说明
    /// 差异在重放后的活状态而非资产/VFS）。首查方向：`\0` 是引擎内部"字符串
    /// token"编码，需查回滚（`StateSnapshot::restore_to`）后宏体 token 的展开
    /// 分支是否走了"原始 token 数组"路径而未剥离该前缀。
    ///
    /// 影响面：`ntex-studio` 因此在 LaTeX 口径**只走全量重编译**（与 Tauri 前端
    /// `compile_document` 同口径）；plain 口径的段级增量不受影响（既有逐位一致
    /// 用例覆盖）。修复后删掉本注解与 `#[ignore]`，用例即变回归锁。
    #[test]
    #[ignore = "已知缺陷：fmt 状态下增量 edit 重放 `\\@filef@und` 坏值（\\0article.cls），见函数文档"]
    fn latex_fmt_import_and_incremental_edit() {
        let Some((root, state)) = latex_assets() else {
            return;
        };
        let src = "\\documentclass{article}\n\\begin{document}\n\\section{Hello}\n\
                   First paragraph here.\n\nSecond paragraph here.\n\\end{document}\n";
        let mut inc = latex_inc(&root, state.clone());
        let out = inc.compile(src).expect("LaTeX 口径编译（fmt + TeX 树）");
        assert!(!out.pages.is_empty(), "LaTeX 文档应产出页面");

        // 改一段正文 → 增量编辑须与全新实例的全量编译节点级一致。
        let segs = inc.segments().to_vec();
        let k = segs
            .iter()
            .position(|s| s.contains("Second paragraph here."))
            .expect("段列表应含目标段");
        let new_text = segs[k].replace(
            "Second paragraph here.",
            "Second paragraph edited into a longer sentence.",
        );
        let edited = replace_segment(src, &segs, k, &new_text);
        let inv = inc.edit(k, &new_text).expect("增量编辑（fmt 状态）");

        let mut fresh = latex_inc(&root, state);
        let reference = fresh.compile(&edited).expect("参考全量编译");
        let reference: Doc = (reference.pages, reference.fonts);
        assert_doc_eq(&inv, &reference, "LaTeX fmt 增量 edit".to_owned());
    }

    /// **LaTeX `tabular`/`array` 端到端（2026-09-19 新增回归锁）**：
    /// LaTeX 的表格全部建立在 `\ialign\bgroup`（`\@preamble`）与
    /// `\endtabular` 的 `\crcr\egroup…` 之上，preamble 里的 `#` 又是
    /// `\let\@sharp##` 的 cs 形态，跨列单元走 `\omit\span\omit`
    /// （`\multispan`）。这三条都要求**命令层**判 `cur_cmd`（tex.web
    /// `scan_left_brace`/`align_peek`/`get_preamble_token`），此前只认字符
    /// 形态 → 任何 `tabular` 在首个 `\halign` 报 "Missing { inserted"、
    /// 整篇 0 页（`samples/latex-sample2e-slim.tex` 的根因）。
    ///
    /// 资产缺失即跳过（与 fmt 用例同口径：验证的是引擎能力，不是资产分发）。
    #[test]
    fn latex_tabular_and_multicolumn_render_pages() {
        let Some((root, state)) = latex_assets() else {
            return;
        };
        let src = "\\documentclass{article}\n\\begin{document}\n\
                   \\begin{tabular}{ll}\nA & B\\\\\n\\multicolumn{2}{l}{wide}\\\\\n\\end{tabular}\n\
                   \\end{document}\n";
        let mut inc = latex_inc(&root, state);
        let out = inc.compile(src).expect("LaTeX tabular 编译");
        assert!(
            !out.pages.is_empty(),
            "tabular + \\multicolumn 应产出页面（0 页 = \\halign\\bgroup / \\@sharp / \
             \\omit\\span\\omit 某条命令层判据仍缺）"
        );
        // 转录不得留错误（此前是 "Missing { inserted" / "ended by \\document"）。
        let log = inc.transcript().to_owned();
        for bad in ["! Missing {", "! Missing #", "ended by", "end occurred inside a group"] {
            assert!(
                !log.contains(bad),
                "LaTeX tabular 转录不应含 {bad:?}：\n{log}"
            );
        }
    }

    /// 载入发行 LaTeX 资产（`assets/fmt/latex.fmt` + `assets/tex-minimal/tex`）；
    /// 缺失即打印跳过提示并返回 `None`。
    fn latex_assets() -> Option<(std::path::PathBuf, ntex_core::expand::FmtState)> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let fmt_path = root.join("fmt/latex.fmt");
        if !fmt_path.is_file() || !root.join("tex-minimal/tex").is_dir() {
            eprintln!("跳过：发行资产不在 {}", root.display());
            return None;
        }
        let bytes = std::fs::read(&fmt_path).expect("读 latex.fmt");
        let state = ntex_format::load(&mut &bytes[..]).expect("解码 latex.fmt");
        Some((root, state))
    }

    /// LaTeX 口径的增量器：与 `ntex-studio::engine::Setup::typesetter(true, ..)`
    /// 同配置的最小版（UTF-8 直写 + fmt 导入 + 发行 TeX 树搜索链）。
    fn latex_inc(root: &std::path::Path, state: ntex_core::expand::FmtState) -> IncrementalTypesetter {
        let mut ts = IncrementalTypesetter::with_tfm_paginated();
        ts.set_utf8_input(true);
        ts.set_vfs(latex_vfs(root));
        ts.import_state(state);
        ts
    }

    /// 发行 TeX 树 → VFS（与 `ntex-studio::engine` 的搜索链同口径的最小版）。
    fn latex_vfs(root: &std::path::Path) -> Box<dyn ntex_io::Vfs> {
        let tex = root.join("tex-minimal/tex");
        let mut vfs = ntex_io::SearchPathVfs::new(Box::new(ntex_io::LocalVfs));
        vfs.push_path(tex.to_string_lossy().into_owned());
        vfs.push_path(tex.join("latex/base").to_string_lossy().into_owned());
        for sub in ["latex", "generic"] {
            let Ok(rd) = std::fs::read_dir(tex.join(sub)) else {
                continue;
            };
            for entry in rd.flatten() {
                if entry.path().is_dir() {
                    vfs.push_path(entry.path().to_string_lossy().into_owned());
                }
            }
        }
        Box::new(vfs)
    }
}
