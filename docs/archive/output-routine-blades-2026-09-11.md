# 输出例程战刀 1–5 实测记录（归档）（2026-09-11 归档）

> ⚠ **历史归档，不再更新。** 同批归档见 [output-routine-survey.md](output-routine-survey.md)（亦已归档）。
> 保留原因：含详细改动面/tex.web 裁决/单测清单，供查证具体某刀。

---

## 5.bis 刀 1 实测记录（2026-09-06，✅ 完成）

**改动面**：`page.rs`（`best_penalty`/`fired_penalty` 断点惩罚记录）+
`TokenSink` 三个新方法（`output_break_penalty`/`take_page_shipped`/
`default_output_routine`）+ `maybe_inject_output` 点火侧语义（§2.2bis 的
引擎侧承担部分：只负责把断点惩罚交予例程 + 死循环保护，六档分派全在宏层）。
原语注册早已在（misc 62/25/43），无需新增原语。

### 5.bis.1 真 TeX 对拍（TinyTeX，tex 3.141592653 / TeX Live 2026，/tmp/ors-work-d1）

对拍探针（plain 格式，与 NTex 同源）：

```tex
\output={\showthe\outputpenalty\shipout\box255}
A\par\vfil\penalty-10000 B\par\end
```

| 触发（§2.2bis 档位） | 真 TeX log | NTex 转录 |
|---|---|---|
| `\par\vfil\penalty-10000`（-\@M / \newpage） | `> -10000.` | `-10000` ✅ |
| `\par\vbox{}\penalty-10001`（-\@Mi / \clearpage） | `> -10001.` | `-10001` ✅ |
| `\par\penalty-10002`（-\@Mii / 行内 float） | `> -10002.` | `-10002` ✅ |
| `\par\penalty-10003`（-\@Miii / 垂直 float） | `> -10003.` | `-10003` ✅ |
| `\par\penalty-10004`（-\@Miv / \end@float 强制页） | `> -10004.` | `-10004` ✅ |
| `\par\penalty-20000`（-\@MM / \supereject） | `> -20000.` | `-20000` ✅ |
| 页满在**胶水**处自然断页（多页） | `> 10000.`（重复） | `10000`（第 2 页起）✅ |
| `\end` 冲页（eject 惩罚 -'10000000000） | `> -1073741824.` | `-1073741824` ✅ |
| `\deadcycles`（每页例程内读，ship 后清零） | `> 1.`（每页） | `1`（每页）✅ |
| `\maxdeadcycles=0` 死循环分支 | `! Output loop---0 consecutive dead cycles.` + help3 三行，照常 ship **2 页** | 同文本 + 2 页 ✅ |

LaTeX 层对拍（`\documentclass{article}` + 勘察报告刀 1 原探针
`\output={\typeout{p=\the\outputpenalty}\shipout\box255}`，
`A\newpage B\clearpage C\newpage D\end{document}`）：真 TeX log 给
`p=-10000`×4、`p=-10001`×2 —— 印证 §2.2bis：`\newpage` 走 -10000 正常臂、
`\clearpage` 的 `\penalty-\@Mi` 走 `\@doclearpage` 臂。

**勘误（简报 → tex.web 实证）**：简报称"例程结束后重置"。tex.web `fire_up`
**无此重置**——`\outputpenalty` 是 `geq_word_define`（全局）且保持到下一次
fire_up；被重置的是**断点节点的 penalty**（`penalty(best_page_break):=inf_penalty`，
NTex 以"新空页丢弃触发节点"等效实现）。NTex 按 tex.web 实现。

### 5.bis.2 已知偏差（待主线核 / 后续刀）

1. **`\write`（`\typeout`）在输出例程内挂死**（既有缺陷，非本刀引入）：`\write`
   的 whatsit 节点在例程内落入主列表 → 页面构建器材料永不清空 → `\end` 冲页
   循环无限（`layout-watchdog` 200 万节点实测复现，真实 TeX 同探针正常）。
   因此 NTex 侧对拍探针改用 `\showthe`（转录专用、不进节点流）。**建议列为
   独立刀**（`\@outputpage` 的页眉页脚路径离不开例程内 `\write`）。
