# 内嵌 TFM 字体（crates/ntex-wasm/fonts）

| 文件 | 字节 | 用途 |
|---|---|---|
| `cmr10.tfm` | 1 296 | 罗马正文（demo.tex 主字体） |
| `cmbx10.tfm` | 1 328 | 粗体（demo.tex `\bf`） |
| `cmti10.tfm` | 1 480 | 斜体（demo.tex `\it`） |
| `cmmi10.tfm` | 1 528 | 数学文本斜体（M4 数学，B 档 demo 预留） |
| `cmsy10.tfm` | 1 124 | 数学符号（同上） |
| `cmex10.tfm` | 992 | 数学大符号（同上） |

合计 7 748 字节，经 `include_bytes!` 进 wasm 模块（A 档内嵌 TFM 的全部体积）。

## 来源

TinyTeX `texmf-dist/fonts/tfm/public/cm/`（本 VM 上 `kpsewhich cmr10.tfm` 所指
路径），与 native 路径 `ntex_font::find_tfm` 命中的是同一份字节——因此 wasm 侧
排版结果与 native 侧共享同一度量来源，不存在"两套字体数据"的偏差风险。
MD5（复制时点）：

```
c834bbb027764024c09d3d2bf908b5f0  cmbx10.tfm
662f679a0b3d2d53c1b94050fdaa3f50  cmex10.tfm
abec98dbc43e172678c11b3b9031252a  cmmi10.tfm
45809c5a464d5f32c8f98ba97c1bb47f  cmr10.tfm
6c73e740cf17375f03eec0ee63599741  cmsy10.tfm
aa8e34af0eb6a2941b776984cf1dfdc4  cmti10.tfm
```

## 许可

Computer Modern 字体（含 TFM 度量）属 American Mathematical Society 的
Computer Modern 字体发布，按其许可可自由再分发（TeX Live `texmf-dist` 以
Knuth License / LPPL 兼容条款随行发布）。NTex 仓库内随源分发这 6 个度量文件，
未改动任何字节。

## 换字体 / 加字体

A 档只内嵌这 6 个。要扩字体：改 `src/lib.rs` 的 `EMBEDDED_TFMS` 表（再加一行
`include_bytes!`），或等 B 档把 `ntex_layout::set_tfm_source` 的注册权开放给 JS
（宿主经 `fetch()` 取 TFM 字节注入）——缝已就位，见
`crates/ntex-layout/src/typeset/wasm_fonts.rs`。
