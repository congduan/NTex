//! TLPDB（TeX Live Package DataBase）解析器 —— 解析层（plan.md §6.2 第 9 条）。
//!
//! **为什么解析只认 TLPDB**：从 `\usepackage{graphicx}` 追问「哪个包真正提供
//! `graphicx.sty`、它又依赖谁、它的字节该是什么哈希」，只有 TLPDB 一次给出三件事——
//! 包 → 文件清单、包 → 依赖、包 → `container-sha512`。CTAN 的 `FILES.byname` 只有
//! 文件名 → 路径（25MB、每日重建、无校验和、无依赖、无版本号），不能作解析依据。
//!
//! ## 格式（ground truth：`/usr/local/texlive/2024basic/tlpkg/texlive.tlpdb`）
//!
//! ```text
//! name <pkg>                     ← 记录起始；空行结束记录
//! category <Package|TLCore|Collection|Scheme>
//! revision <整数>                 ← 可锁定的版本号
//! depend <pkg>                   ← 可重复；另有 dep.ARCH / release/·minrelease/ / opt_·setting_
//! containersize <字节>            ← 容器（.tar.xz）字节数
//! containerchecksum <128 hex>    ← SHA-512（TLConfig::ChecksumLength = 128）
//! runfiles size=N                ← N 是 **RIV 块数**，不是行数（见下）
//!  texmf-dist/tex/latex/…/x.sty  ← 缩进行 = 上一文件清单段的续行
//! binfiles arch=X size=N
//! ```
//!
//! ## 两个曾经踩过的坑（本模块的核心知识，勿凭直觉重写）
//!
//! 1. **`runfiles size=N` 的 N 不是行数，是 RIV 块数**。官方
//!    `TeXLive::TLPOBJ::_recompute_size` 按每个文件
//!    `int(bytes / BlockSize) (+1 若有余数)` 累加，`BlockSize = 4096`
//!    （`TeXLive::TLConfig`）。实测校准：TL2024 的 `ae` 包声明 `size=157`，
//!    实际只有 **114** 个运行面文件，按 4096 字节块逐文件累加恰好得 **157**。
//!    本仓库全库 327 个文件清单段中 **284 段** 的「声明值 ≠ 行数」。
//!    → 因此**绝不能**用 `size=N` 决定读多少行。
//!
//! 2. **段的边界由「续行规则」决定**：官方 `TLPOBJ::from_fh` 对每行做
//!    `split(/\s+/, $line, 2)`，缩进行得到的命令名是空串，于是被推入
//!    `$lastcmd` 对应的清单；一旦出现新的命令名，段即结束。
//!    → 本模块等价实现为：**缩进行续段，非缩进行换段**。
//!
//! 其余细节：未知字段一律忽略（向前兼容，TLPDB 定期加字段）；一条记录内出现第二个
//! `name` 报警（官方 `die`），因为那意味着记录缺了分隔空行——顺着读会凭空造出包。
//! 解析失败一律带 1 基行号与字段名。
//!
//! **内存**：全量 tlpdb ≈ 70k 条文件路径，故建两份倒排索引（按 basename、按规范化路径）。
//! 这是「解析一次、常驻进程」的取舍，见 [`TlPdb`] 文档。

use crate::{basename, Error};

/// 平台占位符后缀：TLPDB 用 `pdftex.ARCH` 引用「当前平台的二进制包」。
pub const ARCH_PLACEHOLDER: &str = "ARCH";

/// TeX Live 的 RIV 块大小（`TeXLive::TLConfig::BlockSize`）。
///
/// 文件清单段声明的 `size=` 以块计，乘它可得**未压缩占用的估算字节数**。
pub const BLOCK_SIZE: u64 = 4096;

/// SHA-512 的十六进制长度（`TeXLive::TLConfig::ChecksumLength`）。
pub const CHECKSUM_LENGTH: usize = 128;

/// 本机平台的 TeX Live 架构名（与 TLPDB `available_architectures` 同口径）。
///
/// **简化**：只做四类映射（macOS 统一 `universal-darwin`、Linux 分 aarch64/x86_64、
/// Windows 统一 `windows`）。未知平台返回空串，此时 `.ARCH` 依赖会自然落入「未解析」，
/// 由调用方显式报告——不静默猜测。见 docs/KNOWN-SIMPLIFICATIONS.md。
pub fn host_arch() -> &'static str {
    if cfg!(target_os = "macos") {
        "universal-darwin"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-linux"
    } else if cfg!(target_os = "linux") {
        "x86_64-linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        ""
    }
}

