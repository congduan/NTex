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

## 21. 2026-09-04 第十五刀进展（0 参数宏定界串匹配 + `\if` 操作数展开——expl3 kernel 原语重命名区越过）

### 21.1 §20.2 阻塞点的根因（两处引擎层机制偏差，均已修）

`\__kernel_primitive:NN \ifeof \tex_ifeof:D`（expl3-code l.398）报
`command-already-defined` 的假设（"引擎缺 `\tex_` 前缀别名 / 重命名语义偏差"）**证伪**：
l3names 表的 `\__kernel_primitive:NN #1#2` = `\global\let #2 #1`（expl3-code l.279），
引擎注册的短名原语经 `\let` 即建立 `\tex_…:D` 别名，无需原生注册；报错真身是
`\__kernel_chk_if_free_cs:N`（l.2056，`\cs_if_free:NF` + `\msg_error:nnee`，Argh 是
bootstrap 版消息处理器的固定模板——`Arguments '' and ''` 空实参是引擎 `\errmessage`
按字符过滤丢弃 cs token 所致，非 `\string`/`\meaning` 失效）。`\cs_if_free:N` 对
**未定义** cs 误判"已定义"，归位为两处机制偏差：

1. **0 参数宏的参数文本（纯定界串）在调用点不匹配**（`macros.rs` `collect_args` +
   `mod.rs` `call_macro`）：tex.web macro_call（L7971）`if info(r)<>end_match_token
   then @<Scan the parameters…@>`——参数文本非空时**即使 0 参数**也须在调用点匹配
   定界串（`|s=null|` 的 "simply scan the delimiter string" 分支）。expl3 条件生成器
   fast form（`\__prg_F_true:w`/`\__prg_TF_true:w`/`\__prg_p_true:w`，参数文本
   `\fi: \use:n` 等）依赖它吞掉 `\fi: <use-宏>` 并由体首 `\fi:` 闭合所在条件；旧实现
   直接返回空实参，`\use:n` 泄出被执行，`\cs_if_free:N`（fast form
   `\if_cs_exist:N #1 \else: \use_none:nnnn \fi: \if_meaning:w #1 \scan_stop:
   \__prg_F_true:w \fi: \use:n`）的 `\if_meaning:w` 被 4 参 gobble 误吞 → 判假。
   探测器（`\meaning` 转录）实测生成体与真实 expl3 fast form **逐 token 一致**，
   偏差只在调用点定界匹配。
2. **`\if`/`\ifcat` 操作数不走 get_x_token 展开**（`cond.rs` `evaluate_if` +
   新增 `get_x_char_operand`）：tex.web `@<Test if two characters match@>` 取操作数
   用 `get_x_token_or_active_char`——宏/可展开原语先展开一次，非字符操作数置
   `cur_cmd:=relax`/`cur_chr:=256` 哨兵（**cs vs cs 恒真、字符 vs cs 恒假**）；旧实现
   取字面 token 且要求两侧都是字符 token（cs vs cs 判假）。expl3 变体生成循环
   `\if:w #4 \__cs_generate_variant_loop_base:N #2`（l.2861）右操作数是宏调用。

### 21.2 实测（latex_probe --initex，口径同 §16.2）

- 基线（ba0e15d，第十四刀后）：转录 1085 B、2 undefined-cs（l.301-302），终态
  expl3 l.398 `command-already-defined` bail out、dumped=false。
- 修根因一后：载入推至 expl3-code **l.3324**（l3names 重命名表、l3bootstrap、
  l3basics、l3quark 入口全过），`\__kernel_primitive:NN` 区（l.398-1360）整体越过。
- 修根因二后（终态）：转录 4914 B（205 行），错误构成 `Extra \fi`×18 +
  undefined-cs×14 + `Extra \else`×12；终态 l.3324
  `\prg_generate_conditional_variant:Nnn \quark_if_no_value:N {c}{p,T,F,TF}` 报
  `conditional-base-undefined`（`\quark_if_no_value_p:N` 不存在）+ "Incomplete \if"
  （残留未闭合条件），dumped=false。

### 21.3 下一真实阻塞点（精确，本刀未修）

**`\cs_generate_variant:Nn` 机器的加载期条件失衡**（证据：`\write16` 标记插桩 +
截断 l.3250 复现）：

- 现场一（加载期）：`\exp_last_unbraced:NNNNo` 展开式定义的
  `\__cs_generate_variant:ww`/`:wwNw`（l.2837/2841，参数文本内嵌 `\tl_to_str:n{ma}`/
  `{pr}` 定界与 `\s__cs_mark`/`\s__cs_stop`）引入未闭合条件（`! Extra \fi.`，锚点
  陈旧报 l.398），使后续定义区落入条件丢弃分支**未定义**——l.2890
  `\__cs_generate_variant_chk:nnTF`、l.2895 `\__cs_generate_variant_loop:nNwN`、
  l.2941/2949 `_loop_end`/`_loop_long`、l.2999 `\__cs_generate_variant:wwNN`。
- 现场二（调用期，级联）：6 处 `\cs_generate_variant:Nn`（l.3245-3248、3293-3294）
  每处 `Extra \else`/`Extra \fi` + `\msg_error:nneeee` Undefined ×2
  （`invalid-variant`/`deprecated-variant` 报错路径被误执行——`\msg_error:nneeee`
  本体 l.11400 区才定义）；终态 `conditional-base-undefined` 是 p 形未生成的下游症状。
- 已排除：`\ifnum 0 \ifdefined…\fi … = 0` 惯用法（l.1406 同型）本就正确；`\ifdefined`
  /`\ifcsname` 对未定义 cs 判定正确（Q1-Q4 实测）；`\cs_if_free:N` 生成体正确。
- 下一刀靶子：`\exp_last_unbraced:NNNNo`（= 4×`\expandafter` + 第 5 参一次展开，
  l.2758）在"参数文本含 `\tl_to_str:n{…}` 定界"定义下的行为，及其与 `\s__cs_mark`
  （= `\scan_stop:`）定界匹配的交互；最小复现需 l3basics 前缀 + 插桩定位失衡点。

### 21.4 改动清单

- `expand/macros.rs`：`collect_args` n==0 臂——参数文本非空时调用点匹配定界串
  （tex.web macro_call），失配报 "Use of macro doesn't match its definition." 并忽略
  该调用（与 n>0 臂同恢复）。
- `expand/mod.rs`：`call_macro` 恒走 `collect_args`（0 参数宏不再短路）。
- `expand/cond.rs`：新增 `get_x_char_operand`（get_x_token_or_active_char 语义：
  展开/未定义 cs 报错当 relax/`\noexpand`→active char/条件机推进，同
  `scan_relation` 臂结构）；`\if`/`\ifcat` 按 tex.web 哨兵语义比较（非字符 →
  `None`，cs vs cs 恒真）。
- `expand/tests.rs`：新增 3 测（0 参数定界串匹配、失配忽略、`\if`/`\ifcat` 操作数
  展开与哨兵）。

### 21.5 验证

- `make check`（fmt + clippy -D warnings + test）：全绿，`ntex-core` 313 测通过
  （含新增 3 测）。
- 最小复现：0 参数定界串（`\def\prg:Ftrue:w\fi:\use:none:n{…}`）与 `\if` 操作数
  展开各一组，修复前 `Extra \else`/cs vs cs 判假，修复后与真实 TeX 一致。
- TRIP/ETRIP：停 HEAD 已知数学组残留（同 §19.5），无新增触发。

### 21.4 字体面备料（2026-09-04，主控）

LaTeX 全家 TFM 已从 TL tlnet 备齐（/tmp/latexsurvey/，勿重下）：

- `fonts/tfm/public/cm/`（75）：cmr/cmsy/cmex/cmmi/cmss/cmtt/cmbx 全系——
  preload.cfg/fonttext.cfg 预载区 + \documentclass 默认字体需求；
- `fonts/tfm/public/latex-fonts/`（23）：lasy*/lcircle*/icm* 等 LaTeX 专属；
- `fonts/tfm/public/amsfonts/`（52）：msam/msbm/eu*——AMS 符号面。

引擎现有 ~/.ntex-fonts/ 仅 cmr10 单个。后续刀到达字体阻塞点时：
TFM 按 NTEX_TFM_DIR 查找路径拷贝所需子集；DVI→PDF 的字体嵌入（Type1/PFB）
按 ntex-pdf 既有机制（cmr10.pfb 已验证）按需扩展。

### 21.5 宏包与中文备料（2026-09-04，主控，与十六刀并行）

TL tlnet 宏包已备齐（/tmp/latexsurvey/tex/latex/，勿重下）：

- 战役后续关：`tools`（latex.ltx 尾部 \@ifpackageloaded 类依赖）、`graphics`、
  `amsmath`、`geometry`、`oberdiek`、`hyperref`+`url`（\documentclass 常用面）；
- M9 中文战役：`ctex`（ctexart.cls 等）、`xecjk`、`fandol` 字体
  （fonts/opentype/public/fandol/ 12 个 OTF：宋/黑/楷/仿宋 + Braille）——
  plan.md §11.0 阶段 B 的"中文 TTF 备料"缺口已消。

---

## 22. 2026-09-05 第十六刀回归收敛（条件帧序改动后的 expl3 加载回归——主控二分 + 本地复现）

### 22.1 回归发现与二分定位

主控 worktree 二分给出区间 b7aeeba..HEAD（d672423 wip 操作数臂 / 4f45101 条件栈帧序
三处修复 / 360c342 TRIP 数学）。本轮用**每提交独立 target 目录**复核（共享
CARGO_TARGET_DIR 跨 worktree 时 cargo 指纹误判 fresh、交付陈旧二进制——同一 HEAD
两次跑出 l.398 与 l.3324 两种结果，见 §22.4），逐提交 `latex_probe --initex`：

| 提交 | 阻塞点 | 签名 |
|---|---|---|
| b7aeeba（良基线） | expl3-code **l.3324** | `conditional-base-undefined`（`\quark_if_no_value_p:N`）+ Incomplete \if |
| d672423 | **l.3246** | `command-already-defined`（`\cs_generate_variant:Nn \cs_replacement_spec:N { c }`）+ Incomplete \if |
| 4f45101 | **l.398** | `command-already-defined`（`\__kernel_primitive:NN \ifeof \tex_ifeof:D`）+ Incomplete \ifx |
| 360c342 | l.398 | 同 4f45101 |

**两个回归源**：d672423 先把阻塞点从 l.3324 拉回 l.3246，4f45101 再拉回 l.398；
360c342 无辜（TRIP 数学，不碰 cond 主路径）。

### 22.2 根因（对拍 + 插桩闭环）

紧复现 = expl3-code 前 3260/3246 行前缀（catcode 归位 + `\ExplLoaderFileDate` 垫片 +
`\immediate\write16{PREFIX-OK}\end`）：b7aeeba 过、HEAD 挂，且 **l.3246 单行触发**
（l.3245 前缀两边都过）。NTEX_COND_TRACE/IFX_TRACE 对拍 + `scan_relation`/
`scan_number_inner`/insert_relax 门逐点插桩，闭环到：

- d672423 在 `get_x_char_operand`（`\if`/`\ifcat` 字符操作数位）新增"嵌套 `\\if*`
  真实求值"臂（step_conditional + 开口帧跨操作数边界存活）。本引擎的宏实参抓取/
  `c` 变换/csname 机器全部按 b7aeeba 的"操作数位条件 token = 数据（非字符哨兵）"
  约定校准；求值留下的开口帧使后续**定界实参扫描在错位 token 流上配对**。
- 终端证据：expl3 `\cs_if_free:cT { exp_args:Nc }`（l.3027）的 csname 文本被污染成
  **`{exp_args:Nc}`（带花括号）**→ `\ifx <该 cs> \scan_stop:` 判"未定义" → 生成机器
  对已存在的 `\exp_args:Nc`（真 Macro，二参 long）重复走 `\cs_new` →
  `\__kernel_chk_if_free_cs:N` → kernel command-already-defined fatal。
  4f45101 的帧序三修（压帧先于求值/skip_ahead 压帧后取定/target-1 真值转换）+ 
  insert_relax 门本身 tex.web §L9725/@<Terminate…@> 忠实，但它把 d672423 臂的
  影响面从 `\if` 扩到了全条件族，l.3246 处错位在 l3names 更早处显形（l.398）。
- d672423 的动机场景（expl3 l.2896 变体循环 `Extra \else` 级联）**已被第十五刀修复**
  （0 参数宏定界串调用点匹配，§21.1-1），该臂属重复治前期病灶。

### 22.3 修复与 tex.web 裁决

`get_x_char_operand` 的条件臂收窄为**只路由 `\else`/`\fi`/`\or`**（fi_or_else 族）：
本条件（栈顶 Evaluating）未决时 insert_relax（frozen `\relax` 回插、token 不消费），
嵌套 `\if*`（if_test 族）维持数据语义落非字符哨兵。

裁决依据（两难不可全得，取 TRIP 锁定面 + 实证 expl3 面）：

- **`\fi`/`\else` 路由必须保留**：TRIP l.313 `\if\the\badness\fi\message{…}` 的
  `\fi` 在本 `\if` 操作数位到来，insert_relax 门是唯一收口（4f45101 注释 + TRIP
  套件锁定）；本轮全量回退该臂的实验即复现 TRIP pass2 `! \if 缺少 \fi`。
- **嵌套 `\if*` 求值必须移除**：tex.web get_x_token 字面语义确实求值（§@<Test if
  two characters match@>），但 NTex 实参抓取层未对齐该语义前，求值 = token 流错位。
  已知残差：`\if n\if c o N\else n\fi X T\else F\fi` 真实 TeX 输出 "X T"，本引擎
  "nX TF"（哨兵语义）；已由 `if_operand_nested_conditional_stays_sentinel` 测试
  锁定现状并注明待办（实参抓取层整体对齐 get_x_token 后重开为求值语义）。
- 关系符位（`scan_relation`）与数字循环（`scan_number`/`maybe_eval_cond`）的既有
  条件臂不动——那里帧由本条件就地收口，无跨边界泄漏（第七/八/十轮验证面）。

### 22.4 验证与工具链坑

- `latex_probe --initex`：阻塞点 **l.398 越过 → l.3324**（与 b7aeeba 等同）；
  l.3246/3260 前缀复现双过。
- `cargo test -p ntex-core`：**315 通过 / 0 失败**（含 if_operand 3 测：2 原样绿 +
  1 个按恢复后语义重写并注明 tex.web 残差）；`cargo fmt --check`、
  `cargo clippy -p ntex-core --all-targets` 干净。
