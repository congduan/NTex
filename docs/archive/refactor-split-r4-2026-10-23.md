# R4 文件拆分战报（2026-10-23）

基线 `49dc935`；`git fetch origin && git log --oneline -3 origin/main` 确认远端仍为 `49dc935`、`4482097`、`761ad90`。开工前工作区只有 `docs/pkg-regression-baseline.md` 的既有改动；本刀未修改该文件、`input.rs` 或 stash。

## 分步提交

| 提交 | 纯移动范围（原文件行号） | 独立核查 |
|---|---|---|
| `dd2e62c` | `typeset/sink.rs` 1054–1118 → `sink/font.rs`，1119–1530 → `sink/math.rs`，1531–1951 → `sink/boxes.rs`，1952–2044 → `sink/align.rs`，2045–2206 → `sink/page.rs`，2207–2283 → `sink/io.rs`；1–1053 与 2284–2578 留在 `sink.rs` | core 581、layout 334；完整输出 `/tmp/ntex-r4-sink-test.out`；clippy `/tmp/ntex-r4-sink-clippy.out` |
| `6f6ab98` | `expand/mod.rs` 1791–2265 → `diagnostics.rs`，2508–3399 → `dispatch.rs`，3401–3793 → `stack_diag.rs`，3794–4268 → `token_input.rs`；公共类型、状态定义与主循环仍在 `mod.rs` | core 581、layout 334；完整输出 `/tmp/ntex-r4-core-test-final.out`；clippy `/tmp/ntex-r4-core-clippy.out` |

跨模块调用的方法仅增添 `pub(super)` 可见性，未重命名，也未改处理语句。`typeset` 原先用 `include!("sink.rs")`，现以 `pub mod sink` 引入相同的 trait 实现；原有排版调用路径继续通过 `NodeBuilder` 与 trait 生效。

## 全量门禁

基线 release 二进制 `/tmp/ntex-r4-baseline-dvi` 在编辑前从 `49dc935` 构建并复制；拆分后 release 二进制 `/tmp/refactor-target/release/ntex-dvi` 的构建日志在 `/tmp/ntex-r4-release-build.out`。文档载入 timeout 为 900 秒，错误数从 stdout/stderr 合流的 `^! ` 行统计。完整原始日志在下表所列路径，以下保留判定用的原始输出行。

| 门禁 | 原始输出路径 | 判定 |
|---|---|---|
| `cargo test -p ntex-core -p ntex-layout` | `/tmp/ntex-r4-core-test-final.out` | core 581、layout 334，全绿且未减少；集成测试也全绿 |
| `cargo clippy -p ntex-core -p ntex-layout --all-targets -- -D warnings` | `/tmp/ntex-r4-final-clippy.out` | 退出码 0 |
| `cargo run --release -p ntex-trip -- --test trip` | `/tmp/ntex-r4-trip.out` | 退出码 0，但当前驱动为 stub，**报告跳过**；不能宣称该命令实跑 TRIP |
| TRIP 构造、tripos | `/tmp/ntex-r4-gates/after-trip-build.out`、`/tmp/ntex-r4-gates/after-tripos.out`、`/tmp/ntex-r4-gates-verify.out` | 直接调用 release `ntex-dvi` 构造 177B；tripos 750B、12 页、0 错 |
| 宏包矩阵 `scripts/pkg-regression.py --no-report` | `/tmp/ntex-r4-pkg.out`、`/tmp/ntex-r4-pkg.json` | 13/13；footmisc 负例 PASS；`--no-report` 保住既有未提交文档 |
| T1 amsldoc 两遍 | `/tmp/ntex-r4-gates/t1diff-after-pass1.out`、`t1diff-after-pass2.out`、`/tmp/ntex-r4-t1-diff.out` | 44 页、1 错，唯一错误 `Multicol Error` |
| T2 testmath 两遍 | `/tmp/ntex-r4-gates/after-t2-pass1.out`、`after-t2-pass2.out`、`/tmp/ntex-r4-gates-verify.out` | 40 页、0 错 |
| T3 fntguide 两遍 | `/tmp/ntex-r4-gates/after-t3-pass1.out`、`after-t3-pass2.out`、`/tmp/ntex-r4-gates-verify.out` | 49 页、0 错 |
| 基线/拆分后 SHA-256 | `/tmp/ntex-r4-gates-verify.out`、`/tmp/ntex-r4-t1-diff.out`、两侧 `.dvi` | tripos 与 T1 逐份相等 |

测试与 clippy 原始输出：

```text
test result: ok. 581 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
test result: ok. 334 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.95s
```

TRIP 指定命令原始输出及独立构造输出：

```text
Running `target/release/ntex-trip --test trip`
TRIP：跳过（驱动 stub）：驱动 stub 未实现
已写出 trip.dvi（177 字节，1 页，5 字体）
```

tripos、T1/T2/T3、宏包矩阵的原始摘要：

```text
after-tripos {"exit": 0, "summary": "已写出 tripos.dvi（750 字节，12 页，5 字体）", "errors": 0, "error_types": []}
t1diff-after-pass2 {"exit": 0, "summary": "已写出 /tmp/ntex-r4-gates/t1-differential/amsldoc.dvi（295729 字节，44 页，80 字体）", "errors": 1, "error_types": ["! Multicol Error."]}
after-t2-pass2 {"exit": 0, "summary": "已写出 /tmp/ntex-r4-gates/t2/testmath.dvi（114123 字节，40 页，81 字体）", "errors": 0, "error_types": []}
after-t3-pass2 {"exit": 0, "summary": "已写出 /tmp/ntex-r4-gates/t3/fntguide.dvi（148526 字节，49 页，116 字体）", "errors": 0, "error_types": []}
PASS 13/13（判定口径：ntex-dvi 写出 DVI 且 0 错）
负例 footmisc PASS（报错含包名+反查指引=True，未产 DVI=True）
```

## 零语义变更对照

tripos 两侧在 `/tmp/ntex-r4-gates/trip/` 先后运行；T1 两侧在同一路径 `/tmp/ntex-r4-gates/t1-differential/` 各跑两遍并在各自第一遍后运行 `makeindex`，两侧开始前均清空工作目录生成物以保持相同辅助文件初态。对照脚本 `/tmp/ntex-r4-t1-diff.py`；原始输出：

```text
baseline-tripos SHA256 574045fbe51f4ec48f8de7878d5bd559e5a301b3ce5f5a5afe347222ffe9c150
after-tripos SHA256 574045fbe51f4ec48f8de7878d5bd559e5a301b3ce5f5a5afe347222ffe9c150
baseline SHA256 b8113ad5554e75e2b4c5281b965d6a8aa04e31472272bec7856962423990cabe
after SHA256 b8113ad5554e75e2b4c5281b965d6a8aa04e31472272bec7856962423990cabe
COMPARE EQUAL
```

首次 T1 对照未重置 `.aux/.ind`，后跑侧继承先跑侧生成物，得到 296048B 对 295729B 的假差异；保留原始记录 `/tmp/ntex-r4-gates-verify.out`。复位同一路径的生成物后，两侧两遍均为 295729B、44 页、1 条相同的 `Multicol Error`，SHA-256 相等。`\toks` tracing 已在基线之前的 `761ad90` 锁定，两个二进制都包含该行为。
