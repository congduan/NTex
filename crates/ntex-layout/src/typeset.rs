//! 排版器（M3-2）：模式状态机 + token→节点构建。
//!
//! 消费 ntex-core 展开引擎的排版事件（[`TokenSink`]），维护 TeX 模式状态机：
//!
//! - [`Mode::Vertical`]：垂直列表（页面主列表 / vbox 内容）；字符触发段落；
//! - [`Mode::Horizontal`]：段落水平列表；`\par` 或输入结束将其封装为 hbox；
//! - [`Mode::RestrictedHorizontal`]：`\hbox{...}` 内容。
//!
//! 排版原语参数（`\hskip`/`\vskip`/`\kern`/`\penalty`/`\hrule`/`\vrule`）
//! 由 VM 扫描，本层收到结构化结果；`\hbox`/`\vbox`/`\vtop`/`\par`/`\indent`
//! 由本层解释。内部参数（`\parindent`/`\baselineskip`/`\lineskip`/`\lineskiplimit`）
//! 经 `param_changed` 事件镜像，随组作用域快照/恢复。
//!
//! M3-2 范围说明（后续子步补齐）：
//! - 字体度量（M3-4 TFM）前，字符维度由 [`Typesetter::with_metrics`] 提供，默认全零；
//! - 词间空白（M3-3）未实现，空格 token 被忽略；
//! - `\hbox to <glue>` / `\hbox spread <glue>` 规格暂拒。

use ntex_core::error::{Error, Result};
use ntex_core::expand::Expander;
use ntex_core::param::{ParamKind, ParamValue, Params};
use ntex_core::register::Glue;
use ntex_core::token::Token;
use ntex_core::{Primitive, TokenSink};

use crate::node::{BoxKind, BoxNode, FontId, Node};

/// 模式（TeX 模式状态机的 M3-2 子集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// 垂直模式：垂直列表（页面主列表 / vbox 内容）；字符触发段落。
    Vertical,
    /// 水平模式：段落水平列表；`\par` / 输入结束封装为 hbox。
    Horizontal,
    /// 受限水平模式：`\hbox{...}` 内容。
    RestrictedHorizontal,
}

/// 待封装盒子种类（`\hbox`/`\vbox`/`\vtop` 的下一个组）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingBox {
    HBox,
    VBox,
    VTop,
}

impl PendingBox {
    fn is_vertical(self) -> bool {
        matches!(self, Self::VBox | Self::VTop)
    }
}

/// 组上下文（group_begin 压栈，group_end 弹出）。
#[derive(Debug)]
struct GroupCtx {
    /// 本组是否为盒子内容（`\hbox`/`\vbox`/`\vtop` 紧邻的组）。
    box_kind: Option<PendingBox>,
}

/// 字符度量函数：`(width, height, depth)`，单位 sp。
pub type MetricsFn = fn(FontId, u32) -> (i64, i64, i64);

/// 词间空白胶水函数（空格 token → 胶水；M3-4 TFM 前由调用方提供）。
pub type SpaceFn = fn(FontId) -> Glue;

/// 节点构建 sink：把 VM 排版事件转成节点列表。
#[derive(Debug)]
struct NodeBuilder {
    /// 列表栈（栈顶 = 当前列表；栈底 = 主垂直列表）。
    /// 栈内元素按入栈顺序：盒子内容、段落。`[list_modes]` 与之并行，
    /// 记录每个列表的模式（段落 = Horizontal，hbox 内容 = RestrictedHorizontal，
    /// vbox 内容 / 主列表 = Vertical）。作用域组（非盒子）不压列表。
    lists: Vec<Vec<Node>>,
    list_modes: Vec<Mode>,
    /// 组上下文栈（与列表栈独立：作用域组只压 ctx）。
    groups: Vec<GroupCtx>,
    /// 等待下一个组的盒子种类。
    pending_box: Option<PendingBox>,
    /// 内部参数镜像（随 `param_changed` 事件更新，组作用域快照/恢复）。
    params: Params,
    /// 组开始时的参数快照（group_end 恢复）。
    param_stack: Vec<Params>,
    /// `\noindent`：下一个段落不缩进。
    noindent_next: bool,
    /// 字符度量（M3-4 TFM 前占位）。
    metrics: MetricsFn,
    /// 词间空白胶水（M3-4 TFM 前占位）。
    space: SpaceFn,
}

