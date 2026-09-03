//! 段边界状态快照 + 段级依赖记录（M5 阶段二，plan.md §7"依赖追踪"）。
//!
//! 失效判定的三层信息都在这里：
//! - [`StateSnapshot`]：段执行前/后的引擎状态——可还原的完整检查点
//!   （值状态精确比较 + eqtb 槽级比较 + 回滚用真值/控制状态）；
//! - [`ChainDelta`]：活状态相对旧缓存链的偏差（一次全量扫描，各段判定只查
//!   偏差集 ∩ 依赖，免每段全状态比较）；
//! - [`SegmentDeps`]：该段**读过**哪些 cs（词法超近似）与**写过**哪些 cs，
//!   以及缓存时各读依赖的宏槽版本。

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use crate::eqtb::EqSlot;
use crate::expand::{EngineCheckpoint, Expander, ValueState};
use crate::intern::InternTable;

/// 段边界引擎状态快照（缓存失效判定的基准）。
///
/// **轻量**是硬约束：寄存器文件 32768×5 槽整份克隆 ~3MB，逐段存完整快照不可行，
/// 故寄存器只带 [`crate::register::Registers::dirty`] 影子表（已写槽的精确值，
/// 未写槽恒为零值）、其余 `.fmt` 未覆盖字段走指纹（比较）+ 真值（还原）。
///
/// 阶段二起快照**可还原**：`EngineCheckpoint` 额外携带指纹覆盖字段的真实值与
/// 控制状态，`edit` 借此把引擎整体回滚到段前（不再重建引擎）；比较口径不变
/// （[`ValueState::PartialEq`] + eqtb 逐槽语义相等）。
#[derive(Debug, Clone)]
pub struct StateSnapshot {
    /// 完整检查点（eqtb 槽 + 值状态 + 指纹字段真值 + 控制状态）。
    pub(crate) cp: EngineCheckpoint,
}

impl StateSnapshot {
    pub fn capture(e: &Expander) -> Self {
        Self {
            cp: e.capture_checkpoint(),
        }
    }

    /// 值状态（比较口径；[`ValueState::PartialEq`] 含运行时指纹）。
    pub(crate) fn value(&self) -> &ValueState {
        &self.cp.value
    }

    /// eqtb 全部槽（浅拷贝：宏体 `Arc` 共享；槽级比较支持"只失效受影响段"）。
    pub(crate) fn eqtb(&self) -> &[EqSlot] {
        &self.cp.eqtb
    }

    /// 整体还原检查点（M5 阶段三排版层管线的跨 crate 回滚入口——检查点字段
    /// 对外不可见，还原必须经此方法）。
    pub fn restore_to(&self, e: &mut Expander) {
        e.restore_checkpoint(&self.cp);
    }
}

/// 当前状态是否与快照语义一致（值状态精确相等 + eqtb 逐槽语义相等）。
///
/// 阶段二 `edit` 用它做回滚后的完整性校验（防御检查点字段遗漏）；阶段三
/// 排版层管线（ntex-layout）用它做"缓存段可否免执行"的边界判定——排版层
/// 没有词法依赖可用，同步态（活状态与段前快照精确一致）是它唯一可判的口径。
/// 阶段二重放中的失效判定仍走 [`ChainDelta`]（一次全量扫描 + 各段依赖检查）。
pub fn state_matches(value: &ValueState, eqtb: &[EqSlot], snap: &StateSnapshot) -> bool {
    value == snap.value() && slots_semantically_equal(eqtb, snap.eqtb())
}

/// 活状态相对旧缓存链的偏差（一次全量扫描的产物，供后续各段廉价判定）。
///
/// 编辑重算只改链上一个点：重放首个执行段之后，"活状态 vs 旧链"的偏差就固定
/// 为一个小集合（通常 = 被编辑宏所在槽）。以它做各段失效判定，免去阶段一
/// "每段一次全状态比较"（值指纹 + eqtb 全槽深比较——500 段基准的主要开销）。
/// 偏差为空 = 活状态与旧链精确一致（同步态，缓存段免判定）。
#[derive(Debug, Clone)]
pub(crate) struct ChainDelta {
    /// 值状态有偏差（寄存器/参数/catcode/编码表/流——不做槽级归因，全局失效）。
    pub(crate) value_differs: bool,
    /// 语义有偏差的 eqtb 槽（含槽规模变化时多出的尾部槽）。
    pub(crate) slots: HashSet<u32>,
}

impl ChainDelta {
    /// 空偏差（同步态：活状态与参照状态精确一致）。
    pub(crate) fn empty() -> Self {
        Self {
            value_differs: false,
            slots: HashSet::new(),
        }
    }

