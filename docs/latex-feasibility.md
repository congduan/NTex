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

## 13. 2026-09-04 第七轮进展（`\csname` 未定义名 = `\relax` 语义——越过 L1113 expl3 门闩）

> 本刀按 §12.2 的"下一刀首选"执行：给 `\csname` 对**未定义名**的制造语义补
> tex.web `eq_define(cur_cs,relax,256)`（L7753-7754）——未定义名经 `\csname`
> 制造后与 `\relax` 原语同义，`\ifx` 相等。这是 eqtb 槽模型的核心语义
> （动态名构造），latex.ltx L1113 expl3 门闩正依赖它。

### 13.1 `\csname` 未定义名 → relax —— 已修复

- tex.web `@<Manufacture a control...@>`（L7744-7757）：`\csname<name>\endcsname`
  名字查表后 `if eq_type(cur_cs)=undefined_cs then eq_define(cur_cs,relax,256)`
  （N.B. save_stack 可能变——即局部定义，TeX 2.9 版本注记 "made ... relax
  local"）。原语 `\relax` 同是 relax/256（L5728），故制造产物与 `\relax` 可
  `\ifx` 相等。
- 引擎此前把 `\csname`-未定义与直接-未定义都归 `EqSlot::Undefined`，`ifx_equal`
  判 `Undefined ≠ Primitive(Relax)` → L1113 门闩误走 `\else`（`\GenericInfo`
  undefined + `\expandafter\endinput` L1117 早退）。
- 修复（expr.rs）：新增 `csname_define_relax(csid)`，在**两个制造点**调用——
  ① `exec_csname`（主循环可展开分发路径，dispatch_expandable→exec_csname）；
  ② `expand_once` 的 `Csname` 分支（`\edef`/`\write`/`\expandafter` 展开上下文，
  expr.rs:188）。`scan_csname` 本身不加（`\ifcsname` 与它共享，e-TeX 语义
  `\ifcsname` 不得制造定义）。
  - 只在槽为 `Undefined` 时制造（已定义——宏/原语/`\let`——不重定义，TeX 同）；
  - 组内局部：`group_level>0` 时 push `save_stack`（prev=Undefined），组末恢复
    未定义（TeX 2.9 "relax local"，与 `\def` 同走 save_stack）；
  - **不**走 `set_slot_scoped`：csname 制造发生在 expand() 中、非赋值命令，
    不消费 `\global` 前缀、不触发 `\afterassignment`、不受 `\globaldefs` 影响。
- 记录偏差（注释内已记）：
  1. 引擎存 `EqSlot::Primitive(Relax)`（与 `\relax` 原语同槽）。`\ifx`/执行/
     `\ifdefined` 与 TeX 一致；唯 e-TeX `\ifprimitive` 对制造产物误判为真
     （TeX 中它是 eq_define 产物、非原语）。
  2. `\meaning` 对 Primitive 槽按**当前 cs 名**显示 `\名`（meaning_text 既有
     约定，primitive.rs:415）——制造出的 relax 显示 `\nope` 而非 TeX 的
     "relax"；`\relax` 原语因名恰为 relax 表面一致。非本刀引入，留 \meaning 刀。
- **不动**直接使用未定义 cs 的路径：`\undefinedcs` 主循环仍报 "Undefined
  control sequence." 并当 relax 继续（TeX 错误恢复一致）；`\ifx\undefined\relax`
  仍为假（TeX：undefined_cs ≠ relax 的 eq_type）。只改 `\csname` 制造产物。

### 13.2 验证（单测 + make check）

- 新单测 `csname_undefined_becomes_relax`（tests.rs，6 断言）：
  1. `\expandafter\ifx\csname nope\endcsname\relax T\else F\fi` → T（L1113 门闩
     最小形态；注意需 `\expandafter`——`\ifx` 操作数不展开，裸 `\ifx\csname…`
     比较的是 `\csname` 原语自身，TeX/引擎同）；
  2. `\csname nopecs\endcsname\ifdefined\nopecs yes\else no\fi` → yes（制造持久、
     执行 no-op 不报错；名须全字母——`\nope2` 在正文切 `\nope`+`2`）；
  3. `\csname nope2\endcsname` 带数字名只能经 `\csname` 再引用 → T（TeX 同）；
  4. `\edef\a{\csname qqq\endcsname}\expandafter\ifx\a\relax T\else F\fi` → T
     （expand_once 路径同样制造）；
  5. `{\csname grpname\endcsname}\ifdefined\grpname` → no（TeX 2.9 局部：组末恢复）；
  6. 制造后 `\meaning` 不再含 "undefined"；直接未定义 `\ifdefined\direct…` → no。
- `make check` 全绿（fmt / clippy -D warnings / `cargo test --workspace`，含 278
  lib 单测）。TRIP/ETRIP driver 停在 HEAD 已知数学组残留（未逐字节对比），以
  "残留签名一致 + 全量测试绿"为门禁依据。

### 13.3 latex.ltx --initex 实测（L1113 门闩越过，进入 expl3 加载区）

- **门闩越过**：转录不再出现 `\GenericInfo … Skipping` + L1117 `\endinput` 早退；
  加载推进到 L1146-1148 `\IfFileExists{expl3.ltx}` 区。
- 无 expl3.ltx（survey 目录原状）→ `\IfFileExists` 假分支 `\errmessage{LaTeX
  requires expl3}` → `\batchmode\read -1`（终端读）→ 引擎 `\read` 流未打开硬错，
  pass1 **ERROR**（此前 L1117 早退是 OK+dumped=false——早退≠推进）。
- 取 l3kernel（tlnet `l3kernel.tar.xz` → expl3.ltx/expl3-code.tex/…，拷入
  survey base 目录）后：**expl3.ltx 真正载入** → `\input expl3-code.tex` →
  expl3 kernel bootstrap 区，最终以 `非法输入：\advance 目标必须是寄存器或内部
  参数` 硬错终止（dumped=false）。

### 13.4 阻塞点刷新（第七轮）

| # | 状态 | 说明 |
|---|---|---|
| L1113 区 | **已越过** | expl3 门闩 `\expandafter\ifx\csname tex_let:D\endcsname\relax` 走对分支；`\csname`-未定义=relax 语义落地（§13.1） |
| L1146-1148 | **已越过** | `\IfFileExists{expl3.ltx}` 真分支 + `{\input expl3.ltx }` 执行 |
| **新（next）** | **L1147 engine-check `\ifnum0\ifdefined\pdffilesize 1\fi\ifdefined…>0`** | 即 §12.2 记录的 L728 嵌套条件偏差族（scan_number_inner 十进制数字循环缺 maybe_eval_cond 臂，条件在 `\ifnum` 数字内不被展开求值）→ 报 "Missing = inserted for \ifnum" + 恢复吞 token；latex.ltx L1147 与 expl3-code.tex 早期（l.192 等）反复出现。expl3 全篇用 `\ifnum0\ifdefined…\fi…=11`/`>0` 惯用法，此偏差从"报错但走对分支"升级为 expl3 加载的首个真实阻塞。**修法：数字扫描循环补条件臂（基数分支同款已有）** |
| expl3 bootstrap | 级联症状 | `\__kernel_primitive:NN` 原语别名层（expl3-code.tex l.276 起：`\let\tex_global:D\global`… + 数百行 `\__kernel_primitive:NN \ifeof \tex_ifeof:D`）在条件栈/组平衡被 13.4 上项污染后报 "Extra \else/\fi、Argument of \__kernel_primitive:NN has an extra }、Too many }'s、\tex_…:D undefined" 并级联，终以 `\advance` 硬错终止——多数是上项恢复污染的次生症状，待上项修复后重测归因 |
| pdfTeX 引擎墙 | 语义事实（非本轮修） | latex.ltx L1147 engine-check 测 `\pdffilesize`/`\filesize`/`\luatexversion`/`\kanjiskip`——真实 TeX 语义下 NTex（无这些原语）会被 `\errmessage{LaTeX requires the e-TeX primitives…pdfTeX/XeTeX/LuaTeX/e-upTeX…}` 拒绝；本轮因 13.4 嵌套条件误判**碰巧放行**进到 expl3。要合法构建 latex.fmt，需决定"伪装 pdftex（实现探测原语）"或另设引擎开关 |
| 终态 | 未动 | pass1 ERROR（`\advance` 硬错）、dumped=false、无栈超限；下刀首选 = scan_number_inner 条件臂（§13.4 新 next） |

