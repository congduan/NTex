// ---------- LaTeX 源特征检测（M9 中文刀 6；三端共用的单一事实源） ----------
//
// 源含 `\documentclass` / `\usepackage` / `\begin{document}` 任一即判 LaTeX。
// 调用方（`ntex-dvi` CLI 的格式自动检测、`ntex-studio` 工作台的模式切换、
// Tauri/浏览器前端）据此决定是否套用注入的 `latex.fmt`；`ntex-tauri`
// `ui/main.js::looksLikeLatex` 是同一口径的 JS 侧实现（容器边界外的唯一副本）。
//
// **检测前先剥行注释**：`%` 到行尾是 TeX 注释，注释里出现的 `\documentclass`
// （被注释掉的模板头）不应触发格式切换；`\%`（转义百分号）不开启注释。近似
// 处理：只剥行注释——verbatim 环境内的 `%` 仍会被当注释，后果仅是模式判断
// 偏差（源码特征字符串在 verbatim 里出现的概率极低）。
//
// 本文件由 typeset/mod.rs `include!` 嵌入 `typeset` 模块。

/// 源码是否具备 LaTeX 特征（格式自动检测入口）。
///
/// 用法：宿主在编译前判一次，命中则 `import_state(latex.fmt 快照)` 并**不**
/// 预载 plain（fmt 已含目标格式全量状态，叠加会污染）。
pub fn looks_like_latex(src: &str) -> bool {
    for raw in src.lines() {
        // 剥行注释：转义对（`\%`、`\\` 等）原样保留两个字符，均不算注释起始。
        let mut line = String::new();
        let mut chars = raw.char_indices().peekable();
        while let Some((i, ch)) = chars.next() {
            if ch == '\\' {
                let byte_end = (i + ch.len_utf8() + 1).min(raw.len());
                line.push_str(&raw[i..byte_end]);
                chars.next();
                continue;
            }
            if ch == '%' {
                break;
            }
            line.push(ch);
        }
        if line.contains("\\documentclass")
            || line.contains("\\usepackage")
            || line.contains("\\begin{document}")
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod latex_detect_tests {
    use super::looks_like_latex;

    #[test]
    fn detects_latex_features() {
        assert!(looks_like_latex("\\documentclass{article}\n\\begin{document}"));
        assert!(looks_like_latex("\\usepackage{graphicx}"));
        assert!(looks_like_latex("foo\n\\begin{document}\nbar"));
        // 空格变体不命中（TeX 语义里 `\begin {document}` 合法，但三端口径只认
        // 紧凑写法——与 JS 侧 LATEX_SRC_RE / 既有 Rust 实现逐字一致）
        assert!(!looks_like_latex("\\begin {document}"));
    }

    #[test]
    fn plain_source_is_not_latex() {
        assert!(!looks_like_latex("\\font\\cmr=cmr10\\cmr hello\\end"));
        assert!(!looks_like_latex("\\tolerance 10000\n\\hsize 300pt"));
    }

    /// 注释里的特征不触发（注释掉的模板头是最常见的误报源）。
    #[test]
    fn commented_features_do_not_trigger() {
        assert!(!looks_like_latex("% \\documentclass{article}\nhi"));
        // 转义百分号不开启注释：`\%` 之后的特征仍算正文
        assert!(looks_like_latex("100\\% \\documentclass{article}"));
    }
}