- TRIP/ETRIP driver（`ntex-trip --driver ntex`）：残留签名与 360c342 基线**逐字
  一致**（TRIP pass2 `组未闭合 groups=[SemiSimple, MathLeft, MathLeft, Align]`；
  ETRIP pass2 `group_end 无配对 group_begin` @ Checking \currentgrouptype 区），
  无新增触发。
- **工具链坑（二分纪律）**：共享 CARGO_TARGET_DIR 跨 worktree 复用会因 cargo 指纹
  误判交付陈旧二进制（哈希名只含包元数据 `latex_probe-1f77894351f38db8`，各提交
  同名互覆）；bisect 必须每 worktree 独立 target 目录（本仓全量构建仅 ~20 s，可
  接受）。worktree 还须手拷 `fixtures/`（gitignore 不随 checkout）。
- 下一刀靶子（恢复 l.3324 之后的真阻塞）：`\prg_generate_conditional_variant:Nnn`
  的 p 形生成（`conditional-base-undefined`，`\quark_if_no_value_p:N` 未生成），
  沿 §21.3 的 `\exp_last_unbraced:NNNNo` + 定界实参链下探；前置工程为
  §22.3 残差的实参抓取层对齐。

### 22.5 十六刀主靶极简复现（五轮接管，主控验收）

五轮（glm，跨夜复现迭代 56 样本，/tmp/r16b/）被接管时无代码产出（纯复现），
但其 g3→极简化收敛出**直指机制层的最小复现**：

```tex
\def\ttl#1{#1}
\expandafter\def\expandafter\resW\expandafter{\ttl { ma }}
\message{W: \meaning\resW}
```

- 引擎实测：`W: macro:-> ma`（\ttl 被展开、组剥、空格异常）
- 真实 TeX：链3 A=`{` B=`\ttl` → `{` 放回、`\ttl` 展开读参 `{ ma }` → 流 =
  `\def\resW{ ma }` → `W: macro:-> ma`（**引擎此处正确**）
- 关键差异场景（min3）：

```tex
\def\ttl#1{#1}
\def\testY#1#2{\expandafter\def\expandafter\resY\expandafter{\expandafter#1#2}}
\testY \ttl { ma }
% 引擎: Y: macro:->\ttl ma   真实 TeX: Y: macro:->\ttl {ma}
```

**差异 = `\expandafter#1#2` 展开中 B=`{ ma }` 组参数回流后，def 体收集
丢组符**（对照 min2 直接 `\def\resZ{ \ttl {ma} }` 组保留正常——收集器本身
无恙，是 `\expandafter` 的 `expand_once` 路径中组参数 token 的 `csid=None`
回流形态有问题）。下一刀靶子：`expand_once`（expr.rs L65）尾部对非 cs
token（组符/字符）的回写与 fetch 流的组配对。修复后预期 l.3324 越过
（`\\__cs_generate_variant:ww` 的参数文本 `{ \\tl_to_str:n{ma} }` 定界依赖
组原样回流）。

## 23. 2026-09-05 第十七轮：store_arg 组实参语义裁决（简报前提证伪）+ 两项 tex.web 保真修复

主控简报锁定"`collect_undelimited_arg` 剥组是 l.3324 根因，须改为连组存储"。
本轮按硬约束（TRIP fixtures 为 ground truth）先做语义裁决，**结论：简报前提不成立**，
现行剥组实现正确；随后落两项 tex.web 保真修复，latex.ltx 载入 l.3324 → l.10298。

### 23.1 语义裁决：真实 TeX 对无分隔组实参**存储前剥组**（"连组存储"不存在）

tex.web macro_call（L8144-8160）两层机制：

1. `@<Contribute an entire group to the current parameter@>`：组**连同花括号**暂存
   （`fast_store_new_token`/`rbrace_ptr:=p`）；
2. `@<Tidy up the parameter just scanned, and tuck it away@>`：判定
   `(m=1) and (info(p)<right_brace_limit)` → `link(rbrace_ptr):=null; free_avail(p)`
   **剥去外层花括号**后才 `pstack[n]:=…`。Knuth 注释原文："If the parameter consists
   of a single group enclosed in braces, we must strip off the enclosing braces."

无分隔实参首轮即 `goto found` ⇒ **`m` 恒为 1** ⇒ 组实参必被剥组；宏体回填
（`begin_token_list(param_stack[...],parameter)`，L7518）直接逐 token 读出，**无
第二级剥组**。即不存在"存储带组、使用剥组"的两级模型。

三路独立证据互相印证：

- **TRIP log**（官方 ground truth）：45 条 tracing_macros `#N<-…` 追踪**无一含花括号**；
- **极简复现**（§22.5 的 testY）tex.web 逐 token 推演：`#2`=`␣ma␣`（剥组后含首尾空格），
  最内层 `\expandafter` 的两次 `get_token`（L7699-7703，**非** get_x_token）读到空格
  token 不展开、原样放回 ⇒ `\resY` 体 = `\ttl␣ma␣` ⇒ `\meaning` = `macro:->\ttl ma `。
  引擎实测输出 `Y: [macro:->\ttl ma ]`（尾空格即 #2 首尾空格）**与推演逐 token 一致**；
  简报所记"真实 TeX(对): `\ttl {ma}`"系预测而非实测（本机无真实 TeX 二进制可对照）；
- **TRIP 全篇排版输出**：若参数连组存储，几乎每个宏调用的 `#1` 都会向 DVI 泄出
  `{`/`}` 字符，trip.log 8000+ 行无从对上。

### 23.2 真偏差（第十七刀）：定界实参整体恰为单组时未剥组

tex.web 的剥组判定对**定界/无分隔两族一视同仁**，仅要求 `m=1`（整个实参恰为一个
顶层贡献单元且其为组）。现行 `collect_delimited_arg` 只按定界符后缀截断、不剥组：

```tex
\def\a#1!{…}  \a{x}!   % 引擎(旧): #1={x}   tex.web: #1=x  （m=1 → 剥）
\def\b#1!{…}  \b xy!   % 两边一致: #1=xy
\def\c#1!{…}  \c {x}y! % 两边一致: #1={x}y  （m=2 → 不剥）
```

修复：`macros.rs` 新增 `strip_single_group`——buf 首 token 为 `{` 且其配对 `}` 恰为
buf 末 token（等价 `m=1`）时剥一层。仅正常 found 路径剥：runaway / extra-} /
Paragraph ended 恢复路径在 tex.web 里 `pstack[n]:=link(temp_head)` 原样保留。
`\a{{x}}!` → `{x}`（只剥一层）。

### 23.3 修复二：`\def` 体 "Illegal parameter number" 由 fatal 改 tex.web 可恢复

tex.web `@<Look for parameter number or ##@>`（L9416-9423）：报错 + `back_error`
（越界 token 放回输入流）+ `cur_tok:=s`（存回字面 `#`）后**继续扫描**。旧实现
`return Err(invalid_input)` 整体中止（`\expanded`/`\edef` 体共用同一路径）。
现按 tex.web 恢复：`unread(越界 token)` + `write_error_help`(含 Knuth 三行 help)
+ `out.push(字面 #)`；`scan_balanced_text`/`scan_edef_body` 线程化 `def_name`
（`\expanded` 无定义中 cs，传空省略 "of \X" 段）。

### 23.4 验证与遗留

| 门禁 | 结果 |
| --- | --- |
| ntex-core 全测 | 315 通过 / 0 失败（`delimited_arg_nested_groups_do_not_error` 预期由旧偏差改为 tex.web 语义，另补 m≥1 不剥与只剥一层两断言） |
| TRIP（--fixtures 临时布局） | log 与基线**逐字节一致**；pass2 终态硬错不变（`组未闭合 groups=[SemiSimple,MathLeft,MathLeft,Align]`） |
| ETRIP | fixtures 缺失（`fixtures/trip/etrip/`）——既有跳过，非本轮引入 |
| latex.ltx --initex | **l.3324 → l.10298**（越过了 conditional-base-undefined bail out + "Incomplete \ifx; all text was ignored after line 3324"） |

**十六刀主靶未达成**：`Extra \else`/`Extra \fi` 家族与修复前逐字节相同（transcript
前 199 行一致，含 l.398 1 处 + l.3245-3395 每个变体调用 7 错：1×Extra \else、
3×Extra \fi、2×`\msg_error:nneeee` undefined、2×后续级联）。剥组/Illegal-param
两项修复只是让载入**越过 bail out 继续读**，错误家族原样沿后续千行重复，故错误
计数 46 → 7669——其中绝大部分是先前被 "all text was ignored" 屏蔽的同一根因。

**新阻塞点（下一刀靶）**：`l.10298` 输入栈超限（5001 帧 > 5000），同族根因在
`\__cs_generate_variant_loop:nNwN`（expl3-code.tex L2891-2920）的 `\if:w` 三层嵌套
与 `{ ~ { } \fi: \__cs_generate_variant_loop_long:wNNnn } ~` 组内藏 `\fi:` 惯用法——
定性线索：per-call 7 错中 `Extra \else`×1/`Extra \fi`×3 的 1+3 结构恰对应该函数
`\if:w`/`\else:`/`\fi:` 的错配形态；`\msg_error:nneeee` undefined 说明变体机器
走了**本不该走的** variant-same-as-base/invalid-variant 分支（载入期
`\__cs_generate_variant_chk:nnTF` 是恒 FALSE 桩 `\use_ii:nn`，expl3-code.tex
L2890，L5762 才替换为 `\str_if_eq:nnTF`），即桩的 `\use_ii:nn` 取参被错位。

## 24. 2026-09-05 第十八轮：操作数位嵌套条件重开求值语义（§22.3 裁决推翻）+ protected 数值扫描门拆除

主控简报锁定的 l.10298 输入栈超限（5001 帧）只是表现层。本轮用 `NTEX_COND_TRACE`
+ 临时 `collect_args` 实参转储（已拆）把变体机器逐迭代铺开，定位**两个**引擎偏差，
合并修复后 latex.ltx 载入错误 **7669 → 5**（transcript 666446 → 1335 字节），
l.10298 越过，expl3 变体机器 l.3245-3395 **每个变体调用 7 错的家族清零**。

### 24.1 修复一：`\if` 操作数位嵌套 `\if*` 重开为求值语义（§22.3 裁决推翻）

§22.3 曾把操作数位的嵌套 `\if*` 裁为**数据哨兵**（非字符）。本轮证明该裁决的
实害不在求值本身，而在**求值后帧不收口**：d672423 求值时内层 `\if` 帧跨操作数
边界存活，但其 `\else:`/`\fi:` 落到**外层**条件——`\else` 提前终止外层跳过、
`\fi` 把外层帧弹掉。expl3 变体机器的 `\if:w 0 <A><C>0 …\else:<invalid>\fi:`
（expl3-code L2899-2905）正是此形态：

- A 测试 `\if:w N #4 \else:\if:w n #4 \else:1\fi:\fi:` 数据哨兵化后不进条件机，
  其 `\fi:` 落到外层 `\if:w 0` → 外层被 A 链错误收口；
- C 测试/`0`/`\special`/`\else:<invalid>` 全部落到顶层裸执行 → invalid-variant
  误触发（`\msg_error:nneeee` undefined）+ Extra `\else`/`\fi` 级联 = 每调用
  7 错（1×Extra \else、3×Extra \fi、2×undefined、2×级联，expl3 l.3245-3395
  全体变体调用重复）→ 错误计数 46→7669，末态 l.10298 输入栈超限。

**修复**（`cond.rs` `get_x_char_operand`）：操作数位的 `\if*`（if_test 族）就地
求值 = `step_conditional` + 被弃分支 `drain_open_skip`（同 `expr.rs` `\expandafter`
臂的组合；Else/Or 的 drain 目标下移一层）。帧由内层条件**自带收口**，被弃分支
就地排空，只有选中分支文本落入操作数扫描——tex.web get_x_token → expand →
conditional 的字面语义。`\else:`/`\fi:`/`\or:` 路由（TRIP l.313 门）与
`\noexpand` 臂不动。

**§22.3 的 V1 残差翻正**：`\if n\if c o N\else n\fi X T\else F\fi` 四值
V1-V4 = X T / F / F / F（与 tex.web `@<Test if two characters match@>` 逐点
推演一致）。`if_operand_nested_conditional_stays_sentinel` 改名
`if_operand_nested_conditional_evaluates` 并锁新值。

### 24.2 修复二：protected 宏的数值/操作数扫描门拆除（e-TeX 抑制面收窄）

quark 模块 `\__kernel_quark_new_test:N`（expl3 l.3782）报 "Missing number,
treated as zero" + `\exp_end_continue_f:w` 回读。根因：引擎把 e-TeX
`\protected` 抑制面实现为全局 `suppress_expansion` 计数，数值扫描
（`scan_number_inner`/`scan_dimen_inner` 单位词两臂）也查它——而
`\expanded`/f 型实参内 `suppress_expansion > 0`，expl3 l3expan 全族的
`\exp:w \exp_end_continue_f:w <stuff>`（romannumeral 技巧；`\exp_end_continue_f:w`
是 **protected** 宏，expl3-code L2792 `\cs_new_protected:Npn`）被挡成不可展开。

etex-manual 原文："Protected macros … are not expanded **when building an
expanded token list**"（\edef/\xdef/\message/\errmessage/\special/\mark/\marks/
\write + 对齐 \noalign/\omit 前瞻）——**抑制只盖 token 列表吸收，数值扫描
（scan_int/scan_dimen 的 get_x_token）不在其列**。修复：三处数值扫描的
`EqSlot::Macro(_)` 一律可展开（列表构建处的门保留：`scan_edef_body`、
`process_expand_only`、`fetch_non_filler` 等）。

### 24.3 验证与遗留

| 门禁 | 结果 |
| --- | --- |
| ntex-core 全测 | **315 通过 / 0 失败**（sentinel 测试按 §24.1 翻正） |
| TRIP | 与 HEAD（6128722 worktree 隔离重建）输出 **逐字节一致**（仅 tempdir 路径与计时尾差）；pass2 终态硬错不变（`组未闭合 groups=[SemiSimple,MathLeft,MathLeft,Align]`） |
| latex.ltx --initex | 错误 **7669 → 5**（2×l.301/302 `\^^J` undefined 既有 + 3×新阻塞点级联）；transcript 666446 → 1335 字节；**l.10298 越过**（输入栈不再超限，pass1 OK） |
| ETRIP | fixtures 缺失（既有跳过，非本轮引入） |

