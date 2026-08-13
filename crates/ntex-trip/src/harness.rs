//! TRIP 一致性测试运行逻辑。
//!
//! 流程：将 trip.tex 复制到临时工作目录 → 用指定驱动以 batch 模式编译 →
//! 将产出的 `trip.typ`/`trip.log` 与参考文件（归一化后）逐行比对。

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use ntex_test_support::{diff, EngineDriver, InteractionMode, OutputFormat, RunOutput, RunRequest};

use crate::fixtures::TripFixtures;

/// TRIP 测试结果。
#[derive(Debug)]
pub enum TripOutcome {
    /// 与参考输出一致。
    Pass,
    /// 输出存在差异（附 diff）。
    Fail { log_diff: String, typ_diff: String },
    /// 无法执行（fixtures 缺失 / 驱动未实现），不算失败。
    Skipped { reason: String },
}

/// 运行一次 TRIP 测试。
pub fn run_trip(driver: &dyn EngineDriver, fixtures: &TripFixtures) -> Result<TripOutcome> {
    if !fixtures.ready() {
        return Ok(TripOutcome::Skipped {
            reason: "fixtures 缺失（需要 trip.tex/trip.typ/trip.log），请运行 scripts/fetch-trip-fixtures.sh"
                .to_owned(),
        });
    }

    let work = tempfile::tempdir().context("创建临时工作目录失败")?;
    fs::copy(fixtures.trip_tex(), work.path().join("trip.tex"))
        .with_context(|| format!("复制 trip.tex 失败：{}", fixtures.trip_tex().display()))?;

    let output = run_driver(driver, work.path())?;
    match output.status {
        ntex_test_support::DriverStatus::NotImplemented => Ok(TripOutcome::Skipped {
            reason: format!("驱动 {} 未实现", driver.name()),
        }),
        ntex_test_support::DriverStatus::Failure { code } => {
            let log = read_log(work.path());
            Ok(TripOutcome::Fail {
                log_diff: format!("编译失败（退出码 {code:?}）\n{log}"),
                typ_diff: String::new(),
            })
        }
        ntex_test_support::DriverStatus::Success => {
            let actual_typ = fs::read_to_string(work.path().join("trip.typ"))
                .context("成功编译但缺少 trip.typ 输出")?;
            let actual_log = fs::read_to_string(work.path().join("trip.log"))
                .context("成功编译但缺少 trip.log 输出")?;

            let expected_typ =
                fs::read_to_string(fixtures.trip_typ()).context("读取参考 trip.typ 失败")?;
            let expected_log =
                fs::read_to_string(fixtures.trip_log()).context("读取参考 trip.log 失败")?;

            let typ_diff = diff::render_diff(
                &diff::normalize_log(&expected_typ),
                &diff::normalize_log(&actual_typ),
                3,
            );
            let log_diff = diff::render_diff(
                &diff::normalize_log(&expected_log),
                &diff::normalize_log(&actual_log),
                3,
            );

            if typ_diff.is_empty() && log_diff.is_empty() {
                Ok(TripOutcome::Pass)
            } else {
                Ok(TripOutcome::Fail { log_diff, typ_diff })
            }
        }
    }
}

/// 执行一次编译请求（固定工作目录与参数）。
fn run_driver(driver: &dyn EngineDriver, working_dir: &Path) -> Result<RunOutput> {
    let request = RunRequest {
        source: working_dir.join("trip.tex"),
        working_dir: working_dir.to_path_buf(),
        interaction: InteractionMode::Batch,
        format: OutputFormat::Log,
    };
    driver.run(&request).context("驱动执行失败")
}

/// 读取工作目录中的日志（用于失败诊断），尽力而为。
fn read_log(working_dir: &Path) -> String {
    fs::read_to_string(working_dir.join("trip.log")).unwrap_or_else(|_| "<无 trip.log>".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ntex_test_support::StubDriver;

    #[test]
    fn stub_driver_yields_skipped() {
        let dir = TripFixtures::locate(None).unwrap();
        let outcome = run_trip(&StubDriver::default(), &dir).unwrap();
        match outcome {
            TripOutcome::Skipped { reason } => assert!(!reason.is_empty()),
            other => panic!("预期 Skipped，实际 {other:?}"),
        }
    }
}
