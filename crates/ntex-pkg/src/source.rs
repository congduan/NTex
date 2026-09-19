//! 取料层：四源可插拔（plan.md §6.2 第 9 条「取料层」）。
//!
//! 源不是「哪个能用」，而是**按优先级依次试**，且优先级由三件事决定：
//! 需不需要联网、有没有校验和、能不能锁定版本。
//!
//! | # | 源 | 联网 | 校验和 | 版本 | 定位 |
//! |---|---|---|---|---|---|
//! | ① | [`SourceKind::LocalTexLive`] | 否 | 由 TL 树自身保证 | 本机年度 | 已有 TeX Live 就直接用 |
//! | ② | [`SourceKind::Tlnet`] | 是 | SHA-512 + GPG 签名 | 有 revision | 主源 |
//! | ③ | [`SourceKind::Ctan`] | 是 | **无** | **无** | 回落：不可达 / 未收录 / 需旧版 |
//! | ④ | [`SourceKind::OfflineArchive`] | 否 | 由归档提供方保证 | 归档快照 | 内网 / air-gapped（opt-in） |
//!
//! **三态取料结果**（[`FetchOutcome`]）：取到容器 / 该源不提供容器 / 该源未实现。
//! 刻意不把后两者压成同一个「失败」——「本地树不需要容器」是正常情况，
//! 「tlnet 取料还没写」是欠账，二者混起来会让人误以为功能已就绪。
//!
//! **本模块不联网**：`is_available` 只做本地探测；[`PackageSource::fetch_container`]
//! 的实现者才可能联网，而本轮只有 ① 落地（②③④ 显式返回
//! [`FetchOutcome::Unimplemented`]，见 docs/KNOWN-SIMPLIFICATIONS.md）。

use ntex_io::Vfs;

use crate::tlpdb::TlPackage;
use crate::Error;

/// 取料源种类。**声明顺序即优先级**（小者先试）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceKind {
    /// ① 本机已有的 TeX Live 树：零下载、零风险（`\input` 搜索链第 7 项已在做）。
    LocalTexLive,
    /// ② TeX Live `tlnet` 仓库：SHA-512 + GPG 签名，且带 `revision` 可锁定。
    Tlnet,
    /// ③ CTAN 镜像的 `install/**.tds.zip`：TDS 单包、无需 tree prefix，
    ///    但**无校验和、无版本号**，故只能作回落。
    Ctan,
    /// ④ 离线归档包：内网 / air-gapped 场景，opt-in。
    OfflineArchive,
}

impl SourceKind {
    /// 短名（报告用）。
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::LocalTexLive => "local-texlive",
            SourceKind::Tlnet => "tlnet",
            SourceKind::Ctan => "ctan",
            SourceKind::OfflineArchive => "offline-archive",
        }
    }

    /// 中文说明（CLI/报错用）。
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::LocalTexLive => "本地 TeX Live 树",
            SourceKind::Tlnet => "tlnet 镜像",
            SourceKind::Ctan => "CTAN install 包",
            SourceKind::OfflineArchive => "离线归档包",
        }
    }

    /// 是否网络源。
    pub fn is_network(self) -> bool {
        matches!(self, SourceKind::Tlnet | SourceKind::Ctan)
    }

    /// 该源是否自带校验和（能支撑锁定）。
    pub fn has_checksums(self) -> bool {
        matches!(self, SourceKind::Tlnet | SourceKind::OfflineArchive)
    }

    /// 按优先级排列的四源。
    pub fn all() -> [SourceKind; 4] {
        [
            SourceKind::LocalTexLive,
            SourceKind::Tlnet,
            SourceKind::Ctan,
            SourceKind::OfflineArchive,
        ]
    }
}

/// 某包在一棵本地 TeX Live 树里的存在情况。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalStatus {
    /// 本地树根（原始字符串）。
    pub root: String,
    /// 已在本地存在的运行面文件（相对 TL 根的路径）。
    pub present: Vec<String>,
    /// 本地缺失的运行面文件。
    pub missing: Vec<String>,
}

