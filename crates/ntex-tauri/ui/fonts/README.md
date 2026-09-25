# NTex Studio 前端字体（真字形通道）

分两类：**拉丁/数学族**（给内嵌 TFM 字体补轮廓，`set_glyph_font`）与
**中文族**（OpenType 原生字体，无 TFM，`set_otf_font` 同时打通度量与轮廓）。

## 一、拉丁 / 数学（Latin Modern）

Latin Modern OpenType（与 Computer Modern 同源，度量一致；TeX 字体名 → 文件的
映射见 `crates/ntex-backend/src/glyphs.rs::lm_file_name`）。前端 fetch 后经
wasm 导出 `set_glyph_font(tex_name, bytes)` 注入进程级注册表（wasm 无文件
系统），`Document.set_glyphs(true)` 后渲染走真字形轮廓。

**覆盖面口径**（2026-09-25 定）：本目录必须覆盖 LaTeX 实际会点名的**全部** CM
字体名——`tex/latex/base/{ot1cm*,om*}.fd` 声明的尺寸 × 字形矩阵，加上 EC「TC」
TS1 的 5 个随包族。名字清单在 `../main.js::GLYPH_FONTS`（唯一手写处），
映射与命名规则在 `lm_file_name`；两侧一致性 + 文档级覆盖面由
`glyphs.rs::ui_font_manifest_matches_rust_claim` 与
`ntex-wasm` 的 `workbench_manifest_covers_fonts_used_by_latex_docs` 钉住。
**漏一个名字 = 该字体在预览里整段灰方框**（现场两条：`\LaTeX` 徽标的 A 走
`cmr8`、`$E=mc^2$` 走 `cmmi12`+`cmr8`）。

| 组 | 文件（LM 命名） | 对应 TeX 名 |
|---|---|---|
| 罗马 | `lmroman{5,6,7,8,9,10,12,17}-regular` | cmr5…cmr17 |
| 罗马粗 | `lmroman{5,6,7,8,9,10,12}-bold` | cmbx5…cmbx12 / cmb10 |
| 罗马意大利 | `lmroman{7,8,9,10,12}-italic` | cmti7…cmti12 |
| 斜体 slanted | `lmromanslant{8,9,10,12,17}-regular`、`lmromanslant10-bold` | cmsl8…cmsl12、cmbxsl10 |
| 粗意大利 | `lmroman10-bolditalic` | cmbxti10 |
| 小体大写 | `lmromancaps10-regular` | cmcsc10 |
| 直立体 | `lmromanunsl10-regular` | cmu10（`\pounds` 用） |
| 无衬线 | `lmsans{8,9,10,12,17}-regular`、`lmsans10-bold`、`lmsans{8,9,10,12,17}-oblique`、`lmsansdemicond10-regular` | cmss*、cmssbx10、cmssi*、cmssdc10 |
| 打字机 | `lmmono{8,9,10,12}-regular`、`lmmono10-italic`、`lmmonoslant10-regular`、`lmmonocaps10-regular` | cmtt*、cmitt10、cmsltt10、cmtcsc10 |
| 变宽打字机 | （LM 无对应）按 `lmmono{8,9,10,12}-regular`/`lmmono10-italic` 近似 | cmvtt10、cmvtti10 |
| 数学 | `latinmodern-math.otf`（单文件） | cmmi*、cmsy*、cmex*、cmmib10、cmbsy10、icmmi8、icmsy8、icmex10 |
| EC TS1 | 按上表同族取名（`tcrm1000`→`lmroman10-regular` 等） | tcrm/tcti/tcbx/tcss/tctt × 全档 |

- 来源：texlive 2024 basic（`texmf-dist/fonts/opentype/public/lm/` 与
  `lm-math/`；数学族 LM 无独立 OTF，统一走 lm-math 包的 OpenType MATH 单文件）
- 许可：GUST Font License（可再分发，见字体文件内嵌 LICENSE 声明）
- slot→Unicode 按字体编码分发（OT1 / OML·OMS·OMX / EC-TS1，见
  `crates/ntex-backend/src/glyphs.rs`）；未映射的字符渲染时逐字符回落
  占位方框（引擎契约：不报错不 panic）

## 二、中文（Fandol Song）

| 文件 | TeX 字体名 | 用途 |
|---|---|---|
| `FandolSong-Regular.otf` | `FandolSong-Regular` | 中文正文/标题（衬线宋体） |

- **来源**：CTAN `fonts/fandol`（GPL），**经 `scripts/make-cjk-subset.py`
  子集化**——只保 ASCII + GB2312 符号区 + 一级常用汉字 3755 字，4.9 MB → 2.0 MB。
  重做/换档位：`python3 scripts/make-cjk-subset.py <源.otf> -t l1`。
- **为什么用 `set_otf_font` 而不是 `set_glyph_font`**：Fandol 没有 TFM，
  排版阶段就需要字体字节建度量（`TfmSource::otf_bytes`）。
  `set_glyph_font` 只写渲染侧注册表，中文会在 `\font\zh=FandolSong-Regular`
  这一步就报 `not loadable`。`set_otf_font` 一次注册两侧（2026-09-11 之前
  wasm 壳缺这条通路，Tauri 完全排不了中文）。
- **配套输入开关**：中文源文件直写需要 `\utfinputmode=1`
  （UTF-8 多字节 → 单个 21-bit 字符 token）。Tauri 前端在编译前按 UI 的
  「UTF-8」开关自动前置该赋值，源里显式写过则以源的为准。
- 已知简化：只入库 Regular，**中文暂不加粗**（`\bf` 下的汉字仍取常规字形轮廓）；
  无 CJK 断行规则（汉字间无断点，长段需空行/手工分段，见
  `docs/KNOWN-SIMPLIFICATIONS.md`）。

