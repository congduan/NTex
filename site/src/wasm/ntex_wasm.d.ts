/* tslint:disable */
/* eslint-disable */

/**
 * 编译结果（JS 视图）：DVI 字节 + log 文本 + 页数 + 字体清单。
 */
export class CompileResult {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * PDF 字节（A4；与 [`Document::pdf_bytes`] 同口径，字体须先用
     * [`set_glyph_font`] / [`set_otf_font`] 或兼容的 [`set_pfb_font`] 注册）。本方法每次调用都会重新解析 DVI——`compile_tex`
     * 路径只带字节不带页树；实时预览等重复导出场景请走 [`Document`] 句柄
     * （页树常驻，成本仍是每次重排 PDF 对象）。
     */
    pdf_bytes(): Uint8Array;
    /**
     * DVI **实际引用**的字体名（`fnt_def` 表）→ PDF 导出所需字体清单。
     *
     * 与 [`CompileResult::fonts`] 的差别是导出正确性的关键：`fonts` 是已载入
     * 全表（plain 预载 48 件 CM 全在），而 DVI 只给真正被 `set_font` 过的字体发
     * `fnt_def`。按 `fonts` 去 fetch PFB 会白拉几十份资源、并对正文根本没用到的
     * 字体误报「缺字体」；按本清单则恰好只取该嵌的那几份。
     */
    used_fonts(): string[];
    /**
     * DVI 字节（JS 侧为 `Uint8Array`；交 dvipdfmx 类驱动或 B 档渲染器）。
     */
    readonly dvi: Uint8Array;
    /**
     * 用到的字体名（供 B 档渲染器选字形通道）。**引擎侧已载入全表**：
     * plain 预载的 CM 家族（48 件）哪怕正文一个字符都没用到也会列在这里。
     * 要「PDF 里该嵌哪些字体」请用 [`CompileResult::used_fonts`]。
     */
    readonly fonts: string[];
    /**
     * `\shipout` 页数。
     */
    readonly page_count: number;
    /**
     * 转录文本（TeX .log 主体：`\message`/`\show`/`\write16`/错误上下文）。
     */
    readonly transcript: string;
}

/**
 * 编译产物句柄：持有整页盒树与字体度量常驻，可反复按页/按 dpi/按开关渲染
 * （**不重排版**）——实时预览的 JS 侧锚点：编辑防抖后重建 Document，
 * 翻页/调 dpi/切 overlay 只调 [`Document::render_page`]。
 */
export class Document {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * PDF 字节（A4，与预览同页尺寸口径）。
     *
     * 与 native `ntex-pdf` 命令共用 `ntex_pdf::convert`。工作台把预览所用的
     * Latin Modern OTF 同时登记给 PDF，TFM 槽位经同一编码表映射到 OTF CID，
     * 因而屏幕与下载文件不再使用两套字体；PFB 仍是兼容回落。
     *
     * 未注册的字体按 `ntex-pdf` 既有口径**降级**：`/BaseFont` 保留但不写
     * `/FontFile` 流——多数查看器会以替代字体渲染或干脆留白，所以前端应在
     * 调用前把名字注册齐，并对缺字体给出可见提示（Tauri 工作台即如此）。
     */
    pdf_bytes(): Uint8Array;
    /**
     * 渲染第 `index` 页（0 起）→ RGBA8 像素（白底、行主序、每像素 4 字节）。
     *
     * JS 侧：`const img = doc.render_page(0, 144, false);` →
     * `new ImageData(new Uint8ClampedArray(img.rgba), img.width, img.height)`
     * → `ctx.putImageData(...)`。`dpi` 任意正数（A4@72dpi ≈ 595×842、
     * @144dpi ≈ 1191×1684）；`debug` 叠加排版调试 overlay（盒边界/glue/
     * 断点标记，与 native `ntex-backend --debug` 同口径）；字形口径由
     * [`Document::set_glyphs`] 控制（默认方框）。
     */
    render_page(index: number, dpi: number, debug: boolean): PageImage;
    /**
     * 切换真字形轮廓渲染（on = true）与占位方框口径（on = false）。
     *
     * 只影响后续 [`Document::render_page`] 调用（不重排版/不重编译）；
     * 字体未注册时开启亦安全——逐字符回落方框（引擎契约）。
     */
    set_glyphs(on: boolean): void;
    /**
     * DVI **实际引用**的字体名（`fnt_def` 表）→ PDF 导出所需字体清单。
     *
     * 工作台正常路径已由 [`set_glyph_font`] 把预览 OTF 同步登记给 PDF；PFB
     * 仅作为未加载 OTF 字体的兼容回落。
     *
     * 与 [`Document::fonts`] 的差别见后者说明；DVI 为空（0 页）时返回空表。
     */
    used_fonts(): string[];
    /**
     * DVI 字节（交 dvipdfmx 类驱动或下载；与 `compile_tex` 产物同构）。
     */
    readonly dvi: Uint8Array;
    /**
     * 用到的字体名清单。**引擎侧已载入全表**（plain 预载 48 件 CM 全在，
     * 不区分正文是否真的用到）——PDF 导出该注入哪些 PFB 请用
     * [`Document::used_fonts`]。
     */
    readonly fonts: string[];
    /**
     * `\shipout` 页数。
     */
    readonly page_count: number;
    /**
     * 转录文本（TeX .log 主体：`\message`/`\show`/`\write16`/错误上下文）。
     */
    readonly transcript: string;
}

