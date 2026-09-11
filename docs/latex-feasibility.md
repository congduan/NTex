# LaTeX 兼容战役（活文档）

> **本文档只保留当前有效信息。** 已修完的历史逐刀记录（§8–§37 共 30 节、约 2900 行）
> 归档于 [archive/latex-feasibility-full-2026-09-11.md](archive/latex-feasibility-full-2026-09-11.md)
> —— 需要查具体某刀的根因链/tex.web 锚点时去那里 grep。

**目标**：`latex.ltx` 加载成功 → `latex.fmt` 构建 → `\documentclass{article}` → PDF。

**当前状态**：🔴 **阻塞在 expl3 加载期的输入栈无终止条件**（见 §A）。

---

## A. 当前阻塞点（唯一活跃项）

### A1. expl3 变体生成器压栈不终止

```
== pass1 ERROR: 输入栈超限（5001 帧 > 5000）
   展开/参数扫描疑似无终止条件（定义 \l__iow_line_part_tl 的替换文本时）
```

watchdog 现场（`latex_probe --initex`，2026-09-11 复测）：

```
last_tok = \cs_generate_variant:Nn
         → \__prg_F_true:w → \if_meaning:w → \tex_advance:D
栈内出现 TokenList(12640tok, pos=4801)  ← 大 token 列表反复重入
```

**定性**：`\__cs_generate_variant_loop:nNwN` 一带的三层嵌套 + `{ ~ { } \fi: ... } ~`
组内藏 `\fi:` 惯用法，chose 桩 `\use_ii:nn` 取参错位 → 递归不终止。

**方向**：修递归终止条件。**不要**抬高 `MAX_INPUT_STACK`（5000 是 tex.web/TL
生产契约，卡 5001 = 递归不终止的防护网，改大只是掩盖）。

**复现**：
```bash
export PATH="$HOME/.cargo/bin:$PATH" NTEX_TFM_DIR="$HOME/.ntex-fonts"
# 备料（若 /tmp/r27b 被清）：取 latex.tar.xz + l3kernel.tar.xz 的生成版
cargo run --release -p ntex-test-support --example latex_probe -- \
    /tmp/r27b/latex.ltx --initex 2>&1 | tail -30
```

### A1.bis 精确定位（2026-09-11，转录首现场）

**`latex.ltx.transcript` 的第一个错误不在末端，而在 expl3-code.tex l.21291**：

```
! Extra \else.
l.21291 \cs_new:Npe \__fp_atan_default:w #1#2#3 @ { #1 #2 #3 \c_one_fp @ }
! Missing number, treated as zero.
<to be read again> \__fp_sep:
! Improper alphabetic constant.   ×2（`` ` `` 后跟非单字符）
! Missing endcsname inserted.
<to be read again> \__fp_sep:
```

**根因定性：`Npe` 变体宏未生成**。`\cs_new:Npe` 是
`\cs_generate_variant:Nn \cs_new:Npn { Npe }` 的产物；生成失败 → `\cs_new:Npe`
落成未定义/退化宏 → `@`（cat 12 定界符）与 `\c_one_fp` 错位 → 整条链崩
（2489 条 `Missing endcsname`，末端停在 l.25851，栈超限 5001 帧）。

**变体生成器现场**（expl3-code.tex L2807-2833）：
- L2807 `\cs_generate_variant:Nn` 定义体用 `\use:e{...}` 包裹
  `\__cs_generate_variant:nnNN`（**`\use:e` = `\edef` 全展开**）
- L2822 `\cs_new_protected:Npe \__cs_generate_variant:N` —— **自举**：该宏自身
  就是 `Npe` 变体，定义体里用双 `\exp_not:N` + 同 token 探测
  `\cs_new_protected:Npe` 是否已定义
- L2891 `\__cs_generate_variant_loop:nNwN`：三层嵌套条件
  `\if:w N #4 \else:\if:w n #4 \else:1\fi:\fi:` + `{ ~ { } \fi: ... } ~`

**已单独验证正常**（排除嫌疑）：
- `\exp_not:N \exp_not:N #1 #1` 同 token 探测惯用法 → 与 pdfTeX 同为 `SAME`
- `\if:w N #1 T\else:F\fi:`（字面 char vs 实参）→ 与 pdfTeX 同为 `T`
- `\ifx` 版嵌套条件短路 → `A0 B0 C10` 正确
- INITEX 初表 `\catcode0` = 9（与 pdfTeX -ini 一致，**非 G4 问题**）

**下一刀入口**：在 `\use:e`（`\edef` 全展开）路径上验证 `\__cs_generate_variant:nnNN`
的展开结果——重点看 `\cs_split_function:N` 与 `\tl_to_str:n` 在 `\edef` 内的交互
（`\exp_not:N` 标记在 `\edef` 中是否被正确消费）。

### A1.ter 调用链定位（2026-09-11，JSONL trace 产出）

用新设施 `NTEX_TRACE_JSONL` + `scripts/trace-view.py` 抓 **122 万事件**，得到决定性数据：

```
栈深峰值 5001 @ step 228182   frame=TokenList(1tok)  tok=\tex_edef:D
高频 token top5: \__kernel_tl_set:Nx(144217)  \tex_edef:D(143772)
                \if_int_compare:w(110190)  \exp_after:wN(110091)
                \tex_expanded:D(95115)
```

**栈重建**（`--stack-at 227257`）显示**递归下降形态**：

```
[1099785] TokenList(30tok) ←tok=\__kernel_tl_set:Nx
[1099788] TokenList(16tok) ←tok=\__kernel_tl_set:Nx
[1099791] TokenList(1tok)  ←tok=\__kernel_tl_set:Nx
[1099794] TokenList(28tok) ←tok=\__kernel_tl_set:Nx
[1099797] TokenList(16tok) ←tok=\__kernel_tl_set:Nx
[1099800] TokenList(1tok)  ←tok=\__kernel_tl_set:Nx
   …  token 数递减 30→28→26→24→22→20→18→16→14→12，每次吐 (16tok, 1tok) 后回自身
```