impl NodeBuilder {
    fn new(metrics: MetricsFn, space: SpaceFn) -> Self {
        Self {
            lists: vec![Vec::new()],
            list_modes: vec![Mode::Vertical],
            groups: Vec::new(),
            pending_box: None,
            params: Params::default(),
            param_stack: Vec::new(),
            noindent_next: false,
            metrics,
            space,
        }
    }

    fn mode(&self) -> Mode {
        *self.list_modes.last().expect("列表栈非空")
    }

    fn append(&mut self, node: Node) {
        self.lists.last_mut().expect("列表栈非空").push(node);
    }

    /// 结束开放段落：把水平列表封装为 hbox 并追加到上层列表。
    fn close_paragraph(&mut self) {
        let children = self.lists.pop().expect("段落列表");
        self.list_modes.pop();
        self.push_box(Node::Box(BoxNode::new_hbox(children)));
    }

    /// 封装盒子内容（group_end 用）。
    fn package_box(&mut self, kind: PendingBox) {
        let children = self.lists.pop().expect("盒子列表");
        self.list_modes.pop();
        let node = match kind {
            PendingBox::HBox => Node::Box(BoxNode::new_hbox(children)),
            PendingBox::VBox => Node::Box(BoxNode::new_vbox(children)),
            PendingBox::VTop => {
                // \vtop：维度同 vbox，参考点移到首行基线（shift 待 M3-5 对 DVI 校准）。
                let mut b = BoxNode::new_vbox(children);
                b.shift = b.height;
                Node::Box(b)
            }
        };
        self.push_box(node);
    }

    /// 追加盒子到当前列表；垂直列表中前驱为盒子时插入 interline glue
    /// （tex.web `append_to_vlist`：d = \baselineskip − (depth 前 + height 新)，
    /// d < \lineskiplimit 用 \lineskip，否则用宽度调整为 d 的 \baselineskip）。
    fn push_box(&mut self, node: Node) {
        if self.mode() == Mode::Vertical {
            if let Some(Node::Box(prev)) = self.lists.last().and_then(|l| l.last()) {
                let height = match &node {
                    Node::Box(b) => b.height,
                    _ => 0,
                };
                let d = self.params.baselineskip.width - (prev.depth + height);
                let g = if d < self.params.lineskiplimit {
                    self.params.lineskip
                } else {
                    let mut g = self.params.baselineskip;
                    g.width = d;
                    g
                };
                self.append(Node::Glue {
                    width: g.width,
                    stretch: g.stretch,
                    shrink: g.shrink,
                });
            }
        }
        self.append(node);
    }

    /// 插入段落缩进（TeX `new_graf`）：\parindent>0 空盒，<0 kern，=0 无；
    /// `\noindent` 抑制。
    fn insert_indent(&mut self) {
        if self.noindent_next {
            self.noindent_next = false;
            return;
        }
        let ind = self.params.parindent;
        if ind > 0 {
            self.append(Node::Box(BoxNode {
                kind: BoxKind::HBox,
                width: ind,
                height: 0,
                depth: 0,
                shift: 0,
                children: Vec::new(),
            }));
        } else if ind < 0 {
            self.append(Node::Kern { width: ind });
        }
    }

    /// 字符 token → Char 节点；非字符（控制序列等）返回 None。
    fn char_node(&self, tok: Token) -> Option<Node> {
        let charcode = tok.charcode()?;
        let (w, h, d) = (self.metrics)(FontId(0), charcode);
        Some(Node::Char {
            font: FontId(0),
            charcode,
            width: w,
            height: h,
            depth: d,
        })
    }
}

