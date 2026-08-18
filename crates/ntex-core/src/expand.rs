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
//! - M1-8 分隔参数（delimited）未实现；
//! - M1-9 条件原语全实现（含 `\ifcase` 与惰性跳过）；
//! - M1-10 寄存器 `\count/\dimen/\skip/\toks` 与 `\the` 已实现（`\box`/`\muskip` 未实现）；
//! - M1-11 组作用域（朴素快照回滚 + `\global`）已实现；
//! - 空行 → `\par` 语义未实现（M1-14 修）。

use std::sync::Arc;

use crate::bytecode::{compile, Bytecode, Instruction};
use crate::catcode::{Catcode, CatcodeTable};
use crate::eqtb::{EqSlot, Eqtb, Primitive, StreamKind};
use crate::error::{Error, Result};
use crate::font::{FontLoader, NoFontLoader};
use crate::input::scan_token;
use crate::intern::InternTable;
use crate::macrodef::{MacroDef, ParamSpec, TokenArray};
use crate::param::{ParamKind, ParamValue, Params};
use crate::register::{
    format_count, format_dimen, format_glue, unit_to_sp, Glue, RegKind, RegisterState, Registers,
    REGISTER_COUNT, SP_PER_PT,
};
use crate::sink::{TokenSink, VecSink};
use crate::token::{Token, TokenKind};
use ntex_io::{LocalVfs, Vfs};

/// 输入帧：token 来源栈（LIFO，栈顶为当前帧）。
#[derive(Debug)]
enum InputFrame {
    /// 源码帧：字节流 + 扫描位置（catcode 表由引擎全局持有）。
    Source { bytes: Arc<[u8]>, pos: usize },
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
    Eqtb { csid: u32, prev: EqSlot },
    Count { idx: usize, prev: i64 },
    Dimen { idx: usize, prev: i64 },
    Skip { idx: usize, prev: Glue },
    Toks { idx: usize, prev: TokenArray },
    Catcode { byte: u8, prev: Catcode },
    Param { kind: ParamKind, prev: ParamValue },
    /// `\sfcode`：spacefactor 表项（M3-4 词间距）。
    Sfcode { byte: u8, prev: u32 },
    /// `\output`：输出例程 token 列表（M3-5-3）。
    Output { prev: Option<TokenArray> },
}

/// `\ifx` 语义键：解析别名后比较含义（TeX：同含义即相等）。
#[derive(Debug, Clone, PartialEq)]
enum MeaningKey {
    Undefined,
    Macro(Arc<MacroDef>),
    Primitive(Primitive),
    Char { catcode: Catcode, charcode: u32 },
    Alias(u32),
    Font(u32),
    Register(RegKind, usize),
    Stream(StreamKind, usize),
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
}

