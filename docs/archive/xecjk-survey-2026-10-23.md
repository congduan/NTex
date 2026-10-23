# xeCJK 首错勘察（2026-10-23）

## 结论

`origin/main` 与开工 HEAD 均为 `2ec4500`，工作树干净；按指定 `CARGO_TARGET_DIR=/tmp/biblatex-target` 重建 `ntex-dvi`。ctex 的 DVI 分支改动没有治愈 xeCJK。原样 `article` + `xeCJK` 中文探针在 `--fmt assets/fmt/latex.fmt --auto-pkg --interaction nonstopmode` 下停在 l3 消息包装循环；35 秒有界运行超时，无 DVI。旧 WASM 审计所见 `File ended while scanning use of \__iow_wrap_next_line:w` 可用不加载 xeCJK 的 8 行探针独立重现。

**首个确定的上游门槛在 `assets/tex-minimal/tex/xelatex/xecjk/xeCJK.sty:44`**：`\sys_if_engine_xetex:F` 触发 `Require-XeTeX` critical 消息。它在 `ctexhook`、`ctexpatch`、`xtemplate` 等依赖加载之前；不是 fd/xunicode/l3keys 依赖链首错。第 37–43 行定义消息。真 pdfTeX 运行缩减探针正常给出 `Critical Package xeCJK Error: The xeCJK package requires XeTeX to function.`；NTex 本应先呈现这条能力边界，却在格式化该消息时转为内部扫描报错。`\XeTeXversion`/`\XeTeXrevision` 第 405 行及 `\XeTeXcharclass` 第 409 行又证实继续载入需要 XeTeX 原语族；不能靠 ctex 的 `\pdfoutput=0` 路线使其可用。归入 XeTeX 系能力边界（fontspec/unicode-math 同组），消息机诊断缺口另留后刀。

## 最小二分和锚点

| 输入 | 当前 NTex 结果 | 解释 |
|---|---|---|
| `fixtures/repro/xecjk-minimal.tex` | 35 秒有界运行超时，无 DVI；看门狗在约 46k–51k 步反复记录 `\__iow_wrap_next_line:w`、`\__iow_wrap_store_do:n`、`\__kernel_tl_set:Nx` | 原样用户路径仍未通过。CLI 将转录缓冲到结束，超时转录只有看门狗快照，不能把它当成完整首错日志。 |
| `fixtures/repro/xecjk-critical-wrap.tex` | exit 1，`Runaway argument?` + `! File ended while scanning use of \__iow_wrap_next_line:w.`，无 DVI | 只取 xeCJK 第 37–44 行的消息形状；没有包依赖、中文文本、XeTeX 原语调用。精确复现旧审计签名。 |
| `fixtures/repro/xecjk-direct-wrap.tex` | exit 0，0 错，1 页 DVI | 直接 `\iow_wrap:nnnN` 长文本写日志可结束，故不能笼统定性为所有长文本包装或换行 catcode 都失败。 |
| 短 critical 消息与短消息加 `\` 的临时探针 | 均正常显示 `Critical Package x Error` | 错误依赖 xeCJK 消息的组合形状；单独 critical 或换行标记不足以触发。 |

`expl3-code.tex:10920–10938` 是 critical 消息的 `\__msg_interrupt_wrap:nnn` 调用链；`12251–12298` 是 `\iow_wrap:nnnN` 的文本转换；`12434–12449` 定义被报告的 `\__iow_wrap_next_line:w #1#2 \s__iow_stop`。引擎错误出口在 `crates/ntex-core/src/expand/macros.rs:391–403`，定界实参收集于同文件约 365–480 行。**当前证据只证明宏展开/包装过程导致终止符未被扫描到，随后参数扫描在 EOF 报错；不能据 EOF 字样断言扫描器的定界符比较或换行 catcode 是根因。** 下刀应在 `\__iow_wrap_line_end:nw` → `\__iow_wrap_next_line:w` 前后追踪 `\s__iow_stop`、`\__iow_wrap_end_chunk:w` 的 token 身份和展开顺序，再与真 pdfTeX 比对。旧 l3 载入期 `^^J` catcode 问题已在 `docs/archive/expl3-lvt-scoreboard.md` 第五刀修过，不应直接套用旧结论。

## 一键复现与范围

```bash
scripts/xecjk-survey.sh /tmp/xecjk-survey-run
```

脚本重建当前 HEAD `ntex-dvi`，用指定 fmt/auto-pkg/nonstopmode 跑三份入库探针；`NTEX_MAX_STEPS=52000` 和 `timeout` 防止消息包装循环长挂。实测摘要：原样 `exit=124, errors=0, dvi=no`（0 是**被超时截断的日志**，不是成功）；缩减消息 `exit=1, errors=1, dvi=no`；直接包装 `exit=0, errors=0, dvi=yes`。原始日志在 `/tmp/xecjk-survey-run/`。没有改引擎，因此本刀只用既有短成功样例和三探针对比守护勘察结论，未重跑全基线。
