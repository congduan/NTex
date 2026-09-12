# PDF 导出字体（Type1 / PFB）

**47 个 Computer Modern 字体程序**，供 `Document::pdf_bytes` 导出 PDF 时嵌入
（合计约 1.5 MB）。前端 fetch 后经 wasm 导出 `set_pfb_font(tex_name, bytes)`
注入进程级注册表，`ntex-pdf` 把它们原样写进 PDF 的 `/FontFile` 流。

## 为什么这里还有一份字体（`ui/fonts/` 不是已经有 OTF 了吗）

两条**互不相通**的通道，别混：

| 目录 | 格式 | 谁在用 | 解决什么 |
|---|---|---|---|
| `ui/fonts/` | OTF/TTF | `set_glyph_font` / `set_otf_font` | **屏幕上**的字形轮廓（skrifa 提轮廓 → 光栅化） |
| `ui/pfb/`（本目录） | Type1 PFB | `set_pfb_font` | **PDF 里**的字体程序（原样作 `/FontFile` 流） |

PDF 的 `/FontFile` 只接受 Type1 程序——把 OTF 塞进去查看器渲染不出字形
（要嵌 OpenType 得走 CID/OpenType 那条路，是另一个量级的工作，属另案）。
所以屏幕端用 Latin Modern OTF 渲染、导出端仍需另配一份 CM PFB，两者字形
同源（LM 与 CM 度量一致），页面上不会出现「屏上一套、导出另一套」的观感差。

## 为什么是「plain 预载全集」而不是按需挑几个

与 `crates/ntex-wasm/fonts/`（48 件内嵌 TFM）同集。前端按
`Document::used_fonts()`（**DVI 实际引用**的字体表）逐名 fetch，plain 作业
通常只用得到 1–4 件；但把全集一次备齐，任何 plain 写法都不会在导出时才发现
缺字体。

> ⚠ 别用 `Document::fonts()` 去决定 fetch 哪些——那是**引擎侧已载入全表**
> （plain 预载 48 件全在里面，哪怕正文一个字符都没用到），会白拉几十份资源
> 并对用不到的字误报「缺字体」。这两个口径由回归测试
> `document_used_fonts_is_dvi_table_and_pdf_export_works` 钉住。

## 分组清单（47 件）

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
| 其他 | `cmcsc10`（小型大写）`cmdunh10`（装饰）`cmu10`（数学 U） |

**48 缺 1 = `manfnt`**：它只有 Metafont/TFM 发行，没有 Type1 版本。正文用不到
它；万一源里真的 `\font\a=manfnt` 并排版了字符，导出会按 `ntex-pdf` 既有口径
降级（`/BaseFont` 保留、不写 `/FontFile`），前端会给出可见提示而不是静默产出
一份打开后页页空白的 PDF。

## 来源与许可

- **来源**：TeX Live（`kpsewhich cmr10.pfb` 定位；本机 `2024basic` 的
  `texmf-dist/fonts/type1/public/amsfonts/cm/`）。字节**原样复制**，不改造。
- **许可**：AMS Computer Modern 字体发布，可自由再分发（与
  `crates/ntex-wasm/fonts/` 的 TFM 同一许可来源）。
- 与 native 路径 `ntex_pdf::type1::find_pfb` 命中的是同一份字节，故
  wasm 端导出的 PDF 与命令行 `cargo run -p ntex-pdf -- x.dvi` 的嵌入结果一致。

## 重新抓取 / 校验

```bash
bash scripts/fetch-cm-pfb.sh     # 幂等：重抓并覆盖，附段头校验
```

校验单件（应与 TeX Live 一致）：

```bash
md5 "$(kpsewhich cmr10.pfb)" crates/ntex-tauri/ui/pfb/cmr10.pfb
```

## 已知缺口

- **中文（Fandol Song 等 OTF）当前没有 PDF 输出路径**：`ntex-pdf` 只懂 Type1
  （`/FontFile` 原样嵌 PFB）。中文导 PDF 需要 `ntex-pdf` 支持 CID/OpenType
  嵌入，属另案；在此之前中文文档只能走 DVI/位图路径。
- PFB 是**矢量**字体程序，导出的 PDF 文字可选中、可搜索、可缩放——与
  「位图 PDF」方案相比体积小得多（一页纯文字约几十 KB）。
