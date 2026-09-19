//! 渲染侧字形层（M8 预览字形先行；完整字体子系统属 M9）。
//!
//! TFM 只有度量无轮廓（引擎布局事实源不变），本模块在**渲染侧**补字形：
//!
//! - TeX 字体名（cmr10 等）→ Latin Modern OpenType 文件（LM 与 CM 同源，
//!   度量一致；文本族走光学尺寸族文件，数学族 cmmi/cmsy/cmex 共用
//!   `latinmodern-math.otf` 单文件）；
//! - slot → Unicode 按字体编码分发（[`slot_to_unicode`]）：文本族 OT1
//!   （`ot1enc.def`），数学族 OML/OMS/OMX（槽位锚定 plain.tex mathchardef
//!   与 canonical TeX 编码布局，数学字母数字取 Unicode Mathematical
//!   Alphanumeric Symbols 区）→ skrifa cmap 查字形 id；
//! - 字形绘制双通道：vello glyph run（`vello.rs::append_prims`）与软光栅
//!   轮廓填充（[`GlyphFont::outline_paths`] → `raster::fill_polygon`）；
//! - 字体字节来源：`kpsewhich`/texlive 文件系统之外，
//!   [`register_font_bytes`] 提供进程级注入（wasm 前端 fetch OTF 字节后
//!   注入，无文件系统依赖；native 亦可用于测试与打包分发）。
//!
//! 环境依赖：`kpsewhich`（或 `/usr/local/texlive/*/…/lm` 目录）提供 LM
//! 字体文件；未注册且环境不可用时逐字体回落方框，不报错不 panic（引擎契约）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};

use peniko::{Blob, FontData};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::*;

/// OT1 编码 slot（0..=127）→ Unicode 码位（prims 字形通道用）。
///
/// 依据 `ot1enc.def`：\ss=25、\ae=26、\oe=27、\o=28、\AE=29、\OE=30、\O=31、
/// \i=16、\j=17、\textendash=123、\textemdash=124、\textquotedblleft=92、
/// 引号类在 0x22/0x27/0x60/0x7D、重音 accent 在 0x12..0x18 与 0x7E/0x7F。
/// 0x00..0x0A 为希腊大写（lmroman 缺字形时按码位查不到，自然回落方框）。
pub(crate) fn ot1_to_unicode(slot: u8) -> Option<u32> {
    Some(match slot {
        0x00 => 0x0393, // Γ
        0x01 => 0x0394, // Δ
        0x02 => 0x0398, // Θ
        0x03 => 0x039B, // Λ
        0x04 => 0x039E, // Ξ
        0x05 => 0x03A0, // Π
        0x06 => 0x03A3, // Σ
        0x07 => 0x03A5, // Υ
        0x08 => 0x03A6, // Φ
        0x09 => 0x03A8, // Ψ
        0x0A => 0x03A9, // Ω
        0x0B => 0xFB00, // ff
        0x0C => 0xFB01, // fi
        0x0D => 0xFB02, // fl
        0x0E => 0xFB03, // ffi
        0x0F => 0xFB04, // ffl
        0x10 => 0x0131, // ı
        0x11 => 0x0237, // ȷ
        0x12 => 0x0060, // grave（组合 accent 位）
        0x13 => 0x2019, // quoteright / acute 双用
        0x14 => 0x02DD, // hungarumlaut
        0x15 => 0x02C6, // circumflex
        0x16 => 0x02C7, // caron
        0x17 => 0x00AF, // macron
        0x18 => 0x02D8, // breve
        0x19 => 0x00DF, // ß
        0x1A => 0x00E6, // æ
        0x1B => 0x0153, // œ
        0x1C => 0x00F8, // ø
        0x1D => 0x00C6, // Æ
        0x1E => 0x0152, // Œ
        0x1F => 0x00D8, // Ø
        0x20..=0x21 | 0x23..=0x5B | 0x5D..=0x5F | 0x61..=0x7A => slot as u32,
        0x22 => 0x201D, // "（quotedblright）
        0x5C => 0x201C, // \（quotedblleft）
        0x60 => 0x2018, // `（quoteleft）
        0x7B => 0x2013, // endash
        0x7C => 0x2014, // emdash
        0x7D => 0x2019, // quoteright
        0x7E => 0x02DC, // tilde accent
        0x7F => 0x00A8, // dieresis accent
        // 128..=255：OT1 未定义（T1 等其他编码的高位区），回落方框。
        _ => return None,
    })
}

/// TeX 字体名（如 `cmr10`）的族前缀（首个数字前的字母段）。
pub(crate) fn family_prefix(tex_name: &str) -> (&str, &str) {
    let split = tex_name
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(tex_name.len());
    let (family, size) = tex_name.split_at(split);
    let size = if size.is_empty() { "10" } else { size };
    (family, size)
}

