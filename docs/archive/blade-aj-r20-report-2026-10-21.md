# R20：testmath 原始代码泄漏假设核查（WIP）

日期：2026-10-21；起点 `0f30375`，未改引擎代码。用户提出的「amsmath 高级命令未定义，约 180 处源码上纸」假设经官方原文和 pdfTeX PDF 对照被证伪。此报告保留原始输出路径；不能将官方文档有意展示的代码计入泄漏。

## p22 最小块与定义状态

官方 `testmath.tex` 第 1386–1401 行将同一 `align` 块写了两遍：第一遍是真公式，第二遍在 `verbatim` 环境展示源码。GT PDF p23 也依次呈现公式和 `\begin{align}...\pmod{m^2}...\end{align}` 原文。`\genfrac` 同样在第 1438–1457 行先排公式、后进 `verbatim`；p17 的 `\boldsymbol` 示例亦含明确的代码展示段。

独立探针 `/tmp/doc-tests/r20-probe.tex` 使用 `\typeout{\meaning...}` 并排三种数学构造。`/tmp/doc-tests/r20-probe.ntex.out` 与 `.gt.out` 三行逐字相同：

```text
R20-PMOD=macro:->\protect \pmod  
R20-GENFRAC=macro:->\protect \genfrac  
R20-BOLDSYMBOL=macro:->\protect \boldsymbol  
```

NTex 探针转录没有 `!` 错误；`/tmp/doc-tests/r20-probe.pdf` 提取为 `x ≡y + 1 (mod m2)`、`n + 1 / 2 + π`，没有三个命令名。故该最小块不支持 undefined、展开中途失败或 `\DeclareRobustCommand` 定义链缺口的判断。

## PDF 原始代码核对

以 PyMuPDF 对新重排 `/tmp/doc-tests/r20/t2/testmath.pdf` 和 GT `/tmp/doc-tests/t2/gt/testmath.pdf` 全文提取 `\\[A-Za-z@]+`，结果如下。页码不同，比较的是整篇次数：

| 指标 | NTex | GT |
|---|---:|---:|
| 页数 | 39 | 41 |
| 反斜杠控制词总数 | 1021 | 1033 |
| `\pmod` / `\genfrac` / `\boldsymbol` | 2 / 7 / 8 | 2 / 7 / 8 |
| `\cfrac` / `\dfrac` / `\tfrac` / `\substack` | 8 / 8 / 1 / 5 | 8 / 8 / 1 / 5 |
| `\begin` / `\end` | 47 / 47 | 47 / 47 |

原 PDF 的 p17–p35 抽样出现原始文本，不能推导为漏展开；GT 本来就印出同类文本。逐行文本集合有 28 条 NTex 独有、35 条 GT 独有，主要受换行和字形抽取影响，不能作为公式等价证明。该统计只证伪本刀的「命令名泄漏约 180 处」指标，不判定所有公式版面正确。

## 验证原始输出与剩余卡点

- `CARGO_TARGET_DIR=/tmp/r20-target cargo test -p ntex-core -p ntex-layout`：退出码 0；core **575 passed / 2 ignored**，layout **325 passed / 3 ignored**，其余测试组全绿。原始输出 `/tmp/bladeAJ/r20-core-layout-tests.out`，退出码 `/tmp/bladeAJ/r20-core-layout-tests.exit`。
- testmath 两遍原样重排：第二遍 **39 页/9 错**（`Missing { inserted` ×4、`missing \item` ×5），与本刀起点一致，并非用户任务所称 0 错。原始输出 `/tmp/doc-tests/r20/t2/pass{1,2}.out`；PDF `/tmp/doc-tests/r20/t2/testmath.pdf`。
- fntguide **50 页/0 错**、Transformer **53 页/0 错**、amsldoc **45 页/18 实质错**、TRIP 构造 **177B**、tripos **12 页/750B/0 错**、宏包 **13/13**：这些是同代码版本在上一刀的原始记录，见 `/tmp/bladeAJ/docfix2-report.md` 及其中列出的输出路径；本刀无引擎改动，未重复执行。用户给出的 tripos 614B 与当前基线 750B 不一致。
- Claude 短简报尝试一次：`claude -p --model gpt-6-sol` 返回 API 400「模型不存在」，非超时，未取得 review。简报与原始输出 `/tmp/bladeAJ/r20-review-brief.md`、`r20-claude-review.{out,err,exit}`。

**WIP 裁决：**本刀没有可归责的高级命令原文泄漏，因此不改 `\newcommand` / `\DeclareRobustCommand`，也不逐个硬补命令。正常展开路径的 undefined cs 已报 `Undefined control sequence`；`docs/KNOWN-SIMPLIFICATIONS.md` 已登记构串阶段静默跳过的独立技术债，本刀未发现其导致上述 PDF 文本。未新增声称能锁住不存在故障的回归测试。要解决真实问题，应单独对照 testmath 的九条错误和 39/41 页差，给出可证伪的首现场与 GT 差分。
