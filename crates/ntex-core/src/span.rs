//! 源码位置与区间。
//!
//! RFC-1 Q1 决策：token 不携带位置信息（保持 8B 紧凑）；需要错误定位 / IDE 跳转时，
//! 由独立的 side-table 建立"token 序号 → [`Span`]"的映射。本模块只定义位置类型本身。

use crate::error::{Error, Result};

/// 输入源标识：每个被打开的文件（或内存缓冲）一个 id。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(pub u32);

/// 字节偏移，相对源起点（0 基）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BytePos(pub u64);

impl BytePos {
    /// 下一字节位置。
    pub fn advance(self, n: u64) -> Self {
        Self(self.0 + n)
    }
}

/// 1 基的行列位置（供终端展示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

/// 源码区间 `[start, end)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub source: SourceId,
    pub start: BytePos,
    pub end: BytePos,
}

impl Span {
    /// 构造区间；要求 `start <= end`。
    pub fn new(source: SourceId, start: BytePos, end: BytePos) -> Result<Self> {
        if start > end {
            return Err(Error::internal("Span: start 大于 end"));
        }
        Ok(Self { source, start, end })
    }

    /// 区间字节长度。
    pub fn len(&self) -> u64 {
        self.end.0 - self.start.0
    }

    /// 区间是否为空（长度为 0）。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `pos` 是否落在区间内（含端点）。
    pub fn contains(&self, pos: BytePos) -> bool {
        self.start <= pos && pos <= self.end
    }

    /// 合并两个区间（必须同源，且要求 `self.start <= other.start`）。
    pub fn merge(&self, other: &Span) -> Result<Self> {
        if self.source != other.source {
            return Err(Error::internal("Span::merge: 源不同"));
        }
        Self::new(
            self.source,
            self.start.min(other.start),
            self.end.max(other.end),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> SourceId {
        SourceId(0)
    }

    #[test]
    fn span_construction_and_len() {
        let span = Span::new(s(), BytePos(2), BytePos(5)).unwrap();
        assert_eq!(span.len(), 3);
        assert!(!span.is_empty());
        assert!(span.contains(BytePos(2)));
        assert!(span.contains(BytePos(5)));
        assert!(!span.contains(BytePos(6)));
    }

    #[test]
    fn span_rejects_reversed_bounds() {
        assert!(Span::new(s(), BytePos(5), BytePos(2)).is_err());
    }

    #[test]
    fn span_merge_requires_same_source() {
        let a = Span::new(SourceId(0), BytePos(0), BytePos(3)).unwrap();
        let b = Span::new(SourceId(1), BytePos(0), BytePos(3)).unwrap();
        assert!(a.merge(&b).is_err());

        let c = Span::new(SourceId(0), BytePos(1), BytePos(4)).unwrap();
        let merged = a.merge(&c).unwrap();
        assert_eq!(merged.start, BytePos(0));
        assert_eq!(merged.end, BytePos(4));
    }

    #[test]
    fn bytepos_advance() {
        assert_eq!(BytePos(1).advance(9), BytePos(10));
    }
}
