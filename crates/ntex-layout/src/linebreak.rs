//! 断行（M3-3）：TeX badness 与 Knuth-Plass 动态规划。
//!
//! - [`badness`]：tex.web §7 的精确实现（`r=297t/s`，`badness=(r³+2¹⁷) div 2¹⁸`，
//!   `r>1290` 视为 10000）。
//! - [`knuth_plass`]：tex.web `line_break` 的 Rust 表达——active 断点集 + DP，
//!   最小总 demerits；强制断点（`\penalty≤-10000`）冲洗 active 并定案路径。
//!
//! 行语义：行从断点 a 到断点 b，自然宽度 = `width(b)−width(a)`（断点自身胶水
//! 被丢弃；其 stretch/shrink 计入行可用量，TeX §845 语义）。

use crate::node::Node;

/// 无限坏度（tex.web `inf_bad`）。
const INF_BAD: u16 = 10_000;

/// TeX badness：胶水需调整 `t≥0`（sp）而可用拉伸/收缩为 `s`（sp）时的坏度。
///
/// tex.web §7 精确算法：`t=0→0`；`s≤0→10000`；否则
/// `r=(297t)/s`（防溢出分支），`r>1290→10000`，`badness=(r³+2¹⁷) div 2¹⁸`。
pub fn badness(t: i64, s: i64) -> u16 {
    if t == 0 {
        return 0;
    }
    if s <= 0 {
        return INF_BAD;
    }
    let r = if t <= 7_230_584 {
        (t * 297) / s
    } else if s >= 1_663_497 {
        t / (s / 297)
    } else {
        t
    };
    if r > 1290 {
        return INF_BAD;
    }
    let cube = r as i128 * r as i128 * r as i128;
    ((cube + 131_072) / 262_144).min(10_000) as u16
}

/// 断点（tex.web `break_node` 预处理的 Rust 表达）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BreakSpec {
    /// 断点节点下标（末尾强制断点为列表长度；虚拟起点为 0）。
    index: usize,
    /// 断点后下一行的内容起点（虚拟起点 0；真实断点 = index+1）。
    content_start: usize,
    /// 累计宽度（不含断点自身 glue；TeX §845）。
    width: i64,
    /// 累计可拉伸（按无穷阶，**含**断点自身 glue）。
    stretch: [i64; 4],
    /// 累计可收缩（按无穷阶，**含**断点自身 glue）。
    shrink: [i64; 4],
    /// 断点惩罚（glue 断点 0；`≤-10000` 强制；`≥10000` 禁断）。
    penalty: i64,
    /// 是否强制断行（penalty ≤ -10000）。
    is_forced: bool,
}

/// 取最高阶上两侧差值非零的 `(阶, 量)`（tex.web：高阶胶水优先）。
fn highest(hi: [i64; 4], lo: [i64; 4]) -> (u8, i64) {
    for o in (0..=3).rev() {
        let d = hi[o] - lo[o];
        if d != 0 {
            return (o as u8, d);
        }
    }
    (0, 0)
}

/// 预处理：水平列表 → 断点序列（TeX §845 语义）。
/// 尾部裁剪/parfillskip 由排版器负责；末尾追加强制断点。
fn preprocess(hlist: &[Node]) -> Vec<BreakSpec> {
    let mut out = vec![BreakSpec {
        index: 0,
        content_start: 0,
        width: 0,
        stretch: [0; 4],
        shrink: [0; 4],
        penalty: 0,
        is_forced: false,
    }];
    let mut width = 0i64;
    let mut stretch = [0i64; 4];
    let mut shrink = [0i64; 4];
    for (i, node) in hlist.iter().enumerate() {
        match node {
            Node::Glue {
                width: w,
                stretch: st,
                shrink: sh,
                stretch_order: so,
                shrink_order: ro,
            } => {
                let mut st_total = stretch;
                st_total[(*so as usize).min(3)] += st;
                let mut sh_total = shrink;
                sh_total[(*ro as usize).min(3)] += sh;
                out.push(BreakSpec {
                    index: i,
                    content_start: i + 1,
                    width,
                    stretch: st_total,
                    shrink: sh_total,
                    penalty: 0,
                    is_forced: false,
                });
                width += w;
                stretch = st_total;
                shrink = sh_total;
            }
            Node::Penalty { penalty } => {
                // penalty ≥ 10000：禁止断（不算断点）
                if *penalty < 10_000 {
                    out.push(BreakSpec {
                        index: i,
                        content_start: i + 1,
                        width,
                        stretch,
                        shrink,
                        penalty: *penalty,
                        is_forced: *penalty <= -10_000,
                    });
                }
            }
            other => {
                width += other.dimensions().width;
            }
        }
    }
    // 末尾强制断点（段落必然可收束）
    out.push(BreakSpec {
        index: hlist.len(),
        content_start: hlist.len(),
        width,
        stretch,
        shrink,
        penalty: -10_000,
        is_forced: true,
    });
    out
}

