# NTex 实施计划

> 依据：[idea.md](file:///Users/congduan/Desktop/code/_vibe_coding_/NTex/idea.md) 架构 + 性能决策（字节码预编译 / 深 .fmt / CJK 整形捷径 / 并行 / 增量）
> 原则：**正确性优先、性能架构前置、基准先行**

## 当前进度（2026-09-03）

| 里程碑           | 状态                                                                                                                                                                                                                                                                                                         |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| M0 地基         | ✅ 完成（workspace/CI/TRIP·diff·bench 工具链；RFC-1/RFC-4 定稿）                                                                                                                                                                                                                                                      |
| M1 展开内核       | 🟡 核心完成：M1-1\~7、M1-9\~11 已实现（95 用例）+ **A2 空行→`\par` 行状态机 + A4 `\outer` 语义 + M1-13 错误上下文行 `l.N`**（2026-08-23 评审修复）；M1-13 错误恢复待补；**M1-14 TRIP 冲刺推进中**（2026-08-25 a00a4d9：`\mag`+pc/cc 单位、扫描恢复语义、^^ 十六进制输入、`\write` 流 -1、缺 cs/字体/数学/排版错误恢复；trip.tex 2026-09-03 起可全程跑完不 panic（finish 收尾冲页后清理输出例程残留列表，3401038），semantic diff 基线 -5619/+1412 收尾中）                                                                                                                                                                                                                              |
| M2 字节码        | 🟡 双轨完成（100 用例等价）；**性能 P0 补课落地**（字节码 u64 原始字执行器 + release 调优，80022b4）；**吞吐重测 2026-09-03：比值 1.01x，≥2x 未达（结构性——RFC-4 零解包 IR 与解释器 TokenArray 同构，见 §4 M2-6 根因）**；P1 热路径消分配落地（含 `error_context` O(n²) 修复，展开吞吐 +61%）；M2-5 arena 未做 |                                                                                                                                                                                                                             |
| M3 排版核心       | ✅ M3-1\~M3-4 完成（58+ 用例）；**M3-5 DVI 写出 +** **`\shipout`** **+ 断页 DP + lig/kern +** **`\sfcode`** **+** **`\output`** **例程/box255** 完成（dvipdfmx 验收 + 与 TeX 差分对照）；**RFC-3 VFS + 副作用模型落地**（10 原语走 `ntex-io` VFS，延迟写入 shipout 边界提交）；**`.fmt`** **v1 内存快照**（`ntex-format` 确定性编码 + roundtrip）                     |
| M4 数学 + e-TeX | ✅ **全部完成**：数学模式状态机（`$`/`$$`、8 类原子、spacing 表、上下标、字阶）、分式/根式/定界符、样式原语、fontdimen 数学参数 + 数学字体族、显示数学细化、Liang 断字、错误模型、e-TeX 核心 + 扩展（`\protected`/`\ifdefined`/`\ifcsname`/`\unless`/`\numexpr`/`\detokenize`/`\unexpanded`/`\eTeXversion`/`\dimexpr`/`\glueexpr`/`\ifprimitive`/`\scantokens`）；**ETRIP 冲刺**：A 组 42/42 ✅（2026-09-03 `\muexpr` 销账）、B 组 36/36、C 组全部已接线；`etrip.log` 逐字节比照 2026-09-03 降级口径推进（语义 diff 归零 + 错误块抽查，逐字节留 M8 L2，见 §6 末尾） |
| M5 增量计算 | 🟡 阶段一\~五完成（2026-09-03：0099b1f→7601d1d→c2a2418→b585864→55bcb09→14f7bc7）：段级增量重算（POC）→ 可回滚段快照 · edit 单路径化 → 端到端增量排版（连续编辑逐位一致）→ 排版层依赖判定（宏体编辑省重排）→ 复用判定开销削减（**增量墙钟翻正**：release 120 段文档改正文 4.3x / 改宏体 1.1x，增量 < 全量）；**仍存边界** + **阶段六待办**见 §7 |
| 输出端           | 🟢 正式 PDF 后端可用（`ntex-pdf`：DVI → PDF 直出 + Type1 嵌入，替换临时 Helvetica 切片；demo 两页与 dvipdfmx 渲染一致；023ff2a 修 DVI 字体选择按字体号 k 归位——demo.dvi → PDF 不再报"未定义字体"）；**多字体嵌入 2026-09-04 验证**（CM 全家族 6 族 8 页 demo-multi 全链：cmr10/cmbx10/cmss12/cmtt10/cmmi10/cmsy10，`/FontFile` 逐字节等于 PFB、`/BaseFont` 取 PFB `/FontName`；每页 `/Resources /Font` 只列本页实际引用字体、跨页复用同一字典对象无冲突；PFB 查找链补 `NTEX_TYPE1_DIR` + 备料根兜底；`cargo test -p ntex-pdf` 11 用例全绿） |

**下一步**：主线已按 §6 末尾 2026-09-03 转场决策**转入 M5**（阶段一\~五完成），当前双线并行：**M5 阶段六**（§7：执行段成本削减/检查点增量维护、副作用边界 `\output`·`\write`·`\input`·marks、`Expand(source, snapshot)` 纯函数化、槽级归因、随机编辑模糊测试）+ **TRIP/ETRIP 收尾**——P0 必修剩 **M1-13 错误恢复通用机制**（`back_input`/`\errhelp`，trip.tex 恢复路径卡点根因，契约级，见 §6 收尾决策项 ③）；TRIP/ETRIP 语义 diff 归零 + 错误块抽查，`etrip.log`/TRIP log 逐字节口径留 M8 L2。性能 backlog **P1** 消分配已落地（展开吞吐 +61%），`expand-throughput`（release）469ms / 42.6 万调用/s 已入库。

***

## 性能优化 backlog（2026-08-22 评审）

> 来源：高级 Rust 工程师评审结论（热路径实现层欠账 + 工程化闭环），不引入新功能，
> 属既定里程碑补课。纪律：**先量化再冲 M5/M7**——不在 1.12x 的基线上做增量层。
> （2026-09-03 量化已完成：比值 1.01x 未达 2x 且属结构性，见 §4 M2-6；P1 消分配落地，
> 展开吞吐 +61%。）

### P0 构建配置（零成本 10~30%，最先做）—— ✅ 已提交 80022b4

- [x] 工作区 `[profile.release]`：`lto = "thin"`、`codegen-units = 1`、
  `panic = "abort"`（已确认全库无 `catch_unwind`/无 `unsafe`，可直接上）
- 验证：~~每千 token 吞吐提升入档~~ —— ✅ 已入档（2026-09-03 重测，见 §4 M2-6 记录：
  469ms / 42.6 万调用/s；ntex 驱动基准死循环已于 2026-08-23 修复）

### P0 M2-6 补课：字节码执行器走 u64 原始字（RFC-4 设计落地）—— ✅ 已提交 80022b4

- [x] `Bytecode.code` 改存 `Arc<[u64]>` 原始字（不再存 16B `Instruction` 枚举，
  现 `encode`/`to_words` 只用于序列化，热路径零解包设计被浪费）
- [x] `fetch()` 的 Bytecode 分支按 `word >> 60` 分发：tag 0..=3 直接
  `Token::from_raw(word)`，消除每步解包
- 验证：M2 双轨等价测试全绿 ✅；吞吐重测 2026-09-03 完成——**比值 1.01x，≥2x 未达**
  （详见 §4 M2-6 重测记录与根因）

### P1 M2-6 小步优化：热路径消分配（每 token / 每宏调用）

- [x] `process_token`：`eqtb.slot(csid).clone()` → 按引用 match
  （省每控制序列 token 的 EqSlot 克隆，含两次 Arc refcount）——✅ 2026-08-23
  （SlotAction 两阶段模式：`&self` 读槽提取小值/单 Arc，`&mut self` 执行）
- [x] `fetch()` 的 EmitArg/宏参数分支：实参 `Arc<[Token]>` 直接复用——✅ 2026-09-03
  （新增 `InputFrame::MacroArg` 变体，引用计数 +1 直压帧；实参 token 无 noexpand
  语义（TeX 宏替换后照常展开），帧级免标记即逐位等价；去 `Vec<(Token,bool)>`
  重包装 + 二次堆分配）
- [x] `unread`/`next_is_math_shift`：单 token 回推改栈上内联槽，
  去掉每次 `Arc::from([...])`（`$$` 检测每命中一次）——✅ 2026-08-23
  （`InputFrame::One` 变体，含 `\noexpand`/`\csname` 回推）
- [ ] `call_macro` 实参 `Vec<TokenArray>` → SmallVec（实参 ≤9 个）——未做：
  需新增 `smallvec` 依赖；0 参宏本就零分配、1~2 参仅 1 次 malloc，预估收益 <5%，
  留待后续评估
- 附带（perf 火焰图定位，非原清单，行为逐位不变）——✅ 2026-09-03：
  - [x] `error_context`/`error_context_pos` 行定位改 `line_starts` 二分（原为两次
    O(pos) 全文扫描 + 整行 `from_utf8_lossy`）+ `cond.rs` 8 处条件帧行号改
    `error_line_no()`（同一 `error_anchor` 回溯语义、不建行字符串）。**每 `\if*`
    入栈都记行号**，长样张上整体 O(n²)——正是吞吐被压到 ~3.1 万 token/s 的主因
    （0b2fc9b 对 `current_line_no` 同类修复的漏网处）；修后 ~152 万 token/s。
    等价性由 `locate_line_matches_linear_scan` 性质测试锁定
  - [x] `NTEX_COND_TRACE`/`NTEX_TRACE_EXEC`/`NTEX_IFNUM_TRACE`/`NTEX_SANITY_CHECK`
    诊断开关进程级缓存（原每条件/每原语一次 `std::env::var` = getenv + String 分配）
  - [x] 看门狗心跳/last_tok/单步超时检查每步 → 每 64 步（看门狗线程 2s 轮询 +
    10s 阈值，16Hz 心跳绰绰有余；纯诊断税实测 ~10%）
- 验证：M1 全部用例双轨重跑全绿 ✅（ntex-core 262 用例）；TRIP/ETRIP 语义 diff 与
  基线 c2a2418 **逐字节一致**（干净 worktree 对照实验）；`make check` 全绿 ✅；
  火焰图热区由 `error_context` ~93% 前移至词法扫描/实参收集/sink 分散分布

### P1 正确性加固：panic 审计 + fuzz（引擎契约：任意畸形输入不 panic）

- [ ] 生产路径 unwrap/expect 审计（input.rs 10 / ntex-format 15 / ntex-io 6 等），
  输入可达路径一律改 `Error`（与 M1-13 错误模型收尾联动）
- [x] `cargo-fuzz` 目标（2026-08-25，基础设施 #2）：`ntex-layout/tests/fuzz.rs`
  随机字节喂 `Typesetter::typeset_bytes`（复用 in-process 驱动，策略：片段/ascii/
  转义深，quick 2k 轮入 CI、深 5k 轮/策略 `--ignored`）——**已修复违约 2 处**：
  - `\the\meaning` / `\the\jobname` 无限递归栈溢出（`expand_once` 缺 Meaning/
    JobName 分支，`\meaning` 被保留 → `the_tokens_after` 自递归）——expr.rs 补分支
  - 垂直模式行内数学后 `\par` 空列表栈 panic（`close_math` 后无条件
    `close_paragraph`，把唯一主列表弹空）——sink.rs 仅在仍处水平模式时关段落
  - 回归测试：`the_meaning_expands_without_recursion`、
    `the_jobname_expands_without_recursion`（ntex-core）、
    `par_after_vertical_inline_math_keeps_main_list`（ntex-layout）
  - 既有实例（2026-08-22）：`ntex-bench --driver ntex --bench expand-throughput`
    死循环——**✅ 已修复（2026-08-23）**：根因 = `knuth_plass` 折行 O(n²)
    （active 集不淘汰）+ 默认 `\tolerance=10000`（应为 200）→ active 永不淘汰。
    修复 = active 集淘汰（tex.web §880）+ `\tolerance` 默认 200。基准
    expand-throughput 现 730ms / 27.4 万调用/s；折行与暴力最优对照全绿。
- 验证：深 fuzz 2 万轮零 panic ✅；错误路径输出与 pdfTeX 一致（待 TRIP 收尾）

### P2 工程化闭环：CI/基准门禁

- [x] CI 门禁（2026-08-25，基础设施 #1）：`.github/workflows/ci.yml`
  - `check` job（门禁）：fmt + clippy（`-D warnings`）+ 单元测试
    （`cargo test --workspace`，含 fuzz quick）+ stub 冒烟（trip/diff/bench）
  - `real-engine` job（观测，`continue-on-error`）：ntex 驱动 TRIP/ETRIP/diff/
    bench + 深 fuzz，让每次提交看到"离全绿差多远"；全绿后移除
    `continue-on-error` 即转硬门禁
- [ ] `make bench` 补 `--release`（现为 debug + stub，数字无参考价值）
- [ ] nightly perf job：release 跑 `ntex-bench`（真实驱动），对照入库基线，
  吞吐下降 >15% 即失败（plan 中"基准数字入库，CI 防回归"落地）
- [ ] token 级微基准引入 divan（秒级宏基准保留手写 measure）
- 验证：check job 可全绿；TRIP/ETRIP 观测待冲刺后转门禁

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
| M2 | 字节码编译      | 字节码 IR + 编译器 + 执行器                 | TRIP 仍绿；展开吞吐 ≥ 解释器 2x（2026-09-03 重测 1.01x **未达**，结构性） |
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
  已用 ntex 驱动实跑 TRIP/ETRIP（TRIP 冲刺 a00a4d9 已推进至扫描/字体/数学错误恢复段）
- [x] 差分测试工具：同一 .tex 分别跑 pdfTeX/XeTeX 与本引擎，diff DVI/log
- [x] **设计文档（RFC）先行**，评审通过再编码：
  - RFC-1 Token 表示与内存布局 —— **已定稿**（8B token / TokenArray / InternTable / eqtb 版本化）
  - RFC-2 不可变状态模型与版本化（CoW 结构选型）—— **已由 M5 实际落地**：可回滚检查点
    捕获/写回（expand/checkpoint.rs，§7 阶段二）+ 盒子寄存器 Rc 共享·写时复制（§7 阶段五）；
    独立 RFC 文档仍待补
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
- [x] `\csname..\endcsname` 动态建 cs：命中复用、未命中追加（不可变版本切换）——✅（`exec_csname`/`\endcsname`，ETRIP 冲刺已实现）
- [x] `\meaning` 名字查询走 InternTable
- 验证：用例 1（动态建名含空 csname、非法字符）；并发预留（M6 再做原子追加）

**M1-3 eqtb 槽版本化**（RFC-1 §5）

- [x] eqtb 槽枚举：`Undefined/Macro{version, def}/Primitive(prim_id)/Register/RegisterIndex/Alias(csid)`
- [x] `\let\a\b` → `Alias(csid)` 间接（不复制宏体）；链式别名
- [x] `\mathchardef\cs=<num>` → `MathChar` 槽（ETRIP 已实现）
- [ ] `\chardef\cs=<num>` → 字符等价——待补
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

- [x] 无分隔参数：`#1..#9` 实参收集（平衡组规则）——✅（collect_undelimited_arg）
- [x] 分隔参数：分隔串按 **token 序列**匹配（RFC-1 §8 用例 7）——✅（collect_delimited_arg，7956ca7）
- [ ] `\long` 前缀（允许实参含 `\par`）——待补（`\par` 禁检查 is_par_token 已做，`\long` 前缀未接线）
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
- [x] 数学模式——✅ M4 填实（`Mode::Math/DisplayMath`，见 §6）
- 验证：模式切换错误信息与 pdfTeX 一致

**M1-13 错误模型**

- [x] 四种交互模式（`\batchmode`/`\nonstopmode`/`\scrollmode`/`\errorstopmode` + `\interactionmode` 参数，默认 batchmode）——✅（free.rs 已接线，评审补查）
- [x] 错误上下文行（"! ..." + `l.N <行内容>`）——✅ 2026-08-23（A3：`error_context`/`report_error_context`；Undefined cs 消息附 `l.N`，宏内未定义回退调用行）
- [ ] 出错后继续排版（TeX 逐错误特化恢复：`back_input`/插入恢复 token）+ `\errhelp`——待补
  （与 ETRIP 错误段联动；历史上是 trip.tex 卡在"非法输入：组未闭合"等恢复点的根因；
  3401038 后 trip.tex 已可**全程跑完不 panic**，缺恢复路径转为影响各报错原语的
  "报错后继续"路径与错误块对齐——§6 收尾决策 P0 必修项 ③）
- 验证：构造错误用例，输出与 pdfTeX 逐字符一致（待恢复机制后）

**M1-14 TRIP 冲刺**

- [x] 扫描恢复语义（2026-08-25 a00a4d9）：+ 号忽略、逗号小数点、反引号字符常量、
  嵌套条件求值、Missing number/Improper alphabetic/Bad number 钳制、溢出截断、
  大小写不敏感关键字、内部 dimen/glue 参数作尺寸
- [x] 输入层：^^ 十六进制对仅小写、控制词名内 ^^ 解码字母并入（a00a4d9）
- [x] `\write` 流 -1（log-only）与非法流号 whatsit 化（a00a4d9）
- [x] 缺控制序列恢复（`\def`/`\mathchardef` token 放回）、`#{` 参数文本终止、
  宏调用不匹配恢复（a00a4d9）
- [x] 字体恢复：`\font` 作当前字体选择器、`\textfont` 族号钳制、字体加载失败恢复、
  `\the\textfont`/`\the\scriptfont`、`\fontdimen6\the\scriptfont`（a00a4d9）
- [x] 数学/排版错误恢复：Missing $ inserted、Display math should end with $$、
  `\left` 缺 `\right`、Missing {、`\par` 受限模式、void 盒、`\showbox` void、
  多余 } 恢复、cat 15 非法字符跳过（a00a4d9）
- [x] `\mag` 整数参数（默认 1000）+ pc/cc 单位 + `\the\catcode`/`\the\output`（a00a4d9）
- [x] 测试基建：WorkDirVfs（`\input` 相对路径）+ tripos.tex 复制（a00a4d9）
- [x] 涉及排版/字体的部分用 `\hbox` 兜底占位（硬口径：语义 bug 绝不带进 M2）——
  ~~未完成~~ 2026-08-26 核查：M3 已真实实现排版，TRIP 排版段走真实路径，占位已被 M3 取代
- [x] `\muskip`/`\muskipdef`：~~1mu=1pt 硬映射待改~~——2026-09-03 与 `\muexpr` 同源销账
  （tex.web 证据链：muskip 寄存器 mu 数值刻度与 pt 相同即 TeX 本义（scan_dimen
  attach_fraction），em/18 换算仅在 layout 排版（70a8492 已做）；`\the\muskip` "X.0mu"
  显示由 `muskip_params_assign_and_the`/`muskip_register_and_muskipdef` 等单测锁定）；
  残留仅为 etrip/trip 段联调，非结构待办
- [ ] `\showbox`/`\showlists` 等诊断输出格式逐字节对齐（TRIP log diff 重灾区）——
  **按 §6 收尾决策降级**：近似可用即可，逐字节对齐放 M8 L2
- [ ] **TRIP 语义 diff 归零**（输出 diff 可读化脚本已在 M0 就绪）——2026-09-03 起
  trip.tex 已可**全程跑完不 panic**（3401038：finish 收尾冲页后清理输出例程残留列表，
  修复前停于"非法输入：组未闭合"等恢复点）；semantic diff 基线 -5619/+1412（合计
  7031）收尾中（e7db595；4cbca63 已补 `\tracingoutput` shipout 转录大块缺口）；
  缺 M1-13 错误恢复路径使报错后继续路径未全对齐；**逐字节口径按 §6 降级策略放 M8 L2**
- 文档同步（2026-08-26）：`\long`（M1-8）/`\chardef`（M1-3）/`\box`/`\muskip`（M1-10）
  代码均已实现，旧标注"待补/未实现"是文档滞后，勿再按旧标注排期
- 验证：`\input trip` 输出与参考文件一致

**M1-15 性能基线**

- [x] 记录每千 token 展开吞吐基线（**不做优化**，仅存档，供 M2 对照）

**验收**：TRIP 通过（硬口径）；错误行为与 pdfTeX 一致。
**风险**：TRIP 是"实现后才知道哪错"的黑盒 → 提前做好 TRIP 输出 diff 的可读化。
**现状**：M1 核心（1\~7、9\~11）已实现；M1-13 错误上下文行 `l.N` + 交互模式已做（2026-08-23 A3）、错误恢复（`back_input`/`\errhelp`）待补（§6 收尾决策 P0 必修项 ③）；**M1-14 TRIP 冲刺**（a00a4d9 起扫描/字体/数学/排版错误恢复 + `\mag` 等；3401038 起 trip.tex **全程跑完不 panic**；semantic diff -5619/+1412 收尾中）——TRIP 语义 diff 归零前 M1 验收项保持未勾选（逐字节全绿已按 §6 降级策略解绑，改记 M8 L2 观测项）。

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
- [x] ~~常量条件折叠：平衡 `\iftrue/\iffalse..\else..\fi` 编译期求值~~——**已撤销**（2026-08-23 A6）：TeX 的 `\if*` 是展开期求值，定义后 `\let\iftrue\iffalse` 使折叠产物与运行期语义不符（双轨不等价）——全部原样发射，由运行期条件机处理
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
- [x] **吞吐重测**（2026-09-03，release，新基准 `ntex-bench expand-dual`：20 万次宏调用
  （带参宏 + 常量条件）/ 30 万 token 输出，预热 ≥2 + 取样 ≥5 取中位，两轨输出逐 token
  断言一致）——**比值 1.01x，≥2x 未达（未达标入库）**：
  - 字节码 197ms（102 万调用/s / 152 万 token/s）vs 解释器 198ms（101 万调用/s）
  - 宏体执行占主导的对照样张（深宏链、50 万 token 输出）也仅 **1.04x**
  - 全管线 `expand-throughput`（release）：469ms / 42.6 万调用/s（ntex 驱动基准死循环
    已修通；debug 旧参考 730ms / 27.4 万调用/s）
- [x] **根因（结构性，非实现欠账）**：RFC-4 `Emit` 指令**内联 8B token 原值**
  （RFC-1 布局零解包），与解释器轨道 `TokenArray` 的 8B/token **完全同构**；执行器逐
  token 工作（取指 → 判 tag → 发射）与解释器（取 token → 判 MacroParam → 发射）等价，
  **无可消除的解码开销**。而展开吞吐大头是两轨共享的词法扫描/实参收集/eqtb 查询/条件机
  /sink（perf 实测执行器本体 <10%）——Amdahl 上限远低于 2x。≥2x 需改 IR 设计
  （宏体专用指令 / 预解码参数槽 / 常量折叠）或降共享成本（M2-5 arena）——**转决策点**
- [x] 火焰图定位热点（perf，2026-09-03）：重测前 `error_context` 占 ~93%（条件帧行号
  记录触发的 O(pos) 全文扫描 + 整行 UTF-8 转换，长样张整体 O(n²)——0b2fc9b 漏网处）；
  修复后热区前移至词法扫描/实参收集/sink（共享路径）。eqtb 槽缓存行布局、arena 等
  ——仍转 backlog P1
- 验证：基准数字入库（`expand-dual` 加入 M0 基准集，`--tex-file` 可换外部样张）；
  CI 防回归待接（基准暂不在 CI 计时断言）

**M2-7 双轨框架移交**

- [x] 双轨等价测试框架保留并文档化（M7 升级 .fmt 时继续使用）
- [x] 决策点：字节码成为默认执行路径（`Expander::new()`），解释器退为调试工具

**验收**：TRIP 在字节码路径全绿——**未达成（继承 M1-14 缺口）**；吞吐基准达标（≥ 解释器 2x）——**未达成（2026-09-03 重测 1.01x；结构性未达——RFC-4 零解包 IR 与解释器同构，见 M2-6 根因）**。
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
**进度**：**M4 全部完成**——M4-1/2/3/4/5（13 + 16 + 20 + 14 + 6 用例）、M4-6 Liang 断字、M4-7 错误模型、e-TeX 扩展（`\dimexpr`/`\glueexpr`/`\ifprimitive`/`\scantokens`，+6 用例）。ETRIP 冲刺已于 **2026-09-03 收尾转场**（A 组 42/42 ✅、B 组 36/36 ✅、C 组已全部接线，见 ETRIP-primitives.md；收尾降级口径见 §6），主线转 **M5 增量计算**（见 §7）；TRIP 收尾见 §3 M1-14。

### ETRIP 冲刺（2026-08\~09，2026-09-03 已收尾转场；原语接线全部完成）

**当前状态（2026-09-03）**：管线已跑通（fixtures + harness 泛化 + ntex 驱动）；
**pass1 全流程已走通**（e-IniTeX → `\dump`）；pass2（重载 `.fmt` 再运行）逐段推进，
`\numexpr/\dimexpr/\glueexpr/\muexpr` 全段（括号、溢出块、"Expr quotient rounding
1-8"、"Expr fraction rounding 1-3"、运算符优先级、胶水引用计数）+ `\mutoglue/\gluetomu`
段 + (mu)glue identity 段均已通过。2026-08-25 三处卡点已逐一销账/降级：
- gluestretchorder 段 "wrong glue stretch/shrink order" → **值语义销账**（
  `glue_order_value_semantics` 5 组用例，见 §6 P0 待办）；
- sparse arrays 段（etrip.tex L970 `\2\countdef`）越界 → **恢复路径已对齐**（countdef
  五连 + marks class 越界，! 消息 / read-again token / l.N / help 行逐行一致）；
- `\tracingassigns` `{changing/into}` → 主体已实现；`\the\muexpr` "5.0mu" → 实测已对。

收尾口径：`etrip.log` **逐字节比对 → 语义 diff + 错误块抽查**（基线 −3063/+2326
逐段销账中；逐字节留 M8 L2，见 §6 收尾降级策略）。

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
- **第三波批量补全（A/B 组剩余原语，2026-08-23）**：盒子操作 `\copy/\unhbox/\unvbox/
  \unhcopy/\unvcopy/\lastbox`、盒子尺寸 `\wd/\ht/\dp`、段落/断页参数
  （`\leftskip/\rightskip/\prevdepth/\interlinepenalty/\clubpenalty/\widowpenalty/
  \displaywidowpenalty`）、惩罚数组（`\interlinepenalties` 等 4 项）、列表尾操作
  （`\unskip/\lastpenalty/\unpenalty`）、诊断（`\showgroups/\showlists`）、mu 互转
  （`\mutoglue/\gluetomu`）、丢弃物（`\pagediscards/\splitdiscards/\lostchars`）、
  `\tracingparagraphs/\omit`；**Primitive 枚举 repr(u8)→repr(u16)**（变体超 256 防回绕）
  + `.fmt` codec 同步

**不符规范原语待办**（2026-08-26 盘点，按优先级；完整分组清单 + 每原语进展标记见
**[ETRIP-primitives.md](ETRIP-primitives.md)** 唯一状态源。总览：A 组 ✅ 42/42 ·
B 组 ✅ 36/36 · C 组 ✅ 全部已接线 · 收尾 ⏳（语义 diff + 错误块抽查，见 §6 降级策略））。

P0 —— ETRIP 收尾硬差距（2026-09-03 全面销账/降级，详见 §6 末尾收尾决策 + ETRIP-primitives.md）：
- [x] `\muexpr`（销账：tex.web 证据链——expander 端 mu 数值刻度 = pt 是 TeX 本义，em/18
      换算仅在 layout（70a8492 已做）；`\the\muexpr` "X.0mu" 显示已对；mu_error help1 已加）
- [x] sparse arrays 段寄存器越界恢复（2026-09-03 二轮：countdef 五连越界改走
      `write_error_help`，! 消息 / read-again token / l.N 两行 / help 2 行与参考逐行一致、
      l.970 对齐；read-again 输出 7→17 处；三轮补 marks class 越界 2 块同款）
- [x] gluestretchorder 段值语义（销账：0 分量带阶 / 负分量+阶 / 表达式·转换链 / 负号前缀，
      `glue_order_value_semantics` 5 组单测锁定）
- [x] `\tracingassigns` `{changing/into}` 行（主体已实现；引擎 72 行 vs 参考 102 行，
      尾部差异归入语义 diff 抽查）
- [ ] `etrip.log` **语义 diff 归零 + 错误块抽查**（口径从"逐字节比对"降级，见 §6 末尾
      收尾决策；逐字节留 M8 L2）

P0 —— TRIP 硬差距（M1-13 联动；3401038 起 trip.tex 已全程跑完不 panic，semantic diff
-5619/+1412 收尾中，逐字节口径已按 §6 末尾降级策略放 M8 L2）：
- [ ] 错误恢复通用机制：`back_input`/插入恢复 token + `\errhelp`——
      影响**所有报错原语**的"报错后继续路径"（扫描/模式/宏错误消息已对齐，继续跑缺）；
      **2026-09-03 降级后 P0 唯一剩余必修项**（§6 末尾收尾决策项 ③）
- [x] `\muskip`/`\muskipdef`（2026-09-03 与 `\muexpr` 同源销账：mu 数值刻度 = pt 是 TeX
      本义，em/18 换算仅在 layout（70a8492）；`\the\muskip` "X.0mu" 显示单测锁定）
- [ ] `\showbox`/`\showlists` 等诊断输出格式与 trip.log 参考逐字节对齐——
      **降级**：近似可用即可（~3000 行树形诊断差属 L2），M8 再对齐

P1 —— D 组语义简化点（REVIEW-2026-08-23，偏离规范但可接受）：
- [ ] `\insert` 只收集不排版（脚注不可用）
- [ ] `\badness` 等只读整数单独出现为 no-op（规范应报错）
- [ ] `\omit` no-op 占位 + `\halign/\valign/\cr/\noalign/\span` 对齐语义未实现
- [ ] 数学矩阵（`\matrix`/`\eqalign` 等）
- [ ] `\scriptfont` 未接真实字体
- [ ] `\vsplit` marks 拆分暂空（`\splitfirstmarks` 等返回空）
- [ ] `\output` 例程消费判定用 count 启发式（应显式 `\shipout\box255`）
- [ ] `\lefthyphenmin`/`\righthyphenmin` 词界规则待校准

P2 —— 架构决策：
- [ ] A5 输入层 8-bit catcode：CJK 多字节被切单字节 token，影响所有字符类原语
      （P0 决策项，需独立排期）

**冲刺纪律**：每次迭代前先 `cargo build -p ntex-trip` 确认全绿再跑（避免脏构建旧产物
误报，如误报过的 `\ifcase 序号不能为负`）；对照 etrip.log 参考逐段验证，不做整体 diff。

### ETRIP/TRIP 冲刺收尾降级策略（2026-09-03 决策）

> 依据：M3 已达成 DVI 与真实 TeX 逐字节一致、M4 功能全落地、引擎已能跑完 etrip.tex
> 全程（semantic diff −3063/+2326，逐段销账中）——内核语义正确性已基本被证明；
> TRIP/ETRIP 未全绿的残余项多为"教科书级错误用例 + log 逐字节格式"类边际收益递减
> 工作。据此把收尾口径定为**内核正确性优先、log 字节对齐止损转场**，不为 L2 提前付利息。

**必要项（做，P0 语义正确性）**：
- **错误恢复通用机制**（M1-13 `back_input`/`\errhelp`，TRIP 卡点根因）：引擎契约级能力，
  直接关联"任意畸形输入不 panic"契约（AGENTS.md §4），且是通用机制——不做则所有报错
  原语缺"报错后继续"路径；
- ~~**真语义 bug 修复**：`\muexpr` 1mu=em/18 换算 + `\the\muexpr` "5.0mu" 显示 + mu_error
  恢复；gluestretchorder/glueshrinkorder 值语义~~ —— ✅ 2026-09-03 全部清除：
  `\muexpr` 经 tex.web 证据链销账（mu 刻度=pt 是 TeX 本义，em/18 换算仅在 layout，
  70a8492 已做；`\the\muexpr` "X.0mu" 显示实测已对）；胶水阶值语义经
  `glue_order_value_semantics` 5 组用例锁定（0 分量带阶、负分量+阶、表达式/转换链、
  负号前缀）。剩余错误恢复通用机制见上条。

**降级项（放过字节格式，逐字节口径留 M8 L2）**：
- `etrip.log` **逐字节比对 → 语义 diff + 错误块抽查**（read-again 与 l.N 间独立 `...`
  省略行、两行光标未全覆盖、`\write` 转录空格、79 列断行等格式差不再逐字节死磕）；
- `\showbox`/`\showlists` 诊断输出格式逐字节对齐、TRIP log 格式项——近似可用即可，
  L2（字节兼容）阶段再对齐（M8）。

**收尾验收口径（调整后）**：
- semantic diff 归零（语义层）+ 错误路径不崩溃 + fuzz 零 panic + M2 双轨等价全绿；
- TRIP/ETRIP "逐字节全绿"不再作为 M1/M4 验收的唯一口径，改记 M8 L2 观测项；
- 本决策为文档状态源，与 ETRIP-primitives.md 收尾口径保持一致。

**转场**：~~上述收尾完成后主线转 M5~~ —— ✅ 已转场（2026-09-03，见下"转场执行决策"：
必修 ①② 清除后同日转入 **M5 增量计算**，阶段一\~五落地，见 §7）；性能 backlog **P1**
（热路径消分配 +61% / expand-throughput 469ms 入库）已并行落地。

**转场执行决策（2026-09-03 补充，冲刺排序）**：

> 现状盘点：M1/M4 验收已与 TRIP/ETRIP"逐字节全绿"解绑（改记 M8 L2 观测项，见上）——
> 验收口径上转场 M5 已无硬卡点。**转场已执行（2026-09-03 同日）**：必修 ①② 清除后即
> 转 M5，阶段一\~五落地（段级增量重算 → 可回滚快照·edit 单路径化 → 排版层端到端 → 依赖
> 判定 → 复用开销削减·墙钟翻正，见 §7）；③ 错误恢复通用机制与 M5 阶段六并行推进。

- **必修项执行顺序**（P0 语义正确性；①/② 已于 2026-09-03 清除，剩 ③）：
  1. ✅ `\muexpr`（2026-09-03 销账：tex.web 证据链——expander 端 mu 数值刻度与 pt
     相同即 TeX 本义（scan_dimen attach_fraction），em/18 换算仅在 layout（70a8492 已
     做）；mu_error help1 行已加；原"expander 1mu=em/18 重构"为误标，见
     ETRIP-primitives.md §A）；
  2. ✅ gluestretchorder/glueshrinkorder 值语义（2026-09-03 销账：0 分量带阶保留、
     负分量+阶、表达式/\mutoglue 链、负号前缀均与参考 etrip.log 一致，单测
     glue_order_value_semantics 5 组锁）；
  3. M1-13 错误恢复通用机制（`back_input`/`\errhelp`，TRIP 卡点根因）——较大，
     契约级（AGENTS.md §4 任意畸形输入不 panic），所有"报错后继续"原语共用路径。
- **并行先行项**（不依赖收尾，2026-09-03 更新）：性能 backlog **P1**——消分配已落地
  （`buf_stack` 去分配，展开吞吐 +61%），`expand-throughput`（release）469ms /
  42.6 万调用/s 已入库（33732bd），吞吐量化见 §4 M2-6；**M5 架构设计 → 直接落地**：
  求值图纯函数化 `Expand(source, snapshot)` + 依赖追踪已随 M5 阶段一\~五实际落地（§7），
  纯函数化收尾列为 §7 阶段六待办。
- **暂缓项**：**M7** .fmt v2（等吞吐量化——纪律：先量化再冲，不在 1.12x 基线上做增量
  吞吐，M2 ≥2x 验收量化——2026-09-03 已量化 1.01x 且属结构性，见 §4 M2-6）；**M6**
  并行（等 M5 副作用隔离成果落地）。

```
✅ 必修 ①\muexpr ②glue order 销账（2026-09-03）→ 主线已转 M5（同日，阶段一~五落地，§7）
   ③back_input/\errhelp（P0 唯一剩余必修）与 M5 阶段六并行推进
        ├─ 并行：P1（消分配 +61%，expand-throughput 469ms/42.6 万调用/s 入库）
        │        / TRIP·ETRIP semantic diff 收尾（基线 -5619/+1412 与 −3063/+2326 归零中）
        └─ 暂缓：M6（等 M5 副作用边界成果）/ M7（.fmt v2，M2 已量化 1.01x 结构性，见 §4 M2-6）
收尾记账：semantic diff 归零 + 错误路径不崩溃 + fuzz 零 panic + M2 双轨等价全绿
        └─ 逐字节口径（TRIP/ETRIP log · \showbox）记 M8 L2
```

***

## 7. M5 增量计算（相对 cTeX 的胜负手）

**目标**：编辑体验级的增量重算。

- [ ] 求值图：`Expand(source, snapshot) → (tokens, snapshot')` 纯函数化
      —— 部分落地（可回滚检查点捕获/写回，阶段二）；纯函数化收尾在阶段六
- [x] 依赖追踪：记录每个结果依赖的 catcode/宏定义/计数器/上游段
      —— ✅ 落地（词法读/写依赖 SegmentDeps，阶段一/四）
- [x] 失效传播：编辑定位 → 只重算失效子图 → Box 级缓存复用
      —— ✅ 落地（段缓存复用 + 行盒免重排；页面装配按需重跑，阶段三/四）
- [ ] 副作用边界：`\write` 延迟到 `\shipout`；aux/toc 增量更新与去重合并
      —— `\write` shipout 边界 M3/RFC-3 已落地；aux/toc 增量在阶段六
- [ ] `\the\count` 等全局读 → 数据依赖登记（防缓存失效错误）
      —— 词法/寄存器读依赖已登记（阶段二/四）；随槽级归因（阶段六）再校口径
- [ ] `.aux`/`.toc` 增量；两次编译收敛语义不变 —— 阶段六
- [ ] **基准**：改 1 字 → 重算耗时 vs 全量重编（目标：差 ≥ 100x）
      —— 2026-09-03 实测 120 段文档改正文 4.3x / 改宏体 1.1x（release，阶段五）

**验收**：增量结果与全量重编逐位一致（随机编辑模糊测试）。
**风险**：副作用漏追踪 → 缓存错 → 必须用"增量 vs 全量 diff"作 CI 常驻检查。

**实施进度（2026-09-03，主线已转入 M5 并推进至阶段五）**：`ntex-core/src/incremental/`
（expand 层）+ `ntex-layout` `IncrementalTypesetter`（排版层）逐段推进，铁律
「增量结果与全量重跑**逐位一致**」全程由测试锁死。提交序列 0099b1f → 7601d1d →
c2a2418 → b585864 → 55bcb09 → 14f7bc7：

- **阶段一 POC**（0099b1f，expand 层）：文本级切段（空行/行首 `\par`，注释与转义
  感知，拼接恒等不变式）+ 段边界轻量快照（eqtb 逐槽**语义**比较 + 寄存器
  `Registers::dirty` 影子表 + 其余状态指纹）+ 词法读/写依赖与闭包 + `edit` 双路径
  + 状态中性段跳过；增量与全量逐位一致单测锁定；基准 501 段改 1 段 6.2x（廉价）/
  2.3x（保底）
- **阶段二**（c2a2418；前置 7601d1d 修 `\foo@bar` 类非字母宏名词法依赖漏记 → 缓存
  错用）：**可回滚完整检查点**（expand/checkpoint.rs：eqtb 槽/ValueState/ValueExtras
  /ControlState 四部分，纯捕获/写回分片、零语义分支）→ `edit` **单路径化**（引擎
  整体回滚到段前快照 + 从该段起重放，保底路径整体删除）；判定改「执行段后一次链
  偏差全量扫描（ChainDelta）+ 各缓存段查偏差集 ∩ 依赖」，加宏槽 `Arc::ptr_eq`
  快路径与捕获期预计算状态中性；501 段基准场景 A 改正文 ~56x / B 改宏体 ~26x
- **阶段三**（b585864）：段级增量延伸至**排版层端到端**（IncrementalTypesetter）——
  段边界排版检查点（NodeBuilder 克隆 + shipped 计数 + expand StateSnapshot）+
  主列表**节点流录制**（缓存段重新注入、行盒免重排、页面装配重跑——编辑造成的
  页面后移自然传播到其后各页）；增量与全量 typeset_dvi 逐位一致（多页真实文档
  首/中/末段、公式/列表段、连续多次编辑三重口径）；回归修复 paging.rs
  `eject_one_page` 无限冲页（残留 [空盒,vfill,惩罚] 三元组自持循环，千万空页
  OOM）等 2 处
- **阶段四**（55bcb09）：**排版层依赖判定（宏体编辑省重排）**——录制词法读/写
  依赖（SegmentDeps）+ 状态中性；复用判定两级化：偏差为空 → 同步态直接复用；
  偏差非空 → 四闸（值状态/`\csname`/读闭包/写集 + 状态中性，同 expand 层
  `cache_reject_reason_inner`）+ 排版半边闸（`side_effects_match`）——改宏体后
  未引用段缓存复用，仅读闭包触及被改宏槽的段重排；`record_post` 与回滚边界解耦
  （修偏差路径把上轮状态冲进活状态的连续编辑不一致隐患）；端到端基准入库
- **阶段五**（14f7bc7）：**复用判定开销削减（增量墙钟翻正）**——盒子寄存器文件
  `Rc` 共享 + 写时复制（克隆/对齐退化为引用计数，闸比较走指针相等 BoxFile）；
  链偏差跨段携带（复用段不重扫全量状态，只查"偏差集 ∩ 依赖"）；段边界两档化
  `DocBoundary{side, rollback}`（每段只捕获排版副作用字段快照 ~1µs，expand
  检查点 + builder 整份克隆按 `ROLLBACK_EVERY=8` 稀疏化，段出错撤销其后回滚点）；
  **实测（release / 120 段文档）：改正文 4.3x（13.4ms vs 58ms，复用 111/169）/
  改宏体 1.1x（30.1ms vs 33ms，复用 60/121）——增量墙钟首次 < 全量**；
  incremental 14 项 + workspace 全测试、make check 全绿

**仍存边界**（离 M5 目标还差什么，incremental.rs 模块文档同步）：**执行段成本 ≈
全量段成本**（执行段每段多付两次 expand 检查点捕获 + 词法依赖提取 ~55µs——改宏体
场景执行段占多数时加速比被压 ~1.1x，要再上台阶须把检查点捕获做成**增量维护**，
ntex-core 引擎侧）；**副作用边界**（`\output` 例程回放、`\write` 流内容、`\input`
VFS 注入不在逐位一致口径——注入路径观测到即禁用复用保守全量重排；字体表随管线
单调增长、marks 族读取段亦不在口径内）；**内存** O(文档) 量级。

**阶段六待办**：检查点捕获增量维护（执行段成本削减）；副作用边界（`\output` 例程
/`\write` 流内容/`\input` VFS 注入/marks 语义）；`Expand(source, snapshot)` 纯
函数化；寄存器/参数**槽级归因**（消除偏差全局失效）；**随机编辑模糊测试**（增量
vs 全量 diff 作 CI 常驻检查，闭环 §7 风险项）；`.aux`/`.toc` 增量；基准冲 100x
（现改正文 4.3x / 改宏体 1.1x，2026-09-03 release 口径）。

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
- [x] **渲染后端**（2026-09-05 选型由 Skia 改为 vello：纯 Rust/wgpu 栈、WASM 同构、免 C++ 绑定，见 §13）：`crates/ntex-backend` 落地 `Backend` trait + 双实现——软光栅（离线环境自研位图/PNG）+ **vello 0.10 GPU 路径**（wgpu 29 无头渲染 Rgba8Unorm 纹理回读，area 亚像素 AA）。两后端共享「盒树 → 矩形指令」prims 遍历，demo 差分：墨区面积差 0.15%、几何完全重合（差异仅亚像素边缘表现）；GPU 不可用报错回落软光栅。CLI：`cargo run -p ntex-backend -- demo.tex out 144 --vello`。字符暂为占位方框（TFM 无轮廓，M9 字形在同一 prims 层扩展曲线指令，两后端同步受益）
- [x] **排版调试 overlay**（2026-09-05）：prims 层独立 `debug` 通道（`RenderOptions::debug` / CLI `--debug`）——版心+盒边界描边（HBox 蓝/VBox 紫红）、基线（青）、glue 自然宽带+stretch/shrink 指示线（fil 阶亮绿）、kern 橙线、penalty 断点标记（禁断深红粗线）；两后端在内容之后绘制，`rects` 通道与差分口径零变化（demo 实测 GPU/软光栅 debug 位图逐字节一致）
- [x] **实时预览工作台**（2026-09-05，M8 §10 WASM 预览的桌面先行形态）：`crates/ntex-studio`——左侧 TeX 语法高亮编辑器（注释/控制序列/数学/组符分色）+ 右侧 vello GPU 表面渲染。egui-wgpu 0.35 三段式回调桥接（prepare 离屏 Rgba8Unorm 矢量光栅化 → paint 全屏 quad blit），缩放/平移只改 Scene 仿射、始终按显示分辨率重光栅化（放大不糊）；250ms 防抖同步重排，dpi/调试 overlay 开关、翻页、编译错误进状态栏不 panic。**依赖硬约束：eframe 0.35 ↔ vello 0.10 恰共用 wgpu 29**（eframe 0.36 已用 wgpu 30 会分裂，升级前必验）。`ntex-backend::build_scene` 抽为公共 Scene 构建（无头回读与 GUI 共用同一事实源）
- [ ] WASM 前端：wasm-bindgen + Canvas2D/WebGL 实时预览（接 M5 增量，毫秒级刷新）
  ——**骨架已建（A 档，2026-09-06）**：`crates/ntex-wasm`（cdylib）——`compile_tex(tex) →
  DVI 字节 + 转录 + 页数 + 字体清单`，plain 子集、无渲染；TFM 经
  `ntex_layout::set_tfm_source` 注入（内嵌 6 个 CM TFM，7.7 KB）；wasm32 分叉点只有
  三处且全部带注释（`param.rs` 时间固定 1970-01-01、`expand::run` 看门狗整段门控、
  `TfmLoader` 字节源分叉），native 行为逐字节不变（`cargo test --workspace` 全绿、
  ntex-trip 失败签名与基线一致）。B 档（vello/wgpu web 渲染）另案，C 档（LaTeX `.fmt`
  载入）待 `ntex-format` 快照完备；详见 `crates/ntex-wasm/README.md`
- [ ] SyncTeX 源码映射（IDE 点击跳转）

**验收**：简单文档 L2 一致；WASM 演示在浏览器增量预览流畅。

***

## 11. M9 生态冲刺

**目标**：中文生态落地。

### 11.0 中文支持依赖链与分阶段规划（2026-09-04 立项细化）

> 背景：LaTeX 兼容战役（latex.ltx/expl3 载入，docs/latex-feasibility.md）进行中。
> 中文支持是**独立战役**，不混入当前验证口径（战役以 latex.ltx/英文文档为准）；
> 本节固化依赖链、缺口清单与分阶段验收，防止 M9 展开时才发现前置缺口。

**依赖链**（严格顺序，不可跳）：

```
LaTeX 兼容战役（进行中）
  → ① \documentclass{article} + hello world 排版通（英文全链：加载→排版→DVI→PDF）
  → ② 字体子系统升级（ntex-font M9 部分：TTF/OTF 解析 + HarfBuzz 整形，
      替代 TFM-only 度量——中文每字都是 Unicode char，8-bit TFM 装不下）
  → ③ 输入层 UTF-8（REVIEW 表 A5，未修——中文文档前提）
  → ④ CJK 原语面（对齐 XeTeX/LuaTeX，非 pdfTeX）：
     \XeTeXlinebreaklocale 类断行钩子、CJK 字距/标点挤压、Unicode 编码向量
  → ⑤ ctex/xeCJK 宏兼容（本节其余条目）
  → ⑥ 中文文档端到端验收（300 页中文冷编基准，§11 验收）
```

**缺口清单**（每项 = 未来一刀/一 PR 粒度）：

| 缺口 | 现状 | 前置 |
|---|---|---|
| TTF/OTF 字体解析 | 无（TFM-only，cmr10 单字体） | ② |
| HarfBuzz 整形/整形缓存 | 无（M6 铺路条目已列） | ② |
| 输入层 UTF-8 | REVIEW A5 未修 | ③ |
| CJK 断行规则（linebreak locale） | 无 | ④ |
| 中文标点挤压/字距 | 无 | ④ |
| ctex 宏包兼容 | 无（xeCJK 最小子集起步） | ⑤ |
| 中文 TTF 字体备料 | 无（Fandol/Noto CJK 待取） | ② |

**分阶段验收标准**：

- **阶段 A（英文 LaTeX 全链通）**= LaTeX 兼容战役出口：`\documentclass{article}`
  文档 `\input`→排版→DVI→PDF 与真实 TeX 输出对照可接受（阻塞点清零）。
  **此前一切中文工作不开始**（避免双线作战）。
- **阶段 B（中文冒烟）**：`\documentclass{ctexart}` 或 xeCJK 最小集，一行中文
  排版出 PDF（字形正确、无 missing glyph）——字体子系统 + UTF-8 + 最小 CJK 原语。
- **阶段 C（ctex 常用面）**：ctex 文档类常用命令（\section/\ctexset/中文目录名）、
  标点挤压、中英混排断行；300 页中文冷编基准达标（§11 验收）。

**备料约定**（对齐 LaTeX 兼容战役 §21.4 模式）：中文 TTF（Fandol/思源/Noto CJK）
从 CTAN tlnet archive 按需取，入库路径与 license 记录同步 plan/报告。

- [ ] ctex/xeCJK 宏兼容（这本身是巨型工作量，从 xeCJK 最小子集开始）
- [ ] OpenType 字体：fontspec 兼容路径 + HarfBuzz 复杂整形
- [ ] **CJK 整形捷径**：无复杂特性时跳过 HarfBuzz，直接读 hmtx（目标 5\~10x）
- [ ] 宏包 CI 回归集：geometry、amsmath、hyperref、biblatex、tikz、ctex
- [ ] **宏包管理**（用户需求，2026-08-25）：借鉴 Go 包管理器（go.mod 依赖声明、
  版本化模块仓库、go.sum 校验和、本地模块缓存）设计宏包仓库与依赖解析机制，
  实现 `.sty`/`.cls` 宏包的版本化安装/更新/依赖管理
- [ ] 引擎身份模拟（`\pdftexversion` 等），兼容依赖引擎行为的宏包
- [ ] **MCP server / AI 工具链**（用户需求，2026-08-25）：stdio JSON-RPC 服务，复用
  `Typesetter::typeset_dvi` 库接口 + `ntex-io::MemVfs`（无落盘内存 VFS），暴露三种工具形态：
  - ① 排版渲染：TeX/LaTeX 源码 → PDF（base64 返回；纯 Rust 单二进制部署，
    无需目标机装 TeX Live——对比裸调 pdflatex 的核心卖点）
  - ② 宏展开/诊断：TeX 片段 → 展开后 token 流 + 结构化错误（复用 M1-13 错误模型，
    任意畸形输入不 panic 的引擎契约正适合 AI 驱动）
  - ③ 会话式增量编译：接 M5 增量计算，多轮编辑只重排改动段落（差异化卖点）
  - 安全基线 = RFC-3 副作用隔离（`\write18` 拒绝、写文件走 VFS shipout 边界提交）
  - 排期依赖：形态①依赖输出端（已就绪），纯 TeX 子集即可起步；
    形态②随 M1 错误模型收尾接线；形态③依赖 M5
- [ ] MCP 工具验收：`tex→PDF` 端到端经 stdio 调用，返回 PDF + 结构化日志/错误（首个里程碑口径）

**验收**：目标宏包回归全绿；300 页中文冷编基准达标；MCP `tex→PDF` 工具经 stdio 端到端可调用。

***

## 12. Crate 划分建议（Rust workspace）

| Crate                    | 职责                                                                                                                        | 现状                                                    |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| `ntex-core`              | token（8B tagged union）、catcode、InternTable、eqtb 版本化、展开引擎、**字节码 IR/编译器/执行器（M2 落在此 crate，`ntex-vm`** **未单建）**、内部参数、sink 事件流 | ✅ 已建                                                  |
| `ntex-layout`            | 节点、**主循环（模式状态机）**、badness/断行点、Knuth-Plass、TFM 字体接入、断页、数学排版                                                                | ✅ 已建（M3-1\~4 完成）                                      |
| `ntex-pdf`               | **正式 DVI → PDF 后端**：DVI 解析 + PDF 1.4 写出 + Type1(PFB) 嵌入；替换临时切片                                                            | ✅ 已建（M8）                                              |
| `ntex-font`              | TFM/OFM、ttf-parser、HarfBuzz 整形、整形缓存                                                                                       | ✅ 已建（M3-4：TFM 解析 + `\font` 加载 + 缩放；ttf/HarfBuzz 待 M9） |
| `ntex-format`            | .fmt 序列化/反序列化（v1 快照已完成；v2 mmap 零拷贝 + 字节码固化 + 部分求值）                                                                        | ✅ 已建（M3 收尾：v1 内存快照；v2 待 M7）                           |
| `ntex-incremental`（不单建）| 增量计算：段级重算 / 可回滚检查点 / 依赖追踪 / 失效传播——落在 `ntex-core/src/incremental/`（expand 层）+ `ntex-layout` `IncrementalTypesetter`（排版层），同 RFC-4 字节码入 core 先例 | ✅ 已落地（M5 阶段一\~五，见 §7）                            |
| `ntex-io`                | VFS、aux 增量                                                                                                                | ✅ 已建（RFC-3：Vfs trait + LocalVfs/MemVfs + 读写原语）        |
| `ntex-backend`           | 渲染后端：`Backend` trait + 软光栅 + vello GPU（wgpu 无头纹理回读，M8）+ PNG 导出；与软光栅共享 prims 遍历可差分                          | ✅ 已建（2026-09-05，M8）                                  |
| `ntex-cli` / `ntex-wasm` | 命令行 / WASM 前端                                                                                                             | `ntex-wasm` **骨架已建**（2026-09-06，M8 A 档：引擎核心 plain 子集 → DVI + log；渲染另案 B 档）；`ntex-cli` 未建（M9）                                                |
| `ntex-mcp`              | **MCP server（stdio JSON-RPC）**：tex→PDF / 宏展开诊断 / 增量会话（M9 生态，复用 ntex-cli 库化接口 + ntex-io MemVfs）                       | 未建（M9）                                                |

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
7. **MCP 工具形态优先级**（2026-08-25）：先做"排版渲染"（依赖输出端即可起步，纯 TeX
   子集）；"宏展开/诊断"随 M1 错误模型收尾即可接；"会话式增量编译"依赖 M5。三项
   不阻塞主线，M9 生态冲刺期实施，不影响 L1 兼容 / .fmt / 增量主线排期。

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

