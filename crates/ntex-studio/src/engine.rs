//! 引擎接线：发行资产（LaTeX `.fmt` + TeX 文件树 + TFM）、TeX 搜索链、字体注入。
//!
//! 本模块是 native `ntex-studio` 对 `ntex-tauri`（wasm 前端）C 档能力的**对齐
//! 层**——同一个工作台在两种宿主下的差异只剩"资产怎么送进来"：
//!
//! | 能力 | Tauri（wasm 前端） | 本模块（native） |
//! |---|---|---|
//! | LaTeX 模式识别 | `main.js::applyMode` | [`ntex_layout::typeset::looks_like_latex`] |
//! | `.fmt` 快照 | `set_bundle`（IPC 资产包） | 直接读 `<assets>/fmt/latex.fmt` |
//! | `\input`/`\usepackage` 的 TeX 文件 | 包内文件灌 `MemVfs` | [`SearchPathVfs`] + 发行 `tex-minimal` 树 + kpsewhich |
//! | 额外 TFM 度量 | 包内 TFM 灌 `TfmSource` | [`DirTfmSource`] 读 `<assets>/tfm`（缺则回落文件系统） |
//! | 字体轮廓（LM OTF） | `ui/fonts/` fetch → `set_glyph_font` | [`register_font_dir`] 读同一目录 |
//! | 中文字体（Fandol） | `set_otf_font`（度量+轮廓） | [`DirTfmSource::otf_bytes`] + [`register_font_dir`] |
//! | CJK 回落 | `set_fallback_font` | [`IncrementalTypesetter::set_fallback_font`] |
//! | UTF-8 直写源码 | `set_utf8_input(true)` | [`IncrementalTypesetter::set_utf8_input`] |
//!
//! 字体目录的唯一副本在 `crates/ntex-tauri/ui/fonts/`（Tauri 前端 fetch 的就是
//! 它）；native 侧直接读盘，不复制第二份——避免"两处字体集漂移"。

use std::path::{Path, PathBuf};

use ntex_layout::typeset::{IncrementalTypesetter, TfmSource};

/// 发行资产根（含 `fmt/latex.fmt` 与 `tex-minimal/tex` 的那层 `assets/`）。
///
/// 搜索顺序与 `ntex-tauri::assets_root`、`ntex-dvi` 的发行链同精神：先
/// `NTEX_ASSETS` 环境变量、再可执行文件旁、再入仓开发树、最后 cwd。
pub fn assets_root() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("NTEX_ASSETS") {
        candidates.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("assets"));
            candidates.push(dir.join("../../assets"));
        }
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets"));
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("assets"));
    }
    candidates
        .into_iter()
        .find(|root| root.join("fmt/latex.fmt").is_file() && root.join("tex-minimal/tex").is_dir())
}

/// 发行字体目录（LM OTF + Fandol 中文 OTF；与 Tauri 前端 fetch 的目录同一份）。
pub fn font_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ntex-tauri/ui/fonts");
    dir.is_dir().then_some(dir)
}

/// 启动期一次性安装的发行资产与字体（`Studio::new` 调用一次）。
pub struct Setup {
    /// 发行资产根（`None` = 找不到：LaTeX 不可用，回落 plain 子集）。
    pub root: Option<PathBuf>,
    /// LaTeX 格式快照（`latex.fmt` 解码结果）。`None` = 无 fmt（LaTeX 不可用）。
    pub fmt: Option<ntex_core::expand::FmtState>,
    /// 已注入字形注册表的 TeX 字体名（诊断/状态栏）。
    pub fonts: Vec<String>,
    /// 实际生效的 CJK 回落字体名（字体目录缺 Fandol 时为 `None`）。
    pub fallback_font: Option<String>,
    /// 解析过程中的可读提示（资产缺失/字体缺失等），仅用于状态栏说明。
    pub note: String,
}

impl Setup {
    /// 安装字体（进程级注册表）并解码 `latex.fmt`。**必须在 UI 线程调用**：
    /// TFM 字节源注册表是 `thread_local`，编译在哪个线程跑就要在哪个线程注册。
    pub fn install() -> Self {
        let root = assets_root();
        let mut fonts = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        // 字体轮廓：与 Tauri 前端 `GLYPH_FONTS` 同一张名单（Rust 侧等价生成）。
        let mut fallback_font = None;
        match font_dir() {
            Some(dir) => {
                fonts = ntex_backend::glyphs::register_font_dir(&dir);
                if fonts.iter().any(|n| n == "FandolSong-Regular") {
                    fallback_font = Some("FandolSong-Regular".to_owned());
                } else {
                    notes.push("字体目录缺 FandolSong-Regular（中文源码将缺字形）".to_owned());
                }
                if fonts.is_empty() {
                    notes.push("字体目录无可认领字体（真字形回落方框）".to_owned());
                }
            }
            None => {
                notes.push("找不到发行字体目录（真字形回落环境查找链/方框）".to_owned());
            }
        }
        // TFM 字节源：优先发行 `<assets>/tfm`（与 Tauri 资产包里的 TFM 同源），
        // 未命中返回 None → `TfmLoader` 继续走 native 文件系统/kpsewhich。
        if let Some(root) = &root {
            ntex_layout::set_tfm_source(Box::new(DirTfmSource {
                tfm_dirs: vec![root.join("tfm")],
                otf_dirs: font_dir().into_iter().collect(),
            }));
        }
        let fmt = match &root {
            Some(root) => match std::fs::read(root.join("fmt/latex.fmt")) {
                Ok(bytes) => match ntex_format::load(&mut &bytes[..]) {
                    Ok(state) => Some(state),
                    Err(e) => {
                        notes.push(format!("解析 latex.fmt 失败：{e}"));
                        None
                    }
                },
                Err(e) => {
                    notes.push(format!("读取 latex.fmt 失败：{e}"));
                    None
                }
            },
            None => {
                notes.push("找不到发行资产目录（需含 fmt/latex.fmt 与 tex-minimal/tex；可用 NTEX_ASSETS 指定）".to_owned());
                None
            }
        };
        let note = notes.join("；");
        Self {
            root,
            fmt,
            fonts,
            fallback_font,
            note,
        }
    }

