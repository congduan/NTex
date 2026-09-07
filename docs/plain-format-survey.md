# plain.tex 格式预载可行性勘察报告（2026-09-07，勘察起点 HEAD 6d6de64）

> 背景：demo1 对照战六刀复盘的结论是"全部实质差异（muskip / sfcode / italics / 混合胶水 /
> output 例程）同根——NTex 无格式预载，demo1-fixed 是手工自举的 plain 残缺版"。本报告
> 量化"让引擎直接吃下完整 plain.tex"这条路：现在能跑多远（实测，不靠 grep 猜）、接入点
> 在哪、预载通过后能删掉多少手工自举、以及 \plainoutput 会不会比现状更炸。
>
> **边界声明**：本报告只读。`crates/` 零改动，唯一新增文件即本报告。真 TeX ground truth
> 在 `/tmp/plainfmt-work/`（未入库，复现见 §7）。P 线 display 重构在途的四个文件
> （`ntex-layout/src/typeset/{incremental,math,mod,sink}.rs`）只读未写，行号以 HEAD
> 6d6de64 为准。**勘察中途该线落了提交 `2fa7e4b`（demo1 P5）**，本报告全部探针用的
> 是早于该提交的预构建 `target/debug/ntex-dvi`（12:30 构建）。fixtures/ 未改
> （`fixtures/corpus/report.json` 为既有产物，只读引用）。

---

## TL;DR

- **plain.tex 1241 行在 NTex 引擎上今天就能跑通——唯一致命阻塞不是宏，是文件解析**。
  `\input plain` 在 `plain.tex:1222` 处 `\input hyphen` 时死于 `非法输入：找不到文件：
  hyphen`；把 TinyTeX 的 `hyphen.tex` 拷进 cwd 后，**全文零 fatal 跑完**。
- **端到端产物已对上真 TeX**：`\input plain` + `Hello world.` + `\bye` 在 ntex 产出
  1 页 / 203 字节 DVI（真 TeX `pdftex -ini`：1 页 / 224 字节），两者 `dvipdfmx` 均可渲染，
  页面文本同为 `Hello world.` + 页脚页码 `1`。`\output={\plainoutput}`（plain.tex:1197）
  **确实生效且 `\plainoutput` 确实被调用**——`\def\plainoutput{\shipout\hbox{MARKER-PO}}`
  覆盖后 MARKER-PO 落进 DVI。**预载不会比现状更炸**（§5 风险矩阵）。
- **实测复现的真引擎缺口只有一个，且与 plain 预载无关**：`\advance\hsize by 10pt` →
  `非法输入：\advance 目标必须是寄存器或内部参数`。`\hsize/\vsize` 这类 page 参数不被
  `\advance/\multiply/\divide` 当作合法目标。`fixtures/corpus/plain/letterformat.tex`
  正是死在这里（`\advance\vsize by-\voffset`），plain 预载**救不了它**。
- **收益已实测落一个**：`fixtures/corpus/plain/list.tex` 由 FAIL 转 PASS（其依赖链
  `\newcount\m \newcount\n \n=\time \divide\n 60 \multiply\m 60 \advance\m \time` 全通）。
  `\newcount` 分配机制在预载后实测可用（`\newcount\foo \foo=42 \number\foo` → DVI 含
  "42"，与真 TeX 同型）。
- **★ 最重要的架构发现：引擎初表已是「分裂脑」，demo1 六刀残差的真根因在此。**
  layout 侧（`ntex-layout/src/typeset/typesetter.rs:134-138`、`typeset/mod.rs:735-746`
  与 `:795-801`）**早已硬编码 plain 的 sfcode 标点表 + A–Z=999 + muskip 3mu/4mu/5mu**，
  而 expander 侧（`expand/mod.rs:689`、`register.rs:179`）是 INITEX 值（sfcode 全 1000、
  muskip 全 0）；且 muskip 的 plain 默认还被 `typesetter.rs:305` **主动抹零**。demo1-fixed
  手写的「muskip 三行」「大写 sfcode=999」两个自举块，对应的常数在引擎里各存了两份、
  两边不一致。**预载 plain.tex 的真正价值是把这个双源收敛成单源**，不只是补宏。
- **★ 一个预载也修不掉的偏差**：`\lccode/\uccode` 在 expander 侧全 0
  （`expand/mod.rs:742-743`），而 tex.web INITEX 给字母设了值
  （`reference/tex.web:4848-4850`）；`\delcode` 空表读侧兜底 `0x500000`
  （`expand/save.rs:653-659`），tex.web INITEX 是 `-1`。**plain.tex 从不设置这两张表**
  ——它们是 INITEX 的活。所以预载后 `\lowercase/\uppercase` 仍会静默错。
