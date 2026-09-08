//! 断行（M3-3）：TeX badness 与 Knuth-Plass 动态规划。
//!
//! - [`badness`]：tex.web §7 的精确实现（`r=297t/s`，`badness=(r³+2¹⁷) div 2¹⁸`，
//!   `r>1290` 视为 10000）。
//! - [`knuth_plass`]：tex.web `line_break` 的 Rust 表达——active 断点集 + DP，
//!   最小总 demerits；强制断点（`\penalty≤-10000`）冲洗 active 并定案路径。
//!
//! 行语义：行从断点 a 到断点 b，自然宽度 = `width(b)−width(a)`（断点自身胶水
//! 被丢弃；其 stretch/shrink 计入行可用量，TeX §845 语义）。

use crate::node::{hbox_dimensions, Node};

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
    /// 断点类型（`\tracingparagraphs` 显示名；tex.web print_esc 语义）。
    kind: BreakKind,
}

/// 断点类型（tex.web 断点描述：glue 断点不打印类型名）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BreakKind {
    /// 虚拟起点（@@0）。
    Start,
    /// 胶水断点（类型名空——`@ via @@n`）。
    Glue,
    /// 惩罚断点（`@\penalty via`）。
    Penalty,
    /// 断字节点（`@\discretionary via`，行号带 `-` 后缀）。
    Disc,
    /// 段落末尾强制断点（`@\par via`）。
    Par,
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
        kind: BreakKind::Start,
    }];
    let mut width = 0i64;
    let mut stretch = [0i64; 4];
    let mut shrink = [0i64; 4];
    for (i, node) in hlist.iter().enumerate() {
        match node {
            Node::Glue {
                name: None,
                width: w,
                stretch: st,
                shrink: sh,
                stretch_order: so,
                shrink_order: ro,
            } => {
                // 断点胶水不入行：stretch/shrink 与 width 一样**不含**本胶水
                // （tex.web line_break：try_break 在胶水加入累计宽度之前调用）。
                out.push(BreakSpec {
                    index: i,
                    content_start: i + 1,
                    width,
                    stretch,
                    shrink,
                    penalty: 0,
                    is_forced: false,
                    kind: BreakKind::Glue,
                });
                width += w;
                stretch[(*so as usize).min(3)] += st;
                shrink[(*ro as usize).min(3)] += sh;
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
                        kind: BreakKind::Penalty,
                    });
                }
            }
            // 断字节点（M4-6）：断点惩罚 = \hyphenpenalty（plain 默认 50）。
            // 断在该点 → 行宽 = 累计 + pre（连字符）宽；未断 → 自身贡献 0
            // （字母留在主列表，replace 为空）。
            Node::Discretionary { pre, .. } => {
                out.push(BreakSpec {
                    index: i,
                    content_start: i + 1,
                    width: width + hbox_dimensions(pre).width,
                    stretch,
                    shrink,
                    penalty: HYPHEN_PENALTY,
                    is_forced: false,
                    kind: BreakKind::Disc,
                });
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
        kind: BreakKind::Par,
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

/// `\hyphenpenalty`（plain 默认 50）：discretionary 断点的惩罚（M4-6）。
const HYPHEN_PENALTY: i64 = 50;

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
            let over = w - hsize;
            // tex.web：收缩不足 → b = inf_bad+1（>inf_bad 才触发 active 淘汰；
            // 拉伸不足的 b 上限是 inf_bad，active 保留不产生候选）。
            if over > amt {
                (INF_BAD + 1, LineKind::Shrink)
            } else {
                (badness(over, amt), LineKind::Shrink)
            }
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

/// 折行遍（tex.web first_pass/second_pass）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// 第一遍：\pretolerance（失败 = active 集空，无兜底）。
    First,
    /// 第二遍：\tolerance（active 空兜底恢复最近起点，保证强制末点可达）。
    Second,
}

