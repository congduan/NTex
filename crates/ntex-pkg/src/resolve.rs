//! 解析层：从「用户要什么」推到「该装哪些包」——(plan.md §6.2 第 9 条)。
//!
//! 两步走，中间不许猜：
//!
//! ```text
//! ① 需求解析   \usepackage{geometry} / \documentclass{article} / \input{x}
//!              → 候选文件名（geometry.sty）→ 提供包（TL 包 `geometry`）
//! ② 依赖闭包   种子包 → 传递依赖（pkg.ARCH 展开、release/·opt_ 剔除、环安全）
//!              → 闭包 + 未解析项 + 下载体量估算
//! ```
//!
//! 第 ① 步是 CTAN 给不了的那一层：CTAN 只知道「有个文件叫 geometry.sty」，
//! 不知道「它由 TL 包 geometry 提供、该包又依赖 atbegshi」。所以需求的解析
//! **只认 TLPDB**。
//!
//! **同名校验**：一个文件名可能有多个提供者（真实重复存在）。本模块取
//! 排序首项作默认选择，并把全部候选带在 [`Resolved::providers`] 里，
//! 绝不静默丢弃备选。

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::tlpdb::{ProviderRef, TlPdb};
use crate::Error;

/// 需求种类：决定候选扩展名。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequireKind {
    /// `\usepackage{...}`：`x.sty` → `x.ltx` → `x.tex`。
    UsePackage,
    /// `\documentclass{...}`：`x.cls`。
    DocumentClass,
    /// `\input{...}` / `\include{...}`：`x.tex`。
    Input,
    /// NFSS 字体定义：`x.fd`。
    FontDef,
    /// 原样文件名（调用方已给出扩展名）。
    Raw,
}

impl RequireKind {
    /// 候选文件名，按查找优先级。
    fn candidates(self, name: &str) -> Vec<String> {
        match self {
            RequireKind::UsePackage => vec![
                format!("{name}.sty"),
                format!("{name}.ltx"),
                format!("{name}.tex"),
            ],
            RequireKind::DocumentClass => vec![format!("{name}.cls")],
            RequireKind::Input => vec![format!("{name}.tex")],
            RequireKind::FontDef => vec![format!("{name}.fd")],
            RequireKind::Raw => vec![name.to_owned()],
        }
    }

    /// 人类可读名（错误提示用）。
    pub fn label(self) -> &'static str {
        match self {
            RequireKind::UsePackage => "\\usepackage",
            RequireKind::DocumentClass => "\\documentclass",
            RequireKind::Input => "\\input",
            RequireKind::FontDef => "字体定义",
            RequireKind::Raw => "文件名",
        }
    }
}

/// 一次需求解析的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// 原请求（如 `geometry`）。
    pub request: String,
    /// 实际命中的文件名（如 `geometry.sty`）。
    pub file: String,
    /// 全部提供者（已按 TDS 偏好序排序，首项为默认选择）。
    pub providers: Vec<ProviderRef>,
}

impl Resolved {
    /// 默认选中的提供包名。
    pub fn package(&self) -> &str {
        self.providers
            .first()
            .map(|p| p.package.as_str())
            .unwrap_or("")
    }

    /// 是否存在多个提供者（调用方可据此提示歧义）。
    pub fn is_ambiguous(&self) -> bool {
        self.providers.len() > 1
    }
}

/// 无法解析的依赖边。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnresolvedDep {
    /// 谁依赖它（种子来自需求解析时记为 `(种子)`）。
    pub from: String,
    /// 依赖名。
    pub dep: String,
}

/// 依赖闭包解析结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Closure {
    /// 需求解析明细（仅 [`closure_for_requires`] 填充）。
    pub seeds: Vec<Resolved>,
    /// 闭包内全部包名（含种子），按名字序。
    pub packages: BTreeSet<String>,
    /// 包 → 直接依赖（已裁剪到闭包内视角，按名字序、已去重）。
    pub edges: BTreeMap<String, Vec<String>>,
    /// 无法在 TLPDB 中解析的依赖项。
    pub unresolved: Vec<UnresolvedDep>,
    /// 按 `containersize` 估算的下载体量（字节；无该字段的包计 0）。
    pub download_bytes: u64,
}

impl Closure {
    /// 闭包内运行面文件总数。
    pub fn run_file_count(&self, db: &TlPdb) -> usize {
        self.packages
            .iter()
            .filter_map(|p| db.get(p))
            .map(|p| p.run_files.len())
            .sum()
    }

