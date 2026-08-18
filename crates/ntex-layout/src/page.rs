//! 断页 DP（M3-5-2）：TeX page builder（tex.web §598-§611 子集）。
//!
//! 实现 `build_page` / `fire_up` / `vpackage`：
//!
//! - 贡献列表（顶层垂直列表）逐节点搬入当前页，跟踪 `page_total`（自然高度）、
//!   各阶拉伸、收缩、最后盒子深度与 `\maxdepth` 钳制；
//! - 胶水/kern（后继为胶水）/penalty 是合法断点，按 TeX §602 计算 badness 与
//!   成本 `c`，维护最佳断点（`<=` 平局取后者）；`c=awful` 或 `\penalty≤-10000`
//!   立即 `fire_up`；
//! - `fire_up` 按最佳断点把页面切为 vbox（`vpackage` 以 `best_size`=页目标高打包，
//!   glue_set 按 TeX 累积舍入烘焙进胶水宽度），余下节点退回贡献列表前端；
//!   触发节点（eject 的 penalty 等）留待对（新）空页重处理并被丢弃——
//!   这正是 `\end` 不产生多余空页的机制。
//!
//! 不支持：insert/mark/whatsit（M4+）。用户 output routine（M3-5-3）由
//! [crate::typeset::NodeBuilder] 在页面产出后路由到 box255 + 引擎 token 注入。

use ntex_core::param::Params;

use crate::node::{BoxKind, BoxNode, Node};

/// 无穷惩罚 / badness 常量（tex.web §3258、§18983）。
const INF_PENALTY: i64 = 10_000;
const EJECT_PENALTY: i64 = -INF_PENALTY;
const INF_BAD: i64 = 10_000;
const AWFUL_BAD: i64 = (1 << 30) - 1; // tex.web `@'7777777777`
const DEPLORABLE: i64 = 100_000;

/// `ignore_depth`（tex.web §321）：无前驱盒子时 interline glue 被抑制。
pub const IGNORE_DEPTH: i64 = -65_536_000;

/// TeX `badness(t, s)`（tex.web §7）：`t≥0` 超出量，`s` 拉伸/收缩量。
fn badness(t: i64, s: i64) -> i64 {
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
    ((cube + 131_072) / 262_144).min(10_000) as i64
}

/// 页面构建器状态（tex.web `page_so_far` 的 Rust 表达）。
#[derive(Debug)]
pub struct PageBuilder {
    /// 当前页节点（已接收的贡献；`fire_up` 时按最佳断点切片）。
    page: Vec<Node>,
    /// page_so_far[1]：自然高度。
    total: i64,
    /// page_so_far[2..6]：各阶拉伸 [normal, fil, fill, filll]。
    stretch: [i64; 4],
    /// page_so_far[6]：收缩。
    shrink: i64,
    /// page_so_far[7]：最后盒子深度（`\maxdepth` 钳制后）。
    depth: i64,
    /// page_so_far[0]：页目标高度（freeze 时 `\vsize`）。
    goal: i64,
    /// 页最大深度（freeze 时 `\maxdepth`）。
    max_depth: i64,
    /// 是否已有盒子（`freeze_page_specs(box_there)` 已执行）。
    has_box: bool,
    /// 最佳断点（`page` 下标；= page.len() 表示"触发节点处"即页末端）。
    best: Option<usize>,
    /// 最佳断点成本（`least_page_cost`）。
    best_cost: i64,
    /// 最佳断点时的目标高度（`best_size`，fire_up 打包用）。
    best_size: i64,
    /// 最后压入节点是否为盒子/规则（断点合法性 `precedes_break`）。
    last_is_box: bool,
    /// 页面最后**盒子**的深度（tex.web `prev_depth`；胶水不重置，
    /// 规则/断页重置为 [`IGNORE_DEPTH`]——interline glue 的依据）。
    prev_depth: i64,
}

/// `process` 单节点处理结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// 节点已入页/丢弃，继续处理下一个贡献。
    Continue,
    /// 发生断页：触发节点仍在贡献列表前端，需 `fire_up`。
    FireUp,
}

impl Default for PageBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl PageBuilder {
    pub fn new() -> Self {
        Self {
            page: Vec::new(),
            total: 0,
            stretch: [0; 4],
            shrink: 0,
            depth: 0,
            goal: 0,
            max_depth: 0,
            has_box: false,
            best: None,
            best_cost: AWFUL_BAD,
            best_size: 0,
            last_is_box: false,
            prev_depth: IGNORE_DEPTH,
        }
    }

