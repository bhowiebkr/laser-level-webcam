//! Readings over time: heat drift, steps, and when the set-up is ready to measure.
//!
//! Every reading goes in with its time. The monitor averages them into short bins and works on
//! those:
//!
//! - **Drift rate** over the last `rate_window_s`, robust (Theil–Sen), per minute.
//! - **Steps**: a bump or a re-level shows as a jump between two windows that a common slope
//!   does not explain. A step restarts the estimate from the step on.
//! - **Settle fit**: an exponential approach since the start or the last step ([`settle`]).
//! - **Allan deviation**: how long to average a reading.
//! - **Readiness** against the planned run: ready when the drift over the run is below the
//!   tolerance and the beam is calm. The fit gives the time to wait with its range; the fit is
//!   used only while it agrees with the measured drift rate. With no fit to use, the rate comes
//!   from up to a run's worth of readings, and it must be two standard errors inside the
//!   allowed rate for Ready, or outside it for Drifting. Between the two, more readings decide.

use serde::{Deserialize, Serialize};

use crate::allan::{self, AllanPoint};
use crate::settle::{self, SettleFit};
use crate::stats;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MonitorConfig {
    /// Readings are averaged into bins of this length, s.
    pub bin_s: f64,
    /// Window for the drift rate, s.
    pub rate_window_s: f64,
    /// Length of each side of the step test, s.
    pub step_window_s: f64,
    /// A step must be this many standard errors.
    pub step_sigmas: f64,
    /// Time after the start or a step before any estimate, s.
    pub min_fit_s: f64,
    /// Most bins kept; older ones are dropped.
    pub max_bins: usize,
    /// Window for the Allan deviation, s.
    pub allan_window_s: f64,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            bin_s: 1.0,
            rate_window_s: 120.0,
            step_window_s: 15.0,
            step_sigmas: 8.0,
            min_fit_s: 60.0,
            max_bins: 4 * 3600,
            allan_window_s: 600.0,
        }
    }
}

