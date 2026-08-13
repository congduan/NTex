//! 统一的错误类型。
//!
//! 设计约束：
//! - 引擎 core 层只依赖 `thiserror`，不引入 anyhow 等上层工具（工具链 crate 用 anyhow）；
//! - 刻意**不**提供 `From<io::Error>` 的 blanket 实现，要求调用方在构造处补上
//!   "操作意图 + 路径"上下文，避免错误信息丢失。

use std::fmt;
use std::io;
use std::path::PathBuf;

/// NTex 核心错误。
#[derive(Debug)]
pub enum Error {
    /// I/O 失败，附带操作意图与目标路径。
    Io {
        op: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    /// 输入非法（M1 起由输入层/展开层产生）。
    InvalidInput { message: String },
    /// 内部不变量被破坏（bug 保护，表示引擎自身缺陷而非用户输入问题）。
    Internal { message: String },
}

impl Error {
    /// 构造带上下文的 I/O 错误。
    pub fn io(op: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            op,
            path: path.into(),
            source,
        }
    }

    /// 构造非法输入错误。
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::InvalidInput {
            message: message.into(),
        }
    }

    /// 构造内部不变量错误。
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { op, path, .. } => write!(f, "{op} 失败：{}", path.display()),
            Error::InvalidInput { message } => write!(f, "非法输入：{message}"),
            Error::Internal { message } => write!(f, "内部错误：{message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// NTex 核心 Result 别名。
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;
    use std::io;

    #[test]
    fn io_error_display_includes_op_and_path() {
        let err = Error::io(
            "读取",
            "/tmp/x.tex",
            io::Error::new(io::ErrorKind::NotFound, "no such file"),
        );
        let text = err.to_string();
        assert!(text.contains("读取"));
        assert!(text.contains("失败"));
        assert!(text.contains("/tmp/x.tex"));
        assert!(err.source().is_some());
    }

    #[test]
    fn invalid_input_and_internal_errors() {
        let e = Error::invalid_input("bad catcode");
        assert_eq!(e.to_string(), "非法输入：bad catcode");
        assert!(e.source().is_none());

        let e = Error::internal("invariant broken");
        assert_eq!(e.to_string(), "内部错误：invariant broken");
    }
}
