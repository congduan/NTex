# NTex 整体进度

> **本文件是全仓唯一的进度文档**（2026-09-19 重构）。
> 任何"做到哪了 / 还差什么 / 下一步做什么"只信这里，不再散落在多份文档。
>
> | 你要找的 | 去哪 |
> |---|---|
> | 目录结构、crate 职责 | [AGENTS.md](AGENTS.md) |
> | 构建命令、开发约定 | [README.md](README.md) |
> | 架构决策（已定稿） | [RFC-1](RFC-1-token.md) / [RFC-3](RFC-3-side-effects.md) / [RFC-4](RFC-4-bytecode.md) / [RFC-5](RFC-5-parallel.md) |
> | 架构构想（愿景） | [idea.md](idea.md) |
> | 技术债明细 | [docs/KNOWN-SIMPLIFICATIONS.md](docs/KNOWN-SIMPLIFICATIONS.md) |
> | 定位工具与判读纪律 | [docs/tooling-trust.md](docs/tooling-trust.md) |
> | 历史逐刀战报、勘察全文 | [docs/archive/](docs/archive/README.md) |
>
> **维护纪律**：进度变化只改本文件；一刀闭环后，逐刀细节移入 `docs/archive/`，
> 正文只留结论 + 归档指针（归档文件按原名保留，便于全文检索）。

---

## 0. 一页速览

### 0.1 当前焦点：LaTeX/expl3 兼容战役（唯一主线）

**目标**：`latex.ltx` 加载成功 → `latex.fmt` 构建 → `\documentclass{article}` → PDF。

| 阶段 | 状态 | 关键数字 |
|---|---|---|
| plain.tex 预载 | ✅ G0–G3 完成 | 1241 行全通；`\newif` 端到端与 pdfTeX 一致；剩 G4（`\lccode/\uccode` 初表）、G5（初表分裂脑收敛） |
| expl3 全文载入 | ✅ 载入走通 | `[LOAD-DONE]` + DVI 落盘；载入期错误 2492 → **4**（NTex 独有仅 1，不阻断） |
| latex.ltx 主体加载 | 🟡 **当前主墙** | 实测停点 **pos=685828（88.4%）**、500s 超时、steps=11180000；根因待定 |
| `\documentclass` / article.cls | ⬜ 未开始 | — |
| 结构宏（`\maketitle`/`\section`）+ NFSS 字体 | 🟡 部分 | `\section` 链已推进；NFSS 字号墙未越过 |

**最近两刀**（2026-09-17 / 09-18）：

- **第十八刀** 尾递归鞍具校准 → `make check` 797 → **800 全绿**；pdfTeX 对照证明原否定结论有误；
  **真实主墙仍未越过**（输入栈泄漏与 Call/Ret 均已证伪）。
- **第二十五刀** `\the\value{counter}` 操作数宏展开 → corpus `\the` 同源错误群清零
  （`small2e` `^!` 7→4、`lppl` 60→31、`sec1` `\the` 错 1→0）；`ntex-core` 435 passed。

**fmt 快照重建刀**（2026-09-20）：真实论文（arXiv 1706.03762 standalone）`a\_b` 死循环根因 =
**陈旧 fmt 快照**——`assets/fmt/latex.fmt` 建于 09-19 00:33，早于 `883b9b3` active 槽隔离；旧槽
模型下 latex.ltx L15916 `\gdef_{\_}` 把 active `_` 定义写进同名单 cs `\_` 槽，robust 壳被覆盖成
自引用体（`\meaning\_` = `macro:->\_`，3700 万步不收敛）。引擎侧 `exec_def` 的 `active_slot`
臂已正确（plain 路径探针实证），重生成 fmt 即愈：最小复现文本/数学模式出 DVI，
`\copyright`/`\sqrt`/`\textunderscore` 无回归；paper 推进到 l.342 `tabular`（`\if 缺 \fi`，
下一堵墙，halign 域）。重建曾被宿主依赖卡住：`\InputIfFileExists{hyphen.cfg}` 命中宿主 TinyTeX
babel 配置（引用宿主缺失的 dehypht-x-2024-02-28.tex）→ 断字配置已钉进发行树（`hyphen.cfg`
垫片转投官方回退 `hyphen.ltx` + Knuth `hyphen.tex` 入库），默认 `--generate-fmt` 从此确定性。
**流程教训：引擎语义提交后必须重生成发行 fmt**（README「引擎语义变更后重生成发行 fmt」本次漏执行）。
新露头残差（登记未修）：auxiii `\ifx\reserved@a\reserved@b` 判 NE（pdfTeX GT 判 EQ），robust
壳缺 `\x@protect ⟨cs⟩` 前缀（嫌疑 286eddc \meaning 语义变更）——\write 文本面偏差，非阻断。

**新墙（只登记未修）**：`\ifdim` 的 NFSS size range 解析 ——
`Missing number <to be read again> \@M`（裸 `\show\@M` = `\mathchar"2710`，`\number\@M` = 10000）。

**expl3 官方套件口径**（l3kernel `.lvt` × 187）：载入终点 **l.36005 = 89.4% 行 / 90.0% 字节**，
载入期真错 573 → **1**（不阻断）；墙 = `\__codepoint_finalize_blocks_aux:n` 的 `\__int_step:Nw` 块循环。
剩余 10.6% 行 = codepoint 收尾 / l3text 五段 / l3legacy+l3deprecation / 文件尾 / exgeneric 尾。

### 0.2 各线状态

