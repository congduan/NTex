# ntex-wasm — NTex 引擎核心的 WASM 薄壳（M8-A 骨架 + B 档第一刀）

在浏览器（或 Node）里跑 NTex：喂 `.tex` 字符串 + 内嵌 TFM 度量，引擎排版，
回传 **DVI 字节 + 转录文本**，并可**逐页软光栅渲染到 canvas**（实时预览工作台）。
plain 子集、无 LaTeX。

## 三档路线

| 档 | 内容 | 状态 |
|---|---|---|
| **A（本 crate）** | 引擎核心 WASM 化：`compile_tex()` 全链在 wasm 内完成，TFM 经 `ntex_layout::set_tfm_source` 注入（`fonts/` 内嵌 6 个 CM TFM，共 7.7 KB） | ✅ 已建（2026-09-06） |
| **B** | 渲染。**第一刀已建（2026-09-06）**：`compile_document()` → `Document` 句柄（页盒树常驻）→ `render_page()` 走 ntex-backend **软光栅**（`prims` 事实源 + `Pixmap`，与桌面同代码；vello 经 feature 门控不进 wasm），RGBA 回传 JS `putImageData`；翻页/调 dpi/切 overlay 不重排版。后续：vello/wgpu web 后端、真字形（内嵌 LM OTF）、增量接口 | 🟡 第一刀已建 |
| **C** | LaTeX：`.fmt` 快照经 `MemVfs` 喂入 + `Typesetter::import_state`，在薄壳上加 `load_format(bytes)` | 等 `ntex-format` 快照完备（C 档依赖载入战） |

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
import init, { compile_document, compile_tex, engine_version, embedded_fonts, demo_tex }
    from './pkg/ntex_wasm.js';
await init();

console.log(engine_version());            // "NTex WASM 0.1.0 (plain subset; ...)"
console.log(embedded_fonts());            // ["cmr10","cmbx10","cmti10","cmmi10","cmsy10","cmex10"]

// —— B 档第一刀：编译 → 句柄 → 逐页软光栅渲染 ——
const doc = compile_document(demo_tex());
doc.page_count;                           // 2（\shipout 页数）
doc.transcript;                           // TeX .log 主体
doc.dvi;                                  // Uint8Array，DVI 字节流（dvipdfmx 类驱动可消费）

const img = doc.render_page(0, 144, /*debug=*/false);   // 第 0 页 @144dpi → RGBA
// img.width × img.height（A4@144dpi = 1191×1684），img.rgba = Uint8Array(w*h*4)
ctx.putImageData(new ImageData(new Uint8ClampedArray(img.rgba), img.width, img.height), 0, 0);

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
4. **渲染口径（B 档第一刀）**：软光栅 + 占位方框——字形通道依赖 kpsewhich/texlive
   定位 Latin Modern OTF（ntex-backend `glyphs.rs`），wasm 无文件系统恒回落方框，
   且软光栅本就不走字形通道（桌面 `ntex-backend` 软光栅同口径）；规则/glue/盒子
   几何与桌面路径逐位同源（`prims.rs` 事实源）。真字形（内嵌 LM OTF）属 B 档后续。

除上述四处，wasm 与 native 跑的是同一份引擎代码；native 行为零改动（门控只在
wasm32 目标 / feature 组合下生效——`ntex-backend` 的 `vello` feature 为 default，
native 路径不受影响）。

## 验证边界（重要）

- `cargo test --workspace`（native）全绿，其中 `crates/ntex-wasm` 的 6 个单测在
  **native 上跑与 wasm 完全相同的管线**（`compile_pipeline`：内嵌 TFM 注册 →
  `MemVfs` → `typeset_dvi` → DVI；`render_page_core`：盒树 → `prims` → `Pixmap`
  软光栅）——因为 TFM 注入缝与渲染核心函数不分目标编译（见
  `ntex-layout/src/typeset/wasm_fonts.rs` 的取舍说明）。渲染测试锁：A4 尺寸
  （595×842@72dpi / 1191×1684@144dpi）、白底有墨、debug overlay 增墨、
  越界页码与 dpi=0 可恢复报错（不 panic）。
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
- 数学字体（cmmi/cmsy/cmex）已内嵌但 demo 未用数学——plain 格式的
  `\textfont`/`\scriptfont` 装配属 C 档 `.fmt` 路线。
- 无增量接口（M5 的 `IncrementalTypesetter` 暴露给 JS 是 B 档「毫秒级刷新」的前置；
  当前每次编辑全量重排，demo 级文档毫秒级完成，大文档会线性变慢）。
- 真字形未做（见偏差 4）；vello/wgpu web 后端未做（B 档后续，体积/宿主要求另评估）。
