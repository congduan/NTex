# expl3 官方测试跑分（l3kernel `.lvt`）

> **本文档是 expl3 攻坚的进度仪表盘。** 状态源：`scripts/lvt-run.py`。
> 上游权威数据：`latex3/latex3` 仓库 `l3kernel/testfiles/`（每例配 `.tlg` 期望转录）。

## 第五刀跑分（2026-09-14 主控独立复测）🎉 判定分布翻盘：RAN 0→3，STACK 187→180

| 判定 | 四刀后（09-14 上午）| 第五刀（7 件套装齐后）|
|---|---|---|
| **RAN** | **0** | **3**（m3bitset002 / m3expan004 / m3pdf001）|
| CRASH | 1 | 3 |
| TIMEOUT | 0 | 1 |
| STACK | 186 | **180** |

自 09-11 以来首次出现 RAN。**口径警示**（详见下文 09-11 基线节）：本轮起
expl3 载入链与 pdfTeX 同口径（backend + Unicode 数据 7 件套），与旧基线
不可直接比。STACK 180 = 载入期残差（cond 臂 `\__int_compare:NNw` 级联，
见第五刀节「下一刀」）+ 各用例测试体差异。

## 第五刀（2026-09-14）fp 探针 23→8：UTF-8 单字符 cs 名 + catcode(10) 表项两根因；全 CRASH 悬案破案 = lvt 缺 expl3 载入链件

引擎两处修复（`crates/ntex-core/src/expand/scan.rs` + `catcode.rs`）把 fp 全载探针
（`probes/fp-load/`，纯载入口径）从 **23 键降到 8 键**、exit 0 **跑完全程**；
`make check` 774 全绿。此前停在 23 的两大墙（iow_wrap `^^J` 定界失配、
`` `\^^fe `` Improper）一次清掉。

### 引擎侧两根因（主控猜测面部分证伪）

1. **单字符 cs 名按 UTF-8 字符数判，不按字节数**（scan.rs）。tex.web
   L8742-8744 的 `single_base` 区按**字符**直落；NTex 的
   `single_char_cs`/`try_control_symbol`/`try_scan_backquote` 按字符串长度判，
   `` `\^^fe ``（2 字节 UTF-8 名）误判多字符 → 去 csname 臂 → 报
   `Improper alphabetic constant`。修法 = `chars().count() == 1`。
2. **catcode(chr(10)) 必须是 12（other），不是 5**（catcode.rs）。tex.web
   §1273 INITEX 默认表 LF=12；物理行界由扫描器按**字节身份** `b == b'\n'`
   判定（0925600 既有结论），catcode(0x0A) 只决定行中 `^^J` 解码产物的
   token 身份。设成 5 会让 `\__iow_wrap_fix_newline:w #1 ^^J #2 ^^J` 的
   chr(10) 定界符在**定义位**变 `\par`、**调用位**被吞 —— iow_wrap
   3+2+2 键的根因。表项与测试（`default_table_specials` /
   `initex_table_is_tex_web_1273`）一起钉死。

**证伪记录**：任务书猜测「反引号单字符 cs 探针双引擎一致 ⇒ tokenizer 与此簇
无关」不成立——探针形状只覆盖 ASCII 单字节名，恰是唯一无偏差的子集；
形状对 ≠ 面盖全。scoreboard 上一节「shim harness 脱节」「引擎 outer 判据
过宽」两候选根因**均证伪**（见下）。

### 残差 8 键的归因（次序 = /tmp 探针转录实测）

| # | 签名 | 归因 |
|---|---|---|
| 1 | `Forbidden control sequence … scanning definition of ^^L` | **根因 #3**（本轮不动）：NTex 把 active 字符与单字符 cs 放同一 intern 槽，真 TeX 是 `active_base`/`single_base` **两区**；工程量大已建档 |
| 2-6 | `Missing endcsname` ×1 + `Missing number` ×2 + `\use_i:nn` extra `}` ×2 | **scan_csname 缺条件求值臂**（l.6844 `\__int_compare:NNw` 分派名构造级联，见「下一刀」）|
| 7-8 | `\c_e_fp already defined` + `Undefined control sequence` | 上述级联的降级载入下游污染 |

### 下一刀（已验证、未落地）：scan_csname / scan_file_name 条件求值臂

tex.web 的 `\csname` 名字扫描逐 token 走 `expand()`（文件名扫描同，L10210
`get_x_token`）——**条件原语在扫描内就地求值**：真支字符收进名字、假支就地
跳过。NTex 缺此臂，条件 token 落入不可展开臂报 `Missing endcsname`。两个真
现场（expl3-code.tex）：

- **l.6838-6847** `\__int_compare:NNw`：分派名构造
  `\use:c { __int_compare_ \token_to_str:N #1 \if_meaning:w = #2 = \fi: :NNw }`
  ——`\int_compare:n` 每次比较都走；
- **l.3347-3351** `\__quark_if_empty_if:o`（文件名机器 quark 快路）：展开成
  **不闭合的** `\if_meaning:w \q_nil … \q_nil` 留在流里等扫描器执行。

对照实验已完整：单加 scan_csname 条件臂（照 `exec_expandafter` 的
`step_conditional` + `drain_open_skip` 模式）即 8→1，cond-in-csname 触发 4 次
全部分派名正确（`__int_compare_<:NNw` / `__int_compare_end_=:NNw`）；但连锁
暴露 `\read`/`\readline` 扫描臂缺失（`\__ior_get:NN` = `\tex_read:D #1 to #2`，
l.11922）+ `\the` 偏差，peel 链深度未知 → 当轮按止损纪律 revert。**下一刀
按序：cond 双臂 → `\read`/`\readline` 扫描 → `\the` 偏差 → 根因 #3**。
独立可并行项：实现 `\lastnamedcs`（pdfTeX 原语，l3names l.967 无条件映射）
让 `\cs_if_exist:c` 走 pdfTeX 同路径。

