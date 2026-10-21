# 刀 AJ R17：TRIP 残差归因与同源修复

日期：2026-10-21。起点 `db10d95`，开工时工作区干净；只使用 `/tmp/aj-r17-target` 构建，未触碰 stash。

## 复测口径

`CARGO_TARGET_DIR=/tmp/aj-r17-target cargo build --release -p ntex-dvi` 从当前源码构建。20s watchdog：

- 格式构造：`trip.tex --no-plain --dump ...`，工作目录带 `trip.tfm` → 1 页、177B；
- 全本和 tripos：使用 R8 快照 `/tmp/bladeAJ-r15/trip-ntex-r8.fmt`，工作目录**不放** `trip.tfm` → 分别 9 页/1169B/13 Missing number、12 页/614B/0 错。带 TFM 会变成 2428B/750B，不能与 R16 字节基线混算。
- GT `fixtures/trip/trip.log`：16 页、6 Missing number。

开工源码与 GT 同位 4 条（trip.tex 253、257、298、419），NTex 独有 9 条（318、336×2、390、410、411、428、429×2），GT 独有 2 条（106、生成的 tripos 源行 2）；9−2=净多 7。逐条类型、实证与裁决见 plan.md「TRIP 残差归因（2026-10-21）」矩阵。L315 的 J 仍是第一个可见先行节点分叉：GT 普通字母，NTex 在故意错配的嵌套显示数学之后见未定义 active J。它属于后续盒子链差异的上游，不是合法 LaTeX 构造的独立反例。

## 同源项和改动

TRIP L410 的 `\advance\xspaceskip by-\xspaceskip` 在 GT 合法、NTex 多报 Missing number；L411 `\spacefactor2000` 被错位扫描又多报一条。仓库真实 LaTeX 文件 `assets/tex-minimal/tex/latex/base/nfssfont.tex:160` 使用 `\advance\xspaceskip by\fontdimen7\testfont`，因此归为同源必修。

最小修法：`param_kind_of(XSpaceSkip)` 映射到 `ParamKind::XSpaceSkip`，复用通用 `advance_param` 与 `scan_glue` 内部量路径；删除特殊增量分支，避免两份参数赋值逻辑；`\the\xspaceskip` 接入同一参数读回。新增 core 测试以非零宽度和伸长量验证组内 `\advance\xspaceskip by-\xspaceskip` 得 `0.0pt`、组外恢复 `10.0pt plus 2.0pt` 且无 Missing number；这两个值已用 pdfTeX 裸探针核对。去修复源码的独立探针 NTex 两条 Missing number、`X=0`，GT 零错、`X=0.0pt`；修复后 NTex 零错、`X=0.0pt`、`SF=2000`。

## 验证

- `cargo test -p ntex-core -p ntex-layout`：core 567 passed（2 ignored），layout 320 passed（3 ignored），集成测试全绿。新测试单跑通过。
- release `trip.tex --fmt ...`：9 页/1169B、Missing number 11（L410/411 消失）；`tripos.tex --fmt ...`：12 页/614B、0 错；构造阶段 1 页/177B。全本 20s watchdog 正常退出。
- 宏包回归 13/13 PASS：用原 `scripts/pkg-regression.py` 模块及 release 二进制路径覆盖执行，`--no-report` 避免覆盖基线文档；原始输出 `/tmp/bladeAJ/r17-pkg.out`。
- `small2e.tex` 1 页/0 错，`sample2e.tex` 2 页/0 错；`cargo fmt --check`、`git diff --check` 通过。
- 独立 `\vbox{\vskip2pt\kern1pt\penalty\lastkern}` 双引擎均零错，说明 L318 需依附 TRIP 前序畸形状态。

## 结论

修复后仍有 7 条 NTex 独有、2 条 GT 独有，净差 5；这些位于 TRIP 刻意制造的畸形数学、对齐、条件及 catcode/自写文件链。TRIP 专项关闭，16 页逐字对齐及 tripos 页内容差异转长期观测。真实 LaTeX 主线继续按 plan.md 推进。

## Review

Claude Code CLI 首次按环境默认模型调用，120s 超时、0B 正文（stderr 仅报 `glm-5.3-flash` 未识别）；第二次显式 `--model sonnet`，60s 超时、0B 正文。因此外部 Claude review 缺席，未虚构结论。主控人工复核 diff：`param_kind_of` 的 XSpaceSkip 映射只增加现有 `ParamKind::XSpaceSkip` 的读取入口；`advance_param` 在扫描增量后读取旧值，组作用域及 retaining 统一走 `assign_param`；`save.rs` 的 `\the` 值类为 Glue。组内/组外双引擎探针与全量门禁无回退，未发现阻断项。
