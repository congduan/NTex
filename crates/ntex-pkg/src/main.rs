//! ntex-pkg 命令行：把解析层 / 契约层 / 取料层跑在真实 TLPDB 上。
//!
//! 文件访问一律走 [`ntex_io::Vfs`]（RFC-3）——本 CLI 与将来的 WASM 宿主共用同一条
//! 副作用边界，因此这里的证据对两端都成立。
//!
//! 退出码：`0` 正常 · `1` 出错 · `3` `check` 发现锁文件已过期（便于进 CI）。

use std::process::ExitCode;

use ntex_io::{LocalVfs, Vfs};
use ntex_pkg::cache;
use ntex_pkg::lock::{self, LockedPackage};
use ntex_pkg::resolve::{self, Closure, RequireKind};
use ntex_pkg::source::{LocalTexLiveSource, PackageSource, ResolveOutcome, SourceChain};
use ntex_pkg::tlnet::{self, HostContainerIo, TlnetSource};
use ntex_pkg::tlpdb::{host_arch, TlPdb};
use ntex_pkg::vendor::{self, PlannedFile};
use ntex_pkg::Error;

/// `check` 发现锁漂移时的退出码。
const EXIT_DRIFT: u8 = 3;

const USAGE: &str = "\
ntex-pkg — NTex 宏包解析与取料（M9 · plan.md §6.2 第 8/9 条）

用法：
  ntex-pkg index   <tlpdb>
      统计 TLPDB：记录数、类别分布、文件数、校验和覆盖率。

  ntex-pkg provide <tlpdb> <文件名|相对路径>...
      反查提供者（等价 kpsewhich / tlmgr search --global --file）。

  ntex-pkg resolve <tlpdb> [--documentclass 名]... [--input 文件]... [包名]...
      需求 → 提供包 → 依赖闭包；打印闭包、未解析项、不可校验项与体量估算。

  ntex-pkg lock    <tlpdb> [--documentclass 名]... [--input 文件]... [包名]...
      生成 ntex.lock 并写到 stdout（确定性编码，可直接入库）。

  ntex-pkg check   <tlpdb> <ntex.lock>
      检查锁文件相对当前 TLPDB 是否过期（退出码 3 = 已漂移）。

  ntex-pkg local   <tlpdb> <TL 树根> <包名|文件名>...
      探测包在本地 TeX Live 树上的存在情况（① 源，零下载）。

  ntex-pkg vendor  <tlpdb> <TL 树根> <目标目录> [--write] [--lock <路径>] [需求...]
      把依赖闭包**物化**成目标目录下的一棵 TDS 子树（默认 dry-run，只报差异）。
      `--write` 落盘；`--lock <路径>` 同时写出闭包的 ntex.lock（把资产集钉死）。
      这是「内置名单 ≡ 闭包」的执行端：资产集由种子名单推导，不靠手抄目录。

  ntex-pkg fetch   <tlpdb> <仓库 URL> <缓存根> [需求...]
      从 tlnet 镜像把闭包取到本地，铺成一棵**可当 TL 树根用**的缓存树
      （`<缓存根>/tlpkg/texlive.tlpdb` + `<缓存根>/texmf-dist/**`）。
      每个容器都按 TLPDB 的 SHA-512 校验；随后可把 <缓存根> 当 <TL 树根> 喂给 vendor。
";

/// CLI 错误：库错误 + 用法错误（用法问题不该污染库的错误模型）。
enum CliError {
    Pkg(Error),
    Usage(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Pkg(e) => write!(f, "{e}"),
            CliError::Usage(m) => write!(f, "{m}\n\n{USAGE}"),
        }
    }
}

impl From<Error> for CliError {
    fn from(e: Error) -> Self {
        CliError::Pkg(e)
    }
}

