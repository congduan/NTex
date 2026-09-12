//! TFM（TeX Font Metrics）解析（tftopl.web / TeXbook 附录 F）。
//!
//! TFM 文件是 32 位字（big-endian）流；字节数恒为 4 的倍数。
//! 头部 12 个 16 位计数（2 个/字，共 6 字），其后依次为：
//! 头部字 / 字符信息 / 宽度表 / 高度表 / 深度表 / 斜修正表 /
//! 连字-字距程序 / 字距表 / 可扩展表 / 参数表（均为 32 位 `fix_word`）。
//! 恒等式：`lf = 6 + lh + (ec-bc+1) + nw + nh + nd + ni + nl + nk + ne + np`。
//!
//! `fix_word` 为 12.20 定点（带符号）：设计字号存 pt，其余维度存设计字号单位；
//! 换算到 sp：`sp = word × design_size_sp / 2^20`（四舍五入）。
//!
//! M3-4 范围：字符 width/height/depth + 字体参数（space/space_stretch/
//! space_shrink/x_height/quad/extra_space）+ **连字/字距程序**（lig_kern 表：
//! cmr10 的 fi/fl 连字与 kern 对）。lig_kern 程序在字符追加时由排版器执行。

use std::fmt;

use ntex_core::register::Glue;

/// TFM 解析错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TfmError(pub String);

impl fmt::Display for TfmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TFM 解析失败：{}", self.0)
    }
}

impl std::error::Error for TfmError {}

type Result<T> = std::result::Result<T, TfmError>;

/// char_info 表项（TFM 每字符 32 位字；TeXbook 附录 F）：
/// 维度表索引 + tag 关联信息（tag=1 → lig/kern 起点；tag=2 → 更大变体字符）。
#[derive(Debug, Clone, Copy)]
struct CharInfoEntry {
    width_index: usize,
    height_index: usize,
    depth_index: usize,
    /// 斜体修正表索引（byte2 高 6 位；tex.web `char_italic_end`）
    italic_index: usize,
    /// tag=1：lig/kern 程序起始索引
    lig_kern: Option<u16>,
    /// tag=2：更大变体字符码（list_tag，Op 大算符放大取字形来源）
    larger: Option<u8>,
}

/// lig/kern 程序步（TFM lig_kern 表的一个 32 位字；tex.web §10583-10605）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LigKernStep {
    /// skip 字节：≥128 = 程序结束（stop_flag，无命令）；否则为右字符不匹配时
    /// 跳过的条目数（下一步 = 当前 + skip + 1）。
    pub skip_byte: u8,
    /// next_char：待匹配的右字符（匹配则执行命令并停止）。
    pub next_char: u8,
    /// 操作码：≥128 = kern 步（kern 索引 = 256×(op−128) + remainder）；
    /// <128 = 连字步（op = 4a+2b+c：b=0 删左字符、c=0 删右字符、a 越过的字符数）。
    pub op_byte: u8,
    /// remainder：kern → 索引低字节；连字 → 结果字符码。
    pub remainder: u8,
}

/// lig/kern 程序匹配结果（排版器应用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LigKern {
    /// 字距：在左字符后插入 kern（sp）。
    Kern(i64),
    /// 连字：左字符替换为结果字符码，右字符被丢弃。
    Lig(u8),
}

/// 字节读取器（大端）。
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.bytes.len() {
            return Err(TfmError(format!(
                "文件截断（需要 {n} 字节，位置 {}）",
                self.pos
            )));
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
}

