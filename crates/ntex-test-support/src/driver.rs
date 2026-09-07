//! 引擎驱动抽象。
//!
//! TRIP 框架、差分测试、基准全部面向 [`EngineDriver`] 编程：
//! 上层工具只关心"发一次编译请求 → 拿回产物与状态"，不关心请求由谁执行。
//! 这使得引擎本体接入时零改动工具逻辑。

use std::fmt;
use std::fs;
use std::io::{self, Write};
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

/// 本地 VFS 包装：`read` 先试原路径（进程 cwd），失败回退工作目录
/// （in-process 驱动下 `\input tripos` 等相对路径按 TeX 语义在工作目录解析）。
#[derive(Debug)]
struct WorkDirVfs {
    wd: std::path::PathBuf,
}

impl ntex_io::Vfs for WorkDirVfs {
    fn read(&mut self, path: &str) -> io::Result<Option<Vec<u8>>> {
        if let Ok(bytes) = std::fs::read(path) {
            return Ok(Some(bytes));
        }
        match std::fs::read(self.wd.join(path)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(path, bytes)
    }

    fn append(&mut self, path: &str, bytes: &[u8]) -> io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        f.write_all(bytes)
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
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

        // TRIP/ETRIP 需要真实 TFM 度量 + 自动分页（统计行需要页数/DVI 字节数）。
        // \input 相对路径（tripos 等）在工作目录解析：注入回退 VFS。
        // DVI 产物与统计行（"Output written on ... (N pages, M bytes)."）需要
        // 自动分页语义（`\vsize` 断页 + 收尾冲页），与 `ntex-dvi` CLI 一致。
        let mut ts = ntex_layout::Typesetter::with_tfm_paginated();
        ts.set_vfs(Box::new(WorkDirVfs {
            wd: request.working_dir.clone(),
        }));
        let run = ts.typeset_dvi(&String::from_utf8_lossy(&source));
        // 终端转录（\message/\show/\showthe/\write16）。注意：TRIP/ETRIP 参考
        // （trip.log/etrip.log）是**预加载格式**路径的输出；pass1 是 initex 路径
        // （`\dump` 前），仅作驱动诊断，不进入最终 log/typ。pass1 转录先暂存，
        // 若发生 dump 则最终产物只取 pass2（格式路径）。
        let transcript = ts.take_transcript();
        // e-IniTeX 语义：`\dump` 后保存 fmt，再以该格式重跑同一源（\einitex 已定义
        // → 跳过前导，进入 ETRIP 测试体）；最终产物只保留 pass2 转录。
        let dumped = ts.dumped();
        let mut transcript2 = String::new();
        // (DVI 字节, 页数)：由**实际生成 DVI 的那个 typesetter**提供，两者必须同源
        // （dumped 路径 DVI 来自 pass2 的 ts2，若取 pass1 的 ts 页数必错——pass1
        // 在 \dump 处停止，没有页面）。
        let mut dvi: Option<(Vec<u8>, usize)> = None;
        let (status, produced) = if dumped {
            let mut buf = Vec::new();
            ntex_format::save(&mut buf, &ts.export_state())
                .with_context(|| "序列化 .fmt 快照失败")?;
            let fmt_path = request.working_dir.join(format!("{base}.fmt"));
            fs::write(&fmt_path, &buf).with_context(|| "写入 .fmt 产物失败")?;

            let mut ts2 = ntex_layout::Typesetter::with_tfm_paginated();
            ts2.set_vfs(Box::new(WorkDirVfs {
                wd: request.working_dir.clone(),
            }));
            let mut reader = &buf[..];
            let state = ntex_format::load(&mut reader).with_context(|| "加载 .fmt 快照失败")?;
            ts2.import_state(state);
            let run2 = ts2.typeset_dvi(&String::from_utf8_lossy(&source));
            transcript2 = ts2.take_transcript();
            // 调试辅助：NTEX_KEEP_TRANSCRIPT=<path> 保存 pass2 转录到文件
            if let Ok(path) = std::env::var("NTEX_KEEP_TRANSCRIPT") {
                let _ = std::fs::write(&path, &transcript2);
            }
            if let Err(e) = &run2 {
                let tail: String = transcript2
                    .chars()
                    .rev()
                    .take(200)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                eprintln!(
                    "[driver] pass2 error: {e} | section={} | transcript2 len={} first={:?} tail={:?}",
                    ts2.current_section(),
                    transcript2.len(),
                    transcript2.chars().take(80).collect::<String>(),
                    tail
                );
            }
            // 成功跑完才有 DVI 产物（TeX：中途出错则无 "Output written" 统计）。
            if run2.is_ok() {
                let fonts = ts2.fonts_snapshot();
                dvi = Some((
                    ntex_dvi::write_dvi_with_counts(
                        ts2.shipped_pages(),
                        ts2.shipped_page_counts(),
                        &fonts,
                    ),
                    ts2.shipped_pages().len(),
                ));
            }
            if !transcript2.is_empty() {
                if !log.ends_with('\n') {
                    log.push('\n');
                }
                log.push_str(&transcript2);
                if !transcript2.ends_with('\n') {
                    log.push('\n');
                }
            }
            let produced = vec![format!("{base}.log"), format!("{base}.typ")];
            match run2 {
                Ok(_) => (DriverStatus::Success, produced),
                Err(e) => {
                    log.push_str(&format!("Engine error: {e}\n"));
                    (DriverStatus::Failure { code: None }, produced)
                }
            }
        } else {
            let produced = vec![format!("{base}.log"), format!("{base}.typ")];
            match run {
                Ok(_) => {
                    if !transcript.is_empty() {
                        if !log.ends_with('\n') {
                            log.push('\n');
                        }
                        log.push_str(&transcript);
                        if !transcript.ends_with('\n') {
                            log.push('\n');
                        }
                    }
                    (DriverStatus::Success, produced)
                }
                Err(e) => {
                    eprintln!(
                        "[driver] pass1 error: {e} | section={}",
                        ts.current_section()
                    );
                    log.push_str(&format!("Engine error: {e}\n"));
                    (DriverStatus::Failure { code: None }, produced)
                }
            }
        };

        if status == DriverStatus::Success {
            if !log.is_empty() && !log.ends_with('\n') {
                log.push('\n');
            }
            match dvi {
                Some((bytes, pages)) => {
                    fs::write(request.working_dir.join(format!("{base}.dvi")), &bytes)
                        .with_context(|| "写入 .dvi 产物失败")?;
                    log.push_str(&format!(
                        "Output written on {base}.dvi ({pages} pages, {} bytes).\n",
                        bytes.len()
                    ));
                }
                None => log.push_str("No pages of output.\n"),
            }
        }

        fs::write(request.working_dir.join(format!("{base}.log")), &log)
            .with_context(|| "写入 .log 产物失败")?;
        // .typ（终端转录）：\message/\show 等累积文本（仅格式路径，pass1 不入 typ）。
        // 统计行只属于 .log（tex.web write_dvi 到 log 文件，终端无此行）——统计行
        // 只 push 进 log 变量，transcript2 从未含它，typ 无需任何剔除。
        let typ = if dumped { transcript2 } else { transcript };
        fs::write(request.working_dir.join(format!("{base}.typ")), &typ)
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