| 线 | 状态 |
|---|---|
| 主线：LaTeX/expl3 兼容 | 🟡 88.4% 主墙（见 §0.1） |
| 输出例程战 | ✅ 刀 1–5 全落（`\outputpenalty`/box255/insert 分配/`\newinsert`/页号链） |
| 格式预载（G 线） | ✅ G0–G3；剩 G4/G5 |
| M5 增量计算 | 🟡 阶段一~五完成（改正文 4.3x / 改宏体 1.1x）；阶段六待办见 §2 M5 |
| M9 中文 | 🟡 刀 1/2/3/5 落地（`\char` 扩位 + `\utfinputmode` + 宿主端端到端 + CJK 断点/断字最小宽）；遗留见 §5 P1 |
| M2 字节码 | 🟡 双轨等价全绿；吞吐 **1.01x**，≥2x 结构性不可达 → 转 M7 `.fmt` v2 |
| 渲染/预览端 | ✅ PDF 后端 + 软光栅/vello + studio + wasm/tauri 工作台可用；真字形与缩放等遗留见 §2 M8 |
| 技术债 | 见 §5；明细 [docs/KNOWN-SIMPLIFICATIONS.md](docs/KNOWN-SIMPLIFICATIONS.md)（待做 33 处 / 已修 ✅ 29 处 / 部分 ⚠️ 8 处，按行计，最近维护 2026-09-18） |

### 0.3 KPI 与口径登记

- **corpus 探针**：⚠ **两处口径冲突，已登记待对齐** ——
  本节此前记 **5/8 PASS + 3 EMPTY**（2026-09-17）；2026-09-19 在 `b08d77d` 后实测为
  **2/8 PASS + 0 EMPTY**（PASS = `math/basic-expressions`、`math/symbols-matrix`；
  plain 三例从 EMPTY 转 FAIL：仍缺 `\shipout` 类债；`small2e` 90s 超时）。
  待补一次对照说明；在那之前**以 09-19 实测为准**。
- **corpus 的 LaTeX PASS 是降级渲染**（`\documentclass` 等全 Undefined，只排出裸文字），
  真 LaTeX 结构/字体/版式未通 —— 不要把它读成 LaTeX 可用。
- **TRIP / ETRIP**：硬口径"逐字节全绿"已按 2026-09-03 决策解绑，改记 M8 L2 观测项
  （见 §2 M4）。当前收尾口径 = semantic diff 归零 + 错误块抽查。
- **EVIDENCE 口径**：任何数字入库须带 commit / 命令 / 样本规模；仪器失真史见
  [docs/tooling-trust.md](docs/tooling-trust.md)（开工定位前先读）。

### 0.4 下一步（按执行序）

1. **定根因**：latex.ltx 88.4% 主墙（输入栈泄漏、Call/Ret 均已证伪）——单点最大信息量。
2. **NFSS 字号墙**：`\ifdim` size range 解析 / `try@simples`；连带 `preload.ltx` l.47
   `\DeclarePreloadSizes` → `! \font 后缺少字体名`（当前致命终止点）。
3. **数学字母登记链**：`\SetMathAlphabet\mathsf/\mathit{bold}`（fontmath l.73/74）
   → ``Command `' not defined as a math alphabet`` ×2；`\in@` 与尾空格 csname 名构造已证伪，
   待查 `\alpha@list`/`\version@list` 与 `\meaning#4` 文本。
4. 越过 88.4% 后：ltoutput 尾段 / lttagging / ltfinal 至 `\dump`（约 11.6%）
   → article.cls → 结构宏 → 真 LaTeX 版面 PDF。
5. 并行线：M5 阶段六、M1-13 错误恢复通用机制、格式预载 G4/G5。

---

## 1. 里程碑总览

| # | 里程碑 | 核心交付 | 状态 | 验收标准（Exit Criteria） |
|---|---|---|---|---|
| M0 | 地基 | workspace / CI / 基准框架 / TRIP 框架 | ✅ 完成 | 基准脚本与测试框架跑通 |
| M1 | 解释器内核 | Token + 16 catcode + 宏展开 + 核心原语 | 🟡 核心完成；TRIP 未归零 | **TRIP 通过** |
| M2 | 字节码编译 | 字节码 IR + 编译器 + 执行器 | 🟡 双轨全绿；吞吐未达标 | TRIP 仍绿；吞吐 ≥ 解释器 2x（实测 **1.01x 未达，结构性**） |
| M3 | 排版核心 | 节点/盒子/胶水/Knuth-Plass/TFM/断页/DVI | ✅ 核心完成 | DVI 与真实 TeX 逐字节一致 |
| M4 | 数学 + e-TeX | 数学模式 / e-TeX 原语 / 断字 / 错误模型 | ✅ 功能完成；ETRIP 收尾中 | **ETRIP 通过**（逐字节口径已降级，见 §2 M4） |
| M5 | 增量计算 | 求值图 / 依赖追踪 / 副作用隔离 / VFS | 🟡 阶段一~五 | 改 1 处仅重算受影响段落（目标 100x；实测 4.3x/1.1x） |
| M6 | 并行 | 段落级并行展开 / 布局 / 整形 | ⬜ 未开始 | 8 核加速比 ≥ 3x（300 页基准） |
| M7 | `.fmt` v2 | 三层 fmt + 部分求值 + 项目级 fmt | ⬜ 未开始（v1 已可用） | `latex.ltx` 加载 <200ms；ctex 300 页冷编 <10s |
| M8 | 渲染 / 输出 | PDF 后端 + GPU 渲染 + WASM 预览 | 🟢 主体可用 | 简单文档 L2 逐字节一致；WASM 增量预览流畅 |
| M9 | 生态冲刺 | ctex/xeCJK/OpenType/CJK 捷径 | 🟡 中文刀 1–5 落地；宏包管理解析/契约层落地（`ntex-pkg`） | 目标宏包 CI 回归全绿 |

