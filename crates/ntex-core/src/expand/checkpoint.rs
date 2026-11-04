// ---------- 方法分片：checkpoint.rs（M5 阶段二：段边界完整状态检查点） ----------
//
// 纯状态通道：`capture_checkpoint`（只读捕获）与 `restore_checkpoint`（整体写回）
// 逐字段镜像，不含任何语义分支——`run_source`/主循环/模式机/扫描器零改动。
//
// ## 为什么是"完整快照还原"而不是撤销日志或虚拟组
//
// - **虚拟组**（给段包一层组跑保存栈）：TeX `\def` 是组作用域，段末恢复会把
//   跨段定义一起丢掉——语义改变，不可用；
// - **稀疏撤销日志**（在赋值入口挂记录钩子）：赋值/定义入口散布十余处
//   （`assign_count`/`define_macro_scoped`/`\catcode`/`\fontdimen`/字体表/流…），
//   漏挂一处 = 静默回滚错误；且 `SavedValue` 不覆盖 `\everypar` 等字段——
//   完整性靠人工对账，不可靠；
// - **完整快照还原**（本文件）：捕获即完整（结构上不可能漏字段），还原即整体
//   写回（不可能半途）。代价是每段边界多存一份全量状态（eqtb ~1 槽/名字 +
//   extras ~10KB），段一本就存每段 pre/post 两份 eqtb，本阶段把它们升级为
//   可还原检查点，500 段内存增量 ~10MB 量级。
//
// ## 字段清单（新增引擎状态字段时同步这里）
//
// 引擎可变字段分三类：
// 1. **比较 + 还原**：`eqtb`、[`ValueState`]（catcode/sfcode/lccode/uccode、
//    内部参数、输出例程、寄存器影子表、字体表与数学字体族）；
// 2. **其余值字段**：由 `runtime_digest` 指纹比较，真值由 [`ValueExtras`]
//    携带供还原；精确比较字段中的内部参数、lccode/uccode 与字体表
//    也保留在其中供还原；
// 3. **控制状态**（输入栈/组栈/条件栈/悬挂前缀）→ [`ControlState`]。不进指纹
//    （段边界干净时均为空/默认），脏边界段的回滚靠它才能正确。
//
// **不还原**：`intern`（追加型——还原已驻留名字无害，csid 稳定性反而依赖它）、
// `sink`/`query_sink`（每段各自换新，查询 sink 只在同步展开区内非空）、
// `vfs`/`font_loader`/`use_bytecode`/`watchdog`（配置与运行期设施）。
// 完整性由 `incremental::tests::checkpoint_roundtrip_restores_value_state` 锁定：
// 对每类状态改动做 捕获 → 改动 → 还原 → `value_state()` 必须逐位复原。

// 说明：本文件由 mod.rs `include!` 进 `expand` 模块，与其共享 `use` 导入
// （HashMap/Arc/EqSlot/ValueState/Token/Params 等已在 mod.rs 顶部导入）。

/// 段边界完整状态检查点：还原 = 把全部可变运行时字段写回捕获值。
///
/// 对外是不透明句柄（字段 `pub(crate)`）：M5 阶段三排版层增量管线
/// （ntex-layout）借它做段边界回滚，与阶段二 `SegmentEngine` 同一套
/// 捕获/还原通道；构造与字段访问仍留在本 crate 内。
#[derive(Debug, Clone)]
pub struct EngineCheckpoint {
    /// eqtb 全部槽（比较 + 还原）。
    ///
    /// `Rc` 共享（M5 阶段六第七刀）：捕获 = 引用计数 +1，不复制槽向量；
    /// 活引擎此后首个 eqtb 写经 `Rc::make_mut` 分离（检查点这份不被污染）。
    /// 宏体 `Arc` 共享不变。
    ///
    /// 还原用整体替换：段执行期间新驻留的 cs 槽被截掉——它们在段前确实未定义；
    /// 名字仍在驻留表里，重放该段时按同一 csid 重建同值槽。
    pub(crate) eqtb: Rc<Vec<EqSlot>>,
    /// 值状态（比较口径 = [`ValueState::PartialEq`]：catcode/sfcode/lccode/uccode 表、
    /// 内部参数、输出例程、寄存器影子表、字体表与数学字体族、其余值字段指纹）。
    pub(crate) value: ValueState,
    /// 运行时值字段的真值（还原用；见 [`ValueExtras`]）。
    pub(crate) extras: ValueExtras,
    /// 控制状态（见 [`ControlState`]）。
    pub(crate) control: ControlState,
}