### 仪器侧：lvt 全 CRASH 悬案破案（187 例不是引擎问题）

「09-12 起全 CRASH 待定性」的真根因 = **lvt 临时目录缺 expl3 载入链文件**：
现行 expl3.ltx l.105 `\sys_load_backend:n` 按 `\c_sys_backend_str` 找
`l3backend-<engine>.def`，codepoint 模块载入期还要 `\ior_open` 读 Unicode
数据 —— 缺任一件即 Emergency stop，187 例**全部**误判 CRASH。修法：

- `scripts/lvt-run.py` `EXPL3_FILES` 扩成 7 件套（expl3.ltx/expl3-code.tex/
  l3backend-dvips.def/l3debug.def/UnicodeData.txt/CaseFolding.txt/
  GraphemeBreakProperty.txt/SpecialCasing.txt）；
- 件源 = TinyTeX 同批（LPPL），入库 `fixtures/l3kernel/`（与 09-13 入库的
  expl3-code.tex 同口径）。

跑分纪律：**跑中严禁任何 cargo 编译**——runner 每例重新 exec 二进制，
中途重编 = 污染整轮（本轮实测踩过一次，作废重跑）。

### 重立分布（harness 修复后首轮）

```
【待填：lvt-run6】
```

注意口径：09-11「RAN 180」基线是在**旧 expl3 载入链**（无 `\sys_load_backend:n`）
下取的，与本轮数字**不可直接比**——本轮起 expl3 与 pdfTeX 同口径（backend
载入 + Unicode 数据），STACK/CRASH 的含义随之变化（从「载入即死」变为
「测试体真阻塞」）。首轮分布：m3basics001 = STACK
`输入栈超限（5001 帧 > 5000）……（定义 \__kernel_chk_var_exist:N 的替换文本时）l.458`
——lvt 线下一刀靶子（定义扫描递归无终止，与 fp 线 cond 臂不同源）。

## 09-13 复测·四（载入终点定位）爆栈循环主体 = expl3 quark `\q_stop` 自展开

**本节的分数与上一节相同**（`lvt-run.py --all` 仍 STACK 187/187）——它记的是
**定位推进**：载入的硬阻塞（终点爆栈）**第一次拿到 token 级现场**。探针入库：
`probes/fp-load/probe-load.tex`（纯载入唯一口径，走通才打印 `[LOAD-DONE]`）。

### 仪器侧（先修仪器；事故 #8，见 `docs/tooling-trust.md` §2.7）

`NTEX_STACK_DUMP*` 在宏递归爆栈现场**此前永远不触发**：
- 转储只挂在 `fetch()` 的 `len > MAX_INPUT_STACK` 兜底上，而宏帧守卫
  `call_macro_inner` 用 `len >= MAX_INPUT_STACK` **先命中** → 最需要现场的
  宏/字节码递归爆栈恰恰拿不到转储；
- 逐帧转储取 `stack[..head]`（栈**底**）却按 `n-1-i` 标注成「栈顶帧」——
  **底/顶反转**：栈顶看循环主体、栈底看起点，方向反了结论必反；
- 帧型渲染漏 `Macro`/`MacroArg`/`AlignU`/`AlignV`/`OutputRoutine` 五种
  （全部落 `Other`），而宏递归现场全是 `Macro`/`Bytecode` 帧。

修后新增：`at=`（帧的**当前 pos** = 正在展开哪几个 token；`head=` 只说明
「这是哪一帧」，长宏体上只看 head 会失明）、`NTEX_STACK_DUMP_BOTTOM=N`、
`NTEX_CALL_TRACE=N`（宏调用环形轨迹——栈只含**未弹出**帧，入口帧早已弹出）。
方向性单测：`crates/ntex-core/src/expand/tests_diag.rs`（8 例）。

### 现场（`probe-load.tex`，`57cf379` 之后）

| 量 | 值 |
|---|---|
| 错误数 / `\???` 签名 | 573 / 6 —— 与第三刀基线**逐位一致**（文档数字未漂移）|
| 最远到达 | ≈ l.27,200（l3regex 段；watchdog `Source(pos=937113/1387070)` = 67.6%）|
| 终止形态 | `TeX capacity exceeded, sorry [input stack size = 5000]`，无 `[LOAD-DONE]` |
| 栈型直方图 | `{Bytecode: 4997, MacroArg: 2, TokenList: 1}` |
| 栈顶 60 帧签名 | **60 × `Bytecode[\q_stop end]`**（100% 同一族）|
| 自展开层数 | `\q_stop` **4993 层** |

**入口链（调用轨迹一次给出，可复跑复现）**：

```
… \ior_if_eof:NF \use_i:nn \tl_head:w \c_hash_str \tl_if_blank:nF \use:n
  \__codepoint_data_auxi:w → \__codepoint_data_auxii:w → \__codepoint_data_auxiii:w
  → \cs_set_nopar:cpe → \exp_args:Nc → \q_stop × 4993
```

**定性**：`\q_stop` 是 expl3 **quark**——`\quark_new:N`（expl3-code L3250-3254）
用 `\cs_gset_nopar:Npn #1 {#1}` 把它定义成**自展开宏**，语义上只能当**定界符**
被吞掉（在 TeX 里被当普通宏展开同样会死循环，所以这不是「NTex 怕自展开」）。
NTex 走到了「把它当普通宏展开」的位置：`\exp_args:Nc`（L1511-1512
`\exp_after:wN #1 \cs:w #2 \cs_end:`，即 `c` 变体的 csname 构造路径），
4993 层自复制把输入栈打满。

**已证伪的假设（两次对照电池，双引擎取值逐字一致——别在下一轮重走）**：

