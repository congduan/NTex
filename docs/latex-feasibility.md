# LaTeX 兼容战役（活文档）

> **本文档只保留当前有效信息。** 已修完的历史逐刀记录（§8–§37 共 30 节、约 2900 行）
> 归档于 [archive/latex-feasibility-full-2026-09-11.md](archive/latex-feasibility-full-2026-09-11.md)
> —— 需要查具体某刀的根因链/tex.web 锚点时去那里 grep。

**目标**：`latex.ltx` 加载成功 → `latex.fmt` 构建 → `\documentclass{article}` → PDF。

**当前状态**：🔴 **阻塞在 expl3 加载期的输入栈无终止条件**（见 §A）。

---

## A. 当前阻塞点（唯一活跃项）

### A1. expl3 变体生成器压栈不终止

```
== pass1 ERROR: 输入栈超限（5001 帧 > 5000）
   展开/参数扫描疑似无终止条件（定义 \l__iow_line_part_tl 的替换文本时）
```

watchdog 现场（`latex_probe --initex`，2026-09-11 复测）：

```
last_tok = \cs_generate_variant:Nn
         → \__prg_F_true:w → \if_meaning:w → \tex_advance:D
栈内出现 TokenList(12640tok, pos=4801)  ← 大 token 列表反复重入
```

**定性**：`\__cs_generate_variant_loop:nNwN` 一带的三层嵌套 + `{ ~ { } \fi: ... } ~`
组内藏 `\fi:` 惯用法，chose 桩 `\use_ii:nn` 取参错位 → 递归不终止。

**方向**：修递归终止条件。**不要**抬高 `MAX_INPUT_STACK`（5000 是 tex.web/TL
生产契约，卡 5001 = 递归不终止的防护网，改大只是掩盖）。

**复现**：
```bash
export PATH="$HOME/.cargo/bin:$PATH" NTEX_TFM_DIR="$HOME/.ntex-fonts"
# 备料（若 /tmp/r27b 被清）：取 latex.tar.xz + l3kernel.tar.xz 的生成版
cargo run --release -p ntex-test-support --example latex_probe -- \
    /tmp/r27b/latex.ltx --initex 2>&1 | tail -30
```

### A2. 连锁：`\reserved@a` 未定义自引用

```
l.301  \edef\reserved@a{\expandafter\reserved@a\string^^J\@@}
! Undefined control sequence.
```
expl3 失败后 latex.ltx 继续执行暴露的上层连锁，**A1 修好后应自消**，先不动。

---

## B. 引擎侧：已具备（不需重做）

| 能力 | 状态 |
|---|---|
| fmt 构建链路 | ✅ iniTeX 加载 → `\dump` → `.fmt` 存/取 → pass2（TRIP/ETRIP 双 pass 验证）|
| e-TeX 原语层 | ✅ `\numexpr`/`\dimexpr`/`\glueexpr`/`\detokenize`/`\scantokens`/`\everyeof`/`\ifprimitive`/`\interactionmode` |
| plain.tex 预载 | ✅ 1241 行全通（G0–G3，`\newif` 端到端与 pdfTeX 一致）|
| DVI → PDF | ✅ ntex-pdf 成熟（多字体 Type1 嵌入）|
| 错误恢复推进 | ✅ 22838 行可在 ~43ms 吞错处理完 |

## C. 引擎侧：已修的关键语义（速查表）

> 完整根因链/tex.web 锚点见归档文档对应节。此表只为「这坑修过没有」的快速查询。

| # | 修复内容 | commit | 归档节 |
|---|---|---|---|
| 1 | INITEX 初始 catcode 表（`{`=12 判别纯 initex） | — | §8 |
| 2 | 文件通道落盘（`\openout`/`\write<流>`）+ 缺文件错误语义 | — | §9 |
| 3 | `\global` 前缀链 | — | §10 |
| 4 | `<internal dimen>` 扫描臂 | — | §11 |
| 5 | `scan_left_brace` filler 语义 | — | §12 |
| 6 | `\csname` 未定义名 = `\relax` | — | §13 |
| 7 | `scan_number` 内嵌套条件 + 注释行状态 | — | §14 |
| 8 | pdfTeX 引擎伪装（12 探测原语） | a6c499b | §15 |
| 9 | 关系符扫描臂 `get_x_token` 展开 | — | §16 |
| 10 | 宏实参扫描的 `\else`/`\fi`/`\or` 一律数据 | — | §17 |
| 11 | l.398 根因链：四引擎层缺口 + 行模型 catcode | — | §18 |
| 12 | 分隔实参整组贡献 + 定界符按 token 同一 | — | §19 |
| 13 | `\string` `sprint_cs` 语义 + 参数文本 `#{` hash_brace | — | §20 |
| 14 | 0 参数宏定界串匹配 + `\if` 操作数展开 | — | §21 |
| 15 | store_arg 组实参语义（无分隔组实参**存储前剥组**） | 6128722 | §23 |
| 16 | 操作数位嵌套条件重开求值 | — | §24 |
| 17 | `math_display` `$$` 闭合侧接线 | 360c342 | §25 |
| 18 | `\romannumeral` 字母常量后继续展开（expl3 f 型） | — | §26 |
| 19 | unsave 的 retain 守卫（组内局部触碰后 `\global` 被回滚） | — | §27 |
| 20 | 0 参数宏纯定界串在展开上下文漏匹配 | — | §28 |
| 21 | 表达式终结符前瞻 `get_x_token` 化（l.9365 清零） | — | §29 |
| 22 | `\ifx` 补 `\noexpand` 替换臂 | — | §30 |
| 23 | 数字扫描符号循环的条件机推进 | — | §31 |
| 24 | 数字扫描跳过区臂 + e-TeX 表达式三处机制缺口 | — | §32 |
| 25 | 表达式因子/运算符位 `fi_or_else` 消费 + 别名解引用 | — | §33 |
| 26 | `\token_if_*` 生成条件区：组定界别名 cs 不得归一成字面 `{` | — | §34 |
| 27 | l.9386：`\meaning` 补 `\protected` 前缀（expl3 变体降级根因） | — | §35 |
| 28 | `\expanded` 实参 IPN ×256 清零 + `\lowercase` 转 active char | 78dd892 | §36 |
| 29 | `\let` 左侧缺 cs 走 TeX 恢复而非终止引擎 | — | §37 |
| 30 | plain 预载 `\newif` 链（`\meaning` 丢定界符致误判） | c096f8b | §38–41 |