impl LocalStatus {
    /// 运行面文件是否齐全。
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty() && !self.present.is_empty()
    }

    /// 覆盖率（0.0~1.0）；包没有运行面文件时返回 0.0。
    pub fn coverage(&self) -> f64 {
        let total = self.present.len() + self.missing.len();
        if total == 0 {
            0.0
        } else {
            self.present.len() as f64 / total as f64
        }
    }
}

/// 取料结果：三态（见模块文档）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    /// 取到了容器字节（`.tar.xz`）。
    Container(Vec<u8>),
    /// 该源不提供容器（如本地树已直接提供文件）。
    NotApplicable,
    /// 该源尚未实现——**显式**，不静默降级。
    Unimplemented,
}

/// 取料源接口。
pub trait PackageSource: std::fmt::Debug {
    /// 源种类。
    fn kind(&self) -> SourceKind;

    /// 该源是否可用。**不得联网**（只用本地信息判断）。
    fn is_available(&self, vfs: &mut dyn Vfs) -> bool;

    /// 本地已有情况；网络源一律返回 `None`。
    fn local_status(&self, pkg: &TlPackage, vfs: &mut dyn Vfs) -> Option<LocalStatus>;

    /// 取容器字节。
    fn fetch_container(&mut self, package: &str, vfs: &mut dyn Vfs) -> Result<FetchOutcome, Error>;
}

/// ① 本地 TeX Live 树源。
///
/// 语义是「文件已经在本机」——它不下载任何东西，因此 [`Self::fetch_container`]
/// 返回 [`FetchOutcome::NotApplicable`]，取料走 [`LocalTexLiveSource::read_file`]。
///
/// **代价说明**：NTex 的 [`Vfs`] 没有 `exists`，探测只能靠「尝试读」，
/// 故 [`Self::probe`] 对包内每个运行面文件各读一次。大包（如 `l3kernel`）探测有
/// IO 代价，快速存在性探测待 VFS 侧补 `exists` 后再优化，
/// 见 docs/KNOWN-SIMPLIFICATIONS.md。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalTexLiveSource {
    root: String,
}

impl LocalTexLiveSource {
    /// 以某棵 TL 树根构造（例：`/usr/local/texlive/2024basic`）。
    pub fn new(root: impl Into<String>) -> Self {
        Self { root: root.into() }
    }

    /// 树根。
    pub fn root(&self) -> &str {
        &self.root
    }

    /// 运行面文件在本机的绝对路径（原始字符串形态，不做平台 Path 解析）。
    ///
    /// 入参是 **TLPDB 的运行面原串**，经 [`crate::tlpdb::install_rel_path`] 映射：
    /// `RELOC/tex/a.sty`（tlnet 库写法）→ `<root>/texmf-dist/tex/a.sty`；
    /// `texmf-dist/tex/a.sty`（已安装库写法）→ 原样。两种库因此共用同一条路径规则。
    pub fn file_path(&self, rel_path: &str) -> String {
        crate::join_path(&self.root, &crate::tlpdb::install_rel_path(rel_path))
    }

    /// 探测某包运行面文件的本机存在情况。
    pub fn probe(&self, pkg: &TlPackage, vfs: &mut dyn Vfs) -> LocalStatus {
        let mut status = LocalStatus {
            root: self.root.clone(),
            present: Vec::with_capacity(pkg.run_files.len()),
            missing: Vec::new(),
        };
        for rel in &pkg.run_files {
            let path = self.file_path(rel);
            match vfs.read(&path) {
                Ok(Some(_)) => status.present.push(rel.clone()),
                Ok(None) => status.missing.push(rel.clone()),
                // 读失败（权限/IO）按「缺失」处理，但不吞掉原因：路径进 missing 供人工判读
                Err(_) => status.missing.push(rel.clone()),
            }
        }
        status
    }