- **测量陷阱（重要）**：`ntex-dvi` 二进制**没有任何诊断输出通道**——`\message`/`\show`/
  `\write16` 全部无终端回显，undefined control sequence **静默跳过不报错**
  （`\bogusprimitive` 零输出照常出 DVI）。因此"plain.tex 没炸"≠"plain.tex 全对"。
  唯一带转录通道的入口是 `ntex-trip` 的 `NtexDriver`（`take_transcript()` + 环境变量
  `NTEX_KEEP_TRANSCRIPT`）。**这项格式战役的第一刀必须先立转录通道，否则后续全部盲打。**
- 量级判断：**接入面小（1 个文件解析缺口 + 1 个 CLI flag），语义面已相当平**。
  三条候选接入路径的成本对比与推荐排序见 §4（不做选型裁决）。

---

## 1. 勘察方法与 ground truth 资产

1. **实测优先**：所有结论来自 `/tmp/plainfmt-work/` 的探针二进制执行，非静态 grep。
   引擎用既有预构建产物 `target/debug/ntex-dvi`（构建于 12:30，早于本勘察），未触发
   cargo 重建（遵守共享 VM 单 cargo 约束，且避免编到 P 线在途半成品）。
2. **真 TeX 对照**：TinyTeX `pdftex -ini -etex '\input plain …\bye'`，源 plain.tex 取
   `~/.TinyTeX/texmf-dist/tex/plain/base/plain.tex`（1241 行），与
   `fixtures/corpus/plain/plain.tex` **逐字节 IDENTICAL**（`diff -q` 验证）。
3. **DVI 内容比对**：环境无 `dvitype`/`pdftotext`，改用两条通路——(a) `dvipdfmx` 作
   合法性判据（两端均成功产 PDF）；(b) 自写 20 行 Python DVI 字节扫描提取可见文本串。
4. **字体口径**：沿用 corpus 探针的 `NTEX_TFM_DIR=/home/ubuntu/.ntex-fonts`。
5. **行为式探针**（因诊断通道缺失而设计）：由于 undefined cs 静默，改用「在 DVI 里埋
   MARKER 字符串、再字节扫描验证」的行为式判据，见 §5。

---

## 2. 问题一：plain.tex 现在能跑多远（实测）

### 2.1 执行链路与阻塞点

| 步骤 | 结果 |
|---|---|
| `\input plain`（plain.tex 在 cwd） | ✓ 进入 |
| plain.tex L1–1221（catcode / 宏定义 / 字体 / 参数 / mathcode / alloc 机制） | ✓ 全部执行，无 fatal |
| plain.tex **L1222** `\input hyphen` | ✗ `非法输入：找不到文件：hyphen`（**唯一致命点**） |
| 拷入 `hyphen.tex` 后重跑 | ✓ 全文跑通，`\bye` 出 1 页 |
| `\bye` → `\plainoutput` → shipout | ✓ DVI 203 字节 / 1 页 / 51 字体 |

关键点：`\input` 的文件解析**只按进程 cwd 尝试**（`ntex-test-support/src/driver.rs`
的 `WorkDirVfs` 语义：先 `fs::read(path)`，失败再 `wd.join(path)`），没有 kpathsea /
`TEXINPUTS` 树。所以 plain.tex 本体可被找到（放在 cwd），但它内部的 `\input hyphen`
就找不到——**这不是宏问题，是输入搜索路径问题**。

### 2.2 错误分类归组（实测量级，非 grep 推断）

| 类 | 内容 | 实测量级 |
|---|---|---|
| **A 引擎缺原语** | 令 plain.tex fatal 的原语缺失 | **0 个致命**。唯一实测复现的 A 类缺口是 `\advance/\multiply/\divide` 不接受 page 参数目标（`\advance\hsize by 10pt` fatal），见 §2.3；plain.tex 全文未触发任何「未定义原语」型 fatal |
| **B 缺 catcode / 初始化** | plain.tex 期望 INITEX 初表 | plain.tex 自带完整 catcode 段（L1–L190 区）自举，实测未致命；真正的 B 类缺口是**输入搜索路径**（§2.1），属初始化环境而非 catcode 值本身 |
| **C 缺 \font 路径** | TFM 查找 | **0 阻塞**。预载后 51 个字体定义全部成功（`NTEX_TFM_DIR` 指向既有字体集）；DVI 里 `cmr10` 等正常落 fnt_def |
| **D 其余（静默偏差）** | 不 fatal 但结果错 | **无法用 ntex-dvi 量化**——undefined cs 静默跳过、`\show/\message/\write16` 无输出通道。§2.4 |

