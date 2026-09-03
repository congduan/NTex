//! 基准集合：plan.md M0 基准集的基准（M2-6 补入双轨展开吞吐对照）。
//!
//! 每个基准面向 [`EngineDriver`] 编写：驱动未实现时返回 `Skipped`（不算失败），
//! 引擎本体就绪后无需改动基准定义即可接入。

use anyhow::{bail, Result};

use crate::measure::{measure, Stats};
use ntex_test_support::{DriverStatus, EngineDriver, InteractionMode, OutputFormat, RunRequest};

/// 基准参数（由 CLI 注入）。
#[derive(Debug, Clone)]
pub struct BenchOptions {
    pub warmup: usize,
    pub iterations: usize,
    /// 外部样张覆盖（CLI `--tex-file`；`None` = 使用基准内嵌样张）。
    pub tex_override: Option<String>,
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

// ---------- 4. 双轨展开吞吐对照（M2-6 验收） ----------

/// 字节码执行器 vs 纯解释器：同一宏密集样张分别跑 [`ntex_core::expand::Expander`]
/// 的两条轨道（`with_bytecode(true/false)`），M2-6 验收"展开吞吐 ≥ 解释器 2x"。
///
/// 与 `expand-throughput`（全管线，排版/字体成本稀释比值）不同，本基准直接驱动
/// 展开层，隔离出执行器本身的差异；两轨输出逐 token 断言一致（双轨等价的运行期
/// 抽查）。语张沿用 1.12x 历史测量（`bytecode_vs_interpreter_throughput`）的形态
/// （带参宏调用 + 常量条件），但按真实 TeX 行长断行、样本放大到 20 万次调用、
/// 预热 ≥2 次 + 计时 ≥5 次取中位数。
pub struct ExpandDualTrack;

impl Benchmark for ExpandDualTrack {
    fn name(&self) -> &'static str {
        "expand-dual"
    }
    fn description(&self) -> &'static str {
        "双轨展开吞吐（字节码 vs 解释器，M2-6 验收）"
    }
    fn run(&self, _driver: &dyn EngineDriver, opts: &BenchOptions) -> Result<BenchResult> {
        use ntex_core::expand::Expander;

        const CALLS: usize = 200_000;
        // 每行 4 次调用（2 次带参宏 + 2 次常量条件），行长 ≈ 24 字符（真实 TeX 行长量级）
        let built_in = || {
            let mut tex = String::from("\\def\\foo#1{#1X}\\def\\bar{\\iftrue Y\\else N\\fi}\n");
            for _ in 0..CALLS / 4 {
                tex.push_str("\\foo{a}\\bar\\foo{b}\\bar\n");
            }
            (tex, Some(CALLS))
        };
        // CLI `--tex-file` 覆盖（main.rs 已读为内容）：外部样张只报 token/s 与比值
        let (tex, calls) = match &opts.tex_override {
            Some(body) => (body.clone(), None),
            None => built_in(),
        };

        let run_once = |use_bytecode: bool| -> Result<Vec<ntex_core::token::Token>> {
            let mut e = if use_bytecode {
                Expander::new()
            } else {
                Expander::new_interpreter()
            };
            e.run_source(&tex)?;
            Ok(e.output().to_vec())
        };

        // 双轨等价抽查：同一样张输出必须逐 token 一致
        let bc_out = run_once(true)?;
        let ip_out = run_once(false)?;
        if bc_out != ip_out {
            bail!(
                "expand-dual：双轨输出不一致（字节码 {} token / 解释器 {} token）",
                bc_out.len(),
                ip_out.len()
            );
        }

        let warmup = opts.warmup.max(2);
        let iterations = opts.iterations.max(5);
        let stats_of = |use_bytecode: bool| -> Result<Stats> {
            measure(warmup, iterations, || run_once(use_bytecode).map(|_| ()))
        };
        let bc = stats_of(true)?;
        let ip = stats_of(false)?;

        let ms = |s: &Stats| s.median_ns() as f64 / 1e6;
        let per_s = |s: &Stats| calls.unwrap_or(0) as f64 / (s.median_ns() as f64 / 1e9);
        let tok_per_s = |s: &Stats| bc_out.len() as f64 / (s.median_ns() as f64 / 1e9);
        // 比值按耗时反算（≥1 表示字节码更快）；调用计数缺失（外部样张）时用 token 吞吐
        let ratio = match calls {
            Some(_) => ms(&ip) / ms(&bc),
            None => tok_per_s(&bc) / tok_per_s(&ip),
        };
        let calls_txt = match calls {
            Some(n) => format!(
                "{n} 次调用（{:.0} 调用/s vs {:.0} 调用/s）",
                per_s(&bc),
                per_s(&ip)
            ),
            None => "外部样张".to_owned(),
        };
        let note = format!(
            "{calls_txt} / {} token 输出；字节码 {:.1}ms（{:.0} token/s）vs 解释器 {:.1}ms \
             （{:.0} token/s）；比值 {ratio:.2}x（验收 ≥2x）",
            bc_out.len(),
            ms(&bc),
            tok_per_s(&bc),
            ms(&ip),
            tok_per_s(&ip),
        );
        Ok(BenchResult::Measured { stats: bc, note })
    }
}

// ---------- 5. 增量场景：改 1 字 ----------

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

/// 全量基准列表。
pub const ALL: &[&dyn Benchmark] = &[
    &LatexFmtLoad,
    &Doc300,
    &ExpandThroughput,
    &ExpandDualTrack,
    &IncrementalEdit,
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
    // CLI `--tex-file` 覆盖基准内嵌样张（对照外部样张吞吐用）。
    let tex = opts.tex_override.as_deref().unwrap_or(tex);
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
