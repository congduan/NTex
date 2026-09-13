# expl3 官方测试跑分（l3kernel `.lvt`）

> **本文档是 expl3 攻坚的进度仪表盘。** 状态源：`scripts/lvt-run.py`。
> 上游权威数据：`latex3/latex3` 仓库 `l3kernel/testfiles/`（每例配 `.tlg` 期望转录）。

## 最新基线（2026-09-13 复测·第二刀）首错前移 l.9320 → l.18141——outer 双槽位修复落地

| 判定 | 一刀前 | 本轮 |
|---|---|---|
| STACK | 187 | 187（判定分布未变，首错前移）|

**主控独立复测**（8217f86 + 本刀工作树，2026-09-13 10:40）：单例 m3basics001
转录 4933 行，首错 = `Use of \??? doesn't match its definition`（fp 模块），
总错误 625 → 623；`\???` 签名 15 → 15（非本刀回归铁证）、
`Forbidden control sequence` 2 → 1（靶错消除）。全量 `lvt-run.py --all`
仍 187 STACK（fp 首错阻断载入，到不了 END-TEST-LOG）。

**本轮修复（2026-09-13，定性见 `docs/latex-feasibility.md` §A3）**：上一节
两个候选根因里，**候选 2 成立但机理更具体**——不是 scanner_status 误判，而是
**eqtb 单槽合并了 active char 与同名单字符 cs**：plain.tex L20
`\outer\def^^L{\par}` 写进 active 槽的 outer 被 cs 形式 `\^^L` 继承，expl3
L9320 `\char_set_catcode_active:N \^^L` 实参扫描即报
`Forbidden control sequence`（pdfTeX 同点 0 错）。修复 =
`MacroDef::active_slot` 位 + 统一判据 `is_outer_for_token`（**outer 仅在
token 形式 ↔ 槽写入形式一致时可见**，tex.web L242 双槽语义的压缩替身），
8 处 token 面检查点统一；`.fmt` v16 序列化一字节伴随（codec.rs）。

**效果**：载入首错 char 模块 l.9320 → **fp 模块 l.18141**
（`\fp_const:Nn \c_e_fp { 2.718 2818 2845 9045 }`，症状
`! Use of \??? doesn't match its definition.`——msg 渲染机症状，真偏差在
`\__fp_parse:n`；旧转录同签名已有 15 条，非本刀回归）。全载探针
错误 625 → 623。**判定分布未翻盘**（fp 首错仍阻断 END-TEST-LOG），
按「首错单调前移」口径记一刀；下一刀靶子见 §A3.2。

## 09-13 复测·一（历史）⚠ STACK 187/187——shim harness 与引擎修复批次脱节

| 判定 | 数量 |
|---|---|
| STACK | **187**（全部）|

**定性（2026-09-13 bisect 实测）**：回归**不在 09-13 拉取**——f18a343
（拉取前 HEAD）配同一 harness 同样 STACK（m3basics001 单例探针；main 全量
187/187 STACK）。时间线与已证事实：

- 09-11 深夜跑分板 RAN 180 → **09-12 06:26 `46ab227`**：配当前 harness 已
  STACK（旧签名 `5001 帧 /\l__iow_line_part_tl`，l.25851）——与
  blocker-history 同 ts 的「`\read` 流未打开 REGRESSION」同期；
- 09-12 白天六刀（`261544d..f18a343`，scanner_status/outer/行尾/everyeof）
  后，失败**换签名**：`\q_stop 递归`，转录首错 l.48
  `Forbidden ... \char_set_catcode_active:N`（→ `Improper alphabetic
  constant` 级联，625 错）；
- 窗口内 `scripts/lvt/` 与 `lvt-run.py` **零改动**（diff --stat 为空）；
- **engine 裸跑健康**：main 不载 expl3 直跑 m3basics001 → END-TEST-LOG ✅
  rc=0；`latex_probe --initex latex.ltx` 无爆栈，终点 = iow_wrap（A1.vicies
  记录一致）。

