// ---------- 方法分片：incremental.rs（M5 阶段三/四：端到端增量排版 + 依赖判定） ----------
//
// # 段落排版缓存 —— 编辑 1 段 → 增量 DVI（plan.md §7）
//
// 阶段一/二的增量停在 expand 层（段输出是 token 流，未接排版）；本阶段把段级
// 机制延伸到排版层，打通 source → 段执行 → 行盒 → 页面 → DVI 全链：
//
// - **段边界 ≈ TeX 段落边界**：[`segmentize`] 把空行 / 行首 `\par` 行归**前段**
//   （`\par` 触发的 close_paragraph 已完成），段边界处排版器处于垂直模式、
//   无进行中段落——正是可检查点的干净状态。
// - **段贡献缓存**：段边界记录排版检查点（[`LayoutBoundary`] = NodeBuilder
//   整体克隆 + 已 shipout 页数）与 expand 检查点（[`StateSnapshot`]）；段执行
//   期间把进入主列表的节点流录下来（[`NodeBuilder::record_main`]，`append`/
//   `push_node` 两个入列路径挂钩）——行盒（Knuth-Plass 折行产物）、段间胶水/
//   惩罚、显示公式盒都在这条流里。
// - **编辑重放**：`edit(k)` 回滚到段 k 前（expand 检查点整体还原 + 排版状态
//   整体还原 + shipped 截断），随后从 k 起重放：
//   * **缓存段可复用判定（同步态）**：`boundary_is_clean` + 活 expand 状态与
//     段前快照精确一致（值状态 + eqtb 逐槽）。成立 = 该段 token 流必然复现，
//     缓存节点流**重新注入主列表**（行盒免重排），页面装配（断页）照常重跑
//     ——编辑造成的页面后移由此自然传播到其后各页；
//   * 复用后把 expand 状态还原到该段**执行后**检查点、排版副作用字段（盒子
//     寄存器/marks/参数镜像/当前字体/断字表…）对齐到边界快照——主列表、页面
//     构建器、已 shipout 页面保留注入重算的结果；并刷新边界快照的排版半边
//     （断页点已变，后续编辑的回滚点必须是新文档的状态）；
//   * 判定不成立（编辑改了宏体/寄存器、段边界脏）→ 该段起照常执行。
// - **铁律**：增量 DVI 与全量 DVI 逐位一致（`incremental_tests` 锁死：多页
//   真实排版文档编辑首段宏定义段/中段/末段/公式列表段 + 连续多次 edit）。
//   全量 `Typesetter::typeset_dvi` 路径零改动，逐位一致由同一测试双口径验证。
//
// ## 阶段四：排版层依赖判定（宏体编辑省重排）
//
// 阶段三的复用判定是逐段全状态**同步比较**——编辑造成状态偏差（改宏体）即
// 其后各段整体重排，收益归零。本阶段把 expand 层的词法依赖机制接入排版层
// （`ntex-core::incremental::snapshot` 公开导出：[`lex_deps`] / [`ChainDelta`] /
// `reads_touch_slots` / `writes_touch_slots` / `snapshot_states_equal`，选导出
// 而非复制——词法超近似的保守延伸规则只有一份实现）：
//
// - **录制**：段执行时录下词法读/写依赖（[`SegmentDeps`]，含 `\csname` 标记）
//   与**状态中性**标记（执行前后 expand 快照语义一致）；
// - **判定**（[`IncrementalTypesetter::reuse_verdict`]）：活状态相对**该段录制时
//   段前快照**取偏差（[`ChainDelta::capture`]，一次全量扫描）——
//   1. 偏差为空 → **同步态**：执行必然复现同一输出，缓存复用且活状态推进到
//      旧 post（阶段三口径不变）；
//   2. 偏差非空 → 四闸（同 expand 层 `cache_reject_reason_inner`）：值状态偏差
//      （寄存器/参数/catcode，未槽级归因）全局失效；`\csname` 保守失效；读依赖
//      闭包触及变化槽（该段引用的宏被改）重排；写集触及变化槽重排（跳过会让
//      本应重写的值停留在被改值）；段非状态中性重排（偏差下不能还原旧 post——
//      会把偏差槽新值冲掉，只有"执行本不改状态"才可跳过）；
//   3. **排版半边闸**：活 `NodeBuilder` 副作用字段 ≠ 段前边界（当前字体/参数/
//      marks/盒子寄存器/断字表…）→ 重排——缓存节点流按旧排版上下文烘焙，上下文
//      漂移则注入结果不可信。全过 → 缓存节点流复用，expand 状态**原地不动**
//      （状态中性 = 执行本不改状态），排版副作用字段对齐旧 post（与执行等价）。
// - **边界刷新**：偏差路径复用后把段后边界刷成**活状态**（expand 半边一并）——
//   旧链快照在偏差下不再是当前文档的状态，后续 `edit` 回滚到它会丢掉偏差槽的
//   新值（宏体编辑 + 连续编辑的逐位一致测试锁定此点）。
// - 判定留痕 [`IncrementalTypesetter::last_rejects`]（expand 层 `last_rejects` 惯例）。
//
// ## 阶段五：复用判定开销削减（增量墙钟翻正）
//
// 阶段四正确性完备但墙钟为负（release、120 段文档：改正文 0.3x / 改宏体 0.2x
// ——增量比全量慢）。逐项剖析（`Instant` 插桩，各环节占总耗时）定位出与直觉
// 相反的分布：逐段 `ChainDelta::capture` 只占 ~2%（30µs/段），大头是**每段
// 整份状态拷贝**——32768 槽盒子寄存器文件随边界克隆/副作用对齐/闸比较每段
// 复制三份（~1.5ms/段）。本阶段三件事：
//
// 1. **盒子寄存器文件 `Rc` 共享 + 写时复制**（[`NodeBuilder::boxes_mut`]）：
//    克隆/对齐退化为引用计数，闸比较走指针相等（[`BoxFile`]）。语义不变——
//    共享期间无人可写，写入前 `Rc::make_mut` 拆出独享副本。
// 2. **链偏差跨段携带**（expand 层 `SegmentEngine::replay` 同款）：偏差只在
//    **执行段之后**重算一次（参照 = 该段**旧** `record_post`，与下一段旧
//    `record_exp` 同源），复用段不执行故偏差不变（同步态归零 / 偏差态段状态
//    中性）；逐段判定的全量状态扫描免掉，各缓存段只查"偏差集 ∩ 依赖"。
//    同步态复用非中性段才还原旧 `post`（中性段短路——整份检查点克隆 +
//    还原是纯开销，expand 层同款）。
// 3. **段边界两档化**（[`DocBoundary`]）：每段必capture的只剩排版副作用字段
//    快照（[`SideEffects`]，复用判定的对齐源 + 闸比较面，~1µs）；expand 检查点
//    + builder 整份克隆（回滚点，~65µs）按 [`ROLLBACK_EVERY`] 间隔稀疏化，
//    后续 `edit` 回滚到最近回滚点、中间段随重放补齐（有界代价）。
//
// 实测（同一基准，release / 2GB VM / 2026-09-03）：改正文 **4.3x**（13.6ms vs
// 58.4ms，复用 111/169）、改宏体 **1.1x**（30.3ms vs 34.8ms，复用 60/121）。
//
// ## 仍存边界（离 M5 目标还差什么）
//
// - **执行段成本 ≈ 全量段成本**：被编辑段及其引用段照常执行，每段多付两次
//   expand 检查点捕获 + 词法依赖提取（~55µs/段）——改宏体场景执行段占多数时
//   加速比被压在 ~1.1x。要再上台阶须把检查点捕获做成增量维护（ntex-core 引擎
//   侧，非本阶段范围）。
// - **副作用边界**：`\output` 例程（页面改道 box255+例程产出，注入路径不执行
//   例程——观测到即禁用复用，保守全量重排）、`\write` 流内容、`\input`（本
//   管线未注入 VFS）不在逐位一致口径内；字体表随管线单调增长（编辑删除
//   `\font` 定义可能改变字体编号，与全新全量编译不逐位一致——本阶段测试
//   不做此类编辑）。marks 族（`\topmark` 等）在断页轮转、随副作用字段整体
//   对齐旧值——读 marks 的段不在本阶段逐位一致口径内（与阶段三同口径）。
// - **内存**：每段边界存副作用字段快照（回滚点再带 expand 检查点 + builder
//   克隆，按 1/8 密度）+ 主列表节点流，O(文档) 量级（页面构建器 ≤1 页/段、
//   节点流 ≈ 文档节点总数）。

