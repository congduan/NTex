# ntex-wasm — NTex 引擎核心的 WASM 薄壳（M8-A 骨架 + B 档 + C 档第一刀）

在浏览器（或 Node）里跑 NTex：喂 `.tex` 字符串 + 内嵌 TFM 度量，引擎排版，
回传 **DVI 字节 + 转录文本**，并可**逐页软光栅渲染到 canvas**（实时预览工作台）。
plain 子集开箱即用；**LaTeX（`.fmt`）经宿主注入资产包后可用**（C 档，见下）。

> 本文件是**绑定契约与用法文档**。三档路线的进度、剩余项与排期一律看
> [../../plan.md](../../plan.md) §2 M8；此处表格只说明各档"提供什么能力"。

## 三档路线

| 档 | 内容 | 状态 |
|---|---|---|
| **A（本 crate）** | 引擎核心 WASM 化：`compile_tex()` 全链在 wasm 内完成，TFM 经 `ntex_layout::set_tfm_source` 注入（`fonts/` 内嵌 **48 个 CM TFM，共 196 KB** —— 覆盖内嵌 plain 预载字体块全集，2026-09-11 由 14 件补齐；见 `fonts/README.md`） | ✅ 已建（2026-09-06） |
| **B** | 渲染。**第一刀已建（2026-09-06）**：`compile_document()` → `Document` 句柄（页盒树常驻）→ `render_page()` 走 ntex-backend **软光栅**（`prims` 事实源 + `Pixmap`，与桌面同代码；vello 经 feature 门控不进 wasm），RGBA 回传 JS `putImageData`；翻页/调 dpi/切 overlay 不重排版。**真字形与中文已通**（2026-09-11）：`set_glyph_font`（Latin/Modern，走 ntex-backend 轮廓注册表）+ `set_otf_font`（任意 OTF/TTF，**同时写排版度量与渲染轮廓两侧**）由宿主 fetch 后注入，`Document::set_glyphs(true)` 切轮廓渲染；`set_utf8_input(true)` 让源文件直写中文。后续：vello/wgpu web 后端、增量接口 | ✅ 第一刀 + 真字形/中文已建 |
| **C** | LaTeX。**第一刀已建（2026-09-18）**：宿主把发行资产打成一个**资产包**（`.fmt` 快照 + `.cls`/`.sty` 等 TeX 文件 + TFM 度量）经 [`set_bundle`] 一次注入 → `ntex_format::load` 还原 `FmtState` → `Typesetter::import_state`；`\documentclass{article}` 等真 LaTeX 宏可用（`set_latex_mode(true)` 开模式）。打包端在 `crates/ntex-tauri/src/main.rs::build_latex_bundle`，容器契约见 `src/lib.rs` 的 `BUNDLE_MAGIC` 文档。wasm 无文件系统，故 fmt/tex/tfm **三者都只能由宿主喂** | ✅ 第一刀已建 |

B 档第一刀的实测（M2 MacBook，A4@144dpi）：demo 作业编译 ~3-11ms + 渲染 ~6-9ms，
250ms 编辑防抖下即点即见；wasm 产物 832K（含 skrifa/peniko 软光栅链）。

## 构建

```bash
rustup target add wasm32-unknown-unknown
cargo check -p ntex-wasm --target wasm32-unknown-unknown   # 编译验证
cargo build -p ntex-wasm --target wasm32-unknown-unknown --release
# 产物：target/wasm32-unknown-unknown/release/ntex_wasm.wasm
```

**本机已知坑（Homebrew rust 与 rustup 并存时）**：若 PATH 里 Homebrew 的
`cargo/rustc` 优先（`which cargo` 指向 `/opt/homebrew/bin`），构建 wasm 会报
`E0463 can't find crate for std`（Homebrew rust 无法 `rustup target add`）。
改用 rustup toolchain 并显式指定 RUSTC：

```bash
RUSTC=$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin/rustc \
    rustup run stable cargo build -p ntex-wasm \
    --target wasm32-unknown-unknown --release
```

若链接报 `rust-lld … Library not loaded: libLLVM.dylib`，是 toolchain 装成了
minimal profile（本仓库 `rust-toolchain.toml` 即如此），补组件即可：
`rustup component add llvm-tools-preview`。

生成 JS/TS 绑定（需要 wasm-bindgen CLI，**版本必须与 `wasm-bindgen` crate 完全一致**，
见 `Cargo.lock`）：

```bash
cargo install wasm-bindgen-cli --version <Cargo.lock 里的 wasm-bindgen 版本>
wasm-bindgen --out-dir www/pkg --target web \
    target/wasm32-unknown-unknown/release/ntex_wasm.wasm
```

`wasm-pack` 同样可用（`wasm-pack build crates/ntex-wasm --target web --release`）；
装不上 CLI 时的验证边界见文末。

