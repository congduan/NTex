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
//!
//! 字体字节有两个来源，**注册表优先**：
//! 1. [`register_pfb`] 的进程级注入（wasm/浏览器唯一来源；native 下覆盖环境
//!    字体，用于测试与打包分发）——见 `ntex-wasm` 的 `set_pfb_font`；
//! 2. 宿主文件系统查找链（[`find_pfb`]：环境变量 → TeX Live 路径 → 备料目录
//!    → `kpsewhich`）。**该链整段为 native 专属**：wasm32-unknown-unknown 既
//!    无文件系统（`std::fs` 一律 `Err`）也无子进程（`std::process` 未实现），
//!    故 wasm 侧不走到 `kpsewhich`，未注入即 [`load_pfb`] 直接返回 `Err`。

use std::collections::HashMap;
use std::io::{self, ErrorKind};
use std::sync::{LazyLock, Mutex};

/// PFB 首段头魔数（`0x80 0x01` = ASCII 段）。注册时用作最低限度格式校验：
/// 非 PFB 字节嵌进 `/FontFile` 只会让查看器静默渲染失败，不如当场拒绝。
const PFB_MAGIC: [u8; 2] = [0x80, 0x01];

/// 进程级 PFB 字节注册表：[`register_pfb`] 写入，[`load_pfb`] 优先命中
/// （早于宿主文件系统查找）。
static REGISTRY: LazyLock<Mutex<HashMap<String, Vec<u8>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 注册 Type1 字体字节（`tex_name` 为 TeX 字体名，如 `cmr10`——须与 DVI
/// `fnt_def` 的外部名一致）。
///
/// 应用场景：宿主无文件系统时（wasm/浏览器）前端 fetch `<name>.pfb` 后注入，
/// 之后 [`load_pfb`] 即按名字命中；native 端用于免 TeX Live 环境的打包分发与
/// 测试。同名重复注册为覆盖（含覆盖宿主查找结果）。
///
/// 字节段头不符（非 PFB，如误传 OTF/PFA）返回 `false`，调用方据此报错——
/// 不要静默降级：不嵌字体的 PDF 在多数查看器里渲染不出内容。
pub fn register_pfb(tex_name: &str, bytes: &[u8]) -> bool {
    if !bytes.starts_with(&PFB_MAGIC) {
        return false;
    }
    match REGISTRY.lock() {
        Ok(mut m) => {
            m.insert(tex_name.to_owned(), bytes.to_vec());
            true
        }
        // 锁毒化：持锁线程已 panic，注册按失败处理（不传播错误，引擎契约）。
        Err(_) => false,
    }
}

/// 取已注册的 PFB 字节（不触碰宿主文件系统）。
pub fn registered_pfb(tex_name: &str) -> Option<Vec<u8>> {
    REGISTRY.lock().ok()?.get(tex_name).cloned()
}

/// 解析出的 Type1 字体（PFB 字节，可直接作 PDF `/FontFile` 流）。
pub struct Type1Font {
    /// 字体名（`/FontName`，如 "CMR10"）。
    pub name: String,
    /// 原始 PFB 字节（PDF `/FontFile` 流内容）。
    pub pfb: Vec<u8>,
    /// 内建编码表：码位 → 字形名（下标即码位，0..=255；`None` = 未定义）。
    /// 取自 PFB ASCII 段的 `/Encoding 256 array … dup N /Name put` 程序；
    /// 内建 `StandardEncoding` 的 TS1 族（cm-super `sfrm*`/`sfti*` 等）则
    /// 回落到 [`TS1_ENCODING`]。写出端据此生成 `/Encoding << /Differences … >>`
    /// ——实测 CoreGraphics 对无 `/Encoding` 的 Type1 按 StandardEncoding 兜底，
    /// TeX 编码里落在控制区的字形（cmsy10 的 `\cdot`=0x01、bullet=0x0F）会画成
    /// notdef；TS1 槽位布局同样与 StandardEncoding 不同（引号、剑标等全错位）。
    pub encoding: Vec<Option<String>>,
}

