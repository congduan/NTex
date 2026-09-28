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

// ---------- CFF 子集化（运行时按用字裁剪，xdvipdfmx 同款） ----------

/// CFF 子集化：只保留 GID 0（`.notdef`）与 `used`（内容流实际写出的 CID 值，
/// 语义由 `mapping` 约定）对应的字形，重写 charset / CharStrings / FDArray /
/// FDSelect / Private 等结构；字形轮廓与子程序字节**原样搬运**（不重编码
/// Type 2，全局/局部子程序整体保留），任何畸形输入只需 `Err`、不会 panic。
///
/// ## 为什么内容流的 CID 不用重排
///
/// 子集的 charset 直接写**原 CID**（新 GID → 原 CID 映射表），查看器按 CID
/// 反查字形与全量字体完全一致——内容流、`/W`、[`CidMap`] 三者零改动。这是
/// 体积收益的全部来源（FandolSong 全量 CFF 4.8 MB，其中 CharStrings 4.83 MB
/// 独占 10379 个字形；4 字文档子集 <2 KB）。
///
/// 退化映射（[`Mapping::GlyphIdFallback`]）同理：charset 写 `used` 原值，与
/// 内容流保持同一口径（此时两者都是 GID）。
///
/// `used` 里 charset 反查落空或 GID 超字形数的项**跳过**而非报错：内容流里
/// 多一个查看器画不出的 CID，代价只是 notdef，不至于让整个 PDF 退回全量。
pub fn subset_cff(cff: &[u8], used: &[u16], mapping: Mapping) -> Result<Vec<u8>, CidError> {
    // ---- 解析：结构与 [`parse_cff`] 同一条路，额外取 INDEX 原字节区间 ----
    let hdr_size = usize::from(*cff.get(2).ok_or_else(|| CidError("CFF 头部截断".into()))?);
    let (names, p) =
        read_index(cff, hdr_size).ok_or_else(|| CidError("CFF Name INDEX 损坏".into()))?;
    let (tops, p) = read_index(cff, p).ok_or_else(|| CidError("CFF TopDICT INDEX 损坏".into()))?;
    let strings_at = p;
    let (_strings, p) =
        read_index(cff, p).ok_or_else(|| CidError("CFF String INDEX 损坏".into()))?;
    // String INDEX 整段原样搬运：ROS / FontName /版本等 SID 不能挪位
    let strings_raw = slice(cff, strings_at, p - strings_at)
        .ok_or_else(|| CidError("CFF String INDEX 越界".into()))?;
    let gsubr_at = p;
    let (_gsubrs, p) =
        read_index(cff, p).ok_or_else(|| CidError("CFF Global Subr INDEX 损坏".into()))?;
    let gsubr_raw = slice(cff, gsubr_at, p - gsubr_at)
        .ok_or_else(|| CidError("CFF Global Subr INDEX 越界".into()))?;
    let top = tops
        .first()
        .ok_or_else(|| CidError("CFF 无 TopDICT".into()))?;

    let spans = dict_spans(top);
    let op_raw = |key: (u8, bool)| -> Option<&[u8]> {
        spans
            .iter()
            .find(|(_, op)| *op == key)
            .map(|(r, _)| &top[r.clone()])
    };
    let usize_of = |key: (u8, bool)| -> Option<usize> {
        op_raw(key)
            .and_then(|raw| dict_operands(raw).into_iter().next())
            .and_then(|v| usize::try_from(v).ok())
    };
    // CID-keyed 判据与 [`parse_cff`] 同源：Top DICT 有 ROS 操作数（12 30）
    if op_raw((30, true)).is_none() {
        return Err(CidError("CFF 非 CID-keyed（无 ROS），暂不子集化".into()));
    }

    let charset_off = usize_of((15, false)).unwrap_or(0);
    let cs_off = usize_of((17, false)).ok_or_else(|| CidError("TopDICT 缺 CharStrings".into()))?;
    let (charstrings, _) =
        read_index(cff, cs_off).ok_or_else(|| CidError("CharStrings INDEX 越界".into()))?;
    let glyph_count = charstrings.len();
    let charset: Vec<u16> = if charset_off <= 2 {
        (0..glyph_count).map(|g| g as u16).collect()
    } else {
        parse_charset(cff, charset_off, glyph_count)?
    };

    // ---- 保留集：内容流值 → 原 GID（按值升序，即新 GID 的次序） ----
    let mut keep: Vec<(u16, u16)> = match mapping {
        // charset 反查表：值(CID) → 首个 GID（charset 理论上不重复，重复取小）
        Mapping::CharsetCid => {
            let mut rev: Vec<(u16, u16)> = charset
                .iter()
                .enumerate()
                .skip(1)
                .map(|(g, &c)| (c, g as u16))
                .collect();
            rev.sort_unstable();
            rev.dedup_by_key(|(c, _)| *c);
            used.iter()
                .filter_map(|&cid| {
                    rev.binary_search_by_key(&cid, |(c, _)| *c)
                        .ok()
                        .map(|i| (cid, rev[i].1))
                })
                .collect()
        }
        Mapping::GlyphIdFallback => used.iter().map(|&v| (v, v)).collect(),
    };
    keep.sort_unstable();
    keep.dedup();
    keep.retain(|&(_, g)| usize::from(g) < glyph_count);

    // ---- 各段产物。偏移类操作数用定长编码（[`dict_int_fixed`]），故先以占位
    // 值建一遍求各段长度，再以真值建一遍产出，两遍字节数完全一致 ----
    // charset（format 0）：新 GID 1.. 每字形一个 u16 = 内容流值（原 CID）
    let mut charset_out = Vec::with_capacity(1 + 2 * keep.len());
    charset_out.push(0u8);
    for &(cid, _) in &keep {
        charset_out.extend_from_slice(&cid.to_be_bytes());
    }

    // FDSelect：原字体带 FDSelect 才写（单 FD 可省，规范允许），format 0 定长
    let fdselect_out = match usize_of((37, true)) {
        Some(off) => {
            let fd_of_gid = parse_fdselect(cff, off, glyph_count)?;
            let mut v = Vec::with_capacity(2 + keep.len());
            v.push(0u8); // format 0
            v.push(0u8); // GID 0 → FD 0
            for &(_, g) in &keep {
                v.push(fd_of_gid.get(usize::from(g)).copied().unwrap_or(0));
            }
            Some(v)
        }
        None => None,
    };

    // CharStrings INDEX：GID 0（.notdef）+ 保留字形，轮廓字节原样
    let mut cs_items: Vec<&[u8]> = Vec::with_capacity(keep.len() + 1);
    cs_items.push(charstrings.first().copied().unwrap_or(&[]));
    cs_items.extend(keep.iter().map(|&(_, g)| charstrings[g as usize]));
    let charstrings_out = build_index(&cs_items);

    // FDArray 与各 FD 的 Private（含局部 Subrs 原字节区间）
    let fd_dicts: Vec<&[u8]> = match usize_of((36, true)) {
        Some(off) => {
            read_index(cff, off)
                .ok_or_else(|| CidError("FDArray INDEX 越界".into()))?
                .0
        }
        None => Vec::new(),
    };
    let privates: Vec<PrivateSrc> = fd_dicts
        .iter()
        .map(|d| PrivateSrc::of_fd(cff, d))
        .collect::<Result<_, _>>()?;

    let name = names.first().copied().unwrap_or(b"NTexSub");
    let build_top = |r: TopRefs| -> Vec<u8> {
        let mut v = Vec::with_capacity(top.len() + 32);
        let mut wrote_charset = false;
        for (range, op) in spans.iter() {
            let value = match *op {
                (15, false) => {
                    wrote_charset = true;
                    Some(r.charset)
                }
                (17, false) => Some(r.charstrings),
                (36, true) if !fd_dicts.is_empty() => Some(r.fdarray),
                // 原字体带 FDSelect 才写；否则该操作符整个丢弃
                (37, true) => r.fdselect,
                // CID-keyed 的 Private 只属于 FDArray 字典，Top DICT 不写
                (18, false) => continue,
                _ => {
                    v.extend_from_slice(&top[range.clone()]);
                    None
                }
            };
            if let Some(val) = value {
                dict_int_fixed(val, &mut v);
            }
            v.extend_from_slice(&op_bytes(*op));
        }
        // 原 Top DICT 缺 charset（预定义 charset，偏移 0/1/2）：补上显式表
        if !wrote_charset {
            dict_int_fixed(r.charset, &mut v);
            v.extend_from_slice(&op_bytes((15, false)));
        }
        v
    };
    let build_fd_array = |priv_at: &[usize], priv_len: &[usize]| -> Vec<u8> {
        let items: Vec<Vec<u8>> = fd_dicts
            .iter()
            .enumerate()
            .map(|(i, d)| rewrite_private_ref(d, priv_len[i], priv_at[i]))
            .collect();
        let refs: Vec<&[u8]> = items.iter().map(Vec::as_slice).collect();
        build_index(&refs)
    };

    // ---- 布局 ----
    let name_out = build_index(&[name]);
    let priv_len: Vec<usize> = privates.iter().map(|p| p.rewrite_subrs(0).len()).collect();
    let zeros: Vec<usize> = privates.iter().map(|_| 0).collect();
    let top_len = wrap_index(&build_top(TopRefs {
        charset: 0,
        charstrings: 0,
        fdselect: fdselect_out.as_ref().map(|_| 0),
        fdarray: 0,
    }))
    .len();
    let fdarray_len = if fd_dicts.is_empty() {
        0
    } else {
        build_fd_array(&zeros, &priv_len).len()
    };

    let mut pos = 4 + name_out.len() + top_len + strings_raw.len() + gsubr_raw.len();
    let charset_at = pos;
    pos += charset_out.len();
    let fdselect_at = pos;
    pos += fdselect_out.as_ref().map_or(0, Vec::len);
    let cs_at = pos;
    pos += charstrings_out.len();
    let fdarray_at = pos;
    pos += fdarray_len;
    let mut priv_at = Vec::with_capacity(privates.len());
    for &len in &priv_len {
        priv_at.push(pos);
        pos += len;
    }
    let subrs_at: Vec<usize> = priv_at
        .iter()
        .zip(&priv_len)
        .map(|(&at, &len)| at + len)
        .collect();

    // ---- 产出 ----
    let mut out = Vec::with_capacity(pos + 16);
    out.extend_from_slice(&[0x01, 0x00, 0x04, 0x04]);
    out.extend_from_slice(&name_out);
    out.extend_from_slice(&wrap_index(&build_top(TopRefs {
        charset: charset_at,
        charstrings: cs_at,
        fdselect: fdselect_out.as_ref().map(|_| fdselect_at),
        fdarray: fdarray_at,
    })));
    out.extend_from_slice(strings_raw);
    out.extend_from_slice(gsubr_raw);
    out.extend_from_slice(&charset_out);
    if let Some(f) = &fdselect_out {
        out.extend_from_slice(f);
    }
    out.extend_from_slice(&charstrings_out);
    if !fd_dicts.is_empty() {
        out.extend_from_slice(&build_fd_array(&priv_at, &priv_len));
    }
    for (i, p) in privates.iter().enumerate() {
        out.extend_from_slice(&p.rewrite_subrs(subrs_at[i]));
    }
    for p in &privates {
        if let Some(raw) = p.subrs_raw {
            out.extend_from_slice(raw);
        }
    }
    Ok(out)
}