**依赖链**：M0 → M1 → M2 → M3 → M4 → M5 → M6 → M7 → M8 → M9
（M5 依赖 M2/M3；M7 依赖 M2/M5；M6 可与 M5 部分并行）。
**北极星对照**：M5/M7/M8/M9 + LaTeX 兼容战役共同收口四项用户指标（见 §3）。

---

## 2. 里程碑明细

> 只记"当前状态 + 剩余项"；已闭环的实施细节在 `docs/archive/plan-full-2026-09-19.md`。

### M0 地基 ✅

- workspace / CI（`.github/workflows/ci.yml`：门禁 job + 观测 job）/ 差分工具 / 基准框架
  全部就绪。
- **RFC 状态**：RFC-1（Token 表示）、RFC-3（副作用模型）、RFC-4（字节码）**已定稿**；
  RFC-5（并行）在册；**RFC-2（不可变状态模型）未单独成文**——其内容已由 M5 实际落地
  （可回滚检查点 + 盒子寄存器写时复制），独立文档仍待补。
- **基准集**：已建"每千 token 展开吞吐"；未建——`latex.ltx` 加载耗时（M7 前无意义）、
  300 页中文冷编（M7 前）、增量"改 1 字重算耗时"（M5 前）。
- **剩余**：RFC-2 成文（低优先）。

### M1 解释器内核 🟡

核心（M1-1~7、M1-9~12、M1-15）已实现。逐项现状：

| 项 | 状态 |
|---|---|
| M1-1 Token 骨架 / M1-2 InternTable / M1-3 eqtb 槽 | ✅ |
| M1-4 输入与 catcode 固化 / M1-6 展开主循环 / M1-7 扫描顺序原语 | ✅ |
| M1-5 宏定义（`\def`/`\edef`/`\gdef`） | ✅（`\newcommand` 引擎侧未内建，LaTeX 宏层自备） |
| M1-8 参数匹配 | ✅ 无分隔 + 分隔均实现；`\long` 前缀已注册接线 |
| M1-9 条件原语 / M1-10 寄存器 / M1-11 组作用域 | ✅（M1-11 朴素快照回滚未改 eqtb 版本指针，O(1) 优化未做） |
| M1-12 模式状态机 | ✅ 由 `ntex-layout::typeset` 承接 |
| M1-13 错误模型 | 🟡 交互模式 + 上下文行 `l.N` 已做；**错误恢复通用机制（`back_input`/`\errhelp`）待补** ← P0 |
| M1-14 TRIP 冲刺 | 🟡 `trip.tex` 可全程跑完不 panic；semantic diff **−5619/+1412**，未归零 |
| M1-15 性能基线 | ✅ |

> 历史标注澄清：`\long`（M1-8）/`\chardef`（M1-3）/`\box`、`\muskip`（M1-10）
> 代码**均已实现并注册**，旧"待补/未实现"标注是文档滞后，勿再按旧标注排期。

### M2 字节码编译 🟡

- 已落地：定长 u64 IR（`Emit{token}` / `EmitArg{n}` / `End`）、定义期编译器、执行器、
  双轨等价框架（**100 用例等价全绿**）、原语 dispatch 表、字节码成为默认路径
  （`Expander::new()`，解释器退为调试工具）。
- **可选优化项已撤销**：常量条件折叠（A6 —— TeX 的 `\if*` 是展开期求值，`\let\iftrue\iffalse`
  会使折叠产物与运行期语义不符）。
- **未做**：M2-5 arena（token/指令 bump 分配）、`\expandafter`/`\futurelet` 专用指令、
  编译失败路径。
- **吞吐结论（2026-09-03，release，`ntex-bench expand-dual`）**：比值 **1.01x**，
  未达 2x 且属**结构性**——RFC-4 零解包 IR 与解释器 `TokenArray` 完全同构
  （均为 8B/token），执行器本体 <10%，Amdahl 上限远低于 2x。全管线
  `expand-throughput` 469ms / 42.6 万调用/s。**≥2x 需改 IR 设计或降共享成本 → 转 M7。**

### M3 排版核心 ✅

- 节点 / 盒子（`\hbox`/`\vbox`/`\vtop`）/ 胶水与 badness / Knuth-Plass 折行 /
  TFM 解析与字体表 / lig+kern + `\sfcode` / 断页 DP / `\output` 例程 + box255 /
  `\shipout` → DVI —— **全部完成**；`dvipdfmx` 实机验收，DVI 与真实 TeX 逐字节一致
  （残余差异仅排版器未实现字体 kern 表）。
- **RFC-3 VFS + 副作用模型**落地：10 个读写原语走 `ntex-io` VFS，`\write` 延迟到 shipout
  边界提交，`\write18` 拒绝。
- **`.fmt` v1** 内存快照（`ntex-format`：确定性编码 + roundtrip 测试）；mmap 零拷贝留 M7。

> `badness` 重复实现已合并；折行 active 集淘汰 + `\tolerance` 默认值 200 为评审 A1 修复项。

### M4 数学 + e-TeX ✅（功能）/ 🟡（收尾）

- **功能全落**：数学模式状态机（`$`/`$$`、8 类原子、spacing 表、上下标、字阶）、
  分式/根式/定界符、样式原语、fontdimen 数学参数 + 数学字体族、显示数学细化、
  Liang 断字 + `\patterns`、数学错误模型、e-TeX 核心与扩展
  （`\protected`/`\ifdefined`/`\ifcsname`/`\unless`/`\numexpr`/`\detokenize`/`\unexpanded`/
  `\dimexpr`/`\glueexpr`/`\ifprimitive`/`\scantokens`/`\eTeXversion`）。