    /// 当前页是否为空（供收尾 eject 判断）。
    pub fn is_empty(&self) -> bool {
        !self.has_box && self.page.is_empty()
    }

    /// 页面最后盒子的深度（push_box 的 interline glue 用；tex.web `prev_depth`）。
    pub fn prev_depth(&self) -> i64 {
        self.prev_depth
    }

    /// 页面最后节点是否为盒子/规则（push_box 的 interline glue 用）。
    pub fn last_is_box(&self) -> bool {
        self.last_is_box
    }

    /// 把贡献列表逐节点搬入当前页（tex.web `build_page` 主循环）。
    ///
    /// 增量版：每产出**一页**即返回（M3-5-3 输出例程需要页面产出后暂停，
    /// 让引擎在 token 边界执行例程，再继续处理剩余贡献）。
    /// 返回 `None` 表示贡献耗尽且无页面产出。
    pub fn feed_one(&mut self, contrib: &mut Vec<Node>, params: &Params) -> Option<BoxNode> {
        loop {
            if contrib.is_empty() {
                return None;
            }
            match self.process(contrib, params) {
                Outcome::Continue => {}
                Outcome::FireUp => {
                    // fire_up 已把页内剩余节点拼回贡献前端，由调用方继续处理
                    return Some(self.fire_up(contrib));
                }
            }
        }
    }

    /// 处理贡献列表头节点（tex.web "Move node p to the current page"）。
    ///
    /// 入页/丢弃时从贡献移除；fire_up 时保留在贡献前端。
    fn process(&mut self, contrib: &mut Vec<Node>, params: &Params) -> Outcome {
        let node = contrib[0].clone();
        match node {
            Node::Box(b) => {
                if !self.has_box {
                    // 页面初始化 + 首盒前插入 \topskip 胶水（tex.web §509）
                    self.freeze(params);
                    let w = (params.topskip.width - b.height).max(0);
                    if w > 0 {
                        self.page.push(Node::Glue {
                            width: w,
                            stretch: params.topskip.stretch,
                            shrink: params.topskip.shrink,
                            stretch_order: 0,
                            shrink_order: 0,
                        });
                        // topskip 胶水 update_heights（前驱为 glue 非断点）
                        self.total += w;
                        self.last_is_box = false;
                    }
                }
                contrib.remove(0);
                self.page.push(Node::Box(b));
                self.add_box_dims();
                self.last_is_box = true;
                Outcome::Continue
            }
            Node::Rule {
                width,
                height,
                depth,
            } => {
                if !self.has_box {
                    self.freeze(params);
                    let w = (params.topskip.width - height).max(0);
                    if w > 0 {
                        self.page.push(Node::Glue {
                            width: w,
                            stretch: params.topskip.stretch,
                            shrink: params.topskip.shrink,
                            stretch_order: 0,
                            shrink_order: 0,
                        });
                        self.total += w;
                        self.last_is_box = false;
                    }
                }
                contrib.remove(0);
                self.page.push(Node::Rule {
                    width,
                    height,
                    depth,
                });
                self.add_box_dims();
                self.last_is_box = true;
                Outcome::Continue
            }
            Node::Glue {
                width,
                stretch,
                shrink,
                stretch_order,
                shrink_order,
            } => {
                // 空页上的胶水直接丢弃（tex.web §495）
                if !self.has_box {
                    contrib.remove(0);
                    return Outcome::Continue;
                }
                // 胶水是断点 iff 前驱是盒子/规则（tex.web §496）
                if self.last_is_box && self.try_break(0) == Some(Outcome::FireUp) {
                    return Outcome::FireUp;
                }
                contrib.remove(0);
                self.page.push(Node::Glue {
                    width,
                    stretch,
                    shrink,
                    stretch_order,
                    shrink_order,
                });
                // update_heights（tex.web §550）
                self.total += self.depth + width;
                self.depth = 0;
                self.stretch[stretch_order as usize] += stretch;
                self.shrink += shrink;
                self.last_is_box = false;
                Outcome::Continue
            }
            Node::Kern { width } => {
                if !self.has_box {
                    contrib.remove(0);
                    return Outcome::Continue;
                }
                // kern 仅在后继为胶水时才是断点（tex.web §498，需前瞻贡献）
                let followed_by_glue =
                    matches!(contrib.get(1), Some(Node::Glue { .. }));
                if followed_by_glue && self.try_break(0) == Some(Outcome::FireUp) {
                    return Outcome::FireUp;
                }
                contrib.remove(0);
                self.page.push(Node::Kern { width });
                self.total += self.depth + width;
                self.depth = 0;
                self.last_is_box = false;
                Outcome::Continue
            }
            Node::Penalty { penalty } => {
                if !self.has_box {
                    contrib.remove(0);
                    return Outcome::Continue;
                }
                if penalty < INF_PENALTY
                    && self.try_break(penalty) == Some(Outcome::FireUp)
                {
                    return Outcome::FireUp;
                }
                contrib.remove(0);
                self.page.push(Node::Penalty { penalty });
                Outcome::Continue
            }
            // mark/leaders 等：直接入页（不参与断点与测量）
            _ => {
                contrib.remove(0);
                self.page.push(node);
                Outcome::Continue
            }
        }
    }

