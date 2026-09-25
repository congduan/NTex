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

use std::collections::{BTreeMap, HashMap, VecDeque};
// 原子量/Mutex 仅线程看门狗用（M8-A WASM 骨架线：wasm32 下看门狗整段门控，
// 随之不导入，免 wasm 构建出现 unused import 警告）。
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex;
use std::sync::{Arc, OnceLock};

use crate::bytecode::{compile, Bytecode, Instruction};
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
    RegisterState, RegisterValue, Registers, MAX_DIMEN, MAX_INT, REGISTER_COUNT, SP_PER_PT,
};
use crate::sink::{AlignCellEnd, TokenSink, VecSink};
use crate::token::{meaning, Token, TokenKind};
use ntex_io::{LocalVfs, Vfs};

/// 字体加载记录：外部名 + at 规格 + scaled（pass2 恢复字体表用；.fmt 不含字体表）。
type FontLoad = (String, Option<i64>, Option<i64>);

/// 输入栈帧数上限（tex.web `stack_size`；TeX Live 2024 `texmf.cnf` 实际 10000）。
///
/// 宏递归展开（`\def\x{\x a}\x` 类）在 tex.web 里以 "TeX capacity exceeded,
/// sorry [input stack size=N]" 致命终止；本引擎由 [`Expander::call_macro`]
/// 入口检查同一上限（防御单步内的无界递归——主循环 10M 步看门狗够不到）。
///
/// ⚠ 2026-09-15 修正：原值 5000 与注释「TeX Live 默认 5000」均属事实错误。
/// 权威值 = `texmf-dist/web2c/texmf.cnf: stack_size = 10000  % simultaneous
/// input sources`，pdfTeX 实测报错原文即 `[input stack size=10000]`。取回
/// 同值使本引擎的致命报告与 GT 逐字一致（此前 GT 侧 10000、本侧 5000，
/// 报错文本不可直接对拍）。
///
/// ⚠ 注意本常量**不是**尾递归的判据：[`Expander::drain_depleted_frames`]
/// 修掉尾递归线性涨栈后，纯自尾调用（`\def\x{\x}\x`）不再涨栈——tex.web
/// 同款进无限循环（pdfTeX 实测挂死，不报容量错）；只有**调用者帧仍有余 token**
/// 的递归（`\def\x{\x a}\x`）才真正涨栈并命中本上限。
const MAX_INPUT_STACK: usize = 10000;

/// 宏调用轨迹环形缓冲上限（诊断用；`NTEX_CALL_TRACE=N` 开启，N = 转储段数）。
///
/// 输入栈只保留**未弹出的**帧，而递归循环的入口帧往往早已弹出——想回答
/// 「谁把自展开的宏打进流里」只能靠调用轨迹（2026-09-13 expl3 载入 l.27200
/// 现场：栈上 4997 帧全是 `\q_stop`，看不出上游是谁）。
///
/// ⚠ 容量必须**大于输入栈上限**：循环自身就把栈压到 5000 帧，缓冲若比它小，
/// 尾巴全被循环吞掉，入口帧（答案所在）已被挤出（4096 首版即栽在这里）。
/// 取 64K 段 ≈ 256 KB，仅诊断开启时分配。
const MACRO_TRACE_CAP: usize = 1 << 16;

/// 字节码 dispatch 护栏的最近控制序列容量（第十八刀（三）现场取证）。
const BC_GUARD_TRACE_CAP: usize = 32;

/// `Params.misc` 下标（与 `free::int_param_index` 对齐）：`\deadcycles`、
/// `\maxdeadcycles`、`\outputpenalty`（输出例程刀 1 的 fire_up 点火侧语义）。
const DEAD_CYCLES_IDX: usize = 25;
const MAX_DEAD_CYCLES_IDX: usize = 43;
const OUTPUT_PENALTY_IDX: usize = 62;
/// tex.web `inf_penalty`：非惩罚节点断点（胶水/kern 自然断页）时
/// `\outputpenalty` 的值（`@<Set the value of |output_penalty|@>`）。
const INF_PENALTY: i64 = 10_000;

/// 输入帧：token 来源栈（LIFO，栈顶为当前帧）。
///
/// `Clone`（M5 阶段二）：段级回滚还原 [`ControlState::stack`]，需要克隆悬挂
/// 的输入帧（内容均为 `Arc`/`Copy`，克隆廉价）。
pub(crate) type ArgArray = Arc<[(Token, bool)]>;

#[derive(Debug, Clone)]
pub(crate) enum InputFrame {
    /// 源码帧：字节流 + 扫描位置 + 行状态（空行 → `\par` 判定）。
    Source {
        bytes: Arc<[u8]>,
        pos: usize,
        state: ScanState,
        /// 行起始偏移表（预建，`current_line_no` 二分用；替代逐帧线性数 `\n`）。
        line_starts: Arc<[u32]>,
        /// `\endinput` force_eof 截断点（tex.web `force_eof`：**当前行读完**
        /// 才关文件）。`None`=正常；`Some(n)`=本帧读到字节偏移 `n`（截断行的
        /// 行尾 `\n` 之后）即视为文件结束——pop 帧时注入 `\everyeof`。
        /// 主文件（栈中最后一个 Source 帧）截断时置 `ended`（tex.web 同款）。
        eof_mark: Option<usize>,
    },
    /// 宏展开帧：宏体 + 实参。
    Macro {
        body: TokenArray,
        pos: usize,
        args: Vec<ArgArray>,
    },
    /// 字节码帧（M2）：预编译指令 + 实参（解释器轨道的替代）。
    Bytecode {
        code: Arc<Bytecode>,
        pc: usize,
        args: Vec<ArgArray>,
    },
    /// token 列表帧：`(token, noexpand 标记)`。
    TokenList {
        items: Arc<[(Token, bool)]>,
        pos: usize,
    },
    /// 实参帧（P1 热路径消分配）：宏实参 token 列表。
    ///
    /// `#n`/`EmitArg` 展开直接复用收集期的 `Arc<[(Token,bool)]>`（引用计数 +1）。
    /// 这里必须保留 `\noexpand` 的一次性冻结位：LaTeX `\protect` 会在
    /// `\edef`/`\write` 的同一展开区域内经宏实参转交可展开 token。
    MacroArg { items: ArgArray, pos: usize },
    /// 单 token 回推槽（B1：`unread`/`$$` 探测/`\noexpand` 回推用，免 Arc 包装）。
    One { tok: Token, noexpand: bool },
    /// 输出例程帧（M3-5-3）：同 TokenList，但耗尽时复位输出例程激活标志。
    OutputRoutine {
        items: Arc<[(Token, bool)]>,
        pos: usize,
    },
    /// M4-5 对齐 u 模板帧（tex.web u_template token list）：耗尽时
    /// align_state←0（单元 raw 扫描开始；end_token_list 的 u_template 分支）。
    AlignU { items: TokenArray, pos: usize },
    /// M4-5 对齐 v 模板帧（tex.web v_template）：耗尽即 \endtemplate（endv）
    /// → fin_col。空帧 = `\omit` 单元的 omit_template。
    AlignV { items: TokenArray, pos: usize },
}

/// 条件分支状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CondState {
    /// 处理当前分支。
    Processing,
    /// 跳过直到本帧的 `\else`/`\fi`。
    Skipping,
    /// 参数扫描中（tex.web `if_limit=if_code`：帧已压、`\if` 求值未完成——
    /// 此时遇 `\or`/`\else`/`\fi` 走 `insert_relax` 而非弹帧/报 Extra）。
    Evaluating,
}