## 在浏览器使用

`www/` 是**实时预览工作台**（与 native `ntex-studio` 同交互骨架）：左 TeX 编辑 /
右 canvas 预览，编辑 250ms 防抖重排重渲；翻页/dpi/排版 overlay 只重渲染（不重排版）；
转录面板 + DVI 导出。

```bash
wasm-bindgen --out-dir www/pkg --target web \
    target/wasm32-unknown-unknown/release/ntex_wasm.wasm
cd www && python3 -m http.server 8000   # 浏览器打开 http://localhost:8000
```

JS API（wasm-bindgen 生成后）：

```js
import init, { compile_document, compile_tex, engine_version, embedded_fonts, demo_tex,
               set_glyph_font, set_otf_font, set_utf8_input, utf8_input, set_fallback_font,
               set_bundle, set_latex_mode, latex_mode, bundle_summary }
    from './pkg/ntex_wasm.js';
await init();

console.log(engine_version());            // "NTex WASM 0.1.0 (plain subset; ...)"
console.log(embedded_fonts().length);      // 48（plain 预载字体块全集）
console.log(embedded_fonts().slice(0, 6)); // ["cmr5","cmr6","cmr7","cmr8","cmr9","cmr10"]

// —— 字形注入：真字形 + 中文（2026-09-11）——
// 两张注册表各司其职：set_glyph_font 只喂「渲染轮廓」，set_otf_font 一次写
// 「排版度量 + 渲染轮廓」两侧（少写度量＝排得出来但渲染方框，反之亦然）。
const lm = await fetch('fonts/lmroman10-regular.otf').then(r => r.arrayBuffer());
set_glyph_font('cmr10', new Uint8Array(lm));

const fandol = await fetch('fonts/FandolSong-Regular.otf').then(r => r.arrayBuffer());
set_otf_font('FandolSong-Regular', new Uint8Array(fandol));   // true = 度量解析成功

set_utf8_input(true);                     // 源文件可直写中文（等价 \utfinputmode=1）
console.log(utf8_input());                // true

// —— CJK 字体回落（2026-09-18）——
// 源文件不写 `\font\zh=FandolSong-Regular` 也能排中文：当前字体（cmr10 等
// 8-bit TFM）缺字形且码位超过 0xFF 时，该字符自动改用回落字体排。
// ASCII/latin-1 永不回落（TRIP "Missing character" 硬口径原样保留）。
set_fallback_font('FandolSong-Regular');  // null = 关闭（默认）

// —— B 档第一刀：编译 → 句柄 → 逐页软光栅渲染 ——
const doc = compile_document(demo_tex());
doc.page_count;                           // 2（\shipout 页数）
doc.transcript;                           // TeX .log 主体
doc.dvi;                                  // Uint8Array，DVI 字节流（dvipdfmx 类驱动可消费）
doc.set_glyphs(true);                     // 切真字形轮廓渲染（未注入的字体逐字回落方框）

const img = doc.render_page(0, 144, /*debug=*/false);   // 第 0 页 @144dpi → RGBA
// img.width × img.height（A4@144dpi = 1191×1684），img.rgba = Uint8Array(w*h*4)
ctx.putImageData(new ImageData(new Uint8ClampedArray(img.rgba), img.width, img.height), 0, 0);
```

> 中文文档（plain + Fandol Song）完整可用样例见 `crates/ntex-tauri/ui/`：那里的
> `index.html`/`main.js` 已按上面的顺序（fetch 字体 → 注入 → 开 UTF-8 → 编译）
> 接好，配合 `resume-plain.tex` 可端到端验证。

```js
// —— C 档：LaTeX（2026-09-18）——
// 资产包 = 宿主把 .fmt + .cls/.sty + TFM 打成一包（容器魔数 NTEXBND1），
// wasm 无文件系统，三类文件都只能这样喂。Tauri 侧打包命令见 ntex-tauri。
const bundle = await fetch('/latex.bundle').then(r => r.arrayBuffer());
set_bundle(new Uint8Array(bundle));      // 解析失败抛错（坏魔数/截断/坏 fmt 整体拒绝）
console.log(bundle_summary());           // "latex.fmt · 177 tex · 643 tfm"（实测 821 条 ≈11.7 MB）
set_latex_mode(true);                    // 有 fmt → 用 fmt（不再预载 plain）；无 fmt → 回落 plain
console.log(latex_mode());               // true

const doc = compile_document(articleSrc); // \documentclass{article}... 真 LaTeX 宏
```

```js
// —— A 档原接口（一次性拿 DVI + 转录，不持句柄）——
const r = compile_tex(demo_tex());
console.log(r.dvi.length, r.page_count, r.fonts);

try {
    compile_tex("\\font\\x=nosuchfont10\\x hi\\end");
} catch (e) {
    // TeX 式错误（含 l.N 上下文行），不 panic
}
```