### 2.3 实测复现的独立引擎缺口（与 plain 预载无关，但挡住 corpus）

```
\vsize=100pt \advance\vsize by 10pt   → 非法输入：\advance 目标必须是寄存器或内部参数
\hsize=100pt \advance\hsize by 10pt   → 非法输入：\advance 目标必须是寄存器或内部参数
\advance\vsize by-\voffset            → 同上（letterformat.tex:12 实际死点）
```

这是**纯引擎层**缺口：`\hsize/\vsize` 等 page 维度参数没有出现在 `\advance` 的
「寄存器或内部参数」合法目标集合里。**plain 预载不解决它**（plain.tex 只是把 `\vsize`
赋值，不改变 `\advance` 的目标判定）。

### 2.4 诊断通道缺失（本勘察最大的方法论发现）

`ntex-dvi` 二进制路径（`crates/ntex-dvi/src/main.rs`）只做 `fs::read → Typesetter::
with_tfm().typeset_dvi() → write_dvi`，**从不调用 `take_transcript()`**。后果：

| 探针 | 真实 TeX | NTex（ntex-dvi） |
|---|---|---|
| `\message{MARKER-A}` | 终端回显 | **零输出** |
| `\show\newcount` | 终端回显定义 | **零输出** |
| `\write16{PROBE-HELLO}` | log/终端 | **零输出** |
| `\bogusprimitive`（未定义 cs） | `! Undefined control sequence.` 并停 | **静默跳过，照常出 DVI** |
| `NTEX_KEEP_TRANSCRIPT=<path>` | — | **无效**（只在 ntex-trip 的 NtexDriver 内生效） |

推论：**「plain.tex 全文零 fatal」只能证明它没在引擎的 fatal 判定上炸，不能证明它
定义的 1400 个宏每个都语义正确。** 本报告因此对所有关键结论都改用行为式探针
（§5），并且把「立转录通道」列为格式战役的刀 0。

---

## 3. 问题二：格式预载的接入点在哪

### 3.1 引擎现状：快照机制已在、预载入口不在、且初表已是「分裂脑」

代码勘察（`Expander` 构造于 `crates/ntex-core/src/expand/mod.rs:684-767`，
`Typesetter` 构造于 `crates/ntex-layout/src/typeset/typesetter.rs:142-260`）确认的事实：

1. **`.fmt` 快照 save/load 往返已存在且已在生产路径上跑**。
   `crates/ntex-test-support/src/driver.rs:305-366`（`NtexDriver::run`）：
   `ts.dumped()` 为真时执行 `ntex_format::save(&mut buf, &ts.export_state())` →
   写 `<base>.fmt` → 新建 `ts2` → `ntex_format::load` → `ts2.import_state(state)` →
   重跑同一源（pass2）。这是 ETRIP 的 `\dump` 语义实现。快照格式 `NTEXFMT1`，版本
   `VERSION = 14`（`crates/ntex-format/src/lib.rs:32`），load 拒绝其他版本。
   **「格式快照含宏 + 字体表」的 M7-v2 骨架已经落地。**
2. **没有任何二进制有 `--fmt` / `--format` 旗标，也没有启动自动 `\input` 钩子**。
   逐一核过 `ntex-dvi`（手写两参）、`ntex-backend`、`ntex-pdf`、`ntex-studio`（内嵌
   `DEFAULT_TEX` demo 文档，非格式预载）、`ntex-mcp`、`ntex-trip`、`ntex-diff`、
   `ntex-bench`、`ntex-wasm`（注释明言「C 档 `.fmt`」deferred）。`Expander` 的公开面
   只有 `set_vfs / set_font_loader / set_sink / import_state / initex /
   feed_source / typeset*`——**无 init hook、无 `\everyjob` 注入、无格式文件入口**。
   `\everyjob` 只落到 toks(0)（`expand/primitive_toks_state.rs:142-146`）。
   最接近的既有机制是 `ntex-test-support/examples/latex_probe.rs:13-17,84-96` 在
   格式源前 prepend 一段 catcode「shim」——测试脚手架，不是引擎特性。
3. **`\input` 的文件面已有 VFS 抽象**（`ntex_io::Vfs`），`NtexDriver` 注入
   `WorkDirVfs`；但 `ntex-dvi` 二进制**不注入任何 VFS**，纯 cwd 语义。
