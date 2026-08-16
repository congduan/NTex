//! 断行基础（M3-2-3）：badness 与断行点扫描；Knuth-Plass DP 在 M3-3。
//!
//! - [`badness`]：胶水拉伸/收缩过量度的坏度（TeXbook p.97：`100·(t/s)³` 截断）。
//! - [`collect_breakpoints`]：把水平列表扫描成断行候选点（在胶水/惩罚处，
//!   tex.web 的 break_node 预处理）。
//!
//! 断点语义：行从断点 a 到断点 b 的自然宽度 = `width(b) − width(a)`
//! （断点自身胶水/惩罚不占宽度，折行时被丢弃）。

use crate::node::Node;

/// TeX badness：胶水需要调整 `t`（sp）而可用拉伸/收缩为 `s`（sp）时的坏度。
///
/// 0..=10000；`t<=0` 为 0（无调整需求），`s<=0` 且 `t>0` 为 10000（无胶水可用）。
/// 近似公式 `100·(t/s)³`（TeXbook p.97），经 r=round(100t/s) 整数实现。
pub fn badness(t: i64, s: i64) -> u16 {
    if t <= 0 {
        return 0;
    }
    if s <= 0 {
        return 10_000;
    }
    // r = round(100·t/s)，截断防溢出
    let r = ((100_i128 * t as i128 + s as i128 / 2) / s as i128).min(10_000);
    let b = (r * r * r) / 10_000;
    b.min(10_000) as u16
}

/// 断行候选点（tex.web break_node 的 Rust 表达）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakPoint {
    /// 断点节点在水平列表中的下标（glue 或 penalty）。
    pub index: usize,
    /// 断点前（不含断点节点）的累计宽度。
    pub width: i64,
    /// 累计可拉伸量（不含断点节点）。
    pub stretch: i64,
    /// 累计可收缩量（不含断点节点）。
    pub shrink: i64,
    /// 断点惩罚：glue 断点为 0；penalty 断点为其值。
    pub penalty: i64,
}

/// 扫描水平列表的断行候选点：在胶水（penalty 0）与惩罚（自身值）处产生。
///
/// 累计量按节点顺序叠加：字符/盒子/规则/字距贡献 width，胶水贡献
/// width/stretch/shrink（其自身作为候选点时先记录、后叠加）。
pub fn collect_breakpoints(hlist: &[Node]) -> Vec<BreakPoint> {
    let mut out = Vec::new();
    let mut width = 0i64;
    let mut stretch = 0i64;
    let mut shrink = 0i64;
    for (i, node) in hlist.iter().enumerate() {
        match node {
            Node::Glue {
                width: w,
                stretch: st,
                shrink: sh,
            } => {
                out.push(BreakPoint {
                    index: i,
                    width,
                    stretch,
                    shrink,
                    penalty: 0,
                });
                width += w;
                stretch += st;
                shrink += sh;
            }
            Node::Penalty { penalty } => {
                out.push(BreakPoint {
                    index: i,
                    width,
                    stretch,
                    shrink,
                    penalty: *penalty,
                });
            }
            other => {
                width += other.dimensions().width;
            }
        }
    }
    out
}