/// 运行时值字段的真值载体（M5 阶段二还原用）。
///
/// 运行时值字段在这里保留真值供还原；内部参数、lccode/uccode、字体表与
/// 数学字体族由 `ValueState` 精确比较。还原遗漏由 `checkpoint_roundtrip` 捕获。
#[derive(Debug, Clone)]
pub(crate) struct ValueExtras {
    pub(crate) params: Params,
    /// `\everypar`/`\everymath`/… token 列表。
    pub(crate) everypar: Vec<Token>,
    pub(crate) everymath: Vec<Token>,
    pub(crate) everyhbox: Vec<Token>,
    pub(crate) everyvbox: Vec<Token>,
    pub(crate) everycr: Vec<Token>,
    pub(crate) everydisplay: Vec<Token>,
    pub(crate) everyeof: Vec<Token>,
    pub(crate) errhelp: Vec<Token>,
    /// pdfTeX 内部 toks 参数（beamer/hyperref 页面属性）。
    pub(crate) pdf_page_attr: Vec<Token>,
    pub(crate) pdf_pages_attr: Vec<Token>,
    pub(crate) lccodes: [i64; 256],
    pub(crate) uccodes: [i64; 256],
    pub(crate) mathcodes: HashMap<u32, u32>,
    pub(crate) delcodes: HashMap<u32, u32>,
    pub(crate) fontdimens: FontDimens,
    /// 刀H：\rpcode/\lpcode 凸出量表（赋值恒全局，与 fontdimens 同族）。
    pub(crate) protrusions: ProtrusionCodes,
    pub(crate) hyphenchars: HashMap<u32, i64>,
    pub(crate) skewchars: HashMap<u32, i64>,
    pub(crate) pdf_last_obj: i64,
    pub(crate) pdf_last_xform: i64,
    pub(crate) pdf_page_resources: Vec<Token>,
    /// 字体表（加载记录/外部名/cs 名；`\font` 副作用）。
    pub(crate) font_loads: Vec<Option<FontLoad>>,
    pub(crate) font_names: Vec<Option<String>>,
    pub(crate) font_cs_names: Vec<Option<String>>,
    pub(crate) math_fonts: [[u32; 16]; 3],
    pub(crate) parshape: Vec<(i64, i64)>,
    pub(crate) penalty_arrays: [Vec<i64>; 4],
    /// 读写流（`\openin`/`\openout`/`\write` 登记；内容级副作用边界是 RFC-3）。
    pub(crate) read_streams: Vec<Option<ReadStream>>,
    pub(crate) write_streams: Vec<Option<WriteStream>>,
    pub(crate) log_write_pending: Vec<Arc<[Token]>>,
    pub(crate) dumped: bool,
    pub(crate) in_math: bool,
    pub(crate) output_active: bool,
}

/// 控制状态：输入栈/组栈/条件栈/对齐状态机/悬挂前缀。
///
/// 决定"接下来从哪读输入、读到什么算什么"。段边界干净时均为空/默认（克隆廉价）；
/// **脏边界**（跨段构造：未闭合 `{`/`\if`/`$`、跨段参数扫描）的段回滚必须带上
/// 它——只还原值状态会把悬挂构造丢掉，重放语义就错了。
#[derive(Debug, Clone)]
pub(crate) struct ControlState {
    pub(crate) stack: Vec<InputFrame>,
    /// 子展开读取下限。
    pub(crate) read_floor: usize,
    pub(crate) cond_stack: Vec<CondFrame>,
    pub(crate) group_level: u32,
    pub(crate) math_left_depth: usize,
    pub(crate) align_frames: Vec<AlignFrame>,
    pub(crate) group_cond_depth: Vec<usize>,
    /// 赋值保存栈（组结束恢复用；悬挂项随段还原）。
    pub(crate) save_stack: Vec<(u32, SavedValue)>,
    pub(crate) global_pending: bool,
    pub(crate) aftergroup: Vec<(u32, Token)>,
    pub(crate) afterassignment: Option<Token>,
    pub(crate) immediate_pending: bool,
    pub(crate) protected_pending: bool,
    pub(crate) outer_pending: bool,
    pub(crate) long_pending: bool,
    pub(crate) unless_pending: bool,
    pub(crate) suppress_expansion: usize,
    pub(crate) expand_only: bool,
    pub(crate) cur_if_type: i32,
    pub(crate) cur_if_branch: i32,
    pub(crate) pending_box_arg: bool,
    pub(crate) pending_box_arg_mode: i64,
    pub(crate) output_trigger_line: usize,
    pub(crate) output_prev_count: usize,
    pub(crate) ended: bool,
    /// 诊断性状态：不影响输出 token 流，但随段还原让错误消息/追踪与全量一致。
    pub(crate) err_snapshot: Option<(u32, usize)>,
    pub(crate) last_tok: Option<String>,
    pub(crate) shown_trace_mode: Option<String>,
    pub(crate) trace_suppress: u32,
    pub(crate) trace_suppress_defer: bool,
    pub(crate) error_anchor: Option<usize>,
    pub(crate) section_label: String,
    pub(crate) debug_expand_caller: &'static str,
}

