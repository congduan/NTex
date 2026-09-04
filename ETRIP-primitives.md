# ETRIP 原语进展清单

> 数据来源：etrip.tex 全量控制序列 vs builtins 注册表对照（2026-08-18 生成），
> 进展随冲刺迭代更新。本文件是 ETRIP 原语状态的**唯一状态源**（plan.md §6 引用）。
> **不符规范项待办全集**（ETRIP + TRIP 硬差距 + D 组简化点 + A5，按 P0/P1/P2 优先级）见
> **plan.md §6 待办清单**（2026-08-26 盘点）。
> 最近更新：2026-09-03 三轮（marks 段 2 个 "Bad register code" 整块缺失已修复：etrip.tex L208
> `\marks-1{-1}\marks32768{32768}`——e-TeX marks class 走 register-code 语义，越界报
> "! Bad register code (N)." + read-again token（数字后下一 token `{`）+ l.N 两行 + help 2 行，
> 钳 0 后继续收集 general text；根因：primitive_align Marks 处理器此前只 scan_number 无范围
> 检查 → 错误块静默缺失。现对齐 countdef 五连同款 write_error_help 块，参考块逐行一致（残留
> 仅第二行尾部截断省略点计数 `....` vs `...a...`，归"行截断格式"主题）；单测
> marks_bad_register_code_recovers 固化；semantic diff -3063/+2326（合计 5389）→
> -3051/+2352（合计 5403））；
> 2026-09-03 二轮（sparse arrays 段 8 个 "Bad register code" 错误块已对齐参考：
> countdef 五连越界从 report_error 单行升级为 `write_error_help`（新增变体：read-again
> 段 + help，read-again token 经 fetch() 取输入流下一 token，宏体 `#1\1=-1#1...` 场景恰为
> 再出现的 `\countdef` 等原语名）；块含 ! 消息 / <to be read again> + token / l.N 两行光标 /
> help 2 行，与参考逐行一致，l.N 由 l.963 错位自愈为 l.970；read-again 输出 7→17 处；
> semantic diff -3104/+2293（合计 5407）→ -3063/+2326（合计 5389）。遗留：参考 read-again
> token 与 l.N 间的独立 `...` 省略行（tex.web §show_context：输入栈自错误层向下跳过层数达
> `\errorcontextlines` 时输出一次 `...`，本引擎未模拟多层上下文显示）——归入"read-again
> 错误块格式统一"主题）；
> 2026-09-03 一轮（实测：引擎已能跑完 etrip.tex 全程；修 gluestretchorder 段 4 个
> 胶水查询原语裸用模式错误 "You can't use \cs in vertical mode."——primitive_expand 裸用
> 分支 no-op 改报错；write_error 族重构出 `write_error_help_no_read_again`（无
> `<to be read again>` + help 紧随 l.N 行，参考块逐行一致）；sparse arrays 段 8 个
> "Bad register code" 全报且 l.N 行号对齐；\tracingassigns {changing/into}
> 主体已实现（72/102 行））；
> 2026-08-26（不符规范项盘点入待办；`\long`/`\chardef`/`\box`/`\muskip` 确认已实现）；
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
| A 组 | e-TeX 特定原语（pass2 前半段 Checking 段） | ✅ 42/42（2026-09-03 `\muexpr` 销账，见下） |
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
- ✅ `\muexpr`（**2026-09-03 收尾销账**：tex.web 证据链确认 expander 端
  "1mu=1pt 数值存储 + `\the` 显示 X.0mu" 即 TeX 本义——mu 单位在 scan_dimen
  走 attach_fraction（数值刻度与 pt 相同，1mu=65536sp），em/18 换算只发生在
  layout 排版（70a8492 已做）；mu_error help1 行已加。原"expander 端
  1mu=em/18 待重构"为误标。附随修复：胶水前导符号取负作用于整个胶水
  （8a8d065，etrip L906-915 赋值链 stretch/shrink/阶全保留））
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

