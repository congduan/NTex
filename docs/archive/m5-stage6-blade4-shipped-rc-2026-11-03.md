# M5 阶段六第四刀战报：shipped 页面 Rc 共享（2026-11-02/03）

## 概述

- **刀靶**：`finish_doc` / `finish` 交付页面时的整页树深拷贝（`shipped.clone()`，第三刀遗留两靶之一，实测 ~3.8ms/次）。
- **提交**：`d033930`（刀主体）+ `94f8fd2`（基线元数据）。
- **改动**：`page_state.shipped: Vec<BoxNode>` → `Vec<Rc<BoxNode>>`（`push_shipped` 入队处包 `Rc`，paging.rs）；`CompileOutput.pages` / `Typesetter::shipped` / `FinishOutput.shipped` 同步改共享句柄；`ntex-dvi::write_dvi`/`write_dvi_with_counts` 泛型化 `P: Borrow<BoxNode>`（源兼容，空表处 `write_dvi::<BoxNode>` 显式标注）；ntex-wasm `Compiled`/`Document`/`render_page_core` 持共享句柄；ntex-backend 渲染 trait 不动，演示 CLI 与端到端用例在边界摊平。

## 安全性前提

页树入队（`push_shipped`）后只读：入队前 `materialize_leaders` 已物化引导符，唯一后续操作是回滚 `truncate`（只动 Vec 长度，不碰元素）。Codex review（gpt-6-sol 只读，六项定向）确认无隐藏可变路径、无 `Rc::make_mut`/unsafe 改写已入队页。

## 实测（release，HEAD 对照 worktree 交错 4 轮）

| 场景 | 前 | 后 |
|---|---|---|
| 改正文增量 | 8.92–9.55ms | **7.11–7.37ms（−20%）** |
| 改正文加速比 | 7.6–7.9x | 7.5–7.8x（全量同幅 66–80→55ms） |
| 改宏体增量 | 30.5–32.6ms | 28.8–29.7ms（−7%，无回退） |

本机绝对墙钟约为第三刀记录环境的 1.4 倍（同 commit 对照），故以对照差值判读。

## 副刀裁决：复用段 `contrib` 共享判无收益，保留 `clone`

探针实测缓存节点流深拷贝 0.46ms/轮（1026 节点 × 50 轮 = 23.18ms），而 `append` 必须 owned——`append_ref` 逐字段拷贝对 Box/Vec 变体同样是深拷贝，只能省外层 Vec 缓冲与一次搬运（≤0.05ms，<1%），不足以抵消新增 API 与借用面，故不做。

## 回归锁

`shipped_pages_shared_not_deep_copied`（incremental_tests.rs）：交付后 `Rc::strong_count ≥ 2`（builder 与输出同源）+ 编辑后前缀页 `Rc::ptr_eq` 仍共享（复用路径未重新分配）+ 与全量逐位一致。review 确认敏感性：恢复深拷贝 → 引用计数断言红；编辑重建前缀页 → 指针相同断言红。

## 门禁

layout 357（+4 集成面全绿）/ core 605 / clippy 0 / fmt 0 / tripos 750B 12 页 MD5 `92100387…` 与基线一致 / trip 4015B 9 页与 HEAD 逐字节一致（MD5 `7b3ae2c5…`；/tmp/tripbase 留档 177B 为陈旧产物，vsize 语义修正后全本 9 页是正确形态，skill 有案）/ 宏包回归 13/13。

## 流程记录

上一轮 cron 勘察留 WIP（14 文件，plan.md 已写战报）→ 本轮主控验收补齐门禁（core/clippy/fmt/TRIP/宏包）→ Codex review 只读六项全过、无阻塞 → 两笔提交推送。

## M5 阶段六剩余

见 plan.md §2「阶段六待办」：expand 侧 `StateSnapshot::capture`（每执行段两次 ~30µs）、副作用边界、`Expand` 纯函数化、槽级归因、模糊测试进 CI。
