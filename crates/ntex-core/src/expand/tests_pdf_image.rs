use super::*;

// ── 图片管线 Step A：pdfTeX 图片三原语（\pdfximage/\pdflastximage/\pdfrefximage）
//    与三种图源的浅尺寸解析 ─────────────────────────────────────────────

/// 手工拼一个最小 PNG（签名 + IHDR + 可选 pHYs + IEND；解析只读 IHDR/pHYs，
/// 不需要真实压缩数据）。
fn png_bytes(w: u32, h: u32, phys: Option<(u32, u32, u8)>) -> Vec<u8> {
    let mut d = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let chunk = |typ: &[u8; 4], data: &[u8], out: &mut Vec<u8>| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(typ);
        out.extend_from_slice(data);
        out.extend_from_slice(&[0, 0, 0, 0]); // CRC：解析不校验
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // bitdepth/color/压缩/滤波/隔行
    chunk(b"IHDR", &ihdr, &mut d);
    if let Some((x, y, unit)) = phys {
        let mut p = Vec::new();
        p.extend_from_slice(&x.to_be_bytes());
        p.extend_from_slice(&y.to_be_bytes());
        p.push(unit);
        chunk(b"pHYs", &p, &mut d);
    }
    chunk(b"IEND", &[], &mut d);
    d
}

#[test]
fn png_size_uses_phys_density() {
    // 11811 px/m = 300dpi：1520×2239 px → 366.168pt × 539.375pt（GT 对照：
    // transformer-standalone.log `<Figures/ModalNet-21.png, 366.168pt x 539.3751pt>`）
    let d = png_bytes(1520, 2239, Some((11811, 11811, 1)));
    let (w, h) = image_natural_size(&d).expect("PNG 须解析");
    let dpi = 11811.0_f64 * 0.0254;
    assert_eq!(w, (1520.0_f64 * 72.0 / dpi * 65781.76).round() as i64);
    assert_eq!(h, (2239.0_f64 * 72.0 / dpi * 65781.76).round() as i64);
    // pt 换算对照（GT 逐位一致）
    assert!((w as f64 / 65536.0 - 366.168).abs() < 0.002, "w={}", w as f64 / 65536.0);
    assert!((h as f64 / 65536.0 - 539.375).abs() < 0.002, "h={}", h as f64 / 65536.0);
}

#[test]
fn png_size_without_phys_is_72dpi() {
    // 无 pHYs → 1px = 1bp（100px = 100.375pt）
    let d = png_bytes(100, 50, None);
    let (w, h) = image_natural_size(&d).expect("PNG 须解析");
    assert!((w as f64 / 65536.0 - 100.375).abs() < 0.001);
    assert!((h as f64 / 65536.0 - 50.1875).abs() < 0.001);
}

#[test]
fn pdf_size_reads_mediabox() {
    let d = b"%PDF-1.4\n1 0 obj\n<< /Type /Page /MediaBox [ 0 0 612 792 ] >>\nendobj\n";
    let (w, h) = image_natural_size(d).expect("PDF 须解析");
    // MediaBox 是 bp：612bp = 8.5in = 614.295pt
    assert!((w as f64 / 65536.0 - 614.295).abs() < 0.01);
    assert!((h as f64 / 65536.0 - 794.97).abs() < 0.01);
}

#[test]
fn eps_size_reads_bbox_hires_first() {
    let lo = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 80\n%%HiResBoundingBox: 0 0 100.5 80.25\n";
    let (w, h) = image_natural_size(lo).expect("EPS 须解析");
    // HiRes 优先（实数 bp；1bp = 1.00375pt）
    assert!((w as f64 / 65536.0 - 100.5 * 72.27 / 72.0).abs() < 0.01);
    assert!((h as f64 / 65536.0 - 80.25 * 72.27 / 72.0).abs() < 0.01);
    let hi = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 80\n";
    let (w, h) = image_natural_size(hi).expect("EPS 须解析");
    assert!((w as f64 / 65536.0 - 100.375).abs() < 0.01);
    assert!((h as f64 / 65536.0 - 80.3).abs() < 0.01);
}

#[test]
fn unknown_image_format_is_none() {
    assert!(image_natural_size(b"hello world").is_none());
}

#[test]
fn pdfximage_registers_and_sets_last_id() {
    let mut vfs = MemVfs::new();
    vfs.insert("t.png", png_bytes(100, 50, None));
    let (out, _) = expand_vfs(r"\pdfximage{t.png}\the\pdflastximage", vfs).unwrap();
    assert_eq!(out, "1");
}

