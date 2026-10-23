# ctex/ctexart Fandol native DVI 闭环（2026-10-23）

## 前提与定性

开工先 `git fetch origin`，`origin/main=3fd1477595dbd5f80f89b1e406137492a68074dd`，工作树干净；以该 HEAD、`CARGO_TARGET_DIR=/tmp/biblatex-target` 重建 `ntex-dvi`/`ntex-pdf`。先读 `AGENTS.md`、`plan.md` §0.4/第 24 条、`KNOWN-SIMPLIFICATIONS.md`、`tooling-trust.md`，并考古 2026-09-27 ctex 战役记录。历史中文样张用自定义 `\setCJKmainfont`/字体桥，并非 ctex 默认 Fandol 已通过。

初始假说“缺 Fandol OTF 使 ctex 报 unavailable”被源码与 A/B 证伪。`assets/tex-minimal/tex/latex/ctex/fontset/ctex-fontset-fandol.def` 第 29 行的第一臂对 **pdfTeX PDF 输出模式**直接调用 `\ctex_fontset_error:n {fandol}`；第二臂对 **pdfTeX DVI 输出模式**走 CJKutf8 + `ctex-zhmap-fandol.tex`。分支判定来自 `ctex.sty` 第 805–815 行的 `\sys_if_engine_pdftex:TF` 和 `\sys_if_output_pdf:TF`。这不是文件查找失败，也不是“ctex 必须 XeTeX”：ctex 上游明确提供 pdfTeX/DVI 兼容臂。`ctex-zhmap-fandol.tex` 自身注明 pdfTeX 不支持 OTF，DVI 时发 `pdf:mapline` special 交 dvipdfmx 载入 Fandol OTF。

NTex CLI 自动 LaTeX 路线原将 `\pdfoutput` 置 1，以便 graphics 宏包选 pdfTeX 驱动；尽管最终产物仍是 DVI，ctex 因此选到上游显式报错臂。原样最小探针报 `Critical Class ctexart Error`，后续 8 条 Undefined，DVI 640B。仅在源前加 `\pdfoutput=0` 即 0 条 `!`、1 页 1704B；该手动分支结果与修复后的原样探针 DVI SHA-256 **逐字节相同**（`cbc998951217996e963fd1506309971412feb16086adf718c1686a3990550a11`）。

## 改动与验收

`ntex-dvi/src/main.rs` 复用现有去注释 `scan_package_requests`，检测 `ctexart`/`ctexrep`/`ctexbook` 类或 `ctex` 包；自动载入 latex.fmt 时对这些作业将 `\pdfoutput` 驱动信号置 0。其它 LaTeX 作业仍置 1，源码中的显式 `\pdfoutput` 赋值仍可覆盖。新增扫描回归锁，含类选项、逗号包列表与注释负例。未改 fmt codec 或发行 fmt。

原样探针：

```tex
\documentclass{ctexart}
\begin{document}中文测试 Hello.\end{document}
```

以当前二进制运行 `ntex-dvi probe.tex after.dvi`：退出 0、`!` 计数 0、1 页 1704B；日志确认载入 `ctex-zhmap-fandol.tex`。`dvipdfmx -o after.pdf after.dvi`：退出 0、1 页 PDF 4564B；PyMuPDF 文本提取 `1\n中文测试Hello.\n`，中文可见、可提取。产物及原始转录在 `/tmp/ctex-fandol-2026-10-23/`（`after.log`、`after.dvi`、`after-pdf.log`、`after.pdf`）。

同根的 `\documentclass{article}\usepackage{ctex}` 探针也经自动分支零错出 1 页 1732B DVI；dvipdfmx PDF 的 PyMuPDF 文本为 `中文测试Hello.\n1\n`（`package-probe.log`、`package-pdf.log`）。

**边界**：仓库 `ntex-pdf` 目前不消费 CJK `pdf:mapline`；转换同一 DVI 会警告 `unisong*.pfb` 未找到，PyMuPDF 只得 `1\n- \x87 K Õ\nHello.\n`，不满足中文 PDF 验收。此次 PDF 验收走现成 dvipdfmx。Fandol OTF 不在仓库 `assets/`，本机 dvipdfmx 从已安装 TeX Live 字体树取字；故这是 **native + TeX Live 字体/dvipdfmx** 闭环，不能宣称自包含发行或 WASM 资产审计也已转 PASS。WASM bundle 资产与 PDF 后端映射分别另案；不能把 ctex/ctexart 与 fontspec、unicode-math、xeCJK 归并为同一个“XeTeX 必需”组。本刀的 XeCJK `\__iow_wrap_next_line:w` 勘察止损分支未触发。

## 门禁原始输出

| 门禁 | 结果 | 原始输出 |
|---|---|---|
| `cargo test -p ntex-dvi --bin ntex-dvi ctex_classes_select_dvi_fontset_branch` | 1/1 | `/tmp/ctex-fandol-2026-10-23/dvi-unit-test.log` |
| `cargo test -p ntex-core -p ntex-layout` | core 582、layout 335；其余集成测试全绿 | `/tmp/ctex-fandol-2026-10-23/core-layout-test.log` |
| TRIP 构造与 tripos | 177B；12 页/750B/0 错；两份 SHA 与上一刀相同 | `/tmp/ctex-fandol-2026-10-23/gates/trip/trip-build.log`、`tripos.log` |
| `scripts/pkg-regression.py --no-report`（已将当前二进制复制到脚本固定路径 `target/debug/ntex-dvi`） | 13/13 PASS，footmisc 负例 PASS | `/tmp/ctex-fandol-2026-10-23/pkg-regression.log` |
| T2 testmath 两遍 | 40 页/0 错 | `/tmp/ctex-fandol-2026-10-23/gates/t2/pass1.log`、`pass2.log` |
| T3 fntguide 两遍 | 49 页/148526B/0 错；SHA 与上一刀相同 | `/tmp/ctex-fandol-2026-10-23/gates/t3-exact/pass1.log`、`pass2.log` |

两份基线日志使用显式 `--fmt latex.fmt`，与自动格式检测口径不同。复用上一刀 T2 辅助文件并采用同一显式 fmt 后，T2 DVI SHA-256 与上一刀**逐字节一致**（`520d1392…`），原始输出 `gates/t2/explicit-fmt.log`。T3 从空辅助文件开始、同一显式 fmt 连跑两遍，SHA-256 与上一刀**逐字节一致**（`52720321…`）；先前把自动检测作业与显式 fmt 基线相比，出现 33B 假差，已用同口径复核证伪回归。
