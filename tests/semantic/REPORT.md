# tex.web 语义测试矩阵（自动生成用例 × pdfTeX 期望值）

> **方法论**：期望值一律 pdfTeX 实跑固化，不接受人写/模型猜。
> 管线：`rules.yaml` → `gen.py gen` → pdfTeX 批跑固化 → `gen.py ntex` → 红绿矩阵。
> 模型可翻译规则为探针，但猜测进不了期望值（被 pdfTeX 行为物理过滤）。

## 用法

```bash
python3 tests/semantic/gen.py gen          # 规则 → 探针池
python3 tests/semantic/gen.py reference    # pdfTeX 固化期望 → cases/
python3 tests/semantic/gen.py ntex         # NTex 跑批
python3 tests/semantic/gen.py matrix       # 红/绿矩阵
```

## 首轮矩阵（2026-09-12，22 规则）

**PASS 13 / DIFF 9**（此前「全红」源于归一化缺陷，非语义失败）

### ✅ 绿灯区（已锁定的语义，回归即红）

数字扫描：尾随空格消散 / cs→space 别名消散 / 嵌套条件累计 / 反引号字符码 /
numexpr-relax 吸收 / numexpr 作 ifnum 操作数；
宏定义：常规配平 / skip 区组不配平 / ## 折叠 / IPN 可恢复 / unbalance 起点；
诊断：原语规范名（show/meaning）。

### 🔴 红灯区（9 例 = 6 个语义缺口 + 3 个探针期望问题）

| 簇 | 用例 | want vs got | 定性 |
|---|---|---|---|
| **A1 scan_expr 别名 stop** | numexpr_csalias_stop / expand_alias_chain_numexpr | `! Undefined \scan_stop:` vs 正常出值 | **plain 无 `\scan_stop:`**——探针用错 cs，应别名到 `\relax`。**探针问题**，需改用 expl3 真实形态（catcode 下 `\let\mystop\relax`） |
| **A2 溢出钳制** | scan_int_overflow_clamp | `! Number too big` vs 静默钳制 | **真缺口**：NTex 未报 `Number too big`（expl3 数字常量上万级，低优先但该报） |
| **B1 hash_brace `#{`** | scan_toks_hash_brace_delim | `[X]{Y}` vs `[X]Y` | **真缺口**：`#{` 的「`{` 存为末参定界并追加体尾」语义缺失（expl3 `\tl_...` 分隔大量依赖） |
| **B2 begingroup 计数** | scan_toks_begingroup_counts | `Extra \endgroup` vs `Too many }` | 两侧都报错但恢复路径不同——需精读 tex.web 对齐恢复细节 |
| **B3 skip 区 outer** | cond_skip_outer_in_skipping | `Incomplete \ifcase` vs 静默通过 | **真缺口**：skip 区 outer 检查未实现（TRIP L363） |
| **B4 空组惯用法/Runaway** | scan_toks_empty_edef / runaway_report | File ended 通报 vs 静默 | unbalance 修复后仍缺「File ended」通报行——通报文本对齐问题 |
| **E1 反引号帮助文本** | scan_int_backquote_alphabetic_err | 顺序差异 | 归一化需容错帮助行顺序，**矩阵问题**非语义 |

## 纪律

- 每修一个语义缺口 → 对应用例从 🔴 转 ✅，**永久回归锁**。
- 新增规则必须附 tex.web 节号 + pdfTeX 实跑期望。
- 探针非法（pdfTeX 都打不出确定输出）→ 自动丢弃，不入矩阵。

---

# 更新（2026-09-12 第二轮）：C 组交错场景全绿——推翻重构前提

新增 10 条取 token 交错场景规则（c_*）：
**10/10 全绿**。`collect_args`/`scan_edef_body`/`expr_peek` 三消费者
在现有 push_frame 架构下语义正确（含条件 token 作实参数据、hash_brace、
noexpand 生命周期、protected 抑制、0 参数定界串）。

**结论修正**：此前「三消费者交错需 Expander 级 pending 队列重构」的前提
被矩阵证伪——现有架构无此缺口；两次队列实验的失败是队列自身破坏了
expand_once 的「原样返回→exec_primitive」判断链（已归档回退）。

## Number too big 修复补全

报错后**停止数字累计**（tex.web goto done）：pdfTeX 报 1 次 vs NTex 曾
每位报一次（5 次）。现在两侧均 1 次 + 钳制 2147483647。

## 矩阵现状：26/32 绿

6 红全部归类为「诊断信息保真度校准」（不影响排版结果）：
- error_line_no 行号锚定（3 例：want l.2/l.3 vs got l.5/l.6）
- Runaway 的吞入上下文展示行（->ABC \end，2 例）
- 反引号帮助文本行顺序（1 例）

→ 新立「校准层」待办，与语义修复分离推进。