    /// 闭包内**缺少 SHA-512** 的包——它们无法进入锁文件（不可校验 = 不可锁定）。
    pub fn unverifiable(&self, db: &TlPdb) -> Vec<String> {
        self.packages
            .iter()
            .filter(|name| {
                db.get(name)
                    .map(|p| p.container_sha512.is_none())
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    /// 闭包内按类别统计（诊断用）。
    pub fn category_histogram(&self, db: &TlPdb) -> BTreeMap<String, usize> {
        let mut hist: BTreeMap<String, usize> = BTreeMap::new();
        for name in &self.packages {
            let key = db
                .get(name)
                .and_then(|p| p.category.as_ref())
                .map(|c| c.as_str().to_owned())
                .unwrap_or_else(|| "(无类别)".to_owned());
            *hist.entry(key).or_insert(0) += 1;
        }
        hist
    }

    /// 闭包是否干净（无未解析、无不可校验项）。
    pub fn is_clean(&self, db: &TlPdb) -> bool {
        self.unresolved.is_empty() && self.unverifiable(db).is_empty()
    }
}

/// 按文件名取提供者；含 `/` 视为路径，否则视为 basename。
fn providers_for<'a>(db: &'a TlPdb, file: &str) -> &'a [ProviderRef] {
    if file.contains('/') {
        db.providers_of_path(file)
    } else {
        db.providers_of(file)
    }
}

/// 解析一条需求：`(名字, 种类)` → 命中的文件名 + 全部提供者。
///
/// 候选扩展名逐个尝试，**第一个有任何提供者的候选胜出**；全部落空时返回
/// [`Error::PackageNotFound`]，其 `hint` 字段携带可复制的反查命令
/// （对应 §6.2 第 8 条「缺包即报 + 一条命令补」）。
pub fn resolve_require(db: &TlPdb, name: &str, kind: RequireKind) -> Result<Resolved, Error> {
    if name.trim().is_empty() {
        return Err(Error::PackageNotFound {
            name: name.to_owned(),
            hint: "需求名为空".to_owned(),
        });
    }
    for candidate in kind.candidates(name) {
        let providers = providers_for(db, &candidate);
        if !providers.is_empty() {
            return Ok(Resolved {
                request: name.to_owned(),
                file: candidate,
                providers: providers.to_vec(),
            });
        }
    }
    Err(Error::PackageNotFound {
        name: name.to_owned(),
        hint: not_found_hint(name, kind),
    })
}

/// 找不到包时给出的可复制反查指引。
fn not_found_hint(name: &str, kind: RequireKind) -> String {
    let probe = match kind {
        RequireKind::DocumentClass => format!("{name}.cls"),
        RequireKind::Input => format!("{name}.tex"),
        RequireKind::FontDef => format!("{name}.fd"),
        _ => format!("{name}.sty"),
    };
    format!(
        "该{}请求的文件（试过 `{probe}` 等候选）在本 TLPDB 中无提供者。\
         反查：`ntex-pkg provide <tlpdb> {probe}`；或 `tlmgr search --global --file {probe}`。\
         若确认存在却查不到，说明当前发行档未收录（如 scheme-basic），\
         需切到更高档或走 CTAN 回落源。",
        kind.label()
    )
}

/// 计算依赖闭包（种子必须是已存在的包名）。
///
/// 规则：
/// - 每个包只处理一次（BTreeSet 去重 + BFS），**结构上免疫依赖环**；
/// - `pkg.ARCH` 按 `arch` 展开；`release/…`·`minrelease/…`·`opt_…`·`setting_…` 剔除；
/// - 依赖指向不存在的包时**不失败**，记入 [`Closure::unresolved`]——因为
///   「TLPDB 不完整」与「用户打错包名」需要不同的处置，由调用方决定。
pub fn closure(db: &TlPdb, seeds: &[String], arch: &str) -> Closure {
    let mut out = Closure::default();
    let mut queue: VecDeque<String> = VecDeque::new();

    for seed in seeds {
        if db.get(seed).is_none() {
            out.unresolved.push(UnresolvedDep {
                from: "(种子)".to_owned(),
                dep: seed.clone(),
            });
            continue;
        }
        if out.packages.insert(seed.clone()) {
            queue.push_back(seed.clone());
        }
    }

    while let Some(name) = queue.pop_front() {
        let Some(pkg) = db.get(&name) else {
            continue;
        };
        out.download_bytes = out
            .download_bytes
            .saturating_add(pkg.container_size.unwrap_or(0));

        let mut edges: Vec<String> = Vec::new();
        for dep in pkg.package_deps(arch) {
            edges.push(dep.clone());
            if db.get(&dep).is_none() {
                out.unresolved.push(UnresolvedDep {
                    from: name.clone(),
                    dep,
                });
                continue;
            }
            if out.packages.insert(dep.clone()) {
                queue.push_back(dep);
            }
        }
        edges.sort();
        edges.dedup();
        out.edges.insert(name, edges);
    }

    out.unresolved.sort();
    out.unresolved.dedup();
    out
}

/// 解析需求明细并求闭包（CLI 与上层的主入口）。
pub fn closure_for_requires(
    db: &TlPdb,
    requests: &[(String, RequireKind)],
    arch: &str,
) -> Result<Closure, Error> {
    let mut resolved = Vec::with_capacity(requests.len());
    let mut seeds = Vec::with_capacity(requests.len());
    for (name, kind) in requests {
        let r = resolve_require(db, name, *kind)?;
        seeds.push(r.package().to_owned());
        resolved.push(r);
    }
    seeds.sort();
    seeds.dedup();
    let mut c = closure(db, &seeds, arch);
    c.seeds = resolved;
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdata::{mini, MINI_TLPDB};

    #[test]
    fn use_package_resolves_to_providing_package() {
        let db = mini();
        let r = resolve_require(&db, "geometry", RequireKind::UsePackage).expect("geometry 可解析");
        assert_eq!(r.file, "geometry.sty");
        assert_eq!(r.package(), "geometry");
        assert!(!r.is_ambiguous());
    }

    #[test]
    fn document_class_uses_cls_extension() {
        // 夹具里没有 .cls，用 provide 语义验证候选扩展名确实只试 .cls
        let db = mini();
        let err = resolve_require(&db, "geometry", RequireKind::DocumentClass)
            .expect_err("geometry 没有 .cls");
        match err {
            Error::PackageNotFound { hint, .. } => {
                assert!(
                    hint.contains("geometry.cls"),
                    "hint 应给出 .cls 反查：{hint}"
                );
                assert!(
                    hint.contains("ntex-pkg provide"),
                    "hint 应可直接复制：{hint}"
                );
            }
            other => panic!("应为 PackageNotFound，实际 {other:?}"),
        }
    }

    #[test]
    fn unknown_package_reports_copyable_hint() {
        let db = mini();
        match resolve_require(&db, "no-such-pkg", RequireKind::UsePackage) {
            Err(Error::PackageNotFound { name, hint }) => {
                assert_eq!(name, "no-such-pkg");
                assert!(hint.contains("no-such-pkg.sty"));
                assert!(
                    hint.contains("tlmgr search"),
                    "hint 应给出 tlmgr 反查：{hint}"
                );
            }
            other => panic!("应为 PackageNotFound，实际 {other:?}"),
        }
    }

    #[test]
    fn empty_request_is_rejected() {
        let db = mini();
        assert!(matches!(
            resolve_require(&db, "  ", RequireKind::UsePackage),
            Err(Error::PackageNotFound { .. })
        ));
    }

    #[test]
    fn raw_kind_matches_exact_filename() {
        let db = mini();
        let r = resolve_require(&db, "geometry.sty", RequireKind::Raw).expect("精确文件名");
        assert_eq!(r.package(), "geometry");
    }

    #[test]
    fn closure_expands_arch_and_filters_non_package_deps() {
        let db = mini();
        let c = closure(&db, &["geometry".to_owned()], "universal-darwin");
        assert_eq!(
            c.packages.iter().cloned().collect::<Vec<_>>(),
            vec![
                "atbegshi".to_owned(),
                "geometry".to_owned(),
                "pdftex.universal-darwin".to_owned()
            ],
            "release/2024 与 opt_* 不得进闭包；.ARCH 必须展开"
        );
        assert!(c.unresolved.is_empty(), "夹具内依赖应全部可解析");
        assert_eq!(c.download_bytes, 4096 + 2048, "体量按 containersize 累加");
        assert_eq!(
            c.edges["geometry"],
            vec!["atbegshi".to_owned(), "pdftex.universal-darwin".to_owned()]
        );
    }

    #[test]
    fn closure_walks_transitive_deps() {
        let db = mini();
        let c = closure(&db, &["l3kernel".to_owned()], "universal-darwin");
        assert!(c.packages.contains("expl3"), "传递依赖必须入闭包");
        assert_eq!(c.packages.len(), 2);
    }

    #[test]
    fn unknown_seed_is_reported_not_inserted() {
        let db = mini();
        let c = closure(&db, &["mystery".to_owned()], "universal-darwin");
        assert!(c.packages.is_empty(), "不存在的种子不得进闭包");
        assert_eq!(c.unresolved.len(), 1);
        assert_eq!(c.unresolved[0].from, "(种子)");
        assert_eq!(c.unresolved[0].dep, "mystery");
    }

    #[test]
    fn dangling_dep_is_recorded_with_its_depender() {
        let text =
            "name a\ncategory Package\ncontainersize 10\ndepend b\ndepend missing\ndepend a\n\n\
                    name b\ncategory Package\ncontainersize 20\ndepend a\n";
        let db = TlPdb::parse(text).expect("fixture 可解析");
        let c = closure(&db, &["a".to_owned()], "universal-darwin");
        assert_eq!(
            c.packages.iter().cloned().collect::<Vec<_>>(),
            vec!["a".to_owned(), "b".to_owned()],
            "自环与互环都不得导致重复或死循环"
        );
        assert_eq!(c.unresolved.len(), 1);
        assert_eq!(c.unresolved[0].from, "a");
        assert_eq!(c.unresolved[0].dep, "missing");
        assert_eq!(c.download_bytes, 30);
        assert!(!c.is_clean(&db), "有未解析项即不干净");
    }

    #[test]
    fn closure_counts_files_and_flags_unverifiable() {
        let db = mini();
        let c = closure(&db, &["geometry".to_owned()], "universal-darwin");
        assert_eq!(
            c.run_file_count(&db),
            5,
            "geometry 3 + atbegshi 1 + pdftex 1"
        );
        assert_eq!(
            c.unverifiable(&db),
            vec!["pdftex.universal-darwin".to_owned()]
        );
        assert!(!c.is_clean(&db), "夹具里 pdftex 包无 checksum，故不干净");
    }

    #[test]
    fn category_histogram_after_closure() {
        let db = mini();
        let c = closure(&db, &["geometry".to_owned()], "universal-darwin");
        let h = c.category_histogram(&db);
        assert_eq!(h.get("Package"), Some(&2), "geometry + atbegshi");
        assert_eq!(h.get("TLCore"), Some(&1));
    }

    #[test]
    fn closure_for_requires_fills_seed_details() {
        let db = mini();
        let reqs = vec![("geometry".to_owned(), RequireKind::UsePackage)];
        let c = closure_for_requires(&db, &reqs, "universal-darwin").expect("需求可解析");
        assert_eq!(c.seeds.len(), 1);
        assert_eq!(c.seeds[0].file, "geometry.sty");
        assert!(c.packages.contains("geometry"));
    }

    #[test]
    fn closure_for_requires_propagates_not_found() {
        let db = mini();
        let reqs = vec![("nope".to_owned(), RequireKind::UsePackage)];
        assert!(matches!(
            closure_for_requires(&db, &reqs, "universal-darwin"),
            Err(Error::PackageNotFound { .. })
        ));
    }

    #[test]
    fn duplicate_seeds_do_not_double_count_size() {
        let db = mini();
        let c = closure(
            &db,
            &["geometry".to_owned(), "geometry".to_owned()],
            "universal-darwin",
        );
        assert_eq!(c.download_bytes, 4096 + 2048, "重复种子不得重复计体积");
    }

    /// 用真实 tlpdb 做端到端校验（环境依赖：文件不存在即跳过）。
    ///
    /// 其中 `ae` 一段是**针对真实数据的回归锁**：声明 `runfiles size=157`
    /// 而实际 114 行——`size=` 是 RIV 块数，不是行数。任何「按 size= 读 N 行」
    /// 的实现都会在这里失败。
    #[test]
    fn works_against_real_tlpdb_when_present() {
        let path = "/usr/local/texlive/2024basic/tlpkg/texlive.tlpdb";
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let db = TlPdb::parse(&text).expect("真实 tlpdb 必须可解析");
        assert!(db.len() >= 340, "真实档记录数异常：{}", db.len());

        let ae = db.get("ae").expect("TL 基础档必含 ae");
        assert_eq!(ae.run_size_blocks, Some(157), "上游声明的 RIV 块数");
        assert_eq!(
            ae.run_files.len(),
            114,
            "实际文件行数由续行规则决定，与 size= 无关"
        );
        assert!(
            ae.run_files.iter().all(|f| f.starts_with("texmf-dist/")),
            "清单条目不得混入后续字段"
        );

        // `plain` 在本地 tlpdb 里是叶子包（无 depend 行）——闭包应恰为自身
        let leaf = closure(&db, &["plain".to_owned()], crate::tlpdb::host_arch());
        assert_eq!(
            leaf.packages.len(),
            1,
            "plain 无依赖，闭包应为自身：{:?}",
            leaf.packages
        );

        // `pdftex` 带 10 条依赖（含 .ARCH 平台包）——验证真实规模下的传递闭包
        let c = closure(&db, &["pdftex".to_owned()], crate::tlpdb::host_arch());
        assert!(c.packages.len() > 5, "pdftex 闭包过小：{:?}", c.packages);
        assert!(
            c.packages.iter().any(|p| p.starts_with("pdftex.")),
            ".ARCH 依赖必须展开为平台包：{:?}",
            c.packages
        );
        assert!(
            c.unresolved.is_empty(),
            "真实 tlpdb 闭包不应有悬空依赖：{:?}",
            c.unresolved
        );
        assert!(MINI_TLPDB.len() < text.len());
    }
}
