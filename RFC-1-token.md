# RFC-1：Token 表示与内存布局

- 状态：草稿（M0 评审）
- 关联：plan.md M1/M2；RFC-4（字节码 IR）引用本设计
- 范围：TeX VM 中 token 的数据结构、内存布局、驻留与版本化，不涉及展开算法本身

---

## 1. 背景与目标

Token 是 TeX VM 的基本数据单元：展开引擎消费 token 流，排版阶段消化 token 生成的节点。
本 RFC 决定 token 在内存中的**物理形态**，目标是同时满足：

1. **紧凑**：单 token 固定小体积（目标 8 字节），支撑每秒数亿次 token 操作的吞吐；
2. **无 GC**：token 不携带可变堆指针，全部落在 arena / 连续内存中；
3. **可驻留（internable）**：控制序列名、字符串统一驻留为 u32 id，`.fmt` mmap 后无需字符串查找；
4. **快照友好**：宏体 token 数组不可变，支持 CoW 版本化与多线程共享；
5. **字节码友好**：M2 的字节码可直接引用/内联 token 载荷（立即数）。

## 2. TeX token 语义回顾（必须逐条满足）

| token 类别 | 判定/比较规则 | 备注 |
|---|---|---|
| 字符 token | 相等 ⇔ (catcode, 字符码) 相同 | catcode ∈ 0..15 |
| 控制序列 token | 相等 ⇔ 是**同一个控制序列**（同名即同体） | 不比较"当前定义" |
| 宏参数 token | `#1`..`#9`，仅宏体内合法 | 展开时被实参替换 |
| 对齐/结束 token | `\end`、右花括号等内部标记 | 展开期间不可见 |

关键语义细节（实现必须保真）：

- **token 一经生成，catcode 即固化**：输入时查 catcode 表生成字符 token，之后不再回读 catcode 表；
  `\meaning`/`\show` 输出的就是 token 生成时的分类；
- **`\ifx` 比较**：字符 token 比 (catcode, char)；控制序列 token 比是否同一 cs，
  **不比较定义内容**；
- **`\csname..\endcsname` 动态生成** cs：存在则复用（同一 csid），不存在则新建；
- **active char（catcode 13）**：展开为宏调用，token 仍是"字符 token + catcode=13"，查 eqtb 找等价；
- **`\let\a\b`**：产生"cs → cs"的等价关系，需要 csid 间接层，不能复制宏体。

## 3. Token 物理表示：8 字节 tagged union

用 `u64` 承载，Rust 侧用 `#[repr(transparent)]` 包装枚举保证零开销。

```
u64 layout（高 → 低）：
┌───────┬───────────────┬────────────────────────────────────────────┐
│ tag   │ 载荷字段       │ 说明                                        │
│ 4 bit │               │ 见下                                        │
└───────┴───────────────┴────────────────────────────────────────────┘
```

**变体设计**（tag 高 4 bit，载荷剩余 60 bit）：

| tag | 变体 | 载荷（60 bit 分配） | 说明 |
|---|---|---|---|
| 0 | `Char` | catcode(4b) + charcode(21b) + 保留(35b) | catcode 0..15，charcode 0..0x1FFFFF（兼容 Unicode 全量） |
| 1 | `ControlSeq` | csid(32b) + 保留(28b) | csid 为 InternTable 索引 |
| 2 | `MacroParam` | num(4b) + 保留(56b) | `#1`..`#9`，0 表示 `#` 本身 |
| 3 | `EndGroup` | 保留(60b) | 右花括号（扫描结束标记） |
| 4 | `OuterCall` / `End` | 保留 | 内部控制 token（不进用户可见流） |
| 5..15 | 保留 | — | 预留给字节码扩展标记 / 调试信息 |

**设计要点**：

- **控制序列 token 不携带定义**：cs 的"等价"（eqtb 项）通过 csid 查表获得。
  好处：token 本身无堆指针、可复制共享；`\let` 别名只改 eqtb，不动 token；
  不可变快照只需版本化 eqtb，token 数组原样共享。
- **Char 变体的 catcode 固化**：token 内部保存生成时的 catcode（见 §2），
  与"输入后改 catcode 不影响已读 token"的语义天然一致。
- **无指针 → 无 GC**：token 数组可整体放入 arena，或直接驻留为不可变切片。

## 4. 驻留表：InternTable

```
InternTable（不可变版本 + CoW）：
  names: Vec<ByteSlice>          // 控制序列名/字符串，去重
  csid → (name_id, eqtb_slot)    // csid 即数组下标，O(1) 定位
```

