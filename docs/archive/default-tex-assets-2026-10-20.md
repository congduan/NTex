# 默认常用宏文件资产与兼容性记录（2026-10-20）

## 发行范围

- 清单：27 个宏包入口、3 个文档类；完整上游依赖图 174 包，叠加既有 blindtext 后 ntex.lock 共 175 包。
- SHA-512 校验后取料；导入 3,533 个新文件，tex 树合计 3,996 文件/3,932 个主名。
- 运行树取 tex/**，排除 *-dev；外部引擎、安装脚本和字体集合不进入本次发行范围。
- runtime-assets.tsv 校验 3,850 个上游路径；15 个基础版本保留项（旧 l3kernel/backend）同时记录上游和发行 SHA-256。其余原有零散资产继续保留。
- 宏文件总字节 53,598,303；实际 Tauri bundle 62,364,073 字节，3,932 个主名内容全部在包内。
- root ntex.lock 固定取料源；文件级清单固定实际输出。通过 check-default-tex-assets.py 可以检测丢文件、字节漂移，并从已校验暂存树恢复。

## WASM 最小加载与出页

使用实际 ntex-tauri --dump-bundle 导出的资产包和 ui/pkg/ntex_wasm_bg.wasm，
每项单独 Node 进程，12 秒限时；宏包用 article + usepackage，文档类单独 documentclass，
正文均为 Asset probe.。PASS 仅表示零错误且有页，不代表所有宏功能已通过兼容验收。

| 入口 | 最小加载/出页 | 首个错误或限制 |
|---|---|---|
| `amsthm` | PASS | — |
| `mathtools` | PASS | — |
| `siunitx` | TIMEOUT | 12 秒未完成 |
| `enumitem` | PASS | — |
| `fancyhdr` | PASS | — |
| `titlesec` | PASS | — |
| `setspace` | PASS | — |
| `parskip` | PASS | — |
| `float` | PASS | — |
| `caption` | PASS | — |
| `subcaption` | PASS | — |
| `makecell` | PASS | — |
| `tabularray` | PASS | — |
| `hyperref` | PASS | — |
| `cleveref` | PASS | — |
| `natbib` | PASS | — |
| `biblatex` | ERROR | ! Package biblatex Error: Patching 'ifthen' package failed. |
| `listings` | PASS | — |
| `fancyvrb` | PASS | — |
| `minted` | ERROR | Error: 非法输入：输入栈超限（10001 帧 > 10000）——展开/参数扫描疑似无终止条件（首现场：! LaTeX Error: Missing \begin{document}.） |
| `tikz` | ERROR | Error: 非法输入：输入栈超限（10001 帧 > 10000）——展开/参数扫描疑似无终止条件（首现场：! LaTeX Error: Missing \begin{document}.） |
| `pgfplots` | ERROR | Error: 非法输入：输入栈超限（10001 帧 > 10000）——展开/参数扫描疑似无终止条件（首现场：! LaTeX Error: Missing \begin{document}.） |
| `ctex` | ERROR | ! Critical Package ctex Error: CTeX fontset `fandol' is unavailable in |
| `xeCJK` | ERROR | ! File ended while scanning use of \__iow_wrap_next_line:w. |
| `babel` | PASS | — |
| `fontspec` | ERROR | Error: 非法输入：cannot \read from terminal in nonstop modes（\read 流 0 未打开）（首现场：! Fatal Package fontspec Error: The fontspec package requires either XeTeX or） |
| `unicode-math` | ERROR | ! Package unicode-math Error: Cannot be run with pdftex! |
| `beamer` | ERROR | Error: 非法输入：输入栈超限（10001 帧 > 10000）——展开/参数扫描疑似无终止条件（首现场：! LaTeX Error: Missing \begin{document}.） |
| `ctexart` | ERROR | ! Critical Class ctexart Error: CTeX fontset `fandol' is unavailable in |
| `standalone` | PASS | — |

结果为 **19 PASS / 10 ERROR / 1 TIMEOUT**；所有 30 项入口文件均已入包，没有把兼容失败记作资产缺失。
fontspec/unicode-math/xeCJK 的引擎要求、ctex 的字体通道、biblatex 补丁语义及
PGF/minted/beamer 展开问题需要各自独立修复；siunitx 本次仅判定限时未完成，不推断根因。

## 回归与维护

- 既有 WASM 测试 31/31 通过；原 multline、三组 align 和 blindtext 首页回归仍通过。
- 完整门禁暴露 fmt_reload 测试中 TRIP/ETRIP 并发改写 NTEX_TFM_DIR 的既有竞态；
  用测试进程内互斥锁串行隔离字体目录选择，生产路径不变。
- 资产检查加入 make check 的 assets-check 门禁；无需网络、宿主 TeX Live 或生成 WASM。
- 逐文件 SHA-256、实际 bundle 覆盖、暂存树恢复均通过；ntex-pkg check 对本次 TLPDB 无漂移。
- 本次没有刷新 latex.fmt、已有基础宏文件或字体集。

## 复现原料

取料镜像：https://mirrors.aliyun.com/CTAN/systems/texlive/tlnet。
需求与 TLPDB SHA-256 在 assets/tex-minimal/default-packages.json；上游容器 revision/SHA-512
在根 ntex.lock，许可证与 CTAN 位置在 runtime-licenses.tsv。
下载和物化仍走 ntex-pkg（TLPDB 单一解析源）；暂存树中缺少的两个 tlpkg/tlpostcode
原生安装脚本属于本次未发行的范围，不属于 tex 运行文件缺失。
