# NTex 整体进度

> **本文件是全仓唯一的进度文档**（2026-09-19 重构）。
> 任何"做到哪了 / 还差什么 / 下一步做什么"只信这里，不再散落在多份文档。
>
> | 你要找的 | 去哪 |
> |---|---|
> | 目录结构、crate 职责 | [AGENTS.md](AGENTS.md) |
> | 构建命令、开发约定 | [README.md](README.md) |
> | 架构决策（已定稿） | [RFC-1](RFC-1-token.md) / [RFC-3](RFC-3-side-effects.md) / [RFC-4](RFC-4-bytecode.md) / [RFC-5](RFC-5-parallel.md) |
> | 架构构想（愿景） | [idea.md](idea.md) |
> | 技术债明细 | [docs/KNOWN-SIMPLIFICATIONS.md](docs/KNOWN-SIMPLIFICATIONS.md) |
> | 定位工具与判读纪律 | [docs/tooling-trust.md](docs/tooling-trust.md) |
> | 历史逐刀战报、勘察全文 | [docs/archive/](docs/archive/README.md) |
>
> **维护纪律**：进度变化只改本文件；一刀闭环后，逐刀细节移入 `docs/archive/`，
> 正文只留结论 + 归档指针（归档文件按原名保留，便于全文检索）。
> README（[英文](README.md) / [中文](README.zh-CN.md)）的「开发进度」节是**速览快照**，
> 里程碑状态或关键数字变化时一并刷新，并更新节首快照日期。

---

## 0. 一页速览

### 0.1 当前焦点：LaTeX/expl3 兼容战役（唯一主线）

**目标**：`latex.ltx` 加载成功 → `latex.fmt` 构建 → `\documentclass{article}` → PDF。

| 阶段 | 状态 | 关键数字 |
|---|---|---|
| plain.tex 预载 | ✅ G0–G3 完成 | 1241 行全通；`\newif` 端到端与 pdfTeX 一致；剩 G4（`\lccode/\uccode` 初表）、G5（初表分裂脑收敛） |
| expl3 全文载入 | ✅ 载入走通 | `[LOAD-DONE]` + DVI 落盘；载入期错误 2492 → **4**（NTex 独有仅 1，不阻断） |
| latex.ltx 主体加载 | ✅ **主墙已越（口径刷新 2026-10-02）** | 发行 latex.fmt 全链（latex.ltx + expl3 + NFSS 字体）可用；`\documentclass{article}` 文档端到端出 DVI（见下行）。initex 逐行推进口径：fixtures 闭包补齐后 `blocker-track` 最远 **l.36005**（expl3-code 载入段，`扫描到输入末尾`；09-12 旧口径 1147 是缺件伪影），非当前主战场 |
| `\documentclass` / article.cls | ✅ 最小闭环已通 | **主控实测（fae2350，2026-10-02）**：small2e **0 错** 1 页、sample2e **0 错**（原 Missing number ×2，第十九刀 `scan_dimen` mu 参数内部量臂 + 正号小数臂清零，b497294 同日先修 `\read` 未开流臂）、testpage `\read` 未开流已对齐 GT（Emergency stop 语义，GT nonstop 同 fatal，销账）；transformer-standalone **18 页/87 字体/49245B 与基线一致**、40 错（hypertext/URL 域字符为主） |
| 结构宏（`\maketitle`/`\section`）+ NFSS 字体 | 🟡 主体已通 | `\section`/`\maketitle`/`\LaTeX` 徽标/字号代换链已通（09-24/25 现场四连）；余 NFSS 长尾（`\ifdim` size range、`try@simples`）见 §5 P1 |

**最近两刀**（2026-09-17 / 09-18）：

- **第十八刀** 尾递归鞍具校准 → `make check` 797 → **800 全绿**；pdfTeX 对照证明原否定结论有误；
  **真实主墙仍未越过**（输入栈泄漏与 Call/Ret 均已证伪）。
- **第二十五刀** `\the\value{counter}` 操作数宏展开 → corpus `\the` 同源错误群清零
  （`small2e` `^!` 7→4、`lppl` 60→31、`sec1` `\the` 错 1→0）；`ntex-core` 435 passed。

**现场四连**（2026-09-24 / 09-25，全部有 GT 逐字/DVI 对照）：

1. **EC「TC」TS1 字形通道**（`e87321b`）：`\thanks` 脚注标记（`\textasteriskcentered`）
   走 TS1/`tcrm*`，此前预览方框 + 工作台 `Font … not loadable`。修：`ts1_to_unicode`
   （槽位锚定 `ts1enc.def`）+ EC 四位数尺寸名归档 + `assets/tfm` 随包 5 族度量；
   残差见 [docs/KNOWN-SIMPLIFICATIONS.md](docs/KNOWN-SIMPLIFICATIONS.md) §5。
2. **`cur_font` 组回滚**（2026-09-25）：tex.web 的 `cur_font_loc` **是 eqtb 字**（随组保存），
   NTex 的 expander 镜像是裸字段 → `\LaTeX` 徽标内层 `\fontsize\sf@size\z@\selectfont`
   的字体泄漏出组，第二个徽标起的 `\kern-.36em`/`\kern-.1667em`/`\lower.5ex` 全按内层
   字体的 quad/x-height 算——用户看到"LaTeX 的 a 位置不对"。修：`SavedValue::CurFont`
   （排版侧 `NodeBuilder` 早有 `font_stack`，缺的只是 expander 这一半）。验收：用户样张
   DVI 与 pdfTeX 逐指令一致（±1sp 取整），文档纵向节奏同时回正 ~13pt；回归锁
   `ntex-core::expand::tests::cur_font_is_saved_and_restored_by_groups`（去修复必红）。
3. **工作台字形覆盖面**（2026-09-25）：`ui/fonts` 只覆盖手挑的少数 CM 尺寸 → 用户两条
   现场（`\LaTeX` 徽标 A 灰方框 = `cmr8`；`$E=mc^2$` 不渲染 = `cmmi12`+`cmr8`）。
   修：按 `ot1cm*.fd`/`om*.fd` 声明的名字矩阵补 33 个 LM OTF、`GLYPH_FONTS` 重写
   （70 名）、`lm_file_name` 补 `lmromanslant`/`lmromancaps`/`lmmono*-italic|slant|caps`/
   `lmromanunsl`/`bolditalic`/`lmsans*-oblique|bold`/`lmsansdemicond` + 数学粗体与大字号族
   （顺手改对 `cmsl` 的错名：LM 里的文件叫 `lmromanslant*`，不是 `lmroman*-oblique`）。
   两面由 `ui_font_manifest_matches_rust_claim`（名单↔映射）与
   `workbench_manifest_covers_fonts_used_by_latex_docs`（文档级：探针文档用到的每个
   字体都有轮廓 + 度量）钉住；遗留见 KNOWN-SIMPLIFICATIONS §5。
4. **`\limits`/`\nolimits`（limit_switch）**（2026-09-25）：`\[ \int_0^1 \frac{dx}{e^x} \]`
   的上下限堆到积分号**上下左侧**（用户截图"积分渲染有问题"）。`\int` = `\intop\nolimits`
   （tex.web：op_noad subtype 0=normal/displaylimits、1=limits、2=nolimits；make_op 只在
   subtype=1，或 subtype=0 且 display 样式时堆叠），而 NTex 的 make_op 只看"是否 display"
   → `\nolimits` 被忽略。修：core 新增 `Sink::math_limit_switch`（limit_switch 原语）→
   layout 记入 `MathChar.limits` → make_op 分派，nolimits 走 tex.web 的 make_scripts
   （副标在算符右侧，含斜体修正 delta 的水平偏移）。验收：`intg.tex` DVI 上下标 x 偏移与
   GT 逐点一致（sup +655361、sub +364090 sp）；回归锁
   `ntex-layout::typeset::tests_math::op_limit_switch_controls_script_placement`
   （用 `\mathchardef` 原语构造，不依赖 plain 预载；去 limits 判定必红）。2026-09-26
   续批补齐 [docs/breadth-2026-09-23.md](docs/breadth-2026-09-23.md) #25/#26：
   display style 分支加回归锁（`\sum` displaylimits 堆叠、`\intop\nolimits` 仍右侧脚本位），
   `make_op` 大算符盒宽纳入 italic correction，裸 `\displaystyle\int` 约 10pt、有下标
   nolimits 时扣回约 5.55557pt。
