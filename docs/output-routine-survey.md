# NTex 输出例程勘察报告（2026-09-06，基于 HEAD 08a97c6）

> 目标：LaTeX 兼容战役从「宏载入战」转场「输出例程战」前的量化勘察——latex.ltx 的
> `\output` 侧到底要什么、ntex-layout 现状有什么、缺口按「阻塞 sample2e 出 PDF」排序、
> 逐刀怎么切。
>
> **边界声明**：本报告只读。`crates/ntex-core/` 一行未读（主线会话领地），引擎对
> ntex-core 侧的推断都显式标注「待主线核」；唯一新增文件即本报告。真实 TeX
> ground truth 在 `/tmp/ors-work/`（工作目录，未入库，复现见 §6）。

---

## TL;DR

- **「输出例程战」其实是三层，不是一层**。latex.ltx 里 `\output` 例程（L20798）只是
  中层；它上面有 **ltshipout.dtx（L20159–20606）把 `\shipout` 整个重定义**成 l3 包装
  （`\cs_gset_eq:NN \shipout \__shipout_execute:`，L20207），下面有 **ltmarks 的
  expl3 标记机**（每页必调，sample2e 3 页窗口里 `\splitfirstmarks/\splitbotmarks`
  各触发 24 次）。只盯 `\@outputpage` 会漏掉两头的硬依赖。
- **真 TeX 实测（TinyTeX 2026，sample2e 3 页）**：输出例程窗口共 18755 行 trace、
  6765 次宏展开、349 个不同宏（其中 `\@` 系 131 个）。原语需求面 top：
  `\par`×303、`\global`×277、`\let`×220、`\lastnodetype`×181、`\hskip`×172、
  `\unskip`×155、`\lastskip`×102、`\aftergroup`×61、`\catcode`×72、`\setbox`×70、
  `\lastbox`×26、`\splitfirstmarks/\splitbotmarks` 各×24、`\penalty`×46。
- **简报里「`\the\pagegoal`/`\pagetotal` 是否齐」是个伪需求**：latex.ltx 全文
  `\pagegoal`×0、`\pagedepth`×0、`\pagestretch`×0、`\pagetotal`×1、`\pageshrink`×1
  （后者只在 `\enlargethispage` 路径）。LaTeX 的输出例程**不读页构建器影子值**，
  它自己用 `\@colht/\@colroom` 记账。真正卡脖子的是另外几样。
- **真缺口（按阻塞排序）**：① `\outputpenalty`（`\output` 例程**第一个 token** 就是
  `\ifnum\outputpenalty<-\@M`，引擎断页时不记断点惩罚；LaTeX 的**罚分协议**六档
  `-10000/-10001/-10002/-10003/-10004/-20000` 全靠它分派，见 §2.2bis）② box255 寄存器化
  （255 是页面队列而非寄存器：`\unvbox\@cclv`×2、`\vsplit\@cclv`×1 在 latex.ltx 中
  **现状必炸**）③ insert 完全没建模（`\insert` 体被 `toks_to_text` 有损压串，
  `\unvbox\footins` 不可用；sample2e 有一个脚注，**必经**）④ `\deadcycles`
  （latex.ltx 6 个用点：`\maxdeadcycles=100`(20608)、`\@specialoutput`(20831)、
  `\enddocument`(15495)、`\stop`(15560)、`\include`(9784)、ltshipout discard 分支
  `\tex_deadcycles:D`(20245)）
  ⑤ `\count0` 页号链（引擎现在硬编码 `[0.0.0.0.N]`，真 TeX 实测 `[5.7]` 格式）。
- **好消息**：`\insertpenalties`/`\holdinginserts` 在 latex.ltx **零使用**，insert 断页
  成本可以后置；e-TeX 页影子值基本不用（`\pagegoal`×0）、`\mark` 经典原语在
  latex.ltx **零使用**（ltmarks 全走 `\newmarks`+`\tex_marks:D`）、`Node::Ins` 事件面已存在。
- 量级：到 sample2e 出 3 页 DVI/PDF（验收锚点：**3 页 / 7576 字节 / 页号 [1][2][3]**），
  输出例程专项预计 **8–10 刀**（§5）；另有 1 个结构性 flag（增量管线对 LaTeX 文档
  恒定退化为全量，§4-G10）。

---

## 1. 勘察方法与 ground truth 资产

### 1.1 真实 TeX 对拍法（本轮新增，可复现）

VM 自带 TinyTeX（`export PATH=$HOME/.local/bin:$PATH`；pdfTeX
3.141592653-2.6-1.40.29，TeX Live 2026）。核心手法：

```tex
%&latex
\tracingmacros=1\tracingoutput=1
\input sample2e.tex
```

`\tracingmacros` 会把**每次宏展开的定义体**打进 log，于是输出例程窗口 =
`[第一处 \@makecol -> …, "Completed box being shipped out [N]"]`。对这段窗口做
grep 即得到「输出例程实际执行了哪些宏、用了哪些原语、频次多少」——这是本报告
全部量化数据的来源。sample2e 3 页的窗口：

| 页 | 窗口（s2.log 行） | 宽度 |
|---|---|---|
| 1 | 121315–124021 | 2706 行 |
| 2 | 133464–136132 | 2668 行 |
| 3 | 137241–140069 | 2828 行 |

合计 18755 行 trace、6765 次宏展开、**349 个不同宏**（`\@` 系 131 个）。

### 1.2 对照产物（验收锚点）

- `sample2e` → `Output written on s2.dvi (3 pages, 7576 bytes)`。
- 页号标签语义实测：`\setcounter{page}{5}\count1=7` → log 里 `[5.7` … `[6.7]`
  ——**`\count0` 起逐段追加非零 `\count1..9`，点分**，不是引擎现在的
  `[0.0.0.0.N]`（paging.rs:12）。
- 极小文档链序实测（`min2.tex`：`\newpage` 触发）：

```
\newpage → \par → \@makecol → \@outputbox@removebskip → \@outputbox@append
→ \@if@footnotes@TF → \@backup@outputbox@depth → \@outputbox@appendfootnotes
→ \@outputbox@attachfloats → \@make@normalcolbox → \@textbottom → \@opcol
→ \@expl@@@mark@update@singlecol@structures@@ → \@outputpage
→ (\@resetactivechars \@makeother×11 \@parboxrestore \reset@font \normalsize
   \@begindvi \@thehead/\@oddhead \@thefoot/\@oddfoot)
→ \shipout(l3 包装) → "Completed box being shipped out [1]"
```

---

## 2. latex.ltx 侧清单（2026-06-01，22838 行）

### 2.1 第一层：ltshipout.dtx（L20159–20606，447 行）——`\shipout` 被重定义

| 位置 | 内容 | 引擎含义 |
|---|---|---|
| L20178–20180 | `\box_new:N \l_shipout_box`、`\cs_set_eq:NN \ShipoutBox \l_shipout_box`、`\box_new:N \l__shipout_raw_box` | expl3 盒寄存器分配（3 个） |
| L20201–20207 | `\__shipout_execute:` 用 `\tex_afterassignment:D` + `\tex_setbox:D \l_shipout_box`，然后 **`\cs_gset_eq:NN \shipout \__shipout_execute:`** | **`\shipout` csname 被夺走**；原语只能以 `\tex_shipout:D` 存活 |
| L20209–20212 | `\tex_currentgrouplevel:D` 比较 + `\tex_aftergroup:D` | 需 `\afterassignment`/`\currentgrouplevel`/`\aftergroup` |
| L20241–20266 | `\g_shipout_totalpages_int`/`\g_shipout_readonly_int`、`\@abspage@last` | 绝对页号计数 |
| L20276 | **`\tex_shipout:D \box_use:N \l_shipout_box`** | 唯一的原语点火点（`\shipout\box<reg>` 形态） |
| L20346–20355 | `\l_shipout_box_ht/_dp/_wd_dim` | `\box_ht:N` 族查询 |

