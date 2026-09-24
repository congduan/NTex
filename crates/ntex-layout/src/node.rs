//! 排版节点模型（M3-1）。
//!
//! 语义对齐 TeX the Program 的节点体系，用 Rust enum 表达：
//!
//! | TeX 节点        | 本模块                  |
//! |-----------------|-------------------------|
//! | `char_node`     | [`Node::Char`]          |
//! | `hlist/vlist`   | [`Node::Box`]           |
//! | `rule_node`     | [`Node::Rule`]          |
//! | `glue_node`     | [`Node::Glue`]          |
//! | `kern_node`     | [`Node::Kern`]          |
//! | `penalty_node`  | [`Node::Penalty`]       |
//! | `leader_node`   | [`Node::Leaders`]       |
//!
//! 所有维度（width/height/depth）与胶水量统一以 scaled point（sp）为单位，
//! `1pt = 65536sp`（见 [`ntex_core::register::SP_PER_PT`]），与展开引擎的
//! 寄存器模型一致。
//!
//! # 盒子维度（TeXbook p.81 / tex.web hpackage / vpackage）
//!
//! **hbox**：`width` = 子节点 width 之和；`height` = 有纵向维度的子节点
//! （char/box/rule/leaders）height 最大值；`depth` = 同集合 depth 最大值。
//!
//! **vbox**：`width` = 子节点 width 最大值；`height + depth` 总和 =
//! 有纵向维度的子节点各自 `height + depth` 之和；`height` = 第一个有纵向维度
//! 的子节点的 height（无则 0）；`depth` = 总和 − height。
//!
//! 胶水/字距/惩罚不参与 height/depth：水平列表里它们有 width，垂直列表里无维度。

use ntex_core::register::Glue;

/// 字体标识：由字体表（M3-4 TFM 解析）分配；`FontId(0)` 为默认字体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontId(pub u32);

/// 盒子种类（`\hbox` / `\vbox`；`\vtop` 通过 [`BoxNode`] 的 `shift` 表达）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxKind {
    HBox,
    VBox,
}

/// 盒子的宽/高/深（单位 sp）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BoxDimensions {
    pub width: i64,
    pub height: i64,
    pub depth: i64,
}

impl BoxDimensions {
    /// 全零维度。
    pub const ZERO: Self = Self {
        width: 0,
        height: 0,
        depth: 0,
    };

    /// height + depth：垂直方向总占据。
    pub fn total(self) -> i64 {
        self.height + self.depth
    }
}

/// 引导符（`\leaders`/`\cleaders`/`\xleaders`）种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeadersKind {
    /// `\leaders`：被重复的 box 与间隙等宽。
    Leaders,
    /// `\cleaders`：box 居中。
    Cleaders,
    /// `\xleaders`：box 与间隙交替至两端。
    Xleaders,
}

/// 胶水无穷阶（tex.web `glue_ord`）：0=普通，1=fil，2=fill，3=filll。
pub type GlueOrder = u8;

/// `fil` 阶常量。
pub const GLUE_ORDER_FIL: GlueOrder = 1;

/// `fill` 阶常量（`\hfill`/`\vfill` 等）。
pub const GLUE_ORDER_FILL: GlueOrder = 2;

/// 盒子节点：维度在构建时固化并存储（与 TeX 的 box 节点一致），
/// `\raise`/`\lower`/`\vtop` 等通过 `shift` 表达参考点位移。
#[derive(Debug, Clone, PartialEq)]
pub struct BoxNode {
    pub kind: BoxKind,
    pub width: i64,
    pub height: i64,
    pub depth: i64,
    /// 参考点位移（sp），默认 0。
    pub shift: i64,
    pub children: Vec<Node>,
}

impl BoxNode {
    /// 构建 hbox，维度按 [`hbox_dimensions`] 计算。
    pub fn new_hbox(children: Vec<Node>) -> Self {
        let d = hbox_dimensions(&children);
        Self {
            kind: BoxKind::HBox,
            width: d.width,
            height: d.height,
            depth: d.depth,
            shift: 0,
            children,
        }
    }

    /// 构建 vbox，维度按 [`vbox_dimensions`] 计算。
    pub fn new_vbox(children: Vec<Node>) -> Self {
        let d = vbox_dimensions(&children);
        Self {
            kind: BoxKind::VBox,
            width: d.width,
            height: d.height,
            depth: d.depth,
            shift: 0,
            children,
        }
    }

    /// 已固化的维度。
    pub fn dimensions(&self) -> BoxDimensions {
        BoxDimensions {
            width: self.width,
            height: self.height,
            depth: self.depth,
        }
    }

