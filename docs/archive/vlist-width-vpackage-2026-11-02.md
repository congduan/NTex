# vlist 宽度口径对齐 tex.web vpackage——\moveleft 刀 review 必修收口

日期：2026-11-02 ｜ 基线 HEAD：15006da ｜ 分工：CodeBuddy/Codex 实施（WIP 交接）→ Codex gpt-6-sol R2 review → 主控独立复测收口

## 一句话

竖直列表宽度从「max 全部子节点 width」修正为 tex.web `vpackage`（L13187-13218）口径：
**只有 box（width+shift）/rule/leaders（取 leader_ptr 宽）贡献宽度，竖直 glue/kern 只推进自然高**；
该口径下沉为 `vlist_width()` 单一 helper，`vpack`/`\vsplit`/插入体/页盒装配全部收敛到它。

## 背景

`\moveleft/\moveright` 刀（上一轮，工作树 WIP 交接）第一轮 review 提出两阻塞：

1. **阻塞 1**：宽度规则只挂在显式 VBox/VCenter/VTop 三臂，共用 `vpack`（node.rs
   `vbox_dimensions`）仍走旧口径——`\vsplit`、插入体、竖直对齐装配经此路径宽度仍错。
2. **阻塞 2**：页盒直接装配（page.rs `package()`）与插入累积盒（paging.rs
   `insert_accumulate`）绕过 vpackage 口径；插入盒追加体子树后不重算宽度。

本轮 WIP 按简报（/tmp/blade-moveleft-fix-brief.md）落地两阻塞，主控复测后 Codex R2 review
五项定向核查全过、阻塞清单空，仅一处非阻塞注释行号修正（leaders 依据在 L13216-13218
glue 归并块内，非 L13205-13206 的 box/rule 分支），已随本刀修正。

## 改动面

| 文件 | 改动 |
|---|---|
| `crates/ntex-layout/src/node.rs` | 新增 `pub fn vlist_width(&[Node]) -> i64`（tex.web 单一宽度口径）；`vbox_dimensions` 的 width 改调它；模块文档同步；单测按 tex.web 口径更正 + 新增 helper 锁 |
| `crates/ntex-layout/src/page.rs` | `package()` 页盒宽度改走 `vlist_width`（tex.web fire_up：box255:=vpackage(...)——页盒与 vpack 同口径）；删除逐节点把 glue/kern width 计入页宽的旧逻辑；depth 归并原样保留 |
| `crates/ntex-layout/src/typeset/paging.rs` | `insert_accumulate` 追加体子树后补 `b.width = d.width`（同页多次 insert / 多插入族每次重算） |
| `crates/ntex-core/src/expand/primitive_math.rs` | `\moveleft/\moveright` sink 传正值（布局侧取负成对）；非法模式拦截扩到 hmode 全族（2\|3\|5\|6）按 tex.web 文案，报错即恢复不扫参数 |
| `crates/ntex-layout/src/typeset/sink/boxes.rs` | 注释校正（逻辑未动） |
| `crates/ntex-layout/src/typeset/tests.rs` | 回归锁：`vbox_move_shifts_affect_width_as_tex_vpackage` / `vsplit_width_uses_vlist_width_with_child_shift` / `page_box_width_ignores_vertical_glue` / `insert_accumulator_width_recomputed_from_appended_body` / `hbox_raise_uses_shift_for_height_and_depth` / `moveleft_in_restricted_horizontal_mode_reports_tex_error`（node.rs 另有 helper 单测 2 把 + 既有锁按 tex.web 口径更正 1 把） |

## GT 铁证（real tex 3.141592653，plain 通道）

```
\setbox0=\vbox{\hbox{aa}\moveleft 10pt \hbox{bbb}}
\setbox1=\vsplit0 to 10pt
\immediate\write16{W=[\the\wd1][\the\wd0]}
→ W=[10.00003pt][6.66672pt]
```

余量 = bbb 16.66672pt − 10pt = 6.66672pt → **子盒 shift 必须计宽**（不计宽会得 16.66672pt）。
补充区分探针（/tmp/blade-moveleft-fix/gt.log）：moveright 正偏移顶部 = aa+10pt=20.00003pt；
vbox 内 glue 不计宽（D=[16.66672pt]）。

## 验收（主控独立复测，全部亲跑）

| 项 | 结果 |
|---|---|
| `cargo test -p ntex-layout` | 353+4+10+4+3 全绿（0 failed） |
| `cargo test -p ntex-core` | 605 绿（基线持平） |
| clippy `-p ntex-core -p ntex-layout --all-targets` | 0 警告 |
| `cargo fmt --all --check` | 净 |
| TRIP：trip.tex `--no-plain --dump` | 177B / 1 页（基线一致） |
| TRIP：tripos.tex `--fmt trip.fmt` | 750B / 12 页（基线一致） |
| 宏包回归 `scripts/pkg-regression.py` | **13/13 PASS**（跑后 baseline 已 checkout） |
| Codex R2 review（5 项定向） | 全正确，阻塞清单空 |

## 教训

1. **「宽度口径」与「高度口径」在 vlist 里不同源**——tex.web vpackage 的 width 只认
   box/rule/leaders（glue 的 width 是竖直推进量不是横向占用），NTex 旧实现把两口径混在
   一次 max 里。同类混口径点已由 helper 下沉一次清空（vpack/split_vbox/页盒/插入体）。
2. **review 必修的「下沉」不能只修显式三臂**——第一轮把宽度规则挂在显式构造臂上，
   共用装配路径（vpack）漏网；review 的价值正在抓「同一语义的第二条代码路径」
   （刀AG「两条 split 路径」同型教训）。
3. **WIP 交接最优形态再证**：上一轮留 WIP + 简报（含 GT 探针源与已实锤事实），
   本轮执行方在其上落地两阻塞 + 6 把回归锁 + 完整报告，主控复测一轮过。

## 现场

- 简报 /tmp/blade-moveleft-fix-brief.md；执行方报告 /tmp/blade-moveleft-fix/report.md；
  GT 探针 /tmp/blade-moveleft-fix/{gt_exact.tex,gt.tex,gt_exact.log}；
  R2 review 输出 /tmp/blade-moveleft-review2-out.log。
