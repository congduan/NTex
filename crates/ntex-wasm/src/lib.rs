//! # ntex-wasm — NTex 引擎核心的 WASM 薄壳（M8-A 骨架线）
//!
//! 目标：浏览器 / Node 里跑通 **plain 子集**——喂 `.tex` 字符串 + 内嵌 TFM，
//! NTex 排版，DVI 字节 + log（转录）回传 JS，并可**逐页软光栅渲染到 canvas**
//! （B 档第一刀）。不做 LaTeX（`.fmt` 依赖载入战是 C 档）。
//!
//! 三档路线（与 plan.md M8 一致）：
//! - **A 档（本 crate）**：引擎核心 WASM 化。`compile_tex()` 全链在 wasm 内完成，
//!   TFM 度量经 [`ntex_layout::set_tfm_source`] 注入（`fonts/` 内嵌 6 个 CM TFM）。
//! - **B 档（第一刀已建）**：渲染。`compile_document()` 返回 [`Document`] 句柄
//!   （整页盒树 + 字体度量常驻），`render_page()` 走 ntex-backend 软光栅
//!   （`prims` 事实源 + `Pixmap`，与桌面路径同代码），RGBA 回传 JS
//!   `putImageData` 上 canvas；翻页/调 dpi 不重排版。vello/wgpu web 后端与
//!   真字形（内嵌 Latin Modern OTF）属 B 档后续；增量接口（M5
//!   `IncrementalTypesetter` 暴露给 JS）是「毫秒级刷新」的前置。
//! - **C 档（待 `.fmt`）**：LaTeX。`.fmt` 快照经 `MemVfs` 喂入 + `import_state`，
//!   等 `ntex-format` 快照完备度上来后在此薄壳上加 `load_format(bytes)`。
//!
//! WASM 侧与 native 的已知偏差（均带注释，见对应源码）：
//! 1. `\day`/`\month`/`\year`/`\time` 固定为 1970-01-01 00:00（`ntex-core/src/param.rs`：
//!    wasm32 的 std 无 OS 时钟；不引 js-sys 进 core）；
//! 2. 线程看门狗 / 单步计时整段跳过（`ntex-core/src/expand/mod.rs`：单线程 +
//!    无 `Instant`；死循环防线只剩步数上限 + 宿主页面超时）；
//! 3. 字体度量来源是注册的 [`ntex_layout::TfmSource`]（内嵌字节），而非文件系统
//!    查找（`ntex-layout` `TfmLoader` 的 wasm 分叉）；
//! 4. 字形通道：TFM 度量内嵌（见上），轮廓字体经 [`set_glyph_font`] 由宿主
//!    注入 Latin Modern OTF 字节（进程级注册表，ntex-backend `glyphs.rs`；
//!    Tauri/浏览器前端 fetch 后注册，wasm 无文件系统）；未注入的字体逐字符
//!    回落占位方框口径（渲染仍可用）。
//! 5. PDF 导出（[`Document::pdf_bytes`] / [`CompileResult::pdf_bytes`]）：与
//!    native `ntex-pdf` 同一份 DVI→PDF 代码，**字体嵌入只能来自宿主注入的
//!    Type1 PFB**（[`set_pfb_font`]）——`ntex-pdf` 的宿主查找链（环境变量
//!    目录 / TeX Live 路径 / kpsewhich）为 native 专属，见
//!    `ntex-pdf/src/type1.rs::read_host_pfb` 的 wasm 分叉。
//!
//! 已知缺口（如实记录）：`\write` 到非 16 流 / `\openout` 产出的文件
//! 现留在 `MemVfs` 内不回传——`ntex-io` 的 `MemVfs` 暂无枚举 API（只有 `read`/
//! `write`/`append`/`get`），补枚举接口属 ntex-io 领地，不在本刀范围。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use wasm_bindgen::prelude::*;

/// 内嵌 CM 字体（**plain 预载全集** 48 个）：来源 TeX Live
/// `texmf-dist/fonts/tfm/public/{cm,knuth-lib}/`（Computer Modern + manfnt，
/// 可再分发；来源与许可见 `fonts/README.md`）。
///
/// 为什么是"preload 全集"而非"用到哪几个嵌哪几个"：wasm 管线走
/// `set_preload_plain(true)`（等价源首行 `\input plain`），内嵌 `plain.tex`
/// 的字体段会把整套 `\preloaded` 字体装进 eqtb——缺一个就吐一行
/// `! Font cmr9 not loadable: Metric (TFM) file not found.`。A 档只嵌了 14 个
/// 10pt/7pt/5pt 常用件，于是 resume-plain.tex 这类 plain 作业在 Tauri 里会带
/// 34 行缺字体噪声（2026-09-11 修复）。全集仅 ~196 KB（每个 TFM 1~1.5 KB），
/// 换 log 与排版语义干净，值。
///
/// 分组：正文族 r / 粗体 bx / 打字机 tt / 斜体 ti / 数学斜体 mi / 数学符号 sy /
/// 大算符 ex / 无衬线 ss / 细体 sl / 小型大写 csc + manfnt（plain 的 `\manfnt`
/// 提示字符）。名字逐字对应内嵌 `plain.tex` 的 `\font\preloaded=<name>` 行，
/// 清单由该文件 grep 得出而非手抄（见 `fonts/README.md` 的「清单来源」）。
const EMBEDDED_TFMS: &[(&str, &[u8])] = &[
    // 罗马正文族（10pt 基准 + 8/9pt + 脚本层 5/6/7pt + 标题 12pt）
    ("cmr5", include_bytes!("../fonts/cmr5.tfm")),
    ("cmr6", include_bytes!("../fonts/cmr6.tfm")),
    ("cmr7", include_bytes!("../fonts/cmr7.tfm")),
    ("cmr8", include_bytes!("../fonts/cmr8.tfm")),
    ("cmr9", include_bytes!("../fonts/cmr9.tfm")),
    ("cmr10", include_bytes!("../fonts/cmr10.tfm")),
    ("cmr12", include_bytes!("../fonts/cmr12.tfm")),
    // 粗体扩展族
    ("cmbx5", include_bytes!("../fonts/cmbx5.tfm")),
    ("cmbx6", include_bytes!("../fonts/cmbx6.tfm")),
    ("cmbx7", include_bytes!("../fonts/cmbx7.tfm")),
    ("cmbx8", include_bytes!("../fonts/cmbx8.tfm")),
    ("cmbx9", include_bytes!("../fonts/cmbx9.tfm")),
    ("cmbx10", include_bytes!("../fonts/cmbx10.tfm")),
    // 打字机体族
    ("cmtt8", include_bytes!("../fonts/cmtt8.tfm")),
    ("cmtt9", include_bytes!("../fonts/cmtt9.tfm")),
    ("cmtt10", include_bytes!("../fonts/cmtt10.tfm")),
    // 意大利体族
    ("cmti7", include_bytes!("../fonts/cmti7.tfm")),
    ("cmti8", include_bytes!("../fonts/cmti8.tfm")),
    ("cmti9", include_bytes!("../fonts/cmti9.tfm")),
    ("cmti10", include_bytes!("../fonts/cmti10.tfm")),
    // 数学文本斜体（OML）
    ("cmmi5", include_bytes!("../fonts/cmmi5.tfm")),
    ("cmmi6", include_bytes!("../fonts/cmmi6.tfm")),
    ("cmmi7", include_bytes!("../fonts/cmmi7.tfm")),
    ("cmmi8", include_bytes!("../fonts/cmmi8.tfm")),
    ("cmmi9", include_bytes!("../fonts/cmmi9.tfm")),
    ("cmmi10", include_bytes!("../fonts/cmmi10.tfm")),
    ("cmmib10", include_bytes!("../fonts/cmmib10.tfm")),
    // 数学符号（OMS）+ 粗体符号
    ("cmsy5", include_bytes!("../fonts/cmsy5.tfm")),
    ("cmsy6", include_bytes!("../fonts/cmsy6.tfm")),
    ("cmsy7", include_bytes!("../fonts/cmsy7.tfm")),
    ("cmsy8", include_bytes!("../fonts/cmsy8.tfm")),
    ("cmsy9", include_bytes!("../fonts/cmsy9.tfm")),
    ("cmsy10", include_bytes!("../fonts/cmsy10.tfm")),
    ("cmbsy10", include_bytes!("../fonts/cmbsy10.tfm")),
    // 大算符（OMX）
    ("cmex10", include_bytes!("../fonts/cmex10.tfm")),
    // 无衬线族（含 `\ssq` 倾斜/引号变体）
    ("cmss10", include_bytes!("../fonts/cmss10.tfm")),
    ("cmssbx10", include_bytes!("../fonts/cmssbx10.tfm")),
    ("cmssi10", include_bytes!("../fonts/cmssi10.tfm")),
    ("cmssq8", include_bytes!("../fonts/cmssq8.tfm")),
    ("cmssqi8", include_bytes!("../fonts/cmssqi8.tfm")),
    // 细体族（cmsl / cmsltt 斜体打字机）
    ("cmsl8", include_bytes!("../fonts/cmsl8.tfm")),
    ("cmsl9", include_bytes!("../fonts/cmsl9.tfm")),
    ("cmsl10", include_bytes!("../fonts/cmsl10.tfm")),
    ("cmsltt10", include_bytes!("../fonts/cmsltt10.tfm")),
    // 小型大写 + 装饰 + 数学 U + manfnt（plain `\manfnt` 提示字形）
    ("cmcsc10", include_bytes!("../fonts/cmcsc10.tfm")),
    ("cmdunh10", include_bytes!("../fonts/cmdunh10.tfm")),
    ("cmu10", include_bytes!("../fonts/cmu10.tfm")),
    ("manfnt", include_bytes!("../fonts/manfnt.tfm")),
];