/// 条件栈帧（M1-9）。
#[derive(Debug, Clone)]
pub(crate) struct CondFrame {
    /// 是否为 `\ifcase` 帧。
    is_case: bool,
    state: CondState,
    /// 跳过时若为 true：`\else` 会恢复 Processing（false 分支的 then 被跳过）；
    /// 为 false：`\else` 只是被匹配（跳过 else 分支/惰性嵌套帧）。
    owns_skip: bool,
    /// `\ifcase` 跳过计数：还需跳过的 `\or` 数。
    ors_left: Option<usize>,
    else_seen: bool,
    /// `\ifcase` 已选中分支（跳够 `\or` 进入 Processing 后，多余 `\or`/`\else`
    /// 应保持跳过——`\else` 是选中分支后的内容，不是兜底落点）。
    case_selected: bool,
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
    IfInCsname,
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
            Primitive::IfInCsname => Self::IfInCsname,
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
///
/// `Clone`（M5 阶段二）：段级回滚把 [`ControlState::save_stack`] 整体还原到
/// 段前状态，需要克隆悬挂的保存项。
#[derive(Debug, Clone)]
pub(crate) enum SavedValue {
    Eqtb {
        csid: u32,
        prev: EqSlot,
        /// 压栈时槽的层级（tex.web eq_level，随槽值一起恢复）：
        /// 0 = 全局（tex.web level_one），1 = 组内局部。
        prev_level: u8,
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
    /// M9 中文刀 4：>255 码位的 catcode 覆盖（`\utfinputmode=1` 下
    /// `\catcode`，=13；prev None = 此前未覆盖，回滚为默认 letter）。
    UnicodeCatcode {
        cp: u32,
        prev: Option<Catcode>,
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
    /// `\fontdimen`：字体参数覆盖（仅旧实现路径构造；tex.web 语义恒全局，
    /// 活跃路径不进 save stack——见 primitive_font.rs exec_fontdimen 臂注）。
    FontDimen {
        font: u32,
        num: u32,
        prev: Option<i64>,
    },
    /// 当前字体（tex.web `cur_font_loc`，它**是 eqtb 字**：`define(cur_font_loc,…)`
    /// 随组保存/恢复）。
    ///
    /// 此前 `Expander::cur_font` 是裸字段、选择即永久生效——组内换字体
    /// 出组泄漏，而 em/ex 内部单位（`\kern-.36em`、`\lower.5ex`、
    /// `\fontdimen` 语境）正是读它算的：`\LaTeX` 徽标的 `\fontsize\sf@size`
    /// 内层组把 cur_font 设成 cmr7 后不收口，第二个起的徽标全部改用 cmr7 的
    /// quad（7.97224pt ≠ 10pt）→ `\kern-.36em` 少缩 0.73pt、`\TeX` 的
    /// `-.1667em`/`.5ex` 同步偏，徽标 A 与 L/T 的相对位置错（2026-09-25 现场）。
    /// 排版侧 `NodeBuilder` 早有 `font_stack`（sink.rs group_begin/end），
    /// 缺的就是 expander 这一半。
    CurFont {
        prev: u32,
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
    /// `\uccode`：大写码表项（M9 中文刀 6 修：此前 exec_uccode 复用 LcCode
    /// 变体入栈、恢复侧写回 `self.lccodes`——字段错位双重 bug：uccode 组内
    /// 赋值出组泄漏 + lccodes 同下标被无辜改写。amsmath `\uppercase{\gdef
    /// \macro@#1#2#3#4\macro@{…}}` 靠 uccode 组作用域隔离参数数字转换，
    /// 泄漏导致 `#4`→`#r` Illegal parameter number，载入即炸）。
    UcCode {
        byte: u8,
        prev: i64,
    },
    /// `\every*` 族（everypar/everyhbox/everyvbox/everycr/everydisplay/errhelp，
    /// kind 0..=5）：组作用域保存（tex.web local_base+8 起的 eqtb toks 槽）。
    ///
    /// 此前是 Expander 裸字段、赋值即永久生效——LaTeX 内核 `\@lign`（"restore
    /// inside \displ@y"）在每个对齐单元里 `\everycr{}`，靠组结束恢复 `\displ@y`
    /// 设的值；裸字段让该清空泄漏到整个对齐 → 每行 `\noalign{\global\column@\z@}`
    /// 不再执行 → `\add@amps` 少吐 `&` → `\math@cr@@@align` 的 `\omit` 落格内。
    EveryToks {
        kind: u8,
        prev: Vec<Token>,
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
    /// `\noexpand` 冻结的**可展开** cs 的临时含义（tex.web l.7506-7516：
    /// `frozen_dont_expand` 标记被读取时 `if cur_cmd>max_command then
    /// (cur_cmd,cur_chr):=(relax,no_expand_flag)`，no_expand_flag=257——
    /// 真实 `\relax` 的 cur_chr 是 eqtb 指针，永不为 257，故这是一个任何
    /// 实际 cs 都取不到的独立含义键）。expl3 `\__exp_eval_register:N` 的
    /// "宏还是寄存器"判别 `\exp_after:wN \if_meaning:w \exp_not:N #1 #1`
    /// 正依赖此语义：宏（可展开）→ 本键，与原含义不等 → `\else` 臂
    /// （展开一次取值）；寄存器（不可展开）→ 原含义 → `\the` 臂。
    NoExpandRelax,
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
///
/// `Clone`（M5 阶段二）：段级回滚还原读流表；`data` 在 `\openin` 后只读
/// （`\read` 只推进 `pos`），克隆即完整状态。
#[derive(Debug, Clone)]
pub(crate) struct ReadStream {
    /// 目标路径（错误信息用）。
    _path: String,
    /// 文件内容。
    data: Vec<u8>,
    /// 当前字节位置。
    pos: usize,
}

/// 写流（RFC-3）：`\openout` 登记路径，`\write` 入队，flush 边界落盘。
///
/// `Clone`（M5 阶段二）：段级回滚还原写流表（待写内容 token 列表 `Arc` 共享）。
#[derive(Debug, Clone)]
pub(crate) struct WriteStream {
    /// 目标路径；`None` = 流未打开（tex.web `write_open[j]=false`）。
    path: Option<String>,
    /// 已在本轮截断（创建）过目标文件。RFC-3 §4.4：`\openout` 语义为覆盖——
    /// 首次实际写出用 `Vfs::write` 清空目标（丢弃上次作业残留），其后 `append`。
    /// 注意：该标志随段级回滚还原为 false，重放段首条写出会再截断一次
    /// （副作用段本就被 M5 的 side_effects_match 闸整体重放，可接受）。
    created: bool,
    /// 延迟待写 token 列表（`\write` 入队；flush 时展开落盘）。
    pending: Vec<TokenArray>,
}

/// `\every*` token 列表族（list 机制刀起入 `.fmt` 快照）。
///
/// LaTeX 段落钩子机器在格式装载期执行
/// `\tex_everypar:D{\g__para_standard_everypar_tl}`（latex.ltx l.9137）——
/// 该赋值必须跨快照存活，否则恢复后 `\everypar` 恒空、list 机制
/// （`\@newlist` 清位）失效。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EveryToks {
    /// `\everypar`（段首触发；list 机制根因所在）。
    pub everypar: Vec<Token>,
    /// `\everymath`。
    pub everymath: Vec<Token>,
    /// `\everyhbox`。
    pub everyhbox: Vec<Token>,
    /// `\everyvbox`。
    pub everyvbox: Vec<Token>,
    /// `\everycr`。
    pub everycr: Vec<Token>,
    /// `\everydisplay`。
    pub everydisplay: Vec<Token>,
    /// `\errhelp`。
    pub errhelp: Vec<Token>,
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
    /// `\every*` token 列表族（`\everypar` 段首触发链）。
    pub every_toks: EveryToks,
    /// FontId → 外部字体名（`\fontname` 查询用；加载后需重新 `\font`）。
    pub font_names: Vec<Option<String>>,
    /// FontId → (外部名, at, scaled)（pass2 恢复字体表用；.fmt 不含字体表，
    /// 恢复时按此重新加载 TFM——pass1 定义的 `\font\trip` 在 pass2 不重跑）。
    pub font_loads: Vec<Option<FontLoad>>,
    /// FontId → cs 名（showbox 字体标识显示 `.\trip 1`；pass2 保留）。
    pub font_cs_names: Vec<Option<String>>,
    /// pass1 结束时的当前字体（fmt 恢复后字符用正确字体——etrip.tex L62
    /// `\trip` 选择后 dump，pass2 若不恢复则全 nullfont + Missing 警告）。
    pub current_font: u32,
    /// `\delcode` 表（v21：`.fmt` 此前不携带——fontmath.ltx 在 fmt 生成期做的
    /// `\delcode`(/`[` 等赋值全丢，恢复后回退引擎 INITEX 默认 → `\left(`
    /// 报 "Missing delimiter" 并退化普通尺寸）。
    pub delcodes: std::collections::HashMap<u32, u32>,
    /// `\mathcode` 表（v21：fontmath.ltx 的 `\DeclareMathSymbol` 字符赋值
    ///（`<`→cmsy class3、`,`→class6 punct 等）随 fmt 丢失，所有非字母字符
    /// 退回 0x7000+码 初表——字形落 cmr 同槽（`<`→`¡`）且 class 全 7/0
    /// → punct/rel/bin 间距整族失效）。
    pub mathcodes: std::collections::HashMap<u32, u32>,
    /// `\lccode` 表（v21）。
    pub lccodes: [i64; 256],
    /// `\uccode` 表（v21）。
    pub uccodes: [i64; 256],
}

/// 线程看门狗共享状态（挂死诊断）。
///
/// `run()` 主循环每步更新心跳与状态快照；独立线程每 2s 检查心跳，
/// 停更超过 10s（单步内部死循环 / layout 侧死循环导致 process_one 不返回）
/// 即打印最后状态并退出（one-shot）。不参与 `.fmt` 序列化。
///
/// WASM（M8-A）：整段随看门狗门控——单线程环境看门狗线程无意义，
/// 且 `std::thread::spawn`/`sleep`/`SystemTime::now` 在 wasm32-unknown-unknown
/// 上不可用（panic），见 [`Expander::run`] 注释。
#[cfg(not(target_arch = "wasm32"))]
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
    /// TEMP DEBUG（第十七刀取证）：卡死前最近事件环（NTEX_BREAK17 开启时记录）。
    trace: Mutex<Vec<String>>,
}

/// 当前 UNIX 毫秒（线程看门狗心跳用）。
///
/// WASM（M8-A）：随看门狗整段门控——wasm32-unknown-unknown 的 std 无 OS 时钟
/// （`SystemTime::now` 直接 panic），单线程环境本就看门狗无意义（见 `run()`）。
#[cfg(not(target_arch = "wasm32"))]
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 诊断环境开关（TRIP/ETRIP 冲刺期的挂死定位设施），进程级缓存。
///
/// `NTEX_COND_TRACE`/`NTEX_TRACE_EXEC`/`NTEX_IFNUM_TRACE`/`NTEX_SANITY_CHECK`
/// 位于每条件/每原语执行的热路径上——原实现每次 `std::env::var`（getenv +
/// String 分配，实测占展开吞吐 1~3%）。进程内环境不变，缓存首次读取结果，
/// 语义不变（含"设为空串也算开启"）。
pub(crate) fn diag_enabled(key: &'static str) -> bool {
    static CACHE: OnceLock<HashMap<&'static str, bool>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            DIAG_KEYS
                .iter()
                .map(|&k| (k, std::env::var(k).is_ok()))
                .collect()
        })
        .get(key)
        .copied()
        .unwrap_or(false)
}

/// 展开步数上限（死循环防线；`NTEX_MAX_STEPS` 可覆盖，非法/空值回落默认）。
///
/// expl3 真载入（codepoint 三表 `\read` 装载 + finalize + CaseFolding/
/// SpecialCasing）2026-09-15 实测超 10⁷ 步——原 10⁷ 常数会把合法慢载入
/// 误判成死循环（探针死于 `\__codepoint_finalize_blocks` 中段）。
pub(crate) fn max_steps() -> u64 {
    // 测试内收紧用线程局部覆盖——env 是进程全局，cargo test 并行线程会互相污染。
    #[cfg(test)]
    if let Some(v) = MAX_STEPS_OVERRIDE.with(std::cell::Cell::get) {
        return v;
    }
    static CACHE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| {
        std::env::var("NTEX_MAX_STEPS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|&v| v > 0)
            .unwrap_or(64_000_000)
    })
}

/// 字节码 dispatch 护栏。默认关闭，避免给正常展开热路径增加计数；显式设置
/// `NTEX_BC_GUARD=N` 后，单次 [`Expander::fetch`] 内连续 N 次分发仍未交还 token
/// 即中止并转储现场。这专门覆盖主循环步数无法观察的帧内自旋。
fn bytecode_guard_limit() -> u64 {
    static CACHE: OnceLock<u64> = OnceLock::new();
    *CACHE.get_or_init(|| {
        std::env::var("NTEX_BC_GUARD")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|&v| v > 0)
            .unwrap_or(0)
    })
}

/// handler 进入轨迹开关。仅定位帧内自旋时开启：记录最近 32 个进入 dispatcher
/// 的控制序列及其读取下限，待 bytecode 护栏触发时统一转储，避免正常运行刷屏。
fn handler_trace_enabled() -> bool {
    static CACHE: OnceLock<bool> = OnceLock::new();
    *CACHE.get_or_init(|| std::env::var_os("NTEX_HANDLER_TRACE").is_some())
}

/// 把字节偏移吸附到不超过它的最近字符边界（已在边界上则原样返回）。
///
/// 错误上下文输出（`write_error_impl`）要做**字符**口径的裁剪，而输入位置是
/// **字节**偏移，两者必须显式换算。本函数是该换算的防御性底座：词法器理论上
/// 只按字符推进（偏移恒为边界），但错误路径必须对任意输入成立
/// （AGENTS.md 引擎契约「任意畸形输入不 panic」），故不假设该前提。
fn snap_char_boundary(s: &str, mut at: usize) -> usize {
    if at >= s.len() {
        return s.len();
    }
    while at > 0 && !s.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[cfg(test)]
thread_local! {
    static MAX_STEPS_OVERRIDE: std::cell::Cell<Option<u64>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_max_steps_override(v: Option<u64>) {
    MAX_STEPS_OVERRIDE.with(|c| c.set(v));
}

/// [`diag_enabled`] 认识的全部诊断开关。
const DIAG_KEYS: &[&str] = &[
    "NTEX_COND_TRACE",
    "NTEX_IFNUM_TRACE",
    "NTEX_IFX_TRACE",
    "NTEX_LETDEF_TRACE",
    "NTEX_TRACE_EXEC",
    "NTEX_TRACE_STACK",
    "NTEX_SANITY_CHECK",
    "NTEX_ALIGN_TRACE",
    "NTEX_BIGLIST_TRACE",
];

/// 结构化 trace 通道（JSONL）——**挂死/膨胀类定位的核心设施**。
///
/// ## 为什么不是 `eprintln`
///
/// 文本 `eprintln` 的痛点是**不可机读、不可过滤、不可跨运行比对**；定位
/// 「输入栈为什么膨胀」这类问题时，watchdog 只能给「最后一帧 + 栈快照」，
/// 无法回答「是谁 push 了它」。JSONL 每行一个事件，可事后按字段查询。
///
/// ## 用法
///
/// ```bash
/// NTEX_TRACE_JSONL=/tmp/t.jsonl ntex-dvi doc.tex
/// scripts/trace-view.py /tmp/t.jsonl --grep cs_generate   # 按 token 过滤
/// scripts/trace-view.py /tmp/t.jsonl --around 50000       # 第 5 万步前后
/// scripts/trace-view.py /tmp/t.jsonl --stack-at 49999     # 那一刻的完整栈
/// ```
///
/// ## 与 `diag_enabled` 的关系
///
/// 文本开关（`NTEX_COND_TRACE` 等）保留；JSONL 是**增量**设施，两者可同时开。
/// 事件 schema：`{"step":N,"kind":"push|pop|fetch|expand|cond|note",
/// "tok":"\\foo","csid":123,"depth":D,"frame":"Macro(...)","extra":{...}}`
///
/// 实现约束：**零成本开关**——未设 `NTEX_TRACE_JSONL` 时 `trace_event` 只做
/// 一次缓存查表即返回；不得在热路径引入 String 分配（延迟到确实开启时）。
#[cfg(not(target_arch = "wasm32"))]
pub mod trace {
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};

    /// 事件种类（写进 JSONL 的 `kind` 字段）。
    pub const KIND_PUSH: &str = "push";
    pub const KIND_POP: &str = "pop";
    pub const KIND_FETCH: &str = "fetch";
    pub const KIND_EXPAND: &str = "expand";
    pub const KIND_COND: &str = "cond";
    pub const KIND_NOTE: &str = "note";

    struct Sink {
        /// 缓冲写出器（唯一写出通道）。进程被 SIGKILL 时尾部会丢——可接受：
        /// 定位挂死时 watchdog 已打快照；正常退出由 Drop 冲刷。
        buf: std::io::BufWriter<std::fs::File>,
    }

    static SINK: OnceLock<Option<Mutex<Sink>>> = OnceLock::new();

    fn sink() -> Option<&'static Mutex<Sink>> {
        SINK.get_or_init(|| {
            let path = std::env::var("NTEX_TRACE_JSONL").ok()?;
            let file = std::fs::File::create(&path).ok()?;
            Some(Mutex::new(Sink {
                buf: std::io::BufWriter::new(file),
            }))
        })
        .as_ref()
    }

    /// 是否开启（供调用点做昂贵字段构造前的短路判断）。
    #[inline]
    pub fn enabled() -> bool {
        std::env::var_os("NTEX_TRACE_JSONL").is_some()
    }

    /// 写一条事件。字段全部可选，`None` 的键不出现（保持 JSONL 精简）。
    pub fn event(
        step: u64,
        kind: &str,
        tok: Option<&str>,
        csid: Option<u32>,
        depth: usize,
        frame: Option<&str>,
        extra: Option<&str>,
    ) {
        let Some(m) = sink() else { return };
        let Ok(mut s) = m.lock() else { return };
        // 手写 JSON（避免引入 serde 依赖；字段值需转义引号/反斜杠）
        let esc = |v: &str| v.replace('\\', "\\\\").replace('"', "\\\"");
        let mut line = format!("{{\"step\":{step},\"kind\":\"{}\"", esc(kind));
        if let Some(t) = tok {
            line.push_str(&format!(",\"tok\":\"{}\"", esc(t)));
        }
        if let Some(c) = csid {
            line.push_str(&format!(",\"csid\":{c}"));
        }
        line.push_str(&format!(",\"depth\":{depth}"));
        if let Some(f) = frame {
            line.push_str(&format!(",\"frame\":\"{}\"", esc(f)));
        }
        if let Some(e) = extra {
            line.push_str(&format!(",\"extra\":\"{}\"", esc(e)));
        }
        line.push_str("}\n");
        let _ = s.buf.write_all(line.as_bytes());
        // 每条即冲（static 无 Drop，进程退出不会自动 flush）。trace 本就是
        // 诊断态，性能非目标；保证 SIGKILL 也能拿到已写事件。
        let _ = s.buf.flush();
    }
}

/// WASM 侧空实现（单线程无诊断通道；保持调用点零改动）。
#[cfg(target_arch = "wasm32")]
pub mod trace {
    pub const KIND_PUSH: &str = "push";
    pub const KIND_POP: &str = "pop";
    pub const KIND_FETCH: &str = "fetch";
    pub const KIND_EXPAND: &str = "expand";
    pub const KIND_COND: &str = "cond";
    pub const KIND_NOTE: &str = "note";
    #[inline]
    pub fn enabled() -> bool {
        false
    }
    #[inline]
    pub fn event(
        _step: u64,
        _kind: &str,
        _tok: Option<&str>,
        _csid: Option<u32>,
        _depth: usize,
        _frame: Option<&str>,
        _extra: Option<&str>,
    ) {
    }
}

/// tex.web `scanner_status`（L6597-6604）——扫描上下文状态机。
///
/// **唯一**决定 outer 宏是否报错的条件：`check_outer_validity`（L7152）判据是
/// `scanner_status <> normal`。tex.web 的赋值点：
///
/// | 状态 | 设置处 | 含义 |
/// |---|---|---|
/// | `normal` | L7096 初值 / L7714 取单 token 前临时 | 常规 |
/// | `skipping` | L9663 `pass_text` | 跳过条件文本（`\if` 假分支）|
/// | `defining` | L9324 `scan_toks(macro_def=true)` / L9454 `scan_def` | 扫宏定义 |
/// | `matching` | L8008 `macro_call` 扫实参 | 扫宏实参 |
/// | `aligning` | L15368 `init_align` | 扫对齐 preamble |
/// | `absorbing` | L9325 `scan_toks(macro_def=false)` | 扫平衡文本（`\edef` 等）|
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ScannerStatus {
    /// 常规上下文（外层）。
    #[default]
    Normal,
    /// 跳过条件文本（`pass_text`）——最低级：`check_outer_validity` 走
    /// 「Incomplete \if; all text was ignored」分支（L7157-7166）。
    Skipping,
    /// 扫宏定义（`scan_toks` macro_def / `scan_def`）。
    Defining,
    /// 扫宏实参（`macro_call`）。
    Matching,
    /// 扫对齐 preamble（`init_align`）。
    Aligning,
    /// 扫平衡文本（`scan_toks` 非 macro_def：`\edef`/`\write`/`\message` 等）。
    Absorbing,
}

/// 展开引擎。
#[derive(Debug)]
pub struct Expander {
    intern: InternTable,
    eqtb: Eqtb,
    /// tex.web `scanner_status`（L6572-6604）：当前扫描上下文。**唯一**决定
    /// outer 宏是否报 `Forbidden control sequence` 的条件——
    /// `check_outer_validity`（L7152）判据是 `scanner_status <> normal`。
    ///
    /// ⚠ 此前的实现用「输入栈里是否有 Macro 帧」当代理判据（mod.rs 展开臂的
    /// `stack.iter().any(InputFrame::Macro)`），**是错的**：栈含 Macro 帧
    /// ≠ `scanner_status<>normal`。实测代价——plain 的 `^^L` 是 active char
    /// 且 `\outer\def^^L{\par}`（active char 的 `cur_cmd := eq_type` =
    /// `outer_call`，故 outer 检查会跑），但宏实参扫描用 `get_token`
    /// （L7714 `save_scanner_status; scanner_status:=normal; get_token; 恢复`）
    /// **不设 scanner_status**，pdfTeX 因此不报错；NTex 误报 →
    /// expl3-code.tex L9320 `\char_set_catcode_active:N \^^L` 处级联，
    /// `Extra \or` 224 条（占 expl3 载入现场错误 43.8%）。
    scanner_status: ScannerStatus,
    /// tex.web `warning_index`：报错时指向「正在扫描谁的」cs（宏名）。
    warning_index: Option<u32>,
    /// 线程看门狗共享状态（挂死诊断；不参与 .fmt 序列化，run 时创建）。
    /// WASM（M8-A）：随看门狗整段门控（单线程环境无诊断线程）。
    #[cfg(not(target_arch = "wasm32"))]
    watchdog: Option<Arc<WatchdogShared>>,
    catcodes: CatcodeTable,
    /// `\sfcode` 表（M3-4 词间距 spacefactor；TeX 默认全 1000，plain 对
    /// .,?!=3000、:=2000、;=1500、,=1250，由排版器按 plain 默认初始化）。
    sfcodes: [u32; 256],
    /// tex.web `cur_font`（引擎内部当前字体，FMF382）：字体选择 cs 执行时更新，
    /// `\font`（Primitive::Font）作字体标识符时查询（`\fontdimen6\font`、
    /// `\the\font` 等 scan_font_ident 路径）。此前查 `sink.current_font()`——
    /// 查询 sink 的实时态不可靠（LaTeX NFSS 语境返回 0 → `\section` 前置
    /// skip `-3.5ex` 读 ex 得 0pt → Missing number ×2 + `\Large` 14.4pt 残留）。
    cur_font: u32,
    stack: Vec<InputFrame>,
    /// 宏调用环形轨迹（诊断开关；默认 `None` = 零开销）。
    /// 见 [`MACRO_TRACE_CAP`]：栈帧只回答「谁还在栈上」，轨迹才回答「谁调用了谁」。
    macro_trace: Option<VecDeque<u32>>,
    /// 输出 sink（M3-2）：token/组/原语事件流；默认 [`VecSink`] 收集 token。
    sink: Box<dyn TokenSink>,
    /// 展开区域（\edef/\write/\message）期间 sink 被临时替换为 VecSink，
    /// 内部量查询（\currentgrouptype/\lastnodetype）转发到原 sink。
    query_sink: Option<Box<dyn TokenSink>>,
    /// \shipout 触发行号（tex.web：output 例程在 shipout 行注入——例程组
    /// 的 entering 行号 = shipout 行而非注入时刻的输入位置）。
    output_trigger_line: usize,
    /// 读取下限：`fetch` 只允许从下标 >= 该值的帧读取；
    /// 用于划分子展开（`\edef`/`\expandafter` 区域）的边界。
    read_floor: usize,
    /// 条件栈（M1-9）。
    cond_stack: Vec<CondFrame>,
    /// e-TeX `\ifincsname` 旗标（e-tex.web `name_in_progress`）：当前正处于
    /// `\csname`/`\ifcsname` 名字扫描。由 scan_csname 进出置位/还原（保存
    /// 旧值而非置假——嵌套 csname 时内层还原不得清掉外层）。
    name_in_progress: bool,
    /// NTEX_SANITY_CHECK 设施：错误恢复前状态快照 (组级, 条件栈深)。
    /// report_error 时记录第一个错误；主循环下一次迭代校验恢复后的状态
    /// 与错误前是否大幅偏离（偏离 = 错误恢复本身写坏了状态，内部 bug 信号）。
    err_snapshot: Option<(u32, usize)>,
    /// 组层级（M1-11）。
    group_level: u32,
    /// 本次 `end_group` 是否由 `\endgroup` 原语进入（vs `}` 字符）：
    /// 组外闭合的报错文本不同（`Extra \endgroup.` vs `Too many }'s.`）。
    cur_group_close_via_primitive: bool,
    /// `\left`/`\middle` 打开的 math left group（16）嵌套深度：expander 侧配对
    /// 保护——`\left` +1、`\middle` 关一开一（净 0）、`\right` -1；为 0 时
    /// `\right`/`\middle` 不触发 end_group（避免关掉外层非数学组；TeX 语义
    /// `\right` 前必须有 `\left`，缺配对由 layout 侧报错恢复）。
    math_left_depth: usize,
    /// M4-5 对齐帧栈（`\halign`/`\valign`；嵌套对齐 = 栈式多帧）。
    /// preamble 解析、u/v 模板注入、align_state 平衡计数（见 align.rs）。
    align_frames: Vec<AlignFrame>,
    /// M4-5 最外层对齐的 align_state（tex.web 全局 align_state；嵌套时各帧
    /// 自带，此字段只在帧栈空/栈底时有效——tex.web pop_alignment 恢复点）。
    align_state: i64,
    /// 组开始时的条件栈深度（组结束必须回到该深度）。
    group_cond_depth: Vec<usize>,
    /// 赋值保存栈：组结束时按层回滚（朴素快照回滚）。
    save_stack: Vec<(u32, SavedValue)>,
    /// `\global` 前缀：作用于下一个赋值。
    global_pending: bool,
    /// `\aftergroup`：`(组层级, token)`。
    aftergroup: Vec<(u32, Token)>,
    /// 统一作用域栈：VM 组（`begin_group`/静默组）与**数学组**（`$` 的
    /// tex.web `math_shift_group`——`init_math` 同样 `new_save_level`）都
    /// 入栈；`\aftergroup` 挂**最内层作用域**。此前数学层只记排版侧
    /// enter/close 事件、不算 save group，`\everymath` 体里的
    /// `\aftergroup\@ignorefalse`（LaTeX `\frozen@everymath`）错挂到外包
    /// 对齐/盒子组，在 `\cr` 行界把 `\global…` 泄给 align_peek，
    /// `\halign{#\cr $x$ \cr}` 之后的 `}` 被当成新行首列凭空开列。
    /// true = 数学作用域，false = VM 组。
    scope_stack: Vec<bool>,
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
    /// FontId → (外部名, at, scaled)：.fmt 序列化用，pass2 恢复字体表。
    font_loads: Vec<Option<FontLoad>>,
    /// FontId → cs 名（showbox 字体标识显示；fmt 序列化 + font_defined 事件）。
    font_cs_names: Vec<Option<String>>,
    /// `\output` 例程 token 列表（M3-5-3）；None = 未定义（断页直通 shipout）。
    output_toks: Option<TokenArray>,
    /// 输出例程正在执行（防嵌套：例程内再次断页报错）。
    output_active: bool,
    /// 是否已执行显式 `\end`（finish 收尾对未闭合组/math 按 TeX 语义降级为警告）。
    ended: bool,
    /// 子展开（expand_region）步数累计——护栏计数（主循环 steps 是局部变量，
    /// 子展开内的 process_one 递归不受其约束，latex.ltx l.16900 \@preamble
    /// \\edef 曾无限循环 900s 无护栏）。
    region_steps: u64,
    /// 最近一次处理的 token（watchdog/单步超时诊断用；不参与 .fmt 序列化）。
    last_tok: Option<String>,
    /// 第十八刀（三）字节码护栏的最近控制序列环；仅开启护栏时写入。
    bc_guard_trace: VecDeque<u32>,
    /// 当前一个 process_one dispatch 是否仍在 handler 内；fetch 据此跨调用累计。
    bc_guard_active: bool,
    /// 当前 dispatch 内经历的 fetch 循环次数（含 handler 的参数扫描）。
    bc_guard_dispatches: u64,
    /// 第十八刀（四）handler 进入环：`NTEX_HANDLER_TRACE` 开启时记录，
    /// `read_floor` 是判定子展开是否越界读取的必要现场。
    handler_trace: Option<VecDeque<(u32, usize)>>,
    /// 主循环步数镜像（结构化 trace 的 `step` 字段用；不参与 .fmt 序列化）。
    /// 由 `run()` 每步同步——未开 trace 时仅一次整数赋值，无分配。
    steps: u64,
    /// `\tracingcommands`：上次打印的模式（tex.web shown_mode——模式变化才打前缀）。
    shown_trace_mode: Option<String>,
    /// 子展开（`\write` 内容、marks 查询等 expand_region）追踪抑制计数——
    /// TeX 只在 main_control 主循环追踪（show_cur_cmd_chr），扫描器内部不追踪。
    trace_suppress: u32,
    /// `\moveleft/\moveright` 的 box 参数扫描中（tex.web scan_box：参数 token
    /// 不追踪——`\moveleft20pt\copy200` 的 `{\copy}`、`\moveright20pt\hbox{` 的
    /// `{\hbox}`/`{` 均不输出；组内容执行恢复追踪）。
    pending_box_arg: bool,
    /// 进入 box 参数扫描时的模式码（模式变化 = 盒子组内容开始 → 立即恢复追踪）。
    pending_box_arg_mode: i64,
    /// 非组盒子原语（`\copy`/`\box` 等）处理完恢复追踪的延迟标记（本次仍抑制）。
    trace_suppress_defer: bool,
    /// 报错上下文锚点：值扫描起始的 Source pos（clamp_dimen 报错时 pos 已推进
    /// 到下一行——用锚点回溯到值所在行，tex.web l.N 上下文语义）。
    error_anchor: Option<usize>,
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
    /// 当前 macro_call 的实参扫描是否已由「Paragraph ended」类恢复中止。
    /// tex.web 此时不再继续扫描后续参数、更不展开该宏体；恢复材料交回外层输入。
    arg_scan_recovered: bool,
    /// `\fontdimen` 覆盖表：(font_id, 参数号) → 值（sp）。TFM 度量在排版层，
    /// 此处仅存覆盖项；无覆盖读回 0（后续接入 TFM 时回退真实参数）。
    /// （第九刀：附每字体最大参数号缓存——越界判定 O(1)，见 fontdimens.rs。）
    fontdimens: FontDimens,
    /// `\hyphenchar` 覆盖表：font_id → 断字符码（无覆盖 = 字体默认 45）。
    hyphenchars: HashMap<u32, i64>,
    /// `\delcode` 表：字符码 → 定界符码（TeX delcode；无覆盖 = 0x500000 默认）。
    delcodes: HashMap<u32, u32>,
    /// TRIP 冲刺：`\mathcode` 表：字符码 → 数学码（TeX initex 默认：
    /// 字母（cat 11）= 0x7100+码（fam 1 数学斜体）、其余 cat 11/12 =
    /// 0x7000+码（fam 0）、非 11/12 = 0x8000 无效）。
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
    /// 当前作业名（`\jobname`）。默认 `texput`，CLI/宿主可按输入文件名覆盖。
    job_name: String,
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
    /// TRIP 冲刺：`\everypar` token 列表（段落开始时注入；存储与查询）。
    everypar_toks: Vec<Token>,
    /// TRIP 冲刺：`\everyhbox` token 列表。
    everyhbox_toks: Vec<Token>,
    /// TRIP 冲刺：`\everyvbox` token 列表。
    everyvbox_toks: Vec<Token>,
    /// TRIP 冲刺：`\everycr` token 列表。
    everycr_toks: Vec<Token>,
    /// TRIP 冲刺：`\errhelp` token 列表（错误帮助文本）。
    errhelp_toks: Vec<Token>,
    /// TRIP 补全批次：`\everydisplay` token 列表（显示数学进入时注入）。
    everydisplay_toks: Vec<Token>,
    /// pdfTeX/e-TeX：`\everyeof` token 列表——**文件帧读尽时注入**（tex.web
    /// pdfTeX `every_eof` 扩展）。expl3 `\__sys_get` 依赖它在 `\input` 结束
    /// 边界回注 `\q_no_value` 探测标记。
    everyeof_toks: Vec<Token>,
    /// TRIP 补全批次：`\skewchar<font>=<num>` 字体偏斜字符表。
    skewchars: HashMap<u32, i64>,
    /// TRIP 冲刺：当前是否处于数学模式（`$`/`$$` 切换；决定 everymath 注入时机）。
    in_math: bool,
    /// 图片管线 Step A：pdfTeX xobject 表（`\pdfximage` 登记；下标 = id-1，
    /// 元素 = (宽 sp, 高 sp, 文件名)。**只追加、不可变**——回滚/检查点无需
    /// 覆盖（同一 id 永远指向同一尺寸），故不进 checkpoint/FmtState。
    pdf_xobjects: Vec<(i64, i64, String)>,
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

