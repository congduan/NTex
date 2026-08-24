//! 基准集合：plan.md M0 基准集的四个基准。
//!
//! 每个基准面向 [`EngineDriver`] 编写：驱动未实现时返回 `Skipped`（不算失败），
//! 引擎本体就绪后无需改动基准定义即可接入。

use anyhow::{bail, Result};

use crate::measure::{measure, Stats};
use ntex_test_support::{DriverStatus, EngineDriver, InteractionMode, OutputFormat, RunRequest};

/// 基准参数（由 CLI 注入）。
#[derive(Debug, Clone, Copy)]
pub struct BenchOptions {
    pub warmup: usize,
    pub iterations: usize,
}

/// 基准结果。
#[derive(Debug)]
pub enum BenchResult {
    /// 测得耗时。
    Measured { stats: Stats, note: String },
    /// 无法执行（驱动未实现等），不算失败。
    Skipped { reason: String },
}

/// 一个基准的定义。
pub trait Benchmark {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn run(&self, driver: &dyn EngineDriver, opts: &BenchOptions) -> Result<BenchResult>;
}

// ---------- 1. latex.ltx 全量加载 ----------

/// 空 article 文档的全量初始化（隐含加载 latex.ltx）。
pub struct LatexFmtLoad;

impl Benchmark for LatexFmtLoad {
    fn name(&self) -> &'static str {
        "latex.fmt-load"
    }
    fn description(&self) -> &'static str {
        "加载 latex.ltx（空 article 文档全量初始化）"
    }
    fn run(&self, driver: &dyn EngineDriver, opts: &BenchOptions) -> Result<BenchResult> {
        let tex = "\\documentclass{article}\n\\begin{document}\n\\end{document}\n";
        run_measured(
            driver,
            opts,
            "latex-load",
            tex,
            OutputFormat::Pdf,
            String::new(),
        )
    }
}

// ---------- 2. 300 页文档冷编 ----------

/// 生成 300 页文档样张（章节 + 段落）。
///
/// 样张为英文（pdflatex 参考驱动无 CJK 宏包会致命报错）；中文样张待
/// xelatex 参考驱动接入后单独补充（CJK 整形属于字体层，另设基准）。
pub struct Doc300;

impl Benchmark for Doc300 {
    fn name(&self) -> &'static str {
        "doc-300"
    }
    fn description(&self) -> &'static str {
        "300 页文档冷编（英文样张）"
    }
    fn run(&self, driver: &dyn EngineDriver, opts: &BenchOptions) -> Result<BenchResult> {
        const PAGES: usize = 300;
        let mut tex = String::from("\\documentclass{article}\n\\begin{document}\n");
        for i in 0..PAGES {
            tex.push_str(&format!("\\section{{Section {}}}\n", i + 1));
            // 约 20 行文本（含强调与行内公式），接近一页
            for j in 0..20 {
                tex.push_str(&format!(
                    "Paragraph {i}-{j}: this is a sample sentence with \\textit{{emphasis}} and math $a^2 + b^2 = c^2$.\n\n"
                ));
            }
        }
        tex.push_str("\\end{document}\n");
        run_measured(
            driver,
            opts,
            "doc-300",
            &tex,
            OutputFormat::Pdf,
            String::new(),
        )
    }
}

// ---------- 3. 每千 token 展开吞吐 ----------

/// 高强度宏展开文档：粗略估算展开吞吐（近似指标，见注释）。
pub struct ExpandThroughput;

impl Benchmark for ExpandThroughput {
    fn name(&self) -> &'static str {
        "expand-throughput"
    }
    fn description(&self) -> &'static str {
        "宏展开吞吐（高频 \\foo 调用，近似 token/s）"
    }
    fn run(&self, driver: &dyn EngineDriver, opts: &BenchOptions) -> Result<BenchResult> {
        const CALLS: usize = 200_000;
        let mut tex = String::from(
            "\\documentclass{article}\n\\def\\foo#1{\\textbf{#1}}\n\\begin{document}\n",
        );
        for _ in 0..CALLS / 4 {
            // 每行 4 次调用
            tex.push_str("\\foo{a} \\foo{b} \\foo{c} \\foo{d}\n");
        }
        tex.push_str("\\end{document}\n");

        let note = format!("约 {} 次宏调用（近似：token/时间 仅为参考量级）", CALLS);
        match run_measured(driver, opts, "expand", &tex, OutputFormat::Pdf, note)? {
            BenchResult::Measured { stats, note } => {
                let per_s = CALLS as f64 / (stats.median_ns() as f64 / 1e9);
                Ok(BenchResult::Measured {
                    note: format!("{note}；吞吐 ≈ {per_s:.0} 调用/s"),
                    stats,
                })
            }
            other => Ok(other),
        }
    }
}