**新阻塞点（下一刀靶）**：quark 模块 l.3782 `\__kernel_quark_new_test:N
\__tl_if_recursion_tail_break:nN` → `Module quark, message name
"invalid-function": Arguments 'test' and ''` bail out。已定位到
`\__quark_module_name:N`（expl3-code L3505-3525，`\__quark_tmp:w` 以
`\tl_to_str:n { : _ }` 实例化的**参数化嵌套定义**族）：引擎中它返回**未走
chunk-walk 的原名**而非模块名 `tl`。疑似点（本轮探针 B/C 已排除了
`\cs_to_str:N`/`\exp_last_unbraced:Nf` 单独路径）：`\exp:w \exp_end_continue_f:w
{ \cs_to_str:N #1 }` 中**组内容的展开时机**（tex.web romannumeral 在 `{` 处停，
真实 TeX 里由 e 型实参的 xpand 循环先展开组内 token 再被 `:w` 的 `##1` 抓走；
引擎 scan_edef_body 的组收集与宏实参抓取的交错序）与多 token 定界符
（`#1<sp>:<sp>#2<sp>\s__quark`，qp14/qp15 实测引擎把首个 `<sp>` 留给了
`#1`，真实 TeX 后缀匹配应收掉）两处，连带 `\exp_args:NNcc` undefined
（quark 条件生成器动态造的 exp_args 变体未落地）与 latex.ltx l.398
`\__kernel_primitive:NN` 行的级联。另：`\show`/`\meaning` 的宏参数文本打印
不显示定界符（primitive.rs L431 只拼 `#1..#n`），本轮排查中曾误导，待修。

## 25. 2026-09-05 math_display 单测回归修复（360c342 的 `$$` 闭合侧接线缺陷）

### 25.1 回归与定位

`cargo test -p ntex-layout math_display` 6 失败（math_display_formula 等，全部
`数学模式未闭合（缺少 $）`——typesetter.rs finish 校验 `builder.math` 非空）。
主控 worktree 逐提交二分：b585864…4f45101 全绿，**360c342 转红**。该提交动机
正当（TRIP L210/L340 受限水平 `$$` 退化），但 ntex-layout 单测没跑——
`$$x$$`（垂直模式、文档开头）当场报未闭合。

插桩追踪 `$$x$$`：`[entering=true allowed=true display=true]` → sink Vertical
臂进显示数学 ✓；`[entering=false allowed=false display=false]` → sink
DisplayMath 臂收显示数学 ✓；随后**第三个事件** `[entering=true display=false]`
在垂直模式重开行内公式，math.len=1 永不归零——多出来的事件就是被 back_input
的第二个 `$`。

### 25.2 根因：tex.web 两侧不对称，360c342 只改对了一半

tex.web 的 `$` 处理**进入/闭合两侧语义不对称**：

- 进入侧 `hmode+math_shift: init_math`（tex.web L21700/L21703）：`get_token`
  后 `if (cur_cmd=math_shift) and (mode>0)` 才进显示数学，否则 `back_input` 进
  **普通**数学——360c342 已如实接线（`math_display_allowed`；受限水平
  mode<0 → 放回，`$$` 退化为两次独立进出）。
- 闭合侧 `mmode+math_shift: after_math`（tex.web L22401）：**显示数学**
  （mode=+mmode）收尾必 "Check that another $ follows"（L22585：`get_x_token`，
  非 `$` 则报 "Display math should end with $$" + back_error 照收）；**行内
  数学**（mode=-mmode）走 "Finish math in text"，**根本不 peek**——紧随的
  `$` 落回水平/垂直模式由 init_math 重新判定（`$x$$y$` = `$x$`+`$y$`，
  real TeX 如此）。

360c342 把消费门改成 `entering && math_display_allowed()`，闭合侧
（entering=false）一律"只 peek 不消费"：`$$x$$` 的闭合 `$` 把第二个 `$`
back_input 后，它在非数学态（垂直模式，allowed=true）重开一个**永不闭合**的
行内公式。同文件 ntex-layout/tests/modecheck.rs 首测当时即转红——其注释
（"已知缺口：…探测须改为查询 sink 的 `mode>0`"）预告的正是这条，但断言写的
是**现状**（`}` 在数学模式被追踪）而非 tex.web 语义，等于把缺口钉进了门禁。

### 25.3 修复：闭合侧按 sink 模式分派（新增 `math_close_consumes_dollar`）

- `crates/ntex-core/src/sink.rs`：新增 trait 钩子 `math_close_consumes_dollar()`
  （默认 true：纯展开轨道无模式概念，保持"peek 到即消费"的既有行为）；
  `math_display_allowed` 语义不变（进入侧 mode>0）。
- `crates/ntex-layout/src/typeset/sink.rs`：NodeBuilder 覆写为
  `matches!(mode, DisplayMath)`——只有**显示**数学收尾要求配对 `$`。
- `crates/ntex-core/src/expand/mod.rs`：`consume_for_display = entering ?
  math_display_allowed() : math_close_consumes_dollar()`。
- `crates/ntex-layout/tests/modecheck.rs`：首测断言翻正为 tex.web/trip.log
  语义（`{math mode: math shift character $}` + `{restricted horizontal mode:
  end-group character }}`，即 trip.log L1832/L1834 同款）。

进入侧行为与 360c342 完全一致（TRIP 受限水平退化不动）；闭合侧只在
**DisplayMath** 下与 360c342 不同（消费配对 `$`），行内数学闭合与 360c342
逐位一致（不消费）。约束条件逐 token 对拍过 TRIP L210 `\hbox{$$}$\par}`：
`$`#1 peek 放回 `$`#2 进普通数学 → `$`#2 peek `}` 放回并收数学 → `}` 落回
受限水平，与 trip.log L1831/L1832/L1834 逐行一致。

### 25.4 验证

| 门禁 | 结果 |
| --- | --- |
| `cargo test -p ntex-layout` | **159 lib + 4 + 4 + 3 全绿**（math_display 9/9，含 `math_display_inside_hbox_falls_back_to_inline_math` 受限退化用例；modecheck 3/3） |
| `cargo test -p ntex-core` | **315 通过 / 0 失败**（f1c1503 的 if_operand V1-V4 不回退） |
| TRIP | 与 HEAD（99c3a63 worktree 隔离重建 + 独立 CARGO_TARGET_DIR）残留**逐字节一致**；pass2 终态硬错不变（`组未闭合 groups=[SemiSimple,MathLeft,MathLeft,Align]`） |
| ETRIP | fixtures 现已在场（§24.3 时缺失，非本轮引入）。**净改善**：NTex 自生的 `! Display math should end with $$.` 消失（参考 etrip.log 0 处）；`\box0` 从空 vbox+marks 变为与参考同构的 a/beginL/b/beginR/p/mathon/q/…/mathoff 链（度量仍差，thinmuskip 10.0 vs 4.99988）；终态同为既有内部错 `group_end 无配对 group_begin`（transcript 7628 → 8678 字节，走得更远） |
| latex.ltx --initex | 与 HEAD、与 f1c1503 worktree **逐字节一致**（5 错误、quark l.3782 → l.398/400 级联、pass1 OK、dumped=false——§24.3 状态原样） |
| `cargo test --workspace` | 除 ntex-pdf 既有失败外全绿 |

**f1c1503 判定**：与 math_display 回归无关。其改动在 `\if` 操作数位求值与
protected 数值扫描门，不在 `$$` 接线；本轮在 f1c1503 worktree（隔离重建）
复跑 latex_probe 与 HEAD/修复后逐字节一致，且 6 个 math_display 测试在含
f1c1503 的当前树全绿。主根唯一：360c342。

**既有失败（非本轮引入，均有 worktree 实证）**：
- `ntex-backend` clippy `-D warnings` 红：`chunks_exact_mut(4)` 常量块
  （raster.rs:58，`clippy::chunks_exact_to_as_chunks`）——HEAD 同红。
- `ntex-pdf` `write_pdf_embeds_multi_font_family_and_pages_use_own_fonts` 红：
  `/BaseFont /cmr10` 未按 Type1 惯例大写为 `/CMR10`（pdf.rs:454 断言）——
  HEAD 同红。

**遗留**：sink 的 `Mode::Math + display=true` 臂（"Missing $ inserted." + 收 +
重进）在两侧门都接线后已不可达（360c342 起即如此），留作恢复路径保险丝未删；
连带的 math 组生命周期 P0（§24.3 遗留）不受本轮影响。

## 26. 2026-09-05 第十九轮：`\romannumeral` 字母常量后继续展开——expl3 f 型展开落地，quark 区越过

主控简报锁定的 `Module quark, message name "invalid-function": Arguments 'test' and ''`
（expl3-code l.3782 `\__kernel_quark_new_test:N \__tl_if_recursion_tail_break:nN`）
只是一根表现层线头。本轮把 quark 条件生成链逐环 dump（`\cs_to_str:N` →
`\__quark_module_name:w` → `\__quark_module_name_loop:w` → `\__quark_module_name_end:w`）
后定位到**一个**引擎偏差，修复后 latex.ltx 载入从 expl3 **l.3782 → l.29550+**
（regex 模块 `\__regex_replacement_c_E:w` 区），invalid-function / Missing
endcsname / Argh bail out / Incomplete \ifx 四个错误全部消失。

### 26.1 根因：`\exp:w \exp_end_continue_f:w <stuff>` 的 f 型展开在引擎里是"惰性空转"

