//! 取料层 ②（tlnet 镜像）：**下载 → 校验 → 解包 → 落地**成可当 TL 树根用的缓存树。
//!
//! ## 位置（plan.md §6.2 第 9 条）
//!
//! 四源可插拔里的第二源。与 ① 本地 TeX Live 树的关系是**互补**而非替代：
//! ① 零下载但只覆盖「本机已装的那一档」，② 能取到与 TLPDB 同代的精确版本。
//!
//! ## 三步都是"可信"的
//!
//! 1. **URL 由 TLPDB 唯一决定**：`<仓库>/archive/<包名>.r<revision>.tar.xz`。
//!    revision 缺失即拒绝（不可锁定就不可复现，不猜"最新"）。
//! 2. **字节由 SHA-512 兜底**：TLPDB 的 `containerchecksum` 就是整包 `.tar.xz` 的
//!    SHA-512（2026-09-19 实测 `infwarerr.r79461` / `graphics-def.r76719` 逐字匹配），
//!    校验通过才允许解包。这一层是 RFC-3 管不到的：副作用隔离能保证"引擎不写外部
//!    世界"，管不了"下载回来的包被换过"。
//! 3. **路径由 TLPDB 裁剪**：只取该包声明的 `runfiles`，容器里的其余条目
//!    （`tlpkg/tlpobj/*`、doc、source）一律丢弃——不把整个容器灌进资产树。
//!
//! ## 边缘（edge）与逻辑（logic）分离
//!
//! 「字节怎么来」（HTTP/TLS）与「容器怎么解」（xz/tar）是**边缘**，归
//! [`ContainerIo`]；URL 构造、校验和比对、TDS 重定位、条目裁剪是**逻辑**，
//! 在本模块。于是逻辑可以完全离线测试（测试注入假 io），
//! 也不需要把 reqwest / xz2 这类重依赖拖进 ntex-pkg。
//!
//! 默认的边缘实现是 [`HostContainerIo`]（调宿主 `curl` / `tar`）——与 `ntex-dvi`
//! 把 `kpsewhich` 当最后一层搜索链是同一手法：**外部可执行文件只当传输适配器**，
//! 判定与校验始终在 Rust 侧。见 docs/KNOWN-SIMPLIFICATIONS.md。

use std::path::{Path, PathBuf};

use ntex_io::Vfs;

use crate::cache::{self, sha512_hex};
use crate::resolve::Closure;
use crate::source::{FetchOutcome, LocalStatus, PackageSource, SourceKind};
use crate::tlpdb::{normalize_path, TlPackage, TlPdb};
use crate::{join_path, Error};

/// 容器内的一个文件条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// 容器内路径（TDS 相对，如 `tex/latex/graphics-def/pdftex.def`）。
    pub path: String,
    /// 文件字节。
    pub bytes: Vec<u8>,
}

impl ArchiveEntry {
    /// 路径里是否残留 `RELOC/` / `texmf-dist/` 前缀（归一化漏了才会为真）。
    ///
    /// 归一化是"资产/缓存路径可直接用"的前提，故做成可断言的谓词。
    pub fn has_prefix_leak(&self) -> bool {
        self.path.starts_with("RELOC/") || self.path.starts_with("texmf-dist/")
    }
}

/// 取料**边缘**：字节从哪来、容器怎么解。
///
/// 抽成 trait 是为了让 URL 构造 / 校验 / 裁剪这层逻辑可以离线测试，
/// 并把「要不要引 HTTP 客户端」这个决定留在实现侧。
pub trait ContainerIo: std::fmt::Debug {
    /// 取回 `url` 的原始字节（实现负责 TLS、重定向、超时与错误映射）。
    fn fetch(&mut self, url: &str) -> Result<Vec<u8>, Error>;

    /// 解开一个 `.tar.xz` 容器，返回其中的文件条目（顺序不作保证）。
    fn unpack_tar_xz(&mut self, container: &[u8]) -> Result<Vec<ArchiveEntry>, Error>;
}

