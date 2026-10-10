# 宏包 CI 回归基线（auto-pkg 放量）

- 日期：2026-10-10；HEAD：`c0849ed`；二进制：`target/debug/ntex-dvi`（mtime 2026-10-10 19:07）
- 跑法：`scripts/pkg-regression.py`——ntex-dvi `--auto-pkg --pkg-tlpdb ~/.TinyTeX/tlpkg/texlive.tlpdb --pkg-root ~/.TinyTeX` vs pdfTeX GT `-interaction=nonstopmode` 两趟；顺序执行、单文档 timeout 240s；本地 TL 树，无网络依赖。
- 判定口径：**PASS = ntex-dvi 写出 DVI 且 0 错**（`^!` 行计数）。GT 对照分级报告：GT 也错的构造不算 NTex 缺口。
- 文本层 = pymupdf 词序列 difflib 相似度（GT PDF vs DVI→dvipdfmx PDF）。
- 矩阵替换说明：任务单所列 caption/parskip 在本机 TL 树（2024basic）与 TLPDB 均缺席，双跑皆无源可比：parskip 弃用，caption 转作缺包负例（见下）；以 array/xcolor/tabularx/graphicx/etoolbox 替补凑足矩阵。
- auto-pkg 通路说明：13 包的 `.sty` 均已被既有检索路径（assets + 宿主 TinyTeX 树）命中，auto-pkg 探测零缺失、取料为 no-op——本机放量验收面因此落在「缺包即报 + 反查指引」契约（负例）与全矩阵 0 错判定上。

| 包 | NTex 错 | DVI | NTex 页 | GT 错 | GT 页 | 文本层 | 像素差 | 判定 | auto-pkg 取料 |
|---|---|---|---|---|---|---|---|---|---|
| geometry | 1 | 有 | 4 | 0 | 3 | 80.0% | 0.58% | **FAIL(1)** | （源已在路径） |
| amsmath | 0 | 有 | 1 | 0 | 1 | 92.6% | 0.44% | **PASS** | （源已在路径） |
| amssymb | 0 | 有 | 1 | 0 | 1 | 100.0% | 0.12% | **PASS** | （源已在路径） |
| booktabs | 0 | 有 | 1 | 0 | 1 | 100.0% | 0.07% | **PASS** | （源已在路径） |
| multirow | 0 | 有 | 1 | 0 | 1 | 100.0% | 0.02% | **PASS** | （源已在路径） |
| enumitem | 0 | 有 | 1 | 0 | 1 | 100.0% | 0.52% | **PASS** | （源已在路径） |
| hyperref | 0 | 有 | 1 | 0 | 1 | 79.1% | 0.92%† | **PASS** | （源已在路径） |
| microtype | 1128 | 有 | 1 | 0 | 1 | 80.4% | — | **FAIL(1128)** | （源已在路径） |
| array | 0 | 有 | 1 | 0 | 1 | 96.8% | 0.49% | **PASS** | （源已在路径） |
| xcolor | 0 | 有 | 1 | 0 | 1 | 100.0% | — | **PASS** | （源已在路径） |
| tabularx | 0 | 有 | 1 | 0 | 1 | 94.9% | 1.44% | **PASS** | （源已在路径） |
| graphicx | 0 | 有 | 1 | 0 | 1 | 68.4% | — | **PASS** | （源已在路径） |
| etoolbox | 0 | 有 | 1 | 0 | 1 | 94.7% | 0.16% | **PASS** | （源已在路径） |

† 页面尺寸不合：GT 裸 article 在本机 TinyTeX 缺省 **A4**，载入 hyperref/xcolor 后 GT 侧被翻成 **letter**（612×792），NTex/dvipdfmx 侧保持 A4——该列像素差已按左上交集裁剪对照，不代表内容全分歧。

**PASS 10/13**。

## auto-pkg 缺包负例（契约验收）

- **caption**（TLPDB 无提供者）：✅ 通过——报错含包名+反查指引=`True`、未产 DVI=`True`。

## 非 PASS 明细（首错摘录）

### geometry — FAIL(1)（NTex 1 错，GT 0 错）
- NTex：`! LaTeX Error: Missing \begin{document}.`

### microtype — FAIL(1128)（NTex 1128 错，GT 0 错；刀F 后载入不再 fatal）
- 刀F（2026-10-10）：`^^` 置换产物缺 reswitch 重分派（input.rs）——`\MT@fix@catcode{17}{14}`
  后行首 `^^Q` 注释失效，`\MT@ifdefined@c@T` ^^X/^^Q 双引擎变体全进 def 体，下游
  `\MT@prlist@family@` 自引用 edef 栈超限 fatal。修复后整篇跑通（1 页 DVI、文本层 80.4%），
  719→1128 是"fatal 截断"→"全文累积"的口径变化，非回归。
- 残差（记录不顺手修）：pdfTeX 原语缺口为主——`! Undefined control sequence`
  337 = `\rpcode`162+`\lpcode`114+`\pdfmatch`56+`\pdffontexpand`3+MT 内部 2；
  `Missing number`（503， protrusion 尺寸链连锁）、`Missing font identifier`（171）。
  下一刀 = pdfTeX 字符 protrusion 原语族（`\lpcode`/`\rpcode`/`\pdfmatch`）。

### xcolor — PASS（刀E 后 NTex 0 错 = GT 平齐；原 FAIL(10)）
- 根因：`scan_dimen` 数字循环缺宏展开臂 + `数量×内部量` 截断 + print_scaled 舍入偏差
  （`\rshift@`/`\lshift@` 定点小数族地基，见刀E 提交）。
- 文本层 93.9%→100.0%（颜色表达式数值全对齐）。

### graphicx — PASS（刀G 后 NTex 0 错 = GT 平齐；原 FAIL(40)）
- 根因：`scan_dimen` 符号循环对 `+` 的前瞻消解偏离 tex.web（非数字臂把 `+` 退回流头）
  + 内部量单位探针缺 blank-skipping（`数量␣内部量` 的空格被拒）。
- 文本层 45.6%→68.4%（`\rotatebox` 角度计算链 trig→graphicx 全通）。

