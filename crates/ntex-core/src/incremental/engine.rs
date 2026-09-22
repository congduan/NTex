//! 段级执行器（M5 阶段二，plan.md §7）：切段 → 带快照执行 → 缓存 → 回滚重算。
//!
//! 执行模型：段在**同一个** [`Expander`] 上按序 `feed_source + run`（顺序重放），
//! 每段记录**执行前/执行后**两份可还原快照（[`StateSnapshot`] = 完整状态检查点）
//! + 词法依赖 + 输出 token 并缓存。
//!
//! ## `edit` 单路径：回滚到段前 → 重放（正确性铁律：增量 == 全量逐位一致）
//!
//! 编辑第 k 段时，引擎的活状态是上一轮全文跑完的状态，而重算第 k 段需要的
//! 是"第 k 段执行前"的状态。阶段二快照可还原，于是**只有一条路径**：
//!
//! 1. 把引擎整体还原到 `pre(k)`（[`Expander::restore_checkpoint`]；编辑的段
//!    无缓存时取最近的更早缓存段——中间段会随重放重新执行）；
//! 2. 从 k 起重放：前缀段 0..k **不参与任何判定/重放**（状态已精确回到 k 前，
//!    它们的缓存输出原样保留）。
//!
//! 阶段一的"保底路径"（从全新引擎重建状态链）已删除——改正文/改宏体/改
//! catcode 都走同一条回滚路径，且用户注入的环境（预载 `.fmt`/自定义 VFS）
//! 不再随重建丢失。
//!
//! ## 链偏差与同步态：复用段时状态与缓存链保持一致
//!
//! 重放循环维护**链偏差**（[`ChainDelta`]）：活状态相对旧缓存链"当前位置"的
//! 偏差（值状态是否不同 + 哪些 eqtb 槽不同）。它在每次**执行段之后**重算一次
//! （基准 = 该段旧 `post`，缺省取下一段的 `pre`——段是背靠背执行的，二者都是
//! 旧链上"下一段执行前"的状态）；复用段不执行、偏差不变（有偏差时只复用状态
//! 中性段，零偏差时非中性段还原其 `post`，偏差集因此持续成立）。
//!
//! 回滚后的入口处偏差为零 = **同步态**：活状态与缓存链精确一致，执行必然复现
//! 同一输出，缓存段**免判定**直接复用（阶段一判定分支 1 的前提已由回滚保证）。
//! 复用不执行，状态须显式推进到该段执行后的样子：
//!
//! - 段状态中性（`pre ≈ post`）：活状态已是对的状态，不动；
//! - 段有状态副作用：还原其 `post` 检查点（"复用 = 用缓存输出 + 缓存后状态"
//!   替代执行，即 `Expand(source, snapshot) → (tokens, snapshot')` 的纯函数语义）。
//!
//! 偏差非零（编辑改了宏体/寄存器等）时缓存段逐个判定（见下）；判定发现活状态
//! 与 `pre(j)` 精确一致则转入同步态——编辑段之后状态重新收敛到缓存链。
//!
//! ## 有偏差时：缓存段 j 可跳过执行的判定（按序判定，任一不满足即重算）
//!
//! 1. 段边界干净（[`Expander::boundary_is_clean`]）；
//! 2. **值状态分支**：值状态无偏差（寄存器/参数/catcode/编码表/流不做槽级
//!    归因，有偏差即全局失效）；
//! 3. **依赖分支**："读依赖闭包 ∩ 偏差槽 = ∅"（输出不变）、"写集 ∩ 偏差槽 =
//!    ∅"（本应重写的值没有停留在被改值）、"j 状态中性"（跳过执行不改状态——
//!    有偏差时不能还原旧 `post`，那会把链上偏差的新值冲掉）。三者合取才健全。
//!
//! 副作用（`\write`/`\openout`/`\openin` 读位置）随段重算会重复提交，属副作用
//! 边界（RFC-3），不在本阶段逐位一致口径内。
//!
//! ## 阶段二边界（离 M5 目标 ≥100x 还差什么）
//!
//! - **链偏差全量扫描**：每次执行段后重算一次偏差（`value_state` 指纹 + eqtb
//!   全槽比较）。要把偏差集增量维护（赋值入口挂记录 / COW 影子表）才能免掉
//!   这一次扫描——那是仅剩的 O(全状态) 开销。
//! - **副作用边界**：`\write`/`\openout`/读流位置随段重算重复提交（RFC-3 把
//!   副作用移到 shipout 后才能干净重算）。
//! - **槽级归因**：寄存器/参数/catcode/编码表变化仍按"全局失效"处理
//!   （[`ValueState`] 整体比较），未归因到槽。
//! - **文本级切段**：跨段构造（未闭合 `\if`/`{`/`$`）的语义本就不可切段，
//!   需引擎级 checkpoint（在线切段）取代。