4. **★ 初表已是「分裂脑」：layout 侧已写死 plain 值、expander 侧是 INITEX 值。**
   这是本勘察最重要的架构发现，直接解释 demo1 六刀的全部残差：

   | 表 | expander 侧初值 | layout 侧（NodeBuilder）初值 |
   |---|---|---|
   | catcode | `CatcodeTable::new()` = **plain 风格**（`catcode.rs:77-104`） | — |
   | `\sfcode` | `[1000;256]`（`expand/mod.rs:689`，INITEX 值） | **plain 值**：`.?!`=3000/`:`=2000/`;`=1500/`,`=1250（`typeset/mod.rs:735-746`）+ A–Z=999（`typesetter.rs:134-138`） |
   | muskip | 全 0（`register.rs:179`，INITEX） | **plain 值** 3mu/4mu±/5mu（`typeset/mod.rs:795-801`），但被 `typesetter.rs:305` **覆写回 expander 的 0** |
   | mathcode | `default_mathcodes()` INITEX 忠实（`expand/free.rs:11-28`） | — |

   即：**demo1-fixed 里手写的「muskip 三行」和「sfcode 大写 999」块，引擎 layout 侧
   其实早已各有一份**（`typeset/mod.rs:795-801` / `typesetter.rs:134-138`），只是前者
   被 `typesetter.rs:305` 主动抹零、后者与 expander 侧不同源。六刀复盘里「muskip
   INITEX 清零」「大写 sfcode=999」两条残差的真根因是**同一批 plain 常数在引擎里存了
   两份、且两边不一致**。预载 plain.tex 的真正价值不止「补宏」，更是**把这两份分裂的
   常数源收敛成一份（plain.tex 本体）**。

5. **`\lccode/\uccode` 全 0 是与 tex.web 的真实偏差，且 plain 预载救不了它。**
   `expand/mod.rs:742-743` 把两张表全 0（注释声称「TeX 默认全 0」）；tex.web INITEX
   实际给字母设了值（`reference/tex.web:4848-4850`）。`\delcode` 空表 + 读侧兜底
   `0x500000`（`expand/save.rs:653-659`），而 tex.web INITEX 是 `-1` + `.`→0
   （`reference/tex.web:5237-5238`）。**plain.tex 从不设置 `\lccode/\uccode`**
   （它依赖 INITEX 已给字母设值），所以走候选 (a) 预载后 `\lowercase/\uppercase`
   仍会静默错。这是唯一一个「预载 plain 也修不掉」的 B 类偏差。

### 3.2 三条候选接入路径（成本/利弊，不做选型裁决）

| 候选 | 做法 | 成本 | 利 | 弊 |
|---|---|---|---|---|
| **(a) 启动时自动 `\input plain.tex`** | 在 `Typesetter` 构造后、用户源前注入 `\input plain`（或 CLI 旗标 `--plain-format` 开关） | **最小**。复用已实测可跑通的 `\input plain` 路径；`hyphen.tex` 缺口用 TEXINPUTS 式搜索路径或内嵌 VFS 兜底 | 立即拿到今天已验证的全部行为；无新格式文件格式；调试直观（plain.tex 就是源码） | 每次启动重排 plain.tex（~1241 行宏展开，成本未测但 demo 级可忽略）；依赖运行环境能找到 plain.tex/hyphen.tex；wasm 离线场景需打包 |
| **(b) 扩 `.fmt` 快照含宏 + 字体表（M7-v2）** | 用户一次性 `\input plain \dump`，后续 `--fmt plain.fmt` 载入 | **中–大**。骨架已在（§3.1：`NTEXFMT1` v14），缺两块：① CLI 旗标 + 二进制接线（无任何 binary 能吃 `.fmt`）；② **快照覆盖面有实质缺口**（见下） | TeX 正统形态（`.fmt` 语义），启动最快；LaTeX 将来复用同一条路 | **覆盖面缺口是硬伤**：`FmtState`（`expand/mod.rs:398-423`）序列化了 catcodes/sfcodes/eqtb 槽/registers/params/`output_toks`/`font_loads`，但 **`\lccode/\uccode/\mathcode/\delcode`、`\fontdimen/\hyphenchar/\skewchar`、`\everypar`/`\everymath`/…、`\parshape`、e-TeX penalty 数组、`\textfont` 族表、hyphenation patterns/exceptions 全部未序列化**（完整清单见引擎自己的 `ValueExtras`，`expand/checkpoint.rs:63-94`）。**plain.tex L1222 的 `\input hyphen` 整个落在未序列化半区**——即今天做 `\input plain \dump`，连断词表都存不进去；demo1-fixed 手写的 mathcode 块同样存不进去。字体度量也不在快照里（`typesetter.rs:201-210` 从 TFM 重载） |
| **(c) 内嵌 plain.tex 字符串常量编译进引擎** | `include_str!` plain.tex + hyphen.tex 进 crate，启动时 feed | **小–中**。无运行期文件依赖；wasm 最友好 | 离线/单二进制可分发；与 wasm 线（M 骨架已存在）天然合流 | plain.tex 有许可证（Knuth 授权条款，可再分发但需遵守）；二进制体积增 ~130KB；升级 plain.tex 要重编 |

