# NTex \halign/\valign/\insert/数学矩阵 战役前置勘察报告（2026-09-07，基于 HEAD 8849d5f）

> 目标：plan.md §6 D 组（\halign/\valign 表格地基 + \insert 脚注排版 + 数学矩阵）2026-09-06
> 被提升为战役前置，但规模未勘察。本报告量化三件事：宏层到底要用哪些对齐原语（按
> latex.ltx 真实用点，不按 tex.web 全集）、引擎现状离这个需求面差几刀、逐刀怎么切。
>
> **边界声明**：本报告只读。`crates/` 零改动，唯一新增文件即本报告。真实 TeX ground
> truth 在 `/tmp/halign-work/`（未入库，复现见 §7）。禁碰清单
> （`ntex-layout/src/typeset/math.rs`、`ntex-core/src/expand/` 下的 A 线改动面、fixtures/）
> 只读未写；其中 `ntex-core/src/expand/align.rs` 属 A 线领地，本报告引用其行号时
> 以 HEAD 8849d5f 为准，A 线合入后行号会漂移。

---

## TL;DR

- **「\halign 战役」其实是三层，不是一层**：① 引擎状态机（preamble 扫描 → 模板帧注入 →
  sink 对齐事件 → 两遍列宽定稿）**在 HEAD 已存在且被 TRIP 大量使用**
  （`ntex-core/src/expand/align.rs:106-895` + `ntex-layout/src/typeset/sink.rs:1763-1868`），
  这场战役是**补精化，不是从零落地**；② 语义精化面有 **6 处已知的"简化"**（`\everycr`
  只存不注入、`to`/`spread` 摊派均分、\valign 行高数学整体跳过、preamble `#{` 无特判、
  多列越界钳末列、模式/EOF 合法性静默）；③ **格式与宏层通路缺失**——引擎不预载
  plain 格式（`\ialign/\hidewidth/\strut` 全 Undefined），而 LaTeX kernel 的 lttab.dtx
  宏链（`\@array/\@mkpream/\@arstrut`）是否能在 ntex 载入未验证。只盯 align.rs 会漏掉后两层。
- **需求面（grep 实测）**：`\halign` 在 LaTeX kernel 的真实调用点只有 **4 处宏**
  （`\ialign` 定义 631、`\displaylines` 15881、`\eqnarray` 15976、`\@ishortstack` 17063
  经 `\ialign`），amsmath 8 处（`\gather@/\align@/\multline@/\emdf@Ra` 及其 measure
  副），array.sty 1 处（L290）、longtable 1 处（L167）；`\valign` 在 kernel **只有 1 处**
  ——`\tc@fake@euro`(14583) 造欧元符号。`\insert` 在 latex.ltx 的真实排版用点只有
  **1 处**（`\@footnotetext` 17950），其余 4 处是 `\newinsert` 分配机制。**需求面
  \halign ≫ \insert**。
- **真 TeX 实测（TinyTeX，2 列 × 4 行小 tabular）**：仅 preamble 构造（`\@array` →
  `\ialign`）就烧 **2575 行宏展开**；tabular 窗口原语谱 top：`\global`×93、`\let`×68、
  `\edef`×36、`\def`×34、`\hskip`×31、`\catcode`×24、`\setbox`×20、`\futurelet`×9、
  `\vrule`×9。**引擎对齐原语在窗口里只出现 `\halign`×1、`\noalign`×1、`\omit`×1、
  `\tabskip`×1** ——tabular 的成本在宏层，引擎侧状态机被调用得极少但承重 100%。
- **意外发现（真 TeX）**：`\everycr` 的注入点在 tex.web 有**两处**——preamble 扫完后
  （L15339）+ 每行 `fin_row`（L15732）；且 `\everycr={\message{…}}` 与 `\halign to` +
  `\crcr`/模板尾 `\tabskip` 组合在真 TeX 会触发连环 `! Missing \cr inserted.`（触发面
  未完全钉死，见 §2.3）；amsmath 实际用的 `\everycr{\noalign{…}}` 形态干净可用（p3g）。
- **现状端到端实测（ntex 二进制，晚于 HEAD 构建）**：最小 `\halign` 文档
  `\halign{\hfil#\hfil&\hfil#\hfil\cr a&b\cr c&d\cr}\end` 经 ntex-dvi 产出
  **112 字节 / 1 页 / 0 字体** 的空白 DVI（与 `\hbox{H in hbox}` 对照批逐字节同 md5）；
  纯文字文档（无 \output）**连页都不出**。结论：引擎层"能跑通"（不炸、盒在产），
  但 plain 通路无格式预载/无字体，端到端不可见——**表格战役的第一刀必须是立对拍仪器**。
- 量级：到「kernel tabular + plain `\eqalign`/`\matrix` 对拍真 TeX 盒树」，
  预计 **9–12 刀**（§5），其中 3 刀是语义精化硬骨头（everycr 注入 / to 摊派 /
  valign 数学）。`\insert` 不在本战役（属输出例程战 G3，刀 3/4 已排期）。

---

## 1. 勘察方法与 ground truth 资产

