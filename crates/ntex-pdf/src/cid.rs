//! CID 映射：Unicode 码位 → 嵌入字体 CFF charset 里的**真 CID**（M9 中文 PDF 修复）。
//!
//! ## 为什么需要本层
//!
//! `/Encoding /Identity-H` 的内容流里，两字节串是 **CID 原值**，查看器再按嵌入
//! 字体自己的映射把 CID 换成字形（CID-keyed CFF 用 CFF 的 charset；CIDFontType2
//! 用 `/CIDToGIDMap`）。因此内容流必须写**字体认得的那种 CID**，而不是 Unicode：
//!
//! - FandolSong 等中文 OTF 是 **CID-keyed CFF**，ROS = `Adobe-GB1-5`，
//!   charset 给的是 **Adobe-GB1 的 CID**（`中` → GID 1497 → CID 4559 `0x11CF`，
//!   而它的 Unicode 是 `0x4E2D`）。写 Unicode 进去 → 查看器按 CID 4559 反查失败
//!   → 整页中文空白（2026-09-13 修复前的现场）。
//! - 对照定标：xdvipdfmx 对同一字体写出的内容流是 `[<11cf0ed30b8603f104a90d6b>…]`，
//!   即 GB1 CID；其内嵌 CFF 子集的 charset 正是 `GID→CID = [… 4559]`。
//!
//! 映射链：**Unicode →（字体 cmap）→ GID →（CFF charset）→ CID**。
//! cmap 与 charset 都在字体里，本期不引入新依赖（不解析 CFF charstring）。
//!
//! ## 形态判定
//!
//! - CFF 带 ROS（`12 30`）→ CID-keyed：[`Mapping::CharsetCid`]，走 charset 查表，**正路**；
//! - CFF 无 ROS（name-keyed）→ charset 存的是 SID（字形名）而非 CID，PDF 侧
//!   无法据此构造映射。退化为 [`Mapping::GlyphIdFallback`]（CID = GID，部分
//!   查看器按 identity 口径能显示）并**如实告警**——挂账（见 `docs/` 说明）。
//!
//! 错误模型：任何畸形字节（截断/越界/非法结构）返回 `Err`，不 panic
//! （引擎契约：任意畸形输入不 panic）；查不到某个码位是正常态，返回 `None`。

use std::fmt;

/// CFF 顶层 DICT 的 CIDSystemInfo（ROS：Registry-Ordering-Supplement）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ros {
    /// 注册机构（如 `Adobe`）。
    pub registry: String,
    /// 字符集（如 `GB1`、`Japan1`、`Identity`）。
    pub ordering: String,
    /// 补遗号（如 `5`）。
    pub supplement: u16,
}

/// CID 语义来源（决定 `cid()` 的可信度）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mapping {
    /// CFF 自带 ROS：charset 的条目就是 CID（正路）。
    CharsetCid,
    /// 非 CID-keyed CFF：退化为 CID = GID（兼容口径，挂账）。
    GlyphIdFallback,
}

/// 字体 CID 映射（按码位升序表，二分查询）。
#[derive(Debug, Clone)]
pub struct CidMap {
    /// `(Unicode 码位, CID)`，码位升序且唯一。
    entries: Vec<(u32, u16)>,
    /// CFF 的 ROS（name-keyed 字体为 `None`）。
    ros: Option<Ros>,
    /// 语义来源。
    mapping: Mapping,
}

impl CidMap {
    /// 查码位对应的 CID（字体未覆盖该码位 → `None`）。
    pub fn cid(&self, cp: u32) -> Option<u16> {
        self.entries
            .binary_search_by_key(&cp, |(c, _)| *c)
            .ok()
            .map(|i| self.entries[i].1)
    }

    /// 字体声明的 ROS（供 `/CIDSystemInfo` 如实照写）。
    pub fn ros(&self) -> Option<&Ros> {
        self.ros.as_ref()
    }

    /// 语义来源。
    pub fn mapping(&self) -> Mapping {
        self.mapping
    }

