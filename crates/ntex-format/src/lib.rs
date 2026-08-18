//! `.fmt` 序列化/反序列化（M3 收尾，v1 内存快照）。
//!
//! v1 目标：把展开引擎状态（[`ntex_core::expand::FmtState`]）编码为二进制文件，
//! 下次启动直接加载，跳过 preamble（宏集）的重新排版。编码为**确定性**
//! 小端格式：同一状态两次编码字节一致（便于校验/增量）。
//!
//! v2（M7）：mmap 零拷贝布局 + 字节码固化 + 部分求值——本 crate 的
//! 编码层保持兼容，v2 在解码路径上做内存映射优化。

use std::io::{self, Read, Write};

use ntex_core::catcode::Catcode;
use ntex_core::eqtb::{EqSlot, Primitive, StreamKind};
use ntex_core::expand::FmtState;
use ntex_core::macrodef::{MacroDef, ParamSpec};
use ntex_core::register::{Glue, RegKind, RegisterState};
use ntex_core::token::Token;
use ntex_core::version::Version;

/// 文件魔数（8 字节）。
const MAGIC: &[u8; 8] = b"NTEXFMT1";
/// 格式版本。
const VERSION: u8 = 1;

/// 编码一个 `.fmt` 快照。
pub fn save(w: &mut impl Write, state: &FmtState) -> io::Result<()> {
    w.write_all(MAGIC)?;
    w.write_all(&[VERSION])?;

    // intern 表
    w.write_all(&(state.intern_names.len() as u32).to_le_bytes())?;
    for name in &state.intern_names {
        w.write_all(&(name.len() as u32).to_le_bytes())?;
        w.write_all(name.as_bytes())?;
    }

    // catcode
    w.write_all(state.catcodes.raw())?;

    // sfcodes
    for v in &state.sfcodes {
        w.write_all(&v.to_le_bytes())?;
    }

    // eqtb
    w.write_all(&(state.eqtb.len() as u32).to_le_bytes())?;
    for slot in &state.eqtb {
        write_slot(w, slot)?;
    }

    // registers
    write_registers(w, &state.registers)?;

    // params
    let p = &state.params;
    write_glue(w, p.baselineskip)?;
    write_glue(w, p.lineskip)?;
    write_glue(w, p.topskip)?;
    write_glue(w, p.parskip)?;
    for v in [
        p.parindent,
        p.lineskiplimit,
        p.hsize,
        p.tolerance,
        p.vsize,
        p.maxdepth,
    ] {
        w.write_all(&v.to_le_bytes())?;
    }

    // output_toks
    write_opt_tokens(w, state.output_toks.as_deref())?;
    Ok(())
}

/// 解码一个 `.fmt` 快照。
pub fn load(r: &mut impl Read) -> io::Result<FmtState> {
    let mut magic = [0u8; 8];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(invalid("不是 .fmt 文件（魔数不符）"));
    }
    let mut version = [0u8; 1];
    r.read_exact(&mut version)?;
    if version[0] != VERSION {
        return Err(invalid("不支持的 .fmt 版本"));
    }

    // intern 表
    let n_names = read_u32(r)? as usize;
    let mut intern_names = Vec::with_capacity(n_names);
    for _ in 0..n_names {
        let len = read_u32(r)? as usize;
        let mut bytes = vec![0u8; len];
        r.read_exact(&mut bytes)?;
        intern_names.push(String::from_utf8(bytes).map_err(|_| invalid("名字非 UTF-8"))?);
    }

    // catcode
    let mut raw_cat = [0u8; 256];
    r.read_exact(&mut raw_cat)?;
    let catcodes = ntex_core::catcode::CatcodeTable::from_raw(raw_cat);

    // sfcodes
    let mut sfcodes = [0u32; 256];
    for v in &mut sfcodes {
        *v = read_u32(r)?;
    }

    // eqtb
    let n_slots = read_u32(r)? as usize;
    let mut eqtb = Vec::with_capacity(n_slots);
    for _ in 0..n_slots {
        eqtb.push(read_slot(r)?);
    }

    // registers
    let registers = read_registers(r)?;

    // params
    let baselineskip = read_glue(r)?;
    let lineskip = read_glue(r)?;
    let topskip = read_glue(r)?;
    let parskip = read_glue(r)?;
    let parindent = read_i64(r)?;
    let lineskiplimit = read_i64(r)?;
    let hsize = read_i64(r)?;
    let tolerance = read_i64(r)?;
    let vsize = read_i64(r)?;
    let maxdepth = read_i64(r)?;
    let params = ntex_core::param::Params {
        parindent,
        baselineskip,
        lineskip,
        lineskiplimit,
        hsize,
        tolerance,
        vsize,
        topskip,
        maxdepth,
        parskip,
    };

    // output_toks
    let output_toks = read_opt_tokens(r)?.map(ntex_core::macrodef::TokenArray::from);

    Ok(FmtState {
        intern_names,
        catcodes,
        sfcodes,
        eqtb,
        registers,
        params,
        output_toks,
    })
}

