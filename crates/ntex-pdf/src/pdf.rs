//! PDF 写出器（M8 输出后端）：DVI 页面 → PDF 1.4 文档。
//!
//! - 每页一个内容流：同行字符按基线聚成一个 `TJ` 数组（同 dvipdfmx 的整行串，
//!   词内间隙为 0，避免 poppler 等提取器在逐字符定位下误判出幻影空格）；
//!   词间空隙插入空格字形并用调整量把后续字符精确落在 TFM 位置；
//! - 规则用 `re f` 填充矩形；
//! - 字体：TFM 8-bit 字体走 Type1 嵌入（PFB 流，见 [`crate::type1`]），
//!   `/Encoding` 不指定——查看器用字体程序内建编码（cmr10 的 TeX 编码 =
//!   DVI 字符码，天然一致）；`unicode_native` 字体（中文 Fandol 等，M9）
//!   走 Type0/CIDFontType0 + `/FontFile3 /CIDFontType0C`（裸 CFF）
//!   （见 [`crate::otf`]），内容流字符写成两字节十六进制串（Identity-H），
//!   **串值 = 字体 CFF charset 里的真 CID**（Unicode →（cmap）→ GID →
//!   （charset）→ CID，见 [`crate::cid`]；Fandol 即 Adobe-GB1 CID——写
//!   Unicode 会让查看器 CID→字形查表落空、整页中文空白，2026-09-13 修复）。
//!   同时按**排版器自己用的度量**写 `/W` 宽度数组：查看器推进量与排版器
//!   一致，逐字调整量才落得准；字体未覆盖的码位跳过字形（不画也不推进）。
//!
//! - 多字体去重按 `fnt_def` 外部名（同字体多页复用同一组对象）；页面资源字典
//!   只列本页实际引用的字体（`page_fonts`），跨页互不泄漏（M8 多字体验证）。
//! - 坐标转换：DVI 原点在页左上、y 向下（sp）；PDF 原点在左下、y 向上（pt），
//!   字符参考点为基线（DVI 的 v 即基线）。

use std::collections::BTreeMap;
use std::io::{self, Write};

use crate::cid::{self, CidMap, Mapping};
use crate::dvi::{DrawOp, Dvi};
use crate::otf::load_otf;
use crate::type1::load_pfb;

/// DVI 单位：1pt = 65536sp（mag=1000）。
const SP_PER_PT: f64 = 65_536.0;

/// DVI 原点偏移：DVI 坐标 (0,0) 对应页面左上 (1in, 1in)（TeX 的 \hoffset/
/// \voffset 默认 0 即 1in 边距，dvipdfmx/dvips 同口径）。换算成页面坐标时
/// 水平 +72pt、垂直（自页顶）+72pt。
const ORIGIN_PT: f64 = 72.0;

/// 转换参数。
pub struct PdfOptions {
    /// 页面尺寸（pt）：宽、高。
    pub page_size: (f64, f64),
    /// 图片搜索路径（图片管线 Step B）：`\includegraphics` 的图源按文件名
    /// 在这些目录下找回（与 ntex-dvi 的 `--input-path` 同口径；空名 = cwd）。
    pub input_paths: Vec<String>,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            // 纸张缺省 US letter（8.5×11in = 612×792bp）：TeX 世界的缺省纸
            // （pdftexconfig.tex 的 \pdfpagewidth 8.5in、dvips `-t letter` 缺省、
            // article.cls `\ExecuteOptions{letterpaper,...}`）。LaTeX 文档不带
            // paper 选项时 pdfTeX 落 letter（GT 实测 612×792）；DVI 本身无纸张
            // 信息，驱动缺省即文档纸面。
            page_size: (612.0, 792.0), // US letter
            input_paths: Vec::new(),
        }
    }
}

/// `ntex-image` special 解析出的图片引用（DVI 载荷 → 嵌入素材）。
struct ImageSpec {
    /// 显示宽/高（sp；核心侧已乘 scale= 等缩放）。
    w_sp: i64,
    h_sp: i64,
    /// 图源文件名（`\pdfximage` 登记名，随载荷长度前缀携带）。
    name: String,
}

/// 已解码图片的全局登记（同文件多页复用同一 XObject 对象）。
struct ImageEntry {
    /// 像素宽/高（PDF Image XObject 的 /Width /Height）。
    width: u32,
    height: u32,
    /// DeviceRGB 样本流（已 FlateDecode 压缩，见 [`crate::image`]）。
    rgb: Vec<u8>,
    /// DeviceGray 软掩码流（已压缩）。None = 不透明，无需 /SMask。
    smask: Option<Vec<u8>>,
}

/// 解析 `ntex-image <w_sp> <h_sp> <名长> <名>` 载荷；非本协议载荷 → None
/// （普通 \special 透传忽略，与既有口径一致）。
fn parse_image_special(payload: &[u8]) -> Option<ImageSpec> {
    let text = std::str::from_utf8(payload).ok()?;
    let rest = text.strip_prefix("ntex-image ")?;
    let mut it = rest.splitn(3, ' ');
    let w_sp = it.next()?.parse().ok()?;
    let h_sp = it.next()?.parse().ok()?;
    // 名长前缀：名字可含空格，按前缀给出的字节数截取
    let tail = it.next()?;
    let (len_s, name) = tail.split_once(' ')?;
    let len: usize = len_s.parse().ok()?;
    let name = name.get(..len)?;
    Some(ImageSpec {
        w_sp,
        h_sp,
        name: name.to_owned(),
    })
}

/// 按名取回并解码图片（全局去重：同文件只解码一次）。找不到/不支持 → Err
/// （调用方降级跳图 + 警告，不 panic）。
fn ensure_image(
    spec: &ImageSpec,
    input_paths: &[String],
    images: &mut Vec<ImageEntry>,
    index: &mut BTreeMap<String, usize>,
) -> Result<usize, String> {
    if let Some(&i) = index.get(&spec.name) {
        return Ok(i);
    }
    let path = if std::path::Path::new(&spec.name).is_absolute() {
        Some(std::path::PathBuf::from(&spec.name))
    } else {
        input_paths
            .iter()
            .map(|d| std::path::Path::new(d).join(&spec.name))
            .find(|p| p.exists())
    };
    let Some(path) = path else {
        return Err(format!("文件未找到（搜索路径 {input_paths:?}）"));
    };
    let data = std::fs::read(&path).map_err(|e| format!("读取 {}：{e}", path.display()))?;
    let decoded =
        crate::image::decode_png(&data).map_err(|e| format!("{}：{e}", path.display()))?;
    let idx = images.len();
    images.push(ImageEntry {
        width: decoded.width,
        height: decoded.height,
        rgb: decoded.rgb,
        smask: decoded.smask,
    });
    index.insert(spec.name.clone(), idx);
    Ok(idx)
}

