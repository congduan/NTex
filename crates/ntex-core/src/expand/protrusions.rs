// 字符凸出量表（pdfTeX \rpcode/\lpcode 的存储）：`(font_id, char) → 值`。
//
// pdfTeX 语义（texmf pdftex manual §3.4.2 / GT 对拍 2026-10-11）：
// - `\rpcode\<font>\<char> = <int>` / `<int> = \rpcode\<font>\<char>`；
//   单位 1/1000 em（字符右/左凸出量），未设置读 0；
// - 赋值**恒全局**（组内写 200，\endgroup 后读仍 200——p2 探针
//   H2-INGROUP/H2-AFTERGROUP 双证；tex.web 字体参数表同族语义）；
// - 不触发任何排版行为本身，只记账；消费方（断行收缩/shipout 位移）
//   是后续刀口。
//
// 与 [`FontDimens`] 同构，但**不进 .fmt 快照**（microtype 每次作业都在
// preamble 重灌 mt-*.cfg 全表，快照化收益为零；省一次 FORMAT_VERSION
// bump——登记于 docs/KNOWN-SIMPLIFICATIONS.md「凸出量不持久化」）。
//
// 第十刀（M5 阶段六）：内部结构 `Rc` 化（写时复制）并迁入 [`ValueState`]
// 精确比较。此前本表**既不进 `runtime_digest` 也不进 `ValueState`**——
// 比较面盲区：只改凸出量的段会被判「状态未变」，缓存错复用属正确性洞，
// 本刀一并收口。健全性前提与快路径见 [`FontDimens`] 头注
// （「指针相同 ⇒ 内容必同」铁律）。
//
// （include! 分片：HashMap/Rc 用 expand/mod.rs 顶部的既有 import。）

/// 字符凸出量表：右（\rpcode）/左（\lpcode）两张 per-font 表。
///
/// `pub` 仅为作 [`ValueState`] 公有字段的类型（外部只做相等比较与还原，
/// 构造/写方法保持 `pub(crate)`）。
#[derive(Debug, Clone, Default)]
pub struct ProtrusionCodes {
    inner: Rc<ProtrusionInner>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ProtrusionInner {
    right: HashMap<(u32, u32), i64>,
    left: HashMap<(u32, u32), i64>,
}

/// 语义相等：`Rc::ptr_eq` 快路径 + 深比较兜底（同 [`FontDimens`]）。
impl PartialEq for ProtrusionCodes {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner) || *self.inner == *other.inner
    }
}
impl Eq for ProtrusionCodes {}

/// 凸出侧（\rpcode = 右缘凸出，\lpcode = 左缘凸出）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProtrudeSide {
    Right,
    Left,
}

impl ProtrusionCodes {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 读一项；未设置回 0（pdfTeX：字符凸出量缺省 0）。
    pub(crate) fn get(&self, side: ProtrudeSide, font: u32, ch: u32) -> i64 {
        match side {
            ProtrudeSide::Right => self.inner.right.get(&(font, ch)),
            ProtrudeSide::Left => self.inner.left.get(&(font, ch)),
        }
        .copied()
        .unwrap_or(0)
    }

    /// 写一项（恒全局语义由调用方保证：不进 save stack）。
    /// 写路径必经 `Rc::make_mut`（共享时分离，检查点视图不被污染）。
    pub(crate) fn set(&mut self, side: ProtrudeSide, font: u32, ch: u32, value: i64) {
        let inner = Rc::make_mut(&mut self.inner);
        let slot = match side {
            ProtrudeSide::Right => &mut inner.right,
            ProtrudeSide::Left => &mut inner.left,
        };
        slot.insert((font, ch), value);
    }

    /// 清空两表（fmt 加载 = 新文档起点：凸出量不随快照持久化，同实例旧值
    /// 不得泄漏过界）。与所有写路径一样经 `Rc::make_mut`。
    pub(crate) fn clear(&mut self) {
        let inner = Rc::make_mut(&mut self.inner);
        inner.right.clear();
        inner.left.clear();
    }

    /// 检查点整体替换（M5 增量）：共享目标的 `Rc`（O(1)），两侧此后的
    /// 首个写路径经 `Rc::make_mut` 自动分离——语义与逐项复制等价。
    pub(crate) fn replace_from(&mut self, other: &Self) {
        self.inner = Rc::clone(&other.inner);
    }
}

#[cfg(test)]
mod protrusions_tests {
    use super::{ProtrudeSide, ProtrusionCodes};

    #[test]
    fn get_unset_is_zero() {
        let pc = ProtrusionCodes::new();
        assert_eq!(pc.get(ProtrudeSide::Right, 1, 97), 0);
        assert_eq!(pc.get(ProtrudeSide::Left, 1, 97), 0);
    }

    #[test]
    fn sides_are_independent() {
        let mut pc = ProtrusionCodes::new();
        pc.set(ProtrudeSide::Right, 1, 97, 100);
        pc.set(ProtrudeSide::Left, 1, 97, 50);
        assert_eq!(pc.get(ProtrudeSide::Right, 1, 97), 100);
        assert_eq!(pc.get(ProtrudeSide::Left, 1, 97), 50);
        // 同侧异字体/异字符互不串
        assert_eq!(pc.get(ProtrudeSide::Right, 2, 97), 0);
        assert_eq!(pc.get(ProtrudeSide::Right, 1, 98), 0);
    }

    #[test]
    fn set_overwrites_and_replace_from_swaps_both() {
        let mut pc = ProtrusionCodes::new();
        pc.set(ProtrudeSide::Right, 1, 97, 10);
        pc.set(ProtrudeSide::Right, 1, 97, 20);
        assert_eq!(pc.get(ProtrudeSide::Right, 1, 97), 20);
        let mut snap = ProtrusionCodes::new();
        snap.set(ProtrudeSide::Left, 3, 65, 7);
        pc.replace_from(&snap);
        assert_eq!(pc.get(ProtrudeSide::Right, 1, 97), 0);
        assert_eq!(pc.get(ProtrudeSide::Left, 3, 65), 7);
    }

    /// 第十刀：`replace_from` 共享后写侧分离（CoW 隔离），且写不污染源侧。
    #[test]
    fn replace_from_shares_then_writes_diverge() {
        let mut a = ProtrusionCodes::new();
        a.set(ProtrudeSide::Right, 1, 97, 10);
        let mut b = ProtrusionCodes::new();
        b.replace_from(&a);
        b.set(ProtrudeSide::Right, 1, 97, 99);
        assert_eq!(b.get(ProtrudeSide::Right, 1, 97), 99);
        assert_eq!(a.get(ProtrudeSide::Right, 1, 97), 10, "源侧不被写污染");
    }
}
