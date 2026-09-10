//! M9 中文刀 1 回归：`\char` 码位空间的**双向锁**。
//!
//! 本刀把 `\char` / `\iffontchar` / `\fontchar*` 的合法上界从写死的 `255`
//! 改为**由当前字体决定**（`FontLoader::char_code_limit`）：
//!
//! - **8-bit 字体（TFM）→ 255**：`! Bad character code (256).` 是 tex.web §1108
//!   的硬口径，`reference/trip/tripin.log` 与 `fixtures/etrip/etrip.log` 均有
//!   参考块，**不能放宽**；前两个测试锁这条。
//! - **Unicode 直映字体（OTF）→ 0x10FFFF**：对齐 XeTeX，使 `\char"4E2D`
//!   能排汉字；后三个测试锁这条通路（字体字节经 `TfmSource::otf_bytes`
//!   注入，不走 native 文件系统，与 wasm 宿主同一条加载缝）。
//!
//! 样本缺失（CI 无 `~/.ntex-fonts`）时逐测试早退跳过，与 `ntex-font` 的
//! otf 测试同一约定。取样本：
//! `curl -sL https://mirrors.ustc.edu.cn/CTAN/fonts/fandol.zip -o /tmp/fandol.zip`
//! `&& unzip -j -o /tmp/fandol.zip '*.otf' -d ~/.ntex-fonts/`（GPL，CTAN fonts/fandol）。
//!
//! ## 断言口径：看取值，不看报错文本
//!
//! `\fontchar*` 的越界恢复路径调 `report_error`，而该调用在 **`\message`
//! 参数展开期间不会落到转录里**（`\message` 自身输出会出现，错误行不会——
//! 预先存在的行为，与本次改动无关）。因此"闸门是否放行"一律用**取到的
//! 维度值**判定：越界 → `0.0pt`，放行 → `10.0pt`（10pt 设计字号下的 1em）。

use ntex_layout::{set_tfm_source, Node, TfmSource, Typesetter};

/// `\char"4E2D`（汉字「中」）的十进制码位。
const ZHONG: u32 = 0x4E2D;

/// 未映射码位（非字符区，任何字体都不该有字形）。
const UNMAPPED: u32 = 0x10_FFFD;

/// Unicode 上界 + 1（`\char"110000`，越界样本）。
const OVER_UNICODE_MAX: i64 = 0x11_0000;

/// 10pt 设计字号下 1em 的 sp（otf 的 `DEFAULT_DESIGN_SP`）。
///
/// 注意 sp↔pt：1pt = 65536sp，故 655360sp = **10.0pt**——`\the` 输出是
/// `10.0pt` 而非 `654.32pt`。
const EM_SP: i64 = 655_360;

/// 放行时 `\the` 期望字面量；越界恢复值是 `0.0pt`。
const EM_PT: &str = "10.0pt";
const ZERO_PT: &str = "0.0pt";

// ---------- 样本定位与注入 ----------

/// trip.tfm 字节（8-bit TFM 样本；缺则跳过，与 `charwarning.rs` 同约定）。
fn trip_tfm_bytes() -> Option<Vec<u8>> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/trip");
    let p = dir.join("trip.tfm");
    p.is_file().then(|| std::fs::read(&p).ok())?
}

/// Fandol 中文 OTF 字节（缺则跳过）。
fn fandol_bytes() -> Option<Vec<u8>> {
    let p = ntex_font::find_otf("FandolSong-Regular")?;
    std::fs::read(&p).ok()
}

/// 两个样本都在？
fn samples_ready() -> bool {
    trip_tfm_bytes().is_some() && fandol_bytes().is_some()
}

/// 按名供字节的固定源：`trip` → TFM 字节，`zh` → 中文 OTF 字节。
#[derive(Debug)]
struct FixedSource {
    tfm: Vec<u8>,
    otf: Vec<u8>,
}

impl TfmSource for FixedSource {
    fn tfm_bytes(&mut self, name: &str) -> Option<Vec<u8>> {
        (name == "trip").then(|| self.tfm.clone())
    }
    fn otf_bytes(&mut self, name: &str) -> Option<Vec<u8>> {
        (name == "zh").then(|| self.otf.clone())
    }
}

/// 注册字体源。注册表是 `thread_local`，`#[test]` 各自线程 → 测试间零串扰。
fn install_fonts() {
    set_tfm_source(Box::new(FixedSource {
        tfm: trip_tfm_bytes().expect("trip.tfm 已检查存在"),
        otf: fandol_bytes().expect("Fandol 已检查存在"),
    }));
}

