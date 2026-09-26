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
//!   子节点宽度保持自然值——tex.web 把 glue_set 存在盒上而 NTex 无此字段，烘焙会
//!   经 `\unvbox\@cclv` 残留进输出例程的重打包），余下节点退回贡献列表前端；
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

/// 页面构建器状态（tex.web `page_so_far` 的 Rust 表达）。
/// `Clone`（M5 阶段三）：排版层段边界检查点克隆页面构建器
/// （当前页内容 + page_so_far 测量 + 最佳断点），回滚/复用推进共用。
#[derive(Debug, Clone)]
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
    /// 最佳断点是否惩罚节点及其惩罚值（tex.web `fire_up` 的
    /// `@<Set the value of |output_penalty|@>`：`type(best_page_break)=penalty_node`
    /// → `\outputpenalty := penalty(best_page_break)`；否则（胶水/kern 断点）
    /// → `\outputpenalty := inf_penalty`(10000)。输出例程点火前由引擎经
    /// TokenSink 查询写入 [`Primitive::OutputPenalty`](ntex_core::eqtb)。
    best_penalty: Option<i64>,
    /// `fire_up` 已为最近产出的页面写定的 `\outputpenalty` 值（tex.web
    /// `@<Set the value of |output_penalty|@>` 在 fire_up 入口执行，先于
    /// `start_new_page` 重置）——引擎在例程点火前经 [`Self::take_output_penalty`]
    /// 取走（take 语义，防例程延后注入时读到陈旧断点）。
    fired_penalty: Option<i64>,
    /// 最佳断点成本（`least_page_cost`）。
    best_cost: i64,
    /// 最佳断点时的目标高度（`best_size`，fire_up 打包用）。
    best_size: i64,
    /// 最后压入节点是否为盒子/规则（断点合法性 `precedes_break`）。
    last_is_box: bool,
    /// 页面最后**盒子**的深度（tex.web `prev_depth`；胶水不重置，
    /// 规则/断页重置为 [`IGNORE_DEPTH`]——interline glue 的依据）。
    prev_depth: i64,
    /// `\tracingpages>0`：本次 feed 的断页追踪（tex.web `Display the page break cost`）。
    tracing: bool,
    /// 空页零尺寸空盒持有槽（裁决未定）：真 TeX 会把 `\vbox{}` 入页使页非空，
    /// 其后的强制惩罚照常点火；NTex 为压 `\clearpage` 空白页而暂扣该盒，
    /// 待下一贡献裁决去向。见 [`Self::process`] Box 臂注释。
    held_zero_empty: Option<Node>,
    /// 持有盒放行旗标：裁决为 -10003 二击标记时置位，Box 臂消费后清除。
    keep_zero_empty: bool,
    /// 放行的页首零盒只用于让后继顶部胶水保留，不作为首基线插入 topskip。
    release_zero_without_topskip: bool,
    /// 追踪输出缓冲（feed_one 后由调用方取走写转录）。
    trace_buf: String,
    /// `\vsize` 的**事件面**最新值（[`Primitive::VSize`] 赋值经 `param_changed`
    /// 送达；与 `params.vsize` 镜像并行的第二通道）。输出例程在组内执行，组尾
    /// 的参数回滚会不经 `param_changed` 直接改写镜像，LaTeX 输出例程尾
    /// `\global\vsize\@colroom` 的目标收紧因此到不了 freeze——浮体页按整页
    /// 目标断出后再叠上浮体高度，正文冲出版心（transformer p6/p9 实测）。
    /// 事件面值只在显式赋值时更新，恰好承载 LaTeX「每次例程尾都重设 \vsize」
    /// 的契约；None 时退回镜像。
    vsize_live: Option<i64>,
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
            best_penalty: None,
            fired_penalty: None,
            best_cost: AWFUL_BAD,
            best_size: 0,
            last_is_box: false,
            prev_depth: IGNORE_DEPTH,
            tracing: false,
            held_zero_empty: None,
            keep_zero_empty: false,
            release_zero_without_topskip: false,
            trace_buf: String::new(),
            vsize_live: None,
        }
    }

    /// `\vsize` 事件面更新（[NodeBuilder::param_changed] 调用）。
    pub fn note_vsize(&mut self, v: i64) {
        self.vsize_live = Some(v);
    }

    /// 本轮 feed 生效的断页目标（事件面优先，镜像兜底）。
    fn live_vsize(&self, params: &Params) -> i64 {
        self.vsize_live.unwrap_or(params.vsize)
    }

    /// 当前页是否为空（供收尾 eject 判断）。
    pub fn is_empty(&self) -> bool {
        !self.has_box
    }

    /// 页面是否含**可冲页内容**（tex.web `page_head<>null` 判据）。
    ///
    /// tex.web 的 `page_head` 只在 `append_to_vlist` 加入**盒子/规则**时建立；
    /// 纯胶水（`\vskip`/`\vfill`）、kern、惩罚**不建立** `page_head`。故
    /// `fire_up` 在 `page_head=null` 时**丢弃该页**——这正是 pdfTeX 对
    /// `\vfill\supereject\end`（plain `\bye` 的形态）输出 `No pages of output.`
    /// 的原因，而 NTex 曾冲出一个**纯胶水假页**（corpus `plain/*` 三个 EMPTY）。
    ///
    /// ⚠ 注意本判据**只在「冲页触发点」使用**：`\end` 的 `its_all_over` 路径
    /// 对纯胶水**仍然冲页**（pdfTeX 实测 `\vfill\end` → 1 页）。两条路径的
    /// 差别见 `docs/archive/latex-feasibility.md` 与 `scripts/page-eject-matrix.py`。
    ///
    /// 判据（对齐 tex.web `append_to_vlist` 会建 `page_head` 的节点类型）：
    /// `Box` / `Rule` 算；`Glue` / `Kern`(垂直) / `Penalty` / `Mark` / `Insert` 不算。
    pub fn has_shippable_content(&self) -> bool {
        self.page
            .iter()
            .any(|n| matches!(n, Node::Box(_) | Node::Rule { .. }))
            || self.has_box
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
        // \tracingpages：misc 59（与 ntex-core int_param_index 对齐）
        self.tracing = params.misc[59] > 0;
        if self.tracing && std::env::var("NTEX_DEBUG_TRACINGPAGES").is_ok() {
            eprintln!("[tracingpages] feed_one: tracing={}", self.tracing);
        }
        loop {
            // LaTeX 浮体契约的页构建器侧落点：`\@addtocurcol` 把浮体放上**当前**
            // 页时经 `\@flupdates` 缩小 `\@colroom`，输出例程尾的 `\global\vsize
            // \@colroom` 随之收紧断页目标。真 TeX 靠 `\end@float` 的 -\@Miii(-10003)
            // 强制惩罚触发 `\@specialoutput` 把半成品页 `\unvbox\@holdpg` 退回贡献
            // 列表重建，新目标因此生效；NTex 的 -10003 走既有强制断页臂（页已
            // 成形再退回会在例程里丢材料），故这里在页构建器入口补同一效果：
            // **目标在页中途被收紧 → 半成品页整表退回贡献、按新目标重冻**。
            // 只认收紧（`\vsize\maxdimen` 的「本页不再断」臂与 `\enlargethispage`
            // 的放宽不触发），且只认已冻页（has_box）——页空时下一次 freeze 自然
            // 取到新值。每轮收紧至多触发一次（退回后 has_box=false）。
            let vsize = self.live_vsize(params);
            if self.has_box && self.goal > vsize {
                contrib.splice(0..0, self.page.drain(..));
                self.start_new_page();
            }
            // 持有盒裁决：NTex 的贡献列表在每次入页后即被抽干，tex.web 的
            // 「空盒入页 → 惩罚照常点火」次序在此不可见，只能扣住空盒等下一个
            // 贡献揭晓身份。`\end@float` 的 -\@Miii(-10003) 是「页已断、
            // `\@holdpg` 待回流」二击标记 → 放回空盒让惩罚在非空页点火；
            // 其余（`\clearpage` 尾 -\@Mi、`\@emptycol` -\@M）→ 按第二十二刀
            // 语义丢弃持有盒，防空空白页不变量不变。
            if !contrib.is_empty() {
                if let Some(held) = self.held_zero_empty.take() {
                    let keep = match contrib.first() {
                        Some(Node::Penalty { penalty: -10003 }) => {
                            self.keep_zero_empty = true;
                            true
                        }
                        // tex.web 空盒入页使页非空：后续**非惩罚**材料（如
                        // `\@maketitle` 的 `\null\vskip 2em` 顶部留空）在真 TeX
                        // 里胶水照常保留——盒不空则页顶胶水不丢。此处放行入页，
                        // 让顶部 `\vskip` 落在盒后（GT：标题基线低 22pt+topskip）。
                        // 其余惩罚（`\clearpage` 尾 -\@Mi、`\@emptycol` -\@M）
                        // 仍按第二十二刀语义丢弃持有盒，防空空白页不变量不变。
                        Some(n) if !matches!(n, Node::Penalty { .. }) => {
                            self.keep_zero_empty = true;
                            self.release_zero_without_topskip = true;
                            true
                        }
                        _ => false,
                    };
                    if keep {
                        contrib.insert(0, held);
                    }
                }
            }
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
                // LaTeX `\clearpage` 尾部在强制惩罚前放一个 `\vbox{}`；`\end@float`
                // 尾部放 `\vbox{}\penalty-\@Miii(-10003)`。真 TeX 不区分两者：空盒
                // 入页使页非空（tex.web 盒分支无条件 `page_contents:=box_there`），
                // 其后的强制惩罚照常点火——`\clearpage` 由 `\@doclearpage` 例程吞页
                // 防空白，`\end@float` 靠这记二次点火做 `\unvbox\@holdpg` 回流。
                //
                // NTex 无例程语境下第二十二刀（675b50a）把空页零盒丢弃以压
                // `\clearpage` 空白页。但若空盒被丢，-10003 落在空页被 tex.web
                // L19502 丢惩罚规则吞掉 → 二击永不发生 → `\@specialoutput` 存入
                // `\@holdpg` 的整页内容永久滞留 = 浮体之前的已排版段落整段蒸发
                // （transformer-standalone Figure 1 起丢半篇、少 ~8000 字形）。
                // 故这里**持有待裁决**：扣住空盒，feed_one 下一贡献揭晓身份——
                // -10003 → 放行入页（二击成立）；其余 → 丢弃（原语义）。
                // `eject_one_page` 自造的 `\hbox to \hsize{}` 有宽度，不走此臂，
                // M5 防空页死循环不变量不变。
                if !self.has_box && b.is_zero_empty() && !std::mem::take(&mut self.keep_zero_empty)
                {
                    self.held_zero_empty = Some(Node::Box(b));
                    contrib.remove(0);
                    return Outcome::Continue;
                }
                let suppress_topskip =
                    b.is_zero_empty() && std::mem::take(&mut self.release_zero_without_topskip);
                if !self.has_box && !suppress_topskip {
                    // 页面初始化 + 首盒前插入 \topskip 胶水（tex.web §509）。
                    // topskip 的断点尝试（p=0，t=0）只输出追踪行，不作为断点候选
                    // （避免 \vsize 极小 + topskip 超页时空页 fire_up 死循环）。
                    self.freeze(params);
                    let w = (params.topskip.width - b.height).max(0);
                    if w > 0 {
                        if self.tracing {
                            let b_now = self.badness_now();
                            let c = if b_now < AWFUL_BAD {
                                if b_now < INF_BAD {
                                    b_now
                                } else {
                                    DEPLORABLE
                                }
                            } else {
                                b_now
                            };
                            self.trace_buf.push_str(&self.trace_break_line(0, b_now, c));
                        }
                        self.page.push(Node::Glue {
                            name: None,
                            width: w,
                            stretch: params.topskip.stretch,
                            shrink: params.topskip.shrink,
                            stretch_order: params.topskip.stretch_order,
                            shrink_order: params.topskip.shrink_order,
                        });
                        // topskip 胶水 update_heights（前驱为 glue 非断点）
                        self.total += w;
                        self.stretch[params.topskip.stretch_order as usize] +=
                            params.topskip.stretch;
                        self.shrink += params.topskip.shrink;
                        self.last_is_box = false;
                    }
                } else if !self.has_box {
                    self.freeze(params);
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
                            name: None,
                            width: w,
                            stretch: params.topskip.stretch,
                            shrink: params.topskip.shrink,
                            stretch_order: params.topskip.stretch_order,
                            shrink_order: params.topskip.shrink_order,
                        });
                        self.total += w;
                        self.stretch[params.topskip.stretch_order as usize] +=
                            params.topskip.stretch;
                        self.shrink += params.topskip.shrink;
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
                name: None,
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
                // 胶水是断点 iff 前驱节点 precedes_break（tex.web：type < math_node，
                // 含盒子/规则/胶水/kern——非 penalty/mark/insert 即可断）
                if self.precedes_break() && self.try_break(0, false) == Some(Outcome::FireUp) {
                    return Outcome::FireUp;
                }
                contrib.remove(0);
                self.page.push(Node::Glue {
                    name: None,
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
                let followed_by_glue = matches!(contrib.get(1), Some(Node::Glue { .. }));
                if followed_by_glue && self.try_break(0, false) == Some(Outcome::FireUp) {
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
                if penalty < INF_PENALTY && self.try_break(penalty, true) == Some(Outcome::FireUp) {
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

    /// tex.web `precedes_break`：最后节点类型 < math_node（盒子/规则/胶水/kern/
    /// 空页尾）为合法断点前驱；penalty/mark/insert/whatsit 不可断。
    /// 注意：topskip 不参与（它直接入页，仅手动输出追踪行），避免 \vsize 极小
    /// 时 topskip 成为空页断点候选 → fire_up 空页死循环。
    fn precedes_break(&self) -> bool {
        matches!(
            self.page.last(),
            None | Some(Node::Box(_) | Node::Rule { .. } | Node::Glue { .. } | Node::Kern { .. })
        )
    }

    /// 计算当前页 badness（tex.web §593-599，不含触发节点自身；C3：与折行共享实现）。
    fn badness_now(&self) -> i64 {
        if self.total < self.goal {
            if self.stretch[1] != 0 || self.stretch[2] != 0 || self.stretch[3] != 0 {
                0
            } else {
                crate::linebreak::badness(self.goal - self.total, self.stretch[0]) as i64
            }
        } else if self.total - self.goal > self.shrink {
            AWFUL_BAD
        } else {
            crate::linebreak::badness(self.total - self.goal, self.shrink) as i64
        }
    }

    /// 尝试在触发节点处断页（tex.web §552-577）：
    /// 计算 badness 与成本 `c`，更新最佳断点；`c=awful` 或强制断点 → fire_up。
    /// `at_penalty`：断点是否惩罚节点（`\outputpenalty` 的取值依据，见
    /// [`Self::output_break_penalty`]；胶水/kern 断点为 false）。
    fn try_break(&mut self, pi: i64, at_penalty: bool) -> Option<Outcome> {
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
        // \tracingpages：每次断点尝试输出成本行（tex.web `Display the page break cost`）
        if self.tracing {
            self.trace_buf.push_str(&self.trace_break_line(pi, b, c));
        }
        if c <= self.best_cost {
            // 触发节点尚未入页：断点位置 = 页末端
            self.best = Some(self.page.len());
            self.best_penalty = if at_penalty { Some(pi) } else { None };
            self.best_size = self.goal;
            self.best_cost = c;
        }
        if c == AWFUL_BAD || pi <= EJECT_PENALTY {
            Some(Outcome::FireUp)
        } else {
            None
        }
    }

    /// 断点成本追踪行（tex.web §588）：`% t=<totals> g=<goal> b=<b> p=<pi> c=<c>#`。
    /// `b`/`c` 为 awful 显示 `*`；`#` 仅当该断点成为新最佳（c <= best_cost）。
    fn trace_break_line(&self, pi: i64, b: i64, c: i64) -> String {
        let b_str = if b == AWFUL_BAD {
            "*".to_string()
        } else {
            b.to_string()
        };
        let c_str = if c == AWFUL_BAD {
            "*".to_string()
        } else {
            c.to_string()
        };
        let mut s = format!(
            "% t={} g={} b={} p={} c={}",
            self.print_totals(),
            ntex_core::register::format_dimen(self.goal),
            b_str,
            pi,
            c_str,
        );
        if c <= self.best_cost {
            s.push('#');
        }
        s.push('\n');
        s
    }

    /// `print_totals`（tex.web §19273）：自然高度 + 各阶拉伸（plus）+ 收缩（minus）。
    fn print_totals(&self) -> String {
        let mut s = ntex_core::register::format_dimen(self.total);
        for (order, suffix) in [(0, ""), (1, "fil"), (2, "fill"), (3, "filll")] {
            let v = self.stretch[order];
            if v != 0 {
                s.push_str(&format!(
                    " plus {}{}",
                    ntex_core::register::format_dimen(v),
                    suffix
                ));
            }
        }
        if self.shrink != 0 {
            s.push_str(&format!(
                " minus {}",
                ntex_core::register::format_dimen(self.shrink)
            ));
        }
        s
    }

    /// 取走本次 feed 的追踪输出（调用方写转录）。
    pub fn take_trace(&mut self) -> Option<String> {
        if self.trace_buf.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.trace_buf))
        }
    }

    /// 取走 `fire_up` 为最近产出页面写定的 `\outputpenalty` 值（tex.web
    /// `@<Set the value of |output_penalty|@>`）：最佳断点是惩罚节点 → 其惩罚值
    /// （如 `\newpage` 的 -10000、`\clearpage` 的 -10001、`\supereject` 的
    /// -20000）；胶水/kern 断点（页满自然断）与无记录 → `inf_penalty`(10000)。
    pub fn take_output_penalty(&mut self) -> Option<i64> {
        self.fired_penalty.take()
    }

    /// `fire_up`（tex.web §709+）：按最佳断点打包页面、余下退回贡献、重置页面。
    fn fire_up(&mut self, contrib: &mut Vec<Node>) -> BoxNode {
        // tex.web fire_up 入口：先写 \outputpenalty（最佳断点的惩罚值），再打包
        // 页面并 start_new_page（后者会重置断点记录，故必须在此捕获）。
        self.fired_penalty = Some(self.best_penalty.unwrap_or(INF_PENALTY));
        let cut = self.best.unwrap_or(self.page.len());
        let page = self.package(cut);
        // 余下节点 [cut..] 拼回贡献列表前端（触发节点之前）
        contrib.splice(0..0, self.page.drain(cut..));
        self.start_new_page();
        page
    }

    /// `vpackage(link(page_head), best_size, exactly, max_depth)`（tex.web §161-184）：
    /// tex.web 把 glue_set（sign/order/ratio）**存在盒上**、节点宽度一律保持自然值
    /// ——烘焙会使比值经 `\unvbox\@cclv` 残留：LaTeX `\@makecol` 把 box255 拆进
    /// `\vbox to\@colht` 重打包时，parskip 等有限拉伸胶已被页构建器预胀（段间距
    /// 按页放大的真根因），而真 TeX 的重打包从自然宽度重新结算（`\@textbottom`
    /// 的 .0001fil 吃掉全部余量）。NTex 盒上无 glue_set 字段，竖直列表渲染从顶
    /// 累加自然宽度、`(height−自然高)` 落在页尾——正是真 TeX 例程重打包后尾 fil
    /// 吸收余量的几何（\raggedbottom），故这里只定高度/深度/宽度，不动子节点。
    fn package(&mut self, cut: usize) -> BoxNode {
        let children = &self.page[0..cut];
        // 页深（\boxmaxdepth 钳制）与宽度
        let mut d = 0i64;
        let mut width = 0i64;
        for c in children {
            match c {
                Node::Box(b) => {
                    d = b.depth;
                    width = width.max(b.width + b.shift);
                }
                Node::Rule {
                    width: w, depth, ..
                } => {
                    d = *depth;
                    width = width.max(*w);
                }
                Node::Glue { width: w, .. } | Node::Kern { width: w } => {
                    d = 0;
                    width = width.max(*w);
                }
                Node::Leaders {
                    width: w, inner, ..
                } => {
                    let dims = inner.dimensions();
                    d = dims.depth;
                    width = width.max(*w);
                }
                Node::Char { .. }
                | Node::Ligature { .. }
                | Node::Penalty { .. }
                | Node::Discretionary { .. }
                | Node::Direction { .. }
                | Node::Mark { .. }
                | Node::Ins { .. }
                | Node::Adjust { .. }
                | Node::Whatsit { .. }
                | Node::MathOn { .. }
                | Node::MathOff { .. } => {}
            }
        }
        if d > self.max_depth {
            d = self.max_depth;
        }
        BoxNode {
            kind: BoxKind::VBox,
            width,
            height: self.best_size,
            depth: d,
            shift: 0,
            children: children.to_vec(),
        }
    }

    /// `freeze_page_specs(box_there)`（tex.web §519）：首盒到达时定格页规格。
    fn freeze(&mut self, params: &Params) {
        self.has_box = true;
        self.goal = self.live_vsize(params);
        self.max_depth = params.maxdepth;
        self.depth = 0;
        self.total = 0;
        self.stretch = [0; 4];
        self.shrink = 0;
        self.best = None;
        self.best_penalty = None;
        self.best_cost = AWFUL_BAD;
        self.best_size = self.goal;
        self.last_is_box = false;
        self.prev_depth = IGNORE_DEPTH;
        // \tracingpages：freeze 时输出目标行（tex.web freeze_page_specs）
        if self.tracing {
            self.trace_buf.push_str(&format!(
                "%% goal height={}, max depth={}\n",
                ntex_core::register::format_dimen(self.goal),
                ntex_core::register::format_dimen(self.max_depth)
            ));
        }
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
        self.best_penalty = None;
        self.best_cost = AWFUL_BAD;
        self.best_size = 0;
        self.last_is_box = false;
        self.prev_depth = IGNORE_DEPTH;
    }
}