/// 核心 DP：返回（最优断点路径下标序列，最小总 demerits）。
///
/// active 集保存可作行起点的断点；badness ≤ 容差且非强制时行可接受；
/// 强制断点处冲洗 active（此前路径定案）。每个断点按末行拟合类分别保留最优
/// （tex.web `minimal_demerits[fit_class]`，`\adjdemerits` 依赖相邻行拟合类差）。
///
/// 第一遍（\pretolerance）active 集变空 = 失败（返回 None，内容断不开）→
/// 调用方走第二遍（\tolerance）。`tracing` 时累积 `\tracingparagraphs` 输出
/// （`@firstpass`/`@secondpass`、候选断点 `@<类型> via @@n b=.. p=.. d=..`、
/// 活动节点 `@@n: line .. t=.. -> @@m`）；失败时已累积的 trace 一并返回。
fn best_path(
    breaks: &[BreakSpec],
    hsize: i64,
    threshold: i64,
    pass: Pass,
    tracing: bool,
) -> Option<(Vec<usize>, i64, String)> {
    let n = breaks.len();
    // best[i][fc]：断点 i 结束、末行拟合类 fc 的最小总 demerits。
    let mut best: Vec<[i64; 4]> = vec![[i64::MAX; 4]; n];
    // best_prev[i][fc]：(前一断点下标, 前一行拟合类)。
    let mut best_prev: Vec<[Option<(usize, FitClass)>; 4]> = vec![[None; 4]; n];
    // best_lines[i][fc]：最优路径到断点 i（fc 结束）的行数（活动节点行号显示）。
    let mut best_lines: Vec<[i64; 4]> = vec![[0; 4]; n];
    let mut active: Vec<usize> = vec![0];
    best[0][DECENT as usize] = 0; // 虚拟起点拟合类 = decent（tex.web §17032）
    let mut trace = String::new();
    if tracing {
        trace.push_str(match pass {
            Pass::First => "@firstpass\n",
            Pass::Second => "@secondpass\n",
        });
    }

    for i in 1..n {
        let bi = breaks[i];
        // tex.web try_break（§859-899）逐 active 的语义：
        // - b > inf_bad（overfull，badness 钳 10000）或强制断点 → 淘汰臂：
        //   final pass 且本断点尚无候选且 active 仅剩这一个 → artificial
        //   demerits（d=0）在 i 记录可行断行（overfull 行兜底，tex.web
        //   @<Prepare to deactivate...@>）；否则淘汰（不入 survivors）。
        // - b ≤ inf_bad：active 一律保留；b ≤ threshold 记录候选，超阈值仅
        //   不产生候选（tex.web `goto continue`）——淘汰与记录门槛解耦。
        let is_final = pass == Pass::Second;
        let mut champion: [Option<(i64, usize, FitClass)>; 4] = [None; 4];
        let mut survivors: Vec<usize> = Vec::new();
        for &a in &active {
            let (bad, kind) = line_badness_kind(&bi, &breaks[a], hsize);
            let forced_drop = bi.is_forced && bad as i64 > threshold;
            if bad > INF_BAD || forced_drop {
                if is_final && champion.iter().all(|c| c.is_none()) && active.len() == 1 {
                    // artificial demerits：d=0（行 demerits 不计，路径链仍建立）
                    let fit = fit_class_of(bad, kind);
                    let (af, &ad) = best[a]
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, &v)| v)
                        .expect("起点必有余量");
                    champion[fit as usize] = Some((ad, a, af as FitClass));
                }
                // 淘汰该起点（不入 survivors）
            } else {
                survivors.push(a);
                if bad as i64 > threshold {
                    continue;
                }
                let fit = fit_class_of(bad, kind);
                for (af, &ad) in best[a].iter().enumerate() {
                    if ad == i64::MAX {
                        continue;
                    }
                    let d = ad + line_demerits(bad, bi.penalty, fit, af as FitClass);
                    // \tracingparagraphs：每个可行断点输出一行（tex.web
                    // `@<类型> via @@<prev> b=.. p=.. d=..`；glue 断点类型名空）。
                    if tracing {
                        let name = match bi.kind {
                            BreakKind::Start | BreakKind::Glue => String::new(),
                            BreakKind::Penalty => "\\penalty".to_string(),
                            BreakKind::Disc => "\\discretionary".to_string(),
                            BreakKind::Par => "\\par".to_string(),
                        };
                        let b_str = if bad == 10_000 { "*" } else { &bad.to_string() };
                        trace.push_str(&format!(
                            "@{name} via @@{a} b={b_str} p={} d={}\n",
                            bi.penalty,
                            line_demerits(bad, bi.penalty, fit, af as FitClass)
                        ));
                    }
                    let slot = &mut champion[fit as usize];
                    if slot.map_or(true, |(bd, _, _)| d < bd) {
                        *slot = Some((d, a, af as FitClass));
                    }
                }
            }
        }
        active = survivors;
        if active.is_empty() {
            match pass {
                // 第一遍：active 淘汰殆尽（overfull 行）→ 失败重跑第二遍
                //（tex.web 主循环 `link(active)=last_active` 停扫 + 非 done 重来）
                Pass::First => return None,
                // 第二遍不会到这（单 active 的 overfull 走 artificial 已记录候选）
                Pass::Second => {}
            }
        }
        if champion.iter().any(|c| c.is_some()) {
            // \tracingparagraphs：新活动节点（tex.web `@@n: line x.y[-] t=.. -> @@m`）
            if tracing {
                let (fc, c) = champion
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.is_some())
                    .min_by_key(|(_, c)| c.unwrap().0)
                    .map(|(fc, c)| (fc, c.unwrap()))
                    .unwrap();
                let (d, a, _af) = c;
                let hyphen = if bi.kind == BreakKind::Disc { "-" } else { "" };
                trace.push_str(&format!(
                    "@@{i}: line {}.{fc}{hyphen} t={d} -> @@{a}\n",
                    best_lines[i][fc]
                ));
            }
            for (fc, c) in champion.iter().enumerate() {
                if let Some((d, a, af)) = *c {
                    best[i][fc] = d;
                    best_prev[i][fc] = Some((a, af));
                    best_lines[i][fc] = best_lines[a][af as usize] + 1;
                }
            }
            active.push(i);
        }
        if bi.is_forced {
            // 强制断行：此前的路径已定案，后续只能从本断点起行
            active.clear();
            active.push(i);
        }
    }
    // 最优路径回溯（tex.web：last_active 链 + 每断点最优拟合类）
    let mut path = vec![n - 1];
    let mut cur = n - 1;
    let mut fc = (0..4)
        .min_by_key(|&fc| best[cur][fc])
        .expect("末尾强制断点必有路径") as FitClass;
    while let Some((prev, prev_fc)) = best_prev[cur][fc as usize] {
        path.push(prev);
        cur = prev;
        fc = prev_fc;
    }
    path.reverse();
    Some((path, best[n - 1].iter().min().copied().unwrap_or(0), trace))
}