/// 递归收集节点树里所有 `Char` 的 `(charcode, width)`。
fn collect_chars(nodes: &[Node], out: &mut Vec<(u32, i64)>) {
    for n in nodes {
        match n {
            Node::Char {
                charcode, width, ..
            } => out.push((*charcode, *width)),
            Node::Box(b) => collect_chars(&b.children, out),
            Node::Ligature { components, .. } => {
                out.extend(components.iter().map(|c| (u32::from(*c), 0)));
            }
            _ => {}
        }
    }
}

/// 排版并取「节点树中的字符」+「终端转录」。
fn run(src: &str) -> (Vec<(u32, i64)>, String) {
    let mut ts = Typesetter::with_tfm();
    let nodes = ts.typeset(src).expect("排版成功");
    let mut chars = Vec::new();
    collect_chars(&nodes, &mut chars);
    (chars, ts.take_transcript())
}

// ---------- 锁一：8-bit TFM 的 255 硬上界不许放宽 ----------

/// `\char256` 在 TFM 字体下仍须报 `! Bad character code (256).`
/// （tex.web §1108；TRIP/ETRIP 参考块回归锁）。
#[test]
fn tfm_font_still_rejects_char_256() {
    if trip_tfm_bytes().is_none() {
        eprintln!("trip.tfm 缺失，跳过");
        return;
    }
    install_fonts();
    let (_, t) = run("\\font\\t=trip \\t \\char256\n");
    assert!(
        t.contains("Bad character code (256)."),
        "8-bit 字体上界必须仍是 255（tex.web §1108），实际转录：\n{t}"
    );
}

/// 边界另一侧：`\char255` 合法（255 是 8-bit 的最后一格），不得误报越界
/// ——字符未定义时只该报 Missing character。
#[test]
fn tfm_font_accepts_char_255() {
    if trip_tfm_bytes().is_none() {
        eprintln!("trip.tfm 缺失，跳过");
        return;
    }
    install_fonts();
    let (_, t) = run("\\tracinglostchars=0 \\font\\t=trip \\t \\char255\n");
    assert!(
        !t.contains("Bad character code"),
        "255 仍在 8-bit 上界内，不该报越界：\n{t}"
    );
}

/// 反向锁：8-bit TFM 字体下 `\fontchar*` 的**两条可达入口**同样不许越界放宽
/// （否则 TRIP/ETRIP 口径会在另一条路径上被悄悄改掉）。
///
/// 取值断言优先：越界 → `0.0pt`；若口径被放宽，`trip.tfm` 的 256 号字符
/// 不存在，取到的是「无度量」的 0，仍是 `0.0pt`——所以**尺寸上下文那条
/// 额外断言错误行确实出现**（该上下文里 `report_error` 能落到转录），
/// 而 `\the` 那条以错误恢复值判定闸门已拦下。
#[test]
fn tfm_font_char_queries_keep_255() {
    if trip_tfm_bytes().is_none() {
        eprintln!("trip.tfm 缺失，跳过");
        return;
    }
    install_fonts();

    // 入口 2：尺寸上下文（`scan_dimen_inner`）。这里错误行会进转录。
    let (_, t) =
        run("\\font\\t=trip \\t \\dimen0=\\fontcharwd\\t256 \\message{scan=\\the\\dimen0}\n");
    assert!(
        t.contains("Bad character code"),
        "256 超出 TFM 的 255 上界，尺寸上下文入口应报越界：\n{t}"
    );
    assert!(
        t.contains(&format!("scan={ZERO_PT}")),
        "越界恢复值应为 {ZERO_PT}：\n{t}"
    );

    // 入口 3：`\the`（`the_tokens_after`）。错误行在此上下文被吞，
    // 故以恢复值判定闸门确实拦下（未放宽则不会是 1em）。
    let (_, t2) = run("\\font\\t=trip \\t \\message{the=\\the\\fontcharwd\\t256}\n");
    assert!(
        t2.contains(&format!("the={ZERO_PT}")),
        "256 超出 255 上界，\\the 入口应拦下并给 {ZERO_PT}（若为 {EM_PT} 即上界被误放宽）：\n{t2}"
    );
    assert!(!t2.contains(EM_PT), "256 在本字体上不该取到 1em：\n{t2}");
}