#[test]
fn pdfximage_ids_increment_and_refximage_runs() {
    let mut vfs = MemVfs::new();
    vfs.insert("a.png", png_bytes(10, 20, None));
    vfs.insert("b.png", png_bytes(30, 40, None));
    // 第二张图 id=2；\pdfrefximage 合成的占位盒在水平列表中不产生字符输出
    let (out, _) =
        expand_vfs(r"\pdfximage{a.png}\pdfximage{b.png}\the\pdflastximage:\pdfrefximage2", vfs)
            .unwrap();
    assert_eq!(out, "2:");
}

#[test]
fn pdfximage_missing_file_errors() {
    let vfs = MemVfs::new();
    let e = expand_vfs(r"\pdfximage{nope.png}", vfs).unwrap_err();
    assert!(format!("{e}").contains("not found"), "{e}");
}

#[test]
fn pdfrefximage_invalid_id_errors() {
    let e = expand_vfs(r"\pdfrefximage7", MemVfs::new()).unwrap_err();
    assert!(format!("{e}").contains("invalid image id"), "{e}");
}

#[test]
fn pdfximage_skips_attr_page_and_pagebox_keywords() {
    // pdftex.def 真实调用形（\\Gread@@pdftex）：attr 组 + page 数字 + 页面盒词
    let mut vfs = MemVfs::new();
    vfs.insert("t.png", png_bytes(4, 4, None));
    let src = r"\pdfximage attr{/Interpolate true} page 1 cropbox{t.png}\the\pdflastximage";
    let (out, _) = expand_vfs(src, vfs).unwrap();
    assert_eq!(out, "1");
}

#[test]
fn pdfximage_hard_bounds_scan_and_scale() {
    // beamer \pgfdeclareimage 真实调用形：`width 14pt height 12pt attr{..}{名}`——
    // 硬边界词必须被消费、文件名组完整识别（回归：此前 width/height 落进
    // 文件名扫描，`height 12pt width 14ptattr {..} {beamericonbook.pdf}` 乱串）。
    // 盒尺寸验证走 CLI 通道（expand_vfs 的默认 sink 无盒寄存器，\setbox/\wd
    // 查询不出值），此处锁 placed_box 纯函数与扫描两端。
    let mut vfs = MemVfs::new();
    vfs.insert("t.png", png_bytes(100, 50, None));
    let src = r"\pdfximage width 14pt height 12pt attr {/Interpolate false } {t.png}\the\pdflastximage";
    let (out, _) = expand_vfs(src, vfs).unwrap();
    assert_eq!(out, "1");
    // placed_box：**混合缩放**（GT pdftex 17 组定案，/tmp/r21b/gt*.tex）——
    // 给出维字面生效、未给维按缩放比等比导出（详见解构 placed_box 文档）。
    // `width 14 height 12` 两维全给 → 字面直用 [14][12][0]。
    let img = PdfXImage {
        natural_w: 100 * 65536,
        natural_h: 50 * 65536,
        name: "t.png".into(),
        hard: Some((14 * 65536, 12 * 65536, 0)),
    };
    assert_eq!(img.placed_box(), (14 * 65536, 12 * 65536, 0));
    // 只给 height → 盒高字面、宽按总高等比导出（GT BOXH：2:1 图
    // `height 10pt` → [20pt][10pt][0]）。
    let img2 = PdfXImage {
        natural_w: 100 * 65536,
        natural_h: 50 * 65536,
        name: "t.png".into(),
        hard: Some((0, 10 * 65536, 0)),
    };
    assert_eq!(img2.placed_box(), (20 * 65536, 10 * 65536, 0));
    // 无硬边界 = 自然尺寸。
    let img3 = PdfXImage {
        natural_w: 100 * 65536,
        natural_h: 50 * 65536,
        name: "t.png".into(),
        hard: None,
    };
    assert_eq!(img3.placed_box(), (100 * 65536, 50 * 65536, 0));
}

