//! M5 增量（段落级缓存）在本 crate 的实现：折行输入 → 内容指纹。
//!
//! 缓存边界选在**折行输入**（断字 + `\parfillskip` 之后的最终水平列表）与
//! (hsize, tolerance) 处：这组输入完全决定 Knuth-Plass 折行与行盒物化的输出，
//! 因此同指纹 ⇒ 同折行结果，可安全复用缓存的行盒。
//!
//! 指纹覆盖节点全部"影响折行"的字段（字体/字符/维度/胶水/惩罚/方向/mark 文本等），
//! 以固定变体次序喂入 [`Fnv1a`]，保证确定性。断字消化的模式表/异常词表/字体度量
//! 已物化进最终列表（discretionary 节点的 pre/post/replace），故无需额外登记
//! 模式表版本——缓存天然随断字结果变化失效。

use ntex_incremental::{Fingerprint, Fnv1a};

use crate::node::{BoxNode, LeadersKind, Node};
use ntex_core::sink::DirectionKind;

/// 一个段落折行计算的缓存值：产出的行盒 + 末行自然宽度。
#[derive(Debug, Clone)]
pub struct CachedParagraph {
    /// 每行一个 hbox（目标宽 = `\hsize`），已物化好的行盒。
    pub line_boxes: Vec<BoxNode>,
    /// 末行自然宽度（`\parfillskip` 拉伸前的宽度；显示数学 short 判定用）。
    pub last_natural: Option<i64>,
}

/// 段落折行输入的内容指纹：最终水平列表 + hsize + tolerance。
pub fn paragraph_fingerprint(children: &[Node], hsize: i64, tolerance: i64) -> Fingerprint {
    let mut h = Fnv1a::new();
    h.feed_i64(hsize);
    h.feed_i64(tolerance);
    h.feed_u64(children.len() as u64);
    for node in children {
        feed_node(&mut h, node);
    }
    h.finish()
}

/// 递归喂入一个节点：**变体标记 + 字段按固定次序**（次序即指纹语义的一部分）。
fn feed_node(h: &mut Fnv1a, node: &Node) {
    match node {
        Node::Char {
            font,
            charcode,
            width,
            height,
            depth,
        } => {
            h.feed_u64(0);
            h.feed_u64(font.0 as u64);
            h.feed_u64(*charcode as u64);
            h.feed_i64(*width);
            h.feed_i64(*height);
            h.feed_i64(*depth);
        }
        Node::Box(b) => {
            h.feed_u64(1);
            feed_box(h, b);
        }
        Node::Rule {
            width,
            height,
            depth,
        } => {
            h.feed_u64(2);
            h.feed_i64(*width);
            h.feed_i64(*height);
            h.feed_i64(*depth);
        }
        Node::Glue {
            width,
            stretch,
            shrink,
            stretch_order,
            shrink_order,
        } => {
            h.feed_u64(3);
            h.feed_i64(*width);
            h.feed_i64(*stretch);
            h.feed_i64(*shrink);
            h.feed_u64(u64::from(*stretch_order));
            h.feed_u64(u64::from(*shrink_order));
        }
        Node::Kern { width } => {
            h.feed_u64(4);
            h.feed_i64(*width);
        }
        Node::Penalty { penalty } => {
            h.feed_u64(5);
            h.feed_i64(*penalty);
        }
        Node::Leaders {
            kind,
            inner,
            width,
            stretch,
            shrink,
        } => {
            h.feed_u64(6);
            h.feed_u64(leader_kind_tag(*kind));
            feed_box(h, inner);
            h.feed_i64(*width);
            h.feed_i64(*stretch);
            h.feed_i64(*shrink);
        }
        Node::Discretionary { pre, post, replace } => {
            h.feed_u64(7);
            feed_nodes(h, pre);
            feed_nodes(h, post);
            feed_nodes(h, replace);
        }
        Node::Direction { kind } => {
            h.feed_u64(8);
            h.feed_u64(direction_kind_tag(*kind));
        }
        Node::Mark { class, text } => {
            h.feed_u64(9);
            h.feed_u64(class.map_or(u64::MAX, |v| v as u64));
            feed_str(h, text);
        }
        Node::Ins { class, text } => {
            h.feed_u64(10);
            h.feed_u64(*class as u64);
            feed_str(h, text);
        }
        Node::Adjust { text } => {
            h.feed_u64(11);
            feed_str(h, text);
        }
        Node::Whatsit { text } => {
            h.feed_u64(12);
            feed_str(h, text);
        }
    }
}

fn feed_box(h: &mut Fnv1a, b: &BoxNode) {
    h.feed_u64(kind_tag_of(&b.kind));
    h.feed_i64(b.width);
    h.feed_i64(b.height);
    h.feed_i64(b.depth);
    h.feed_i64(b.shift);
    feed_nodes(h, &b.children);
}

fn feed_nodes(h: &mut Fnv1a, nodes: &[Node]) {
    h.feed_u64(nodes.len() as u64);
    for node in nodes {
        feed_node(h, node);
    }
}

fn feed_str(h: &mut Fnv1a, s: &str) {
    h.feed_u64(s.len() as u64);
    h.feed_bytes(s.as_bytes());
}

fn kind_tag_of(kind: &crate::node::BoxKind) -> u64 {
    match kind {
        crate::node::BoxKind::HBox => 0,
        crate::node::BoxKind::VBox => 1,
    }
}

fn leader_kind_tag(kind: LeadersKind) -> u64 {
    match kind {
        LeadersKind::Leaders => 0,
        LeadersKind::Cleaders => 1,
        LeadersKind::Xleaders => 2,
    }
}

fn direction_kind_tag(kind: DirectionKind) -> u64 {
    match kind {
        DirectionKind::BeginL => 0,
        DirectionKind::EndL => 1,
        DirectionKind::BeginR => 2,
        DirectionKind::EndR => 3,
    }
}
