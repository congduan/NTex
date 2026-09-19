# fixtures/repro/ — 最小复现归档与回归流程

## 用途

每次调试定位的**最小复现**在此归档，与 ETRIP/TRIP 全量基准解耦——
**禁止**为复现而临时替换 `fixtures/etrip/etrip.tex`（历史教训：bakA..bakT 20 个备份）。

## 流程（新增复现 → 转单测）

1. 调试定位后：最小复现写入 `fixtures/repro/repro_<问题>.tex`（自包含：`\def\typeout` + `\font\trip=trip` + 复现段 + `\bye`）
2. 运行：`rm -f tripos && ./target/release/ntex-trip --driver ntex --test etrip fixtures/repro/repro_x.tex`
   （或复制到临时位置跑）
3. **修复验证通过后，把断言转成单测**（`crates/ntex-core/src/expand/tests.rs` 或
   `crates/ntex-layout/src/typeset/tests.rs`），命名 `repro_<问题>`——防止全量回归
4. 在 `docs/KNOWN-SIMPLIFICATIONS.md` 维护记录登记修复

## 清单

| 文件 | 问题 | 修复 commit |
|---|---|---|
| repro_discards.tex / repro_discards2.tex | `\splitdiscards` 数学模式 Missing number ×11（内部量无 `=` 误当赋值） | 9b0bc69 |
| repro_left.tex | `\left` 组 currentgrouptype 应为 16 | d980920 |
| repro_mkern_mu.tex | `\mkern-9mu`/`\mskip9mu` 报 Illegal unit（pt 上下文扫 mu） | 3a2cb63 |
| repro_math_in_dd.tex | 数学内 `$$` 丢失第二个 `$`（显示数学未开） | 3a2cb63 |
| repro_right_noleft.tex | `\right` 无 `\left` 硬错误中断（应恢复式 Extra \right） | 9b0bc69 |
| repro_expaf.tex | `$\expandafter$` 数学残留（参考 $\x 靠 \scriptfont 报错关闭） | 部分（3a2cb63 兜底；\scriptfont 检查未修） |
| repro_everymath_radical.tex | `\everymath{\radical"3}` 的 mathord 报错（l.412；\radical 字段扫描上下文敏感） | 已修（隐含组修复连带，未提交） |

## 证据产物（`pdf/`，2026-09-19 自仓库根目录迁入）

复现/差异定位时产出的**对照 PDF** 集中在此，避免散落在仓库根目录。它们不是"可再生
构建产物"——原输出环境（pdfTeX 版本、字体树）不可复现，故**入库留存**为来源证明；
`.gitignore` 的 `*.pdf` 全局规则管不到已追踪文件，因此为该目录补了负例。

| 文件 | 证据内容 | 首次入库 |
|---|---|---|
| `pdf/hb7.pdf` | `\def\usepkg#1#{OK-BODY}` 调用侧死循环的极简复现输出（`\usepackage` 缺包路径） | 211a54a |
| `pdf/stop.pdf` | `write`/`show` 组字符字面输出——tex.web `token_show` 语义对照（矩阵 hash_brace 簇） | 211a54a |
| `pdf/und.pdf` | `\ifx\<未定义>\@undefined` 在 harness 上下文判假（确凿引擎差异 A1.quindecies） | d712c35 |
| `pdf/resume-plain.pdf` | `samples/resume-plain.tex` 端到端渲染产出（源在 `samples/`，被 `ntex-wasm` `include_str!` 消费） | a766aba |
| `pdf/demo1-fixed.pdf` | `samples/demo1-fixed.tex` 端到端渲染产出（同上） | a766aba |

> 与 `repro_*.tex` 的分工：`*.tex` 是**可复跑的输入**，`pdf/` 是**不可复跑的输出快照**。

## 注意

- 复现文件可能含**故意未闭合**的组/数学（复现错误恢复链场景）——运行预期有报错输出，属正常
- 归档后如该场景已转单测，文件保留作为"来源证明"；单测是回归的正式防线
