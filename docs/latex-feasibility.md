# LaTeX 兼容可行性勘察报告（2026-09-04，基于 HEAD 023ff2a）

> 目标：真实尝试以 iniTeX 方式加载 `latex.ltx`（→ 构建 `latex.fmt` → `\documentclass{article}`
> → PDF），量化引擎语义缺口。**本阶段只勘察不修引擎**（唯一新增代码是勘察工具
> `crates/ntex-test-support/examples/latex_probe.rs`，不动核心语义）。

## TL;DR

- 现成的 fmt 构建链路**可用**（TRIP/ETRIP 已验证的 `Typesetter::typeset_bytes → \dump →
  ntex_format::save/load`），不需要新驱动，一个 ~100 行勘察 example 即可发起真实加载。
- latex.ltx（2026-05 生成版，22838 行 / 776142 B）**能被引擎读入并处理全程**（约 43 ms），
  引擎自带的错误恢复让它"吞错推进"；但**远未到可用**：首个真阻塞是
  **INITEX 初始 catcode 表缺失**（L100 即误报"格式已预载"），随后
  **文件通道未落盘 / 缺文件静默跳过**，以及一个**可复现死循环挂死**
  （`\input texsys.cfg` 成功后触发，哪怕该文件为空）。
- **expl3 是硬依赖**：latex.ltx L1120–1146 显式 `\IfFileExists{expl3.ltx}` → 缺则
  `\errmessage{LaTeX requires expl3}`；L1157 起大量 `\ExplSyntaxOn/Off` 内联。
  本轮**未触达**该层（挂死点在它之前），量级评估按"待实测"处理。
- 正面结论：**e-TeX 原语层已基本就位**（`\eTeXversion`/`\numexpr`/`\dimexpr`/
  `\glueexpr`/`\scantokens`/`\everyeof`/`\ifprimitive` 等，latex.ltx L113/L417 的
  e-TeX 守卫均未报错）。
- 结论：**可行，但不是"差几个原语"**；阻塞集中在「初始状态（INITEX 表）+ 文件 I/O
  语义 + 一处死循环」三类，预计 1–2 周可推进到"latex.fmt 能构建出、并在 pass2 冒烟"，
  `\documentclass{article}` 需再叠加 NFSS/字体机制（量级另估）。

---

## 1. 勘察方法与工具

### 1.1 引擎的 fmt 构建入口（现成路径，已确认）

`crates/ntex-test-support/src/driver.rs`（`NtexDriver::run`，被 `ntex-trip --driver ntex` 使用）：

```rust
let mut ts = ntex_layout::Typesetter::with_tfm();      // 或 with_tfm_paginated()
ts.set_vfs(Box::new(WorkDirVfs { wd }));               // \input/\openin/\write 的 VFS
ts.typeset_bytes(src)                                  // 跑到 \dump 停（ts.dumped() == true）
let mut buf = Vec::new();
ntex_format::save(&mut buf, &ts.export_state())?;      // 存 .fmt
let mut ts2 = ntex_layout::Typesetter::with_tfm();
ts2.import_state(ntex_format::load(&mut &buf[..])?);   // pass2 重载
ts2.typeset_bytes(doc)
```

最小调用即此 8 行；本轮勘察将其封装为：

- **`crates/ntex-test-support/examples/latex_probe.rs`**（本次唯一入库代码）
  用法：`cargo run -p ntex-test-support --example latex_probe -- <file.tex> [--shim]`
  输出：pass1 OK/ERROR、`dumped?`、完整转录（写 `<file>.transcript`）、若 `\dump`
  则保存 fmt 并做 pass2 冒烟。`--shim` 注入 INITEX catcode 归位（见 §3-A1）。

### 1.2 latex.ltx 的获取（题给 URL 已失效）

- `https://mirrors.ctan.org/macros/latex/base/latex.ltx` → **404**。CTAN 的
  `macros/latex/base/` 只发布 `.dtx`/`.ins` 源与文档，**生成的 `latex.ltx` 不入 CTAN 树**
  （需本地跑 docstrip 生成）。
- 改从 TeX Live tlnet 包档案取**生成版**（与 TeX Live 所装一致，LPPL）：
  `https://mirror.ctan.org/systems/texlive/tlnet/archive/latex.tar.xz`（264 KB），
  解包得 `tex/latex/base/`：`latex.ltx`（22838 行 / 776142 B）+ 配套
  `fonttext.ltx`/`fontmath.ltx`/`hyphen.ltx`/`preload.ltx`/`ltxdoc-extra.ltx`/
  `texsys.cfg`/`utf8.def`/`*.fd` 等。
- 勘察工作目录：`/tmp/latexsurvey/`（未入库；复现步骤见 §7）。

---

## 2. 加载进度（全部实测）