### 13.5 本轮改动清单

- `crates/ntex-core/src/expand/expr.rs`：`csname_define_relax` 新增；
  `exec_csname` 与 `expand_once` 的 Csname 分支制造前调用。注释含 tex.web
  行号对照与两处记录偏差（\ifprimitive / \meaning）。
- `crates/ntex-core/src/expand/tests.rs`：新增 `csname_undefined_becomes_relax`
  （6 断言，见 §13.2）。
- 回归：`make check` 全绿；TRIP/ETRIP 门禁以残留签名一致 + 全量测试绿为依据。
- 环境（不入库）：/tmp/latexsurvey 增 l3kernel 文件（expl3.ltx/expl3-code.tex
  等 29 个拷入 base 目录），供后续 expl3 加载实测。

## 14. 2026-09-04 第八轮进展（scan_number 内嵌套条件 + 注释行状态——engine-check 按真实语义求值）

> 本刀按 §13.4"下一刀首选"执行：给 `scan_number_inner` 十进制数字循环补条件臂
> （tex.web scan_int 的 get_x_token 对 if_test 与 fi_or_else 一律 expand 的就地
> 求值语义）。修完单测后发现 latex.ltx L1122 仍报 "Missing = inserted"——根因
> 不在 scan_number 而在**注释行后行首状态**（`\ifnum0%` 后换行缩进的空格被当行中
> 空格产出 token、提前终止 `0`），补 input.rs 注释吞行状态重置后方按真实 TeX
> 语义求值。

### 14.1 `scan_number_inner` 十进制数字循环条件臂 —— 已修复

- 数字循环 `None` 臂补条件处理：遇条件 token 一律步进条件机（`step_conditional`）。
  - If\* 开始原语（`\ifdefined`/`\ifnum`/`\ifx`/…）就地求值：真 → 产 1 继续累计
    （expl3 `\ifnum0\ifdefined X 1\fi...` 聚合）、假 → `skip_ahead` 跳过（无贡献）；
  - `\fi`/`\else`/`\or` 闭合**本数字扫描期间**开启的条件帧（`\ifdefined` 真分支的
    `1\fi`），必须步进后继续——放回会让外层 `scan_relation` 误报 "Missing = inserted
    for \ifnum"（第七轮 L1122/expl3 级联偏差的根因）；
  - 无帧可闭的游离 `\fi`/`\else`/`\or`（cond_stack 空）维持原放回语义
    （"Missing number" 恢复，行为不变）；
  - 真条件的 `\else` 死分支：`step_conditional` 把最内层帧翻 Skipping 后，循环顶部
    丢弃非条件 token（与主循环惰性跳过同款），`\fi` 弹帧后回到累计。
- **偏差注释**（与符号/基数循环不同）：符号/基数循环里 `\fi` 等属外层求值的未决
  终结符须放回（TRIP L82 `\ifnum'\ifnum10=10 12="\fi` 的 \fi 在 hex 循环里由外层
  skip_ahead 闭合）；十进制循环聚合语义要求继续。TRIP L82 走基数路径，不受扰。
- 语义确认（与真实 TeX 对齐的副作用）：`\count0=1\ifnum\count0=1`（无空格）在
  RHS 数字扫描中即就地求值 → 读到旧值 0 → **F**（真实 TeX 的知名陷阱：数字后须
  空格/`\relax` 分隔，赋值完成才轮到 `\ifnum`）；`\count0=1 \ifnum...`（有空格）
  → T。既有单测 `muskip_order_repro` 首条 src 原为无空格 raw 形式（编码旧延迟
  语义），按真实语义补空格修正（§14.4）。

### 14.2 input.rs 注释吞行后行首状态重置 —— 已修复（L1122 实际根因）

- latex.ltx L1122 `\ifnum0%` 后换行缩进再 `\ifdefined` 探针：引擎扫描器注释
  （cat 14）吞行后**状态不重置**，下一行行首缩进空格被当"行中空格"产出 token，
  数字循环在 `0` 后遇空格即终止 → 探针落空 → 恒报 "Missing = inserted"。单测
  证明 `\ifnum0%\n  \ifdefined\relax 1\fi...>0`（缩进形态）此前输出 F。
- tex.web get_next：行尾（注释 `@<Finish line,|goto switch|@>` 后 `loc>limit`）
  → `state:=new_line`（L7277），下一行**行首**空格在 new_line 状态被忽略
  （`new_line+spacer` 属"被忽略字符"，L7310）。修复：注释吞行（含行尾）后
  `*state = ScanState::LineStart`。
- 该修是本刀"越过 L1122"的**实际使能者**：重置后探针直接续接数字聚合。

### 14.3 验证（单测 + make check）

- 新单测 `nested_cond_in_number_scan`（10 断言）：`\ifnum0\ifdefined\relax 1\fi>0`
  真假两向、多探针聚合（两真 011=11、`=11` 精确比较）、`\else` 变体
  （`1\else 0\fi` 真假）、嵌套多层（真分支再套 \ifdefined）。
- 新单测 `nested_cond_in_number_scan_with_newline_indent`（2 断言）：latex.ltx
  L1122 实际缩进形态真/假两向。
- input.rs 新单测 `comment_resets_to_line_start_ignoring_next_indent`（2 断言）：
  `a%c\n   b` → 缩进空格不产出 token；注释行后空行仍 `\par`。
- 既有 `muskip_order_repro` 首条 src 补空格（见 §14.1 语义确认注释）。
- `make check` 全绿（fmt / clippy -D warnings / `cargo test --workspace`，ntex-core
  lib 281 单测）。TRIP/ETRIP driver 残留签名一致（6 条 wrong group/node 数学组
  生命周期 P0 残留，未逐字节对比），以"残留签名一致 + 全量测试绿"为门禁依据。

### 14.4 latex.ltx --initex 实测（engine-check 按真实语义求值 → pdfTeX 引擎墙）

- **L1122 engine-check 修正求值**：转录 "Missing = inserted for \ifnum" 从反复出现
  → **0 次**；错误总数从 round7 expl3 bootstrap 级联（~10+ 条）→ **4 条**
  （l.301/302 `\string^^J` LF=5 已知残留 2 条 + LaTeX 自身 `\errmessage` 1 条 +
  `\read -1` 的 "Bad number (-1)" 1 条）。
- 加载按真实 TeX 语义停在 **L1122 `\ifnum0\ifdefined\pdffilesize...` 引擎检查的
  \else 分支**：4 个探测原语（\pdffilesize/\filesize/\luatexversion/\kanjiskip）
  NTex 全未定义 → 聚合 0 >0 假 → `\errmessage{LaTeX requires the e-TeX
  primitives...pdfTeX/XeTeX/LuaTeX/e-(u)pTeX...}` + `\batchmode\read -1` → 引擎
  `\read` 流未打开硬错（pass1 ERROR、dumped=false）。**不再**误入 expl3 bootstrap。

### 14.5 阻塞点刷新（第八轮）