/// TLPDB 记录的类别。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    /// 真正含文件的一层（`scheme → collection → package`，只有它带 runfiles）。
    Package,
    /// TeX Live 内核组件。
    TlCore,
    /// 包的集合（自身通常不含文件，只依赖包）。
    Collection,
    /// 顶层发行方案。
    Scheme,
    /// 未知/未来类别（原样保留；官方对未知类别只告警不报错）。
    Other(String),
}

impl Category {
    fn parse(s: &str) -> Self {
        match s {
            "Package" => Category::Package,
            "TLCore" => Category::TlCore,
            "Collection" => Category::Collection,
            "Scheme" => Category::Scheme,
            other => Category::Other(other.to_owned()),
        }
    }

    /// 原文字面（用于锁文件与报告）。
    pub fn as_str(&self) -> &str {
        match self {
            Category::Package => "Package",
            Category::TlCore => "TLCore",
            Category::Collection => "Collection",
            Category::Scheme => "Scheme",
            Category::Other(s) => s,
        }
    }
}

/// 一条 TLPDB 记录。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TlPackage {
    /// 包名（TLPDB 主键；官方约束 `^[-.\w]+$`，故不含路径分隔符）。
    pub name: String,
    /// 类别。
    pub category: Option<Category>,
    /// 版本号（锁定用）。
    pub revision: Option<u64>,
    /// 一句话说明（诊断/CLI 展示；官方为累加语义，故可能由多行拼成）。
    pub short_desc: Option<String>,
    /// 依赖原样列表（含 `.ARCH` / `release/…` / `opt_…` 等全部形态）。
    pub depends: Vec<String>,
    /// 运行面文件清单（相对 TL 根，例 `texmf-dist/tex/latex/geometry/geometry.sty`）。
    pub run_files: Vec<String>,
    /// 运行面清单声明的 **RIV 块数**（`size=`）——**不是**行数，见模块文档。
    pub run_size_blocks: Option<u64>,
    /// 平台二进制文件清单（NTex 不使用；`arch` 归属未保留，见 KNOWN-SIMPLIFICATIONS）。
    pub bin_files: Vec<String>,
    /// 容器字节数（`.tar.xz`）。
    pub container_size: Option<u64>,
    /// 容器 SHA-512（128 位十六进制）——锁定与校验的锚点。
    pub container_sha512: Option<String>,
    /// CTAN 许可证标识（法务登记用）。
    pub catalogue_license: Option<String>,
    /// CTAN 目录键（回溯上游用）。
    pub catalogue_ctan: Option<String>,
    /// `execute` 指令（`AddFormat`/`addMap` 等，NTex 不执行，保留供诊断）。
    pub execute: Vec<String>,
    /// `relocated` 字段原样保留（**语义未实现**，见 KNOWN-SIMPLIFICATIONS）。
    pub relocated: Option<String>,
}

impl TlPackage {
    /// 真实包依赖：剔除版本约束（`release/`、`minrelease/`）与安装选项
    /// （`opt_*`、`setting_*`），并把 `.ARCH` 占位换成 `arch` 实参。
    pub fn package_deps(&self, arch: &str) -> Vec<String> {
        self.depends
            .iter()
            .filter(|d| is_real_dep(d))
            .map(|d| expand_arch(d, arch))
            .collect()
    }

    /// 该记录是否可能被 NTex 直接使用（含运行面文件）。
    pub fn has_run_files(&self) -> bool {
        !self.run_files.is_empty()
    }

    /// 运行面未压缩占用的估算字节数（RIV 块数 × [`BLOCK_SIZE`]）。
    pub fn run_size_bytes(&self) -> Option<u64> {
        self.run_size_blocks.map(|b| b * BLOCK_SIZE)
    }
}

/// 依赖项是否为「真实包依赖」。
///
/// 实测（TeX Live 2024basic，560 条 `depend`）只存在四类：真实包名、`pkg.ARCH`、
/// `release/…`·`minrelease/…`、`opt_…`/`setting_…`。后两类不是包，须剔除。
fn is_real_dep(dep: &str) -> bool {
    !dep.is_empty() && !dep.contains(':') && !dep.contains('/')
}

