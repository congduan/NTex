# beamer 最小闭环刀：`\pdfpageattr`/`\pdfpagesattr` toks 参数注册（2026-10-24）

## 靶形与成果

资产审计 10 ERROR（8e506b2 基线，plan.md §0.4 后 L139）中 beamer 的首错之一：beamer.cls
经 hyperref 链每页 shipout 触发 `\pdfpageattr`——注册前 beamer 最小加载每页 2 错
（`Undefined control sequence` + `You can't use \the with this`）×6 页。

本刀成果（发行资产树口径，`--fmt latex.fmt` 直跑）：

| 项 | 前 | 后 |
|---|---|---|
| beamer 最小文档错误数 | 每页 2 错 | **0 错** |
| DVI | — | 22590B / 1 页 / 45 字体 |
| PDF 文本层 | — | `Hello beamer.` |

「加载→正文可见→PDF 正确」三要素全达成。WASM 资产包通道（Node + 12s 限时）本会话
不可复现（dump-bundle 环境缺失），发行口径 CLI 验证替代；审计表 beamer 行的升级
待下一轮 WASM 通道复测确认。

## 根因与修法

`\pdfpageattr`/`\pdfpagesattr` 是 pdfTeX 的 **toks 型参数**（hyperref hpdftex.def
L623-652 每页 `\the\pdfpageattr` 读 + `\global\pdfpageattr{…}` 写；lscape.sty L54
`\pdfpageattr{/Rotate 90}`）。NTex 输出 DVI，值不进 PDF 对象——只需「可写可读」存储。

修法（2026-10-24 接管轮定稿）：`Primitive::PdfPageAttr/PdfPagesAttr` 变体 +
Expander 独立字段 `pdf_page_attr_toks/pdf_pages_attr_toks`（非 toks 寄存器槽——
GT 实测 `\toks32765` 与 `\pdfpageattr` 互相隔离），赋值臂在
`dispatch_toks_state`（RHS 语义同 `\every*` 族：filler + 组/toks 引用，
`set_every_scoped` kind 7/8 组局部恢复），checkpoint ValueExtras 携带 +
fmt eqtb digest 覆盖。**不新增 fmt codec 字段、不 bump VERSION**（注册走
builtins 注册链入 eqtb，重生成发行 fmt 即可见）。

## GT 铁证（pdfTeX 实测，`\immediate\write16` 通道）

toks 赋值**不展开**替换文本——`\global\pdfpageattr{\attr^^J/Trans{}>>}` 存别名原样，
`\the` 读回 `\attr ⏎/Trans{}>>`。**递归形态 `\global\pdfpageattr{\the\pdfpageattr…}`
在真 pdfTeX 下 TeX capacity exceeded（input stack 10000）**——上一轮遗留 WIP 的
回归锁正用了这个递归形态且断言「ATTR=[/Rotate 90」展开值，属锁期望错非引擎错。
本刀按 GT 同构改锁：edef 先展开再存、断言存储不展开 + pdfpagesattr round-trip。

回归锁 `pdfpageattr_roundtrips_as_toks_parameter`（去修复必红：删 set_register 即
Undefined）。GT 探针现场 `/tmp/gt-pageattr/t1-t4.tex`。

## ⚠ 接管收口记（10-24 cron 轮）

上一轮工作树在「toks 槽」方案重构为「独立字段」方案**中途被打断**：BUILTINS
长度声明 454 vs 实际 456（编译崩）、`dispatch_toks_state` 无赋值臂（走到
internal error）、`every_field/set_every_field` 缺 kind 7/8 映射（恢复臂有、
赋值会错写进 everymath）。战报早前声称的验证数字（587 绿等）出自方案切换前
的中间态，不可信。接管轮补齐三处后全链实测：core **587 绿**（+1 新锁）/
layout 336 / clippy 0 / fmt clean / 宏包 **13/13** / trip 177B+tripos 12 页
750B / beamer E2E 0 错 22493B DVI → PDF 文本 `Hello beamer.`（全部本轮
亲测原始输出）。

## ⚠ 关键教训：fmt 快照是 eqtb 权威源——「注册了 ≠ fmt 路径可见」

单测（in-process 注册链）绿 + **plain 通道 CLI 绿**，但 `--fmt latex.fmt` 路径
`\pdfpageattr` Undefined：fmt 快照恢复整体覆盖 eqtb，今天的注册不在昨天生成的快照里。
`pdfpagewidth` 能活是因为 latex.fmt 生成晚于其 v23 注册。

- 判别法：新原语「单测绿 + plain 绿 + fmt 红」三态 = fmt 快照陈旧，非注册缺失。
- 铁律（再证）：**修引擎语义后必须重生成发行 fmt 并入库**。本刀
  `cargo run -p ntex-dvi -- --generate-fmt` → 9097042B 入库 `assets/fmt/latex.fmt`
  （FORMAT_VERSION 26 不变——注册走既有槽位非 codec 字段）。

## TRIP 防回退线（跑法坑再证）

首轮复测 tripos 0 页假红 = 直跑缺 `--no-plain`（plain 预载污染 TRIP fmt，
trip.tex DVI 251B≠177B 即污染证据）。正确姿势（`docs/archive/math-accent-skew-2026-10-23.md` L38）：

```bash
cd <workdir 含 trip.tex/tripos.tex/trip.tfm>
ntex-dvi trip.tex --no-plain --dump trip.fmt   # 177B / 1 页 / 5 字体
ntex-dvi tripos.tex --fmt trip.fmt --no-plain  # 750B / 12 页
```

复测 177B + 12 页×750B 达标。

## 验证原始输出

- `cargo test -p ntex-core`: **587 passed**（+1 新锁）；`-p ntex-layout`: **336 passed**
- clippy `-p ntex-core --all-targets`: 0 error；`cargo fmt --all --check`: clean
- 宏包回归 **13/13 PASS**（amsmath 94.8% / hyperref 93.0% / graphicx 68.4% …，
  负例 footmisc PASS）
- beamer E2E：`ntex-dvi min.tex --fmt latex.fmt`（发行资产树，无 --auto-pkg）
  grep -c '^!' = **0**；dvipdfmx → pymupdf `Hello beamer.`

## 残余与下一步

- 审计 10 ERROR 现状：biblatex ✅（3fd1477）；beamer 首错 ✅（本刀，WASM 通道待复测）；
  ctex/ctexart（fandol fontset）+ xeCJK + fontspec + unicode-math 疑 XeTeX 系能力边界；
  minted/tikz/pgfplots = 输入栈超限族（beamer 同族已破，按 beamer 修复经验重测优先）。
- beamer 深水区（frame 机制/overlay/主题）不在本刀范围——本刀只保证最小加载→出页。