/// 展开引擎。
#[derive(Debug)]
pub struct Expander {
    intern: InternTable,
    eqtb: Eqtb,
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
    /// `\output` 例程 token 列表（M3-5-3）；None = 未定义（断页直通 shipout）。
    output_toks: Option<TokenArray>,
    /// 输出例程正在执行（防嵌套：例程内再次断页报错）。
    output_active: bool,
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
    /// `\immediate` 前缀（作用于下一个 write/openout/closeout）。
    immediate_pending: bool,
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
            group_cond_depth: Vec::new(),
            save_stack: Vec::new(),
            global_pending: false,
            aftergroup: Vec::new(),
            afterassignment: None,
            registers: Registers::new(),
            params: Params::default(),
            font_loader: Box::new(NoFontLoader),
            output_toks: None,
            output_active: false,
            output_prev_count: usize::MAX,
            use_bytecode,
            vfs: Box::new(LocalVfs),
            read_streams: Vec::new(),
            write_streams: Vec::new(),
            immediate_pending: false,
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
                    EqSlot::Macro(v) if v.value.code.is_none() => {
                        Some((v.value.params.clone(), v.value.body.clone()))
                    }
                    _ => None,
                };
                if let Some((params, body)) = pending {
                    let code = Arc::new(compile(&body, &eqtb));
                    if let EqSlot::Macro(v) = eqtb.slot_mut(csid) {
                        v.value = Arc::new(MacroDef {
                            params,
                            body,
                            code: Some(code),
                        });
                    }
                }
            }
        }
        self.eqtb = eqtb;
        self.registers = Registers::import(state.registers);
        self.params = state.params;
        self.output_toks = state.output_toks;
        // 运行时状态重置（新文档起点）
        self.stack.clear();
        self.read_floor = 0;
        self.cond_stack.clear();
        self.group_level = 0;
        self.group_cond_depth.clear();
        self.save_stack.clear();
        self.global_pending = false;
        self.aftergroup.clear();
        self.afterassignment = None;
        self.output_active = false;
        self.output_prev_count = usize::MAX;
        self.immediate_pending = false;
        self.read_streams.clear();
        self.write_streams.clear();
    }

    /// 追加一个源码输入（后续 `\input`/VFS 在 M3 接入）。
    pub fn feed_source(&mut self, text: impl Into<Vec<u8>>) {
        self.stack.push(InputFrame::Source {
            bytes: Arc::from(text.into()),
            pos: 0,
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
        loop {
            // 输出例程激活期间（例程帧在栈上）不重复注入
            if !self.output_active && self.maybe_inject_output()? {
                continue;
            }
            // RFC-3：页面真正输出（shipout 边界）时 flush 延迟写流
            if self.sink.take_write_flush_pending() {
                self.flush_writes()?;
            }
            if !self.process_one()? {
                break;
            }
        }
        if !self.cond_stack.is_empty() {
            return Err(Error::invalid_input("条件未闭合（缺少 \\fi）"));
        }
        Ok(())
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
                // 条件 token（\if*/\\else/\\fi/\\or）优先由条件机处理（无论是否跳过）
                if let Some(op) = self.cond_op(tok) {
                    self.step_conditional(op)?;
                    return Ok(true);
                }
                if self.is_skipping() {
                    // 跳过模式：其余 token 直接丢弃（不展开）
                    return Ok(true);
                }
                if noexpand {
                    // \noexpand：临时不可展开，原样输出
                    self.sink.token(tok)?;
                } else {
                    self.process_token(tok)?;
                }
                Ok(true)
            }
        }
    }

    /// 处理单个 token（展开宏/原语，其余输出）。
    fn process_token(&mut self, tok: Token) -> Result<()> {
        match tok.kind() {
            TokenKind::ControlSeq => {
                let csid = tok.csid().expect("ControlSeq 必有 csid");
                let slot = self.eqtb.slot(csid).clone();
                match slot {
                    EqSlot::Undefined => Err(Error::invalid_input(format!(
                        "未定义的控制序列：\\{}",
                        self.intern.name(csid)
                    ))),
                    EqSlot::Alias(target) => self.process_token(Token::control_sequence(target)),
                    EqSlot::Char { catcode, charcode } => {
                        self.sink.token(Token::char(catcode, charcode))
                    }
                    EqSlot::Register(..) | EqSlot::Stream(..) => Err(Error::invalid_input(
                        "寄存器/流引用不能直接使用（需在数字/尺寸扫描上下文中）",
                    )),
                    EqSlot::Macro(m) => {
                        let def = m.value.clone();
                        let args = if def.params.num_params > 0 {
                            self.collect_args(&def)?
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
                    EqSlot::Font(font) => self.sink.font_selected(font),
                    EqSlot::Primitive(p) => self.exec_primitive(p),
                }
            }
            TokenKind::Char => {
                // 组定界符（cat 1/2）在主流层建立/结束组（M1-11）
                match tok.catcode() {
                    Some(Catcode::BeginGroup) => self.begin_group(),
                    Some(Catcode::EndGroup) => self.end_group(),
                    _ => self.sink.token(tok),
                }
            }
            _ => self.sink.token(tok),
        }
    }

    // ---------- 输入获取 ----------

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
                InputFrame::Source { bytes, pos } => {
                    match scan_token(bytes, pos, &self.catcodes, &mut self.intern)? {
                        Some(tok) => return Ok(Some((tok, false))),
                        None => {
                            self.stack.pop();
                            continue;
                        }
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
                    if *pc >= code.instructions().len() {
                        self.stack.pop();
                        continue;
                    }
                    match code.instructions()[*pc] {
                        Instruction::Emit { token } => {
                            *pc += 1;
                            return Ok(Some((token, false)));
                        }
                        Instruction::EmitArg { n } => {
                            *pc += 1;
                            let arg = args
                                .get((n.saturating_sub(1)) as usize)
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
                        Instruction::End => {
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

    // ---------- 宏调用与实参 ----------

    /// 收集宏的全部实参（无分隔参数）。
    fn collect_args(&mut self, def: &MacroDef) -> Result<Vec<TokenArray>> {
        let mut args = Vec::with_capacity(def.params.num_params as usize);
        for _ in 0..def.params.num_params {
            args.push(self.collect_undelimited_arg(def.params.long)?);
        }
        Ok(args)
    }

    /// 收集一个无分隔实参：
    /// 跳过前导空格；`{...}` 取组内容（去外层花括号），否则取单个 token。
    fn collect_undelimited_arg(&mut self, long: bool) -> Result<TokenArray> {
        // 跳过前导空格
        loop {
            let tok = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("实参扫描到输入末尾"))?
                .0;
            if tok.catcode() != Some(Catcode::Space) {
                self.unread(tok);
                break;
            }
        }
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("实参扫描到输入末尾"))?
            .0;
        match tok.catcode() {
            Some(Catcode::BeginGroup) => {
                let mut tokens = Vec::new();
                let mut depth = 0usize;
                loop {
                    let t = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("实参组未闭合"))?
                        .0;
                    match t.catcode() {
                        Some(Catcode::BeginGroup) => {
                            depth += 1;
                            tokens.push(t);
                        }
                        Some(Catcode::EndGroup) => {
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                            tokens.push(t);
                        }
                        _ => tokens.push(t),
                    }
                }
                Ok(Arc::from(tokens))
            }
            _ => {
                if !long && self.is_par_token(tok) {
                    return Err(Error::invalid_input("参数包含 \\par（宏未声明 \\long）"));
                }
                Ok(Arc::from([tok]))
            }
        }
    }

    /// 判断 token 是否为 `\par`（M1：cat 5 字符或名为 "par" 的控制序列）。
    fn is_par_token(&self, tok: Token) -> bool {
        tok.catcode() == Some(Catcode::EndOfLine)
            || tok.csid().is_some_and(|id| self.intern.name(id) == "par")
    }

    // ---------- 原语执行 ----------

    fn exec_primitive(&mut self, prim: Primitive) -> Result<()> {
        match prim {
            Primitive::Relax => Ok(()),
            Primitive::Expandafter => self.exec_expandafter(),
            Primitive::Noexpand => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\noexpand 后无 token"))?;
                self.stack.push(InputFrame::TokenList {
                    items: Arc::from([(t.0, true)]),
                    pos: 0,
                });
                Ok(())
            }
            Primitive::Def => self.exec_def(false),
            Primitive::Edef => self.exec_def(true),
            // \gdef ≡ \global\def
            Primitive::Gdef => {
                self.global_pending = true;
                self.exec_def(false)
            }
            Primitive::Let => self.exec_let(),
            Primitive::Catcode => self.exec_catcode(),
            Primitive::SfCode => self.exec_sfcode(),
            Primitive::End => {
                self.stack.clear();
                self.output_active = false;
                // TeX `\end` 收尾：flush 所有延迟写流（final_cleanup 语义）
                self.flush_writes()?;
                Ok(())
            }
            // M1-7 扫描顺序原语
            Primitive::Futurelet => self.exec_futurelet(),
            Primitive::Aftergroup => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\aftergroup 后无 token"))?
                    .0;
                self.aftergroup.push((self.group_level, t));
                Ok(())
            }
            Primitive::Afterassignment => {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\afterassignment 后无 token"))?
                    .0;
                self.afterassignment = Some(t);
                Ok(())
            }
            // M1-9 条件原语：由 process_one 拦截，不应到达此处
            Primitive::If
            | Primitive::IfCat
            | Primitive::IfNum
            | Primitive::IfDim
            | Primitive::IfX
            | Primitive::IfOdd
            | Primitive::IfCase
            | Primitive::IfTrue
            | Primitive::IfFalse
            | Primitive::Else
            | Primitive::Fi
            | Primitive::Or => Err(Error::internal("条件原语不应到达 exec_primitive")),
            // M1-10 寄存器
            Primitive::Count | Primitive::Dimen | Primitive::Skip | Primitive::Toks => {
                self.exec_register(prim)
            }
            Primitive::The => self.exec_the(),
            Primitive::Global => {
                self.global_pending = true;
                Ok(())
            }
            // M1-11 组
            Primitive::BeginGroup => self.begin_group(),
            Primitive::EndGroup => self.end_group(),
            // M3-2 排版原语
            // 盒子：直通 sink（规格 to/spread 属 M3-2-2，暂拒）
            Primitive::HBox | Primitive::VBox | Primitive::VTop => {
                self.reject_box_spec()?;
                self.sink.primitive(prim)
            }
            Primitive::Par => self.sink.primitive(prim),
            // 带参数扫描的排版原语：扫描在 VM 侧完成，结果交给 sink
            Primitive::HSkip | Primitive::VSkip => {
                let g = self.scan_glue()?;
                self.sink.glue(g)
            }
            Primitive::Kern => {
                let w = self.scan_dimen()?;
                self.sink.kern(w)
            }
            Primitive::Penalty => {
                let p = self.scan_number()?;
                self.sink.penalty(p)
            }
            Primitive::HRule | Primitive::VRule => {
                let [h, d, w] = self.scan_rule_specs()?;
                self.sink.rule(w, h, d)
            }
            // M3-2-2 内部参数赋值
            Primitive::ParIndent | Primitive::LineSkipLimit => {
                let v = self.scan_dimen()?;
                let kind = if prim == Primitive::ParIndent {
                    ParamKind::ParIndent
                } else {
                    ParamKind::LineSkipLimit
                };
                self.assign_param(kind, ParamValue::Dimen(v))
            }
            Primitive::BaselineSkip | Primitive::LineSkip => {
                let g = self.scan_glue()?;
                let kind = if prim == Primitive::BaselineSkip {
                    ParamKind::BaselineSkip
                } else {
                    ParamKind::LineSkip
                };
                self.assign_param(kind, ParamValue::Glue(g))
            }
            // 段落缩进：直通 sink 由排版器解释
            Primitive::Indent | Primitive::NoIndent => self.sink.primitive(prim),
            // M3-3 折行参数
            Primitive::HSize => {
                let v = self.scan_dimen()?;
                self.assign_param(ParamKind::HSize, ParamValue::Dimen(v))
            }
            Primitive::Tolerance => {
                let v = self.scan_number()?;
                self.assign_param(ParamKind::Tolerance, ParamValue::Number(v))
            }
            // M3-5 断页参数
            Primitive::VSize | Primitive::MaxDepth => {
                let v = self.scan_dimen()?;
                let kind = if prim == Primitive::VSize {
                    ParamKind::VSize
                } else {
                    ParamKind::MaxDepth
                };
                self.assign_param(kind, ParamValue::Dimen(v))
            }
            Primitive::TopSkip | Primitive::ParSkip => {
                let g = self.scan_glue()?;
                let kind = if prim == Primitive::TopSkip {
                    ParamKind::TopSkip
                } else {
                    ParamKind::ParSkip
                };
                self.assign_param(kind, ParamValue::Glue(g))
            }
            // M3-4 字体
            Primitive::Font => self.exec_font(),
            // M3-5 输出：\shipout 直通 sink（排版器解释：封装下一盒子为页面）
            Primitive::ShipOut => self.sink.primitive(prim),
            // M3-5-3 输出例程：\output=<general text> 存储 token 列表
            Primitive::Output => {
                self.expect_equals()?;
                let val = self.scan_group_contents()?;
                self.assign_output(Arc::from(val));
                Ok(())
            }
            // M3-5-3 盒子寄存器：\box<n> 交给 sink（shipout_next 时封装为页面）
            Primitive::Box => {
                let idx = self.scan_register_index()?;
                self.sink.box_register(idx)
            }
            // M3 收尾（RFC-3）：VFS 副作用原语
            Primitive::Input => self.exec_input(),
            Primitive::OpenIn => self.exec_openin(),
            Primitive::CloseIn => self.exec_closein(),
            Primitive::NewRead => self.exec_new_stream(StreamKind::Read),
            Primitive::Read => self.exec_read(),
            Primitive::NewWrite => self.exec_new_stream(StreamKind::Write),
            Primitive::OpenOut => self.exec_openout(),
            Primitive::CloseOut => self.exec_closeout(),
            Primitive::Write => self.exec_write(),
            Primitive::Immediate => {
                self.immediate_pending = true;
                Ok(())
            }
        }
    }

    /// `\font<cs>[=]<名字>[at <dimen>|scaled <int>]`：加载字体并定义 cs 为字体选择器。
    ///
    /// 语法扫描在 VM 侧（cs、可选 `=`、字体名、可选 at/scaled），实际加载交给
    /// [`FontLoader`]（ntex-layout 的 TFM 加载器维护字体表并返回 FontId）。
    fn exec_font(&mut self) -> Result<()> {
        let name = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\font 后缺少控制序列"))?
            .0;
        let csid = name
            .csid()
            .ok_or_else(|| Error::invalid_input("\\font 后必须是控制序列"))?;
        // 可选赋值符 '='
        self.skip_spaces()?;
        let probe = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\font 后缺少字体名"))?
            .0;
        if probe.charcode() != Some(b'=' as u32) {
            self.unread(probe);
        }
        let font_name = self.scan_font_name()?;
        // 可选 at / scaled（互斥）
        let keyword = self.scan_keyword(|w| w == "at" || w == "scaled")?;
        let (at, scaled) = match keyword.as_deref() {
            Some("at") => (Some(self.scan_dimen()?), None),
            Some("scaled") => (None, Some(self.scan_number()?)),
            _ => (None, None),
        };
        let font = self.font_loader.load(&font_name, at, scaled)?;
        // 组作用域 + \global 语义（同 \def）
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Eqtb {
                    csid,
                    prev: self.eqtb.slot(csid).clone(),
                },
            ));
        }
        self.eqtb.set_font(csid, font);
        self.finish_assignment();
        Ok(())
    }

    /// 扫描外部字体名：连续 cat 11（字母）/ cat 12（其他）字符，遇空格/组/控制序列结束。
    fn scan_font_name(&mut self) -> Result<String> {
        self.skip_spaces()?;
        let mut name = String::new();
        while let Some((tok, _)) = self.fetch()? {
            match tok.catcode() {
                Some(Catcode::Letter) | Some(Catcode::Other) => {
                    let ch = tok
                        .charcode()
                        .and_then(char::from_u32)
                        .ok_or_else(|| Error::invalid_input("字体名含非法字符"))?;
                    if !ch.is_ascii() {
                        return Err(Error::invalid_input("字体名仅支持 ASCII（M3-4 范围）"));
                    }
                    name.push(ch);
                }
                _ => {
                    self.unread(tok);
                    break;
                }
            }
        }
        if name.is_empty() {
            return Err(Error::invalid_input("\\font 后缺少字体名"));
        }
        Ok(name)
    }

    // ---------- M3 收尾（RFC-3）：VFS 副作用原语 ----------

    /// `\input<file>`：读文件内容推入 `Source` 输入帧（支持嵌套）。
    ///
    /// 文件名扫描（TeX `scan_file_name`）：`{file}` 花括号形式或普通形式
    /// （cat 11/12 字符，空格终止）。找不到时先试原名、再补 `.tex`。
    fn exec_input(&mut self) -> Result<()> {
        let name = self.scan_file_name()?;
        let mut content = self
            .vfs
            .read(&name)
            .map_err(|e| Error::io("VFS 读取", &name, e))?;
        if content.is_none() {
            let alt = format!("{name}.tex");
            content = self
                .vfs
                .read(&alt)
                .map_err(|e| Error::io("VFS 读取", &alt, e))?;
        }
        match content {
            Some(bytes) => {
                self.stack.push(InputFrame::Source {
                    bytes: Arc::from(bytes),
                    pos: 0,
                });
                Ok(())
            }
            None => Err(Error::invalid_input(format!("找不到文件：{name}"))),
        }
    }

    /// 扫描文件名：`{...}` 或连续 cat 11/12 字符（空格终止）。
    fn scan_file_name(&mut self) -> Result<String> {
        self.skip_spaces()?;
        let first = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("扫描到输入末尾"))?
            .0;
        let mut name = String::new();
        if first.catcode() == Some(Catcode::BeginGroup) {
            // {file}：组内字符原样收集（含空格）
            loop {
                let t = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("文件名组未闭合"))?
                    .0;
                match t.catcode() {
                    Some(Catcode::EndGroup) => break,
                    Some(Catcode::Letter) | Some(Catcode::Other) | Some(Catcode::Space) => {
                        let ch = t
                            .charcode()
                            .and_then(char::from_u32)
                            .ok_or_else(|| Error::invalid_input("文件名含非法字符"))?;
                        name.push(ch);
                    }
                    _ => return Err(Error::invalid_input("文件名含非法 token")),
                }
            }
        } else {
            self.unread(first);
            loop {
                let Some((t, _)) = self.fetch()? else { break };
                match t.catcode() {
                    Some(Catcode::Letter) | Some(Catcode::Other) => {
                        let ch = t
                            .charcode()
                            .and_then(char::from_u32)
                            .ok_or_else(|| Error::invalid_input("文件名含非法字符"))?;
                        name.push(ch);
                    }
                    _ => {
                        self.unread(t);
                        break;
                    }
                }
            }
        }
        if name.is_empty() {
            return Err(Error::invalid_input("缺少文件名"));
        }
        Ok(name)
    }

    /// 扫描流号（0..=max）。
    fn scan_stream_index(&mut self, what: &str, max: i64) -> Result<usize> {
        let n = self.scan_number()?;
        if !(0..=max).contains(&n) {
            return Err(Error::invalid_input(format!("{what} 流号越界：{n}")));
        }
        Ok(n as usize)
    }

    /// `\openin<n>=<file>`：文件存在 → 读入内存打开；不存在 → 流保持未打开（不报错）。
    fn exec_openin(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\openin", 15)?;
        self.expect_equals()?;
        let name = self.scan_file_name()?;
        let content = self
            .vfs
            .read(&name)
            .map_err(|e| Error::io("VFS 读取", &name, e))?;
        self.ensure_read_stream(idx);
        self.read_streams[idx] = content.map(|data| ReadStream {
            _path: name,
            data,
            pos: 0,
        });
        Ok(())
    }

    /// `\closein<n>`：关闭读流。
    fn exec_closein(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\closein", 15)?;
        self.ensure_read_stream(idx);
        self.read_streams[idx] = None;
        Ok(())
    }

    /// `\newwrite<cs>` / `\newread<cs>`：分配最小空闲流号，绑定到 cs（流引用）。
    fn exec_new_stream(&mut self, s: StreamKind) -> Result<()> {
        let t = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("流分配后缺少控制序列"))?
            .0;
        let csid = t
            .csid()
            .ok_or_else(|| Error::invalid_input("流分配后必须是控制序列"))?;
        let free = match s {
            StreamKind::Read => (0..=15)
                .find(|&i| self.read_streams.get(i).is_none_or(|s| s.is_none())),
            StreamKind::Write => (0..=17)
                .find(|&i| self.write_streams.get(i).is_none_or(|s| s.is_none())),
        };
        let n = free.ok_or_else(|| Error::invalid_input("无空闲流号"))?;
        // cs 绑定为流引用（组作用域回滚，独立于 count 槽）
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Eqtb {
                    csid,
                    prev: self.eqtb.slot(csid).clone(),
                },
            ));
        }
        *self.eqtb.slot_mut(csid) = EqSlot::Stream(s, n);
        self.finish_assignment();
        Ok(())
    }

    /// `\read<n> to <cs>`：从流读一行，按当前 catcode 表 token 化，`\def` 赋给 cs。
    fn exec_read(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\read", 15)?;
        self.scan_keyword(|w| w == "to")?;
        self.skip_spaces()?;
        let t = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\read 后缺少控制序列"))?
            .0;
        let csid = t
            .csid()
            .ok_or_else(|| Error::invalid_input("\\read to 后必须是控制序列"))?;
        // 取下一行（到 \n 或文件末尾）
        let line = {
            let Some(stream) = self.read_streams.get_mut(idx).and_then(|s| s.as_mut()) else {
                return Err(Error::invalid_input("\\read 流未打开"));
            };
            if stream.pos >= stream.data.len() {
                return Err(Error::invalid_input("\\read 到文件末尾（EOF）"));
            }
            let start = stream.pos;
            let end = stream.data[start..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|i| start + i)
                .unwrap_or(stream.data.len());
            let line = stream.data[start..end].to_vec();
            stream.pos = if end < stream.data.len() { end + 1 } else { end };
            line
        };
        // token 化（catcode 表）
        let mut pos = 0usize;
        let mut toks = Vec::new();
        while let Some(tok) = scan_token(&line, &mut pos, &self.catcodes, &mut self.intern)? {
            toks.push(tok);
        }
        // \def 语义赋值
        let def = MacroDef {
            params: ParamSpec {
                num_params: 0,
                long: false,
                delimiter: None,
            },
            body: Arc::from(toks),
            code: None,
        };
        self.define_macro_scoped(csid, def);
        Ok(())
    }

    /// `\openout<n>=<file>`：登记写流目标路径（不立即创建文件）。
    fn exec_openout(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\openout", 17)?;
        self.expect_equals()?;
        let name = self.scan_file_name()?;
        let immediate = self.take_immediate();
        self.ensure_write_stream(idx);
        self.write_streams[idx] = Some(WriteStream {
            path: Some(name.clone()),
            pending: Vec::new(),
        });
        if immediate {
            // \immediate\openout：立即创建（TeX 语义）
            self.vfs
                .write(&name, b"")
                .map_err(|e| Error::io("VFS 写入", &name, e))?;
        }
        Ok(())
    }

    /// `\closeout<n>`：flush 待写内容并关闭。
    fn exec_closeout(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\closeout", 17)?;
        let immediate = self.take_immediate();
        if !immediate {
            self.flush_write_stream(idx)?;
        } else {
            self.flush_write_stream(idx)?;
        }
        self.ensure_write_stream(idx);
        self.write_streams[idx] = None;
        Ok(())
    }

    /// `\write<n><general text>`：token 列表入队（延迟）或立即展开落盘（`\immediate`）。
    fn exec_write(&mut self) -> Result<()> {
        let idx = self.scan_stream_index("\\write", 18)?;
        if idx == 18 {
            return Err(Error::invalid_input("\\write18（shell 转义）暂不支持"));
        }
        let toks = Arc::from(self.scan_general_text()?);
        self.ensure_write_stream(idx);
        if self.take_immediate() {
            let s = self.expand_to_string(&toks)?;
            let path = self
                .write_streams
                .get(idx)
                .and_then(|s| s.as_ref())
                .and_then(|st| st.path.clone())
                .ok_or_else(|| Error::invalid_input("\\write 到未打开的流"))?;
            let mut out = s;
            out.push('\n');
            self.vfs
                .append(&path, out.as_bytes())
                .map_err(|e| Error::io("VFS 写入", &path, e))?;
        } else {
            self.write_streams[idx]
                .as_mut()
                .expect("exec_write 已 ensure 流槽")
                .pending
                .push(toks);
        }
        Ok(())
    }

    /// 扫描 `<general text>`：到 `\relax`（无条件）或外层组结束（吸收 `}`）为止。
    fn scan_general_text(&mut self) -> Result<Vec<Token>> {
        self.skip_spaces()?;
        let mut toks = Vec::new();
        let mut depth = 0usize;
        loop {
            let t = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("\\write 文本未闭合"))?
                .0;
            match t.catcode() {
                Some(Catcode::BeginGroup) => {
                    depth += 1;
                    toks.push(t);
                }
                Some(Catcode::EndGroup) => {
                    if depth == 0 {
                        break; // 组外 }：终止
                    }
                    depth -= 1;
                    if depth == 0 {
                        break; // 匹配到最外层 {：终止并吸收
                    }
                    toks.push(t);
                }
                _ => {
                    // \relax 无条件终止
                    if t.csid()
                        .is_some_and(|c| self.eqtb.slot(c) == &EqSlot::Primitive(Primitive::Relax))
                    {
                        break;
                    }
                    toks.push(t);
                }
            }
        }
        Ok(toks)
    }

    /// 把 token 列表展开成字符串（flush 边界写文件用）：完全展开后
    /// 字符 token → 字节、空格 → ` `；不可展开的 cs → 报错。
    fn expand_to_string(&mut self, toks: &[Token]) -> Result<String> {
        let expanded = self.expand_region(toks.to_vec())?;
        let mut s = String::new();
        for t in expanded {
            match t.catcode() {
                Some(Catcode::Space) => s.push(' '),
                Some(Catcode::Letter) | Some(Catcode::Other) => {
                    let ch = t
                        .charcode()
                        .and_then(char::from_u32)
                        .ok_or_else(|| Error::invalid_input("\\write 输出含非法字符"))?;
                    s.push(ch);
                }
                _ => {
                    return Err(Error::invalid_input(
                        "\\write 输出含不可展开的 token（宏/原语泄漏）",
                    ));
                }
            }
        }
        Ok(s)
    }

    /// flush 单个写流：展开全部待写 token 并追加到目标文件（每条后加换行）。
    fn flush_write_stream(&mut self, idx: usize) -> Result<()> {
        let (path, pending) = {
            let Some(stream) = self.write_streams.get_mut(idx).and_then(|s| s.as_mut()) else {
                return Ok(()); // 未打开：无操作
            };
            if stream.pending.is_empty() {
                return Ok(());
            }
            (stream.path.clone(), std::mem::take(&mut stream.pending))
        };
        let Some(path) = path else {
            return Err(Error::invalid_input("\\write 到未打开的流"));
        };
        let mut out = String::new();
        for toks in pending {
            out.push_str(&self.expand_to_string(&toks)?);
            out.push('\n');
        }
        self.vfs
            .append(&path, out.as_bytes())
            .map_err(|e| Error::io("VFS 写入", &path, e))
    }

    /// flush 所有打开且有待写内容的写流（shipout 边界 / `\end` / 排版结束调用）。
    pub fn flush_writes(&mut self) -> Result<()> {
        for i in 0..self.write_streams.len() {
            self.flush_write_stream(i)?;
        }
        Ok(())
    }

    /// 消费 `\immediate` 前缀。
    fn take_immediate(&mut self) -> bool {
        let v = self.immediate_pending;
        self.immediate_pending = false;
        v
    }

    /// 确保读流槽存在。
    fn ensure_read_stream(&mut self, idx: usize) {
        while self.read_streams.len() <= idx {
            self.read_streams.push(None);
        }
    }

    /// 确保写流槽存在（未打开时补空槽）。
    fn ensure_write_stream(&mut self, idx: usize) {
        while self.write_streams.len() <= idx {
            self.write_streams.push(None);
        }
        if self.write_streams[idx].is_none() {
            self.write_streams[idx] = Some(WriteStream {
                path: None,
                pending: Vec::new(),
            });
        }
    }

    /// 内部参数赋值（组作用域 + sink 镜像通知）。
    fn assign_param(&mut self, kind: ParamKind, value: ParamValue) -> Result<()> {
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Param {
                    kind,
                    prev: self.params.get(kind),
                },
            ));
        }
        self.params.set(kind, value);
        self.sink.param_changed(kind, value)?;
        self.finish_assignment();
        Ok(())
    }

    /// 暂拒 `\hbox to <glue>` / `\hbox spread <glue>` 规格（待实现）。
    fn reject_box_spec(&mut self) -> Result<()> {
        if self.scan_keyword(|w| w == "to" || w == "spread")?.is_some() {
            return Err(Error::invalid_input(
                "\\hbox/\\vbox 的 to/spread 规格暂不支持（M3-2-2）",
            ));
        }
        Ok(())
    }

    /// 扫描 `\hrule`/`\vrule` 的可选规格：
    /// `height <dimen> depth <dimen> width <dimen>`（任意顺序、可省略，缺省 0）。
    /// 返回 `[height, depth, width]`。
    fn scan_rule_specs(&mut self) -> Result<[i64; 3]> {
        let mut specs = [0i64; 3];
        for _ in 0..3 {
            let Some(kw) = self.scan_keyword(|w| matches!(w, "height" | "depth" | "width"))?
            else {
                break;
            };
            let idx = match kw.as_str() {
                "height" => 0,
                "depth" => 1,
                _ => 2,
            };
            specs[idx] = self.scan_dimen()?;
        }
        Ok(specs)
    }

    /// `\def`/`\edef`：扫描控制序列名 + 参数文本 + 替换文本并定义。
    fn exec_def(&mut self, expand_body: bool) -> Result<()> {
        let name = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\def 后缺少控制序列"))?
            .0;
        let csid = name
            .csid()
            .ok_or_else(|| Error::invalid_input("\\def 后必须是控制序列"))?;

        let num_params = self.scan_parameter_text()?;
        let body_raw = self.scan_balanced_text()?;
        let body: TokenArray = if expand_body {
            Arc::from(self.expand_region(body_raw)?)
        } else {
            Arc::from(body_raw)
        };

        let mut def = MacroDef {
            params: ParamSpec {
                num_params,
                long: false,
                delimiter: None,
            },
            body,
            code: None,
        };
        // M2：编译期预编译字节码（常量条件折叠等），解释器轨道不编译
        if self.use_bytecode {
            def.code = Some(Arc::new(compile(&def.body, &self.eqtb)));
        }
        self.define_macro_scoped(csid, def);
        Ok(())
    }

    /// 扫描参数文本直到 `{`；解析 `#n` → 参数计数。`##` → 字面 `#`。
    fn scan_parameter_text(&mut self) -> Result<u8> {
        let mut num = 0u8;
        loop {
            let tok = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("\\def 参数文本未闭合（缺少 {）"))?
                .0;
            match tok.catcode() {
                Some(Catcode::BeginGroup) => break,
                _ if is_parameter_char(tok) => {
                    let next = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("参数文本中 # 后无 token"))?
                        .0;
                    if let Some(d) = digit_value(next) {
                        if d == 0 {
                            return Err(Error::invalid_input("非法参数号 #0"));
                        }
                        num = num.max(d);
                    } else if is_parameter_char(next) {
                        // ## → 字面 #，跳过
                    } else {
                        return Err(Error::invalid_input("参数文本中 # 后必须跟数字或 #"));
                    }
                }
                _ => {}
            }
        }
        Ok(num)
    }

    /// 扫描平衡花括号内的替换文本；`#n` → 参数槽 token，`##` → 字面 `#`。
    fn scan_balanced_text(&mut self) -> Result<Vec<Token>> {
        let mut out = Vec::new();
        let mut depth = 0usize;
        loop {
            let tok = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("替换文本未闭合（缺少 }）"))?
                .0;
            match tok.catcode() {
                Some(Catcode::EndGroup) => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    out.push(tok);
                }
                Some(Catcode::BeginGroup) => {
                    depth += 1;
                    out.push(tok);
                }
                _ if is_parameter_char(tok) => {
                    let next = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("替换文本中 # 后无 token"))?
                        .0;
                    if let Some(d) = digit_value(next) {
                        out.push(Token::macro_param(d));
                    } else if is_parameter_char(next) {
                        out.push(Token::char(Catcode::Parameter, b'#' as u32));
                    } else {
                        return Err(Error::invalid_input("替换文本中 # 后必须跟数字或 #"));
                    }
                }
                _ => out.push(tok),
            }
        }
        Ok(out)
    }

    /// 把 token 列表放到输入流顶并全展开（`\edef` 用），返回展开结果。
    ///
    /// 通过临时提升 `read_floor` 划定区域边界，防止越过该区域读取外层输入；
    /// 区域内条件必须闭合（回到进入时的条件栈深度）。
    fn expand_region(&mut self, tokens: Vec<Token>) -> Result<Vec<Token>> {
        let saved_floor = self.read_floor;
        let depth = self.stack.len();
        let cond_depth = self.cond_stack.len();
        self.read_floor = depth;
        // 区域输出重定向到临时 VecSink（M3-2：sink 替代 output 字段）
        let saved = std::mem::replace(&mut self.sink, Box::new(VecSink::default()));

        let items: Vec<(Token, bool)> = tokens.into_iter().map(|t| (t, false)).collect();
        self.stack.push(InputFrame::TokenList {
            items: Arc::from(items),
            pos: 0,
        });
        while self.process_one()? {}

        if self.cond_stack.len() != cond_depth {
            self.sink = saved;
            self.read_floor = saved_floor;
            return Err(Error::invalid_input("条件未闭合（缺少 \\fi）"));
        }
        let temp = std::mem::replace(&mut self.sink, saved);
        let result = temp
            .take_tokens()
            .expect("expand_region 安装了 VecSink");
        self.read_floor = saved_floor;
        Ok(result)
    }

    /// `\let\cs<token>`：cs 别名到控制序列或等价于字符。
    /// TeX 语义：`=` 是可选赋值符（`\let\cs=x` 等价 `\let\cs x`）。
    fn exec_let(&mut self) -> Result<()> {
        let name = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\let 后缺少控制序列"))?
            .0;
        let csid = name
            .csid()
            .ok_or_else(|| Error::invalid_input("\\let 后必须是控制序列"))?;

        // 可选空格 + 可选 '='
        self.skip_spaces()?;
        let probe = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\let 后缺少被别名 token"))?
            .0;
        let rhs = if probe.charcode() == Some(b'=' as u32) {
            self.skip_spaces()?;
            self.fetch()?
                .ok_or_else(|| Error::invalid_input("\\let 后缺少被别名 token"))?
                .0
        } else {
            self.unread(probe);
            self.fetch()?
                .ok_or_else(|| Error::invalid_input("\\let 后缺少被别名 token"))?
                .0
        };

        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Eqtb {
                    csid,
                    prev: self.eqtb.slot(csid).clone(),
                },
            ));
        }
        match rhs.kind() {
            TokenKind::ControlSeq => {
                let target = rhs.csid().expect("ControlSeq 必有 csid");
                self.eqtb.alias(csid, target);
            }
            TokenKind::Char => {
                let catcode = rhs.catcode().expect("Char 必有 catcode");
                let charcode = rhs.charcode().expect("Char 必有 charcode");
                self.eqtb.char_alias(csid, catcode, charcode);
            }
            _ => return Err(Error::invalid_input("\\let 仅支持控制序列或字符")),
        }
        self.finish_assignment();
        Ok(())
    }

    /// `\catcode<byte>=<num>`：修改 catcode 表（组内局部、可 `\global`）。
    fn exec_catcode(&mut self) -> Result<()> {
        let byte = self.scan_number()?;
        let byte = u8::try_from(byte).map_err(|_| Error::invalid_input("\\catcode 字符码越界"))?;
        self.expect_equals()?;
        let code = self.scan_number()?;
        let cat = Catcode::from_u8(
            u8::try_from(code).map_err(|_| Error::invalid_input("catcode 必须在 0..=15"))?,
        )
        .ok_or_else(|| Error::invalid_input("catcode 必须在 0..=15"))?;
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Catcode {
                    byte,
                    prev: self.catcodes.get(byte),
                },
            ));
        }
        self.catcodes.set(byte, cat);
        self.finish_assignment();
        Ok(())
    }

    /// `\sfcode<字符>=<值>`：设置字符的 spacefactor（TeX define_char_code 类）。
    fn exec_sfcode(&mut self) -> Result<()> {
        let byte = self.scan_number()?;
        let byte = u8::try_from(byte).map_err(|_| Error::invalid_input("\\sfcode 字符码越界"))?;
        self.expect_equals()?;
        let value = self.scan_number()?;
        let value = u32::try_from(value).map_err(|_| Error::invalid_input("\\sfcode 值越界"))?;
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Sfcode {
                    byte,
                    prev: self.sfcodes[byte as usize],
                },
            ));
        }
        self.sfcodes[byte as usize] = value;
        self.sink.sfcode_changed(byte, value)?;
        self.finish_assignment();
        Ok(())
    }

    /// `\expandafter a b`：输出 a，再输出 b 的一次展开结果。
    ///
    /// 展开"一次"：宏 → 实参替换后的宏体（不再递归展开）；`\expandafter` → 递归；
    /// `\noexpand` → 标记下一 token；其余原样。
    fn exec_expandafter(&mut self) -> Result<()> {
        let t1 = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\expandafter 后无 token"))?;
        let t2 = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\expandafter 后无第二个 token"))?;
        let mut expansion = Vec::new();
        self.expand_once(t2, &mut expansion)?;
        let mut seq = Vec::with_capacity(1 + expansion.len());
        seq.push(t1);
        seq.extend(expansion);
        self.stack.push(InputFrame::TokenList {
            items: Arc::from(seq),
            pos: 0,
        });
        Ok(())
    }

    /// 展开单个 token 一次，结果追加到 `out`。
    fn expand_once(&mut self, item: (Token, bool), out: &mut Vec<(Token, bool)>) -> Result<()> {
        if item.1 {
            // 已被 \noexpand 标记：不展开
            out.push(item);
            return Ok(());
        }
        let tok = item.0;
        if let Some(csid) = tok.csid() {
            match self.eqtb.slot(csid).clone() {
                EqSlot::Alias(target) => {
                    out.push((Token::control_sequence(target), false));
                }
                EqSlot::Macro(m) => {
                    let def = m.value.clone();
                    let args = if def.params.num_params > 0 {
                        self.collect_args(&def)?
                    } else {
                        Vec::new()
                    };
                    let materialized = materialize(&def.body, &args);
                    out.extend(materialized.into_iter().map(|t| (t, false)));
                }
                EqSlot::Primitive(Primitive::Expandafter) => {
                    let a = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("\\expandafter 链中断"))?;
                    let b = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("\\expandafter 链中断"))?;
                    out.push(a);
                    self.expand_once(b, out)?;
                }
                EqSlot::Primitive(Primitive::Noexpand) => {
                    let t = self
                        .fetch()?
                        .ok_or_else(|| Error::invalid_input("\\noexpand 后无 token"))?;
                    out.push((t.0, true));
                }
                EqSlot::Primitive(Primitive::The) => {
                    let tokens = self.the_tokens()?;
                    out.extend(tokens.into_iter().map(|t| (t, false)));
                }
                _ => {
                    // 未定义/不可展开原语：原样保留
                    out.push((tok, false));
                }
            }
        } else {
            out.push((tok, false));
        }
        Ok(())
    }

    // ---------- 数字与赋值辅助 ----------

    /// 扫描十进制整数；支持 `\count<idx>` 寄存器引用（M1 简化版）。
    fn scan_number(&mut self) -> Result<i64> {
        self.skip_spaces()?;
        let mut neg = false;
        if let Some(tok) = self.fetch()?.map(|t| t.0) {
            if tok.charcode() == Some(b'-' as u32) {
                neg = true;
            } else {
                self.unread(tok);
            }
        }
        // 寄存器引用：\count<idx> 或 \count\cs（\newcount 分配的 cs）
        if let Some(csid) = self.peek_csid()? {
            let slot = self.eqtb.slot(csid).clone();
            match slot {
                EqSlot::Register(RegKind::Count, idx) => {
                    self.fetch()?; // 消费 cs
                    let v = self.registers.count(idx);
                    return Ok(if neg { -v } else { v });
                }
                EqSlot::Stream(_, n) => {
                    self.fetch()?; // 消费 cs
                    return Ok(if neg { -(n as i64) } else { n as i64 });
                }
                EqSlot::Primitive(Primitive::Count) => {
                    self.fetch()?; // 消费 \count
                    let idx = self.scan_register_index()?;
                    let v = self.registers.count(idx);
                    return Ok(if neg { -v } else { v });
                }
                _ => {}
            }
        }
        let mut val: i64 = 0;
        let mut any = false;
        while let Some((tok, _)) = self.fetch()? {
            match digit_value(tok) {
                Some(d) => {
                    val = val * 10 + i64::from(d);
                    any = true;
                }
                None => {
                    self.unread(tok);
                    break;
                }
            }
        }
        if !any {
            return Err(Error::invalid_input("预期数字"));
        }
        // TeX 规则：数字后跟随的空格被吞掉（实测 pdfTeX `\ifnum3>2 yes` → "yes"）
        self.skip_trailing_spaces()?;
        Ok(if neg { -val } else { val })
    }

    /// 跳过前导空格 token（输入耗尽视为合法，返回 Ok）。
    fn skip_spaces(&mut self) -> Result<()> {
        loop {
            let Some((tok, _)) = self.fetch()? else { return Ok(()) };
            if tok.catcode() != Some(Catcode::Space) {
                self.unread(tok);
                return Ok(());
            }
        }
    }

    /// 吞掉数字/尺寸后的尾随空格（输入耗尽时直接返回）。
    fn skip_trailing_spaces(&mut self) -> Result<()> {
        loop {
            match self.fetch()? {
                None => return Ok(()),
                Some((tok, _)) => {
                    if tok.catcode() == Some(Catcode::Space) {
                        continue;
                    }
                    self.unread(tok);
                    return Ok(());
                }
            }
        }
    }

    /// 期望赋值符 `=`（允许前后空格）。
    fn expect_equals(&mut self) -> Result<()> {
        self.skip_spaces()?;
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("扫描到输入末尾"))?
            .0;
        if tok.charcode() != Some(b'=' as u32) {
            return Err(Error::invalid_input("预期 '='（赋值符）"));
        }
        Ok(())
    }

    /// 注册 M1 内建原语。
    fn register_builtins(&mut self) {
        const BUILTINS: [(&str, Primitive); 69] = [
            ("def", Primitive::Def),
            ("edef", Primitive::Edef),
            ("gdef", Primitive::Gdef),
            ("let", Primitive::Let),
            ("relax", Primitive::Relax),
            ("expandafter", Primitive::Expandafter),
            ("noexpand", Primitive::Noexpand),
            ("catcode", Primitive::Catcode),
            ("end", Primitive::End),
            // M1-7
            ("futurelet", Primitive::Futurelet),
            ("aftergroup", Primitive::Aftergroup),
            ("afterassignment", Primitive::Afterassignment),
            // M1-9
            ("if", Primitive::If),
            ("ifcat", Primitive::IfCat),
            ("ifnum", Primitive::IfNum),
            ("ifdim", Primitive::IfDim),
            ("ifx", Primitive::IfX),
            ("ifodd", Primitive::IfOdd),
            ("ifcase", Primitive::IfCase),
            ("iftrue", Primitive::IfTrue),
            ("iffalse", Primitive::IfFalse),
            ("else", Primitive::Else),
            ("fi", Primitive::Fi),
            ("or", Primitive::Or),
            // M1-10
            ("count", Primitive::Count),
            ("dimen", Primitive::Dimen),
            ("skip", Primitive::Skip),
            ("toks", Primitive::Toks),
            ("the", Primitive::The),
            ("global", Primitive::Global),
            // M1-11
            ("begingroup", Primitive::BeginGroup),
            ("endgroup", Primitive::EndGroup),
            // M3-2 排版原语
            ("hbox", Primitive::HBox),
            ("vbox", Primitive::VBox),
            ("vtop", Primitive::VTop),
            ("hskip", Primitive::HSkip),
            ("vskip", Primitive::VSkip),
            ("kern", Primitive::Kern),
            ("penalty", Primitive::Penalty),
            ("hrule", Primitive::HRule),
            ("vrule", Primitive::VRule),
            ("par", Primitive::Par),
            // M3-2-2 内部参数与段落
            ("parindent", Primitive::ParIndent),
            ("baselineskip", Primitive::BaselineSkip),
            ("lineskip", Primitive::LineSkip),
            ("lineskiplimit", Primitive::LineSkipLimit),
            ("indent", Primitive::Indent),
            ("noindent", Primitive::NoIndent),
            // M3-3 折行参数
            ("hsize", Primitive::HSize),
            ("tolerance", Primitive::Tolerance),
            // M3-4 字体
            ("font", Primitive::Font),
            // M3-5 输出
            ("shipout", Primitive::ShipOut),
            // M3-5 断页参数
            ("vsize", Primitive::VSize),
            ("topskip", Primitive::TopSkip),
            ("maxdepth", Primitive::MaxDepth),
            ("parskip", Primitive::ParSkip),
            // M3-4 词间距
            ("sfcode", Primitive::SfCode),
            // M3-5-3 输出例程
            ("output", Primitive::Output),
            ("box", Primitive::Box),
            // M3 收尾（RFC-3）：VFS 副作用原语
            ("input", Primitive::Input),
            ("openin", Primitive::OpenIn),
            ("closein", Primitive::CloseIn),
            ("newread", Primitive::NewRead),
            ("read", Primitive::Read),
            ("newwrite", Primitive::NewWrite),
            ("openout", Primitive::OpenOut),
            ("closeout", Primitive::CloseOut),
            ("write", Primitive::Write),
            ("immediate", Primitive::Immediate),
        ];
        for (name, prim) in BUILTINS {
            let csid = self.intern.intern(name);
            self.eqtb.set_primitive(csid, prim);
        }
    }

    // ---------- M1-7 扫描顺序原语 ----------

    /// `\futurelet\cs T1 T2`：\cs ← \let T2（不展开），T1、T2 继续正常处理。
    fn exec_futurelet(&mut self) -> Result<()> {
        let name = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\futurelet 后缺少控制序列"))?
            .0;
        let csid = name
            .csid()
            .ok_or_else(|| Error::invalid_input("\\futurelet 后必须是控制序列"))?;
        let t1 = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\futurelet 后缺少 token"))?
            .0;
        let t2 = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\futurelet 后缺少被观察 token"))?
            .0;
        self.let_to(csid, t2);
        let items: Vec<(Token, bool)> = vec![(t1, false), (t2, false)];
        self.stack.push(InputFrame::TokenList {
            items: Arc::from(items),
            pos: 0,
        });
        Ok(())
    }

    /// `\let` 语义：cs 等价于 token（控制序列别名 / 字符等价）。
    fn let_to(&mut self, csid: u32, rhs: Token) {
        match rhs.kind() {
            TokenKind::ControlSeq => {
                let target = rhs.csid().expect("ControlSeq 必有 csid");
                self.eqtb.alias(csid, target);
            }
            TokenKind::Char => {
                let catcode = rhs.catcode().expect("Char 必有 catcode");
                let charcode = rhs.charcode().expect("Char 必有 charcode");
                self.eqtb.char_alias(csid, catcode, charcode);
            }
            _ => {} // 非字符/控制序列：忽略（TeX 报错，M1 宽松）
        }
    }

    // ---------- M1-10 寄存器 ----------

    /// `\count/\dimen/\skip/\toks` 赋值。
    fn exec_register(&mut self, prim: Primitive) -> Result<()> {
        let kind = match prim {
            Primitive::Count => RegKind::Count,
            Primitive::Dimen => RegKind::Dimen,
            Primitive::Skip => RegKind::Skip,
            Primitive::Toks => RegKind::Toks,
            _ => unreachable!("exec_register 只处理寄存器原语"),
        };
        let idx = self.scan_register_target(kind)?;
        self.expect_equals()?;
        match prim {
            Primitive::Count => {
                let val = self.scan_number()?;
                self.assign_count(idx, val);
            }
            Primitive::Dimen => {
                let val = self.scan_dimen()?;
                self.assign_dimen(idx, val);
            }
            Primitive::Skip => {
                let val = self.scan_glue()?;
                self.assign_skip(idx, val);
            }
            Primitive::Toks => {
                let val = self.scan_group_contents()?;
                self.assign_toks(idx, Arc::from(val));
            }
            _ => unreachable!("exec_register 只处理寄存器原语"),
        }
        Ok(())
    }

    /// 扫描寄存器目标：数字下标（`\count0`）或 cs 引用（`\count\foo`，须已分配）。
    fn scan_register_target(&mut self, kind: RegKind) -> Result<usize> {
        self.skip_spaces()?;
        let t = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("预期寄存器下标"))?
            .0;
        if let Some(csid) = t.csid() {
            match self.eqtb.slot(csid) {
                EqSlot::Register(k, idx) if *k == kind => Ok(*idx),
                _ => Err(Error::invalid_input(
                    "寄存器未分配（先 \\newcount 等分配）",
                )),
            }
        } else {
            self.unread(t);
            self.scan_register_index()
        }
    }

    /// 扫描寄存器下标（0..=255）。
    fn scan_register_index(&mut self) -> Result<usize> {
        let n = self.scan_number()?;
        if !(0..REGISTER_COUNT as i64).contains(&n) {
            return Err(Error::invalid_input(format!("寄存器下标越界：{n}")));
        }
        Ok(n as usize)
    }

    /// 扫描平衡花括号内的 token 列表（`\toks0={...}` 用）。
    fn scan_group_contents(&mut self) -> Result<Vec<Token>> {
        let open = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("扫描到输入末尾"))?
            .0;
        if open.catcode() != Some(Catcode::BeginGroup) {
            return Err(Error::invalid_input("预期 {（组开始）"));
        }
        let mut tokens = Vec::new();
        let mut depth = 0usize;
        loop {
            let t = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("组未闭合"))?
                .0;
            match t.catcode() {
                Some(Catcode::BeginGroup) => {
                    depth += 1;
                    tokens.push(t);
                }
                Some(Catcode::EndGroup) => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    tokens.push(t);
                }
                _ => tokens.push(t),
            }
        }
        Ok(tokens)
    }

    fn assign_count(&mut self, idx: usize, val: i64) {
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Count {
                    idx,
                    prev: self.registers.count(idx),
                },
            ));
        }
        self.registers.set_count(idx, val);
        self.finish_assignment();
    }

    fn assign_dimen(&mut self, idx: usize, val: i64) {
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Dimen {
                    idx,
                    prev: self.registers.dimen(idx),
                },
            ));
        }
        self.registers.set_dimen(idx, val);
        self.finish_assignment();
    }

    fn assign_skip(&mut self, idx: usize, val: Glue) {
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Skip {
                    idx,
                    prev: self.registers.skip(idx),
                },
            ));
        }
        self.registers.set_skip(idx, val);
        self.finish_assignment();
    }

    fn assign_toks(&mut self, idx: usize, val: TokenArray) {
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Toks {
                    idx,
                    prev: self.registers.toks(idx),
                },
            ));
        }
        self.registers.set_toks(idx, val);
        self.finish_assignment();
    }

    /// `\output=<general text>`：设置输出例程 token 列表（M3-5-3）。
    /// 通知排版器：fire_up 改道 box255 + 待执行；组内局部、可 `\global`。
    fn assign_output(&mut self, val: TokenArray) {
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Output {
                    prev: self.output_toks.clone(),
                },
            ));
        }
        self.output_toks = Some(val);
        let _ = self.sink.output_defined(true);
        self.finish_assignment();
    }

    /// `\the<寄存器>`：把寄存器值展开为 token 流。
    fn exec_the(&mut self) -> Result<()> {
        let tokens = self.the_tokens()?;
        let items: Vec<(Token, bool)> = tokens.into_iter().map(|t| (t, false)).collect();
        self.stack.push(InputFrame::TokenList {
            items: Arc::from(items),
            pos: 0,
        });
        Ok(())
    }

    /// 计算 `\the` 的 token 序列（`\count/\dimen/\skip/\toks`）。
    fn the_tokens(&mut self) -> Result<Vec<Token>> {
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("\\the 后缺少参数"))?
            .0;
        let csid = tok
            .csid()
            .ok_or_else(|| Error::invalid_input("\\the 需要寄存器参数"))?;
        match self.eqtb.slot(csid) {
            EqSlot::Primitive(p) => match p {
                Primitive::Count => {
                    let idx = self.scan_register_index()?;
                    Ok(emit_count(self.registers.count(idx)))
                }
                Primitive::Dimen => {
                    let idx = self.scan_register_index()?;
                    Ok(emit_dimen(self.registers.dimen(idx)))
                }
                Primitive::Skip => {
                    let idx = self.scan_register_index()?;
                    Ok(emit_glue(self.registers.skip(idx)))
                }
                Primitive::Toks => {
                    let idx = self.scan_register_index()?;
                    Ok(self.registers.toks(idx).to_vec())
                }
                Primitive::ParIndent
                | Primitive::BaselineSkip
                | Primitive::LineSkip
                | Primitive::LineSkipLimit
                | Primitive::HSize
                | Primitive::Tolerance
                | Primitive::VSize
                | Primitive::TopSkip
                | Primitive::MaxDepth
                | Primitive::ParSkip => {
                    let kind = match p {
                        Primitive::ParIndent => ParamKind::ParIndent,
                        Primitive::BaselineSkip => ParamKind::BaselineSkip,
                        Primitive::LineSkip => ParamKind::LineSkip,
                        Primitive::LineSkipLimit => ParamKind::LineSkipLimit,
                        Primitive::HSize => ParamKind::HSize,
                        Primitive::Tolerance => ParamKind::Tolerance,
                        Primitive::VSize => ParamKind::VSize,
                        Primitive::TopSkip => ParamKind::TopSkip,
                        Primitive::MaxDepth => ParamKind::MaxDepth,
                        _ => ParamKind::ParSkip,
                    };
                    Ok(match self.params.get(kind) {
                        ParamValue::Dimen(v) => emit_dimen(v),
                        ParamValue::Glue(g) => emit_glue(g),
                        ParamValue::Number(v) => emit_count(v),
                    })
                }
                _ => Err(Error::invalid_input(
                    "\\the 只支持 \\count\\dimen\\skip\\toks 与内部参数",
                )),
            },
            _ => Err(Error::invalid_input("\\the 需要寄存器参数")),
        }
    }

    /// 组作用域（M1-11）。
    fn begin_group(&mut self) -> Result<()> {
        self.group_level += 1;
        // 记录组开始时的条件栈深度：组结束时条件必须回到该深度（跨组开条件 → 错误）
        self.group_cond_depth.push(self.cond_stack.len());
        // M3-2：通知 sink 组开始（排版器据此构建盒子内容）
        self.sink.group_begin()
    }

    fn end_group(&mut self) -> Result<()> {
        if self.group_level == 0 {
            return Err(Error::invalid_input("多余的 }"));
        }
        let cond_depth = self
            .group_cond_depth
            .pop()
            .expect("begin_group 与 end_group 必须配对");
        if self.cond_stack.len() != cond_depth {
            return Err(Error::invalid_input("组内条件未闭合（缺少 \\fi）"));
        }
        // 恢复本层保存的赋值
        while let Some((level, _)) = self.save_stack.last() {
            if *level != self.group_level {
                break;
            }
            let (_, v) = self.save_stack.pop().expect("last() 已检查非空");
            self.restore(v);
        }
        // 触发 \aftergroup
        let tokens: Vec<Token> = self
            .aftergroup
            .iter()
            .filter(|(l, _)| *l == self.group_level)
            .map(|(_, t)| *t)
            .collect();
        self.aftergroup.retain(|(l, _)| *l != self.group_level);
        if !tokens.is_empty() {
            let items: Vec<(Token, bool)> = tokens.into_iter().map(|t| (t, false)).collect();
            self.stack.push(InputFrame::TokenList {
                items: Arc::from(items),
                pos: 0,
            });
        }
        self.group_level -= 1;
        // M3-2：通知 sink 组结束（排版器封装盒子内容）。
        // 放在 `\aftergroup` 之后：其 token 在组外上下文继续处理，不落入盒子。
        self.sink.group_end()
    }

    fn restore(&mut self, v: SavedValue) {
        match v {
            SavedValue::Eqtb { csid, prev } => {
                *self.eqtb.slot_mut(csid) = prev;
            }
            SavedValue::Count { idx, prev } => self.registers.set_count(idx, prev),
            SavedValue::Dimen { idx, prev } => self.registers.set_dimen(idx, prev),
            SavedValue::Skip { idx, prev } => self.registers.set_skip(idx, prev),
            SavedValue::Toks { idx, prev } => self.registers.set_toks(idx, prev),
            SavedValue::Catcode { byte, prev } => self.catcodes.set(byte, prev),
            SavedValue::Param { kind, prev } => self.params.set(kind, prev),
            SavedValue::Sfcode { byte, prev } => {
                self.sfcodes[byte as usize] = prev;
                let _ = self.sink.sfcode_changed(byte, prev);
            }
            SavedValue::Output { prev } => {
                self.output_toks = prev;
                let _ = self.sink.output_defined(self.output_toks.is_some());
            }
        }
    }

    /// 带作用域的宏定义：组内局部保存 + `\afterassignment` 触发。
    fn define_macro_scoped(&mut self, csid: u32, def: MacroDef) {
        let global = self.is_global();
        if !global && self.group_level > 0 {
            self.save_stack.push((
                self.group_level,
                SavedValue::Eqtb {
                    csid,
                    prev: self.eqtb.slot(csid).clone(),
                },
            ));
        }
        self.eqtb.define_macro(csid, def);
        self.finish_assignment();
    }

    /// 消费 `\global` 前缀（每个赋值只消费一次）。
    fn is_global(&mut self) -> bool {
        let g = self.global_pending;
        self.global_pending = false;
        g
    }

    /// 赋值完成后触发 `\afterassignment`。
    fn finish_assignment(&mut self) {
        if let Some(tok) = self.afterassignment.take() {
            self.unread(tok);
        }
    }

    // ---------- M1-9 条件 ----------

    fn cond_op(&self, tok: Token) -> Option<CondOp> {
        let csid = tok.csid()?;
        match self.eqtb.slot(csid) {
            EqSlot::Primitive(p) => CondOp::from_prim(*p),
            _ => None,
        }
    }

    /// 是否处于跳过模式（最内层条件帧在跳过）。
    fn is_skipping(&self) -> bool {
        self.cond_stack
            .last()
            .is_some_and(|f| f.state == CondState::Skipping)
    }

    /// 条件状态机单步推进。
    fn step_conditional(&mut self, op: CondOp) -> Result<()> {
        match op {
            CondOp::Fi => {
                if self.cond_stack.pop().is_none() {
                    return Err(Error::invalid_input("多余的 \\fi"));
                }
                Ok(())
            }
            CondOp::Else => {
                let Some(top) = self.cond_stack.last_mut() else {
                    return Err(Error::invalid_input("多余的 \\else"));
                };
                if top.else_seen {
                    return Err(Error::invalid_input("多余的 \\else"));
                }
                top.else_seen = true;
                match top.state {
                    CondState::Skipping => {
                        if top.owns_skip {
                            top.state = CondState::Processing;
                        }
                    }
                    CondState::Processing => {
                        top.state = CondState::Skipping;
                        top.owns_skip = false;
                    }
                }
                Ok(())
            }
            CondOp::Or => {
                let Some(top) = self.cond_stack.last_mut() else {
                    return Err(Error::invalid_input("多余的 \\or"));
                };
                if !top.is_case {
                    return Err(Error::invalid_input("多余的 \\or"));
                }
                match top.state {
                    CondState::Skipping => {
                        if let Some(k) = top.ors_left {
                            if k == 1 {
                                top.state = CondState::Processing;
                                top.ors_left = None;
                            } else {
                                top.ors_left = Some(k - 1);
                            }
                        }
                    }
                    CondState::Processing => {
                        top.state = CondState::Skipping;
                        top.owns_skip = false;
                    }
                }
                Ok(())
            }
            CondOp::If
            | CondOp::IfCat
            | CondOp::IfNum
            | CondOp::IfDim
            | CondOp::IfX
            | CondOp::IfOdd
            | CondOp::IfTrue
            | CondOp::IfFalse => {
                if self.is_skipping() {
                    // 惰性：不评估测试，仅计数（未走的分支中的宏不被展开）
                    self.cond_stack.push(CondFrame {
                        is_case: false,
                        state: CondState::Skipping,
                        owns_skip: false,
                        ors_left: None,
                        else_seen: false,
                    });
                    return Ok(());
                }
                let truth = self.evaluate_if(op)?;
                self.cond_stack.push(CondFrame {
                    is_case: false,
                    state: if truth {
                        CondState::Processing
                    } else {
                        CondState::Skipping
                    },
                    owns_skip: !truth,
                    ors_left: None,
                    else_seen: false,
                });
                Ok(())
            }
            CondOp::IfCase => {
                if self.is_skipping() {
                    self.cond_stack.push(CondFrame {
                        is_case: true,
                        state: CondState::Skipping,
                        owns_skip: false,
                        ors_left: None,
                        else_seen: false,
                    });
                    return Ok(());
                }
                let n = self.scan_number()?;
                if n < 0 {
                    return Err(Error::invalid_input("\\ifcase 序号不能为负"));
                }
                self.cond_stack.push(CondFrame {
                    is_case: true,
                    state: if n == 0 {
                        CondState::Processing
                    } else {
                        CondState::Skipping
                    },
                    owns_skip: n > 0,
                    ors_left: (n > 0).then_some(n as usize),
                    else_seen: false,
                });
                Ok(())
            }
        }
    }

    /// 评估条件测试（跳过模式下不会被调用）。
    fn evaluate_if(&mut self, op: CondOp) -> Result<bool> {
        match op {
            CondOp::IfTrue => Ok(true),
            CondOp::IfFalse => Ok(false),
            CondOp::If => {
                let t1 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\if 缺操作数"))?
                    .0;
                let t2 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\if 缺操作数"))?
                    .0;
                Ok(matches!(t1.kind(), TokenKind::Char)
                    && matches!(t2.kind(), TokenKind::Char)
                    && t1 == t2)
            }
            CondOp::IfCat => {
                let t1 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifcat 缺操作数"))?
                    .0;
                let t2 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifcat 缺操作数"))?
                    .0;
                Ok(t1.catcode() == t2.catcode())
            }
            CondOp::IfX => {
                let t1 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifx 缺操作数"))?
                    .0;
                let t2 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifx 缺操作数"))?
                    .0;
                Ok(self.ifx_equal(t1, t2))
            }
            CondOp::IfNum => {
                let a = self.scan_number()?;
                let rel = self.scan_relation()?;
                let b = self.scan_number()?;
                Ok(compare(a, b, rel))
            }
            CondOp::IfDim => {
                let a = self.scan_dimen()?;
                let rel = self.scan_relation()?;
                let b = self.scan_dimen()?;
                Ok(compare(a, b, rel))
            }
            CondOp::IfOdd => Ok(self.scan_number()? % 2 != 0),
            CondOp::IfCase | CondOp::Else | CondOp::Fi | CondOp::Or => {
                unreachable!("step_conditional 已分流")
            }
        }
    }

    /// `\ifx`：字符按 (catcode,char)；控制序列按含义（解析别名）；其余 false。
    fn ifx_equal(&self, t1: Token, t2: Token) -> bool {
        match (t1.kind(), t2.kind()) {
            (TokenKind::Char, TokenKind::Char) => t1 == t2,
            (TokenKind::ControlSeq, TokenKind::ControlSeq) => {
                self.meaning_key(t1.csid().expect("ControlSeq 必有 csid"))
                    == self.meaning_key(t2.csid().expect("ControlSeq 必有 csid"))
            }
            _ => false,
        }
    }

    /// 控制序列的含义键（沿 Alias 链解析）。
    fn meaning_key(&self, csid: u32) -> MeaningKey {
        let mut id = csid;
        let mut hops = 0usize;
        while let EqSlot::Alias(target) = self.eqtb.slot(id) {
            id = *target;
            hops += 1;
            if hops > 64 {
                return MeaningKey::Alias(id);
            }
        }
        match self.eqtb.slot(id).clone() {
            EqSlot::Undefined => MeaningKey::Undefined,
            EqSlot::Macro(m) => MeaningKey::Macro(m.value),
            EqSlot::Primitive(p) => MeaningKey::Primitive(p),
            EqSlot::Char { catcode, charcode } => MeaningKey::Char { catcode, charcode },
            EqSlot::Alias(t) => MeaningKey::Alias(t),
            EqSlot::Font(font) => MeaningKey::Font(font),
            EqSlot::Register(k, n) => MeaningKey::Register(k, n),
            EqSlot::Stream(k, n) => MeaningKey::Stream(k, n),
        }
    }

    fn scan_relation(&mut self) -> Result<Relation> {
        self.skip_spaces()?;
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("关系符扫描到输入末尾"))?
            .0;
        match tok.charcode() {
            Some(c) if c == b'<' as u32 => Ok(Relation::Lt),
            Some(c) if c == b'=' as u32 => Ok(Relation::Eq),
            Some(c) if c == b'>' as u32 => Ok(Relation::Gt),
            _ => Err(Error::invalid_input("预期 < = > 关系符")),
        }
    }

    /// 扫描一个尺寸：数字（含小数）+ 可选单位；支持 `\dimen<idx>` 引用。
    ///
    /// 换算对照 pdfTeX：`scaled = (int + frac/10^k) * unit_sp`（逐项截断）。
    fn scan_dimen(&mut self) -> Result<i64> {
        self.skip_spaces()?;
        let mut neg = false;
        if let Some(tok) = self.fetch()?.map(|t| t.0) {
            if tok.charcode() == Some(b'-' as u32) {
                neg = true;
            } else {
                self.unread(tok);
            }
        }
        // 寄存器引用：\dimen<idx>
        if let Some(csid) = self.peek_csid()? {
            if let EqSlot::Primitive(Primitive::Dimen) = self.eqtb.slot(csid) {
                self.fetch()?; // 消费 \dimen
                let idx = self.scan_register_index()?;
                let v = self.registers.dimen(idx);
                return Ok(if neg { -v } else { v });
            }
        }
        // 数字：整数部分 + 可选小数
        let mut int_part: i64 = 0;
        let mut frac: i64 = 0;
        let mut frac_len: u32 = 0;
        let mut any = false;
        let mut saw_dot = false;
        while let Some((tok, _)) = self.fetch()? {
            if let Some(d) = digit_value(tok) {
                if saw_dot {
                    frac = frac * 10 + i64::from(d);
                    frac_len += 1;
                } else {
                    int_part = int_part * 10 + i64::from(d);
                }
                any = true;
            } else if tok.charcode() == Some(b'.' as u32) && !saw_dot {
                saw_dot = true;
            } else {
                self.unread(tok);
                break;
            }
        }
        if !any {
            return Err(Error::invalid_input("预期尺寸数字"));
        }
        // 单位：连续字母（缺省 pt）
        let mut unit_tokens = Vec::new();
        while let Some((tok, _)) = self.fetch()? {
            if let Some(ch) = tok.charcode().and_then(char::from_u32) {
                if ch.is_ascii_alphabetic() {
                    unit_tokens.push(ch);
                    continue;
                }
            }
            self.unread(tok);
            break;
        }
        let unit = if unit_tokens.is_empty() {
            "pt".to_owned()
        } else {
            unit_tokens.into_iter().collect()
        };
        let unit_sp =
            unit_to_sp(&unit).ok_or_else(|| Error::invalid_input(format!("未知单位：{unit}")))?;
        // scaled = (int + frac/10^k) * unit_sp（i128 防溢出，截断）
        let num_pt: i128 = (i128::from(int_part) * i128::from(SP_PER_PT))
            + (i128::from(frac) * i128::from(SP_PER_PT)) / 10i128.pow(frac_len);
        let scaled: i128 = num_pt * i128::from(unit_sp) / i128::from(SP_PER_PT);
        let scaled = if neg { -scaled } else { scaled };
        let scaled = i64::try_from(scaled).map_err(|_| Error::invalid_input("尺寸溢出"))?;
        // TeX 规则：尺寸后跟随的空格被吞掉
        self.skip_trailing_spaces()?;
        Ok(scaled)
    }

    /// 扫描胶水：width + 可选 `plus <dimen>` / `minus <dimen>`。
    /// 非 plus/minus 字母（如正文）原样放回（TeX `scan_keyword` 语义）。
    fn scan_glue(&mut self) -> Result<Glue> {
        let width = self.scan_dimen()?;
        let mut stretch = 0i64;
        let mut shrink = 0i64;
        for _ in 0..2 {
            let Some(word) = self.scan_keyword(|w| w == "plus" || w == "minus")? else {
                break;
            };
            if word == "plus" {
                stretch = self.scan_dimen()?;
            } else {
                shrink = self.scan_dimen()?;
            }
        }
        Ok(Glue {
            width,
            stretch,
            shrink,
        })
    }

    /// 跳过空格后读取一个裸字母词（TeX `scan_keyword` 语义：`\hskip 5pt plus 2pt`
    /// 中的 `plus`、`\hrule height 1pt` 中的 `height` 都是裸字母词）。
    /// `is_kw` 判定是否为关键字；非关键字时字母原样放回（保持顺序）并返回 None。
    fn scan_keyword(&mut self, is_kw: impl Fn(&str) -> bool) -> Result<Option<String>> {
        self.skip_spaces()?;
        let mut word = String::new();
        let mut letters: Vec<(Token, bool)> = Vec::new();
        while let Some((tok, _)) = self.fetch()? {
            if let Some(ch) = tok.charcode().and_then(char::from_u32) {
                if ch.is_ascii_alphabetic() {
                    word.push(ch);
                    letters.push((tok, false));
                    continue;
                }
            }
            self.unread(tok);
            break;
        }
        if word.is_empty() {
            return Ok(None);
        }
        if is_kw(&word) {
            return Ok(Some(word));
        }
        self.stack.push(InputFrame::TokenList {
            items: Arc::from(letters),
            pos: 0,
        });
        Ok(None)
    }

    /// 窥视下一个 token 是否为控制序列（fetch + unread）。
    fn peek_csid(&mut self) -> Result<Option<u32>> {
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("扫描到输入末尾"))?
            .0;
        let csid = tok.csid();
        self.unread(tok);
        Ok(csid)
    }

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

