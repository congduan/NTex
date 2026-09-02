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

## 注意

- 复现文件可能含**故意未闭合**的组/数学（复现错误恢复链场景）——运行预期有报错输出，属正常
- 归档后如该场景已转单测，文件保留作为"来源证明"；单测是回归的正式防线
