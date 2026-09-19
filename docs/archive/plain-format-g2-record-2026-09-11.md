# 格式预载 G2(a) 实测记录（归档）（2026-09-11 归档）

> ⚠ **历史归档，不再更新。** 同批归档见 [plain-format-survey.md](plain-format-survey.md)（亦已归档）。
> 保留原因：含详细改动面/tex.web 裁决/单测清单，供查证具体某刀。

---

## 5.bis.g2 G2(a) 实测记录（2026-09-07，✅ 完成——启动 `\input plain` 内嵌接入，提交 510431b）

**改动面**：`crates/ntex-layout/resources/{plain.tex,hyphen.tex}`（逐字节 verbatim，
plain.tex 与 `fixtures/corpus/plain/plain.tex` `diff -q` 一致；hyphen.tex 头部自述
"NOT TO BE CHANGED IN ANY WAY"，合计 72,475B 内嵌）、`typeset/plain_format.rs`
（资源常量 + `EmbeddedFormatVfs` 兜底层）、`typesetter.rs`（`preload_plain`/
`embedded_vfs_installed` 两字段 + `set_preload_plain`/`plain_format`/
`use_embedded_format`/`run_plain_preload`）、`typeset/mod.rs`（模块声明）、
`tests.rs`（`mod tests_plain_format`）、`ntex-dvi/main.rs`（默认开 + `--no-plain`）。
**零 ntex-core 改动**（`\input`/`\patterns` 原语一行未碰）。

**裁决（含否决论证，架构评审口径）**：
- **接入层次 = ntex-layout 库层 + 二进制层默认开**。否决「纯 ntex-dvi main 层把
  plain.tex 前拼进用户文本」——wasm/mcp/backend 不受益，且 main 层拿不到 Expander，
  `\input hyphen` 的 VFS 注入仍得在库层做；否决「`Typesetter::new/with_tfm` 构造时
  无条件预载」——这四个构造器被 TRIP/latex probe/corpus math 等 INITEX 语义调用方
  共用（latex.ltx L99 靠「纯 initex」catcode 判别），无条件预载砸穿不相干线；
  否决「`typeset_dvi` 入口无条件预载」同理。落点：**库层 opt-in 显式 API** +
  **ntex-dvi 默认开**（plain 文档直通，`--no-plain` 退路）——「自动」与「不砸
  不相干线」的交点。
- **资源来源 = 内嵌字符串**（survey §3.2 (a)+(c) 合流）。否决「读文件 + G1 搜索
  路径」：G1 只解决「找得到」，目标机没有 TinyTeX 依旧跑不了——分发问题没解决
  （§3.2 共同前置明言）；wasm 先例（ntex-wasm 已内嵌 6 个 CM TFM）+ 单二进制
  离线可用是既定方向；72KB 对 wasm 可承受。否决「跳过 patterns」：hyphen.tex 仅
  27,860B，跳过反而制造「plain 预载了但断词表缺失」的新静默偏差（`\patterns`/
  `\hyphenation` 原语已在，ntex-core builtins.rs:127/245）。
- **内嵌层是兜底不是抢先**：`EmbeddedFormatVfs` 读侧先问 inner（本地/宿主/搜索
  路径命中即返），全落空才答 `plain.tex`/`hyphen.tex`——本地文件优先，不改变既有
  搜索语义。且与预载开关**正交**：`--no-plain` 下源内 `\input plain` 仍走内嵌。

**真 TeX 对拍**（TinyTeX pdftex 3.141592653，`NTEX_TFM_DIR=/home/ubuntu/.ntex-fonts`，
`/tmp/g2work/`，cwd 无任何 .tex 文件）：

| 探针 | 真 TeX | NTex（本刀后） |
|---|---|---|
| `\input plain` + `Hello world.` + `\bye`（G1 文件路 `--input-path`） | 224B / 1 页 / 0 错 | 203B / 1 页 ✅（字节数差为 set_char 编码档位，survey §4.3 同源） |
| 同上，**纯内嵌** + 源内 `\input plain` | — | 203B，**md5 73a2ca13…** ✅ |
| 同上，纯内嵌 + 启动预载（源内不写 `\input plain`） | — | 203B，md5 同 ✅ |
| 同上，`--no-plain` + 源内 `\input plain`（内嵌兜底） | — | 203B，md5 同 ✅ |
| 纯 initex 错误数（pdftex -ini 同源跑全文） | **0** | 29/遍（见下） |