// ---------- 自由函数 ----------

/// 宏体实参替换：`#n` → 第 n 个实参（整段借用，零拷贝）。
fn materialize(body: &[Token], args: &[TokenArray]) -> Vec<Token> {
    let mut out = Vec::with_capacity(body.len());
    for &t in body {
        if let Some(n) = t.param_number() {
            if let Some(arg) = args.get(n.saturating_sub(1) as usize) {
                out.extend_from_slice(arg);
            }
        } else {
            out.push(t);
        }
    }
    out
}

/// token 是否为参数符 `#`（cat 6）。
fn is_parameter_char(tok: Token) -> bool {
    tok.catcode() == Some(Catcode::Parameter)
}

/// 字符 token 是否为十进制数字；返回数字值。
fn digit_value(tok: Token) -> Option<u8> {
    let ch = tok.charcode()? as u8;
    if ch.is_ascii_digit() {
        Some(ch - b'0')
    } else {
        None
    }
}

/// 整数 → `\the` token 序列（十进制字符，cat 12）。
fn emit_count(v: i64) -> Vec<Token> {
    format_count(v)
        .bytes()
        .map(|b| Token::char(Catcode::Other, u32::from(b)))
        .collect()
}

/// 尺寸 → `\the` token 序列（如 "2.5pt"）。
fn emit_dimen(v: i64) -> Vec<Token> {
    let mut s = format_dimen(v);
    s.push_str("pt");
    s.bytes()
        .map(|b| Token::char(Catcode::Other, u32::from(b)))
        .collect()
}