/// 随 crate 发布的示例源（`examples/demo.tex`）：`demo_tex()` 与裸冒烟入口共用。
const DEMO_TEX: &str = include_str!("../examples/demo.tex");

/// OpenType 度量注册表条目：`(TeX 字体名, 字体文件字节)`。
type OtfMetricEntry = (String, Vec<u8>);

/// OpenType 度量注册表本体：`LazyLock` 惰性初始化 + `Mutex` 互斥。
///
/// 抽成别名而非内联写 `LazyLock<Mutex<Vec<(String, Vec<u8>)>>>`：后者会被
/// clippy `type_complexity` 拦下（`make lint` 带 `-D warnings`）。
type OtfMetricRegistry = LazyLock<Mutex<Vec<OtfMetricEntry>>>;

/// 进程级 OpenType 度量注册表（TeX 字体名 → 字体字节），供
/// [`EmbeddedTfmSource::otf_bytes`] 命中。
///
/// 与 [`ntex_backend::glyphs::register_font_bytes`] 的**字形**注册表配对：
/// 本表喂**排版度量**（ntex-layout `TfmLoader` → `ntex_font::build_metrics`），
/// 后者喂**渲染轮廓**（ntex-backend `GlyphCache`）。两张表存在的原因是
/// 排版与渲染是两个 crate、两条独立解析路径；[`set_otf_font`] 一次调用把
/// 两侧都写上，避免"排出来了但渲染成方框"的半吊子态。
///
/// 用 `Mutex` 而非 `thread_local`：wasm32 单线程无所谓，但 native 测试
/// 多线程并行跑同一 crate，全局表要能安全共享（与 ntex-backend 注册表同构）。
static OTF_METRICS: OtfMetricRegistry = LazyLock::new(|| Mutex::new(Vec::new()));

/// UTF-8 输入默认开关（宿主经 [`set_utf8_input`] 设置）：进程级，影响后续全部
/// [`compile_tex`] / [`compile_document`] 调用。
///
/// 用 `AtomicBool` 而非 `Mutex`：只有一个 bool、无复合状态，且读点在每次
/// 编译的热路径上，省一次加锁。
static UTF8_INPUT: AtomicBool = AtomicBool::new(false);

/// 内嵌 TFM 源：[`ntex_layout::TfmSource`] 的 `include_bytes!` 实现。
#[derive(Debug)]
struct EmbeddedTfmSource;

impl ntex_layout::TfmSource for EmbeddedTfmSource {
    fn tfm_bytes(&mut self, name: &str) -> Option<Vec<u8>> {
        EMBEDDED_TFMS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, bytes)| bytes.to_vec())
    }

    /// 宿主经 [`set_otf_font`] 注入的 OpenType 字体（CJK/任意 OTF/TTF）。
    ///
    /// 无此项时走 trait 默认 `None`，`TfmLoader` 在 wasm32 下只能回落文件
    /// 系统（不可用）→ `! Font FandolSong-Regular not loadable`：这正是
    /// Tauri 之前排不了中文的根因（2026-09-11 修复）。
    fn otf_bytes(&mut self, name: &str) -> Option<Vec<u8>> {
        OTF_METRICS
            .lock()
            .ok()?
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, bytes)| bytes.clone())
    }
}

/// 一次编译的全部产物（导出给 JS 的 [`CompileResult`]/[`Document`] 的 Rust
/// 侧本体；native 单测直接消费它，绕开 wasm-bindgen 边界）。
#[derive(Debug)]
pub struct Compiled {
    /// DVI 字节流（`ntex-dvi::write_dvi` 产物，与 native `ntex-dvi` 驱动同构）。
    pub dvi: Vec<u8>,
    /// 转录文本（`\message`/`\show`/`\write16`/错误上下文——即 TeX 的 .log 主体）。
    pub transcript: String,
    /// 逐页整页盒（`\shipout` 产物，与 `ntex_dvi::write_dvi` 输入同源）——
    /// B 档渲染的事实源（直接走盒树渲染，不经 DVI 反解析）。
    pub pages: Vec<ntex_layout::node::BoxNode>,
    /// 排版字体度量表（`FontId` 下标引用；渲染字形度量/占位方框尺寸用）。
    pub font_metrics: Vec<ntex_font::FontMetrics>,
    /// 用到的字体名（DVI 字体表，id 0 的 nullfont 占位已剔除）。
    pub font_names: Vec<String>,
}

/// 排版管线（目标无关：wasm 与 native 单测同路径）。
///
/// `MemVfs`：`\input`/`\openin`/`\write` 全部封闭在内存（RFC-3；wasm 无文件系统）。
/// 与 native `ntex-dvi` 驱动同一路径：TFM 模式 + `typeset_dvi`（自动分页）。
fn compile_pipeline(tex: &str) -> ntex_core::error::Result<Compiled> {
    compile_pipeline_with(tex, UTF8_INPUT.load(Ordering::Relaxed))
}

