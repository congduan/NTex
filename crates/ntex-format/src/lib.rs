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
/// v4：M1-8 参数文本全量序列化——定界符标志改为参数文本 token 数组；
/// v6：ETRIP `\parfillskip` 胶水参数；v7：胶水无穷阶；
/// v8：ETRIP 第二波——`\leftskip`/`\rightskip`/`\prevdepth`/`\interlinepenalty`
/// /`\clubpenalty`/`\widowpenalty`/`\displaywidowpenalty` 与 misc 扩 33；
/// v9：`\outer` 宏标志序列化（MacroDef.outer）；
/// v10：TRIP 冲刺——`\nulldelimiterspace`/`\scriptspace`/`\overfullrule`/`\voffset`/`\hoffset`；
/// v11：TRIP 冲刺——`\xspaceskip` 胶水参数；
/// v13：ETRIP——font_loads（pass2 恢复字体表）+ font_cs_names（showbox 字体 cs 名）；
/// v14：ETRIP——current_font（pass2 恢复当前字体，防全 nullfont）；
/// v15：M9 中文刀 4——catcode >255 码位覆盖表（\utfinputmode=1 的 \catcode`，=13）；
/// v16：outer 双槽位（MacroDef::active_slot，expl3 L9320 Forbidden 根治的伴随序列化）；
/// v17：文件头写入 NTex 引擎版本号，加载时强校验，防旧 fmt 静默腐蚀。
/// v18：every* token 列表族（`\everypar` 段首触发链 / LaTeX 段落钩子机器）——
///      此前快照不携带，恢复后 `\everypar` 恒空，list 机制（`\@newlist` 清位）失效。
/// v19：misc 数组增位（67→68，`\pdflastximage` 槽；图片管线 Step A）——
///      misc 定长序列化，长度变而版本不变时旧快照在数组读取处报
///      "failed to fill whole buffer"（无声错配），故布局变必须同步 bump。
/// v21：四张 code 表（`\delcode`/`\mathcode`/`\lccode`/`\uccode`）——
///      fontmath.ltx 在 fmt 生成期做的 `\DeclareMathSymbol`/`\delcode`
///      字符赋值此前不随快照携带，恢复后全部退回引擎 INITEX 初表
///      （0x7000+码/0x500000）：`<`>` 落 cmr 同槽（`¡`/`¿`）、punct/rel/bin
///      间距整族失效、`\left(`/`\right[` 大定界符报 Missing delimiter。
/// v22：`\fontdimen`/`\hyphenchar` 覆盖表——l3kernel intarray 的 pdftex 回退
///      分支把整数组模拟成字体（条目存 `\fontdimen`、count 存 `\hyphenchar`，
///      expl3-code l.15574 `\__intarray_new:N`），数组在 fmt 生成期创建；
///      此前快照不带这两张表，恢复后 count 退回 TFM 默认 45、条目全 0
///      ——四个常量 cctab 读回全 0，`\cctab_select:N` 把 catcode 全抹 0
///      （ctex `\cctab_const:Nn \c__ctex_package_cctab` 挂点）。
pub const FORMAT_VERSION: u8 = 22;

/// 当前引擎版本号：随 crate 版本进入 `.fmt` 文件头。
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 解码期预分配/名字长度上限。`.fmt` 的长度字段是**文件内容**（u32），恶意
/// 或损坏的文件可填 2^32-1：按其值 `with_capacity`/`vec![0u8; n]` 会一次性
/// 申请数 GB——容量溢出 panic 或 OOM abort（TeX 引擎的传统是永不死于坏输入，
/// 错误恢复后继续）。故预分配一律钳到本上限（循环计数不变，合法文件零影响：
/// 名字/cs 名/槽数至多数千），越界的名字长度直接报 invalid——损坏文件在
/// read_exact EOF 或显式检查处优雅失败。64 KiB 同时盖住 `read_and_check_
/// engine_version` 的 u16 长度域（65535 = 本上限，检查恒过）。
const PREALLOC_CAP: usize = 1 << 16;