| # | 状态 | 说明 |
|---|---|---|
| L728/§12.2 嵌套条件偏差 | **已越过** | 十进制数字循环补条件臂（§14.1）；"Missing = inserted for \ifnum" 消除，expl3 `\ifnum0\ifdefined…1\fi` 聚合惯用法成立 |
| L1122 engine-check 嵌套条件误判 | **已越过** | 注释行状态重置使 `\ifnum0%`+缩进探针按真实 TeX 语义聚合（§14.2）；engine-check **如实求值** |
| **pdfTeX 引擎墙** | **新（next）** | latex.ltx L1122 engine-check 按真实语义现判 **FALSE** → LaTeX 自身 `\errmessage` 拒载（NTex 无 \pdffilesize/\filesize/\luatexversion/\kanjiskip 探测原语）。要合法构建 latex.fmt 须决定**伪装 pdftex**（实现探测原语使 \ifdefined 为真；expl3 加载后这些原语后续还会被调用——\pdffilesize 需真实现文件尺寸语义或至少 \relax 占位）或**另设引擎开关**。此决策从 round7 的"误判碰巧放行"变为本轮"正确拒载"后的**必经决策点** |
| 终态 | 未动 | pass1 ERROR（`\read` 流未打开——engine-check \else 分支的 `\batchmode\read -1`）、dumped=false、无栈超限；下刀首选 = pdfTeX 引擎墙决策（伪装 pdftex 探测原语） |

### 14.6 本轮改动清单

- `crates/ntex-core/src/expand/scan.rs`：`scan_number_inner` 十进制数字循环
  `None` 臂补条件步进 + 循环顶部 `is_skipping` 死分支丢弃。注释含 tex.web
  scan_int get_x_token 语义对照与三处记录偏差（\fi/\else/\or 无帧放回、与符号/
  基数循环差异、数字后无空格条件的真实 TeX 陷阱）。
- `crates/ntex-core/src/input.rs`：`Catcode::Comment` 吞行（含行尾）后
  `*state = ScanState::LineStart`（tex.web L7277/L7310 对照）。
- `crates/ntex-core/src/expand/tests.rs`：新增 `nested_cond_in_number_scan` /
  `nested_cond_in_number_scan_with_newline_indent`；`muskip_order_repro` 首条
  src 补空格。
- `crates/ntex-core/src/input.rs` tests：新增
  `comment_resets_to_line_start_ignoring_next_indent`。
- 回归：`make check` 全绿；TRIP/ETRIP 门禁以残留签名一致 + 全量测试绿为依据。

## 15. 2026-09-04 第九轮进展（pdfTeX 引擎伪装——越过 engine-check 引擎墙，进入 l3kernel bootstrap）

### 15.1 决策与最小探测集（主控决策：伪装 pdfTeX）

engine-check（latex.ltx L1122-1134）是 **OR 门**（`\ifnum0\ifdefined\pdffilesize 1\fi
\ifdefined\filesize 1\fi\ifdefined\luatexversion…\ifdefined\kanjiskip 1\fi >0`），
但 l3kernel 的引擎判定是**互斥拼接**（expl3-code.tex L7934 `\c_sys_engine_str` 依
`\cs_if_exist:NT \tex_<name>:D` 依次拼 hitex/luatex/pdftex/ptex·uptex/xetex）。
**结论：只注册 pdfTeX 一族**，其他引擎标记（\luatexversion/\kanjiskip/\filesize/
\XeTeXversion/\HINTversion）必须保持未定义——多引擎同定义会把引擎串拼成无法识别
的混合值（如 "luatexpdftex"），反而瘫掉后端选择。单测
`pdftex_probe_primitives_are_defined_and_engine_fence_holds` 固化该栅栏。

关键机制：l3kernel L278 起用 `\__kernel_primitive:NN <prim> \tex_<name>:D`
做 `\global\let` 无条件别名（L672-817），e-TeX 同名族在 L825-830/L898-907
**条件别名**（`\ifdefined#1` 才 let）。因此：
- `\tex_pdftexversion:D` 的"存在性"完全由 NTex 是否定义 `\pdftexversion` 决定
  （引擎判定自动跟随，NTex 侧无需自己提供 `\tex_:D` 别名）；
- `\pdfstrcmp → \tex_strcmp:D`（L5129 `\cs_new_eq:NN \__str_if_eq:nn`）与
  `\pdffilesize → \tex_filesize:D`（L12679 `\__file_size:n`）是**无条件别名且被
  expl3 全篇调用**——二者不做真实现则 expl3 字符串比较/文件名解析在首次调用时报
  "未定义控制序列"。这是"最小集合"的真正边界：不是 engine-check 的 4 探针，
  而是探针 + 两个被无条件别名的行为原语。

### 15.2 注册的原语（12 项，eqtb/primitive.rs → builtins.rs → 处理器全链路）

| 原语 | 语义 | 实现档次 |
|---|---|---|
| `\pdftexversion` | 只读整数 **140**（对齐 pdfTeX 1.40.x 世代；latex.ltx L22500 `\ifnum\pdftexversion=140` 证实值域） | 真实现（探测） |
| `\pdftexrevision` | 只读整数 **25**。**勘误**：任务简报猜"字符串 .200000"——latex.ltx L22501 `\ifnum\pdftexrevision<22` 证明是**整数**（1.40.**25** 的尾段） | 真实现（探测） |
| `\pdftexbanner` | 可展开字符串 `This is pdfTeX, Version 1.40.25 (NTex pdfTeX compatibility layer)`。**保留 NTex 标识**：banner 会进日志/文档元数据，伪装"原语面"不冒充产物 | 真实现（诚实标注） |
| `\pdfoutput` | misc 63 整数参数，**默认 0 = DVI 模式**（与 pdfTeX 默认一致，NTex 亦输出 DVI）。可赋值（expl3 `\c_sys_output_str`/graphics/hyperref 读写） | 真实现（值面）；非 0 值无 PDF 后端承接 = 已记录偏差（§15.4） |
| `\pdfshellescape` | 只读整数 0（无 shell escape；expl3 `\c_sys_shell_escape_int` 无条件读取） | 真实现（安全面） |
| `\pdfelapsedtime` | 只读整数 0（无计时器） | 真实现（安全面）；偏差已记录 |
| `\pdfrandomseed` | 只读整数（misc 64 种子状态） | 真实现（状态面） |
| `\pdfsetrandomseed` | `<number>` → 写 misc 64 | 真实现 |
| `\pdfuniformdeviate` | 可展开 `0 ≤ r < n`；确定性 LCG 推进种子（同种子同序列，可复现） | 真实现（确定性，非随机源——偏差已记录） |
| `\pdfstrcmp` | 可展开，两 token 串按 `\detokenize` 同规则转字节后字典序比较 → -1/0/1 | **真实现**（expl3 无条件别名） |
| `\pdffilesize` | 可展开，文件字节数；**缺失 → 空展开**（l3kernel `\file_full_name:n` L12696 以空返回判定"未找到"，pdfTeX 同语义）。文件读取走 VFS（`Vfs::read` 取长度，非 stat） | **真实现**（expl3 无条件别名） |
| `\pdfcreationdate` | 可展开 `D:YYYYMMDDHHMMSSZ'00'`，取自 \day/\month/\year/\time（`\time` 仅分钟精度 → 秒恒 00） | 真实现；秒精度偏差已记录 |

**未注册（占位决策）**：\pdfmdfivesum/\pdffiledump/\pdfsavepos/\pdfannot 等
行为族。expl3 只 `\let` 别名不调用，保持未定义使误用报"未定义控制序列"而非
静默空操作——比 relax 占位更符合"不静默错"约束。若后续加载路径真用到
（如 `\file_mdfive_hash:n`，latex.ltx L9978 只别名不调用），届时补真语义。

### 15.3 实测：latex.ltx --initex 越过 engine-check

- **拒载错误消失**：转录中不再有 "LaTeX requires the e-TeX primitives"；
  latex.ltx L1122 engine-check OR 门聚合 `01>0` → 真 → `\input expl3.ltx` 执行。
- expl3.ltx（199 行 loader）+ expl3-code.tex（40266 行）**真载入**：
  转录出现 `Package: expl3 2026-08-10 L3 programming layer (code)`（首次），
  `\c_sys_engine_str` 探测链已读到 pdftex 分支。
- 新停点：**expl3-code.tex L196**（l3kernel 自身的第二个引擎门闩）→ 详见 §15.5。
- 终态：pass1 ERROR（级联下游 `\advance 目标必须是寄存器`）、dumped=false。

