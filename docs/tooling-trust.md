# 定位基础设施与仪器可信度（Tooling Trust）

> **一句话纪律：仪器必须先于结论被验证。** 本文是 NTex 调试设施的使用手册 +
> 仪器失真事故的登记与防复发清单。开工定位前先读本文。

---

## 0. 为什么会有这份文档

NTex 是「错误恢复式引擎」——它会吞掉错误继续跑。这让两类判断极易失真：

1. **「跑完了」≠「对了」**：22838 行 latex.ltx 能在 43ms 内"处理完"，实为多处
   缺口叠加的假象（archive/latex-feasibility.md §2）。
2. **诊断工具自身的输出可能有 bug**：`\meaning` 长期丢失参数文本定界符
   （`macro:->` 而非 `macro:if->`），直接导致两轮定位跑偏（§38 → §39 → §41 才推翻）。

**结论：错误计数、跑完与否、以及诊断原语本身的输出，三者都不可盲信。**

---

## 1. 九次真实事故（登记在案，防复发）

| # | 事故 | 失真方式 | 后果 | 防复发设施 |
|---|---|---|---|---|
| 1 | `corpus-probe` 产物路径 | 假设产物在仓库根，实际 ntex-dvi 保留源文件目录 | 假阴性：全库判 **0/8**（实际都成功） | 已修（路径按 `tex.parent`）+ 三级判定 |
| 2 | `corpus-probe` 只判存在性 | 宏/格式文件的**空页**也算 PASS | 假阳性：曾报 8/8（实为 5/8+3 EMPTY） | 已修（非白像素 `MIN_INK=200`） |
| 3 | **`\meaning` 丢定界符** | 只渲染 `#n`，不渲染 `params.text` | **两轮定位跑偏**：§38 断言 → §39 固化 → §41 推翻 | ✅ `make instrument-check`（13 用例）|
| 4 | `--no-plain` 语义 | 跳过预载但保留 plain 表，非真 INITEX | `\the\catcode0` 给 12（应 9），init 域对拍失真 | ✅ 已修（`--no-plain` 切 INITEX 表）|
| 5 | 探针自伤 | 手写探针出错（`\let\else:` 配对、`%` 续行、缺 plain 定义） | 多轮浪费——实测 6 个探针里 3 个是探针自身问题 | ✅ `scripts/abcheck.py`（自动对拍）|
| 6 | 转录靠手工 grep | 5620 条错误里只能看到「数量最多的那类」 | 首现场判断反复出错（曾以为在 l.21291，实为 **l.301**，早 2 万行）| ✅ `scripts/logtrace.py`（首现场/震中/级联形状）|
| 7 | **oracle 未冻结 + 旧二进制** | fh9 定位战拿「pdfTeX 会正常恢复」当已知起点，实际其在恢复 edge case 的行为未探明（`\ifcsname` 假条件+`\else:` 别名输出 `[Z]` 丢弃 N——未建档语义）；同时结论基于 stale 二进制 | 25 个手工变体、结论反转 3 次、197 秒无进展被打断；「scan_csname 静默截断」定性被矩阵直接证伪 | ✅ `scripts/abmatrix.py`（freeze 前强制人工核对 + `oracle-verify` 自检）|
| 8 | **栈转储底/顶反转**（2026-09-13 修） | `NTEX_STACK_DUMP_FRAMES` 取 `stack[..head]`（栈**底**）却按 `n-1-i` 标注成「栈顶帧」；且转储只挂在 `fetch` 的 `>` 兜底上，而宏帧守卫用 `>=` **先命中**——宏递归爆栈**永远看不到转储** | 09-11 从转储读出「栈顶 4997 个 TokenList 帧」，实为栈底视图；据此判断的循环主体不可信；本次爆栈只报「`\q_stop` 递归疑似无终止」而拿不到现场 | ✅ 抽 `frame_dump_plan` 纯函数 + 方向性单测（`tests_diag.rs`，8 例）；两处守卫共用入口；帧型补齐至全部 9 种 |
| 9 | **GT 采集方法缺陷：期望值抄自失真通道**（2026-09-14 定性，457615a 四刀 31 红复盘） | 三条通道全部失真：① 宏扰动 `\write16` 形探针——`\the` 藏在 `\write` 实参里，**不经过**数字循环的 `get_x_token` 现场，采到的是赋值后的值（`\advance\count200by3\the\count200` 裸形 GT=`40`，write 形采到 `8`）；② 盒内容（`\showbox`）通道混入 italic-kern/前次残留，不能当 token 流真值（`expandafter` meaning 期望成 kern 残留文本）；③ plain 格式初表掩盖读时序——`lccode`B` 在 plain 预设 98，赋值前/后读都给 98，换成初表为 0 的字符（`!`）才分得出「赋值前读」 | 31 项门禁红灯里 30 项是期望值错而非引擎错；12 条期望按 pdftex 裸形真值重校准，引擎只收窄 1 处（`scan_keyword` 补 `get_x_token`） | ✅ 纪律入 `archive/expl3-lvt-scoreboard.md`「GT 采集方法缺陷更正」节：**期望值必须采自 pdftex 裸形/字符流实测**（探针里不许夹 `\write`/`\showbox` 中介），且先确认所测字符在**被测格式的初表里非平凡** |
**共同模式**：把「工具的输出」当成了「世界的真相」。#9 的变体：把「**探针形状下的输出**」当成了「**待测语义的真相**」——探针形状本身就换了语义（write 形 vs 裸形读时序不同），先问「这条探针真的在测我说的那个位置吗」。

---

## 2. 七件套（按使用频率排）

### 2.1 `make instrument-check` — 仪器自检（**改诊断原语后必跑**）

```bash
make instrument-check
```

对拍 13 个诊断原语用例（`\meaning` 的定界符/参数/混合、`\the`、`\number`、
`\romannumeral`、`\string`、`\catcode` 初表 …）与 `pdfTeX` 输出。**失真即回归**。

- 脚本：`scripts/instrument-check.py`
- 依赖：`~/.local/bin/pdftex`（TinyTeX）
- 当前状态：**13/13 一致**
- 何时跑：改了 `meaning_text` / `slot_display` / `show_toks` / `\the` 读回 /
  `\string` 类原语之后；发版前。

### 2.2 `scripts/abcheck.py` — 双引擎差分对拍（**写探针时用**）

```bash
scripts/abcheck.py probe.tex --trace          # 比对 \message 信号
scripts/abcheck.py probe.tex --plain          # 两侧都 \input plain
scripts/abcheck.py probe.tex --grep '^R='     # 自定义抽取
make abcheck TEX=probe.tex ARGS=--trace
```

pdfTeX 侧是 ground truth。探针约定与样例见 `scripts/abcheck-examples/`。

**探针写法契约**（避免事故 #5）：
- 信号用 `\message{KEY=VALUE}`（KEY 形如 `[A-Za-z_][A-Za-z_0-9]{0,19}`）
- 每个 `\message` 单独一行；结尾用 `\end`
- 不依赖两侧行结构（pdfTeX log 内联混排、NTex 拼接无换行，脚本按标记切片）
- 正则**不要依赖尾随空白**（NTex 的 message 落在行尾，如 `hyphenationI=42`）

### 2.3 `NTEX_TRACE_JSONL` + `scripts/trace-view.py` — 结构化 trace（**查栈膨胀时用**）

```bash
NTEX_TRACE_JSONL=/tmp/t.jsonl ntex-dvi doc.tex
scripts/trace-view.py /tmp/t.jsonl --summary     # 概览 + 栈深峰值
scripts/trace-view.py /tmp/t.jsonl --spikes      # 栈深增长最快区间 ← 自我复制宏链
scripts/trace-view.py /tmp/t.jsonl --grep cs_generate
scripts/trace-view.py /tmp/t.jsonl --stack-at 50000   # 重建那一刻的栈
scripts/trace-view.py /tmp/t.jsonl --around 50000
```

事件 schema：`{"step","kind","tok","csid","depth","frame","extra"}`，
`kind ∈ {push,pop,fetch,expand,cond,note}`。

**为什么不是 `eprintln`**：watchdog 只给「最后一帧 + 栈快照」，无法回答
**「是谁 push 了它」**。JSONL 可事后按字段查询、跨运行比对。

- 实现：`crates/ntex-core/src/expand/mod.rs` 的 `pub mod trace`（native + wasm 空实现）
- 零成本开关：未设环境变量时只做一次缓存查表
- 每条 flush（static 无 Drop，不 flush 会丢全部事件——实测踩过）

### 2.4 `scripts/blocker-track.sh` — 阻塞点单调性看板

```bash
scripts/blocker-track.sh           # 跑 probe → 追加历史 → 单调性判定
scripts/blocker-track.sh --show    # 只看历史
```

自动抽取「最远到达行 + 首错签名 + 终止方式 + 栈超限命中」，与上一条对比给出：

| verdict | 含义 | 动作 |
|---|---|---|
| `FORWARD` | 行号前移 | ✅ 健康（剥洋葱） |
| `STALL` | 位置与签名均未变 | ⏸ 换靶或换角度 |
| `SIGMA-CHANGE` | 同位置但错误类型变了 | ⚠ 可能「根因被推翻」或修复改判——**勿当停滞** |
| `REGRESSION` | 行号后退 | 🔴 回归，先查 diff |

历史：`docs/blocker-history.tsv`（入库，作为战役仪表盘）。
退出码：`REGRESSION`/`SIGMA-CHANGE` 非零（可接 CI）。

### 2.5 `scripts/logtrace.py` — 转录/log 结构分析（**拿到 5000+ 错误时用**）

```bash
make logtrace LOG=/tmp/r27b/latex.ltx.transcript
python3 scripts/logtrace.py X.transcript --first-new --context 8
python3 scripts/logtrace.py X.transcript --compare pdftex.log   # 找"我方独有"
python3 scripts/logtrace.py X.transcript --emit-seq /tmp/seq.txt # 跨轮 diff
python3 scripts/logtrace.py X.transcript --kinds trace,restore,file
```

**解决的痛点**：latex.ltx 转录有 **5620 条错误**，人工 grep 只能看到「数量最多的
那类」，但真正要回答的是结构性问题。本工具把错误洪流压成五段报告：

| 段 | 回答的问题 |
|---|---|
| ① 首现场 | 第一条错误 + 上下文（**注意：可能不是你以为的那条**）|
| ② 震中 | 错误最集中的位置行 topN（错误火山口）|
| ③ 级联形状 | **扇出**（同一错误重复 N 次）vs **链式**（跨多行传播）|
| ④ 首次出现序 | 每种错误「冒头」的顺序 —— **第一个新种类比第一个错误更接近真起点** |
| ⑤ 归一化序列 | 去数字/去 cs 名后可直接跨轮 diff |

**`--compare` 是与 pdfTeX 参考 log 对比的关键能力**：把错误分三类——
「我方独有」（**真偏差**）/「参考有我方无」（吞错或语义缺失）/「共有但数量差异大」。

**实测价值（首次运行即产出）**：对 5620 条错误的转录，工具立刻指出真首现场是
**`l.301`**（texsys 探测区），而非此前人工反复定位的 `l.21291`——**早了 2 万行**；
震中是 `l.21039`（占 22.3%）。

---

### 2.6 `make recovery-check` / `make oracle-verify` — 错误恢复语义矩阵（2026-09-12 新设）

**abcheck 只对拍「正常输出」；报错有无 + 恢复后行为的对拍走这里。**

```bash
make oracle-verify     # oracle 仪器自检：复跑 pdfTeX 对比冻结判据（先跑这个）
make recovery-check    # 跑 NTex 出发散地图（OK/DIFF；DIFF=恢复语义发散，非失败）
python3 scripts/abmatrix.py freeze [case…] --note "人工核对记录"   # 冻结/重冻结
```

- 语料库：`fixtures/recovery/cases/<name>/{case.tex, oracle.expect, note.md?}`；
  首批 32 case = `\ifcsname` 错误恢复家族（fh9 战役 25+ 变体沉淀，判据见各 note.md）
- 判定只比「错误有无 + `\write16` marker 输出（`X:[…]`）」，**不比错误措辞**
  （中英文必异；措辞分歧在 run 表格错误对比列留档）
- **判读**：修复验收 = 指定 case DIFF→OK 且 `oracle-verify` 无 DRIFT；矩阵单调转绿
- 基线（2026-09-12）：**OK 32 / DIFF 0**——含 NTex 复现 pdfTeX 未建档怪异
  （U2fix `[Z]`、chk `[Z][Z][NZ]`：假 `\ifcsname` 条件丢弃 N 分支，疑与
  `\last_cs_name` 缓存有关；NTex 行为一致 ⇒ **非修复目标**）
- **freeze 前必须人工核对语义**（事故七：盲 freeze 会把参考引擎自身的
  未建档行为冻成「规范」）
- **防旧二进制门禁**：`scripts/ntex_bin.py::ensure_fresh()` 对比源码树 mtime，
  落后即自动 `cargo build -p ntex-dvi`（失败硬退；`NTex_SKIP_REBUILD=1` 逃生口）。
  abcheck / abmatrix / lvt-run / frame_dump / page-eject-matrix 已接线——
  **新诊断 runner 一律走 `ensure_fresh()`，不再手工保证构建新鲜**
  （blocker-track 走 `cargo run` 天然免疫）。

### 2.7 `NTEX_STACK_DUMP*` / `NTEX_CALL_TRACE` — 爆栈现场转储（**查「谁在自复制」时用**；2026-09-13 修，见事故 #8）

```bash
# 最小现场：类型直方图 + 栈顶签名重复度 + 宏调用轨迹
NTEX_STACK_DUMP=1 NTEX_CALL_TRACE=40 ntex-dvi probe.tex