/// 把解析出的 DVI 写成 PDF 字节。
pub fn write_pdf(dvi: &Dvi, opts: &PdfOptions) -> io::Result<Vec<u8>> {
    let (_, h_pt) = opts.page_size;
    // 字体形态只分类一次：写内容流（要按 CID 映射取字节）与组装对象字典
    // （要按同一形态定对象数）必须同源，否则两边各说一套
    let forms = classify_fonts(dvi);
    // 内容流实际写出的 CID：字体号 →（CID → 宽度，1/1000 em），供 `/W`。
    // BTreeMap 而非 HashMap：写出顺序确定，PDF 可复现 diff。
    let mut used: Vec<BTreeMap<u16, i64>> = vec![BTreeMap::new(); dvi.fonts.len()];
    let mut contents: Vec<Vec<u8>> = Vec::with_capacity(dvi.pages.len());
    // 图片管线 Step B：跨页全局图登记（同文件复用同一 XObject）+ 每页首次
    // 引用序（资源字典按页列、内容流按 /Im<序> 引用）。
    let mut images: Vec<ImageEntry> = Vec::new();
    let mut image_index: BTreeMap<String, usize> = BTreeMap::new();
    let mut page_images: Vec<Vec<usize>> = vec![Vec::new(); dvi.pages.len()];
    for (page_i, page) in dvi.pages.iter().enumerate() {
        let mut c = Vec::new();
        // 当前行 run：连续、同字体、同基线的字符（(font, code, x_pt, y_pt)；
        // code 全宽 u32，Unicode 字体的码位可达 0x10FFFF）
        let mut run: Vec<(u32, u32, f64, f64)> = Vec::new();
        for op in &page.ops {
            match op {
                DrawOp::Char { font, code, h, v } => {
                    // DVI 原点在页面 (1in,1in)：水平 +72pt；y 自页顶量起再翻转
                    let x = ORIGIN_PT + *h as f64 / SP_PER_PT;
                    let y = h_pt - ORIGIN_PT - *v as f64 / SP_PER_PT;
                    match run.last() {
                        Some(&(pf, _, _, py)) if pf != *font || (py - y).abs() > 0.001 => {
                            emit_line(&mut c, dvi, &run, &forms, &mut used)?;
                            run.clear();
                        }
                        _ => {}
                    }
                    run.push((*font, *code, x, y));
                }
                DrawOp::Rule {
                    h,
                    v,
                    width,
                    height,
                } => {
                    emit_line(&mut c, dvi, &run, &forms, &mut used)?;
                    run.clear();
                    let x = ORIGIN_PT + *h as f64 / SP_PER_PT;
                    let w = *width as f64 / SP_PER_PT;
                    let y = h_pt - ORIGIN_PT - *v as f64 / SP_PER_PT;
                    let hgt = *height as f64 / SP_PER_PT;
                    if w > 0.0 && hgt > 0.0 {
                        writeln!(c, "{:.4} {:.4} {:.4} {:.4} re f", x, y, w, hgt)?;
                    }
                }
                DrawOp::Special { h, v, payload } => {
                    emit_line(&mut c, dvi, &run, &forms, &mut used)?;
                    run.clear();
                    let Some(spec) = parse_image_special(payload) else {
                        continue; // 非图片载荷：透传忽略（xxx 原口径）
                    };
                    match ensure_image(&spec, &opts.input_paths, &mut images, &mut image_index) {
                        Ok(gi) => {
                            // 页内首次引用序 = /Im 名（1 起）
                            let local = match page_images[page_i].iter().position(|&x| x == gi) {
                                Some(p) => p + 1,
                                None => {
                                    page_images[page_i].push(gi);
                                    page_images[page_i].len()
                                }
                            };
                            // whatsit 的 DVI 当前点 = 图左下角（核心侧 \pdfrefximage
                            // 在占位盒参考点发 special）；图自锚点向右上铺显示宽高
                            let x = ORIGIN_PT + *h as f64 / SP_PER_PT;
                            let y = h_pt - ORIGIN_PT - *v as f64 / SP_PER_PT;
                            let wp = spec.w_sp as f64 / SP_PER_PT;
                            let hp = spec.h_sp as f64 / SP_PER_PT;
                            if wp > 0.0 && hp > 0.0 {
                                writeln!(
                                    c,
                                    "q {wp:.4} 0 0 {hp:.4} {x:.4} {y:.4} cm /Im{local} Do Q"
                                )?;
                            }
                        }
                        Err(e) => {
                            eprintln!(
                                "[ntex-pdf] 警告：图片 `{}' 未嵌入（{e}）——版面占位保留、图区空白",
                                spec.name
                            );
                        }
                    }
                }
            }
        }
        emit_line(&mut c, dvi, &run, &forms, &mut used)?;
        contents.push(c);
    }

    build_document(
        dvi,
        &contents,
        &forms,
        &used,
        opts,
        &images,
        &page_images,
    )
}

/// 写一行字符（一个 `TJ` 数组）。
///
/// `Tm` 定基线起点（取本行**第一个画得出的**字形位置）；此后每遇到下一个
/// 已画字形都发一个调整量（`TJ` 数字：正数左移、负数右移，单位 1/1000 em），
/// 把笔位移到该字形的排版位置。词间空隙因此自然表现为一个大调整量
/// （poppler 等提取器据此识别为空格，同 dvipdfmx 的做法）。
///
/// 调整量以**上一个已画字形**的宽度为基准，所以被跳过的字形（字体未覆盖的
/// 码位）既不画、也不推进笔位——落点仍精确，不会因缺字形把后续字符整体推移。
///
/// 与 `/W` 的自洽性：调整量成立的前提是「查看器推进量 == 排版器推进量」，
/// 故 [`write_pdf`] 按同一份度量把用到的每个 CID 写进 `/W` 宽度数组。
///
/// 字体形态分叉：TFM 8-bit 字体写 `(...)` 字面串（DVI 码 = 内建编码）；
/// `unicode_native` 字体写 `<XXXX>` 两字节十六进制串（Identity-H），串值取
/// [`crate::cid`] 给出的**字体真 CID**（不是 Unicode 码位）。
fn emit_line(
    c: &mut Vec<u8>,
    dvi: &Dvi,
    run: &[(u32, u32, f64, f64)],
    forms: &FontForms,
    used: &mut [BTreeMap<u16, i64>],
) -> io::Result<()> {
    if run.is_empty() {
        return Ok(());
    }
    let font = run[0].0;
    let y0 = run[0].3;
    let fm = &dvi.fonts[font as usize];
    let size = font_size_pt(fm);
    // 字号非法（畸形 DVI 的 d/s 域）：本行整行不画——写出去会引入除零/NaN
    if !size.is_finite() || size <= 0.0 {
        return Ok(());
    }
    let cid_map = match forms.form(font) {
        Some(FontForm::Otf { cid, .. }) => cid.as_ref(),
        _ => None,
    };

    // 先筛出真正画得出的字形：Unicode 字体查 CID 映射，字体未覆盖的码位跳过
    let mut glyphs: Vec<(u32, f64, Option<u16>)> = Vec::with_capacity(run.len());
    for &(_, code, x, _) in run {
        if fm.unicode_native {
            let cid = match cid_map {
                // 正路：字体真 CID（见 [`crate::cid`]）
                Some(map) => map.cid(code),
                // 无映射（字体未注入/映射构建失败）：保留降级旧口径——把码位当
                // CID 直写（> 0xFFFF 表不成 2 字节，跳过），查看器按 `/BaseFont`
                // 名以本地字体替代时或有可显示之机；这是最后手段，非正路。
                None => (code <= 0xFFFF).then_some(code as u16),
            };
            match cid {
                Some(cid) => glyphs.push((code, x, Some(cid))),
                None => continue, // 画不出：不画也不推进（不进 glyphs）
            }
        } else {
            glyphs.push((code, x, None));
        }
    }
    let Some(&(_, x0, _)) = glyphs.first() else {
        return Ok(()); // 整行无可画字形（如全落在字体 cmap 覆盖之外）
    };
    write!(
        c,
        "BT /F{} {:.6} Tf 1 0 0 1 {:.4} {:.4} Tm [",
        font + 1,
        size,
        x0,
        y0
    )?;

    let mut prev_code = glyphs[0].0;
    let mut prev_x = x0;
    for (i, &(code, x, cid)) in glyphs.iter().enumerate() {
        if i > 0 {
            let w_prev = fm.char_metrics(prev_code).0 as f64 / SP_PER_PT;
            let delta = x - prev_x;
            let adj = (w_prev - delta) * 1000.0 / size;
            if adj.abs() > 0.01 {
                write!(c, " {:.2}", adj)?;
            }
        }
        match cid {
            Some(cid) => write!(c, "<{cid:04X}>")?,
            // TFM 字体的 DVI 码恒 ≤ 0xFF（TeX 8-bit 语义）
            None => write!(c, "({})", escape_byte(code as u8))?,
        }
        if let (Some(cid), Some(entry)) = (cid, used.get_mut(font as usize)) {
            entry.entry(cid).or_insert_with(|| width_1000(fm, code));
        }
        prev_code = code;
        prev_x = x;
    }
    writeln!(c, "] TJ ET")?;
    Ok(())
}