- **ETRIP 原语**：A 组 **42/42 ✅**、B 组 **36/36 ✅**、C 组**全部已接线 ✅**
  （原语级明细见 [docs/archive/ETRIP-primitives.md](docs/archive/ETRIP-primitives.md)）。
- **收尾口径（2026-09-03 决策，仍有效）**：`etrip.log` 从"逐字节比对"降级为
  **semantic diff 归零 + 错误块抽查**（当前 **−3051/+2352**）；`\showbox`/`\showlists`
  诊断格式逐字节对齐、read-again 与 `l.N` 间的 `...` 省略行、两行光标、79 列断行等
  格式差**不再死磕**，留 M8 L2。
- **剩余 P0**：错误恢复通用机制（与 M1-13 同一项，见 §5）。

### M5 增量计算 🟡（阶段一~五）

铁律：**增量结果与全量重跑逐位一致**，全程由测试锁死。

| 阶段 | 内容 | 提交 |
|---|---|---|
| 一 | 文本级切段 + 段边界轻量快照 + 词法读写依赖 + `edit` 双路径 | `0099b1f` |
| 二 | 可回滚完整检查点 + `edit` 单路径化 + ChainDelta 偏差判定 | `c2a2418`（前置 `7601d1d`） |
| 三 | 段级增量延伸到排版层端到端（`IncrementalTypesetter`） | `b585864` |
| 四 | 排版层依赖判定（宏体编辑省重排）+ 四闸复用判定 | `55bcb09` |
| 五 | 复用判定开销削减（盒子寄存器写时复制）→ **增量墙钟首次 < 全量** | `14f7bc7` |

**实测（release / 120 段文档）**：改正文 **4.3x**（13.4ms vs 58ms）、改宏体 **1.1x**
（30.1ms vs 33ms）。目标 100x。

**阶段六待办**（= M5 全部剩余项）：

1. **检查点捕获增量维护** —— 执行段成本 ≈ 全量段成本（每段多付 ~55µs），
   宏体编辑场景加速比被压到 1.1x 的根因，须在 `ntex-core` 引擎侧解决；
2. **副作用边界** —— `\output` 例程回放 / `\write` 流内容 / `\input` VFS 注入 /
   marks 语义（当前观测到即保守全量重排）；
3. `Expand(source, snapshot)` **纯函数化**收尾；
4. 寄存器/参数**槽级归因**（消除偏差全局失效）；
5. **随机编辑模糊测试进 CI**（增量 vs 全量 diff 常驻，闭环本里程碑风险项）；
6. `.aux`/`.toc` 增量；
7. 内存：现为 O(文档) 量级。

### M6 并行 ⬜

段落级并行布局（rayon）、展开阶段并行（段边界 + 快照隔离 + 无跨段副作用）、
字体整形并行（为 M9 HarfBuzz 铺路）、确定性保证（每段独立快照、按段序合并）。
**暂缓**：等 M5 副作用边界成果落地。验收：8 核 ≥3x 且与单线程逐位一致。

### M7 `.fmt` v2 ⬜

三层 fmt（状态快照 / 预编译字节码 / 预计算索引）、部分求值、项目级 fmt、
mmap 只读 + 页级 COW。**暂缓理由已消解**：M2 吞吐已量化（1.01x，结构性），
不再是"先量化再冲"的阻塞点。验收：`latex.ltx` 加载 <200ms、ctex 300 页冷编 <10s。

### M8 渲染 / 输出 🟢（主体可用）

已落地：

- **正式 PDF 后端**：`ntex-pdf` 直出（DVI 解析 + PDF 1.4 写出 + Type1/PFB 嵌入）；
  与 dvipdfmx 渲染一致（墨水比 1.02~1.04，差异为字体提示/抗锯齿）。
- **渲染后端**：`ntex-backend` —— `Backend` trait + 软光栅 + **vello 0.10 GPU 路径**
  （wgpu 29 无头纹理回读、area 亚像素 AA），共享 prims 遍历可差分；自研 PNG 导出。
- **排版调试 overlay**（盒边界/glue/kern/断点，独立通道不影响正常渲染）。
- **实时预览工作台** `ntex-studio`（eframe/egui-wgpu 0.35 + vello 表面渲染、TeX 高亮、
  250ms 防抖、缩放/平移/翻页、LaTeX/plain 自动切换、Log 转录面板）。
  **依赖硬约束：eframe 0.35 ↔ vello 0.10 恰共用 wgpu 29**（升 0.36 会分裂）。
- **WASM 端**：`ntex-wasm` B/C 档（`compile_document` + `render_page` 软光栅、
  真字形轮廓、中文 OTF 度量+轮廓双侧注入、LaTeX 资产包 `NTEXBND1`）。
- **Tauri 壳**：`ntex-tauri` 纯壳工作台，排版与渲染全在前端 WASM 内；PDF 导出按需
  fetch PFB 注入。

剩余：

- [ ] **L2 字节兼容**：简单文档对照 pdfTeX 逐字节 diff（关时间戳/元数据随机性）——
  TRIP/ETRIP 的逐字节口径也归在此。
- [ ] **预览三端增量接线**：`IncrementalTypesetter` 暴露到 wasm（JS 绑定）/ studio / tauri；
  编辑只重排受影响段，预览刷新 <50ms。前置 = M5 阶段六副作用边界。
- [ ] vello web 后端（wasm 目前只走软光栅）。
- [ ] **SyncTeX 源码映射**（点击跳转 / 错误波浪线）；基础是 RFC-1 side-table 位置信息，
  与 M1-13 错误定位联动。
- [ ] wasm C 档剩余：`plain.fmt` 与数学回归、缩放字号（受 `ntex-core` 字体名扫描既有 bug 拖累）。

### M9 生态冲刺 🟡

