# NTex Studio 前端字体（真字形通道）

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
