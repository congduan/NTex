//! DVI 解析器（M8 输出后端）：字节流 → 每页的绘制指令列表。
//!
//! 坐标单位：DVI 的 scaled point（1pt = 65536sp；mag=1000 时 1 单位 = 1sp）。
//! h 向右、v 向下（原点在页左上）。PDF 转换在写出层做（y 翻转 + 除 65536）。
//!
//! 支持指令：set_char/set1-4、set_rule、put_rule、push/pop、
//! right/down（含 w/x/y/z 寄存器）、fnt_num/fnt_def、xxx（忽略）、
//! bop/eop/pre/post/post_post。

use std::collections::HashMap;
use std::io::{self, ErrorKind};

use ntex_font::{parse_tfm, FontMetrics};

/// 一页的绘制指令（DVI 坐标，sp）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawOp {
    /// 字符：`font` 选择器（[`Page::fonts`] 下标）、字符码、参考点 (h, v)。
    Char { font: u32, code: u8, h: i64, v: i64 },
    /// 规则：覆盖 x ∈ [h, h+width]、y ∈ [v-height, v]（DVI 坐标）。
    Rule {
        h: i64,
        v: i64,
        width: i64,
        height: i64,
    },
}

/// 一页的绘制内容。
#[derive(Debug)]
pub struct Page {
    pub ops: Vec<DrawOp>,
}

/// 解析出的 DVI 文档。
#[derive(Debug)]
pub struct Dvi {
    /// 页面（顺序）。
    pub pages: Vec<Page>,
    /// 全部字体（下标即 [`DrawOp::Char`] 的 font 字段）。
    pub fonts: Vec<FontMetrics>,
    /// 字体名（`fnt_def` 的 name；与 `fonts` 一一对应）。
    pub font_names: Vec<String>,
}

/// 读一个 4 字节大端有符号整数。
fn read_i32(b: &[u8], i: &mut usize) -> io::Result<i64> {
    if *i + 4 > b.len() {
        return Err(io::Error::new(ErrorKind::UnexpectedEof, "DVI 截断"));
    }
    let v = i32::from_be_bytes([b[*i], b[*i + 1], b[*i + 2], b[*i + 3]]) as i64;
    *i += 4;
    Ok(v)
}

/// 读 `n` 字节无符号整数（1..=4）。
fn read_uint(b: &[u8], i: &mut usize, n: usize) -> io::Result<u32> {
    if *i + n > b.len() {
        return Err(io::Error::new(ErrorKind::UnexpectedEof, "DVI 截断"));
    }
    let mut v = 0u32;
    for k in 0..n {
        v = (v << 8) | b[*i + k] as u32;
    }
    *i += n;
    Ok(v)
}

/// 读 `n` 字节大端有符号整数（1..=4；最高字节的最高位为符号位）。
///
/// 3/4 字节形式必须按位宽符号扩展：DVI 运动量（right/down/w/x/y/z）大量使用
/// 负的 `*3`/`*4` 编码（数学上下标回退、行间回跳），若按无符号读会把
/// "上移 13pt"（0xF30000 = -851968sp）当成 "+15925248sp ≈ 243pt"，
/// 坐标整体飞出页面（demo1 数学区即被推出页宽外）。
fn read_signed(b: &[u8], i: &mut usize, n: usize) -> io::Result<i64> {
    let v = read_uint(b, i, n)? as i64;
    Ok(match n {
        1 => (v as u8 as i8) as i64,
        2 => (v as u16 as i16) as i64,
        3 => {
            if v & 0x0080_0000 != 0 {
                v - 0x0100_0000
            } else {
                v
            }
        }
        _ => (v as u32 as i32) as i64,
    })
}

/// 解析 DVI 字节流。
pub fn parse(bytes: &[u8]) -> io::Result<Dvi> {
    let p = Parser {
        b: bytes,
        i: 0,
        fonts: Vec::new(),
        font_names: Vec::new(),
        font_ids: HashMap::new(),
    };
    p.run()
}

/// DVI 移动寄存器（w/x/y/z）。
#[derive(Debug, Clone, Copy, Default)]
struct Moves {
    w: i64,
    x: i64,
    y: i64,
    z: i64,
}

/// 移动栈条目（push/pop 保存全部状态）。
#[derive(Debug, Clone, Copy)]
struct StackEntry {
    h: i64,
    v: i64,
    moves: Moves,
    font: u32,
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    fonts: Vec<FontMetrics>,
    font_names: Vec<String>,
    /// DVI 字体号 k → [`Parser::fonts`] 下标（`fnt_def` 登记、`fnt_num` 查询）。
    font_ids: HashMap<u32, u32>,
}