// ---------- 编码原语 ----------

fn write_tokens(w: &mut impl Write, toks: &[Token]) -> io::Result<()> {
    w.write_all(&(toks.len() as u32).to_le_bytes())?;
    for t in toks {
        w.write_all(&t.raw().to_le_bytes())?;
    }
    Ok(())
}

fn read_tokens(r: &mut impl Read) -> io::Result<Vec<Token>> {
    let n = read_u32(r)? as usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(Token::from_raw(read_u64(r)?));
    }
    Ok(out)
}

fn write_opt_tokens(w: &mut impl Write, toks: Option<&[Token]>) -> io::Result<()> {
    match toks {
        Some(t) => {
            w.write_all(&[1])?;
            write_tokens(w, t)
        }
        None => w.write_all(&[0]),
    }
}

fn read_opt_tokens(r: &mut impl Read) -> io::Result<Option<Vec<Token>>> {
    let mut flag = [0u8; 1];
    r.read_exact(&mut flag)?;
    match flag[0] {
        0 => Ok(None),
        1 => Ok(Some(read_tokens(r)?)),
        _ => Err(invalid("output_toks 标志非法")),
    }
}

fn write_slot(w: &mut impl Write, slot: &EqSlot) -> io::Result<()> {
    match slot {
        EqSlot::Undefined => w.write_all(&[0]),
        EqSlot::Macro(v) => {
            w.write_all(&[1])?;
            w.write_all(&v.version.get().to_le_bytes())?;
            let def: &MacroDef = &v.value;
            w.write_all(&[def.params.num_params])?;
            w.write_all(&[def.params.long as u8])?;
            match &def.params.delimiter {
                Some(d) => {
                    w.write_all(&[1])?;
                    write_tokens(w, d)
                }
                None => {
                    w.write_all(&[0])?;
                    Ok(())
                }
            }?;
            write_tokens(w, &def.body)
        }
        EqSlot::Primitive(p) => {
            w.write_all(&[2])?;
            w.write_all(&[p.as_u8()])
        }
        EqSlot::Alias(target) => {
            w.write_all(&[3])?;
            w.write_all(&target.to_le_bytes())
        }
        EqSlot::Char { catcode, charcode } => {
            w.write_all(&[4])?;
            w.write_all(&[catcode.as_u8()])?;
            w.write_all(&charcode.to_le_bytes())
        }
        EqSlot::Font(font) => {
            w.write_all(&[5])?;
            w.write_all(&font.to_le_bytes())
        }
        EqSlot::Register(kind, idx) => {
            w.write_all(&[6])?;
            w.write_all(&[reg_kind_u8(*kind)])?;
            w.write_all(&(*idx as u32).to_le_bytes())
        }
        EqSlot::Stream(kind, idx) => {
            w.write_all(&[7])?;
            w.write_all(&[stream_kind_u8(*kind)])?;
            w.write_all(&(*idx as u32).to_le_bytes())
        }
    }
}

