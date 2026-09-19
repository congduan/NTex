# 原语语义正确性保障 Check Table（2026-09-05 主控实测盘点）

> 对应问题："300 多个原语如何保证语义实现的正确性？"
> 本表 = 保障机制的**实际覆盖现状**（2026-09-05 实测），非设计愿景。

## 总体答案：四层防线

| 层 | 机制 | 覆盖什么 | 现状（实测） | 防不住什么 |
|---|---|---|---|---|
| ① | **TRIP/ETRIP 一致性**（对真 TeX 逐字节 log 对拍） | 原语的真实语义 + 错误恢复路径 | A 组 42/42 ✅、B 组 36/36 ✅、C 组已接线 ✅；**但 etrip 收尾段实测仍失败**（pass2 `group_end 无配对 group_begin`，卡 \currentgrouptype 段 l.358） | 引擎崩溃在 diff 之前的段落 → 后面全测不到 |
| ② | **语义锁单测**（对照 tex.web 行为逐点断言） | 单个原语的边界语义（越界/恢复/错误块格式） | ntex-core 318 测（expand/tests.rs 217 个）+ workspace 共 575 测全绿（23:55 实测） | 测试矩阵外的边界（catcode/组参数/非字母 cs 名——M5 与 4f45101 两次实证） |
| ③ | **错恢复错误块对拍**（"Bad register code" 类逐行一致） | 错误消息 + read-again + l.N 光标 + help 文本 | sparse arrays 8 块 + marks 2 块已逐行对齐；semantic diff -3051/+2352 | 独立 `...` 省略行（\errorcontextlines 多层上下文）未模拟 |
| ④ | **latex.ltx/expl3 实载探针**（生产宏层当压力测试） | 原语在真实宏组合下的行为 | 每刀复测：错误 7669→9，阻塞点推进到 l.8073 | 只测"不炸+推进"，不做逐 token 语义对拍 |

## 关键结论（诚实版）

1. **"全部正确"目前无硬证明**——ETRIP 收尾的 etrip.log 逐字节比对**尚未完成**（这是唯一
   能给全量原语下"语义正确"结论的口径，状态源 ETRIP-primitives.md 明确标 ❌ 未开始→销账中）。
2. **本轮实测新增发现**：etrip 跑到 \currentgrouptype 段（l.358 区）引擎报
   `group_end 无配对 group_begin` 内部错误 → TRIP 同位置崩。**这是 21 刀级靶子**
   （数学左组 math left group 收尾的组类型记账缺陷），比 expl3 str_case 更基础。
3. 最大的结构性风险 = **子 agent 测试矩阵外的边界**（历史两次实证：M5 词法漏记、
   4f45101 条件栈帧序回归——TRIP/单测全绿但 expl3 路径没测）。
   对策已固化：跨机拉取后必跑战役探针；验收必构造反例测试。

## 建议的下一步（按收益排序）

| 优先级 | 动作 | 收益 |
|---|---|---|
| P0 | 修 TRIP/ETRIP l.358 区 math left group 组配对崩溃（现卡点） | 解锁 etrip.log 逐字节比对——原语全量正确性的唯一硬口径 |
| P1 | etrip.log 逐字节比对销账（read-again 格式统一 + l.N 光标全覆盖） | 语义锁从"抽查"变"全查" |
| P2 | corpus-probe 爬坡（0/8 → N/8）纳入每 3~4 轮验收 | 端到端产物级验证 |
| P3 | 双轨等价（bytecode vs 解释器）已有，但注意：双轨一致 ≠ 语义正确，只证明两轨同病 | 不追加投入 |

## 数据来源
- Primitive 枚举 439 变体（grep 实测，含别名/内部变体，对外原语约 300+）
- ETRIP-primitives.md（唯一状态源）：A 42/42、B 36/36、收尾 ❌
- 实测：workspace 575 测全绿；ntex-trip --driver ntex TRIP/ETRIP 双双卡 l.358 区
- 二十刀（str_case l.8073）Claude 委派进行中，与本表发现互不阻塞