/// 编码一个 `.fmt` 快照。
pub fn save(w: &mut impl Write, state: &FmtState) -> io::Result<()> {
    w.write_all(MAGIC)?;
    w.write_all(&[FORMAT_VERSION])?;
    write_engine_version(w)?;

    // intern 表
    w.write_all(&(state.intern_names.len() as u32).to_le_bytes())?;
    for name in &state.intern_names {
        w.write_all(&(name.len() as u32).to_le_bytes())?;
        w.write_all(name.as_bytes())?;
    }

    // catcode
    w.write_all(state.catcodes.raw())?;
    // v15 追加：>255 码位覆盖表（`\utfinputmode=1` 下 `\catcode`，=13 的赋值）。
    // 必须与 `load` 侧的读取顺序严格对称，否则整体字节流错位。
    let overrides: Vec<(u32, u8)> = state.catcodes.unicode_overrides().collect();
    w.write_all(&(overrides.len() as u32).to_le_bytes())?;
    for (cp, v) in overrides {
        w.write_all(&cp.to_le_bytes())?;
        w.write_all(&[v])?;
    }

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
        p.mag,
    ] {
        w.write_all(&v.to_le_bytes())?;
    }
    // TRIP 冲刺（v10 追加）：数学/整页 dimen 内部参数
    for v in [
        p.nulldelimiterspace,
        p.scriptspace,
        p.overfullrule,
        p.voffset,
        p.hoffset,
    ] {
        w.write_all(&v.to_le_bytes())?;
    }
    // ETRIP 冲刺（v4 追加）：TeX/e-TeX 内部整数参数（misc 数组）
    for v in p.misc {
        w.write_all(&v.to_le_bytes())?;
    }
    // ETRIP 冲刺（v6 追加）：\parfillskip 胶水
    write_glue(w, p.parfillskip)?;
    // TRIP 冲刺（v11 追加）：\xspaceskip 胶水
    write_glue(w, p.xspaceskip)?;

    // ETRIP 第二波（v8 追加）：\leftskip/\rightskip/\prevdepth + 行间/孤行/段首断页惩罚
    write_glue(w, p.leftskip)?;
    write_glue(w, p.rightskip)?;
    for v in [
        p.prevdepth,
        p.interlinepenalty,
        p.clubpenalty,
        p.widowpenalty,
        p.displaywidowpenalty,
    ] {
        w.write_all(&v.to_le_bytes())?;
    }
    // TRIP 冲刺（v12 追加）：补充标准参数原语（\hangindent/\spaceskip/\tabskip/
    // \lastskip/\splittopskip 胶水 + \hfuzz/\vfuzz/\boxmaxdepth/\splitmaxdepth/
    // \emergencystretch/\displayindent/\delimitershortfall/\lastkern 尺寸）
    for g in [
        p.hangindent,
        p.spaceskip,
        p.tabskip,
        p.lastskip,
        p.splittopskip,
        p.pagestretch,
        p.pagefilstretch,
        p.pagefillstretch,
    ] {
        write_glue(w, g)?;
    }
    for v in [
        p.hfuzz,
        p.vfuzz,
        p.boxmaxdepth,
        p.splitmaxdepth,
        p.emergencystretch,
        p.displayindent,
        p.delimitershortfall,
        p.lastkern,
        p.mathsurround,
    ] {
        w.write_all(&v.to_le_bytes())?;
    }

    // output_toks
    write_opt_tokens(w, state.output_toks.as_deref())?;

    // every* token 列表族（v18：`\everypar` 段首触发链 / LaTeX 段落钩子机器）
    write_tokens(w, &state.every_toks.everypar)?;
    write_tokens(w, &state.every_toks.everymath)?;
    write_tokens(w, &state.every_toks.everyhbox)?;
    write_tokens(w, &state.every_toks.everyvbox)?;
    write_tokens(w, &state.every_toks.everycr)?;
    write_tokens(w, &state.every_toks.everydisplay)?;
    write_tokens(w, &state.every_toks.errhelp)?;

    // font_loads（v13：pass2 恢复字体表用；FontId → (外部名, at, scaled)）
    w.write_all(&(state.font_loads.len() as u32).to_le_bytes())?;
    for load in &state.font_loads {
        match load {
            Some((name, at, scaled)) => {
                w.write_all(&(name.len() as u32).to_le_bytes())?;
                w.write_all(name.as_bytes())?;
                w.write_all(&(at.unwrap_or(0) as u32).to_le_bytes())?;
                w.write_all(&(at.is_some() as u32).to_le_bytes())?;
                w.write_all(&(scaled.unwrap_or(0) as u32).to_le_bytes())?;
                w.write_all(&(scaled.is_some() as u32).to_le_bytes())?;
            }
            None => w.write_all(&0u32.to_le_bytes())?,
        }
    }
    // font_cs_names（v13：showbox 字体标识 cs 名，pass2 保留）
    w.write_all(&(state.font_cs_names.len() as u32).to_le_bytes())?;
    for name in &state.font_cs_names {
        match name {
            Some(n) => {
                w.write_all(&(n.len() as u32).to_le_bytes())?;
                w.write_all(n.as_bytes())?;
            }
            None => w.write_all(&0u32.to_le_bytes())?,
        }
    }
    // fontdimens / hyphenchars（v22：intarray 模拟字体的条目与 count；导出侧
    // 已按键排序，字节确定性成立）
    w.write_all(&(state.fontdimens.len() as u32).to_le_bytes())?;
    for (f, n, v) in &state.fontdimens {
        w.write_all(&f.to_le_bytes())?;
        w.write_all(&n.to_le_bytes())?;
        w.write_all(&v.to_le_bytes())?;
    }
    w.write_all(&(state.hyphenchars.len() as u32).to_le_bytes())?;
    for (f, c) in &state.hyphenchars {
        w.write_all(&f.to_le_bytes())?;
        w.write_all(&c.to_le_bytes())?;
    }
    // current_font（v14：pass2 恢复当前字体，防全 nullfont）
    w.write_all(&state.current_font.to_le_bytes())?;

    // v21：四张 code 表（`\delcode`/`\mathcode`/`\lccode`/`\uccode`）。
    // mathcodes/delcodes 覆盖表按码位升序（确定性）；lccode/uccode 定长 256。
    // fontmath.ltx 在 fmt 生成期的字符级数学分派赋值此前随快照丢失。
    let mut dels: Vec<(u32, u32)> = state.delcodes.iter().map(|(k, v)| (*k, *v)).collect();
    dels.sort_unstable();
    w.write_all(&(dels.len() as u32).to_le_bytes())?;
    for (k, v) in dels {
        w.write_all(&k.to_le_bytes())?;
        w.write_all(&v.to_le_bytes())?;
    }
    let mut maths: Vec<(u32, u32)> = state.mathcodes.iter().map(|(k, v)| (*k, *v)).collect();
    maths.sort_unstable();
    w.write_all(&(maths.len() as u32).to_le_bytes())?;
    for (k, v) in maths {
        w.write_all(&k.to_le_bytes())?;
        w.write_all(&v.to_le_bytes())?;
    }
    for v in &state.lccodes {
        w.write_all(&v.to_le_bytes())?;
    }
    for v in &state.uccodes {
        w.write_all(&v.to_le_bytes())?;
    }
    // v21 兼容尾扩展：旧 v21 文件到此结束；新文件若有 cctab 数据，load 侧
    // 读到尾巴即恢复，读不到则由 core 的旧 latex.fmt 补种逻辑兜底。
    w.write_all(&(state.catcode_tables.len() as u32).to_le_bytes())?;
    for table in &state.catcode_tables {
        match table {
            Some(t) => {
                w.write_all(&[1])?;
                w.write_all(t.raw())?;
                let overrides: Vec<(u32, u8)> = t.unicode_overrides().collect();
                w.write_all(&(overrides.len() as u32).to_le_bytes())?;
                for (cp, v) in overrides {
                    w.write_all(&cp.to_le_bytes())?;
                    w.write_all(&[v])?;
                }
            }
            None => w.write_all(&[0])?,
        }
    }
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
    if version[0] != FORMAT_VERSION {
        return Err(invalid(&format!(
            "不支持的 .fmt 版本：文件={}，当前={}；请用 --generate-fmt 重新 dump",
            version[0], FORMAT_VERSION
        )));
    }
    read_and_check_engine_version(r)?;

    // intern 表
    let n_names = read_u32(r)? as usize;
    let mut intern_names = Vec::with_capacity(n_names.min(PREALLOC_CAP));
    for _ in 0..n_names {
        let len = read_u32(r)? as usize;
        if len > PREALLOC_CAP {
            return Err(invalid("intern 名字长度越界（文件损坏）"));
        }
        let mut bytes = vec![0u8; len];
        r.read_exact(&mut bytes)?;
        intern_names.push(String::from_utf8(bytes).map_err(|_| invalid("名字非 UTF-8"))?);
    }

    // catcode
    let mut raw_cat = [0u8; 256];
    r.read_exact(&mut raw_cat)?;
    let mut catcodes = ntex_core::catcode::CatcodeTable::from_raw(raw_cat);
    // v15 追加：>255 码位覆盖表回填
    let n_overrides = read_u32(r)?;
    for _ in 0..n_overrides {
        let cp = read_u32(r)?;
        let mut b = [0u8; 1];
        r.read_exact(&mut b)?;
        if let Some(c) = ntex_core::catcode::Catcode::from_u8(b[0]) {
            catcodes.set_codepoint(cp, c);
        }
    }

    // sfcodes
    let mut sfcodes = [0u32; 256];
    for v in &mut sfcodes {
        *v = read_u32(r)?;
    }

    // eqtb
    let n_slots = read_u32(r)? as usize;
    let mut eqtb = Vec::with_capacity(n_slots.min(PREALLOC_CAP));
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
    let mag = read_i64(r)?;
    // TRIP 冲刺（v10）：数学/整页 dimen 内部参数
    let nulldelimiterspace = read_i64(r)?;
    let scriptspace = read_i64(r)?;
    let overfullrule = read_i64(r)?;
    let voffset = read_i64(r)?;
    let hoffset = read_i64(r)?;
    // ETRIP 冲刺（v4）：TeX/e-TeX 内部整数参数（misc 数组）
    let mut misc = [0i64; ntex_core::param::MISC_INTS];
    for v in &mut misc {
        *v = read_i64(r)?;
    }
    // ETRIP 冲刺（v6）：\parfillskip 胶水
    let parfillskip = read_glue(r)?;
    // TRIP 冲刺（v11）：\xspaceskip 胶水
    let xspaceskip = read_glue(r)?;
    // ETRIP 第二波（v8）：段落/断页参数
    let leftskip = read_glue(r)?;
    let rightskip = read_glue(r)?;
    let prevdepth = read_i64(r)?;
    let interlinepenalty = read_i64(r)?;
    let clubpenalty = read_i64(r)?;
    let widowpenalty = read_i64(r)?;
    let displaywidowpenalty = read_i64(r)?;
    let hangindent = read_glue(r)?;
    let spaceskip = read_glue(r)?;
    let tabskip = read_glue(r)?;
    let lastskip = read_glue(r)?;
    let splittopskip = read_glue(r)?;
    let pagestretch = read_glue(r)?;
    let pagefilstretch = read_glue(r)?;
    let pagefillstretch = read_glue(r)?;
    let hfuzz = read_i64(r)?;
    let vfuzz = read_i64(r)?;
    let boxmaxdepth = read_i64(r)?;
    let splitmaxdepth = read_i64(r)?;
    let emergencystretch = read_i64(r)?;
    let displayindent = read_i64(r)?;
    let delimitershortfall = read_i64(r)?;
    let lastkern = read_i64(r)?;
    let mathsurround = read_i64(r)?;
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
        parfillskip,
        xspaceskip,
        abovedisplayskip,
        belowdisplayskip,
        abovedisplayshortskip,
        belowdisplayshortskip,
        predisplaypenalty,
        postdisplaypenalty,
        leftskip,
        rightskip,
        prevdepth,
        interlinepenalty,
        clubpenalty,
        widowpenalty,
        displaywidowpenalty,
        hangindent,
        spaceskip,
        tabskip,
        lastskip,
        hfuzz,
        vfuzz,
        boxmaxdepth,
        splitmaxdepth,
        splittopskip,
        emergencystretch,
        displayindent,
        delimitershortfall,
        lastkern,
        mathsurround,
        pagestretch,
        pagefilstretch,
        pagefillstretch,
        endlinechar,
        newlinechar,
        defaulthyphenchar,
        defaultskewchar,
        mag,
        nulldelimiterspace,
        scriptspace,
        overfullrule,
        voffset,
        hoffset,
        misc,
    };

    // output_toks
    let output_toks = read_opt_tokens(r)?.map(ntex_core::macrodef::TokenArray::from);

    // every* token 列表族（v18；顺序与 save 严格一致）
    let every_toks = ntex_core::expand::EveryToks {
        everypar: read_tokens(r)?,
        everymath: read_tokens(r)?,
        everyhbox: read_tokens(r)?,
        everyvbox: read_tokens(r)?,
        everycr: read_tokens(r)?,
        everydisplay: read_tokens(r)?,
        errhelp: read_tokens(r)?,
    };

    // font_loads（v13：pass2 恢复字体表；FontId → (外部名, at, scaled)）
    let n_loads = read_u32(r)? as usize;
    let mut font_loads = Vec::with_capacity(n_loads.min(PREALLOC_CAP));
    for _ in 0..n_loads {
        let len = read_u32(r)? as usize;
        if len == 0 {
            font_loads.push(None);
        } else {
            if len > PREALLOC_CAP {
                return Err(invalid("字体名长度越界（文件损坏）"));
            }
            let mut buf = vec![0u8; len];
            r.read_exact(&mut buf)?;
            let name = String::from_utf8_lossy(&buf).into_owned();
            let at = read_u32(r)? as i64;
            let at_some = read_u32(r)? != 0;
            let scaled = read_u32(r)? as i64;
            let scaled_some = read_u32(r)? != 0;
            font_loads.push(Some((
                name,
                at_some.then_some(at),
                scaled_some.then_some(scaled),
            )));
        }
    }
    // font_cs_names（v13：showbox 字体标识 cs 名；**保留**——pass1 定义的
    // `\font\trip` 在 pass2 不重跑，showbox 需 cs 名）
    let n = read_u32(r)? as usize;
    let mut font_cs_names = Vec::with_capacity(n.min(PREALLOC_CAP));
    for _ in 0..n {
        let len = read_u32(r)? as usize;
        if len == 0 {
            font_cs_names.push(None);
        } else {
            if len > PREALLOC_CAP {
                return Err(invalid("字体 cs 名长度越界（文件损坏）"));
            }
            let mut buf = vec![0u8; len];
            r.read_exact(&mut buf)?;
            font_cs_names.push(Some(String::from_utf8_lossy(&buf).into_owned()));
        }
    }
    // current_font（v14）
    let current_font = read_u32(r)?;

    // v21：四张 code 表
    let n_del = read_u32(r)? as usize;
    let mut delcodes = std::collections::HashMap::with_capacity(n_del.min(PREALLOC_CAP));
    for _ in 0..n_del {
        let k = read_u32(r)?;
        let v = read_u32(r)?;
        delcodes.insert(k, v);
    }
    let n_math = read_u32(r)? as usize;
    let mut mathcodes = std::collections::HashMap::with_capacity(n_math.min(PREALLOC_CAP));
    for _ in 0..n_math {
        let k = read_u32(r)?;
        let v = read_u32(r)?;
        mathcodes.insert(k, v);
    }
    let mut lccodes = [0i64; 256];
    for v in &mut lccodes {
        *v = read_i64(r)?;
    }
    let mut uccodes = [0i64; 256];
    for v in &mut uccodes {
        *v = read_i64(r)?;
    }
    // v22：fontdimens / hyphenchars 覆盖表（intarray 模拟字体的条目与 count）
    let n_fd = read_u32(r)? as usize;
    let mut fontdimens = Vec::with_capacity(n_fd.min(1 << 20));
    for _ in 0..n_fd {
        let f = read_u32(r)?;
        let n = read_u32(r)?;
        let v = read_i64(r)?;
        fontdimens.push((f, n, v));
    }
    let n_hc = read_u32(r)? as usize;
    let mut hyphenchars = Vec::with_capacity(n_hc.min(PREALLOC_CAP));
    for _ in 0..n_hc {
        let f = read_u32(r)?;
        let c = read_i64(r)?;
        hyphenchars.push((f, c));
    }
    let mut tail = Vec::new();
    r.read_to_end(&mut tail)?;
    let catcode_tables = if tail.is_empty() {
        Vec::new()
    } else {
        let mut tr = &tail[..];
        read_catcode_tables(&mut tr)?
    };

    Ok(FmtState {
        intern_names,
        catcodes,
        catcode_tables,
        sfcodes,
        eqtb,
        registers,
        params,
        output_toks,
        every_toks,
        // .fmt v1 不含字体表（加载后需重新 \font）：font_names 一并置空
        font_names: Vec::new(),
        font_loads,
        font_cs_names,
        current_font,
        delcodes,
        mathcodes,
        lccodes,
        uccodes,
        fontdimens,
        hyphenchars,
    })
}