// ---------- 4. 增量场景：改 1 字 ----------

/// 全量重编 vs 单字编辑后的重编（引擎实现增量前，两者应几乎相同）。
pub struct IncrementalEdit;

impl Benchmark for IncrementalEdit {
    fn name(&self) -> &'static str {
        "incremental-edit"
    }
    fn description(&self) -> &'static str {
        "编辑 1 个字符后的重编耗时（增量架构的量化指标）"
    }
    fn run(&self, driver: &dyn EngineDriver, opts: &BenchOptions) -> Result<BenchResult> {
        let base = "\\documentclass{article}\n\\begin{document}\n".to_owned()
            + &("\\par paragraph body \\the\\count0\\par\n".repeat(2000))
            + "\\end{document}\n";
        let edited = base.replacen("paragraph", "paragraph.", 1);

        let full = run_measured(
            driver,
            opts,
            "full",
            &base,
            OutputFormat::Pdf,
            String::new(),
        )?;
        let edit = run_measured(
            driver,
            opts,
            "edit",
            &edited,
            OutputFormat::Pdf,
            String::new(),
        )?;

        let (full_stats, edit_stats) = match (full, edit) {
            (BenchResult::Measured { stats: a, .. }, BenchResult::Measured { stats: b, .. }) => {
                (a, b)
            }
            (BenchResult::Skipped { reason }, _) | (_, BenchResult::Skipped { reason }) => {
                return Ok(BenchResult::Skipped { reason });
            }
        };

        let ratio = full_stats.median_ns() as f64 / edit_stats.median_ns().max(1) as f64;
        let note = format!(
            "全量 {:.1}ms / 编辑后 {:.1}ms（比值 {ratio:.2}x；增量落地前应 ≈1x）",
            full_stats.median_ns() as f64 / 1e6,
            edit_stats.median_ns() as f64 / 1e6,
        );
        Ok(BenchResult::Measured {
            stats: edit_stats,
            note,
        })
    }
}

// ---------- 5. 段落折行计算：全量 vs 增量命中 ----------

/// 段落折行层的增量收益量化（M5）：对同一段水平列表，比较
/// 全量 `knuth_plass` 折行耗时 vs 增量路径（内容指纹 + 记忆表命中）耗时。
///
/// 语义说明：M5 的真实收益在**文档级**——编辑改动原子（如改 1 字）只影响个别
/// 段落，全量对 P 段全部跑 Knuth-Plass，增量只做 P 次 O(n) 指纹 + 查表（命中
/// 复用已物化行盒），仅改动段重算。本基准构造 P 段等长文档：
///
/// - 全量：对 P 段逐段跑 Knuth-Plass（冷排版）；
/// - 增量：P 段均已入缓存，仅对每段做指纹 + 查表（命中即跳过折行动态规划）。
///
/// 比值 ≈ P×折行 / P×指纹+查表：折行 O(段长×断点候选) 远贵于指纹 O(段长)，
/// 段数越多、段越长收益越大。此度量专注"折行计算"层，文档级宏展开/求值跳过
/// 属 M5 后续（求值图），届时端到端体现。
pub struct IncrementalParagraph;