**引擎含义**：引擎的 `\shipout` 原语与 `\let` 出来的 `\tex_shipout:D` 别名必须独立于
`\shipout` csname 的重定义（expl3 原语重命名区已在主线战场，此处是它第一次承担
「原语别名必须逃过宏层重定义」的输出侧责任）。

### 2.2 第二层：ltoutput.dtx（L20606–21870，1264 行，~220 条 def/alloc）

`\output` 例程本体（L20798–20825）逐臂职责：

```
\output{%                                  L20798
  \let\par\@@par                           L20799  例程内禁 \par 重定义
  \ifnum\outputpenalty<-\@M                L20800  ★ 第一个判据（-\@M=-10000）
    \@specialoutput                        L20801  特殊输出（\newpage/\clearpage/\supereject）
  \else
    \@makecol                              L20803  列装配（\@cclv→\@outputbox）
    \@opcol                                 20804  单/双栏分派 + marks 更新
    \@startcolumn                           20805  浮动体试排
    \@whilesw\if@fcolmade\fi{\@opcol\@startcolumn}  20806-20808
  \fi
  \ifnum\outputpenalty>-\@Miv              L20810  (-\@Miv=-10004)
    \ifdim\@colroom<1.5\baselineskip ...   20811-20816  仅浮动体页警告 + \@emptycol
    \else \global\vsize\@colroom \fi       20817/20820  ★ 例程改 \vsize → 页构建器须感知
  \else \global\vsize\maxdimen \fi         L20823  强制页（\clearpage 尾）后放开 \vsize
}
\gdef\@specialoutput{...}                  L20826–20872  按 \outputpenalty 五档分派：
                                                   >-\@Mii(>-10002)→\@doclearpage(L20879)
                                                   <-\@Miii(<-10003)→\@holdpg 暂存，
                                                     <-\@MM(<-20000) 时 \deadcycles\z@
                                                   其余→\@holdpg+\@cclv 合并+\lastbox 剥行
                                                   (20837)+\@reinserts+\@addtocurcol/
                                                   \@addmarginpar（\@next\@currbox 判别 20843）
```

关键阈值常量：`\@M=10000`（L328）、`\@MM=20000`（L329）、`\@Mi=10001`（L8786）、
`\@Mii=10002`（L8787）、`\@Miii=10003`（L8788）、`\@Miv=10004`（L8789）、
`\@m=1000`（L327）、`\@xxxii=32`（L8785）；`\maxdeadcycles=100`（L20608）。

### 2.2bis 罚分协议（刀 1 必须精确复刻的规格）

| 罚分 | 发出点 | 例程侧走向 |
|---|---|---|
| `-\@M` = -10000 | `\newpage`(L20728：`\par\vfil\penalty-\@M`)、`\@emptycol`(L20743) | `\ifnum\outputpenalty<-\@M` 为**假** → 正常输出臂 `\@makecol…` |
| `-\@Mi` = -10001 | `\clearpage`(L20704：`\vbox{}\penalty-\@Mi`) | **真** → `\@specialoutput` → `>-\@Mii` 真 → `\@doclearpage` |
| `-\@Mii` = -10002 | 行内 float（`\@xfloat` 设 `\@floatpenalty-\@Mii`）、边注 | `\@specialoutput` → `>-\@Mii` **假** → `\@holdpg` 合并 + `\@addtocurcol` |
| `-\@Miii` = -10003 | 垂直模式 float | 同上但 `<-\@Miii` 真 → 暂存 `\@holdpg` 等下一页 |
| `-\@Miv` = -10004 | `\end@float`(L17739：`\penalty-\@Miv\vbox{}\penalty\@floatpenalty`) | 例程末臂 `\global\vsize\maxdimen`（强制页） |
| `-\@MM` = -20000 | **latex.ltx 不发**（`\supereject` 属 plain/类层）；latex.ltx 只在 20831 **测试**它，另作 `\floatingpenalty\@MM`(17954) | `<-\@MM` 真 → `\deadcycles\z@`（清死循环计数） |
| 就地插入后 `\outputpenalty\z@` | `\@addtocurcol` L21518–21519 | 重置，防止重复触发 |

> 这张表就是 `\outputpenalty` 的**语义规格**：引擎只需在 FireUp 时把触发断点的
> penalty 交给例程，latex.ltx 自己完成全部分派；但分派对 ±1 的档位差敏感
> （-10000 与 -10001 走完全不同的路），**不能模糊化**。

**主链宏（定义行号 + 一句话职责；闭包全量 ~155–165 个名字，此处列 60 个承重点）**：

*派发/链表机制（cons-cell：列表是 `\@elt\bx@A\@elt\bx@B…` 形式的宏体）*

| 宏 | 行号 | 职责 |
|---|---|---|
| `\@elt` | 20609/20624/20641 | 运行时遍历器槽位，`\let` 切换；20641 变成 `\noexpand\@elt\noexpand` 以便 `edef` 序列化 |
| `\@next` / `\@xnext` | 20610 / 20612 | 弹链表头（`\expandafter` + 定界 `\@@`）；`\@xnext` 做 car/cdr 拆分 |
| `\@cons` | 1227 | cons 追加：`\begingroup\let\@elt\relax\xdef#1{#1\@elt #2}\endgroup` |
| `\@bitor` / `\@xbitor` | 20616 / 20618 | float 说明符位测试：`\divide`+`\ifodd` |
| `\@whilesw` | 8810 | `\if@fcolmade` 循环 |
| `\@testfalse/\@testtrue/\if@test` | 20613/20614 | 位测试结果（`\global\let\if@test\if…`，非 `\newif`） |
| `\@freelist` | 20625/20642 | 52 个 float 盒的空闲池（`\bx@A…\bx@ZZ`，20630/20640 裸调用真分配） |
| `\@latexbug` / `\@fltovf` | 9082 / 9080 | `\@next` 取空兜底 / "Too many unprocessed floats" |

*列盒构造*

| 宏 | 行号 | 职责 |
|---|---|---|
| `\@makecol` | 20930 | `\setbox\@outputbox\box\@cclv` + 剥尾胶水 + socket + `\ifvbox\@kludgeins` 分派 + `\global\maxdepth\@maxdepth` |
| `\@make@normalcolbox` | 20948 | `\vbox to\@colht{\@texttop …\unvbox\@outputbox \vskip-\@outputbox@depth \@textbottom}` ——**精确高度列** |
| `\@make@specialcolbox` | 20957 | `\enlargethispage` 路径；**用 `\pageshrink`**(20962) |
| `\@outputbox@depth` | 20947 | `\newdimen`，暂存 `\@outputbox` 深度（`\vbox to` 会丢深度） |
| `\@outputbox@removebskip` | 20978 | `\lastskip`+`\gluestretchorder`+`\unskip`，`\xdef`+`\noexpand`+`\the` 动态重建"回插胶水"闭包 |
| `\@backup@outputbox@depth` | 21004 | `\maxdepth` 超深补偿 |
| `\@outputbox@append` | 21015 | `\setbox\@outputbox\vbox{\boxmaxdepth\@maxdepth …\unvbox\@outputbox #1}` ——**自重装** |
| `\@outputbox@appendfootnotes` | 21025 | `\ifvoid\footins` + `\skip\footins` + `\footnoterule`(17873) + `\unvbox\footins` |
| `\@outputbox@attachfloats/top/bottom` | 21043/21047/21050 | 浮动体接挂（`\@cflt`/`\@cflb`） |
| `\@combinefloats` | 21272 | `\let` 到 `\@outputbox@attachfloats`（旧 API 兼容别名） |
| `\@reinserts` | 21146 | `\insert\footins{\unvbox\footins}` + `\ifvbox\@kludgeins…`——**insert 重注入** |
| `\@if@footnotes@TF` / `\@if@bottomfloats@TF` / `\@if@flushbottom@TF` | 21063 / 21070 / 21056 | `\ifvoid\footins` / `\ifx\@botlist\@empty` / `\ifx\@textbottom\relax` |
| `\@texttop` / `\@textbottom` | 21151 / 21152 | 可重定义宏（`\raggedbottom/\flushbottom` 18620–18623 改写）——**不是寄存器** |

