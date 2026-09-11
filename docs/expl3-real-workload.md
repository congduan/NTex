# expl3 真实工作量评估（2026-09-11 深夜，主控实测）

> **本文档纠正 scoreboard 的一个根本性误读。** 数据全部来自实测，非估计。

## 一、核心结论：187 例分母不度量 expl3

`scripts/lvt-run.py` 的 `RAN` 判定 = 「跑到 `END-TEST-LOG`」，
**不比对 `.tlg` 期望输出**。

实测 `m3basics001`（判为 RAN）的转录：
- **77 个 `! Undefined control sequence.`**
- 去重后包含该测试的**全部测试目标**：
  `\cs_if_exist_use:N/:NF/:NT/:NTF/:c/:cF/:cT/:cTF`、`\cs_set:Npn`、
  `\cs_generate_variant:Nn`、`\exp_args:Nc`、`\int_set:Nn`、
  `\token_to_meaning:N`、`\tex_escapechar:D`、`\debug_on:n`

⇒ **187 例的 harness（`lvt-shim.tex`）不载入 expl3 本体**
（`grep expl3 scripts/lvt/lvt-shim.tex` 无载入语句；
`regression-test.tex` 亦不载入）。用例直接使用 expl3 函数 → 全部 undefined
→ 靠错误恢复跑到末尾 → 判 RAN。

**「RAN 180/187 = 96.3%」度量的是「NTex 能带着一堆 undefined 跑到末尾」，
不是 expl3 能力。**

## 二、expl3 本体真实规模

| 项 | 值 |
|---|---|
| `expl3-code.tex` | **40,266 行**（1.39 MB） |
| 已入库的 `l3kernel/*.dtx` 合计 | ~119,000 行 |
| l3kernel 官方 `.lvt` | 187 |
| latex3 全仓 `.lvt`（含 l3packages/l3trial） | **266** ← 更真实的分母候选 |

## 三、直接载入 expl3 本体的真实进度（实测）

```bash
printf '\\input expl3-code.tex\n\\end\n' > probe.tex
./target/debug/ntex-dvi probe.tex 2>&1 > out.txt
```

| 指标 | 值 |
|---|---|
| 载入到达行 | **25,853 / 40,266 = 64%** |
| 错误总数 | **3,114** |
| 终止原因 | `排版失败：非法输入：双重下标（Double subscript）` |

### 错误类型分布（真实计数）

| 次数 | 错误 | 判定 |
|---|---|---|
| **1,503** | `Missing = inserted for \ifnum` | `<to be read again> exp_stop_f:` —— `\exp_stop_f:` 是 `~`（cat 10 空格），NTex 的 `\ifnum` 扫描在比较符位置读到空格后**报错**（pdfTeX 静默跳过） |
| **1,456** | `Font \FONT? has only 13 fontdimen parameters` | `\fontdimen` 越界（expl3 探测字体能力） |
| 32 | `Missing number, treated as zero` | 数值扫描 |
| 20 | `Extra \fi` / `\or` / `\else` | **条件栈不平衡** |
| 19 | `Improper alphabetic constant` | |
| 10 | `Use of \??? doesn't match its definition` | |
| 9+4 | `Argument of \__iow_wrap_line_loop:w has an extra }` | 组扫描 |
| 8 | `Missing endcsname inserted` | |
| 6 | `Undefined control sequence` | |

## 四、可复现的引擎差异（已对拍 pdfTeX）

```tex
\let\stopmark=~
\ifnum 1\stopmark=1 \immediate\write16{TRUE}\else\immediate\write16{FALSE}\fi
```

| | pdfTeX | NTex |
|---|---|---|
| 结果 | `FALSE` | `FALSE`（一致） |
| 报错 | **无** | **2 个**（`Missing = inserted for \ifnum` + `Missing number`） |

⇒ 结果一致但**错误恢复语义不一致**。1503 次这个错误说明它是主噪声源，
但不改变最终取值——**是「脏转录」而非「错结果」**。
（对照：`\ifnum 1=1` 两边都正确。）

## 五、终止点（最高价值的下一刀）

```tex
l.25851  { \__tl_analysis_a_group:nw { \exp_after:wN ^^@ \if_false: } \fi: } }
l.25852  \char_
              set_catcode_group_end:N ^^@
l.25853  \cs_new_protected:Npn \__tl_analysis_a_egroup:w
```

