//! Allan deviation: how the noise of an average changes with the averaging time. Averaging
//! longer helps while the deviation falls; past its lowest point, drift gets in.

use serde::{Deserialize, Serialize};

/// Fewest differences behind one point.
const MIN_TERMS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AllanPoint {
    /// Averaging time, s.
    pub tau_s: f64,
    /// Allan deviation, in reading units.
    pub deviation: f64,
    /// Number of differences behind it.
    pub terms: usize,
}

/// Overlapping Allan deviation of evenly spaced readings `x`, one every `tau0_s` seconds, at
/// averages of 1, 2, 4, … readings.
pub fn allan_deviation(x: &[f64], tau0_s: f64) -> Vec<AllanPoint> {
    let n = x.len();
    let first = x.first().copied().unwrap_or(0.0);
    let mut cum = Vec::with_capacity(n + 1);
    cum.push(0.0);
    for v in x {
        cum.push(cum.last().copied().unwrap_or(0.0) + (v - first));
    }
    let mut out = Vec::new();
    let mut m = 1;
    while 2 * m <= n {
        let terms = n - 2 * m + 1;
        if terms < MIN_TERMS {
            break;
        }
        let mf = m as f64;
        let sum: f64 = (0..terms)
            .map(|k| {
                let a = (cum[k + m] - cum[k]) / mf;
                let b = (cum[k + 2 * m] - cum[k + m]) / mf;
                (b - a) * (b - a)
            })
            .sum();
        out.push(AllanPoint { tau_s: mf * tau0_s, deviation: (sum / (2.0 * terms as f64)).sqrt(), terms });
        m *= 2;
    }
    out
}

/// The averaging time with the lowest deviation.
pub fn best_averaging(points: &[AllanPoint]) -> Option<AllanPoint> {
    points.iter().copied().min_by(|a, b| a.deviation.total_cmp(&b.deviation))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn white_noise_falls_as_root_tau() {
        let mut rng = Rng::new(3);
        let x: Vec<f64> = (0..8192).map(|_| rng.normal()).collect();
        let a = allan_deviation(&x, 1.0);
        assert!((a[0].deviation - 1.0).abs() < 0.05, "{:?}", a[0]);
        let p64 = a.iter().find(|p| p.tau_s == 64.0).unwrap();
        assert!((p64.deviation - 0.125).abs() < 0.03, "{p64:?}");
    }

    #[test]
    fn drift_sets_the_best_averaging_time() {
        let mut rng = Rng::new(4);
        let x: Vec<f64> = (0..8192).map(|i| rng.normal() + 0.002 * i as f64).collect();
        let best = best_averaging(&allan_deviation(&x, 1.0)).unwrap();
        assert!(best.tau_s >= 8.0 && best.tau_s <= 128.0, "{best:?}");
    }
}
