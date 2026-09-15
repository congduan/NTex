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

### 残差（3 条，均不阻断）

- `Forbidden control sequence … scanning definition of ^^L`（l.26865 regex 段）
  ——根因#3（active/cs 共用 intern 槽），已记录可保留。
- `Missing number`（`<to be read again> \unhbox`）+ `Incompatible list can't
  be unboxed`（plain 预载段，expl3 之前）——第七刀已记为 preload 噪声；
  本刀从 3 条减到 2 条（`\global\chardef` 盒寄存器分配语义修直后）。

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