*输出/页眉页脚*

| 宏 | 行号 | 职责 |
|---|---|---|
| `\@opcol` | 20919 | 单栏→`\@outputpage`；双栏→`\@outputdblcol`；先更新 marks 结构 |
| `\@outputpage` | 21170 | 页装配 + `\shipout`(21227)（见下） |
| `\@begindvi` | 21268 | `\unvbox\@begindvibox` 后自废（`\global\let\@begindvi\@empty`，仅首页） |
| `\@thehead`/`\@thefoot` | 20702/20703 | 页眉脚间接层；`\@outputpage` 21214–21220 按 `\ifodd\count\z@` 切奇偶 |
| `\@outputdblcol` | 21809 | 双栏：`\@leftcolumn←\copy\@outputbox` + `\hbox to\textwidth` 拼两栏 + `\columnseprule` |
| `\@emptycol` | 20743 | `\vbox{}\penalty-\@M`（空栏触发下一次例程） |
| `\@resetactivechars` / `\@activechar@info` | 21163 / 21153 | 例程期 active char 告警 |
| `\ps@empty` / `\ps@plain` | 18597 / 18600 | 页样式（`\ps@headings` 在类文件，不在 latex.ltx） |
| `\pagenumbering` / `\thepage` / `\c@page` | 15085–15087 / 15086 / **15083 `\countdef\c@page=0`** | **`\count0` ≡ `\c@page`**；`\stepcounter{page}` 调用点 21265 |

*浮动体支线（sample2e 不触发，量级另计）*

