# LaTeX 官方三文档验收刀续：数学链三签名与终验

日期：2026-10-21。修复提交：`777ef95`（`split`）、`97e8f0e`（limits）、`a276f8e`（mu 胶）。三份官方源文件保持原样；终验在 `/tmp/doc-tests/closure/t{1,2,3}/` 使用逐字节副本、`assets/fmt/latex.fmt`、`--auto-pkg`、每遍 `timeout 300`。T1 两遍间用 TinyTeX `makeindex` 构建索引。原 GT 数字取上一批 `/tmp/bladeAJ/doctest-report.md`。

## 根因与回归锁

1. `split`：amsmath `split` 检查 `\if@display` 与 `\ifinner`，而 `\if@display` 由 `\everydisplay` 设置。VM 已保存该 token 列表却没有在进入显示数学时执行；按 tex.web `init_math` 分别选择 `\everydisplay` 和 `\everymath`。最小 `equation`/`split` 探针在修复前报错，修复后零错；`display_math_runs_everydisplay_before_equation_body` 通过。T2 原 `split` ×11 → 0。
2. limits：`\sum@`/`\prod@`/`\intop` 的 `\mathchardef` 值正确（如 `\sum@=\mathchar"1350`），但嵌套数学返回后 VM `in_math` 可能与排版层数学列表失步，按文本字符交付，产生 Ord noad。派发改问真实模式，收数学 shift 后同步 VM 标志；`mathchardef_uses_actual_math_mode_after_nested_shift` 通过。T2 `split` 修复后显露的 limits ×52 → 0（最初基线 ×34）。
3. mu 胶：24 次均出自 amsmath `\colon` 的合法 `\mkern-\thinmuskip`，并非 gather 垂直列表。`scan_dimen_mu` 读取 mu 参数时不应报 pt/mu 混用；pt 尺寸 `\kern-\thinmuskip` 仍应报。pdfTeX 最小探针只报后者 1 次，新增 `mkern_muskip_param_keeps_mu_units` 通过。T2 ×24 → 0。

## 官方文档终验

错误按转录中 `!` 开头行统计；T1 信息行 `! OK.` 单列扣除。上一批基线为三签名之前的 34/35/48 页、64/134/55 实质错误。当前三篇均两遍重排、DVI 与 PDF 生成，GT 仍用上一批有效两遍索引对照。

| 文档 | 上批 NTex 页/错 | 本次 NTex 页/实质错 | GT 页/错 | 仍存的错误 |
|---|---:|---:|---:|---|
| T1 amsldoc | 34 / 64 | **45 / 18**（另 `! OK.` ×15） | 44 / 0 | `\error` 名冲突、矩阵/对齐等；三数学签名均 0 |
| T2 testmath | 35 / 134 | **39 / 9** | 41 / 0 | `missing \item` ×5、`Missing { inserted` ×4；三数学签名均 0 |
| T3 fntguide | 48 / 55 | **50 / 0** | 46 / 0 | 无；此实测与任务预估“55 不动”不同 |

T1 PDF `/tmp/doc-tests/closure/t1/amsldoc.pdf`；T2 PDF `/tmp/doc-tests/closure/t2/testmath.pdf`；T3 PDF `/tmp/doc-tests/closure/t3/fntguide.pdf`。详细 NTex 转录分别为相邻的 `pass2.out`；T1 `makeindex.out`、各目录 `dvipdfmx.out` 留存。T1 dvipdfmx 仍警告部分 hyperref special，不能视为 GT 等价链接。

## 门禁与未收口项

- `CARGO_TARGET_DIR=/tmp/docfix2-target cargo test -p ntex-core -p ntex-layout`：core **575 passed/2 ignored**，layout **325 passed/3 ignored**；其余跨 crate 测试组通过。原始输出 `/tmp/bladeAJ/docfix2-full-tests.out`。
- TRIP 构造 `trip.tex --no-plain --dump trip.fmt`：**177B/1 页**；tripos：**12 页/750B/0 错**。任务要求 614B；从 a933fe6 独立工作树重建的基线二进制以同源、同 fmt、同工作目录实测也为 750B，本轮无新增字节回退。证据 `/tmp/bladeAJ/docfix2-trip/{construct,tripos,tripos-a933}.out`。
- Transformer 原文：**53 页/0 错**，任务要求 42 页；a933fe6 独立重建基线同命令也为 53 页/0 错。`plan.md` 既有 R19 记录是 42→53 页，故任务门槛沿用了 R19 以前数字。证据 `/tmp/bladeAJ/docfix2-transformer{,-a933}.out`。
- `/home/ubuntu/.venvs/pixtools/bin/python scripts/pkg-regression.py --no-report --work /tmp/bladeAJ/docfix2-pkg-work`（`NTEX=/tmp/docfix2-target/debug/ntex-dvi`）：**13/13 PASS**，见 `/tmp/bladeAJ/docfix2-pkg.out`。
- Claude 短简报 review 试一次，`timeout 45` 退出 **124**、stdout 0B；stderr 只有 `unrecognized_model glm-5.3-flash` 的 CLI 元数据异常。简报 `/tmp/bladeAJ/docfix2-review-brief.md`，输出 `/tmp/bladeAJ/docfix2-claude-review.{out,err}`；以三枚定向锁、全量测试、原文重排和宏包矩阵作为本轮验证依据，未声称外部 review 已通过。

**终验状态：WIP。** 三个指定数学签名已归零并各自提交；官方三文档仍未达到 GT 零错及页数一致，且任务中 tripos 614B / Transformer 42 页门槛与当前起点不一致。后续先定位 T1 的 `\error` 格式快照名冲突与剩余对齐错误、T2 的九条剩余错误；页数差需继续按盒树/断页对照，不以零数学错误代替视觉等价。