四路 DVI **逐位一致**＝「内嵌通路与文件通路等价」的最强判据；预载后用户文档不再
需要写 `\input plain`。

**G0 首轮错误清单逐条复测**（HEAD 3ef1f67 + 本刀；对照 4ec6a84 记录）：

| G0 靶子 | 复测 | 判定 |
|---|---|---|
| `\footins`/`\topins` 落 insert255 | `\footins=\insert254`、`\topins=\insert253` | **已消**——输出例程刀 4 `\newinsert` 分配器落地（3ef1f67），分配号与真 plain 格式一致（254/253） |
| `\newif`/`\newbox`/`\newinsert` 区（l.598/599/1023/1121/1149/1177） | 24×`! Use of macro doesn't match its definition.` | **仍在**——量恰 = plain.tex 全部 8 处 `\newif` 调用点 ×3，G6 普查靶子，本刀不修 |
| l.1237 | 2×`! Missing number, treated as zero.`（`<to be read again> \z@` / `\tenrm`） | **已消**——`scan_int` 补 dimendef'd cs 数字上下文臂（04d2023，plain 预载全通）；剩余 `\newif` 链与字体备料见 G6 |
| `\bye` Incompatible list | 未复现（本轮未观测到该签名） | 待 G6 构造探针复验 |
| （G0 未照见，本刀首见）shipout 路径 | 3×`Missing number …\z@` 无行上下文，仅在有页面冲出时出现 | **新增可见**——预载前 `\plainoutput`/`\makefootline`/`\advancepageno` 不存在、不可达；钳制探针：`\def\plainoutput{\shipout\vbox{\makefootline}}` 与 `…\advancepageno` 单独即可少 2 处 → 疑 `\pagebody`/`\makeheadline` 一带，G6 靶子 |

**corpus plain 3 样例**（ntex-dvi 缺省旗标 + dvipdfmx 全链）：

| 文件 | G1 期（survey §4.2） | 本刀后 | 判定 |
|---|---|---|---|
| `list.tex` | FAIL（`\multiply` 目标） | 121B / 1 页 / 53 字体，PDF OK | **PASS**（`\newcount` 分配机制，不回退） |
| `plain.tex` | FAIL（找不到 hyphen） | 172B / 1 页 / 51 字体，PDF OK | **PASS**（`\input hyphen` 走内嵌） |
| `letterformat.tex` | FAIL（`\advance` 目标） | 121B / 1 页 / 53 字体，PDF OK | **PASS***——旧死点 `\advance\vsize by-\voffset` 由 G3 刀解开（3af87dd，`\advance/\multiply/\divide` 目标集合扩 page 参数）；带 * 是因为它与 list.tex 同为宏/格式文件（无正文、无 `\bye`），真 TeX 是 **0 页**，NTex 预载后经 `\plainoutput` 收尾冲页出一页**空页**（见下「发现未修」#1） |

**demo1-fixed 全链**（验收口径：md5 变化须判定语义）：
- md5 `5e2acc8a…`（1697B / 15 字体）→ `703a356e…`（1722B / 53 字体）。
- pymupdf 抽文本 diff：**唯一差异一行 = `1`**（x≈页心 303.5pt、y≈622.7pt，页高
  841.9pt → 页体底下缘），其余 657 字符逐行一致；两侧均 1 页、24 个 fnt_def。
- **判定：预载正确覆盖手写缺漏，非回归**。demo1-fixed 手工自举块补了字体/参数/
  muskip/mathcode/宏，但没补 `\output={\plainoutput}`→`\makefootline`→`\folio`
  输出例程层；预载把这一层带上，页脚页码 1 正是真 plain TeX 对 `\bye` 文档的行为
  （survey §5.1 真 TeX 同样在页脚出页码）。五自举块与 plain 定义的重复定义
  （`\bye`/`\centerline`/`\medskip`/`\item`/`\it`/`\tt`/`\LaTeX` + 14 个 `\font`）
  全部按「后写覆盖」干净收敛，无冲突报错。