/// What the user is about to measure.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunPlan {
    /// Planned run length: stations × time per station, s.
    pub run_s: f64,
    /// Drift allowed over the run, in reading units.
    pub tolerance: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bin {
    /// Mean time of the readings in the bin, s.
    pub t_s: f64,
    pub mean: f64,
    pub count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// About when it happened, s (within a bin or two).
    pub t_s: f64,
    pub size: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Readiness {
    /// Not enough data since the start or the last step, or not yet enough to tell the drift
    /// from the noise. `need_s` is then an estimate.
    Collecting { have_s: f64, need_s: f64 },
    /// The reading moves too much from bin to bin to tell a drift inside the tolerance.
    Noisy { noise: f64 },
    /// Still settling. Ready in about `in_s`, between `earliest_in_s` and `latest_in_s`.
    Settling { in_s: f64, earliest_in_s: f64, latest_in_s: Option<f64> },
    /// Drifting too fast, with no sign of slowing down that the fit can use.
    Drifting { rate_per_min: f64 },
    /// The drift over the planned run is below the tolerance and the beam is calm.
    Ready,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Status {
    /// Time of the last reading, s.
    pub t_s: f64,
    /// Start of the current estimate: the first reading, the last step or a restart, s.
    pub since_s: f64,
    pub latest: Option<f64>,
    pub rate_per_min: Option<f64>,
    pub rate_sigma_per_min: Option<f64>,
    /// Standard deviation of one bin.
    pub noise: Option<f64>,
    pub allan_best: Option<AllanPoint>,
    pub settle: Option<SettleFit>,
    pub steps: Vec<Step>,
    pub readiness: Readiness,
}

#[derive(Clone, Copy, Debug)]
struct OpenBin {
    index: i64,
    sum_t: f64,
    sum_x: f64,
    n: u32,
}

impl OpenBin {
    fn add(&mut self, t_s: f64, x: f64) {
        self.sum_t += t_s;
        self.sum_x += x;
        self.n += 1;
    }
}

#[derive(Clone, Debug)]
pub struct Monitor {
    cfg: MonitorConfig,
    origin_s: Option<f64>,
    epoch_s: f64,
    last_s: f64,
    bins: Vec<Bin>,
    open: Option<OpenBin>,
    steps: Vec<Step>,
}

impl Monitor {
    pub fn new(cfg: MonitorConfig) -> Self {
        Self { cfg, origin_s: None, epoch_s: 0.0, last_s: 0.0, bins: Vec::new(), open: None, steps: Vec::new() }
    }

    pub fn config(&self) -> &MonitorConfig {
        &self.cfg
    }

    /// Bins since the start of the current estimate, for the drift graph.
    pub fn bins(&self) -> &[Bin] {
        &self.bins
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Add one reading at time `t_s`. Returns a step when this reading completes one.
    pub fn push(&mut self, t_s: f64, x: f64) -> Option<Step> {
        if !t_s.is_finite() || !x.is_finite() {
            return None;
        }
        let origin = match self.origin_s {
            Some(o) => o,
            None => {
                self.origin_s = Some(t_s);
                self.epoch_s = t_s;
                t_s
            }
        };
        self.last_s = self.last_s.max(t_s);
        let index = ((t_s - origin) / self.cfg.bin_s).floor() as i64;
        match &mut self.open {
            Some(b) if index <= b.index => {
                b.add(t_s, x);
                None
            }
            _ => {
                let closed = self.open.replace(OpenBin { index, sum_t: t_s, sum_x: x, n: 1 });
                closed.and_then(|b| self.close(b))
            }
        }
    }

    /// Start the estimate again from `t_s`, after the user re-levels or moves the set-up.
    pub fn restart(&mut self, t_s: f64) {
        self.epoch_s = t_s;
        self.bins.clear();
        self.open = None;
    }

    fn close(&mut self, b: OpenBin) -> Option<Step> {
        let n = f64::from(b.n);
        let bin = Bin { t_s: b.sum_t / n, mean: b.sum_x / n, count: b.n };
        if bin.t_s < self.epoch_s {
            return None;
        }
        self.bins.push(bin);
        if self.bins.len() > self.cfg.max_bins {
            let extra = self.bins.len() - self.cfg.max_bins;
            self.bins.drain(..extra);
        }
        let step = self.detect_step()?;
        self.steps.push(step);
        self.epoch_s = step.t_s;
        self.bins.retain(|b| b.t_s > step.t_s);
        Some(step)
    }

    /// Standard deviation of one bin, from the differences of the latest bins.
    fn bin_noise(&self) -> Option<f64> {
        let tail = &self.bins[self.bins.len().saturating_sub(600)..];
        let means: Vec<f64> = tail.iter().map(|b| b.mean).collect();
        stats::diff_noise(&means)
    }

    /// A step: the medians of the last two windows differ by more than `step_sigmas` standard
    /// errors once the drift is taken out. The step is then placed at the split that best separates
    /// the two sides, and its size is measured across that split.
    fn detect_step(&self) -> Option<Step> {
        let w = (self.cfg.step_window_s / self.cfg.bin_s).round().max(3.0) as usize;
        let n = self.bins.len();
        if n < 2 * w {
            return None;
        }
        let win = &self.bins[n - 2 * w..];
        let first = jump(win, w)?;
        let se = step_se(self.bin_noise()?, w);
        if first.size.abs() <= self.cfg.step_sigmas * se.max(1e-12) {
            return None;
        }
        let d = &first.detrended;
        let total: f64 = d.iter().sum();
        let mut left = 0.0;
        let mut split = w;
        let mut best = 0.0;
        for j in 1..d.len() {
            left += d[j - 1];
            let (nl, nr) = (j as f64, (d.len() - j) as f64);
            let score = ((total - left) / nr - left / nl).abs() * (nl * nr).sqrt();
            if score > best {
                (split, best) = (j, score);
            }
        }
        let size = if split >= 3 && win.len() - split >= 3 { jump(win, split)?.size } else { first.size };
        Some(Step { t_s: 0.5 * (win[split - 1].t_s + win[split].t_s), size })
    }

    pub fn status(&self, plan: &RunPlan) -> Status {
        let t_now = self.last_s;
        let since = if self.origin_s.is_some() { t_now - self.epoch_s } else { 0.0 };
        let noise = self.bin_noise();

        let rate_bins: Vec<&Bin> = self.bins.iter().filter(|b| b.t_s >= t_now - self.cfg.rate_window_s).collect();
        let rate = (rate_bins.len() >= 6).then(|| robust_rate(&rate_bins)).flatten();
        let rate_mid = rate_bins.first().map_or(t_now, |b| 0.5 * (b.t_s + t_now));

        let allan_x: Vec<f64> =
            self.bins.iter().filter(|b| b.t_s >= t_now - self.cfg.allan_window_s).map(|b| b.mean).collect();
        let allan_best = allan::best_averaging(&allan::allan_deviation(&allan_x, self.cfg.bin_s));

        let settle = if since >= self.cfg.min_fit_s {
            let (t, x) = thinned(&self.bins, FIT_POINTS);
            settle::fit(&t, &x, self.epoch_s)
        } else {
            None
        };

        let readiness = match rate {
            Some(rate) if since >= self.cfg.min_fit_s => self.readiness(plan, noise, rate, rate_mid, settle.as_ref()),
            _ => Readiness::Collecting { have_s: since, need_s: self.cfg.min_fit_s },
        };
        Status {
            t_s: t_now,
            since_s: self.epoch_s,
            latest: self.bins.last().map(|b| b.mean),
            rate_per_min: rate.map(|r| r.0 * 60.0),
            rate_sigma_per_min: rate.map(|r| r.1 * 60.0),
            noise,
            allan_best,
            settle,
            steps: self.steps.clone(),
            readiness,
        }
    }

    fn readiness(
        &self,
        plan: &RunPlan,
        noise: Option<f64>,
        (rate, rate_sigma): (f64, f64),
        rate_mid: f64,
        settle: Option<&SettleFit>,
    ) -> Readiness {
        let t_now = self.last_s;
        if let Some(n) = noise.filter(|&n| n > plan.tolerance) {
            return Readiness::Noisy { noise: n };
        }
        if let Some(fit) = settle.filter(|f| f.tau_high_s.is_some()) {
            let model = fit.rate_at(rate_mid);
            if (model - rate).abs() <= (3.0 * rate_sigma).max(0.25 * model.abs()) {
                let r = fit.ready_at(plan.run_s, plan.tolerance);
                if r.best_s > t_now {
                    return Readiness::Settling {
                        in_s: r.best_s - t_now,
                        earliest_in_s: (r.earliest_s - t_now).max(0.0),
                        latest_in_s: r.latest_s.map(|l| (l - t_now).max(0.0)),
                    };
                }
                // The fit says ready. On noisy readings any fit agrees with the rate, so trust it
                // only when the rate was precise enough to check it at the scale of the
                // tolerance. Otherwise the rate test below decides.
                if 2.0 * rate_sigma * plan.run_s <= plan.tolerance {
                    return Readiness::Ready;
                }
            }
        }

        // No settle curve: a straight drift, or none. Measure the rate over up to a run's worth of
        // readings, and decide only when it is two standard errors clear of the allowed rate.
        let limit = plan.tolerance / plan.run_s.max(1e-9);
        let span = self.cfg.rate_window_s.max(plan.run_s).min(self.cfg.max_bins as f64 * self.cfg.bin_s);
        let (r, rs) = long_rate(&self.bins, t_now, span).unwrap_or((rate, rate_sigma));
        let gap = r.abs() - limit;
        if gap <= -2.0 * rs {
            return Readiness::Ready;
        }
        if gap > 2.0 * rs {
            return Readiness::Drifting { rate_per_min: r * 60.0 };
        }
        // Not sure yet. On white noise the error falls as the span to the power 1.5, so more
        // readings settle it, unless even a full span is not enough.
        let have = (t_now - self.epoch_s).clamp(0.0, span);
        let sigma_at_span = rs * (have / span).powf(1.5);
        if gap.abs() > 2.0 * sigma_at_span {
            let need = have * (2.0 * rs / gap.abs()).powf(2.0 / 3.0);
            return Readiness::Collecting { have_s: have, need_s: need.clamp(have, span) };
        }
        match noise {
            Some(n) if 2.0 * sigma_at_span >= 0.5 * limit => Readiness::Noisy { noise: n },
            _ => Readiness::Drifting { rate_per_min: r * 60.0 },
        }
    }
}

/// Most points the settle fit takes. Its time grows with its points, about 1 ms for each 1,000, and
/// it runs once a second while the engine is locked. Past this, the bins go in runs that are
/// averaged first: 15 minutes of 1 s bins go in as they are, 4 hours in runs of 16.
const FIT_POINTS: usize = 900;

/// Times and readings of at most `most` points: the bins, or the means of runs of them. The runs
/// are counted from the newest bin, so a short run, if any, is the oldest.
fn thinned(bins: &[Bin], most: usize) -> (Vec<f64>, Vec<f64>) {
    let k = bins.len().div_ceil(most.max(1)).max(1);
    let mut points: Vec<(f64, f64)> = bins
        .rchunks(k)
        .map(|run| {
            let n = run.len() as f64;
            (run.iter().map(|b| b.t_s).sum::<f64>() / n, run.iter().map(|b| b.mean).sum::<f64>() / n)
        })
        .collect();
    points.reverse();
    points.into_iter().unzip()
}

/// Drift rate for the Ready test, from the bins in the last `span_s`. The bins go into at most 240
/// groups first, since the Theil–Sen slope takes all pairs.
fn long_rate(bins: &[Bin], t_now: f64, span_s: f64) -> Option<(f64, f64)> {
    let bins = &bins[bins.partition_point(|b| b.t_s < t_now - span_s)..];
    let groups: Vec<Bin> = bins
        .chunks(bins.len().div_ceil(240).max(1))
        .map(|c| {
            let n = c.len() as f64;
            Bin {
                t_s: c.iter().map(|b| b.t_s).sum::<f64>() / n,
                mean: c.iter().map(|b| b.mean).sum::<f64>() / n,
                count: c.iter().map(|b| b.count).sum(),
            }
        })
        .collect();
    if groups.len() < 6 {
        return None;
    }
    robust_rate(&groups.iter().collect::<Vec<_>>())
}

/// Theil–Sen slope per s and its standard error.
fn robust_rate(bins: &[&Bin]) -> Option<(f64, f64)> {
    let t: Vec<f64> = bins.iter().map(|b| b.t_s).collect();
    let x: Vec<f64> = bins.iter().map(|b| b.mean).collect();
    let slope = stats::theil_sen(&t, &x)?;
    let mt = t.iter().sum::<f64>() / t.len() as f64;
    let resid: Vec<f64> = t.iter().zip(&x).map(|(ti, xi)| xi - slope * (ti - mt)).collect();
    let mid = stats::median(&resid)?;
    let resid: Vec<f64> = resid.iter().map(|r| r - mid).collect();
    let sxx: f64 = t.iter().map(|ti| (ti - mt).powi(2)).sum();
    let sigma = stats::mad_sigma(&resid)? * stats::correlation_inflation(&resid).sqrt() / sxx.sqrt();
    Some((slope, sigma))
}

struct Jump {
    size: f64,
    detrended: Vec<f64>,
}

/// Jump between `bins[..split]` and `bins[split..]`: the drift slope comes from pairs inside each
/// side only, so the jump does not tilt it; the size is the difference of the detrended medians.
fn jump(bins: &[Bin], split: usize) -> Option<Jump> {
    let mut slopes = Vec::new();
    for side in [&bins[..split], &bins[split..]] {
        for i in 0..side.len() {
            for j in i + 1..side.len() {
                slopes.push((side[j].mean - side[i].mean) / (side[j].t_s - side[i].t_s));
            }
        }
    }
    let slope = stats::median(&slopes)?;
    let t0 = bins.first()?.t_s;
    let detrended: Vec<f64> = bins.iter().map(|b| b.mean - slope * (b.t_s - t0)).collect();
    let size = stats::median(&detrended[split..])? - stats::median(&detrended[..split])?;
    Some(Jump { size, detrended })
}

/// Standard error of `jump`'s size on noise alone, for `w` bins a side of noise `sigma` each.
/// The two medians add (π/2)σ²/w each. The slope, from pairs inside each side, is off by about
/// σ/sqrt(0.88·Sxx), Sxx = w(w²−1)/6 bins², and that error moves the size by the w bins between
/// the centres of the two sides. Theil–Sen keeps about 0.88 of the least-squares efficiency here;
/// a test checks the whole on simulated noise.
fn step_se(sigma: f64, w: usize) -> f64 {
    let w = w as f64;
    sigma * (std::f64::consts::PI / w + 6.0 * w / (0.88 * (w * w - 1.0))).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn the_fit_takes_short_waits_whole_and_long_ones_in_runs() {
        let bins =
            |n: usize| -> Vec<Bin> { (0..n).map(|k| Bin { t_s: k as f64, mean: 2.0 * k as f64, count: 30 }).collect() };
        let (t, x) = thinned(&bins(900), FIT_POINTS);
        assert_eq!((t.len(), t[899], x[899]), (900, 899.0, 1798.0));
        // Runs of 2 from the newest bin: the oldest bin is alone.
        let (t, _) = thinned(&bins(901), FIT_POINTS);
        assert_eq!((t.len(), t[0], t[1], t[450]), (451, 0.0, 1.5, 899.5));
        // 4 hours: runs of 16.
        let (t, x) = thinned(&bins(14_400), FIT_POINTS);
        assert_eq!((t.len(), t[899], x[899]), (900, 14_391.5, 28_783.0));
        assert!(thinned(&[], FIT_POINTS).0.is_empty());
    }

    /// On pure noise, the size of a jump spreads by its standard error, so `step_sigmas` means
    /// what it says.
    #[test]
    fn step_error_matches_pure_noise() {
        for w in [8, 15, 30] {
            let mut rng = Rng::new(0x5eed + w as u64);
            let runs = 4000;
            let mut sum_z2 = 0.0;
            for _ in 0..runs {
                let bins: Vec<Bin> = (0..2 * w).map(|k| Bin { t_s: k as f64, mean: rng.normal(), count: 30 }).collect();
                let z = jump(&bins, w).unwrap().size / step_se(1.0, w);
                sum_z2 += z * z;
            }
            let rms = (sum_z2 / runs as f64).sqrt();
            assert!((rms - 1.0).abs() < 0.06, "w {w}: the jump spreads {rms:.3} standard errors");
        }
    }
}