fn read_slot(r: &mut impl Read) -> io::Result<EqSlot> {
    let mut tag = [0u8; 1];
    r.read_exact(&mut tag)?;
    match tag[0] {
        0 => Ok(EqSlot::Undefined),
        1 => {
            let version = Version::from_raw(read_u64(r)?);
            let num_params = read_u8(r)?;
            let long = read_u8(r)? != 0;
            let delimiter = match read_u8(r)? {
                0 => None,
                1 => Some(ntex_core::macrodef::TokenArray::from(read_tokens(r)?)),
                _ => return Err(invalid("delimiter 标志非法")),
            };
            let body = ntex_core::macrodef::TokenArray::from(read_tokens(r)?);
            Ok(EqSlot::Macro(ntex_core::version::Versioned {
                value: std::sync::Arc::new(MacroDef {
                    params: ParamSpec {
                        num_params,
                        long,
                        delimiter,
                    },
                    body,
                    code: None,
                }),
                version,
            }))
        }
        2 => {
            let p = Primitive::from_u8(read_u8(r)?)
                .ok_or_else(|| invalid("未知原语编号"))?;
            Ok(EqSlot::Primitive(p))
        }
        3 => Ok(EqSlot::Alias(read_u32(r)?)),
        4 => {
            let cat = Catcode::from_u8(read_u8(r)?).ok_or_else(|| invalid("未知 catcode"))?;
            Ok(EqSlot::Char {
                catcode: cat,
                charcode: read_u32(r)?,
            })
        }
        5 => Ok(EqSlot::Font(read_u32(r)?)),
        6 => {
            let kind = reg_kind_from_u8(read_u8(r)?)?;
            Ok(EqSlot::Register(kind, read_u32(r)? as usize))
        }
        7 => {
            let kind = stream_kind_from_u8(read_u8(r)?)?;
            Ok(EqSlot::Stream(kind, read_u32(r)? as usize))
        }
        _ => Err(invalid("未知 eqtb 槽类型")),
    }
}

fn write_registers(w: &mut impl Write, r: &RegisterState) -> io::Result<()> {
    for v in &r.counts {
        w.write_all(&v.to_le_bytes())?;
    }
    for v in &r.dimens {
        w.write_all(&v.to_le_bytes())?;
    }
    for g in &r.skips {
        write_glue(w, *g)?;
    }
    w.write_all(&(r.toks.len() as u32).to_le_bytes())?;
    for (idx, toks) in &r.toks {
        w.write_all(&(*idx as u32).to_le_bytes())?;
        write_tokens(w, toks)?;
    }
    Ok(())
}

fn read_registers(r: &mut impl Read) -> io::Result<RegisterState> {
    let mut counts = [0i64; 256];
    for v in &mut counts {
        *v = read_i64(r)?;
    }
    let mut dimens = [0i64; 256];
    for v in &mut dimens {
        *v = read_i64(r)?;
    }
    let mut skips = [Glue::ZERO; 256];
    for g in &mut skips {
        *g = read_glue(r)?;
    }
    let n_toks = read_u32(r)? as usize;
    let mut toks = Vec::with_capacity(n_toks);
    for _ in 0..n_toks {
        let idx = read_u32(r)? as usize;
        let t = ntex_core::macrodef::TokenArray::from(read_tokens(r)?);
        toks.push((idx, t));
    }
    Ok(RegisterState {
        counts,
        dimens,
        skips,
        toks,
    })
}

fn write_glue(w: &mut impl Write, g: Glue) -> io::Result<()> {
    w.write_all(&g.width.to_le_bytes())?;
    w.write_all(&g.stretch.to_le_bytes())?;
    w.write_all(&g.shrink.to_le_bytes())
}

fn read_glue(r: &mut impl Read) -> io::Result<Glue> {
    Ok(Glue {
        width: read_i64(r)?,
        stretch: read_i64(r)?,
        shrink: read_i64(r)?,
    })
}

fn reg_kind_u8(k: RegKind) -> u8 {
    match k {
        RegKind::Count => 0,
        RegKind::Dimen => 1,
        RegKind::Skip => 2,
        RegKind::Toks => 3,
    }
}

fn reg_kind_from_u8(v: u8) -> io::Result<RegKind> {
    match v {
        0 => Ok(RegKind::Count),
        1 => Ok(RegKind::Dimen),
        2 => Ok(RegKind::Skip),
        3 => Ok(RegKind::Toks),
        _ => Err(invalid("未知寄存器类型")),
    }
}

fn stream_kind_u8(k: StreamKind) -> u8 {
    match k {
        StreamKind::Read => 0,
        StreamKind::Write => 1,
    }
}

fn stream_kind_from_u8(v: u8) -> io::Result<StreamKind> {
    match v {
        0 => Ok(StreamKind::Read),
        1 => Ok(StreamKind::Write),
        _ => Err(invalid("未知流类型")),
    }
}

