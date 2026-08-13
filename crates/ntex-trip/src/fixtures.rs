//! TRIP fixtures 的定位与就绪检查。
//!
//! TRIP（Knuth 官方一致性测试）需要三个文件：
//! - `trip.tex`：测试源码（可 `kpsewhich trip.tex` 或从 CTAN 获取）；
//! - `trip.typ`：参考文本输出；
//! - `trip.log`：参考日志。
//!
//! fixtures 缺失时框架返回 `Skipped` 而非失败，保证 CI 在未配置网络/TeX 的环境下仍绿。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// TRIP fixtures 集合。
#[derive(Debug, Clone)]
pub struct TripFixtures {
    dir: PathBuf,
}

impl TripFixtures {
    /// 定位 fixtures：优先 `--fixtures` 覆盖，否则 workspace 根下的 `fixtures/trip`。
    pub fn locate(override_dir: Option<&Path>) -> Result<Self> {
        let dir = match override_dir {
            Some(dir) => dir.to_path_buf(),
            None => workspace_root()?.join("fixtures").join("trip"),
        };
        Ok(Self { dir })
    }

    /// fixtures 是否齐全（trip.tex / trip.typ / trip.log 都在）。
    pub fn ready(&self) -> bool {
        self.trip_tex().is_file() && self.trip_typ().is_file() && self.trip_log().is_file()
    }

    pub fn trip_tex(&self) -> PathBuf {
        self.dir.join("trip.tex")
    }

    pub fn trip_typ(&self) -> PathBuf {
        self.dir.join("trip.typ")
    }

    pub fn trip_log(&self) -> PathBuf {
        self.dir.join("trip.log")
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
        let f = TripFixtures::locate(Some(Path::new("/tmp/xyz"))).unwrap();
        assert_eq!(f.trip_tex(), Path::new("/tmp/xyz").join("trip.tex"));
        assert!(!f.ready());
    }
}
