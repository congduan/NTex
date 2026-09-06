# ntex-wasm — NTex 引擎核心的 WASM 薄壳（M8-A 骨架线）

在浏览器（或 Node）里跑 NTex：喂 `.tex` 字符串 + 内嵌 TFM 度量，引擎排版，
回传 **DVI 字节 + 转录文本**。plain 子集、无渲染、无 LaTeX。

## 三档路线

| 档 | 内容 | 状态 |
|---|---|---|
| **A（本 crate）** | 引擎核心 WASM 化：`compile_tex()` 全链在 wasm 内完成，TFM 经 `ntex_layout::set_tfm_source` 注入（`fonts/` 内嵌 6 个 CM TFM，共 7.7 KB） | ✅ 已建（2026-09-06） |
| **B** | 渲染：DVI/盒子树 → vello/wgpu web 后端画到 canvas（与 `ntex-backend` 桌面路径共用 `build_scene` 的 prims 事实源） | 另案未做 |
| **C** | LaTeX：`.fmt` 快照经 `MemVfs` 喂入 + `Typesetter::import_state`，在薄壳上加 `load_format(bytes)` | 等 `ntex-format` 快照完备（C 档依赖载入战） |

## 构建

```bash
rustup target add wasm32-unknown-unknown
cargo check -p ntex-wasm --target wasm32-unknown-unknown   # 编译验证
cargo build -p ntex-wasm --target wasm32-unknown-unknown --release
# 产物：target/wasm32-unknown-unknown/release/ntex_wasm.wasm
```

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

`www/` 有最小示例页（编译随包 demo 并打印 DVI 长度 / 转录）：

```bash
wasm-bindgen --out-dir www/pkg --target web \
    target/wasm32-unknown-unknown/release/ntex_wasm.wasm
cd www && python3 -m http.server 8000   # 浏览器打开 http://localhost:8000
```

JS API（wasm-bindgen 生成后）：

```js
import init, { compile_tex, engine_version, embedded_fonts, demo_tex }
    from './pkg/ntex_wasm.js';
await init();

console.log(engine_version());            // "NTex WASM 0.1.0 (plain subset; ...)"
console.log(embedded_fonts());            // ["cmr10","cmbx10","cmti10","cmmi10","cmsy10","cmex10"]

const r = compile_tex(demo_tex());        // 随包示例
const bytes = r.dvi;                      // Uint8Array，DVI 字节流
const log   = r.transcript;               // TeX .log 主体（\message/\show/\write16/错误上下文）
console.log(r.page_count, r.fonts);       // 页数 + 字体清单

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

除上述三处，wasm 与 native 跑的是同一份引擎代码；native 行为零改动（门控只在
wasm32 目标下编译进 wasm 分支）。

## 验证边界（重要）

- `cargo test --workspace`（native）全绿，其中 `crates/ntex-wasm` 的 4 个单测在
  **native 上跑与 wasm 完全相同的管线**（`compile_pipeline`：内嵌 TFM 注册 →
  `MemVfs` → `typeset_dvi` → DVI）——因为 TFM 注入缝不分目标编译（见
  `ntex-layout/src/typeset/wasm_fonts.rs` 的取舍说明）。这保证引擎逻辑被真实回归。
- wasm 目标验证：`cargo check --target wasm32-unknown-unknown` 通过（本仓库交付时点）。
  若环境装有 wasm-bindgen CLI，跑 `www/` demo 页即完成端到端冒烟；CLI 装不上时
  **未做** JS 侧运行时验证（wasm-bindgen 的 String/Uint8Array 编组层未经真实
  浏览器/Node 执行）——这是 A 档的已知验证缺口，不是引擎逻辑缺口。
- TRIP：native 上 `cargo run -p ntex-trip -- --driver ntex --test trip` 失败签名与
  基线一致（TRIP 在主线本就是进行中状态，见 plan.md §6/M1；本刀未改变其结果）。

## 已知缺口（A 档不做，如实记录）

- `\write` 到非 16 流 / `\openout` 产出的文件留在 `MemVfs` 内不回传：
  `ntex-io::MemVfs` 暂无枚举 API（只有 `read`/`write`/`append`/`get`），补枚举接口
  属 ntex-io 领地，需另开一刀。
- 数学字体（cmmi/cmsy/cmex）已内嵌但 demo 未用数学——plain 格式的
  `\textfont`/`\scriptfont` 装配属 C 档 `.fmt` 路线。
- 无增量接口（M5 的 `IncrementalTypesetter` 暴露给 JS 是 B 档「毫秒级刷新」的前置）。
