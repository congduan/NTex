// ---------- 方法分片：speculate.rs（M6 段组投机并行，RFC-5） ----------
//
// # 段组投机并行（speculative segment parallelism）
//
// TeX 执行是强顺序的：展开推进 eqtb，断页依赖前文累积高度。直接"多线程跑 TeX"
// 不可行。本文件利用 M5 已经证明的事实——文档可切成**段**（`segmentize`，段边界
// 垂直模式、组/条件/数学闭合干净），且多数段是**状态中性**的（执行前后 expand
// 值状态与 eqtb 语义不变、layout 副作用字段不变），纯文本段落即如此——做段组
// 投机：窗口内各段从**同一个**窗口起点快照 `S0` 独立执行，产出段贡献；装配器
// （主线程，唯一权威状态）按段序校验、采纳、注入主列表、断页、执行 `\output`
// 例程、shipout。
//
// ## 为什么窗口内各段可以共用 S0
//
// 顺序依赖只有三类（RFC-5 §2）：
// - **A 展开状态**：neutral 段不改状态 ⇒ 窗口内各段的输入状态都等于 S0。
// - **B 排版副作用**（字体表 / 参数镜像 / marks / 盒子寄存器 / 断字表）：
//   同上，由双重 neutral 判定（`snapshot_states_equal` + `SideEffects::matches`）
//   保证；不成立即作废该段（主线程顺序执行）。
// - **C 装配上下文**（主列表尾部 → 段首行间胶水宽度；页累积高度）：唯一的
//   真链式依赖。前者用「段首胶水延后烘焙」消除（[`SpecMode`]：投机执行跳过
//   段首胶水，采纳时装配器按**活**上下文补插），后者只影响页面构建器，
//   本来就只能在装配时处理（页面构建器不属于 SideEffects，投机份随结果丢弃）。
//
// ## 窗口协议（RFC-5 §3.1）
//
// 1. 开窗条件：当前活状态干净（[`NodeBuilder::is_shaped`]）、无 `\output` 例程、
//    投机开关打开；
// 2. 预过滤：段的词法依赖命中 [`SPEC_BLACKLIST`]（读装配状态 / 外部输入）或含
//    `\csname`（动态 cs 名）→ 不进窗口，主线程顺序执行；
// 3. 发射：窗口内各段从 S0 投机（[`speculate_segment`]）；
// 4. 采纳：逐段校验 `adoptable()`——遇不可采纳段，窗口终止于此段（作废其投机
//    结果，主线程顺序执行，即现路径原样），随后从下一段开新窗口。
//
// ## 副作用隔离（RFC-3）
//
// 投机执行器是 [`Expander::fork_isolated`] 派生的独立引擎，装两件哨兵/副本：
// - **VFS 哨兵**（[`SpeculationVfs`]）：任何读写都置 taint 且不接触真实后端
//   ——投机段里 `\input`/`\openin`/`\read` 一律作废（外部输入不是纯状态函数）；
// - **私有字体表副本**：投机段的 `\font` 装载 / CJK 回落写副本，主表不受污染；
//   装载同时反映到 `font_loads`/`font_cs_names` → 双重 neutral 失败 → 作废。
//
// ## 与 M5 缓存的衔接
//
// 采纳段的缓存（[`SegmentCache`]）与顺序 `execute_segment` **同构**：节点流是
// 「补过段首胶水的完整流」、`entry_ctx` 是采纳时的活上下文（= 顺序执行同一位置
// 的上下文）、`record_exp`/`record_post` 取采纳时的活状态（段状态中性 ⇒ 二者
// 语义相同）。因此后续 `edit` 的复用判定（M5 路径）对新写的缓存无需任何改动。
//
// ## 未实现（M6-3~M6-5）
//
// 本文件目前是 **M6-1 + M6-2**：投机执行原语 + 窗口调度器（单线程依次投机，
// 每段仍从同一 S0 出发——与真并行行为等价，故同一套一致性测试即可覆盖）。
// 真并行（rayon 工作池 + 结果槽）需要 `Expander`/`NodeBuilder`/`Fonts` 的
// `Send` 化（`Rc` → `Arc`、trait 加 `Send` 约束），是 M6-3 的范围。

