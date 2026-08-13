//! 差分测试运行逻辑。
//!
//! 对每个 fixture：参考驱动与引擎驱动各编一次，比对归一化后的日志；
//! 引擎未实现 / 参考缺失视为"跳过"（不失败），保证 CI 冒烟可跑。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ntex_test_support::{diff, EngineDriver, InteractionMode, OutputFormat, RunOutput, RunRequest};

/// 单个 fixture 的比对结果。
#[derive(Debug)]
pub struct FixtureResult {
    pub name: String,
    /// `Some(true)` 一致；`Some(false)` 不一致；`None` 无法比对（跳过）。
    pub matches: Option<bool>,
    pub detail: String,
}

/// 汇总统计。
#[derive(Debug, Default)]
pub struct Summary {
    pub total: usize,
    pub pass: usize,
    pub mismatch: usize,
    pub skipped: usize,
    pub failed: usize,
}

impl Summary {
    /// 是否有需要 CI 判失败的项。
    pub fn has_failures(&self) -> bool {
        self.mismatch > 0 || self.failed > 0
    }
}

/// 对目录下全部 `.tex` fixture 执行差分比对。
pub fn run_fixture_set(
    fixtures_dir: &Path,
    reference: &dyn EngineDriver,
    engine: &dyn EngineDriver,
    context: usize,
) -> Result<(Vec<FixtureResult>, Summary)> {
    let mut files: Vec<PathBuf> = fs::read_dir(fixtures_dir)
        .with_context(|| format!("读取 fixtures 目录失败：{}", fixtures_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("tex"))
        .collect();
    files.sort();

    let mut results = Vec::new();
    let mut summary = Summary::default();

    for file in files {
        summary.total += 1;
        let result = run_one_fixture(&file, reference, engine, context)?;
        match result.matches {
            Some(true) => summary.pass += 1,
            Some(false) => {
                if result.detail.contains("编译失败") {
                    summary.failed += 1;
                } else {
                    summary.mismatch += 1;
                }
            }
            None => summary.skipped += 1,
        }
        results.push(result);
    }
    Ok((results, summary))
}

fn run_one_fixture(
    file: &Path,
    reference: &dyn EngineDriver,
    engine: &dyn EngineDriver,
    context: usize,
) -> Result<FixtureResult> {
    let name = file.file_name().unwrap().to_string_lossy().into_owned();
    let ref_dir = tempfile::tempdir().context("创建参考工作目录失败")?;
    let eng_dir = tempfile::tempdir().context("创建引擎工作目录失败")?;

    fs::copy(file, ref_dir.path().join(file.file_name().unwrap()))
        .with_context(|| format!("复制 {} 失败", file.display()))?;
    fs::copy(file, eng_dir.path().join(file.file_name().unwrap()))
        .with_context(|| format!("复制 {} 失败", file.display()))?;

    let ref_out = run_one(reference, ref_dir.path())?;
    let eng_out = run_one(engine, eng_dir.path())?;

    match (ref_out.status, eng_out.status) {
        (ntex_test_support::DriverStatus::NotImplemented, _) => Ok(FixtureResult {
            name,
            matches: None,
            detail: "参考驱动未实现".to_owned(),
        }),
        (_, ntex_test_support::DriverStatus::NotImplemented) => Ok(FixtureResult {
            name,
            matches: None,
            detail: "引擎未实现".to_owned(),
        }),
        (ntex_test_support::DriverStatus::Failure { .. }, _) => Ok(FixtureResult {
            name,
            matches: None,
            detail: "参考编译失败".to_owned(),
        }),
        (_, ntex_test_support::DriverStatus::Failure { code }) => Ok(FixtureResult {
            name,
            matches: Some(false),
            detail: format!("引擎编译失败（退出码 {code:?}）"),
        }),
        (ntex_test_support::DriverStatus::Success, ntex_test_support::DriverStatus::Success) => {
            let ref_log = read_log(ref_dir.path());
            let eng_log = read_log(eng_dir.path());
            let a = diff::normalize_log(&ref_log);
            let b = diff::normalize_log(&eng_log);
            if a == b {
                Ok(FixtureResult {
                    name,
                    matches: Some(true),
                    detail: "日志一致".to_owned(),
                })
            } else {
                let d = diff::render_diff(&a, &b, context);
                Ok(FixtureResult {
                    name,
                    matches: Some(false),
                    detail: format!("日志不一致，前 {context} 行上下文 diff：\n{d}"),
                })
            }
        }
    }
}

fn run_one(driver: &dyn EngineDriver, working_dir: &Path) -> Result<RunOutput> {
    let file_name = working_dir
        .read_dir()?
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().and_then(|x| x.to_str()) == Some("tex"))
        .map(|e| e.file_name())
        .context("工作目录缺少 .tex 输入")?;
    let request = RunRequest {
        source: working_dir.join(file_name),
        working_dir: working_dir.to_path_buf(),
        interaction: InteractionMode::Batch,
        format: OutputFormat::Log,
    };
    driver.run(&request).context("驱动执行失败")
}

fn read_log(working_dir: &Path) -> String {
    let log = working_dir.join("out.log");
    let candidate = fs::read_dir(working_dir)
        .ok()
        .and_then(|it| {
            it.filter_map(Result::ok)
                .map(|e| e.path())
                .find(|p| p.extension().and_then(|x| x.to_str()) == Some("log"))
        })
        .unwrap_or(log);
    fs::read_to_string(candidate).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ntex_test_support::StubDriver;

    #[test]
    fn stub_engine_is_skipped_not_failed() {
        // fixtures/diff 是仓库自带示例，仅用于验证管路。
        let dir = workspace_root().join("fixtures").join("diff");
        let (results, summary) =
            run_fixture_set(&dir, &StubDriver::default(), &StubDriver::default(), 3).unwrap();
        assert!(!results.is_empty());
        assert_eq!(summary.total, results.len());
        assert_eq!(summary.skipped, results.len());
        assert!(!summary.has_failures());
    }

    fn workspace_root() -> PathBuf {
        let manifest = env!("CARGO_MANIFEST_DIR");
        Path::new(manifest)
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .to_path_buf()
    }
}