impl<'a> Parser<'a> {
    /// 主循环：pre → 页循环 → post/post_post（忽略）。
    fn run(mut self) -> io::Result<Dvi> {
        if self.b.first() != Some(&247) {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "不是 DVI 文件（缺 pre 指令）",
            ));
        }
        // pre：i(1) num(4) den(4) mag(4) 注释长度(1) 注释
        self.i += 2; // 跳过 opcode 与格式版本 i
        let _num = read_i32(self.b, &mut self.i)?;
        let _den = read_i32(self.b, &mut self.i)?;
        let _mag = read_i32(self.b, &mut self.i)?;
        let clen = self.b[self.i] as usize;
        self.i += 1 + clen;

        let mut pages = Vec::new();
        loop {
            let op = *self
                .b
                .get(self.i)
                .ok_or_else(|| io::Error::new(ErrorKind::UnexpectedEof, "DVI 截断"))?;
            match op {
                139 => pages.push(self.page()?),
                248 => break, // post
                other => {
                    return Err(io::Error::new(
                        ErrorKind::InvalidData,
                        format!("指令流中意外 opcode {other}"),
                    ));
                }
            }
        }
        Ok(Dvi {
            pages,
            fonts: self.fonts,
            font_names: self.font_names,
        })
    }

    /// 一页：bop（10 计数 + 前一指针）→ 指令 → eop。
    fn page(&mut self) -> io::Result<Page> {
        self.i += 1; // bop
        for _ in 0..11 {
            read_i32(self.b, &mut self.i)?;
        }
        let mut ops = Vec::new();
        let mut h = 0i64;
        let mut v = 0i64;
        let mut moves = Moves::default();
        let mut font = 0u32;
        let mut stack: Vec<StackEntry> = Vec::new();
        loop {
            let op = *self
                .b
                .get(self.i)
                .ok_or_else(|| io::Error::new(ErrorKind::UnexpectedEof, "页面截断"))?;
            self.i += 1;
            match op {
                140 => break, // eop
                138 => {}     // nop
                141 => stack.push(StackEntry { h, v, moves, font }),
                142 => {
                    let e = stack
                        .pop()
                        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "pop 栈下溢"))?;
                    h = e.h;
                    v = e.v;
                    moves = e.moves;
                    font = e.font;
                }
                // set_char_0..127
                0..=127 => {
                    ops.push(DrawOp::Char {
                        font,
                        code: op,
                        h,
                        v,
                    });
                    h += self.char_width(font, op)?;
                }
                // set1..set4
                128..=131 => {
                    let n = (op - 127) as usize;
                    let code = read_uint(self.b, &mut self.i, n)? as u8;
                    ops.push(DrawOp::Char { font, code, h, v });
                    h += self.char_width(font, code)?;
                }
                // put1..put4（画字符但不推进）
                133..=136 => {
                    let n = (op - 132) as usize;
                    let code = read_uint(self.b, &mut self.i, n)? as u8;
                    ops.push(DrawOp::Char { font, code, h, v });
                }
                132 => {
                    // set_rule a b：a=height（垂直尺寸）、b=width（水平尺寸），
                    // 规则左上角在 (h,v)，画出后 h ← h + b。曾把 a/b 互换读，
                    // 导致 h 按 height 推进（坐标漂移）且 PDF 里规则纵横颠倒。
                    let height = read_i32(self.b, &mut self.i)?;
                    let width = read_i32(self.b, &mut self.i)?;
                    ops.push(DrawOp::Rule {
                        h,
                        v,
                        width,
                        height,
                    });
                    h += width;
                }
                137 => {
                    // put_rule a b：同 set_rule，但不推进 h
                    let height = read_i32(self.b, &mut self.i)?;
                    let width = read_i32(self.b, &mut self.i)?;
                    ops.push(DrawOp::Rule {
                        h,
                        v,
                        width,
                        height,
                    });
                }
                // right1..4
                143..=146 => {
                    let n = (op - 142) as usize;
                    let delta = read_signed(self.b, &mut self.i, n)?;
                    h += delta;
                }
                // w0
                147 => h += moves.w,
                // w1..w4
                148..=151 => {
                    let n = (op - 147) as usize;
                    let delta = read_signed(self.b, &mut self.i, n)?;
                    moves.w = delta;
                    h += moves.w;
                }
                // x0
                152 => h += moves.x,
                // x1..x4
                153..=156 => {
                    let n = (op - 152) as usize;
                    let delta = read_signed(self.b, &mut self.i, n)?;
                    moves.x = delta;
                    h += moves.x;
                }
                // down1..4
                157..=160 => {
                    let n = (op - 156) as usize;
                    let delta = read_signed(self.b, &mut self.i, n)?;
                    v += delta;
                }
                // y0
                161 => v += moves.y,
                // y1..y4
                162..=165 => {
                    let n = (op - 161) as usize;
                    let delta = read_signed(self.b, &mut self.i, n)?;
                    moves.y = delta;
                    v += moves.y;
                }
                // z0
                166 => v += moves.z,
                // z1..z4
                167..=170 => {
                    let n = (op - 166) as usize;
                    let delta = read_signed(self.b, &mut self.i, n)?;
                    moves.z = delta;
                    v += moves.z;
                }
                // fnt_num_0..63
                171..=234 => font = self.font_id((op - 171) as u32)?,
                // fnt1..fnt4
                235..=238 => {
                    let n = (op - 234) as usize;
                    let k = read_uint(self.b, &mut self.i, n)?;
                    font = self.font_id(k)?;
                }
                // xxx1..4：跳过
                239..=242 => {
                    let n = (op - 238) as usize;
                    let len = read_uint(self.b, &mut self.i, n)? as usize;
                    self.i += len;
                }
                // fnt_def1..4
                243..=246 => {
                    let n = (op - 242) as usize;
                    self.fnt_def(n, &mut font)?;
                }
                other => {
                    return Err(io::Error::new(
                        ErrorKind::InvalidData,
                        format!("页面指令流中意外 opcode {other}"),
                    ));
                }
            }
        }
        Ok(Page { ops })
    }

    /// `fnt_def n`：k(1) c(4) s(4) d(4) a(1) l(1) name → 登记字体号 k 并选为当前字体。
    fn fnt_def(&mut self, n: usize, font: &mut u32) -> io::Result<()> {
        let k = read_uint(self.b, &mut self.i, n)?;
        let _checksum = read_i32(self.b, &mut self.i)?;
        let scale = read_i32(self.b, &mut self.i)?; // s：实际字号 sp
        let design = read_i32(self.b, &mut self.i)?; // d：设计字号 sp
        let _a = self.b[self.i] as usize;
        self.i += 1;
        let l = self.b[self.i] as usize;
        self.i += 1;
        let name = String::from_utf8_lossy(&self.b[self.i..self.i + l]).into_owned();
        self.i += l;

        // DVI 规范：k 是本定义的字体号，`fnt_num k` 按它选择——以 k 为键归位，
        // 不按注册顺序（k 可跳号、可重复定义同名；引擎 0 号字体未被使用时首个
        // 发出的 fnt_def 即 k≠0）。每个 k 一条度量（同名不同字号是不同字体），
        // 同名字体的 PDF 对象去重由写出端按名完成。
        let id = match self.font_ids.get(&k) {
            Some(&id) => id,
            None => {
                let fm = load_tfm(&name, scale, design)?;
                let id = u32::try_from(self.fonts.len())
                    .map_err(|_| io::Error::new(ErrorKind::InvalidData, "字体过多"))?;
                self.font_names.push(name);
                self.fonts.push(fm);
                self.font_ids.insert(k, id);
                id
            }
        };
        *font = id;
        Ok(())
    }

    /// DVI 字体号 k → 字体表下标（未定义即报错）。
    fn font_id(&self, k: u32) -> io::Result<u32> {
        self.font_ids
            .get(&k)
            .copied()
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, format!("未定义字体号 {k}")))
    }

    /// 字符宽度（sp）：按当前缩放后的 TFM 度量。
    fn char_width(&self, font: u32, code: u8) -> io::Result<i64> {
        let fm = self
            .fonts
            .get(font as usize)
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "未定义字体"))?;
        Ok(fm.char_metrics(code as u32).0)
    }
}