fn usage(msg: impl Into<String>) -> CliError {
    CliError::Usage(msg.into())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("ntex-pkg: {e}");
            ExitCode::from(1)
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, CliError> {
    let Some((cmd, rest)) = args.split_first() else {
        print!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    };
    match cmd.as_str() {
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        "index" => cmd_index(rest),
        "provide" => cmd_provide(rest),
        "resolve" => cmd_resolve(rest),
        "lock" => cmd_lock(rest),
        "check" => cmd_check(rest),
        "local" => cmd_local(rest),
        "vendor" => cmd_vendor(rest),
        "fetch" => cmd_fetch(rest),
        other => Err(usage(format!("未知子命令 `{other}`"))),
    }
}

/// 经 VFS 读文本（RFC-3：本 CLI 不直接碰 `std::fs`）。
fn read_text(vfs: &mut dyn Vfs, path: &str, intent: &'static str) -> Result<String, CliError> {
    let bytes = vfs
        .read(path)
        .map_err(|e| Error::io(intent, e))?
        .ok_or(Error::MissingFile {
            path: path.to_owned(),
            intent,
        })?;
    String::from_utf8(bytes).map_err(|e| {
        CliError::Pkg(Error::MissingFile {
            path: format!("{path}（非 UTF-8：{e}）"),
            intent,
        })
    })
}

/// 读并解析 TLPDB。
fn load_pdb(vfs: &mut dyn Vfs, path: &str) -> Result<TlPdb, CliError> {
    let text = read_text(vfs, path, "读取 TLPDB")?;
    Ok(TlPdb::parse(&text)?)
}

/// 解析 `[--documentclass 名] [--input 文件] [包名]...` 形式的需求列表。
fn parse_requests(rest: &[String]) -> Result<Vec<(String, RequireKind)>, CliError> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < rest.len() {
        match rest[i].as_str() {
            "--documentclass" | "--class" => {
                let v = rest
                    .get(i + 1)
                    .ok_or_else(|| usage("`--documentclass` 缺少取值"))?;
                out.push((v.clone(), RequireKind::DocumentClass));
                i += 2;
            }
            "--input" => {
                let v = rest.get(i + 1).ok_or_else(|| usage("`--input` 缺少取值"))?;
                out.push((v.clone(), RequireKind::Input));
                i += 2;
            }
            flag if flag.starts_with("--") => {
                return Err(usage(format!("未知选项 `{flag}`")));
            }
            name => {
                out.push((name.to_owned(), RequireKind::UsePackage));
                i += 1;
            }
        }
    }
    Ok(out)
}

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

fn short_sha(sha: &str) -> String {
    sha.chars().take(12).collect()
}