/// [`compile_pipeline`] 的可注入版本（`utf8_input` 显式给出，绕开进程级开关）。
///
/// 拆分动机：进程级开关在 `cargo test` 的并行线程间是共享状态，测试要能
/// 各自指定编码模式而不互相污染。
fn compile_pipeline_with(tex: &str, utf8_input: bool) -> ntex_core::error::Result<Compiled> {
    // TFM 源注册（幂等：每次编译前重挂，wasm 模块可反复编译无需额外初始化）。
    ntex_layout::set_tfm_source(Box::new(EmbeddedTfmSource));
    let mut ts = ntex_layout::Typesetter::with_tfm();
    ts.set_vfs(Box::new(ntex_io::MemVfs::new()));
    // UTF-8 直写开关（M9 中文刀 3）：宿主在编译前设定，源文件即可直接写中文。
    // 走引擎参数注入口（而非在源码前拼 `\utfinputmode=1`）——后者会让 log 的
    // `l.N` 与编辑器行号错位一行（见 Typesetter::utf8_input_default 注释）。
    ts.set_utf8_input(utf8_input);
    // 格式预载（G2(a)/G4）：与 native `ntex-dvi` 驱动同路径——内嵌 plain 兜底
    // + 启动预载（等价源首行 `\input plain`）。wasm 无文件系统，`\input plain`
    // 只能走内嵌资源；缺此则 plain 宏（`\hsize` 等）全缺，样例产空页。
    ts.use_embedded_format();
    ts.set_preload_plain(true);
    // 出错也要收转录：TeX 语义是错误上下文行进 log，作业不止于 stderr。
    let result = ts.typeset_dvi(tex);
    let transcript = ts.take_transcript();
    let (pages, fonts) = result?;
    // 无页面不出 DVI（与 native `ntex-dvi` 驱动一致：空作业拒绝写 post 段）。
    let dvi = if pages.is_empty() {
        Vec::new()
    } else {
        ntex_dvi::write_dvi(&pages, &fonts)
    };
    // 字体表首是 nullfont 占位（id 0，见 ntex-layout TfmLoader 注释），不回传。
    let font_names = fonts.iter().skip(1).map(|f| f.name.clone()).collect();
    Ok(Compiled {
        dvi,
        transcript,
        pages,
        font_metrics: fonts,
        font_names,
    })
}

// ---------- wasm-bindgen 导出面 ----------

/// DVI 字节 → PDF 字节（两个导出面共用；`pages` 仅作 0 页守卫）。
///
/// 页尺寸取 [`ntex_pdf::PdfOptions::default`] = A4，与预览
/// （`ntex_backend::RenderOptions::default`）同口径，导出件与屏幕上看到的一致。
///
/// 错误类型取 `String` 而非 `JsError`：本函数是**目标无关的纯 Rust 核心**，
/// native 单测可直接断言文案；`JsError` 只在两个 wasm 导出面各映射一次。
fn pdf_from_dvi(dvi: &[u8], pages: u32) -> Result<Vec<u8>, String> {
    if pages == 0 {
        return Err("0 页作业无 PDF 可导出（无 \\shipout 产出）".to_owned());
    }
    ntex_pdf::convert(dvi, &ntex_pdf::PdfOptions::default())
        .map_err(|e| format!("PDF 写出失败：{e}"))
}

/// 编译 plain 子集 TeX 源码 → DVI + log。
///
/// JS 侧：`const r = compile_tex("\\font\\cmr=cmr10\\cmr hello\\end");`
/// 引擎报错时抛 `JsError`，消息含 TeX 式错误（含 `l.N` 上下文行）。
#[wasm_bindgen]
pub fn compile_tex(tex: &str) -> Result<CompileResult, JsError> {
    match compile_pipeline(tex) {
        Ok(compiled) => Ok(CompileResult {
            dvi: compiled.dvi,
            transcript: compiled.transcript,
            pages: compiled.pages.len() as u32,
            fonts: compiled.font_names,
        }),
        Err(e) => Err(JsError::new(&e.to_string())),
    }
}

/// 编译结果（JS 视图）：DVI 字节 + log 文本 + 页数 + 字体清单。
#[wasm_bindgen]
pub struct CompileResult {
    dvi: Vec<u8>,
    transcript: String,
    pages: u32,
    fonts: Vec<String>,
}

#[wasm_bindgen]
impl CompileResult {
    /// DVI 字节（JS 侧为 `Uint8Array`；交 dvipdfmx 类驱动或 B 档渲染器）。
    #[wasm_bindgen(getter)]
    pub fn dvi(&self) -> Vec<u8> {
        self.dvi.clone()
    }

    /// 转录文本（TeX .log 主体：`\message`/`\show`/`\write16`/错误上下文）。
    #[wasm_bindgen(getter)]
    pub fn transcript(&self) -> String {
        self.transcript.clone()
    }

    /// `\shipout` 页数。
    #[wasm_bindgen(getter)]
    pub fn page_count(&self) -> u32 {
        self.pages
    }

    /// 用到的字体名（供 B 档渲染器选字形通道）。**引擎侧已载入全表**：
    /// plain 预载的 CM 家族（48 件）哪怕正文一个字符都没用到也会列在这里。
    /// 要「PDF 里该嵌哪些字体」请用 [`CompileResult::used_fonts`]。
    #[wasm_bindgen(getter)]
    pub fn fonts(&self) -> Vec<String> {
        self.fonts.clone()
    }

    /// DVI **实际引用**的字体名（`fnt_def` 表）→ PDF 导出该注入的 PFB 清单。
    ///
    /// 与 [`CompileResult::fonts`] 的差别是导出正确性的关键：`fonts` 是已载入
    /// 全表（plain 预载 48 件 CM 全在），而 DVI 只给真正被 `set_font` 过的字体发
    /// `fnt_def`。按 `fonts` 去 fetch PFB 会白拉几十份资源、并对正文根本没用到的
    /// 字体误报「缺字体」；按本清单则恰好只取该嵌的那几份。
    pub fn used_fonts(&self) -> Vec<String> {
        ntex_pdf::parse_dvi(&self.dvi)
            .map(|d| d.font_names)
            .unwrap_or_default()
    }

    /// PDF 字节（A4；与 [`Document::pdf_bytes`] 同口径，字体须先用
    /// [`set_pfb_font`] 注册）。本方法每次调用都会重新解析 DVI——`compile_tex`
    /// 路径只带字节不带页树；实时预览等重复导出场景请走 [`Document`] 句柄
    /// （页树常驻，成本仍是每次重排 PDF 对象）。
    pub fn pdf_bytes(&self) -> Result<Vec<u8>, JsError> {
        pdf_from_dvi(&self.dvi, self.pages).map_err(|e| JsError::new(&e))
    }
}

// ---------- B 档第一刀：渲染句柄（软光栅 → RGBA 上 canvas） ----------

/// 软光栅渲染一页（目标无关：wasm 导出面与 native 单测同路径）。
///
/// 页面尺寸/边距取 [`ntex_backend::RenderOptions::default`]（A4 + 72pt 四边，
/// 同 ntex-pdf 与桌面 `ntex-backend` 驱动的默认口径）；`dpi`/`debug`/`glyphs`
/// 由参数覆盖。`glyphs = true` 走真字形轮廓填充（字体需先经
/// [`set_glyph_font`] 注册，未注册字体回落占位方框，见模块文档偏差 4）。
fn render_page_core(
    pages: &[ntex_layout::node::BoxNode],
    fonts: &[ntex_font::FontMetrics],
    index: u32,
    dpi: f64,
    debug: bool,
    glyphs: bool,
) -> Result<ntex_backend::Pixmap, String> {
    let page = pages
        .get(index as usize)
        .ok_or_else(|| format!("页码越界：请求第 {index} 页（0 起），共 {} 页", pages.len()))?;
    use ntex_backend::Backend as _;
    let opts = ntex_backend::RenderOptions {
        dpi,
        debug,
        glyphs,
        ..Default::default()
    };
    let mut pages = ntex_backend::TinySkiaBackend
        .render(std::slice::from_ref(page), fonts, &opts)
        .map_err(|e| format!("渲染第 {index} 页失败：{e}"))?;
    match pages.pop() {
        Some(pm) => Ok(pm),
        None => Err(format!("渲染第 {index} 页失败：后端未返回像素")),
    }
}

