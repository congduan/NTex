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
    pub const ZERO: Self = Self { width: 0, height: 0, depth: 0 };

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
    /// 盒子（hlist / vlist）。
    Box(BoxNode),
    /// 规则（`\hrule` / `\vrule`）。
    Rule { width: i64, height: i64, depth: i64 },
    /// 胶水：可拉伸 / 可收缩。
    Glue {
        width: i64,
        stretch: i64,
        shrink: i64,
        /// 拉伸无穷阶（0=普通，1=fil，2=fill，3=filll）。
        stretch_order: GlueOrder,
        /// 收缩无穷阶。
        shrink_order: GlueOrder,
    },
    /// 字距（不可拉伸）。
    Kern { width: i64 },
    /// 断行惩罚；`penalty < 0` 表示可选断行点，`penalty >= 10000` 禁止断行。
    Penalty { penalty: i64 },
    /// 引导符：行为类似胶水（width/stretch/shrink），内部重复 box 提供 height/depth。
    Leaders {
        kind: LeadersKind,
        /// 被重复的 box。
        inner: BoxNode,
        /// 胶水规格。
        width: i64,
        stretch: i64,
        shrink: i64,
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
}

impl Node {
    /// 该节点的 width/height/depth。
    pub fn dimensions(&self) -> BoxDimensions {
        match self {
            Node::Char { width, height, depth, .. } => BoxDimensions {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Node::Box(b) => b.dimensions(),
            Node::Rule { width, height, depth } => BoxDimensions {
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
            Node::Leaders { width, inner, .. } => BoxDimensions {
                width: *width,
                height: inner.height,
                depth: inner.depth,
            },
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
        }
    }

    /// 是否具有纵向维度（参与盒子 height/depth 计算）。
    pub fn has_vertical_extent(&self) -> bool {
        matches!(
            self,
            Node::Char { .. } | Node::Box(_) | Node::Rule { .. } | Node::Leaders { .. }
        )
    }

    /// 是否可丢弃节点（折行时 glue/kern/penalty 在断行点可被丢弃）。
    pub fn is_discardable(&self) -> bool {
        matches!(self, Node::Glue { .. } | Node::Kern { .. } | Node::Penalty { .. })
    }
}

/// hbox 维度计算（TeXbook p.81 / tex.web `hpackage`）：
/// `width` = Σ 子节点 width；`height`/`depth` = 有纵向维度的子节点各自的最大值。
pub fn hbox_dimensions(children: &[Node]) -> BoxDimensions {
    let mut dims = BoxDimensions::ZERO;
    for c in children {
        let d = c.dimensions();
        dims.width += d.width;
        if c.has_vertical_extent() {
            dims.height = dims.height.max(d.height);
            dims.depth = dims.depth.max(d.depth);
        }
    }
    dims
}

/// vbox 维度计算（tex.web `vpackage`）：
/// `width` = max 子节点 width；`total` = Σ 有纵向维度的子节点（height+depth）；
/// `height` = 第一个有纵向维度的子节点的 height（无则 0）；`depth` = total − height。
pub fn vbox_dimensions(children: &[Node]) -> BoxDimensions {
    let mut width = 0;
    let mut total = 0;
    let mut first_height = 0;
    let mut found = false;
    for c in children {
        let d = c.dimensions();
        width = width.max(d.width);
        if c.has_vertical_extent() {
            total += d.total();
            if !found {
                first_height = d.height;
                found = true;
            }
        }
    }
    BoxDimensions {
        width,
        height: first_height,
        depth: total - first_height,
    }
}

/// `vpack`（tex.web §661 "vpackage"）：把垂直列表打包为总高（height+depth）**恰好**
/// `height` 的 vbox。
///
/// 占位度量阶段简化：差额直接调整维度（高度优先，超 `maxdepth` 语义未建模；
/// 不逐节点烘焙 glue_set）。无差额时与 [`BoxNode::new_vbox`] 等价。
pub fn vpack(children: Vec<Node>, height: i64) -> BoxNode {
    let natural = vbox_dimensions(&children);
    let mut b = BoxNode::new_vbox(children);
    let diff = height - (natural.height + natural.depth);
    if diff >= 0 {
        b.height += diff; // 拉伸：全部加在高度上
    } else {
        // 收缩：先缩高度（≥0），剩余缩深度
        let dh = b.height.min(-diff);
        b.height -= dh;
        let dd = (-diff - dh).min(b.depth);
        b.depth -= dd;
    }
    b
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
            Node::Rule { height: h, depth, .. } => h + depth,
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
    (vpack(top, height), BoxNode::new_vbox(rest))
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
            Node::Rule { width: 4, height: 20, depth: 1 },
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
        // 首盒 h=10 d=2，次盒 h=3 d=1：total=16，height=10，depth=6。
        let d = vbox_dimensions(&[char_of(5, 10, 2), char_of(5, 3, 1)]);
        assert_eq!(d.height, 10);
        assert_eq!(d.depth, 6);
        assert_eq!(d.total(), 16);
    }

    #[test]
    fn vbox_glue_only_has_width_but_no_height_depth() {
        // vpackage 对 width 取所有子节点（含胶水）最大值；胶水不贡献 height/depth。
        let d = vbox_dimensions(&[glue(10), kern(5), Node::Penalty { penalty: 0 }]);
        assert_eq!(d.width, 10);
        assert_eq!(d.height, 0);
        assert_eq!(d.depth, 0);
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
        assert_eq!(vb.height, 9);
        assert_eq!(vb.depth, 5);
        assert_eq!(vb.kind, BoxKind::VBox);
    }

    #[test]
    fn leaders_dimensions_come_from_inner_box() {
        let inner = BoxNode::new_hbox(vec![char_of(12, 3, 4)]);
        let ld = Node::Leaders {
            kind: LeadersKind::Leaders,
            inner: inner.clone(),
            width: 120,
            stretch: 0,
            shrink: 0,
        };
        let d = ld.dimensions();
        assert_eq!(d.width, 120);
        assert_eq!(d.height, 3);
        assert_eq!(d.depth, 4);
        assert!(ld.has_vertical_extent());
    }

    #[test]
    fn discardable_classification() {
        assert!(
            Node::Glue {
                width: 0,
                stretch: 0,
                shrink: 0,
                stretch_order: 0,
                shrink_order: 0,
            }
            .is_discardable()
        );
        assert!(Node::Kern { width: 0 }.is_discardable());
        assert!(Node::Penalty { penalty: 0 }.is_discardable());
        assert!(!char_of(1, 1, 1).is_discardable());
        assert!(!Node::Box(BoxNode::new_hbox(vec![])).is_discardable());
    }
}