/// 字体宽度的 1/1000 em 表示（PDF `/W` 口径），与排版器给的 sp 宽度同源。
///
/// 「查看器推进量 == 排版器推进量」是逐字调整量落得准的前提：两者不同源时，
/// 每画一个字都会累积一次（PDF 宽度 − 排版宽度）的偏移。
fn width_1000(fm: &ntex_font::FontMetrics, code: u32) -> i64 {
    let size_sp = (fm.design_size_sp as i128 * fm.scale as i128) >> 20;
    if size_sp <= 0 {
        return 1000; // 防御：尺度非法时退全角（与 /DW 同值）
    }
    let w = i128::from(fm.char_metrics(code).0) * 1000 / size_sp;
    w.clamp(0, i128::from(i64::MAX)) as i64
}

/// 字体的实际字号（pt）：design × scale / 2^20（`scaled_by` 后的 scale 字段）。
fn font_size_pt(fm: &ntex_font::FontMetrics) -> f64 {
    let sp = (fm.design_size_sp as i128 * fm.scale as i128) >> 20;
    sp as f64 / SP_PER_PT
}

/// PDF 字符串转义（字节直出；`( ) \` 与 <0x20、>=0x7F 转八进制）。
fn escape_byte(b: u8) -> String {
    match b {
        b'(' => "\\(".to_owned(),
        b')' => "\\)".to_owned(),
        b'\\' => "\\\\".to_owned(),
        c if !(0x20..0x7F).contains(&c) => format!("\\{:03o}", c),
        c => (c as char).to_string(),
    }
}

/// PDF 字面字符串（`(...)`，用于 `/CIDSystemInfo` 的 ROS 名）内容转义。
fn escape_pdf_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '(' | ')' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// `/W` 宽度数组：`[ cid [w] cid [w] … ]`（单 CID 用紧凑形 `[w]`）。
///
/// 只列内容流**实际写出**的 CID——未出现的 CID 由 `/DW`（1000，全角）兜底。
/// 取的是排版器自己用的那份度量（见 [`width_1000`]），宽度口径与 `/DW` 一致
/// （1/1000 em）。空表写 `[]`（`/W` 允许空数组，等价于全用 `/DW`）。
fn width_array(used: Option<&BTreeMap<u16, i64>>) -> String {
    let Some(map) = used else {
        return "[]".to_owned();
    };
    let mut s = String::from("[");
    for (i, (cid, w)) in map.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{cid} [{w}]"));
    }
    s.push(']');
    s
}

/// 一族字体的嵌入形态。
enum FontForm {
    /// Type1：PFB 原样进 `/FontFile`（`/BaseFont` 取 PFB 内 `/FontName`，大写）；
    /// `/Widths` 按 TFM 度量逐码位列出（缺它查看器推进为 0，字形叠架）；
    /// `/Encoding /Differences` 按 PFB 内建编码显式声明（缺它 CoreGraphics
    /// 按 StandardEncoding 兜底，TeX 编码控制区的字形画成 notdef）。
    Type1 {
        name: String,
        pfb: Vec<u8>,
        /// 码位 0..=255 的推进宽（1/1000 em）。
        widths: Vec<u16>,
        /// 码位 0..=255 的字形名（PFB 内建编码，`None` = 未定义）。
        encoding: Vec<Option<String>>,
    },
    /// Type0：裸 CID-keyed CFF 进 `/FontFile3`（`/Subtype /CIDFontType0C`，
    /// 见 [`crate::cid::bare_cff`]）。
    /// `cid` = Unicode→真 CID 映射（见 [`crate::cid`]）；`None` 表示 CFF 解析
    /// 失败——此时内容流画不出任何字形，退回不嵌入降级以免写错映射。
    Otf { cff: Vec<u8>, cid: Option<CidMap> },
    /// 不嵌入降级：`unicode=true` 仍给 Type0+后代字典（无 `/FontFile3`），
    /// `false` 退最小裸 Type1 字典（M8 行为）。
    Bare { unicode: bool },
}

/// 去重后的一族字体。
struct FontEntry {
    /// DVI `fnt_def` 外部名（去重键，也是 `/F<n>` 资源名与 CSS 名之外的引用键）。
    tex_name: String,
    /// PDF `/BaseFont` 名（Type1 取 PFB 内 `/FontName`；Type0 用 DVI 名原样）。
    base_name: String,
    /// 嵌入形态。
    form: FontForm,
}

/// 全文的字体分类结果。
struct FontForms {
    /// 每个唯一字体名一项（跨页/多号数复用同一组 PDF 对象）。
    uniq: Vec<FontEntry>,
    /// DVI 字体号 → [`Self::uniq`] 下标。
    of_font: Vec<usize>,
}

impl FontForms {
    /// 按 DVI 字体号取嵌入形态。
    fn form(&self, font_id: u32) -> Option<&FontForm> {
        self.of_font
            .get(font_id as usize)
            .and_then(|&i| self.uniq.get(i))
            .map(|e| &e.form)
    }
}

impl FontForm {
    /// 本形态占用的 PDF 对象数（供对象号分配）。
    fn object_count(&self) -> u32 {
        match self {
            FontForm::Type1 { .. } => 3,
            FontForm::Otf { .. } => 4,
            FontForm::Bare { unicode: true } => 2,
            FontForm::Bare { unicode: false } => 1,
        }
    }
}

/// 给每个 DVI 字体定嵌入形态（去重与告警按外部名，每个名字只报一次）。
///
/// 判定：`unicode_native` 度量（M9 中文）走 Type0——OTF 字节按 [`load_otf`]
/// 定位（注册表 → 宿主查找链）原样嵌入，并构建 CID 映射（见 [`crate::cid`]；
/// 映射构建失败或非 CFF 轮廓时降级不嵌）。其余走 Type1/PFB。
fn classify_fonts(dvi: &Dvi) -> FontForms {
    let mut uniq: Vec<FontEntry> = Vec::new();
    let mut of_font: Vec<usize> = Vec::with_capacity(dvi.fonts.len());
    for (name, fm) in dvi.font_names.iter().zip(&dvi.fonts) {
        // 同名字体（多号数/多页）复用首次结论：不重复加载字体、不重复告警
        if let Some(pos) = uniq.iter().position(|e| e.tex_name == *name) {
            of_font.push(pos);
            continue;
        }
        let entry = if fm.unicode_native {
            classify_unicode(name)
        } else {
            classify_type1(name, fm)
        };
        of_font.push(uniq.len());
        uniq.push(entry);
    }
    FontForms { uniq, of_font }
}

/// Unicode 直映字体：Type0 + 真 CID 映射；失败则如实告警并降级。
fn classify_unicode(name: &str) -> FontEntry {
    let bare = |base: String| FontEntry {
        tex_name: name.to_owned(),
        base_name: base,
        form: FontForm::Bare { unicode: true },
    };
    let bytes = match load_otf(name) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("警告：{e}（以不嵌入方式引用字体）");
            return bare(name.to_owned());
        }
    };
    if !bytes.starts_with(b"OTTO") {
        // TrueType 轮廓（glyf）：/OpenType 流只收 CFF，挂账不嵌
        eprintln!("警告：{name} 是 TrueType 轮廓，OpenType 嵌入暂不支持（以不嵌入方式引用）");
        return bare(name.to_owned());
    }
    let cid = match cid::build(&bytes) {
        Ok(m) => {
            if m.mapping() == Mapping::GlyphIdFallback {
                // name-keyed CFF：charset 存的是 SID 而非 CID，PDF 侧造不出真
                // 映射，只能退 CID = GID 的兼容口径（部分查看器可显示）——挂账
                eprintln!(
                    "警告：{name} 的 CFF 非 CID-keyed（无 ROS），PDF CID 映射退化为 \
                     CID = GID（部分查看器可能显示不出）"
                );
            }
            Some(m)
        }
        Err(e) => {
            eprintln!("警告：{e}（{name} 以不嵌入方式引用）");
            return bare(name.to_owned());
        }
    };
    // 剥掉 sfnt 壳只嵌裸 CFF（/CIDFontType0C）：整包 OTTO 以 /OpenType 嵌入
    // 时 poppler/CoreGraphics 会把 CID 解析到错误字形（见 [`cid::bare_cff`]）
    let cff = match cid::bare_cff(&bytes) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("警告：{e}（{name} 以不嵌入方式引用）");
            return bare(name.to_owned());
        }
    };
    FontEntry {
        tex_name: name.to_owned(),
        base_name: name.to_owned(),
        form: FontForm::Otf { cff, cid },
    }
}

