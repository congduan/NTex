/* @ts-self-types="./ntex_wasm.d.ts" */

/**
 * 编译结果（JS 视图）：DVI 字节 + log 文本 + 页数 + 字体清单。
 */
export class CompileResult {
    static __wrap(ptr) {
        const obj = Object.create(CompileResult.prototype);
        obj.__wbg_ptr = ptr;
        CompileResultFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        CompileResultFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_compileresult_free(ptr, 0);
    }
    /**
     * DVI 字节（JS 侧为 `Uint8Array`；交 dvipdfmx 类驱动或 B 档渲染器）。
     * @returns {Uint8Array}
     */
    get dvi() {
        const ret = wasm.compileresult_dvi(this.__wbg_ptr);
        var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v1;
    }
    /**
     * 用到的字体名（供 B 档渲染器选字形通道）。**引擎侧已载入全表**：
     * plain 预载的 CM 家族（48 件）哪怕正文一个字符都没用到也会列在这里。
     * 要「PDF 里该嵌哪些字体」请用 [`CompileResult::used_fonts`]。
     * @returns {string[]}
     */
    get fonts() {
        const ret = wasm.compileresult_fonts(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]);
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * `\shipout` 页数。
     * @returns {number}
     */
    get page_count() {
        const ret = wasm.compileresult_page_count(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * PDF 字节（A4；与 [`Document::pdf_bytes`] 同口径，字体须先用
     * [`set_glyph_font`] / [`set_otf_font`] 或兼容的 [`set_pfb_font`] 注册）。本方法每次调用都会重新解析 DVI——`compile_tex`
     * 路径只带字节不带页树；实时预览等重复导出场景请走 [`Document`] 句柄
     * （页树常驻，成本仍是每次重排 PDF 对象）。
     * @returns {Uint8Array}
     */
    pdf_bytes() {
        const ret = wasm.compileresult_pdf_bytes(this.__wbg_ptr);
        if (ret[3]) {
            throw takeFromExternrefTable0(ret[2]);
        }
        var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v1;
    }
    /**
     * 转录文本（TeX .log 主体：`\message`/`\show`/`\write16`/错误上下文）。
     * @returns {string}
     */
    get transcript() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.compileresult_transcript(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * DVI **实际引用**的字体名（`fnt_def` 表）→ PDF 导出所需字体清单。
     *
     * 与 [`CompileResult::fonts`] 的差别是导出正确性的关键：`fonts` 是已载入
     * 全表（plain 预载 48 件 CM 全在），而 DVI 只给真正被 `set_font` 过的字体发
     * `fnt_def`。按 `fonts` 去 fetch PFB 会白拉几十份资源、并对正文根本没用到的
     * 字体误报「缺字体」；按本清单则恰好只取该嵌的那几份。
     * @returns {string[]}
     */
    used_fonts() {
        const ret = wasm.compileresult_used_fonts(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]);
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
}
if (Symbol.dispose) CompileResult.prototype[Symbol.dispose] = CompileResult.prototype.free;

/**
 * 编译产物句柄：持有整页盒树与字体度量常驻，可反复按页/按 dpi/按开关渲染
 * （**不重排版**）——实时预览的 JS 侧锚点：编辑防抖后重建 Document，
 * 翻页/调 dpi/切 overlay 只调 [`Document::render_page`]。
 */
export class Document {
    static __wrap(ptr) {
        const obj = Object.create(Document.prototype);
        obj.__wbg_ptr = ptr;
        DocumentFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        DocumentFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_document_free(ptr, 0);
    }
    /**
     * DVI 字节（交 dvipdfmx 类驱动或下载；与 `compile_tex` 产物同构）。
     * @returns {Uint8Array}
     */
    get dvi() {
        const ret = wasm.document_dvi(this.__wbg_ptr);
        var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v1;
    }
    /**
     * 用到的字体名清单。**引擎侧已载入全表**（plain 预载 48 件 CM 全在，
     * 不区分正文是否真的用到）——PDF 导出该注入哪些 PFB 请用
     * [`Document::used_fonts`]。
     * @returns {string[]}
     */
    get fonts() {
        const ret = wasm.document_fonts(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]);
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * `\shipout` 页数。
     * @returns {number}
     */
    get page_count() {
        const ret = wasm.document_page_count(this.__wbg_ptr);
        return ret >>> 0;
    }
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
     * @returns {Uint8Array}
     */
    pdf_bytes() {
        const ret = wasm.document_pdf_bytes(this.__wbg_ptr);
        if (ret[3]) {
            throw takeFromExternrefTable0(ret[2]);
        }
        var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v1;
    }
    /**
     * 渲染第 `index` 页（0 起）→ RGBA8 像素（白底、行主序、每像素 4 字节）。
     *
     * JS 侧：`const img = doc.render_page(0, 144, false);` →
     * `new ImageData(new Uint8ClampedArray(img.rgba), img.width, img.height)`
     * → `ctx.putImageData(...)`。`dpi` 任意正数（A4@72dpi ≈ 595×842、
     * @144dpi ≈ 1191×1684）；`debug` 叠加排版调试 overlay（盒边界/glue/
     * 断点标记，与 native `ntex-backend --debug` 同口径）；字形口径由
     * [`Document::set_glyphs`] 控制（默认方框）。
     * @param {number} index
     * @param {number} dpi
     * @param {boolean} debug
     * @returns {PageImage}
     */
    render_page(index, dpi, debug) {
        const ret = wasm.document_render_page(this.__wbg_ptr, index, dpi, debug);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return PageImage.__wrap(ret[0]);
    }
    /**
     * 切换真字形轮廓渲染（on = true）与占位方框口径（on = false）。
     *
     * 只影响后续 [`Document::render_page`] 调用（不重排版/不重编译）；
     * 字体未注册时开启亦安全——逐字符回落方框（引擎契约）。
     * @param {boolean} on
     */
    set_glyphs(on) {
        wasm.document_set_glyphs(this.__wbg_ptr, on);
    }
    /**
     * 转录文本（TeX .log 主体：`\message`/`\show`/`\write16`/错误上下文）。
     * @returns {string}
     */
    get transcript() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.document_transcript(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * DVI **实际引用**的字体名（`fnt_def` 表）→ PDF 导出所需字体清单。
     *
     * 工作台正常路径已由 [`set_glyph_font`] 把预览 OTF 同步登记给 PDF；PFB
     * 仅作为未加载 OTF 字体的兼容回落。
     *
     * 与 [`Document::fonts`] 的差别见后者说明；DVI 为空（0 页）时返回空表。
     * @returns {string[]}
     */
    used_fonts() {
        const ret = wasm.document_used_fonts(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]);
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
}
if (Symbol.dispose) Document.prototype[Symbol.dispose] = Document.prototype.free;

/**
 * 一页渲染结果（JS 视图）：`rgba` 为 RGBA8 行主序字节，直接灌 `ImageData`。
 */
export class PageImage {
    static __wrap(ptr) {
        const obj = Object.create(PageImage.prototype);
        obj.__wbg_ptr = ptr;
        PageImageFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        PageImageFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_pageimage_free(ptr, 0);
    }
    /**
     * 页高（px）。
     * @returns {number}
     */
    get height() {
        const ret = wasm.pageimage_height(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * 像素字节（RGBA8 行主序；JS 侧为 `Uint8Array`，长度 = width×height×4）。
     * @returns {Uint8Array}
     */
    get rgba() {
        const ret = wasm.pageimage_rgba(this.__wbg_ptr);
        var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v1;
    }
    /**
     * 页宽（px）。
     * @returns {number}
     */
    get width() {
        const ret = wasm.pageimage_width(this.__wbg_ptr);
        return ret >>> 0;
    }
}
if (Symbol.dispose) PageImage.prototype[Symbol.dispose] = PageImage.prototype.free;

/**
 * 已注入资产的概要（`None` = 还没 `set_bundle`）。前端状态条/诊断用：
 * 例如 `latex.fmt · 1234 tex · 613 tfm`。
 * @returns {string | undefined}
 */
export function bundle_summary() {
    const ret = wasm.bundle_summary();
    let v1;
    if (ret[0] !== 0) {
        v1 = getStringFromWasm0(ret[0], ret[1]);
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
    }
    return v1;
}

/**
 * 清空当前项目目录及其 aux/toc 缓存。
 */
export function clear_project_files() {
    wasm.clear_project_files();
}

/**
 * 编译 plain 子集 TeX 源码 → 渲染句柄（B 档入口；引擎报错抛 `JsError`，
 * 消息含 TeX 式错误上下文）。可恢复的 TeX 错误（缺字体等）不抛——作业
 * 继续、转录含错误行，页面按实际 `\shipout` 产出。
 * @param {string} tex
 * @returns {Document}
 */
export function compile_document(tex) {
    const ptr0 = passStringToWasm0(tex, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.compile_document(ptr0, len0);
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return Document.__wrap(ret[0]);
}

/**
 * 编译 plain 子集 TeX 源码 → DVI + log。
 *
 * JS 侧：`const r = compile_tex("\\font\\cmr=cmr10\\cmr hello\\end");`
 * 引擎报错时抛 `JsError`，消息含 TeX 式错误（含 `l.N` 上下文行）。
 * @param {string} tex
 * @returns {CompileResult}
 */
export function compile_tex(tex) {
    const ptr0 = passStringToWasm0(tex, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.compile_tex(ptr0, len0);
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return CompileResult.__wrap(ret[0]);
}

/**
 * 随包示例源（`examples/demo.tex`；demo 页共用）。
 * @returns {string}
 */
export function demo_tex() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.demo_tex();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * 取内嵌 TFM 字体字节（B 档渲染器做 DVI 字体度量/字形定位用）。
 * @param {string} name
 * @returns {Uint8Array | undefined}
 */
export function embedded_font_bytes(name) {
    const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.embedded_font_bytes(ptr0, len0);
    let v2;
    if (ret[0] !== 0) {
        v2 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
    }
    return v2;
}

/**
 * 内嵌 TFM 字体名清单（`\font` 目前只能用这些名字）。
 * @returns {string[]}
 */
export function embedded_fonts() {
    const ret = wasm.embedded_fonts();
    var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]);
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v1;
}

/**
 * 引擎版本与能力描述（一行；JS 侧显示用）。
 * @returns {string}
 */
export function engine_version() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.engine_version();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * 当前 LaTeX 模式开关（前端回显 UI 状态用）。
 * @returns {boolean}
 */
export function latex_mode() {
    const ret = wasm.latex_mode();
    return ret !== 0;
}

/**
 * 当前项目文件概要，供 Tauri 状态栏回显。
 * @returns {string | undefined}
 */
export function project_summary() {
    const ret = wasm.project_summary();
    let v1;
    if (ret[0] !== 0) {
        v1 = getStringFromWasm0(ret[0], ret[1]);
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
    }
    return v1;
}

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
 * @param {Uint8Array} bytes
 */
export function set_bundle(bytes) {
    const ptr0 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.set_bundle(ptr0, len0);
    if (ret[1]) {
        throw takeFromExternrefTable0(ret[0]);
    }
}

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
 * @param {string | null} [name]
 */
export function set_fallback_font(name) {
    var ptr0 = isLikeNone(name) ? 0 : passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    var len0 = WASM_VECTOR_LEN;
    wasm.set_fallback_font(ptr0, len0);
}

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
 * @param {string} tex_name
 * @param {Uint8Array} bytes
 * @returns {boolean}
 */
export function set_glyph_font(tex_name, bytes) {
    const ptr0 = passStringToWasm0(tex_name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
    const len1 = WASM_VECTOR_LEN;
    const ret = wasm.set_glyph_font(ptr0, len0, ptr1, len1);
    return ret !== 0;
}

/**
 * 切换 LaTeX 模式（开 = 编译套用已注入的 `.fmt` 且不再预载 plain）。
 *
 * 默认**关**（plain 子集口径，既有前端行为不变）。已注入资产但本开关关着时，
 * 编译仍走 plain——这不是错误，是"资产就绪 ≠ 模式打开"的显式分层。
 * @param {boolean} on
 */
export function set_latex_mode(on) {
    wasm.set_latex_mode(on);
}

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
 * @param {string} tex_name
 * @param {Uint8Array} bytes
 * @returns {boolean}
 */
export function set_otf_font(tex_name, bytes) {
    const ptr0 = passStringToWasm0(tex_name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
    const len1 = WASM_VECTOR_LEN;
    const ret = wasm.set_otf_font(ptr0, len0, ptr1, len1);
    return ret !== 0;
}

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
 * @param {string} tex_name
 * @param {Uint8Array} bytes
 * @returns {boolean}
 */
export function set_pfb_font(tex_name, bytes) {
    const ptr0 = passStringToWasm0(tex_name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
    const len1 = WASM_VECTOR_LEN;
    const ret = wasm.set_pfb_font(ptr0, len0, ptr1, len1);
    return ret !== 0;
}

/**
 * 注入用户选择的项目目录。宿主把所有文件按 [`PROJECT_MAGIC`] 契约打包，
 * 文件名保留相对路径；后一次调用整体替换前一次并清空跨项目辅助文件。
 * @param {Uint8Array} bytes
 */
export function set_project_files(bytes) {
    const ptr0 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.set_project_files(ptr0, len0);
    if (ret[1]) {
        throw takeFromExternrefTable0(ret[0]);
    }
}

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
 * @param {boolean} on
 */
export function set_utf8_input(on) {
    wasm.set_utf8_input(on);
}

/**
 * 当前 UTF-8 输入默认开关（前端回显 UI 状态用）。
 * @returns {boolean}
 */
export function utf8_input() {
    const ret = wasm.utf8_input();
    return ret !== 0;
}
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg_Error_408e67f47ca7b58b: function(arg0, arg1) {
            const ret = Error(getStringFromWasm0(arg0, arg1));
            return ret;
        },
        __wbg___wbindgen_throw_bb96b2010945f0bc: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg_error_a537a7d5ab79fdc0: function(arg0, arg1) {
            console.error(getStringFromWasm0(arg0, arg1));
        },
        __wbindgen_cast_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./ntex_wasm_bg.js": import0,
    };
}

const CompileResultFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_compileresult_free(ptr, 1));
const DocumentFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_document_free(ptr, 1));
const PageImageFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_pageimage_free(ptr, 1));

function getArrayJsValueFromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    const mem = getDataViewMemory0();
    const result = [];
    for (let i = ptr; i < ptr + 4 * len; i += 4) {
        result.push(wasm.__wbindgen_externrefs.get(mem.getUint32(i, true)));
    }
    wasm.__externref_drop_slice(ptr, len);
    return result;
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function isLikeNone(x) {
    return x === undefined || x === null;
}

function passArray8ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 1, 1) >>> 0;
    getUint8ArrayMemory0().set(arg, ptr / 1);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__externref_table_dealloc(idx);
    return value;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedDataViewMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (!module.ok) {
            throw new Error(`failed to fetch Wasm: ${module.status} ${module.statusText} fetching '${module.url}'`);
        }

        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('ntex_wasm_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