    /// LaTeX 口径是否可用（资产齐备）；不可用时一律 plain 子集。
    pub fn latex_ready(&self) -> bool {
        self.fmt.is_some()
    }

    /// 按模式新建增量排版器（**切换模式必须重建**：plain 预载与 `.fmt` 导入
    /// 是互斥的两种初始状态，叠加会污染；与 native `ntex-dvi --fmt` 同口径）。
    ///
    /// `extra_input_dirs`：当前打开文件所在目录——`\input` 相对路径可见
    /// （编辑器里打开的文档与它的同目录子文件）。
    pub fn typesetter(&self, latex: bool, extra_input_dirs: &[PathBuf]) -> IncrementalTypesetter {
        let mut ts = IncrementalTypesetter::with_tfm_paginated();
        // UTF-8 直写源码（M9 中文刀 3；对齐 Tauri 前端默认开）。
        ts.set_utf8_input(true);
        ts.set_fallback_font(self.fallback_font.clone());
        ts.set_vfs(self.tex_vfs(extra_input_dirs));
        if latex {
            if let Some(state) = self.fmt.clone() {
                ts.import_state(state);
            }
            // 内嵌 plain 文件仅作 `\input` 兜底（不预载——fmt 已含全量状态）。
            ts.use_embedded_format();
        } else {
            // plain 口径：预载内嵌 plain.tex（等价源首行 `\input plain`），
            // 与 wasm 侧 `compile_pipeline_assets` 的 `set_preload_plain(true)`
            // 一致——缺它则 plain 宏（`\hsize` 之外的 plain 定义）全缺。
            ts.set_preload_plain(true);
        }
        ts
    }

    /// TeX 文件搜索链：发行 `tex-minimal` 树（含 `latex/base`、各宏包目录）→
    /// 打开文件所在目录 → kpsewhich 兜底。
    fn tex_vfs(&self, extra_input_dirs: &[PathBuf]) -> Box<dyn ntex_io::Vfs> {
        let mut paths: Vec<PathBuf> = Vec::new();
        paths.extend(extra_input_dirs.iter().cloned());
        if let Some(root) = &self.root {
            let tex = root.join("tex-minimal/tex");
            add_tex_tree_paths(&mut paths, tex);
        }
        let mut vfs = ntex_io::SearchPathVfs::new(Box::new(ntex_io::LocalVfs));
        for p in paths {
            vfs.push_path(p.to_string_lossy().into_owned());
        }
        Box::new(ntex_io::KpsewhichVfs::new(Box::new(vfs)))
    }
}

/// 发行 TeX 树展开为搜索目录（与 `ntex-dvi` 的 `add_tex_tree_paths` 同口径）。
fn add_tex_tree_paths(out: &mut Vec<PathBuf>, root: PathBuf) {
    out.push(root.clone());
    out.push(root.join("latex/base"));
    push_children(out, &root.join("latex"));
    push_children(out, &root.join("generic"));
}

fn push_children(out: &mut Vec<PathBuf>, dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.push(path);
        }
    }
}

/// 目录式字体源：TFM 度量走 `<assets>/tfm`，OpenType 度量走发行字体目录。
///
/// 命中即返回字节；**未命中返回 `None`**——`TfmLoader` 随后回落 native 文件
/// 系统/kpsewhich（`load_metrics` 的分派链），故本源只补"发行树里多出来"的
/// 那份（LaTeX 字号族的 cmbx12/cmr17 等），不改变既有环境查找语义。
#[derive(Debug)]
struct DirTfmSource {
    /// TFM 目录（按名找 `<name>.tfm`）。
    tfm_dirs: Vec<PathBuf>,
    /// OpenType 目录（按名找 `<name>.otf`/`.ttf`/`.ttc`；中文字体走这条）。
    otf_dirs: Vec<PathBuf>,
}

impl TfmSource for DirTfmSource {
    fn tfm_bytes(&mut self, name: &str) -> Option<Vec<u8>> {
        let file = format!("{name}.tfm");
        self.tfm_dirs
            .iter()
            .find_map(|dir| std::fs::read(dir.join(&file)).ok())
    }

    /// OpenType 度量（对齐 Tauri 的 `set_otf_font`：无 TFM 的字体如
    /// `FandolSong-Regular` 靠这条进排版度量）。
    fn otf_bytes(&mut self, name: &str) -> Option<Vec<u8>> {
        for ext in ["otf", "ttf", "ttc"] {
            let file = format!("{name}.{ext}");
            if let Some(bytes) = self
                .otf_dirs
                .iter()
                .find_map(|dir| std::fs::read(dir.join(&file)).ok())
            {
                return Some(bytes);
            }
        }
        None
    }
}