// 说明：本文件由 typeset/mod.rs `include!` 进 `typeset` 模块，与其共享 `use`
// 导入（Error/Result/Expander/FontMetrics/Node/BoxNode 等）与私有项
// （[`NodeBuilder`] / [`Fonts`] / [`Mode`] / [`TfmLoader`]）。
use ntex_core::incremental::segmentize;
use ntex_core::incremental::snapshot::{
    lex_deps, reads_touch_slots, snapshot_states_equal, writes_touch_slots, ChainDelta,
    SegmentDeps, StateSnapshot,
};

// DVI 写出由调用方完成（`ntex-dvi` 依赖本 crate，不能反向依赖）：
// `write_dvi(&output.pages, &output.fonts)`。

/// 从已安装 sink 取 NodeBuilder（自由函数而非方法：调用点常同时持有
/// `self.bounds` 的不可变借用，须按字段精度借用 `expander`）。
fn sink_builder(e: &mut Expander) -> &mut NodeBuilder {
    e.sink_mut()
        .as_any_mut()
        .downcast_mut::<NodeBuilder>()
        .expect("增量排版器安装了 NodeBuilder")
}

/// 主列表"首盒上方盒子深度"上下文：`push_box` 决定行间胶水宽度所依据的
/// 垂直上下文。语义镜像 `push_box` 的取法——优先主列表里最后一个 Box/Rule
/// （断页 fire_up 暂停的残余），否则页面构建器的 `prev_depth`；页空则为 None。
/// `Some(d)`：该段首行会插一条按深度 d 烘焙的行间胶水；`None`：首行在空页/
/// 新页（走 topskip，缓存里的前导胶水会被页面构建器丢弃）。见
/// [`IncrementalTypesetter::main_context_matches`] 与 [`SegmentCache::entry_ctx`]。
fn main_above_depth(b: &NodeBuilder) -> Option<i64> {
    match b.lists[0]
        .iter()
        .rev()
        .find(|n| matches!(n, Node::Box(_) | Node::Rule { .. }))
    {
        Some(Node::Box(x)) => Some(x.depth),
        Some(Node::Rule { depth, .. }) => Some(*depth),
        _ => {
            let pd = b.page_state.page.prev_depth();
            if pd > crate::page::IGNORE_DEPTH {
                Some(pd)
            } else {
                None
            }
        }
    }
}