fn read_catcode_tables(r: &mut impl Read) -> io::Result<Vec<Option<ntex_core::CatcodeTable>>> {
    let n_tables = read_u32(r)? as usize;
    if n_tables > PREALLOC_CAP {
        return Err(invalid("catcode table 数量越界（文件损坏）"));
    }
    let mut tables = Vec::with_capacity(n_tables);
    for _ in 0..n_tables {
        let tag = read_u8(r)?;
        if tag == 0 {
            tables.push(None);
            continue;
        }
        if tag != 1 {
            return Err(invalid("catcode table 标记非法（文件损坏）"));
        }
        let mut raw = [0u8; 256];
        r.read_exact(&mut raw)?;
        let mut table = ntex_core::CatcodeTable::from_raw(raw);
        let n_overrides = read_u32(r)?;
        for _ in 0..n_overrides {
            let cp = read_u32(r)?;
            let cat = read_u8(r)?;
            if let Some(c) = ntex_core::Catcode::from_u8(cat) {
                table.set_codepoint(cp, c);
            }
        }
        tables.push(Some(table));
    }
    Ok(tables)
}

fn write_engine_version(w: &mut impl Write) -> io::Result<()> {
    let bytes = ENGINE_VERSION.as_bytes();
    let len = u16::try_from(bytes.len()).map_err(|_| invalid("引擎版本号过长"))?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(bytes)
}

