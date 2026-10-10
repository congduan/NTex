//! 寄存器文件与内部量（M1-10）。
//!
//! - `\count`：256 个整数
//! - `\dimen`：256 个 scaled point（1pt = 65536sp）
//! - `\skip`：256 个胶水（width + stretch + shrink）
//! - `\muskip`：256 个 mu 胶水（1mu = 65536 单位；ETRIP，pdfTeX 实测 \mutoglue/\gluetomu 1:1）
//! - `\toks`：256 个 token 列表
//!
//! 单位换算与 `\the` 输出格式均**对照真实 pdfTeX 实测校准**（见函数注释），
//! 精确逐位一致性留待 M3（TRIP）验证。

use crate::macrodef::TokenArray;
use std::collections::BTreeMap;
use std::sync::Arc;

/// 寄存器个数（eTeX 扩展：0..=32767；经典 TeX 为 0..=255）。
pub const REGISTER_COUNT: usize = 32768;

/// 1 pt = 65536 sp。
pub const SP_PER_PT: i64 = 65_536;

/// TeX null_flag：尺寸\"未定\"哨兵（tex.web `null_flag=-2^30`；showbox 显示 `*`，
/// 如 TRIP `\leaders\hrule\hskip10pt` 的 rule width）。
pub const NULL_FLAG: i64 = -1_073_741_824;

/// TeX max_int：整数范围 |v| ≤ 0x7FFFFFFF（e-TeX 表达式超限 → "! Arithmetic overflow."）。
pub const MAX_INT: i64 = 0x7FFF_FFFF;
/// TeX max_dimen：尺寸范围 |v| ≤ 0x3FFFFFFF（scan_dimen 超限 → "! Dimension too large."）。
pub const MAX_DIMEN: i64 = 0x3FFF_FFFF;

/// 胶水：宽度 + 拉伸 + 收缩（单位 sp）+ 无穷阶（TeX glue_ord：0=普通、1=fil、2=fill、3=filll）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glue {
    pub width: i64,
    pub stretch: i64,
    pub shrink: i64,
    /// 拉伸无穷阶（`\gluestretchorder` 读；`plus 3fil` 解析）。
    pub stretch_order: u8,
    /// 收缩无穷阶（`\glueshrinkorder` 读；`minus 0fill` 解析）。
    pub shrink_order: u8,
}

/// 无穷阶常量（TeX glue_ord）。
pub mod order {
    /// 普通（无阶）。
    pub const NORMAL: u8 = 0;
    /// `fil`。
    pub const FIL: u8 = 1;
    /// `fill`。
    pub const FILL: u8 = 2;
    /// `filll`。
    pub const FILLL: u8 = 3;
}

impl Glue {
    pub const ZERO: Glue = Glue {
        width: 0,
        stretch: 0,
        shrink: 0,
        stretch_order: 0,
        shrink_order: 0,
    };

    /// 无阶胶水（width/stretch/shrink；`..Glue::ZERO` 结构更新可覆盖 order）。
    pub const fn new(width: i64, stretch: i64, shrink: i64) -> Glue {
        Glue {
            width,
            stretch,
            shrink,
            stretch_order: 0,
            shrink_order: 0,
        }
    }

    /// 三分量取负（tex.web scan_something_internal 的 negative 对 glue 级量语义：
    /// `\skip=-\muskip1` 等负号作用于整个胶水；无穷阶不变——分量负值 + 原阶
    /// 由显示层输出 `plus -2.0fil` 形态，与参考 etrip.log 一致）。
    pub fn negated(self) -> Glue {
        Glue {
            width: -self.width,
            stretch: -self.stretch,
            shrink: -self.shrink,
            ..self
        }
    }
}

/// 胶水加法（TeX `\advance` 语义）：宽度直接相加；拉伸/收缩取阶更高者的值，
/// 阶相同则分量相加（tex.web 的 glue 组合规则）。
pub fn add_glue(a: Glue, b: Glue) -> Glue {
    let (stretch, stretch_order) = match (a.stretch_order, b.stretch_order) {
        (ao, bo) if ao == bo => (a.stretch + b.stretch, ao),
        (ao, bo) if ao > bo => (a.stretch, ao),
        (_, bo) => (b.stretch, bo),
    };
    let (shrink, shrink_order) = match (a.shrink_order, b.shrink_order) {
        (ao, bo) if ao == bo => (a.shrink + b.shrink, ao),
        (ao, bo) if ao > bo => (a.shrink, ao),
        (_, bo) => (b.shrink, bo),
    };
    Glue {
        width: a.width + b.width,
        stretch,
        shrink,
        stretch_order,
        shrink_order,
    }
}

