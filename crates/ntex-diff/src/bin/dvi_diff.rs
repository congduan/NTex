//! DVI 位置级差分工具（M8 L2 字节兼容诊断，plan.md §10）。
//!
//! 把两份 DVI 各自仿真成"每页字形/规则序列"（字体按 (名字, scale) 归一，
//! 消除 `fnt_def` 编号 k 的编码差异），逐页逐元素对比，报告首个差异与统计。
//!
//! L2 口径：字形码位 + 绝对坐标 (h, v) 逐 sp 一致 = 通过。
//! 编码层差异（w/y/z 寄存器选用、push/pop 结构、页首前奏指令）不进入对比，
//! 但它们造成的**最终坐标偏移**会被如实报告——L2 收尾要求偏移归零。
//!
//! 用法：`dvi-diff <engine.dvi> <reference.dvi> [--max-diffs N]`；
//! 典型入口：`make l2`（端到端管路见 `scripts/l2-check.sh`）。

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::Parser;

/// 字体语义键：`fnt_def` 的 (名字, scale)。
///
/// DVI 字体编号 k 只是编码层选择器（本引擎与真实 TeX 的 k 序列不同），
/// 语义上同一字体 = 同名同缩放，对比按此归一。
#[derive(Debug, Clone, PartialEq, Eq)]
struct FontKey {
    name: String,
    scale: i64,
}

/// 页面元素：字形或规则（流内顺序，绝对 sp 坐标）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Op {
    Char {
        font: Option<FontKey>,
        code: u8,
        h: i64,
        v: i64,
    },
    Rule {
        h: i64,
        v: i64,
        width: i64,
        height: i64,
    },
}

impl Op {
    fn describe(&self) -> String {
        match self {
            Op::Char { font, code, h, v } => {
                let f = font.as_ref().map_or_else(
                    || "未定义字体".to_owned(),
                    |k| format!("{}@{}", k.name, k.scale),
                );
                let ch = match *code {
                    0x20..=0x7E => (*code as char).to_string(),
                    _ => format!("\\{code:03o}"),
                };
                format!(
                    "Char[{f}] '{ch}' h={h}({:.2}pt) v={v}({:.2}pt)",
                    h_pt(*h),
                    h_pt(*v)
                )
            }
            Op::Rule {
                h,
                v,
                width,
                height,
            } => format!(
                "Rule h={h}({:.2}pt) v={v}({:.2}pt) w={width} ht={height}",
                h_pt(*h),
                h_pt(*v)
            ),
        }
    }
}

/// sp → pt（显示用）。
fn h_pt(sp: i64) -> f64 {
    sp as f64 / 65_536.0
}

/// 仿真出的整份文档：每页的元素序列。
struct Doc {
    pages: Vec<Vec<Op>>,
}

#[derive(Parser)]
#[command(
    name = "dvi-diff",
    about = "DVI 位置级差分：两份 DVI 仿真为字形/规则序列后逐页对比（M8 L2）"
)]
struct Args {
    /// 引擎侧 DVI。
    engine: PathBuf,
    /// 参考侧 DVI（如真实 TeX 输出）。
    reference: PathBuf,
    /// 最多报告的差异条数。
    #[arg(long, default_value_t = 10)]
    max_diffs: usize,
}

