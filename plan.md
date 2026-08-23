# NTex 实施计划

> 依据：[idea.md](file:///Users/congduan/Desktop/code/_vibe_coding_/NTex/idea.md) 架构 + 性能决策（字节码预编译 / 深 .fmt / CJK 整形捷径 / 并行 / 增量）
> 原则：**正确性优先、性能架构前置、基准先行**

## 当前进度（2026-08）

| 里程碑           | 状态                                                                                                                                                                                                                                                                                                         |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| M0 地基         | ✅ 完成（workspace/CI/TRIP·diff·bench 工具链；RFC-1/RFC-4 定稿）                                                                                                                                                                                                                                                      |
| M1 展开内核       | 🟡 核心完成：M1-1\~7、M1-9\~11 已实现（95 用例）；M1-8 分隔参数、M1-13 错误模型、**M1-14 TRIP 冲刺** 待补                                                                                                                                                                                                                              |
| M2 字节码        | 🟡 双轨完成（100 用例等价）；**性能 P0 补课落地**（字节码 u64 原始字执行器 + release 调优，80022b4）；吞吐 ≥2x 待重测（ntex 驱动基准死循环，见 backlog P1）；M2-5 arena 未做 |                                                                                                                                                                                                                             |
| M3 排版核心       | ✅ M3-1\~M3-4 完成（58+ 用例）；**M3-5 DVI 写出 +** **`\shipout`** **+ 断页 DP + lig/kern +** **`\sfcode`** **+** **`\output`** **例程/box255** 完成（dvipdfmx 验收 + 与 TeX 差分对照）；**RFC-3 VFS + 副作用模型落地**（10 原语走 `ntex-io` VFS，延迟写入 shipout 边界提交）；**`.fmt`** **v1 内存快照**（`ntex-format` 确定性编码 + roundtrip）                     |
| M4 数学 + e-TeX | ✅ **全部完成**：数学模式状态机（`$`/`$$`、8 类原子、spacing 表、上下标、字阶）、分式/根式/定界符、样式原语、fontdimen 数学参数 + 数学字体族、显示数学细化、Liang 断字、错误模型、e-TeX 核心 + 扩展（`\protected`/`\ifdefined`/`\ifcsname`/`\unless`/`\numexpr`/`\detokenize`/`\unexpanded`/`\eTeXversion`/`\dimexpr`/`\glueexpr`/`\ifprimitive`/`\scantokens`）；验收 **ETRIP 全绿** 待冲 |
| 输出端           | 🟢 正式 PDF 后端可用（`ntex-pdf`：DVI → PDF 直出 + Type1 嵌入，替换临时 Helvetica 切片；demo 两页与 dvipdfmx 渲染一致）                                                                                                                                                                                                                |

**下一步**：**ETRIP 冲刺**（管线已通，pass2 逐段推进）+ 性能 backlog **P1**（热路径消分配 / 修通 ntex 驱动基准补测吞吐）。

***

## 性能优化 backlog（2026-08-22 评审）

> 来源：高级 Rust 工程师评审结论（热路径实现层欠账 + 工程化闭环），不引入新功能，
> 属既定里程碑补课。纪律：**先量化再冲 M5/M7**——不在 1.12x 的基线上做增量层。

### P0 构建配置（零成本 10~30%，最先做）—— ✅ 已提交 80022b4

- [x] 工作区 `[profile.release]`：`lto = "thin"`、`codegen-units = 1`、
  `panic = "abort"`（已确认全库无 `catch_unwind`/无 `unsafe`，可直接上）
- 验证：~~每千 token 吞吐提升入档~~ 吞吐量化受阻（ntex 驱动基准死循环，见 P1），
  待修通后补测

### P0 M2-6 补课：字节码执行器走 u64 原始字（RFC-4 设计落地）—— ✅ 已提交 80022b4

- [x] `Bytecode.code` 改存 `Arc<[u64]>` 原始字（不再存 16B `Instruction` 枚举，
  现 `encode`/`to_words` 只用于序列化，热路径零解包设计被浪费）
- [x] `fetch()` 的 Bytecode 分支按 `word >> 60` 分发：tag 0..=3 直接
  `Token::from_raw(word)`，消除每步解包
- 验证：M2 双轨等价测试全绿 ✅；吞吐 ≥ 2x 待重测（现基准死循环，见 P1）

### P1 M2-6 小步优化：热路径消分配（每 token / 每宏调用）

- [ ] `process_token`：`eqtb.slot(csid).clone()` → 按引用 match
  （省每控制序列 token 的 EqSlot 克隆，含两次 Arc refcount）
- [ ] `fetch()` 的 EmitArg/宏参数分支：实参 `Arc<[Token]>` 直接复用，
  帧加 `noexpand` 标记位，去掉 `Vec<(Token,bool)>` 重包装 + 二次堆分配
- [ ] `unread`/`next_is_math_shift`：单 token 回推改栈上内联槽，
  去掉每次 `Arc::from([...])`（`$$` 检测每命中一次）
- [ ] `call_macro` 实参 `Vec<TokenArray>` → SmallVec（实参 ≤9 个）
- 验证：M1 全部用例双轨重跑全绿；火焰图对比热区前移

### P1 正确性加固：panic 审计 + fuzz（引擎契约：任意畸形输入不 panic）

- [ ] 生产路径 unwrap/expect 审计（input.rs 10 / ntex-format 15 / ntex-io 6 等），
  输入可达路径一律改 `Error`（与 M1-13 错误模型收尾联动）
- [ ] `cargo-fuzz` 目标：随机字节喂 `Expander + scan_token`，断言永不 panic
  （复用 TRIP/ETRIP in-process 驱动）
  - 已发现实例（2026-08-22，P0 验证时）：`ntex-bench --driver ntex --bench
    expand-throughput`（200k 宏调用）在 in-process 引擎**死循环**（HEAD 与
    P0 改动后均复现；CI 仅 stub 驱动未覆盖此路径）——优先列入 fuzz 回归集
- 验证：fuzz 长时间运行零 panic；错误路径输出与 pdfTeX 一致

### P2 工程化闭环：CI/基准门禁

- [ ] `make bench` 补 `--release`（现为 debug + stub，数字无参考价值）
- [ ] nightly perf job：release 跑 `ntex-bench`（真实驱动），对照入库基线，
  吞吐下降 >15% 即失败（plan 中"基准数字入库，CI 防回归"落地）
- [ ] token 级微基准引入 divan（秒级宏基准保留手写 measure）
- 验证：CI 绿且基线数字可追踪

***

## 0. 三个策略性结论（决定排期顺序）

1. **字节码编译要早做，不能放到最后**。它是冷编性能的最大蛋糕（确定性 2\~5x），
   且影响 `.fmt` 格式与求值图的设计——越晚做，重构成本越高。
   → 排在解释器跑通 TRIP 之后立即做。
2. **`.fmt`** **分两代**。v1（内存快照）在排版阶段即可用；v2（快照 + 字节码 + 部分求值）
   在增量层就绪后升级。格式设计必须预留扩展位。
3. **副作用隔离（VFS + shipout 边界）在 M3 就要定型**，否则增量层（M6）无法落地，
   而它是相对 cTeX 的胜负手。

***

## 1. 里程碑总览

| #  | 里程碑        | 核心交付                               | 验收标准（Exit Criteria）                   |
| -- | ---------- | ---------------------------------- | ------------------------------------- |
| M0 | 地基         | 仓库/CI/基准框架/TRIP 测试框架               | 基准脚本与测试框架跑通                           |
| M1 | 解释器内核      | Token+16 catcode+宏展开+核心原语          | **TRIP 通过**                           |
| M2 | 字节码编译      | 字节码 IR + 编译器 + 执行器                 | TRIP 仍绿；展开吞吐 ≥ 解释器 2x                 |
| M3 | 排版核心       | 节点/盒子/胶水/Knuth-Plass/TFM/断页/DVI 输出 | 与 pdfTeX 折行结果一致；简单文档 DVI 差分一致         |
| M4 | 数学 + e-TeX | 数学模式/e-TeX 原语/断字/错误模型              | **ETRIP 通过**                          |
| M5 | 增量计算       | 求值图/依赖追踪/副作用隔离/VFS                 | 改 1 处仅重算受影响段落（基准可量化）                  |
| M6 | 并行         | 段落级并行展开/布局/整形                      | 8 核加速比 ≥ 3x（300 页基准）                  |
| M7 | .fmt v2    | 三层 fmt + 部分求值 + 项目级 fmt            | latex.ltx 加载 <200ms；ctex 300 页冷编 <10s |
| M8 | 渲染/输出      | PDF 后端 + Skia + WASM 预览            | 简单文档 PDF 逐字节一致；WASM 演示可用              |
| M9 | 生态冲刺       | ctex/xeCJK/OpenType/CJK 捷径         | 目标宏包 CI 回归全绿                          |

**依赖链**：M0 → M1 → M2 → M3 → M4 → M5 → M6 → M7 → M8 → M9
（M5 依赖 M2/M3；M7 依赖 M2/M5；M6 可与 M5 部分并行开发）

***

## 2. M0 地基

**目标**：一切后续工作的脚手架，**基准先行**。

- [x] 初始化 Rust workspace（crate 划分见 §8）
- [x] CI：单元测试 + TRIP/ETRIP + 差分测试 + 基准（跑基准防回归）
- [x] **基准集**（所有里程碑复用）：
  - [ ] `latex.ltx` 全量加载耗时（M7 前无意义，未建）
  - [ ] 300 页中英混排文档冷编总时长（M7 前未建）
  - [x] 每千 token 展开吞吐（对照 pdfTeX 基线）
  - [ ] 增量场景：改 1 字 → 重算耗时（M5 前未建）
- [x] TRIP/ETRIP 测试框架：自动跑 `\input trip` 并 diff 输出（框架就绪，trip 全绿待 M1-14）
  —— **ETRIP 冲刺管线已跑通**：fixtures 获取（CTAN knuth dist + TeX Live 镜像，
  `scripts/fetch-trip-fixtures.sh`）+ harness 泛化（`ntex-trip --test trip|etrip|both`）+
  **ntex 引擎驱动**（in-process Typesetter，产出 .log/.typ，首个错误即停）；
  首次实跑：TRIP/ETRIP 均在首个未支持构造处停下（错误上下文/转录能力待补）
- [x] 差分测试工具：同一 .tex 分别跑 pdfTeX/XeTeX 与本引擎，diff DVI/log
- [x] **设计文档（RFC）先行**，评审通过再编码：
  - RFC-1 Token 表示与内存布局 —— **已定稿**（8B token / TokenArray / InternTable / eqtb 版本化）
  - RFC-2 不可变状态模型与版本化（CoW 结构选型）—— 未开始
  - RFC-3 副作用模型（VFS + 输出边界提交）—— **已定稿**（M3 收尾落地；见 [RFC-3-side-effects.md](RFC-3-side-effects.md)）
  - RFC-4 字节码 IR 草案 —— **已定稿**（M2 直接落地为定长 u64 指令）

**RFC-1 已定决策**（后续里程碑直接引用，不再反复讨论）：

1. **Token = 8 字节 tagged union（u64）**，不用 16B；源码位置走独立 side-table，不进 token；
2. **控制序列 token 不携带定义**：token 只有 csid，等价关系一律查 eqtb 槽
   （`Undefined/Macro{version,def}/Primitive/Register/Alias`）；
3. **宏体 = 连续不可变 TokenArray**（非链表），另配字节码双表示；
4. **InternTable 线性化**：csid=u32 下标，`.fmt` mmap 后零字符串查找；新建 cs 走原子追加 + 版本切换；
5. RFC-1 遗留开放问题 Q2\~Q4（保留位用途/内部标记拆分类/InternTable CoW 粒度）在 M0 评审会一并拍板。

**关卡**：四份 RFC 评审通过；基准与 TRIP 框架可一键运行。

***

## 3. M1 解释器内核（正确性第一，不做性能）

**目标**：按 RFC-1 数据模型跑通 TeX 展开语义，通过 TRIP（硬口径）。

### 实施步骤（依赖驱动，每步独立可验收）

**M1-1 Token 类型骨架**（RFC-1 §3）

- [x] `Token` 枚举：`#[repr(transparent)]` 包装 u64，变体与位分配严格按 RFC-1：
  `Char(catcode 4b + charcode 21b)` / `ControlSeq(csid 32b)` / `MacroParam(num 4b)` /
  `EndGroup` / 保留 tag 5..15
- [x] `Debug`/`Meaning` 输出（对照 TeX `\show` 输出格式）
- 验证：单元测试覆盖 RFC-1 §3 变体表 + §8 用例 4（BMP+ 字符）、5（active char 的 meaning）、8（往返一致性）

**M1-2 InternTable（csid 驻留）**（RFC-1 §4）

- [x] 名字去重表：`name → csid`，csid=u32 数组下标
- [ ] `\csname..\endcsname` 动态建 cs：命中复用、未命中追加（不可变版本切换）——待补
- [x] `\meaning` 名字查询走 InternTable
- 验证：用例 1（动态建名含空 csname、非法字符）；并发预留（M6 再做原子追加）

**M1-3 eqtb 槽版本化**（RFC-1 §5）

- [x] eqtb 槽枚举：`Undefined/Macro{version, def}/Primitive(prim_id)/Register/RegisterIndex/Alias(csid)`
- [x] `\let\a\b` → `Alias(csid)` 间接（不复制宏体）；链式别名
- [ ] `\chardef`/`\mathchardef` → `RegisterIndex` 语义——待补
- 验证：用例 2（链式别名、`\let` 到 `\outer`）、6（`\ifx` 对 Alias 不展开）

**M1-4 输入与 catcode 固化**

- [x] 字节流输入 → 16 种 catcode 表查询 → 生成 `Char` token，**token 内固化当时 catcode**
- [x] catcode 动态修改原语（`\catcode`）只影响后续输入，不回写已生成 token
- [x] active char（catcode 13）生成后查 eqtb 展开
- 验证：与 pdfTeX 对照"改 catcode 后已读 token 不受影响"的行为

**M1-5 宏定义与 MacroDef**（RFC-1 §5）

- [x] `MacroDef { params: ParamSpec, body: TokenArray }`，body 为连续不可变切片
- [x] `\def`/`\edef`/`\gdef`：定义时捕获 token 数组；`\edef` 定义期全展开
- [x] 宏体内 `#` 三态：`#1` 参数槽 / `##` 字面 / 非法 `#` 报错（RFC-1 §8 用例 5）
- [ ] `\newcommand`（经 `\def` + 存在性检查语义）——未实现
- 验证：用例 5、7（分隔串按 token 序列匹配——分隔参数待 M1-8）

**M1-6 展开主循环**

- [x] "读 token → 可展开则展开（循环至不可展开）→ 节点入队 / 原语执行"
- [x] 可展开/不可展开二分表（含 `\protected` 语义占位，e-TeX 在 M4 补全）
- [x] 宏调用 = 查 eqtb → 展开 MacroDef（M2 已换字节码）
- 验证：TRIP 基础部分逐步点亮（待 M1-14）

**M1-7 扫描顺序原语**

- [x] `\expandafter`/`\noexpand`/`\futurelet`/`\aftergroup`/`\afterassignment`
- 验证：与 pdfTeX 对照这些原语的交互行为（这是最易翻车处，专项测试）

**M1-8 参数匹配**

- [ ] 无分隔参数：`#1..#9` 实参收集（平衡组规则）——已实现（无分隔部分）
- [ ] 分隔参数：分隔串按 **token 序列**匹配（RFC-1 §8 用例 7）——待补
- [ ] `\long` 与"参数中禁 `\par`"错误语义——待补
- 验证：嵌套宏实参传递用例集

**M1-9 条件原语**

- [x] `\if`/`\ifnum`/`\ifdim`/`\ifx`/`\ifcase` + `\else`/`\fi`
- [x] **惰性求值**：未走分支不展开（跳过 token 流）
- [x] `\ifx` 比较规则：Char 比 (catcode,char)，CS 比同一 csid（RFC-1 §2）
- 验证：含嵌套 `\if` 与跨宏条件用例

**M1-10 寄存器与内部量**

- [x] `\count`/`\dimen`/`\skip`/`\toks` + 赋值原语 + `\the`（`\box`/`\muskip` 未实现）
- [x] 内部量表示：scaled point（sp，2^-16 pt）定点数
- [x] 寄存器读写走 eqtb 槽（版本化，为 M5 依赖追踪铺路）
- 验证：`\the\count`/`\the\dimen` 输出与 pdfTeX 逐位一致（单位换算已校准）

**M1-11 组与作用域**

- [x] `{...}`/`\begingroup...\endgroup`：组内赋值组尾回滚（朴素快照回滚）
- [ ] 朴素快照回滚 → eqtb 版本指针（O(1)）——未做
- 验证：`\global` 与非全局赋值回滚行为对照

**M1-12 模式状态机（空壳）**

- [x] 垂直/水平/数学/内部 四种模式 —— 被 M3-2 取代：模式状态机实现在 `ntex-layout::typeset`（Vertical/Horizontal/RestrictedHorizontal）
- [ ] 数学模式——M4 填实
- 验证：模式切换错误信息与 pdfTeX 一致

**M1-13 错误模型**

- [ ] 错误上下文输出（"! ..." + 上下文行）+ 四种交互模式（`\batchmode` 等）——待补
- 验证：构造错误用例，输出与 pdfTeX 逐字符一致

**M1-14 TRIP 冲刺**

- [ ] 涉及排版/字体的部分用 `\hbox` 兜底占位（硬口径：语义 bug 绝不带进 M2）——**未完成，M1 大门未关**
- [ ] **TRIP 全绿**（输出 diff 可读化脚本已在 M0 就绪）
- 验证：`\input trip` 输出与参考文件一致

**M1-15 性能基线**

- [x] 记录每千 token 展开吞吐基线（**不做优化**，仅存档，供 M2 对照）

**验收**：TRIP 通过（硬口径）；错误行为与 pdfTeX 一致。
**风险**：TRIP 是"实现后才知道哪错"的黑盒 → 提前做好 TRIP 输出 diff 的可读化。
**现状**：M1 核心（1\~7、9\~11）已实现；M1-8 分隔参数、M1-13 错误模型、M1-14 TRIP 冲刺待补——TRIP 未全绿前 M1 验收项保持未勾选。

***

## 4. M2 字节码编译（冷编性能第一刀）

**目标**：宏展开从"token 替换"升级为"执行字节码"，语义严格等价（双轨 diff 保证）。

### 实施步骤

**M2-1 字节码 IR 定稿**（RFC-4 草案落地，对接 RFC-1）

- [x] 指令集定稿：定长 u64 指令 `Emit{token}`（token 8B 内联）/ `EmitArg{n}` / `End`
- [x] 操作数编码：**token 以 8B 原值内联**（RFC-1 布局零解包）；cs 以 csid 索引 eqtb
- 验证：IR 编码往返测试（指令流 → 反汇编 → 等价）

**M2-2 宏定义期编译器**

- [x] `MacroDef.body: TokenArray` → 字节码（`#n` 参数槽 → `EmitArg(n)`）
- [x] 常量条件折叠：平衡 `\iftrue/\iffalse..\else..\fi` 编译期求值，死分支不输出
- [ ] `\expandafter`/`\futurelet` → 专用指令（保持扫描顺序语义）——未做（仍走运行时）
- [ ] 编译失败路径：非法宏体（保留报错语义，不静默降级）——未做
- 验证：每个编译产物与解释器逐 token 展开结果相等（单元级 diff）

**M2-3 字节码执行器 + 双轨并存**

- [x] 字节码解释循环（dispatch loop），执行 `MacroDef.code`
- [x] 解释器（M1）/字节码（M2）双轨，`Expander::new_interpreter()` 切换
- [x] **等价性框架**：全部用例自动双轨重跑断言输出一致
- 验证：M1 全部用例在字节码路径重跑全绿

**M2-4 原语 dispatch 表**

- [x] `prim_id → 处理函数`（jump table）：Rust `match` 派发（等价位跳转表），`Primitive` 槽命中即派发
- [x] 原语表与 InternTable 预注册（`register_builtins` 建立的内建 cs）
- 验证：全量原语冒烟测试

**M2-5 内存落地**

- [ ] token/指令分配：bumpalo arena（整段分配、无逐项 malloc）——未做
- [ ] TokenArray/Bytecode 同 arena 共存，`.fmt` 序列化友好布局（M7 复用）——未做
- 验证：arena 泄漏/越界（Debug 断言）+ 长文档稳定性

**M2-6 性能达标与定位**

- [x] **基准**：每千 token 展开吞吐——实测 **1.12x**，**未达 2x 目标**（记录入库）
- [x] **字节码 u64 原始字执行器 + release 调优已落地**（2026-08-22，backlog P0 提交 80022b4）
- [ ] 火焰图定位热点（预期：eqtb 查询 / 实参拷贝 / arena 边界）、eqtb 槽缓存行布局、
  实参零拷贝（切片借用）等——转 backlog P1 热路径消分配
- 验证：基准数字入库（M0 基准集），CI 防回归——吞吐重测待 ntex 驱动基准修通（见 backlog P1/P2）

**M2-7 双轨框架移交**

- [x] 双轨等价测试框架保留并文档化（M7 升级 .fmt 时继续使用）
- [x] 决策点：字节码成为默认执行路径（`Expander::new()`），解释器退为调试工具

**验收**：TRIP 在字节码路径全绿——**未达成（继承 M1-14 缺口）**；吞吐基准达标（≥ 解释器 2x）——**未达成（1.12x）**。
**关卡**：字节码与解释器双轨的等价性测试框架保留到 M7（防 .fmt 升级回归）。

***

## 5. M3 排版核心

**目标**：能产出与 TeX 一致的页面，输出 DVI。

- [x] 排版节点：Char/Glue/Kern/Box/Leaders/Penalty（M3-1，Arena 分配后续做）
- [x] `\hbox`/`\vbox`/`\vtop` + 维度计算（width/height/depth）（M3-1/2-1；`\vtop` shift 待 M3-5 校准、`to/spread` 规格待实现）
- [x] 基础：`\par`、`\indent`、`\baselineskip`、`\lineskip`（M3-2-2；`\parindent` 等内部参数 + interline glue；段落形状 `\hangindent` 等留待）
- [x] 胶水拉伸/收缩 + badness + 断行点（breakpoints）（M3-2-3：`badness`/`collect_breakpoints`；词间空白 glue）
- [x] Knuth-Plass 折行（单线程，先保证逐位一致）（M3-3：`linebreak::knuth_plass`，DP + active 集 + fil 阶无限胶水 + `\parfillskip`；demerits 常量待 M3-5 对照 pdfTeX 校准；O(n²) 未做 active 淘汰）
- [x] TFM 解析 + 字体表（M3-4：`ntex-font` 解析 cmr10；`\font<cs>=<名字>[at/scaled]` 原语 + `EqSlot::Font` 选择器；字符维度/词间距来自真实度量）
- [x] 断页：page builder 状态机 + 断页 DP（M3-5-2：`page::PageBuilder` 的 build\_page/fire\_up/vpackage，`\vsize`/`\topskip`/`\maxdepth`/`\parskip`，`\end` 冲页不产生多余空页）
- [x] lig/kern 连字字距 + `\sfcode` 词间距（M3-4 补：TFM lig\_kern 表解析 + `append_char` 应用 + spacefactor 词间胶水）
- [x] `\output` 例程 + box255（M3-5-3：token 列表存储 + fire\_up 改道 + 引擎 token 边界注入；`\box<n>` 寄存器；例程不消费 box255 → 页面丢弃）
- [x] `\shipout` → DVI 写出（M3-5-1：`ntex-dvi` 写出器 + `\shipout` 原语 + `Typesetter::typeset_dvi`；dvipdfmx 实机验收，`pre/bop/fnt_def/set_char/right/down/push/pop/post/post_post` 与真实 TeX 逐字节一致，残余差异仅排版器未实现字体 kern 表）
- [x] `\vtop` shift 对照 DVI 校准（M3-5-1：vtop 首行基线 = shift=height，见 `package_box`）
- [x] **VFS 层 + 副作用模型落地**（RFC-3）：`\input`/`\openin`/`\closein`/`\newread`/
  `\read...to`/`\newwrite`/`\openout`/`\closeout`/`\write`/`\immediate` 全实现，走 VFS
  （`ntex-io`：`Vfs` trait + LocalVfs + MemVfs）；`\write` 延迟到 shipout 边界 /
  `\end` 收尾统一落盘（页面丢弃不写）；流号经 `EqSlot::Stream` 独立绑定（不占 count 槽）；
  `\write18`（shell）拒绝。端到端验证：多文件 `\input` + `\write` aux 落盘。
- [x] `.fmt v1`：状态快照序列化（`ntex-format`：确定性小端二进制编码 + 魔数/版本校验 +
  roundtrip 测试；`Expander::export_state`/`import_state` 全量导出 intern/eqtb/catcode/
  sfcode/寄存器/参数/`\output`，加载时宏字节码重建、运行时栈清零；排版层
  `Typesetter` 包装 + 快照前后排版一致验证）。mmap 零拷贝布局留待 M7（v2）。
- [x] `\parindent=` 等赋值中 `=` 的接受——扫描器支持无 `=` 形式，含 `=` 的
  dimen/glue 赋值待 M4 统一扫描器时补（v1 快照测试用无 `=` 形式）

**验收**：简单文档（含表格、标题、引用）DVI 与 pdfTeX 差分一致；
折行结果与 pdfTeX 一致。
**关卡**：DVI 差分一致后，才允许进入数学（M4）。

***

## 6. M4 数学 + e-TeX

**目标**：ETRIP 通过。

- [x] 数学模式全规则：8 类原子 + spacing 表 + 上下标（脚本分层）
  —— M4-1/2 落地：`Mode::Math/DisplayMath`、`$`/`$$` 进出（expand peek 判定）、
  8 类原子 + 附录 G spacing 表、`^`/`_` 脚本（脚本字段组）、字阶缩放（text/script/scriptscript）
- [x] 分式/根式/括号伸缩（delimiter 变体）
  —— `\over`/`\atop`（组内/脚本内/`\left...\right` 内收尾）、`\sqrt`、`\left.\right.` 定界符；
  **矩阵未做**
- [x] `\displaystyle`/`\textstyle`/字号层级（font size 阶梯）
  —— 4 样式原语 + 字阶比例缩放（10/7/5pt）；**真实 scriptfont 待 M4-3**
- [x] 数学字体参数表（fontdimen）+ 数学字体族（`\textfont` 等）
- [x] 显示数学细化（M4-4）
  —— `$$...$$` 收尾段落（水平模式先 `\par`）作垂直元素：
  `\predisplaypenalty` + `\abovedisplayskip`（或短变体）+ 居中公式盒
  （`\hbox to \hsize` 两侧 `\hfil`）+ `\belowdisplayskip` + `\postdisplaypenalty`；
  6 个新内部参数（4 个 skip glue + 前后 penalty，plain 默认）可赋值 +
  `.fmt` v2 序列化；short 判定 = 末行自然宽度 < `\displaywidth`（≈`\hsize`）；
  公式后文字续排（无 parskip/缩进）；`$$` 在 `\hbox` 内报错
- [x] e-TeX 展开扩展（核心）：`\protected`（\edef/\write 抑制 + .fmt 保留）、
  `\ifdefined`/`\ifcsname`/`\unless`、`\numexpr`（\the/\ifnum/任意整数上下文）、
  `\detokenize`（控制词补空格）、`\unexpanded`、`\eTeXversion`/`\eTeXrevision`
- [x] e-TeX 扩展补全：`\dimexpr`（尺寸表达式，可在任意尺寸上下文求值）、
  `\glueexpr`（胶水表达式，width 求和 + stretch/shrink 取最后非零项）、
  `\ifprimitive`（cs 是否为内建原语）、`\scantokens`（组内容 detokenize 后
  按当前 catcode 重新扫描，等价于从字符串 `\input`）——M4 至此全部完成
- [x] Liang 断字算法 + `\patterns` 语言包
  —— `\patterns` 语言包 + **断字接入段落折行**完成：sink 事件 + expand 原语
  （扫描平衡组、字母/数字/`.` 抽取、空格折叠分隔）+ `PatternTrie::parse` +
  `Node::Discretionary`（三段式）折行断点（penalty = `\hyphenpenalty` 50）+
  `close_paragraph` 物化（行尾补 pre 连字符、行内 replace）端到端接线；
  **词界规则（`\lefthyphenmin`/`\righthyphenmin`）留待 ETRIP 校准**
- [x] 错误模型补全（数学相关错误信息）
  —— 数学模式外 `^`/`_`（cat 7/8）报 "Missing $ inserted"（不再静默渲染字面）；
  数学错误消息统一为 TeX 标准原文（Double superscript/Missing { inserted/
  Missing \left inserted/Ambiguous…/Display math should end with $$），
  8 个用例锁消息；完整上下文行（"l.N …"）留 M1-13

**验收**：**ETRIP 全绿**；含数学的文档差分一致。
**进度**：**M4 全部完成**——M4-1/2/3/4/5（13 + 16 + 20 + 14 + 6 用例）、M4-6 Liang 断字、M4-7 错误模型、e-TeX 扩展（`\dimexpr`/`\glueexpr`/`\ifprimitive`/`\scantokens`，+6 用例）。下一步：**ETRIP 冲刺**（先搭管线：fixtures + harness 泛化 + 引擎驱动）。

### ETRIP 冲刺（2026-08 进行中）

**当前状态**：管线已跑通（fixtures + harness 泛化 + ntex 驱动）；**pass1 全流程已走通**
（e-IniTeX → `\dump`）；pass2（重载 `.fmt` 再运行）逐段推进中，**已通过
`\numexpr/\dimexpr/\glueexpr/\muexpr` 全段**（括号、溢出块、"Expr quotient rounding
1-8"、"Expr fraction rounding 1-3"、运算符优先级、胶水引用计数）+ `\mutoglue/\gluetomu`
段（两原语未实现，走未定义 cs 恢复继续）+ (mu)glue identity 段。当前卡点：
**gluestretchorder 段**（etrip.tex L937-957）——`\1` 宏 `\ifnum\glueshrinkorder#5=#1`
值不匹配（"wrong glue shrink order"）后 "预期 < = > 关系符"（参数绑定/值语义待核对）。
`etrip.log` 逐字节比对待 pass2 走通后开始（已知差距：`\tracingassigns` 的
`{changing/into}` 行、`\the\muexpr` 的 "5.0mu" 显示、错误消息上下文行、`\mutoglue/
\gluetomu` 未实现）。

**本轮（2026-08-23）已完成**：
- 原语族：`\iffontchar`/`\fontcharwd/ht/dp/ic`（char_metric + 条件码 20）、
  `\showifs`、`\parshape` 全族（访问器语义按 TeX 实证）
- 表达式核心：`\let\9=\relax` 别名正确终止表达式；**中间量 i128、仅最终结果
  超限报 "! Arithmetic overflow."**（fraction rounding 全过，`"7FFFFFFE*"7FFFFFFE/
  "7FFFFFFD` 中间乘积 2^62 不误判）；dimen/glue 的 `*`/`/` 右操作数支持括号子
  表达式；表达式除法四舍五入（ties away）
- 胶水表达式语义实证（对照参考 log）：width 求和、**stretch/shrink 值求和**、
  **无穷阶取最后一个非零分量项的阶**（`\skip90+0pt` 保留 1fil、`\skip5+0pt` 清 0）
- 扫描器恢复：`\dimexpr/\glueexpr/\muexpr` 作整数操作数（`\ifnum#4=\dimexpr...`）；
  `\count43pt`（寄存器 + 单位）；**负号与未定义 cs 统一循环恢复**（`-\mutoglue-
  \gluetomu9pt` 逐 token 当 \relax 继续）；`scan_dimen` 支持 glueexpr 宽度
- **eTeX 寄存器扩展**：REGISTER_COUNT 256 → **32768**（Box 堆分配防栈溢出）；
  `scan_register_index` 越界报 "! Bad register code (N)." 并钳 0；`.fmt` codec 同步
- 段级错误定位（`\typeout{Checking ...}` 段标题 → 错误报告带 section）

**剩余原语待办**：完整分组清单 + 每原语进展标记见 **[ETRIP-primitives.md](ETRIP-primitives.md)**（唯一状态源，2026-08-23 更新）。
当前总览：**A 组** 🟡 24/42 · **B 组** 🟡 15/36 · **C 组** ✅ 全部已接线 · **收尾**（`etrip.log` 逐字节比对）⏳。
下一批：`\mutoglue`/`\gluetomu` 实现（当前走未定义恢复）+ gluestretchorder 段宏绑定核对。

**冲刺纪律**：每次迭代前先 `cargo build -p ntex-trip` 确认全绿再跑（避免脏构建旧产物
误报，如误报过的 `\ifcase 序号不能为负`）；对照 etrip.log 参考逐段验证，不做整体 diff。

***

## 7. M5 增量计算（相对 cTeX 的胜负手）

**目标**：编辑体验级的增量重算。

- [ ] 求值图：`Expand(source, snapshot) → (tokens, snapshot')` 纯函数化
- [ ] 依赖追踪：记录每个结果依赖的 catcode/宏定义/计数器/上游段
- [ ] 失效传播：编辑定位 → 只重算失效子图 → Box 级缓存复用
- [ ] 副作用边界：`\write` 延迟到 `\shipout`；aux/toc 增量更新与去重合并
- [ ] `\the\count` 等全局读 → 数据依赖登记（防缓存失效错误）
- [ ] `.aux`/`.toc` 增量；两次编译收敛语义不变
- [ ] **基准**：改 1 字 → 重算耗时 vs 全量重编（目标：差 ≥ 100x）

**验收**：增量结果与全量重编逐位一致（随机编辑模糊测试）。
**风险**：副作用漏追踪 → 缓存错 → 必须用"增量 vs 全量 diff"作 CI 常驻检查。

***

## 8. M6 并行

**目标**：多核吃满。

- [ ] 段落级并行布局（rayon）：段内串行、段间并行
- [ ] 展开阶段并行：仅限"段落边界 + 状态快照隔离 + 无跨段副作用"
- [ ] 字体整形并行（M9 的 HarfBuzz 在此铺路）
- [ ] 确定性保证：每段独立快照、结果按段序合并

**验收**：8 核下 300 页基准加速比 ≥ 3x，输出与单线程逐位一致。

***

## 9. M7 .fmt v2（冷编性能第二刀）

**目标**：把用户冷编变热编。

- [ ] 三层 .fmt：状态快照 / 预编译字节码 / 预计算索引（控制序列→字节码指针）
- [ ] 部分求值：latex.ltx、ctex 中"参数固定、无副作用"模板宏的构建期专用化
- [ ] 项目级 .fmt：preamble 固定时缓存"ctex + 用户宏包 + preamble"为独立 fmt
- [ ] mmap 只读 + 页级 COW（多进程共享不污染）
- [ ] 基准：latex.ltx 加载 <200ms；ctex 300 页冷编 <10s

**验收**：冷编基准达标；TRIP/ETRIP 在 .fmt 路径全绿。

***

## 10. M8 渲染/输出

**目标**：三种输出端，交互预览可用。

- [x] 临时切片先行（M3 中途）：`ntex-pdf` 已产出可看 PDF（Helvetica 标准字体 + 贪心折行；M3-3 Knuth-Plass / M3-4 TFM / M3-5 DVI 后替换）
- [x] **正式 PDF 后端**：DVI → PDF 直出（跳过 xdv→xdvipdfmx 中间步）。`ntex-pdf` 改为
  DVI 解析器（set\_char/set\_rule/push/pop/right/down/wxyz/fnt\_def 全支持）+ PDF 1.4 写出器
  （对象/xref/页面树/内容流）+ Type1 字体嵌入（原始 PFB + /FontFile）。内容流按行成 TJ 串
  （同 dvipdfmx），词距用调整量表达——pdftotext 提取干净、渲染与 dvipdfmx 像素级一致
  （demo.tex 两页对比墨水比 1.02\~1.04，均为字体提示/抗锯齿差异）。
- [ ] L2 字节兼容：简单文档对照 pdfTeX 逐字节 diff（关闭时间戳/元数据随机性）
- [ ] Skia 渲染后端（桌面/服务端）
- [ ] WASM 前端：wasm-bindgen + Canvas2D/WebGL 实时预览（接 M5 增量，毫秒级刷新）
- [ ] SyncTeX 源码映射（IDE 点击跳转）

**验收**：简单文档 L2 一致；WASM 演示在浏览器增量预览流畅。

***

## 11. M9 生态冲刺

**目标**：中文生态落地。

- [ ] ctex/xeCJK 宏兼容（这本身是巨型工作量，从 xeCJK 最小子集开始）
- [ ] OpenType 字体：fontspec 兼容路径 + HarfBuzz 复杂整形
- [ ] **CJK 整形捷径**：无复杂特性时跳过 HarfBuzz，直接读 hmtx（目标 5\~10x）
- [ ] 宏包 CI 回归集：geometry、amsmath、hyperref、biblatex、tikz、ctex
- [ ] 引擎身份模拟（`\pdftexversion` 等），兼容依赖引擎行为的宏包

**验收**：目标宏包回归全绿；300 页中文冷编基准达标。

***

## 12. Crate 划分建议（Rust workspace）

| Crate                    | 职责                                                                                                                        | 现状                                                    |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| `ntex-core`              | token（8B tagged union）、catcode、InternTable、eqtb 版本化、展开引擎、**字节码 IR/编译器/执行器（M2 落在此 crate，`ntex-vm`** **未单建）**、内部参数、sink 事件流 | ✅ 已建                                                  |
| `ntex-layout`            | 节点、**主循环（模式状态机）**、badness/断行点、Knuth-Plass、TFM 字体接入、断页、数学排版                                                                | ✅ 已建（M3-1\~4 完成）                                      |
| `ntex-pdf`               | **正式 DVI → PDF 后端**：DVI 解析 + PDF 1.4 写出 + Type1(PFB) 嵌入；替换临时切片                                                            | ✅ 已建（M8）                                              |
| `ntex-font`              | TFM/OFM、ttf-parser、HarfBuzz 整形、整形缓存                                                                                       | ✅ 已建（M3-4：TFM 解析 + `\font` 加载 + 缩放；ttf/HarfBuzz 待 M9） |
| `ntex-format`            | .fmt 序列化/反序列化（v1 快照已完成；v2 mmap 零拷贝 + 字节码固化 + 部分求值）                                                                        | ✅ 已建（M3 收尾：v1 内存快照；v2 待 M7）                           |
| `ntex-incremental`       | 求值图、依赖追踪、失效传播                                                                                                             | 未建（M5）                                                |
| `ntex-io`                | VFS、aux 增量                                                                                                                | ✅ 已建（RFC-3：Vfs trait + LocalVfs/MemVfs + 读写原语）        |
| `ntex-backend`           | PDF/Skia/WebGPU 后端 trait + 实现                                                                                             | 未建（M8）                                                |
| `ntex-cli` / `ntex-wasm` | 命令行 / WASM 前端                                                                                                             | 未建（M9）                                                |

***

## 13. 关键决策

### 已拍板（RFC-1，M0）

1. Token = **8B tagged union**；源码位置走 side-table；
2. 控制序列 token 只含 **csid**，定义查 **eqtb 槽**（Macro/Primitive/Register/Alias）；
3. 宏体 = **TokenArray**（连续不可变切片）+ 字节码双表示；
4. InternTable 线性化，csid=u32 下标，`.fmt` mmap 零字符串查找；
5. M1 的 TRIP 采用**硬口径**：M1 结束全绿，排版部分用 `\hbox` 兜底占位。
6. **输出端两段式**（2026-08）：先临时 Helvetica 切片拉通输出端（验证流水线）；M3-5 DVI 就绪后
   `ntex-pdf` 改造为**正式 DVI → PDF 后端**（Type1 嵌入，dvipdfmx 渲染一致）并提前推进（M8 前移），
   临时切片已下线。
7. **VM 保持纯 token 级**（2026-08，M3-2）：排版事件经 `TokenSink`（token/组/原语/glue/kern/
   penalty/rule/内部参数）单向流出，排版器（ntex-layout）持模式状态机；VM 不依赖布局 crate。

### 待拍板（每项影响后续架构）

1. **L1 vs L2 兼容优先级**：先 L1（语义/折行一致）冲 M4，L2（字节）推迟到 M8——已按此排期，需确认。
2. **字节码 vs JIT**：M2 只做字节码；JIT 作为 M9 之后的可选加速，不进入本次计划主线。
3. **CJK 捷径的兼容边界**：跳过 HarfBuzz 的判定条件要保守，否则字形渲染不一致——M9 单独评审。
4. **ctex 兼容范围**：先 xeCJK 最小子集（简体中文常用排版），而非一步到位全量。
5. **数学输出**：先保证 DVI/PDF 逐位一致，MathML 输出为可选项（不阻塞主线）。
6. **RFC-1 开放问题 Q2\~Q4**：保留位用途 / 内部标记拆分 / InternTable CoW 粒度——M0 评审会拍板。

***

## 14. 风险与关卡清单

| 风险             | 关卡（Gate）                 | 触发时动作                    |
| -------------- | ------------------------ | ------------------------ |
| TRIP 长期不绿      | M1 结束必须全绿                | 冻结新增功能，只修语义              |
| 字节码与解释器不一致     | M2 双轨 diff 测试            | 禁用字节码路径，回溯 IR 设计         |
| 增量缓存出错（副作用漏追踪） | M5 增量-vs-全量模糊 diff 常驻 CI | 修复依赖登记，不回退功能             |
| 并行破坏确定性        | M6 输出逐位 diff             | 收窄并行边界（只并布局/整形）          |
| ctex 兼容工作量失控   | M9 按 xeCJK 子集分阶段验收       | 缩减目标宏包范围，先保 ctex 常用      |
| 冷编基准不达标        | M7 验收                    | 优先级：深 .fmt > CJK 捷径 > 多核 |

