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
// 第十刀（M5 阶段六）：内部结构 `Rc` 化（写时复制，与第七刀 eqtb 槽向量、
// 第五刀盒子寄存器同形态）。动机：检查点**每执行段恒真捕获**（record_post），
// expl3 intarray 场景表可达数万条（实测 34931 条时 `FontDimens::clone`
// ~6.4µs、逐项哈希 ~80µs/段），Rc 化后捕获 = 引用计数 +1（实测 ~3ns）。
// 本类型同时迁入 [`ValueState`] 精确比较（走出 `runtime_digest` 指纹），
// 比较走 `PartialEq` 的 `Rc::ptr_eq` 快路径——段未触字体参数表时两侧共享
// 同一分配，O(1) 判等；指针不同才逐项深比较（内容等价仍判等）。
//
// **「指针相同 ⇒ 内容必同」铁律**（PartialEq 快路径的健全性前提）：一切
// 写路径（`insert`/`clear`）都先经 `Rc::make_mut`——共享期间（strong_count
// > 1）`make_mut` 克隆出新分配、原缓冲区自此只读；独占时就地写，但此刻
// 无其他快照持有同一指针。因此两个**存活**的 `FontDimens` 若 `Rc::ptr_eq`
// 相等，其间必无写路径触达该缓冲区。`ValueState`/`ValueExtras` 持有强
// 引用，比较期间两份必存活 → 无「地址复用」假等（这是不走「指针身份进
// digest」路线的原因：u64 指纹不持引用，地址复用会制造假相等）。
// `cow_tests::checkpoint_capture_shares_fontdimens` 锁死该前提。
//
// （include! 分片：HashMap/Rc 用 expand/mod.rs 顶部的既有 import。）

/// `\fontdimen` 覆盖表（含每字体最大参数号缓存；内部 `Rc` 写时复制）。
///
/// `pub` 仅为作 [`ValueState`] 公有字段的类型（外部只做相等比较与还原，
/// 构造/写方法保持 `pub(crate)`）。
#[derive(Debug, Clone, Default)]
pub struct FontDimens {
    inner: Rc<FontDimensInner>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct FontDimensInner {
    map: HashMap<(u32, u32), i64>,
    /// 每字体已写最大参数号 = `map` 中该字体键的最大 n（O(1) 读）。
    max_num: HashMap<u32, u32>,
}

/// 语义相等：优先 `Rc::ptr_eq`（共享同一分配 = 未被任何写路径触达），
/// 否则逐项深比较。见类型头注的「指针相同 ⇒ 内容必同」铁律。
impl PartialEq for FontDimens {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner) || *self.inner == *other.inner
    }
}
impl Eq for FontDimens {}

impl FontDimens {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn get(&self, font: u32, num: u32) -> Option<i64> {
        self.inner.map.get(&(font, num)).copied()
    }

    /// 写入一个覆盖项；同时 O(1) 维护该字体的最大参数号。
    ///
    /// 写路径必经 `Rc::make_mut`：共享时分离（检查点视图不被污染），
    /// 独占时就地写。
    pub(crate) fn insert(&mut self, font: u32, num: u32, value: i64) {
        let inner = Rc::make_mut(&mut self.inner);
        inner.map.insert((font, num), value);
        let slot = inner.max_num.entry(font).or_insert(0);
        if num > *slot {
            *slot = num;
        }
    }

    pub(crate) fn clear(&mut self) {
        let inner = Rc::make_mut(&mut self.inner);
        inner.map.clear();
        inner.max_num.clear();
    }

    /// 字体的「扩容分量」= 已写最大参数号（0 = 未扩容）。
    pub(crate) fn max_num(&self, font: u32) -> u32 {
        self.inner.max_num.get(&font).copied().unwrap_or(0)
    }

    /// 快照导出（v22 fmt 持久化）。排序保证编码确定性。
    pub(crate) fn to_vec(&self) -> Vec<(u32, u32, i64)> {
        let mut v: Vec<(u32, u32, i64)> = self
            .inner
            .map
            .iter()
            .map(|(&(f, n), &v)| (f, n, v))
            .collect();
        v.sort_unstable();
        v
    }

    /// 检查点整体替换（M5 增量）：共享目标的 `Rc`（O(1)），两侧此后的
    /// 首个写路径经 `Rc::make_mut` 自动分离——语义与逐项复制等价，
    /// 杜绝 `map`/`max_num` 缓存失一致。
    pub(crate) fn replace_from(&mut self, other: &Self) {
        self.inner = Rc::clone(&other.inner);
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

    /// 第十刀：`replace_from` 共享后写侧分离——两侧互不影响（CoW 隔离）。
    #[test]
    fn replace_from_shares_then_writes_diverge() {
        let mut a = FontDimens::new();
        a.insert(1, 1, 10);
        let mut b = FontDimens::new();
        b.replace_from(&a);
        assert_eq!(a, b, "共享即相等");
        b.insert(1, 1, 99);
        assert_ne!(a.get(1, 1), b.get(1, 1), "写侧应分离（make_mut）");
        assert_eq!(a.get(1, 1), Some(10), "源侧不被写污染");
        // 深比较路径：内容等价的不同分配仍判等。
        let mut c = FontDimens::new();
        c.insert(1, 1, 10);
        assert_eq!(a, c, "内容等价 ⇒ 语义相等（PartialEq 深比较兜底）");
    }
}