上下文（`expl3-code.tex` L25847+）：
```tex
\group_begin:
  \char_set_catcode_group_begin:N \^^@ % {
  \cs_new_protected:Npn \__tl_analysis_a_bgroup:w
    { \__tl_analysis_a_group:nw { \exp_after:wN ^^@ \if_false: } \fi: } }
  \char_set_catcode_group_end:N \^^@
  \cs_new_protected:Npn \__tl_analysis_a_egroup:w
    { \__tl_analysis_a_group:nw { \if_false: { \fi: ^^@ } } % }
\group_end:
```

**首错 = `\char_set_catcode_group_end:N` 被切成 `\char_` + `set_catcode_group_end:N`**
⇒ 该处 `_` 不是 cat 11。**且 `\^^@` 刚被设为组定界符**（`\char_set_catcode_group_begin:N \^^@`），
说明 **`^^@` 作为组定界符的机制与 `_` 的 catcode 交互**是最可疑处。

⚠ 注意：**验证过 `\char_set_catcode_group_begin:N` 不破坏 `_` 的 catcode**
（最小探针 `\char_set_catcode_group_begin:N \^^@` 后 `\mycmd_x` 仍正常解析）。
⇒ 破坏发生在更早，或与 `^^@` 定界符的组扫描有关。**需要先定位「从哪一行起 `_` 变 cat 8」**。

## 六、下次开工的第一刀（单点最大信息量）

**不要**再去猜。用二分法定位 expl3 载入中 `_` 的 catcode 何时失守：

```bash
cd /home/ubuntu/NTex && export PATH="$HOME/.cargo/bin:$PATH"
# 在载入点插 catcode 探针：每 N 行输出一次 \catcode`\_ 与当前行号
# 或：把 expl3-code.tex 二分截断，看 25851 行错误在哪个前缀下首次出现
```

**已知边界**：L37 起就有 `Use of \@ doesn't match its definition`（首个错误），
L25851 是**终止点**。先查 L37 那个首错是否与 `_`/`@` catcode 相关。

## 七、诚实的工作量判断

**无法给出数字。** 理由：
1. 当前分母（187）不度量 expl3（见 §一）；
2. expl3 本体 40,266 行，实测到 64% 且 3,114 错；
3. 更真实的分母是 266（全仓），且 `.tlg` 精确比对从未启用。

**结构性判断**：瓶颈在**底座**（catcode 自举 + 条件/数值扫描语义），
不在收尾那 7 例。收尾 7 例是「错结果」，底座是「载不进来」——
后者是前者的前提。

---

# 附：首个可定位错误的真相（2026-09-11 深夜追加）

## 首个带行号的错误

```
! Undefined control sequence.
\AtBeginDocument
l.815   \__kernel_primitive:NN \shbscode              \tex_shbscode:D

! Argument of \__file_tmp:w has an extra }.
! Argument of \__file_full_name_aux:Nnn has an extra }.
（级联 4+ 次）
```

## 两个关键事实

**1. `\AtBeginDocument` 在 NTex 上是 undefined**
- 它是 **LaTeX 内核**命令；`expl3-code.tex` L13157 用它包裹一段代码块
- `lvt-shim.tex`（plain 垫片）**未提供 `\AtBeginDocument`**
- ⇒ `\cs_if_exist:NT \@filelist { \AtBeginDocument {...} }` 的分支被错误执行

**2. 行号严重错位（次级症状，但证据价值高）**
- 报错说 `l.815`，但 `\AtBeginDocument` 实际在 **L13157**
- 报错现场前后的输出是变量 `\count` 定义（`\c__ior_term_noprompt_ior=\count36` 等），
  突然跳到 `\__file_tmp:w` → **说明宏展开把执行流带到了极远处，NTex 的
  「当前行号」与真实执行位置不同步**

## 判定：这是**脚手架缺陷**（不是引擎缺陷）

对照仓库纪律（`docs/tooling-trust.md` 十七种失真 + 「怀疑顺序：仪器→被测→脚手架」）：

- 症状形态（`_` 被切断、双重下标、catcode 异常）**看起来完全像引擎 bug**
- 实际根因是 **harness 缺 LaTeX 符号**（`\AtBeginDocument`）+ **行号追踪失真**
- 这与 2026-09-11 那轮「爆栈 4 次误归因 → 最终定案为脚手架（harness 载入两次）」
  **是同一类事故的第二次出现**