2. **极小 `\vsize` + 页满胶水断页场景下，首页例程读到 `\outputpenalty`=0**
   （应为 10000；第 2 页起正确）。疑似首次例程注入早于 `fired_penalty` 写定
   的时序差，非六档协议路径（页未满时惩罚先触发），已锁在单测注释里。
3. **多页排队时 `\outputpenalty` 取最后一次 fire_up 的值**：NTex 的例程是
   延迟注入（token 边界），一帧内连断多页时队列各页共享最后一次记录；tex.web
   逐页 fire_up 逐页写。sample2e 单页断点场景不受影响，`\@specialoutput`
   五档分派不受影响。
4. **TRIP 基线**：`cargo run -p ntex-trip -- --driver ntex --test trip` 在
   HEAD 9dec0b7（本刀前）即失败于 pass2 `组未闭合（缺少 }）：groups=[SemiSimple,
   MathLeft, MathLeft, Align]`（载入战主线 9dec0b7 既有）；本刀改动下输出与
   基线**逐字节一致**（diff 为空）。

---

## 5.bis.3 刀 2 实测记录（2026-09-07，✅ 完成）

**实际现状（简报假设证伪一处）**：G2 的判定成立——`\unvbox\@cclv`/`\vsplit\@cclv
to\z@`/`\ifvoid\@cclv`/`\ht\@cclv` 在改动前全部恒 void；但「255 不在寄存器文件」
的表述不准：`\box255`（M3-5-3）**本就感知队列**（`box_register(255)` →
`pending_pages.pop_front()`），炸的是其余访问点（`take_or_clone_box`/`copy_box`/
`box_register_kind`/`box_dim`/`set_box_dim`/`vsplit`/`showbox` 只读 `boxes`）。

**裁决（tex.web 实证，reference/tex.web）**：`box(255)` 就是普通寄存器——fire_up
`@<Break the current page at node |p|, put it in box~255...@>` 直接
`box(255):=vpackage(...)`（同层裸写，不入 save stack）；例程结束
`@<Ensure that box 255 is empty after output@>` 检查；unpackage/vsplit 对
void 盒**静默**（`if p=null then return` / `vsplit:=null; return`）。因此
NTex 采用**统一访问面路由**而非「255 进寄存器文件」：`pending_pages` 队列即
寄存器 255 的物理存储、队首即寄存器内容（延迟注入使多页排队，tex.web 里
fire_up 覆写 box(255) ≙ push_back）。三个助手：`box_view`（读）、
`take_box_at`（取走=裸写，无组级日志——若入日志，例程组回滚会把已消费页塞回
队列导致重复输出）、`write_box`（写半边，`store_box` 与 group_end 回滚共用，
`\setbox255` 走 eq_save 语义）。

**真 TeX 对拍**（TinyTeX 2026 plain，/tmp/knife2/p*.tex）：

| 探针 | 真 TeX | NTex |
|---|---|---|
| 页在：`\ifvoid/\ifvbox/\ifhbox255` | `N / Y / N` | 同 ✅ |
| `\ht255`/`\dp255`/`\wd255` | `30.0pt`/`1.94444pt`/`100.0pt` | 结构一致（维度值随度量源）✅ |
| `\setbox2=\vbox{\unvbox255}` 后 shipout | 盒树子节点与直通 `\shipout\box255` 同形 | 同（子节点逐项相等）✅ |
| `\setbox0=\vsplit255 to 10pt`（页在） | `\ht0`=10.0pt（exactly）、余量留 255 | `\ht0+\dp0`=10.0pt、余量非 void ✅ |
| `\setbox255=\vbox{\box255\vfil}` | `\ht255`=31.94444pt=30.0+1.94444 | 页 = vbox{原页整体， vfil} ✅ |
| void 255 的 `\vsplit255 to 10pt` | 静默：box0 void、`\ht0`=0、255 保持 void | 同（改动前为硬错 `InvalidInput`）✅ |
| void 255 的 `\ifvoid255`/`\ht255` | `Y` / `0.0pt` | 同 ✅ |