| 假设 | 对照形状 | 结果 |
|---|---|---|
| 定界符匹配失败（单 token / 多参数） | `\C p \q_stop`、`\D m;n\q_stop`、`\B a;…;i \q_stop` | 双引擎 `<C:p\|>`/`<D:m\|n\|>`/`<B:a\|i>` **一致** |
| 跨外层组 `}` 继续扫描（L36057 形状） | `\U{ \auxA X lower Y } \q_stop` | 一致 `<A:X\|lower\|Y\|\|>` |
| `;`+空格 多 token 定界（L36055 形状） | `\U{ \auxB p; q; r; } ; \q_stop`、`\U{ \auxC m; n; o } \q_stop` | 一致 `<B:p\| q\| r\|  \| >` / `<C:m\| n\| o  >` |
| `\csname` 内 quark 的差异 | `\csname \q_stop ab\endcsname` | **双引擎都无限循环**（`\csname` 会展开内容）⇒ 非 NTex 独有，**别拿它当靶子** |

**下一刀靶子（按信息量排）**：
1. **`\exp_args:Nc` 的 `#2`（csname 规格）为何带出 439-token 的 MacroArg**，
   且 `\cs_end:` 未被先匹配到——栈底现场 frame 6 = `MacroArg[rem=334/439]`，
   `at=` 显示 `{ grapheme } \ior_close:N \g__codepoint_data_ior \exp_after:wN
   \__iow_wrap_line_loop:w …`（据 pos 推进量推断，被展开的 `\q_stop` 在该
   MacroArg 索引 ~104 处）。即 `\cs_set_nopar:cpe`（L2086-2089 `\__cs_tmp:w`
   生成的**只发射 token 的别名** `{ \exp_args:Nc \cs_set_nopar:Npe }`）→
   `\exp_args:Nc` 这一段。
2. **栈底 frame 0 是 `TokenList[rem=1/1] \par`** —— 一个游离 `\par` 进入了
   `\ior_str_map_inline` 行映射循环。若它破坏了 `;` 字段对齐，
   `\__codepoint_data_auxiii:w`（L35767，9 个 `;` 字段 + `~ \q_stop`）会向前
   扫到 `\q_stop` 才停 —— 与「入口链里 auxiii 紧邻出现」的现象吻合。
3. cctab 集群（522/573 = 91%）仍未定性（本轮未推进）。

## 最新基线（2026-09-13 复测·第三刀）fp 首错三偏差修复 + 第四偏差（数字循环全展开）已定位待落地

| 指标（全载探针 `probe-fp.tex`：exgeneric + expl3-code 全文 + `\fp_const:Nn \c_e_fp`） | 二刀后 | 本刀（落地部分） | 本刀（含实验版第四刀，未落地） |
|---|---|---|---|
| NTex 总错误 | 623 | **573** | **23** |
| `\???` 签名 | 15 | **6** | **0** |
| FP-OK | 0 | 0 | 0 |
| pdfTeX 同探针 | 2 错（自身）/FP-OK 1 | 同 | 同 |

**本刀落地三个真偏差**（各自探针双引擎验证，`make check` 全绿）：

1. **`\mathchardef` 操作数丢负号**（`scan.rs` EqSlot::MathChar 臂）。tex.web
   L8707-8721：scan_int 收尾处**一条无条件** `if negative then negate(cur_val)`
   盖全部取值臂（字母常量/内部量/数值常量）；`\mathchardef` 常量经
   `scan_something_internal`（L8373 `char_given,math_given:scanned_result`）
   流入同一收尾。NTex 只在数值常量臂取负 →
   `-\c__fp_minus_min_exponent_int`（=`\mathchar"2710`=10000）得 +10000，
   fp 舍入机把 −2³⁰ 垃圾指数当真。pdfTeX 探针 t7：取负前 2 错→取负后同点
   0 错。
2. **`\noexpand` cs 在 `\if`/`\ifcat` 操作数位的码值臆测**（`cond.rs` ~L941）。
   第十五刀的 `256+csid`/cat-13 猜测证伪。tex.web L9816-9838：
   `get_x_token_or_active_char` 后「非字符」归一为 `relax/256` 哨兵——`\noexpand`
   只在操作数是 **active char** 时回填字符码（L7509-7517 标记路径 +
   L4527/L6169 token 基址证明 cs 形式 cur_chr≥256 恒为哨兵）。NTex 的
   active char 是带 noexpand 标志的 Char token，走字符臂自然命中；cs 形式
   返回 `(None,None)` 即对齐。pdfTeX 探针 t9 CC2 验证。
3. **`\pdfstrcmp` 实参按原义收集**（`scan_group_contents_xpand(bool)` 新增 +
   expr.rs/primitive_expand.rs 两处 PdfStrCmp 调用点）。tex.web
   `scan_toks(macro_def,xpand)` body 循环（L9378-9391）在每个 token 位先展开，
   `{`/`}` 只进 unbalance；`<general text>` = `scan_toks(false,true)`
   （L21237）。`\__fp_str_if_eq:nn`（= `\pdfstrcmp`，expl3-code L16158）靠
   `\noexpand` 把表达式终结符 cs 原样送进串比较（L17579），组内不展开则串里
   多出 `\exp_not:N` 原语名 → 终结符漏判 → 表达式机级联 extra-}。
   pdfTeX 探针 t12 决定性验证。
   **⚠ e-TeX 特例必须保留非展开**：`\detokenize`/`\unexpanded`/`\scantokens`
   五处调用点继续走 `scan_group_contents_expanding()`（xpand=false，实测
   pdftex `\detokenize{\zzz}` 存 `\zzz` 原义；两条单测钉死）。本轮曾一刀
   全改 xpand(true) → 立刻砸 2 测，已回退。