/**
 * 一页渲染结果（JS 视图）：`rgba` 为 RGBA8 行主序字节，直接灌 `ImageData`。
 */
export class PageImage {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * 页高（px）。
     */
    readonly height: number;
    /**
     * 像素字节（RGBA8 行主序；JS 侧为 `Uint8Array`，长度 = width×height×4）。
     */
    readonly rgba: Uint8Array;
    /**
     * 页宽（px）。
     */
    readonly width: number;
}

/**
 * 已注入资产的概要（`None` = 还没 `set_bundle`）。前端状态条/诊断用：
 * 例如 `latex.fmt · 1234 tex · 613 tfm`。
 */
export function bundle_summary(): string | undefined;

/**
 * 清空当前项目目录及其 aux/toc 缓存。
 */
export function clear_project_files(): void;

/**
 * 编译 plain 子集 TeX 源码 → 渲染句柄（B 档入口；引擎报错抛 `JsError`，
 * 消息含 TeX 式错误上下文）。可恢复的 TeX 错误（缺字体等）不抛——作业
 * 继续、转录含错误行，页面按实际 `\shipout` 产出。
 */
export function compile_document(tex: string): Document;

/**
 * 编译 plain 子集 TeX 源码 → DVI + log。
 *
 * JS 侧：`const r = compile_tex("\\font\\cmr=cmr10\\cmr hello\\end");`
 * 引擎报错时抛 `JsError`，消息含 TeX 式错误（含 `l.N` 上下文行）。
 */
export function compile_tex(tex: string): CompileResult;

/**
 * 随包示例源（`examples/demo.tex`；demo 页共用）。
 */
export function demo_tex(): string;

/**
 * 取内嵌 TFM 字体字节（B 档渲染器做 DVI 字体度量/字形定位用）。
 */
export function embedded_font_bytes(name: string): Uint8Array | undefined;

/**
 * 内嵌 TFM 字体名清单（`\font` 目前只能用这些名字）。
 */
export function embedded_fonts(): string[];

/**
 * 引擎版本与能力描述（一行；JS 侧显示用）。
 */
export function engine_version(): string;

/**
 * 当前 LaTeX 模式开关（前端回显 UI 状态用）。
 */
export function latex_mode(): boolean;

/**
 * 当前项目文件概要，供 Tauri 状态栏回显。
 */
export function project_summary(): string | undefined;

/**
 * 注入 C 档发行资产包（**一次调用装齐** fmt + TeX 文件 + 额外 TFM 度量）。
 *
 * 容器格式见 [`BUNDLE_MAGIC`]；构建端是 `crates/ntex-tauri/src/main.rs` 的
 * `build_latex_bundle`（Tauri 只读 `assets/` 后打包，经一次原样字节 IPC 送到
 * 前端，避免 600+ 文件逐个往返）。wasm 无文件系统，这是 LaTeX 唯一的来源。
 *
 * 语义：**幂等替换**（后一次调用整体替换前一次），解析失败即整体拒绝并抛
 * `JsError`——不留下"fmt 装了但 tex 没装"的半吊子状态。
 *
 * **不**自动打开 LaTeX 模式：模式由 [`set_latex_mode`] 显式控制，plain 作业
 * 不受影响（前端按源特征切换即可）。
 */
export function set_bundle(bytes: Uint8Array): void;

