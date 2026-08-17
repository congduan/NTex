//! # ntex-dvi：DVI 写出器（M3-5-1）。
//!
//! 把 `ntex-layout` 的 `\shipout` 页面（[`BoxNode`] 树）+ 字体表
//! （[`FontMetrics`]）写成 DVI 二进制（dvitype.web / TeXbook 格式）。
//!
//! 关键约定：
//! - `pre` 的 num/den = 25400000/473628672（TeX 默认），此时 **1 DVI 单位 = 1 sp**
//!   （1/65536 pt），页面坐标（x 向右、y 向下，原点 = 盒子参考点）直接取节点维度；
//! - 支持的指令子集：`set_char` / `set_rule` / `push` / `pop` / `right` / `down` /
//!   `fnt_def` / `fnt_num` / `pre` / `bop` / `eop` / `post` / `post_post`；
//! - 页面参考点：`\shipout\hbox{...}` 的基线在原点（升部向上为负 y）；
//!   `\shipout\vbox{...}` 的首行基线在原点——与 TeX `\shipout` 语义一致，
//!   驱动（dvipdfmx 等）据内容求页面包围盒。
//!
//! M3-5 范围：断页 DP（`\vsize` 自动分页）与 `\box` 寄存器留待后续子步。

#![deny(unsafe_code)]

use ntex_font::FontMetrics;
use ntex_layout::node::{BoxKind, BoxNode, Node};

/// `pre` 的 num/den（TeX 默认：1 DVI 单位 = 1 sp）。
const DVI_NUM: u32 = 25_400_000;
const DVI_DEN: u32 = 473_628_672;

/// 把页面与字体表写成 DVI 字节。
///
/// - `pages`：`\shipout` 的页面（顺序 = DVI 页面顺序）；
/// - `fonts`：字体表快照（`FontId` 下标 = DVI 字体编号）。
pub fn write_dvi(pages: &[BoxNode], fonts: &[FontMetrics]) -> Vec<u8> {
    let mut w = Writer::new();
    w.pre();
    let mut prev_bop = -1i64; // 首页无前一 bop（TeX 写 -1）
    let mut last_bop = 0i64; // 最后（当前）一页的 bop 位置
    let mut max_h = 0i64; // post 的 l：最大页高
    let mut max_w = 0i64; // post 的 u：最大页宽
    for page in pages {
        let pos = w.out.len() as i64;
        w.page(page, fonts, prev_bop);
        prev_bop = pos;
        last_bop = pos;
        max_h = max_h.max(page.height + page.depth);
        max_w = max_w.max(page.width);
    }
    w.post(fonts, pages.len(), last_bop, max_h, max_w);
    w.out
}

/// DVI 写出器（内部状态：字节缓冲 + push/pop 栈深 + 当前字体 + 已定义字体）。
struct Writer {
    out: Vec<u8>,
    stack: u16,
    max_stack: u16,
    font: Option<u32>,
    /// 已发出过 fnt_def 的字体（TeX 每个字体只在首次使用页定义一次）。
    defined: Vec<bool>,
}

