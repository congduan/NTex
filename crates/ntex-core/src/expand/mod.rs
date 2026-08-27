//! 展开引擎主循环（M1-6 ~ M1-11 + M2 字节码轨道）。
//!
//! 职责：消费 token 流（源码 / 宏展开 / 字节码 / token 列表），
//! 展开可展开项（宏、`\expandafter`、`\noexpand`、`\the`），
//! 执行不可展开原语（`\def`/`\let`/`\catcode`/寄存器/组等），
//! 其余 token 原样输出。
//!
//! **M2 双轨**：宏调用优先走预编译字节码（[`crate::bytecode`]），
//! 解释器轨道（[`Expander::new_interpreter`]）用于双轨等价验证；
//! 全部测试用例自动双轨重跑断言输出一致。
//!
//! M1 范围说明（后续里程碑补齐）：
//! - M1-7 扫描顺序原语：`\futurelet`/`\aftergroup`/`\afterassignment` 已实现；
//! - M1-8 分隔参数（delimited）未实现（`collect_args` 已支持，宏定义期 `\def#1..#2` 待核）；
//! - M1-10 寄存器 `\count/\dimen/\skip/\toks` 与 `\the` 已实现（`\box`/`\muskip` 未实现）；
//! - M1-11 组作用域（朴素快照回滚 + `\global`）已实现；
//! - 空行 → `\par` 已实现（`scan_token` 行状态机，A2）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::bytecode::{compile, Bytecode};
use crate::catcode::{Catcode, CatcodeTable};
use crate::eqtb::{EqSlot, Eqtb, Primitive, StreamKind};
use crate::error::{Error, Result};
use crate::font::{FontLoader, NoFontLoader};
use crate::input::{scan_token, ScanState};
use crate::intern::InternTable;
use crate::macrodef::{MacroDef, ParamSpec, TokenArray};
use crate::param::{ParamKind, ParamValue, Params};
use crate::register::{
    add_glue, format_count, format_dimen, format_glue, format_mu_glue, unit_to_sp, Glue, RegKind,
    RegisterState, Registers, MAX_DIMEN, MAX_INT, REGISTER_COUNT, SP_PER_PT,
};
use crate::sink::{TokenSink, VecSink};
use crate::token::{meaning, Token, TokenKind};
use ntex_io::{LocalVfs, Vfs};

/// 输入帧：token 来源栈（LIFO，栈顶为当前帧）。
#[derive(Debug)]
enum InputFrame {
    /// 源码帧：字节流 + 扫描位置 + 行状态（空行 → `\par` 判定）。
    Source {
        bytes: Arc<[u8]>,
        pos: usize,
        state: ScanState,
    },
    /// 宏展开帧：宏体 + 实参。
    Macro {
        body: TokenArray,
        pos: usize,
        args: Vec<TokenArray>,
    },
    /// 字节码帧（M2）：预编译指令 + 实参（解释器轨道的替代）。
    Bytecode {
        code: Arc<Bytecode>,
        pc: usize,
        args: Vec<TokenArray>,
    },
    /// token 列表帧：`(token, noexpand 标记)`。
    TokenList {
        items: Arc<[(Token, bool)]>,
        pos: usize,
    },
    /// 单 token 回推槽（B1：`unread`/`$$` 探测/`\noexpand` 回推用，免 Arc 包装）。
    One { tok: Token, noexpand: bool },
    /// 输出例程帧（M3-5-3）：同 TokenList，但耗尽时复位输出例程激活标志。
    OutputRoutine {
        items: Arc<[(Token, bool)]>,
        pos: usize,
    },
}

/// 条件分支状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CondState {
    /// 处理当前分支。
    Processing,
    /// 跳过直到本帧的 `\else`/`\fi`。
    Skipping,
}

/// 条件栈帧（M1-9）。
#[derive(Debug, Clone)]
struct CondFrame {
    /// 是否为 `\ifcase` 帧。
    is_case: bool,
    state: CondState,
    /// 跳过时若为 true：`\else` 会恢复 Processing（false 分支的 then 被跳过）；
    /// 为 false：`\else` 只是被匹配（跳过 else 分支/惰性嵌套帧）。
    owns_skip: bool,
    /// `\ifcase` 跳过计数：还需跳过的 `\or` 数。
    ors_left: Option<usize>,
    else_seen: bool,
    /// 进入该条件前的外层 `cur_if_type`/`cur_if_branch`（`\fi` 时恢复）。
    saved_if_type: i32,
    saved_if_branch: i32,
    /// 本条件类型码（`if_type_code`；输入结束报 `Incomplete \ifxxx` 用）。
    if_type: i32,
    /// 开条件时的源码行号（Incomplete 消息 "after line N"；tex.web final_cleanup）。
    line: usize,
}

/// 条件操作（process_one 拦截的 token）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CondOp {
    If,
    IfCat,
    IfNum,
    IfDim,
    IfX,
    IfOdd,
    IfCase,
    IfTrue,
    IfFalse,
    // e-TeX（M4-5）
    IfDefined,
    IfCsname,
    IfPrimitive,
    IfInner,
    // ETRIP 冲刺：模式/盒子/EOF 条件
    IfVMode,
    IfHMode,
    IfMMode,
    IfEof,
    IfVoid,
    IfHBox,
    IfVBox,
    // ETRIP 冲刺：\iffontchar（字体含字符测试）
    IfFontChar,
    Else,
    Fi,
    Or,
}

impl CondOp {
    fn from_prim(p: Primitive) -> Option<Self> {
        Some(match p {
            Primitive::If => Self::If,
            Primitive::IfCat => Self::IfCat,
            Primitive::IfNum => Self::IfNum,
            Primitive::IfDim => Self::IfDim,
            Primitive::IfX => Self::IfX,
            Primitive::IfOdd => Self::IfOdd,
            Primitive::IfCase => Self::IfCase,
            Primitive::IfTrue => Self::IfTrue,
            Primitive::IfFalse => Self::IfFalse,
            Primitive::IfDefined => Self::IfDefined,
            Primitive::IfCsname => Self::IfCsname,
            Primitive::IfPrimitive => Self::IfPrimitive,
            Primitive::IfInner => Self::IfInner,
            Primitive::IfVMode => Self::IfVMode,
            Primitive::IfHMode => Self::IfHMode,
            Primitive::IfMMode => Self::IfMMode,
            Primitive::IfEof => Self::IfEof,
            Primitive::IfVoid => Self::IfVoid,
            Primitive::IfHBox => Self::IfHBox,
            Primitive::IfVBox => Self::IfVBox,
            Primitive::IfFontChar => Self::IfFontChar,
            Primitive::Else => Self::Else,
            Primitive::Fi => Self::Fi,
            Primitive::Or => Self::Or,
            _ => return None,
        })
    }
}

/// 关系符（`\ifnum`/`\ifdim` 用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Relation {
    Lt,
    Eq,
    Gt,
}

