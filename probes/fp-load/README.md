# fp 模块载入探针（lvt 第三刀，2026-09-13）

双引擎对拍：NTex `target/debug/ntex-dvi <probe>`（转录到 stdout）vs
pdfTeX（TinyTeX，`~/.local/bin/pdftex -interaction=nonstopmode <probe>`
转录在 `<probe>.log`）。**NTex 侧须把 stdout 重定向到独立命名文件**
（NTex 转录走 stdout，pdfTeX 走 .log，别混）。

| 文件 | 靶 | pdfTeX 期望 |
|---|---|---|
| `probe-load.tex` | **纯载入**（exgeneric + expl3-code 全文，不带 fp 后缀）| 打印 `[LOAD-DONE]`（信号 = `\message`，**不是** `\typeout`：后者非原语、plain 无定义，两引擎实测均打不出）|
| `probe-chardef-scope.tex`（第八刀）| `\chardef` 作用域（`\global` 组内绑定不回滚/局部回滚）+ char_given 显示面（`\meaning` → `\char"7B`）+ `\if` 不透视 char_given + 无空格数字 `\the` 续数 | 六信号两引擎逐字一致（见文件头注释）|
| `probe-fontparam-global.tex`（第八刀，ia2）| 字体参数赋值恒全局（`\hyphenchar`/`\fontdimen`/`\skewchar` 不进 save stack）——l3intarray pdftex 回退分支的 intarray 模拟依赖此语义 | `[out:100][out-fd:14.0pt][skew:100]` 出组不回滚 |
| `probe-mathchar-neg.tex`（stub7）| `\mathchardef` 操作数取负（`-\c__fp_minus_min_exponent_int`）| 取负后同点 0 错 |
| `probe-noexpand-operand.tex`（stub9）| `\noexpand` cs 在 `\if_catcode:w`/`\if_meaning:w` 操作数位 | CC2/MM2 均 EQ（L9816-9838 归一）|
| `probe-pdfstrcmp-expand.tex`（stub12）| `\pdfstrcmp`（`\__fp_str_if_eq:nn`）实参组内展开 | V1 `1`（原义收集则 0）|
| `probe-fp.tex` + `exgeneric.tex` | 全载 expl3 + `\fp_const:Nn \c_e_fp`（fp 首错现场，l.18141）| 2 错（探针自身）/ FP-OK 1 |

**`probe-load.tex` 是「载入是否走通」的唯一口径**（`probe-fp.tex` 只在载入走通后
才轮得到 fp）：走通才打印 `[LOAD-DONE]`。**当前状态 = 走通且载入期错误已清至
残差**（第九刀口径修正，2026-09-16）：错误总数 3 → **1**
（探针首行加 `\let\_\relax` 消除 `\unhbox` 簇 2 条——GT 实证：那 2 条
pdfTeX 跑旧探针同样报（plain l.667 `\def\_{\leavevmode…}` 在数字位被
`\catcode` 链触发），是探针写法噪声而非引擎缺陷；修后 pdfTeX 0 错、
NTex 仅剩 `Forbidden ^^L` 根因#3 一条）。rc=0、DVI 落盘。
载入墙钟 4:58（第九刀 `0e0ea5d` O(N²) 根治后；第七刀 11:07），秒级
未达——剩余为解释器常量因子（~115ns/token vs pdfTeX 15-25ns，跨帧
重构另立项）。

<details><summary>历史：第七刀之前的状态（l.36005 爆栈墙，已于 6172da3 + 第七刀解除）</summary>

复测（2026-09-14 23:50，HEAD `b1cce11`，即补 `WordBreakProperty.txt` 之后）：

| 量 | 值 |
|---|---|
| 载入期错误 | **3 条**：expl3 语义错仅 **1 条**（`Forbidden control sequence … scanning definition of ^^L`，l.26865）；另 2 条是 **preload/hyphenation 段**的 `\unhbox` 噪声（发生在 expl3 之前，与 expl3 无关）。09-13 口径为 573 |
| pdfTeX | 改用 `\message` 后**打出 `[LOAD-DONE]`** ✅（旧 `\typeout` 版不可能打出，见下）|
| 载入终点 | **l.36005** —— UnicodeData 装载组收尾 `}`，紧邻 `\group_begin: \ior_open:Nn { CaseFolding.txt }`（l.36006）|
| 进度 | **89.4% 行**（36005 / 40266）、**90.0% 字节**（1248905 / 1387070）|
| 耗时 / 步数 | ~31s，末次 watchdog `steps=1160576`，`last_tok=\exp_after:wN` |
| 栈型 | `{Bytecode: 4978, MacroArg: 6, Source: 3, TokenList: 14}`，depth=5001 |
| 自旋环（`NTEX_CALL_TRACE`）| 6 段环 `\tl_if_eq:ccT → \exp_args:Ncc → \tl_if_eq:NNT → \use_none:n → \__int_step:Nw → \__int_map_1:w`（末 40 段全在此环内）|

**确定性**：终点 l.36005 在 5 次跑（含 1 次干净目录）中 **5/5 一致**；错误条数
4/5 为 3 条，首跑只录到 1 条 —— 差值恰为上表两条 preload 噪声，
**疑似转录通道丢块（仪器侧，待查；tooling-trust 纪律：先怀疑仪器）**。

**墙的主体** = `\__codepoint_finalize_blocks_aux:n`（expl3-code l.35939 起）的
`\int_step_inline:nn { \tl_use:c { l__codepoint_ #1 _block_tl } - 1 }` 块循环
——正是 `docs/expl3-lvt-scoreboard.md`「下一刀」标的 `\__int_step:Nw` 家族。
WordBreak 数据补齐后 wordbreak 段真实数据进入同一 finalize，墙的位置随之
前移到 CaseFolding 入口。完整现场与「载入还差哪几段」见该文档复测节。

**pdfTeX 路径**：本文档原写 `~/.local/bin/pdftex`（TinyTeX）——**本机无此文件**，
实际为 TeX Live 2024：`/Library/TeX/texbin/pdftex`（`preloaded format=pdftex`）。
注意该预载格式里 **`\typeout` 也未定义**，故对拍信号一律用 `\message`。

爆栈现场取法（诊断开关说明见 `docs/tooling-trust.md` §2.7）。**在临时目录跑**，
不要往库里拷 `expl3-code.tex`（未被 .gitignore 覆盖，会脏工作区）：

```bash
d=$(mktemp -d)
cp fixtures/l3kernel/expl3-code.tex probes/fp-load/{probe-load.tex,exgeneric.tex} "$d"/
# ① 只需入口链（最省）
(cd "$d" && NTEX_STACK_DUMP=1 NTEX_CALL_TRACE=40 \
   "$OLDPWD/target/debug/ntex-dvi" probe-load.tex 2>&1 | grep stack-dump)
# ② 要 token 级现场：栈顶 0 帧（全是循环体无信息）+ 栈底 45 帧（循环墙的起点在这里）
(cd "$d" && NTEX_STACK_DUMP=1 NTEX_STACK_DUMP_FRAMES=1 \
   NTEX_STACK_DUMP_TOP=0 NTEX_STACK_DUMP_BOTTOM=45 \
   "$OLDPWD/target/debug/ntex-dvi" probe-load.tex 2>&1 | grep '\[frame')
```

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

</details>
