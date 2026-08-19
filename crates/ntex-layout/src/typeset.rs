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
//! - 字体度量：M3-4 TFM 前由 [`Typesetter::with_metrics`] 提供 fn 指针（默认全零）；
//!   [`Typesetter::with_tfm`] 启用真实 TFM 度量（`\font\cs=cmr10` + 词间距来自字体参数）；
//! - `\hbox to <glue>` / `\hbox spread <glue>` 规格暂拒。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use ntex_core::error::{Error, Result};
use ntex_core::expand::Expander;
use ntex_core::param::{ParamKind, ParamValue, Params};
use ntex_core::register::{Glue, REGISTER_COUNT, SP_PER_PT};
use ntex_core::token::Token;
use ntex_core::{FontLoader, Primitive, TokenSink};
use ntex_font::{FontMetrics, LigKern};

use crate::linebreak::knuth_plass;
use crate::node::{hpack, BoxKind, BoxNode, FontId, Node, GLUE_ORDER_FIL};
use crate::page::PageBuilder;

/// 模式（TeX 模式状态机的 M3-2 子集 + M4 数学）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// 垂直模式：垂直列表（页面主列表 / vbox 内容）；字符触发段落。
    Vertical,
    /// 水平模式：段落水平列表；`\par` / 输入结束封装为 hbox。
    Horizontal,
    /// 受限水平模式：`\hbox{...}` 内容。
    RestrictedHorizontal,
    /// 数学模式（行内 `$...$`，textstyle）。
    Math,
    /// 显示数学模式（`$$...$$`，displaystyle）。
    DisplayMath,
}

/// 数学原子类别（TeXbook 附录 G：8 类原子）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathClass {
    Ord,
    Bin,
    Op,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
}

/// 数学样式（决定字阶与 spacing 表；TeXbook p.140-141）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathStyle {
    Display,
    Text,
    Script,
    ScriptScript,
}

impl MathStyle {
    /// 字阶缩放（相对 textfont；TeX 数学三阶字体 text/script/scriptscript，
    /// 10pt 基字阶对应 7pt/5pt——ETRIP 前以比例近似，M4-3 用 fontdimen 精化）。
    fn scale(self) -> (i64, i64) {
        match self {
            MathStyle::Display | MathStyle::Text => (1, 1),
            MathStyle::Script => (7, 10),
            MathStyle::ScriptScript => (5, 10),
        }
    }

    /// 下一级（脚本的字阶）：Text→Script、Script→ScriptScript、Display→Script。
    fn next(self) -> MathStyle {
        match self {
            MathStyle::Display | MathStyle::Text => MathStyle::Script,
            MathStyle::Script => MathStyle::ScriptScript,
            MathStyle::ScriptScript => MathStyle::ScriptScript,
        }
    }
}

/// 数学字符原子：类 + 族 + 字符码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MathChar {
    class: MathClass,
    fam: u8,
    charcode: u32,
}

/// 数学列表原子（M4-1/2：字符/上下标/分式/根式/定界符/类化/样式/空格）。
#[derive(Debug, Clone, PartialEq)]
enum MathAtom {
    /// 字符原子（8 类之一）。
    Char(MathChar),
    /// 上下标：`x^a` / `x_b` / `x_a^b`（字段为子列表）。
    Scripts {
        base: Box<MathAtom>,
        sub: Option<Vec<MathAtom>>,
        sup: Option<Vec<MathAtom>>,
    },
    /// 分式（`\over`/`\atop`/`\above`）：num/den 子列表；thickness 为分式线厚度
    /// （None=字体默认、Some(0)=`\atop` 无线）。
    Fraction {
        num: Vec<MathAtom>,
        den: Vec<MathAtom>,
        thickness: Option<i64>,
    },
    /// 根式（`\sqrt`）：radicand 子列表。
    Radical { base: Vec<MathAtom> },
    /// `\left<delim>...\right<delim>`：定界符为 None 表示空（`.`）。
    Delimited {
        left: Option<u32>,
        body: Vec<MathAtom>,
        right: Option<u32>,
    },
    /// 显式定类字段（`\mathbin{...}` 等）：内容作为一个指定类的原子。
    Classed {
        class: MathClass,
        content: Vec<MathAtom>,
    },
    /// 样式切换（`\displaystyle`/`\textstyle`/`\scriptstyle`/`\scriptscriptstyle`）。
    Style(MathStyle),
    /// 数学空格（`\mskip`/`\mkern` 结果；`nonscript`：`\nonscript` 后脚本模式丢弃）。
    MSkip {
        width: i64,
        stretch: i64,
        shrink: i64,
        nonscript: bool,
    },
    /// 已排版盒子（数学模式内 `\hbox{...}` 产出）。
    Box(BoxNode),
}

/// 数学组字段类别（group_begin 压层时记录；group_end 按类别处理）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathFieldKind {
    /// 脚本字段（`^`/`_` 后的组）：Some(is_sup)。
    Script(bool),
    /// `\sqrt` 后的组（radicand）。
    Sqrt,
    /// `\mathbin` 等后的组：内容作为指定类原子。
    Class(MathClass),
}

/// 数学列表层级：原子列表 + 是否为特殊字段组。
#[derive(Debug, Default)]
struct MathLevel {
    atoms: Vec<MathAtom>,
    /// 本组字段类别（`^`/`_`、`\sqrt`、`\mathbin` 后的 `{...}`）；None = 普通组。
    field: Option<MathFieldKind>,
}

/// `\over`/`\atop` 中间态：numerator 已收集，当前数学层 atoms 继续收集 denominator。
#[derive(Debug)]
struct FractionPending {
    thickness: Option<i64>,
    num: Vec<MathAtom>,
}

/// 数学间距码（TeXbook 附录 G 规则 18）：0 无 / 1 thin / 2 med / 3 thick / 4 *（紧排）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpacingCode {
    None,
    Thin,
    Med,
    Thick,
    Tight,
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
    /// 本组是否为 `\shipout` 的目标（封装的盒子作为页面而非追加）。
    /// 随组传递：`\shipout\vbox{...\box255...}` 内层盒子不被 shipout。
    shipout: bool,
}

/// 字符度量函数：`(width, height, depth)`，单位 sp。
pub type MetricsFn = fn(FontId, u32) -> (i64, i64, i64);

/// 词间空白胶水函数（空格 token → 胶水；M3-4 TFM 前由调用方提供）。
pub type SpaceFn = fn(FontId) -> Glue;

/// 字体度量来源：fn 指针占位（M3-4 前 / ntex-pdf 临时切片）或 TFM 字体表（M3-4）。
#[derive(Debug, Clone)]
enum Fonts {
    /// fn 指针占位：字符维度/词间距由调用方提供（`with_metrics`/`with_space`）。
    Fn {
        metrics: MetricsFn,
        space: SpaceFn,
    },
    /// TFM 字体表（`with_tfm`）：FontId → 度量；`\font` 加载时追加。
    /// `Rc<RefCell>` 让加载器（[`TfmLoader`]）与节点构建器共享同一张表。
    Tfm(Rc<RefCell<Vec<FontMetrics>>>),
}

impl Fonts {
    fn metrics(&self, font: FontId, charcode: u32) -> (i64, i64, i64) {
        match self {
            Fonts::Fn { metrics, .. } => (metrics)(font, charcode),
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .map(|fm| fm.char_metrics(charcode))
                .unwrap_or((0, 0, 0)),
        }
    }

    fn space(&self, font: FontId) -> Glue {
        match self {
            Fonts::Fn { space, .. } => (space)(font),
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .map(|fm| fm.space_glue())
                .unwrap_or(Glue::ZERO),
        }
    }

    /// `\sfcode≥2000` 空格追加的 extra_space（cmr10 = 72818 sp；M3-4 词间距）。
    fn extra_space(&self, font: FontId) -> i64 {
        match self {
            Fonts::Fn { .. } => 0,
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .map(|fm| fm.extra_space)
                .unwrap_or(0),
        }
    }

    /// 查左字符的 lig/kern 程序（TFM 模式；fn 指针占位无程序）。
    fn lig_kern(&self, font: FontId, left: u8, right: u8) -> Option<LigKern> {
        match self {
            Fonts::Fn { .. } => None,
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .and_then(|fm| fm.apply_lig_kern(left, right)),
        }
    }

    /// 数学 em（quad = fontdimen 6；数学间距 1em 基准）。fn 指针占位返回 0。
    fn quad(&self, font: FontId) -> i64 {
        match self {
            Fonts::Fn { .. } => 0,
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .map(|fm| fm.quad)
                .unwrap_or(0),
        }
    }

    /// x 高度（fontdimen 5；数学上标提升基准）。fn 指针占位返回 0。
    fn x_height(&self, font: FontId) -> i64 {
        match self {
            Fonts::Fn { .. } => 0,
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .map(|fm| fm.x_height)
                .unwrap_or(0),
        }
    }
}

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
    /// `\sfcode` 表（随 `sfcode_changed` 事件更新；plain 默认 .,?!=3000、:=2000、
    /// ;=1500、,=1250，其余 1000）。
    sfcodes: [u32; 256],
    /// 当前 spacefactor（tex.web `space_factor`；段落/\hbox 开始 = 1000，
    /// 随字符 sfcode 更新，控制词间空格胶水）。
    space_factor: i64,
    /// `\noindent`：下一个段落不缩进。
    noindent_next: bool,
    /// 字体度量来源（M3-4：fn 指针占位或 TFM 字体表）。
    fonts: Fonts,
    /// 当前字体（TFM 模式由 `font_selected` 事件更新；fn 指针模式恒为 FontId(0)）。
    current_font: FontId,
    /// `\shipout`：下一个封装盒子作为页面（DVI shipout，M3-5）。
    shipout_next: bool,
    /// 已 \shipout 的页面（按顺序）。
    shipped: Vec<BoxNode>,
    /// M3-5-2 断页：启用自动分页（`typeset_dvi` 打开；旧 `typeset` 保持切片行为）。
    pagination: bool,
    /// 页面构建器（`pagination` 时把顶层垂直列表拆成页面）。
    page: PageBuilder,
    /// 盒子寄存器（M3-5-3）：`\box<n>` 读写（box255 为待输出例程页面队列，见
    /// [`Self::pending_pages`]，不占此表）。
    boxes: Vec<Option<BoxNode>>,
    /// `\output` 例程是否已定义（true：fire_up 改道 box255 + 待执行）。
    output_defined: bool,
    /// 待输出例程处理的页面队列（M3-5-3）：`\output` 定义时 fire_up 产出的页面
    /// 排队，`\box255` 逐页取出，例程反复运行直至队列清空。队列而非单槽——
    /// `close_paragraph` 一次推入多行可能连续产出多页，逐页交错执行例程。
    pending_pages: VecDeque<BoxNode>,
    /// RFC-3：页面真正输出（`\shipout` 边界）时置位，通知引擎 flush 延迟写流。
    write_flush_pending: bool,
    /// 数学列表栈（M4-1）：数学模式期间一层；`{...}` 数学组/脚本字段压层。
    math: Vec<MathLevel>,
    /// 当前数学样式（进入 Math=Text、DisplayMath=Display；`\displaystyle` 等修改）。
    math_style: MathStyle,
    /// 待挂载的脚本方向（`^`=Some(true)、`_`=Some(false)）：等待下一个原子/组。
    pending_script: Option<bool>,
    /// `\over`/`\atop`：numerator 已收集，等待 denominator（当前 math 层 atoms）。
    fraction_pending: Option<FractionPending>,
    /// `\left<delim>`：等待 `\right`（嵌套 `\left` 暂不支持）。
    left_pending: Option<Option<u32>>,
    /// `\sqrt`：等待 radicand 字段（下一个原子或组）。
    sqrt_pending: bool,
    /// `\mathbin` 等：等待字段（下一个原子或组），应用指定类。
    class_pending: Option<MathClass>,
    /// `\nonscript`：下一个数学空格在脚本模式丢弃。
    nonscript_pending: bool,
}

