//! TRIP / ETRIP fixtures 的定位与就绪检查。
//!
//! - TRIP（Knuth 官方一致性测试）：`trip.tex` / `trip.typ` / `trip.log`；
//! - ETRIP（e-TeX）：`etrip.tex` / `etrip.log`（web2c 参考输出）。
//!
//! fixtures 由 `scripts/fetch-trip-fixtures.sh` 获取（CTAN knuth dist + TeX Live
//! 源码镜像），缺失时框架返回 `Skipped` 而非失败，保证 CI 在未配置网络的环境下仍绿。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// 一致性测试种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestKind {
    /// TRIP（Knuth 官方 TeX 一致性测试）。
    Trip,
    /// ETRIP（e-TeX 一致性测试）。
    Etrip,
}

impl TestKind {
    pub fn name(self) -> &'static str {
        match self {
            TestKind::Trip => "TRIP",
            TestKind::Etrip => "ETRIP",
        }
    }

    /// fixtures 子目录。
    pub fn dir(self) -> &'static str {
        match self {
            TestKind::Trip => "trip",
            TestKind::Etrip => "etrip",
        }
    }

    /// 测试源码文件名（如 `trip.tex`）。
    pub fn tex_name(self) -> &'static str {
        match self {
            TestKind::Trip => "trip.tex",
            TestKind::Etrip => "etrip.tex",
        }
    }

    /// 需要比对输出、存在的参考文件（相对 fixtures 目录）。
    pub fn reference_files(self) -> &'static [&'static str] {
        match self {
            TestKind::Trip => &["trip.typ", "trip.log"],
            TestKind::Etrip => &["etrip.log"],
        }
    }
}

/// 一致性测试 fixtures 集合（按种类定位）。
#[derive(Debug, Clone)]
pub struct TestFixtures {
    dir: PathBuf,
    kind: TestKind,
}

impl TestFixtures {
    /// 定位 fixtures：优先 `--fixtures` 覆盖（其下再按 kind 子目录），
    /// 否则 workspace 根下的 `fixtures/<kind>`。
    pub fn locate(kind: TestKind, override_dir: Option<&Path>) -> Result<Self> {
        let dir = match override_dir {
            Some(dir) => dir.join(kind.dir()),
            None => workspace_root()?.join("fixtures").join(kind.dir()),
        };
        Ok(Self { dir, kind })
    }

    pub fn kind(&self) -> TestKind {
        self.kind
    }

    /// fixtures 是否齐全（源码 + 全部参考文件都在）。
    pub fn ready(&self) -> bool {
        self.tex().is_file()
            && self
                .kind
                .reference_files()
                .iter()
                .all(|f| self.dir.join(f).is_file())
    }

    /// 测试源码路径。
    pub fn tex(&self) -> PathBuf {
        self.dir.join(self.kind.tex_name())
    }

    /// 参考日志路径。
    pub fn log(&self) -> PathBuf {
        self.dir.join(format!("{}.log", self.kind_name()))
    }

    /// 参考终端输出路径（仅 TRIP 有；ETRIP 返回 None）。
    pub fn typ(&self) -> Option<PathBuf> {
        (self.kind == TestKind::Trip).then(|| self.dir.join("trip.typ"))
    }

    fn kind_name(&self) -> &'static str {
        match self.kind {
            TestKind::Trip => "trip",
            TestKind::Etrip => "etrip",
        }
    }
}

/// 定位 workspace 根（`NTex/`）：当前 crate 的 manifest 目录向上两级。
fn workspace_root() -> Result<PathBuf> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let root = Path::new(manifest_dir)
        .parent()
        .and_then(Path::parent)
        .context("无法定位 workspace 根目录")?;
    Ok(root.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_root_points_to_repo_root() {
        let root = workspace_root().unwrap();
        // workspace 根下应存在 Cargo.toml
        assert!(
            root.join("Cargo.toml").is_file(),
            "root: {}",
            root.display()
        );
    }

    #[test]
    fn locate_uses_override_when_given() {
        for kind in [TestKind::Trip, TestKind::Etrip] {
            let f = TestFixtures::locate(kind, Some(Path::new("/tmp/xyz"))).unwrap();
            assert_eq!(
                f.tex(),
                Path::new("/tmp/xyz").join(kind.dir()).join(kind.tex_name())
            );
            assert!(!f.ready());
        }
    }

    #[test]
    fn etrip_has_no_typ_reference() {
        let f = TestFixtures::locate(TestKind::Etrip, Some(Path::new("/tmp/xyz"))).unwrap();
        assert!(f.typ().is_none());
        let t = TestFixtures::locate(TestKind::Trip, Some(Path::new("/tmp/xyz"))).unwrap();
        assert_eq!(t.typ().unwrap().file_name().unwrap(), "trip.typ");
    }
}