/// 编译产物句柄：持有整页盒树与字体度量常驻，可反复按页/按 dpi/按开关渲染
/// （**不重排版**）——实时预览的 JS 侧锚点：编辑防抖后重建 Document，
/// 翻页/调 dpi/切 overlay 只调 [`Document::render_page`]。
#[wasm_bindgen]
pub struct Document {
    pages: Vec<ntex_layout::node::BoxNode>,
    font_metrics: Vec<ntex_font::FontMetrics>,
    font_names: Vec<String>,
    transcript: String,
    dvi: Vec<u8>,
    /// 是否走真字形轮廓渲染（默认关 = 占位方框口径，兼容既有前端；
    /// 字体经 [`set_glyph_font`] 注册后 JS 调 [`Document::set_glyphs`] 开启）。
    use_glyphs: bool,
}

#[wasm_bindgen]
impl Document {
    /// `\shipout` 页数。
    #[wasm_bindgen(getter)]
    pub fn page_count(&self) -> u32 {
        self.pages.len() as u32
    }

    /// 切换真字形轮廓渲染（on = true）与占位方框口径（on = false）。
    ///
    /// 只影响后续 [`Document::render_page`] 调用（不重排版/不重编译）；
    /// 字体未注册时开启亦安全——逐字符回落方框（引擎契约）。
    pub fn set_glyphs(&mut self, on: bool) {
        self.use_glyphs = on;
    }

    /// 转录文本（TeX .log 主体：`\message`/`\show`/`\write16`/错误上下文）。
    #[wasm_bindgen(getter)]
    pub fn transcript(&self) -> String {
        self.transcript.clone()
    }

    /// 用到的字体名清单。**引擎侧已载入全表**（plain 预载 48 件 CM 全在，
    /// 不区分正文是否真的用到）——PDF 导出该注入哪些 PFB 请用
    /// [`Document::used_fonts`]。
    #[wasm_bindgen(getter)]
    pub fn fonts(&self) -> Vec<String> {
        self.font_names.clone()
    }

    /// DVI **实际引用**的字体名（`fnt_def` 表）→ PDF 导出该注入的 PFB 清单。
    ///
    /// 导出前对本清单逐名 fetch `<name>.pfb` 并 [`set_pfb_font`] 注入即可；
    /// 拿不到的字体名就是最终 PDF 里「有 `/BaseFont` 无 `/FontFile`」的那些，
    /// 前端据此给一次可见提示（而不是静默产出打不开字体的 PDF）。
    ///
    /// 与 [`Document::fonts`] 的差别见后者说明；DVI 为空（0 页）时返回空表。
    pub fn used_fonts(&self) -> Vec<String> {
        ntex_pdf::parse_dvi(&self.dvi)
            .map(|d| d.font_names)
            .unwrap_or_default()
    }

    /// DVI 字节（交 dvipdfmx 类驱动或下载；与 `compile_tex` 产物同构）。
    #[wasm_bindgen(getter)]
    pub fn dvi(&self) -> Vec<u8> {
        self.dvi.clone()
    }

    /// PDF 字节（A4，与预览同页尺寸口径）。
    ///
    /// 与 native `ntex-pdf` 命令共用 `ntex_pdf::convert`（DVI 解析 + Type1
    /// 嵌入 + PDF 1.4 写出）。**字体嵌入依赖已注册的 PFB**：wasm 无文件系统，
    /// `ntex-pdf` 的宿主查找链在浏览器里必然落空，故导出前须对本
    /// [`Document::fonts`] 逐名 fetch `<name>.pfb` 并 [`set_pfb_font`] 注入。
    ///
    /// 未注册的字体按 `ntex-pdf` 既有口径**降级**：`/BaseFont` 保留但不写
    /// `/FontFile` 流——多数查看器会以替代字体渲染或干脆留白，所以前端应在
    /// 调用前把名字注册齐，并对缺字体给出可见提示（Tauri 工作台即如此）。
    pub fn pdf_bytes(&self) -> Result<Vec<u8>, JsError> {
        pdf_from_dvi(&self.dvi, self.page_count()).map_err(|e| JsError::new(&e))
    }

    /// 渲染第 `index` 页（0 起）→ RGBA8 像素（白底、行主序、每像素 4 字节）。
    ///
    /// JS 侧：`const img = doc.render_page(0, 144, false);` →
    /// `new ImageData(new Uint8ClampedArray(img.rgba), img.width, img.height)`
    /// → `ctx.putImageData(...)`。`dpi` 任意正数（A4@72dpi ≈ 595×842、
    /// @144dpi ≈ 1191×1684）；`debug` 叠加排版调试 overlay（盒边界/glue/
    /// 断点标记，与 native `ntex-backend --debug` 同口径）；字形口径由
    /// [`Document::set_glyphs`] 控制（默认方框）。
    pub fn render_page(&self, index: u32, dpi: f64, debug: bool) -> Result<PageImage, JsError> {
        render_page_core(
            &self.pages,
            &self.font_metrics,
            index,
            dpi,
            debug,
            self.use_glyphs,
        )
        .map(|pm| PageImage {
            width: pm.width(),
            height: pm.height(),
            rgba: pm.data().to_vec(),
        })
        .map_err(|e| JsError::new(&e))
    }
}

/// 一页渲染结果（JS 视图）：`rgba` 为 RGBA8 行主序字节，直接灌 `ImageData`。
#[wasm_bindgen]
pub struct PageImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[wasm_bindgen]
impl PageImage {
    /// 页宽（px）。
    #[wasm_bindgen(getter)]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// 页高（px）。
    #[wasm_bindgen(getter)]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// 像素字节（RGBA8 行主序；JS 侧为 `Uint8Array`，长度 = width×height×4）。
    #[wasm_bindgen(getter)]
    pub fn rgba(&self) -> Vec<u8> {
        self.rgba.clone()
    }
}

/// 编译 plain 子集 TeX 源码 → 渲染句柄（B 档入口；引擎报错抛 `JsError`，
/// 消息含 TeX 式错误上下文）。可恢复的 TeX 错误（缺字体等）不抛——作业
/// 继续、转录含错误行，页面按实际 `\shipout` 产出。
#[wasm_bindgen]
pub fn compile_document(tex: &str) -> Result<Document, JsError> {
    match compile_pipeline(tex) {
        Ok(c) => Ok(Document {
            pages: c.pages,
            font_metrics: c.font_metrics,
            font_names: c.font_names,
            transcript: c.transcript,
            dvi: c.dvi,
            use_glyphs: false,
        }),
        Err(e) => Err(JsError::new(&e.to_string())),
    }
}