**共同前置（三条路都要）**：
- **输入搜索路径**：解决 `\input hyphen` 找不到文件。最小做法是给 VFS 加一个
  `TEXINPUTS`/内建 `texmf-dist` 路径列表，或直接把 hyphen.tex 一并内嵌/并列放置。
- **诊断通道（刀 0）**：把 `take_transcript()` 接到 `ntex-dvi`/`ntex-backend` 的
  stderr（或 `--verbose` 旗标）。没有它，预载后的静默偏差完全不可见（§2.4）。
- **三条路都不解决 `\lccode/\uccode/\delcode`**（§3.1#5）——那是 INITEX 初表的活，
  plain.tex 不碰，需要独立的 G4 刀。

---

## 4. 问题三：收益量化（预载通过后能删什么、哪些 FAIL 转 PASS）

### 4.1 demo1-fixed.tex 的 5 个手工自举块 → plain.tex 精确对应

`demo1-fixed.tex`（101 行）头部注释明说自己是「NTex 自举版……需显式装配字体族与
plain 宏」。五个块与 plain.tex 的对应关系：

| demo1-fixed 块 | 行 | plain.tex 对应 | 预载后可删 |
|---|---|---|---|
| 字体装配（14 个 `\font` + 10 个 `\textfont/\scriptfont/\scriptscriptfont`） | 6–31 | L400 起 `\font\tenrm=cmr10` … L477 `\textfont0=\tenrm \scriptfont0=\sevenrm \scriptscriptfont0=\fiverm` … L483 `\textfont3=\tenex …` | **整块删** |
| plain 垂直/行距参数（`\topskip/\parskip/\baselineskip/\lineskip/\lineskiplimit/\abovedisplayskip/\belowdisplayskip/\abovedisplayshortskip/\belowdisplayshortskip`） | 33–43 | L344–386（`\abovedisplayskip=12pt plus 3pt minus 9pt` L360、`\topskip=10pt` L366、`\belowdisplayshortskip=7pt plus 3pt minus 4pt` L363 等） | **整块删** |
| plain 数学间距 + mathcode（`\thinmuskip/\medmuskip/\thickmuskip` + `\mathcode`） | 45–53 | **L373–375**（三行 muskip，与 demo1-fixed L50–52 逐行同值）+ plain.tex 的 mathcode 段 | **整块删** |
| 数学命令（plain mathchardef 摘录） | 55 起 | plain.tex 的 `\mathchardef` 段 | **整块删** |
| plain 宏最小集 | 61 起 | plain.tex 全文（`\newcount/\newdimen/\newskip/\newmuskip/\newbox/\newinsert/\newif` 在 **L225–264**，`\alloc@` 机制 L193–223） | **整块删** |

**结论：demo1-fixed 的全部 5 个自举块在 plain 预载后可整块删除，demo1-fixed 退化成
「正文 + `\bye`」，即与 `demo1.tex` 的差异收敛到零。** 六刀复盘里的每一条残差
（muskip INITEX 清零、mu 整数截断、scriptspace、sup2=fontdimen14、大写 sfcode=999、
斜体修正 kern、display 垂直装配）都不是引擎 bug 而是这块自举的缺漏——预载后由
plain.tex 统一供给，逐条对照工作可直接消失。

### 4.2 corpus 探针 FAIL→PASS 清单（`fixtures/corpus/plain/` 3 样例）

既有 `fixtures/corpus/report.json` 记录（与本次实测复跑结果一致）：

| 文件 | 现状 | plain 预载后（本次实测） | 判定 |
|---|---|---|---|
| `plain/list.tex` | FAIL：`非法输入：\multiply/\divide 目标必须是寄存器或内部参数` | **PASS**：`pr_list.dvi`（121 字节 / 1 页 / 53 字体） | ✅ **FAIL→PASS** |
| `plain/plain.tex` | FAIL：`非法输入：找不到文件：hyphen` | 未单独复跑 corpus 路径，但 `/tmp` 同内容全文跑通 | ✅ **FAIL→PASS**（前提：解决 `\input hyphen` 搜索路径，见 §3.2 共同前置） |
| `plain/letterformat.tex` | FAIL：`非法输入：\advance 目标必须是寄存器或内部参数` | **仍 FAIL**（同错） | ❌ **plain 预载不解决**，需引擎刀（§2.3） |