/// 默认边缘实现：宿主 `curl` + `tar`。
///
/// 选它而不引 HTTP 客户端，是为了不让 ntex-pkg 的依赖面从 1 个涨到几十个
/// （TLS 栈没法手写）。`curl` / `tar` 在 macOS/Linux 上恒有；缺失时给出
/// **可判读的**错误（哪个命令、哪个 URL），不静默失败。
#[derive(Debug, Clone)]
pub struct HostContainerIo {
    /// `curl` 可执行名（可用绝对路径覆盖）。
    pub curl: String,
    /// `tar` 可执行名。
    pub tar: String,
    /// 单个容器的下载超时（秒）。
    pub timeout_secs: u64,
}

impl Default for HostContainerIo {
    fn default() -> Self {
        Self {
            curl: "curl".to_owned(),
            tar: "tar".to_owned(),
            timeout_secs: 300,
        }
    }
}

impl HostContainerIo {
    /// 起一个临时目录（不引 tempfile：进程内唯一的名字即可，用完即删）。
    fn temp_dir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        p.push(format!("ntex-pkg-{tag}-{}-{nanos}", std::process::id()));
        p
    }
}

/// 递归收集目录下的文件（相对 `base` 的路径 + 字节），顺序确定（按名排序）。
fn collect_dir(base: &Path, dir: &Path, out: &mut Vec<ArchiveEntry>) -> Result<(), Error> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::io("遍历解包目录", e))?;
    let mut paths: Vec<PathBuf> = Vec::new();
    for e in entries {
        let e = e.map_err(|e| Error::io("读取解包目录项", e))?;
        paths.push(e.path());
    }
    // `read_dir` 的顺序未定义——排序后输出才可复现。
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_dir(base, &path, out)?;
            continue;
        }
        let rel = path
            .strip_prefix(base)
            .map_err(|e| {
                Error::io(
                    "解包路径不在临时目录内",
                    std::io::Error::other(e.to_string()),
                )
            })?
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = std::fs::read(&path).map_err(|e| Error::io("读解包出的文件", e))?;
        out.push(ArchiveEntry { path: rel, bytes });
    }
    Ok(())
}

impl ContainerIo for HostContainerIo {
    fn fetch(&mut self, url: &str) -> Result<Vec<u8>, Error> {
        let out = std::process::Command::new(&self.curl)
            .arg("-sSL") // 静默但保留错误信息 + 跟随重定向
            .arg("--fail") // HTTP >= 400 直接失败，别把错误页当内容
            .arg("--max-time")
            .arg(self.timeout_secs.to_string())
            .arg(url)
            .output()
            .map_err(|e| {
                Error::io(
                    "启动 curl 下载 tlnet 容器",
                    std::io::Error::other(format!("{}：{e}", self.curl)),
                )
            })?;
        if !out.status.success() {
            return Err(Error::io(
                "下载 tlnet 容器",
                std::io::Error::other(format!(
                    "curl 退出码 {:?}（{url}）：{}",
                    out.status.code(),
                    String::from_utf8_lossy(&out.stderr).trim()
                )),
            ));
        }
        Ok(out.stdout)
    }

    fn unpack_tar_xz(&mut self, container: &[u8]) -> Result<Vec<ArchiveEntry>, Error> {
        let dir = Self::temp_dir("unpack");
        std::fs::create_dir_all(&dir).map_err(|e| Error::io("建临时解包目录", e))?;
        let archive = dir.join("container.tar.xz");
        std::fs::write(&archive, container).map_err(|e| Error::io("写临时容器文件", e))?;

        let out = std::process::Command::new(&self.tar)
            .arg("-xJf")
            .arg(&archive)
            .arg("-C")
            .arg(&dir)
            .output()
            .map_err(|e| {
                Error::io(
                    "启动 tar 解包容器",
                    std::io::Error::other(format!("{}：{e}", self.tar)),
                )
            })?;
        if !out.status.success() {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(Error::io(
                "解包 tlnet 容器",
                std::io::Error::other(format!(
                    "tar 退出码 {:?}：{}",
                    out.status.code(),
                    String::from_utf8_lossy(&out.stderr).trim()
                )),
            ));
        }

        let mut entries = Vec::new();
        let collected = collect_dir(&dir, &dir, &mut entries);
        // 容器文件本身也躺在临时目录里，别把它当成条目。
        entries.retain(|e| e.path != "container.tar.xz");
        let _ = std::fs::remove_dir_all(&dir);
        collected?;
        Ok(entries)
    }
}