**第四偏差（已定位、pdfTeX 决定性证实、573→23 的主杠杆）——下一刀靶子**：

- **机制**：tex.web @<Accumulate the constant...@>（L8797-8812）数字循环尾是
  **无条件 `get_x_token`**（L7825：get_next → 可展开则 expand → 重来），即
  数字串后紧跟的宏/可展开原语**就地展开、产物继续累计**，首个不可展开产物
  `back_input`。NTex 数字循环尾停在第一个 cs 放回（第十二轮权宜，报告
  §18 已记偏差），`\<可展开>` 不被吸收。
- **pdfTeX 决定性证据**（INITEX，`\write16` 通道）：
  `\def\zz{4} \count11=2\zz` → `B=24`（**吸收**，非 NTex 的 2）。
- **对 fp 的杠杆**：expl3 fp 数字消化机 `\ifnum 9 < 1 \token_to_str:N #1
  \exp_stop_f:`（expl3-code L16681/L16749）左操作数停在 1、`9<1` 为假 →
  数字被当 other 分派 → `\__fp_parse_one_other:NN`/`\__fp_parse_infix:NN`
  级联错位。实验版补丁（数字循环尾换成无条件展开臂，`Macro` 臂带
  `protected && suppress_expansion>0` 抑制）使全载探针 **573 → 23 错、
  `\???` 6 → 0**。
- **落地阻塞（本轮未过门禁的原因）**：28 项既有 ntex-core 单测钉死了
  「数字后停在可展开项」的旧语义，典型如
  `advance_register_arithmetic`（`\count20=0\advance\count20 1\the\count20`
  期望 `2`，tex.web 真语义 `\the\count20` 被吸收进当前数 → 12）、
  `number_scan_skips_false_branch_of_nested_romannumeral`、
  `scan_left_brace_expandable_filler` 等。其中**部分测试的输入是钉偏差而非钉
  tex.web**（pdfTeX `B=24` 已证），须逐条重算期望值；另有十二/十八/二十二轮
  在数字/表达式扫描周边打磨出的帧收口语义（`\expandafter` 揭示条件开始、
  表达式终结符前瞻 `\relax` 单次吸收）可能与全展开相互作用，须在 TRIP 基线
  签名锚点下重验。**这是独立的第四刀，不是本刀的收尾活**。
- 实验版补丁正文（可整段替换数字循环尾的 `\expandafter` 特例块）见
  本文档末尾附录 A。

**结论（按 90 分钟止损口径）**：fp 首错未达 FP-OK；三偏差修复落地（门禁全绿、
双引擎各自验证）；第四偏差完成定性 + 双引擎证实 + 杠杆量化（573→23），
落地（含 28 测期望值重算与 TRIP 重验）留给下一刀。
`lvt-run.py --all` 复测（本刀工作树）：**STACK 187/187 判定分布未变**——
fp 首错仍阻断 END-TEST-LOG，本刀收益在探针层（错误 623→573、`\???` 15→6；
含实验版 23/0），不到 harness 判定层；按「探针首错前移」口径记刀。

## 09-13 复测·二（历史）首错前移 l.9320 → l.18141——outer 双槽位修复落地

| 判定 | 一刀前 | 本轮 |
|---|---|---|
| STACK | 187 | 187（判定分布未变，首错前移）|

**主控独立复测**（8217f86 + 本刀工作树，2026-09-13 10:40）：单例 m3basics001
转录 4933 行，首错 = `Use of \??? doesn't match its definition`（fp 模块），
总错误 625 → 623；`\???` 签名 15 → 15（非本刀回归铁证）、
`Forbidden control sequence` 2 → 1（靶错消除）。全量 `lvt-run.py --all`
仍 187 STACK（fp 首错阻断载入，到不了 END-TEST-LOG）。

**本轮修复（2026-09-13，定性见 `docs/latex-feasibility.md` §A3）**：上一节
两个候选根因里，**候选 2 成立但机理更具体**——不是 scanner_status 误判，而是
**eqtb 单槽合并了 active char 与同名单字符 cs**：plain.tex L20
`\outer\def^^L{\par}` 写进 active 槽的 outer 被 cs 形式 `\^^L` 继承，expl3
L9320 `\char_set_catcode_active:N \^^L` 实参扫描即报
`Forbidden control sequence`（pdfTeX 同点 0 错）。修复 =
`MacroDef::active_slot` 位 + 统一判据 `is_outer_for_token`（**outer 仅在
token 形式 ↔ 槽写入形式一致时可见**，tex.web L242 双槽语义的压缩替身），
8 处 token 面检查点统一；`.fmt` v16 序列化一字节伴随（codec.rs）。

**效果**：载入首错 char 模块 l.9320 → **fp 模块 l.18141**
（`\fp_const:Nn \c_e_fp { 2.718 2818 2845 9045 }`，症状
`! Use of \??? doesn't match its definition.`——msg 渲染机症状，真偏差在
`\__fp_parse:n`；旧转录同签名已有 15 条，非本刀回归）。全载探针
错误 625 → 623。**判定分布未翻盘**（fp 首错仍阻断 END-TEST-LOG），
按「首错单调前移」口径记一刀；下一刀靶子见 §A3.2。

## 09-13 复测·一（历史）⚠ STACK 187/187——shim harness 与引擎修复批次脱节

| 判定 | 数量 |
|---|---|
| STACK | **187**（全部）|

**定性（2026-09-13 bisect 实测）**：回归**不在 09-13 拉取**——f18a343
（拉取前 HEAD）配同一 harness 同样 STACK（m3basics001 单例探针；main 全量
187/187 STACK）。时间线与已证事实：

- 09-11 深夜跑分板 RAN 180 → **09-12 06:26 `46ab227`**：配当前 harness 已
  STACK（旧签名 `5001 帧 /\l__iow_line_part_tl`，l.25851）——与
  blocker-history 同 ts 的「`\read` 流未打开 REGRESSION」同期；