impl Type1Font {
    /// 由 PFB 字节构造：名字取 PFB 内 `/FontName`，缺失时按 TeX 名大写兜底。
    /// 两条来源（注册表/文件系统）共用，保证 `/BaseFont` 命名口径一致。
    fn from_pfb(tex_name: &str, pfb: Vec<u8>) -> Self {
        let name = extract_font_name(&pfb).unwrap_or_else(|| tex_name.to_ascii_uppercase());
        let mut encoding = extract_encoding(&pfb);
        // cm-super 的 TS1 字形程序（sfrm1095 等）内建 `/Encoding StandardEncoding
        // def`，没有 `dup` 程序可提取；TS1 槽位与 StandardEncoding 不同，必须按
        // cm-super-ts1.enc 补 `/Differences`，否则引号/剑标/角度括号等全错位。
        if encoding.iter().all(Option::is_none) && is_ts1_family(tex_name) {
            encoding = ts1_encoding();
        }
        Self {
            name,
            pfb,
            encoding,
        }
    }
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
/// 缺失、TeX Live 常规安装路径也可能不存在，此时扫描仓库随行资产
/// `assets/type1`（26 件最小集，见其 README）是唯一无需用户配置即可拿到
/// CM 系 PFB 的路径，代价是每个未命中字体多几次 `exists()` 探测（字体数
/// ≤ 数十，可忽略）。仓库根同时按构建期 `CARGO_MANIFEST_DIR` 与 CWD 推出，
/// 覆盖 CLI（仓库根运行）与 `cargo test`（crate 目录运行）两种入口。
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
    // 备用根兜底：
    // 1. 同一资产目录的构建期绝对路径——`cargo test` 的 CWD 是 crate 目录，
    //    仅此路径可用（二进制搬到别处后自动失效，由后续兜底接管）；**先于**
    //    CWD 相对路径，避免从仓库外运行时误中同名 `assets/type1`；
    // 2. 仓库随行资产 `assets/type1`（CWD 为仓库根，`make`/CI/README 入口均如此）；
    // 3. 工作区本地字体暂存目录（.gitignore；HOME 只读时按 `make fixture-extras`
    //    说明拷入）；
    // 4. 调研期临时备料树。
    let repo_assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/type1");
    for root in [
        repo_assets,
        PathBuf::from("assets/type1"),
        PathBuf::from(".local-fonts"),
        PathBuf::from("/tmp/latexsurvey/fonts/type1/public/amsfonts/cm"),
    ] {
        let p = root.join(&file);
        if p.exists() {
            return Some(p);
        }
    }
    // kpsewhich 直接查 .pfb（覆盖 TFM 同目录、texmf 树等）
    find_pfb_via_kpsewhich(&file)
}

/// `kpsewhich <name>.pfb` 兜底查找（**native 专属**）。
///
/// wasm 侧整段不可用：wasm32-unknown-unknown 的 `std::process` 无实现（起子进程
/// 必然失败且可能直接 trap），故 wasm 分支恒 `None`——字体字节只能经
/// [`register_pfb`] 注入。两侧函数签名一致，调用点无需 cfg。
#[cfg(not(target_arch = "wasm32"))]
fn find_pfb_via_kpsewhich(file: &str) -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let out = std::process::Command::new("kpsewhich")
        .arg(file)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if p.is_empty() {
        return None;
    }
    let pb = PathBuf::from(p);
    pb.exists().then_some(pb)
}

#[cfg(target_arch = "wasm32")]
fn find_pfb_via_kpsewhich(_file: &str) -> Option<std::path::PathBuf> {
    None
}

