# fp 模块载入探针（lvt 第三刀，2026-09-13）

双引擎对拍：NTex `target/debug/ntex-dvi <probe>`（转录到 stdout）vs
pdfTeX（TinyTeX，`~/.local/bin/pdftex -interaction=nonstopmode <probe>`
转录在 `<probe>.log`）。**NTex 侧须把 stdout 重定向到独立命名文件**
（NTex 转录走 stdout，pdfTeX 走 .log，别混）。

| 文件 | 靶 | pdfTeX 期望 |
|---|---|---|
| `probe-mathchar-neg.tex`（stub7）| `\mathchardef` 操作数取负（`-\c__fp_minus_min_exponent_int`）| 取负后同点 0 错 |
| `probe-noexpand-operand.tex`（stub9）| `\noexpand` cs 在 `\if_catcode:w`/`\if_meaning:w` 操作数位 | CC2/MM2 均 EQ（L9816-9838 归一）|
| `probe-pdfstrcmp-expand.tex`（stub12）| `\pdfstrcmp`（`\__fp_str_if_eq:nn`）实参组内展开 | V1 `1`（原义收集则 0）|
| `probe-fp.tex` + `exgeneric.tex` | 全载 expl3 + `\fp_const:Nn \c_e_fp`（fp 首错现场，l.18141）| 2 错（探针自身）/ FP-OK 1 |

`exgeneric.tex` 是 expl3 载入垫片（来源 = `scripts/lvt` harness 同款，
**已入库** `fixtures/l3kernel/`），`probe-fp.tex` 依赖同目录的
`expl3-code.tex`（基准件已入库 `fixtures/l3kernel/`，md5
7a1cc7249b9eeccb4029956317d7a295；跑前 `cp` 过来即可）。

第四刀（数字循环尾无条件展开，573→23）补丁正文与落地清单：
`docs/expl3-lvt-scoreboard.md` 第三刀节 + 附录 A。

**stub 探针不能单独跑**：它们用 `\iow_log:e`/`\__fp_*` 函数，须接在
expl3-code 截断版之后。生成法（N=7/9/12）：

```python
from pathlib import Path
gen = Path('probes/fp-load/exgeneric.tex').read_text()
code = Path('expl3-code.tex').read_text().splitlines(keepends=True)
inp = next(l for l in gen.splitlines() if 'expl3-code' in l)
Path(f't{N}.tex').write_text(
    gen.replace(inp, ''.join(code[:18140]) + Path(f'probes/fp-load/probe-<name>.tex').read_text()))
```

（截断到 l.18140 = fp 首错行 l.18141 之前，使 stub 成为首个 fp 调用。）