4. **Tauri PDF 与预览字体同源**（2026-09-25）：预览走 LM OTF，PDF 另抓只覆盖 plain
   47 件的 CM PFB；LaTeX 新字号/TS1 请求 404 后仍导出裸 `/BaseFont`，查看器替代导致
   样式变化、无替代字形时直接丢字。修：`set_glyph_font` 同时向 PDF 登记 OTF + 预览
   同源 slot→Unicode 表，`ntex-pdf` 给 TFM 8-bit 字体新增 slot→CID 的 Type0/CFF 写出，
   前端对已注册 OTF 不再 fetch PFB。实链验收：wasm 导出 `cmr10/cmmi10/cmr7` 三族均
   `CID Type 0C / embedded=yes`，Poppler 144dpi 渲染正文、数学、上标全部可见；回归锁
   `write_pdf_reuses_preview_otf_for_tfm_8bit_font` + `registers_8bit_slot_map_with_font_bytes`。

**fmt 快照重建刀**（2026-09-20）：真实论文（arXiv 1706.03762 standalone）`a\_b` 死循环根因 =
**陈旧 fmt 快照**——`assets/fmt/latex.fmt` 建于 09-19 00:33，早于 `883b9b3` active 槽隔离；旧槽
模型下 latex.ltx L15916 `\gdef_{\_}` 把 active `_` 定义写进同名单 cs `\_` 槽，robust 壳被覆盖成
自引用体（`\meaning\_` = `macro:->\_`，3700 万步不收敛）。引擎侧 `exec_def` 的 `active_slot`
臂已正确（plain 路径探针实证），重生成 fmt 即愈：最小复现文本/数学模式出 DVI，
`\copyright`/`\sqrt`/`\textunderscore` 无回归；paper 推进到 l.342 `tabular`（`\if 缺 \fi`，
下一堵墙，halign 域）。重建曾被宿主依赖卡住：`\InputIfFileExists{hyphen.cfg}` 命中宿主 TinyTeX
babel 配置（引用宿主缺失的 dehypht-x-2024-02-28.tex）→ 断字配置已钉进发行树（`hyphen.cfg`
垫片转投官方回退 `hyphen.ltx` + Knuth `hyphen.tex` 入库），默认 `--generate-fmt` 从此确定性。
**流程教训：引擎语义提交后必须重生成发行 fmt**（README「引擎语义变更后重生成发行 fmt」本次漏执行）。
新露头残差（登记未修）：auxiii `\ifx\reserved@a\reserved@b` 判 NE（pdfTeX GT 判 EQ），robust
壳缺 `\x@protect ⟨cs⟩` 前缀（嫌疑 286eddc \meaning 语义变更）——\write 文本面偏差，非阻断。

**新墙（只登记未修）**：`\ifdim` 的 NFSS size range 解析 ——
`Missing number <to be read again> \@M`（裸 `\show\@M` = `\mathchar"2710`，`\number\@M` = 10000）。