# 需要 token 级现场时再加逐帧详情（默认栈顶 60 帧 + 栈底 5 帧）
NTEX_STACK_DUMP=1 NTEX_STACK_DUMP_FRAMES=1 ntex-dvi probe.tex
NTEX_STACK_DUMP=1 NTEX_STACK_DUMP_FRAMES=1 NTEX_STACK_DUMP_TOP=0 NTEX_STACK_DUMP_BOTTOM=45 ntex-dvi probe.tex
```

| 变量 | 作用 |
|---|---|
| `NTEX_STACK_DUMP` | 帧型直方图 + **栈顶帧签名重复度** + 调用轨迹（三者都开） |
| `NTEX_STACK_DUMP_FRAMES` | 逐帧 token 级详情（单独设置亦生效） |
| `NTEX_STACK_DUMP_TOP=N` | 栈顶帧条数（默认 60；`0` = 不看栈顶） |
| `NTEX_STACK_DUMP_BOTTOM=N` | 栈底帧条数（默认 5；**调大看「循环墙从哪一帧起」**，墙的起点在栈底侧） |
| `NTEX_CALL_TRACE=N` | 宏调用环形轨迹（默认关 = 零开销；N = 显示段数，缓冲上限 64K 段） |

**为什么需要调用轨迹**：栈帧只包含**未弹出**的帧，而递归循环的入口帧早已弹出。
例：expl3 载入 l.27200 的爆栈现场栈上 4997 帧全是 `\q_stop`，看不出上游是谁；
轨迹一次给出入口链 `… \__codepoint_data_auxi:w → :auxii:w → :auxiii:w → \cs_set_nopar:cpe → \exp_args:Nc → \q_stop`。

**与 §2.3 的分工**：`NTEX_TRACE_JSONL` 是全量结构化 trace（可事后任意切片、跨运行比对，
但要落盘、每条 flush，重载语料下体积大）；本节的环形缓冲是**同进程内、近零开销、
活到溢出那一刻**的现场快照，适合「跑 4 万行 expl3 直到爆栈」这种长距离场景。
两者互补：先用本节锁定入口链，再用 §2.3 追细粒度事件。

**判读要点（血泪）**：
- 栈顶看**循环主体**（谁在自复制），栈底看**起点**（循环从哪一帧开始）——`TOP=0 BOTTOM=N`
  是定位「墙的起点」的正确姿势；两段用途不同，别混。
- 帧详情里 `head=` 是「这一帧是什么」，**`at=` 才是当前现场**：长宏体/长实参
  （本例 439 token 的 MacroArg）上只看 `head=` 会完全失明。
- 签名重复度是**循环主体的直接证据**：`60 × Bytecode[\q_stop end]` 一句话定性。

---

## 3. 判读纪律（血泪沉淀）

1. **进度指标 = 阻塞点位置单调前移**，不是「跑完」、不是错误计数。
2. **指标跃迁（0/8 → 8/8、错误数腰斩）出现时，先怀疑仪器再庆祝。**
3. **文档待办滞后于实测**：先复测再销账（§38 的「24 错」在 `cd98a94` 后已消解）。
4. **插桩一次拿全数据**，不要「加一行 eprintln 跑一次」。环境变量门控多处同时打点。
5. **改 `@` 类 cs 先 `\catcode`\@=11`**（plain.tex L1239 把 `@` 改回 12；
   预载后 `\if@` 不可访问是**正确行为**，pdfTeX 同样切成 `\if` + `@`）。