/// OML 编码（cmmi 数学斜体）slot（0..=127）→ Unicode 码位。
///
/// 槽位锚定 plain.tex mathchardef（`\alpha`="010B…`\omega`="0121、`\partial`="0140、
/// `\ell`="0160、`\imath`="017B 等）；拉丁字母取 Unicode Mathematical Italic
/// 区（U+1D434/U+1D44E 起），斜体 h 因 U+1D455 保留改取 U+210E（ℎ）；
/// 希腊小写 U+1D6FC 起（σ/τ 起跳过 ς 位），变体形式（εϑϖϱςϕ∂）取
/// U+1D715..1D71B 的 symbol 位。
pub(crate) fn oml_to_unicode(slot: u8) -> Option<u32> {
    const GREEK_AFTER_PI: [u32; 7] = [
        0x1D70E, // σ
        0x1D70F, // τ
        0x1D710, // υ
        0x1D711, // φ
        0x1D712, // χ
        0x1D713, // ψ
        0x1D714, // ω
    ];
    Some(match slot {
        // 大写序（Unicode 在 Ρ 后多 ϴ 位，非线性）：Γ Δ Θ Λ Ξ Π Σ Υ Φ Ψ Ω。
        0x00 => 0x1D6E4, // Γ
        0x01 => 0x1D6E5, // Δ
        0x02 => 0x1D6E9, // Θ
        0x03 => 0x1D6EC, // Λ
        0x04 => 0x1D6EF, // Ξ
        0x05 => 0x1D6F1, // Π
        0x06 => 0x1D6F4, // Σ
        0x07 => 0x1D6F6, // Υ
        0x08 => 0x1D6F7, // Φ
        0x09 => 0x1D6F9, // Ψ
        0x0A => 0x1D6FA, // Ω
        0x0F => 0x1D716, // \epsilon（lunate ϵ，区别于 0x22 的标准 ε）
        // 0x0B..=0x18：α..ξ 连续段（U+1D6FC 起；0x0F 已被上一臂截走）。
        0x0B..=0x18 => 0x1D6FC + (slot - 0x0B) as u32,
        // 0x19..=0x21：π 起跳过 Unicode 序的 ο（omicron）位。
        0x19 => 0x1D70B, // π
        0x1A => 0x1D70C, // ρ
        0x1B..=0x21 => GREEK_AFTER_PI[(slot - 0x1B) as usize],
        0x22 => 0x1D700,                               // \varepsilon（标准 ε 形）
        0x23 => 0x1D717,                               // \vartheta（ϑ）
        0x24 => 0x1D71B,                               // \varpi（ϖ）
        0x25 => 0x1D71A,                               // \varrho（ϱ）
        0x26 => 0x1D70D,                               // \varsigma（ς，Unicode 序在 ρ 后）
        0x27 => 0x1D719,                               // \varphi（ϕ）
        0x28 => 0x21BC,                                // \leftharpoonup（↼）
        0x29 => 0x21BD,                                // \leftharpoondown（↽）
        0x2A => 0x21C0,                                // \rightharpoonup（⇀）
        0x2B => 0x21C1,                                // \rightharpoondown（ↁ 形 U+21C1）
        0x2E => 0x25B7,                                // \triangleright（▷）
        0x2F => 0x25C1,                                // \triangleleft（◁）
        0x30..=0x39 => slot as u32,                    // 旧式数字（cmap 直通 ASCII 位）
        0x3C => 0x003C,                                // <
        0x3E => 0x003E,                                // >
        0x40 => 0x1D715,                               // \partial（∂ 斜体）
        0x41..=0x5A => 0x1D434 + (slot - 0x41) as u32, // A..Z 斜体
        0x5E => 0x2323,                                // \smile（⌣）
        0x5F => 0x2322,                                // \frown（⌢）
        0x60 => 0x2113,                                // \ell（ℓ）
        // 0x61..=0x7A：a..z 斜体；h 位 U+1D455 在 Unicode 保留，改 ℎ。
        0x61..=0x7A => {
            if slot == b'h' {
                0x210E
            } else {
                0x1D44E + (slot - 0x61) as u32
            }
        }
        0x7B => 0x0131, // \imath（ı）
        0x7C => 0x0237, // \jmath（ȷ）
        0x7D => 0x2118, // \wp（℘）
        _ => return None,
    })
}

