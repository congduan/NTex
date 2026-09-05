//! otf 模块测试——Fandol 实字体 ground truth（样本在
//! /tmp/latexsurvey/fonts/opentype/public/fandol/，CI 无样本时跳过）。

use super::*;

const FANDOL: &str = if cfg!(feature = "never") {
    "unreachable"
} else {
    "/tmp/latexsurvey/fonts/opentype/public/fandol/FandolSong-Regular.otf"
};

fn load_fandol() -> Option<OtfFont> {
    OtfFont::load(std::path::Path::new(FANDOL)).ok()
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
    assert!(o.points.len() >= 32, "『永』字八法轮廓点数合理：{}", o.points.len());
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