**`\__kernel_tl_set:Nx` 反复压帧且不回退** = 递归不终止。

### 别名链（expl3-code.tex）

```tex
L3560  \cs_new_eq:NN \__kernel_tl_set:Nx \cs_set_nopar:Npe
L1556  \tex_global:D \tex_let:D \cs_set_nopar:Npe \tex_edef:D
```

即 `\__kernel_tl_set:Nx` → `\cs_set_nopar:Npe` → `\tex_edef:D`（= `\edef`）。

### 已排除的嫌疑（本轮实测）

| 嫌疑 | 结论 |
|---|---|
| `\let` 到宏/Alias 解析 | ✅ 正常（`\let\cs_set_nopar:Npe\tex_edef:D` 后 `\meaning` 与调用均正确）|
| `\tex_gdef:D <cs> {body}` 空参宏定义 | ✅ 正常（`\meaning` 得 `macro:->\tex_long:D\tex_xdef:D`）|
| `\exp_not:N` 同 token 探测惯用法 | ✅ 与 pdfTeX 同为 SAME |
| `\if:w N #4` 字面字符比较 | ✅ 与 pdfTeX 同为真 |
| INITEX 初表 | ✅ 已修（`--no-plain` 切 INITEX 表）|

### 下一刀入口（收窄后）

死循环在 **`\__kernel_tl_set:Nx`（= `\edef` 别名）的调用点**，且伴随
`\if_int_compare:w` 高频（循环判据）。**下一步**：

```bash
NTEX_TRACE_JSONL=/tmp/t.jsonl ntex-test-support/latex_probe ...
scripts/trace-view.py /tmp/t.jsonl --grep tl_set:Nx --limit 200   # 看首次出现的调用点
scripts/trace-view.py /tmp/t.jsonl --around <首次step>            # 那一步的上下文
```

重点查 `\edef` 在**其参数（tl 变量）尚未定义**时的行为：NTex 若在
`\edef <未定义 cs> {...}` 上走了「展开自身」而非「报错 + 当 `\relax`」，
就会形成 `\edef` → 展开体 → 又见 `\edef` 的自我复制。

### A1.quater 单步内爆栈的精确定位（2026-09-11，trace 二次分析）

栈深时序（每 5 万事件采样）显示**长期健康、末步爆炸**：

```
idx=1050000  step=219323   depth=57      ← 一直很浅
idx=1100000  step=227286   depth=39
idx=1150000  step=228182   depth=1838    ← 同一步内
idx=1200000  step=228182   depth=4010
idx=1222683  step=228182   depth=5001    ← 超限
```

**step 228182 内压帧 116,850 次**，形态：

```
depth= 26  TokenList(38tok)      tok=\tex_edef:D
depth= 26  TokenList(1tok)       tok=\tex_edef:D
depth= 27  TokenList(33426tok)   ← 巨型 token 列表
depth= 19  TokenList(16709tok)   ← 又一个巨型列表
depth= 20..30  TokenList(1tok) ×11   ← 单调递增
depth= 21..28  TokenList(1tok) ×8    ← 回落后再增（周期 ≈12-14 帧）
   …重复 11 万次
```

**判读**：`\edef`（`\tex_edef:D`）在展开一个**巨型 token 列表**时，每处理
一批 token 就压入若干 `TokenList(1tok)` 帧且**不被弹出**；周期性回落说明有
递归调用自身的行为，每轮留下残留帧。**这是「单步内无终止」的典型形态**——
watchdog 只在 `process_one` 返回时计数，所以只能靠栈深暴露（`NTEX_TRACE_JSONL`
正是为此而生）。

**定位到函数**：`crates/ntex-core/src/expand/macros.rs::scan_edef_body`
（`\edef` 体扫描）——它是 `\edef` 展开巨型列表时的帧生产者。

**下一刀靶**（收窄后可动手）：`scan_edef_body` 的 `'scan` 循环里，
`self.fetch()` 返回的帧（尤其 `TokenList`）在循环内是否**成对弹出**；
重点查 L710 起的 `csid` 分支与 L677 的 `fetch()` 交互——巨型
`TokenList(33426tok)` 被压入后逐 token 消费，若每次消费又经
`expand_once` 压入新帧而不复用，即形成累积。

### A1.quinquies logtrace 首现场纠偏（2026-09-11）

新增设施 `scripts/logtrace.py` 首次运行即**纠偏了首现场判断**：

```
总行数 25061 | 错误 5620 | 归一化后 14 种

① 首现场（第 6 行）  Undefined control sequence.  @ l.301
② 震中   l.21039  1253 条 (22.3%)   ← 错误火山口
         l.20279   204 条 ( 3.6%)
         l.25851    58 条 ( 1.0%)
④ 首次出现序：
   #6     l.301     Undefined control sequence.   ← 真起点
   #79    l.0       Missing = inserted for \?
   #144   l.20279   Font \?? has only N fontdimen
```

**纠偏**：此前人工定位（§A1.bis）判首现场为 `l.21291`（`\cs_new:Npe`），
logtrace 显示**真首现场在 `l.301`，早 2 万行** —— 该行属 latex.ltx/texsys 探测区
（`\edef\reserved@a{\expandafter\reserved@a\string^^J\@@}`，其后紧邻
`\@input` 链）。

**注意「l.0」条目**：多条错误**无位置行**（引擎丢失行号信息）——本身是缺陷信号，
列入待查。

**下一刀顺序修正**：先查 l.301 的 `Undefined control sequence`（真起点），
再回头处理 l.21039 震中（1253 条）与 `\edef` 爆栈（§A1.quater）。