/// 组作用域保存项（M1-11 朴素快照回滚）。
#[derive(Debug)]
enum SavedValue {
    Eqtb {
        csid: u32,
        prev: EqSlot,
    },
    Count {
        idx: usize,
        prev: i64,
    },
    Dimen {
        idx: usize,
        prev: i64,
    },
    Skip {
        idx: usize,
        prev: Glue,
    },
    /// `\muskip`：mu 胶量寄存器（ETRIP；1mu = 65536 单位）。
    Muskip {
        idx: usize,
        prev: Glue,
    },
    Toks {
        idx: usize,
        prev: TokenArray,
    },
    Catcode {
        byte: u8,
        prev: Catcode,
    },
    Param {
        kind: ParamKind,
        prev: ParamValue,
    },
    /// `\sfcode`：spacefactor 表项（M3-4 词间距）。
    Sfcode {
        byte: u8,
        prev: u32,
    },
    /// `\output`：输出例程 token 列表（M3-5-3）。
    Output {
        prev: Option<TokenArray>,
    },
    /// `\fontdimen`：字体参数覆盖（prev None = 此前无覆盖）。
    FontDimen {
        font: u32,
        num: u32,
        prev: Option<i64>,
    },
    /// `\hyphenchar`：字体断字符覆盖（prev None = 此前无覆盖）。
    HyphenChar {
        font: u32,
        prev: Option<i64>,
    },
    /// `\delcode`：定界符码表项（ETRIP；组内局部保存）。
    DelCode {
        byte: u8,
        prev: Option<u32>,
    },
    /// TRIP 冲刺：`\mathcode`：数学码表项（组内局部保存）。
    MathCode {
        byte: u8,
        prev: Option<u32>,
    },
    /// `\lccode`：小写码表项（ETRIP 断字；组内局部保存）。
    LcCode {
        byte: u8,
        prev: i64,
    },
    /// ETRIP 第二波：e-TeX 惩罚数组（\interlinepenalties 等；组内局部保存）。
    /// `kind`：0=interline/1=club/2=widow/3=displaywidow。
    PenaltyArray {
        kind: u8,
        prev: Vec<i64>,
    },
}

/// `\ifx` 语义键：解析别名后比较含义（TeX：同含义即相等）。
#[derive(Debug, Clone, PartialEq)]
enum MeaningKey {
    Undefined,
    /// 宏：含义 = 参数规格 + 宏体（含 `\long`）+ **outer 标志**（tex.web：
    /// `\ifx` 比较 eqtb 条目，`outer` 是 eq_type 的一部分，需区分）。
    Macro {
        def: Arc<MacroDef>,
        outer: bool,
    },
    Primitive(Primitive),
    Char {
        catcode: Catcode,
        charcode: u32,
    },
    Alias(u32),
    Font(u32),
    Register(RegKind, usize),
    Stream(StreamKind, usize),
    /// `\mathchardef` 数学字符（ETRIP）。
    MathChar(u32),
}

/// eqtb 槽执行动作（B1）：先在 `&self` 阶段按引用读槽、提取所需数据
/// （仅拷贝小值/单个 Arc），再在 `&mut self` 阶段执行——避免克隆整个
/// [`EqSlot`] 枚举（含 Macro 槽的 Arc refcount bump）与借用冲突。
#[derive(Debug)]
enum SlotAction {
    Undefined(String),
    Alias(u32),
    Char { catcode: Catcode, charcode: u32 },
    Register(RegKind, usize),
    Stream,
    MathChar(u32),
    Macro(Arc<MacroDef>),
    Font(u32),
    Primitive(Primitive),
}

/// 读流（RFC-3）：`\openin` 时读入内存，`\read` 逐行消费。
#[derive(Debug)]
struct ReadStream {
    /// 目标路径（错误信息用）。
    _path: String,
    /// 文件内容。
    data: Vec<u8>,
    /// 当前字节位置。
    pos: usize,
}

/// 写流（RFC-3）：`\openout` 登记路径，`\write` 入队，flush 边界落盘。
#[derive(Debug)]
struct WriteStream {
    /// 目标路径；None = 未 `\openout`（`\write` 到该流报错）。
    path: Option<String>,
    /// 延迟待写 token 列表（`\write` 入队；flush 时展开落盘）。
    pending: Vec<TokenArray>,
}

/// 展开引擎状态快照（`.fmt` v1，M3 收尾）：可序列化的全部展开状态。
///
/// 数据由 `ntex-format` crate 编码/解码；本结构只承载数据。
/// 限制：字体度量（`EqSlot::Font`）与字节码不随快照（加载时重建）。
#[derive(Debug, Clone, PartialEq)]
pub struct FmtState {
    /// 驻留表名字（csid = 下标；加载时按序重建）。
    pub intern_names: Vec<String>,
    /// catcode 表。
    pub catcodes: CatcodeTable,
    /// `\sfcode` 表。
    pub sfcodes: [u32; 256],
    /// eqtb 全部槽（含原语、宏、别名、字体选择器、寄存器/流引用）。
    pub eqtb: Vec<EqSlot>,
    /// 寄存器文件（count/dimen/skip/toks）。
    pub registers: RegisterState,
    /// 内部参数。
    pub params: Params,
    /// `\output` 例程 token 列表。
    pub output_toks: Option<TokenArray>,
    /// FontId → 外部字体名（`\fontname` 查询用；加载后需重新 `\font`）。
    pub font_names: Vec<Option<String>>,
}

/// 线程看门狗共享状态（挂死诊断）。
///
/// `run()` 主循环每步更新心跳与状态快照；独立线程每 2s 检查心跳，
/// 停更超过 10s（单步内部死循环 / layout 侧死循环导致 process_one 不返回）
/// 即打印最后状态并退出（one-shot）。不参与 `.fmt` 序列化。
#[derive(Debug, Default)]
struct WatchdogShared {
    /// 主循环最后心跳（UNIX 毫秒）。
    heartbeat_ms: AtomicU64,
    /// `run()` 正常结束标志（避免正常完成后误报）。
    done: AtomicBool,
    /// 最近处理 token（每步更新，卡死时必留痕）。
    last_tok: Mutex<String>,
    /// 状态快照（steps / 输入栈），每 5000 步刷新。
    state: Mutex<String>,
}