**发现未修（本刀不动，留独立刀）**：
1. **例程帧末 token 的参数扫描前瞻提前触发 end_group**（ntex-core
   expand/mod.rs，本刀禁碰）：`...\shipout\box<n>` 裸尾时，`<n>` 的数字扫描
   fetch 到帧外 → 帧弹出 + `end_group`（组级回滚）先于 `\box<n>` 落地——
   本例程 `\setbox` 的回滚反噬（NTex 侧观测为 dead cycles→默认输出）。
   例程体补 `\relax` 即规避（单测已注释）；tex.web `end_token_list` 在
   token 真正消耗完后才收帧。
2. **unpackage 的 void→静默返回未实现**：`\unhbox234`（void）NTex 报
   "! Incompatible list can't be unboxed."，tex.web `unpackage` 是
   `if p=null then return`。TRIP l.396/l.425 参考行因类型不符（`\unhcopy3`/
   `\unhbox10`）同样报错，故逐字对齐未破；改语义须连 TRIP 对照一起核。
3. **fire_up 前后的 box255 空检查未接**：tex.web 「\box255 is not void」
   （fire_up 前）与「Output routine didn't use all of \box255」（例程后）
   在延迟注入架构下无对应点（前者与队列语义冲突，后者需例程结束 hook）。
4. **`output_prev_count` 未在队列清空时即时复位**：`maybe_inject_output`
   的「例程未消费→丢弃」判据在「消费完恰好一页 + 输入立即结束」时误判
   （单测以单页文档规避；多页页数不变性待引擎侧修）。
5. TRIP：pass2 既有失败不变（同 `组未闭合` groups）；pass1 delta 39 行全部
   落在 `\setbox 254=\box255`/`\ifvoid 254`/`\box255` 消费路径（即本刀修复的
   语义——take 真正取走页，`\ifvoid254` 由真变假）；ETRIP 逐字节一致。

## 5.bis.4 刀 3 实测记录（2026-09-07，✅ 部分完成——token 体结构化落地，体排版受阻于 expand/）

**改动面**（全部 ntex-layout，零 ntex-core 改动——`insert_node` 的 sink 事件签名未动，
`expand/primitive_align.rs` 的 Insert 臂一行未碰）：`node.rs`（`Node::Ins` 从
`{class, text: String}` 改为 `{class, body: Vec<Token>, split_top_skip, split_max_depth,
float_cost}`）+ `sink.rs`（insert 事件捕获三参数镜像 + `\insert255` 报错改道 0）+
`sink_showbox.rs`（tex.web show_node 格式 + 体 token 串显示）+ `paging.rs`（`accept_page`
前 `insert_accumulate`：页上 ins_node 的体进 `box(class)`、ins_node 从页里删除）+
`tests.rs` 5 条靶向单测。

**tex.web 裁决**（`begin_insert_or_adjust` + insert_group 收口 + fire_up
`@<Either insert the material specified by node |p| into box |n|...@>`）：
①体在**内部垂直模式**排版后 `vpack(natural)` 挂 `ins_ptr`；②`\splittopskip`/
`\splitmaxdepth`/`\floatingpenalty` 在**组体收口时**读取，存 `split_top_ptr`/`depth`/
`float_cost`；③ins_node 的 `height` = 体高+体深（参与页记账），ins_node 本身在
fire_up 被删除（装得下时），体进 `box(class)`——**`box(c)` 就是插入号 c 的累积盒**，
输出例程 `\unvbox\footins` 由此回流。④`\insert255` 报错改道 0。

**真 TeX 对拍**（TinyTeX 2026 plain，/tmp/knife3/p1–p4.tex）：