/// Top DICT 里需重写的四处偏移（布局期占位 0 一遍、真值一遍）。
#[derive(Clone, Copy)]
struct TopRefs {
    charset: usize,
    charstrings: usize,
    /// `None` = 不写 FDSelect 操作符（原字体就没有）。
    fdselect: Option<usize>,
    fdarray: usize,
}

/// FD 字典引用的 Private：私有 DICT 原字节与其局部 Subrs 原字节区间。
struct PrivateSrc<'a> {
    /// 私有 DICT 原字节（重建时只改 Subrs 偏移一个操作数）。
    dict: &'a [u8],
    /// 局部 Subrs INDEX 原字节（整体搬运，子程序号在 Type 2 体里是下标引用，
    /// 不能裁剪也不能重排——除非重编码全部字形轮廓）。
    subrs_raw: Option<&'a [u8]>,
}

impl<'a> PrivateSrc<'a> {
    /// 抽 FD 字典里的 Private 引用（无 Private → 空段，不占布局空间）。
    fn of_fd(c: &'a [u8], fd: &'a [u8]) -> Result<Self, CidError> {
        for (range, op) in dict_spans(fd) {
            if op != (18, false) {
                continue;
            }
            let vals = dict_operands(&fd[range]);
            let Some(i) = vals.len().checked_sub(2) else {
                break; // 操作数不足：<Private 大小> <偏移>，当无 Private 处理
            };
            let size = usize::try_from(vals[i]).map_err(|_| CidError("Private 大小为负".into()))?;
            let off =
                usize::try_from(vals[i + 1]).map_err(|_| CidError("Private 偏移为负".into()))?;
            let dict = slice(c, off, size).ok_or_else(|| CidError("Private DICT 越界".into()))?;
            return Ok(Self {
                dict,
                subrs_raw: Self::subrs_raw(c, dict)?,
            });
        }
        Ok(Self {
            dict: &[],
            subrs_raw: None,
        })
    }

