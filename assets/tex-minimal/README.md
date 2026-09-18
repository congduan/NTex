# NTex 最小 TeX 发行闭包

本目录是第二十四刀加入的自包含发行资产，目标是在新机器只安装 NTex、没有
TinyTeX/TeX Live/kpsewhich 的情况下，仍能跑通常见 `\documentclass{article}` 文档。

## 来源

- TeX 树来源：TinyTeX TeX Live 2026，路径为 `~/.TinyTeX/texmf-dist/tex/`。
- 基础闭包来源：`/tmp/ntex-minimal/tex/latex/base/`，由主控按
  `/tmp/clean-{a,c,d,f}.log` 的逐文件 not-found 链推导。
- 字体度量：`assets/tfm/`，复用 `crates/ntex-wasm/fonts/` 的 48 个 CM TFM
  （约 200KB），覆盖 plain/LaTeX 默认 Computer Modern 字体块。
- 分发 fmt：`assets/fmt/latex.fmt`，由 `/tmp/fp11/latex6-v2.fmt` 转换为当前
  `ntex-format` v17 文件头；`assets/fmt/ltxinit.tex` 是进阶用户重新 dump 的输入。

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
tex/generic/iftex/
tex/generic/infwarerr/
tex/generic/ltxcmds/
tex/generic/pdftexcmds/
tex/generic/atbegshi/
```

当前 TeX 文件体量约 5.3MB，TFM 度量约 200KB。完整文件清单可用：

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
