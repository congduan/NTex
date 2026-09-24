//! 内嵌 plain 格式（格式预载战役 G2(a)）：plain.tex + hyphen.tex 编译进二进制。
//!
//! **裁决**（plain-format-survey §3.2 候选 (a)「启动自动 `\input plain`」与 (c)
//! 「内嵌字符串」合流）：资源逐字节内嵌，不读运行期文件——
//! ① 分发问题：G1 的 [`ntex_io::SearchPathVfs`] 只解决「找得到」，目标机没有
//!    TinyTeX 树依旧跑不了（survey §3.2 共同前置明言）；
//! ② wasm 先例：ntex-wasm 已内嵌 6 个 CM TFM（`wasm_fonts.rs`），单二进制
//!    离线可用是既定方向；plain.tex+hyphen.tex 合计 ~72KB（wasm 可承受）；
//! ③ 双通路同源：启动预载（字符串直接 feed）与源内 `\input plain`（VFS 读侧）
//!    吃同一份常量，不会再出现「手写自举块 vs plain.tex」的两份 plain 常数
//!    （survey §3.1#4 分裂脑的病根）。
//!
//! **否决论证**：否决「读文件 + 搜索路径」——见①，且 `\input hyphen` 的
//! 分发缺口（survey §2.1 唯一致命点）正是不内嵌才有的；否决「跳过 patterns」
//! ——hyphen.tex 仅 27,860B，内嵌成本可忽略，`\patterns`/`\hyphenation` 原语
//! 已在（ntex-core builtins.rs），跳过反而制造「plain 预载了但断词表缺失」
//! 的新静默偏差。
//!
//! **许可**：plain.tex / hyphen.tex 均为 Knuth 作品、逐字节 verbatim 拷贝
//! （plain.tex 取自 TinyTeX `texmf-dist/tex/plain/base/plain.tex`，与
//! `fixtures/corpus/plain/plain.tex` 逐字节一致；hyphen.tex 头部明言
//! "NOT TO BE CHANGED IN ANY WAY"）。两文件均允许原样再分发，不满足
//! 「不得更名」的修改版须另起文件名。

/// plain 格式本体（1241 行；plain.tex 不含 `\dump`，运行期 `\input` 安全）。
pub const PLAIN_TEX: &str = include_str!("../../resources/plain.tex");

/// 默认断字模式表 + 例外词表（plain.tex:1222 `\input hyphen`）。
pub const HYPHEN_TEX: &str = include_str!("../../resources/hyphen.tex");

/// booktabs + multirow 兼容层（最小语义；资源头注释有逐条语义与动机）。
pub const BOOKTABS_COMPAT_TEX: &str = include_str!("../../resources/booktabs-compat.tex");

/// 内嵌表：`\input` 搜索的最后一层兜底（键 = 带 `.tex` 后缀的规范名；
/// 引擎 `\input plain` 先试裸名再补 `.tex`（`expand/io.rs` exec_input），
/// 故只需登记 `.tex` 形）。
const BUNDLED: &[(&str, &str)] = &[
    ("plain.tex", PLAIN_TEX),
    ("hyphen.tex", HYPHEN_TEX),
    ("booktabs-compat.tex", BOOKTABS_COMPAT_TEX),
];

/// 内嵌格式文件 VFS 兜底层：读侧先问 inner（本地文件/宿主 VFS 命中优先，
/// 与 G1 搜索路径「原样优先」同口径），全落空再查内嵌表。
///
/// 写侧（`\write`/`\openout`）与内嵌无关，透传 inner。
#[derive(Debug)]
pub struct EmbeddedFormatVfs {
    inner: Box<dyn ntex_io::Vfs>,
}

impl EmbeddedFormatVfs {
    /// 包住既有后端（本地 [`ntex_io::LocalVfs`]、搜索路径层或宿主注入层）。
    pub fn new(inner: Box<dyn ntex_io::Vfs>) -> Self {
        Self { inner }
    }

    /// 内嵌表命中？（诊断/测试用）。
    pub fn lookup(name: &str) -> Option<&'static str> {
        BUNDLED.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
    }
}

impl ntex_io::Vfs for EmbeddedFormatVfs {
    fn read(&mut self, path: &str) -> std::io::Result<Option<Vec<u8>>> {
        if let Some(bytes) = self.inner.read(path)? {
            return Ok(Some(bytes));
        }
        Ok(Self::lookup(path).map(|s| s.as_bytes().to_vec()))
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.inner.write(path, bytes)
    }

    fn append(&mut self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.inner.append(path, bytes)
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