impl NodeBuilder {
    fn new(fonts: Fonts) -> Self {
        Self::with_pagination(fonts, false)
    }

    /// 创建构建器；`pagination` 打开 M3-5-2 断页（自动分页 + parskip + 收尾冲页）。
    fn with_pagination(fonts: Fonts, pagination: bool) -> Self {
        // plain 格式 \sfcode 默认（TeXbook p.75）：.,?! = 3000、: = 2000、; = 1500、, = 1250
        let mut sfcodes = [1000u32; 256];
        for (c, v) in [
            (b'.', 3000),
            (b'?', 3000),
            (b'!', 3000),
            (b':', 2000),
            (b';', 1500),
            (b',', 1250),
        ] {
            sfcodes[c as usize] = v;
        }
        Self {
            lists: vec![Vec::new()],
            list_modes: vec![Mode::Vertical],
            groups: Vec::new(),
            pending_box: None,
            params: Params::default(),
            param_stack: Vec::new(),
            sfcodes,
            space_factor: 1000,
            noindent_next: false,
            current_font: FontId(0),
            shipout_next: false,
            shipped: Vec::new(),
            pagination,
            page: PageBuilder::new(),
            boxes: vec![None; REGISTER_COUNT],
            output_defined: false,
            pending_pages: VecDeque::new(),
            write_flush_pending: false,
            math: Vec::new(),
            math_style: MathStyle::Text,
            pending_script: None,
            fraction_pending: None,
            left_pending: None,
            sqrt_pending: false,
            class_pending: None,
            nonscript_pending: false,
            fonts,
        }
    }

    fn mode(&self) -> Mode {
        *self.list_modes.last().expect("列表栈非空")
    }

    fn append(&mut self, node: Node) {
        self.lists.last_mut().expect("列表栈非空").push(node);
        // M3-5-2：顶层垂直模式追加后运行页面构建器（TeX build_page 的触发点）。
        // 增量（feed_one）：每产出一页即暂停——若定义了输出例程，让引擎在 token
        // 边界执行例程（ship box255）后再继续；未定义时页面直通 shipped。
        if self.pagination && self.mode() == Mode::Vertical && self.lists.len() == 1 {
            if let Some(p) = self.page.feed_one(&mut self.lists[0], &self.params) {
                self.accept_page(p);
            }
        }
    }

    /// 接收一个已产出的页面（M3-5-3）：定义了输出例程 → 进入待处理队列
    /// （`\box255` 逐页取出）；否则直接进 shipped（与 M3-5-2 默认行为一致）。
    fn accept_page(&mut self, p: BoxNode) {
        if self.output_defined {
            self.pending_pages.push_back(p);
        } else {
            self.shipped.push(p);
            self.write_flush_pending = true;
        }
    }

    /// 结束开放段落：Knuth-Plass 折行成行 hbox 并追加到上层列表（行间插 interline glue）。
    /// 段落末尾：裁剪尾部可丢弃节点 + 追加 `\parfillskip`（0pt plus 1fil，末行无限拉伸）。
    fn close_paragraph(&mut self) {
        let mut children = self.lists.pop().expect("段落列表");
        self.list_modes.pop();
        while children.last().is_some_and(Node::is_discardable) {
            children.pop();
        }
        if children.is_empty() {
            return; // 空段落不产生盒子
        }
        children.push(Node::Glue {
            width: 0,
            stretch: 1,
            shrink: 0,
            stretch_order: GLUE_ORDER_FIL,
            shrink_order: 0,
        });
        let lines = knuth_plass(&children, self.params.hsize, self.params.tolerance);
        for (s, e) in lines {
            // 断点胶水已在折行时排除；末行保留 \parfillskip（fil 拉伸填满行宽）
            let line: Vec<Node> = children[s..e].to_vec();
            // 行盒 = `\hbox to \hsize`（tex.web line_break：恰好 hsize 宽，胶水拉伸/收缩）
            self.push_box(Node::Box(hpack(&line, self.params.hsize)));
        }
    }