    /// 读出一个运行面文件；不存在返回 `Ok(None)`。
    pub fn read_file(&self, rel_path: &str, vfs: &mut dyn Vfs) -> Result<Option<Vec<u8>>, Error> {
        let path = self.file_path(rel_path);
        vfs.read(&path)
            .map_err(|e| Error::io("读取本地 TeX Live 树文件", e))
    }
}

impl PackageSource for LocalTexLiveSource {
    fn kind(&self) -> SourceKind {
        SourceKind::LocalTexLive
    }

    fn is_available(&self, vfs: &mut dyn Vfs) -> bool {
        // 只探一个约定路径：树根下有 tlpkg/ 即视为一棵 TL 树（不联网）
        let probe = crate::join_path(&self.root, "tlpkg/texlive.tlpdb");
        matches!(vfs.read(&probe), Ok(Some(_)))
    }

    fn local_status(&self, pkg: &TlPackage, vfs: &mut dyn Vfs) -> Option<LocalStatus> {
        Some(self.probe(pkg, vfs))
    }

    fn fetch_container(
        &mut self,
        _package: &str,
        _vfs: &mut dyn Vfs,
    ) -> Result<FetchOutcome, Error> {
        Ok(FetchOutcome::NotApplicable)
    }
}

/// ②③④ 源的占位实现：**显式未实现**，绝不用空实现假装可用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnimplementedSource {
    kind: SourceKind,
    location: String,
}

impl UnimplementedSource {
    /// 以种类与位置（URL / 目录）构造。
    pub fn new(kind: SourceKind, location: impl Into<String>) -> Self {
        Self {
            kind,
            location: location.into(),
        }
    }

    /// 配置的位置。
    pub fn location(&self) -> &str {
        &self.location
    }
}

impl PackageSource for UnimplementedSource {
    fn kind(&self) -> SourceKind {
        self.kind
    }

    fn is_available(&self, _vfs: &mut dyn Vfs) -> bool {
        // 未实现的源一律不可用——绝不返回 true 让上层以为能取到
        false
    }

    fn local_status(&self, _pkg: &TlPackage, _vfs: &mut dyn Vfs) -> Option<LocalStatus> {
        None
    }

    fn fetch_container(
        &mut self,
        package: &str,
        _vfs: &mut dyn Vfs,
    ) -> Result<FetchOutcome, Error> {
        Err(Error::SourceNotImplemented {
            kind: self.kind.as_str(),
            package: package.to_owned(),
        })
    }
}

/// 一次源解算的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// 本地树已完整提供该包——零下载。
    LocalComplete(LocalStatus),
    /// 本地树只提供了一部分（需补齐），或全部源都取不到容器。
    LocalPartial(LocalStatus),
    /// 从某源取到了容器。
    Container {
        /// 来源。
        kind: SourceKind,
        /// 容器字节数。
        bytes: usize,
    },
    /// 没有任何源能提供（`tried` 记录试过哪些）。
    Unavailable {
        /// 试过的源，按尝试顺序。
        tried: Vec<SourceKind>,
    },
}

/// 按优先级串联的源链。
///
/// 解算顺序：本地树优先（能零下载就零下载），其次按 [`SourceKind`] 声明序。
/// **不做**任何隐式联网：网络源未实现时直接跳过并记账，返回
/// [`ResolveOutcome::Unavailable`] 或本地部分命中，由调用方决定是否提示用户。
#[derive(Debug, Default)]
pub struct SourceChain {
    sources: Vec<Box<dyn PackageSource>>,
}

impl SourceChain {
    /// 空链。
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一个源。
    pub fn push(&mut self, source: Box<dyn PackageSource>) -> &mut Self {
        self.sources.push(source);
        self
    }

    /// 已登记的源数。
    pub fn len(&self) -> usize {
        self.sources.len()
    }

