# fixtures/extras/ — NTex 补充对照 fixtures

> 与 `fixtures/trip/`、`fixtures/etrip/` 并列；专为 PDF / pdftex 端到端对照服务。
> 抓取脚本：`scripts/fetch-extras-fixtures.sh`。

## 当前内容

| 子目录 | 内容 | 价值 |
|---|---|---|
| `pdftex/expanded.tex` | David Carlisle / Bruno Le Floch（2018, Public Domain）。12 个 `\expanded` 端到端测试：参数构造、torture case、`\ifincsname`、`\the\toks`、嵌套 `\unexpanded` 等边界 | **直接喂入 NTex 可跑**；与 pdftex 反向对照 `\expanded` 原语语义 |
| `pdftex/expanded.txt` | 同作者给出的 36 行期望输出（pdftex `--recorder` 风格 `\show` 输出基线） | 与 NTex 跑出的 log 做逐行对照 |

合计 ~2.5 KB，git 友好。

## `fixtures/recovery/` — 错误恢复语义语料库（2026-09-12 新设）

32 个自包含 case（`\ifcsname` 错误恢复家族，fh9 战役沉淀）：每 case =
`case.tex` + `oracle.expect`（pdfTeX 冻结判据，**freeze 前人工核对**）+ 可选
`note.md`（语义核对记录）。runner：`scripts/abmatrix.py`（freeze/verify/run
三模式），入口 `make recovery-check` / `make oracle-verify`，
判读纪律见 `docs/tooling-trust.md` §2.6 与事故七。

## 为什么不补这些？

- **Knuth `story.tex/sample.tex/testmath.tex/small.tex`**：CTAN 镜像（`mirrors.ctan.org`）被国内反代到 `sustech/hust.edu.cn` 等被墙节点；`knuth/dist/tex/` 下实际仅含 `tex.web/texbook.tex/trip.*/glue.web`，**没有这些"演示用"文件**。若真要补，需直接 inline（来源公开）——不属于"测试库"。
- **tectonic regression**：GitHub 上 `tectonic-typesetting/tectonic/tests/` 仅含 `tests/assets` 目录，无真正的 `.tex` fixture。tectonic 用 `TYPST`/`XAR` 等格式，不是直接回归库。
- **expl3 testsuite**：latex3 仓库**没有 `testsuite/` 顶层目录**，l3build 在每个包里就地保存 `.lvt/.tlg`，结构分散；批量下载不现实。
- **pdftex `.test` 文件**：`cnfline.test`、`etriptest.test`、`wprob.test` 等是 shell 驱动脚本，依赖原生 `pdftex --ini/--etex`；NTex 暂未提供 ini 模式，**直接喂入不可行**。
- **luatex/xetex tests**：TeX-Live 仓库内对应 `tests/` 仅含 `luaimage.tex` 等单文件，颗粒度太粗，不形成对照库。

## 用法约定

```bash
# 端到端跑一次（已 fixture 后）：
cargo run -p ntex-dvi --release -- fixtures/pdftex/expanded.tex

# 与期望基线逐步对照（含 \show 输出）：
diff <(ntotext fixtures/pdftex/expanded.log) fixtures/pdftex/expanded.txt
```

## 增量更新流程

如需追加新库请编辑 `scripts/fetch-extras-fixtures.sh`；每行一个文件 + 一条 fallback URL 链。
新增文件要在本 README 加一行、并在 `AGENTS.md §6` 状态表加一条标注。

## 测试字体缓存（~/.ntex-fonts，环境约定）

`ntex-layout` 的增量排版测试（`typeset/incremental_tests.rs`）经
`ensure_tfm_dir()` 查找 `~/.ntex-fonts/cmr10.tfm`：**缺失时退化为 nullfont
全零度量，多页夹具塌成 1 页，13 个测试的"≥2 页"哨兵断言连环失败**
（2026-09-09 在 Mac 实测复现并定位，非代码回归）。

补齐方式（源 = 仓库自带 wasm 内嵌 CM 度量表）：

```bash
mkdir -p ~/.ntex-fonts && cp crates/ntex-wasm/fonts/*.tfm ~/.ntex-fonts/
```
