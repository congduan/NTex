//! 增量计算基础设施（M5）。
//!
//! M5 目标（plan.md §7）：编辑一处仅重算受影响段落。本 crate 提供无依赖的
//! 三块基石，供排版层（`ntex-layout`）与未来求值图（M5 后续）复用：
//!
//! - [`Fingerprint`]/[`Fnv1a`]：128 位"内容指纹"增量哈希。指纹一致性是增量缓存
//!   正确性的前提——**同输入必同指纹**；反之不同输入发生指纹碰撞的概率 ~2^-128
//!   （计划风险橡皮标：增量 vs 全量 diff 常驻 CI 兜底）。
//! - [`ParagraphCache`]：有界容量（按插入序驱逐最旧）的段落级记忆表，命中即跳过
//!   昂贵的折行（Knuth-Plass）计算。条目可附 [`DependencySet`] 依赖快照，支持
//!   O(tags) 的失效传播（`get_incremental`/`invalidate`）。
//! - [`DependencySet`]：M5 阶段二——计算结果的**声明式依赖版本登记**（`tag → 版本`）。
//!   失效判定不必重新展开求值重算指纹，比较版本号即可（前述依赖追中用于
//!   "编辑 → 只重算失效子图"）。
//!
//! 依赖策略：本 crate 不依赖 `ntex-layout` 的 `Node` 具体类型，缓存值以泛型
//! `V: Clone` 承载；指纹喂入由调用方（排版层）按 `Node` 结构逐字段进行。

/// 默认段落缓存容量（段数）。
pub const DEFAULT_CAPACITY: usize = 4096;

/// 128 位内容指纹：两个独立种子 FNV-1a 车道合成，碰撞概率 ~2^-128。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint(u128);

impl Fingerprint {
    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }
}

/// FNV-1a 增量哈希器（双车道 → 128 位输出）。
///
/// 用法：`feed_*` 逐字段喂入，最后 [`Fnv1a::finish`] 收束。**喂入顺序即指纹语义**，
/// 调用方必须按固定次序喂入每个字段，否则同输入可能因字段次序不同得到不同指纹。
#[derive(Debug, Clone, Copy)]
pub struct Fnv1a {
    a: u64,
    b: u64,
}

const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// 第二车道的不同偏移（与主车道偏置不同，减少碰撞相关性）。
const OFFSET_B: u64 = 0x8422_2325_cbf2_9ce4;

impl Fnv1a {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一段字节。
    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.a ^= u64::from(b);
            self.a = self.a.wrapping_mul(PRIME);
            // b 车道反序喂入：末字节先到，与 a 车道方向错开
            self.b ^= u64::from(b);
            self.b = self.b.wrapping_mul(PRIME);
        }
    }

    /// 喂入一个 u64（小端字节序）。
    pub fn feed_u64(&mut self, v: u64) {
        self.feed_bytes(&v.to_le_bytes());
    }

    /// 喂入一个 i64（按位视为 u64 喂入）。
    pub fn feed_i64(&mut self, v: i64) {
        self.feed_u64(v as u64);
    }

    /// 收束为 128 位指纹。
    #[must_use]
    pub fn finish(&self) -> Fingerprint {
        Fingerprint((u128::from(self.a) << 64) | u128::from(self.b))
    }
}

impl Default for Fnv1a {
    fn default() -> Self {
        Self {
            a: OFFSET,
            b: OFFSET_B,
        }
    }
}

/// 缓存命中/未命中统计（基准与失效验证用）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// 命中次数。
    pub hits: u64,
    /// 未命中次数。
    pub misses: u64,
    /// 依赖漂移导致的失效次数（M5 阶段二：`get_incremental` 触发 + `invalidate`）。
    pub invalids: u64,
    /// 累计入库次数。
    pub puts: u64,
    /// 缓存容量（段数）。
    pub capacity: usize,
    /// 当前占用（段数）。
    pub len: usize,
}

impl CacheStats {
    /// 命中率（0.0 ~ 1.0；从未查询时返回 1.0，避免除零）。
    #[must_use]
    pub fn hit_rate(&self) -> f32 {
        let total = self.hits + self.misses;
        if total == 0 {
            1.0
        } else {
            self.hits as f32 / total as f32
        }
    }
}