### A1.sexies 首现场诊断与一处真修复（2026-09-11）

**首现场 = latex.ltx L295-303 的 TeX 版本嗅探**：

```tex
\ifx\@TeXversion\@undefined
  \ifx\@undefined\inputlineno
    \def\@TeXversion{2}
  \else
   {\catcode`\^^J=\active
     \def\reserved@a#1#2\@@{\if#1\string^3\fi}
     \edef\reserved@a{\expandafter\reserved@a\string^^J\@@}   ← L301
     \ifx\reserved@a\@empty\else\gdef\@TeXversion{3}\fi}
  \fi
\fi
```

**✅ 已修真 bug：`\string` 对 active char 多打 escapechar 前缀**

`string_token`（`expand/free.rs`）对 `TokenKind::ControlSeq` **无条件**加
`escapechar` 前缀；而本引擎把 active char 编码为「带 `CS_ACTIVE_FLAG` 的
csid token」→ `\string^^J` 输出 `\<换行>` 而非裸换行。

tex.web 依据（`conv_toks` L9280-9281）：
```pascal
string_code: if cur_cs<>0 then sprint_cs(cur_cs) else print_char(cur_chr);
```
—— 判据是 **`cur_cs≠0`**，而 active char 的 `cur_cs=0` → 走裸字符路径。

修复：`string_token` 里按 `tok.is_active()` 归位到字符分支。
验证（`cat -A` 逐字节）：

| 探针 | pdfTeX | NTex（修复后）|
|---|---|---|
| `\message{[B \string^^J]}`（`^^J` active）| `[B $`（裸换行）| `[B $` ✅ |

**该 bug 会污染下游 token 流**：多出的 `\` 被当 cs 前缀 → 未定义 cs 报错
（正是首现场里的 `! Undefined control sequence.` + 孤立 `\`）。

**⏳ 首现场仍未全消**：`l.301` 仍报同错。已收窄到
**`\edef` + `\expandafter\string^^J`** 的组合（`\edef\z{\expandafter\t\string^^J\@@}` 复现），
`\message` 未及执行即报错。已排除 `\string` 本身（`\string^`/`\string x`/`\string\relax`
逐字节与 pdfTeX 一致）。

**下一刀靶**：`\edef` 体的 token 扫描遇到 **char 10（换行）token** 时的处理——
`\string^^J` 产出的是 charcode 10 的 Other 字符，本引擎行模型（LF=cat 5）
可能让它在 token 流中触发特殊路径。查 `scan_edef_body` 与 `fetch()` 对该
charcode 的处理，并在 `abcheck` 上加一个固定探针
（`scripts/abcheck-examples/string-active-newline.tex`）。

### A1.septies 首现场真根因：charcode 10 token 在流中被行尾模型吞掉（2026-09-11）

§A1.sexies 修掉了 `\string` 对 active char 的 escapechar 前缀（真 bug，已入库），
但 `l.301` 首现场**未消**。本轮用最小对照把真根因钉死。

**决定性实验**：

```tex
\def\showit#1{[GOT #1]}
\expandafter\showit\char10
```

| | 输出 |
|---|---|
| **pdfTeX** | `[BEFORE]` + `! Missing number` + `[AFTER]` —— `\showit` **被调用**（char 10 进了 `#1`）|
| **NTex** | `[BEFORE][AFTER]` —— **`[GOT]` 完全未输出，`\showit` 根本没被调用** |

**根因**：本引擎行模型 **`LF`（charcode 10）cat 5 = 行尾符**（见
`CatcodeTable::initex()` 的"偏差说明"与 `docs/latex-feasibility.md` A1）。当
**charcode 10 作为普通 token 进入 token 流**（`\char10`、`\string^^J`、
`\string` 对 active `^^J`）时，被引擎按行尾处理**吞掉/转换**，无法作为
字符数据传递。

**影响面**（结构性，非单点）：

1. `\string^^J` 类构造（latex.ltx L301 TeX 版本嗅探、`\@ifnextchar` 变体等）
2. `\char10` / `\char` 对 10 的使用
3. 任何 `^^J`（LF）作为 active char 被 `\string`/`\edef` 处理的场景
4. expl3 的 `\c__char_lf` 相关构造

**为何是深层问题**：`LF=cat 5` 是引擎**字节流直读**设计的直接后果（不剥行尾字节，
靠 catcode 5 找行尾）。这个设计让「源文件里的 LF」与「token 流里的 char 10」
**无法区分**——而 tex.web 里读取层剥掉行尾 LF、再补 `\endlinechar`，两者
天然分离。

**下一刀方向**（两条路，需裁决）：

- **A 路（外科）**：在 `\char`/`\string`/`\edef` 产出 char 10 token 时加
  「非行尾」标记位（Token 有 8 字节，尚有 spare bit），读取层遇该标记不触发行尾
  语义。改动小、风险可控，但需全链检查 `fetch()` 的 LF 处理点。
- **B 路（根治）**：改输入层为「剥行尾 LF + 补 `\endlinechar`」（tex.web 原语义），
  彻底分离「源字节 LF」与「token char 10」。**波及面大**（所有依赖 LF=5 找行尾
  的路径：注释跳行、空行→`\par`），需大范围回归。

**建议先走 A 路**（外科、可回退），B 路作为 M 级架构债登记。

### A1.octies ⚠ 本轮两次误判的更正 + 一个新的真差异（2026-09-11）

**必须诚实记录**：本轮我在 §A1.septies 之后提出的两个"根因"**都是探针误判**，
已用 pdfTeX ground truth 逐个证伪。这是仪器纪律的又一次实战（见
`docs/tooling-trust.md`）。

#### 误判 1：「charcode 10 token 被行尾模型吞掉」——证伪

`\char10` **单独执行完全正常**（产出 char 10）；`\string^^J` 也已由 §A1.sexies
修复。此前用 `\expandafter\showit\char10` 得出"char 10 被吞"的结论，
实际是**探针本身构造错误**（见下）。

#### 误判 2：「宏实参位置不展开可展开原语」——tex.web 语义如此，非缺陷

我观察到 `\showit\char65` → `"[\0]65"`，判定为缺陷。**pdfTeX ground truth 否定**：

```
\def\showit#1{[#1]}
\message{[A]}\showit\char65\message{[B]}

pdfTeX:  [A] + "! Missing number, treated as zero." + [B]
NTex  :  [A][B]（无错误）
```

**tex.web 的实参扫描走 `get_token`——既不展开也不推进条件机**（
`collect_undelimited_arg` 的既有注释 L348-352 早就写明）。故 `#1` = `\char`
单 token，`65` 留在外面，`\char` 执行时读不到数字 → **pdfTeX 报
`Missing number`**。

**NTex 的行为差异（真差异，方向相反）**：NTex **不报错**、静默继续。
即 NTex 比 tex.web **更宽松**——`\char` 在拿不到数字时未走
`! Missing number, treated as zero.` 恢复路径。

**已回退**：我曾按"实参应展开"的误判改了 `collect_delimited_arg`（加
可展开原语执行臂），验证 pdfTeX 后**立即 `git checkout` 回退**——该改动会让
NTex 偏离 tex.web（在实参位置多做一次展开）。

#### 新增待查项（真差异，低优先）

**NTex 缺 `\char` 无数字时的 Missing number 报错**：

| 构造 | pdfTeX | NTex |
|---|---|---|
| `\showit\char65` | `! Missing number, treated as zero.` | 静默（`[\0]65`）|
| `\expandafter\showit\char65` | 同上 | 静默 |

归类：**错误报告面缺失**（不影响排版结果，影响诊断保真与 TRIP 口径）。
登记为待办，不阻塞 latx.ltx 推进。

#### 教训（写入纪律）

**探针要先用 pdfTeX 验证「预期是否正确」，再拿 NTex 结果下结论。**
本轮的两次误判都源于「先假设 tex.web 语义、再造探针」——而 `abcheck.py`
正是为此存在：**任何"NTex 有 bug"的结论，必须先经 abcheck 确认 pdfTeX
的行为与之不同**。

### A1.nonies 首现场收窄：`\edef` 产出含 char 10 的宏后，后续间歇报 undefined（2026-09-11）

用最小分离实验把 `l.301` 首现场收窄到**可复现的最小形态**（已排除 `\reserved@a`、
`\@@`、`\if...\fi` 结构）：

```tex
\input plain
\catcode`\@=11
{\catcode`\^^J=\active
  \edef\x{\string^^J}                       ← 单独执行**成功**（x = char 10）
  \ifx\x\@empty \message{[ISEMPTY]}\else \message{[NOTEMPTY]}\fi   ← 判定正确
  \message{[AFTER]}}                          ← 此处前多一条 undefined
```

**实测**：

| 观察点 | 结果 |
|---|---|
| `\edef\x{\string^^J}` 本身 | ✅ 成功（`\x` 非空、`[NOTEMPTY]` 正确）|
| `\edef\x{abc}`（对照，无 active char）| ✅ 干净无错 |
| 之后任意 `\message` | ❌ 前多一条 `! Undefined control sequence.` + 孤立 `\` |
| 错误消息体 | **`\` + 空行** —— 被报的 cs **名字为空** |

**当前判读（未定论）**：`\x` 含 char 10 后，引擎在后续处理中出现「空名 cs」
被当未定义控制序列报错。可能位置：

1. `\meaning`/`\message` 输出通路遇 char 10 时行结构错位（但本实验已排除
   `\message` 直接打印 `\x`）；
2. **intern 表里「空名 cs」的来源** —— `\string` 对 active char 走
   intern 名首字符，若该名字为空/异常则产出空名 token；
3. `\ifx` 比较时对含 char 10 的 token 列表的处理。

**下一刀（明确、可执行）**：在 `exec_string`/`string_token` 出口加
`NTEX_TRACE_JSONL` note 事件打印**产出的 token 序列**（kind/cat/code），
即可确定 char 10 之后是否混入了空名 cs。**不要再靠语义猜测造探针**
（本轮已因此误判两次，见 §A1.octies）。

### A1.decies 首现场再收窄（2026-09-11，⚠ 含一次旧二进制误读的更正）

**⚠ 先更正一个过程错误**：本轮中段一度报告「active LF 后每行报错」，
那是**跑在含调试插桩的旧二进制**上得到的结果（`git checkout` 回退源码后
忘记 rebuild）。**重新 build 后该现象消失**。教训：**改完源码必须
`cargo build` 再验证，否则读的是旧产物**（skill 已记录该坑，此处再次命中）。

#### 干净二进制下的分层结果

| 用例 | 错误数 | 输出 |
|---|---|---|
| `{\catcode`\^^J=\active` + 换行 + `\message` | 0 | 正常 |
| 同上 + `\def\x{Q}` | 0 | 正常 |
| 同上 + `\edef\x{Q}` | 0 | 正常 |
| 同上 + `\edef\x{\string^^J}`，再 `\message{[M \meaning\x]}` | **0** | `[M macro:->` + 换行 ✅ |
| **latex.ltx L301 原场景**（`\def\reserved@a#1#2\@@{...}` + `\edef\reserved@a{\expandafter\reserved@a\string^^J\@@}` + 再 `\message{[…\meaning\reserved@a]}`）| **1** | `[PRE][DEF-OK][DONE]`，**`[EDEF-OK …]` 整条丢** |

#### 错误形态与位置

```
! Undefined control sequence.
\                      ← 被报的 cs 名字为空
l.8   \message{[EDEF-OK \meaning\reserved@a]}}
```
错误挂在**下一行**（`l.8`），实际执行的是 `l.7` 的 `\edef` —— **错误上下文行号错位一格**。

#### 已排除（本轮新增，全部经干净二进制复核）

- ✅ `\catcode`\^^J=\active` 后换行本身（多行/单行均正常）
- ✅ `\edef` 含 active char 的普通内容
- ✅ `\string^^J` 产出 char 10（插桩确认 `name_bytes=[10]`）
- ✅ **`\meaning` 对含 char 10 的宏**（`[M macro:->` + 换行，0 错）

#### 未定（最小剩余差异）

与上面「已排除」各项的**唯一差别**是 `\reserved@a` 是**带定界参数文本的宏**
（`#1#2\@@`），且 `\edef` 体里用 `\expandafter\reserved@a\string^^J\@@`
把它**递归调用自身**。怀疑点收窄到：

1. `\expandafter` + **带定界实参的宏**在 `\edef` 展开上下文里的实参收集
   （`\@@` 作定界符的匹配/消费）；
2. 或 `\meaning` 对**带定界参数文本的宏**输出时，`params.text` 里的 `\@@`
   与 char 10 的组合。

**下一刀（不靠猜）**：对 `l.7` 的 `\edef` 加 `NTEX_TRACE_JSONL`
note 事件（在 `exec_def`/`expand_region` 出口打印产出的宏体 token 序列），
直接看 `\reserved@a` 被赋成了什么。**先 build 再验**。

### A1.undecies 爆栈现场精查（2026-09-11）：单 token 纯递归，非巨型列表

**决定性数据**（trace 二次精查 step 228182 的 116,850 条）：

```
帧类型分布: TokenList = 116,850（100%）
token 分布: \tex_edef:D = 116,850（100%，无其他 token）

最后 12 条（爆栈瞬间）：
  d=4990 TokenList(1tok) tok=\tex_edef:D
  d=4991 TokenList(1tok) tok=\tex_edef:D
  …
  d=5001 TokenList(1tok) tok=\tex_edef:D
```

**判读（推翻前两轮的假说）**：

- ❌ 此前认为「`\edef` 展开**巨型** token 列表（曾见 `TokenList(33426tok)`）」——
  那些巨型帧是**早先的**事件；爆栈段**全部是 `TokenList(1tok)`**。
- ✅ 真相：**`\tex_edef:D` 单 token 纯递归**——深度**单调递增到 5001，从不回落**，
  每一层都是一个「含 `\tex_edef:D` 的 1-token 帧」。

**唯一可能机制**：`\tex_edef:D` 调用自身（每层只推 1 个 token，且就是 `\tex_edef:D`）。

#### 已排除（本轮实测）

| 嫌疑 | 结果 |
|---|---|
| 跨组条件惯用法 `\if_false: { \fi: }` / `{ \if_false: } \fi:`（expl3-code L3801/L3808/L12405 的核心构造）| ✅ **与 pdfTeX 逐字一致**（`[B macro:-> ABC ]`）|
| 深宏递归本身（`\countdown{200}`）| ✅ 两引擎**同样爆**（pdfTeX 也 `input stack size=10000`）——非 NTex 特有 |

#### 附带发现：栈上限差异（次要，非根因）

| 引擎 | 上限 |
|---|---|
| tex.web 源 | `stack_size=200`（L403，**输入源数**）|
| pdfTeX（TeX Live 编译版） | **10000** |
| **NTex** | **5000**（`expand/mod.rs:53`）|

NTex 是 pdfTeX 的一半。对 expl3 深递归代码，这会**提前触发**上限——但**不是**本次爆栈的根因（真因是单 token 无终止递归，10000 也照样爆）。

#### 下一刀（明确）

在 `call_macro` / `exec_def` 的入口加 JSONL note，记录**递归深度与调用者**，直接看
`\tex_edef:D` 自我复制的**触发者**（是 `\__iow_wrap_break:w` 的
`\tex_edef:D \l__iow_line_part_tl { \if_false: } \fi: …`
（expl3-code L12405）在 `\if_false:` 跳过区的行为，还是别的）。

**具体待查构造**（expl3-code L12399-12410）：

```tex
\__iow_tmp:w #1                       ← 外层 \cs_set_protected:Npn，含参数定界
  { \cs_new:Npn \__iow_wrap_break:w
      { \tex_edef:D \l__iow_line_part_tl
          { \if_false: } \fi:        ← `{ \if_false: }` 组 + 组外 \fi:
            \exp_after:wN \__iow_wrap_break_first:w … } }
```

**注意 `{ \if_false: } \fi:` 与 `\if_false: { \fi: }` 的**开关顺序差别**——
本轮只测了后者（`\if_false: { \fi: }`，通过）。**前者（`{ \if_false: } \fi:`）
尚未单独测试**，即 `\edef <cs> { \if_false: } \fi:` 形态——这正是 L12405 的写法。
下一刀先补这个最小用例。

### A1.duodecies ⚠ 第三次误判的更正：`\edef{ \if_false: } \fi:` 行为正确（2026-09-11）

**必须记录**：本轮我判定「`scan_edef_body` 的 `is_skipping()` 分支丢弃 `}` 不减
depth 是爆栈根因」，并依此改了代码（跳过区加组计数）。**该判断被 pdfTeX 证伪**。

**决定性对照**（补全 grep 模式后）：

```tex
\let\if_false:\iffalse \let\fi:\fi
\edef\zz{ \if_false: } \fi:

pdfTeX: [A] + "! Extra \fi." + [B]
NTex  : [A] + "! Extra \fi." + [B]      ← 完全一致
```

**此前误读的原因**：第一次对拍用的 grep 模式没覆盖 `Extra \fi.`，只看到
`[A]`/`[B]` 输出就判为「pdfTeX 无错误」。**这是纯粹的仪器用法错误**，
不是引擎差异。

**已回退**：改动（`scan_edef_body` 跳过区加 `{`/`}` 与 `\begingroup`/`\endgroup`
组计数，41 行）虽通过全量门禁（41 套件全绿），但**没有证据支撑**，故
`git checkout` 回退。教训：**改动必须先有证伪过的 ground truth 支撑再落地**。

#### 本轮对爆栈现场的全部结论（净）

**已确证**（trace 铁证，见 §A1.undecies）：
- step 228182 内 116,850 次压帧，**100% 是 `TokenList(1tok)` + `\tex_edef:D`**
- 深度**单调递增至 5001，从不回落** → **`\tex_edef:D` 单 token 纯递归**

**已排除（本轮新增）**：
- 跨组条件惯用法 `\if_false: { \fi: }` → 与 pdfTeX 逐字一致
- 反序 `{ \if_false: } \fi:` / `\edef\zz{ \if_false: } \fi:` → **两引擎同样报
  `Extra \fi.`**，NTex 行为正确
- 深宏递归（`\countdown{200}`）→ 两引擎同样爆（pdfTeX 亦 `input stack size=10000`）

**下一个应查的方向**（换思路，不再猜语义）：`\tex_edef:D` 的**纯自我递归**意味着
某个 `\edef` 的**参数扫描**不断把 `\tex_edef:D` 送回输入流。可能位置：
`scan_parameter_text`（tex.web 用 `get_token` 纯词法）或 `expand_region` 的
`read_floor` 边界。**建议用 JSONL 记录 `call_macro` 的调用者链**（当前 trace 只记
`push_frame`，未记「谁调用」），这需要给 `call_macro` 加 note 事件。

### A1.terdecies 爆栈根因锁定：`\@@@end` 绑定失败 → `\END` 宏体无限重放（2026-09-11）

**靶子从 4 万行 latex.ltx 缩到 5 行**（关键突破）：

```tex
\documentclass{minimal}
\input{regression-test}
\begin{document}
\START
\END
```

**`\START` + `\END` 即可复现**（不需要 `\TYPE`）。这使 `m3basics001`
（以及 187 例中 111 例 STACK-END）的爆栈成为**可定点调试**的小问题。

#### 机制（trace 铁证）

```
step=4127  d=393  MacroArg(14tok)  tok=\immediate
step=4127  d=393  TokenList(14tok) tok=\immediate
step=4129  d=392  Bytecode         tok=\@@@end      ← \END 宏体里的 \@@@end
step=4130  d=393  TokenList(1tok)  tok=\ifnum        ← \END 被从头重放！depth +1
   …每轮 +1，4986 轮后 input stack size=5000 爆栈
```

累计 token：`\ifnum`(16.5万) / `\immediate`(6万) / `\LONGTYPEOUT`(9千) / `\@@@end`(4.6千)。

**即 `\END` 的宏体被反复重放**，而**`\end` 原语从未执行**
（插桩 `NTEX_END_DBG` 实测 `[END]` 出现 **0 次**）。

#### 根因：`\@@@end` 是 undefined，不是 `\end`

harness（`regression-test.tex` L64-68）的绑定：

```tex
\ifx\@@end\@undefined
  \let\@@@end\end        ← 本应走这里
\else
  \let\@@@end\@@end      ← 实际走了这里（\@@end 存在）
\fi
```

**NTex 里 `\@@end` 的实测状态**：

| 探针 | 结果 |
|---|---|
| `\meaning\@@end` | `undefined` |
| `\ifx\@@end\relax` | 假 |
| `\ifx\@@end\@undefined` | **假**（`\@undefined` 被 shim `\let` 成 `\relax`）|

→ 走了 **else 分支** → `\@@@end := \@@end`（**undefined**）→ `\END` 调 `\@@@end`
= 调一个 undefined cs → **不终止作业**，`\END` 体继续/重放 → 爆栈。

**根因归类**：「`\ifx` 对『未定义 cs』与『`\relax`』的判定」——tex.web 里
LaTeX 的 `\@undefined` 约定依赖二者在 `\ifx` 下**同类**。shim 把 `\@undefined`
设成 `\relax` 后，与真正 undefined 的 `\@@end` **不同类** → 判定翻转。

**注意**：这是 **shim 与 harness 的交互问题**，未必是 NTex 引擎缺陷——
需先确认 **pdfTeX 下 `\ifx\<未定义>\relax` 的真假**（tex.web：`\ifx` 比较
eqtb 槽类型，undefined 槽 vs relax 原语槽 **类型不同应为假**）。若 pdfTeX 亦为假，
则 harness 本就不该走 `\if` 分支——那问题在 shim 该提供 `\@@end`（LaTeX 内核里
`\@@end` 是 **plain `\end` 的别名，存在**）。

**下一刀**：
1. 用 pdfTeX 对拍 `\ifx\@@end\@undefined`（真 LaTeX 环境下 `\@@end` 存在）；
2. 若确认 shim 需补 `\@@end` → 在 shim 中 `\let\@@end\end`（LaTeX 内核同款），
   使 harness 走 else 分支且 `\@@@end` 绑到真 `\end`。

**这是一处 shim 侧的修复，不是引擎缺陷** —— 修正后 5 行最小复现应不再爆栈，
再跑 187 例看 STACK-END 衰减。

### A1.quaterdecies 交接受阻点：`\@@@end` 仍被绑成 `\END`（2026-09-11）

§A1.terdecies 判定的「shim 缺 `\@@end`」已实施（`\let\@@end\end`，且在
`\def\end{...}` **之前**绑原语），但 5 行最小复现**仍爆栈**：

```
[m1dbg/min5.lvt] \message{[M \meaning\@@@end]}
  → [M macro:->\ifnum\currentgrouplevel>0 \LONGTYPEOUT{...}\fi\ifnum\c…
      ↑ \@@@end 的 meaning 显示的是 **\END 自己的宏体**
```

**判读**：`\@@@end` ≡ `\END`，而非 `\end` 或 wrapper。两种可能：

1. harness L83-87 的第二个 `\ifx\@@end\@undefined` 块改了绑定路径
   （该块 body 是 `\def\END{...}`，但 `\if` 判定不同会走 else 变体）；
2. NTex 的 `\ifx` 对 undefined cs 的判定与 pdfTeX 不同，致 harness 走错分支。

**下一刀（明确的 3 步）**：
1. 打印 harness **两条** `\ifx\@@end\@undefined` 的实际走向（两个块各插探针）；
2. pdfTeX 对拍同一 harness 的走向（ground truth）；
3. 据分歧点决定改 shim 还是登记引擎缺陷。

**已确认的引擎侧修复**（保留，独立有效）：
- `process_one` 顶部 + `process_token` 顶部加 `if self.ended { return ... }`
  （`\end` 终结语义；简单用例 `\end` 后 `\message` 不输出已验证 ✅）

### A1.quindecies ⭐ 确凿引擎差异：`\ifx\<未定义>\@undefined` 在 harness 上下文判假（2026-09-11）

**这是本轮最有价值的发现：有 pdfTeX ground truth 的、可复现的引擎级分歧。**

#### 决定性对照（同一 harness，两引擎）

在 `regression-test.tex` 的两处 `\ifx\@@end\@undefined` 各插 `\message` 探针
（`/tmp/m1dbg/rt-probe.tex`）：

| 分支 | pdfTeX | NTex |
|---|---|---|
| `\ifx\@@end\@undefined`（块 1，L64）| **TRUE** | **FALSE** ❌ |
| `\ifx\@@end\@undefined`（块 2，L87）| **TRUE** | **FALSE** ❌ |

**pdfTeX 输出**：`[BLOCK1 TRUE] [BLOCK2 TRUE]`
**NTex 输出**：`[BLOCK1 FALSE] [BLOCK2 FALSE]`

#### 后果链（完整闭环）

```
NTex 判 FALSE
  → 块1 else：\let\@@@end\@@end          （\@@end 非 undefined）
  → 块2 else：\let\@@end\END
  → \END 宏体里的 \@@@end 指向 \@@end（已被改成 \END）
  → 调 \@@@end = 调 \END = **宏体自我重放**
  → 4986 轮后 input stack size=5000 爆栈
```

**pdfTeX 判 TRUE 则**：`\let\@@@end\end`（真终止符）+ `\let\end\END` →
`\END` 末尾 `\@@@end` 正确终止作业 → **exit=0，无爆栈**。

#### 注意：纯 plain 环境下 NTex 的 `\@@end` 判定是**正确的**

```
纯 plain：  \meaning\@@end = undefined
            \ifx\@@end\@undefined = **真**  ✅
            \ifx\@@end\relax      = 假     ✅
```

**即差异只在「harness 上下文」出现** —— 说明有**别的东西在 harness 载入时改了
`\@@end` 或 `\@undefined` 的槽**。shim 侧的 `\let\@undefined\relax` 与
`\let\@@end\end` 均已移除，但判定仍为 FALSE → **嫌疑转向 NTex 的 `\ifx`
对「两个未定义 cs」的比较**，或 harness 自身某条定义链的副作用。

#### 下一刀（精确定位，3 步）

1. **纯 plain + 最小 harness 片段**复现：逐行加 harness 语句，找翻转点；
2. `\tracingcommands`/`\tracingassigns` 追踪 `\@@end`/`\@undefined` 槽的赋值；
3. 对照 pdfTeX 同片段（ground truth 已有 `[BLOCK TRUE]` 可复现）。

**这一处修通，187 例中 111 例 STACK-END 应批量转绿**（`docs/expl3-lvt-scoreboard.md`）。

### A2. 连锁：`\reserved@a` 未定义自引用

```
l.301  \edef\reserved@a{\expandafter\reserved@a\string^^J\@@}
! Undefined control sequence.
```
expl3 失败后 latex.ltx 继续执行暴露的上层连锁，**A1 修好后应自消**，先不动。

---

## B. 引擎侧：已具备（不需重做）

| 能力 | 状态 |
|---|---|
| fmt 构建链路 | ✅ iniTeX 加载 → `\dump` → `.fmt` 存/取 → pass2（TRIP/ETRIP 双 pass 验证）|
| e-TeX 原语层 | ✅ `\numexpr`/`\dimexpr`/`\glueexpr`/`\detokenize`/`\scantokens`/`\everyeof`/`\ifprimitive`/`\interactionmode` |
| plain.tex 预载 | ✅ 1241 行全通（G0–G3，`\newif` 端到端与 pdfTeX 一致）|
| DVI → PDF | ✅ ntex-pdf 成熟（多字体 Type1 嵌入）|
| 错误恢复推进 | ✅ 22838 行可在 ~43ms 吞错处理完 |

## C. 引擎侧：已修的关键语义（速查表）

> 完整根因链/tex.web 锚点见归档文档对应节。此表只为「这坑修过没有」的快速查询。

| # | 修复内容 | commit | 归档节 |
|---|---|---|---|
| 1 | INITEX 初始 catcode 表（`{`=12 判别纯 initex） | — | §8 |
| 2 | 文件通道落盘（`\openout`/`\write<流>`）+ 缺文件错误语义 | — | §9 |
| 3 | `\global` 前缀链 | — | §10 |
| 4 | `<internal dimen>` 扫描臂 | — | §11 |
| 5 | `scan_left_brace` filler 语义 | — | §12 |
| 6 | `\csname` 未定义名 = `\relax` | — | §13 |
| 7 | `scan_number` 内嵌套条件 + 注释行状态 | — | §14 |
| 8 | pdfTeX 引擎伪装（12 探测原语） | a6c499b | §15 |
| 9 | 关系符扫描臂 `get_x_token` 展开 | — | §16 |
| 10 | 宏实参扫描的 `\else`/`\fi`/`\or` 一律数据 | — | §17 |
| 11 | l.398 根因链：四引擎层缺口 + 行模型 catcode | — | §18 |
| 12 | 分隔实参整组贡献 + 定界符按 token 同一 | — | §19 |
| 13 | `\string` `sprint_cs` 语义 + 参数文本 `#{` hash_brace | — | §20 |
| 14 | 0 参数宏定界串匹配 + `\if` 操作数展开 | — | §21 |
| 15 | store_arg 组实参语义（无分隔组实参**存储前剥组**） | 6128722 | §23 |
| 16 | 操作数位嵌套条件重开求值 | — | §24 |
| 17 | `math_display` `$$` 闭合侧接线 | 360c342 | §25 |
| 18 | `\romannumeral` 字母常量后继续展开（expl3 f 型） | — | §26 |
| 19 | unsave 的 retain 守卫（组内局部触碰后 `\global` 被回滚） | — | §27 |
| 20 | 0 参数宏纯定界串在展开上下文漏匹配 | — | §28 |
| 21 | 表达式终结符前瞻 `get_x_token` 化（l.9365 清零） | — | §29 |
| 22 | `\ifx` 补 `\noexpand` 替换臂 | — | §30 |
| 23 | 数字扫描符号循环的条件机推进 | — | §31 |
| 24 | 数字扫描跳过区臂 + e-TeX 表达式三处机制缺口 | — | §32 |
| 25 | 表达式因子/运算符位 `fi_or_else` 消费 + 别名解引用 | — | §33 |
| 26 | `\token_if_*` 生成条件区：组定界别名 cs 不得归一成字面 `{` | — | §34 |
| 27 | l.9386：`\meaning` 补 `\protected` 前缀（expl3 变体降级根因） | — | §35 |
| 28 | `\expanded` 实参 IPN ×256 清零 + `\lowercase` 转 active char | 78dd892 | §36 |
| 29 | `\let` 左侧缺 cs 走 TeX 恢复而非终止引擎 | — | §37 |
| 30 | plain 预载 `\newif` 链（`\meaning` 丢定界符致误判） | c096f8b | §38–41 |

## D. 战役 KPI（corpus-probe）

```bash
~/.venvs/pixtools/bin/python scripts/corpus-probe.py
```

**当前：5/8 PASS + 3 EMPTY**（2026-09-11）

| 样例 | ink | chars | 判定 |
|---|---|---|---|
| latex/sample2e · small2e · testpage | 6148–14044 | 922–2074 | PASS |
| math/basic-expressions · symbols-matrix | 579–985 | 84–166 | PASS |
| plain/plain · letterformat · list | 0–7 | 0–1 | EMPTY（空页）|

> ⚠ **latex 三例的 PASS 是「降级渲染」**：`\documentclass`/`\newcommand`/`\begin`/`\end`
> 全部 Undefined（无 latex.ltx），引擎错误恢复把**裸正文文字**排了出来。
> 结构、字体、版式全丢。**真 LaTeX 渲染未通**，勿据此报喜。
> （`\maketitle` 标题块能出现是因为它退化成了纯文本流。）

**EMPTY 三例** = `plain/*.tex` 无正文无 `\bye`，真 TeX 0 页、NTex 经 `\plainoutput`
收尾冲一页空页（survey §5.bis 发现未修 #1）。

## E. 缺件清单（攻坚前置）

| 项 | 状态 | 取法 |
|---|---|---|
| `latex.ltx` 生成版 | ✅ 在 `/tmp/r27b/`（勿重下） | TL tlnet `archive/latex.tar.xz` |
| `expl3.ltx` / `expl3-code.tex` | ✅ 在 `/tmp/r27b/` | TL tlnet `archive/l3kernel.tar.xz` |
| `latex2e-first-aid-*.ltx` | ❌ 缺 | TL tlnet `archive/firstaid.tar.xz` |
| `article.cls` | ✅ 在 `/tmp/repro2/` | TL tlnet `archive/latex.tar.xz` |
| CM 字体组 TFM（cmr7/9/cmss10/cmbx10/cmti10…）| ✅ `~/.ntex-fonts` 已扩至全家族 | TL `archive/cm.tar.xz` |
| Fandol 中文 OTF | ✅ `~/.ntex-fonts`（8 个） | CTAN `fonts/fandol.zip` |

## F. 方法论纪律（血泪沉淀）

1. **仪器先于结论被验证**。三次仪器失真教训：
   - `corpus-probe` 产物路径假设 → 假阴性 0/8
   - `corpus-probe` 只判存在性 → 空页假阳性
   - **`\meaning`/`\show` 丢参数文本定界符 → 误报 `macro:->`**（害 §38/§39
     两轮定位跑偏，§41 才推翻）
2. **"没炸"≠"对"**。引擎吞错推进会让"跑完/错误计数"失真——**进度判定一律用
   「阻塞点位置单调前移」**，不用跑完与否。
3. **文档待办滞后于实测**。§38 记录的"plain 预载 24 错"在 `cd98a94` 后已消解，
   §39 实测 0 条。**先复测再销账**。
4. **插桩一次拿全数据**，不要"加一行 eprintln 跑一次"。环境变量门控多处同时打点。
5. **改 `@` 类 cs 先 `\catcode`\@=11`**。plain.tex L1239 把 `@` 改回 cat 12，
   预载后 `\if@` 不可访问是**正确行为**（pdfTeX 同样切成 `\if` + `@`）。
6. **math 组生命周期大改会死循环**（290 万步卡 `}`/`$`）——禁直接改
   `MathShift`/组结束/`close_math` 路径；分阶段 + `timeout 200` 验证。
