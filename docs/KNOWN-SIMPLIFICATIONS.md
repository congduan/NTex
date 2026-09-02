# 已知简化实现与技术债清单（KNOWN SIMPLIFICATIONS）

> 维护规则：**新增任何"简化/no-op/暂不"实现时，必须在此登记**（文件:行）。
> 修复某项后，从清单移除并注明 commit。本清单是"踩坑前先查表"的索引，
> 防止同一领域反复探路（如数学状态机、对齐组）。

## 1. 数学原语（存在性测试/消费参数——最高风险区）

| 位置 | 现状 | 影响 | 状态 |
|---|---|---|---|
| `expand/builtins.rs:296` | 数学原语批量"存在性测试" | 参数被吃掉，数学列表节点缺失 | ⚠️ 部分已修（mkern/mskip mu 上下文 3a2cb63；fraction 族 721646c；其余逐项排查） |
| `expand/primitive.rs:753` | 数学原语"简化实现消费参数" | 同上 | 逐项排查中 |
| `eqtb/primitive.rs:559` | 同 753 | 同上 | 同上 |
| `expand/primitive.rs:780-800` | **fraction 原语族（\abovewithdelims/\above/\atopwithdelims/\overwithdelims）此前只扫参数不挂 sink.math_fraction** | 分子分母混收当前层，trip l.276 数学状态崩 | ✅ 已修（721646c；同层嵌套歧义改恢复式，参考 l.257 Ambiguous） |
| `expand/primitive.rs:726` / `eqtb/primitive.rs:382` | `\vcenter` 简化按 vbox | d 组待做（收集不执行） | 待做 |
| `expand/primitive.rs:799` | `\eqno/\leqno` no-op（数学内）；非数学模式报错+pretend 恢复已补 | 显示公式编号不落节点 | ⚠️ 报错已补（未提交）；多报暴露数学状态（l.280/298 数学丢，C 类） |
| `typeset/sink.rs:1082` | 数学模式 `\penalty` 忽略 | 数学断行点缺失（M4-1） | 待做 |
| `typeset/sink.rs:1097` | 数学模式 `\vrule` 忽略 | 规则原子缺失 | 待做 |
| `typeset/math.rs:495` | 分式节点 M4-2 简化（垂直堆叠） | 分式线/字号精化未做（M4-3 fontdimen） | 待做 |
| `typeset/math.rs:540` | 根式节点 M4-2 简化（横线） | cmex10 根号未换（M4-3） | 待做 |
| `typeset/sink.rs:668` | `\radical` 定界符号不参与渲染 | `\radical"161` 等只出 radicand | 待做（l.412 `\everymath{\radical"3}` 的 mathord 报错 1 行与此相关） |
| `typeset/sink.rs:98` | 非数学模式样式错误（原"简化忽略"） | TeX 报错缺失 | ✅ 已修（math_mode_error，未提交；TRIP/ETRIP 未触发） |

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
| `expand/primitive.rs:82` | `\outer` 限制语义 | ⚠️ 部分已修（未提交）：展开上下文禁止（宏体/实参——scan_depth>0）+ \def 体跳过 + cs 名 ^^ 转义；Runaway 块/对齐模板场景待错误恢复链统一 |
| `expand/io.rs:304` | `\write18` shell 转义拒绝（Error） | ✅ 设计如此（RFC-3 副作用隔离；TRIP/ETRIP 不触发） |
| `typeset/sink.rs:1519,1525` / `mod.rs:841` | `\moveleft/\moveright` 位移不落节点（取走即清） | 待做 |
| `typeset/sink.rs:1101` | 非引导上下文未定宽度简化落 0 | 待做 |
| `typeset/sink.rs:1439` | `\showlists` 简化转录（诊断用） | 待做 |
| `expand/primitive.rs:2684` | `\showifs` 简化格式 | ✅ 诊断原语（ETRIP l.651 被错误交互打断，无直接比对场景） |
| `ntex-trip/harness.rs:8` | ETRIP 终端输出经 dvitype 比对暂不纳入 | 待做 |
| `expand/macros.rs:844` | 宏不复制宏体（M1 简化） | 待做 |
| `expand/scan.rs:32` | 十进制扫描 M1 简化版 | 待做 |

---

## 维护记录

- 2026-09-02：建清单（58 处标记扫描归档）；数学内 `$$`、`\mkern/\mskip` mu 上下文已修（3a2cb63）
- 2026-09-02：内部量 no-op 统一、`\right` 缺配对恢复、数学模式组结束 Missing $（9b0bc69）
- 2026-09-02：fraction 原语族挂载 + Ambiguous 恢复式（721646c）；\tracingcommands2 可选 `=` 赋值开启追踪（6cc16f0）；mode_name internal vertical（483b640）；\if 求值 {true}/{false}（3a5ec1e）
- 2026-09-02：\eqno/\leqno 非数学报错 + math_style 报错（942346e）；\outer 展开上下文禁止（未提交）
- **收尾纪律提醒**：后续每轮修复后同步更新本清单（已修项标 ✅ + commit；维护记录追加）