/// 排版副作用字段快照（M5 阶段五）：复用判定的排版半边——既是"段前上下文"闸的
/// 比较面，也是"缓存段复用后副作用字段推进"的对齐源。
///
/// 字段清单 = [`NodeBuilder`] 上随执行演化、但不进节点流/页面产出的状态。不进
/// 快照：`lists`/`list_modes`（主列表）、`page`（页面构建器）、`shipped`（页面
/// 产出）、`transcript`（转录不参与逐位一致口径）、`nodes_appended`/
/// `last_appended`（诊断）、`pagination`/`fonts`/`record_main`（配置与共享设施）。
/// **新增 builder 字段时同步 [`NodeBuilder::side_effects`] /
/// [`NodeBuilder::restore_side_effects`] 两处。**
///
/// 独立成结构（阶段五）：整份 [`NodeBuilder`] 克隆（含页面构建器的当前页节点）
/// 每段一次是增量热路径大头；本快照只拷贝这批小字段（盒子寄存器文件已 `Rc`
/// 共享），每段 ~1µs，使"回滚点整份克隆"得以按间隔稀疏化（见 [`DocBoundary`]）。
#[derive(Debug, Clone, PartialEq)]
struct SideEffects {
    pending_box: Option<PendingBox>,
    pending_kind: Option<GroupKind>,
    pending_shift: Option<i64>,
    pending_hshift: Option<i64>,
    pending_leaders: Option<LeadersKind>,
    leaders_box: Option<(LeadersKind, Node)>,
    params: Params,
    penalty_arrays: [Vec<i64>; 4],
    param_stack: Vec<Params>,
    sfcodes: [u32; 256],
    font_stack: Vec<FontId>,
    space_factor: i64,
    noindent_next: bool,
    align_stack: Vec<(AlignDir, AlignCtx)>,
    last_par_line: i64,
    font_cs_names: Vec<Option<String>>,
    current_font: FontId,
    shipout_next: bool,
    ship_seq: u32,
    /// `\count0..9` 镜像（输出例程刀 5 页号链：页标签/bop 计数取值源）。
    page_counts: [i64; 10],
    boxes: BoxFile,
    box_saves: Vec<(usize, usize, Option<BoxNode>)>,
    output_defined: bool,
    pending_pages: std::collections::VecDeque<BoxNode>,
    write_flush_pending: bool,
    /// 页面 shipout 过（`dead_cycles` 清零依据；输出例程刀 1）。
    page_shipped: bool,
    math_style: MathStyle,
    pending_script: Option<bool>,
    sqrt_pending: bool,
    radical_pending: Option<u32>,
    class_pending: Option<MathClass>,
    accent_pending: bool,
    underline_pending: bool,
    overline_pending: bool,
    nonscript_pending: bool,
    math_fonts: Vec<[Option<FontId>; 3]>,
    patterns: PatternTrie,
    hyph_exceptions: Vec<(Vec<u8>, Vec<usize>)>,
    setbox_target: Option<usize>,
    setbox_global: bool,
    pending_box_spec: Option<(Option<i64>, Option<i64>)>,
    predisplay_size: i64,
    after_display: bool,
    muskip_params: [ntex_core::Glue; 3],
    muskip_is_mu: [bool; 3],
    marks_top: std::collections::HashMap<i64, String>,
    marks_first: std::collections::HashMap<i64, String>,
    marks_bot: std::collections::HashMap<i64, String>,
    marks_split_top: std::collections::HashMap<i64, String>,
    marks_split_first: std::collections::HashMap<i64, String>,
    marks_split_bot: std::collections::HashMap<i64, String>,
    lastbox_hold: Option<BoxNode>,
}

/// 盒子寄存器文件的共享句柄（M5 阶段五）。
///
/// 相等比较**先走指针相等**：复用路径里边界快照与活 builder 共享同一份寄存器
/// 文件（克隆 = 引用计数），派生 `PartialEq` 会退化成 32768 槽逐槽比较
/// （~100µs/段，复用判定的最大单项开销）；不共享时才退回逐槽语义比较。
#[derive(Debug, Clone)]
struct BoxFile(std::rc::Rc<Vec<Option<BoxNode>>>);

impl PartialEq for BoxFile {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

impl SideEffects {
    /// 复用判定的排版半边闸：活 builder 的副作用字段是否与本快照一致。
    ///
    /// `groups`/`math` 不进快照（恒为空是干净段边界的不变式——带缓存的段其段前
    /// 边界必干净，有缓存 ⇒ 该段执行进入时 `builder_shaped` 成立）；活侧非空即
    /// 判不匹配（与阶段四行为一致）。
    fn matches(&self, b: &NodeBuilder) -> bool {
        b.groups.is_empty() && b.math_state.math.is_empty() && b.side_effects() == *self
    }
}

/// 段边界排版检查点：NodeBuilder 整体克隆 + 已 shipout 页数（回滚点用）。
///
/// 克隆时先把 `shipped` 摘走（页面不随检查点复制，只记数量）——整体还原时
/// 按 [`Self::shipped_count`] 截断现有页面列表（编辑点之前的页面由前缀不变式
/// 保证逐位一致，之后的页面随重放重新装配）。
#[derive(Debug, Clone)]
struct LayoutBoundary {
    /// NodeBuilder 全字段克隆（`shipped` 恒为空）。
    builder: NodeBuilder,
    /// 捕获时刻的已 shipout 页数。
    shipped_count: usize,
}

impl LayoutBoundary {
    /// 捕获（须持有 builder 可变引用：摘/还 shipped）。
    fn capture(b: &mut NodeBuilder) -> Self {
        let shipped = std::mem::take(&mut b.page_state.shipped);
        let shipped_count = shipped.len();
        let builder = b.clone();
        b.page_state.shipped = shipped;
        Self {
            builder,
            shipped_count,
        }
    }

