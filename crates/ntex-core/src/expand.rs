//! 展开引擎主循环（M1-6）。
//!
//! 职责：消费 token 流（源码 / 宏展开 / token 列表），
//! 展开可展开项（宏、`\expandafter`、`\noexpand`），
//! 执行不可展开原语（`\def`/`\let`/`\catcode` 等），
//! 其余 token 原样输出。
//!
//! M1 范围说明（后续里程碑补齐）：
//! - 无分隔参数匹配已实现；分隔参数（M1-8）、`\futurelet`/`\aftergroup` 等（M1-7）未实现；
//! - 条件原语 `\if*`（M1-9）、寄存器 `\count` 等（M1-10）、组作用域（M1-11）未实现；
//! - 空行 → `\par` 语义未实现（M1-14 修）。

use std::sync::Arc;

use crate::catcode::{Catcode, CatcodeTable};
use crate::eqtb::{EqSlot, Eqtb, Primitive};
use crate::error::{Error, Result};
use crate::input::scan_token;
use crate::intern::InternTable;
use crate::macrodef::{MacroDef, ParamSpec, TokenArray};
use crate::token::{Token, TokenKind};

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
    /// token 列表帧：`(token, noexpand 标记)`。
    TokenList {
        items: Arc<[(Token, bool)]>,
        pos: usize,
    },
}

/// 展开引擎。
#[derive(Debug)]
pub struct Expander {
    intern: InternTable,
    eqtb: Eqtb,
    catcodes: CatcodeTable,
    stack: Vec<InputFrame>,
    output: Vec<Token>,
    /// 读取下限：`fetch` 只允许从下标 >= 该值的帧读取；
    /// 用于划分子展开（`\edef`/`\expandafter` 区域）的边界。
    read_floor: usize,
}

impl Expander {
    /// 创建引擎并注册 M1 内建原语。
    pub fn new() -> Self {
        let mut e = Self {
            intern: InternTable::new(),
            eqtb: Eqtb::new(),
            catcodes: CatcodeTable::new(),
            stack: Vec::new(),
            output: Vec::new(),
            read_floor: 0,
        };
        e.register_builtins();
        e
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
    pub fn run(&mut self) -> Result<()> {
        while self.process_one()? {}
        Ok(())
    }

    /// 单步处理一个 token；返回 false 表示输入耗尽。
    fn process_one(&mut self) -> Result<bool> {
        match self.fetch()? {
            None => Ok(false),
            Some((tok, noexpand)) => {
                if noexpand {
                    // \noexpand：临时不可展开，原样输出
                    self.output.push(tok);
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
                        self.output.push(Token::char(catcode, charcode));
                        Ok(())
                    }
                    EqSlot::Macro(m) => {
                        let def = m.value.clone();
                        let args = if def.params.num_params > 0 {
                            self.collect_args(&def)?
                        } else {
                            Vec::new()
                        };
                        self.stack.push(InputFrame::Macro {
                            body: def.body.clone(),
                            pos: 0,
                            args,
                        });
                        Ok(())
                    }
                    EqSlot::Primitive(p) => self.exec_primitive(p),
                }
            }
            _ => {
                self.output.push(tok);
                Ok(())
            }
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
                InputFrame::TokenList { items, pos } => {
                    if *pos >= items.len() {
                        self.stack.pop();
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
            // M1-11 组作用域实现前，\gdef 与 \def 相同
            Primitive::Gdef => self.exec_def(false),
            Primitive::Let => self.exec_let(),
            Primitive::Catcode => self.exec_catcode(),
            Primitive::End => {
                self.stack.clear();
                Ok(())
            }
        }
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

        let def = MacroDef {
            params: ParamSpec {
                num_params,
                long: false,
                delimiter: None,
            },
            body,
        };
        self.eqtb.define_macro(csid, def);
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
    /// 通过临时提升 `read_floor` 划定区域边界，防止越过该区域读取外层输入。
    fn expand_region(&mut self, tokens: Vec<Token>) -> Result<Vec<Token>> {
        let saved_floor = self.read_floor;
        let depth = self.stack.len();
        self.read_floor = depth;
        let saved = std::mem::take(&mut self.output);

        let items: Vec<(Token, bool)> = tokens.into_iter().map(|t| (t, false)).collect();
        self.stack.push(InputFrame::TokenList {
            items: Arc::from(items),
            pos: 0,
        });
        while self.process_one()? {}

        let result = std::mem::take(&mut self.output);
        self.output = saved;
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
        Ok(())
    }

    /// `\catcode<byte>=<num>`：修改全局 catcode 表。
    fn exec_catcode(&mut self) -> Result<()> {
        let byte = self.scan_number()?;
        let byte = u8::try_from(byte).map_err(|_| Error::invalid_input("\\catcode 字符码越界"))?;
        self.expect_equals()?;
        let code = self.scan_number()?;
        let cat = Catcode::from_u8(
            u8::try_from(code).map_err(|_| Error::invalid_input("catcode 必须在 0..=15"))?,
        )
        .ok_or_else(|| Error::invalid_input("catcode 必须在 0..=15"))?;
        self.catcodes.set(byte, cat);
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

    /// 扫描十进制整数（M1 简化版：字符 token 的字符码为数字即取）。
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
        let mut val: i64 = 0;
        let mut any = false;
        loop {
            let tok = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("数字扫描到输入末尾"))?
                .0;
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
        Ok(if neg { -val } else { val })
    }

    /// 跳过前导空格 token。
    fn skip_spaces(&mut self) -> Result<()> {
        loop {
            let tok = self
                .fetch()?
                .ok_or_else(|| Error::invalid_input("扫描到输入末尾"))?
                .0;
            if tok.catcode() != Some(Catcode::Space) {
                self.unread(tok);
                return Ok(());
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
        const BUILTINS: [(&str, Primitive); 9] = [
            ("def", Primitive::Def),
            ("edef", Primitive::Edef),
            ("gdef", Primitive::Gdef),
            ("let", Primitive::Let),
            ("relax", Primitive::Relax),
            ("expandafter", Primitive::Expandafter),
            ("noexpand", Primitive::Noexpand),
            ("catcode", Primitive::Catcode),
            ("end", Primitive::End),
        ];
        for (name, prim) in BUILTINS {
            let csid = self.intern.intern(name);
            self.eqtb.set_primitive(csid, prim);
        }
    }

    // ---------- 只读访问（测试/上层用） ----------

    pub fn intern(&self) -> &InternTable {
        &self.intern
    }

    pub fn eqtb(&self) -> &Eqtb {
        &self.eqtb
    }

    pub fn output(&self) -> &[Token] {
        &self.output
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 运行源码并输出为字符串（字符 token 按 char 输出；非字符标记为 `�`）。
    fn expand(src: &str) -> Result<String> {
        let mut e = Expander::new();
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
        // 注意：数字扫描器与真实 TeX 一样会预读紧邻 token（无空格写法会先按旧表把
        // `\abc` 扫描成控制序列，导致 "Undefined control sequence"，与 pdfTeX 行为一致），
        // 因此用空格分隔数字与后续输入。
        assert_eq!(expand("\\catcode92=12 \\abc").unwrap(), " \\abc");
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
        assert_eq!(
            expand("\\def\\wrap#1{[#1]}\\wrap{a{b}c}").unwrap(),
            "[a{b}c]"
        );
    }

    #[test]
    fn empty_argument() {
        assert_eq!(expand("\\def\\wrap#1{[#1]}\\wrap{}").unwrap(), "[]");
    }
}