/// 注入轮廓字体字节（OTF/TTF；`tex_name` 为 TeX 排版字体名如 `cmr10`）。
///
/// **仅注册渲染字形通道**（ntex-backend `glyphs.rs::register_font_bytes`）——
/// 用于"已有 TFM 度量（cmr10 等）+ 想补真字形轮廓"的场景。此后
/// [`Document::set_glyphs`]（true）渲染即走真字形轮廓，未注册字体逐字符
/// 回落占位方框。坏字节返回 false 不 panic（引擎契约）。Latin Modern
/// 与 CM 同源（度量一致），文件来源/许可见各前端 `fonts/` 目录 README。
///
/// 若字体**没有 TFM**（CJK 等 OpenType 原生字体），须改用 [`set_otf_font`]
/// ——它同时打通排版度量，只有字形注册的话 `\font\zh=FandolSong-Regular`
/// 会在排版阶段就报 `not loadable`。
#[wasm_bindgen]
pub fn set_glyph_font(tex_name: &str, bytes: &[u8]) -> bool {
    ntex_backend::glyphs::register_font_bytes(tex_name, bytes)
}

/// 注入 Type1（PFB）字体字节 → 供 PDF 导出嵌入（`ntex_pdf::type1::register_pfb`）。
///
/// **与 [`set_glyph_font`] 是两条独立通道**，别混：
/// - [`set_glyph_font`] 管**屏幕上的字形轮廓**（OTF/TTF，走 skrifa 提轮廓）；
/// - 本条管**PDF 里的字体程序**（Type1 PFB，原样写进 `/FontFile`）——
///   PDF 的 `/FontFile` 只接受 Type1 程序，OTF 塞不进去（那要走 CID/OpenType
///   嵌入，属另案），故屏幕端用 OTF 也要**另配一份 PFB** 才能嵌出完整 PDF。
///
/// 名字与 `doc.fonts`（DVI `fnt_def` 外部名，如 `cmr10`）一致；同名重复注册
/// 为覆盖。字节非 PFB（段头不是 `0x80 0x01`，如误传 OTF）返回 false 不 panic。
#[wasm_bindgen]
pub fn set_pfb_font(tex_name: &str, bytes: &[u8]) -> bool {
    ntex_pdf::type1::register_pfb(tex_name, bytes)
}

/// 注入 **OpenType 排版字体**（无 TFM 的字体：中文 Fandol/思源、西文 OTF）。
///
/// 一次调用注册两侧，`true` 表示度量与字形**均**可用：
/// 1. **排版度量**：写入本模块 [`OTF_METRICS`] 表，`TfmLoader` 解析
///    `\font\zh=FandolSong-Regular` 时经 [`ntex_layout::TfmSource::otf_bytes`]
///    取字节 → `ntex_font::build_metrics` 建度量（hmtx + bbox）；
/// 2. **渲染字形**：转交 `ntex_backend::glyphs::register_font_bytes`，
///    `Document::set_glyphs(true)` 后按 cmap 直查画轮廓（`FontMetrics::
///    unicode_native` 直通 Unicode 码位）。
///
/// 同名覆盖（前端重复 fetch 幂等）。坏字节 / 空名字返回 `false` 不 panic
/// （引擎契约）。**与 [`set_glyph_font`] 的分工**：本函数管"从零接入一个
/// OpenType 字体"，后者管"给已有 TFM 字体补轮廓"。
///
/// JS 侧（Tauri `ui/main.js` 的用法，本地 fetch 后注入）：
/// ```js
/// const bytes = new Uint8Array(await (await fetch('fonts/FandolSong-Regular.otf')).arrayBuffer());
/// set_otf_font('FandolSong-Regular', bytes);   // 排版 + 渲染双通
/// ```
#[wasm_bindgen]
pub fn set_otf_font(tex_name: &str, bytes: &[u8]) -> bool {
    if tex_name.is_empty() {
        return false;
    }
    // 先验度量可解析——坏字节在此拒绝（而非等到排版时报 not loadable）。
    if ntex_font::build_metrics(bytes.to_vec(), tex_name).is_err() {
        return false;
    }
    let Ok(mut table) = OTF_METRICS.lock() else {
        // 锁毒化：持锁线程已 panic，注入失败按"环境无字体"处理（不传播）。
        return false;
    };
    match table.iter_mut().find(|(n, _)| n == tex_name) {
        Some(slot) => slot.1 = bytes.to_vec(),
        None => table.push((tex_name.to_owned(), bytes.to_vec())),
    }
    drop(table);
    // 字形侧同步注册：两表同进同出，避免"排了但渲染方框"。
    ntex_backend::glyphs::register_font_bytes(tex_name, bytes)
}

/// UTF-8 输入默认开关（M9 中文刀 3）：开则后续编译把 `\utfinputmode` 预置为 1，
/// 源文件可直接写中文（输入层把 UTF-8 多字节合并成单个 21-bit 字符 token，
/// >255 码位默认 catcode letter，XeTeX 惯例）。
///
/// 引擎默认是 bytes 模式（0）——**TRIP/ETRIP/expl3 的 8-bit 口径依赖它**，
/// 所以本开关只在宿主侧显式打开（Tauri/浏览器前端按 UI 的「UTF-8」勾选调用）。
/// 源文件里显式的 `\utfinputmode=0/1` 仍优先生效（后写覆盖预置）。
///
/// 与"前端在源码前拼一行 `\utfinputmode=1`"的区别：走引擎参数注入口，
/// **用户源文本逐字节不动**，log/转录里的 `l.N` 与编辑器行号对齐
/// （`docs/tooling-trust.md` 的仪器可信度纪律）。
#[wasm_bindgen]
pub fn set_utf8_input(on: bool) {
    UTF8_INPUT.store(on, Ordering::Relaxed);
}

/// 当前 UTF-8 输入默认开关（前端回显 UI 状态用）。
#[wasm_bindgen]
pub fn utf8_input() -> bool {
    UTF8_INPUT.load(Ordering::Relaxed)
}

/// 引擎版本与能力描述（一行；JS 侧显示用）。
#[wasm_bindgen]
pub fn engine_version() -> String {
    format!(
        "NTex WASM {} (plain subset; DVI out; embedded CM TFMs: {})",
        env!("CARGO_PKG_VERSION"),
        EMBEDDED_TFMS.len()
    )
}

/// 内嵌 TFM 字体名清单（`\font` 目前只能用这些名字）。
#[wasm_bindgen]
pub fn embedded_fonts() -> Vec<String> {
    EMBEDDED_TFMS.iter().map(|(n, _)| (*n).to_owned()).collect()
}

/// 取内嵌 TFM 字体字节（B 档渲染器做 DVI 字体度量/字形定位用）。
#[wasm_bindgen]
pub fn embedded_font_bytes(name: &str) -> Option<Vec<u8>> {
    EMBEDDED_TFMS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, bytes)| bytes.to_vec())
}

/// 随包示例源（`examples/demo.tex`；demo 页共用）。
#[wasm_bindgen]
pub fn demo_tex() -> String {
    DEMO_TEX.to_owned()
}

// ---------- 冒烟说明 ----------

// 原计划留一个 `#[no_mangle] extern "C"` 裸导出（返回 DVI 字节数）供无
// wasm-bindgen CLI 的环境做 Node 冒烟；但 Rust 1.98 起该属性被归入 unsafe
// 属性，工作区 `unsafe_code = "deny"`（AGENTS.md 工程红线）直接拒绝编译——
// 不为冒烟开 unsafe 例外，裸导出撤除。因此 JS 侧冒烟必须先经 wasm-bindgen
// CLI 生成绑定（见 README「构建」一节）；CLI 装不上时的验证边界见 README。

// ---------- native 单测（与 wasm 同路径，见 README「验证边界」） ----------