    /// 覆盖的码位数（诊断/测试用）。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空表（无任何可用映射）。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// CID 映射构建错误（消息内嵌操作意图）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CidError(pub String);

impl fmt::Display for CidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CID 映射构建失败：{}", self.0)
    }
}

impl std::error::Error for CidError {}

/// 从 sfnt（OTTO）容器抽出裸 CFF 表。
///
/// 用于 PDF 嵌入：CIDFontType0 的 `/FontFile3` 应为 **裸 CID-keyed CFF**
/// （`/Subtype /CIDFontType0C`，dvipdfmx 同款做法）。实验证实：整包 OTTO
/// 以 `/Subtype /OpenType` 嵌入时，poppler/CoreGraphics 对同一 CID 解析出
/// 错误字形（内容流 CID 与 xdvipdfmx 逐字节一致仍错），剥壳后即正常。
pub fn bare_cff(bytes: &[u8]) -> Result<Vec<u8>, CidError> {
    let tables = sfnt_tables(bytes).ok_or_else(|| CidError("非 sfnt 字节（无表目录）".into()))?;
    let (cff_off, cff_len) = tables
        .iter()
        .find(|(tag, _)| *tag == *b"CFF ")
        .ok_or_else(|| CidError("无 CFF 表（TrueType/glyf 轮廓）".into()))?
        .1;
    Ok(slice(bytes, cff_off, cff_len)
        .ok_or_else(|| CidError("CFF 表越界".into()))?
        .to_vec())
}

/// 从 sfnt（OTTO/CFF）字节构建 CID 映射。
pub fn build(bytes: &[u8]) -> Result<CidMap, CidError> {
    let tables = sfnt_tables(bytes).ok_or_else(|| CidError("非 sfnt 字节（无表目录）".into()))?;
    let (cff_off, cff_len) = tables
        .iter()
        .find(|(tag, _)| *tag == *b"CFF ")
        .ok_or_else(|| CidError("无 CFF 表（TrueType/glyf 轮廓）".into()))?
        .1;
    let cff = slice(bytes, cff_off, cff_len).ok_or_else(|| CidError("CFF 表越界".into()))?;
    let cff = parse_cff(cff)?;

    let (cmap_off, _) = tables
        .iter()
        .find(|(tag, _)| *tag == *b"cmap")
        .ok_or_else(|| CidError("无 cmap 表".into()))?
        .1;
    let pairs = parse_cmap(bytes, cmap_off)?;

    let mapping = if cff.ros.is_some() {
        Mapping::CharsetCid
    } else {
        Mapping::GlyphIdFallback
    };
    let mut entries: Vec<(u32, u16)> = Vec::with_capacity(pairs.len());
    for (cp, gid) in pairs {
        let cid = match mapping {
            // 正路：charset[gid]（GID 0 = .notdef，其 CID 恒 0，不参与映射）
            Mapping::CharsetCid => match cff.charset.get(gid as usize) {
                Some(&c) => c,
                None => continue, // 越界 GID：字体自相矛盾，跳过不猜
            },
            // 退化：CID = GID（name-keyed 字体的兼容口径）
            Mapping::GlyphIdFallback => gid,
        };
        if cid != 0 {
            entries.push((cp, cid));
        }
    }
    entries.sort_unstable_by_key(|(cp, _)| *cp);
    entries.dedup_by_key(|(cp, _)| *cp);

    Ok(CidMap {
        entries,
        ros: cff.ros,
        mapping,
    })
}

// ---------- sfnt / CFF / cmap 解析（只读、全越界检查） ----------

/// 取 `bytes[off..off+len]`，越界返回 `None`。
fn slice(b: &[u8], off: usize, len: usize) -> Option<&[u8]> {
    b.get(off..off.checked_add(len)?)
}

/// 大端 u16（越界 → `None`）。
fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    let s = b.get(off..off + 2)?;
    Some(u16::from_be_bytes([s[0], s[1]]))
}

/// 大端 u32（越界 → `None`）。
fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    let s = b.get(off..off + 4)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// sfnt 表目录条目：`(tag, (offset, length))`。
type SfntTable = ([u8; 4], (usize, usize));