use crate::error::{Error, Result};
use crate::expand::Expander;
use crate::intern::InternTable;
use crate::sink::VecSink;
use crate::token::{Token, TokenKind};

use super::segment::segmentize;
use super::snapshot::{
    lex_deps, reads_touch_slots, slots_semantically_equal, state_matches, writes_touch_slots,
    ChainDelta, SegmentDeps, StateSnapshot,
};

/// 缓存的段：输出 + 失效判定所需信息。
#[derive(Debug, Clone)]
struct CachedSegment {
    /// 执行前快照。
    snapshot_pre: StateSnapshot,
    /// 执行后快照（同步态复用非中性段时还原它——"复用 = 缓存输出 + 缓存后状态"）。
    snapshot_post: StateSnapshot,
    /// 词法依赖（读/写 cs、`\csname` 标记、读依赖版本）。
    deps: SegmentDeps,
    /// 该段是否状态中性（执行前后状态语义一致 = 无净状态副作用）。
    /// 捕获时算一次，判定不再做全快照比较。
    state_neutral: bool,
    /// 该段输出 token（拼接即文档输出流）。
    output: Vec<Token>,
    /// 该段终端转录（`\message`/`\show`/`\write16`；诊断用，不参与逐位比较）。
    transcript: String,
    /// 段执行错误（`Err` 的字符串形态；出错即终止后续段——与全量 `run_source` 一致）。
    error: Option<String>,
}

/// 段执行结果（`run`/`edit` 返回值元素）。
#[derive(Debug, Clone)]
pub struct SegmentResult {
    /// 段下标。
    pub index: usize,
    /// 该段输出 token。
    pub output: Vec<Token>,
    /// 本轮是否来自缓存（未重算）。
    pub from_cache: bool,
    /// 段执行错误。
    pub error: Option<String>,
}

/// 缓存命中统计（基准与验证用）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SegmentStats {
    /// 本引擎生命周期内实际执行（重算）的段次数。
    pub executed: usize,
    /// 缓存复用（跳过执行）次数。
    pub reused: usize,
    /// 回滚后状态校验失败退回重建的次数（检查点不完整 = bug 信号；正常为 0）。
    pub restarts: usize,
}

/// 段级增量执行器。
#[derive(Debug)]
pub struct SegmentEngine {
    expander: Expander,
    segments: Vec<String>,
    cache: Vec<Option<CachedSegment>>,
    /// 最近一次重放中各段的缓存拒绝原因（`last_rejects`；判定时刻记录）。
    rejects: Vec<Option<&'static str>>,
    stats: SegmentStats,
}

impl Default for SegmentEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// 输出 token → 文本（"增量 == 全量逐位一致"的比较口径）。
///
/// 字符 token 还原字符；控制序列渲染为 `\名字`；宏参数 `#n`；组结束 `}`。
pub fn render_tokens(toks: &[Token], intern: &InternTable) -> String {
    let mut s = String::new();
    for t in toks {
        match t.kind() {
            TokenKind::Char => {
                s.push(t.charcode().and_then(char::from_u32).unwrap_or('\u{fffd}'));
            }
            TokenKind::ControlSeq => {
                s.push('\\');
                if let Some(csid) = t.csid() {
                    s.push_str(intern.name(csid));
                }
            }
            TokenKind::MacroParam => {
                s.push('#');
                if let Some(n) = t.param_number() {
                    s.push_str(&n.to_string());
                }
            }
            TokenKind::EndGroup => s.push('}'),
            TokenKind::EndTemplate => {}
        }
    }
    s
}

impl SegmentEngine {
    /// 新建引擎（字节码轨道，M1 内建原语已注册）。
    pub fn new() -> Self {
        Self::with_expander(Expander::new())
    }

    /// 以既有引擎为底（预载 `.fmt` 快照 / 自定义 VFS 的场景，M5 阶段二复用）。
    pub fn with_expander(expander: Expander) -> Self {
        Self {
            expander,
            segments: Vec::new(),
            cache: Vec::new(),
            rejects: Vec::new(),
            stats: SegmentStats::default(),
        }
    }

    /// 只读访问内部引擎（`.fmt` 导出 / 转录检查）。
    pub fn expander(&self) -> &Expander {
        &self.expander
    }

    /// 可变访问内部引擎（挂排版器 sink 等；挂上后段输出不再进 `VecSink`）。
    pub fn expander_mut(&mut self) -> &mut Expander {
        &mut self.expander
    }

