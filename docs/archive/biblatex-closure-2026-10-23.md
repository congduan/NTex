# biblatex 资产闭环（2026-10-23）

## 定性

开工先执行 `git fetch origin && git log main..origin/main --oneline`，远端无新增提交；
工作区干净。以当前 HEAD `07a1751` 和独立目标目录 `/tmp/biblatex-target` 重建
`ntex-dvi`/`ntex-pdf`，避免陈旧二进制误判。

`ifthen.sty` 已存在于 `assets/tex-minimal/tex/latex/base/`，不是资产缺口。
最小 `ifthen` + `etoolbox` 探针：

```tex
\documentclass{article}
\usepackage{ifthen}
\usepackage{etoolbox}
\makeatletter
\patchcmd\ifthenelse{\let\isundefined\TE@undef}
  {\let\isundefined\TE@undef\message{PATCHED}}
  {\message{SUCCESS}}{\message{FAILED}}
\makeatother
\begin{document}Hello.\end{document}
```

pdfTeX 输出 `SUCCESS`，原 NTex 输出 `FAILED`。打开 etoolbox 的
`\tracingpatches` 后，失败点为 `macro cannot be retokenized cleanly`：原宏是
`\long\def\ifthenelse`，但 NTex 的 `\meaning\ifthenelse` 错印为 `macro:`，
漏掉 `\long macro:`。etoolbox 从 `\meaning` 文本重建宏并用 `\ifx` 核验，
重建宏丢失 long 属性，核验失败；biblatex 的 `Patching 'ifthen' package failed`
即此失败的上层报错。不存在缺少 `\AtEndOfPackage` 原语的问题。

## 修复

`ntex-core` 的 `\meaning`、`\show` 与槽显示统一输出 protected/long/outer
前缀，最后一个前缀与 `macro:` 间留空格，顺序对齐 pdfTeX。宏定义已有 long/outer
字段，未增加原语或修改 fmt codec，发行 `latex.fmt` 不变。新增 long 与
protected+long 的 pdfTeX 对照回归测试。修复后同一补丁探针输出 `SUCCESS`。

完整探针为 `\documentclass{article}\usepackage{biblatex}` 后以正文
`Hello.` 结束。`ntex-dvi` 0 条 `!` 错误，输出 234B、1 页 DVI；`ntex-pdf`
输出 39126B、1 页 PDF，PyMuPDF 提取文本为 `Hello.\n1\n`。
`probe.bbl` 不存在仅给出提示；引用解析未测试。真实运行约需 2 分钟，
`\begin{document}` 钩子中曾出现 watchdog 的慢调用提示，但最终完成且零错。

## 门禁与原始输出

| 项目 | 结果 | 原始输出 |
|---|---|---|
| `cargo test -p ntex-core -p ntex-layout` | core 582、layout 335，全部通过 | `/tmp/bib-test/core-layout-test.log` |
| `make instrument-check` | 13/13 | `/tmp/bib-test/gates/instrument-check.log` |
| ifthen 补丁探针 | `SUCCESS` | `/tmp/bib-test/ifthen-after.stdout` |
| biblatex 载入与 DVI | 0 错、1 页、234B | `/tmp/bib-test/after.stdout`、`/tmp/bib-test/out.dvi` |
| PDF | 1 页，文本 `Hello.\n1\n` | `/tmp/bib-test/pdf.stdout`、`/tmp/bib-test/pdf-text.log` |
| TRIP 构造 | 177B | `/tmp/bib-test/gates/trip/trip-build.log` |
| tripos（`--no-plain --dump trip.fmt` 后载入） | 12 页、750B、0 错，SHA-256 `574045fb…` 与基线相同 | `/tmp/bib-test/gates/trip/tripos.log` |
| 宏包回归 `--no-report` | 13/13 PASS | `/tmp/bib-test/gates/pkg-regression.log` |
| T2 testmath 两遍 | 40 页、114154B、0 错 | `/tmp/bib-test/gates/t2/pass1.log`、`pass2.log` |
| T3 fntguide 两遍 | 49 页、148526B、0 错，SHA-256 `52720321…` 与基线相同 | `/tmp/bib-test/gates/t3/pass1.log`、`pass2.log` |

TRIP 构造输入包含其预期的错误构造，验收是 177B；tripos 的零错单独核对。