    /// 封装盒子内容（group_end 用）。`ship` 为本组是否为 `\shipout` 目标
    /// （M3-5）：是则封装为页面进 shipped，否则追加到上层列表。
    fn package_box(&mut self, kind: PendingBox, ship: bool) {
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
        if ship {
            if let Node::Box(b) = node {
                self.shipped.push(b);
                self.write_flush_pending = true;
            }
            return;
        }
        // 数学模式内 `\hbox{...}`：结果盒子转数学原子（TeX 数学盒子）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            let atom = match node {
                Node::Box(b) => MathAtom::Box(b),
                other => unreachable!("数学模式盒子封装必产出 Box 节点：{other:?}"),
            };
            self.math_push_atom(atom).expect("数学模式盒子挂载");
            return;
        }
        self.push_box(node);
    }

    /// 追加盒子到当前列表；垂直列表中前驱为盒子时插入 interline glue
    /// （tex.web `append_to_vlist`：d = \baselineskip − (depth 前 + height 新)，
    /// d < \lineskiplimit 用 \lineskip，否则用宽度调整为 d 的 \baselineskip）。
    fn push_box(&mut self, node: Node) {
        if self.mode() == Mode::Vertical {
            // 分页模式下顶层前驱盒子的深度/类型：页面构建器里的盒子，或
            // 断页后仍在贡献列表中的残余盒子（未入页，interline glue 的依据）。
            let (prev_is_box, prev_depth) = if self.pagination && self.lists.len() == 1 {
                match self
                    .lists[0]
                    .iter()
                    .rev()
                    .find(|n| matches!(n, Node::Box(_) | Node::Rule { .. }))
                {
                    Some(Node::Box(prev)) => (true, prev.depth),
                    Some(Node::Rule { depth, .. }) => (true, *depth),
                    _ => (
                        self.page.prev_depth() > crate::page::IGNORE_DEPTH,
                        self.page.prev_depth(),
                    ),
                }
            } else {
                match self.lists.last().and_then(|l| l.last()) {
                    Some(Node::Box(prev)) => (true, prev.depth),
                    _ => (false, 0),
                }
            };
            if prev_is_box {
                let height = match &node {
                    Node::Box(b) => b.height,
                    _ => 0,
                };
                let d = self.params.baselineskip.width - (prev_depth + height);
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
                    stretch_order: 0,
                    shrink_order: 0,
                });
            }
        }
        self.append(node);
    }

    /// 冲页后清理：新空页上的 glue/kern/penalty 本就会被页面构建器丢弃，
    /// 但 feed_one 在 fire_up 后即返回（触发节点残留在贡献前端），若不主动
    /// 清除，eject 循环会把这些可丢弃节点误判为"有待冲材料"而反复追加
    /// eject 节点 → 空页死循环。
    fn drop_empty_page_discardables(&mut self) {
        if !self.page.is_empty() {
            return;
        }
        while let Some(n) = self.lists[0].first() {
            if matches!(n, Node::Glue { .. } | Node::Kern { .. } | Node::Penalty { .. }) {
                self.lists[0].remove(0);
            } else {
                break;
            }
        }
    }

    /// M3-5-2 `\end` 冲页（tex.web `its_all_over`）：页或贡献非空时追加
    /// `\hbox to \hsize{}\vfill\penalty-'10000000000` 强制断页；触发节点（penalty）
    /// 面对新空页被页面构建器丢弃，故不产生多余空页。
    ///
    /// 增量版：每调用最多冲出一页（返回是否冲出）；`finish` 与输出例程
    /// 交错执行——页面经 [`Self::accept_page`] 路由（box255+例程 或 直通 shipout）。
    fn eject_one_page(&mut self) -> Result<bool> {
        loop {
            if self.page.is_empty() && self.lists[0].is_empty() {
                return Ok(false);
            }
            let hsize = self.params.hsize;
            let mut empty_box = BoxNode::new_hbox(Vec::new());
            empty_box.width = hsize; // \hbox to \hsize{}
            self.lists[0].push(Node::Box(empty_box));
            self.lists[0].push(Node::Glue {
                width: 0,
                stretch: 1,
                shrink: 0,
                stretch_order: GLUE_ORDER_FIL,
                shrink_order: 0,
            });
            self.lists[0].push(Node::Penalty {
                penalty: -(1 << 30), // \penalty-'10000000000
            });
            // 产出一页则返回；材料全部入页但未触发断页 → 补充 eject 节点再试
            if let Some(p) = self.page.feed_one(&mut self.lists[0], &self.params) {
                self.accept_page(p);
                self.drop_empty_page_discardables();
                return Ok(true);
            }
        }
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
        let (w, h, d) = self.fonts.metrics(self.current_font, charcode);
        Some(Node::Char {
            font: self.current_font,
            charcode,
            width: w,
            height: h,
            depth: d,
        })
    }

    /// 词间空白胶水（tex.web `append_normal_space` / `app_space`）：
    /// spacefactor = 1000 → 字体空格原样；否则按 `app_space` 调整——sf≥2000
    /// 宽度加 extra_space；stretch ×= sf/1000；shrink ×= 1000/sf（xn_over_d 舍入）。
    fn append_space_glue(&mut self) {
        let g = self.fonts.space(self.current_font);
        let (width, stretch, shrink) = if self.space_factor == 1000 {
            (g.width, g.stretch, g.shrink)
        } else {
            let sf = self.space_factor;
            let width = if sf >= 2000 {
                g.width + self.fonts.extra_space(self.current_font)
            } else {
                g.width
            };
            (
                width,
                xn_over_d(g.stretch, sf, 1000),
                xn_over_d(g.shrink, 1000, sf),
            )
        };
        self.append(Node::Glue {
            width,
            stretch,
            shrink,
            stretch_order: 0,
            shrink_order: 0,
        });
    }

    /// 水平模式追加字符（tex.web main_loop 子集）：
    /// 1. 更新 spacefactor（adjust_space_factor，对被连字消费的字符也执行）；
    /// 2. 与列表尾同字体字符查 lig/kern 程序——kern 在其前插入 kern 节点；
    ///    lig 把尾字符替换为结果字符并丢弃当前字符（ffi/ffl 由结果字符的
    ///    程序在下一字符到来时自然连续匹配）。
    fn append_char(&mut self, node: Node) {
        if let Node::Char {
            font,
            charcode,
            width,
            height,
            depth,
        } = node
        {
            self.adjust_space_factor(charcode);
            // 先取出前驱 (font, charcode)，避免借用冲突
            let prev = match self.lists.last().and_then(|l| l.last()) {
                Some(Node::Char {
                    font: pf,
                    charcode: pc,
                    ..
                }) if *pf == font => Some((*pf, *pc)),
                _ => None,
            };
            let mut drop_cur = false;
            if let Some((pf, pc)) = prev {
                if let Some(action) =
                    self.fonts.lig_kern(pf, pc as u8, charcode as u8)
                {
                    match action {
                        LigKern::Kern(kern) => self.append(Node::Kern { width: kern }),
                        LigKern::Lig(result) => {
                            // 尾字符替换为结果字符（fi/fl/ff 等），当前字符丢弃
                            let (w, h, d) = self.fonts.metrics(font, result as u32);
                            if let Some(Node::Char {
                                charcode: rc,
                                width: rw,
                                height: rh,
                                depth: rd,
                                ..
                            }) = self.lists.last_mut().and_then(|l| l.last_mut())
                            {
                                *rc = result as u32;
                                *rw = w;
                                *rh = h;
                                *rd = d;
                            }
                            drop_cur = true;
                        }
                    }
                }
            }
            if !drop_cur {
                self.append(Node::Char {
                    font,
                    charcode,
                    width,
                    height,
                    depth,
                });
            }
        } else {
            self.append(node);
        }
    }

    /// tex.web `adjust_space_factor`：sfcode=0 不变；=1000 → 1000；<1000 且 >0
    /// → 取该值；>1000 且当前 <1000 → 1000；否则取该值。
    fn adjust_space_factor(&mut self, charcode: u32) {
        let s = self
            .sfcodes
            .get(charcode as usize)
            .copied()
            .unwrap_or(1000) as i64;
        if s == 1000 {
            self.space_factor = 1000;
        } else if s < 1000 {
            if s > 0 {
                self.space_factor = s;
            }
        } else if self.space_factor < 1000 {
            self.space_factor = 1000;
        } else {
            self.space_factor = s;
        }
    }

    // ---------- M4-1 数学模式 ----------

    /// 进入数学模式：压数学层 + 占位列表层（公式节点经 close_math 落回上层列表）。
    /// `mode` 为 Math（行内，textstyle）或 DisplayMath（显示，displaystyle）。
    fn enter_math(&mut self, mode: Mode) -> Result<()> {
        let style = match mode {
            Mode::DisplayMath => MathStyle::Display,
            _ => MathStyle::Text,
        };
        self.math.push(MathLevel::default());
        self.math_style = style;
        self.lists.push(Vec::new());
        self.list_modes.push(mode);
        Ok(())
    }

    /// 退出数学模式：数学列表转 hlist 追加到上层列表（行内/显示公式）。
    fn close_math(&mut self) -> Result<()> {
        if self.pending_script.is_some() {
            return Err(Error::invalid_input(
                "数学模式中 ^/_ 后缺少上标/下标（Missing { inserted）",
            ));
        }
        let mut level = self
            .math
            .pop()
            .ok_or_else(|| Error::internal("close_math 无数学层"))?;
        let style = self.math_style;
        self.list_modes.pop();
        self.lists.pop();
        // 公式末尾收尾：未闭合 \left 报错；待定分式收尾（TeX 允许空分母）
        if self.left_pending.is_some() {
            return Err(Error::invalid_input("\\left 后缺少 \\right（Extra } or forgotten \\right）"));
        }
        Self::math_finish_fraction(&mut self.fraction_pending, &mut level);
        let nodes = self.math_to_hlist(&level.atoms, style);
        for n in nodes {
            self.append(n);
        }
        Ok(())
    }

    /// 数学模式字符：`^`/`_` 设待挂脚本；字母/其他 → Ord 原子（M4-2 原子类化）。
    fn math_char_tok(&mut self, tok: Token) -> Result<()> {
        let (Some(cat), Some(ch)) = (tok.catcode(), tok.charcode()) else {
            return Ok(());
        };
        match cat {
            ntex_core::Catcode::Superscript => {
                self.pending_script = Some(true);
                Ok(())
            }
            ntex_core::Catcode::Subscript => {
                self.pending_script = Some(false);
                Ok(())
            }
            ntex_core::Catcode::Letter | ntex_core::Catcode::Other => self
                .math_push_atom(MathAtom::Char(MathChar {
                    class: MathClass::Ord,
                    fam: 0,
                    charcode: ch,
                })),
            _ => Ok(()), // active 等已在展开侧处理；其余忽略
        }
    }

    /// 追加原子到当前数学层；处理待定字段（`\sqrt`/`\mathbin`/`\nonscript`）与
    /// 脚本挂载（`x^2`/`x_i`/`x_i^2`）。
    fn math_push_atom(&mut self, atom: MathAtom) -> Result<()> {
        // `\sqrt` 单原子字段：`\sqrt x`
        if self.sqrt_pending {
            self.sqrt_pending = false;
            let level = self
                .math
                .last_mut()
                .ok_or_else(|| Error::internal("数学原子无数学层"))?;
            level.atoms.push(MathAtom::Radical { base: vec![atom] });
            return Ok(());
        }
        // `\mathbin` 等单原子字段：`\mathbin+`（Char 改类，其余包 Classed）
        if let Some(class) = self.class_pending.take() {
            let atom = match atom {
                MathAtom::Char(mut mc) => {
                    mc.class = class;
                    MathAtom::Char(mc)
                }
                other => MathAtom::Classed {
                    class,
                    content: vec![other],
                },
            };
            return self.math_push_atom_raw(atom);
        }
        // `\nonscript`：下一个数学空格标记为脚本模式丢弃
        let atom = match atom {
            MathAtom::MSkip {
                width,
                stretch,
                shrink,
                ..
            } if self.nonscript_pending => MathAtom::MSkip {
                width,
                stretch,
                shrink,
                nonscript: true,
            },
            other => other,
        };
        self.nonscript_pending = false;
        self.math_push_atom_raw(atom)
    }

    /// 原始追加（含脚本挂载）：`x^2`/`x_i`/`x_i^2`。
    fn math_push_atom_raw(&mut self, atom: MathAtom) -> Result<()> {
        let level = self
            .math
            .last_mut()
            .ok_or_else(|| Error::internal("数学原子无数学层"))?;
        if let Some(is_sup) = self.pending_script.take() {
            let mut base = level.atoms.pop().ok_or_else(|| {
                Error::invalid_input("数学模式中 ^/_ 前缺少原子（Missing { inserted）")
            })?;
            if let MathAtom::Scripts { sub, sup, .. } = &mut base {
                if is_sup {
                    if sup.is_some() {
                        return Err(Error::invalid_input("双重上标（Double superscript）"));
                    }
                    *sup = Some(vec![atom]);
                } else {
                    if sub.is_some() {
                        return Err(Error::invalid_input("双重下标（Double subscript）"));
                    }
                    *sub = Some(vec![atom]);
                }
                level.atoms.push(base);
            } else {
                let (sub, sup) = if is_sup {
                    (None, Some(vec![atom]))
                } else {
                    (Some(vec![atom]), None)
                };
                level.atoms.push(MathAtom::Scripts {
                    base: Box::new(base),
                    sub,
                    sup,
                });
            }
        } else {
            level.atoms.push(atom);
        }
        Ok(())
    }

    /// 完成待定分式：denominator = 当前数学层 atoms → Fraction 原子（TeX fin_mlist）。
    fn math_finish_fraction(fraction_pending: &mut Option<FractionPending>, level: &mut MathLevel) {
        if let Some(fp) = fraction_pending.take() {
            let den = std::mem::take(&mut level.atoms);
            level.atoms.push(MathAtom::Fraction {
                num: fp.num,
                den,
                thickness: fp.thickness,
            });
        }
    }

    /// 脚本字段组结束（`x^{...}`/`x_{...}`）：字段挂到外层末尾原子。
    fn math_attach_script(
        parent: &mut MathLevel,
        is_sup: bool,
        field: Vec<MathAtom>,
    ) -> Result<()> {
        let mut base = parent.atoms.pop().ok_or_else(|| {
            Error::invalid_input("数学模式中 ^/_ 前缺少原子（Missing { inserted）")
        })?;
        if let MathAtom::Scripts { sub, sup, .. } = &mut base {
            if is_sup {
                if sup.is_some() {
                    return Err(Error::invalid_input("双重上标（Double superscript）"));
                }
                *sup = Some(field);
            } else {
                if sub.is_some() {
                    return Err(Error::invalid_input("双重下标（Double subscript）"));
                }
                *sub = Some(field);
            }
            parent.atoms.push(base);
        } else {
            let (sub, sup) = if is_sup {
                (None, Some(field))
            } else {
                (Some(field), None)
            };
            parent.atoms.push(MathAtom::Scripts {
                base: Box::new(base),
                sub,
                sup,
            });
        }
        Ok(())
    }

    /// 原子类别（spacing 表用；脚本原子取 base 的类）。
    fn math_class(atom: &MathAtom) -> Option<MathClass> {
        match atom {
            MathAtom::Char(mc) => Some(mc.class),
            MathAtom::Scripts { base, .. } => Self::math_class(base),
            MathAtom::Classed { class, .. } => Some(*class),
            MathAtom::Fraction { .. } => Some(MathClass::Inner),
            MathAtom::Delimited { .. } => Some(MathClass::Inner),
            MathAtom::Radical { .. } => Some(MathClass::Ord),
            MathAtom::Box(_) => Some(MathClass::Ord),
            MathAtom::MSkip { .. } | MathAtom::Style(_) => None,
        }
    }

    /// 数学列表 → 水平节点（M4-1：spacing 胶水 + 字符 + 上下标盒）。
    fn math_to_hlist(&self, atoms: &[MathAtom], style: MathStyle) -> Vec<Node> {
        let mut out = Vec::new();
        let mut style = style;
        let mut prev: Option<MathClass> = None;
        for atom in atoms {
            // 样式切换原子就地生效（影响后续原子字阶与 spacing）
            if let MathAtom::Style(s) = atom {
                style = *s;
                continue;
            }
            let cur = Self::math_class(atom);
            if let (Some(p), Some(c)) = (prev, cur) {
                match spacing_code(p, c, style) {
                    SpacingCode::None | SpacingCode::Tight => {}
                    code => {
                        let quad = self.fonts.quad(self.current_font);
                        let (w, st, sh) = muskip(code, quad);
                        out.push(Node::Glue {
                            width: w,
                            stretch: st,
                            shrink: sh,
                            stretch_order: 0,
                            shrink_order: 0,
                        });
                    }
                }
            }
            if cur.is_some() {
                prev = cur;
            }
            out.extend(self.math_atom_nodes(atom, style));
        }
        out
    }

    /// 原子 → 节点（M4-1/2：Char/Scripts/Fraction/Radical/Delimited/Classed/MSkip/Box）。
    fn math_atom_nodes(&self, atom: &MathAtom, style: MathStyle) -> Vec<Node> {
        match atom {
            MathAtom::Char(mc) => {
                let (num, den) = style.scale();
                let (w, h, d) = self.math_metrics(self.current_font, mc.charcode, num, den);
                vec![Node::Char {
                    font: self.current_font,
                    charcode: mc.charcode,
                    width: w,
                    height: h,
                    depth: d,
                }]
            }
            MathAtom::Scripts { base, sub, sup } => {
                let mut out = self.math_atom_nodes(base, style);
                let s_style = style.next();
                // 上标：内容打包为 hbox，shift 上移（hlist 内 Box.shift 为垂直位移）
                if let Some(sup_atoms) = sup {
                    let nodes = self.math_to_hlist(sup_atoms, s_style);
                    let mut b = BoxNode::new_hbox(nodes);
                    b.shift = -self.script_rise(style);
                    out.push(Node::Box(b));
                }
                if let Some(sub_atoms) = sub {
                    let nodes = self.math_to_hlist(sub_atoms, s_style);
                    let mut b = BoxNode::new_hbox(nodes);
                    b.shift = self.script_drop();
                    out.push(Node::Box(b));
                }
                out
            }
            MathAtom::Fraction {
                num,
                den,
                thickness,
            } => self.fraction_nodes(num, den, *thickness, style),
            MathAtom::Radical { base } => self.radical_nodes(base, style),
            MathAtom::Delimited { left, body, right } => {
                let mut out = Vec::new();
                if let Some(d) = left {
                    out.extend(self.delim_nodes(*d, style));
                }
                out.extend(self.math_to_hlist(body, style));
                if let Some(d) = right {
                    out.extend(self.delim_nodes(*d, style));
                }
                out
            }
            MathAtom::Classed { content, .. } => self.math_to_hlist(content, style),
            MathAtom::MSkip {
                width,
                stretch,
                shrink,
                nonscript,
            } => {
                if *nonscript
                    && matches!(style, MathStyle::Script | MathStyle::ScriptScript)
                {
                    Vec::new()
                } else {
                    vec![Node::Glue {
                        width: *width,
                        stretch: *stretch,
                        shrink: *shrink,
                        stretch_order: 0,
                        shrink_order: 0,
                    }]
                }
            }
            MathAtom::Box(b) => vec![Node::Box(b.clone())],
            MathAtom::Style(_) => unreachable!("Style 原子在 math_to_hlist 循环中处理"),
        }
    }

    /// 分式 → 节点（M4-2 简化：分子/分式线/分母垂直堆叠；M4-3 用 fontdimen 精化）。
    fn fraction_nodes(
        &self,
        num: &[MathAtom],
        den: &[MathAtom],
        thickness: Option<i64>,
        style: MathStyle,
    ) -> Vec<Node> {
        let num_b = BoxNode::new_hbox(self.math_to_hlist(num, style));
        let den_b = BoxNode::new_hbox(self.math_to_hlist(den, style));
        let width = num_b.width.max(den_b.width);
        let num_height = num_b.height;
        let t = thickness.unwrap_or(SP_PER_PT * 2 / 5); // 默认分式线 0.4pt
        let gap = 2 * SP_PER_PT; // 分子/分母与线的间隙（M4-3 用 fontdimen num1 等）
        let mut children = Vec::new();
        children.push(Node::Box(num_b));
        // 垂直间隙（vbox 内 x 不推进；宽度 0 避免抬高 vbox 总宽）
        children.push(Node::Glue {
            width: 0,
            stretch: 0,
            shrink: 0,
            stretch_order: 0,
            shrink_order: 0,
        });
        if t > 0 {
            children.push(Node::Rule {
                width,
                height: t,
                depth: 0,
            });
        }
        children.push(Node::Glue {
            width: 0,
            stretch: 0,
            shrink: 0,
            stretch_order: 0,
            shrink_order: 0,
        });
        children.push(Node::Box(den_b));
        let mut b = BoxNode::new_vbox(children);
        // 参考点 = 分式线：顶部（num 顶）到线 = num 高 + gap + t/2（hlist 内 shift 为垂直位移）
        b.shift = num_height + gap + t / 2;
        vec![Node::Box(b)]
    }

    /// 根式 → 节点（M4-2 简化：内容上方画分式线式横线；M4-3 换 cmex10 根号）。
    fn radical_nodes(&self, base: &[MathAtom], style: MathStyle) -> Vec<Node> {
        let base_b = BoxNode::new_hbox(self.math_to_hlist(base, style));
        let base_height = base_b.height;
        let t = 2 * SP_PER_PT / 5; // 0.4pt
        let gap = SP_PER_PT; // 内容上缘到线的间隙
        let mut children = Vec::new();
        children.push(Node::Rule {
            width: base_b.width,
            height: t,
            depth: 0,
        });
        children.push(Node::Glue {
            width: 0,
            stretch: 0,
            shrink: 0,
            stretch_order: 0,
            shrink_order: 0,
        });
        children.push(Node::Box(base_b));
        let mut b = BoxNode::new_vbox(children);
        // 参考点 = 内容基线：线在基线上方 base 高 + gap + t 处（shift 为负 = 上移）
        b.shift = -(base_height + gap + t);
        vec![Node::Box(b)]
    }

    /// 定界符字符节点（当前字体 + 字阶缩放；M4-3 换 cmex10 变体伸缩）。
    fn delim_nodes(&self, d: u32, style: MathStyle) -> Vec<Node> {
        let (num, den) = style.scale();
        let (w, h, dd) = self.math_metrics(self.current_font, d, num, den);
        vec![Node::Char {
            font: self.current_font,
            charcode: d,
            width: w,
            height: h,
            depth: dd,
        }]
    }

    /// 按字阶缩放字符度量（M4-1 比例近似；M4-3 换真实 scriptfont）。
    fn math_metrics(&self, font: FontId, ch: u32, num: i64, den: i64) -> (i64, i64, i64) {
        let (w, h, d) = self.fonts.metrics(font, ch);
        if num == den {
            return (w, h, d);
        }
        (
            xn_over_d(w, num, den),
            xn_over_d(h, num, den),
            xn_over_d(d, num, den),
        )
    }

    /// 上标提升量（M4-1 近似：script 字阶的 x_height；M4-3 用 fontdimen）。
    fn script_rise(&self, style: MathStyle) -> i64 {
        let xh = self.fonts.x_height(self.current_font);
        let (num, den) = style.scale();
        xn_over_d(xh, num, den)
    }

    /// 下标下降量（M4-1 近似：0.5pt；M4-3 用 fontdimen sub_drop）。
    fn script_drop(&self) -> i64 {
        0
    }
}

