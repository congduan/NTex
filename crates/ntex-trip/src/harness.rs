//! TRIP / ETRIP 一致性测试运行逻辑。
//!
//! 流程：将测试源码（`trip.tex`/`etrip.tex`）复制到临时工作目录 → 用指定驱动以
//! batch 模式编译 → 将产出的 `.log`/`.typ` 与参考文件（归一化后）逐行比对。
//!
//! 参考文件按测试种类：
//! - TRIP：`trip.typ`（终端输出）+ `trip.log`；
//! - ETRIP：`etrip.log`（web2c 参考；终端输出经 dvitype 比对，暂不纳入）。

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use ntex_test_support::{diff, EngineDriver, InteractionMode, OutputFormat, RunOutput, RunRequest};

use crate::fixtures::{TestFixtures, TestKind};

/// 一致性测试结果。
#[derive(Debug)]
pub enum TripOutcome {
    /// 与参考输出一致。
    Pass,
    /// 输出存在差异（附 diff）。
    Fail { log_diff: String, typ_diff: String },
    /// 无法执行（fixtures 缺失 / 驱动未实现），不算失败。
    Skipped { reason: String },
}

/// 运行一次一致性测试（TRIP 或 ETRIP）。
pub fn run_test(driver: &dyn EngineDriver, fixtures: &TestFixtures) -> Result<TripOutcome> {
    let kind = fixtures.kind();
    if !fixtures.ready() {
        return Ok(TripOutcome::Skipped {
            reason: format!(
                "{} fixtures 缺失（需要 {}），请运行 scripts/fetch-trip-fixtures.sh",
                kind.name(),
                fixtures.tex().display()
            ),
        });
    }

    let work = tempfile::tempdir().context("创建临时工作目录失败")?;
    let tex_name = kind.tex_name();
    fs::copy(fixtures.tex(), work.path().join(tex_name))
        .with_context(|| format!("复制 {} 失败：{}", tex_name, fixtures.tex().display()))?;

    // 测试字体（trip.tfm/etrip.tfm）与源码同目录：让 ntex-font::find_tfm 能找到。
    // 环境变量进程级、只读一次即可（两个 kind 的 fixtures 目录不同，各设置一次）。
    std::env::set_var("NTEX_TFM_DIR", fixtures.dir());
    let output = run_driver(driver, work.path(), tex_name)?;
    match output.status {
        ntex_test_support::DriverStatus::NotImplemented => Ok(TripOutcome::Skipped {
            reason: format!("驱动 {} 未实现", driver.name()),
        }),
        ntex_test_support::DriverStatus::Failure { code } => {
            let log = read_artifact(work.path(), kind, "log");
            Ok(TripOutcome::Fail {
                log_diff: format!("编译失败（退出码 {code:?}）\n{log}"),
                typ_diff: String::new(),
            })
        }
        ntex_test_support::DriverStatus::Success => {
            let mut log_diff = String::new();
            let mut typ_diff = String::new();
            for (name, actual, expected) in compare_artifacts(fixtures, work.path())? {
                let d = diff::render_diff(
                    &diff::normalize_log(&expected),
                    &diff::normalize_log(&actual),
                    3,
                );
                if !d.is_empty() {
                    if name == "typ" {
                        typ_diff = d;
                    } else {
                        log_diff = d;
                    }
                }
            }
            if log_diff.is_empty() && typ_diff.is_empty() {
                Ok(TripOutcome::Pass)
            } else {
                Ok(TripOutcome::Fail { log_diff, typ_diff })
            }
        }
    }
}

/// 收集 (产物名, 实际内容, 参考内容)：log 必有；typ 仅 TRIP。
fn compare_artifacts(
    fixtures: &TestFixtures,
    work_dir: &Path,
) -> Result<Vec<(&'static str, String, String)>> {
    let mut out = Vec::new();
    let kind = fixtures.kind();
    let log_actual = fs::read_to_string(work_dir.join(format!("{}.log", kind_name(kind))))
        .context("成功编译但缺少 .log 输出")?;
    let log_expected = fs::read_to_string(fixtures.log()).context("读取参考 .log 失败")?;
    out.push(("log", log_actual, log_expected));
    if let Some(typ_path) = fixtures.typ() {
        let typ_actual = fs::read_to_string(work_dir.join("trip.typ"))
            .context("成功编译但缺少 trip.typ 输出")?;
        let typ_expected = fs::read_to_string(&typ_path).context("读取参考 trip.typ 失败")?;
        out.push(("typ", typ_actual, typ_expected));
    }
    Ok(out)
}

/// 执行一次编译请求（固定工作目录与参数）。
fn run_driver(driver: &dyn EngineDriver, working_dir: &Path, tex_name: &str) -> Result<RunOutput> {
    let request = RunRequest {
        source: working_dir.join(tex_name),
        working_dir: working_dir.to_path_buf(),
        interaction: InteractionMode::Batch,
        format: OutputFormat::Log,
    };
    driver.run(&request).context("驱动执行失败")
}

/// 读取工作目录中的产物（用于失败诊断），尽力而为。
fn read_artifact(working_dir: &Path, kind: TestKind, ext: &str) -> String {
    fs::read_to_string(working_dir.join(format!("{}.{ext}", kind_name(kind))))
        .unwrap_or_else(|_| format!("<无 {}.{ext}>", kind_name(kind)))
}

fn kind_name(kind: TestKind) -> &'static str {
    match kind {
        TestKind::Trip => "trip",
        TestKind::Etrip => "etrip",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ntex_test_support::StubDriver;

    #[test]
    fn stub_driver_yields_skipped() {
        let fixtures = TestFixtures::locate(TestKind::Trip, None).unwrap();
        let outcome = run_test(&StubDriver::default(), &fixtures).unwrap();
        match outcome {
            TripOutcome::Skipped { reason } => assert!(!reason.is_empty()),
            other => panic!("预期 Skipped，实际 {other:?}"),
        }
    }
}
