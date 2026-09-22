# 分页保守性战役：vsize 时机修复被「杂胶 1000pt」打穿（未落码，纯诊断）

日期：2026-09-22。起点 1abbbe6（浮体页 `\vsize` 事件面 + feed_one 只认收紧重建臂）。
靶子：transformer-standalone 16 页 → GT 14 页（±1）。

## 一、v10（1abbbe6）过度保守的量化结论

GT 对照（transformer-standalone.pdf，真 TeX）：14 页。NTex v10：16 页。
逐页首行对照定位出两处「虚短页」：

| 页 | 现象 | 量化 |
|---|---|---|
| p7 | 浮体页（Fig 2）之后一页只排 291.7pt 文字就断 | 下方浪费 249.69pt ≈ ht(Fig2)+\textfloatsep |
| p10 | 同型：用了 270.03pt 目标就断 | 下方浪费 ~270pt |

合计浪费 ~520pt ≈ 1 页。其余页与 GT 断点一致；p6（浮体页 + 291.6pt 正文，最低文字
y=673.6）**不是**过度保守，它就是真 TeX 的浮体栏形态。

**根因链**：`\@flupdates` 收紧 `\@colroom` → OR 尾 `\global\vsize\@colroom` 收紧
`\vsize`。真 TeX 里这个收紧**即时**生效（`-\@Miv` 强制惩罚触发 `\@specialoutput`
把半成品页 `\unvbox\@holdpg` 退回贡献重建）；NTex 的输出例程在 token 边界注入、
**晚一页**执行 → 收紧值滞留到下一页 freeze（`vsize_live` 事件面只能让 freeze 取到
收紧值，改变不了「什么时候取」）。

## 二、修复尝试（pending_pages 闸）→ 19 页回归

做法：append 路径在 `page_state.pending_pages` 非空（例程已 ship 未消费完）时不继续
喂贡献，让收紧目标在下一页**起点**生效。float 页自身按 291.70908pt 冻结——时机对了。

但随即暴露 NTex 自身缺陷：**主文档流里出现一根刚性 `\vskip 1000.00002pt`
（65536001sp，stretch/shrink 全 0，name=None）**，紧跟 OR 尾 `\vsize` 写入之后：

```
SHIP pg=5 h=629.40024
note_vsize=291.70908            ← OR 尾 \global\vsize\@colroom
VSkip w=1000.00002 st=0 so=0 line=205 depth=4   ← 杂胶
FREEZE goal=291.70908
FIRE_UP pg=9 goal=291.70908 natural=1040.32919 pen=10000  ← 2 行 + 杂胶即断页
```

浮体页因此只吃 2 行正文 → 19 页、大面积空白。v10 之所以没炸，是因为陈旧目标让页先
装满 291.7pt 文字、杂胶恰好落在断点之后（同一根胶一直在，只是从未被断页器计账）。

### 杂胶发射点（有序通道法定位，复现用 head -205 截断版）

`\tracingmacros` 未接转录、`\message`/`\write16` 与 eprintln 缓冲序不可比 → 把 Rust
侧探针改走 `sink.write16()`（与 `\typeout` 同通道，序保真），再用文档级 `\let/\def`
给 OR 宏包 marker，得到：

```
M:tryfcolumn        ← \@startcolumn → \@tryfcolumn \@deferlist（\@fcolmade=false）
M:scolelt           ← \let\@elt\@scolelt \reserved@b
M:addtonextcol      ← \@scolelt#1 → \@addtonextcol
RUSTVSKIP VSkip w=1000.00002 depth=4   ← 在 \@addtonextcol 调用链内发射
```

**latex.ltx 的 `\@addtonextcol`/`\@addtotoporbot`/`\@flupdates`/`\@flcheckspace`
全链没有任何 `\vskip`** → 这是 NTex 侧合成的胶水（嫌疑面：insert 类寄存器读
`\count\@currbox`/`\ht\@currbox`、`\@cons` 的 `\xdef` 重扫、`\@bitor` 位测、或
`\vbox{}`（`\end@float` 尾）在 NTex 的空盒处理）。值 65536001 = 1000pt + 1sp，
与 `prevdepth` 哨兵 −1000pt/IGNORE_DEPTH 同源数量级，但 `\the` 读回
`\@tempskipa`/`\@tempdima`/`\@colroom`/`\@colht`/`\@fptop`/`\@fpbot` 均非 1000pt。

### 消融证据

- 截断到 ≤204 行 → 杂胶 0 次；205 行（itemize 前的 `\subsubsection`）→ 1 次。位置敏感。
- 去掉 `\subsubsection` 行 → 0 次；去掉 `\begin{itemize}` → 仍有。头是必要条件。
- `\@tempskipa`（skip 别名赋值、经宏实参传入再赋值）三种形态均正常（40/41/42pt），
  排除「skip 赋值值臂缺失」。

## 三、下一步（接手者从这里开）

1. 先杀杂胶：在 `\@addtonextcol` 链逐宏 `\typeout` marker + `write16` 有序通道二分，
   或给 `CoreSink::glue` 加「合成胶」来源标记。**必须先于任何分页时机改动**。
2. 杂胶清零后重放 pending_pages 闸（本次已验证：闸本身让 float 页按 291.70908 冻结，
   时机正确）。
3. 预期收益：p7/p10 两处 ~520pt 浪费回收 → 16 → 15 页（再往上要靠 `\@specialoutput`
   的 `\unvbox\@holdpg` 半成品页回退语义，见 memory `latex-float-page-vsize-event-channel`）。

复现件：`/tmp/probe-tp/{t1.tex（head-205）,probe.tex（marker 包裹）}`；
探针环境变量 `NTEX_DEBUG_VSIZE`/`NTEX_DEBUG_VSIZE_FULL`；驱动须
`ntex-dvi t.tex t.dvi --input-path /tmp/probe-tp --input-path /tmp/real-paper`。
