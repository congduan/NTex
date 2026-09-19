# Corpus 勘察记录（2026-09-19）

本轮在 `b08d77d` 后运行：

```bash
~/.venvs/pixtools/bin/python scripts/corpus-probe.py
```

结果：`2/8 PASS`，`0 EMPTY`。脚本会更新 `fixtures/corpus/report.json` 并清理部分
旧 DVI 产物；按本仓库本轮任务约束，已恢复 `fixtures/` 下所有变更，本文件仅记录观察值。

## PASS

- `math/basic-expressions.tex`：DVI/PDF/ink 均产出。
- `math/symbols-matrix.tex`：DVI/PDF/ink 均产出。

## FAIL 首错

- `latex/sample2e.tex`：`Incomplete \iftrue; all text was ignored after line 204`，结束时仍在
  group level 2，且未 shipout。
- `latex/small2e.tex`：DVI 阶段 90s timeout。
- `latex/testpage.tex`：在交互式纸张选项提示后触发 `\read` 流未打开。
- `plain/letterformat.tex`：未产出页面，仍是缺少 `\shipout` 类债。
- `plain/list.tex`：未产出页面，仍是缺少 `\shipout` 类债。
- `plain/plain.tex`：未产出页面，仍是缺少 `\shipout` 类债。

## 判读

本轮没有挑到“一眼小修”的 2-3 个独立点：`small2e` timeout 与 `sample2e` 条件/分组收尾
都需要继续定位；plain 三例集中在输出/shipout 口径；`testpage` 的 `\read` 交互恢复语义可作为
后续较小候选。