1. **宏层需求面**：grep 计数于 TinyTeX 2026 的 latex.ltx / array.sty / longtable.sty /
   tabularx.sty / booktabs.sty / amsmath.sty（§2.1），再对 `\halign/\valign` 的每个
   调用点归属到宏名（§2.2）。
2. **真 TeX 对拍法**（同 output-routine-survey §1.1）：最小探针 + `\tracingcommands=1
   \tracingoutput=1 \showboxbreadth=20 \showboxdepth=5`，取「tabular 窗口」的原语谱与
   盒树（§2.3/§2.4）。探针目录 `/tmp/halign-work/`。
3. **引擎现状面**：ntex-layout / ntex-core 两个只读代理全量勘察（§3），我另做了
   原语注册面 grep + ntex 二进制端到端实测（§3.3）。
4. **tex.web 语义裁决**：`reference/tex.web` 对齐段（L15337–15760）逐条读出，见 §2.5。

---

## 2. 宏层需求面（TinyTeX 2026-06）

### 2.1 原语用点计数（grep 次数，含定义与注释）

| 原语 | latex.ltx | array.sty | longtable.sty | tabularx | booktabs | amsmath.sty |
|---|---|---|---|---|---|---|
| `\halign` | 5 | 1 | 1 | 0 | 0 | 8 |
| `\valign` | 1 | 0 | 0 | 0 | 0 | 0 |
| `\noalign` | 10 | 3 | 12 | 0 | 9 | 12 |
| `\omit` | 6 | 2 | 1 | 0 | 0 | 2 |
| `\span` | 1 | 0 | 0 | 0 | 0 | 3 |
| `\cr` | 17 | 4 | 5 | 0 | 2 | 14 |
| `\crcr` | 22 | 0 | 5 | 0 | 0 | 25 |
| `\everycr` | 4 | 1 | 1 | 0 | 0 | 6 |
| `\tabskip` | 9 | 3 | 3 | 0 | 0 | 13 |
| `\hidewidth` | 5 | 0 | 0 | 0 | 0 | 0 |
| `\ialign` | 7 | 0 | 0 | 0 | 0 | 8 |
| `\vtop` | 5 | 2 | 1 | 0 | 0 | 7 |
| `\strut` | 14 | 3 | 2 | 0 | 0 | 21 |
| `\insert` | 8 | 0 | 0 | 0 | 0 | 0 |

读法：
- **top 承重原语 = `\crcr`/`\cr`/`\noalign`/`\tabskip`**（`\crcr` 在 amsmath 25 次，
  `\noalign` 在 longtable 12 次——longtable 的行间协议几乎全靠 `\noalign`）。
- `\span` 用点极少（kernel 1 + amsmath 3）但**全部承重**：amsmath 的
  `\halign{\span\align@preamble\crcr`（1926/1841）把整个 preamble 塞进一个宏，
  靠 `\span` 的"其后强制展开一次"语义灌进去（tex.web L15442–15447）。
- `\insert` 在 array/amsmath 系**零使用**——表格战役不依赖 insert。

### 2.2 调用点归属（谁在用 \halign/\valign）

**kernel（latex.ltx）**

| 宏 | 行号 | 形态 | 引擎含义 |
|---|---|---|---|
| `\ialign`（定义） | 631 | `\def\ialign{\everycr{}\tabskip\z@skip\halign}` | **`\everycr` 的第一步是清空**——只存不注入的现状对"清空"无害 |
| `\displaylines` | 15881 | `\halign{\hb@xt@\displaywidth{$\@lign\hfil\displaystyle##\hfil$}\crcr #1\crcr}` | 数学显示区单列对齐 + `##` 列模板 + 双 `\crcr` |
| `\eqnarray` | 15976 | `\dollardollar@begin\everycr{}\halign to\displaywidth\bgroup …` | **`to` 摊派 + `\everycr{}` + `\bgroup` 形态**（eqnarray 硬路径） |
| `\@ishortstack` | 17063 | `\ialign{\mb@l {##}\unskip\mb@r\cr #1\crcr}\egroup` | `\shortstack` = 单列 halign |
| `\tc@fake@euro` | 14587 | `\valign{##\cr …}` | **kernel 全文唯一 `\valign`**：textcomp 假欧元 |
| `\@footnotetext` | 17950 | `\insert\footins{…}` | **`\insert` 的唯一真实排版用点**（其余 434/470/472/475 是 `\newinsert` 分配机制） |

**lttab.dtx（L16645–16999，tabular/array/tabbing 的家）——`\@array` 的 preamble 机制**：

