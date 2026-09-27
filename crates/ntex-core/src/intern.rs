//! 控制序列/字符串驻留表（RFC-1 §4）。
//!
//! csid = u32，即 names 数组下标：`.fmt` 线性化后 mmap 直接索引，零字符串查找。
//! 表只追加、不修改（名字不可变），快照/并发由上层版本化（M6）处理。

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

/// FxHash 风格快速哈希（rustc-hash 算法的内联版，避免新增依赖）。
///
/// 驻留表查找在引擎最热路径上：每个源码控制序列 token 扫描、每次 `\csname`
/// 构造都要 `intern()` 一遍。键是引擎自己驻留的名字（非外部不可信输入），
/// std 默认 SipHash 的抗碰撞/抗注入开销在此纯属浪费。非加密场景。
#[derive(Default)]
pub struct FastHasher {
    hash: u64,
}

/// FxHash 乘法种子（rustc-hash 同常数）。
const FX_SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FastHasher {
    #[inline]
    fn add(&mut self, w: u64) {
        self.hash = (self.hash.rotate_left(5) ^ w).wrapping_mul(FX_SEED);
    }
}

impl Hasher for FastHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in chunks.by_ref() {
            self.add(u64::from_le_bytes(c.try_into().expect("8 字节块")));
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            let mut tail = [0u8; 8];
            tail[..rem.len()].copy_from_slice(rem);
            self.add(u64::from_le_bytes(tail));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
}

/// 任意字节串的单值哈希（`\csname` 单槽缓存的键；与 `FastHasher` 同族）。
pub(crate) fn fast_hash(bytes: &[u8]) -> u64 {
    let mut h = FastHasher::default();
    h.write(bytes);
    h.finish()
}

/// 名字驻留表。
#[derive(Debug, Clone, Default)]
pub struct InternTable {
    names: Vec<Arc<str>>,
    map: HashMap<Arc<str>, u32, BuildHasherDefault<FastHasher>>,
}

impl InternTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// 驻留名字；已存在则返回既有 csid。
    pub fn intern(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.map.get(name) {
            return id;
        }
        let id = self.names.len() as u32;
        let arc: Arc<str> = Arc::from(name);
        self.map.insert(arc.clone(), id);
        self.names.push(arc);
        id
    }

    /// 查询名字；不存在返回 `None`（不创建）。
    pub fn lookup(&self, name: &str) -> Option<u32> {
        self.map.get(name).copied()
    }

    /// csid → 名字（越界 panic 视为内部错误）。
    pub fn name(&self, csid: u32) -> &str {
        &self.names[csid as usize]
    }

    /// 已驻留名字数。
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// 全部名字（`.fmt` 快照：按 csid 顺序导出，加载时按序重建 csid 一致）。
    pub fn names_vec(&self) -> Vec<String> {
        self.names.iter().map(|s| s.to_string()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_deduplicates() {
        let mut t = InternTable::new();
        let a = t.intern("alpha");
        let b = t.intern("alpha");
        assert_eq!(a, b);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn intern_assigns_sequential_ids() {
        let mut t = InternTable::new();
        let a = t.intern("a");
        let b = t.intern("b");
        assert_eq!(a, 0);
        assert_eq!(b, 1);
    }

    #[test]
    fn lookup_and_name_round_trip() {
        let mut t = InternTable::new();
        let csid = t.intern("\\\\");
        assert_eq!(t.lookup("\\\\"), Some(csid));
        assert_eq!(t.lookup("missing"), None);
        assert_eq!(t.name(csid), "\\\\");
    }

    #[test]
    fn empty_table() {
        let t = InternTable::new();
        assert!(t.is_empty());
        assert_eq!(t.lookup("x"), None);
    }
}