### 15.4 新增单测与偏差清单

`crates/ntex-core/src/expand/tests.rs` 新增 10 测（`pdftex_*`/`pdf*_…`）：
探针存在性 + 引擎栅栏、version/revision 作数字操作数（latex.ltx L22500 原形）、
banner 字符串、\pdfoutput 默认 0/可赋值/DVI 判定、\pdfstrcmp 四象限 + `\edef`
展开、\pdffilesize 字节数 + 缺失空展开、\pdfuniformdeviate 值域 + 种子可读 +
同种子同序列、\pdfshellescape/\pdfelapsedtime 安全默认、\pdfcreationdate 格式、
engine-check OR 门原形（L1122 缩进形态）、\numexpr/\the 路径。

**已记录偏差**（均不静默）：\pdfelapsedtime 恒 0；\pdfuniformdeviate 确定性 LCG
（真 pdfTeX 为随机源）；\pdfcreationdate 秒恒 00；\pdfoutput 非 0 值无 PDF 后端；
banner 含 NTex 标识。\pdfoutput=1 不报错（ packages 常见赋值路径，报错会阻断
加载；效果偏差由 DVI 后端承接）。

### 15.5 阻塞点刷新（第九轮，下一刀）

| # | 状态 | 说明 |
|---|---|---|
| L1122 engine-check（pdfTeX 引擎墙） | **已越过** | 12 原语注册；OR 门聚合为真（§15.3） |
| expl3 loader/`\input expl3.ltx` | **已越过** | expl3-code.tex 40266 行真载入，expl3 包头已打转录 |
| **expl3-code.tex L193-206 引擎门闩** | **新（next）** | `\ifnum0%` 后接 `\expandafter\ifx\csname luatexversion\endcsname\relax…=0 %`：`scan_int` 读毕左操作数 `0` 之后的**关系符扫描臂不展开 token**（tex.web 该处 `repeat get_x_token` 语义），遇 `\expandafter` 直接报 `! Missing = inserted for \ifnum. <to be read again> expandafter`。**最小复现**：`\def\z{=}\ifnum0\z 0 T\else F\fi`——真实 TeX 得 `T`，NTex 得 `=0 T`（`\ifnum` 失败后 `=0 T` 泄漏为排版文本）。与第七/八刀所修**同族**（数字内条件聚合），位置在**关系符扫描**而非数字循环 |
| 映射表级联（L341/364/398/819） | 待定位（疑似同根因） | L196 门闩失配后 `\ifnum` 残留未决，后续 `\else`/`\fi`/`\ifeof` 被**悬挂条件机**吞掉（`Extra \else`/`Extra \fi`/`Argument … extra }`），`\/`/`\above`/`\accent`/`\advance` 报"不能在此模式使用/目标必须是寄存器"。已证伪"宏参数扫描执行原语"假说（`\def\id#1{<\string#1>}\id\hbox` → `<\hbox>` 正常）。修 L196 后需复测该级联是否自消 |
| 终态 | 未动 | pass1 ERROR、dumped=false；下刀首选 = scan_int 关系符扫描臂补 get_x_token 展开（复用 §14.1 条件臂经验），随后复测 expl3 bootstrap |

### 15.6 本轮改动清单

- `crates/ntex-core/src/eqtb/primitive.rs`：`Primitive` 加 12 个 `Pdf*` 变体
  （5 个入 `EXPANDABLE:`），族注释写明语义边界与"只注册 pdftex 一族"的互斥理由。
- `crates/ntex-core/src/expand/builtins.rs`：注册 12 名字（399 → 411），含
  \pdfstrcmp/\pdffilesize 必须真实现的无条件别名依据。
- `crates/ntex-core/src/param.rs`：`MISC_INTS` 63 → 65（misc 63 = \pdfoutput、
  64 = 随机种子；fmt roundtrip 测试通过）。
- `crates/ntex-core/src/expand/free.rs`：`int_param_index` 加 `PdfOutput => 63`
  （\pdfrandomseed 故意**不入**此表以保只读）；`is_expandable_prim` 加 10 个
  `Pdf*`；新增 `pdf_text_tokens`/`pdf_banner_tokens`/`pdf_creation_date_tokens`/
  `pdf_strcmp_value`/`pdf_detokenize_bytes` 助手与 `PDF_BANNER` 常量。
- `crates/ntex-core/src/expand/scan.rs`：`scan_number_inner` 补 5 个只读整数臂；
  数字上下文白名单（number_cs）补 4 个探测整数（\pdfoutput 由 int_param_index 覆盖）。
- `crates/ntex-core/src/expand/save.rs`：`\the` 上下文 7 个臂；`misc_int_name`
  反向表补 63/64。
- `crates/ntex-core/src/expand/expr.rs`：`expand_once` 补 10 个臂——**白名单原语
  必须在此有分支**，否则 `_` 原样保留触发"展开后重试"空转（OOM，见该文件
  fuzz 挂死修复注释）。
- `crates/ntex-core/src/expand/primitive_expand.rs`：`dispatch_expandable` 补
  10 个臂（只读整数单独出现 → 展开为数字，仿 \eTeXversion）。
- `crates/ntex-core/src/expand/primitive.rs`：`exec_primitive` 补
  `\pdfsetrandomseed` 执行臂。
- `crates/ntex-core/src/expand/tests.rs`：新增 10 测（§15.4）。
- 回归：`make check` 全绿（fmt / clippy -D warnings / cargo test --workspace）；
  TRIP/ETRIP 零回归（新原语族独立命名空间，既有"未定义"断言不受影响——
  `\luatexversion` 等其他引擎标记仍保持未定义）。

---

## 16. 2026-09-04 第十轮进展（关系符扫描臂 get_x_token 展开——expl3 L196 引擎门闩越过）

### 16.1 根因与修复落点

tex.web 关系符扫描（`@<Test relation between integers or dimensions@>`，tex.web
9785 起）取 token 用的不是裸 `get_token`，而是
`repeat get_x_token until cur_cmd<>spacer`（tex.web 8222）——**关系符位置的可展开
filler 先展开再判 `<`/`=`/`>`**。引擎此前的 `scan_relation` 直接 `fetch` 字面 token：
`\def\z{=}\ifnum0\z 0 T\else F\fi` 中 `\z` 不展开 → `! Missing = inserted for \ifnum.`
→ `\z` 放回后 `=` 落到右操作数被当垃圾 → `=0 T` 泄漏为排版文本（§15.5 最小复现）。

修复 = `crates/ntex-core/src/expand/cond.rs` `scan_relation` 重写为 get_x_token
循环，三臂（对齐 tex.web `x_token` 的 `expand` 分派表 7690-7691）：

1. **可展开项**：宏（非 protected）/`p.is_expandable()` 原语 → `expand_once` +
   TokenList 压栈重取；未定义 cs 报 `! Undefined control sequence.` 当 `\relax`
   继续循环（get_x_token 错误恢复）。`\noexpand` 冻结 token 不展开。
2. **条件原语（if_test **与** fi_or_else 都走 expand）**：`\ifx…\else 1\fi` 的
   `\else` 不落"非关系符"臂，而是 `step_conditional` + `drain_open_skip` 跳到
   `\fi` 弹帧（tex.web `@<Terminate the current conditional…@>`，9894-9903）。
   **不可用 `maybe_eval_cond`**（数字扫描版刻意对 `\fi`/`\else`/`\or` 返回 false
   放回外层——TRIP L82 十六进制循环的 `\fi` 属外层未决条件）；关系符位是
   **展开位置**，语义就是 get_x_token 本身。此处与 `expr.rs` `\expandafter` 臂
   同款惯用法（step + drain，含 Else/Or 的 `depth-1`）。
3. **非关系符恢复臂不变**：`<`/`=`/`>` 直写形式（TRIP 大量用例面）零改动；
   其余 token 仍报 `Missing = inserted`、放回、按 `=` 恢复（back_error）。

