# expl3 真实跑分（接线后第一版）

> **本文档替代旧 `expl3-lvt-scoreboard.md` 的「RAN 180/187」口径。**
> 旧口径是**假分数**：187 例的 harness 从不载入 expl3，用例里的
> `\cs_if_exist_use:N` 等函数全部 undefined（实测 77 个/例），
> 却因错误恢复跑到 `END-TEST-LOG` 被判 RAN。

## 接线做了什么（2026-09-11）

1. **`scripts/lvt-run.py`** 新增 `ensure_expl3()`：
   确保 `expl3.ltx` + `expl3-code.tex` 进 testfiles 目录与每例临时目录。
   来源优先级：缓存 `CACHE/expl3-built/` → `EXPL3_SRC` 环境变量 →
   用 latex3 的 `l3kernel.ins` docstrip 生成。
2. **`scripts/lvt/lvt-shim.tex`** 在跑用例前 `\input expl3.ltx`
   （由 `\lvtuseexplthree` 宏门控，值由 lvt-run.py 注入）。
3. 缺失 expl3 时 lvt-run.py **显式警告「假分数」**，不再静默。

### 三个踩坑（都写进代码注释）

| 坑 | 现象 | 正解 |
|---|---|---|
| 载入器 | 直接 `\input expl3-code.tex` → `No expl3 loader detected`（loader 检查 `\ifx\csname ExplLoaderFileDate\endcsname\relax`） | 载 **`expl3.ltx`**（首行 `\let\ExplLoaderFileDate\ExplFileDate`） |
| 探测文件 | `\IfFileExists{expl3.ltx}` 走 false 分支却仍 `\input`；`\openin`+`\ifeof` 对存在的文件也报 eof——**两个原语在 NTex 上均不可靠** | 由 lvt-run.py **显式注入宏**决定，不做文件探测 |
| 宏名 | `\chardef\lvt@useexplthree=1` 只读到 `\lvt`（`@` 在驱动文件执行时不是 cat 11） → `\the` 得到 `0@useexplthree` | 宏名**只用字母**：`\lvtuseexplthree` |

## 单例对照（`m3basics001`）

| 指标 | 假模式（不载 expl3） | 接线后（真载入） |
|---|---|---|
| `! Undefined control sequence` | **77** | **7** |
| expl3 载入踪迹 | 0 | 1 |
| 主要错误 | 全部是 expl3 函数 undefined | `Missing endcsname` 2490 / `Missing = for \ifnum` 1500 / `fontdimen` 1456 |

⇒ **真实差异浮现**：`Missing endcsname`（2490）指向 `\csname` 展开，
`Missing = for \ifnum`（1500）指向 `\exp_stop_f:`（= `~`，cat 10 空格）在
比较符位置被误读。这些都是**引擎语义差异**，不再是「函数没定义」。

## 用法

```bash
cd /home/ubuntu/NTex && export PATH="$HOME/.cargo/bin:$PATH"
cargo build -p ntex-dvi
export EXPL3_SRC=/tmp/e3load          # 含 expl3.ltx + expl3-code.tex
python3 scripts/lvt-run.py --all --jobs 2 --timeout 40
```

## 判据的下一步（重要）

当前 `lvt-run.py` 的判定仍是 **RAN / CRASH / NO-END**（只判「是否跑到
END-TEST-LOG」）。接线后建议再进一档：

- **`PASS`**：转录与 `.tlg` 期望一致（`scripts/lvt-tlg-diff.py` 已有雏形）；
- 在此之前，「跑了多少例」仍不等于「对了多少例」。

⚠ **不要**再把 RAN 数当作 expl3 进度。真实进度要看
**载入 expl3 后的 PASS 数**，而 PASS 判定尚未启用。

## 第七刀：l.36005「90% 墙」破案——不是自旋，是步数上限截断（2026-09-15）

**任务前提证伪**：简报 suspect `\cs_if_exist_use:cF` 动态名构造自旋
（「S1 后即死」）。实测 rt3.tex 在旧二进制下**从未到达 S1**——载入是
**慢而前进**（不是环），`\if_cs_exist:w` 作为 last_tok 高频采样点只是
「最快的采样位」非循环体。hypothesis A/B/C 全不成立：载入期
`\if_cs_exist:w`/`\tl_set_eq:NN`/`\token_to_str:N` 路径无异动。

### 真根因（两处，都不在简报假设里）

1. **步数上限 10M 硬编码 vs 全量载入实测 ~11.0M 步**。
   expl3-code.tex（1,387,070 B / 40,266 行）里的 UnicodeData.txt 数据驱动段
   （l.35647-36005，34,931 行 × 9 字段解析成 clist/intarray）按
   ~25k 步/s 消耗 ~10-11M 步 → 旧上限恰在 ~90% 处击杀
   （`处理步骤超限（疑似死循环）`，静默）。39 次「单步超时 5s」事件
   是**均匀慢**（每 50k 步 0.4-1.4s），非卡死——看门狗措辞误导。
    bytes/step 随载入退化 0.69→0.23（码点数增长 + `\romannumeral`
   展开变长），数据驱动段的成本曲线本来就是超线性。
2. **`\read` 到 EOF 报错**（tex.web read_toks L9510-9517 语义缺失）：
   真语义 = `a_close + read_open:=closed` + **赋空表、不报错**，
   `\ifeof<n>`（L9765 判 `read_open=closed`）随之为真。l3kernel
   `\__ior_map_variable_loop` 的 `\if_eof:w` 收束靠此。

### 落地

| 改动 | 位置 | 说明 |
|---|---|---|
| 步数上限 env 化 | `expand/mod.rs` `max_steps()` | `NTEX_MAX_STEPS` 覆盖，默认 10M→**64M**（wasm 侧唯一应用层死循环防线保留）|
| `\read` EOF 臂 | `expand/io.rs` `exec_read` | EOF → 撤流条目（⇔closed）+ 定义空宏体，不报错 |
| 栈超限报错带宏名 | `expand/mod.rs` | `输入栈超限（N 帧）——宏 \foo 递归展开疑似无终止条件`（对齐 GT 排障信息）|

### 复测（HEAD 6172da3 + 本刀）

- **rt3.tex 四信号全出**：`S1 S2 S3-NOEXIST S4`，DVI 落盘，无致命错 ✅
  （载入走完 UnicodeData → CaseFolding → SpecialCasing → l3backend-dvips.def）
- **载入终点 l.36005 → 末尾**：385 次 watchdog 采样，末次 `steps=10.95M`、
  expl3 源 pos=1,356,735（越过 l.36005 字节位 1,248,905 达 108k）✅
- 6172da3 `drain_depleted_frames` **确证且 GT 对齐**：纯尾递归 `\def\x{\x}\x`
  真TeX **平栈死循环不报错**（pdfTeX 实测挂死），NTex 同为平栈、由步数上限
  收束；条件形 `\def\x{\iftrue\x\fi}\x` → `TeX capacity exceeded, sorry
  [input stack size = 10000]` + 宏名，与 pdfTeX 逐字一致。
  既有测 `unbounded_macro_recursion_hits_input_stack_limit` 钉的是
  **GT 背离期望**（纯尾形期待栈超限）→ 拆成两测按 GT 重钉。

### 残余（第八刀标的）

载入全程 **2486 × `! Use of \??? doesn't match its definition.`** +
`Missing number, treated as zero` / `A number should have been here`，
**全部聚在 l.36007 起 CaseFolding.txt 解析环**
（`\__codepoint_load_data:nn { CaseFolding }` → clist/intarray 写入路径）。
载入能走完是错误恢复在兜底；要把 CaseFolding 数据载对（`\str_case`
折叠系函数才可用），下一刀从 l.36007 的 `\???` 首现场开。

## 第八刀：`\tex_chardef:D` 流号死循环破案——真根因两处都在 eqtb 作用域层（2026-09-15）

**简报假设 vs 实测**：简报指向 l.36007 单点错误恢复重试（`\ior_open:Nn` →
`\__ior_open_stream:Nn` Missing number → `\g__codepoint_data_ior` 无流 → 2486
`\???` + ~10M 步）。链路方向对，但「第一块骨牌」不在 ior/chardef 分配本身，
也不需要给错误恢复加上限——**作用域层两处语义偏差**修掉后级联整体消失：

### 真根因（两处，GT 见 `probes/fp-load/probe-{chardef-scope,fontparam-global}.tex`）

1. **`\global\chardef` 组内绑定被回滚**（`save.rs` / `primitive_toks_state.rs`）。
   tex.web shorthand_def 的临时 `define(p,relax,256)` 与最终 `define(p,a,…)`
   **读同一 `global_defs`**（读取非消费）；旧实现临时绑定走
   `set_slot_scoped` 抢走 `\global` 旗标 → 最终绑定落局部 → `\group_end:`
   回滚成临时 `\relax`。expl3 的 `\g__codepoint_data_ior` 系流号 chardef
   全在此墙下：cd2 探针 pdfTeX `[out:\char"41]` vs NTex `[out:\relax]`。
   修复 = `peek_global()`（只读裁决）+ `set_slot_temp_relax()`（不引燃
   `\afterassignment`）+ `set_slot_scoped_with(…, fire_after)`。
2. **字体参数赋值被局部化**（`primitive_font.rs`）。tex.web：`\fontdimen`/
   `\hyphenchar`/`\skewchar` 赋值**恒全局**、不进 save stack。l3intarray 的
   pdftex 回退分支（expl3-code l.15574-15690）把 intarray 模拟成字体——
   count 存 `\hyphenchar`、条目存 `\fontdimen<n>`——而 codepoint finalize
   （l.35939 起）在 `\group_begin:` 内做 `\cs_gset_eq:cc` + `\cs_undefine:c`，
   出组即回滚 → 每个 `\c__codepoint_*_intarray` 读数报
   `Missing font identifier` + count=45（cmr10 \hyphenchar 默认）→ OOB →
   `\msg_expandable_error` → `\???` 车辆错误 ×2486（每行 CaseFolding 3+1 条）。
   ia2 探针 pdfTeX `[out:100][out-fd:14.0pt]` vs NTex 旧 `[out:45][out-fd:4.30554pt]`。

附带同刀落地：`\meaning`/`\show` 的 char_given 臂改 tex.web print_cmd_chr
语义 `\char"<十六进制>`（ia4 GT：`\chardef\y=123 \meaning\y` → `\char"7B`；
旧印 `the character {`）。

### 复测（probe-load.tex 全量，本刀 HEAD）

