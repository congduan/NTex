# 内嵌 TFM 字体（crates/ntex-wasm/fonts）

**48 个 Computer Modern 度量**，经 `include_bytes!` 进 wasm 模块（合计约 196 KB）。

## 为什么是「plain 预载全集」而不是按需挑几个

wasm 管线走 `set_preload_plain(true)`（等价源首行 `\input plain`），内嵌
`plain.tex` 的字体段会把整套 `\font\preloaded=<name>` 装进 eqtb。**缺任何一个
都会吐一行**：

```
! Font cmr9 not loadable: Metric (TFM) file not found.
```

A 档最初只嵌了 14 个 10pt/7pt/5pt 常用件，于是任何 plain 作业在 Tauri 里
都带 34 行缺字体噪声（2026-09-11 修，现场是 `resume-plain.tex`）。
全集增量仅 ~130 KB，换 log 与排版语义干净。

## 清单来源（不要手抄）

清单是**从内嵌格式源机器提取**的，不是人工列的：

```bash
grep -oE '=[a-z]{2,}[a-z]*[0-9]+' crates/ntex-layout/resources/plain.tex \
  | sed 's/^=//' | sort -u
```

`crates/ntex-wasm/src/lib.rs` 的 `EMBEDDED_TFMS` 与之逐名对应，并由回归测试
`embedded_tfms_cover_plain_preload_set` 钉住（改 plain.tex 字体段后测试会先红）。

## 分组清单

| 族 | 文件 |
|---|---|
| 罗马正文 `cmr` | `cmr5` `cmr6` `cmr7` `cmr8` `cmr9` `cmr10` `cmr12` |
| 粗体 `cmbx` | `cmbx5` `cmbx6` `cmbx7` `cmbx8` `cmbx9` `cmbx10` |
| 打字机 `cmtt` | `cmtt8` `cmtt9` `cmtt10` |
| 意大利 `cmti` | `cmti7` `cmti8` `cmti9` `cmti10` |
| 数学斜体 `cmmi`（OML） | `cmmi5` `cmmi6` `cmmi7` `cmmi8` `cmmi9` `cmmi10` `cmmib10` |
| 数学符号 `cmsy`（OMS） | `cmsy5` `cmsy6` `cmsy7` `cmsy8` `cmsy9` `cmsy10` `cmbsy10` |
| 大算符 `cmex`（OMX） | `cmex10` |
| 无衬线 `cmss` | `cmss10` `cmssbx10` `cmssi10` `cmssq8` `cmssqi8` |
| 细体 `cmsl` | `cmsl8` `cmsl9` `cmsl10` `cmsltt10` |
| 其他 | `cmcsc10`（小型大写）`cmdunh10`（装饰）`cmu10`（数学 U）`manfnt`（plain 提示字形） |

`cmr12` 不在 plain 预载清单里，但它服务 `\font\sectfont=cmr12 scaled \magstep1`
这类标题写法（`scaled` 只改尺寸、不改度量来源），因此一并内嵌。

## 来源

TeX Live 2024 basic（本机 `/usr/local/texlive/2024basic/texmf-dist/fonts/tfm/public/`）：

- `cm/` —— 上表中除 `manfnt` 外的全部；
- `knuth-lib/manfnt.tfm` —— `manfnt`。

与 native 路径 `ntex_font::find_tfm` 命中的是同一份字节（`kpsewhich cmr10.tfm`
所指路径），因此 wasm 侧排版结果与 native 侧共享同一度量来源，不存在"两套
字体数据"的偏差风险。复制时未改动任何字节。

校验（任一文件）：

```bash
md5 /usr/local/texlive/2024basic/texmf-dist/fonts/tfm/public/cm/cmr9.tfm \
    crates/ntex-wasm/fonts/cmr9.tfm    # 两者应一致
```

## 许可

Computer Modern 字体（含 TFM 度量）属 American Mathematical Society 的
Computer Modern 字体发布，按其许可可自由再分发（TeX Live `texmf-dist` 以
Knuth License / LPPL 兼容条款随行发布）。

## 换字体 / 加字体

两条路：

1. **改内嵌表**：从 TeX Live 复制 `.tfm` 到本目录，在 `src/lib.rs` 的
   `EMBEDDED_TFMS` 加一行 `include_bytes!`（记得同步本文件的清单与
   `PLAIN_PRELOAD_FONTS` 回归表）；
2. **运行时注入**（推荐给非 CM 字体）：
   - 有 TFM 的字体 —— 走 `TfmSource` 的 `tfm_bytes`（wasm 侧已由
     `EMBEDDED_TFMS` 实现，可扩展为 fetch 注入）；
   - **无 TFM 的 OpenType 字体（中文等）** —— 宿主 fetch 后调
     `set_otf_font(name, bytes)`，一次注册「排版度量 + 渲染轮廓」两侧
     （见 `crates/ntex-tauri/ui/fonts/README.md` 的中文一档）。