**第二根因（同点补出）**：`\else` 翻 Skipping 后是**惰性跳过**模型，扫描循环必须
自带 `is_skipping` 臂（与 `scan_number` 数字循环 438-443 同款）——否则被拒分支的
`1` 被当垃圾报 `Missing =`、`\fi` 被右操作数数字循环吞掉（`b` 取 0 而非 1）。
两臂缺一则复现用例分别停在 `=0 T` / `F`。

### 16.2 实测（latex_probe，latex.ltx 2026-06-01 + l3kernel 2026-08-10）

| 模式 | 修复前 | 修复后 | 变化 |
|---|---|---|---|
| 非 initex（=第九轮基线口径） | 26 错误行 | **24** | **恰去 2 行** |
| `--initex` | —（基线未留同模式转录） | **23** | 同比去 2 行 |

去掉的 2 行**全部**来自 expl3-code.tex L193-206 门闩（签名比对确认，其余错误签名
逐一相同 → **零连带变化**）：

- `! Missing = inserted for \ifnum.` `<to be read again> expandafter`
- 其下游 `! Missing number, treated as zero.`（`l.196 = 0 %`）

`\ifnum0%\expandafter\ifx\csname luatexversion\endcsname\relax…=0` 现按真实
TeX 语义求值：`\csname` 未定义名 = `\relax`（第六刀）→ 内层 `\ifx` 真 → 分支产出
空 → `=0` 在关系符位就位 → 门闩聚合真。终态不变：pass1 ERROR（`\advance 目标必须
是寄存器或内部参数`）、dumped=false。

### 16.3 单测（`crates/ntex-core/src/expand/tests.rs` 新增 3 测）

`ifnum_relation_position_expands_filler`（`\def\z{=}` 两向 + `<`/`>` 同族 +
`\ifdim`）、`ifnum_relation_position_evaluates_nested_cond`（L196 门闩原形两向 +
`\iftrue`/`\iffalse…\else` 在关系符位求值）、`ifnum_relation_literal_forms_unchanged`
（`<`/`=`/`>` 直写 + 数字 char 落非关系符臂的 back_error 恢复）。

### 16.4 级联证伪 + 下一真实阻塞点

**L341/364/398/819 级联 ≠ 本根因**（任务简报的"疑似同根因"证伪）：修复后
`! Extra \else.`（l.341）/`! Extra \fi.`（l.364）/`! Missing control sequence
inserted.`+`! Too many }'s.`（l.398 `\ifeof`）/`! You can't use \/ in vertical
mode.`/`\over`/`\tex_accent:D` undefined/l.819 全部原样保留。真根因在
**expl3-code.tex `\__kernel_primitive:NN` 别名表（l3names.dtx，L279-830）**：

```
\long \def \__kernel_primitive:NN #1#2 { \tex_global:D \tex_let:D #2 #1 }
\__kernel_primitive:NN \else     \tex_else:D      % ← l.341
\__kernel_primitive:NN \fi       \tex_fi:D        % ← l.364
\__kernel_primitive:NN \ifeof    \tex_ifeof:D     % ← l.398
```

`\else`/`\fi`/`\or` 在这里是**宏实参数据**（`#1`），但 NTex 实参扫描把它们当
"外层条件终结符"交给条件机并丢弃：
`collect_undelimited_arg`（`crates/ntex-core/src/expand/macros.rs:255`）对
Else/Fi/Or 无条件 `step_conditional` 后 recurse → `! Extra \else.`/`! Extra \fi.`，
且实参错位一格（`#1` 吃到 `\tex_else:D`，后续表项连锁错位 → l.398/`\/`/`\over`/
`\accent` 等签名）；定界版 `collect_args`（macros.rs:130-180）同款。**真实 TeX 的
宏实参扫描不展开任何 token**（tex.web `scan_args` 用 `get_token`）——`\fi` 只在
"闭合本次实参扫描之前已开启的帧"时才该交条件机，判定须带归属
（`owns_skip`/帧 line 归属，与第六/七轮 `drain_open_skip` 的边界同源）。
次级问题同域：`collect_args` 的 If\* 白名单缺 IfEof/IfVoid/IfHBox/IfVBox/IfInner/
IfVMode/IfHMode/IfMMode/IfFontChar（`\ifeof` 作实参被就地求值而非当数据）。

**下一刀首选**：`collect_undelimited_arg`/`collect_args` 的 fi_or_else 归属判定
（`\else`/`\fi`/`\or` 是否属本次实参扫描之前开启的外层帧——用帧的 line/归属标记
而非裸 `arg_cond` 计数），预期一次性消掉 l.341/364/398 及其连锁签名。次选：
`\PackageError` 早启 fallback（expl3-code.tex L208-215，`\lowercase` + `\catcode`
`\ =11` + 空格命名 cs 的 `\def\PackageError`）未生效 → l.220
`\protected\edef\ExplSyntaxOff` 的 `\PackageError`/`\ShortText`/`\LongText`
undefined 三连。