| 量 | 第七刀 | 第八刀 |
|---|---|---|
| `Use of \???` | 2486 | **0** |
| `Missing font identifier` | 4599 | **0** |
| 载入期错误总数 | 2492 | **4**（残差见下）|
| rc / `[LOAD-DONE]` / DVI | ✅ | ✅（205B 落盘）|
| 单步超时事件 | 134 | 37 |
| 墙钟 | 11:52 | **12:33**（`time` 实测 752.6s，user 11m45）|

**任务题设「600s→秒级」未达成**：死循环（错误恢复重试）解除后，腾出的步数
预算被**真实的数据装载**吃掉——watchdog 末段采样聚在 pos=1,248,905（l.36007
CaseFolding 装载环），`last_tok` 高频命中 `\__intarray_bounds:NNnTF` /
`\__intarray_gset_overflow_test:nw`：intarray 的 pdftex 字体模拟臂
（每条目 = 一次 `\fontdimen` 写 + `\hyphenchar` 边界读）是**新墙**。
pdfTeX 同一模拟分支原生 `fontdimen` 是 C 数组写；NTex 每次写走全量
eqtb/字体机制。**下一刀标的 = 字体模拟臂的快速通道**（`\fontdimen` 写路径
常量级开销），而非错误恢复。

### 残差（第九刀口径修正后 = 1 条，不阻断）

- `Forbidden control sequence … scanning definition of ^^L`（l.26865 regex 段）
  ——根因#3（active/cs 共用 intern 槽），已记录可保留。
- ~~`Missing number`（`<to be read again> \unhbox`）+ `Incompatible list can't
  be unboxed`~~ ——**第九刀 GT 对拍证伪为探针噪声**（2026-09-16）：pdfTeX 跑
  旧探针同样报这 2 条（plain l.667 `\def\_{\leavevmode…}`：探针首行
  `\catcode`\_=11` 触发 `\_` 展开链落在数字位）。探针首行加 `\let\_\relax`
  后 pdfTeX 0 错、NTex 仅剩 Forbidden ^^L 1 条。

### GT 方法附记（本刀新钉，防复踩）

- **无空格数字后续 `\the` 就地续数**：`\chardef\gX=12\the\gX` → 报
  `You can't use `\relax' after \the.` 且 **\gX=120**（数字循环出环 token 是
  `\the` → 就地扫内部量，临时 `\relax` 绑定被当 0；th4/th5 两引擎一致）。
  裸内部量不续、back_input：`\count0=12\count5\relax` → [bare:12]（th6）。
- **`\if` 不透视 char_given**：`\chardef\gB=66 \if\gB B` → no（th8，两引擎
  一致；if_test 只认 letter/other_char token）。
- **探针 cs 名粘连**：`\if\gXB` 是单个 cs 名 `gXB`（th7 pdfTeX
  `Undefined control sequence`）——写探针须在 cs 名后留空格。

## 第九刀：intarray 字体模拟臂 O(N²) 根治——`\fontdimen` 越界判定全表扫描（2026-09-16）

**任务**：expl3 载入 11min→秒级（lvt 跑分恢复，纯性能刀）。达标线 <60s、
理想 <10s。方案 A = `\fontdimen` 写快速通道（GT 锚 tex.web `set_font_dimen`
L5976 / `find_font_dimen` L11251-11294）；方案 B = intarray 原生化（备用）。

### 真根因（一处，不在题设猜的「合并边界检查」里）

`fontdimen_effective_count`（`expand/primitive_font.rs`）每次 `\fontdimen`
写都**全表扫** `fontdimens: HashMap<(u32,u32),i64>` 取该字体已写最大参数号。
l3intarray 的 pdftex 回退分支把每个 intarray 模拟成一个字体
（expl3-code l.15574-15690：`\__intarray_entry:w = \tex_fontdimen:D`、count 存
`\hyphenchar`），codepoint 数据装载（l.35600-36100）~155k 次条目写 × 每次扫
同步长到 ~155k 项的表 = **O(N²)**。debug no-opt 每表项 ~18ns → ~6 分钟，
恰是 11:17→5:11 砍掉的部分。附带小项（`\__intarray_bounds:NNnTF` /
`\__intarray_gset_overflow_test:nw` 的边界判定成本）被同一缓存顺带覆盖。

### 落地

| 改动 | 位置 | 说明 |
|---|---|---|
| `FontDimens` 结构 | 新 `expand/fontdimens.rs` | map + **每字体 max_num 缓存**；insert/remove O(1)，remove 只在移除当前最大时重算；`clear`/`iter` 面不变；4 条单测 |
| 字段类型替换 | `expand/mod.rs` | `fontdimens: FontDimens`（digest 面走 `iter()` 零改动）|
| 检查点 | `expand/checkpoint.rs` | `ValueExtras` 同型替换；restore 改 `replace_from`（两份内部结构一起换，杜绝缓存失一致）|
| 回滚臂 | `expand/save.rs` | `SavedValue::FontDimen` 走 get/insert/remove 新面 |
| 越界判定 O(1) | `primitive_font.rs` `fontdimen_effective_count` | `grown = self.fontdimens.max_num(font)` |

### 复测（同机 before/after，`probes/perf/probe-load-bench.sh` 实测）

| 配置 | 墙钟 | 备注 |
|---|---|---|
| debug（01a43bd 基线） | 11:17.58 | 本刀前 |
| debug（本刀后） | **5:11.08** | **2.17x** |
| release（opt-level 3） | 1:07.37 | |
| debug + `CARGO_PROFILE_DEV_OPT_LEVEL=1` | 1:18.49 | 领地外杠杆，未入库 |

语义门全绿：rc=0、`[LOAD-DONE]`、`!` 行 = 3（第八刀残差：`Forbidden ^^L`
×1 + `\unhbox` 簇 ×2）、DVI 205B / 1 页 / 77 字体；make check
778+4=782 全绿；第八刀 5 条 chardef 测 + 2 探针（chardef-scope /
fontparam-global）逐字复现。

**达标线 <60s 未达成**（debug 5:11 / release 1:07）。编译旗标已饱和：
dev+opt1（1:18）≈ release（1:07）差 11%，再往上没有旗标可拧——剩下的
是解释器常量因子（见下）。

### 归因链（剩余墙在哪——下一刀的地图）

方法：进程内插桩（`perf_event_paranoid=4` 禁 perf；gprofng 采样计时器被改
"10007→0" 仅 ~10% 有偏样本，不可用）——单步直方图 + last_tok 微秒权重表 +
热点函数纳秒累计，env 门控，**验收前已全部移除**（复刻点：run 循环步计时、
`fetch`/`push_frame`（mod.rs）、`expand_once`（expr.rs）、`collect_args`
（macros.rs）、`scan_number_inner`（scan.rs）、`find_font_dimen`/`fontdimen`
（primitive_font.rs）各包一层 `_inner` 计时）。

1. **单步直方图**（release 全程 10.9M 步）：0-1µs 5.88M / 1-10µs 4.29M /
   10-100µs 668k / **0.1-1ms 105.5k** / 1-10ms 45 / 0.1-1s 2——墙钟大头在
   0.1-1ms 的长步，长步 = 单步内含操作数就地展开的几百个子步。
2. **last_tok 微秒权重表**（前四）：`\if_int_compare:w` 961,543 步 / 26.3s
   （27µs/步）、`\__kernel_tl_set:Nx` 391,172 / 15.7s、`\exp_after:wN`
   1,439,723 / 8.7s、`\tex_expanded:D` 71,894 / 2.6s——全部是「操作数就地
   展开」类，展开的正是 codepoint 装载的 clist 机器。
3. **热点函数**（插桩放大 ~2x 的 release 跑，看次数与 ns/call、别看总时长，
   嵌套计时互相重叠）：`fetch` 581.5M 次 @49ns、`push_frame` 216.3M @195ns、
   `expand_once` 30.2M @6.8µs、`collect_args` 25.8M @1.8µs、
   `scan_number_inner` 10.3M @13.3µs。→ **每 token 子步 ~115ns**
   （67s / 581M 次 fetch），pdfTeX 同路径 C 实现 ~15-25ns，结构差 5-8x。
4. **fontdimen 写路径已干净**：fdstats 曲线（map_len 长到 155k 全程采样）
   终点 find_font_dimen 累计 1.85s（debug）/ 394ms（release），155k 写
   × ~14µs（含数字扫描）——不再是墙。

### 方案 B 证伪（量化，别再回头试）

原生 intarray 分支只省写路径：fdstats 终点显示全部 intarray 写只剩
~0.4-1.9s。**墙在两分支共享的 codepoint 装载 clist 机器**：
`\__codepoint_add:nn`/`\__codepoint_range:nnn`（l.35600-36100，每行
UnicodeData ~5 次调用）每次做 `\clist_put_right:cn` + `\clist_count:c`，
`\clist_count:N`（l3kernel l.8957）= `0 \clist_map_function:NN … \__clist_count:n`
纯宏递归，每 64 项块 ~2-5k token 子步，无引擎钩子可换——原生化 intarray
不触碰它。NTex 侧也无干净解法（宏递归对引擎不可见，模式识别属 hack）；
正解在 l3kernel 源面（改用计数器累加/intarray 直存则墙自消）。

### 下一刀标的（60s→秒级的路，按杠杆排序）

1. **root Cargo.toml `[profile.dev] opt-level = 1`**（在领地外，本刀未动；
   实测单独从 5:11 拉到 1:18）——一行改动的最大杠杆，建议先做。
2. **解释器常量因子**：`push_frame`/帧构造 216M 次、eqtb slot
   clone-per-access、Token 逐个拷贝——115ns→~30ns = 3-4x，需要跨帧结构
   重构（Arc 共享实参帧、eqtb 读借用化），非单刀工作量。
3. **长步摊平**：0.1-1ms 的 105.5k 步 = clist 机器整段展开塞在单步里，
   若第 2 条做完仍不达秒级，考虑 expl3 源面调研（上游协作）。

---

## 第十刀（2026-09-16）：`\prg_return_*:` 条件归约挂死根治 + 一条意外收获

### 结论（三条修复，一次提交）

| # | 根因 | 修复 | 现场 |
|---|------|------|------|
| 1 | `\unless` 只在 `free.rs is_expandable_prim`（执行侧），未进 `eqtb/primitive.rs` 的 `EXPANDABLE:` 清单（扫描侧 ~20 处守卫用）——**守卫双表分裂**，`\unless` 在 edef/write/数字扫描等展开上下文掉成数据 | `Unless` 入 `EXPANDABLE:` + expand_once 补 `expand_unless_in_place`（就地拉取下一 `\if*` 取反求值） | l.6699 首 `\NewDocumentCommand` → `\str_tail:n` → `\expandafter\__str_tail_auxi:w \reverse_if:N \if_charcode:w a a X\else Y\fi` 条件体被当实参吞 → `! Extra \fi` → lthooks 区条件栈失衡 → 全载后段 `\prg_return_true:/\prg_return_false:` 交替归约**真挂死**（steps=10,955,000） |
| 2 | INITEX `lccodes/uccodes` 全零初表；tex.web §191 `@!init` 段本就预载 `lccode[A-Z]=+@'40`、`lccode[a-z]=自身`、uccode 对称 | `default_lccodes()/default_uccodes()` 预载（expand/mod.rs 初始化） | `\lowercase` 对字母失能（lc 探针 `LC1:[PT]` 原样）；latex.ltx 自身**不设** lccode（INITEX 直载假定引擎自带） |
| 3 | `\uppercase/\lowercase` 误列 `EXPANDABLE:` 清单——tex.web 二者**不可展开** | 移出清单（edef 体内回归 tex.web 语义：存数据、用时执行；`ZZ=[macro:->\lowercase{\def\q{AB}}]`→`Q=[macro:->ab]`） | **数字扫描终止位就地展开**（见下） |