**expl3 官方套件口径**（l3kernel `.lvt` × 187）：载入终点 **l.36005 = 89.4% 行 / 90.0% 字节**，
载入期真错 573 → **1**（不阻断）；墙 = `\__codepoint_finalize_blocks_aux:n` 的 `\__int_step:Nw` 块循环。
**✅ 2026-10-04 墙拆除（四刀连环 e080686）**：①`\ifeof` EOF 尾行判定（8c75995）②else 分支
尾递归输入栈帧消解（025784d，34931 行读循环）③scan_int EOF 探测误报 fatal——
`try_scan_backquote`/`peek_csid` 的 fetch(None) 改交 Missing number 恢复（beffd0f）。
发行 fmt **v23 已生成入库**（assets/fmt/latex.fmt，9082372B），`--generate-fmt` 全链复活。
expl3 载入终点已越过 l.36005（exgeneric 过、backend def 链通）。
**✅ 2026-10-09 载入闭包补件 + dumped=true（76d11bb）**：fixtures 缺件链补齐
（5 enc.def + 41 base/*.fd + hyphen/zerohyph + utf8 系，51 件）——latex_probe
`pass1 OK, dumped=true`，载入终点 **l.38446**（l3text-map 段，89.4%→95%+）。
SurveyVfs read() mem 优先（写后读一致）。廿七刀证伪 chardef 假说但落 3 把
回归锁（fd50153）。残 1 错 Missing number @38446（01F0 mapping，带病可恢复；
纯 expl3 场景不现，依赖 latex.ltx 前置语境，另案）。
剩余 10.6% 行 = codepoint 收尾 / l3text 五段 / l3legacy+l3deprecation / 文件尾 / exgeneric 尾。

### 0.2 各线状态

| 线 | 状态 |
|---|---|
| 主线：LaTeX/expl3 兼容 | ✅ 主墙已越（2026-10-02 口径刷新）；`\documentclass` 最小闭环已通；余长尾见 §0.1/§5 |
| 输出例程战 | ✅ 刀 1–5 全落（`\outputpenalty`/box255/insert 分配/`\newinsert`/页号链） |
| 格式预载（G 线） | ✅ G0–G3；剩 G4/G5 |
| M5 增量计算 | 🟡 阶段一~五完成（改正文 4.3x / 改宏体 1.1x）；阶段六待办见 §2 M5 |
| M9 中文 | 🟡 刀 1/2/3/5 落地（`\char` 扩位 + `\utfinputmode` + 宿主端端到端 + CJK 断点/断字最小宽）；遗留见 §5 P1 |
| M2 字节码 | 🟡 双轨等价全绿；吞吐 **1.01x**，≥2x 结构性不可达 → 转 M7 `.fmt` v2 |
| 渲染/预览端 | ✅ PDF 后端 + 软光栅/vello + studio + wasm/tauri 工作台可用；真字形与缩放等遗留见 §2 M8 |
| 技术债 | 见 §5；明细 [docs/KNOWN-SIMPLIFICATIONS.md](docs/KNOWN-SIMPLIFICATIONS.md)（待做 33 处 / 已修 ✅ 29 处 / 部分 ⚠️ 8 处，按行计，最近维护 2026-09-18） |

### 0.3 KPI 与口径登记

- **corpus 探针**：⚠ **两处口径冲突，已登记待对齐** ——
  本节此前记 **5/8 PASS + 3 EMPTY**（2026-09-17）；2026-09-19 在 `b08d77d` 后实测为
  **2/8 PASS + 0 EMPTY**（PASS = `math/basic-expressions`、`math/symbols-matrix`；
  plain 三例从 EMPTY 转 FAIL：仍缺 `\shipout` 类债；`small2e` 90s 超时）。
  待补一次对照说明；在那之前**以 09-19 实测为准**。
- **corpus 的 LaTeX PASS 是降级渲染**（`\documentclass` 等全 Undefined，只排出裸文字），
  真 LaTeX 结构/字体/版式未通 —— 不要把它读成 LaTeX 可用。
- **TRIP / ETRIP**：硬口径"逐字节全绿"已按 2026-09-03 决策解绑，改记 M8 L2 观测项
  （见 §2 M4）。当前收尾口径 = semantic diff 归零 + 错误块抽查。
- **EVIDENCE 口径**：任何数字入库须带 commit / 命令 / 样本规模；仪器失真史见
  [docs/tooling-trust.md](docs/tooling-trust.md)（开工定位前先读）。

### 0.4 下一步（按执行序，2026-10-02 口径刷新）

> 主墙 88.4% 已越、`\documentclass` 最小闭环已通（见 §0.1），本节旧序作废重排：

1. **`\read` 未开流恢复臂** ~~（testpage 卡死点）~~ ✅ 已修（b497294，第十八刀）；
   testpage 销账：GT nonstop 下同 fatal（交互式版面询问文档）。
2. ~~**sample2e `Missing number, treated as zero` ×2**~~ ✅ 已清（fae2350，第十九刀）——
   根因非 NFSS：`scan_dimen_body` 缺 `\thinmuskip` 系内部量臂 + `+`/小数正号前瞻臂
   （LaTeX `\,` = `\tmspace+\thinmuskip{...}`/`\kern+.16667em` 级联）。余 NFSS 长尾
   （`\ifdim` size range / `try@simples`）未动，撞上即做。
3. ~~**transformer-standalone 40 错清账**~~ ✅ 销账（2026-10-02 GT 裁决）：40 错全部为
   「原文档对 GT 也不合法」构造——①23× Undefined（`\multirow`/`\specialrule`/
   `\cmidrule` 等，standalone 简化件没 `\usepackage{booktabs,multirow}`，GT 同报）；
   ②15× Missing number/Illegal unit（`\newlength` 声明的 **skip 寄存器**裸用于数学
   维度/上标语境，GT 同构复现 3 错同型；真 TeX 语义即如此）；③2× Limit controls
   （`\_` 非算符，GT 同病）。**引擎错误构成已与 GT 对齐，无引擎刀**；hyperref 评估
   转宏包生态闭包时做。
   **2026-10-03 复核**：booktabs/multirow 入 assets 后重跑仍 40 错——与预期一致
   （样张源头缺 `\usepackage{booktabs,multirow}`，非引擎缺口；样张补包名属改原文，
   不做）。
4. **宏包生态闭包**（最大体量项）：geometry/amsmath/hyperref/biblatex/tikz/ctex
   CI 回归集 + ntex-pkg 引擎接线（缺包即报 + 一条命令补）。
   **✅ 引擎接线（3f09883，2026-10-09 卅一刀）**：`--auto-pkg` 预处理路线——
   typeset 前 quick-scan 包名单，VFS miss 项走 ntex-pkg resolve→SourceChain
   （本地 TL 树→tlnet）取料注入；miss 报错含包名+反查指引。实测 enumitem
   零手工备料自动载入 0 错出 DVI。auto_pkg_cli 集成测试 2 passed。
   **刀 1（b81b587，2026-10-03 第二十一刀）**：booktabs ✅（本体零债）+ multirow ✅
   （`scan_glue` 补 countdef'd cs int_val 落穿臂——`\advance\myskip \mycnt\mydim`
   count 系数×dimen 内部量乘积；tex.web L9094 scan_glue S 分支，无硬报错路径）。
   三线表最小样张 0 错出 DVI。assets 备料已入（booktabs.sty/multirow.sty，9e2e90a）。
   **刀 2（b735923，2026-10-03 第二十二刀）**：`\cmidrule`/`\specialrule` 墙拆除 ✅——
   `\noalign{\ifnum0=`}\fi` 陷阱此前把 halign 组干净关掉（Extra \endgroup +
   ended by）。修：`close_group_and_resume_align`（tex.web L21663 handle_right_brace
   no_align_group 臂）——裁决键=分派时组级，token 级平衡计数不可靠（msg 机器扫描层
   消费组定界符）。b26 cmidrule 真宏 13 错→0 错。三线表全构造（top/mid/bottom/
   **刀 3（bd533ad，2026-10-03 第二十三刀，Codex）**：hyperref 8 个 PDF 原语注册 ✅——
   `\pdfminorversion`（misc int）/`\pdflinkmargin`（dimen 参数）可赋可读；
   `\pdfinfo`/`\pdfcatalog`/`\pdfcolorstack`/`\pdfdest`/`\pdfstartlink`/`\pdfendlink`
   吞参数发 `PDF-PRIMITIVE-STUB` specials。fmt VERSION 22→23（misc 扩容）。
   h1 探针 19→6 错（Undefined 16→7），HYPER-OK 出 DVI。KNOWN-SIMPLIFICATIONS 已登记。
   **遗留债（定性修正 2026-10-03 深夜）**：fmt 生成停 v22 **非回归**——v22（09-27
   生成）时 latex.ltx 载入墙在 88.4%（早于 l.36005），\dump 前根本没走到 codepoint
   段；ctex 战役把载入推过 l.36005 后才暴露 **expl3 官方套件 89.4% 墙**（§0.1 已
   登记的 \__codepoint_finalize_blocks_aux:n \__int_step:Nw 块循环）= fmt 生成与
   latex_probe 同一堵墙。修复路径 = 既有战役目标（expl3 codepoint 装载段），
   非 fmt 链语义破坏。**影响不变**：fmt 停 v22 期间，新原语作业走无 fmt 慢路径；
   h1 残 6 错 = v22 回落 plain 后的结构宏 Undefined（非本刀域）。
   **✅ CI 回归集落地（刀D，2026-10-10）**：`scripts/pkg-regression.py` + `fixtures/pkg-regression/`
   13 包矩阵 + 基线文档入库；**刀E/F/G 三连修复后 12/13 PASS**（xcolor/graphicx/geometry
   全部转绿，仅剩 microtype——pdfTeX protrusion 原语缺口 rpcode/lpcode/pdfmatch/
   pdffontexpand，FAIL(1128) 但整篇跑通出 DVI，文本层 80.4%）。刀G（0c38ca4）：
   `\ifdim` 谓词扫描缺宏展开臂（trig `\TG@@sin` 递归收敛全炸的根因）；刀E
   （dc89ccb+4a988dc）：scan_dimen 数字循环缺展开臂 + 数量×内部量截断 +
   format_dimen 舍入 + preamble `\rshift` 泄残流误报；刀F（23c7a2d）：`^^` 置换
   产物缺 reswitch 重分派（microtype `^^Q`/`^^X` 引擎条件编译失效 → 自引用定义
   → edef 栈超限）。
   13 包矩阵双跑（ntex --auto-pkg vs pdfTeX GT），PASS 9/13；基线报告
   `docs/pkg-regression-baseline.md`（重跑逐字节幂等）。非 PASS=geometry 1 错、
   xcolor 10 错、graphicx 40 错（均出 DVI）、microtype 719 错（无 DVI）；
   caption 缺包负例验「缺包即报+反查指引」契约 ✅。
5. 并行线：M1-13 错误恢复通用机制收口、M5 阶段六、M7 `.fmt` v2
   （发行 fmt 载入仍分钟级，产品化前必修）。

---

## 1. 里程碑总览

| # | 里程碑 | 核心交付 | 状态 | 验收标准（Exit Criteria） |
|---|---|---|---|---|
| M0 | 地基 | workspace / CI / 基准框架 / TRIP 框架 | ✅ 完成 | 基准脚本与测试框架跑通 |
| M1 | 解释器内核 | Token + 16 catcode + 宏展开 + 核心原语 | 🟡 核心完成；TRIP 未归零 | **TRIP 通过** |
| M2 | 字节码编译 | 字节码 IR + 编译器 + 执行器 | 🟡 双轨全绿；吞吐未达标 | TRIP 仍绿；吞吐 ≥ 解释器 2x（实测 **1.01x 未达，结构性**） |
| M3 | 排版核心 | 节点/盒子/胶水/Knuth-Plass/TFM/断页/DVI | ✅ 核心完成 | DVI 与真实 TeX 逐字节一致 |
| M4 | 数学 + e-TeX | 数学模式 / e-TeX 原语 / 断字 / 错误模型 | ✅ 功能完成；ETRIP 收尾中 | **ETRIP 通过**（逐字节口径已降级，见 §2 M4） |
| M5 | 增量计算 | 求值图 / 依赖追踪 / 副作用隔离 / VFS | 🟡 阶段一~五 | 改 1 处仅重算受影响段落（目标 100x；实测 4.3x/1.1x） |
| M6 | 并行 | 段落级并行展开 / 布局 / 整形 | ⬜ 未开始 | 8 核加速比 ≥ 3x（300 页基准） |
| M7 | `.fmt` v2 | 三层 fmt + 部分求值 + 项目级 fmt | ⬜ 未开始（v1 已可用） | `latex.ltx` 加载 <200ms；ctex 300 页冷编 <10s |
| M8 | 渲染 / 输出 | PDF 后端 + GPU 渲染 + WASM 预览 | 🟢 主体可用 | 简单文档 L2 逐字节一致；WASM 增量预览流畅 |
| M9 | 生态冲刺 | ctex/xeCJK/OpenType/CJK 捷径 | 🟡 中文刀 1–5 落地；宏包管理解析/契约层落地（`ntex-pkg`） | 目标宏包 CI 回归全绿 |

**依赖链**：M0 → M1 → M2 → M3 → M4 → M5 → M6 → M7 → M8 → M9
（M5 依赖 M2/M3；M7 依赖 M2/M5；M6 可与 M5 部分并行）。
**北极星对照**：M5/M7/M8/M9 + LaTeX 兼容战役共同收口四项用户指标（见 §3）。

---

## 2. 里程碑明细

> 只记"当前状态 + 剩余项"；已闭环的实施细节在 `docs/archive/plan-full-2026-09-19.md`。

### M0 地基 ✅

- workspace / CI（`.github/workflows/ci.yml`：门禁 job + 观测 job）/ 差分工具 / 基准框架
  全部就绪。
- **RFC 状态**：RFC-1（Token 表示）、RFC-3（副作用模型）、RFC-4（字节码）**已定稿**；
  RFC-5（并行）在册；**RFC-2（不可变状态模型）未单独成文**——其内容已由 M5 实际落地
  （可回滚检查点 + 盒子寄存器写时复制），独立文档仍待补。
- **基准集**：已建"每千 token 展开吞吐"；未建——`latex.ltx` 加载耗时（M7 前无意义）、
  300 页中文冷编（M7 前）、增量"改 1 字重算耗时"（M5 前）。
- **剩余**：RFC-2 成文（低优先）。

### M1 解释器内核 🟡

核心（M1-1~7、M1-9~12、M1-15）已实现。逐项现状：

| 项 | 状态 |
|---|---|
| M1-1 Token 骨架 / M1-2 InternTable / M1-3 eqtb 槽 | ✅ |
| M1-4 输入与 catcode 固化 / M1-6 展开主循环 / M1-7 扫描顺序原语 | ✅ |
| M1-5 宏定义（`\def`/`\edef`/`\gdef`） | ✅（`\newcommand` 引擎侧未内建，LaTeX 宏层自备） |
| M1-8 参数匹配 | ✅ 无分隔 + 分隔均实现；`\long` 前缀已注册接线 |
| M1-9 条件原语 / M1-10 寄存器 / M1-11 组作用域 | ✅（M1-11 朴素快照回滚未改 eqtb 版本指针，O(1) 优化未做） |
| M1-12 模式状态机 | ✅ 由 `ntex-layout::typeset` 承接 |
| M1-13 错误模型 | 🟡 交互模式 + 上下文行 `l.N` 已做；**错误恢复通用机制（`back_input`/`\errhelp`）待补** ← P0 |
| M1-14 TRIP 冲刺 | 🟡 `trip.tex` 可全程跑完不 panic；semantic diff **−5619/+1412**，未归零 |
| M1-15 性能基线 | ✅ |

> 历史标注澄清：`\long`（M1-8）/`\chardef`（M1-3）/`\box`、`\muskip`（M1-10）
> 代码**均已实现并注册**，旧"待补/未实现"标注是文档滞后，勿再按旧标注排期。

### M2 字节码编译 🟡

- 已落地：定长 u64 IR（`Emit{token}` / `EmitArg{n}` / `End`）、定义期编译器、执行器、
  双轨等价框架（**100 用例等价全绿**）、原语 dispatch 表、字节码成为默认路径
  （`Expander::new()`，解释器退为调试工具）。
- **可选优化项已撤销**：常量条件折叠（A6 —— TeX 的 `\if*` 是展开期求值，`\let\iftrue\iffalse`
  会使折叠产物与运行期语义不符）。
- **未做**：M2-5 arena（token/指令 bump 分配）、`\expandafter`/`\futurelet` 专用指令、
  编译失败路径。
- **吞吐结论（2026-09-03，release，`ntex-bench expand-dual`）**：比值 **1.01x**，
  未达 2x 且属**结构性**——RFC-4 零解包 IR 与解释器 `TokenArray` 完全同构
  （均为 8B/token），执行器本体 <10%，Amdahl 上限远低于 2x。全管线
  `expand-throughput` 469ms / 42.6 万调用/s。**≥2x 需改 IR 设计或降共享成本 → 转 M7。**

### M3 排版核心 ✅

- 节点 / 盒子（`\hbox`/`\vbox`/`\vtop`）/ 胶水与 badness / Knuth-Plass 折行 /
  TFM 解析与字体表 / lig+kern + `\sfcode` / 断页 DP / `\output` 例程 + box255 /
  `\shipout` → DVI —— **全部完成**；`dvipdfmx` 实机验收，DVI 与真实 TeX 逐字节一致
  （残余差异仅排版器未实现字体 kern 表）。
- **RFC-3 VFS + 副作用模型**落地：10 个读写原语走 `ntex-io` VFS，`\write` 延迟到 shipout
  边界提交，`\write18` 拒绝。
- **`.fmt` v1** 内存快照（`ntex-format`：确定性编码 + roundtrip 测试）；mmap 零拷贝留 M7。

> `badness` 重复实现已合并；折行 active 集淘汰 + `\tolerance` 默认值 200 为评审 A1 修复项。

### M4 数学 + e-TeX ✅（功能）/ 🟡（收尾）

- **功能全落**：数学模式状态机（`$`/`$$`、8 类原子、spacing 表、上下标、字阶）、
  分式/根式/定界符、样式原语、fontdimen 数学参数 + 数学字体族、显示数学细化、
  Liang 断字 + `\patterns`、数学错误模型、e-TeX 核心与扩展
  （`\protected`/`\ifdefined`/`\ifcsname`/`\unless`/`\numexpr`/`\detokenize`/`\unexpanded`/
  `\dimexpr`/`\glueexpr`/`\ifprimitive`/`\scantokens`/`\eTeXversion`）。
- **ETRIP 原语**：A 组 **42/42 ✅**、B 组 **36/36 ✅**、C 组**全部已接线 ✅**
  （原语级明细见 [docs/archive/ETRIP-primitives.md](docs/archive/ETRIP-primitives.md)）。
- **收尾口径（2026-09-03 决策，仍有效）**：`etrip.log` 从"逐字节比对"降级为
  **semantic diff 归零 + 错误块抽查**（当前 **−3051/+2352**）；`\showbox`/`\showlists`
  诊断格式逐字节对齐、read-again 与 `l.N` 间的 `...` 省略行、两行光标、79 列断行等
  格式差**不再死磕**，留 M8 L2。
- **剩余 P0**：错误恢复通用机制（与 M1-13 同一项，见 §5）。

### M5 增量计算 🟡（阶段一~五）

铁律：**增量结果与全量重跑逐位一致**，全程由测试锁死。

| 阶段 | 内容 | 提交 |
|---|---|---|
| 一 | 文本级切段 + 段边界轻量快照 + 词法读写依赖 + `edit` 双路径 | `0099b1f` |
| 二 | 可回滚完整检查点 + `edit` 单路径化 + ChainDelta 偏差判定 | `c2a2418`（前置 `7601d1d`） |
| 三 | 段级增量延伸到排版层端到端（`IncrementalTypesetter`） | `b585864` |
| 四 | 排版层依赖判定（宏体编辑省重排）+ 四闸复用判定 | `55bcb09` |
| 五 | 复用判定开销削减（盒子寄存器写时复制）→ **增量墙钟首次 < 全量** | `14f7bc7` |

**实测（release / 120 段文档）**：改正文 **4.3x**（13.4ms vs 58ms）、改宏体 **1.1x**
（30.1ms vs 33ms）。目标 100x。

**阶段六待办**（= M5 全部剩余项）：

1. **检查点捕获增量维护** —— 执行段成本 ≈ 全量段成本（每段多付 ~55µs），
   宏体编辑场景加速比被压到 1.1x 的根因，须在 `ntex-core` 引擎侧解决；
2. **副作用边界** —— `\output` 例程回放 / `\write` 流内容 / `\input` VFS 注入 /
   marks 语义（当前观测到即保守全量重排）；
3. `Expand(source, snapshot)` **纯函数化**收尾；
4. 寄存器/参数**槽级归因**（消除偏差全局失效）；
5. **随机编辑模糊测试进 CI**（增量 vs 全量 diff 常驻，闭环本里程碑风险项）；
6. `.aux`/`.toc` 增量；
7. 内存：现为 O(文档) 量级。

### M6 并行 ⬜

段落级并行布局（rayon）、展开阶段并行（段边界 + 快照隔离 + 无跨段副作用）、
字体整形并行（为 M9 HarfBuzz 铺路）、确定性保证（每段独立快照、按段序合并）。
**暂缓**：等 M5 副作用边界成果落地。验收：8 核 ≥3x 且与单线程逐位一致。

### M7 `.fmt` v2 ⬜

三层 fmt（状态快照 / 预编译字节码 / 预计算索引）、部分求值、项目级 fmt、
mmap 只读 + 页级 COW。**暂缓理由已消解**：M2 吞吐已量化（1.01x，结构性），
不再是"先量化再冲"的阻塞点。验收：`latex.ltx` 加载 <200ms、ctex 300 页冷编 <10s。

### M8 渲染 / 输出 🟢（主体可用）

已落地：

- **正式 PDF 后端**：`ntex-pdf` 直出（DVI 解析 + PDF 1.4 写出 + Type1/PFB 嵌入）；
  与 dvipdfmx 渲染一致（墨水比 1.02~1.04，差异为字体提示/抗锯齿）。
- **渲染后端**：`ntex-backend` —— `Backend` trait + 软光栅 + **vello 0.10 GPU 路径**
  （wgpu 29 无头纹理回读、area 亚像素 AA），共享 prims 遍历可差分；自研 PNG 导出。
- **排版调试 overlay**（盒边界/glue/kern/断点，独立通道不影响正常渲染）。
- **实时预览工作台** `ntex-studio`（eframe/egui-wgpu 0.35 + vello 表面渲染、TeX 高亮、
  250ms 防抖、缩放/平移/翻页、LaTeX/plain 自动切换、Log 转录面板）。
  **依赖硬约束：eframe 0.35 ↔ vello 0.10 恰共用 wgpu 29**（升 0.36 会分裂）。
- **WASM 端**：`ntex-wasm` B/C 档（`compile_document` + `render_page` 软光栅、
  真字形轮廓、中文 OTF 度量+轮廓双侧注入、LaTeX 资产包 `NTEXBND1`）。
- **Tauri 壳**：`ntex-tauri` 纯壳工作台，排版与渲染全在前端 WASM 内；PDF 导出按需
  fetch PFB 注入。
- **字形通道编码覆盖**（2026-09-24）：OT1 + OML/OMS/OMX 之外补 **EC「TC」TS1**
  （`glyphs.rs::ts1_to_unicode`，槽位锚定 `ts1enc.def`；EC 四位数尺寸名归一到 LM
  光学尺寸档）——LaTeX 的 `\text…` 符号与 `\thanks` 脚注标记走这批字体
  （现场：`tcrm1000`/`tcrm0700` 此前落方框）。C 档资产包随之补 `tcrm/tcti/tcbx/
  tcss/tctt` 度量（`assets/tfm`），studio 认领表与工作台前端名单由
  `ui_font_manifest_matches_rust_claim` 钉住同源；残差见
  [docs/KNOWN-SIMPLIFICATIONS.md](docs/KNOWN-SIMPLIFICATIONS.md) §5。

剩余：

- [ ] **L2 字节兼容**：简单文档对照 pdfTeX 逐字节 diff（关时间戳/元数据随机性）——
  TRIP/ETRIP 的逐字节口径也归在此。
- [ ] **预览三端增量接线**：`IncrementalTypesetter` 暴露到 wasm（JS 绑定）/ studio / tauri；
  编辑只重排受影响段，预览刷新 <50ms。前置 = M5 阶段六副作用边界。
- [ ] vello web 后端（wasm 目前只走软光栅）。
- [ ] **SyncTeX 源码映射**（点击跳转 / 错误波浪线）；基础是 RFC-1 side-table 位置信息，
  与 M1-13 错误定位联动。
- [ ] wasm C 档剩余：`plain.fmt` 与数学回归、缩放字号（受 `ntex-core` 字体名扫描既有 bug 拖累）。

### M9 生态冲刺 🟡

**依赖链（严格顺序）**：LaTeX 战役 → ① 英文全链通 → ② 字体子系统（TTF/OTF + HarfBuzz）
→ ③ 输入层 UTF-8 → ④ CJK 原语面（对齐 XeTeX/LuaTeX）→ ⑤ ctex/xeCJK 兼容 →
⑥ 中文 300 页端到端验收。**阶段 A 未出口前不启动中文工作**（避免双线作战）。

已落地（中文刀 1–5）：

| 刀 | 内容 | 状态 |
|---|---|---|
| 1 | `\char` 上界按字体判定 + OTF 度量直映通道 | ✅ |
| 2 | `\utfinputmode`（misc 65）UTF-8 直写通路，默认 bytes 零改动 | ✅ |
| 3 | 宿主端端到端（Tauri/WASM）：内嵌 TFM 48 件 + OTF 度量缝 + 引擎级 UTF-8 开关 + `\fam\bffam` 误判修复 | ✅ |
| 5 | `\cjkbreakmode`（misc 66）CJK 字间断点 + 禁则；中西文交界断点 + `\lefthyphenmin`/`\righthyphenmin` | ✅（XeTeX 对照 oracle + `cmp` 逐字节零回归） |
| 4 | 中文粗体/斜体：FandolSong-Bold / FandolHei / FandolKai 三款全链路（OTF 度量 → Type0 嵌入 → CFF 子集化）；`\bfseries`→粗宋、`\itshape`→楷体由**宏包层伴随字体桥**承接（见 `samples/demo-cjk3.tex`） | ✅（引擎零改动；引擎级 family fallback 未做，见 KNOWN-SIMPLIFICATIONS §5） |

**宏包管理（`ntex-pkg`，2026-09-19 新建）**：边界见 §6.2 第 8/9 条。已落地**解析层 + 契约层 + 取料层 ①②**——
`tlpdb.rs`（TLPDB 状态机解析 + `by_basename`/`by_path` 反查 + TDS 优先级 + `RELOC/` 归一）、`resolve.rs`
（`\usepackage`/`\documentclass`→包解析 + 依赖闭包 BFS）、`lock.rs`（`ntex.lock` 确定性编解码 + 漂移 `diff`）、
`cache.rs`（内容寻址缓存布局 + 名字安全 + **真实 SHA-512 字节哈希**）、
`source.rs`（`PackageSource` trait + ① 本地 TeX Live 树源）、
`tlnet.rs`（② tlnet 镜像：URL 由 `revision` 钉死 + 下载后 SHA-512 校验 + 容器按 `runfiles` 裁剪）、
`vendor.rs`（闭包物化成 TDS 子树；四态逐字节比对，源缺失显式报错并阻塞收敛）、
CLI `ntex-pkg`（`index`/`provide`/`resolve`/`lock`/`check`/`local`/**`vendor`**/**`fetch`**）。
取料层 ③ CTAN / ④ 离线归档仍为**显式未实现插口**（不静默降级）。
验收：解析层真实 TLPDB + `kpsewhich` oracle 9/9；② 协议层离线假边缘全覆盖 + 真实镜像端到端（见待办项）。

