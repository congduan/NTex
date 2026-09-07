# 已知简化实现与技术债清单（KNOWN SIMPLIFICATIONS）

> 维护规则：**新增任何"简化/no-op/暂不"实现时，必须在此登记**（文件:行）。
> 修复某项后，从清单移除并注明 commit。本清单是"踩坑前先查表"的索引，
> 防止同一领域反复探路（如数学状态机、对齐组）。

## 1. 数学原语（存在性测试/消费参数——最高风险区）

| 位置 | 现状 | 影响 | 状态 |
|---|---|---|---|
| `expand/builtins.rs:296` | 数学原语批量"存在性测试" | 参数被吃掉，数学列表节点缺失 | ⚠️ 部分已修（mkern/mskip mu 上下文 3a2cb63；fraction 族 721646c；其余逐项排查） |
| `typeset/math.rs:131` | math_char_tok 字符 fam 恒 0（\mathcode/\fam 未解析） | \scriptfont undefined 检查不可行（直接检查 127 次爆炸，已回退） | 待做（C 类：fam 解析前置） |
| `expand/primitive.rs:753` | 数学原语"简化实现消费参数" | 同上 | 逐项排查中 |
| `eqtb/primitive.rs:559` | 同 753 | 同上 | 同上 |
| `expand/primitive.rs:780-800` | **fraction 原语族（\abovewithdelims/\above/\atopwithdelims/\overwithdelims）此前只扫参数不挂 sink.math_fraction** | 分子分母混收当前层，trip l.276 数学状态崩 | ✅ 已修（721646c；同层嵌套歧义改恢复式，参考 l.257 Ambiguous） |
| `expand/primitive.rs:726` / `eqtb/primitive.rs:382` | `\vcenter` 简化按 vbox | d 组待做（收集不执行） | 待做 |
| `expand/primitive_math.rs:188` | `\eqno/\leqno` 数学内 no-op；非数学报错+pretend 已补（942346e） | 显示公式编号不落节点 | ⚠️ 数学内 no-op 属功能未做；多报已随隐含组修复消除（f75a638） |
| `typeset/sink.rs:1082` | 数学模式 `\penalty` 忽略 | 数学断行点缺失（M4-1） | 待做 |
| `typeset/sink.rs:1097` | 数学模式 `\vrule` 忽略 | 规则原子缺失 | 待做 |
| `typeset/math.rs:495` | 分式节点 M4-2 简化（垂直堆叠） | 分式线/字号精化未做（M4-3 fontdimen） | 待做 |
| `typeset/math.rs:540` | 根式节点 M4-2 简化（横线） | cmex10 根号未换（M4-3） | 待做 |
| `typeset/sink.rs:623` | `\radical` 定界符号不参与渲染 | `\radical"161` 等只出 radicand | 待做（l.412 mathord 报错已随隐含组 f75a638 修复消除） |
| `typeset/sink.rs:116` | 非数学模式样式错误（原"简化忽略"） | TeX 报错缺失 | ✅ 已修（942346e math_mode_error；TRIP/ETRIP 未触发） |

## 2. 数学状态机（本轮 l.260-285 暴露；见 MATH-STATE-MACHINE.md）

- `math_shift` 嵌套语义：数学模式内 `$$` 已修（3a2cb63），`\scriptfont 未定义检查` 缺失（trip L260 `$\x` 报错对齐参考）
- 数学内 `$`/`$$` 的 display 探测消费规则（expander mod.rs:1490）
- 数学模式组结束 `Missing $ inserted` 检查（sink.rs group_end，entered_math + box 组排除）
- 数学原语 no-op 的读值语义（`\splitdiscards` 等，9b0bc69 已修）

## 3. 对齐组（halign/valign）

| 位置 | 现状 | 状态 |
|---|---|---|
| `expand/primitive.rs:680` | `\cr` 无操作（对齐组按盒子处理） | 待做 |
| `expand/primitive.rs:707` | `\span` 列合并无操作 | 待做 |
| `expand/primitive.rs:805` | `\crcr` 与 `\-` 简化 no-op | 待做 |
| `expand/primitive.rs:1356` / `eqtb/primitive.rs:504` | `\omit` 简化为 no-op | 待做 |
| `typeset/sink.rs:488` | 组类型 7 不另开列表（沿用对齐组列表） | 待做 |
| `typeset/sink.rs:1483` | 数据行边界（此前 no-op 导致列内容混入 vbox，已修） | ✅ 已修 |

## 4. 内部量单独出现（no-op 语义——已系统性修完，勿回退）

- `expand/primitive.rs:55, 355, 1031, 1033, 1059, 1368`：内部整数/只读整数/胶水分量查询单独出现一律 no-op
  （TeX 主循环不读值；参考 trip `{\tracingstats}` 追踪后无操作；9b0bc69 统一）
- `expand/scan.rs:1212`：`\pagegoal` 等排版状态参数暂按 0 读（expander 无排版状态）

## 5. 字体/连字/断字

| 位置 | 现状 | 状态 |
|---|---|---|
| `ntex-font/tfm.rs:191` | 保留左/右字符的连字（罕见）暂不支持 | 待做 |
| `expand/primitive.rs:893` | `\varunit` 字体单位 no-op | ✅ 无单独场景（TRIP 仅 dimen 上下文 `20\varunit`） |
| `expand/save.rs:656` | `\the\font` 简化（expander 无排版状态） | 待做 |
| `hyphen.rs:10,47` | 词界限制 `.` 暂不参与断点过滤 | 待做 |
| `expand/primitive.rs:1492` | 断字表单语言全局（sink 不分语言；无 lccode 二次比较） | 待做 |

## 6. 其他