/// OMS 编码（cmsy 数学符号）slot（0..=127）→ Unicode 码位。
///
/// 槽位锚定 plain.tex mathchardef（`\pm`="2206、`\circ`="220E、`\bullet`="220F、
/// `\leq`="3214、`\rightarrow`="3221、`\langle` 用 \delimiter"426830A → 小件
/// cmsy 0x68 等）与 canonical OMS 布局。
pub(crate) fn oms_to_unicode(slot: u8) -> Option<u32> {
    Some(match slot {
        0x00 => 0x2212, // −
        0x01 => 0x22C5, // ⋅
        0x02 => 0x00D7, // ×
        0x03 => 0x2217, // ∗
        0x04 => 0x00F7, // ÷
        0x05 => 0x22C4, // ⋄
        0x06 => 0x00B1, // ±
        0x07 => 0x2213, // ∓
        0x08 => 0x2295, // ⊕
        0x09 => 0x2296, // ⊖
        0x0A => 0x2297, // ⊗
        0x0B => 0x2298, // ⊘
        0x0C => 0x2299, // ⊙
        0x0D => 0x25EF, // ◯（\bigcirc）
        0x0E => 0x2218, // ∘
        0x0F => 0x2219, // •
        0x10 => 0x224D, // ≍（\asymp）
        0x11 => 0x2261, // ≡
        0x12 => 0x2286, // ⊆
        0x13 => 0x2287, // ⊇
        0x14 => 0x2264, // ≤
        0x15 => 0x2265, // ≥
        0x16 => 0x227C, // ≼
        0x17 => 0x227D, // ≽
        0x18 => 0x223C, // ∼
        0x19 => 0x2248, // ≈
        0x1A => 0x2282, // ⊂
        0x1B => 0x2283, // ⊃
        0x1C => 0x226A, // ≪
        0x1D => 0x226B, // ≫
        0x1E => 0x227A, // ≺
        0x1F => 0x227B, // ≻
        0x20 => 0x2190, // ←
        0x21 => 0x2192, // →
        0x22 => 0x2191, // ↑
        0x23 => 0x2193, // ↓
        0x24 => 0x2194, // ↔
        0x25 => 0x2197, // ↗
        0x26 => 0x2198, // ↘
        0x27 => 0x2195, // ↕
        0x28 => 0x21D0, // ⇐
        0x29 => 0x21D2, // ⇒
        0x2A => 0x21D1, // ⇑
        0x2B => 0x21D3, // ⇓
        0x2C => 0x21D4, // ⇔
        0x2D => 0x2196, // ↖
        0x2E => 0x2199, // ↙
        0x2F => 0x221D, // ∝
        0x31 => 0x221E, // ∞（\infty="1231）
        0x32 => 0x2032, // ′（\prime）
        0x33 => 0x2205, // ∅
        0x34 => 0x22A4, // ⊤
        0x35 => 0x22A5, // ⊥
        0x38 => 0x2200, // ∀
        0x39 => 0x2203, // ∃
        0x3A => 0x00AC, // ¬
        0x3C => 0x211C, // ℜ（\Re）
        0x3D => 0x2111, // ℑ（\Im）
        0x40 => 0x2135, // ℵ
        0x50 => 0x2211, // ∑（\sum="1350）
        0x5B => 0x222A, // ∪
        0x5C => 0x2229, // ∩
        0x5D => 0x228E, // ⊎（\uplus）
        0x5E => 0x2227, // ∧
        0x5F => 0x2228, // ∨
        0x60 => 0x22A2, // ⊢
        0x61 => 0x22A3, // ⊣
        0x64 => 0x2308, // ⌈
        0x65 => 0x2309, // ⌉
        0x66 => 0x230A, // ⌊
        0x67 => 0x230B, // ⌋
        0x68 => 0x27E8, // ⟨（\langle 小件）
        0x69 => 0x27E9, // ⟩
        0x6A => 0x2223, // ∣（\mid）
        0x6B => 0x2225, // ∥
        0x6E => 0x2216, // ∖（\setminus）
        0x6F => 0x2240, // ≀（\wr）
        0x71 => 0x2A3F, // ⨿（\amalg）
        0x72 => 0x2207, // ∇
        0x74 => 0x2294, // ⊔
        0x75 => 0x2293, // ⊓
        0x76 => 0x2291, // ⊑
        0x77 => 0x2292, // ⊒
        0x79 => 0x2020, // †
        0x7A => 0x2021, // ‡
        0x7C => 0x2663, // ♣
        0x7D => 0x2662, // ♢
        0x7E => 0x2661, // ♡
        0x7F => 0x2660, // ♠
        _ => return None,
    })
}

/// OMX 编码（cmex 大型定界/积分/大算符）slot → Unicode 码位（常用子集；
/// display 变体经 next_larger 放大（如 `\sum` cmsy 0x50 → cmex 0x58）映射回
/// 同一基字符 Unicode，字号由 TFM 度量放大；括号拼接件属多段图形，留方框
/// 口径待后续）。
pub(crate) fn omx_to_unicode(slot: u8) -> Option<u32> {
    Some(match slot {
        0x46 | 0x47 => 0x2A06, // ⨆（\bigsqcup 及 display 变体）
        0x4A | 0x4B => 0x2A00, // ⨀
        0x4C | 0x4D => 0x2A01, // ⨁
        0x4E | 0x4F => 0x2A02, // ⨂
        0x50 | 0x58 => 0x2211, // ∑（display = next_larger 终点 0x58）
        0x51 | 0x59 => 0x220F, // ∏
        0x52 | 0x5A => 0x222B, // ∫（\intop="1352）
        0x53 | 0x5B => 0x22C3, // ⋃
        0x54 | 0x5C => 0x22C2, // ⋂
        0x55 | 0x5D => 0x2A04, // ⨄
        0x56 | 0x5E => 0x22C0, // ⋀
        0x57 | 0x5F => 0x22C1, // ⋁
        0x60 | 0x61 => 0x2210, // ∐（\coprod）
        0x70..=0x73 => 0x221A, // √（\radical"270370 及 next_larger 链）
        _ => return None,
    })
}

/// 按字体族分发的 slot → Unicode（prims 字形通道统一入口）：
/// cmmi→OML、cmsy→OMS、cmex→OMX、cmtt 系→ASCII 直通（cmtt 编码的花括号/
/// 反斜杠在 OT1 位上是 ligature/标点，`\string`/`\char` 转录须按字面出），
/// 其余文本族→OT1。
pub(crate) fn slot_to_unicode(tex_name: &str, slot: u8) -> Option<u32> {
    let (family, _) = family_prefix(tex_name);
    match family {
        "cmmi" => oml_to_unicode(slot),
        "cmsy" => oms_to_unicode(slot),
        "cmex" => omx_to_unicode(slot),
        "cmtt" | "cmsltt" | "cmtex" => {
            if (0x20..=0x7E).contains(&slot) {
                Some(slot as u32)
            } else {
                ot1_to_unicode(slot)
            }
        }
        _ => ot1_to_unicode(slot),
    }
}

