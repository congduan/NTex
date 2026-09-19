# docs/archive — 历史归档

> **本目录只读。** 这里存放已冻结的历史文档：逐刀战报、勘察全文、跑分基线、一次性评审。
> 进度类内容已全部抽取到 [../../plan.md](../../plan.md)；此处保留原文以便全文检索
> （定位某刀根因链、tex.web 锚点、复现步骤时来这里 grep）。
>
> **归档纪律**：归档时**保留原文件名**，不做日期前缀——代码注释与旧文档里的
> `xxx.md §N` 引用仍可按名检索到。若同一文件有多个时点快照，才加 `-<日期>` 后缀。

## 归档清单

### 进度类（内容已并入 plan.md）

| 文件 | 内容 | 最后更新 | 归档日 |
|---|---|---|---|
| `plan-full-2026-09-19.md` | `plan.md` 重构前全文（各里程碑逐项实施步骤与战报） | 2026-09-19 | 2026-09-19 |
| `ETRIP-primitives.md` | ETRIP 原语逐条状态清单（A 组 42/42、B 组 36/36、C 组已接线 + M4 基线） | 2026-09-18 | 2026-09-19 |
| `latex-feasibility.md` | LaTeX 兼容战役活文档（A0 当前阻塞点 + A1.* 逐刀 ~30 节） | 2026-09-17 | 2026-09-19 |
| `latex-feasibility-full-2026-09-11.md` | 更早的 §8–§37 逐刀记录全文 | 2026-09-11 | 2026-09-11 |
| `latex-feasibility-relay28b.md` | 第二十八刀接力简报（`\expanded` 实参 IPN×256 根因） | 2026-09-06 | 2026-09-19 |
| `expl3-real-scoreboard.md` | expl3 真实跑分与逐刀记录（第一~二十五刀） | 2026-09-18 | 2026-09-19 |
| `expl3-lvt-scoreboard.md` | l3kernel 官方套件（`.lvt` × 187）跑分 + 载入终点复测 | 2026-09-18 | 2026-09-19 |
| `expl3-real-workload.md` | expl3 真实工作量评估（187 例分母辨析、载入点探针） | 2026-09-12 | 2026-09-19 |
| `plain-format-survey.md` | 格式预载可行性勘察（G0–G3 实测 + 接入路径候选） | 2026-09-11 | 2026-09-19 |
| `output-routine-survey.md` | 输出例程勘察（latex.ltx 三层清单 + 罚分协议 + 刀 1–5） | 2026-09-11 | 2026-09-19 |
| `plain-format-g2-record-2026-09-11.md` | 格式预载 G2 阶段快照 | 2026-09-11 | 2026-09-11 |
| `output-routine-blades-2026-09-11.md` | 输出例程逐刀快照 | 2026-09-11 | 2026-09-11 |
| `halign-survey.md` | `\halign`/`\valign`/`\insert`/数学矩阵战役前置勘察（含 S1–S7 刀序与 tex.web 裁决） | 2026-09-07 | 2026-09-19 |
| `trip-missing-primitives.md` | TRIP 缺原语清单（Undefined 124 → ~10） | 2026-09-03 | 2026-09-19 |
| `primitive-correctness-checktable.md` | 原语语义正确性四层防线盘点 | 2026-09-06 | 2026-09-19 |
| `corpus-probe-2026-09-19.md` | corpus 探针实录（2/8 PASS 原始记录与判读） | 2026-09-19 | 2026-09-19 |
| `REVIEW-2026-08-23.md` | 代码审查（A 正确性 / B 性能 / C 工程健壮性 / D 语义简化 / F 优先级路线图） | 2026-08-23 | 2026-09-19 |
| `schema-dump.txt` | 早期方案摘要草稿（内容已并入 `idea.md` 与 `README.md`） | 2026-09-03 | 2026-09-19 |

### 仍在 docs/ 下活跃的长期参考（未归档）

- [../KNOWN-SIMPLIFICATIONS.md](../KNOWN-SIMPLIFICATIONS.md) — 技术债清单（改动前先查表）
- [../tooling-trust.md](../tooling-trust.md) — 定位基础设施与仪器可信度
- [../MATH-STATE-MACHINE.md](../MATH-STATE-MACHINE.md) — 数学状态机语义规格
- [../blocker-history.tsv](../blocker-history.tsv) — 阻塞点看板数据

## 怎么用

- 想查"某个数字从哪来" → 先看 `plan.md §7 溯源表`，再进本目录。
- 想查"某刀当时怎么定位的 / tex.web 依据是哪一节" → 在本目录全文 grep
  （`grep -rn '<关键词>' docs/archive/`）。
- 想恢复一份被归档文档 → `git mv docs/archive/<file> docs/<file>`，
  并在 `plan.md` 与 `AGENTS.md` 同步登记。