/// 解析后的字体度量（维度已按设计字号换算为 sp）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontMetrics {
    /// 设计字号（sp，**未缩放**的原设计字号；DVI `fnt_def` 的 d 字段）。
    pub design_size_sp: i64,
    /// DVI `fnt_def` 的 s 字段：实际尺寸/设计字号 × 2^20（`scaled_by` 更新；
    /// 原尺寸 = 2^20）。
    pub scale: i64,
    /// TFM 头部校验和（header[0]；DVI `fnt_def` 的 c 字段）。
    pub checksum: u32,
    /// 外部字体名（如 "cmr10"；加载器填充，DVI `fnt_def` 的 name）。
    pub name: String,
    /// charcode → (width, height, depth)（sp，已按当前缩放换算）。
    pub chars: Vec<Option<(i64, i64, i64)>>,
    /// charcode → 斜体修正（sp，已按当前缩放换算；tex.web `char_italic`；
    /// 数学行组装按 tex.web §759 追加为显式 kern）。未定义字符为 0。
    pub char_italic: Vec<i64>,
    /// 字体参数（TeX 参数 1..=7，sp；缺失为 0）。
    pub slant: i64,
    pub space: i64,
    pub space_stretch: i64,
    pub space_shrink: i64,
    pub x_height: i64,
    pub quad: i64,
    pub extra_space: i64,
    /// lig/kern 程序（TFM lig_kern 表，原样；缩放不改变）。
    pub lig_kern_steps: Vec<LigKernStep>,
    /// 字距值表（TFM 字距表，已按当前缩放换算为 sp）。
    pub kern_values: Vec<i64>,
    /// charcode → lig/kern 程序起始索引（char_info tag=1；无程序为 None）。
    pub lig_kern_index: Vec<Option<u16>>,
    /// charcode → 更大变体字符（char_info tag=2 list_tag，TeXbook 附录 F；
    /// tex.web make_op display 大算符放大 / var_delimiter 定界符放大用；
    /// cmex10：char 80 (text Σ) → 88 (display Σ) 等）。无变体为 None。
    pub next_larger: Vec<Option<u8>>,
    /// 全量字体参数（fontdimen；`font_params[i-1]` = TFM 参数 i，已缩放）。
    /// 数学字体用：参数 8+（sup/sub 高度、分式间距、delimiter 等）。
    pub font_params: Vec<i64>,
    /// Unicode 直映字体标记（M9 中文刀 1）：本字体由 OTF/TTF 加载，字符度量
    /// 按 Unicode 码位存于 [`Self::unicode_chars`]，[`Self::chars`]（8-bit 槽表）
    /// 保持空。`\char` 的合法码位上限、后端字形查找口径均据此分叉——TFM 字体
    /// 走 TeX 8-bit 语义（上限 255），本类字体走 XeTeX Unicode 语义（0x10FFFF）。
    pub unicode_native: bool,
    /// Unicode 码位 → `(width, height, depth)`（sp，已按当前缩放换算）。
    /// 仅 `unicode_native` 字体非空；**按码位升序**（二分查询）。
    pub unicode_chars: Vec<(u32, (i64, i64, i64))>,
}

impl FontMetrics {
    /// 字符维度；未定义字符返回 (0, 0, 0)。
    pub fn char_metrics(&self, charcode: u32) -> (i64, i64, i64) {
        self.char_metrics_opt(charcode).unwrap_or((0, 0, 0))
    }

    /// 字符维度查询（含"是否存在"语义）：无该字符返回 `None`。
    ///
    /// 两条字体路径的统一入口——`unicode_native` 按 Unicode 码位二分，
    /// 否则按 8-bit 槽表直查。`\iffontchar`/`\fontchar*` 与排版建节点共用。
    pub fn char_metrics_opt(&self, charcode: u32) -> Option<(i64, i64, i64)> {
        if self.unicode_native {
            return self
                .unicode_chars
                .binary_search_by_key(&charcode, |(cp, _)| *cp)
                .ok()
                .map(|i| self.unicode_chars[i].1);
        }
        self.chars.get(charcode as usize).copied().flatten()
    }

    /// 字符斜体修正（tex.web `char_italic(f)(q)`）；未定义字符为 0。
    pub fn char_italic(&self, charcode: u32) -> i64 {
        self.char_italic
            .get(charcode as usize)
            .copied()
            .unwrap_or(0)
    }

    /// 字符是否在字体中定义（tex.web `char_exists(char_info(f)(c))`；
    /// `new_character` 据此决定建节点还是发 "Missing character" 警告）。
    pub fn char_exists(&self, charcode: u32) -> bool {
        self.char_metrics_opt(charcode).is_some()
    }