/// Knuth-Plass 断行：返回行区间 `(start, end)`（`end` 不含；行内容 = `[start, end)`）。
///
/// 两遍折行（tex.web line_break）：先以 `pretolerance` 尝试第一遍（失败 =
/// active 集空、内容断不开）；失败且 `pretolerance >= 0` 时第二遍用
/// `tolerance`（tex.web `\pretolerance=-1` 跳过第一遍直接第二遍）。
/// `tracing` 时返回 `\tracingparagraphs` 追踪文本（第二返回值）。
pub fn knuth_plass(
    hlist: &[Node],
    hsize: i64,
    tolerance: i64,
    pretolerance: i64,
    tracing: bool,
) -> (Vec<(usize, usize)>, String) {
    if hlist.is_empty() {
        return (Vec::new(), String::new());
    }
    let breaks = preprocess(hlist);
    let mut trace = String::new();
    // 第一遍：\pretolerance（>=0 时）。成功即用；失败走第二遍 \tolerance。
    if pretolerance >= 0 {
        match best_path(&breaks, hsize, pretolerance, Pass::First, tracing) {
            Some((path, _total, t)) => {
                trace.push_str(&t);
                return (path_to_lines(&breaks, &path), trace);
            }
            None => {
                // 失败：保留 @firstpass 标记，走第二遍（tex.web second_pass）
                if tracing {
                    trace.push_str("@firstpass\n");
                }
            }
        }
    }
    // 第二遍：\tolerance（tex.web second_pass；\pretolerance=-1 时唯一一遍）
    let (path, _total, t) =
        best_path(&breaks, hsize, tolerance, Pass::Second, tracing).expect("第二遍必有路径");
    trace.push_str(&t);
    (path_to_lines(&breaks, &path), trace)
}

