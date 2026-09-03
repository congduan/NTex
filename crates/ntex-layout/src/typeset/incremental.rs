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
// ## 仍存边界（离 M5 目标还差什么）
//
// - **偏差全量扫描**：每个缓存段判定一次"活状态 vs 录制段前快照"（值指纹 +
//   eqtb 全槽）。expand 层靠"一次扫描 + 链偏差集随重放推进"免掉逐段扫描；排版
//   层各段录制快照互不相同（节点流按各自段前状态烘焙），要免扫描须先把偏差集
//   增量维护起来——复用路径当前的主要增量开销。
// - **副作用边界**：`\output` 例程（页面改道 box255+例程产出，注入路径不执行
//   例程——观测到即禁用复用，保守全量重排）、`\write` 流内容、`\input`（本
//   管线未注入 VFS）不在逐位一致口径内；字体表随管线单调增长（编辑删除
//   `\font` 定义可能改变字体编号，与全新全量编译不逐位一致——本阶段测试
//   不做此类编辑）。marks 族（`\topmark` 等）在断页轮转、随副作用字段整体
//   对齐旧值——读 marks 的段不在本阶段逐位一致口径内（与阶段三同口径）。
// - **内存**：每段边界存 expand 检查点 + NodeBuilder 克隆 + 主列表节点流，
//   O(文档) 量级（页面构建器 ≤1 页/段、节点流 ≈ 文档节点总数）。

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
            let pd = b.page.prev_depth();
            if pd > crate::page::IGNORE_DEPTH {
                Some(pd)
            } else {
                None
            }
        }
    }
}

/// 段边界排版检查点：NodeBuilder 整体克隆 + 已 shipout 页数。
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
        let shipped = std::mem::take(&mut b.shipped);
        let shipped_count = shipped.len();
        let builder = b.clone();
        b.shipped = shipped;
        Self {
            builder,
            shipped_count,
        }
    }

    /// 整体还原（编辑回滚点）：builder 全字段 + shipped 截断到边界。
    fn restore_full(&self, b: &mut NodeBuilder) {
        let mut pages = std::mem::take(&mut b.shipped);
        *b = self.builder.clone();
        pages.truncate(self.shipped_count);
        b.shipped = pages;
    }

    /// 副作用字段推进（缓存段复用时）：该段未执行，其排版副作用（盒子寄存器、
    /// marks、参数镜像、当前字体、断字表等）不会发生——对齐到边界快照里的
    /// "执行后"样子；主列表 / 页面构建器 / shipped 保留注入重算的结果（页面
    /// 断点随编辑后移，这正是增量要的效果）。
    ///
    /// 不还原：`lists`/`list_modes`（主列表）、`page`（页面构建器）、`shipped`
    /// （页面产出）、`transcript`（转录不参与逐位一致口径）、
    /// `nodes_appended`/`last_appended`（诊断）、`pagination`/`fonts`/
    /// `record_main`（配置与共享设施）。新增字段时同步这里。
    fn restore_side_effects(&self, b: &mut NodeBuilder) {
        let s = &self.builder;
        b.groups = s.groups.clone();
        b.pending_box = s.pending_box;
        b.pending_kind = s.pending_kind;
        b.pending_shift = s.pending_shift;
        b.pending_hshift = s.pending_hshift;
        b.pending_leaders = s.pending_leaders;
        b.leaders_box = s.leaders_box.clone();
        b.params = s.params;
        b.penalty_arrays = s.penalty_arrays.clone();
        b.param_stack = s.param_stack.clone();
        b.sfcodes = s.sfcodes;
        b.font_stack = s.font_stack.clone();
        b.space_factor = s.space_factor;
        b.noindent_next = s.noindent_next;
        b.align_dir = s.align_dir;
        b.align_columns = s.align_columns.clone();
        b.last_par_line = s.last_par_line;
        b.font_cs_names = s.font_cs_names.clone();
        b.current_font = s.current_font;
        b.shipout_next = s.shipout_next;
        b.ship_seq = s.ship_seq;
        b.boxes = s.boxes.clone();
        b.box_saves = s.box_saves.clone();
        b.output_defined = s.output_defined;
        b.pending_pages = s.pending_pages.clone();
        b.write_flush_pending = s.write_flush_pending;
        b.math = s.math.clone();
        b.math_style = s.math_style;
        b.pending_script = s.pending_script;
        b.sqrt_pending = s.sqrt_pending;
        b.radical_pending = s.radical_pending;
        b.class_pending = s.class_pending;
        b.accent_pending = s.accent_pending;
        b.underline_pending = s.underline_pending;
        b.overline_pending = s.overline_pending;
        b.nonscript_pending = s.nonscript_pending;
        b.math_fonts = s.math_fonts.clone();
        b.patterns = s.patterns.clone();
        b.hyph_exceptions = s.hyph_exceptions.clone();
        b.setbox_target = s.setbox_target;
        b.setbox_global = s.setbox_global;
        b.pending_box_spec = s.pending_box_spec;
        b.display_short = s.display_short;
        b.after_display = s.after_display;
        b.muskip_params = s.muskip_params;
        b.muskip_is_mu = s.muskip_is_mu;
        b.marks_top = s.marks_top.clone();
        b.marks_first = s.marks_first.clone();
        b.marks_bot = s.marks_bot.clone();
        b.marks_split_top = s.marks_split_top.clone();
        b.marks_split_first = s.marks_split_first.clone();
        b.marks_split_bot = s.marks_split_bot.clone();
        b.lastbox_hold = s.lastbox_hold.clone();
    }
}

