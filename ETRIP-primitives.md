# ETRIP 原语进展清单

> 数据来源：etrip.tex 全量控制序列 vs builtins 注册表对照（2026-08-18 生成），
> 进展随冲刺迭代更新。本文件是 ETRIP 原语状态的**唯一状态源**（plan.md §6 引用）。
> **不符规范项待办全集**（ETRIP + TRIP 硬差距 + D 组简化点 + A5，按 P0/P1/P2 优先级）见
> **plan.md §6 待办清单**（2026-08-26 盘点）。
> 最近更新：2026-08-26（不符规范项盘点入待办；`\long`/`\chardef`/`\box`/`\muskip` 确认已实现）；
> 2026-08-23（第二波：表达式 i128/胶水阶语义/未定义恢复/eTeX 32768 寄存器；
> marks 族原语 6 项全实现；第三波：A/B 组剩余原语批量补全——盒子操作（\copy/\unhbox/
> \unvbox/\unhcopy/\unvcopy/\lastbox）、盒子尺寸（\wd/\ht/\dp）、段落/断页参数（\leftskip/
> \rightskip/\prevdepth/\interlinepenalty/\clubpenalty/\widowpenalty/\displaywidowpenalty）、
> 惩罚数组（\interlinepenalties 等 4 项）、列表尾操作（\unskip/\lastpenalty/\unpenalty）、
> 诊断（\showgroups/\showlists）、mu 互转（\mutoglue/\gluetomu）、丢弃物（\pagediscards/
> \splitdiscards/\lostchars）、\tracingparagraphs/\omit；Primitive 枚举 repr(u8)→u16（变体数
> 超 256 防溢出）；全部已接线并编译/测试全绿，联调待 etrip 逐段验证）

图例：✅ 完成 · ❌ 未完成

## 总览

| 分组 | 范围 | 进度 |
|---|---|---|
| M4 基线 | e-TeX 核心/扩展原语（M4 验收时已全部落地） | ✅ 全部完成 |
| A 组 | e-TeX 特定原语（pass2 前半段 Checking 段） | ❌ 41/42（仅 `\muexpr` 待校准） |
| B 组 | TeX 基础原语（pass2 中后段） | ✅ 全部完成（36/36） |
| C 组 | 已注册未接线 | ✅ 全部已接线 |
| 收尾 | `etrip.log` 逐字节比对 | ❌ 未开始 |

> 统计口径：按**单个原语**逐个计数（A 组 11 个清单行 = 42 原语；B 组 8 个清单行 = 36 原语）。

## M4 基线（✅ 全部完成）

M4 数学 + e-TeX 验收时已实现的 e-TeX 原语：

- ✅ `\protected`（\edef/\write 抑制 + .fmt 保留）
- ✅ `\ifdefined`、`\ifcsname`、`\unless`
- ✅ `\numexpr`、`\dimexpr`、`\glueexpr`（整数/尺寸/胶水表达式）
- ✅ `\detokenize`、`\unexpanded`
- ✅ `\eTeXversion`、`\eTeXrevision`
- ✅ `\ifprimitive`、`\scantokens`
- ✅ `\patterns`（Liang 断字语言包）

## A 组：e-TeX 特定原语（❌ 41/42）

### 显示类
- ✅ `\showtokens`（`<general text>` 展开后显示；80022b4 改走 `scan_group_contents_expanding`）
- ✅ `\showgroups`（2026-08-23 第三波：sink 组栈格式化到转录，待 etrip 逐段联调）
- ✅ `\showifs`（2026-08-23：`\showifs` 转储 if 栈；配套 `\currentiftype/level/branch` 只读整数）
- ✅ `\showlists`（2026-08-23 第三波：sink 列表栈递归格式化，待联调）

### marks 族
- ✅ `\marks`（已注册 → 已接线）
- ✅ `\topmarks`（2026-08-23：可展开查询；sink 接口 + HashMap<class,String> 状态；断页轮转）
- ✅ `\firstmarks`（同上）
- ✅ `\botmarks`（同上）
- ✅ `\splitfirstmarks`（同上；vsplit 拆分 marks 暂空，接口预留）
- ✅ `\splittopmarks`（同上）
- ✅ `\splitbotmarks`（同上）

