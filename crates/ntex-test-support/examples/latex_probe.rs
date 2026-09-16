//! 勘察工具：以 iniTeX 方式加载一个真实格式源（如 latex.ltx），报告首个阻塞点。
//!
//! 用法：
//! ```text
//! cargo run -p ntex-test-support --example latex_probe -- <path/to/latex.ltx> [--shim]
//! ```
//! `--shim`：在源前注入 INITEX 默认 catcode 归位（引擎初始表是 plain 风格，
//! latex.ltx 首个 `\ifnum\catcode`\{=1` 检查会误判为"已预载格式"）。

use anyhow::{Context, Result};
use ntex_io::{MemVfs, Vfs};

const INITEX_CATCODE_SHIM: &str = concat!(
    "\\catcode`\\{=12 \\catcode`\\}=12 \\catcode`\\$=12 \\catcode`\\&=12 ",
    "\\catcode`\\#=12 \\catcode`\\^=12 \\catcode`\\_=12 \\catcode`\\~=12 ",
    "\\catcode`\\^^I=12 \n"
);

/// 勘察用 VFS：只读目录映射（CTAN 下载的内核支持文件），写走内存。
#[derive(Debug)]
struct SurveyVfs {
    root: String,
    /// 额外查找目录（CLI `--input-path`，主 root miss 后依次尝试）。
    extra_roots: Vec<String>,
    mem: MemVfs,
}

/// 路径归一化：剥掉前导 `./`（RFC-3 不解析路径语义；此处是勘察工具侧让内存
/// 后端对齐真实文件系统——LocalVfs 天然把 `./x` 与 `x` 视为同一文件，而
/// MemVfs 是精确字符串键。latex.ltx L195 的 `\@currdir` 探测正依赖这一语义：
/// `\immediate\openout15=texsys.aux` 后 `\IfFileExists{./texsys.aux}`）。
fn normalize(path: &str) -> String {
    let mut p = path;
    while let Some(rest) = p.strip_prefix("./") {
        p = rest;
    }
    p.to_owned()
}

impl SurveyVfs {
    #[allow(dead_code)] // mem 直构后保留的便捷构造（工具示例代码）
    fn new(root: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            extra_roots: Vec::new(),
            mem: MemVfs::new(),
        }
    }
}

impl Vfs for SurveyVfs {
    fn read(&mut self, path: &str) -> std::io::Result<Option<Vec<u8>>> {
        let path = &normalize(path);
        let stripped = path.strip_prefix(&self.root).unwrap_or(path);
        // 多路查找：主 root 优先，其后 extra_roots（--input-path，对齐
        // ntex-dvi CLI 语义）。latex.ltx 载入闭包跨目录时必需（如源在
        // fixtures/latex2e/ 而 expl3-code.tex 在 fixtures/l3kernel/）。
        let mut roots = vec![self.root.clone()];
        roots.extend(self.extra_roots.iter().cloned());
        for root in &roots {
            let p = std::path::Path::new(root).join(stripped);
            match std::fs::read(&p) {
                Ok(bytes) => return Ok(Some(bytes)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            }
        }
        self.mem.read(path)
    }
    fn write(&mut self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.mem.write(&normalize(path), bytes)
    }
    fn append(&mut self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.mem.append(&normalize(path), bytes)
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let shim = args.iter().any(|a| a == "--shim");
    let initex = args.iter().any(|a| a == "--initex");
    // 主输入文件 = 首个非旗标且非 `--input-path 值` 的参数（值跟在旗标后，
    // 不跳过会把查找目录误当主文件读 → "Is a directory"）。
    let mut args_skip_next = false;
    let path = args
        .iter()
        .find(|a| {
            if args_skip_next {
                args_skip_next = false;
                return false;
            }
            if a.as_str() == "--input-path" {
                args_skip_next = true;
                return false;
            }
            !a.starts_with('-')
        })
        .map(String::as_str)
        .unwrap_or("/tmp/latexsurvey/tex/latex/base/latex.ltx");
    // --input-path <dir>（可多次）：额外文件查找目录，供载入闭包跨目录取件
    //（如 fixtures/latex2e 的 latex.ltx 要 input fixtures/l3kernel 的
    // expl3-code.tex）。对齐 ntex-dvi CLI 语义。
    let mut extra_roots: Vec<String> = Vec::new();
    let mut ai = args.iter().peekable();
    while let Some(a) = ai.next() {
        if a == "--input-path" {
            if let Some(dir) = ai.peek() {
                extra_roots.push((*dir).clone());
                ai.next();
            }
        }
    }
    // 未显式给 --input-path 时默认带上仓库 fixtures 的 l3kernel+latex2e 兄弟
    // 目录（源在 fixtures 内时）——跑 `fixtures/latex2e/latex.ltx` 即开箱即用。
    if extra_roots.is_empty() {
        let src_root = std::path::Path::new(path)
            .parent()
            .unwrap_or(std::path::Path::new("."));
        for probe in ["../l3kernel", ".", "../probes"] {
            if let Ok(d) = src_root.join(probe).canonicalize() {
                if d != src_root.canonicalize().unwrap_or(d.clone()) {
                    extra_roots.push(d.to_string_lossy().into_owned());
                }
            }
        }
    }
    let root = std::path::Path::new(path)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or(".")
        .to_owned();

    let src = std::fs::read(path).with_context(|| format!("读取 {path} 失败"))?;
    let mut input: Vec<u8> = Vec::new();
    if shim {
        input.extend_from_slice(INITEX_CATCODE_SHIM.as_bytes());
    }
    input.extend_from_slice(&src);

    let mut ts = ntex_layout::Typesetter::with_tfm();
    if initex {
        ts = ts.initex();
    }
    ts.set_vfs(Box::new(SurveyVfs {
        root,
        extra_roots,
        mem: MemVfs::new(),
    }));
    let res = ts.typeset_bytes(input);
    let transcript = ts.take_transcript();

    match res {
        Ok(_) => {
            println!("== pass1 OK, dumped={} ==", ts.dumped());
        }
        Err(e) => {
            println!("== pass1 ERROR: {e}");
            println!("== section: {}", ts.current_section());
            println!("== dumped: {}", ts.dumped());
        }
    }
    std::fs::write(format!("{path}.transcript"), &transcript)
        .with_context(|| "写 transcript 失败")?;
    println!(
        "== transcript {} bytes (-> {path}.transcript)",
        transcript.len()
    );
    let lines: Vec<&str> = transcript.lines().collect();
    if lines.len() > 70 {
        for l in &lines[..30] {
            println!("| {l}");
        }
        println!("| ... ({} 行省略) ...", lines.len() - 60);
    }
    let tail = lines.len().saturating_sub(40);
    for l in &lines[tail..] {
        println!("| {l}");
    }

    if ts.dumped() {
        let mut buf = Vec::new();
        ntex_format::save(&mut buf, &ts.export_state())?;
        println!("== fmt saved: {} bytes", buf.len());
        let mut ts2 = ntex_layout::Typesetter::with_tfm();
        let state = ntex_format::load(&mut &buf[..])?;
        ts2.import_state(state);
        let r2 = ts2.typeset_bytes("\\message{format loaded}\\relax");
        println!(
            "== pass2 smoke: err={:?} transcript={:?}",
            r2.as_ref().err().map(|e| e.to_string()),
            ts2.take_transcript()
        );
    }
    Ok(())
}