/// sfnt 表目录；魔数不符返回 `None`。
fn sfnt_tables(b: &[u8]) -> Option<Vec<SfntTable>> {
    let magic = b.get(0..4)?;
    if magic != b"OTTO" && magic != [0x00, 0x01, 0x00, 0x00] && magic != b"true" {
        return None;
    }
    let num = u16_at(b, 4)? as usize;
    let mut tabs = Vec::with_capacity(num);
    for i in 0..num {
        let p = 12usize.checked_add(i.checked_mul(16)?)?;
        let tag: [u8; 4] = b.get(p..p + 4)?.try_into().ok()?;
        let off = u32_at(b, p + 8)? as usize;
        let len = u32_at(b, p + 12)? as usize;
        tabs.push((tag, (off, len)));
    }
    Some(tabs)
}

/// CFF 解析产物（只取本项目需要的部分）。
struct Cff {
    /// GID → CID/SID（下标即 GID；`[0]` 为 .notdef）。
    charset: Vec<u16>,
    /// ROS（name-keyed 为 `None`）。
    ros: Option<Ros>,
}

/// 解析 CFF 顶层结构：header / Name INDEX / TopDICT / String INDEX / charset。
fn parse_cff(c: &[u8]) -> Result<Cff, CidError> {
    let hdr_size = *c.get(2).ok_or_else(|| CidError("CFF 头部截断".into()))? as usize;
    // Name INDEX（跳过）、Top DICT INDEX（取首个）、String INDEX、Global Subr INDEX
    let (_names, p) =
        read_index(c, hdr_size).ok_or_else(|| CidError("CFF Name INDEX 损坏".into()))?;
    let (top, p) = read_index(c, p).ok_or_else(|| CidError("CFF TopDICT INDEX 损坏".into()))?;
    let (strings, _) = read_index(c, p).ok_or_else(|| CidError("CFF String INDEX 损坏".into()))?;
    let top = top
        .first()
        .ok_or_else(|| CidError("CFF 无 TopDICT".into()))?;

    let ops = parse_dict(top);
    let ros = match ops.get(&(30, true)) {
        Some(v) if v.len() >= 3 => {
            // 字符串 SID ≥ 391 才落在 String INDEX（0..=390 是标准字形名表）
            let name = |sid: i32| -> Option<String> {
                let idx = usize::try_from(sid.checked_sub(391)?).ok()?;
                strings
                    .get(idx)
                    .and_then(|b| std::str::from_utf8(b).ok().map(str::to_owned))
            };
            match (name(v[0]), name(v[1])) {
                (Some(registry), Some(ordering)) => Some(Ros {
                    registry,
                    ordering,
                    supplement: v[2].clamp(0, i32::from(u16::MAX)) as u16,
                }),
                // ROS 的字符串解析不出来（用了标准 SID，正常字体不会发生）：按无 ROS 处理
                _ => None,
            }
        }
        _ => None,
    };

    // charset 偏移：0/1/2 是预定义 charset（ISOAdobe/Expert/ExpertSubset），
    // 对 CID 语义等同 identity（CID = GID）
    let charset_off = match ops.get(&(15, false)) {
        Some(v) if !v.is_empty() => *v.last().unwrap_or(&0),
        _ => 0,
    };
    let charstrings_off = match ops.get(&(17, false)) {
        Some(v) if !v.is_empty() => *v.last().unwrap_or(&0),
        _ => return Err(CidError("TopDICT 缺 CharStrings".into())),
    };
    let charstrings_off =
        usize::try_from(charstrings_off).map_err(|_| CidError("CharStrings 偏移为负".into()))?;
    let glyph_count = usize::from(
        u16_at(c, charstrings_off).ok_or_else(|| CidError("CharStrings INDEX 越界".into()))?,
    );

    let charset = if charset_off <= 2 {
        (0..glyph_count).map(|g| g as u16).collect()
    } else {
        let off = usize::try_from(charset_off).map_err(|_| CidError("charset 偏移为负".into()))?;
        parse_charset(c, off, glyph_count)?
    };

    Ok(Cff { charset, ros })
}