/// 当前 UNIX 毫秒（线程看门狗心跳用）。
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 展开引擎。
#[derive(Debug)]
pub struct Expander {
    intern: InternTable,
    eqtb: Eqtb,
    /// 线程看门狗共享状态（挂死诊断；不参与 .fmt 序列化，run 时创建）。
    watchdog: Option<Arc<WatchdogShared>>,
    catcodes: CatcodeTable,
    /// `\sfcode` 表（M3-4 词间距 spacefactor；TeX 默认全 1000，plain 对
    /// .,?!=3000、:=2000、;=1500、,=1250，由排版器按 plain 默认初始化）。
    sfcodes: [u32; 256],
    stack: Vec<InputFrame>,
    /// 输出 sink（M3-2）：token/组/原语事件流；默认 [`VecSink`] 收集 token。
    sink: Box<dyn TokenSink>,
    /// 读取下限：`fetch` 只允许从下标 >= 该值的帧读取；
    /// 用于划分子展开（`\edef`/`\expandafter` 区域）的边界。
    read_floor: usize,
    /// 条件栈（M1-9）。
    cond_stack: Vec<CondFrame>,
    /// 组层级（M1-11）。
    group_level: u32,
    /// 对齐组深度（`\halign`/`\valign` 组内 `{`/`}` 不建普通组，TeX alignment 状态机语义）。
    align_depth: i32,
    /// 组开始时的条件栈深度（组结束必须回到该深度）。
    group_cond_depth: Vec<usize>,
    /// 赋值保存栈：组结束时按层回滚（朴素快照回滚）。
    save_stack: Vec<(u32, SavedValue)>,
    /// `\global` 前缀：作用于下一个赋值。
    global_pending: bool,
    /// `\aftergroup`：`(组层级, token)`。
    aftergroup: Vec<(u32, Token)>,
    /// `\afterassignment`：下一个赋值完成后插入的 token。
    afterassignment: Option<Token>,
    /// 寄存器文件（M1-10）。
    registers: Registers,
    /// 内部参数（M3-2-2）：`\parindent`/`\baselineskip`/`\lineskip`/`\lineskiplimit`。
    params: Params,
    /// 字体加载器（M3-4）：`\font` 执行时把字体名解析为 FontId。
    font_loader: Box<dyn FontLoader>,
    /// `\fontname` 查询用：FontId → 外部字体名（`\font` 加载时登记；TRIP L218）。
    font_names: Vec<Option<String>>,
    /// `\output` 例程 token 列表（M3-5-3）；None = 未定义（断页直通 shipout）。
    output_toks: Option<TokenArray>,
    /// 输出例程正在执行（防嵌套：例程内再次断页报错）。
    output_active: bool,
    /// 是否已执行显式 `\end`（finish 收尾对未闭合组/math 按 TeX 语义降级为警告）。
    ended: bool,
    /// 最近一次处理的 token（watchdog/单步超时诊断用；不参与 .fmt 序列化）。
    last_tok: Option<String>,
    /// `\tracingcommands`：上次打印的模式（tex.web shown_mode——模式变化才打前缀）。
    shown_trace_mode: Option<String>,
    /// 子展开（`\write` 内容、marks 查询等 expand_region）追踪抑制计数——
    /// TeX 只在 main_control 主循环追踪（show_cur_cmd_chr），扫描器内部不追踪。
    trace_suppress: u32,
    /// 上一轮注入输出例程时待处理页面的数量（判断例程是否消费了 box255）。
    output_prev_count: usize,
    /// 是否启用字节码轨道（M2；解释器轨道用于双轨等价验证）。
    use_bytecode: bool,
    /// VFS（RFC-3）：文件读写唯一入口。
    vfs: Box<dyn Vfs>,
    /// 读流表（`\openin`/`\read`；下标 = 流号）。
    read_streams: Vec<Option<ReadStream>>,
    /// 写流表（`\openout`/`\write`；下标 = 流号）。
    write_streams: Vec<Option<WriteStream>>,
    /// TRIP：`\write-1`（log-only 特殊流）的延迟待写 token 列表（shipout 边界写 log）。
    log_write_pending: Vec<Arc<[Token]>>,
    /// `\immediate` 前缀（作用于下一个 write/openout/closeout）。
    immediate_pending: bool,
    /// e-TeX（M4-5）：
    /// `\protected` 前缀：下一个 `\def` 定义的宏标记 protected。
    protected_pending: bool,
    /// `\outer` 前缀：下一个 `\def`/`\xdef` 等定义的宏标记 outer。
    /// （当前仅消费前缀；outer 语义限制——实参不得含 outer 宏——后续迭代补。）
    outer_pending: bool,
    /// `\long` 前缀：下一个 `\def` 等定义的宏允许参数中含 `\par`。
    long_pending: bool,
    /// `\fontdimen` 覆盖表：(font_id, 参数号) → 值（sp）。TFM 度量在排版层，
    /// 此处仅存覆盖项；无覆盖读回 0（后续接入 TFM 时回退真实参数）。
    fontdimens: HashMap<(u32, u32), i64>,
    /// `\hyphenchar` 覆盖表：font_id → 断字符码（无覆盖 = 字体默认 45）。
    hyphenchars: HashMap<u32, i64>,
    /// `\delcode` 表：字符码 → 定界符码（TeX delcode；无覆盖 = 0x500000 默认）。
    delcodes: HashMap<u32, u32>,
    /// TRIP 冲刺：`\mathcode` 表：字符码 → 数学码（TeX initex 默认：
    /// catcode 11/12 字符 = 0x7000+码，其余 = 0x8000 无效）。
    mathcodes: HashMap<u32, u32>,
    /// `\lccode` 表：字符码 → 小写码（TeX 默认全 0；etrip 断字测试用）。
    lccodes: [i64; 256],
    /// TRIP 冲刺：`\uccode` 表：字符码 → 大写码（TeX 默认全 0；`\uppercase` 用）。
    uccodes: [i64; 256],
    /// ETRIP 冲刺：`\dump` 已执行（initex 收尾；驱动据此保存 fmt 并二次运行）。
    dumped: bool,
    /// `\unless` 前缀：取反下一个条件的结果。
    unless_pending: bool,
    /// e-TeX 只读整数（`\currentiftype`/`\currentifbranch`）：当前最内层条件的
    /// 类型码（0=无；负号 = `\unless` 前缀）与分支（0=未决、+1=true、-1=false）。
    /// TeX 语义：`\if*` 遇到时立即置新值（参数扫描期间 branch=0），\fi 恢复外层。
    cur_if_type: i32,
    cur_if_branch: i32,
    /// protected 宏抑制展开的上下文深度（>0：`\edef`/`\write`/`\detokenize` 等）。
    suppress_expansion: usize,
    /// `\edef`/`\xdef`/`\write` 展开上下文（TeX `expand()`）：只展开可展开项，
    /// 不可展开原语/未定义 cs/字符/组定界原样保留在输出（不执行、不建组）。
    expand_only: bool,
    /// 临时调试：expand_region 的调用来源（"edef"/"write"）。
    debug_expand_caller: &'static str,
    /// 数学字体族已赋值表：[字体样式 0=text/1=script/2=scriptscript][族号] → FontId
    /// （ETRIP：`\scriptfont1=\textfont1` 等族间复制与 `\textfont<n>` 字体位置读取）。
    math_fonts: [[u32; 16]; 3],
    /// 最近一次 `\typeout{Checking ...}` 段标题（ETRIP 错误定位：出错时报告卡在哪个段）。
    section_label: String,
    /// `\parshape` 段落形状表：(缩进, 宽度)（sp；ETRIP：\parshapelength/indent/dimen 读取）。
    parshape: Vec<(i64, i64)>,
    /// ETRIP 第二波：e-TeX 惩罚数组（key: 0=interline/1=club/2=widow/3=displaywidow）。
    /// `\interlinepenalties n p1 ... pn` 等：扫描 n 个 penalty 值存储（断页器后续读取）。
    /// 当前仅在 expander 侧存储，未镜像给排版器（pass2 仅需扫描语义正确即可推进）。
    penalty_arrays: [Vec<i64>; 4],
    /// TRIP 冲刺：`\everymath` token 列表（进入数学模式时注入输入栈）。
    everymath: Vec<Token>,
    /// TRIP 冲刺：当前是否处于数学模式（`$`/`$$` 切换；决定 everymath 注入时机）。
    in_math: bool,
}

impl Expander {
    /// 创建引擎（字节码轨道）并注册 M1 内建原语。
    pub fn new() -> Self {
        Self::with_bytecode(true)
    }

    /// 创建解释器轨道引擎（M2 双轨等价验证用）。
    pub fn new_interpreter() -> Self {
        Self::with_bytecode(false)
    }

