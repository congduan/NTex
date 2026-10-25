# `\pdfmdfivesum` 原语 + pdftexcmds/minted 探测链闭环（2026-10-25）

## 背景与靶形

8e506b2 WASM 资产审计 10 ERROR 中的 minted 首错链：minted → pdftexcmds →
l3kernel `\file_mdfive_hash:n` 探测 `\pdfmdfivesum`。缺注册时
`\tex_mdfivesum:D` Undefined 族报错（简报预估 30 错），pdftexcmds 走
`\pdftexcmds@nopdftex` 退化臂。

本刀实为 **cron 前一轮 Codex 遗留 WIP 的主控接管收口**（工作树完整实现未提交，
含 MD5 RFC 1321 实现 + handler 双臂 + 注册四件 + GT 向量测试，工作树时间线：
10-21 10:21 写码 → 16:20 构建 + 跑宏包回归 0 FAIL + 刷新基线元数据）。

## 实现（四处接线，对照 `\pdfstrcmp` 同族形态）

1. `eqtb/primitive.rs`：`EXPANDABLE` 清单 + 枚举**尾部**追加 `PdfMdfiveSum`
   （编号源是第二个清单，EXPANDABLE 只喂 `is_expandable!` 宏匹配——
   `.fmt` 序号安全，主控核过宏定义；发行 latex.fmt 已重生成入库
   9097042→9097034B，含第 457 个内建名）。
2. `expand/builtins.rs`：BUILTINS 456→457（一致性测试覆盖完整性）。
3. `expand/primitive_pdf_mdfive.rs`（新文件）：RFC 1321 Rust 直译，
   零外部依赖，7 条官方向量 + 1000 字节跨块（python hashlib 对拍）。
4. `expand/primitive_expand.rs`（dispatch_expandable 臂）+
   `expand/expr.rs`（`\numexpr` 系展开位臂）——双臂同源
   （free.rs PdfMatch 注释记载的「缺一臂空转」教训遵守）。
5. `expand/free.rs`：`is_expandable_prim` 白名单（`\edef` 缓存键 +
   l3kernel 直接展开语境）。

GT 语义（pdflatex 健康通道实测）：`\pdfmdfivesum{<text>}` 实参先全展开
再取字节 → 32 字符**大写** hex；`file{name}` 花括号形取文件字节摘要，
缺文件 → 空展开（l3kernel 以空判定「未找到」）。
**⚠ 语法陷阱：真 pdfTeX 的 file 形是 `file{name}` 花括号形**（pdftexcmds.sty
`\def\pdf@filemdfivesum#{\pdfmdfivesum file}%` 实证）；无花括号裸名 GT 也报
`Missing {`（主控实测，勿当 NTex 缺陷）。

## 验证（主控独立复测，原始输出）

| 项 | 结果 |
|---|---|
| RFC 1321 向量 + CLI text/file 形 | `abc`→900150983CD24FB0D6963F7D28E17F72、`Hello World`→B10A8DB164E0754105B7A99BE72E3FE5、file→5D41402ABC4B2A76B9719D911017C592（与 GT/md5sum 逐字一致） |
| 端到端 | pdftexcmds 0 错载入，探测 native 后走原生分支，`\pdf@mdfivesum`/`\pdf@filemdfivesum` 双形值对 |
| 单测 | core **593** 绿（+2 mdfive）+ layout 336 绿；clippy `-D warnings` 0 |
| 宏包回归 | **13/13**（主控复跑，非只采信 WIP 轮自证） |
| TRIP 防回退 | trip 177B/1 页、tripos **750B/12 页**（`--no-plain --dump trip2.fmt` 姿势，biblatex 战报口径） |
| small2e | 0 错 1 页 |
| `cargo fmt --check` | 0 |

## Review

Claude 定向 review（glm-5.3-flash，本轮无 124 超时）：结论**可继续**，
5 项定向核验全过（双臂同源/escapechar misc[34] 同源/MSRV/`scan_general_text`
语义/测试覆盖）。唯一建议项 = file 形 VFS 单测（对照 `\pdffilesize` 既有标准），
主控已补 `pdfmdfivesum_file_form_reads_vfs_and_empty_for_missing`（GT 实测值
md5("0123456789")=781E5E245D69B566979B86E28D23F2C7，修前红/修后绿同轮验证）。

## 已知边界（不阻塞）

- minted 全量加载闭环未测（本刀目标 = pdftexcmds/l3kernel 探测链；
  minted 还需 shell-escape/fancyvrb 等后续链路）。
- 引擎 `scan_file_name` 接受裸名（真 pdfTeX 只认 `file{name}`）——容差非缺陷
  （review 备忘）。
- expr.rs 臂无独立单测（`\pdfstrcmp` 同现状，`\edef` 测试已间接穿过）。

## 教训

- **首轮 file 形 CLI 探针「空输出」是探针姿势**（无花括号裸名 GT 同病），
  对照 GT 后定性，未误开修复刀。
- l3 探针文档忘 `\makeatletter` 会把 `\pdf@mdfivesum` 切碎成字面排版
  （`@` cat-12），0 错但 MD5=[@mdfivesum{abc}]——**0 错 ≠ 链路通**，
  端到端验收必须看值不看错数。