impl TokenSink for NodeBuilder {
    fn token(&mut self, tok: Token) -> Result<()> {
        // 空格（cat 10）：垂直模式忽略；水平模式转词间空白胶水
        // （行首或胶水/惩罚之后忽略，TeX spacer 语义）。
        if tok.catcode() == Some(ntex_core::Catcode::Space) {
            match self.mode() {
                Mode::Vertical => {}
                Mode::Horizontal | Mode::RestrictedHorizontal => {
                    let ignorable = match self.lists.last().and_then(|l| l.last()) {
                        None => true,
                        Some(Node::Glue { .. } | Node::Penalty { .. }) => true,
                        Some(_) => false,
                    };
                    if !ignorable {
                        let g = (self.space)(FontId(0));
                        self.append(Node::Glue {
                            width: g.width,
                            stretch: g.stretch,
                            shrink: g.shrink,
                        });
                    }
                }
            }
            return Ok(());
        }
        let Some(node) = self.char_node(tok) else {
            return Ok(()); // 控制序列等无可排版语义
        };
        match self.mode() {
            Mode::Vertical => {
                // 垂直模式字符触发段落（TeX new_graf）
                self.lists.push(Vec::new());
                self.list_modes.push(Mode::Horizontal);
                self.insert_indent();
                self.append(node);
            }
            Mode::Horizontal | Mode::RestrictedHorizontal => self.append(node),
        }
        Ok(())
    }

    fn group_begin(&mut self) -> Result<()> {
        let kind = self.pending_box.take();
        self.groups.push(GroupCtx { box_kind: kind });
        self.param_stack.push(self.params);
        if let Some(k) = kind {
            let new_mode = match k {
                PendingBox::HBox => Mode::RestrictedHorizontal,
                PendingBox::VBox | PendingBox::VTop => Mode::Vertical,
            };
            self.lists.push(Vec::new());
            self.list_modes.push(new_mode);
        }
        Ok(())
    }

    fn group_end(&mut self) -> Result<()> {
        let ctx = self
            .groups
            .pop()
            .ok_or_else(|| Error::internal("group_end 无配对 group_begin"))?;
        // 先恢复参数镜像（与 VM 的 save_stack 恢复对齐），随后的缩进/interline 用外层值
        if let Some(prev) = self.param_stack.pop() {
            self.params = prev;
        }
        // 垂直盒子内容结束时，开放段落先封装（\vbox{a} → vbox[hbox(a)]）
        if ctx.box_kind.is_some_and(PendingBox::is_vertical) && self.mode() == Mode::Horizontal {
            self.close_paragraph();
        }
        if let Some(kind) = ctx.box_kind {
            self.package_box(kind);
        }
        Ok(())
    }

    fn primitive(&mut self, prim: Primitive) -> Result<()> {
        match prim {
            Primitive::HBox => self.pending_box = Some(PendingBox::HBox),
            Primitive::VBox => self.pending_box = Some(PendingBox::VBox),
            Primitive::VTop => self.pending_box = Some(PendingBox::VTop),
            Primitive::Par => match self.mode() {
                Mode::Horizontal => self.close_paragraph(),
                // 垂直模式 \par 无操作；受限水平模式拒绝
                Mode::Vertical => {}
                Mode::RestrictedHorizontal => {
                    return Err(Error::invalid_input(
                        "\\par 不允许出现在受限水平模式（\\hbox 内）",
                    ));
                }
            },
            Primitive::Indent => match self.mode() {
                Mode::Vertical => {
                    // 垂直模式 \indent 强制开段并缩进（TeX：new_graf）
                    self.lists.push(Vec::new());
                    self.list_modes.push(Mode::Horizontal);
                    self.insert_indent();
                }
                Mode::Horizontal | Mode::RestrictedHorizontal => self.insert_indent(),
            },
            Primitive::NoIndent => {
                // 垂直模式：下一个段落不缩进；水平模式无操作
                if self.mode() == Mode::Vertical {
                    self.noindent_next = true;
                }
            }
            // 参数扫描型原语经 glue/kern/penalty/rule 事件处理
            _ => {}
        }
        Ok(())
    }

    fn param_changed(&mut self, kind: ParamKind, value: ParamValue) -> Result<()> {
        self.params.set(kind, value);
        Ok(())
    }

    fn glue(&mut self, g: Glue) -> Result<()> {
        self.append(Node::Glue {
            width: g.width,
            stretch: g.stretch,
            shrink: g.shrink,
        });
        Ok(())
    }

    fn kern(&mut self, width: i64) -> Result<()> {
        self.append(Node::Kern { width });
        Ok(())
    }

