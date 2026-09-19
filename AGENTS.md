# AGENTS.md — NTex 仓库目录结构

> 本文件**只描述目录结构与各部分职责**。
> 进度与待办看 [plan.md](plan.md)（唯一进度源）；构建命令与开发约定看 [README.md](README.md)。

---

## 1. 顶层布局

```
NTex/
├── crates/            Rust workspace（引擎全部实现，见 §2）
├── assets/            发行资产：latex.fmt / TFM / tex-minimal 最小 TeX 宏文件集（见 assets/tex-minimal/README.md）
├── fixtures/          测试 fixtures：TRIP/ETRIP、差分样例、l3kernel 官方套件、corpus 探针用例
├── samples/           端到端样张（demo / demo-multi / demo-cjk / resume-plain / latex-sample2e-slim 等）
├── probes/            最小复现探针（fp-load 载入探针、perf 基准脚本）
├── scripts/           辅助脚本：定位仪器（abcheck / logtrace / trace-view / blocker-track）、lvt 跑分、语料探针、字体子集化
├── tests/             跨 crate 测试（semantic 语义对照等）
├── docs/              长期参考文档（仅 3 份 + 归档，见 §4）
├── reference/         ground truth 参照物（tex.web、pdfTeX 期望输出等）
├── screenshots/       文档用截图
├── site/              项目站点静态资源
├── plan.md            唯一进度源
├── README.md          上手入口与开发约定
├── idea.md            架构构想（愿景，冻结）
├── RFC-*.md           架构决策（RFC-1 token / RFC-3 副作用 / RFC-4 字节码 / RFC-5 并行）
├── Makefile           质量门禁与各管路入口
└── Cargo.toml         workspace 定义 + release profile
```

---

## 2. crates/ — workspace 成员与职责

三层解耦：

```
TeX/LaTeX 源码 → 0.输入层(ntex-io VFS) → 1.TeX VM 求值(ntex-core) → 纯节点流
  → 2.布局引擎(ntex-layout：折行/断页/数学) → 盒子树 + \shipout → DVI(ntex-dvi)
  → 3.渲染后端(ntex-pdf / ntex-backend) → PDF / PNG / 屏幕
```

