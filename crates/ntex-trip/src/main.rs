//! TRIP 一致性测试框架入口。

mod fixtures;
mod harness;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use ntex_test_support::build_driver;

use crate::fixtures::TripFixtures;

#[derive(Parser)]
#[command(
    name = "ntex-trip",
    version,
    about = "TRIP 一致性测试框架：验证引擎与 Knuth 官方测试输出逐位一致"
)]
struct Args {
    /// 引擎驱动：`stub` 或程序名（如 `external=pdflatex`）。
    #[arg(long, default_value = "stub")]
    driver: String,

    /// fixtures 目录覆盖（默认：workspace 根下 fixtures/trip）。
    #[arg(long)]
    fixtures: Option<PathBuf>,
}

fn main() -> Result<ExitCode> {
    let args = Args::parse();
    let driver = build_driver(&args.driver)?;
    let fixtures = TripFixtures::locate(args.fixtures.as_deref())?;

    let outcome = harness::run_trip(driver.as_ref(), &fixtures)?;

    match &outcome {
        harness::TripOutcome::Pass => {
            println!("TRIP：通过（驱动 {}）", driver.name());
            Ok(ExitCode::SUCCESS)
        }
        harness::TripOutcome::Skipped { reason } => {
            println!("TRIP：跳过（驱动 {}）：{reason}", driver.name());
            Ok(ExitCode::SUCCESS)
        }
        harness::TripOutcome::Fail { log_diff, typ_diff } => {
            eprintln!("TRIP：失败（驱动 {}）", driver.name());
            eprintln!("===== typ diff =====");
            eprintln!("{typ_diff}");
            eprintln!("===== log diff =====");
            eprintln!("{log_diff}");
            Ok(ExitCode::FAILURE)
        }
    }
}