/// 投机窗口的静态黑名单（段的词法读依赖命中即不投机，RFC-5 §3.5 第 1 条）。
///
/// 判据：**neutral 判定覆盖不到**的读取面——这些原语读的是"窗口内前段造成的
/// 装配上下文"（主列表尾部 / 页推进 / marks 轮转），而投机 builder 的上下文是
/// 窗口起点 S0。运行时也有 taint 兜底（`NodeBuilder::mark_spec_taint`），本表
/// 只是**预过滤**（省下一次无用的投机执行）。
///
/// **维护纪律**：新增"读装配状态"的原语时必须同步登记本表（AGENTS.md 新增原语
/// 六步链路的第 6 步），并在 `spec_tests` 里补定向用例。
const SPEC_BLACKLIST: &[&str] = &[
    // 主列表尾部读取（窗口内前段的产出不在投机 builder 里）
    "lastbox",
    "unskip",
    "unpenalty",
    "unkern",
    "lastnodetype",
    "lastpenalty",
    "lastskip",
    "lastkern",
    // 页构建器累计量（投机 builder 的页面从 S0 克隆，不含窗口内前段推进）
    "insertpenalties",
    "pagegoal",
    "pagetotal",
    "prevdepth",
    // marks 族（随断页轮转演化）
    "topmark",
    "firstmark",
    "botmark",
    "splitfirstmark",
    "splittopmark",
    "splitbotmark",
    // 外部输入与全局设施（RFC-3：投机执行器装哨兵，命中即 taint）
    "input",
    "openin",
    "read",
    "font",
    "patterns",
    "hyphenation",
];

/// 投机执行的 VFS 哨兵：任何文件访问都标记 taint（段作废），且**不接触**真实
/// 后端（读写都作用在哨兵自身，无外部副作用）。
///
/// 为什么不是"fork 一个真实 VFS"：读文件是**隐式输入依赖**（内容取决于宿主
/// 文件系统，不是纯状态函数），neutral 判定覆盖不到；而写侧必须隔离（投机段
/// 的 `\write`/`\openout` 若落到真实后端，作废时就无法撤销）。哨兵两条都满足。
#[derive(Debug)]
struct SpeculationVfs(std::rc::Rc<std::cell::Cell<bool>>);

impl ntex_io::Vfs for SpeculationVfs {
    fn read(&mut self, _path: &str) -> std::io::Result<Option<Vec<u8>>> {
        self.0.set(true);
        Ok(None)
    }

    fn write(&mut self, _path: &str, _bytes: &[u8]) -> std::io::Result<()> {
        self.0.set(true);
        Ok(())
    }

    fn append(&mut self, _path: &str, _bytes: &[u8]) -> std::io::Result<()> {
        self.0.set(true);
        Ok(())
    }

