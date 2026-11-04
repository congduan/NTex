# M5 阶段六第十刀：fontdimens/protrusions 迁 ValueState 精确比较 + Rc 写时复制

日期：2026-11-05。commit `4e111af`（实施）+ 本轮 review 必修（protrusions fmt 加载清空）。
管线：CodeBuddy/Claude 实施（Co-Authored-By Claude Code）→ 主控独立复测 → Codex gpt-6-sol
独立 review（需先修 1 项）→ 主控落码修复 → 全门禁复测。

## 动机与实测

检查点捕获剩余大表成本：expl3 intarray 场景实测 34931 条 fontdimens 时
`capture` 93.9µs/段，其中 `runtime_digest` 逐项哈希 79.6µs（占 85%）、
`FontDimens::clone` 深拷贝 6.4µs。post 捕获的最后一座大表成本山头。

## 实施（4e111af）

- `FontDimens` / `ProtrusionCodes` 内部 `Rc<Inner>` 化（第七刀 eqtb 槽向量、
  第一刀 InsertRegs 同形态）：写路径（insert/clear/set）必经 `Rc::make_mut`，
  `replace_from` 共享分配，捕获 = 引用计数（实测大表 capture 93.9→3.4µs，27.9×，
  与空表打平）；
- 两表迁入 `ValueState` 精确比较，`runtime_digest` 删 fontdimens 项；`PartialEq`
  走 `Rc::ptr_eq` 快路径（未触表段 O(1) 判等，实测 188ns）+ 深比较兜底（内容
  等价仍判等，无假失配）——拒绝「指针身份进 digest」路线：u64 指纹不持引用，
  地址复用制造假相等；「指针相同 ⇒ 内容必同」铁律由一切写路径过 make_mut 保证，
  头注释含完整健全性论证；
- 顺手收口预存正确性洞：protrusions 此前**既不进 digest 也不进 ValueState**
  （比较面盲区——只改 `\rpcode`/`\lpcode` 的段被判「状态未变」，缓存错复用），
  迁移锁在旧 HEAD 上即红（主控核验：旧 ValueState 无两字段、protrusions 两处
  均不在）；
- 回归锁 4 把：迁移锁（digest 不变 + value_state 必变 + 深比较兜底双向断言）、
  共享锁（ptr_eq/计数≥2/写分离/还原重共享）、fork 隔离锁、roundtrip 泥沙锁扩
  样本（`\fontdimen`/`\rpcode` 进 snippets+mud），全部「去修复必红」实证。

## 主控独立复测（本轮，HEAD 重建二进制）

| 门禁 | 结果 |
|---|---|
| cargo test -p ntex-core -p ntex-layout | 全绿（core 615 / layout 362 汇总口径与 commit 声明一致） |
| clippy -D warnings（core+layout，all-targets） | 0 |
| cargo fmt --all --check | 0 |
| trip 直接跑（plain 预载） | 251B/1 页，与 fixtures/trip/trip.dvi golden 逐字节一致 |
| trip.tex 全本（两步 `--no-plain --dump` → `--fmt`） | 4015B/9 页（blade8/9 定案口径） |
| tripos（`--fmt trip-ntex.fmt`） | 750B/12 页，逐字节一致 |
| 宏包回归 | **13/13 PASS**（`target/debug/ntex-dvi` 已覆盖为 HEAD 新二进制；基线报告元数据脏改动已 checkout 还原） |

fmt FORMAT_VERSION 26 未动：FmtState 序列化的是 `to_vec()` 排序 Vec（格式未变），
无需 bump / 重生成发行 fmt。

## Codex review（gpt-6-sol，只读）结论：需先修 1 项

5 项定向重点：①CoW 铁律（写路径全覆盖 make_mut、无字段直访/裸指针写/iter 残留）
**通过**；②恢复/重建路径**需修**：`import_state` 重建 fontdimens 却不清
protrusions——凸出量不随 fmt 持久化，同实例已有 `\rpcode`/`\lpcode` 后再加载
新 fmt，旧值泄漏进新文档（预存缺口，非本刀引入，但属本刀同域两表对称面）；
③M5 比较面（value_state + ValueExtras 捕获/还原两通道都带两表）**通过**；
④PartialEq 假等/假失配（fork 各自写回同值必须判等；存活 Rc 无地址复用假等）
**通过**；⑤MSRV/clippy/panic 面**通过**。

主控同步走读（grep 独立验证）与 review 结论一致。

## 必修落码（本轮主控亲征）

- `protrusions.rs` 补 `clear()`（`Rc::make_mut` 两表清空，CoW 铁律对齐——顺带
  发现：实施刀漏了这个方法，`import_state` 处若当时就写 clear 会编译不过）；
- `mod.rs` `import_state` 在 fontdimens clear+重建后补 `self.protrusions.clear()`
  （新 fmt = 新文档起点，运行时覆盖表不得跨文档泄漏）；
- 回归锁 `fmt_import_clears_stale_protrusions`：同实例写 `\rpcode`/`\lpcode` →
  import 新 fmt → 两表读 0（去修复必红：旧代码泄漏 50/30 即红）。

## 修复后门禁（全绿后提交）

core+layout 全绿 / clippy 0 / fmt 0 / trip 251B golden + 全本 4015B/9 页 +
tripos 750B/12 页逐字节一致 / 宏包 13/13。

## 余项（阶段六待办顺延）

digest 剩余成分（mathcodes/hyphenchars/skewchars 小表实测 ~ns 级留指纹）；
record_post 恒真捕获；副作用边界（`\output` 回放/`\write` 流/`\input` 注入/marks
保守全量重排）；槽级归因；模糊测试进 CI；`.aux`/`.toc` 增量。
