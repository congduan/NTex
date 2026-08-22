# ETRIP 原语进展清单

> 数据来源：etrip.tex 全量控制序列 vs builtins 注册表对照（2026-08-18 生成），
> 进展随冲刺迭代更新。本文件是 ETRIP 原语状态的**唯一状态源**（plan.md §6 引用）。
> 最近更新：2026-08-23（本轮：\iffontchar/fontchar*/showifs/parshape 族 + 表达式 * / 与溢出恢复）

图例：✅ 完成 · ❌ 未完成

## 总览

| 分组 | 范围 | 进度 |
|---|---|---|
| M4 基线 | e-TeX 核心/扩展原语（M4 验收时已全部落地） | ✅ 全部完成 |
| A 组 | e-TeX 特定原语（pass2 前半段 Checking 段） | ❌ 24/42 |
| B 组 | TeX 基础原语（pass2 中后段） | ❌ 15/36 |
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

## A 组：e-TeX 特定原语（❌ 24/42）

### 显示类
- ✅ `\showtokens`（`<general text>` 展开后显示；80022b4 改走 `scan_group_contents_expanding`）
- ❌ `\showgroups`
- ✅ `\showifs`（2026-08-23：`\showifs` 转储 if 栈；配套 `\currentiftype/level/branch` 只读整数）
- ❌ `\showlists`

### marks 族
- ✅ `\marks`（已注册 → 已接线）
- ❌ `\topmarks`
- ❌ `\firstmarks`
- ❌ `\botmarks`
- ❌ `\splitfirstmarks`
- ❌ `\splittopmarks`
- ❌ `\splitbotmarks`

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
- ❌ `\muexpr`（已注册，暂映射 Glueexpr；mu 单位与 `\the` 显示 "5.0mu" 待校准）
- ❌ `\mutoglue`
- ❌ `\gluetomu`

### 胶水阶
- ✅ `\gluestretchorder`
- ✅ `\glueshrinkorder`
- ✅ `\gluestretch`
- ✅ `\glueshrink`
  （Glue 增 order 字段 + scan_dimen 阶后缀 + `.fmt` v7）

### 惩罚数组
- ❌ `\interlinepenalties`
- ❌ `\clubpenalties`
- ❌ `\widowpenalties`
- ❌ `\displaywidowpenalties`

### 丢弃物
- ❌ `\pagediscards`
- ❌ `\splitdiscards`
- ❌ `\lostchars`（`\savingvdiscards` 相关）

## B 组：TeX 基础原语（❌ 15/36）

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
- ❌ `\copy`
- ❌ `\unvbox`
- ❌ `\unhbox`
- ❌ `\unhcopy`
- ❌ `\unvcopy`
- ❌ `\lastbox`

### 盒子尺寸
- ❌ `\wd`
- ❌ `\ht`
- ❌ `\dp`

### 其他
- ❌ `\tracingparagraphs`
- ❌ `\rightskip`、`\leftskip`
- ❌ `\omit`
- ❌ `\prevdepth`
- ❌ `\interlinepenalty`
- ❌ `\clubpenalty`
- ❌ `\widowpenalty`
- ❌ `\displaywidowpenalty`
- ❌ `\unskip`
- ❌ `\lastpenalty`
- ❌ `\unpenalty`

## C 组：已注册未接线（✅ 全部已接线）

- ✅ `\deadcycles`、`\raise`、`\lower`、`\span`、`\special`、`\jobname`、`\vcenter`、`\marks`、`\vsplit`
- ✅ `\discretionary`、`\insert`、`\vadjust`、`\halign`、`\valign`、`\cr`、`\noalign`、`\mathchoice`
- ✅ `\dump`、`\everyjob`

## 收尾（❌ 未开始）

- ❌ `etrip.log` 逐字节比对（消息格式/上下文行/dvitype 暂不纳入）

## 冲刺纪律

每次迭代前先 `cargo build -p ntex-trip` 确认全绿再跑（避免脏构建旧产物误报，如误报过的
`\ifcase 序号不能为负`）；对照 etrip.log 参考逐段验证，不做整体 diff。
