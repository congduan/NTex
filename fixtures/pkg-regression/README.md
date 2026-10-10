# pkg-regression fixtures

宏包 CI 回归集样张（刀D，2026-10-10）。每包一个最小文档：`\documentclass` +
`\usepackage` + 一处真实用法，由 `scripts/pkg-regression.py` 双跑
（ntex-dvi `--auto-pkg` vs pdfTeX GT）并汇总进 `docs/pkg-regression-baseline.md`。

- 判定口径：PASS = ntex-dvi 写出 DVI 且 0 错；GT 对照分级报告，GT 也错的构造不算缺口。
- 样张刻意避开 `\label`/`\ref`（aux 协议噪声）与中文（CJK 走 ctex 独立战线）， isolate 包语义。
- caption 在本机 TLPDB 无提供者，作缺包负例（`scripts/pkg-regression.py` 内 `NEGATIVE_PKGS`）。