**依赖链（严格顺序）**：LaTeX 战役 → ① 英文全链通 → ② 字体子系统（TTF/OTF + HarfBuzz）
→ ③ 输入层 UTF-8 → ④ CJK 原语面（对齐 XeTeX/LuaTeX）→ ⑤ ctex/xeCJK 兼容 →
⑥ 中文 300 页端到端验收。**阶段 A 未出口前不启动中文工作**（避免双线作战）。

已落地（中文刀 1–5）：

| 刀 | 内容 | 状态 |
|---|---|---|
| 1 | `\char` 上界按字体判定 + OTF 度量直映通道 | ✅ |
| 2 | `\utfinputmode`（misc 65）UTF-8 直写通路，默认 bytes 零改动 | ✅ |
| 3 | 宿主端端到端（Tauri/WASM）：内嵌 TFM 48 件 + OTF 度量缝 + 引擎级 UTF-8 开关 + `\fam\bffam` 误判修复 | ✅ |
| 5 | `\cjkbreakmode`（misc 66）CJK 字间断点 + 禁则；中西文交界断点 + `\lefthyphenmin`/`\righthyphenmin` | ✅（XeTeX 对照 oracle + `cmp` 逐字节零回归） |
| 4 | 中文粗体/斜体（现仅 Fandol Regular） | ⬜ |

**宏包管理（`ntex-pkg`，2026-09-19 新建）**：边界见 §6.2 第 8/9 条。已落地**解析层 + 契约层 + 取料层 ①②**——
`tlpdb.rs`（TLPDB 状态机解析 + `by_basename`/`by_path` 反查 + TDS 优先级 + `RELOC/` 归一）、`resolve.rs`
（`\usepackage`/`\documentclass`→包解析 + 依赖闭包 BFS）、`lock.rs`（`ntex.lock` 确定性编解码 + 漂移 `diff`）、
`cache.rs`（内容寻址缓存布局 + 名字安全 + **真实 SHA-512 字节哈希**）、
`source.rs`（`PackageSource` trait + ① 本地 TeX Live 树源）、
`tlnet.rs`（② tlnet 镜像：URL 由 `revision` 钉死 + 下载后 SHA-512 校验 + 容器按 `runfiles` 裁剪）、
`vendor.rs`（闭包物化成 TDS 子树；四态逐字节比对，源缺失显式报错并阻塞收敛）、
CLI `ntex-pkg`（`index`/`provide`/`resolve`/`lock`/`check`/`local`/**`vendor`**/**`fetch`**）。
取料层 ③ CTAN / ④ 离线归档仍为**显式未实现插口**（不静默降级）。
验收：解析层真实 TLPDB + `kpsewhich` oracle 9/9；② 协议层离线假边缘全覆盖 + 真实镜像端到端（见待办项）。

剩余清单：

- [ ] 中文粗体/斜体（刀 4）；`\catcode` >255 赋值扩展（A5 全量收口）
- [ ] CJK 标点挤压、HarfBuzz 整形与整形缓存、**CJK 整形捷径**（无复杂特性时跳过 HarfBuzz 直读 hmtx，目标 5~10x）
- [ ] ctex/xeCJK 宏兼容（从 xeCJK 最小子集起步）；OpenType fontspec 路径
- [ ] 宏包 CI 回归集：geometry / amsmath / hyperref / biblatex / tikz / ctex
- [x] **宏包管理·解析/契约层**（2026-09-19，`ntex-pkg` crate，§6.2 第 8/9 条落地）：
  TLPDB 解析 + 文件反查索引 + `\usepackage`/`\documentclass`→包解析 + 依赖闭包 +
  `ntex.lock` 确定性契约（包名 + revision + sha512）+ 内容寻址缓存 + 可插拔取料源链
  （① 本地 TeX Live 树已实现）。**解析层验收**：真实 TLPDB（2024basic，346 记录）+
  `kpsewhich` 路径 oracle **9/9 一致**；73 单测 + `make check` 全绿。简化登记见
  [docs/KNOWN-SIMPLIFICATIONS.md §9](docs/KNOWN-SIMPLIFICATIONS.md)。
- [x] **宏包管理·取料层 ② + 资产物化**（2026-09-19，同日第二刀）：
  `tlnet.rs`（② tlnet 镜像：`<仓库>/archive/<包>.r<rev>.tar.xz` 由 revision 钉死、按 TLPDB
  `containerchecksum` 做**真实 SHA-512 字节校验**、容器条目按 `runfiles` 裁剪 + `RELOC/`→`texmf-dist/`
  重定位）+ `vendor.rs`（闭包 → TDS 子树物化，四态 `Add`/`Differ`/`Identical`/`SourceMissing`
  逐字节比对，源缺失**显式报告并阻塞收敛**）+ `Vfs::create_dir_all`（宿主侧建目录能力，默认
  no-op 不破平坦后端）+ `tlpdb.rs::install_rel_path`（已安装树与缓存树共用一条路径映射）+
  `tds_rank` 修为**先归一后判档** + CLI `vendor` / `fetch`。
  **验收**：`make check` 全绿（ntex-pkg **92** 单测）；真实镜像端到端（阿里云 CTAN 镜像 + 真实
  TL2026 TLPDB 20.7MB）：`fetch infwarerr` → 1 容器 1 文件 8.2KB（SHA-512 通过）；`fetch
  --documentclass article` → `latex.r79618` 171 文件 2.7MB；`vendor --write --lock` → 目标树
  171 文件、复跑**一致 171 / 新增 0**（幂等）；`resolve --documentclass article` 选定稳定版
  `latex` 而非 `latex-base-dev`（`tds_rank` 修复在真实库生效）。刻意简化（宿主 `curl`/`tar`
  当边缘、无 GPG 验签、无断点续传、缓存布局两套）见
  [docs/KNOWN-SIMPLIFICATIONS.md §9](docs/KNOWN-SIMPLIFICATIONS.md)。