/// 把 `pkg.ARCH` 展开为 `pkg.<arch>`。
fn expand_arch(dep: &str, arch: &str) -> String {
    match dep.strip_suffix(ARCH_PLACEHOLDER) {
        Some(head) => format!("{head}{arch}"),
        None => dep.to_owned(),
    }
}

/// 文件名提供者：某包在某路径下提供该文件。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderRef {
    /// 提供包名。
    pub package: String,
    /// 相对 TL 根的路径。
    pub path: String,
}

/// 描述统计（CLI `index` 与验收用）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PdbStats {
    /// 记录总数。
    pub records: usize,
    /// `category Package` 数。
    pub packages: usize,
    /// `category Collection` 数。
    pub collections: usize,
    /// `category Scheme` 数。
    pub schemes: usize,
    /// `category TLCore` 数。
    pub tl_core: usize,
    /// 运行面文件总数。
    pub run_files: usize,
    /// 平台二进制文件总数。
    pub bin_files: usize,
    /// 带 `container-sha512` 的记录数。
    pub with_checksum: usize,
    /// 去重后的 basename 数。
    pub distinct_basenames: usize,
}

/// TLPDB 全量视图：记录表 + 两份倒排索引。
///
/// 索引把「文件名 → 提供者」变成 O(log n) 查询，这是 `\usepackage` 解析与
/// `kpsewhich` 等价查找的基础。同名文件可以有多个提供者（真实重复存在：
/// `expl3-code.tex` 同时出现在 `tex/latex/base/` 与 `tex/latex/l3kernel/`），
/// 因此每个键对应一个**已排序**的提供者列表，`find_*` 取首项。
#[derive(Debug, Default, Clone)]
pub struct TlPdb {
    packages: std::collections::BTreeMap<String, TlPackage>,
    by_basename: std::collections::BTreeMap<String, Vec<ProviderRef>>,
    by_path: std::collections::BTreeMap<String, Vec<ProviderRef>>,
}

impl TlPdb {
    /// 解析 TLPDB 全文。
    ///
    /// `text` 通常由 [`ntex_io::Vfs`] 读出（本 crate 不直接碰 `std::fs`）。
    pub fn parse(text: &str) -> Result<Self, Error> {
        let lines: Vec<&str> = text.lines().collect();
        let mut db = TlPdb::default();
        let mut cur: Option<(usize, TlPackage)> = None;
        // 当前文件清单段；非文件清单命令一律把它清空（官方 $lastcmd 语义）
        let mut section: Option<&'static str> = None;
        let mut i = 0usize;

        while i < lines.len() {
            let line_no = i + 1;
            let line = strip_cr(lines[i]);
            i += 1;

            // 空行 = 记录边界
            if line.is_empty() {
                if let Some((start, pkg)) = cur.take() {
                    db.insert(pkg, start)?;
                }
                section = None;
                continue;
            }

            // 缩进行 = 上一文件清单段的续行（官方：命令名为空串即续行）
            if line.starts_with(char::is_whitespace) {
                let path = line.trim();
                let Some(sec) = section else {
                    return Err(Error::Tlpdb {
                        line: line_no,
                        field: "(缩进行)",
                        detail: "缩进行只允许跟随 runfiles/binfiles/docfiles/srcfiles 段"
                            .to_owned(),
                    });
                };
                if path.is_empty() {
                    return Err(Error::Tlpdb {
                        line: line_no,
                        field: sec,
                        detail: "文件清单条目为空".to_owned(),
                    });
                }
                let Some((_, pkg)) = cur.as_mut() else {
                    return Err(Error::Tlpdb {
                        line: line_no,
                        field: sec,
                        detail: "文件清单条目出现在任何 `name` 之前".to_owned(),
                    });
                };
                match sec {
                    "runfiles" => pkg.run_files.push(path.to_owned()),
                    "binfiles" => pkg.bin_files.push(path.to_owned()),
                    // docfiles / srcfiles 不进运行面（NTex 不吃文档与源码）
                    _ => {}
                }
                continue;
            }

            let (field, rest) = split_label(line);

            if field == "name" {
                if cur.is_some() {
                    return Err(Error::Tlpdb {
                        line: line_no,
                        field: "name",
                        detail: "同一条记录内出现第二个 `name`（记录之间须以空行分隔）".to_owned(),
                    });
                }
                if rest.is_empty() {
                    return Err(Error::Tlpdb {
                        line: line_no,
                        field: "name",
                        detail: "包名为空".to_owned(),
                    });
                }
                section = None;
                cur = Some((
                    line_no,
                    TlPackage {
                        name: rest.to_owned(),
                        ..TlPackage::default()
                    },
                ));
                continue;
            }

            // 非文件清单命令即结束续行段
            section = match field {
                "runfiles" => Some("runfiles"),
                "binfiles" => Some("binfiles"),
                "docfiles" => Some("docfiles"),
                "srcfiles" => Some("srcfiles"),
                _ => None,
            };

            let Some((_, pkg)) = cur.as_mut() else {
                return Err(Error::Tlpdb {
                    line: line_no,
                    field: "name",
                    detail: format!("记录未以 `name` 起始，却出现字段 `{field}`"),
                });
            };

            match field {
                "category" => pkg.category = Some(Category::parse(rest)),
                "revision" => pkg.revision = Some(parse_u64(rest, line_no, "revision")?),
                // 官方为累加语义（`.=`），此处保持一致
                "shortdesc" => match pkg.short_desc.as_mut() {
                    Some(s) => s.push_str(rest),
                    None => pkg.short_desc = Some(rest.to_owned()),
                },
                "depend" => {
                    if rest.is_empty() {
                        return Err(Error::Tlpdb {
                            line: line_no,
                            field: "depend",
                            detail: "依赖名为空".to_owned(),
                        });
                    }
                    pkg.depends.push(rest.to_owned());
                }
                "execute" => pkg.execute.push(rest.to_owned()),
                "containersize" => {
                    pkg.container_size = Some(parse_u64(rest, line_no, "containersize")?)
                }
                "containerchecksum" => pkg.container_sha512 = Some(rest.to_owned()),
                "catalogue-license" => pkg.catalogue_license = Some(rest.to_owned()),
                "catalogue-ctan" => pkg.catalogue_ctan = Some(rest.to_owned()),
                "relocated" => pkg.relocated = Some(rest.to_owned()),
                // RIV 块数：登记备查，**绝不**用它决定读多少行
                "runfiles" => pkg.run_size_blocks = parse_size_tag(rest, line_no, "runfiles")?,
                // binfiles/docfiles/srcfiles 的 size=/arch= 标签对 NTex 无用途，忽略
                _ => {}
            }
        }

        if let Some((start, pkg)) = cur.take() {
            db.insert(pkg, start)?;
        }
        db.rebuild_index();
        Ok(db)
    }

