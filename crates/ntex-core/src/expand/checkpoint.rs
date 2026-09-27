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
// 1. **比较 + 还原**：`eqtb`、[`ValueState`]（catcode/sfcode/`\output`/寄存器影子表）；
// 2. **只进指纹的值字段** → [`ValueExtras`]（真值载体；指纹不可逆，没有真值
//    就无法还原）。字段集与 `runtime_digest` 一一对应；
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
    /// eqtb 全部槽（比较 + 还原；宏体 `Arc` 共享，克隆廉价）。
    ///
    /// 还原用整体替换：段执行期间新驻留的 cs 槽被截掉——它们在段前确实未定义；
    /// 名字仍在驻留表里，重放该段时按同一 csid 重建同值槽。
    pub(crate) eqtb: Vec<EqSlot>,
    /// 值状态（比较口径 = [`ValueState::PartialEq`]：catcode/sfcode 表、
    /// `\output` 例程、寄存器影子表、其余值字段指纹）。
    pub(crate) value: ValueState,
    /// 指纹覆盖字段的真值（还原用；见 [`ValueExtras`]）。
    pub(crate) extras: ValueExtras,
    /// 控制状态（见 [`ControlState`]）。
    pub(crate) control: ControlState,
}

/// 指纹覆盖字段的真值载体（M5 阶段二还原用）。
///
/// 字段集与 `Expander::runtime_digest` 一一对应：指纹覆盖到的每一个字段在这里
/// 都有真值——否则回滚后指纹对不上（被 `checkpoint_roundtrip` 测试捕获）。
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
    pub(crate) lccodes: [i64; 256],
    pub(crate) uccodes: [i64; 256],
    pub(crate) mathcodes: HashMap<u32, u32>,
    pub(crate) delcodes: HashMap<u32, u32>,
    pub(crate) fontdimens: FontDimens,
    pub(crate) hyphenchars: HashMap<u32, i64>,
    pub(crate) skewchars: HashMap<u32, i64>,
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

impl Expander {
    /// 捕获完整状态检查点（只读；M5 阶段二段级回滚 / 阶段三排版层管线用）。
    pub fn capture_checkpoint(&self) -> EngineCheckpoint {
        EngineCheckpoint {
            eqtb: self.eqtb.slots().to_vec(),
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
                lccodes: self.lccodes,
                uccodes: self.uccodes,
                mathcodes: self.mathcodes.clone(),
                delcodes: self.delcodes.clone(),
                fontdimens: self.fontdimens.clone(),
                hyphenchars: self.hyphenchars.clone(),
                skewchars: self.skewchars.clone(),
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
        self.eqtb.replace_slots(cp.eqtb.clone());
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
        self.lccodes = x.lccodes;
        self.uccodes = x.uccodes;
        self.mathcodes = x.mathcodes.clone();
        self.delcodes = x.delcodes.clone();
        self.fontdimens.replace_from(&x.fontdimens);
        self.hyphenchars = x.hyphenchars.clone();
        self.skewchars = x.skewchars.clone();
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
}