/// 一组被某计算结果依赖的状态版本（M5 阶段二：依赖登记 / 失效传播）。
///
/// 语义：`tag → version` 的多元集（`u32` 标签由调用方约定——如 0=内部参数、
/// 1=宏定义、2=寄存器……）。**单调版本**：同一 tag 只保留最近一次登记的最高版本；
/// 结果缓存条目携带其产生时的依赖快照，命中校验用 O(tags) 比较版本号即可判断
/// "结果是否仍有效"，不必重新展开求值以重算内容指纹。这是求值图上"编辑 → 只
/// 重算失效子图"跳开展开的机制。
///
/// 约定（与 `version.rs` 的"改必增、增必改"一致）：依赖内容变化必须登记更高的
/// 版本，否则失效传播会漏报（缓存错用）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DependencySet {
    deps: std::collections::BTreeMap<u32, u64>,
}

impl DependencySet {
    /// 空依赖集（无任何依赖 ⇒ 结果永不因依赖失效）。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一个依赖：记录 tag 最近一次读到的版本（`BTreeMap` 覆盖旧值，
    /// 版本必须单调，否则覆盖可能"倒退"而漏报失效）。
    pub fn record(&mut self, tag: u32, version: u64) {
        self.deps.insert(tag, version);
    }

    /// 读取某 tag 的已登记版本（未登记返回 None）。
    #[must_use]
    pub fn version(&self, tag: u32) -> Option<u64> {
        self.deps.get(&tag).copied()
    }

    /// 依赖项数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.deps.len()
    }

    /// 是否无依赖。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.deps.is_empty()
    }

    /// 失效判定：当且仅当结果所依赖的每个 tag 未被 `trigger` 改到更高版本，结果仍有效
    /// （返回 `true`）。即：对快照中的每个依赖 tag，触发要么没动它、要么动的版本 ≤
    /// 快照记录——"覆盖"了本轮复查的依赖。
    ///
    /// 读作：`trigger` 是"本轮编辑可能已改变的状态"集合；只要结果实际依赖的 tags
    /// 在本轮未被改动（版本未超），则结果无需重算。**翻转处**：快照不依赖的 tag 在
    /// 触发中被改，不影响本结果（无关改动不波及其他段）。
    #[must_use]
    pub fn covers(&self, trigger: &DependencySet) -> bool {
        self.deps.iter().all(|(&tag, &mine)| {
            match trigger.deps.get(&tag) {
                Some(&trig) => trig <= mine,
                None => true, // 触发未动此 tag：不影响本结果
            }
        })
    }

    /// 失效传播：`!covers` —— 任一触发依赖版本高于本快照 → 结果必须重算。
    #[must_use]
    pub fn needs_invalidation(&self, trigger: &DependencySet) -> bool {
        !self.covers(trigger)
    }
}

/// 有界段落记忆表：`Fingerprint → (依赖快照, V)`，满时按插入序驱逐最旧条目。
///
/// 每条记录附带其产生时的依赖版本快照（见 [`DependencySet`]）。查询可附带本轮
/// 的"可能已变状态"（`trigger`）做失效传播：命中且依赖快照未被触发覆盖时才复用，
/// 否则视为失效并保守重算（防依赖漏追踪导致缓存错用——同一 key 覆盖旧条目）。
///
/// 非并发（M6 并行阶段再加锁/分片）。`insert` 对已存在的 key 覆盖值并保持其位置。
#[derive(Debug)]
pub struct ParagraphCache<V: Clone> {
    entries: Vec<Entry<V>>,
    capacity: usize,
    hits: u64,
    misses: u64,
    puts: u64,
    invalids: u64,
}

#[derive(Debug, Clone)]
struct Entry<V: Clone> {
    key: Fingerprint,
    deps: DependencySet,
    value: V,
}