**两个候选根因（均未证伪，待定性）**：
1. **shim 脱节**——09-12 批次只对齐了 latex_probe/单元测试口径，lvt shim
   未同步（正例：09-11 RAN 180 之后 shim 从未随引擎批次回归过）；
2. **引擎 outer 判据过宽**——首错恰是 `Forbidden ... while scanning use of
   \char_set_catcode_active:N`，outer 正是批次动过的语义；pdfTeX 对
   expl3-generic 全程 0 错，若对拍同点 NTex 多报 Forbidden，则是引擎侧
   回归（261544d/63db4b6/c9ca9b4 三刀嫌疑）。

**下一刀**：① 最小探针对拍 pdfTeX（`expl3-code.tex` 载入前 N 行，比
Forbidden 首错点）；② 探针显示引擎差异 → 修引擎；对拍一致 → 修 shim，
重跑全量重立基线。**在重立基线前，本板数字（含 09-11 RAN 180）与引擎
 HEAD 不可比**；期间进展看 `latex_probe` 终点推进 + 376 单测口径。

## 09-11 基线（历史，待 shim 修复后重立）⭐ RAN 180/187 = 96.3%

| 判定 | 数量 | 占比 |
|---|---|---|
| **RAN** | **180** | **96.3%** ✅ |
| CRASH | 6 | 3.2% |
| STACK | 1 | 0.5% |


### 本战役累计（起点 → 现在）

| 判定 | 起点 | 现在 |
|---|---|---|
| STACK-END | **112** | **0** ✅ |
| RAN | **0** | **170** ✅ |

### 三个真修复（都改变了通过率）

| # | 修复 | 效果 |
|---|---|---|
| 1 | shim 重复载入 harness 致 `\END` 自递归 | STACK-END 112 → 0 |
| 2 | shim 补 `\ExplSyntaxOn`/`\ExplSyntaxOff`（LaTeX 内核提供，plain 无）| RAN 114 → 135 |
| 3 | **`\input` 文件名扫描漏收非 Letter/Other catcode**（tex.web L10210 判据为 `cur_cmd>other_char`）→ 路径含 `_` 被截断 | RAN 135 → **170** |

### 剩余 16 例 CRASH 的 8 个簇

| 例数 | 首错 |
|---|---|
| **8** | `! 实参扫描到输入末尾`（m3fp-logic004/m3int001/m3int003/m3prg001/m3skip002/m3skip006/m3tl002/m3tlist002）|
| 2 | `forbidden control sequence \+`（outer 宏出现在展开上下文）—— m3fp-parse002/m3regex005 |
| 1 each | 组未闭合 / `\f` outer / 双重上标 / `\CS 赋值 RHS` / `\CS 需要寄存器参数` |

**⭐ 根因已锁定（1 行复现）**：8 例「实参扫描到输入末尾」

```tex
\immediate\write128{~}      % ← FAIL
\immediate\write128{a}      % ok
\immediate\write128{~a}     % ok
```

**关键事实（推翻了先前 3 行的中间结论）**：
- **不需要 `\ExplSyntaxOn`** —— 裸 `\immediate\write128{~}` 即可复现；
- **不是 `collect_undelimited_arg`** —— 报错来自 `\write` 的参数扫描
  （`io.rs::scan_general_text`，L457+ 的 `fetch()` 返回 None）；
- **规律**：参数组内**只有 active 字符**（`~`，cat 13）时失败；后面跟任何
  字符（`~a`）则通过。

**机制推测**：`scan_general_text` 展开组内容，`~`（active）展开为
`\nobreakspace` 之类 → 压帧 → 该帧读尽后 `fetch()` 立刻返回 None，
**在「展开结果帧刚耗尽、还需读下一个 token」的边界上配对不齐**。

