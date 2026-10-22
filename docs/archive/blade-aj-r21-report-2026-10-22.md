# 刀 AJ R21：testmath 九错收口与 `\error` 快照冲突（WIP）

日期：2026-10-22。修复提交：`e02c7e1`（显示公式后 `\par`）、`9d2ab93`（数学重音字段）、`8f46b2d`（发行 fmt 重建）。官方三篇源码逐字节复制到 `/tmp/bladeAJ/r21/t{1,2,3}/`，每遍 `timeout 300`、`--auto-pkg`。本刀没有修改官方原文；stash 未动。

## 根因与最小对拍

1. **T2 `missing \item` ×5**：五条分别在 testmath L610、616、645、672、719 的 `cor`/`defn`/`proof` 结束处。单独的定理或 proof 无错；`cor` 内有展示公式、紧接 proof 的最小片段稳定重现。GT 在 proof 内容结束时 `\@newlist=F`，NTex 旧行为为 `T`。显示公式的 `after_display` 跨显式 `\par` 留到下一个环境，抑制了 `\everypar`，amsthm 的 deferred heading 未消费。垂直模式显式 `\par` 清除续排标志后，两侧均为 `F`；回归 `explicit_par_after_display_restarts_everypar` 验证触发两次。
2. **T2 `Missing { inserted` ×4**：前两页的 `\wh X=\{\hat x_1,\dots\}`、`\wh Y=\{\hat y_1,\dots\}` 等同形公式。`$\wh X$` 与单独 `$\hat x$` 均无错，`$\wh X=\hat x$` 报一条；GT 同式零错。布局层 `accent_pending` 仅在末原子已经是 `Accent` 时消费，首个无花括号 nucleus 后仍悬挂，遇下一个重音原语误报。现首个原子立即装入 `Accent.nucleus` 并清 pending；回归 `consecutive_unbraced_math_accents_do_not_leave_pending_field`。
   该修复只覆盖字段生命周期：既有 `\mathaccent` 重音编号未传到布局、字形未绘制的问题在 `docs/KNOWN-SIMPLIFICATIONS.md` 登记，T2 零错不等于重音外观与 GT 等价。
3. **T1 `\error`**：`README.md` 的生成命令经 `ntex-dvi --generate-fmt` 读取 `assets/fmt/ltxinit.tex`，载入 latex.ltx 后 `\dump`。旧快照保留已移出 builtins 的 `\error` 槽；重 dump 后探针原始输出 `R21-ERROR-UNDEFINED`，amsldoc.cls 的 `\newenvironment{error}` 不再冲突。新旧快照头均为 `NTEXFMT1`、版本 **24**；仅内容变化，序列化 schema 未动，不 bump VERSION。debug 生成在 300 秒超时（未覆盖发行文件）；release 生成完成，原始末行：

   ```text
   已写出 /tmp/bladeAJ/latex.fmt（9096774 字节，格式快照）
   ```

   新文件与入库 `assets/fmt/latex.fmt` 逐字节相同。旧大小 9019208B。原始生成记录 `/tmp/bladeAJ/r21-fmt-release.out`，debug 超时记录 `/tmp/bladeAJ/r21-fmt.out`。

## 官方文档两遍与 T1 快诊

| 文档 | R20 起点 | R21 第二遍 | GT | PDF |
|---|---:|---:|---:|---|
| T1 amsldoc | 45 页 / 18 实质错 | **44 页 / 2 错** | 44 页 / 0 错 | `/tmp/bladeAJ/r21/t1/amsldoc.pdf` |
| T2 testmath | 39 页 / 9 错 | **39 页 / 0 错** | 41 页 / 0 错 | `/tmp/bladeAJ/r21/t2/testmath.pdf` |
| T3 fntguide | 50 页 / 0 错 | **50 页 / 0 错** | 46 页 / 0 错 | `/tmp/bladeAJ/r21/t3/fntguide.pdf` |