/// 解析 CFF charset（format 0/1/2）→ GID → CID/SID 全表（长度 = 字形数）。
fn parse_charset(c: &[u8], off: usize, glyph_count: usize) -> Result<Vec<u16>, CidError> {
    let mut out = vec![0u16; glyph_count];
    let fmt = *c.get(off).ok_or_else(|| CidError("charset 越界".into()))?;
    match fmt {
        // format 0：GID 1.. 起逐个 u16
        0 => {
            let gids = glyph_count.saturating_sub(1);
            let body = slice(
                c,
                off + 1,
                gids.checked_mul(2)
                    .ok_or_else(|| CidError("charset 长度溢出".into()))?,
            )
            .ok_or_else(|| CidError("charset format 0 截断".into()))?;
            for g in 1..glyph_count {
                out[g] = u16::from_be_bytes([body[2 * (g - 1)], body[2 * (g - 1) + 1]]);
            }
        }
        // format 1/2：`first u8/u16 nLeft` 的连续段（每段覆盖 nLeft + 1 个 GID）
        1 | 2 => {
            let n_left_len = if fmt == 1 { 1 } else { 2 };
            let mut p = off + 1;
            let mut gid = 1usize;
            while gid < glyph_count {
                let first = u16_at(c, p).ok_or_else(|| CidError("charset 段越界".into()))?;
                let n_left = if fmt == 1 {
                    usize::from(
                        *c.get(p + 2)
                            .ok_or_else(|| CidError("charset 段截断".into()))?,
                    )
                } else {
                    usize::from(u16_at(c, p + 2).ok_or_else(|| CidError("charset 段截断".into()))?)
                };
                for k in 0..=n_left {
                    if gid >= glyph_count {
                        break;
                    }
                    out[gid] = first.wrapping_add(k as u16);
                    gid += 1;
                }
                p = p
                    .checked_add(2 + n_left_len)
                    .ok_or_else(|| CidError("charset 段落位置溢出".into()))?;
            }
        }
        other => return Err(CidError(format!("charset 格式 {other} 未知"))),
    }
    Ok(out)
}

/// 读 CFF INDEX：返回（条目列表, INDEX 结束位置）。count = 0 时结束位置 = pos + 2。
fn read_index(b: &[u8], pos: usize) -> Option<(Vec<&[u8]>, usize)> {
    let count = usize::from(u16_at(b, pos)?);
    if count == 0 {
        return Some((Vec::new(), pos.checked_add(2)?));
    }
    let off_size = usize::from(*b.get(pos + 2)?);
    if !(1..=4).contains(&off_size) {
        return None;
    }
    let offs_start = pos.checked_add(3)?;
    let mut offsets = Vec::with_capacity(count + 1);
    for i in 0..=count {
        let p = offs_start.checked_add(i.checked_mul(off_size)?)?;
        let raw = b.get(p..p + off_size)?;
        let mut v: usize = 0;
        for &byte in raw {
            v = (v << 8) | usize::from(byte);
        }
        offsets.push(v);
    }
    let data_start = offs_start.checked_add((count + 1).checked_mul(off_size)?)?;
    let base = offsets.first().copied()?;
    let end_rel = offsets.last().copied()?;
    if base == 0 || end_rel < base {
        return None; // CFF 要求偏移从 1 起且单调
    }
    let mut items = Vec::with_capacity(count);
    for i in 0..count {
        let s = data_start.checked_add(offsets[i] - 1)?;
        let e = data_start.checked_add(offsets[i + 1] - 1)?;
        items.push(b.get(s..e)?);
    }
    Some((items, data_start.checked_add(end_rel - 1)?))
}

