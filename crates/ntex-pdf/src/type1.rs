//! Type1 字体（PFB）嵌入（M8 输出后端）。
//!
//! cmr10 等 CM 字体以 PFB（二进制）发行。PDF `/FontFile` 允许直接嵌入
//! PFB 格式的 Type1 字体程序，查看器自行 eexec 解密渲染——本层只负责
//! 定位 .pfb 文件与提取 `/FontName`（供 `/BaseFont` 使用），不解密密文。
//!
//! 注意：PFB 段头为 `0x80 类型(1) 长度(2, LE) 数据`，但部分发行版的段长
//! 与实际数据不完全一致，因此**不做段级重组**，原样嵌入最稳妥。
//!
//! 多字体（M8）：CM 全家族（cmr/cmbx/cmss/cmtt/cmmi/cmsy/cmex 等多号数）
//! 均走同一条查找链；`/BaseFont` 取自 PFB 内 `/FontName`（如 cmbx10 → CMBX10）。

use std::io::{self, ErrorKind};

/// 解析出的 Type1 字体（PFB 字节，可直接作 PDF `/FontFile` 流）。
pub struct Type1Font {
    /// 字体名（`/FontName`，如 "CMR10"）。
    pub name: String,
    /// 原始 PFB 字节（PDF `/FontFile` 流内容）。
    pub pfb: Vec<u8>,
}

/// 在 TeX 树中查找 Type1 字体文件（`<name>.pfb`）。
/// 查找链与 `ntex-font::find_tfm` 对齐：环境变量 NTEX_TYPE1_DIR → NTEX_TFM_DIR
/// → 备用根目录 → kpsewhich。
///
/// 环境变量取**对称双入口**而非并入 NTEX_TFM_DIR：PFB 是二进制字形程序、
/// TFM 是文本度量，发行布局不同目录（`fonts/type1` vs `fonts/tfm`），
/// 独立变量才能各自指向正确目录；未设时退回 NTEX_TFM_DIR 保持与旧部署
/// （cmr10.pfb/tfm 同目录）兼容。
///
/// 备用根目录兜底的取舍：查找链中 kpathsea（kpsewhich）在精简环境可能
/// 缺失、TeX Live 常规安装路径也可能不存在，此时扫描本仓库备料目录
/// `/tmp/latexsurvey` 是唯一无需用户配置即可拿到 CM 全家族 PFB 的路径，
/// 代价是每个未命中字体多几次 `exists()` 探测（字体数 ≤ 数十，可忽略）。
pub fn find_pfb(name: &str) -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let file = format!("{name}.pfb");
    for var in ["NTEX_TYPE1_DIR", "NTEX_TFM_DIR"] {
        if let Ok(dir) = std::env::var(var) {
            let p = PathBuf::from(dir).join(&file);
            if p.exists() {
                return Some(p);
            }
        }
    }
    // CM Type1 字体常见安装位置（texlive texmf-dist / texmf）
    const ROOTS: [&str; 3] = [
        "/usr/local/texlive/2024basic",
        "/usr/local/texlive/2023",
        "/usr/share/texlive",
    ];
    for root in ROOTS {
        for sub in [
            "/texmf-dist/fonts/type1/public/amsfonts/cm/",
            "/texmf-dist/fonts/type1/public/cm-super/",
            "/texmf/fonts/type1/public/amsfonts/cm/",
        ] {
            let p = PathBuf::from(root).join(sub).join(&file);
            if p.exists() {
                return Some(p);
            }
        }
    }
    // 备用根兜底：本仓库备料树（含 CM 全家族 75 个 PFB）与工作区本地字体暂存
    // 目录（.gitignore；环境 HOME 只读时经 `make fixture-extras` 备料说明拷入）。
    const SURVEY_ROOTS: [&str; 2] = [
        "/tmp/latexsurvey/fonts/type1/public/amsfonts/cm",
        ".local-fonts",
    ];
    for root in SURVEY_ROOTS {
        let p = PathBuf::from(root).join(&file);
        if p.exists() {
            return Some(p);
        }
    }
    // kpsewhich 直接查 .pfb（覆盖 TFM 同目录、texmf 树等）
    let out = std::process::Command::new("kpsewhich")
        .arg(&file)
        .output()
        .ok()?;
    if out.status.success() {
        let p = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if !p.is_empty() {
            let pb = PathBuf::from(p);
            if pb.exists() {
                return Some(pb);
            }
        }
    }
    None
}