    /// 整体还原（编辑回滚点）：builder 全字段 + shipped 截断到边界。
    fn restore_full(&self, b: &mut NodeBuilder) {
        let mut pages = std::mem::take(&mut b.page_state.shipped);
        // 页号链快照与页面同长同截（输出例程刀 5：push_shipped 双表同步）。
        let mut page_counts = std::mem::take(&mut b.page_state.shipped_counts);
        *b = self.builder.clone();
        pages.truncate(self.shipped_count);
        page_counts.truncate(self.shipped_count);
        b.page_state.shipped = pages;
        b.page_state.shipped_counts = page_counts;
    }
}

/// 段边界：排版半边快照（每段都有）+ 回滚点（按间隔稀疏化）。
///
/// 阶段五把"每段边界"拆成两档：
/// - **排版半边 [`SideEffects`]**：每段必捕获——复用判定（段前上下文闸）与
///   复用后副作用字段推进都以它为准，是链数据，便宜（~1µs）；
/// - **回滚点**（expand 检查点 + builder 整份克隆，~65µs）：只按
///   [`ROLLBACK_EVERY`] 间隔 + 末段刷新。后续 `edit` 回滚到**最近的回滚点**、
///   中间段随重放补齐（复用段重注入 / 执行段重算）——多回滚的段重放是
///   有界代价（≤ [`ROLLBACK_EVERY`] 段），换掉的是每段一次的整份克隆。
#[derive(Debug, Clone)]
struct DocBoundary {
    side: SideEffects,
    rollback: Option<RollbackPoint>,
}

/// 回滚点：expand 半边 + 排版半边整份克隆。
#[derive(Debug, Clone)]
struct RollbackPoint {
    exp: StateSnapshot,
    layout: LayoutBoundary,
}

/// 回滚点稀疏化间隔：每这么多段刷新一个回滚点（末段必刷）。
const ROLLBACK_EVERY: usize = 8;

/// 缓存的段贡献：该段进入主列表的节点流（增量重放免执行）。
///
/// `entry_ctx`：录制该段时**段首**的垂直上下文（首个盒子上方盒子的深度，见
/// [`IncrementalTypesetter::main_context_matches`]）——缓存里段首行间胶水的
/// 宽度按它烘焙；复用要求活上下文与它一致，否则该段照常执行（自愈缓存）。
///
/// `record_exp`：录制该段时的 expand 段前状态（完整快照）。缓存只在该状态下
/// 复现——重放中**执行**过前一段后，段边界 `bounds[j]` 已被刷新成当前文档的
/// 新状态，不能再用它判定"活状态 == 录制状态"（改了 `\hsize` 之类参数的编辑
/// 会因边界刷新而漏检、错误复用旧宽度排版）；复用判定须对照录制时的段前状态。
///
/// `record_post`：录制该段时的 expand 段后状态。同步态复用用它把活状态推进到
/// "该段执行后"（与 `record_exp` 同源成对，才是该缓存成立的准绳）——**不能**用
/// `bounds[j+1].exp`：偏差路径复用会把段后边界刷成当轮活状态（回滚点必须是
/// 新文档状态），旧编辑轮次的状态留在边界里，同步态还原它会冲掉活状态。
///
/// `deps` / `state_neutral`（阶段四）：词法读/写依赖（超近似）与状态中性标记
/// ——同步态（活状态与 `record_exp` 精确一致）之外的第二条复用通道：偏差不触及
/// 该段依赖且跳过执行不动状态时，缓存节点流同样成立（见
/// [`IncrementalTypesetter::reuse_verdict`]）。
#[derive(Debug, Clone)]
struct SegmentCache {
    contrib: Vec<Node>,
    entry_ctx: Option<i64>,
    record_exp: StateSnapshot,
    record_post: StateSnapshot,
    deps: SegmentDeps,
    state_neutral: bool,
}

/// 缓存段复用判定通过的通道。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReuseVerdict {
    /// 同步态：活状态与录制时段前状态精确一致 → 执行必然复现同一输出，
    /// 复用后活状态推进到旧 post（阶段三口径）。
    Synced,
    /// 依赖判定通过：状态有偏差，但该段读/写依赖不触及变化槽、段状态中性、
    /// 排版上下文未漂移 → 复用后 expand 状态原地不动（不还原旧 post）。
    DepsCleared,
}

/// 增量统计（基准与验证用）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IncrementalStats {
    /// 最近一次 `compile`/`edit` 实际执行（重算）的段数。
    pub executed: usize,
    /// 最近一次 `compile`/`edit` 复用缓存（免执行）的段数。
    pub reused: usize,
}

/// 端到端增量编译结果：已 shipout 页面 + 字体表快照（DVI 输入）。
#[derive(Debug, Clone)]
pub struct CompileOutput {
    pub pages: Vec<BoxNode>,
    /// 各页面 shipout 边界的 `\count0..9` 快照（与 `pages` 一一对应；输出例程刀 5）。
    pub page_counts: Vec<[i64; 10]>,
    pub fonts: Vec<FontMetrics>,
}

/// 端到端增量排版器（M5 阶段三）：段级 expand 增量 + 段贡献缓存 + 页面装配重跑。
///
/// 用法：`compile(source)` 首次全量（建缓存）；`edit(k, new_text)` 编辑第 k 段
/// 增量重生。两法都返回已 shipout 页面，DVI 字节由调用方 `ntex_dvi::write_dvi`
/// 写出；增量与全量的 DVI 必须逐位一致（测试锁死）。
#[derive(Debug)]
pub struct IncrementalTypesetter {
    expander: Expander,
    /// TFM 字体表（`Rc<RefCell>` 与 NodeBuilder/TfmLoader 共享，单调增长）。
    fonts: Fonts,
    segments: Vec<String>,
    /// 段 j 执行前边界（j ∈ 0..=n；bounds[n] = 末段执行后、收尾冲页前）。
    bounds: Vec<Option<DocBoundary>>,
    cache: Vec<Option<SegmentCache>>,
    /// 观测到 `\output` 例程：页面改道 box255 + 例程，注入路径不执行例程，
    /// 此后禁用缓存复用（保守全量重排，正确性优先）。
    output_routine: bool,
    /// 最近一次 `edit` 重放中各段的缓存拒绝原因（判定时刻记录；`None` = 复用了
    /// 缓存，或该段无可复用缓存——被编辑段/首次执行/合并段）。
    rejects: Vec<Option<&'static str>>,
    stats: IncrementalStats,
    /// CJK 字体回落名（与 [`Typesetter::fallback_font`] 同语义；增量路径的
    /// install_builder 也要同步到 NodeBuilder，否则增量段中文回落失效）。
    fallback_font: Option<String>,
}

