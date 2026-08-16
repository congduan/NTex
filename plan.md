# NTex 实施计划

> 依据：[idea.md](file:///Users/congduan/Desktop/code/_vibe_coding_/NTex/idea.md) 架构 + 性能决策（字节码预编译 / 深 .fmt / CJK 整形捷径 / 并行 / 增量）
> 原则：**正确性优先、性能架构前置、基准先行**

---

## 0. 三个策略性结论（决定排期顺序）

1. **字节码编译要早做，不能放到最后**。它是冷编性能的最大蛋糕（确定性 2~5x），
   且影响 `.fmt` 格式与求值图的设计——越晚做，重构成本越高。
   → 排在解释器跑通 TRIP 之后立即做。
2. **`.fmt` 分两代**。v1（内存快照）在排版阶段即可用；v2（快照 + 字节码 + 部分求值）
   在增量层就绪后升级。格式设计必须预留扩展位。
3. **副作用隔离（VFS + shipout 边界）在 M3 就要定型**，否则增量层（M6）无法落地，
   而它是相对 cTeX 的胜负手。

---

## 1. 里程碑总览

| # | 里程碑 | 核心交付 | 验收标准（Exit Criteria） |
|---|---|---|---|
| M0 | 地基 | 仓库/CI/基准框架/TRIP 测试框架 | 基准脚本与测试框架跑通 |
| M1 | 解释器内核 | Token+16 catcode+宏展开+核心原语 | **TRIP 通过** |
| M2 | 字节码编译 | 字节码 IR + 编译器 + 执行器 | TRIP 仍绿；展开吞吐 ≥ 解释器 2x |
| M3 | 排版核心 | 节点/盒子/胶水/Knuth-Plass/TFM/断页/DVI 输出 | 与 pdfTeX 折行结果一致；简单文档 DVI 差分一致 |
| M4 | 数学 + e-TeX | 数学模式/e-TeX 原语/断字/错误模型 | **ETRIP 通过** |
| M5 | 增量计算 | 求值图/依赖追踪/副作用隔离/VFS | 改 1 处仅重算受影响段落（基准可量化） |
| M6 | 并行 | 段落级并行展开/布局/整形 | 8 核加速比 ≥ 3x（300 页基准） |
| M7 | .fmt v2 | 三层 fmt + 部分求值 + 项目级 fmt | latex.ltx 加载 <200ms；ctex 300 页冷编 <10s |
| M8 | 渲染/输出 | PDF 后端 + Skia + WASM 预览 | 简单文档 PDF 逐字节一致；WASM 演示可用 |
| M9 | 生态冲刺 | ctex/xeCJK/OpenType/CJK 捷径 | 目标宏包 CI 回归全绿 |

**依赖链**：M0 → M1 → M2 → M3 → M4 → M5 → M6 → M7 → M8 → M9
（M5 依赖 M2/M3；M7 依赖 M2/M5；M6 可与 M5 部分并行开发）

---

## 2. M0 地基

**目标**：一切后续工作的脚手架，**基准先行**。

- [ ] 初始化 Rust workspace（crate 划分见 §8）
- [ ] CI：单元测试 + TRIP/ETRIP + 差分测试 + 基准（跑基准防回归）
- [ ] **基准集**（所有里程碑复用）：
  - `latex.ltx` 全量加载耗时
  - 300 页中英混排文档冷编总时长
  - 每千 token 展开吞吐（对照 pdfTeX 基线）
  - 增量场景：改 1 字 → 重算耗时
- [ ] TRIP/ETRIP 测试框架：自动跑 `\input trip` 并 diff 输出
- [ ] 差分测试工具：同一 .tex 分别跑 pdfTeX/XeTeX 与本引擎，diff DVI/log
- [ ] **设计文档（RFC）先行**，评审通过再编码：
  - RFC-1 Token 表示与内存布局 —— **已定稿**（8B token / TokenArray / InternTable / eqtb 版本化）
  - RFC-2 不可变状态模型与版本化（CoW 结构选型）—— 未开始
  - RFC-3 副作用模型（VFS + shipout 边界）—— 未开始
  - RFC-4 字节码 IR 草案（提前定，M2 直接用）—— 未开始

**RFC-1 已定决策**（后续里程碑直接引用，不再反复讨论）：
1. **Token = 8 字节 tagged union（u64）**，不用 16B；源码位置走独立 side-table，不进 token；
2. **控制序列 token 不携带定义**：token 只有 csid，等价关系一律查 eqtb 槽
   （`Undefined/Macro{version,def}/Primitive/Register/Alias`）；
3. **宏体 = 连续不可变 TokenArray**（非链表），另配字节码双表示；
4. **InternTable 线性化**：csid=u32 下标，`.fmt` mmap 后零字符串查找；新建 cs 走原子追加 + 版本切换；
5. RFC-1 遗留开放问题 Q2~Q4（保留位用途/内部标记拆分类/InternTable CoW 粒度）在 M0 评审会一并拍板。

**关卡**：四份 RFC 评审通过；基准与 TRIP 框架可一键运行。

---

## 3. M1 解释器内核（正确性第一，不做性能）

**目标**：按 RFC-1 数据模型跑通 TeX 展开语义，通过 TRIP（硬口径）。

### 实施步骤（依赖驱动，每步独立可验收）

**M1-1 Token 类型骨架**（RFC-1 §3）
- [ ] `Token` 枚举：`#[repr(transparent)]` 包装 u64，变体与位分配严格按 RFC-1：
      `Char(catcode 4b + charcode 21b)` / `ControlSeq(csid 32b)` / `MacroParam(num 4b)` /
      `EndGroup` / 保留 tag 5..15
- [ ] `Debug`/`Meaning` 输出（对照 TeX `\show` 输出格式）
- 验证：单元测试覆盖 RFC-1 §3 变体表 + §8 用例 4（BMP+ 字符）、5（active char 的 meaning）、8（往返一致性）

**M1-2 InternTable（csid 驻留）**（RFC-1 §4）
- [ ] 名字去重表：`name → csid`，csid=u32 数组下标
- [ ] `\csname..\endcsname` 动态建 cs：命中复用、未命中追加（不可变版本切换）
- [ ] `\meaning` 名字查询走 InternTable
- 验证：用例 1（动态建名含空 csname、非法字符）；并发预留（M6 再做原子追加）

**M1-3 eqtb 槽版本化**（RFC-1 §5）
- [ ] eqtb 槽枚举：`Undefined/Macro{version, def}/Primitive(prim_id)/Register/RegisterIndex/Alias(csid)`
- [ ] `\let\a\b` → `Alias(csid)` 间接（不复制宏体）；链式别名
- [ ] `\chardef`/`\mathchardef` → `RegisterIndex` 语义
- 验证：用例 2（链式别名、`\let` 到 `\outer`）、6（`\ifx` 对 Alias 不展开）

**M1-4 输入与 catcode 固化**
- [ ] 字节流输入 → 16 种 catcode 表查询 → 生成 `Char` token，**token 内固化当时 catcode**
- [ ] catcode 动态修改原语（`\catcode`）只影响后续输入，不回写已生成 token
- [ ] active char（catcode 13）生成后查 eqtb 展开
- 验证：与 pdfTeX 对照"改 catcode 后已读 token 不受影响"的行为

**M1-5 宏定义与 MacroDef**（RFC-1 §5）
- [ ] `MacroDef { params: ParamSpec, body: TokenArray }`，body 为连续不可变切片
- [ ] `\def`/`\edef`/`\gdef`：定义时捕获 token 数组；`\edef` 定义期全展开
- [ ] 宏体内 `#` 三态：`#1` 参数槽 / `##` 字面 / 非法 `#` 报错（RFC-1 §8 用例 5）
- [ ] `\newcommand`（经 `\def` + 存在性检查语义）
- 验证：用例 5、7（分隔串按 token 序列匹配）

**M1-6 展开主循环**
- [ ] "读 token → 可展开则展开（循环至不可展开）→ 节点入队 / 原语执行"
- [ ] 可展开/不可展开二分表（含 `\protected` 语义占位，e-TeX 在 M4 补全）
- [ ] 宏调用 = 查 eqtb → 展开 MacroDef（先做朴素 token 替换，M2 换字节码）
- 验证：TRIP 基础部分逐步点亮

**M1-7 扫描顺序原语**
- [ ] `\expandafter`/`\noexpand`/`\futurelet`/`\aftergroup`/`\afterassignment`
- 验证：与 pdfTeX 对照这些原语的交互行为（这是最易翻车处，专项测试）

**M1-8 参数匹配**
- [ ] 无分隔参数：`#1..#9` 实参收集（平衡组规则）
- [ ] 分隔参数：分隔串按 **token 序列**匹配（RFC-1 §8 用例 7）
- [ ] `\long` 与"参数中禁 `\par`"错误语义
- 验证：嵌套宏实参传递用例集

**M1-9 条件原语**
- [ ] `\if`/`\ifnum`/`\ifdim`/`\ifx`/`\ifcase` + `\else`/`\fi`
- [ ] **惰性求值**：未走分支不展开（跳过 token 流）
- [ ] `\ifx` 比较规则：Char 比 (catcode,char)，CS 比同一 csid（RFC-1 §2）
- 验证：含嵌套 `\if` 与跨宏条件用例

**M1-10 寄存器与内部量**
- [ ] `\count`/`\dimen`/`\skip`/`\toks` + 赋值原语 + `\the`
- [ ] 内部量表示：scaled point（sp，2^-16 pt）定点数
- [ ] 寄存器读写走 eqtb 槽（版本化，为 M5 依赖追踪铺路）
- 验证：`\the\count`/`\the\dimen` 输出与 pdfTeX 逐位一致

**M1-11 组与作用域**
- [ ] `{...}`/`\begingroup...\endgroup`：组内赋值组尾回滚
- [ ] M1 先做**朴素快照回滚**（正确性），M2 换 eqtb 版本指针（O(1)）
- 验证：`\global` 与非全局赋值回滚行为对照

**M1-12 模式状态机（空壳）**
- [ ] 垂直/水平/数学/内部 四种模式 + 切换规则（`\par`/`$`/`\hbox{}` 等触发点占位）
- [ ] 数学模式 M1 只承接 token 不排版（M4 填实）
- 验证：模式切换错误信息与 pdfTeX 一致

**M1-13 错误模型**
- [ ] 错误上下文输出（"! ..." + 上下文行）+ 四种交互模式（`\batchmode` 等）
- 验证：构造错误用例，输出与 pdfTeX 逐字符一致

**M1-14 TRIP 冲刺**
- [ ] 涉及排版/字体的部分用 `\hbox` 兜底占位（硬口径：语义 bug 绝不带进 M2）
- [ ] **TRIP 全绿**（输出 diff 可读化脚本已在 M0 就绪）
- 验证：`\input trip` 输出与参考文件一致

**M1-15 性能基线**
- [ ] 记录每千 token 展开吞吐基线（**不做优化**，仅存档，供 M2 对照）

**验收**：TRIP 通过（硬口径）；错误行为与 pdfTeX 一致。
**风险**：TRIP 是"实现后才知道哪错"的黑盒 → 提前做好 TRIP 输出 diff 的可读化。

---

## 4. M2 字节码编译（冷编性能第一刀）

**目标**：宏展开从"token 替换"升级为"执行字节码"，语义严格等价（双轨 diff 保证）。

### 实施步骤

**M2-1 字节码 IR 定稿**（RFC-4 草案落地，对接 RFC-1）
- [ ] 指令集定稿：`PushTok`/`Lookup`/`ExpandCall`/`Branch`/`Assign`/`PopGroup`/`LoadArg(n)`/
      `ExpandAfter` 等
- [ ] 操作数编码：**token 以 8B 原值内联**（RFC-1 布局零解包）；cs 以 csid 索引 eqtb
- 验证：IR 编码往返测试（指令流 → 反汇编 → 等价）

**M2-2 宏定义期编译器**
- [ ] `MacroDef.body: TokenArray` → 字节码（`#n` 参数槽 → `LoadArg(n)`）
- [ ] `\if` 条件 → 字节码跳转（保留惰性求值语义：未走分支跳过）
- [ ] `\expandafter`/`\futurelet` → 专用指令（保持扫描顺序语义）
- [ ] 编译失败路径：非法宏体（保留报错语义，不静默降级）
- 验证：每个编译产物与解释器逐 token 展开结果相等（单元级 diff）

**M2-3 字节码执行器 + 双轨并存**
- [ ] 字节码解释循环（dispatch loop），执行 `MacroDef.code`
- [ ] 解释器（M1）/字节码（M2）双轨，环境变量切换
- [ ] **等价性框架**：TRIP + 随机 token 序列 + 宏包片段，双轨输出 diff
- 验证：M1 全部用例在字节码路径重跑全绿

**M2-4 原语 dispatch 表**
- [ ] `prim_id → 处理函数`（jump table）；`Primitive` 槽命中即派发
- [ ] 原语表与 InternTable 预注册（`\catcode` 建立的内建 cs）
- 验证：全量原语冒烟测试

**M2-5 内存落地**
- [ ] token/指令分配：bumpalo arena（整段分配、无逐项 malloc）
- [ ] TokenArray/Bytecode 同 arena 共存，`.fmt` 序列化友好布局（M7 复用）
- 验证：arena 泄漏/越界（Debug 断言）+ 长文档稳定性

**M2-6 性能达标与定位**
- [ ] **基准**：每千 token 展开吞吐 ≥ 解释器 2x；与 pdfTeX 对照记录差距
- [ ] 火焰图定位热点（预期：eqtb 查询 / 实参拷贝 / arena 边界）
- [ ] 小步优化：eqtb 槽缓存行布局、实参零拷贝（切片借用）等
- 验证：基准数字入库（M0 基准集），CI 防回归

**M2-7 双轨框架移交**
- [ ] 双轨等价测试框架保留并文档化（M7 升级 .fmt 时继续使用）
- [ ] 决策点：字节码成为默认执行路径，解释器退为调试工具

**验收**：TRIP 在字节码路径全绿；吞吐基准达标（≥ 解释器 2x）。
**关卡**：字节码与解释器双轨的等价性测试框架保留到 M7（防 .fmt 升级回归）。

---

## 5. M3 排版核心

**目标**：能产出与 TeX 一致的页面，输出 DVI。

- [x] 排版节点：Char/Glue/Kern/Box/Leaders/Penalty（M3-1，Arena 分配后续做）
- [x] `\hbox`/`\vbox`/`\vtop` + 维度计算（width/height/depth）（M3-1/2-1；`\vtop` shift、`to/spread` 规格留待 M3-2-2/M3-5）
- [x] 基础：`\par`、`\indent`、`\baselineskip`、`\lineskip`（M3-2-2；`\parindent` 等内部参数 + interline glue；段落形状 `\hangindent` 等留待）
- [x] 胶水拉伸/收缩 + badness + 断行点（breakpoints）（M3-2-3：`badness`/`collect_breakpoints`；词间空白 glue）
- [ ] Knuth-Plass 折行（单线程，先保证逐位一致）
- [ ] 断页：page builder 状态机 + 断页 DP
- [ ] TFM 解析 + 字体表（`\font`）；`\shipout` → DVI 写出
- [ ] **VFS 层 + 副作用模型落地**（RFC-3）：`\write`/`\read`/`\input` 走 VFS
- [ ] `.fmt v1`：状态快照序列化 + mmap（此时仍是"快照"级）

**验收**：简单文档（含表格、标题、引用）DVI 与 pdfTeX 差分一致；
折行结果与 pdfTeX 一致。
**关卡**：DVI 差分一致后，才允许进入数学（M4）。

---

## 6. M4 数学 + e-TeX

**目标**：ETRIP 通过。

- [ ] 数学模式全规则：8 类原子 + spacing 表 + 上下标（脚本分层）
- [ ] 分式/根式/矩阵/括号伸缩（delimiter 变体）
- [ ] `\displaystyle`/`\textstyle`/字号层级（font size 阶梯）
- [ ] 数学字体参数表（fontdimen）
- [ ] e-TeX 扩展：`\protected`、`\ifdefined`、`\numexpr`/`\dimexpr`、`\detokenize` 等
- [ ] Liang 断字算法 + `\patterns` 语言包
- [ ] 错误模型补全（数学相关错误信息）

**验收**：**ETRIP 全绿**；含数学的文档差分一致。

---

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

---

## 8. M6 并行

**目标**：多核吃满。

- [ ] 段落级并行布局（rayon）：段内串行、段间并行
- [ ] 展开阶段并行：仅限"段落边界 + 状态快照隔离 + 无跨段副作用"
- [ ] 字体整形并行（M9 的 HarfBuzz 在此铺路）
- [ ] 确定性保证：每段独立快照、结果按段序合并

**验收**：8 核下 300 页基准加速比 ≥ 3x，输出与单线程逐位一致。

---

## 9. M7 .fmt v2（冷编性能第二刀）

**目标**：把用户冷编变热编。

- [ ] 三层 .fmt：状态快照 / 预编译字节码 / 预计算索引（控制序列→字节码指针）
- [ ] 部分求值：latex.ltx、ctex 中"参数固定、无副作用"模板宏的构建期专用化
- [ ] 项目级 .fmt：preamble 固定时缓存"ctex + 用户宏包 + preamble"为独立 fmt
- [ ] mmap 只读 + 页级 COW（多进程共享不污染）
- [ ] 基准：latex.ltx 加载 <200ms；ctex 300 页冷编 <10s

**验收**：冷编基准达标；TRIP/ETRIP 在 .fmt 路径全绿。

---

## 10. M8 渲染/输出

**目标**：三种输出端，交互预览可用。

- [x] 临时切片先行（M3 中途）：`ntex-pdf` 已产出可看 PDF（Helvetica 标准字体 + 贪心折行；M3-3 Knuth-Plass / M3-4 TFM / M3-5 DVI 后替换）
- [ ] PDF 后端：直接生成（跳过 xdv→xdvipdfmx 中间步），对象批量缓冲写
- [ ] L2 字节兼容：简单文档对照 pdfTeX 逐字节 diff（关闭时间戳/元数据随机性）
- [ ] Skia 渲染后端（桌面/服务端）
- [ ] WASM 前端：wasm-bindgen + Canvas2D/WebGL 实时预览（接 M5 增量，毫秒级刷新）
- [ ] SyncTeX 源码映射（IDE 点击跳转）

**验收**：简单文档 L2 一致；WASM 演示在浏览器增量预览流畅。

---

## 11. M9 生态冲刺

**目标**：中文生态落地。

- [ ] ctex/xeCJK 宏兼容（这本身是巨型工作量，从 xeCJK 最小子集开始）
- [ ] OpenType 字体：fontspec 兼容路径 + HarfBuzz 复杂整形
- [ ] **CJK 整形捷径**：无复杂特性时跳过 HarfBuzz，直接读 hmtx（目标 5~10x）
- [ ] 宏包 CI 回归集：geometry、amsmath、hyperref、biblatex、tikz、ctex
- [ ] 引擎身份模拟（`\pdftexversion` 等），兼容依赖引擎行为的宏包

**验收**：目标宏包回归全绿；300 页中文冷编基准达标。

---

## 12. Crate 划分建议（Rust workspace）

| Crate | 职责 |
|---|---|
| `ntex-core` | token（8B tagged union）、catcode、InternTable、eqtb 版本化、展开引擎 |
| `ntex-vm` | 字节码 IR/编译器/执行器、状态版本化 |
| `ntex-layout` | 节点、Knuth-Plass、断页、数学排版 |
| `ntex-font` | TFM/OFM、ttf-parser、HarfBuzz 整形、整形缓存 |
| `ntex-format` | .fmt 序列化/反序列化、mmap、部分求值 |
| `ntex-incremental` | 求值图、依赖追踪、失效传播 |
| `ntex-io` | VFS、aux 增量 |
| `ntex-backend` | PDF/Skia/WebGPU 后端 trait + 实现 |
| `ntex-cli` / `ntex-wasm` | 命令行 / WASM 前端 |

---

## 13. 关键决策

### 已拍板（RFC-1，M0）
1. Token = **8B tagged union**；源码位置走 side-table；
2. 控制序列 token 只含 **csid**，定义查 **eqtb 槽**（Macro/Primitive/Register/Alias）；
3. 宏体 = **TokenArray**（连续不可变切片）+ 字节码双表示；
4. InternTable 线性化，csid=u32 下标，`.fmt` mmap 零字符串查找；
5. M1 的 TRIP 采用**硬口径**：M1 结束全绿，排版部分用 `\hbox` 兜底占位。

### 待拍板（每项影响后续架构）
1. **L1 vs L2 兼容优先级**：先 L1（语义/折行一致）冲 M4，L2（字节）推迟到 M8——已按此排期，需确认。
2. **字节码 vs JIT**：M2 只做字节码；JIT 作为 M9 之后的可选加速，不进入本次计划主线。
3. **CJK 捷径的兼容边界**：跳过 HarfBuzz 的判定条件要保守，否则字形渲染不一致——M9 单独评审。
4. **ctex 兼容范围**：先 xeCJK 最小子集（简体中文常用排版），而非一步到位全量。
5. **数学输出**：先保证 DVI/PDF 逐位一致，MathML 输出为可选项（不阻塞主线）。
6. **RFC-1 开放问题 Q2~Q4**：保留位用途 / 内部标记拆分 / InternTable CoW 粒度——M0 评审会拍板。

---

## 14. 风险与关卡清单

| 风险 | 关卡（Gate） | 触发时动作 |
|---|---|---|
| TRIP 长期不绿 | M1 结束必须全绿 | 冻结新增功能，只修语义 |
| 字节码与解释器不一致 | M2 双轨 diff 测试 | 禁用字节码路径，回溯 IR 设计 |
| 增量缓存出错（副作用漏追踪） | M5 增量-vs-全量模糊 diff 常驻 CI | 修复依赖登记，不回退功能 |
| 并行破坏确定性 | M6 输出逐位 diff | 收窄并行边界（只并布局/整形） |
| ctex 兼容工作量失控 | M9 按 xeCJK 子集分阶段验收 | 缩减目标宏包范围，先保 ctex 常用 |
| 冷编基准不达标 | M7 验收 | 优先级：深 .fmt > CJK 捷径 > 多核 |