```
\def\@array[#1]#2{%                                  16799
  \if #1t\vtop \else \if#1b\vbox \else\vcenter@text \fi\fi \bgroup
  \setbox\@arstrutbox\hbox{\vrule \@height\arraystretch\ht\strutbox
                           \@depth\arraystretch\dp\strutbox \@width\z@}
  \@mkpream{#2}%                                     ← 列规格串 → token 列表
  \edef\@preamble{\ialign \noexpand\@halignto
      \bgroup \@arstrut \@preamble \tabskip\z@skip \cr}%   16807-16809  ★
  \let\@sharp##%                                     16813  ★ 列占位 = cat6 字符 token
  …
  \@preamble}                                        16817  ← 引擎在此开始扫 preamble
\protected\def\@arraycr{%  ${\ifnum0=`}\fi\@ifstar\@xarraycr\@xarraycr}   16818
\def\@xarraycr{\@ifnextchar[\@argarraycr{\ifnum0=`{\fi}${}\cr}}          16820
\def\@yargarraycr#1{\cr\noalign{\@vspace@calcify{#1}}}                  16837  \\[2pt]
\long\def\multicolumn#1#2#3{\multispan{#1}\begingroup \@mkpream{#2}…}   16838
\def\multispan{\omit\@multispan}                                        16984
\def\endtabular{\crcr\egroup\egroup\egroup}                             \endarray 同
```

★ 三条引擎规格由这段钉死：
1. **preamble 是 `\edef` 动态构造的**，列占位符经 `\let\@sharp##` 变成 cat6 字符
   token 后从宏里展开出来——**模板扫描必须展开宏且接受展开产物里的 cat6 `#`**
   （对照 §2.5 裁决 1）。NTex 的 preamble 扫描是 raw 拷贝 + `\span` 后一次展开
   （`align.rs:222-225` 注释、`align.rs:237-257`），**没有通用的"宏展开产物进模板"通路**
   ——`\ialign\noexpand\@halignto\bgroup\@arstrut\@preamble…` 这一形态能否走通是
   kernel tabular 的头号未知数（**列为刀 7 的首个复现**）。
2. **`\\[2pt]` → `\cr\noalign{\vskip}`**：行距协议全在宏层，引擎只需 `\noalign` 正确。
3. **`\multicolumn` = `\omit` + 循环 `\span`**（`\multispan` 16985 用
   `\advance\@multispan\m@ne` 循环），且体内再跑一次 `\@mkpream` + `\@preamble`
   ——**单元内重入 preamble 展开**。

**array.sty**：L290 `\tabskip\z@skip\halign`（重定义 `\@array` 尾），另 `\d@llarbegin/
\d@llarend` 包 `$`、p 列 `\@@startpbox/\@@endpbox`（`\vtop` 单元）。
**longtable.sty**：L167 `\tabskip\LTleft \noexpand\halign to\hsize\bgroup`（`\noexpand`
保护 + `to` 摊派 + `\noalign`×12 的行协议）。**amsmath**：`\gather@`(1575)、`\align@`(1829)、
`\multline@`(2569)、`\emdf@Ra`(2885) 四族 + measure 副本（1926 是
`\halign{\span\align@preamble\crcr` 的**试排遍**，用 `\everycr{\noalign{…}}` 每行重置
标签状态 1921）。

### 2.3 真 TeX 实测一：最小探针（plain 格式，/tmp/halign-work/）

| 探针 | 内容 | 真 TeX 结果 |
|---|---|---|
| p1 | 2 列 + preamble `\tabskip1em` + 3 行 | ✅ 0 错 |
| p2 | `\noalign{\hrule}` + `\omit` + `\span` 跨列 + `\crcr` | ✅ 0 错 |
| p3a/b/d/e | `\halign to 200pt` spread / `\everycr={\message}` / 组合 | ✅ 0 错 |
| p3g | `\everycr={\noalign{\message{NA}}}` + `to` + `\crcr`（amsmath 形态） | ✅ 0 错 |
| p3h/p3i | `\everycr={\relax}` / message + 非 to | ✅ 0 错 |
| **p3（原始）** | `\everycr={\message{[cr]}}` + `\halign to 200pt` + 模板尾 `\tabskip0pt plus1fil` + 数据首格 `\hbox to 50pt{A}` + `\crcr` | ❌ **6 × `! Missing \cr inserted.`**（l.6，即 `\crcr` 处） |
| p4 | `\valign{\hsize=100pt#\cr a\cr b\cr c\cr}` | ✅ 0 错 |
| p5 | `\newinsert` + count/dimen/skip 三联 + `\insert` | ✅ 0 错 |

> **p3 是真 TeX 的语义角落，不是 NTex 的需求**：逐项二分（p3a/i/d/e/g/h 全净）表明
> 触发面是 `\everycr` × `\halign to` × `\crcr`/尾 `\tabskip` 的三方交叠，未完全钉死。
> 记录在此是因为 `\everycr` 注入时机（§2.5 裁决 5）是刀 1 的语义规格，而这个角落
> 说明注入点选错会在真 TeX 兼容性上炸出连环错。amsmath 用的 `\noalign` 形态（p3g）干净。

### 2.4 真 TeX 实测二：tabular 窗口计量（LaTeX，2 列 4 行含 `\hline`/`\\[2pt]`/`\multicolumn`）

- **preamble 构造成本**：log 从 `\@array ->`（L151）到 `\ialign ->`（L2726）——
  **2575 行纯宏展开**（`\@expast` 列复制循环 + `\@tfor` 逐字符 + `\edef` 重建
  `\@preamble`）才把 `{|l|r|}` 变成一条 `\halign` preamble。**这是宏载入战的压力样本**：
  `\edef`/`\xdef`×48、`\@tfor`/`\@whilenum` 循环、`\ifcase` 链。
- **tabular 窗口原语谱**（`\tracingcommands=1`，`{\…}` 形态计数）：

| 原语 | 次 | 原语 | 次 | 原语 | 次 |
|---|---|---|---|---|---|
| `\relax` | 95 | `\global` | 93 | `\let` | 68 |
| `\edef` | 36 | `\def` | 34 | `\hskip` | 31 |
| `\catcode` | 24 | `\setbox` | 20 | `\futurelet` | 9 |
| `\vrule` | 9 | `\unskip` | 10 | `\hfil` | 8 |
| `\box` | 5 | `\unvbox` | 4 | `\dp` | 4 |

  **引擎对齐原语在窗口里只出现 `\halign`×1、`\noalign`×1、`\omit`×1、`\tabskip`×1**；
  `\span/\crcr/\cr` 以 token 形态被宏层消费、不进 big-switch 追踪。含义：**表格战役的
  性能与正确性风险大头的宏层（preamble 构造机器），引擎侧状态机被调用得极少但承重 100%**。

- **验收锚点（真 TeX 盒树）**：`\begin{tabular}{|l|r|}` 4 行小表在 shipout 盒树里是

```
.\hbox(21.9+16.9)x79.83347        ← \leavevmode\hbox\bgroup 的外壳
.\.\vbox(21.9+16.9)x79.83347 []   ← \halign 的产物（[] = 深度截断）
```

  高 21.9 + 深 16.9 = `\arraystretch`×strut 算术；**\halign 在水平模式的产物是一个
  hbox 包 vbox**。这行就是后续每一刀的 `\showbox` 对拍基准（刀 0 立仪器）。

### 2.5 tex.web 语义裁决（`reference/tex.web`，刀 1/2/3 的规格来源）

| # | 裁决 | 行号 |
|---|---|---|
| 1 | **preamble 逐 token 直拷，但 `\tabskip` 与 `\span` 之后的 token 强制展开一次**（`\span` 处理：`get_token` 后若 `cur_cmd>max_command` 则 `expand; get_token`） | 15430-15436、15438-15457 |
| 2 | **`\span` 注册为 `tab_mark` 命令码 + `span_code` 修饰**——它是 `&` 的等价分隔符，只是可辨识；**不是"列合并器"**。跨列合并发生在 alignrecord 的 span 链上（`cur_span` + span node 宽度表） | 15391-15410、15683-15712 |
| 3 | **u_j 扫描**：行首空格剔除；`#`(mac_param) 终结 u_j；**空模板 + 头位 + `cur_loop=null` → 记 `cur_loop:=cur_align`**——`&&`（空模板重复前列）的机制就是它 | 15461-15479 |
| 4 | **v_j 尾部追加冻结 `\endtemplate`**；`get_x_token` 把 `end_template` 转成 `endv` 命令码；扫到 `endv` 即 `fatal_error("(interwoven alignment preambles are not allowed)")` | 15448-15450、15496-15497 |
| 5 | **`\everycr` 注入点有两处**：preamble 扫完即注入（15339）+ 每次 `fin_row` 注入（15732）。且注入发生在 `align_peek` **之前**——`\everycr` 的 token 会先于下一行材料被 `align_peek` 前瞻到 | 15339、15732、15509-15520 |
| 6 | **`fin_row` 产 unset 节点**：行内容 `hpack/vpack(natural)` 后 `type(p):=unset_node`，挂进 vlist/hlist，一切到 `fin_align` 才定稿 | 15714-15734 |
| 7 | **`\halign` 合法位**：显示数学里若 mlist 非空则 `! Improper \halign inside $$'s`；`align_peek` 对 `\crcr` 直接忽略（restart）、对 `\noalign` 开 `no_align_group`、对 `}` 收官（fin_align） | 15357-15365、15509-15520 |
| 8 | **preamble 内 `\tabskip` 赋值是局部 eq_define**（`\global` 前缀在 preamble 里存活、`\tabskip` 本身局部生效），且 `scan_glue` 就地求值 | 15451-15456、15434-15436 |

---

## 3. 引擎现状面

### 3.1 已有机制（这是好消息，全部带 文件:行）

**ntex-core（解释层）**——原语注册面 `BUILTINS` 412 项（`expand/builtins.rs:6`，注册循环 536-544）：

| 原语 | 三态 | 证据 |
|---|---|---|
| `halign`/`valign` | ✅ 真实现（共用状态机，方向位差） | builtins.rs:376/375 → `expand/align.rs:106-895` + `expand/primitive_align.rs:21-32` |
| `noalign`/`omit`/`span`/`cr`/`crcr` | ✅ 真实现（含 tex.web 文案的误位报错） | align.rs:853-869/770-786/332-336/320-331/876-880；primitive_align.rs:35-126 |
| `everycr` | ⚠️ **注册了、可赋值、只存不注入** | builtins.rs:316；`primitive_toks_state.rs:106-140`；**align.rs 里 0 次引用** |
| `tabskip` | ✅ 参数槽 + 赋值 + preamble 快照 | builtins.rs:476；`param.rs:67/300/453/515`；align.rs:151-156/339 |
| `insert` | ⚠️ 扫描链完整、产物是有损文本节点 | primitive_align.rs:92-97 → `Node::Ins{text:String}` |
| `vtop`/`vcenter` | ✅ 已注册 | builtins.rs:46、391 |
| `hidewidth`/`ialign`/`multispan` | ✅ 正确缺席（plain.tex/latex.ltx 宏，非原语） | `plain.tex:591-592`、latex.ltx:16984 |
| `cramp` | ✅ 不存在（tex.web 本无此原语，**任务清单伪项**） | — |

- 对齐状态机：`align_on_token` 在主循环 `process_one` 顶部拦截（`expand/mod.rs:1552`）；
  `\halign` 臂 = `scan_box_spec` → `sink.box_spec` → `sink.align_begin(is_h)` →
  `align_scan_left_brace` → `align_start`（压 `AlignFrame{align_state:-1000000, Preamble}` +
  `sink.group_begin` + `begin_silent_group`）。
- 模板回放走 `InputFrame::AlignU/AlignV`（`mod.rs:109-114`），**不是 Macro 帧**——
  preamble 的 `#` 与宏参数机制天然不冲突（`align.rs:258-275`：cat6 在深度 0 即 u→v 分界，
  第二个 `#` 报 `Only one # is allowed per tab.`）。
- 增量一致性：`boundary_is_clean()` 已把"对齐帧非空"视为段边界不干净（`mod.rs:2267`）——
  **跨段对齐封包的增量策略挂点已预留**。

**ntex-layout（排版层）**：

- **模式机没有 Align 模式**（`Mode` 5 变体 `mod.rs:41-52`），对齐走
  `align_stack: Vec<(AlignDir, AlignCtx)>` + `GroupKind::Align=6/NoAlign=7`
  （`mod.rs:249-271`）；单元内容借用 `RestrictedHorizontal`（\halign）/`Vertical`（\valign）
  （`sink.rs:1620-1628`）。
- **TokenSink 已有完整对齐事件子面**：`align_begin`/`noalign_begin`/`align_preamble_end
  (tabskips: Vec<Glue>)`/`align_cell_begin`/`align_cell_end(AlignCellEnd{Tab,Cr}, span_len)`/
  `align_row_end`（`ntex-core/src/sink.rs:459-493`；布局实现 sink.rs:1581-1665）。
  **`\cr` 已建模；`\tabskip` 有专用载荷不占 glue 参数槽。**
- **两遍列宽定稿 `align_fin`**（`sink.rs:1763-1868`）：单列宽 → 跨列摊派 → `to` 摊派 →
  行封装 → vbox 封装。
- 数学层 `math.rs` **没有任何阵列/矩阵结构**（`MathAtom` 全集 mod.rs:121-177，只有
  分式/根式/上下划线/定界等）——矩阵必须走 `\halign` 宏路径。

### 3.2 已知简化（layout 侧注释自认的 6 处）

| # | 简化 | 证据 | 真实 TeX 行为 |
|---|---|---|---|
| S1 | **`\everycr` 只存不注入**（core 侧字段从未被读） | `expand/mod.rs:653` vs align.rs 0 引用 | 两处注入（裁决 5）；amsmath 每行重置标签状态靠它 |
| S2 | **tabskip 只取 `g.width`**：拉伸/收缩不参与定稿与 `to` 摊派（差额均分） | sink.rs:1770、1819-1843 | `\halign to` 的 glue set 按 tabskip 的 stretch/shrink 4 阶分摊（裁决 8 + tex.web fin_align） |
| S3 | **`\valign` 分支整体简化**（注释"不做行高数学"） | sink.rs:1772-1795 | 行高/深度取各列 `\vtop`/`\vbox` 的极值 + struts |
| S4 | **preamble `#{` 无特判**（BeginGroup 一律 `brace_depth+=1`） | align.rs:291-300 | `#{` 型不平衡模板（宏定义侧已有对照实现 `macros.rs:524-537`） |
| S5 | **多列越界钳末列**（注释自认"本实现改为钳在末列"） | align.rs:742-754 | tex.web 的恢复语义 |
| S6 | **`\halign` 无模式合法性检查 + EOF 静默**（hmode 内 `\halign` 静默开组；对齐中 EOF 直接 `return Ok(())`） | primitive_align.rs:21-32；align.rs:848-851 | `! Improper \halign` / `! File ended while in \halign`（裁决 7） |
| S7 | **spread 被丢弃**（`\halign spread 12pt` 的 spread 位不摊派） | sink.rs:1581-1593 注释 | TRIP L331 `\halign spread-12.truedd` 失真 |

另有两点**非对齐专属但 \halign 会踩**：`hpack` 只累计 `name: None` 的 glue 进 glue set
（`node.rs:477-489, 523-530`——命名 glue 如 leftskip 不拉伸，tabskip 走匿名节点
`align_tabskip_node` sink.rs:1872-1891 所以没事）；`Node::Leaders` 无 stretch_order 字段
（node.rs:186-194）。

### 3.3 端到端实测（ntex 二进制，ntex-dvi 构建时间晚于 HEAD，实测有效）

| 探针 | 真 TeX | ntex 现状 |
|---|---|---|
| `\halign{\hfil#\hfil&\hfil#\hfil\cr a&b\cr c&d\cr}\end` | 正常出页 | **112 字节 DVI / 1 页 / 0 字体**，PNG 整页空白 |
| `\hbox{H in hbox}\vfil\eject\end`（对照） | 正常出页 | **与上一条逐字节同 md5**（同一张空白页） |
| 纯文字 `Hello\end` | 正常出页 | **"没有可渲染的页面（\shipout 未触发？）"**——连页都不出 |
| `\output={\shipout\box255}` + 文字 + `\end` | 正常出页 | 同上，不出页 |
| TRIP 全量（ntex-trip --driver ntex） | — | **既有基线失败**（`组未闭合 groups=[SemiSimple, MathLeft, MathLeft, Align]`，宏载入战已知）；log 中 `Only one # is allowed per tab` 是 **fixtures/trip/trip.log 基线本就有**的预期错误，非偏差 |

> 三个读数：① 引擎层"能跑通"属实（不炸、有盒产出、`\end` 冲页成功——比纯文字文档
> 更进一步）；② **plain 通路无格式预载（无 `\font`、无 `\hsize`）→ 内容不可见**，
> `0 字体` 是铁证；③ **表格战役的对拍仪器现在不存在**——PNG 通路（ntex-backend）
> 与 DVI 通路（ntex-dvi）都需要先有一条"plain/LaTeX 文档 → 与真 TeX 可比的
> 盒树/DVI"的通道。**这就是刀 0。**

---

## 4. 缺口地图（按阻塞性排序）

> 量级记号同 output-routine-survey：**刀** = 一个最小复现 + 一条 tex.web 裁决 + 一次
> 真 TeX 对拍。依赖关系用 ← 标注。

| # | 缺口 | 证据 | 量级 | 为何排在这 |
|---|---|---|---|---|
| G0 | **对拍仪器**：plain 文档在 ntex 出空白 DVI（0 字体），无 `\showbox` 盒树可比通道 | §3.3 | 0.5–1 刀（勘察 + 决策：预载 plain 子集 / 复用 LaTeX 通路 / 纯单测锚点） | **没有它每一刀都无法验收**；也决定后续所有刀用 DVI 对拍还是盒树对拍 |
| G1 | **`\everycr` 注入**（S1）：两处注入点（preamble 后 + fin_row），且在 `align_peek` 前瞻之前 | align.rs 0 引用 vs 裁决 5 | 1 原语接线；**1 刀** | amsmath `\everycr{\noalign{…}}`（1921）每行重置标签；plain `\ialign`/LaTeX `\eqnarray` 依赖"清空"语义成立 |
| G2 | **`to`/`spread` 摊派真语义**（S2/S7）：tabskip 的 stretch/shrink 4 阶参与 glue set | sink.rs:1770/1819-1843/1581-1593 | `align_fin` 纯函数精化；**1–2 刀** | `\eqnarray`(`\halign to\displaywidth`)、longtable(L167)、TRIP L331 全压在这；**这是表格列宽正确性的心脏** |
| G3 | **preamble 宏展开通路 + `#{` 特判**（S4）：kernel 的 `\edef\@preamble{…}` + `\let\@sharp##` 形态能否走通 | align.rs:222-225（raw 拷贝）/291-300 | 1–2 刀（先复现裁决再改） | **kernel tabular 的头号未知数**（§2.2 ★1）；`\span` 后展开已实现（align.rs:237-257），需确认"宏展开产物中的 cat6 `#`"被当作列占位 |
| G4 | **`\valign` 行高/深度数学**（S3） | sink.rs:1772-1795 | 1 刀 | kernel 唯一用点是假欧元（14587），**不阻塞 tabular**；但 `\vtop` 单元（array p 列）依赖正确行高 |
| G5 | **合法性/报错面**（S6）：`\halign` 模式检查 + interwoven + EOF 报错 + 多列越界恢复（S5） | primitive_align.rs:21-32；align.rs:848-851/742-754 | 0.5–1 刀（可拆两把小刀） | 静默错位比报错更伤：对齐组不闭合正是 TRIP 现有基线失败的形态之一（`groups=[…, Align]`） |
| G6 | **kernel lttab 宏通路冒烟**：`\@array/\@mkpream/\@arstrut/\@arraycr/\multicolumn` 全链在 ntex 载入并出对拍盒树 | §2.2、§2.4 锚点 | 1–2 刀（宏层为主，引擎侧只补 G3） | tabular 走通的验收本体；2575 行 preamble 构造是宏载入战的压力样本 |
| G7 | **array.sty / p 列**：`\d@llarbegin`、`\@@startpbox/\@@endpbox`（`\vtop` 单元内垂直材料）、`\extracolsep` | array.sty | 1–2 刀 | 依赖 G4（行高）与 G6 |
| G8 | **数学矩阵**：plain `\eqalign/\matrix/\pmatrix`（= `\vcenter{\ialign{…}}`）与 amsmath `\align@`（`\span\align@preamble`） | plain.tex:1091-1135；amsmath:1829/1926 | 1 刀（引擎侧增量≈0，主要是宏层 + `\vcenter` + `Improper \halign` 检查） | **与 \halign 共享全部地基**（见 §6 Q3）；`\eqalign` 是最好的最小对拍样张 |
| G9 | **longtable / booktabs**：`\noalign`×12 行协议、`\futurelet` lookahead、`\tabskip\LTleft` | longtable.sty/booktabs.sty | 另计 2–3 刀 | 依赖 G1/G2/G6；`\multicolumn` 的 `\@mkpream` 单元内重入在此收口 |

**不在缺口里的（明确排除，免得白做）**：`\cramp`（tex.web 无此原语，任务清单伪项）、
`\hidewidth`/`\ialign`/`\multispan`（宏，非原语——但**依赖 plain.tex/kernel 宏层可用**，
归 G0/G6）、`\insertpenalties`/`\holdinginserts` 的页侧消费（属输出例程战 G3）、
`\valign` 的完整表格化（`\valign` 在 LaTeX kernel 只有假欧元一个用点，plain 的
`\valign` 用点也少）。

---

## 5. 验收路径（建议切刀顺序）

```
刀 0  对拍仪器（G0）——plain 冒烟通道决策 + \showbox/盒树对拍口径
      复现：\halign{\hfil#\hfil&\hfil#\hfil\cr a&b\cr c&d\cr}\end
      验收：ntex 与真 TeX 的盒树同形（真 TeX 锚点见 §2.4；最小基准 =
            \hbox 外壳包 \vbox、行数正确、列宽非零）
刀 1  \everycr 注入（G1）
      tex.web 裁决 5（两处注入点 + 先于 align_peek）
      对拍：p3g 形态（\everycr{\noalign{…}}）+ p3 角落行为
刀 2  preamble 展开通路 + #{} 特判（G3）
      对拍：\edef\@preamble 形态最小化（\let\@sharp## + 宏灌模板）
刀 3  to/spread 摊派（G2）
      对拍：\halign to 200pt{\hfil#\hfil&\hfil#\hfil\tabskip0pt plus1fil\cr …}
            与 \halign spread-12.truedd（TRIP L331）
刀 4  合法性/报错（G5，含 S5 越界恢复）        ┐ 与刀 3 无依赖，可并行
刀 5  \valign 行高数学（G4）                   ┘
刀 6  kernel tabular 全链（G6）：{\halign 己}→ \begin{tabular}{|l|r|} 盒树对拍
刀 7  array.sty / p 列（G7）
刀 8  数学矩阵（G8）：\eqalign/\matrix 盒树对拍（顺带收 \vcenter + Improper \halign）
刀 9  longtable/booktabs（G9）
```

**依赖链**：刀 0 → 一切；刀 1/2 → 刀 3/4/5（同一状态机）；刀 3 → 刀 6（`to` 摊派是
tabular 列宽前提）；刀 6 → 刀 7/8/9。
**与并行线的交界**：刀 2/3 改 `expand/align.rs` 与 `sink.rs:1763-1868`，与 A 线
（math.rs、expand/ 残差）在 `expand/` 目录上有合流风险——开工前须与 A 线对表
rebase 顺序。`Node::Ins` 改造**不在本战役**（输出例程战 G3），但 G3 的
`scan_group_contents` 复用点与本战役刀 2 相邻，先做 insert 体 token 化会碰同一扫描器。

---

## 6. 报告必答四问

**Q1：tabular 走通最少需要哪些原语/机制？（按 latex.ltx 真实用点）**
原语面**全部已在**（`\halign/\noalign/\omit/\span/\cr/\crcr/\tabskip/\vtop/\vcenter/
\everycr`，§3.1）——最小集不是"注册原语"，而是四件事：
1. `\everycr` 注入（G1，kernel `\ialign`/`\eqnarray` 第一步就是 `\everycr{}`）；
2. preamble 宏展开通路 + `#{`（G3，`\edef\@preamble` + `\let\@sharp##` 是 lttab 的
   骨架形态）；
3. `to` 摊派（G2，`\eqnarray`/longtable 用 `to`；纯 `\tabular`（无 `to`）可以
   侥幸绕过，但 `|` 列的 `\vrule` 宽与 `\tabcolsep` 全靠列宽定稿正确）；
4. kernel 宏层可载入（G6，lttab 的 `\@expast/\@tfor/\ifcase` 机器——属宏载入战
   既有能力的压力样本）。
   `\multicolumn`（`\multispan`=\omit+循环`\span`）与 `\\[2pt]`（`\cr\noalign{\vskip}`）
   在现状骨架上**大概率已通**，列为刀 6 的验收项而非独立刀。

**Q2：`\insert` 排版化与 `\halign` 哪个更前置？依赖关系？**
**`\halign` 更前置。** 需求面：`\halign` kernel 4 宏 + amsmath 8 处 + array/longtable/
booktabs；`\insert` 在 latex.ltx 真实排版用点只有 `\@footnotetext` 1 处（17950）。
依赖面：`\insert` 排版化需要断页器/`\vsplit`/insert 三联寄存器（输出例程战 G3，
刀 3/4 已排期），与本战役**零共享地基**（\halign 走组/盒组装，insert 走页构建器）；
反向才有交点——insert 体 token 化会碰 `scan_group_contents`（本战役刀 2 的邻居）。
结论：表格文档（tabular/array/eqnarray/matrix）全卡 \halign；脚注卡 \insert；
两者可并行，谁先取决于战役目标（表格类文档 → \halign）。

**Q3：数学矩阵（\matrix/\eqalign）与 \halign 共享多少地基？**
**全部共享，矩阵不需要独立引擎战役。** plain 的 `\matrix/\pmatrix/\eqalign/\cases`
= `\vcenter{\normalbaselines\m@th\ialign{…\crcr#1\crcr}}`（plain.tex:1091-1135），
amsmath 的 `\align@` = `\halign\bgroup\span\align@preamble\crcr`（1829/1841）+
measure 试排遍（1926）。引擎侧唯一增量 = ① `\vcenter` 已注册（builtins.rs:391），
需验证其在数学模式的包装正确；② `Improper \halign inside $$'s` 检查（裁决 7，
S6 的一部分）；③ 数学层落点：`GroupKind::Align` 组结束分支把产物盒交给
`MathAtom::Box`（layout 挂钩点候选 4，`sink.rs:804-815`）。`\eqalign` 2 列模板简单，
是本战役**最好的最小对拍样张**。

**Q4：总刀数量级 + 建议切刀顺序？**
**9–12 刀**（G0 0.5–1 + G1 1 + G2 1–2 + G3 1–2 + G4 1 + G5 0.5–1 + G6 1–2 + G7 1–2
+ G8 1，longtable/booktabs 另计 2–3）。顺序见 §5：**刀 0（仪器）→ 1（everycr）→
2（preamble 通路）→ 3（to 摊派）→ 4/5（并行小刀）→ 6（kernel tabular）→ 7（array）→
8（矩阵）→ 9（longtable/booktabs）**。其中刀 2/3 与 A 线在 `expand/` 有合流风险，
开工前对表。

---

## 7. 复现步骤（ground truth 重建，约 3 分钟）

```bash
export PATH=$HOME/.local/bin:$PATH
mkdir -p /tmp/halign-work && cd /tmp/halign-work
# 1) 最小探针（§2.3 的 p1-p5；真 TeX 全净，p3 是记录在案的语义角落）
tex -interaction=nonstopmode p1.tex && grep -c '^!' p1.log        # 0
# 2) tabular 计量（§2.4）
latex -interaction=nonstopmode tab2.tex   # \tracingcommands=1 \tracingoutput=1
awk '/Completed box being shipped/,0' tab2.log | head -30          # 盒树锚点
# 3) 需求面计数
B=$HOME/.TinyTeX/texmf-dist/tex/latex
for p in halign valign noalign omit span crcr everycr tabskip hidewidth; do
  printf '\\%-9s' "$p"; for f in base/latex.ltx tools/array.sty tools/longtable.sty \
     amsmath/amsmath.sty; do printf ' %s' "$(grep -o "\\\\$p" $B/$f | wc -l)"; done; echo; done
# 4) ntex 现状（§3.3；二进制须比 HEAD 新）
/home/ubuntu/NTex/target/debug/ntex-dvi /tmp/halign-work/c3.tex   # 112 字节 / 0 字体
# 5) TRIP 对齐段既有基线
/home/ubuntu/NTex/target/debug/ntex-trip --driver ntex --test trip
grep -c 'Only one # is allowed per tab' /home/ubuntu/NTex/fixtures/trip/trip.log  # 基线本就有
```

---

## 8. 边界与诚实声明

1. **行号可信度分级**：需求面计数与调用点归属全部 grep 实测；tex.web 行号实测；
   引擎侧行号来自两个只读代理的勘察（ntex-core 的 `expand/align.rs` 属 A 线领地，
   A 线合入后行号会漂移，引用以名字为准）；latex.ltx 的 l.NNN 随版本漂移（继承教训，
   勿当常量）。
2. **"kernel tabular 能否走通"仍是未知数**，卡在 G3（preamble 宏展开通路）——
   本报告只确认了现状骨架是 raw 拷贝 + `\span` 后一次展开，没有实证
   `\edef\@preamble{\ialign…}` 这一形态在 ntex 的行为（刀 2 首个复现）。
3. **端到端实测的样本面**：ntex 侧探针全部是 plain 格式、无格式预载；"空白 DVI /
   0 字体"不能区分"对齐产物为空"与"无字体导致内容不可见"（对照批 `\hbox{H in hbox}`
   同样空白，倾向后者）。刀 0 的第一件事就是把这两个假设拆开。
4. **p3 角落未钉死**：`\everycr={\message{…}}` × `\halign to` × `\crcr`/尾 `\tabskip`
   的三方交叠触发 `! Missing \cr inserted.` ×6，逐项二分只排除了单因子（§2.3）。
   刀 1 动 `\everycr` 时应以 tex.web 注入点为准，并以 p3 作回归探针。
5. **TRIP 的 `Only one # is allowed per tab` 不是偏差**：fixtures/trip/trip.log 基线
   本就含它（已 grep 核实）。TRIP 在 HEAD 的失败（`组未闭合 […, Align]`）是宏载入战
   已知基线，本报告未深挖其对齐段的独立贡献。
6. **频率≠依赖**（继承教训）：`\crcr`×25（amsmath）不构成缺口（引擎已实现）；
   缺口判定以「宏层用点存在 且 引擎面简化/缺失」为准（§3.2 的 S1–S7）。
7. 探针与转录均不入库；`fixtures/`、`crates/` 零改动；本轮未跑任何 cargo 构建
   （复用 target/ 已有二进制，ntex-dvi 构建时间 2026-09-07T00:00 晚于 HEAD
   2026-09-06T23:31，实测有效）。