- 09-12 白天六刀（`261544d..f18a343`，scanner_status/outer/行尾/everyeof）
  后，失败**换签名**：`\q_stop 递归`，转录首错 l.48
  `Forbidden ... \char_set_catcode_active:N`（→ `Improper alphabetic
  constant` 级联，625 错）；
- 窗口内 `scripts/lvt/` 与 `lvt-run.py` **零改动**（diff --stat 为空）；
- **engine 裸跑健康**：main 不载 expl3 直跑 m3basics001 → END-TEST-LOG ✅
  rc=0；`latex_probe --initex latex.ltx` 无爆栈，终点 = iow_wrap（A1.vicies
  记录一致）。

**两个候选根因（均未证伪，待定性）**：
1. **shim 脱节**——09-12 批次只对齐了 latex_probe/单元测试口径，lvt shim
   未同步（正例：09-11 RAN 180 之后 shim 从未随引擎批次回归过）；
2. **引擎 outer 判据过宽**——首错恰是 `Forbidden ... while scanning use of
   \char_set_catcode_active:N`，outer 正是批次动过的语义；pdfTeX 对
   expl3-generic 全程 0 错，若对拍同点 NTex 多报 Forbidden，则是引擎侧
   回归（261544d/63db4b6/c9ca9b4 三刀嫌疑）。

**下一刀**：① 最小探针对拍 pdfTeX（`expl3-code.tex` 载入前 N 行，比
Forbidden 首错点）；② 探针显示引擎差异 → 修引擎；对拍一致 → 修 shim，
重跑全量重立基线。**在重立基线前，本板数字（含 09-11 RAN 180）与引擎
 HEAD 不可比**；期间进展看 `latex_probe` 终点推进 + 376 单测口径。

## 09-11 基线（历史，待 shim 修复后重立）⭐ RAN 180/187 = 96.3%

| 判定 | 数量 | 占比 |
|---|---|---|
| **RAN** | **180** | **96.3%** ✅ |
| CRASH | 6 | 3.2% |
| STACK | 1 | 0.5% |


### 本战役累计（起点 → 现在）

| 判定 | 起点 | 现在 |
|---|---|---|
| STACK-END | **112** | **0** ✅ |
| RAN | **0** | **170** ✅ |

### 三个真修复（都改变了通过率）

| # | 修复 | 效果 |
|---|---|---|
| 1 | shim 重复载入 harness 致 `\END` 自递归 | STACK-END 112 → 0 |
| 2 | shim 补 `\ExplSyntaxOn`/`\ExplSyntaxOff`（LaTeX 内核提供，plain 无）| RAN 114 → 135 |
| 3 | **`\input` 文件名扫描漏收非 Letter/Other catcode**（tex.web L10210 判据为 `cur_cmd>other_char`）→ 路径含 `_` 被截断 | RAN 135 → **170** |

### 剩余 16 例 CRASH 的 8 个簇

| 例数 | 首错 |
|---|---|
| **8** | `! 实参扫描到输入末尾`（m3fp-logic004/m3int001/m3int003/m3prg001/m3skip002/m3skip006/m3tl002/m3tlist002）|
| 2 | `forbidden control sequence \+`（outer 宏出现在展开上下文）—— m3fp-parse002/m3regex005 |
| 1 each | 组未闭合 / `\f` outer / 双重上标 / `\CS 赋值 RHS` / `\CS 需要寄存器参数` |

**⭐ 根因已锁定（1 行复现）**：8 例「实参扫描到输入末尾」

```tex
\immediate\write128{~}      % ← FAIL
\immediate\write128{a}      % ok
\immediate\write128{~a}     % ok
```

**关键事实（推翻了先前 3 行的中间结论）**：
- **不需要 `\ExplSyntaxOn`** —— 裸 `\immediate\write128{~}` 即可复现；
- **不是 `collect_undelimited_arg`** —— 报错来自 `\write` 的参数扫描
  （`io.rs::scan_general_text`，L457+ 的 `fetch()` 返回 None）；
- **规律**：参数组内**只有 active 字符**（`~`，cat 13）时失败；后面跟任何
  字符（`~a`）则通过。

**机制推测**：`scan_general_text` 展开组内容，`~`（active）展开为
`\nobreakspace` 之类 → 压帧 → 该帧读尽后 `fetch()` 立刻返回 None，
**在「展开结果帧刚耗尽、还需读下一个 token」的边界上配对不齐**。

**位置（精确到函数）**：`crates/ntex-core/src/expand/scan.rs::scan_number`（不是
`scan_general_text`）——`exec_write`（io.rs L379-384）先 `scan_number()` 读流号再
`scan_general_text()` 读文本；报错文本 `实参扫描到输入末尾` 来自
`macros.rs::collect_undelimited_arg` L300，只有 `scan_number` 路径经过它。

`\immediate\write128{~}` 的执行序：
  1. `\immediate` 置前缀（primitive_io.rs）
  2. `exec_write` → `scan_number()` 读 `128`
  3. `scan_general_text()` 读 `{~}` ← **此处失败**

#### ⭐⭐ 确凿引擎差异（pdfTeX ground truth）

**最小对照**（纯 plain，无 harness）：
```tex
\catcode`\~=\active
\def~#1{[TIE:#1]}          % active 字符是**带参宏**（harness 里 `~` 正是 `\def~#1{\accent"7E #1}`）
\immediate\write128{~}    % 实参扫描时 `~` 需 #1，输入已耗尽
\immediate\write128{[AFTER]}
\end
```

| | pdfTeX | NTex |
|---|---|---|
| 结果 | `Runaway argument` + 可恢复错误，**继续执行** → 输出 `[AFTER]` ✅ | `! 实参扫描到输入末尾` → **作业终止** ❌ |

**结论：这是错误恢复语义的缺失** —— tex.web 里实参扫描遇输入耗尽报
`Runaway argument`（**可恢复**，`error` + 继续），NTex 当成**致命错**。

**harness 里的表现**：`~` 被定义成 `\def~#1{\accent"7E #1}`（LaTeX 重音宏），
于是任何「`~` 作为 write/参数组最后一个 token」的写法都终止整个测试 ——
8 例 CRASH 全因此。