## 与 native 的已知偏差（全部带注释，见对应源码）

1. **日期时间**：`\day`/`\month`/`\year`/`\time` 固定为 1970-01-01 00:00
   （`ntex-core/src/param.rs`：wasm32 的 std 无 OS 时钟，`SystemTime::now` 直接 panic；
   不为取真时钟把 js-sys 引进 ntex-core）。副作用是确定性的：同输入 → 同 DVI 字节。
2. **看门狗**：`Expander::run()` 的线程看门狗 + 单步计时整段 cfg 门控跳过
   （`ntex-core/src/expand/mod.rs`：单线程环境无线程可起、无 `Instant` 可用）。
   死循环防线剩两条：步数上限（`steps > 10_000_000`，两目标一致）+ 浏览器宿主页面超时。
3. **字体度量来源**：注册的 `TfmSource`（内嵌字节）而非文件系统查找
   （`ntex-layout` `TfmLoader` 的字节源分叉；`fonts/README.md` 记录了字体来源与许可）。
4. **渲染口径（B 档第一刀 + 真字形）**：字形通道**不再是"wasm 恒回落方框"**
   （2026-09-11 更新）。wasm 无文件系统，但宿主可 fetch 字体字节后经
   `set_glyph_font`（轮廓）/ `set_otf_font`（度量+轮廓）注入，再
   `Document::set_glyphs(true)` 切轮廓渲染；**未注入的字体仍逐字回落方框**
   （缺字形不 panic）。桌面侧仍走 kpsewhich/texlive 定位（`glyphs.rs`），
   文本路径与 wasm 的注入路径共用同一轮廓注册表。规则/glue/盒子几何与桌面
   路径逐位同源（`prims.rs` 事实源）。
   另注：软光栅与 vello 两后端都支持字形通道；wasm 侧只编软光栅
   （vello 经 feature 门控不进），故浏览器内不做 GPU 光栅。

除上述四处，wasm 与 native 跑的是同一份引擎代码；native 行为零改动（门控只在
wasm32 目标 / feature 组合下生效——`ntex-backend` 的 `vello` feature 为 default，
native 路径不受影响）。

**字体注入的两表设计（易踩）**：`ntex-layout`（排版度量，`TfmSource::otf_bytes`
→ `ntex_font::build_metrics`）与 `ntex-backend`（渲染轮廓，`glyphs::register_font_bytes`）
是**两个 crate、两张独立进程级注册表**——排版与渲染是两条独立解析路径。
`set_glyph_font` 只写后者，`set_otf_font` **两个都写**。CJK 字体必须用
`set_otf_font`（Fandol 没有 TFM）：只调 `set_glyph_font` 会得到
「字体加载失败、整段中文消失」，只写度量侧则得到「排得出来但渲染成方框」。

## 验证边界（重要）

- `cargo test --workspace`（native）全绿，其中 `crates/ntex-wasm` 的 **21 个单测**在
  **native 上跑与 wasm 完全相同的管线**（`compile_pipeline`：内嵌 TFM 注册 →
  `MemVfs` → `typeset_dvi` → DVI；`render_page_core`：盒树 → `prims` → `Pixmap`
  软光栅）——因为 TFM 注入缝与渲染核心函数不分目标编译（见
  `ntex-layout/src/typeset/wasm_fonts.rs` 的取舍说明）。渲染测试锁：A4 尺寸
  （595×842@72dpi / 1191×1684@144dpi）、白底有墨、debug overlay 增墨、
  越界页码与 dpi=0 可恢复报错（不 panic）。
  2026-09-11 新增 4 个回归锁（针对 Tauri 中文故障）：
  `embedded_tfms_cover_plain_preload_set`（内嵌 TFM 必须覆盖 plain 预载字体块
  全集——防清单再漂移）、`utf8_input_switch_reaches_engine_and_yields_to_source`
  （引擎参数开关不被源文件覆盖）、`otf_injection_enables_cjk_typesetting`
  （`set_otf_font` 后中文可排）、`resume_plain_compiles_clean_under_wasm_font_set`
  （真文档 `resume-plain.tex` 在 wasm 字体集下零错误编译）。
  2026-09-18 新增 5 个 C 档锁（**测试直接从入库 `assets/` 现读 fmt/tex/tfm 组装
  资产包**，即测的就是发行资产本体，不是测试夹具）：
  `latex_bundle_typesets_article_class`（`\documentclass{article}` 零 Undefined、
  零 not-found、出页，`cmbx12` 进 DVI 字体表）、
  `bundle_parse_classifies_entries_and_rejects_corruption`（坏魔数/截断/非法
  kind/坏 fmt 各自拒绝，**整体拒绝不半信半疑**）、
  `latex_assets_without_format_fall_back_to_plain`（无 fmt 回落 plain，不炸）、
  `missing_latex_package_error_carries_first_error_line`（缺宏包错误带转录
  首现场 `!` 行）、`injected_tfm_keys_are_bare_font_names`（TFM 键必须是裸字体名
  `cmbx12`——曾经用 `cmbx12.tfm` 导致 `Font cmbx12 not loadable`）。
