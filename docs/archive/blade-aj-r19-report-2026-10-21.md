# 刀 AJ R19：纵向版心 text height 修复

日期 2026-10-21。起点 `6dd16ce`。未动 stash@{0}–{3}；原有 `docs/pkg-regression-baseline.md` 时间戳改动未纳入提交。所有 cargo 串行，统一 `CARGO_TARGET_DIR=/tmp/r19-target`。

## 先测与根因

最小 `article[11pt]` 双引擎探针 `/tmp/real-doc/dim-min.tex` 与真实文档插桩 `/tmp/bladeAJ/dim-probe.tex` 表明，pdfTeX 与 NTex 的 `\vsize=\textheight=\@colroom=541.40024pt` 完全一致；输出见 `/tmp/real-doc/dim-min.{gt,ntex}.out`、`/tmp/bladeAJ/dim-probe.{gt,ntex}.out`。LaTeX 赋值链不是根因。真实文档 `\tracingpages=1` 原始输出 `/tmp/bladeAJ/r19-trace.out` 中普通页 `goal height=541.40024`，另有 LaTeX 特殊输出阶段预期的 `16383.99998pt`。

无浮体探针 `/tmp/real-doc/r19-min.tex`（110 段）旧 NTex 10 页、GT 14 页，旧页底 y≈852pt。旧追踪 `/tmp/bladeAJ/r19-min-trace.out` 的 pagegoal 541.4pt、断页点 total≈536.6pt，却在出货盒中含约 54 行。根因是 `page.rs::process` 只匹配 `Node::Glue { name: None }`：`\baselineskip` 等带名胶水落在页表但进入 `_` 分支，不增加 `page_total`、不作为断点。修后所有正页高带名胶水参与计账，并原样保留来源名。

INITEX 在 `\vsize=0` 的 TRIP 格式构造期依赖旧行为：直接统一计账会使构造页 177B/1 页退化为 312B/4 页。最终改动只在零页高时保留旧行为，正页高一律正常计账。该例外已登记 `docs/KNOWN-SIMPLIFICATIONS.md`；pdfTeX `-ini trip.tex` 在 `\dump` 后没有 DVI 页，NTex 的 177B 是项目既有构造基线。

回归测试 `page::named_glue_tests::named_interline_glue_counts_toward_page_goal` 用 35sp 目标、10sp 行盒与 5sp 带名行间胶水，断言第一页最多容纳两行，锁定漏算路径。原始输出 `/tmp/bladeAJ/r19-targeted-test.out`。

## 最终验证与原始输出

- `CARGO_TARGET_DIR=/tmp/r19-target cargo test -p ntex-core -p ntex-layout`：core 571 passed、layout 324 passed、新增 1；集成组 4/10/4/3 全绿。`/tmp/bladeAJ/r19-core-layout-tests-final.out`。
- 最终 release build 成功：`/tmp/bladeAJ/r19-build-final.out`。
- 无浮体探针：NTex 10→14 页，与 GT 14 页一致；修后 PDF `/tmp/bladeAJ/r19-min-fixed.pdf` 页底最大 y=701.43pt。`/tmp/bladeAJ/r19-min-{gt,nt,fixed}.out`。
- 真实 113881B 原文 `/tmp/real-doc/technical-report.tex` 未改，两趟均为 151827B DVI、53 页、87 字体、0 条 `^!` 错误；GT 54 页。原始输出 `/tmp/bladeAJ/r19-real-final-pass{1,2}.out`、`/tmp/bladeAJ/r19-dvipdfmx-final.out`。最终 PDF `/tmp/bladeAJ/r19-technical-report-ntex.pdf`。
- PyMuPDF 逐页全部文字块底边：旧 PDF 42 页中 26 页 y>752pt（20 页 y>792pt），最大 801.75pt；新 PDF 53 页中 **0 页 y>752pt、0 页 y>792pt**，最大 701.43pt；GT 54 页最大 701.43pt。逐页原始表 `/tmp/bladeAJ/r19-page-heights.out`（最终 PDF 同值再次验证）。
- 公式 `/tmp/bladeAJ/r19-geometry-final.out`：GT/NTex 各 10 处，公式 x0=203.2pt、第二式 x0=325.8pt、编号右缘 x1=484.4pt；无 R18 回退。
- TRIP 构造 `/tmp/bladeAJ/r19-trip/build-final.out`：177B/1 页；tripos `/tmp/bladeAJ/r19-trip/tripos-final.out`：614B/12 页/0 错。工作目录与 R18 相同，tripos 使用 R8 快照且没有 trip.tfm。
- `NTEX=/tmp/r19-target/release/ntex-dvi python3.12 scripts/pkg-regression.py --no-report --work /tmp/bladeAJ/r19-pkg-work`：PASS 13/13；原始输出 `/tmp/bladeAJ/r19-pkg.out`。caption 负例的 FAIL 是既有输出口径，不在 13 包分母。
- `git diff --check` 通过。

## Claude review

`timeout 60 claude --model sonnet -p` 对 `/tmp/bladeAJ/r19-review.diff` 尝试一次，退出码 124，stdout/stderr 均 0B；`/tmp/bladeAJ/r19-claude-review.{out,err,exit}`。外部 review 缺席，未虚构审查结论；以上可复验输出作为验收依据。

## 结论

用户报告的页底正文裁切已消除；真实文档 53 页接近 GT 54 页。剩余 1 页差异尚未归因，不归入本刀验收硬指标。