/// TeX 文本字体名 → Latin Modern OpenType 文件名（无映射的字体返回 None，
/// 调用方回落占位方框）。
///
/// 覆盖 cm 常用文本族；数学族（cmmi/cmsy/cmex）共用 OpenType MATH 字体
/// `latinmodern-math.otf`（LM 无独立 lmmi/lmsy OTF 文件，数学字形全在
/// lm-math 包这一个 MATH 表字体内，cmap 覆盖数学字母数字区）。
fn lm_file_name(tex_name: &str) -> Option<String> {
    let (family, size) = family_prefix(tex_name);
    // LM 光学尺寸族：5/6/7/8/9/10/12/17；其余字号取就近存在的文件由
    // kpsewhich 决定（找不到即回落）。
    let style = match family {
        "cmr" => format!("lmroman{size}-regular"),
        "cmbx" | "cmb" => format!("lmroman{size}-bold"),
        "cmti" => format!("lmroman{size}-italic"),
        "cmsl" => format!("lmroman{size}-oblique"),
        "cmtt" => format!("lmmono{size}-regular"),
        "cmsltt" => format!("lmmono{size}-oblique"),
        "cmss" => format!("lmsans{size}-regular"),
        "cmssbx" => format!("lmsans{size}-bold"),
        "cmssi" => format!("lmsans{size}-oblique"),
        // 数学族：LM 无独立 OTF，统一走 OpenType MATH 单文件。
        "cmmi" | "cmsy" | "cmex" => "latinmodern-math".to_owned(),
        _ => return None,
    };
    Some(format!("{style}.otf"))
}

/// 已解析的字形字体（共享只读）。
pub struct GlyphFont {
    /// 原始字体字节（软光栅轮廓提取用；与 FontData/Blob 内部 Arc 同源共享）。
    bytes: Arc<[u8]>,
    /// vello glyph run 绘制句柄（Blob 内部 Arc，clone 零拷贝）。
    font: FontData,
    /// Unicode 码位 → 字形 id（BMP 全量预填，一次解析常驻查询）。
    map: HashMap<u32, u32>,
}

impl GlyphFont {
    /// 解析 OTF 字节：skrifa 校验格式 + dump cmap（全 Unicode 区间预填，
    /// ~111 万次二分查找，毫秒级一次性成本；数学字母数字区在平面 1，
    /// latinmodern-math 亦在此区）。
    fn load(bytes: Vec<u8>) -> Option<Self> {
        let bytes: Arc<[u8]> = bytes.into();
        let font_ref = skrifa::FontRef::new(bytes.as_ref()).ok()?;
        let charmap = font_ref.charmap();
        let mut map = HashMap::new();
        for ch in 0u32..=0x10FFFF {
            if let Some(gid) = charmap.map(ch) {
                map.insert(ch, gid.to_u32());
            }
        }
        Some(Self {
            // Blob 仅支持 From<Vec<u8>>（多一次拷贝，注册期一次性成本）。
            font: FontData::new(Blob::from(bytes.to_vec()), 0),
            bytes,
            map,
        })
    }

    /// Unicode → 字形 id。
    pub(crate) fn glyph_id(&self, ch: u32) -> Option<u32> {
        self.map.get(&ch).copied()
    }

    /// vello 绘制句柄。
    pub fn font_data(&self) -> &FontData {
        &self.font
    }

    /// 字形在给定字号下的墨迹纵向跨度（页面坐标，y 向下；基线锚定测量）：
    /// `(lo, hi)`，lo ≤ 基线（上方为负）。轮廓缺失/解析失败返回 None。
    pub(crate) fn glyph_vspan_px(&self, gid: u32, size_px: f64) -> Option<(f64, f64)> {
        let paths = self.outline_paths(gid, size_px, 0.0, 0.0);
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for c in &paths {
            for p in c {
                lo = lo.min(p[1] as f64);
                hi = hi.max(p[1] as f64);
            }
        }
        (hi >= lo).then_some((lo, hi))
    }

    /// 提取字形轮廓为页面坐标多边形组（软光栅填充用；vello 走 glyph run
    /// 不经此路）。
    ///
    /// `(ox, oy)` = 基线原点（px，y 向下，同 `RectPrim` 坐标系）；
    /// `size_px` = em 的像素数（skrifa unhinted 按 ppem 缩放）；
    /// 字体坐标 y 向上，收集时翻转到页面系。缺字形/解析失败返回空组
    ///（调用方按占位方框口径回落，由 prims 层保证）。
    pub(crate) fn outline_paths(
        &self,
        gid: u32,
        size_px: f64,
        ox: f64,
        oy: f64,
    ) -> Vec<Vec<[f32; 2]>> {
        let Ok(font_ref) = skrifa::FontRef::new(self.bytes.as_ref()) else {
            return Vec::new();
        };
        let Some(glyph) = font_ref.outline_glyphs().get(GlyphId::new(gid)) else {
            return Vec::new();
        };
        let mut pen = PathPen::new(ox, oy);
        if glyph
            .draw(
                DrawSettings::unhinted(Size::new(size_px as f32), LocationRef::default()),
                &mut pen,
            )
            .is_err()
        {
            return Vec::new();
        }
        pen.finish()
    }
}

/// skrifa 轮廓 → 页面坐标多边形收集器（`OutlinePen` 实现）。
///
/// 贝塞尔按控制多边形折线长自适应展平（~1.25px/段，4..=32 夹断）；
/// 子路径在 `close`/下一段 `move_to` 时收束（不足 3 点的退化段丢弃）。
struct PathPen {
    contours: Vec<Vec<[f32; 2]>>,
    cur: Vec<[f32; 2]>,
    ox: f32,
    oy: f32,
}

impl PathPen {
    fn new(ox: f64, oy: f64) -> Self {
        Self {
            contours: Vec::new(),
            cur: Vec::new(),
            ox: ox as f32,
            oy: oy as f32,
        }
    }