    /// 词间空白胶水（space / space_stretch / space_shrink）。
    pub fn space_glue(&self) -> Glue {
        Glue::new(self.space, self.space_stretch, self.space_shrink)
    }

    /// 执行左字符的 lig/kern 程序查右字符（tex.web main_loop 的
    /// "ligature/kern command relevant to cur_l and cur_r"）：
    /// 从程序起点逐条目：stop（skip==128）→ 无命令；next_char 匹配 → kern/连字；
    /// 不匹配 → 跳 skip+1。**入口重定向**（skip>128，TFM 紧凑格式：char_info
    /// 的 remainder 指向重定向表条目，目标 = u16(op_byte, remainder)——texcraft
    /// deserialize 同款解析）：先解引用到真实程序再匹配。连字仅支持
    /// `x y =: z`（a=b=c=0，左右都删）。
    pub fn apply_lig_kern(&self, left: u8, right: u8) -> Option<LigKern> {
        let start = self.lig_kern_index.get(left as usize).copied().flatten()? as usize;
        // 入口重定向（skip>128）：目标程序索引 = op_byte<<8 | remainder
        let mut k = match self.lig_kern_steps.get(start) {
            Some(s) if s.skip_byte > 128 => ((s.op_byte as usize) << 8) | s.remainder as usize,
            _ => start,
        };
        loop {
            let s = *self.lig_kern_steps.get(k)?;
            if s.skip_byte == 128 {
                return None; // stop_flag：程序结束，无命令
            }
            // 程序中间的重定向（罕见）：继续解引用
            if s.skip_byte > 128 {
                k = ((s.op_byte as usize) << 8) | s.remainder as usize;
                continue;
            }
            if s.next_char == right {
                if s.op_byte >= 128 {
                    // kern 步：kern 索引 = 256×(op−128) + remainder
                    let ki = 256 * (s.op_byte as usize - 128) + s.remainder as usize;
                    return Some(LigKern::Kern(
                        self.kern_values.get(ki).copied().unwrap_or(0),
                    ));
                }
                // 连字步 op = 4a+2b+c：仅支持 a=0、b=0、c=0（删左右、插 remainder）
                let (a, b, c) = (s.op_byte / 4, (s.op_byte / 2) % 2, s.op_byte % 2);
                if a == 0 && b == 0 && c == 0 {
                    return Some(LigKern::Lig(s.remainder));
                }
                return None; // 保留左/右字符的连字（罕见）暂不支持
            }
            // 不匹配：跳过 skip 个中间条目（tex.web `main_k + skip + 1`）
            k += s.skip_byte as usize + 1;
        }
    }

    /// 按 `num/den` 缩放全部维度（`\font..at 12pt`：num=12pt, den=design_size_sp；
    /// `\font..scaled 1200`：num=1200, den=1000）。逐项四舍五入（远离零）。
    ///
    /// 设计字号（`design_size_sp`）保持原值——DVI `fnt_def` 的 d 用原设计字号、
    /// s 用 [`Self::scale`]（= round(num/den × 2^20)）。
    pub fn scaled_by(&self, num: i64, den: i64) -> FontMetrics {
        let scale = |v: i64| {
            let n = v as i128 * num as i128;
            let d = den as i128;
            if n >= 0 {
                ((n + d / 2) / d) as i64
            } else {
                -(((-n) + d / 2) / d) as i64
            }
        };
        let scale_s = scale(1 << 20);
        FontMetrics {
            design_size_sp: self.design_size_sp,
            scale: scale_s,
            checksum: self.checksum,
            name: self.name.clone(),
            chars: self
                .chars
                .iter()
                .map(|c| c.map(|(w, h, d)| (scale(w), scale(h), scale(d))))
                .collect(),
            char_italic: self.char_italic.iter().map(|&v| scale(v)).collect(),
            slant: scale(self.slant),
            space: scale(self.space),
            space_stretch: scale(self.space_stretch),
            space_shrink: scale(self.space_shrink),
            x_height: scale(self.x_height),
            quad: scale(self.quad),
            extra_space: scale(self.extra_space),
            lig_kern_steps: self.lig_kern_steps.clone(),
            kern_values: self.kern_values.iter().map(|&v| scale(v)).collect(),
            lig_kern_index: self.lig_kern_index.clone(),
            next_larger: self.next_larger.clone(),
            font_params: self.font_params.iter().map(|&v| scale(v)).collect(),
            unicode_native: self.unicode_native,
            // 码位顺序在缩放中不变（逐项线性变换），二分前提得以保持
            unicode_chars: self
                .unicode_chars
                .iter()
                .map(|&(cp, (w, h, d))| (cp, (scale(w), scale(h), scale(d))))
                .collect(),
        }
    }
}

