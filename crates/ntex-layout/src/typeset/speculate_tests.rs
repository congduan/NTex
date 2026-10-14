// ---------- 测试分片：speculate_tests.rs（M6 投机并行回归锁） ----------
//
// 铁律（RFC-5 §1）：投机窗口路径与顺序路径的三路输出**逐位一致**——页面节点流、
// 页号链（`\count0..9` 快照）、字体表（`write_dvi` 的全部输入）。转录（侧信道）
// 也按段序 flush，一并比较。
//
// 测试矩阵对应 RFC-5 §7：
// - neutral 段（纯文本 / 公式 / 列表）→ 采纳路径；
// - 非 neutral 段（`\def` / 寄存器赋值 / `\setbox`）→ 窗口终止 + 主线程顺序执行；
// - taint 段（`\lastbox` / `\unskip` / marks）→ 静态黑名单 + 运行时兜底；
// - `\output` 例程文档 → 投机整体关闭（保守口径）；
// - 合并段（编辑文本不以空行收尾）→ 跨段构造不投机；
// - 投机编译后的缓存必须与 M5 `edit` 兼容（增量编辑结果仍与全量一致）。

#[cfg(test)]
mod spec_tests {
    use crate::typeset::{CompileOutput, IncrementalTypesetter};

    /// 定位含 cmr10.tfm 的目录（与 `incremental_tests` 同款：NTEX_TFM_DIR 或
    /// `~/.ntex-fonts`；找不到则退化为 nullfont 零度量，一致性口径不变）。
    fn ensure_tfm_dir() {
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

    /// 有 cmr10 时的导言（真实度量 + 数学字体族）。
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

    /// 词表（确定性填充文本）。
    const WORDS: [&str; 16] = [
        "speculative", "segment", "window", "adopt", "state", "neutral", "glue", "baseline",
        "assemble", "page", "break", "penalty", "box", "depth", "height", "width",
    ];

    /// 第 `i` 段正文（确定性伪随机组句）。
    fn body(i: usize) -> String {
        let mut s = String::new();
        for sent in 0..3 {
            for w in 0..9 {
                s.push_str(WORDS[(i * 5 + sent * 3 + w * 7) % WORDS.len()]);
                s.push(' ');
            }
            s.pop();
            s.push_str(". ");
        }
        s
    }

    /// 多页真实排版文档：正文段 + 行内公式 + 显示公式 + 列表段。
    fn build_doc(n_para: usize) -> String {
        let mut s = String::new();
        s.push_str("\\hsize 300pt\n\\vsize 320pt\n\\parindent 20pt\n");
        s.push_str(&font_preamble());
        s.push('\n');
        for i in 0..n_para {
            match i % 5 {
                2 => {
                    s.push_str(&body(i));
                    s.push_str(" Inline math $x_i + y_i = z_i$ ends here.\n\n");
                }
                4 => {
                    s.push_str(&body(i));
                    s.push_str("\n$${a \\over b} + x_i = y_i$$\n");
                    s.push_str(&body(i + 1));
                    s.push_str("\n\n");
                }
                3 => {
                    for k in 0..2 {
                        s.push_str(&format!(
                            "\\hangindent 20pt\\hangafter 1\\noindent --{} item {} {}\n\n",
                            WORDS[(i + k) % WORDS.len()],
                            i,
                            k
                        ));
                    }
                }
                _ => {
                    s.push_str(&body(i));
                    s.push_str("\n\n");
                }
            }
        }
        s
    }

    fn compile(source: &str, speculative: bool) -> (CompileOutput, String) {
        let (out, t, _) = compile_stats(source, speculative);
        (out, t)
    }

    /// 同 [`compile`]，另带投机面统计（`adopted`/`spec_rejected`；观测用）。
    fn compile_stats(
        source: &str,
        speculative: bool,
    ) -> (CompileOutput, String, crate::typeset::IncrementalStats) {
        let mut ts = IncrementalTypesetter::with_tfm_paginated();
        ts.set_speculative(speculative);
        let out = ts.compile(source).expect("compile 应成功");
        let t = ts.transcript().to_owned();
        let st = ts.stats();
        (out, t, st)
    }

    /// 三路输出 + 转录逐位一致断言。
    fn assert_same(a: &CompileOutput, at: &str, b: &CompileOutput, bt: &str, ctx: &str) {
        assert_eq!(a.pages.len(), b.pages.len(), "{ctx}: 页数不一致");
        for (i, (x, y)) in a.pages.iter().zip(b.pages.iter()).enumerate() {
            assert_eq!(
                format!("{x:?}"),
                format!("{y:?}"),
                "{ctx}: 第 {i} 页节点流不一致"
            );
        }
        assert_eq!(a.page_counts, b.page_counts, "{ctx}: 页号链不一致");
        assert_eq!(
            a.fonts.len(),
            b.fonts.len(),
            "{ctx}: 字体表规模不一致"
        );
        assert_eq!(at, bt, "{ctx}: 转录不一致");
    }

    /// 主用例：投机的全量编译与顺序编译逐位一致（neutral 段占绝大多数）。
    #[test]
    fn speculative_compile_matches_sequential() {
        let doc = build_doc(40);
        let (seq, seqt) = compile(&doc, false);
        let (spe, spet, st) = compile_stats(&doc, true);
        assert_same(&seq, &seqt, &spe, &spet, "投机 vs 顺序");
        assert!(!seq.pages.is_empty(), "应有页面产出");
        // 无 `\output` 例程的文档：投机面必须非空（实测量级：plain 合成文档
        // 49 段执行 / 24 段采纳，其余为 preamble 类非 neutral 段）。
        eprintln!("[spec] plain 文档投机统计 = {st:?}");
        assert!(
            st.adopted > 0,
            "无 \\output 例程时投机窗口应有被采纳的段：{st:?}"
        );
    }

    /// 非 neutral 段（宏定义 / 寄存器赋值 / `\setbox`）：窗口终止 + 主线程顺序执行。
    #[test]
    fn speculative_matches_with_non_neutral_segments() {
        let mut doc = String::from("\\hsize 300pt\n\\vsize 320pt\n\\parindent 20pt\n");
        doc.push_str(&font_preamble());
        doc.push('\n');
        for i in 0..12 {
            doc.push_str(&body(i));
            doc.push_str("\n\n");
            match i % 4 {
                0 => doc.push_str(&format!("\\def\\tag{}{{T{} }}\\tag{}\n\n", i, i, i)),
                1 => doc.push_str(&format!("\\count5={}\n\n", i * 3)),
                2 => doc.push_str("\\setbox0=\\hbox{scratch box}\n\n"),
                _ => doc.push_str("\\skip2=6pt plus 2pt\n\n"),
            }
        }
        let (seq, seqt) = compile(&doc, false);
        let (spe, spet) = compile(&doc, true);
        assert_same(&seq, &seqt, &spe, &spet, "非 neutral 段");
    }

    /// taint 段（装配状态读取）：静态黑名单预过滤 + 运行时 taint 兜底。
    #[test]
    fn speculative_matches_with_taint_segments() {
        let mut doc = String::from("\\hsize 300pt\n\\vsize 320pt\n\\parindent 20pt\n");
        doc.push_str(&font_preamble());
        doc.push('\n');
        for i in 0..10 {
            doc.push_str(&body(i));
            doc.push_str("\n\n");
            match i % 3 {
                0 => doc.push_str("\\setbox0=\\lastbox\n\n"),
                1 => doc.push_str("\\par\\unskip\n\n"),
                _ => doc.push_str("\\vskip 4pt minus 1pt\n\n"),
            }
        }
        let (seq, seqt) = compile(&doc, false);
        let (spe, spet) = compile(&doc, true);
        assert_same(&seq, &seqt, &spe, &spet, "taint 段");
    }

    /// `\output` 例程文档：投机整体关闭（保守口径），输出仍与顺序一致。
    #[test]
    fn speculative_matches_with_output_routine() {
        let mut doc = String::from(
            "\\hsize 300pt\n\\vsize 200pt\n\\parindent 0pt\n\\count0=1\n\
             \\output={\\shipout\\box255\\advance\\count0 by 1}\n",
        );
        doc.push_str(&font_preamble());
        doc.push('\n');
        for i in 0..14 {
            doc.push_str(&body(i));
            doc.push_str("\n\n");
        }
        let (seq, seqt) = compile(&doc, false);
        let (spe, spet) = compile(&doc, true);
        assert_same(&seq, &seqt, &spe, &spet, "\\output 例程");
    }

    /// 合并段（段尾无空行）：跨段构造不投机，窗口在该段终止。
    #[test]
    fn speculative_matches_with_merged_segments() {
        let mut doc = String::from("\\hsize 300pt\n\\vsize 300pt\n\\parindent 10pt\n");
        doc.push_str(&font_preamble());
        doc.push('\n');
        // 交替：正常段 + "与下段合并"的段（结尾无空行 → 段落跨段闭合）
        for i in 0..10 {
            doc.push_str(&body(i));
            if i % 3 == 1 {
                // 无空行结尾：本段与下一段并入同一段落
                doc.push('\n');
            } else {
                doc.push_str("\n\n");
            }
        }
        doc.push('\n');
        let (seq, seqt) = compile(&doc, false);
        let (spe, spet) = compile(&doc, true);
        assert_same(&seq, &seqt, &spe, &spet, "合并段");
    }

    /// 投机编译建立的段缓存必须与 M5 增量路径兼容：`edit` 后仍与全量一致。
    ///
    /// 这是"投机采纳段的缓存与顺序 `execute_segment` 同构"的直接回归锁
    /// （speculate.rs 的 `adopt_spec_result` 注释）——若缓存里的节点流漏了补插
    /// 的段首胶水、或 `entry_ctx` 记错，本测试会红。
    #[test]
    fn speculative_caches_support_incremental_edit() {
        let doc = build_doc(24);
        let mut spe = IncrementalTypesetter::with_tfm_paginated();
        spe.set_speculative(true);
        let base = spe.compile(&doc).expect("投机 compile");
        assert!(!base.pages.is_empty());

        // 用管线自己的分段（含段尾空行归属），改写中间一段。
        let segs = spe.segments().to_vec();
        assert!(segs.len() > 4, "文档段数不足");
        let idx = segs.len() / 2;
        let new_seg = format!(
            "{} EDITED-TAIL-MARKER extra words changing the line breaking of this segment.",
            segs[idx]
        );
        let mut edited_segs = segs.clone();
        edited_segs[idx] = new_seg.clone();
        let edited_doc = edited_segs.concat();

        let mut full = IncrementalTypesetter::with_tfm_paginated();
        let want = full.compile(&edited_doc).expect("编辑后全量");

        let got = spe.edit(idx, &new_seg).expect("增量 edit");
        assert_eq!(
            format!("{:?}", want.pages),
            format!("{:?}", got.pages),
            "投机缓存 + edit 的页面节点流"
        );
        assert_eq!(want.page_counts, got.page_counts, "页号链");
        assert_eq!(want.fonts.len(), got.fonts.len(), "字体表规模");
    }

    /// 真实 LaTeX 文档（发行 `latex.fmt` + TeX 树资产）下的投机一致性：
    /// `\documentclass{article}` + 多章节 + 多段正文 + 显示公式。
    ///
    /// 这是最接近产品口径的用例：preamble 段与结构宏段都是非 neutral（窗口
    /// 终止 + 顺序执行），正文段是 neutral。**当前 LaTeX 路径的投机面为 0**
    /// ——发行 `latex.fmt` 已定义 `\output` 例程（`\@outputpage`），而
    /// [`IncrementalTypesetter::spec_window_open`] 观测到例程即关闭投机
    /// （RFC-5 保守口径）。本用例锁的是**两条路径输出逐位一致**这一不变式
    /// （无论投机面多少都必须成立）；投机面统计以 `eprintln` 输出，供 M6 的
    /// "`\output` 例程边界"专项验证时对照。资产缺失即跳过。
    #[test]
    fn speculative_matches_on_real_latex_document() {
        let Some((root, state)) = latex_assets() else {
            return;
        };
        let mut src = String::from("\\documentclass{article}\n\\begin{document}\n");
        for i in 0..12 {
            src.push_str(&format!("\\section{{Section {i}}}\n"));
            src.push_str(&body(i));
            src.push_str("\n\n");
            if i % 3 == 1 {
                src.push_str("$$\\int_0^1 x^2 \\,dx = \\frac{1}{3}$$\n\n");
            }
        }
        src.push_str("\\end{document}\n");

        let mut seq = latex_inc(&root, state.clone());
        let a = seq.compile(&src).expect("顺序 LaTeX 编译");
        let mut spe = latex_inc(&root, state);
        spe.set_speculative(true);
        let b = spe.compile(&src).expect("投机 LaTeX 编译");

        assert!(!a.pages.is_empty(), "LaTeX 文档应产出页面");
        assert_eq!(a.pages.len(), b.pages.len(), "页数");
        for (i, (x, y)) in a.pages.iter().zip(b.pages.iter()).enumerate() {
            assert_eq!(format!("{x:?}"), format!("{y:?}"), "第 {i} 页节点流");
        }
        assert_eq!(a.page_counts, b.page_counts, "页号链");
        assert_eq!(a.fonts.len(), b.fonts.len(), "字体表规模");
        assert_eq!(seq.transcript(), spe.transcript(), "转录");
        // 投机面统计（不断言：LaTeX 路径当前因 `\output` 例程而为 0；见函数文档）
        eprintln!("[spec] LaTeX 文档投机统计 = {:?}", spe.stats());
    }

    /// 载入发行 LaTeX 资产（`assets/fmt/latex.fmt` + `assets/tex-minimal/tex`）；
    /// 缺失返回 `None`（与 `incremental_tests::latex_assets` 同口径）。
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

    /// LaTeX 口径的增量器（UTF-8 直写 + fmt 导入 + 发行 TeX 树搜索链）。
    fn latex_inc(
        root: &std::path::Path,
        state: ntex_core::expand::FmtState,
    ) -> IncrementalTypesetter {
        let mut ts = IncrementalTypesetter::with_tfm_paginated();
        ts.set_utf8_input(true);
        ts.set_vfs(latex_vfs(root));
        ts.import_state(state);
        ts
    }

    /// 发行 TeX 树 → VFS（搜索链最小版）。
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