fn main() -> Result<ExitCode> {
    let args = Args::parse();
    let engine_bytes =
        fs::read(&args.engine).with_context(|| format!("读取 {}", args.engine.display()))?;
    let reference_bytes =
        fs::read(&args.reference).with_context(|| format!("读取 {}", args.reference.display()))?;
    let engine = simulate(&engine_bytes)?;
    let reference = simulate(&reference_bytes)?;

    let matched = compare(&engine, &reference, args.max_diffs);
    Ok(if matched {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// 逐页对比，打印报告；返回是否全部一致。
fn compare(engine: &Doc, reference: &Doc, max_diffs: usize) -> bool {
    let mut all_matched = true;
    if engine.pages.len() != reference.pages.len() {
        println!(
            "页数不一致：引擎 {} vs 参考 {}",
            engine.pages.len(),
            reference.pages.len()
        );
        all_matched = false;
    }
    let n_pages = engine.pages.len().max(reference.pages.len());
    let mut total_diffs = 0usize;
    for p in 0..n_pages {
        let empty: Vec<Op> = Vec::new();
        let e = engine.pages.get(p).unwrap_or(&empty);
        let r = reference.pages.get(p).unwrap_or(&empty);
        let mut page_diffs: Vec<(usize, String, String)> = Vec::new();
        let n_common = e.len().min(r.len());
        for i in 0..n_common {
            if e[i] != r[i] {
                page_diffs.push((i, e[i].describe(), r[i].describe()));
            }
        }
        let page_matched = page_diffs.is_empty() && e.len() == r.len();
        let mark = if page_matched { "一致" } else { "不一致" };
        println!(
            "第 {} 页：{}（引擎 {} 元素 vs 参考 {} 元素，共同段差异 {} 处）",
            p + 1,
            mark,
            e.len(),
            r.len(),
            page_diffs.len()
        );
        if !page_matched {
            all_matched = false;
        }
        for (idx, de, dr) in page_diffs
            .iter()
            .take(max_diffs.saturating_sub(total_diffs))
        {
            println!("  元素 #{idx}:");
            println!("    引擎: {de}");
            println!("    参考: {dr}");
        }
        if e.len() != r.len() {
            let (more, tag) = if e.len() > r.len() {
                (e, "引擎")
            } else {
                (r, "参考")
            };
            let extra: Vec<String> = more[n_common..].iter().take(3).map(Op::describe).collect();
            println!(
                "  {} 多出 {} 个元素（前几个：{}）",
                tag,
                more.len() - n_common,
                extra.join("；")
            );
        }
        total_diffs += page_diffs.len();
        if total_diffs >= max_diffs {
            println!("（已达 --max-diffs {max_diffs}，后续差异省略）");
            break;
        }
    }
    if all_matched {
        println!("L2 位置级对比：全部一致");
    } else {
        println!("L2 位置级对比：存在差异（共报告 {total_diffs} 处）");
    }
    all_matched
}

/// DVI 仿真：字节流 → 每页元素序列。
///
/// 实现全套移动指令语义（h/v/w/x/y/z/push/pop）与 fnt_def 登记；
/// special（xxx）与 bop 计数不进入对比。
fn simulate(bytes: &[u8]) -> Result<Doc> {
    if bytes.first() != Some(&247) {
        bail!("不是 DVI 文件（缺 pre 指令）");
    }
    let mut sim = Sim {
        b: bytes,
        i: 2, // 跳过 pre opcode 与格式版本
        fonts: HashMap::new(),
        pages: Vec::new(),
    };
    // pre：num(4) den(4) mag(4) 注释长度(1) 注释
    sim.i += 12;
    let clen = sim.u8()?;
    sim.i += clen as usize;
    loop {
        let op = sim.u8()?;
        match op {
            139 => {
                let page = sim.page()?;
                sim.pages.push(page);
            }
            248 => break, // post（后记中的 fnt_def 与页面内容无关，忽略）
            138 => {}     // nop
            other => bail!("页间意外 opcode {other}"),
        }
    }
    Ok(Doc { pages: sim.pages })
}

struct Sim<'a> {
    b: &'a [u8],
    i: usize,
    fonts: HashMap<u32, FontKey>,
    pages: Vec<Vec<Op>>,
}

/// DVI 状态栈帧：push/pop 保存/恢复 (h, v, w, x, y, z, font)。
type Frame = (i64, i64, i64, i64, i64, i64, Option<u32>);

impl<'a> Sim<'a> {
    fn u8(&mut self) -> Result<u8> {
        let v = self
            .b
            .get(self.i)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("DVI 截断 @ {}", self.i))?;
        self.i += 1;
        Ok(v)
    }

    fn uint(&mut self, n: usize) -> Result<u32> {
        if self.i + n > self.b.len() {
            bail!("DVI 截断 @ {}（读 {n} 字节）", self.i);
        }
        let mut v = 0u32;
        for k in 0..n {
            v = (v << 8) | u32::from(self.b[self.i + k]);
        }
        self.i += n;
        Ok(v)
    }

    fn i32(&mut self) -> Result<i64> {
        if self.i + 4 > self.b.len() {
            bail!("DVI 截断 @ {}（读 4 字节）", self.i);
        }
        let v = i32::from_be_bytes(self.b[self.i..self.i + 4].try_into()?);
        self.i += 4;
        Ok(i64::from(v))
    }

    /// 读 n 字节有符号整数（1/2 字节按符号扩展）。
    fn signed(&mut self, n: usize) -> Result<i64> {
        let v = self.uint(n)?;
        Ok(match n {
            1 => i64::from(v as i8),
            2 => i64::from(v as i16),
            _ => i64::from(v),
        })
    }

    /// 一页：bop 计数已消费，仿真指令流至 eop。
    fn page(&mut self) -> Result<Vec<Op>> {
        self.i += 48; // 11 个 count(4) + 前页指针(4)
        let mut ops = Vec::new();
        let (mut h, mut v) = (0i64, 0i64);
        let (mut w, mut x, mut y, mut z) = (0i64, 0i64, 0i64, 0i64);
        let mut font: Option<u32> = None;
        let mut stack: Vec<Frame> = Vec::new();
        loop {
            let op = self.u8()?;
            match op {
                140 => break, // eop
                138 => {}     // nop
                141 => stack.push((h, v, w, x, y, z, font)),
                142 => {
                    let (ph, pv, pw, px, py, pz, pf) = stack
                        .pop()
                        .ok_or_else(|| anyhow::anyhow!("pop 栈下溢 @ {}", self.i))?;
                    h = ph;
                    v = pv;
                    w = pw;
                    x = px;
                    y = py;
                    z = pz;
                    font = pf;
                }
                0..=127 => ops.push(self.char(font, op, h, v)?),
                128..=131 => {
                    let code = self.uint(op as usize - 127)? as u8;
                    ops.push(self.char(font, code, h, v)?);
                }
                132 | 137 => {
                    let width = self.i32()?;
                    let height = self.i32()?;
                    ops.push(Op::Rule {
                        h,
                        v,
                        width,
                        height,
                    });
                    if op == 132 {
                        h += width;
                    }
                }
                133..=136 => {
                    let code = self.uint(op as usize - 132)? as u8;
                    ops.push(self.char(font, code, h, v)?);
                }
                143..=146 => h += self.signed(op as usize - 142)?,
                147 => h += w,
                148..=151 => {
                    w = self.signed(op as usize - 147)?;
                    h += w;
                }
                152 => h += x,
                153..=156 => {
                    x = self.signed(op as usize - 152)?;
                    h += x;
                }
                157..=160 => v += self.signed(op as usize - 156)?,
                161 => v += y,
                162..=165 => {
                    y = self.signed(op as usize - 161)?;
                    v += y;
                }
                166 => v += z,
                167..=170 => {
                    z = self.signed(op as usize - 166)?;
                    v += z;
                }
                171..=234 => font = Some(u32::from(op) - 171),
                235..=238 => font = Some(self.uint(op as usize - 234)?),
                239..=242 => {
                    let len = self.uint(op as usize - 238)? as usize;
                    if self.i + len > self.b.len() {
                        bail!("special 长度越界 @ {}", self.i);
                    }
                    self.i += len;
                }
                243..=246 => self.fnt_def(op as usize - 242)?,
                other => bail!("页面指令流中意外 opcode {other} @ {}", self.i - 1),
            }
        }
        Ok(ops)
    }

    /// 字形元素（字体键按当前 DVI 字体号解析；未定义 = None，如实对比）。
    fn char(&self, font: Option<u32>, code: u8, h: i64, v: i64) -> Result<Op> {
        let key = match font {
            Some(k) => self.fonts.get(&k).cloned(),
            None => None,
        };
        Ok(Op::Char {
            font: key,
            code,
            h,
            v,
        })
    }

    /// `fnt_def`：k(n) c(4) s(4) d(4) a(1) l(1) area[a] name[l]。
    fn fnt_def(&mut self, n: usize) -> Result<()> {
        let k = self.uint(n)?;
        self.i += 4; // checksum
        let scale = self.i32()?;
        self.i += 4; // design size（语义键只取名字与 scale）
        let area_len = self.u8()? as usize;
        let name_len = self.u8()? as usize;
        if self.i + area_len + name_len > self.b.len() {
            bail!("fnt_def 名字越界 @ {}", self.i);
        }
        let name_start = self.i + area_len;
        let name = String::from_utf8_lossy(&self.b[name_start..name_start + name_len]).into_owned();
        self.i += area_len + name_len;
        self.fonts.insert(k, FontKey { name, scale });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造最小 DVI：pre + 一页（fnt_def + set_char + down + set_char）+ post。
    fn sample_dvi(font_k: u32, extra_font_def: bool) -> Vec<u8> {
        let mut d = vec![247u8, 2];
        d.extend(254_000_000i32.to_be_bytes());
        d.extend(473_628_672i32.to_be_bytes());
        d.extend(1000i32.to_be_bytes());
        d.push(3);
        d.extend(b"ntx");
        d.push(139); // bop
        for _ in 0..11 {
            d.extend(0i32.to_be_bytes());
        }
        // 前页指针（首页 = -1）；缺了它 bop 长度非 48，后续按字节错位
        d.extend((-1i32).to_be_bytes());
        // fnt_def1：k=font_k，cmr10 10pt；再补一个不用的 k=9 定义（编号差异形态）
        let fdef = |d: &mut Vec<u8>, k: u32| {
            d.push(243);
            d.push(k as u8);
            d.extend(0i32.to_be_bytes());
            d.extend(655_360i32.to_be_bytes());
            d.extend(655_360i32.to_be_bytes());
            d.push(0);
            d.push(5);
            d.extend(b"cmr10");
        };
        fdef(&mut d, font_k);
        if extra_font_def {
            fdef(&mut d, 9);
        }
        // 选择字体 k（fnt1）并写 'a'，down4 12pt，写 'b'
        d.push(235);
        d.push(font_k as u8);
        d.push(b'a');
        d.push(160);
        d.extend(786_432i32.to_be_bytes());
        d.push(b'b');
        d.push(140); // eop
        d.push(248); // post
        d
    }

    #[test]
    fn simulates_glyphs_with_absolute_positions() {
        let dvi = sample_dvi(0, false);
        let doc = simulate(&dvi).unwrap();
        assert_eq!(doc.pages.len(), 1);
        let ops = &doc.pages[0];
        assert_eq!(ops.len(), 2, "{ops:?}");
        let key = Some(FontKey {
            name: "cmr10".to_owned(),
            scale: 655_360,
        });
        assert_eq!(
            ops[0],
            Op::Char {
                font: key.clone(),
                code: b'a',
                h: 0,
                v: 0
            }
        );
        assert_eq!(
            ops[1],
            Op::Char {
                font: key,
                code: b'b',
                h: 0,
                v: 786_432
            }
        );
    }

    /// fnt_def 编号 k 不同（引擎 k=1 起步 vs 真实 TeX k=0 起步）不影响对比：
    /// 字体按 (名字, scale) 归一后应判定一致。
    #[test]
    fn font_number_encoding_difference_is_normalized() {
        let a = simulate(&sample_dvi(0, false)).unwrap();
        let b = simulate(&sample_dvi(1, true)).unwrap();
        assert_eq!(a.pages, b.pages, "k 编号差异应被归一");
    }

    /// 坐标不同必须报差异（防归一化把真实差异也抹掉）。
    #[test]
    fn position_difference_is_reported() {
        let mut d = sample_dvi(0, false);
        // 把页内 down 值 786432 改成 786433：定位并改一个字节
        let pos = d
            .windows(4)
            .position(|w| w == 786_432i32.to_be_bytes())
            .unwrap_or_default();
        d[pos..pos + 4].copy_from_slice(&786_433i32.to_be_bytes());
        let a = simulate(&sample_dvi(0, false)).unwrap();
        let b = simulate(&d).unwrap();
        assert_ne!(a.pages, b.pages, "坐标差异应被检出");
    }
}
