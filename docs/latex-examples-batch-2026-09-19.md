# LaTeX-examples 批量测试报告（2026-09-19）

## 测试对象
MartinThoma/LaTeX-examples（GitHub）全量 **906 个 .tex**（去重后 927 次编译记录，
含少量同名子文档）。

## 引擎版本
HEAD = ead2609（含 amsmath 载入链三连修：`#<cs>` 定界、uccode 组作用域、
detok cs 尾空格）+ `\the\parfillskip` 补分支。

## 总体结果

| 结果 | 数量 |
|---|---|
| PASS | **0** |
| FAIL | 927 |

0 PASS 的结构性原因：该仓库文档几乎全部依赖重宏包
（beamer/tikz/standalone/KOMA/hyperref/…），而 NTex 发行版闭包
（assets/tex-minimal）只覆盖 LaTeX 核心子集。这不是本轮修复引入的回归，
而是资产覆盖面 + 宏包生态支持度的客观现状。

## FAIL 分类（按首错）

| 类别 | 数量 | 定性 |
|---|---|---|
| Undefined control sequence | 381 | 混合：缺宏包的级联 + 引擎原语缺口 |
| File not found | 223 | 资产缺口（详见下表）；tlmgr 补装即可解除 |
| timeout（60s） | 92 | **下轮优先**：逐个判"挂死 vs 慢"——挂死是高价值引擎 bug |
| 排版失败：非法输入（消息为空/截断） | ~197 | 抽样 = `\toks 赋值 RHS 需为 {token list}`（`\toks0={...}` 语法支持缺口）等 |
| Missing number ×17、math alphabet ×12、其他 | ~30 | amsmath/amssymb 深水区 |

### File not found 缺口 TOP（TinyTeX tlmgr 可补装）

| 宏包 | 文档数 | 备注 |
|---|---|---|
| standalone.cls | 83 | 已补装 ✓（asymptote 系还需 asy 外部编译器，仍不可用） |
| KOMA 系（scrartcl/scrreprt/scrbook/scrlttr2） | 90+ | 已补装 ✓；但 KOMA 的 keyval 选项解析（scrbase）有引擎缺口 |
| preview.sty | 10 | 已补装 ✓ |
| csquotes/nomencl/listings | 各 1-3 | 已补装 ✓ |
| IEEEtran.cls | 2 | ieetran 包安装失败，待查 |

## 本轮已修复并提交（ead2609）

1. **参数文本 `#<cs>` 定界**（macros.rs）：tex.web scan_toks 第三分支缺失，
   `\def\foo#1#2\bar{}` 直接报错——amsmath.sty l.2790 载入即炸；
2. **uccode 组作用域泄漏**（mod.rs/primitive.rs/save.rs）：exec_uccode 复用
   LcCode 变体 + 恢复写错数组（双重字段错位）——amsmath 的
   `\uppercase{\gdef\macro@#1#2#3#4\macro@{…}}` 致命；
3. **detok_tokens cs 尾空格**（free.rs）：`\meaning` 缺 show_token_list 补
   空格规则——amsmath `\@tempb#1>#2#3<空格>#4` 切分崩；
4. **`\the\parfillskip`**（save.rs）：KOMA setparsizes 即触发；
5. 3 处陈旧测试期望按 pdfTeX GT 对齐 + 新增回归锁。

修复后 amsmath.sty 载入从 l.558 硬失败推进到 l.742 后（约 +200 行 / +3%）。
纯 LaTeX 核心文档（article + amsmath 的中文论文，无 amssymb）可出 1 页 DVI/PDF。

## 下轮优先级

1. **92 个 timeout 逐个排查**（挂死=高价值引擎 bug）；
2. `\maketitle` 链 `\if 缺少 \fi`（LaTeX 文档最高频命令）；
3. amssymb 载入链（`not defined as a math alphabet`，12 文档）；
4. `\toks0={...}` 赋值语法支持（197 文档"非法输入"的主力嫌疑，需逐个确认）；
5. TikZ/beamer 支持评估（55+ 文档，属大工程需单独立项）。

## 测试方法备注

- 每文档 timeout 60s，`--input-path` 指向文档目录（支持同目录子文件）；
- 判 PASS 唯一标准 = 输出"已写出 …dvi"；
- 结果 tsv：/tmp/batch-results.tsv（临时，重启丢失）；
- 批量脚本：/tmp/batch-probe.sh（同样临时，脚本本体已内联到本文档描述）。

## 附录：timeout 排查进展（2026-09-19 深夜）

92 个 timeout 抽验 16 个（rebuild 后复测）：
- 4 个 COMPLETED（chap1 子文件等，原 timeout 判定有误——可能是并发构建慢）
- 12+ 个 HANG，**全部是 beamer 系演示文稿**

最小复现（7 行）：
```
\documentclass{beamer}
\begin{document}
\begin{frame}
Hello
\end{frame}
\end{document}
```
- 症状：1040 万+ 步高速空转不终止；watchdog 栈浅
  （`TokenList(1tok,pos=0) | Bytecode(pc=N)`），尾部执行 `\ifx`/`\let`/`\def`
- 特征：Let/Def 各 11.8 万次持续增长（NTEX_TRACE_EXEC 统计）——
  疑 beamer.cls 载入期某 \def 循环（`\DeclareOptionBeamer` ×
  `\beamer@dokv` × keyval `\define@key` 链）展开不终止
- bisect beamer.cls（截断法）：FIRST-BAD≈136 行，该区是
  `\DeclareOptionBeamer` 密集区（依赖 `\newrobustcmd`+`\@ifnextchar`+
  `\define@key`）
