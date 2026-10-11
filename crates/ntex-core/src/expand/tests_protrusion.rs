use super::*;

// ── 刀H：pdfTeX 凸出/字体扩展原语族（\rpcode/\lpcode/\pdfmatch/\pdffontexpand）
//    GT 对拍：pdflatex p1/p2 探针（2026-10-11，/tmp/bladeH） ───────────────────

#[test]
fn protrude_assignment_is_global_across_group() {
    // GT p2：组内写 200，\endgroup 后读仍 200（恒全局，同 \fontdimen 族）
    let (out, _) = expand_vfs(
        r"\rpcode\nullfont 97=10\begingroup\rpcode\nullfont 97=20\endgroup\number\rpcode\nullfont 97",
        MemVfs::new(),
    )
    .unwrap();
    assert_eq!(out, "20");
}

#[test]
fn rpcode_readable_in_ifnum_operand() {
    // GT p2：\ifnum\rpcode\font`a>50 → T（scan_int 内部位可读；
    // microtype.sty L1038 继承链 RHS 即此路径）。写/读须**分句**：
    // 同句 `=75\ifnum\rpcode…` 的读落在数字循环内联求值位，见到的是
    // 赋值落地前的旧值（tex.web scan_int get_x_token 语义，GT p4 对拍
    // \count 同形 = F）——\relax 终结写侧后读才见新值。
    let (out, _) = expand_vfs(
        r"\rpcode\nullfont 97=75\relax\ifnum\rpcode\nullfont 97>50 T\else F\fi\ifnum\lpcode\nullfont 97>50 X\else Y\fi",
        MemVfs::new(),
    )
    .unwrap();
    assert_eq!(out, "TY");
}

#[test]
fn rpcode_is_not_expandable_in_edef() {
    // GT p2：\edef\m{\rpcode\font`a} 保留字面 token（H2-EDEF:\rpcode \font `a）。
    // \show 经 transcript 验证宏体未展开。
    let src = r"\edef\mx{\rpcode\nullfont 97}\show\mx";
    let (r, t) = run_transcript(src);
    assert!(r.is_ok(), "run failed: {r:?}\n{t}");
    assert!(t.contains("\\rpcode"), "edef 须保留 \\rpcode 字面：{t}");
    assert!(!t.contains("-> 100"), "edef 不得展开出凸出值：{t}");
}

#[test]
fn pdfmatch_boolean_and_bad_pattern() {
    // GT p1/p2：匹配=1（含分组也=1，非子匹配计数）、不匹配=0、坏 pattern=-1
    let (out, _) = expand_vfs(
        concat!(
            r"\edef\ma{\pdfmatch{ab}{xaby}}",
            r"\edef\mb{\pdfmatch{a(b)(c)}{xabc}}",
            r"\edef\mc{\pdfmatch{zzz}{xaby}}",
            r"\edef\md{\pdfmatch{[}{x}}",
            r"\number\ma\number\mb\number\mc\number\md",
        ),
        MemVfs::new(),
    )
    .unwrap();
    assert_eq!(out, "110-1");
}

#[test]
fn pdfmatch_microtype_patterns() {
    // microtype-pdftex.def L69 \MT@ifint / L86 \MT@ifdimen 全形
    // （GT p2：IFINT-TRUE=1 / IFINT-FALSE=0 / IFDIMEN-TRUE=1）
    let ifint = r"{^-*[0-9]+ *$}";
    let ifdimen = r"{^([0-9]+([.,][0-9]+)?|[.,][0-9]+)(em|ex|cm|mm|in|pc|pt|dd|cc|bp|sp|nd|nc|px)? *$}";
    let probe = |arg: &str| {
        let src = format!(
            r"\edef\r{{\pdfmatch{ifint}{{{arg}}}}}\number\r",
            ifint = ifint,
            arg = arg
        );
        expand_vfs(&src, MemVfs::new()).unwrap().0
    };
    assert_eq!(probe("100"), "1");
    assert_eq!(probe("-42"), "1");
    assert_eq!(probe("10pt"), "0");
    let src = format!(r"\edef\r{{\pdfmatch{d}{{10pt}}}}\number\r", d = ifdimen);
    assert_eq!(expand_vfs(&src, MemVfs::new()).unwrap().0, "1");
    let src = format!(r"\edef\r{{\pdfmatch{d}{{1em2}}}}\number\r", d = ifdimen);
    assert_eq!(expand_vfs(&src, MemVfs::new()).unwrap().0, "0");
}