**勘误（基线口径）**：§15.3/§15.5 引用的"pass1 ERROR（级联下游）"转录是非 initex
模式产物；`--initex` 模式初始 catcode 表不同（`\ifnum\catcode`\{=1` 假 → 不触发
"LaTeX must be made using an initex" 误报），两模式错误行数 24/23。后续轮次对比
须锁定同一模式。

## 17. 2026-09-04 第十一刀进展（宏实参扫描的 \else/\fi/\or 一律是数据——expl3 别名表 \else/\fi 项越过）

### 17.1 根因与修复落点

§16.4 定位的 expl3 `\__kernel_primitive:NN` 别名表项
（`expl3-code.tex` L341 `\else`、L364 `\fi`）按 tex.web 语义修齐：

**TeX 宏实参扫描（tex.web `macro_call` 的 `@<Scan a parameter…@>`，7951-8130）取
token 一律用 `get_token`**——它只做 get_next + 词法包装，既不展开、也不推进条件机。
`\else`/`\fi`/`\or` 被条件机消费的唯一位点是 `expand`（get_x_token 的
`fi_or_else` 分支，7663-7691），而实参扫描不属展开位置。故实参位置的条件终结符
**恒为数据**，不存在"闭合本次扫描前已开启的帧"的情况——该帧的 `\else`/`\fi` 若在
实参里，要等实参经宏体重新入流、由主循环 get_x_token 才消费；§16.4"归属判定须带帧
归属"的设想实际退化为"一律当数据"。组实参（`@<Contribute an entire group…@>`，
8129-8143）同为 get_token，无例外。

修法（`crates/ntex-core/src/expand/macros.rs`）：

- `collect_undelimited_arg`：删去对 Else/Fi/Or 的 `step_conditional` + recurse。
  旧实现产生两类偏差：① 外层无帧时误报 `! Extra \else.`/`! Extra \fi.`
  （l.341/l.364 实签）；② 有帧时翻转/弹出外层帧并丢弃 token → 实参错位一格
  （#1 吃到 `#2` 内容，expl3 别名表连锁错位）。旧注释引的 `\expandafter\2\fi`
  惯用法实际不经实参扫描——`\fi` 由 `\expandafter` 的展开位置（expr.rs 的
  fi_or_else 臂）在 `\2` 实参扫描开始前已消费。
- `collect_delimited_arg`：删去 `arg_cond` 计数逻辑（含 If\* 白名单）——所有
  条件 token（含 §16.4 缺的 IfEof/IfVoid/IfHBox/IfVBox/IfInner/IfVMode/IfHMode/
  IfMMode/IfFontChar）在实参位置一律是数据。§16.4 的"白名单补齐"子问题被
  更强规则整体吸收，无需逐项补。
- 惰性跳过模型不变量：实参扫描不会在条件跳过区里运行（`process_one` 对跳过区非
  条件 token 直接丢弃、不派发宏调用；各扫描循环自带 `is_skipping` 臂只推进条件机
  不展开），故实参扫描无需 `is_skipping` 臂（与 scan.rs 数字循环/scan_relation
  不同——那些是展开位置）。

### 17.2 实测（latex_probe，--initex；基线同 §16.2 口径）

| 模式 | 第十轮 | 本轮 | 变化 |
|---|---|---|---|
| `--initex` | 23 错误行 | **21** | 恰去 2 行（l.341 `Extra \else.` + l.364 `Extra \fi.`，签名比对确认） |

去掉的 2 行全部来自别名表的 `\else`/`\fi` 两项；其余错误签名逐一相同
（含 l.398/l.819/\advance 终态）→ **零连带变化、零回归**。

### 17.3 单测（`crates/ntex-core/src/expand/tests.rs` 新增 3 测，双轨自动覆盖）

`arg_cond_terminators_are_data_undelimited` / `arg_cond_terminators_are_data_delimited`
（`\else`/`\fi`/`\or` 及 e-TeX If\* 全集作实参数据收进 #1，无 Extra 报错；
`\iftrue\f\fi x` 的 #1=`\fi` 数据在宏体里被主循环闭合外层帧 → `[]x`）/
`arg_cond_terminators_build_primitive_alias_expl3`（`\kp\else\myelse…` 建原语别名后
在 `\ifcase` 当终结符用 → `y`；`\expandafter\2\fi` 惯用法由展开位置消费 `\fi`）。

### 17.4 阻塞点刷新（第十一刀，下一刀）——§16.4 假设的"级联"证伪

**§16.4 预期 l.398/`\/`/`\over`/`\accent`/l.819 是 \else/\fi 错位级联——证伪。**
修复后这些错误**原签名保留**（21 = 23 − 2，恰好只去 \else/\fi 两行），且：

- l.398 `\ifeof` 项的实参扫描路径本就把 IfEof 当数据（新旧同），修复**不触及**该
  路径 → l.398 是独立阻塞点；
- 最小复现（l3names 同款 catcode 归位 + `\let\tex_global:D\global`/
  `\let\tex_let:D\let` + 表头 + `\else`/`\fi`/`\if`/`\ifcase`/`\ifeof` 五项别名）
  在 --initex 下**全绿**（`AFTER-TABLE` 达、零错误）→ 别名机制本身正确。

**下一真实阻塞点（精确）**：expl3-code.tex L190-220 的"引擎门闩/中止"控制流
（L196 `\ifnum0%…=0` 判断引擎过旧 → 走 `\next` = `\PackageError{expl3}…\endgroup
\endinput` 中止路径）。实测签名：expl3 载入推进到 l3names 表 l.398 `\ifeof` 项时，
一个宏体（Bytecode 帧，实参为 `\input expl3-code.tex ` 的载入包装）内冒出
字面 `\def` + Parameter `#`（`! Missing control sequence inserted.`，随后
`! Too many }'s.`），错误恢复吞掉该项 #2（`\tex_ifeof:D` 丢失）→ 表后续仍继续
（l.399+ 正常），但 l.819 次表区（`\tex_long:D \tex_def:D \use_ii:nn…` + 守卫版
`\__kernel_primitive:NN`，其体含 `\tex_ifdefined:D #1 … \tex_fi:D` 条件惯用法）
在 l.826+ pdfTeX-only 项上报 `! Argument of \__kernel_primitive:NN has an extra }.`
级联 → 终态 pass1 ERROR `\advance 目标必须是寄存器或内部参数`（dumped=false）。
次选同前：`\PackageError` 早启 fallback（l.205-215，`\lowercase` + 空格命名 cs 的
`\def\PackageError`）未生效 → l.220 undefined 三连（该三连在 l.398 恢复后出现，
疑为恢复吞行所致，非独立首因）。

### 17.5 本轮改动清单

- `crates/ntex-core/src/expand/macros.rs`：`collect_undelimited_arg`/
  `collect_delimited_arg` 实参位置条件 token 一律当数据（删 Else/Fi/Or
  step_conditional 与 `arg_cond` 计数），注释载 tex.web macro_call/scan_toks
  行号对照、偏差记录与惰性跳过不变量。
- `crates/ntex-core/src/expand/tests.rs`：新增 3 测（§17.3）。

## 18. 2026-09-04 第十二刀进展（l.398 阻塞点根因链——四引擎层缺口 + 行模型 catcode）

### 18.1 结论：§17.4 的"l.398 是独立阻塞点"进一步证伪——它是四条根因链的合流现场

§17.4 已证伪"l.398 是 \else/\fi 错位级联"；本刀用 `NTEX_TRACE_EXEC` 逐步定位证明
**l.398/l.819/\advance 终态是更上游根因的合流现场**。修复后该现场整体消失，载入
从 expl3-code L819 区（约 L290 处 `\advance` 终态）推进到 **L25851**（l3tl/l3fp
分析段）才遇下一阻塞。l.398 现场本身由**四条独立缺口**依次引爆：

### 18.2 根因一：`\expanded` 原语缺失 → expl3 L196 引擎门闩误判"引擎过旧"

expl3-code L190-196 门闩 `\ifnum0\expandafter\ifx\csname luatexversion\endcsname
\relax \expandafter\ifx\csname expanded\endcsname\relax\else 1\fi =0`——NTex 伪装
pdfTeX 但漏注册 `\expanded`（第九刀只加了 12 项，无此），`\csname expanded\endcsname`
= relax → 门闩为真 → 进入"引擎过旧"中止分支（L197-215）。第九/十/十一刀"越过
L196"实为**错误恢复误闯**：中止分支的 fallback 报错后 recovery 吞掉 `\endinput`，
执行"穿过"中止块继续，状态已被污染。

修复：`\expanded` 真注册为可展开原语（eqtb EXPANDABLE + numbered、builtins、
primitive.rs exec、expr.rs expand_once、free.rs is_expandable_prim 见下）——
语义 = `\edef` 内联（scan_left_brace + scan_edef_body，protected 宏抑制；偏差：
`#` 转换沿用 macro_def 语义，字面 `#` 直达实参在 LaTeX 源中极罕见）。

### 18.3 根因二：`scan_general_text` 在平衡组内以 relax 同义 cs 截断 → `\def` 后接字面 `#`

中止分支的 fallback L205 `\lowercase{\endgroup\def\PackageError#1#2#3{...}}`：
L199 `\expandafter\ifx\csname PackageError\endcsname\relax` 先把 `\PackageError`
**制造为 relax**；`\lowercase` 实参扫描（`scan_general_text`，io.rs）在组内遇到
该 relax 同义 cs 即无条件 break——组被截断为 `[\endgroup, \def]`，放回后 `\def`
的下一 token 是源里紧跟的字面 `#` → "! Missing control sequence inserted." +
"! Too many }'s."。tex.web scan_toks 的 general text：`\relax` 只作 `{` 前的
`<filler>`，平衡组内容一律数据。

修复：io.rs `scan_general_text` 的 `\relax` 终止加 `depth == 0` 守卫（组内为数据）。

### 18.4 根因三：scan_number 数字中途不展开 `\expandafter` → 门闩假分支数字 1 泄漏到关系符位

注册 `\expanded` 后门闩应走假分支（`\else 1\fi` → 1），但 scan_number 十进制循环
遇 `\expandafter`（非数字/非条件）直接放回 break，左操作数停在 0；关系符扫描
（scan_relation）处理 `\expandafter\ifx…` 链时拿到**活跃假分支的数字 1** → 
"! Missing = inserted for \ifnum."。修复：十进制循环遇 `\expandafter` 且其揭示的
下一 token 是**条件开始**（\if*）时展开重取（门闩 `\expandafter\ifx…` 即此形）；
指向普通命令（`\ifnum1=1\expandafter\chardef…` 的已完成数之后）不展开——避免把
帧外 `\else` 急切消费（回归）。其他可展开项（`\number`/`\the`/宏）维持旧行为
（停在它们处放回）——全展开会把 `\count0=5\number\count0` 的后续 `\number` 吸入
当前数，与既有语义/测试相悖（**记录偏差**：真实 TeX get_x_token 会吸入，NTex
不吸；TRIP/既有 300+ 单测锁死该语义）。

### 18.5 根因四：行模型把行尾硬编码为 cat-10 空格 → expl3 cat 32=Ignore 下 `\def` 参数文本带尾随空格

input.rs scan_token 在行中行尾无条件产出 `Char(cat=Space, ch=32)`（A1 行模型
"LF=5" 硬编码）；expl3 把 cat 32 设 Ignored(9)（L235 `\catcode 32=9`），真实 TeX
行尾插入的 char-32 被忽略、行边界消失。NTex 注入的 cat-10 空格进入
`\def\__kernel_primitive:NN #1#2` 的**参数文本** → `#2` 变"空格定界"实参 → 别名表
每一项的实参扫描一路吞到 l.819 首个 `}`（`\use_ii:nn #1#2 {#2}`）→ "Argument of
\__kernel_primitive:NN has an extra }" + 别名丢失 + `\/`/`\above`/`\accent` 执行 +
终态 `\advance` 目标错误。l.398 的 `\ifeof` 项只是吞行 recovery 的落点。

