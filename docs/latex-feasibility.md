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

## 8. 本次改动

- 新增 `crates/ntex-test-support/examples/latex_probe.rs`（勘察工具，~100 行，不动引擎语义）。
- 新增本报告。**未修改任何引擎核心语义**；`--shim` 的 INITEX catcode 归位只存在于勘察工具内。