/// 数学间距（TeXbook 附录 G 规则 18；text/script 模式；display 对 op 修正）。
/// 行 = 左原子类、列 = 右原子类；0 无 / 1 thin / 2 med / 3 thick / 4 *（紧排）。
fn spacing_code(l: MathClass, r: MathClass, style: MathStyle) -> SpacingCode {
    const TABLE: [[u8; 8]; 8] = [
        //          ord op  bin rel open close punct inner
        /*ord*/    [0, 1, 2, 3, 0, 0, 0, 1],
        /*op*/     [1, 1, 4, 3, 0, 0, 0, 1],
        /*bin*/    [2, 2, 4, 4, 2, 2, 2, 2],
        /*rel*/    [3, 3, 4, 0, 3, 3, 3, 3],
        /*open*/   [0, 0, 4, 0, 0, 0, 0, 0],
        /*close*/  [0, 1, 2, 3, 0, 0, 0, 1],
        /*punct*/  [1, 1, 4, 1, 1, 1, 1, 1],
        /*inner*/  [1, 1, 2, 3, 1, 0, 1, 1],
    ];
    let idx = |c: MathClass| match c {
        MathClass::Ord => 0,
        MathClass::Op => 1,
        MathClass::Bin => 2,
        MathClass::Rel => 3,
        MathClass::Open => 4,
        MathClass::Close => 5,
        MathClass::Punct => 6,
        MathClass::Inner => 7,
    };
    let raw = TABLE[idx(l)][idx(r)];
    // display 模式（TeXbook p.170）：op 前后的 thin(1) 升为 thick(3)。
    if style == MathStyle::Display
        && raw == 1
        && (l == MathClass::Op || r == MathClass::Op)
    {
        return SpacingCode::Thick;
    }
    match raw {
        0 => SpacingCode::None,
        1 => SpacingCode::Thin,
        2 => SpacingCode::Med,
        3 => SpacingCode::Thick,
        _ => SpacingCode::Tight,
    }
}

/// muskip 宽度（1mu = quad/18）：thin=3mu、med=4mu±2mu∓4mu、thick=5mu±5mu。
fn muskip(code: SpacingCode, quad: i64) -> (i64, i64, i64) {
    let mu = quad / 18;
    match code {
        SpacingCode::Thin => (3 * mu, 0, 0),
        SpacingCode::Med => (4 * mu, 2 * mu, 4 * mu),
        SpacingCode::Thick => (5 * mu, 5 * mu, 0),
        _ => (0, 0, 0),
    }
}

/// `xn_over_d`（tex.web）：t×n/d 四舍五入（负值按远离零）。
fn xn_over_d(t: i64, n: i64, d: i64) -> i64 {
    if t >= 0 {
        (t * n + d / 2) / d
    } else {
        -(((-t) * n + d / 2) / d)
    }
}

impl TokenSink for NodeBuilder {
    /// 数学移位（`$`，cat 3）：VM 已 peek 出 `display`（连续 `$$`）。
    /// - Math：结束行内公式；
    /// - DisplayMath：`$$` 结束显示公式，单 `$` 报错（TeX "Display math should end with $$"）；
    /// - 非数学模式：display → 显示数学（垂直模式开段），否则行内数学。
    fn math_shift(&mut self, display: bool) -> Result<()> {
        match self.mode() {
            Mode::Math => self.close_math(),
            Mode::DisplayMath => {
                if display {
                    self.close_math()
                } else {
                    Err(Error::invalid_input("Display math should end with $$."))
                }
            }
            Mode::Vertical => {
                // 开段（TeX new_graf）：公式属于段落（显示数学 M4-4 改独立段）
                if self.pagination {
                    let ps = self.params.parskip;
                    self.append(Node::Glue {
                        width: ps.width,
                        stretch: ps.stretch,
                        shrink: ps.shrink,
                        stretch_order: 0,
                        shrink_order: 0,
                    });
                }
                self.lists.push(Vec::new());
                self.list_modes.push(Mode::Horizontal);
                self.space_factor = 1000; // new_graf：段落开始重置 spacefactor
                self.insert_indent();
                self.enter_math(if display {
                    Mode::DisplayMath
                } else {
                    Mode::Math
                })
            }
            Mode::Horizontal | Mode::RestrictedHorizontal => self.enter_math(if display {
                Mode::DisplayMath
            } else {
                Mode::Math
            }),
        }
    }

