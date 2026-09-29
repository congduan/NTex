//! OTF/TTF 静态解析层（M9 §11.0 阶段②前置：TTF/OTF 解析缺口第 1 项）。
//!
//! 基于 [`ttf_parser`]（纯 Rust、零 unsafe、零堆分配的只读解析器，
//! 与仓库 `unsafe_code = deny` 契约对齐；不引入 HarfBuzz/skrifa）。
//!
//! **与 [`crate::tfm::FontMetrics`] 的边界**：TFM 是 TeX 8-bit 度量语义
//! （charcode ≤ 255 → width/height/depth，单位 sp，含连字/字距程序），
//! 服务引擎布局事实源；本模块是 Unicode 语义（char → glyph id →
//! advance/lsb/bbox/outline，单位 = 字体设计单位），服务 M9 中文
//! 渲染链路。两者**不混用**：`FontMetrics` 不因本模块存在而改动。
//!
//! 定位：引擎侧字体正身（替代 `ntex-backend/src/glyphs.rs` 现行的
//! kpsewhich 运行时依赖）；本层只做静态解析，不做整形
//! （HarfBuzz 整形属 §11.0 阶段②后续条目）。
//!
//! 错误模型：加载路径走 `Result`；查询路径全 `Option`（码位/字形缺失
//! 是正常态，不是错误）——任何畸形输入（空文件/截断/非字体）返回
//! `Err`，不 panic（引擎契约：任意畸形输入不 panic）。

use std::fmt;
use std::path::{Path, PathBuf};

use ttf_parser::{Face, OutlineBuilder};

use crate::tfm::FontMetrics;

/// OTF/TTF 解析错误（操作意图上下文内嵌于消息）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtfError(pub String);

impl fmt::Display for OtfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OTF/TTF 解析失败：{}", self.0)
    }
}

impl std::error::Error for OtfError {}

type Result<T> = std::result::Result<T, OtfError>;

/// 已解析的 OTF/TTF 字体（持有所需表的堆快照；Face 本体零分配借用内部缓冲）。
///
/// 参照 `ntex-core` eqtb/InternTable 的堆归置：数据归 `OtfFont`，
/// [`OtfFont::face`] 每次按需轻量重建（Face 是表视图，无表数据拷贝）。
#[derive(Debug, Clone)]
pub struct OtfFont {
    /// 原始文件字节（Face 借用存活保障 + 零拷贝表快照底座）。
    data: Box<[u8]>,
}

/// 全局字体度量（head 提取；单位 = 字体设计单位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalMetrics {
    /// units_per_em（head.unitsPerEm；字体设计单位总数，Fandol 系 = 1000）。
    pub units_per_em: u16,
    /// 全局包围盒（head.xMin/…，设计单位）。
    pub bbox: Rect,
}

/// 轴对齐包围盒 / 矩形（设计单位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// xMin。
    pub x_min: i16,
    /// yMin。
    pub y_min: i16,
    /// xMax。
    pub x_max: i16,
    /// yMax。
    pub y_max: i16,
}

/// 单字形横向度量（hmtx；单位 = 字体设计单位，调用方按 upem/字号换算）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphMetrics {
    /// 前进宽度（hmtx advance）。
    pub advance_width: u16,
    /// 左侧 bearing（hmtx lsb；字形 bbox 左缘相对原点的水平偏移，可负）。
    pub left_side_bearing: i16,
}