impl ControlState {
    /// 对**后续可观察输出**可见的控制状态是否一致（转录格式 / 诊断行号 /
    /// 错误恢复快照 / 作业终结标志）。
    ///
    /// 用途：段状态中性判定（[`crate::incremental::snapshot_states_equal`]）。
    /// `ValueState` + eqtb 比较覆盖不到这些字段，但它们在段执行中被改写后
    /// **不会**改 eqtb——只比前两者会把"转录格式已漂移"的段误判为中性，
    /// 复用时不推进活状态、回滚点又沿用旧 `post`，下一轮 `edit` 回滚即恢复
    /// 出旧段的 `shown_trace_mode`（`\tracingcommands` 的模式前缀记忆），
    /// 后续转录与全量不一致（2026-11 P1-1）。
    ///
    /// **刻意排除**：
    /// - `last_tok` / `debug_expand_caller`：纯看门狗/调试观测，不进入任何
    ///   输出；`last_tok` 每 token 必变，纳入即把所有段判成非中性（复用与
    ///   投机全军覆没）。
    /// - `error_anchor`：值扫描报错锚点，生命周期已根治为「一次值扫描内有效，
    ///   扫描结束（含 clamp）即恢复外层值」——段边界上恒为 `None`，粘滞值不
    ///   存在，无需比较；它只在 clamp_dimen 回溯 `l.N` 时起作用。
    /// - 结构帧（输入栈/组栈/条件栈/对齐/悬挂前缀等）：它们的差异要么已由
    ///   `ValueState`/eqtb 反映，要么由复用前的 `boundary_is_clean` 闸挡掉；
    ///   把脏 `post` 的结构帧还原到干净活状态反而是错的。
    ///
    /// **字段清单与 [`Expander::control_output_visible_equal`] 必须同步**：后者
    /// 是"活状态 vs 快照控制态"的同口径版本（链偏差判定用，不能先克隆整份
    /// `ControlState`），两处逐字段镜像，改动任一处都要同步另一处。
    pub(crate) fn output_visible_equal(&self, other: &Self) -> bool {
        self.shown_trace_mode == other.shown_trace_mode
            && self.trace_suppress == other.trace_suppress
            && self.trace_suppress_defer == other.trace_suppress_defer
            && self.output_trigger_line == other.output_trigger_line
            && self.output_prev_count == other.output_prev_count
            && self.err_snapshot == other.err_snapshot
            && self.section_label == other.section_label
            && self.ended == other.ended
    }
}

