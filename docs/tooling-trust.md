# 定位基础设施与仪器可信度（Tooling Trust）

> **一句话纪律：仪器必须先于结论被验证。** 本文是 NTex 调试设施的使用手册 +
> 仪器失真事故的登记与防复发清单。开工定位前先读本文。

---

## 0. 为什么会有这份文档

NTex 是「错误恢复式引擎」——它会吞掉错误继续跑。这让两类判断极易失真：

1. **「跑完了」≠「对了」**：22838 行 latex.ltx 能在 43ms 内"处理完"，实为多处
   缺口叠加的假象（latex-feasibility §2）。
2. **诊断工具自身的输出可能有 bug**：`\meaning` 长期丢失参数文本定界符
   （`macro:->` 而非 `macro:if->`），直接导致两轮定位跑偏（§38 → §39 → §41 才推翻）。

**结论：错误计数、跑完与否、以及诊断原语本身的输出，三者都不可盲信。**

---

## 1. 五次真实事故（登记在案，防复发）

| # | 事故 | 失真方式 | 后果 | 防复发设施 |
|---|---|---|---|---|
| 1 | `corpus-probe` 产物路径 | 假设产物在仓库根，实际 ntex-dvi 保留源文件目录 | 假阴性：全库判 **0/8**（实际都成功） | 已修（路径按 `tex.parent`）+ 三级判定 |
| 2 | `corpus-probe` 只判存在性 | 宏/格式文件的**空页**也算 PASS | 假阳性：曾报 8/8（实为 5/8+3 EMPTY） | 已修（非白像素 `MIN_INK=200`） |
| 3 | **`\meaning` 丢定界符** | 只渲染 `#n`，不渲染 `params.text` | **两轮定位跑偏**：§38 断言 → §39 固化 → §41 推翻 | ✅ `make instrument-check`（13 用例）|
| 4 | `--no-plain` 语义 | 跳过预载但保留 plain 表，非真 INITEX | `\the\catcode0` 给 12（应 9），init 域对拍失真 | ✅ 已修（`--no-plain` 切 INITEX 表）|
| 5 | 探针自伤 | 手写探针出错（`\let\else:` 配对、`%` 续行、缺 plain 定义） | 多轮浪费——实测 6 个探针里 3 个是探针自身问题 | ✅ `scripts/abcheck.py`（自动对拍）|

**共同模式**：把「工具的输出」当成了「世界的真相」。

---

## 2. 四件套（按使用频率排）

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