fn read_and_check_engine_version(r: &mut impl Read) -> io::Result<()> {
    let len = read_u16(r)? as usize;
    debug_assert!(len <= PREALLOC_CAP, "u16 长度域被 PREALLOC_CAP 覆盖");
    let mut bytes = vec![0u8; len];
    r.read_exact(&mut bytes)?;
    let file_version = String::from_utf8(bytes).map_err(|_| invalid(".fmt 引擎版本号非 UTF-8"))?;
    if file_version != ENGINE_VERSION {
        return Err(invalid(&format!(
            ".fmt 引擎版本不匹配：文件={file_version}，当前={ENGINE_VERSION}；请用 --generate-fmt 重新 dump"
        )));
    }
    Ok(())
}

// ---------- 编码原语（include! 嵌入） ----------
include!("codec.rs");

#[cfg(test)]
mod tests {
    use super::*;
    use ntex_core::expand::Expander;

    /// panic 审计守护（债务表项 4）：生产路径（`#[cfg(test)]` 之前的源码，
    /// 含 include! 进来的 codec.rs）禁 `.unwrap()`——损坏 .fmt 必须报 io::Error
    /// 而非 panic（TeX 引擎永不 panic；不可达位用 `.expect("不变量")`）。
    #[test]
    fn production_code_has_no_unwrap() {
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
            .expect("源文件应可读");
        let prod = src.split("#[cfg(test)]").next().expect("应有测试分界");
        for (i, line) in prod.lines().enumerate() {
            assert!(
                !line.contains(".unwrap()"),
                "生产代码出现 .unwrap()（lib.rs 第 {} 行）：{line}",
                i + 1
            );
        }
    }