impl Benchmark for IncrementalParagraph {
    fn name(&self) -> &'static str {
        "incremental-paragraph"
    }
    fn description(&self) -> &'static str {
        "段落折行层：P 段全量 Knuth-Plass vs P 段指纹+命中复用"
    }
    fn run(&self, _driver: &dyn EngineDriver, opts: &BenchOptions) -> Result<BenchResult> {
        use ntex_incremental::{Fingerprint, ParagraphCache};
        use ntex_layout::incremental::paragraph_fingerprint;
        use ntex_layout::knuth_plass;
        use ntex_layout::node::{FontId, Node, GLUE_ORDER_FIL};

        const PARAS: usize = 40; // 40 段
        const WORDS: usize = 200; // 段长 200 词 × 8 字母 ≈ 1600 字符，约 10 行
        const HSIZE: i64 = 50 * 1000; // 词间距 2500（sp），行约 20 词
        const TOLERANCE: i64 = 200;

        // 构造 P 段"内容各异的水平列表"（与真实文档同构），并算好各自指纹。
        fn make_para(seed: u32) -> Vec<Node> {
            let mut list: Vec<Node> = Vec::new();
            for w in 0..WORDS {
                for k in 0..8u32 {
                    let ch = ((w as u32 * 7 + seed * 3 + k) % 26) + b'a' as u32;
                    list.push(Node::Char {
                        font: FontId(0),
                        charcode: ch,
                        width: 500 + i64::from(ch),
                        height: 6000,
                        depth: 1500,
                    });
                }
                if w + 1 < WORDS {
                    list.push(Node::Glue {
                        width: 2500,
                        stretch: 0,
                        shrink: 0,
                        stretch_order: GLUE_ORDER_FIL,
                        shrink_order: 0,
                    });
                }
            }
            list
        }
        let paras: Vec<Vec<Node>> = (0..PARAS).map(|i| make_para(i as u32)).collect();
        let keys: Vec<Fingerprint> = paras
            .iter()
            .map(|p| paragraph_fingerprint(p, HSIZE, TOLERANCE))
            .collect();

        // 全量：每次对 P 段逐段跑 Knuth-Plass（冷排版全部段落）。
        let full = measure(opts.warmup, opts.iterations, || {
            let mut total = 0usize;
            for p in &paras {
                total = total.wrapping_add(knuth_plass(p, HSIZE, TOLERANCE).len());
            }
            let _ = std::hint::black_box(total);
            Ok(())
        })?;

        // 增量命中路径：P 段全部入缓存，仅做指纹 + 查表，命中即跳过折行。
        let mut cache: ParagraphCache<usize> = ParagraphCache::new(4096);
        for (i, k) in keys.iter().enumerate() {
            cache.put(*k, i);
        }
        let inc = measure(opts.warmup, opts.iterations, || {
            let mut total = 0usize;
            for k in &keys {
                if let Some(v) = std::hint::black_box(cache.get(*k)) {
                    total += *v;
                }
            }
            let _ = std::hint::black_box(total);
            Ok(())
        })?;

        let ratio = full.median_ns() as f64 / inc.median_ns().max(1) as f64;
        let note = format!(
            "全量 {} 段折行 {:.3}ms / 增量{} 段指纹+命中 {:.3}ms（单次文档排印比值 ≈ {ratio:.0}x）",
            PARAS,
            full.median_ns() as f64 / 1e6,
            PARAS,
            inc.median_ns() as f64 / 1e6,
        );
        Ok(BenchResult::Measured { stats: inc, note })
    }
}

/// 全量基准列表。
pub const ALL: &[&dyn Benchmark] = &[
    &LatexFmtLoad,
    &Doc300,
    &ExpandThroughput,
    &IncrementalEdit,
    &IncrementalParagraph,
];

/// 按名称查找基准。
pub fn find(name: &str) -> Option<&'static dyn Benchmark> {
    ALL.iter().copied().find(|b| b.name() == name)
}

// ---------- 辅助 ----------

/// 写临时 .tex 并计时编译，处理驱动状态。
fn run_measured(
    driver: &dyn EngineDriver,
    opts: &BenchOptions,
    tag: &str,
    tex: &str,
    format: OutputFormat,
    note: String,
) -> Result<BenchResult> {
    let dir = tempfile::tempdir().map_err(|e| anyhow::anyhow!("创建临时目录失败：{e}"))?;
    let source = dir.path().join("bench.tex");
    std::fs::write(&source, tex).map_err(|e| anyhow::anyhow!("写 {tag} 样张失败：{e}"))?;

    let request = RunRequest {
        source,
        working_dir: dir.path().to_path_buf(),
        interaction: InteractionMode::Batch,
        format,
    };

    // 预热前先探测一次：驱动未实现 / 编译失败直接定论，不进入计时。
    match driver.run(&request)?.status {
        DriverStatus::NotImplemented => {
            return Ok(BenchResult::Skipped {
                reason: format!("驱动 {} 未实现", driver.name()),
            });
        }
        DriverStatus::Failure { code } => bail!("基准 {tag}：驱动编译失败（退出码 {code:?}）"),
        DriverStatus::Success => {}
    }

    let stats = measure(opts.warmup, opts.iterations, || {
        driver.run(&request).map(|_| ())
    })?;
    Ok(BenchResult::Measured { stats, note })
}