    /// 私有 DICT 的 Subrs 偏移 → 该 INDEX 的原字节区间。
    fn subrs_raw(c: &'a [u8], dict: &'a [u8]) -> Result<Option<&'a [u8]>, CidError> {
        for (range, op) in dict_spans(dict) {
            if op != (19, false) {
                continue;
            }
            let Some(&off) = dict_operands(&dict[range]).last() else {
                break;
            };
            let off = usize::try_from(off).map_err(|_| CidError("Subrs 偏移为负".into()))?;
            let (_, end) =
                read_index(c, off).ok_or_else(|| CidError("局部 Subrs INDEX 越界".into()))?;
            return Ok(slice(c, off, end - off));
        }
        Ok(None)
    }

    /// 重建私有 DICT：Subrs 偏移换成子集里的新位置，其余字节原样。
    fn rewrite_subrs(&self, subrs_off: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.dict.len() + 8);
        for (range, op) in dict_spans(self.dict) {
            if op == (19, false) {
                dict_int_fixed(subrs_off, &mut v);
            } else {
                v.extend_from_slice(&self.dict[range]);
            }
            v.extend_from_slice(&op_bytes(op));
        }
        v
    }
}

/// 重建 FD 字典：Private 换成子集里的新 (大小, 偏移)，其余字节原样。
fn rewrite_private_ref(fd: &[u8], priv_len: usize, priv_off: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(fd.len() + 16);
    for (range, op) in dict_spans(fd) {
        if op == (18, false) {
            dict_int_fixed(priv_len, &mut v);
            dict_int_fixed(priv_off, &mut v);
        } else {
            v.extend_from_slice(&fd[range]);
        }
        v.extend_from_slice(&op_bytes(op));
    }
    v
}