    /// 零尺寸且无子节点的空盒。
    ///
    /// 页构建器用它区分 LaTeX `\clearpage` 在空页上留下的 `\vbox{}` 与真正
    /// 建立 `page_contents` 的盒子；`\hbox to \hsize{}` 这类有宽度的 eject
    /// 材料不属于此类。
    pub fn is_zero_empty(&self) -> bool {
        self.children.is_empty() && self.width == 0 && self.height == 0 && self.depth == 0
    }
}

/// 排版节点（TeX 节点体系的 Rust 表达）。
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// 字符：维度来自字体表（TFM，M3-4 实现）；在此由调用方填充。
    Char {
        font: FontId,
        charcode: u32,
        width: i64,
        height: i64,
        depth: i64,
    },
    /// 连字节点（tex.web ligature_node；lastnodetype=7）：`x y =: z` 程序匹配后
    /// 左字符替换为结果字符、右字符被消费。`components` = 组成字符序列
    /// （showbox 显示 `.\trip r (ligature u|)`——组成用 `|` 连接）。
    /// 连续连字（ffi）时尾 Ligature 继续参与匹配，components 累积。
    Ligature {
        font: FontId,
        charcode: u32,
        width: i64,
        height: i64,
        depth: i64,
        components: Vec<u8>,
    },
    /// 盒子（hlist / vlist）。
    Box(BoxNode),
    /// 规则（`\hrule` / `\vrule`）。
    Rule {
        width: i64,
        height: i64,
        depth: i64,
    },
    /// 胶水：可拉伸 / 可收缩。
    Glue {
        /// 来源名（\thinmuskip/\medmuskip/\thickmuskip 等；showbox 显示
        /// `\glue(\thinmuskip)`——tex.web glue_spec 的来源标记）。
        name: Option<&'static str>,
        width: i64,
        stretch: i64,
        shrink: i64,
        /// 拉伸无穷阶（0=普通，1=fil，2=fill，3=filll）。
        stretch_order: GlueOrder,
        /// 收缩无穷阶。
        shrink_order: GlueOrder,
    },
    /// 字距（不可拉伸）。
    Kern {
        width: i64,
    },
    /// 断行惩罚；`penalty < 0` 表示可选断行点，`penalty >= 10000` 禁止断行。
    Penalty {
        penalty: i64,
    },
    /// 引导符：行为类似胶水（width/stretch/shrink），内部重复 box 提供 height/depth。
    Leaders {
        kind: LeadersKind,
        /// 被重复的 box（或 rule——`\leaders\hrule\hskip10pt`，TeX 允许 rule 作引导内容）。
        inner: Box<Node>,
        /// 胶水规格。tex.web 的引导符节点就是 glue_node（subtype=leader 旗标 +
        /// leader_ptr），故拉伸/收缩连同**无穷阶**一并携带——`\@dottedtocline`
        /// 行的 `\hfill`（1fill）须压过 `\parfillskip=-\rightskip` 的 -1fil，
        /// 阶丢失会让 hpack 按 fil 阶结算、点线不铺。
        width: i64,
        stretch: i64,
        shrink: i64,
        stretch_order: GlueOrder,
        shrink_order: GlueOrder,
    },
    /// 断字节点（M4-6）：三段式 pre/post/replace。
    ///
    /// - 折行断在该点：当前行行尾取 `pre`（含连字符）、下一行行首取 `post`；
    /// - 未断：取 `replace`（本实现中字母留在主列表，pre 仅含连字符、post/replace 为空，
    ///   故未断时自身宽度为 0，连字符只在断点处计入行宽）。
    Discretionary {
        pre: Vec<Node>,
        post: Vec<Node>,
        replace: Vec<Node>,
    },
    /// TeXXeT 方向节点（e-TeX `\beginL`/`\endL`/`\beginR`/`\endR`；宽度 0）。
    Direction {
        kind: ntex_core::sink::DirectionKind,
    },
    /// mark 节点（`\mark`/`\marks<n>`；无维度）：class 为 `\marks` 的寄存器号，
    /// `\mark` 为 None。
    Mark {
        class: Option<i64>,
        text: String,
    },
    /// insert 节点（`\insert<num>{...}`；无维度）：class 为插入寄存器号。
    ///
    /// tex.web `begin_insert_or_adjust`：类号扫入后 `new_save_level(insert_group)`、
    /// `push_nest; mode:=-vmode`——组体在**内部垂直模式**由主循环照常排版，
    /// `}` 处（insert_group）`end_graf; vpack(natural)` 后挂上 `ins_node`，
    /// `\splittopskip`/`\splitmaxdepth`/`\floatingpenalty` 同站读入。
    /// NTex 同构：组体经 [`crate::typeset::GroupKind::Insert`] 组在主循环执行，
    /// 此处 `body` 即排版后的 vlist（脚注文本由此真正可回流到页底）。
    Ins {
        class: usize,
        /// 组体排版后的垂直列表（tex.web `ins_ptr`；`\ifvoid`/`\unvbox` 的回流内容）
        body: BoxNode,
        /// `\splittopskip`（tex.web `split_top_ptr`）
        split_top_skip: Glue,
        /// `\splitmaxdepth`（tex.web `depth`）
        split_max_depth: i64,
        /// `\floatingpenalty`（tex.web `float_cost`）
        float_cost: i64,
    },
    /// adjust 节点（`\vadjust{<vertical material>}`；无维度）。
    Adjust {
        text: String,
    },
    /// whatsit 节点（`\write<n>{...}` 与 `\special{...}`；无维度）。
    /// tex.web 里两者同型（write_node/special_node 都归 whatsit），但 shipout
    /// 出口不同：`\write` 写流（不进 DVI），`\special` 经 DVI xxx 落后端——
    /// `special: true` 标后者（DVI 写出器据此发 xxx，见 ntex-dvi hlist/vlist）。
    Whatsit {
        text: String,
        /// `\special` 通道（shipout → DVI xxx）；false = 延迟 `\write`。
        special: bool,
    },
    /// 行内数学边界标记（tex.web math_node）：`$` 进入/退出时插入；
    /// 无维度。`surrounded` = 当时的 `\\mathsurround`（showbox 显示
    /// `\\mathon, surrounded 12.3`）。
    MathOn {
        surrounded: i64,
    },
    MathOff {
        surrounded: i64,
    },
}

