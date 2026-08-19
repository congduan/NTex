//! # ntex-test-support
//!
//! NTex 测试/差分/基准共用的基础设施：
//!
//! - [`driver`]：**引擎驱动**抽象——TRIP 框架、差分测试、基准全部面向
//!   [`EngineDriver`](driver::EngineDriver) 编程。引擎本体（ntex-cli）就绪后，
//!   只需新增一个进程内驱动即可接入全部工具链，无需改动工具逻辑。
//! - [`diff`]：日志归一化（剔除时间戳/版本横幅等非确定性内容）与文本 diff。
//!
//! M0 阶段内置两种驱动，ETRIP 冲刺新增本引擎驱动：
//! - [`StubDriver`](driver::StubDriver)：立即返回 `NotImplemented`，用于验证工具链
//!   管路与 CI 冒烟（保证"引擎未就绪时基础设施仍可运行、可测试"）；
//! - [`ExternalDriver`](driver::ExternalDriver)：调用系统 TeX 引擎
//!   （pdflatex/xelatex 等）作为参考实现；
//! - [`NtexDriver`](driver::NtexDriver)：本引擎（in-process Typesetter），
//!   TRIP/ETRIP 管线接入点。

#![deny(unsafe_code)]

pub mod diff;
pub mod driver;

pub use driver::{
    build_driver, DriverStatus, EngineDriver, ExternalDriver, InteractionMode, NtexDriver,
    OutputFormat, RunOutput, RunRequest, StubDriver,
};