| 探针 | 真 TeX | NTex |
|---|---|---|
| p1 `\setbox0=\vbox{\insert150{...三参数赋值...\hbox{FN}}}\showbox0` | `\insert150, natural size 6.83331; split(10.0 plus 2.0fil,1.0); float cost 200` + `.\hbox(...)` 体子树 | `\insert150, natural size 0.0; split(10.0 plus 2.0fil,1.0); float cost 200` + `.{\cs… {FN}}`（三参数逐字一致；**natural size 与体子树待体排版**）✅ 部分 |
| p4 `\hbox{X}\insert150{\hbox{A}}\vfill\penalty-10000 ␣\ifvoid150` | `FULL` | `FULL` ✅（fire_up 后 `\ifvoid` 经既有盒寄存器面判非 void） |
| p4' 同上后 `\setbox3=\vbox{\unvbox150}\showbox3` + `\ifvoid150` | box3 含体子树、150 复归 void | box3 含 ins 节点（体 token 保留）、150 复归 void ✅（取走语义） |
| p3 `\penalty-10000\ifvoid150`（**紧邻**） | `VOID`（条件在数字扫描前瞻位求值） | `VOID` ✅ 同款——探针须用空格隔开 |
| 断页后的页盒树 | 无 ins 节点 | 无 ins 节点 ✅（真 TeX 页形） |
| `\insert255{...}` | `! You can't \insert255.` + help + 改道 0 | 同（缺 `<to be read again>`/l.N 上下文行——sink 无输入栈可见性）✅ |

**发现未修（本刀不动，按领地约束记录）**：
1. **体排版被领地卡住（本刀核心缺口）**：`Node::Ins.body` 已无损保留，但体**执行**
   需要 `\insert` 臂把组体交给主循环（tex.web `begin_insert_or_adjust`：`saved(0):=class;
   new_save_level(insert_group); scan_left_brace; normal_paragraph; push_nest; mode:=-vmode`
   + 收口在 `insert_group` 组事件做 `vpack(natural)` + 三参数读取）。落点
   `ntex-core/src/expand/primitive_align.rs` 的 `Primitive::Insert` 臂（现
   `scan_group_contents(None)` 有损收集）+ 新 sink 事件对（如 `insert_begin(class)`/
   复用 group 机制）+ `GroupKind::Insert`（`\currentgrouptype` 须报 11，ETRIP L325
   已在测）。layout 侧无法替代：`scan_group_contents` 收集后 sink 只拿到 token 串，
   而 sink 无 expander 回指、无 cs 名解析面（intern 表在 expander 侧）——故 showbox
   的体只能显示为 `\cs<下标>` 占位。**体一旦执行即同时解决**：natural size、
   `\hbox{FN}` 子树同形、体内 `\splittopskip=` 等赋值生效（本刀三参数取扫描点镜像，
   体内赋值不生效——sample2e `\@footnotetext` 的隐藏需求）、sink 的 cs 名显示。
2. **`\vadjust`/`\special` 仍是 `toks_to_text`**：同一有损压缩模式在 adjust 通路
   未动（本刀领地只覆盖 insert）。体执行刀落地时同法处理。
3. **insert 号与盒寄存器撞号静默**：`\setbox150=\hbox{}` 后 `\insert150` 在
   `insert_accumulate` 直接重建 vbox（tex.web `ensure_vbox` 报
   "! Improper \hbox" 类错误）——撞号报错待补。
4. **TRIP 基线**（共享工作树含 D 线未提交改动，A/B 以 HEAD 基线逐行比对）：
   本刀贡献 7 行 delta——`! You can't \insert255.` 错误块 4 行（tex.web 要求的新增
   正确行为）+ `\insert255 999` → `\insert0, natural size 0.0; split(...); float cost 100`
   + 体行（**格式向参考 `\insert200, natural size ...; split(...); float cost ...` 收敛**，
   仅 natural size/split 值因体未排版而异）；其余 15 行为 D 线斜体 kern
   （`.\kern0.69999`/盒宽 1.39999→2.09999）。ETRIP 当前失败
   （`group_end 无配对 group_begin` @ l.358 math left group）**与本刀无关**——
   临时回退本刀 4 文件后 ETRIP 同样失败（D 线 math.rs 在途改动所致）；knife 2 时代
   ETRIP 逐字节一致。
