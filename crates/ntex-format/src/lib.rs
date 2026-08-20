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
/// 格式版本（v2：M4-4 显示数学间距参数；v3：ETRIP 内部整数参数；
/// v4：M1-8 参数文本全量序列化——定界符标志改为参数文本 token 数组）。
const VERSION: u8 = 5;

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
    // M4-4 显示数学间距（v2 追加）
    for g in [
        &p.abovedisplayskip,
        &p.belowdisplayskip,
        &p.abovedisplayshortskip,
        &p.belowdisplayshortskip,
    ] {
        write_glue(w, *g)?;
    }
    for v in [p.predisplaypenalty, p.postdisplaypenalty] {
        w.write_all(&v.to_le_bytes())?;
    }
    // ETRIP 冲刺（v3 追加）：TeX 内部整数参数
    for v in [
        p.endlinechar,
        p.newlinechar,
        p.defaulthyphenchar,
        p.defaultskewchar,
    ] {
        w.write_all(&v.to_le_bytes())?;
    }
    // ETRIP 冲刺（v4 追加）：TeX/e-TeX 内部整数参数（misc 数组）
    for v in p.misc {
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
    // M4-4 显示数学间距（v2）
    let abovedisplayskip = read_glue(r)?;
    let belowdisplayskip = read_glue(r)?;
    let abovedisplayshortskip = read_glue(r)?;
    let belowdisplayshortskip = read_glue(r)?;
    let predisplaypenalty = read_i64(r)?;
    let postdisplaypenalty = read_i64(r)?;
    // ETRIP 冲刺（v3）：TeX 内部整数参数
    let endlinechar = read_i64(r)?;
    let newlinechar = read_i64(r)?;
    let defaulthyphenchar = read_i64(r)?;
    let defaultskewchar = read_i64(r)?;
    // ETRIP 冲刺（v4）：TeX/e-TeX 内部整数参数（misc 数组）
    let mut misc = [0i64; ntex_core::param::MISC_INTS];
    for v in &mut misc {
        *v = read_i64(r)?;
    }
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
        abovedisplayskip,
        belowdisplayskip,
        abovedisplayshortskip,
        belowdisplayshortskip,
        predisplaypenalty,
        postdisplaypenalty,
        endlinechar,
        newlinechar,
        defaulthyphenchar,
        defaultskewchar,
        misc,
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

// ---------- 编码原语（include! 嵌入） ----------
include!("codec.rs");


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
                muskip: Vec::new(),
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
        assert_eq!(state.intern_names, loaded.intern_names, "intern_names");
        assert_eq!(state.catcodes, loaded.catcodes, "catcodes");
        assert_eq!(state.sfcodes, loaded.sfcodes, "sfcodes");
        assert_eq!(state.eqtb, loaded.eqtb, "eqtb");
        assert_eq!(state.registers, loaded.registers, "registers");
        assert_eq!(state.params, loaded.params, "params");
        assert_eq!(state.output_toks, loaded.output_toks, "output_toks");
        assert_eq!(state, loaded, "文件 roundtrip 后状态应一致");
    }
}
