//! ntex-pkg：宏包解析与取料（M9 生态冲刺 · **宏包管理**）。
//!
//! **定位**：实现 plan.md §6.2 第 8、9 条拍下的资产边界中的 **T2 全量可用层**——
//! 不内置任何宏包，一律按需解析、按需取料、显式锁定。
//!
//! ## 分层职责（与 §6.2 第 9 条一一对应）
//!
//! ```text
//! 契约层   ntex.lock（包名 + revision + container-sha512）  →  lock.rs
//! 解析层   只认 TLPDB 的 包 / 文件清单 / 依赖表             →  tlpdb.rs + resolve.rs
//! 取料层   四源可插拔（本地树 → tlnet → CTAN → 离线归档）    →  source.rs
//! 落地层   内容寻址缓存路径 + 校验和比对                    →  cache.rs
//! ```
//!
//! **为什么不直连 CTAN 作解析依据**（§6.2 第 9 条）：CTAN 的 `FILES.byname` 只有
//! 文件名 → 路径（25MB、每日重建、**无校验和、无依赖图、无版本号**）。从
//! `\usepackage{graphicx}` 追问「哪个包真正提供 `graphicx.sty`、它又依赖谁」，
//! 只有 TLPDB 能回答。
//!
//! ## 副作用纪律（RFC-3）
//!
//! 本 crate **自身不联网、不写盘**：文件访问一律经 [`ntex_io::Vfs`]，取料动作经
//! [`source::PackageSource`]。这样它既能在 CLI 里跑，也能被 WASM 工作台（`MemVfs` +
//! 宿主注入）复用——与 `ntex-wasm` 的 `set_bundle` 是同一套副作用边界。
//!
//! ## 当前实现边界（未实现项已登记 docs/KNOWN-SIMPLIFICATIONS.md）
//!
//! **已实现**：TLPDB 解析与文件名反查索引、`\usepackage` → 包解析、依赖闭包、
//! `.lock` 契约确定性编解码、缓存路径布局、本地 TeX Live 树源探测。
//!
//! **未实现**：tlnet / CTAN / 离线归档三源的取料（trait 插口已在，调用返回
//! [`Error::SourceNotImplemented`]）；SHA-512 字节级校验（需新依赖，见 [`cache`] 模块文档）。

pub mod cache;
pub mod lock;
pub mod resolve;
pub mod source;
#[cfg(test)]
pub(crate) mod testdata;
pub mod tlpdb;

/// ntex-pkg 错误模型。
///
/// 约定（同 `ntex-core`）：输入可达路径禁止 `unwrap` / `expect` / `panic`；
/// 错误一律带操作意图或定位上下文，**不提供 blanket `From<io::Error>`**。
///
/// `PackageNotFound` / `FileNotProvided` 带 `hint` 字段——它是 §6.2 第 8 条
/// 「缺包即报 + 一条命令补」的载体：报错时直接把可复制的补包命令交给用户，
/// 而不是让用户自己去找包名。
#[derive(Debug)]
pub enum Error {
    /// TLPDB 解析失败：带 1 基行号与字段名。
    Tlpdb {
        line: usize,
        field: &'static str,
        detail: String,
    },
    /// 锁文件损坏：带 1 基行号。
    Lock { line: usize, detail: String },
    /// 包在 TLPDB 中不存在。
    PackageNotFound { name: String, hint: String },
    /// 文件名在所有包的文件清单中都没有提供者。
    FileNotProvided { file: String, hint: String },
    /// 取料源尚未实现（§6.2 第 9 条刻意留的插口，**不静默降级**）。
    SourceNotImplemented { kind: &'static str, package: String },
    /// 校验和不匹配（锁定值与实际值不一致）。
    ChecksumMismatch {
        package: String,
        expected: String,
        actual: String,
    },
    /// 名称不合法：拼接缓存路径等场景下会造成越界（`..`、`/`、绝对路径）。
    UnsafeName { name: String, context: &'static str },
    /// 包缺少锁定所需的元数据（如无 `container-sha512`）——不可锁定即不可复现。
    NotLockable { package: String, reason: String },
    /// 输入文件不存在或不可读（`intent` 说明这次读它在做什么）。
    MissingFile { path: String, intent: &'static str },
    /// 文件 IO：`intent` 说明这次读写在做什么（不做 blanket From）。
    Io {
        intent: &'static str,
        source: std::io::Error,
    },
}

impl Error {
    /// 构造带意图上下文的 IO 错误。
    pub fn io(intent: &'static str, source: std::io::Error) -> Self {
        Error::Io { intent, source }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Tlpdb {
                line,
                field,
                detail,
            } => write!(f, "TLPDB 解析失败（l.{line}，字段 `{field}`）：{detail}"),
            Error::Lock { line, detail } => write!(f, "锁文件解析失败（l.{line}）：{detail}"),
            Error::PackageNotFound { name, hint } => {
                write!(f, "宏包 `{name}` 不在 TLPDB 中。{hint}")
            }
            Error::FileNotProvided { file, hint } => {
                write!(f, "文件名 `{file}` 没有提供者。{hint}")
            }
            Error::SourceNotImplemented { kind, package } => write!(
                f,
                "取料源 `{kind}` 尚未实现（包 `{package}`）；当前仅支持本地 TeX Live 树源"
            ),
            Error::ChecksumMismatch {
                package,
                expected,
                actual,
            } => write!(
                f,
                "包 `{package}` 校验和不匹配：锁定 {expected}，实际 {actual}"
            ),
            Error::UnsafeName { name, context } => {
                write!(
                    f,
                    "名称 `{name}` 不合法（{context}）：禁止 `..`、`/` 与隐藏名"
                )
            }
            Error::NotLockable { package, reason } => {
                write!(f, "包 `{package}` 不可锁定：{reason}")
            }
            Error::MissingFile { path, intent } => {
                write!(f, "文件不存在或不可读（{intent}）：{path}")
            }
            Error::Io { intent, source } => write!(f, "IO 失败（{intent}）：{source}"),
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

/// 原始字符串路径拼接（不做平台 `Path` 语义解析——与 `ntex-io` 的口径一致，WASM 友好）。
///
/// 前段末尾有 `/` 则不再补；空前段直接返回后段。
pub fn join_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        return name.to_owned();
    }
    if prefix.ends_with('/') {
        format!("{prefix}{name}")
    } else {
        format!("{prefix}/{name}")
    }
}

/// 取原始字符串路径的最后一段（basename）。
pub fn basename(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_path_handles_trailing_slash() {
        assert_eq!(join_path("/a/b", "c"), "/a/b/c");
        assert_eq!(join_path("/a/b/", "c"), "/a/b/c");
        assert_eq!(join_path("", "c"), "c");
    }

    #[test]
    fn basename_splits_last_segment() {
        assert_eq!(
            basename("texmf-dist/tex/latex/geometry/geometry.sty"),
            "geometry.sty"
        );
        assert_eq!(basename("geometry.sty"), "geometry.sty");
    }

    #[test]
    fn io_error_display_carries_intent() {
        let e = Error::io("读取 TLPDB", std::io::Error::other("boom"));
        assert!(e.to_string().contains("读取 TLPDB"));
    }
}