5. **探针教训**：`\penalty-10000\ifvoid150` 紧邻时，`\ifvoid` 在 `\penalty` 数字扫描的
   前瞻位（expandable `get_x_token`）被求值——真 TeX 同款（p3 VOID / p4 带空格 FULL，
   两边一致），探针写法须用空格或 `\relax` 隔开。
6. `cargo test -p ntex-layout`：181 过 / 2 失败均为 D 线在途数学斜体测试
   （`math_italic_correction_kern_after_ord_char`/`math_italic_kern_sup_only_but_not_sub_only`，
   与本刀无关）；本刀 5 条全绿。`cargo test -p ntex-core`：335 全绿。

## 5.bis.4bis 刀 4 实测记录（2026-09-07，✅ 完成——`\newinsert` 分配器 + 三联寄存器，提交 3ef1f67）

**改动面**：`\newinsert` 原语落地（分配器 + 三联寄存器 `\count/\dimen/\skip` 与
insert 类联动），分配号与真 plain 格式一致——`\footins=\insert254`、`\topins=\insert253`。

**验证**：plain 格式预载侧复测通过（plain-format-survey.md §5.bis.g2：G0 首轮
`\footins`/`\topins` 落 insert255 条目判"已消"；corpus plain `list.tex` PASS）；
TRIP/单元测试无回归。

**残留**：`Node::Ins` 体排版仍挂账（体被收集但排版化未做，脚注仍不可用）——
见 §4 G3 行注记与 KNOWN-SIMPLIFICATIONS.md。

## 5.bis.5 刀 5 实测记录（2026-09-07，✅ 完成——页号链 count0 页标签 + bop 计数接线）

**改动面**：`ntex-core/src/sink.rs`（`TokenSink::count_changed(idx, value)` 新事件，
默认 no-op）+ `ntex-core/src/expand/save.rs`（`assign_count` 末尾 `idx<10` 推送 +
组回滚 `restore` 臂同款，各 3 行——**此文件属并行线领地，是本刀唯一的 expand/
侵入点**，改动为纯追加、可无冲突回退）+ `ntex-layout`（`mod.rs` 镜像字段
`page_counts: [i64;10]` 与页级快照 `shipped_counts`、`typesetter.rs` fmt 播种 +
`shipped_page_counts()` 访问器、`incremental.rs` SideEffects 三处同步 +
`restore_full` 双表同截、`paging.rs` `format_page_label` + `ship_page`/`\shipout`
直通两路入快照、`sink.rs` 事件实现、`tests.rs` 5 条）+ `ntex-dvi`
（`write_dvi_with_counts(pages, counts, fonts)`；`write_dvi` 签名不变，回落
count0=页序号）+ `ntex-test-support/src/driver.rs`（改走 counts 版）。

**tex.web 裁决**（ship_out L12687-12708）：页标签 = `print_char("["); j:=9;
while (count(j)=0)and(j>0) do decr(j); for k:=0 to j do print_int(count(k)) ...`——
count0 起逐段点分、**遇 0 截断**（j 是"最高非零下标"，不是"首个零"）；**全零时
j 停在 0 → 恒打 `[0]`**（探针 3 的答案：打 `[0]`，不省略标签）；负值照打。bop 的
10 计数字是**全量写入不截断**（截断只发生在 log/终端标签）。`]` 的落点分两支：
tracing_output>0 时 `]` 先于盒树（L12702），否则 ship 完再补 `]`（L12706）。