/// 拟合类（tex.web §16099-16105）：very_loose=0, loose=1, decent=2, tight=3。
type FitClass = u8;
const VERY_LOOSE: FitClass = 0;
const LOOSE: FitClass = 1;
const DECENT: FitClass = 2;
const TIGHT: FitClass = 3;

/// 强制断点惩罚（tex.web `eject_penalty`）。
const EJECT_PENALTY: i64 = -10_000;

/// `\linepenalty`（plain 默认 10）与 `\adjdemerits`（plain 默认 10000）。
const LINE_PENALTY: i64 = 10;
const ADJ_DEMERITS: i64 = 10_000;

/// 行伸缩方向（tex.web §16790-16813）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    Stretch,
    Shrink,
}

/// 行 badness + 伸缩方向（tex.web §16790-16813）：
/// 需拉伸时 fil/fill/filll 视为无限 → badness 0；收缩只用普通阶。
fn line_badness_kind(bi: &BreakSpec, ap: &BreakSpec, hsize: i64) -> (u16, LineKind) {
    let w = bi.width - ap.width;
    if w < hsize {
        let (o, amt) = highest(bi.stretch, ap.stretch);
        if o > 0 {
            (0, LineKind::Stretch)
        } else {
            (badness(hsize - w, amt), LineKind::Stretch)
        }
    } else {
        let (o, amt) = highest(bi.shrink, ap.shrink);
        if o > 0 {
            (0, LineKind::Shrink)
        } else {
            (badness(w - hsize, amt), LineKind::Shrink)
        }
    }
}

/// 拟合类（tex.web §16790-16813）：收缩 `b>12` → tight；拉伸 `b>12` →
/// `b>99` → very_loose 否则 loose；其余 decent（含 fil 无限拉伸 b=0）。
fn fit_class_of(bad: u16, kind: LineKind) -> FitClass {
    if bad > 12 {
        match kind {
            LineKind::Shrink => TIGHT,
            LineKind::Stretch => {
                if bad > 99 {
                    VERY_LOOSE
                } else {
                    LOOSE
                }
            }
        }
    } else {
        DECENT
    }
}

/// 单行 demerits（tex.web §16900-16910 精确算法）：
///
/// `d = (line_penalty + badness)²`（|和| ≥ 10000 时钳为 10⁸）；
/// 断点惩罚 `pi≠0` 时附加 `pi²`（`pi<0` 且非强制时等价于加 `pi²`；
/// 强制断点 pi=eject 不加）；相邻行拟合类差 > 1 时加 `\adjdemerits`。
/// 参数常量取 plain 默认：`\linepenalty=10`、`\adjdemerits=10000`。
fn line_demerits(bad: u16, pi: i64, fit: FitClass, prev_fit: FitClass) -> i64 {
    let mut d = LINE_PENALTY + i64::from(bad);
    d = if d.abs() >= 10_000 {
        100_000_000
    } else {
        d * d
    };
    if pi != 0 {
        if pi > 0 {
            d += pi * pi;
        } else if pi > EJECT_PENALTY {
            d -= pi * pi;
        }
    }
    if (fit as i64 - prev_fit as i64).abs() > 1 {
        d += ADJ_DEMERITS;
    }
    d
}

