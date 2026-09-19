# samples/ — 人工样张（示例文档与故障现场）

> 本目录收纳**人写给人看**的 TeX 样张：端到端演示文档、里程碑验收样张、以及真实故障的
> 现场文档。与 `fixtures/` 分工不同——`fixtures/corpus/` 是批量探针的**分母**（机器对照
> 语料），本目录是**手敲一条命令就能跑起来**的样本。

## 怎么跑

`ntex-dvi` / `ntex-backend` 的输出落在**输入文件旁**（`<input>.dvi`、`<prefix>-01.png`），
所以命令一律在**仓库根**启动、把样张连目录一起传：

```bash
cargo run -p ntex-dvi -- samples/demo.tex                        # → samples/demo.dvi
cargo run -p ntex-pdf -- samples/demo.dvi                        # → samples/demo.pdf
cargo run -p ntex-backend -- samples/demo.tex demo 144 --vello    # → samples/demo-01.png…
```

派生物（`samples/*.dvi`、`samples/*.png`；`*.aux`/`*.log`/`*.out` 由全局规则覆盖）已在
`.gitignore` 忽略，**不要入库**。

## 清单

| 文件 | 用途 | 引用方 |
|---|---|---|
| `demo.tex` | 原生 CLI 招牌示例：plain 子集 + 连字/kerning/折行断页全特性，`AGENTS.md` / `README.md` 端到端演示的入口 | `crates/ntex-pdf/src/dvi.rs`（按"demo.tex 实际形态"选字体子集 k） |
| `demo1.tex` | Plain TeX 手写体（**依赖缺口版**）：NTex 无 plain 预载时的原貌 | `docs/plain-format-survey.md` |
| `demo1-fixed.tex` | 上者的 NTex 自举版：5 个手工自举块显式装配字体族与 plain 宏（101 行） | **`crates/ntex-wasm/src/lib.rs` `include_str!`**（回归锁 `self_bootstrapped_plain_demo_compiles_without_undefined_eject`） |
| `demo-cjk.tex` | M9 中文刀 1 验收样张：OpenType 中文字体 + 大码位 `\char`（**刻意不走** UTF-8 输入） | `plan.md` |
| `demo-cjk2.tex` | M9 中文刀 2 验收样张：**源文件直写中文**（UTF-8 输入），与上者对照 | `plan.md` |
| `resume-plain.tex` | Plain TeX 简历真文档，含 XeTeX / pdfTeX / NTex 三分支字体判定（靠 `\ifx\utfinputmode\undefined` 分流） | **`crates/ntex-wasm/src/lib.rs` `include_str!`**；2026-09-11 Tauri 缺字体现场 |
| `resume1-plain.tex` | LLM 手写的"朴素 plain"简历（73 行，夹带 LaTeX 习惯写法），**文件本身保持零改动** | 2026-09-17 CJK 断行/字距现场，见 `docs/KNOWN-SIMPLIFICATIONS.md` |
| `resume1-driver.tex` | `resume1-plain.tex` 的**环境注入驱动**：只补齐运行环境再 `\input` 目标文件（用法见文件头注释） | 与上者配对使用 |
| `plain-cv-sample.tex` | 自造 Plain TeX 简历样本，用 `\sect`/`\job`/`\point` 宏验 bare plain 排版 | — |
| `latex-sample2e-slim.tex` | LaTeX 官方 `sample2e` 的瘦身变体（1144B），专测 `lingmacros`/`tree-dvips` 宏包闭包与 `tabular` 前言的交互 | `crates/ntex-wasm/src/lib.rs`、`crates/ntex-wasm/README.md`、`assets/tex-minimal/README.md` |

## 注意

- **`include_str!` 是编译期硬路径**：`demo1-fixed.tex` 与 `resume-plain.tex` 被
  `ntex-wasm` 编进二进制（`crates/ntex-wasm/src/lib.rs` 的
  `include_str!("../../../samples/…")`），改名或移动会**直接编译失败**。
- 其余样张在源码/文档里只是**散文式提及**（按文件名），移动不需改引用方。
- **中文样张需要字体**：`demo-cjk*.tex` / `resume-plain.tex` / `resume1-*.tex` 依赖
  Fandol Song（`~/.ntex-fonts/FandolSong-Regular.otf`，或经 `NTEX_OTF_DIR` 指定），
  缺字体时回落 Computer Modern、中文不可见。
- `resume1-driver.tex` 里的 `\input` 路径**相对仓库根**（`samples/resume1-plain.tex`），
  与上文"在仓库根启动"的口径一致。
- 迁移历史：2026-09-19 自仓库根目录迁入。其中 `latex-sample2e-slim.tex` 原名
  `latex1.tex`、`plain-cv-sample.tex` 原名 `cv.tex`（两者曾短暂暂放 `probes/`）。