| Crate | 职责 |
|---|---|
| `ntex-core` | **TeX VM**：token（8B tagged union）/ catcode / InternTable / eqtb 版本化 / 展开引擎 / 字节码 IR·编译器·执行器 / 内部参数 / `TokenSink` 事件流；增量层 `src/incremental/`（段级重算 + 可回滚检查点 + 依赖追踪） |
| `ntex-layout` | **排版器**：主循环（模式状态机）/ 折行 Knuth-Plass / 断页 / 输出例程 / 数学（`typeset/` 目录）/ 断字。`typeset::looks_like_latex` 是 LaTeX 格式自动检测的**单一事实源**（`ntex-dvi` 与 `ntex-studio` 共用；JS 侧 `main.js` 是同口径副本） |
| `ntex-font` | 字体度量：TFM 解析 + OTF/TTF 度量通道（`FontMetrics` 注册表，PDF 写出的唯一度量来源）+ 缩放 |
| `ntex-dvi` | DVI 写出器（与真实 TeX 逐字节一致）+ CLI：`--fmt` 快路径、`--generate-fmt`、`\input` 搜索链 |
| `ntex-pdf` | DVI → PDF 正式后端（PDF 1.4 写出 + Type1/PFB 字体嵌入） |
| `ntex-format` | `.fmt` 序列化 / 反序列化（v1 内存快照；v2 mmap 零拷贝待做） |
| `ntex-io` | VFS 抽象 + LocalVfs / MemVfs（RFC-3 副作用隔离的载体） |
| `ntex-pkg` | **宏包管理**（M9 生态冲刺，plan.md §6.2 第 8/9 条）：`texlive.tlpdb` 解析（含 continuation 行状态机 / RIV 块计数 / `.ARCH` 展开）+ 文件反查索引 + `\usepackage`/`\documentclass`→包解析 + 依赖闭包 + `ntex.lock` 确定性契约（包名 + revision + sha512；身份字段校验、参考字段忽略）+ 内容寻址缓存布局 + 可插拔取料源链；`tlnet.rs`（② tlnet 镜像：URL 由 revision 钉死 + SHA-512 容器校验 + 按 `runfiles` 裁剪与 `RELOC/` 重定位）/ `vendor.rs`（闭包 → TDS 子树物化，四态逐字节比对，源缺失显式报错；`fetch`/`vendor` 分别为"取料"与"物化"两半）；CTAN / 离线归档两源仍为**显式未实现插口，不静默降级**）。**解析层只认 tlpdb**（CTAN `FILES.byname` 无校验和/依赖图/版本号，只作回落源）。CLI `ntex-pkg`：`index` / `provide` / `resolve` / `lock` / `check` / `local`（漂移退出码 3）/ `vendor` / `fetch` |
| `ntex-test-support` | 测试 / 差分 / 基准基础设施（`EngineDriver` 抽象） |
| `ntex-trip` | TRIP / ETRIP 一致性测试框架（`--test trip\|etrip\|both`，in-process ntex 驱动） |
| `ntex-diff` | 差分测试工具（参考引擎 vs 本引擎，diff DVI/log） |
| `ntex-bench` | 基准框架（含 `expand-throughput` / `expand-dual`） |
| `ntex-backend` | **渲染后端**：`Backend` trait + 软光栅 + vello 0.10 GPU（wgpu 29 无头纹理回读）+ PNG 导出；`build_scene(prims) -> Scene` 为无头回读与 GUI 表面渲染共用的公共 Scene 构建；**字形通道** `glyphs.rs`（OT1→Unicode→Latin Modern OpenType 轮廓，双通道：vello glyph run / 软光栅 nonzero + 4x 超采样填充；位置宽度仍按 TFM；缺字体逐字符回落方框） |
| `ntex-studio` | **实时预览工作台**（native）：eframe/egui-wgpu 0.35 + vello 表面渲染（离屏 Rgba8Unorm → blit 上屏）、TeX 高亮编辑器、250ms 防抖重排、缩放/平移/翻页、dpi / overlay / 真字形开关、LaTeX 模式自动切换与 Log 转录面板（`src/engine.rs` 承载 native 侧格式与资产装配）。**依赖硬约束：eframe 0.35 ↔ vello 0.10 恰共用 wgpu 29**（升 eframe 大版本前必验对齐） |
| `ntex-wasm` | **WASM 薄壳**：浏览器/Node 内编译 + 软光栅渲染。`compile_tex`（plain 子集 → DVI + 转录）、`compile_document`/`render_page`（页盒树常驻 + 按需渲染）、`set_tfm_source`（内嵌 48 个 CM TFM）、`set_glyph_font`（LM OTF 轮廓，进程级轮廓注册表）、`set_otf_font`（任意 OTF/TTF，一次同写**排版度量 + 渲染轮廓**两侧）、`set_utf8_input`、`set_bundle`/`set_latex_mode`（`NTEXBND1` 资产包：TeX 文件 / TFM / `.fmt` 快照；打包端在 `ntex-tauri`，契约见 `src/lib.rs`）。wasm32 分叉仅三处（`param.rs` 时间固定 / `expand::run` 看门狗门控 / `TfmLoader` 字节源）；`www/` 为工作台前端 |
| `ntex-tauri` | **Tauri 2 纯壳工作台**：桌面窗口 + 静态前端 `ui/`，排版与渲染全部在前端 WASM 内；Rust 侧仅两个非零 IPC——只读资产命令 `ntex_latex_bundle`（把 `assets/` 打成资产包）与下载落盘回调。`ui/pkg/` 为生成物不入库 |
| `ntex-mcp` | **MCP server**（M9 形态①）：stdio JSON-RPC 2.0，TeX/LaTeX 源码 → 内存 DVI → PDF（base64 返回）。**不引外部 SDK**——`json.rs`（RFC 8259 最小实现）/ `base64.rs`（RFC 4648）/ `server.rs`（协议与安全基线，见模块文档）/ `stdio.rs`（传输）。安全基线 = RFC-3 副作用隔离 |

**常用源文件布局**：

- `crates/ntex-core/src/expand/`：展开引擎——`builtins.rs`（原语注册表，单一事实源）、
  `primitive.rs` / `primitive_math.rs` / `primitive_box.rs` / `primitive_param.rs`（处理器）、
  `expr.rs`（`\numexpr`/`\dimexpr`/`\glueexpr`）、`cond.rs`、`macros.rs`、`scan.rs`、
  `align.rs`（对齐状态机）、`checkpoint.rs`、`save.rs`、`free.rs`、`io.rs`、`tests*.rs`；