**位置（精确到函数）**：`crates/ntex-core/src/expand/scan.rs::scan_number`（不是
`scan_general_text`）——`exec_write`（io.rs L379-384）先 `scan_number()` 读流号再
`scan_general_text()` 读文本；报错文本 `实参扫描到输入末尾` 来自
`macros.rs::collect_undelimited_arg` L300，只有 `scan_number` 路径经过它。

`\immediate\write128{~}` 的执行序：
  1. `\immediate` 置前缀（primitive_io.rs）
  2. `exec_write` → `scan_number()` 读 `128`
  3. `scan_general_text()` 读 `{~}` ← **此处失败**

#### ⭐⭐ 确凿引擎差异（pdfTeX ground truth）

**最小对照**（纯 plain，无 harness）：
```tex
\catcode`\~=\active
\def~#1{[TIE:#1]}          % active 字符是**带参宏**（harness 里 `~` 正是 `\def~#1{\accent"7E #1}`）
\immediate\write128{~}    % 实参扫描时 `~` 需 #1，输入已耗尽
\immediate\write128{[AFTER]}
\end
```

| | pdfTeX | NTex |
|---|---|---|
| 结果 | `Runaway argument` + 可恢复错误，**继续执行** → 输出 `[AFTER]` ✅ | `! 实参扫描到输入末尾` → **作业终止** ❌ |

**结论：这是错误恢复语义的缺失** —— tex.web 里实参扫描遇输入耗尽报
`Runaway argument`（**可恢复**，`error` + 继续），NTex 当成**致命错**。

**harness 里的表现**：`~` 被定义成 `\def~#1{\accent"7E #1}`（LaTeX 重音宏），
于是任何「`~` 作为 write/参数组最后一个 token」的写法都终止整个测试 ——
8 例 CRASH 全因此。

**修法方向**：参照 tex.web `macro_call` 的实参扫描 EOF 路径（`scan_toks` 的
`Runaway argument` 恢复），把 NTex 的
`Error::invalid_input("实参扫描到输入末尾")`（macros.rs L300/L309）
改为**可恢复**：报 `Runaway argument` 到转录 + 按空实参继续。

**待查**：`scan_number_inner`（L61+）的 token 循环在读完 `128` 后，
为何会走到 `collect_undelimited_arg` 并因 `~`（active 展开压帧）判输入耗尽。

**影响 8 例**：m3fp-logic004 / m3int001 / m3int003 / m3prg001 / m3skip002 /
m3skip006 / m3tl002 / m3tlist002

**旧记录（已被本节取代）**：" + old.split("
")[0].replace("**下一刀（已收窄到 3 行最小复现）**：", "") + "



```tex
\ExplSyntaxOn
\TYPE{~}      % ← 失败；`\TYPE{ ~~ }`、`\TYPE{a~b}` 均通过
\ExplSyntaxOff
```

**规律**：active 字符 `~`（cat 13）**作为实参的最后一个 token**（后面紧跟 `}`）时，
实参扫描取不到 token → `! 实参扫描到输入末尾`。`~` 后有别的字符则正常。

**位置**：`crates/ntex-core/src/expand/macros.rs::collect_undelimited_arg`
L296-310（跳过前导空格后 `fetch()` 返回 None）。推测：active char 展开
（`~` → `\nobreakspace` 之类）压帧后，`fetch()` 在帧耗尽处的 pop/读取配对不对，
使下一个 `fetch()` 误判输入耗尽。

**影响**：8 例（m3fp-logic004/m3int001/m3int003/m3prg001/m3skip002/m3skip006/
m3tl002/m3tlist002）—— 都是 `~` 出现在参数/展开上下文末尾的写法。

#### 原「下一刀」记录（保留）

**旧记录：8 例「实参扫描到输入末尾」**（最大簇，单一根因概率高）。
已收窄到 `m3int001.lvt` L171-176 `\int_to_arabic:n { ( 2+7 ) / 3 }` 一带；
该表达式在 `\ExplSyntaxOn` 下（`_`/`:` 为 letter）展开时与 `\TYPE` 交互出错。

## 历史基线：爆栈修复（2026-09-11 晚）