- 下一步：抓活锁循环的 cs 名（给 Let/Def 打点带 intern 名，env 门控），
  或对 `\newrobustcmd`/`\define@key` 写最小单测

## 附录补完：92 个 timeout 全量复测结论（同日晚）

- 复测判定：41 HANG / 8 COMPLETED（其余为路径含空格被 tsv 切碎的碎片，同属上述两类）
- **41 个 HANG 全部指向 beamer.cls 载入活锁**（presentations 系 35 个 +
  其他目录中引用 beamer 的文档；少数 COMPLETED 是被 \input 的无 preamble 子文件）
- 最小复现 7 行（见上）；根因圈定 `\DeclareOptionBeamer`→`\newrobustcmd`/
  `\@ifnextchar`/`\define@key` 链路展开不终止
- 修复路径建议：给 Let/Def 执行打点带 intern 名（env 门控）抓循环 cs 名，
  或对 `\newrobustcmd`/`\define@key` 做最小单测

## 附录 2：beamer 活锁根因实锤（2026-09-19，input.rs 行尾语义）

### 活锁链（最小 12 行复现）

```tex
\catcode`\^^M=12
\long\def\eatone#1^^M{\message{GOT: [#1]}\eatnext}
\def\eatnext{\message{EATNEXT}\eatone}
\eatone
第一行
...
```
NTex：`#1^^M` 定界永不匹配 → \eatone 吞到文件尾递归 \eatnext → 死循环。
pdfTeX GT：`GOT: [第一行]` 正常逐行消费。

### 根因

`input.rs` l.302：行尾字节（LF）**硬编码** `cat = Catcode::EndOfLine`——
行尾 token 的 catcode 不查表。而 tex.web `@<Read next line…@>`（L7578-7579）
的真语义是：`buffer[limit]:=end_line_char`（把 \endlinechar 的**值**写入行尾
位置），token 化时按该字符的**当前 catcode** 分派。

beamer `beamerbasemodes.sty` l.50-91 的逐行消费器依赖：
`\endlinechar=13`（默认）+ `\catcode`^^M=12` → 行尾 token 变 **cat12 数据
字符**，可作 `#1^^M` 的定界参数。NTex 永远分派为行尾语义 → 行尾 token
不进宏参数流 → `#1^^M` 永不匹配 → `\let\next=\beamer@processline` 循环
（实测 \next 被 let 148,524 次，line=0 即宏展开产物）。

### 修复方案（下刀执行）

行尾字节处理改为：
1. 字符码取 `\endlinechar` 参数值（而非硬编码 LF=10）；
2. catcode 按**该字符码查当前 catcode 表**（cat5 → 行尾空格语义；
   其他 cat → 数据字符 token 进入宏参数流，可作定界符）；
3. `-1` = 不追加（tex.web end_line_char_inactive）。

**风险评估**：TRIP/ETRIP 逐字节一致性敏感区（latex.ltx L299 `\^^J=active`
依赖"物理 LF≠endline_char 语义"的注释所描述的探针——修复后需重验
`\^^J=active` 场景：endlinechar=13 而 ^^J 是 char 10，二者不同码位，
active 探针不应受影响，但必须实测）。修复后须重生成 latex.fmt 并
全量跑 TRIP/ETRIP + corpus-probe。

## 附录 3：endlinechar 修复落地 + beamer 残余问题分层（同日）

- 修复已落地：`scan_token` 增 `endlinechar` 参数；行尾 token 字符码取
  `\endlinechar` 值、catcode 按该码位查当前表（cat5 维持空格语义；
  cat12 等产出数据字符可作 `#1^^M` 定界；<0 或 >0xFF 不追加）。
  12 处调用点（含 `\read`、主输入、utf8 测试）全部传参。
- 修复后 beamer.cls **活锁解除**（不再千万步空转，能跑到载入完成 +
  文档处理），但最小 beamer 文档仍未产出页面——**第二层问题**：
  beamer 消费器（`\beamer@startcomment`/`\beamer@processline`）启动/
  终止逻辑在 NTex 上不收敛（`\let\next=\beamer@processline` 25 万次，
  stop 判定串 `\beamer@stopdocument` 宏定义本身已验证正确）。
  涉及 `\string`+`\escapechar=-1`+`\ifx` 宏比较链，需下一刀单独排查。
- 回归验证：中文 article 论文（\section+\subsection+CJK 折行）✓、
  plain 回归 ✓、ntex-core 438 全绿 ✓、clippy ✓。

## 附录 4：list 机制缺口勘察（09-21，未修复）

- 症状：itemize/quotation/abstract 全报 "perhaps a missing \item"
  （itemize 单环境即炸；quotation 双环境报 2 次）
- 机制链：\list 尾 \global\@newlisttrue → \item 的 \@item[...] 走
  addpenalty 分支 → **\everypar{\@newlistfalse ...}**（latex.ltx L16181
  区）在段首清标志
- NTex 缺口：\everypar token 列表有存储/赋值/读取（primitive_toks_state.rs
  /save.rs），但**引擎从不触发执行**（三处 new_graf 开段点 sink.rs l.49-79/
  l.484/l.493 均无注入；无跨 crate 回调机制）
- 修复需要架构变更：NodeBuilder（ntex-layout）开段点 → 通知 Expander
  （ntex-core）展开 everypar_toks。方案：TokenSink trait 加
  take_par_begin() 查询方法，Expander 输出 token 前轮询
- 报错时刻的 {\u0000A} 组痕迹 = active 字符处理组（与根因无关的观察）
