//! Exponential settle fit: `x(t) = settled + amplitude · e^(−(t − t0)/τ)`.
//!
//! A set-up that warms up (camera, laser, stand) drifts fast at first and then slows down. The fit
//! tries τ on a log grid; for each τ the other two numbers come from linear least squares, so there
//! is no start value to get wrong. The τ range is every τ that fits nearly as well as the best
//! (residual sum of squares within 4 noise variances, the noise variance raised for correlated
//! residuals). If the best τ is at the top of the grid, the data still look like a straight line:
//! there is no upper bound and no honest estimate of when it settles.

use serde::{Deserialize, Serialize};

use crate::stats;

const GRID: usize = 80;
/// Residual sum of squares above the best, in noise variances, that still counts as a fit.
const RANGE_DELTA: f64 = 4.0;
/// Largest τ tried, as a multiple of the time span of the data.
const TAU_MAX_SPANS: f64 = 50.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub tau_s: f64,
    pub settled: f64,
    pub amplitude: f64,
    pub rss: f64,
}

impl Candidate {
    fn drift_over(&self, t0: f64, t: f64, run_s: f64) -> f64 {
        self.amplitude.abs() * (-(t - t0) / self.tau_s).exp() * (1.0 - (-run_s / self.tau_s).exp())
    }

