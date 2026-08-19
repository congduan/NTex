//! 引擎驱动抽象。
//!
//! TRIP 框架、差分测试、基准全部面向 [`EngineDriver`] 编程：
//! 上层工具只关心"发一次编译请求 → 拿回产物与状态"，不关心请求由谁执行。
//! 这使得引擎本体接入时零改动工具逻辑。

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

/// 编译产物格式（决定驱动成功后要收集哪些文件）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// DVI 中间格式。
    Dvi,
    /// PDF 直出。
    Pdf,
    /// 仅日志。
    Log,
}

impl OutputFormat {
    /// 该格式下需要收集的产物扩展名（log 始终收集，用于 diff 与诊断）。
    fn extensions(self) -> &'static [&'static str] {
        match self {
            OutputFormat::Dvi => &["dvi", "log"],
            OutputFormat::Pdf => &["pdf", "log"],
            OutputFormat::Log => &["log"],
        }
    }
}

/// 交互模式（对应 TeX 的四种交互模式）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionMode {
    Batch,
    Nonstop,
    Scroll,
    ErrorStop,
}

impl InteractionMode {
    /// 对应的命令行 flag。
    pub fn flag(self) -> &'static str {
        match self {
            InteractionMode::Batch => "-interaction=batchmode",
            InteractionMode::Nonstop => "-interaction=nonstopmode",
            InteractionMode::Scroll => "-interaction=scrollmode",
            InteractionMode::ErrorStop => "-interaction=errorstopmode",
        }
    }
}

/// 一次编译请求。
#[derive(Debug, Clone)]
pub struct RunRequest {
    /// 主 `.tex` 文件路径。
    pub source: PathBuf,
    /// 工作目录：产物与辅助文件写入此处（调用方保证已存在）。
    pub working_dir: PathBuf,
    pub interaction: InteractionMode,
    pub format: OutputFormat,
}

/// 一次编译的结果。
#[derive(Debug, Clone)]
pub struct RunOutput {
    pub status: DriverStatus,
    /// 本次产生的产物文件（绝对路径）。
    pub produced: Vec<PathBuf>,
}

/// 驱动执行状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverStatus {
    /// 编译成功。
    Success,
    /// 编译失败（进程退出码非零 / 引擎自身报错）。
    Failure { code: Option<i32> },
    /// 引擎尚未实现（占位驱动返回，工具链应视为"跳过"而非"失败"）。
    NotImplemented,
}

/// 引擎驱动：抽象"任何能编译 TeX 输入的东西"。
pub trait EngineDriver: fmt::Debug {
    /// 驱动名（用于报告与日志）。
    fn name(&self) -> &str;

    /// 执行一次编译请求。
    fn run(&self, request: &RunRequest) -> Result<RunOutput>;
}

/// 占位驱动：立即返回 [`DriverStatus::NotImplemented`]。
///
/// 用途：M0 验证工具链管路与 CI 冒烟；引擎本体就绪后由真实驱动替换。
#[derive(Debug, Clone)]
pub struct StubDriver {
    pub name: String,
}

impl Default for StubDriver {
    fn default() -> Self {
        Self {
            name: "stub".to_owned(),
        }
    }
}

impl EngineDriver for StubDriver {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(&self, _request: &RunRequest) -> Result<RunOutput> {
        Ok(RunOutput {
            status: DriverStatus::NotImplemented,
            produced: Vec::new(),
        })
    }
}

/// 外部进程驱动：调用系统 TeX 引擎（pdflatex/xelatex/...）作为参考实现。
#[derive(Debug, Clone)]
pub struct ExternalDriver {
    /// 可执行程序名（或路径）。
    pub program: String,
    /// 附加命令行参数（追加在标准参数之后、源文件之前）。
    pub extra_args: Vec<String>,
    /// 显示名（默认取 program 名）。
    pub name: String,
}

impl ExternalDriver {
    /// 以默认参数创建外部驱动。
    pub fn new(program: impl Into<String>) -> Self {
        let program = program.into();
        Self {
            program: program.clone(),
            extra_args: Vec::new(),
            name: program,
        }
    }

    /// 追加额外参数。
    #[must_use]
    pub fn with_args(mut self, args: &[&str]) -> Self {
        self.extra_args = args.iter().map(|s| (*s).to_owned()).collect();
        self
    }
}

impl EngineDriver for ExternalDriver {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(&self, request: &RunRequest) -> Result<RunOutput> {
        let mut cmd = Command::new(&self.program);
        cmd.args([
            "-output-directory",
            request.working_dir.to_str().unwrap_or_default(),
        ])
        .arg(request.interaction.flag())
        .args(&self.extra_args)
        .arg(&request.source)
        .current_dir(&request.working_dir);

        let output = cmd
            .output()
            .with_context(|| format!("无法启动外部驱动 {}", self.program))?;

        let status = if output.status.success() {
            DriverStatus::Success
        } else {
            DriverStatus::Failure {
                code: output.status.code(),
            }
        };

        let produced = collect_artifacts(&request.working_dir, request.format.extensions())?;
        Ok(RunOutput { status, produced })
    }
}

