//! 字体加载器边界（M3-4 TFM）。
//!
//! ntex-core 是纯 token 级 VM，不解析字体文件。`\font` 原语扫描语法
//! （`\font<cs>[=]<名字>[at <dimen>|scaled <int>]`）后，把外部字体名交给
//! [`FontLoader`] 解析为字体槽号（FontId）；VM 将 cs 定义为
//! [`crate::eqtb::EqSlot::Font`]。真正加载 TFM、维护字体表的是上层
//! （ntex-layout 的 TFM 加载器）。
//!
//! `at` 为设计字号目标（sp）；`scaled` 为千分比（`scaled 1200` = 1.2×）。
//! 未安装加载器时 `\font` 报错（默认 [`NoFontLoader`]）。

use crate::error::{Error, Result};

/// 字体加载器：把外部字体名解析为字体槽号（FontId）。
///
/// 返回值直接用作 [`crate::eqtb::EqSlot::Font`] 的槽号；表由实现方维护，
/// 供排版器（ntex-layout）按 FontId 查询字符度量。
pub trait FontLoader: std::fmt::Debug {
    /// 加载字体。
    ///
    /// - `at`：`\font..at <dimen>` 的目标尺寸（sp），缩放因子 = at / 设计字号；
    /// - `scaled`：`\font..scaled <int>` 的千分比缩放（`scaled 1000` = 原尺寸）。
    ///   两者互斥（扫描层保证不同时给出）；全 None = 设计字号。
    fn load(&mut self, name: &str, at: Option<i64>, scaled: Option<i64>) -> Result<u32>;

    /// 查询字体字符度量（sp）：`(width, height, depth)`。
    ///
    /// ETRIP：`\iffontchar`（含该字符为真）与 `\fontcharwd`/`\fontcharht`/
    /// `\fontchardp`/`\fontcharic`（维度查询）用。字体未加载/无该字符 → None。
    fn char_metric(&mut self, _font: u32, _ch: u32) -> Option<(i64, i64, i64)> {
        None
    }
}

/// 默认加载器：未安装时 `\font` 报错（提示安装排版层加载器）。
#[derive(Debug)]
pub struct NoFontLoader;

impl FontLoader for NoFontLoader {
    fn load(&mut self, name: &str, _at: Option<i64>, _scaled: Option<i64>) -> Result<u32> {
        Err(Error::invalid_input(format!(
            "\\font 需要安装字体加载器（ntex-layout 的 TFM 加载器），无法加载：{name}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_loader_errors() {
        let mut l = NoFontLoader;
        assert!(l.load("cmr10", None, None).is_err());
    }
}
