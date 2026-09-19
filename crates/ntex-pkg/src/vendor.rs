//! 取料层 ① 的收尾：把**依赖闭包**物化成一棵 TDS 子树（或 dry-run 报差异）。
//!
//! ## 动机（plan.md §6.2 第 8/9 条）
//!
//! 发行资产集（`assets/tex-minimal/`）此前靠**手抄**若干目录维护
//! （见 `assets/tex-minimal/README.md` 的「更新方法」：若干条 `cp -R`）。
//! 手抄的失效模式不是「抄错」而是**「不知道漏了什么」**——依赖散在别的目录里，
//! 少一个包不会有任何提示。真实事故：`graphics-def`（含 `pdftex.def` 驱动）与
//! `graphics-cfg`（含 `graphics.cfg`/`color.cfg`）两个包不在
//! `tex/latex/graphics/` 下，于是 `\usepackage{graphicx}` 在 Tauri 里拿不到
//! 驱动文件——资产"看起来齐全"，实际缺依赖。
//!
//! 本模块把「资产集内容」定义成 **种子名单 → 依赖闭包 → 逐文件落地**，
//! 从机制上保证「内置名单 ≡ 闭包」（T1 精选层的判据），并让缺口**可见**：
//! 源树缺文件是 [`FileState::SourceMissing`]，不是静默跳过。
//!
//! ## 三态 + 一
//!
//! 逐文件比对源树（本机 TeX Live）与目标树（资产目录），得
//! [`FileState::Add`] / [`FileState::Identical`] / [`FileState::Differ`] /
//! [`FileState::SourceMissing`]。比对按**字节**（`ntex-io` 的 `Vfs` 没有哈希，
//! 与其引入第二种"相等"判据，不如直接比字节——资产体量在 MB 级，代价可接受）。

use std::fmt::Write as _;

use ntex_io::Vfs;

use crate::resolve::Closure;
use crate::source::LocalTexLiveSource;
use crate::tlpdb::{normalize_path, TlPdb};
use crate::{join_path, Error};

/// 单个文件相对目标树的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FileState {
    /// 源树有、目标树没有 → 需要写入。
    Add,
    /// 两边字节一致 → 无需动作。
    Identical,
    /// 两边都有但字节不同 → 需要刷新（通常是版本漂移）。
    Differ,
    /// **源树也没有**（TLPDB 声明了该运行面文件，本机树里却读不到）
    /// → 取料失败；必须显式报告，绝不当作"已完成"。
    SourceMissing,
}

impl FileState {
    /// 中文标签（CLI 输出与测试断言共用同一份措辞）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Add => "新增",
            Self::Identical => "一致",
            Self::Differ => "需刷新",
            Self::SourceMissing => "源缺失",
        }
    }
}

/// 计划中的一条：某个包的一个运行面文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    /// 提供该文件的 TL 包名。
    pub package: String,
    /// 目标树内的 TDS 相对路径（已去 `texmf-dist/` 前缀，如
    /// `tex/latex/graphics-def/pdftex.def`）——就是写进资产目录的路径。
    pub rel_path: String,
    /// 源树内的绝对路径（`<TL 根>/texmf-dist/tex/...`）。
    pub src_path: String,
    /// 状态。
    pub state: FileState,
    /// 源文件字节数（`SourceMissing` 时为 0）。
    pub bytes: u64,
}

/// 物化计划：闭包内全部运行面文件的逐文件差异。
#[derive(Debug, Clone, Default)]
pub struct VendorPlan {
    /// 逐文件条目，按（包名，路径）排序——输出确定性。
    pub files: Vec<PlannedFile>,
    /// 参与闭包的包名（已排序）。
    pub packages: Vec<String>,
    /// 目标树根（供报告回显）。
    pub target_root: String,
    /// 源树根。
    pub source_root: String,
}

impl VendorPlan {
    /// 处于某状态的条目数。
    pub fn count(&self, state: FileState) -> usize {
        self.files.iter().filter(|f| f.state == state).count()
    }

    /// 处于某状态的条目字节合计。
    pub fn bytes(&self, state: FileState) -> u64 {
        self.files
            .iter()
            .filter(|f| f.state == state)
            .map(|f| f.bytes)
            .sum()
    }

    /// 源树缺失的条目（**必须为空**才能宣称资产集完整）。
    pub fn source_missing(&self) -> Vec<&PlannedFile> {
        self.files
            .iter()
            .filter(|f| f.state == FileState::SourceMissing)
            .collect()
    }

