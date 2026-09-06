//! # ntex-wasm — NTex 引擎核心的 WASM 薄壳（M8-A 骨架线）
//!
//! 目标：浏览器 / Node 里跑通 **plain 子集**——喂 `.tex` 字符串 + 内嵌 TFM，
//! NTex 排版，DVI 字节 + log（转录）回传 JS。不做渲染（vello/wgpu web 后端
//! 是 B 档，另案），不做 LaTeX（`.fmt` 依赖载入战是 C 档）。
//!
//! 三档路线（与 plan.md M8 一致）：
//! - **A 档（本 crate）**：引擎核心 WASM 化。`compile_tex()` 全链在 wasm 内完成，
//!   TFM 度量经 [`ntex_layout::set_tfm_source`] 注入（`fonts/` 内嵌 6 个 CM TFM）。
//! - **B 档（另案）**：渲染。DVI/盒子树 → vello/wgpu web 后端画到 canvas；
//!   本 crate 的 DVI/转录返回值即其输入。
//! - **C 档（待 `.fmt`）**：LaTeX。`.fmt` 快照经 `MemVfs` 喂入 + `import_state`，
//!   等 `ntex-format` 快照完备度上来后在此薄壳上加 `load_format(bytes)`。
//!
//! WASM 侧与 native 的已知偏差（均带注释，见对应源码）：
//! 1. `\day`/`\month`/`\year`/`\time` 固定为 1970-01-01 00:00（`ntex-core/src/param.rs`：
//!    wasm32 的 std 无 OS 时钟；不引 js-sys 进 core）；
//! 2. 线程看门狗 / 单步计时整段跳过（`ntex-core/src/expand/mod.rs`：单线程 +
//!    无 `Instant`；死循环防线只剩步数上限 + 宿主页面超时）；
//! 3. 字体度量来源是注册的 [`ntex_layout::TfmSource`]（内嵌字节），而非文件系统
//!    查找（`ntex-layout` `TfmLoader` 的 wasm 分叉）。
//!
//! 已知缺口（如实记录，A 档不做）：`\write` 到非 16 流 / `\openout` 产出的文件
//! 现留在 `MemVfs` 内不回传——`ntex-io` 的 `MemVfs` 暂无枚举 API（只有 `read`/
//! `write`/`append`/`get`），补枚举接口属 ntex-io 领地，不在本刀范围。

use wasm_bindgen::prelude::*;

/// 内嵌 CM 字体（plain 子集够用的 6 个）：来源 TinyTeX `texmf-dist/fonts/tfm/public/cm/`
/// （Computer Modern，可再分发；来源与许可见 `fonts/README.md`）。
/// cmmi/cmsy/cmex 供数学（M4）与 B 档 demo 预留；随包 demo.tex 用到前三个。
const EMBEDDED_TFMS: &[(&str, &[u8])] = &[
    ("cmr10", include_bytes!("../fonts/cmr10.tfm")),
    ("cmbx10", include_bytes!("../fonts/cmbx10.tfm")),
    ("cmti10", include_bytes!("../fonts/cmti10.tfm")),
    ("cmmi10", include_bytes!("../fonts/cmmi10.tfm")),
    ("cmsy10", include_bytes!("../fonts/cmsy10.tfm")),
    ("cmex10", include_bytes!("../fonts/cmex10.tfm")),
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

/// 一次编译的全部产物（导出给 JS 的 [`CompileResult`] 的 Rust 侧本体；
/// native 单测直接消费它，绕开 wasm-bindgen 边界）。
#[derive(Debug)]
pub struct Compiled {
    /// DVI 字节流（`ntex-dvi::write_dvi` 产物，与 native `ntex-dvi` 驱动同构）。
    pub dvi: Vec<u8>,
    /// 转录文本（`\message`/`\show`/`\write16`/错误上下文——即 TeX 的 .log 主体）。
    pub transcript: String,
    /// `\shipout` 页数。
    pub pages: usize,
    /// 用到的字体名（DVI 字体表，id 0 的 nullfont 占位已剔除）。
    pub fonts: Vec<String>,
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
    let font_names = fonts.into_iter().skip(1).map(|f| f.name).collect();
    Ok(Compiled {
        dvi,
        transcript,
        pages: pages.len(),
        fonts: font_names,
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
            pages: compiled.pages as u32,
            fonts: compiled.fonts,
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
            compiled.pages >= 1,
            "应产出至少一页，实得 {}",
            compiled.pages
        );
        assert_eq!(compiled.dvi[0], 247, "DVI pre 命令码");
        assert_eq!(compiled.dvi[1], 2, "DVI 格式版本");
        assert!(
            compiled.dvi.ends_with(&[223, 223, 223, 223]),
            "post_post 应以 4×223 填充结尾"
        );
        assert!(
            compiled.fonts.iter().any(|name| name == "cmr10"),
            "字体表应含 cmr10，实得 {:?}",
            compiled.fonts
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
        assert_eq!(compiled.pages, 0, "无 \\shipout，不应有页面");
        assert!(compiled.dvi.is_empty());
    }

    /// 空输入 → 不产出页面（`typeset_dvi` 对无 `\shipout` 作业返回空页表）。
    #[test]
    fn empty_input_yields_no_pages() {
        let compiled = compile_pipeline("\\end").expect("空作业不应报错");
        assert_eq!(compiled.pages, 0);
        assert!(compiled.dvi.is_empty());
    }
}