剩余清单：

- [ ] ~~中文粗体/斜体（刀 4）~~ ✅（2026-10-01，见上表；遗留引擎级 family fallback 与 FandolHei-Bold，见 KNOWN-SIMPLIFICATIONS §5）；`\catcode` >255 赋值扩展（A5 全量收口）
- [ ] ~~CJK 标点挤压~~ ✅（M9 中文刀 6，2026-10-01：`\cjkbreakmode` 挤压面——行尾闭标点让半格/连续闭标点压半格/汉字邻接源内空格吞掉；zh-paper gap 方差 −95%、最大 5.86→0.62pt，见 KNOWN-SIMPLIFICATIONS §5；遗留行首开括号挤压与 PunctStyle 可配置）；HarfBuzz 整形与整形缓存、**CJK 整形捷径**（无复杂特性时跳过 HarfBuzz 直读 hmtx，目标 5~10x）
- [ ] ctex/xeCJK 宏兼容（从 xeCJK 最小子集起步）；OpenType fontspec 路径
- [ ] 宏包 CI 回归集：geometry / amsmath / hyperref / biblatex / tikz / ctex
- [x] **宏包管理·解析/契约层**（2026-09-19，`ntex-pkg` crate，§6.2 第 8/9 条落地）：
  TLPDB 解析 + 文件反查索引 + `\usepackage`/`\documentclass`→包解析 + 依赖闭包 +
  `ntex.lock` 确定性契约（包名 + revision + sha512）+ 内容寻址缓存 + 可插拔取料源链
  （① 本地 TeX Live 树已实现）。**解析层验收**：真实 TLPDB（2024basic，346 记录）+
  `kpsewhich` 路径 oracle **9/9 一致**；73 单测 + `make check` 全绿。简化登记见
  [docs/KNOWN-SIMPLIFICATIONS.md §9](docs/KNOWN-SIMPLIFICATIONS.md)。