- [ ] **宏包管理·取料与接线（余项）**：取料层 ③ CTAN / ④ 离线归档 + GPG detached 验签 +
  `\usepackage` 缺包即报（MiKTeX 式一条命令补）↔ 引擎接线，目标机零 TeX Live
- [ ] 引擎身份模拟（`\pdftexversion` 等）
- [ ] **MCP server / AI 工具链**（stdio JSON-RPC，复用 `Typesetter` 库接口 + `MemVfs`）：
  ① tex→PDF（已可起步）② 宏展开/诊断（随错误模型收尾）③ 会话式增量编译（依赖 M5）；
  安全基线 = RFC-3 副作用隔离。验收：`tex→PDF` 经 stdio 端到端可调用。

---

## 3. 北极星：Markdown 级编辑体验（2026-09-06 立项）

**产品总纲**：编辑 LaTeX 像 Markdown 一样简单和高性能。四个用户可感知指标：

| 指标 | Markdown 基线 | NTex 现状（更新至 2026-09-19） | 差距清单（条目引用） |
|---|---|---|---|
| ① 什么文档都能排 | 任意文本即开即排 | plain TeX 子集；`\documentclass` 未通 | LaTeX 战役（§0.1）；`\halign` 精化 / `\insert` / 数学矩阵（§5 P1） |
| ② 打开快（冷启动） | ~0ms | plain 毫秒级；latex.ltx 未加载，`.fmt` v1 不含字体表 | M7 `.fmt` v2（§2 M7）；字体按需备料 + 项目级缓存 |
| ③ 改字快（增量） | 全文重排无感（<16ms） | 改正文 4.3x / 改宏体 1.1x（目标 100x）；预览端仍是 250ms 防抖全量重排 | M5 阶段六全部（§2 M5）；预览三端接线（§2 M8）；M6 并行 |
| ④ 出错不糊 + 零安装 | 语法错误不挡渲染；零环境依赖 | 单点错误终止全文档；目标机需 TeX Live 备料 | M1-13 错误恢复（§5 P0）；SyncTeX（§2 M8）；panic 审计；宏包管理（§2 M9） |

**关键路径**：

```
正确性闭环（指标①④）：LaTeX 战役 + M1-13 错误恢复
  → 增量可见（指标③）：预览三端接 IncrementalTypesetter + 随机编辑 fuzz 进 CI
  → 增量冲高 + 冷启动（指标②③）：M5 阶段六 → 100x；M7 .fmt v2 → <200ms
  → 放大 + 生态（指标②④）：M6 并行；M9（UTF-8/字体/宏包管理/MCP）
```

**验收画像**（四指标齐备即达成）：`\documentclass{article}` 文档打开 <200ms；
改 1 字重排 <50ms 且预览无感刷新；表格/脚注/数学/主流宏包文档全链可排；
任意错误不中断渲染且定位到源码行、目标机零 TeX Live。

---

## 4. 性能 backlog（2026-08-22 评审）

> 不引入新功能，属既定里程碑补课。

**P0 —— ✅ 全部完成**

- 工作区 `[profile.release]`：`lto = "thin"` / `codegen-units = 1` / `panic = "abort"`
  （已确认全库无 `catch_unwind`、无 `unsafe`）—— `80022b4`
- 字节码执行器走 u64 原始字（RFC-4 设计落地）：`Bytecode.code: Arc<[u64]>`，
  `fetch()` 按 `word >> 60` 分发，热路径零解包 —— `80022b4`

**P1 —— 部分完成**

- ✅ 热路径消分配（2026-08-23 / 09-03）：`process_token` 按引用 match（SlotAction 两阶段）、
  `InputFrame::MacroArg` 复用实参 Arc、单 token 回推内联槽（`InputFrame::One`）。
- ✅ 火焰图附带修复：`error_context` 行定位改 `line_starts` 二分（原 O(pos) 全文扫描 +
  整行 UTF-8 转换，长样张 O(n²)，是吞吐被压到 3.1 万 token/s 的主因）；诊断开关进程级
  缓存；看门狗检查由每步改每 64 步。**展开吞吐 +61%，469ms / 42.6 万调用/s 入库。**
- [ ] `call_macro` 实参 `Vec<TokenArray>` → SmallVec（需新依赖，预估收益 <5%）
- [ ] **panic 审计 + fuzz**：生产路径 unwrap/expect（input.rs 10 / ntex-format 15 /
  ntex-io 6 等）改 `Error`，与 M1-13 联动。
  fuzz 目标已建（`ntex-layout/tests/fuzz.rs`，quick 2k 轮入 CI、深 5k 轮 `--ignored`），
  **已修两处违约**（`\the\meaning`/`\the\jobname` 自递归栈溢出；垂直模式行内数学后 `\par`
  空列表 panic），深 fuzz 2 万轮零 panic。

**P2 —— 工程化闭环**

- ✅ CI 门禁（`.github/workflows/ci.yml`）：门禁 job（fmt + clippy `-D warnings` +
  `cargo test --workspace` 含 fuzz quick + stub 冒烟）+ 观测 job（`continue-on-error`，
  真引擎 TRIP/ETRIP/diff/bench + 深 fuzz）；全绿后去掉 `continue-on-error` 转硬门禁。
- [ ] `make bench` 补 `--release`（现为 debug + stub，数字无参考价值）
- [ ] nightly perf job：release 跑真驱动基准，对照入库基线，吞吐降 >15% 即失败
- [ ] token 级微基准引入 divan