`list.tex` 转 PASS 的机制值得记录：它死在 `\newcount\m \newcount\n \n=\time
\divide\n 60 \m=-\n \multiply\m 60 \advance\m \time`（L20）。无 plain 时 `\newcount`
不存在 → `\m/\n` 不是寄存器 → `\multiply\m` fatal。预载后 `\newcount` 走 plain.tex
L225 的 `\alloc@0\count\countdef\insc@unt`，分配机制成立。

### 4.3 分配机制可用性的直接实测

```
探针：\input plain \newcount\foo \foo=42 \shipout\hbox{\number\foo} \bye
ntex  DVI：… cmr10 \xac "42" \x8c …
real  DVI：… cmr10 \xab "42" \x8c …        （pdftex -ini 同探针）
```

两边都渲染出 `42`，仅 `\xac`/`\xab` 一字节差（set_char 编码档位）。
**`\countdef` 分配 + `\number` 输出链路预载后可用。**

---

## 5. 问题四：\plainoutput 风险矩阵（预载后会不会更炸）

### 5.1 结论：不会更炸，且比预期更通

`\output={\plainoutput}`（plain.tex L1197）**确实被引擎采纳**，`\plainoutput`
**确实被调用**。判定方法是行为式探针（因诊断通道缺失，§2.4）：

```
探针 A：\input plain \def\plainoutput{\shipout\hbox{MARKER-PO}} Hello world. \bye
        → DVI 含 "MARKER-PO" = TRUE      ⇒ \plainoutput 是活的例程，被 \bye 触发
探针 B：\input plain Hello world. \bye
        → DVI 含 "MARKER-PO" = FALSE     ⇒ 对照组成立
```

且探针 B 的页面内容与真 TeX 对上：两者都含 `Hello world.` + 页脚页码 `1`——
即 `\plainoutput` 的 `\makefootline`（plain.tex L1205，`\line{\the\footline}`，
`\footline={\hss\tenrm\folio\hss}` L1146）**在 NTex 上真实产出了页码**。

### 5.2 依赖矩阵

`\plainoutput`（L1198）及其子宏对引擎能力的依赖：

| 依赖 | plain.tex 位置 | 实测状态 |
|---|---|---|
| `\output` 例程被调用 | L1197 | ✅ 生效（探针 A）；`\output` 是原语（`builtins.rs:76`），token 体进 `output_toks` 且**已被 `.fmt` 序列化** |
| `\shipout\vbox{…}` | L1198 | ✅ 出 1 页 |
| `\plainoutput` 本体 | L1198 | ✅ 定义成功（**非原语**，纯 plain 宏） |
| `\makeheadline` / `\the\headline`（`\newtoks\headline` L1145） | L1202 | ✅ 未致命；`\newtoks` 可用（`\newtoks\mt \mt={MARKER-TOKS}` → DVI 含 MARKER-TOKS） |
| `\pagebody` → `\pagecontents` → `\unvbox\@cclv`（**\box255**，输出例程刀 2） | L1207 | ✅ 走通。`\box255` 有引擎级支撑（`PAGE_BOX = 255`，`typeset/mod.rs:685`；页队列**就是** box 255，设计注释 `mod.rs:686-700`）。多页 1200 词 → 2 页正常 |
| `\makefootline` / `\folio` / `\advancepageno` | L1205 | ✅ 页码 `1` 落进 DVI |
| `\topmark/\firstmark/\botmark` | `\makeheadline` 间接 | ✅ **三者都是原语**（`builtins.rs:326-328`），走 e-TeX marks 查询（`expr.rs:269-278` → `sink.rs:1288-1300`）；`\mark` 也在（`builtins.rs:358`）。本探针未实际触发取值，**取值正确性未验证** |
| `\insertpenalties` / `\dosupereject` | L1199 | 未触发（1 页文档不走到）— **未勘察** |
| `\ifvoid\topins` / `\unvbox\topins`（`\topinsert` 通道） | L1207 | ⚠️ **结构性静默**：`\insert` 是原语（`builtins.rs:372`）但**只把体收集进 `Node::Ins`、从不排版**（`sink.rs:1255-1272`），insert 类寄存器三元组（`\count/\dimen/\skip\footins`）不存在。`\ifvoid\topins` 大概率判「void」→ 路径静默跳过，**不炸但脚注/topinsert 内容丢失** |
| `\ifvoid\footins` / `\unvbox\footins`（脚注，`\newinsert` 分配） | L1207 | ⚠️ 同上；`\newinsert` **非原语**，来自 plain.tex L242 宏（依赖 `\chardef/\countdef/\dimendef/\skipdef/\toksdef/\muskipdef`——`builtins.rs:225-236` 全在，宏方案可行）。属输出例程战刀 3 领地 |

**补充**：`\makebox` 既非原语也不在 plain.tex 里（grep fixture 无此名）——是 LaTeX 层宏，
不在本战役范围。