| 位置 | 现状 | 状态 |
|---|---|---|
| `expand/macros.rs:175,323` / `save.rs` | `\outer` 限制语义 | ⚠️ 部分已修（a4c2aeb）：展开上下文禁止（宏体/实参——scan_depth>0）+ \def 体跳过 + cs 名 ^^ 转义；Runaway 块/对齐模板场景待错误恢复链统一 |
| `expand/io.rs:304` | `\write18` shell 转义拒绝（Error） | ✅ 设计如此（RFC-3 副作用隔离；TRIP/ETRIP 不触发） |
| `typeset/sink.rs:1519,1525` / `mod.rs:841` | `\moveleft/\moveright` 位移不落节点（取走即清） | 待做 |
| `typeset/sink.rs:1101` | 非引导上下文未定宽度简化落 0 | 待做 |
| `typeset/sink.rs:1439` | `\showlists` 简化转录（诊断用） | 待做 |
| `expand/primitive.rs:2684` | `\showifs` 简化格式 | ✅ 诊断原语（ETRIP l.651 被错误交互打断，无直接比对场景） |
| `ntex-trip/harness.rs:8` | ETRIP 终端输出经 dvitype 比对暂不纳入 | 待做 |
| `expand/macros.rs:844` | 宏不复制宏体（M1 简化） | 待做 |
| `expand/scan.rs:32` | 十进制扫描 M1 简化版 | 待做 |

---

## 7. demo1 数学/输出一致性战果（2026-09-07 登记）

demo1 六刀 + 输出例程刀 2/3/5 的修复登记；全部已提交，留作回归排查索引（踩坑前先查表）。

| 位置 | 事项 | 状态 |
|---|---|---|
| `typeset/typesetter.rs`（tex.web §4852） | 大写字母 `\sfcode=999` 初始化（原 INITEX 初表清零、须 plain 赋值一类） | ✅ 已修（e0fb4fd） |
| `typeset/math.rs` + `ntex-font/tfm.rs` | 斜体修正 kern 全链（tex.web §759；无下标才落、cmmi10 E=37773 截断） | ✅ 已修（0236876） |
| `typeset/math.rs:1069` | mu→sp 整数截断 `mu_to_sp`（`cur_mu=em/18` 截断不可省——demo1 对照 5×36408，精确除法差 4sp） | ✅ 已修（70a8492 缩放 + cf048af 截断语义） |
| `typeset/math.rs` / `mod.rs` | display 公式盒 interline glue（tex.web 垂直列表 + append_to_vlist，非"水平嵌 hbox"） | ✅ 已修（2fa7e4b） |
| 同上（退出时长短 skip 裁决） | pre_display_size 长短 skip 裁决 | ✅ 已修（2fa7e4b） |
| `ntex-dvi/lib.rs` | 页号链 count0 页标签（原 `[0.0.0.0.N]` 硬编码） | ✅ 已修（375b390） |
| `typeset/mod.rs` / `sink.rs` | box255 寄存器化统一访问面（box_view/take_box_at/write_box 三访问面） | ✅ 已修（c5d02b9） |
| `typeset/paging.rs` / `node.rs` | insert 结构化 token 体保留 | ⚠️ 部分已修（577ed3c）：体已保留，**体排版仍挂账**（脚注仍不可用，见 AGENTS.md §7） |
| `ntex-dvi` transcript 通道 + `--input-path` | 诊断转录（stderr 默认开）+ `\input` 搜索路径（SearchPathVfs）——补"undefined cs 静默跳过"盲区 | ✅ 已修（4ec6a84） |

---

## 8. 架构债（2026-09-07 架构评估登记）

结构性债务，非语义简化；与逐项修复分开追踪。括号内为 R3 当日实测，供治理时校准。

| 对象 | 问题 | 状态 |
|---|---|---|
| `ntex-core/src/sink.rs:49` `TokenSink` | 单接口百级方法（评估口径 116；实测 trait 内 107 个 fn），VM↔排版耦合面过宽 | R1 治理中 |
| `ntex-layout/src/typeset/mod.rs:511` `NodeBuilder` | 上帝对象（评估口径 51 字段；实测 62，511-679 行），状态难回滚/难并行 | 待 R1 后的 R1b |
| `ntex-layout/src/typeset/tests.rs`（2606 行）、`ntex-core/src/expand/tests.rs`（4300 行） | 巨型测试文件，定位与并行编辑困难 | R2 治理中 |
| `ntex-dvi/src/lib.rs:37-44` `write_dvi` | 旧驱动仍用合成 counter（count0=页序号，刀 5 残留）——换装 `write_dvi_with_counts` 即可 | 待做 |

---

## 维护记录

- 2026-09-02：建清单（58 处标记扫描归档）；数学内 `$$`、`\mkern/\mskip` mu 上下文已修（3a2cb63）
- 2026-09-02：内部量 no-op 统一、`\right` 缺配对恢复、数学模式组结束 Missing $（9b0bc69）
- 2026-09-02：fraction 原语族挂载 + Ambiguous 恢复式（721646c）；\tracingcommands2 可选 `=` 赋值开启追踪（6cc16f0）；mode_name internal vertical（483b640）；\if 求值 {true}/{false}（3a5ec1e）
- 2026-09-02：\eqno/\leqno 非数学报错 + math_style 报错（942346e）；\outer 展开上下文禁止（a4c2aeb，R3 补登 commit）
- 2026-09-07：R3 账实同步——5 处"未提交"悬空项核实归位（eqno 多报 f75a638、radical l.412 报错 f75a638、math_mode_error 942346e、\outer a4c2aeb，位置列顺带刷新到分片后路径）；demo1 六刀 + 输出例程刀 2/3/5 战果登记（§7）；架构债小节新设（§8）
- **收尾纪律提醒**：后续每轮修复后同步更新本清单（已修项标 ✅ + commit；维护记录追加）