/// 排版副作用字段是否一致（复用判定的排版半边闸）。
///
/// 与 [`LayoutBoundary::restore_side_effects`] **同一字段清单**（新增字段时两处
/// 同步）：闸判定要求"活 builder 的这些字段 == 该段执行前边界里的字段"，此后
/// 缓存节点流按旧上下文烘焙仍然成立、且"执行后副作用字段 == 旧 post"（副作用
/// 演化是 (段前字段, token 流, 引擎读值) 的确定函数）——复用路径的字段对齐与
/// 执行等价。`groups`/`math`/`pending_box`/`pending_kind`/`shipout_next` 不在
/// 清单：带缓存的段其段前边界必是干净段边界（有缓存 ⇒ 该段执行进入时
/// `builder_shaped` 成立），进行中构造两侧恒为空，此处仅作不变式断言。
fn side_effects_match(a: &NodeBuilder, b: &NodeBuilder) -> bool {
    a.groups.is_empty()
        && b.groups.is_empty()
        && a.math.is_empty()
        && b.math.is_empty()
        && a.pending_leaders == b.pending_leaders
        && a.leaders_box == b.leaders_box
        && a.pending_shift == b.pending_shift
        && a.pending_hshift == b.pending_hshift
        && a.params == b.params
        && a.penalty_arrays == b.penalty_arrays
        && a.param_stack == b.param_stack
        && a.sfcodes == b.sfcodes
        && a.font_stack == b.font_stack
        && a.space_factor == b.space_factor
        && a.noindent_next == b.noindent_next
        && a.align_dir == b.align_dir
        && a.align_columns == b.align_columns
        && a.last_par_line == b.last_par_line
        && a.font_cs_names == b.font_cs_names
        && a.current_font == b.current_font
        && a.ship_seq == b.ship_seq
        && a.boxes == b.boxes
        && a.box_saves == b.box_saves
        && a.output_defined == b.output_defined
        && a.pending_pages == b.pending_pages
        && a.write_flush_pending == b.write_flush_pending
        && a.math_style == b.math_style
        && a.pending_script == b.pending_script
        && a.sqrt_pending == b.sqrt_pending
        && a.radical_pending == b.radical_pending
        && a.class_pending == b.class_pending
        && a.accent_pending == b.accent_pending
        && a.underline_pending == b.underline_pending
        && a.overline_pending == b.overline_pending
        && a.nonscript_pending == b.nonscript_pending
        && a.math_fonts == b.math_fonts
        && a.patterns == b.patterns
        && a.hyph_exceptions == b.hyph_exceptions
        && a.setbox_target == b.setbox_target
        && a.setbox_global == b.setbox_global
        && a.pending_box_spec == b.pending_box_spec
        && a.display_short == b.display_short
        && a.after_display == b.after_display
        && a.muskip_params == b.muskip_params
        && a.muskip_is_mu == b.muskip_is_mu
        && a.marks_top == b.marks_top
        && a.marks_first == b.marks_first
        && a.marks_bot == b.marks_bot
        && a.marks_split_top == b.marks_split_top
        && a.marks_split_first == b.marks_split_first
        && a.marks_split_bot == b.marks_split_bot
        && a.lastbox_hold == b.lastbox_hold
}