- `\exp:w` = `\tex_let:D \exp:w \tex_romannumeral:D`（expl3-code L1498），
  `\exp_end_continue_f:w` = protected 宏，体为 `` `^^@ `` ——即 `\romannumeral`
  的**字母常量**（char code 0）。l3expan 的权威注释："after a character code,
  TeX will still look for further digits, so full expansion continues until an
  unexpandable token is found"——即字母常量之后数字扫描**继续逐 token 展开**。
- 引擎在 [`scan_number_inner`] 里走完 `try_scan_backquote` 后直接
  `skip_trailing_spaces()` 收尾返回——字母常量成了扫描终点，`<stuff>` 一个
  token 都没展开。于是 `\__quark_module_name:N`（expl3-code L3505）的
  `\exp_last_unbraced:Nf \__quark_module_name:w { \cs_to_str:N ##1 } : \s__quark`
  把 **`\cs_to_str:N ##1` 以裸 token 列**交进 `##1` 的实参扫描：`##1`（`:`
  定界）只能落在显式 `:` 上取到整个名字、`_` 分割随之全部落空 →
  `\__quark_new_test_aux:Nn` 收到"原名"而非模块名 `tl` →
  `\csname q__<原名>_recursion_tail` → Missing endcsname → invalid-function。
- **实证链**（qp6/qp9 最小复现，`\write16` 逐环 dump）：修复前
  `W:base=[exp_after:wN]tail=[]`、`LOOP:arg=[exp_after:wN]`；修复后
  `W:base=[exp_after]tail=[wN]`、`LOOP:arg=[exp]`、`END:a=[exp]` ✓。

### 26.2 修复：`scan.rs` 字母常量臂改为"继续 get_x_token 展开到不可展开 token"

`try_scan_backquote` 成功后进入循环：宏/可展开原语 `expand_once` 后续扫、
条件 token `step_conditional`（`\fi`/`\else`/`\or` 在空条件栈上维持放回）、
不可展开 token 放回收尾，`skip_trailing_spaces()` 语义不变。**展开产物不折入
数值**（放回照常输出）——`\lccode`B=`b\the\lccode`B` = "0" 即 tex.web
`@<Scan an optional space@>`（`get_x_token; if cur_cmd<>spacer then
back_input`）的字面语义：get_x_token 展开 `\the`、back_input 只放回当前
token、此刻赋值未发生。与 `\exp_end:`（chardef 0 → 内部整数立即收尾不前瞻）
恰成 expl3 文档化的两态对照。

**测试翻正**：`lccode_assign_and_read` 第 3 断言 `"98"` → 新增空格变体
`\lccode`B=`b \the\lccode`B` = `"98"`（空格被吞、`\the` 赋值后才求值）+
无空格变体 = `"0"`（§26.2 语义），并附 tex.web 依据。

### 26.3 验证

| 门禁 | 结果 |
| --- | --- |
| ntex-core 全测 | **315 通过 / 0 失败**（同 §24.3 基线；1 断言按 §26.2 翻正） |
| TRIP | 与 HEAD（35282f9 worktree 隔离重建 + 独立 CARGO_TARGET_DIR）输出**逐字节一致**（仅 tempdir 路径与 systemd-run 包裹尾差）；pass2 终态硬错不变（`group_end 无配对 group_begin`） |
| ETRIP | 同上，与 HEAD **逐字节一致** |
| clippy/fmt（ntex-core） | `-D warnings` 绿、`fmt --check` 绿 |
| latex.ltx --initex | quark l.3782 区**越过**：invalid-function/Missing endcsname/Argh/Incomplete \ifx 全消；载入推进到 regex 模块 `\__regex_replacement_c_E:w`（expl3-code ≈l.29550）后被新阻塞点**致命**截断（见 26.4）；transcript 25 错（6 → 25 为**前进性增长**：原 6 错里 4 锚点已消，新错误全部来自新读入的 ~2.6 万行） |

### 26.4 新阻塞点（下一刀靶）

`\char_set_catcode_group_end:N \^^@`（expl3-code L29548，regex 模块用 char 0
当 catcode-2 组尾做 `\if_false: { \fi: ^^@` 平衡技巧）报 **Undefined control
sequence** → `\__regex_replacement_c_E:w` 的体扫描把 `^^@` 当普通 token、
吞到 EOF → 致命 `替换文本未闭合（缺少 }）`。疑点：`\char_set_catcode_*:N`
族定义在 L9173-9207（**早于**使用点），却在使用点 undefined——需查
L9173 定义是否真落地（`char_set_catcode:nn` 体含 `` `#1 `` 反引号+参数 token，
疑 [`try_scan_backquote`] 只认 Char/ControlSeq 不认 MacroParam）。连带未定义
清单：`\exp_args:NNcc`、`\exp_args:Nno`、`\c__tl_rescan_marker_tl`、
`\msg_expandable_error:nn`、`\char_set_catcode_math_subscript:N`。

**同轮顺手澄清（勿再当靶）**：`\write16` 的 token 打印**丢弃 cs token**
（NTex io.rs 只收集字符 token；tex.web 会打 `\csname`）且**丢弃组 token
花括号**——本轮所有 `\write16` 探针读数都受此影响，读数时须自行补回
`{...}`/`\cs` 形态；另 `\escapechar=-1` 未被 `\string` 尊重（仍打 `\`）。

## 27. 2026-09-05 第二十轮：unsave 的 retain 守卫缺失——"组内先局部触碰后 \global 赋值"全局定义被回滚

### 27.0 简报前提核伪

任务简报判定 l.19236 阻塞点 = `#{` 宏（`\declare@robustcommand@auxi#1#2#{` 等）
的**调用侧**死循环，并给出 hb7 极简复现（`\def\usepkg#1#{OK-BODY}` +
`\usepkg{opt}`）。实测证伪：

- hb7 在 HEAD（b9a9851 预编译二进制）上 **通过**（输出 OK-DONE，无循环）——
  十四刀落地的 hash_brace 存储语义 + 调用侧定界符匹配（`{` 作末参定界符，
  depth==0 时定界优先于整组贡献，§22 判定顺序）已对齐 tex.web。
- hb2/hb3/hb4/hb10 电池全部通过（hb3/hb4 残留 `\foo` undefined 与本轮无关）。
- TRIP L159/L161 用例不回退（318 测全绿，ntex-trip 4 测全绿）。

l.19236 的 `\declare@robustcommand@auxi` 级联是**下游症状**，真根因在
expl3-code.tex 更早处（见 27.1）。

### 27.1 真根因：save 恢复缺 "retaining" 守卫

**plain-TeX 层最小复现**（`/tmp/hb6/gsave.tex`）：

```tex
\begingroup
  \expandafter\let\csname GXX\endcsname\relax   % 局部赋值 → 压 save 条目
  \global\def\GXX{GLOBAL-VALUE}                 % 全局赋值 → 不压条目
\endgroup
\ifdefined\GXX \message{SURVIVED}\else\message{LOST}\fi   % 修前: LOST（真 TeX: SURVIVED）
```

tex.web 语义：eqtb 每槽带层级 eq_level；unsave 恢复时
`@<Store save_stack[save_ptr] in eqtb[p], unless eqtb[p] holds a global value@>`
——当前槽层级 = level_one（全局）→ **retaining**（丢弃陈旧 save 条目，保留组内
\global 赋值）；否则恢复旧值连同旧层级。NTex 无层级模型，restore 无条件覆盖：
组内"先局部触碰（压条目）→ 后 \global 赋值（不压）"的条目在组末把全局定义
回滚成触碰前的值。

**expl3 全线踩中**（save 条目多由 `\csname` 制造 relax 压入，TeX 2.9 起局部）：

1. expl3.ltx l.23-30：`\csname c__kernel_expl_date_tl\endcsname`（制造 relax）
   + `\global\let` 守卫——组末被回滚；
2. expl3-code l.3195-3248 变体生成器 `\__cs_tmp:w { nc }` 族：
   `\group_begin:` + `\cs_if_free:cT`（c 型展开的 `\csname` 制造 relax）+
   `\cs_gset:cpn`（全局）+ `\group_end:`——`\exp_args:Nno`/`:Nnc`/`:NNcc`
   等全部变体组末蒸发（l.3202 区 Undefined control sequence 级联，即 round 20
   遗留的"下一靶"）；
3. **致命下游**：`\char_set_catcode_group_end:N`/`\char_set_catcode_math_subscript:N`
   （expl3-code l.9175/9187 定义）因同类路径失效，使用点（l.25849/l.29535 regex
   与 tl_analysis 区的 `^^@` catcode 舞台）报未定义——`^^@` 停在
   `\char_set_catcode_group_begin:N` 置的 cat 1 未被复位 →
   `\cs_new_protected:Npn \__tl_analysis_a_egroup:w` 与
   `\__regex_replacement_c_E:w` 的定义体扫描靠 `^^@`(cat1) 隐形开组配平，
   多吞一层深度 → **体 runaway 吞穿 expl3-code.tex 到 latex.ltx**
   （transcript 错误行序 36605→39988→1361→1405→6844→18946→19236，即简报
   说的 l.19236 `#{` 级联——它是 runaway 体扫描吃到的第 N 个 `#`+`{`）。

### 27.2 修复：eq_level 最小两档化

tex.web 层级模型的最小落地（不引入全量 level 字段，只区分"当前值是否全局"）：

- `Eqtb` 增 `levels: Vec<u8>` 侧表（0=全局/底层组，1=组内局部；`.fmt` 载入后
  全 0——dump 后的槽语义上等同初表原语 level_one）；
- `SavedValue::Eqtb` 增 `prev_level`（tex.web 的 save 条目本就随槽值存层级）；
- 全部 cs 槽赋值点（\def/\gdef、set_slot_scoped、\let、\futurelet、\csname 制造
  relax、\openin/\openout 流、\font）压栈带 `prev_level` 并经 `eq_mark_level`
  登记层级；其中 **\futurelet 此前完全不压栈不登记**（tex.web \futurelet 走同一
  作用域 define 路径）——顺带修正；
- restore：当前层级==0 → retain（`\tracingrestores` 下输出 `{retaining …}`，
  对齐 tex.web restore_trace 的 retaining 分支）；否则恢复旧值连同旧层级。

语义自检（全部与 tex.web 一致）：组内 global 先于 local → 组末恢复全局值
（`local_after_global_in_group_restores_global_value`）；global 后于 local →
retain（新增 3 测）。315 旧测 + ntex-trip 全绿无回归。

### 27.3 修复后的新终态（下一靶）

修复使 expl3 变体/c 项目守卫真正生效，加载显著推进，但暴露**下一层引擎 bug**，
新终态为 fatal：

```
== pass1 ERROR: 输入栈超限（5001 帧 > 5000）
! Argument of \__str_case_end:nw has an extra }.  (l.8073 区)
```

错误级联自 expl3-code **l.5600-5700**（`\str_case`/`\__str_change_case` 区，
`\exp_last_unbraced:NNNNo` + `\cs_generate_variant:Nn \__str_change_case_output:nw { f }`
一带）以 `! Extra \fi.` 开始（修前该区静默通过——当时变体已被 27.1 缺陷整批
蒸发，`\cs_if_free` 走"已存在"跳过分支）。**此为修后新暴露的偏差**，非本轮
修复引入的回归（318 测 + hb 电池 + TRIP 全绿佐证）；f 型变体现在真正生成并
参与运行，其展开/扫描路径有待下一刀（round 15/16 曾在 `\exp_last_unbraced:NNNNo`
交过手）。错误上下文 `l.N` 行号在 Extra-\fi 级联区仍显示陈旧锚点行
（`error_anchor` 的一次性消费已修 undef 出口；其余扫描出口残留待清）。

### 27.4 同轮顺手修正

- **\futurelet 作用域**：此前不压 save 栈（局部 `\futurelet` 组末不回滚，且
  层级失登记破坏 retain 判定）——补齐压栈 + 登记（tex.web \futurelet 与 \let
  同一 define 路径）。
- **\global 前缀消费**：\futurelet 现按赋值语义消费 `\global` 前缀。
- **error_anchor 泄漏**：undefined-cs 出口消费锚点后不清除，污染后续所有
  `l.N` 上下文（27.1 排查中花费大量时间的"l.398 假行号"即此）——undef 出口
  补清除；其余扫描类出口（数字/维度扫描）的锚点残留仍在。

### 27.5 勘误与工具坑（继承记录）

- **hb7 复现脚本**：`/tmp/hb6/hb7.tex`；gsave 复现：`/tmp/hb6/gsave.tex`；
  `\ifx`/`\csname`/`\global` 电池：`/tmp/hb6/let1-6.tex`。
- **trim 探针法**：把 survey 树的 `expl3-code.tex` 换成 `head -N` + `\endinput`
  再跑 latex.ltx 探针，可精确定位"定义成功 vs 使用点失效"（27.1 判定
  `\char_set_catcode_group_end:N` 属后者的关键）；用后必须还原。
- **`\ifdefined` 与控制词**：组外 `_` 回到 cat 8 时 `\ifdefined\c__kt` 测的是
  `\c`（控制词遇非字母终止）——回归测试须把 `\catcode`\_=11` 放组外整行，
  否则误判引擎回归（本轮写测时自坑一次）。

## 28. 2026-09-06 第二十一轮：0 参数宏纯定界串在展开上下文漏匹配——`\__str_case:nw` extra-} 全线消除

### 28.1 阻塞点与症状

HEAD=09211e9（二十轮 retain 守卫）。latex.ltx --initex 探针终态：**7016 条错误**
（去重 7 类签名），末态死因 `输入栈超限（5001 帧 > 5000）`，级联核心在
expl3-code.tex l.7934 起 `\str_const:Ne \c_sys_engine_str` /
`\c_sys_engine_version_str`（内含 `\str_case:on`）执行区：

| 签名 | 条数 | 说明 |
|---|---|---|
| `Argument of \__str_case_end:nw has an extra }` | 3987 | str_case 收尾机器失衡 |
| `Argument of \__str_case:nw has an extra }` | 1992 | 同上，迭代级 |
| `Extra \fi` | 1002 | fast-form 条件体首 `\fi:` 泄漏 |
| `Missing endcsname inserted` / `Extra \endcsname` | 8/7 | `\csname` 名字扫描被污染 |
| `Missing number, treated as zero` | 18 | `\exp:w` f 型展开误吞数据 |
| `Undefined control sequence` | 2 | 既有 `^^J`（l.301/302，本轮外） |

### 28.2 根因：两处调用点对 0 参数宏跳过参数匹配

`\cs_if_exist:NT` 由 expl3 条件生成器产成（本引擎 `\show` 实测）：

```tex
\cs_if_exist:NT=macro:#1->\if_meaning:w#1\scan_stop:\use_i:nnnn\else:\fi:
  \if_cs_exist:N#1\__prg_T_true:w\fi:\use_none:n
```

尾段是 l3kernel fast form：`\__prg_T_true:w` 是 **0 参数宏 + 纯定界串参数文本**
`\def\__prg_T_true:w\fi:\use_none:n{\fi:\use:n}`——调用点必须匹配并**吞掉**
`\fi: \use_none:n`（tex.web macro_call `if info(r)<>end_match_token then
@<Scan the parameters@>`：参数文本非空时 0 参数宏同样走参数匹配，实参扫描用
get_token，`\fi` 在此是**数据**、不推进条件机），再由体首 `\fi:` 闭合
`\if_cs_exist:N` 帧。

引擎的执行路径（`call_macro`，expand/mod.rs）早已按此契约走 `collect_args`
（20 轮前修复），但**展开上下文的两个调用点**仍按 `num_params > 0` 分流，0 参数
直接 `Vec::new()` 跳过匹配：

- `expand_once` 的 `EqSlot::Macro` 臂（expr.rs）——`\edef` 体扫描
  （`scan_edef_body`）、`\romannumeral` f 型展开等一切"展开一次"位；
- `scan_csname` 的名字扫描展开臂（expr.rs）。

后果（`\edef\res{\cs_if_exist:NT\foo{YES}}` 级）：定界串 `\fi: \use_none:n`
滞留输入流 → 泄给条件机把 `\if_cs_exist:N` 帧提前弹掉 → 体首 `\fi:` 无帧可闭
→ `! Extra \fi.` + 实参错位一格；`\str_case` 的 case 列表逐格错位后连续组
`}{` 相撞 → `\__str_case(_end):nw extra }` 全线失衡、自持递归至栈超限。
`\csname` 位同理：定界串泄进名字文本 → `Missing endcsname inserted` 级联。

修复：两处一律走 `collect_args`（其 n==0 臂已按 tex.web 实现"纯定界串匹配 +
失配报 Use of \X doesn't match 并忽略调用"，空参数文本零开销）。

### 28.3 验证

| 门禁 | 前 | 后 |
|---|---|---|
| latex.ltx --initex 错误（transcript 全量） | 7016 | **1835** |
| 错误签名类数 | 7 | **3** |
| str_case 系（3 签名合计 6981） | 6981 | **0** |
| 终态阻塞点 | l.8073（栈超限） | **l.9365**（`\char_generate:nn` 区 `\if_case:w \tex_numexpr:D 13-#2`，`\ifnum` 关系符位遇 `+` 自持 → 步数超限） |
| `cargo test -p ntex-core` | 318 绿 | **319 绿**（新增回归测） |
| TRIP（ntex-trip --driver ntex） | 基线 | 与基线逐字节一致（仅 systemd unit id/临时目录/时计伪影） |

新增回归测 `zero_param_macro_delimiter_text_consumed_in_expansion_contexts`：
以 fast form 最小同构盖 `\edef`（真/假分支）与 `\csname` 名字扫描两个调用点；
在旧实现上实测失败（`""` vs `"YES"`），防回滚。

### 28.4 下一阻塞点（本轮未修）

l.9365 `\__char_generate_aux:nnw`（`\char_generate:nn` bootstrap，l.9341 起
`\int_step_function:nnN {0}{255} \__char_tmp:n` 每 iteration 约 7 条）：
`! Missing = inserted for \ifnum.`（关系符位读到 `+`，cat 12）×1812，
终态步数超限。与本轮同区（`\str_const:Ne` 执行区已过），性质另案；
另有 21 条 `Missing number`（l.7952 区 `\str_if_eq_p:Vn`/`\bool_if:nTF`，
本轮前已存在，条数 18→21 略增——str_case 链修复后走得更远所致）。

### 28.5 工具坑（继承 + 新增）

- **探针终态行号看 `Source(...,pos=N)` 帧**：watchdog/栈超限消息里的
  `Source(bytes,pos)` 对应嵌套文件字节偏移，`python3` 按字节计数换算行号——
  transcript 的 `l.NNN` 只反映报错时最内层文件行，二者常差 12 行（本层 patch
  后）。
- **二分 harness**：`/tmp/r21/w2/`（latex.ltx + expl3.ltx + expl3-code.tex +
  texsys.cfg 拷贝即自洽，SurveyVfs 以输入文件父目录为根）；`cp expl3-code.tex.orig
  expl3-code.tex` + python 按行号 patch 可做"执行级 bisect"（比 head -N 截断
  稳——expl3-code 截断点必须落在定义边界）。
- **最小复现的 `\fi:` 必须先 `\let\fi:\fi`**：expl3 catcode 下 `\fi:` 是独立
  控制词，直接写 `\fi:` 是未定义 cs——不 Aliasing 会让复现呈现"帧不闭合"
  假象（本轮 cond2/cond4 排查绕了一圈才定位到此）。