impl IncrementalTypesetter {
    /// TFM 字体模式 + 自动分页（与 [`Typesetter::with_tfm_paginated`] 同配置）。
    pub fn with_tfm_paginated() -> Self {
        Self {
            expander: Expander::new(),
            fonts: Fonts::Tfm(std::rc::Rc::new(std::cell::RefCell::new(Vec::new()))),
            segments: Vec::new(),
            bounds: Vec::new(),
            cache: Vec::new(),
            output_routine: false,
            rejects: Vec::new(),
            stats: IncrementalStats::default(),
            fallback_font: None,
        }
    }

    /// CJK 字体回落（与 [`Typesetter::set_fallback_font`] 同语义；增量路径
    /// 的 install_builder 同步到 NodeBuilder）。
    pub fn set_fallback_font(&mut self, name: Option<String>) {
        self.fallback_font = name;
    }

    /// 注入 VFS 后端（`\input`/`\write` 等副作用原语的文件接口）。
    pub fn set_vfs(&mut self, vfs: Box<dyn ntex_io::Vfs>) {
        self.expander.set_vfs(vfs);
    }

    /// 当前段列表（切段结果）。
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// 最近一次 `compile`/`edit` 的增量统计。
    pub fn stats(&self) -> IncrementalStats {
        self.stats
    }

    /// 最近一次 `edit` 重放中各段的缓存拒绝原因（`None` = 复用了缓存，或该段
    /// 本就无可复用缓存）。
    ///
    /// **就地**判定：原因在重放到该段前一刻记录（活状态随重放推进），离线事后
    /// 推断会拿到过期状态下的原因。
    pub fn last_rejects(&self) -> &[Option<&'static str>] {
        &self.rejects
    }

    /// 全量编译（首次/重建缓存）：切段 → 逐段执行并记录边界检查点与贡献缓存
    /// → 收尾冲页 → 页面。
    pub fn compile(&mut self, source: &str) -> Result<CompileOutput> {
        self.segments = segmentize(source);
        let n = self.segments.len();
        self.bounds = (0..=n).map(|_| None).collect();
        self.cache = (0..n).map(|_| None).collect();
        self.output_routine = false;
        self.rejects = vec![None; n];
        self.stats = IncrementalStats::default();
        self.install_builder();
        self.capture_boundary(0, true);
        for j in 0..n {
            if let Err(e) = self.execute_segment(j) {
                // 段执行出错：本轮止于此段，其后各段边界不再反映当前文档状态
                // ——撤销它们的回滚点（后续 `edit` 退到更早的回滚点重放）。
                self.invalidate_rollback_after(j);
                return Err(e);
            }
        }
        self.finish_doc()
    }