**测试纪律提醒**：`[profile.release]` 开 `panic = "abort"` + `lto`，
**`cargo test --release` 会失败** —— 测试一律用默认 dev profile（CI 亦如此）。

---

## 5. 待办汇总

> 全仓唯一待办清单。技术债明细（文件:行）见
> [docs/KNOWN-SIMPLIFICATIONS.md](docs/KNOWN-SIMPLIFICATIONS.md)。

### P0 —— 战役关键路径

- [ ] **latex.ltx 88.4% 主墙根因**（pos=685828）；输入栈泄漏与 Call/Ret 均已证伪
- [ ] **NFSS 字号墙**：`\ifdim` size range 解析 / `try@simples`（`Missing number … \@M`）
- [ ] `preload.ltx` l.47 `\DeclarePreloadSizes` → `! \font 后缺少字体名`（当前致命终止点，
      `\font` 原语在 `\small@sizes` 展开体上的扫描偏差）
- [ ] `\SetMathAlphabet\mathsf/\mathit{bold}`（fontmath l.73/74）→
      ``Command `' not defined as a math alphabet`` ×2
- [ ] **M1-13 错误恢复通用机制**：`back_input` / 插入恢复 token / `\errhelp` ——
      影响**所有报错原语**的"报错后继续"路径；TRIP 卡点根因；契约级（任意畸形输入不 panic）
- [ ] **TRIP / ETRIP semantic diff 归零**（−5619/+1412 与 −3051/+2352）+ 错误块抽查
- [ ] `\documentclass` / article.cls → 结构宏 → 真 LaTeX 版面 PDF

### P1 —— LaTeX 前置能力（撞上即做）

- [ ] `\halign`/`\valign` 精化剩余刀（刀 0 对拍仪器 / 刀 2 preamble 宏展开通路 + S3–S7；
      刀 1/3 已落：`\everycr` 两点注入、align_peek 入口复位、to/spread 摊派）
- [ ] `\insert` **体排版化**（脚注仍不可用；分配面已落：`\newinsert` + 三联寄存器）
- [ ] 数学矩阵（`\matrix`/`\eqalign`）—— 勘察结论：无独立引擎战役，
      本体是宏层 + `\halign` 地基，引擎增量仅 `\vcenter` 数学包装验证 +
      `Improper \halign inside $$'s` 检查 + Align 产物盒交 `MathAtom::Box`
- [ ] `\vsplit` marks 拆分（`\splitfirstmarks` 等现返回空）
- [ ] `\output` 例程消费判定改显式 `\shipout\box255`（现为 count 启发式）
- [ ] `\scriptfont` 接真实字体
- [ ] `\catcode` >255 赋值扩展（Unicode catcode 表，A5 全量）+ `\catcode"XXXX=13` 预读时序粘连
- [ ] `.fmt` 快照 misc 数组扩容的版本兼容（MISC_INTS 65→66→67 已两次扩容）

### P1 —— 工程

- [ ] panic 审计（见 §4 P1）
- [ ] 基准进 CI（`make bench --release`、nightly perf job、divan 微基准）
- [ ] M5 阶段六（7 项，见 §2 M5）
- [ ] 预览三端增量接线 + 随机编辑 fuzz 进 CI
- [ ] SyncTeX 源码映射

### P2 —— 未来里程碑

- [ ] M6 并行（等 M5 副作用边界）／ M7 `.fmt` v2 ／ M9 生态清单（见 §2 M9）

---

## 6. 关键决策与风险

### 6.1 已拍板

1. **Token = 8B tagged union**；源码位置走独立 side-table，不进 token。
2. **控制序列 token 只含 csid**，定义一律查 eqtb 槽
   （`Undefined / Macro{version,def} / Primitive / Register / RegisterIndex / Alias`）。
3. **宏体 = 连续不可变 TokenArray + 字节码双表示**；InternTable 线性化，csid = u32 下标，
   `.fmt` mmap 后零字符串查找。
4. **VM 保持纯 token 级**：排版事件经 `TokenSink` 单向流出，VM 不依赖布局 crate。
5. **副作用隔离**：`\write` 延迟到 shipout 边界提交，`\write18` 拒绝，读写流走 `ntex-io` VFS。
6. **输出端两段式**：临时 Helvetica 切片先行已下线，`ntex-pdf` 已改造为正式 DVI → PDF 后端。
7. **渲染后端选型 vello**（2026-09-05，原 Skia）：纯 Rust/wgpu 栈、WASM 同构、免 C++ 绑定。
8. **L1 优先于 L2 兼容**：先语义/折行一致（M4），字节级一致推迟到 M8。
9. **字节码成为默认执行路径**，解释器退为调试工具，双轨等价测试保留至 M7。
10. **CJK 口径**：`\char` 上界按当前字体判定；`\utfinputmode` 默认关（保 TRIP/ETRIP 字节口径）；
    `\cjkbreakmode` 默认关（禁用断点才改折行结果）。
11. **TRIP/ETRIP 逐字节口径降级**（2026-09-03）：验收改为 semantic diff 归零 + 错误块抽查，
    逐字节对齐记 M8 L2 观测项。

### 6.2 待拍板