| 判定 | 数量 | 说明 |
|---|---|---|
| **RAN** | **114** | 跑通 harness（修前为 0）|
| CRASH | 72 | 中途致命错误（下一战场）|
| STACK | 1 | 中途栈超限 |
| ~~STACK-END~~ | **0** ✅ | **修前 112 —— 已全部消除** |

### 根因与修法

**不是引擎 bug，是 shim 设计缺陷**：`lvt-shim.tex` 自己 `\input regression-test.tex`，
而**用例又 `\input{regression-test}`** → harness **载入两次** →
第二次时 BLOCK2（L87-91）走真分支 `\let\end\END`，而 `\END` 宏体末尾的
`\@@@end` 已被 BLOCK1 绑到**当时的 `\end`**（此时已是 `\END`）→
**`\END` 自我递归** → 输入栈爆。

**修**：shim 不再代劳载入 harness（由用例自己载入，与官方 l3build 驱动语义一致）。

**最小复现（修前，5 行）**：
```tex
\documentclass{minimal}
\input{regression-test}
\begin{document}
\end
```

## 为什么改用官方测试套件（2026-09-11）

此前 expl3 攻坚方式是「跑 4 万行 `latex.ltx` → 看错误 → **手写探针猜根因**」。
两个致命缺陷：

1. **没有分母** —— 无法回答「expl3 还差多少」；
2. **探针错误率高** —— 实测一轮内 **3 次误判**（详见 `docs/tooling-trust.md`）。

官方套件解决两者：**187 个可跑用例**（l3kernel，分模块）+ **`.tlg` 权威期望**，
用例由 LaTeX 项目维护，**不需要我们造探针**。

## 怎么跑

```bash
scripts/lvt-run.py --fetch              # 抓 l3kernel 测试（GitHub latex3/latex3）
scripts/lvt-run.py --list               # 列出全部用例
scripts/lvt-run.py m3basics001          # 跑单个
scripts/lvt-run.py --all --jobs 2       # 全量跑分
```

产物：`/tmp/lvt-results.tsv`（每例的判定，跨轮 diff 用）。

## 机制（为什么能跑）

`.lvt` 头部是 `\documentclass{minimal}` + `\input{regression-test}` —— 需要 LaTeX
内核，而 LaTeX 内核依赖 expl3（循环依赖）。

解法：`scripts/lvt/lvt-shim.tex`（**plain 垫片**）提供那几个 LaTeX 符号
（`\documentclass`/`\begin`/`\end`/`\makeatletter`/`\@undefined`），再载入官方
`regression-test.tex`（**纯 expl3 + TeX 原语**，不依赖 LaTeX 内核）。

**踩过的坑**（已修，勿重犯）：
- `\def\end#1{}` 会**遮蔽原语 `\end`**（`regression-test.tex` 的
  `\let\@@@end\end` 要绑原语）→ 用 `\futurelet` 只吞 `{document}` 参数组；
- `\input{\LVTFILE}` **不可**：TeX 的 `\input` 是文件名扫描，不吃 `{}` 分组；
- `\@undefined` 必须 `\let` 成 `\relax`（LaTeX 的「未定义哨兵」约定）；
- **NTex 转录走 stderr**（不是 stdout），跑分器必须两侧都收；
- 判据顺序：**先看 `END-TEST-LOG` 再看致命错** —— 「跑到末尾但中途报错」
  （STACK-END/CRASH-END）是 expl3 引导可用的**强信号**，不等于没跑起来。

## 判据词汇

| 判定 | 含义 | 信号强度 |
|---|---|---|
| `STACK-END` | **跑到末尾**，触输入栈超限 | ⭐ 机制基本可用，只差栈 |
| `CRASH-END` | **跑到末尾**，中途有致命错 | ⭐ 同上 |
| `STACK` / `CRASH` | **中途**终止 | 有早期硬阻塞 |
| `NO-END` | 未跑完且无致命错 | 待查 |
| `RAN` / `PASS?` | 跑完且与 `.tlg` 粗对齐 | — |