6. **`> file 2>&1` 全缓冲丢日志**：SIGKILL/OOM 时 eprintln 缓冲丢失。
7. **`strings` 只提取 ASCII**：中文错误消息会被滤掉，用 `grep -a` 直接作用于原始输出。
8. **oracle 侧未知语义**：对拍前先冻结参考引擎行为并人工核对（`abmatrix freeze`），
   两个未知数相减不是定位；NTex 与 pdfTeX **一致地怪** ⇒ 语义事实，不是 bug。
9. **转储的「标注」也是仪器的一部分**（事故 #8）：序号/方向/标签错位与内容错位同罪。
   看到「同一个名字重复几千次」先问一句**这是栈顶还是栈底**——顶看主体，底看起点。
   修仪器本身要配方向性单测（`tests_diag.rs`），否则下次还会往反方向读。
10. **对拍一个假设只需一次实验，但别让实验自己变成新假设**：本次 `\q_stop` 排查中
   「定界符匹配失败」假设被连续两个对照电池（单 token / 跨组边界 / `;`+空格 多 token）
   直接证伪——**先证伪再深挖**，比沿着错误假设读 300 行源码便宜得多。

---

## 4. 事故 #3 的完整复盘（最有价值的一课）

| 轮次 | 做了什么 | 依据 | 结局 |
|---|---|---|---|
| §38 | 断言「plain 预载 24 错锚定 `\if@`」 | `\meaning` 输出 | 靠的是失真的仪器 |
| §39 | 固化 RED 测试「`\uppercase` 内 `\gdef` 参数文本丢失」 | 同上 | 根因判断错误 |
| §40 | 「收敛」到 `\uppercase` 语义问题 | 同上 | 仍在错误方向 |
| §41 | **插桩**（`NTEX_UPCASE_DBG` + `NTEX_PARAM_DBG`）| 一次拿全数据 | 推翻前三轮：参数文本**从未丢失**，真缺陷是 `\meaning` 不渲染 `params.text` |

**代价**：三轮定位 + 三轮文档 + 两次提交，净产出是**一行显示层修复**。

**若当时先跑 5 分钟的 `make instrument-check`**（或简单地用 pdfTeX 对拍
`\meaning` 输出），三轮全部可省。

**这就是本文存在的理由。**