    fn with_bytecode(use_bytecode: bool) -> Self {
        let mut e = Self {
            intern: InternTable::new(),
            eqtb: Eqtb::new(),
            catcodes: CatcodeTable::new(),
            sfcodes: [1000; 256],
            stack: Vec::new(),
            sink: Box::new(VecSink::default()),
            read_floor: 0,
            cond_stack: Vec::new(),
            group_level: 0,
            align_depth: 0,
            group_cond_depth: Vec::new(),
            save_stack: Vec::new(),
            global_pending: false,
            aftergroup: Vec::new(),
            afterassignment: None,
            registers: Registers::new(),
            params: Params::default(),
            font_loader: Box::new(NoFontLoader),
            font_names: Vec::new(),
            watchdog: None,
            output_toks: None,
            output_active: false,
            ended: false,
            last_tok: None,
            shown_trace_mode: None,
            trace_suppress: 0,
            output_prev_count: usize::MAX,
            use_bytecode,
            vfs: Box::new(LocalVfs),
            read_streams: Vec::new(),
            write_streams: Vec::new(),
            log_write_pending: Vec::new(),
            immediate_pending: false,
            protected_pending: false,
            outer_pending: false,
            long_pending: false,
            fontdimens: HashMap::new(),
            hyphenchars: HashMap::new(),
            delcodes: HashMap::new(),
            // TRIP 冲刺：initex 默认 mathcode（tex.web `init_math_codes`）：
            // letter/other_char（catcode 11/12）→ 0x7000+码（class 7 variable, family 0），
            // 其余 → 0x8000（无效，触发 "Missing character" 语义一致）。
            mathcodes: default_mathcodes(),
            lccodes: [0; 256],
            uccodes: [0; 256],
            dumped: false,
            unless_pending: false,
            cur_if_type: 0,
            cur_if_branch: 0,
            suppress_expansion: 0,
            expand_only: false,
            debug_expand_caller: "",
            math_fonts: [[0; 16]; 3],
            section_label: String::new(),
            parshape: Vec::new(),
            penalty_arrays: Default::default(),
            everymath: Vec::new(),
            in_math: false,
        };
        e.register_builtins();
        e
    }

    /// 注入 VFS 后端（RFC-3；默认本地文件系统）。
    pub fn set_vfs(&mut self, vfs: Box<dyn Vfs>) {
        self.vfs = vfs;
    }

    /// 取回 VFS（测试断言写入内容用）。
    pub fn take_vfs(&mut self) -> Box<dyn Vfs> {
        std::mem::replace(&mut self.vfs, Box::new(LocalVfs))
    }

    // ---------- M3 收尾（`.fmt` v1 内存快照） ----------

    /// 导出引擎状态快照（`.fmt` v1）。
    ///
    /// 范围：intern 表、catcode、sfcode、eqtb（含宏定义/版本）、寄存器、内部参数、
    /// `\output` 例程。**不含**：字体表（`EqSlot::Font` 的度量在排版器侧，
    /// 加载后需重新 `\font`）、字节码（加载时按需重建）、VFS 与流状态（运行时）。
    pub fn export_state(&self) -> FmtState {
        FmtState {
            intern_names: self.intern.names_vec(),
            catcodes: self.catcodes.clone(),
            sfcodes: self.sfcodes,
            eqtb: self.eqtb.slots().to_vec(),
            registers: self.registers.export(),
            params: self.params,
            output_toks: self.output_toks.clone(),
            font_names: self.font_names.clone(),
        }
    }

    /// 加载引擎状态快照（`.fmt` v1）：整体替换展开状态，运行时栈清零。
    ///
    /// 宏定义在字节码轨道下重建预编译字节码（`code` 不随快照序列化）。
    pub fn import_state(&mut self, state: FmtState) {
        // 重建 intern（按名字顺序驻留 → csid 与导出时一致）
        let mut intern = InternTable::new();
        for name in &state.intern_names {
            intern.intern(name);
        }
        self.intern = intern;
        self.catcodes = state.catcodes;
        self.sfcodes = state.sfcodes;
        // eqtb 替换；字节码轨道下补编译缺失的宏字节码
        let mut eqtb = Eqtb::new();
        eqtb.replace_slots(state.eqtb);
        if self.use_bytecode {
            for csid in 0..eqtb.slots().len() as u32 {
                // 先取出待编译的宏体，再重建 MacroDef（保留版本号）
                let pending = match eqtb.slot(csid) {
                    EqSlot::Macro(v) if v.value.code.is_none() => Some((
                        v.value.params.clone(),
                        v.value.body.clone(),
                        v.value.protected,
                        v.value.outer,
                    )),
                    _ => None,
                };
                if let Some((params, body, protected, outer)) = pending {
                    let code = Arc::new(compile(&body));
                    if let EqSlot::Macro(v) = eqtb.slot_mut(csid) {
                        v.value = Arc::new(MacroDef {
                            params,
                            body,
                            code: Some(code),
                            protected,
                            outer,
                        });
                    }
                }
            }
        }
        self.eqtb = eqtb;
        self.registers = Registers::import(state.registers);
        self.params = state.params;
        self.output_toks = state.output_toks;
        self.font_names = state.font_names;
        // 运行时状态重置（新文档起点）
        self.stack.clear();
        self.read_floor = 0;
        self.cond_stack.clear();
        self.group_level = 0;
        self.align_depth = 0;
        self.group_cond_depth.clear();
        self.save_stack.clear();
        self.global_pending = false;
        self.protected_pending = false;
        self.outer_pending = false;
        self.long_pending = false;
        self.fontdimens.clear();
        self.aftergroup.clear();
        self.afterassignment = None;
        self.output_active = false;
        self.output_prev_count = usize::MAX;
        self.immediate_pending = false;
        self.read_streams.clear();
        self.write_streams.clear();
        self.suppress_expansion = 0;
        self.expand_only = false;
    }

    /// 追加一个源码输入（后续 `\input`/VFS 在 M3 接入）。
    pub fn feed_source(&mut self, text: impl Into<Vec<u8>>) {
        self.stack.push(InputFrame::Source {
            bytes: Arc::from(text.into()),
            pos: 0,
            state: ScanState::LineStart,
        });
    }

    /// 喂入并运行至输入耗尽。
    pub fn run_source(&mut self, text: &str) -> Result<()> {
        self.feed_source(text);
        self.run()
    }

