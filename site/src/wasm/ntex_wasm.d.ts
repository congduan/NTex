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
     * DVI 字节（JS 侧为 `Uint8Array`；交 dvipdfmx 类驱动或 B 档渲染器）。
     */
    readonly dvi: Uint8Array;
    /**
     * 用到的字体名（供 B 档渲染器选字形通道）。
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
     * DVI 字节（交 dvipdfmx 类驱动或下载；与 `compile_tex` 产物同构）。
     */
    readonly dvi: Uint8Array;
    /**
     * 用到的字体名清单。
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
 * 注入轮廓字体字节（OTF/TTF；`tex_name` 为 TeX 排版字体名如 `cmr10`）。
 *
 * wasm 无文件系统，前端 fetch Latin Modern OTF 后经此注册（进程级表，
 * 见 ntex-backend `glyphs.rs::register_font_bytes`）；此后
 * [`Document::set_glyphs`]（true）渲染即走真字形轮廓，未注册字体逐字符
 * 回落占位方框。坏字节返回 false 不 panic（引擎契约）。Latin Modern
 * 与 CM 同源（度量一致），文件来源/许可见各前端 `fonts/` 目录 README。
 */
export function set_glyph_font(tex_name: string, bytes: Uint8Array): boolean;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_compileresult_free: (a: number, b: number) => void;
    readonly __wbg_document_free: (a: number, b: number) => void;
    readonly __wbg_pageimage_free: (a: number, b: number) => void;
    readonly compile_document: (a: number, b: number) => [number, number, number];
    readonly compile_tex: (a: number, b: number) => [number, number, number];
    readonly compileresult_dvi: (a: number) => [number, number];
    readonly compileresult_fonts: (a: number) => [number, number];
    readonly compileresult_page_count: (a: number) => number;
    readonly compileresult_transcript: (a: number) => [number, number];
    readonly demo_tex: () => [number, number];
    readonly document_dvi: (a: number) => [number, number];
    readonly document_fonts: (a: number) => [number, number];
    readonly document_page_count: (a: number) => number;
    readonly document_render_page: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly document_set_glyphs: (a: number, b: number) => void;
    readonly document_transcript: (a: number) => [number, number];
    readonly embedded_font_bytes: (a: number, b: number) => [number, number];
    readonly embedded_fonts: () => [number, number];
    readonly engine_version: () => [number, number];
    readonly pageimage_height: (a: number) => number;
    readonly pageimage_rgba: (a: number) => [number, number];
    readonly pageimage_width: (a: number) => number;
    readonly set_glyph_font: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
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
