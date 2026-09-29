//! # ntex-dvi：DVI 写出器（M3-5）。
//!
//! 把 `ntex-layout` 的页面（[`BoxNode`] 树）+ 字体表（[`FontMetrics`]）写成
//! DVI 二进制（dvitype.web / TeXbook 格式），逐字节对照真实 TeX（tex.web
//! `ship_out`/`hlist_out`/`vlist_out`/`movement`）：
//!
//! - 坐标惰性模型：`cur_h`/`cur_v` 累积未输出的位移，仅在字符/规则等需要
//!   定位时经 [`synch_h`/`synch_v`] 发出 `movement`（tex.web §261-280）；
//! - `movement` 用 down/right 栈做 w/x/y/z 命中优化（tex.web §297-366）：
//!   相同位移在命中时可改写为 w0/y0 等一字节指令；
//! - `set_rule` 语义：当前点为矩形**左下角**，向上画高、向右画宽，h 自动前进
//!   （dvitype.web §456）；`put_rule` 同但不前进；
//! - `fnt_def` 在字体**首次使用处**惰性发出（`font_used` 全局标记），post 段
//!   只列使用过的字体（TeX 倒序）；
//! - push/pop 对空盒自动抵消（tex.web `dvi_pop` §371）。
//!
//! M3-5 范围：自动分页（`\vsize`）与 `\shipout` 页面；insert/mark/leaders 留待。

#![deny(unsafe_code)]

use ntex_font::FontMetrics;
use ntex_layout::node::{BoxKind, BoxNode, Node};

/// `pre` 的 num/den（TeX 默认：1 DVI 单位 = 1 sp）。
const DVI_NUM: u32 = 25_400_000;
const DVI_DEN: u32 = 473_628_672;

/// 把页面与字体表写成 DVI 字节。
///
/// - `pages`：`\shipout` / 自动分页的页面（顺序 = DVI 页面顺序）；
/// - `fonts`：字体表快照（下标 = 字体编号）。
///
/// bop 计数：无 `\count0..9` 快照时的回落——count0 = 页序号（plain 的
/// `\advancepageno` 语义），count1..9 = 0。携带快照用
/// [`write_dvi_with_counts`]（输出例程刀 5）。
pub fn write_dvi(pages: &[BoxNode], fonts: &[FontMetrics]) -> Vec<u8> {
    let synthetic: Vec<[i64; 10]> = (1..=pages.len() as i64)
        .map(|n| {
            let mut c = [0i64; 10];
            c[0] = n;
            c
        })
        .collect();
    write_dvi_with_counts(pages, &synthetic, fonts)
}

/// 同 [`write_dvi`]，bop 的 10 计数字改取各页 shipout 边界的 `\count0..9`
/// 快照（输出例程刀 5：tex.web `ship_out` 的 `dvi_out(count(k))`，全量写入
/// 不截断——截断只发生在 log/终端的 `[...]` 页标签，tex.web L12694-12699）。
///
/// - `counts`：与 `pages` 一一对应；不足处（调用方快照缺失）按 0 补齐。
pub fn write_dvi_with_counts(
    pages: &[BoxNode],
    counts: &[[i64; 10]],
    fonts: &[FontMetrics],
) -> Vec<u8> {
    let mut w = Writer::new(fonts);
    w.pre();
    let mut prev_bop = -1i64; // 首页无前一 bop（TeX 写 -1）
    let mut last_bop = 0i64;
    let mut max_h = 0i64; // post 的 l = max(页高+深)
    let mut max_w = 0i64; // post 的 u = max 页宽
    for (i, page) in pages.iter().enumerate() {
        let pos = w.out.len() as i64;
        let page_counts = counts.get(i).copied().unwrap_or([0i64; 10]);
        w.page(page, prev_bop, &page_counts);
        prev_bop = pos;
        last_bop = pos;
        max_h = max_h.max(page.height + page.depth);
        max_w = max_w.max(page.width);
    }
    w.post(pages.len(), last_bop, max_h, max_w);
    w.out
}

/// 移动栈条目（tex.web `movement_node`）：width/location/info。
#[derive(Debug, Clone, Copy)]
struct MoveEntry {
    width: i64,
    /// 指令字节位置（补丁改写用）。
    loc: usize,
    /// yz_OK=3 / y_OK=4 / z_OK=5 / d_fixed=6 / y_here=1 / z_here=2。
    info: u8,
}