/// 字形轮廓（闭折线段列表；设计单位）。
///
/// **表示取舍**：不桥接 ttf-parser 的 `OutlineBuilder` 回调到 vello 路径，
/// 而是二次贝塞尔控制点直接保留在 [`OutlinePoint::Quad`] 中，理由：
///
/// 1. 轮廓含真实曲线语义（TTF 二次 / CFF 三次转二次），后续 vello
///    路径与自研光栅化都吃二次贝塞尔——提前摊平成折线会在放大时
///    出现多边形化误差，且丢失可编辑性；
/// 2. OTF/CFF（Fandol 全系）轮廓是三次贝塞尔，ttf-parser 统一转二次
///    后统一表示，下游只需处理一种曲线类型；
/// 3. 定长数组 + `len`，无堆分配，`Outline` 本身 `Copy`。
///
/// 如需纯折线（如超轻量光栅化），可由下游对 `Quad` 做细分。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outline {
    /// 轮廓点序（`MoveTo`/`LineTo`/`QuadTo` 展平后的点列）。
    pub points: [OutlinePoint; MAX_OUTLINE_POINTS],
    /// 有效点数（≤ 64；超过截断——见 [`MAX_OUTLINE_POINTS`]）。
    pub len: u8,
}

/// 轮廓点上限（静态截断：保前缀，`len == MAX_OUTLINE_POINTS` 表示可能截断）。
///
/// 注：繁字（如『龘』U+9F98）控制点可超 64，此时渲染精度损失可接受：
/// 本层是 M9 前置的度量/正身层，逐字形保真渲染属 M9 后续
/// （`ntex-backend` vello 字形通道走完整缓存，不经本表示）。
pub const MAX_OUTLINE_POINTS: usize = 64;

/// OpenType 字体的默认设计字号 = 10pt（655360 sp）。
///
/// OTF/TTF 没有 TeX 意义上的"设计字号"（TFM 头部 header[1]）。取 10pt 为
/// 基准与 XeTeX 对未指定尺寸 OpenType 字体的处理一致；`at`/`scaled` 由
/// 调用方按 TFM 同款 [`FontMetrics::scaled_by`] 施加，使两条字体路径的
/// 缩放语义（`design_size_sp` 保持基准、`scale` = round(num/den × 2^20)）
/// 完全一致——DVI `fnt_def` 的 d/s 两字段因此无需分叉。
pub const DEFAULT_DESIGN_SP: i64 = 655360;

/// 全量字符度量快照（单位 = 字体设计单位；由 [`build_metrics`] 换算为 sp）。
///
/// [`OtfFont::metrics_snapshot`] 的产物：cmap 全量枚举（仅 Unicode 兼容
/// 子表）+ 逐字形 hmtx/bbox，`Face` 只解析一次。这是"逐字符查询"（每个
/// 字符一次 cmap 查表 + 一次 CFF charstring 解析）的批量化替代——引擎
/// 布局热路径按码位查表，不能在每次 `char_metrics` 里重解析字形轮廓。
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    /// head.unitsPerEm（advance/bbox 换算为 sp 的分母；0 表示头部非法）。
    pub units_per_em: u16,
    /// hhea ascender（设计单位；空字形 bbox 缺失时的纵向回落）。
    pub ascender: i16,
    /// hhea descender（设计单位，通常为负）。
    pub descender: i16,
    /// maxp.numGlyphs（诊断用）。
    pub number_of_glyphs: u16,
    /// `(Unicode 码位, hmtx 前进宽度, 墨迹包围盒)`，按码位升序去重。
    pub chars: Vec<(u32, u16, Rect)>,
}

/// 单个轮廓点（设计单位）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutlinePoint {
    /// 新子路径起点（每轮廓一个；ttf-parser `move_to`）。
    MoveTo { x: f32, y: f32 },
    /// 直线段终点（`line_to`）。
    LineTo { x: f32, y: f32 },
    /// 二次贝塞尔：隐含控制点 (x, y) + 终点接续下一点/闭合。
    /// （CFF 三次已由 ttf-parser 转二次；三次转换策略见 ttf-parser 文档。）
    QuadTo { x: f32, y: f32 },
    /// 子路径闭合（`close`；回到最近 `MoveTo`，不占点位）。
    Close,
}