- `crates/ntex-core/src/eqtb/`：`mod.rs`（槽模型）+ `primitive.rs`（`Primitive` 枚举）；
- `crates/ntex-core/src/incremental/`：段级增量（M5）；
- `crates/ntex-layout/src/typeset/`：`typesetter.rs`（主循环）/ `paragraph.rs` / `paging.rs` /
  `math.rs` / `sink.rs` / `plain_format.rs` / `incremental.rs`；
- `crates/ntex-layout/src/`：`page.rs`（断页）/ `linebreak.rs`（折行）/ `node.rs` / `font.rs`；
- `crates/ntex-pkg/src/`：`tlpdb.rs`（TLPDB 解析 + `by_basename`/`by_path` 反查索引 + TDS 优先级）/
  `resolve.rs`（`RequireKind` 候选扩展名链 + 包解析 + 依赖闭包 BFS）/
  `lock.rs`（`ntex.lock` 编解码 + 漂移 `diff`）/ `cache.rs`（内容寻址布局 + 名字安全 + sha512 形状校验）/
  `source.rs`（`PackageSource` trait + `SourceChain` 优先级 + ① 本地 TL 树源）/
  `testdata.rs`（共享 mini tlpdb fixture）/ `main.rs`（CLI）。

**新增原语的完整链路**（六步，缺一不可）：

1. `eqtb/primitive.rs` 加 `Primitive` 变体；
2. `expand/builtins.rs` 注册内建 cs；
3. sink / 查询接口（`sink.rs` 事件或查询方法）打通 VM → 排版器；
4. 对应 `primitive*.rs` 实现处理器语义；
5. 单元测试（对照 pdfTeX / tex.web 行为）；
6. 在 [plan.md](plan.md) 的待办与 [docs/KNOWN-SIMPLIFICATIONS.md](docs/KNOWN-SIMPLIFICATIONS.md) 登记状态。

---

## 3. 各目录内的 README

子目录的局部约定就地维护，不重复到顶层文档：

| 位置 | 内容 |
|---|---|
| `assets/tex-minimal/README.md` | 发行资产清单与搜索链约定 |
| `assets/tfm/README.wasm-fonts.md` | wasm 内嵌 TFM 来源 |
| `fixtures/README.md` | 对照 fixtures 状态表与抓取约定 |
| `fixtures/repro/README.md` | 复现用例 |
| `samples/README.md` | 样张与派生物 |
| `probes/fp-load/README.md`、`probes/perf/README.md` | 探针用法 |
| `crates/ntex-wasm/README.md` | wasm 三档路线与 JS 绑定契约 |
| `crates/ntex-tauri/ui/fonts/README.md`、`ui/pfb/README.md` | 前端字形与 Type1 资产名单 |
| `scripts/abcheck-examples/README.md` | 对拍脚本示例 |

---

## 4. docs/ — 长期参考文档

| 文件 | 定位 |
|---|---|
| `KNOWN-SIMPLIFICATIONS.md` | **技术债清单**（文件:行 + 状态）。新增任何"简化/no-op/暂不"必须当天登记；修复后标 ✅ + commit。改动前先查表 |
| `tooling-trust.md` | **定位基础设施与仪器可信度**：七件套用法（`instrument-check`/`abcheck`/JSONL trace/`blocker-track`/`logtrace`/`recovery-check`/`STACK_DUMP`）+ 九次仪器失真事故登记 + 判读纪律。**开工定位前先读**；改诊断原语后必跑 `make instrument-check` |
| `MATH-STATE-MACHINE.md` | 数学状态机语义规格（`$`/`$$` 分模式语义、组结束、嵌套）+ 修改守则 |
| `blocker-history.tsv` | 阻塞点看板数据（`make blocker-track` 读取） |
| `archive/` | **历史归档**（逐刀战报、勘察全文、跑分基线、一次性评审）。只读不改，需要考古时全文 grep。索引见 `archive/README.md` |

> **文档纪律**：进度只写 `plan.md`；一刀闭环后把逐刀细节移入 `docs/archive/`（保留原文件名），
> `plan.md` 只留结论 + 归档指针。新增长期参考文档须是"怎么做/是什么"，不得写"做到哪了"。