    /// 活引擎状态相对 `reference`（旧链上"下一段执行前"的状态）的偏差。
    ///
    /// 槽规模不一致（段间新驻留 cs / 回滚截断）时，超出共同前缀的槽一律视为
    /// 变化——"未驻留"即 `Undefined`，与已有槽不等。
    pub(crate) fn capture(e: &Expander, reference: &StateSnapshot) -> Self {
        let value_differs = e.value_state() != *reference.value();
        let cur = e.eqtb().slots();
        let snap = reference.eqtb();
        let common = cur.len().min(snap.len());
        let mut slots: HashSet<u32> = HashSet::new();
        for (csid, (a, b)) in cur[..common].iter().zip(&snap[..common]).enumerate() {
            if !slot_semantically_equal(a, b) {
                slots.insert(csid as u32);
            }
        }
        for csid in common..cur.len().max(snap.len()) {
            slots.insert(csid as u32);
        }
        Self {
            value_differs,
            slots,
        }
    }

    /// 是否无偏差（值状态一致 + 无变化槽）。
    pub(crate) fn is_empty(&self) -> bool {
        !self.value_differs && self.slots.is_empty()
    }
}

/// 段级依赖记录。
///
/// 词法**超近似**：只多不少。漏报会导致缓存错（正确性铁律不允许），多报只会
/// 多算几个段（性能损失）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SegmentDeps {
    /// 段内词法引用的控制序列名（含定义命令自身；注释里的引用也计入——多报无害）。
    pub read_cs: BTreeSet<String>,
    /// 段内词法定义/赋值的控制序列名（`\def`/`\let`/`\countdef`/`\font` 等定义
    /// 命令后的第一个 cs；宏体内的嵌套定义也计入——保守多报）。
    pub write_cs: BTreeSet<String>,
    /// 段内出现 `\csname`：cs 名可动态构造 → 视为读**任意**槽（保守失效）。
    pub dynamic_cs: bool,
    /// 缓存时各读依赖的宏槽版本（未驻留 / 非宏槽为 `None`；诊断与报告用——
    /// 失效判定走 [`StateSnapshot`] 的槽比较，不依赖这里）。
    pub read_versions: BTreeMap<String, Option<u64>>,
}

/// 定义类命令：其后第一个控制序列是**写**目标（`\let\a=\b` 的 `\a` 等）。
const DEFINING_CS: &[&str] = &[
    "def",
    "gdef",
    "edef",
    "xdef",
    "let",
    "futurelet",
    "chardef",
    "mathchardef",
    "countdef",
    "dimendef",
    "skipdef",
    "muskipdef",
    "toksdef",
    "font",
];

/// 词法提取段内控制序列的读/写依赖（不展开、不看引擎）。
pub(crate) fn lex_deps(source: &str) -> SegmentDeps {
    let mut deps = SegmentDeps::default();
    let bytes = source.as_bytes();
    // pending_def：刚见过定义类命令 → 下一个 cs 名是写目标
    let mut pending_def = false;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        let start = i + 1;
        if start >= bytes.len() {
            break;
        }
        let end = if bytes[start].is_ascii_alphabetic() {
            let mut j = start;
            while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
                j += 1;
            }
            j
        } else {
            start + 1 // 控制符 cs（单字节名字）
        };
        // cs 名可含非字母字符（catcode 11 的 @/数字等，如 `\foo@bar`）——文本层
        // 无法知 catcode，保守**延伸到定界符**（空白/组括号/反斜杠/换行）取整段名；
        // 同时保留字母前缀（真名可能就是前缀：@ 非 letter 时 `\def\foo@bar` 定义
        // 的是 \foo）。双记只多不少（超近似）；漏记会缓存错（tmp 测试实证：
        // 编辑 \foo@bar 定义后引用段错误复用旧输出）。
        let mut end2 = end;
        while end2 < bytes.len()
            && !matches!(
                bytes[end2],
                b' ' | b'\t' | b'\r' | b'\n' | b'{' | b'}' | b'\\' | b'%' | b'\0'
            )
        {
            end2 += 1;
        }
        // 候选名集合：字母前缀（`\foo@bar` 的真名可能是 foo）+ 整段扩展名
        // （可能是 foo@bar）——文本层不知 catcode，双记只多不少（超近似）。
        let name = std::str::from_utf8(&bytes[start..end]).unwrap_or_default();
        let full = if end2 > end {
            std::str::from_utf8(&bytes[start..end2]).unwrap_or_default()
        } else {
            ""
        };
        // 处理顺序：
        // ① 定义目标（pending_def）：候选名全部记**写**，清位；
        // ② 定义类命令自身（读 + 置 pending_def，其后的 cs 才是写目标）；
        // ③ 其余（含 \csname）记**读**。
        let mut targets: Vec<&str> = Vec::new();
        if !full.is_empty() {
            targets.push(full);
            if full != name && !name.is_empty() {
                targets.push(name);
            }
        } else if !name.is_empty() {
            targets.push(name);
        }
        if pending_def {
            for t in &targets {
                deps.write_cs.insert(t.to_string());
            }
            pending_def = false;
        } else if targets.iter().any(|t| DEFINING_CS.contains(t)) {
            for t in &targets {
                deps.read_cs.insert(t.to_string());
            }
            pending_def = true;
        } else {
            for t in &targets {
                if *t == "csname" {
                    deps.dynamic_cs = true;
                } else {
                    deps.read_cs.insert(t.to_string());
                }
            }
        }
        // 指针推进到整段名末尾（扩展名存在时其字符是宏名的一部分，不再作文本扫）
        i = end2;
    }
    deps
}