/// ② tlnet 镜像源。
///
/// `repo` 是 tlnet 根（如 `https://mirrors.aliyun.com/CTAN/systems/texlive/tlnet`），
/// 不含 `archive/` 段——那句由 [`Self::container_url`] 拼，避免两处各写一半。
#[derive(Debug)]
pub struct TlnetSource {
    repo: String,
    io: Box<dyn ContainerIo>,
}

impl TlnetSource {
    /// 以镜像根 + 边缘实现构造。
    pub fn new(repo: impl Into<String>, io: Box<dyn ContainerIo>) -> Self {
        Self {
            repo: repo.into(),
            io,
        }
    }

    /// 仓库根（原样，未规整尾斜杠）。
    pub fn repo(&self) -> &str {
        &self.repo
    }

    /// 容器的 URL：`<仓库>/archive/<包名>.r<revision>.tar.xz`。
    ///
    /// **无 revision 即拒绝**：tlnet 的文件名带 revision，没有它就没有确定的
    /// URL，也就无法复现——这正是 plan.md §6.2 第 9 条"可锁定"的含义。
    pub fn container_url(&self, pkg: &TlPackage) -> Result<String, Error> {
        let Some(revision) = pkg.revision else {
            return Err(Error::NotLockable {
                package: pkg.name.clone(),
                reason: "TLPDB 未给出 revision，无法拼出确定版本的 tlnet 容器 URL".to_owned(),
            });
        };
        let base = self.repo.trim_end_matches('/');
        Ok(format!("{base}/archive/{}.r{revision}.tar.xz", pkg.name))
    }

    /// 下载并**校验**容器字节（SHA-512 对 TLPDB 的 `containerchecksum`）。
    pub fn fetch_verified(&mut self, pkg: &TlPackage) -> Result<Vec<u8>, Error> {
        let Some(expected) = pkg.container_sha512.clone() else {
            return Err(Error::NotLockable {
                package: pkg.name.clone(),
                reason: "TLPDB 未给出 container-sha512，无法校验下载内容".to_owned(),
            });
        };
        let url = self.container_url(pkg)?;
        let bytes = self.io.fetch(&url)?;
        let actual = sha512_hex(&bytes);
        cache::check_sha512(&pkg.name, &expected, &actual)?;
        Ok(bytes)
    }

    /// 下载 → 校验 → 解包 → **裁剪到 TLPDB 声明的 runfiles**。
    ///
    /// 返回 `(TDS 相对路径, 字节)`，路径已去 `RELOC/` / `texmf-dist/` 前缀，
    /// 可直接作为资产树或缓存树内的相对路径。
    ///
    /// 容器里声明了却找不到的文件**报错**（[`Error::MissingFile`]）——静默少一个
    /// 文件正是"资产看起来齐全、实际缺依赖"的病根。
    pub fn fetch_run_files(&mut self, pkg: &TlPackage) -> Result<Vec<ArchiveEntry>, Error> {
        let container = self.fetch_verified(pkg)?;
        let entries = self.io.unpack_tar_xz(&container)?;
        // 容器内路径 → 字节。键统一归一化，免得 `./`、反斜杠等写法差异漏匹配。
        let mut index: std::collections::BTreeMap<String, &ArchiveEntry> =
            std::collections::BTreeMap::new();
        for e in &entries {
            index.insert(normalize_path(&e.path), e);
        }

        let mut out = Vec::with_capacity(pkg.run_files.len());
        for rel in &pkg.run_files {
            let tds = normalize_path(rel);
            match index.get(&tds) {
                Some(e) => out.push(ArchiveEntry {
                    path: tds,
                    bytes: e.bytes.clone(),
                }),
                None => {
                    return Err(Error::MissingFile {
                        path: format!("{} 容器内缺 {}", pkg.name, tds),
                        intent: "从 tlnet 容器中取该包的运行面文件",
                    });
                }
            }
        }
        Ok(out)
    }
}