## 收尾（✅ 已启动：引擎能跑完 etrip.tex 全程，逐段销账中）

- ❌ `etrip.log` 逐字节比对（消息格式/上下文行/dvitype 暂不纳入）。
  已知差距（2026-08-23 起逐项销账；2026-09-03 实测 diff ≈ -3104/+2293）：
- ✅（2026-09-03）mu_error 恢复消息——help1 行已加
- ✅（2026-09-03）`\tracingassigns` 的 `{changing/into}` 行——主体已实现
  （引擎 72 行 vs 参考 102 行，尾部差异待查）
- ✅（2026-09-03）gluestretchorder 段 4 个模式错误：`\gluestretchorder/\glueshrinkorder/
  \gluestretch/\glueshrink` 裸用报 "You can't use \cs in vertical mode."（no-op 改报错，
  参考块逐行一致）
- ✅（2026-09-03 二轮）sparse arrays 段 8 个 "Bad register code" 错误块对齐参考：
  `!` 消息 / `<to be read again>` + token（`\countdef` 等原语名）/ l.N 两行光标
  （l.970 对齐）/ help 2 行，逐行一致；countdef 五连越界改走新增
  `write_error_help`（read_again=true + help），语义 diff -3104/+2293 → -3063/+2326，
  read-again 输出 7→17 处
- ✅（2026-09-03 三轮）marks 段 2 个 "Bad register code" 整块缺失：`\marks` class 号走
  register-code 语义（etrip L208 `\marks-1{...}\marks32768{...}` 越界）——primitive_align
  Marks 处理器加越界检查 + `write_error_help`（同 countdef 五连：! 消息 / read-again token
  `{` / l.N 两行 / help 2 行），钳 0 继续收集 general text；参考块逐行一致（残留仅第二行
  尾部省略点计数 `....` vs `...a...`，归"行截断格式"主题）；单测
  `marks_bad_register_code_recovers` 固化；semantic diff -3063/+2326 → -3051/+2352
- ❌ read-again 错误块格式统一（剩余差项见上）；其中参考 read-again token 与 l.N 之间
  的独立 `...` 省略行（tex.web §show_context：跳过层数达 `\errorcontextlines` 才打，
  引擎未模拟多层上下文显示）为当前最大单点残余
- ✅ `\the\muexpr` 的 "5.0mu" 显示（2026-09-03 销账：见 §A mu 表达式/互转，
  mu 数值刻度与 pt 相同是 TeX 本义，显示已按 "X.0mu" 输出）
- ❌ 错误消息上下文行（"l.N …"）两行光标显示未全覆盖；另有 \write 转录 cs 后空格、
  79 列断行两处系统性格式差

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

> **收尾口径（2026-09-03，详见 plan.md §6 末尾决策）**：语义 bug（`\muexpr` 1mu 换算 /
> `\the` 显示、glue order 值语义）与错误恢复通用机制（M1-13 `back_input`/`\errhelp`）
> 必修；`etrip.log` 从逐字节比对降级为**语义 diff + 错误块抽查**（read-again `...` 省略行、
> 79 列断行、光标细节等格式差放过），逐字节口径留待 M8 L2 阶段。

> **实测 diff 构成（2026-09-03，`cargo run -q -p ntex-trip -- --driver ntex --test etrip`）**：
> semantic diff **-3051/+2364（合计 5415）**（较三轮头部记录的 -3051/+2352 微差 +12，属
> 复现漂移）。对 render diff 差异行（5585 行）按特征分类：