/// 解析 TFM 文件字节。
pub fn parse_tfm(bytes: &[u8]) -> Result<FontMetrics> {
    let mut r = Reader::new(bytes);
    let lf = r.u16()?;
    let lh = r.u16()?;
    let bc = r.u16()?;
    let ec = r.u16()?;
    let nw = r.u16()?;
    let nh = r.u16()?;
    let nd = r.u16()?;
    let ni = r.u16()?;
    let nl = r.u16()?;
    let nk = r.u16()?;
    let ne = r.u16()?;
    let np = r.u16()?;

    // 长度恒等式校验（lf 为 32 位字数）
    let count = ec
        .checked_sub(bc)
        .map(|d| d as usize + 1)
        .ok_or_else(|| TfmError("bc > ec".into()))?;
    let words = 6
        + lh as usize
        + count
        + nw as usize
        + nh as usize
        + nd as usize
        + ni as usize
        + nl as usize
        + nk as usize
        + ne as usize
        + np as usize;
    if words != lf as usize {
        return Err(TfmError(format!("长度恒等式不符：{words} ≠ lf={lf}")));
    }

    // 头部字：header[0] = checksum，header[1] = 设计字号（fix_word，pt）
    let mut design_word: i64 = 0;
    let mut checksum: u32 = 0;
    for i in 0..lh {
        let w = r.u32()?;
        if i == 0 {
            checksum = w;
        } else if i == 1 {
            design_word = w as i32 as i64;
        }
    }
    // design_sp = design_pt × 2^16 = (word / 2^20) × 2^16 = word / 16
    let design_size_sp = design_word / 16;
    // fix_word（设计字号单位）→ sp：**截断**（对照真实 TeX：cmr10 'H' 高
    // 0xAEEEE×0.625=447828.75 → 447828，而非四舍五入 447829）
    let scale = |v: i64| {
        if v >= 0 {
            (v * design_size_sp) >> 20
        } else {
            -(((-v) as i128 * design_size_sp as i128 + (1 << 19)) >> 20) as i64
        }
    };

    // 字符信息表：(ec - bc + 1) 个 32 位字
    // 布局：byte0 = width 索引；byte1 = height(高4位)|depth(低4位)；
    // byte2 = italic(高6位)|tag(低2位)；byte3 = remainder。
    // tag=1 → remainder = lig/kern 程序索引；tag=2 → remainder = 更大变体
    // 字符（list_tag，TeXbook 附录 F）。
    let mut char_info: Vec<CharInfoEntry> = Vec::with_capacity(count);
    for _ in 0..count {
        let w = r.u32()?;
        let tag = (w >> 8) & 0x03;
        let remainder = (w & 0xFF) as u16;
        char_info.push(CharInfoEntry {
            width_index: ((w >> 24) & 0xFF) as usize,
            height_index: ((w >> 20) & 0x0F) as usize,
            depth_index: ((w >> 16) & 0x0F) as usize,
            italic_index: ((w >> 10) & 0x3F) as usize,
            lig_kern: (tag == 1).then_some(remainder),
            larger: (tag == 2).then_some(remainder as u8),
        });
    }

    // 维度表（fix_word）：widths / heights / depths / italics
    let widths = read_words(&mut r, nw)?;
    let heights = read_words(&mut r, nh)?;
    let depths = read_words(&mut r, nd)?;
    let italics = read_words(&mut r, ni)?;
    // lig/kern 程序（nl 字）：b0 = skip 字节，b1 = next_char，b2 = op 字节，b3 = remainder
    let mut lig_kern_steps = Vec::with_capacity(nl as usize);
    for _ in 0..nl {
        let w = r.u32()?;
        lig_kern_steps.push(LigKernStep {
            skip_byte: (w >> 24) as u8,
            next_char: (w >> 16) as u8,
            op_byte: (w >> 8) as u8,
            remainder: w as u8,
        });
    }
    // 字距表（nk 字，fix_word → sp）
    let kern_values = read_words(&mut r, nk)?
        .iter()
        .map(|&v| scale(v))
        .collect::<Vec<_>>();
    // 跳过可扩展表（M4 起用）
    r.skip(ne as usize * 4)?;
    // 参数表（fix_word）
    let params = read_words(&mut r, np)?;

    // 字符度量组装
    let mut chars = vec![None; 256];
    let mut char_italic = vec![0i64; 256];
    let mut lig_kern_index = vec![None; 256];
    let mut next_larger = vec![None; 256];
    for (i, e) in char_info.iter().enumerate() {
        let charcode = bc as usize + i;
        if charcode < chars.len() {
            let width = widths.get(e.width_index).map(|&w| scale(w)).unwrap_or(0);
            let height = heights.get(e.height_index).map(|&h| scale(h)).unwrap_or(0);
            let depth = depths.get(e.depth_index).map(|&d| scale(d)).unwrap_or(0);
            chars[charcode] = Some((width, height, depth));
            char_italic[charcode] = italics.get(e.italic_index).map(|&v| scale(v)).unwrap_or(0);
            lig_kern_index[charcode] = e.lig_kern;
            next_larger[charcode] = e.larger;
        }
    }
    // 参数：TeX 参数 1=slant 2=space 3=space_stretch 4=space_shrink
    //       5=x_height 6=quad 7=extra_space（params[0] 即参数 1）
    let p = |i: usize| params.get(i).map(|&v| scale(v)).unwrap_or(0);
    let font_params = params.iter().map(|&v| scale(v)).collect();
    Ok(FontMetrics {
        design_size_sp,
        scale: 1 << 20, // 设计字号 = 原尺寸
        checksum,
        name: String::new(), // 加载器（ntex-layout TfmLoader）填充
        chars,
        char_italic,
        slant: p(0),
        space: p(1),
        space_stretch: p(2),
        space_shrink: p(3),
        x_height: p(4),
        quad: p(5),
        extra_space: p(6),
        lig_kern_steps,
        kern_values,
        lig_kern_index,
        next_larger,
        font_params,
        // TFM 是 8-bit 编码向量字体：永远走 `chars` 槽表（0..=255）
        unicode_native: false,
        unicode_chars: Vec::new(),
    })
}