/// 段边界完整检查点：expand 半边 + 排版半边。
#[derive(Debug, Clone)]
struct DocBoundary {
    exp: StateSnapshot,
    layout: LayoutBoundary,
}

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
        }
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
        self.bounds[0] = Some(self.capture_boundary());
        for j in 0..n {
            self.execute_segment(j)?;
        }
        self.finish_doc()
    }

    /// 编辑第 `index` 段并增量重生（单路径：回滚到段前 → 重放）。
    ///
    /// 回滚点之前的段缓存原样保留（页面由前缀不变式保证逐位一致，不重排）；
    /// 被编辑段必算；其后各段按 [`Self::reuse_verdict`] 判定复用缓存节点流
    /// （页面装配重跑）或照常执行。
    pub fn edit(&mut self, index: usize, new_text: &str) -> Result<CompileOutput> {
        let n = self.segments.len();
        if index >= n {
            return Err(Error::invalid_input(format!(
                "段下标 {index} 越界：当前文档共 {n} 段"
            )));
        }
        if self.bounds[index].is_none() {
            return Err(Error::invalid_input("无段边界检查点：请先 compile() 全文编译"));
        }
        self.stats = IncrementalStats::default();
        self.rejects = vec![None; n];
        // 1) 回滚到段 index 前（expand 检查点整体还原 + 排版状态整体还原 +
        //    shipped 截断——之后页面的逐位一致由重放重新装配保证）。
        {
            let bnd = self.bounds[index].as_ref().expect("上方已检查");
            bnd.exp.restore_to(&mut self.expander);
            bnd.layout.restore_full(sink_builder(&mut self.expander));
        }
        self.segments[index] = new_text.to_owned();
        // 2) 从 index 起重放。
        for j in index..n {
            // 判定：被编辑段 / 观测到输出例程 / 无缓存 → 必算；其余按复用判定。
            let outcome: std::result::Result<ReuseVerdict, &'static str> = if j == index {
                Err("被编辑段（必算）")
            } else if self.output_routine {
                Err("观测到 \\output 例程（注入路径不执行例程，保守重排）")
            } else if self.cache[j].is_none() {
                Err("无缓存（合并段 / 前轮执行止于此段之前）")
            } else {
                self.reuse_verdict(j)
            };
            let verdict = match outcome {
                Ok(v) => v,
                Err(reason) => {
                    self.rejects[j] = Some(reason);
                    self.execute_segment(j)?;
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
                let side = &self.bounds[j + 1].as_ref().expect("段后边界检查点").layout;
                side.restore_side_effects(b);
                if verdict == ReuseVerdict::Synced {
                    // 同步态：活状态与录制时段前状态精确一致 → 执行必然复现，推进
                    // 到**录制时**的段后状态（非中性段须显式推进，中性段还原
                    // no-op）。
                    let post = self.cache[j]
                        .as_ref()
                        .expect("已判可复用")
                        .record_post
                        .clone();
                    post.restore_to(&mut self.expander);
                }
                // 偏差路径（依赖判定通过）：段状态中性（判定已闸）→ 执行本不改
                // expand 状态，活状态即该段执行后的正确状态；**不能**还原旧 post
                // ——会把偏差槽（被改宏体）的新值冲掉。
            }
            // 段后边界刷新为活状态：排版半边断页点已变；偏差路径下 expand 半边
            // 也随活状态走——旧链快照不再是当前文档状态，后续 `edit` 回滚到它
            // 会丢掉偏差槽新值。
            self.bounds[j + 1] = Some(self.capture_boundary());
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
        builder.muskip_params = self.expander.muskip_registers();
        builder.muskip_is_mu = [true; 3];
        builder.sync_params(self.expander.params_ref());
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
            && b.math.is_empty()
            && b.pending_box.is_none()
            && b.pending_kind.is_none()
            && !b.shipout_next
    }

    /// 段 `j` 的缓存复用判定（`Err(原因)` = 不可复用，须照常执行）。
    ///
    /// 复用 = 缓存节点流重新注入（行盒免重排）+ 页面装配重跑。按序判定，任一
    /// 不成立即重算：
    ///
    /// 1. **段边界干净**（expand 无悬挂构造 + 排版器垂直模式、无进行中段落/盒/
    ///    数学/组）；
    /// 2. **活 expand 状态 vs 录制时段前状态的偏差**——对照缓存的 `record_exp`
    ///    而非 `bounds[j]`：重放执行前一段后 `bounds[j]` 已刷新成当前文档的新状态
    ///    （改 `\hsize` 等参数的编辑会漏检），录制状态才是该缓存成立的准绳。
    ///    偏差为空 → 同步态（[`ReuseVerdict::Synced`]，执行必然复现同一输出）；
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
    fn reuse_verdict(&mut self, j: usize) -> std::result::Result<ReuseVerdict, &'static str> {
        if !self.expander.boundary_is_clean() {
            return Err("expand 段边界不干净（跨段构造：未闭合组/条件/数学）");
        }
        if !self.builder_shaped() {
            return Err("排版器非干净段边界（进行中段落/盒子/数学/组）");
        }
        let cache = self.cache[j].as_ref().expect("调用方已判 Some");
        let delta = ChainDelta::capture(&self.expander, &cache.record_exp);
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
                .layout
                .builder;
            if !side_effects_match(sink_builder(&mut self.expander), pre) {
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

    /// 捕获段边界检查点（expand + 排版）。
    fn capture_boundary(&mut self) -> DocBoundary {
        DocBoundary {
            exp: StateSnapshot::capture(&self.expander),
            layout: LayoutBoundary::capture(self.builder_mut()),
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
        self.bounds[j + 1] = Some(self.capture_boundary());
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
        self.bounds[j + 1] = Some(self.capture_boundary());
        if self.builder_mut().output_defined {
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
            if b.pending_box.is_some() {
                if ended {
                    b.pending_box = None;
                } else {
                    return Err(Error::invalid_input("\\hbox/\\vbox 后缺少组"));
                }
            }
            if b.shipout_next {
                if ended {
                    b.shipout_next = false;
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
            if !b.math.is_empty() {
                if ended {
                    let _ = b.write16("(end occurred inside a math list)\n".to_string());
                    b.math.clear();
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
                if b.pagination {
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
        let pages = self.builder_mut().shipped.clone();
        let fonts = match &self.fonts {
            Fonts::Tfm(table) => table.borrow().clone(),
            Fonts::Fn { .. } => Vec::new(),
        };
        Ok(CompileOutput { pages, fonts })
    }
}