impl<V: Clone> ParagraphCache<V> {
    /// 指定容量的空缓存。
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            capacity,
            hits: 0,
            misses: 0,
            puts: 0,
            invalids: 0,
        }
    }

    /// 查询：命中返回缓存值并计数 +1；未命中返回 None 并计数 +1。
    pub fn get(&mut self, key: Fingerprint) -> Option<&V> {
        self.get_incremental(key, None)
    }

    /// 写入：容量内追加；满则驱逐最旧条目并写入。不带依赖快照（空依赖）。
    pub fn put(&mut self, key: Fingerprint, value: V) {
        self.put_dep(key, DependencySet::new(), value);
    }

    /// M5 阶段二：带依赖快照的查询（失效传播入口）。
    ///
    /// `trigger` 为本轮"可能已变的状态"依赖集（编辑定位结果）。命中时若缓存条目的
    /// 依赖快照被 trigger 覆盖（未被本轮改动）→ 复用并计命中；否则视为**失效**：
    /// 计失效、移除该条目（保守重算，防依赖漏追踪导致缓存错用）并返回 None。
    pub fn get_incremental(
        &mut self,
        key: Fingerprint,
        trigger: Option<&DependencySet>,
    ) -> Option<&V> {
        let pos = self.entries.iter().position(|e| e.key == key);
        let Some(pos) = pos else {
            self.misses += 1;
            return None;
        };
        if let Some(tr) = trigger {
            if self.entries[pos].deps.needs_invalidation(tr) {
                // 依赖已变：失效并驱逐该条目（同 key 用不同依赖也不用旧值）。
                self.entries.remove(pos);
                self.invalids += 1;
                return None;
            }
        }
        self.hits += 1;
        Some(&self.entries[pos].value)
    }

    /// M5 阶段二：带依赖快照的写入。已在缓存中的同 key 条目：覆盖值并更新依赖快照。
    pub fn put_dep(&mut self, key: Fingerprint, deps: DependencySet, value: V) {
        if self.capacity == 0 {
            return;
        }
        if let Some(entry) = self.entries.iter_mut().find(|e| e.key == key) {
            entry.value = value;
            entry.deps = deps;
            return;
        }
        if self.entries.len() >= self.capacity {
            self.entries.remove(0);
        }
        self.entries.push(Entry { key, deps, value });
        self.puts += 1;
    }

    /// 驱逐指定依赖快照中任一 tag 被 trigger 覆盖（已变）的条目。
    /// 返回被驱逐条数。这是"按失效传播批量清缓存"的入口（未命中单条场景用）。
    #[must_use]
    pub fn invalidate(&mut self, trigger: &DependencySet) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| !e.deps.needs_invalidation(trigger));
        let removed = before - self.entries.len();
        self.invalids += removed as u64;
        removed
    }

    /// 当前统计快照。
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            invalids: self.invalids,
            puts: self.puts,
            capacity: self.capacity,
            len: self.entries.len(),
        }
    }

    /// 取统计快照并**仅**清零计数器（命中/未命中/失效/入库），保留已缓存内容。
    /// 适合"按编辑轮次计量命中率"：内容在轮次间复用，计数每轮清零。
    #[must_use]
    pub fn take_stats(&mut self) -> CacheStats {
        let s = self.stats();
        self.hits = 0;
        self.misses = 0;
        self.invalids = 0;
        self.puts = 0;
        s
    }

    /// 清空内容与统计（容量保留）。
    pub fn clear(&mut self) {
        self.entries.clear();
        self.hits = 0;
        self.misses = 0;
        self.invalids = 0;
        self.puts = 0;
    }
}

impl<V: Clone> Default for ParagraphCache<V> {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_deterministic_and_order_sensitive() {
        let mut h1 = Fnv1a::new();
        h1.feed_u64(1);
        h1.feed_u64(2);
        let f1 = h1.finish();

        let mut h2 = Fnv1a::new();
        h2.feed_u64(1);
        h2.feed_u64(2);
        assert_eq!(f1, h2.finish(), "同输入必同指纹");

        let mut h3 = Fnv1a::new();
        h3.feed_u64(2);
        h3.feed_u64(1);
        assert_ne!(f1, h3.finish(), "字段次序影响指纹");
    }

    #[test]
    fn cache_hit_miss_stats() {
        let mut c = ParagraphCache::new(2);
        assert!(c.get(Fingerprint(1)).is_none());
        c.put(Fingerprint(1), "a");
        assert_eq!(c.get(Fingerprint(1)), Some(&"a"));
        let s = c.stats();
        assert_eq!(s.hits, 1);
        assert_eq!(s.misses, 1);
        assert_eq!(s.puts, 1);
    }

    #[test]
    fn cache_evicts_oldest_on_full() {
        let mut c = ParagraphCache::new(2);
        c.put(Fingerprint(1), "a");
        c.put(Fingerprint(2), "b");
        c.put(Fingerprint(3), "c"); // 驱逐最旧 1
        assert!(c.get(Fingerprint(1)).is_none());
        assert_eq!(c.get(Fingerprint(2)), Some(&"b"));
        assert_eq!(c.get(Fingerprint(3)), Some(&"c"));
    }