/// 读取 n 个 32 位 fix_word（带符号 12.20）。
fn read_words(r: &mut Reader, n: u16) -> Result<Vec<i64>> {
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        out.push(r.u32()? as i32 as i64);
    }
    Ok(out)
}

/// 查找 `<name>.tfm`：环境变量 NTEX_TFM_DIR → 常见 TeX Live 安装路径 → kpsewhich。
pub fn find_tfm(name: &str) -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let file = format!("{name}.tfm");
    if let Ok(dir) = std::env::var("NTEX_TFM_DIR") {
        let p = PathBuf::from(dir).join(&file);
        if p.exists() {
            return Some(p);
        }
    }
    // CM 字体常见安装位置（texlive texmf-dist / texmf）
    const ROOTS: [&str; 3] = [
        "/usr/local/texlive/2024basic",
        "/usr/local/texlive/2023",
        "/usr/share/texlive",
    ];
    for root in ROOTS {
        for sub in [
            "/texmf-dist/fonts/tfm/public/cm/",
            "/texmf/fonts/tfm/public/cm/",
        ] {
            let p = PathBuf::from(root).join(sub).join(&file);
            if p.exists() {
                return Some(p);
            }
        }
    }
    if let Ok(out) = std::process::Command::new("kpsewhich").arg(&file).output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            let p = PathBuf::from(s.trim());
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