**真 TeX 对拍**（TinyTeX 2026 plain，/tmp/ors-work-k5/k5-p1–p3.tex ↔ NTex 同源
n1–n3.tex，ntex-dvi 驱动 stdout 逐字符）：

| 探针 | 真 TeX | NTex |
|---|---|---|
| p1 `\count0=5 \count1=7` + `\shipout\hbox{aa}` | `...shipped out [5.7]` | `[5.7]` ✅ |
| p2 两页 + `\output={\shipout\box255 \global\advance\count0 by 1}` | `[5.7]` / `[6.7]` | `[5.7]` / `[6.7]` ✅ |
| p3 `\count0=0`（全零特例） | `[0]` | `[0]` ✅ |
| p2 的 DVI bop 10 计数（python 解析） | `[[5,7,0…],[6,7,0…]]` | 快照同值 ✅（见残留 ①） |

**发现未修（按领地约束记录）**：
1. **ntex-dvi/src/main.rs 仍调 `write_dvi`**（G0 线领地）：库侧
   `write_dvi_with_counts` + `Typesetter::shipped_page_counts()` 已就绪，
   驱动侧一行换接即得真 bop 计数；现驱动的 DVI 走回落
   （count0=页序号 1,2,3…，**已顺带修掉每页恒写 1 的旧偏差**）。ntex-mcp/
   ntex-wasm/ntex-backend 的 `write_dvi` 调用点同批换接。
2. **`.fmt` 重载后的播种**（已修）：fmt 恢复的 count 不经 `assign_count`
   （无事件），镜像在 `Typesetter::import_state` 从 `FmtState.registers.counts`
   取初值、`install_builder` 播种——TRIP 首页标签由 `[0.0.0.0.1]` 纠正为
   **`[0.0.0.0.11]` 与参考逐字符一致**（trip.tex L89 递归 `\sh` 后
   `\count4=11` 存 fmt）。曾试 `impl Expander { page_counts() }` 放 sink.rs，
   被 `registers` 字段模块私有权拦下（字段级私有，跨模块 inherent impl 不可见），
   故改走 FmtState 公共面。
3. **M5 段级回滚的寄存器还原不走事件**：`Registers::restore_dirty`（影子表重放）
   在 `register.rs`，无 sink 可达——增量段回滚若回滚了 `\count0..9`，镜像滞后
   （下一段重排时 SideEffects 快照会带回，窗口极窄）。根治须 `restore_dirty` 加
   事件或快照对比，归 M5 线。
4. **TRIP 门禁无回归**：pass2 失败签名逐字符不变（`组未闭合 … groups=[SemiSimple,
   MathLeft, MathLeft, Align]`），log diff 由 3 行标签差异收敛到 1 行精确一致 +
   2 行计数残留（`[0.0.0.0.11]`/`[-2.0.0.0.11]` vs 参考 `[-5000.0.0.0.11.53110374]`
   等——pass2 例程体在既有失败点之后未执行，count0 的 `\countz=\outputpenalty`
   链路未走到，归 TRIP 主线非本刀）。
5. **探针教训**：①真 TeX 的输出例程体在**组内**执行，`\advance\count0` 须
   `\global`（plain `\advancepageno` 正是 `\global\advance\pageno by 1`），非
   global 时真 TeX 两页都是 `[5.7]`；②惩罚断页探针 penalty 前须 `\par` 进垂直
   模式（刀 1 同款教训）；③NTex 驱动（TFM 路径、无 `\font`）字符全 nullfont →
   自动分页产空页被丢弃 → "未产出页面"，探针须带 `\font\tenrm=cmr10`（nullfont
   空页丢弃是既有行为，与本刀无关）。
6. `cargo test -p ntex-layout`：188 过（本刀 5 条新增全绿）；`-p ntex-core`：335 过；
   `-p ntex-dvi`：8 过（bop 2 条新增）。工作区构建除 ntex-studio（glib-sys 系统
   依赖缺失，既有环境问题）全绿。