/// 收集工作目录中符合扩展名集合的产物文件（排序保证确定性）。
fn collect_artifacts(dir: &Path, extensions: &[&str]) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("读取产物目录失败：{}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        if extensions.contains(&ext) && path.is_file() {
            found.push(path);
        }
    }
    found.sort();
    Ok(found)
}

/// 本引擎驱动：in-process 运行 [`ntex_layout::Typesetter`]，把引擎转录
/// （版本横幅 + 运行结果/首个错误）写入 `.log`/`.typ` 产物。
///
/// TRIP/ETRIP 需要逐 token 转录（`\show`/`\message`/错误上下文与恢复）——
/// 引擎目前是"首个错误即停"，本驱动先跑通管线、让 diff 展示引擎停在哪、
/// 差多远，随后按 diff 迭代补齐转录能力。
#[derive(Debug, Clone)]
pub struct NtexDriver {
    pub name: String,
}

impl Default for NtexDriver {
    fn default() -> Self {
        Self {
            name: "ntex".to_owned(),
        }
    }
}

impl EngineDriver for NtexDriver {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(&self, request: &RunRequest) -> Result<RunOutput> {
        let source = std::fs::read(&request.source)
            .with_context(|| format!("读取源文件失败：{}", request.source.display()))?;
        let base = request
            .source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("input")
            .to_owned();

        let mut log = String::new();
        log.push_str("This is NTex, Version 0.1.0 (TRIP/ETRIP pipeline v1)\n");
        log.push_str(&format!("(input: {})\n", request.source.display()));

        let mut ts = ntex_layout::Typesetter::new();
        let run = ts.typeset_bytes(source);
        // 终端转录（\message/\show/\showthe/\write16）→ .log 与 .typ 共用
        let transcript = ts.take_transcript();
        log.push_str(&transcript);
        if !transcript.is_empty() && !transcript.ends_with('\n') {
            log.push('\n');
        }
        let (status, produced) = match run {
            Ok(_) => {
                log.push_str("Engine: run completed.\n");
                (
                    DriverStatus::Success,
                    vec![format!("{base}.log"), format!("{base}.typ")],
                )
            }
            Err(e) => {
                log.push_str(&format!("Engine error: {e}\n"));
                // 首个错误即停：产物保留以便 diff 展示差距；状态标记失败
                (
                    DriverStatus::Failure { code: None },
                    vec![format!("{base}.log"), format!("{base}.typ")],
                )
            }
        };

        fs::write(request.working_dir.join(format!("{base}.log")), &log)
            .with_context(|| "写入 .log 产物失败")?;
        // .typ（终端转录）：\message/\show 等累积文本
        fs::write(request.working_dir.join(format!("{base}.typ")), &transcript)
            .with_context(|| "写入 .typ 产物失败")?;

        Ok(RunOutput {
            status,
            produced: produced
                .into_iter()
                .map(|f| request.working_dir.join(f))
                .collect(),
        })
    }
}

/// 从 CLI 规格字符串构造驱动：
/// - `stub` → 占位驱动
/// - `ntex` → 本引擎驱动（in-process Typesetter）
/// - `pdflatex` / `external=pdflatex` → 外部驱动
pub fn build_driver(spec: &str) -> Result<Box<dyn EngineDriver>> {
    let spec = spec.trim();
    if spec == "stub" {
        return Ok(Box::<StubDriver>::default());
    }
    if spec == "ntex" {
        return Ok(Box::<NtexDriver>::default());
    }
    let program = spec.strip_prefix("external=").unwrap_or(spec);
    if program.is_empty() {
        anyhow::bail!("驱动规格无效：{spec:?}（应为 stub、ntex 或程序名）");
    }
    Ok(Box::new(ExternalDriver::new(program)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_driver_returns_not_implemented() {
        let driver = StubDriver::default();
        assert_eq!(driver.name(), "stub");
        let req = RunRequest {
            source: PathBuf::from("x.tex"),
            working_dir: PathBuf::from("."),
            interaction: InteractionMode::Batch,
            format: OutputFormat::Log,
        };
        let out = driver.run(&req).unwrap();
        assert_eq!(out.status, DriverStatus::NotImplemented);
        assert!(out.produced.is_empty());
    }

    #[test]
    fn interaction_flags_map_to_tex_flags() {
        assert_eq!(InteractionMode::Batch.flag(), "-interaction=batchmode");
        assert_eq!(
            InteractionMode::ErrorStop.flag(),
            "-interaction=errorstopmode"
        );
    }

    #[test]
    fn build_driver_parses_specs() {
        assert!(build_driver("stub").unwrap().name() == "stub");
        assert!(build_driver("external=pdflatex").unwrap().name() == "pdflatex");
        assert!(build_driver("pdflatex").unwrap().name() == "pdflatex");
        assert!(build_driver("").is_err());
    }

    #[test]
    fn artifact_collection_ignores_dirs_and_unrelated_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.log"), "log").unwrap();
        std::fs::write(dir.path().join("b.dvi"), "dvi").unwrap();
        std::fs::write(dir.path().join("c.tex"), "tex").unwrap();
        std::fs::create_dir(dir.path().join("d.log")).unwrap();

        let artifacts = collect_artifacts(dir.path(), &["dvi", "log"]).unwrap();
        let names: Vec<String> = artifacts
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.log", "b.dvi"]);
    }
}