/// 核心 DP：返回（最优断点路径下标序列，最小总 demerits）。
///
/// active 集保存可作行起点的断点；badness ≤ tolerance 且非强制时行可接受；
/// 强制断点处冲洗 active（此前路径定案）。每个断点按末行拟合类分别保留最优
/// （tex.web `minimal_demerits[fit_class]`，`\adjdemerits` 依赖相邻行拟合类差）。
fn best_path(breaks: &[BreakSpec], hsize: i64, tolerance: i64) -> (Vec<usize>, i64) {
    let n = breaks.len();
    // best[i][fc]：断点 i 结束、末行拟合类 fc 的最小总 demerits。
    let mut best: Vec<[i64; 4]> = vec![[i64::MAX; 4]; n];
    // best_prev[i][fc]：(前一断点下标, 前一行拟合类)。
    let mut best_prev: Vec<[Option<(usize, FitClass)>; 4]> = vec![[None; 4]; n];
    let mut active: Vec<usize> = vec![0];
    best[0][DECENT as usize] = 0; // 虚拟起点拟合类 = decent（tex.web §17032）

    for i in 1..n {
        let bi = breaks[i];
        // 按本行拟合类分槽的候选（tex.web `minimal_demerits`/`best_place`）。
        let mut champion: [Option<(i64, usize, FitClass)>; 4] = [None; 4];
        for &a in &active {
            let ap = breaks[a];
            let (bad, kind) = line_badness_kind(&bi, &ap, hsize);
            // 强制断点即使过满也可接受
            if bad as i64 > tolerance && !bi.is_forced {
                continue;
            }
            let fit = fit_class_of(bad, kind);
            for (af, &ad) in best[a].iter().enumerate() {
                if ad == i64::MAX {
                    continue;
                }
                let d = ad + line_demerits(bad, bi.penalty, fit, af as FitClass);
                let slot = &mut champion[fit as usize];
                if slot.map_or(true, |(bd, _, _)| d < bd) {
                    *slot = Some((d, a, af as FitClass));
                }
            }
        }
        for (fc, c) in champion.iter().enumerate() {
            if let Some((d, a, af)) = *c {
                best[i][fc] = d;
                best_prev[i][fc] = Some((a, af));
            }
        }
        if champion.iter().any(|c| c.is_some()) {
            active.push(i);
            if bi.is_forced {
                // 强制断行：此前的路径已定案，后续只能从本断点起行
                active.clear();
                active.push(i);
            }
        }
    }

    // 末点（末尾强制断点）：取总 demerits 最小的拟合类回溯
    let (total, fc0) = (0..4)
        .map(|fc| (best[n - 1][fc], fc as FitClass))
        .min_by_key(|(d, _)| *d)
        .expect("末尾强制断点必有路径");
    let mut path = vec![n - 1];
    let mut cur = n - 1;
    let mut cur_fit = fc0;
    while let Some((p, pf)) = best_prev[cur][cur_fit as usize] {
        path.push(p);
        cur = p;
        cur_fit = pf;
    }
    path.reverse();
    (path, total)
}