/**
 * 设置 CJK 字体回落（workbench 档；`name = null/undefined` 关闭，默认）。
 *
 * 前端在 [`set_otf_font`]`('FandolSong-Regular', bytes)` 之后调
 * `set_fallback_font('FandolSong-Regular')`。之后 [`compile_document`] 里，
 * 当前字体（如 cmr10）缺字形且字符码位 > 0xFF（utf8 输入才可能）时，该字符
 * 自动改用回落字体排版——用户源**逐字节不动**（resume1-plain 这类"plain
 * 格式直写中文"的文档不再整段 Missing character）。
 *
 * 8-bit 码位（ASCII/latin-1）永不回落：TRIP/ETRIP 的 "Missing character"
 * 与 "Bad character code" 硬口径原样保留。
 */
export function set_fallback_font(name?: string | null): void;

/**
 * 注入轮廓字体字节（OTF/TTF；`tex_name` 为 TeX 排版字体名如 `cmr10`）。
 *
 * 同时注册屏幕字形通道与 PDF 的 8-bit OTF 通道——用于"已有 TFM 度量
 * （cmr10 等）+ 想补真字形轮廓"的场景。PDF 仍使用 TFM 度量，只复用同一
 * OTF 字形及 [`ntex_backend::glyphs::slot_to_unicode`] 编码表。此后
 * [`Document::set_glyphs`]（true）渲染即走真字形轮廓，未注册字体逐字符
 * 回落占位方框。坏字节返回 false 不 panic（引擎契约）。Latin Modern
 * 与 CM 同源（度量一致），文件来源/许可见各前端 `fonts/` 目录 README。
 *
 * 若字体**没有 TFM**（CJK 等 OpenType 原生字体），须改用 [`set_otf_font`]
 * ——它同时打通排版度量，只有字形注册的话 `\font\zh=FandolSong-Regular`
 * 会在排版阶段就报 `not loadable`。
 */
export function set_glyph_font(tex_name: string, bytes: Uint8Array): boolean;

/**
 * 切换 LaTeX 模式（开 = 编译套用已注入的 `.fmt` 且不再预载 plain）。
 *
 * 默认**关**（plain 子集口径，既有前端行为不变）。已注入资产但本开关关着时，
 * 编译仍走 plain——这不是错误，是"资产就绪 ≠ 模式打开"的显式分层。
 */
export function set_latex_mode(on: boolean): void;

/**
 * 注入 **OpenType 排版字体**（无 TFM 的字体：中文 Fandol/思源、西文 OTF）。
 *
 * 一次调用注册三侧，`true` 表示度量与字形**均**可用：
 * 1. **排版度量**：写入本模块 [`OTF_METRICS`] 表，`TfmLoader` 解析
 *    `\font\zh=FandolSong-Regular` 时经 [`ntex_layout::TfmSource::otf_bytes`]
 *    取字节 → `ntex_font::build_metrics` 建度量（hmtx + bbox）；
 * 2. **渲染字形**：转交 `ntex_backend::glyphs::register_font_bytes`，
 *    `Document::set_glyphs(true)` 后按 cmap 直查画轮廓（`FontMetrics::
 *    unicode_native` 直通 Unicode 码位）；
 * 3. **PDF 导出**：转交 `ntex_pdf::otf::register_otf`，写出端按
 *    Type0/CIDFontType0 + `/FontFile3 /CIDFontType0C`（裸 CFF）嵌入；内容流 CID 由
 *    `ntex_pdf::cid` 按字体 cmap+charset 换算为**字体真 CID**（Fandol 为
 *    Adobe-GB1，非 Unicode 码位）——中文 PDF 从此真嵌字体，无需 Type1 PFB。
 *
 * 同名覆盖（前端重复 fetch 幂等）。坏字节 / 空名字返回 `false` 不 panic
 * （引擎契约）。**与 [`set_glyph_font`] 的分工**：本函数管"从零接入一个
 * OpenType 字体"，后者管"给已有 TFM 字体补轮廓"。
 *
 * JS 侧（Tauri `ui/main.js` 的用法，本地 fetch 后注入）：
 * ```js
 * const bytes = new Uint8Array(await (await fetch('fonts/FandolSong-Regular.otf')).arrayBuffer());
 * set_otf_font('FandolSong-Regular', bytes);   // 排版 + 渲染 + PDF 嵌入三通
 * ```
 */
export function set_otf_font(tex_name: string, bytes: Uint8Array): boolean;

