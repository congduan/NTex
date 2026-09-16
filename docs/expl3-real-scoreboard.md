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