#[test]
fn pdfximage_depth_lifts_placeholder_box() {
    // depth/单维组合语义（GT pdftex 17 组实测，/tmp/r21b/gt{,d2,e,f,g}.tex；
    // sp 换算 65781.76/65536 = 1.00375：自然高 50.1875px → 50.3757pt）。
    // 混合缩放模型：给出维字面生效、未给维按缩放比导出；depth 符号参与
    // 缩放比、显示为 max(0,·)；盒深 = max(0, depth) 不设上钳（GT BOXBIG
    // [14][0][99]）。
    let unit = 65781.76_f64;
    let nat = |w: f64, h: f64| PdfXImage {
        natural_w: (w * unit).round() as i64,
        natural_h: (h * unit).round() as i64,
        name: "t.png".into(),
        hard: None,
    };
    let nh = nat(100.375, 50.1875).natural_h;
    let nw = nat(100.375, 50.1875).natural_w;
    let approx = |v: i64, pt: f64| {
        assert!(
            (v as f64 / 65536.0 - pt).abs() < 0.01,
            "{v} vs {pt}pt"
        );
    };
    // depth-only：不缩放、宽高保持自然（GT BOXD=[100.375][48.1875][2]；
    // 回归：此前早退 [nat][nat][0] 丢 depth）。
    let d_only = PdfXImage {
        hard: Some((0, 0, 2 * 65536)),
        ..nat(100.375, 50.1875)
    };
    let (w, h, d) = d_only.placed_box();
    assert_eq!(w, nw, "{w}");
    assert_eq!(h, nh - 2 * 65536, "{h}");
    approx(d, 2.0);
    // width + depth（GT BOX14=[14][5][2]：等比总高 7pt 切 2pt 盒深）。
    let wd = PdfXImage {
        hard: Some((14 * 65536, 0, 2 * 65536)),
        ..nat(100.375, 50.1875)
    };
    let (w, h, d) = wd.placed_box();
    approx(w, 14.0);
    approx(h, 5.0);
    approx(d, 2.0);
    // height-only：宽按高度等比导出（GT BOXH：2:1 图 height 10pt →
    // [20][10][0]；回归：此前宽保持自然）。
    let h_only = PdfXImage {
        hard: Some((0, 10 * 65536, 0)),
        ..nat(100.375, 50.1875)
    };
    let (w, h, d) = h_only.placed_box();
    approx(w, 20.0);
    assert_eq!(h, 10 * 65536, "{h}");
    assert_eq!(d, 0, "{d}");
    // height + depth：等比宽按总高 (h+d) 算（GT BOXHD3：height 10 depth 2
    // → [24][10][2]）；height 字面生效（GT BOXHD：height 3 depth 2 →
    // [14][3][2]，显示总高 5pt = h+d，depth 不被 height 吞）。
    let hd = PdfXImage {
        hard: Some((0, 10 * 65536, 2 * 65536)),
        ..nat(100.375, 50.1875)
    };
    let (w, h, d) = hd.placed_box();
    approx(w, 24.0);
    assert_eq!(h, 10 * 65536, "{h}");
    approx(d, 2.0);
    // 负 depth：无 height → 盒高 = 自然高 + |d|、深钳 0（GT BOXNEGD
    // [100.375][51.1875][0]）；有 height → height 字面、负深钳 0
    //（GT BOXHDN [14][3][0] / BOXHDN-2pt 组 [14][5][0]）。
    let neg = PdfXImage {
        hard: Some((0, 0, -65536)),
        ..nat(100.375, 50.1875)
    };
    let (w, h, d) = neg.placed_box();
    assert_eq!(w, nw, "{w}");
    assert_eq!(h, nh + 65536, "{h}");
    assert_eq!(d, 0, "{d}");
    let neg_h = PdfXImage {
        hard: Some((14 * 65536, 3 * 65536, -2 * 65536)),
        ..nat(100.375, 50.1875)
    };
    let (w, h, d) = neg_h.placed_box();
    approx(w, 14.0);
    assert_eq!(h, 3 * 65536, "{h}");
    assert_eq!(d, 0, "{d}");
    // depth ≥ 等比总高：盒高钳 0、**盒深不钳**（GT BOXBIG=[14][0][99]；
    // 回归：此前 clamp 到自然总高丢边界）。
    let big = PdfXImage {
        hard: Some((14 * 65536, 0, 99 * 65536)),
        ..nat(100.375, 50.1875)
    };
    let (w, h, d) = big.placed_box();
    approx(w, 14.0);
    assert_eq!(h, 0, "{h}");
    approx(d, 99.0);
}

#[test]
fn pdf_minorversion_and_linkmargin_roundtrip() {
    let (out, _) = expand_vfs(
        r"\pdfminorversion=5 \the\pdfminorversion:\pdflinkmargin=2pt \the\pdflinkmargin",
        MemVfs::new(),
    )
    .unwrap();
    assert_eq!(out, "5:2.0pt");
}

#[test]
fn pdfobj_reserve_and_stream_consume_exact_arguments() {
    let (out, _) = expand_vfs(
        r"\pdfobj reserveobjnum\pdfobj reserveobjnum\the\pdflastobj:\immediate\pdfobj useobjnum 1 stream attr{/Type/X}{data}X",
        MemVfs::new(),
    ).unwrap();
    assert_eq!(out, "2:X");
}