- **回归测的 catcode 行**：`\catcode`\_=11` 的反引号不可省；`run_transcript`
  断言 write16 消息、`expand()` 断言 token 输出，二者用途不同。

## 29. 2026-09-06 第二十二轮：表达式终结符前瞻 get_x_token 化——`\ifnum` 关系符位读到 `+`（l.9365）1812 条清零

### 29.1 阻塞点与症状

HEAD=ba73a66（二十一轮 0 参数纯定界串匹配）。latex.ltx --initex 探针终态
**1835 条错误**：

| 签名 | 条数 | 说明 |
|---|---|---|
| `Missing = inserted for \ifnum` | 1812 | 关系符位读到 `+`（cat 12）——本轮靶子 |
| `Missing number, treated as zero` | 21 | l.7952 区 `\str_if_eq_p:Vn`（二十轮前已有） |
| `Undefined control sequence` | 2 | 既有 `^^J`（l.301/302，本轮外） |

1812 条**全部无 `l.NNN` 上下文行**（错误发生在宏展开产物里而非源行），且
`Missing =` 后紧跟的 `<to be read again>` 恒为 `Char(cat=Other,ch=43)`——高度
规律 = 自持循环（终态死因 `处理步骤超限`，步数帽打满）。位置：expl3-code.tex
l.9364 `\int_step_function:nnN { 0 } { 255 } \__char_tmp:n`（`\char_generate:nn`
bootstrap，l.9341 起），transcript 的 `l.NNN` 与真实行号差 ~12。

### 29.2 定位路径（三层收敛，各一步到位）

1. **harness 复现**：`/tmp/r21/w2`（latex.ltx + expl3.ltx + expl3-code.tex +
   texsys.cfg）+ `patch1.sh`（把 l.7934-7946 `\str_const:Ne \c_sys_engine_str`
   块替换为 `\str_const:Nn …{ pdftex }`）+ **release** 二进制 → 1835 精确复现。
   陷阱：`target/debug/examples/latex_probe`（23:52）早于二十一轮提交，跑出来是
   7016/2537 的旧签名；主控基线用 `--release`。
2. **行级 patch bisect**（改 expl3-code.tex 单行 + 重跑，~0.3 s/次）：

   | patch | 结果 | 结论 |
   |---|---|---|
   | `{0}{255}`→注释 | `Missing =` 消失 | 循环是唯一来源 |
   | `{0}{255}`→`{0}{3}` | 仍 1812 | 一次迭代即自持（非逐迭代累加） |
   | 去掉 `#5{#2}`（l.7048） | 错误**变多**（4976） | 与 `\__char_tmp:n` 无关 |
   | 去掉尾递归（l.7049-7051） | `Missing =` 消失 | 污染在尾递归的值构造 |

3. **输入栈帧转储**：临时在 `scan_relation` 的 "Missing =" 臂挂
   `NTEX_IFNUM_TRACE` 转储（含 Bytecode 帧按 tag≤3 解码）。首个失败点栈：

   ```
   #14 BC(pc=2/31)  \__int_step:Nw 体首（\if_int_compare:w #2 #1 #4 \exp_stop_f:）
   #15 ARG(7 tok)   '0' \exp_after:wN '+' \int_value:w \__int_eval:w \c_one_int \exp_after:wN
   ```

   `\__int_step:Nw` 的 `#2`（当前值）= **setup 阶段残留的裸 token**
   （`0 \expandafter + \number\numexpr \c_one_int \expandafter`），不是数字。

### 29.3 根因：`\numexpr` 终结符前瞻用了 get_token 而非 get_x_token

expl3 的计数器传值习语（l.7017 起 `\int_step_function:nnnN`）：

```tex
\cs_new:Npn \int_step_function:nnnN #1#2#3
  {
    \exp_after:wN \__int_step:w
    \int_value:w \__int_eval:w #1 \exp_after:wN \__int_sep:
    ...
```

`\__int_sep:` 是 `\let` 别名（`=\__kernel_int_sep:=\tex_let:D`），作**定界符**。
真 TeX 链路：`\exp_after:wN` 的 B 是 `\int_value:w`（convert，`cur_cmd>max_command`
→ expand），`conv_toks`→`scan_int`→`scan_expr`；`scan_expr` 的**运算符前瞻**是
`get_x_token`——`\expandafter`（可展开）被就地展开一次，产物首 token
`\__int_sep:` 判为终结符 → `back_input`。最终流是
`\__int_step:w 0 \__int_sep: 1 \__int_sep: 255 \__int_sep: \__char_tmp:n`。

tex.web 依据：§7697 `\expandafter` 实现
（`get_token; t:=cur_tok; get_token; if cur_cmd>max_command then expand else back_input; cur_tok:=t; back_input;`）
与 §8707 `scan_int` 数字累计循环
（`loop … else goto done; … get_x_token; end; done: if cur_cmd<>spacer then back_input;`）——
**数值/表达式扫描的每个前瞻位都是 get_x_token 位**。

NTex 的 `peek_int_op`（expr.rs）用裸 `fetch()`：`\expandafter` 原样放回 →
`\__int_step:w` 的定界实参扫描把 `\expandafter`+下一值整段吞进 `#1` → 下一值
没被求值就进了实参 → 下一迭代 `\if_int_compare:w #2 #1 #4` 的左操作数是
`\number\numexpr 0` + 关系符位读到 `+ #3` 的 `+` → 1812 条自持循环。

### 29.4 修复（机制层，两处）

`expr.rs::peek_int_op`：

1. **get_x_token 臂**（本轮主修）：循环内 fetch 后，`EqSlot::Macro`（非
   protected 抑制面）/`Primitive::is_expandable()` 展开一次、产物压回、`continue`
   重探；`\noexpand` 冻结 token 不展开；条件原语不在此臂（与数字循环同限：
   游离 `\fi` 属外层条件，放回——`scan.rs` 数字循环第十二刀注释同款约束）。
   同函数被 `\dimexpr`/`\glueexpr`/`\muexpr` 的运算符循环复用，一并修正。
2. **`\relax` 吸收限加法层**（`absorb_relax: bool` 参数，回归门逼出来的第二处）：
   本引擎乘/加两级各做一次前瞻（etex.web scan_expr 是单层循环单次前瞻）。若
   乘法层也吸收 `\relax`，加法层的第二次前瞻就越过表达式终点，把**外侧**
   token 展开吞进表达式：`\skip0=\glueexpr 1pt \relax\the\skip0` 里 `\the\skip0`
   被前瞻吞掉，产物 `0pt` 泄漏为排版文本 → `glueexpr_in_skip_assignment` 失败
   （319 绿 → 1 红）。现乘法层（`expr_mul_term`/`glue_expr_mul_term`）传 false、
   加法层（`eval_int/dimen/glue_expression`）传 true。

### 29.5 验证

| 门 | 结果 |
|---|---|
| latex_probe --initex（+patch1） | **1835 → 105**（1812 条清零），阻塞点 l.9365 → **l.10298** |
| `cargo test -p ntex-core` | **321 绿**（319 既有 + 2 新增回归测） |
| `cargo run -p ntex-trip -- --driver ntex --test trip` | 与二十一轮基线**逐字节一致**（差异仅 unit id/临时目录/时计） |

新增回归测：`expr_terminator_peek_expands_expandable_token`（原语级复刻
`\int_step` 链：`\sep`=`\let` 别名定界 + `\expandafter\number\numexpr` 尾递归，
断言 `[0][1][2][3]B`；另含 `\let` 别名作终结符、可展开宏作终结符两护栏）与
`expr_relax_absorbed_once_at_add_level`（`\relax` 单次吸收/泄漏护栏）。

### 29.6 下一阻塞点与残留

l.10298 区（expl3-code l.10232-10298，`\__prop_pair:wn` / `\tl_set:Nn #3`
prop 值哨兵区），终态死因 `输入栈超限（5001 帧）`。105 条签名：
`Missing number, treated as zero` 48、`Argument of \use_ii:nn has an extra }` 32、
`Undefined control sequence` 6、`Illegal parameter number in definition of` 3、
`Extra \fi` 3、`use_i:nn` extra-} 3、`Missing endcsname`/`use_none:nnnn`/`use_ii:nnn`
各 2、`Too many }'s`/`Missing = for \ifnum`/`Missing )`/`Arithmetic overflow` 各 1。
`\exp_last_unbraced:NNNNo` / prop 模块（第十五刀遗留靶）在此区。

### 29.7 本轮踩坑

- **陈旧 debug 二进制**：`target/debug/examples/*` 与 release 产物时间戳可差数
  小时，跨轮跑探针先 `ls -la` 对提交时间，否则签名全错（本轮 7016/2537 假象）。
- **探针转录只收 write16/错误**：typeset 字符（如条件真分支的 `B`）不进
  transcript——用探针做行为对照时必须走 `\message`，否则会误判"token 被吞"
  （本轮绕了三步假线索）。
- **`--initex` 模式缺 `\message` 原语**：INITEX eqtb 缺项（`{A}` 被当排版文本），
  plain 模式正常——独立缺口，未在本轮修。
- **Rust `concat!` 里的 `%`**：无真实换行时注释吞到 EOF（repo 已有注释告诫，
  tests.rs `nested_param_in_macro_arg_repro`）；本轮新测全部 `%\n`。

## 30. 2026-09-06 第二十三轮：`\ifx` 漏掉 `\noexpand` 替换臂——expl3 V 变体族首个根因落地

### 30.1 起点与简报靶子证伪

HEAD=0095edd，探针终态 **105 条错误**（本轮复测逐签名一致，见 §29.6）。简报三靶
复核结果：

1. **`\c_max_intarrray_int` "拼接" 假说证伪**：expl3-code.tex l.7473 上游原文就是
   `\int_const:Nn \c_max_intarrray_int { 1 073 741 823 }`（l3array 专用上限常量，
   `/tmp/latexsurvey` 副本同行同文），日志行 `\c_max_intarrray_int=\count25` 是
   `\int_new:N`（值 > `\c__int_max_constdef_int` 走 newcount 分支）的正常分配
   打印——**无任何引擎缺陷**。
2. `\exp_last_unbraced:NNNNo` 族（32 条 extra-}）：非独立根因，见 30.4。
3. `\c__prop_basis_int` undefined：级联——l.10008 `\int_const:Nn \c__prop_basis_int
   { \c_max_char_int - ``! }` 执行时数字扫描已带病，常量未建成（transcript
   `\c__prop_basis_int` 3 处 undefined + l.10024 `\char_generate:nn { ``\! + #1 }`
   区 5 条 Missing number）。

### 30.2 定位路径（write16 探针 + 输入栈帧转储）

- 新增探针临时插桩（已随本轮移除）：`report_missing_number` 挂
  `NTEX_MISSING_NUM_TRACE`，dump `debug_stack_summary()` + 最深 4 帧的
  `pos` 与后续 10 个 token（cs 名解码）。一轮跑出全部 48 条 Missing number 的
  失败点栈，比逐条 patch bisect 快一个量级。
- `Source(1387070B,pos=N)` 字节偏移换行号（§28.5 方法）给出精确源行：
  l.7952（`\__sys_const:nn`×`\str_if_eq_p:Vn`，12 条）、l.7959/8016/8073
  （sys 同族 9 条）、l.9468-9645（token 模块 20 条）、l.10018/10024/10232
  （prop 8 条）。
- **最小探针对照**（在 harness 里插 `\immediate\write16`，l.7948 前）：

  | 探针 | 结果 | 结论 |
  |---|---|---|
  `\tex_strcmp:D {pdftex}{pdftex}` | `0` | strcmp 正常 |
  `\if:w 0 \tex_strcmp:D {…}{…}` | `EQ` | `\if` + strcmp 正常 |
  `\if_predicate:w \str_if_eq_p:Vn \c_sys_engine_str {pdftex}` | `FALSE`（应 TRUE） | `\ifodd` 操作数链断 |
  `\bool_if:nTF {\str_if_eq_p:Vn …} TRUE\else FALSE\fi` | `RUEFALSE`（应 TRUE） | 分支选择崩坏 |
  `\exp_args:NV \use:n \c_sys_engine_str` | `! You can't use \the with this.` | **直接暴露根因** |

### 30.3 根因：`\ifx` 比较丢了 `\noexpand` 替换语义

`\str_if_eq_p:Vn` = `\exp_args:NV \str_if_eq_p:nn`，其 V 展开核心是 l3expan
`\__exp_eval_register:N`（expl3-code l.2533）：

```tex
\exp_after:wN \if_meaning:w \exp_not:N #1 #1   % \ifx\noexpand#1 #1：判"宏还是寄存器"
  \if_meaning:w \scan_stop: #1 \__exp_eval_error_msg:w \fi:
\else:
  \exp_after:wN \use_i_ii:nn                    % 宏臂：\use_i_ii:nnn #1#2#3 → #1#2 摘掉 \the
\fi:
\exp_after:wN \exp_end: \tex_the:D #1           % 寄存器臂：\the 取值，\exp_end:(chardef 0) 终止 \exp:w
```

tex.web 语义（l.7506-7516，`no_expand_flag=257`）：`\noexpand` 只是往输入里插
`frozen_dont_expand` 标记；该标记被读到时 `if cur_cmd>max_command then
(cur_cmd,cur_chr):=(relax,257)`——**可展开 cs（宏/可展开原语）的临时含义是
(relax,257)**，真实 `\relax` 的 cur_chr 是 eqtb 指针永不为 257，故这是一个独立
含义键；不可展开 cs / 字符 token 含义原样保留。因此 `\ifx\noexpand#1#1`
对宏为**假**（走 `\else` 宏臂）、对寄存器为**真**（走 `\the` 臂）。

NTex 的 `CondOp::IfX` 用 `fetch().0` 取操作数，把 `noexpand` 标记**整枚丢弃**，
`ifx_equal` 直接比含义键 → 宏被判成"寄存器" → `\the\c_sys_engine_str` →
`! You can't use \the with this.` → 数字扫描带病继续 → 每个使用 V/v 变体的
模块级联报 Missing number / extra-}。

（`\if`/`\ifcat` 的 `\noexpand`→active char 臂 cond.rs 早已实现（tex.web
`get_x_token_or_active_char`），只有 `\ifx` 缺。）

### 30.4 修复（机制层两处，均 tex.web 直译）

`cond.rs`：

