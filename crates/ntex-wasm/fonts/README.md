# 内嵌 TFM 字体（crates/ntex-wasm/fonts）

| 文件 | 字节 | 用途 |
|---|---|---|
| `cmr10.tfm` | 1 296 | 罗马正文（demo.tex 主字体） |
| `cmbx10.tfm` | 1 328 | 粗体（demo.tex `\bf`） |
| `cmti10.tfm` | 1 480 | 斜体（demo.tex `\it`） |
| `cmmi10.tfm` | 1 528 | 数学文本斜体（M4 数学，B 档 demo 预留） |
| `cmsy10.tfm` | 1 124 | 数学符号（同上） |
| `cmex10.tfm` | 992 | 数学大符号（同上） |
| `cmr12.tfm` | 1 280 | 标题字号（demo1 `scaled 1728` 的度量来源） |
| `cmr7.tfm` | 1 148 | 脚本层罗马（`\scriptfont0`） |
| `cmr5.tfm` | 1 152 | 二阶脚本罗马（`\scriptscriptfont0`） |
| `cmmi7.tfm` | 1 424 | 脚本层数学斜体（`\scriptfont1`） |
| `cmmi5.tfm` | 1 408 | 二阶脚本数学斜体（`\scriptscriptfont1`） |
| `cmsy7.tfm` | 1 056 | 脚本层数学符号（`\scriptfont2`） |
| `cmsy5.tfm` | 1 004 | 二阶脚本数学符号（`\scriptscriptfont2`） |

合计 7 748 + 8 476 字节，经 `include_bytes!` 进 wasm 模块（A 档内嵌 TFM 的全部
体积）。

## 来源

TinyTeX `texmf-dist/fonts/tfm/public/cm/`（本 VM 上 `kpsewhich cmr10.tfm` 所指
路径），与 native 路径 `ntex_font::find_tfm` 命中的是同一份字节——因此 wasm 侧
排版结果与 native 侧共享同一度量来源，不存在"两套字体数据"的偏差风险。
MD5（复制时点）：

```
c834bbb027764024c09d3d2bf908b5f0  cmbx10.tfm
662f679a0b3d2d53c1b94050fdaa3f50  cmex10.tfm
abec98dbc43e172678c11b3b9031252a  cmmi10.tfm
3b32edd0d68f6498a5a375e78f9edc5e  cmmi5.tfm
e2423ae06dc7dee599cceb79d1c9dc32  cmmi7.tfm
45809c5a464d5f32c8f98ba97c1bb47f  cmr10.tfm
655e228510b4c2a1abe905c368440826  cmr12.tfm
ad296dff3c8796c18053ab7b9f86ad7c  cmr5.tfm
53d07721103816e093902637bc167021  cmr7.tfm
6c73e740cf17375f03eec0ee63599741  cmsy10.tfm
14d5d5f6bd3c949edecb5b872f295553  cmsy5.tfm
2b3f9b25605010c69bc328bea6ac000f  cmsy7.tfm
aa8e34af0eb6a2941b776984cf1dfdc4  cmti10.tfm
```

## 许可

Computer Modern 字体（含 TFM 度量）属 American Mathematical Society 的
Computer Modern 字体发布，按其许可可自由再分发（TeX Live `texmf-dist` 以
Knuth License / LPPL 兼容条款随行发布）。NTex 仓库内随源分发这 13 个度量文件，
未改动任何字节。

## 换字体 / 加字体

A 档只内嵌 6 个（现已扩到 13 个）。要扩字体：改 `src/lib.rs` 的 `EMBEDDED_TFMS` 表（再加一行
`include_bytes!`），或等 B 档把 `ntex_layout::set_tfm_source` 的注册权开放给 JS
（宿主经 `fetch()` 取 TFM 字节注入）——缝已就位，见
`crates/ntex-layout/src/typeset/wasm_fonts.rs`。
