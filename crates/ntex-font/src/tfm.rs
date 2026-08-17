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
//! space_shrink/x_height/quad/extra_space）；连字/字距程序跳过（M4 断字时补）。

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
    /// 设计字号（sp）。
    pub design_size_sp: i64,
    /// charcode → (width, height, depth)（sp）。
    pub chars: Vec<Option<(i64, i64, i64)>>,
    /// 字体参数（TeX 参数 1..=7，sp；缺失为 0）。
    pub slant: i64,
    pub space: i64,
    pub space_stretch: i64,
    pub space_shrink: i64,
    pub x_height: i64,
    pub quad: i64,
    pub extra_space: i64,
}

impl FontMetrics {
    /// 字符维度；未定义字符返回 (0, 0, 0)。
    pub fn char_metrics(&self, charcode: u32) -> (i64, i64, i64) {
        self.chars
            .get(charcode as usize)
            .copied()
            .flatten()
            .unwrap_or((0, 0, 0))
    }

    /// 词间空白胶水（space / space_stretch / space_shrink）。
    pub fn space_glue(&self) -> Glue {
        Glue {
            width: self.space,
            stretch: self.space_stretch,
            shrink: self.space_shrink,
        }
    }

    /// 按 `num/den` 缩放全部维度（`\font..at 12pt`：num=12pt, den=design_size_sp；
    /// `\font..scaled 1200`：num=1200, den=1000）。逐项四舍五入（远离零）。
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
        FontMetrics {
            design_size_sp: scale(self.design_size_sp),
            chars: self
                .chars
                .iter()
                .map(|c| c.map(|(w, h, d)| (scale(w), scale(h), scale(d))))
                .collect(),
            slant: scale(self.slant),
            space: scale(self.space),
            space_stretch: scale(self.space_stretch),
            space_shrink: scale(self.space_shrink),
            x_height: scale(self.x_height),
            quad: scale(self.quad),
            extra_space: scale(self.extra_space),
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
    let words = 6 + lh as usize + count + nw as usize + nh as usize + nd as usize
        + ni as usize + nl as usize + nk as usize + ne as usize + np as usize;
    if words != lf as usize {
        return Err(TfmError(format!(
            "长度恒等式不符：{words} ≠ lf={lf}"
        )));
    }

    // 头部字：header[1] = 设计字号（fix_word，pt）
    let mut design_word: i64 = 0;
    for i in 0..lh {
        let w = r.u32()? as i32 as i64;
        if i == 1 {
            design_word = w;
        }
    }
    // design_sp = design_pt × 2^16 = (word / 2^20) × 2^16 = word / 16
    let design_size_sp = design_word / 16;
    // fix_word（设计字号单位）→ sp（四舍五入）
    let scale = |v: i64| (v * design_size_sp + (1 << 19)) >> 20;

    // 字符信息表：(ec - bc + 1) 个 32 位字
    let mut char_info: Vec<(usize, usize, usize)> = Vec::with_capacity(count);
    for _ in 0..count {
        let w = r.u32()?;
        let width_index = ((w >> 24) & 0xFF) as usize;
        let height_index = ((w >> 20) & 0x0F) as usize;
        let depth_index = ((w >> 16) & 0x0F) as usize;
        char_info.push((width_index, height_index, depth_index));
    }

    // 维度表（fix_word）：widths / heights / depths / italics
    let widths = read_words(&mut r, nw)?;
    let heights = read_words(&mut r, nh)?;
    let depths = read_words(&mut r, nd)?;
    let _italics = read_words(&mut r, ni)?;
    // 跳过连字/字距程序与可扩展表（各 32 位字）
    r.skip(nl as usize * 4)?;
    r.skip(nk as usize * 4)?;
    r.skip(ne as usize * 4)?;
    // 参数表（fix_word）
    let params = read_words(&mut r, np)?;

    // 字符度量组装
    let mut chars = vec![None; 256];
    for (i, &(wi, hi, di)) in char_info.iter().enumerate() {
        let charcode = bc as usize + i;
        if charcode < chars.len() {
            let width = widths.get(wi).map(|&w| scale(w)).unwrap_or(0);
            let height = heights.get(hi).map(|&h| scale(h)).unwrap_or(0);
            let depth = depths.get(di).map(|&d| scale(d)).unwrap_or(0);
            chars[charcode] = Some((width, height, depth));
        }
    }
    // 参数：TeX 参数 1=slant 2=space 3=space_stretch 4=space_shrink
    //       5=x_height 6=quad 7=extra_space（params[0] 即参数 1）
    let p = |i: usize| params.get(i).map(|&v| scale(v)).unwrap_or(0);
    Ok(FontMetrics {
        design_size_sp,
        chars,
        slant: p(0),
        space: p(1),
        space_stretch: p(2),
        space_shrink: p(3),
        x_height: p(4),
        quad: p(5),
        extra_space: p(6),
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
        for sub in ["/texmf-dist/fonts/tfm/public/cm/", "/texmf/fonts/tfm/public/cm/"] {
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
    }

    #[test]
    fn scaled_by_scales_all_dims() {
        let fm = parse_tfm(&synthetic_tfm()).expect("解析合成 TFM");
        // scaled 1200 → 1.2×：设计 10pt → 12pt；A 宽 375000 → 450000
        let s = fm.scaled_by(1200, 1000);
        assert_eq!(s.design_size_sp, 12 * 65_536);
        assert_eq!(s.char_metrics(65), (450_000, 300_000, 75_000));
        assert_eq!(s.space, 225_000);
        assert_eq!(s.quad, 525_000);
        // scaled 1000 → 恒等
        assert_eq!(fm.scaled_by(1000, 1000), fm);
        // scaled 0 → 全零
        let z = fm.scaled_by(0, 1000);
        assert_eq!(z.design_size_sp, 0);
        assert_eq!(z.char_metrics(65), (0, 0, 0));
    }
}