- [x] **宏包管理·取料层 ② + 资产物化**（2026-09-19，同日第二刀）：
  `tlnet.rs`（② tlnet 镜像：`<仓库>/archive/<包>.r<rev>.tar.xz` 由 revision 钉死、按 TLPDB
  `containerchecksum` 做**真实 SHA-512 字节校验**、容器条目按 `runfiles` 裁剪 + `RELOC/`→`texmf-dist/`
  重定位）+ `vendor.rs`（闭包 → TDS 子树物化，四态 `Add`/`Differ`/`Identical`/`SourceMissing`
  逐字节比对，源缺失**显式报告并阻塞收敛**）+ `Vfs::create_dir_all`（宿主侧建目录能力，默认
  no-op 不破平坦后端）+ `tlpdb.rs::install_rel_path`（已安装树与缓存树共用一条路径映射）+
  `tds_rank` 修为**先归一后判档** + CLI `vendor` / `fetch`。
  **验收**：`make check` 全绿（ntex-pkg **92** 单测）；真实镜像端到端（阿里云 CTAN 镜像 + 真实
  TL2026 TLPDB 20.7MB）：`fetch infwarerr` → 1 容器 1 文件 8.2KB（SHA-512 通过）；`fetch
  --documentclass article` → `latex.r79618` 171 文件 2.7MB；`vendor --write --lock` → 目标树
  171 文件、复跑**一致 171 / 新增 0**（幂等）；`resolve --documentclass article` 选定稳定版
  `latex` 而非 `latex-base-dev`（`tds_rank` 修复在真实库生效）。刻意简化（宿主 `curl`/`tar`
  当边缘、无 GPG 验签、无断点续传、缓存布局两套）见
  [docs/KNOWN-SIMPLIFICATIONS.md §9](docs/KNOWN-SIMPLIFICATIONS.md)。
  **2026-09-25 补充**：`assets/tex-minimal` 对该闭包的缺口（新增 116 · 需刷新 7，当日曾登记
  为待产品决策）已按 `\documentclass{book}` 需求落盘补齐——`fetch --documentclass book`
  （同解析到 `latex.r79618`）→ `vendor --write --lock`，仓库根新增 `ntex.lock`（包 latex
  r79618），复跑幂等（一致 171 / 新增 0）；`book.cls` + `bk10|11|12.clo` 随闭包入库，
  `\documentclass{book}` + `\chapter` 文档经 `ntex-dvi` 端到端编译通过（1 页 DVI）。
