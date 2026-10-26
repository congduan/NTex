# standalone.cls 闭环 + `\pdffilemoddate` 原语（2026-10-26）

> 战报索引：plan.md §0.1 近期进展第 16 条。本文件是逐验收记录。

## 背景

`scripts/asset-load-audit.sh`（本轮入库，CLI 通道资产审计矩阵）30 项里 standalone
此前是 ERROR 之一（8e506b2 的 WASM 审计 10 ERROR 家族）。本轮 WIP（前一轮 cron
遗留，接手时工作区未提交）以 **`\pdffilemoddate`** 原语为 standalone.cls 闭环靶：

- standalone.cls L909 重编译比对链 `\edef\filemodbefore{...\pdffilemoddate...}`；
- filemod.sty 的在场性探测 `\ifx\pdffilemoddate\@undefined`（缺注册即 PackageError）
  与 `\expandafter\filemod@parse\pdffilemoddate{#2}` 定界拆参（要求可展开）。

## 实现（对齐 pdfTeX 语义，`\pdfstrcmp`/`\pdfmdfivesum` 同族先例）

1. `eqtb/primitive.rs`：`PdfFileModDate` 变体**追加尾部**（既有 .fmt 原语编号不重排）；
2. `expand/builtins.rs`：BUILTINS 457→458 注册 `pdffilemoddate`；
3. 三展开位全接线（「可展开 → 展开后重试」空转教训，双分支缺一即空转）：
   - `primitive_expand.rs` dispatch_expandable 臂；
   - `expr.rs` numexpr/explorer 臂；
   - `save.rs` `\the` 臂（PdfCreationDate 同款先例，防 Missing number）；
4. `free.rs`：`is_expandable_prim` 白名单 + `pdf_file_moddate_tokens`——
   Howard Hinnant civil-from-days（与 `system_date_time` 同族算法，零依赖）；
5. `ntex-io`：`Vfs` trait 加 `file_mtime(&mut self, path) -> Option<u64>`（默认 None）；
   `LocalVfs` 实现（mtime → epoch 秒；NotFound → None 不报错，GT 同语义）；
6. **CLI VFS 链三层转发（本轮接管时补的关键缺口，见「接管补刀」）**：
   `SearchPathVfs` 按读路径同序解析取 mtime；`KpsewhichVfs` 落空后走同一
   kpsewhich 定位（`locate()` helper，read/mtime 共用）；
7. `assets/fmt/latex.fmt` 已重生成入库（第 458 个 builtin 名；fmt-only 类测试 0 红）。

## ⚠ 接管补刀：首轮 CLI e2e 假绿

单元测试（MtimeVfs 固定锚点）全绿，但首轮 CLI 探针 `MODDATE=[]` 恒空——
**根因 = CLI 的 VFS 是 `KpsewhichVfs(SearchPathVfs(LocalVfs))`，两层包装都没转发
`file_mtime`**（trait 默认实现返回 None）。standalone 0 错曾因此是「空 vs 空
相等」的假绿。补转发后 CLI 返回真值。教训登记：**trait 加默认实现的新方法，
所有包装器必须显式转发并在 CLI 通道端到端验证**——单测通道（直接用
LocalVfs/MemVfs）照不出包装层断链。

## 已登记偏差（docs/KNOWN-SIMPLIFICATIONS.md 2026-10-26 条）

- **时区形态**：真 pdfTeX 输出本地时区偏移（GT `D:20261026140627+08'00'` 实测）；
  NTex 输出 UTC + `Z`（`D:…Z'00'`，VFS 只供 epoch 秒）。无害性：filemod.sty 按位
  拆数字段、standalone.cls 只做前后 `\ifx` 相等比对——偏移段不参与比较语义。
- **WASM 确定性**：`MemVfs` 无时间概念 → `file_mtime` 默认 None → 恒空展开
  （确定性换真实时间）。

## 验收（全部原始输出实测，二进制 = 当前 HEAD 全量重建）