1. `MeaningKey` 新增 `NoExpandRelax` 变体（mod.rs，注释引 tex.web l.7506）；
2. `ifx_equal(t1, ne1, t2, ne2)` 带上 noexpand 标记，经新助手 `ifx_meaning`：
   标记 + ControlSeq 且槽位可展开（`Macro` / `Primitive::is_expandable()`，
   即 tex.web `cur_cmd>max_command`）→ `NoExpandRelax`；否则走原 `meaning_key`。
   两侧对称（`\ifx\noexpand\A\noexpand\A` 两键同为 NoExpandRelax → 真，同 tex.web）。

`\if`/`\ifcat` 路径与字符 token 完全不受影响；`noexpand=false` 时行为逐位不变。

### 30.5 验证

| 门 | 结果 |
|---|---|
| latex_probe --initex（/tmp/r21/w2，无 patch1） | **105 → 97**（l.7952 区 12→6、l.8016 4→2 等 8 条清零） |
| `cargo test -p ntex-core` | **323 绿**（321 既有 + 2 新增回归测） |
| TRIP | 见 30.7 说明（本修复的 delta 路径在 trip.tex 不可达） |

新增回归测：`ifx_noexpand_macro_is_unequal`（宏/`\let` 到 relax/可展开原语/
不可展开原语四臂 + 无 `\noexpand` 对照）、`expl3_v_variant_macro_value_via_expandafter_ifx`
（`\__exp_eval_register:N` 机制级复刻，断言宏就地展开且 `\the` 被摘除）。

### 30.6 下一阻塞点：`\prg_return_true:` 的 romannumeral 新语义（本轮证据链已备好）

剩余 97 条的**主根因**已定位到一半：2026 版 l3kernel 把
`\prg_return_true:` / `\prg_return_false:` 从 chardef（`\c_true_bool`/`\c_false_bool`）
改成了 romannumeral 技巧（expl3-code l.1673-1676）：

```tex
\cs_gset:Npn \prg_return_true:  { \exp_after:wN \use_i:nn  \exp:w }
\cs_gset:Npn \prg_return_false: { \exp_after:wN \use_ii:nn \exp:w }
```

即条件体的 `\if:w <test> \prg_return_true: \else: \prg_return_false: \fi:` 里，
`\prg_return_*:` 自带一个 `\exp:w`（romannumeral）：数字扫描先吞 `\else`（条件机
翻 Skipping）、跳过假分支、在 `\fi` 弹帧，再在 `\exp_end:`（chardef 0，**数字扫描
中段**的内部量）取 0 终止，随后 `\use_i:nn`/`\use_ii:nn` 在剩余 token 里挑真/假
分支。配套生成器 `\__prg_T_true:w \fi: \use_none:n` / `\__prg_F_true:w \fi: \use:n` /
`\__prg_TF_true:w \fi: \use_ii:nn` / `\__prg_p_true:w \fi: \c_false_bool`
（l.1819-1823，帧转储里 `[\fi: \use_none:n]`/`[\fi: \use:n]` 即其定界串）。

本轮帧转储显示 `\ifodd`（`\if_predicate:w`）操作数数字扫描在此链报 Missing number
（read-again `\use:n`）——与 §29 的 peek_int_op 同族：**数字扫描的
"中段内部量 + 条件机步进" 组合臂缺**（tex.web scan_int 的
`min_internal<=cur_cmd<=max_internal → scan_something_internal` 分支只在
首 token 位实现（scan.rs peek_csid 的 `EqSlot::MathChar` 等），经 `\else`/`\fi`
步进后才到达的内部量落在十进制数字循环（scan.rs ~L560）里被当终结符）。
修复方向：数字循环在 `!any` 时遇到内部量（MathChar/Register/\the 族）按 tex.web
直接取值返回；或让符号循环（scan.rs ~L100）对 `\else`/`\fi`/`\or` 按 get_x_token
继续步进（须保住 TRIP L82 十六进制循环"游离 `\fi` 放回"契约，见 scan.rs
maybe_eval_cond 注释）。

### 30.7 TRIP 门禁说明（诚实记录）

`cargo run -p ntex-trip -- --driver ntex --test trip` 本轮报
`TRIP：失败（驱动 ntex）`，pass2 终态 `组未闭合（缺少 }）groups=[SemiSimple,
MathLeft, MathLeft, Align]`——**HEAD=0095edd 基线对拍逐字节一致**（`git worktree
add /tmp/r23/head 0095edd` + 独立 `CARGO_TARGET_DIR` 冷构建，diff 仅 cargo
banner 两行；共享 target 目录会交付陈旧二进制，见 §22）。即该失败是 HEAD 既有
状态，非本轮引入。辅助论证：delta 路径只在 "`\ifx` 操作数带 noexpand 标记"时
可达，`fixtures/trip/trip.tex` 全部 7 处 `\ifx`（l.11/389/390/400/401/405/417）
均无 `\noexpand` 前缀操作数（l.405 的 `\expandafter` 展开的是 `\csname`，
无标记；l.436 的 `\expandafter\noexpand\dol` 落在 `\if$` 字符比较臂，本修复
未触碰该臂）。

### 30.8 本轮踩坑

- **write16 探针会污染错误计数**：插 `\immediate\write16{…\cs_split_function:N …}`
  后错误 97→30（探针表达式自身的错误恢复改变了后续状态）——探针只能用于
  定位，不能当计数基线；每轮插桩后必须 `cp expl3-code.tex.orig expl3-code.tex`
  再复测终态。
- **`\write16` 打印 `{…}` 组不可见**：`expand_to_string` 只收 cat 10/11/12，
  `{base}{signature}` 印成 `basesignature`——第一眼会误判"冒号丢了"
  （本轮在 `\cs_split_function:N` 上绕了三步）。
- **裸 `\ifx\noexpand\A\A` 在真实 TeX 里也是假**：第一操作数是 `\noexpand`
  原语本身（get_next 不展开）——expl3 惯用法必须带 `\expandafter`
  （`\expandafter\ifx\noexpand#1#1`）。写回归测时若漏掉 `\expandafter`
  会把"修复无效"误判出来。

## 31. 2026-09-06 第二十四轮：数字扫描符号循环的 `\else`/`\fi`/`\or` 条件机推进——`\cs_to_str:N` 族首个根因落地

### 31.1 起点与靶子

HEAD=1ac7dd9，探针终态 **97**（40 Missing number + 32 `\use_ii:nn` extra-} +
6 Undefined + 其余零头）。本轮靶子 = §30.6 留下的 `\prg_return_true:`/`\false:`
romannumeral 新语义（"数字扫描中段内部量"）。

### 31.2 定位：`NTEX_MISSING_NUM_TRACE` 帧转储（Rust 侧，零 tex 探针）

在 `report_missing_number` 挂 env 开关转储最深 6 帧的 pos 后 token（**直接读帧
内切片，不消费**——比 §30.2 的 fetch-回放法干净，且不碰 expl3-code.tex，无
§30.8 探针污染问题），再在 scan_number_inner 四个循环出口（符号/反引号尾随/
radix/十进制）挂出口标签。一轮跑出 97 条的完整失败链：

```
[num-scan] backquote-trail stop at \c_false_bool   ← 最内层：romannumeral f-前瞻（正确）
[num-scan] sign-loop stop at \fi:                  ← 中层：\number 谓词扫描符号循环
[num-scan] decimal-loop stop at \use:n             ← 外层：十进制循环错位弹帧后撞上 \use:n
[missing-num] #0 \use:n … #3 { \__bool_if_p_aux:w } \group_align_safe_begin: …（\__bool_if_p:n 体耗尽）
```

**根因**：符号循环只经 `maybe_eval_cond` 处理条件**开始**（`\if*`）；
`\else`/`\fi`/`\or` 被当普通终结符放回 → 十进制数字循环随后拾起并以
`cond_stack 非空` 为由 `step_conditional` **错位弹帧**（弹掉的是外层条件帧）→
扫描撞上 `\use:n` → Missing number + `\__prg_*_true:w` 定界串失衡 →
`\use_ii:nn` extra-} 级联。

tex.web 依据：`expand` 的 fi_or_else 臂（§9897
`@<Terminate the current conditional and skip to \fi@>`）——get_x_token 对
`\else`/`\fi`/`\or` **同样走 expand**：栈顶帧 `if_limit=if_code`（本引擎
`CondState::Evaluating`，外层条件操作数扫描中）→ `insert_relax`（token 放回
+ 前插 frozen `\relax`，本扫描按 Missing number 收场、`\fi` 留给外层条件机）；
帧已完成求值 → skip/弹帧。

### 31.3 修复（scan.rs 符号循环一处，cond.rs 零改动）

`maybe_eval_cond` 之后补 Fi/Else/Or 臂：`cond_stack` 非空 → `step_conditional`
（insert_relax 门/跳过/弹帧全是 cond.rs 既有机器）；`cond_stack` 空 → 维持旧
"放回 + Missing number"（游终结符，TRIP L82 契约）。radix（`"`/`'`）循环
**未触碰**（L82 的 `\fi` 放回在 radix 循环，见 §22/§29）；十进制循环既有
Fi/Else/Or 臂不动。

依赖此语义的真实 l3kernel 样例 = expl3-code **l.1888-1893 `\cs_to_str:N`**
（`\romannumeral \if:w … \__cs_to_str:w \fi: \exp_after:wN \__cs_to_str:N
\token_to_str:N`，`\__cs_to_str:w` 体 `- \int_value:w \fi: \exp_after:wN
\c_zero_int`）：`\if:w` 真臂里 `\number` 的嵌套数字扫描在**符号循环**遇
`\fi:`——弹掉 `\if:w` 帧后须继续到 `\expandafter`/`\c_zero_int`（首 token 位
内部量）取 0 终止，随后 `\number` 产物 `0` 作外层数字、`\string` 产物全名
留在流中（`\cs_to_str:N\abc` → `abc`）。

### 31.4 验证

| 门 | 结果 |
|---|---|
| latex_probe --initex（/tmp/r21/w2，无 patch1） | **97 → 87**（Missing number 40→28；l.9468-9530 token 区每点 2→1） |
| `cargo test -p ntex-core` | **325 绿**（323 既有 + 2 新增） |
| TRIP | `组未闭合（缺少 }）groups=[SemiSimple, MathLeft, MathLeft, Align]`——与 HEAD 基线对拍一致（worktree + 独立 CARGO_TARGET_DIR 冷构建，见 §30.7 方法） |

新增回归测：`number_scan_sign_loop_steps_fi_of_completed_frame`（`\cs_to_str:N`
机制级复刻，断言无 Missing number / 无 Extra \fi；**最小复现必须
`\let\if:w\if`**——expl3 别名在 initex 初表未注册，漏了会呈现"修复无效"假象）、
`number_scan_sign_loop_keeps_free_fi_for_outer_machine`（游离 `\fi` 契约锁）。

### 31.5 剩余 87 条的定性（下一轮靶子）

- **34 `\use_ii:nn` extra-} + 28 Missing number**：与 §30.6 假设的"中段**内部量**"
  不同——帧转储显示十进制循环撞的是 `\use:n`/`}`，属**生成 p-form 真臂**
  （`<test> \exp_end: \c_true_bool \c_false_bool`）的**真分支侧**：`\number`
  取走 `\exp_end:`（0）后 `\c_true_bool \c_false_bool` 残留在流，
  `\__bool_choose:NNN` 的 `#3`（应为 `)`/`&`/`|`）拿到 bool 常量 →
  `\use:c{__bool_<名>_…}` 造名失衡 → 2 条 `Missing endcsname inserted`
  与 extra-} 同链。假分支侧（`\c_false_bool` 直接作数）本已自洽。
  tex.web 口径下 `\exp_end:`（chardef 0）是首 token 位内部量、值恒 0——
  2026 l3kernel 若真依赖"真臂取 1"，则其自洽性必须在**别处**闭合
  （候选：`\number` 前的 romannumeral f-前瞻截停位、或 `\__bool_p:Nw` 的
  `\int_value:w` 展开时序），下一轮先对拍真实 TeX 的 `\showthe\numexpr` 级
  最小样例再动手。
- **6 Undefined control sequence**：l.301/302（`^^J`，既有）+ l.10024 区 3 条
  （prop 模块级联，随上链消除）。
- **3 Illegal parameter number / 3 Extra \fi / l.398 Missing )**：既有零头。

### 31.6 本轮踩坑

- **`expand()` 双轨断言会掩盖错误恢复差异**：`run_transcript`（断言 write16
  消息）与 `expand`（断言 token 输出）必须并用——`\cs_to_str:N` 复刻在
  修复前后**输出相同**（都是 `abc`），差异只在 Missing number 消息。
- **帧转储直接读帧内切片**（`InputFrame` 各变体的 `pos` 后 slice）比 fetch-
  回放法安全：fetch 会推进 pos / 触发惰性跳过，转储本身改变状态。
- **worktree 冷构建对拍**：`git worktree add` 后 fixtures 不随迁
  （fixtures/ 在 .gitignore 之外但未入库的子目录需手拷，§16 有前科）。

## 32. 2026-09-06 第二十五轮：数字扫描跳过区臂 + e-TeX 表达式三处机制缺口——主簇根因改判为表达式扫描

### 32.1 起点与简报靶子的改判

HEAD=08a97c6，探针终态 **87**（34 `\use_ii:nn` extra-} + 28 Missing number +
2 Missing endcsname 为主簇）。简报靶 = §31.5 留下的 p-form 真臂时序
（`\number` 取走 `\exp_end:`(0) 后 `\c_true_bool \c_false_bool` 残留）。

本轮先对拍真实机制（VM 无真实 TeX，取 tex.web 本地拷贝 + GitHub 拉 l3basics.dtx/
l3prg.dtx 原文），把生成条件体的两条臂拆清（l3basics `\@@_generate_*_form`）：

- **fast 臂**（`\@@_generate_conditional_fast:nw` 命中、`#8 = \use_i:nn`）：
  体 = `<test> \__prg_p_true:w \fi: \c_false_bool` → 展开为 `\fi: \c_true_bool`，
  由体首 `\fi:` 闭合条件帧、bool 常量作值。**本引擎此前已对**（mech 复刻
  FAST-TRUE=1/FAST-FALSE=0）。