/// TFM 8-bit 字体：Type1/PFB；找不到 PFB 则退裸字典（`/BaseFont` 用大写兜底名）。
///
/// `widths_thousandths`：逐码位推进宽（1/1000 em 口径，与 TFM 度量同源）。
/// 实测 poppler/CoreGraphics 在 Type1 无 `/Widths` 时把字形推进当 0 处理，
/// 全行字形叠架成一团（resume 的 `\tt` 行与列表圆点即此症状），故必须写。
fn type1_widths(fm: &ntex_font::FontMetrics) -> Vec<u16> {
    let size_pt = font_size_pt(fm);
    (0..=255u32)
        .map(|code| {
            let w_pt = fm.char_metrics(code).0 as f64 / SP_PER_PT;
            (w_pt / size_pt * 1000.0).round() as u16
        })
        .collect()
}

fn classify_type1(name: &str, fm: &ntex_font::FontMetrics) -> FontEntry {
    match load_pfb(name) {
        Ok(f) => FontEntry {
            tex_name: name.to_owned(),
            base_name: f.name.clone(),
            form: FontForm::Type1 {
                name: f.name,
                pfb: f.pfb,
                widths: type1_widths(fm),
                encoding: f.encoding,
            },
        },
        Err(e) => {
            eprintln!("警告：{e}（以不嵌入方式引用字体）");
            FontEntry {
                tex_name: name.to_owned(),
                base_name: name.to_ascii_uppercase(),
                form: FontForm::Bare { unicode: false },
            }
        }
    }
}

