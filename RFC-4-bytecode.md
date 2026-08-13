# RFC-4：字节码指令集设计（TeX 宏展开 VM IR）

- 状态：草稿（M0 评审 / M2-1 定稿用）
- 关联：RFC-1（token 8B tagged union / csid / eqtb）；plan.md M2-1~M2-3
- 范围：宏展开阶段字节码的**指令集、编码、编译规则、执行模型、验证规范**。
  不涉及排版节点与渲染。

---

## 1. 背景与目标

宏展开是冷编性能最大开销（50~70%）。M2 的目标是：把宏体从"逐 token 文本替换"升级为
"执行字节码"。本设计是 M2-1 定稿的 IR。

**约束（继承 RFC-1）**：

1. token 为 8B tagged union，**指令操作数可直接内联 8B token 原值**（零解包）；
2. 控制序列 token 只含 csid，等价关系由**展开时**查 eqtb 槽获得——**不烘焙可变绑定**；
3. 宏体双表示：`TokenArray`（语义/`\meaning`）+ `Bytecode`（执行），编译期保持等价，
   双轨 diff 验证（M2-3）。

**目标**：

- 指令集覆盖 TeX 宏体能出现的全部 token 与结构；
- 对"可静态分析"的宏体（平衡组、静态条件）产生结构化、跳转式字节码；
- 对"不可静态分析"的构造（跨宏 `\if`、动态 `\csname`、扫描顺序原语）提供**运行时逃生口**，
  语义与解释器逐位一致；
- 提供反汇编与**往返测试**规范（M2-1 验证项）。

---

## 2. 设计原则

1. **展开/执行分离**：字节码只负责"宏体展开 → 产出 token 流"。
   赋值、建盒、`\par` 等**不可展开**原语，由主循环在拿到 token 后执行——**不进字节码**。
   宏体内的 `\def`、`\hbox` 等一律编译为 `EMIT_CS`（发射 cs token），由主循环处置。
2. **可展开原语 = 运行时协作**：`\expandafter`/`\noexpand`/`\futurelet`/`\csname`/
   `\the`/`\number`/`\string`/`\meaning` 等可展开原语，其操作数来自**当前输入流**（可能越过
   宏边界），编译期无法静态闭合 → 编译为运行时指令，扫描/展开动作由执行器完成。
3. **条件双模式**：
   - **平衡模式**：`\if..\else..\fi` 完整落在单个宏体内 → 编译为结构化跳转（快路径）；
   - **动态模式**：条件跨宏体（`\if` 在某宏打开、`\fi` 在另一宏关闭）→ 编译为
     `IF_*_SCAN`，运行时用**条件栈**扫描跳过（语义与 TeX 完全一致）。
   两种模式共用同一个条件栈，保证嵌套/跨宏正确。
4. **不变性安全**：任何"把 cs 烘焙成特定含义"的优化（如 `CALL_PRIM`）必须有
   eqtb 槽版本证明，否则回退 `CALL_CS`（展开时查表）。
5. **确定性**：单线程语义优先；并行（M6）只复用编译产物，不改指令语义。

---

## 3. 执行模型

### 3.1 输入帧栈（Input Frame Stack）

VM 用经典 TeX 输入栈承载 token 来源，每帧一种：

| 帧类型 | 内容 | 说明 |
|---|---|---|
| `SourceFrame` | 源码缓冲 + 行/列 + 状态 | catcode 查表发生在**取 token 时**（RFC-1 M1-4） |
| `BytecodeFrame` | `macro_id, pc, args[0..9], 输出缓冲` | 本设计核心；pc 为指令字偏移 |
| `TokenListFrame` | `&[Token]` 切片 + 游标 | `\toks`、`\write` 内容、`\edef` 中间产物 |
| `OutputFrame` | 排版列表 | 由主循环维护，非展开层职责 |

### 3.2 主循环（运行时）

```
loop:
  t = fetch_token()                // 从栈顶帧取下一个 token
  if noexpand_pending and t 可展开: 标记为不可展开，清标志      // \noexpand
  if t 可展开 (宏/可展开原语):
     expand(t)                      // 宏→push BytecodeFrame; 原语→运行时执行
  else:
     输出 t 到主流 / 执行不可展开原语
```

字节码帧被 push 后，由 `fetch_token()` 驱动：执行指令直到产生下一个可发射 token，
或 `EMIT_END`（弹帧）。

### 3.3 运行时状态（不进 token，RFC-1 Q1 决策）