/// 临时贪心折行（M3-3 Knuth-Plass 前的过渡实现）：按断点贪心切行。
///
/// 返回 `(start, end)` 行区间（`end` 为断点下标，**不含**；行内容 = `[start, end)`，
/// 断点胶水/惩罚被丢弃）。行自然宽度 = `width(end 断点) − width(start 断点)`。
///
/// 局限：不考虑 badness/惩罚/过满调整，仅保证宽度不超 `max_width`；
/// M3-3 用 Knuth-Plass DP 替换。
pub fn simple_lines(hlist: &[Node], max_width: i64) -> Vec<(usize, usize)> {
    if hlist.is_empty() {
        return Vec::new();
    }
    let breaks = collect_breakpoints(hlist);
    if breaks.is_empty() {
        return vec![(0, hlist.len())];
    }
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut start_w = 0i64;
    // 候选切点：最近一个未超宽的断点（0 = 无）
    let mut cut = 0usize;
    let mut cut_w = 0i64;
    for b in &breaks {
        if b.width - start_w > max_width {
            let end = if cut > start { cut } else { b.index };
            if end > start {
                lines.push((start, end));
                start = end + 1;
                start_w = if end == cut { cut_w } else { b.width };
            }
            cut = 0;
            cut_w = 0;
        } else {
            cut = b.index;
            cut_w = b.width;
        }
    }
    if start < hlist.len() {
        lines.push((start, hlist.len()));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{FontId, Node};

    fn char_of(w: i64) -> Node {
        Node::Char {
            font: FontId(0),
            charcode: 65,
            width: w,
            height: 0,
            depth: 0,
        }
    }

    fn glue(w: i64, st: i64, sh: i64) -> Node {
        Node::Glue {
            width: w,
            stretch: st,
            shrink: sh,
        }
    }

    #[test]
    fn badness_calibration() {
        assert_eq!(badness(0, 1000), 0); // 无需调整
        assert_eq!(badness(1000, 0), 10_000); // 无胶水可用
        assert_eq!(badness(1000, 1000), 100); // 拉伸到满 → 100
        assert_eq!(badness(2000, 1000), 800); // 2x 拉伸 → 800
        assert_eq!(badness(1000, 2000), 12); // 0.5x → 12（100·0.125）
        assert_eq!(badness(1000, -5), 10_000); // 负可用量视为无
        assert_eq!(badness(-100, 1000), 0); // 负 t（收缩侧）按 0
    }

    #[test]
    fn badness_caps_at_10000() {
        assert_eq!(badness(10_000, 1), 10_000);
        assert_eq!(badness(1_000_000, 1), 10_000);
    }

    #[test]
    fn breakpoints_at_glue_and_penalty() {
        let hlist = vec![
            char_of(10),
            glue(3, 2, 1),
            char_of(20),
            Node::Penalty { penalty: -50 },
            char_of(30),
            glue(5, 0, 0),
        ];
        let breaks = collect_breakpoints(&hlist);
        assert_eq!(breaks.len(), 3);
        // 胶水断点：断点前累计，penalty 0
        assert_eq!(
            breaks[0],
            BreakPoint {
                index: 1,
                width: 10,
                stretch: 0,
                shrink: 0,
                penalty: 0,
            }
        );
        // 惩罚断点：累计含此前胶水
        assert_eq!(
            breaks[1],
            BreakPoint {
                index: 3,
                width: 33,
                stretch: 2,
                shrink: 1,
                penalty: -50,
            }
        );
        assert_eq!(
            breaks[2],
            BreakPoint {
                index: 5,
                width: 63,
                stretch: 2,
                shrink: 1,
                penalty: 0,
            }
        );
    }

    #[test]
    fn kern_contributes_width_but_no_break() {
        let hlist = vec![
            char_of(10),
            Node::Kern { width: 7 },
            glue(3, 0, 0),
        ];
        let breaks = collect_breakpoints(&hlist);
        assert_eq!(breaks.len(), 1);
        assert_eq!(breaks[0].width, 17); // 含 kern 宽度
    }

    #[test]
    fn empty_list_no_breaks() {
        assert!(collect_breakpoints(&[]).is_empty());
    }

    #[test]
    fn simple_lines_greedy() {
        // 两词 + 两词：max_width 只够一行放一个词
        let hlist = vec![
            char_of(10),
            char_of(10),
            glue(3, 0, 0),
            char_of(10),
            char_of(10),
            char_of(10),
            glue(3, 0, 0),
            char_of(10),
        ];
        let lines = simple_lines(&hlist, 25);
        assert_eq!(lines, vec![(0, 2), (3, 8)]);
        // 更窄：每个词单独一行
        let lines = simple_lines(&hlist, 5);
        assert_eq!(lines, vec![(0, 2), (3, 6), (7, 8)]);
    }

    #[test]
    fn simple_lines_fits_one_line() {
        let hlist = vec![char_of(10), glue(3, 0, 0), char_of(10)];
        assert_eq!(simple_lines(&hlist, 100), vec![(0, 3)]);
    }

    #[test]
    fn simple_lines_no_breaks_single_word() {
        let hlist = vec![char_of(10), char_of(10)];
        assert_eq!(simple_lines(&hlist, 5), vec![(0, 2)]);
        assert!(simple_lines(&[], 5).is_empty());
    }
}