#[test]
fn pdffontexpand_swallow_args_and_optional_keywords() {
    // microtype-pdftex.def L428 现场：`\pdffontexpand\MT@font <s> <s> <step>
    // \MT@auto@\relax`（\MT@auto@ = `auto` 或空）。pdfTeX 手册关键词仅
    // `auto`；吞参 no-op（NTex 无字体扩展引擎）。scan_keyword 前缀匹配与
    // pdfTeX scan_keyword 同构：`autoexpand` 会吃 `auto` 留 `expand`
    // （luatex 专用形，pdflatex microtype 不可达）。
    let (out, _) = expand_vfs(
        concat!(
            r"\pdffontexpand\nullfont 20 10 5 auto\relax Z",
            r"\pdffontexpand\nullfont 2 1 1\relax W",
            r"\pdffontexpand\nullfont 2 1 1 X",
        ),
        MemVfs::new(),
    )
    .unwrap();
    assert_eq!(out, "ZWX");
}

#[test]
fn microtype_protrusion_heir_chain_error_free() {
    // microtype.sty L1036-39 继承链原形（\MT@set@pr@heirs）：赋值两侧
    // 同侧读写 + \relax 收尾——注册前 162+114 处 Undefined 的现场。
    // 读侧读的是**上一句已落地**的 char 97（tex.web 内联求值位语义，
    // 与 microtype 继承链真实数据流一致）。
    let src = concat!(
        r"\rpcode\nullfont 97=100\relax ",
        r"\lpcode\nullfont 97=40\relax ",
        r"\def\heir#1{\lpcode\nullfont #1=\lpcode\nullfont 97\relax ",
        r"\rpcode\nullfont #1=\rpcode\nullfont 97\relax}",
        r"\heir{98}",
    );
    let out = expand_vfs(
        &format!("{src}\\number\\rpcode\\nullfont 98:\\number\\lpcode\\nullfont 98"),
        MemVfs::new(),
    )
    .unwrap()
    .0;
    assert_eq!(out, "100:40");
    let (r, t) = run_transcript(src);
    assert!(r.is_ok(), "run failed: {r:?}\n{t}");
    assert!(
        !t.contains("Undefined control sequence"),
        "凸出原语须已注册：{t}"
    );
}


#[test]
fn scan_font_ident_expands_macro_before_font_slot() {
    // 回归锁（刀H 级联修复 1）：tex.web §586 scan_font_ident 用 get_x_token——
    // 字体位允许宏展开（microtype `\MT@font` ≡ `macro:->\OT1/cmr/m/n/10`，
    // 全部 rpcode/lpcode/fontdimen/hyphenchar 调用点经此）。修复前 mt2
    // 450 处 `Missing font identifier` 由此而来；去修复（退回直接
    // scan_font_ident_slot）必红。
    let src = r"\def\mtf{\nullfont}\rpcode\mtf 97=33\relax \number\rpcode\mtf 97";
    let (out, _) = expand_vfs(src, MemVfs::new()).unwrap();
    assert_eq!(out, "33");
    let (r, t) = run_transcript(src);
    assert!(r.is_ok(), "run failed: {r:?}\n{t}");
    assert!(!t.contains("Missing font identifier"), "{t}");
}

#[test]
fn fontcharwd_readable_in_integer_context() {
    // 回归锁（刀H 级联修复 2）：microtype.sty `\MT@count=\fontcharwd\MT@font\MT@char`
    // ——dimen 内部量在 <internal integer> 语境按 sp 值读（tex.web
    // scan_something_internal 降级）。修复前 mt2 113 处 Missing number 级联。
    let (r, t) = run_transcript(r"\count0=\fontcharwd\nullfont 97\relax \number\count0");
    assert!(r.is_ok(), "run failed: {r:?}\n{t}");
    assert!(!t.contains("Missing number"), "{t}");
}