/// 读取 PFB 并提取字体名（PFB 的 ASCII 段内含 `/FontName /XXX def`）。
///
/// 查找顺序：注册表（[`register_pfb`]）→ 宿主文件系统链（native 专属）。两者
/// 都未命中返回 `Err`——注意 `pdf::build_document` 对 `Err` 是**降级**处理
/// （`/BaseFont` 保留但不嵌字体程序），调用方若在意可用性应自查
/// [`registered_pfb`] 而非依赖此处报错。
pub fn load_pfb(name: &str) -> io::Result<Type1Font> {
    if let Some(bytes) = registered_pfb(name) {
        return Ok(Type1Font::from_pfb(name, bytes));
    }
    read_host_pfb(name).map(|bytes| Type1Font::from_pfb(name, bytes))
}

/// 读宿主文件系统里的 PFB（查找链见 [`find_pfb`]）。
#[cfg(not(target_arch = "wasm32"))]
fn read_host_pfb(name: &str) -> io::Result<Vec<u8>> {
    let path = find_pfb(name).ok_or_else(|| {
        io::Error::new(
            ErrorKind::NotFound,
            format!("找不到 Type1 字体：{name}.pfb"),
        )
    })?;
    std::fs::read(&path)
}

/// wasm32：无文件系统可用，字体字节只能经 [`register_pfb`] 注入。
#[cfg(target_arch = "wasm32")]
fn read_host_pfb(name: &str) -> io::Result<Vec<u8>> {
    Err(io::Error::new(
        ErrorKind::NotFound,
        format!("找不到 Type1 字体：{name}.pfb（wasm 无文件系统，须先 register_pfb 注入字节）"),
    ))
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

/// TS1（Cork Text Companion）编码向量：槽位 → PFB 字形名（`.notdef` = 空槽）。
///
/// 逐字节照抄 cm-super 0.3.4 的 `dvips/cm-super-ts1.enc`（GPL-2.0-or-later +
/// 字体嵌入例外）；同包 `dvips/cm-super-ts1.map` 的对应行即
/// `tcrm1095 SFRM1095 "TS1Encoding ReEncodeFont" <cm-super-ts1.enc <sfrm1095.pfb`
/// ——TS1 字形程序内建 StandardEncoding，必须由驱动按本向量重编码。
/// 128 个非 `.notdef` 名已逐个对照 `afm/sfrm1095.afm` 的 `N` 项，无缺项。
///
/// EC「TC」族前缀清单与 `ntex-backend/src/glyphs.rs::is_ts1_family` 同源
/// （该表是渲染侧 TS1 判定的单一事实源，此处只为 PDF 侧复写一份前缀集合）。
/// 表按源文件 8 项一行排布，`rustfmt::skip` 保住与 `.enc` 的逐行对应。
#[rustfmt::skip]
const TS1_ENCODING: [&str; 256] = [
    "Grave", "Acute", "Circumflex", "Tilde", "Dieresis", "Hungarumlaut", "Ring", "Caron",
    "Breve", "Macron", "Dotaccent", "cedilla", "ogonek", "quotesinglbase.ts1", ".notdef", ".notdef",
    ".notdef", ".notdef", "quotedblbase.ts1", ".notdef", ".notdef", "twelveudash", "threequartersemdash", "afii61664.cap",
    "arrowleft", "arrowright", "tieaccentlowercase", "tieaccentcapital", "tieaccentlowercase.new", "tieaccentcapital.new", ".notdef", "afii61664.asc",
    "uni2422", ".notdef", ".notdef", ".notdef", "dollar", ".notdef", ".notdef", "quotesingle",
    ".notdef", ".notdef", "asteriskmath", ".notdef", "comma", "hyphendbl", "period", "fraction",
    "zerooldstyle", "oneoldstyle", "twooldstyle", "threeoldstyle", "fouroldstyle", "fiveoldstyle", "sixoldstyle", "sevenoldstyle",
    "eightoldstyle", "nineoldstyle", ".notdef", ".notdef", "angleleft", "minus", "angleright", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", "uni2127", ".notdef", "circle",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", "Omega",
    ".notdef", ".notdef", ".notdef", "uni301A", ".notdef", "uni301B", "arrowup", "arrowdown",
    "grave.ts1", ".notdef", "born", "divorced", "died", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", "leaf", "married", "musicalnote", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", "tildelow", "hyphendbl.alt",
    "breve.ts1", "caron.ts1", "hungarumlaut.ts1", "dblgrave.ts1", "dagger", "daggerdbl", "uni2016", "perthousand",
    "bullet", "uni2103", "dollaroldstyle", "centoldstyle", "florin", "colonmonetary", "uni20A9", "uni20A6",
    "guarani", "peso", "lira", "recipe", "uni203D", "gnaborretni", "dong", "trademark",
    "uni2031", "paragraph.alt", "uni0E3F", "afii61352", "discount", "estimated", "openbullet", "uni2120",
    "uni2045", "uni2046", "cent", "sterling", "currency", "yen", "brokenbar", "section",
    "dieresis.ts1", "copyright", "ordfeminine", "copyleft", "logicalnot", "uni2117", "registered", "macron.ts1",
    "degree", "plusminus", "twosuperior", "threesuperior", "acute.ts1", "mu", "paragraph", "periodcentered",
    "uni203B", "onesuperior", "ordmasculine", "radical", "onequarter", "onehalf", "threequarters", "Euro",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", "multiply", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", "divide", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
];

/// TS1 族 TeX 名判定：剥掉末尾尺寸数字（`tcrm1095` → `tcrm`）后对照前缀表。
fn is_ts1_family(tex_name: &str) -> bool {
    let family = tex_name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    matches!(
        family,
        "tcrm"
            | "tcsl"
            | "tcti"
            | "tcbx"
            | "tcrb"
            | "tcbi"
            | "tcbl"
            | "tcui"
            | "tcss"
            | "tcsi"
            | "tcsx"
            | "tcso"
            | "tctt"
            | "tcst"
            | "tcit"
            | "tcvt"
            | "tcvi"
    )
}

/// [`TS1_ENCODING`] → 与 [`extract_encoding`] 同形的码位表。
fn ts1_encoding() -> Vec<Option<String>> {
    TS1_ENCODING
        .iter()
        .map(|g| (*g != ".notdef").then(|| (*g).to_owned()))
        .collect()
}

/// 从 PFB ASCII 段解析 `/Encoding` 的 `dup N /Name put` 序列。
///
/// CM 系列 PFB 的写法固定为 `/Encoding 256 array 0 1 255 {…put} for` 后跟
/// 逐项 `dup N /Name put`；非此形态（如 `StandardEncoding def`）返回全
/// `None`，调用方跳过 `/Differences`。容忍截断/乱序：解析到多少算多少。
fn extract_encoding(pfb: &[u8]) -> Vec<Option<String>> {
    let mut out = vec![None; 256];
    let Some(text) = ascii_segment(pfb) else {
        return out;
    };
    let Some(enc_start) = text.find("/Encoding") else {
        return out;
    };
    // 编码程序体：从 /Encoding 起到收束的 `def`（`readonly def` 或行首独立
    // `def`）。**不能用子串查找 "def"**——CM 字体的填充循环里就有 `/.notdef`，
    // 会把编码块错切一半（2026-09-13 圆点缺失现场：dup 序列全被丢弃）。
    let body_full = &text[enc_start..];
    let body_end = body_full
        .find("readonly def")
        .or_else(|| body_full.find("\ndef"))
        .unwrap_or(body_full.len());
    let body = &body_full[..body_end];
    let mut search = 0;
    while let Some(rel) = body[search..].find("dup ") {
        let p = search + rel + 4;
        let rest = &body[p..];
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        // 完整形态是三 token：`<code> /<name> put`——要求 `put` 收尾，
        // 避免把别处 `dup` 用法（如 `dict dup begin`）误当编码项
        if tokens.len() >= 3 && tokens[2] == "put" {
            if let (Ok(code), Some(glyph)) =
                (tokens[0].parse::<usize>(), tokens[1].strip_prefix('/'))
            {
                if code < 256 && !glyph.is_empty() {
                    out[code] = Some(glyph.to_owned());
                }
            }
        }
        search = p;
    }
    out
}

/// PFB 首个 ASCII 段的文本（无则 `None`）。
fn ascii_segment(pfb: &[u8]) -> Option<&str> {
    let head = pfb.iter().position(|&b| b == 0x80)? + 4;
    let ascii_end = pfb[head..]
        .iter()
        .position(|&b| b == 0x80)
        .map_or(pfb.len(), |p| head + p);
    std::str::from_utf8(&pfb[head..ascii_end]).ok()
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
    fn extracts_real_cm_encoding_despite_notdef_trap() {
        // 回归（2026-09-13）：CM PFB 的填充循环 `{ 1 index exch /.notdef put} for`
        // 里 `/.notdef` 含子串 "def"——旧实现按首个 "def" 截断，编码块被错切
        // 一半，`/Encoding /Differences` 整个丢失，CoreGraphics 把 TeX 控制
        // 区字形（如 cmsy10 的 \bullet=0x0F）按 StandardEncoding 画成错形。
        // 用真实 cmsy10.pfb 验证（机器上有备料则跑真件，否则用等形构造件）。
        if let Ok(font) = load_pfb("cmsy10") {
            assert_eq!(
                font.encoding[0].as_deref(),
                Some("minus"),
                "cmsy10 码位 0 应为 /minus"
            );
            assert_eq!(
                font.encoding[0x0F].as_deref(),
                Some("bullet"),
                "cmsy10 码位 0x0F 应为 /bullet（\\item 圆点）"
            );
        }
        // 构造件：与 CM 形态逐字一致，不依赖环境备料
        let ascii = b"/FontName /CMXX10 def\n\
                      /Encoding 256 array\n\
                      0 1 255 { 1 index exch /.notdef put} for\n\
                      dup 0 /minus put\n\
                      dup 15 /bullet put\n\
                      readonly def\n";
        let mut pfb = vec![0x80, 1];
        pfb.extend((ascii.len() as u16).to_le_bytes());
        pfb.extend_from_slice(ascii);
        let font = Type1Font::from_pfb("cmxx10", pfb);
        assert_eq!(font.encoding[0].as_deref(), Some("minus"));
        assert_eq!(font.encoding[15].as_deref(), Some("bullet"));
        assert!(font.encoding[1].is_none(), "未列码位应保持 None");
    }

    #[test]
    fn tcrm1095_is_cm_super_sfrm_with_ts1_encoding() {
        // 回归（2026-11-02）：先前把 cm-super 的 SFTC1095（TCSC，打字机
        // 大小写）当 tcrm1095 入库；官方 `cm-super-ts1.map` 指定
        // `tcrm1095 → SFRM1095`，且 TS1 字形程序内建 StandardEncoding，
        // 必须补 cm-super-ts1.enc 的 `/Differences`。
        let Ok(font) = load_pfb("tcrm1095") else {
            eprintln!("未找到 tcrm1095.pfb，跳过");
            return;
        };
        assert_eq!(font.name, "SFRM1095", "/FontName 应为 sfrm1095 的 SFRM1095");
        assert_eq!(
            font.encoding[0x27].as_deref(),
            Some("quotesingle"),
            "TS1 0x27 = 直单引号"
        );
        assert_eq!(
            font.encoding[0x3D].as_deref(),
            Some("minus"),
            "TS1 0x3D = 减号"
        );
        assert_eq!(
            font.encoding[0x84].as_deref(),
            Some("dagger"),
            "TS1 0x84 = 剑标"
        );
        assert_eq!(font.encoding[0x85].as_deref(), Some("daggerdbl"));
        assert_eq!(font.encoding[0x00].as_deref(), Some("Grave"));
        assert!(
            font.encoding[0x0E].is_none(),
            "TS1 空槽应保持 notdef（不写 Differences）"
        );
        // 字母槽在 TS1 里是空槽：切不可回落到 StandardEncoding 的字形名。
        assert!(font.encoding[0x41].is_none(), "TS1 0x41 不是 'A'");
    }

    #[test]
    fn ts1_family_detection_and_table_shape() {
        assert!(is_ts1_family("tcrm1095"));
        assert!(is_ts1_family("tcti1000"));
        assert!(is_ts1_family("tctt0800"));
        assert!(!is_ts1_family("cmr10"), "OT1 CM 不是 TS1");
        assert!(!is_ts1_family("ecrm1000"), "T1 EC 不是 TS1");
        assert_eq!(TS1_ENCODING.len(), 256);
        assert_eq!(
            TS1_ENCODING.iter().filter(|g| **g != ".notdef").count(),
            128,
            "cm-super-ts1.enc 的 128 个实槽"
        );
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

    /// 造一段最小可解析 PFB（ASCII 段带 `/FontName`，尾部二进制段）。
    fn fake_pfb(font_name: &str) -> Vec<u8> {
        let ascii = format!("/FontName /{font_name} def\n");
        let mut pfb = vec![0x80, 1];
        pfb.extend((ascii.len() as u16).to_le_bytes());
        pfb.extend_from_slice(ascii.as_bytes());
        pfb.extend_from_slice(&[0x80, 2, 0, 0]);
        pfb
    }

    #[test]
    fn registered_bytes_take_priority_and_are_used_verbatim() {
        // 名字取唯一值：宿主查找链（TeX Live / kpsewhich）必然找不到它，
        // 于是「命中」只可能来自注册表——否则本测试会因环境碰巧有同名 PFB
        // 而假通过。
        let name = "zz-registry-probe10";
        assert!(
            find_pfb(name).is_none(),
            "前置假设：{name}.pfb 不在宿主查找链上"
        );
        let bytes = fake_pfb("ZzRegistryProbe10");
        assert!(register_pfb(name, &bytes), "合法 PFB 段头应注册成功");
        assert_eq!(registered_pfb(name).as_deref(), Some(&bytes[..]));
        let f = load_pfb(name).expect("注册后 load_pfb 应命中注册表");
        assert_eq!(f.name, "ZzRegistryProbe10", "/FontName 由注册字节提取");
        assert_eq!(f.pfb, bytes, "字节须原样保留（/FontFile 逐字节嵌入）");
    }

    #[test]
    fn register_rejects_bytes_without_pfb_header() {
        // 误传 OTF/PFA/空 一律拒绝，避免把无效字节写进 /FontFile
        // （查看器会静默渲染不出内容，比当场报错难查得多）。
        assert!(!register_pfb("zz-probe-otf10", b"\x00\x01\x00\x00OTTO"));
        assert!(!register_pfb("zz-probe-pfa10", b"%!PS-AdobeFont-1.0"));
        assert!(!register_pfb("zz-probe-empty10", &[]));
        assert!(registered_pfb("zz-probe-otf10").is_none(), "拒绝的不入库");
        assert!(
            load_pfb("zz-probe-otf10").is_err(),
            "未注册 + 宿主无此字体 → Err（调用方据此提示）"
        );
    }

    #[test]
    fn registered_bytes_overwrite_host_lookup() {
        // native 同名覆盖：注入一份假 cmr17，load_pfb 必须返回注入字节而非
        // TeX Live 里的真 cmr17.pfb——这是「打包分发/测试不必装 TeX」的基础。
        //
        // 字体名取 cmr17（仓库内无人引用）而非 cmr10：注册表是**进程级**的，
        // 测试并行共用同一进程，覆盖真名会污染 `build_document` 那几条逐字节
        // 校验 /FontFile 的用例（先跑注册的用例会让它们拿到假字节）。
        let name = "cmr17";
        if find_pfb(name).is_none() {
            eprintln!("宿主无 {name}.pfb，跳过「覆盖宿主」断言（不影响注册表逻辑）");
            return;
        }
        let injected = fake_pfb("CMR17Injected");
        assert!(register_pfb(name, &injected));
        let f = load_pfb(name).expect("注册后不应再走文件系统");
        assert_eq!(f.name, "CMR17Injected", "应命中注入字节，而非宿主 PFB");
        assert_eq!(f.pfb, injected);
    }
}
