# NTex

**100% 兼容 LaTeX/TeX 宏机制的现代排版 & 渲染引擎**——不修改宏语义，只换掉它的"运行平台"：
把 1970 年代的单线程 C 解释器（Web2C），重写为 **现代 VM + 增量计算 + 并行布局 + GPU 渲染** 的现代排版内核。

> 最简概括：**"一个基于状态快照的 TeX 虚拟机（求值）+ 纯节点流的多线程增量布局（排版）+ GPU/Canvas（渲染）"**

![ntex-studio 实时预览工作台（左：TeX 源码编辑；右：vello GPU 渲染）](screenshots/screenshot1.png)

## 核心架构

```
TeX/LaTeX 源码
    ▼
0. 输入层（VFS / 文件抽象）
    ▼
1. TeX 虚拟机（阶段 1：求值）
   ┌────────────┐   ┌──────────────────────┐
   │ Token 流    │◄─►│ 不可变状态（catcode/  │
   │ (8B token) │   │ 宏字典/寄存器）快照    │
   └─────┬──────┘   └──────────────────────┘
         ▼ 宏展开 + 执行循环
   纯排版节点流（Node List）
    ▼
2. 增量缓存层（Salsa 式求值图）
    ▼
3. 布局引擎（阶段 2：排版，并行）
   段落折行 Knuth-Plass │ 断页 │ 数学排版 │ 断字 │ 字体
    ▼
4. 盒子树 + 输出例程（\shipout）→ DVI
    ▼
5. 渲染后端（阶段 3：渲染）ntex-pdf（DVI→PDF）/ Skia / WASM
```

## 四大性能支柱

1. **状态快照 + CoW**：不可变状态表，微秒级快照，为增量编译与撤销/重做铺路
2. **预编译 `.fmt` 内存 Dump + mmap**：毫秒级完成 latex.ltx 初始化
3. **Salsa 式记忆化增量求值**：改第 50 页的一个字，前 49 页 0 毫秒跳过
4. **Arena 内存池**：token/节点连续分配，零 GC 暂停

## 工作区结构

```
crates/
  ntex-core        引擎基础类型 + TeX VM 数据模型 + 展开引擎 + 字节码 VM（RFC-1 / RFC-4）
  ntex-layout      排版核心：主循环（模式状态机）/ 折行 / 断页 / 数学 / 输出例程（M3/M4）
  ntex-font        TFM 解析与真实字体度量（M3-4；ttf/HarfBuzz 待 M9）
  ntex-dvi         DVI 写出器，与真实 TeX 逐字节一致（M3-5）
  ntex-pdf         DVI → PDF 正式后端：Type1 字体嵌入
  ntex-io          VFS 抽象 + LocalVfs/MemVfs（RFC-3 副作用隔离）
  ntex-format      .fmt v1 状态快照序列化 / 反序列化（v2 待 M7）
  ntex-test-support 测试/差分/基准基础设施（EngineDriver 抽象）
  ntex-trip        TRIP/ETRIP 一致性测试框架
  ntex-diff        差分测试工具（参考引擎 vs 本引擎）
  ntex-bench       基准框架
fixtures/          测试 fixtures（diff 示例 / trip 获取脚本）
scripts/           辅助脚本（如 fetch-trip-fixtures.sh）
docs（RFC）        RFC-1 token 表示 / RFC-4 字节码指令集
```

后续里程碑按计划加入：`ntex-incremental`（增量计算，M5）、`ntex-backend`（Skia/WebGPU，M8）、`ntex-cli` / `ntex-wasm`（M9）。

## 快速开始

```bash
make check      # 质量门禁：fmt + clippy(-D warnings) + 单元测试
make fixtures   # 获取 TRIP 测试 fixtures
make trip       # TRIP 一致性测试（stub 驱动验证管路）
make diff       # 差分测试（示例 fixtures）
make bench      # 基准（stub 驱动验证管路）

# 端到端演示：demo.tex → DVI → PDF（正式后端）
cargo run -p ntex-dvi -- demo.tex     # 排版（TFM / Knuth-Plass / 断页 / \shipout）→ demo.dvi
cargo run -p ntex-pdf -- demo.dvi     # DVI → PDF（Type1 字体嵌入）→ demo.pdf

# 接真实参考引擎
cargo run -p ntex-diff -- --fixtures fixtures/diff --reference external=pdflatex --engine stub
cargo run -p ntex-bench --release -- --driver external=pdflatex
```

## 设计文档

- [idea.md](idea.md) — 总体架构构想（三阶段解耦 / 增量计算 / 并行布局）
- [plan.md](plan.md) — 实施计划（M0~M9 里程碑与验收标准）
- [RFC-1-token.md](RFC-1-token.md) — Token 表示与内存布局（8B tagged union）
- [RFC-4-bytecode.md](RFC-4-bytecode.md) — 字节码指令集设计（宏展开 VM IR）

## 里程碑状态

| 里程碑 | 内容 | 状态 |
|---|---|---|
| M0 地基 | workspace / CI / 基准 / TRIP / 差分工具链 | ✅ 完成 |
| M1 内核 | Token/InternTable/eqtb/catcode/扫描器/展开引擎 | 🟡 核心完成；**TRIP 冲刺推进中**（扫描/字体/数学错误恢复已落地，逐段攻剩余恢复点） |
| M2 字节码 | 定长 u64 IR + 编译器 + 双轨等价 | 🟡 双轨 100 用例等价全绿；吞吐 1.12x 未达 2x，arena 未做 |
| M3 排版 | 折行/TFM/断页/lig+kern/`\output`/shipout→DVI | ✅ 核心完成（DVI 逐字节对照一致）；VFS + `.fmt` v1 已落地 |
| M4 数学 + e-TeX | 数学模式/e-TeX 原语/断字/错误模型 | ✅ 完成；**ETRIP 冲刺进行中**（A 组 41/42、B 组 36/36、C 组已接线） |
| 输出端 | DVI→PDF 正式后端（Type1 嵌入） | ✅ 可用（dvipdfmx 渲染一致） |
| M5+ | 增量 / 并行 / .fmt v2 / 渲染 / 生态 | ⏳ 待实施 |

## License

MIT OR Apache-2.0
