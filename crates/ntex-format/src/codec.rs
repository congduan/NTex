// ---------- 编码原语 ----------

use ntex_core::REGISTER_COUNT;

fn write_tokens(w: &mut impl Write, toks: &[Token]) -> io::Result<()> {
    w.write_all(&(toks.len() as u32).to_le_bytes())?;
    for t in toks {
        w.write_all(&t.raw().to_le_bytes())?;
    }
    Ok(())
}

fn read_tokens(r: &mut impl Read) -> io::Result<Vec<Token>> {
    let n = read_u32(r)? as usize;
    // 预分配钳制：损坏 .fmt 的 token 数可填 2^32-1，按值预分配会 OOM abort
    //（见 lib.rs PREALLOC_CAP 注释）；逐个读取计数不变，损坏文件走 EOF 报错。
    let mut out = Vec::with_capacity(n.min(PREALLOC_CAP));
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
            w.write_all(&[def.protected as u8])?; // e-TeX \protected（M4-5）
            w.write_all(&[def.outer as u8])?; // \outer（v9）
            w.write_all(&[def.active_slot as u8])?; // 定义目标 active char 槽（v16，见 MacroDef::active_slot）
            write_tokens(w, &def.params.text)?; // M1-8 参数文本（含定界符）
            write_tokens(w, &def.body)
        }
        EqSlot::Primitive(p) => {
            w.write_all(&[2])?;
            w.write_all(&p.as_u16().to_le_bytes())
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
        EqSlot::MathChar(code) => {
            w.write_all(&[8])?;
            w.write_all(&code.to_le_bytes())
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
            let protected = read_u8(r)? != 0; // e-TeX \protected（M4-5）
            let outer = read_u8(r)? != 0; // \outer（v9）
            let active_slot = read_u8(r)? != 0; // 定义目标 active char 槽（v16）
            let text = ntex_core::macrodef::TokenArray::from(read_tokens(r)?); // M1-8 参数文本
            let body = ntex_core::macrodef::TokenArray::from(read_tokens(r)?);
            Ok(EqSlot::Macro(ntex_core::version::Versioned {
                value: std::sync::Arc::new(MacroDef {
                    params: ParamSpec {
                        num_params,
                        long,
                        text,
                    },
                    body,
                    code: None,
                    protected,
                    outer,
                    active_slot,
                }),
                version,
            }))
        }
        2 => {
            let p = Primitive::from_u16(read_u16(r)?)
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
        8 => Ok(EqSlot::MathChar(read_u32(r)?)),
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
    // muskip 寄存器（v5：RegKind 编号变更 + muskip 序列化；稀疏存全槽量）
    for i in 0..REGISTER_COUNT {
        let g = r
            .muskip
            .iter()
            .find(|(idx, _)| *idx == i)
            .map(|(_, g)| *g)
            .unwrap_or(Glue::ZERO);
        write_glue(w, g)?;
    }
    w.write_all(&(r.toks.len() as u32).to_le_bytes())?;
    for (idx, toks) in &r.toks {
        w.write_all(&(*idx as u32).to_le_bytes())?;
        write_tokens(w, toks)?;
    }
    Ok(())
}

fn read_registers(r: &mut impl Read) -> io::Result<RegisterState> {
    let mut counts = vec![0i64; REGISTER_COUNT];
    for v in &mut counts {
        *v = read_i64(r)?;
    }
    let mut dimens = vec![0i64; REGISTER_COUNT];
    for v in &mut dimens {
        *v = read_i64(r)?;
    }
    let mut skips = vec![Glue::ZERO; REGISTER_COUNT];
    for g in &mut skips {
        *g = read_glue(r)?;
    }
    let mut muskip = Vec::new();
    for i in 0..REGISTER_COUNT {
        let g = read_glue(r)?;
        if g != Glue::ZERO {
            muskip.push((i, g));
        }
    }
    let n_toks = read_u32(r)? as usize;
    let mut toks = Vec::with_capacity(n_toks.min(PREALLOC_CAP));
    for _ in 0..n_toks {
        let idx = read_u32(r)? as usize;
        let t = ntex_core::macrodef::TokenArray::from(read_tokens(r)?);
        toks.push((idx, t));
    }
    Ok(RegisterState {
        counts: counts.into_boxed_slice(),
        dimens: dimens.into_boxed_slice(),
        skips: skips.into_boxed_slice(),
        muskip,
        toks,
    })
}

fn write_glue(w: &mut impl Write, g: Glue) -> io::Result<()> {
    w.write_all(&g.width.to_le_bytes())?;
    w.write_all(&g.stretch.to_le_bytes())?;
    w.write_all(&g.shrink.to_le_bytes())?;
    // v7：无穷阶（stretch_order/shrink_order）
    w.write_all(&[g.stretch_order, g.shrink_order])
}

fn read_glue(r: &mut impl Read) -> io::Result<Glue> {
    Ok(Glue {
        width: read_i64(r)?,
        stretch: read_i64(r)?,
        shrink: read_i64(r)?,
        stretch_order: read_u8(r)?,
        shrink_order: read_u8(r)?,
    })
}

fn reg_kind_u8(k: RegKind) -> u8 {
    match k {
        RegKind::Count => 0,
        RegKind::Dimen => 1,
        RegKind::Skip => 2,
        RegKind::Muskip => 3,
        RegKind::Toks => 4,
    }
}

fn reg_kind_from_u8(v: u8) -> io::Result<RegKind> {
    match v {
        0 => Ok(RegKind::Count),
        1 => Ok(RegKind::Dimen),
        2 => Ok(RegKind::Skip),
        3 => Ok(RegKind::Muskip),
        4 => Ok(RegKind::Toks),
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

fn read_u16(r: &mut impl Read) -> io::Result<u16> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
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