impl Node {
    /// 该节点的 width/height/depth。
    pub fn dimensions(&self) -> BoxDimensions {
        match self {
            Node::Char {
                width,
                height,
                depth,
                ..
            } => BoxDimensions {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Node::Ligature {
                width,
                height,
                depth,
                ..
            } => BoxDimensions {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Node::Box(b) => b.dimensions(),
            Node::Rule {
                width,
                height,
                depth,
            } => BoxDimensions {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Node::Glue { width, .. } => BoxDimensions {
                width: *width,
                height: 0,
                depth: 0,
            },
            Node::Kern { width } => BoxDimensions {
                width: *width,
                height: 0,
                depth: 0,
            },
            Node::Penalty { .. } => BoxDimensions::ZERO,
            Node::Leaders { width, inner, .. } => {
                let dims = inner.dimensions();
                BoxDimensions {
                    width: *width,
                    height: dims.height,
                    depth: dims.depth,
                }
            }
            // 断字节点：未断时宽度 = replace（本实现为空 → 0）；连字符只在断点计入行宽。
            Node::Discretionary { replace, .. } => {
                if replace.is_empty() {
                    BoxDimensions::ZERO
                } else {
                    hbox_dimensions(replace)
                }
            }
            // 方向节点：宽度 0（TeX：begin_L/end_L 无维度）。
            Node::Direction { .. } => BoxDimensions::ZERO,
            // mark 节点：无维度。
            Node::Mark { .. } => BoxDimensions::ZERO,
            // insert/adjust/whatsit 节点：无维度。
            Node::Ins { .. }
            | Node::Adjust { .. }
            | Node::Whatsit { .. }
            | Node::MathOn { .. }
            | Node::MathOff { .. } => BoxDimensions::ZERO,
        }
    }

    /// e-TeX `\lastnodetype` 节点类型码（char=0 ... penalty=13）。
    ///
    /// 对齐 e-TeX 的 node type 编号：char 0、hlist 1、vlist 2、rule 3、ins 4、
    /// mark 5、adjust 6、disc 8、whatsit 9、math 10、glue 11、kern 12、
    /// penalty 13。方向节点在 e-TeX 中是 math_node 子类型（10）。
    pub fn node_type_code(&self) -> i64 {
        match self {
            Node::Char { .. } => 0,
            // tex.web：ligature_node 类型码 7
            Node::Ligature { .. } => 7,
            Node::Box(b) => match b.kind {
                BoxKind::HBox => 1,
                BoxKind::VBox => 2,
            },
            Node::Rule { .. } => 3,
            Node::Ins { .. } => 4,
            Node::Mark { .. } => 5,
            Node::Adjust { .. } => 6,
            Node::Discretionary { .. } => 8,
            Node::Whatsit { .. } => 9,
            // tex.web：math_node（\mathon/\mathoff）类型码 10；15 是伪节点
            // math_mode_node（空数学列表的 lastnodetype 值），不是真实节点类型
            Node::MathOn { .. } | Node::MathOff { .. } => 10,
            Node::Direction { .. } => 10,
            Node::Glue { .. } | Node::Leaders { .. } => 11,
            Node::Kern { .. } => 12,
            Node::Penalty { .. } => 13,
        }
    }

    /// 是否具有纵向维度（参与盒子 height/depth 计算）。
    pub fn has_vertical_extent(&self) -> bool {
        matches!(
            self,
            Node::Char { .. }
                | Node::Ligature { .. }
                | Node::Box(_)
                | Node::Rule { .. }
                | Node::Leaders { .. }
        )
    }

    /// 是否可丢弃节点（折行时 glue/kern/penalty 在断行点可被丢弃）。
    pub fn is_discardable(&self) -> bool {
        matches!(
            self,
            Node::Glue { .. } | Node::Kern { .. } | Node::Penalty { .. }
        )
    }
}

/// hbox 维度计算（TeXbook p.81 / tex.web `hpackage`）：
/// `width` = Σ 子节点 width；`height`/`depth` = 有纵向维度的子节点各自的最大值。
pub fn hbox_dimensions(children: &[Node]) -> BoxDimensions {
    let mut dims = BoxDimensions::ZERO;
    for c in children {
        let d = c.dimensions();
        dims.width += d.width;
        match c {
            // tex.web `@<Incorporate box dimensions...@>`（L12976）：盒的
            // shift 计入垂直占比——`h:=height-s`、`d:=depth+s`（s 仅盒有，
            // 规则/字符恒 0）；初值 0、严格大于才更新，负占比不回压下界。
            Node::Box(b) => {
                dims.height = dims.height.max(b.height - b.shift);
                dims.depth = dims.depth.max(b.depth + b.shift);
            }
            _ if c.has_vertical_extent() => {
                dims.height = dims.height.max(d.height);
                dims.depth = dims.depth.max(d.depth);
            }
            _ => {}
        }
    }
    dims
}

/// vbox 维度计算（tex.web `vpackage` L13175-13211 的自然维度归并）：
/// `width` = max 子节点 width；纵向用 (x, d) 二元组推进——
///
/// - 盒/规则：`x += d + height; d = depth`（把上一件的挂起深度结转进自然高）；
/// - **glue/kern**：`x += d + width; d = 0`（tex.web L13196/L13209——胶水宽度
///   计入自然高）。vbox 里的 `\vskip` 与断行插入的行间 `\baselineskip` glue
///   都必须占高，否则多行段落折进 `\vtop` 时行间 glue 归零、行盒相互贴死
///   （resume-plain.tex 换行条目行距消失的根因）；
/// - leaders 兼具 glue 推进（按 width）与宽度归并；其余节点不推进。
///
/// `height` = x（自然高，不含末件挂起深度）；`depth` = d（最后一件的深度）。
/// 空列表 → 零维度。
pub fn vbox_dimensions(children: &[Node]) -> BoxDimensions {
    let mut width = 0;
    let mut x = 0; // 已结转的自然高（tex.web 的 x）
    let mut d = 0; // 挂起深度（tex.web 的 d：最后一件的 depth，下一件到来时结转）
    for c in children {
        let dims = c.dimensions();
        width = width.max(dims.width);
        match c {
            Node::Box(b) => {
                x += d + b.height;
                d = b.depth;
            }
            Node::Rule { height, depth, .. } => {
                x += d + height;
                d = *depth;
            }
            // 纵向字符/连字（实际 vlist 不出现，测试与既有行为保留）：与盒同规
            Node::Char { .. } | Node::Ligature { .. } => {
                x += d + dims.height;
                d = dims.depth;
            }
            Node::Glue { width: gw, .. } => {
                x += d + gw;
                d = 0;
            }
            Node::Kern { width: kw } => {
                x += d + kw;
                d = 0;
            }
            Node::Leaders { width: lw, .. } => {
                x += d + lw;
                d = 0;
            }
            _ => {}
        }
    }
    BoxDimensions {
        width,
        height: x,
        depth: d,
    }
}

/// `vpack`（tex.web §661 "vpackage"）：把垂直列表打包为总高（height+depth）**恰好**
/// `height` 的 vbox；`max_depth` 为深度上限（tex.web vpackage 第四参——\vbox 的
/// boxmaxdepth；自然深度超限 → 深度钳到限制）。
///
/// 占位度量阶段简化：差额直接调整维度（高度优先，超 `maxdepth` 语义部分建模——
/// 只钳 depth 不动 height；exactly 目标下差额由高度吸收（TRIP L316
/// `\vbox to10pt{\boxmaxdepth=-1pt\mark{vii}}` → (10.0+-1.0) 对齐参考））。
/// 无差额时与 [`BoxNode::new_vbox`] 等价。
pub fn vpack(children: Vec<Node>, height: i64, max_depth: i64) -> BoxNode {
    // tex.web vpackage：`list_ptr(r):=p`——整表保留，**不剥前导 discardable**。
    // 真 pdflatex `\@outputpage` 的 ship 盒首子 `.\glue 16.0`（\topmargin）为证。
    // （旧版曾在此剥前导 glue，与 package_box 侧的 vbox_dimensions 全表度量
    // 相矛盾：glue 计入目标高、却被剥出 children——高度被"幻影"烘焙、glue 节点
    // 丢失，页盒 y 定位短 16pt。）
    let natural = vbox_dimensions(&children);
    let diff = height - (natural.height + natural.depth);
    let adjusted = vpack_adjust_glue(&children, diff);
    let adjusted_dims = vbox_dimensions(&adjusted);
    let mut b = BoxNode::new_vbox(adjusted);
    let residual = height - (adjusted_dims.height + adjusted_dims.depth);
    if residual >= 0 {
        b.height += residual;
    } else {
        let dh = b.height.min(-residual);
        b.height -= dh;
        let dd = (-residual - dh).min(b.depth);
        b.depth -= dd;
    }
    // tex.web vpackage：自然深度超 max_depth 限制 → 深度钳到限制（超出部分
    // tex.web 转进高度差额 x，exactly 目标下被 glue set 吸收——简化只钳 depth）
    if b.depth > max_depth {
        b.depth = max_depth;
    }
    b
}

fn vpack_adjust_glue(children: &[Node], diff: i64) -> Vec<Node> {
    let mut total_stretch = [0i64; 4];
    let mut total_shrink = [0i64; 4];
    for c in children {
        if let Node::Glue {
            stretch,
            shrink,
            stretch_order,
            shrink_order,
            ..
        } = c
        {
            total_stretch[*stretch_order as usize] += stretch;
            total_shrink[*shrink_order as usize] += shrink;
        }
    }
    #[derive(Clone, Copy)]
    enum Sign {
        Normal,
        Stretch,
        Shrink,
    }
    let (sign, order, gs) = if diff == 0 {
        (Sign::Normal, 0, 0.0)
    } else if diff > 0 {
        match (0..4).rev().find(|&o| total_stretch[o] != 0) {
            Some(o) => (Sign::Stretch, o, diff as f64 / total_stretch[o] as f64),
            None => (Sign::Normal, 0, 0.0),
        }
    } else {
        match (0..4).rev().find(|&o| total_shrink[o] != 0) {
            Some(o) => {
                let gs = (-diff) as f64 / total_shrink[o] as f64;
                let gs = if o == 0 && total_shrink[o] < -diff {
                    1.0
                } else {
                    gs
                };
                (Sign::Shrink, o, gs)
            }
            None => (Sign::Normal, 0, 0.0),
        }
    };
    let mut out = Vec::with_capacity(children.len());
    let mut cum = 0f64;
    let mut prev_g = 0f64;
    for c in children {
        match c {
            Node::Glue {
                name,
                width,
                stretch,
                shrink,
                stretch_order,
                shrink_order,
            } => {
                let mut width = *width;
                match sign {
                    Sign::Stretch if *stretch_order as usize == order => {
                        cum += *stretch as f64;
                        let g = (gs * cum).round();
                        width += g as i64 - prev_g as i64;
                        prev_g = g;
                    }
                    Sign::Shrink if *shrink_order as usize == order => {
                        cum -= *shrink as f64;
                        let g = (gs * cum).round();
                        width += g as i64 - prev_g as i64;
                        prev_g = g;
                    }
                    _ => {}
                }
                out.push(Node::Glue {
                    name: *name,
                    width,
                    stretch: *stretch,
                    shrink: *shrink,
                    stretch_order: *stretch_order,
                    shrink_order: *shrink_order,
                });
            }
            other => out.push(other.clone()),
        }
    }
    out
}

/// `vsplit`（tex.web §1168）：把 vbox 从顶部切出高为 `height` 的部分。
/// 返回 (顶部结果, 底部余量)。纵向距离累计：盒子/规则按 h+d、胶水/字距按 width。
pub fn split_vbox(b: BoxNode, height: i64) -> (BoxNode, BoxNode) {
    let mut acc = 0i64;
    let mut split = b.children.len();
    for (i, c) in b.children.iter().enumerate() {
        let d = match c {
            Node::Glue { width, .. } | Node::Kern { width } => *width,
            Node::Box(bx) => bx.height + bx.depth,
            Node::Rule {
                height: h, depth, ..
            } => h + depth,
            _ => c.dimensions().total(),
        };
        acc += d;
        if acc >= height {
            split = i + 1;
            break;
        }
    }
    let top = b.children[..split].to_vec();
    let rest = b.children[split..].to_vec();
    (vpack(top, height, i64::MAX), BoxNode::new_vbox(rest))
}

/// `hpack`（tex.web §656 "hpackage"）：把水平列表打包成**恰好** `width` 宽的 hbox。
///
/// 行盒语义（`\hbox to \hsize`）：宽度锁定为目标值，胶水按 glue_set 拉伸/收缩，
/// 调整量按累积舍入（tex.web `hlist_out` 的 `cur_g`）烘焙进胶水宽度。
/// height/depth 保持自然值（hbox_dimensions）。
pub fn hpack(children: &[Node], width: i64) -> BoxNode {
    let natural = hbox_dimensions(children);
    let mut total_stretch = [0i64; 4];
    let mut total_shrink = [0i64; 4];
    // 计入**全部** glue：`name` 只是 showbox 转录标签（`\glue(\leftskip)`），
    // 不参与 glue set 语义。此前按 `name: None` 过滤，行首/行尾 `\leftskip`、
    // `\rightskip`（带名）被排除在拉伸之外——`\centering` 的
    // `\leftskip=\@flushglue`（0pt plus 1fil）永不拉伸，居中/flushleft 全失效。
    // 引导符节点在 tex.web 里就是 glue_node（subtype=leader 旗标 + leader_ptr），
    // 拉伸/收缩与普通胶水同权：`\@dottedtocline` 行的全部可伸量都在
    // `\leaders\hbox{…}\hfill` 上，漏计则页码推不到右边距。
    for c in children {
        match c {
            Node::Glue {
                stretch,
                shrink,
                stretch_order,
                shrink_order,
                ..
            }
            | Node::Leaders {
                stretch,
                shrink,
                stretch_order,
                shrink_order,
                ..
            } => {
                total_stretch[*stretch_order as usize] += stretch;
                total_shrink[*shrink_order as usize] += shrink;
            }
            _ => {}
        }
    }
    let excess = width - natural.width;
    #[derive(Clone, Copy)]
    enum Sign {
        Normal,
        Stretch,
        Shrink,
    }
    let (sign, order, gs) = if excess == 0 {
        (Sign::Normal, 0, 0.0)
    } else if excess > 0 {
        match (0..4).rev().find(|&o| total_stretch[o] != 0) {
            Some(o) => (Sign::Stretch, o, excess as f64 / total_stretch[o] as f64),
            None => (Sign::Normal, 0, 0.0),
        }
    } else {
        match (0..4).rev().find(|&o| total_shrink[o] != 0) {
            Some(o) => {
                let gs = (-excess) as f64 / total_shrink[o] as f64;
                let gs = if o == 0 && total_shrink[o] < -excess {
                    1.0
                } else {
                    gs
                };
                (Sign::Shrink, o, gs)
            }
            None => (Sign::Normal, 0, 0.0),
        }
    };
    let mut out: Vec<Node> = Vec::with_capacity(children.len());
    let mut cum = 0f64;
    let mut prev_g = 0f64;
    for c in children {
        match c {
            Node::Glue {
                name,
                width: w,
                stretch,
                shrink,
                stretch_order,
                shrink_order,
            } => {
                let mut w = *w;
                match sign {
                    Sign::Stretch if *stretch_order as usize == order => {
                        cum += *stretch as f64;
                        let g = (gs * cum).round();
                        w += g as i64 - prev_g as i64;
                        prev_g = g;
                    }
                    Sign::Shrink if *shrink_order as usize == order => {
                        cum -= *shrink as f64;
                        let g = (gs * cum).round();
                        w += g as i64 - prev_g as i64;
                        prev_g = g;
                    }
                    _ => {}
                }
                out.push(Node::Glue {
                    name: *name,
                    width: w,
                    stretch: *stretch,
                    shrink: *shrink,
                    stretch_order: *stretch_order,
                    shrink_order: *shrink_order,
                });
            }
            // 引导符节点宽度同胶水结算（tex.web hlist_out 的 leader glue 取同一
            // glue_set）——结算后 shipout 物化时才知道点线要铺多宽。
            Node::Leaders {
                kind,
                inner,
                width: w,
                stretch,
                shrink,
                stretch_order,
                shrink_order,
            } => {
                let mut w = *w;
                match sign {
                    Sign::Stretch if *stretch_order as usize == order => {
                        cum += *stretch as f64;
                        let g = (gs * cum).round();
                        w += g as i64 - prev_g as i64;
                        prev_g = g;
                    }
                    Sign::Shrink if *shrink_order as usize == order => {
                        cum -= *shrink as f64;
                        let g = (gs * cum).round();
                        w += g as i64 - prev_g as i64;
                        prev_g = g;
                    }
                    _ => {}
                }
                out.push(Node::Leaders {
                    kind: *kind,
                    inner: inner.clone(),
                    width: w,
                    stretch: *stretch,
                    shrink: *shrink,
                    stretch_order: *stretch_order,
                    shrink_order: *shrink_order,
                });
            }
            other => out.push(other.clone()),
        }
    }
    BoxNode {
        kind: BoxKind::HBox,
        width,
        height: natural.height,
        depth: natural.depth,
        shift: 0,
        children: out,
    }
}

/// shipout 物化引导符（tex.web §623-629 `hlist_out`/`vlist_out` leader 分支）：
/// 把 [`Node::Leaders`] 替换为引导单元的逐份拷贝（配 `\kern` 站位），递归子树。
/// DVI 写出器把 Leaders 当 no-op 跳过，故引导内容必须在 ship 边界落成实体节点。
/// 引导节点 width 已由 `hpack` 结算（NTex 盒子不存 glue_set），直接当区域宽；
/// 不做 tex.web 的 `rule_wd+10`/`edge-10` 补偿（定点算术的浮点容差手段）。
pub fn materialize_leaders(b: &mut BoxNode) {
    let horiz = b.kind == BoxKind::HBox;
    let mut out: Vec<Node> = Vec::with_capacity(b.children.len());
    let mut cur = 0i64; // 盒内相对光标（tex.web cur_h−left_edge / cur_v−top_edge）
    for c in b.children.drain(..) {
        match c {
            Node::Leaders {
                kind, inner, width, ..
            } => {
                let unit = match *inner {
                    Node::Box(mut ib) => {
                        materialize_leaders(&mut ib); // 引导单元内部可再含引导符
                        Node::Box(ib)
                    }
                    other => other,
                };
                let d = unit.dimensions();
                // 水平引导量单元 width；垂直引导量单元 h+d（tex.web L12637）。
                let unit_ext = if horiz { d.width } else { d.total() };
                if unit_ext <= 0 || width <= 0 {
                    continue; // tex.web：装不下一个完整单元 → 整段不输出
                }
                let edge = cur + width;
                let (mut nxt, lx) = match kind {
                    LeadersKind::Leaders => {
                        // 对齐网格：left_edge(=0) 起 unit_ext 的最小整数倍 ≥ cur
                        let first = (cur / unit_ext) * unit_ext;
                        (if first < cur { first + unit_ext } else { first }, 0)
                    }
                    LeadersKind::Cleaders => {
                        let lr = width % unit_ext;
                        (cur + lr / 2, 0)
                    }
                    LeadersKind::Xleaders => {
                        let lq = width / unit_ext;
                        let lr = width % unit_ext;
                        let lx = lr / (lq + 1);
                        (cur + (lr - (lq - 1) * lx) / 2, lx)
                    }
                };
                while nxt + unit_ext <= edge {
                    let gap = nxt - cur;
                    if gap != 0 {
                        out.push(Node::Kern { width: gap });
                    }
                    out.push(unit.clone());
                    cur = nxt + unit_ext;
                    nxt += unit_ext + lx;
                }
                // 尾部字距补齐区域余量：替换的是宽为 width 的节点，后续内容
                // （同一列表里引导符之后的节点）必须从 edge 起排。
                let rest = edge - cur;
                if rest != 0 {
                    out.push(Node::Kern { width: rest });
                }
                cur = edge;
            }
            Node::Box(mut ib) => {
                materialize_leaders(&mut ib);
                cur += if horiz {
                    ib.width
                } else {
                    ib.height + ib.depth
                };
                out.push(Node::Box(ib));
            }
            other => {
                cur += other.dimensions().width;
                out.push(other);
            }
        }
    }
    b.children = out;
}

#[cfg(test)]
mod tests {
    use super::*;
    use ntex_core::register::SP_PER_PT;

    fn char_of(w: i64, h: i64, d: i64) -> Node {
        Node::Char {
            font: FontId(0),
            charcode: 65,
            width: w,
            height: h,
            depth: d,
        }
    }

    fn glue(w: i64) -> Node {
        Node::Glue {
            name: None,
            width: w,
            stretch: 0,
            shrink: 0,
            stretch_order: 0,
            shrink_order: 0,
        }
    }

    fn kern(w: i64) -> Node {
        Node::Kern { width: w }
    }

    /// 单位一致性：1pt = 65536sp，与 `ntex_core::register` 常量一致。
    #[test]
    fn sp_units_match_register() {
        let d = hbox_dimensions(&[char_of(SP_PER_PT, 0, 0)]);
        assert_eq!(d.width, SP_PER_PT);
        assert_eq!(d.width, 65_536);
    }

    #[test]
    fn hbox_width_is_sum() {
        let d = hbox_dimensions(&[
            char_of(100, 10, 2),
            glue(30),
            kern(7),
            Node::Penalty { penalty: -50 },
            char_of(3, 5, 1),
        ]);
        assert_eq!(d.width, 140);
    }

    #[test]
    fn hbox_height_depth_are_max() {
        let d = hbox_dimensions(&[
            char_of(10, 12, 3),
            char_of(10, 7, 9),
            Node::Rule {
                width: 4,
                height: 20,
                depth: 1,
            },
            glue(5),
        ]);
        assert_eq!(d.height, 20);
        assert_eq!(d.depth, 9);
        // 胶水不参与 height/depth。
        let only_glue = hbox_dimensions(&[glue(50), kern(50), Node::Penalty { penalty: 0 }]);
        assert_eq!(only_glue.height, 0);
        assert_eq!(only_glue.depth, 0);
    }

    #[test]
    fn hbox_empty_is_zero() {
        assert_eq!(hbox_dimensions(&[]), BoxDimensions::ZERO);
    }

    #[test]
    fn vbox_width_is_max() {
        let d = vbox_dimensions(&[char_of(10, 5, 0), char_of(40, 5, 0), glue(25)]);
        assert_eq!(d.width, 40);
    }

    #[test]
    fn vbox_total_is_sum_height_first_depth_remainder() {
        // 首件 h=10 d=2，次件 h=3 d=1：x = 0+10 + (2+3) = 15，d = 1。
        // （tex.web 结转算法：height=自然高 x=15，depth=末件挂起 d=1；total=16 不变。）
        let d = vbox_dimensions(&[char_of(5, 10, 2), char_of(5, 3, 1)]);
        assert_eq!(d.height, 15);
        assert_eq!(d.depth, 1);
        assert_eq!(d.total(), 16);
    }

    #[test]
    fn vbox_glue_kern_advance_natural_height() {
        // tex.web L13196/L13209：glue/kern 结转挂起深度并按 width 推进自然高。
        // glue(10) + kern(5) → height=15，depth=0（无盒则无挂起深度）。
        // 回归背景：旧实现胶水不占高，\vtop 内多行中文的行间 glue 归零、
        // 行盒贴死（resume-plain.tex 现场报告）。
        let d = vbox_dimensions(&[glue(10), kern(5), Node::Penalty { penalty: 0 }]);
        assert_eq!(d.width, 10);
        assert_eq!(d.height, 15);
        assert_eq!(d.depth, 0);
    }

    #[test]
    fn vbox_glue_between_boxes_flushes_pending_depth() {
        // 盒(h=8,d=2) + glue(5) + 盒(h=3,d=1)：
        // x = 8 + (2+5) + 3 = 18，d = 1（末盒深度保持挂起）→ (18+1)。
        let b1 = BoxNode::new_hbox(vec![char_of(10, 8, 2)]);
        let b2 = BoxNode::new_hbox(vec![char_of(10, 3, 1)]);
        let d = vbox_dimensions(&[Node::Box(b1), glue(5), Node::Box(b2)]);
        assert_eq!(d.height, 18);
        assert_eq!(d.depth, 1);
        assert_eq!(d.total(), 19);
    }

    #[test]
    fn vbox_empty_is_zero() {
        assert_eq!(vbox_dimensions(&[]), BoxDimensions::ZERO);
    }

    #[test]
    fn box_node_constructors_fix_dimensions() {
        let hb = BoxNode::new_hbox(vec![char_of(10, 4, 1), char_of(20, 6, 3)]);
        assert_eq!(hb.width, 30);
        assert_eq!(hb.height, 6);
        assert_eq!(hb.depth, 3);
        assert_eq!(hb.shift, 0);
        assert_eq!(hb.kind, BoxKind::HBox);

        let vb = BoxNode::new_vbox(vec![char_of(8, 9, 1), char_of(8, 2, 2)]);
        assert_eq!(vb.width, 8);
        // 结转算法：x = 9 + (1+2) = 12，d = 2。
        assert_eq!(vb.height, 12);
        assert_eq!(vb.depth, 2);
        assert_eq!(vb.kind, BoxKind::VBox);
    }

    #[test]
    fn leaders_dimensions_come_from_inner_box() {
        let inner = BoxNode::new_hbox(vec![char_of(12, 3, 4)]);
        let ld = Node::Leaders {
            kind: LeadersKind::Leaders,
            inner: Box::new(Node::Box(inner.clone())),
            width: 120,
            stretch: 0,
            shrink: 0,
            stretch_order: 0,
            shrink_order: 0,
        };
        let d = ld.dimensions();
        assert_eq!(d.width, 120);
        assert_eq!(d.height, 3);
        assert_eq!(d.depth, 4);
        assert!(ld.has_vertical_extent());
    }

    #[test]
    fn discardable_classification() {
        assert!(Node::Glue {
            name: None,
            width: 0,
            stretch: 0,
            shrink: 0,
            stretch_order: 0,
            shrink_order: 0,
        }
        .is_discardable());
        assert!(Node::Kern { width: 0 }.is_discardable());
        assert!(Node::Penalty { penalty: 0 }.is_discardable());
        assert!(!char_of(1, 1, 1).is_discardable());
        assert!(!Node::Box(BoxNode::new_hbox(vec![])).is_discardable());
    }
}