    /// 当前段列表（切段结果）。
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// 缓存命中统计。
    pub fn stats(&self) -> SegmentStats {
        self.stats
    }

    /// 段是否带有可用缓存（已执行过且未被丢弃）。
    pub fn is_cached(&self, index: usize) -> bool {
        self.cache.get(index).is_some_and(Option::is_some)
    }

    /// 全量入口：切段并逐段执行（全新缓存）。
    pub fn run(&mut self, source: &str) -> Vec<SegmentResult> {
        self.segments = segmentize(source);
        self.cache = self.segments.iter().map(|_| None).collect();
        self.stats = SegmentStats::default();
        self.replay(0, false, None)
    }

    /// 编辑第 `index` 段并增量重算（单路径：回滚到段前 → 重放）。
    ///
    /// 回滚点之前的段缓存原样保留（`from_cache == true`）、不参与判定与重放；
    /// 被编辑段必算；其后各段按模块头注释的判定复用或重算。
    pub fn edit(&mut self, index: usize, new_source: &str) -> Result<Vec<SegmentResult>> {
        if index >= self.segments.len() {
            return Err(Error::invalid_input(format!(
                "段下标 {index} 越界：当前文档共 {} 段",
                self.segments.len()
            )));
        }
        // 回滚目标：被编辑段自己的 pre 快照。段无缓存（其执行在前一轮失败处
        // 之后，从未跑到）时退到最近的更早缓存段，重放从那里开始——中间段随
        // 重放重新执行。缓存为空只出现在"尚无任何段执行过"的退化情形（`run`
        // 至少会把段 0 连同其错误一起缓存），此时无状态可回滚，按畸形输入报错。
        let target = if self.cache[index].is_some() {
            index
        } else {
            (0..index)
                .rev()
                .find(|&i| self.cache[i].is_some())
                .ok_or_else(|| Error::invalid_input("无可回滚的段检查点：请先 run() 全文执行"))?
        };
        // 必须在清缓存**之前**取：回滚目标快照 + 编辑段旧 post（重放链偏差基准）。
        let restore = self.cache[target]
            .as_ref()
            .map(|c| c.snapshot_pre.clone())
            .expect("target 已筛选为有缓存段");
        let expect = self.cache[index]
            .as_ref()
            .map(|c| (index, c.snapshot_post.clone()));
        // 单路径核心：引擎整体还原到段前（阶段一在活状态已推进时从全新引擎
        // 重建状态链，用户注入的环境丢失——已删除）。
        self.expander.restore_checkpoint(&restore.cp);
        // 防御：还原后必须与快照精确一致。不一致 = 检查点字段不完整（引擎新增
        // 状态字段未进 checkpoint——bug 信号），此时回滚不可信，退回重建保证
        // 正确性（铁律优先）；正常路径不可达，`restarts` 恒为 0。
        if !state_matches(
            &self.expander.value_state(),
            self.expander.eqtb().slots(),
            &restore,
        ) {
            self.stats.restarts += 1;
            self.expander = Expander::new();
            self.segments[index] = new_source.to_owned();
            self.cache = self.segments.iter().map(|_| None).collect();
            return Ok(self.replay(0, false, None));
        }
        self.segments[index] = new_source.to_owned();
        self.cache[index] = None;
        // 入口即同步：活状态已被还原成 pre(target) 本身。
        Ok(self.replay(target, true, expect))
    }

    /// 汇总输出 token（已执行段按序拼接）。
    pub fn tokens(&self) -> Vec<Token> {
        let mut out = Vec::new();
        for cached in self.cache.iter().flatten() {
            out.extend_from_slice(&cached.output);
        }
        out
    }

    /// 输出 token 的文本形态（逐位一致比较口径）。
    pub fn text(&self) -> String {
        render_tokens(&self.tokens(), self.expander.intern())
    }

    /// 每段缓存状态报告（验证/基准输出用）。
    pub fn cache_report(&self) -> String {
        let mut lines = Vec::new();
        for (idx, cached) in self.cache.iter().enumerate() {
            match cached {
                Some(c) => {
                    let mut line = format!(
                        "段{idx}（{} 字节）：已缓存 | 读 {} 项、写 {} 项{} | 输出 {} token{}",
                        self.segments[idx].len(),
                        c.deps.read_cs.len(),
                        c.deps.write_cs.len(),
                        if c.deps.dynamic_cs {
                            "、含 \\csname"
                        } else {
                            ""
                        },
                        c.output.len(),
                        if c.error.is_some() {
                            "（执行出错）"
                        } else {
                            ""
                        }
                    );
                    // 段级转录（\message/\show/\write16）：诊断用，不参与逐位一致口径
                    if !c.transcript.is_empty() {
                        line.push_str(&format!(" | 转录 {:?}", c.transcript));
                    }
                    lines.push(line);
                }
                None => lines.push(format!("段{idx}：无缓存")),
            }
        }
        lines.join("\n")
    }