    /// 需要写入的条目（`Add` + `Differ`）。
    pub fn pending(&self) -> Vec<&PlannedFile> {
        self.files
            .iter()
            .filter(|f| matches!(f.state, FileState::Add | FileState::Differ))
            .collect()
    }

    /// 计划是否已收敛：无待写、无源缺失。
    pub fn is_settled(&self) -> bool {
        self.pending().is_empty() && self.source_missing().is_empty()
    }

    /// 多行人类可读摘要（CLI 与测试共用口径）。
    pub fn summary(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "目标树 {} ← 源树 {}", self.target_root, self.source_root);
        let _ = writeln!(
            s,
            "  包 {} 个 · 运行面文件 {} 个",
            self.packages.len(),
            self.files.len()
        );
        let _ = writeln!(
            s,
            "  新增 {}（{}）· 需刷新 {}（{}）· 一致 {} · 源缺失 {}",
            self.count(FileState::Add),
            human_bytes(self.bytes(FileState::Add)),
            self.count(FileState::Differ),
            human_bytes(self.bytes(FileState::Differ)),
            self.count(FileState::Identical),
            self.count(FileState::SourceMissing),
        );
        s
    }
}

/// 逐文件比对，生成物化计划（**只读**：不建目录、不写文件）。
///
/// 遍历顺序是「闭包包名（`BTreeSet`，已排序）→ 包内 `run_files` 原始顺序」，
/// 故输出确定；单包内文件按 TLPDB 声明顺序，便于与官方清单对照。
pub fn plan(
    db: &TlPdb,
    closure: &Closure,
    source: &LocalTexLiveSource,
    target_root: &str,
    vfs: &mut dyn Vfs,
) -> Result<VendorPlan, Error> {
    let mut out = VendorPlan {
        files: Vec::new(),
        packages: closure.packages.iter().cloned().collect(),
        target_root: target_root.to_owned(),
        source_root: source.root().to_owned(),
    };

    for name in &out.packages {
        let Some(pkg) = db.get(name) else {
            // 闭包里的包必然来自本 TLPDB（`closure` 就是这么建的），走到这里
            // 说明调用方传了不属于该库的闭包——报出来，不假装没事。
            return Err(Error::PackageNotFound {
                name: name.clone(),
                hint: "闭包中的包不在该 TLPDB 中——闭包与库不匹配，勿混用两棵树的产物".to_owned(),
            });
        };
        for rel in &pkg.run_files {
            let src_path = source.file_path(rel);
            let dst_rel = normalize_path(rel);
            let dst_path = join_path(target_root, &dst_rel);

            let src = vfs
                .read(&src_path)
                .map_err(|e| Error::io("读取源树文件", e))?;
            let (state, bytes) = match src {
                None => (FileState::SourceMissing, 0),
                Some(src_bytes) => {
                    let bytes = src_bytes.len() as u64;
                    let dst = vfs
                        .read(&dst_path)
                        .map_err(|e| Error::io("读取目标树文件", e))?;
                    match dst {
                        None => (FileState::Add, bytes),
                        Some(dst_bytes) if dst_bytes == src_bytes => (FileState::Identical, bytes),
                        Some(_) => (FileState::Differ, bytes),
                    }
                }
            };
            out.files.push(PlannedFile {
                package: name.clone(),
                rel_path: dst_rel,
                src_path,
                state,
                bytes,
            });
        }
    }

    Ok(out)
}

/// 执行计划（`Add` + `Differ` 落盘；`Identical` 跳过；`SourceMissing` 只统计）。
///
/// 每个文件写前先 [`Vfs::create_dir_all`] 其父目录——**重复建目录是幂等的**，
/// 调用方不必自己去重目录（资产树目录数远小于文件数，这点冗余换实现简单）。
pub fn apply(
    plan: &VendorPlan,
    target_root: &str,
    vfs: &mut dyn Vfs,
) -> Result<VendorReport, Error> {
    let mut report = VendorReport {
        skipped_identical: plan.count(FileState::Identical),
        skipped_source_missing: plan.source_missing().len(),
        ..Default::default()
    };

    for f in plan.pending() {
        if let Some(parent) = parent_dir(&join_path(target_root, &f.rel_path)) {
            vfs.create_dir_all(parent)
                .map_err(|e| Error::io("创建目标目录", e))?;
        }
        let src = vfs
            .read(&f.src_path)
            .map_err(|e| Error::io("读取源树文件", e))?;
        let Some(bytes) = src else {
            // 计划与执行之间源文件消失了（竞态）：**计入缺失**而不是写个空文件。
            report.source_missing_after_plan.push(f.rel_path.clone());
            continue;
        };
        let dst = join_path(target_root, &f.rel_path);
        vfs.write(&dst, &bytes)
            .map_err(|e| Error::io("写入目标树文件", e))?;
        report.written_files += 1;
        report.written_bytes += bytes.len() as u64;
        if f.state == FileState::Add {
            report.added += 1;
        } else {
            report.refreshed += 1;
        }
    }
    Ok(report)
}