impl OtfFont {
    /// 从文件加载 OTF/TTF 字体。
    ///
    /// 读全量字节 + `ttf_parser::Face::parse` 校验；提取 name/head
    /// 到 [`GlobalMetrics`] 与名字字段；表数据零拷贝借用 `data`。
    pub fn load(path: &Path) -> Result<OtfFont> {
        let data =
            std::fs::read(path).map_err(|e| OtfError(format!("读取 {}: {e}", path.display())))?;
        Self::from_data(data)
    }

    /// 从内存字节构建（测试与 VFS 通路复用；校验同 [`OtfFont::load`]）。
    pub fn from_data(data: Vec<u8>) -> Result<OtfFont> {
        Face::parse(&data, 0).map_err(|e| OtfError(format!("Face 解析：{e}")))?;
        Ok(OtfFont {
            data: data.into_boxed_slice(),
        })
    }

    /// 按需重建 Face 表视图（Face 无堆分配，重建即查表偏移，代价可忽略）。
    fn face(&self) -> Result<Face<'_>> {
        Face::parse(&self.data, 0).map_err(|e| OtfError(format!("Face 解析：{e}")))
    }

    /// 字体家族名（name 表 nameID=1；无则 None）。
    pub fn family_name(&self) -> Option<String> {
        self.find_name(ttf_parser::name_id::FAMILY)
    }

    /// 字体样式名（name 表 nameID=2；无则 None，如 "Regular"/"Bold"）。
    pub fn style_name(&self) -> Option<String> {
        self.find_name(ttf_parser::name_id::SUBFAMILY)
    }

    fn find_name(&self, name_id: u16) -> Option<String> {
        let names = self.face().ok()?.names();
        names
            .into_iter()
            .filter(|name| name.name_id == name_id)
            .find_map(|name| name.to_string())
    }

    /// 全局度量（head units_per_em + head 全局 bbox）。
    pub fn global_metrics(&self) -> Result<GlobalMetrics> {
        let face = self.face()?;
        let bbox = face.global_bounding_box();
        Ok(GlobalMetrics {
            units_per_em: face.units_per_em(),
            bbox: Rect {
                x_min: bbox.x_min,
                y_min: bbox.y_min,
                x_max: bbox.x_max,
                y_max: bbox.y_max,
            },
        })
    }

    /// Unicode 码位 → 字形 id（cmap format 4/12；未映射返回 None）。
    pub fn glyph_index(&self, ch: char) -> Option<u16> {
        Some(self.face().ok()?.glyph_index(ch)?.0)
    }

    /// 字形横向度量（hmtx advance + lsb；设计单位）。
    pub fn glyph_metrics(&self, gid: u16) -> Option<GlyphMetrics> {
        let hmtx = self.face().ok()?.tables().hmtx?;
        let advance_width = hmtx.advance(ttf_parser::GlyphId(gid))?;
        let left_side_bearing = hmtx.side_bearing(ttf_parser::GlyphId(gid)).unwrap_or(0);
        Some(GlyphMetrics {
            advance_width,
            left_side_bearing,
        })
    }

    /// 字形包围盒（`outline_glyph` 返回的 `Rect` 对 glyf/CFF 两种轮廓格式
    /// 统一给出；设计单位）。
    pub fn glyph_bbox(&self, gid: u16) -> Option<Rect> {
        let face = self.face().ok()?;
        let r = face.outline_glyph(ttf_parser::GlyphId(gid), &mut OutlineAcc::default())?;
        Some(Rect {
            x_min: r.x_min,
            y_min: r.y_min,
            x_max: r.x_max,
            y_max: r.y_max,
        })
    }

    /// 字形轮廓点集（二次贝塞尔；设计单位，坐标 f32）。
    pub fn glyph_outline(&self, gid: u16) -> Option<Outline> {
        let face = self.face().ok()?;
        let mut acc = OutlineAcc::default();
        face.outline_glyph(ttf_parser::GlyphId(gid), &mut acc);
        Some(acc.outline())
    }

    /// 全量字符度量快照（cmap 枚举 + 逐字形 hmtx/bbox）。
    ///
    /// 空字形（如 U+0020 空白、`.notdef`）的 bbox 可能为全零或缺失，此时
    /// 落 `ascender`/`descender` 作纵向度量——调用方据此决定纵向回落。
    ///
    /// 成本：`maxp.numGlyphs` 次 hmtx 读 +（CFF 字体）同样次数的 charstring
    /// 解析，中文字体（约 3 万字形）为一次性加载开销，不在排版热路径。
    pub fn metrics_snapshot(&self) -> Result<MetricsSnapshot> {
        let face = self.face()?;
        let units_per_em = face.units_per_em();
        if units_per_em == 0 {
            return Err(OtfError("head.unitsPerEm 为 0（字体头部非法）".into()));
        }
        let ascender = face.ascender();
        let descender = face.descender();
        let number_of_glyphs = face.number_of_glyphs();
        // 逐字形度量：glyf 字体直接读表级 bbox，CFF 字体解析 charstring。
        let mut per_glyph: Vec<(u16, Rect)> = Vec::with_capacity(number_of_glyphs as usize);
        for i in 0..number_of_glyphs {
            let gid = ttf_parser::GlyphId(i);
            let advance = face.glyph_hor_advance(gid).unwrap_or(0);
            let bbox = face
                .glyph_bounding_box(gid)
                .map(from_ttf_rect)
                .unwrap_or(Rect {
                    x_min: 0,
                    y_min: descender,
                    x_max: 0,
                    y_max: ascender,
                });
            per_glyph.push((advance, bbox));
        }
        // cmap 枚举（仅 Unicode 兼容子表；多子表重复码位在排序后去重）
        let mut chars: Vec<(u32, u16, Rect)> = Vec::new();
        if let Some(cmap) = face.tables().cmap {
            for st in cmap.subtables {
                if !st.is_unicode() {
                    continue;
                }
                st.codepoints(|cp| {
                    if let Some(gid) = st.glyph_index(cp) {
                        if let Some(&(advance, bbox)) = per_glyph.get(gid.0 as usize) {
                            chars.push((cp, advance, bbox));
                        }
                    }
                });
            }
        }
        chars.sort_by_key(|c| c.0);
        chars.dedup_by_key(|c| c.0);
        Ok(MetricsSnapshot {
            units_per_em,
            ascender,
            descender,
            number_of_glyphs,
            chars,
        })
    }
}