/// 单位 → sp 换算。
///
/// 实测 pdfTeX 输出：`\dimen0=1in\the\dimen0` → "72.26999pt" 等，
/// 反推存储值：pt=65536, sp=1, bp=65781, in=4736286, cm=1864679, mm=186467。
pub fn unit_to_sp(unit: &str) -> Option<i64> {
    match unit {
        "sp" => Some(1),
        "pt" => Some(SP_PER_PT),
        "bp" => Some(65_781),
        "in" => Some(4_736_286),
        "cm" => Some(1_864_679),
        "mm" => Some(186_467),
        // TRIP：pica（1pc = 12pt）+ cicero（1cc = 12pt × 1157/1236 ≈ 736166sp）
        "pc" => Some(12 * SP_PER_PT),
        "cc" => Some(736_166),
        // didot（1dd = 1157/1238 pt ≈ 61347sp）；TRIP L331 `\halign spread-12.truedd`
        "dd" => Some(61_347),
        _ => None,
    }
}

/// 寄存器类别（`\count`/`\dimen`/`\skip`/`\muskip`/`\toks`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegKind {
    Count,
    Dimen,
    Skip,
    Muskip,
    Toks,
}

/// 寄存器文件（eTeX 32768 槽；堆分配 Box 切片，避免栈上 ~2MB 数组）。
#[derive(Debug, Clone)]
pub struct Registers {
    counts: Box<[i64]>,
    dimens: Box<[i64]>,
    skips: Box<[Glue]>,
    muskip: Box<[Glue]>,
    toks: Box<[TokenArray]>,
    /// 已写入槽位的影子表（M5 增量，plan.md §7）。寄存器文件 32768×5 槽，
    /// 全量克隆/比较每段 ~3MB 不可行；只记录被 `set_*` 写过的槽，增量快照
    /// 携带此表即可**精确**比较——未写入槽恒为零值，无需逐槽比对。
    /// 键 = (RegKind 编号, 槽下标)，BTreeMap 保证快照顺序确定（可相等比较）。
    dirty: BTreeMap<(u8, usize), RegisterValue>,
}

/// 寄存器槽的值（M5 增量影子表用；与槽类型一一对应）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterValue {
    /// count / dimen（整数，sp）。
    Int(i64),
    /// skip / muskip（胶）。
    Glue(Glue),
    /// toks（token 列表）。
    Toks(TokenArray),
}

impl Default for Registers {
    fn default() -> Self {
        Self::new()
    }
}

impl Registers {
    pub fn new() -> Self {
        Self {
            counts: vec![0; REGISTER_COUNT].into_boxed_slice(),
            dimens: vec![0; REGISTER_COUNT].into_boxed_slice(),
            skips: vec![Glue::ZERO; REGISTER_COUNT].into_boxed_slice(),
            muskip: vec![Glue::ZERO; REGISTER_COUNT].into_boxed_slice(),
            toks: vec![Arc::from([]); REGISTER_COUNT].into_boxed_slice(),
            dirty: BTreeMap::new(),
        }
    }

    /// 已写入槽位的影子表（M5 增量：失效判定精确比较用）。
    pub fn dirty(&self) -> &BTreeMap<(u8, usize), RegisterValue> {
        &self.dirty
    }

    pub fn count(&self, idx: usize) -> i64 {
        self.counts[idx]
    }

    pub fn set_count(&mut self, idx: usize, v: i64) {
        self.counts[idx] = v;
        self.dirty.insert((0, idx), RegisterValue::Int(v));
    }

    pub fn dimen(&self, idx: usize) -> i64 {
        self.dimens[idx]
    }

    pub fn set_dimen(&mut self, idx: usize, v: i64) {
        self.dimens[idx] = v;
        self.dirty.insert((1, idx), RegisterValue::Int(v));
    }

    pub fn skip(&self, idx: usize) -> Glue {
        self.skips[idx]
    }

    pub fn set_skip(&mut self, idx: usize, v: Glue) {
        self.skips[idx] = v;
        self.dirty.insert((2, idx), RegisterValue::Glue(v));
    }

    pub fn muskip(&self, idx: usize) -> Glue {
        self.muskip[idx]
    }

    pub fn set_muskip(&mut self, idx: usize, v: Glue) {
        self.muskip[idx] = v;
        self.dirty.insert((3, idx), RegisterValue::Glue(v));
    }

    pub fn toks(&self, idx: usize) -> TokenArray {
        self.toks[idx].clone()
    }