/// DVI 写出器（tex.web `ship_out` 的 Rust 表达）。
struct Writer<'a> {
    out: Vec<u8>,
    fonts: &'a [FontMetrics],
    /// 逻辑坐标与已输出坐标（tex.web `cur_h`/`dvi_h` 等）。
    cur_h: i64,
    cur_v: i64,
    dvi_h: i64,
    dvi_v: i64,
    /// 当前字体（`dvi_f`）与字体使用标记（`font_used`，全局）。
    font: Option<u32>,
    used: Vec<bool>,
    /// push 栈深（`cur_s`，初始 -1）与最大栈深（post 的 s）。
    stack: i16,
    max_stack: i16,
    /// 横向/纵向移动栈（w/x 与 y/z 命中优化）。
    h_moves: Vec<MoveEntry>,
    v_moves: Vec<MoveEntry>,
    /// PDF 变换域的对角缩放栈（`ntex-ctm` 标记驱动；栈底恒 (1,1)）。
    ctm: Vec<(f64, f64)>,
}

impl<'a> Writer<'a> {
    fn new(fonts: &'a [FontMetrics]) -> Self {
        Self {
            out: Vec::new(),
            fonts,
            cur_h: 0,
            cur_v: 0,
            dvi_h: 0,
            dvi_v: 0,
            font: None,
            used: vec![false; fonts.len()],
            stack: -1,
            max_stack: 0,
            h_moves: Vec::new(),
            v_moves: Vec::new(),
            ctm: vec![(1.0, 1.0)],
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
    /// 参考点（tex.web `ship_out` §736-739）：hbox 基线在 `height`、vbox 顶在 0，
    /// 首个 down 由第一个字符/规则的 `synch_v` 惰性发出。
    fn page(&mut self, page: &BoxNode, prev_bop: i64, counts: &[i64; 10]) {
        self.out.push(139); // bop
        for c in counts {
            // \count0..9 全量快照（tex.web ship_out：dvi_out(count(0..9))，不截断）
            self.out.extend((*c as u32).to_be_bytes());
        }
        self.out.extend((prev_bop as u32).to_be_bytes()); // 首页 -1（0xFFFFFFFF）
        self.font = None;
        self.cur_h = 0;
        self.cur_v = 0;
        self.dvi_h = 0;
        self.dvi_v = 0;
        match page.kind {
            BoxKind::HBox => {
                self.cur_v = page.height; // 基线在页高
                self.hlist(page);
            }
            // vbox 页：ship_out 先置 cur_v=height，vlist_out 内 `cur_v -= height` 回到 0
            BoxKind::VBox => {
                self.cur_v = page.height;
                self.vlist(page);
            }
        }
        self.out.push(140); // eop
                            // 跨页清空移动栈（TeX 每页的 down/right 栈独立）
        self.h_moves.clear();
        self.v_moves.clear();
    }

    /// `post`：指针→post_post、num、den、mag、l/u/s/t，随后**使用过的**字体
    /// 定义（TeX 倒序），尾部 `post_post` + 版本 2 + 4×223 哨兵。
    /// 对照真实 TeX / dvipdfmx 实测约定：
    /// - post 的 4 字节指针字段 = 最后一页 bop 位置（dvipdfmx 用其回溯页面）；
    /// - 尾部 = `post_post` 块（249+指针4+版本2+4×223），文件必须恰好以其结尾，
    ///   全文不得出现 nop(0)（dvipdfmx 报 "Unexpected op code: 0"）。
    fn post(&mut self, total_pages: usize, last_bop: i64, max_h: i64, max_w: i64) {
        let post_pos = self.out.len() as u32;
        self.out.push(248);
        self.out.extend((last_bop as u32).to_be_bytes());
        self.out.extend(DVI_NUM.to_be_bytes());
        self.out.extend(DVI_DEN.to_be_bytes());
        self.out.extend(1000u32.to_be_bytes()); // mag
        self.out.extend((max_h as u32).to_be_bytes()); // l
        self.out.extend((max_w as u32).to_be_bytes()); // u
        self.out.extend((self.max_stack as u16).to_be_bytes()); // s
        self.out.extend((total_pages as u16).to_be_bytes()); // t
                                                             // 仅使用过的字体、倒序（tex.web `@<Output the font definitions...@>`）
        for (k, fm) in self.fonts.iter().enumerate().rev() {
            if self.used[k] {
                self.fnt_def(k as u8, fm);
            }
        }
        self.out.push(249);
        self.out.extend(post_pos.to_be_bytes());
        self.out.push(2); // 版本
        self.out.extend([223, 223, 223, 223]); // 尾部哨兵
    }

    /// `fnt_def1`：k(1) c(4) s(4) d(4) a(1) l(1) name。
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

    // ---------- 盒子输出（tex.web hlist_out/vlist_out） ----------

    /// 水平列表：参考点 = 基线（x 向右推进，y 保持基线）。
    fn hlist(&mut self, bx: &BoxNode) {
        self.enter_box();
        let save_loc = self.out.len();
        let base_line = self.cur_v;
        for n in &bx.children {
            match n {
                Node::Char {
                    font,
                    charcode,
                    width,
                    ..
                } => {
                    self.synch_h();
                    self.synch_v();
                    self.select_font(font.0);
                    self.set_char(*charcode);
                    // 驱动按 TFM 宽度自动前进（tex.web §375）
                    self.cur_h += width;
                    self.dvi_h = self.cur_h;
                }
                Node::Ligature {
                    font,
                    charcode,
                    width,
                    ..
                } => {
                    // 连字节点：DVI 输出结果字符（tex.web：ligature 在 hpack 后
                    // 已是字符序列，输出同字符）
                    self.synch_h();
                    self.synch_v();
                    self.select_font(font.0);
                    self.set_char(*charcode);
                    self.cur_h += width;
                    self.dvi_h = self.cur_h;
                }
                Node::Glue { width, .. } | Node::Kern { width } => self.cur_h += width,
                Node::Box(inner) => {
                    if inner.children.is_empty() {
                        self.cur_h += inner.width;
                    } else {
                        let save_h = self.dvi_h;
                        let save_v = self.dvi_v;
                        let edge = self.cur_h;
                        self.cur_v = base_line + inner.shift; // 盒下移 shift
                        match inner.kind {
                            BoxKind::HBox => self.hlist(inner),
                            BoxKind::VBox => self.vlist(inner),
                        }
                        self.dvi_h = save_h;
                        self.dvi_v = save_v;
                        self.cur_h = edge + inner.width;
                        self.cur_v = base_line;
                    }
                }
                Node::Rule {
                    width,
                    height,
                    depth,
                } => {
                    if *height + *depth > 0 && *width > 0 {
                        self.synch_h();
                        self.cur_v = base_line + depth; // 规则底在基线+depth
                        self.synch_v();
                        self.set_rule(*height + *depth, *width);
                        self.cur_v = base_line;
                        self.dvi_h += width; // set_rule 自动前进 h
                    }
                    self.cur_h += width;
                }
                // `\special` whatsit：当前点落 xxx 载荷（tex.web `ship_out` 的
                // dvi_special；零宽，不推进 cur_h）。延迟 `\write` whatsit 不进
                // DVI（tex.web write_out 走写流）。
                Node::Whatsit {
                    text,
                    special: true,
                } => {
                    self.synch_h();
                    self.synch_v();
                    let payload = text.as_bytes();
                    self.special(payload);
                }
                Node::Penalty { .. }
                | Node::Leaders { .. }
                | Node::Discretionary { .. }
                | Node::Ins { .. }
                | Node::Adjust { .. }
                | Node::Whatsit { .. }
                | Node::MathOn { .. }
                | Node::MathOff { .. }
                | Node::Direction { .. }
                | Node::Mark { .. } => {}
            }
        }
        self.prune_movements(save_loc);
        self.leave_box(save_loc);
    }

    /// 垂直列表：参考点 = 页顶（y 向下推进，x 保持左边）。
    fn vlist(&mut self, bx: &BoxNode) {
        self.enter_box();
        let save_loc = self.out.len();
        let left_edge = self.cur_h;
        self.cur_v -= bx.height; // 顶 = 参考点 − height
        for n in &bx.children {
            match n {
                Node::Box(inner) => {
                    if inner.children.is_empty() {
                        self.cur_v += inner.height + inner.depth;
                    } else {
                        self.cur_v += inner.height;
                        self.synch_v();
                        let save_h = self.dvi_h;
                        let save_v = self.dvi_v;
                        self.cur_h = left_edge + inner.shift;
                        match inner.kind {
                            BoxKind::HBox => self.hlist(inner),
                            BoxKind::VBox => self.vlist(inner),
                        }
                        self.dvi_h = save_h;
                        self.dvi_v = save_v;
                        self.cur_v = save_v + inner.depth;
                        self.cur_h = left_edge;
                    }
                }
                Node::Glue { width, .. } | Node::Kern { width } => self.cur_v += width,
                Node::Rule {
                    width,
                    height,
                    depth,
                } => {
                    // tex.web L12598：vlist 里 rule 的 null 宽 = 包含盒宽
                    // （`\hrule` 无 width 说明 → 默认 \hsize）
                    let w = if *width == ntex_layout::NULL_FLAG {
                        bx.width
                    } else {
                        *width
                    };
                    self.cur_v += height + depth; // 移到规则底
                    if *height + *depth > 0 && w > 0 {
                        self.synch_h();
                        self.synch_v();
                        self.put_rule(*height + *depth, w);
                    }
                }
                // 垂直列表里的 `\special`：当前点（下一盒参考点将落处）发 xxx。
                Node::Whatsit {
                    text,
                    special: true,
                } => {
                    self.synch_h();
                    self.synch_v();
                    let payload = text.as_bytes();
                    self.special(payload);
                }
                Node::Penalty { .. }
                | Node::Leaders { .. }
                | Node::Char { .. }
                | Node::Ligature { .. }
                | Node::Discretionary { .. }
                | Node::Ins { .. }
                | Node::Adjust { .. }
                | Node::Whatsit { .. }
                | Node::MathOn { .. }
                | Node::MathOff { .. }
                | Node::Direction { .. }
                | Node::Mark { .. } => {}
            }
        }
        self.prune_movements(save_loc);
        self.leave_box(save_loc);
    }

    /// 进入盒子（tex.web §351-353）：栈深 +1，非顶层发 push。
    fn enter_box(&mut self) {
        self.stack += 1;
        if self.stack > 0 {
            self.out.push(141); // push
        }
        self.max_stack = self.max_stack.max(self.stack);
    }

    /// 离开盒子（tex.web §357-371）：prune 已在调用方做；空盒抵消 push/pop。
    fn leave_box(&mut self, save_loc: usize) {
        if self.stack > 0 {
            if self.out.len() == save_loc {
                self.out.pop(); // 抵消 `push pop` 对（tex.web dvi_pop §371）
            } else {
                self.out.push(142); // pop
            }
        }
        self.stack -= 1;
    }

    // ---------- 指令字节 ----------

    /// `set_char_*` / `set1..set4`（DVI 规范 §2.6.2）：按码位宽度取最短编码。
    ///
    /// 8-bit 字体只用 `set_char`/`set1`，与真实 TeX 一致；Unicode 字体
    /// （M9 中文刀 1，码位可达 0x10FFFF）用 `set2`/`set3`——此前一律
    /// `ch as u8`，会把 `\char"4E00` 静默截断成 `\char"00`。
    fn set_char(&mut self, ch: u32) {
        if ch < 128 {
            self.out.push(ch as u8); // set_char_0..127
        } else if ch < 0x100 {
            self.out.push(128); // set1（1 字节操作数）
            self.out.push(ch as u8);
        } else if ch < 0x1_0000 {
            self.out.push(129); // set2（2 字节操作数）
            self.out.extend_from_slice(&(ch as u16).to_be_bytes());
        } else if ch < 0x100_0000 {
            self.out.push(130); // set3（3 字节操作数）
            let b = ch.to_be_bytes();
            self.out.extend_from_slice(&b[1..4]);
        } else {
            self.out.push(131); // set4（4 字节操作数）
            self.out.extend_from_slice(&ch.to_be_bytes());
        }
    }

    /// `set_rule`：当前点 = 矩形**左下角**，向上画 h、向右画 w（dvitype.web §456）。
    fn set_rule(&mut self, h: i64, w: i64) {
        self.out.push(132);
        self.out.extend((h as u32).to_be_bytes());
        self.out.extend((w as u32).to_be_bytes());
    }

    /// `put_rule`：同 set_rule 但不前进。
    fn put_rule(&mut self, h: i64, w: i64) {
        self.out.push(137);
        self.out.extend((h as u32).to_be_bytes());
        self.out.extend((w as u32).to_be_bytes());
    }

    /// 字体选择（tex.web §383-391）：首次使用时惰性 `fnt_def`。
    /// `xxx4`（tex.web `dvi_special`）+ NTex 图片传输协议：
    ///
    /// - `ntex-ctm push|matrix a b c d|pop`：`\pdfsave/\pdfsetmatrix/
    ///   \pdfrestore` 落下的变换标记——在此（**重放侧**）维护对角缩放栈，
    ///   不落 DVI（graphicx 的 \Gscale@box 内容盒先排、矩阵域后开，引擎侧
    ///   \pdfrefximage 取不到缩放，只能在盒节点重放时结算）；
    /// - `ntex-image <w> <h> <名长> <名>`：图片引用——尺寸就地乘当前缩放
    ///   （scale=0.6 → 0.6×自然尺寸），写成 xxx 载荷给 ntex-pdf；
    /// - 其余载荷：原样透传为 xxx（\special 原义）。
    fn special(&mut self, payload: &[u8]) {
        let text = std::str::from_utf8(payload).ok();
        let mut words = text.unwrap_or("").split_whitespace().map(|s| s.to_owned());
        match words.next().as_deref() {
            Some("ntex-ctm") => {
                let (sx, sy) = *self.ctm.last().unwrap_or(&(1.0, 1.0));
                match words.next().as_deref() {
                    Some("push") => self.ctm.push((sx, sy)),
                    Some("matrix") => {
                        // 只取对角分量（a→sx、d→sy）；旋转矩阵退化为 cos 缩放
                        let m: Vec<f64> = words
                            .by_ref()
                            .take(4)
                            .filter_map(|s| s.parse().ok())
                            .collect();
                        let top = self.ctm.last_mut().expect("缩放栈底恒在");
                        top.0 *= m.first().copied().unwrap_or(1.0);
                        top.1 *= m.get(3).copied().unwrap_or(1.0);
                    }
                    Some("pop") => {
                        if self.ctm.len() > 1 {
                            self.ctm.pop();
                        } else {
                            self.ctm[0] = (1.0, 1.0); // 失衡 \pdfrestore 保底
                        }
                    }
                    _ => {}
                }
                return; // 标记只作用于写出器状态，不落 DVI
            }
            Some("ntex-image") => {
                // `ntex-image <w_sp> <h_sp> <名长> <名>`：前两个数乘缩放
                let vals: Vec<i64> = words
                    .by_ref()
                    .take(2)
                    .filter_map(|s| s.parse().ok())
                    .collect();
                let rest: Vec<String> = words.collect();
                if vals.len() == 2 && rest.len() >= 2 {
                    let (sx, sy) = *self.ctm.last().unwrap_or(&(1.0, 1.0));
                    let out = format!(
                        "ntex-image {} {} {} {}",
                        (vals[0] as f64 * sx).round() as i64,
                        (vals[1] as f64 * sy).round() as i64,
                        rest[0],
                        rest[1..].join(" ")
                    );
                    self.xxx4(out.as_bytes());
                    return;
                }
            }
            _ => {}
        }
        self.xxx4(payload);
    }

    /// xxx4 原样发出（4 字节长度域；真实 TeX 按长度分档 1..4，此处非目标）。
    fn xxx4(&mut self, payload: &[u8]) {
        self.out.push(242); // xxx4
        self.out
            .extend_from_slice(&(payload.len() as u32).to_be_bytes());
        self.out.extend_from_slice(payload);
    }

    fn select_font(&mut self, f: u32) {
        if self.font == Some(f) {
            return;
        }
        // 字体表外（如 fn 指针占位模式）只发 fnt_num，不发 fnt_def
        if let Some(fm) = self.fonts.get(f as usize) {
            if !self.used[f as usize] {
                self.fnt_def(f as u8, fm);
                self.used[f as usize] = true;
            }
        }
        if f < 64 {
            self.out.push(171 + f as u8); // fnt_num_0..63
        } else {
            self.out.push(235); // fnt1
            self.out.push(f as u8);
        }
        self.font = Some(f);
    }

    /// `synch_h` / `synch_v`（tex.web §276-280）：把累积位移以 movement 发出。
    fn synch_h(&mut self) {
        if self.cur_h != self.dvi_h {
            let d = self.cur_h - self.dvi_h;
            self.movement(d, false);
            self.dvi_h = self.cur_h;
        }
    }

    fn synch_v(&mut self) {
        if self.cur_v != self.dvi_v {
            let d = self.cur_v - self.dvi_v;
            self.movement(d, true);
            self.dvi_v = self.cur_v;
        }
    }

    /// `movement`（tex.web §297-366）：带 w/x/y/z 命中优化的位移输出。
    ///
    /// `o` = down1/right1；栈搜索命中时把先前 down/right 改写成 y/w（+5）
    /// 或 z/x（+10）并发出 y0/w0、z0/x0 一字节指令；否则发普通 down/right。
    fn movement(&mut self, w: i64, vert: bool) {
        let o: u8 = if vert { 157 } else { 143 };
        let stack: &mut Vec<MoveEntry> = if vert {
            &mut self.v_moves
        } else {
            &mut self.h_moves
        };
        // 搜索（tex.web §360-366）：mstate 0=无、6=y_seen、12=z_seen
        let mut mstate = 0u8;
        let mut found: Option<usize> = None;
        let mut p = stack.len();
        while p > 0 {
            p -= 1;
            let (width, loc, info) = {
                let e = &stack[p];
                (e.width, e.loc, e.info)
            };
            if width == w {
                let v = mstate + info;
                match v {
                    // none+yz_OK, none+y_OK, z_seen+yz_OK, z_seen+y_OK → 改 y/w
                    3 | 4 | 15 | 16 => {
                        self.out[loc] += 5; // y1-down1（w1-right1 同 5）
                        stack[p].info = 1; // y_here
                        found = Some(p);
                        break;
                    }
                    // none+z_OK, y_seen+yz_OK, y_seen+z_OK → 改 z/x
                    5 | 9 | 11 => {
                        self.out[loc] += 10; // z1-down1（x1-right1 同 10）
                        stack[p].info = 2; // z_here
                        found = Some(p);
                        break;
                    }
                    // none+y_here, none+z_here, y_seen+z_here, z_seen+y_here → 命中
                    1 | 2 | 8 | 13 => {
                        found = Some(p);
                        break;
                    }
                    _ => {} // 其余信息不做处理
                }
            } else {
                match mstate + info {
                    1 => mstate = 6,  // none+y_here → y_seen
                    2 => mstate = 12, // none+z_here → z_seen
                    8 | 13 => break,  // y_seen+z_here / z_seen+y_here → not_found
                    _ => {}
                }
            }
        }
        let q_loc = self.out.len();
        let q_info = match found {
            Some(p) => {
                let p_info = stack[p].info;
                if p_info == 1 {
                    self.out.push(o + 4); // y0 / w0（y0-down1 = w0-right1 = 4）
                    for k in ((p + 1)..stack.len()).rev() {
                        match stack[k].info {
                            3 => stack[k].info = 5, // yz_OK → z_OK
                            4 => stack[k].info = 6, // y_OK → d_fixed
                            _ => {}
                        }
                    }
                } else {
                    self.out.push(o + 9); // z0 / x0（z0-down1 = x0-right1 = 9）
                    for k in ((p + 1)..stack.len()).rev() {
                        match stack[k].info {
                            3 => stack[k].info = 4, // yz_OK → y_OK
                            5 => stack[k].info = 6, // z_OK → d_fixed
                            _ => {}
                        }
                    }
                }
                p_info
            }
            None => {
                emit_move(&mut self.out, o, w);
                3 // yz_OK
            }
        };
        stack.push(MoveEntry {
            width: w,
            loc: q_loc,
            info: q_info,
        });
    }

    /// `prune_movements`（tex.web §439-451）：删除本盒内产生的移动栈条目。
    fn prune_movements(&mut self, l: usize) {
        self.h_moves.retain(|e| e.loc < l);
        self.v_moves.retain(|e| e.loc < l);
    }
}

/// 普通 down/right 编码（tex.web §355-359）：|w| 分档 1/2/3/4 字节。
fn emit_move(out: &mut Vec<u8>, o: u8, w: i64) {
    let abs = w.abs();
    if abs >= (1 << 23) {
        out.push(o + 3); // down4/right4
        out.extend((w as i32).to_be_bytes());
    } else if abs >= (1 << 15) {
        out.push(o + 2); // down3/right3
        out.extend(((w as u32) & 0xFF_FFFF).to_be_bytes()[1..].to_vec());
    } else if abs >= (1 << 7) {
        out.push(o + 1); // down2/right2
        out.extend((w as u16).to_be_bytes());
    } else {
        out.push(o); // down1/right1
        out.push((w as i8) as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ntex_font::FontMetrics;
    use ntex_layout::node::{BoxNode, FontId, Node};
    use ntex_layout::typeset::Typesetter;

    /// panic 审计守护（债务表项 4）：生产路径（`#[cfg(test)]` 之前的源码）
    /// 禁 `.unwrap()`——TeX 引擎的传统是永不 panic，损坏/恶意输入走错误恢复。
    /// 不可达位允许 `.expect("不变量说明")`（ctm 缩放栈底恒在，见 special 处理）。
    #[test]
    fn production_code_has_no_unwrap() {
        let files = [
            concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"),
            concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"),
        ];
        for file in files {
            let src = std::fs::read_to_string(file).expect("源文件应可读");
            let prod = src.split("#[cfg(test)]").next().expect("应有测试分界");
            for (i, line) in prod.lines().enumerate() {
                assert!(
                    !line.contains(".unwrap()"),
                    "生产代码出现 .unwrap()（{} 第 {} 行）：{line}",
                    file.rsplit('/').next().unwrap_or(file),
                    i + 1
                );
            }
        }
    }

    fn cmr_metrics(name: &str) -> FontMetrics {
        FontMetrics {
            hyphenchar: 45,
            unicode_native: false,
            unicode_chars: Vec::new(),
            design_size_sp: 10 * 65_536,
            scale: 1 << 20,
            checksum: 0x1234_5678,
            name: name.to_owned(),
            chars: vec![None; 128],
            char_italic: vec![0; 128],
            slant: 0,
            space: 218_453,
            space_stretch: 109_227,
            space_shrink: 72_818,
            x_height: 430_965,
            quad: 655_360,
            extra_space: 109_227,
            lig_kern_steps: Vec::new(),
            kern_values: Vec::new(),
            lig_kern_index: Vec::new(),
            next_larger: Vec::new(),
            font_params: Vec::new(),
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
        assert!(dvi.contains(&248));
        assert!(dvi.contains(&249));
    }

    #[test]
    fn writes_font_def_and_chars() {
        let fonts = vec![cmr_metrics("cmr10")];
        let page = hbox_page(vec![char_node(b'H', 500_000), char_node(b'i', 260_000)]);
        let dvi = write_dvi(&[page], &fonts);
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
        // fnt_num_0 = 171 + 0；H、i 相邻（set_char 后无 right，驱动按 TFM 前进）
        assert!(dvi.contains(&171), "fnt_num(0) 应出现");
        assert!(
            dvi.windows(3).any(|w| w == [171, 72, 105]),
            "H、i 应相邻：{dvi:?}"
        );
    }

    #[test]
    fn glue_and_rules_advance() {
        let fonts = vec![cmr_metrics("cmr10")];
        let page = hbox_page(vec![
            char_node(b'a', 100),
            Node::Glue {
                name: None,
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
        // 规则（tex.web §421-429）：down(+depth) → set_rule(h+d, w) → down(-depth)，
        // 无 push/pop（set_rule 的 h 自动前进 w）。测试数据为 sp：h=40+5=45、w=30。
        assert!(dvi.contains(&132), "set_rule 应出现");
        assert!(dvi.windows(9).any(|w| w == [132, 0, 0, 0, 45, 0, 0, 0, 30]));
    }

    #[test]
    fn multiple_pages_get_bop_chain() {
        let fonts = vec![cmr_metrics("cmr10")];
        let p1 = hbox_page(vec![char_node(b'a', 100)]);
        let p2 = hbox_page(vec![char_node(b'b', 100)]);
        let dvi = write_dvi(&[p1, p2], &fonts);
        assert_eq!(dvi.iter().filter(|&&b| b == 139).count(), 2);
        assert_eq!(dvi.iter().filter(|&&b| b == 140).count(), 2);
        // post 的 t = 2：248(1) + pp(4) + num/den/mag(12) + l/u(8) + s(2)，t 在 +27
        let t_pos = dvi.iter().position(|&b| b == 248).unwrap() + 1 + 4 + 4 + 4 + 4 + 4 + 4 + 2;
        assert_eq!(&dvi[t_pos..t_pos + 2], &[0, 2]);
    }

    // ---------- 与排版器集成（真实 cmr10 + 自动分页） ----------

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
        // nullfont 占 id 0（9376715），字体表 = nullfont + cmr10
        assert_eq!(fonts.len(), 2);
        let cmr = fonts
            .iter()
            .find(|f| f.name == "cmr10")
            .expect("字体表应含 cmr10");
        assert_eq!(cmr.name, "cmr10");
        assert_eq!(cmr.checksum, fm.checksum, "checksum 应来自 TFM 头");

        let dvi = write_dvi(&pages, &fonts);
        for ch in *b"NTex" {
            assert!(dvi.contains(&ch), "缺字符 {ch} 的 set_char");
        }
        // 字体名出现两次：页面 fnt_def + post 字体列表
        let name: Vec<u8> = b"cmr10".to_vec();
        assert_eq!(
            dvi.windows(name.len())
                .filter(|w| *w == name.as_slice())
                .count(),
            2,
            "cmr10 应在页面与 post 各定义一次"
        );
    }

    #[test]
    fn automatic_pagination_ships_pages() {
        let Some(path) = ntex_font::find_tfm("cmr10") else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let bytes = std::fs::read(path).expect("读取 cmr10.tfm");
        let fm = ntex_font::parse_tfm(&bytes).expect("解析 cmr10.tfm");

        let mut ts = Typesetter::with_tfm();
        // 小 \vsize + 窄 \hsize：长文本自动分多页（tolerance=200 折行紧凑，vsize 取 40pt 保证多行必然分页）
        let src = r"\font\cmr=cmr10\hsize 200pt\vsize 40pt\cmr ".to_owned()
            + "This is the first paragraph of a test document that has to be "
            + "long enough to wrap around into several lines of text, so that "
            + "the page builder will eventually overflow the rather small "
            + "vertical size and fire up a page break at the best place. "
            + "More words are needed to make the paragraph really long, and "
            + "even more words are needed to push it past the small vertical "
            + "size so that the page builder must fire up at least twice. "
            + "\\par\\cmr "
            + "And here is the second paragraph to be paginated separately. "
            + "It also has some words to fill up a few more lines, since the "
            + "pagination logic must handle the remainder of the first page, "
            + "and then some additional trailing words to make sure that the "
            + "second page actually gets material as well.";
        let (pages, fonts) = ts.typeset_dvi(&src).expect("排版失败");
        assert!(
            pages.len() >= 2,
            "小 \\vsize 应至少分两页，实际 {} 页",
            pages.len()
        );
        // nullfont 占 id 0（9376715），字体表 = nullfont + cmr10
        assert_eq!(fonts.len(), 2);
        let cmr = fonts
            .iter()
            .find(|f| f.name == "cmr10")
            .expect("字体表应含 cmr10");
        assert_eq!(cmr.checksum, fm.checksum);
        // 每页为 vbox（自动分页），高度 = vsize（40pt）
        for p in &pages {
            assert_eq!(p.kind, BoxKind::VBox, "自动分页页面应为 vbox");
            assert_eq!(p.height, 40 * 65_536, "页高应为 \\vsize");
        }
        let dvi = write_dvi(&pages, &fonts);
        assert!(!dvi.is_empty());
    }

    // ---------- 输出例程刀 5：bop 的 10 计数字（\count0..9 全量写入） ----------

    /// bop 起始 44 字节：139 + 10×4 计数 + 前页指针。
    fn bop_counters(dvi: &[u8]) -> Vec<i64> {
        let at = dvi.iter().position(|&b| b == 139).expect("应含 bop");
        (0..10)
            .map(|k| {
                let b = &dvi[at + 1 + 4 * k..at + 5 + 4 * k];
                i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as i64
            })
            .collect()
    }

    #[test]
    fn bop_counters_carry_shipout_counts() {
        // tex.web ship_out：`for k:=0 to 15? `——bop 写 count(0..9) **全量**，
        // 不做尾零截断（截断只发生在 log/终端的 `[...]` 页标签）。
        let mut counts = [0i64; 10];
        counts[0] = 5;
        counts[1] = 7;
        let page = hbox_page(vec![char_node(b'H', 500_000)]);
        let dvi = write_dvi_with_counts(std::slice::from_ref(&page), &[counts], &[]);
        assert_eq!(
            bop_counters(&dvi),
            (0..10)
                .map(|k| if k == 0 {
                    5
                } else if k == 1 {
                    7
                } else {
                    0
                })
                .collect::<Vec<_>>(),
            "bop 应写 \\count0=5 \\count1=7，其余 0"
        );
        // 负值照写（TRIP：\count0=-5000 → 4 字节二进制补码）
        let mut neg = [0i64; 10];
        neg[0] = -5000;
        let dvi = write_dvi_with_counts(std::slice::from_ref(&page), &[neg], &[]);
        assert_eq!(bop_counters(&dvi)[0], -5000, "负 count 照写");
    }

    #[test]
    fn write_dvi_falls_back_to_sequential_page_numbers() {
        // 无 counts 快照的旧入口：count0 = 页序号（plain \advancepageno 语义），
        // count1..9 = 0——修正此前每页恒写 1 的多页偏差。
        let pages = vec![
            hbox_page(vec![char_node(b'H', 500_000)]),
            hbox_page(vec![char_node(b'i', 260_000)]),
        ];
        let dvi = write_dvi(&pages, &[]);
        assert_eq!(bop_counters(&dvi)[0], 1, "首页 count0 = 1");
        let second = {
            let at = dvi.iter().rposition(|&b| b == 139).expect("第二页 bop");
            let b = &dvi[at + 1..at + 5];
            i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as i64
        };
        assert_eq!(second, 2, "次页 count0 = 2（此前恒 1）");
        // 快照缺失时按 0 补齐（不 panic）
        let dvi = write_dvi_with_counts(&pages, &[], &[]);
        assert_eq!(bop_counters(&dvi), vec![0; 10]);
    }
}