    #[test]
    fn zero_capacity_is_noop() {
        let mut c = ParagraphCache::new(0);
        c.put(Fingerprint(1), "a");
        assert_eq!(c.stats().len, 0);
        assert!(c.get(Fingerprint(1)).is_none());
        assert_eq!(c.stats().misses, 1);
    }

    #[test]
    fn hit_rate() {
        let mut c = ParagraphCache::new(4);
        assert_eq!(c.stats().hit_rate(), 1.0, "从未查询视为 1.0");
        c.put(Fingerprint(1), "a");
        c.get(Fingerprint(1));
        c.get(Fingerprint(9));
        assert!((c.stats().hit_rate() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn clear_resets_content_and_stats() {
        let mut c = ParagraphCache::new(4);
        c.put(Fingerprint(1), "a");
        c.get(Fingerprint(1));
        c.clear();
        assert_eq!(c.stats().hits, 0);
        assert_eq!(c.stats().len, 0);
        assert!(c.get(Fingerprint(1)).is_none());
    }

    // ---------- M5 阶段二：依赖登记 / 失效传播 ----------

    fn deps(items: &[(u32, u64)]) -> DependencySet {
        let mut d = DependencySet::new();
        for &(tag, ver) in items {
            d.record(tag, ver);
        }
        d
    }

    #[test]
    fn dependency_cover_and_invalidation() {
        // 结果依赖 宏1@5、参数3@2；本轮编辑只改 参数9（无关 tag）→ 仍有效
        let snap = deps(&[(1, 5), (3, 2)]);
        let trig_unrelated = deps(&[(9, 1)]);
        assert!(snap.covers(&trig_unrelated), "无关 tag 不使结果失效");
        assert!(!snap.needs_invalidation(&trig_unrelated));

        // 编辑改了 宏1@6（版本超本快照）→ 必须重算
        let trig_para = deps(&[(1, 6)]);
        assert!(!snap.covers(&trig_para));
        assert!(snap.needs_invalidation(&trig_para));

        // 触发版本 ≤ 快照 → 仍有效（版本单调：不倒退）
        let trig_same = deps(&[(3, 2)]);
        assert!(snap.covers(&trig_same));
    }

    #[test]
    fn cache_get_incremental_invalidates_on_dep_change() {
        let mut c = ParagraphCache::new(4);
        let k = Fingerprint(42);
        // 入库时登记 宏1@5
        c.put_dep(k, deps(&[(1, 5)]), "v1");

        // 无关编辑：触发 参数9 → 仍复用
        let trig_unrelated = deps(&[(9, 1)]);
        assert_eq!(
            c.get_incremental(k, Some(&trig_unrelated)),
            Some(&"v1"),
            "无关依赖不失效"
        );

        // 相关编辑：宏1@6 → 失效并驱逐，返回 None；统计记失效
        let trig = deps(&[(1, 6)]);
        assert_eq!(c.get_incremental(k, Some(&trig)), None, "依赖已变应失效");
        let s = c.stats();
        assert_eq!(s.invalids, 1);
        // 已驱逐：无 trigger 再查也不命中（同 key 不残留旧值）
        assert!(c.get(k).is_none());
    }

    #[test]
    fn cache_invalidate_batch_removes_stale_only() {
        let mut c = ParagraphCache::new(8);
        c.put_dep(Fingerprint(1), deps(&[(1, 5)]), "a"); // 依赖宏1@5
        c.put_dep(Fingerprint(2), deps(&[(1, 5)]), "b"); // 依赖宏1@5
        c.put_dep(Fingerprint(3), deps(&[(2, 1)]), "c"); // 依赖宏2@1（无关）

        // 编辑改了 宏1@6：两条相关条目被批量逐出
        let removed = c.invalidate(&deps(&[(1, 6)]));
        assert_eq!(removed, 2);
        assert!(c.get(Fingerprint(1)).is_none());
        assert!(c.get(Fingerprint(2)).is_none());
        assert_eq!(c.get(Fingerprint(3)), Some(&"c"), "无关条目保留");
        assert_eq!(c.stats().invalids, 2);
    }
}