/// 从 OTF/TTF 字节构建引擎侧字体度量（`unicode_native = true`，设计字号
/// [`DEFAULT_DESIGN_SP`]）。
///
/// 产出物可直接交给 [`FontMetrics::scaled_by`] 施加 `at`/`scaled`——与
/// TFM 路径共用同一套缩放代码，两条路径的 DVI `fnt_def` 语义因此一致。
///
/// 与 TFM 的字段取舍：
/// - `chars` / `char_italic` / `lig_kern_*` / `next_larger` 保持空——8-bit
///   槽表概念对 Unicode 字体无意义（连字/字距属 HarfBuzz 整形，见 §11.0）；
/// - `space` 取 U+0020 前进宽度，stretch/shrink 按 TeX 文本字体惯例
///   1/2、1/3；缺失时退 1/4 em；
/// - `x_height` 取 'x' 墨迹上缘；`quad` = 1 em；`extra_space` = 1/18 em；
/// - `checksum` 置 0（OTF 无 TFM 校验和；DVI 按字体名区分）。
pub fn build_metrics(data: Vec<u8>, name: &str) -> Result<FontMetrics> {
    let font = OtfFont::from_data(data)?;
    let snap = font.metrics_snapshot()?;
    let design = DEFAULT_DESIGN_SP;
    let upem = i128::from(snap.units_per_em);
    let ascender = i64::from(snap.ascender);
    let descender = i64::from(snap.descender);

    // 设计单位 → sp（round-away-from-zero，与 tfm::parse_tfm 的取整口径一致）
    let to_sp = |v: i64| -> i64 {
        let n = v as i128 * design as i128;
        if n >= 0 {
            ((n + upem / 2) / upem) as i64
        } else {
            -(((-n) + upem / 2) / upem) as i64
        }
    };
    let fallback_h = to_sp(ascender);
    let fallback_d = to_sp(-descender);

    let mut unicode_chars: Vec<(u32, (i64, i64, i64))> = Vec::with_capacity(snap.chars.len());
    for &(cp, advance, bbox) in &snap.chars {
        let w = to_sp(i64::from(advance));
        // 纵向优先取字形墨迹；全零 bbox（空格等空字形）回落字体全局纵横
        let (h, d) = if bbox.y_max == 0 && bbox.y_min == 0 {
            (fallback_h, fallback_d)
        } else {
            (
                to_sp(i64::from(bbox.y_max).max(0)),
                to_sp((-i64::from(bbox.y_min)).max(0)),
            )
        };
        unicode_chars.push((cp, (w, h, d)));
    }

    // unicode_chars 已按码位升序（metrics_snapshot 保证），可直接二分
    let char_w = |cp: u32| -> Option<i64> {
        unicode_chars
            .binary_search_by_key(&cp, |(k, _)| *k)
            .ok()
            .map(|i| unicode_chars[i].1 .0)
    };
    let space = char_w(0x20).filter(|&w| w > 0).unwrap_or(design / 4);
    let hyphenchar = if char_w(0x2d).is_some() { 45 } else { -1 };
    let space_stretch = space / 2;
    let space_shrink = space / 3;
    let x_height = snap
        .chars
        .binary_search_by_key(&0x78, |c| c.0) // 'x'
        .ok()
        .map(|i| to_sp(i64::from(snap.chars[i].2.y_max).max(0)))
        .filter(|&v| v > 0)
        .unwrap_or(design / 2);
    let extra_space = design / 18;

    Ok(FontMetrics {
        design_size_sp: design,
        scale: 1 << 20,
        checksum: 0,
        name: name.to_owned(),
        chars: Vec::new(),
        char_italic: Vec::new(),
        slant: 0,
        space,
        space_stretch,
        space_shrink,
        x_height,
        quad: design,
        extra_space,
        lig_kern_steps: Vec::new(),
        kern_values: Vec::new(),
        lig_kern_index: Vec::new(),
        next_larger: Vec::new(),
        // 1-based TFM 参数序：1=slant 2=space 3=stretch 4=shrink 5=x_height 6=quad
        font_params: vec![
            0,
            space,
            space_stretch,
            space_shrink,
            x_height,
            design,
            extra_space,
        ],
        hyphenchar,
        unicode_native: true,
        unicode_chars,
    })
}