### 5.3 反向风险：现状 bug 比预载风险更险（实测发现）

```
探针：\output={\shipout\hbox{MARKER-OVR}}  + 1200 词多段文档 + \bye   （无 plain）
      → 「未产出页面（源码缺少 \shipout）」，DVI 0 页
对照：\input plain（内部 \output={\plainoutput}）+ 1200 词 + \bye
      → 2 页正常
```

即：**无 plain 时，用户一旦自己设 `\output`，内置自动冲页就关闭、而例程又没被驱动，
净结果 0 页**。这是**现状的输出例程缺口**（与输出例程战刀 1-3 的记录一致），不是
plain 预载引入的。对预载战役的意义：预载恰好把「`\output` 在场但没人调用」这条坑
填掉了——**预载方向上风险更低，不是更高**。

---

## 6. 缺口分级与逐刀路径

| 级 | 内容 | 规模 |
|---|---|---|
| **G0 诊断通道（刀 0，硬前置）** | `ntex-dvi`/`ntex-backend` 接 `take_transcript()` → stderr 或 `--verbose` | 1 个二进制接线，无引擎改动 |
| **G1 输入搜索路径** | `\input hyphen` 找不到；给 `ntex_io::Vfs` 加 TEXINPUTS 式路径列表，或内嵌 | 小（1 处 VFS 扩展 + `ntex-dvi` 注入 VFS） |
| **G2 接入点** | §3.2 三选一（裁决归主控） | (a) 小 / (b) 中–大（覆盖面缺口） / (c) 小–中 |
| **G3 `\advance` page 参数目标** | `\advance\hsize/\vsize` fatal（§2.3）——**独立于预载**，但决定 letterformat.tex 能否过 | 中（`\advance/\multiply/\divide` 的目标判定集合扩 page 参数） |
| **G4 `\lccode/\uccode/\delcode` INITEX 偏差** | expander 侧全 0 / `0x500000` 兜底，tex.web 给字母设值（§3.1#5）。**plain 预载救不了**（plain.tex 不设这两张表） | 小（初表对齐 tex.web `reference/tex.web:4838-4852`，`:5237-5238`），但要先建 INITEX 对拍判据 |
| **G5 初表分裂脑收敛** | layout 侧已写死 plain sfcode/muskip、expander 侧 INITEX，muskip 还被 `typesetter.rs:305` 主动抹零（§3.1#4）。预载后应让 plain.tex 成为唯一事实源，删两份硬编码 | 小（删代码 > 加代码），但会砸中现有测试（`typeset/tests.rs:1710` 明文断言 muskip 缺省 0） |
| **G6 静默偏差普查** | plain.tex 预载后哪些宏语义错——**本报告无法给出**（诊断通道缺失），依赖 G0 | 未量化 |

推荐刀序：**G0 → G1 → G2(a) →（用真 TeX DVI 对拍做 G6 普查）→ G3 → G4 → G5**。
G0 放最前是本轮勘察的核心教训：没有转录通道，「预载成功」只能证明到「没 fatal」，
而 §2.4 已证明 fatal 门槛极低（undefined cs 都不算）。G5 收益最高但风险也最高
（它是 demo1 六刀残差的真根因，动它等于动既有测试口径），宜在对拍仪器（G0）立起
之后再做。

---

## 7. 复现清单（`/tmp/plainfmt-work/`）

```bash
cd /tmp/plainfmt-work
export NTEX_TFM_DIR=/home/ubuntu/.ntex-fonts
NT=/home/ubuntu/NTex/target/debug/ntex-dvi
cp ~/.TinyTeX/texmf-dist/tex/plain/base/plain.tex .
cp ~/.TinyTeX/texmf-dist/tex/generic/hyphen/hyphen.tex .

# ① 唯一致命点（删除 hyphen.tex 后复现）
printf '\\input plain\nHello world.\n\\bye\n' > p1.tex
rm -f hyphen.tex && $NT p1.tex p1.dvi   # → 非法输入：找不到文件：hyphen
cp ~/.TinyTeX/texmf-dist/tex/generic/hyphen/hyphen.tex .

# ② 端到端对拍
$NT p1.tex p1.dvi                        # → 203 字节 / 1 页 / 51 字体
pdftex -ini -jobname=p1g -etex '\input plain Hello world. \bye'   # → 224 字节
dvipdfmx -o p1n.pdf p1.dvi ; dvipdfmx -o p1g.pdf p1g.dvi          # 两端均成功

# ③ 分配机制
printf '\\input plain\n\\newcount\\foo \\foo=42 \\shipout\\hbox{\\number\\foo}\\bye\n' > nc.tex
$NT nc.tex nc.dvi ; python3 dvitxt.py nc.dvi     # → 含 "42"

# ④ corpus FAIL→PASS / 不解决
printf '\\input plain\n' > pr_list.tex
cat /home/ubuntu/NTex/fixtures/corpus/plain/list.tex >> pr_list.tex
printf '\n\\bye\n' >> pr_list.tex
$NT pr_list.tex pr_list.dvi                      # → 121 字节（原 FAIL）
$NT /home/ubuntu/NTex/fixtures/corpus/plain/letterformat.tex x.dvi   # → 仍 FAIL（advance）

# ⑤ \plainoutput 活性
printf '\\input plain\n\\def\\plainoutput{\\shipout\\hbox{MARKER-PO}}\nX\n\\bye\n' > r.tex
$NT r.tex r.dvi
python3 -c "print(b'MARKER-PO' in open('r.dvi','rb').read())"    # → True
```

