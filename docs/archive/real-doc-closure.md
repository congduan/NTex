# 真实文档验收收尾（2026-10-21）

起点 `7017b61`。源文件 `/tmp/real-doc/technical-report.tex` 保持原样（113881B）；`stash@{0}`–`stash@{3}` 未操作。工作区原有 `docs/pkg-regression-baseline.md` 改动由别的工作留下，本刀未改、未纳入提交。

## 两项缺口

1. **数学 limit_switch。** 从论文 L190–196 的 `align*` 抽出 `/tmp/real-doc/min-limits.tex`，并加入 `\mathop{X}\nolimits_1` 以隔离直接触发点。GT pdfLaTeX 零错一页；修复前 NTex 1 条 `Limit controls must follow a math operator`。根因在 `NodeBuilder::math_limit_switch` 只接受 `MathAtom::Char(Op)`；`\mathop{...}` 经字段组收口为 `MathAtom::Classed(Op)`，是合法的 `op_noad`。现在 Classed 保留 subtype 0/1/2，limit_switch 识别它，脚本布局按 displaylimits/limits/nolimits 选择堆叠或旁排。`classed_operator_accepts_limit_controls_and_keeps_their_subtype` 锁定合法/非法两边和实际布局；隔离 worktree 中只去掉 Classed Op 判定分支，定向测试实测 FAILED（合法 `\mathop{X}\limits_1^2` 由单个 VBox 退化为旁排两盒）；原始红灯 `/tmp/real-doc/red-test.out`，退出码 101。
2. **文末浮动体。** `/tmp/real-doc/min-float.tex` 为 `figure` 后直接 `\end{document}`。当前 HEAD 的 NTex 与 GT 均零错一页；本刀开工重新构建的整文档也没有 `Float(s) lost`，故它已随前刀的收尾链在现行源码中消失，无需再改分页代码。新增二进制级测试 `latex_float_before_end_document_is_not_lost`：真实 latex.fmt、DVI 一页、无错误、DVI 有浮动体文字。旧 33 页/5 错记录来自开工前二进制，不代表现行 `7017b61` 源码（其新构建基线为 42 页/2 错）。

## 真实文档原始验收输出

命令：`timeout 300 /tmp/rdf-target/debug/ntex-dvi technical-report.tex --fmt /home/ubuntu/NTex/assets/fmt/latex.fmt --auto-pkg --pkg-tlpdb /home/ubuntu/.TinyTeX/tlpkg/texlive.tlpdb --pkg-root /home/ubuntu/.TinyTeX`，在 `/tmp/real-doc` 连跑两趟；原始输出 `/tmp/real-doc/closure-pass{1,2}.out`，退出码均 0。错误行 `^!` 均为 0。

```text
已写出 technical-report.dvi（150850 字节，42 页，87 字体）
```

`dvipdfmx -o /tmp/real-doc/technical-report-ntex-final.pdf /tmp/real-doc/technical-report.dvi` 退出码 0；原始输出 `/tmp/real-doc/closure-dvipdfmx.out`：

```text
/tmp/real-doc/technical-report.dvi -> /tmp/real-doc/technical-report-ntex-final.pdf
[1][2][3][4][5][6][7][8][9][10][11][12][13][14][15][16][17][18][19][20][21][22][23][24][25][26][27][28][29][30][31][32][33][34][35][36][37][38][39][40][41][42]
408420 bytes written
```

无 image special 警告。**最终 PDF：`/tmp/real-doc/technical-report-ntex-final.pdf`。**GT `/tmp/real-doc/gt/technical-report.dvi` 为 54 页；GT PDF `/tmp/bladeAJ/technical-report-pdftex-gt.pdf`。

## 42 对 54 页归因

修复前的当前源码为 42 页/2 错，修复后 42 页/0 错：这两项错误**没有**造成页数差。GT 与 NTex PDF 均为 612×792pt，正文主字体 10.91pt，横向正文范围约 x=126–484pt；微型 `article[11pt]` 对照里两侧的 `\textheight` 与 `\vsize` 也同为 541.40024pt、`\baselineskip` 同为 13.6pt。差异集中在纵向分页/输出：GT 正文页文字底部常在 y≈660–672pt，NTex 多页延伸至 y≈780–799pt，明显进入 GT 的页脚以下区域。PDF 文本提取统计：NTex 有约 7509 个字符的 span 起点位于 y>720pt，GT 为 0；原始论文和增补章节直到参考文献均出现在两份 PDF 中。**归因：NTex 页面承载过多竖向材料，版心/断页链仍与 GT 不同；12 页差额不属于本刀 limit_switch 或文末 float 错误。**不改源文档或人为补页。后续需独立追踪页目标高度到 shipout 盒的纵向定位。

## 门禁

- `CARGO_TARGET_DIR=/tmp/rdf-target cargo test -p ntex-core -p ntex-layout`：core **571 passed**（2 ignored），layout **322 passed**（3 ignored），其余集成测试全绿；原始输出 `/tmp/real-doc/core-layout-tests.out`。
- `CARGO_TARGET_DIR=/tmp/rdf-target cargo test -p ntex-dvi latex_float_before_end_document_is_not_lost -- --nocapture`：1 passed；原始输出 `/tmp/real-doc/float-regression-test.out`。
- TRIP 构造：`trip.tex --no-plain --dump trip.fmt`，工作目录含 `trip.tfm`，`trip.dvi` **177B**；`tripos.tex --fmt /tmp/bladeAJ-r15/trip-ntex-r8.fmt`，无 `trip.tfm` 的运行目录，**12 页/614B/0 错**。原始输出 `/tmp/real-doc/trip-construct.out`、`/tmp/real-doc/tripos.out`。
- `scripts/pkg-regression.py --no-report`：**13/13 PASS**；原始输出 `/tmp/real-doc/pkg-regression.out`。其 `caption` 缺包负例仍 FAIL（开工前未提交基线已记录此状态），未纳入 13 包 PASS 分母。
- `git diff --check` 通过。`cargo fmt --check` 全仓仍失败于原有 `crates/ntex-dvi/tests/align_label.rs`、`float_clearpage.rs`、`crates/ntex-layout/src/page.rs` 格式差；本刀改过的文件已按 rustfmt 建议调整。原始输出 `/tmp/real-doc/fmt-check.out`。

## Claude review

短简报 `/tmp/real-doc/claude-brief.txt`（含变更 diff 与门禁数据）尝试一次 `timeout 300 claude -p --max-turns 1`。命令 1 退出，非超时；原始输出 `/tmp/real-doc/claude-review.out` 仅 127B：`[claude-code:unrecognized_model] {"model":"glm-5.3-flash","query_source":"generate_session_title"}` 与 `Error: Reached max turns (1)`，无审查结论。按实登记 Claude 通道缺席；本刀以 tex.web §1184 的 `type(tail)=op_noad` 条件、有效/无效双轨测试、去修复红灯及整文档/宏包/TRIP 门禁作为验证依据。
