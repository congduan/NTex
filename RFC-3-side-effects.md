# RFC-3：副作用模型（VFS + 输出边界提交）

- 状态：草稿（M3 收尾定稿用）
- 关联：plan.md M3 收尾 / M5 增量；idea.md §2.4
- 范围：文件读写原语（`\input`/`\read`/`\write`/`\openin`/`\openout` 等）的
  统一 **VFS 抽象**、**延迟写入**语义与**副作用提交边界**。不涉及排版与渲染。

---

## 1. 背景与目标

TeX 的排版核心是"纯函数化"的：给定输入与状态，产出页面。唯一破坏纯函数性的
是**副作用原语**——文件读写、shell 转义。M5 增量计算（改 1 处只重算受影响段落）
的前提是副作用可隔离、可重放、可在确定边界提交。

**目标**：

1. 所有文件读写走统一 **VFS 层**：本地路径 / 内存 / 网络 / WASM 虚拟文件系统
   可互换，排版核心不感知后端；
2. `\write` 采用 TeX 原语义的**延迟写入**：token 列表先入队，在**页面真正输出
   （`\shipout` 边界）**时统一落盘——页面被 `\output` 例程丢弃则内容不写；
3. 副作用边界成为 M5 增量缓存的提交点：纯计算部分可缓存，副作用只在边界提交；
4. 为 `.fmt`/WASM 提供统一的文件抽象。

---

## 2. 设计原则

1. **VFS 是唯一入口**：排版内核（ntex-core）不直接触碰 `std::fs`；
   读（`\input`/`\openin`/`\read`）与写（`\write`/`\openout`）全部经 `Vfs` trait。
2. **读与写分离**：`\input` 在 VM 侧读取并推入输入帧；`\write` 的 token 列表
   在 VM 侧扫描入队，**落盘动作**由输出边界触发。两者共享同一个 `Vfs` 实例。
3. **延迟写入**：`\write<n><general text>` 只把 token 列表入队（同 TeX：页面
   输出时才真正写）。`\immediate\write` 绕过延迟立即写。`\closeout` 显式 flush。
4. **输出例程边界**：页面 `fire_up` 后经 `\output` 例程（或直通）`\shipout`，
   写入发生在"页面真正进入输出列表"那一刻——丢弃页面不产生写入。
5. **确定性**：默认单线程顺序语义；同一输入两次编译产出相同文件内容
   （`.aux`/`.toc` 收敛语义，M5 验证）。

---

## 3. VFS 接口（ntex-io）

新 crate `ntex-io`，提供：

```rust
/// 虚拟文件系统：排版内核与真实/虚拟后端的唯一文件接口。
pub trait Vfs {
    /// 读整个文件；不存在返回 `Ok(None)`（TeX `\openin` 语义：不报错）。
    fn read(&mut self, path: &str) -> io::Result<Option<Vec<u8>>>;
    /// 写整个文件（覆盖）。
    fn write(&mut self, path: &str, bytes: &[u8]) -> io::Result<()>;
    /// 追加写（`\openout` 流累积后一次写；不提供逐次 append 也行）。
    fn append(&mut self, path: &str, bytes: &[u8]) -> io::Result<()>;
}
```

实现：

- `LocalVfs`：`std::fs` 直读直写（默认后端）；
- `MemVfs`：`HashMap<String, Vec<u8>>`，测试 / WASM 用（可注入初始文件）。

设计取舍：

- 不做"流式打开句柄"抽象——TeX 语义下 `\openout` 只是登记目标路径，
  实际写出在 flush 边界一次完成；`\openin` 在打开时读入内存（文件小）；
- 路径一律字符串（不解析成平台 PathBuf 语义，保留原始字符串，WASM 友好）；
- shell 转义（`\write18`）**不在 M3 范围**：遇到 `\write18` 报错拒绝。

---

## 4. 原语语义

### 4.1 读侧

| 原语 | 语义 |
|---|---|
| `\input<file>` | 读文件内容推入 `Source` 输入帧（嵌套）；缺失报错（TeX："I can't find file"） |
| `\openin<n>=<file>` | `vfs.read` 试探；存在 → 打开（读入内存），不存在 → 流未打开（不报错） |
| `\closein<n>` | 关闭读流 |
| `\newread<cs>` | 分配最小空闲读流号，`\def`-等价赋给 cs（TeX：全占用则报错） |
| `\read<n> to <cs>` | 从流读**一行**（到行尾/EOF），按当前 catcode 表 token 化，`\def` 赋给 cs；EOF 则 cs 未定义（TeX：`\read` 到 EOF 报错 "end of file" 且 cs 不变） |