/// 组装 PDF 文档。
///
/// 对象布局：1 Catalog、2 Pages、每页 (Page, Contents) 两对象、
/// 之后按字体分配（对象数随形态而定，见 [`FontForm::object_count`]）。
///
/// 字体形态由 [`classify_fonts`] 决定（与内容流写字节同一份结论）：Type0
/// 分支除 Type0/后代/描述符/`/FontFile3` 四对象外，还按字体 ROS 写
/// `/CIDSystemInfo`、按内容流实际用到的 CID 写 `/W` 宽度数组。
fn build_document(
    dvi: &Dvi,
    contents: &[Vec<u8>],
    forms: &FontForms,
    used: &[BTreeMap<u16, i64>],
    opts: &PdfOptions,
    images: &[ImageEntry],
    page_images: &[Vec<usize>],
) -> io::Result<Vec<u8>> {
    // 每族字体字典的对象号（去重表下标 → 对象号）+ DVI 字体号 → 对象号；
    // 图片对象排在全部字体之后（图片管线 Step B；对象号只需唯一，段序无关）。
    let n_pages = contents.len();
    let (nums, font_obj): (Vec<u32>, Vec<u32>) = {
        let mut nums = Vec::with_capacity(forms.uniq.len());
        let mut cursor = (3 + 2 * n_pages) as u32;
        for e in &forms.uniq {
            nums.push(cursor);
            cursor += e.form.object_count();
        }
        let font_obj = forms
            .of_font
            .iter()
            .map(|&idx| nums.get(idx).copied().unwrap_or(cursor))
            .collect();
        (nums, font_obj)
    };
    // 图片对象号：每个 Image 一个对象，含软掩码再加一个（/SMask 引用对象）。
    let mut img_obj: Vec<u32> = Vec::with_capacity(images.len());
    {
        let font_total: u32 = forms.uniq.iter().map(|e| e.form.object_count()).sum();
        let mut cursor = (3 + 2 * n_pages) as u32 + font_total;
        for im in images {
            img_obj.push(cursor);
            cursor += 1 + u32::from(im.smask.is_some());
        }
    }

    let (w_pt, h_pt) = opts.page_size;
    let mut buf = Vec::new();
    let mut offsets = Vec::new();
    let obj = |buf: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(buf.len());
        buf.extend_from_slice(body);
        buf.push(b'\n');
    };

    buf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    obj(
        &mut buf,
        &mut offsets,
        b"1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj",
    );

    let mut kids = String::new();
    for i in 0..n_pages {
        if i > 0 {
            kids.push(' ');
        }
        kids.push_str(&format!("{} 0 R", 3 + 2 * i));
    }
    obj(
        &mut buf,
        &mut offsets,
        format!("2 0 obj << /Type /Pages /Kids [{kids}] /Count {n_pages} >> endobj").as_bytes(),
    );

    // 页面 + 内容流（资源字典只列本页用到的字体，避免跨页资源冲突/膨胀：
    // 例如 A 页 cmr10、B 页 cmtt10 时，双方 /Font 互不可见）。
    for (i, c) in contents.iter().enumerate() {
        let page_obj = 3 + 2 * i;
        let content_obj = 4 + 2 * i;
        let mut used = page_fonts(i, dvi);
        used.sort_unstable();
        used.dedup();
        let mut res = String::new();
        for f in used {
            res.push_str(&format!("/F{} {} 0 R ", f + 1, font_obj[f as usize]));
        }
        // /XObject：本页引用的图（资源按页列，与 /Font 同口径）
        let mut xobjs = String::new();
        for (p, &gi) in page_images[i].iter().enumerate() {
            xobjs.push_str(&format!("/Im{} {} 0 R ", p + 1, img_obj[gi]));
        }
        let xobj_dict = if xobjs.is_empty() {
            String::new()
        } else {
            format!("/XObject << {xobjs}>> ")
        };
        obj(
            &mut buf,
            &mut offsets,
            format!(
                "{page_obj} 0 obj << /Type /Page /Parent 2 0 R \
                 /MediaBox [0 0 {w_pt:.4} {h_pt:.4}] \
                 /Resources << /Font << {res}>> {xobj_dict}>> /Contents {content_obj} 0 R >> endobj"
            )
            .as_bytes(),
        );
        let mut body =
            format!("{content_obj} 0 obj << /Length {} >>\nstream\n", c.len()).into_bytes();
        body.extend_from_slice(c);
        body.extend_from_slice(b"\nendstream\nendobj");
        obj(&mut buf, &mut offsets, &body);
    }

    // 字体对象（去重后逐族写出）。
    // - Type1：/BaseFont 与 /FontDescriptor /FontName 统一取 PFB 内 /FontName
    //   （Type1 惯例大写，如 CMR10），而非 DVI 侧小写引用名（tex fnt_def 名是
    //   引擎内部引用键；PDF 名字必须与嵌入字体自声明名一致，否则部分查看器
    //   按名匹配度量失败）。fallback（无 PFB）路径同样走大写兜底名。
    // - Type0：/BaseFont 用 DVI 引用名原样（FandolSong-Regular 本身即合法
    //   PostScript 名）。/CIDSystemInfo 照抄字体 CFF 的 ROS（Fandol 即
    //   Adobe-GB1-5）；/W 按内容流实际用到的 CID 写宽度——与排版器度量同源，
    //   查看器推进量才与逐字调整量自洽（见 `emit_line` 文档）。
    for (idx, e) in forms.uniq.iter().enumerate() {
        let FontEntry {
            base_name, form, ..
        } = e;
        let base_obj = nums[idx];
        match form {
            FontForm::Type1 {
                name,
                pfb,
                widths,
                encoding,
            } => {
                let (dict_obj, desc_obj, file_obj) = (base_obj, base_obj + 1, base_obj + 2);
                let widths_str = widths
                    .iter()
                    .map(|w| w.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                // /Differences：仅列 PFB 内建编码给出的码位（逐项显式编号，
                // 容忍稀疏）；内建编码缺失（非标准 PFB）时省略整个键
                let diffs: String = encoding
                    .iter()
                    .enumerate()
                    .filter_map(|(code, g)| g.as_ref().map(|g| format!("{code} /{g}")))
                    .collect::<Vec<_>>()
                    .join(" ");
                let enc_key = if diffs.is_empty() {
                    String::new()
                } else {
                    format!(" /Encoding << /Differences [{diffs}] >>")
                };
                obj(
                    &mut buf,
                    &mut offsets,
                    format!(
                        "{dict_obj} 0 obj << /Type /Font /Subtype /Type1 /BaseFont /{name} \
                         /FirstChar 0 /LastChar 255 /Widths [{widths_str}]{enc_key} \
                         /FontDescriptor {desc_obj} 0 R >> endobj"
                    )
                    .as_bytes(),
                );
                obj(
                    &mut buf,
                    &mut offsets,
                    format!(
                        "{desc_obj} 0 obj << /Type /FontDescriptor /FontName /{name} \
                         /Flags 4 /ItalicAngle 0 /Ascent 0 /Descent 0 /CapHeight 0 /StemV 0 \
                         /FontFile {file_obj} 0 R >> endobj"
                    )
                    .as_bytes(),
                );
                let mut fbody =
                    format!("{file_obj} 0 obj << /Length {} >>\nstream\n", pfb.len()).into_bytes();
                fbody.extend_from_slice(pfb);
                fbody.extend_from_slice(b"\nendstream\nendobj");
                obj(&mut buf, &mut offsets, &fbody);
            }
            FontForm::Otf { cff, cid } => {
                let (dict_obj, cid_obj, desc_obj, file_obj) =
                    (base_obj, base_obj + 1, base_obj + 2, base_obj + 3);
                // CIDSystemInfo：如实照抄字体的 ROS；缺 ROS（退化映射）时保持
                // Identity（此时 CID = GID，Identity 正是对应口径）
                let info = match cid.as_ref().and_then(|m| m.ros()) {
                    Some(ros) => format!(
                        "/Registry ({}) /Ordering ({}) /Supplement {}",
                        escape_pdf_string(&ros.registry),
                        escape_pdf_string(&ros.ordering),
                        ros.supplement
                    ),
                    None => "/Registry (Adobe) /Ordering (Identity) /Supplement 0".to_owned(),
                };
                obj(
                    &mut buf,
                    &mut offsets,
                    format!(
                        "{dict_obj} 0 obj << /Type /Font /Subtype /Type0 /BaseFont /{base_name} \
                         /Encoding /Identity-H /DescendantFonts [{cid_obj} 0 R] >> endobj"
                    )
                    .as_bytes(),
                );
                // `/W`：同一字体名的**全部** DVI 字体号（多号数）用到的 CID 取并集。
                //
                // `used` 按 **DVI 字体号** 索引，而 `idx` 是**去重后 uniq 的下标**
                // ——同一名字的不同号数（如 Fandol 11pt/14.4pt/20.74pt）共享一个
                // PDF 字体对象，此前只取 `used.get(idx)`（首个号数的桶），其余号数
                // 用到的 CID 全部落进 `/DW 1000` 兜底：CJK 恰好也是全宽 1000 所以
                // 看不出来，ASCII 等非全宽字形则被按全角推进——查看器端表现为
                // 「字距被拉宽」（2026-09-17 现场：resume1-plain.tex 的
                // `138-XXXX-XXXX`、`Python`）。
                let mut widths_of_name: BTreeMap<u16, i64> = BTreeMap::new();
                for (font_id, &u) in forms.of_font.iter().enumerate() {
                    if u != idx {
                        continue;
                    }
                    if let Some(m) = used.get(font_id) {
                        for (&c, &w) in m {
                            widths_of_name.entry(c).or_insert(w);
                        }
                    }
                }
                obj(
                    &mut buf,
                    &mut offsets,
                    format!(
                        "{cid_obj} 0 obj << /Type /Font /Subtype /CIDFontType0 /BaseFont \
                         /{base_name} /CIDSystemInfo << {info} >> /FontDescriptor {desc_obj} 0 R \
                         /DW 1000 /W {} >> endobj",
                        width_array(Some(&widths_of_name))
                    )
                    .as_bytes(),
                );
                // Ascent/Descent 以 CJK 字体典型值占位（upem=1000 口径）——度量
                // 结构未存 OS/2 上下延；查看器行高推断以嵌入字体自带 hhea/OS2
                // 为准，此处仅兜底。
                obj(
                    &mut buf,
                    &mut offsets,
                    format!(
                        "{desc_obj} 0 obj << /Type /FontDescriptor /FontName /{base_name} \
                         /Flags 4 /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 0 \
                         /StemV 0 /FontFile3 {file_obj} 0 R >> endobj"
                    )
                    .as_bytes(),
                );
                let mut fbody = format!(
                    "{file_obj} 0 obj << /Subtype /CIDFontType0C /Length {} >>\nstream\n",
                    cff.len()
                )
                .into_bytes();
                fbody.extend_from_slice(cff);
                fbody.extend_from_slice(b"\nendstream\nendobj");
                obj(&mut buf, &mut offsets, &fbody);
            }
            // 不嵌入降级：Type0 族仍写完整结构（缺 FontFile3/FontDescriptor），
            // 查看器按 BaseFont 名以本地字体替代；TFM 族退最小裸字典（M8 行为）。
            FontForm::Bare { unicode: true } => {
                let (dict_obj, cid_obj) = (base_obj, base_obj + 1);
                obj(
                    &mut buf,
                    &mut offsets,
                    format!(
                        "{dict_obj} 0 obj << /Type /Font /Subtype /Type0 /BaseFont /{base_name} \
                         /Encoding /Identity-H /DescendantFonts [{cid_obj} 0 R] >> endobj"
                    )
                    .as_bytes(),
                );
                obj(
                    &mut buf,
                    &mut offsets,
                    format!(
                        "{cid_obj} 0 obj << /Type /Font /Subtype /CIDFontType0 /BaseFont \
                         /{base_name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) \
                         /Supplement 0 >> /DW 1000 >> endobj"
                    )
                    .as_bytes(),
                );
            }
            FontForm::Bare { unicode: false } => {
                obj(
                    &mut buf,
                    &mut offsets,
                    format!(
                        "{base_obj} 0 obj << /Type /Font /Subtype /Type1 /BaseFont /{base_name} >> endobj"
                    )
                    .as_bytes(),
                );
            }
        }
    }

    // 图片 XObject（图片管线 Step B）：FlateDecode 的 DeviceRGB 样本流
    // （解码期已完成 PNG unfilter 与白底合成，无需 PDF 预测器参数）；
    // 含 alpha 的源另挂 DeviceGray 软掩码（透明度语义保留给查看器）。
    for (gi, im) in images.iter().enumerate() {
        let obj_num = img_obj[gi];
        let mut head = format!(
            "{obj_num} 0 obj << /Type /XObject /Subtype /Image /Width {} /Height {} \
             /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode",
            im.width, im.height
        );
        if im.smask.is_some() {
            head.push_str(&format!(" /SMask {} 0 R", obj_num + 1));
        }
        head.push_str(&format!(" /Length {} >>\nstream\n", im.rgb.len()));
        let mut body = head.into_bytes();
        body.extend_from_slice(&im.rgb);
        body.extend_from_slice(b"\nendstream\nendobj");
        obj(&mut buf, &mut offsets, &body);
        if let Some(sm) = &im.smask {
            let mut sb = format!(
                "{} 0 obj << /Type /XObject /Subtype /Image /Width {} /Height {} \
                 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode \
                 /Length {} >>\nstream\n",
                obj_num + 1,
                im.width,
                im.height,
                sm.len()
            )
            .into_bytes();
            sb.extend_from_slice(sm);
            sb.extend_from_slice(b"\nendstream\nendobj");
            obj(&mut buf, &mut offsets, &sb);
        }
    }

    // xref / trailer
    let xref_pos = buf.len();
    buf.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
    buf.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        buf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    buf.extend_from_slice(
        format!(
            "trailer << /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
            offsets.len() + 1
        )
        .as_bytes(),
    );
    Ok(buf)
}