impl Expander {
    /// 活状态的**输出可见控制态**是否等于 `other`（快照里的控制状态）。
    ///
    /// 用途：[`crate::incremental::snapshot::ChainDelta::capture`] —— 链偏差的
    /// 第三维。`shown_trace_mode`（`\tracingcommands` 的模式前缀记忆）、追踪
    /// 抑制、`output_trigger_line`、`err_snapshot`、`section_label`、`ended`
    /// 这些字段只活在 `ControlState` 里，**既不进** `ValueState` **也不进**
    /// eqtb 槽；只比前两维会把"编辑段只改了这些字段"的链偏差判成同步态 → 其后
    /// 缓存段跳过执行、回滚点却共享旧 `post`（带旧控制态）→ 下一轮 `edit`
    /// 回滚后转录漂移。
    ///
    /// 字段清单**逐项镜像** [`ControlState::output_visible_equal`]（含同样的
    /// 排除项，见该函数文档）；`ControlState` 是 crate 内类型，此处逐字段比较
    /// 免一次整份克隆。两处清单必须同步，改动任一处都要同步另一处。
    pub(crate) fn control_output_visible_equal(&self, other: &ControlState) -> bool {
        self.shown_trace_mode == other.shown_trace_mode
            && self.trace_suppress == other.trace_suppress
            && self.trace_suppress_defer == other.trace_suppress_defer
            && self.output_trigger_line == other.output_trigger_line
            && self.output_prev_count == other.output_prev_count
            && self.err_snapshot == other.err_snapshot
            && self.section_label == other.section_label
            && self.ended == other.ended
    }

    /// 捕获完整状态检查点（只读；M5 阶段二段级回滚 / 阶段三排版层管线用）。
    pub fn capture_checkpoint(&self) -> EngineCheckpoint {
        EngineCheckpoint {
            eqtb: self.eqtb.slots_shared(),
            value: self.value_state(),
            extras: ValueExtras {
                params: self.params,
                everypar: self.everypar_toks.clone(),
                everymath: self.everymath.clone(),
                everyhbox: self.everyhbox_toks.clone(),
                everyvbox: self.everyvbox_toks.clone(),
                everycr: self.everycr_toks.clone(),
                everydisplay: self.everydisplay_toks.clone(),
                everyeof: self.everyeof_toks.clone(),
                errhelp: self.errhelp_toks.clone(),
                pdf_page_attr: self.pdf_page_attr_toks.clone(),
                pdf_pages_attr: self.pdf_pages_attr_toks.clone(),
                lccodes: self.lccodes,
                uccodes: self.uccodes,
                mathcodes: self.mathcodes.clone(),
                delcodes: self.delcodes.clone(),
                fontdimens: self.fontdimens.clone(),
                protrusions: self.protrusions.clone(),
                hyphenchars: self.hyphenchars.clone(),
                skewchars: self.skewchars.clone(),
                pdf_last_obj: self.pdf_last_obj,
                pdf_last_xform: self.pdf_last_xform,
                pdf_page_resources: self.pdf_page_resources.clone(),
                font_loads: self.font_loads.clone(),
                font_names: self.font_names.clone(),
                font_cs_names: self.font_cs_names.clone(),
                math_fonts: self.math_fonts,
                parshape: self.parshape.clone(),
                penalty_arrays: self.penalty_arrays.clone(),
                read_streams: self.read_streams.clone(),
                write_streams: self.write_streams.clone(),
                log_write_pending: self.log_write_pending.clone(),
                dumped: self.dumped,
                in_math: self.in_math,
                output_active: self.output_active,
            },
            control: ControlState {
                stack: self.stack.clone(),
                read_floor: self.read_floor,
                cond_stack: self.cond_stack.clone(),
                group_level: self.group_level,
                math_left_depth: self.math_left_depth,
                align_frames: self.align_frames.clone(),
                group_cond_depth: self.group_cond_depth.clone(),
                save_stack: self.save_stack.clone(),
                global_pending: self.global_pending,
                aftergroup: self.aftergroup.clone(),
                afterassignment: self.afterassignment,
                immediate_pending: self.immediate_pending,
                protected_pending: self.protected_pending,
                outer_pending: self.outer_pending,
                long_pending: self.long_pending,
                unless_pending: self.unless_pending,
                suppress_expansion: self.suppress_expansion,
                expand_only: self.expand_only,
                cur_if_type: self.cur_if_type,
                cur_if_branch: self.cur_if_branch,
                pending_box_arg: self.pending_box_arg,
                pending_box_arg_mode: self.pending_box_arg_mode,
                output_trigger_line: self.output_trigger_line,
                output_prev_count: self.output_prev_count,
                ended: self.ended,
                err_snapshot: self.err_snapshot,
                last_tok: self.last_tok.clone(),
                shown_trace_mode: self.shown_trace_mode.clone(),
                trace_suppress: self.trace_suppress,
                trace_suppress_defer: self.trace_suppress_defer,
                error_anchor: self.error_anchor,
                section_label: self.section_label.clone(),
                debug_expand_caller: self.debug_expand_caller,
            },
        }
    }