    /// 是否为空链。
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// 可用源的种类（按优先级）。
    pub fn available_kinds(&self, vfs: &mut dyn Vfs) -> Vec<SourceKind> {
        let mut kinds: Vec<SourceKind> = self
            .sources
            .iter()
            .filter(|s| s.is_available(vfs))
            .map(|s| s.kind())
            .collect();
        kinds.sort();
        kinds
    }

    /// 解算一个包的取料路径。
    pub fn resolve(&mut self, pkg: &TlPackage, vfs: &mut dyn Vfs) -> Result<ResolveOutcome, Error> {
        let mut order: Vec<usize> = (0..self.sources.len()).collect();
        order.sort_by_key(|&i| self.sources[i].kind());

        let mut tried = Vec::new();
        let mut best_partial: Option<LocalStatus> = None;

        for i in order {
            let kind = self.sources[i].kind();
            if !self.sources[i].is_available(vfs) {
                continue;
            }
            tried.push(kind);

            if let Some(status) = self.sources[i].local_status(pkg, vfs) {
                if status.is_complete() {
                    return Ok(ResolveOutcome::LocalComplete(status));
                }
                if best_partial
                    .as_ref()
                    .map(|b| status.coverage() > b.coverage())
                    .unwrap_or(true)
                {
                    best_partial = Some(status);
                }
                continue;
            }

            match self.sources[i].fetch_container(&pkg.name, vfs)? {
                FetchOutcome::Container(bytes) => {
                    return Ok(ResolveOutcome::Container {
                        kind,
                        bytes: bytes.len(),
                    })
                }
                FetchOutcome::NotApplicable | FetchOutcome::Unimplemented => continue,
            }
        }

        Ok(match best_partial {
            Some(status) => ResolveOutcome::LocalPartial(status),
            None => ResolveOutcome::Unavailable { tried },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdata::mini;
    use ntex_io::MemVfs;

    /// 造一棵假的 TL 树：只有 tlpkg/texlive.tlpdb 与给定文件。
    fn fake_tree(files: &[&str]) -> MemVfs {
        let mut vfs = MemVfs::new();
        vfs.insert("/tl/tlpkg/texlive.tlpdb", b"name x\n".to_vec());
        for f in files {
            vfs.insert(format!("/tl/{f}"), b"content".to_vec());
        }
        vfs
    }

    #[test]
    fn source_kinds_are_ordered_by_priority_and_capability() {
        let kinds = SourceKind::all();
        assert_eq!(kinds[0], SourceKind::LocalTexLive, "本地树必须最先试");
        assert_eq!(kinds[3], SourceKind::OfflineArchive);
        assert!(!SourceKind::LocalTexLive.is_network());
        assert!(SourceKind::Tlnet.is_network());
        assert!(SourceKind::Ctan.is_network());
        assert!(SourceKind::Tlnet.has_checksums());
        assert!(SourceKind::OfflineArchive.has_checksums());
        assert!(
            !SourceKind::Ctan.has_checksums(),
            "CTAN 无校验和——这正是它只能作回落源的原因"
        );
    }

    #[test]
    fn local_source_availability_is_offline_probe() {
        let src = LocalTexLiveSource::new("/tl");
        assert!(src.is_available(&mut fake_tree(&[])));
        assert!(!src.is_available(&mut MemVfs::new()), "空树不可用");
        assert_eq!(src.root(), "/tl");
    }

    #[test]
    fn probe_separates_present_from_missing() {
        let db = mini();
        let geometry = db.get("geometry").expect("geometry 存在");
        let mut vfs = fake_tree(&["texmf-dist/tex/latex/geometry/geometry.sty"]);
        let src = LocalTexLiveSource::new("/tl");

        let status = src.probe(geometry, &mut vfs);
        assert_eq!(
            status.present,
            vec!["texmf-dist/tex/latex/geometry/geometry.sty"]
        );
        assert_eq!(status.missing.len(), 2, "另两个文件应记为缺失");
        assert!(!status.is_complete());
        assert!((status.coverage() - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn full_local_hit_is_complete_and_zero_download() {
        let db = mini();
        let geometry = db.get("geometry").expect("geometry 存在");
        let mut vfs = fake_tree(&[
            "texmf-dist/tex/latex/geometry/geometry.sty",
            "texmf-dist/doc/latex/geometry/geometry.pdf",
            "texmf-dist/tex/latex/geometry/geometry.cfg",
        ]);
        let mut chain = SourceChain::new();
        chain.push(Box::new(LocalTexLiveSource::new("/tl")));
        chain.push(Box::new(UnimplementedSource::new(
            SourceKind::Tlnet,
            "http://mirror.ctan.org/systems/texlive/tlnet",
        )));

        match chain.resolve(geometry, &mut vfs).expect("解算不得失败") {
            ResolveOutcome::LocalComplete(s) => {
                assert!(s.is_complete());
                assert_eq!(s.present.len(), 3);
            }
            other => panic!("应本地完整命中，实际 {other:?}"),
        }
    }

    #[test]
    fn local_native_source_never_returns_container() {
        let mut vfs = fake_tree(&[]);
        let mut src = LocalTexLiveSource::new("/tl");
        assert_eq!(
            src.fetch_container("geometry", &mut vfs)
                .expect("本地源不报错"),
            FetchOutcome::NotApplicable
        );
    }

    #[test]
    fn unimplemented_sources_are_explicit_not_silent() {
        let mut vfs = MemVfs::new();
        let mut src = UnimplementedSource::new(SourceKind::Tlnet, "http://example.invalid/tlnet");
        assert!(!src.is_available(&mut vfs), "未实现源不得声称可用");
        assert!(src.local_status(&dummy_pkg(), &mut vfs).is_none());
        match src.fetch_container("geometry", &mut vfs) {
            Err(Error::SourceNotImplemented { kind, package }) => {
                assert_eq!(kind, "tlnet");
                assert_eq!(package, "geometry");
            }
            other => panic!("应显式报未实现，实际 {other:?}"),
        }
    }

    fn dummy_pkg() -> TlPackage {
        TlPackage {
            name: "dummy".to_owned(),
            ..TlPackage::default()
        }
    }

    #[test]
    fn unavailable_when_no_source_can_provide() {
        let db = mini();
        let geometry = db.get("geometry").expect("geometry 存在");
        let mut vfs = fake_tree(&[]);
        let mut chain = SourceChain::new();
        chain.push(Box::new(LocalTexLiveSource::new("/tl")));

        match chain.resolve(geometry, &mut vfs).expect("解算不得失败") {
            ResolveOutcome::LocalPartial(s) => {
                assert!(s.present.is_empty(), "一个文件都没有");
                assert_eq!(s.missing.len(), 3);
                assert!(s.coverage() < 1e-9);
            }
            other => panic!("本地全缺时应报 LocalPartial，实际 {other:?}"),
        }
    }

    #[test]
    fn empty_chain_reports_unavailable_with_empty_tried() {
        let db = mini();
        let geometry = db.get("geometry").expect("geometry 存在");
        let mut vfs = MemVfs::new();
        let mut chain = SourceChain::new();
        assert!(chain.is_empty());
        assert_eq!(
            chain.resolve(geometry, &mut vfs).expect("解算不得失败"),
            ResolveOutcome::Unavailable { tried: Vec::new() }
        );
    }

    #[test]
    fn chain_tries_local_before_network_sources() {
        let mut vfs = fake_tree(&[]);
        let mut chain = SourceChain::new();
        // 故意先 push 网络源，验证解算会按优先级重排
        chain.push(Box::new(UnimplementedSource::new(
            SourceKind::Ctan,
            "https://mirror.example.invalid/tex-archive",
        )));
        chain.push(Box::new(LocalTexLiveSource::new("/tl")));
        assert_eq!(chain.len(), 2);
        assert_eq!(
            chain.available_kinds(&mut vfs),
            vec![SourceKind::LocalTexLive],
            "只有本地源可用（其余未实现）"
        );
    }
}
