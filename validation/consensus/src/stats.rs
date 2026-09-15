//! Order statistics over a category's absolute deltas (nearest-rank percentiles).

#[derive(Debug, Clone, serde::Serialize)]
pub struct Summary {
    pub n: usize,
    pub mean_signed: f64,
    pub mean_abs: f64,
    pub rms: f64,
    pub p50: f64,
    pub p99: f64,
    pub p99_9: f64,
    pub max: f64,
    /// Index (into the caller's record list) of the maximum |delta|.
    pub argmax: usize,
}

pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// `values` are signed deltas paired with the record index they came from.
pub fn summarize(values: &[(usize, f64)]) -> Summary {
    let n = values.len();
    if n == 0 {
        return Summary {
            n: 0,
            mean_signed: f64::NAN,
            mean_abs: f64::NAN,
            rms: f64::NAN,
            p50: f64::NAN,
            p99: f64::NAN,
            p99_9: f64::NAN,
            max: f64::NAN,
            argmax: usize::MAX,
        };
    }
    let mut abs: Vec<f64> = values.iter().map(|(_, v)| v.abs()).collect();
    let (argmax, max) = values.iter().map(|(i, v)| (*i, v.abs())).fold(
        (usize::MAX, f64::NEG_INFINITY),
        |acc, x| {
            if x.1 > acc.1 { x } else { acc }
        },
    );
    abs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mean_signed = values.iter().map(|(_, v)| v).sum::<f64>() / n as f64;
    let mean_abs = abs.iter().sum::<f64>() / n as f64;
    let rms = (values.iter().map(|(_, v)| v * v).sum::<f64>() / n as f64).sqrt();
    Summary {
        n,
        mean_signed,
        mean_abs,
        rms,
        p50: percentile(&abs, 0.50),
        p99: percentile(&abs, 0.99),
        p99_9: percentile(&abs, 0.999),
        max,
        argmax,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles() {
        let v: Vec<f64> = (1..=1000).map(|i| i as f64).collect();
        assert_eq!(percentile(&v, 0.5), 500.0);
        assert_eq!(percentile(&v, 0.99), 990.0);
        assert_eq!(percentile(&v, 0.999), 999.0);
        assert_eq!(percentile(&v, 1.0), 1000.0);
        assert_eq!(percentile(&[7.0], 0.999), 7.0);
        assert!(percentile(&[], 0.5).is_nan());
    }

    #[test]
    fn summary_uses_abs_for_order_stats_and_signed_for_bias() {
        let s = summarize(&[(0, -3.0), (1, 1.0), (2, 2.0), (3, -1.0)]);
        assert_eq!(s.n, 4);
        assert!((s.mean_signed - (-0.25)).abs() < 1e-12);
        assert!((s.mean_abs - 1.75).abs() < 1e-12);
        assert_eq!(s.max, 3.0);
        assert_eq!(s.argmax, 0);
        assert_eq!(s.p50, 1.0);
        assert!((s.rms - (15.0f64 / 4.0).sqrt()).abs() < 1e-12);
    }
}