### 输入
- ✅ `\readline`（原始行 + `\endlinechar` 附加）

### 只读整数
- ✅ `\currentiflevel`、`\currentiftype`、`\currentifbranch`（+ `cur_if_type/cur_if_branch` 状态机，`\unless` 取反类型）
- ✅ `\inputlineno`（恒 0 占位）、`\currentgrouplevel`、`\currentgrouptype`、`\lastnodetype`（80022b4）

### 字体字符度量
- ✅ `\fontcharwd`
- ✅ `\fontcharht`
- ✅ `\fontchardp`
- ✅ `\fontcharic`
- ✅ `\iffontchar`
  （2026-08-23：font.rs 增 `char_metric` 默认方法，TfmLoader 读字体表；
  `\iffontchar` 条件码 20；参数越界报 "! Bad character code." 取假）

### 段落形状
- ✅ `\parshape`
- ✅ `\parshapelength`
- ✅ `\parshapeindent`
- ✅ `\parshapedimen`
  （2026-08-23：parshape 访问器语义按 TeX 实证——indent<n>=值[2n-1]、
  length<n>=值[2n]、dimen<n>=值[n]；n≤0→0；越界钳制/奇偶回退）

### mu 表达式/互转
- ❌ `\muexpr`（已注册，暂映射 Glueexpr——表达式算术已全通（i128 中间量、
  四舍五入除法、`\ifnum#4=\muexpr...` 整数上下文）；**2026-09-03 P0 部分校准**：
  layout 端 muskip_params 字段以 mu 数值存，math_to_hlist 按当前 style
  family-2 em/18 转 sp（NodeBuilder 加 `muskip_is_mu: [bool;3]` 标位 + math_em
  链路：family 2 → current_font fontdimen 6 → fallback 10pt），并加 mu_error
  help1 行 "I'm going to assume that 1mu=1pt when they're mixed."。仍待：expander
  端 1mu=em/18 与 `\the` 显示 "5.0mu"——按 1mu=1pt 数值存不重构）
- ✅ `\mutoglue`（2026-08-23 第三波：注册 + 扫描 mu 胶水转胶水，待联调）
- ✅ `\gluetomu`（同上；胶水转 mu 胶水）

### 胶水阶
- ✅ `\gluestretchorder`
- ✅ `\glueshrinkorder`
- ✅ `\gluestretch`
- ✅ `\glueshrink`
  （Glue 增 order 字段 + scan_dimen 阶后缀 + `.fmt` v7）

### 惩罚数组
- ✅ `\interlinepenalties`（2026-08-23 第三波：注册 + 数组存储（penalty_arrays）+ 处理器，待联调）
- ✅ `\clubpenalties`（同上）
- ✅ `\widowpenalties`（同上）
- ✅ `\displaywidowpenalties`（同上）

### 丢弃物
- ✅ `\pagediscards`（2026-08-23 第三波：misc 整数参数 30，待联调）
- ✅ `\splitdiscards`（misc 整数参数 31，待联调）
- ✅ `\lostchars`（misc 整数参数 32，默认 2；`\savingvdiscards` 相关）

## B 组：TeX 基础原语（✅ 36/36）

### 条件
- ✅ `\ifinner`
- ✅ `\ifeof`
- ✅ `\ifvmode`
- ✅ `\ifhmode`
- ✅ `\ifmmode`
- ✅ `\ifvoid`
- ✅ `\ifhbox`
- ✅ `\ifvbox`
  （sink 增 mode_code/box_register_kind）

### 定义/查询
- ✅ `\csname`、`\endcsname`
- ✅ `\mathchardef`（`EqSlot::MathChar` + 越界报错）
- ✅ `\meaning`（可展开）

### 算术
- ✅ `\multiply`、`\divide`（除 0 保持不变；胶水逐分量）

### 宏定义
- ✅ `\edef`/`\xdef`/`\gdef` 体扫描式展开（`scan_edef_body`，80022b4：scan_toks(macro_def, xpand) 语义）

