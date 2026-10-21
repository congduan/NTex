# 刀 AJ R18：equation display 几何修复

日期 2026-10-21。起点 `1b62e49`；预存改动 `docs/pkg-regression-baseline.md` 未触碰，stash@{0}–{3} 未触碰。构建统一用 `CARGO_TARGET_DIR=/tmp/r18-target`，cargo 串行。

## 复现与根因

独立 `article + amsmath + \begin{equation}\label{eq:probe} C(n,d)=n^2d+nd^2, \qquad M(n)=n^2+nd. \end{equation}` 与 pdfTeX 的公式 x0=203.2pt、编号 x1=484.4pt 已一致，说明错误依赖前文。完整文档旧 PDF `/tmp/real-doc/technical-report-ntex-final.pdf` 的相同公式 x0=16.9/14.2pt、编号 `(10)` x1=125.8pt；GT `/tmp/bladeAJ/technical-report-pdftex-gt.pdf` 为 x0=203.2pt、编号 x1=484.4pt。

定位插桩原始数据 `/tmp/bladeAJ/r18-probe/instrument-debug.out`：第一条 display `z=23592960sp (360pt)`，后续 display `z=0`，但公式内 `\message` 读 VM 参数为 `R18BEFORE=360.0pt/360.0pt/0.0pt`（hsize/displaywidth/displayindent）。VM 的 `displaywidth` 全局值未变，布局镜像在组尾恢复为 0；`expand/mod.rs` 入口旧守卫只在 VM 值变化时发 `param_changed`，后续 display 不再同步。tex.web init_math 的 `eq_word_define(display_width,l)` 每次进入 display 都执行。修复是每次进入 display 都给布局层同步值，原公式居中与 `\eqno` 右置装配保持原样。

新增 `displaywidth_stays_live_after_grouped_equation` 布局测试：`{ $$x\eqno(1)$$ } $$x\eqno(2)$$`，要求两条公式有正向居中偏移，编号右缘都等于 `displaywidth`。修复前红：第二盒 `shift=-2125sp`、`width=2125sp`，原始输出 `/tmp/bladeAJ/r18-red-test.out`；修复后绿 `/tmp/bladeAJ/r18-green-test.out`。

## 验证（原始输出）

- `CARGO_TARGET_DIR=/tmp/r18-target cargo test -p ntex-core -p ntex-layout`：`571 passed; 0 failed; 2 ignored`，`323 passed; 0 failed; 3 ignored`；其余集成组 4/10/4/3 passed，退出码 0。完整输出 `/tmp/bladeAJ/r18-core-layout-tests.out`。
- release 构建：`cargo build --release -p ntex-dvi`，退出码 0，输出 `/tmp/bladeAJ/r18-release-build.out`。
- 完整文档 `technical-report.tex` 两趟 `--fmt assets/fmt/latex.fmt --auto-pkg`：两趟原文均为 `已写出 technical-report.dvi（150832 字节，42 页，87 字体）`，`^!` 错误行 0，退出码均 0。输出 `/tmp/bladeAJ/r18-real-pass{1,2}.out`。基线旧文档 42 页/0 错，页数不变；GT 54 页仍是独立的纵向排版残差。
- `dvipdfmx` 退出码 0、42 页，原始输出 `/tmp/bladeAJ/r18-dvipdfmx.out`。**交付 PDF：`/tmp/bladeAJ/r18-technical-report-ntex.pdf`**。
- PyMuPDF 断言 `/tmp/bladeAJ/r18-geometry.py` 退出码 0，完整逐页输出 `/tmp/bladeAJ/r18-geometry.out`：NTex 与 GT 的 10 处同款公式均为首式 x0=203.2pt、第二式 x0=325.8pt；编号 `(4)`、`(5)`、`(10)` 右缘 x1=484.4pt。每处首式 x0>100pt，`\qquad` 后第二式与首式分隔 >10pt，编号与第二式分隔 >40pt。任务单给的“x1>500pt”被 GT 的 484.4pt 证伪；按 GT 实际右缘验收。
- 宏包 `scripts/pkg-regression.py`（以本刀 release 二进制覆盖 `NTEX`，`--no-report`，避免改写用户基线）`PASS 13/13`，退出码 0，原始输出 `/tmp/bladeAJ/r18-pkg.out`。脚本的 `caption` 负例显示 `FAIL` 是既有输出口径，不计入 13 包矩阵。
- TRIP 构造：`trip.tex --no-plain --dump`，`trip.dvi` **177B/1 页**，退出码 0，输出 `/tmp/bladeAJ/r18-trip/build.out`；R8 快照下 `tripos.tex --fmt`（工作目录无 trip.tfm）**614B/12 页/0 错**，退出码 0，输出 `/tmp/bladeAJ/r18-trip/tripos.out`。
- `git diff --check` 通过。`cargo fmt --all -- --check` 因仓库原有 `crates/ntex-dvi/tests/align_label.rs`、`float_clearpage.rs` 格式差异非零；本刀两处改动未出现在格式差异中，原始输出 `/tmp/bladeAJ/r18-fmt-check.out`。

## Claude review

`timeout 60 claude --model sonnet -p` 输入短简报，退出码 124，stdout/stderr 均 0B；原始文件 `/tmp/bladeAJ/r18-claude-review.{out,err,exit}`。外部审查缺席，未虚构结论。人工复核：入口条件仅在真正进入 display 时成立；赋值与事件值同为当前 `hsize`；布局测试锁定组恢复后同步，文档几何逐页与 GT 对齐，未见必修缺陷。

## 结论

用户报告的公式左溢、两式挤压、编号插中间均消除。42 页/0 错保持；剩余 42 vs GT 54 页差异不属于本刀公式水平几何。