| 状态 | 作用 |
|---|---|
| 条件栈 `CondStack` | 每个 `IF_*` push 一项；平衡模式 `FI` 弹、动态模式扫描 `\fi` 弹 |
| 求值栈 `EvalStack` | 存 `IF_*` 的测试结果（bool）/ `\ifcase` 值（u32），供 `BRANCH_*`/`BRANCH_CASE` 消费 |
| `noexpand_pending: bool` | `\noexpand` 语义（影响**下一个取到的 token**，取后清） |
| `csname_buf` | `\csname..\endcsname` 动态名字缓冲（运行时结构） |
| 组层级、模式、寄存器 | 全局状态（RFC-2 覆盖，本设计只标注接触点） |

### 3.4 跳过模式（Skip Mode）

动态模式的条件为假时，运行时进入跳过态：继续执行字节码但**丢弃发射**，同时用条件栈
计数嵌套 `IF_*`，直到 `\else`（深度 1 时）或匹配 `\fi`（深度归零）恢复。帧弹出而条件
未闭合时，**延续到下一帧**扫描（复刻 TeX 跨宏 `\if` 语义）。

---

## 4. 指令编码

### 4.1 字布局

指令 = 1 或 2 个 8B 字，8B 对齐（与 RFC-1 token 同宽，便于同 arena 共存）：

```
word0: [ opcode:u8 | flags:u8 | aux:u16 | imm0:u32 ]
word1: [ imm1:u64 ]        // 仅需要 8B 立即数的指令（如整 token）
```

- `opcode` 0..255，`flags` 留作扩展（如"条件取反 \unless"折叠位、调试断点位）；
- `imm0` 语义随指令而定（u32/i32 偏移/复合位段）；
- token 操作数：`Char(catcode 4b + charcode 21b)` 可压进 `imm0`（25b ≤ 32b）；
  其余 token（`ControlSeq` 等 8B 原值）放 `word1`。

### 4.2 指令长度

| 指令 | 字数 | 编码要点 |
|---|---|---|
| `EMIT_CHAR` | 1 | imm0 = `catcode(4b)<<21 | charcode(21b)` |
| `EMIT_CS` / `CALL_CS` | 1 | imm0 = csid |
| `EMIT_TOKEN` | 2 | word1 = 8B token 原值（通用兜底） |
| `BRANCH*` | 1 | imm0 = i32 字偏移 |
| `BRANCH_CASE` | n | word0 表头 + n 个 8B 目标偏移 |
| 其余 | 1 | 无/少量立即数 |

### 4.3 反汇编器

每个指令必须有对应助记符反汇编格式（`op mnemonic operands`），供：
- 往返测试（见 §10）；
- 双轨 diff 定位（解释器 token 流 ↔ 字节码反汇编）。

---

## 5. 指令集

### 5.A 发射类（产出 token 到输入流）

| 助记符 | 操作数 | 语义 | 运行时行为 |
|---|---|---|---|
| `EMIT_CHAR` | `cat, ch` | 发射字符 token（catcode 固化，RFC-1 §2） | 追加到输出缓冲 |
| `EMIT_CS` | `csid` | 发射控制序列 token | 追加到输出缓冲；是否展开由主循环决定 |
| `EMIT_ARG` | `n` | 发射第 n 个实参（`#n` 替换） | 把实参 TokenArray **切片借用**追加（零拷贝） |
| `EMIT_TOKEN` | `token:u64` | 通用发射（内部标记等） | 追加 |
| `EMIT_END` | — | 宏体结束 | 弹帧 |

> 覆盖说明：catcode 1/2（组）、3（数学切换）、4（对齐 tab）、5（`\par`）、10（空格）、
> 13（active）全部走 `EMIT_CHAR`；active 的展开由主循环查 eqtb 完成（RFC-1 M1-4）。

### 5.B 调用类

| 助记符 | 操作数 | 语义 | 说明 |
|---|---|---|---|
| `CALL_CS` | `csid` | 展开时查 eqtb → 宏则 push 帧，可展开原语则运行时执行，不可展开则发射 | **默认路径**，永不烘焙绑定 |
| `CALL_PRIM` | `prim_id` | 直接派发原语处理函数 | **受限优化**：仅当编译器记录"该 csid 自编译起 eqtb 版本未变"才生成，否则 `CALL_CS`（见 §7） |

### 5.C 控制流类

**求值 + 分支分离**：`IF_*` 只"求值 + push 条件栈"，结果入求值栈；
`BRANCH_IF_*` 消费求值栈并跳转；`FI` 弹条件栈。