    /// 盒子/规则入页后的测量（tex.web §518-519）：吸收前盒深度，更新本盒深度。
    /// `prev_depth`：盒子记录其深度、规则抑制（tex.web `append_to_vlist` §326）。
    fn add_box_dims(&mut self) {
        let (height, depth) = match self.page.last().expect("刚入页") {
            Node::Box(b) => (b.height, b.depth),
            Node::Rule { height, depth, .. } => (*height, *depth),
            _ => unreachable!("add_box_dims 只用于盒子/规则"),
        };
        self.total += self.depth + height;
        self.depth = depth;
        self.clamp_depth();
        match self.page.last().expect("刚入页") {
            Node::Box(b) => self.prev_depth = b.depth,
            Node::Rule { .. } => self.prev_depth = IGNORE_DEPTH,
            _ => unreachable!(),
        }
    }

    /// `\maxdepth` 钳制（tex.web §524-529）：超出的深度移到页高测量上。
    fn clamp_depth(&mut self) {
        if self.depth > self.max_depth {
            self.total += self.depth - self.max_depth;
            self.depth = self.max_depth;
        }
    }

    /// 计算当前页 badness（tex.web §593-599，不含触发节点自身）。
    fn badness_now(&self) -> i64 {
        if self.total < self.goal {
            if self.stretch[1] != 0 || self.stretch[2] != 0 || self.stretch[3] != 0 {
                0
            } else {
                badness(self.goal - self.total, self.stretch[0])
            }
        } else if self.total - self.goal > self.shrink {
            AWFUL_BAD
        } else {
            badness(self.total - self.goal, self.shrink)
        }
    }

    /// 尝试在触发节点处断页（tex.web §552-577）：
    /// 计算 badness 与成本 `c`，更新最佳断点；`c=awful` 或强制断点 → fire_up。
    fn try_break(&mut self, pi: i64) -> Option<Outcome> {
        let b = self.badness_now();
        let c = if b < AWFUL_BAD {
            if pi <= EJECT_PENALTY {
                pi
            } else if b < INF_BAD {
                b + pi
            } else {
                DEPLORABLE
            }
        } else {
            b
        };
        if c <= self.best_cost {
            // 触发节点尚未入页：断点位置 = 页末端
            self.best = Some(self.page.len());
            self.best_size = self.goal;
            self.best_cost = c;
        }
        if c == AWFUL_BAD || pi <= EJECT_PENALTY {
            Some(Outcome::FireUp)
        } else {
            None
        }
    }

    /// `fire_up`（tex.web §709+）：按最佳断点打包页面、余下退回贡献、重置页面。
    fn fire_up(&mut self, contrib: &mut Vec<Node>) -> BoxNode {
        let cut = self.best.unwrap_or(self.page.len());
        let page = self.package(cut);
        // 余下节点 [cut..] 拼回贡献列表前端（触发节点之前）
        contrib.splice(0..0, self.page.drain(cut..));
        self.start_new_page();
        page
    }

