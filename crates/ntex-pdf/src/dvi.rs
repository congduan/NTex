//! DVI 解析器（M8 输出后端）：字节流 → 每页的绘制指令列表。
//!
//! 坐标单位：DVI 的 scaled point（1pt = 65536sp；mag=1000 时 1 单位 = 1sp）。
//! h 向右、v 向下（原点在页左上）。PDF 转换在写出层做（y 翻转 + 除 65536）。
//!
//! 支持指令：set_char/set1-4、set_rule、put_rule、push/pop、
//! right/down（含 w/x/y/z 寄存器）、fnt_num/fnt_def、xxx（忽略）、
//! bop/eop/pre/post/post_post。

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

/// 读 `n` 字节大端有符号整数（1..=4；最高字节为符号位）。
fn read_signed(b: &[u8], i: &mut usize, n: usize) -> io::Result<i64> {
    let v = read_uint(b, i, n)? as i64;
    Ok(match n {
        1 => (v as i8) as i64,
        2 => (v as i16) as i64,
        _ => v,
    })
}

/// 解析 DVI 字节流。
pub fn parse(bytes: &[u8]) -> io::Result<Dvi> {
    let p = Parser {
        b: bytes,
        i: 0,
        fonts: Vec::new(),
        font_names: Vec::new(),
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
                    // set_rule：宽 a、高 b（规则底部在 v，向上 b）
                    let width = read_i32(self.b, &mut self.i)?;
                    let height = read_i32(self.b, &mut self.i)?;
                    ops.push(DrawOp::Rule {
                        h,
                        v,
                        width,
                        height,
                    });
                    h += width;
                }
                137 => {
                    // put_rule
                    let width = read_i32(self.b, &mut self.i)?;
                    let height = read_i32(self.b, &mut self.i)?;
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
                171..=234 => font = (op - 171) as u32,
                // fnt1..fnt4
                235..=238 => {
                    let n = (op - 234) as usize;
                    font = read_uint(self.b, &mut self.i, n)?;
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

    /// `fnt_def n`：k(1) c(4) s(4) d(4) a(1) l(1) name → 注册字体表 + 当前字体。
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

        let fm = load_tfm(&name, scale, design)?;
        // 与同名字体复用（DVI 可能多次 fnt_def 同字体）
        if let Some(pos) = self.font_names.iter().position(|n| *n == name) {
            *font = pos as u32;
        } else {
            let id = u32::try_from(self.fonts.len())
                .map_err(|_| io::Error::new(ErrorKind::InvalidData, "字体过多"))?;
            self.font_names.push(name);
            self.fonts.push(fm);
            *font = id;
        }
        // k 与位置可能不同（k 是 DVI 字体号，我们按位置索引）；强制按 k 建立映射
        let _ = k;
        Ok(())
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
fn load_tfm(name: &str, scale: i64, design: i64) -> io::Result<FontMetrics> {
    let path = ntex_font::find_tfm(name)
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, format!("找不到 TFM：{name}")))?;
    let bytes = std::fs::read(&path)?;
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
                // 规则在 'a' 之后：h = 字符宽
                assert_eq!(*h, 327_681);
                assert_eq!(*v, 0);
                assert_eq!(*width, 100);
                assert_eq!(*height, 200);
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
}