- [ ] **宏包管理·取料与接线（余项）**：取料层 ③ CTAN / ④ 离线归档 + GPG detached 验签 +
  `\usepackage` 缺包即报（MiKTeX 式一条命令补）↔ 引擎接线，目标机零 TeX Live
- [ ] 引擎身份模拟（`\pdftexversion` 等）
- [ ] **MCP server / AI 工具链**（stdio JSON-RPC，复用 `Typesetter` 库接口 + `MemVfs`）：
  ① tex→PDF（已可起步）② 宏展开/诊断（随错误模型收尾）③ 会话式增量编译（依赖 M5）；
  安全基线 = RFC-3 副作用隔离。验收：`tex→PDF` 经 stdio 端到端可调用。

---

## 3. 北极星：Markdown 级编辑体验（2026-09-06 立项）

**产品总纲**：编辑 LaTeX 像 Markdown 一样简单和高性能。四个用户可感知指标：

| 指标 | Markdown 基线 | NTex 现状（更新至 2026-09-19） | 差距清单（条目引用） |
|---|---|---|---|
| ① 什么文档都能排 | 任意文本即开即排 | plain TeX 子集；`\documentclass` 未通 | LaTeX 战役（§0.1）；`\halign` 精化 / `\insert` / 数学矩阵（§5 P1） |
| ② 打开快（冷启动） | ~0ms | plain 毫秒级；latex.ltx 未加载，`.fmt` v1 不含字体表 | M7 `.fmt` v2（§2 M7）；字体按需备料 + 项目级缓存 |
| ③ 改字快（增量） | 全文重排无感（<16ms） | 改正文 4.3x / 改宏体 1.1x（目标 100x）；预览端仍是 250ms 防抖全量重排 | M5 阶段六全部（§2 M5）；预览三端接线（§2 M8）；M6 并行 |
| ④ 出错不糊 + 零安装 | 语法错误不挡渲染；零环境依赖 | 单点错误终止全文档；目标机需 TeX Live 备料 | M1-13 错误恢复（§5 P0）；SyncTeX（§2 M8）；panic 审计；宏包管理（§2 M9） |

**关键路径**：

```
正确性闭环（指标①④）：LaTeX 战役 + M1-13 错误恢复
  → 增量可见（指标③）：预览三端接 IncrementalTypesetter + 随机编辑 fuzz 进 CI
  → 增量冲高 + 冷启动（指标②③）：M5 阶段六 → 100x；M7 .fmt v2 → <200ms
  → 放大 + 生态（指标②④）：M6 并行；M9（UTF-8/字体/宏包管理/MCP）
```

**验收画像**（四指标齐备即达成）：`\documentclass{article}` 文档打开 <200ms；
改 1 字重排 <50ms 且预览无感刷新；表格/脚注/数学/主流宏包文档全链可排；
任意错误不中断渲染且定位到源码行、目标机零 TeX Live。

---

## 4. 性能 backlog（2026-08-22 评审）

> 不引入新功能，属既定里程碑补课。

**P0 —— ✅ 全部完成**

- 工作区 `[profile.release]`：`lto = "thin"` / `codegen-units = 1` / `panic = "abort"`
  （已确认全库无 `catch_unwind`、无 `unsafe`）—— `80022b4`