**修法方向**：参照 tex.web `macro_call` 的实参扫描 EOF 路径（`scan_toks` 的
`Runaway argument` 恢复），把 NTex 的
`Error::invalid_input("实参扫描到输入末尾")`（macros.rs L300/L309）
改为**可恢复**：报 `Runaway argument` 到转录 + 按空实参继续。

**待查**：`scan_number_inner`（L61+）的 token 循环在读完 `128` 后，
为何会走到 `collect_undelimited_arg` 并因 `~`（active 展开压帧）判输入耗尽。

**影响 8 例**：m3fp-logic004 / m3int001 / m3int003 / m3prg001 / m3skip002 /
m3skip006 / m3tl002 / m3tlist002

**旧记录（已被本节取代）**：" + old.split("
")[0].replace("**下一刀（已收窄到 3 行最小复现）**：", "") + "



```tex
\ExplSyntaxOn
\TYPE{~}      % ← 失败；`\TYPE{ ~~ }`、`\TYPE{a~b}` 均通过
\ExplSyntaxOff
```

**规律**：active 字符 `~`（cat 13）**作为实参的最后一个 token**（后面紧跟 `}`）时，
实参扫描取不到 token → `! 实参扫描到输入末尾`。`~` 后有别的字符则正常。

**位置**：`crates/ntex-core/src/expand/macros.rs::collect_undelimited_arg`
L296-310（跳过前导空格后 `fetch()` 返回 None）。推测：active char 展开
（`~` → `\nobreakspace` 之类）压帧后，`fetch()` 在帧耗尽处的 pop/读取配对不对，
使下一个 `fetch()` 误判输入耗尽。

**影响**：8 例（m3fp-logic004/m3int001/m3int003/m3prg001/m3skip002/m3skip006/
m3tl002/m3tlist002）—— 都是 `~` 出现在参数/展开上下文末尾的写法。

#### 原「下一刀」记录（保留）

**旧记录：8 例「实参扫描到输入末尾」**（最大簇，单一根因概率高）。
已收窄到 `m3int001.lvt` L171-176 `\int_to_arabic:n { ( 2+7 ) / 3 }` 一带；
该表达式在 `\ExplSyntaxOn` 下（`_`/`:` 为 letter）展开时与 `\TYPE` 交互出错。

## 历史基线：爆栈修复（2026-09-11 晚）

| 判定 | 数量 | 说明 |
|---|---|---|
| **RAN** | **114** | 跑通 harness（修前为 0）|
| CRASH | 72 | 中途致命错误（下一战场）|
| STACK | 1 | 中途栈超限 |
| ~~STACK-END~~ | **0** ✅ | **修前 112 —— 已全部消除** |

### 根因与修法

**不是引擎 bug，是 shim 设计缺陷**：`lvt-shim.tex` 自己 `\input regression-test.tex`，
而**用例又 `\input{regression-test}`** → harness **载入两次** →
第二次时 BLOCK2（L87-91）走真分支 `\let\end\END`，而 `\END` 宏体末尾的
`\@@@end` 已被 BLOCK1 绑到**当时的 `\end`**（此时已是 `\END`）→
**`\END` 自我递归** → 输入栈爆。

**修**：shim 不再代劳载入 harness（由用例自己载入，与官方 l3build 驱动语义一致）。

**最小复现（修前，5 行）**：
```tex
\documentclass{minimal}
\input{regression-test}
\begin{document}
\end
```

## 为什么改用官方测试套件（2026-09-11）

此前 expl3 攻坚方式是「跑 4 万行 `latex.ltx` → 看错误 → **手写探针猜根因**」。
两个致命缺陷：

1. **没有分母** —— 无法回答「expl3 还差多少」；
2. **探针错误率高** —— 实测一轮内 **3 次误判**（详见 `docs/tooling-trust.md`）。

官方套件解决两者：**187 个可跑用例**（l3kernel，分模块）+ **`.tlg` 权威期望**，
用例由 LaTeX 项目维护，**不需要我们造探针**。

## 怎么跑

```bash
scripts/lvt-run.py --fetch              # 抓 l3kernel 测试（GitHub latex3/latex3）
scripts/lvt-run.py --list               # 列出全部用例
scripts/lvt-run.py m3basics001          # 跑单个
scripts/lvt-run.py --all --jobs 2       # 全量跑分
```

产物：`/tmp/lvt-results.tsv`（每例的判定，跨轮 diff 用）。

## 机制（为什么能跑）

`.lvt` 头部是 `\documentclass{minimal}` + `\input{regression-test}` —— 需要 LaTeX
内核，而 LaTeX 内核依赖 expl3（循环依赖）。

解法：`scripts/lvt/lvt-shim.tex`（**plain 垫片**）提供那几个 LaTeX 符号
（`\documentclass`/`\begin`/`\end`/`\makeatletter`/`\@undefined`），再载入官方
`regression-test.tex`（**纯 expl3 + TeX 原语**，不依赖 LaTeX 内核）。

**踩过的坑**（已修，勿重犯）：
- `\def\end#1{}` 会**遮蔽原语 `\end`**（`regression-test.tex` 的
  `\let\@@@end\end` 要绑原语）→ 用 `\futurelet` 只吞 `{document}` 参数组；