    /// Earliest time at which the drift over a run of `run_s` is at most `tolerance`.
    fn ready_at(&self, t0: f64, run_s: f64, tolerance: f64) -> f64 {
        let first = self.drift_over(t0, t0, run_s);
        if first <= tolerance { t0 } else { t0 + self.tau_s * (first / tolerance).ln() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettleFit {
    /// Start of the data the fit uses, s.
    pub t0_s: f64,
    /// Reading the set-up settles to.
    pub settled: f64,
    /// Distance still to go at `t0_s`.
    pub amplitude: f64,
    pub tau_s: f64,
    pub tau_low_s: f64,
    /// `None`: no upper bound; the data still look like a straight line.
    pub tau_high_s: Option<f64>,
    /// Residual root mean square.
    pub rms: f64,
    pub points: usize,
    #[serde(skip)]
    accepted: Vec<Candidate>,
}

/// When the drift over the planned run is small enough, s on the same clock as the readings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadyAt {
    pub best_s: f64,
    pub earliest_s: f64,
    /// `None` when τ has no upper bound.
    pub latest_s: Option<f64>,
}

impl SettleFit {
    fn best(&self) -> Candidate {
        Candidate { tau_s: self.tau_s, settled: self.settled, amplitude: self.amplitude, rss: 0.0 }
    }

    pub fn value_at(&self, t_s: f64) -> f64 {
        self.settled + self.amplitude * (-(t_s - self.t0_s) / self.tau_s).exp()
    }

    /// Drift rate of the fitted curve, per s.
    pub fn rate_at(&self, t_s: f64) -> f64 {
        -self.amplitude / self.tau_s * (-(t_s - self.t0_s) / self.tau_s).exp()
    }

    /// Drift the fit predicts over a run of `run_s` that starts at `t_s`.
    pub fn drift_over(&self, t_s: f64, run_s: f64) -> f64 {
        self.best().drift_over(self.t0_s, t_s, run_s)
    }

    pub fn ready_at(&self, run_s: f64, tolerance: f64) -> ReadyAt {
        let best_s = self.best().ready_at(self.t0_s, run_s, tolerance);
        let times = self.accepted.iter().map(|c| c.ready_at(self.t0_s, run_s, tolerance));
        let earliest_s = times.clone().fold(best_s, f64::min);
        let latest_s = self.tau_high_s.map(|_| times.fold(best_s, f64::max));
        ReadyAt { best_s, earliest_s, latest_s }
    }
}

/// Fit readings `x` at times `t` (sorted, s) from `t0_s` on. `None` with fewer than 6 points or no
/// time span.
pub fn fit(t: &[f64], x: &[f64], t0_s: f64) -> Option<SettleFit> {
    let n = t.len().min(x.len());
    if n < 6 {
        return None;
    }
    let (t, x) = (&t[..n], &x[..n]);
    let span = t[n - 1] - t0_s;
    if span <= 0.0 {
        return None;
    }
    let mean = x.iter().sum::<f64>() / n as f64;
    let xc: Vec<f64> = x.iter().map(|v| v - mean).collect();
    let tau_min = (2.0 * span / (n - 1) as f64).max(1e-3);
    let tau_max = TAU_MAX_SPANS * span;
    let grid: Vec<f64> = (0..GRID).map(|k| tau_min * (tau_max / tau_min).powf(k as f64 / (GRID - 1) as f64)).collect();
    let cands: Vec<Option<Candidate>> = grid.iter().map(|&tau| solve(t, &xc, t0_s, tau)).collect();
    let (ibest, mut best) =
        cands.iter().enumerate().filter_map(|(i, c)| c.map(|c| (i, c))).min_by(|a, b| a.1.rss.total_cmp(&b.1.rss))?;

    // Golden-section search on ln τ between the neighbours of the best grid point.
    let (mut lo, mut hi) = (grid[ibest.saturating_sub(1)].ln(), grid[(ibest + 1).min(GRID - 1)].ln());
    let g = 0.618_033_988_749_895;
    for _ in 0..40 {
        let a = hi - g * (hi - lo);
        let b = lo + g * (hi - lo);
        let fa = solve(t, &xc, t0_s, a.exp()).map_or(f64::INFINITY, |c| c.rss);
        let fb = solve(t, &xc, t0_s, b.exp()).map_or(f64::INFINITY, |c| c.rss);
        if fa < fb { hi = b } else { lo = a }
    }
    if let Some(c) = solve(t, &xc, t0_s, (0.5 * (lo + hi)).exp())
        && c.rss < best.rss
    {
        best = c;
    }

    let resid: Vec<f64> = t
        .iter()
        .zip(&xc)
        .map(|(ti, xi)| xi - best.settled - best.amplitude * (-(ti - t0_s) / best.tau_s).exp())
        .collect();
    let noise_var = best.rss / (n as f64 - 3.0).max(1.0) * stats::correlation_inflation(&resid);
    let limit = best.rss + RANGE_DELTA * noise_var;
    let rss_at = |ln_tau: f64| solve(t, &xc, t0_s, ln_tau.exp()).map_or(f64::INFINITY, |c| c.rss);
    // Walk out along the grid from the best τ, then bisect for where the fit gets too poor.
    let edge = |dir: isize| -> Option<f64> {
        let mut inside = best.tau_s.ln();
        let mut k = ibest;
        loop {
            k = match k.checked_add_signed(dir) {
                Some(k) if k < GRID => k,
                _ => return None,
            };
            let g = grid[k].ln();
            if (dir < 0 && g >= inside) || (dir > 0 && g <= inside) {
                continue;
            }
            if rss_at(g) > limit {
                let mut outside = g;
                for _ in 0..40 {
                    let mid = 0.5 * (inside + outside);
                    if rss_at(mid) > limit { outside = mid } else { inside = mid }
                }
                return Some(inside.exp());
            }
            inside = g;
        }
    };
    let tau_low_s = edge(-1).unwrap_or(grid[0]);
    let tau_high_s = edge(1);
    let top = tau_high_s.unwrap_or(grid[GRID - 1]);
    let mut accepted: Vec<Candidate> =
        cands.iter().flatten().filter(|c| c.tau_s >= tau_low_s && c.tau_s <= top).copied().collect();
    accepted.extend([tau_low_s, top].iter().filter_map(|&tau| solve(t, &xc, t0_s, tau)));
    let shift = |c: Candidate| Candidate { settled: c.settled + mean, ..c };

    Some(SettleFit {
        t0_s,
        settled: best.settled + mean,
        amplitude: best.amplitude,
        tau_s: best.tau_s,
        tau_low_s,
        tau_high_s,
        rms: (best.rss / n as f64).sqrt(),
        points: n,
        accepted: accepted.into_iter().map(shift).collect(),
    })
}

/// Best `settled` and `amplitude` for one τ, by linear least squares.
fn solve(t: &[f64], x: &[f64], t0: f64, tau: f64) -> Option<Candidate> {
    let n = t.len() as f64;
    let (mut su, mut suu, mut sx, mut sux) = (0.0, 0.0, 0.0, 0.0);
    for (ti, xi) in t.iter().zip(x) {
        let u = (-(ti - t0) / tau).exp();
        su += u;
        suu += u * u;
        sx += xi;
        sux += u * xi;
    }
    let det = n * suu - su * su;
    if det <= 1e-12 * n * suu.max(1e-300) {
        return None;
    }
    let amplitude = (n * sux - su * sx) / det;
    let settled = (sx - amplitude * su) / n;
    let rss = t.iter().zip(x).map(|(ti, xi)| (xi - settled - amplitude * (-(ti - t0) / tau).exp()).powi(2)).sum();
    Some(Candidate { tau_s: tau, settled, amplitude, rss })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;
    use crate::synth::WarmUp;

    #[test]
    fn finds_tau_and_ready_time() {
        let w = WarmUp { start: 20.0, settled: 0.0, tau_s: 300.0, steps: vec![], jitter: 0.05 };
        let mut rng = Rng::new(9);
        let t: Vec<f64> = (0..900).map(f64::from).collect();
        let x: Vec<f64> = t.iter().map(|&ti| w.sample(ti, &mut rng)).collect();
        let f = fit(&t, &x, 0.0).unwrap();
        assert!((f.tau_s - 300.0).abs() < 15.0, "{f:?}");
        assert!(f.tau_low_s <= 300.0 && f.tau_high_s.unwrap() >= 300.0, "{f:?}");
        let truth = w.true_ready_s(600.0, 0.5);
        let r = f.ready_at(600.0, 0.5);
        assert!(r.earliest_s <= truth + 20.0 && r.latest_s.unwrap() >= truth - 20.0, "{r:?} vs {truth}");
    }

    #[test]
    fn a_straight_line_has_no_upper_bound() {
        let t: Vec<f64> = (0..120).map(f64::from).collect();
        let mut rng = Rng::new(2);
        let x: Vec<f64> = t.iter().map(|&ti| 0.01 * ti + 0.05 * rng.normal()).collect();
        let f = fit(&t, &x, 0.0).unwrap();
        assert_eq!(f.tau_high_s, None, "{f:?}");
        assert_eq!(f.ready_at(600.0, 0.5).latest_s, None);
    }
}