fn read_u8(r: &mut impl Read) -> io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64(r: &mut impl Read) -> io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn read_i64(r: &mut impl Read) -> io::Result<i64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(i64::from_le_bytes(b))
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ntex_core::catcode::Catcode;
    use ntex_core::expand::Expander;

    fn roundtrip(state: &FmtState) -> FmtState {
        let mut buf = Vec::new();
        save(&mut buf, state).unwrap();
        let mut loaded = FmtState {
            intern_names: Vec::new(),
            catcodes: ntex_core::catcode::CatcodeTable::new(),
            sfcodes: [0; 256],
            eqtb: Vec::new(),
            registers: RegisterState {
                counts: [0; 256],
                dimens: [0; 256],
                skips: [Glue::ZERO; 256],
                toks: Vec::new(),
            },
            params: ntex_core::param::Params::default(),
            output_toks: None,
        };
        let mut cur = buf.as_slice();
        loaded = load(&mut cur).unwrap();
        assert!(cur.is_empty(), "解码后应消费全部输入");
        assert_eq!(*state, loaded, "roundtrip 后状态应一致");
        loaded
    }

    /// 构造包含宏/寄存器/参数/输出例程的引擎状态。
    fn sample_state() -> FmtState {
        let mut e = Expander::new();
        for (i, c) in [
            r"\def\greet#1{Hi #1!}\let\g\greet",
            r"\count0=42",
            r"\parindent 20pt",
            r"\toks0={abc}",
            r"\output={x}",
            r"\catcode 95=11",
        ]
        .iter()
        .enumerate()
        {
            e.run_source(*c).unwrap_or_else(|err| panic!("段 {i} [{c}] 失败：{err}"));
        }
        e.export_state()
    }

    #[test]
    fn fmt_roundtrip_preserves_state() {
        let state = sample_state();
        let loaded = roundtrip(&state);
        assert_eq!(state.intern_names, loaded.intern_names);
        assert!(state.intern_names.iter().any(|n| n == "greet"));
    }

    #[test]
    fn fmt_encoding_is_deterministic() {
        let state = sample_state();
        let mut a = Vec::new();
        let mut b = Vec::new();
        save(&mut a, &state).unwrap();
        save(&mut b, &state).unwrap();
        assert_eq!(a, b, "同一状态两次编码应逐字节一致");
    }

    #[test]
    fn fmt_rejects_bad_magic() {
        let mut bad = b"NOPEXXXX".to_vec();
        bad.extend_from_slice(&[1]);
        assert!(load(&mut bad.as_slice()).is_err());
    }

    #[test]
    fn fmt_load_then_use_produces_same_output() {
        // 快照引擎定义宏 → 保存 → 加载到新引擎 → 展开同一文档，输出一致
        let preamble = r"\def\greet#1{Hi #1!}\def\echo#1{#1}";
        let doc = r"\greet{World} \echo{ok}";

        let mut orig = Expander::new();
        orig.run_source(preamble).unwrap();
        let mut buf = Vec::new();
        save(&mut buf, &orig.export_state()).unwrap();

        let mut fresh = Expander::new();
        fresh.import_state(load(&mut buf.as_slice()).unwrap());

        let mut out_orig = Expander::new();
        out_orig.run_source(preamble).unwrap();
        let r1 = expand_to_str(&mut out_orig, doc);
        let r2 = expand_to_str(&mut fresh, doc);
        assert_eq!(r1, r2, "加载 .fmt 后展开应与全新排版一致");
    }

    fn expand_to_str(e: &mut Expander, src: &str) -> String {
        e.run_source(src).unwrap();
        e.output()
            .iter()
            .map(|t| t.charcode().and_then(char::from_u32).unwrap_or('\u{FFFD}'))
            .collect()
    }

    #[test]
    fn fmt_preserves_output_routine() {
        let state = sample_state();
        let loaded = roundtrip(&state);
        assert!(loaded.output_toks.is_some(), "\\output 例程应保留");
    }

    #[test]
    fn fmt_file_roundtrip() {
        let state = sample_state();
        let path = std::env::temp_dir().join(format!("ntex-fmt-test-{}.fmt", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            save(&mut f, &state).unwrap();
        }
        let loaded = {
            let mut f = std::fs::File::open(&path).unwrap();
            load(&mut f).unwrap()
        };
        std::fs::remove_file(&path).ok();
        assert_eq!(state, loaded, "文件 roundtrip 后状态应一致");
    }
}
