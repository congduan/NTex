//! 增量计算基础设施（M5）。
//!
//! M5 目标（plan.md §7）：编辑一处仅重算受影响段落。本 crate 提供无依赖的
//! 两块基石，供排版层（`ntex-layout`）与未来求值图（M5 后续）复用：
//!
//! - [`Fingerprint`]/[`Fnv1a`]：128 位"内容指纹"增量哈希。指纹一致性是增量缓存
//!   正确性的前提——**同输入必同指纹**；反之不同输入发生指纹碰撞的概率 ~2^-128
//!   （计划风险橡皮标：增量 vs 全量 diff 常驻 CI 兜底）。
//! - [`ParagraphCache`]：有界容量（按插入序驱逐最旧）的段落级记忆表，命中即跳过
//!   昂贵的折行（Knuth-Plass）计算。
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

/// 有界段落记忆表：`Fingerprint → V`，满时按插入序驱逐最旧条目。
///
/// 非并发（M6 并行阶段再加锁/分片）。`insert` 对已存在的 key 覆盖值并保持其位置。
#[derive(Debug)]
pub struct ParagraphCache<V: Clone> {
    entries: Vec<Entry<V>>,
    capacity: usize,
    hits: u64,
    misses: u64,
    puts: u64,
}

#[derive(Debug, Clone)]
struct Entry<V: Clone> {
    key: Fingerprint,
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
        }
    }

    /// 查询：命中返回缓存值并计数 +1；未命中返回 None 并计数 +1。
    pub fn get(&mut self, key: Fingerprint) -> Option<&V> {
        if let Some(entry) = self.entries.iter().find(|e| e.key == key) {
            self.hits += 1;
            Some(&entry.value)
        } else {
            self.misses += 1;
            None
        }
    }

    /// 写入：容量内追加；满则驱逐最旧条目并写入。
    pub fn put(&mut self, key: Fingerprint, value: V) {
        if self.capacity == 0 {
            return;
        }
        if let Some(entry) = self.entries.iter_mut().find(|e| e.key == key) {
            entry.value = value;
            return;
        }
        if self.entries.len() >= self.capacity {
            self.entries.remove(0);
        }
        self.entries.push(Entry { key, value });
        self.puts += 1;
    }

    /// 当前统计快照。
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            puts: self.puts,
            capacity: self.capacity,
            len: self.entries.len(),
        }
    }

    /// 取统计快照并**仅**清零计数器（命中/未命中/入库），保留已缓存内容。
    /// 适合"按编辑轮次计量命中率"：内容在轮次间复用，计数每轮清零。
    #[must_use]
    pub fn take_stats(&mut self) -> CacheStats {
        let s = self.stats();
        self.hits = 0;
        self.misses = 0;
        self.puts = 0;
        s
    }

    /// 清空内容与统计（容量保留）。
    pub fn clear(&mut self) {
        self.entries.clear();
        self.hits = 0;
        self.misses = 0;
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
}
