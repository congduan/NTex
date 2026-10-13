// ---------- LaTeX 源特征检测（M9 中文刀 6；三端共用的单一事实源） ----------
//
// 源含 `\documentclass` / `\usepackage` / `\begin{document}` / **任一
// `\begin{<env>}`·`\end{<env>}` 环境标记** 即判 LaTeX。
// 调用方（`ntex-dvi` CLI 的格式自动检测、`ntex-studio` 工作台的模式切换、
// Tauri/浏览器前端）据此决定是否套用注入的 `latex.fmt`；`ntex-tauri`
// `ui/main.js::looksLikeLatex` 是同一口径的 JS 侧实现（容器边界外的唯一副本）。
//
// **片段（fragment）口径（2026-10-13）**：`\begin{center}` / `\begin{tabular}`
// 这类**没有** `\documentclass`/`\begin{document}` 的裸片段同样是 LaTeX 语料——
// 旧规则只认三个整文档特征，片段被误判成 plain 后走 plain 语义（`\begin`
// undefined、`&` 成对齐制表符），现场就是"粘一段 table 进工作台，渲染出的是
// 一行乱码"。工作台侧另有 [`wrap_latex_fragment`] 给片段补最小文档壳。
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
        let line = strip_line_comment(raw);
        if line.contains("\\documentclass")
            || line.contains("\\usepackage")
            || line.contains("\\begin{document}")
            || has_env_marker(&line)
        {
            return true;
        }
    }
    false
}

/// 源码是否需要补最小文档壳（工作台专用，CLI/引擎**不**自动补）。
///
/// 返回 `Some(补壳后的源)`：源是 LaTeX 语料（[`looks_like_latex`]）却既无
/// `\documentclass` 也无 `\begin{document}`——工作台（Tauri 前端 / studio）把它
/// 当"片段"编译：`\documentclass{article}\begin{document}` 拼在**首行行首**、
/// `\end{document}` 另起一行追加，故用户源码的行号（转录 `l.N`）一列不移。
///
/// 返回 `None`：源已是完整文档（有 `\documentclass` 或 `\begin{document}`）、
/// 或根本不是 LaTeX（plain/TRIP 系源不受影响）。有 `\documentclass` 但缺
/// `\begin{document}` 的**残缺前导**也不补——LaTeX 自己的 "Missing
/// \begin{document}" 才是该形态的正确诊断。
///
/// 这是**工作台预览策略**而非引擎语义：`ntex-dvi`/`ntex-wasm` 的原语面照旧
/// TeX 口径（裸片段报错即报错），只有编辑器形态的前端调用本函数。
pub fn wrap_latex_fragment(src: &str) -> Option<String> {
    if !looks_like_latex(src) {
        return None;
    }
    let mut has_documentclass = false;
    let mut has_begin_document = false;
    for raw in src.lines() {
        let line = strip_line_comment(raw);
        if line.contains("\\documentclass") {
            has_documentclass = true;
        }
        if line.contains("\\begin{document}") {
            has_begin_document = true;
        }
    }
    if has_documentclass || has_begin_document {
        return None;
    }
    // 前缀拼在首行行首（不新增行）、尾壳另起一行（只影响末尾行号之后的行）
    // ——用户源码的 l.N 与编辑器行号恒对齐。
    let mut out = String::with_capacity(src.len() + 48);
    out.push_str("\\documentclass{article}\\begin{document}");
    out.push_str(src);
    if !src.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("\\end{document}\n");
    Some(out)
}

/// 剥掉行注释（`%` 到行尾；`\%`/`\\` 等转义对原样保留，不开启注释）。
fn strip_line_comment(raw: &str) -> String {
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
    line
}

/// `\begin{<env>}` / `\end{<env>}`（紧凑写法，环境名非空且不以 `}` 起头）。
///
/// 与 JS 侧 `LATEX_SRC_RE` / 既有 Rust 实现同口径：`\begin {center}`（中间带
/// 空格）**不**命中——三端口径只认紧凑写法。
fn has_env_marker(line: &str) -> bool {
    for kw in ["\\begin{", "\\end{"] {
        let mut rest = line;
        while let Some(i) = rest.find(kw) {
            let after = &rest[i + kw.len()..];
            match after.chars().next() {
                Some(c) if c != '}' && !c.is_whitespace() => return true,
                _ => {}
            }
            rest = after;
        }
    }
    false
}

#[cfg(test)]
mod latex_detect_tests {
    use super::{looks_like_latex, wrap_latex_fragment};

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
    fn detects_latex_fragments() {
        // 片段（无 documentclass/document）同样是 LaTeX 语料：`\begin{...}` 环境
        // 标记任一命中。现场：粘一段 `\begin{tabular}` 进工作台被当 plain →
        // `\begin` undefined、`&` 成对齐制表符 → 渲染成一行乱码。
        assert!(looks_like_latex(
            "\\begin{center}\n\\begin{tabular}{c c c}\n a & b \\\\\n\\end{tabular}\n\\end{center}"
        ));
        assert!(looks_like_latex("\\begin{itemize}\n\\item x\n\\end{itemize}"));
        assert!(looks_like_latex("\\end{document}"));
        // 紧凑写法才命中：`\begin {tabular}` 不命中；空名 `\begin{}` 不命中
        assert!(!looks_like_latex("\\begin {tabular}{c} a \\end {tabular}"));
        assert!(!looks_like_latex("\\begin{} x \\end{}"));
        // 注释里的环境标记不触发
        assert!(!looks_like_latex("% \\begin{tabular}{c}\nplain text"));
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

    #[test]
    fn wraps_fragment_and_keeps_line_numbers() {
        let frag = "\\begin{center}\n\\begin{tabular}{c}\n cell1 \\\\\n\\end{tabular}\n\\end{center}\n";
        let wrapped = wrap_latex_fragment(frag).expect("片段应补壳");
        assert!(wrapped.starts_with("\\documentclass{article}\\begin{document}"));
        assert!(wrapped.trim_end().ends_with("\\end{document}"));
        // 用户第 n 行 = 补壳后第 n 行（前缀只在首行行首、尾壳另起一行）
        let a: Vec<&str> = frag.lines().collect();
        let b: Vec<&str> = wrapped.lines().collect();
        assert_eq!(a.len() + 1, b.len(), "只多出尾壳一行：{b:?}");
        for (i, line) in a.iter().enumerate() {
            assert!(
                b[i].ends_with(line),
                "第 {} 行应保留（仅首行前拼壳）：{:?} vs {:?}",
                i + 1,
                b[i],
                line
            );
        }
    }

    #[test]
    fn does_not_wrap_complete_documents_or_plain() {
        assert!(wrap_latex_fragment("\\documentclass{book}\n\\begin{document}\nx").is_none());
        assert!(wrap_latex_fragment("\\begin{document}\nx\n\\end{document}").is_none());
        // 有 documentclass 缺 document（残缺前导）不补：LaTeX 自身的
        // "Missing \begin{document}" 才是正确诊断
        assert!(wrap_latex_fragment("\\documentclass{article}\nhi").is_none());
        // 非 LaTeX 源不补（plain 不受影响）
        assert!(wrap_latex_fragment("\\font\\cmr=cmr10 hello\\end").is_none());
    }
}