## 基线（2026-09-11，187 例）

| 判定 | 数量 | 占比 |
|---|---|---|
| **STACK-END** | **0** | **59%** |
| **CRASH** | 73 | 39% |
| NO-END | 2 | 1% |
| STACK | 1 | 1% |

### 判读（重要）

**59% 的官方用例能跑到末尾** —— 说明 expl3 的宏机制**大部分已能工作**，
NTex 缺失的不是"expl3 基础"，而是**少数几处引擎级偏差**在放大。

**单一最大阻碍 = 输入栈超限**（0/187 = 60%），与 `latex.ltx` 的爆栈
**同一根因**（`\tex_edef:D` 单 token 递归，见 `docs/latex-feasibility.md`
§A1.undecies）。**修掉它，通过率可能量级跃升** —— 这是当前**最高杠杆**的一刀。

## 下一刀（有据可依）

`m3basics001` 是**最小靶子**（162 行，官方最基础的 `\cs_if_exist_use:` 等），
且已定位为 `STACK-END`。用它做定点调试，比 4 万行 `latex.ltx` 高效得多：

```bash
scripts/lvt-run.py m3basics001                    # 看是否 STACK-END
NTEX_TRACE_JSONL=/tmp/m1.jsonl ntex-dvi ...       # 拿调用链
scripts/trace-view.py /tmp/m1.jsonl --spikes      # 找自我复制宏链
```

## KPI 纪律

- **进度指标 = 用例判定分布的变化**（`STACK-END`/`CRASH` → `RAN`），
  不是「latx.ltx 跑到第几行」；
- 每轮跑分后把 `lvt-results.tsv` 与本表对比，**回退必须解释**；
- 与 `make blocker-track` 互补：那个看「单文件阻塞点位置」，这个看「用例通过面」。

---

## ⚠ 指标解读的关键限制（2026-09-11 实测，必读）

**当前的 `DIFF` 数字不能直接读作「引擎语义缺口」。** 原因：

`.lvt` 用例假设 **expl3 已载入**（l3build 用 `--fmt=...latex` 预载 format；
`l3kernel/build.lua` 的 `checkdeps = { }` 印证无需额外包）。但我们的垫片
**没有载入 expl3**——而用例正文调用的是 expl3 kernel 函数：

```tex
\TEST{cs~if~exist~use}{
  \cs_if_exist_use:N   \TRUE          ← expl3 kernel 函数
  \cs_if_exist_use:NF  \TRUE { \ERROR }
  ...
```

实测 `m3basics001` 转录：
```
TEST 1: cs if exist use          ← 标题已对（`~`=cat10 修复后）
! Undefined control sequence.
\cs_if_exist_use:N               ← 函数未定义 → 落字面文本
```

**全 187 例共用到 2595 个不同的 expl3 函数** —— 无法靠垫片补齐（那就是 expl3 本身）。

### 因此当前指标的真实含义

| 判定 | 读作 |
|---|---|
| `RAN` | harness 跑完了（**不代表任何语义正确**）|
| `DIFF` | 输出与 `.tlg` 不符 —— 但**大部分差异来自「expl3 未载入」而非引擎缺陷** |
| **`PASS`** | **真正有意义的目标** —— 需先让 expl3 载入 |

### 正确的推进顺序

1. **先让 expl3 载入**（`\input expl3.ltx` 走通）—— 这正是战役主线目标
2. 载入后重跑 `lvt-tlg-diff`，此时的 `DIFF`/`PASS` 才**真正度量引擎语义**
3. 垫片类修复（`~`=cat10、`\debug_on:n`、变量名去 `_`）是**必要的基础**，
   但**不足以让 PASS > 0**

**教训**：`DIFF 177/189` 这个数字很漂亮（像"只差一点点"），实际是
**「expl3 缺席」的度量**，不是「引擎快好了」的度量。**指标要有正确的读法。**