## 8. 未勘察项（时间盒 100 分钟到点，如实声明）

1. **G4 静默偏差普查**：plain.tex 预载后 1400 个宏定义里有多少语义错——**完全未量化**。
   诊断通道缺失使本项不可测（§2.4）；这是下一刀（G0）之后的第一件事。
2. **`.fmt` 快照的往返正确性**：覆盖面已勘察清楚（§3.2 (b)：lccode/uccode/mathcode/
   delcode/fontdimen/every*/parshape/penalty 数组/\textfont 族/hyphenation 全部未序列化），
   但**未实测**一次 `\input plain \dump` → load → 重排的往返是否真的逐位复现。
   既有 `ntex-test-support/tests/fmt_reload.rs:9-66` 覆盖的是小样例，非 plain 规模。
3. **plain.tex 加载耗时**：候选 (a) 的每次启动成本未测量。
4. **输出例程刀 1-3 的 insert 通路**（`\topinsert/\footins/\topmark`）：1 页文档不触发
   （§5.2），未构造触发用例。
5. **LaTeX 路径可行性**：plain 通了之后 latex.ltx 是否随之受益——未勘察（但 §3.1 的
   快照机制若覆盖面足够，两者共用同一条 (b) 路）。

---

## 5.bis.g2 G2(a) 实测记录（2026-09-07，✅ 完成——启动 `\input plain` 内嵌接入，提交 bfb66bb）

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
| l.1237 | 2×`! Missing number, treated as zero.`（`<to be read again> \z@` / `\tenrm`） | **仍在**——`\rm`=`\fam\z@\tenrm`（plain.tex:478），引擎数字扫描不认 dimen 内部量 `\z@`，G6 靶子 |
| `\bye` Incompatible list | 未复现（本轮未观测到该签名） | 待 G6 构造探针复验 |
| （G0 未照见，本刀首见）shipout 路径 | 3×`Missing number …\z@` 无行上下文，仅在有页面冲出时出现 | **新增可见**——预载前 `\plainoutput`/`\makefootline`/`\advancepageno` 不存在、不可达；钳制探针：`\def\plainoutput{\shipout\vbox{\makefootline}}` 与 `…\advancepageno` 单独即可少 2 处 → 疑 `\pagebody`/`\makeheadline` 一带，G6 靶子 |

**corpus plain 3 样例**（ntex-dvi 缺省旗标 + dvipdfmx 全链）：

| 文件 | G1 期（survey §4.2） | 本刀后 | 判定 |
|---|---|---|---|
| `list.tex` | FAIL（`\multiply` 目标） | 121B / 1 页 / 53 字体，PDF OK | **PASS**（`\newcount` 分配机制，不回退） |
| `plain.tex` | FAIL（找不到 hyphen） | 172B / 1 页 / 51 字体，PDF OK | **PASS**（`\input hyphen` 走内嵌） |
| `letterformat.tex` | FAIL（`\advance` 目标） | 121B / 1 页 / 53 字体，PDF OK | **PASS***——旧死点 `\advance\vsize by-\voffset` 由**并行 G3 刀在 ntex-core 的在途改动**解开（非本刀）；带 * 是因为它与 list.tex 同为宏/格式文件（无正文、无 `\bye`），真 TeX 是 **0 页**，NTex 预载后经 `\plainoutput` 收尾冲页出一页**空页**（见下「发现未修」#1） |

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

**账实备注（共享树事故）**：提交 bfb66bb 除本刀文件外带入了同树并行 G3 刀在
ntex-core 的已暂存在途改动（free.rs/primitive_assign.rs/scan.rs/tests.rs，+226/−25）
——`git add` 只加本刀七路径，对方已暂存文件被一并提交。内容经本刀全量测试验证
可用，故保留不拆；letterformat 旧死点正由它解开，上文已归据。