- **normal 臂**（`#8 = \use_i_ii:nnn`，test 不以
  `\prg_return_true: \else: \prg_return_false: \fi:` 收尾或含嵌套时）：
  体 = `<test> \prg_return_true: \else: \prg_return_false: \fi: \exp_end:
  \c_true_bool \c_false_bool`——`\exp_end:` 是给 `\prg_return_*:` 的 `\exp:w`
  收口用的**哨兵数**，其后两个 bool 常量是 `\use_i:nn`/`\use_ii:nn` 的两个
  实参（l3basics 原文："the logic-returning functions expect two arguments to
  be present after `\exp_end:`"）。真臂 → `\c_true_bool`(chardef 1)，
  假臂 → `\c_false_bool`(chardef 0)。

§31.5"真臂取 1 须在别处闭合"的疑问就此闭合：自洽性在 **`\else:`/`\fi:` 由
`\exp:w` 的数字扫描展开消费**（即第 24 轮符号循环 Fi/Else/Or 臂的同一机制）。

### 32.2 本轮四处机制修复（全部 tex.web 直译）

| # | 位置 | tex.web 依据 | 复现 |
|---|---|---|---|
| 1 | `scan.rs` `scan_number_inner` 符号循环头部补 `is_skipping` 臂 | fi_or_else 臂在 expand 内**同步** `pass_text` 到 `\fi` 后弹帧（§9897），假分支 token 到不了数字扫描的取 token 位；本引擎惰性 Skipping 帧由各取 token 位自行问 `is_skipping()`（十进制循环已有同款臂） | `\number\ifnum1=1\prg_return_true:\else\prg_return_false:\fi\exp_end:\c_true_bool\c_false_bool` 旧 0 → 新 1 |
| 2 | `expr.rs` 新增 `expr_peek_factor_token`，四处因子位（int/dimen/glue/number）改用 | etex.web `scan_expr` 取 token 循环同为 get_x_token——`( <expr> )` 因子臂须对**展开产物**判定 | `\numexpr\paren*4`（`\def\paren{(2+3)}`）旧 `0(2+3)*4` → 新 20 |
| 3 | `peek_int_op` 头部补 `skip_spaces` | 因子后的空格由 scan_int 的 trailing-space 规则吸收，`) / 2` 的空格不得充当终结符 | `\__int_div_truncate:NwNw` 体含空格：`(140-(100-1)/2)` 旧 41+残留 `/ 2)` → 新正确 |
| 4 | `expand_once` 的 `EqSlot::Alias` 臂改为**沿链解引用后按目标含义展开** | tex.web `\let` 在 eqtb 层复制含义（别名即原义）；expl3 全篇 `\cs_new_eq:NN` 两级别名链（`\__int_eval:w → \tex_numexpr:D → \numexpr`、`\__int_sep: → \tex_let:D → \let`） | `\expandafter X \int_value:w` 的 `\number` 须就地求值 |

另在 `peek_int_op` 补条件开始（`\if*`）就地求值臂（同 get_x_token → expand →
conditional()）；`\else`/`\fi`/`\or` 仍维持放回（TRIP L82 游离 `\fi` 契约）。

### 32.3 主簇根因改判（下一轮靶，证据链已闭合）

probe 终态 86 的**主簇不是独立的真臂时序问题，而是 l.8073 一处表达式失真的
下游级联**。链条（转录顺序 = 实际执行顺序）：

1. **l.8023-8073 `\str_const:Ne \c_sys_engine_version_str`**：pdftex 分支的
   `\int_div_truncate:nn {\tex_pdftexversion:D}{100}`（l.6652）→
   `\__int_div_truncate:NwNw` 的体（l.6660-6672）在 e-TeX 表达式里嵌
   `\if_meaning:w`（`#1#2 \if_meaning:w - #1 + \else: - \fi: (...) / 2`）。
   实测该常量被定义成 `140(100-1)/2)/100\__int_eval_end:.0(...)*100\__int_eval_end:
   .\tex_pdftexversion:D`（应为 `1.40.25`）→ 2× "Missing number(`(`)" +
   3× "Missing ) inserted for expression"。
2. **l.9386 `\tl_const:Ne \c_catcode_other_space_tl {\char_generate:nn ...}`**：
   "Illegal parameter number in definition of" + "Too many }'s"——组/参数扫描
   失衡沿失衡态传播。
3. **l.9468-9530 `\token_if_*` 生成条件区**：34× `\use_ii:nn` extra-} +
   27× Missing number + 2× Missing endcsname（即 §31.5 的主簇本体）。

**机制级最小复现已锁**（/tmp/r25/mech19.tex，与真实 expl3 同构：
2 级 `\let` 别名链 + `\__int_sep:`=`\let` + 体含 `\if_meaning:w`）：

```tex
\let\texnumD\number \let\iv\texnumD   % \int_value:w 同构
\let\texnumE\numexpr \let\ev\texnumE  % \__int_eval:w 同构
\let\texlet\let \let\sep\texlet       % \__int_sep: = \let（不是 \relax！）
\long\def\auxb#1#2\sep#3#4\sep{\if_meaning:w0#1 0\else:(#1#2\if_meaning:w-#1+
  \else:-\fi:(\if_meaning:w-#3-\fi:#3#4-1)/2)\fi:/#3#4}
\edef\ga{\iv\ev\expandafter\auxb \iv\ev 140 \expandafter\sep \iv\ev 100 \sep \eend}
```

本引擎 → `140(100-1)/2)/100`（应 1）。剩余待解的两处（已定位到行）：

- **数字扫描内部量分派不 deref 别名**：`scan_number_inner` 的大 match 无
  `EqSlot::Alias` 臂（`_ => {}` 落空）→ `\number \ev 140` 报 Missing number 取 0。
  tex.web 口径别名即原义，分派前应沿链解引用再进 `scan_something_internal`。
- **表达式因子/运算符位对 `\if_meaning:w` 的求值时序**：第 2、4 处修复后仍差
  一步——`\expandafter\auxb` 展开后 `\auxb` 的实参扫描把未展开的
  `\if_meaning:w`-序列当数据吞进 `#2`，条件机的帧归属随之错位。tex.web 侧
  `\ifx` 操作数用 `get_token`（不展开，数据语义）而 `\if_meaning:w` 自身在
  expand 位被消费——需按"条件在 expand 位消费、其操作数是数据"重排
  `peek_int_op`/`expr_peek_factor_token` 的消费顺序。

### 32.4 验证（诚实记录）

| 门 | 结果 |
|---|---|
| latex_probe --initex（/tmp/r21/w2，未插桩） | **87 → 86**（主簇未落清——简报"进 <30"未达） |
| `cargo test -p ntex-core` | **328 绿**（325 既有 + 3 新增回归测：`number_scan_skips_false_branch_of_nested_romannumeral`、`expr_factor_expands_before_paren_dispatch`、`expr_operator_peek_skips_spaces_and_evals_cond`） |
| TRIP | `组未闭合（缺少 }）groups=[SemiSimple, MathLeft, MathLeft, Align]`——与 HEAD 基线逐字节一致（stash 前后各跑一次对拍） |
| 插桩还原 | 探针一律写 /tmp/r25（复制体），w2 harness 的 `expl3-code.tex` 未动（`cmp` 通过） |

计数未显著下降的判读：4 处修复各自由机制级复刻验证为**真修**（旧值 → 新值
对照见 §32.2 表），但都落在主簇的**上游**——上游解锁后 l.8073 的失真表达式
得以走到下一层、在 l.9386/l.9468 暴露为新的失败形态（"Missing )" 1→3）。
主簇的实质门闩是 §32.3 的两处表达式扫描语义，非本轮靶的真臂时序。

### 32.5 本轮踩坑

- **探针插桩点必须是顶层**：`expl3-code.tex` 全篇是 `\cs_new...{...}` 序列，
  插进定义体/组内会被当 token 吸收（write16 不触发且无报错）。已验证的顶层
  插入点：l.2994（`\__cs_generate_variant:wwNN` 前）、l.8074
  （`\sys_load_backend:n` 前）。二分定位"执行到哪"时只信顶层点。
- **机制级复刻必须带齐 expl3 的三件套**，否则失败形态是假的（本轮误判 3 次）：
  1. catcode `\:`/`\_` = 11——`\if_meaning:w` 否则解析成 `\if_meaning` + `:w`，
     错误形态变成"残留 `:` 字符"；
  2. `\let\else:\else \let\fi:\fi \let\if_meaning:w\ifx` 等别名——`\else:` 未定义
     时 NTex 的 edef `\if` 配对检查直接报"缺 \fi"；
  3. `\__int_sep:` 是 **`\let`**（`\cs_new_eq:NN \__kernel_int_sep: \tex_let:D`，
     l.4729）不是 `\relax`——用 `\relax` 当分隔符的复刻会在表达式收口位
     提前失败（`\relax` 被运算符前瞻吸收）。
- **`\__kernel_int_sep: = \let`** 本身是 l3kernel 的设计：分隔符必须是
  不可展开且非 `\relax` 的 token，才能既终结 `\numexpr` 又留在流里给
  宏实参扫描当定界符。
- **仓库工作树非本会话独占**：`mod.rs`/`sink.rs`/`ntex-layout/*` 有并行会话的
  "输出例程刀 1"改动在场——本刀提交只取 `expr.rs`/`scan.rs`/`tests.rs` 三文件
  （TRIP 对拍时 `git stash` 会连他人改动一起藏起，对拍结论不受影响）。

## 33. 2026-09-06 第二十六轮：表达式因子/运算符位的 fi_or_else 消费 + 数字扫描别名解引用——l.8073 根治、主簇改判独立

### 33.1 起点、真 TeX 地面真值与靶子改判

HEAD=1026f00，探针终态 **86**（32 `\use_ii:nn` extra-} + 27 Missing number +
3 Missing ) 为主簇）。本轮先建**地面真值**（VM 有 TinyTeX pdftex
1.40.29，直接实测，不再纸面推演）：

- `/tmp/r25/mech19.tex` 真实 TeX 输出 **`FULL-DIV=1`，0 错误**；
- 实参扫描两引擎逐 token 一致（`#1=[1] #2=[40] #3=[1] #4=[00]`——
  `\expandafter` 在运算符位把 `\sep` 放回、内层 `\number\numexpr 100` 产
  `100` 后 `\sep` 归位，故 `#1#2`=`1``40`）；
- `\if_meaning:w`（=`\ifx`）在表达式内的**机制级最小复现**（/tmp/r26/micro3）：
  | 样例 | 真 TeX | NTex（改前） |
  |---|---|---|
  | T1 `\numexpr 140\ifx-1+\else:-\fi:(100-1)/2\relax` | **90** | `140(100-1)/2` |
  | T2 `\numexpr 140\ifx-1+\else:-\fi: 2\relax` | 138 | 138（**假阳性**：`140+(-2)`） |
  | T3 `\numexpr(\ifx-1-\fi: 100-1)/2\relax` | 50 | 50（巧合一致） |

- **e-TeX `\numexpr` 除法是舍入不是截断**（实测 `91/100=1`、`99/2=50`、
  `-91/100=-1`；NTex 已一致）。这是 mech19 出 `1` 的前提：
  `\__int_div_truncate:NwNw` 的体 `( #1#2 - ( #3#4-1 )/2 ) / #3#4` 正是
  用**舍入除法**实现截断（`(140-99/2)/100` → `(140-50)/100` → `90/100`
  舍入 = 1 = `140/100` 截断）；截断引擎会算出 0。
- 简报靶 #1（`scan_number_inner` 缺 `EqSlot::Alias` 臂）**对 mech19 不成立**：
  本引擎 `\let` 在定义时**压缩别名链**（`let_to`：Alias(t2)→指向 t2 的目标；
  原语/寄存器/字符/字体/流直接复制含义），Alias 槽只对**宏/未定义**目标保留。
  mech19 的 `\iv`/`\ev` 经两级 `\let` 后已是 `Primitive(Number/NumExpr)` 的
  直接拷贝，`\number\ev 140` 实测本来就通（/tmp/r26/micro1 M2/M3=140）。
  该缺口对**宏别名**真实存在（`\def\a{140}\let\b\a\number\b` 落 Missing
  number），本轮一并补上（见 §33.2）。

### 33.2 真根因与修复（全部 tex.web 直译）

**真根因（一处机制、两个位点）**：`\if_meaning:w` 在数字扫描的**十进制循环**
就地求值（22 刀契约：`\ifnum0\ifdefined…` 惯用法；`\ifx - 1` 假 →
`skip_ahead` 同步吃 `+`/`\else:`、帧留栈 Processing 等 `\fi`），随后
`\fi:` 落到表达式**因子位**（`-` 成为运算符之后）与**运算符位**——
`expr_peek_factor_token`/`peek_int_op` 都不消费 fi_or_else：`\fi` 被当因子
放回 `scan_number`，帧由符号循环弹出，`( … )` 撞十进制循环报
"Missing number"→表达式在 `-` 后收 0、外层 `(` 配不上 `)` 再报
"Missing )"，值失真为 `140(100-1)/2)/100`。tex.web 侧这些位点全是
get_x_token，`\fi` 经 expand→conditional()（§9897）就地闭合后继续取 token。

| # | 位置 | 修复 | 对照 |
|---|---|---|---|
| 1 | `expr.rs expr_peek_factor_token` | 补 fi_or_else 臂：`Fi\|Else\|Or` 且 `cond_stack` 非空 → `step_conditional` 后继续取 token；**游离**终结符（栈空）维持放回（TRIP L82 契约） | T1 `140(100-1)/2` → `90` |
| 2 | `expr.rs peek_int_op` | 同款臂（栈空 → 放回 + 表达式收口） | 同上 |
| 3 | `scan.rs scan_number_inner` 十进制循环 expandable 检查、反引号后续展开检查、内部量分派（`peek_csid` 后） | 新增 `deref_alias_chain` 助手（上限 100，同 fetch_non_filler），三处追链后再判定 | `\number\宏别名`：Missing number→`140` |
| 4 | `expr.rs expr_peek_factor_token`/`peek_int_op` 的 expandable 检查 | 同款追链（`peek_int_op` 的 `\relax` 链检查已有，展开检查没有） | `\numexpr 1+\宏别名`：→`6` |

`insert_relax` 门（栈顶帧 Evaluating）由 `step_conditional` 内部处理，token
放回 + frozen `\relax` 前插，表达式按 Missing number 收场——tex.web 同款，
无需表达式侧特判。

### 33.3 探针结果与主簇改判（§32.3 级联理论证伪）

latex.ltx --initex（/tmp/r21/w2，未插桩，`cmp expl3-code.tex .orig` 通过）：
**86 → 81**。

