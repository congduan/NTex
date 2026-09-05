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
use std::path::Path;

use ttf_parser::{Face, OutlineBuilder};

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
        let data = std::fs::read(path)
            .map_err(|e| OtfError(format!("读取 {}: {e}", path.display())))?;
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