- `\input{\LVTFILE}` **不可**：TeX 的 `\input` 是文件名扫描，不吃 `{}` 分组；
- `\@undefined` 必须 `\let` 成 `\relax`（LaTeX 的「未定义哨兵」约定）；
- **NTex 转录走 stderr**（不是 stdout），跑分器必须两侧都收；
- 判据顺序：**先看 `END-TEST-LOG` 再看致命错** —— 「跑到末尾但中途报错」
  （STACK-END/CRASH-END）是 expl3 引导可用的**强信号**，不等于没跑起来。

## 判据词汇

| 判定 | 含义 | 信号强度 |
|---|---|---|
| `STACK-END` | **跑到末尾**，触输入栈超限 | ⭐ 机制基本可用，只差栈 |
| `CRASH-END` | **跑到末尾**，中途有致命错 | ⭐ 同上 |
| `STACK` / `CRASH` | **中途**终止 | 有早期硬阻塞 |
| `NO-END` | 未跑完且无致命错 | 待查 |
| `RAN` / `PASS?` | 跑完且与 `.tlg` 粗对齐 | — |

## 基线（2026-09-11，187 例）

| 判定 | 数量 | 占比 |
|---|---|---|
| **STACK-END** | **0** | **59%** |
| **CRASH** | 73 | 39% |
| NO-END | 2 | 1% |
| STACK | 1 | 1% |

### 判读（重要）

**59% 的官方用例能跑到末尾** —— 说明 expl3 的宏机制**大部分已能工作**，
NTex 缺失的不是"expl3 基础"，而是**少数几处引擎级偏差**在放大。

**单一最大阻碍 = 输入栈超限**（0/187 = 60%），与 `latex.ltx` 的爆栈
**同一根因**（`\tex_edef:D` 单 token 递归，见 `docs/latex-feasibility.md`
§A1.undecies）。**修掉它，通过率可能量级跃升** —— 这是当前**最高杠杆**的一刀。

## 下一刀（有据可依）

`m3basics001` 是**最小靶子**（162 行，官方最基础的 `\cs_if_exist_use:` 等），
且已定位为 `STACK-END`。用它做定点调试，比 4 万行 `latex.ltx` 高效得多：

```bash
scripts/lvt-run.py m3basics001                    # 看是否 STACK-END
NTEX_TRACE_JSONL=/tmp/m1.jsonl ntex-dvi ...       # 拿调用链
scripts/trace-view.py /tmp/m1.jsonl --spikes      # 找自我复制宏链
```

## KPI 纪律

- **进度指标 = 用例判定分布的变化**（`STACK-END`/`CRASH` → `RAN`），
  不是「latx.ltx 跑到第几行」；
- 每轮跑分后把 `lvt-results.tsv` 与本表对比，**回退必须解释**；
- 与 `make blocker-track` 互补：那个看「单文件阻塞点位置」，这个看「用例通过面」。

---

## ⚠ 指标解读的关键限制（2026-09-11 实测，必读）

**当前的 `DIFF` 数字不能直接读作「引擎语义缺口」。** 原因：

`.lvt` 用例假设 **expl3 已载入**（l3build 用 `--fmt=...latex` 预载 format；
`l3kernel/build.lua` 的 `checkdeps = { }` 印证无需额外包）。但我们的垫片
**没有载入 expl3**——而用例正文调用的是 expl3 kernel 函数：

```tex
\TEST{cs~if~exist~use}{
  \cs_if_exist_use:N   \TRUE          ← expl3 kernel 函数
  \cs_if_exist_use:NF  \TRUE { \ERROR }
  ...
```

实测 `m3basics001` 转录：
```
TEST 1: cs if exist use          ← 标题已对（`~`=cat10 修复后）
! Undefined control sequence.
\cs_if_exist_use:N               ← 函数未定义 → 落字面文本
```

**全 187 例共用到 2595 个不同的 expl3 函数** —— 无法靠垫片补齐（那就是 expl3 本身）。

### 因此当前指标的真实含义

| 判定 | 读作 |
|---|---|
| `RAN` | harness 跑完了（**不代表任何语义正确**）|
| `DIFF` | 输出与 `.tlg` 不符 —— 但**大部分差异来自「expl3 未载入」而非引擎缺陷** |
| **`PASS`** | **真正有意义的目标** —— 需先让 expl3 载入 |

### 正确的推进顺序

1. **先让 expl3 载入**（`\input expl3.ltx` 走通）—— 这正是战役主线目标
2. 载入后重跑 `lvt-tlg-diff`，此时的 `DIFF`/`PASS` 才**真正度量引擎语义**
3. 垫片类修复（`~`=cat10、`\debug_on:n`、变量名去 `_`）是**必要的基础**，
   但**不足以让 PASS > 0**

**教训**：`DIFF 177/189` 这个数字很漂亮（像"只差一点点"），实际是
**「expl3 缺席」的度量**，不是「引擎快好了」的度量。**指标要有正确的读法。**

## 附录 A：第四刀实验版补丁（数字循环尾无条件展开，573→23 未过门禁）

替换 `crates/ntex-core/src/expand/scan.rs` 数字循环尾的
「`\expandafter` 揭示条件开始才展开」整块（自
`// 数字循环尾只对 ...` 注释起至其闭合 `}` 止）为：

```rust
                    // tex.web @<Accumulate the constant...@>（L8797-8812）循环尾的**无条件**
                    // `get_x_token`：展开直到不可展开（实验版，28 项既有测试回归——见
                    // 本文档「第三刀」节）
                    if let Some(csid) = tok.csid() {
                        let expandable = match self.eqtb.slot(self.deref_alias_chain(csid)).clone() {
                            EqSlot::Macro(m) => !(m.value.protected && self.suppress_expansion > 0),
                            EqSlot::Primitive(p) if p.is_expandable() => true,
                            _ => false,
                        };
                        if expandable {
                            let mut expansion = Vec::new();
                            self.expand_once((tok, false), &mut expansion)?;
                            if !expansion.is_empty() {
                                self.push_frame(InputFrame::TokenList { items: expansion.into(), pos: 0 });
                            }
                            continue;
                        }
                    }
```