- **消失的 5 条 = l.8023-8073 全链**：`Missing ) inserted for expression`
  3→0（pdftex/luatex 分支 + `\int_mod` 级联）、`Missing number` 27→25。
  `\c_sys_engine_version_str` 一线的表达式失真**已根治**（机制级标尺
  mech19 = `1` 逐字符对拍）。
- **32×`\use_ii:nn` extra-} + 25×Missing number 的 l.9468+ 主簇纹丝不动**——
  §32.3"主簇是 l.8073 失真的下游级联"**证伪**。主簇有自己的上游：
  `\prg_new_conditional:Npnn` 生成器在 **l.2329**（`\cs_if_eq:NN`）首次
  使用**无错**，从 **l.9468**（`\token_if_group_begin:N`）起**每条**
  `\token_if_*` 定义都报同一签名（1×"Missing number(``}``)" 于定义收口
  `}` + 2×"Argument of \use_ii:nn has an extra }"），直至 l.10298 输入栈
  超限收场（该终止为既有）。l.2329 与 l.9468 两个调用点之间的差异
  （`{ p , T , F , TF }` forms 列表经 `\tl_to_str:n`、`\@@_generate_conditional_test:w`
  分派 fast/normal 臂、`\use:c { @@_generate_#8_form:wNNnnnnN }` 造名）
  是下一轮的勘察入口；l.9386 `\char_generate:nn` 的 Illegal parameter number
  （1 条）与此簇无关（`\c_catcode_other_space_tl` 首次使用在 l.12219）。

### 33.4 验证（诚实记录）

| 门 | 结果 |
|---|---|
| mech19.tex（机制级标尺） | **`FULL-DIV=1`，0 错误**——与真 TeX 逐字符一致 |
| micro3 T1/T2/T3 | **90/138/50**——与真 TeX 一致（改前 140(100-1)/2 / 138 假阳性 / 50 巧合） |
| `cargo test -p ntex-core` | **330 绿**（328 既有 + 新增 `expr_factor_consumes_fi_after_evaluated_cond`、`number_scan_derefs_alias_to_target_meaning`；`expr_operator_peek_skips_spaces_and_evals_cond` 去·假阳性化，见 §33.5） |
| latex_probe --initex | **86 → 81**（简报"主簇落到 <30"未达——§32.3 级联前提证伪，见 §33.3） |
| TRIP | 与 HEAD 基线**逐字节一致**（44472B，仅 tempdir 名非确定；基线二进制取 target/debug/ntex-trip 17:07 构建=HEAD 先于本轮编辑，未走 worktree 冷构建——本轮编辑均先于 18:00，17:15 的探针二进制实测仍报改前失真可佐证） |
| 插桩还原 | 诊断用临时 `dual_run`/`tmp_disk_mech19` 已全部移除；`git status` 仅剩 expr.rs/scan.rs/tests.rs 三文件 |

### 33.5 本轮踩坑

- **二十五刀的 `expr_operator_peek_skips_spaces_and_evals_cond` 是假阳性门**，
  两处失真叠加：① 源码缺 `\let\if_meaning:w\ifx` 别名组（初表全 cat12 下
  `\if_meaning:w` 劈成 `\if`+`_meaning:w`，`\let\if` 反把 **`\if` 定义成
  `_` 的字符别名**——§32.5 三件套自己写了却没进测试）；② 断言
  `transcript.contains("GA=1")` 被真实输出 `GA=10(140-1+-(-1-100-1)/2)/100`
  的**前缀**满足。该测长年绿、mech19 同构源实际一直错。教训：
  **write16 断言必须锚定行首并逐行相等**（`lines().find(starts_with)` +
  `assert_eq!`），`contains` 只配锁"有/无"不配锁"值"。
- **harness 分裂的假象**：单测（VecSink）过而探针（Typesetter）败时，先怀疑
  路径分裂、后证明**同 harness 同源也败**（磁盘读入 mech19 走 run_transcript
  同样 140(100-1)/2)/100）——真因是断言假阳性，不是 sink 差异。对照实验要
  做到"同字节、同助手"再下结论。
- **`\let` 别名链在定义时压缩**：Alias 槽只对宏/未定义目标保留。给扫描位
  补 deref 时，别用"expl3 两级别名链"当复现——`\let\iv\texnumD` 两步后
  就是直接含义拷贝；宏别名（`\let\b\a`，a 为宏）才是唯一走 Alias 的路径。
- **探针 A/B 计数须防管道截断**：`probe | grep | head` 会在 transcript 写盘
  后因 SIGPIPE 提前退出，紧随的 `grep -c` 计到旧文件（本轮 81/86 首测即此
  坑）。改 `>out 2>&1` 落盘再计数、A/B 各自拷贝 transcript。
- **真 TeX 除法舍入是 expl3 截断补偿的地基**：`\int_div_truncate` /
  `\int_mod` 全族用舍入除法构造截断语义；若引擎截断/舍入与 e-TeX 不符，
  这族函数全体失真且**无错误消息**（静默错值），对拍时优先锁
  `91/100=1`、`99/2=50`、`-91/100=-1` 三个锚点。

## 34. 2026-09-06 第二十七轮：`\token_if_*` 生成条件区主簇根治——组定界别名 cs 在 token 列表扫描被归一成字面花括号

### 34.1 起点、靶子与判别路径

HEAD=f480973，探针终态 **81**（32×`\use_ii:nn` extra-} + 25×Missing number 主簇 +
6 Undefined + 3 Illegal parameter number + 3 `\use_i:nn` extra-} + 3 Extra \fi +
2 `\use_none:nnnn` extra-} + 2 Missing endcsname + …）。简报四候选逐一判别：

1. **"l.9468 区使用形态不同"** —— **证伪**。同签名的 `\prg_new_conditional:Npnn`
   在 l.3307-4596（`\quark_if_*`/`\tl_if_empty:*`/`\tl_if_head_eq_catcode:nN`，
   后者同样用 `\if_catcode:w` + `\exp_not:N`）全部无错；l.9468 起每条都错。
   差异不在生成器使用形态，而在**体内引用了 `\c_group_begin_token` 等
   `\let` 到字符 token 的 cs**（见 34.2）。
2. **"l.9386 `\char_generate:nn` 的 Illegal parameter number 污染后续"** ——
   **证伪**（doc §33.3 判断成立）。python 精确改写 l.9386 为
   `\tl_const:Nn \c_catcode_other_space_tl { }` → **81 → 79**（只消掉它自己和
   紧随的 Too many }'s），主簇 57 条纹丝不动。
3. **"`\use_ii:nn` 本身还有第三处调用点漏"** —— **证伪**。`\use_ii:nn` 是
   `\__prg_generate_p_form:wNNnnnnN` 的 else 臂选择器，报错是**下游症状**，
   不是独立根因。
4. **"l.9468 是陈旧锚点"** —— 部分成立但不影响定位：错误锚在 l.9468
   （`\token_if_group_end:N` 体的收口 `}`），首个失败定义是 l.9459
   `\token_if_group_begin:N`，级联沿后续每条 `\token_if_*` 传播。

### 34.2 判别实验（/tmp/r27/vary.py：逐轮改写 expl3-code.tex.orig 的 l.9461 体 → 探针计数）

| l.9461 体 | 错误数 |
|---|---|
| `\if_catcode:w \exp_not:N #1 \c_group_begin_token`（原样） | **81** |
| `\if_meaning:w #1#2`（`\cs_if_eq:NN` 同款） | 11（该变体提前 bail，错误移到下一条 `\token_if_group_end:N`） |
| `\if_catcode:w #1 \c_group_begin_token`（去 `\exp_not:N`） | 81 |
| `\if_catcode:w #1 \scan_stop:` / `#1 a` / `a b` / `#1 #1` | 11 |
| `\if_odd:w 1 \else: \fi:` | 11 |
| **`\if_catcode:w a \c_group_begin_token`（无参数字符）** | **81** |
| **`\if_meaning:w #1 \c_group_begin_token`** | **81** |
| **`\scan_stop: \c_group_begin_token`（无条件、无 `#`）** | **81** |
| `\c_group_begin_token \if_catcode:w a b` | 81 |

**结论：判别式是"体内是否出现 `\c_group_begin_token`"，与 `\if*`、`#`、
`\exp_not:N` 全无关。** `\c_group_begin_token` 由 l.9456
`\tex_global:D \tex_let:D \c_group_begin_token {` 造出——cs 绑定字符 token。

### 34.3 真根因（tex.web 直译）：token 列表扫描的组判定看 `cur_tok`，不是 `cur_cmd`

tex.web `scan_toks`（§1338）与 e-TeX `scan_general_text` 收集 token 列表时，
组起止判定是 `cur_tok<right_brace_limit`——**cs token 永不小于该界**
（`cur_tok = cs_token_flag+cs`）。`\let\bg={` 型 cs 经 `get_token` 取出仍是
cs token，"组性"只体现在 `cur_cmd = eq_type = left_brace`（§1223 `\let` 直接
存 `cur_cmd/cur_chr`），**在 token 列表扫描里它是数据**。`cur_cmd` 判定只在
`scan_left_brace`（L8194，开括号搜索）这类"要一个花括号"的上下文成立。

本引擎 `scan.rs scan_group_contents_expanding`（`\use:e`/e 型体、
`\unexpanded`/`\detokenize` 的 general text 走此路径）把组定界别名 cs 经
`resolve_group_char` 归一成**字面 `{` 字符 token**：组深 +1 永不闭合 →
`\__prg_generate_conditional_test:w`（定界实参扫描）吞到定义体外 →
体内 `\prg_return_false:` 的 `\exp:w`（romannumeral）在错位状态里把
`\exp_end:`（chardef 0）当数吃 → **1×Missing number + 2×`\use_ii:nn`
extra-} 每条定义**，即 §33.3 的主簇（expl3 `\token_if_*` 区 34 条定义 ×3
≈ 57 错）。修复 = **删去该扫描内两处归一**（开括号位的 match 臂保留——
它对应 `scan_left_brace` 的 `cur_cmd` 判定，tex.web 正确）；
`scan_group_contents`（`\toks` 类赋值，TRIP 依赖）与 `fetch_non_filler`
（`scan_left_brace` filler）的归一**保留不动**。

### 34.4 验证（诚实记录）

| 门 | 结果 |
|---|---|
| latex_probe --initex（/tmp/r21/w2 同目录条件） | **81 → 5**（简报"预期 <25 甚至个位数"达成）。剩余：1×Illegal parameter number（l.9386 `\char_generate:nn`）+ 1×Too many }'s + 1×Missing number + 2×Undefined；加载推进到 l.11835（原 l.10298 输入栈超限收场消失） |
| `cargo test -p ntex-core` | **332 绿**（331 既有 + 新增 `group_delimiter_alias_is_data_in_token_list_scan`：`\let\bg={\def\f#1{[#1]}\edef\x{\f\bg}\meaning\x}` → `macro:->[\bg]`） |
| TRIP | 终态签名与 HEAD 基线一致：`组未闭合（缺少 }）groups=[SemiSimple, MathLeft, MathLeft, Align]`（§30.7/§32.4 同款）。**本轮未做 worktree 冷构建逐字节对拍**（时间盒已过 + 简报禁 stash 对比），留给主控复核 |
| 插桩还原 | 未插桩 expl3-code.tex（变体一律在 /tmp/r27 拷贝体上做，`expl3-code.tex` 未动） |

### 34.5 下一轮靶（证据链已备好）

1. **`EqSlot::Char` 混装 `char_given` 与 `\let` 到字符**（本轮实锤的第二处 tex.web
   偏差）：`\chardef\cb=123` 与 `\let\bg={` 在本引擎同为 `EqSlot::Char`，数字扫描
   内部量分派（scan.rs `EqSlot::Register(Count,_) | EqSlot::Char` 臂）把两者都当
   内部整数。**真 pdftex 实测**（`/tmp/r27/m/x1.tex`）`\ifnum\bg=123`（`\bg`=`\let` 到
   `{`）→ `! Missing number, treated as zero. <to be read again> \bg`；本引擎静默
   取 123。修法：拆出 `char_given` 槽（`\chardef`/`\mathchardef` 专用），数字位只认它；
   `\ifx` 的 MeaningKey 亦须区分（tex.web 比较的是 `eq_type`，两者不等）。
2. l.9386 `\char_generate:nn` 在 e 型定义体扫描内报 "Illegal parameter number in
   definition of"（`<to be read again> \exp_not:N`）+ Too many }'s：定界串/参数位
   在 e 型扫描的 `#` 处理（tex.web scan_toks `if macro_def then` 门）待对齐。
3. `scan_group_contents`（`\toks` 类）的 `resolve_group_char` 归一与测试
   `scan_left_brace_expandable_filler` 第二断言（`\everyjob\f`、`\f={\bgroup y\egroup}`
   期望 `y`）是**引擎自设行为**：真 pdftex 同源输入报 runaway（`\egroup` 不配平）。
   本轮因 TRIP 依赖未动，标记为与 tex.web 的已知偏差（记录在案，待专项裁决）。

### 34.6 本轮踩坑

- **探针变体 harness 的 sed 会吃反斜杠**：`sed "9461s/.*/    \\if_catcode:w …/"`
  的替换段把 `\i`/`\e`/`\c` 当普通字符剥掉（文件字节数 -4 可作自检信号），
  得出 11 错的假结论。变体一律用 python `split('\n')` 改写下标。
- **探针目录状态参与结果**：`texsys.aux` 在场与否会翻转失败形态
  （有 = 81 错主簇；无 = 在第二条 `\token_if_*` 处 `\use_i:nn`/conditional-form-unknown
  提前 bail 收场，11 错）。A/B 计数必须复制整目录状态
  （`latex.ltx`/`expl3.ltx`/`texsys.cfg`/`texsys.aux`/`tripos`）。
- **initex 初表的 `{`/`}` 是 cat 12**：机制级微复现（`latex_probe --initex` 跑小文件）
  必须先 `\catcode`\{=1 \catcode`\}=2 \catcode`\#=6`，否则 `\def\a{X}` 直接报
  "参数文本未闭合（缺少 {）"（真 INITEX 语义，非引擎缺陷）。
- **主簇改判的教训（续 §33.5）**：本轮最初沿简报候选 1（生成器使用形态）勘察
  白费三步——判别式的正确找法是"**最小可变体**"：把同一条定义的体逐成分替换
  （去 `\exp_not:N`、换条件、换操作数），一次一个变量，其余不动。
