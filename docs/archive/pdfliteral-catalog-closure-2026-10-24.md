# `\pdfliteral` 三模式 + `\pdfcatalog` openaction 吞参刀（2026-10-24）

## 一句话

`\pdfliteral` 从单臂升级为 origin/direct/page 三模式（origin 按 GT 铁证以 DVI
当前点锚平移域包裹），`\pdfcatalog` 补 `openaction` 子句完整吞参——hyperref
真实文档 PDF 文本层不再泄漏 `openaction goto page 1{/Fit}`，宏包回归 hyperref
文本相似度 93.0% → **100.0%**。

## 背景

上一刀（`1a6eeb8` pdfpageattr）后勘察发现两个相邻缺口：

1. `\pdfliteral` 只有单臂（吞 `direct|page` 关键字但载荷无模式标记），所有
   模式都按 origin 渲染——direct 模式字面量坐标会被错误地平移。
2. `\pdfcatalog{...}` 只吞组不吞后缀子句——hyperref `\PDF@SetupDoc`
   （hpdftex.def L535）真实调用形 `\pdfcatalog{/PageMode /UseNone} openaction
   goto page 1{/Fit}` 的 `openaction...` 残流被排版成正文。

## GT 铁证

pdftex 内容流（/tmp/lit1/gt1.pdf，`line before\pdfliteral{...}line after`）：

```
1 0 0 1 99.397 759.927 cm   ← origin 模式：DVI 当前点锚平移域
0.5 0 0 RG 0 0 m 50 20 l S
1 0 0 1 -99.397 -759.927 cm ← 域收束
```

## 实现

### 载荷协议（core → pdf 后端）

- origin（默认）：`ntex-pdf-literal <data>` → 渲染 `q 1 0 0 1 x y cm … Q`
- direct：`ntex-pdf-literal-direct <data>` → 原样写流
- page：`ntex-pdf-literal-page <data>` → 原样写流（页级隔离未实现，
  KNOWN-SIMPLIFICATIONS 登记）

独立标签防「正文数据恰好以 direct 开头」歧义（`ntex-pdf-literal direct
stray` 落 origin 域，测试锁定）。

### `\pdfcatalog` openaction 吞参

组后可选子句按 pdfTeX action 文法吞：`goto page⟨int⟩⟨group⟩` /
`goto name⟨group⟩` / 未知形态兜底吞组。**关键字只扫一次并按命中分支吞
实参**——此前实现曾对同一词重复 scan_keyword（第二次必不命中，第一次已
消费），把数字实参漏给组扫描器（Missing { inserted，hyperref 实测回归）。

## 验收（全部主控亲测，HEAD `1d3a110` 重建二进制）

| 项 | 结果 |
|---|---|
| core 单测 | 590 通过（+3：pdfliteral_modes_produce_tagged_payloads / pdfcatalog_swallows_openaction_clause / ntex-pdf parse_literal_special_modes 共 34）|
| layout 单测 | 336 通过 |
| clippy -D warnings / fmt | 0 / 干净 |
| lit2 端到端 | origin `q 1 0 0 1 194.5834 657.0000 cm … Q`、direct 裸写流，与 GT 逐字符合 |
| oa1（hyperref 真实 openaction 文档） | 0 错 1 页，PDF 文本层仅 `Hello openaction. 1`，无泄漏 |
| beamer 最小文档 | 0 错，DVI 22590B，文本 `Hello beamer.` |
| 宏包回归 | **13/13**，hyperref 相似度 93.0%→100.0%（本刀真实改善） |
| trip/tripos | 177B / 12 页 750B，SHA `574045fb` 与基线逐字节相等 |

## Review

Claude Code review 通道 124 超时（历史多次，本轮 glm 与默认模型均挂）——按
预案登记不可用，主控亲自 review：scan_keyword 消费语义（命中即消费/不匹配
back_input）、数字与 `{` 交界、坐标换算与图片通道口径一致、panic 面逐项过；
发现 1 项必修（注释声称 page 简化已登记 KNOWN-SIMPLIFICATIONS 但实际未
登记），已补登记后提交。

## 遗留

- page 模式页级隔离（待页级渲染语境需求出现再实现）
- `\pdfcatalog`/`\pdfinfo` 仍不产真实 PDF Catalog/Info 对象（stub 标记，
  第二十三刀登记的债不变）