fn cmd_index(rest: &[String]) -> Result<ExitCode, CliError> {
    let Some(path) = rest.first() else {
        return Err(usage("`index` 需要 <tlpdb> 路径"));
    };
    let mut vfs = LocalVfs;
    let db = load_pdb(&mut vfs, path)?;
    let s = db.stats();
    println!("TLPDB：{path}");
    println!(
        "  记录 {} = Package {} · TLCore {} · Collection {} · Scheme {}",
        s.records, s.packages, s.tl_core, s.collections, s.schemes
    );
    println!(
        "  文件 runfiles {} · binfiles {}（去重 basename {}）",
        s.run_files, s.bin_files, s.distinct_basenames
    );
    let covered = if s.records == 0 {
        0.0
    } else {
        s.with_checksum as f64 * 100.0 / s.records as f64
    };
    println!(
        "  带 container-sha512 的记录 {}/{}（{:.1}%）",
        s.with_checksum, s.records, covered
    );
    println!("  本机平台（.ARCH 展开目标）：{}", host_arch());
    if let Some(pkg) = db.get("00texlive.installation") {
        if let Some(loc) = pkg
            .depends
            .iter()
            .find_map(|d| d.strip_prefix("opt_location:"))
        {
            println!("  该树的上游仓库：{loc}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_provide(rest: &[String]) -> Result<ExitCode, CliError> {
    let (path, files) = rest
        .split_first()
        .ok_or_else(|| usage("`provide` 需要 <tlpdb> 路径"))?;
    if files.is_empty() {
        return Err(usage("`provide` 需要至少一个文件名"));
    }
    let mut vfs = LocalVfs;
    let db = load_pdb(&mut vfs, path)?;
    for file in files {
        let providers = if file.contains('/') {
            db.providers_of_path(file)
        } else {
            db.providers_of(file)
        };
        if providers.is_empty() {
            println!("{file}: 无提供者");
            continue;
        }
        println!("{file}: {} 个提供者", providers.len());
        for p in providers {
            println!("  {} ← {}", p.package, p.path);
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// 需求 → 闭包；`resolve` 与 `lock` 共用。
fn build_closure(
    vfs: &mut dyn Vfs,
    path: &str,
    rest: &[String],
) -> Result<(TlPdb, Closure), CliError> {
    let requests = parse_requests(rest)?;
    if requests.is_empty() {
        return Err(usage("需要至少一个包名（或 --documentclass / --input）"));
    }
    let db = load_pdb(vfs, path)?;
    let closure = resolve::closure_for_requires(&db, &requests, host_arch())?;
    Ok((db, closure))
}

fn cmd_resolve(rest: &[String]) -> Result<ExitCode, CliError> {
    let (path, tail) = rest
        .split_first()
        .ok_or_else(|| usage("`resolve` 需要 <tlpdb> 路径"))?;
    let mut vfs = LocalVfs;
    let (db, closure) = build_closure(&mut vfs, path, tail)?;

    println!("需求解析：");
    for s in &closure.seeds {
        println!("  {} → {}", s.request, s.file);
        for (i, p) in s.providers.iter().enumerate() {
            let mark = if i == 0 { "选定" } else { "备选" };
            println!("    {mark} 包 {} ← {}", p.package, p.path);
        }
    }

    println!(
        "\n依赖闭包（{} 个包，按 containersize 估算取料 {}）：",
        closure.packages.len(),
        human_bytes(closure.download_bytes)
    );
    for name in &closure.packages {
        match db.get(name) {
            Some(p) => println!(
                "  {:<12} {:<32} rev {:<8} {}",
                p.category
                    .as_ref()
                    .map(|c| c.as_str().to_owned())
                    .unwrap_or_else(|| "-".to_owned()),
                name,
                p.revision
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                human_bytes(p.container_size.unwrap_or(0)),
            ),
            None => println!("  {:<12} {name}", "(缺记录)"),
        }
    }
    println!("  运行面文件合计 {}", closure.run_file_count(&db));

    let hist = closure.category_histogram(&db);
    let hist_line: Vec<String> = hist.iter().map(|(k, v)| format!("{k} {v}")).collect();
    println!("  类别分布：{}", hist_line.join(" · "));

    if closure.unresolved.is_empty() {
        println!("\n未解析依赖：无");
    } else {
        println!("\n未解析依赖（{}）：", closure.unresolved.len());
        for u in &closure.unresolved {
            println!("  {} ← {}", u.dep, u.from);
        }
    }

    let unverifiable = closure.unverifiable(&db);
    if unverifiable.is_empty() {
        println!("不可校验（无 sha512）：无——闭包可完整锁定");
    } else {
        println!(
            "不可校验（无 sha512，{}）：{}",
            unverifiable.len(),
            unverifiable.join(", ")
        );
    }
    println!(
        "\n结论：{}",
        if closure.is_clean(&db) {
            "闭包干净（无悬空依赖、可完整锁定）"
        } else {
            "闭包不干净——见上方未解析 / 不可校验项"
        }
    );
    Ok(ExitCode::SUCCESS)
}

fn cmd_lock(rest: &[String]) -> Result<ExitCode, CliError> {
    let (path, tail) = rest
        .split_first()
        .ok_or_else(|| usage("`lock` 需要 <tlpdb> 路径"))?;
    let mut vfs = LocalVfs;
    let (db, closure) = build_closure(&mut vfs, path, tail)?;
    let items = lock::from_closure(&db, &closure)?;
    print!("{}", lock::encode(&items)?);
    Ok(ExitCode::SUCCESS)
}

fn cmd_check(rest: &[String]) -> Result<ExitCode, CliError> {
    let Some(tlpdb_path) = rest.first() else {
        return Err(usage("`check` 需要 <tlpdb> 路径"));
    };
    let Some(lock_path) = rest.get(1) else {
        return Err(usage("`check` 需要 <ntex.lock> 路径"));
    };
    let mut vfs = LocalVfs;
    let db = load_pdb(&mut vfs, tlpdb_path)?;
    let locked = lock::decode(&read_text(&mut vfs, lock_path, "读取锁文件")?)?;

    let mut current = Vec::with_capacity(locked.len());
    for item in &locked {
        if let Some(pkg) = db.get(&item.name) {
            let (Some(revision), Some(sha512)) = (pkg.revision, pkg.container_sha512.clone())
            else {
                return Err(CliError::Pkg(Error::NotLockable {
                    package: item.name.clone(),
                    reason: "当前 TLPDB 未给出 revision / container-sha512".to_owned(),
                }));
            };
            current.push(LockedPackage {
                name: item.name.clone(),
                category: pkg
                    .category
                    .as_ref()
                    .map(|c| c.as_str().to_owned())
                    .unwrap_or_default(),
                revision,
                size: pkg.container_size.unwrap_or(0),
                sha512,
                files: pkg.run_files.len(),
            });
        }
    }

    let d = lock::diff(&locked, &current);
    println!(
        "锁文件 {lock_path}：{} 个包（源 TLPDB {tlpdb_path}）",
        locked.len()
    );
    println!(
        "  changed {} · removed {} · added {}",
        d.changed.len(),
        d.removed.len(),
        d.added.len()
    );
    for name in &d.changed {
        let before = locked.iter().find(|p| &p.name == name);
        let after = current.iter().find(|p| &p.name == name);
        if let (Some(b), Some(a)) = (before, after) {
            println!(
                "    {name}：rev {} → {}，sha {} → {}",
                b.revision,
                a.revision,
                short_sha(&b.sha512),
                short_sha(&a.sha512)
            );
        }
    }
    for name in &d.removed {
        println!("    {name}：TLPDB 中已不存在");
    }

    if d.is_empty() {
        println!("\n结论：锁与当前 TLPDB 一致（可复现）");
        Ok(ExitCode::SUCCESS)
    } else {
        println!(
            "\n结论：锁已{}，请重生成——`ntex-pkg lock {tlpdb_path} … > {lock_path}`",
            if d.alters_output() {
                "漂移（版本/校验和变了，会改变产出字节）"
            } else {
                "漂移（仅包集合增减）"
            }
        );
        Ok(ExitCode::from(EXIT_DRIFT))
    }
}

fn cmd_local(rest: &[String]) -> Result<ExitCode, CliError> {
    let Some(tlpdb_path) = rest.first() else {
        return Err(usage("`local` 需要 <tlpdb> 路径"));
    };
    let Some(root) = rest.get(1) else {
        return Err(usage("`local` 需要 <TL 树根> 路径"));
    };
    let names = &rest[2..];
    if names.is_empty() {
        return Err(usage("`local` 需要至少一个包名"));
    }
    let mut vfs = LocalVfs;
    let db = load_pdb(&mut vfs, tlpdb_path)?;
    let src = LocalTexLiveSource::new(root.clone());
    let mut chain = SourceChain::new();
    chain.push(Box::new(src.clone()));

    println!("本地树 {root}（可用：{}）", src.is_available(&mut vfs));
    for name in names {
        let Some(pkg) = db.get(name) else {
            println!("\n{name}：TLPDB 中无此包");
            continue;
        };
        match chain.resolve(pkg, &mut vfs)? {
            ResolveOutcome::LocalComplete(s) => {
                println!(
                    "\n{name}：已完整存在（{} 个运行面文件，零下载）",
                    s.present.len()
                );
            }
            ResolveOutcome::LocalPartial(s) => {
                println!(
                    "\n{name}：{} / {}（覆盖率 {:.1}%）——需补齐 {} 个文件",
                    s.present.len(),
                    s.present.len() + s.missing.len(),
                    s.coverage() * 100.0,
                    s.missing.len()
                );
                for m in s.missing.iter().take(10) {
                    println!("    缺 {m}");
                }
                if s.missing.len() > 10 {
                    println!("    …（另 {} 个）", s.missing.len() - 10);
                }
            }
            ResolveOutcome::Container { kind, bytes } => {
                println!(
                    "\n{name}：由 {} 取到容器 {}",
                    kind.label(),
                    human_bytes(bytes as u64)
                );
            }
            ResolveOutcome::Unavailable { tried } => {
                let tried: Vec<&str> = tried.iter().map(|k| k.label()).collect();
                println!(
                    "\n{name}：无线下载（可尝试源：{}）；缓存目录约定 {}",
                    if tried.is_empty() {
                        "无".to_owned()
                    } else {
                        tried.join(", ")
                    },
                    match pkg.revision {
                        Some(r) => cache::package_dir(cache::DEFAULT_CACHE_ROOT, name, r)?,
                        None => "(无 revision，不可锁定)".to_owned(),
                    }
                );
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `vendor` 的选项。
struct VendorArgs {
    tlpdb: String,
    root: String,
    target: String,
    write: bool,
    lock_path: Option<String>,
    requests: Vec<(String, RequireKind)>,
}

/// 解析 `vendor` 参数：**前三个自由参数固定为 `<tlpdb> <TL 根> <目标目录>`**，
/// 其后的自由参数才是需求名（避免"包名与位置参数混在一起"的歧义）。
fn parse_vendor_args(rest: &[String]) -> Result<VendorArgs, CliError> {
    let mut free: Vec<String> = Vec::new();
    let mut write = false;
    let mut lock_path: Option<String> = None;
    let mut req_tokens: Vec<String> = Vec::new();

    let mut i = 0usize;
    while i < rest.len() {
        match rest[i].as_str() {
            "--write" => {
                write = true;
                i += 1;
            }
            "--dry-run" => {
                write = false;
                i += 1;
            }
            "--lock" => {
                let v = rest.get(i + 1).ok_or_else(|| usage("`--lock` 缺少取值"))?;
                lock_path = Some(v.clone());
                i += 2;
            }
            "--documentclass" | "--class" | "--input" => {
                let v = rest
                    .get(i + 1)
                    .ok_or_else(|| usage(format!("`{}` 缺少取值", rest[i])))?;
                req_tokens.push(rest[i].clone());
                req_tokens.push(v.clone());
                i += 2;
            }
            flag if flag.starts_with("--") => {
                return Err(usage(format!("`vendor` 未知选项 `{flag}`")));
            }
            other => {
                free.push(other.to_owned());
                i += 1;
            }
        }
    }

    if free.len() < 3 {
        return Err(usage(
            "`vendor` 需要三个位置参数：<tlpdb> <TL 树根> <目标目录>",
        ));
    }
    // 第 4 个起的自由参数是包名（需求）。
    req_tokens.extend(free[3..].iter().cloned());
    let requests = parse_requests(&req_tokens)?;
    if requests.is_empty() {
        return Err(usage(
            "`vendor` 需要至少一个需求（包名，或 --documentclass / --input）",
        ));
    }
    Ok(VendorArgs {
        tlpdb: free[0].clone(),
        root: free[1].clone(),
        target: free[2].clone(),
        write,
        lock_path,
        requests,
    })
}

/// 列出待写/缺失条目（截断，避免刷屏）。
fn print_file_list(title: &str, files: &[&PlannedFile], limit: usize) {
    if files.is_empty() {
        return;
    }
    println!("\n{title}（{}）：", files.len());
    for f in files.iter().take(limit) {
        println!("  {:<28} {}", f.package, f.rel_path);
    }
    if files.len() > limit {
        println!("  …（另 {} 个）", files.len() - limit);
    }
}

fn cmd_vendor(rest: &[String]) -> Result<ExitCode, CliError> {
    let a = parse_vendor_args(rest)?;
    let mut vfs = LocalVfs;
    let db = load_pdb(&mut vfs, &a.tlpdb)?;
    let closure = resolve::closure_for_requires(&db, &a.requests, host_arch())?;
    let src = LocalTexLiveSource::new(a.root.clone());

    let plan = vendor::plan(&db, &closure, &src, &a.target, &mut vfs)?;
    print!("{}", plan.summary());

    print_file_list("待写入", &plan.pending(), 40);
    let missing = plan.source_missing();
    if !missing.is_empty() {
        println!(
            "\n源树缺失（{}）——TLPDB 声明了这些运行面文件，但 `{}` 里读不到：",
            missing.len(),
            a.root
        );
        for f in missing.iter().take(40) {
            println!("  {:<28} {}", f.package, f.rel_path);
        }
        if missing.len() > 40 {
            println!("  …（另 {} 个）", missing.len() - 40);
        }
    }

    if let Some(lock_path) = &a.lock_path {
        let items = lock::from_closure(&db, &closure)?;
        let text = lock::encode(&items)?;
        vfs.write(lock_path, text.as_bytes())
            .map_err(|e| Error::io("写出锁文件", e))?;
        println!("\n锁文件已写出 {lock_path}（{} 个包）", items.len());
    }

    if a.write {
        let report = vendor::apply(&plan, &a.target, &mut vfs)?;
        println!(
            "\n已写入 {}（新增 {} · 刷新 {}，{} 字节）· 跳过一致 {} · 跳过源缺失 {}",
            report.written_files,
            report.added,
            report.refreshed,
            human_bytes(report.written_bytes),
            report.skipped_identical,
            report.skipped_source_missing,
        );
        if !report.source_missing_after_plan.is_empty() {
            println!(
                "计划后消失的源文件（{}）：{}",
                report.source_missing_after_plan.len(),
                report.source_missing_after_plan.join(", ")
            );
        }
    } else {
        println!("\n（dry-run；加 `--write` 落盘）");
    }

    // 取料不完整必须非零退出——否则 CI 会把"半个资产集"当成功。
    if !missing.is_empty() {
        println!("\n结论：源树缺 {} 个文件，资产集不完整", missing.len());
        return Ok(ExitCode::from(1));
    }
    println!(
        "\n结论：{}",
        if plan.is_settled() {
            "目标树与闭包一致（无需动作）"
        } else if a.write {
            "已按闭包补齐目标树"
        } else {
            "目标树落后于闭包（上方「待写入」）"
        }
    );
    Ok(ExitCode::SUCCESS)
}

fn cmd_fetch(rest: &[String]) -> Result<ExitCode, CliError> {
    let (tlpdb_path, tail) = rest
        .split_first()
        .ok_or_else(|| usage("`fetch` 需要 <tlpdb> 路径"))?;
    let (repo, tail) = tail
        .split_first()
        .ok_or_else(|| usage("`fetch` 需要 <仓库 URL>（tlnet 根，不含 archive/）"))?;
    let (cache_root, tail) = tail
        .split_first()
        .ok_or_else(|| usage("`fetch` 需要 <缓存根>"))?;
    let requests = parse_requests(tail)?;
    if requests.is_empty() {
        return Err(usage(
            "`fetch` 需要至少一个需求（包名，或 --documentclass / --input）",
        ));
    }

    let mut vfs = LocalVfs;
    let text = read_text(&mut vfs, tlpdb_path, "读取 TLPDB")?;
    let db = TlPdb::parse(&text)?;
    let closure = resolve::closure_for_requires(&db, &requests, host_arch())?;

    println!("TLPDB {tlpdb_path}\n仓库 {repo}\n缓存根 {cache_root}");
    println!("闭包 {} 个包", closure.packages.len());

    // 先打印全部待取 URL：失败要能归因到具体包，而不是"下到一半炸了"。
    let mut src = TlnetSource::new(repo.clone(), Box::new(HostContainerIo::default()));
    let mut planned = 0usize;
    for name in &closure.packages {
        let Some(pkg) = db.get(name) else { continue };
        if pkg.run_files.is_empty() {
            println!("  跳过 {name}（无运行面文件，仅 binfiles）");
            continue;
        }
        println!("  {}", src.container_url(pkg)?);
        planned += 1;
    }
    println!("\n开始取料（{planned} 个容器）…");

    let report = tlnet::materialize(&mut src, &db, &closure, cache_root, &text, &mut vfs)?;
    println!(
        "\n已取到 {} 个容器 · 落地 {} 个文件（{}）",
        report.containers,
        report.files,
        human_bytes(report.bytes)
    );
    if !report.text_free.is_empty() {
        println!(
            "无运行面文件（仅 binfiles，跳过）：{}",
            report.text_free.join(", ")
        );
    }
    println!(
        "\n下一步（把缓存树当 TL 树根物化进资产）：\n  ntex-pkg vendor {tlpdb_path} {cache_root} <目标目录> [--write] …"
    );
    Ok(ExitCode::SUCCESS)
}
