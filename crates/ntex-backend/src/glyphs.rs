//! 渲染侧字形层（M8 预览字形先行；完整字体子系统属 M9）。
//!
//! TFM 只有度量无轮廓（引擎布局事实源不变），本模块在**渲染侧**补字形：
//!
//! - TeX 字体名（cmr10 等）→ Latin Modern OpenType 文件（LM 与 CM 同源，
//!   度量一致；光学尺寸取名字中的设计字号）；
//! - OT1 编码 slot → Unicode（数据源：latex base `ot1enc.def` 的
//!   symbol/accent slot 声明 + cmr 常识位）→ skrifa cmap 查字形 id；
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

/// TeX 文本字体名 → Latin Modern OpenType 文件名（无映射的字体返回 None，
/// 调用方回落占位方框）。
///
/// 覆盖 cm 常用文本族；数学族（cmmi/cmsy/cmex）的 LM 文件本地常见精简
/// 安装不含，尝试映射、找不到文件同样回落。
fn lm_file_name(tex_name: &str) -> Option<String> {
    // 拆前缀（字母）+ 设计字号（数字，如 cmr10 → cmr + 10）。
    let split = tex_name
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(tex_name.len());
    let (family, size) = tex_name.split_at(split);
    let size = if size.is_empty() { "10" } else { size };
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
        "cmmi" => format!("lmmi{size}-regular"),
        "cmsy" => format!("lmsy{size}-regular"),
        "cmex" => "lmex10-regular".to_owned(),
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
    /// 解析 OTF 字节：skrifa 校验格式 + dump cmap（BMP 全量，~6.5 万次
    /// 二分查找，毫秒级一次性成本；数学字体扩展无需再改）。
    fn load(bytes: Vec<u8>) -> Option<Self> {
        let bytes: Arc<[u8]> = bytes.into();
        let font_ref = skrifa::FontRef::new(bytes.as_ref()).ok()?;
        let charmap = font_ref.charmap();
        let mut map = HashMap::new();
        for ch in 0u32..=0xFFFF {
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

    /// 按名字解析（命中缓存直接返回；注册表 → kpsewhich/texlive 均未命中
    /// 返回 None，字符走方框口径）。
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
}
