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
