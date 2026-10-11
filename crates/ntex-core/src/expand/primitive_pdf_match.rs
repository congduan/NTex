// \pdfmatch 的正则子集引擎（刀H）。
//
// pdfTeX 语义（GT 对拍 2026-10-11，p1/p2 探针）：可展开，返回
// 1=匹配 / 0=不匹配 / -1=pattern 非法；搜索语义（未锚定即在任意位置匹配）。
//
// 引擎为回溯式微型实现，覆盖 microtype-pdftex.def 两个 pattern
// （L69 `^-*[0-9]+ *$`、L86 `^([0-9]+([.,][0-9]+)?|[.,][0-9]+)(em|ex|...)? *$`）
// 所需的 POSIX 子集：
//   字面字符、`.`、`[...]`（范围/取反/`]` 首位字面）、`(...)` 分组、`|` 选择、
//   `*` `+` `?` 量词（无贪回影响——只取布尔结果）、`^`/`$` 锚、`\` 转义标点。
// 其余 POSIX 语法（`{m,n}` 计数量词、`[:alpha:]` 类、后向引用）不识别——
// 解析失败即 -1（pdfTeX 对坏 pattern 同返回 -1），见 KNOWN-SIMPLIFICATIONS。
//
// （include! 分片：无 use，Token 等用 expand/mod.rs 顶部既有 import。）

/// 正则节点：单个可量化原子。
#[derive(Debug, Clone, PartialEq, Eq)]
enum RxAtom {
    Char(u8),
    Any,
    /// 字符类；`items` 为单字节或字节范围；`neg` = `[^...]`。
    Class { neg: bool, items: Vec<(u8, u8)> },
    Group(Vec<Vec<RxItem>>),
    /// `^`：仅串首匹配。
    Start,
    /// `$`：仅串尾匹配。
    End,
}

/// 量词。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RxQuant {
    Once,
    Opt,
    Star,
    Plus,
}

/// 序列项：原子 + 量词。
#[derive(Debug, Clone, PartialEq, Eq)]
struct RxItem {
    atom: RxAtom,
    quant: RxQuant,
}

/// 解析 pattern；非法返回 None（\pdfmatch 展开为 -1）。
fn rx_parse(pattern: &[u8]) -> Option<Vec<Vec<RxItem>>> {
    struct P<'a> {
        b: &'a [u8],
        i: usize,
    }
    impl P<'_> {
        fn peek(&self) -> Option<u8> {
            self.b.get(self.i).copied()
        }
        fn bump(&mut self) -> Option<u8> {
            let c = self.peek()?;
            self.i += 1;
            Some(c)
        }
        // alternation := seq ('|' seq)*
        fn alternation(&mut self, depth: usize) -> Option<Vec<Vec<RxItem>>> {
            let mut alts = vec![self.seq(depth)?];
            while self.peek() == Some(b'|') {
                self.i += 1;
                alts.push(self.seq(depth)?);
            }
            Some(alts)
        }
        // seq := item*，在 `|` 或 `)`（depth>0）或串尾停
        fn seq(&mut self, depth: usize) -> Option<Vec<RxItem>> {
            let mut items = Vec::new();
            loop {
                match self.peek() {
                    None | Some(b'|') => break,
                    Some(b')') if depth > 0 => break,
                    Some(b')') => return None, // 顶层裸 `)`：坏 pattern
                    Some(_) => items.push(self.item()?),
                }
            }
            Some(items)
        }
        fn item(&mut self) -> Option<RxItem> {
            let atom = self.atom()?;
            let quant = match self.peek() {
                Some(b'*') => {
                    self.i += 1;
                    RxQuant::Star
                }
                Some(b'+') => {
                    self.i += 1;
                    RxQuant::Plus
                }
                Some(b'?') => {
                    self.i += 1;
                    RxQuant::Opt
                }
                _ => RxQuant::Once,
            };
            Some(RxItem { atom, quant })
        }
        fn atom(&mut self) -> Option<RxAtom> {
            Some(match self.bump()? {
                b'.' => RxAtom::Any,
                b'^' => RxAtom::Start,
                b'$' => RxAtom::End,
                b'(' => {
                    let alts = self.alternation(1)?;
                    if self.bump() != Some(b')') {
                        return None;
                    }
                    RxAtom::Group(alts)
                }
                b'[' => self.class()?,
                b'\\' => {
                    let c = self.bump()?;
                    if c.is_ascii_alphanumeric() {
                        // `\d` 等类速记不在子集内（pdfTeX 识别，microtype 不用）
                        return None;
                    }
                    RxAtom::Char(c)
                }
                // `*+?` 无原子可量化：坏 pattern
                b'*' | b'+' | b'?' => return None,
                c => RxAtom::Char(c),
            })
        }
        // class := '[' ['^'] items ']'；`]` 紧跟 `[`（或 `[^`）时为字面
        fn class(&mut self) -> Option<RxAtom> {
            let neg = if self.peek() == Some(b'^') {
                self.i += 1;
                true
            } else {
                false
            };
            let mut items = Vec::new();
            let mut first = true;
            loop {
                let c = self.bump()?;
                if c == b']' && !first {
                    break;
                }
                first = false;
                let lo = if c == b'\\' { self.bump()? } else { c };
                if self.peek() == Some(b'-') && self.b.get(self.i + 1) != Some(&b']') {
                    self.i += 1; // 吞 '-'
                    let hi_c = self.bump()?;
                    let hi = if hi_c == b'\\' { self.bump()? } else { hi_c };
                    items.push((lo, hi));
                } else {
                    items.push((lo, lo));
                }
            }
            Some(RxAtom::Class { neg, items })
        }
    }
    let mut p = P { b: pattern, i: 0 };
    let alts = p.alternation(0)?;
    if p.i != pattern.len() {
        return None; // 顶层有多余 `)`
    }
    Some(alts)
}

