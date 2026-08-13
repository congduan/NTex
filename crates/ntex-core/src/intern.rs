//! 控制序列/字符串驻留表（RFC-1 §4）。
//!
//! csid = u32，即 names 数组下标：`.fmt` 线性化后 mmap 直接索引，零字符串查找。
//! 表只追加、不修改（名字不可变），快照/并发由上层版本化（M6）处理。

use std::collections::HashMap;
use std::sync::Arc;

/// 名字驻留表。
#[derive(Debug, Clone, Default)]
pub struct InternTable {
    names: Vec<Arc<str>>,
    map: HashMap<Arc<str>, u32>,
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