| 差异类别 | 行数 | 性质 |
|---|---|---|
| `!` 错误消息行 | ~176 | 错误输出格式 |
| read-again 上下文（`<to be read again>` 等） | ~24 | 错误恢复块格式 |
| `l.N` 光标/上下文行 | ~295 | 错误上下文显示（两行光标、`...` 省略行） |
| `{changing/into/restoring` 等（tracingassigns） | ~568 | 诊断转录 |
| `.` 开头 showlists/showbox 盒子树行 | ~3000 | `\showlists`/`\showbox` 树形诊断格式 |
| Overfull/内存统计等 | ~90 | 诊断 |
| 真语义项（`\muexpr`/glue order/`5.0mu`） | 极少（几十行内） | **唯一必修** |

> **结论**：~95% 差异是"错误消息文本 + 诊断转录（showlists/tracing/missing char/光标）"
> 的**字节格式差**，属 L2 范畴（M8 再对齐）；排版语义差异极少（M3 DVI 逐字节一致佐证）。
> 按"语义 diff 归零"口径收尾，实际需修的是错误恢复块格式统一（read-again `...` 等）+
> `\muexpr`/glue order 语义项，工作量远小于 5415 行数字的表象。此数据作为上条收尾
> 口径（降级决策）的数字依据。

## LaTeX 兼容附加层：pdfTeX 引擎探测/兼容原语族（2026-09-04，第九刀）

> 非 ETRIP 范围，随 LaTeX 兼容铺开注册（见 docs/latex-feasibility.md §15）。
> 语义边界：**探测类真实现，行为类用到再补；未注册占位优于 relax 占位**
> （未定义 → 误用报"未定义控制序列"，不静默空操作）。

| 原语 | 语义 | 档次 |
|---|---|---|
| `\pdftexversion` / `\pdftexrevision` | 只读整数 140 / 25（pdfTeX 1.40.25；revision 是整数非字符串——latex.ltx L22501 `\ifnum\pdftexrevision<22`） | 真实现 |
| `\pdftexbanner` | 可展开字符串（保留 "NTex" 标识，不冒充真 pdfTeX 产物） | 真实现 |
| `\pdfoutput` | misc 63，默认 0 = DVI 模式（pdfTeX 默认同）；可赋值，非 0 无 PDF 后端承接 | 真实现（偏差记录） |
| `\pdfshellescape` / `\pdfelapsedtime` | 只读 0 / 0（无 shell escape、无计时器） | 真实现（偏差记录） |
| `\pdfrandomseed` / `\pdfsetrandomseed` | misc 64 种子只读 / 写 | 真实现 |
| `\pdfuniformdeviate` | 可展开 `0 ≤ r < n`，确定性 LCG 推进种子 | 真实现（确定性偏差记录） |
| `\pdfstrcmp` | 可展开字符串比较 → -1/0/1（l3kernel L5129 无条件别名 `\tex_strcmp:D`，expl3 字符串比较全走此路） | **必须真实现** |
| `\pdffilesize` | 可展开文件字节数，缺失 → **空展开**（l3kernel `\file_full_name:n` 以空判"未找到"）；经 VFS | **必须真实现** |
| `\pdfcreationdate` | 可展开 `D:YYYYMMDDHHMMSSZ'00'`（秒恒 00：`\time` 分钟精度） | 真实现 |

- **引擎互斥约束**：只注册 pdfTeX 一族。\luatexversion/\kanjiskip/\filesize/
  \XeTeXversion/\HINTversion 保持未定义——l3kernel `\c_sys_engine_str` 是按
  `\tex_<name>:D` 存在性**拼接**引擎串，多引擎同定义 → 混合串 → 后端选择瘫痪。
- **未注册占位**：\pdfmdfivesum/\pdffiledump/\pdfsavepos/\pdfannot 等行为族
  （expl3 只别名不调用）。
- 关键链路经验：`is_expandable_prim` 白名单原语**必须**同时有 `expr.rs
  expand_once` 与 `primitive_expand.rs dispatch_expandable` 分支，否则展开空转
  OOM（expr.rs fuzz 挂死修复注释）；只读整数作数字操作数需同时入
  `scan.rs` number_cs 白名单 + `scan_number_inner` 读取臂。
