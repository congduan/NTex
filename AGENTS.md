# AGENTS.md — NTex 仓库协作指南

> 面向在此仓库工作的 AI Agent 与人类开发者。**开工前先读本文件 + plan.md + ETRIP-primitives.md**，
> 三份文档是工程状态与约定的事实来源。

---

## 1. 项目是什么

NTex 是一个 **100% 兼容 LaTeX/TeX 宏机制的现代排版 & 渲染引擎**：不修改宏语义，
只把 1970 年代的 C 解释器重写为"现代 VM + 增量计算 + 并行布局 + GPU 渲染"。

三阶段解耦：

```
TeX/LaTeX 源码 → 0.输入层 → 1.TeX VM 求值(宏展开,阶段1) → 纯节点流
  → 2.布局引擎(折行/断页/数学,阶段2) → 盒子树 + \shipout → DVI
  → 3.渲染后端(阶段3): ntex-pdf(DVI→PDF) / Skia / WASM
```

**关键架构决策**（见 RFC-1 / RFC-4 / RFC-3）：
- Token = **8 字节 tagged union（u64）**；控制序列 token 只含 `csid`（u32），定义一律查 eqtb 槽
  （`Undefined/Macro{version,def}/Primitive/Register/RegisterIndex/Alias`）；
- 宏体 = **连续不可变 TokenArray** + 字节码双表示；InternTable 线性化，csid=u32 下标；
- **字节码是默认执行路径**（`Expander::new()`），解释器退为调试工具，双轨等价测试保留；
- **排版事件经 `TokenSink` 单向流出**（token/组/原语/glue/kern/penalty/rule/内部参数），
  VM 不依赖布局 crate；
- **副作用隔离（RFC-3）**：`\write` 延迟到 shipout 边界提交，`\write18`（shell）拒绝；
  `\input`/读写流走 `ntex-io` VFS（LocalVfs/MemVfs）。

## 2. 快速命令（质量门禁）

```bash
make check      # 门禁三件套 = fmt + clippy(-D warnings) + 单元测试（提交前必须全绿）
make fmt        # cargo fmt --all -- --check
make lint       # cargo clippy --workspace --all-targets -- -D warnings
make test       # cargo test --workspace
make trip       # TRIP 管路（stub 驱动冒烟）
make diff       # 差分管路（stub vs stub）
make bench      # 基准管路（stub 驱动冒烟）
make fixtures       # 获取 TRIP/ETRIP fixtures（kpsewhich 优先，失败则下载）
make fixture-extras  # 获取补充对照 fixtures（pdftex expanded.{tex,txt} 等，详见 fixtures/README.md）

# 端到端演示
cargo run -p ntex-dvi -- demo.tex   # → demo.dvi
cargo run -p ntex-pdf -- demo.dvi   # → demo.pdf
cargo run -p ntex-backend -- demo.tex demo 144 --vello  # → demo-01.png…（vello GPU；去掉 --vello 走软光栅）
cargo run -p ntex-backend -- demo.tex demo 144 --debug  # → demo-01-debug.png…（排版调试 overlay：盒边界/glue/断点标记，独立通道不影响正常渲染）
```

**重要限制**：`[profile.release]` 开了 `panic = "abort"` + `lto` + `codegen-units = 1`，
`cargo test --release` 会失败——**测试一律用默认 dev profile**（CI 亦如此）。

## 3. 工作区结构与职责

| Crate | 职责 |
|---|---|
| `ntex-core` | token/catcode/InternTable/eqtb 版本化/展开引擎/字节码 IR·编译器·执行器/内部参数/sink 事件流 |
| `ntex-layout` | 排版核心：主循环（模式状态机）/折行/断页/输出例程/数学（`typeset/` 目录） |
| `ntex-font` | TFM 解析与字体度量（ttf/HarfBuzz 待 M9） |
| `ntex-dvi` | DVI 写出器，与真实 TeX 逐字节一致 |
| `ntex-pdf` | DVI → PDF 正式后端（Type1/PFB 字体嵌入） |
| `ntex-io` | VFS 抽象 + LocalVfs/MemVfs（RFC-3） |
| `ntex-format` | `.fmt` 序列化/反序列化（v1 内存快照完成，v2 待 M7） |
| `ntex-test-support` | 测试/差分/基准基础设施（EngineDriver 抽象） |
| `ntex-trip` | TRIP/ETRIP 一致性测试框架（`--test trip|etrip|both`，ntex in-process 驱动） |
| `ntex-diff` | 差分测试工具（参考引擎 vs 本引擎） |
| `ntex-bench` | 基准框架 |
| `ntex-backend` | 渲染后端（M8）：`Backend` trait + 软光栅 + vello 0.10 GPU 实现（wgpu 29 无头纹理回读、area 亚像素 AA；与软光栅共享 prims 矩形遍历，位图可差分）+ 自研 PNG 导出 |

常用源文件布局：
- `crates/ntex-core/src/expand/`：展开引擎拆分目录——`builtins.rs`（原语注册）、`primitive.rs`
  （原语处理器）、`expr.rs`（\numexpr/\dimexpr/\glueexpr 等）、`cond.rs`、`macros.rs`、
  `scan.rs`、`save.rs`、`free.rs`、`io.rs`、`tests.rs`；
