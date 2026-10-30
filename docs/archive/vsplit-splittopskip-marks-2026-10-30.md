# vsplit splittopskip/mark 语义刀（2026-10-30）

日期：2026-10-30。基线 HEAD `44a3b89`，收口 `8860bf0` + `540b172`（已推 origin/main）。
性质：cron 前轮 WIP 主控接管收口。

## 来源与形态

工作树遗留 WIP（5 文件）＝两个逻辑单元：

1. **vsplit 语义刀**（node.rs + boxes.rs + tests_box.rs，本次主提交）：
   - `prune_vsplit_top` 从「无脑删 glue/kern/penalty」改为 tex.web `prune_page_top`
     语义——删到首个盒/规则、穿透 Mark 等无维度节点、在首盒前插入
     `max(0, splittopskip − 首盒高)` 胶水（`Node::dimensions().height` 取高，
     负宽度钳 0）。`split_vbox` 签名加 `splittopskip: Glue`。
   - `\vsplit` marks 写入侧：切分前清 `marks_split_first/bot`，遍历 result
     顶层 children 收集 Mark（`or_insert_with` 保 first / `insert` 覆盖 bot，
     按 class 分桶）。字段/读侧/快照恢复链此前已在（mod.rs/incremental.rs），
     本刀只补写入点。
2. **ntex-mcp 跟随**（server.rs 注释 + mcp_server.rs 测试期望）：跟随已提交的
   e0d97c8（`\write18` 降级 `runsystem(...)disabled`）更新文档与断言，独立
   docs commit。

## GT 对拍（真 etex，TinyTeX，四探针逐字一致）

| 探针 | 期望（=GT 实测） | 结果 |
|---|---|---|
| splitfirst/splitbotmarks 双类两次切分 | R1=[M1\|M2\|A1\|A2]、R2=[M3\|M3\|A3\|A3] | ✅ 逐字 |
| 余量高度（splittopskip=10pt，首盒 2pt） | REM1=[12.0pt]、REM2=[10.0pt] | ✅ 逐字 |
| 负宽度钳制（首盒 15pt > splittopskip 10pt） | REM=[12.0pt] 双侧相同 | ✅ |
| 无合法断点 fallback（q=null） | FB=[5.0pt\|0.0pt] 双侧相同 | ✅ |

## review 通道

Claude Code 124 超时 ×2（opus-4-6 / glm-5.3-flash 各 240s，0 输出）——
登记不可用，**主控亲自 review**。核查项与结论：
- `Node::Rule` 的 `dimensions()` 臂正确返回 height 字段（node.rs:300）；
- `split_vbox` 全仓调用点仅 boxes.rs:111 + 测试（已同步）；
- 负宽度钳制与 fallback 路径均有 GT 对拍（上表 3/4 行）；
- mark 收集只看顶层——与 tex.web split 遍历一致（嵌套盒内 mark 不计入
  splitfirst/botmarks）；
- clippy -D warnings 0 / cargo fmt 0（MSRV 合规）。

## 门禁

- cargo test ntex-core + ntex-layout + ntex-mcp + ntex-trip 全绿
  （layout 344，含新增 3 把 transcript 回归锁，期望=GT 实测值）；
- tripos 12 页 750B SHA `574045fb` 与基线逐字节相等（TRIP 线防回退锁）；
- 宏包回归 13/13（pkg-regression.py，baseline 文件元数据改动已 checkout 丢弃）。

## 本轮口径再证

tripos 验收姿势三件套（缺一会跑出「未产出页面」假形态）：
①工作目录放 trip.tfm；②`trip.tex --no-plain --dump trip.fmt`
（**--no-plain 必带**，plain 注入的 `\shipout`/`\output` 会毒化快照）；
③`tripos.tex --fmt trip.fmt`（空格分隔，非 `--fmt=`）。10-28 诊断刀已登记，
本轮复证。

## 残留 / 下一刀候选

- `\glue` 的 dimensions().height 恒 0——prune_vsplit_top 不会把 glue 误当盒
  （match 臂只认 Box/Rule），无风险；
- marks 类桶用 `class.unwrap_or(0)`：`\mark{...}` 无类形归 0 类，GT 一致；
- page builder insert split 路径（vert_break）不在本刀范围（此前刀AG 已对齐）。