    /// 损坏 .fmt 的合法文件头前缀（魔数 + 版本 + 引擎版本号）。
    fn corrupt_header() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(MAGIC);
        buf.push(FORMAT_VERSION);
        buf.extend_from_slice(&(ENGINE_VERSION.len() as u16).to_le_bytes());
        buf.extend_from_slice(ENGINE_VERSION.as_bytes());
        buf
    }

    #[test]
    fn load_huge_count_fails_fast_not_oom() {
        // 恶意/损坏 .fmt：intern 表计数填 2^32-1 → 预分配钳制后立刻 EOF 优雅
        // 报错，而非按值申请数 GB（容量溢出 panic / OOM abort）
        let mut buf = corrupt_header();
        buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        let err = load(&mut buf.as_slice()).unwrap_err();
        assert!(
            !err.to_string().contains("越界"),
            "计数越界应走 EOF 而非显式检查：{err}"
        );
    }

    #[test]
    fn load_oversized_name_length_rejected() {
        // 名字长度域填 2^32-1 → 显式 invalid 错（不再 vec![0u8; len] 整块分配）
        let mut buf = corrupt_header();
        buf.extend_from_slice(&1u32.to_le_bytes()); // n_names = 1
        buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // 名字长度
        let err = load(&mut buf.as_slice()).unwrap_err();
        assert!(
            err.to_string().contains("名字长度越界"),
            "应报名字长度越界：{err}"
        );
    }

    #[test]
    fn load_huge_eqtb_slots_fails_fast() {
        // eqtb 槽计数越界：EqSlot 非零尺寸，按值预分配会容量溢出 panic
        let mut buf = corrupt_header();
        buf.extend_from_slice(&0u32.to_le_bytes()); // n_names = 0
        buf.extend_from_slice(&[0u8; 256]); // catcode 表
        buf.extend_from_slice(&0u32.to_le_bytes()); // catcode 覆盖表
        for _ in 0..256 {
            buf.extend_from_slice(&0u32.to_le_bytes()); // sfcodes
        }
        buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // n_slots
        let err = load(&mut buf.as_slice()).unwrap_err();
        assert!(
            !err.to_string().contains("capacity overflow"),
            "不应容量溢出：{err}"
        );
    }

    fn roundtrip(state: &FmtState) -> FmtState {
        let mut buf = Vec::new();
        save(&mut buf, state).unwrap();
        let mut cur = buf.as_slice();
        let loaded = load(&mut cur).unwrap();
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
            e.run_source(c)
                .unwrap_or_else(|err| panic!("段 {i} [{c}] 失败：{err}"));
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

    /// v22：fontdimen/hyphenchar 覆盖表随快照往返（l3kernel intarray 模拟
    /// 字体的条目与 count；fmt 生成期创建的四个常量 cctab 恢复后读回全 0
    /// /45 的根因回归锁——空表「写 0 个 + 读 0 个」也自洽，须**非空**）。
    #[test]
    fn fmt_roundtrip_preserves_fontdimen_and_hyphenchar_overrides() {
        let mut state = sample_state();
        // 模拟 fmt 生成期的 intarray 字体：1 号字体 count=257、条目若干
        state.hyphenchars.push((1, 257));
        state.fontdimens.push((1, 66, 11));
        state.fontdimens.push((1, 257, 13));
        state.fontdimens.push((2, 3, -7));
        let loaded = roundtrip(&state);
        assert_eq!(loaded.hyphenchars, vec![(1, 257)], "hyphenchar 覆盖应还原");
        assert_eq!(
            loaded.fontdimens,
            vec![(1, 66, 11), (1, 257, 13), (2, 3, -7)],
            "fontdimen 覆盖应逐条还原（含负值）"
        );
    }

    /// v15：>255 码位覆盖表随快照往返（save/load 字节流对称性的回归锁）。
    ///
    /// 覆盖表为空时「写 0 个 + 读 0 个」也能自洽，故须**非空**才真正锁住
    /// 对称性——空表版本在 save 漏写 `n_overrides` 时仍会通过。
    #[test]
    fn fmt_roundtrip_preserves_unicode_catcode_overrides() {
        use ntex_core::catcode::Catcode;
        let mut state = sample_state();
        state.catcodes.set_codepoint(0xFF0C, Catcode::Active); // 全角逗号
        state.catcodes.set_codepoint(0x4E2D, Catcode::MathShift);
        let loaded = roundtrip(&state);
        assert_eq!(loaded.catcodes, state.catcodes, "覆盖表应逐条还原");
        assert_eq!(
            loaded.catcodes.get_codepoint(0xFF0C),
            Catcode::Active,
            "还原后查表须命中覆盖"
        );
        // 空表对照：仍应往返成功（覆盖表为增量的可选段）
        let empty = sample_state();
        assert_eq!(empty.catcodes.unicode_overrides().count(), 0);
        assert_eq!(roundtrip(&empty).catcodes, empty.catcodes);
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
    fn fmt_rejects_wrong_engine_version() {
        let state = sample_state();
        let mut buf = Vec::new();
        save(&mut buf, &state).unwrap();
        let version_pos = 8;
        assert_eq!(buf[version_pos], FORMAT_VERSION);
        let len_pos = 9;
        let len = u16::from_le_bytes([buf[len_pos], buf[len_pos + 1]]) as usize;
        let start = len_pos + 2;
        assert_eq!(&buf[start..start + len], ENGINE_VERSION.as_bytes());
        buf[start] = match buf[start] {
            b'0' => b'1',
            _ => b'0',
        };
        let err = load(&mut buf.as_slice()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("引擎版本不匹配") && msg.contains("--generate-fmt"),
            "错误消息应指明重 dump：{msg}"
        );
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