- **csid = u32**，即 InternTable 下标；`.fmt` 序列化时把整个表线性化，
  mmap 后 csid 直接索引，**零字符串查找**；
- `\csname foo\endcsname`：哈希查表 → 命中返回 csid，未命中则追加（产生新版本 InternTable）；
- **并发（M6）**：InternTable 的只读查询可并行；新建项走原子追加 + 版本切换，不锁读路径；
- **别名 `\let\a\b`**：eqtb 项类型为 `Alias(csid)`，展开时解引用一层。

## 5. 宏体表示：连续 Token 数组（非链表）

传统 TeX 的宏体是**单链表 token 列表**，展开时逐节点拷贝。
现代选择：**连续内存的不可变 token 切片**（`&'a [Token]` 或 arena 内 `TokenArray { ptr, len }`）。

优势：
- 展开/扫描 O(1) 随机访问（分隔参数匹配、回退扫描受益）；
- 整段共享：`\let` 到已定义宏、`.fmt` 加载、增量缓存复用，都是"指针借用"而非拷贝；
- M2 字节码的"取实参"指令可直接索引宏体槽位。

```
MacroDef（不可变）：
  params: ParamSpec        // 参数数(0..9)、long 标志、分隔参数 token 列表
  body:   TokenArray       // 展开体（token 表示，字节码由 M2 另建指令区）
```

宏定义版本化在 **eqtb** 层完成：

```
eqtb_slot: enum {
    Undefined,
    Macro { version: u64, def: Arc<MacroDef> },
    Primitive(prim_id),            // 原语，M2 后可指向字节码入口
    Register(reg_kind, idx),       // \count/\dimen/\skip/\toks 等
    Alias(csid),                   // \let
    ...                            // 字符/参数/占用位等
}
```

版本号用于增量缓存（M5）的依赖追踪：展开结果记录"依赖了 eqtb 哪些槽的哪个版本"。

## 6. 与字节码 IR（RFC-4）的衔接

- 字节码指令的立即数可以是：`Token` 原值（8B 内联，加载即用）或 `csid`/`TokenArray` 索引；
- 宏体双表示：`body: TokenArray`（语义/`\meaning` 用）+ `code: Bytecode`（执行用），
  两者由编译器保持等价（M2 双轨 diff 测试验证）；
- token 的 8B 布局保证"内联到指令流"零解包开销。

## 7. 内存归属与生存期

| 对象 | 分配方式 | 生存期 |
|---|---|---|
| 字符/控制序列 token 数组（输入流、宏体） | bumpalo arena | 所属编译单元（`.fmt` 或文档段） |
| InternTable | 不可变版本 + Arc | `.fmt` 存活期 |
| eqtb 槽 | 版本化数组（CoW） | 快照链共享 |
| MacroDef | arena + Arc | 与 eqtb 版本绑定 |

## 8. 边界情况清单（实现与测试必须覆盖）

1. `\csname` 动态建 cs，含空 csname、非法字符（catcode 变化后的重解析规则）；
2. `\let` 链式别名与"别名再别名"；`\let` 到 `\outer` 宏的传播；
3. active char（catcode 13）展开为宏，`\meaning` 输出格式；
4. Unicode 字符 > BMP（XeTeX 兼容路径预留 21 bit charcode）；
5. 宏体内 `#` 的三种含义：参数（`#1`）、字面 `##`、非法 `#`（报错语义）；
6. `\ifx` 对上述各类 token 的比较规则（尤其 `Alias` 不展开）；
7. 分隔参数的 token 级匹配（分隔串是 token 序列而非字符序列）；
8. `.fmt` 序列化往返：InternTable + eqtb + token 数组 round-trip 后逐位一致。

## 9. 开放问题（M0 评审讨论）

- **Q1**：是否需要 16 字节 token（多出的空间存"源码位置"或"protected 标记"）？
  建议：默认 8B；源码映射走独立 side-table，不进 token。
- **Q2**：Char 变体 35 bit 保留空间是否挪用给"字形变体选择器"等 XeTeX 扩展？
- **Q3**：`EndGroup`/内部标记是否拆分到独立 enum 而非复用 token 位域？
- **Q4**：InternTable 的 CoW 粒度（整表复制 vs 分桶复制），与 M5 依赖追踪的代价权衡。

## 10. 参考

- D. E. Knuth, *The TeXbook*, Ch. 7–9, 20, 24
- Victor Eijkhout, *TeX by Topic*, "Tokens" / "Macros" 章节
- e-TeX 扩展规范（`\protected`、`\ifdefined` 对 token 语义的影响）