/**
 * 注入 Type1（PFB）字体字节 → 供 PDF 导出嵌入（`ntex_pdf::type1::register_pfb`）。
 *
 * 这是 [`set_glyph_font`] 的兼容回落：后者已让屏幕与 PDF 共用 OTF；本条在
 * 宿主只有 Type1 字体时原样写进 `/FontFile`，显式注入时优先于同名 OTF。
 *
 * 名字与 `doc.fonts`（DVI `fnt_def` 外部名，如 `cmr10`）一致；同名重复注册
 * 为覆盖。字节非 PFB（段头不是 `0x80 0x01`，如误传 OTF）返回 false 不 panic。
 *
 * **OTF 字体（中文 Fandol 等）不走这里**——[`set_otf_font`] 注入的字节会
 * 顺带登记进 PDF 侧 OTF 注册表，导出按 Type0（裸 CFF）嵌入，无需另配 PFB。
 */
export function set_pfb_font(tex_name: string, bytes: Uint8Array): boolean;

/**
 * 注入用户选择的项目目录。宿主把所有文件按 [`PROJECT_MAGIC`] 契约打包，
 * 文件名保留相对路径；后一次调用整体替换前一次并清空跨项目辅助文件。
 */
export function set_project_files(bytes: Uint8Array): void;

/**
 * UTF-8 输入默认开关（M9 中文刀 3）：开则后续编译把 `\utfinputmode` 预置为 1，
 * 源文件可直接写中文（输入层把 UTF-8 多字节合并成单个 21-bit 字符 token，
 * >255 码位默认 catcode letter，XeTeX 惯例）。
 *
 * 引擎默认是 bytes 模式（0）——**TRIP/ETRIP/expl3 的 8-bit 口径依赖它**，
 * 所以本开关只在宿主侧显式打开（Tauri/浏览器前端按 UI 的「UTF-8」勾选调用）。
 * 源文件里显式的 `\utfinputmode=0/1` 仍优先生效（后写覆盖预置）。
 *
 * 与"前端在源码前拼一行 `\utfinputmode=1`"的区别：走引擎参数注入口，
 * **用户源文本逐字节不动**，log/转录里的 `l.N` 与编辑器行号对齐
 * （`docs/tooling-trust.md` 的仪器可信度纪律）。
 */
export function set_utf8_input(on: boolean): void;

/**
 * 当前 UTF-8 输入默认开关（前端回显 UI 状态用）。
 */
export function utf8_input(): boolean;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_compileresult_free: (a: number, b: number) => void;
    readonly __wbg_document_free: (a: number, b: number) => void;
    readonly __wbg_pageimage_free: (a: number, b: number) => void;
    readonly bundle_summary: () => [number, number];
    readonly clear_project_files: () => void;
    readonly compile_document: (a: number, b: number) => [number, number, number];
    readonly compile_tex: (a: number, b: number) => [number, number, number];
    readonly compileresult_dvi: (a: number) => [number, number];
    readonly compileresult_fonts: (a: number) => [number, number];
    readonly compileresult_page_count: (a: number) => number;
    readonly compileresult_pdf_bytes: (a: number) => [number, number, number, number];
    readonly compileresult_transcript: (a: number) => [number, number];
    readonly compileresult_used_fonts: (a: number) => [number, number];
    readonly demo_tex: () => [number, number];
    readonly document_dvi: (a: number) => [number, number];
    readonly document_fonts: (a: number) => [number, number];
    readonly document_page_count: (a: number) => number;
    readonly document_pdf_bytes: (a: number) => [number, number, number, number];
    readonly document_render_page: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly document_set_glyphs: (a: number, b: number) => void;
    readonly document_transcript: (a: number) => [number, number];
    readonly document_used_fonts: (a: number) => [number, number];
    readonly embedded_font_bytes: (a: number, b: number) => [number, number];
    readonly embedded_fonts: () => [number, number];
    readonly engine_version: () => [number, number];
    readonly latex_mode: () => number;
    readonly pageimage_height: (a: number) => number;
    readonly pageimage_rgba: (a: number) => [number, number];
    readonly pageimage_width: (a: number) => number;
    readonly project_summary: () => [number, number];
    readonly set_bundle: (a: number, b: number) => [number, number];
    readonly set_fallback_font: (a: number, b: number) => void;
    readonly set_glyph_font: (a: number, b: number, c: number, d: number) => number;
    readonly set_latex_mode: (a: number) => void;
    readonly set_otf_font: (a: number, b: number, c: number, d: number) => number;
    readonly set_pfb_font: (a: number, b: number, c: number, d: number) => number;
    readonly set_project_files: (a: number, b: number) => [number, number];
    readonly set_utf8_input: (a: number) => void;
    readonly utf8_input: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __externref_drop_slice: (a: number, b: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