/// Knuth-Plass 断行：返回行区间 `(start, end)`（`end` 不含；行内容 = `[start, end)`）。
///
/// `hsize`：行目标宽度（sp）；`tolerance`：可接受最大 badness（tex.web `tolerance`）。
/// 目前为 O(n²)（未做 active 淘汰）；`\parfillskip`/右端对齐等留待后续。
pub fn knuth_plass(hlist: &[Node], hsize: i64, tolerance: i64) -> Vec<(usize, usize)> {
    if hlist.is_empty() {
        return Vec::new();
    }
    let breaks = preprocess(hlist);
    let (path, _total) = best_path(&breaks, hsize, tolerance);
    let mut lines = Vec::new();
    for w in path.windows(2) {
        let (a, b) = (breaks[w[0]], breaks[w[1]]);
        let (start, end) = (a.content_start, b.index);
        if end > start {
            lines.push((start, end));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{FontId, GLUE_ORDER_FIL, Node};

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
            stretch_order: 0,
            shrink_order: 0,
        }
    }

    fn fil_glue() -> Node {
        Node::Glue {
            width: 0,
            stretch: 1,
            shrink: 0,
            stretch_order: GLUE_ORDER_FIL,
            shrink_order: 0,
        }
    }

    // ---------- badness（tex.web §7 校准值） ----------

    #[test]
    fn badness_calibration() {
        assert_eq!(badness(0, 1000), 0);
        assert_eq!(badness(1000, 0), 10_000);
        assert_eq!(badness(1000, 1000), 100); // 满拉伸
        assert_eq!(badness(2000, 1000), 800); // 2x
        assert_eq!(badness(1000, 2000), 12); // 0.5x → 100·(0.5)³
        assert_eq!(badness(1000, -5), 10_000);
        assert_eq!(badness(10_000, 1), 10_000); // r>1290
        assert_eq!(badness(1_000_000, 1), 10_000);
    }

    // ---------- Knuth-Plass vs 暴力最优 ----------

    /// 独立暴力实现：最小总 demerits（同一接受规则；状态 = (断点, 末行拟合类)）。
    fn brute_min(breaks: &[BreakSpec], hsize: i64, tolerance: i64) -> i64 {
        let n = breaks.len();
        let mut memo = vec![[None; 4]; n];
        fn dfs(
            breaks: &[BreakSpec],
            hsize: i64,
            tolerance: i64,
            i: usize,
            prev_fit: FitClass,
            memo: &mut Vec<[Option<i64>; 4]>,
        ) -> i64 {
            if let Some(v) = memo[i][prev_fit as usize] {
                return v;
            }
            if i == breaks.len() - 1 {
                memo[i][prev_fit as usize] = Some(0);
                return 0; // 末尾强制断点：无需再断
            }
            let bi = breaks[i];
            let mut best = i64::MAX;
            for j in (i + 1)..breaks.len() {
                let bj = breaks[j];
                let (bad, kind) = line_badness_kind(&bj, &bi, hsize);
                if bad as i64 > tolerance && !bj.is_forced {
                    continue;
                }
                let fit = fit_class_of(bad, kind);
                let d = line_demerits(bad, bj.penalty, fit, prev_fit)
                    + dfs(breaks, hsize, tolerance, j, fit, memo);
                best = best.min(d);
            }
            memo[i][prev_fit as usize] = Some(best);
            best
        }
        dfs(breaks, hsize, tolerance, 0, DECENT, &mut memo)
    }

    fn dp_min(hlist: &[Node], hsize: i64, tolerance: i64) -> i64 {
        let breaks = preprocess(hlist);
        let (_, total) = best_path(&breaks, hsize, tolerance);
        total
    }

    /// 词列表：每个词一个字符盒，词间空格 glue（宽 3，拉伸 100，收缩 5）。
    fn words(ws: &[i64]) -> Vec<Node> {
        let mut out = Vec::new();
        for (i, w) in ws.iter().enumerate() {
            if i > 0 {
                out.push(glue(3, 100, 5));
            }
            out.push(char_of(*w));
        }
        out
    }

    #[test]
    fn knuth_plass_matches_brute_force() {
        // 多个词宽组合 × 多个 hsize × 两个 tolerance：DP 总 demerits == 暴力最小
        let cases: [(&[i64], i64); 6] = [
            (&[10, 10, 10, 10], 25),
            (&[10, 10, 10, 10], 20),
            (&[10, 10, 10, 10], 12),
            (&[30, 30], 35),
            (&[5, 5, 5, 5, 5], 8),
            (&[10, 3, 10, 3, 10], 15),
        ];
        for (ws, hsize) in cases {
            for tolerance in [200, 10_000] {
                let hlist = words(ws);
                let dp = dp_min(&hlist, hsize, tolerance);
                let brute = brute_min(&preprocess(&hlist), hsize, tolerance);
                assert_eq!(dp, brute, "hsize={hsize} tol={tolerance} words={ws:?}");
            }
        }
    }

    #[test]
    fn knuth_plass_single_line_when_fits() {
        let hlist = words(&[10, 10, 10]);
        assert_eq!(knuth_plass(&hlist, 100, 200), vec![(0, 5)]);
    }

    #[test]
    fn knuth_plass_breaks_at_spaces() {
        // 三词 a b c：hsize 25 下 "a" | "b c" 为最优（末行含起点胶水拉伸）
        let hlist = words(&[10, 10, 10]);
        let lines = knuth_plass(&hlist, 25, 200);
        assert_eq!(lines, vec![(0, 1), (2, 5)]);
    }

    #[test]
    fn knuth_plass_forced_break_splits() {
        let hlist = vec![
            char_of(10),
            glue(3, 0, 0),
            Node::Penalty { penalty: -10_000 },
            char_of(20),
            glue(3, 0, 0),
            char_of(30),
        ];
        let lines = knuth_plass(&hlist, 10_000, 200);
        // 强制断点（index 2）之前定案：第一行 [0, 2)；之后继续
        assert_eq!(lines, vec![(0, 2), (3, 6)]);
    }

    #[test]
    fn knuth_plass_empty_input() {
        assert!(knuth_plass(&[], 100, 200).is_empty());
    }

    #[test]
    fn knuth_plass_no_breaks_single_word() {
        assert_eq!(knuth_plass(&[char_of(10), char_of(10)], 5, 200), vec![(0, 2)]);
    }

    #[test]
    fn knuth_plass_keeps_trailing_glue() {
        // 预处理不裁剪尾部（裁剪在排版器 close_paragraph）
        let mut hlist = words(&[10, 10]);
        hlist.push(glue(3, 0, 0));
        let lines = knuth_plass(&hlist, 100, 200);
        assert_eq!(lines, vec![(0, 4)]);
    }

    #[test]
    fn fil_glue_makes_line_acceptable() {
        // 两词 + parfillskip（0pt plus 1fil）：
        // 1 行（23 宽，需收缩 1、可用 5 → badness 1 → demerits (10+1)²=121）
        // vs 2 行（各 0 badness → demerits 100+100=200）→ 1 行胜（TeX 精确 demerits）
        let mut hlist = words(&[10, 10]);
        hlist.push(fil_glue());
        let lines = knuth_plass(&hlist, 22, 200);
        assert_eq!(lines, vec![(0, 4)]);
    }

    #[test]
    fn fil_glue_single_line_when_fits() {
        let mut hlist = words(&[10, 10]);
        hlist.push(fil_glue());
        assert_eq!(knuth_plass(&hlist, 100, 200), vec![(0, 4)]);
    }
}
