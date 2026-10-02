# NTex 最小 TeX 发行闭包

本目录是第二十四刀加入的自包含发行资产，目标是在新机器只安装 NTex、没有
TinyTeX/TeX Live/kpsewhich 的情况下，仍能跑通常见 `\documentclass{article}` /
`\documentclass{book}`（含 `\chapter`，2026-09-25 起）文档。

## 来源

- TeX 树来源：TinyTeX TeX Live 2026，路径为 `~/.TinyTeX/texmf-dist/tex/`。
- 基础闭包来源：`/tmp/ntex-minimal/tex/latex/base/`，由主控按
  `/tmp/clean-{a,c,d,f}.log` 的逐文件 not-found 链推导。
- 字体度量：`assets/tfm/`，**2026-09-18 由 48 件补齐到 643 件（≈1.5MB，含 `ec/`
  子目录）**——原来那 48 件只够 plain 默认字体块，`\documentclass{article}` 拉起的
  `cmbx12`/`cmr17`/`cmti12` 等 LaTeX 字号族全部 not-found。48 件那批同时内嵌在
  `crates/ntex-wasm/fonts/`（wasm 内嵌副本，见 `crates/ntex-wasm/README.md`）。
- 分发 fmt：`assets/fmt/latex.fmt`（6,502,563 字节，2026-09-18 由
  `cargo run -p ntex-dvi -- --generate-fmt` **真生成**）；
  `assets/fmt/ltxinit.tex` 是进阶用户重新 dump 的输入。

> **警告（2026-09-18 踩过）——fmt 只能真生成，不许"改文件头"。**
> `ntex-format` 的 codec 版本号只是文件头里一个整数，改它能骗过读取时的版本检查，
> 但体内的字节布局（InternTable / eqtb 槽 / TokenArray 偏移）已经变了，载入时
> 必然 `failed to fill whole buffer`。仓库里曾有过一个这样"转换"来的 `latex.fmt`
> （由 `/tmp/fp11/latex6-v2.fmt` 改头而来），后果是 Tauri 工作台 LaTeX 通道整体
> 报废、只能回落 plain 子集，表现为"LaTeX 源码被当成正文排版"。引擎语义或
> `ntex-format` codec 变更后，一律按文末「更新方法」重新生成。

## 内容

基础闭包**由 `ntex-pkg vendor` 按依赖闭包全量物化**（不再手抄）。2026-09-25 起闭包
以 TL 包 `latex`（`latex.r79618`，171 个运行面文件）为锚，`book.cls`/`bk10|11|12.clo`
随闭包一并入库；精确文件清单与版本由仓库根 [`ntex.lock`](../../ntex.lock) 钉死，
逐文件名单随时可用下面命令重生成：

```bash
find tex/latex/base -type f | sort
```

常用 article 文档补齐目录：

```text
tex/latex/tools/
tex/latex/graphics/
tex/latex/xcolor/
tex/latex/url/
tex/latex/geometry/
tex/latex/l3kernel/
tex/latex/booktabs/       ← 2026-10-03 新增：三线表（booktabs.sty）
tex/latex/multirow/       ← 2026-10-03 新增：跨行单元格（multirow.sty）
tex/latex/misc/          ← 2026-09-18 新增：lingmacros.sty + tree-dvips.sty
tex/generic/iftex/
tex/generic/infwarerr/
tex/generic/ltxcmds/
tex/generic/pdftexcmds/
tex/generic/atbegshi/
```

`tex/latex/misc/` 的两个宏包服务于语言学示例文档（如仓库 `samples/latex-sample2e-slim.tex` 的
`\enumsentence`/`\shortex`/`\node`），取自 CTAN
`/macros/latex209/contrib/trees/tree-dvips`（阿里云镜像；CTAN 主站与清华镜像
被本机网络策略拦截时改走它）。tree-dvips 是 LaTeX209 风格、依赖 dvips
`\special` 画树线——ntex 引擎把 `\special` 当 whatsit 节点吞掉，故树形连线
不可视、文本结构完好。

当前 TeX 文件体量约 6.8MB（312 个文件，2026-09-25 book 闭包落盘后），TFM 度量 643 件
约 1.5MB（`assets/tfm/` 顶层 78 件 + `ec/` 子目录 565 件）。入包时按**主名去重**
（`tex/latex/base/` 与 `tex/latex/l3kernel/` 有同名副本如 `expl3-code.tex`，wasm 侧
`MemVfs` 按键覆盖），入包精确条数随闭包演进而变，以打包端实测为准。
完整文件清单可用：

```bash
find assets/tex-minimal -type f | sort
```

## 默认搜索链

