//! 基准框架：plan.md M0 基准集的执行入口。
//!
//! 用法示例：
//! - `cargo run -p ntex-bench -- --driver stub`（验证管路）
//! - `cargo run -p ntex-bench -- --driver external=pdflatex --bench all --iterations 5`

mod benchmarks;
mod measure;

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use ntex_test_support::build_driver;

use benchmarks::{find, BenchOptions, BenchResult, ALL};
use measure::format_duration;

#[derive(Parser)]
#[command(name = "ntex-bench", version, about = "NTex 基准框架")]
struct Args {
    /// 引擎驱动：`stub` 或程序名（如 `external=pdflatex`）。
    #[arg(long, default_value = "stub")]
    driver: String,

    /// 要运行的基准名（`all` 或某个 name）。
    #[arg(long, default_value = "all")]
    bench: String,

    /// 预热次数。
    #[arg(long, default_value_t = 1)]
    warmup: usize,

    /// 计时取样次数。
    #[arg(long, default_value_t = 5)]
    iterations: usize,
}

fn main() -> Result<ExitCode> {
    let args = Args::parse();
    let driver = build_driver(&args.driver)?;

    let selected: Vec<&'static dyn benchmarks::Benchmark> = if args.bench == "all" {
        ALL.to_vec()
    } else {
        match find(&args.bench) {
            Some(b) => vec![b],
            None => {
                eprintln!(
                    "未知基准：{}（可用：all, {}）",
                    args.bench,
                    ALL.iter().map(|b| b.name()).collect::<Vec<_>>().join(", ")
                );
                return Ok(ExitCode::FAILURE);
            }
        }
    };

    let opts = BenchOptions {
        warmup: args.warmup,
        iterations: args.iterations,
    };
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    println!(
        "NTex benchmark | driver={} | cpus={cpus} | warmup={} iterations={}",
        driver.name(),
        args.warmup,
        args.iterations
    );
    println!("{}", "-".repeat(110));

    for bench in selected {
        let result = match bench.run(driver.as_ref(), &opts)? {
            BenchResult::Measured { stats, note } => format!(
                "min={:<10} median={:<10} mean={:<10} max={:<10} | {}",
                format_duration(stats.min_ns()),
                format_duration(stats.median_ns()),
                format_duration(stats.mean_ns()),
                format_duration(stats.max_ns()),
                note,
            ),
            BenchResult::Skipped { reason } => format!("SKIPPED | {reason}"),
        };
        println!(
            "{:<22} {:<44} {}",
            bench.name(),
            bench.description(),
            result
        );
    }

    Ok(ExitCode::SUCCESS)
}