#[cfg(test)]
mod tests {
    use super::*;

    /// Tauri 前端实际 fetch 的那份 Fandol Song 子集（`crates/ntex-tauri/ui/fonts/`，
    /// GPL / CTAN fandol，见 `scripts/make-cjk-subset.py`）——测试直接锁线上字节，
    /// 避免"测试用一份字体、前端用另一份"的漂移。
    const FANDOL_BYTES: &[u8] = include_bytes!("../../ntex-tauri/ui/fonts/FandolSong-Regular.otf");

    /// 仓库根的简历示例（原故障现场：Tauri 渲染报 34 行缺字体 + 数百行缺字形）。
    const RESUME_PLAIN: &str = include_str!("../../../resume-plain.tex");

    /// 内嵌示例 → DVI 非空、结构正确（pre=247 / 版本 2，post_post 尾随 4×223）、
    /// 含 cmr10 字体定义、页数 ≥ 1。
    #[test]
    fn embedded_demo_compiles_to_dvi() {
        let compiled = compile_pipeline(DEMO_TEX).expect("示例应能编译");
        assert!(
            !compiled.pages.is_empty(),
            "应产出至少一页，实得 {}",
            compiled.pages.len()
        );
        assert_eq!(compiled.dvi[0], 247, "DVI pre 命令码");
        assert_eq!(compiled.dvi[1], 2, "DVI 格式版本");
        assert!(
            compiled.dvi.ends_with(&[223, 223, 223, 223]),
            "post_post 应以 4×223 填充结尾"
        );
        assert!(
            compiled.font_names.iter().any(|name| name == "cmr10"),
            "字体表应含 cmr10，实得 {:?}",
            compiled.font_names
        );
    }

    /// 自举 plain 示例不能依赖格式预载：`\bye` 所需的 `\eject` 已在源内以
    /// 核心 penalty 定义。此回归覆盖 Tauri/WASM 所走的无预载编译管线。
    #[test]
    fn self_bootstrapped_plain_demo_compiles_without_undefined_eject() {
        const SOURCE: &str = include_str!("../../../demo1-fixed.tex");
        let compiled = compile_pipeline(SOURCE).expect("自举 plain 示例应能编译");
        assert!(!compiled.pages.is_empty(), "示例应至少产出一页");
        assert!(
            !compiled
                .transcript
                .contains("! Undefined control sequence.\n\\eject"),
            "不得再出现未定义 \\eject：{}",
            compiled.transcript
        );
    }

    /// 统计非白像素数（渲染有墨验证；alpha=0 或任一通道 ≥250 视为白）。
    fn ink_pixels(pm: &ntex_backend::Pixmap) -> usize {
        (0..pm.height())
            .flat_map(|y| (0..pm.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| pm.pixel_nonwhite(x, y))
            .count()
    }

    /// 渲染管线（B 档第一刀）：demo → 盒树 → 软光栅。首两页 72dpi 为
    /// A4（595×842）白底有墨；144dpi 尺寸翻倍；越界页码报错不 panic。
    #[test]
    fn render_pipeline_paints_demo_pages() {
        let compiled = compile_pipeline(DEMO_TEX).expect("示例应能编译");
        assert!(
            compiled.pages.len() >= 2,
            "随包示例应 ≥2 页（第三段溢出 \\vsize 触发分页）"
        );
        let pm = render_page_core(
            &compiled.pages,
            &compiled.font_metrics,
            0,
            72.0,
            false,
            false,
        )
        .expect("第 0 页应可渲染");
        assert_eq!((pm.width(), pm.height()), (595, 842), "A4@72dpi");
        // 首段文字（占位方框口径）必然落墨；demo 首页数百字符 × 每字符 3 矩形。
        let ink = ink_pixels(&pm);
        assert!(ink > 1_000, "首页应有可观墨迹，实得 {ink} px");
        let pm2 = render_page_core(
            &compiled.pages,
            &compiled.font_metrics,
            1,
            72.0,
            false,
            false,
        )
        .expect("第 1 页应可渲染");
        assert!(ink_pixels(&pm2) > 0, "第 1 页应有墨");
        // dpi 翻倍 → 像素尺寸翻倍（1191×1684）。
        let pm144 = render_page_core(
            &compiled.pages,
            &compiled.font_metrics,
            0,
            144.0,
            false,
            false,
        )
        .expect("144dpi 应可渲染");
        assert_eq!((pm144.width(), pm144.height()), (1191, 1684), "A4@144dpi");
        // 越界：可恢复错误消息（不 panic——引擎契约）。
        let err = render_page_core(
            &compiled.pages,
            &compiled.font_metrics,
            99,
            72.0,
            false,
            false,
        )
        .expect_err("越界页码应报错");
        assert!(err.contains("越界"), "错误消息应含上下文：{err}");
    }

    /// debug overlay：独立通道叠加后墨迹严格增加，且 dpi=0 非法配置报错。
    #[test]
    fn render_pipeline_debug_overlay_and_bad_dpi() {
        let compiled = compile_pipeline(DEMO_TEX).expect("示例应能编译");
        let plain = render_page_core(
            &compiled.pages,
            &compiled.font_metrics,
            0,
            72.0,
            false,
            false,
        )
        .expect("应可渲染");
        let dbg = render_page_core(
            &compiled.pages,
            &compiled.font_metrics,
            0,
            72.0,
            true,
            false,
        )
        .expect("debug 应可渲染");
        assert!(
            ink_pixels(&dbg) > ink_pixels(&plain),
            "overlay 应增加墨迹（版心描边等）"
        );
        let err = render_page_core(
            &compiled.pages,
            &compiled.font_metrics,
            0,
            0.0,
            false,
            false,
        )
        .expect_err("dpi=0 应报错");
        assert!(err.contains("渲染"), "错误消息应含上下文：{err}");
    }

    /// 字形注入全链路：注册 LM OTF → glyphs=true 渲染 → 实心字形墨迹
    /// 显著多方框口径（demo 正文为 cmr10；见 ntex-backend `glyphs.rs` 测试）。
    /// 字体字节复用 ntex-backend 的入库 fixture（tests/data/README 同源）。
    #[test]
    fn glyph_font_injection_paints_outlines() {
        const LM_ROMAN: &[u8] =
            include_bytes!("../../ntex-backend/tests/data/lmroman10-regular.otf");
        assert!(
            set_glyph_font("cmr10-glyph-test", LM_ROMAN),
            "合法 OTF 应注册成功"
        );
        assert!(
            !set_glyph_font("cmr10-glyph-test-bad", &[0u8; 8]),
            "坏字节应拒绝"
        );

        let compiled = compile_pipeline(DEMO_TEX).expect("示例应能编译");
        let boxes = render_page_core(
            &compiled.pages,
            &compiled.font_metrics,
            0,
            96.0,
            false,
            false,
        )
        .expect("方框口径应可渲染");
        // 用注入名替换排版字体名再渲染（compile 用 "cmr10"，注册名带后缀
        // 避免污染其它测试的环境查找；直接重注册到真名即可命中）。
        assert!(set_glyph_font("cmr10", LM_ROMAN));
        let outlines = render_page_core(
            &compiled.pages,
            &compiled.font_metrics,
            0,
            96.0,
            false,
            true,
        )
        .expect("字形口径应可渲染");
        // 判据：两口径墨迹均非零，且像素差异显著——若注入未生效（glyphs
        // 静默回落方框）两图将完全一致，diff=0 即可捕获回归。
        // （不做大小比较：空心方框描边的墨迹本可与实心字形相当。）
        assert!(ink_pixels(&outlines) > 0 && ink_pixels(&boxes) > 0);
        let diff = outlines
            .data()
            .iter()
            .zip(boxes.data().iter())
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            diff > 5000,
            "字形/方框两口径应有显著像素差异（实测 {diff} 字节；疑似注入未生效）"
        );
    }