// ---------- 锁二：OTF 字体走 Unicode 码位空间 ----------

/// OTF 字体下 `\char"4E2D` 必须排成 Char 节点，宽 = 1em，
/// 且**码位不被截断**（DVI `set1`/`set4` 写出路径的回归点）。
#[test]
fn otf_font_typesets_cjk_beyond_255() {
    if !samples_ready() {
        eprintln!("样本缺失，跳过");
        return;
    }
    install_fonts();
    let (chars, t) = run("\\font\\zh=zh \\zh \\hbox{\\char\"4E2D}\n");
    let hit = chars
        .iter()
        .find(|(c, _)| *c == ZHONG)
        .unwrap_or_else(|| panic!("未见 U+4E2D 的 Char 节点，实得 {chars:?}\n转录：{t}"));
    assert_eq!(
        hit.1, EM_SP,
        "全角汉字 = 1em（10pt 设计字号基准 {EM_SP}sp），实得 {}",
        hit.1
    );
    assert!(
        !chars.iter().any(|(c, _)| *c == 0),
        "不得出现码位被截断成 0 的 Char 节点：{chars:?}"
    );
}

/// OTF 字体下超过 Unicode 上界（`\char"110000`）仍须越界报错——
/// 「放宽到 255 之上」不等于「取消上界」。
#[test]
fn otf_font_rejects_beyond_unicode_max() {
    if !samples_ready() {
        eprintln!("样本缺失，跳过");
        return;
    }
    install_fonts();
    let (_, t) = run(&format!("\\font\\zh=zh \\zh \\char{OVER_UNICODE_MAX}\n"));
    assert!(
        t.contains("Bad character code"),
        "0x110000 超出 Unicode 上界，应报越界：\n{t}"
    );
}

/// `\fontchar*` 的两条可达入口必须与 `\char` 同口径（M9 中文刀 1）：
///
/// 1. `\iffontchar`——字形存在性判定（`CondOp::IfFontChar`）；
/// 2. 尺寸上下文——`\dimen0=\fontcharwd\zh"4E2D`（`scan_dimen_inner`）；
/// 3. `\the` —— `\the\fontcharwd\zh"4E2D`（`the_tokens_after`）。
///
/// 后两条是同一语义的两处实现（另有走执行循环的 `exec_fontchar_dimen`，但
/// `\fontcharwd` 不可展开，该分支从常规输入无法到达），历史上只放宽了其中
/// 一处，于是 `\iffontchar\zh"4E2D` 判「有」而 `\dimen0=\fontcharwd\zh"4E2D`
/// 报「Bad character code」——本测试就是这条口径一致性的锁。
#[test]
fn otf_font_char_queries_share_unicode_range() {
    if !samples_ready() {
        eprintln!("样本缺失，跳过");
        return;
    }
    install_fonts();
    let src = format!(
        "\\font\\zh=zh \\zh \
         \\count0=0 \\count1=0 \
         \\iffontchar\\zh\"4E2D \\count0=1 \\fi \
         \\iffontchar\\zh\"{UNMAPPED:X} \\count1=1 \\fi \
         \\dimen0=\\fontcharwd\\zh\"4E2D \
         \\message{{has=\\the\\count0 absent=\\the\\count1 scan=\\the\\dimen0 \
         the=\\the\\fontcharwd\\zh\"4E2D}}\n"
    );
    let (_, t) = run(&src);
    assert!(
        t.contains("has=1"),
        "U+4E2D 在 Fandol 中有字形，\\iffontchar 应取真：\n{t}"
    );
    assert!(t.contains("absent=0"), "未映射码位 U+10FFFD 应取假：\n{t}");
    assert!(
        !t.contains("Bad character code"),
        "U+4E2D 在 Unicode 上界内，两条入口都不该报越界：\n{t}"
    );
    // 两条入口都要给 1em。逐条字面断言（不用 `matches(\"0.0pt\")`：`\"10.0pt\"`
    // 本身含该子串，会误判）。
    assert!(
        t.contains(&format!("scan={EM_PT}")),
        "尺寸上下文入口应给 1em（{EM_SP}sp = {EM_PT}），实际转录：\n{t}"
    );
    assert!(
        t.contains(&format!("the={EM_PT}")),
        "\\the 入口应给 1em（{EM_SP}sp = {EM_PT}），实际转录：\n{t}"
    );
}
