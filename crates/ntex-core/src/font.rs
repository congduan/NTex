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

/// Unicode 码位上限（U+10FFFF）——Unicode 直映字体的 `\char` 合法上界。
///
/// 注意与 [`crate::token::Token`] 的 charcode 位宽（21 bit = 0x1FFFFF）区分：
/// 前者是 Unicode 标准约束，后者是 token 表示容量，取两者中更严的语义边界。
pub const UNICODE_MAX_CHARCODE: u32 = 0x10FFFF;

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

    /// 查询字体参数（sp，1-based TFM 参数序：1=slant 2=space … 5=x_height 6=quad
    /// 7=extra_space，8+=数学扩展）。
    ///
    /// TeX 内部单位 em/ex（tex.web scan_dimen：`em → quad(cur_font)`、
    /// `ex → x_height(cur_font)`）用。字体未加载/无该参数 → None（按 0 计）。
    fn font_param(&mut self, _font: u32, _param: usize) -> Option<i64> {
        None
    }

    /// 该字体的合法字符码上限（`\char`/`\iffontchar`/`\fontchar*` 的校验上界）。
    ///
    /// 默认 255——TeX 8-bit 语义，越界报 `! Bad character code (N).` 并恢复
    /// （tex.web §1108；TRIP/ETRIP 均有该错误块的硬口径）。Unicode 直映字体
    /// （OpenType，见 ntex-font `FontMetrics::unicode_native`）覆写为
    /// [`UNICODE_MAX_CHARCODE`]，对齐 XeTeX 的 `\char"4E00` 可排汉字。
    ///
    /// 未加载的字体槽（含 nullfont）一律按 255 处理。
    fn char_code_limit(&mut self, _font: u32) -> u32 {
        255
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