fn class_matches(neg: bool, items: &[(u8, u8)], c: u8) -> bool {
    let inside = items.iter().any(|&(lo, hi)| c >= lo && c <= hi);
    inside != neg
}

/// 单原子在 `pos` 的全部可行终点（分组/选择可变长 → 多终点）。
fn atom_ends(atom: &RxAtom, pos: usize, text: &[u8]) -> Vec<usize> {
    match atom {
        RxAtom::Char(c) => (text.get(pos) == Some(c)).then_some(pos + 1).into_iter().collect(),
        RxAtom::Any => (pos < text.len()).then_some(pos + 1).into_iter().collect(),
        RxAtom::Class { neg, items } => text
            .get(pos)
            .filter(|&&c| class_matches(*neg, items, c))
            .map(|_| pos + 1)
            .into_iter()
            .collect(),
        RxAtom::Start => (pos == 0).then_some(pos).into_iter().collect(),
        RxAtom::End => (pos == text.len()).then_some(pos).into_iter().collect(),
        RxAtom::Group(alts) => {
            let mut ends = Vec::new();
            for alt in alts {
                seq_ends(alt, 0, pos, text, &mut ends);
            }
            ends.sort_unstable();
            ends.dedup();
            ends
        }
    }
}

/// 序列 `seq[idx..]` 自 `pos` 起的全部可行终点（回溯枚举；pattern 微型，
/// 不做记忆化）。
fn seq_ends(seq: &[RxItem], idx: usize, pos: usize, text: &[u8], out: &mut Vec<usize>) {
    let Some(item) = seq.get(idx) else {
        out.push(pos);
        return;
    };
    match item.quant {
        RxQuant::Once => {
            for e in atom_ends(&item.atom, pos, text) {
                seq_ends(seq, idx + 1, e, text, out);
            }
        }
        RxQuant::Opt => {
            // 贪婪序无关布尔结果：先吃后跳
            for e in atom_ends(&item.atom, pos, text) {
                seq_ends(seq, idx + 1, e, text, out);
            }
            seq_ends(seq, idx + 1, pos, text, out);
        }
        RxQuant::Star | RxQuant::Plus => {
            let min_left = if item.quant == RxQuant::Plus { 1 } else { 0 };
            rep_ends(&item.atom, min_left, pos, seq, idx, text, out);
        }
    }
}

/// 重复原子：`min_left` 为还需满足的最少次数。空匹配不推进（防死循环）；
/// 剩余下限可由空匹配凑足时直接收尾。
fn rep_ends(
    atom: &RxAtom,
    min_left: u32,
    pos: usize,
    seq: &[RxItem],
    idx: usize,
    text: &[u8],
    out: &mut Vec<usize>,
) {
    if min_left == 0 {
        seq_ends(seq, idx + 1, pos, text, out);
    }
    for e in atom_ends(atom, pos, text) {
        if e == pos {
            if min_left > 0 {
                // 只剩空匹配：每个空重复各满足一个下限单位
                seq_ends(seq, idx + 1, pos, text, out);
            }
            continue;
        }
        rep_ends(atom, min_left.saturating_sub(1), e, seq, idx, text, out);
    }
}