/// 胶水 → `\the` token 序列（如 "1.0pt plus 2.0pt minus 0.5pt"）。
fn emit_glue(g: Glue) -> Vec<Token> {
    format_glue(g)
        .bytes()
        .map(|b| Token::char(Catcode::Other, u32::from(b)))
        .collect()
}

/// 按关系符比较两个内部量。
fn compare(a: i64, b: i64, rel: Relation) -> bool {
    match rel {
        Relation::Lt => a < b,
        Relation::Eq => a == b,
        Relation::Gt => a > b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use ntex_io::MemVfs;

    /// 运行源码（**双轨等价**）：字节码与解释器轨道各跑一次并断言输出一致，
    /// 返回字节码轨道结果。全部用例自动覆盖 M2 双轨验证。
    fn expand(src: &str) -> Result<String> {
        let bytecode = expand_track(src, true)?;
        let interp = expand_track(src, false)?;
        assert_eq!(bytecode, interp, "双轨输出不一致：{src}");
        Ok(bytecode)
    }

    fn expand_track(src: &str, use_bytecode: bool) -> Result<String> {
        let mut e = if use_bytecode {
            Expander::new()
        } else {
            Expander::new_interpreter()
        };
        e.run_source(src)?;
        Ok(e.output()
            .iter()
            .map(|t| t.charcode().and_then(char::from_u32).unwrap_or('\u{FFFD}'))
            .collect())
    }

    #[test]
    fn simple_def_and_use() {
        assert_eq!(expand("\\def\\foo{Hello}\\foo").unwrap(), "Hello");
    }

    #[test]
    fn macro_with_argument() {
        assert_eq!(
            expand("\\def\\greet#1{Hi #1!}\\greet{World}").unwrap(),
            "Hi World!"
        );
    }

    #[test]
    fn macro_with_two_arguments() {
        assert_eq!(
            expand("\\def\\pair#1#2{[#1:#2]}\\pair{x}{y}").unwrap(),
            "[x:y]"
        );
    }

    #[test]
    fn edef_expands_at_definition() {
        assert_eq!(expand("\\def\\a{1}\\edef\\y{\\a2}\\y").unwrap(), "12");
    }

    #[test]
    fn let_alias() {
        assert_eq!(expand("\\def\\a{XY}\\let\\b\\a\\b").unwrap(), "XY");
    }

    #[test]
    fn let_to_char() {
        assert_eq!(expand("\\let\\X=x\\X").unwrap(), "x");
    }

    #[test]
    fn expandafter_classic() {
        // 经典：\expandafter\def\expandafter\x\expandafter{\b} 使 \x = \b 的展开。
        // 注：\b 中 \a 与 C 之间的空格在扫描时被控制词吞掉，故为 "ABC"（与真实 TeX 一致）。
        let src =
            "\\def\\a{B}\\def\\b{A\\a C}\\expandafter\\def\\expandafter\\x\\expandafter{\\b}\\x";
        assert_eq!(expand(src).unwrap(), "ABC");
    }

    #[test]
    fn noexpand_defers_expansion() {
        // \edef 时 \noexpand\y 使 \y 保持为 token；\z 使用时 \y 才展开
        let src = "\\def\\y{YY}\\def\\x{A\\noexpand\\y B}\\edef\\z{\\x}\\z";
        assert_eq!(expand(src).unwrap(), "AYYB");
    }

    #[test]
    fn catcode_change_affects_later_input() {
        // \catcode92=12 后 `\` 变为普通字符。
        // 用 \relax 隔离数字与后续输入（数字扫描会预读紧邻 token，与真实 TeX 一致）。
        assert_eq!(expand("\\catcode92=12\\relax\\abc").unwrap(), "\\abc");
    }

    #[test]
    fn active_char_can_be_defined() {
        let src = "\\def~{TILDE}\\def\\x{a~b}\\x";
        assert_eq!(expand(src).unwrap(), "aTILDEb");
    }

    #[test]
    fn undefined_control_sequence_errors() {
        let err = expand("\\def\\foo{Hi}\\bar").unwrap_err();
        assert!(err.to_string().contains("未定义的控制序列"));
    }

    #[test]
    fn end_stops_processing() {
        // \end 后内容不再处理
        assert_eq!(
            expand("\\def\\foo{Hi}\\foo\\end\\def\\bar{Bad}\\bar").unwrap(),
            "Hi"
        );
    }

    #[test]
    fn nested_braces_in_argument() {
        // 实参内层花括号在主流层建立组（不输出），与真实 TeX 一致
        assert_eq!(expand("\\def\\wrap#1{[#1]}\\wrap{a{b}c}").unwrap(), "[abc]");
    }

    #[test]
    fn empty_argument() {
        assert_eq!(expand("\\def\\wrap#1{[#1]}\\wrap{}").unwrap(), "[]");
    }

    // ---------- M1-7 扫描顺序原语 ----------

    #[test]
    fn futurelet_captures_next_token() {
        // \futurelet\next\relax a → \next := 字符 'a'（\let 语义），\relax 与 a 照常处理；
        // 用 \ifx 与另一个 \let 到 'a' 的控制序列比较（\ifx CS vs 裸字符必为假）。
        let src =
            "\\futurelet\\next\\relax a\\let\\expected a\\ifx\\next\\expected yes\\else no\\fi";
        assert_eq!(expand(src).unwrap(), "ayes");
    }

    #[test]
    fn aftergroup_inserts_token_at_group_end() {
        let src = "\\def\\X{Z}\\begingroup\\aftergroup\\X\\endgroup";
        assert_eq!(expand(src).unwrap(), "Z");
    }

    #[test]
    fn afterassignment_inserts_token_after_assignment() {
        let src = "\\def\\X{done}\\afterassignment\\X\\count0=5";
        assert_eq!(expand(src).unwrap(), "done");
    }

    #[test]
    fn afterassignment_survives_group_scope() {
        // \afterassignment 触发时已出组，赋值本身组内局部
        let src = "\\count0=1\\def\\X{a}\\begingroup\\afterassignment\\X\\count0=2\\endgroup\\the\\count0";
        assert_eq!(expand(src).unwrap(), "a1");
    }

    // ---------- M1-9 条件原语 ----------

    #[test]
    fn iftrue_else_branch() {
        assert_eq!(expand("\\iftrue yes\\else no\\fi").unwrap(), "yes");
        assert_eq!(expand("\\iffalse yes\\else no\\fi").unwrap(), "no");
    }

    #[test]
    fn if_compares_char_tokens() {
        // 操作数取紧邻两个 token；分支用花括号包裹以避开尾随空格歧义
        assert_eq!(expand("\\if aa{y}\\else n\\fi").unwrap(), "y");
        assert_eq!(expand("\\if ab{y}\\else n\\fi").unwrap(), "n");
    }

    #[test]
    fn ifcat_compares_catcodes() {
        // 'a' cat11 vs '1' cat12 → 不等；'a' vs 'b' → 相等
        assert_eq!(expand("\\ifcat a1{y}\\else n\\fi").unwrap(), "n");
        assert_eq!(expand("\\ifcat ab{y}\\else n\\fi").unwrap(), "y");
    }

    #[test]
    fn ifnum_with_relations() {
        assert_eq!(expand("\\ifnum3>2 yes\\else no\\fi").unwrap(), "yes");
        assert_eq!(expand("\\ifnum2>3 yes\\else no\\fi").unwrap(), "no");
        assert_eq!(expand("\\ifnum5=5 yes\\else no\\fi").unwrap(), "yes");
        assert_eq!(expand("\\ifnum4<4 yes\\else no\\fi").unwrap(), "no");
    }

    #[test]
    fn ifdim_with_units() {
        assert_eq!(expand("\\ifdim1pt<2pt yes\\else no\\fi").unwrap(), "yes");
        assert_eq!(
            expand("\\ifdim1in=72.27pt yes\\else no\\fi").unwrap(),
            "yes"
        );
    }

    #[test]
    fn ifx_compares_meanings() {
        // 相同定义的宏 → 相等；不同定义 → 不等
        assert_eq!(
            expand("\\def\\a{A}\\def\\b{A}\\ifx\\a\\b yes\\else no\\fi").unwrap(),
            "yes"
        );
        assert_eq!(
            expand("\\def\\a{A}\\def\\b{B}\\ifx\\a\\b yes\\else no\\fi").unwrap(),
            "no"
        );
        // \let 别名解析后与目标同义
        assert_eq!(
            expand("\\def\\a{A}\\let\\c\\a\\ifx\\a\\c yes\\else no\\fi").unwrap(),
            "yes"
        );
    }

    #[test]
    fn ifodd() {
        assert_eq!(expand("\\ifodd3 yes\\else no\\fi").unwrap(), "yes");
        assert_eq!(expand("\\ifodd4 yes\\else no\\fi").unwrap(), "no");
    }

    #[test]
    fn ifcase_selects_branch() {
        assert_eq!(
            expand("\\ifcase2 zero\\or one\\or two\\or three\\else many\\fi").unwrap(),
            "two"
        );
        assert_eq!(
            expand("\\ifcase0 zero\\or one\\or two\\else many\\fi").unwrap(),
            "zero"
        );
        assert_eq!(
            expand("\\ifcase5 zero\\or one\\else many\\fi").unwrap(),
            "many"
        );
    }

    #[test]
    fn conditional_skips_without_expanding() {
        // 跳过分支中的 \def 与嵌套 \if 不得执行/展开
        let src = "\\iffalse \\def\\bad{OOPS}\\bad \\ifnum1=1 hi\\else no\\fi \\else good\\fi";
        assert_eq!(expand(src).unwrap(), "good");
    }

    #[test]
    fn nested_conditionals() {
        let src = "\\iftrue A\\ifnum2>1 B\\else C\\fi D\\else E\\fi";
        assert_eq!(expand(src).unwrap(), "ABD");
        let src2 = "\\iffalse A\\ifnum2>1 B\\else C\\fi D\\else E\\fi";
        assert_eq!(expand(src2).unwrap(), "E");
    }

    #[test]
    fn unbalanced_fi_errors() {
        assert!(expand("\\iftrue A").is_err());
        assert!(expand("\\fi").is_err());
        assert!(expand("\\else").is_err());
    }

    // ---------- M1-10 寄存器 ----------

    #[test]
    fn count_assignment_and_the() {
        assert_eq!(expand("\\count0=5\\the\\count0").unwrap(), "5");
        assert_eq!(
            expand("\\count0=42\\count1=\\count0\\the\\count1").unwrap(),
            "42"
        );
        assert_eq!(expand("\\count0=-7\\the\\count0").unwrap(), "-7");
    }

    #[test]
    fn dimen_assignment_and_the() {
        assert_eq!(expand("\\dimen0=2.5pt\\the\\dimen0").unwrap(), "2.5pt");
        assert_eq!(expand("\\dimen0=1pt\\the\\dimen0").unwrap(), "1.0pt");
        assert_eq!(expand("\\dimen0=1in\\the\\dimen0").unwrap(), "72.26999pt");
    }

    #[test]
    fn skip_assignment_and_the() {
        assert_eq!(
            expand("\\skip0=1pt plus 2pt minus 0.5pt\\the\\skip0").unwrap(),
            "1.0pt plus 2.0pt minus 0.5pt"
        );
    }

    #[test]
    fn toks_assignment_and_the() {
        assert_eq!(expand("\\toks0={Hi}\\the\\toks0").unwrap(), "Hi");
    }

    #[test]
    fn the_in_edef_expands() {
        assert_eq!(
            expand("\\count0=7\\edef\\x{\\the\\count0}\\x").unwrap(),
            "7"
        );
    }

    // ---------- M1-11 组与作用域 ----------

    #[test]
    fn local_def_restored_at_group_end() {
        let src = "\\def\\a{X}\\begingroup\\def\\a{Y}\\a\\endgroup\\a";
        assert_eq!(expand(src).unwrap(), "YX");
    }

    #[test]
    fn global_def_persists() {
        let src = "\\def\\a{X}\\begingroup\\global\\def\\a{Y}\\a\\endgroup\\a";
        assert_eq!(expand(src).unwrap(), "YY");
    }

    #[test]
    fn gdef_is_global() {
        let src = "\\def\\a{X}\\begingroup\\gdef\\a{Y}\\a\\endgroup\\a";
        assert_eq!(expand(src).unwrap(), "YY");
    }

    #[test]
    fn count_local_and_global_scoping() {
        let local = "\\count0=1\\begingroup\\count0=2\\the\\count0\\endgroup\\the\\count0";
        assert_eq!(expand(local).unwrap(), "21");
        let global = "\\count0=1\\begingroup\\global\\count0=2\\endgroup\\the\\count0";
        assert_eq!(expand(global).unwrap(), "2");
    }

    #[test]
    fn begingroup_endgroup_primitives() {
        let src = "\\def\\a{X}\\begingroup\\def\\a{Y}\\a\\endgroup\\a";
        assert_eq!(expand(src).unwrap(), "YX");
    }

    #[test]
    fn nested_groups() {
        let src = "\\def\\a{X}\\begingroup\\begingroup\\def\\a{1}\\a\\endgroup\\a\\endgroup\\a";
        assert_eq!(expand(src).unwrap(), "1XX");
    }

    #[test]
    fn too_many_end_groups_errors() {
        assert!(expand("\\def\\a{X}\\a}").is_err());
    }

    #[test]
    fn conditional_inside_group_must_close() {
        // 组内开 \if 未闭合就 \endgroup → 错误
        assert!(expand("\\begingroup\\iftrue A\\endgroup").is_err());
    }

    // ---------- M2-6 展开吞吐基准（手动运行：cargo test -p ntex-core -- --ignored） ----------

    #[test]
    #[ignore]
    fn bytecode_vs_interpreter_throughput() {
        use std::time::Instant;

        // 高频宏调用语料：2000 个含参数宏调用 + 常量条件
        let mut src =
            String::from("\\def\\foo#1{#1X}\\def\\bar{\\iftrue Y\\else N\\fi}\\def\\run{");
        for _ in 0..2000 {
            src.push_str("\\foo{a}\\bar\\foo{b}\\bar");
        }
        src.push_str("}\\run");

        let run_track = |use_bytecode: bool| -> f64 {
            let mut e = if use_bytecode {
                Expander::new()
            } else {
                Expander::new_interpreter()
            };
            e.run_source("\\def\\__warm{1}").unwrap(); // 预热（代码路径加载）
            let t0 = Instant::now();
            e.run_source(&src).unwrap();
            let elapsed = t0.elapsed().as_secs_f64();
            e.output().len() as f64 / elapsed
        };

        // 预热各一次后正式计时（各 3 次取最大吞吐）
        let _ = run_track(true);
        let _ = run_track(false);
        let bc = (0..3).map(|_| run_track(true)).fold(0.0f64, f64::max);
        let ip = (0..3).map(|_| run_track(false)).fold(0.0f64, f64::max);

        eprintln!(
            "字节码吞吐：{bc:.0} token/s；解释器吞吐：{ip:.0} token/s；比值 {:.2}x",
            bc / ip
        );
        // 不设硬断言（CI 波动大），仅报告数字
    }

    // ---------- M3-2-2 内部参数 ----------

    #[test]
    fn param_assignment_and_the() {
        assert_eq!(expand("\\parindent 20pt\\the\\parindent").unwrap(), "20.0pt");
        assert_eq!(
            expand("\\baselineskip 10pt plus 2pt\\the\\baselineskip").unwrap(),
            "10.0pt plus 2.0pt"
        );
        assert_eq!(expand("\\lineskip 3pt\\the\\lineskip").unwrap(), "3.0pt");
        assert_eq!(
            expand("\\lineskiplimit -1pt\\the\\lineskiplimit").unwrap(),
            "-1.0pt"
        );
    }

    #[test]
    fn param_defaults() {
        assert_eq!(expand("\\the\\parindent").unwrap(), "0.0pt");
        assert_eq!(expand("\\the\\baselineskip").unwrap(), "12.0pt");
        assert_eq!(expand("\\the\\lineskip").unwrap(), "0.0pt");
        assert_eq!(expand("\\the\\lineskiplimit").unwrap(), "0.0pt");
    }

    #[test]
    fn param_local_scoped_at_group_end() {
        let src = "\\parindent 20pt\\begingroup\\parindent 30pt\\endgroup\\the\\parindent";
        assert_eq!(expand(src).unwrap(), "20.0pt");
    }

    #[test]
    fn param_global_scoped() {
        let src = "\\parindent 20pt\\begingroup\\global\\parindent 30pt\\endgroup\\the\\parindent";
        assert_eq!(expand(src).unwrap(), "30.0pt");
    }

    #[test]
    fn param_afterassignment_fires() {
        // \afterassignment 在参数赋值后触发（与寄存器一致）
        let src = "\\def\\x{Y}\\afterassignment\\x\\parindent 10pt\\the\\parindent";
        assert_eq!(expand(src).unwrap(), "Y10.0pt");
    }

    // ---------- M3-3 折行参数 ----------

    #[test]
    fn hsize_and_tolerance_assignment() {
        assert_eq!(expand("\\hsize 100pt\\the\\hsize").unwrap(), "100.0pt");
        assert_eq!(expand("\\tolerance 300\\the\\tolerance").unwrap(), "300");
        // 默认值（TeX initex）：\hsize=6.5in、\tolerance=10000
        assert!(expand("\\the\\tolerance").unwrap().ends_with("10000"));
    }

    // ---------- M3-4 字体 ----------

    /// 记录事件流的测试 sink（`font_selected` 事件用）。
    #[derive(Debug, Default)]
    struct EventSink {
        chars: Vec<char>,
        fonts: Vec<u32>,
    }

    impl TokenSink for EventSink {
        fn token(&mut self, tok: Token) -> Result<()> {
            if let Some(c) = tok.charcode().and_then(char::from_u32) {
                self.chars.push(c);
            }
            Ok(())
        }
        fn font_selected(&mut self, font: u32) -> Result<()> {
            self.fonts.push(font);
            Ok(())
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    /// 记录加载请求的测试加载器（每次加载返回递增 FontId）。
    /// 调用记录经 `Rc<RefCell>` 共享，移入 Expander 后仍可读取。
    type FontCall = (String, Option<i64>, Option<i64>);

    #[derive(Debug, Clone, Default)]
    struct MockLoader {
        calls: Rc<RefCell<Vec<FontCall>>>,
    }

    impl FontLoader for MockLoader {
        fn load(&mut self, name: &str, at: Option<i64>, scaled: Option<i64>) -> Result<u32> {
            let mut calls = self.calls.borrow_mut();
            calls.push((name.to_owned(), at, scaled));
            Ok(calls.len() as u32 - 1)
        }
    }

    /// 用 MockLoader 运行源码，返回 (输出字符, 字体选择事件, 加载请求)。
    fn font_run(src: &str) -> Result<(String, Vec<u32>, Vec<FontCall>)> {
        let loader = MockLoader::default();
        let calls = loader.calls.clone();
        let mut e = Expander::new();
        e.set_font_loader(Box::new(loader));
        let sink = EventSink::default();
        e.set_sink(Box::new(sink));
        e.run_source(src)?;
        let mut sink = e.take_sink();
        let sink = sink.as_any_mut().downcast_mut::<EventSink>().unwrap();
        let chars: String = sink.chars.iter().copied().collect();
        let fonts = sink.fonts.clone();
        let loaded = calls.borrow().clone();
        Ok((chars, fonts, loaded))
    }

    #[test]
    fn font_defines_selector_and_emits_selection() {
        let (chars, fonts, calls) = font_run("\\font\\foo=cmr10\\foo a").unwrap();
        assert_eq!(calls, vec![("cmr10".to_owned(), None, None)]);
        assert_eq!(fonts, vec![0], "执行 \\foo 应触发 font_selected");
        assert!(chars.contains('a'));
    }

    #[test]
    fn font_at_and_scaled_variants() {
        let src = r"\font\a=cmr10 at 12pt \font\b=cmr10 scaled 1200";
        let (_, _, calls) = font_run(src).unwrap();
        assert_eq!(
            calls,
            vec![
                ("cmr10".to_owned(), Some(12 * SP_PER_PT), None),
                ("cmr10".to_owned(), None, Some(1200)),
            ]
        );
    }

    #[test]
    fn font_equals_is_optional() {
        let (_, _, calls) = font_run("\\font\\foo cmr10").unwrap();
        assert_eq!(calls, vec![("cmr10".to_owned(), None, None)]);
    }

    #[test]
    fn font_without_loader_errors() {
        let mut e = Expander::new(); // 默认 NoFontLoader
        assert!(e.run_source("\\font\\foo=cmr10").is_err());
    }

    #[test]
    fn ifx_compares_font_meanings() {
        // 同一 cs 与自身相等（Font(0) == Font(0)）
        let out = font_run(r"\font\a=cmr10\ifx\a\a yes\else no\fi").unwrap().0;
        assert_eq!(out, "yes");
        // 两次加载得到不同 FontId → 不等
        let out = font_run(r"\font\a=cmr10\font\b=cmr10\ifx\a\b yes\else no\fi").unwrap().0;
        assert_eq!(out, "no");
    }

    // ---------- M3 收尾（RFC-3）：VFS 副作用原语 ----------

    /// 运行源码（MemVfs 后端），返回 (输出字符串, VFS)。副作用用例不跑双轨。
    fn expand_vfs(src: &str, mut vfs: MemVfs) -> Result<(String, MemVfs)> {
        let mut e = Expander::new();
        e.set_vfs(Box::new(vfs));
        e.run_source(src)?;
        let out = e
            .output()
            .iter()
            .map(|t| t.charcode().and_then(char::from_u32).unwrap_or('\u{FFFD}'))
            .collect();
        let mut vfs = e.take_vfs();
        let vfs = vfs
            .as_any_mut()
            .downcast_mut::<MemVfs>()
            .ok_or_else(|| Error::internal("测试 VFS 应为 MemVfs"))?;
        Ok((out, vfs.clone()))
    }

    #[test]
    fn input_reads_file_from_vfs() {
        let mut vfs = MemVfs::new();
        vfs.insert("ch1.tex", "Chapter One");
        let (out, _) = expand_vfs("\\input{ch1}", vfs).unwrap();
        assert_eq!(out, "Chapter One");
    }

    #[test]
    fn input_falls_back_to_tex_extension() {
        let mut vfs = MemVfs::new();
        vfs.insert("ch2.tex", "Ch2");
        let (out, _) = expand_vfs("\\input ch2", vfs).unwrap();
        assert_eq!(out, "Ch2");
    }

    #[test]
    fn input_nests_and_returns() {
        let mut vfs = MemVfs::new();
        vfs.insert("a.tex", "A\\input{b}B");
        vfs.insert("b.tex", "X");
        let (out, _) = expand_vfs("\\input{a}", vfs).unwrap();
        assert_eq!(out, "AXB");
    }

    #[test]
    fn input_missing_file_errors() {
        let vfs = MemVfs::new();
        assert!(expand_vfs("\\input{nope}", vfs).is_err());
    }

    #[test]
    fn immediate_write_appends() {
        let vfs = MemVfs::new();
        let (_, vfs) = expand_vfs(
            "\\newwrite\\f\\immediate\\openout\\f=out.txt\\immediate\\write\\f{abc}\\closeout\\f",
            vfs,
        )
        .unwrap();
        assert_eq!(vfs.get("out.txt"), Some(b"abc\n".as_slice()));
    }

    #[test]
    fn write_defers_until_end() {
        let vfs = MemVfs::new();
        // 无 \immediate：\write 入队，\end 收尾统一 flush
        let (_, vfs) = expand_vfs(
            "\\newwrite\\f\\openout\\f=out.txt\\write\\f{abc}\\write\\f{def}\\end",
            vfs,
        )
        .unwrap();
        assert_eq!(vfs.get("out.txt"), Some(b"abc\ndef\n".as_slice()));
    }

    #[test]
    fn write_expands_the_at_write_time() {
        let vfs = MemVfs::new();
        // \write 时展开 \the\count0 与宏（TeX 语义：写文件时展开）
        let (_, vfs) = expand_vfs(
            "\\count0=42\\def\\mark{X}\\newwrite\\f\\openout\\f=o.txt\\write\\f{\\the\\count0\\mark}\\end",
            vfs,
        )
        .unwrap();
        assert_eq!(vfs.get("o.txt"), Some(b"42X\n".as_slice()));
    }

    #[test]
    fn write_to_unopened_stream_errors() {
        let vfs = MemVfs::new();
        assert!(expand_vfs("\\write0{abc}\\end", vfs).is_err());
    }

    #[test]
    fn read_line_defines_cs() {
        let mut vfs = MemVfs::new();
        vfs.insert("data.txt", "Hello\nWorld\n");
        let (out, _) = expand_vfs(
            "\\newread\\r\\openin\\r=data.txt\\read\\r to \\line\\line",
            vfs,
        )
        .unwrap();
        assert_eq!(out, "Hello");
    }

    #[test]
    fn read_eof_errors() {
        let mut vfs = MemVfs::new();
        vfs.insert("empty.txt", "");
        assert!(
            expand_vfs("\\newread\\r\\openin\\r=empty.txt\\read\\r to \\line", vfs)
                .is_err()
        );
    }

    #[test]
    fn write18_shell_escape_rejected() {
        let vfs = MemVfs::new();
        assert!(expand_vfs("\\write18{echo hi}\\end", vfs).is_err());
    }
}