落地清单（下一刀）：① 28 项单测逐条重算期望值（pdfTeX 逐条裁决，钉偏差的
改输入或改期望，钉 tex.web 的保留）；② 十二/十八/二十二轮的帧收口与
`\expandafter`/表达式终结符语义在 TRIP 基线签名下重验；③ `make check` 全绿。

## GT 采集方法缺陷更正（2026-09-14，第四刀 31 红复盘）

457615a（第四刀完整落地）带进 31 项门禁红灯。复盘结论：**绝大多数红灯不是
引擎吸收语义错了，而是此前采集期望值用的 GT 方法本身有缺陷**——期望值是从
失真通道里抄出来的，第四刀只是把引擎推进到 tex.web 语义后暴露了它们。

### 三条失效通道（全部 pdftex 实测证伪）

1. **宏扰动 typeout 污染**：用 `\def\zz{\immediate\write16{...}}` 夹在
   数字扫描中段采 `\the` 输出。`\write` 实参在宏体内**不经过**数字循环的
   `get_x_token` 现场——扫描早已收口，采到的是赋值后的值。
   `r.tex`（裸形 `\advance\count200by3\the\count200`）GT = `40`
   （旧值 `5` 的 `5` 被 `by3` 之后的数字循环折叠成 `35`→`by35`）；
   `z.tex`（`\the` 藏在 `\write` 里）采到 `8`。**探针形状决定读时序**，
   形状错了真值就换了一个语义。
2. **盒内容通道 kern 残留**：`\showbox` 通道对 `expandafter` 轮的
   `macro:->\chardef\relax` 断言给出过"kern/glue 残留"形态的文本——
   盒通道是排版产物，混入 italic-correction/前次残留，不能当 token 流真值。
3. **plain 格式初表掩盖读时序**：`\lccode`B`=`b\the\lccode`B` 在 plain
   下得 `98`，看似"赋值后才读"；实为 plain.tex 预设了 lccode`B=98，
   **赋值前读**也返回 98，两种时序不可分辨。换 plain 初表为 0 的字符
   （`!`）即证：赋值前读 → `0`。引擎 INITEX 表给 `0` 是对的。

### 判据权威：pdftex 字符流/裸形实测

据此校准 **12 条期望**（`tests_scan.rs` / `tests_expr.rs` / `tests_macro.rs`
/ `tests.rs`），全部先跑 `/tmp/gt9/{v,w,x,y,z,r,p2,j1,j2,s,h}.tex` 取真值：

| 断言 | 旧期望（失真通道） | pdftex GT | 语义 |
|---|---|---|---|
| `\lccode`B=`b\the\lccode`B` | `98` | `0` | 字母常量后读 lccode 在赋值**前**（plain 初表掩盖过） |
| glue plus 形 `\skip1=...pt plus...` | 尾部折叠 | `0.0pt` | 单位尾 optional-space 只回放**首 token**，整个展开留守 |
| glue_order fill 尾 `\ifdim` | `no` | `yes` | scan_keyword `get_x_token` 展开后续 `\ifdim`，对**未赋值** skip6 求值 |
| `expandafter` meaning | kern 残留 | `macro:->\chardef\relax` | 盒通道污染（\meaning 空格为引擎格式化器遗留偏差） |
| `expandafter` 嵌套 | 有输出 | ``（空） | s.tex N1：\else 在 \chardef 目标扫描位展开→缺 cs 插入→`\a` 未绑定 |
| `\advance\count200by3\the\count200` | `8` | `[40]` | 数字循环折叠旧值 `5`→`by35`（裸形 vs write 形） |
| `\multiply` 同形 | `15` | `[175]` | `5`×`by35` |
| `\divide` 同形 | `2` | `[0]` | `35`/`by5` |

`\count2000` 系旧期望（`15`/`2`）是 **e-TeX 32768 寄存器 vs 引擎 256 槽**
的遗留缺口，非吸收语义问题（pdftex N4 = 15）。

### 引擎收窄：1 处

唯一引擎修复 = **`scan_keyword` 补 `get_x_token` 展开**（tex.web
L8239-8260，注释"recursion is possible here"）：关键字匹配位展开可展开
token、不匹配则 `back_input(cur_tok)` + `back_list(字母)`，spacer 仅在
未收字母时跳过。此前引擎在此位不展开 → fill 尾 `\ifdim` 求值时序偏差。
数字循环尾/单位尾/字母常量尾/寄存器索引位的 tex.web 语义
（457615a 已落地）**全部 pdftex 复核无误，保留**。

### 附带发现（归 layout 所有者，本次不动——领地约束）

- `crates/ntex-layout/src/typeset/tests.rs:331` `expansion_inside_hbox`
  期望 `vec![x,y,7]`：`h.tex` GT = `vec![x,y]`（`7` 为 italic-kern 残留
  进盒通道的又一实例）。HEAD 上本红，非本次引入。
- `crates/ntex-layout/src/typeset/tests_math.rs:142` `math_display_formula`
  硬编码 `13*4_736_286/2`（=30 785 859）：pdftex `\number\hsize` GT =
  **30 785 863**（6.5in = 93951/200 pt，sp 圆整 30 785 863.68 → 进位）。
  本次 hsize 修正使其转红。
- 同值双通道输出不一致：`\the\hsize` → `469.75499pt`（与 pdftex 一致），
  而 TRIP 盒宽通道印 `469.75498`——layout 侧格式化器未走 tex.web
  `print_scaled` 圆整。真 trip.log 亦印 `469.75499`。