fn path_to_lines(breaks: &[BreakSpec], path: &[usize]) -> Vec<(usize, usize)> {
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
    use crate::node::{FontId, Node, GLUE_ORDER_FIL};

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
            name: None,
            width: w,
            stretch: st,
            shrink: sh,
            stretch_order: 0,
            shrink_order: 0,
        }
    }

    fn fil_glue() -> Node {
        Node::Glue {
            name: None,
            width: 0,
            stretch: 1,
            shrink: 0,
            stretch_order: GLUE_ORDER_FIL,
            shrink_order: 0,
        }
    }

    /// 断字 discretionary：pre = 连字符（宽 `w`），post/replace 空。
    fn disc(w: i64) -> Node {
        Node::Discretionary {
            pre: vec![Node::Char {
                font: FontId(0),
                charcode: 45,
                width: w,
                height: 0,
                depth: 0,
            }],
            post: Vec::new(),
            replace: Vec::new(),
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
        let (_, total, _) = best_path(&breaks, hsize, tolerance, Pass::Second, false).unwrap();
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
        let cases: [(&[i64], i64); 4] = [
            (&[10, 10, 10, 10], 25),
            (&[10, 10, 10, 10], 20),
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
        assert_eq!(knuth_plass(&hlist, 100, 200, 100, false).0, vec![(0, 5)]);
    }

    #[test]
    fn knuth_plass_breaks_at_spaces() {
        // 三词 a b c：hsize 25 下 "a b" | "c" 为最优（断点胶水不入行——
        // 行 "a" 无内部胶水、badness 10000，故两词行更优）
        let hlist = words(&[10, 10, 10]);
        let lines = knuth_plass(&hlist, 25, 200, 100, false).0;
        assert_eq!(lines, vec![(0, 3), (4, 5)]);
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
        let lines = knuth_plass(&hlist, 10_000, 200, 100, false).0;
        // 强制断点（index 2）之前定案：第一行 [0, 2)；之后继续
        assert_eq!(lines, vec![(0, 2), (3, 6)]);
    }

    #[test]
    fn knuth_plass_empty_input() {
        assert!(knuth_plass(&[], 100, 200, 100, false).0.is_empty());
    }

    #[test]
    fn knuth_plass_artificial_overfull_chain() {
        // 全部断点处行超宽（shrink 不足，b=inf_bad+1）：按 tex.web artificial
        // demerits 逐断点成行——不再退化为"恢复末起点"的单条巨行。
        let lines = knuth_plass(&words(&[10, 10, 10, 10]), 12, 200, 100, false).0;
        // 可行处照常成行（[0,2] 收缩可容纳），不可行处 artificial 兜底推进——
        // 只断言不再退化为单条巨行（旧行为 = 1 行）且词序保持。
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], (0, 3));
    }

    #[test]
    fn knuth_plass_no_breaks_single_word() {
        assert_eq!(
            knuth_plass(&[char_of(10), char_of(10)], 5, 200, 100, false).0,
            vec![(0, 2)]
        );
    }

    #[test]
    fn knuth_plass_keeps_trailing_glue() {
        // 预处理不裁剪尾部（裁剪在排版器 close_paragraph）
        let mut hlist = words(&[10, 10]);
        hlist.push(glue(3, 0, 0));
        let lines = knuth_plass(&hlist, 100, 200, 100, false).0;
        assert_eq!(lines, vec![(0, 4)]);
    }

    #[test]
    fn fil_glue_makes_line_acceptable() {
        // 两词 + parfillskip（0pt plus 1fil）：
        // 1 行（23 宽，需收缩 1、可用 5 → badness 1 → demerits (10+1)²=121）
        // vs 2 行（各 0 badness → demerits 100+100=200）→ 1 行胜（TeX 精确 demerits）
        let mut hlist = words(&[10, 10]);
        hlist.push(fil_glue());
        let lines = knuth_plass(&hlist, 22, 200, 100, false).0;
        assert_eq!(lines, vec![(0, 4)]);
    }

    #[test]
    fn fil_glue_single_line_when_fits() {
        let mut hlist = words(&[10, 10]);
        hlist.push(fil_glue());
        assert_eq!(knuth_plass(&hlist, 100, 200, 100, false).0, vec![(0, 4)]);
    }

    // ---------- M4-6 断字：discretionary 断点 ----------

    #[test]
    fn discretionary_break_chosen_when_word_too_long() {
        // "m abcdefgh n"（char 宽 10、词间 glue 3/1000/14、词 "abcdefgh" 断点 2 的
        // discretionary 连字符宽 5），hsize 60：
        // 不断字时整行/长行 badness > tolerance 被拒（或强制末行 demerits 巨大），
        // 唯一可行路径断在词内 → 行1 = "m ab-"（行尾补连字符）、行2 = "cdefgh n"。
        let mut hlist: Vec<Node> = vec![
            char_of(10),       // m
            glue(3, 1000, 14), // 词间
            char_of(10),       // a
            char_of(10),       // b
            disc(5),           // 断点 2：pre = 连字符
        ];
        for _ in 0..6 {
            hlist.push(char_of(10)); // c d e f g h
        }
        hlist.push(glue(3, 1000, 14)); // 词间
        hlist.push(char_of(10)); // n
        hlist.push(fil_glue());
        let lines = knuth_plass(&hlist, 60, 200, 100, false).0;
        // 行1 = [0..4]（m 空格 a b）+ discretionary pre；行2 = [5..14]（c..h 空格 n fil）
        assert_eq!(lines, vec![(0, 4), (5, 14)]);
    }

    #[test]
    fn discretionary_ignored_when_word_fits() {
        // 词宽 40 + 断点 discretionary：hsize 100 单行即可（fil 拉伸），不选断字
        let mut hlist: Vec<Node> =
            vec![char_of(10), char_of(10), disc(5), char_of(10), char_of(10)];
        hlist.push(fil_glue());
        assert_eq!(knuth_plass(&hlist, 100, 200, 100, false).0, vec![(0, 6)]);
    }
}
