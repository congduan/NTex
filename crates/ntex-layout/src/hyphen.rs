//! Liang 断字算法（M4-6）：模式 trie + 单词断点计算。
//!
//! TeX 断字（TeXbook 附录 H / Liang 博士论文）：
//! - 模式是数字与字母交错的串，如 `ab5c`（5 在 b 后 = b 与 c 之间 gap=5）、
//!   `3ph`（3 在 p 前 = p 之前的 gap=3）、`.ach4`（词首 `.` 限定；h 后 gap=4）；
//! - 数字的 gap 位置 = 数字出现时已匹配的字母数（`ab5c` → 位置 2、`3ph` → 位置 0）；
//! - 对单词的每个子串匹配模式，同一 gap 位置取最大数字；
//! - 奇数数字 → 该处可断字。
//!
//! 词界限制（`.`）暂不参与断点过滤（ETRIP 校准阶段补）。

use std::collections::HashMap;

/// 模式 trie 节点：字母 → 子节点。
#[derive(Debug, Default, Clone)]
pub struct TrieNode {
    next: HashMap<u8, TrieNode>,
    /// 该字母**前**的 gap 数字（作用于该字母下标位置）。
    gap_before: u8,
    /// 完整匹配结束时词尾位置的 gap 数字（作用于位置 = 字母数）。
    terminal_gap: u8,
    /// 是否为某模式的结束节点。
    terminal: bool,
}

/// Liang 模式表（`\patterns` 解析结果）。
#[derive(Debug, Clone, Default)]
pub struct PatternTrie {
    root: TrieNode,
    /// 模式总数（诊断用）。
    pub count: usize,
}

/// 解析单条模式 → (字母序列, gaps)；`gaps[i]` = 第 i 个字母**前**的 gap
/// （`gaps.len() == letters.len() + 1`，末位为词尾 gap）。
fn parse_pattern(p: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut letters = Vec::new();
    let mut gaps = vec![0u8];
    for &b in p {
        if b.is_ascii_digit() {
            let pos = letters.len();
            gaps[pos] = b - b'0';
        } else if b.is_ascii_alphabetic() {
            letters.push(b.to_ascii_lowercase());
            gaps.push(0);
        }
        // '.' 词界标志：跳过（暂不参与 trie）
    }
    (letters, gaps)
}

impl PatternTrie {
    /// 是否已加载模式（空表跳过断字）。
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// 解析 `\patterns{...}` 的模式表（空格/换行分隔多条）。
    pub fn parse(patterns: &[u8]) -> Self {
        let mut trie = PatternTrie::default();
        for pat in patterns.split(|&b| b == b' ' || b == b'\n' || b == b'\r') {
            if pat.is_empty() {
                continue;
            }
            trie.insert(pat);
        }
        trie
    }

    /// 插入单条模式到 trie。
    fn insert(&mut self, pat: &[u8]) {
        let (letters, gaps) = parse_pattern(pat);
        if letters.is_empty() {
            return;
        }
        let mut node = &mut self.root;
        let n = letters.len();
        for (i, ch) in letters.iter().enumerate() {
            node = node.next.entry(*ch).or_default();
            // gaps[i] = 第 i 个字母前的 gap → 存该字母节点的 gap_before
            node.gap_before = node.gap_before.max(gaps[i]);
            if i == n - 1 {
                node.terminal = true;
                node.terminal_gap = node.terminal_gap.max(gaps[n]);
            }
        }
        self.count += 1;
    }