/// `ttf_parser::Rect` → 本模块 [`Rect`]。
///
/// 两个 `Rect` 字段同名同类型，仅分属不同 crate（ttf-parser 不实现 From），
/// 故手写逐字段搬运。
fn from_ttf_rect(r: ttf_parser::Rect) -> Rect {
    Rect {
        x_min: r.x_min,
        y_min: r.y_min,
        x_max: r.x_max,
        y_max: r.y_max,
    }
}

/// 字体名是否已带字体文件后缀（OTF/TTF 及集合 TTC）。
fn has_font_ext(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".otf") || lower.ends_with(".ttf") || lower.ends_with(".ttc")
}

/// 查找 OTF/TTF/TTC 字体文件（M9 中文刀 1；与 [`crate::tfm::find_tfm`] 同款约定）。
///
/// 搜索链：`NTEX_OTF_DIR` → `~/.ntex-fonts`（仓库既有测试字体缓存约定）→
/// 系统字体目录（macOS `/System/Library/Fonts` 等、Linux `/usr/share/fonts`
/// 含一层子目录）→ `kpsewhich`。`name` 未带后缀时依次试 `.otf` / `.ttf`。
///
/// 找不到返回 `None`——加载失败由调用方报"找不到字体"，不是错误传播点。
pub fn find_otf(name: &str) -> Option<PathBuf> {
    let candidates: Vec<String> = if has_font_ext(name) {
        vec![name.to_owned()]
    } else {
        vec![format!("{name}.otf"), format!("{name}.ttf")]
    };

    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(d) = std::env::var("NTEX_OTF_DIR") {
        dirs.push(PathBuf::from(d));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".ntex-fonts"));
        dirs.push(home.join("Library/Fonts"));
    }
    for d in [
        "/System/Library/Fonts",
        "/System/Library/Fonts/Supplemental",
        "/Library/Fonts",
        "/usr/share/fonts",
        "/usr/local/share/fonts",
    ] {
        dirs.push(PathBuf::from(d));
    }
    // texlive opentype 安装目录（按年份取最新）
    if let Ok(entries) = std::fs::read_dir("/usr/local/texlive") {
        let mut years: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        years.sort();
        for y in years.into_iter().rev() {
            dirs.push(y.join("texmf-dist/fonts/opentype"));
        }
    }

    for dir in &dirs {
        for c in &candidates {
            let p = dir.join(c);
            if p.is_file() {
                return Some(p);
            }
        }
        // 一层子目录（/usr/share/fonts/{truetype,opentype,ttf-*}/…）
        if let Ok(entries) = std::fs::read_dir(dir) {
            for sub in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
                for c in &candidates {
                    let p = sub.join(c);
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    }

    // 最后回落 kpsewhich（PATH 上存在时）
    for c in &candidates {
        if let Ok(out) = std::process::Command::new("kpsewhich").arg(c).output() {
            if out.status.success() {
                let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
            }
        }
    }
    None
}

/// 轮廓累积器：同时服务 `glyph_outline`（点集）与 CFF 路径的 bbox 计算
/// （glyf 有表级 bbox 直读；CFF charstring 只能走轮廓回调）。
#[derive(Default)]
struct OutlineAcc {
    points: Vec<OutlinePoint>,
    start: (f32, f32),
    x_min: f32,
    y_min: f32,
    x_max: f32,
    y_max: f32,
    seen_pt: bool,
}

impl OutlineAcc {
    fn outline(self) -> Outline {
        let mut points = [OutlinePoint::Close; MAX_OUTLINE_POINTS];
        let len = self.points.len().min(MAX_OUTLINE_POINTS);
        points[..len].copy_from_slice(&self.points[..len]);
        Outline {
            points,
            len: len as u8,
        }
    }

    fn track(&mut self, x: f32, y: f32) {
        if !self.seen_pt {
            self.x_min = x;
            self.y_min = y;
            self.x_max = x;
            self.y_max = y;
            self.seen_pt = true;
            return;
        }
        self.x_min = self.x_min.min(x);
        self.y_min = self.y_min.min(y);
        self.x_max = self.x_max.max(x);
        self.y_max = self.y_max.max(y);
    }

    fn push(&mut self, point: OutlinePoint) {
        if self.points.len() < MAX_OUTLINE_POINTS {
            self.points.push(point);
        }
    }
}

impl OutlineBuilder for OutlineAcc {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = (x, y);
        self.track(x, y);
        self.push(OutlinePoint::MoveTo { x, y });
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.track(x, y);
        self.push(OutlinePoint::LineTo { x, y });
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.track(x1, y1);
        self.track(x, y);
        self.push(OutlinePoint::QuadTo { x, y });
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.track(x1, y1);
        self.track(x2, y2);
        self.track(x, y);
        self.push(OutlinePoint::QuadTo { x, y });
    }

    fn close(&mut self) {
        self.push(OutlinePoint::Close);
    }
}

#[cfg(test)]
mod tests;