| 宏 | 行号 | 职责 |
|---|---|---|
| `\@startcolumn` / `\@startdblcolumn` | 21329 / 21342 | `\@colroom=\@colht` + `\@tryfcolumn\@deferlist` + `\@scolelt` 扫描 |
| `\@tryfcolumn` / `\@makefcolumn` | 21354 / 21802 | float 列/float 页试排 |
| `\@vtryfc` / `\@wtryfc` / `\@xtryfc` / `\@ytryfc` / `\@ztryfc` | 21370/21382/21387/21403/21419 | float 页组装 + 资格判定（`\@bitor`/`\@testfp`/`\@testwrongwidth`/`\ht#1>\@colht`） |
| `\@addtocurcol` / `\@addtotoporbot` / `\@addtobot` / `\@addtonextcol` / `\@addtodblcol` | 21474/21452/21438/21539/21570 | 放置决策（`\@reqcolroom` 预算、失败降级、deferlist 入队） |
| `\@setfloattypecounts` / `\@boxfpsbit` / `\@setfpsbit` / `\@flsetnum` / `\@flsettextmin` / `\@flcheckspace` / `\@flupdates` / `\@resethfps` | 21724/21733/21744/21764/21771/21778/21792/**21752** | 位图拆解与簿记（正名 **`\@resethfps`**，无 `\@resethf`） |
| `\@xfloat` / `\end@float` / `\@floatplacement` / `\@dblfloatplacement` | 17662 / 17739 / 17797 / 17805 | float 入口/结束哨兵/预算参数 |
| `\@cflt` / `\@cflb` / `\@comflelt` / `\@combinedblfloats` | 21274/21290/21305/21309 | float 拼接（`\boxmaxdepth` 用点 21279） |
| `\@testwrongwidth` / `\f@depth` | 20873 / 20878+17805 | 双栏 float 深度哨兵 `1sp` |
| `\ShowFloat`（调试） | 21682 | `\showbox`+`\showboxbreadth10\showboxdepth3` ——**`\showboxbreadth/depth` 在 latex.ltx 的唯一用点** |

*边注 / 脚注 / `\enlargethispage` / 标记*

| 宏 | 行号 | 职责 |
|---|---|---|
| `\@addmarginpar` | 21614 | 例程内消费边注：`\@next\@marbox\@currlist` + `\marginparpush` 碰撞 + `\hb@xt@\columnwidth{…\box\@marbox\hss}` |
| `\marginpar` / `\@xympar` / `\@savemarbox` | 17814 / 17856 / 17838 | 入口（**`\@marbox` 是宏，指向 `\@freelist` 的 insert**） |
| `\footnote` / `\@footnotetext` | 17941 / **17950** | `\insert\footins{…}`；体内 **`\splittopskip\footnotesep \splitmaxdepth\dp\strutbox \floatingpenalty\@MM`**(17953–17954) |
| `\enlargethispage` / `\@enlargepage` | 21658 / 21665 | `\insert\@kludgeins{#1\vskip-\@tempskipa}`(21674)；`\wd\@kludgeins=\z@` 判星号版 |
| `\markboth` / `\markright` / `\leftmark` / `\rightmark` | 18606 / 18611 / 18617 / 18618 | 2e 层标记 API，全走 ltmarks 的 `\mark_insert:nn`/`\mark_use_last:nn` |
| `\@expl@@@mark@update@singlecol@structures@@` | 18582 | `\@opcol`(20925) 每页调用；内部 `\__mark_update_singlecol_structures:`(18492) 用 `\vbox_unpack:N` 抽 `\@outputbox` 的标记 |
| `\mark_update_structure_from_material:nn` | 18222 | 用 `\tex_vsplit:D`(18199/18214)+`\tex_splitbotmarks:D`(18234)+`\tex_splitfirstmarks:D`(18246) ——**ltmarks 是 `\vsplit` 的第二个用户** |

*控制流入口*

| 宏 | 行号 | 职责 |
|---|---|---|
| `\clearpage` | 20704 | `\hbox{}`(vm) + `\newpage` + `\@@write\m@ne{}` + `\vbox{}` + **`\penalty-\@Mi`** |
| `\newpage` | 20728 | `\par\vfil\penalty-\@M` |
| `\cleardoublepage` | 20718 | `\clearpage` + 奇偶补空页 |
| `\enddocument` | 15472–15495 | `\clearpage` → hooks → **`\deadcycles\z@\@@end`** |
| `\stop` | 15560 | `\clearpage\deadcycles\z@…\@@end` |
**`\@outputpage`（L21170–21262）的引擎需求面**（这是 DVI 产出的实际装配点）：

1. `\begingroup` 后 **24 条 `\catcode` 赋值**（`\catcode`\^^M 5`、`\catcode`\%14`、
   `\catcode`\^^I 10` 等）——页眉页脚在"活性字符全灭"的 catcode 环境里排版；
2. `\shipout \vbox{…}` ——**盒组体在例程内部现场构造**，体内 `\aftergroup\endgroup`
   与 `\aftergroup\set@typeset@protect` 配对（**`aftergroup`×61/3 页**）；
3. `\ifodd\count\z@` ——**读 `\count0`** 决定 `\@oddhead/\@evenhead`（双面）；
4. `\reset@font \normalsize \normalsfcodes` ——**NFSS 字体选择在例程内再入**
   （min2 窗口里因此拖入 `\pickup@font/\f@size/\f@series` 等 ~20 个字体宏）；
5. `\baselineskip\z@skip \lineskip\z@skip \lineskiplimit\z@` ——胶水参数赋值；
6. `\vskip\topmargin` + `\moveright\@themargin \vbox{…}` ——**垂直模式 `\moveright`**
   （DVI right/push）；
7. `\setbox\@tempboxa \vbox to\headheight{…}` + `\dp\@tempboxa\z@` + `\box\@tempboxa` ——
   页眉盒深度归零后入页；`\vskip\headsep`；`\box\@outputbox`；`\baselineskip\footskip`
   后接页脚盒 `\hb@xt@\textwidth{\@thefoot}`；
8. `\stepcounter{page}` ——**页号计数器在 shipout 之后自增**（L21265）；
9. `\pdfannot@link@off@@` → `\csname pdfannot_link_off:\endcsname`（L21169）——未定义
   即 `\relax`，**依赖「`\csname` 未定义名=relax」语义**（NTex 第七轮已实现）。

### 2.3 第三层：ltmarks（expl3 标记机）

- `\@opcol` 每页必调 `\@expl@@@mark@update@singlecol@structures@@`（L20924）。
- sample2e 3 页窗口里 `\splitfirstmarks`/`\splitbotmarks` **各 24 次**、`__mark_*`
  内部宏 1638 次；`\mark`/`\topmark`/`\firstmark`/`\botmark` 经由 **`\tex_marks:D`**
  包装访问（e-TeX 多 class marks）。
- 引擎侧 marks 目前**绑在贡献流**上（sink.rs:1212-1221），断页轮转 `rotate_marks`
  （mod.rs:790-797）；`\splitfirstmarks/\splitbotmarks` 接口存在但**恒空**
  （sink.rs:1257-1274）。

### 2.4 数据结构面（寄存器 / 插入 / 列表 / 页眉页脚 / 页码）

| 类别 | 明细 | 量 |
|---|---|---|
| 插入 | L20624 `\let\@elt\newinsert` + `\@freelist`(20625)/`\reserved@a`(20631) 批量分配 **`\bx@A…\bx@ZZ` = 52 个**（20630/20640 裸调用真分配）＋ `footins`(17869，带 `\skip`= `\bigskipamount`/`\count`=1000/`\dimen`=8in 三联 17870–17872)、`\@mpfootins`(16578)、`\@kludgeins`(21655)、`\reserved@a`(421) | **56 个 `\newinsert`** |
| 盒寄存器 | `\@cclv=255`（**L325 `\chardef`，不是 `\newbox`**）；`\@outputbox`(20699)、`\@leftcolumn`(20700)、`\@holdpg`(20701)、`\@begindvibox`(20667)、`\voidb@x`(489)、`\@tempboxa`(8796)；ltshipout 侧 `\l_shipout_box`(20178)/`\l__shipout_raw_box`(20180)/`\l__shipout_firstpage_box`(20352)、ltmarks 侧 `\l__mark_box`(18155) | 全文 `\newbox`×19 + expl3 `\box_new:N`×5 |
| **架构要点** | **`\@currbox`/`\@marbox` 没有 `\newbox` 行**——它们是 `\@xnext` 生成的 `\def\@currbox{\bx@A}` 形式的**宏**，指向 free list 的 insert 寄存器；`\count\bx@X` 存 float 说明符位图。这就是"必须用 insert 而非普通 box"的原因（fallback `\def\@currbox{\@tempboxa}` 17828） | 决定刀 4 的设计 |
| 分配总量 | `\newdimen`×88、`\newcount`×58、`\newskip`×34、`\newtoks`×9、`\newmuskip`×1 | 寄存器空间压力 |
| 页面布局维 | `\topmargin`(20652)/`\oddsidemargin`/`\evensidemargin`/`\@themargin`(20655 `\let`)/`\headheight`(20656)/`\headsep`/`\footskip`/`\textheight`/`\textwidth`/`\columnwidth`/`\columnsep`/`\columnseprule`/`\marginpar*`(20664–66)/`\@maxdepth`(20668)/`\paperheight`/`\paperwidth`/`\stockheight`/`\stockwidth`(20673) | 19 个 `\newdimen`（20652–20673） |
| 栏高记账 | `\@colht`(20693)/`\@colroom`(20694)/`\@pageht`/`\@pagedp`(20695–96)/`\@mparbottom`(20697)/`\@textmin`/`\@fpmin`(20691–92)/`\@toproom`/`\@botroom`/`\@dbltoproom`/`\@textfloatsheight`(21716)/`\@reqcolroom`(21715) | LaTeX 自己的"页影子值"（**不读引擎的 `\pagetotal`**） |
| 浮动体 token 列表 | `\@toplist(20645)/\@botlist/\@midlist/\@currlist/\@deferlist/\@dbltoplist/\@dbldeferlist(20651)` + 工作列表 `\@trylist`/`\@failedlist`/`\@flsucceed`/`\@flfail`(21356–21388) | 11 个列表宏 |
| if 开关 | `\if@insert/\if@fcolmade/\if@specialpage/\if@firstcolumn/\if@twocolumn/\if@twoside/\if@reversemargin/\if@mparswitch`(20674–20681) + `\if@test`(非 `\newif`，20613 生成) | 9 个 |
| 页眉页脚 | `\ps@empty`(18597)/`\ps@plain`(18600)（`\ps@headings` 在类文件）；`\@oddhead/\@oddfoot/\@evenhead/\@evenfoot` 仅 `\let…\@empty`(18598–18602)；`\@thehead/\@thefoot`(20702–03) 间接层；21214–21220 按 `\ifodd\count\z@` 切换 | `\@outputpage` 现场拼装 21227–21259 |
| 页码 | **`\c@page` = `\countdef\c@page=0`**(15083)，即 `\count0` 本尊；`\cl@page`(15084)、`\pagenumbering`(15085–87)、`\thepage`(15086)、`\stepcounter{page}` 调用点 **21266**（shipout **之后**） | 读+写双向 |
| 标记/边注/放大 | `\@kludgeins`（`\enlargethispage`，21655–21674）；`\@mpfootins`（minipage 脚注）；`footins`（脚注，17869） | 插入三联 |

**勘误表（简报/直觉 → 实测）**：`\@mpbox` **不存在**（minipage 脚注用 `\@mpfootins`）；
`\@colnumber` **不存在**（正名 `\col@number` 20682）；`\@resethf` **不存在**（正名
`\@resethfps` 21752）；`\@Miv` = 10004（非 40000）；`\mark/\topmarks/\botmarks` 经典
原语在 latex.ltx **零使用**；`\@texttop/\@textbottom` 是**宏**不是寄存器。

**关键用量统计（决定引擎要不要做）**：`\outputpenalty`×8（3 个宏：`\output`、
`\@specialoutput`、`\@addtocurcol`）、`\deadcycles`×6、`\footins`×15（`\ifvoid`×5、
`\skip`×3、`\insert`×2、`\unvbox`×1、`\ht/\dp/\dimen/\count` 各×1）、`\@cclv`×7
（`\box`×3、`\unvbox`×2、`\vsplit`×1、`\setbox`×1）、`\splittopskip/\splitmaxdepth/
\floatingpenalty`×1（`\@footnotetext` 17953–17954，**在 sample2e 脚注路径上**）、
`\ifvoid`×9（6 个宏）、`\ifvbox`×4、`\ifhbox`×**0**、`\lastbox`×8（ltoutput 内 1 宏）、
`\vsplit`×2（`\@doclearpage` + ltmarks）、`\insertpenalties`×**0**、
`\holdinginserts`×**0**、`\pagegoal`×**0**、`\pagedepth`×**0**、`\pagetotal`×1
（`\clearpage` 20707）、`\pageshrink`×1（`\@make@specialcolbox` 20962）。

### 2.5 sample2e 实测需求面（真 TeX trace，3 页窗口合计）

经典原语（宏体内出现次数，含展开频次加权）：

| 原语 | 次 | 原语 | 次 | 原语 | 次 |
|---|---|---|---|---|---|
| `\catcode` | 72 | `\setbox` | 70 | `\penalty` | 46 |
| `\unskip` | 40 | `\unvbox` | 14 | `\lastbox` | 16 |
| `\ifvoid` | 18 | `\lastskip` | 102 | `\aftergroup` | 61 |
| `\vbadness` | 11 | `\vfuzz` | 15 | `\ifodd` | 3 |
| `\ifvbox` | 3 | `\deadcycles` | 1 | `\splitmaxdepth` | 1 |
| `\topskip` | 1 | `\vsplit` | 0 经典/3 经 `tex_vsplit:D` | `\insert` | 1 |
| `\footins` | 16 | `\UseHook` | 165 | `\UseSocket` | 49 |
| `\UseTaggingSocket` | 32 | | | | |

经 `\tex_…:D` 包装触达的原语（expl3 面，说明**原语别名链**在输出例程内同样承重）：

```
par=303  global=277  let=220  lastnodetype=181  hskip=172  unskip=155  parskip=92
everypar=69  setbox=45  advance=42  the=29  lastbox=26  splitfirstmarks=24
splitbotmarks=24  long=24  noindent=23  glueexpr=23  gdef=22  ignoreprimitiveerror=21
currentgrouplevel=12  expanded=11  deadcycles=9  copy=7  vfuzz=6  vbadness=6
splitmaxdepth=6  aftergroup=6  vbox=4  vsplit=3  shipout=3  afterassignment=3
hbox=2  xdef=1  def=1
```

窗口内 distinct 宏 349 个，其中 **hooks/sockets 调用 246 次/3 页** ——hook 机制
（`\UseHook/\UseSocket/\UseTaggingSocket`）在输出例程上下文的承重远超预期。

---

## 3. 引擎侧现状（crates/ntex-layout，HEAD 08a97c6）

### 3.1 已有机制（全部带 文件:行 证据）

- **页面构建器** `PageBuilder`（`src/page.rs:36`）：影子值齐全
  （`total/stretch[4]/shrink/depth/goal/max_depth/best/best_cost/best_size`，
  page.rs:40–58）；`\vsize/\maxdepth` 在本页首盒到达时定格（`freeze()` page.rs:610–621），
  **逐页生效**（`start_new_page` page.rs:634–648）；`\topskip` 首盒胶水（page.rs:153–218）；
  badness/成本判据（`badness_now` page.rs:339–352、`try_break` page.rs:356–384，
  `c<=best_cost` 平局取后者、`pi<=-10000` 或 awful→FireUp）。
- **box255 + 例程路由**：`output_defined`（sink.rs:977–984，置 false 时清
  `pending_pages`）、`output_routine_begin`（sink.rs:1587–1590，组码 8）、
  `output_pending/take_output_pending/output_pending_count`（sink.rs:986–1000）、
  `\box255` 特例走 `box_register(255)` → `pending_pages.pop_front()`（sink.rs:1021–1023，
  队列而非单槽；空队列回退"空 hbox 节点" sink.rs:1028–1031）。
- **`\shipout`**：`shipout_next` 置位（sink.rs:911）→ 盒组认领（sink.rs:530–535）→
  `package_box` + `trace_shipout` + `shipped.push`（sink.rs:1100–1107）。
- **收尾交错**：`finish()` 里 `close_paragraph → run_pending_output →
  eject_one_page↔run_pending_output 循环 → 残留列表清理`（typesetter.rs:399–441，
  TRIP `\output{\unvbox255\end\rb}` 崩溃根因的修复现场）。
- **marks**：事件时同步 first/bot（sink.rs:1212–1221）、断页轮转 `rotate_marks`
  （mod.rs:790–797）、查询（sink.rs:1248–1256）。
- **寄存器面**：`boxes: Rc<Vec<Option<BoxNode>>>` 32768 槽（mod.rs:553–557）、
  组级保存/回滚 `box_saves`（mod.rs:561、1019–1028）、`\global\setbox` 不回滚
  （mod.rs:600–601）。
- **`\tracingoutput`**（params.misc[27]）→ `trace_shipout` 全树转录（paging.rs:6–16）；
  `\tracingpages`（params.misc[59]）→ 断点追踪行（page.rs:369–434）。
- **测试**：输出例程侧 9 个（tests.rs:670/697/711/718/739/748/771/790/1093）+ marks 4 个
  （tests.rs:1649–1705）。**但没有一条非平凡例程体测试**——三个测试例程体分别是
  `\shipout\box255`、`{}`、`\shipout\vbox{\hbox{Header}\box255}`，没有
  `\unvbox255`/`\pagegoal`/`\ifvoid255`/`\lastbox` 类。

### 3.2 逐项对齐表（latex.ltx 需求 → 引擎现状）

| 需求（latex.ltx 用点） | 现状 | 证据 |
|---|---|---|
| `\box\@cclv`（×3，`\@makecol` 第一步） | ✅ 队列 pop | sink.rs:1021–1023 |
| `\unvbox\@cclv`（×2，`\@doclearpage`/plain `\pagebody`） | ❌ **"Incompatible list can't be unboxed."** | sink.rs:1480–1497 经 `take_or_clone_box`(811–824) 只读 `boxes`，255 恒 void |
| `\vsplit\@cclv to\z@`（×1，`\@doclearpage`） | ❌ 不感知队列 + `\vsplit` 仅几何拆分 | sink.rs:364–384；node.rs:442–461 |
| `\ifvoid/\ifvbox\@cclv`、`\wd/\ht/\dp\@cclv` | ❌ 恒 void/0 | sink.rs:1365–1373、1500–1509 |
| `\outputpenalty`（×8，`\output` 第一个 token） | ❌ **断点惩罚未存**（`try_break` 局部变量） | page.rs:356–384 |
| `\deadcycles/\maxdeadcycles`（×4 + L20608） | ❌ 无死循环保护 | 全 crate 无 hit |
| `\insert\footins{…}` 体 | ❌ **token 体被 `toks_to_text` 压串（cs 全丢）** | sink.rs:1224–1230；sink_showbox.rs:11–17 |
| `\ifvoid\footins`/`\unvbox\footins`/`\skip\footins`/`\count\footins`/`\dimen\footins` | ❌ **插入寄存器完全没建模**（无 `\count/\dimen/\skip` 与插入类联动） | 全 crate 无 hit；`Node::Ins{class,text}` node.rs:215–219 |
| insert 参与断页成本 | ❌ 完全不参与（`Ins` 直接入页、不测量、无 `\insertpenalties`） | page.rs:293–298、513–518；头注 page.rs:15 |
| `\pagegoal/\pagetotal/\pagedepth/\pagestretch/\pageshrink` | ❌ 影子值是 PageBuilder 私有字段，无 sink API | page.rs:40–58；全 crate grep 无 hit（**但 latex.ltx 基本不用，见 §2.4**） |
| `\count0` → 页号（`[5.7]` 格式） | ❌ `trace_shipout` 硬编码 `[0.0.0.0.{ship_seq}]`；NodeBuilder 无 count 镜像 | paging.rs:11–12；mod.rs:543–545 |
| `\vsize` 例程中途改（L20817/20820/20823） | ✅ 逐页生效（页中途改到下页起效，与 tex.web 一致） | page.rs:610–648 |
| `\topmark/\firstmark/\botmark` | ✅（绑贡献流而非页内容——语义待核） | sink.rs:1212–1256 |
| `\splitfirstmarks/\splitbotmarks`（每页 24 次） | ⚠️ 接口在、恒空 | sink.rs:1257–1274 |
| `\vsplit` 完整语义（`\splittopskip/\splitmaxdepth`/split marks/可拆性） | ⚠️ 仅几何拆分 | sink.rs:364–384 |
| `\lastbox` | ⚠️ 只摘当前列表尾盒，拿不到 255 页内容 | sink.rs:1385–1402 |
| `\showboxbreadth/\showboxdepth` | ❌ 无界递归（对拍口径缺失） | sink_showbox.rs:36–65 |
| 例程在 token 边界中途执行 | ⚠️ layout 只在 `finish` 调 `run_pending_output`（typesetter.rs:400/420）；中途轮询在 ntex-core 侧 **待主线核**（TRIP 的 `\output{\unvbox255\end\rb}` 能跑通，间接说明已通） | typesetter.rs:322–323 注释 |
| 增量管线 | ⚠️ 观测到 `output_defined` 即禁用全部段缓存复用 | incremental.rs:369–371、772–774、492–493 |

---

## 4. 缺口地图（按「阻塞 sample2e 出 PDF」排序）

> 量级记号：**宏数** = 需要引擎侧新语义支撑才能跑通的 latex.ltx 用点数；
> **新原语** = 需注册/接线的原语；**刀数** = 参照 docs/latex-feasibility.md 战役格式的
> 单刀单位（一个最小复现 + 一次真 TeX 对拍）。

| # | 缺口 | 证据 | 量级 | 为何排在这 |
|---|---|---|---|---|
| G1 | **`\outputpenalty`**：FireUp 不记断点惩罚，`\output` 第一个判据就失真（8 个用点；**六档罚分协议** -\@M/-\@Mi/-\@Mii/-\@Miii/-\@Miv/-\@MM 全靠它分派，规格见 §2.2bis） | page.rs:356–384 | 1 原语 + PageBuilder 1 字段 + 1 sink API；**1 刀** | 例程第一 token；档位差 ±1 就换路（-10000→正常臂，-10001→`\@doclearpage`），**不能模糊化** |
| G2 | **box255 寄存器化**：255 不在 `boxes` 寄存器文件，`\unvbox\@cclv`×2/`\vsplit\@cclv`×1/`\ifvoid\@cclv`/`\wd\ht\dp\@cclv` 全废 | sink.rs:1021–1023 vs 1480–1497/1365–1373/1500–1509 | 1 处结构（255 进 `boxes` 或队列感知面）+ 5 个调用点；**1–2 刀** | `\@makecol` 只用 `\box\@cclv`（已通），但 `\end{document}` 的 `\clearpage`→`\@doclearpage` 必碰 `\vsplit\@cclv to\z@`；sample2e 最后一页必经 |
| G3 | **insert 建模**：`\insert` 体被有损压串 + `\count/\dimen/\skip` 三联缺失 + `\newinsert` 分配面（**56 个：52 个 float 池 `bx@A…bx@ZZ` + `footins`/`\@mpfootins`/`\@kludgeins`/`\reserved@a`**）；且 `\@currbox`/`\@marbox` 是**指向 insert 的宏**（`\count\@currbox` 须经宏展开寻址） | sink.rs:1224–1230；node.rs:215–219 | 1 个结构改造（`Node::Ins` 存 token 体/BoxNode）+ 3 类寄存器联动 + `\newinsert` 分配器；**2–3 刀** | sample2e 有脚注：`\@footnotetext`(17950)→`\insert\footins`（体内 `\splittopskip\footnotesep\splitmaxdepth\dp\strutbox\floatingpenalty\@MM`，17953–54）→`\@outputbox@appendfootnotes`(21025) 的 `\ifvoid\footins`/`\unvbox\footins`/`\skip\footins` 必经 |
| G4 | **`\deadcycles`/`\maxdeadcycles`**：无死循环保护；latex.ltx 6 个用点（20608/20831/15495/15560/9784/20245） | 全 crate 无 hit | 1 原语 + fire_up 处 1 判据；**0.5 刀**（可与 G1 并刀） | 带来输出例程时代必需的安全网（例程不出页 → `pending_pages` 无限堆积，现在不报错） |
| G5 | **页号链**：`\count0..9` 快照进 `trace_shipout`/DVI 页标签（`[5.7]` 格式实测） | paging.rs:11–12 | count 镜像 1 个 + 格式化 1 处；**0.5–1 刀** | `\@outputpage` 的 `\ifodd\count\z@` + `\stepcounter{page}` 是页眉选择与页码来源；对拍真 TeX 的 log/DVI 都靠它 |
| G6 | **`\vsplit` 完整语义**：`\splittopskip/\splitmaxdepth`/split marks/可拆性判据；**两个用户**——`\@doclearpage`(20884) 与 ltmarks 的 `\mark_update_structure_from_material:nn`(18222 用 `\tex_vsplit:D`) | sink.rs:364–384；node.rs:442–461 | 2 参数 + 判据改写；**1–2 刀** | `\@doclearpage` 的 `\vsplit\@cclv to\z@` 只为"取走页首 discardables"，几何拆分可能侥幸；但浮动体页（非 sample2e）必炸；ltmarks 是第二用户 |
| G7 | **split marks 真实化**：断页时把 marks 从页内容切出；`\splitfirstmarks/\splitbotmarks` 返回真值 | sink.rs:1257–1274；mod.rs:790–797 | marks 三件套迁移；**1 刀** | 每页 24 次调用，只要**不炸**就不阻塞 sample2e（plain 页样式不读 `\leftmark`）；语义正确性后置 |
| G8 | **`\pagegoal/\pagetotal/\pagedepth/\pagestretch/\pageshrink` 暴露** | page.rs:40–58 私有 | 5 个 sink 查询 API；**0.5 刀** | **伪需求**：latex.ltx 只用 `\pageshrink`×1（`\@make@specialcolbox`，`\enlargethispage` 路径）与 `\pagetotal`×1；sample2e 不碰。为宏包（float.sty 等）预留 |
| G9 | **`\showboxbreadth/\showboxdepth`**：`showbox` 无界递归 | sink_showbox.rs:36–65 | 2 参数 + 递归限深；**0.5–1 刀** | 不阻塞 PDF，但阻塞「与真 TeX 逐行对拍盒树」这一验收口径本身 |
| G10 | **（结构性 flag，非阻塞）**：`output_defined` 恒真（latex.ltx L20798 无条件定义 `\output`）→ 增量管线对**一切 LaTeX 文档**禁用段缓存复用，M5 增量收益结构性归零 | incremental.rs:369–371、492–493 | 待设计（如：例程体未变 + 段内无断页 → 局部放行） | 出 PDF 不卡；但 LaTeX 化后"增量 1.1x/4.3x"叙事需要重估 |

**不在缺口里的（明确排除，免得白做）**：`\insertpenalties`/`\holdinginserts`
（latex.ltx **零使用**——insert 断页成本与 `\holdinginserts` 可后置）、
`\pagegoal/\pagedepth/\pagestretch`（零使用）、经典 `\mark/\topmarks/\botmarks`
原语（零使用，ltmarks 全走 `\newmarks`+`\tex_marks:D`）、`\ifhbox`（零使用）；
注意 **`\floatingpenalty`/`\splittopskip`/`\splitmaxdepth` 不能排除**——它们在
`\@footnotetext`(17953–17954) 的 insert 体内，属 sample2e 路径。

**宏层既有关口（非本轮新增，但输出例程窗口承重最高）**：hooks/sockets
（246 次/3 页）、`\catcode` 组内批量改 + 组恢复（24 条/页）、`\aftergroup`
（61 次/3 页）、`\afterassignment`/`\currentgrouplevel`（ltshipout 包装）、
NFSS 在例程内再入（`\reset@font\normalsize`）。这些属于宏载入战战场，本报告
只标记「输出例程上下文须回归」。

---

## 5. 验收路径（逐刀切分）

> 每刀遵循战役格式：**一个最小复现 + 一条 tex.web 语义裁决 + 一次真 TeX 对拍**。
> 探针文档放 `/tmp/ors-work/`（勿入 fixtures/），对拍基准用 §6 方法现场生成。

**刀 1（G1+G4）：`\outputpenalty` + `\deadcycles`**
- 落点：`page.rs` `try_break`/`fire_up` 记录断点惩罚 → sink/引擎暴露 `\outputpenalty`；fire_up 处加 `\deadcycles>\maxdeadcycles` 检查（tex.web `fire_up` 的 dead cycles 死循环分支）。
- 复现：`\output{\typeout{p=\the\outputpenalty}\shipout\box255}`，分别用 `A\newpage B`（-10000）、`A\clearpage B`（-10001）、`\newpage\penalty-20000\relax`（\supereject 类）触发，对拍真 TeX 的 `p=` 值与走臂。
- 验收：**§2.2bis 罚分协议表逐行对拍**；例程恒不出页时 `maxdeadcycles`（latex.ltx=100）处报 `Output loop` 类错误。

**刀 2（G2）：box255 寄存器化**
- 落点：255 进 `boxes` 寄存器文件（或 `take_or_clone_box`/`box_register_kind`/`box 尺寸查询`/`vsplit` 四点全部感知 `pending_pages`）。
- 复现：`\setbox0\box255 \unvbox0`、`\ifvoid255`、`\wd255`、`\setbox0\vsplit255 to 10pt`。
- 验收：与真 TeX 对拍 `\showbox`/`\ifvoid` 结果；`\@doclearpage` 的 `\vsplit\@cclv to\z@` 不再报 `Incompatible list`。

**刀 3（G3a）：insert 结构化——token 体保留**
- 落点：`Node::Ins` 携带真实 token 体（或独立 insert 页内段），废除 `toks_to_text` 有损压缩。
- 复现：`\insert\footins{\splittopskip\footnotesep\splitmaxdepth\dp\strutbox\floatingpenalty\@MM\hbox{FN}}` → `\ifvoid\footins\else\unvbox\footins\fi` 往返，`\showbox` 对拍。
- 验收：脚注文本出现在 shipout 盒树中（与真 TeX `[1]` 页盒树同形）；insert 体内三个 split 参数赋值不炸（**这是 sample2e 脚注路径的隐藏需求**）。

**刀 4（G3b）：`\newinsert` 分配 + `\count/\dimen/\skip` 三联**
- 落点：`\newinsert` 分配器（盒+同号 count/dimen/skip），插寄存器进 `SideEffects` 快照（M5 增量一致性）；`\count\@currbox` 这类**经宏间接寻址**的用点须走 csname 展开（`\@currbox`=`\def\@currbox{\bx@A}`）。
- 复现：`\newinsert\foo \count\foo=3 \skip\foo=5pt plus1sp \insert\foo{}` + latex.ltx L20624 批量 **52 连分配**（`\bx@A…\bx@ZZ`，验寄存器空间与 32768 槽不冲突、`\count\bx@X` 位图读写）。
- 验收：`\ifvoid\footins`/`\skip\footins`/`\ht\footins`/`\count\footins` 在 `\@outputbox@appendfootnotes` 与 `\@reinserts`(21146) 全链不炸。

**刀 5（G5）：页号链**
- 落点：NodeBuilder 暴露 count 镜像（或引擎在 shipout 边界供值），`trace_shipout` 与 DVI 页标签改 `[count0[.count1…]]` 尾零截断。
- 复现：`\setcounter{page}{5}\count1=7` 两页文档，对拍真 TeX `[5.7]`/`[6.7]`。
- 验收：`\tracingoutput` 标题行与真 TeX 逐字符一致（`Completed box being shipped out [1]`）。

**刀 6（G9）：`\showboxbreadth/\showboxdepth` + 盒树对拍口径**
- 验收：`\tracingoutput=1` 下 sample2e 单页盒树与 TinyTeX log 同形（深度/宽度截断一致）——这是后续所有刀的**验收仪器**，越早越好。

**刀 7（G6）：`\vsplit` 完整语义**（`\splittopskip`/`\splitmaxdepth`/split marks/可拆性；两个用户：`\@doclearpage` 与 ltmarks 18222）
**刀 8（G7）：split marks 真实化**（`\@expl@@@mark@update@singlecol@structures@@`(18582) 全链不炸且语义对拍；ltmarks 侧 `\newmarks` 寄存器面）
**刀 9（G8）：页影子值五原语暴露**（`\pagegoal/\pagetotal/\pagedepth/\pagestretch/\pageshrink`；为 float.sty/宏包备料，sample2e 不依赖——latex.ltx 自身只用 `\pageshrink`×1 + `\pagetotal`×1）
**刀 10：端到端**
- `article` 单页 `\tracingoutput` 盒树与 TinyTeX 逐行对拍 → sample2e 3 页 DVI（页数 3 / 字节 7576 / 页号 [1][2][3]）→ PDF。

> 依赖关系：刀 6 是仪器，应尽早；刀 1→2→3→4 是 sample2e 硬路径
> （`\output` 判据 → `\@makecol` → 脚注 → `\clearpage`）；刀 5 可并行；
> 刀 7/8/9 在端到端之后补语义正确性。

---

## 6. 复现步骤（ground truth 资产重建，约 2 分钟）

```bash
export PATH=$HOME/.local/bin:$PATH
mkdir -p /tmp/ors-work && cd /tmp/ors-work
# 1) sample2e 带 trace（3 页输出例程窗口）
printf '%%&latex\n\\tracingmacros=1\\tracingoutput=1\n\\input sample2e.tex\n' > s2.tex
latex -interaction=batchmode -jobname=s2 s2.tex
grep -n 'Completed box being shipped out' s2.log        # 3 个 shipout 行号
#   窗口起点 = 每个 shipout 行号之前最后一个 '^\\@makecol ->'
# 2) 页号格式
printf '\\documentclass{article}\\begin{document}\\setcounter{page}{5}\\count1=7\nA.\\newpage B.\\end{document}\n' > pn.tex
latex -interaction=batchmode -jobname=pn pn.tex; grep -o '\[[0-9.]*\]' pn.log   # [5.7] [6.7]
# 3) 极小链序（\newpage 触发一次输出例程）
printf '\\documentclass{article}\\begin{document}\\tracingmacros=1\\tracingoutput=1\nHi.\\par\\newpage\\end{document}\n' > min2.tex
latex -interaction=batchmode -jobname=min2 min2.tex
# 4) 输出例程窗口原语面
awk 'NR>=<起> && NR<=<止>' s2.log > win.txt
grep -o 'tex_[a-zA-Z]*:D' win.txt | sort | uniq -c | sort -rn     # expl3 原语面
grep -o '^[\\]@[a-zA-Z@]*' win.txt | sort -u                      # \@ 系宏面
```

latex.ltx 侧：`/tmp/latexsurvey/tex/latex/base/latex.ltx`（2026-06-01，22838 行，
获取方法见 docs/latex-feasibility.md §1.2）。

---

## 7. 边界与诚实声明

1. **ntex-core 未读**。`\output` 例程在文档中途的 token 边界执行（`Expander::run_pending_output`
   的轮询点）、`\outputpenalty`/`\deadcycles` 原语的注册位置、expl3 原语别名链
   （`\tex_shipout:D` 等）能否逃过 `\shipout` 重定义，均属 ntex-core，本报告只给
   「待主线核」清单，不给断言。间接证据：TRIP 的 `trip.tex` 带非平凡 `\output`
   例程能跑通 → 中途执行大概率已通。
2. **本报告的"支持/不支持"判定全部来自 ntex-layout 的可见面**（sink API、字段、
   grep 零命中）。`\pagegoal` 等的"缺"可能由引擎侧另一条通道补足——刀 9 开工前
   须先在引擎侧确认一次。
3. **样本面**：量化数据来自 sample2e（3 页，无浮动体、无双栏、无 `\enlargethispage`、
   页样式 plain）。浮动体（`\@tryfcolumn`/`\@deferlist`/`\@scolelt`，§2.2 支线表）、
   双栏（`\@outputdblcol` 21809/`\@startdblcolumn` 21342）、边注（`\@addmarginpar` 21614）
   三条支线**未被 sample2e 实测覆盖**，其引擎需求按 latex.ltx 文本推断，量级另计
   （预计再加 3–5 刀，含 G6 的完整 `\vsplit`）。
4. **行号可信度分级**：承重宏（`\@opcol` 20919、`\@makecol` 20930、
   `\@outputbox@append` 21015、`\@outputpage` 21170、`\@tryfcolumn` 21354、
   `\@footnotetext` 17950、`\c@page` 15083、`\@cclv` 325 等）与全部原语计数
   （`\outputpenalty`×8、`\insertpenalties`×0、`\pagegoal`×0…）为 grep 实测；
   「闭包全量 ~155–165 个名字」为清点估计。latex.ltx 的 l.NNN 行号随版本漂移
   （继承教训：勿把旧行号当常量），引用时以宏名为准。
5. **频率≠依赖**：`\par`×303、`\let`×220 这类高频项不构成缺口（引擎已支持）；
   缺口判定以「latex.ltx 用点存在 且 引擎面零命中」为准（§3.2 末列）。
   反向也成立：`\ifhbox` 在 latex.ltx 零使用（ltshipout/ltmarks 走
   `\box_if_horizontal:NTF`），不构成第一优先。
6. 探针/转录污染教训（继承）：`\write16` 探针会改错误计数；本轮对拍一律用
   `\typeout`（转录）+ log 窗口切片，不碰 fixtures/。

---

## 5.bis 刀 1 实测记录（2026-09-06，✅ 完成）

**改动面**：`page.rs`（`best_penalty`/`fired_penalty` 断点惩罚记录）+
`TokenSink` 三个新方法（`output_break_penalty`/`take_page_shipped`/
`default_output_routine`）+ `maybe_inject_output` 点火侧语义（§2.2bis 的
引擎侧承担部分：只负责把断点惩罚交予例程 + 死循环保护，六档分派全在宏层）。
原语注册早已在（misc 62/25/43），无需新增原语。

### 5.bis.1 真 TeX 对拍（TinyTeX，tex 3.141592653 / TeX Live 2026，/tmp/ors-work-d1）

对拍探针（plain 格式，与 NTex 同源）：

```tex
\output={\showthe\outputpenalty\shipout\box255}
A\par\vfil\penalty-10000 B\par\end
```

| 触发（§2.2bis 档位） | 真 TeX log | NTex 转录 |
|---|---|---|
| `\par\vfil\penalty-10000`（-\@M / \newpage） | `> -10000.` | `-10000` ✅ |
| `\par\vbox{}\penalty-10001`（-\@Mi / \clearpage） | `> -10001.` | `-10001` ✅ |
| `\par\penalty-10002`（-\@Mii / 行内 float） | `> -10002.` | `-10002` ✅ |
| `\par\penalty-10003`（-\@Miii / 垂直 float） | `> -10003.` | `-10003` ✅ |
| `\par\penalty-10004`（-\@Miv / \end@float 强制页） | `> -10004.` | `-10004` ✅ |
| `\par\penalty-20000`（-\@MM / \supereject） | `> -20000.` | `-20000` ✅ |
| 页满在**胶水**处自然断页（多页） | `> 10000.`（重复） | `10000`（第 2 页起）✅ |
| `\end` 冲页（eject 惩罚 -'10000000000） | `> -1073741824.` | `-1073741824` ✅ |
| `\deadcycles`（每页例程内读，ship 后清零） | `> 1.`（每页） | `1`（每页）✅ |
| `\maxdeadcycles=0` 死循环分支 | `! Output loop---0 consecutive dead cycles.` + help3 三行，照常 ship **2 页** | 同文本 + 2 页 ✅ |

LaTeX 层对拍（`\documentclass{article}` + 勘察报告刀 1 原探针
`\output={\typeout{p=\the\outputpenalty}\shipout\box255}`，
`A\newpage B\clearpage C\newpage D\end{document}`）：真 TeX log 给
`p=-10000`×4、`p=-10001`×2 —— 印证 §2.2bis：`\newpage` 走 -10000 正常臂、
`\clearpage` 的 `\penalty-\@Mi` 走 `\@doclearpage` 臂。

**勘误（简报 → tex.web 实证）**：简报称"例程结束后重置"。tex.web `fire_up`
**无此重置**——`\outputpenalty` 是 `geq_word_define`（全局）且保持到下一次
fire_up；被重置的是**断点节点的 penalty**（`penalty(best_page_break):=inf_penalty`，
NTex 以"新空页丢弃触发节点"等效实现）。NTex 按 tex.web 实现。

### 5.bis.2 已知偏差（待主线核 / 后续刀）

1. **`\write`（`\typeout`）在输出例程内挂死**（既有缺陷，非本刀引入）：`\write`
   的 whatsit 节点在例程内落入主列表 → 页面构建器材料永不清空 → `\end` 冲页
   循环无限（`layout-watchdog` 200 万节点实测复现，真实 TeX 同探针正常）。
   因此 NTex 侧对拍探针改用 `\showthe`（转录专用、不进节点流）。**建议列为
   独立刀**（`\@outputpage` 的页眉页脚路径离不开例程内 `\write`）。
2. **极小 `\vsize` + 页满胶水断页场景下，首页例程读到 `\outputpenalty`=0**
   （应为 10000；第 2 页起正确）。疑似首次例程注入早于 `fired_penalty` 写定
   的时序差，非六档协议路径（页未满时惩罚先触发），已锁在单测注释里。
3. **多页排队时 `\outputpenalty` 取最后一次 fire_up 的值**：NTex 的例程是
   延迟注入（token 边界），一帧内连断多页时队列各页共享最后一次记录；tex.web
   逐页 fire_up 逐页写。sample2e 单页断点场景不受影响，`\@specialoutput`
   五档分派不受影响。
4. **TRIP 基线**：`cargo run -p ntex-trip -- --driver ntex --test trip` 在
   HEAD 9dec0b7（本刀前）即失败于 pass2 `组未闭合（缺少 }）：groups=[SemiSimple,
   MathLeft, MathLeft, Align]`（载入战主线 9dec0b7 既有）；本刀改动下输出与
   基线**逐字节一致**（diff 为空）。