修复：input.rs 行尾分支查 cat 32 当前 catcode——Ignored → 忽略（行边界消失），
Space → 原空格，其他 catcode → 按该 catcode 产出（tex.web end_line_char 语义的
行模型子集；cat 32 非 Space 时才偏离旧行为，plain/TRIP/LaTeX 恒 Space 不受影响）。

### 18.6 根因五（次生）：scan_csname 按 cs **名**而非**含义**判终止 → `\cs_end:` 不闭合

l3names 之后 expl3 通篇 `\csname…\cs_end:`（`\cs_end:` = L1494
`\let\cs_end:\tex_endcsname:D`，槽是 EndCsname 原语）；scan_csname 只认名为
"endcsname" 的 token → 每个 `\cs_end:` 都报 Missing endcsname 级联。修复：沿别名
链解引用后判槽 == `Primitive::EndCsname`（tex.web cur_cmd=end_csname 语义）。

### 18.7 实测（latex_probe --initex，基线 §16.2 口径）

- 改动前（93f673e）：21 错误行，终态 `\advance 目标必须是寄存器或内部参数`
  （l.819 区，dumped=false）。
- 改动后：l3names 首表（~L281-818，约 300 项 `\tex_*:D` 别名）**完整执行**；
  载入推进至 **expl3-code L25851**（l3tl/l3fp 分析段）才遇下一阻塞；transcript
  3.89MB/9989 错误行，终态输入栈超限（5001 帧 > 5000）——**下游 expl3 生成器
  无界递归**，非本刀根因链。

### 18.8 下一真实阻塞点（精确）

l3basics（L1475 起）条件生成器 `\__prg_generate_conditional:NNnnnnNw`
（L1759-1775）被 L1907 `\prg_gset_conditional:Npnn \cs_if_exist:N #1 { p , T , F ,
TF }` 调用时：`\use:c { __prg_generate_#8_form:wNNnnnnN }`（csname 形式分派，
`#8` 遍历 p/T/F/TF）后随 `\tl_if_empty:nF {#8} { \msg_error:nnee … }` 作**尾参**。
NTex 未把该尾参当数据传给形式生成器，而是**就地执行** `\tl_if_empty:nF`（此时
l3tl 未载入、未定义）→ undefined-cs + Missing endcsname 级联，最终在递归生成中
栈超限。属"展开结构/尾参数据"类缺口，与 §16.4/§17.4 同族的下一独立阻塞点。

已知表象：错误上下文前几条误标 `l.398`——`scan_dimen_inner` 设 `error_anchor`
（clamp_dimen 回溯用）后未在成功路径清除，Undefined-cs 处理器直读
`error_context()` 不清锚 → 锚残留把后续错误上下文钉在 l.398（纯报错行号表象，
不影响执行语义；待后续刀清锚）。

### 18.9 改动清单

- `crates/ntex-core/src/eqtb/primitive.rs`：`Expanded` 进 EXPANDABLE 列表 + 编号列表。
- `crates/ntex-core/src/expand/builtins.rs`：注册 `("expanded", Primitive::Expanded)`（数组长 411→412）。
- `crates/ntex-core/src/expand/primitive.rs`：exec 主分发加 `Primitive::Expanded => self.exec_expanded()`。
- `crates/ntex-core/src/expand/expr.rs`：`exec_expanded`/`scan_expanded_group`（scan_left_brace
  + suppress_expansion + scan_edef_body）+ expand_once 加 Expanded 臂；`scan_csname`
  终止判定改按含义（别名链解引用判 EndCsname 原语）。
- `crates/ntex-core/src/expand/io.rs`：`scan_general_text` 的 `\relax` 终止加 `depth==0` 守卫。
- `crates/ntex-core/src/expand/scan.rs`：scan_number 十进制循环——`\expandafter` 揭示
  条件开始时展开重取（窄子集）；否则维持放回（偏差记录 §18.4）。
- `crates/ntex-core/src/input.rs`：行尾插入字符按 cat 32 当前 catcode 处理（Ignored→忽略）。
- `crates/ntex-core/src/expand/tests.rs`：新增 6 测（§18 各根因最小复现，双轨自动覆盖）。

### 18.10 验证

- `cargo test -p ntex-core --lib`：304 通过（含新增 6 测）；全 workspace `make check`
  （fmt + clippy -D warnings + cargo test 含 TRIP/ETRIP）全绿。

## 19. 2026-09-04 第十三刀进展（分隔实参整组贡献 + 定界符按 token 同一——l3prg w 尾参阻塞）

### 19.1 §18.8 阻塞点的两处根因（都已修，tex.web macro_call 语义归位）

l3basics 条件生成器 `\__prg_generate_conditional:NNnnnnNw`（L1759-1775）的
w 尾参 `\tl_if_empty:nF {#8} {…}` 被就地执行，根因不在"该不该展开"，而在引擎的
**分隔实参扫描器**与 tex.web `macro_call` 有两处偏差：

1. **定界实参内的平衡组被扁平扫描**：`{…}` 组内的 `}` 被当成深度 0 的"额外 `}`"
   报错截断，组内定界符也参与匹配。tex.web macro_call 的
   "Contribute an entire group" 把 `{` 起的平衡组**整体**作为实参数据贡献——组内
   token 只配对、不匹配定界符、`}` 不触发 extra-}。expl3 的 w 尾参
   `\tl_if_empty:nF {p} {…} \use_none:nnnnnnnn`（p_form 的 `#1 \s__prg_stop` 定界
   实参）正是此形 → 组内首个 `}` 处报错截断 → `\tl_if_empty:nF` 泄出被就地执行
   （l3tl 未载入 = undefined-cs 级联 + 递归栈超限）。
2. **定界符匹配按含义而非 token 同一**：`delim_token_eq` 对控制序列用
   `meaning_key`（\ifx 语义）。expl3 的定界/quark token 常**通篇无定义**
   （`\s__prg_stop`/`\q__prg_recursion_tail` 全文件只有使用无定义），而 w 尾参
   首 token `\tl_if_empty:nF`（l3tl 未载入）同样未定义——两个**不同名**的未定义
   cs 按含义比较相等 → `\tl_if_empty:nF` 被误作 `\s__prg_stop` 定界符提前终止。
   tex.web `macro_call` 的 `cur_tok=info(r)` 是 token 相等（cs 名同一），两个不同
   名的未定义 cs 是不同 token，不定界。

修复（`crates/ntex-core/src/expand/macros.rs`）：
- `collect_delimited_arg` 加组深度跟踪：`{` 计入深度，组内 token 只配对不匹配
  定界符、`}` 在 depth>0 时配对弹出；分隔符后缀匹配只在 depth==0 做；depth==0 的
  额外 `}` 走原 "Argument of \X has an extra }." 恢复（TRIP 格式不变）。non-long
  的 `\par` 检查保持"任意深度禁止"（tex.web 整组贡献循环同款）。
- `delim_token_eq` 控制序列改按 `csid` 同一（token 相等，tex.web 语义）；字符仍按
  (catcode,char)。记录偏差消除：此前含义比较会让 `\let` 同义或同未定义的不同 cs
  误作定界符。