    /// 编辑第 `index` 段并增量重生（单路径：回滚到段前 → 重放）。
    ///
    /// 回滚点：编辑段之前**最近的回滚点**边界（复用段只按 [`ROLLBACK_EVERY`]
    /// 间隔留回滚点，见 [`DocBoundary`]）——回滚点之前的段缓存原样保留（页面
    /// 由前缀不变式保证逐位一致，不重排）；回滚点与编辑段之间的段随重放补齐
    /// （可复用的重注入，其余重算，代价 ≤ [`ROLLBACK_EVERY`] 段）。
    /// 被编辑段必算；其后各段按 [`Self::reuse_verdict`] 判定复用缓存节点流
    /// （页面装配重跑）或照常执行。
    pub fn edit(&mut self, index: usize, new_text: &str) -> Result<CompileOutput> {
        let n = self.segments.len();
        if index >= n {
            return Err(Error::invalid_input(format!(
                "段下标 {index} 越界：当前文档共 {n} 段"
            )));
        }
        let start = (0..=index)
            .rev()
            .find(|&i| {
                self.bounds[i]
                    .as_ref()
                    .is_some_and(|b| b.rollback.is_some())
            })
            .ok_or_else(|| {
                Error::invalid_input("无段边界检查点：请先 compile() 全文编译")
            })?;
        self.stats = IncrementalStats::default();
        self.rejects = vec![None; n];
        // 1) 回滚到段 start 前（expand 检查点整体还原 + 排版状态整体还原 +
        //    shipped 截断——之后页面的逐位一致由重放重新装配保证）。
        {
            let rp = self.bounds[start]
                .as_ref()
                .and_then(|b| b.rollback.as_ref())
                .expect("上方已筛选回滚点");
            rp.exp.restore_to(&mut self.expander);
            rp.layout.restore_full(sink_builder(&mut self.expander));
        }
        self.segments[index] = new_text.to_owned();
        // 2) 从 start 起重放。链偏差随重放推进（expand 层 SegmentEngine 同款，
        //    阶段五）：入口回滚后活状态即旧链上"段 start 前"的状态，先对
        //    `record_exp(start)` 捕获一次；此后**只在执行段之后**重算——
        //    复用段不执行（同步态归零 / 偏差态段状态中性不变），逐段判定的
        //    全量状态扫描由此免掉，各缓存段只查"偏差集 ∩ 依赖"。
        //    参照取该段**旧** post（覆盖缓存前取）：它与下一段旧 `record_exp`
        //    同源（段背靠背执行，两快照同状态），正是下一段缓存成立的准绳。
        let mut delta = self.cache[start]
            .as_ref()
            .map(|c| ChainDelta::capture(&self.expander, &c.record_exp));
        for j in start..n {
            // 判定：被编辑段 / 观测到输出例程 / 无缓存 → 必算；其余按复用判定。
            let outcome: std::result::Result<ReuseVerdict, &'static str> = if j == index {
                Err("被编辑段（必算）")
            } else if self.output_routine {
                Err("观测到 \\output 例程（注入路径不执行例程，保守重排）")
            } else if self.cache[j].is_none() {
                Err("无缓存（合并段 / 前轮执行止于此段之前）")
            } else {
                match delta.as_ref() {
                    Some(d) => self.reuse_verdict(j, d),
                    None => Err("无链偏差基准（此前各段无旧缓存，保守重算）"),
                }
            };
            let verdict = match outcome {
                Ok(v) => v,
                Err(reason) => {
                    self.rejects[j] = Some(reason);
                    let reference = self.cache[j]
                        .as_ref()
                        .map(|c| c.record_post.clone())
                        .or_else(|| {
                            self.cache
                                .get(j + 1)
                                .and_then(|c| c.as_ref())
                                .map(|c| c.record_exp.clone())
                        });
                    if let Err(e) = self.execute_segment(j) {
                        // 段执行出错：本轮重放止于此段，其后各段边界不再反映
                        // 当前文档状态——撤销它们的回滚点，后续 `edit` 退到更早
                        // 的回滚点重放（不会用到过期状态）。
                        self.invalidate_rollback_after(j);
                        return Err(e);
                    }
                    delta = reference
                        .as_ref()
                        .map(|r| ChainDelta::capture(&self.expander, r));
                    continue;
                }
            };
            self.rejects[j] = None;
            // 复用 = 缓存节点流重新注入（行盒免重排）+ 页面装配重跑。
            let contrib = self.cache[j].as_ref().expect("已判可复用").contrib.clone();
            {
                let b = sink_builder(&mut self.expander);
                for node in contrib {
                    b.append(node);
                }
                // 排版副作用字段对齐到该段执行后的样子（主列表/页面构建器/
                // shipped 保留注入重算的结果）。
                let side = &self
                    .bounds
                    .get(j + 1)
                    .and_then(|b| b.as_ref())
                    .ok_or_else(|| Error::invalid_input("无段后排版边界检查点"))?
                    .side;
                b.restore_side_effects(side);
                if verdict == ReuseVerdict::Synced {
                    // 同步态：活状态与录制时段前状态精确一致 → 执行必然复现，推进
                    // 到**录制时**的段后状态。段状态中性（pre ≈ post，执行本不改
                    // 状态）时活状态已是对的状态，整体还原是纯开销——expand 层
                    // 同款短路。链偏差随之归零（活状态 == 旧链下一段的段前状态）。
                    let (neutral, post) = {
                        let c = self.cache[j].as_ref().expect("已判可复用");
                        (c.state_neutral, &c.record_post)
                    };
                    if !neutral {
                        post.clone().restore_to(&mut self.expander);
                    }
                    // `ChainDelta::empty` 是 ntex-core 的 crate 内构造器；这里按
                    // 公开字段直接构造零偏差（值状态一致 + 无变化槽）。
                    delta = Some(ChainDelta {
                        value_differs: false,
                        slots: std::collections::HashSet::new(),
                    });
                }
                // 偏差路径（依赖判定通过）：段状态中性（判定已闸）→ 执行本不改
                // expand 状态，活状态即该段执行后的正确状态，**链偏差不变**；
                // **不能**还原旧 post——会把偏差槽（被改宏体）的新值冲掉。
            }
            // 段后边界刷新为活状态：排版半边断页点已变；偏差路径下 expand 半边
            // 也随活状态走——旧链快照不再是当前文档状态，后续 `edit` 回滚到它
            // 会丢掉偏差槽新值（回滚点按间隔稀疏化，见 [`DocBoundary`]）。
            self.capture_boundary(j + 1, false);
            self.stats.reused += 1;
        }
        self.finish_doc()
    }

    /// 安装 NodeBuilder（自动分页）并同步参数镜像；挂 TFM 字体加载器。
    ///
    /// 与 [`Typesetter::install_builder`] 同款镜像：参数、muskip 寄存器
    /// （fresh 引擎为 0——NodeBuilder 的 3mu/4mu/5mu 默认值必须被覆盖，否则
    /// 数学间距与全量路径不一致）、FontId → cs 名表。
    fn install_builder(&mut self) {
        if let Fonts::Tfm(table) = &self.fonts {
            self.expander
                .set_font_loader(Box::new(TfmLoader { table: table.clone() }));
        }
        let mut builder = NodeBuilder::with_pagination(self.fonts.clone(), true);
        builder.font_cs_names = self.expander.font_cs_names_ref().clone();
        builder.math_state.muskip_params = self.expander.muskip_registers();
        builder.math_state.muskip_is_mu = [true; 3];
        builder.sync_params(self.expander.params_ref());
        // \sfcode 默认（大写 999）与全量路径 install_builder 同源，否则增量
        // 段与全量段的大写-标点空格因子钳制不一致（见 init_sfcodes 文档）。
        init_sfcodes(&mut builder);
        builder.set_fallback_font_name(self.fallback_font.clone());
        self.expander.set_sink(Box::new(builder));
    }

    fn builder_mut(&mut self) -> &mut NodeBuilder {
        sink_builder(&mut self.expander)
    }

    /// 排版器是否处于"干净段边界"（垂直模式、无进行中段落/盒子/数学/组）。
    /// 复用判定的 shape 条件与 execute_segment 的缓存有效性共用。
    fn builder_shaped(&mut self) -> bool {
        let b = sink_builder(&mut self.expander);
        b.groups.is_empty()
            && b.lists.len() == 1
            && b.math_state.math.is_empty()
            && b.box_state.pending_box.is_none()
            && b.box_state.pending_kind.is_none()
            && !b.page_state.shipout_next
    }

    /// 段 `j` 的缓存复用判定（`Err(原因)` = 不可复用，须照常执行）。
    ///
    /// 复用 = 缓存节点流重新注入（行盒免重排）+ 页面装配重跑。按序判定，任一
    /// 不成立即重算：
    ///
    /// 1. **段边界干净**（expand 无悬挂构造 + 排版器垂直模式、无进行中段落/盒/
    ///    数学/组）；
    /// 2. **活 expand 状态相对旧链的偏差**（重放循环携带的 `delta`，参照 =
    ///    该段**录制时**的段前状态）——不能对照 `bounds[j]`：重放执行前一段后
    ///    `bounds[j]` 已刷新成当前文档的新状态（改 `\hsize` 等参数的编辑会漏检），
    ///    录制状态才是该缓存成立的准绳。偏差为空 → 同步态
    ///    （[`ReuseVerdict::Synced`]，执行必然复现同一输出）；
    /// 3. 偏差非空 → **依赖四闸**（expand 层 `cache_reject_reason_inner` 同款）：
    ///    值状态偏差（寄存器/参数/catcode/编码表/流，未做槽级归因）全局失效；
    ///    段含 `\csname`（动态 cs 名）保守失效；读依赖闭包触及变化槽（该段引用
    ///    的宏被改动）失效；写集触及变化槽失效（跳过会让本应重写的值停留在被改
    ///    值）；段非状态中性失效（偏差下跳过执行不还原旧 post，只有"执行也不改
    ///    状态"才健全）；
    /// 4. **排版半边闸**：活 `NodeBuilder` 副作用字段 ≠ 段前边界（[`main_above_depth`]
    ///    之外的排版上下文：当前字体、参数、marks、盒子寄存器、断字表…）失效
    ///    ——缓存节点流按旧上下文烘焙；
    /// 5. **主列表垂直上下文一致**（[`main_above_depth`]）：缓存里的段首行间胶水
    ///    （`push_box` 插在段落首行前的 baselineskip glue）按录制时上方盒子的深度
    ///    烘焙（宽度 = `\baselineskip − (上盒深度 + 本行高)`；深度看行内有无下行
    ///    字母：无下行字母行深度 0，有则 ~1.94pt）。编辑改动该段之前的内容后
    ///    （行高/深度、合并段、断页移动），上方盒深度漂移则缓存胶水宽度过期——
    ///    复用会把旧宽度注入（如上方盒深度 0 ↔ 127431 差一位即 `\baselineskip`
    ///    胶水宽差整行深度），与全量逐位不一致。
    ///
    /// 任一不成立 → 该段照常执行（重算行盒/胶水，缓存与边界快照随之刷新，后续段
    /// 仍可复用）。
    ///
    /// 阶段五：链偏差 `delta` 由重放循环携带（执行段后重算一次），此处只查表，
    /// 不再做"活状态 vs 录制段前快照"的全量扫描。
    fn reuse_verdict(
        &mut self,
        j: usize,
        delta: &ChainDelta,
    ) -> std::result::Result<ReuseVerdict, &'static str> {
        if !self.expander.boundary_is_clean() {
            return Err("expand 段边界不干净（跨段构造：未闭合组/条件/数学）");
        }
        if !self.builder_shaped() {
            return Err("排版器非干净段边界（进行中段落/盒子/数学/组）");
        }
        let cache = self.cache[j].as_ref().expect("调用方已判 Some");
        if delta.is_empty() {
            self.entry_ctx_verdict(j)?;
            return Ok(ReuseVerdict::Synced);
        }
        if delta.value_differs {
            return Err("值状态变化（寄存器/参数/catcode/编码表/流，未做槽级归因）");
        }
        if cache.deps.dynamic_cs {
            return Err("段含 \\csname（动态 cs 名，保守失效）");
        }
        if reads_touch_slots(
            self.expander.intern(),
            self.expander.eqtb().slots(),
            &cache.deps,
            &delta.slots,
        ) {
            return Err("读依赖闭包触及变化槽（该段引用的宏被改动）");
        }
        if writes_touch_slots(self.expander.intern(), &cache.deps.write_cs, &delta.slots) {
            return Err("写集触及变化槽（跳过会让本应重写的值停留在被改值）");
        }
        if !cache.state_neutral {
            return Err("段有状态副作用（有偏差时跳过不还原 post，不能跳过执行）");
        }
        {
            let pre = &self
                .bounds
                .get(j)
                .and_then(|b| b.as_ref())
                .ok_or("无段前排版边界")?
                .side;
            if !pre.matches(sink_builder(&mut self.expander)) {
                return Err("排版副作用字段偏离段前边界（字体/参数/marks/盒子/断字表）");
            }
        }
        self.entry_ctx_verdict(j)?;
        Ok(ReuseVerdict::DepsCleared)
    }

    /// 主列表垂直上下文是否与录制时一致（见 [`Self::reuse_verdict`] 条件 5）。
    fn entry_ctx_verdict(&mut self, j: usize) -> std::result::Result<(), &'static str> {
        let want = self.cache[j].as_ref().expect("调用方已判 Some").entry_ctx;
        let cur = main_above_depth(sink_builder(&mut self.expander));
        if cur == want {
            Ok(())
        } else {
            Err("主列表垂直上下文漂移（段首行间胶水按旧深度烘焙）")
        }
    }

    /// 捕获段边界（段 `j` 执行/复用后 → 写入 `bounds[j+1]`）。
    ///
    /// 排版半边快照每段必capture（复用判定的对齐源/闸比较面）；回滚点按
    /// [`ROLLBACK_EVERY`] 间隔 + 末段刷新（`force` 用于段 0 与需要精确回滚点的
    /// 场景）。
    fn capture_boundary(&mut self, at: usize, force: bool) {
        let side = sink_builder(&mut self.expander).side_effects();
        let rollback = if force || at % ROLLBACK_EVERY == 0 || at == self.segments.len() {
            let exp = StateSnapshot::capture(&self.expander);
            let layout = LayoutBoundary::capture(self.builder_mut());
            Some(RollbackPoint { exp, layout })
        } else {
            None
        };
        self.bounds[at] = Some(DocBoundary { side, rollback });
    }

    /// 撤销段 `j` 之后各边界的回滚点（本轮重放未经过它们，内容已过期）。
    fn invalidate_rollback_after(&mut self, j: usize) {
        for b in self.bounds[j + 1..].iter_mut().flatten() {
            b.rollback = None;
        }
    }

    /// 执行第 `j` 段：录制主列表节点流 → 缓存 → 捕获段后边界。
    fn execute_segment(&mut self, j: usize) -> Result<()> {
        // 段缓存只对该段"自含贡献"有效：段执行进入时须无悬挂构造（否则本段是
        // 在吸收前段未闭合段落——录制流跨段，不能代表本段），退出时须回到干净
        // 段边界（否则本段留下的开放段落在后续段才闭合，录制流同样跨段）。
        // 编辑文本不以空行/`\par` 收尾即触发（TeX 语义下与下一段合并成一段）；
        // 合并段的缓存作废，杜绝后续 `edit` 复用错位缓存（M5 阶段三测试锁定：
        // 还原编辑必须回到基线）。
        let dirty_entry = !self.builder_shaped();
        let entry_ctx = main_above_depth(sink_builder(&mut self.expander));
        let record_exp = StateSnapshot::capture(&self.expander);
        let source = self.segments[j].clone();
        // 词法读/写依赖（超近似；`read_versions` 仅诊断留痕）。
        let deps = lex_deps(&source).attach_versions(self.expander.intern(), self.expander.eqtb());
        self.builder_mut().record_main = Some(Vec::new());
        self.expander.feed_source(source);
        let result = self.expander.run();
        let contrib = self.builder_mut().record_main.take().unwrap_or_default();
        // 出错即终止后续段（与全量 `run_source` 语义一致）；出错段的边界/缓存
        // 不更新——管线停在错误前状态，下次 `compile` 重建。
        result?;
        // 段后快照（与 `record_exp` 同源成对：同步态复用推进 + 状态中性判定都用它；
        // 独立于 `bounds[j+1]`——边界会被偏差路径复用刷新成当轮活状态）。
        let record_post = StateSnapshot::capture(&self.expander);
        let dirty_exit = !self.builder_shaped();
        // 状态中性 = 执行前后 expand 状态语义一致（捕获时算一次，判定只查布尔；
        // 有链偏差时它是"跳过执行且不还原旧 post"的健全前提）。
        let state_neutral = snapshot_states_equal(&record_exp, &record_post);
        self.cache[j] = if dirty_entry || dirty_exit {
            None
        } else {
            Some(SegmentCache {
                contrib,
                entry_ctx,
                record_exp,
                record_post,
                deps,
                state_neutral,
            })
        };
        self.capture_boundary(j + 1, false);
        if self.builder_mut().page_state.output_defined {
            self.output_routine = true;
        }
        self.stats.executed += 1;
        Ok(())
    }

    /// 收尾冲页（与 [`super::Typesetter::finish`] 步骤 1-4 同序的镜像实现，
    /// 语义分支一致）——但不摘 sink：管线保持可继续 `edit`。随后取走页面
    /// 与字体表快照（DVI 由调用方写出）。
    fn finish_doc(&mut self) -> Result<CompileOutput> {
        let ended = self.expander.is_ended();
        {
            let b = self.builder_mut();
            if b.box_state.pending_box.is_some() {
                if ended {
                    b.box_state.pending_box = None;
                } else {
                    return Err(Error::invalid_input("\\hbox/\\vbox 后缺少组"));
                }
            }
            if b.page_state.shipout_next {
                if ended {
                    b.page_state.shipout_next = false;
                } else {
                    return Err(Error::invalid_input("\\shipout 后缺少盒子"));
                }
            }
            if !b.groups.is_empty() {
                if ended {
                    let _ = b.write16(format!(
                        "(end occurred inside a group at level {})\n",
                        b.groups.len()
                    ));
                    b.groups.clear();
                } else {
                    return Err(Error::invalid_input("组未闭合（缺少 }）"));
                }
            }
            if !b.math_state.math.is_empty() {
                if ended {
                    let _ = b.write16("(end occurred inside a math list)\n".to_string());
                    b.math_state.math.clear();
                } else {
                    return Err(Error::invalid_input("数学模式未闭合（缺少 $）"));
                }
            }
            if b.mode() == Mode::Horizontal {
                b.close_paragraph();
            }
            if ended {
                while b.lists.len() > 1 {
                    b.lists.pop();
                    b.list_modes.pop();
                }
            }
        }
        // 执行 close_paragraph 产出的待执行输出例程，随后 `\end`/EOF 冲页
        // 交错（冲一页 → 执行例程 → 再冲）。
        self.expander.run_pending_output()?;
        loop {
            let ejected = {
                let b = self.builder_mut();
                if b.page_state.pagination {
                    b.eject_one_page()?
                } else {
                    false
                }
            };
            if !ejected {
                break;
            }
            self.expander.run_pending_output()?;
        }
        // 输出例程可能重新打开水平列表：再收段 + 丢弃残留层，仅留主列表。
        {
            let b = self.builder_mut();
            if b.mode() == Mode::Horizontal {
                b.close_paragraph();
            }
            while b.lists.len() > 1 {
                b.lists.pop();
                b.list_modes.pop();
            }
        }
        // RFC-3：排版结束收尾 flush 残留延迟写流（TeX \end final_cleanup 语义）。
        self.expander.flush_writes()?;
        // 页面**克隆**给调用方（不摘走）：builder 里的 shipped 是下一次 `edit`
        // 回滚截断的基准（前缀页面的逐位一致由它保证）。
        let pages = self.builder_mut().page_state.shipped.clone();
        let page_counts = self.builder_mut().shipped_page_counts().to_vec();
        let fonts = match &self.fonts {
            Fonts::Tfm(table) => table.borrow().clone(),
            Fonts::Fn { .. } => Vec::new(),
        };
        Ok(CompileOutput {
            pages,
            page_counts,
            fonts,
        })
    }
}