/// 写入结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VendorReport {
    /// 新写入的文件数。
    pub added: usize,
    /// 覆盖刷新的文件数。
    pub refreshed: usize,
    /// 因源缺失而跳过的文件数（计划阶段判定）。
    pub skipped_source_missing: usize,
    /// 因源缺失而跳过的文件数（执行阶段才发现的竞态）。
    pub source_missing_after_plan: Vec<String>,
    /// 因两边一致而跳过的文件数。
    pub skipped_identical: usize,
    /// 实际写出的字节数。
    pub written_bytes: u64,
    /// 实际写出的文件数（`added + refreshed`）。
    pub written_files: usize,
}

/// 取原始字符串路径的父目录（`/` 之前的部分）。
///
/// 无 `/` 时返回 `None`（写当前目录，无需建目录）。
fn parent_dir(path: &str) -> Option<&str> {
    path.rfind('/')
        .map(|i| &path[..i])
        .filter(|s| !s.is_empty())
}

/// 人类可读字节（与 CLI `human_bytes` 同口径，避免两处格式漂移）。
fn human_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    let v = n as f64;
    if v >= KB * KB {
        format!("{:.1} MB", v / (KB * KB))
    } else if v >= KB {
        format!("{:.1} KB", v / KB)
    } else {
        format!("{n} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::{closure_for_requires, RequireKind};
    use crate::testdata;
    use ntex_io::MemVfs;

    /// 把 mini fixture 的包铺成"源树"里的 MemVfs 文件（路径按 TLPDB 声明）。
    fn source_vfs(db: &TlPdb, skip: &[(&str, &str)]) -> MemVfs {
        let mut vfs = MemVfs::new();
        vfs.insert("tl/tlpkg/texlive.tlpdb", b"");
        for (name, pkg) in db.iter() {
            for rel in &pkg.run_files {
                let skipped = skip
                    .iter()
                    .any(|(sname, srel)| *sname == name && rel.ends_with(srel));
                if !skipped {
                    vfs.insert(join_path("tl", rel), format!("% {name} {rel}").into_bytes());
                }
            }
        }
        vfs
    }

    fn mini_closure() -> (TlPdb, Closure) {
        let db = TlPdb::parse(testdata::MINI_TLPDB).expect("mini fixture 应能解析");
        let reqs = vec![("geometry".to_owned(), RequireKind::UsePackage)];
        let c = closure_for_requires(&db, &reqs, "universal-darwin").expect("闭包应可解析");
        (db, c)
    }

    #[test]
    fn fresh_target_is_all_adds() {
        let (db, c) = mini_closure();
        let mut vfs = source_vfs(&db, &[]);
        let src = LocalTexLiveSource::new("tl");
        let p = plan(&db, &c, &src, "assets", &mut vfs).expect("plan 应成功");

        assert_eq!(
            p.count(FileState::Add),
            p.files.len(),
            "空目标树应全部是新增"
        );
        assert_eq!(p.count(FileState::Identical), 0);
        assert_eq!(
            p.count(FileState::SourceMissing),
            0,
            "源树齐备时不该有源缺失"
        );
        assert!(p.pending().len() == p.files.len());
        assert!(!p.is_settled(), "有待写文件就不算收敛");
        // 去 texmf-dist 前缀：目标路径就是 TDS 相对路径。
        assert!(
            p.files
                .iter()
                .all(|f| !f.rel_path.starts_with("texmf-dist/")),
            "目标路径应已剥掉 texmf-dist/ 前缀"
        );
    }

    #[test]
    fn apply_then_replan_is_settled_and_idempotent() {
        let (db, c) = mini_closure();
        let mut vfs = source_vfs(&db, &[]);
        let src = LocalTexLiveSource::new("tl");

        let p1 = plan(&db, &c, &src, "assets", &mut vfs).expect("plan 应成功");
        let r = apply(&p1, "assets", &mut vfs).expect("apply 应成功");
        assert_eq!(r.written_files, p1.files.len(), "首次应全部写出");
        assert_eq!(r.refreshed, 0);
        assert_eq!(r.source_missing_after_plan.len(), 0);

        // 再跑一次：应为一致 + 收敛（这就是"资产集已同步"的判据）。
        let p2 = plan(&db, &c, &src, "assets", &mut vfs).expect("plan 应成功");
        assert_eq!(p2.count(FileState::Identical), p2.files.len());
        assert_eq!(p2.pending().len(), 0);
        assert!(p2.is_settled(), "写完后应收敛");
    }

    #[test]
    fn byte_difference_is_reported_as_refresh_not_add() {
        let (db, c) = mini_closure();
        let mut vfs = source_vfs(&db, &[]);
        let src = LocalTexLiveSource::new("tl");
        let p1 = plan(&db, &c, &src, "assets", &mut vfs).expect("plan 应成功");
        apply(&p1, "assets", &mut vfs).expect("apply 应成功");

        // 篡改目标树里的一个文件 → 应被判为"需刷新"而不是"一致"。
        let victim = p1
            .files
            .iter()
            .find(|f| f.rel_path.ends_with("geometry.sty"))
            .expect("fixture 应含 geometry.sty");
        vfs.write(&join_path("assets", &victim.rel_path), b"% tampered")
            .expect("写入应成功");

        let p2 = plan(&db, &c, &src, "assets", &mut vfs).expect("plan 应成功");
        assert_eq!(p2.count(FileState::Differ), 1, "被改动的文件应报需刷新");
        assert!(!p2.is_settled(), "有需刷新项就不算收敛");

        let r = apply(&p2, "assets", &mut vfs).expect("apply 应成功");
        assert_eq!(r.refreshed, 1);
        assert_eq!(r.added, 0);
    }

    #[test]
    fn source_missing_is_explicit_and_blocks_settlement() {
        let (db, c) = mini_closure();
        // 源树里故意缺掉 geometry 包的一个运行面文件。
        let mut vfs = source_vfs(&db, &[("geometry", "geometry.sty")]);
        let src = LocalTexLiveSource::new("tl");
        let p = plan(&db, &c, &src, "assets", &mut vfs).expect("plan 应成功");

        let missing = p.source_missing();
        assert_eq!(missing.len(), 1, "应精确报出 1 个源缺失");
        assert_eq!(missing[0].package, "geometry");
        assert!(missing[0].rel_path.ends_with("geometry.sty"));
        assert!(!p.is_settled(), "源缺失必须让计划不收敛");

        let r = apply(&p, "assets", &mut vfs).expect("apply 应成功");
        assert_eq!(r.skipped_source_missing, 1, "源缺失应在执行阶段被跳过");
        assert_eq!(r.source_missing_after_plan.len(), 0);
    }

    #[test]
    fn closure_naming_unknown_package_is_rejected() {
        let (db, c) = mini_closure();
        // 闭包里塞一个本 TLPDB 没有的包名 → 必须显式报错，不静默漏掉它。
        let mut packages = c.packages.clone();
        packages.insert("definitely-not-in-this-pdb".to_owned());
        let mismatched = Closure {
            seeds: c.seeds.clone(),
            packages,
            edges: Default::default(),
            unresolved: Vec::new(),
            download_bytes: 0,
        };
        let mut vfs = source_vfs(&db, &[]);
        let src = LocalTexLiveSource::new("tl");
        match plan(&db, &mismatched, &src, "assets", &mut vfs) {
            Err(Error::PackageNotFound { name, hint }) => {
                assert_eq!(name, "definitely-not-in-this-pdb");
                assert!(hint.contains("闭包与库不匹配"), "hint 应说明根因：{hint}");
            }
            other => panic!("应报 PackageNotFound，实际 {other:?}"),
        }
        // 对照：原闭包能正常成plan（证明上面的失败确实来自那个陌生包名）。
        assert!(plan(&db, &c, &src, "assets", &mut vfs).is_ok());
    }

    #[test]
    fn parent_dir_handles_nesting_and_flat_names() {
        assert_eq!(parent_dir("a/b/c.tex"), Some("a/b"));
        assert_eq!(parent_dir("c.tex"), None, "无分隔符时不该要求建目录");
        assert_eq!(
            parent_dir("/c.tex"),
            None,
            "空父目录（根）不该被当成要建的目录"
        );
        assert_eq!(parent_dir("a//c.tex"), Some("a/"));
    }
}