    /// 从 `from` 起重放：缓存仍有效的复用，其余重算；输入错误终止（同全量语义）。
    ///
    /// `entry_synced`：入口活状态是否精确等于 `pre(from)`（`edit` 回滚后为真；
    /// 全量 `run` 为假——无缓存链期望可言）。`expect` 是编辑段的 `(段下标, 旧
    /// post)`——该段执行后的链偏差基准；其余执行段用各自缓存里的旧 `post`，其
    /// 下一段的 `pre` 与之相同（段是背靠背执行的），故等价可用。
    ///
    /// 链偏差在**每次执行段之后**重算一次（[`ChainDelta::capture`]，一次全量
    /// 扫描），后续各缓存段的失效判定只查偏差集 ∩ 依赖（见模块头注释）。复用
    /// 段不执行、偏差集不变——非同步态只复用状态中性段（其执行本就不改状态，
    /// 旧链状态也不动，偏差集继续成立），同步态还原非中性段的 `post`。
    fn replay(
        &mut self,
        from: usize,
        entry_synced: bool,
        mut expect: Option<(usize, StateSnapshot)>,
    ) -> Vec<SegmentResult> {
        // 本轮重算过的段（`from_cache` 标记用；BTreeSet 保持结果组装顺序无关）
        let mut recomputed = std::collections::BTreeSet::<usize>::new();
        self.rejects = vec![None; self.segments.len()];
        let mut failed_at: Option<usize> = None;
        // 链偏差：活状态相对旧缓存链"当前位置"的状态。入口已回滚到 pre(from)
        // 时为零偏差；全量 `run` 无缓存可比（也无缓存段需要判定）。
        let mut delta = if entry_synced {
            Some(ChainDelta::empty())
        } else {
            None
        };
        for idx in from..self.segments.len() {
            if self.cache[idx].is_some() {
                let reason = match delta.as_ref() {
                    // 无偏差 + 干净边界 → 状态与缓存链一致，执行必然复现同一输出，
                    // 免判定（判定分支 1 的前提由回滚/状态推进保证）
                    Some(d) if d.is_empty() && self.expander.boundary_is_clean() => None,
                    // 偏差集上的依赖判定（脏边界在其中单独成原因）
                    Some(d) => self.cache_reject_reason_inner(idx, d),
                    // 无基准（该段前全无旧缓存）→ 保守重算
                    None => Some("无链偏差基准（此前各段无旧缓存，保守重算）"),
                };
                self.rejects[idx] = reason;
                if reason.is_none() {
                    self.stats.reused += 1;
                    let cached = self.cache[idx].as_ref().expect("上方已判 is_some");
                    if !cached.state_neutral {
                        // 复用 = 不执行 → 状态须推进到该段执行后的状态。只有零
                        // 偏差（同步态）会走到这里：缓存链就是正确状态，非中性
                        // 段还原其 post（"复用 = 缓存输出 + 缓存后状态"）。有偏差
                        // 时的非中性段已被判定拒掉——旧 post 会把链上偏差的新值
                        // 冲掉，不能整体还原。
                        let post = cached.snapshot_post.clone();
                        self.expander.restore_checkpoint(&post.cp);
                    }
                    if cached.error.is_some() {
                        // 复用的出错段同样终止后续段（全量语义：出错即停）
                        failed_at = Some(idx);
                        break;
                    }
                    continue;
                }
            }
            // 执行该段，然后重算链偏差（一次全量扫描；基准 = 该段旧 post，缺省
            // 用下一段的 pre——二者都是旧链上"下一段执行前"的状态）
            let reference: Option<StateSnapshot> =
                if expect.as_ref().is_some_and(|(at, _)| *at == idx) {
                    expect.take().map(|(_, p)| p)
                } else {
                    self.cache[idx].as_ref().map(|c| c.snapshot_post.clone())
                }
                .or_else(|| {
                    self.cache
                        .get(idx + 1)
                        .and_then(|c| c.as_ref())
                        .map(|c| c.snapshot_pre.clone())
                });
            let cached = self.execute_segment(idx);
            let failed = cached.error.is_some();
            self.stats.executed += 1;
            recomputed.insert(idx);
            self.cache[idx] = Some(cached);
            delta = reference
                .as_ref()
                .map(|r| ChainDelta::capture(&self.expander, r));
            if failed {
                failed_at = Some(idx);
                break;
            }
        }
        // 结果覆盖 0..stop：stop 之前都已产出（重算或缓存），失败段之后不再有输出
        let stop = failed_at.map_or(self.segments.len(), |i| i + 1);
        (0..stop)
            .map(|idx| match self.cache[idx].as_ref() {
                Some(c) => SegmentResult {
                    index: idx,
                    output: c.output.clone(),
                    from_cache: !recomputed.contains(&idx),
                    error: c.error.clone(),
                },
                None => SegmentResult {
                    index: idx,
                    output: Vec::new(),
                    from_cache: false,
                    error: None,
                },
            })
            .collect()
    }

