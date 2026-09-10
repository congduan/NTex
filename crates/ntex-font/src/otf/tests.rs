//! otf 模块测试——Fandol 实字体 ground truth。
//!
//! 样本定位：优先 [`find_otf`]（`NTEX_OTF_DIR` → `~/.ntex-fonts` →
//! 系统字体目录 → kpsewhich，M9 中文刀 1 起的正式查找链），回落历史
//! `/tmp/latexsurvey` 路径；两者都缺时逐测试早退跳过（CI 无样本）。
//!
//! 取样本：`curl -sL https://mirrors.ustc.edu.cn/CTAN/fonts/fandol.zip -o /tmp/fandol.zip`
//! `&& unzip -j -o /tmp/fandol.zip '*.otf' -d ~/.ntex-fonts/`（GPL，CTAN fonts/fandol）。

use super::*;

fn fandol_path() -> Option<std::path::PathBuf> {
    find_otf("FandolSong-Regular").or_else(|| {
        let p = std::path::PathBuf::from(
            "/tmp/latexsurvey/fonts/opentype/public/fandol/FandolSong-Regular.otf",
        );
        p.is_file().then_some(p)
    })
}

fn load_fandol() -> Option<OtfFont> {
    OtfFont::load(&fandol_path()?).ok()
}

#[test]
fn fandol_load_and_head() {
    let Some(f) = load_fandol() else {
        eprintln!("样本缺失，跳过");
        return;
    };
    assert_eq!(f.family_name().as_deref(), Some("FandolSong"));
    assert_eq!(f.style_name().as_deref(), Some("Regular"));
    assert_eq!(f.global_metrics().unwrap().units_per_em, 1000);
}

#[test]
fn cmap_unicode_lookup() {
    let Some(f) = load_fandol() else {
        eprintln!("样本缺失，跳过");
        return;
    };
    // '中'（U+4E2D）与 'A'（U+41）有映射
    assert!(f.glyph_index('中').is_some());
    assert!(f.glyph_index('A').is_some());
    // 私用区（PUEA，U+E000）Fandol 无映射 → None（正常回落态）
    assert_eq!(f.glyph_index('\u{E000}'), None);
}

#[test]
fn hmtx_advance_fullwidth_and_latin() {
    let Some(f) = load_fandol() else {
        eprintln!("样本缺失，跳过");
        return;
    };
    let gid = f.glyph_index('中').unwrap();
    let m = f.glyph_metrics(gid).unwrap();
    assert_eq!(m.advance_width, 1000, "全角 = 1em（upem=1000）");
    let ga = f.glyph_index('A').unwrap();
    let ma = f.glyph_metrics(ga).unwrap();
    assert!(ma.advance_width > 0 && ma.advance_width < 1000, "拉丁半宽");
}

#[test]
fn outline_cff_points_present() {
    let Some(f) = load_fandol() else {
        eprintln!("样本缺失，跳过");
        return;
    };
    let gid = f.glyph_index('永').unwrap();
    let o = f.glyph_outline(gid).unwrap();
    assert!(
        o.points.len() >= 32,
        "『永』字八法轮廓点数合理：{}",
        o.points.len()
    );
}

#[test]
fn malformed_input_errors_not_panic() {
    let dir = std::env::temp_dir();
    let p1 = dir.join("ntex-otf-empty.otf");
    std::fs::write(&p1, b"").unwrap();
    assert!(OtfFont::load(&p1).is_err());
    let p2 = dir.join("ntex-otf-trunc.otf");
    std::fs::write(&p2, b"OTTO\x00\x01 truncated garbage").unwrap();
    assert!(OtfFont::load(&p2).is_err());
    let p3 = dir.join("ntex-otf-text.otf");
    std::fs::write(&p3, "this is not a font at all").unwrap();
    assert!(OtfFont::load(&p3).is_err());
    let _ = std::fs::remove_file(&p1);
    let _ = std::fs::remove_file(&p2);
    let _ = std::fs::remove_file(&p3);
}

// ---------- M9 中文刀 1：OTF → 引擎度量（build_metrics） ----------