/// CFF DICT 操作数 → `{(操作符, 是否转义) → 操作数栈}`（本项目只取偏移与 ROS）。
fn parse_dict(d: &[u8]) -> std::collections::HashMap<(u8, bool), Vec<i32>> {
    let mut ops: std::collections::HashMap<(u8, bool), Vec<i32>> = std::collections::HashMap::new();
    let mut stack: Vec<i32> = Vec::new();
    let mut i = 0usize;
    while i < d.len() {
        let b0 = d[i];
        match b0 {
            0..=21 => {
                if b0 == 12 {
                    match d.get(i + 1) {
                        Some(&b1) => {
                            ops.insert((b1, true), std::mem::take(&mut stack));
                            i += 2;
                        }
                        None => break,
                    }
                } else {
                    ops.insert((b0, false), std::mem::take(&mut stack));
                    i += 1;
                }
            }
            28 => {
                match (d.get(i + 1), d.get(i + 2)) {
                    (Some(&a), Some(&b)) => stack.push(i16::from_be_bytes([a, b]) as i32),
                    _ => break,
                }
                i += 3;
            }
            29 => {
                match d.get(i + 1..i + 5) {
                    Some(s) if s.len() == 4 => {
                        stack.push(i32::from_be_bytes([s[0], s[1], s[2], s[3]]))
                    }
                    _ => break,
                }
                i += 5;
            }
            30 => {
                // 实数（BCD）：本项目不消费实数操作数，跳过其字节
                i += 1;
                while i < d.len() {
                    let v = d[i];
                    i += 1;
                    if v >> 4 == 0xF || v & 0xF == 0xF {
                        break;
                    }
                }
                stack.push(0);
            }
            32..=246 => {
                stack.push(i32::from(b0) - 139);
                i += 1;
            }
            247..=250 => {
                match d.get(i + 1) {
                    Some(&b1) => stack.push((i32::from(b0) - 247) * 256 + i32::from(b1) + 108),
                    None => break,
                }
                i += 2;
            }
            251..=254 => {
                match d.get(i + 1) {
                    Some(&b1) => stack.push(-(i32::from(b0) - 251) * 256 - i32::from(b1) - 108),
                    None => break,
                }
                i += 2;
            }
            // 22..=27 / 31 / 255：保留字节，跳过
            _ => i += 1,
        }
    }
    ops
}

/// 解析 cmap → `(Unicode 码位, GID)` 列表（优先 format 12，其次 format 4）。
fn parse_cmap(b: &[u8], off: usize) -> Result<Vec<(u32, u16)>, CidError> {
    let n = usize::from(u16_at(b, off + 2).ok_or_else(|| CidError("cmap 头部截断".into()))?);
    // 选子表：优先 (3,10)/(0,4) format 12，其次 (3,1)/(0,3) format 4
    let mut best: Option<(u8, usize)> = None;
    for i in 0..n {
        let p = off
            .checked_add(4 + 8 * i)
            .ok_or_else(|| CidError("cmap 子表越界".into()))?;
        let pid = u16_at(b, p).ok_or_else(|| CidError("cmap 子表截断".into()))?;
        let eid = u16_at(b, p + 2).ok_or_else(|| CidError("cmap 子表截断".into()))?;
        let sub_off = off
            .checked_add(u32_at(b, p + 4).ok_or_else(|| CidError("cmap 子表截断".into()))? as usize)
            .ok_or_else(|| CidError("cmap 子表偏移溢出".into()))?;
        let is_unicode = pid == 0 || (pid == 3 && matches!(eid, 1 | 10));
        if !is_unicode {
            continue;
        }
        let fmt = u16_at(b, sub_off).ok_or_else(|| CidError("cmap 子表格式截断".into()))?;
        let priority: u8 = match (fmt, eid) {
            (12, 10) => 3,
            (12, _) => 2,
            (4, 1) => 1,
            (4, _) => 0,
            _ => continue,
        };
        if best.map_or(true, |(bp, _)| priority > bp) {
            best = Some((priority, sub_off));
        }
    }
    let (_, sub_off) = best.ok_or_else(|| CidError("cmap 无可用 Unicode 子表".into()))?;
    let fmt = u16_at(b, sub_off).ok_or_else(|| CidError("cmap 子表格式截断".into()))?;
    match fmt {
        4 => Ok(cmap_format4(b, sub_off)?),
        12 => Ok(cmap_format12(b, sub_off)?),
        other => Err(CidError(format!("cmap 格式 {other} 暂不支持"))),
    }
}