- 字节码执行器走 u64 原始字（RFC-4 设计落地）：`Bytecode.code: Arc<[u64]>`，
  `fetch()` 按 `word >> 60` 分发，热路径零解包 —— `80022b4`

**P1 —— 部分完成**

- ✅ 热路径消分配（2026-08-23 / 09-03）：`process_token` 按引用 match（SlotAction 两阶段）、
  `InputFrame::MacroArg` 复用实参 Arc、单 token 回推内联槽（`InputFrame::One`）。
- ✅ 火焰图附带修复：`error_context` 行定位改 `line_starts` 二分（原 O(pos) 全文扫描 +
  整行 UTF-8 转换，长样张 O(n²)，是吞吐被压到 3.1 万 token/s 的主因）；诊断开关进程级
  缓存；看门狗检查由每步改每 64 步。**展开吞吐 +61%，469ms / 42.6 万调用/s 入库。**
- [ ] `call_macro` 实参 `Vec<TokenArray>` → SmallVec（需新依赖，预估收益 <5%）
- [ ] **panic 审计 + fuzz**：生产路径 unwrap/expect（input.rs 10 / ntex-format 15 /
  ntex-io 6 等）改 `Error`，与 M1-13 联动。
  fuzz 目标已建（`ntex-layout/tests/fuzz.rs`，quick 2k 轮入 CI、深 5k 轮 `--ignored`），
  **已修两处违约**（`\the\meaning`/`\the\jobname` 自递归栈溢出；垂直模式行内数学后 `\par`
  空列表 panic），深 fuzz 2 万轮零 panic。

**P2 —— 工程化闭环**

- ✅ CI 门禁（`.github/workflows/ci.yml`）：门禁 job（fmt + clippy `-D warnings` +
  `cargo test --workspace` 含 fuzz quick + stub 冒烟）+ 观测 job（`continue-on-error`，
  真引擎 TRIP/ETRIP/diff/bench + 深 fuzz）；全绿后去掉 `continue-on-error` 转硬门禁。
- [ ] `make bench` 补 `--release`（现为 debug + stub，数字无参考价值）
- [ ] nightly perf job：release 跑真驱动基准，对照入库基线，吞吐降 >15% 即失败
- [ ] token 级微基准引入 divan

**测试纪律提醒**：`[profile.release]` 开 `panic = "abort"` + `lto`，
**`cargo test --release` 会失败** —— 测试一律用默认 dev profile（CI 亦如此）。

---

## 5. 待办汇总

> 全仓唯一待办清单。技术债明细（文件:行）见
> [docs/KNOWN-SIMPLIFICATIONS.md](docs/KNOWN-SIMPLIFICATIONS.md)。

### P0 —— 战役关键路径

- [ ] **P0 刷新（2026-10-02）**：~~latex.ltx 88.4% 主墙根因~~ ✅ 已越（发行 fmt 全链可用，
      `\documentclass` 最小闭环已通，见 §0.1）；余 P0 重排：
- [ ] **`\read` 未开流恢复臂**（testpage 卡死点；pdfTeX 报错可恢复，现 fatal）
- [ ] **NFSS 长尾**：`\ifdim` size range 解析 / `try@simples`（sample2e
      `Missing number, treated as zero` ×2 同源）；
      `preload.ltx` l.47 `\DeclarePreloadSizes` / `\SetMathAlphabet` fontmath 链
      （initex 逐行口径下仍登记，发行 fmt 路径已绕过）
- [ ] **M1-13 错误恢复通用机制**：`back_input` / 插入恢复 token / `\errhelp` ——
      影响**所有报错原语**的"报错后继续"路径；TRIP 卡点根因；契约级（任意畸形输入不 panic）
      **🟡 首刀落地（0f271ee，2026-10-09）**：fetch() 向扫描语境暴露 Source EOF；
      定界参数 EOF → `File ended while scanning use of \<name>` + 插 `\par` +
      废弃坏宏调用 + 父输入继续；`Incomplete \iffalse` EOF 补 `\fi`。
      Transformer 实测 48957B 出页（基线 49245B）。
      **✅ 刀C（c0849ed，2026-10-10）**：`\errhelp` 消费链（interaction 门控 +
      组作用域）+ `\errmessage`/`\message` 实参展开吞错修复（undefined cs 在扫描位
      正常报错不断链）+ error_anchor 跨扫描污染修复（err1 场景 l.5→l.2，与 GT 同构）。
      ntex-core 508 绿含回归锁 2 把。余：`\write` 展开层静默丢弃独立评估、
      half_error_line 32 vs TL2026 50（预存常量分歧）、errorstopmode 停等。
- [ ] **amsmath 后续**：pmatrix `\left(`/`\right)` 垂直模式恢复分歧（NTex 报
      can't use in vertical mode 丢括号 vs GT Missing $ 保留括号——计数同构字形
      分歧，刀A 遗留①，2026-10-10 登记）；输入栈超限消息非 GT 逐字（fetch 护栏
      先于 call_macro 触发，刀A 遗留②）。amsmath 载入本体已 0 错（10b88fb 刀A：
      调用点 collapse_tail_conditionals_for_macro_call 偷走定界实参数据 `\fi`）。
- [ ] **TRIP / ETRIP semantic diff 归零**（−5619/+1412 与 −3051/+2352）+ 错误块抽查
- [ ] ~~`\documentclass` / article.cls → 结构宏 → 真 LaTeX 版面 PDF~~ ✅ 最小闭环已通
      （small2e 0 错 / transformer 18 页与基线一致，2026-10-02 主控实测）；
      余项转宏包生态闭包（§2 M9）

### P1 —— LaTeX 前置能力（撞上即做）

- [ ] `\halign`/`\valign` 精化剩余刀（刀 0 对拍仪器 / 刀 2 preamble 宏展开通路 + S3–S7；
      刀 1/3 已落：`\everycr` 两点注入、align_peek 入口复位、to/spread 摊派）
- [ ] `\insert` **体排版化**：第三十刀已让 insert 体参与断页计账（`\count/\dimen/\skip`
      三联寄存器）并可在输出例程中 `\unvbox\footins` 回流；plain `\footnote*{...}` 最小
      DVI 对拍 1 页通过。余：长脚注 split/holdover 与 `\insertpenalties`。
- [ ] 数学矩阵（`\matrix`/`\eqalign`）—— 勘察结论：无独立引擎战役，
      本体是宏层 + `\halign` 地基，引擎增量仅 `\vcenter` 数学包装验证 +
      `Improper \halign inside $$'s` 检查 + Align 产物盒交 `MathAtom::Box`
- [ ] **P1 长脚注余项**：`\vsplit` marks 拆分（`splitfirstmarks` 等现返回空）与
      `\insertpenalties`。**✅ split 基础面已通（刀B，1f26809，2026-10-10）**：
      长脚注超页样张 0 错 2 页、脚注落第 1 页底（PDF 文本层与 GT 同构）；
      真根因 = `\vfootnote` 通路两扫描缺陷（非 insert 拆分臂缺失——GT 在该
      样张下同样不走 split）。ntex-layout 266 绿。
- [ ] `\output` 例程消费判定改显式 `\shipout\box255`（现为 count 启发式）
- [ ] `\scriptfont` 接真实字体
- [ ] `\catcode` >255 赋值扩展（Unicode catcode 表，A5 全量）+ `\catcode"XXXX=13` 预读时序粘连
- [ ] `.fmt` 快照 misc 数组扩容的版本兼容（MISC_INTS 65→66→67 已两次扩容）

### P1 —— 工程

- [ ] panic 审计（见 §4 P1）
- [ ] 基准进 CI（`make bench --release`、nightly perf job、divan 微基准）
- [ ] M5 阶段六（7 项，见 §2 M5）
- [ ] 预览三端增量接线 + 随机编辑 fuzz 进 CI
- [ ] SyncTeX 源码映射

### P2 —— 未来里程碑

- [ ] M6 并行（等 M5 副作用边界）／ M7 `.fmt` v2 ／ M9 生态清单（见 §2 M9）

