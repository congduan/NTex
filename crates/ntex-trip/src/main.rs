//! TRIP/ETRIP 一致性测试框架入口。

mod fixtures;
mod harness;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use ntex_test_support::build_driver;

use crate::fixtures::{TestFixtures, TestKind};
use crate::harness::{run_test, TripOutcome};

#[derive(Parser)]
#[command(
    name = "ntex-trip",
    version,
    about = "TRIP/ETRIP 一致性测试框架：验证引擎与 Knuth 官方测试输出逐位一致"
)]
struct Args {
    /// 引擎驱动：`stub`、`ntex`（本引擎）或程序名（如 `external=pdflatex`）。
    #[arg(long, default_value = "stub")]
    driver: String,

    /// 运行哪种测试：`trip`、`etrip` 或 `both`。
    #[arg(long, default_value = "both")]
    test: String,

    /// fixtures 目录覆盖（默认：workspace 根下 fixtures）。
    #[arg(long)]
    fixtures: Option<PathBuf>,
}

fn main() -> Result<ExitCode> {
    let args = Args::parse();
    let driver = build_driver(&args.driver)?;

    let kinds = match args.test.as_str() {
        "trip" => vec![TestKind::Trip],
        "etrip" => vec![TestKind::Etrip],
        "both" => vec![TestKind::Trip, TestKind::Etrip],
        other => anyhow::bail!("无效的 --test 值：{other:?}（应为 trip、etrip 或 both）"),
    };

    let mut any_failed = false;
    for kind in kinds {
        let fixtures = TestFixtures::locate(kind, args.fixtures.as_deref())?;
        let outcome = run_test(driver.as_ref(), &fixtures)?;
        match &outcome {
            TripOutcome::Pass => {
                println!("{}：通过（驱动 {}）", kind.name(), driver.name());
            }
            TripOutcome::Skipped { reason } => {
                println!("{}：跳过（驱动 {}）：{reason}", kind.name(), driver.name());
            }
            TripOutcome::Fail { log_diff, typ_diff } => {
                any_failed = true;
                eprintln!("{}：失败（驱动 {}）", kind.name(), driver.name());
                if !typ_diff.is_empty() {
                    eprintln!("===== typ diff =====");
                    eprintln!("{typ_diff}");
                }
                eprintln!("===== log diff =====");
                eprintln!("{log_diff}");
            }
        }
    }

    Ok(if any_failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