    /// 整体写回检查点（M5 阶段二段级回滚：把引擎还原到捕获时刻的状态）。
    ///
    /// 逐字段赋值，与 [`Self::capture_checkpoint`] 镜像；不触碰的字段见文件头
    /// 注释（intern 追加型、sink 每段各自换新、vfs/字体加载器为配置）。
    pub fn restore_checkpoint(&mut self, cp: &EngineCheckpoint) {
        self.eqtb.replace_slots_shared(Rc::clone(&cp.eqtb));
        self.catcodes = cp.value.catcodes.clone();
        self.catcode_tables = cp.value.catcode_tables.clone();
        self.sfcodes = cp.value.sfcodes;
        self.output_toks = cp.value.output_toks.clone();
        self.registers.restore_dirty(&cp.value.registers);
        let x = &cp.extras;
        self.params = x.params;
        self.everypar_toks = x.everypar.clone();
        self.everymath = x.everymath.clone();
        self.everyhbox_toks = x.everyhbox.clone();
        self.everyvbox_toks = x.everyvbox.clone();
        self.everycr_toks = x.everycr.clone();
        self.everydisplay_toks = x.everydisplay.clone();
        self.everyeof_toks = x.everyeof.clone();
        self.errhelp_toks = x.errhelp.clone();
        self.pdf_page_attr_toks = x.pdf_page_attr.clone();
        self.pdf_pages_attr_toks = x.pdf_pages_attr.clone();
        self.lccodes = x.lccodes;
        self.uccodes = x.uccodes;
        self.mathcodes = x.mathcodes.clone();
        self.delcodes = x.delcodes.clone();
        self.fontdimens.replace_from(&x.fontdimens);
        self.protrusions.replace_from(&x.protrusions);
        self.hyphenchars = x.hyphenchars.clone();
        self.skewchars = x.skewchars.clone();
        self.pdf_last_obj = x.pdf_last_obj;
        self.pdf_last_xform = x.pdf_last_xform;
        self.pdf_page_resources = x.pdf_page_resources.clone();
        self.font_loads = x.font_loads.clone();
        self.font_names = x.font_names.clone();
        self.font_cs_names = x.font_cs_names.clone();
        self.math_fonts = x.math_fonts;
        self.parshape = x.parshape.clone();
        self.penalty_arrays = x.penalty_arrays.clone();
        self.read_streams = x.read_streams.clone();
        self.write_streams = x.write_streams.clone();
        self.log_write_pending = x.log_write_pending.clone();
        self.dumped = x.dumped;
        self.in_math = x.in_math;
        self.output_active = x.output_active;
        let c = &cp.control;
        self.stack = c.stack.clone();
        self.read_floor = c.read_floor;
        self.cond_stack = c.cond_stack.clone();
        self.group_level = c.group_level;
        self.math_left_depth = c.math_left_depth;
        self.align_frames = c.align_frames.clone();
        self.group_cond_depth = c.group_cond_depth.clone();
        self.save_stack = c.save_stack.clone();
        self.global_pending = c.global_pending;
        self.aftergroup = c.aftergroup.clone();
        self.afterassignment = c.afterassignment;
        self.immediate_pending = c.immediate_pending;
        self.protected_pending = c.protected_pending;
        self.outer_pending = c.outer_pending;
        self.long_pending = c.long_pending;
        self.unless_pending = c.unless_pending;
        self.suppress_expansion = c.suppress_expansion;
        self.expand_only = c.expand_only;
        self.cur_if_type = c.cur_if_type;
        self.cur_if_branch = c.cur_if_branch;
        self.pending_box_arg = c.pending_box_arg;
        self.pending_box_arg_mode = c.pending_box_arg_mode;
        self.output_trigger_line = c.output_trigger_line;
        self.output_prev_count = c.output_prev_count;
        self.ended = c.ended;
        self.err_snapshot = c.err_snapshot;
        self.last_tok = c.last_tok.clone();
        self.shown_trace_mode = c.shown_trace_mode.clone();
        self.trace_suppress = c.trace_suppress;
        self.trace_suppress_defer = c.trace_suppress_defer;
        self.error_anchor = c.error_anchor;
        self.section_label = c.section_label.clone();
        self.debug_expand_caller = c.debug_expand_caller;
    }