**发现未修（本刀只测量，留独立刀）**：
1. **预载后空页**：`\input plain` + `\end`（无正文）真 TeX **0 页**，NTex 出
   **1 页空页**（172B）。`\plainoutput` 在输入结束冲页时对空页列表照常 shipout。
   预载前该路径不可达（无 plain 就没人设 `\output`），属输出例程线的收尾冲页判据。
2. **G4 预期兑现（非本刀，登记在案）**：`\lccode`/`\uccode` 初表仍全 0
   （expander 侧 INITEX 偏差），plain.tex 不设这两张表 → 预载后 `\lowercase`/
   `\uppercase` 仍会静默错。且本刀实测到它的**第二个受害者**：断词。见 #3/#4。
3. **`PatternTrie::parse` 忽略 `.` 词界标志**（hyphen.rs `parse_pattern` 注释
   「暂不参与 trie」的既有简化）：真实 hyphen.tex 的词首/词尾受限模式
   （`.ach4`、`5hand.`）被当成任意位置模式。对 "hyphenation" 复算 trie 得断点
   `[1..10]` 全集（真 TeX 是 `[2,6]`）。toy 表单测
   （tests.rs `patterns_hyphenates_word_across_lines`）不触发，真实表一上就显形。
4. **TFM 字体的字母 run 被 kern 节点切碎**：`hyphenate_paragraph` 的 run 收集器
   遇非 Char 节点即断，cmr10 的字间 kern 把 "hyphenation" 切成单字母段 → 无断词。
   cmtt10（无 kern）整段 intact 也未见 discretionary（叠加 #2 的 lccode 疑点：
   断词查表前本应经 `\lccode` 归一小写）。→ **端到端断词暂不落地**；本刀单测只
   断言「`\patterns` 事件入 builder（4448 条）」与「内嵌表可解析」，不断言
   discretionary。
5. **预载成本（survey §8.3 的未测量项，实测补账）**：debug 构建 ~0.23s/次
   （1241 行宏展开 + 51 个 TFM 加载）；同规模无 plain 文档 <0.01s。release 构建
   未测。这是 (a) 路的结构性成本——(b) `.fmt` 快照路（§3.2）是摊销它的正解，
   但受 §3.2 (b) 覆盖面缺口约束。
6. **下游接线待办**：ntex-backend / ntex-wasm / ntex-mcp 尚未接
   `set_preload_plain`（一行 + 旗标面）；wasm 侧无需额外资源（`include_str!`
   随库编译进 wasm 二进制）。

**靶向单测 ×7**（`ntex-layout/src/typeset/tests_plain_format.rs`）：缺省不预载
（`\ifx\newcount\undefined` → UNDEF）/ 预载先行且 `\newcount` 分配可用
（`\message{N=\the\foo}` → `N=42`）/ builder 形 / 源内 `\input plain` 纯内嵌可解 /
本地 plain.tex 优先于内嵌 / `\patterns` 事件入 builder（4448 条）/ 内嵌表可解析。
`cargo test -p ntex-layout -p ntex-io -p ntex-dvi` 全绿（196 + 8 + …），
`cargo test -p ntex-trip` 无回归；TRIP ntex 驱动 l.358 数学组生命周期失败为 HEAD
既有基线（trip-round4-findings），本刀代码在其路径上零触碰（`preload_plain`
缺省 false，`run_plain_preload` 直接返回）。

**账实备注（共享树事故，已核实）**：G2(a) 提交 510431b 除本刀文件外带入了同树并行 G3 刀在
ntex-core 的已暂存在途改动（free.rs/primitive_assign.rs/scan.rs/tests.rs，+226/−25）
——`git add` 只加本刀七路径，对方已暂存文件被一并提交。内容经本刀全量测试验证
可用，故保留不拆；letterformat 旧死点已由 G3 刀正式落地解开（3af87dd，上文已归据）。