---

## 6. 关键决策与风险

### 6.1 已拍板

1. **Token = 8B tagged union**；源码位置走独立 side-table，不进 token。
2. **控制序列 token 只含 csid**，定义一律查 eqtb 槽
   （`Undefined / Macro{version,def} / Primitive / Register / RegisterIndex / Alias`）。
3. **宏体 = 连续不可变 TokenArray + 字节码双表示**；InternTable 线性化，csid = u32 下标，
   `.fmt` mmap 后零字符串查找。
4. **VM 保持纯 token 级**：排版事件经 `TokenSink` 单向流出，VM 不依赖布局 crate。
5. **副作用隔离**：`\write` 延迟到 shipout 边界提交，`\write18` 拒绝，读写流走 `ntex-io` VFS。
6. **输出端两段式**：临时 Helvetica 切片先行已下线，`ntex-pdf` 已改造为正式 DVI → PDF 后端。
7. **渲染后端选型 vello**（2026-09-05，原 Skia）：纯 Rust/wgpu 栈、WASM 同构、免 C++ 绑定。
8. **L1 优先于 L2 兼容**：先语义/折行一致（M4），字节级一致推迟到 M8。
9. **字节码成为默认执行路径**，解释器退为调试工具，双轨等价测试保留至 M7。
10. **CJK 口径**：`\char` 上界按当前字体判定；`\utfinputmode` 默认关（保 TRIP/ETRIP 字节口径）；
    `\cjkbreakmode` 默认关（禁用断点才改折行结果）。
11. **TRIP/ETRIP 逐字节口径降级**（2026-09-03）：验收改为 semantic diff 归零 + 错误块抽查，
    逐字节对齐记 M8 L2 观测项。

### 6.2 待拍板

1. **RFC-1 开放问题 Q2~Q4**：保留位用途 / 内部标记拆分类 / InternTable CoW 粒度。
2. **字节码 vs JIT**：JIT 作 M9 之后的可选加速，不进主线。
3. **RFC-2 是否成文**：不可变状态模型已由 M5 落地，独立 RFC 文档是否补。
4. **CJK 捷径的兼容边界**：跳过 HarfBuzz 的判定条件须保守（M9 单独评审）。
5. **ctex 兼容范围**：先 xeCJK 最小子集，不一步到位全量。
6. **数学输出**：先保 DVI/PDF 逐位一致，MathML 为可选（不阻塞主线）。
7. **MCP 工具形态优先级**：① 排版渲染 → ② 宏展开/诊断 → ③ 会话式增量编译。
8. **资产内置边界**（2026-09-19 讨论，**建议**待拍板——勿全量内置 TeX Live 体量宏包：
   TL2026 scheme-full 无文档 4312MB，现内置 823 条 ≈11.7MB）。建议分三层：
   **T0 引导层**静态内置（预算 ≤50MB，判据「没有它就排不出任何文档」）；
   **T1 精选层**按需落本地缓存（判据「引擎语义面已验证 **且** 已进宏包 CI 回归集」，**内置名单 ≡ 回归名单**）；
   **T2 全量可用层**从不内置（= M9 宏包管理 + M7 项目级 fmt）。必配项：缺包即报 + 一条命令补（MiKTeX 式）。
   离线段仅在离线/内网场景需要，且应为 opt-in 的 medium 级镜像包，非全量。
9. **按需补包的取料源边界**（同日讨论，**建议**待拍板——CTAN 只作回落源，不作接口）：
   对外契约 = 版本化仓库 + lock（包名 + revision + sha512），不等于直连 CTAN。
   **解析层只认 tlpdb**（CTAN 的 `FILES.byname` 无校验和 / 无依赖图 / 无版本号，不能作解析依据）。
   **取料层四源可插拔**：① 本地已有 TeX Live 树（零下载，搜索链已含）→ ② tlnet 镜像（SHA-512 + GPG 签名 +
   revision）→ ③ CTAN `install/**.tds.zip`（TDS 单包；回落场景：tlnet 不可达 / TL 未收录 / 需旧版本）→
   ④ 离线归档包（内网 opt-in）。**禁止**在 `\input`/`\usepackage` 失败时隐式自动联网
   （违反可复现性、安全与证据口径三条）。多镜像 + 离线回落是既有事实要求——本仓库已发生过
   「CTAN 主站与清华镜像被网络策略拦截」（见 assets/tex-minimal/README.md）。
   **落地进度（2026-09-19）**：① 与 ② 已实现（② 的 GPG 验签**未做**，只做 SHA-512 容器校验；
   且 `HostContainerIo` 把 HTTP/TLS/xz 交给宿主 `curl`/`tar`）→ 见 KNOWN-SIMPLIFICATIONS §9；
   ③ ④ 仍为显式未实现插口。

### 6.3 风险与关卡

| 风险 | 关卡（Gate） | 触发时动作 |
|---|---|---|
| TRIP 长期不绿 | M1 结束必须全绿 | 冻结新增功能，只修语义 |
| 字节码与解释器不一致 | M2 双轨 diff 测试 | 禁用字节码路径，回溯 IR 设计 |
| 增量缓存出错（副作用漏追踪） | M5 增量-vs-全量模糊 diff 常驻 CI | 修复依赖登记，不回退功能 |
| 并行破坏确定性 | M6 输出逐位 diff | 收窄并行边界（只并布局/整形） |
| ctex 兼容工作量失控 | M9 按 xeCJK 子集分阶段验收 | 缩减目标宏包范围，先保 ctex 常用 |
| 冷编基准不达标 | M7 验收 | 优先级：深 `.fmt` > CJK 捷径 > 多核 |
| 仪器失真误导定位 | 改诊断原语后必跑 `make instrument-check` | 见 docs/tooling-trust.md 九次事故登记 |

---

## 7. 溯源

| 归档文档（docs/archive/） | 内容 | 冻结日期 |
|---|---|---|
| `plan-full-2026-09-19.md` | 本文件重构前的全文（含各里程碑逐项实施步骤与战报） | 2026-09-19 |
| `ETRIP-primitives.md` | ETRIP 原语逐条状态（A/B/C 组 + M4 基线） | 2026-09-18 |
| `latex-feasibility.md` | LaTeX 战役活文档全文（A0 阻塞点 + A1.* 逐刀 ~30 节） | 2026-09-19 |
| `latex-feasibility-full-2026-09-11.md` | 更早的 §8–§37 逐刀记录全文 | 2026-09-11 |
| `latex-feasibility-relay28b.md` | 第二十八刀接力简报（IPN×256 根因） | 2026-09-06 |
| `expl3-real-scoreboard.md` | expl3 真实跑分与逐刀（第一~二十五刀） | 2026-09-19 |
| `expl3-lvt-scoreboard.md` | l3kernel 官方套件跑分与载入终点复测 | 2026-09-19 |
| `expl3-real-workload.md` | expl3 真实工作量评估（187 例分母辨析） | 2026-09-11 |
| `plain-format-survey.md` | 格式预载可行性勘察（G0–G3 记录） | 2026-09-11 |
| `output-routine-survey.md` | 输出例程勘察（刀 1–5 记录） | 2026-09-11 |
| `halign-survey.md` | `\halign`/数学矩阵战役前置勘察（含 S1–S7 刀序） | 2026-09-07 |
| `trip-missing-primitives.md` | TRIP 缺原语清单（124 → ~10） | 2026-08-28 |
| `primitive-correctness-checktable.md` | 原语语义正确性四层防线盘点 | 2026-09-05 |
| `corpus-probe-2026-09-19.md` | corpus 探针实录（2/8 PASS 原始记录） | 2026-09-19 |
| `REVIEW-2026-08-23.md` | 代码审查（A 正确性 / B 性能 / C 工程 / D 简化点 / F 路线图） | 2026-09-19 |
| `schema-dump.txt` | 早期方案摘要草稿（内容已并入 idea.md / README） | 2026-09-19 |