/// 一页实际引用的 DVI 字体号集合（`DrawOp::Char` 的 font 字段）。
fn page_fonts(page: usize, dvi: &Dvi) -> Vec<u32> {
    dvi.pages[page]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Char { font, .. } => Some(*font),
            DrawOp::Rule { .. } | DrawOp::Special { .. } => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dvi::Page;
    use crate::type1;
    use ntex_font::parse_tfm;

    #[test]
    fn escapes_pdf_string_bytes() {
        assert_eq!(escape_byte(b'('), "\\(");
        assert_eq!(escape_byte(b')'), "\\)");
        assert_eq!(escape_byte(b'\\'), "\\\\");
        assert_eq!(escape_byte(b'a'), "a");
        assert_eq!(escape_byte(0x00), "\\000");
        assert_eq!(escape_byte(0xFF), "\\377");
    }

    /// 用真实 cmr10 度量构造 Dvi（10pt、未缩放），供内容流测试。
    fn test_dvi() -> Option<Dvi> {
        let path = ntex_font::find_tfm("cmr10")?;
        let bytes = std::fs::read(path).ok()?;
        let mut fm = parse_tfm(&bytes).ok()?;
        fm.name = "cmr10".to_owned();
        fm.design_size_sp = 655_360;
        fm.scale = 1 << 20; // 设计字号 10pt，未缩放
        Some(Dvi {
            pages: Vec::new(),
            fonts: vec![fm],
            font_names: vec!["cmr10".to_owned()],
        })
    }

    #[test]
    fn run_groups_chars_and_adjusts_word_gaps() {
        let Some(dvi) = test_dvi() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let fm = &dvi.fonts[0];
        let w_t = fm.char_metrics(b'T' as u32).0 as f64 / SP_PER_PT;
        let w_h = fm.char_metrics(b'h' as u32).0 as f64 / SP_PER_PT;
        // 一行：T h 空格 h（词距 5pt）——词距以负调整量表达
        let run1 = vec![
            (0, u32::from(b'T'), 0.0, 10.0),
            (0, u32::from(b'h'), w_t, 10.0),
            (0, u32::from(b'h'), w_t + w_h + 5.0, 10.0),
        ];
        let mut c = Vec::new();
        let forms = classify_fonts(&dvi);
        let mut used = vec![BTreeMap::new(); dvi.fonts.len()];
        emit_line(&mut c, &dvi, &run1, &forms, &mut used).unwrap();
        let s = String::from_utf8(c).unwrap();
        assert!(s.starts_with("BT /F1 10.000000 Tf 1 0 0 1 0.0000 10.0000 Tm ["));
        // 词内相邻字符无调整量（delta == 字符宽），词距产生负调整量 -500
        assert!(s.contains("(T)(h)"), "词内不应有调整量：{s}");
        assert!(s.contains(" -500.00"), "词距应有 -500.00 调整量：{s}");
        assert!(s.ends_with("] TJ ET\n"), "{s}");
    }

    #[test]
    fn write_pdf_splits_lines_by_baseline_and_rules() {
        let Some(mut dvi) = test_dvi() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let fm = &dvi.fonts[0];
        let w_t = fm.char_metrics(b'T' as u32).0 as f64 / SP_PER_PT;
        let page = Page {
            ops: vec![
                DrawOp::Char {
                    font: 0,
                    code: u32::from(b'T'),
                    h: 0,
                    v: 0,
                },
                DrawOp::Rule {
                    h: 100,
                    v: 0,
                    width: 200,
                    height: 100,
                },
                DrawOp::Char {
                    font: 0,
                    code: u32::from(b'T'),
                    h: 0,
                    v: 655_360, // 下一行
                },
            ],
        };
        dvi.pages = vec![page];
        let pdf = write_pdf(
            &dvi,
            &PdfOptions {
                page_size: (100.0, 20.0),
                ..Default::default()
            },
        )
        .unwrap();
        let s = String::from_utf8_lossy(&pdf);
        // 两条 TJ 行（规则前后各一）+ 一条规则
        assert_eq!(s.matches("] TJ ET").count(), 2, "{s}");
        assert!(s.contains("re f\n"));
        // 第二行基线 = 页高 - v = 20 - 10 = 10pt
        assert!(s.contains("Tm [("), "{s}");
        let _ = w_t;
    }

    /// DVI 原点偏移：DVI (0,0) 应画到页面 (1in,1in)（自页左/自页顶），
    /// 与 dvipdfmx 同口径。曾遗漏 +72pt 导致内容整体上移贴住页顶。
    #[test]
    fn tm_places_origin_at_one_inch() {
        let Some(mut dvi) = test_dvi() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        dvi.pages = vec![Page {
            ops: vec![DrawOp::Char {
                font: 0,
                code: u32::from(b'T'),
                h: 0,
                v: 0,
            }],
        }];
        // letter 纵向 11in = 792pt
        let pdf = write_pdf(
            &dvi,
            &PdfOptions {
                page_size: (612.0, 792.0),
                ..Default::default()
            },
        )
        .unwrap();
        let s = String::from_utf8_lossy(&pdf);
        // x = 72；y = 792 - 72 - 0 = 720
        assert!(s.contains("1 0 0 1 72.0000 720.0000 Tm"), "{s}");
    }

    /// 多字体（M8）：两页两字体（cmr10/cmtt10，缺任一度量则跳过）。
    /// 验证：/FontFile 逐字节等于本地 PFB；/BaseFont 为 PFB /FontName（大写）；
    /// 每页 /Resources /Font 只列本页实际引用的字体，跨页复用同一字典对象。
    #[test]
    fn write_pdf_embeds_multi_font_family_and_pages_use_own_fonts() {
        let mut fonts = Vec::new();
        let mut names = Vec::new();
        for name in ["cmr10", "cmtt10"] {
            let Some(path) = ntex_font::find_tfm(name) else {
                eprintln!("未找到 {name}.tfm，跳过");
                return;
            };
            let Ok(bytes) = std::fs::read(&path) else {
                eprintln!("读取 {name}.tfm 失败，跳过");
                return;
            };
            let Ok(mut fm) = parse_tfm(&bytes) else {
                eprintln!("解析 {name}.tfm 失败，跳过");
                return;
            };
            fm.name = name.to_owned();
            fm.design_size_sp = 655_360;
            fm.scale = 1 << 20;
            fonts.push(fm);
            names.push(name.to_owned());
        }
        // 页 1 用字体 0（cmr10）、页 2 用字体 1（cmtt10）——互不引用对方。
        let page = |font: u32| Page {
            ops: vec![DrawOp::Char {
                font,
                code: u32::from(b'T'),
                h: 0,
                v: 0,
            }],
        };
        let dvi = Dvi {
            pages: vec![page(0), page(1)],
            fonts,
            font_names: names,
        };
        let pdf = write_pdf(&dvi, &PdfOptions::default()).unwrap();
        let s = String::from_utf8_lossy(&pdf);

        // /BaseFont 来自 PFB /FontName（Type1 惯例大写）
        assert!(s.contains("/BaseFont /CMR10 "), "{s}");
        assert!(s.contains("/BaseFont /CMTT10 "), "{s}");
        // /Widths 必写（1/1000 em，与 TFM 度量同源）：缺它 poppler/CoreGraphics
        // 把字形推进当 0，整行字形叠架成一团（2026-09-13 resume \tt 行现场）
        assert_eq!(s.matches("/Widths [").count(), 2, "{s}");
        // cmr10 'T'（0x54）槽位的宽度 = TFM 推进折算千分数（与度量自洽）
        {
            let probe = test_dvi().expect("cmr10 度量");
            let fm0 = &probe.fonts[0];
            let w1000 = (fm0.char_metrics(u32::from(b'T')).0 as f64 / SP_PER_PT / font_size_pt(fm0)
                * 1000.0)
                .round() as u16;
            let widths = s
                .split("/Widths [")
                .nth(1)
                .unwrap()
                .split(']')
                .next()
                .unwrap();
            assert_eq!(
                widths.split_whitespace().nth(0x54),
                Some(&w1000.to_string()[..]),
                "cmr10 0x54 槽位应为 TFM 宽度 {w1000}：[{widths}]"
            );
        }
        // 两组 FontDescriptor + /FontFile（6 字体同机制，此处抽查 2 族）
        assert_eq!(s.matches("/FontFile ").count(), 2, "{s}");

        // /FontFile 流内容逐字节等于本地 PFB（原样嵌入，不重组）
        for (idx, name) in ["cmr10", "cmtt10"].iter().enumerate() {
            let pfb = type1::load_pfb(name).unwrap().pfb;
            let dict_obj = 7 + 3 * idx; // 2 页时 font_base = 7
            let file_obj = dict_obj + 2;
            let marker = format!("{file_obj} 0 obj << /Length {} >>\nstream\n", pfb.len());
            let pos = pdf
                .windows(marker.len())
                .position(|w| w == marker.as_bytes())
                .unwrap();
            let start = pos + marker.len();
            assert_eq!(
                &pdf[start..start + pfb.len()],
                &pfb[..],
                "{name} 应原样嵌入"
            );
        }

        // 页面资源字典：页 1 只有 /F1（cmr10 字典对象 7）、页 2 只有 /F2（对象 10）
        assert!(s.contains("/Resources << /Font << /F1 7 0 R >> >>"), "{s}");
        assert!(s.contains("/Resources << /Font << /F2 10 0 R >> >>"), "{s}");
    }

    /// Unicode 直映字体（M9 中文）：`unicode_native` 度量 + 注入的 OTF 字节
    /// → Type0/CIDFontType0 + Identity-H + /FontFile3（/OpenType 原样嵌入），
    /// 内容流字符写两字节十六进制（**CID = 字体 CFF charset 里的真 CID**，
    /// 不是 Unicode 码位），/CIDSystemInfo 照抄字体 ROS，/W 按用到的 CID 写宽。
    /// OTF 取仓库内 Tauri 前端自带的 FandolSong（找不到则跳过，不硬依赖）。
    ///
    /// 判据来源：xdvipdfmx 对**同一字体**写出的内容流为
    /// `[<11cf0ed30b8603f104a90d6b>…]`（中=0x11CF=4559=Adobe-GB1 CID），
    /// 本测试据此锁死「中」的 CID——写 Unicode 时查看器按 4559 反查落空，
    /// 中文整页空白（2026-09-13 修复的现场）。
    #[test]
    fn write_pdf_embeds_unicode_font_as_type0_opentype() {
        let otf_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ntex-tauri/ui/fonts/FandolSong-Regular.otf");
        let Ok(otf) = std::fs::read(&otf_path) else {
            eprintln!("未找到 {}，跳过", otf_path.display());
            return;
        };
        // 唯一注册名：不污染其他测试的字体注册表
        let name = "FandolSong-Type0Probe";
        assert!(crate::otf::register_otf(name, &otf), "OTTO 魔数应注册成功");

        let mut fm = ntex_font::FontMetrics {
            unicode_native: false,
            unicode_chars: Vec::new(),
            design_size_sp: 655_360,
            scale: 1 << 20,
            checksum: 0,
            name: name.to_owned(),
            chars: vec![None; 128],
            char_italic: vec![0; 128],
            slant: 0,
            space: 0,
            space_stretch: 0,
            space_shrink: 0,
            x_height: 0,
            quad: 655_360,
            extra_space: 0,
            lig_kern_steps: Vec::new(),
            kern_values: Vec::new(),
            lig_kern_index: Vec::new(),
            next_larger: Vec::new(),
            font_params: Vec::new(),
        };
        fm.unicode_native = true;
        // 全角宽 1 em（Fandol 全系 upem=1000，汉字前进宽度即 1000）
        for cp in [0x4E2Du32, 0x56FD] {
            fm.unicode_chars.push((cp, (655_360, 0, 0)));
        }
        fm.unicode_chars.sort_by_key(|e| e.0);

        let dvi = Dvi {
            pages: vec![Page {
                ops: vec![
                    DrawOp::Char {
                        font: 0,
                        code: 0x4E2D,
                        h: 0,
                        v: 0,
                    },
                    DrawOp::Char {
                        font: 0,
                        code: 0x56FD,
                        h: 655_360,
                        v: 0,
                    },
                ],
            }],
            fonts: vec![fm],
            font_names: vec![name.to_owned()],
        };
        let pdf = write_pdf(&dvi, &PdfOptions::default()).unwrap();
        let s = String::from_utf8_lossy(&pdf);

        // Type0 四对象组齐备
        assert!(s.contains("/Subtype /Type0"), "{s}");
        assert!(s.contains("/Subtype /CIDFontType0"), "{s}");
        assert!(s.contains("/Encoding /Identity-H"), "{s}");
        assert!(s.contains("/FontFile3"), "{s}");
        assert!(s.contains("/Subtype /CIDFontType0C /Length"), "{s}");
        assert!(
            !s.contains("/Subtype /OpenType"),
            "不得再以 OTTO 整包嵌入：{s}"
        );
        assert!(s.contains("/DW 1000"), "{s}");
        // CIDSystemInfo 照抄字体 CFF 的 ROS（FandolSong = Adobe-GB1-5）
        assert!(
            s.contains("/Registry (Adobe) /Ordering (GB1) /Supplement 5"),
            "ROS 应如实照抄：{s}"
        );
        // 内容流：真 CID——「中」= 0x11CF（GB1 CID 4559，与 xdvipdfmx 一致），
        // 「国」为该字体 charset 给出的另一个 CID（此处按映射自洽校验）
        assert!(s.contains("<11CF>"), "「中」应写 GB1 CID 0x11CF：{s}");
        let map = crate::cid::build(&otf).expect("构建 CID 映射");
        assert_eq!(map.cid(0x4E2D), Some(0x11CF), "「中」的 CID 应为 4559");
        let guo = map.cid(0x56FD).expect("「国」应在字体 cmap 内");
        assert!(
            s.contains(&format!("<{guo:04X}>")),
            "「国」应写其真 CID：{s}"
        );
        // 不应把 Unicode 码位当 CID 写（修复前正是这样，导致查看器查不到字形）
        assert!(!s.contains("<4E2D>"), "不得再写 Unicode 码位作 CID：{s}");
        // /W 按内容流实际用到的 CID 列宽（与排版器度量同源：全角 1000/1000 em；
        // 表按 CID 升序，故「国」(1875) 在「中」(4559) 前）
        assert_eq!(guo, 0x0753, "「国」的 CID 应为 1875（GB1 口径）");
        assert!(
            s.contains("/W [1875 [1000] 4559 [1000]]"),
            "应列用到的 CID 及其宽度：{s}"
        );
        // 不应再有 TFM 时代的八位字面串字形
        assert!(!s.contains("/Subtype /Type1"), "{s}");
        // 裸 CID-keyed CFF 嵌入（/CIDFontType0C）：流体 = sfnt 内 CFF 表原样，
        // 以裸 CFF 魔数（header major=1）开始；整包 OTTO 嵌入会让 poppler/
        // CoreGraphics 把 CID 解析到错误字形（2026-09-13 实验定标）
        let marker = format!("<< /Subtype /CIDFontType0C /Length {} >>\nstream\n", {
            let cff = crate::cid::bare_cff(&otf).unwrap();
            assert_ne!(cff.len(), otf.len(), "裸 CFF 应小于 sfnt 整包");
            cff.len()
        });
        let pos = pdf
            .windows(marker.len())
            .position(|w| w == marker.as_bytes())
            .expect("FontFile3 流头");
        let cff = crate::cid::bare_cff(&otf).unwrap();
        assert_eq!(
            &pdf[pos + marker.len()..pos + marker.len() + cff.len()],
            &cff[..],
            "裸 CFF 应原样嵌入"
        );
        assert_eq!(&cff[..2], &[0x01, 0x00], "CFF 头魔数");
    }

    /// 同一字体名的**多个号数**共用一组 PDF 对象时，`/W` 必须取全部号数用到
    /// 的 CID 的并集（`used` 按 DVI 字体号索引、PDF 字体对象按**名字**去重，
    /// 两者不同维）。
    ///
    /// 现场（2026-09-17，resume1-plain.tex）：Fandol 11pt/14.4pt/20.74pt 三个
    /// 号数——`/W` 只列了首个号数（标题）用到的 CID，其余全落 `/DW 1000` 兜底。
    /// CJK 恰好也是全宽 1000 所以看不出，ASCII（数字、`Python`）被按全角推进，
    /// 查看器端表现为「字距被拉宽」。
    ///
    /// 顺带锁住 `/W` 的**字号无关性**：同一字形在 11pt 与 10pt 两个号数下的
    /// 宽度值必须落在同一个数上（`width_1000` 以设计字号为分母）。
    #[test]
    fn width_array_unions_cids_of_all_sizes_sharing_one_font_name() {
        let otf_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ntex-tauri/ui/fonts/FandolSong-Regular.otf");
        let Ok(otf) = std::fs::read(&otf_path) else {
            eprintln!("未找到 {}，跳过", otf_path.display());
            return;
        };
        let name = "FandolSong-MultiSizeProbe";
        assert!(crate::otf::register_otf(name, &otf), "OTTO 魔数应注册成功");

        // 夹具必须走引擎同款路径：`scaled_by` 会**连同字符宽度一起**缩放，并把
        // 缩放比记进 `scale`（见 `ntex_font::FontMetrics::scaled_by`）。手写
        // 「宽度仍是设计值、只把 scale 改成 1.1」的夹具不忠实——那等于宣称
        // 「字号变大而字形没变大」，`width_1000`（宽度 / 缩放后字号）就会凭空
        // 差 1/scale（1000 → 909）。
        let mk = |unicode_chars: Vec<(u32, (i64, i64, i64))>| ntex_font::FontMetrics {
            unicode_native: true,
            unicode_chars,
            design_size_sp: 655_360,
            scale: 1 << 20,
            checksum: 0,
            name: name.to_owned(),
            chars: vec![None; 128],
            char_italic: vec![0; 128],
            slant: 0,
            space: 0,
            space_stretch: 0,
            space_shrink: 0,
            x_height: 0,
            quad: 655_360,
            extra_space: 0,
            lig_kern_steps: Vec::new(),
            kern_values: Vec::new(),
            lig_kern_index: Vec::new(),
            next_larger: Vec::new(),
            font_params: Vec::new(),
        };
        // 设计字号 10pt 下的宽度（sp）：汉字全角 1 em；'A' 取 3/8 em（375000sp）
        // ——非全宽字形才看得出 /W 是否按字号归一化（见下方 572 的断言）。
        let mut chars = vec![
            (0x4E2Du32, (655_360i64, 0i64, 0i64)), // 「中」
            (0x56FD, (655_360, 0, 0)),             // 「国」
            (0x41, (375_000, 0, 0)),               // 'A'
        ];
        chars.sort_by_key(|e| e.0);
        let base = mk(chars);
        // 号数 0 = 11pt（10pt 设计字号 × 1.1，`\font..at 11pt` 的口径）
        // 号数 1 = 10pt（未缩放）
        let fm_11 = base.scaled_by(11 * 65_536, 655_360);
        let fm_10 = base.clone();

        // 号数 0 画 'A' 与「中」，号数 1 画「国」：三者都要进同一份 /W。
        // y 各自不同 → 各自成为一个 run（run 以 (字体, 基线) 为界）。
        let at = |font, code, v| DrawOp::Char {
            font,
            code,
            h: 0,
            v,
        };
        let dvi = Dvi {
            pages: vec![Page {
                ops: vec![
                    at(0, 0x41, 0),
                    at(0, 0x4E2D, 655_360),
                    at(1, 0x56FD, 1_310_720),
                ],
            }],
            fonts: vec![fm_11, fm_10],
            font_names: vec![name.to_owned(), name.to_owned()],
        };
        let pdf = write_pdf(&dvi, &PdfOptions::default()).unwrap();
        let s = String::from_utf8_lossy(&pdf);
        assert_eq!(
            s.matches("/Subtype /Type0").count(),
            1,
            "同名字体应只写一组 PDF 对象：{s}"
        );

        let map = crate::cid::build(&otf).expect("构建 CID 映射");
        let cid_a = map.cid(u32::from(b'A')).expect("'A' 应在字体 cmap 内");
        let cid_zhong = map.cid(0x4E2D).expect("「中」应在字体 cmap 内");
        let cid_guo = map.cid(0x56FD).expect("「国」应在字体 cmap 内");
        assert_eq!(cid_zhong, 0x11CF, "「中」的 CID 应为 GB1 4559");
        assert_eq!(cid_guo, 0x0753, "「国」的 CID 应为 GB1 1875");

        // 期望宽度（1/1000 em，按 CID 升序——`/W` 的写出顺序）：
        // 「中」「国」全角 1000；'A' = 375000sp × 1000 / 655360sp = 572。
        // 572 是**字号无关**的：11pt 号数下宽度与字号同乘 1.1，商不变;
        // 若 /W 随字号漂移（错把缩放后字号当分母又没缩宽度），这里会是 520。
        let mut want: Vec<(u16, i64)> = vec![(cid_a, 572), (cid_guo, 1000), (cid_zhong, 1000)];
        want.sort_unstable();
        want.dedup();
        assert_eq!(want.len(), 3, "三个 CID 应互不相同：{want:?}");
        let want = format!(
            "/W [{}]",
            want.iter()
                .map(|(c, w)| format!("{c} [{w}]"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert!(
            s.contains(&want),
            "两个号数用到的 CID 都应进同一份 /W，且宽度按设计字号归一化（期望 {want}）：{s}"
        );
    }

    /// Unicode 字体未注入 OTF：降级为 Type0+后代字典（无 FontFile3），
    /// 内容流照写十六进制 CID（查看器按 BaseFont 名以本地字体替代）。
    #[test]
    fn unicode_font_without_otf_degrades_to_type0_bare_dict() {
        // 宿主查找链必然没有这个名字（注册表也未注册）
        let name = "zz-type0-bare-probe";
        let mut fm = ntex_font::FontMetrics {
            unicode_native: false,
            unicode_chars: Vec::new(),
            design_size_sp: 655_360,
            scale: 1 << 20,
            checksum: 0,
            name: name.to_owned(),
            chars: vec![None; 128],
            char_italic: vec![0; 128],
            slant: 0,
            space: 0,
            space_stretch: 0,
            space_shrink: 0,
            x_height: 0,
            quad: 655_360,
            extra_space: 0,
            lig_kern_steps: Vec::new(),
            kern_values: Vec::new(),
            lig_kern_index: Vec::new(),
            next_larger: Vec::new(),
            font_params: Vec::new(),
        };
        fm.unicode_native = true;
        fm.unicode_chars.push((0x4E2D, (1000 * 65536, 0, 0)));

        let dvi = Dvi {
            pages: vec![Page {
                ops: vec![DrawOp::Char {
                    font: 0,
                    code: 0x4E2D,
                    h: 0,
                    v: 0,
                }],
            }],
            fonts: vec![fm],
            font_names: vec![name.to_owned()],
        };
        let pdf = write_pdf(&dvi, &PdfOptions::default()).unwrap();
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.contains("/Subtype /Type0"), "{s}");
        assert!(s.contains("/Encoding /Identity-H"), "{s}");
        assert!(!s.contains("/FontFile3"), "降级不应带字体流：{s}");
        assert!(s.contains("<4E2D>"), "{s}");
    }
}
