# TRIP 收敛诊断刀（2026-10-28，审计队列①收尾）

日期：2026-10-28。HEAD `49bba04`。性质：**诊断裁决刀，零引擎代码改动**。
背景：2026-10-21 项目偏离度审计把「TRIP 收敛诊断刀」列为队列①——沿 R17 后首个
共同分叉判定剩余 Missing number 与缺页是否与真实 LaTeX 文档同源；不同源则 TRIP
专项维持关闭，结论入 plan.md。

## 本轮新钉的口径陷阱（重要，防下一轮重蹈）

**trip.tex 全本的 GT/自测都必须走 TRIP 自己的 fmt 两步姿势**：
`ntex-dvi trip.tex --no-plain --dump trip.fmt`（工作目录放 `trip.tfm`）→
`ntex-dvi trip.tex --fmt trip.fmt`。若省略 `--no-plain`（plain 预载进快照），
快照里被 plain 注入的 `\shipout`/`\output` 会接管全本，跑出
「9 错 + 未产出页面」的**假形态**——与 tripos 的「latex.fmt 直跑 nullfont 刷屏」
是同族坑（10-23 已登记）的 trip.tex 变体，本轮实测再次确认。

GT 侧：`tex -ini -interaction=nonstopmode trip.tex`（产 trip.fmt，**13 错 =
健康签名**）→ `tex -fmt=trip -interaction=nonstopmode trip.tex`。
Knuth tex，禁 pdftex（trip.tex 的 `\catcode256` 与 pdfTeX 扩展冲突，10-18 已实证）。

## 当前 HEAD 实测（采集于本轮，二进制 = HEAD 重建）

| 通道 | NTex | GT (Knuth tex) |
|---|---|---|
| ini 构造段 | `--no-plain --dump`：**6 错** / trip.dvi 177B/1 页/5 字体 | `tex -ini`：**13 错**（健康签名）/ trip.dvi 251B |
| tripos.tex | `--fmt trip.fmt`：**12 页 / 750B / 0 错** | 12 页 `[0.0.0.0.11]`×12 |
| trip.tex 全本 | **137 错 / 9 页 / 2492B**（Missing number 11） | **8 错 / 16 页 / 2920B**（Missing number 1，`l.94 \openout-'78`） |
| 真实文档判据 | small2e / sample2e 双 **0 错**；宏包回归 **13/13**；资产审计 **27 PASS + 3 ENGINE-GATE / 0 ERROR** | — |

R17 记录的全本 13 条 Missing number → 本轮 11 条：`xspaceskip` 同源项已修清掉 2 条；
剩余 11 条位点（253 `\global\setbox=`、257 `\abovewithdelims.?\right`、298
`\leqno\mathchardef A`、318 `\penalty\lastkern` 链、336 `\span` 对齐模板×2、
390 `\ifdim72p\iftrue`、419 `\ifcase`、428/429 `^^M` catcode 游戏×3）与 R17
归因表逐条吻合，**全部是 TRIP 折磨构造**（缺寄存器号的 `\setbox=`、无数名的
`\mathchardef`、故意的 `\span`/条件嵌套——真实文档不存在这类输入）。

页数差（9 vs 16）主因仍是 R17 已登记的早期发散级联：L315 `J` 的 active
字符分叉起，后续页序在假状态下累积；末页死循环链（GT 也有 Output loop
3 dead cycles + Unbalanced output routine，两侧同构）不改变裁决。

ini 段 NTex 6 错 vs GT 13 错：NTex 对部分 ini 折磨（`\lccode256`、
`\mathchardef\a="8000`、`\skewchar` 非法 mag 等）**接受或静默钳制**而非报错——
报错路径覆盖少于 GT，属「少报不误报」方向；逐条补齐属 TRIP 特有域，无真实文档收益。

## 裁决

1. **剩余 Missing number / 页数差与真实 LaTeX 文档不同源**（四重实证：small2e/
   sample2e 双 0 错、宏包 13/13、资产审计 30 项 0 ERROR、官方三文档 R21 后
   amsldoc 2 错/testmath 0/fntguide 0）。
2. **TRIP 专项维持关闭**，16 页逐字对齐维持长期观测；tripos 12/12 页零错 +
   全本无挂死即为阶段成果（审计定案的够用阈值，本轮复核仍成立且 DVI 750B
   与基线逐字节一致）。
3. 若未来重新开 TRIP 刀，入口是 L315 `J` active 字符分叉（首个可见前序发散点）
   + ini 段 7 条「GT 报 NTex 不报」的报错路径补齐；两者均无真实文档同源证据，
   不满足审计定的「真实文档同时改善」投入门槛。

原始转录：`/tmp/trip-diag/`（trip-full.log / ntex-ini.log / runfull.sh）；
GT：`/tmp/trip-gt/`（trip-ini-gt.log / trip-full-gt.log / gt.sh）。
门禁：core 602 + layout 338 + trip 4 全绿、宏包 13/13、资产审计 27+3/0、
trip 177B + tripos 750B 与基线一致。