| 助记符 | 操作数 | 语义 | 说明 |
|---|---|---|---|
| `IF_TOKEN` | `mode` | `\if`/`\ifx` 类：运行时扫描 2 个 token 比较 | mode 区分 \if(1)/\ifx(2)/\ifcat(3) |
| `IF_NUM` | — | `\ifnum`：扫描数字比较（sp 语义） | |
| `IF_DIM` | — | `\ifdim`：扫描尺寸比较 | |
| `IF_ODD` | — | `\ifodd`：扫描数判奇偶 | |
| `IF_CASE` | — | `\ifcase`：扫描数，push u32 到求值栈 | |
| `IF_CSNAME` | — | `\ifcsname`：扫描名字，push bool | e-TeX |
| `IF_UNDEFINED` | — | `\ifdefined`/`\ifundef`：push bool | e-TeX |
| `IF_*_SCAN` | — | 动态模式：运行时扫描到匹配 `\else`/`\fi`，无编译期目标 | 见 §6 |
| `BRANCH` | `off:i32` | 无条件跳转（字偏移） | |
| `BRANCH_IF_FALSE` / `BRANCH_IF_TRUE` | `off:i32` | 弹 bool，按值跳转 | 消费求值栈顶 |
| `BRANCH_CASE` | `tbl: [i32; k]` | 弹 u32 n，跳到第 n 个 `\or` 目标；越界跳默认 | `\ifcase` 平衡模式 |
| `FI` | — | 弹条件栈 | 平衡模式闭合；与动态 `\fi` 共用栈 |

> `\unless`（e-TeX）由编译器折叠：取反 `IF_*` 的分支极性，或反转 `BRANCH_IF_*`。

### 5.D 可展开原语指令（运行时逃生口）

| 助记符 | 操作数 | 语义 | 运行时行为 |
|---|---|---|---|
| `EXPAND_AFTER` | — | `\expandafter` | 取下一个 token（可能来自**源码流**）展开一次，语义与 TeX 完全一致 |
| `NOEXPAND` | — | `\noexpand` | 置 `noexpand_pending`；下个 token 取到时标不可展开并清标志 |
| `EMIT_CS_NE` | `csid` | 融合指令：`NOEXPAND` + `EMIT_CS` | 微优化，等价性由双轨 diff 保证 |
| `FUTURELET` | `csid, target` | `\futurelet\cs<token>` | 展开至不可展开 token，`\let` 语义赋给 target cs，token 回插流 |
| `CSNAME_START` | — | `\csname` 动态段开始 | 建立名字缓冲，进入"扫描+展开直到 \endcsname" |
| `CSNAME_END` | — | 动态段结束 | 用缓冲建 cs（存在复用/未存在新建，RFC-1 §4），发射 `ControlSeq` token |
| `EXPAND_THE` | — | `\the` | 扫描参数（寄存器/`\font`/`\skip` 等），发射展开 token 列表 |
| `EXPAND_NUMBER` | — | `\number` | 扫描数，发射十进制字符 token（catcode 12） |
| `EXPAND_ROMAN` | — | `\romannumeral` | 同上，罗马数字 |
| `EXPAND_STRING` | — | `\string` | 扫描 token，发射 catcode 12 字符序列 |
| `EXPAND_MEANING` | — | `\meaning` | 发射 `\meaning` 输出 token 序列 |
| `EXPAND_DETOKENIZE` | — | `\detokenize` | e-TeX；等价 `\string` 化 token 列表 |

### 5.E 杂项

| 助记符 | 操作数 | 语义 |
|---|---|---|
| `NOP` | — | 空操作（对齐/调试） |
| `DEBUG` | `tag:u16` | 调试断点锚（release 剔除） |

---

## 6. 条件双模式的编译规则

编译 `\if...\else...\fi` 时，编译器扫描宏体：

```
情形 A（平衡，最内层闭合）：
  \ifX ... \else ... \fi   全部在体内，且体内嵌套条件也平衡
  → IF_X; BRANCH_IF_FALSE L_else; <then>; BRANCH L_fi;
    L_else: <else>; L_fi: FI

情形 B（\ifX 在体内，\else/\fi 缺失，或体内出现未闭合嵌套）：
  → IF_X_SCAN（运行时扫描到匹配 \else/\fi，用条件栈计数）
```

判定规则：递归扫描 body 的 token 序列，维护条件深度；深度清零处若都能找到闭合，
用情形 A，否则体内该点用情形 B。`\ifcase..\or..\or..\else..\fi` 类似，
平衡时生成 `IF_CASE; BRANCH_CASE tbl`。

**为何需要情形 B**：TeX 允许 `\def\a{\ifnum1=1}` + `\def\b{\else\fi}` 跨宏闭合。
字节码无法在编译期预知跨宏配对 → 只能运行时扫描。情形 B 的执行器在跳过态计数
`IF_*`/`\fi`（含其他宏帧），与 TeX 语义逐位一致。

---

## 7. 编译规则（TokenArray → Bytecode）

### 7.1 逐 token 规则