    fn create_dir_all(&mut self, _path: &str) -> std::io::Result<()> {
        self.0.set(true);
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// 单段投机执行结果（RFC-5 §3.2）。
#[derive(Debug)]
struct SegSpecResult {
    /// 该段进入主列表的节点流（**段首胶水未烘焙**，见 `first_glue_pending`）。
    contrib: Vec<Node>,
    /// 段缓冲转录（`\message`/`\write16`/Overfull 警告…）；采纳时按段序 flush。
    transcript: String,
    /// 双重 neutral：expand 状态语义一致（A）+ 排版副作用字段一致（B）。
    neutral: bool,
    /// 触碰装配状态读取 / 外部输入（[`NodeBuilder::mark_spec_taint`] 或 VFS 哨兵）。
    taint: bool,
    /// 段首胶水待补插：`Some((落点下标, 首个垂直盒高度))`（见 [`SpecMode`]）。
    first_glue_pending: Option<(usize, i64)>,
    /// 段执行错误（`Err` 的字符串形态）：作废该段，主线程顺序执行复现错误路径。
    error: Option<String>,
    /// 段进入时（窗口起点 S0）是否干净段边界——决定该段可否建立缓存。
    shaped_entry: bool,
    /// 段退出时是否干净段边界——同上；`false` 同时意味着窗口必须终止
    /// （该段留下的开放段落在后续段闭合，其节点流跨段）。
    shaped_exit: bool,
}

impl SegSpecResult {
    /// 采纳条件（RFC-5 §3.1 窗口终止条件的反命题）。
    fn adoptable(&self) -> bool {
        self.error.is_none() && self.neutral && !self.taint
    }
}

/// 投机执行单段（RFC-5 §3.2）：从 `S0`（主实例的检查点 + builder 快照）派生
/// 独立执行器执行 `source`，产出段贡献与判定，**不触碰主实例任何状态**。
///
/// `expander` 只读借出——`fork_isolated` 克隆其状态；投机执行器随后装上自己的
/// 哨兵 VFS / 私有字体表 / builder 克隆，主实例的 sink 与字体表皆不受影响。
fn speculate_segment(
    expander: &Expander,
    s0_builder: &NodeBuilder,
    s0_state: &StateSnapshot,
    s0_side: &SideEffects,
    main_fonts: &Fonts,
    source: &str,
) -> SegSpecResult {
    let taint_flag = std::rc::Rc::new(std::cell::Cell::new(false));
    // 私有字体表副本：`\font` 装载与 CJK 回落写副本（主表零污染）；度量查询
    // 读副本（内容 = S0 时刻的主表，查询结果与顺序执行一致）。
    let (spec_fonts, loader): (Fonts, Box<dyn FontLoader>) = match main_fonts {
        Fonts::Tfm(table) => {
            let copy = Rc::new(RefCell::new(table.borrow().clone()));
            (
                Fonts::Tfm(copy.clone()),
                Box::new(TfmLoader { table: copy }),
            )
        }
        Fonts::Fn { metrics, space } => (
            Fonts::Fn {
                metrics: *metrics,
                space: *space,
            },
            Box::new(ntex_core::font::NoFontLoader),
        ),
    };
    let mut e = expander.fork_isolated(Box::new(SpeculationVfs(taint_flag.clone())), loader);
    let mut b = s0_builder.clone();
    b.fonts = spec_fonts;
    b.spec_mode = Some(SpecMode::default());
    b.spec_taint.set(false);
    b.record_main = Some(Vec::new());
    let transcript_len = b.io_state.transcript.len();
    let shaped_entry = b.is_shaped();
    e.set_sink(Box::new(b));
    e.feed_source(source);
    let result = e.run();
    // 段后状态（双重 neutral 的 A 半边）。
    let post_state = StateSnapshot::capture(&e);
    let mut boxed = e.take_sink();
    let b = boxed
        .as_any_mut()
        .downcast_mut::<NodeBuilder>()
        .expect("投机执行器安装的是 NodeBuilder");
    SegSpecResult {
        contrib: b.record_main.take().unwrap_or_default(),
        transcript: b.io_state.transcript[transcript_len..].to_owned(),
        neutral: snapshot_states_equal(s0_state, &post_state) && s0_side.matches(b),
        taint: taint_flag.get() || b.spec_taint.get(),
        first_glue_pending: b.spec_mode.and_then(|s| s.pending_first_glue),
        error: result.err().map(|e| e.to_string()),
        shaped_entry,
        shaped_exit: b.is_shaped(),
    }
}

impl IncrementalTypesetter {
    /// 投机窗口的发射深度 `W`（段数；RFC-5 §3.1：默认 `2 × 线程数`）。
    fn spec_window_width(&self) -> usize {
        let threads = std::env::var("NTEX_THREADS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1)
            .max(1);
        2 * threads
    }

    /// 开窗条件（RFC-5 §3.1）：投机开关打开、无 `\output` 例程、活状态干净。
    ///
    /// `\output` 例程一旦观测到就关闭投机（M5 同口径的保守处置）：例程在断页
    /// 边界由 core 注入执行，而采纳路径是主线程直接 `append` 节点（不经 token
    /// 循环），例程要等到 `finish_doc` 才补跑——时机差异是否影响逐位一致
    /// **未经验证**（LaTeX 文档必然定义 `\output`，故 LaTeX 路径当前完全不走
    /// 投机；见 [`IncrementalStats::adopted`] 与 plan.md M6 的待办）。
    fn spec_window_open(&mut self) -> bool {
        if !self.spec_enabled || self.output_routine {
            return false;
        }
        // CJK 回落档（wasm/工作台）：回落装载路径写字体表，虽已隔离到私有副本，
        // 但该档另有宿主侧字体登记，投机口径未经验证——保守关闭。
        if self.fallback_font.is_some() {
            return false;
        }
        sink_builder(&mut self.expander).is_shaped()
    }

    /// 该段能否进窗口（静态预过滤，见 [`SPEC_BLACKLIST`]）。
    fn spec_candidate(&self, j: usize) -> bool {
        let deps = lex_deps(&self.segments[j]);
        !deps.dynamic_cs
            && !deps
                .read_cs
                .iter()
                .any(|n| SPEC_BLACKLIST.contains(&n.as_str()))
    }

    /// 全量编译的投机窗口路径（RFC-5 §3.1）。
    ///
    /// 与顺序路径（`compile` 的 `for j in 0..n { execute_segment(j) }`）输出逐位
    /// 一致，正确性论证见 RFC-5 §4（归纳：采纳段的主线程状态 == 顺序执行同点
    /// 状态；非采纳段即顺序路径本身）。
    ///
    /// M6-1/M6-2 阶段：窗口内各段**依次**投机（单线程模拟），每段仍从同一 S0
    /// 出发——与真并行的行为完全一致（协议不涉及线程时序），是 M6-3 接入 rayon
    /// 前的可测试形态。
    fn run_segments_speculative(&mut self) -> Result<()> {
        let n = self.segments.len();
        let width = self.spec_window_width();
        let mut j = 0usize;
        while j < n {
            if !self.spec_window_open() || !self.spec_candidate(j) {
                // 顺序执行该段（现路径原样）
                if let Err(e) = self.execute_segment(j) {
                    self.invalidate_rollback_after(j);
                    return Err(e);
                }
                j += 1;
                continue;
            }
            // ---- 发射：窗口内各段从同一 S0 投机 ----
            let s0_state = StateSnapshot::capture(&self.expander);
            let s0_side = sink_builder(&mut self.expander).side_effects();
            let s0_builder = sink_builder(&mut self.expander).clone();
            let mut batch: Vec<SegSpecResult> = Vec::new();
            for i in j..n.min(j + width) {
                if !self.spec_candidate(i) {
                    break;
                }
                let r = speculate_segment(
                    &self.expander,
                    &s0_builder,
                    &s0_state,
                    &s0_side,
                    &self.fonts,
                    &self.segments[i],
                );
                let stop = !r.adoptable() || !r.shaped_exit;
                batch.push(r);
                if stop {
                    break;
                }
            }
            // ---- 采纳：按段序 ----
            let mut i = j;
            for r in batch {
                if !r.adoptable() {
                    // 窗口终止于此段：作废投机结果，主线程顺序执行（现路径原样）
                    self.stats.spec_rejected += 1;
                    if let Err(e) = self.execute_segment(i) {
                        self.invalidate_rollback_after(i);
                        return Err(e);
                    }
                    i += 1;
                    break;
                }
                let source = self.segments[i].clone();
                self.adopt_spec_result(i, &source, r)?;
                i += 1;
            }
            j = i;
        }
        Ok(())
    }

    /// 采纳一段投机结果（RFC-5 §3.3/§3.4）：按段序 flush 转录 → 补段首胶水 →
    /// 注入 contrib → 写缓存与段后边界。
    ///
    /// 全部在主线程权威状态上进行，与顺序 `execute_segment` 同一代码路径
    /// （`NodeBuilder::append`——断页、marks 轮转、`\tracingpages` 都在里面）。
    fn adopt_spec_result(&mut self, j: usize, source: &str, r: SegSpecResult) -> Result<()> {
        // 1) 转录 flush（RFC-5 §3.4：输出只在被采纳后按段序进入全局转录，
        //    与顺序执行的 write16 逐次累积逐字同源）。
        if !r.transcript.is_empty() {
            sink_builder(&mut self.expander)
                .io_state
                .transcript
                .push_str(&r.transcript);
        }
        // 2) 补段首胶水 + 注入 contrib；缓存节点流 = 补胶水后的完整流（与顺序
        //    执行录制的流同构，M5 复用路径无需感知投机来源）。
        //
        //    补插位置：段贡献流里**首个垂直盒之前**（`at`）——不是整段之前。
        //    段内更早产出的 `\parskip`/`\vskip` 等节点顺序执行时就在胶水前面
        //    （现场：`[fil] [baselineskip glue] [行盒]`；插到最前会把两胶水换位）。
        let entry_ctx = main_above_depth(sink_builder(&mut self.expander));
        let mut full: Vec<Node> = r.contrib;
        if let Some((at, height)) = r.first_glue_pending {
            let b = sink_builder(&mut self.expander);
            if let Some(prev_depth) = b.previous_interline_depth() {
                let glue = b.interline_glue_node(height, prev_depth);
                full.insert(at.min(full.len()), glue);
            }
        }
        {
            let b = sink_builder(&mut self.expander);
            for node in &full {
                b.append(node.clone());
            }
        }
        if sink_builder(&mut self.expander).page_state.output_defined {
            self.output_routine = true;
        }
        // 3) 段缓存（与顺序 execute_segment 同构）：段状态中性 ⇒ record_exp ==
        //    record_post == 采纳时的活状态（执行不改状态）；entry_ctx 取注入前
        //    的活上下文（= 顺序执行同一位置的上下文）。
        if r.shaped_entry && r.shaped_exit {
            let state = StateSnapshot::capture(&self.expander);
            let deps =
                lex_deps(source).attach_versions(self.expander.intern(), self.expander.eqtb());
            self.cache[j] = Some(SegmentCache {
                contrib: full,
                entry_ctx,
                record_exp: state.clone(),
                record_post: state,
                deps,
                state_neutral: true,
            });
        } else {
            // 跨段构造（进入或退出时不干净）：不建立缓存（顺序路径同款判定）。
            self.cache[j] = None;
        }
        self.capture_boundary(j + 1, false);
        self.stats.executed += 1;
        self.stats.adopted += 1;
        Ok(())
    }
}
