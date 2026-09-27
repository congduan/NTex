// \fontdimen 覆盖表：`(font_id, 参数号) → 值（sp）`，附每字体最大参数号缓存。
//
// 第九刀（纯性能）：原实现是裸 `HashMap<(u32, u32), i64>`，而
// `fontdimen_effective_count`（tex.web `font_params[f]` 的 NTex 对应物）
// 每次越界判定都要**全表扫描**求该字体的最大参数号。expl3 intarray 的
// pdftex 回退分支把整数组模拟成 `\fontdimen` 写（codepoint 数据 34931 行
// × 多字段），表随写增长 → 单次 O(N) × N 次写 = O(N²)，expl3 载入 11 分钟。
// 这里把「每字体最大参数号」增量维护成 O(1) 查询：
// - `insert`：`max_num[f] = max(max_num[f], n)`；
// - `remove`：仅当移除的正是当前最大值时才重算（O(该字体键数)，tex.web
//   无此操作，仅 save-stack 回滚可达）；
// - `clear` / `replace_from`：整体重建。
//
// 语义与 tex.web 对齐：`font_params[f]` 一旦扩容**永久生效**（只要键还在
// 表内），与「该字体当前是否是最后装载的字体」无关。
//
// （include! 分片：HashMap 用 expand/mod.rs 顶部的既有 import。）

/// `\fontdimen` 覆盖表（含每字体最大参数号缓存）。
#[derive(Debug, Clone, Default)]
pub(crate) struct FontDimens {
    map: HashMap<(u32, u32), i64>,
    /// 每字体已写最大参数号 = `map` 中该字体键的最大 n（O(1) 读）。
    max_num: HashMap<u32, u32>,
}

impl FontDimens {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn get(&self, font: u32, num: u32) -> Option<i64> {
        self.map.get(&(font, num)).copied()
    }

    /// 写入一个覆盖项；同时 O(1) 维护该字体的最大参数号。
    pub(crate) fn insert(&mut self, font: u32, num: u32, value: i64) {
        self.map.insert((font, num), value);
        let slot = self.max_num.entry(font).or_insert(0);
        if num > *slot {
            *slot = num;
        }
    }

    /// 删除一个覆盖项（tex.web 无此操作；仅 save-stack 回滚走这里）。
    /// 若移除的正是当前最大值，才重算该字体的最大参数号。
    pub(crate) fn remove(&mut self, font: u32, num: u32) {
        if self.map.remove(&(font, num)).is_none() {
            return;
        }
        if self.max_num.get(&font).copied() == Some(num) {
            let rest = self
                .map
                .keys()
                .filter_map(|&(f, n)| (f == font).then_some(n))
                .max();
            match rest {
                Some(m) => {
                    self.max_num.insert(font, m);
                }
                None => {
                    self.max_num.remove(&font);
                }
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.max_num.clear();
    }

    /// 字体的「扩容分量」= 已写最大参数号（0 = 未扩容）。
    pub(crate) fn max_num(&self, font: u32) -> u32 {
        self.max_num.get(&font).copied().unwrap_or(0)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&(u32, u32), &i64)> {
        self.map.iter()
    }

    /// 快照导出（v22 fmt 持久化）。排序保证编码确定性。
    pub(crate) fn to_vec(&self) -> Vec<(u32, u32, i64)> {
        let mut v: Vec<(u32, u32, i64)> =
            self.map.iter().map(|(&(f, n), &v)| (f, n, v)).collect();
        v.sort_unstable();
        v
    }

    /// 检查点整体替换（M5 增量）：两份内部结构一起换，杜绝缓存失一致。
    pub(crate) fn replace_from(&mut self, other: &Self) {
        self.map.clone_from(&other.map);
        self.max_num.clone_from(&other.max_num);
    }
}

#[cfg(test)]
mod fontdimens_tests {
    use super::FontDimens;

    #[test]
    fn max_num_tracks_inserts_per_font() {
        let mut fd = FontDimens::new();
        fd.insert(1, 3, 30);
        fd.insert(1, 9, 90);
        fd.insert(2, 5, 50);
        assert_eq!(fd.max_num(1), 9);
        assert_eq!(fd.max_num(2), 5);
        assert_eq!(fd.max_num(3), 0);
        assert_eq!(fd.get(1, 9), Some(90));
    }

    #[test]
    fn remove_recomputes_only_on_max_removal() {
        let mut fd = FontDimens::new();
        fd.insert(1, 3, 30);
        fd.insert(1, 9, 90);
        // 移除非最大值：缓存不动
        fd.remove(1, 3);
        assert_eq!(fd.max_num(1), 9);
        // 移除最大值：重算到次大
        fd.remove(1, 9);
        assert_eq!(fd.max_num(1), 0);
        assert_eq!(fd.iter().count(), 0);
    }

    #[test]
    fn remove_absent_key_is_noop() {
        let mut fd = FontDimens::new();
        fd.insert(1, 3, 30);
        fd.remove(1, 4);
        fd.remove(2, 3);
        assert_eq!(fd.max_num(1), 3);
        assert_eq!(fd.get(1, 3), Some(30));
    }

    #[test]
    fn clear_and_replace_keep_cache_consistent() {
        let mut fd = FontDimens::new();
        fd.insert(1, 7, 70);
        fd.clear();
        assert_eq!(fd.max_num(1), 0);
        let mut snapshot = FontDimens::new();
        snapshot.insert(4, 2, 20);
        fd.replace_from(&snapshot);
        assert_eq!(fd.max_num(4), 2);
        assert_eq!(fd.get(4, 2), Some(20));
    }
}