/// 读取 PFB 并提取字体名（PFB 的 ASCII 段内含 `/FontName /XXX def`）。
pub fn load_pfb(name: &str) -> io::Result<Type1Font> {
    let path = find_pfb(name).ok_or_else(|| {
        io::Error::new(
            ErrorKind::NotFound,
            format!("找不到 Type1 字体：{name}.pfb"),
        )
    })?;
    let bytes = std::fs::read(&path)?;
    let font_name = extract_font_name(&bytes).unwrap_or_else(|| name.to_ascii_uppercase());
    Ok(Type1Font {
        name: font_name,
        pfb: bytes,
    })
}

/// 从 PFB 的 ASCII 段提取 `/FontName /XXX def`（TeX 字体名大写化兜底）。
fn extract_font_name(pfb: &[u8]) -> Option<String> {
    // 取首个 0x80 段头之后、下一个段头之前的 ASCII 文本
    let head = pfb.iter().position(|&b| b == 0x80)? + 4;
    let ascii_end = pfb[head..]
        .iter()
        .position(|&b| b == 0x80)
        .map_or(pfb.len(), |p| head + p);
    let text = std::str::from_utf8(&pfb[head..ascii_end]).ok()?;
    let marker = "/FontName /";
    let start = text.find(marker)? + marker.len();
    let rest = &text[start..];
    let name = rest.split(|c: char| c.is_whitespace() || c == '/').next()?;
    Some(name.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_cm_family_pfb_via_fallback_roots() {
        // M8 多字体：CM 全家族至少 6 族可经查找链定位（NTEX_*_DIR 未设、
        // 无 TeX Live / kpsewhich 时备料兜底根生效——CI 与本地一致）。
        for name in [
            "cmr10", "cmbx10", "cmss12", "cmtt10", "cmmi10", "cmsy10", "cmex10",
        ] {
            let path = std::env::var_os("NTEX_TYPE1_DIR")
                .or_else(|| std::env::var_os("NTEX_TFM_DIR"))
                .map(|dir| std::path::PathBuf::from(dir).join(format!("{name}.pfb")))
                .filter(|p| p.exists())
                .or_else(|| find_pfb(name));
            assert!(
                path.is_some(),
                "{name}.pfb 应能被 find_pfb 定位（备料根或环境目录）"
            );
        }
    }

    #[test]
    fn loads_real_cm_pfbs_with_font_name() {
        // 真实 PFB：/FontName 提取正确（大写），字节原样保留（供 /FontFile 嵌入）。
        for (name, expected) in [
            ("cmr10", "CMR10"),
            ("cmbx10", "CMBX10"),
            ("cmtt10", "CMTT10"),
        ] {
            let Ok(font) = load_pfb(name) else {
                eprintln!("未找到 {name}.pfb，跳过");
                continue;
            };
            assert_eq!(font.name, expected, "{name} 的 /FontName 应为 {expected}");
            assert_eq!(&font.pfb[..2], &[0x80, 1], "PFB 头应原样（0x80 01）");
        }
    }

    #[test]
    fn extracts_font_name_from_pfb_ascii_segment() {
        // 构造：0x80 01 + len + "/FontName /TestFoo def\n"
        let ascii = b"/FontName /TestFoo def\n";
        let mut pfb = vec![0x80, 1];
        pfb.extend((ascii.len() as u16).to_le_bytes());
        pfb.extend_from_slice(ascii);
        // 追加一段 0x80 02 二进制尾
        pfb.push(0x80);
        pfb.push(2);
        pfb.extend(0u16.to_le_bytes());
        assert_eq!(
            extract_font_name(&pfb).as_deref(),
            Some("TestFoo"),
            "应从 PFB ASCII 段提取 /FontName"
        );
    }

    #[test]
    fn font_name_falls_back_to_uppercase() {
        let pfb = vec![0x80, 1, 0, 0]; // 无 /FontName
        assert_eq!(
            extract_font_name(&pfb),
            None,
            "无 /FontName 时应返回 None（调用方大写化兜底）"
        );
    }
}