    /// 数学样式原语：数学模式内 push 样式原子（影响后续字阶与 spacing）。
    fn math_style(&mut self, style: u8) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Ok(()); // TeX 报错，简化忽略（非数学模式样式无意义）
        }
        let s = match style {
            0 => MathStyle::Display,
            1 => MathStyle::Text,
            2 => MathStyle::Script,
            _ => MathStyle::ScriptScript,
        };
        self.math_push_atom(MathAtom::Style(s))
    }

    /// `\over`/`\atop`/`\above`：numerator 已收集（当前 math 层），等待 denominator。
    fn math_fraction(&mut self, thickness: Option<i64>) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Err(Error::invalid_input("\\over 只能在数学模式使用"));
        }
        if self.fraction_pending.is_some() {
            return Err(Error::invalid_input(
                "\\over 歧义（Ambiguous; you need another { and }）",
            ));
        }
        if self.pending_script.is_some() {
            return Err(Error::invalid_input("\\over 前不能有未挂脚本（Missing { inserted）"));
        }
        let level = self
            .math
            .last_mut()
            .ok_or_else(|| Error::internal("\\over 无数学层"))?;
        let num = std::mem::take(&mut level.atoms);
        self.fraction_pending = Some(FractionPending { thickness, num });
        Ok(())
    }

    /// `\left<delim>`：记录定界符，等待 `\right`（嵌套暂不支持）。
    fn math_left(&mut self, delim: Option<u32>) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Err(Error::invalid_input("\\left 只能在数学模式使用"));
        }
        if self.left_pending.is_some() {
            return Err(Error::invalid_input("\\left 不能嵌套（Extra \\left）"));
        }
        self.left_pending = Some(delim);
        Ok(())
    }

    /// `\right<delim>`：当前 math 层内容收为 \left...\right 的 body。
    fn math_right(&mut self, delim: Option<u32>) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Err(Error::invalid_input("\\right 只能在数学模式使用"));
        }
        let left = self
            .left_pending
            .take()
            .ok_or_else(|| Error::invalid_input("\\right 前缺少 \\left（Missing \\left inserted）"))?;
        let level = self
            .math
            .last_mut()
            .ok_or_else(|| Error::internal("\\right 无数学层"))?;
        // 先收 \left(...\over...\right) 的分式
        Self::math_finish_fraction(&mut self.fraction_pending, level);
        let body = std::mem::take(&mut level.atoms);
        level.atoms.push(MathAtom::Delimited {
            left,
            body,
            right: delim,
        });
        Ok(())
    }

    /// `\sqrt`：等待 radicand 字段（下一个原子或组）。
    fn math_sqrt(&mut self) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Err(Error::invalid_input("\\sqrt 只能在数学模式使用"));
        }
        self.sqrt_pending = true;
        Ok(())
    }

    /// `\mathord` 等：给下一个字段定类。
    fn math_class(&mut self, class: u8) -> Result<()> {
        if !matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Err(Error::invalid_input("\\mathord 等只能在数学模式使用"));
        }
        self.class_pending = Some(match class {
            0 => MathClass::Ord,
            1 => MathClass::Bin,
            2 => MathClass::Op,
            3 => MathClass::Rel,
            4 => MathClass::Open,
            5 => MathClass::Close,
            6 => MathClass::Punct,
            _ => MathClass::Inner,
        });
        Ok(())
    }

    fn token(&mut self, tok: Token) -> Result<()> {
        // 空格（cat 10）：垂直/数学模式忽略；水平模式转词间空白胶水
        // （行首或胶水/惩罚之后忽略，TeX spacer 语义）。
        if tok.catcode() == Some(ntex_core::Catcode::Space) {
            match self.mode() {
                Mode::Vertical | Mode::Math | Mode::DisplayMath => {}
                Mode::Horizontal | Mode::RestrictedHorizontal => {
                    let ignorable = match self.lists.last().and_then(|l| l.last()) {
                        None => true,
                        Some(Node::Glue { .. } | Node::Penalty { .. }) => true,
                        Some(_) => false,
                    };
                    if !ignorable {
                        self.append_space_glue();
                    }
                }
            }
            return Ok(());
        }
        // 数学模式：字符转数学原子（^/_ 挂脚本，字母/其他 → Ord）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_char_tok(tok);
        }
        let Some(node) = self.char_node(tok) else {
            return Ok(()); // 控制序列等无可排版语义
        };
        match self.mode() {
            Mode::Vertical => {
                // 垂直模式字符触发段落（TeX new_graf）
                // M3-5-2：段落起始追加上下段间距 \parskip（空页上被页面构建器丢弃）
                if self.pagination {
                    let ps = self.params.parskip;
                    self.append(Node::Glue {
                        width: ps.width,
                        stretch: ps.stretch,
                        shrink: ps.shrink,
                        stretch_order: 0,
                        shrink_order: 0,
                    });
                }
                self.lists.push(Vec::new());
                self.list_modes.push(Mode::Horizontal);
                self.space_factor = 1000; // new_graf：段落开始重置 spacefactor
                self.insert_indent();
                self.append_char(node);
            }
            Mode::Horizontal | Mode::RestrictedHorizontal => self.append_char(node),
            Mode::Math | Mode::DisplayMath => unreachable!("数学模式已在上面分支返回"),
        }
        Ok(())
    }

    fn group_begin(&mut self) -> Result<()> {
        let kind = self.pending_box.take();
        // `\shipout` 目标 = 紧邻的盒子组（内层盒子不消费该标记）
        let ship = if kind.is_some() {
            std::mem::take(&mut self.shipout_next)
        } else {
            false
        };
        self.groups.push(GroupCtx {
            box_kind: kind,
            shipout: ship,
        });
        self.param_stack.push(self.params);
        if let Some(k) = kind {
            let new_mode = match k {
                PendingBox::HBox => Mode::RestrictedHorizontal,
                PendingBox::VBox | PendingBox::VTop => Mode::Vertical,
            };
            self.lists.push(Vec::new());
            self.list_modes.push(new_mode);
            // \hbox 内容从 spacefactor=1000 开始（tex.web：进入受限水平模式重置）
            if new_mode == Mode::RestrictedHorizontal {
                self.space_factor = 1000;
            }
        } else if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            // 数学组：`{...}`（含脚本/根式/定类字段）压 math 层。
            let field = if let Some(is_sup) = self.pending_script.take() {
                Some(MathFieldKind::Script(is_sup))
            } else if self.sqrt_pending {
                self.sqrt_pending = false;
                Some(MathFieldKind::Sqrt)
            } else if let Some(class) = self.class_pending.take() {
                Some(MathFieldKind::Class(class))
            } else {
                None
            };
            self.math.push(MathLevel {
                atoms: Vec::new(),
                field,
            });
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
        // 数学组：内容并入外层（普通组）或作为字段挂到外层 base（^/_ 后组等）。
        if ctx.box_kind.is_none() && matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            let level = self
                .math
                .pop()
                .ok_or_else(|| Error::internal("数学组结束无配对 math 层"))?;
            let parent = self
                .math
                .last_mut()
                .ok_or_else(|| Error::internal("数学组结束无外层 math 层"))?;
            let field_atoms = match level.field {
                Some(MathFieldKind::Script(is_sup)) => {
                    // `x^{...}`：先收组内分式（`x^{a\over b}`），再作为脚本字段挂载
                    let mut lv = level;
                    Self::math_finish_fraction(&mut self.fraction_pending, &mut lv);
                    let field = lv.atoms;
                    if !field.is_empty() {
                        // `x^{}`：空字段合法（TeX 空组字段）
                        Self::math_attach_script(parent, is_sup, field)?;
                    }
                    return Ok(());
                }
                Some(MathFieldKind::Sqrt) => {
                    // `\sqrt{...}`：先收组内分式（`\sqrt{a\over b}`），再作 radicand
                    let mut lv = level;
                    Self::math_finish_fraction(&mut self.fraction_pending, &mut lv);
                    parent.atoms.push(MathAtom::Radical { base: lv.atoms });
                    return Ok(());
                }
                Some(MathFieldKind::Class(class)) => {
                    // `\mathbin{...}`：内容作为一个指定类原子
                    let mut lv = level;
                    Self::math_finish_fraction(&mut self.fraction_pending, &mut lv);
                    parent.atoms.push(MathAtom::Classed {
                        class,
                        content: lv.atoms,
                    });
                    return Ok(());
                }
                None => {
                    // 普通数学组：先收组内分式（`{a\over b}`），再并入外层
                    let mut lv = level;
                    Self::math_finish_fraction(&mut self.fraction_pending, &mut lv);
                    lv.atoms
                }
            };
            parent.atoms.extend(field_atoms);
            return Ok(());
        }
        // 垂直盒子内容结束时，开放段落先封装（\vbox{a} → vbox[hbox(a)]）
        if ctx.box_kind.is_some_and(PendingBox::is_vertical) && self.mode() == Mode::Horizontal {
            self.close_paragraph();
        }
        if let Some(kind) = ctx.box_kind {
            self.package_box(kind, ctx.shipout);
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
                // 垂直模式 \par 无操作；受限水平/数学模式拒绝
                Mode::Vertical => {}
                Mode::RestrictedHorizontal => {
                    return Err(Error::invalid_input(
                        "\\par 不允许出现在受限水平模式（\\hbox 内）",
                    ));
                }
                Mode::Math | Mode::DisplayMath => {
                    return Err(Error::invalid_input(
                        "\\par 不允许出现在数学模式（\\par 应在 $ 外）",
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
                Mode::Math | Mode::DisplayMath => {}
            },
            Primitive::NoIndent => {
                // 垂直模式：下一个段落不缩进；水平模式无操作
                if self.mode() == Mode::Vertical {
                    self.noindent_next = true;
                }
            }
            // M3-5：\shipout 后的下一个盒子封装为页面
            Primitive::ShipOut => self.shipout_next = true,
            // M4-2：\nonscript 使下一个数学空格在脚本模式丢弃
            Primitive::Nonscript => {
                if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
                    self.nonscript_pending = true;
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

    fn sfcode_changed(&mut self, charcode: u8, value: u32) -> Result<()> {
        self.sfcodes[charcode as usize] = value;
        Ok(())
    }

    fn output_defined(&mut self, defined: bool) -> Result<()> {
        self.output_defined = defined;
        if !defined {
            // 例程恢复未定义：未处理页面无法再经例程产出，直接丢弃（TeX 语义）
            self.pending_pages.clear();
        }
        Ok(())
    }

    fn output_pending(&self) -> bool {
        !self.pending_pages.is_empty()
    }

    fn take_output_pending(&mut self) -> bool {
        !self.pending_pages.is_empty()
    }

    fn output_pending_count(&self) -> usize {
        self.pending_pages.len()
    }

    fn discard_pending_pages(&mut self) {
        self.pending_pages.clear();
    }

    /// `\box<n>`（M3-5-3）：取出盒子寄存器；`\shipout` 前缀时封装为页面，
    /// 否则作为节点追加到当前列表。void 盒子报错（TeX "Box n is void"）。
    /// box255 = 待输出例程处理页面的队首。
    fn box_register(&mut self, idx: usize) -> Result<()> {
        let b = if idx == 255 {
            self.pending_pages.pop_front()
        } else {
            self.boxes.get_mut(idx).and_then(|s| s.take())
        };
        let Some(b) = b else {
            return Err(Error::invalid_input(format!("盒子 {idx} 为空（void）")));
        };
        if self.shipout_next {
            self.shipout_next = false;
            self.shipped.push(b);
            self.write_flush_pending = true;
        } else {
            self.append(Node::Box(b));
        }
        Ok(())
    }

    fn font_selected(&mut self, font: u32) -> Result<()> {
        // fn 指针模式恒为 FontId(0)；TFM 模式更新当前字体
        self.current_font = FontId(font);
        Ok(())
    }

    fn take_write_flush_pending(&mut self) -> bool {
        let v = self.write_flush_pending;
        self.write_flush_pending = false;
        v
    }

    fn glue(&mut self, g: Glue) -> Result<()> {
        // 数学模式 `\hskip`：转数学空格原子（TeX 数学模式 \hskip ≡ \mskip）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            self.math_push_atom(MathAtom::MSkip {
                width: g.width,
                stretch: g.stretch,
                shrink: g.shrink,
                nonscript: false,
            })?;
            return Ok(());
        }
        self.append(Node::Glue {
            width: g.width,
            stretch: g.stretch,
            shrink: g.shrink,
            stretch_order: 0,
            shrink_order: 0,
        });
        Ok(())
    }

    fn kern(&mut self, width: i64) -> Result<()> {
        // 数学模式 `\kern`：转数学空格原子（TeX 数学模式 \kern ≡ \mkern）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            self.math_push_atom(MathAtom::MSkip {
                width,
                stretch: 0,
                shrink: 0,
                nonscript: false,
            })?;
            return Ok(());
        }
        self.append(Node::Kern { width });
        Ok(())
    }

    fn penalty(&mut self, penalty: i64) -> Result<()> {
        // 数学模式 `\penalty`：M4-1 忽略（数学断行点后续补）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Ok(());
        }
        self.append(Node::Penalty { penalty });
        Ok(())
    }

    fn rule(&mut self, width: i64, height: i64, depth: i64) -> Result<()> {
        // 数学模式 `\vrule`：M4-1 忽略（规则原子后续补）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return Ok(());
        }
        self.append(Node::Rule { width, height, depth });
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// TFM 字体加载器（M3-4）：`\font` 执行时按名字查找/解析 TFM，追加到共享字体表。
#[derive(Debug)]
struct TfmLoader {
    table: Rc<RefCell<Vec<FontMetrics>>>,
}

impl FontLoader for TfmLoader {
    fn load(&mut self, name: &str, at: Option<i64>, scaled: Option<i64>) -> Result<u32> {
        if at.is_some() && scaled.is_some() {
            return Err(Error::invalid_input("\\font 的 at 与 scaled 不能同时给出"));
        }
        let path = ntex_font::find_tfm(name)
            .ok_or_else(|| Error::invalid_input(format!("找不到 TFM 文件：{name}")))?;
        let bytes = std::fs::read(&path).map_err(|e| Error::io("读取 TFM", path, e))?;
        let mut fm = ntex_font::parse_tfm(&bytes)
            .map_err(|e| Error::invalid_input(format!("解析 {name}: {e}")))?;
        fm.name = name.to_owned(); // DVI fnt_def 的字体名
        // at：目标尺寸/设计字号；scaled：千分比
        let fm = match (at, scaled) {
            (Some(at_sp), None) => {
                let den = fm.design_size_sp;
                if den <= 0 {
                    return Err(Error::invalid_input(format!("{name} 设计字号非法")));
                }
                fm.scaled_by(at_sp, den)
            }
            (None, Some(s)) => fm.scaled_by(s, 1000),
            _ => fm,
        };
        let mut table = self.table.borrow_mut();
        let id = u32::try_from(table.len())
            .map_err(|_| Error::internal("字体表溢出（> 2^32 字体）"))?;
        table.push(fm);
        Ok(id)
    }
}

