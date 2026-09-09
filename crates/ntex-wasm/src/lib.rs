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
//!
//! 已知缺口（如实记录）：`\write` 到非 16 流 / `\openout` 产出的文件
//! 现留在 `MemVfs` 内不回传——`ntex-io` 的 `MemVfs` 暂无枚举 API（只有 `read`/
//! `write`/`append`/`get`），补枚举接口属 ntex-io 领地，不在本刀范围。

use wasm_bindgen::prelude::*;

/// 内嵌 CM 字体（plain 子集 14 个）：来源 TinyTeX `texmf-dist/fonts/tfm/public/cm/`
/// （Computer Modern，可再分发；来源与许可见 `fonts/README.md`）。
/// 10pt 七件套覆盖正文与数学文本字号（cmtt10 供 `\tt`）；7/5pt 五件套供数学
/// 上下标（plain.tex `\scriptfont`/`\scriptscriptfont` 装配口径）；cmr12 供标题
/// （`scaled` 缩放仅改尺寸不改度量来源）。
const EMBEDDED_TFMS: &[(&str, &[u8])] = &[
    ("cmr10", include_bytes!("../fonts/cmr10.tfm")),
    ("cmbx10", include_bytes!("../fonts/cmbx10.tfm")),
    ("cmti10", include_bytes!("../fonts/cmti10.tfm")),
    ("cmtt10", include_bytes!("../fonts/cmtt10.tfm")),
    ("cmmi10", include_bytes!("../fonts/cmmi10.tfm")),
    ("cmsy10", include_bytes!("../fonts/cmsy10.tfm")),
    ("cmex10", include_bytes!("../fonts/cmex10.tfm")),
    ("cmr12", include_bytes!("../fonts/cmr12.tfm")),
    ("cmr7", include_bytes!("../fonts/cmr7.tfm")),
    ("cmr5", include_bytes!("../fonts/cmr5.tfm")),
    ("cmmi7", include_bytes!("../fonts/cmmi7.tfm")),
    ("cmmi5", include_bytes!("../fonts/cmmi5.tfm")),
    ("cmsy7", include_bytes!("../fonts/cmsy7.tfm")),
    ("cmsy5", include_bytes!("../fonts/cmsy5.tfm")),
];

/// 随 crate 发布的示例源（`examples/demo.tex`）：`demo_tex()` 与裸冒烟入口共用。
const DEMO_TEX: &str = include_str!("../examples/demo.tex");

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
    // TFM 源注册（幂等：每次编译前重挂，wasm 模块可反复编译无需额外初始化）。
    ntex_layout::set_tfm_source(Box::new(EmbeddedTfmSource));
    let mut ts = ntex_layout::Typesetter::with_tfm();
    ts.set_vfs(Box::new(ntex_io::MemVfs::new()));
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

    /// 用到的字体名（供 B 档渲染器选字形通道）。
    #[wasm_bindgen(getter)]
    pub fn fonts(&self) -> Vec<String> {
        self.fonts.clone()
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

    /// 用到的字体名清单。
    #[wasm_bindgen(getter)]
    pub fn fonts(&self) -> Vec<String> {
        self.font_names.clone()
    }

    /// DVI 字节（交 dvipdfmx 类驱动或下载；与 `compile_tex` 产物同构）。
    #[wasm_bindgen(getter)]
    pub fn dvi(&self) -> Vec<u8> {
        self.dvi.clone()
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
/// wasm 无文件系统，前端 fetch Latin Modern OTF 后经此注册（进程级表，
/// 见 ntex-backend `glyphs.rs::register_font_bytes`）；此后
/// [`Document::set_glyphs`]（true）渲染即走真字形轮廓，未注册字体逐字符
/// 回落占位方框。坏字节返回 false 不 panic（引擎契约）。Latin Modern
/// 与 CM 同源（度量一致），文件来源/许可见各前端 `fonts/` 目录 README。
#[wasm_bindgen]
pub fn set_glyph_font(tex_name: &str, bytes: &[u8]) -> bool {
    ntex_backend::glyphs::register_font_bytes(tex_name, bytes)
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
    /// 作业继续不 panic）；不产页面、不出 DVI。
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
        assert_eq!(compiled.pages.len(), 0, "无 \\shipout，不应有页面");
        assert!(compiled.dvi.is_empty());
    }

    /// 空输入 → 不产出页面（`typeset_dvi` 对无 `\shipout` 作业返回空页表）。
    #[test]
    fn empty_input_yields_no_pages() {
        let compiled = compile_pipeline("\\end").expect("空作业不应报错");
        assert_eq!(compiled.pages.len(), 0);
        assert!(compiled.dvi.is_empty());
    }
}
