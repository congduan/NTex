//! 轻量计时与统计。
//!
//! M0 阶段的基准框架刻意不引入 criterion：基准对象是"一次完整编译"（秒级），
//! 只需预热 + N 次取样 + 最小/中位/均值统计即可给出稳定数字。

use anyhow::Result;
use std::time::Instant;

/// 一组计时样本（单位：纳秒）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stats {
    pub samples_ns: Vec<u64>,
}

impl Stats {
    pub fn min_ns(&self) -> u64 {
        self.samples_ns.iter().copied().min().unwrap_or(0)
    }

    pub fn max_ns(&self) -> u64 {
        self.samples_ns.iter().copied().max().unwrap_or(0)
    }

    /// 中位数（排序后取中间；偶数个取较小中位数，避免浮点）。
    pub fn median_ns(&self) -> u64 {
        let mut sorted = self.samples_ns.clone();
        sorted.sort_unstable();
        sorted[sorted.len() / 2]
    }

    pub fn mean_ns(&self) -> u64 {
        let sum: u64 = self.samples_ns.iter().sum();
        sum / self.samples_ns.len().max(1) as u64
    }
}

/// 执行 `warmup` 次预热 + `iterations` 次计时取样。
///
/// 取样中任何一次 `f` 失败都会终止并返回错误（不允许"部分成功"污染统计）。
pub fn measure<F>(warmup: usize, iterations: usize, mut f: F) -> Result<Stats>
where
    F: FnMut() -> Result<()>,
{
    for _ in 0..warmup {
        f()?;
    }
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let t0 = Instant::now();
        f()?;
        samples.push(t0.elapsed().as_nanos() as u64);
    }
    Ok(Stats {
        samples_ns: samples,
    })
}

/// 把纳秒格式化为人类可读时长。
pub fn format_duration(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.3}s", ns as f64 / 1e9)
    } else if ns >= 1_000_000 {
        format!("{:.1}ms", ns as f64 / 1e6)
    } else if ns >= 1_000 {
        format!("{:.1}us", ns as f64 / 1e3)
    } else {
        format!("{ns}ns")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn stats_aggregation() {
        let s = Stats {
            samples_ns: vec![100, 50, 30, 20, 10],
        };
        assert_eq!(s.min_ns(), 10);
        assert_eq!(s.max_ns(), 100);
        assert_eq!(s.median_ns(), 30);
        assert_eq!(s.mean_ns(), 42);
    }

    #[test]
    fn measure_collects_samples() {
        let stats = measure(1, 3, || {
            std::thread::sleep(Duration::from_micros(1));
            Ok(())
        })
        .unwrap();
        assert_eq!(stats.samples_ns.len(), 3);
        assert!(stats.min_ns() > 0);
    }

    #[test]
    fn measure_propagates_error() {
        let mut calls = 0;
        let err = measure(1, 2, || {
            calls += 1;
            if calls > 1 {
                anyhow::bail!("boom");
            }
            Ok(())
        })
        .unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration(500), "500ns");
        assert_eq!(format_duration(1500), "1.5us");
        assert_eq!(format_duration(1_500_000), "1.5ms");
        assert_eq!(format_duration(2_500_000_000), "2.500s");
    }
}