// ---------- 进程级 TFM 字节注册表（M9：PDF 导出进 wasm） ----------

use std::collections::HashMap;
use std::io::{self, ErrorKind};
use std::sync::{LazyLock, Mutex};

/// 进程级 TFM 字节注册表：[`register_tfm_bytes`] 写入，[`read_tfm`] 优先命中。
static TFM_BYTES: LazyLock<Mutex<HashMap<String, Vec<u8>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 注册 TFM 字节（`tex_name` 为 TeX 字体名，如 `cmr10`——须与 DVI `fnt_def`
/// 的外部名一致；同名覆盖）。
///
/// 与 [`find_tfm`] 的分工：那条链按**路径**找文件（环境变量 → TeX Live →
/// kpsewhich），本注册表按**名字**直接给字节——为无文件系统环境（wasm32）
/// 而设：`ntex-wasm` 启动时把内嵌 CM TFM 全表注册进来，`ntex-pdf` 的 PDF
/// 写出经 [`read_tfm`] 优先命中。不注册（native 默认）行为不变。
/// 与 `ntex-layout::set_tfm_source` 的缝分工：那条缝服务排版器（`\font`
/// 解析），本注册表服务 PDF 写出（读度量）。
///
/// 只做字节搬运、不做解析校验：坏 TFM 由 `parse_tfm` 在使用处报带上下文的
/// 错，注册表不该替调用方预判格式。
///
/// 返回 `false` 仅当注册表锁毒化（持锁线程 panic；不传播错误，引擎契约）。
pub fn register_tfm_bytes(name: &str, bytes: &[u8]) -> bool {
    match TFM_BYTES.lock() {
        Ok(mut m) => {
            m.insert(name.to_owned(), bytes.to_vec());
            true
        }
        Err(_) => false,
    }
}

/// 已注册的 TFM 字节（不触碰文件系统）。
pub fn registered_tfm_bytes(name: &str) -> Option<Vec<u8>> {
    TFM_BYTES.lock().ok()?.get(name).cloned()
}