    /// 派生一个**独立执行器**（M6 段组投机并行的地基）。
    ///
    /// 语义：返回值是"与本实例此刻状态等价"的新引擎，二者之后互不影响——
    /// 投机执行（[`crate`] 之下的 `ntex-layout` 段组管线）让每个段在自己的
    /// 执行器上跑，产出与顺序执行逐位相同的段贡献（见 RFC-5 §4 归纳论证）。
    ///
    /// 复制面（其余字段由 [`Self::with_bytecode`] 的默认值承担）：
    /// - **`intern` 表整体克隆**：csid 与主实例逐一对应（注释相同的名字得到
    ///   相同编号），段内 token 解释与主路径同口径；intern 是追加型表，克隆
    ///   不影响主实例后续驻留。
    /// - **运行时状态**：完整检查点（[`Self::capture_checkpoint`] →
    ///   [`Self::restore_checkpoint`]）——eqtb 槽 + 值状态 + 扩展字段真值 +
    ///   控制状态（输入栈/组栈/条件栈/悬挂前缀）。
    /// - **配置与注入设施**：字节码轨道、`\jobname`、`pdf_xobjects`（只追加、
    ///   与检查点同口径不回滚）、诊断环（按容量重建，不共享句柄）。
    /// - **扫描细态**（不在检查点内的可变字段）：照抄——段边界干净时它们本就
    ///   是默认值，照抄使"派生即等价"不依赖调用方的边界假设。
    ///
    /// 不复制：sink（调用方随后 [`Self::set_sink`] 装该段自己的 builder）、
    /// `query_sink`（默认空）、看门狗（[`Self::run`] 每次自行新建）、
    /// `csname_cache`（查询缓存，可重建）。
    ///
    /// `vfs` / `font_loader` **必须由调用方显式给出**——这是 RFC-3 副作用隔离
    /// 的落点：投机段对文件系统/字体装载的访问要么走哨兵（标记 taint 并作废
    /// 该段），要么走私有副本（字体表克隆），绝不能触碰主实例的真实后端。
    pub fn fork_isolated(&self, vfs: Box<dyn Vfs>, font_loader: Box<dyn FontLoader>) -> Self {
        let mut e = Self::with_bytecode(self.use_bytecode);
        e.intern = self.intern.clone();
        e.restore_checkpoint(&self.capture_checkpoint());
        // 配置与注入设施。
        e.vfs = vfs;
        e.font_loader = font_loader;
        e.job_name = self.job_name.clone();
        e.pdf_xobjects = self.pdf_xobjects.clone();
        // 诊断环：同形重建（容量对齐），不共享句柄。
        e.macro_trace = self
            .macro_trace
            .as_ref()
            .map(|t| VecDeque::with_capacity(t.capacity()));
        e.handler_trace = self
            .handler_trace
            .as_ref()
            .map(|_| VecDeque::with_capacity(32));
        e.bc_guard_trace = VecDeque::with_capacity(self.bc_guard_trace.capacity());
        // 扫描细态（`ControlState` 未覆盖的可变字段）。
        e.scanner_status = self.scanner_status;
        e.warning_index = self.warning_index;
        e.cur_cs = self.cur_cs;
        e.align_state = self.align_state;
        e.scope_stack = self.scope_stack.clone();
        e.scope_silent = self.scope_silent.clone();
        e.arg_scan_recovered = self.arg_scan_recovered;
        e.expand_once_recovered = self.expand_once_recovered;
        e.force_eof = self.force_eof;
        e.file_name_in_progress = self.file_name_in_progress;
        e.output_group_level = self.output_group_level;
        e.output_boundary_yield = self.output_boundary_yield;
        e
    }
}