/// cmap 枚举预算：单字体最多枚举这么多码位。真实 CJK 字体（Noto CJK ~4.5 万
/// 条目）远在其下；上限只为把畸形 cmap（超大/重叠段）的遍历代价钉死，避免
/// 恶意字体造成长时间占用（引擎契约：任意畸形输入不 panic，也不失控）。
const CMAP_ENUM_BUDGET: usize = 1 << 21;

/// cmap format 4（BMP，分段映射）。
fn cmap_format4(b: &[u8], off: usize) -> Result<Vec<(u32, u16)>, CidError> {
    let seg_x2 = usize::from(u16_at(b, off + 6).ok_or_else(|| CidError("cmap4 头部截断".into()))?);
    let seg_count = seg_x2 / 2;
    let ends = off + 14;
    let starts = ends + seg_x2 + 2; // 跳过 endCode[] 与 reservedPad
    let deltas = starts + seg_x2;
    let range_offsets = deltas + seg_x2;
    let mut out = Vec::new();
    let mut budget = CMAP_ENUM_BUDGET;
    for i in 0..seg_count {
        let end = u16_at(b, ends + 2 * i).ok_or_else(|| CidError("cmap4 endCode 截断".into()))?;
        let start =
            u16_at(b, starts + 2 * i).ok_or_else(|| CidError("cmap4 startCode 截断".into()))?;
        let delta =
            u16_at(b, deltas + 2 * i).ok_or_else(|| CidError("cmap4 idDelta 截断".into()))?;
        let range_off = u16_at(b, range_offsets + 2 * i)
            .ok_or_else(|| CidError("cmap4 idRangeOffset 截断".into()))?;
        if start > end {
            continue;
        }
        // 0xFFFF 是格式 4 的段终止哨兵，不是真字符
        for cp in u32::from(start)..=u32::from(end) {
            if cp == 0xFFFF {
                continue;
            }
            if budget == 0 {
                return Ok(out);
            }
            budget -= 1;
            let gid = if range_off == 0 {
                (cp as u16).wrapping_add(delta)
            } else {
                // 公式出自 OT 规范：位置以 idRangeOffset[i] 自身地址为基准
                let loc = range_offsets
                    + 2 * i
                    + usize::from(range_off)
                    + 2 * (cp - u32::from(start)) as usize;
                match u16_at(b, loc) {
                    Some(0) | None => continue,
                    Some(g) => g.wrapping_add(delta),
                }
            };
            if gid != 0 {
                out.push((cp, gid));
            }
        }
    }
    Ok(out)
}

/// cmap format 12（全平面，分段映射）。
fn cmap_format12(b: &[u8], off: usize) -> Result<Vec<(u32, u16)>, CidError> {
    let n_groups = u32_at(b, off + 12).ok_or_else(|| CidError("cmap12 头部截断".into()))? as usize;
    let mut out = Vec::new();
    let mut budget = CMAP_ENUM_BUDGET;
    for i in 0..n_groups {
        let p = match off.checked_add(16 + 12 * i) {
            Some(p) => p,
            None => break,
        };
        let (start, end, start_gid) = match (u32_at(b, p), u32_at(b, p + 4), u32_at(b, p + 8)) {
            (Some(a), Some(c), Some(d)) => (a, c, d),
            _ => break, // 畸形/截断：取已得部分（不猜）
        };
        if start > end {
            continue;
        }
        if start > 0x10FFFF {
            break; // 组按码位升序，后面的更没有意义
        }
        let end = end.min(0x10FFFF);
        for cp in start..=end {
            if budget == 0 {
                return Ok(out);
            }
            budget -= 1;
            let gid = start_gid.wrapping_add(cp - start);
            // GID 超 u16（巨型字体）或 0（缺字形）都跳过
            if gid != 0 && gid <= u32::from(u16::MAX) {
                out.push((cp, gid as u16));
            }
        }
    }
    Ok(out)
}