impl PackageSource for TlnetSource {
    fn kind(&self) -> SourceKind {
        SourceKind::Tlnet
    }

    fn is_available(&self, _vfs: &mut dyn Vfs) -> bool {
        // **不联网探测**（trait 契约）：仓库串非空即视为"配置可用"，
        // 真正能不能连上由 fetch 阶段如实报错。
        !self.repo.trim().is_empty()
    }

    fn local_status(&self, _pkg: &TlPackage, _vfs: &mut dyn Vfs) -> Option<LocalStatus> {
        None
    }

    fn fetch_container(
        &mut self,
        package: &str,
        _vfs: &mut dyn Vfs,
    ) -> Result<FetchOutcome, Error> {
        // 该 trait 方法只拿得到包名，拼不出含 revision 的 URL——需要 `TlPackage`
        // 的调用方请走 [`TlnetSource::fetch_verified`]。这里显式说明，不猜。
        Err(Error::NotLockable {
            package: package.to_owned(),
            reason: "tlnet 取料需要 TlPackage（revision + container-sha512）；\
                     请用 TlnetSource::fetch_verified"
                .to_owned(),
        })
    }
}

/// [`materialize`] 的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaterializeReport {
    /// 取到的容器数。
    pub containers: usize,
    /// 落地的文件数。
    pub files: usize,
    /// 落地字节数。
    pub bytes: u64,
    /// 只有 binfiles、没有运行面文本文件的包（计入容器但不落地文件）。
    pub text_free: Vec<String>,
}

/// 把整个闭包从 tlnet 取到本地，铺成一棵**可当 TL 树根用**的缓存树。
///
/// 落地布局与已安装树同构：
/// ```text
/// <cache>/tlpkg/texlive.tlpdb      ← 同一份 TLPDB（缓存自描述，可再过 lock/check）
/// <cache>/texmf-dist/tex/...       ← 运行面文件
/// ```
/// 于是取完可以直接：
/// `LocalTexLiveSource::new(<cache>)` → `vendor` 铺进资产目录。
/// 这条路把"下载"与"物化"解耦：下载只需正确一次，物化可反复跑。
pub fn materialize(
    source: &mut TlnetSource,
    db: &TlPdb,
    closure: &Closure,
    cache_root: &str,
    tlpdb_text: &str,
    vfs: &mut dyn Vfs,
) -> Result<MaterializeReport, Error> {
    let mut report = MaterializeReport::default();

    for name in &closure.packages {
        let Some(pkg) = db.get(name) else {
            return Err(Error::PackageNotFound {
                name: name.clone(),
                hint: "闭包中的包不在该 TLPDB 中——闭包与库不匹配".to_owned(),
            });
        };
        if pkg.run_files.is_empty() {
            // 纯 binfiles 的记录（如 `epstopdf.<arch>`）：没有运行面文本文件要落地，
            // 计入容器但写 0 个文件。不是错误——但也不假装取了料。
            report.containers += 1;
            report.text_free.push(name.clone());
            continue;
        }
        for entry in source.fetch_run_files(pkg)? {
            let dst = join_path(cache_root, &join_path("texmf-dist", &entry.path));
            if let Some(parent) = parent_dir(&dst) {
                vfs.create_dir_all(parent)
                    .map_err(|e| Error::io("创建缓存目录", e))?;
            }
            vfs.write(&dst, &entry.bytes)
                .map_err(|e| Error::io("写入缓存文件", e))?;
            report.files += 1;
            report.bytes += entry.bytes.len() as u64;
        }
        report.containers += 1;
    }

    // 缓存树自描述：TLPDB 一起落地，`LocalTexLiveSource::is_available` 才认它。
    let tlpdb_dst = join_path(cache_root, "tlpkg/texlive.tlpdb");
    if let Some(parent) = parent_dir(&tlpdb_dst) {
        vfs.create_dir_all(parent)
            .map_err(|e| Error::io("创建缓存 tlpkg 目录", e))?;
    }
    vfs.write(&tlpdb_dst, tlpdb_text.as_bytes())
        .map_err(|e| Error::io("写入缓存 TLPDB", e))?;

    Ok(report)
}