| 阶段 | 结果 |
|---|---|
| 读入 latex.ltx | 全文 776142 B 进入 Source 栈 |
| **L100** | `\ifnum\catcode`\{=1 \errmessage{LaTeX must be made using an initex with no format preloaded}` **触发**（引擎初始表 `{`=1，见 §3-A1）；引擎把 errmessage 记入转录后**继续执行**（不中断） |
| L166 | `\input texsys.cfg` —— **分水岭**（见下） |
| texsys.cfg **缺席** | 全文处理完毕，**~43 ms**，转录仅 1 个错误（L100 那条）；**未到 `\dump`**（`dumped=false`，尾部 `\@input{latex2e-first-aid-for-external-files.ltx}`/`\errorstopmode`/`\dump` 未执行到） |
| texsys.cfg **在场**（任意内容，**含 0 字节空文件**） | **挂死**：watchdog 报 `疑似挂死：11940ms 无心跳`，~10 min 无进展、RSS ~1 GB（2GB VM 已入 swap） |
| 前缀二分（1000…22838 行，cfg 缺席） | 任意前缀都 ~22–43 ms 完成 → 挂死不是"某个超大定义"，而是 cfg 在场才触发的状态污染 |

挂死时的 watchdog 栈（唯一现场证据）：

```
stack=Source(776142B,pos=604682,state=MidLine) | TokenList(1tok) | Bytecode(pc=16)
      | MacroArg(1tok) | TokenList(1tok) | Bytecode(pc=49)
```

`pos=604682 ≈ L17335`（ltpictur.dtx 的 `\@circ`/`\bezier` 区域），带两层宏帧 + 宏参数帧，
且**单步 >10 s 无心跳** → 指向"某一个展开步骤内部死循环"。注意：这与 §3-A2 的
`\openout` 未落盘矛盾被排除（`\IfFileExists` 三连探测全走 EOF 分支，`\read` 单测正常，
`t.aux` 从未被创建）——**root-cause 未定位**，属引擎修复阶段工作（见 §6-3）。

---

## 3. 阻塞点清单（按类）

### A. 引擎语义缺口

| # | 缺口 | 精确事实 | tex.web 对照 |
|---|---|---|---|
| A1 | **INITEX 初始 catcode 表缺失** | `CatcodeTable::new()`（`ntex-core/src/catcode.rs:73`）是 **plain 风格**：`{`=1 `}`=2 `$`=3 `&`=4 `#`=6 `^`=7 `_`=8 `~`=13 `^^I`=10。INITEX 应为"除 `\`=0、`%`=14、空格=10、CR=5、DEL=15、字母=11、NUL=9 外全 12"。latex.ltx L99 正靠 `{`=12 判别"纯 initex" | tex.web §1273 INITEX 默认表 |
| A2 | **文件通道未落盘** | `\immediate\openout15=texsys.aux` + `\write15{...}` + `\closeout15` 后 **`texsys.aux` 不存在**；`\write15` 文本落进了终端转录（当 `\write16` 处理）。→ 一切 `\IfFileExists` 类探测恒走"不存在"分支 | tex.web `open_out_file`/`write_out` |
| A3 | **`\input` 缺文件静默跳过** | `\input texsys.cfg`（文件缺席）无任何错误/警告直接继续；`\@input{first-aid...}` 同样。真实 TeX：`! I can't find file` 且 batchmode 致命。本轮它反而"帮"引擎跑完了全文（进度指标失真） | tex.web `file_error`/`start_input` |
| A4 | **死循环挂死** | `\input` 任一真实文件成功后（cfg 哪怕为空）触发；现场见 §2。**唯一最高优先级不明项** | （待 root-cause） |
| A5 | 错误恢复语义 | `\errmessage` 只记转录不中断（batchmode 里真实 TeX 对多数错误可继续，此处行为碰巧一致，但缺"abort/交互"语义，导致无法用错误计数判断加载质量） | tex.web `error`/interaction |

### B. 缺文件（已用 CTAN/TL 真文件补齐勘察，未 stub）

| 文件 | 状态 | 来源 |
|---|---|---|
| `texsys.cfg` | 已补（TL 版，纯注释） | `latex.tar.xz` |
| `latex2e-first-aid-for-external-files.ltx` | **缺**（在 TL `firstaid` 包，非 `latex` 包）→ 尾部 `\@input` 处 | 待取 `archive/firstaid.tar.xz` |
| `expl3.ltx` 等 l3kernel 文件集 | **缺**（见 §4） | 待取 `archive/l3kernel.tar.xz` |
| `utf8.def`/`*.dfu`/`*.fd` | 随 `latex.tar.xz` 已在（`tex/latex/base/`） | 已有 |

### C. 缺字体（未触达，按 latex.ltx 结构预警）

- 引擎仅 `~/.ntex-fonts/{cmr10.tfm,cmr10.pfb}`。latex.ltx 内 `preload.ltx`/`fonttext.ltx`
  段会 `\font\tenrm=cmr10` 之外还要求 cmr7/cmr9/cmss10/cmbx10/cmti10 等一组 CM TFM，
  `\fontdimen8\tenln`（ltpictur 前 L17318 附近）也依赖 `\tenln`(tencirc/texln)。
- **本轮未实测**（挂死点之前），量级按"需要一组 CM 字体 TFM + `\fontdimen` 已实现与否确认"估。

### D. 缺宏层

- **expl3（l3kernel）**：见 §4，硬依赖，未触达。

---

## 4. expl3 依赖结论

**结论：现代 latex.ltx（2026-05 版）硬依赖 expl3，无法绕过。**

- L1120 `\IfFileExists{expl3.ltx}` → L1146 `\input expl3.ltx`；
  L1143 缺失分支：`\errmessage{LaTeX requires expl3}`；
  L1151 起：`You need to update your installation of 'l3kernel'`。
- L1157 起 `latex.ltx` 自身含大段 `\ExplSyntaxOn ... \ExplSyntaxOff` 内联代码
  → 引擎必须支持 expl3 语法期的 catcode 切换（`_`/`:` 变 letter 等）——
  机制上就是现有 catcode 表切换，量级在"验证"而非"新建"。
- 引擎无 `\directlua`（ltluatex 段被 `\ifx\directlua\undefined` 守卫跳过，无害）。

**本轮未触达该层**：挂死点在 L~17335 之前、expl3 加载在 L1120——注意 expl3 段
**位于挂死点之前**却未先炸，说明 L1120 的 `\IfFileExists{expl3.ltx}` 走了
"文件不存在 → errmessage"分支且被吞（转录仅 1 条错误的原因待查，
L1143 那条 errmessage 未出现在转录中——这本身又是一条待核实的错误通道缺口）。

---

## 5. 正面结论（已具备，不需重做）

1. **fmt 构建链路完整**：iniTeX 式加载 → `\dump` → `.fmt` 序列化/重载 → pass2，
   TRIP/ETRIP 双 pass 已验证（`ntex-trip --driver ntex`）。
2. **e-TeX 原语层基本就位**：`\eTeXversion`/`\eTeXrevision`/`\numexpr`/`\dimexpr`/
   `\glueexpr`/`\detokenize`/`\scantokens`/`\everyeof`/`\ifprimitive`/`\interactionmode`
   （`ntex-core/src/eqtb/primitive.rs`）——latex.ltx 的 L113 e-TeX 守卫与 L417 起的
   `\numexpr` 用法均未报错。
3. **错误恢复式推进**：全文 22838 行能被吞错处理完（43 ms 量级），说明引擎吞吐
   不是 LaTeX 兼容的风险项。

---

## 6. 建议铺开路径（按依赖顺序）

| 步骤 | 内容 | 量级 |
|---|---|---|
| 1 | **INITEX 初始状态**：独立的 init catcode 表（`\`=0、`%`=14、空格=10、CR=5、DEL=15、字母=11、NUL=9，其余全 12）+ INITEX 特有的 `\par` 等空 eqtb；**不得影响现有 plain/TRIP 路径**（加构造开关，`Typesetter::initex()` 之类） | 0.5–1 天 |
| 2 | **文件通道真实现**：`\openout` 落盘（VFS write）、`\write<流>` 按流分发、`\input`/`\@input` 缺文件错误语义（batchmode 致命）、`\input@path` 搜索 | 1–2 天 |
| 3 | **挂死 root-cause**：用 `latex_probe` + 前缀二分 + watchdog 栈缩到具体构造（现场：`Source pos≈604682` + `Bytecode×2`+`MacroArg`，单步 >10 s）；先加"单步内步数上限"防挂保护再修 | 1–2 天（不确定度最高） |
| 4 | 重跑加载 → 取**下一个**阻塞点（预期：expl3 层、`\fontdimen`、preload 字体集） | 1–2 天 |
| 5 | 引入 l3kernel 源文件集（`l3kernel.tar.xz`）+ expl3 语法期 catcode 验证 | 2–5 天 |
| 6 | CM 字体组 TFM（`cm.tar.xz`）+ NFSS 起步 | 另估 |

**里程碑判定建议**：以"latex.fmt 构建成功且 pass2 冒烟（`\message` 输出版本串）"为
第一个可验证节点；`\documentclass{article}` 需等 NFSS/输出例程，量级另估。
**验证指标必须用"阻塞点位置"，不要用"跑完/错误计数"**——引擎吞错推进会让后者失真
（本轮 43 ms"跑完"实为多处缺口叠加的假象）。

---

## 7. 复现步骤

```bash
# 1) 取真源（CTAN 无生成版 latex.ltx，从 TL tlnet 取）
mkdir -p /tmp/latexsurvey && cd /tmp/latexsurvey
curl -sL -o latex.tar.xz https://mirror.ctan.org/systems/texlive/tlnet/archive/latex.tar.xz
tar xf latex.tar.xz                      # → tex/latex/base/{latex.ltx,texsys.cfg,...}

# 2) 加载尝试（cfg 在场 → 复现挂死；移走 cfg → 43 ms 跑完但未到 \dump）
~/.local/bin/ntexl cargo run -p ntex-test-support --example latex_probe -- \
    /tmp/latexsurvey/tex/latex/base/latex.ltx

# 3) 对照：cfg 缺席（唯一差异即 L166 \input texsys.cfg 成败）
cp /tmp/latexsurvey/tex/latex/base/latex.ltx /tmp/latexsurvey/flat.ltx
target/debug/examples/latex_probe /tmp/latexsurvey/flat.ltx   # 43 ms, 1 err, dumped=false
```

## 8. 2026-09-04 第二轮进展（A1 修复 + A4 root-cause + 防挂保护）

> 本轮把 §6 步骤 1（INITEX 初始 catcode 表）落地，并把步骤 3（死循环挂死）定位到根因
> 并加上 TeX 同款保护。勘察工具 `latex_probe` 新增 `--initex`。

### 8.1 A1 INITEX 初始 catcode 表 —— 已修复

- 新增 [`CatcodeTable::initex()`](`crates/ntex-core/src/catcode.rs`)：tex.web §1273
  默认表（`\`=0、`%`=14、空格=10、CR=5、DEL=15、字母=11、NUL=9，其余全 12）。
- **引擎偏差（有意）**：tex.web 的 INITEX 里 LF 是 12（行尾符由读取层剥掉、再补
  `\endlinechar`=13/CR）。本引擎扫描器是字节流直读（`input.rs::scan_token`），
  不剥行尾字节，注释跳行/空行→`\par` 都靠 catcode 5 找行尾——若按 tex.web 原样给
  LF=12，**第一条注释会一路吞到 EOF**（实测：全文 30 ms"跑完"、转录 0 字节、
  `dumped=false`，实为整文件被跳过）。故引擎的 initex 表让 LF 与 CR 同为 5。
- 通道：`Expander::initex()` → `Typesetter::initex()` → `latex_probe --initex`。
  **plain/TRIP 路径不受影响**（默认构造仍是 plain 表；TRIP pass1 继续用 plain 表，
  trip.log 对照零回归）。
- 验证：`--initex` 下 latex.ltx L98 `\ifnum\catcode`\{=1` 不再误报
  "LaTeX must be made using an initex with no format preloaded"（plain 表路径仍报，
  作对照）。

### 8.2 A4 死循环挂死 —— root-cause：无界宏递归（TeX 输入栈上限缺失）

**§2 的"cfg 在场才挂"是误判。** 复测证据链：

1. 删掉 L166 `\input texsys.cfg` 这一行 → **仍然挂死**（同一 469762048 字节分配失败）。
   cfg 缺席之所以"43 ms 跑完"，是因为 `\input` 缺文件在本引擎是**硬错误**
   （`expand/io.rs:30` `找不到文件：…`，直接终止加载）——整个加载在 L166 就停了，
   根本没走到触发点，"cfg 在场"只是"没在 L166 早退"的别名。
   **这同时纠正 §3-A3 的描述**：缺文件不是"静默跳过"而是"硬错误终止"；
   §2 中"cfg 缺席 → 全文处理完毕"实为"L166 终止"（转录里那条 `l.167 \begingroup`
   上下文行就是终止点），"43 ms"只是终止得快。
2. 前缀/截断二分定位触发点：**L17342–17399**（ltpictur 区）
   `\def\bezier#1)#2(#3)#4({\@bezier#1)(#3)(}` + `\def\@bezier#1(#2,#3)(#4,#5)(#6,#7){…}`
   + `\MakeRobust\bezier`。最小复现（60 行，`latex.ltx` L17342–17405）即挂。
3. 看门狗现场（SIGSTOP/SIGCONT 拉长墙钟后抓到，而非 10 s 一条）：
   `stack=TokenList(1tok,pos=0) | Bytecode(pc=9) × 数千`——**输入栈被同名宏的递归展开
   无界推深**。宏名由新增的栈上限报错直接点名：**`\@`**。
4. 机理：plain 表下 `@` 是 catcode 12，于是 `\def\@bezier…` 定义的是**控制符号 `\@`**
   （参数文本 `bezier#1(#2,#3)(#4,#5)(#6,#7)`），`\bezier` 被调用时其体里的
   `\@bezier` 展开成 `\@` + 字母 → `\@` 又是一个带定界参数的宏 → 定界实参扫描
   展开它自己 → 无界递归。真实 TeX 对此有 **`stack_size`（TeX Live 取 5000）上限**，
   报 "TeX capacity exceeded, sorry [input stack size=5000]" 致命终止；本引擎没有，
   于是单步内递归把 RSS 撑到 OOM（主循环 10M 步看门狗够不到——递归发生在一步之内）。

**修复**（`crates/ntex-core/src/expand/mod.rs`）：

- `call_macro`：压宏帧前检查 `stack.len() >= MAX_INPUT_STACK`（5000，tex.web
  `stack_size` 语义），转录报 "TeX capacity exceeded, sorry [input stack size = 5000]."
  后以可读错误终止（消息含递归宏名）；
- `fetch()`：同一上限的全帧型兜底（TokenList/Source 等帧的循环注入不经过 `call_macro`；
  每次压帧后必经 fetch，故这里是唯一收口点）。

**效果**：latex.ltx 全文（cfg 在场/缺席、`--initex` 与否四种组合）从"挂死 + RSS 入 swap"
变为**干净终止并给出可读错误**（~4 s、几十 MB），且报错点名递归宏。
还原了"引擎契约：畸形输入不 panic、不无限循环"。

### 8.3 阻塞点刷新

| # | 状态 | 说明 |
|---|---|---|
| A1 | **已修复** | `CatcodeTable::initex()` + `Typesetter::initex()` + `latex_probe --initex`；LF=5 为引擎行模型偏差（§8.1） |
| A4 | **已修复（root-cause + 防挂）** | 无界宏递归；TeX `stack_size` 同款上限 5000（§8.2） |
| A2 | 未动 | 文件通道未落盘（`\write15` 落转录、`\IfFileExists` 恒假） |
| A3 | **描述已纠正** | `\input` 缺文件 = 硬错误终止（非"静默跳过"）——它让加载在 L166 早退、掩盖后续阻塞点（§8.2 证据 1）；对齐真实 TeX 的 batchmode/交互语义仍是后续工作 |
| 新 | **下一阻塞点** | `--initex` 下加载推进到 **L17474**（`ltmath`/`\@inmatherr` 区）后仍触发输入栈超限；转录累计 2576 条错误（2540 条 `! Undefined control sequence`，多为 `\@@`/`\@inmatherr` 等内核 cs）——下一刀应先解"错误恢复后 `\@` 类 cs 名失配"与 expl3 缺失 |

### 8.4 本轮改动清单

- `crates/ntex-core/src/catcode.rs`：`CatcodeTable::initex()` + 单测（tex.web §1273 + LF 偏差）。
- `crates/ntex-core/src/expand/mod.rs`：`Expander::initex()`；`MAX_INPUT_STACK=5000`
  上限（`call_macro` 前置检查 + `fetch` 兜底）；`NTEX_TRACE_STACK` 诊断开关
  （每原语打印输入栈摘要，挂死定位用，与 `NTEX_TRACE_EXEC` 同款）。
- `crates/ntex-layout/src/typeset/typesetter.rs`：`Typesetter::initex()`。
- `crates/ntex-test-support/examples/latex_probe.rs`：`--initex` 开关。
- `crates/ntex-core/src/expand/tests.rs`：无界宏递归 → 输入栈超限的回归单测。
- 全量回归：`make check`（fmt/clippy/test）全绿。
- **TRIP/ETRIP 零回归验证法**：`cargo run -p ntex-trip -- --driver ntex` 在 HEAD 本就
  失败（数学组 `group_end 无配对 group_begin`，属已知的数学/模式机未完成区）——
  在 HEAD 干净 worktree 上重建后逐字节比对，本轮前后输出**一致**（仅临时目录名不同）；
  且本轮改动在该运行中零触发（无 `输入栈超限`、无 `initex` 路径）。门禁以 `make check` 为准。

- 新增 `crates/ntex-test-support/examples/latex_probe.rs`（勘察工具，~100 行，不动引擎语义）。
- 新增本报告。**未修改任何引擎核心语义**；`--shim` 的 INITEX catcode 归位只存在于勘察工具内。

---

## 9. 2026-09-04 第三轮进展（A2 文件通道 + A3 缺文件错误语义）

> 本轮落地 §6 步骤 2（文件通道真实现）。依据 tex.web §24598-24601 / `write_out`
> / `out_what` / `prompt_file_name` 重排了写流分发与缺文件错误语义。

### 9.1 A2 文件通道 —— 已实现

**关键 tex.web 事实**（此前实现的主要偏差来源）：`\write` 的流号先**钳制**为
`j`（负→17、>15→16，`write_open[16]`/`write_open[17]` 恒 false），然后
`write_out` 按 `if write_open[j] then selector:=j else begin … "write to the
terminal if file isn't open" end` 分发。即：

| 流号 | `\openout` 过 | tex.web 落点 |
|---|---|---|
| 0..15 | 是 | 该文件（`selector:=j`） |
| 0..15 | 否 | **终端+log**（非丢弃！） |
| >15（j=16，如 `\write16`、LaTeX `\typeout`=`\write17`） | — | 终端+log |
| 负（j=17，`\write-1`/`\wlog`） | — | 仅 log |

旧实现把「流 15/16/17」与终端混同、且把未打开流的延迟写静默丢弃——流 15 是
合法**文件流**（latex.ltx L176 `\immediate\openout15=texsys.aux`），而
`\typeout` 用流 17（L129）、`\GenericError` 用未打开的流 0（`\@unused`）。

- `crates/ntex-core/src/expand/io.rs::exec_write` 按上表重排：已 `\openout` 的
  0..15 → VFS 文件；`\immediate` 的其余流 → 转录；负流号非 immediate →
  既有延迟 log 队列（TRIP L441 参考输出 `write->…` 已验证路径）。
- **RFC-3 §4.4 补齐**：`\openout` 语义为覆盖——新增 `WriteStream.created`，
  首次实际写出用 `Vfs::write` 清空目标（丢弃上次作业残留），其后 `append`；
  `\closeout` 对登记过路径但零写出的流也创建空文件（tex.web `a_open_out`→
  `a_close`）。
- `\newwrite` 分配范围 0..=17 → **0..=15**（16/17 是钳制哨兵，不可打开；
  LaTeX `\newwrite` 同为 `\sixt@@n` 上限，L367）。
- **带引号文件名**：`scan_file_name` 支持 `"name with spaces"`（web2c 剥引号；
  latex.ltx L1094 `\openin\@inputcheck"#1" `、L9766 `\openout\@partaux "#1.aux"`）。
- 移除「为 `\write`/`\closeout` 补空流槽」的逻辑——幽灵槽会让后续 `\newwrite`
  跳号（分配漂移）。
- 勘察工具 `latex_probe` 的 `SurveyVfs` 增加前导 `./` 归一（MemVfs 是精确字符串
  键，LocalVfs 天然解析 `./x`；latex.ltx L195 `\IfFileExists{./texsys.aux}` 正
  依赖该语义）。

**验证**：`latex.ltx` 头部 L160-215 的 `texsys.aux` 探测区首次真实走通——
`\immediate\openout15` 落盘 → `\IfFileExists{./texsys.aux}` **探测到文件** →
`\read` 回读首行（转录首行 `BAD: old file …` 即回读内容与 `\today` 的对比输出）。
最小用例（流 15 文件落盘）入单测 `write15_after_openout15_lands_in_file`。

### 9.2 A3 `\input` 缺文件错误语义 —— 已对齐

§8.2 已纠正「静默跳过」为「硬错误终止」；本轮把消息与终止形态对齐 tex.web
`prompt_file_name`（s="input file name"）+ `fatal_error`：

```
! I can't find file `nope.tex'.
l.N <当前行>
Please type another input file name
! Emergency stop.
*** (job aborted, file error in nonstop mode)
```

引擎无交互层（无法交互询问替代文件名），故一律取 `interaction < scroll_mode`
的致命分支：报错进转录后 `run()` 返回 `Err` 终止。守卫型探测（`\IfFileExists`/
`\@input`）走 `\openin`，不经此路径（latex.ltx L9889 `\@input` 缺文件 =
`\typeout{No file …}`，非错误）。

### 9.3 阻塞点刷新（含新发现的下一阻塞点）

| # | 状态 | 说明 |
|---|---|---|
| A2 | **已修复** | 写流按 tex.web 分发、流 15 可作文件、未打开流 `\immediate` 写进转录；`latex.ltx` L160-215 探测区走通 |
| A3 | **已修复** | `! I can't find file …` + 致命终止（引擎无交互层的等价形态） |
| A1 / A4 | 已修复（上轮） | — |
| **新（next）** | **L488 `\newbox\voidb@x` 失败** | `\voidb@x` 未定义 → `\strutbox` 等级联（转录 `! Undefined control sequence \voidb@x` + 一串 `Missing number`）。根因指向 `\e@alloc` 的 `\global#2#6\allocationnumber`，其中 `#2` 是 `\ifnum…\expandafter\chardef\else…\fi`——**`\global` 后接条件式赋值目标需要展开层支持**（TeX 在 `\global` 后展开 token 直至遇到真赋值原语）。位于 expl3（L1120）**之前**，故本刀未到达 "expl3 缺失" 报错 |
| 新（噪声） | `\today` 渲染带空格 | 转录 `BAD: old file 2026/09/04: 02:32 (should be  2026/09/04: 02:32)`——回读内容与 `\today` 差在空格（`\two@digits`/`\number` 展开），非文件通道问题；后果仅 `\@currdir` 取 `\@empty` 而非 `./`（非致命） |
| 新（偏差） | `\write` 中的 `^^J` | latex.ltx L126 `\newlinechar`^^J``；引擎行模型 LF=5（§8.1 偏差）把 `^^J` 在**扫描期**归并为空格 token → 写出空格而非换行。影响 `.aux` 内容保真与 `\typeout` 换行；修复属行模型（A1 延伸） |
| 新（偏差） | 文件名终止空格 | tex.web `scan_file_name` **消费**名字后的空格；引擎 `unread` 回输入流 → 泄漏空格 token（非致命） |

### 9.4 有意的简化（记录在案）

- `expand_to_string` 只取 cat 10/11/12 字符：tex.web `token_show` 对**所有**字符
  token（含组字符 `{`/`}`）都印其字符。该函数被 `\message`/`\show`/`\special`
  共享，为不扰动既有转录对齐而保持原状。
- 非 `\immediate` 写到未打开/越界流 → 丢弃（tex.web 会在 shipout 的 `write_out`
  写出，但 leaders 内的 whatsit 被跳过——`if not doing_leaders`）。引擎 flush
  不感知 leaders（whatsit 节点只存文本），若入队会把 TRIP L137
  `\write111{\help}`（leaders 内、`\help` 未定义）展开 → 运行中止。
- `\write18`（shell）仍拒绝（RFC-3）。

### 9.5 本轮改动清单与回归

- `crates/ntex-core/src/expand/io.rs`：`exec_write` 流分发重排、`open_if_needed`
  （截断语义）、`exec_openout`/`exec_closeout`、`scan_file_name` 引号、
  `exec_input` 致命错误块、`\newwrite` 0..=15、移除幽灵流槽。
- `crates/ntex-core/src/expand/mod.rs`：`WriteStream.created`。
- `crates/ntex-core/src/expand/tests.rs`：新增 9 个用例（流 15 文件、截断、
  空文件、`\write16`/`\write17`/负流号、引号文件名、缺文件转录块）；2 个既有
  用例改用 `\immediate`（tex.web：非 immediate 的 `\write16` 属延迟写，无
  shipout 不落转录）。
- `crates/ntex-test-support/examples/latex_probe.rs`：`SurveyVfs` 路径归一。
- 回归：`make check`（fmt/clippy/test）全绿；`cargo run -p ntex-trip -- --driver
  ntex` 仍停在 HEAD 已知的数学组错误（`group_end 无配对 group_begin`），无写
  通道相关新错误。

---

## 10. 2026-09-04 第四轮进展（`\global` 前缀链打通——L488 分配区越过）

> 本刀任务假设"`\global` 后接条件式赋值目标"是引擎缺口。**实测推翻**：主循环
> 天然支持前缀后隔可展开序列（`\global` 只置 `global_pending` 标志，`\ifnum`/
> `\expandafter`/宏展开由主循环逐 token 处理，标志持续到真赋值原语消费）。
> **真实根因在别处**：`\expandafter` 的第二 token 是 `\else`/`\or` 时，推回输入
> 的第一 token 被惰性跳过区吞掉。

### 10.1 真实根因：`\expandafter` 前置 token 被惰性跳过区吞掉

tex.web 有**两条**"跳过分支"语义，本引擎只实现了惰性那条：

| tex.web | 语义 | 引擎现状 |
|---|---|---|
| `expand()` 的 `fi_or_else` | `while cur_chr<>fi_code do pass_text; pop`——分支**就地消费**到配对 `\fi` 后立刻弹帧 | `\else`/`\or` 只把帧转 `Skipping`，分支 token 由主循环逐 token 丢弃（`process_one` 的 `is_skipping`）、`\fi` 到来才弹帧——**惰性** |
| `conditional()` 的 false 分支 | 立即 `pass_text` 到 `\else`/`\fi` | 已实现（`skip_ahead`，急切）✓ |
| 主循环逐 token 路径 | — | 惰性与急切**等价**（token 顺序不变、无推回）✓ |

惰性实现只在"**跳过区开着的同一时刻有 token 被推回输入**"时出错——即
`\expandafter`（tex.web：`get_token; get_token; if cur_cmd>max_command then expand;
back_input; cur_tok:=t; back_input`）。展开 `\else` 时分支就地消费完毕、帧弹出
**之后**才放回 `t`；而引擎放回 `t` 时帧仍 `Skipping`，主循环随即将 `t` 丢弃。

`latex.ltx` L488 `\newbox\voidb@x` 的展开链正是此形态（`ltplain.dtx` `\e@alloc`）：

```tex
\global\ifnum\allocationnumber<\@cclvi
       \expandafter\chardef   \else   % ← \expandafter 的第二 token 是 \else
       \expandafter\e@alloc@chardef\fi
  \voidb@x\allocationnumber           % ← 赋值目标随后
```

引擎行为：`\chardef` 被吞 → `\voidb@x` 在主循环当未定义 cs 报错 →
`\allocationnumber`（`\countdef`'d）被当赋值目标执行 → 数量扫描撞上 `\wlog`
的 `\immediate` → `! Missing number`。转录即 §9.3 所记 L488 级联。

**修复**（`expand/cond.rs` 新增 `drain_open_skip(depth)`；`expand/expr.rs` 的
`exec_expandafter` 与 `expand_once` 的 Expandafter 臂调用）：

- 推进条件机后，若刚进入 `Skipping` 的帧（`\else`/`\or` 转换的栈顶帧，或
  `\if*`/`\ifcase` 新压帧）仍在跳过，就**就地消费**分支 token（不展开、不执行、
  不报错——`pass_text` 语义），直到该帧被 `\fi` 弹出或 `\ifcase` 的 `\or`
  选中分支（回到 `Processing`，选中分支必须保持活）。
- 嵌套 `\if*` 经条件机压惰性 Skipping 帧（等价 `pass_text` 的 `incr(l)`）；
  本层 `\else`/`\or` 继续跳（`while cur_chr<>fi_code`）。
- 输入耗尽不报错：帧保持 `Skipping`，交给 `\end`/EOF 的 `Incomplete \if` 收口
  （与主循环惰性路径一致）。
- **主循环的惰性跳过保持原样**（`\tracingcommands` 对 `\fi` 的追踪行为不变，
  TRIP/ETRIP 转录零回归），仅展开上下文改急切。

### 10.2 `\global` 前缀语义核查（对照 tex.web `prefixed_command`）

前缀组合全部按主循环语义工作（新增单测锁定）：

| 用例 | 结果 |
|---|---|
| `\global\ifnum1=1\chardef\x=3\else\chardef\x=4\fi` | ✓ x=3（else 分支同样 ✓）|
| `\global\expandafter\chardef\csname y\endcsname=5` | ✓ y=5 |
| `\global\relax\chardef\r=9`（tex.web 前缀循环跳 `\relax`）| ✓ |
| `\global\z`（`\z` 展开为 `\chardef\q=7`：标志不随宏展开丢失）| ✓ |
| `\global\ifnum…` 组内赋值 → 组外可见（真全局）| ✓ |

**顺带补齐**：`\globaldefs`（tex.web `prefixed_command` 的 `Adjust for \globaldefs`）
此前只注册不生效——`is_global()` 现按 tex.web 语义处理（`>0` 所有赋值隐式全局、
`<0` 取消显式 `\global`）。该函数是全部 26 处赋值路径的唯一作用域收口点，
latex.ltx L12804/L13103/L13168 的 `\globaldefs\@ne` 后续需要它。

**未做（记录）**：tex.web 对"前缀后接不可加前缀命令"的报错
`! You can't use a prefix with `\unskip'.`（TRIP L345）引擎仍未实现；当前
`\global\unskip` 会把 `global_pending` **泄漏**给下一个赋值。需要"可加前缀命令"
分类表（`max_non_prefixed_command` 语义），本刀未动。

### 10.3 阻塞点刷新（第四轮）

| # | 状态 | 说明 |
|---|---|---|
| L488 分配区 | **已越过** | `\newbox\voidb@x` 正常：转录 `\voidb@x =\box 10`（真实 TeX 同款 `\wlog` 行）；错误总数 2576 → 2564 |
| **新（next）** | **L532 `\boxmaxdepth=\maxdimen`** | `scan_dimen_inner` 认 `\skipdef`/`\muskipdef`'d cs 作尺寸值，**不认 `\dimendef`'d cs**（tex.web `scan_dimen` 的 `<internal dimen>` 分支：`cur_cmd=assign_dimen` → 直接取 `eqtb[].sc`）。最小复现：`\dimendef\m=10 \m=100pt \hsize=\m` → `! Missing number, treated as zero. <to be read again> \m`。**级联**：报错后 `\m` 被放回输入 → 主循环把它当赋值目标执行 → 吞掉后续 `=<值>` 并改写 `\m` 自身 → L532–L547 连锁 6 条 `Missing number` + 1 条 `Missing {`（后者插入的 `{` 使组嵌套偏移，是 L16789+ 症状的疑似源头）。**属扫描器语义，本刀禁改**——修复面极小（在 `scan_dimen_inner` 补一个 `RegKind::Dimen` 臂，与既有 Skip/Muskip 臂同构），下一刀首选 |
| 下游症状 | 未动 | L16789 `! Too many }'s.`、L16792+ `\@namedef`/`\newif` undefined（定义被组回滚的形态）、L17474 `\@inmatherr` 级联 2492 条（`dumped=false`）——待 L532 修复后重测，判断是否随之消失 |
| `^^J` / `\today` | 复核仍在 | L301/L302 `\string^^J` → `\` + undefined（LF=5 行模型，§9.3）；转录头 `BAD: old file … (should be  2026/09/04…)` 双空格（`\today` 展开差空格 → `\@currdir` 取 `.`）。均非致命，本刀未处理 |

### 10.4 本轮改动清单

- `crates/ntex-core/src/expand/cond.rs`：新增 `drain_open_skip(depth)`（展开上下文的
  急切分支消费；`pass_text` 语义）。
- `crates/ntex-core/src/expand/expr.rs`：`exec_expandafter` 与 `expand_once` 的
  Expandafter 臂在推进条件机后调用它（`\fi` 不需要：立即弹帧、无分支滞留）。
- `crates/ntex-core/src/expand/save.rs`：`is_global()` 补 `\globaldefs` 语义。
- `crates/ntex-core/src/expand/tests.rs`：新增 4 个用例（条件选赋值目标 ×2、
  可展开链、`\expandafter`+`\else` 的 t1 保全（主循环 / `\edef` / 嵌套条件）、
  `\globaldefs` 正负两向）。
- 回归：`make check`（fmt/clippy/test）全绿；`cargo run -p ntex-trip -- --driver
  ntex`（trip + etrip 两路）在 HEAD 与本刀下输出**逐字节一致**（仅 systemd unit
  名/临时目录名/耗时内存行不同），仍停在 HEAD 已知的数学组错误
  （`group_end 无配对 group_begin`）。

## 11. 2026-09-04 第五轮进展（`<internal dimen>` 补臂——L532 区越过）

> 本刀按 §10.3 的"下一刀首选"执行：`scan_dimen_inner` 补 dimendef'd cs 作尺寸值的臂。
> 约束：与既有 Skip/Muskip 臂同构，只动 `scan_dimen` 的 cs 值分支。

### 11.1 scan_dimen 补 `<internal dimen>` 臂 —— 已修复

- tex.web `scan_dimen` 开头：`cur_cmd∈[min_internal,max_internal]` →
  `scan_something_internal(dimen_val,false)`；返回 `cur_val_level=dimen_val` 时
  `goto attach_sign`——值**就是**尺寸、不再扫单位；前置 `-` 号与负寄存器值由
  `if cur_val<0 → negative:=not negative; negate(cur_val)` 与 `attach_sign` 合成。
- NTex 此前**值位置**只认 `\dimen<n>`/`\skip<n>` 原语与 skipdef/muskipdef'd cs；
  dimendef'd cs 落到数字扫描 → `Missing number`。factor 位置的
  `<factor><internal dimen>` 臂（`11\mydimen`）此前已有，未动。
- 修复：`scan_dimen_inner` 值链补 `EqSlot::Register(RegKind::Dimen, idx)` 臂
  （scan.rs，位于 Skip 臂前、三个 RegKind 臂按枚举序排列），与既有臂同构：
  消费 cs → 读 `registers.dimen(idx)` → 前置 `-` 取负 → 直接返回（无单位）。
- **顺带复核**：Skip/Muskip 臂完整——胶水寄存器取 `.width` 分量，即 tex.web
  `<Coerce glue to a dimension>`（`cur_val_level≥glue_val → v:=width(cur_val)`），
  无需改动。
- 最小复现通过：`\dimendef\m=10 \m=100pt \hsize=\m` → 100.0pt（新单测
  `dimendef_cs_as_dimen_value` 三例：直接作值 / 负值+前置 `-` 合成 / factor 臂回归）。

### 11.2 阻塞点刷新（第五轮）

| # | 状态 | 说明 |
|---|---|---|
| L532 分配区 | **已越过** | `\maxdimen =\dimen 10`、`\hideskip =\skip 10`、`\p@ =\dimen 11`、`\z@ =\dimen 12`、`\z@skip =\skip 11` 照常 `\wlog`；**L532–L547 连锁 6 条 `Missing number` + 1 条 `Missing {` 全部消失**，L532–L726 区间零错误。错误总数 2564 → 2561 |
| **新（next）** | **L727 `\everyjob\expandafter{\the\everyjob\the\LaTeXReleaseInfo}`** | toks 参数赋值要求**字面 `{`**：引擎 `\everyjob` 走 `expect_equals()` + `scan_group_contents()`（`\everyjob` 等 toks 参数的值扫描不展开 filler）；tex.web `scan_left_brace`（L8194–8201）用 **`get_x_token`**（可展开 filler）并跳 `spacer`/`\relax`——`\expandafter` 在 filler 位置被展开、`{` 由它压回。孤立复现：`\everyjob\expandafter{\the\everyjob\the\toks0}` → `! Missing { inserted.`（l.3 报行正确）。**同病因子集**：凡 `scan_left_brace` 语义处（toks 参数、`\setbox`…）。修法：值扫描前用 x-token 语义找 `{`（跳 spacer/\relax、可展开 token 展开），属"必选 `{`"通用助手，非 `\everyjob` 专属 |
| 错误行号偏差 | 记录 | latex.ltx 中该错误报 `l.547`（陈旧行号；孤立复现报行正确）——错误锚点在多帧输入下取到旧 source pos，独立小问题，本刀未动 |
| 下游症状 | 归因收敛 | L16789 `! Too many }'s.` **未消失**，但其唯一上游已收敛为 L727 的 `Missing { inserted`：按语义插入左括号（`incr(align_state)`）+ 收组越过源行 → 组嵌套自此偏移 1。L16792+ `\@namedef`/`\newif`/`\setlength`/`\NewHookWithArguments` undefined、L17048/17051/17338 `Missing font identifier`（`\fontdimen8\tenln`）、`\bezier`/`\@bezier` 20 条、L17474 `\@inmatherr` 2492 条——**待下一刀修 scan_left_brace 后重测**，判断是否随之消失 |
| `^^J` / `\today` | 复核仍在 | L301/L302 `\string^^J` → `\` + undefined（LF=5 行模型，§9.3）；转录头 `BAD: old file …` 双空格。均非致命，本刀未处理 |
| 终态 | 未动 | pass1 终止于输入栈超限（5001>5000 防挂保护）、`dumped=false`，与第二/三轮记录一致（L17474 区） |

### 11.3 本轮改动清单

- `crates/ntex-core/src/expand/scan.rs`：`scan_dimen_inner` 值链补
  `RegKind::Dimen` 臂（`<internal dimen>`，注释含 tex.web 对照）。
- `crates/ntex-core/src/expand/tests.rs`：新增 `dimendef_cs_as_dimen_value`
  （3 例：latex.ltx L532 复现 / 负值合成 / factor 臂回归）。
- 回归：`make check`（fmt/clippy/test，26 个套件）全绿；TRIP/ETRIP driver
  （`cargo run -p ntex-trip -- --driver ntex`）两路均停在 **HEAD 已知**的
  数学组残留（TRIP pass1 `group_end 无配对 group_begin`、ETRIP pass2
  `l.356` math left group 区），无新增 diff——本刀未跑 HEAD 逐字节对比
  （禁 stash），以"残留签名一致 + 全量测试绿"为门禁依据。

## 12. 2026-09-04 第六轮进展（`scan_left_brace` filler 语义——L727 区越过）

> 本刀按 §11.2 的"下一刀首选"执行：给 toks/寄存器/`\every*` 等值扫描补
> tex.web `scan_left_brace` 的 filler 语义（`get_x_token` 可展开 filler 展开、
> 跳 spacer/`\relax`）。只影响**值扫描路径**；宏定界参数扫描零改动。

### 12.1 scan_left_brace filler 语义 —— 已修复

- tex.web `scan_left_brace`（L8194-8206）实为 `@<Get the next non-blank
  non-relax non-call token@>`（L8208-8210）：`repeat get_x_token until
  (cur_cmd<>spacer) and (cur_cmd<>relax)`。引擎此前 toks 值扫描要求**字面 `{`**
  （`skip_spaces` + fetch + catcode 检查），latex.ltx L727
  `\everyjob\expandafter{\the\everyjob\the\LaTeXReleaseInfo}` 的 `\expandafter`
  在 filler 位置不被展开 → `Missing { inserted`；恢复插入的 `{` 使组嵌套偏移 1，
  是 §11.2 记录的 L16789 `Too many }'s` + L16792+ 数千条 undefined 的上游。
- 修复（scan.rs 重构）：
  - 新增 `fetch_non_filler`：filler 循环。spacer（cat 10）与 `\relax`（含
    `\let` 链别名）跳过；可展开 token（`\expandafter`/宏/`\the`/`\csname`…）经
    `expand_once` 展开后压回输入栈顶重判（`\expandafter` 把 `{` 压回的语义）；
    `\let\bgroup={` 类组定界别名经 `resolve_group_char` 归一；被 `\noexpand`
    冻结的 token（fetch 返回 `ne=true`）本轮**不展开**但照做 spacer/`\relax`
    判定——TeX 一次性闩锁，否则 `expand_once` 会把冻结 token 原样压回造成
    无限重取（挂死类缺陷）。
  - 新增 `scan_left_brace`：`fetch_non_filler` 取 token，非 `{` → "Missing {
    inserted." 恢复（token 放回 + 隐含 `{`，tex.web `incr(align_state)`）；
    `scan_group_contents` 入口改调它，`\toks`/`\every*`/`\output`/`\message`/
    `\mark`/`\insert`/`\discretionary`/`\special`/`\mathchoice`/`\patterns` 等
    既有"必选 `{`"值扫描一次收敛。
  - `scan_toks_rhs`（toks 寄存器 RHS）与 `\everydisplay`/`\everypar` 等
    （primitive_toks_state.rs）RHS 取 token 改走 `fetch_non_filler`；
    `\the\everyjob` 读回补上（save.rs，映射 toks 0，与既有赋值映射同源）。
- **宏定界参数扫描零改动**：macros.rs 的 `collect_delimited_arg`/
  `collect_undelimited_arg`/`scan_balanced_text` 不经过上述函数（grep 核实），
  TRIP/ETRIP 零回归。
- 修正中间态两个 bug（先审查后收尾发现）：
  1. `scan_mathchoice_branch` 先 `scan_left_brace()`、又经 `scan_group_contents`
     入口再调一次 → 双重消费：空分支 `{}` 的 `}` 被当开组 token → 伪报一次
     "Missing { inserted."（TRIP L438 回归面）——删显式调用，只留
     `scan_group_contents`。
  2. clippy `never_loop`：relax-skip 移入 `fetch_non_filler` 后 `scan_toks_rhs`
     的 `loop` 已无 `continue`，去包装（单遍）。

### 12.2 阻塞点刷新（第六轮）

| # | 状态 | 说明 |
|---|---|---|
| L727 区 | **已越过** | `\everyjob\expandafter{\the\everyjob\the\LaTeXReleaseInfo}` 正常赋值；后续 `\LaTeXReleaseInfo =\toks 10`（寄存器 wlog）与 L733 `\write16` 的 `LaTeX2e <2026-06-01>` 照常出现。L727 插入 `{` 的组偏移级联（L16789 `Too many }'s`、L16792+ undefined、L17474 `\@inmatherr` 2492 条）**全部消失**。错误总数 2561 → **4**，load 提前终止于 L1113-1117 区（见下），`dumped=false` |
| **新（next）** | **L1113 ltexpl.dtx expl3 门闩：`\expandafter\ifx\csname tex\string _let:D\endcsname\relax`** | 判别"expl3 原语是否已存在"。tex.web：`\csname` 对未定义名 `eq_define(cur_cs,relax,256)`（L7754，注释明言 "will now match `\.{\relax}'"），原语 `\relax` 同为 `primitive("relax",relax,256)`（L5728）——二者在 `\ifx` 下相等，门闩走"未定义 → 加载 expl3"分支。引擎把 `\csname`-未定义与直接-未定义都归 `EqSlot::Undefined`，`ifx_equal` 判 `Undefined≠Primitive(Relax)` → 门闩误判"expl3 已存在"→ 走 `\else` 执行 `\GenericInfo{}{Skipping…}`（其定义在 L8841，未达 → 报 undefined）再 `\expandafter\endinput` → **load 在 L1117 早退**，`\dump` 未达。修法：`\csname` 对未定义名创建为 `EqSlot::Primitive(Relax)`（对齐 `eq_define(cs,relax,256)`），或 `ifx_equal` 特判 Undefined≡Relax；属 `\ifx`/`\csname` 语义刀 |
| L728 嵌套条件 | 偏差（报错但恢复正确） | `\ifnum0\ifnum\patch@level=0\ifx…1\fi\fi>0`：外层 `\ifnum` 操作数读到 `0` 后遇嵌套 `\ifnum` 不展开（`scan_number_inner` 十进制数字循环缺 `maybe_eval_cond` 臂——基数分支同款已有，cond 值读进数字）→ 报 "Missing = inserted for \ifnum"，但恢复后**仍走对分支**（L733 写流正确）。小修（数字循环补条件臂）可与上式同刀或后刀 |
| 下游症状 | 归因收敛 | 转录余 4 错：L301/302 `\string^^J` → `\`+undefined（LF=5 行模型，§9.3 已知）、L728 嵌套条件（上）、`\GenericInfo` undefined（L1113 门闩下游，非独立阻塞点）。错误锚点 l.547 陈旧（§11.2 已知偏差） |
| 终态 | 未动 | pass1 OK、`dumped=false`（L1117 `\endinput` 早退，**非**输入栈超限——首度未触 5000 帧防挂保护）；待 expl3 门闩修复后重测 |

### 12.3 本轮改动清单

- `crates/ntex-core/src/expand/scan.rs`：`fetch_non_filler`/`scan_left_brace`/
  `push_expansion` 新增；`scan_group_contents`/`scan_toks_rhs`/
  `scan_mathchoice_branch` 入口改走 `scan_left_brace`（含 mathchoice 双重消费
  修正、scan_toks_rhs 去 `loop`）。注释含 tex.web 行号对照与两处记录偏差
  （条件原语不在 `is_expandable()` 白名单、`\noexpand` 一次性闩锁）。
- `crates/ntex-core/src/expand/primitive_toks_state.rs`：`\everydisplay`/
  `\everypar` 等 toks RHS 取 token 改 `fetch_non_filler`（tex.web assign_toks
  L22951 对照）。
- `crates/ntex-core/src/expand/save.rs`：`\the\everyjob` 读回臂（映射 toks 0，
  与既有赋值映射同源）。
- `crates/ntex-core/src/expand/tests.rs`：新增 `scan_left_brace_expandable_filler`
  （5 例：`\expandafter` filler / `\bgroup` 别名组定界 / spacer 与 `\relax` 跳过 /
  toks 寄存器 RHS 复制不受影响 / 非 `{` 报 "Missing { inserted." 恢复）。
- 回归：`make check`（fmt/clippy/test，全套件含 277 lib 单测）全绿；TRIP/ETRIP
  driver 停在 HEAD 已知数学组残留（未跑逐字节对比，禁 stash），以"残留签名
  一致 + 全量测试绿"为门禁依据。