1. **RFC-1 开放问题 Q2~Q4**：保留位用途 / 内部标记拆分类 / InternTable CoW 粒度。
2. **字节码 vs JIT**：JIT 作 M9 之后的可选加速，不进主线。
3. **RFC-2 是否成文**：不可变状态模型已由 M5 落地，独立 RFC 文档是否补。
4. **CJK 捷径的兼容边界**：跳过 HarfBuzz 的判定条件须保守（M9 单独评审）。
5. **ctex 兼容范围**：先 xeCJK 最小子集，不一步到位全量。
6. **数学输出**：先保 DVI/PDF 逐位一致，MathML 为可选（不阻塞主线）。
7. **MCP 工具形态优先级**：① 排版渲染 → ② 宏展开/诊断 → ③ 会话式增量编译。
8. **资产内置边界**（2026-09-19 讨论，**建议**待拍板——勿全量内置 TeX Live 体量宏包：
   TL2026 scheme-full 无文档 4312MB，现内置 823 条 ≈11.7MB）。建议分三层：
   **T0 引导层**静态内置（预算 ≤50MB，判据「没有它就排不出任何文档」）；
   **T1 精选层**按需落本地缓存（判据「引擎语义面已验证 **且** 已进宏包 CI 回归集」，**内置名单 ≡ 回归名单**）；
   **T2 全量可用层**从不内置（= M9 宏包管理 + M7 项目级 fmt）。必配项：缺包即报 + 一条命令补（MiKTeX 式）。
   离线段仅在离线/内网场景需要，且应为 opt-in 的 medium 级镜像包，非全量。
9. **按需补包的取料源边界**（同日讨论，**建议**待拍板——CTAN 只作回落源，不作接口）：
   对外契约 = 版本化仓库 + lock（包名 + revision + sha512），不等于直连 CTAN。
   **解析层只认 tlpdb**（CTAN 的 `FILES.byname` 无校验和 / 无依赖图 / 无版本号，不能作解析依据）。
   **取料层四源可插拔**：① 本地已有 TeX Live 树（零下载，搜索链已含）→ ② tlnet 镜像（SHA-512 + GPG 签名 +
   revision）→ ③ CTAN `install/**.tds.zip`（TDS 单包；回落场景：tlnet 不可达 / TL 未收录 / 需旧版本）→
   ④ 离线归档包（内网 opt-in）。**禁止**在 `\input`/`\usepackage` 失败时隐式自动联网
   （违反可复现性、安全与证据口径三条）。多镜像 + 离线回落是既有事实要求——本仓库已发生过
   「CTAN 主站与清华镜像被网络策略拦截」（见 assets/tex-minimal/README.md）。
   **落地进度（2026-09-19）**：① 与 ② 已实现（② 的 GPG 验签**未做**，只做 SHA-512 容器校验；
   且 `HostContainerIo` 把 HTTP/TLS/xz 交给宿主 `curl`/`tar`）→ 见 KNOWN-SIMPLIFICATIONS §9；
   ③ ④ 仍为显式未实现插口。

### 6.3 风险与关卡

| 风险 | 关卡（Gate） | 触发时动作 |
|---|---|---|
| TRIP 长期不绿 | M1 结束必须全绿 | 冻结新增功能，只修语义 |
| 字节码与解释器不一致 | M2 双轨 diff 测试 | 禁用字节码路径，回溯 IR 设计 |
| 增量缓存出错（副作用漏追踪） | M5 增量-vs-全量模糊 diff 常驻 CI | 修复依赖登记，不回退功能 |
| 并行破坏确定性 | M6 输出逐位 diff | 收窄并行边界（只并布局/整形） |
| ctex 兼容工作量失控 | M9 按 xeCJK 子集分阶段验收 | 缩减目标宏包范围，先保 ctex 常用 |
| 冷编基准不达标 | M7 验收 | 优先级：深 `.fmt` > CJK 捷径 > 多核 |
| 仪器失真误导定位 | 改诊断原语后必跑 `make instrument-check` | 见 docs/tooling-trust.md 九次事故登记 |

---

## 7. 溯源

| 归档文档（docs/archive/） | 内容 | 冻结日期 |
|---|---|---|
| `plan-full-2026-09-19.md` | 本文件重构前的全文（含各里程碑逐项实施步骤与战报） | 2026-09-19 |
| `ETRIP-primitives.md` | ETRIP 原语逐条状态（A/B/C 组 + M4 基线） | 2026-09-18 |
| `latex-feasibility.md` | LaTeX 战役活文档全文（A0 阻塞点 + A1.* 逐刀 ~30 节） | 2026-09-19 |
| `latex-feasibility-full-2026-09-11.md` | 更早的 §8–§37 逐刀记录全文 | 2026-09-11 |
| `latex-feasibility-relay28b.md` | 第二十八刀接力简报（IPN×256 根因） | 2026-09-06 |
| `expl3-real-scoreboard.md` | expl3 真实跑分与逐刀（第一~二十五刀） | 2026-09-19 |
| `expl3-lvt-scoreboard.md` | l3kernel 官方套件跑分与载入终点复测 | 2026-09-19 |
| `expl3-real-workload.md` | expl3 真实工作量评估（187 例分母辨析） | 2026-09-11 |
| `plain-format-survey.md` | 格式预载可行性勘察（G0–G3 记录） | 2026-09-11 |
| `output-routine-survey.md` | 输出例程勘察（刀 1–5 记录） | 2026-09-11 |
| `halign-survey.md` | `\halign`/数学矩阵战役前置勘察（含 S1–S7 刀序） | 2026-09-07 |
| `trip-missing-primitives.md` | TRIP 缺原语清单（124 → ~10） | 2026-08-28 |
| `primitive-correctness-checktable.md` | 原语语义正确性四层防线盘点 | 2026-09-05 |
| `corpus-probe-2026-09-19.md` | corpus 探针实录（2/8 PASS 原始记录） | 2026-09-19 |
| `REVIEW-2026-08-23.md` | 代码审查（A 正确性 / B 性能 / C 工程 / D 简化点 / F 路线图） | 2026-09-19 |
| `schema-dump.txt` | 早期方案摘要草稿（内容已并入 idea.md / README） | 2026-09-19 |