    /// 字体坐标 → 页面坐标（y 翻转 + 基线平移）。
    fn map(&self, x: f32, y: f32) -> [f32; 2] {
        [self.ox + x, self.oy - y]
    }

    /// 曲线细分段数：`a → pts` 折线长 / 1.25px。
    fn segments(a: [f32; 2], pts: &[[f32; 2]]) -> usize {
        let mut len = 0.0f32;
        let mut prev = a;
        for p in pts {
            len += (p[0] - prev[0]).hypot(p[1] - prev[1]);
            prev = *p;
        }
        ((len / 1.25).ceil() as usize).clamp(4, 32)
    }

    fn end_contour(&mut self) {
        if self.cur.len() >= 3 {
            self.contours.push(std::mem::take(&mut self.cur));
        } else {
            self.cur.clear();
        }
    }

    fn finish(mut self) -> Vec<Vec<[f32; 2]>> {
        self.end_contour();
        self.contours
    }
}

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.end_contour();
        self.cur.push(self.map(x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        if !self.cur.is_empty() {
            self.cur.push(self.map(x, y));
        }
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let Some(&a) = self.cur.last() else { return };
        let c = self.map(cx, cy);
        let b = self.map(x, y);
        let n = Self::segments(a, &[c, b]);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let p = [
                u * u * a[0] + 2.0 * u * t * c[0] + t * t * b[0],
                u * u * a[1] + 2.0 * u * t * c[1] + t * t * b[1],
            ];
            self.cur.push(p);
        }
    }

    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        let Some(&a) = self.cur.last() else { return };
        let c1 = self.map(c1x, c1y);
        let c2 = self.map(c2x, c2y);
        let b = self.map(x, y);
        let n = Self::segments(a, &[c1, c2, b]);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let p = [
                u * u * u * a[0]
                    + 3.0 * u * u * t * c1[0]
                    + 3.0 * u * t * t * c2[0]
                    + t * t * t * b[0],
                u * u * u * a[1]
                    + 3.0 * u * u * t * c1[1]
                    + 3.0 * u * t * t * c2[1]
                    + t * t * t * b[1],
            ];
            self.cur.push(p);
        }
    }

    fn close(&mut self) {
        self.end_contour();
    }
}

/// 定位 LM 字体文件：`kpsewhich`（PATH）→ texlive 常见目录回落（GUI 进程
/// 可能没有用户 PATH，直接扫安装目录）。
fn locate_font(file: &str) -> Option<PathBuf> {
    if let Ok(out) = Command::new("kpsewhich").arg(file).output() {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    // texlive 回落：/usr/local/texlive/<年>/texmf-dist/fonts/opentype/public/lm/
    let mut hits: Vec<PathBuf> = glob_lm_candidates(file);
    hits.sort(); // 年份字典序取最新
    hits.pop()
}

/// 收集 texlive 安装目录下的候选路径（目录缺失时为空）。
fn glob_lm_candidates(file: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/usr/local/texlive") else {
        return out;
    };
    for year in entries.flatten() {
        let p = year
            .path()
            .join("texmf-dist/fonts/opentype/public/lm")
            .join(file);
        if p.is_file() {
            out.push(p);
        }
    }
    out
}

/// 按 TeX 字体名直接定位 OpenType 字体文件（M9 中文刀 1）。
///
/// 与 [`lm_file_name`] 的「CM 家族名 → LM 文件名」改名映射不同：这里把 TeX
/// 字体名**直接当字体文件名**（中文 Fandol/思源、用户自带 OTF/TTF），查找链
/// 复用 `ntex_font::find_otf`——与引擎侧 `TfmLoader` 完全同源，保证
/// 「引擎能量到的字体，渲染端也能量到」，不会出现度量已按 CJK 字体算、
/// 渲染却回落占位方框的错配。
fn locate_by_font_name(tex_name: &str) -> Option<Arc<GlyphFont>> {
    let path = ntex_font::find_otf(tex_name)?;
    let bytes = std::fs::read(path).ok()?;
    GlyphFont::load(bytes).map(Arc::new)
}

/// 进程级字体字节注册表：[`register_font_bytes`] 写入，
/// [`GlyphCache::resolve`] 优先于文件系统查找命中。
static REGISTRY: LazyLock<Mutex<HashMap<String, Arc<GlyphFont>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 注册字体字节（`tex_name` 为 TeX 排版字体名如 `cmr10`，OTF/TTF 均可）。
///
/// 解析失败（坏格式）返回 `false`；成功后两后端（软光栅/vello）按名字即刻
/// 可用。wasm 前端 fetch OTF 字节后经此注入（无 kpsewhich/文件系统）；
/// native 端用于测试与打包分发场景，同名覆盖环境字体。
pub fn register_font_bytes(tex_name: &str, bytes: &[u8]) -> bool {
    let Some(font) = GlyphFont::load(bytes.to_vec()) else {
        return false;
    };
    match REGISTRY.lock() {
        Ok(mut m) => {
            m.insert(tex_name.to_owned(), Arc::new(font));
            true
        }
        // 锁毒化：持锁线程已 panic，注册失败按环境回落处理（不传播错误）。
        Err(_) => false,
    }
}