- wasm 目标验证：`cargo check/build --target wasm32-unknown-unknown` 通过；本刀
  （2026-09-06）已做**真实浏览器端到端**（Chromium + wasm-bindgen --target web）：
  demo 作业 canvas 1191×1684、墨迹 ~9×10⁴ px、编译 11ms + 渲染 9.4ms；翻页仅
  重渲染（6.4ms，不重排版）；编辑输入 250ms 防抖重排重渲正常。
- TRIP：native 上 `cargo run -p ntex-trip -- --driver ntex --test trip` 失败签名与
  基线一致（TRIP 在主线本就是进行中状态，见 plan.md §6/M1；本刀未改变其结果）。

## 已知缺口（如实记录）

- `\write` 到非 16 流 / `\openout` 产出的文件留在 `MemVfs` 内不回传：
  `ntex-io::MemVfs` 暂无枚举 API（只有 `read`/`write`/`append`/`get`），补枚举接口
  属 ntex-io 领地，需另开一刀。
- 数学字体（cmmi/cmsy/cmex）已内嵌；plain 路径下 `\textfont`/`\scriptfont` 装配
  仍缺（**入包的只有 `latex.fmt`，没有 `plain.fmt`**）。C 档走 `.fmt` 时字体表按
  `FmtState::font_loads` 重建（`Typesetter::import_state`），故 LaTeX 侧的数学
  字号族定义能恢复；但**数学排版未纳入本次回归**（无对应锁，勿据此推断一致性）。
- **NFSS 字号切换的字体名污染——已修（2026-09-18，ntex-core）**：此前正文写
  `\Large` 会得到 `! Font cmr12 at 14.39999pt not loadable`（文字回落 nullfont、
  0 页）。根因是 `more_name` 的空格判据只认 cat-10，而 NFSS `\external@font`
  产出的空格以 cat-12 进入名字扫描。按 pdftex 三案对拍（`/tmp/ntex-repro/
  {at12,qt,qt2}.tex`，TeX Live 2024）修 `ntex-core/expand/io.rs`：**字符码 32
  一律终止名字**——cat-10 空格终止后放回（引号名后空格存活为 glue，既有锁
  不变）、非 cat-10 空格终止后消费（`at`/`scaled` 照常被 `scan_keyword` 识别）。
  修复后 `\Large`/12pt 全部零缺字体、出页（锁 `large_size_switch_loads_cmr12_
  without_pollution`、ntex-core `cat12_space_terminates_names_like_tex_web`）。
  残留小事：cat-12 空格的**来源**未追（NFSS 路径里为何不是 cat-10，`\edef`
  复刻不触发）——后果仅是 `at` 字号被 "Missing number → Improper at size →
  设计字号" 恢复链吃掉（与真 pdftex 同款行为），字号略偏、文本完好。
- **C 档剩余 —— LaTeX `tabular` 对齐前言（下一刀）**：裸 `\halign{#\cr a\cr}`
  正常（0 错），但 `\begin{tabular}{l}x\end{tabular}` 报
  `! Missing { inserted` → `! Missing # inserted in alignment preamble.`
  （最小复现：`/tmp/ntex-repro/run23.mjs`，`tabular_one`）。根因在 latex.ltx
  `\@mkpream`/`\@arstrut` 前言构造路径与 ntex-core 对齐机制的某处交互——
  `samples/latex-sample2e-slim.tex`（`\shortex` 即 tabular 树）当前因此 0 页；`\node`、
  `\enumsentence`（仅 1 条 list 相关错）基本可用。
- **宏包闭包**：`lingmacros`/`tree-dvips` 已入 `assets/tex-minimal/tex/latex/
  misc/`（2026-09-18 取自 CTAN `/macros/latex209/contrib/trees/tree-dvips`，
  阿里云镜像；CTAN 主站与清华镜像被网络策略拦截）。`\usepackage` 未覆盖时
  仍会报错并带转录首现场（`首现场：! LaTeX Error: File 'xxx.sty' not found.`）。
  tree-dvips 的 `\special{ps:...}`（dvips 画树线）引擎按 whatsit 节点吞掉——
  树形连线不可视，文本结构完好。
- 无增量接口（M5 的 `IncrementalTypesetter` 暴露给 JS 是 B 档「毫秒级刷新」的前置；
  当前每次编辑全量重排，demo 级文档毫秒级完成，大文档会线性变慢）。
- 真字形未做（见偏差 4）；vello/wgpu web 后端未做（B 档后续，体积/宿主要求另评估）。