    fn penalty(&mut self, penalty: i64) -> Result<()> {
        self.append(Node::Penalty { penalty });
        Ok(())
    }

    fn rule(&mut self, width: i64, height: i64, depth: i64) -> Result<()> {
        self.append(Node::Rule { width, height, depth });
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// 排版器：VM token 流 → 节点树（主垂直列表）。
pub struct Typesetter {
    expander: Expander,
    metrics: MetricsFn,
    space: SpaceFn,
}

impl Typesetter {
    /// 创建排版器（字符维度/词间距默认全零，M3-4 TFM 前占位）。
    pub fn new() -> Self {
        Self::with_metrics(|_, _| (0, 0, 0))
    }

    /// 创建排版器并指定字符度量函数（词间距默认全零）。
    pub fn with_metrics(metrics: MetricsFn) -> Self {
        Self {
            expander: Expander::new(),
            metrics,
            space: |_| Glue {
                width: 0,
                stretch: 0,
                shrink: 0,
            },
        }
    }

    /// 指定词间空白胶水函数（空格 token → 胶水）。
    pub fn with_space(mut self, space: SpaceFn) -> Self {
        self.space = space;
        self
    }

    /// 排版源码，返回主垂直列表节点。
    pub fn typeset(&mut self, text: &str) -> Result<Vec<Node>> {
        self.expander
            .set_sink(Box::new(NodeBuilder::new(self.metrics, self.space)));
        self.expander.run_source(text)?;
        self.finish()
    }

    /// 排版字节源码。
    pub fn typeset_bytes(&mut self, bytes: impl Into<Vec<u8>>) -> Result<Vec<Node>> {
        self.expander
            .set_sink(Box::new(NodeBuilder::new(self.metrics, self.space)));
        self.expander.feed_source(bytes);
        self.expander.run()?;
        self.finish()
    }

    /// 运行结束收尾：关闭开放段落、校验盒子/组闭合，取回主垂直列表。
    fn finish(&mut self) -> Result<Vec<Node>> {
        let mut sink = self.expander.take_sink();
        let builder = sink
            .as_any_mut()
            .downcast_mut::<NodeBuilder>()
            .ok_or_else(|| Error::internal("typesetter 安装了 NodeBuilder"))?;
        if builder.pending_box.is_some() {
            return Err(Error::invalid_input("\\hbox/\\vbox 后缺少组"));
        }
        if !builder.groups.is_empty() {
            return Err(Error::invalid_input("组未闭合（缺少 }）"));
        }
        if builder.mode() == Mode::Horizontal {
            builder.close_paragraph();
        }
        let mut lists = std::mem::take(&mut builder.lists);
        debug_assert_eq!(lists.len(), 1, "收尾后应只剩主列表");
        Ok(lists.pop().expect("主列表"))
    }
}

impl Default for Typesetter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::BoxKind;
    use ntex_core::SP_PER_PT;

    fn metrics(_font: FontId, ch: u32) -> (i64, i64, i64) {
        // 测试度量：宽 1000sp + 码点，高 6000，深 1500。
        (1000 + i64::from(ch), 6000, 1500)
    }

    fn typeset(text: &str) -> Result<Vec<Node>> {
        Typesetter::with_metrics(metrics).typeset(text)
    }

    fn as_box(n: &Node) -> &BoxNode {
        match n {
            Node::Box(b) => b,
            other => panic!("预期 Box，得到 {other:?}"),
        }
    }

    fn as_char(n: &Node) -> u32 {
        match n {
            Node::Char { charcode, .. } => *charcode,
            other => panic!("预期 Char，得到 {other:?}"),
        }
    }

    #[test]
    fn hbox_of_chars() {
        let main = typeset(r"\hbox{ab}").unwrap();
        assert_eq!(main.len(), 1);
        let b = as_box(&main[0]);
        assert_eq!(b.kind, BoxKind::HBox);
        assert_eq!(b.children.len(), 2);
        // 宽度 = 度量之和（a=1097, b=1098，1000 + 码点）
        assert_eq!(b.width, 1097 + 1098);
        assert_eq!(b.height, 6000);
        assert_eq!(b.depth, 1500);
    }

    #[test]
    fn paragraph_closure_by_par() {
        // 两段之间插入 interline glue：d = 12pt − (depth 1500 + height 6000)
        let main = typeset(r"ab\par cd").unwrap();
        assert_eq!(main.len(), 3);
        let p1 = as_box(&main[0]);
        assert_eq!(p1.children.len(), 2);
        assert_eq!(as_char(&p1.children[0]), b'a' as u32);
        match &main[1] {
            Node::Glue { width, .. } => {
                assert_eq!(*width, 12 * SP_PER_PT - (1500 + 6000));
            }
            other => panic!("预期 interline Glue，得到 {other:?}"),
        }
        let p2 = as_box(&main[2]);
        assert_eq!(as_char(&p2.children[0]), b'c' as u32);
    }

    #[test]
    fn paragraph_closed_at_eof() {
        let main = typeset("ab").unwrap();
        assert_eq!(main.len(), 1);
        assert_eq!(as_box(&main[0]).children.len(), 2);
    }

    #[test]
    fn empty_input_gives_empty_main_list() {
        assert!(typeset("").unwrap().is_empty());
    }

    #[test]
    fn vbox_builds_vertical_list() {
        let main = typeset(r"\vbox{\hbox{a}\vskip 10pt\hbox{b}}").unwrap();
        assert_eq!(main.len(), 1);
        let v = as_box(&main[0]);
        assert_eq!(v.kind, BoxKind::VBox);
        assert_eq!(v.children.len(), 3);
        assert!(matches!(v.children[0], Node::Box(_)));
        match &v.children[1] {
            Node::Glue { width, .. } => assert_eq!(*width, 10 * SP_PER_PT),
            other => panic!("预期 Glue，得到 {other:?}"),
        }
        assert!(matches!(v.children[2], Node::Box(_)));
    }

    #[test]
    fn nested_hbox() {
        let main = typeset(r"\hbox{a\hbox{b}c}").unwrap();
        let outer = as_box(&main[0]);
        assert_eq!(outer.children.len(), 3);
        assert_eq!(as_char(&outer.children[0]), b'a' as u32);
        assert!(matches!(outer.children[1], Node::Box(_)));
        assert_eq!(as_char(&outer.children[2]), b'c' as u32);
    }

    #[test]
    fn scoping_group_does_not_create_box() {
        let main = typeset(r"{\def\x{ab}\x}").unwrap();
        assert_eq!(main.len(), 1);
        assert_eq!(as_box(&main[0]).children.len(), 2);
    }

    #[test]
    fn kern_penalty_rule_in_hbox() {
        let main = typeset(r"\hbox{a\kern 10pt\penalty -50\hrule height 5pt depth 2pt width 100pt}")
            .unwrap();
        let b = as_box(&main[0]);
        assert_eq!(b.children.len(), 4);
        match &b.children[1] {
            Node::Kern { width } => assert_eq!(*width, 10 * SP_PER_PT),
            other => panic!("预期 Kern，得到 {other:?}"),
        }
        match &b.children[2] {
            Node::Penalty { penalty } => assert_eq!(*penalty, -50),
            other => panic!("预期 Penalty，得到 {other:?}"),
        }
        match &b.children[3] {
            Node::Rule { width, height, depth } => {
                assert_eq!(*width, 100 * SP_PER_PT);
                assert_eq!(*height, 5 * SP_PER_PT);
                assert_eq!(*depth, 2 * SP_PER_PT);
            }
            other => panic!("预期 Rule，得到 {other:?}"),
        }
    }

    #[test]
    fn vskip_appends_to_vertical_list() {
        let main = typeset(r"\vskip 10pt").unwrap();
        assert_eq!(main.len(), 1);
        match &main[0] {
            Node::Glue { width, .. } => assert_eq!(*width, 10 * SP_PER_PT),
            other => panic!("预期 Glue，得到 {other:?}"),
        }
    }

    #[test]
    fn vtop_gets_shift() {
        let main = typeset(r"\vtop{\hbox{a}\hbox{b}}").unwrap();
        let b = as_box(&main[0]);
        assert_eq!(b.kind, BoxKind::VBox);
        assert_eq!(b.shift, b.height);
    }

    #[test]
    fn par_in_hbox_is_rejected() {
        assert!(typeset(r"\hbox{a\par}").is_err());
    }

    #[test]
    fn unclosed_box_group_is_rejected() {
        assert!(typeset(r"\hbox{a").is_err());
    }

    #[test]
    fn box_spec_to_is_rejected() {
        assert!(typeset(r"\hbox to 5pt{a}").is_err());
    }

    #[test]
    fn empty_hbox() {
        let main = typeset(r"\hbox{}").unwrap();
        let b = as_box(&main[0]);
        assert!(b.children.is_empty());
        assert_eq!(b.width, 0);
        assert_eq!(b.height, 0);
        assert_eq!(b.depth, 0);
    }

    #[test]
    fn expansion_inside_hbox() {
        // 宏展开、寄存器、\the 在盒子内容里正常工作
        let main = typeset(r"\hbox{\def\x{xy}\x\count0=7\the\count0}").unwrap();
        let b = as_box(&main[0]);
        let chars: Vec<u32> = b.children.iter().map(as_char).collect();
        assert_eq!(chars, vec![b'x' as u32, b'y' as u32, b'7' as u32]);
    }

    // ---------- M3-2-2 段落：缩进 / 行间胶水 ----------

    fn as_glue_width(n: &Node) -> i64 {
        match n {
            Node::Glue { width, .. } => *width,
            other => panic!("预期 Glue，得到 {other:?}"),
        }
    }

    #[test]
    fn indent_forces_paragraph_with_box() {
        let main = typeset(r"\parindent 20pt\indent a").unwrap();
        assert_eq!(main.len(), 1);
        let para = as_box(&main[0]);
        assert_eq!(para.children.len(), 2);
        match &para.children[0] {
            Node::Box(b) => assert_eq!(b.width, 20 * SP_PER_PT),
            other => panic!("预期缩进空盒，得到 {other:?}"),
        }
        assert_eq!(as_char(&para.children[1]), b'a' as u32);
    }

    #[test]
    fn automatic_parindent_on_paragraph_start() {
        let main = typeset(r"\parindent 10pt ab").unwrap();
        let para = as_box(&main[0]);
        assert_eq!(para.children.len(), 3);
        match &para.children[0] {
            Node::Box(b) => assert_eq!(b.width, 10 * SP_PER_PT),
            other => panic!("预期缩进空盒，得到 {other:?}"),
        }
    }

    #[test]
    fn noindent_suppresses_indent() {
        let main = typeset(r"\parindent 10pt\noindent ab").unwrap();
        let para = as_box(&main[0]);
        assert_eq!(para.children.len(), 2); // 无缩进盒
        assert_eq!(as_char(&para.children[0]), b'a' as u32);
    }

    #[test]
    fn negative_parindent_becomes_kern() {
        let main = typeset(r"\parindent -5pt ab").unwrap();
        let para = as_box(&main[0]);
        match &para.children[0] {
            Node::Kern { width } => assert_eq!(*width, -5 * SP_PER_PT),
            other => panic!("预期 Kern，得到 {other:?}"),
        }
    }

    #[test]
    fn indent_inside_hbox() {
        let main = typeset(r"\parindent 20pt\hbox{\indent a}").unwrap();
        let b = as_box(&main[0]);
        assert_eq!(b.children.len(), 2);
        match &b.children[0] {
            Node::Box(ib) => assert_eq!(ib.width, 20 * SP_PER_PT),
            other => panic!("预期缩进空盒，得到 {other:?}"),
        }
    }

    #[test]
    fn interline_uses_custom_baselineskip() {
        // \baselineskip 8pt：d = 8pt − (1500 + 6000)
        let main = typeset(r"\baselineskip 8pt ab\par cd").unwrap();
        assert_eq!(main.len(), 3);
        assert_eq!(as_glue_width(&main[1]), 8 * SP_PER_PT - (1500 + 6000));
    }

    #[test]
    fn interline_uses_lineskip_when_below_limit() {
        // d = 1pt − 7500 < 0 < \lineskiplimit 100pt → 用 \lineskip 3pt
        let src = r"\baselineskip 1pt\lineskip 3pt\lineskiplimit 100pt ab\par cd";
        let main = typeset(src).unwrap();
        assert_eq!(main.len(), 3);
        assert_eq!(as_glue_width(&main[1]), 3 * SP_PER_PT);
    }

    #[test]
    fn param_scoped_affects_indent() {
        // 组内 \parindent 30pt 只影响组内段落；组外恢复 20pt
        let src = r"\parindent 20pt{\parindent 30pt ab\par}cd";
        let main = typeset(src).unwrap();
        assert_eq!(main.len(), 3); // 段1 + interline + 段2
        let p1 = as_box(&main[0]);
        match &p1.children[0] {
            Node::Box(b) => assert_eq!(b.width, 30 * SP_PER_PT),
            other => panic!("预期 30pt 缩进，得到 {other:?}"),
        }
        let p2 = as_box(&main[2]);
        match &p2.children[0] {
            Node::Box(b) => assert_eq!(b.width, 20 * SP_PER_PT),
            other => panic!("预期 20pt 缩进，得到 {other:?}"),
        }
    }

    #[test]
    fn vbox_lines_get_interline_glue() {
        let main = typeset(r"\vbox{\hbox{a}\hbox{b}}").unwrap();
        let v = as_box(&main[0]);
        assert_eq!(v.children.len(), 3); // hbox + interline glue + hbox
        assert!(matches!(v.children[1], Node::Glue { .. }));
    }

    // ---------- M3-2-3 词间空白 ----------

    fn space(_font: FontId) -> Glue {
        Glue {
            width: 10 * SP_PER_PT,
            stretch: 5 * SP_PER_PT,
            shrink: 3 * SP_PER_PT,
        }
    }

    fn typeset_spaced(text: &str) -> Result<Vec<Node>> {
        Typesetter::with_metrics(metrics).with_space(space).typeset(text)
    }

    fn spaced_box_children(text: &str) -> Vec<Node> {
        let main = typeset_spaced(text).unwrap();
        as_box(&main[0]).children.clone()
    }

    #[test]
    fn space_becomes_glue() {
        let children = spaced_box_children(r"\hbox{a b}");
        assert_eq!(children.len(), 3);
        match &children[1] {
            Node::Glue { width, stretch, shrink } => {
                assert_eq!(*width, 10 * SP_PER_PT);
                assert_eq!(*stretch, 5 * SP_PER_PT);
                assert_eq!(*shrink, 3 * SP_PER_PT);
            }
            other => panic!("预期词间 Glue，得到 {other:?}"),
        }
        assert_eq!(as_char(&children[2]), b'b' as u32);
    }

    #[test]
    fn consecutive_spaces_collapse() {
        let children = spaced_box_children(r"\hbox{a  b}"); // 两个空格
        assert_eq!(children.len(), 3); // a + glue + b
        assert!(matches!(children[1], Node::Glue { .. }));
    }

    #[test]
    fn space_after_glue_or_penalty_ignored() {
        let children = spaced_box_children(r"\hbox{a\hskip 5pt  b}");
        assert_eq!(children.len(), 3); // a + glue(5pt) + b（空格被忽略）
        match &children[1] {
            Node::Glue { width, .. } => assert_eq!(*width, 5 * SP_PER_PT),
            other => panic!("预期 5pt Glue，得到 {other:?}"),
        }
        let children = spaced_box_children(r"\hbox{a\penalty -10  b}");
        assert_eq!(children.len(), 3); // a + penalty + b
        assert!(matches!(children[1], Node::Penalty { .. }));
    }

    #[test]
    fn space_at_hbox_start_ignored() {
        let children = spaced_box_children(r"\hbox{ a}");
        assert_eq!(children.len(), 1);
        assert_eq!(as_char(&children[0]), b'a' as u32);
    }

    #[test]
    fn dimen_scan_swallows_trailing_space() {
        // \kern 7pt 后的空格被 dimen 扫描吞掉（TeX 规则），不产生词间胶水
        let children = spaced_box_children(r"\hbox{a\kern 7pt b}");
        assert_eq!(children.len(), 3); // a + kern + b
        assert!(matches!(children[1], Node::Kern { .. }));
    }
}