    /// 内嵌 TFM 能被 ntex-font 解析、设计字号非零（度量链的根）。
    #[test]
    fn embedded_tfms_parse() {
        for (name, bytes) in EMBEDDED_TFMS {
            let fm =
                ntex_font::parse_tfm(bytes).unwrap_or_else(|e| panic!("{name} 应为合法 TFM：{e}"));
            assert!(fm.design_size_sp > 0, "{name} 设计字号应非零");
        }
    }

    /// 缺字体 → **可恢复** TeX 错误（tex.web：`! Font .. not loadable` 进转录，
    /// 作业继续不 panic）。
    ///
    /// ⚠ G4 接线后语义变更：wasm 流水线与 native `ntex-dvi` 一致地预载 plain，
    /// 故 `\end` 触发 plain 的输出例程后**会**产 1 页（plain 的 `\null` 页）。
    /// 旧断言「无 `\shipout` → 0 页」建立在「wasm 不预载」的副作用上——那不是
    /// 契约，只是未接线的表现。此处改断言**作业继续且转录含 TeX 式错误**这一
    /// 真正关心的事实（页面数由 plain 输出例程决定，不作硬编码断言）。
    #[test]
    fn unknown_font_is_recoverable_error() {
        let compiled = compile_pipeline("\\font\\x=nosuchfont10\\x hi\\end").expect("作业应继续");
        assert!(
            compiled
                .transcript
                .contains("! Font nosuchfont10 not loadable"),
            "转录应含 TeX 式缺字体错误：{:?}",
            compiled.transcript
        );
    }

    /// 空作业不报错（`compile_pipeline` 返回 `Ok`）。
    ///
    /// ⚠ G4 接线后语义变更：预载 plain 使空作业也产页（见上）。此处只断言
    /// 「不报错」这一契约，页面数不作硬编码（与 native `ntex-dvi` 行为对齐：
    /// `printf '' | ntex-dvi` 亦产 1 页）。
    #[test]
    fn empty_input_does_not_error() {
        let compiled = compile_pipeline("\\end").expect("空作业不应报错");
        assert_eq!(
            compiled.pages.len(),
            1,
            "预载 plain 后 `\\end` 触发输出例程产 1 页"
        );
        assert!(!compiled.dvi.is_empty(), "有页面即应出 DVI");
    }

    // ---------- 2026-09-11：Tauri 渲染 resume-plain.tex 报错的两条回归锁 ----------

    /// 内嵌 plain.tex 里 `\font\preloaded=<name>` 引用的全部字体（grep 提取，
    /// 见 `fonts/README.md` 的「清单来源」）。缺任一 → plain 预载吐一条
    /// `! Font <name> not loadable: Metric (TFM) file not found.`。
    ///
    /// 回归背景：A 档只内嵌 14 个，Tauri 里跑 plain 作业会带 34 行缺字体噪声。
    const PLAIN_PRELOAD_FONTS: &[&str] = &[
        "cmbsy10", "cmbx10", "cmbx5", "cmbx6", "cmbx7", "cmbx8", "cmbx9", "cmcsc10", "cmdunh10",
        "cmex10", "cmmi10", "cmmi5", "cmmi6", "cmmi7", "cmmi8", "cmmi9", "cmmib10", "cmr10",
        "cmr5", "cmr6", "cmr7", "cmr8", "cmr9", "cmsl10", "cmsl8", "cmsl9", "cmsltt10", "cmss10",
        "cmssbx10", "cmssi10", "cmssq8", "cmssqi8", "cmsy10", "cmsy5", "cmsy6", "cmsy7", "cmsy8",
        "cmsy9", "cmti10", "cmti7", "cmti8", "cmti9", "cmtt10", "cmtt8", "cmtt9", "cmu10",
        "manfnt",
    ];

    /// 内嵌 TFM 表必须覆盖 plain 预载全集（48 件，含 cmr12）。
    #[test]
    fn embedded_tfms_cover_plain_preload_set() {
        for name in PLAIN_PRELOAD_FONTS {
            assert!(
                EMBEDDED_TFMS.iter().any(|(n, _)| n == name),
                "内嵌 TFM 缺 {name}——plain 预载会报 not loadable"
            );
        }
    }

    /// UTF-8 输入开关（进程级）确实落到引擎 `\utfinputmode`，且**源内显式赋值
    /// 覆盖宿主默认**（后写赢）。
    #[test]
    fn utf8_input_switch_reaches_engine_and_yields_to_source() {
        let on = compile_pipeline_with("\\message{m=\\the\\utfinputmode}\\end", true)
            .expect("作业应继续");
        assert!(on.transcript.contains("m=1"), "开关开：{}", on.transcript);

        let off = compile_pipeline_with("\\message{m=\\the\\utfinputmode}\\end", false)
            .expect("作业应继续");
        assert!(off.transcript.contains("m=0"), "开关关：{}", off.transcript);

        let overridden = compile_pipeline_with(
            "\\utfinputmode=0\\message{m=\\the\\utfinputmode}\\end",
            true,
        )
        .expect("作业应继续");
        assert!(
            overridden.transcript.contains("m=0"),
            "源内显式赋值应覆盖宿主默认：{}",
            overridden.transcript
        );
    }

    /// CJK 通路（M9 中文刀 3）：宿主经 [`set_otf_font`] 注入 OpenType 字体后，
    /// wasm 壳能排中文——修复前 `EmbeddedTfmSource` 无 `otf_bytes`，
    /// `\font\zh=FandolSong-Regular` 必然 `not loadable`（Tauri 完全排不了中文）。
    ///
    /// 字体字节用仓库内已入库的 Tauri 前端子集（`ui/fonts/`，GPL / CTAN fandol，
    /// 见 `scripts/make-cjk-subset.py`）——保证测试跑的就是前端实际用的那一份。
    #[test]
    fn otf_injection_enables_cjk_typesetting() {
        assert!(
            set_otf_font("FandolSong-Regular", FANDOL_BYTES),
            "合法 OTF 应注册"
        );
        assert!(
            set_otf_font("FandolSong-Regular", FANDOL_BYTES),
            "同名重复注入应幂等（前端重复 fetch 场景）"
        );
        assert!(!set_otf_font("bad-otf", &[0u8; 16]), "坏字节应拒绝");
        assert!(!set_otf_font("", FANDOL_BYTES), "空名字应拒绝");

        let compiled = compile_pipeline_with(
            "\\font\\zh=FandolSong-Regular at 12pt\\zh\\hsize=200pt\n中文排版\n\\end",
            true,
        )
        .expect("中文作业应能编译");
        assert!(
            !compiled.transcript.contains("not loadable"),
            "字体已注入，不应再 not loadable：\n{}",
            compiled.transcript
        );
        assert!(
            !compiled.transcript.contains("Missing character"),
            "中文字形应全部命中子集字体：\n{}",
            compiled.transcript
        );
        assert!(!compiled.pages.is_empty(), "应产出页面");
    }

