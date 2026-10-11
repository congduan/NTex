# 宏包 CI 回归基线（auto-pkg 放量）

- 日期：2026-10-11；HEAD：`058aa78` + 刀H（工作树）；二进制：`target/debug/ntex-dvi`（mtime 2026-10-11 08:59）
- 跑法：`scripts/pkg-regression.py`——ntex-dvi `--auto-pkg --pkg-tlpdb ~/.TinyTeX/tlpkg/texlive.tlpdb --pkg-root ~/.TinyTeX` vs pdfTeX GT `-interaction=nonstopmode` 两趟；顺序执行、单文档 timeout 240s；本地 TL 树，无网络依赖。
- 判定口径：**PASS = ntex-dvi 写出 DVI 且 0 错**（`^!` 行计数）。GT 对照分级报告：GT 也错的构造不算 NTex 缺口。
- 文本层 = pymupdf 词序列 difflib 相似度（GT PDF vs DVI→dvipdfmx PDF）。
- 矩阵替换说明：任务单所列 caption/parskip 在本机 TL 树（2024basic）与 TLPDB 均缺席，双跑皆无源可比：parskip 弃用，caption 转作缺包负例（见下）；以 array/xcolor/tabularx/graphicx/etoolbox 替补凑足矩阵。
- auto-pkg 通路说明：13 包的 `.sty` 均已被既有检索路径（assets + 宿主 TinyTeX 树）命中，auto-pkg 探测零缺失、取料为 no-op——本机放量验收面因此落在「缺包即报 + 反查指引」契约（负例）与全矩阵 0 错判定上。
- **刀H 刷新说明**：本表 microtype 行为刀H 全量重跑实测；其余 12 行为 `058aa78`（刀E/F/G 后）全矩阵结果**原值保留未复跑**（刀H 未触碰其通路；刀E/F/G 探针 e2t/g4t/g8t 复跑 DVI 逐字节一致）。`--only` 子集跑请配 `--no-report`，否则本表被覆写成子集（write_report 恒整表重写）。

| 包 | NTex 错 | DVI | NTex 页 | GT 错 | GT 页 | 文本层 | 像素差 | 判定 | auto-pkg 取料 |
|---|---|---|---|---|---|---|---|---|---|
| geometry | 0 | 有 | 3 | 0 | 3 | 100.0% | — | **PASS** | （源已在路径） |
| amsmath | 0 | 有 | 1 | 0 | 1 | 92.6% | — | **PASS** | （源已在路径） |
| amssymb | 0 | 有 | 1 | 0 | 1 | 100.0% | — | **PASS** | （源已在路径） |
| booktabs | 0 | 有 | 1 | 0 | 1 | 100.0% | — | **PASS** | （源已在路径） |
| multirow | 0 | 有 | 1 | 0 | 1 | 100.0% | — | **PASS** | （源已在路径） |
| enumitem | 0 | 有 | 1 | 0 | 1 | 100.0% | — | **PASS** | （源已在路径） |
| hyperref | 0 | 有 | 1 | 0 | 1 | 93.0% | — | **PASS** | （源已在路径） |
| microtype | 2 | 有 | 1 | 0 | 1 | 95.7% | — | **FAIL(2)** | （源已在路径） |
| array | 0 | 有 | 1 | 0 | 1 | 96.8% | — | **PASS** | （源已在路径） |
| xcolor | 0 | 有 | 1 | 0 | 1 | 100.0% | — | **PASS** | （源已在路径） |
| tabularx | 0 | 有 | 1 | 0 | 1 | 94.9% | — | **PASS** | （源已在路径） |
| graphicx | 0 | 有 | 1 | 0 | 1 | 68.4% | — | **PASS** | （源已在路径） |
| etoolbox | 0 | 有 | 1 | 0 | 1 | 94.7% | — | **PASS** | （源已在路径） |

**PASS 12/13**（microtype 1128→2，仍 FAIL(2)：残留 `\MT@protrudechars`/`\MT@adjustspacing` 未注册，任务单定为照实记录勿硬修）。

## auto-pkg 缺包负例（契约验收）

- **caption**（TLPDB 无提供者）：✅ 通过——报错含包名+反查指引=`True`、未产 DVI=`True`。

## 非 PASS 明细（首错摘录）

### microtype — FAIL(2)（NTex 2 错，GT 0 错）
- NTex：`! Undefined control sequence.`（`\MT@protrudechars`，1 次）
- NTex：`! Undefined control sequence.`（`\MT@adjustspacing`，1 次）

（其余 12 包刀H 未复跑，明细见 `058aa78` 版本；均 PASS 无明细。）
