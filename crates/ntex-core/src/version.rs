//! 不可变状态的版本号。
//!
//! RFC-1 §5 用 eqtb 槽版本化实现宏定义的 CoW；M5 增量计算的依赖追踪需要
//! "结果依赖了哪些槽的哪个版本"。本模块提供两套机制：
//!
//! - [`Version`]/[`Versioned`]：附着在单个状态槽上的局部单调版本；
//! - [`VersionCounter`]：跨快照分配全局唯一版本的原子计数源（M6 并行展开时可并发分配）。
//!
//! 约定：**改必增、增必改**——内容变化必须递增版本，版本递增必须对应内容变化，
//! 否则增量缓存会失效错误。此约定由使用者保证，本类型不施加运行时校验（仅测试覆盖）。

use std::sync::atomic::{AtomicU64, Ordering};

/// 单调递增版本号。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Version(u64);

impl Version {
    /// 初始版本（0 保留给"从未修改"）。
    pub const INITIAL: Self = Self(0);

    /// 下一个版本（单调递增，永不为 0）。
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// 原始值。
    pub const fn get(self) -> u64 {
        self.0
    }

    /// 从原始值构造（`.fmt` 快照反序列化）。
    pub const fn from_raw(v: u64) -> Self {
        Self(v)
    }
}

/// 版本化值：`(value, version)`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Versioned<T> {
    pub value: T,
    pub version: Version,
}

impl<T> Versioned<T> {
    /// 以初始版本包装一个值。
    pub fn new(value: T) -> Self {
        Self {
            value,
            version: Version::INITIAL,
        }
    }

    /// 用新值替换并递增版本。
    pub fn bump(&mut self, value: T) {
        self.value = value;
        self.version = self.version.next();
    }
}

/// 全局原子版本源：每次调用返回一个新的、全局唯一的版本号。
#[derive(Debug, Default)]
pub struct VersionCounter(AtomicU64);

impl VersionCounter {
    /// 创建计数源（从 1 开始分配）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 分配下一个全局唯一版本号。
    pub fn next(&self) -> Version {
        // Relaxed 即可：只要求"不重复"，不要求与其他内存操作排序。
        Version(self.0.fetch_add(1, Ordering::Relaxed) + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_monotonic() {
        let mut v = Version::INITIAL;
        assert_eq!(v.get(), 0);
        v = v.next();
        assert_eq!(v.get(), 1);
        v = v.next();
        assert_eq!(v.get(), 2);
    }

    #[test]
    fn versioned_bump_increments() {
        let mut x = Versioned::new(1u32);
        assert_eq!(x.version, Version::INITIAL);
        x.bump(2);
        assert_eq!(x.value, 2);
        assert_eq!(x.version, Version(1));
    }

    #[test]
    fn version_counter_is_unique_and_never_zero() {
        let counter = VersionCounter::new();
        let a = counter.next();
        let b = counter.next();
        assert_ne!(a, b);
        assert!(a.get() >= 1);
        assert!(b.get() >= 1);
    }
}