    /// 执行第 `idx` 段：先记前/后快照与依赖，再喂入运行并取回段输出。
    fn execute_segment(&mut self, idx: usize) -> CachedSegment {
        let source = self.segments[idx].clone();
        let deps = lex_deps(&source).attach_versions(self.expander.intern(), self.expander.eqtb());
        let snapshot_pre = StateSnapshot::capture(&self.expander);
        self.expander.set_sink(Box::new(VecSink::default()));
        self.expander.feed_source(source);
        let result = self.expander.run();
        let snapshot_post = StateSnapshot::capture(&self.expander);
        let sink = self.expander.take_sink();
        // 状态中性 = 执行前后状态语义一致（捕获时算一次，判定只查布尔）
        let state_neutral = slots_semantically_equal(snapshot_pre.eqtb(), snapshot_post.eqtb())
            && snapshot_pre.value() == snapshot_post.value();
        // 先取转录再消费 token（take_tokens 按值取走 Box）
        let transcript = sink.transcript().to_owned();
        CachedSegment {
            snapshot_pre,
            snapshot_post,
            deps,
            state_neutral,
            output: sink.take_tokens().unwrap_or_default(),
            transcript,
            error: result.err().map(|e| e.to_string()),
        }
    }

    /// 最近一次 `run`/`edit` 重放中各段的缓存拒绝原因（判定时刻记录；
    /// `None` = 复用了缓存，或该段本就无缓存（首次执行/被编辑））。
    pub fn last_rejects(&self) -> &[Option<&'static str>] {
        &self.rejects
    }

    /// 缓存段 `idx` 在链偏差 `delta` 下的复用判定（`None` = 可复用）。
    ///
    /// **就地**判定：偏差集描述"当前活状态 vs 旧缓存链"的偏差，只在重放到该段
    /// 前一刻有效；离线事后询问会拿到"状态已推进"的假原因——查最近一次重放的
    /// 判定用 [`Self::last_rejects`]。
    fn cache_reject_reason_inner(&self, idx: usize, delta: &ChainDelta) -> Option<&'static str> {
        let cached = self.cache[idx].as_ref()?;
        // 段边界被跨段构造污染：语义无法由"段前状态 + 段源码"还原 → 一律不复用
        // （控制状态不进偏差集，"边界干净"是它唯一可判的等价口径）
        if !self.expander.boundary_is_clean() {
            return Some("段边界不干净（跨段构造：未闭合组/条件/数学/参数扫描）");
        }
        // 值状态偏差（寄存器/参数/catcode/编码表/流）：不做槽级归因，全局失效
        if delta.value_differs {
            return Some("值状态变化（寄存器/参数/catcode/编码表/流，未做槽级归因）");
        }
        if cached.deps.dynamic_cs {
            return Some("段含 \\csname（动态 cs 名，保守失效）");
        }
        let cur_eqtb = self.expander.eqtb().slots();
        // 读依赖闭包触及偏差槽 → 输出可能变，必须重算
        if reads_touch_slots(self.expander.intern(), cur_eqtb, &cached.deps, &delta.slots) {
            return Some("读依赖闭包触及变化槽（该段引用的宏被改动）");
        }
        // 写集触及偏差槽 → 跳过会让"本应重写的值"停留在被改值
        if writes_touch_slots(self.expander.intern(), &cached.deps.write_cs, &delta.slots) {
            return Some("写集触及变化槽（跳过会让本应重写的值停留在被改值）");
        }
        // 状态中性：有偏差时复用 = 状态原地不动（不能还原旧 post——会把链上
        // 偏差的新值冲掉），只有"执行也不改变状态"时才健全
        if !cached.state_neutral {
            return Some("段有状态副作用（有链偏差时跳过不还原 post，不能跳过执行）");
        }
        None
    }
}