#[test]
fn build_metrics_is_unicode_native_with_fullwidth_cjk() {
    let Some(path) = fandol_path() else {
        eprintln!("样本缺失，跳过");
        return;
    };
    let bytes = std::fs::read(&path).unwrap();
    let fm = build_metrics(bytes, "FandolSong-Regular").expect("构建度量");

    // 标记与结构：Unicode 直映，8-bit 槽表保持空
    assert!(fm.unicode_native, "OTF 字体应为 unicode_native");
    assert!(fm.chars.is_empty(), "unicode_native 字体不走 8-bit 槽表");
    assert!(!fm.unicode_chars.is_empty());
    // 码位升序（char_metrics 二分查询的前提）
    assert!(
        fm.unicode_chars.windows(2).all(|w| w[0].0 < w[1].0),
        "unicode_chars 必须严格升序"
    );

    // 设计字号基准 10pt（DEFAULT_DESIGN_SP），1em = 655360sp
    assert_eq!(fm.design_size_sp, DEFAULT_DESIGN_SP);
    // '中'（U+4E2D）是全角汉字：前进宽度 = 1em
    let (w, h, d) = fm.char_metrics(0x4E2D);
    assert_eq!(w, 655360, "全角汉字宽度应为 1em");
    assert!(h > 0 && d >= 0, "汉字应有正的 height（h={h}, d={d}）");
    // 拉丁半宽：'A' 明显窄于全角
    let (wa, _, _) = fm.char_metrics(0x41);
    assert!(wa > 0 && wa < 655360, "拉丁 'A' 应为半宽（{wa}）");

    // 未映射码位：char_metrics 归零、char_metrics_opt 为 None
    let missing = 0x10FFFD; // 非字符码位，任何字体都无映射
    assert_eq!(fm.char_metrics(missing), (0, 0, 0));
    assert!(fm.char_metrics_opt(missing).is_none());
    assert!(!fm.char_exists(missing));
    assert!(fm.char_exists(0x4E2D));
}

#[test]
fn build_metrics_scales_by_at_and_scaled() {
    let Some(path) = fandol_path() else {
        eprintln!("样本缺失，跳过");
        return;
    };
    let bytes = std::fs::read(&path).unwrap();
    let fm = build_metrics(bytes, "FandolSong-Regular").unwrap();

    // \font\zh=... at 20pt：scale 翻倍、宽度翻倍
    let at20 = fm.scaled_by(2 * DEFAULT_DESIGN_SP, DEFAULT_DESIGN_SP);
    assert_eq!(at20.design_size_sp, DEFAULT_DESIGN_SP, "设计字号保持基准");
    assert_eq!(at20.scale, 2 << 20);
    assert_eq!(at20.char_metrics(0x4E2D).0, 2 * 655360);
    // \font\zh=... scaled 500：千分比 0.5×
    let half = fm.scaled_by(500, 1000);
    assert_eq!(half.char_metrics(0x4E2D).0, 655360 / 2);
    // 缩放不破坏升序（二分前提）
    assert!(half.unicode_chars.windows(2).all(|w| w[0].0 < w[1].0));
}

#[test]
fn metrics_snapshot_covers_cjk_block() {
    let Some(f) = load_fandol() else {
        eprintln!("样本缺失，跳过");
        return;
    };
    let t0 = std::time::Instant::now();
    let snap = f.metrics_snapshot().expect("快照");
    eprintln!(
        "metrics_snapshot 耗时 {:?}（{} 字形 / {} 码位）",
        t0.elapsed(),
        snap.number_of_glyphs,
        snap.chars.len()
    );
    assert_eq!(snap.units_per_em, 1000, "Fandol 系 upem=1000");
    // 常用汉字区（U+4E00..=U+9FFF）应有大量映射
    let cjk = snap
        .chars
        .iter()
        .filter(|(cp, _, _)| (0x4E00..=0x9FFF).contains(cp))
        .count();
    assert!(cjk > 6000, "CJK 基本区映射数偏少（{cjk}）");
    // 升序去重
    assert!(snap.chars.windows(2).all(|w| w[0].0 < w[1].0));
}

#[test]
fn find_otf_resolves_extensionless_name() {
    // 无后缀名字应能定位到 .otf（~/.ntex-fonts 或系统字体目录）
    let Some(p) = find_otf("FandolSong-Regular") else {
        eprintln!("样本缺失，跳过");
        return;
    };
    assert!(p.is_file());
    assert!(p.to_string_lossy().to_ascii_lowercase().ends_with(".otf"));
}