## D. 战役 KPI（corpus-probe）

```bash
~/.venvs/pixtools/bin/python scripts/corpus-probe.py
```

**当前：5/8 PASS + 3 EMPTY**（2026-09-11）

| 样例 | ink | chars | 判定 |
|---|---|---|---|
| latex/sample2e · small2e · testpage | 6148–14044 | 922–2074 | PASS |
| math/basic-expressions · symbols-matrix | 579–985 | 84–166 | PASS |
| plain/plain · letterformat · list | 0–7 | 0–1 | EMPTY（空页）|

> ⚠ **latex 三例的 PASS 是「降级渲染」**：`\documentclass`/`\newcommand`/`\begin`/`\end`
> 全部 Undefined（无 latex.ltx），引擎错误恢复把**裸正文文字**排了出来。
> 结构、字体、版式全丢。**真 LaTeX 渲染未通**，勿据此报喜。
> （`\maketitle` 标题块能出现是因为它退化成了纯文本流。）

**EMPTY 三例** = `plain/*.tex` 无正文无 `\bye`，真 TeX 0 页、NTex 经 `\plainoutput`
收尾冲一页空页（survey §5.bis 发现未修 #1）。

## E. 缺件清单（攻坚前置）

| 项 | 状态 | 取法 |
|---|---|---|
| `latex.ltx` 生成版 | ✅ 在 `/tmp/r27b/`（勿重下） | TL tlnet `archive/latex.tar.xz` |
| `expl3.ltx` / `expl3-code.tex` | ✅ 在 `/tmp/r27b/` | TL tlnet `archive/l3kernel.tar.xz` |
| `latex2e-first-aid-*.ltx` | ❌ 缺 | TL tlnet `archive/firstaid.tar.xz` |
| `article.cls` | ✅ 在 `/tmp/repro2/` | TL tlnet `archive/latex.tar.xz` |
| CM 字体组 TFM（cmr7/9/cmss10/cmbx10/cmti10…）| ✅ `~/.ntex-fonts` 已扩至全家族 | TL `archive/cm.tar.xz` |
| Fandol 中文 OTF | ✅ `~/.ntex-fonts`（8 个） | CTAN `fonts/fandol.zip` |

## F. 方法论纪律（血泪沉淀）

1. **仪器先于结论被验证**。三次仪器失真教训：
   - `corpus-probe` 产物路径假设 → 假阴性 0/8
   - `corpus-probe` 只判存在性 → 空页假阳性
   - **`\meaning`/`\show` 丢参数文本定界符 → 误报 `macro:->`**（害 §38/§39
     两轮定位跑偏，§41 才推翻）
2. **"没炸"≠"对"**。引擎吞错推进会让"跑完/错误计数"失真——**进度判定一律用
   「阻塞点位置单调前移」**，不用跑完与否。
3. **文档待办滞后于实测**。§38 记录的"plain 预载 24 错"在 `cd98a94` 后已消解，
   §39 实测 0 条。**先复测再销账**。
4. **插桩一次拿全数据**，不要"加一行 eprintln 跑一次"。环境变量门控多处同时打点。
5. **改 `@` 类 cs 先 `\catcode`\@=11`**。plain.tex L1239 把 `@` 改回 cat 12，
   预载后 `\if@` 不可访问是**正确行为**（pdfTeX 同样切成 `\if` + `@`）。
6. **math 组生命周期大改会死循环**（290 万步卡 `}`/`$`）——禁直接改
   `MathShift`/组结束/`close_math` 路径；分阶段 + `timeout 200` 验证。