#[test]
fn pdfliteral_direct_consumes_group_and_preserves_body_text() {
    let (out, _) = expand_vfs(r"A\pdfliteral direct{0 0 m 1 1 l S}B", MemVfs::new()).unwrap();
    assert_eq!(out, "AB");
}

#[test]
fn pdf_colorstack_push_pop_consumes_group() {
    let (out, _) = expand_vfs(
        r"\pdfcolorstack0 push{red}A\pdfcolorstack0 pop B\pdfcolorstack0 set{blue}C",
        MemVfs::new(),
    )
    .unwrap();
    assert_eq!(out, "A BC");
}

#[test]
fn hyperref_minimal_pdf_primitives_error_free() {
    let src = concat!(
        r"\pdfminorversion=5 ",
        r"\pdflinkmargin=1pt ",
        r"\pdfinfo{/Title(h1)}",
        r"\pdfcatalog{/PageMode/UseNone}",
        r"\pdfcolorstack0 push{0 0 1 rg}",
        r"\pdfdest name{section.1} xyz ",
        r"Section ",
        r"\pdfstartlink attr{/Border[0 0 0]} goto name{section.1}Link\pdfendlink ",
        r"\pdfcolorstack0 pop ",
        r"\message{HYPER-OK}",
    );
    let (r, t) = run_transcript(src);
    assert!(r.is_ok(), "run failed: {r:?}\n{t}");
    assert!(
        !t.contains("Undefined control sequence"),
        "pdfTeX PDF primitive stubs must all be registered: {t}"
    );
    assert!(t.contains("HYPER-OK"), "probe marker missing: {t}");
}

#[test]
fn pdf_dest_and_startlink_consume_common_actions() {
    let (out, _) = expand_vfs(
        concat!(
            r"\pdfdest name{abc} fitr 1pt 2pt 3pt 4pt A",
            r"\pdfdest num 3 xyz B",
            r"\pdfstartlink user{/Subtype/Link/A<<>>}U\pdfendlink ",
            r"\pdfstartlink goto page 1{/Fit}P\pdfendlink",
        ),
        MemVfs::new(),
    )
    .unwrap();
    assert_eq!(out, "A BUP");
}

#[test]
fn pdfpageattr_roundtrips_as_toks_parameter() {
    // beamer/hyperref 闭环（2026-10-24）：`\pdfpageattr`/`\pdfpagesattr` 是
    // pdfTeX 的 toks 型参数。hyperref hpdftex.def 每页 shipout
    // `\the\pdfpageattr` 读 + `\global\pdfpageattr{…}` 写（L623-652）；
    // lscape.sty L54 `\pdfpageattr{/Rotate 90}`。注册前 beamer 最小加载
    // 每页 2 错（Undefined + "You can't use \the with this"）×6 页。
    // 去修复必红：删 builtins.rs 的 pdfpageattr/pdfpagesattr set_register 即红。
    // GT 铁证（pdfTeX t4.tex 实测，\immediate\write16 通道）：
    //   PRE=[/Rotate 90]——首次赋值后 \the 展开，值带 cat10 尾空格；
    //   toks 赋值不展开替换文本（\global\pdfpageattr{\attr…} 存别名原样，
    //   ATTR=[\attr /Trans{}>>]）——递归形态 `\global\pdfpageattr{\the\pdfpageattr…}`
    //   在真 pdfTeX 下 TeX capacity exceeded（input stack 10000），语义即如此。
    let (r, t) = run_transcript(concat!(
        r"\pdfpageattr{/Rotate 90} ",
        // hyperref 同构：读回拼接 + global 重写（先 edef 展开，非递归）
        r"\edef\attr{\the\pdfpageattr}",
        r"\global\pdfpageattr{\attr^^J/Trans{}>>}",
        r"\pdfpagesattr{/CropBox[0 0 100 100]}",
        r"\message{ATTR=[\the\pdfpageattr]}",
        r"\message{PAGES=[\the\pdfpagesattr]}",
    ));
    assert!(r.is_ok(), "run failed: {r:?}\n{t}");
    assert!(
        !t.contains("Undefined control sequence"),
        "pdfpageattr/pdfpagesattr must be registered: {t}"
    );
    assert!(
        !t.contains("can't use"),
        "\\the\\pdfpageattr must read as toks: {t}"
    );
    // 首次赋值的字面值经 edef 展开后存进 toks，读回不展开（GT 同构）。
    assert!(
        t.contains("\\attr") && t.contains("/Trans{}>>"),
        "toks assignment must store tokens verbatim (GT: ATTR=[\\attr …]): {t}"
    );
    assert!(
        t.contains("PAGES=[/CropBox[0 0 100 100]]"),
        "pdfpagesattr round-trip lost: {t}"
    );
}