    pub fn set_toks(&mut self, idx: usize, v: TokenArray) {
        self.toks[idx] = Arc::clone(&v);
        self.dirty.insert((4, idx), RegisterValue::Toks(v));
    }

    /// 从影子表还原（M5 阶段二段级回滚）：未写槽归零、已写槽按影子表重放。
    ///
    /// 寄存器文件 32768×5 槽整份入快照 ~3MB 不可行，快照只携带 [`Self::dirty`]；
    /// 未写入槽恒为零值（`ValueState` 比较口径的前提），故还原 = 整份复位后按
    /// 影子表重放，结果与"从未写过这些槽再逐个赋值"精确一致。类别号与上方
    /// `set_*` 的 insert 键一致（0=count/1=dimen/2=skip/3=muskip/4=toks）。
    pub fn restore_dirty(&mut self, dirty: &BTreeMap<(u8, usize), RegisterValue>) {
        *self = Self::new();
        for (&(kind, idx), value) in dirty {
            match (kind, value) {
                (0, RegisterValue::Int(v)) => self.set_count(idx, *v),
                (1, RegisterValue::Int(v)) => self.set_dimen(idx, *v),
                (2, RegisterValue::Glue(v)) => self.set_skip(idx, *v),
                (3, RegisterValue::Glue(v)) => self.set_muskip(idx, *v),
                (4, RegisterValue::Toks(v)) => self.set_toks(idx, v.clone()),
                _ => {} // 类别号与影子表同源产生，不可达
            }
        }
    }
}

/// 寄存器状态快照（`.fmt` v1 序列化载体）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterState {
    pub counts: Box<[i64]>,
    pub dimens: Box<[i64]>,
    pub skips: Box<[Glue]>,
    /// 非空 `\muskip` 项（(下标, 内容)；ETRIP）。
    pub muskip: Vec<(usize, Glue)>,
    /// 非空 `\toks` 项（(下标, 内容)）。
    pub toks: Vec<(usize, TokenArray)>,
}

impl Registers {
    pub fn export(&self) -> RegisterState {
        RegisterState {
            counts: self.counts.clone(),
            dimens: self.dimens.clone(),
            skips: self.skips.clone(),
            muskip: self
                .muskip
                .iter()
                .enumerate()
                .filter(|(_, g)| **g != Glue::ZERO)
                .map(|(i, g)| (i, *g))
                .collect(),
            toks: self
                .toks
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_empty())
                .map(|(i, t)| (i, t.clone()))
                .collect(),
        }
    }

    pub fn import(state: RegisterState) -> Self {
        let mut r = Self::new();
        r.counts = state.counts;
        r.dimens = state.dimens;
        r.skips = state.skips;
        for (i, g) in state.muskip {
            r.muskip[i] = g;
        }
        for (i, t) in state.toks {
            r.toks[i] = t;
        }
        r
    }
}

/// 整数 → `\the` 输出（十进制）。
pub fn format_count(v: i64) -> String {
    v.to_string()
}

/// scaled → pt 字符串（对照 pdfTeX 的 print_scaled，tex.web §103 逐行移植）：
/// `s:=10*(s mod unity)+5; delta:=10; repeat [delta>unity 时 s+=32768-50000
/// （第 5 位舍入）] 打 `s div unity` 位；`s:=10*(s mod unity); delta*=10;
/// until s<=delta`——余数追上 10^k 即停（尾零自然消隐，至少留 1 位）。
///
/// 此前「真十进制四舍五入到 5 位」把 39322sp（=0.6\p@ 的 tex.web 精确值）
/// 打成 "0.60001pt"（GT "0.6pt"）——`\the` 产物喂 xcolor `\rshift@` 的
/// `##1.##2PT` 定界匹配时多出的 `1` 直接进 ##2（刀E 第三面）。
///
/// 实测样例（pdfTeX GT 逐位对照）：1pt→"1.0"、2.5pt→"2.5"、-3.125pt→"-3.125"、
/// 1in→"72.26999"、1sp→"0.00002"、39321sp→"0.59999"、39322sp→"0.6"。
pub fn format_dimen(scaled: i64) -> String {
    let mut out = String::new();
    let mag = scaled.unsigned_abs() as i128;
    if scaled < 0 {
        out.push('-');
    }
    let unity: i128 = 65_536;
    out.push_str(&(mag / unity).to_string());
    out.push('.');
    let mut s: i128 = 10 * (mag % unity) + 5;
    let mut delta: i128 = 10;
    loop {
        if delta > unity {
            s += 32_768 - 50_000; // round the last digit
        }
        out.push(char::from(b'0' + (s / unity) as u8));
        s = 10 * (s % unity);
        delta *= 10;
        if s <= delta {
            break;
        }
    }
    out
}