    fn insert(&mut self, pkg: TlPackage, line: usize) -> Result<(), Error> {
        if self.packages.contains_key(&pkg.name) {
            return Err(Error::Tlpdb {
                line,
                field: "name",
                detail: format!("重复记录 `{}`", pkg.name),
            });
        }
        self.packages.insert(pkg.name.clone(), pkg);
        Ok(())
    }

    fn rebuild_index(&mut self) {
        self.by_basename.clear();
        self.by_path.clear();
        for (name, pkg) in &self.packages {
            for path in &pkg.run_files {
                let provider = ProviderRef {
                    package: name.clone(),
                    path: path.clone(),
                };
                self.by_basename
                    .entry(basename(path).to_owned())
                    .or_default()
                    .push(provider.clone());
                self.by_path
                    .entry(normalize_path(path))
                    .or_default()
                    .push(provider);
            }
        }
        for bucket in self.by_basename.values_mut() {
            sort_providers(bucket);
        }
        for bucket in self.by_path.values_mut() {
            sort_providers(bucket);
        }
    }

    /// 记录数。
    pub fn len(&self) -> usize {
        self.packages.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// 按包名取记录。
    pub fn get(&self, name: &str) -> Option<&TlPackage> {
        self.packages.get(name)
    }

    /// 按包名序迭代全部记录。
    pub fn iter(&self) -> impl Iterator<Item = (&str, &TlPackage)> {
        self.packages.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// 某 basename 的全部提供者（已按 TDS 偏好序排序）。
    pub fn providers_of(&self, file_name: &str) -> &[ProviderRef] {
        self.by_basename
            .get(file_name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// 某规范化路径的全部提供者。
    pub fn providers_of_path(&self, path: &str) -> &[ProviderRef] {
        self.by_path
            .get(&normalize_path(path))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// 定位提供某文件的包。
    ///
    /// - 含 `/` 视为路径（可带或不带 `texmf-dist/` 前缀），走路径索引；
    /// - 否则视为 basename，走文件名索引。
    ///
    /// 多个提供者时返回排序首项；需要全貌请用 [`TlPdb::providers_of`]。
    pub fn find_by_file(&self, file: &str) -> Option<&ProviderRef> {
        if file.contains('/') {
            self.providers_of_path(file).first()
        } else {
            self.providers_of(file).first()
        }
    }

    /// 描述统计。
    pub fn stats(&self) -> PdbStats {
        let mut s = PdbStats {
            records: self.packages.len(),
            distinct_basenames: self.by_basename.len(),
            ..PdbStats::default()
        };
        for pkg in self.packages.values() {
            match &pkg.category {
                Some(Category::Package) => s.packages += 1,
                Some(Category::Collection) => s.collections += 1,
                Some(Category::Scheme) => s.schemes += 1,
                Some(Category::TlCore) => s.tl_core += 1,
                _ => {}
            }
            s.run_files += pkg.run_files.len();
            s.bin_files += pkg.bin_files.len();
            if pkg.container_sha512.is_some() {
                s.with_checksum += 1;
            }
        }
        s
    }
}

/// TDS 目录偏好序：数字小者优先。
///
/// **简化**：真实 kpathsea 按 `TEXINPUTS`/`texmf.cnf` 的路径顺序查找，这里只用一条
/// 固定的 TDS 层级序（`tex/latex` → `tex/generic` → `tex` → `fonts` → …）。
/// 与真实 TeX 的差异仅出现在「同一 basename 有多个不同层级的提供者」时，
/// 见 docs/KNOWN-SIMPLIFICATIONS.md。
fn tds_rank(path: &str) -> u8 {
    let p = path.strip_prefix("texmf-dist/").unwrap_or(path);
    if p.starts_with("tex/latex/") {
        0
    } else if p.starts_with("tex/generic/") {
        1
    } else if p.starts_with("tex/") {
        2
    } else if p.starts_with("fonts/") {
        3
    } else if p.starts_with("scripts/") {
        4
    } else if p.starts_with("bin/") {
        6
    } else if p.starts_with("doc/") {
        9
    } else {
        5
    }
}

fn sort_providers(bucket: &mut [ProviderRef]) {
    bucket.sort_by(|a, b| {
        (tds_rank(&a.path), a.path.as_str(), a.package.as_str()).cmp(&(
            tds_rank(&b.path),
            b.path.as_str(),
            b.package.as_str(),
        ))
    });
}

/// 规范化路径：去掉前缀 `./` 与 `texmf-dist/`，统一分隔符为 `/`。
pub fn normalize_path(path: &str) -> String {
    let p = path.strip_prefix("./").unwrap_or(path);
    let p = p.strip_prefix("texmf-dist/").unwrap_or(p);
    p.replace('\\', "/")
}

fn strip_cr(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

/// 按「首个空白串」拆出字段名与值（对齐官方 `split(/\s+/, $line, 2)`）。
///
/// 值侧去首尾空白；字段名与值之间允许多个空白（`execute` 行常见）。
fn split_label(line: &str) -> (&str, &str) {
    match line.find(char::is_whitespace) {
        Some(idx) => (&line[..idx], line[idx..].trim()),
        None => (line, ""),
    }
}

fn parse_u64(s: &str, line: usize, field: &'static str) -> Result<u64, Error> {
    s.parse::<u64>().map_err(|_| Error::Tlpdb {
        line,
        field,
        detail: format!("`{s}` 不是非负整数"),
    })
}

/// 取文件清单段声明的 `size=` 标签（RIV 块数）。缺失不是错误——官方解析器也不要求它。
fn parse_size_tag(rest: &str, line: usize, field: &'static str) -> Result<Option<u64>, Error> {
    for token in rest.split_whitespace() {
        if let Some(v) = token.strip_prefix("size=") {
            return v.parse::<u64>().map(Some).map_err(|_| Error::Tlpdb {
                line,
                field,
                detail: format!("`size={v}` 不是非负整数"),
            });
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdata::mini;

    #[test]
    fn parses_records_and_fields() {
        let db = mini();
        assert_eq!(db.len(), 6);

        let g = db.get("geometry").expect("geometry 存在");
        assert_eq!(g.category, Some(Category::Package));
        assert_eq!(g.revision, Some(70501));
        assert_eq!(g.short_desc.as_deref(), Some("Show a page layout"));
        assert_eq!(
            g.run_files.len(),
            3,
            "doc 文件也计入 runfiles（清单原样保留）"
        );
        assert_eq!(g.bin_files.len(), 1);
        assert_eq!(g.container_size, Some(4096));
        assert!(g.container_sha512.is_some());
        assert_eq!(g.depends.len(), 4);
    }

    /// **回归锁（真实数据形态）**：`size=` 是 RIV 块数，与行数无关，绝不能用来决定读几行。
    ///
    /// TL2024 的 `ae` 包声明 `runfiles size=157` 而实际只有 114 行；本夹具取更极端的
    /// 「声明 157、实际 3 行」——按旧实现（读 N 行）必然报错。
    #[test]
    fn runfiles_size_is_a_block_hint_not_a_line_count() {
        let db = mini();
        let g = db.get("geometry").expect("geometry 存在");
        assert_eq!(g.run_size_blocks, Some(157), "声明的 RIV 块数应被登记");
        assert_eq!(
            g.run_files.len(),
            3,
            "文件行数由续行规则决定，与 size= 无关"
        );
        assert_eq!(g.run_size_bytes(), Some(157 * BLOCK_SIZE));
    }

    /// 段边界：遇到新的非缩进行即结束，后续字段不得被吞进清单。
    #[test]
    fn file_section_ends_at_next_command() {
        let db = mini();
        let g = db.get("geometry").expect("geometry 存在");
        assert!(
            !g.run_files.iter().any(|f| f.contains("catalogue")),
            "段结束后的字段名不得混入文件清单：{:?}",
            g.run_files
        );
        assert!(
            g.run_files.iter().all(|f| f.starts_with("texmf-dist/")),
            "清单条目必须都是路径：{:?}",
            g.run_files
        );
    }

    #[test]
    fn package_deps_filters_and_expands_arch() {
        let db = mini();
        let g = db.get("geometry").expect("geometry 存在");
        // release/2024 与 opt_* 被剔除；.ARCH 展开为当前平台
        assert_eq!(
            g.package_deps("universal-darwin"),
            vec!["atbegshi".to_owned(), "pdftex.universal-darwin".to_owned()]
        );
        assert_eq!(g.package_deps("x86_64-linux")[1], "pdftex.x86_64-linux");
    }

    #[test]
    fn basename_index_orders_by_tds_rank_and_localizes() {
        let db = mini();
        assert_eq!(
            db.find_by_file("geometry.sty").map(|p| p.package.as_str()),
            Some("geometry")
        );

        // 同名副本：tex/latex/base 与 tex/latex/l3kernel 都是 rank 0，按路径字典序取 base
        let providers = db.providers_of("expl3-code.tex");
        assert_eq!(providers.len(), 2, "两个提供者都必须可见（不静默丢一个）");
        assert_eq!(providers[0].package, "base");
        assert_eq!(providers[1].package, "l3kernel");
        assert_eq!(
            db.find_by_file("expl3-code.tex")
                .map(|p| p.package.as_str()),
            Some("base")
        );

        // 文件名索引命中 doc 路径的提供者（清单保留原样，不做用途过滤）
        assert_eq!(
            db.find_by_file("geometry.pdf").map(|p| p.package.as_str()),
            Some("geometry")
        );
    }

    #[test]
    fn path_lookup_accepts_prefixed_and_stripped_forms() {
        let db = mini();
        let a = db.find_by_file("texmf-dist/tex/latex/geometry/geometry.sty");
        let b = db.find_by_file("tex/latex/geometry/geometry.sty");
        assert_eq!(a.map(|p| p.package.as_str()), Some("geometry"));
        assert_eq!(b.map(|p| p.package.as_str()), Some("geometry"));
        assert_eq!(db.find_by_file("tex/latex/geometry/nope.sty"), None);
    }

    #[test]
    fn unknown_file_has_no_provider() {
        let db = mini();
        assert!(db.providers_of("no-such-file.sty").is_empty());
        assert!(db.find_by_file("no-such-file.sty").is_none());
    }

    #[test]
    fn stats_counts_categories() {
        let s = mini().stats();
        assert_eq!(s.records, 6);
        assert_eq!(s.packages, 5);
        assert_eq!(s.tl_core, 1);
        assert_eq!(s.schemes, 0);
        assert_eq!(s.collections, 0);
        assert_eq!(s.run_files, 8);
        assert_eq!(s.bin_files, 1);
        assert_eq!(s.with_checksum, 2);
        assert_eq!(s.distinct_basenames, 7);
    }

    #[test]
    fn indented_line_outside_section_is_reported() {
        let text = "name x\ncategory Package\n stray-line\n";
        match TlPdb::parse(text) {
            Err(Error::Tlpdb { line, field, .. }) => {
                assert_eq!(line, 3);
                assert_eq!(field, "(缩进行)");
            }
            other => panic!("应报非法缩进行，实际 {other:?}"),
        }
    }

    #[test]
    fn duplicate_record_is_rejected() {
        let text = "name x\nname x\n";
        match TlPdb::parse(text) {
            Err(Error::Tlpdb { field, detail, .. }) => {
                assert_eq!(field, "name");
                assert!(detail.contains("第二个"), "detail: {detail}");
            }
            other => panic!("应报重复记录，实际 {other:?}"),
        }
    }

    /// 记录之间缺空行时必须报错，而不是悄悄切成两个包。
    #[test]
    fn missing_blank_line_between_records_is_rejected() {
        let text = "name a\ncategory Package\nname b\ncategory Package\n";
        match TlPdb::parse(text) {
            Err(Error::Tlpdb {
                line,
                field,
                detail,
            }) => {
                assert_eq!(line, 3);
                assert_eq!(field, "name");
                assert!(detail.contains("空行"), "detail 应提示分隔空行：{detail}");
            }
            other => panic!("应报记录粘连，实际 {other:?}"),
        }
    }

    #[test]
    fn field_before_name_is_rejected() {
        let text = "category Package\n";
        match TlPdb::parse(text) {
            Err(Error::Tlpdb { line, field, .. }) => {
                assert_eq!(line, 1);
                assert_eq!(field, "name");
            }
            other => panic!("应报记录未以 name 起始，实际 {other:?}"),
        }
    }

    #[test]
    fn crlf_input_is_tolerated() {
        let text = "name x\r\ncategory Package\r\nrunfiles size=1\r\n texmf-dist/a.sty\r\n";
        let db = TlPdb::parse(text).expect("CRLF 输入应可解析（镜像站换行差异）");
        assert_eq!(db.get("x").map(|p| p.run_files.len()), Some(1));
        assert_eq!(db.providers_of("a.sty").len(), 1);
    }

    #[test]
    fn unknown_fields_are_ignored_for_forward_compat() {
        let text =
            "name x\ncategory Package\nbrand-new-field 1\nrunfiles size=1\n texmf-dist/a.sty\n";
        let db = TlPdb::parse(text).expect("未知字段应被忽略");
        assert_eq!(db.providers_of("a.sty").len(), 1);
    }

    #[test]
    fn missing_size_tag_is_not_an_error() {
        let text = "name x\nrunfiles\n texmf-dist/a.sty\n";
        let db = TlPdb::parse(text).expect("官方解析器也不要求 size=，缺失不应报错");
        assert_eq!(db.get("x").and_then(|p| p.run_size_blocks), None);
        assert_eq!(db.providers_of("a.sty").len(), 1);
    }

    #[test]
    fn empty_input_is_empty_db() {
        let db = TlPdb::parse("").expect("空输入应成功");
        assert!(db.is_empty());
        assert_eq!(db.stats().records, 0);
    }

    #[test]
    fn host_arch_is_known_on_supported_platforms() {
        // 本仓库开发机为 macOS/Linux；未知平台返回空串（.ARCH 依赖转「未解析」）
        let a = host_arch();
        assert!(
            a.is_empty() || a == "universal-darwin" || a.ends_with("-linux") || a == "windows",
            "host_arch 取值异常：{a}"
        );
    }

    #[test]
    fn normalize_path_strips_prefixes() {
        assert_eq!(normalize_path("./texmf-dist/a/b.sty"), "a/b.sty");
        assert_eq!(normalize_path("texmf-dist/a/b.sty"), "a/b.sty");
        assert_eq!(normalize_path("a/b.sty"), "a/b.sty");
    }

    #[test]
    fn split_label_handles_multi_space_and_trailing() {
        assert_eq!(split_label("name geometry"), ("name", "geometry"));
        assert_eq!(
            split_label("execute  AddFormat   x"),
            ("execute", "AddFormat   x")
        );
        assert_eq!(split_label("plainword"), ("plainword", ""));
        assert_eq!(split_label("shortdesc   "), ("shortdesc", ""));
    }
}