    /// 计算单词的断点位置（返回断点处字母下标：断在 `breaks[i]` 字母之后，
    /// 如 "hy-phen-ation" → [2, 7]）。
    pub fn hyphenate(&self, word: &[u8]) -> Vec<usize> {
        if word.len() < 2 {
            return Vec::new();
        }
        // marks[j] = 位置 j（第 j 个字母前）的 gap 最大数字
        let mut marks = vec![0u8; word.len() + 1];
        for start in 0..word.len() {
            let mut node = &self.root;
            let mut j = start;
            loop {
                if j >= word.len() {
                    break;
                }
                let Some(next) = node.next.get(&word[j].to_ascii_lowercase()) else {
                    break;
                };
                // 进入字母 j（匹配它之前）：其 gap_before 作用于位置 j
                marks[j] = marks[j].max(next.gap_before);
                node = next;
                j += 1;
            }
            // 完整匹配到词尾：末字母的 terminal_gap 作用于位置 word.len()
            if start < word.len() && j == word.len() && node.terminal {
                marks[word.len()] = marks[word.len()].max(node.terminal_gap);
            }
        }
        // 奇数 gap → 断点（词首位置 0 / 词尾 word.len() 不断）
        (1..word.len())
            .filter(|&i| marks[i] % 2 == 1)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_digit_after_letter_gap() {
        let t = PatternTrie::parse(b"ab5c");
        assert_eq!(t.count, 1);
        // ab5c：5 在 b 后 → b 与 c 之间 gap=5 → 断点 2
        assert_eq!(t.hyphenate(b"abc"), vec![2]);
        assert_eq!(t.hyphenate(b"xabcx"), vec![3]);
    }

    #[test]
    fn classic_hyphenation_matches_tex() {
        // 与 pdfTeX + hyphen.tex 实测一致：hyphenation → hy-phen-ation（断点 2, 6）
        // hy3phen5ation：gap(y,p)=3（位置 2）、gap(n,a)=5（位置 6）
        let t = PatternTrie::parse(b"hy3phen5ation");
        assert_eq!(t.hyphenate(b"hyphenation"), vec![2, 6]);
    }

    #[test]
    fn digit_before_letter_gap_position() {
        // 3ph：3 在 p 前 → p 之前的 gap=3
        let t = PatternTrie::parse(b"3ph");
        // "yph"：y-p 间 gap=3 → 断点 1
        assert_eq!(t.hyphenate(b"yph"), vec![1]);
    }

    #[test]
    fn odd_gaps_only() {
        let t = PatternTrie::parse(b"ab4cd");
        assert_eq!(t.hyphenate(b"abcd"), vec![], "偶数 gap 不断");
    }

    #[test]
    fn multiple_patterns_overlap_take_max() {
        let t = PatternTrie::parse(b"ab5c ab3cd");
        // ab5c → gap(b,c)=5；ab3cd → gap(b,c)=3 → 重叠取 max=5
        assert_eq!(t.hyphenate(b"abcd"), vec![2]);
    }

    #[test]
    fn word_boundary_dot_patterns() {
        // ma5chine：gap(a,c)=5 → machine 在 a 后断（ma-chine）
        let t = PatternTrie::parse(b"ma5chine");
        assert_eq!(t.hyphenate(b"machine"), vec![2]);
        // .ach4：h 后 gap=4（偶数）→ 不产生断点（压制模式）
        let t2 = PatternTrie::parse(b".ach4");
        assert_eq!(t2.hyphenate(b"machine"), Vec::<usize>::new());
    }

    #[test]
    fn no_matches_no_breaks() {
        let t = PatternTrie::parse(b"xy5z");
        assert_eq!(t.hyphenate(b"abcdef"), Vec::<usize>::new());
    }

    #[test]
    fn short_words_never_break() {
        let t = PatternTrie::parse(b"ab5c");
        assert_eq!(t.hyphenate(b"a"), Vec::<usize>::new());
        assert_eq!(t.hyphenate(b"ab"), Vec::<usize>::new());
    }

    #[test]
    fn trailing_digit_is_word_end_gap() {
        // ab5：5 在 b 后且为末 → 词尾 gap=5（位置 2 = 词尾）；词尾位置不断
        let t = PatternTrie::parse(b"ab5");
        assert_eq!(t.hyphenate(b"abx"), Vec::<usize>::new(), "词尾 gap 不产生断点");
    }
}
