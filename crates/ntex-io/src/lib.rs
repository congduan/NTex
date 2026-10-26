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
    /// 递归建目录（已存在即成功）。
    ///
    /// **默认 no-op**：平坦后端（[`MemVfs`] 的键是整串路径，没有目录概念）无需实现。
    /// 只有真正落盘的后端（[`LocalVfs`]）才需要——调用方因此可以「先 ensure 目录再
    /// [`Vfs::write`]」，不必知道后端是不是文件系统。
    ///
    /// 用途：资产物化（`ntex-pkg vendor` 把依赖闭包铺进 TDS 子树）与将来的
    /// T2 下载缓存落盘。RFC-3 语义不变：这是**宿主侧**的 IO 能力，排版内核
    /// 仍然只经由本 trait 读写，不直接碰 `std::fs`。
    fn create_dir_all(&mut self, _path: &str) -> io::Result<()> {
        Ok(())
    }
    /// 文件修改时间（Unix epoch 秒）；不存在或后端无时间概念返回 `None`。
    ///
    /// 消费方：`\pdffilemoddate`（filemod 宏包 / standalone.cls 的 `\input@path`
    /// 重编译比对链）。`None` → 该原语空展开（GT 同语义：pdfTeX 对不存在文件
    /// 返回空串）。**默认 `None`**：[`MemVfs`] 无时间概念（内容可随时改写），
    /// WASM 侧 `\pdffilemoddate` 恒空展开——确定性换真实时间，偏差登记
    /// KNOWN-SIMPLIFICATIONS。
    fn file_mtime(&mut self, _path: &str) -> io::Result<Option<u64>> {
        Ok(None)
    }
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

    fn create_dir_all(&mut self, path: &str) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn file_mtime(&mut self, path: &str) -> io::Result<Option<u64>> {
        match std::fs::metadata(path) {
            Ok(md) => Ok(md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
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

/// 搜索路径层：[`Vfs`] 读侧的 TEXINPUTS 语义最小子集（格式预载战役 G1）。
///
/// [`Vfs::read`] 依次尝试：
/// 1. 原样（相对 cwd / MemVfs 裸键）——保持既有语义，命中即返；
/// 2. 各搜索路径前缀拼原名（见下）——先加先试，第一个命中的返回。
///
/// 全部落空返回 `Ok(None)`：`\input`/`\openin` 的「文件不存在」语义不变。
/// 写侧（write/append）不经搜索路径，原样透传（TeX 的 TEXINPUTS 只管输入）。
///
/// 前缀拼接按本 crate 约定用原始字符串（不做平台 Path 解析，WASM 友好）：
/// 前缀末尾无 `/` 则补一个再接原名；以 `/` 开头的原名视为绝对路径，
/// 跳过搜索（kpathsea 同口径）。
///
/// **范围纪律**：只做单层前缀拼接。kpathsea 的其余语义——`TEXINPUTS`
/// 环境变量冒号多路径展开、递归树扫描、`!!` 哈希缓存——均未实现，待办。
#[derive(Debug)]
pub struct SearchPathVfs {
    inner: Box<dyn Vfs>,
    paths: Vec<String>,
}

impl SearchPathVfs {
    /// 包住既有后端（本地 [`LocalVfs`] 或 [`MemVfs`]）。
    pub fn new(inner: Box<dyn Vfs>) -> Self {
        Self {
            inner,
            paths: Vec::new(),
        }
    }

    /// 追加一条搜索路径（后加者优先级低）。
    pub fn push_path(&mut self, path: impl Into<String>) -> &mut Self {
        self.paths.push(path.into());
        self
    }

    /// 已登记的搜索路径（诊断 / 测试断言用）。
    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    /// 解析：返回命中路径与内容；`None` = 全部落空。
    fn resolve(&mut self, path: &str) -> io::Result<Option<(String, Vec<u8>)>> {
        if let Some(bytes) = self.inner.read(path)? {
            return Ok(Some((path.to_owned(), bytes)));
        }
        if path.starts_with('/') {
            return Ok(None);
        }
        for prefix in &self.paths {
            let joined = if prefix.is_empty() || prefix.ends_with('/') {
                format!("{prefix}{path}")
            } else {
                format!("{prefix}/{path}")
            };
            if let Some(bytes) = self.inner.read(&joined)? {
                return Ok(Some((joined, bytes)));
            }
        }
        Ok(None)
    }
}

/// `kpsewhich` 兜底层：读侧先问 inner，全落空后尝试调用 PATH 上的
/// `kpsewhich <path>` 定位真实 TeX 树文件。
///
/// 这是发行版搜索链的最后一层，精简环境没有 `kpsewhich` 时静默落空，保持
/// [`Vfs::read`] 的 `Ok(None)` 契约。写侧仍原样透传，不参与 kpathsea。
#[derive(Debug)]
pub struct KpsewhichVfs {
    inner: Box<dyn Vfs>,
}

impl KpsewhichVfs {
    /// 包住既有后端。
    pub fn new(inner: Box<dyn Vfs>) -> Self {
        Self { inner }
    }
}

impl KpsewhichVfs {
    /// kpsewhich 定位：返回 PATH 上 `kpsewhich` 找到的真实文件路径。
    /// 无 kpsewhich / 未命中 → `None`（read 与 file_mtime 共用）。
    fn locate(&self, path: &str) -> Option<String> {
        let out = std::process::Command::new("kpsewhich")
            .arg(path)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let p = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if p.is_empty() {
            None
        } else {
            Some(p)
        }
    }
}

impl Vfs for KpsewhichVfs {
    fn read(&mut self, path: &str) -> io::Result<Option<Vec<u8>>> {
        if let Some(bytes) = self.inner.read(path)? {
            return Ok(Some(bytes));
        }
        let Some(p) = self.locate(path) else {
            return Ok(None);
        };
        match std::fs::read(&p) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// mtime 先问 inner，落空后走同一 kpsewhich 定位链（`\pdffilemoddate`
    /// 对 TeX 树内文件也要能取到真实时间）。
    fn file_mtime(&mut self, path: &str) -> io::Result<Option<u64>> {
        if let Some(t) = self.inner.file_mtime(path)? {
            return Ok(Some(t));
        }
        let Some(p) = self.locate(path) else {
            return Ok(None);
        };
        Ok(std::fs::metadata(&p)
            .ok()
            .and_then(|md| md.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs()))
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> io::Result<()> {
        self.inner.write(path, bytes)
    }

    fn append(&mut self, path: &str, bytes: &[u8]) -> io::Result<()> {
        self.inner.append(path, bytes)
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl Vfs for SearchPathVfs {
    fn read(&mut self, path: &str) -> io::Result<Option<Vec<u8>>> {
        Ok(self.resolve(path)?.map(|(_, bytes)| bytes))
    }

    /// mtime 与 read 同一解析序：先原名（cwd），再各搜索路径前缀，
    /// 第一个命中者的 mtime。全落空 → `None`。
    fn file_mtime(&mut self, path: &str) -> io::Result<Option<u64>> {
        if let Some(t) = self.inner.file_mtime(path)? {
            return Ok(Some(t));
        }
        if path.starts_with('/') {
            return Ok(None);
        }
        for prefix in &self.paths {
            let joined = if prefix.is_empty() || prefix.ends_with('/') {
                format!("{prefix}{path}")
            } else {
                format!("{prefix}/{path}")
            };
            if let Some(t) = self.inner.file_mtime(&joined)? {
                return Ok(Some(t));
            }
        }
        Ok(None)
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> io::Result<()> {
        self.inner.write(path, bytes)
    }

    fn append(&mut self, path: &str, bytes: &[u8]) -> io::Result<()> {
        self.inner.append(path, bytes)
    }

    fn create_dir_all(&mut self, path: &str) -> io::Result<()> {
        self.inner.create_dir_all(path)
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

    #[test]
    fn file_mtime_forwards_through_search_path_and_kpsewhich_chain() {
        // `\pdffilemoddate` 消费链（2026-10-26）：CLI 的 VFS 是
        // KpsewhichVfs(SearchPathVfs(LocalVfs))——两层包装都必须转发
        // file_mtime，否则 CLI 端恒空展开（首轮 e2e 实测 MODDATE=[] 假绿）。
        let dir = std::env::temp_dir().join(format!("ntex-io-mtimetest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("probe.tex"), b"x").unwrap();
        let epoch_before = std::fs::metadata(dir.join("probe.tex"))
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // 1) LocalVfs 直读：存在 → Some；不存在 → None（NotFound 不报错）。
        let mut local = LocalVfs;
        let t = local
            .file_mtime(dir.join("probe.tex").to_str().unwrap())
            .unwrap();
        assert!(t.is_some());
        assert!(t.unwrap() >= epoch_before);
        assert_eq!(local.file_mtime("no-such-ntex-io-file.tex").unwrap(), None);

        // 2) SearchPathVfs 按前缀解析取 mtime（原名落空 → 前缀命中）。
        let mut sp = SearchPathVfs::new(Box::new(LocalVfs));
        sp.push_path(dir.to_string_lossy().into_owned());
        let t2 = sp.file_mtime("probe.tex").unwrap();
        assert_eq!(
            t2, t,
            "SearchPathVfs 前缀命中应返回与 LocalVfs 直读相同的 mtime"
        );

        // 3) KpsewhichVfs 转发 inner（有 inner 命中时不触 kpsewhich 子进程）。
        let mut kp = KpsewhichVfs::new(Box::new(sp));
        assert_eq!(kp.file_mtime("probe.tex").unwrap(), t);
        assert_eq!(kp.file_mtime("no-such-ntex-io-file.tex").unwrap(), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_vfs_create_dir_all_enables_nested_write() {
        // 目录不存在时 `std::fs::write` 会失败——`create_dir_all` 是「先铺目录再写」
        // 的前置能力（ntex-pkg vendor 物化 TDS 子树、T2 下载缓存落盘都依赖它）。
        let mut v = LocalVfs;
        let base = std::env::temp_dir().join(format!("ntex-io-dirtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let nested = base.join("tex/latex/base");
        let nested_str = nested.to_str().expect("临时路径应为合法 UTF-8");

        // 先证明不建目录确实写不进去（否则本测试就是空转）。
        let direct = nested.join("article.cls");
        assert!(
            v.write(direct.to_str().unwrap(), b"x").is_err(),
            "父目录不存在时写入应当失败"
        );

        v.create_dir_all(nested_str).expect("建目录应成功");
        v.write(direct.to_str().unwrap(), b"% article")
            .expect("建目录后写入应成功");
        assert_eq!(
            v.read(direct.to_str().unwrap()).unwrap(),
            Some(b"% article".to_vec())
        );

        // 幂等：重复调用不报错（调用方无需先判断存在性）。
        v.create_dir_all(nested_str).expect("重复建目录应成功");
        let _ = std::fs::remove_dir_all(&base);
    }

    // ---------- G1：搜索路径（\input plain → \input hyphen 的缺口） ----------

    #[test]
    fn search_path_resolves_when_bare_misses() {
        let mut mem = MemVfs::new();
        // plain.tex 旁边的 hyphen.tex 不在 cwd：经搜索路径前缀命中。
        mem.insert("tex/plain/base/hyphen.tex", b"\\patterns{...}");
        let mut v = SearchPathVfs::new(Box::new(mem));
        v.push_path("tex/plain/base");
        assert_eq!(
            v.read("hyphen.tex").unwrap(),
            Some(b"\\patterns{...}".to_vec()),
            "搜索路径前缀应参与解析"
        );
        assert_eq!(v.read("absent.tex").unwrap(), None, "全落空仍须 Ok(None)");
        assert_eq!(v.paths(), ["tex/plain/base"]);
    }

    #[test]
    fn bare_path_wins_and_writes_bypass_search() {
        let mut mem = MemVfs::new();
        mem.insert("plain.tex", b"% cwd copy");
        mem.insert("dir/plain.tex", b"% search-path copy");
        let mut v = SearchPathVfs::new(Box::new(mem));
        v.push_path("dir");
        assert_eq!(
            v.read("plain.tex").unwrap(),
            Some(b"% cwd copy".to_vec()),
            "原样命中优先（既有语义不变，cwd 相当于 TEXINPUTS 里的 `.`）"
        );
        // 写侧不走搜索路径：写 "out.tex" 不得落到 dir/out.tex。
        v.write("out.tex", b"x").unwrap();
        v.append("out.tex", b"y").unwrap();
        assert_eq!(v.read("out.tex").unwrap(), Some(b"xy".to_vec()));
        let inner = v.inner.as_any_mut().downcast_ref::<MemVfs>().unwrap();
        assert!(inner.get("out.tex").is_some(), "应写在原样路径");
        assert!(inner.get("dir/out.tex").is_none(), "写侧不经搜索路径");
    }

    #[test]
    fn kpsewhich_layer_preserves_inner_hit() {
        let mut mem = MemVfs::new();
        mem.insert("plain.tex", b"% inner");
        let mut v = KpsewhichVfs::new(Box::new(mem));
        assert_eq!(v.read("plain.tex").unwrap(), Some(b"% inner".to_vec()));
    }
}
