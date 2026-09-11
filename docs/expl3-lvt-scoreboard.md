# expl3 官方测试跑分（l3kernel `.lvt`）

> **本文档是 expl3 攻坚的进度仪表盘。** 状态源：`scripts/lvt-run.py`。
> 上游权威数据：`latex3/latex3` 仓库 `l3kernel/testfiles/`（每例配 `.tlg` 期望转录）。

## 最新基线（2026-09-11 晚）⭐ 爆栈已修

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