impl SegmentDeps {
    /// 解析读依赖的宏槽版本（`Expander::eqtb` 的版本化槽，RFC-1 §5）。
    pub(crate) fn attach_versions(
        mut self,
        intern: &InternTable,
        eqtb: &crate::eqtb::Eqtb,
    ) -> Self {
        for name in &self.read_cs {
            let v = intern
                .lookup(name)
                .and_then(|csid| eqtb.version_of(csid))
                .map(|v| v.get());
            self.read_versions.insert(name.clone(), v);
        }
        self
    }
}

/// 槽的**语义**相等：宏槽只比内容（参数规格 + 宏体），不比版本号。
///
/// 版本号是"改必增"的痕迹——`\def\x{same}` 这类内容不变的重定义也会 bump。
/// 内容不变则后续任何段的输出都不可能变化，依赖判定必须按内容比，才能把
/// 这种空转重定义识别为"未失效"（`MacroDef::PartialEq` 本就只比参数规格与
/// 宏体、不比字节码，即语义相等）。
///
/// 快路径：两份快照/活状态里的宏体多为同一 `Arc`（快照浅拷贝共享），指针相等
/// 即内容相等——免去逐 token 深比较。这是每次失效判定的主要开销来源（500 段
/// 文档一轮判定 = 500 × 全槽扫描），`Arc` 的 `PartialEq` 会退化为解引用深比较。
pub(crate) fn slot_semantically_equal(a: &EqSlot, b: &EqSlot) -> bool {
    match (a, b) {
        (EqSlot::Macro(x), EqSlot::Macro(y)) => {
            Arc::ptr_eq(&x.value, &y.value) || x.value == y.value
        }
        _ => a == b,
    }
}

/// 槽序列的语义相等（长度 + 逐槽 [`slot_semantically_equal`]）。
pub(crate) fn slots_semantically_equal(a: &[EqSlot], b: &[EqSlot]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| slot_semantically_equal(x, y))
}

/// **写集**是否触及偏差槽。
///
/// 段要写的槽若已被动过，跳过该段执行会让"本应重写的值"停留在被改后的值，
/// 后续读它的段就会拿到错值（见 `engine` 失效判定分支 2）。
pub(crate) fn writes_touch_slots(
    intern: &InternTable,
    names: &BTreeSet<String>,
    slots: &HashSet<u32>,
) -> bool {
    names
        .iter()
        .filter_map(|n| intern.lookup(n))
        .any(|csid| slots.contains(&csid))
}

/// 读依赖闭包是否触及**偏差槽**。
///
/// 闭包 = 段内词法引用的 cs → 其宏体内引用的 cs（逐层展开）→ `\let` 别名指向
/// 的 cs。沿**当前**宏体遍历保证健全：闭包里某个宏的槽在偏差集里，访问到它
/// 时立即返回；不在偏差集里则其宏体与缓存时一致，继续遍历不漏。
pub(crate) fn reads_touch_slots(
    intern: &InternTable,
    eqtb: &[EqSlot],
    deps: &SegmentDeps,
    slots: &HashSet<u32>,
) -> bool {
    let csname_csid = intern.lookup("csname");
    let mut visited: HashSet<u32> = HashSet::new();
    let mut queue: Vec<u32> = deps
        .read_cs
        .iter()
        .filter_map(|n| intern.lookup(n))
        .collect();
    while let Some(csid) = queue.pop() {
        if !visited.insert(csid) {
            continue;
        }
        if slots.contains(&csid) {
            return true;
        }
        match eqtb.get(csid as usize) {
            Some(EqSlot::Macro(v)) => {
                // `\csname` 可在运行期构造任意 cs 名 → 保守失效
                if let Some(csn) = csname_csid {
                    if v.value.body.iter().any(|t| t.csid() == Some(csn)) {
                        return true;
                    }
                }
                for t in v.value.body.iter() {
                    if let Some(c) = t.csid() {
                        queue.push(c);
                    }
                }
            }
            // `\let\a\b`：读 \a 即读 \b 的当前含义
            Some(EqSlot::Alias(target)) => queue.push(*target),
            // 寄存器引用/字体/流：值变化走 ValueState（寄存器影子表、字体/流指纹）
            _ => {}
        }
    }
    false
}