/// 批量认领一个字体目录（发行字体集）：**与 `ntex-tauri` 前端
/// `ui/main.js::GLYPH_FONTS` 同一张名单的 Rust 侧等价物**。
///
/// 认领规则见 [`claim_font_dir`]；本函数读盘并逐条注册进进程级注册表，
/// 返回成功注册的 TeX 字体名（诊断/状态栏提示；顺序 = 认领顺序）。
/// 目录不存在/不可读 → 空表（native 环境查找链继续兜底，不是错误）。
pub fn register_font_dir(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    for (tex_name, path) in claim_font_dir(dir) {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if register_font_bytes(&tex_name, &bytes) {
            out.push(tex_name);
        }
    }
    out
}

/// 目录 → 认领表（`TeX 字体名 → 字体文件路径`），**不注册**（纯函数，测试用）。
///
/// 两类认领：
/// 1. **LM 改名映射**：目录内文件名与 [`lm_file_name`] 的候选集
///    （[`lm_tex_name_candidates`]）求交——`lmroman10-regular.otf` → `cmr10`、
///    `latinmodern-math.otf` → cmmi/cmsy/cmex 全系；
/// 2. **同名直取**：其余 OpenType 文件按**文件主名**认领为 TeX 字体名
///    （`FandolSong-Regular.otf` → `FandolSong-Regular`，与引擎侧
///    `find_otf` 的"TeX 名当文件名"口径一致）。
///
/// 顺序：先映射族（同一文件可能被多个 TeX 名认领，故按 TeX 名逐个入表），
/// 再同名直取；已被认领过的 TeX 名/文件不再重复。
fn claim_font_dir(dir: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    // 目录内 OpenType 文件：小写文件名 → 路径（大小写不敏感匹配，跨平台稳）。
    let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".otf") || lower.ends_with(".ttf") || lower.ends_with(".ttc") {
            files.push((lower, path));
        }
    }

    let mut out: Vec<(String, std::path::PathBuf)> = Vec::new();
    // 1) LM 改名映射（latinmodern-math 由多个 TeX 名指向 → 同一路径多次入表）。
    for tex in lm_tex_name_candidates() {
        let Some(file) = lm_file_name(&tex) else {
            continue;
        };
        let file = file.to_ascii_lowercase();
        let Some((_, path)) = files.iter().find(|(n, _)| *n == file) else {
            continue;
        };
        out.push((tex, path.clone()));
    }
    // 2) 同名直取（中文字体等无 LM 映射的 OTF）：文件主名即 TeX 名。
    //    已被映射族认领的**文件**跳过——否则 `lmroman10-regular.otf` 会额外
    //    以 `lmroman10-regular` 之名入表（Tauri 名单里没有这种名字，多余）。
    for (_, path) in &files {
        if out.iter().any(|(_, p)| p == path) {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if out.iter().any(|(n, _)| n == stem) {
            continue;
        }
        out.push((stem.to_owned(), path.clone()));
    }
    out
}

/// LM 认领的候选 TeX 名（族前缀 × 光学尺寸）：[`register_font_dir`] 用它反查
/// [`lm_file_name`]，把目录里的 LM 文件还原成 TeX 名。
///
/// 与 `ntex-tauri` 前端 `GLYPH_FONTS` 名单同源（那份是手写清单，这里是等价
/// 生成式；两侧只要 LM 文件名映射不变即等价）。
fn lm_tex_name_candidates() -> Vec<String> {
    const FAMILIES: &[&str] = &[
        "cmr", "cmbx", "cmb", "cmti", "cmsl", "cmtt", "cmsltt", "cmss", "cmssbx", "cmssi", "cmmi",
        "cmsy", "cmex",
    ];
    const SIZES: &[u32] = &[5, 6, 7, 8, 9, 10, 12, 17];
    let mut out = Vec::new();
    for f in FAMILIES {
        for s in SIZES {
            out.push(format!("{f}{s}"));
        }
    }
    out
}

/// 跨页/跨渲染的字形字体缓存（TeX 字体名 → 解析结果；None = 环境无此字体，
/// 后续同名不再重试）。
#[derive(Default)]
pub struct GlyphCache {
    resolved: HashMap<String, Option<Arc<GlyphFont>>>,
}