| 输入 token | 产出指令 |
|---|---|
| Char token（任意 catcode） | `EMIT_CHAR` |
| CS token（不可展开原语/普通 cs） | `EMIT_CS` |
| 可展开原语（静态已知） | `CALL_CS`（版本证明后可为 `CALL_PRIM`） |
| `\expandafter`/`\noexpand`/`\futurelet`/`\csname` 动态段/`\the`/`\number`/… | 对应 5.D 指令 |
| 平衡 `\if` 结构 | §6 情形 A |
| 未闭合 `\if` | §6 情形 B（`IF_*_SCAN`） |
| MacroParam `#n` | `EMIT_ARG(n)` |
| 字面 `##`（定义时已还原为 `#`，catcode 6 字符） | `EMIT_CHAR(cat=6, '#')` |

### 7.2 常量折叠（保守，仅无副作用时）

- `\csname 静态名\endcsname` → `EMIT_CS(csid)`，**仅当**名字全为静态字符且
  编译时已能确认该 cs 存在或可安全推迟（否则走 `CSNAME_*`）；
- `\noexpand\X` → `EMIT_CS_NE`；
- `\unless` → 分支极性取反；
- **不做**：`\number`/`\the`/`\meaning` 的常量折叠（依赖运行时状态，折叠有风险，留待 M6 后评估）。

### 7.3 版本证明（CALL_PRIM 的唯一前提）

编译器在 eqtb 槽记录编译时的 `version`；执行器遇 `CALL_PRIM` 时校验当前版本，
不一致则按 `CALL_CS` 语义重派发。该机制与 M5 依赖追踪共用版本号。

---

## 8. 执行器（dispatch loop）

```
fetch_next_token(frame):
  loop:
    instr = decode(frame.pc); frame.pc += len(instr)
    match instr.opcode:
      EMIT_*:      产出到 frame.out；返回该 token（主循环继续）
      CALL_CS:     查 eqtb → 宏:push BytecodeFrame(实参=扫描) / 原语:run_prim / 其他:产出
      IF_*:        运行时求值（扫描操作数），push 条件栈 + 求值栈
      BRANCH*:     消费求值栈并跳转
      FI:          弹条件栈
      EXPAND_*:    run_expandable_prim(instr)
      EMIT_END:    pop(frame); 若栈空返回流结束
```

- 跳过态（动态 `\if` 为假）：执行但丢弃 `EMIT_*` 产物，`IF_*`/`FI` 只更新条件栈；
- 栈空 + 条件栈非空：继续从下一帧扫描（跨宏闭合）；主循环结束时条件未闭合 → 按 TeX
  报 "Extra \fi / Missing \fi" 语义处理。

---

## 9. 与解释器双轨等价（M2-3 依赖本设计）

- 解释器按 token 流展开；字节码按指令执行。两者必须**逐 token 输出一致**；
- 等价性框架输入：TRIP、随机 token 序列、宏包片段（plan M2-3）；
- 差异定位：双轨各出"展开 token 日志"，diff + 反汇编定位到指令。

---

## 10. 反汇编与往返测试规范（M2-1 验证项）

**目标**：`Bytecode --disasm--> 文本 --asm--> Bytecode'`，且 `Bytecode' == Bytecode`（逐位）。

1. **反汇编**：每指令一行 `pc: mnemonic operands`；立即数按编码语义展开
   （catcode/charcode 还原、csid → InternTable 名、偏移 → 目标 pc）；
2. **汇编（asm）**：文本可逆解析回指令流；
3. **往返性质**：对全部指令类别构造用例（含空宏体、9 参数、深嵌套条件、`\ifcase` 多分支、
   `\csname` 动态/静态、`\expandafter` 链），断言逐位一致；
4. **边界用例**：立即数极值（charcode=0x1FFFFF、偏移 ±i32::MAX/2 拒绝越界）、
   对齐填充、`BRANCH_CASE` 空表。

---

## 11. 开放问题（M0 评审）

- **Q-A**：`EMIT_TOKEN`（2 字）是否值得为内部标记另设 1 字节 opcode？取决于内部标记频率；
- **Q-B**：`IF_*` 的求值栈是否需要支持嵌套（`\ifcase` 值上再求值）？
  建议：单值栈 + `BRANCH_CASE` 即够，嵌套条件由条件栈处理；
- **Q-C**：`CALL_PRIM` 的版本校验是否纳入 M5 依赖追踪的粒度（version 计数）？
- **Q-D**：指令对齐选 8B（与 token 同宽）还是 4B（更紧凑）？M2-5 arena 布局时定。

---

## 12. 参考

- RFC-1（token 表示/InternTable/eqtb 版本化）
- D. E. Knuth, *The TeXbook*, Ch. 20–21（宏、条件）
- *TeX by Topic*, "Expansion" / "Conditionals" 章节
- plan.md M2-1~M2-3（IR 定稿、编译器、执行器、双轨）