/// 排版器：VM token 流 → 节点树（主垂直列表）。
pub struct Typesetter {
    expander: Expander,
    fonts: Fonts,
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
            fonts: Fonts::Fn {
                metrics,
                space: |_| Glue::ZERO,
            },
        }
    }

    /// 指定词间空白胶水函数（空格 token → 胶水；仅 fn 指针模式生效）。
    pub fn with_space(mut self, space: SpaceFn) -> Self {
        if let Fonts::Fn { space: s, .. } = &mut self.fonts {
            *s = space;
        }
        self
    }

    /// 注入 VFS 后端（RFC-3；`\input`/`\write` 等副作用原语的文件接口）。
    pub fn set_vfs(&mut self, vfs: Box<dyn ntex_io::Vfs>) {
        self.expander.set_vfs(vfs);
    }

    /// 取回 VFS（测试断言写入内容用）。
    pub fn take_vfs(&mut self) -> Box<dyn ntex_io::Vfs> {
        self.expander.take_vfs()
    }

    /// 导出展开引擎状态快照（`.fmt` v1；供 `ntex-format` 序列化）。
    pub fn export_state(&self) -> ntex_core::expand::FmtState {
        self.expander.export_state()
    }

    /// 加载展开引擎状态快照（`.fmt` v1）。
    pub fn import_state(&mut self, state: ntex_core::expand::FmtState) {
        self.expander.import_state(state);
    }

    /// TFM 字体模式（M3-4）：`\font\cs=cmr10` 加载真实度量，
    /// 字符维度/词间空白来自 TFM；`\font` 定义的 cs 作为字体选择器。
    pub fn with_tfm() -> Self {
        Self {
            expander: Expander::new(),
            fonts: Fonts::Tfm(Rc::new(RefCell::new(Vec::new()))),
        }
    }

    /// 排版源码，返回主垂直列表节点。
    pub fn typeset(&mut self, text: &str) -> Result<Vec<Node>> {
        self.install_font_loader();
        self.expander
            .set_sink(Box::new(NodeBuilder::new(self.fonts.clone())));
        self.expander.run_source(text)?;
        self.finish().map(|o| o.main)
    }

    /// 排版字节源码。
    pub fn typeset_bytes(&mut self, bytes: impl Into<Vec<u8>>) -> Result<Vec<Node>> {
        self.install_font_loader();
        self.expander
            .set_sink(Box::new(NodeBuilder::new(self.fonts.clone())));
        self.expander.feed_source(bytes);
        self.expander.run()?;
        self.finish().map(|o| o.main)
    }

    /// 排版源码并取回 `\shipout` 页面（DVI 输出，M3-5）：
    /// 返回 (页面列表, 字体表快照)。需 [`Self::with_tfm`] 模式（否则字体表为空）。
    /// 启用 M3-5-2 断页：顶层垂直列表经页面构建器自动分页（`\vsize`），
    /// 输入结束按 `\end` 语义冲页（`\hbox to \hsize{}\vfill\penalty-2^30`）。
    pub fn typeset_dvi(&mut self, text: &str) -> Result<(Vec<BoxNode>, Vec<FontMetrics>)> {
        self.install_font_loader();
        self.expander.set_sink(Box::new(NodeBuilder::with_pagination(
            self.fonts.clone(),
            true,
        )));
        self.expander.run_source(text)?;
        let out = self.finish()?;
        Ok((out.shipped, out.fonts))
    }

    /// TFM 模式：把共享字体表接给 VM 的 `\font` 加载器。
    fn install_font_loader(&mut self) {
        if let Fonts::Tfm(table) = &self.fonts {
            self.expander
                .set_font_loader(Box::new(TfmLoader { table: table.clone() }));
        }
    }

    /// 运行结束收尾：关闭开放段落、校验盒子/组闭合，冲掉残余页面，取回主列表与页面。
    ///
    /// 输出例程（M3-5-3）需要引擎在 token 边界执行，因此校验/冲页都在
    /// sink 仍挂接引擎时进行：close_paragraph / eject_one_page 产出的页面
    /// 经 box255+例程（或直通 shipout），随后 `run_pending_output` 执行例程。
    fn finish(&mut self) -> Result<FinishOutput> {
        // 1) 校验 + 关闭开放段落（可能产出页面 → box255 + pending）
        {
            let builder = self
                .expander
                .sink_mut()
                .as_any_mut()
                .downcast_mut::<NodeBuilder>()
                .ok_or_else(|| Error::internal("typesetter 安装了 NodeBuilder"))?;
            if builder.pending_box.is_some() {
                return Err(Error::invalid_input("\\hbox/\\vbox 后缺少组"));
            }
            if builder.shipout_next {
                return Err(Error::invalid_input("\\shipout 后缺少盒子"));
            }
            if !builder.groups.is_empty() {
                return Err(Error::invalid_input("组未闭合（缺少 }）"));
            }
            if !builder.math.is_empty() {
                return Err(Error::invalid_input("数学模式未闭合（缺少 $）"));
            }
            if builder.mode() == Mode::Horizontal {
                builder.close_paragraph();
            }
        }
        // 2) 执行 close_paragraph 产出的待执行输出例程
        self.expander.run_pending_output()?;
        // 3) 输入结束按 `\end` 冲掉残余页面（tex.web `its_all_over`），
        //    与输出例程交错：冲一页 → 执行例程 → 再冲。
        loop {
            let ejected = {
                let builder = self
                    .expander
                    .sink_mut()
                    .as_any_mut()
                    .downcast_mut::<NodeBuilder>()
                    .ok_or_else(|| Error::internal("typesetter 安装了 NodeBuilder"))?;
                if builder.pagination {
                    builder.eject_one_page()?
                } else {
                    false
                }
            };
            if !ejected {
                break;
            }
            self.expander.run_pending_output()?;
        }
        // 4) 取走 sink，收集主列表/页面/字体表
        let mut sink = self.expander.take_sink();
        let builder = sink
            .as_any_mut()
            .downcast_mut::<NodeBuilder>()
            .ok_or_else(|| Error::internal("typesetter 安装了 NodeBuilder"))?;
        let mut lists = std::mem::take(&mut builder.lists);
        debug_assert_eq!(lists.len(), 1, "收尾后应只剩主列表");
        let shipped = std::mem::take(&mut builder.shipped);
        let fonts = match &self.fonts {
            Fonts::Tfm(table) => table.borrow().clone(),
            Fonts::Fn { .. } => Vec::new(),
        };
        // RFC-3：排版结束收尾 flush 残留延迟写流（TeX \end final_cleanup 语义）
        self.expander.flush_writes()?;
        Ok(FinishOutput {
            main: lists.pop().expect("主列表"),
            shipped,
            fonts,
        })
    }
}