| 验收项 | 结果 |
|---|---|
| 单测 `pdffilemoddate_epoch_anchor_and_missing_file_empty` | 绿（epoch 0/闰年/2026 三锚点 + edef 语境 + 在场性探测） |
| io 回归锁 `file_mtime_forwards_through_search_path_and_kpsewhich_chain` | 绿（三层链逐层转发断言） |
| CLI 探针 | `MODDATE=[D:20261026100551Z'00']`（GT `D:20261026180551+08'00'`，同刻不同区）/ `MISSING=[]`（GT 同空） |
| **standalone 最小文档** | **0 错 1 页**；DVI→PDF 与 GT pdflatex **逐像素 md5 相同**（fc6a87fa…，54.38×8.86pt 版面一致） |
| core 595 / layout 336 / io 8 | 全绿（core +1 新测试） |
| clippy -D warnings / cargo fmt --check | 0 / 0 |
| TRIP 防回退 | trip 177B/1 页；tripos **750B/12 页/0 错，MD5 `92100387…` 与基线逐字节相同** |
| small2e（自动检测 + `--fmt` 快路径双通道） | 双 0 错 1 页（fmt 冒烟同过） |
| 宏包回归 | **13/13 PASS** |
| 资产审计（CLI 通道 30 项） | **27 PASS / 3 ENGINE-GATE（xeCJK/fontspec/unicode-math，XeTeX 系 by-design）/ 0 ERROR / 0 TIMEOUT** |

## 资产审计矩阵现状（对照 8e506b2 WASM 基线 19 PASS/10 ERROR/1 TIMEOUT）

CLI 通道 30 项全数「可判定」：PASS 27 含此前 ERROR 家族的 minted/tikz/pgfplots/
beamer/standalone/biblatex——近四刀（pdfmdfivesum/pdfliteral 族/pdfpageattr/
本刀）连带清偿了大部分 ERROR。ENGINE-GATE 3 项（xeCJK/fontspec/unicode-math）
为 XeTeX 能力边界，pdfTeX 语义下 by-design 拒绝，与真 pdfLaTeX 同构，可归并销账。
WASM 通道尚未复测（wasm32 构建不在本刀门禁），不入正典。

## 委派与 review

本轮由主控接管前一轮遗留 WIP 直接收口（评估完成度高：全链路已接线、
测试锁已写、fmt 已重生成；唯一实质缺口 = 包装器断链 + KNOWN-SIMPLIFICATIONS
未登记）。Claude Code review 通道按 cron 策略应在本刀提交前做定向 review；
若通道持续 124 超时，按刀 AJ R11 先例主控亲自 review（diff 全量过目 +
全量验证亲跑）并在此登记缺席事实。

## 收口补记（10-27，主控终验 + review 后提交）

- **⚠ 发行 fmt 中间态事故（本刀接手时实测抓到）**：上轮入库的
  `assets/fmt/latex.fmt`（mtime 16:17）**早于 primitive.rs（18:13）/builtins.rs
  （20:11）的最终编辑**——缺第 458 个 builtin 名，接手时新二进制解析报
  「未知原语编号」，全部 fmt 通道作业不可用（single point of failure）。
  本轮用当前 HEAD 二进制重新生成入库（f03bbfdb，9097060B）。教训固化：
  **fmt 重生成必须是 WIP 收口的最后一步**——任何后续源码编辑（哪怕只动
  注释外的东西）都使入库 fmt 变陈旧，收口时须核对 fmt mtime 晚于全部
  枚举/builtins 源文件 mtime（或干脆收口序列里固定「重新生成 → 验证」）。
- **review**：Claude Code 定向 review 完成（非超时），结论**可继续提交**，
  3 low + 3 info 均非阻塞。已采纳：KNOWN-SIMPLIFICATIONS 措辞两处修正
  （输出实为 `D:…Z` 无 `'00'` 尾段；展开位实为 dispatch_expandable +
  expand_once 双位，`\the` 有意不加）；finding ②（同族日期串形态不一致）
  与 finding ③（陈旧 fmt 补种守卫把显式 0 误当陈旧）登记为技术债。
- **复测全链（HEAD 全量重建二进制）**：core 597 / layout 336 / io 8 /
  format 13 全绿；clippy 0 / cargo fmt 0；trip 177B/1 页 + tripos 750B/
  12 页/0 错（MD5 `92100387` 与基线逐字节相同）；宏包 13/13；small2e
  双通道双 0 错；资产审计 30 项 = 27 PASS / 3 ENGINE-GATE / 0 ERROR；
  CLI 探针 `MODDATE=[D:20261026182401Z]` / `MISSING=[]`。
- **工具健壮性顺手修**：`scripts/asset-load-audit.sh` 相对二进制路径在
  cd 输出目录后失效（30 项全 ERROR 假信号）——统一转绝对路径。