    /// 运行主循环直到输入耗尽。
    ///
    /// 输出例程（M3-5-3）在 token 边界注入：fire_up（发生在 sink 调用内）把页面
    /// 放入 box255 并置 pending；本循环在每次取 token 前检查并注入例程 token 帧。
    pub fn run(&mut self) -> Result<()> {
        let mut steps = 0u64;
        let mut step_start = std::time::Instant::now();
        // 线程看门狗：独立执行上下文，主线程卡在单步内部（process_one 不返回）
        // 或 layout 侧死循环时仍能打印最后状态（TRIP L338 挂死定位）。
        let wd = Arc::new(WatchdogShared::default());
        self.watchdog = Some(wd.clone());
        {
            let wd = wd.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_millis(2000));
                if wd.done.load(Ordering::Relaxed) {
                    return;
                }
                let now = now_millis();
                let hb = wd.heartbeat_ms.load(Ordering::Relaxed);
                if now.saturating_sub(hb) > 10_000 {
                    let last = wd
                        .last_tok
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone();
                    let state = wd.state.lock().unwrap_or_else(|p| p.into_inner()).clone();
                    eprintln!(
                        "[watchdog] 疑似挂死：{}ms 无心跳。last_tok={last} {state}",
                        now.saturating_sub(hb)
                    );
                    return; // one-shot：报一次即退，避免刷屏
                }
            });
        }
        loop {
            // 看门狗：防死循环（ETRIP 诊断用；正常作业远低于此）
            steps += 1;
            // 心跳 + last_tok（每步）+ 状态快照（每 5000 步，卡死时保留最后状态）
            wd.heartbeat_ms.store(now_millis(), Ordering::Relaxed);
            *wd.last_tok.lock().unwrap_or_else(|p| p.into_inner()) =
                self.last_tok.clone().unwrap_or_default();
            if steps % 5000 == 0 {
                let state = format!(
                    "steps={steps} last_tok={:?} stack={}",
                    self.last_tok,
                    self.debug_stack_summary()
                );
                *wd.state.lock().unwrap_or_else(|p| p.into_inner()) = state;
            }
            if steps > 10_000_000 {
                wd.done.store(true, Ordering::Relaxed);
                return Err(Error::invalid_input(format!(
                    "处理步骤超限（疑似死循环）；输入栈深 {}：{}",
                    self.stack.len(),
                    self.debug_stack_summary()
                )));
            }
            // 诊断：定期进度日志（stderr 实时可见，进程被 SIGKILL 也不丢）
            if steps % 50_000 == 0 {
                eprintln!(
                    "[watchdog] steps={steps} elapsed={:.1}s last_tok={:?} stack={}",
                    step_start.elapsed().as_secs_f32(),
                    self.last_tok,
                    self.debug_stack_summary()
                );
            }
            // 诊断：单步耗时看门狗——检查放 step **之前**（卡在单步内部时永远到不了
            // step 之后的检查点）。超时 dump 当前状态（TRIP L338 `\halign` 内挂死）。
            if step_start.elapsed().as_secs() >= 5 {
                eprintln!(
                    "[watchdog] 单步超时 5s steps={steps} last_tok={:?} stack={}",
                    self.last_tok,
                    self.debug_stack_summary()
                );
                step_start = std::time::Instant::now();
            }
            // A3：单步执行（注入输出例程 / flush 写流 / 处理一个 token）——
            // 出错时先写 `l.N` 上下文行到转录，再上抛（TeX error() 的上下文行）。
            let step = (|| -> Result<bool> {
                // 输出例程激活期间（例程帧在栈上）不重复注入
                if !self.output_active && self.maybe_inject_output()? {
                    return Ok(true);
                }
                // RFC-3：页面真正输出（shipout 边界）时 flush 延迟写流
                if self.sink.take_write_flush_pending() {
                    self.flush_writes()?;
                }
                self.process_one()
            })();
            match step {
                Ok(false) => break,
                Ok(true) => {}
                Err(e) => {
                    wd.done.store(true, Ordering::Relaxed);
                    self.report_error_context();
                    return Err(e);
                }
            }
        }
        wd.done.store(true, Ordering::Relaxed);
        if !self.cond_stack.is_empty() {
            // TeX final_cleanup：输入结束（\endinput/EOF）时未闭合条件 → 可恢复
            // 转录消息 "! Incomplete \ifxxx; all text was ignored after line N."
            // （tex.web final_cleanup；TRIP L363 `\ifcase3` 故意不闭合，参考 log
            // 报此消息后正常结束），**不终止作业**（旧实现 return Err 曾导致
            // TRIP 全量在 \endinput 后报"条件未闭合"内部错误）。
            #[cfg(debug_assertions)]
            {
                let frames: Vec<String> = self
                    .cond_stack
                    .iter()
                    .map(|f| {
                        format!(
                            "{{is_case={} state={:?} owns_skip={} else={} line={}}}",
                            f.is_case, f.state, f.owns_skip, f.else_seen, f.line
                        )
                    })
                    .collect();
                eprintln!(
                    "[debug] 条件未闭合: depth={} frames={:?}",
                    self.cond_stack.len(),
                    frames
                );
            }
            for f in self.cond_stack.iter() {
                let _ = self.sink.write16(format!(
                    "! Incomplete {}; all text was ignored after line {}.\n",
                    Self::if_type_name(f.if_type),
                    f.line
                ));
            }
            self.cond_stack.clear();
        }
        Ok(())
    }

    /// A3：当前源码上下文——从输入栈找最近的 [`InputFrame::Source`] 帧，
    /// 由扫描位置反推 (行号, 行内容)。宏展开中的错误回退到最近的源文件行
    /// （TeX `l.N` 上下文行语义）。
    fn error_context(&self) -> Option<(usize, String)> {
        for frame in self.stack.iter().rev() {
            if let InputFrame::Source { bytes, pos, .. } = frame {
                let bytes: &[u8] = bytes;
                let end = (*pos).min(bytes.len());
                let line_no = bytes[..end].iter().filter(|&&b| b == b'\n').count() + 1;
                let line_start = bytes[..end]
                    .iter()
                    .rposition(|&b| b == b'\n')
                    .map(|i| i + 1)
                    .unwrap_or(0);
                let line_end = bytes[line_start..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map(|i| line_start + i)
                    .unwrap_or(bytes.len());
                let line = String::from_utf8_lossy(&bytes[line_start..line_end]).into_owned();
                return Some((line_no, line));
            }
        }
        None
    }

    /// A3：把错误上下文行（`l.N <行内容>`）写入转录（TeX error() 的上下文行）。
    fn report_error_context(&mut self) {
        if let Some((n, line)) = self.error_context() {
            let _ = self.sink.write16(format!("l.{n} {line}\n"));
        }
    }

    /// TeX scan_int 缺数恢复：`! Missing number, treated as zero.` + 上下文行
    /// （TRIP 冲刺：`\countdef\countz` 等缺操作数原语）。
    fn report_missing_number(&mut self) {
        let mut msg = "! Missing number, treated as zero.\n".to_string();
        if let Some((n, line)) = self.error_context() {
            msg.push_str(&format!("l.{n} {line}\n"));
        }
        let _ = self.sink.write16(msg);
    }

    /// 是否已执行显式 `\end`（finish 对未闭合组/math 按 TeX 语义降级为警告）。
    pub fn is_ended(&self) -> bool {
        self.ended
    }

    /// `\tracingcommands` 的 token 描述（tex.web print_cmd_chr 语义：
    /// 控制序列 `\名`；字符按 catcode 分类显示 "the letter A" / "blank space  " 等）。
    fn trace_token_desc(&self, tok: Token) -> String {
        if let Some(csid) = tok.csid() {
            return format!("\\{}", self.intern.name(csid));
        }
        let c = tok
            .charcode()
            .and_then(char::from_u32)
            .unwrap_or('\u{FFFD}');
        match tok.catcode() {
            Some(Catcode::Letter) => format!("the letter {c}"),
            Some(Catcode::Other) => format!("the character {c}"),
            // 描述后跟空格字符本身（tex.web chr_cmd：print 描述 + print_char(chr)）
            Some(Catcode::Space) => format!("blank space {c}"),
            Some(Catcode::BeginGroup) => format!("begin-group character {c}"),
            Some(Catcode::EndGroup) => format!("end-group character {c}"),
            Some(Catcode::MathShift) => format!("math shift character {c}"),
            Some(Catcode::Parameter) => format!("macro parameter character {c}"),
            Some(Catcode::Superscript) => format!("superscript character {c}"),
            Some(Catcode::Subscript) => format!("subscript character {c}"),
            _ => format!("the character {c}"),
        }
    }

    /// 诊断：输入栈摘要（watchdog 超限 / 单步超时 / 定期进度 dump 用）。
    fn debug_stack_summary(&self) -> String {
        self.stack
            .iter()
            .map(|f| match f {
                InputFrame::Source { bytes, pos, state } => {
                    format!("Source({}B,pos={},state={:?})", bytes.len(), pos, state)
                }
                InputFrame::Macro { body, pos, .. } => {
                    format!("Macro({}tok,pos={})", body.len(), pos)
                }
                InputFrame::Bytecode { pc, .. } => format!("Bytecode(pc={})", pc),
                InputFrame::TokenList { items, pos } => {
                    format!("TokenList({}tok,pos={})", items.len(), pos)
                }
                InputFrame::One { tok, .. } => format!("One({tok:?})"),
                InputFrame::OutputRoutine { items, pos } => {
                    format!("OutputRoutine({}tok,pos={})", items.len(), pos)
                }
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// 输入耗尽后的收尾：执行所有待执行的输出例程（`finish` 冲页产生）。
    pub fn run_pending_output(&mut self) -> Result<()> {
        while !self.output_active && self.maybe_inject_output()? {
            // 运行例程帧直到其耗尽（output_active 复位）
            while self.output_active && self.process_one()? {}
            // RFC-3：例程内 `\shipout` 产出页面 → flush 延迟写流
            if self.sink.take_write_flush_pending() {
                self.flush_writes()?;
            }
        }
        Ok(())
    }

    /// 若存在待执行的输出例程，注入其 token 帧并返回 true。
    ///
    /// 注入前比较队列长度与上一轮注入时的长度：未减少（例程没取用 box255）说明
    /// 例程不会处理剩余页面（如 `\output={}`）→ 丢弃剩余并停止（TeX：例程不
    /// ship box255 则页面消失）。丢弃在 token 边界进行，避免例程末 token 的
    /// 参数扫描（如 `\box255` 的数字）误触"例程结束"。
    fn maybe_inject_output(&mut self) -> Result<bool> {
        if self.output_toks.is_none() || !self.sink.output_pending() {
            self.output_prev_count = usize::MAX;
            return Ok(false);
        }
        if self.output_active {
            return Err(Error::invalid_input(
                "输出例程内再次触发了断页（嵌套输出例程）",
            ));
        }
        let count = self.sink.output_pending_count();
        if count >= self.output_prev_count {
            // 例程未消费任何待处理页面 → 剩余页面被丢弃（TeX 语义）
            self.sink.discard_pending_pages();
            self.output_prev_count = usize::MAX;
            return Ok(false);
        }
        self.output_prev_count = count;
        let toks = self.output_toks.clone().expect("已检查 is_some");
        self.sink.take_output_pending();
        self.output_active = true;
        let items: Vec<(Token, bool)> = toks.iter().map(|&t| (t, false)).collect();
        self.stack.push(InputFrame::OutputRoutine {
            items: Arc::from(items),
            pos: 0,
        });
        Ok(true)
    }

    /// 安装字体加载器（M3-4）：`\font` 执行时把字体名解析为 FontId。
    pub fn set_font_loader(&mut self, loader: Box<dyn FontLoader>) {
        self.font_loader = loader;
    }

    /// 单步处理一个 token；返回 false 表示输入耗尽。
    fn process_one(&mut self) -> Result<bool> {
        match self.fetch()? {
            None => Ok(false),
            Some((tok, noexpand)) => {
                // 诊断：记录最近处理的 token（watchdog/单步超时 dump 用；cs 显示真实名字）
                self.last_tok = Some(match tok.csid() {
                    Some(csid) => format!("\\{}", self.intern.name(csid)),
                    None => format!("{tok:?}"),
                });
                // \tracingcommands（misc 下标 3）：每命令一行 `{模式: 描述}`；
                // 模式只在变化时打印（tex.web show_cur_cmd_chr 的 shown_mode 语义）；
                // 子展开（expand_region：\write 内容等）抑制——TeX 只在主循环追踪。
                if self.params.misc[3] > 0 && self.trace_suppress == 0 {
                    let m = self.sink.mode_name();
                    let desc = self.trace_token_desc(tok);
                    let line = if self.shown_trace_mode.as_deref() == Some(m.as_str()) {
                        format!("{{{desc}}}\n")
                    } else {
                        self.shown_trace_mode = Some(m.clone());
                        format!("{{{m}: {desc}}}\n")
                    };
                    let _ = self.sink.write16(line);
                }
                // noexpand（`\noexpand`/`\unexpanded` 输出）：临时不可展开，原样输出。
                // 优先于条件机拦截——`\unexpanded{\ifx...}` 里的条件 token 是数据，
                // 不得 push 条件帧，也不得匹配外层 `\else`/`\fi`。
                if noexpand {
                    self.sink.token(tok)?;
                    return Ok(true);
                }
                // 条件 token（\if*/\\else/\\fi/\\or）优先由条件机处理（无论是否跳过）
                if let Some(op) = self.cond_op(tok) {
                    self.step_conditional(op)?;
                    return Ok(true);
                }
                if self.is_skipping() {
                    // 跳过模式：其余 token 直接丢弃（不展开）
                    return Ok(true);
                }
                self.process_token(tok)?;
                Ok(true)
            }
        }
    }

    /// 处理单个 token（展开宏/原语，其余输出）。
    fn process_token(&mut self, tok: Token) -> Result<()> {
        // \edef/\xdef/\write 展开上下文：只展开可展开项，其余保留
        if self.expand_only {
            return self.process_expand_only(tok);
        }
        match tok.kind() {
            TokenKind::ControlSeq => {
                let csid = tok.csid().expect("ControlSeq 必有 csid");
                let action = match self.eqtb.slot(csid) {
                    // M1-13 错误恢复（ETRIP）：未定义 cs 报 "! Undefined control
                    // sequence." 到转录并**当 \relax 继续**（TeX 错误恢复；上下文行
                    // "l.N …" 留 M1-13 后续细化）。
                    EqSlot::Undefined => SlotAction::Undefined(self.intern.name(csid).to_owned()),
                    EqSlot::Alias(target) => SlotAction::Alias(*target),
                    EqSlot::Char { catcode, charcode } => SlotAction::Char {
                        catcode: *catcode,
                        charcode: *charcode,
                    },
                    // \countdef\cs 等绑定的寄存器 cs：执行位置为赋值（`\cs=<值>`，
                    // TeX 中 `=` 可选）；非赋值上下文（\the/\advance/\ifnum 等）由
                    // 各扫描函数处理，不会到达此处。
                    EqSlot::Register(kind, idx) => SlotAction::Register(*kind, *idx),
                    EqSlot::Stream(..) => SlotAction::Stream,
                    EqSlot::MathChar(code) => SlotAction::MathChar(*code),
                    EqSlot::Macro(m) => SlotAction::Macro(m.value.clone()),
                    EqSlot::Font(font) => SlotAction::Font(*font),
                    EqSlot::Primitive(p) => SlotAction::Primitive(*p),
                };
                match action {
                    SlotAction::Undefined(name) => {
                        // A3：TeX 错误格式 `! 消息` + 上下文行 `l.N <行内容>`；
                        // 未定义 cs 当 \relax 继续（TeX 错误恢复）。
                        let mut msg = format!("! Undefined control sequence.\n\\{name}\n");
                        if let Some((n, line)) = self.error_context() {
                            msg.push_str(&format!("l.{n} {line}\n"));
                        }
                        let _ = self.sink.write16(msg);
                        Ok(())
                    }
                    SlotAction::Alias(target) => {
                        self.process_token(Token::control_sequence(target))
                    }
                    // \let\cs=<字符>：等价于该字符（\bgroup/\egroup 等组定界也生效）
                    SlotAction::Char { catcode, charcode } => {
                        let c = Token::char(catcode, charcode);
                        match catcode {
                            Catcode::BeginGroup => self.begin_group(),
                            Catcode::EndGroup => self.end_group(),
                            _ => self.sink.token(c),
                        }
                    }
                    SlotAction::Register(kind, idx) => {
                        self.skip_spaces()?;
                        self.expect_equals()?;
                        match kind {
                            RegKind::Count => {
                                let val = self.scan_number()?;
                                self.assign_count(idx, val);
                            }
                            RegKind::Dimen => {
                                let val = self.scan_dimen()?;
                                self.assign_dimen(idx, val);
                            }
                            RegKind::Skip => {
                                let val = self.scan_glue()?;
                                self.assign_skip(idx, val);
                            }
                            RegKind::Muskip => {
                                let val = self.scan_glue_mu()?;
                                self.assign_muskip(idx, val);
                            }
                            RegKind::Toks => {
                                // RHS 可为 `{token list}` 或另一 toks 寄存器（内容复制，
                                // TRIP L418 `\tokens\toks1`）——统一走 scan_toks_rhs；
                                // 旧的 scan_register_index 路径会把 `\toks1` 的 `\toks`
                                // 原语误判为数字（Missing number）后再丢回主循环。
                                let val = self.scan_toks_rhs()?;
                                self.assign_toks(idx, val);
                            }
                        }
                        Ok(())
                    }
                    SlotAction::Stream => Err(Error::invalid_input(
                        "流引用不能直接使用（需在 \\read/\\write 等扫描上下文中）",
                    )),
                    // ETRIP 冲刺：\mathchardef\cs=<num> 绑定的 cs 执行时输出字符
                    // （数学原子语义在排版器侧细化；此处按 \char 处理）
                    SlotAction::MathChar(code) => {
                        self.sink.token(Token::char(Catcode::Other, code & 0xFF))
                    }
                    SlotAction::Macro(def) => {
                        // e-TeX（M4-5）：protected 宏在展开抑制上下文（\edef/\write 等）
                        // 不展开，原样输出。
                        if def.protected && self.suppress_expansion > 0 {
                            self.sink.token(Token::control_sequence(csid))?;
                            return Ok(());
                        }
                        self.call_macro(csid, def)
                    }
                    SlotAction::Font(font) => self.sink.font_selected(font),
                    SlotAction::Primitive(p) => {
                        // 诊断（NTEX_TRACE_EXEC=1）：打印每个执行的原语，定位挂死点
                        if std::env::var("NTEX_TRACE_EXEC").is_ok() {
                            eprintln!("[trace-exec] {p:?}");
                        }
                        self.exec_primitive(p)
                    }
                }
            }
            TokenKind::Char => {
                // 数学移位（$，cat 3）：peek 下一个 token 判定 `$$`（显示数学），
                // 交给 sink 按自身模式决定进出（M4-1）。
                if tok.catcode() == Some(Catcode::MathShift) {
                    let display = self.next_is_math_shift()?;
                    let entering = !self.in_math;
                    self.in_math = !self.in_math;
                    // TRIP 冲刺：进入数学模式时注入 `\everymath`（TeX `$` 处理语义）
                    if entering && !self.everymath.is_empty() {
                        let items = self
                            .everymath
                            .iter()
                            .map(|t| (*t, false))
                            .collect::<Vec<_>>();
                        self.stack.push(InputFrame::TokenList {
                            items: Arc::from(items),
                            pos: 0,
                        });
                    }
                    return self.sink.math_shift(display);
                }
                // 组定界符（cat 1/2）在主流层建立/结束组（M1-11）。
                // alignment 组内（`\halign`/`\valign`）：`{`/`}` 只调整对齐深度，
                // 不建立普通组（TeX alignment 状态机语义，`\cr`/`&` 由布局层消费）。
                match tok.catcode() {
                    Some(Catcode::BeginGroup) if self.align_depth > 0 => {
                        self.align_depth += 1;
                        Ok(())
                    }
                    Some(Catcode::EndGroup) if self.align_depth > 0 => {
                        self.align_depth -= 1;
                        if self.align_depth == 0 {
                            self.end_group()
                        } else {
                            Ok(())
                        }
                    }
                    Some(Catcode::BeginGroup) => self.begin_group(),
                    Some(Catcode::EndGroup) => self.end_group(),
                    _ => self.sink.token(tok),
                }
            }
            _ => self.sink.token(tok),
        }
    }

    /// 展开上下文（TeX `expand()`，`\edef`/`\xdef`/`\write`）：只展开可展开项——
    /// 宏、可展开原语（`\the`/`\expandafter`/`\noexpand`/`\number`/`\unexpanded`/
    /// `\detokenize`/`\eTeXversion`/`\eTeXrevision`）；条件由 process_one 拦截。
    /// 不可展开原语、未定义 cs、字符、组定界、宏参数一律原样保留（不执行、不建组）。
    fn process_expand_only(&mut self, tok: Token) -> Result<()> {
        match tok.kind() {
            TokenKind::ControlSeq => {
                let csid = tok.csid().expect("ControlSeq 必有 csid");
                match self.eqtb.slot(csid).clone() {
                    EqSlot::Macro(m) => {
                        if m.value.protected && self.suppress_expansion > 0 {
                            return self.sink.token(tok);
                        }
                        // TeX：\edef 展开上下文中 outer 宏 → forbidden
                        if m.value.outer {
                            return Err(Error::invalid_input(format!(
                                "forbidden control sequence \\{}（outer 宏禁止出现在 \\edef 展开上下文）",
                                self.intern.name(csid)
                            )));
                        }
                        self.call_macro(csid, m.value.clone())
                    }
                    EqSlot::Primitive(p) if p.is_expandable() => self.exec_primitive(p),
                    _ => self.sink.token(tok),
                }
            }
            _ => self.sink.token(tok),
        }
    }

    /// 展开宏调用：收集实参，压入字节码（M2）或宏体输入帧。
    fn call_macro(&mut self, csid: u32, def: Arc<MacroDef>) -> Result<()> {
        let args = if def.params.num_params > 0 {
            self.collect_args(csid, &def)?
        } else {
            Vec::new()
        };
        // M2 双轨：字节码优先（未编译则回退解释器轨道）
        if self.use_bytecode {
            if let Some(code) = &def.code {
                self.stack.push(InputFrame::Bytecode {
                    code: code.clone(),
                    pc: 0,
                    args,
                });
                return Ok(());
            }
        }
        self.stack.push(InputFrame::Macro {
            body: def.body.clone(),
            pos: 0,
            args,
        });
        Ok(())
    }

    // ---------- 输入获取 ----------

    /// 探测下一个 token 是否为数学移位（`$$` 检测）：
    /// 是 → 消费该 `$`（连续 `$$` 由 sink 一并处理，不放回）；
    /// 否 → 放回（不消费）。输入耗尽返回 false。
    fn next_is_math_shift(&mut self) -> Result<bool> {
        let Some((tok, ne)) = self.fetch()? else {
            return Ok(false);
        };
        let is = tok.catcode() == Some(Catcode::MathShift);
        if !is {
            self.stack.push(InputFrame::One { tok, noexpand: ne });
        }
        Ok(is)
    }

    /// 取下一个 token；返回 `(token, noexpand)`。输入耗尽或越过读取下限返回 None。
    fn fetch(&mut self) -> Result<Option<(Token, bool)>> {
        loop {
            // 不允许读取位于子展开边界（read_floor）以下的帧
            if self.stack.len() <= self.read_floor {
                return Ok(None);
            }
            let Some(frame) = self.stack.last_mut() else {
                return Ok(None);
            };
            match frame {
                InputFrame::Source { bytes, pos, state } => {
                    match scan_token(bytes, pos, &self.catcodes, &mut self.intern, state) {
                        Ok(Some(tok)) => return Ok(Some((tok, false))),
                        Ok(None) => {
                            self.stack.pop();
                            continue;
                        }
                        // M1-13 错误恢复（TRIP L351）：cat 15 非法字符 → TeX
                        // "Text line contains an invalid character." + 跳过该字符继续
                        // （tex.web get_next invalid_char；scan_token 已消费该字节）。
                        Err(Error::InvalidCharacter { .. }) => {
                            let mut msg =
                                "! Text line contains an invalid character.\n".to_string();
                            if let Some((n, line)) = self.error_context() {
                                msg.push_str(&format!("l.{n} {line}\n"));
                            }
                            msg.push_str("A funny symbol that I can't read has just been input.\n");
                            msg.push_str("Continue, and I'll forget that it ever happened.\n");
                            let _ = self.sink.write16(msg);
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                }
                InputFrame::Macro { body, pos, args } => {
                    if *pos >= body.len() {
                        self.stack.pop();
                        continue;
                    }
                    let tok = body[*pos];
                    *pos += 1;
                    // 宏参数替换：#n → 实参（压帧展开）
                    if let Some(n) = tok.param_number() {
                        let arg = args
                            .get((n.saturating_sub(1)) as usize)
                            .cloned()
                            .unwrap_or_default();
                        if arg.is_empty() {
                            continue;
                        }
                        let items: Vec<(Token, bool)> = arg.iter().map(|&t| (t, false)).collect();
                        self.stack.push(InputFrame::TokenList {
                            items: Arc::from(items),
                            pos: 0,
                        });
                        continue;
                    }
                    return Ok(Some((tok, false)));
                }
                InputFrame::Bytecode { code, pc, args } => {
                    if *pc >= code.len() {
                        self.stack.pop();
                        continue;
                    }
                    // M2-6：按 u64 原始字直接分发（零解包）。tag 0..=3 = token 原值
                    // 内联（RFC-1 布局），EMIT_ARG_TAG/END_TAG 见 bytecode.rs。
                    let word = code.words()[*pc];
                    match word >> crate::bytecode::TAG_SHIFT {
                        0..=3 => {
                            *pc += 1;
                            return Ok(Some((Token::from_raw(word), false)));
                        }
                        crate::bytecode::EMIT_ARG_TAG => {
                            *pc += 1;
                            let n = (word & 0xF) as u8;
                            let arg = args
                                .get(n.saturating_sub(1) as usize)
                                .cloned()
                                .unwrap_or_default();
                            if arg.is_empty() {
                                continue;
                            }
                            let items: Vec<(Token, bool)> =
                                arg.iter().map(|&t| (t, false)).collect();
                            self.stack.push(InputFrame::TokenList {
                                items: Arc::from(items),
                                pos: 0,
                            });
                            continue;
                        }
                        _ => {
                            // End
                            self.stack.pop();
                            continue;
                        }
                    }
                }
                InputFrame::TokenList { items, pos } => {
                    if *pos >= items.len() {
                        self.stack.pop();
                        continue;
                    }
                    let item = items[*pos];
                    *pos += 1;
                    return Ok(Some(item));
                }
                InputFrame::One { tok, noexpand } => {
                    // 先拷贝（结束字段借用）再弹帧（&mut stack）
                    let (t, ne) = (*tok, *noexpand);
                    self.stack.pop();
                    return Ok(Some((t, ne)));
                }
                InputFrame::OutputRoutine { items, pos } => {
                    if *pos >= items.len() {
                        // 例程帧耗尽：复位输出例程激活标志（可再次注入）。
                        // 剩余待处理页面是否丢弃由 maybe_inject_output 按进度判断
                        // （例程未取用 box255 时），不在帧弹出时处理——例程末 token
                        // 的参数扫描（如 \box255 数字）会 fetch 到帧外。
                        self.stack.pop();
                        self.output_active = false;
                        continue;
                    }
                    let item = items[*pos];
                    *pos += 1;
                    return Ok(Some(item));
                }
            }
        }
    }

    /// 把 token 放回输入流（等价于压入单元素 token 列表帧）。
    fn unread(&mut self, tok: Token) {
        self.stack.push(InputFrame::TokenList {
            items: Arc::from([(tok, false)]),
            pos: 0,
        });
    }
}

impl Expander {
    // ---------- 只读访问（测试/上层用） ----------

    pub fn intern(&self) -> &InternTable {
        &self.intern
    }

    pub fn eqtb(&self) -> &Eqtb {
        &self.eqtb
    }

    pub fn output(&self) -> &[Token] {
        self.sink.tokens()
    }

    /// 终端转录文本（`\message`/`\show`/`\write16` 累积；收集型 sink 实现）。
    pub fn transcript(&self) -> &str {
        self.sink.transcript()
    }

    /// ETRIP 冲刺：`\dump` 是否已执行（驱动据此保存 fmt 并二次运行测试体）。
    pub fn dumped(&self) -> bool {
        self.dumped
    }

    /// ETRIP 冲刺：最近一次 `\typeout{Checking ...}` 的段标题（错误定位）。
    pub fn current_section(&self) -> &str {
        &self.section_label
    }

    /// 替换输出 sink（排版器接入点，M3-2）。
    pub fn set_sink(&mut self, sink: Box<dyn TokenSink>) {
        self.sink = sink;
    }

    /// 取出输出 sink（排版器运行结束后取回 builder）。
    pub fn take_sink(&mut self) -> Box<dyn TokenSink> {
        std::mem::replace(&mut self.sink, Box::new(VecSink::default()))
    }

    /// 可变访问输出 sink（收尾冲页/输出例程交错期间仍挂在引擎上）。
    pub fn sink_mut(&mut self) -> &mut dyn TokenSink {
        &mut *self.sink
    }
}

impl Default for Expander {
    fn default() -> Self {
        Self::new()
    }
}

// ---------- 方法分片（include! 嵌入；原 impl Expander 方法按域拆分） ----------
// ---------- 方法分片：macros.rs ----------
include!("macros.rs");

// ---------- 方法分片：primitive.rs ----------
include!("primitive.rs");

// ---------- 方法分片：io.rs ----------
include!("io.rs");

// ---------- 方法分片：save.rs ----------
include!("save.rs");

// ---------- 方法分片：scan.rs ----------
include!("scan.rs");

// ---------- 方法分片：expr.rs ----------
include!("expr.rs");

// ---------- 方法分片：cond.rs ----------
include!("cond.rs");

// ---------- 方法分片：builtins.rs（原语注册表） ----------
include!("builtins.rs");

// ---------- 方法分片：free.rs（模块级自由函数） ----------
include!("free.rs");

include!("tests.rs");
