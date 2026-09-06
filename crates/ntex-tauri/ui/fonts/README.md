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

- 来源：texlive 2024 basic（`texmf-dist/fonts/opentype/public/lm/`）
- 许可：GUST Font License（可再分发，见字体文件内嵌 LICENSE 声明）
- 未注入的字体（如数学 lmmi10/lmsy10/lmex10）渲染时逐字符回落占位方框
  （引擎契约：不报错不 panic）
