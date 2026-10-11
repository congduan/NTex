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
// （include! 分片：HashMap 用 expand/mod.rs 顶部的既有 import。）

/// 字符凸出量表：右（\rpcode）/左（\lpcode）两张 per-font 表。
#[derive(Debug, Clone, Default)]
pub(crate) struct ProtrusionCodes {
    right: HashMap<(u32, u32), i64>,
    left: HashMap<(u32, u32), i64>,
}

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
            ProtrudeSide::Right => self.right.get(&(font, ch)),
            ProtrudeSide::Left => self.left.get(&(font, ch)),
        }
        .copied()
        .unwrap_or(0)
    }

    /// 写一项（恒全局语义由调用方保证：不进 save stack）。
    pub(crate) fn set(&mut self, side: ProtrudeSide, font: u32, ch: u32, value: i64) {
        let slot = match side {
            ProtrudeSide::Right => &mut self.right,
            ProtrudeSide::Left => &mut self.left,
        };
        slot.insert((font, ch), value);
    }

    /// 检查点整体替换（M5 增量）。
    pub(crate) fn replace_from(&mut self, other: &Self) {
        self.right.clone_from(&other.right);
        self.left.clone_from(&other.left);
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
}