    /// iniTeX（INITEX / 格式构建态）语义：换用 tex.web §1273 的初始 catcode 表。
    ///
    /// 只换表——eqtb 本就只含原语（INITEX 无格式预载），语义已对齐。
    /// plain/TRIP 路径继续用 [`Self::new`] 的 plain 风格表，不受影响。
    pub fn initex(mut self) -> Self {
        self.catcodes = CatcodeTable::initex();
        self
    }

    fn with_bytecode(use_bytecode: bool) -> Self {
        let mut e = Self {
            intern: InternTable::new(),
            eqtb: Eqtb::new(),
            scanner_status: ScannerStatus::Normal,
            warning_index: None,
            catcodes: CatcodeTable::new(),
            sfcodes: [1000; 256],
            stack: Vec::new(),
            macro_trace: std::env::var("NTEX_CALL_TRACE")
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|n| *n > 0)
                .map(|n| VecDeque::with_capacity(n.min(MACRO_TRACE_CAP))),
            sink: Box::new(VecSink::default()),
            query_sink: None,
            output_trigger_line: 0,
            read_floor: 0,
            cond_stack: Vec::new(),
            name_in_progress: false,
            err_snapshot: None,
            group_level: 0,
            cur_group_close_via_primitive: false,
            math_left_depth: 0,
            align_frames: Vec::new(),
            align_state: 0,
            group_cond_depth: Vec::new(),
            save_stack: Vec::new(),
            global_pending: false,
            aftergroup: Vec::new(),
            scope_stack: Vec::new(),
            afterassignment: None,
            registers: Registers::new(),
            params: Params::default(),
            font_loader: Box::new(NoFontLoader),
            font_names: Vec::new(),
            font_loads: Vec::new(),
            font_cs_names: Vec::new(),
            cur_font: 0,
            #[cfg(not(target_arch = "wasm32"))]
            watchdog: None,
            output_toks: None,
            output_active: false,
            pending_box_arg: false,
            pending_box_arg_mode: 1,
            trace_suppress_defer: false,
            error_anchor: None,
            ended: false,
            region_steps: 0,
            last_tok: None,
            bc_guard_trace: VecDeque::with_capacity(BC_GUARD_TRACE_CAP),
            bc_guard_active: false,
            bc_guard_dispatches: 0,
            handler_trace: handler_trace_enabled().then(|| VecDeque::with_capacity(32)),
            steps: 0,
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
            arg_scan_recovered: false,
            fontdimens: FontDimens::new(),
            hyphenchars: HashMap::new(),
            delcodes: HashMap::new(),
            // TRIP 冲刺：initex 默认 mathcode（tex.web `init_math_codes`）：
            // 字母（cat 11）→ 0x7100+码（class 7 variable, family 1 数学斜体）、
            // 其余 letter/other_char（catcode 11/12）→ 0x7000+码（class 7
            // variable, family 0），其余 → 0x8000（无效，"Missing character"）。
            mathcodes: default_mathcodes(),
            // tex.web INITEX 初表（default_lccodes 文档）：全零会让
            // `\lowercase` 对字母失能，latex.ltx l.10734 `\rem@pt` 即死于
            // 该惯用法（第十刀收尾新阻塞点根因）。
            lccodes: default_lccodes(),
            uccodes: default_uccodes(),
            dumped: false,
            unless_pending: false,
            cur_if_type: 0,
            cur_if_branch: 0,
            suppress_expansion: 0,
            expand_only: false,
            job_name: "texput".to_string(),
            debug_expand_caller: "",
            math_fonts: [[0; 16]; 3],
            section_label: String::new(),
            parshape: Vec::new(),
            penalty_arrays: Default::default(),
            everymath: Vec::new(),
            everypar_toks: Vec::new(),
            everyhbox_toks: Vec::new(),
            everyvbox_toks: Vec::new(),
            everycr_toks: Vec::new(),
            errhelp_toks: Vec::new(),
            everydisplay_toks: Vec::new(),
            everyeof_toks: Vec::new(),
            skewchars: HashMap::new(),
            in_math: false,
            pdf_xobjects: Vec::new(),
        };
        e.register_builtins();
        e
    }

    /// 注入 VFS 后端（RFC-3；默认本地文件系统）。
    pub fn set_vfs(&mut self, vfs: Box<dyn Vfs>) {
        self.vfs = vfs;
    }