`\read` 的 token 化：空格字节 → catcode 10 空格 token；其余字节 → 按 catcode 表
（字母 11 / 其他 12 / 其他类别）转为 token。**不做展开**（TeX 语义：读入 token
原样存入 cs，使用时才展开）。

### 4.2 写侧

| 原语 | 语义 |
|---|---|
| `\newwrite<cs>` | 分配最小空闲写流号赋给 cs |
| `\openout<n>=<file>` | 登记流 n 的目标路径（不立即创建文件） |
| `\closeout<n>` | flush 流 n 的待写队列并关闭 |
| `\write<n><general text>` | 扫描 general text（到 `\relax` 或外层组结束）入队；`<n>=18` → shell，报错 |
| `\immediate` | 前缀：作用于下一个 `\write`/`\openout`/`\closeout`（立即执行） |

### 4.3 输出时的展开

写文件时（flush 边界）对入队的 token 列表执行**完全展开**（宏、`\the`/`\number`/
`\string`/`\meaning` 等可展开原语），结果按字符输出：

- 字符 token → 其字节；空格 token（catcode 10）→ 一个空格；
- 控制序列 → 报错拒绝（TeX：`\write` 中不可展开的 cs 触发 "You can't use..." 或按 `\string` 输出——M3 简化：报错）。

实现复用引擎的展开器：入队 token 作为新的 token 列表帧，运行到耗尽收集
字符 token（同 `\edef` 的展开收集路径）。

### 4.4 提交边界

写流待写队列在以下**三个边界**提交（任一先到先触发，提交后清空该流队列）：

1. `\immediate\write`：立即展开并写；
2. `\closeout<n>`：flush 流 n；
3. **页面 shipout**：页面真正进入输出列表时，flush 所有打开且有待写内容的流
   （TeX：`ship_out` 中先 `print_write_whatsit`）。页面被 `\output` 例程丢弃
   则跳过；
4. 排版结束（`\end` 收尾）：flush 所有残留（TeX `final_cleanup`）。

`\openout` 的"空文件"语义：TeX 在 `\openout` 或首次写时创建文件；
M3 简化：`\openout` 只登记路径，**首次实际写入时**经 VFS 创建（`write` 覆盖/
`append` 追加，与 TeX `\openout` 覆盖、多次 `\write` 追加一致）。

---

## 5. 实现结构

```
ntex-io        Vfs trait + LocalVfs + MemVfs（无 ntex-core 依赖）
ntex-core      Expander 增加：
                 vfs: Box<dyn Vfs>
                 read_streams: Vec<ReadStream>    // 打开状态 + 内存内容 + 位置
                 write_streams: Vec<WriteStream>  // 路径 + 待写 token 队列
                 immediate_pending: bool
               原语注册 + 参数扫描（VM 侧）
ntex-layout    NodeBuilder：页面进入 shipped 时置 write_flush 标志；
               Typesetter::finish 收尾 flush；\end 收尾
```

sink 交互（同 M3-5-3 的 output_pending 模式）：NodeBuilder 在真正 shipout 页面时
设置 `write_flush_pending` 标志；Expander 在 token 边界检查并调用 `flush_writes`
（可访问自身 VFS 与队列）。默认 `VecSink` 不置标志 → 纯展开轨道无副作用。

---

## 6. 与 M5 增量 / WASM 的关系

- **增量缓存**：`\write` 入队是"纯数据"（token 列表），可随段落缓存；
  落盘只在 shipout 边界提交 → 增量重算不产生半成品 aux 内容；
- **`.aux`/`.toc`**：两次编译收敛语义 = 同一 VFS 输入 → 同一写序列（M5 验证）；
- **WASM**：`MemVfs` 挂载虚拟文件系统，`\input`/`\write` 不依赖宿主文件系统；
- **并行（M6）**：段落级并行仅在"无跨段写流副作用"时启用——VFS 与写队列
  给出显式的副作用登记点。

---

## 7. M3 收尾范围（本期实现）

- [x] ntex-io：Vfs trait + LocalVfs + MemVfs
- [x] `\input`（含嵌套、缺失报错）
- [x] `\openin`/`\closein`/`\newread`/`\read...to`
- [x] `\newwrite`/`\openout`/`\closeout`/`\write`/`\immediate`
- [x] shipout 边界 flush + 排版结束 flush
- [ ] `\write18`（shell）——拒绝（报错），M9 再议
- [ ] `\openout` 追加 vs 覆盖的 `+file` 变体——M3 简化只做覆盖

**验收**：多文件文档（`\input` 分章 + `\write` 生成 .aux + `\read` 回读）走通，
输出文件内容与 TeX 语义一致；纯 `VecSink` 轨道（无排版器）无副作用。
