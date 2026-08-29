//! `Missing character` 警告（tex.web char_warning / new_character）回归测试。
//!
//! tex.web：`\tracinglostchars>0` 且字符不在字体中 → 打印
//! `Missing character: There is no <c> in font <name>!`，且 **不建节点**
//! （`new_character` 返回 null）。NTex 此前把缺失字符按 (0,0,0) 度量
//! 排进去，既无警告又破坏布局。
//!
//! fixtures/trip/trip.tfm 中 char 200（^^c8）未定义——参考 trip.log
//! 正是 `Missing character: There is no ^^c8 in font trip!`。

use ntex_layout::Typesetter;

/// trip.tfm 的绝对路径（缺 fixture 则跳过：与 ntex-trip 的 skip 语义一致）。
fn trip_tfm() -> Option<std::path::PathBuf> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/trip");
    let p = dir.join("trip.tfm");
    p.is_file().then_some(p)
}

fn typeset_with_trip_font(src: &str) -> String {
    let dir = trip_tfm()
        .expect("trip.tfm 已检查存在")
        .parent()
        .unwrap()
        .to_path_buf();
    // SAFETY：测试串行持有（cargo test 同一可执行文件内并发时该 env 只写同一值）
    std::env::set_var("NTEX_TFM_DIR", dir);
    let mut ts = Typesetter::with_tfm();
    let _ = ts.typeset(src);
    ts.take_transcript()
}

#[test]
fn missing_char_warns_with_caret_notation() {
    if trip_tfm().is_none() {
        eprintln!("trip.tfm 缺失，跳过");
        return;
    }
    let t = typeset_with_trip_font("\\tracinglostchars=2 \\font\\t=trip \\t \\char200\n");
    assert!(
        t.contains("Missing character: There is no ^^c8 in font trip!"),
        "char 200 未定义，应报 ^^c8（参考 trip.log L2711）：\n{t}"
    );
}

#[test]
fn missing_char_produces_no_node() {
    if trip_tfm().is_none() {
        eprintln!("trip.tfm 缺失，跳过");
        return;
    }
    let dir = trip_tfm()
        .expect("trip.tfm 已检查存在")
        .parent()
        .unwrap()
        .to_path_buf();
    std::env::set_var("NTEX_TFM_DIR", dir);
    let mut ts = Typesetter::with_tfm();
    // 缺失字符不建节点 → 盒子自然宽度为 0（tex.web new_character 返回 null）
    let out = ts
        .typeset("\\font\\t=trip \\t \\hbox{\\char200}")
        .expect("排版成功");
    let dumped = format!("{out:?}");
    assert!(
        !dumped.contains("Char"),
        "缺失字符不得产出 Char 节点：{dumped}"
    );
}

#[test]
fn present_char_does_not_warn() {
    if trip_tfm().is_none() {
        eprintln!("trip.tfm 缺失，跳过");
        return;
    }
    let t = typeset_with_trip_font("\\tracinglostchars=2 \\font\\t=trip \\t A\n");
    assert!(
        !t.contains("Missing character"),
        "已定义字符（A）不应触发警告：\n{t}"
    );
}

#[test]
fn tracinglostchars_zero_suppresses_warning() {
    if trip_tfm().is_none() {
        eprintln!("trip.tfm 缺失，跳过");
        return;
    }
    let t = typeset_with_trip_font("\\tracinglostchars=0 \\font\\t=trip \\t \\char200\n");
    assert!(
        !t.contains("Missing character"),
        "\\tracinglostchars=0 应抑制警告（TRIP L315 的 -9 同理）：\n{t}"
    );
}