    /// 端到端复现原故障：`resume-plain.tex` 在 wasm 字体口径下编译——
    /// 转录必须**零 `not loadable`、零 `Missing character`**。
    ///
    /// 这条锁同时覆盖两处修复：内嵌 TFM 补齐（plain 预载字体）+ OTF 注入
    /// （中文字体经 `set_otf_font` 进排版度量）。源文件本身用
    /// `\ifx\utfinputmode\undefined` 判别 NTex，故这条也顺带锁住该分流。
    #[test]
    fn resume_plain_compiles_clean_under_wasm_font_set() {
        assert!(set_otf_font("FandolSong-Regular", FANDOL_BYTES));
        let compiled = compile_pipeline_with(RESUME_PLAIN, true).expect("简历示例应能编译");
        assert!(!compiled.pages.is_empty(), "应至少产出一页");
        assert!(
            !compiled.transcript.contains("not loadable"),
            "plain 预载字体应全部命中内嵌 TFM；转录：\n{}",
            compiled.transcript
        );
        assert!(
            !compiled.transcript.contains("Missing character"),
            "中文字形应全部命中子集字体；转录：\n{}",
            compiled.transcript
        );
    }

    // ---------- 2026-09-12：PDF 导出（DVI → PDF，字体只能经 set_pfb_font 注入） ----------

    /// 造一段最小可解析 PFB（ASCII 段带 `/FontName`，尾部二进制段）。
    ///
    /// 与 `ntex-pdf/src/type1.rs` 测试内同名助手同构——那条是 crate 私有
    /// （`#[cfg(test)]` 内的 fn），此处无法复用，故就地重造。
    fn fake_pfb(font_name: &str) -> Vec<u8> {
        let ascii = format!("/FontName /{font_name} def\n");
        let mut pfb = vec![0x80, 1];
        pfb.extend((ascii.len() as u16).to_le_bytes());
        pfb.extend_from_slice(ascii.as_bytes());
        pfb.extend_from_slice(&[0x80, 2, 0, 0]);
        pfb
    }

    /// PDF 导出主链路：DVI → PDF 全程在 wasm 内完成，字体**只能**来自
    /// [`set_pfb_font`] 注入（wasm 无文件系统，`ntex-pdf` 的宿主查找链必落空）。
    ///
    /// 关键断言是 `/BaseFont /CMR10Probe`：注入字节内的 `/FontName` 是
    /// `CMR10Probe`，而宿主 TeX Live 里真 `cmr10.pfb` 的 `/FontName` 是
    /// `CMR10`——出现前者才证明 PDF 用的是**注入字节**而非环境字体，
    /// 也才说明「打包分发不依赖 TeX Live」这条路的字体来源是闭合的。
    #[test]
    fn pdf_export_embeds_injected_pfb() {
        let injected = fake_pfb("CMR10Probe");
        assert!(set_pfb_font("cmr10", &injected), "合法 PFB 段头应注册成功");
        assert!(
            !set_pfb_font("cmr10", b"\x00\x01\x00\x00OTTO"),
            "OTF 字节段头不符（非 0x80 0x01），应拒绝注册而不是塞进 /FontFile"
        );

        let compiled = compile_pipeline_with(
            "\\font\\a=cmr10 \\font\\b=cmbx10 \\hsize=200pt\nAa Bb\n\\end",
            false,
        )
        .expect("作业应能编译");
        assert!(
            compiled.font_names.iter().any(|n| n == "cmr10"),
            "DVI 字体表应含 cmr10：{:?}",
            compiled.font_names
        );

        let pages = compiled.pages.len();
        let pdf = pdf_from_dvi(&compiled.dvi, pages as u32).expect("应能产出 PDF");
        assert!(pdf.starts_with(b"%PDF-1.4"), "应以 PDF 头开始");
        assert!(pdf.ends_with(b"%%EOF\n"), "应以 %%EOF 收尾");
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.contains(&format!("/Count {pages}")), "页数应写入 PDF");
        assert!(
            s.contains("/BaseFont /CMR10Probe "),
            "应取注入字节的 /FontName（非宿主 cmr10.pfb 的 CMR10）——\
             只出现 /BaseFont /CMR10 即说明悄悄回了宿主查找链"
        );
        assert!(s.contains("/FontFile "), "注入了 PFB 即应写出 /FontFile 流");
    }

    /// 未注入 PFB 时**不报错**，按 `ntex-pdf` 既有口径降级：`/BaseFont` 仍在
    /// 但无 `/FontFile`。这条锁住「降级是有意的」——前端据此对缺字体给可见提示，
    /// 而不是让用户拿到一份打开后页页空白的 PDF 还以为是引擎坏了。
    ///
    /// 字体取 `manfnt`：它是内嵌 TFM 清单里**唯一**在标准 TeX Live 下没有
    /// PFB 对应物的（实测 amsfonts/cm 全家族 48 缺 1），所以「宿主查找链也
    /// 命不中」这件事可复现；仍加运行时守卫，宿主万一有 `manfnt.pfb` 就跳过
    /// （环境相关断言不该假绿）。
    ///
    /// ⚠ 源里必须真的**选中**该字体（`\a A`）：DVI 只给被 `set_font` 过的字体
    /// 发 `fnt_def`，而 `Compiled::font_names` 是引擎侧*已载入*全表（plain
    /// 预载 48 件全在里面）。把两者混为一谈会让断言在「字体压根没用上」
    /// 的情况下也过——参见 [`Document::used_fonts`] 的说明。
    #[test]
    fn pdf_export_degrades_without_pfb_but_still_writes_pdf() {
        let name = "manfnt";
        assert!(
            EMBEDDED_TFMS.iter().any(|(n, _)| *n == name),
            "前置假设：{name}.tfm 已内嵌（否则 \\font 会退回 nullfont，不进 DVI 字体表）"
        );
        if ntex_pdf::type1::find_pfb(name).is_some() {
            eprintln!("宿主存在 {name}.pfb，跳过降级断言");
            return;
        }

        let compiled =
            compile_pipeline_with(&format!("\\font\\a={name} \\hsize=200pt\n\\a A\n\\end"), false)
                .expect("字体不可用应只报警告，作业仍继续");
        assert!(
            compiled.font_names.iter().any(|n| n == name),
            "DVI 字体表应含 {name}：{:?}",
            compiled.font_names
        );

        let pdf = pdf_from_dvi(&compiled.dvi, compiled.pages.len() as u32)
            .expect("缺字体也应产出 PDF（降级，不报错）");
        let s = String::from_utf8_lossy(&pdf);
        // 报错时把实际写出的名字带出来——降级名是从 DVI 引用名大写兜底的，
        // 引擎侧改名（如 TFM 自声明名优先）会让这里悄悄漂移。
        let base_fonts: Vec<&str> = s
            .match_indices("/BaseFont /")
            .map(|(i, m)| {
                let rest = &s[i + m.len()..];
                rest.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("")
            })
            .collect();
        assert!(
            s.contains("/BaseFont /MANFNT "),
            "降级时 /BaseFont 仍写入（取 TeX 名大写兜底）；实际写出：{base_fonts:?}，\
             DVI 字体表：{:?}",
            compiled.font_names
        );
        assert!(
            !s.contains("/FontFile "),
            "无字体字节可嵌 → 不应有 /FontFile 流"
        );
    }

    /// 0 页作业无 PDF 可导：守卫在解析 DVI **之前**（返回可读错误而非 panic）。
    #[test]
    fn pdf_export_rejects_empty_document() {
        let err = pdf_from_dvi(&[], 0).expect_err("0 页应报错");
        assert!(err.contains("0 页"), "错误消息应含上下文：{err}");
    }
}