/// 取原始字符串路径的父目录。
fn parent_dir(path: &str) -> Option<&str> {
    path.rfind('/')
        .map(|i| &path[..i])
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    use super::*;
    use crate::resolve::{closure_for_requires, RequireKind};
    use crate::source::LocalTexLiveSource;
    use crate::testdata;
    use ntex_io::MemVfs;

    /// 假边缘：按 URL 返回预置容器，解包返回预置条目。
    ///
    /// **只看 URL、不看真实 tar 字节**——协议逻辑（URL 构造、校验、裁剪）是本模块
    /// 的测试对象；xz/tar 解包属于被替换掉的边缘。
    #[derive(Debug, Default)]
    struct FakeIo {
        /// URL → 容器字节。
        containers: BTreeMap<String, Vec<u8>>,
        /// 容器字节 → 条目。
        unpacked: BTreeMap<Vec<u8>, Vec<ArchiveEntry>>,
        /// 被请求过的 URL（外部持有同一份 `Rc` 以便断言）。
        requested: Rc<RefCell<Vec<String>>>,
    }

    impl ContainerIo for FakeIo {
        fn fetch(&mut self, url: &str) -> Result<Vec<u8>, Error> {
            self.requested.borrow_mut().push(url.to_owned());
            match self.containers.get(url) {
                Some(b) => Ok(b.clone()),
                None => Err(Error::io(
                    "下载 tlnet 容器",
                    std::io::Error::other(format!("假边缘未预置 {url}")),
                )),
            }
        }

        fn unpack_tar_xz(&mut self, container: &[u8]) -> Result<Vec<ArchiveEntry>, Error> {
            Ok(self.unpacked.get(container).cloned().unwrap_or_default())
        }
    }

    /// 造一个**自洽**的假 tlnet：覆盖 fixture 里**每一个有运行面文件的包**
    /// （闭包会牵出 `pdftex.universal-darwin` 一类间接依赖，漏了就会在物化时才炸）。
    ///
    /// 容器字节 = `<container 包名>`；条目 = 该包声明的 runfiles
    /// （`drop_in` 指定的可从容器里删掉，以模拟"声明了却没打进去"）；
    /// 并把真 SHA-512 **写入**（无则新增）TLPDB 文本——fixture 里的校验和是占位值，
    /// 且有若干记录刻意不带校验和，必须补齐才能形成自洽的取料前提。
    fn fake_tlnet(
        pkgs: &[&str],
        drop_in: &[(&str, &str)],
    ) -> (TlnetSource, TlPdb, Rc<RefCell<Vec<String>>>) {
        let base = TlPdb::parse(testdata::MINI_TLPDB).expect("fixture 应能解析");
        let mut text = testdata::MINI_TLPDB.to_owned();
        let mut io = FakeIo::default();
        let requested = Rc::clone(&io.requested);

        for (name, pkg) in base.iter() {
            if !pkgs.is_empty() && !pkgs.contains(&name) {
                continue;
            }
            if pkg.run_files.is_empty() {
                continue;
            }
            // 无 revision 的包在 tlnet 里没有确定的 URL（`container_url` 会拒绝），
            // 故不为它造容器——夹具里 `base`/`expl3` 就是这类记录。
            let Some(rev) = pkg.revision else {
                continue;
            };
            let container = format!("<container {name}>").into_bytes();
            let entries: Vec<ArchiveEntry> = pkg
                .run_files
                .iter()
                .map(|rel| normalize_path(rel))
                .filter(|p| !drop_in.iter().any(|(n, d)| *n == name && p.ends_with(*d)))
                .map(|p| ArchiveEntry {
                    bytes: format!("% {name} {p}").into_bytes(),
                    path: p,
                })
                .collect();
            io.containers.insert(
                format!("https://m/archive/{name}.r{rev}.tar.xz"),
                container.clone(),
            );
            io.unpacked.insert(container.clone(), entries);
            text = upsert_field(&text, name, "containerchecksum", &sha512_hex(&container));
        }

        let db = TlPdb::parse(&text).expect("回填校验和后应能解析");
        (
            TlnetSource {
                repo: "https://m".to_owned(),
                io: Box::new(io),
            },
            db,
            requested,
        )
    }

    fn rev_of(db: &TlPdb, name: &str) -> u64 {
        db.get(name).and_then(|p| p.revision).unwrap_or(0)
    }

    /// 设置 TLPDB 文本中某包的某字段：**有则替换、无则新增**（插在 `name` 行之后）。
    ///
    /// 插在 `name` 之后是安全的：非缩进行会结束上一个文件清单段，而 `name` 本身
    /// 不开段。原有的同名字段行会被丢弃，避免同字段出现两次。
    fn upsert_field(text: &str, pkg: &str, field: &str, value: &str) -> String {
        let prefix = format!("{field} ");
        let mut out = String::with_capacity(text.len() + 160);
        let mut in_pkg = false;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("name ") {
                in_pkg = rest.trim() == pkg;
                out.push_str(line);
                out.push('\n');
                if in_pkg {
                    out.push_str(&format!("{field} {value}\n"));
                }
                continue;
            }
            if in_pkg && line.starts_with(&prefix) {
                continue; // 旧值丢掉（新值已写在 name 之后）
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    #[test]
    fn container_url_is_pinned_by_revision() {
        let db = TlPdb::parse(testdata::MINI_TLPDB).expect("fixture");
        let src = TlnetSource::new("https://m/", Box::new(FakeIo::default()));
        let pkg = db.get("geometry").expect("fixture 应含 geometry");
        assert_eq!(
            src.container_url(pkg).expect("有 revision 应能拼 URL"),
            format!(
                "https://m/archive/geometry.r{}.tar.xz",
                rev_of(&db, "geometry")
            ),
            "尾斜杠应被规整，且用 r<revision> 命名"
        );
    }

    #[test]
    fn container_url_without_revision_is_refused() {
        let db = TlPdb::parse(testdata::MINI_TLPDB).expect("fixture");
        let mut pkg = db.get("geometry").cloned().expect("fixture");
        pkg.revision = None;
        let src = TlnetSource::new("https://m", Box::new(FakeIo::default()));
        match src.container_url(&pkg) {
            Err(Error::NotLockable { package, reason }) => {
                assert_eq!(package, "geometry");
                assert!(reason.contains("revision"), "理由应点名 revision：{reason}");
            }
            other => panic!("无 revision 应拒绝，实际 {other:?}"),
        }
    }

    #[test]
    fn fetch_without_checksum_is_refused() {
        let (mut src, db, _) = fake_tlnet(&["geometry"], &[]);
        let mut pkg = db.get("geometry").cloned().expect("fixture");
        pkg.container_sha512 = None;
        match src.fetch_verified(&pkg) {
            Err(Error::NotLockable { package, reason }) => {
                assert_eq!(package, "geometry");
                assert!(reason.contains("sha512"), "理由应点名校验和：{reason}");
            }
            other => panic!("无校验和应拒绝，实际 {other:?}"),
        }
    }

    #[test]
    fn checksum_mismatch_is_reported() {
        // 预置的容器字节与 TLPDB 里声明的 sha512 对不上（未回填真值）。
        let db = TlPdb::parse(testdata::MINI_TLPDB).expect("fixture");
        let mut io = FakeIo::default();
        let rev = rev_of(&db, "geometry");
        io.containers.insert(
            format!("https://m/archive/geometry.r{rev}.tar.xz"),
            b"<tampered>".to_vec(),
        );
        let mut src = TlnetSource::new("https://m", Box::new(io));
        let pkg = db.get("geometry").expect("fixture");
        match src.fetch_verified(pkg) {
            Err(Error::ChecksumMismatch { package, .. }) => assert_eq!(package, "geometry"),
            other => panic!("篡改的容器必须被拒，实际 {other:?}"),
        }
    }

    #[test]
    fn fetch_run_files_crops_to_declared_runfiles_and_strips_prefixes() {
        let (mut src, db, requested) = fake_tlnet(&["geometry"], &[]);
        let pkg = db.get("geometry").expect("fixture");
        let files = src.fetch_run_files(pkg).expect("应取到运行面文件");

        assert_eq!(
            files.len(),
            pkg.run_files.len(),
            "条目数应等于 TLPDB 声明的 runfiles 数（不含容器里的 tlpkg/ 杂物）"
        );
        assert!(
            files.iter().all(|f| !f.has_prefix_leak()),
            "路径应已归一化：不该残留 RELOC/ 或 texmf-dist/ 前缀"
        );
        // URL 必须按 revision 钉死，且只请求一次。
        assert_eq!(
            requested.borrow().as_slice(),
            [format!(
                "https://m/archive/geometry.r{}.tar.xz",
                rev_of(&db, "geometry")
            )]
        );
    }

    #[test]
    fn missing_runfile_in_container_is_an_error_not_a_silent_skip() {
        let (mut src, db, _) = fake_tlnet(&["geometry"], &[("geometry", "geometry.sty")]);
        let pkg = db.get("geometry").expect("fixture");
        match src.fetch_run_files(pkg) {
            Err(Error::MissingFile { path, .. }) => {
                assert!(path.contains("geometry"), "错误应点名包：{path}");
                assert!(path.contains("geometry.sty"), "错误应点名具体文件：{path}");
            }
            other => panic!("容器缺文件必须报错，实际 {other:?}"),
        }
    }

    #[test]
    fn materialize_builds_a_tree_that_local_source_accepts() {
        // 空名单 = 覆盖 fixture 里全部有运行面文件的包——`geometry` 的闭包会牵出
        // `pdftex.universal-darwin`（间接依赖），只备 geometry 会在物化时才炸。
        let (mut src, db, _) = fake_tlnet(&[], &[]);
        let reqs = vec![("geometry".to_owned(), RequireKind::UsePackage)];
        let closure = closure_for_requires(&db, &reqs, "universal-darwin").expect("闭包应可解析");
        assert!(
            closure.packages.len() > 1,
            "闭包应含间接依赖（否则本测试覆盖不到物化的循环）"
        );

        let mut vfs = MemVfs::new();
        let report = materialize(
            &mut src,
            &db,
            &closure,
            ".ntex/pkgs/tlnet",
            testdata::MINI_TLPDB,
            &mut vfs,
        )
        .expect("物化应成功");
        assert!(report.files > 0, "应落地文件");
        assert_eq!(report.containers, closure.packages.len());

        // 缓存树必须是"能被 LocalTexLiveSource 认作 TL 树"的形态。
        let local = LocalTexLiveSource::new(".ntex/pkgs/tlnet");
        assert!(
            local.is_available(&mut vfs),
            "缓存树应含 tlpkg/texlive.tlpdb"
        );

        // 且每个落地文件都能按 TLPDB 声明的路径读到（路径映射闭环）。
        let pkg = db.get("geometry").expect("fixture");
        for rel in &pkg.run_files {
            let path = local.file_path(rel);
            assert!(
                vfs.read(&path).expect("读缓存应成功").is_some(),
                "缓存里应能按 TLPDB 路径读到 {path}"
            );
        }
    }

    #[test]
    fn package_source_contract_is_honest() {
        let (src, db, _) = fake_tlnet(&["geometry"], &[]);
        let mut vfs = MemVfs::new();
        assert!(matches!(src.kind(), SourceKind::Tlnet));
        assert!(
            src.is_available(&mut vfs),
            "仓库串非空即配置可用（不联网探测）"
        );
        assert!(
            !TlnetSource::new("   ", Box::new(FakeIo::default())).is_available(&mut vfs),
            "空仓库串应判为不可用"
        );
        // `local_status` 对网络源恒为 None（"本地已有"这个概念不适用）。
        let pkg = db.get("geometry").expect("fixture");
        assert!(src.local_status(pkg, &mut vfs).is_none());
        // `fetch_container` 只有包名，拼不出 revision 定的 URL → 显式拒绝，不猜。
        let mut src2 = TlnetSource::new("https://m", Box::new(FakeIo::default()));
        assert!(matches!(
            src2.fetch_container("geometry", &mut vfs),
            Err(Error::NotLockable { .. })
        ));
    }
}