## 下一刀（单点最大信息量，不要猜）

```bash
# 1. 补 harness 缺失的 LaTeX 符号，看载入推进多少
#    \AtBeginDocument / \AtEndDocument / \@filelist（需实测还缺哪些）
# 2. 行号错位单独查：NTex 的 current_line_no 在长 \input 链中是否失同步
#    （这本身就是值得修的诊断能力缺陷——「仪器失真」）
```

⚠ 补 shim 时注意 `docs/expl3-lvt-scoreboard.md` 记录的 shim 13 坑
（`~`=cat10、内部名不用 `_`、`\debug_on:n` 等）。

## 真实工作量（修正后的诚实结论）

1. **必须先分清「脚手架缺陷」与「引擎缺陷」**——当前 3,114 个错误里，
   至少首错簇（`\AtBeginDocument` + 行号错位 + `\@` 不匹配）是脚手架
2. expl3 本体 40,266 行 / 实测到 64%（行号不可信，实际可能更低）
3. **187 例分母无效**（不载入 expl3，见 §一），真实分母候选 266
4. **在脚手架修对之前，任何「还剩多少」的数字都不可信**——
   这与「仪器必须先于结论被验证」是同一条纪律

---

# 🎯🎯 突破性结果：expl3 本体可零错误载入，条件机制根因锁定

## 实验结果（决定性）

```bash
# 只补两个 LaTeX 符号
cat > drv.tex <<'EOF'
\let\AtBeginDocument\relax
\let\AtEndDocument\relax
\input expl3-code.tex
\immediate\write16{[LOAD-DONE]}
\end
EOF
```

| | 补 `\AtBeginDocument` 前 | 补后 |
|---|---|---|
| 错误总数 | **3,114** | **0** ✅ |
| 载入完成 | 中止于 L25851 | **`[LOAD-DONE]`** ✅ |

⇒ **那 3,114 个错误全部是脚手架缺陷**（harness 缺 LaTeX 符号），
**不是引擎缺陷**。expl3-code.tex（40,266 行）NTex **能完整载入**。

## 载入后 expl3 真实状态（关键判据）

```tex
\cs_if_exist:NTF \tex_else:D {...}     → DEFINED      ✅ 别名族已建立
\meaning\else:                          → \else:       ✅ 别名链完整
\ifnum 1=2 A \else: B \fi:              → B（A-ELSE） ✅ 别名条件正常
\meaning\if_false:                      → \iffalse:    ✅
\meaning\prg_return_false:              → undefinedreturnfalse:  ❌ 畸形！
```

**⚠ 我此前「expl3 底座从未建立、\tex_global:D undefined」的结论是错的**——
那是在**不完整载入**（缺 `\AtBeginDocument` 导致提前崩溃）下测的。
**同一个错误的第二次出现：我又一次在脚手架坏掉的状态下测被测对象。**

## 条件机制的真实根因（`l3basics.dtx:1966-1969`）

expl3 的 `:TF` 分派**不靠 `\if`**，靠**可展开 token 选择**：

```tex
\cs_gset:Npn \prg_return_true:  { \exp_after:wN \use_i:nn  \exp:w }
\cs_gset:Npn \prg_return_false: { \exp_after:wN \use_ii:nn \exp:w}
```

- `\prg_return_true:` 展开 → 保留第 1 参数（`\use_i:nn`）
- `\prg_return_false:` → 保留第 2 参数（`\use_ii:nn`）
- 依赖 `\exp:w ... \exp_end:` **完全展开**机制

**NTex 上 `\meaning\prg_return_false:` = `undefinedreturnfalse:`：**
⇒ `\cs_gset:Npn \prg_return_false:` 定义时**名字解析错误**
（`\prg_return_false:` 被切开，`false:` 变成裸文本粘连）。

**这才是 `\bool_if:NTF` / `\int_compare:nNnTF` 两分支都执行的真正根因**——
`\exp:w`/`\use_i:nn`/`\use_ii:nn` 展开选择机制不工作。

## 与 cond_op 修复的关系