/// 加载 TFM 并按 DVI 的 (scale, design) 缩放到实际字号。
///
/// 字节经 [`ntex_font::read_tfm`]：注册表优先（wasm 唯一来源，见
/// `ntex-font` 注册表注释），native 回落 find_tfm + 文件读，行为不变。
fn load_tfm(name: &str, scale: i64, design: i64) -> io::Result<FontMetrics> {
    let bytes = ntex_font::read_tfm(name)?;
    let mut fm = parse_tfm(&bytes)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("解析 {name}.tfm：{e}")))?;
    fm.name = name.to_owned();
    // scale = 实际字号 sp，design = 设计字号 sp：维度 × scale/design
    Ok(if design > 0 && scale != design {
        fm.scaled_by(scale, design)
    } else {
        fm
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造最小 DVI：pre + 一页（set_char 'a'、right、set_rule、eop）+ post。
    fn sample_dvi() -> Vec<u8> {
        let mut d = Vec::new();
        d.push(247); // pre
        d.push(2);
        d.extend(254_000_000i32.to_be_bytes()); // num
        d.extend(473_628_672i32.to_be_bytes()); // den
        d.extend(1000i32.to_be_bytes()); // mag
        d.push(3);
        d.extend(b"ntx");
        d.push(139); // bop
        for _ in 0..11 {
            d.extend(0i32.to_be_bytes());
        }
        // fnt_def1 k=0 c=0 s=655360(10pt) d=655360 name="cmr10"
        d.push(243);
        d.push(0);
        d.extend(0i32.to_be_bytes());
        d.extend(655_360i32.to_be_bytes());
        d.extend(655_360i32.to_be_bytes());
        d.push(0);
        d.push(5);
        d.extend(b"cmr10");
        d.push(171); // fnt_num_0
        d.push(b'a'); // set_char 'a'
        d.push(132); // set_rule 100 200
        d.extend(100i32.to_be_bytes());
        d.extend(200i32.to_be_bytes());
        d.push(140); // eop
        d.push(248); // post
        d.push(0); // post 后续略（解析器不读）
        d
    }

    /// 追加 `fnt_def1`：k、名字，scale/design 均 10pt。
    fn push_fnt_def(d: &mut Vec<u8>, k: u8, name: &[u8]) {
        d.push(243);
        d.push(k);
        d.extend(0i32.to_be_bytes()); // c
        d.extend(655_360i32.to_be_bytes()); // s
        d.extend(655_360i32.to_be_bytes()); // d
        d.push(0);
        d.push(name.len() as u8);
        d.extend_from_slice(name);
    }

    /// 追加一页骨架：bop（11 计数）→ 指令 → eop。
    fn push_page(d: &mut Vec<u8>, body: &[u8]) {
        d.push(139); // bop
        for _ in 0..11 {
            d.extend(0i32.to_be_bytes());
        }
        d.extend_from_slice(body);
        d.push(140); // eop
    }

    /// 最小 pre 头（含注释 "ntx"）。
    fn pre_header() -> Vec<u8> {
        let mut d = Vec::new();
        d.push(247); // pre
        d.push(2);
        d.extend(254_000_000i32.to_be_bytes()); // num
        d.extend(473_628_672i32.to_be_bytes()); // den
        d.extend(1000i32.to_be_bytes()); // mag
        d.push(3);
        d.extend(b"ntx");
        d
    }

    #[test]
    fn parses_single_page() {
        let Some(_) = ntex_font::find_tfm("cmr10") else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let sample = sample_dvi();
        let dvi = parse(&sample).unwrap();
        assert_eq!(dvi.pages.len(), 1);
        assert_eq!(dvi.font_names, vec!["cmr10".to_owned()]);
        let ops = &dvi.pages[0].ops;
        assert_eq!(ops.len(), 2, "{ops:?}");
        match &ops[0] {
            DrawOp::Char { font, code, h, v } => {
                assert_eq!(*font, 0);
                assert_eq!(*code, b'a');
                assert_eq!(*h, 0);
                assert_eq!(*v, 0);
            }
            other => panic!("预期 Char，得到 {other:?}"),
        }
        match &ops[1] {
            DrawOp::Rule {
                h,
                v,
                width,
                height,
            } => {
                // 规则在 'a' 之后：h = 字符宽。
                // set_rule a b：a=height(100)、b=width(200)，修复后不再互换。
                assert_eq!(*h, 327_681);
                assert_eq!(*v, 0);
                assert_eq!(*width, 200);
                assert_eq!(*height, 100);
            }
            other => panic!("预期 Rule，得到 {other:?}"),
        }
        // 'a' 宽来自 cmr10 TFM（按 fnt_def 缩放后；约 500 设计单位 × 0.01pt × 65536）
        let fm = &dvi.fonts[0];
        let (w, _, _) = fm.char_metrics(b'a' as u32);
        assert!(
            (327_680..=327_700).contains(&w),
            "cmr10 'a' 宽约 327680，实际 {w}"
        );
    }

    /// 字体号 k 与注册位置错位：k=1 起步（引擎 0 号字体未用）、同名重复定义到
    /// 更小的 k=3。选择必须按 k 而非注册顺序（demo.tex 的实际形态，曾报
    /// "未定义字体"）。
    #[test]
    fn font_selection_follows_k_not_registration_order() {
        let Some(_) = ntex_font::find_tfm("cmr10") else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let mut body = Vec::new();
        push_fnt_def(&mut body, 1, b"cmr10");
        body.push(172); // fnt_num_1
        body.push(b'a');
        push_fnt_def(&mut body, 3, b"cmr10"); // 同名重复定义，k 回跳
        body.push(174); // fnt_num_3
        body.push(b'b');

        let mut d = pre_header();
        push_page(&mut d, &body);
        d.push(248); // post
        d.push(0);

        let dvi = parse(&d).unwrap();
        let ops = &dvi.pages[0].ops;
        assert_eq!(ops.len(), 2, "{ops:?}");
        // 每个 k 一条度量：k=1 → 下标 0，k=3 → 下标 1（同名各占一条）
        assert_eq!(dvi.font_names, vec!["cmr10".to_owned(), "cmr10".to_owned()]);
        for (op, expect_font) in ops.iter().zip([0u32, 1]) {
            match op {
                DrawOp::Char { font, code, .. } => {
                    assert_eq!(*font, expect_font, "code {code}");
                    assert_eq!(*code, if expect_font == 0 { b'a' } else { b'b' });
                }
                other => panic!("预期 Char，得到 {other:?}"),
            }
        }
        // 字符宽经 k 解析到度量：'b' 的参考点 = 'a' 的 TFM 宽
        let (w, _, _) = dvi.fonts[0].char_metrics(b'a' as u32);
        match &ops[1] {
            DrawOp::Char { h, .. } => assert_eq!(*h, w, "'a' 推进宽 {w}"),
            other => panic!("预期 Char，得到 {other:?}"),
        }
    }

    /// 未定义字体号：fnt_num 指向没有 fnt_def 过的 k → 报错而非取错字体。
    #[test]
    fn rejects_undefined_font_number() {
        let mut body = Vec::new();
        push_fnt_def(&mut body, 1, b"cmr10");
        body.push(171); // fnt_num_0：k=0 从未定义
        body.push(b'a');

        let mut d = pre_header();
        push_page(&mut d, &body);
        d.push(248); // post
        d.push(0);

        let err = parse(&d).unwrap_err();
        assert!(err.to_string().contains("未定义字体号 0"), "{err}");
    }

    /// 3/4 字节运动量的符号扩展：真实 TeX 的 DVI（数学上下标回退、\topskip
    /// 对齐等）大量使用负的 right3/down3/down4。曾按无符号读，把 -917504sp
    /// 当成 +15835776sp，坐标飞出页面（demo1 数学区被推出页宽外）。
    #[test]
    fn signed_motion_values_are_sign_extended() {
        // 纯函数级：0xF30000（3 字节）= -851968；0xFD8286CF（4 字节）= -41682863
        let mut i = 0;
        assert_eq!(
            read_signed(&[0xF3, 0x00, 0x00], &mut i, 3).unwrap(),
            -851_968
        );
        let mut i = 0;
        let b4 = (-41_682_863i32).to_be_bytes();
        assert_eq!(read_signed(&b4, &mut i, 4).unwrap(), -41_682_863);
        // 正值不受影响：0x024239 = 131072+16896+57 = 148025
        let mut i = 0;
        assert_eq!(
            read_signed(&[0x02, 0x42, 0x39], &mut i, 3).unwrap(),
            148_025
        );

        // 端到端：right3(-10pt) + down3(-2pt) 后的字符坐标必须左/上偏移
        let Some(_) = ntex_font::find_tfm("cmr10") else {
            eprintln!("未找到 cmr10.tfm，跳过");
            return;
        };
        let mut body = Vec::new();
        push_fnt_def(&mut body, 0, b"cmr10");
        body.push(171); // fnt_num_0
        body.push(b'a');
        body.push(145); // right3 -655360（-10pt）
        body.extend_from_slice(&(-655_360i32).to_be_bytes()[1..4]);
        body.push(b'b');
        body.push(159); // down3 -131072（-2pt）
        body.extend_from_slice(&(-131_072i32).to_be_bytes()[1..4]);
        body.push(b'c');
        let mut d = pre_header();
        push_page(&mut d, &body);
        d.push(248);
        d.push(0);
        let dvi = parse(&d).unwrap();
        let ops = &dvi.pages[0].ops;
        let char_xy = |op: &DrawOp| match op {
            DrawOp::Char { h, v, .. } => (*h, *v),
            other => panic!("预期 Char，得到 {other:?}"),
        };
        assert_eq!(char_xy(&ops[0]), (0, 0));
        let (w_a, _, _) = dvi.fonts[0].char_metrics(b'a' as u32);
        assert_eq!(char_xy(&ops[1]), (w_a - 655_360, 0), "right3 负值应左移");
        assert_eq!(char_xy(&ops[2]).1, -131_072, "down3 负值应上移");
    }
}