/// 装 INDEX：offSize 取容纳最大偏移的最小宽度，偏移自 1 起（CFF 约定）。
fn build_index(items: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(items.len() as u16).to_be_bytes());
    if items.is_empty() {
        return out;
    }
    let mut offs = Vec::with_capacity(items.len() + 1);
    offs.push(1usize);
    let mut last = 1usize;
    for it in items {
        last = last.saturating_add(it.len());
        offs.push(last);
    }
    let off_size = if last <= 0xFF {
        1
    } else if last <= 0xFFFF {
        2
    } else if last <= 0xFF_FFFF {
        3
    } else {
        4
    };
    out.push(off_size);
    for &o in &offs {
        push_be(&mut out, o, usize::from(off_size));
    }
    for it in items {
        out.extend_from_slice(it);
    }
    out
}

/// 单条目 INDEX（Top DICT 用）。
fn wrap_index(item: &[u8]) -> Vec<u8> {
    build_index(&[item])
}

/// 定宽大端整数（INDEX 偏移，宽 1..=4）。
fn push_be(out: &mut Vec<u8>, v: usize, width: usize) {
    for k in (0..width).rev() {
        out.push((v >> (8 * k)) as u8);
    }
}

/// DICT 整数操作数定长编码：恒用 29（`<i32>`）5 字节，宽度与值无关——子集化
/// 靠这一点免掉「占位一遍求长度 + 补丁」的两遍布局机制（占位 0 与真值等长）。
fn dict_int_fixed(v: usize, out: &mut Vec<u8>) {
    out.push(29);
    out.extend_from_slice(&i32::try_from(v).unwrap_or(i32::MAX).to_be_bytes());
}

/// 操作符字节（12 xx 转义两字节，其余一字节）。
fn op_bytes(op: (u8, bool)) -> Vec<u8> {
    match op {
        (b1, true) => vec![12, b1],
        (b0, false) => vec![b0],
    }
}

/// DICT 逐操作符切分：`(操作数字节区间, 操作符)`，按原顺序（畸形尾部截断）。
fn dict_spans(d: &[u8]) -> Vec<(std::ops::Range<usize>, (u8, bool))> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < d.len() {
        let (op, op_len) = match d[i] {
            12 => match d.get(i + 1) {
                Some(&b1) => ((b1, true), 2usize),
                None => break,
            },
            0..=21 => ((d[i], false), 1usize),
            28 => {
                i += 3;
                continue;
            }
            29 => {
                i += 5;
                continue;
            }
            30 => {
                // 实数（BCD）：本项目不消费实数值，只跳过其字节
                i += 1;
                while i < d.len() {
                    let v = d[i];
                    i += 1;
                    if v >> 4 == 0xF || v & 0xF == 0xF {
                        break;
                    }
                }
                continue;
            }
            32..=246 => {
                i += 1;
                continue;
            }
            247..=254 => {
                i += 2;
                continue;
            }
            // 22..=27 / 31 / 255：保留字节，跳过
            _ => {
                i += 1;
                continue;
            }
        };
        out.push((start..i, op));
        i += op_len;
        start = i;
    }
    out
}

