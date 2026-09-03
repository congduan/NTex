//! 段级执行器（M5 阶段一，plan.md §7）：切段 → 带快照执行 → 缓存 → 编辑重算。
//!
//! 执行模型：段在**同一个** [`Expander`] 上按序 `feed_source + run`（顺序重放），
//! 每段记录**执行前/执行后**两份轻量快照 + 词法依赖 + 输出 token 并缓存。
//!
//! ## `edit` 的两条路径（正确性铁律：增量 == 全量逐位一致）
//!
//! 编辑第 k 段时，**引擎的活状态**是上一轮全文跑完的状态，而重算第 k 段需要
//! 的是"第 k 段执行前"的状态。阶段一没有可回滚的完整状态快照（寄存器文件
//! 32768×5 槽，逐段存 ~3MB 不可行；见 [`StateSnapshot`] 注记），因此：
//!
//! - **廉价路径**（活状态仍等于 `pre(k)` 时）：前缀段缓存原样保留，从 k 起重放。
//!   纯文本/无状态副作用的文档（改错字、改措辞）命中此路径。
//! - **保底路径**（状态已推进）：从全新引擎重建状态链（全量重放），未编辑段
//!   的缓存按下方判定复用。阶段二以"可回滚快照"取代，才能跳过有副作用的前缀段。
//!
//! ## 缓存段 j 可跳过执行（复用输出）的判定（按序判定，任一不满足即重算）
//!
//! 1. 段边界干净（[`Expander::boundary_is_clean`]）；
//! 2. **状态分支**：当前状态与 `pre(j)` 语义一致（值状态精确相等 + eqtb 逐槽
//!    语义相等，宏槽比内容不比版本号）——执行 j 必然复现同一输出；
//! 3. **依赖分支**（2 不成立时）：值状态未变，且"读依赖闭包 ∩ 变化槽 = ∅"、
//!    "写集 ∩ 变化槽 = ∅"，**且** j 状态中性（`pre(j) ≈ post(j)`）。三者合取
//!    才健全：读闭包保证输出不变；写集保证"本应重写的值"没有停留在被改值；
//!    状态中性保证跳过执行不改变状态（阶段一无回滚，跳过 = 状态原地不动）。
//!
//! 副作用（`\write`/`\openout`/`\openin` 读位置）随段重算会重复提交，属阶段二
//! 副作用边界（RFC-3），不在本阶段逐位一致口径内。

use crate::error::{Error, Result};
use crate::expand::Expander;
use crate::intern::InternTable;
use crate::sink::VecSink;
use crate::token::{Token, TokenKind};

use super::segment::segmentize;
use super::snapshot::{
    lex_deps, names_changed_slot, reads_changed_slot, snapshot_semantically_equal, state_matches,
    SegmentDeps, StateSnapshot,
};