/// TFM 取数总入口：注册表优先，回落 [`find_tfm`] + `std::fs::read`。
///
/// `ntex-pdf` 的 PDF 写出经此读度量——wasm 下文件链恒空（无文件系统亦无
/// 子进程），注册表是唯一来源；native 未注册时行为与旧的「find_tfm + read」
/// 逐字一致。
pub fn read_tfm(name: &str) -> io::Result<Vec<u8>> {
    if let Some(bytes) = registered_tfm_bytes(name) {
        return Ok(bytes);
    }
    let path = find_tfm(name)
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, format!("找不到 TFM：{name}")))?;
    std::fs::read(&path)
        .map_err(|e| io::Error::new(e.kind(), format!("读 {}：{e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_u16(out: &mut Vec<u8>, v: u16) {
        out.extend_from_slice(&v.to_be_bytes());
    }

    fn push_u32(out: &mut Vec<u8>, v: u32) {
        out.extend_from_slice(&v.to_be_bytes());
    }

    /// 合成微型 TFM：设计 10pt，字符 A/B/C，参数齐全。
    fn synthetic_tfm() -> Vec<u8> {
        let mut out = Vec::new();
        let lh: u16 = 2;
        let bc: u16 = 65;
        let ec: u16 = 67;
        let nw: u16 = 3;
        let nh: u16 = 2;
        let nd: u16 = 2;
        let ni: u16 = 0;
        let nl: u16 = 0;
        let nk: u16 = 0;
        let ne: u16 = 0;
        let np: u16 = 7;
        push_u16(&mut out, 0); // lf 占位
        for v in [lh, bc, ec, nw, nh, nd, ni, nl, nk, ne, np] {
            push_u16(&mut out, v);
        }
        // 头部字：checksum + 设计字号（10pt = 10×2^20）
        push_u32(&mut out, 0);
        push_u32(&mut out, 10 * (1 << 20));
        // 字符信息：A(w0,h0,d0) B(w1,h1,d0) C(w2,h0,d1)
        push_u32(&mut out, 0); // A：全零索引
        push_u32(&mut out, (1 << 24) | (1 << 20)); // B(w1,h1,d0)
        push_u32(&mut out, (2 << 24) | (1 << 16)); // C(w2,h0,d1)
                                                   // 宽度/高度/深度表（fix_word）
        for w in [600_000u32, 700_000, 800_000] {
            push_u32(&mut out, w);
        }
        for h in [400_000u32, 500_000] {
            push_u32(&mut out, h);
        }
        for d in [100_000u32, 200_000] {
            push_u32(&mut out, d);
        }
        // 参数表：slant space space_stretch space_shrink x_height quad extra_space
        for v in [0u32, 300_000, 150_000, 100_000, 400_000, 700_000, 100_000] {
            push_u32(&mut out, v);
        }
        // 回填 lf（32 位字数）：6 + 2 + 3 + 3 + 2 + 2 + 7 = 25
        let lf = 6 + lh + (ec - bc + 1) + nw + nh + nd + np;
        out[0..2].copy_from_slice(&lf.to_be_bytes());
        out
    }

    #[test]
    fn parses_synthetic_tfm() {
        let fm = parse_tfm(&synthetic_tfm()).expect("解析合成 TFM");
        assert_eq!(fm.design_size_sp, 10 * 65_536);
        // 换算：v × 655360 / 2^20 = v × 0.625
        assert_eq!(fm.char_metrics(65), (375_000, 250_000, 62_500));
        assert_eq!(fm.char_metrics(66), (437_500, 312_500, 62_500));
        assert_eq!(fm.char_metrics(67), (500_000, 250_000, 125_000));
        assert_eq!(fm.space, 187_500);
        assert_eq!(fm.space_stretch, 93_750);
        assert_eq!(fm.space_shrink, 62_500);
        assert_eq!(fm.x_height, 250_000);
        assert_eq!(fm.quad, 437_500);
        assert_eq!(fm.char_metrics(32), (0, 0, 0)); // 未定义
    }

    #[test]
    fn bad_length_rejected() {
        let mut bytes = synthetic_tfm();
        bytes[0] = 0xff; // 破坏 lf
        assert!(parse_tfm(&bytes).is_err());
    }

    #[test]
    fn truncated_file_errors() {
        let mut bytes = synthetic_tfm();
        bytes.truncate(bytes.len() / 2);
        assert!(parse_tfm(&bytes).is_err());
    }

    /// 真实 cmr10.tfm（无 TeX 安装则跳过）。
    #[test]
    fn parses_real_cmr10() {
        let Some(path) = find_tfm("cmr10") else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let bytes = std::fs::read(path).expect("读取 cmr10.tfm");
        let fm = parse_tfm(&bytes).expect("解析 cmr10.tfm");
        assert_eq!(fm.design_size_sp, 10 * 65_536, "cmr10 设计字号应为 10pt");
        // 词间距 ≈ 3.33333pt = 218,453 sp
        assert!((210_000..220_000).contains(&fm.space), "space={}", fm.space);
        // 字符 M（77）：宽 = 0x000EAAAD fix_word ≈ 0.9167em ≈ 600,748 sp；高 ≈ 0.68em，无深度
        let (mw, mh, md) = fm.char_metrics(77);
        assert!((595_000..605_000).contains(&mw), "M 宽 {mw}");
        assert!(mh > 0, "M 高 {mh}");
        assert_eq!(md, 0);
        // 空格字符（32）自身宽度 ≈ 2.777pt（≠ space 参数 3.333pt；TeX 词间距用参数；
        // cmr10 给空格字符也赋了高度，不影响排版）
        let (sw, _, _) = fm.char_metrics(32);
        assert!((175_000..190_000).contains(&sw), "空格字符宽 {sw}");
        assert_ne!(sw, fm.space);
        // lig/kern：'f'+'i' → fi（字符 12）、'f'+'l' → fl（13）、'f'+'f' → ff（11）
        assert_eq!(fm.apply_lig_kern(b'f', b'i'), Some(LigKern::Lig(12)));
        assert_eq!(fm.apply_lig_kern(b'f', b'l'), Some(LigKern::Lig(13)));
        assert_eq!(fm.apply_lig_kern(b'f', b'f'), Some(LigKern::Lig(11)));
        // kern 对（与 pdfTeX DVI 实测一致）：v→e、w→o、n→t = -18205 sp；
        // o→c、b→e = +18205 sp
        assert_eq!(fm.apply_lig_kern(b'v', b'e'), Some(LigKern::Kern(-18_205)));
        assert_eq!(fm.apply_lig_kern(b'w', b'o'), Some(LigKern::Kern(-18_205)));
        assert_eq!(fm.apply_lig_kern(b'n', b't'), Some(LigKern::Kern(-18_205)));
        assert_eq!(fm.apply_lig_kern(b'o', b'c'), Some(LigKern::Kern(18_205)));
        assert_eq!(fm.apply_lig_kern(b'b', b'e'), Some(LigKern::Kern(18_205)));
        // 无程序的字符（如 'a'）或未命中 → None
        assert_eq!(fm.apply_lig_kern(b'a', b'b'), None);
        assert_eq!(fm.apply_lig_kern(b'v', b'x'), None);
    }

    #[test]
    fn scaled_by_scales_all_dims() {
        let fm = parse_tfm(&synthetic_tfm()).expect("解析合成 TFM");
        assert_eq!(fm.scale, 1 << 20, "解析默认原尺寸");
        // scaled 1200 → 1.2×：A 宽 375000 → 450000；设计字号保持 10pt
        let s = fm.scaled_by(1200, 1000);
        assert_eq!(s.design_size_sp, 10 * 65_536, "设计字号不随缩放改变");
        assert_eq!(s.scale, 1_258_291, "DVI s = round(1.2 × 2^20)");
        assert_eq!(s.char_metrics(65), (450_000, 300_000, 75_000));
        assert_eq!(s.space, 225_000);
        assert_eq!(s.quad, 525_000);
        // scaled 1000 → 恒等
        assert_eq!(fm.scaled_by(1000, 1000), fm);
        // scaled 0 → 全零维度，scale 0
        let z = fm.scaled_by(0, 1000);
        assert_eq!(z.design_size_sp, 10 * 65_536);
        assert_eq!(z.scale, 0);
        assert_eq!(z.char_metrics(65), (0, 0, 0));
    }

    /// 注册表语义锁：注册优先于宿主文件链、字节原样搬运（不解析校验）、
    /// 未注册且宿主也无 → `read_tfm` 报 NotFound。名字用不可能真实存在的
    /// 前缀，避免与其他并行测试及宿主 TeX Live 冲突。
    #[test]
    fn tfm_bytes_registry_roundtrip_and_read_tfm_fallback() {
        let name = "zz-not-a-real-font-anywhere";
        assert!(
            registered_tfm_bytes(name).is_none(),
            "前置假设：该名字尚未注册"
        );
        let bytes: &[u8] = b"\x00\x01\x02\x03-registry-is-a-dumb-transport";
        assert!(register_tfm_bytes(name, bytes));
        // 注册命中：read_tfm 返回注册字节本身（哪怕不是合法 TFM——搬运不校验）
        assert_eq!(read_tfm(name).expect("注册后应命中"), bytes.to_vec());
        assert_eq!(registered_tfm_bytes(name).as_deref(), Some(bytes));

        // 未注册 + 宿主查找链也命不中 → NotFound 且带字体名上下文
        let miss = "zz-never-registered-nor-installed";
        let err = read_tfm(miss).expect_err("不存在于任何来源").to_string();
        assert!(err.contains(miss), "错误应带字体名：{err}");
    }
}