/// DICT 操作数字节 → 整数值（实数记 0：本项目只搬字节，不消费实数值）。
fn dict_operands(raw: &[u8]) -> Vec<i32> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < raw.len() {
        match raw[i] {
            28 => match raw.get(i + 1..i + 3) {
                Some(s) => {
                    out.push(i16::from_be_bytes([s[0], s[1]]) as i32);
                    i += 3;
                }
                None => break,
            },
            29 => match raw.get(i + 1..i + 5) {
                Some(s) => {
                    out.push(i32::from_be_bytes([s[0], s[1], s[2], s[3]]));
                    i += 5;
                }
                None => break,
            },
            30 => {
                i += 1;
                while i < raw.len() {
                    let v = raw[i];
                    i += 1;
                    if v >> 4 == 0xF || v & 0xF == 0xF {
                        break;
                    }
                }
                out.push(0);
            }
            32..=246 => {
                out.push(i32::from(raw[i]) - 139);
                i += 1;
            }
            247..=250 => match raw.get(i + 1) {
                Some(&b1) => {
                    out.push((i32::from(raw[i]) - 247) * 256 + i32::from(b1) + 108);
                    i += 2;
                }
                None => break,
            },
            251..=254 => match raw.get(i + 1) {
                Some(&b1) => {
                    out.push(-(i32::from(raw[i]) - 251) * 256 - i32::from(b1) - 108);
                    i += 2;
                }
                None => break,
            },
            _ => i += 1,
        }
    }
    out
}

