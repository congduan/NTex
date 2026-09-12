//! OTF/CFF 字体（OpenType）嵌入支撑（M9 中文 PDF 导出）。
//!
//! 中文 Fandol 等字体只有 OTF/CFF 形态、无 Type1 PFB——PDF 侧按
//! Type0/CIDFontType0 + `/FontFile3 /OpenType` 嵌入整个 sfnt 文件
//! （PDF 1.6+ 合法；PDF 头仍写 1.4 时多数查看器同样接受，见 [`crate::pdf`]）。
//! 本层与 [`crate::type1`] 对称：只负责**定位与校验字体字节**，不解析
//! 字形数据；CID 映射与对象组装在 [`crate::pdf`]。
//!
//! 字体字节有两个来源，**注册表优先**（与 type1 同构）：
//! 1. [`register_otf`] 的进程级注入（wasm/浏览器唯一来源；native 下覆盖环境
//!    字体，用于测试与打包分发）——见 `ntex-wasm` 的 `set_otf_font`；
//! 2. 宿主文件系统查找链（native 专属，复用 [`ntex_font::find_otf`] 的
//!    环境变量/用户字体目录/texlive/kpsewhich 链）。wasm32 无文件系统，
//!    未注入即 [`load_otf`] 直接返回 `Err`。

use std::collections::HashMap;
use std::io::{self, ErrorKind};
use std::sync::{LazyLock, Mutex};

/// 进程级 OTF 字节注册表：[`register_otf`] 写入，[`load_otf`] 优先命中
/// （早于宿主文件系统查找）。
static REGISTRY: LazyLock<Mutex<HashMap<String, Vec<u8>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// sfnt 容器魔数：OTTO = CFF 轮廓（Fandol/LM 均为此形态）；
/// 0x00010000 与 `true` = TrueType 轮廓。注册时用作最低限度格式校验——
/// 非 sfnt 字节嵌进 `/FontFile3` 只会让查看器静默渲染失败，不如当场拒绝。
const SFNT_MAGICS: [&[u8]; 3] = [b"OTTO", &[0x00, 0x01, 0x00, 0x00], b"true"];

/// 注册 OTF/TTF 字体字节（`tex_name` 为 TeX 字体名，如 `FandolSong-Regular`
/// ——须与 DVI `fnt_def` 的外部名一致）。
///
/// 应用场景：wasm/浏览器前端 fetch 字体文件后注入，之后 [`load_otf`] 即按
/// 名字命中；native 端用于免 TeX Live 环境的打包分发与测试。同名重复注册
/// 为覆盖（含覆盖宿主查找结果）。
///
/// 字节魔数不符（非 sfnt，如误传 PFB/PFA）返回 `false`，调用方据此报错。
/// 注意：当前嵌入路径仅支持 CFF 轮廓（OTTO）——TrueType 轮廓（glyf）注册
/// 可成功，但写出端按不嵌入降级（挂账，见 `pdf::build_document`）。
pub fn register_otf(tex_name: &str, bytes: &[u8]) -> bool {
    if !SFNT_MAGICS.iter().any(|m| bytes.starts_with(m)) {
        return false;
    }
    match REGISTRY.lock() {
        Ok(mut m) => {
            m.insert(tex_name.to_owned(), bytes.to_vec());
            true
        }
        // 锁毒化：持锁线程已 panic，注册按失败处理（不传播错误，引擎契约）。
        Err(_) => false,
    }
}

/// 取已注册的 OTF 字节（不触碰宿主文件系统）。
pub fn registered_otf(tex_name: &str) -> Option<Vec<u8>> {
    REGISTRY.lock().ok()?.get(tex_name).cloned()
}

/// 读宿主文件系统里的 OTF（复用 `ntex-font` 的查找链：环境变量 → 用户/系统
/// 字体目录 → texlive → kpsewhich；支持 .otf/.ttf 双扩展名）。
#[cfg(not(target_arch = "wasm32"))]
fn read_host_otf(name: &str) -> io::Result<Vec<u8>> {
    let path = ntex_font::find_otf(name).ok_or_else(|| {
        io::Error::new(ErrorKind::NotFound, format!("找不到 OTF 字体：{name}"))
    })?;
    std::fs::read(&path)
}

/// wasm32：无文件系统可用，字体字节只能经 [`register_otf`] 注入。
#[cfg(target_arch = "wasm32")]
fn read_host_otf(name: &str) -> io::Result<Vec<u8>> {
    Err(io::Error::new(
        ErrorKind::NotFound,
        format!("找不到 OTF 字体：{name}（wasm 无文件系统，须先 register_otf 注入字节）"),
    ))
}

/// 读取 OTF 字体字节（PDF `/FontFile3` 流内容，原样嵌入）。
///
/// 查找顺序：注册表（[`register_otf`]）→ 宿主文件系统链（native 专属）。
/// 两者都未命中返回 `Err`——`pdf::build_document` 对 `Err` 是**降级**处理
/// （Type0 字典保留但不嵌字体程序），调用方若在意可用性应自查
/// [`registered_otf`] 而非依赖此处报错。
pub fn load_otf(name: &str) -> io::Result<Vec<u8>> {
    if let Some(bytes) = registered_otf(name) {
        return Ok(bytes);
    }
    read_host_otf(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最小 sfnt 字节（只到魔数层面，校验不解析表目录）。
    fn fake_otf() -> Vec<u8> {
        let mut b = b"OTTO".to_vec();
        b.extend_from_slice(&[0, 1, 0, 0]); // numTables 等占位
        b
    }

    #[test]
    fn registered_bytes_take_priority_and_are_used_verbatim() {
        // 名字取唯一值：宿主查找链必然找不到它，「命中」只可能来自注册表。
        let name = "zz-otf-registry-probe";
        #[cfg(not(target_arch = "wasm32"))]
        assert!(
            ntex_font::find_otf(name).is_none(),
            "前置假设：{name} 不在宿主查找链上"
        );
        let bytes = fake_otf();
        assert!(register_otf(name, &bytes), "OTTO 魔数应注册成功");
        assert_eq!(registered_otf(name).as_deref(), Some(&bytes[..]));
        assert_eq!(
            load_otf(name).expect("注册后 load_otf 应命中注册表"),
            bytes,
            "字节须原样保留（/FontFile3 逐字节嵌入）"
        );
    }

    #[test]
    fn register_rejects_bytes_without_sfnt_magic() {
        // 误传 PFB/PFA/空 一律拒绝，避免把无效字节写进 /FontFile3
        // （查看器会静默渲染不出内容，比当场报错难查得多）。
        assert!(!register_otf("zz-otf-probe-pfb", &[0x80, 0x01, 0, 0]));
        assert!(!register_otf("zz-otf-probe-pfa", b"%!PS-AdobeFont-1.0"));
        assert!(!register_otf("zz-otf-probe-empty", &[]));
        assert!(registered_otf("zz-otf-probe-pfb").is_none(), "拒绝的不入库");
        assert!(
            load_otf("zz-otf-probe-pfb").is_err(),
            "未注册 + 宿主无此字体 → Err（调用方据此提示）"
        );
    }

    #[test]
    fn truetype_flavor_magic_is_accepted() {
        // TrueType 轮廓（0x00010000）：注册合法（未来 glyf 嵌入挂账），
        // 写出端目前按不嵌入降级。
        let name = "zz-ttf-magic-probe";
        assert!(register_otf(name, &[0x00, 0x01, 0x00, 0x00, 0, 0]));
        assert_eq!(registered_otf(name).is_some(), true);
    }
}