// ---------- 回归锁：eqtb 槽向量 Rc 写时复制（M5 阶段六第七刀） ----------
//
// 两把锁锁住本刀的**唯一**前提：捕获 = 共享（不是深拷贝），写 = 分离（不是
// 就地改共享副本）。前者防"悄悄退回深拷贝"（性能回退且不报错），后者防
// "共享后忘记 make_mut"（检查点/派生引擎被污染 = 语义错）。
//
// 「去修复必红」实证（本地手工验证后已还原）：
// - 把 `capture_checkpoint` 的 `eqtb: self.eqtb.slots_shared()` 换回
//   `self.eqtb.slots().to_vec().into()` → `checkpoint_capture_shares_eqtb_slots`
//   在 `ptr_eq` 断言处红；
// - 把 `Eqtb::slot_mut` 的 `Rc::make_mut` 换成对共享缓冲区就地写 →
//   `fork_isolated_eqtb_cow_isolation` 在"fork 侧写污染主实例"处红。
#[cfg(test)]
mod cow_tests {
    use super::*;
    use ntex_io::MemVfs;

    /// 共享锁：捕获后检查点与活引擎指向同一份槽向量（`Rc` 计数 ≥ 2、指针相等），
    /// 且捕获后活引擎写 eqtb 走 `make_mut` 分离——检查点视图不被污染，还原回旧值。
    #[test]
    fn checkpoint_capture_shares_eqtb_slots() {
        let mut e = Expander::new();
        let _ = e.run_source("\\def\\cow{old}");
        let csid = e.intern.lookup("cow").expect("cow 应已驻留");
        let before = e.eqtb.slot(csid).clone();

        let cp = e.capture_checkpoint();
        {
            let live = e.eqtb.slots_shared();
            assert!(
                Rc::ptr_eq(&live, &cp.eqtb),
                "捕获必须共享槽向量（深拷贝 = 本刀修复被撤销）"
            );
            assert!(
                Rc::strong_count(&live) >= 2,
                "捕获后槽向量应被引擎与检查点共同持有"
            );
        }

        // 活引擎写 eqtb：首次写触发一次 make_mut 分离。
        let _ = e.run_source("\\def\\cow{new}");
        assert!(
            Rc::strong_count(&cp.eqtb) == 1,
            "活引擎写后应与检查点分离（检查点独占旧副本）"
        );
        assert_eq!(
            cp.eqtb[csid as usize], before,
            "检查点视图被活引擎写污染（make_mut 缺失）"
        );
        assert_ne!(e.eqtb.slot(csid), &before, "活引擎应持有新值");

        // 还原：回到旧值，并重新共享同一份槽向量。
        e.restore_checkpoint(&cp);
        assert_eq!(e.eqtb.slot(csid), &before, "还原后应回到捕获时刻的值");
        assert!(
            Rc::ptr_eq(&e.eqtb.slots_shared(), &cp.eqtb),
            "还原后引擎与检查点应重新共享"
        );
    }

    /// fork 隔离锁：`fork_isolated` 后两侧共享槽向量（`Rc`），fork 侧 `\def`
    /// 经 `make_mut` 分离——主实例的槽与后续展开都不受影响。
    ///
    /// 深拷贝实现下本锁天然绿；CoW 下绿即证明"共享 ⇒ 写侧分离"链路完整。
    #[test]
    fn fork_isolated_eqtb_cow_isolation() {
        let mut main = Expander::new();
        let _ = main.run_source("\\def\\cow{main}");
        let csid = main.intern.lookup("cow").expect("cow 应已驻留");
        let before = main.eqtb.slot(csid).clone();

        let mut fork = main.fork_isolated(Box::new(MemVfs::new()), Box::new(NoFontLoader));
        assert!(
            Rc::ptr_eq(&fork.eqtb.slots_shared(), &main.eqtb.slots_shared()),
            "fork 应共享主实例的槽向量"
        );

        let _ = fork.run_source("\\def\\cow{fork}\\def\\fresh{f}");
        assert_ne!(
            fork.eqtb.slot(csid),
            &before,
            "fork 侧写应生效于 fork 自己的槽"
        );
        assert_eq!(
            main.eqtb.slot(csid),
            &before,
            "fork 侧写泄漏到主实例（make_mut 缺失）"
        );
        assert!(
            main.intern.lookup("fresh").is_none(),
            "fork 的 intern 是独立副本"
        );

        // 主实例继续写自己的值：与 fork 的槽仍独立。
        let _ = main.run_source("\\def\\cow{main2}");
        assert_ne!(
            main.eqtb.slot(csid),
            fork.eqtb.slot(csid),
            "主实例写入应只影响自身"
        );
    }
}