- `crates/ntex-core/src/eqtb/`：`mod.rs`（槽模型）+ `primitive.rs`（Primitive 枚举）；
- `crates/ntex-layout/src/typeset/`：`typesetter.rs`（主循环）/`paragraph.rs`/`paging.rs`/`math.rs`/`sink.rs`。

## 4. 编码规范

- **工程级 lint**：workspace 统一 `unsafe_code = "deny"`，clippy 全开——严禁 `unsafe`，禁止 `catch_unwind`；
- **错误模型**：错误走 `Result`/`Error`，输入可达路径禁止 `unwrap`/`expect`/`panic`
  （引擎契约：**任意畸形输入不 panic**）；错误类型不带 blanket `From<io::Error>`，
  强制带操作意图上下文；
- **注释/文档/提交信息用中文**；模块顶部文档引用里程碑 ID（如 `M1-4`）与 RFC 章节
  （如 `RFC-1 §3`）；代码改动需同步更新对应注释（历史上多次出现注释与实现脱节）；
- **格式**：`rustfmt.toml` = edition 2021 / max_width 100 / use_field_init_shorthand；
- **关键常量现状**：`Primitive` 枚举 `repr(u16)`（变体数超 256）；`REGISTER_COUNT = 32768`
  （256 → 32768 扩展）；misc 整数参数至 33（29-32 号：\tracingparagraphs/\pagediscards/
  \splitdiscards/\lostchars）；
- **新增原语的完整链路**（缺一不可，参照既有提交）：
  1. `eqtb/primitive.rs` 加 `Primitive` 变体；
  2. `expand/builtins.rs` 注册内建 cs；
  3. sink/查询接口（`sink.rs` TokenSink 事件或查询方法）打通 VM → 排版器；
  4. `expand/primitive.rs`（或对应模块）实现处理器语义；
  5. 单元测试（对照 pdfTeX/tex.web 行为）；
  6. **更新 [ETRIP-primitives.md](ETRIP-primitives.md)**（唯一状态源）与 plan.md 相关条目。

## 5. 测试纪律

- **提交前 `make check` 全绿**（fmt + clippy -D warnings + 单元测试）；
- 测试金字塔：单元测试（语义锁消息/逐位对照）> 差分测试（同一 .tex 双引擎 diff）>
  TRIP/ETRIP（一致性硬口径）；M2 双轨等价框架（解释器 vs 字节码）必须保持绿；
- **ETRIP 冲刺纪律**（plan.md §6）：
  - 每次迭代前先 `cargo build -p ntex-trip` 确认全绿再跑（避免脏构建旧产物误报）；
  - 对照 `etrip.log` 参考**逐段验证**，不做整体 diff；
  - 进度与状态一律更新到 ETRIP-primitives.md；
- 涉及排版一致性/折行结果，验证方式：`demo.tex → DVI` 与 dvipdfmx/真实 TeX 对照。

## 6. 文档与状态同步（重要）

- [ETRIP-primitives.md](ETRIP-primitives.md) = ETRIP 原语状态**唯一状态源**
  （2026-08-23 更新：A 组 41/42、B 组 36/36、C 组全部已接线、收尾 etrip.log 逐字节比对未开始；`\muexpr` 待校准）；
- [fixtures/README.md](fixtures/README.md) = 补充对照 fixtures 状态表与抓取约定；
  当前入库：`pdftex/expanded.{tex,txt}`（David Carlisle/Bruno Le Floch 2018, Public Domain，
  pdftex `\expanded` 原语 12 个端到端 + 36 行期望基线）；抓取：`make fixture-extras`；
- [plan.md](plan.md) = 里程碑进度 + 性能 backlog（P0 已提交 80022b4；P1 热路径消分配/
  panic 审计 + fuzz；P2 CI 门禁）——完成事项后同步勾选；
- [REVIEW-2026-08-23.md](REVIEW-2026-08-23.md) = 最近一次代码审查（A 正确性 / B 性能 /
  C 工程健壮性 / D 语义简化点 / F 优先级路线图）。已修：A2 空行 → `\par`
  （input.rs 有测试覆盖）；未修：A1 死循环根因（watchdog 只是诊断，不是修复）、
  A4 `\outer` 语义、A5 输入层 UTF-8 方案等——动相关模块前先查此表；
- 提交信息风格（中文，`feat:` 开头，冒号后空格）：
  `feat: ETRIP 冲刺迭代 —— 表达式 i128 中间量 + 胶水阶语义`。

## 7. 已知简化点（改动前务必知悉）

- `\insert` 只收集不排版（脚注不可用）；数学矩阵未做；`\scriptfont` 未接真实字体；
- `.fmt` 快照不含字体表（加载后需重新 `\font`）；
- `\write` 已在 shipout 边界提交；`\muexpr` 的 1mu=1pt 与 `\the` "5.0mu" 显示待校准；
- 数学模式下矩阵/对齐环境（\matrix/\eqalign 等）尚未实现。

## 8. 里程碑一览（当前焦点）

M0 地基 ✅ → M1 展开内核 🟡（TRIP 未全绿）→ M2 字节码 🟡（吞吐 1.12x 未达 2x）→
M3 排版核心 ✅（DVI 逐字节一致）→ M4 数学 + e-TeX ✅（**ETRIP 冲刺进行中**）→
输出端 🟢（ntex-pdf 正式后端）→ M5~M9 未开始。
**当前唯一主线：ETRIP 冲刺**（A/B 组原语基本完成 → `etrip.log` 逐字节比对收尾）。