    /// 设置 `\jobname`。空名退回 TeX 默认的 `texput`。
    pub fn set_job_name(&mut self, name: impl Into<String>) {
        let name = name.into();
        self.job_name = if name.is_empty() {
            "texput".to_string()
        } else {
            name
        };
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
            every_toks: EveryToks {
                everypar: self.everypar_toks.clone(),
                everymath: self.everymath.clone(),
                everyhbox: self.everyhbox_toks.clone(),
                everyvbox: self.everyvbox_toks.clone(),
                everycr: self.everycr_toks.clone(),
                everydisplay: self.everydisplay_toks.clone(),
                errhelp: self.errhelp_toks.clone(),
            },
            font_names: self.font_names.clone(),
            font_loads: self.font_loads.clone(),
            font_cs_names: self.font_cs_names.clone(),
            current_font: self.cur_font, // 引擎镜像（FMF382）；Typesetter::export_state 曾从 NodeBuilder 填充
            delcodes: self.delcodes.clone(),
            mathcodes: self.mathcodes.clone(),
            lccodes: self.lccodes,
            uccodes: self.uccodes,
        }
    }

    /// FontId → cs 名表（.fmt 导入后供 NodeBuilder 同步 showbox 字体标识）。
    /// muskip 寄存器 0-2（\thinmuskip/\medmuskip/\thickmuskip）只读（install_builder
    /// 同步数学间距参数——pass2 的 NodeBuilder 重建后须从 expander 恢复）。
    pub fn muskip_registers(&self) -> [Glue; 3] {
        [
            self.registers.muskip(0),
            self.registers.muskip(1),
            self.registers.muskip(2),
        ]
    }

    pub fn font_cs_names_ref(&self) -> &Vec<Option<String>> {
        &self.font_cs_names
    }

    /// 当前是否定义了用户输出例程（`.fmt` 恢复后供排版层同步 box255 路由）。
    pub fn output_defined(&self) -> bool {
        self.output_toks.is_some()
    }

    /// 加载展开引擎状态快照（`.fmt` v1）。
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
                        v.value.active_slot,
                    )),
                    _ => None,
                };
                if let Some((params, body, protected, outer, active_slot)) = pending {
                    let code = Arc::new(compile(&body));
                    if let EqSlot::Macro(v) = eqtb.slot_mut(csid) {
                        v.value = Arc::new(MacroDef {
                            params,
                            body,
                            code: Some(code),
                            protected,
                            outer,
                            active_slot,
                        });
                    }
                }
            }
        }
        self.eqtb = eqtb;
        self.registers = Registers::import(state.registers);
        self.params = state.params;
        self.output_toks = state.output_toks;
        // `\every*` 族：`\everypar` 段首触发链依赖快照恢复（LaTeX 段落钩子机器）
        self.everypar_toks = state.every_toks.everypar;
        self.everymath = state.every_toks.everymath;
        self.everyhbox_toks = state.every_toks.everyhbox;
        self.everyvbox_toks = state.every_toks.everyvbox;
        self.everycr_toks = state.every_toks.everycr;
        self.everydisplay_toks = state.every_toks.everydisplay;
        self.errhelp_toks = state.every_toks.errhelp;
        self.font_names = state.font_names;
        self.font_loads = state.font_loads;
        self.font_cs_names = state.font_cs_names;
        // cur_font 镜像从 fmt 恢复（pass1 dump 时的当前字体；`.fmt` 载入后
        // `\font` 查询与字体相关的内部单位解析以此为准）。
        self.cur_font = state.current_font;
        // 四张 code 表随 v21 恢复（fontmath.ltx 在 fmt 生成期的 `\mathcode`/
        // `\delcode` 赋值此前全丢——非字母字符数学分派退回 INITEX 初表）
        self.delcodes = state.delcodes;
        self.mathcodes = state.mathcodes;
        self.lccodes = state.lccodes;
        self.uccodes = state.uccodes;
        // 运行时状态重置（新文档起点）
        self.stack.clear();
        self.read_floor = 0;
        self.cond_stack.clear();
        self.group_level = 0;
        self.align_frames.clear();
        self.align_state = 0;
        self.group_cond_depth.clear();
        self.save_stack.clear();
        self.global_pending = false;
        self.protected_pending = false;
        self.outer_pending = false;
        self.long_pending = false;
        self.fontdimens.clear();
        self.aftergroup.clear();
        self.scope_stack.clear();
        self.afterassignment = None;
        self.output_active = false;
        self.output_prev_count = usize::MAX;
        self.immediate_pending = false;
        self.read_streams.clear();
        self.write_streams.clear();
        self.suppress_expansion = 0;
        self.expand_only = false;
    }

    /// TEMP DEBUG（第十七刀取证）：卡死前最近事件环。`NTEX_BREAK17` 开启时记录。
    ///
    /// 事件环挂在 [`WatchdogShared::trace`] 上，而看门狗整体 `not(wasm32)` 门控
    /// （见该结构体注释）——故 WASM 侧本设施随之为空实现。调用点**不**做 cfg
    /// 门控（macros.rs 取参数扫描 / mod.rs 组扫描等热路径就地埋点），换取的
    /// 代价是此处在 wasm 上退化为一次函数调用，而非到处铺 cfg。
    fn diag_trace(&self, line: String) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = line;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if std::env::var_os("NTEX_BREAK17").is_none() {
                return;
            }
            if let Some(wd) = &self.watchdog {
                let mut t = wd.trace.lock().unwrap_or_else(|p| p.into_inner());
                if t.len() >= 256 {
                    t.remove(0);
                }
                t.push(line);
            }
        }
    }

    /// 追加一个源码输入（后续 `\input`/VFS 在 M3 接入）。
    pub fn feed_source(&mut self, text: impl Into<Vec<u8>>) {
        let bytes = Arc::from(text.into());
        self.push_frame(InputFrame::Source {
            line_starts: Arc::from(crate::input::line_starts(&bytes)),
            bytes,
            pos: 0,
            state: ScanState::LineStart,
            eof_mark: None,
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
    ///
    /// WASM（M8-A 骨架线）：native 专属的诊断设施——线程看门狗（`std::thread::
    /// spawn`+`sleep`）与 `Instant` 单步计时——整段 cfg 门控跳过。原因有二：
    /// ① 单线程环境（wasm32-unknown-unknown 无线程）诊断线程本就无法工作；
    /// ② wasm32 的 std 无 OS 时钟（`Instant::now`/`SystemTime::now` 直接 panic）。
    /// 浏览器宿主自带的页面级超时（Chrome "page unresponsive"）承担同等职责。
    /// 与 native 的差异：挂死只剩步数上限（下方 `steps > 10_000_000`，无时钟
    /// 依赖，两目标一致）这一道防线 + 宿主超时。native 路径逐行未动。
    pub fn run(&mut self) -> Result<()> {
        let mut steps = 0u64;
        #[cfg(not(target_arch = "wasm32"))]
        let mut step_start = std::time::Instant::now();
        // 线程看门狗：独立执行上下文，主线程卡在单步内部（process_one 不返回）
        // 或 layout 侧死循环时仍能打印最后状态（TRIP L338 挂死定位）。
        #[cfg(not(target_arch = "wasm32"))]
        let wd = Arc::new(WatchdogShared::default());
        // 起始心跳：心跳改每 64 步刷新后，首轮慢启动（64 步内）不算挂死
        //（cfg 只能挂块语句，裸赋值/方法调用表达式语句不支持——故并作一块）。
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.watchdog = Some(wd.clone());
            wd.heartbeat_ms.store(now_millis(), Ordering::Relaxed);
        }
        #[cfg(not(target_arch = "wasm32"))]
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
                    // TEMP DEBUG（第十七刀取证）：卡死前最近事件环
                    let tr = wd.trace.lock().unwrap_or_else(|p| p.into_inner()).clone();
                    for line in tr.iter().rev().take(40).rev() {
                        eprintln!("[break17-trace] {line}");
                    }
                    return; // one-shot：报一次即退，避免刷屏
                }
            });
        }
        // TEMP DEBUG：巨型 TokenList 帧首现定位（挂死诊断）
        #[cfg(not(target_arch = "wasm32"))]
        let mut biglist_reported = false;
        loop {
            // 看门狗：防死循环（ETRIP 诊断用；正常作业远低于此）
            steps += 1;
            self.steps = steps;
            // TEMP DEBUG：>1M token 的 TokenList 帧出现时打印当年 last_tok + 回溯
            #[cfg(not(target_arch = "wasm32"))]
            if diag_enabled("NTEX_BIGLIST_TRACE") && !biglist_reported {
                if let Some(n) = self.stack.iter().rev().find_map(|f| match f {
                    InputFrame::TokenList { items, .. } if items.len() > 1_000_000 => {
                        Some(items.len())
                    }
                    _ => None,
                }) {
                    biglist_reported = true;
                    eprintln!(
                        "[biglist] {} tok, steps={steps}, last_tok={:?}",
                        n, self.last_tok
                    );
                    eprintln!("{}", std::backtrace::Backtrace::force_capture());
                }
            }
            // 心跳 + last_tok（每 64 步）+ 状态快照（每 5000 步，卡死时保留最后状态）。
            // 看门狗线程 2s 轮询 + 10s 阈值，心跳 16Hz 绰绰有余；每步 clock_gettime +
            // mutex 写是纯诊断税（P1 实测占展开吞吐 ~10%），长文档上白付。
            #[cfg(not(target_arch = "wasm32"))]
            if steps & 63 == 0 {
                wd.heartbeat_ms.store(now_millis(), Ordering::Relaxed);
                *wd.last_tok.lock().unwrap_or_else(|p| p.into_inner()) =
                    self.last_tok.clone().unwrap_or_default();
            }
            #[cfg(not(target_arch = "wasm32"))]
            if steps % 5000 == 0 {
                let state = format!(
                    "steps={steps} last_tok={:?} stack={}",
                    self.last_tok,
                    self.debug_stack_summary()
                );
                *wd.state.lock().unwrap_or_else(|p| p.into_inner()) = state;
            }
            // 步数上限：无时钟依赖（wasm32 同样生效），是 wasm 侧唯一的应用层死循环防线。
            // expl3 真载入（codepoint 三表 \read 装载 + finalize + CaseFolding/
            // SpecialCasing）2026-09-15 实测 ~1.3×10⁷ 步（2-CPU VM 25k steps/s），
            // 10⁷ 上限把合法慢载入误判成死循环（探针死于 finalize 中段）。
            // 默认 6400 万（≈5x 余量）；NTEX_MAX_STEPS 可覆盖（探针/lvt 收紧用）。
            if steps > max_steps() {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    wd.done.store(true, Ordering::Relaxed);
                }
                return Err(Error::invalid_input(format!(
                    "处理步骤超限（疑似死循环）；输入栈深 {}：{}",
                    self.stack.len(),
                    self.debug_stack_summary()
                )));
            }
            // 诊断：定期进度日志（stderr 实时可见，进程被 SIGKILL 也不丢）
            #[cfg(not(target_arch = "wasm32"))]
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
            // 与心跳同频检查（每次循环省一次 clock_gettime；检测粒度 64 步）。
            #[cfg(not(target_arch = "wasm32"))]
            if steps & 63 == 0 && step_start.elapsed().as_secs() >= 5 {
                eprintln!(
                    "[watchdog] 单步超时 5s steps={steps} last_tok={:?} stack={}",
                    self.last_tok,
                    self.debug_stack_summary()
                );
                step_start = std::time::Instant::now();
            }
            // `\end` 已执行 → 主循环立即停（tex.web final_cleanup：终结信号，
            // 无论嵌套多深——expl3 A1.undevicies 爆栈修复）。
            // 注意：不放 `process_one` 内（flush_writes→expand_region 子展开
            // 也调 process_one，会误拦 write 内容展开）。
            if self.ended {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    wd.done.store(true, Ordering::Relaxed);
                }
                break;
            }
            // NTEX_SANITY_CHECK：错误恢复后状态完整性校验（每次迭代检查）
            self.sanity_check_after_error();
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
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        wd.done.store(true, Ordering::Relaxed);
                    }
                    self.report_error_context();
                    return Err(e);
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            wd.done.store(true, Ordering::Relaxed);
        }
        self.report_incomplete_conditions();
        Ok(())
    }

    /// TeX final_cleanup：未闭合条件 → 可恢复转录消息
    /// "! Incomplete \ifxxx; all text was ignored after line N."
    /// （TRIP L363 `\ifcase3` 故意不闭合；\end 与 \endinput/EOF 两个入口
    /// 都走此路径——tex.web final_cleanup。LIFO 顺序对齐条件栈。）
    fn report_incomplete_conditions(&mut self) {
        if self.cond_stack.is_empty() {
            return;
        }
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
        for f in self.cond_stack.iter().rev() {
            let _ = self.sink.write16(format!(
                "! Incomplete {}; all text was ignored after line {}.\n",
                Self::if_type_name(f.if_type),
                f.line
            ));
        }
        self.cond_stack.clear();
    }

    /// A3：当前源码上下文——从输入栈找最近的 [`InputFrame::Source`] 帧，
    /// 当前输入行号（纯 pos 计算——不经过 error_context 的 error_anchor 锚点，
    /// 锚点是 clamp_dimen 报错回溯用，会残留污染 \inputlineno 的值）。
    fn current_line_no(&self) -> usize {
        for frame in self.stack.iter().rev() {
            if let InputFrame::Source {
                line_starts, pos, ..
            } = frame
            {
                // 预建行索引二分（原实现每帧从头数 \n——组事件高频调用下的 O(pos) 热点）。
                return line_starts
                    .partition_point(|&s| (s as usize) <= *pos)
                    .max(1);
            }
        }
        0
    }

    /// 行定位（`line_starts` 二分）：返回 (行号, 行起始偏移, 行结束偏移)。
    ///
    /// 行结束偏移不含行尾 `\n`（与逐字节扫描一致）。替代原 `error_context`/
    /// `error_context_pos` 的两次 O(pos) 全文扫描——条件帧入栈（\if 高频）也取
    /// 行号，长文档上线性扫描整体 O(n²)（0b2fc9b 对 `current_line_no` 同类修复
    /// 的补全）。
    fn locate_line(bytes: &[u8], line_starts: &[u32], end: usize) -> (usize, usize, usize) {
        let line_no = line_starts.partition_point(|&s| (s as usize) <= end).max(1);
        let line_start = line_starts[line_no - 1] as usize;
        // 下一行起始 = 本行行尾 `\n` 下标 + 1（末行无下一行 → 全文长度）
        let line_end = match line_starts.get(line_no) {
            Some(&s) => (s as usize).saturating_sub(1).min(bytes.len()),
            None => bytes.len(),
        };
        (line_no, line_start, line_end)
    }

    /// 由扫描位置反推 (行号, 行内容)。宏展开中的错误回退到最近的源文件行
    /// （TeX `l.N` 上下文行语义）。
    fn error_context(&self) -> Option<(usize, String)> {
        for frame in self.stack.iter().rev() {
            if let InputFrame::Source {
                bytes,
                line_starts,
                pos,
                ..
            } = frame
            {
                let bytes: &[u8] = bytes;
                // 报错锚点（clamp_dimen 值扫描报错）：pos 已推进到报错后的行，
                // 用值扫描起始位置回溯（tex.web l.N 上下文停在值所在行）。
                // 只读（&self）；消费由各报错出口负责（undef 路径自行清除，
                // report_error_context 出口见下）。
                let end = self.error_anchor.unwrap_or(*pos).min(bytes.len());
                let (line_no, line_start, line_end) = Self::locate_line(bytes, line_starts, end);
                let line = String::from_utf8_lossy(&bytes[line_start..line_end]).into_owned();
                return Some((line_no, line));
            }
        }
        None
    }

    /// A3：把错误上下文行（`l.N <行内容>`）写入转录（TeX error() 的上下文行）。
    /// 尾 \n 由 write16 自动追加（与 write_error 的 `l.N\n\n` 块分隔一致）。
    fn report_error_context(&mut self) {
        if let Some((n, line)) = self.error_context() {
            let _ = self.sink.write16(format!("l.{n} {line}\n"));
        }
        // 锚点一次性使用（报错上下文输出后清除——后续报错用当前 pos）
        self.error_anchor = None;
    }

    /// 仅取错误上下文行号（不构建行内容字符串）。
    ///
    /// 条件帧入栈（每次 `\if*`）只记行号供 `! Incomplete \ifxxx ... after line N`
    /// 用——此前走 [`Self::error_context`] 会把**整行**做 UTF-8 转换（长行样张上
    /// 每 `\if` 一次百 KB 级拷贝）。行号计算与 `error_context` 完全一致（同一
    /// `error_anchor` 回溯语义），逐位等价。
    fn error_line_no(&self) -> usize {
        for frame in self.stack.iter().rev() {
            if let InputFrame::Source {
                bytes,
                line_starts,
                pos,
                ..
            } = frame
            {
                let end = self.error_anchor.unwrap_or(*pos).min(bytes.len());
                let (line_no, _, _) = Self::locate_line(bytes, line_starts, end);
                return line_no;
            }
        }
        0
    }

    /// 错误上下文带行内错误位置（Source 帧 pos 相对行首的偏移）。
    fn error_context_pos(&self) -> Option<(usize, String, usize)> {
        for frame in self.stack.iter().rev() {
            if let InputFrame::Source {
                bytes,
                line_starts,
                pos,
                ..
            } = frame
            {
                let bytes: &[u8] = bytes;
                let end = (*pos).min(bytes.len());
                let (line_no, line_start, line_end) = Self::locate_line(bytes, line_starts, end);
                let line = String::from_utf8_lossy(&bytes[line_start..line_end]).into_owned();
                let pos = end.saturating_sub(line_start).min(line.len());
                return Some((line_no, line, pos));
            }
        }
        None
    }

    /// TeX 错误恢复模式统一入口：`! 消息` 写入转录 + `l.N 上下文行`（tex.web
    /// error()：报错后继续执行，不终止作业）。恢复动作由调用方决定（钳制/插入/
    /// 忽略）；`<to be read again>` 与 help 行由 write_error 等完整格式路径输出。
    fn report_error(&mut self, msg: &str) {
        if self.err_snapshot.is_none() {
            self.err_snapshot = Some((self.group_level, self.cond_stack.len()));
        }
        self.sink.report_error(msg);
        self.report_error_context();
    }

    /// ETRIP P0 \muexpr 校准：错误帮助行（tex.web help1..help5）转发到 sink。
    /// mu_error 等场合在 `! <msg>` 后追加一行提示（如 etrip.tex L906："I'm going to
    /// assume that 1mu=1pt when they're mixed."），与参考 log 字面对齐。
    fn report_help(&mut self, text: &str) {
        self.sink.report_help(text);
    }

    /// NTEX_SANITY_CHECK 设施：错误恢复后状态完整性校验。
    /// 在主循环每次迭代开始调用——若存在待校验快照（刚发生过错误恢复），
    /// 对比当前 (组级, 条件栈深) 与错误前：组级偏离 >1 或条件栈偏离 >1 视为
    /// 恢复路径写坏了状态（内部 bug），NTEX_SANITY_CHECK=1 时打警告。
    /// 快照一律清除（一次错误只校验一次）。
    fn sanity_check_after_error(&mut self) {
        let Some((g0, c0)) = self.err_snapshot.take() else {
            return;
        };
        let dg = self.group_level.abs_diff(g0);
        let dc = (self.cond_stack.len() as i64 - c0 as i64).abs();
        if diag_enabled("NTEX_SANITY_CHECK") && (dg > 1 || dc > 1) {
            eprintln!(
                "[sanity] 错误恢复后状态偏离：组级 {g0}->{} (Δ{dg})，条件栈 {c0}->{} (Δ{dc}) last_tok={:?}",
                self.group_level,
                self.cond_stack.len(),
                self.last_tok
            );
        }
    }

    /// 错误上下文 token 的简单显示（TeX show_token_list：字符直接显示、cs 显示 `\名`）。
    fn trace_tok_simple(&self, tok: Token) -> String {
        if let Some(csid) = tok.csid() {
            format!("\\{}", self.intern.name(csid))
        } else if let Some(ch) = tok.charcode() {
            char::from_u32(ch).unwrap_or('?').to_string()
        } else {
            "?".to_string()
        }
    }

    /// 统一错误消息输出（TeX error()/show_context 语义）：`! 消息` 后接
    /// `<to be read again>` 段（peek 输入流下一个 token，18 列）与 `l.N`
    /// 行上下文两行显示（位置前内容；n 空格 + 位置后 ≤trick 字符 + `...`）。
    /// TRIP 对齐：参考 log L9-11 的 `<to be read again>` / l.94 段。
    ///
    /// 该格式逐字对齐 web2c TRIP 参考输出（error_line=79 的 show_context 伪打印）。
    fn write_error(&mut self, msg: &str) {
        self.write_error_impl(msg, true, None);
    }

    /// 报错 + help 一体输出（无 `<to be read again>`，TeX error() 语义：报错
    /// 发生时无 back_input 待读 token——主循环"裸用内部量"类错误，如 etrip
    /// gluestretchorder 段 l.932/l.933 的 4 个 "You can't use ..."）。help
    /// 紧随 l.N 上下文行后、错误块尾空行由 write16 追加的 \n 产生（块结构：
    /// ! 消息 / l.N 两行 / help 4 行 / 空行——与参考逐行一致）。扫描参数失败
    /// 的报错才带 read-again（write_error，如 Missing number / Bad register
    /// code）。
    fn write_error_help_no_read_again(&mut self, msg: &str, help: &str) {
        self.write_error_impl(msg, false, Some(help));
    }

    /// 报错 + help 一体输出（带 `<to be read again>`，扫描参数失败且报错后
    /// 输入流仍有待处理 token 时——etrip sparse arrays 段 l.970-973 的
    /// "Bad register code" 块即此类：read-again token 由 fetch() 取输入流
    /// 下一 token（宏体 `#1\1=-1#1...` 场景恰为再次出现的 `\countdef` 等，
    /// 与参考 `<to be read again>` 后的 token 一致）。块结构与
    /// write_error_help_no_read_again 相同，只是 error 后先输出 read-again 段）。
    fn write_error_help(&mut self, msg: &str, help: &str) {
        self.write_error_impl(msg, true, Some(help));
    }

    /// write_error 实现：read_again=true 时先输出 `<to be read again>` 段
    /// （TRIP 对齐：扫描参数失败时错误恢复后将被读取的 token）。
    fn write_error_impl(&mut self, msg: &str, read_again: bool, help: Option<&str>) {
        let mut s = format!("! {msg}\n");
        // <to be read again>：错误恢复后将被读取的 token（peek 输入流下一个）；
        // 第二行 = 描述宽度（19：`<to be read again> `）空格 + token 直接显示。
        if read_again {
            if let Some((tok, _)) = self.fetch().ok().flatten() {
                let desc = self.trace_tok_simple(tok);
                s.push_str("<to be read again> \n");
                s.push_str(&format!("{}{}\n", " ".repeat(19), desc));
                self.unread(tok);
            }
        }
        // l.N 行上下文（两行：位置前内容 + n 空格 + 位置后字符）
        if let Some((n, line, byte_pos)) = self.error_context_pos() {
            // **口径陷阱（2026-09-19 现场）**：`error_context_pos` 给的是源内的
            // **字节**偏移（源按字节读取），而 tex.web 的 trick_buf /
            // half_error_line 全按**字符**计数。两套口径混用——旧代码把字节
            // 偏移与描述字符数相加、再拿结果当字节下标切 `&str`——会在含多字节
            // 字符（中文）的行上把裁剪点算进字符内部：`&before[start..]` 撞 UTF-8
            // 边界直接 panic，而 wasm 的 `panic = "abort"` 把它退化成一句
            // 无位置信息的 `RuntimeError: unreachable`（Tauri 工作台整窗报
            // 「加载失败」，排查成本极高）。
            // 故此处一律换算成**字符下标**再裁剪：ASCII 源两套口径恒等，
            // TRIP/ETRIP 逐字对齐不受影响（口径只在多字节输入下才有分歧）。
            let chars: Vec<char> = line.chars().collect();
            let pos = line[..snap_char_boundary(&line, byte_pos)]
                .chars()
                .count()
                .min(chars.len());
            // tex.web @<Show the context...@>：第一行超 half_error_line 时左侧
            // 裁剪（`...` + 尾部）。参考 TRIP log 反推 half_error_line=32
            // （l.253：l=6,k=58 → 显示尾部 23 字符；l.2：l=4,k=29 → 尾部 25）。
            let l_desc = format!("l.{n} ");
            let l_len = l_desc.chars().count();
            let k = l_len + pos;
            const HALF_ERROR_LINE: usize = 32;
            let before: String = chars[..pos].iter().collect();
            let (prefix, before_shown) = if l_len + k > HALF_ERROR_LINE {
                // tex.web：trick_buf[(l+k-h+3)..k-1]，trick_buf 前 l_len 字符是描述
                // → before 起点 = k - h + 3（l.2 参考：k=33 → before[4..]="case..."）
                let start = k.saturating_sub(HALF_ERROR_LINE - 3).min(pos);
                ("...", chars[start..pos].iter().collect::<String>())
            } else {
                ("", before)
            };
            let l1 = format!("{l_desc}{prefix}{before_shown}");
            s.push_str(&l1);
            s.push('\n');
            // 第二行上限：TeX trick_count = first_count+1+error_line-half_error_line
            //（TRIP 参考 L94：pos=11 → 42 字符）
            let max_after = pos + 1 + 79 - 48;
            let after: String = chars[pos..].iter().take(max_after).collect();
            let suffix = if chars.len().saturating_sub(pos) > max_after {
                "..."
            } else {
                ""
            };
            s.push_str(&format!(
                "{}{}{}\n",
                " ".repeat(l1.chars().count()),
                after,
                suffix
            ));
        }
        // help1..6：紧随 l.N 上下文行（TeX error() 在上下文行后打印帮助文本，
        // 无额外空行；块间空行由 write16 尾部追加的 \n 产生）
        if let Some(h) = help {
            s.push_str(h);
        }
        let _ = self.sink.write16(s);
    }

    /// TeX scan_int 缺数恢复：`! Missing number, treated as zero.` + `<to be read
    /// again>` + l.N 上下文 + 3 行 help（tex.web error() + @<Report an improper...@>；
    /// 参考 trip.log 各 Missing number 段逐字对齐——l.253/l.419 等格式差即源于
    /// 此前缺失这些行）。
    fn report_missing_number(&mut self) {
        // 第二十刀定位开关：NTEX_NUM_TRACE——现场打出输入栈摘要 + 宏调用链 +
        // 最近 token，用于锁定「数字扫描读到哪个 token、从哪条宏链进来」。
        if std::env::var_os("NTEX_NUM_TRACE").is_some() {
            fn show(intern: &crate::intern::InternTable, t: &Token) -> String {
                if let Some(csid) = t.csid() {
                    format!("\\{}", intern.name(csid))
                } else if let Some(n) = t.param_number() {
                    format!("#{n}")
                } else if let Some(ch) = t.charcode().and_then(char::from_u32) {
                    format!("{ch:?}/cc{:?}", t.catcode())
                } else {
                    format!("{t:?}")
                }
            }
            let mut frames = Vec::new();
            for f in self.stack.iter().rev().take(4) {
                match f {
                    InputFrame::TokenList { items, pos } => {
                        let next = items[*pos..]
                            .iter()
                            .take(10)
                            .map(|(t, _)| show(&self.intern, t))
                            .collect::<Vec<_>>()
                            .join(" ");
                        let done = items[..*pos]
                            .iter()
                            .rev()
                            .take(4)
                            .map(|(t, _)| show(&self.intern, t))
                            .collect::<Vec<_>>()
                            .join(" ");
                        frames.push(format!(
                            "{}tok@{} next=[{next}] done=[{done}]",
                            items.len(),
                            pos
                        ));
                    }
                    InputFrame::MacroArg { items, pos } => {
                        let next = items[*pos..]
                            .iter()
                            .take(10)
                            .map(|(t, _)| show(&self.intern, t))
                            .collect::<Vec<_>>()
                            .join(" ");
                        frames.push(format!("MacroArg {}tok@{} next=[{next}]", items.len(), pos));
                    }
                    InputFrame::Source { bytes, pos, .. } => {
                        let off = (*pos).min(bytes.len());
                        let ctx = String::from_utf8_lossy(&bytes[off..(off + 80).min(bytes.len())])
                            .replace('\n', "⏎");
                        frames.push(format!("source@{} [{ctx}]", pos));
                    }
                    other => frames.push(format!("{other:?}")),
                }
            }
            eprintln!(
                "[missing-number] line={} last_tok={:?}\n  frames(外→内): {}",
                self.error_line_no(),
                self.last_tok,
                frames.join("\n  ")
            );
        }
        self.write_error("Missing number, treated as zero.");
        let _ = self.sink.write16(
            "A number should have been here; I inserted `0'.\n\
             (If you can't figure out why I needed to see a number,\n\
             look up `weird error' in the index to The TeXbook.)\n"
                .to_string(),
        );
    }

    /// 是否已执行显式 `\end`（finish 对未闭合组/math 按 TeX 语义降级为警告）。
    pub fn is_ended(&self) -> bool {
        self.ended
    }

    /// 复位 `\end` 终结标志（宿主复用同一引擎实例编译**下一份文档**时调用）。
    ///
    /// 语义与边界：`\end` 是"本作业结束"的终结信号，`run` 主循环见到它立即停
    /// （tex.web final_cleanup）；但 `\end` 之后的收尾清理（未闭合组/数学/待
    /// shipout 的降级、冲页、输出例程交错）由排版侧 `finish_doc`/`finish`
    /// 完成，**格式状态（eqtb 宏/寄存器/字体表）按 TeX 语义保留**——故复位该
    /// 标志只是允许引擎继续处理新输入，不改变任何格式状态。
    ///
    /// 真现场（2026-09-19）：`IncrementalTypesetter` 长驻实例的第二次 `compile`
    /// 恒 0 页——首份作业以 `\end` 收尾后标志残留，第二份文档全部段被跳过
    /// （ntex-studio 每次编辑重编译走这条路；`ntex-dvi` 一次性进程不受影响）。
    pub fn clear_ended(&mut self) {
        self.ended = false;
    }

    /// `\tracingcommands` 输出一行 `{模式: 描述}`（tex.web show_cur_cmd_chr：
    /// 模式只在变化时打印，shown_mode 记忆）。扫描器内部的可展开原语展开
    /// （tex.web expand() 开头 `if tracing_commands>1 then show_cur_cmd_chr`）
    /// 也复用此方法——级别 2（\tracingcommands2）时展开入口同样追踪。
    fn trace_token_now(&mut self, tok: Token) {
        let m = self.sink.mode_name();
        let desc = self.trace_token_desc(tok);
        // tex.web show_cur_cmd_chr：`{...}` 不带换行，由 end_diagnostic(false) 的
        // print_nl("") 收尾——即 write16 追加的那一个换行。此处不得再带 `\n`
        // （否则每条追踪后多一空行；TRIP 参考中追踪行是连续的）。
        let line = if self.shown_trace_mode.as_deref() == Some(m.as_str()) {
            format!("{{{desc}}}")
        } else {
            self.shown_trace_mode = Some(m.clone());
            format!("{{{m}: {desc}}}")
        };
        let _ = self.sink.write16(line);
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
                InputFrame::Source {
                    bytes, pos, state, ..
                } => format!("Source({}B,pos={},state={:?})", bytes.len(), pos, state),
                InputFrame::Macro { body, pos, .. } => {
                    format!("Macro({}tok,pos={})", body.len(), pos)
                }
                InputFrame::Bytecode { pc, .. } => format!("Bytecode(pc={})", pc),
                InputFrame::TokenList { items, pos } => {
                    format!("TokenList({}tok,pos={})", items.len(), pos)
                }
                InputFrame::MacroArg { items, pos } => {
                    format!("MacroArg({}tok,pos={})", items.len(), pos)
                }
                InputFrame::One { tok, .. } => format!("One({tok:?})"),
                InputFrame::OutputRoutine { items, pos } => {
                    format!("OutputRoutine({}tok,pos={})", items.len(), pos)
                }
                InputFrame::AlignU { items, pos } => {
                    format!("AlignU({}tok,pos={})", items.len(), pos)
                }
                InputFrame::AlignV { items, pos } => {
                    format!("AlignV({}tok,pos={})", items.len(), pos)
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
        // ---- 输出例程刀 1（G1/G4）：tex.web fire_up 的点火侧语义 ----
        // ship_out 清零（tex.web L12707）：上一例程若真出了页，连续死循环计数归零。
        if self.sink.take_page_shipped() && self.params.misc[DEAD_CYCLES_IDX] != 0 {
            self.params.misc[DEAD_CYCLES_IDX] = 0;
            self.sink
                .param_changed(ParamKind::MiscInt(DEAD_CYCLES_IDX), ParamValue::Number(0))?;
        }
        // @<Set the value of |output_penalty|@>：最佳断点是惩罚节点 → 其惩罚值，
        // 否则 inf_penalty。geq_word_define = 全局赋值（不进 save_stack），
        // 例程内 \outputpenalty=... 的覆盖到下一次断页被重写，与 tex.web 一致。
        let bp = self.sink.output_break_penalty().unwrap_or(INF_PENALTY);
        self.params.misc[OUTPUT_PENALTY_IDX] = bp;
        self.sink.param_changed(
            ParamKind::MiscInt(OUTPUT_PENALTY_IDX),
            ParamValue::Number(bp),
        )?;
        // fire_up：dead_cycles >= max_dead_cycles → "Output loop" 错并转默认输出
        // （直接 ship box255），不再点火用户例程——打破"例程永不 ship"死循环。
        if self.params.misc[DEAD_CYCLES_IDX] >= self.params.misc[MAX_DEAD_CYCLES_IDX] {
            let dead = self.params.misc[DEAD_CYCLES_IDX];
            self.sink
                .report_error(&format!("Output loop---{dead} consecutive dead cycles"));
            self.sink.report_help(
                "I've concluded that your \\output is awry; it never does a\n\
                 \\shipout, so I'm shipping \\box255 out myself. Next time\n\
                 increase \\maxdeadcycles if you want me to be more patient!",
            );
            self.sink.default_output_routine();
            self.params.misc[DEAD_CYCLES_IDX] = 0;
            self.sink
                .param_changed(ParamKind::MiscInt(DEAD_CYCLES_IDX), ParamValue::Number(0))?;
            self.output_prev_count = usize::MAX;
            return Ok(false);
        }
        // @<Fire up the user's output routine and |return|@>：incr(dead_cycles)
        // （例程 ship 时由 take_page_shipped 分支清零）。
        self.params.misc[DEAD_CYCLES_IDX] += 1;
        self.sink.param_changed(
            ParamKind::MiscInt(DEAD_CYCLES_IDX),
            ParamValue::Number(self.params.misc[DEAD_CYCLES_IDX]),
        )?;
        let toks = self.output_toks.clone().expect("已检查 is_some");
        self.sink.take_output_pending();
        self.output_active = true;
        let items: Vec<(Token, bool)> = toks.iter().map(|&t| (t, false)).collect();
        // 输出例程隐式组（tex.web：例程在 save_stack 组内执行——\tracingcommands0
        // 等例程内赋值组结束恢复；TRIP L107 例程后 \tracingcommands 回 30）。
        // 组种类 output_group=8（tex.web group_code，ETRIP L396 \currentgrouptype 检查）
        self.sink.output_routine_begin()?;
        self.begin_group()?;
        self.push_frame(InputFrame::OutputRoutine {
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
        if bytecode_guard_limit() == 0 {
            return self.process_one_inner();
        }
        // handler 可经 `expand_region` 嵌套调用 process_one；内层必须沿用外层
        // dispatch 的计数，不能重置，否则恰好会漏掉 `\edef` 中的活锁。
        let outer_dispatch = !self.bc_guard_active;
        if outer_dispatch {
            self.bc_guard_active = true;
            self.bc_guard_dispatches = 0;
        }
        let result = self.process_one_inner();
        if outer_dispatch {
            self.bc_guard_active = false;
        }
        result
    }

    /// `process_one` 的实际 dispatcher；护栏 wrapper 使 handler 内的 fetch 也纳入同一计数。
    fn process_one_inner(&mut self) -> Result<bool> {
        match self.fetch()? {
            None => Ok(false),
            Some((tok, noexpand)) => {
                // 诊断：记录最近处理的 token（watchdog/单步超时 dump 用；cs 显示真实名字）
                self.last_tok = Some(match tok.csid() {
                    Some(csid) => format!("\\{}", self.intern.name(csid)),
                    None => format!("{tok:?}"),
                });
                // 第十八刀（四）：控制序列真正进入 dispatcher 的现场。`fetch` 内的
                // 实参/定义扫描会吞掉大量 token，却不会再次经过这里；故护栏转储的
                // 尾项就是仍占着 handler、尚未向主循环交还 token 的原语/宏。
                let read_floor = self.read_floor;
                if let (Some(csid), Some(trace)) = (tok.csid(), self.handler_trace.as_mut()) {
                    if trace.len() == BC_GUARD_TRACE_CAP {
                        trace.pop_front();
                    }
                    trace.push_back((csid, read_floor));
                }
                if bytecode_guard_limit() != 0 {
                    if let Some(csid) = tok.csid() {
                        if self.bc_guard_trace.len() == BC_GUARD_TRACE_CAP {
                            self.bc_guard_trace.pop_front();
                        }
                        self.bc_guard_trace.push_back(csid);
                    }
                }
                // M4-5 对齐状态机拦截（tex.web §749-823；详见 align.rs）：
                // preamble 阶段分类收集模板 token；body raw 阶段拦 `&`/`\span`/
                // `\cr`/`\crcr`（Insert v_j）与 `}` 平衡（对齐组闭括号）。
                if !noexpand && self.align_on_token(tok)? {
                    return Ok(true);
                }
                // \tracingcommands（misc 下标 3）：每命令一行 `{模式: 描述}`；
                // 模式只在变化时打印（tex.web show_cur_cmd_chr 的 shown_mode 语义）；
                // 子展开（expand_region：\write 内容等）抑制——TeX 只在主循环追踪。
                // \moveleft/\moveright 的 box 参数（tex.web scan_box）同样抑制：
                // 模式变化 = 盒子组内容开始 → 立即恢复；非组盒子原语（\copy 等）
                // 本次仍抑制、执行后恢复（trace_suppress_defer）。
                if self.pending_box_arg {
                    if self.sink.mode_code() != self.pending_box_arg_mode {
                        self.trace_suppress -= 1;
                        self.pending_box_arg = false;
                    } else if let Some(csid) = tok.csid() {
                        if let EqSlot::Primitive(p) = self.eqtb.slot(csid) {
                            if matches!(
                                p,
                                Primitive::Copy
                                    | Primitive::Box
                                    | Primitive::UnHBox
                                    | Primitive::UnHCopy
                                    | Primitive::UnVBox
                                    | Primitive::UnVCopy
                                    | Primitive::LastBox
                                    | Primitive::VSplit
                            ) {
                                self.pending_box_arg = false;
                                self.trace_suppress_defer = true;
                                // tex.web：`\setbox0=\box1` 无组盒子实参在 box_end
                                // 后回到 prefixed_command `done:` 触发 `\afterassignment`
                                // （组版走 begin_group 的同款触发位）。
                                self.finish_assignment();
                            }
                        }
                    }
                }
                // tex.web pass_text：跳过区（\else/\or 后）的 token 不追踪
                // （show_cur_cmd_chr 只在 main_control/get_x_token 的处理路径）——
                // 仅 \fi（跳过区结束标记）保留追踪（参考 log {\fi} 存在）。
                let skipping = self
                    .cond_stack
                    .last()
                    .map(|c| c.state == CondState::Skipping)
                    .unwrap_or(false);
                let is_fi = matches!(tok.kind(), TokenKind::ControlSeq)
                    && matches!(
                        self.eqtb.slot(tok.csid().expect("ControlSeq 必有 csid")),
                        EqSlot::Primitive(Primitive::Fi)
                    );
                if self.params.misc[3] > 0 && self.trace_suppress == 0 && (!skipping || is_fi) {
                    self.trace_token_now(tok);
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
                    self.step_conditional(op, tok)?;
                    return Ok(true);
                }
                if self.is_skipping() {
                    // 跳过模式：其余 token 直接丢弃（不展开）。
                    // tex.web get_next 的 forbidden 检查（TRIP L363）：skip 区
                    // 出现 **outer 宏** → `Incomplete \if...; all text was
                    // ignored after line N.` + 插入 \fi 恢复。此前惰性 Skipping
                    // 帧的主循环丢弃无此检查（\ifcase 真分支 skip_ahead 之外的
                    // 另一条 skip 路径），pdfTeX 报 Incomplete 而 NTex 静默。
                    if tok.csid().is_some() && self.is_outer_for_token(tok) {
                        let name = self.intern.name(tok.csid().expect("已判 csid")).to_owned();
                        let ifname = Self::if_type_name(self.cur_if_type).to_owned();
                        let ln = self.error_line_no();
                        let _ = self.sink.write16(format!(
                                    "! Incomplete \\{ifname}; all text was ignored after line {ln}.\n\
                                     <inserted text>\n                \\fi \n\
                                     <to be read again>\n                   \\{name}\n\
                                     A forbidden control sequence occurred in skipped text.\n\
                                     This kind of error happens when you say `\\if...' and forget\n\
                                     the matching `\\fi'. I've inserted a `\\fi'; this might work.\n"
                                ));
                        // 插入 \fi 闭合全部未决条件帧（对齐 skip_ahead
                        // 的 outer 恢复臂语义）
                        self.cond_stack.clear();
                        self.cur_if_type = 0;
                        self.cur_if_branch = 0;
                        return Ok(true);
                    }
                    return Ok(true);
                }
                // list 机制刀：`\everypar` 触发链（tex.web `new_graf`）。
                // LaTeX 的 `\list`→`\item` 依赖段首触发 `\everypar` 清 `\@newlist`；
                // 引擎此前从不触发，`\@trivlist` 逐条报 "missing \item"。
                // tex.web 顺序（`vmode+letter…` 臂）：`back_input; new_graf(true)`
                // ——先开段、注入 `\everypar`、再回放触发 token。不可回放触发
                // token 的 `\indent`/`\noindent`（`start_par` 臂直接消费）。
                // 门禁 `\everypar` 非空：plain/TRIP 从不设它 → 行为逐字节不变。
                if !self.everypar_toks.is_empty() && self.sink.par_begin_imminent() {
                    if let Some((indented, back_input)) = self.par_trigger_kind(tok) {
                        self.sink.par_begin(indented)?;
                        if back_input {
                            self.unread(tok);
                        }
                        // 后压 = 先展开：`\everypar` 在触发 token 之前落列表
                        // （LaTeX 段落钩子机器 `\box_gset_to_last` 取缩进盒时，
                        // 水平列表里必须只有缩进盒）。
                        let items: Vec<(Token, bool)> =
                            self.everypar_toks.iter().map(|t| (*t, false)).collect();
                        self.push_frame(InputFrame::TokenList {
                            items: Arc::from(items),
                            pos: 0,
                        });
                        return Ok(true);
                    }
                }
                self.process_token(tok)?;
                // 非组盒子原语（\copy 等）作为 \moveleft 参数：执行完恢复追踪
                if self.trace_suppress_defer {
                    self.trace_suppress -= 1;
                    self.trace_suppress_defer = false;
                }
                Ok(true)
            }
        }
    }

    /// 当前 token 是否会开段（tex.web `vmode` 水平材料触发集的子集）：
    /// 返回 `(indented, back_input)`——字母/其他字符开段并把触发 token 压回
    /// 输入（`back_input` 臂）；`\indent`/`\noindent`（`start_par` 臂）开段并
    /// **直接消费**，不回放（否则水平模式 `\indent` 再落一个缩进盒）。
    /// `\let` 别名经 eqtb 槽判定，同样命中。
    ///
    /// `vmode+un_hbox` 臂（tex.web L21104-21111 `back_input; new_graf(true)`）：
    /// `\leavevmode`（=`\unhbox\voidb@x`）赖此进入水平模式——LaTeX `\@tabular`
    /// 的 `\leavevmode\hbox\bgroup`、`\@maketitle` 作者块的 tabular 居中都踩
    /// 这里；此前垂直模式 `\unhbox` 直接拆包，盒落进竖列表 → 永不居中。
    fn par_trigger_kind(&self, tok: Token) -> Option<(bool, bool)> {
        match tok.kind() {
            TokenKind::Char => match tok.catcode() {
                Some(Catcode::Letter) | Some(Catcode::Other) => Some((true, true)),
                _ => None,
            },
            TokenKind::ControlSeq => {
                let csid = tok.csid()?;
                match self.eqtb.slot(csid) {
                    EqSlot::Primitive(Primitive::Indent) => Some((true, false)),
                    EqSlot::Primitive(Primitive::NoIndent) => Some((false, false)),
                    EqSlot::Primitive(Primitive::UnHBox)
                    | EqSlot::Primitive(Primitive::UnHCopy) => Some((true, true)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// 处理单个 token（展开宏/原语，其余输出）。
    fn process_token(&mut self, tok: Token) -> Result<()> {
        // 注意：此处不加 `ended` 拦截——`\end` 收尾时要 flush 延迟写流
        // （`flush_writes` → `expand_to_string` → 本函数），拦截会丢 write
        // 内容。终结由 `process_one` 顶部检查保证。
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
                        // 锚点一次性使用（本出口不经 report_error_context；残留
                        // 锚点会把后续所有 `l.N` 上下文拉回旧行）
                        self.error_anchor = None;
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
                            Catcode::EndGroup => {
                                // `}` 字符闭合：报错文本区分标志（见 end_group）
                                self.cur_group_close_via_primitive = false;
                                self.end_group()
                            }
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
                    // ETRIP 冲刺：\mathchardef\cs=<num> 绑定的 cs 执行时推送
                    // 完整数学字符原子（tex.web main_control `math_given`：
                    // class<<12 | fam<<8 | char 全量进 mlist——demo 差异 #3：
                    // \sum="1350 的 fam3（cmex10 大算符）此前被丢成 fam0 文本
                    // 字符）。数学模式外退回 \char 文本行为（TeX 语义应为
                    // "Missing $ inserted" 报错，ETRIP 后续对齐）。
                    SlotAction::MathChar(code) => {
                        if self.in_math {
                            self.sink.math_char_full(code)
                        } else {
                            self.sink.token(Token::char(Catcode::Other, code & 0xFF))
                        }
                    }
                    SlotAction::Macro(def) => {
                        // tex.web `check_outer_validity`（L7149-7170）——**唯一**判据是
                        // `scanner_status <> normal`：
                        //
                        //   begin if scanner_status<>normal then
                        //     begin deletions_allowed:=false;
                        //     @<Back up an outer control sequence so that it can be reread@>;
                        //     if scanner_status>skipping then
                        //       @<Tell the user what has run away and try to recover@>
                        //     else  begin print_err("Incomplete "); ... end;
                        //     deletions_allowed:=true;
                        //     end;
                        //   end;
                        //
                        // ⚠ 旧实现用「输入栈含 Macro 帧」当代理判据——**错的**：
                        // 栈含 Macro 帧 ≠ `scanner_status<>normal`。宏实参扫描用
                        // `get_token`（L7714 取 token 前临时置 `normal`），此时栈里
                        // 明明有 Macro 帧但 `scanner_status=normal`，pdfTeX 不报错。
                        // 实证：plain `^^L` 为 active char + `\outer\def^^L{\par}`，
                        // expl3-code.tex L9320 `\char_set_catcode_active:N \^^L`
                        // 取实参即触发误报 → `Extra \or` 224 条（43.8%）。
                        // （2026-09-13 再定性：该误报的真正机制是 active char 槽
                        // 与同名单字符 cs 槽共享——见 `MacroDef::active_slot`，
                        // 判据改为「token 形式 ↔ 槽形式一致」。）
                        if def.outer
                            && (tok.is_active() == def.active_slot)
                            && self.scanner_status != ScannerStatus::Normal
                        {
                            // active char token 不报（tex.web：active char 的
                            // cur_cmd=active 走 active 臂；plain \outer\def^^L
                            // 的槽残留 outer 不影响 ^^L active 使用——expl3
                            // L9320-9321 依赖此语义重定义 ^^L）
                            if std::env::var_os("NTEX_OUTER_SITE").is_some() {
                                eprintln!(
                                    "[OUTER-EXPAND] cs={} status={:?}",
                                    self.intern.name(csid),
                                    self.scanner_status
                                );
                            }
                            let name = match self.warning_index {
                                Some(cs) => self.cs_display_name(cs),
                                None => self.cs_display_name(csid),
                            };
                            // `scanner_status > skipping` → Runaway/Forbidden 恢复分支；
                            // `= skipping` → Incomplete \if 分支（L7157-7166）。
                            if self.scanner_status > ScannerStatus::Skipping {
                                match self.scanner_status {
                                    ScannerStatus::Matching => {
                                        let _ = self.sink.write16(format!(
                                            "! Forbidden control sequence found while scanning use of {name}.\\n\\
                                             <inserted text> \\n                \\\\par \\n\\
                                             I suspect you have forgotten a `}}', causing me\\n\\
                                             to read past where you wanted me to stop.\\n\\
                                             I'll try to recover; but if the error is serious,\\n\\
                                             you'd better type `E' or `X' now and fix your file.\\n"
                                        ));
                                    }
                                    ScannerStatus::Absorbing => {
                                        let _ = self.sink.write16(format!(
                                            "! Forbidden control sequence found while scanning text of {name}.\\n\\
                                             <inserted text> \\n                }} \\n\\
                                             I suspect you have forgotten a `}}', causing me\\n\\
                                             to read past where you wanted me to stop.\\n\\
                                             I'll try to recover; but if the error is serious,\\n\\
                                             you'd better type `E' or `X' now and fix your file.\\n"
                                        ));
                                    }
                                    _ => {
                                        // Defining：tex.web L6617 case 只有
                                        // defining/alignment 的措辞，其余同 Forbidden。
                                        let _ = self.sink.write16(format!(
                                            "! Forbidden control sequence found while scanning definition of {name}.\\n"
                                        ));
                                    }
                                }
                            } else {
                                let _ = self.sink.write16(
                                    "! Incomplete \\\\if; all text was ignored after line.\\n\\
                                     A forbidden control sequence occurred in skipped text.\\n"
                                        .to_string(),
                                );
                            }
                            return Ok(());
                        }
                        // e-TeX（M4-5）：protected 宏在展开抑制上下文（\edef/\write 等）
                        // 不展开，原样输出。
                        if def.protected && self.suppress_expansion > 0 {
                            self.sink.token(Token::control_sequence(csid))?;
                            return Ok(());
                        }
                        self.call_macro(csid, def)
                    }
                    SlotAction::Font(font) => {
                        // tex.web：字体 cs 执行即 `define(cur_font_loc,…)` +
                        // 推 sink（排版器靠该事件切换当前字体）。cur_font 是
                        // eqtb 字，**随组保存/恢复**（组内换字体只在本组生效；
                        // em/ex 内部单位读它，泄漏会让组外 `\kern-.36em` 等
                        // 用错字体算——`\LaTeX` 徽标现场）。
                        if !self.is_global() && self.group_level > 0 {
                            self.save_stack.push((
                                self.group_level,
                                SavedValue::CurFont {
                                    prev: self.cur_font,
                                },
                            ));
                        }
                        self.cur_font = font;
                        self.sink.font_selected(font)
                    }
                    SlotAction::Primitive(p) => {
                        // 诊断（NTEX_TRACE_EXEC=1）：打印每个执行的原语，定位挂死点
                        if diag_enabled("NTEX_TRACE_EXEC") {
                            eprintln!("[trace-exec] {p:?}");
                        }
                        if diag_enabled("NTEX_TRACE_STACK") {
                            eprintln!(
                                "[trace-stack] {p:?} last_tok={:?} stack={}",
                                self.last_tok,
                                self.debug_stack_summary()
                            );
                        }
                        self.exec_primitive(p)
                    }
                }
            }
            TokenKind::Char => {
                // 数学移位（$，cat 3）：peek 下一个 token 判定 `$$`（显示数学），
                // 交给 sink 按自身模式决定进出（M4-1）。
                if tok.catcode() == Some(Catcode::MathShift) {
                    // tex.web init_math 的进出裁决在 **mode**（mmode=收），
                    // 不在旗标——核心 in_math 在 `\[\halign{…$x$…}\]` 一类
                    // 结构里与排版层脱钩（显示数学开过即恒 true，单元首 `$`
                    // 被误判为收，everymath 注入错位到第二个 `$` 之后）。
                    let entering = !self.sink.math_shift_will_close();
                    // tex.web init_math：`$$` 进显示数学要求 `mode>0`（垂直/
                    // 普通水平）；受限水平（\hbox/\halign 模板，mode<0）下
                    // peek 到的第二个 `$` 必须 back_input——本 `$` 单独进普通
                    // 数学，第二个 `$` 随后在数学模式下作为独立 token 一记
                    // 闭合（TRIP L210 `\hbox{$$}$`、L340 单元内 `$$`：参考日志
                    // 两行 `{math mode: math shift character $}` + restoring，
                    // 且数学必须关上——否则对齐/数学组悬挂到文档尾致命）。
                    // 数学内（entering=false）是**闭合**语义，与进入侧不对称：
                    // tex.web mmode+math_shift → after_math——显示数学
                    // （mode=+mmode）收尾必 "Check that another $ follows"
                    // （吃掉配对 `$`，否则报 "Display math should end with $$"
                    // 照收）；行内数学（mode=-mmode）走 Finish math in text，
                    // **不 peek**——紧随的 `$` 落回水平模式由 init_math 重新
                    // 判定（`$x$$y$` = `$x$`+`$y$`，real TeX 如此）。360c342
                    // 把闭合侧一律拦成"只 peek 不消费"，`$$x$$` 的第二个 `$`
                    // 被 back_input 后在非数学态重开一个永不闭合的行内公式 →
                    // "数学模式未闭合"。
                    let consume_for_display = if entering {
                        self.sink.math_display_allowed()
                    } else {
                        self.sink.math_close_consumes_dollar()
                    };
                    let display = self.next_is_math_shift(consume_for_display)?;
                    self.in_math = entering;
                    // tex.web init_math：`$` 即 `new_save_level(math_shift_group)`
                    // ——数学层是 save group，先入作用域栈，`\everymath` 体里的
                    // `\aftergroup` 才挂到本层（而非外包盒子/对齐组）。
                    if entering {
                        self.scope_stack.push(true);
                    }
                    // TRIP 冲刺：进入数学模式时注入 `\everymath`（TeX `$` 处理语义）。
                    //
                    // LaTeX fmt 兼容：发行快照可能来自修复前引擎，`\let\frozen@everymath
                    // \everymath` 没有保住 primitive toks 参数，导致 LaTeX 写入 primitive
                    // `\everymath` 的 `\check@mathfonts` 钩子为空。若当前快照已有
                    // LaTeX 的 `\check@mathfonts` 且 primitive everymath 为空，补注入该
                    // 钩子，避免 NFSS 数学尺寸宏 `\tf@size/\sf@size/\ssf@size` 未初始化。
                    if entering {
                        // tex.web init_math（L21721 普通数学 / L21773 显示数学两路）：
                        // 进入公式即 eq_word_define(cur_fam, -1)——每个公式开头
                        // `\fam` 都是 -1（无字族替换），显式 `\fam<n>` 只在公式内生效
                        self.params.misc[46] = -1;
                        let mut items = self
                            .everymath
                            .iter()
                            .map(|t| (*t, false))
                            .collect::<Vec<_>>();
                        if items.is_empty() {
                            if let Some(csid) = self.intern.lookup("check@mathfonts") {
                                if !matches!(self.eqtb.slot(csid), EqSlot::Undefined) {
                                    items.push((Token::control_sequence(csid), false));
                                }
                            }
                        }
                        if !items.is_empty() {
                            self.push_frame(InputFrame::TokenList {
                                items: Arc::from(items),
                                pos: 0,
                            });
                        }
                    }
                    if entering {
                        return self.sink.math_shift(display);
                    }
                    // tex.web after_math → unsave：数学作用域先关，其
                    // `\aftergroup` token 随后在外层模式继续处理。
                    let r = self.sink.math_shift(display);
                    self.close_math_scope();
                    return r;
                }
                // 数学模式普通字符（letter/other，cat 11/12）→ 查 \mathcode 表
                // 改道为完整数学字符原子（tex.web 主控制数学分支：普通字符=
                // 隐式 mathcode 查表；class<<12 | fam<<8 | char）。demo 差异 #1：
                // $E=mc^2$ 字母默认 fam1（cmmi 斜体，initex 表 0x7100+码），
                // 此前布局侧硬编码 fam0（cmr 正体）。`^`/`_`（cat 7/8）、空格
                // （cat 10）、组定界（cat 1/2）不走此路。
                if self.in_math
                    && self.sink.math_code_applies()
                    && matches!(tok.catcode(), Some(Catcode::Letter | Catcode::Other))
                {
                    let ch = tok.charcode().unwrap_or(0);
                    let mut code = self.mathcodes.get(&ch).copied().unwrap_or(0x8000);
                    // tex.web scan_math（L21906-21908）：class ≥ var_code(0x7000)
                    // 且 cur_fam ∈ 0..15 时 **无条件替换** fam 字段——`\rm`/`\it`
                    // （plain `\def\rm{\fam\z@\tenrm}`）正是靠这一条把字母改道到
                    // 正体/斜体字族。数学进入时 cur_fam 复位 -1（L21721），故只在
                    // 显式 `\fam<n>` 后生效；\mathcode 自带的 fam 字段被抹掉。
                    if code >= 0x7000 {
                        code = self.mathcode_apply_fam(code);
                    }
                    return self.sink.math_char_full(code);
                }
                if std::env::var_os("NTEX_HOOK_TRACE").is_some() {
                    if let Some(ch) = tok.charcode().and_then(char::from_u32) {
                        if matches!(ch, 'p' | 'a' | 'r' | '/' | ',' | '0') {
                            let calls = self
                                .macro_trace
                                .as_ref()
                                .map(|tr| {
                                    tr.iter()
                                        .rev()
                                        .take(24)
                                        .rev()
                                        .map(|id| format!("\\{}", self.intern.name(*id)))
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                })
                                .unwrap_or_default();
                            eprintln!(
                                "[hook-char] ch={ch:?} line={} last={:?} stack={} calls={}",
                                self.error_line_no(),
                                self.last_tok,
                                self.debug_stack_summary(),
                                calls
                            );
                        }
                    }
                }
                // 组定界符（cat 1/2）在主流层建立/结束组（M1-11）。
                // 对齐上下文（`\halign`/`\valign`）的 `{`/`}` 平衡计数、
                // `\noalign` 组与对齐组配对已在 align_on_token（align_body_step）
                // 处理；此处只建立/结束普通组。
                match tok.catcode() {
                    Some(Catcode::BeginGroup) => self.begin_group(),
                    Some(Catcode::EndGroup) => {
                        // `}` 字符闭合：报错文本区分标志（见 end_group）
                        self.cur_group_close_via_primitive = false;
                        self.end_group()
                    }
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
        // TeX expand() 语义：条件原语在展开上下文（\edef/\write/\message 参数
        // 收集）中**求值**（tex.web expand 的 if_test/if_case 分支）——`\ifcase`
        // 在 \edef 里执行、\x 收集的是选中分支文本（etrip \5 宏
        // `\edef\6{\ifcase\lastnodetype...}`：\6 = else 分支 "empty"）。
        // 此前原样保留导致 \6 含未求值条件、\typeout 展开时再遇 expand_only
        // 仍不执行 → 输出空（ETRIP L351 `last node type (l.351): ` 缺 empty）。
        if let Some(op) = self.cond_op(tok) {
            return self.step_conditional(op, tok);
        }
        // 条件跳过区（\\ifcase-1 的 \\or 段等）：token 丢弃不收集
        // （TeX expand 的 pass_text 语义；主循环 process_one 同样先查 is_skipping）
        if self.is_skipping() {
            return Ok(());
        }
        match tok.kind() {
            TokenKind::ControlSeq => {
                let csid = tok.csid().expect("ControlSeq 必有 csid");
                match self.eqtb.slot(csid).clone() {
                    EqSlot::Macro(m) => {
                        if m.value.protected && self.suppress_expansion > 0 {
                            return self.sink.token(tok);
                        }
                        // tex.web `@<Tell the user what has run away...@>`（L7184-7200）：
                        // `\edef` 展开上下文里的 outer 宏同样是**可恢复错误**
                        // （`error`，非 fatal）——`scanner_status=absorbing` 分支
                        // （L7219-7222）打印 `... while scanning text of \X` +
                        // **插入 `}`** 后继续。
                        //
                        // ⚠ 实测（2026-09-13 再定性）：`plain` 的 `^^L` 是
                        // `\outer\def^^L{\par}`（写进 tex.web active 槽），cs 形式
                        // `\^^L` 的槽是 undefined——expl3 L9320 靠这一点不报。
                        // 判据按「token 形式 ↔ 槽形式一致」（`is_outer_for_token`）。
                        if self.is_outer_for_token(tok) {
                            let name = self.intern.name(csid).to_owned();
                            let _ = self.sink.write16(format!(
                                "! Forbidden control sequence found while scanning text of \\{name}.\\n\\
                                 <inserted text> \\n                }} \\n\\
                                 I suspect you have forgotten a `}}', causing me\\n\\
                                 to read past where you wanted me to stop.\\n\\
                                 I'll try to recover; but if the error is serious,\\n\\
                                 you'd better type `E' or `X' now and fix your file.\\n"
                            ));
                            // 恢复：按空展开继续（不中断作业）
                            return Ok(());
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
        // tex.web macro_call（L7968-7975）：
        //   save_scanner_status:=scanner_status; save_warning_index:=warning_index;
        //   warning_index:=cur_cs; ...
        //   @<Scan the parameters...@> = begin scanner_status:=matching; unbalance:=0; ...
        //   exit: scanner_status:=save_scanner_status; warning_index:=save_warning_index;
        // 即**扫实参期间** `scanner_status=matching`——outer 宏此刻才被禁。
        //
        // ⚠ 实参扫描本身用 `get_token`，且 `get_token` 内部（L7714）会临时
        // `save_scanner_status; scanner_status:=normal; get_token; 恢复`——
        // 故「取单个 token」不触发 outer 检查；只有扫描过程里的**宏展开**才看到
        // `matching`。这正是 pdfTeX 对 `\csca:N \^^L`（`` `#1 `` 取实参）不报错
        // 的原因（外层 normal 未被 brief `get_token` 时期的恢复覆盖）。
        let save_status = self.scanner_status;
        let save_warning = self.warning_index;
        let save_arg_recovery = self.arg_scan_recovered;
        self.warning_index = Some(csid);
        self.scanner_status = ScannerStatus::Matching;
        self.arg_scan_recovered = false;
        let r = self.call_macro_inner(csid, def);
        self.scanner_status = save_status;
        self.warning_index = save_warning;
        self.arg_scan_recovered = save_arg_recovery;
        r
    }

    /// `call_macro` 主体（`scanner_status` 由调用方设/恢复）。
    fn call_macro_inner(&mut self, csid: u32, def: Arc<MacroDef>) -> Result<()> {
        // 诊断轨迹（`NTEX_CALL_TRACE`）：在守卫**之前**记录，溢出那一次调用
        // 也要在内——「谁调用了爆栈的宏」的答案就在它前面几条。
        if let Some(t) = self.macro_trace.as_mut() {
            if t.len() >= MACRO_TRACE_CAP {
                t.pop_front();
            }
            t.push_back(csid);
        }
        self.diag_trace(format!(
            "CALL \\{} depth={}",
            self.intern.name(csid),
            self.stack.len()
        ));
        // TeX 输入栈上限（tex.web `stack_size`；TeX Live 取 5000）：宏递归展开
        // 无终止条件时以此报错终止，而非耗尽内存。此前无此保护——latex.ltx 加载
        // 曾触发单步内无界递归（每层压一个 Bytecode 帧，主循环 10M 步上限够不到），
        // RSS 涨至 OOM（docs/archive/latex-feasibility.md A4）。真实 TeX 同为
        // "TeX capacity exceeded, sorry [input stack size=N]" 致命错。
        if self.stack.len() >= MAX_INPUT_STACK {
            let name = self.intern.name(csid).to_owned();
            let _ = self.sink.write16(format!(
                "TeX capacity exceeded, sorry [input stack size = {MAX_INPUT_STACK}].\n"
            ));
            self.report_error_context();
            // 现场转储必须紧邻致命报错做掉：宏/字节码递归爆栈的现场只在栈帧里
            // （此类循环中报错行号是失真的——见 2026-09-11/09-13 两次排查结论）。
            self.dump_input_stack("call-macro");
            return Err(Error::invalid_input(format!(
                "输入栈超限（{MAX_INPUT_STACK} 帧）——宏 \\{name} 递归展开疑似无终止条件"
            )));
        }
        // 0 参数宏也走 collect_args：参数文本可能是**纯定界串**（`\def\X\fi:\use:n{...}`），
        // 调用点须匹配并吞掉（tex.web macro_call `if info(r)<>end_match_token`）——
        // 见 collect_args n==0 臂注释。
        let args = self.collect_args(csid, &def)?;
        if self.arg_scan_recovered {
            // tex.web macro_call 的 `long_state=outer_call`/Paragraph-ended 恢复
            // 直接跳到调用结束：宏体不能在残缺实参上继续展开，否则会再次读取
            // 已回推的恢复材料而重放同一残流。
            return Ok(());
        }
        // M2 双轨：字节码优先（未编译则回退解释器轨道）
        if self.use_bytecode {
            if let Some(code) = &def.code {
                self.push_frame(InputFrame::Bytecode {
                    code: code.clone(),
                    pc: 0,
                    args,
                });
                return Ok(());
            }
        }
        self.push_frame(InputFrame::Macro {
            body: def.body.clone(),
            pos: 0,
            args,
        });
        Ok(())
    }

    // ---------- 输入获取 ----------

    /// 探测下一个 token 是否为数学移位（`$$` 检测）：
    /// `consume_for_display`（tex.web `mode>0`，由调用方按 sink 模式判定）为
    /// true 时——是 → 消费该 `$`（显示数学成立）；否则（含受限水平下 peek
    /// 到 `$`）→ 放回（tex.web init_math `back_input`），返回 false。
    /// 非 `$` 一律放回。输入耗尽返回 false。
    ///
    fn next_is_math_shift(&mut self, consume_for_display: bool) -> Result<bool> {
        let Some((tok, ne)) = self.fetch()? else {
            return Ok(false);
        };
        let is = tok.catcode() == Some(Catcode::MathShift);
        if is && consume_for_display {
            Ok(true)
        } else {
            self.push_frame(InputFrame::One { tok, noexpand: ne });
            Ok(false)
        }
    }

    /// 输入栈转储（诊断开关）。
    ///
    /// 触发变量：
    /// - `NTEX_STACK_DUMP`：帧类型直方图 + 栈顶帧签名重复度 + 逐帧内容；
    /// - `NTEX_STACK_DUMP_FRAMES`：仅逐帧内容（单独设置亦生效）；
    /// - `NTEX_STACK_DUMP_TOP=N`：栈顶逐帧转储条数（默认 60）；
    /// - `NTEX_STACK_DUMP_BOTTOM=N`：栈底逐帧转储条数（默认 5；调大可看
    ///   「循环墙从哪一帧开始」——墙的起点在栈底侧）；
    /// - `NTEX_CALL_TRACE=N`：宏调用环形轨迹（N = 显示段数，默认 120；
    ///   未设 = 零开销，见 [`MACRO_TRACE_CAP`]）。
    ///
    /// **两处守卫共用本入口的原因**（2026-09-13 修正）：宏帧守卫
    /// [`Self::call_macro_inner`] 用 `>= MAX_INPUT_STACK`，比 fetch 的统一兜底
    /// （`>`）**先命中**——此前转储只挂在 fetch 上，于是「宏/字节码递归不终止」
    /// 这类最需要现场的爆栈（expl3 载入 l.27200：`\q_stop 递归展开疑似无终止
    /// 条件`）反而永远拿不到转储。两侧改为调用同一函数。
    ///
    /// 转储三件套的分工：类型直方图回答「哪类帧多」；**帧签名重复度**回答
    /// 「哪一族帧在自复制」（循环主体的直接证据）；逐帧详解给出 token 级现场。
    fn dump_input_stack(&mut self, reason: &str) {
        let kinds_on = std::env::var_os("NTEX_STACK_DUMP").is_some();
        let frames_on = std::env::var_os("NTEX_STACK_DUMP_FRAMES").is_some();
        if !kinds_on && !frames_on {
            return;
        }
        let n = self.stack.len();
        let top: usize = std::env::var("NTEX_STACK_DUMP_TOP")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(60);
        // 栈底条数：定位「循环从哪一帧开始」要的是**墙的起点**（栈底侧），
        // 只看栈顶一堆同样的帧反而不知道是谁把第一个 `\q_stop` 打进流里的。
        let bottom: usize = std::env::var("NTEX_STACK_DUMP_BOTTOM")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(5);
        if kinds_on {
            let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
            for f in &self.stack {
                *kinds.entry(Self::frame_kind(f)).or_default() += 1;
            }
            let _ = self.sink.write16(format!(
                "[stack-dump] reason={reason} depth={n} kinds={kinds:?}\n"
            ));
            // 栈顶 top 帧的签名重复度：循环主体必在此处高频自复制。
            let sig_n = top.min(n);
            let mut sigs: BTreeMap<String, usize> = BTreeMap::new();
            for f in &self.stack[n - sig_n..] {
                *sigs.entry(self.frame_marker(f)).or_default() += 1;
            }
            let mut ranked: Vec<(String, usize)> = sigs.into_iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let _ = self.sink.write16(format!(
                "[stack-dump] 栈顶 {sig_n} 帧签名重复度（top 8）：\n"
            ));
            for (sig, cnt) in ranked.iter().take(8) {
                let _ = self
                    .sink
                    .write16(format!("[stack-dump]   {cnt:>5} × {sig}\n"));
            }
            // 宏调用轨迹：栈上没有的「已弹出入口帧」只能从这里看出来。
            // 连续同名合并（`\q_stop×4993`），否则表格被同一个名字淹掉。
            if let Some(tr) = &self.macro_trace {
                let mut runs: Vec<(u32, usize)> = Vec::new();
                for &id in tr.iter() {
                    match runs.last_mut() {
                        Some((prev, cnt)) if *prev == id => *cnt += 1,
                        _ => runs.push((id, 1)),
                    }
                }
                let intern = &self.intern;
                let rendered: Vec<String> = runs
                    .iter()
                    .map(|(id, cnt)| Self::fmt_call_run(intern, *id, *cnt))
                    .collect();
                let show: usize = std::env::var("NTEX_CALL_TRACE")
                    .ok()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(120)
                    .max(1);
                let start = rendered.len().saturating_sub(show);
                let _ = self.sink.write16(format!(
                    "[stack-dump] 宏调用轨迹（共 {} 次，旧→新，显示末 {show} 段）：\n[stack-dump]   {}\n",
                    tr.len(),
                    rendered[start..].join(" ")
                ));
            }
        }
        if frames_on {
            let (plan, omitted) = Self::frame_dump_plan(n, top, bottom);
            for slot in plan {
                match slot {
                    Some(idx) => {
                        let _ = self.sink.write16(format!(
                            "[frame {:>5}] {}\n",
                            idx,
                            self.render_frame_head(&self.stack[idx])
                        ));
                    }
                    None => {
                        let _ = self
                            .sink
                            .write16(format!("[frame ...] （省略 {omitted} 帧）\n"));
                    }
                }
            }
        }
    }

    /// 逐帧转储计划：返回 `(帧序号槽位, 中间省略的帧数)`；`None` = 省略标记行。
    ///
    /// 槽位顺序即输出顺序：**栈顶 `top` 帧（降序）+ 省略 + 栈底 `bottom` 帧（升序）**。
    ///
    /// ⚠ 这里是 2026-09-13 修掉的一处**仪器 bug**：原实现取 `stack[..head]`
    /// （栈**底** head 帧）却按 `n-1-i` 标注成「栈顶帧」——**底/顶正好反了**。
    /// 宏递归每层都压在顶上，循环主体在栈顶；看栈底等于只看最外层作业帧，
    /// 必然误判（09-11 那次「4997 个 TokenList 帧」的结论其实是栈底视图）。
    /// 抽成纯函数是为了让这个方向性约定能被单测钉住。
    fn frame_dump_plan(n: usize, top: usize, bottom: usize) -> (Vec<Option<usize>>, usize) {
        let head = n.min(top);
        let tail = bottom.min(n - head);
        let omitted = n - head - tail;
        let mut plan = Vec::with_capacity(head + tail + 1);
        for idx in (n - head..n).rev() {
            plan.push(Some(idx));
        }
        if omitted > 0 {
            plan.push(None);
        }
        for idx in 0..tail {
            plan.push(Some(idx));
        }
        (plan, omitted)
    }

    /// 渲染一段连续同名调用：`\name` / `\name×n`（转储轨迹用）。
    fn fmt_call_run(intern: &InternTable, csid: u32, n: usize) -> String {
        let name = intern.name(csid);
        if n <= 1 {
            format!("\\{name}")
        } else {
            format!("\\{name}×{n}")
        }
    }

    /// 栈帧类型名（转储直方图用）。
    fn frame_kind(f: &InputFrame) -> &'static str {
        match f {
            InputFrame::Source { .. } => "Source",
            InputFrame::Macro { .. } => "Macro",
            InputFrame::Bytecode { .. } => "Bytecode",
            InputFrame::TokenList { .. } => "TokenList",
            InputFrame::MacroArg { .. } => "MacroArg",
            InputFrame::One { .. } => "One",
            InputFrame::OutputRoutine { .. } => "OutputRoutine",
            InputFrame::AlignU { .. } => "AlignU",
            InputFrame::AlignV { .. } => "AlignV",
        }
    }

    /// 帧签名（转储分组用）：**类型 + 头部 token，不含进度**——同一宏体的不同
    /// 进度应归为同一族，重复度才看得出「谁在自复制」。
    fn frame_marker(&self, f: &InputFrame) -> String {
        let (kind, desc) = match f {
            InputFrame::Source { .. } => ("Source", String::new()),
            InputFrame::Macro { body, .. } => ("Macro", self.render_token_head(body, 8)),
            InputFrame::Bytecode { code, .. } => ("Bytecode", self.render_bytecode_head(code, 8)),
            InputFrame::TokenList { items, .. } => ("TokenList", self.render_pairs_head(items, 8)),
            InputFrame::MacroArg { items, .. } => ("MacroArg", self.render_pairs_head(items, 8)),
            InputFrame::One { tok, .. } => ("One", self.render_token(*tok)),
            InputFrame::OutputRoutine { items, .. } => {
                ("OutputRoutine", self.render_pairs_head(items, 8))
            }
            InputFrame::AlignU { items, .. } => ("AlignU", self.render_token_head(items, 8)),
            InputFrame::AlignV { items, .. } => ("AlignV", self.render_token_head(items, 8)),
        };
        if desc.is_empty() {
            kind.to_owned()
        } else {
            format!("{kind}[{desc}]")
        }
    }

    /// 逐帧详解（转储用）：类型 + 进度 + 头部内容。
    ///
    /// 帧内容**从头**显示（不看 pos）——已消费帧的内容同样是「谁在循环展开」
    /// 的证据。此前实现漏了 [`InputFrame::Macro`]/`MacroArg`/`AlignU`/`AlignV`/
    /// `OutputRoutine` 五个帧型（落进 `Other`），而宏递归爆栈现场恰恰全是
    /// Macro 帧，等于转储在最需要的地方失明。
    fn render_frame_head(&self, f: &InputFrame) -> String {
        match f {
            InputFrame::Source { pos, bytes, .. } => format!("Source[pos={pos}/{}]", bytes.len()),
            InputFrame::Macro { body, pos, args } => format!(
                "{} args={}",
                self.render_seq_frame("Macro", body, *pos),
                args.len()
            ),
            InputFrame::Bytecode { code, pc, args } => format!(
                "Bytecode[pc={pc}/{} args={}] head={} at={}",
                code.len(),
                args.len(),
                self.render_bytecode_span(code, 0, 8),
                self.render_bytecode_span(code, *pc, 8)
            ),
            InputFrame::TokenList { items, pos } => {
                self.render_pair_frame("TokenList", items, *pos)
            }
            InputFrame::MacroArg { items, pos } => self.render_pair_frame("MacroArg", items, *pos),
            InputFrame::One { tok, .. } => format!("One[{}]", self.render_token(*tok)),
            InputFrame::OutputRoutine { items, pos } => {
                self.render_pair_frame("OutputRoutine", items, *pos)
            }
            InputFrame::AlignU { items, pos } => self.render_seq_frame("AlignU", items, *pos),
            InputFrame::AlignV { items, pos } => self.render_seq_frame("AlignV", items, *pos),
        }
    }

    /// token 序列帧的通用渲染：`Kind[rem=r/total] head=… at=…`。
    ///
    /// `head` 用来看「这一帧是什么」（帧从头显示）；**`at` 才是当前现场**——
    /// 长宏体/长实参（本例 439 token 的 MacroArg）上只看 head 会完全失明：
    /// 正被展开的 token 在 pos 处，不在帧头。
    fn render_seq_frame(&self, kind: &str, items: &[Token], pos: usize) -> String {
        format!(
            "{kind}[rem={}/{}] head={} at={}",
            items.len().saturating_sub(pos),
            items.len(),
            self.render_token_span(items, 0, 6),
            self.render_token_span(items, pos, 14)
        )
    }

    /// `(token, noexpand)` 序列帧的通用渲染（TokenList/OutputRoutine）。
    fn render_pair_frame(&self, kind: &str, items: &[(Token, bool)], pos: usize) -> String {
        format!(
            "{kind}[rem={}/{}] head={} at={}",
            items.len().saturating_sub(pos),
            items.len(),
            self.render_pairs_span(items, 0, 6),
            self.render_pairs_span(items, pos, 14)
        )
    }

    /// 渲染 token 序列头部（转储用）；超出 `max` 的以 `…(+k)` 计数收尾。
    fn render_token_head(&self, items: &[Token], max: usize) -> String {
        self.render_token_span(items, 0, max)
    }

    /// 渲染 token 序列从 `from` 起的 `max` 个 token（转储用）。
    fn render_token_span(&self, items: &[Token], from: usize, max: usize) -> String {
        let from = from.min(items.len());
        let rest = items.len() - from;
        let take = rest.min(max);
        let mut parts: Vec<String> = Vec::with_capacity(take + 1);
        for t in &items[from..from + take] {
            parts.push(self.render_token(*t));
        }
        if rest > take {
            parts.push(format!("…(+{})", rest - take));
        }
        parts.join(" ")
    }

    /// 渲染 `(token, noexpand)` 序列头部（TokenList/OutputRoutine 帧转储用）。
    /// `noexpand` 标记以 `~` 前缀表示（`\noexpand` 冻结的 token）。
    fn render_pairs_head(&self, items: &[(Token, bool)], max: usize) -> String {
        self.render_pairs_span(items, 0, max)
    }

    /// 渲染 `(token, noexpand)` 序列从 `from` 起的 `max` 个元素。
    fn render_pairs_span(&self, items: &[(Token, bool)], from: usize, max: usize) -> String {
        let from = from.min(items.len());
        let rest = items.len() - from;
        let take = rest.min(max);
        let mut parts: Vec<String> = Vec::with_capacity(take + 1);
        for (t, ne) in &items[from..from + take] {
            let s = self.render_token(*t);
            parts.push(if *ne { format!("~{s}") } else { s });
        }
        if rest > take {
            parts.push(format!("…(+{})", rest - take));
        }
        parts.join(" ")
    }

    /// 渲染字节码帧头部（转储用）：token 原值按 `\name`，`#n` 实参，`end` 结束。
    fn render_bytecode_head(&self, code: &Bytecode, max: usize) -> String {
        self.render_bytecode_span(code, 0, max)
    }

    /// 渲染字节码从第 `from` 个字起的 `max` 条指令（转储用；`from` 通常取 pc，
    /// 因为待执行指令从 pc 开始，头部已执行部分不是现场）。
    fn render_bytecode_span(&self, code: &Bytecode, from: usize, max: usize) -> String {
        let words = code.words();
        let from = from.min(words.len());
        let rest = words.len() - from;
        let take = rest.min(max);
        let mut parts: Vec<String> = Vec::with_capacity(take + 1);
        for &w in &words[from..from + take] {
            parts.push(match Instruction::decode(w) {
                Instruction::Emit { token } => self.render_token(token),
                Instruction::EmitArg { n } => format!("#{n}"),
                Instruction::End => "end".to_owned(),
            });
        }
        if rest > take {
            parts.push(format!("…(+{})", rest - take));
        }
        parts.join(" ")
    }

    /// 字节码护栏转储：最多三个活动字节码帧，逐帧列出 pc 邻域反汇编与近期 cs 环。
    fn dump_bytecode_guard(&self, dispatches: u64, limit: u64) {
        eprintln!("[bc-guard] dispatch 超限：连续 {dispatches} 次未返回 token（limit={limit}）");
        for (ordinal, (code, pc)) in self
            .stack
            .iter()
            .rev()
            .filter_map(|frame| match frame {
                InputFrame::Bytecode { code, pc, .. } => Some((code, *pc)),
                _ => None,
            })
            .take(3)
            .enumerate()
        {
            let start = pc.saturating_sub(8);
            let end = (pc + 9).min(code.len());
            let disasm = (start..end)
                .map(|at| {
                    let marker = if at == pc { '>' } else { ' ' };
                    let word = code.words()[at];
                    let text = match Instruction::decode(word) {
                        Instruction::Emit { token } => self.render_token(token),
                        Instruction::EmitArg { n } => format!("#{n}"),
                        Instruction::End => "end".to_owned(),
                    };
                    format!("{marker}{at:>4}: {text} [{word:#018x}]")
                })
                .collect::<Vec<_>>()
                .join("; ");
            eprintln!(
                "[bc-guard] frame#{ordinal} pc={pc}/{}: {disasm}",
                code.len()
            );
        }
        let recent = self
            .bc_guard_trace
            .iter()
            .map(|&csid| format!("\\{}", self.intern.name(csid)))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("[bc-guard] recent-cs: {recent}");
        if let Some(trace) = &self.handler_trace {
            let recent = trace
                .iter()
                .map(|&(csid, floor)| format!("\\{}(floor={floor})", self.intern.name(csid)))
                .collect::<Vec<_>>()
                .join(" ");
            eprintln!("[bc-guard] handler-trace: {recent}");
        }
        eprintln!("[bc-guard] stack: {}", self.debug_stack_summary());
    }

    /// 单个 token 的可读渲染（转储用）：`\name` / 可打印字符 / `#n`；
    /// 空白显示为 `␣`，其余控制字符显示为 `^XX`。
    fn render_token(&self, t: Token) -> String {
        match t.kind() {
            TokenKind::ControlSeq => match t.csid() {
                Some(id) => format!("\\{}", self.intern.name(id)),
                None => "\\?".to_owned(),
            },
            TokenKind::MacroParam => match t.param_number() {
                Some(n) => format!("#{n}"),
                None => "#?".to_owned(),
            },
            TokenKind::EndGroup => "}".to_owned(),
            TokenKind::EndTemplate => String::new(),
            TokenKind::Char => match t.charcode() {
                Some(c) => {
                    let ch = char::from_u32(c).unwrap_or('\u{FFFD}');
                    if ch == ' ' {
                        "␣".to_owned()
                    } else if ch.is_ascii_graphic() {
                        ch.to_string()
                    } else {
                        format!("^{:02X}", c & 0x7F)
                    }
                }
                None => "?".to_owned(),
            },
        }
    }

    /// 取下一个 token；返回 `(token, noexpand)`。输入耗尽或越过读取下限返回 None。
    fn fetch(&mut self) -> Result<Option<(Token, bool)>> {
        let guard_limit = bytecode_guard_limit();
        let mut dispatches = 0u64;
        loop {
            if guard_limit != 0 {
                dispatches += 1;
                if self.bc_guard_active {
                    self.bc_guard_dispatches += 1;
                    dispatches = self.bc_guard_dispatches;
                }
                if dispatches > guard_limit {
                    self.dump_bytecode_guard(dispatches, guard_limit);
                    return Err(Error::invalid_input(format!(
                        "字节码 dispatch 超限（{dispatches} 次未返回 token；NTEX_BC_GUARD={guard_limit}）"
                    )));
                }
            }
            // 输入栈深度兜底上限（tex.web `stack_size` 语义）：[`Self::call_macro`]
            // 只盖宏帧，TokenList/Source 等帧的循环注入同样能把栈撑爆——此处统一
            // 兜底（每次压帧后必经 fetch，故为全帧型的唯一收口点）。
            if self.stack.len() > MAX_INPUT_STACK {
                self.dump_input_stack("fetch");
                return Err(Error::invalid_input(format!(
                    "输入栈超限（{} 帧 > {MAX_INPUT_STACK}）——展开/参数扫描疑似无终止条件",
                    self.stack.len()
                )));
            }
            // 不允许读取位于子展开边界（read_floor）以下的帧
            if self.stack.len() <= self.read_floor {
                return Ok(None);
            }
            let Some(frame) = self.stack.last_mut() else {
                return Ok(None);
            };
            match frame {
                InputFrame::Source {
                    bytes,
                    pos,
                    state,
                    eof_mark,
                    ..
                } => {
                    // `\endinput` force_eof（tex.web）：扫描越过截断点 =
                    // 文件提前结束。弹帧 + 注入 `\everyeof`（pdfTeX every_eof
                    // 扩展；expl3 \__sys_get 依赖）。
                    if let Some(mark) = *eof_mark {
                        if *pos >= mark {
                            self.stack.pop();
                            let toks = std::mem::take(&mut self.everyeof_toks);
                            if !toks.is_empty() {
                                let seq: Vec<(Token, bool)> =
                                    toks.into_iter().map(|t| (t, false)).collect();
                                self.push_frame(InputFrame::TokenList {
                                    items: Arc::from(seq),
                                    pos: 0,
                                });
                            }
                            continue;
                        }
                    }
                    match scan_token(
                        bytes,
                        pos,
                        &self.catcodes,
                        &mut self.intern,
                        state,
                        // M9 中文刀 2：\utfinputmode≠0 → 源码按 UTF-8 解码
                        //（只作用于字节→token 入口；宏体/实参 token 流不受影响，
                        // TRIP/ETRIP 默认 bytes 模式零影响）
                        self.params.misc[crate::param::MISC_UTF_INPUT_MODE] != 0,
                        // M9 中文刀 7：\endlinechar 动态传入（beamer 逐行消费器
                        // 的 #1^^M 定界依赖行尾 token 字符码/猫码随参数变化）
                        self.params.endlinechar,
                    ) {
                        Ok(Some(tok)) => return Ok(Some((tok, false))),
                        Ok(None) => {
                            self.stack.pop();
                            // 自然读尽同样注入 `\everyeof`（pdfTeX every_eof
                            // 语义：文件结束边界统一注入，无论正常 EOF 还是
                            // \endinput 截断；expl3 \__sys_get 依赖）。
                            let toks = std::mem::take(&mut self.everyeof_toks);
                            if !toks.is_empty() {
                                let seq: Vec<(Token, bool)> =
                                    toks.into_iter().map(|t| (t, false)).collect();
                                self.push_frame(InputFrame::TokenList {
                                    items: Arc::from(seq),
                                    pos: 0,
                                });
                            }
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
                        self.push_frame(InputFrame::MacroArg { items: arg, pos: 0 });
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
                            self.push_frame(InputFrame::MacroArg { items: arg, pos: 0 });
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
                InputFrame::MacroArg { items, pos } => {
                    if *pos >= items.len() {
                        self.stack.pop();
                        continue;
                    }
                    let (tok, noexpand) = items[*pos];
                    *pos += 1;
                    return Ok(Some((tok, noexpand)));
                }
                InputFrame::One { tok, noexpand } => {
                    // 先拷贝（结束字段借用）再弹帧（&mut stack）
                    let (t, ne) = (*tok, *noexpand);
                    self.stack.pop();
                    return Ok(Some((t, ne)));
                }
                InputFrame::OutputRoutine { items, pos } => {
                    if *pos >= items.len() {
                        // 例程帧耗尽：关隐式组（恢复例程内赋值），复位输出例程
                        // 激活标志（可再次注入）。剩余待处理页面是否丢弃由
                        // maybe_inject_output 按进度判断（例程未取用 box255 时），
                        // 不在帧弹出时处理——例程末 token 的参数扫描（如 \box255
                        // 数字）会 fetch 到帧外。
                        self.stack.pop();
                        self.end_group()?;
                        self.output_active = false;
                        continue;
                    }
                    let item = items[*pos];
                    *pos += 1;
                    return Ok(Some(item));
                }
                // M4-5 对齐模板帧耗尽 hook（tex.web end_token_list 的 u/v 分支）
                InputFrame::AlignU { items, pos } => {
                    if *pos >= items.len() {
                        self.stack.pop();
                        self.align_u_exhausted();
                        continue;
                    }
                    let tok = items[*pos];
                    *pos += 1;
                    return Ok(Some((tok, false)));
                }
                InputFrame::AlignV { items, pos } => {
                    if *pos >= items.len() {
                        self.stack.pop();
                        self.align_fin_col()?;
                        continue;
                    }
                    let tok = items[*pos];
                    *pos += 1;
                    return Ok(Some((tok, false)));
                }
            }
        }
    }

    /// 帧是否「已耗尽」：下一个 [`Self::fetch`] 对它的唯一动作就是 `pop`
    /// （tex.web `loc=null`，module 6638 的定义）。
    ///
    /// **只有游标型帧（`pos`/`pc` 推进式）参与判定**。`One` 虽小却是
    /// 「弹出并返回 token」型——它在栈上时 token **尚未被消费**，永远不算
    /// 耗尽（`fetch` 的 One 臂 `pop` 与 `return Ok(Some(...))` 同时发生）。
    fn frame_depleted(f: &InputFrame) -> bool {
        match f {
            InputFrame::Bytecode { code, pc, .. } => {
                // bytecode 体末尾恒有一个 `End` 指令占位，而 tex.web 的 token
                // 链表以 `null` 收尾——故「已耗尽」= 越过 `words()` 尾 **或**
                // 游标正指 `End` 终止字（与 fetch 的 `_ =>` 臂同判据）。
                *pc >= code.len()
                    || code.words()[*pc] >> crate::bytecode::TAG_SHIFT
                        > crate::bytecode::EMIT_ARG_TAG
            }
            InputFrame::Macro { body, pos, .. } => *pos >= body.len(),
            InputFrame::TokenList { items, pos } => *pos >= items.len(),
            InputFrame::MacroArg { items, pos } => *pos >= items.len(),
            // 以下帧型**不参与** drain，理由各不相同（对照 fetch 各臂）：
            // - `Source`：tex.web 的循环条件 `state=token_list` 同样排除文件层；
            //   其弹出还要注入 `\everyeof`（非纯 pop）；
            // - `One`：见上，在栈上即未消费；
            // - `OutputRoutine`：弹出带 `end_group` + 复位输出例程激活标志；
            // - `AlignU`/`AlignV`：弹出带 `align_u_exhausted`/`align_fin_col`
            //   钩子（tex.web 只排除 v_template，此处从保守侧一并对齐排除）。
            InputFrame::Source { .. }
            | InputFrame::One { .. }
            | InputFrame::OutputRoutine { .. }
            | InputFrame::AlignU { .. }
            | InputFrame::AlignV { .. } => false,
        }
    }

    /// 清掉栈顶**已耗尽**的纯帧（tex.web `end_token_list` 的 "conserve stack
    /// space" 步骤）。
    ///
    /// tex.web module 7978（`macro_call` 的 `@<Feed the macro body and its
    /// parameters to the scanner@>`）与 module 7025（`back_input`）在**压新层
    /// 之前**都执行同一循环，原文附注即点明其目的：
    ///
    /// ```text
    /// while (state=token_list)and(loc=null)and(token_type<>v_template) do
    ///   end_token_list; {conserve stack space}
    /// ```
    ///
    /// > "Then a user macro that ends with a call to itself will not require
    /// >  unbounded stack space."
    ///
    /// 缺此步骤时，**尾递归宏**（自调用是该帧最后一串 token）每轮都把已耗尽的
    /// 调用者帧留在栈上：[`Self::fetch`] 只在帧成为**栈顶**时才弹它，而下一轮的
    /// 帧立刻压在其上——调用者永远等不到再次成为栈顶。栈深随迭代数线性增长。
    ///
    /// [实测 2026-09-15] 这是 expl3 载入在 l.36005 撞墙（`TeX capacity exceeded
    /// [input stack size]`）的**根因**，且与 expl3 无关——纯 plain TeX 复现：
    ///
    /// | 用例 | NTex（修复前）| pdfTeX（GT）|
    /// |---|---|---|
    /// | `\def\step{...\read...\step}` 逐行读完 UnicodeData.txt（34k 行）| 输入栈超限 | 正常读完 |
    /// | `\int_step_inline:nn {5000}{\relax}`（expl3 已载入）| 输入栈超限 | 正常（20000 亦无事）|
    ///
    /// 全栈转储佐证：5001 帧中 **4932 帧**是 `\__ior_map_variable_loop:NNNn`
    /// （每读一行漏 1 帧）、35 帧是 `\__int_step:Nw`，其余为作业帧。
    ///
    /// `read_floor` 是子展开（`\edef`/`\expanded`/`\csname`）边界，其下的帧
    /// 属外层上下文，不得越界——与 [`Self::fetch`] 的 `stack.len() <= read_floor`
    /// 同口径（fetch 允许弹到恰好 `read_floor`，本循环同）。
    fn drain_depleted_frames(&mut self) {
        while self.stack.len() > self.read_floor {
            match self.stack.last() {
                Some(f) if Self::frame_depleted(f) => {
                    self.stack.pop();
                }
                _ => break,
            }
        }
    }

    /// 压入输入帧（全帧型统一入口；TEMP DEBUG 挂钩巨型 TokenList 定位）。
    ///
    /// 压栈前先跑 [`Self::drain_depleted_frames`]：tex.web 把该步骤放在
    /// `macro_call`/`back_input` 两处，这里收到唯一入口——不变量是「栈顶已耗尽
    /// 的纯帧对后续任何读取都不可见」，故在各调用点均等价（见该函数文档）。
    fn push_frame(&mut self, f: InputFrame) {
        self.drain_depleted_frames();
        #[cfg(not(target_arch = "wasm32"))]
        if diag_enabled("NTEX_BIGLIST_TRACE") {
            if let InputFrame::TokenList { items, .. } = &f {
                if items.len() > 300_000 {
                    let head: Vec<String> = items
                        .iter()
                        .take(40)
                        .map(|t| match t.0.csid() {
                            Some(id) => format!("\\{}", self.intern.name(id)),
                            None => format!("c{}", t.0.charcode().unwrap_or(9999)),
                        })
                        .collect();
                    eprintln!(
                        "[bigpush] {} tok last_tok={:?} head={:?}",
                        items.len(),
                        self.last_tok,
                        head
                    );
                }
            }
        }
        self.stack.push(f);
        // 结构化 trace（JSONL）：记录每次压帧——定位「栈为什么膨胀」的关键。
        if crate::expand::trace::enabled() {
            let (tok, frame) = match self.stack.last() {
                Some(fr) => {
                    let desc = match fr {
                        InputFrame::TokenList { items, .. } => {
                            format!("TokenList({}tok)", items.len())
                        }
                        InputFrame::MacroArg { items, .. } => {
                            format!("MacroArg({}tok)", items.len())
                        }
                        InputFrame::Macro { body, .. } => {
                            let name = body
                                .first()
                                .and_then(|t| t.csid())
                                .map(|id| self.intern.name(id).to_string())
                                .unwrap_or_default();
                            format!("Macro({},{}tok)", name, body.len())
                        }
                        InputFrame::One { .. } => "One".to_string(),
                        InputFrame::OutputRoutine { items, .. } => {
                            format!("OutputRoutine({}tok)", items.len())
                        }
                        InputFrame::Source { bytes, .. } => {
                            format!("Source({}B)", bytes.len())
                        }
                        InputFrame::Bytecode { .. } => "Bytecode".to_string(),
                        InputFrame::AlignU { items, .. } => {
                            format!("AlignU({}tok)", items.len())
                        }
                        InputFrame::AlignV { items, .. } => {
                            format!("AlignV({}tok)", items.len())
                        }
                    };
                    (self.last_tok.clone(), desc)
                }
                None => (None, String::new()),
            };
            crate::expand::trace::event(
                self.steps,
                crate::expand::trace::KIND_PUSH,
                tok.as_deref(),
                None,
                self.stack.len(),
                Some(&frame),
                None,
            );
        }
    }

    /// 把 token 放回输入流（等价于压入单元素 token 列表帧）。
    fn unread(&mut self, tok: Token) {
        self.push_frame(InputFrame::TokenList {
            items: Arc::from([(tok, false)]),
            pos: 0,
        });
    }
}

/// 段边界的"值状态"镜像（M5 增量，plan.md §7；`incremental::snapshot` 用）。
///
/// `.fmt` 快照（[`FmtState`]）覆盖宏定义/寄存器/参数主干，但**不含**若干运行时
/// 字段（`\everypar` 等 token 列表、`\mathcode`/`\lccode` 等编码表、`\parshape`、
/// e-TeX 惩罚数组、字体参数覆盖、读写流）。段级缓存的失效判定必须看到**全部**
/// 影响输出的状态，否则会把"缓存仍有效"判错（M5 风险项：副作用漏追踪 → 缓存错）。
///
/// 组成：可精确比较的部分（catcode/sfcode 表、`\output` 例程、寄存器版本戳）
/// + `runtime_digest` 指纹（其余状态；HashMap 逐项哈希后累加，与迭代顺序无关）。
#[derive(Debug, Clone, PartialEq)]
pub struct ValueState {
    /// catcode 表（`\catcode` 可改；影响后续所有 token 化）。
    pub catcodes: CatcodeTable,
    /// `\sfcode` 表。
    pub sfcodes: [u32; 256],
    /// `\output` 例程 token 列表（None = 未定义）。
    pub output_toks: Option<TokenArray>,
    /// 已写入寄存器槽的影子表（`Registers::dirty`；未写槽恒为零值 →
    /// 只比较此表即**精确**等于整份寄存器文件，且与引擎实例无关、可跨重建比较）。
    pub registers: BTreeMap<(u8, usize), RegisterValue>,
    /// 其余运行时状态指纹（`Expander::runtime_digest`）。
    pub runtime_digest: u64,
}

/// FNV-1a：混入单字节。
fn digest_byte(h: &mut u64, b: u8) {
    *h = (*h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
}

/// FNV-1a：混入字节序列。
fn digest_bytes(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        digest_byte(h, b);
    }
}

/// FNV-1a：混入一个 u64（小端逐字节）。
fn digest_u64(h: &mut u64, v: u64) {
    digest_bytes(h, &v.to_le_bytes());
}

/// HashMap 的顺序无关指纹：逐项哈希后按加法累加——加法交换律使结果与迭代
/// 顺序无关（`std` HashMap 遍历顺序不稳定，不能直接按序混入）。
fn digest_pairs(h: &mut u64, pairs: impl Iterator<Item = (u64, u64)>) {
    let mut acc = 0u64;
    for (k, v) in pairs {
        let mut e = 0x9e37_79b9_7f4a_7c15;
        digest_u64(&mut e, k);
        digest_u64(&mut e, v);
        acc = acc.wrapping_add(e);
    }
    digest_u64(h, acc);
}

/// token 列表 → 指纹（长度 + 逐 token 原始 8 字节）。
fn digest_toks(h: &mut u64, toks: &[Token]) {
    digest_u64(h, toks.len() as u64);
    for t in toks {
        digest_u64(h, t.raw());
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

    /// 只读访问 sink（export_state 读当前字体等）。
    pub fn sink_ref(&self) -> &dyn TokenSink {
        self.sink.as_ref()
    }

    /// 内部量查询目标：展开区域内转发到原 sink（VecSink 查询全为默认值）。
    pub fn query_sink_ref(&self) -> &dyn TokenSink {
        self.query_sink.as_deref().unwrap_or(self.sink.as_ref())
    }

    /// 取出输出 sink（排版器运行结束后取回 builder）。
    pub fn take_sink(&mut self) -> Box<dyn TokenSink> {
        std::mem::replace(&mut self.sink, Box::new(VecSink::default()))
    }

    /// 可变访问输出 sink（收尾冲页/输出例程交错期间仍挂在引擎上）。
    pub fn sink_mut(&mut self) -> &mut dyn TokenSink {
        &mut *self.sink
    }

    /// 只读访问内部参数（`.fmt` 加载后排版器镜像同步用）。
    pub fn params_ref(&self) -> &Params {
        &self.params
    }

    /// 写一个内部整数参数（`Params::misc[idx]`，M9 中文刀 3 新增）。
    ///
    /// 用途：宿主/上层要在**用户源文本之外**设引擎级默认值。首个用例是
    /// `\utfinputmode`——前端若靠"在源码前拼一行 `\utfinputmode=1`"来开启
    /// UTF-8，用户源会整体下移一行，log 里的 `l.N` 与编辑器行号错位；
    /// 走这里则源文本逐字节不动（行号即事实，见 docs/tooling-trust.md）。
    ///
    /// 越界下标静默忽略（引擎契约：输入可达路径不 panic）。与 `\utfinputmode`
    /// 赋值同语义：后写的（含用户源里的显式赋值）覆盖先写的。
    pub fn set_misc_int(&mut self, idx: usize, value: i64) {
        if let Some(slot) = self.params.misc.get_mut(idx) {
            *slot = value;
        }
    }

    // ---------- M5 增量计算：只读状态探针（plan.md §7；incremental 模块用） ----------
    //
    // 两个方法都只读、零语义改动：`value_state` 精确镜像 `.fmt` 未覆盖的值状态，
    // `boundary_is_clean` 判定段边界是否被"跨段构造"污染。缓存失效判定必须同时
    // 看这两个信号——只看其中一个会把"缓存仍有效"判错。

    /// 段边界值状态镜像（[`ValueState`]；含其余运行时状态的指纹）。
    pub fn value_state(&self) -> ValueState {
        ValueState {
            catcodes: self.catcodes.clone(),
            sfcodes: self.sfcodes,
            output_toks: self.output_toks.clone(),
            registers: self.registers.dirty().clone(),
            runtime_digest: self.runtime_digest(),
        }
    }

    /// 段边界是否干净：输入栈空、无未闭合组/条件/数学/对齐模板、无悬挂前缀。
    ///
    /// 不干净 = 有构造跨越段边界（未闭合 `{`、未闭合 `\if`、悬空 `$`、未写完的
    /// box 参数……）。此时该段之后**禁止复用缓存**（跨段构造的语义无法由
    /// "段前状态 + 段源码"还原，增量与全量本就不再可比）。
    pub fn boundary_is_clean(&self) -> bool {
        self.stack.is_empty()
            && self.group_level == 0
            && self.cond_stack.is_empty()
            && self.math_left_depth == 0
            && self.align_frames.is_empty()
            && self.group_cond_depth.is_empty()
            && self.save_stack.is_empty()
            && self.aftergroup.is_empty()
            && self.afterassignment.is_none()
            && !self.global_pending
            && !self.immediate_pending
            && !self.protected_pending
            && !self.outer_pending
            && !self.long_pending
            && !self.unless_pending
            && !self.pending_box_arg
            && self.read_floor == 0
            && self.suppress_expansion == 0
            && !self.expand_only
            && self.query_sink.is_none()
            && !self.output_active
            && !self.in_math
    }

    /// 其余运行时状态的 FNV-1a 指纹（64 位）。
    ///
    /// 覆盖 `.fmt` 快照未含、但影响后续输出的字段：`\everypar` 等 token 列表、
    /// `\mathcode`/`\delcode`/`\lccode`/`\uccode`、`\parshape`、e-TeX 惩罚数组、
    /// `\fontdimen`/`\hyphenchar`/`\skewchar` 覆盖、字体名表、读写流位置、
    /// `\dump`/数学模式标志、内部参数（Debug 串字段级覆盖）。
    /// **不含**诊断性状态（`last_tok`/`error_anchor`/`output_trigger_line` 等
    /// 只影响错误消息位置，不影响输出 token）。
    fn runtime_digest(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for toks in [
            &self.everypar_toks,
            &self.everymath,
            &self.everyhbox_toks,
            &self.everyvbox_toks,
            &self.everycr_toks,
            &self.everydisplay_toks,
            &self.errhelp_toks,
        ] {
            digest_toks(&mut h, toks);
        }
        // catcode/sfcode 表与 `\output` 例程**不**入指纹：`ValueState` 已对它们做
        // 精确比较，这里重复混入只会加倍每段判定的开销。
        for v in self.lccodes {
            digest_u64(&mut h, v as u64);
        }
        for v in self.uccodes {
            digest_u64(&mut h, v as u64);
        }
        digest_pairs(
            &mut h,
            self.mathcodes
                .iter()
                .map(|(k, v)| (u64::from(*k), u64::from(*v))),
        );
        digest_pairs(
            &mut h,
            self.delcodes
                .iter()
                .map(|(k, v)| (u64::from(*k), u64::from(*v))),
        );
        digest_pairs(
            &mut h,
            self.fontdimens
                .iter()
                .map(|(k, v)| ((u64::from(k.0) << 32) | u64::from(k.1), *v as u64)),
        );
        digest_pairs(
            &mut h,
            self.hyphenchars
                .iter()
                .map(|(k, v)| (u64::from(*k), *v as u64)),
        );
        digest_pairs(
            &mut h,
            self.skewchars
                .iter()
                .map(|(k, v)| (u64::from(*k), *v as u64)),
        );
        // 字体表与数学字体族
        digest_bytes(&mut h, format!("{:?}", self.font_loads).as_bytes());
        for names in [&self.font_names, &self.font_cs_names] {
            digest_u64(&mut h, names.len() as u64);
            for n in names.iter() {
                match n {
                    Some(s) => digest_bytes(&mut h, s.as_bytes()),
                    None => digest_u64(&mut h, u64::MAX),
                }
            }
        }
        for row in &self.math_fonts {
            for v in row {
                digest_u64(&mut h, u64::from(*v));
            }
        }
        // 段落形状 / e-TeX 惩罚数组
        for (i, w) in &self.parshape {
            digest_u64(&mut h, *i as u64);
            digest_u64(&mut h, *w as u64);
        }
        for arr in &self.penalty_arrays {
            digest_u64(&mut h, arr.len() as u64);
            for v in arr {
                digest_u64(&mut h, *v as u64);
            }
        }
        // 读写流（路径/位置/待写内容的规模摘要；内容级追踪是 M5 阶段二副作用边界）
        digest_u64(&mut h, self.read_streams.len() as u64);
        for s in self.read_streams.iter().flatten() {
            digest_u64(&mut h, s.data.len() as u64);
            digest_u64(&mut h, s.pos as u64);
        }
        digest_u64(&mut h, self.write_streams.len() as u64);
        for s in self.write_streams.iter().flatten() {
            digest_u64(&mut h, s.pending.len() as u64);
        }
        digest_u64(&mut h, self.log_write_pending.len() as u64);
        // 布尔标志
        digest_u64(
            &mut h,
            u64::from(self.dumped)
                | u64::from(self.in_math) << 1
                | u64::from(self.output_active) << 2,
        );
        // 内部参数（Debug 串覆盖全部字段；新增字段自动进入指纹）
        digest_bytes(&mut h, format!("{:?}", self.params).as_bytes());
        h
    }
}

impl Default for Expander {
    fn default() -> Self {
        Self::new()
    }
}

// ---------- 数据分片：fontdimens.rs（\fontdimen 覆盖表：map + 每字体最大参数号缓存） ----------
include!("fontdimens.rs");

// ---------- 方法分片（include! 嵌入；原 impl Expander 方法按域拆分） ----------
// ---------- 方法分片：macros.rs ----------
include!("macros.rs");

// ---------- 方法分片：primitive.rs ----------
include!("primitive.rs");

// ---------- 方法分片（PR-2b：exec_primitive 大 match 按主题拆分为 7 个 dispatcher，
// 依 primitive.rs 主 match 守卫顺序 include） ----------
include!("primitive_box.rs");
include!("primitive_param.rs");
include!("primitive_math.rs");
include!("primitive_expand.rs");
include!("primitive_align.rs");
// M4-5 对齐机制状态机（\halign/\valign preamble + 模板注入；tex.web §749-823）
include!("align.rs");
include!("primitive_toks_state.rs");
include!("primitive_io.rs");

// ---------- 方法分片：primitive_pdf_image.rs（pdfTeX 图片三原语：从 primitive.rs 拆出） ----------
include!("primitive_pdf_image.rs");

// ---------- 方法分片：primitive_font.rs（字体家族：从 primitive.rs 拆出） ----------
include!("primitive_font.rs");

// ---------- 方法分片：primitive_assign.rs（寄存器算术：从 primitive.rs 拆出） ----------
include!("primitive_assign.rs");

// ---------- 方法分片：primitive_codes.rs（字符代码映射：从 primitive.rs 拆出） ----------
include!("primitive_codes.rs");

// ---------- 方法分片：io.rs ----------
include!("io.rs");

// ---------- 方法分片：save.rs ----------
include!("save.rs");

// ---------- 方法分片：checkpoint.rs（M5 阶段二：段边界完整状态检查点） ----------
include!("checkpoint.rs");

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