`ntex-dvi` 默认按以下顺序查找 `\input` 文件：

```text
1. cwd
2. --input-path 显式目录
3. 可执行文件同目录 tex/
4. ~/.ntex/tex/
5. TEXINPUTS
6. 仓库/发行包 assets/tex-minimal/tex/
7. ~/.TinyTeX/texmf-dist/tex/
8. kpsewhich
```

发行包形态推荐把本目录的 `tex/` 复制到 `ntex-dvi` 可执行文件同目录；开发树运行时会
自动探测仓库内的 `assets/tex-minimal/tex/`。

## 更新方法

### 首选：由依赖闭包推导（`ntex-pkg vendor`，2026-09-19 起）

上面那份「常用 article 文档补齐目录」是**手抄**维护的产物。手抄的失效模式不是抄错，
而是**不知道漏了什么**——真实事故：`graphics-def`（`pdftex.def` 驱动）与 `graphics-cfg`
（`graphics.cfg`/`color.cfg`）不在 `tex/latex/graphics/` 下，于是资产"看起来齐全"，
实际 `\usepackage{graphicx}` 拿不到驱动文件。

`ntex-pkg` 把「资产内容」定义成 **种子名单 → 依赖闭包 → 逐文件落地**，缺口**可见**
（源树缺文件报 `SourceMissing`，不静默跳过）：

```bash
# ① 取料：真实镜像 → 一棵可当 TL 树根用的缓存树（每个容器按 TLPDB 的 SHA-512 校验）
ntex-pkg fetch <tlpdb> https://mirrors.aliyun.com/CTAN/systems/texlive/tlnet \
  /tmp/tl-cache --documentclass article

# ② 物化：先看差异（dry-run，只读），确认后加 --write
ntex-pkg vendor <tlpdb> /tmp/tl-cache assets/tex-minimal --documentclass article
ntex-pkg vendor <tlpdb> /tmp/tl-cache assets/tex-minimal --documentclass article \
  --write --lock ntex.lock

# ③ 复跑应报「一致 N / 新增 0」（幂等）；非零即为版本漂移或漏项
```

`<tlpdb>` 用与镜像同代的 `tlpkg/texlive.tlpdb`（`fetch` 会把它一起写进缓存树）。
若本机已装 TeX Live，可跳过 ① 直接把 `~/.TinyTeX` 当 `<TL 树根>` 喂给 `vendor`。
**本机网络备注**：CTAN 主站与清华镜像被策略拦截（403），走阿里云镜像可用。

> **缺口处置（2026-09-25 已补齐）**：2026-09-19 曾登记「以真实 `latex.r79618` 闭包为源
> 做 dry-run 得 一致 48 · 需刷新 7 · 新增 116（1.8MB），是否补齐待产品决策」。2026-09-25
> 因 `\documentclass{book}` 需求正式落盘：`fetch --documentclass book`（同样解析到
> `latex.r79618`）→ `vendor --write --lock ntex.lock`，复跑**一致 171 / 新增 0**（幂等），
> `ntex-pkg check` 对同代 TLPDB 报「锁与当前 TLPDB 一致」。
> 另：`tex/latex/misc/` 的 `lingmacros.sty`/`tree-dvips.sty` 取自 CTAN LaTeX209 目录，
> **不在 TLPDB 闭包内**，`vendor` 无法推导，须手工保留。

### 备用：手工 `cp`（`ntex-pkg` 不可用时的降级路径）

```bash
rm -rf assets/tex-minimal/tex
mkdir -p assets/tex-minimal/tex/latex assets/tex-minimal/tex/generic
cp -R /tmp/ntex-minimal/tex/latex/base assets/tex-minimal/tex/latex/
cp -R ~/.TinyTeX/texmf-dist/tex/latex/{tools,graphics,xcolor,url,geometry,l3kernel} \
  assets/tex-minimal/tex/latex/
cp -R ~/.TinyTeX/texmf-dist/tex/generic/{iftex,infwarerr,ltxcmds,pdftexcmds,atbegshi} \
  assets/tex-minimal/tex/generic/
cp crates/ntex-wasm/fonts/*.tfm assets/tfm/
```

> 注意 `~/.TinyTeX/texmf-dist/…` 这类路径只在**已安装树**上成立；tlnet 库的路径带
> `RELOC/` 前缀，映射规则见 `crates/ntex-pkg/src/tlpdb.rs::install_rel_path`。

引擎语义或 `ntex-format` codec 变更后，必须重新生成 fmt：

```bash
cargo run -p ntex-dvi -- --generate-fmt /tmp/latex.fmt
cp /tmp/latex.fmt assets/fmt/latex.fmt
```
