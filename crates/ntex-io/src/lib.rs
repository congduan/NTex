//! ntex-io：虚拟文件系统（RFC-3 副作用模型）。
//!
//! 排版内核（ntex-core）不直接触碰 `std::fs`——读（`\input`/`\openin`/`\read`）
//! 与写（`\write`/`\openout`）全部经 [`Vfs`] trait。后端可互换：
//! 本地文件（[`LocalVfs`]）、内存（[`MemVfs`]，测试 / WASM 用）。

use std::collections::HashMap;
use std::io::{self, Write};

/// 虚拟文件系统：排版内核与真实/虚拟后端的唯一文件接口。
///
/// 约定：
/// - 路径为原始字符串（不做平台 Path 语义解析，WASM 友好）；
/// - [`Vfs::read`] 对不存在的文件返回 `Ok(None)`（TeX `\openin` 语义：不报错）；
/// - [`Vfs::write`] 覆盖写（TeX `\openout` 语义）。
pub trait Vfs: std::fmt::Debug + std::any::Any {
    /// 读整个文件；不存在返回 `Ok(None)`。
    fn read(&mut self, path: &str) -> io::Result<Option<Vec<u8>>>;
    /// 写整个文件（覆盖）。
    fn write(&mut self, path: &str, bytes: &[u8]) -> io::Result<()>;
    /// 追加写。
    fn append(&mut self, path: &str, bytes: &[u8]) -> io::Result<()>;
    /// 类型擦除互转（测试断言写入内容用）。
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

/// 本地文件系统后端（默认）。
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalVfs;

impl Vfs for LocalVfs {
    fn read(&mut self, path: &str) -> io::Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
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

/// 内存虚拟文件系统：测试 / WASM 用。
#[derive(Debug, Default, Clone)]
pub struct MemVfs {
    files: HashMap<String, Vec<u8>>,
}

impl MemVfs {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注入初始文件。
    pub fn insert(&mut self, path: impl Into<String>, bytes: impl Into<Vec<u8>>) {
        self.files.insert(path.into(), bytes.into());
    }

    /// 读取当前内容（测试断言用）。
    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(|v| v.as_slice())
    }
}

impl Vfs for MemVfs {
    fn read(&mut self, path: &str) -> io::Result<Option<Vec<u8>>> {
        Ok(self.files.get(path).cloned())
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> io::Result<()> {
        self.files.insert(path.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn append(&mut self, path: &str, bytes: &[u8]) -> io::Result<()> {
        let e = self.files.entry(path.to_owned()).or_default();
        e.extend_from_slice(bytes);
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_vfs_roundtrip() {
        let mut v = MemVfs::new();
        assert_eq!(v.read("a.txt").unwrap(), None);
        v.write("a.txt", b"hello").unwrap();
        assert_eq!(v.read("a.txt").unwrap(), Some(b"hello".to_vec()));
        v.append("a.txt", b" world").unwrap();
        assert_eq!(v.read("a.txt").unwrap(), Some(b"hello world".to_vec()));
    }

    #[test]
    fn mem_vfs_preseeded() {
        let mut v = MemVfs::new();
        v.insert("main.tex", b"\\input{ch1}");
        assert_eq!(v.read("main.tex").unwrap(), Some(b"\\input{ch1}".to_vec()));
    }

    #[test]
    fn local_vfs_missing_file_is_none() {
        let mut v = LocalVfs;
        let r = v.read("/nonexistent/ntex-io-test-file.txt");
        assert!(matches!(r, Ok(None)), "{r:?}");
    }
}