### 根因 3 的归因链（本轮最曲折，记录方法论）

`rem@pt` 惯用法（latex.ltx l.10732-10737）：

```tex
\begingroup \catcode`P=12 \catcode`T=12
\lowercase{\def\x{\def\rem@pt##1.##2PT{...}}}
\expandafter\endgroup\x
```

症状：`\rem@pt` 定界符扫描成 `p`(cat-12) + `t`(cat-11)——`T` 的 catcode 赋值"未生效"。

排查弯路（**先证伪的三个假说**）：赋值延迟落表（`\the\catcode`T` 立即读回 12，证伪）；
组回滚吞写（`\endgroup` 前读回也 12，证伪）；两张 catcode 表实例（只有一张，证伪）。
真正的破案点是**给 `\catcode` 与 `\lowercase` 各插一行日志看事件序**：

```
[exec] cs=catcode line=3      ← \catcode`T=12 分派
[tok-born] ch=84 cat=11       ← \lowercase arg 里的 T 已用旧表扫入！
[lc-dbg] …X2 的 arg 扫描完成   ← \lowercase 整个 arg 扫完
[cat-set] byte=84 cat=12      ← \catcode`T=12 的赋值此刻才落地
```

执行序倒挂 ⇒ `\lowercase` 是在 `\catcode`T=12` 的 **`scan_number` 终止段**被展开的：
tex.web `scan_int` 数字循环退出后吸收**一个空格**再 `get_x_token` 找续数字——EOL 变
空格被吸收，下一个 token `\lowercase` **可展开** ⇒ 就地展开 ⇒ arg 在赋值落地前用
旧表扫入。真 TeX 中 `\lowercase` 不可展开，`get_x_token` 原样退回，赋值先行。
V1–V4 四变体（首条赋值生效、次条失效）与 X1/X2 全部由此一击解释。

### 验收

