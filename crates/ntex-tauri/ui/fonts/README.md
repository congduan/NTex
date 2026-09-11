# NTex Studio 前端字体（真字形通道）

分两类：**拉丁/数学族**（给内嵌 TFM 字体补轮廓，`set_glyph_font`）与
**中文族**（OpenType 原生字体，无 TFM，`set_otf_font` 同时打通度量与轮廓）。

## 一、拉丁 / 数学（Latin Modern）

Latin Modern OpenType（与 Computer Modern 同源，度量一致；TeX 字体名 → 文件的
映射见 `crates/ntex-backend/src/glyphs.rs::lm_file_name`）。前端 fetch 后经
wasm 导出 `set_glyph_font(tex_name, bytes)` 注入进程级注册表（wasm 无文件
系统），`Document.set_glyphs(true)` 后渲染走真字形轮廓。

| 文件 | TeX 字体名 | 用途 |
|---|---|---|
| `lmroman10-regular.otf` | cmr10 | 正文罗马 |
| `lmroman10-bold.otf` | cmbx10 | 粗体 |
| `lmroman10-italic.otf` | cmti10 | 意大利体 |
| `lmmono10-regular.otf` | cmtt10 | 打字机体 |
| `lmroman12-regular.otf` | cmr12 | 标题字号 |
| `lmroman7-regular.otf` | cmr7 | 脚本层罗马 |
| `lmroman5-regular.otf` | cmr5 | 二阶脚本罗马 |
| `latinmodern-math.otf` | cmmi10/7/5、cmsy10/7/5、cmex10 | 数学族（斜体字母/符号/大算符） |

- 来源：texlive 2024 basic（`texmf-dist/fonts/opentype/public/lm/` 与
  `lm-math/`；数学族 LM 无独立 OTF，统一走 lm-math 包的 OpenType MATH 单文件）
- 许可：GUST Font License（可再分发，见字体文件内嵌 LICENSE 声明）
- slot→Unicode 按字体编码分发（OT1/OML/OMS/OMX，见
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