    /// `vpackage(link(page_head), best_size, exactly, max_depth)`（tex.web §161-184）：
    /// 计算自然高度与胶水总量，按 best_size 确定 glue_set（stretching/shrinking），
    /// 并把调整量按累积舍入（vlist_out 的 `cur_g`）烘焙进胶水宽度。
    fn package(&mut self, cut: usize) -> BoxNode {
        let children = &self.page[0..cut];
        // 自然高度 / 深度 / 各阶拉伸收缩 / 宽度
        let mut x = 0i64;
        let mut d = 0i64;
        let mut stretch = [0i64; 4];
        let mut shrink = [0i64; 4];
        let mut width = 0i64;
        for c in children {
            match c {
                Node::Box(b) => {
                    x += d + b.height;
                    d = b.depth;
                    width = width.max(b.width + b.shift);
                }
                Node::Rule { width: w, height, depth } => {
                    x += d + height;
                    d = *depth;
                    width = width.max(*w);
                }
                Node::Glue {
                    width: w,
                    stretch: s,
                    shrink: sh,
                    stretch_order,
                    shrink_order,
                } => {
                    x += d + w;
                    d = 0;
                    stretch[*stretch_order as usize] += s;
                    shrink[*shrink_order as usize] += sh;
                    width = width.max(*w);
                }
                Node::Kern { width: w } => {
                    x += d + w;
                    d = 0;
                }
                Node::Leaders { width: w, inner, .. } => {
                    x += d + inner.height + inner.depth;
                    d = inner.depth;
                    width = width.max(*w);
                }
                Node::Char { .. } | Node::Penalty { .. } => {}
            }
        }
        if d > self.max_depth {
            x += d - self.max_depth;
            d = self.max_depth;
        }
        let height = self.best_size;
        let excess = height - x;
        // glue_set（tex.web §236-293）
        enum Sign {
            Normal,
            Stretch,
            Shrink,
        }
        let (sign, order, gs) = if excess == 0 {
            (Sign::Normal, 0, 0.0)
        } else if excess > 0 {
            let o = (0..4).rev().find(|&o| stretch[o] != 0);
            match o {
                Some(o) => (Sign::Stretch, o, excess as f64 / stretch[o] as f64),
                None => (Sign::Normal, 0, 0.0),
            }
        } else {
            let o = (0..4).rev().find(|&o| shrink[o] != 0);
            match o {
                Some(o) => {
                    let gs = (-excess) as f64 / shrink[o] as f64;
                    // 普通阶收缩不足时钳到最大收缩（tex.web §283-288）
                    let gs = if o == 0 && shrink[o] < -excess { 1.0 } else { gs };
                    (Sign::Shrink, o, gs)
                }
                None => (Sign::Normal, 0, 0.0),
            }
        };
        // 烘焙胶水宽度（tex.web vlist_out §607-624 的累积舍入）
        let mut out: Vec<Node> = Vec::with_capacity(cut);
        let mut cum = 0f64;
        let mut prev_g = 0f64;
        for c in children {
            match c {
                Node::Glue {
                    width: w,
                    stretch: s,
                    shrink: sh,
                    stretch_order,
                    shrink_order,
                } => {
                    let mut w = *w;
                    match sign {
                        Sign::Stretch if *stretch_order as usize == order => {
                            cum += *s as f64;
                            let g = (gs * cum).round();
                            w += g as i64 - prev_g as i64;
                            prev_g = g;
                        }
                        Sign::Shrink if *shrink_order as usize == order => {
                            cum -= *sh as f64;
                            let g = (gs * cum).round();
                            w += g as i64 - prev_g as i64;
                            prev_g = g;
                        }
                        _ => {}
                    }
                    out.push(Node::Glue {
                        width: w,
                        stretch: *s,
                        shrink: *sh,
                        stretch_order: *stretch_order,
                        shrink_order: *shrink_order,
                    });
                }
                other => out.push(other.clone()),
            }
        }
        BoxNode {
            kind: BoxKind::VBox,
            width,
            height,
            depth: d,
            shift: 0,
            children: out,
        }
    }

    /// `freeze_page_specs(box_there)`（tex.web §519）：首盒到达时定格页规格。
    fn freeze(&mut self, params: &Params) {
        self.has_box = true;
        self.goal = params.vsize;
        self.max_depth = params.maxdepth;
        self.depth = 0;
        self.total = 0;
        self.stretch = [0; 4];
        self.shrink = 0;
        self.best = None;
        self.best_cost = AWFUL_BAD;
        self.best_size = self.goal;
        self.last_is_box = false;
        self.prev_depth = IGNORE_DEPTH;
    }

    /// `@<Start a new current page@>`（tex.web §373-376）。
    fn start_new_page(&mut self) {
        self.page.clear();
        self.has_box = false;
        self.depth = 0;
        self.max_depth = 0;
        self.total = 0;
        self.stretch = [0; 4];
        self.shrink = 0;
        self.best = None;
        self.best_cost = AWFUL_BAD;
        self.best_size = 0;
        self.last_is_box = false;
        self.prev_depth = IGNORE_DEPTH;
    }
}