- `make check` 全绿（41 个 test-result ok；`lccode_assign_and_read` 期望值随
  lccode 预载更新：`\lccode`C=`b\the\lccode`C` 就地展开读**预载值** 99 而非旧
  全零表的 0——判别力保住，机制断言不变）。
- 全载 `ltxinit.tex`：**0 挂死**（原 steps=10.9M 真挂死），68s 推进至 **l.14365**
  `{ \input{fonttext.ltx} }`；l.14061 `\rem@pt` 旧阻塞点消失。
- 探针全绿：u3/M3 `macro:->Y`（unless 取反走 `\else` 支）；`\rem@pt` 全链
  `ZZ=[rempt:[12][5]]`（`\strip@pt\dimen0`、`\dimen0=12.5pt`）。
- expl3 fmt 链保持：pass1 62s / **pass2 0.07s**（基准 0.05s 同量级）。

### 新阻塞点（第十一刀入口）

**braced `\input{name}` 文件名扫空**：l.14365 `{ \input{fonttext.ltx} }` 报
`File '.tex' not found`（文件在搜索路径上）；裸探针 `\input{tt}` 复现
`! Missing { inserted.`。归因两选一（下一刀核）：引擎 `\input` 文件名扫描
不支持 braced 形态（tex.web `scan_file_name` 的 quote/end 语义），或 latex.ltx
对 `\input` 的宏重定义（`\let\@@input\input` + braced 参数宏）未生效。
残余错误面：77 undefined cs（既有）、21 `\__hook_make_name:w extra }`
（lthooks 区，新浮出）。

### 方法论沉淀（可复用）

1. **守卫双表是结构性陷阱**：`Primitive::is_expandable()`（扫描侧 ~20 守卫）与
  `is_expandable_prim`（执行侧）两清单必须同刀核对，新增可展开原语两处都改；
  本刀第一修复（Unless）与第三修复（Uppercase/Lowercase 除名）是同一陷阱的正反两面。
2. **"赋值未生效"类偏差先插桩看事件序**，别急着建表实例/回滚假说——执行序倒挂
  一眼定位到"扫描发生在赋值前"，假说空间瞬间收敛。
3. **数字扫描终止位会就地展开可展开 token**（空格吸收后 get_x_token 续扫）——
  `12⏎\expandableTHING{…}` 的 arg 在**上一赋值落地前**扫入；whiteboard 上
  "行间语句"直感在此失效。latex.ltx 大量依赖"不可展开原语紧邻赋值行"的时序。

---

## 第十一刀（2026-09-16）：`\ifincsname` 缺失——braced `\input{name}` 链真根因 + br.tex 探针证伪

### 结论（一修复，一证伪，一环境补件）

| 项 | 内容 |
|---|------|
| 任务命题 | `\@ifnextchar\bgroup` braced `\input{name}` 判定失败（l.14365 56.1% 墙） |
| 真根因 | **e-TeX 原语 `\ifincsname` 全缺失**。2025 版 latex.ltx 的 `\IfFileExists`/`\InputIfFileExists`/`\typeout` 全是 `\DeclareRobustCommand` 产物，其 robust 体首 token 即 `\ifincsname`（l.1409 惯用法：csname 内取 `\string` 形/外取实体形）→ `\InputIfFileExists{fonttext.cfg}`（l.14357）一调用就 `! Undefined control sequence` → 文件探测机器散架 → `\@iinput` missing 分支 → `File '.tex' not found`（空名）→ `\@missingfileerror` → `\read\m@ne`（终端流）NTex fatal |
| 修复 | `Primitive::IfInCsname` + `CondOp::IfInCsname`（`\currentiftype` 码 22）+ `Expander::name_in_progress` 旗标（`scan_csname` 包装层保存/还原——嵌套 csname 内层出口不清外层、`?` 错误路径同还原）+ BUILTINS 414→415 |
| GT（pdftex 1.40.29 四例） | csname 扫描内**经任意深度宏展开**旗标皆真（`\def\q{\ifincsname T\else F\fi}` 两层包裹仍 `mT`）；`\ifcsname` 自己的名字扫描内亦真；扫描外/`\endcsname` 闭合后为假；嵌套 `\csname` 内层结果 cs 落外层扫描 = Missing endcsname（NTex 既有臂同判） |

### 任务书"二选一"双双证伪（第三答案）

- **`\@ifnextchar` peek 语义无罪**：修复后 ltxinit 全量实踪，`\input {omlenc.def}` 族
  经 `\input→\@ifnextchar\bgroup→\@iinput→\InputIfFileExists` 全部载入成功
  （`File: omlenc.def/omsenc.def/ot1enc.def` 消息在场）。
- **`\IfFileExists` openin/ifeof 探测链无罪**：`fonttext.cfg` 探测命中（cfg 横幅打出），
  broken 的是其 robust 入口体，非 openin/ifeof。
- **真答案**：2020+ latex.ltx 把 robust 体拆成 `\ifincsname` 双形——缺原语 = 所有
  `\DeclareRobustCommand` 产品的体在首次使用即爆。braced 判定链三层（futurelet 剥空格/
  `\bgroup` 比较/文件探测）本刀前根本没跑到第二层。

### br.tex 复现伪命题（任务书验收项 2 的字面形式不成立）

`--no-plain` INITEX 下 `{`=catcode 12 是 **tex.web 真语义**（plain.tex 才设 `{`=1；
latex.ltx l.98-102 自管同理）——br.tex 的 `\input{fonttext.ltx}` 文件名被字面扫成
`{fonttext.ltx}`。**GT：真 pdftex -ini 对同一 br.tex 报一模一样的
`! I can't find file '{fonttext.ltx}'`**。上一轮"BRACED-OK 打出"的记录与 br.log
（Emergency stop、无 typeout）矛盾，系误读。验收意图（宏层 braced input 族无
`can't find`）由 ltxinit 全量实踪达成（见上）。

### 环境补件（非代码）

`/tmp/fp11` 缺 latex base 运行时件：已从 `~/.TinyTeX/texmf-dist/tex/latex/base/`
补 36 × `*.def` + 41 × `*.fd` + `language.dat`。**第十一刀后的 ltxinit 复跑必须先
补这批件**，否则停在 `File 'omlenc.def' not found`（该报错本身即证探测链已通）。

### 验收

- `make check` 782 全绿；新增 3 测（`ifincsname_false_outside_csname`/
  `ifincsname_true_during_csname_scan`/`ifincsname_robust_body_idiom`，双轨等价）。
- ltxinit 推进：l.14365 停点（`\input{fonttext.ltx}` 未执行）→ **fonttext.ltx 内部
  omlenc/omsenc/ot1enc 三件载入完成**，新终端墙在 ot1enc.def 末行（l.129=EOF 行）
  输入栈超限。`\InputIfFileExists`/`\@ifnextchar`/braced input 链全通。

### 新阻塞点（第十二刀入口，按执行序）

1. **NFSS 定义群静默丢失**（预存在，第十刀已录 77 条，本刀复核非回归）：preload.ltx
  消费时 `\IfFontSeriesContextTF`/`\normalfont`/`\fontfamily`/`\em`/`\emreset`/
  `\symbol`/`\boldmath`/`\unboldmath` 全 undefined——定义区在 l.5000-10500 却没到
  preload.ltx（l.14350）。另有 21 条 `\__hook_make_name:w extra }`（lthooks 区）。
2. **ot1enc.def EOF 输入栈超限**：现场标注"定义 `\cdp@list` 替换文本时"，但独立最小
  复现（`\def\clist{}\def\celt{\noexpand\celt}\xdef\clist{\clist\celt{#1}}` × 3 连）
  **通过** → 非该构造本体，疑为 1 的 knock-on（`\DeclareTextSymbol` 机器残破后的
  实参扫描无终止）。
3. **`\read` 终端流语义缺失**：tex.web 未开流/流 -1 = 终端读（nonstopmode 下 EOF
  得空行）；NTex 现报 `\read 流未打开` fatal。`\@missingfileerror` 的
  `\read\m@ne to\@gtempa` 走此路。
4. **toks 参数 RHS 语义**（GT 已锚）：`\errhelp\@err@`（RHS 为宏）NTex 走数字扫描
  报 `Missing number`；tex.web/真 pdftex = `Missing { inserted` + 组扫描恢复。

### 方法论沉淀

1. **报错名带字面花括号 = catcode 现场，不是名字构造 bug**：`{fonttext.ltx}` 作文件名
   报出，先查当下 catcode 表（INITEX `{`=12），再查名字机器。
2. **2020+ latex.ltx 的入口机器全在 robust 体里**：`\DeclareRobustCommand` 产品
   （`\typeout`/`\IfFileExists`/`\InputIfFileExists`…）首 token `\ifincsname`——
   任何"文件探测链"问题先核该原语在不在。
3. **任务书的探针要先过 GT**：br.tex 这类探针若真 TeX 也过不了，复现的是真语义
   而非缺陷；"上一轮打出 OK"与 log 矛盾时信 log。

## 第十二刀（2026-09-16）：`\ifcat\noexpand~\noexpand#1` 操作数归一——`\noexpand` 冻结位缺失致 `\DeclareRobustCommand` 全群误路（NFSS 定义群 80 条静默丢失根治）

### 结论（一修复，一归因；靶2 系 knock-on）

| 项 | 内容 |
|---|------|
| 任务命题（靶1） | NFSS 定义群静默丢失：`\IfFontSeriesContextTF`/`\normalfont`/`\fontfamily`/`\em`/`\emreset`/`\symbol`/`\boldmath`/`\unboldmath` 在 preload.ltx（l.14350）消费时全 undefined，定义区（l.5000-10500）却跑过 |
| 真根因 | `get_x_char_operand`（cond.rs）的 undefined 臂**先于 noexpand 检查**执行：`\declare@robustcommand` 分派判别式 `\ifcat\noexpand~\noexpand#1`（l.1397-1398）两侧操作数（active `~`、参数位真名 cs `#1`）都带 `\noexpand` 前缀，双双命中 undefined 臂 → 各报一条 `Undefined control sequence` 且都落 relax/256 哨兵 → `\ifcat` 判 **T** → 全体 `\DeclareRobustCommand` 产品走 auxi（active char）误路，体写成 ifincsname 形而非 `foo␣` 星 csname 形 → 正确名字上从未定义（80 条定义位报错 + NFSS 群静默丢失） |
| 修复 | cond.rs `get_x_char_operand` 顶部加 noexpand 臂（tex.web no_expand 语义：token 打标记后原样返回，**不查 eqtb、不报未定义错**）：active char（定义与否无关）→ 回填字符码 + `Catcode::Active`（tex.web get_x_token_or_active_char 的 `cur_chr:=cur_cs-active_base`）；真名 cs → relax/256 哨兵。同时更正 l.1039 旧注解（"active char 是 Char token 自然命中"与表示层不符——active char 是带 CS_ACTIVE_FLAG 的 ControlSeq，见 input.rs Catcode::Active 生成位） |
| GT（pdftex 四例定案） | undefined-active vs undefined-cs=F；defined-active vs undefined-cs=F；undef-act vs def-act=T；undef-act vs macro=F。`\expandafter\meaning\noexpand~`→`\relax`（不报错） |
| 靶2（ot1enc.def l.129 EOF 输入栈超限） | **knock-on 实证**：修复后该 fatal 消失（`\cdp@list` 栈超限 → 不再出现），无需独立修复。第十一刀的"独立最小复现通过 → 疑 knock-on"判断成立 |

### 证据链（事件流，非对拍猜）

1. **转录对位**：80 条 Undefined 全部是 `\DeclareRobustCommand` 产品定义位，且与 `\~` 报错交错（`~` cat13 判别式两臂都炸）。
2. **前缀二分**：micro（latex.ltx l.1393-1460 鲁棒机器 + l.1805-1806 `\makeatletter`）独立复现；p12600 前缀干净、错误自 l.1805 区起。
3. **GT 四例**（`&gtpro` fmt 即时探针）定案操作数归一表（见上）。
4. **修复**：micro 探针 F/F/F、auxiii 星 csname 形正确；全量 `Undefined 80→0`、`Extra } 0`。
5. **前后对照**：run1（修复前）= 80 Undefined + fatal 栈超限@`\cdp@list`（ot1enc EOF）；run3（修复后）= 0 Undefined + fatal 栈超限@`\@kernel@after@begindocument@before`（新墙，见下）。

### 新阻塞点（第十三刀入口）：`\the⟨toks⟩` 在 edef/xdef 里的冻结语义缺失

- **现场**：修复后首错 `! Missing endcsname inserted. <to be read again> \updefault`（1 条，恢复后继续），最终 fatal 栈超限在吸收 `\@kernel@after@begindocument@before` 替换文本时（残局推进到 l.12745-12777 区）。
- **隔离探针（gam4 vs gt4，双方显式 catcode+`\toksdef\toks@=0`）**：`\edef\kb{\let\expandafter\noexpand\csname __hook env/document/begin\endcsname\noexpand\@empty}` + `\g@addto@macro\kb{\reinstall@nfss@defs\init@series@setup}` 后 `\meaning\kb`：
  - GT（真 pdftex）：`macro:->\let \__hook env/document/begin \@empty \reinstall@nfss@defs \init@series@setup`——`\reinstall@nfss@defs` **原样保留**（`\the` 产物冻结，这正是 `\g@addto@macro` 惯用法 `\xdef#1{\the\toks@}` 的存在理由）。
  - NTex：`macro:->\let\__hook env/document/begin\@emptyUPDEFx\init@series@setup`——**宏被再展开内联**。
- **链式后果**：l.12705 `\g@addto@macro\@kernel@after@begindocument@before{\reinstall@nfss@defs\init@series@setup}` 把 `\reinstall@nfss@defs`（体 = 一摞 `\protected\def\upshape{…\fontshape\updefault\selectfont}`，l.12681-12702）当场执行，`\fontshape`（robust→星 csname→`\csname` 拼名）在 xdef 吸收区跑起来，csname 扫描吞到 `\updefault` 报 Missing endcsname；残局让后续同 cs 吸收（`\expandafter{#1#2}`）无界递归 → 栈超限。
- **代码点位**：`exec_the`（save.rs）把 `\the` 产物 push 成裸 `InputFrame::TokenList`，edef/xdef 扫描器对帧内宏照常展开。修复方向：给 `\the`（至少 toks/宏 token 列表臂）产物以"不得再展开"的帧级标记（tex.web `ins_the_toks`/`end_the_toks` + `backed_up` 输入态是机制出处，落地时按 tex.web 对拍）。
- **顺带观察（未追）**：(a) NTex `\meaning` 的 cs 后不打印分隔空格（GT `\let \foo \bar` vs NTex `\let\foo\bar`）；(b) NTex robust 产物 `\x@protect<cs>` 前缀比 GT 多一截（`\string`/escapechar 相关）。两处暂良性，但 `\meaning` 文本是 expl3 变体判据（第二十八刀 l.9386 IPN 教训），留观。

### 方法论沉淀（探针陷阱复盘，两轮误诊自纠）

1. **裸 INITEX 探针必须设全 catcode 序幕**：`\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\@=11`——只设 `#` 不够。本轮 g3/g4/g5/gam2/m1 家族探针因缺 `{`=1 而全线假红（`\def` 体开括号被当 cat12 字面量 → "参数文本未闭合"/"Missing { inserted"）。第一轮误诊为"`#`=cat12 参数机制坏"、第二轮误诊为"edef 内带参宏调用坏"，**A/B 同探针跑 HEAD（git stash 二分）排除修复回归后**才定位到探针自身。 latex.ltx 上下文探针（micro/前缀）不受此坑——它自带序幕。
2. **`\catcode`\#=6` 反引号-单字符 cs 形式在 NTex 本身是好的**（scan.rs `try_scan_backquote` 单字符 cs 臂 + `single_char_cs` 按 UTF-8 字符数），无须绕道十进制码。
3. **GT `&gtpro` fmt 即时探针法**：`pdftex -interaction=nonstopmode -jobname=X '&gtpro' '\input probe.tex' '\end'` 秒级对拍；fmt 由 latex.ltx 生成故 catcode 与内核宏齐备，但探针若引用 fmt 截止点之后的宏（如 `\g@addto@macro`）须自带定义。
4. **凡"修复后墙反而前移"先别当回归**：修复前"推进更远"可能是带病滑行（80 条定义位报错后 `\@kernel@after@begindocument@before` 处于空/半成品态，同一行恰好不炸）。对照口径应是**错误签名**而非停点行号。

### 验收

- `make check` **787 全绿**（785 基线 + 新增 2 测：`ifcat_noexpand_active_char_stays_cat13`/`ifcat_noexpand_undefined_cs_is_silent_relax`），EXIT=0。
- ltxinit：`Undefined control sequence` **80→0**；ot1enc.def EOF 栈超限消失（knock-on 归因成立）；语义零回退清单各项不动。

## 第十三刀（2026-09-17）：`\the⟨toks⟩` 在展开收集语境的冻结位——`\g@addto@macro` 惯用法根治（l.12777 `\set@fontsize` 区 Missing endcsname 墙）

### 结论（两处点位、一个判据）

| 项 | 内容 |
|---|------|
| 任务命题 | `\xdef#1{\the\toks@}` 的产物在 edef/xdef 被再展开（`exec_the` push 裸 TokenList 帧），`\g@addto@macro` 全家族（NFSS 钩子链 `\@kernel@after@begindocument@before` 等）体被内联执行 → l.12777 `\set@fontsize` 区 Missing endcsname/Paragraph ended 级联 |
| 机制出处 | tex.web L9395-9411（`scan_toks` 的 `@<Expand the next part of the input@>`）：xpand 展开器遇 `\the` **不走** `ins_the_toks`/`ins_list`，而是把 `the_toks` 产物**直接接进正在收集的 token 表**（"Here we insert an entire token list created by \|the_toks\| without expanding it further"）——cs 保持宏 token、组字符不过配平、条件原语不过条件机、`#` 不做参数处理；主循环（非收集语境）才走 ins_list，产物照常展开执行 |
| 修复 | 判据 `suppress_expansion > 0`（增点恰为 tex.web scan_toks(xpand=true) 全集：`\edef/\xdef` 体、`expand_region`（\write/\message/\errmessage）、`\expanded`；checkpoint 已快照、零新状态）。两处点位：`exec_the`（save.rs，expand_region 主循环路径）+ `expand_once` 的 `The` 臂（expr.rs:165，`\edef` 体路径）——帧 items 带 noexpand 冻结标记后，`scan_edef_body` 的 noexpand 裸推臂与 `process_one` 的 noexpand 输出臂恰为 tex.web 接表语义 |
| GT 归一表（pdftex 1.40.29） | ① `\edef\x{\the\T}`：体存 `\reinstallA`，定义期零执行，调用 `\x` 才执行（DEF-DONE→R-EXEC→CALL-DONE）；② `\message{[\the\T]}`：打 `\reinstallA `（冻结）；③ `\write`：`\string` 冻结（`[\string \BB ]`）；④ 主循环 `\the\T`（竖/横模式）：产物**照常执行**（EXEC-A）——冻结只在收集语境；⑤ 组字符参与配平的真边界在 **toks 赋值扫描**而非冻结位（`\T={{x}` 不配平 → Runaway text@\T 吃到 EOF）；平衡 `{{x}}` 原样落体（`macro:->a{x}b`）；⑥ `\expanded{\the\T}`：`\the` 产物在 `\expanded` 内展开一次（其结果 ins_list 回流，外层 xpand 重取再展开）→ 体 `\message{R-EXEC}`，无双重展开 |
| NTex 修复前后对拍 | `\g@addto@macro\kb{\reinstall@nfss@defs}`：修复前 `\meaning\kb`=`macro:->BASE\message{R-IN}`（内联）；修复后 `macro:->BASE\reinstall@nfss@defs`（GT 同，唯 cs 后分隔空格为既有良性偏差——第二十九刀观察项） |

### 证据链

1. **\meaning 对拍**：mean.tex（自带 catcode 序幕 + `\toksdef\toks@=0`）双引擎：GT 体保 cs、NTex 体 `\message{R-IN}`。宏体内容失真是真判据——可见输出顺序（DEF-DONE 序）在最小探针里碰巧不分化（`\message` 不可展开被收进体），任务书"NTex 当前 R-EXEC 在 edef 时即打出"仅在体含**可展开侧效应**时成立（`\reinstall@nfss@defs` 体是 `\protected\def` 群，定义期执行即炸 csname 扫描）。
2. **两处点位定位**：`\edef` 体走 `scan_edef_body`→`expand_once`（expr.rs The 臂）；`\write/\message` 走 `expand_region`→`process_expand_only`→`exec_primitive(The)`→`exec_the`（save.rs）。任缺一处即漏半边。
3. **判据选型**：`expand_only` 只盖 expand_region（`exec_def` 路径不设）；`suppress_expansion` 恰为 tex.web 展开收集语境全集且 `\unexpanded` 已同款使用（expr.rs L190 无条件 true 的帧位、L1009 的 `flag = self.expand_only`）。
4. **边角对拍**（修复后全数与 GT 一致）：`\expanded` 双层语义（g6/g6n）；主循环执行语义不砸（`the_toks_still_executes_in_main_loop`）；平衡组字符落体（`the_toks_frozen_group_chars_preserved`）。

### 遗留观察（未追）

1. **`\message`/`\write` 语境里 toks 内容含字面 `\noexpand`**：冻结帧把 `\noexpand` cs 本身当数据收集（串里印 `\noexpand \foo`），GT 在 write_out 展开期让 `\noexpand` 完成标记（印 `\foo`）。latex.ltx `\g@addto@macro` 主通路是 edef 体（裸推臂原样保留 `\noexpand` ✓ tex.web 接表语义同），不受影响；根治需冻结位随 token 存进收集产物（表示层改动，非本轮）。
2. **`Primitive::Toks` 臂（`\the\toks0` 文本转换）保持原样**：cs→cat-12 文本转换是 TRIP 钉子（`\showthe` 打印面），与 Register 臂（裸 token，latex.ltx 真路径）并存；冻结位对纯字符产物惰性。
3. **`\meaning` cs 后分隔空格缺失**（第二十九刀顺带观察）仍在：本轮对拍用它作判据时取不含空格子串。

### 验收

- `make check` **792 全绿**（787 基线 + 新增 5 测：`the_toks_frozen_in_edef_deferred_execution`/`the_toks_gaddto_macro_body_stays_frozen`/`the_toks_frozen_in_message_context`/`the_toks_still_executes_in_main_loop`/`the_toks_frozen_group_chars_preserved`）。
- 最小复现：DEF-DONE → R-EXEC → CALL-DONE ✓；`\g@addto@macro` 体保 cs ✓。
- ltxinit 推进结果：见下节补记。

## 第十三刀补记：ltxinit 推进结果

`\mathchar@type` Missing number 墙本体在第十三刀收口后（l.12777 `\set@fontsize`
区 Missing endcsname 级联消退）推进至 **fontmath.ltx l.509**（`\mathdollar`
符号声明）→ Missing number 294 条 + `! Bad mathchar code` 致命。第十四刀入
场时的墙即此。

## 第十四刀（2026-09-17）：基数常量循环 get_x_token 语义 + 跳过区优先序——`\mathchar@type` Missing number 墙破（fontmath.ltx 全族通过、ltxinit 推进至 preload.ltx l.47）

### 结论（两处点位、一个域修正、两个读臂）

| 项 | 内容 |
|---|------|
| 任务命题 | `\DeclareMathSymbol{\mathdollar}{\mathord}{operators}{"24}` → `\mathchardef\mathdollar"\mathchar@type\mathord\hexnumber@{\count\z@}\hexnumber@{\count\tw@}\relax`（latex.ltx l.13720）在 `" 后首 token 是宏（`\mathchar@type`）→ `! Missing number, treated as zero.`，fontmath.ltx l.509 起 294 条级联 |
| 根因 A（主墙） | `scan_number_inner`/`scan_dimen` 的十六进制 `"`/八进制 `'` 常量循环是**纯数位循环**——tex.web `@<Scan a hexadecimal or octal constant@>` 的循环尾 `get_x_token` 语义（数位间宏/可展开原语就地展开、产物继续累计、首个不可展开产物放回）缺臂。十进制循环第四刀已补（`l.660` 区），基数循环漏同款 |
| 根因 B（修复 A 后第二层） | 基数循环把**数位判定排在跳过区判定之前**——`\ifcase2 0\or 1\or 2\or 3\fi`（`\hexnumber@` 展开体）死分支的数位 0/1/3 进累计 → 0x123 而非选中分支 2。十进制循环的臂序是 `is_skipping → 数位 → cond_op → 可展开 → 放回`（第二十四轮钉子），基数循环必须同序 |
| 修复 | scan.rs 两处基数循环（l.276 scan_int / l.1892 scan_dimen）补齐四臂：跳过区优先、fi_or_else 归属（游终结符放回/本扫描帧就地步进）、宏+可展开原语 `expand_once` 就地展开、不可展开放回 |
| `\mathcode` 域修正 | `exec_mathcode` 原 `& 0x0000_7FFF` 掩码把 `"8000`（active 旗标，fontmath l.159 `\mathcode`\ ="8000` 惯用法）静默抹零、超界值致致命错。改 tex.web assign_math_code 域 **0..=0x8000**，越界报 `! Invalid code (N), should be in the range 0..32768.` 恢复并跳过赋值（pdfTeX 对拍 g14mc：`"8000` 读回 32768、8001 报 Invalid code 后作业继续） |
| 读臂补全 | `\mathcode<char>`（缺省 `"8000`；INITEX 初表由 `default_mathcodes()` 全量预载，字母=0x7100+码）与 `\sfcode<char>`（初值 1000）读臂补进 scan_number 内部量分派——此前 `\cnt=\mathcode`a` 落 Missing number，pdfTeX=29025 |

### GT 新知（pdftex 1.40.29 实测；数字扫描「原子+可选空格」模型）

探针先过 GT 的教训本轮再次兑现——**k14w2 模拟探针用 `\count0`（数字索引）替换
真 LaTeX 的 `\count\z@`（cs 索引），GT 与真构造行为相反**。判定实验链（全部
双引擎对拍）：

| 探针 | pdfTeX | 判据 |
|---|---|---|
| `\count6=5 6\relax` | 5 | 空格 token 终结数位串，后随数位不吸收 |
| `\count6=\cntA 6\relax`（countdef cs） | 2 | cs 型内部量后空格不吸数位 |
| `\count6=\count9 6\relax`（数字索引寄存器） | 2 | 索引数位扫描的 done 路径吞掉可选空格 |
| `\count5=\number\cntA 0\relax`（无空格 token：行尾 `@` 后被 tokenize 吃掉） | 20 | `\number` 产物 `2` 与 `0` 相邻 → 吸收 |
| `\ifcase\number\cntA 0\or…`（宏体内 `#1` 后空格**存留**） | 操作数 2 → 分支 2 | 参数 token 后的空格是真实 spacer，终结操作数 |
| `\ifcase\number\count9 0\or…`（宏体、数字索引） | 操作数 20 → 空分支 | 索引扫描吞空格 → 分支首 `0` 粘上操作数 |
| `\hexnumber@{\count\tw@@}`（cs 索引，`\count2`=4） | 4 | **真 LaTeX 的通路**：操作数 4 → 分支 `4` |

**`\hexnumber@#1{\ifcase\number#1 0\or…\or F\fi}` 能工作的机制全在「`#1` 替换
后空格 token 存留 → 操作数在空格处终结 → 分支选择产出数位」**。基准锚：
真 latex.fmt 下 `\showthe\mathdollar` → **36**（"0024）；NTex 修复后同探针
（k14w9，INITEX 手搭 `\chardef\z@`/`\chardef\tw@` 环境）同样 36 ✓。

修复前 NTex 对拍（同探针链）：`\cnt="\hexnumber@{\cntA}\relax` → 291（=0x123，
死分支全吸收）；修复后 2 ✓。`\cnt="\ifcase2 2\or 5\fi\relax` → 37（修复前）→
Missing number 0（修复后，GT 同）。

### 证据链

1. **主墙点位**：`\mathchardef\mathdollar"` 后首 token `\mathchar@type` 是宏 →
   纯数位循环放回报 Missing number。展开臂补上后 k14y 探针暴露第二层：
   `! Bad mathchar code (1311768430813402744)` = 0x1234567012345678——
   两个 `\hexnumber@` 展开体的死分支数位全部进累计，跳过区优先序缺失。
2. **判据定型**：k14z 五连探针（`\number\count9` / `\ifcase0` / `\ifcase2 2\or 5` /
   `1\ifnum1=1 A\fi` / `\number\count9 0`）修复后全数与 pdfTeX 一致；
   g14w4（`A:[2] B:[]` + Missing number 序）逐字符一致。
3. **域修正对拍**：g14mc（mathcode/catcode/delcode/sfcode 读写 + `"8000` 往返）
   NTex 与 pdfTeX 输出差仅剩 delcode 读臂（见遗留观察）。
4. **既证伪假说（防重查）**：① `\ifx` 对两个 csname 未定义 cs 是否相等——
   双引擎同判 DIFF，探针非判别性，勿再用作 csname 名构造判据；② `\csname
   mathsf ␣\endcsname`（`\space` 已定义展开为空格）名构造——双引擎 `\show`
   均落到同一 cs（`->HELLO`），NFSS 尾空格 csname 惯用法 NTex 本就正确；
   INITEX 里 `\space` 未定义时的报错差异（pdfTeX=Undefined control sequence、
   NTex=Missing endcsname）是探针伪命题，非偏差。

### ltxinit 推进结果

| 指标 | 第十四刀入场 | 收口 |
|---|---|---|
| 错误总数 | 294（`Missing number` 254 + 40 余项）| **22** |
| 终止方式 | `! Bad mathchar code` + `\mathcode` 致命（dumped=false）| preload.ltx l.47 `\DeclarePreloadSizes{OT1}{cmr}{m}{n}{5,7,10}` → `! \font 后缺少字体名`（独立墙） |
| fontmath.ltx | l.509 停 | **全文件通过**：`\symoperators…\symlargesymbols` 四 sym cs + bold 覆写（l.63-65）+ `\DeclareMathSymbol` 全族 + `\SetMathAlphabet` 前 2 条 |
| latex.ltx | 64.4% 停 | 通过 l.22835 `\dump` 前的全部定义区与装载序 |

剩余 22 条：20× `Argument of \__hook_make_name:w has an extra }`（lthooks 区，
预存在）+ 2× `LaTeX Error: Command `' not defined as a math alphabet`
（fontmath l.73/74 `\SetMathAlphabet\mathsf/\mathit{bold}…`，本轮归因现状见下）。

### 新阻塞点（第十五刀入口，按执行序）

1. **`\SetMathAlphabet\mathsf{bold}{OT1}{cmss}{bx}{n}`（fontmath l.73）**
   `Command `' not defined as a math alphabet` ×2。判据链（latex.ltx l.13553
   `\SetMathAlphabet@`）：`\in@#4{\alpha@list}`（`#4`=`\csname mathsf ␣\endcsname`）
   → 假则 `\in@{\string\use@mathgroup}{\meaning#4}` → 假则报错。本轮已证伪：
   `\in@` 本体（最小探针 YES/NO 序一致）、尾空格 csname 名构造（`\show` 同 cs）。
   待查：`\alpha@list` 是否被 l.70 `\DeclareMathAlphabet{\mathsf}{OT1}{cmss}{m}{n}`
   真实登记（`\new@mathalphabet` 的 `\xdef\alpha@list{\alpha@list\alpha@elt #4…}`
   链）、`\version@list` 的 `\mv@bold` 登记、`\meaning#4` 文本含 `\use@mathgroup`
   与否。报错里命令名空串（``Command `'``）本身是线索：`\string#5` 产物为空或
   `\@latex@error` 的 edef 链在 NTex 走样。
2. **`\__hook_make_name:w` extra `}` ×20**（lthooks 区，预存在，l.8912 之后的
   独立族；本轮未动）。
3. **preload.ltx l.47 `\DeclarePreloadSizes{OT1}{cmr}{m}{n}{5,7,10}`** →
   `! \font 后缺少字体名`（NTex `\font` 原语的字体名扫描在 `\small@sizes` 系
   展开体上的偏差；当前致命终止点）。
4. **预存在**：latex.ltx l.8912 区 `Missing number … <to be read again> \let`
   （`\GenericError`/`\errhelp` 区，1 条，本轮未动）。
5. **`\delcode` 读臂缺**（pdfTeX 未赋值读回 -1、可赋值读回；NTex 落 Missing
   number）。补臂前须先审负值存储（`delcodes: HashMap<u32,u32>` 存不下 -1，
   `save.rs:687` 现用 `0x500000` 作恢复缺省——与 tex.web -1 是否同义未审），
   非纯加臂。

### 验收

- `make check` **793 全绿**（793 基线零回退，含 TRIP 门禁）。
- ltxinit：fontmath.ltx l.509 越过、`\DeclareMathSymbol` 全族通过 ✓；错误
  294 → 22；推进至 preload.ltx l.47 新墙（`\font` 字体名扫描）。
- 探针矩阵双引擎对拍：k14z/k14w9/g14w4/g14mc/g14sd/g14p/g14x/g14in/g14cs2
  全数一致（除标注遗留项）。
- 领地：仅 `crates/ntex-core/src/expand/{scan.rs,primitive_font.rs}`；无临时
  插桩入库（诊断走既有 `write16`/`NTEX_COND_TRACE` 通道）。

## 第十六刀（上）（54f68b7，2026-09-17）

### 修复
1. **expand_region 护栏**：子展开 process_one 递归不经主循环步数检查——l.16900
   \@preamble \edef 无限循环 900s 无护栏。region_steps 计数（max_steps 同源）
   + 栈深上限（入口+4096）。
2. **more_name 条件深度护栏**：l3kernel quark 惯用法（\__file_quark_if_nil:nTF
   不闭合 \if_meaning:w 对）在名字流内每次展开再入条件机——cond_stack 无限加深
   直至 OOM/SIGKILL（1GB 内存实测 470s 被 kill）。深度 >64 终止名字。
3. **\ifx 测试对齐 GT**：pdfTeX -ini 实证名字扫描遇 \ifx 就地求值、真支收进
   名字（\a=nullfont）——测试改 \relax 终止名字。主控的"裸流守卫"（IfX
   终止名字）被 GT 证伪后回滚——探针必须先过 GT 的又一次验证。

### 进展
ltxinit 主帧 64.4% → **88.4%**（l.20700 区）。

### 新墙（第十七刀靶，已归因）
`\prg_map_break:Nn` 的 **break 炸弹在 bytecode 执行器内单步不返回**
（watchdog「疑似挂死 10441ms 无心跳」，\prg_map_break + 双 Bytecode 帧，
last_tok=\char_set_catcode:nn）。break 跨帧跳转（tex.web炸栈到 \prg_break_point:
标记再回卷）未在字节码执行器实现——与第十刀 \prg_return_* 解释器挂死同族，
但战场在字节码层。

### 方法论
- 字节码帧（Bytecode(pc=N)）内的死循环不经过主循环 watchdog 步数计数——
  「watchdog 停更 + 进程存活」= 卡在单步内部或子展开，两种护栏都要设。

### 第十七刀补充取证（40218b0）

- tests_break17 4 测全绿（793→797）：跨帧 cs 定界、3 层 break 炸弹 unwind、
  不等名重发——**测试环境语义全部正确**（含 GT 内容流对拍修正两处错误期望：
  未定界收集吞前导空格 → [XY]Z；#5 无条件输出 → XSKIPENDAFTER）。
- 88.4% 挂死复现稳定（ltx23：422s、steps=11,180,000 停更、同栈指纹）。
  栈 = Source(主帧 685828) + Bytecode(pc=122)+Bytecode(pc=98)+Bytecode(pc=0)，
  **无实参帧** → 挂死在 bytecode 执行器的宏调用链内部（疑似 pc 恢复/返回地址
  处理：子帧结束后上层 pc 未前进），非实参扫描、非跨帧定界。
- 挂点在 latex.ltx l.20751 一带（\@ifnextchar [\@topnewpage\@floatplacement，
  输出例程区）；last_tok 交替 \prg_map_break:Nn / \char_set_catcode:nn。
- 下一步：bytecode 执行器「宏调用返回后上层 pc 前进」逻辑审查（Call 帧的
  ret 语义），或用 NTEX_BREAK17 事件环抓 11.18M 步前最后 40 事件。


## 第十八刀：尾递归鞍具校准（2026-09-17）

**本轮未修改引擎；不能把 3 项回归测试转绿记作 88.4% 主墙已修。**

### 根因机理更正

简报中的 O(n) 爆栈样例为 `\ifnum\count0<20000\expandafter\iter\fi`，
上界数字与 `\expandafter` 之间没有空格。`scan_int` 的 `get_x_token` 在数字
尚未结束时先展开 `\expandafter`，此时条件帧还是 Evaluating；`\fi` 走
`insert_relax`，回压尚未消费的 `\relax/\fi`，递归宏却排在它们前面继续执行。
所以 `TokenList(1tok,pos=0)` 是**活 token**，不是已耗尽的调用者帧。

`exec_expandafter` 已通过 `push_frame → drain_depleted_frames` 在回压前
回收耗尽纯帧。重复调用 drain 不改变结果，删除 pos=0 帧则会破坏 TeX 语义。
字节码当前只有 Emit/EmitArg/End，没有独立 Call/Ret 指令；历史「Call/ret
恢复错误」是待证假说，不能由本组尾递归样例推出。

### pdfTeX 对照（TeX Live 2026，1.40.29，INITEX）

```tex
\catcode123=1 \catcode125=2
\count0=0
\def\iter{\advance\count0 by 1 \ifnum\count0<20000\expandafter\iter\fi}
\iter\immediate\write16{DONE=\the\count0}\end
```

`pdftex -ini -interaction=nonstopmode` 实测：以上紧邻形式报
`TeX capacity exceeded, sorry [input stack size=10000]`（退出 1）；
仅在 `20000` 后加一个空格即输出 `DONE=20000`（退出 0）。
对照文件与原始日志：`/tmp/d18-oracle/{tight,space}.{tex,stdout,log}`。
INITEX 的花括号 catcode 必须显式设置，避免把探针定义扫描错误当引擎差异。

### 修改点与长程实测

- 保留 `tests_bytecode18` 三项正式测试，两个上界后补空格；调用点由
  `concat!("\\iter", "DONE")` 改为 `concat!("\\iter ", "DONE")`，
  避免控制词被拼成未定义的 `\iterDONE`。原测试未真正执行循环，不能作为
  正式爆栈证据；临时 probe 的 `\iter|DONE` 才有合法调用边界。
- 双轨均验证完整输出；每个主循环单步边界检查栈深 ≤64、总步数 ≤500 万，
  最后检查条件栈闭合。单轮锚点也增加解释器对拍。
- 删除临时 `probe18.rs` 及其模块注册；没有提高 MAX_INPUT_STACK 或添加绕过。

| 用例 | 迭代数 | 字节码 / 解释器步数 | 主循环边界栈深峰值（双轨相同） |
|---|---:|---:|---:|
| 三层 break 炸弹循环 | 20000（每轮 3 次） | 820010 / 820010 | 6 |
| 跨帧 cs 定界实参循环 | 20000 | 480010 / 480010 | 5 |
| 单轮 break 炸弹锚点 | 1 | 25 / 25 | 3 |

`cargo test -p ntex-core tests_bytecode18 -- --nocapture`：3/3 通过。
栈深为主循环边界观测值，不冒充单步内部每次 push 的峰值。


### 完整门禁与真实复现

- `CARGO_BUILD_JOBS=1 make check`：**800 passed / 0 failed / 5 ignored**，
  基线 797 + 本组三项，既有测试未删除（日志 `/tmp/d18-check.log`）。
- 门禁结束后单独 `cargo build --release -p ntex-dvi`，成功后才启动复现；
  build/test/ltxinit 三者未并行。测试均使用 dev profile。
- 使用简报原命令（`NTEX_TFM_DIR=$HOME/.ntex-fonts`、两个 input-path、
  `--no-plain --dump /tmp/fp11/latex6.fmt /tmp/fp11/ltxinit.tex`），
  外包 `timeout 500`；原始日志 `/tmp/d18-ltxinit.log`。
- **退出 124（超时）**，`time` 实测 real=503.65s / user=136.88s / sys=6.32s。
  最后主帧仍为 **pos=685828 / 776142（88.4%，未推进）**，
  steps=**11180000**。没有 `[LATEX-LTX-DONE]`，不能认定 `.fmt` 成功。
- 最后 watchdog 指纹：last_tok=`\char_set_catcode:nn`，缓存状态中的
  last_tok=`\prg_map_break:Nn`；主帧之后仍是
  `Bytecode(pc=122) | Bytecode(pc=98) | Bytecode(pc=0)`。
  本次没有越过旧墙，故**没有可登记的新墙**。
- 运行中抽样 RSS=365472 KiB、loadavg≈1；超时结束后观测到
  loadavg=46.30/31.32/14.09，超过红线后未再启动重型作业。

**验收结论：鞍具修正和 800 项门禁完成；引擎修复与越过 88.4% 尚未完成。**
当前证据只排除了「本组条件尾递归证明耗尽帧泄漏」的归因，未确定真实主墙根因。
后续应从真实挂点继续取证，不得删除未消费 token 或恢复已证伪的 Call/Ret 断言。

### （四）l.20302 NFSS 错误恢复残流重放（2026-09-17）

**现场定案**：`NTEX_HANDLER_TRACE=1` 在 bytecode 护栏触发时转储最近 32 次
dispatcher 入口及 `read_floor`。尾项为 **`\edef(floor=0)`**，主帧
`pos=703057 / 776142`；故自旋在 `\edef` 的展开扫描内，非 `expand_region`
边界越界（也非 `collect_delimited_arg` 的定界匹配活锁）。

**根因与修复**：non-long 宏实参遇额外 `}` 时，旧恢复先回推该 `}`，再复用
Paragraph-ended 路径；后者会把同一枚 `}` 重读并作为所谓恢复材料再次回推。
而 `collect_args` 又继续扫描下一个参数，于是 NFSS
`\extract@rangefontinfo#1<#2>` / `\check@single#1>#2<#3` 的残流在 `\edef`
handler 内循环重放。现改为：在被拒 `}` 上方压入真实 `\par`、标记本次实参扫描
已恢复，并立即结束整次 macro_call（不扫描后续参数、不展开残缺宏体）；恢复材料交回
外层输入。

**回归与验收**：`tests_bytecode18` 新增最小 `#1<#2>` + `\edef` + 残流场景，
字节码/解释器双轨均 16 步到达 `DONE`。`CARGO_BUILD_JOBS=1 make check` 为
**801 passed / 0 failed / 5 ignored**。同一 200 秒、`NTEX_BC_GUARD=1000000`
真实复现不再出现护栏，已越过 `pos=703057`；未得到 `[LATEX-LTX-DONE]`，最终仍因
未执行 `\dump` 拒绝保存格式。

**新墙指纹（下一刀）**：越墙后的首个稳定错误族为 lthooks 的
`! Argument of \__hook_make_name:w has an extra }.`（伴随 nullfont 缺字符）；
本轮只登记，不把它与 NFSS 恢复修复混刀。

## 第十九刀：hook 名构造的 csname 内部空格 catcode（2026-09-17）

### 探针更正

上一轮手写 hook 探针缺少 INITEX 花括号 catcode 前置，导致 `\def` 参数文本未闭合等
自伤错误；本轮所有可复跑探针首行显式设置 `{`=1、`}`=2。

同时复核了主控提出的 INITEX 初表疑点：在本机 TeX Live 2026
`pdftex -ini` 下，`\showthe\catcode`\{` 与 `\showthe\catcode`\}` 均为 **12**，
裸 `\def\x{A}` 同样报 runaway definition / Missing `{`。因此 NTex INITEX 初表
不漏 `{}`，本轮未改 INITEX catcode 预载表。

探针入库：

- `probes/hook-make-name-core.tex`：锁 `\string\csname a b\endcsname` 的名字内部空格。
- `probes/hook-make-name-min.tex`：锁 lthooks `\__hook_make_name:n` 构造链。

### 根因

`lthooks.dtx` 的核心构造：

```tex
\cs_new:Npn \__hook_make_name:n #1
  {
    \exp_after:wN \exp_after:wN \exp_after:wN \__hook_make_name:w
    \exp_after:wN \token_to_str:N \cs:w __hook~ #1 \cs_end:
  }
\exp_last_unbraced:NNNNo
\cs_new:Npn \__hook_make_name:w #1 \tl_to_str:n { __hook~ } { }
```

`__hook~` 的 `~` 是 space token（cat 10）。真实 TeX 在 `\string` 一个由
`\csname __hook <name>\endcsname` 生成的控制序列时，控制序列名内部的字符码 32
也以 space token 输出。旧 NTex 对控制序列名逐字节一律吐 cat 12，导致
`\__hook_make_name:w` 的定界符最后一枚 token（cat 10 空格）永远匹配不上，
扫描到右花括号时报 `Argument of \__hook_make_name:w has an extra }`。

诊断证据（`NTEX_DELIM_DBG=1`）：定界符尾 token 为
`char ' ' / cat Space`，输入尾部对应 token 为 `char ' ' / cat Other`。

### 落地

- `free.rs`：`\string` 与 `\detokenize` 打印控制序列名时，名字内部字节 `0x20`
  改为 `Catcode::Space`，其余字节仍为 `Other`。
- 双轨回归：
  - `string_of_csname_internal_space_keeps_space_catcode`
  - `latex_hook_make_name_strips_internal_prefix`

### 复测

复现命令使用 `--no-plain`，输入路径为 `/tmp/fp11`、TinyTeX `latex/l3kernel` 与
`latex/base`，输出 `/tmp/fp11/latex6-fixed.fmt`。

| 指标 | 修复前 | 修复后 |
|---|---:|---:|
| 总错误数 | 55 | **5** |
| `\__hook_make_name:w extra }` | 54 | **0** |
| `SetMathAlphabet` | 2 | 2 |
| `\reserved@a extra }` | 2 | 2 |
| Undefined control sequence | 1 | 1 |
| `.fmt` | 未落盘 | 未落盘 |
| `[LATEX-LTX-DONE]` | 未到达 | 未到达 |

修复后首错变为 fontmath 旧账：
`LaTeX Error: Command \`' not defined as a math alphabet.`（fontmath l.73/74
对应日志位置 l.529）。尾部新墙为 2 条 `\reserved@a extra }`；未执行 `\dump`，
因此仍拒绝保存格式。

**验收结论**：本刀清掉 lthooks 主墙，错误面 55→5；未达 `[LATEX-LTX-DONE]`，
按“一刀一墙”记录新墙（fontmath / `\reserved@a` 残余）后收工。

## 第二十刀（收尾，2026-09-18）：l.8912 错误恢复污染根治——`[LATEX-LTX-DONE]` 到达、latex.fmt 落盘

### 战役背景

第十九刀收工时错误面 55→5，主帧停在 pos=556,097/776,142（71.7%），报
「源未执行 \dump」。首错锁在 latex.ltx l.8912 `\GenericError` 小写化块：
`! Missing number, treated as zero.` + `<to be read again> \let`，随后
766 条 `Missing character: There is no X in font nullfont!`（拼出的是
lthooks hook 注册名）——即错误恢复残流被排版，而非真语法偏差。战斗中先
后证伪四个假设：csname 含 `/`、INITEX `{}` catcode、尾递归/帧泄漏、
`\__hook_make_name:w` 空格定界（该项已由第十九刀 6caec85 单独落地）。

### 根因（四处，全部 tex.web/pdftex GT 逐字对拍）

1. **`\edef` 体把组定界原语当终止符**（`macros.rs scan_edef_body`）。
   tex.web scan_toks 的体终止符只有字符 `}`（cat 2）；`\begingroup`/
   `\endgroup` 原语及其 `\let` 别名原样存储、不参与 unbalance 配平
   （pdftex -ini GT `probes/k20-edef-group-verbatim.tex` T1-T3）。旧实现
   在 `Primitive(EndGroup)` 处截断，lthooks 归一化链
   `\group_begin: \use:e { \group_end: … }` 在首个 `\group_end:` 交付
   空串，余 token 落排版流 → 766 条 nullfont Missing character + 实参
   扫描失衡（实参组未闭合 fatal 的真源头）。

2. **`\the` 缺 uc_code/sf_code 读臂**（`save.rs`）。utf8.def l.148-154
   `\uccode`\noexpand\~=\the\uccode`\~`：tex.web scan_something_internal
   里 uc_code/sf_code 是合法 `\the` 操作数。缺臂使 utf8 区落兜底错误——
   且被 `--dump` 拒绝路径**先于**错误打印返回而表现为「静默止步」
   （去掉 `--dump` 重跑才现形；`stale-binary-diagnosis-trap` 同族的
   「现象被通道吞掉」教训）。

3. **文件名引号剥离只看首 token**（`io.rs more_name`）。latex.ltx
   l.9841 `\edef\@filef@und{"\@filef@und" }` 把找到的文件名存成
   `"name" `，而消费点 l.22832 `\@@input\@filef@und` 经宏展开后引号才
   出现在 more_name 循环**中段**。旧实现只剥首个 token 的引号 →
   `找不到文件："latex2e-…ltx"`（引号进文件名）。修复 = more_name 挂
   toggle 状态机（catcode 34 切换、引号态内空格入名），pdftex GT
   /tmp/k20q/qprobe.tex 三案剥壳一致；`\font` 名路径同步改签名。

4. **数字/尺寸扫描的条件机不步进 if 开始**（`scan.rs`）。l.8912 真形态
   `\dimen@\ifx\@TeXversion\@undefined 4\else\@TeXversion\fi\p@`：符号
   循环已有 maybe_eval_cond 臂（B 形态 `\ifx…\Z\fi\p@` 因此早已通过），
   但 **chardef 因子路径整段绕过数字循环**，且数量探针前的 flush 循环
   只步进 `\fi`/`\else`/`\or`——`\ifx` 放回挡住「数量乘内部量」探针，
   `\p@` 泄漏主流被当赋值目标再扫数字 → Missing number 恢复残流污染
   全链。修复 = flush 循环与数字循环补「if 开始就地求值 + 假分支丢弃」
   臂（tex.web 单位位 get_x_token §463）。GT：pdftex 与 NTex 均为
   `B=2.0ptC=2.0pt` 零报错（`probes/k20-dimen-cond-before-quantity.tex`，
   修复前 C 行 Missing number + `\p@` 泄漏）。

### 复测

复现命令（cargo 串行、timeout 500s、`NTEX_TFM_DIR=~/.ntex-fonts`）：

```bash
./target/release/ntex-dvi --no-plain \
  --input-path /tmp/fp11 --input-path ~/.TinyTeX/texmf-dist/tex/latex/base \
  --dump /tmp/fp11/latex6-fixed.fmt /tmp/fp11/ltxdump21.tex   # 日志 /tmp/fp11/ltx30.err
```

哨兵说明：latex.ltx 在 l.22837 **自带 `\dump`**（tex.web \dump=存格式+
final_end，作业终结、控制权不返回），包裹层「\input 后打标记」在任何引擎
都结构性不可达——`ltxdump21.tex` 用 `\let\latexp@dump\dump` + 重定义
`\dump` 把标记挂到真实 \dump 调用点，标记打出当且仅当 latex.ltx 全量载入
走到它自己的 \dump；dumped 仍由原语置位，fmt 照常落盘。

| 指标 | 修复前（第十九刀末） | 修复后 |
|---|---:|---:|
| `[LATEX-LTX-DONE]` | 未到达 | **到达**（ltx30.err l.1248） |
| `.fmt` | 未落盘 | **6,527,620 字节** |
| Missing character | 766 | **0** |
| 可恢复错误 | 5（+残流污染） | 15（全是既有旧账，见下） |
| `\dump` | 未执行 | **已执行** |

验收条款 (a)（标记+fmt）与 (b)（Missing=0、pos=768,709>756,000、\dump
已执行）**同时满足**。15 条错误构成：1× initex 引擎检查（基线既有）、
2× SetMathAlphabet `Command ''`（第十四刀已知）、6× `\???` doesn't
match + 6× hooks 'top-level' reserved（ltpara/socket 旧块，成对出现）。

### fmt 快路径（pass2）

```bash
./target/release/ntex-dvi --no-plain --fmt /tmp/fp11/latex6-fixed.fmt \
  --input-path /tmp/fp11 --input-path ~/.TinyTeX/texmf-dist/tex/latex/base <job>
```

- 空作业（`/tmp/fp11/pass2e.tex`）：**0.100s** 墙钟、`format=LaTeX2e
  version=2026-06-01`、0 错误（`/tmp/fp11/pass2f.err`）。fmt 载入 +
  catcode 往返 + everyjob 全部健康。
- 刀中曾观测到 job 起步 everyjob 风暴（49,947 条 `Missing endcsname`），
  原拟按新墙注册——根因修复 4 落地后**作为级联自然消失**，未及定版。
- 文档级：`\documentclass{article}` 经 fmt 走通 article.cls + size10.clo
  搜索路径载入，止步 `\setbox` Missing number（见新墙 1）。
- 探针坑：`--fmt` 与 plain 预载互斥（main.rs 注释明言），漏 `--no-plain`
  会叠 plain 状态，`\fmtname` 变 plain——pass2 计时必须用 `--no-plain`。

### 回归测试（tests_macro.rs，4 条新增；全套 86 条绿）

- `edef_stores_group_primitives_and_aliases_verbatim`：组原语/别名原样存储
  （根因 1；测试上下文 `:`/`_` 是 cat12，须先设 cat11；游离空格用 house
  idiom `\catcode32=9` 压掉——`expand()` 返回全部输出 token，主流程空格
  token 会混进断言串）。
- `dimen_number_loop_steps_conditional_machine_on_fi`：数字循环条件机
  （根因 4 的数字循环臂；`\ifx` 对两未定义 cs 判等取真分支，期望 9.0pt
  而非 18.0pt——首版期望值算错）。
- `dimen_flushes_cond_frame_before_internal_quantity`：chardef 因子路径
  条件帧先行消化（根因 4 主靶；修复前真实偏差，非期望值错）。
- `the_reads_uc_sf_code_tables`：`\the\uccode`/`\the\sfcode` 读臂（根因 2）。

### 定位开关留存（全部 env 门控、未设零成本）

NTEX_HOOK_TRACE（`[hook-def]`/`[hook-char]`，上一 agent 落地）、
NTEX_NUM_TRACE（Missing number 现场帧栈+token 转储）、NTEX_ARG_EOF
（实参组未闭合现场）、NTEX_ARG_DUMP（实参参数文本转储）、NTEX_COND_TRACE、
NTEX_DELIM_DBG。本次四根因里 1/4 由 NTEX_ARG_EOF 现场点名、2/3 由
「去掉 --dump 看真错误」定性。

### 新墙清单（一刀一墙，只注册不修）

1. ✅ **fmt 文档级**：article.cls/size10.clo 载入后 `\setbox` Missing
   number ×8，残流致「胶水上下文需要 \skip/\muskip 寄存器」fatal
   （`/tmp/fp11/doc1.err`，fmt 加载 0.113s 即达）——第二十一刀修复。
2. **`\meaning`/print_cs 控制字尾空格缺失**：pdftex
   `macro:->\relax ABC`，NTex `macro:->\relaxABC`——tex.web §246 print_cs
   控制字后补一空格，属打印层通用偏差（`\show`/`\meaning`/`\detokenize`
   共用面），GT `/tmp/k20p/mean.tex`、`probes/k20-edef-group-verbatim.tex`
   注释已标。
3. 既有旧账维持：SetMathAlphabet ×2、`\???` ×6、hooks top-level ×6、
   initex 引擎检查 ×1。

**验收结论**：`[LATEX-LTX-DONE]` 到达、latex.fmt 6,527,620 字节落盘、
Missing character 766→0、\dump 执行、fmt 快路径 0.1s 可用；`make check`
fmt+clippy -D warnings+测试 41 套件 **808 passed / 0 failed**（803 基线
只增不减）。第二十刀按验收收工。

## 第二十一刀（2026-09-18）：fmt 快路径 `\documentclass{article}` 端到端

### 现场判定

题设命令未带 `--no-plain`，实测会在 fmt 载入后再次预载 plain，先污染格式身份：

```bash
NTEX_BC_GUARD=1000000 NTEX_DELIM_GUARD=3000000 NTEX_HOOK_TRACE=1 \
./target/release/ntex-dvi --fmt /tmp/fp11/latex6-fixed.fmt ...
```

0.31s 内命中 BC guard，指纹为：

- 首错：`! LaTeX Error: This file needs format ... but this is ...`；
- guard：`\__prop_flatten:w` / hook `para/after` 区域，
  `Bytecode(pc=1381)` + `MacroArg(420tok,pos=43)`；
- 根因：**不是 article.cls 活锁**，而是 fmt 快路径漏 `--no-plain` 后叠 plain。

真正 pass2 命令必须沿第二十刀口径：

```bash
./target/release/ntex-dvi --no-plain --fmt /tmp/fp11/latex6-fixed.fmt \
  --input-path /tmp/fp11 --input-path ~/.TinyTeX/texmf-dist/tex/latex/base \
  /tmp/fp11/doc1.tex
```

### 根因与修复

`--no-plain` 后挂死消失，真实墙为 `size10.clo` 的 `\set@fontsize` 与页面尺寸计算：

1. `\baselineskip\f@linespread\baselineskip`（实际 `1\baselineskip`）要求
   `scan_dimen` 的 `<factor><internal dimen>` 分支把胶水参数降级为 width；
2. `\belowdisplayskip\abovedisplayskip` 要求 `scan_glue` 读取 assign_glue
   内部参数时复制完整 glue（三分量），不能退化成 width；
3. `\divide\@tempdima\baselineskip` 要求 `scan_int` 读取胶水参数 width 的 sp 整数
   （pdfTeX GT：`10pt` → `655360`，`25pt/655360=0.00003pt`）。

修复统一走既有 `param_kind_of`/`ParamValue` 表，避免为胶水参数再维护第二张原语清单。
新增回归：`glue_scan_accepts_internal_glue_params`、
`dimen_scan_multiplies_internal_glue_param_width`、
`number_scan_reads_internal_glue_param_width`。

### 验收

- doc1 成功：`/tmp/fp11/doc1.dvi`（383 字节，4 页，41 字体），墙钟 **0.23s**；
- DVI 字节含 `Hello,` 与 `LaTeX` 字符序列，非空页假阳性；
- 错误清零：`/tmp/doc1-after3.err` 中 `^!` 为 0；
- PDF 链登记：`cargo run -p ntex-pdf -- /tmp/fp11/doc1.dvi /tmp/fp11/doc1.pdf`
  成功，`/tmp/fp11/doc1.pdf` 75,538 字节、4 页，未开新战；
- `make check` 全绿；ntex-core 扫描回归新增 3 条，workspace 总量只增不减。