三份 PDF 的 `dvipdfmx` 均退出 0。T1 两遍间执行 TinyTeX `makeindex amsldoc.idx`；GT 对照取 `/tmp/doc-tests/t1/gt-indexed/`、`t2/gt/`、`t3/gt/`。原始转录、索引日志和 PDF 转换输出均在各 `tN/` 目录。第二遍关键原文：

```text
! Undefined control sequence.
\macc@nucleus
l.1293 $\hat{\hat{A}}$ (\cn{hat}|{\hat{A}}|).
! Multicol Error.
l.462 \end{theindex}
已写出 amsldoc.dvi（291603 字节，44 页，80 字体）
已写出 testmath.dvi（104410 字节，39 页，77 字体）
已写出 fntguide.dvi（147882 字节，50 页，116 字体）
```

T1 原 17 条后续错快诊：首错 `\macc@nucleus` 是 amsmath `\mathaccentV` 双重重音链，GT 同位零错，仍属真实数学兼容问题。旧的 `Argument of \split has an extra }`、`\begin{split} ended by \end{error}`、`Misplaced alignment tab &`、`Extra \right`、`\pmatrix`/`\cr`/`\over` 在官方文档的错误示例章内，`error` 环境未建立导致示例正文被当作 TeX 执行；重建 fmt 后整组消失。新露头 `Multicol Error` 出自 `multicol.sty` 的 `\page@sofar` 在 `\ifvmode` 为假时主动 `\errmessage`，触发于索引收尾，GT 没有；归为输出例程/模式交界。两条均为真文档同源残差，不按 TRIP 特有搁置。

## 回归原始输出

`CARGO_TARGET_DIR=/tmp/r21-target cargo test -p ntex-core -p ntex-layout`（完整原始输出 `/tmp/bladeAJ/r21-core-layout-tests.out`）：

```text
test result: ok. 577 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 24.30s
test result: ok. 327 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 30.72s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.17s
test result: ok. 4 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 14.09s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

指定真实 Transformer 基线是 `/tmp/real-doc/technical-report.tex`（113881B），不是仓库 `samples/transformer-real/transformer-standalone.tex`（48043B）；用前者两遍编译，原始末行：

```text
已写出 technical-report.dvi（151851 字节，53 页，87 字体）
```

第二遍 `^!` 为零，见 `/tmp/bladeAJ/r21/transformer-pass{1,2}.out`。误跑 48043B standalone 的新旧 fmt 均为 17 页且缺图，属不同输入口径，不纳入 53 页比较。

TRIP 构造与 tripos 原始末行（`/tmp/bladeAJ/r21/trip/{build,run}/`）：

```text
已写出 trip.fmt（2247028 字节，格式快照）
已写出 trip.dvi（177 字节，1 页，5 字体）
已写出 tripos.dvi（750 字节，12 页，5 字体）
```

宏包矩阵命令：`NTEX=target/debug/ntex-dvi /home/ubuntu/.venvs/pixtools/bin/python scripts/pkg-regression.py --no-report --work /tmp/bladeAJ/r21/pkg-work`。原始 `/tmp/bladeAJ/r21/pkg.out` 末行：

```text
PASS 13/13（判定口径：ntex-dvi 写出 DVI 且 0 错）
负例 footmisc PASS（报错含包名+反查指引=True，未产 DVI=True）
```

`docs/pkg-regression-baseline.md` 的既有时间戳改动在本刀开工前已由 `8c49816` 提交，工作区无待收的独立文档改动。Claude 短简报按任务只试一次，`timeout 45` 退出 **124**、stdout 0B；stderr 为 `[claude-code:unrecognized_model] {"model":"glm-5.3-flash","query_source":"generate_session_title"}`。简报 `/tmp/bladeAJ/r21-review-brief.md`、输出 `/tmp/bladeAJ/r21-claude-review.{out,err}`；未声称外部评审通过，以实测和源码对拍为验证依据。

**状态：WIP。** T2 九错清零；T1 仍 2 错，T2/T3 页数分别差 −2/+4，不能称官方三文档完整 closure。后续按上述两个 T1 首现场和页数差继续定位。
