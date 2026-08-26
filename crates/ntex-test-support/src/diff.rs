//! 日志归一化与文本 diff。
//!
//! 差分测试必须对比"确定性内容"：日志中的引擎横幅、产物统计等行在不同环境
//! （版本、绝对路径、平台）下必然不同，比较前先归一化剔除。

use std::sync::LazyLock;

use similar::{ChangeTag, TextDiff};

/// 行级归一化规则：`(正则, 替换)`。
/// 命中即整行替换为占位文本，保证两端日志逐行可比。
static NORMALIZE_RULES: LazyLock<Vec<(regex::Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        // 引擎横幅：This is pdfTeX, Version ... (TeX Live ...) (preloaded format=...)
        (
            regex::Regex::new(r"^This is [A-Za-z]+, Version .* \(preloaded format=.*\)$").unwrap(),
            "<ENGINE BANNER>",
        ),
        // 本引擎横幅：This is NTex, Version 0.1.0 (TRIP/ETRIP pipeline v1)
        (
            regex::Regex::new(r"^This is NTex, Version .*$").unwrap(),
            "<ENGINE BANNER>",
        ),
        // 产物统计行（含路径与页数）
        (
            regex::Regex::new(r"^Output written on .* \(\d+ pages?, .*\)\.$").unwrap(),
            "<OUTPUT STATS>",
        ),
        // 日志转录行（含路径）
        (
            regex::Regex::new(r"^Transcript written on .*\.$").unwrap(),
            "<TRANSCRIPT>",
        ),
    ]
});

/// 归一化日志文本：逐行应用规则，未命中的行原样保留。
pub fn normalize_log(text: &str) -> String {
    text.lines()
        .map(|line| {
            for (re, repl) in NORMALIZE_RULES.iter() {
                if re.is_match(line) {
                    return (*repl).to_owned();
                }
            }
            line.to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 渲染行级 diff（上下文窗口为 `context` 行），供报告展示。
pub fn render_diff(expected: &str, actual: &str, context: usize) -> String {
    let diff = TextDiff::from_lines(expected, actual);
    let mut out = String::new();
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => out.push(' '),
            ChangeTag::Delete => out.push('-'),
            ChangeTag::Insert => out.push('+'),
        }
        out.push_str(change.value());
    }
    // 简化：上下文裁剪交给展示层；此处返回全量带标记 diff，行数多时调用方自行截断。
    let _ = context;
    out
}

/// 字节级比较（用于 DVI/PDF 等二进制产物）。
pub fn bytes_differ(a: &[u8], b: &[u8]) -> bool {
    a != b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_engine_banner() {
        let log = "This is pdfTeX, Version 3.141592653-2.6-1.40.26 (TeX Live 2025) (preloaded format=pdflatex)\nsome real line\n";
        let normalized = normalize_log(log);
        assert!(normalized.contains("<ENGINE BANNER>"));
        assert!(normalized.contains("some real line"));
        assert!(!normalized.contains("This is pdfTeX"));
    }

    #[test]
    fn normalize_strips_output_stats() {
        let log = "Output written on /tmp/x.pdf (12 pages, 345678 bytes).\n";
        assert!(normalize_log(log).contains("<OUTPUT STATS>"));
    }

    #[test]
    fn render_diff_marks_changes() {
        let a = "line1\nline2\n";
        let b = "line1\nchanged\n";
        let out = render_diff(a, b, 2);
        assert!(out.contains("-line2"));
        assert!(out.contains("+changed"));
        assert!(out.contains(" line1"));
    }

    #[test]
    fn bytes_differ_detects_difference() {
        assert!(!bytes_differ(b"abc", b"abc"));
        assert!(bytes_differ(b"abc", b"abd"));
        assert!(bytes_differ(b"abc", b""));
    }
}
