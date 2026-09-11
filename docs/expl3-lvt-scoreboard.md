# expl3 官方测试跑分（l3kernel `.lvt`）

> **本文档是 expl3 攻坚的进度仪表盘。** 状态源：`scripts/lvt-run.py`。
> 上游权威数据：`latex3/latex3` 仓库 `l3kernel/testfiles/`（每例配 `.tlg` 期望转录）。

## 最新基线（2026-09-11 深夜）⭐ RAN 170/187 = 90.9%

| 判定 | 数量 | 占比 |
|---|---|---|
| **RAN** | **170** | **90.9%** ✅ |
| CRASH | 16 | 8.6% |
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

**位置**：`crates/ntex-core/src/expand/io.rs::scan_general_text`（L457+），
或 `fetch()` 的帧耗尽 pop 路径。

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
