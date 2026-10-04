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
    // pdftex.def 真实调用形（\Gread@@pdftex）：attr 组 + page 数字 + 页面盒词
    let mut vfs = MemVfs::new();
    vfs.insert("t.png", png_bytes(4, 4, None));
    let src = r"\pdfximage attr{/Interpolate true} page 1 cropbox{t.png}\the\pdflastximage";
    let (out, _) = expand_vfs(src, vfs).unwrap();
    assert_eq!(out, "1");
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