/// `finish` 的返回：主垂直列表 + `\shipout` 页面 + 字体表快照。
struct FinishOutput {
    main: Vec<Node>,
    shipped: Vec<BoxNode>,
    fonts: Vec<FontMetrics>,
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
        // 行盒 = [a, b, \parfillskip]（M3-5 对齐 TeX：parfillskip 留在末行）
        assert_eq!(p1.children.len(), 3);
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
        assert_eq!(as_box(&main[0]).children.len(), 3); // a b + \parfillskip
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
        assert_eq!(as_box(&main[0]).children.len(), 3); // a b + \parfillskip
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
        assert_eq!(para.children.len(), 3); // 缩进盒 + a + \parfillskip
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
        assert_eq!(para.children.len(), 4); // 缩进盒 + a + b + \parfillskip
        match &para.children[0] {
            Node::Box(b) => assert_eq!(b.width, 10 * SP_PER_PT),
            other => panic!("预期缩进空盒，得到 {other:?}"),
        }
    }

    #[test]
    fn noindent_suppresses_indent() {
        let main = typeset(r"\parindent 10pt\noindent ab").unwrap();
        let para = as_box(&main[0]);
        assert_eq!(para.children.len(), 3); // a + b + \parfillskip
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
            Node::Glue {
                width,
                stretch,
                shrink,
                ..
            } => {
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

    // ---------- M3-3 Knuth-Plass 段落折行 ----------

    #[test]
    fn paragraph_wraps_at_hsize() {
        // "ab cd" 总宽 5394sp、\hsize 4000sp：断点胶水不入行（tex.web try_break
        // 先于胶水累计调用），首行 "ab" 无内部胶水 → badness 10000（demerits 10⁸），
        // 单行（末行强制断点 d=0）更优 → 只折一行（与 pdfTeX 语义一致）
        let src = r"\hsize 4000sp ab cd";
        let main = Typesetter::with_metrics(metrics)
            .with_space(|_| Glue {
                width: 1000,
                stretch: 500,
                shrink: 300,
            })
            .typeset(src)
            .unwrap();
        let lines: Vec<&Node> = main.iter().filter(|n| matches!(n, Node::Box(_))).collect();
        assert_eq!(lines.len(), 1, "断点胶水不含入行时单行更优：{main:?}");
    }

    #[test]
    fn paragraph_single_line_when_fits() {
        let src = r"ab\par cd";
        let main = typeset(src).unwrap();
        // 默认 \hsize=6.5in 极大：两段各一行，段间 interline glue
        assert_eq!(main.len(), 3);
        assert!(matches!(main[0], Node::Box(_)));
        assert!(matches!(main[1], Node::Glue { .. }));
        assert!(matches!(main[2], Node::Box(_)));
    }

    // ---------- M3-4 TFM（cmr10） ----------

    /// 解析真实 cmr10 度量（无 TeX 安装则 None，测试跳过）。
    fn cmr10_metrics() -> Option<FontMetrics> {
        let path = ntex_font::find_tfm("cmr10")?;
        let bytes = std::fs::read(path).ok()?;
        ntex_font::parse_tfm(&bytes).ok()
    }

    fn tfm_chars(src: &str) -> (Vec<u32>, i64, i64, i64) {
        let mut ts = Typesetter::with_tfm();
        let main = ts.typeset(src).unwrap();
        assert_eq!(main.len(), 1);
        let b = as_box(&main[0]);
        // 行盒含 \parfillskip（M3-5 对齐 TeX）；字符宽度取字符节点之和
        let (chars, width) = chars_width(&b.children);
        (chars, width, b.height, b.depth)
    }

    #[test]
    fn tfm_char_metrics_from_cmr10() {
        let Some(fm) = cmr10_metrics() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let (chars, width, height, depth) = tfm_chars(r"\font\cmr=cmr10\cmr abc");
        assert_eq!(chars, vec![b'a' as u32, b'b' as u32, b'c' as u32]);
        let (wa, ha, da) = fm.char_metrics(b'a' as u32);
        let (wb, hb, db) = fm.char_metrics(b'b' as u32);
        let (wc, hc, dc) = fm.char_metrics(b'c' as u32);
        assert_eq!(width, wa + wb + wc);
        assert_eq!(height, ha.max(hb).max(hc));
        assert_eq!(depth, da.max(db).max(dc));
    }

    /// 行盒 children（\hbox{\cmr ...}；连字/字距/词间距均在此层）。
    fn tfm_line_children(text: &str) -> Vec<Node> {
        let mut ts = Typesetter::with_tfm();
        let main = ts.typeset(text).unwrap();
        assert_eq!(main.len(), 1);
        as_box(&main[0]).children.clone()
    }

    #[test]
    fn tfm_lig_kern_and_sfcode_applied() {
        if cmr10_metrics().is_none() {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        }
        // 连字：f + i → 单字符 12（fi）；v + e → 字距 -18205sp 插入在 'e' 前
        let children = tfm_line_children(r"\font\cmr=cmr10\cmr \hbox{fi ve}");
        match &children[0] {
            Node::Char { charcode, .. } => assert_eq!(*charcode, 12, "f+i 应连字为字符 12（fi）"),
            other => panic!("预期 Char，得到 {other:?}"),
        }
        assert_eq!(as_char(&children[2]), b'v' as u32);
        assert_eq!(
            children[3],
            Node::Kern { width: -18_205 },
            "v→e 应插入 -18205sp 字距"
        );
        assert_eq!(as_char(&children[4]), b'e' as u32);
        // \sfcode：逗号（sf=1250）后空格 stretch = round(space_stretch × 1250/1000)；
        // cmr10 space_stretch = 109226 sp → 136533
        let children = tfm_line_children(r"\font\cmr=cmr10\cmr \hbox{a, b}");
        match &children[2] {
            Node::Glue { stretch, .. } => {
                assert_eq!(*stretch, 136_533, "逗号后空格 stretch 按 sfcode 放大");
            }
            other => panic!("预期词间 Glue，得到 {other:?}"),
        }
    }

    #[test]
    fn tfm_at_scales_metrics() {
        let Some(fm) = cmr10_metrics() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        // at 12pt：缩放因子 = 12pt / 10pt（= 1.2）
        let scaled = fm.scaled_by(12 * SP_PER_PT, fm.design_size_sp);
        let (chars, width, height, _) = tfm_chars(r"\font\cmr=cmr10 at 12pt\cmr a");
        assert_eq!(chars, vec![b'a' as u32]);
        let (w, h, _) = scaled.char_metrics(b'a' as u32);
        assert_eq!(width, w);
        assert_eq!(height, h);
    }

    #[test]
    fn tfm_scaled_1200_matches() {
        let Some(fm) = cmr10_metrics() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let scaled = fm.scaled_by(1200, 1000);
        let (_, width, _, _) = tfm_chars(r"\font\cmr=cmr10 scaled 1200\cmr a");
        let (w, _, _) = scaled.char_metrics(b'a' as u32);
        assert_eq!(width, w);
    }

    #[test]
    fn tfm_space_glue_from_font_params() {
        let Some(fm) = cmr10_metrics() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let g = fm.space_glue();
        let mut ts = Typesetter::with_tfm();
        let main = ts.typeset(r"\font\cmr=cmr10\hbox{\cmr a b}").unwrap();
        let b = as_box(&main[0]);
        assert_eq!(b.children.len(), 3);
        match &b.children[1] {
            Node::Glue {
                width,
                stretch,
                shrink,
                ..
            } => {
                assert_eq!(*width, g.width, "词间距来自字体 space 参数");
                assert_eq!(*stretch, g.stretch);
                assert_eq!(*shrink, g.shrink);
            }
            other => panic!("预期词间 Glue，得到 {other:?}"),
        }
    }

    #[test]
    fn tfm_missing_font_errors() {
        let mut ts = Typesetter::with_tfm();
        assert!(ts.typeset(r"\font\x=definitely_not_a_font").is_err());
    }

    #[test]
    fn tfm_fonts_persist_across_typeset_calls() {
        let Some(fm) = cmr10_metrics() else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let mut ts = Typesetter::with_tfm();
        ts.typeset(r"\font\cmr=cmr10").unwrap();
        let (_, width, _, _) = tfm_chars_in(&mut ts, r"\cmr a");
        let (w, _, _) = fm.char_metrics(b'a' as u32);
        assert_eq!(width, w);
    }

    fn tfm_chars_in(ts: &mut Typesetter, src: &str) -> (Vec<u32>, i64, i64, i64) {
        let main = ts.typeset(src).unwrap();
        assert_eq!(main.len(), 1);
        let b = as_box(&main[0]);
        // 行盒含 \parfillskip 胶水（M3-5 对齐 TeX）；字符宽度取字符节点之和
        let (chars, width) = chars_width(&b.children);
        (chars, width, b.height, b.depth)
    }

    /// 行盒内的字符序列与自然宽度（忽略 \parfillskip 等胶水）。
    fn chars_width(children: &[Node]) -> (Vec<u32>, i64) {
        let mut w = 0i64;
        let mut chars = Vec::new();
        for c in children {
            match c {
                Node::Char {
                    charcode,
                    width,
                    ..
                } => {
                    chars.push(*charcode);
                    w += width;
                }
                _ => {}
            }
        }
        (chars, w)
    }

    // ---------- M3-5-3 \output 例程 + box255 ----------

    /// 分页排版（fn 指针度量 + 词间距）；返回 \shipout 页面。
    fn paginated(src: &str) -> Result<Vec<BoxNode>> {
        let mut ts = Typesetter::with_metrics(metrics).with_space(|_| Glue {
            width: 1000,
            stretch: 500,
            shrink: 300,
        });
        ts.typeset_dvi(src).map(|(pages, _)| pages)
    }

    /// 小 \vsize + 窄 \hsize 的填充文本（必然分多页）。
    fn fill_words() -> String {
        let words = [
            "aa", "bb", "cc", "dd", "ee", "ff", "gg", "hh", "ii", "jj", "kk", "ll", "mm", "nn",
            "oo", "pp", "qq", "rr", "ss", "tt", "uu", "vv", "ww", "xx", "yy", "zz",
        ];
        format!(
            r"\vsize 100000sp\hsize 20000sp {}",
            words.join(" ")
        )
    }

    #[test]
    fn output_routine_shipout_box255_equals_default() {
        let src = fill_words();
        let default = paginated(&src).unwrap();
        assert!(
            default.len() >= 2,
            "小 \\vsize 应分多页，实际 {}",
            default.len()
        );
        // \output={\shipout\box255} ≡ 默认 shipout
        let with = paginated(&format!(r"\output={{\shipout\box255}} {src}")).unwrap();
        assert_eq!(with, default, r"\output={{\shipout\box255}} 应与默认完全一致");
    }

    #[test]
    fn output_routine_empty_swallows_pages() {
        let src = format!(r"\output={{}} {}", fill_words());
        let pages = paginated(&src).unwrap();
        assert!(pages.is_empty(), "空 \\output 例程应吞掉所有页面");
    }

    #[test]
    fn output_routine_custom_header() {
        let src = fill_words();
        let default = paginated(&src).unwrap();
        // 例程给每页包一个页眉：\output={\shipout\vbox{\hbox{Header}\box255}}
        let with =
            paginated(&format!(r"\output={{\shipout\vbox{{\hbox{{Header}}\box255}}}} {src}"))
                .unwrap();
        assert_eq!(with.len(), default.len(), "例程不改变页数");
        for (p, d) in with.iter().zip(&default) {
            assert_eq!(p.children.len(), 2, "页面应包 Header + 原页：{p:?}");
            match &p.children[0] {
                Node::Box(h) => {
                    assert_eq!(h.children.len(), 6, "Header 为 6 字符 hbox");
                }
                other => panic!("首子节点应为 Header hbox，得到 {other:?}"),
            }
            assert_eq!(&p.children[1], &Node::Box(d.clone()), "box255 应为原页面");
        }
    }

    #[test]
    fn box_register_void_errors() {
        // \box255 在无页面（void）时取用 → 报错
        assert!(
            paginated(r"\shipout\box255").is_err(),
            "void \\box255 应报错"
        );
    }

    #[test]
    fn output_local_restores_after_group() {
        let src = fill_words();
        let default = paginated(&src).unwrap();
        // 组内定义 \output，组结束恢复未定义 → 行为与默认一致（直通 shipout）
        let with = paginated(&format!(r"{{\output={{\shipout\box255}}}} {src}")).unwrap();
        assert_eq!(with, default, "组结束应恢复未定义 \\output");
    }

    // ---------- M3 收尾（RFC-3）：VFS 集成 ----------

    fn ts_with_vfs() -> (Typesetter, ntex_io::MemVfs) {
        let mut ts = Typesetter::with_tfm();
        let vfs = ntex_io::MemVfs::new();
        ts.set_vfs(Box::new(vfs));
        let mut vfs = ts.take_vfs();
        let vfs = vfs
            .as_any_mut()
            .downcast_mut::<ntex_io::MemVfs>()
            .expect("MemVfs");
        (ts, vfs.clone())
    }

    #[test]
    fn vfs_input_splits_document() {
        let (mut ts, mut vfs) = ts_with_vfs();
        vfs.insert("ch1.tex", "Chapter One. ");
        vfs.insert("ch2.tex", "Chapter Two. ");
        ts.set_vfs(Box::new(vfs));
        let (pages, _) = ts
            .typeset_dvi("\\input{ch1}\\input{ch2}\\end")
            .unwrap();
        assert!(!pages.is_empty(), "\\input 分章文档应产出页面");
        // 页面文本应包含两章内容（合并后页数 ≥ 1，且文本含 Chapter）
        let mut text = String::new();
        for p in &pages {
            collect_text(p, &mut text);
        }
        assert!(text.contains("Chapter"), "页面应含输入文本：{text}");
    }

    #[test]
    fn vfs_write_flushed_on_shipout_boundary() {
        let (mut ts, vfs) = ts_with_vfs();
        ts.set_vfs(Box::new(vfs));
        // \write 延迟 → \shipout 边界 flush（first）→ 再 \write → \end flush（second）
        ts.typeset_dvi(
            "\\newwrite\\aux\\openout\\aux=o.aux\\write\\aux{first}\
             \\shipout\\hbox{A}\\write\\aux{second}\\end",
        )
        .unwrap();
        let mut vfs = ts.take_vfs();
        let vfs = vfs
            .as_any_mut()
            .downcast_mut::<ntex_io::MemVfs>()
            .expect("MemVfs");
        assert_eq!(vfs.get("o.aux"), Some(b"first\nsecond\n".as_slice()));
    }

    #[test]
    fn vfs_read_then_write_roundtrip() {
        let (mut ts, mut vfs) = ts_with_vfs();
        vfs.insert("data.txt", "42\n");
        ts.set_vfs(Box::new(vfs));
        // \read 一行 → \line，\write 回显（延迟，\end flush）
        ts.typeset_dvi(
            "\\newread\\r\\openin\\r=data.txt\\read\\r to \\line\
             \\newwrite\\w\\openout\\w=o.txt\\write\\w{\\line}\\end",
        )
        .unwrap();
        let mut vfs = ts.take_vfs();
        let vfs = vfs
            .as_any_mut()
            .downcast_mut::<ntex_io::MemVfs>()
            .expect("MemVfs");
        assert_eq!(vfs.get("o.txt"), Some(b"42\n".as_slice()));
    }

    /// 收集盒子树全部文本（集成断言用）。
    fn collect_text(b: &BoxNode, out: &mut String) {
        for c in &b.children {
            match c {
                Node::Char { charcode, .. } => {
                    out.push(char::from_u32(*charcode).unwrap_or('\u{FFFD}'))
                }
                Node::Box(inner) => collect_text(inner, out),
                _ => {}
            }
        }
    }

    #[test]
    fn fmt_snapshot_preserves_typeset_behavior() {
        // preamble（宏集）→ 快照 → 新排版器加载 → 同一文档排版一致
        let preamble = r"\def\emph#1{[#1]}\def\hi{Hi}";
        let doc = r"\emph{Hello} \hi there.";

        let mut ts1 = Typesetter::with_tfm();
        ts1.typeset_dvi(preamble).unwrap();
        let state = ts1.export_state();

        let mut ts2 = Typesetter::with_tfm();
        ts2.import_state(state);
        let (pages2, _) = ts2.typeset_dvi(doc).unwrap();

        let mut ts3 = Typesetter::with_tfm();
        let (pages3, _) = ts3.typeset_dvi(&format!("{preamble} {doc}")).unwrap();

        let text_of = |pages: &[BoxNode]| {
            let mut s = String::new();
            for p in pages {
                collect_text(p, &mut s);
            }
            s
        };
        assert_eq!(
            text_of(&pages2),
            text_of(&pages3),
            "加载 .fmt 快照后排版应与全新排版一致"
        );
    }

    // ---------- M4-1 数学模式 ----------

    /// 段落行盒 children（`$...$` 触发段落 → 主列表单行 hbox）。
    fn math_line_children(text: &str) -> Vec<Node> {
        let main = typeset(text).unwrap();
        assert_eq!(main.len(), 1, "数学公式应封装为单行段落：{text:?}");
        let line = as_box(&main[0]);
        // 去掉行尾 \parfillskip 胶水
        line.children
            .iter()
            .filter(|n| !matches!(n, Node::Glue { .. }))
            .cloned()
            .collect()
    }

    /// 段落行盒全部 children（含 spacing 胶水，排除行尾 \parfillskip；spacing 测试用）。
    fn math_line_all_children(text: &str) -> Vec<Node> {
        let main = typeset(text).unwrap();
        assert_eq!(main.len(), 1, "数学公式应封装为单行段落：{text:?}");
        let mut children = as_box(&main[0]).children.clone();
        // 去掉行尾 \parfillskip（0pt plus 1fil；hpack 拉伸后宽度非 0，按 fil 阶识别）
        while let Some(Node::Glue {
            stretch,
            stretch_order,
            ..
        }) = children.last()
        {
            if *stretch > 0 && *stretch_order == GLUE_ORDER_FIL {
                children.pop();
            } else {
                break;
            }
        }
        children
    }

    #[test]
    fn math_inline_formula_in_paragraph() {
        let children = math_line_children(r"$x$");
        assert_eq!(children.len(), 1);
        assert_eq!(as_char(&children[0]), b'x' as u32);
    }

    #[test]
    fn math_superscript_builds_script_box() {
        let children = math_line_children(r"$x^2$");
        assert_eq!(children.len(), 2, "x 后应挂上标盒");
        assert_eq!(as_char(&children[0]), b'x' as u32);
        let sup = as_box(&children[1]);
        assert_eq!(sup.kind, BoxKind::HBox);
        assert_eq!(sup.children.len(), 1);
        assert_eq!(as_char(&sup.children[0]), b'2' as u32);
        // 脚本字阶缩放：宽 7/10 × (1000+50)；高 7/10 × 6000
        assert_eq!(sup.children[0].dimensions().width, xn_over_d(1050, 7, 10));
        assert_eq!(sup.children[0].dimensions().height, 4200);
        // fn 指针模式 x_height=0 → 上标提升量 0（M4-3 用 fontdimen 精化）
        assert_eq!(sup.shift, 0);
    }

    #[test]
    fn math_sub_and_superscript_both() {
        let children = math_line_children(r"$x_1^2$");
        assert_eq!(children.len(), 3, "x + 上标盒 + 下标盒");
        assert_eq!(as_char(&children[0]), b'x' as u32);
        assert_eq!(as_char(&as_box(&children[1]).children[0]), b'2' as u32);
        assert_eq!(as_char(&as_box(&children[2]).children[0]), b'1' as u32);
    }

    #[test]
    fn math_sub_then_sup_attach_to_same_base() {
        // x_1^2 与 x^2_1 等价：同一 base 双侧脚本
        let a = math_line_children(r"$x_1^2$");
        let b = math_line_children(r"$x^2_1$");
        assert_eq!(a, b);
    }

    #[test]
    fn math_group_script_field() {
        let children = math_line_children(r"$x^{ab}$");
        assert_eq!(children.len(), 2);
        let sup = as_box(&children[1]);
        assert_eq!(sup.children.len(), 2);
        assert_eq!(as_char(&sup.children[0]), b'a' as u32);
        assert_eq!(as_char(&sup.children[1]), b'b' as u32);
    }

    #[test]
    fn math_group_splices_into_list() {
        // {ab} 数学组直接并入外层（等价 ab）
        let a = math_line_children(r"${ab}$");
        let b = math_line_children(r"$ab$");
        assert_eq!(a, b);
    }

    #[test]
    fn math_ignores_spaces() {
        let children = math_line_children(r"$a b$");
        assert_eq!(children.len(), 2, "数学模式空格应忽略");
        assert_eq!(as_char(&children[0]), b'a' as u32);
        assert_eq!(as_char(&children[1]), b'b' as u32);
    }

    #[test]
    fn math_nested_scripts_use_scriptscript_scale() {
        // x_{y^z}：z 为第三级脚本（5/10 缩放）
        let children = math_line_children(r"$x_{y^z}$");
        assert_eq!(children.len(), 2);
        let sub = as_box(&children[1]);
        assert_eq!(sub.children.len(), 2, "y + z 上标盒");
        let z = &sub.children[1];
        let zw = z.dimensions().width;
        assert_eq!(zw, xn_over_d(1000 + b'z' as i64, 5, 10));
    }

    #[test]
    fn math_display_formula() {
        let main = typeset(r"$$x$$").unwrap();
        assert_eq!(main.len(), 1);
        let line = as_box(&main[0]);
        assert_eq!(line.children.len(), 2, "x + parfillskip");
        let cx = &line.children[0];
        // 显示样式：不缩放（Display → 1,1）
        assert_eq!(cx.dimensions().width, 1000 + b'x' as i64);
    }

    #[test]
    fn math_display_requires_double_dollar_end() {
        assert!(typeset(r"$$x$").is_err(), "显示数学必须以 $$ 结束");
    }

    #[test]
    fn math_unclosed_formula_rejected() {
        let err = typeset(r"$x").unwrap_err();
        assert!(
            err.to_string().contains("数学模式未闭合"),
            "未闭合公式应报错：{err}"
        );
    }

    #[test]
    fn math_script_without_base_rejected() {
        assert!(typeset(r"$^2$").is_err(), "^ 前缺原子应报错");
        assert!(typeset(r"$_{2}$").is_err(), "_ 前缺原子应报错");
    }

    #[test]
    fn math_double_superscript_rejected() {
        assert!(typeset(r"$x^2^3$").is_err(), "双重上标应报错");
    }

    #[test]
    fn math_empty_inline_is_fine() {
        // 空行内公式 $ $ 不产生节点
        let main = typeset(r"a $ $ b").unwrap();
        let line = as_box(&main[0]);
        let chars: Vec<u32> = line
            .children
            .iter()
            .filter_map(|n| match n {
                Node::Char { charcode, .. } => Some(*charcode),
                _ => None,
            })
            .collect();
        assert_eq!(chars, vec![b'a' as u32, b'b' as u32]);
    }

    // ---------- M4-2 数学原语 ----------

    /// 分式盒（`\over`/`\atop`）：vbox = [num 盒, glue, rule?, glue, den 盒]。
    fn fraction_box(text: &str) -> (BoxNode, bool) {
        let children = math_line_children(text);
        assert_eq!(children.len(), 1, "分式应封装为单盒：{text:?}");
        let b = as_box(&children[0]);
        assert_eq!(b.kind, BoxKind::VBox, "分式是垂直堆叠");
        let has_rule = b.children.iter().any(|n| matches!(n, Node::Rule { .. }));
        (b.clone(), has_rule)
    }

    #[test]
    fn math_fraction_over_builds_vbox() {
        let (b, has_rule) = fraction_box(r"$a\over b$");
        assert!(has_rule, "\\over 应画分式线");
        // num 盒含 a、den 盒含 b
        assert_eq!(b.children.len(), 5);
        let num = as_box(&b.children[0]);
        assert_eq!(as_char(&num.children[0]), b'a' as u32);
        let den = as_box(&b.children[4]);
        assert_eq!(as_char(&den.children[0]), b'b' as u32);
    }

    #[test]
    fn math_atop_has_no_rule() {
        let (_, has_rule) = fraction_box(r"$a\atop b$");
        assert!(!has_rule, "\\atop 无线");
    }

    #[test]
    fn math_fraction_in_group() {
        let a = math_line_children(r"${a\over b}$");
        let b = math_line_children(r"$a\over b$");
        assert_eq!(a, b, "组内分式应并入外层");
    }

    #[test]
    fn math_fraction_with_scripts_in_sup() {
        // x^{a\over b}：上标字段内含分式
        let children = math_line_children(r"$x^{a\over b}$");
        assert_eq!(children.len(), 2);
        let sup = as_box(&children[1]);
        assert_eq!(sup.children.len(), 1, "上标字段 = 单个分式盒");
        assert!(matches!(sup.children[0], Node::Box(_)), "上标内应为分式盒");
        let frac = as_box(&sup.children[0]);
        assert_eq!(frac.kind, BoxKind::VBox);
        assert!(frac.children.iter().any(|n| matches!(n, Node::Rule { .. })));
    }

    #[test]
    fn math_sqrt_radical_box() {
        let children = math_line_children(r"$\sqrt{x}$");
        assert_eq!(children.len(), 1);
        let b = as_box(&children[0]);
        assert_eq!(b.kind, BoxKind::VBox);
        // [横线 rule, glue, 内容盒]
        assert!(matches!(b.children[0], Node::Rule { .. }));
        let base = as_box(&b.children[2]);
        assert_eq!(as_char(&base.children[0]), b'x' as u32);
    }

    #[test]
    fn math_sqrt_single_atom() {
        let children = math_line_children(r"$\sqrt x$");
        assert_eq!(children.len(), 1);
        assert_eq!(as_box(&children[0]).kind, BoxKind::VBox);
    }

    #[test]
    fn math_left_right_delimited() {
        let children = math_line_children(r"$\left(x\right)$");
        // 定界符 + body + 定界符
        assert_eq!(children.len(), 3);
        assert_eq!(as_char(&children[0]), b'(' as u32);
        assert_eq!(as_char(&children[1]), b'x' as u32);
        assert_eq!(as_char(&children[2]), b')' as u32);
    }

    #[test]
    fn math_left_right_dot_empty_delims() {
        let children = math_line_children(r"$\left.x\right.$");
        assert_eq!(children.len(), 1, "空定界符不产生字符");
        assert_eq!(as_char(&children[0]), b'x' as u32);
    }

    #[test]
    fn math_style_affects_scale() {
        let children = math_line_children(r"$x{\scriptstyle y}$");
        assert_eq!(children.len(), 2);
        // y 在 script 样式：宽度 7/10 × (1000+121)
        assert_eq!(children[1].dimensions().width, xn_over_d(1121, 7, 10));
    }

    #[test]
    fn math_bin_class_inserts_medskip() {
        // 默认 `+` 是 Ord → 无间距；\mathbin+ → Bin → 两侧 medmuskip
        // （fn 指针模式 quad=0 → 胶水宽 0，但 Glue 节点仍插入）
        let plain = math_line_all_children(r"$a+b$");
        let bin = math_line_all_children(r"$a\mathbin+b$");
        assert_eq!(plain.len(), 3, "全 Ord 无胶水");
        assert_eq!(bin.len(), 5, "\\mathbin+ 两侧插入 medmuskip 胶水");
        assert!(matches!(bin[1], Node::Glue { .. }));
        assert!(matches!(bin[3], Node::Glue { .. }));
        // \mathbin{+} 组形式等价
        let bin_group = math_line_all_children(r"$a\mathbin{+}b$");
        assert_eq!(bin_group.len(), 5);
        assert!(matches!(bin_group[1], Node::Glue { .. }));
    }

    #[test]
    fn math_rel_class_inserts_thickmuskip() {
        let rel = math_line_all_children(r"$a\mathrel=b$");
        assert_eq!(rel.len(), 5, "ord+rel+ord → 两侧 thickmuskip");
        assert!(matches!(rel[1], Node::Glue { .. }));
        assert!(matches!(rel[3], Node::Glue { .. }));
    }

    #[test]
    fn math_over_outside_math_rejected() {
        assert!(typeset(r"a\over b").is_err(), "\\over 只能在数学模式");
    }

    #[test]
    fn math_over_ambiguous_rejected() {
        assert!(typeset(r"$a\over b\over c$").is_err(), "连续 \\over 应歧义报错");
    }

    #[test]
    fn math_left_without_right_rejected() {
        assert!(typeset(r"$\left(x$").is_err(), "\\left 必须配 \\right");
    }

    #[test]
    fn math_right_without_left_rejected() {
        assert!(typeset(r"$x\right)$").is_err(), "\\right 前必须有 \\left");
    }

    #[test]
    fn math_over_empty_denominator_is_fine() {
        // TeX 允许空分母：$a\over$ → 分式盒（只有分子）
        let (_, has_rule) = fraction_box(r"$a\over$");
        assert!(has_rule);
    }
}