impl Writer {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            stack: 0,
            max_stack: 0,
            font: None,
            defined: Vec::new(),
        }
    }

    /// `pre`：i=2, num, den, mag, 注释。
    fn pre(&mut self) {
        self.out.push(247);
        self.out.push(2);
        self.out.extend(DVI_NUM.to_be_bytes());
        self.out.extend(DVI_DEN.to_be_bytes());
        self.out.extend(1000u32.to_be_bytes()); // mag = 1000
        const COMMENT: &str = "NTex DVI";
        self.out.push(COMMENT.len() as u8);
        self.out.extend(COMMENT.bytes());
    }

    /// 一页：bop（10 个计数 + 前一 bop 指针）→ 页面内容 → eop。
    /// 页面原点 (0,0) = 盒子**顶**；先 `down(height)` 把参考点（hbox 基线 /
    /// vbox 首行基线）移到 y=height（对照真实 TeX 的 `\shipout` 输出）。
    /// `\count0` = 1（plain TeX 格式默认；真实值随 `\count0` 赋值，切片固定 1）。
    fn page(&mut self, page: &BoxNode, fonts: &[FontMetrics], prev_bop: i64) {
        self.out.push(139); // bop
        self.out.extend(1u32.to_be_bytes()); // \count0 = 页码（plain 默认 1）
        for _ in 1..10 {
            self.out.extend(0u32.to_be_bytes()); // \count1..9 快照（全 0）
        }
        self.out.extend((prev_bop as u32).to_be_bytes()); // 首页 -1（0xFFFFFFFF）
        self.font = None;
        self.down(page.height); // 盒子顶在原点，参考点下移 height
        // 页面内定义首次使用的字体（TeX 行为：每个字体只在首个使用页 fnt_def）
        for (k, fm) in fonts.iter().enumerate() {
            if self.defined.len() <= k {
                self.defined.resize(k + 1, false);
            }
            if !self.defined[k] {
                self.fnt_def(k as u8, fm);
                self.defined[k] = true;
            }
        }
        match page.kind {
            BoxKind::HBox => self.hlist(&page.children),
            BoxKind::VBox => self.vlist(&page.children),
        }
        self.out.push(140); // eop
    }

    /// `post`：指针→post_post、num、den、mag、l/u/s/t，随后重复字体定义。
    ///
    /// 对照真实 TeX / dvipdfmx 实测约定：
    /// - post 的 4 字节指针字段 = **最后一页 bop 的位置**（dvipdfmx 用其回溯页面；
    ///   填"正确"的 post_post 位置反而被拒）；
    /// - 尾部 = `post_post` 块（249+指针4+版本2+4×223 哨兵），文件必须恰好以其
    ///   结尾，**全文不得出现 nop(0)**（dvipdfmx 报 "Unexpected op code: 0"；
    ///   故不追求 4 字节对齐）。
    fn post(&mut self, fonts: &[FontMetrics], total_pages: usize, last_bop: i64, max_h: i64, max_w: i64) {
        let post_pos = self.out.len() as u32;
        self.out.push(248);
        self.out.extend((last_bop as u32).to_be_bytes()); // dvipdfmx：最后一页 bop 位置
        self.out.extend(DVI_NUM.to_be_bytes());
        self.out.extend(DVI_DEN.to_be_bytes());
        self.out.extend(1000u32.to_be_bytes()); // mag
        self.out.extend((max_h as u32).to_be_bytes()); // l = 最大页高（height+depth）
        self.out.extend((max_w as u32).to_be_bytes()); // u = 最大页宽
        self.out.extend(self.max_stack.to_be_bytes()); // s = 最大栈深
        self.out.extend((total_pages as u16).to_be_bytes()); // t = 页数
        for (k, fm) in fonts.iter().enumerate() {
            self.fnt_def(k as u8, fm);
        }
        self.out.push(249);
        self.out.extend(post_pos.to_be_bytes()); // post_post → post 指针
        self.out.push(2); // 版本
        self.out.extend([223, 223, 223, 223]); // 尾部哨兵（dvipdfmx 严格校验）
    }

    /// `fnt_def1`：k(1) c(4) s(4) d(4) a(1) l(1) name。
    /// 对照真实 TeX 输出：`s` = 当前字号（sp）= design×scale/2^20；
    /// `d` = 设计字号（sp）。驱动按 `fix_word × s/2^20` 求字符宽度（DVI 单位 = sp）。
    fn fnt_def(&mut self, k: u8, fm: &FontMetrics) {
        self.out.push(243);
        self.out.push(k);
        self.out.extend(fm.checksum.to_be_bytes());
        let size_sp = ((fm.design_size_sp * fm.scale) + (1 << 19)) >> 20;
        self.out.extend((size_sp as u32).to_be_bytes()); // s = 当前字号 sp
        self.out.extend((fm.design_size_sp as u32).to_be_bytes()); // d = 设计字号 sp
        self.out.push(0); // 区域名长度
        self.out.push(fm.name.len() as u8);
        self.out.extend(fm.name.bytes());
    }

    // ---------- 水平/垂直列表渲染 ----------

    /// 水平列表：参考点 = 基线（x 向右推进）。
    /// 注意：`set_char` 后**不**输出 right——驱动按 TFM `fix_word × s/2^20`
    /// 自动前进字符宽度（对照真实 TeX DVI；重复输出会双倍间距）。
    fn hlist(&mut self, nodes: &[Node]) {
        for n in nodes {
            match n {
                Node::Char {
                    font,
                    charcode,
                    ..
                } => {
                    self.select_font(font.0);
                    self.set_char(*charcode);
                }
                Node::Glue { width, .. } | Node::Kern { width } => self.right(*width),
                Node::Rule {
                    width,
                    height,
                    depth,
                } => {
                    self.push();
                    self.down(-*height); // 规则顶在基线之上 height
                    self.set_rule(*width, *height + *depth);
                    self.pop();
                    self.right(*width);
                }
                Node::Box(inner) => {
                    self.push();
                    self.down(-inner.shift); // shift > 0 = 抬高参考点
                    match inner.kind {
                        BoxKind::HBox => self.hlist(&inner.children),
                        BoxKind::VBox => self.vlist(&inner.children),
                    }
                    self.pop();
                    self.right(inner.width);
                }
                _ => {} // Penalty/Leaders：切片不输出
            }
        }
    }

    /// 垂直列表：参考点 = 首行基线（y 向下推进）。
    fn vlist(&mut self, nodes: &[Node]) {
        for n in nodes {
            match n {
                Node::Box(b) => {
                    match b.kind {
                        BoxKind::HBox => self.hlist(&b.children),
                        BoxKind::VBox => self.vlist(&b.children),
                    }
                    self.down(-(b.height + b.depth));
                }
                Node::Glue { width, .. } | Node::Kern { width } => self.down(-*width),
                Node::Rule {
                    width,
                    height,
                    depth,
                } => {
                    self.push();
                    self.down(-*height);
                    self.set_rule(*width, *height + *depth);
                    self.pop();
                    self.down(-(*height + *depth));
                }
                _ => {}
            }
        }
    }

    // ---------- 指令字节 ----------

    fn set_char(&mut self, ch: u32) {
        if ch < 128 {
            self.out.push(ch as u8); // set_char_0..127
        } else {
            self.out.push(128); // set1
            self.out.push(ch as u8);
        }
    }

    /// `set_rule`：顶-左角在当前位置。
    fn set_rule(&mut self, w: i64, h: i64) {
        self.out.push(132);
        self.out.extend((h as u32).to_be_bytes());
        self.out.extend((w as u32).to_be_bytes());
    }

    fn select_font(&mut self, f: u32) {
        if self.font == Some(f) {
            return;
        }
        if f < 64 {
            self.out.push(171 + f as u8); // fnt_num_0..63
        } else {
            self.out.push(235); // fnt1
            self.out.push(f as u8);
        }
        self.font = Some(f);
    }

    /// 横向移动（跳过 0；按大小选 right1..right4，与 TeX 一致）。
    fn right(&mut self, dx: i64) {
        if dx == 0 {
            return;
        }
        let (op, n) = match dx {
            -128..=127 => (143, 1),
            -32768..=32767 => (144, 2),
            -8_388_608..=8_388_607 => (145, 3),
            _ => (146, 4),
        };
        self.out.push(op);
        self.out.extend((dx as u32).to_be_bytes()[4 - n..].to_vec());
    }

    /// 纵向移动（跳过 0；按大小选 down1..down4，与 TeX 一致）。正 = 向下。
    fn down(&mut self, dy: i64) {
        if dy == 0 {
            return;
        }
        let (op, n) = match dy {
            -128..=127 => (157, 1),
            -32768..=32767 => (158, 2),
            -8_388_608..=8_388_607 => (159, 3),
            _ => (160, 4),
        };
        self.out.push(op);
        self.out.extend((dy as u32).to_be_bytes()[4 - n..].to_vec());
    }

    fn push(&mut self) {
        self.out.push(141);
        self.stack += 1;
        self.max_stack = self.max_stack.max(self.stack);
    }

    fn pop(&mut self) {
        self.out.push(142);
        self.stack -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ntex_font::FontMetrics;
    use ntex_layout::node::{BoxNode, FontId, Node};
    use ntex_layout::typeset::Typesetter;

    fn cmr_metrics(name: &str) -> FontMetrics {
        FontMetrics {
            design_size_sp: 10 * 65_536,
            scale: 1 << 20,
            checksum: 0x1234_5678,
            name: name.to_owned(),
            chars: vec![None; 128],
            slant: 0,
            space: 218_453,
            space_stretch: 109_227,
            space_shrink: 72_818,
            x_height: 430_965,
            quad: 655_360,
            extra_space: 109_227,
        }
    }

    fn char_node(ch: u8, w: i64) -> Node {
        Node::Char {
            font: FontId(0),
            charcode: ch as u32,
            width: w,
            height: 430_000,
            depth: 0,
        }
    }

    fn hbox_page(children: Vec<Node>) -> BoxNode {
        BoxNode::new_hbox(children)
    }

    #[test]
    fn empty_dvi_has_structural_markers() {
        let dvi = write_dvi(&[], &[]);
        assert_eq!(dvi[0], 247, "pre");
        assert_eq!(dvi[1], 2, "pre 版本");
        // 文件必须以 02 + 4×223 哨兵恰好结尾（dvipdfmx 严格校验，无尾随 nop）
        assert!(
            dvi.ends_with(&[2, 223, 223, 223, 223]),
            "尾部应为版本 2 + 4×223 哨兵：{dvi:?}"
        );
        // post 与 post_post 都存在
        assert!(dvi.contains(&248));
        assert!(dvi.contains(&249));
    }

    #[test]
    fn writes_font_def_and_chars() {
        let fonts = vec![cmr_metrics("cmr10")];
        let page = hbox_page(vec![char_node(b'H', 500_000), char_node(b'i', 260_000)]);
        let dvi = write_dvi(&[page], &fonts);
        // fnt_def1(243) + k=0 + checksum + s=10pt(sp) + d=10pt(sp) + a=0 + l=5 + "cmr10"
        let mut fnt_def = vec![243, 0];
        fnt_def.extend(0x1234_5678u32.to_be_bytes());
        fnt_def.extend(655_360u32.to_be_bytes()); // s = 当前字号 10pt
        fnt_def.extend(655_360u32.to_be_bytes()); // d = 设计字号 10pt
        fnt_def.push(0);
        fnt_def.push(5);
        fnt_def.extend(b"cmr10");
        assert!(
            dvi.windows(fnt_def.len()).any(|w| w == fnt_def),
            "fnt_def 应有 cmr10 的 checksum/scale/design/name：{dvi:?}"
        );
        // fnt_num_0 = 171 + 0；set_char 'H'(72)、'i'(105)（相邻，宽度由驱动按 TFM 前进）
        assert!(dvi.contains(&171), "fnt_num(0) 应出现");
        assert!(
            dvi.windows(3).any(|w| w == [171, 72, 105]),
            "H、i 应相邻（set_char 后无 right）：{dvi:?}"
        );
    }

    #[test]
    fn glue_and_rules_advance() {
        let fonts = vec![cmr_metrics("cmr10")];
        let page = hbox_page(vec![
            char_node(b'a', 100),
            Node::Glue {
                width: 50,
                stretch: 0,
                shrink: 0,
                stretch_order: 0,
                shrink_order: 0,
            },
            Node::Rule {
                width: 30,
                height: 40,
                depth: 5,
            },
            char_node(b'b', 100),
        ]);
        let dvi = write_dvi(&[page], &fonts);
        // set_rule(132) 出现；push(141)/pop(142) 用于规则定位
        assert!(dvi.contains(&132));
        assert!(dvi.contains(&141));
        assert!(dvi.contains(&142));
    }

    #[test]
    fn multiple_pages_get_bop_chain() {
        let fonts = vec![cmr_metrics("cmr10")];
        let p1 = hbox_page(vec![char_node(b'a', 100)]);
        let p2 = hbox_page(vec![char_node(b'b', 100)]);
        let dvi = write_dvi(&[p1, p2], &fonts);
        // 两个 bop(139) 与两个 eop(140)
        assert_eq!(dvi.iter().filter(|&&b| b == 139).count(), 2);
        assert_eq!(dvi.iter().filter(|&&b| b == 140).count(), 2);
        // post 的 t = 2：248(1) + pp(4) + num/den/mag(12) + l/u(8) + s(2)，t 在 +27
        let t_pos = dvi.iter().position(|&b| b == 248).unwrap() + 1 + 4 + 4 + 4 + 4 + 4 + 4 + 2;
        assert_eq!(&dvi[t_pos..t_pos + 2], &[0, 2]);
    }

    // ---------- 与排版器集成（真实 cmr10） ----------

    #[test]
    fn typeset_shipout_to_dvi_with_cmr10() {
        let Some(path) = ntex_font::find_tfm("cmr10") else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let bytes = std::fs::read(path).expect("读取 cmr10.tfm");
        let fm = ntex_font::parse_tfm(&bytes).expect("解析 cmr10.tfm");

        let mut ts = Typesetter::with_tfm();
        let (pages, fonts) = ts
            .typeset_dvi(r"\font\cmr=cmr10\shipout\hbox{\cmr NTex}")
            .expect("排版失败");
        assert_eq!(pages.len(), 1);
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts[0].name, "cmr10");
        assert_eq!(fonts[0].checksum, fm.checksum, "checksum 应来自 TFM 头");

        let dvi = write_dvi(&pages, &fonts);
        // 页面上应含 N/T/e/x 的 set_char
        for ch in [b'N', b'T', b'e', b'x'] {
            assert!(dvi.contains(&ch), "缺字符 {ch} 的 set_char");
        }
        // 字体名出现两次：页面 fnt_def + post 字体列表
        let name: Vec<u8> = b"cmr10".to_vec();
        assert_eq!(
            dvi.windows(name.len()).filter(|w| *w == name.as_slice()).count(),
            2,
            "cmr10 应在页面与 post 各定义一次"
        );
    }
}