/// 缓存的段：输出 + 失效判定所需信息。
#[derive(Debug, Clone)]
struct CachedSegment {
    /// 执行前快照。
    snapshot_pre: StateSnapshot,
    /// 执行后快照（"状态中性"判定用：`pre ≈ post` 即该段无净状态副作用）。
    snapshot_post: StateSnapshot,
    /// 词法依赖（读/写 cs、`\csname` 标记、读依赖版本）。
    deps: SegmentDeps,
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
    /// `edit` 走保底路径（从全新引擎重建状态链）的次数。
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
        self.replay(0)
    }

    /// 编辑第 `index` 段并增量重算。
    ///
    /// 失效传播：该段必算；其余段按模块头注释的判定复用或重算。走廉价路径时
    /// 前缀段缓存原样保留（`from_cache == true`），走保底路径时从全新引擎重建
    /// 状态链（阶段二以可回滚快照取代）。
    pub fn edit(&mut self, index: usize, new_source: &str) -> Result<Vec<SegmentResult>> {
        if index >= self.segments.len() {
            return Err(Error::invalid_input(format!(
                "段下标 {index} 越界：当前文档共 {} 段",
                self.segments.len()
            )));
        }
        // 廉价路径的前提：引擎活状态仍等于该段执行前的快照（跨段构造会把边界
        // 弄脏，此时无从谈起）。必须在清缓存**之前**取 `pre(index)`。
        let pre_k = self.cache[index].as_ref().map(|c| c.snapshot_pre.clone());
        let cheap = pre_k.is_some()
            && self.expander.boundary_is_clean()
            && pre_k.is_some_and(|pre| {
                let value = self.expander.value_state();
                let eqtb = self.expander.eqtb().slots();
                state_matches(&value, eqtb, &pre)
            });
        self.segments[index] = new_source.to_owned();
        self.cache[index] = None;
        if cheap {
            Ok(self.replay(index))
        } else {
            self.stats.restarts += 1;
            // 保底：无可回滚快照，从全新引擎重建状态链（用户注入的环境随新引擎
            // 丢失——阶段二改从 `.fmt` 基线快照重建）
            self.expander = Expander::new();
            Ok(self.replay(0))
        }
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
    fn replay(&mut self, from: usize) -> Vec<SegmentResult> {
        // 本轮重算过的段（`from_cache` 标记用；BTreeMap 保持结果组装顺序无关）
        let mut recomputed = std::collections::BTreeSet::new();
        self.rejects = vec![None; self.segments.len()];
        let mut failed_at: Option<usize> = None;
        for idx in from..self.segments.len() {
            if self.cache[idx].is_some() {
                // 判定必须在"该段即将执行前"的状态上做（结果才是当时的原因）
                let reason = self.cache_reject_reason_inner(idx);
                self.rejects[idx] = reason;
                if reason.is_none() {
                    self.stats.reused += 1;
                    continue;
                }
            }
            let cached = self.execute_segment(idx);
            let failed = cached.error.is_some();
            self.stats.executed += 1;
            recomputed.insert(idx);
            self.cache[idx] = Some(cached);
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
        // 先取转录再消费 token（take_tokens 按值取走 Box）
        let transcript = sink.transcript().to_owned();
        CachedSegment {
            snapshot_pre,
            snapshot_post,
            deps,
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

    /// 缓存段 `idx` 不可复用的原因（`None` = 可复用）。
    ///
    /// **就地**判定：比较"当前引擎状态"与缓存快照，只在重放到该段前一刻询问
    /// 才有意义；离线事后询问会拿到"状态已推进"的假原因——查最近一次重放的
    /// 判定用 [`Self::last_rejects`]。
    fn cache_reject_reason_inner(&self, idx: usize) -> Option<&'static str> {
        let cached = self.cache[idx].as_ref()?;
        // 段边界被跨段构造污染：语义无法由"段前状态 + 段源码"还原 → 一律不复用
        if !self.expander.boundary_is_clean() {
            return Some("段边界不干净（跨段构造：未闭合组/条件/数学/参数扫描）");
        }
        let cur_value = self.expander.value_state();
        let cur_eqtb = self.expander.eqtb().slots();
        // 分支 1：状态与缓存时语义一致 → 执行必然复现同一输出
        if state_matches(&cur_value, cur_eqtb, &cached.snapshot_pre) {
            return None;
        }
        // 分支 2：依赖判定。前置条件——非 eqtb 值状态（寄存器/参数/catcode/编码
        // 表/`\everypar`/流）未变，这些量阶段一不做槽级归因。
        if cur_value != cached.snapshot_pre.value {
            return Some("值状态变化（寄存器/参数/catcode/编码表/流，阶段一全局失效）");
        }
        if cached.deps.dynamic_cs {
            return Some("段含 \\csname（动态 cs 名，保守失效）");
        }
        if reads_changed_slot(
            self.expander.intern(),
            cur_eqtb,
            &cached.snapshot_pre.eqtb,
            &cached.deps,
        ) {
            return Some("读依赖闭包触及变化槽（该段引用的宏被改动）");
        }
        if names_changed_slot(
            self.expander.intern(),
            cur_eqtb,
            &cached.snapshot_pre.eqtb,
            &cached.deps.write_cs,
        ) {
            return Some("写集触及变化槽（跳过会让本应重写的值停留在被改值）");
        }
        // 状态中性：跳过执行 = 状态原地不动，只有"执行也不改变状态"时才健全
        // （阶段一无回滚；阶段二可回滚快照落地后此条件可放宽为"恢复 post 快照"）
        if !snapshot_semantically_equal(&cached.snapshot_pre, &cached.snapshot_post) {
            return Some("段有状态副作用（阶段一无状态回滚，不能跳过执行）");
        }
        None
    }
}