/// 无穷阶后缀（TeX `\the\skip` 显示：plus 3fil 等）。
fn order_suffix(order: u8) -> &'static str {
    match order {
        1 => "fil",
        2 => "fill",
        3 => "filll",
        _ => "",
    }
}

/// 胶水分量显示：阶为 0 → "<值><单位>"（如 "2.0pt"）；阶非 0 → "<值><阶>"（如 "3.0fil"）。
fn glue_part(value: i64, order: u8, unit: &str) -> String {
    if order == 0 {
        format!("{}{}", format_dimen(value), unit)
    } else {
        format!("{}{}", format_dimen(value), order_suffix(order))
    }
}

/// 胶水 → `\the` 输出（"1.0pt plus 2.0pt minus 0.5pt"，零值分量省略；
/// 非零阶在分量后附 fil/fill/filll）。
/// TeX print_glue：`(order < normal) or (d <> 0)` 才显示分量——
/// 值 0（即使带 fil 阶，如 `1pt minus 0.0fil`）不显示。
pub fn format_glue(g: Glue) -> String {
    let mut out = format!("{}pt", format_dimen(g.width));
    if g.stretch != 0 {
        out.push_str(&format!(
            " plus {}",
            glue_part(g.stretch, g.stretch_order, "pt")
        ));
    }
    if g.shrink != 0 {
        out.push_str(&format!(
            " minus {}",
            glue_part(g.shrink, g.shrink_order, "pt")
        ));
    }
    out
}

/// mu 胶水 → `\the` 输出（"1.0mu plus 2.0mu minus 0.5mu"；ETRIP）。
/// pdfTeX 实测：mu 值定点存储（1mu = 65536 单位），数值直通。
pub fn format_mu_glue(g: Glue) -> String {
    let mut out = format!("{}mu", format_dimen(g.width));
    if g.stretch != 0 {
        out.push_str(&format!(
            " plus {}",
            glue_part(g.stretch, g.stretch_order, "mu")
        ));
    }
    if g.shrink != 0 {
        out.push_str(&format!(
            " minus {}",
            glue_part(g.shrink, g.shrink_order, "mu")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_conversions_match_tex() {
        assert_eq!(unit_to_sp("pt"), Some(65_536));
        assert_eq!(unit_to_sp("sp"), Some(1));
        assert_eq!(unit_to_sp("bp"), Some(65_781));
        assert_eq!(unit_to_sp("in"), Some(4_736_286));
        assert_eq!(unit_to_sp("cm"), Some(1_864_679));
        assert_eq!(unit_to_sp("mm"), Some(186_467));
        assert_eq!(unit_to_sp("em"), None);
    }

    #[test]
    fn dimen_formatting_matches_tex() {
        assert_eq!(format_dimen(65_536), "1.0");
        assert_eq!(format_dimen(163_840), "2.5");
        assert_eq!(format_dimen(-204_800), "-3.125");
        assert_eq!(format_dimen(4_736_286), "72.26999");
        assert_eq!(format_dimen(1), "0.00002");
        assert_eq!(format_dimen(0), "0.0");
    }

    #[test]
    fn glue_formatting_matches_tex() {
        let g = Glue::new(65_536, 131_072, 32_768);
        assert_eq!(format_glue(g), "1.0pt plus 2.0pt minus 0.5pt");
        assert_eq!(format_glue(Glue::ZERO), "0.0pt");
        // 无穷阶显示：plus/minus 后跟 fil/fill/filll
        let g = Glue {
            stretch_order: 1,
            ..Glue::new(65_536, 131_072, 0)
        };
        assert_eq!(format_glue(g), "1.0pt plus 2.0fil");
        let g = Glue {
            shrink_order: 3,
            ..Glue::new(0, 0, 32_768)
        };
        assert_eq!(format_glue(g), "0.0pt minus 0.5filll");
    }

    #[test]
    fn registers_default_zeroed() {
        let r = Registers::new();
        assert_eq!(r.count(0), 0);
        assert_eq!(r.dimen(3), 0);
        assert_eq!(r.skip(5), Glue::ZERO);
        assert!(r.toks(7).is_empty());
    }

    #[test]
    fn registers_set_get() {
        let mut r = Registers::new();
        r.set_count(1, 42);
        r.set_dimen(2, 65_536);
        r.set_skip(3, Glue::new(1, 2, 3));
        assert_eq!(r.count(1), 42);
        assert_eq!(r.dimen(2), 65_536);
        assert_eq!(r.skip(3), Glue::new(1, 2, 3));
    }
}