`cond_op` 不解析 Alias 是**真 bug**（已修，别名条件 `\else:` 现在工作 ✅），
但**不是** expl3 `:TF` 全灭的根因——expl3 走的是 `\prg_return_*` + `\exp:w`
机制，**两条路径**。修复仍然正确且有价值（覆盖 `\let` 别名条件场景）。

## 下一刀（单点最大信息量）

```tex
% 最小复现：\prg_return_* 的展开选择机制
\exp_after:wN \use_i:nn \exp:w  ... \prg_return_true: ... \exp_end:
```
1. 先查 `\cs_gset:Npn \prg_return_false:` 的**名字解析**为何出 `undefinedreturnfalse:`
   ——高概率是 `\cs_gset:Npn` 对**带 `:` 的参数**（expl3 名字）处理有偏差，
   或 `\exp_after:wN`/`\exp:w` 展开时机错位；
2. 再查 `\exp:w ... \exp_end:` 完全展开在 NTex 的实现（`l3expan.dtx`）；
3. **修复 harness**：把 `\AtBeginDocument`/`\AtEndDocument`（及实测还缺的
   LaTeX 符号）补进 `scripts/lvt/lvt-shim.tex` —— **这一项立即让 187 例
   从「假 RAN」变成「真载入 expl3」**，是解锁全部度量的前提。

## 工作量结论（修正）

**分母必须换。** 在 harness 修好前，187 例的 RAN 数毫无意义（实测见 §一）。
修好 harness 后，分母会是 187（或 266）例的**真实 .tlg 比对**。
届时才谈得上「还剩多少」。

---

# 🏁 里程碑：187 例的 harness 修好，expl3 真载入

## 关键修复（`scripts/lvt/lvt-shim.tex`）

1. **补 `\AtBeginDocument` / `\AtEndDocument`**（LaTeX 内核符号，plain 无）
   —— 消除手工载入 expl3-code.tex 时的 3,114 个错误（实测归零）。
2. **载入器必须用 `expl3.ltx`，不是 `expl3-code.tex`**：
   `expl3-code.tex` 有 loader 检查
   （`\expandafter\ifx\csname ExplLoaderFileDate\endcsname\relax` →
   `\PackageError{expl3}{No expl3 loader detected}`），
   而 `expl3.ltx` 首行 `\let\ExplLoaderFileDate\ExplFileDate` 提供该标志。

## 真载入前后的对比（实测 `m3basics001`）

| 指标 | 假测试（harness 未载 expl3） | 真载入（`\input expl3.ltx`） |
|---|---|---|
| `! Undefined control sequence` | **77** | **6** |
| expl3 loader 错误 | — | 0（载入成功） |
| 跑到 `END-TEST-LOG` | 是（假 RAN） | 否（exit=1，中止） |
| 主要错误 | 全部是 expl3 函数 undefined | `Missing = inserted for \ifnum` 1503 / `Font has only 13 fontdimen` 1456 / … |

⇒ **换判据后的第一手真实数据**：expl3 真载入后，`m3basics001` **跑不完**，
被 1503 个 `\ifnum` 错误淹没。**这才是 expl3 在 NTex 上的真实状态。**

## 为什么 187 例判定没变（重要）

补 `\AtBeginDocument` 后重跑全量，判定分布**完全一致**（180 RAN / 5 CRASH /
1 STACK / 1 NO-END）——因为 **187 例的 harness 从不载入 expl3**
（`lvt-run.py` 的临时目录只复制 shim + regression-test + 用例）。

⇒ **要得到真实分数，`lvt-run.py` 必须把 `expl3.ltx` 也放进临时目录并在 shim 里载入。**

## 下一步（唯一正确的顺序）

1. **`lvt-run.py` 接线**：
   - 从 latex3 仓库/l3build 生成 `expl3.ltx` + `expl3-code.tex`（或缓存）；
   - 复制进每例临时目录；
   - `lvt-shim.tex` 顶部改为 `\input expl3.ltx`（在用例 `\input{regression-test}` 之前）；
   - **注意 harness 载入两次的旧坑**（`docs/tooling-trust.md`）。
2. 接线后重跑全量 → **这将是 expl3 的第一个真实分数**。
3. 然后才是修 `\ifnum` 扫描（1503 次那个错误簇）。

⚠ 在 (1) 完成前，**不要**再引用「RAN 180/187」作为 expl3 进度。