/// `\pdfmatch` 值语义：1=匹配，0=不匹配，-1=pattern 非法。
/// 搜索语义：未锚定即从每个起点尝试（POSIX regexec 左最匹配的布尔化）。
pub(crate) fn pdf_match_value(pattern: &[u8], subject: &[u8]) -> i64 {
    let Some(alts) = rx_parse(pattern) else {
        return -1;
    };
    for start in 0..=subject.len() {
        let mut ends = Vec::new();
        for alt in &alts {
            seq_ends(alt, 0, start, subject, &mut ends);
        }
        if !ends.is_empty() {
            return 1;
        }
    }
    0
}

#[cfg(test)]
mod pdf_match_tests {
    use super::{pdf_match_value, rx_parse};

    fn m(p: &str, s: &str) -> i64 {
        pdf_match_value(p.as_bytes(), s.as_bytes())
    }

    #[test]
    fn gt_probes_reproduce() {
        // p1/p2 GT 探针值（pdflatex 2026-10-11 实测）
        assert_eq!(m("ab", "xaby"), 1);
        assert_eq!(m("a(b)(c)", "xabc"), 1);
        assert_eq!(m("zzz", "xaby"), 0);
        assert_eq!(m("[", "x"), -1);
    }

    #[test]
    fn microtype_patterns() {
        // microtype-pdftex.def L69 \MT@ifint
        assert_eq!(m("^-*[0-9]+ *$", "100"), 1);
        assert_eq!(m("^-*[0-9]+ *$", "-42"), 1);
        assert_eq!(m("^-*[0-9]+ *$", "  7 "), 0); // `^` 锚定：前导空格不匹配
        assert_eq!(m("^-*[0-9]+ *$", "10pt"), 0);
        assert_eq!(m("^-*[0-9]+ *$", ""), 0);
        assert_eq!(m("^-*[0-9]+ *$", "-"), 0);
        // L86 \MT@ifdimen（换行接续后的完整形）
        let dimen = "^([0-9]+([.,][0-9]+)?|[.,][0-9]+)(em|ex|cm|mm|in|pc|pt|dd|cc|bp|sp|nd|nc|px)? *$";
        assert_eq!(m(dimen, "10pt"), 1);
        assert_eq!(m(dimen, "10.5pt"), 1);
        assert_eq!(m(dimen, "10,5em"), 1);
        assert_eq!(m(dimen, ".5ex"), 1);
        assert_eq!(m(dimen, "12"), 1);
        assert_eq!(m(dimen, "1em2"), 0);
        assert_eq!(m(dimen, "abc"), 0);
        // ` *$` 尾缀在单位组之后：数字与单位间夹空格不匹配
        assert_eq!(m(dimen, "10 pt"), 0);
    }

    #[test]
    fn anchors_quantifiers_classes() {
        assert_eq!(m("^abc$", "abc"), 1);
        assert_eq!(m("^abc$", "xabc"), 0);
        assert_eq!(m("^abc$", "abcx"), 0);
        assert_eq!(m("a.c", "abc"), 1);
        assert_eq!(m("a[0-9]+b", "x a123b y"), 1);
        assert_eq!(m("[^0-9]+", "12a34"), 1);
        assert_eq!(m("^[^0-9]+$", "12a34"), 0);
        assert_eq!(m("[]]", "]"), 1); // `]` 首位为字面
        assert_eq!(m("a\\*b", "a*b"), 1);
        assert_eq!(m("a\\*b", "aab"), 0);
        assert_eq!(m("(ab)+", "ababx"), 1);
        assert_eq!(m("x(y?z)*w", "xzzw"), 1); // 空匹配重复不挂死
        assert_eq!(m("(a*)+", "b"), 1); // 空匹配满足 Plus 下限
    }

    #[test]
    fn bad_patterns_return_minus_one() {
        for p in ["[a-", "a)", "(a", "a\\", "*a", "a\\d", "z)"] {
            assert_eq!(rx_parse(p.as_bytes()), None, "pattern {p} 应解析失败");
            assert_eq!(m(p, "a"), -1);
        }
    }
}