/// 解析 FDSelect（format 0/3）→ GID → FD 号全表（缺项按 0）。
fn parse_fdselect(c: &[u8], off: usize, glyph_count: usize) -> Result<Vec<u8>, CidError> {
    match c.get(off) {
        Some(&0) => Ok(slice(c, off + 1, glyph_count)
            .ok_or_else(|| CidError("FDSelect format 0 截断".into()))?
            .to_vec()),
        Some(&3) => {
            let n = usize::from(
                u16_at(c, off + 1).ok_or_else(|| CidError("FDSelect format 3 截断".into()))?,
            );
            let sentinel = usize::from(
                u16_at(c, off + 3 + 3 * n).ok_or_else(|| CidError("FDSelect 段哨兵截断".into()))?,
            );
            let mut out = vec![0u8; glyph_count];
            for i in 0..n {
                let p = off + 3 + 3 * i;
                let first =
                    usize::from(u16_at(c, p).ok_or_else(|| CidError("FDSelect 段截断".into()))?);
                let fd = *c
                    .get(p + 2)
                    .ok_or_else(|| CidError("FDSelect 段截断".into()))?;
                let next = if i + 1 < n {
                    usize::from(u16_at(c, p + 3).ok_or_else(|| CidError("FDSelect 段截断".into()))?)
                } else {
                    sentinel
                };
                for slot in out.iter_mut().take(next.min(glyph_count)).skip(first) {
                    *slot = fd;
                }
            }
            Ok(out)
        }
        _ => Err(CidError("FDSelect 格式未知".into())),
    }
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

// ---------- 测试 ----------

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 仓库内 FandolSong（Tauri 前端自带）；缺文件则跳过，不硬依赖。
    pub(crate) fn fandol() -> Option<Vec<u8>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ntex-tauri/ui/fonts/FandolSong-Regular.otf");
        std::fs::read(path).ok()
    }

    /// 用引擎同一套解析器拆 CFF：返回（charset 全表, 每字形轮廓字节）。
    pub(crate) fn inspect(cff: &[u8]) -> Result<(Vec<u16>, Vec<Vec<u8>>), CidError> {
        let hdr = usize::from(*cff.get(2).ok_or_else(|| CidError("头部截断".into()))?);
        let (_, p) = read_index(cff, hdr).ok_or_else(|| CidError("Name INDEX".into()))?;
        let (tops, p) = read_index(cff, p).ok_or_else(|| CidError("TopDICT INDEX".into()))?;
        let top = tops.first().ok_or_else(|| CidError("无 TopDICT".into()))?;
        let (_, p) = read_index(cff, p).ok_or_else(|| CidError("String INDEX".into()))?;
        let (_, _p) = read_index(cff, p).ok_or_else(|| CidError("Global Subr INDEX".into()))?;
        let spans = dict_spans(top);
        let val = |key: (u8, bool)| -> usize {
            spans
                .iter()
                .find(|(_, op)| *op == key)
                .and_then(|(r, _)| dict_operands(&top[r.clone()]).into_iter().next())
                .unwrap_or(0) as usize
        };
        let (cs, _) =
            read_index(cff, val((17, false))).ok_or_else(|| CidError("CharStrings".into()))?;
        let charset_off = val((15, false));
        let charset = if charset_off <= 2 {
            (0..cs.len()).map(|g| g as u16).collect()
        } else {
            parse_charset(cff, charset_off, cs.len())?
        };
        Ok((charset, cs.iter().map(|s| s.to_vec()).collect()))
    }

    #[test]
    fn subset_keeps_only_used_cids_and_preserves_ros() {
        let Some(otf) = fandol() else {
            eprintln!("未找到 FandolSong，跳过");
            return;
        };
        let full = bare_cff(&otf).expect("剥壳");
        let map = build(&otf).expect("CID 映射");
        assert_eq!(map.mapping(), Mapping::CharsetCid);
        // 「中」= 0x11CF（Adobe-GB1 CID 4559）、「国」= 0x0753（与 pdf.rs 测试同源）
        let used = [0x11CF_u16, 0x0753];
        let sub = subset_cff(&full, &used, Mapping::CharsetCid).expect("子集化");
        assert!(
            sub.len() < 4096,
            "2 字形子集应在 KB 级：{}（全量 {}）",
            sub.len(),
            full.len()
        );

        // 结构自洽：可再解析、charset 恰为原 CID、字形数 = 用到数 + .notdef
        let (charset, cs) = inspect(&sub).expect("子集应可再解析");
        assert_eq!(charset, vec![0, 0x0753, 0x11CF], "charset 应写原 CID");
        assert_eq!(cs.len(), 3, "GID 0 + 2 个保留字形");
        // 轮廓字节与全量字体逐字节一致（不重编码 Type 2）
        let (full_charset, full_cs) = inspect(&full).expect("全量可解析");
        for (new, cid) in [(1usize, 0x0753u16), (2, 0x11CF)] {
            let old = full_charset
                .iter()
                .position(|&c| c == cid)
                .expect("全量应含该 CID");
            assert_eq!(cs[new], full_cs[old], "CID {cid} 的轮廓应原样搬运");
        }

        // 保留集为空时只剩 .notdef
        let empty = subset_cff(&full, &[], Mapping::CharsetCid).expect("空子集");
        assert_eq!(inspect(&empty).unwrap().1.len(), 1);
    }

    #[test]
    fn subset_is_resubsetable_and_drops_unused_subrs_never() {
        // 子集本身再子集化（收窄保留集）应仍得到合法结构——布局/偏移改写可重入
        let Some(otf) = fandol() else {
            eprintln!("未找到 FandolSong，跳过");
            return;
        };
        let full = bare_cff(&otf).expect("剥壳");
        let sub = subset_cff(&full, &[0x11CF, 0x0753], Mapping::CharsetCid).expect("子集");
        let sub2 = subset_cff(&sub, &[0x11CF], Mapping::CharsetCid).expect("二次子集");
        let (charset, cs) = inspect(&sub2).expect("二次子集应可再解析");
        assert_eq!(charset, vec![0, 0x11CF]);
        assert_eq!(cs.len(), 2);
    }

    #[test]
    fn subset_reports_malformed_input_without_panicking() {
        let Some(otf) = fandol() else {
            eprintln!("未找到 FandolSong，跳过");
            return;
        };
        let full = bare_cff(&otf).expect("剥壳");
        // 截断在各个前缀上都必须 Err，不得 panic（引擎契约：畸形输入不 panic）
        for cut in [0usize, 1, 2, 3, 4, 8, 16, 40, 300, 1000] {
            let Some(bytes) = full.get(..cut) else { break };
            assert!(
                subset_cff(bytes, &[0x11CF], Mapping::CharsetCid).is_err(),
                "截断到 {cut} 字节应报错"
            );
        }
        // 非 CID-keyed（无 ROS）字体：明确拒绝而非产出坏结构
        assert!(subset_cff(b"\x01\x00\x04\x04garbage", &[1], Mapping::CharsetCid).is_err());
    }
}
