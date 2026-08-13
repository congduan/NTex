//! 差分测试工具：同一 fixture 分别跑参考引擎（pdfTeX 等）与本引擎，比对输出。
//!
//! M0 阶段：引擎驱动为 stub 时验证管路并输出"跳过"；接入真实引擎后作为回归门禁。

mod run;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use ntex_test_support::build_driver;

#[derive(Parser)]
#[command(
    name = "ntex-diff",
    version,
    about = "差分测试：参考引擎 vs 本引擎，逐 fixture 比对归一化日志"
)]
struct Args {
    /// fixtures 目录（含 .tex 文件）。
    #[arg(long)]
    fixtures: PathBuf,

    /// 参考驱动：`stub` 或程序名（如 `external=pdflatex`）。
    #[arg(long, default_value = "external=pdflatex")]
    reference: String,

    /// 引擎驱动：`stub` 或程序名。
    #[arg(long, default_value = "stub")]
    engine: String,

    /// diff 上下文行数。
    #[arg(long, default_value_t = 3)]
    context: usize,
}

fn main() -> Result<ExitCode> {
    let args = Args::parse();
    let reference = build_driver(&args.reference)?;
    let engine = build_driver(&args.engine)?;

    let (results, summary) = run::run_fixture_set(
        &args.fixtures,
        reference.as_ref(),
        engine.as_ref(),
        args.context,
    )?;

    println!("{:<40} {:<14} {:<10}", "fixture", "result", "detail");
    println!("{}", "-".repeat(80));
    for r in &results {
        let result = match r.matches {
            Some(true) => "pass",
            Some(false) => "MISMATCH",
            None => "skipped",
        };
        let detail = truncate(&r.detail, 60);
        println!("{:<40} {:<14} {:<10}", r.name, result, detail);
    }
    println!("{}", "-".repeat(80));
    println!(
        "总计 {} | 一致 {} | 不一致 {} | 跳过 {}",
        summary.total, summary.pass, summary.mismatch, summary.skipped
    );

    Ok(if summary.has_failures() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…")
    }
}
