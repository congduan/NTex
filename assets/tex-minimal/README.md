# NTex 最小 TeX 发行闭包

本目录是第二十四刀加入的自包含发行资产，目标是在新机器只安装 NTex、没有
TinyTeX/TeX Live/kpsewhich 的情况下，仍能跑通常见 `\documentclass{article}` 文档。

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

基础闭包固定为 `tex/latex/base/` 下 51 个文件：

```text
article.cls
expl3-code.tex
expl3.ltx
fontmath.ltx
fonttext.ltx
hyphen.ltx
l3backend-dvips.def
latex.ltx
omlcmm.fd
omlcmr.fd
omllcmm.fd
omscmr.fd
omscmsy.fd
omslcmsy.fd
omxcmex.fd
omxlcmex.fd
ot1cmdh.fd
ot1cmfib.fd
ot1cmfr.fd
ot1cmr.fd
ot1cmss.fd
ot1cmtt.fd
ot1cmvtt.fd
ot1lcmss.fd
ot1lcmtt.fd
preload.ltx
size10.clo
t1cmdh.fd
t1cmfib.fd
t1cmfr.fd
t1cmr.fd
t1cmss.fd
t1cmtt.fd
t1cmvtt.fd
t1lcmss.fd
t1lcmtt.fd
ts1cmr.fd
ts1cmss.fd
ts1cmtt.fd
ts1cmvtt.fd
tulmdh.fd
tulmr.fd
tulmss.fd
tulmssq.fd
tulmtt.fd
tulmvtt.fd
ucmr.fd
ucmss.fd
ucmtt.fd
ulasy.fd
ullasy.fd
```

常用 article 文档补齐目录：

```text
tex/latex/tools/
tex/latex/graphics/
tex/latex/xcolor/
tex/latex/url/
tex/latex/geometry/
tex/latex/l3kernel/
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

当前 TeX 文件体量约 5.1MB（182 个文件），TFM 度量 643 件约 1.5MB（`assets/tfm/`
顶层 78 件 + `ec/` 子目录 565 件）。入包时按**主名去重**（`tex/latex/base/` 与
`tex/latex/l3kernel/` 有同名副本如 `expl3-code.tex`，wasm 侧 `MemVfs` 按键覆盖），
故 C 档资产包实测为 **823 条 ≈11.7 MB**（179 tex + 643 tfm + 1 fmt）。
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

引擎语义或 `ntex-format` codec 变更后，必须重新生成 fmt：

```bash
cargo run -p ntex-dvi -- --generate-fmt /tmp/latex.fmt
cp /tmp/latex.fmt assets/fmt/latex.fmt
```