### 盒子
- ✅ `\copy`（2026-08-23 第三波：复制寄存器为节点；支持 `\lastbox` 后续取用，待联调）
- ✅ `\unvbox`（vbox 拆开子节点入当前列表；非 vbox 报错，待联调）
- ✅ `\unhbox`（hbox 拆开子节点入当前列表；非 hbox 报错，待联调）
- ✅ `\unhcopy`（复制后拆开，原寄存器保留，待联调）
- ✅ `\unvcopy`（同上）
- ✅ `\lastbox`（摘下列表尾盒子存入 lastbox_hold，供下一 `\box`/`\copy`，待联调）

### 盒子尺寸
- ✅ `\wd`（读取/赋值盒子宽度；void 报错，待联调）
- ✅ `\ht`（同上；高度）
- ✅ `\dp`（同上；深度）

### 其他
- ✅ `\tracingparagraphs`（2026-08-23 第三波：misc 整数参数 29，待联调）
- ✅ `\rightskip`、`\leftskip`（胶水参数 + `.fmt` v8，待联调）
- ✅ `\omit`（处理器占位，待联调）
- ✅ `\prevdepth`（dimen 参数，待联调）
- ✅ `\interlinepenalty`（整数参数，待联调）
- ✅ `\clubpenalty`（整数参数，待联调）
- ✅ `\widowpenalty`（整数参数，待联调）
- ✅ `\displaywidowpenalty`（整数参数，待联调）
- ✅ `\unskip`（移除列表尾胶水，待联调）
- ✅ `\lastpenalty`（列表尾 penalty 值查询，待联调）
- ✅ `\unpenalty`（移除列表尾 penalty，待联调）

## C 组：已注册未接线（✅ 全部已接线）

- ✅ `\deadcycles`、`\raise`、`\lower`、`\span`、`\special`、`\jobname`、`\vcenter`、`\marks`、`\vsplit`
- ✅ `\discretionary`、`\insert`、`\vadjust`、`\halign`、`\valign`、`\cr`、`\noalign`、`\mathchoice`
- ✅ `\dump`、`\everyjob`

## 收尾（❌ 未开始）

- ❌ `etrip.log` 逐字节比对（消息格式/上下文行/dvitype 暂不纳入）。
  已知差距（2026-08-23 起逐项销账）：
- ✅（2026-09-03）mu_error 恢复消息——help1 行已加
- ❌ `\tracingassigns` 的 `{changing/into}` 行
- ❌ `\the\muexpr` 的 "5.0mu" 显示（expander 端 1mu=em/18 待重构）
- ❌ 错误消息上下文行（"l.N …"）

## 引擎基础设施（2026-08-23）

- eTeX 寄存器扩展：`REGISTER_COUNT` 256 → **32768**（`\count32767` 可用；
  `.fmt` codec 同步；越界报 "! Bad register code (N)." 钳 0）
- **`Primitive` 枚举 `repr(u8)` → `repr(u16)`**（第三波变体数超 256，u8 判别值回绕
  导致编译 ICE；`.fmt` codec 原语编号改 2 字节小端 + 新增 read_u16）
- 表达式核心：i128 中间量（仅最终结果溢出报错）、四舍五入除法、
  `\let\9=\relax` 别名终止、dimen/glue `*`/`/` 括号因子、胶水阶"最后非零项"语义
- 扫描恢复：未定义 cs 当 \relax 继续（未实现原语不致命）、`\dimexpr/\glueexpr`
  作整数操作数、`\count43pt` 寄存器+单位
- 第二波参数体系：`\leftskip/\rightskip/\prevdepth/\interlinepenalty/\clubpenalty/
  \widowpenalty/\displaywidowpenalty`（Params 字段 + `.fmt` v8）；惩罚数组
  `penalty_arrays` 存储 + `SavedValue` 变体；misc 扩至 33（29-32 号）

## 冲刺纪律

每次迭代前先 `cargo build -p ntex-trip` 确认全绿再跑（避免脏构建旧产物误报，如误报过的
`\ifcase 序号不能为负`）；对照 etrip.log 参考逐段验证，不做整体 diff。