### 19.2 实测（latex_probe --initex，口径同 §16.2）

- 改动前（e992ea8）：`\__prg_generate_conditional:NNnnnnNw` 的 recursion 在 l.1907
  无界自递归，转录 119779 行（9989 undefined-cs / 4984 extra-} 的重复块），终态
  输入栈超限。`\cs_if_exist:NTF` 等条件从未生成。
- 改动后：**recursion 干净终止**——每 `\prg_gset_conditional` 恰 2 次
  `NNnnnnNw`（首 form + recursion-tail 终止），`Argument of
  \__prg_generate_conditional:NNnnnnNw has an extra }` **0 条**（4984→0），转录
  241 行、22 undefined-cs。`\cs_if_exist:NTF` 仍未生成（见 §19.3），级联到
  expl3 L2089 `\cs_set_nopar:cpe` 的 "command-already-defined" 内部错误 +
  "Incomplete \if" 终态。

### 19.3 下一真实阻塞点（精确，本刀未修）

`\cs_if_exist:N` 的条件生成**仍未产出** p/T/F/TF：csname 分派构造出的是
`\__prg_generate_1_form:wNNnnnnN`（形为 `1`），而非 `p`。NTEX_COND_TRACE 证据链：
`\__prg_generate_conditional:nnNNNnnn`（L1733）实参绑定
`#6="#"`、`#7="1"`、`#8="p,T,F,TF"`——`{p,T,F,TF}` 表单清单落在 `#8`，而
`\tl_to_str:n {#7}`（L1750）对 `#7`（=`1`）取串 → NNnnnnNw 的 w 实参（表单分派
位）= `1` → `__prg_generate_1_form:wNNnnnnN` 未定义 → `\csname` 制造为 relax →
尾参 `\tl_if_empty:nF {1} {…}` 走"表单未知"分支被就地执行。

疑点：`#6`/`#7` 与表单清单的错位源于 `\__prg_generate_conditional_parm:NNNpnn`
（L1691 `#1#2#3#4#`，引擎按 4 个无分隔参数定义）的 p 实参只吞了 `#1` 的 `#`
（`#4="#"`），数字 `1` 与 `{p,T,F,TF}` 留在输入流被 nnNNNnnn 续吞为 `#7`/`#8`。
属 expl3 p 型签名/参数文本编排的更深层语义（非 w 定界扫描），且混入
`\cs_split_function:N` e 展开的 `\c_true_bool` 标记移位；最小复现需整段
l3basics~l3prg 前缀，无法再降。本刀禁改模式/数学机语义，留给下一刀。

另：错误上下文前几条误标 `l.398` 的 error_anchor 残留问题（§18.8）仍在，纯报错
表象。

### 19.4 改动清单

- `crates/ntex-core/src/expand/macros.rs`：`collect_delimited_arg` 平衡组整组贡献
  （depth 跟踪，定界符仅 depth==0 匹配）；`delim_token_eq` 控制序列改 csid 同一。
- `crates/ntex-core/src/expand/tests.rs`：新增 4 测（§19.1 各根因最小复现 + §19.3
  未定义定界符/未定义尾参数据对照，双轨自动覆盖）。

### 19.5 验证

- `cargo test -p ntex-core --lib`：308 通过（含新增 4 测）；全 workspace
  `make check`（fmt + clippy -D warnings + cargo test，26 套件）全绿。
- TRIP/ETRIP driver（`ntex-trip --driver ntex`）仍停 HEAD 已知数学组残留
  （`group_end 无配对 group_begin`，模式机未完成区），无新增触发；门禁以
  "残留签名一致 + 全量测试绿"为准。

---

## 20. 2026-09-04 第十四刀进展（\\string sprint_cs 语义 + 参数文本 `#{` hash_brace——p 型签名绑定归位）

### 20.1 根因与修复落点

§19.3 阻塞点（`\\cs_if_exist:N` 条件生成 p/T/F/TF 失败：`\\__prg_generate_conditional_parm:NNNpnn`
L1691 `#1#2#3#4#` 的 p 实参只吞 `#`、数字 `1` 与 `{p,T,F,TF}` 错位）归位为两处机制偏差：

1. **`\\string` 控制序列后误补尾随空格**（`expr.rs`/`primitive_expand.rs`/`free.rs`）：
   原 `detokenize_token` 是 e-TeX `\\detokenize` 语义（控制词后补空格），而 `\\string` 用 tex.web
   `sprint_cs`（"never prints a space after the control sequence"，L5622-5627）。expl3
   `\\cs_to_str:N`/`\\cs_split_function:N` 依赖无空格签名：`\\string\\cs_if_exist:N` 若带空格，
   csname 分派构造 `\\cs_if_exist:N TF`（含空格）→ 全部 Undefined。新增 `string_token`（无尾随
   空格版），`\\detokenize` 仍走 `detokenize_token`。
2. **参数文本 `#{` 收尾的 hash_brace 语义缺失**（`macros.rs`）：
   tex.web `scan_toks`（macro_def）`#{` 分支——`#` 丢弃（非定界符），该 `{` **计入末参定界符**
   （末参 = 分隔实参，定界符末 token 即此 `{`），且宏体末尾须另补同一枚 `{`（hash_brace），调用时
   与源 `{...}` 组 `}` 配对、组内容整体作 p 实参。expl3 p 型签名（`NNNpnn` 的 `#1#2#3#4#`）即
   "p 实参 = 到下一个 `{` 组为止的用户参数文本"。同刀修正实参收集的定界符判定顺序：depth==0 输入
   `{` 若构成完整定界符后缀 → 作定界符消费（不开组），否则整组贡献（第十三刀 w 尾参语义保留）。

### 20.2 实测（latex_probe --initex，口径同 §16.2）

- 基线（9ff4f9f，第十三刀后）：转录 241 行、**22 undefined-cs**，终态 expl3
  `command-already-defined` + "Incomplete \\if"（§19.3 级联）。
- 改动后：转录 1085 B（~35 行）、**2 undefined-cs**（l.301-302 `\\edef\\reserved@a{...\\string^^J\\@@}`
  区，错误恢复继续），§19.3 的 csname 生成错误风暴**清零**（p 型签名绑定归位）。
- 终态新阻塞：`\\__kernel_primitive:NN \\ifeof \\tex_ifeof:D`（l.398 标注，error_anchor 残留
  见 §18.8）报 `command-already-defined`（Argh bail out，dumped=false）——expl3 kernel 原语
  重命名机制：引擎疑似只注册原语短名（`\\ifeof`）缺 `\\tex_ifeof:D` 等 `\\tex_` 前缀别名，
  或 `\\__kernel_primitive:NN` 对"短名已定义"的接受语义与 pdfTeX 不一致（pdfTeX 中短名与
  `\\tex_` 名并存，expl3 先检测 `\\tex_` 名存在）。**下一刀靶子**。

### 20.3 改动清单

- `expr.rs`/`primitive_expand.rs`：`\\string` 用 `string_token`（sprint_cs，无尾随空格）。
- `free.rs`：新增 `string_token`（Char→Other/空格 cat 映射同 detokenize；ControlSeq 名不补空格；
  MacroParam/EndGroup 同 detokenize）。
- `macros.rs`：`scan_parameter_text` 返回第三元组 `hash_brace`（`#{` 收尾时该 `{` 入末参定界符
  文本 + 体尾补 token）；宏定义体尾追加 hash_brace（tex.web scan_toks hash_brace）；实参收集
  depth==0 时定界符后缀优先于整组贡献。
- `tests.rs`：修正 `\\string` 断言（去尾随空格，4 处）+ 新增 hash_brace/`#{` 定界符用例。

### 20.4 验证

- `cargo test -p ntex-core`：310 通过（含新增）；fmt + clippy（-D warnings）全绿。
- TRIP/ETRIP：停 HEAD 已知数学组残留（同 §19.5），无新增触发。