impl GlyphCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// 按名字解析（命中缓存直接返回；注册表 → kpsewhich/texlive → 字体目录
    /// 均未命中返回 None，字符走方框口径）。
    pub(crate) fn resolve(&mut self, tex_name: &str) -> Option<Arc<GlyphFont>> {
        if let Some(hit) = self.resolved.get(tex_name) {
            return hit.clone();
        }
        // 注册表优先（显式注入覆盖环境查找），未注册再走文件系统定位。
        let registered = REGISTRY.lock().ok().and_then(|m| m.get(tex_name).cloned());
        let loaded = registered.or_else(|| {
            lm_file_name(tex_name)
                .and_then(|file| locate_font(&file))
                .and_then(|path| std::fs::read(path).ok())
                .and_then(GlyphFont::load)
                .map(Arc::new)
                // 非 LM 字体（M9 中文刀 1）：TeX 字体名直接当字体文件名定位
                // （FandolSong-Regular / xxx.otf 等），与引擎侧 find_otf 同源。
                .or_else(|| locate_by_font_name(tex_name))
        });
        self.resolved.insert(tex_name.to_owned(), loaded.clone());
        loaded
    }

    /// 解析失败（无字体文件）的 TeX 字体名，供调用方提示回落。
    pub fn missing(&self) -> impl Iterator<Item = &str> {
        self.resolved
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ot1_ascii_passthrough_and_specials() {
        // ASCII 主干直通。
        assert_eq!(ot1_to_unicode(b'A'), Some(0x41));
        assert_eq!(ot1_to_unicode(b'z'), Some(0x7A));
        assert_eq!(ot1_to_unicode(b'0'), Some(0x30));
        // ot1enc.def 权威 slot。
        assert_eq!(ot1_to_unicode(25), Some(0x00DF)); // ß
        assert_eq!(ot1_to_unicode(31), Some(0x00D8)); // Ø
        assert_eq!(ot1_to_unicode(123), Some(0x2013)); // endash
        assert_eq!(ot1_to_unicode(124), Some(0x2014)); // emdash
        assert_eq!(ot1_to_unicode(92), Some(0x201C)); // quotedblleft
                                                      // ligature 区。
        assert_eq!(ot1_to_unicode(0x0B), Some(0xFB00)); // ff
        assert_eq!(ot1_to_unicode(0x0E), Some(0xFB03)); // ffi
                                                        // 全区间 0..=127 均有映射（OT1 无空洞）。
        for b in 0u8..=127 {
            assert!(ot1_to_unicode(b).is_some(), "slot {b:#x} 缺映射");
        }
    }

    #[test]
    fn lm_file_name_common_families() {
        assert_eq!(
            lm_file_name("cmr10").as_deref(),
            Some("lmroman10-regular.otf")
        );
        assert_eq!(
            lm_file_name("cmti10").as_deref(),
            Some("lmroman10-italic.otf")
        );
        assert_eq!(
            lm_file_name("cmbx12").as_deref(),
            Some("lmroman12-bold.otf")
        );
        assert_eq!(
            lm_file_name("cmssbx10").as_deref(),
            Some("lmsans10-bold.otf")
        );
        // 未知族回落。
        assert_eq!(lm_file_name("unknown10"), None);
        // 数学族统一走 OpenType MATH 单文件。
        assert_eq!(
            lm_file_name("cmmi10").as_deref(),
            Some("latinmodern-math.otf")
        );
        assert_eq!(
            lm_file_name("cmsy7").as_deref(),
            Some("latinmodern-math.otf")
        );
    }

    #[test]
    fn math_encoding_slot_anchors() {
        // OML（cmmi）：plain.tex mathchardef 权威位。
        assert_eq!(slot_to_unicode("cmmi10", 0x19), Some(0x1D70B)); // \pi="0119
        assert_eq!(slot_to_unicode("cmmi10", 0x0B), Some(0x1D6FC)); // \alpha="010B
        assert_eq!(slot_to_unicode("cmmi10", 0x1B), Some(0x1D70E)); // \sigma="011B
        assert_eq!(slot_to_unicode("cmmi10", 0x22), Some(0x1D700)); // \varepsilon="0122
        assert_eq!(slot_to_unicode("cmmi10", 0x0F), Some(0x1D716)); // \epsilon（lunate）
        assert_eq!(slot_to_unicode("cmmi10", 0x40), Some(0x1D715)); // \partial="0140
        assert_eq!(slot_to_unicode("cmmi10", 0x60), Some(0x2113)); // \ell="0160
        assert_eq!(slot_to_unicode("cmmi10", 0x68), Some(0x210E)); // 斜体 h → ℎ（U+1D455 保留）
        assert_eq!(slot_to_unicode("cmmi10", b'a'), Some(0x1D44E));
        assert_eq!(slot_to_unicode("cmmi10", b'Z'), Some(0x1D44D));
        // OMS（cmsy）：demo1 实测翻车位——\sum 的 0x50 不能再当 OT1 'P'。
        assert_eq!(slot_to_unicode("cmsy10", 0x50), Some(0x2211)); // ∑
        assert_eq!(slot_to_unicode("cmsy10", 0x0F), Some(0x2219)); // \bullet="220F
        assert_eq!(slot_to_unicode("cmsy10", 0x08), Some(0x2295)); // ⊕（\oplus="2208）
        assert_eq!(slot_to_unicode("cmsy10", 0x14), Some(0x2264)); // ≤（\leq="3214）
        assert_eq!(slot_to_unicode("cmsy10", 0x21), Some(0x2192)); // →（\rightarrow="3221）
        assert_eq!(slot_to_unicode("cmsy10", 0x40), Some(0x2135)); // ℵ（\aleph="0240）
        assert_eq!(slot_to_unicode("cmsy10", 0x31), Some(0x221E)); // ∞（\infty="1231）
                                                                   // cmtt 编码：花括号/反斜杠按字面 ASCII（\string 转录口径）。
        assert_eq!(slot_to_unicode("cmtt10", 0x7B), Some(0x7B)); // {
        assert_eq!(slot_to_unicode("cmtt10", 0x5C), Some(0x5C)); // \
        assert_eq!(slot_to_unicode("cmr10", 0x7B), Some(0x2013)); // OT1 endash 对照
        assert_eq!(slot_to_unicode("cmsy10", 0x68), Some(0x27E8)); // ⟨（\delimiter"426830A）
                                                                   // OMX（cmex）：plain.tex 大算符位。
        assert_eq!(slot_to_unicode("cmex10", 0x70), Some(0x221A)); // √（\radical"270370）
        assert_eq!(slot_to_unicode("cmex10", 0x52), Some(0x222B)); // ∫（\intop="1352）
        assert_eq!(slot_to_unicode("cmex10", 0x51), Some(0x220F)); // ∏（\prod="1351）
                                                                   // 文本族仍走 OT1。
        assert_eq!(slot_to_unicode("cmr10", 0x7B), Some(0x2013)); // endash
                                                                  // 未知族回落 OT1，OMS 空洞返回 None。
        assert_eq!(slot_to_unicode("cmsy10", 0x50 ^ 0xFF), None);
        assert_eq!(slot_to_unicode("unknown10", b'A'), Some(0x41));
    }

    /// tests/data/latinmodern-math.otf：texlive lm-math 包（GUST Font
    /// License，可再分发）；入库供数学字形通道测试脱离环境依赖。
    const LM_MATH: &[u8] = include_bytes!("../tests/data/latinmodern-math.otf");

    /// 数学编码表全覆盖：凡映射出的 Unicode 码位必须命中
    /// latinmodern-math 的 cmap（防"表写了、字体没字形"的静默方框）。
    #[test]
    fn math_tables_all_hit_lm_math_cmap() {
        assert!(register_font_bytes("lm-math-cmap-test", LM_MATH));
        let mut cache = GlyphCache::new();
        let font = cache.resolve("lm-math-cmap-test").expect("注册表命中");
        let mut checked = 0;
        for slot in 0u8..=127 {
            for name in ["cmmi10", "cmsy10", "cmex10"] {
                if let Some(cp) = slot_to_unicode(name, slot) {
                    assert!(
                        font.glyph_id(cp).is_some(),
                        "{name} slot {slot:#x} → U+{cp:04X} 不在 latinmodern-math cmap"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 100, "映射应成规模（实测 {checked}）");
    }

    /// tests/data/lmroman10-regular.otf：texlive 2024 basic（GUST Font
    /// License，可再分发）；入库供字形层测试脱离环境依赖。
    const LM_ROMAN: &[u8] = include_bytes!("../tests/data/lmroman10-regular.otf");

    #[test]
    fn register_bytes_then_resolve_and_outline() {
        // 坏字节注册失败，不 panic。
        assert!(!register_font_bytes("bad-font", &[0u8; 16]));
        // 正常注册（用独立名避免污染其余测试的环境查找路径）。
        assert!(register_font_bytes("cmr-outline-test", LM_ROMAN));
        let mut cache = GlyphCache::new();
        let font = cache
            .resolve("cmr-outline-test")
            .expect("注册表命中（优先于文件系统）");

        // 'A'（U+0041）→ gid → 100px em 轮廓：约 2~3 个闭合子路径（外轮廓
        // + 三角孔），bbox 顶部在基线上方 ~0.65..0.75em。
        let gid = font.glyph_id(0x41).expect("cmap 有 'A'");
        let paths = font.outline_paths(gid, 100.0, 200.0, 150.0);
        assert!(!paths.is_empty(), "'A' 应有轮廓");
        assert!(paths.len() >= 2, "'A' 应含内孔（非零绕组用）");
        let top = paths
            .iter()
            .flat_map(|c| c.iter().map(|p| p[1]))
            .fold(f32::MAX, f32::min);
        // 基线 y=150，cap 高度 ≈ 0.7em → 顶 ≈ 80±10。
        assert!(
            (70.0..=95.0).contains(&top),
            "'A' 顶部 y={top} 应在基线上方 ~0.7em"
        );

        // 软光栅填充：'A' 干线内部有墨、基线下方（无 descender）保持白。
        let mut pm = crate::raster::Pixmap::new(400, 300);
        pm.fill(255, 255, 255);
        pm.fill_polygon(&paths, (0, 0, 0));
        let ink = (0..pm.height())
            .flat_map(|y| (0..pm.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| pm.pixel_nonwhite(x, y))
            .count();
        assert!(ink > 200, "'A'@100px 墨迹应显著（实测 {ink} px）");
        assert!(!pm.pixel_nonwhite(200, 155), "基线下方应无墨");
    }

    /// 发行字体目录认领（`register_font_dir` 的纯函数半边）：LM 改名映射
    /// （`lmroman10-regular.otf` → `cmr10`）与同名直取（中文 OTF）各一路。
    ///
    /// 用**临时目录**造场景：零全局注册表污染（并行测试互不干扰），且不依赖
    /// 仓库里 `ui/fonts/` 的实际内容。
    #[test]
    fn claim_font_dir_maps_lm_and_takes_same_name() {
        let dir = std::env::temp_dir().join(format!(
            "ntex-claim-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        // 内容只是"合法 OpenType 字节"的占位：本测试不解析，只查认领表。
        std::fs::write(dir.join("lmroman10-regular.otf"), LM_ROMAN).expect("写 LM 文件");
        std::fs::write(dir.join("FandolSong-Regular.otf"), LM_ROMAN).expect("写中文占位");
        // 非字体文件必须被忽略。
        std::fs::write(dir.join("README.md"), b"not a font").expect("写说明");

        let claimed = claim_font_dir(&dir);
        let names: Vec<&str> = claimed.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"cmr10"), "LM 改名映射：{names:?}");
        assert!(names.contains(&"FandolSong-Regular"), "同名直取：{names:?}");
        assert!(
            !names.contains(&"lmroman10-regular"),
            "已被映射族认领的文件不应再按文件名入表：{names:?}"
        );
        assert!(
            !names.contains(&"cmr12"),
            "目录里没有 lmroman12-regular.otf：{names:?}"
        );
        assert!(
            !names.iter().any(|n| n.ends_with("README")),
            "非 OpenType 文件不入表：{names:?}"
        );
        // 数学族共用 latinmodern-math.otf：本目录未放该文件 → 无 cmmi/cmsy/cmex。
        assert!(
            !names.contains(&"cmmi10"),
            "无 math 文件则无数学族：{names:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
