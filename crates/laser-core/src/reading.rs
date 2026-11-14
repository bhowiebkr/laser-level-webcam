//! A reading at a station: the frames of one sample averaged into one position, with an
//! uncertainty that allows for frames that move together (air currents, a slow wobble).
//!
//! Frames far from the median are left out, the rest are averaged, and the standard error is
//! raised for the lag-1 correlation of the frames. The reading carries flags for what makes it less
//! trustworthy, so the results can show them next to each station.

use serde::{Deserialize, Serialize};

use crate::beam::{Fix, NoBeam, Warnings};
use crate::stats;

/// Frames farther than this from the median, in robust sigmas, are left out of the mean.
const OUTLIER_SIGMAS: f64 = 5.0;
/// A frame warning shows on the reading when more than this share of frames carry it.
const FLAG_SHARE: f64 = 0.1;
/// Fewer frames used than this raises `few_frames`.
const MIN_FRAMES: usize = 5;
/// The two halves of a sample differ by more than this many standard errors of their difference:
/// `moving`. High, because the correlation of a short half is a rough estimate.
const MOVING_SE: f64 = 8.0;
/// The moving check needs halves of at least five frames.
const MOVING_FRAMES: usize = 10;

/// One frame of a sample. Times are seconds on one clock for the whole session.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Subsample {
    pub t_s: f64,
    pub frame: FrameResult,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum FrameResult {
    Fix {
        px: f64,
        uncertainty_px: f64,
        #[serde(default)]
        warnings: Warnings,
    },
    NoBeam {
        reason: NoBeam,
    },
}

impl Subsample {
    pub fn new(t_s: f64, result: &Result<Fix, NoBeam>) -> Self {
        let frame = match result {
            Ok(f) => FrameResult::Fix { px: f.position_px, uncertainty_px: f.uncertainty_px, warnings: f.warnings },
            Err(e) => FrameResult::NoBeam { reason: *e },
        };
        Self { t_s, frame }
    }
}

/// What makes a reading or a height less trustworthy. The value is still given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Flags {
    /// Frames were clipped. Lower the exposure, the gain or the laser power.
    pub clipped: bool,
    /// The line is not square to the sensor.
    pub tilted: bool,
    /// The beam is close to the sensor edge.
    pub near_edge: bool,
    /// Some frames had no beam.
    pub dropouts: bool,
    /// Frames far from the rest were left out.
    pub outliers: bool,
    /// The reading moved during the sample: the block was still settling, or it was bumped.
    pub moving: bool,
    /// Too few frames for a trustworthy uncertainty.
    pub few_frames: bool,
    /// Taken before the first or after the last zero visit, so the drift since the zero is not
    /// checked. Measure the zero station again.
    pub drift_unchecked: bool,
}

impl Flags {
    pub fn any(&self) -> bool {
        self.names().next().is_some()
    }

    /// Names of the flags that are set, in snake case.
    pub fn names(&self) -> impl Iterator<Item = &'static str> {
        [
            (self.clipped, "clipped"),
            (self.tilted, "tilted"),
            (self.near_edge, "near_edge"),
            (self.dropouts, "dropouts"),
            (self.outliers, "outliers"),
            (self.moving, "moving"),
            (self.few_frames, "few_frames"),
            (self.drift_unchecked, "drift_unchecked"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
    }

    /// Flags set in either.
    pub fn union(self, o: Flags) -> Flags {
        Flags {
            clipped: self.clipped || o.clipped,
            tilted: self.tilted || o.tilted,
            near_edge: self.near_edge || o.near_edge,
            dropouts: self.dropouts || o.dropouts,
            outliers: self.outliers || o.outliers,
            moving: self.moving || o.moving,
            few_frames: self.few_frames || o.few_frames,
            drift_unchecked: self.drift_unchecked || o.drift_unchecked,
        }
    }
}

/// One sample's frames reduced to a position.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    /// Mean time of the frames used, s. Drift correction interpolates at this time.
    pub t_s: f64,
    /// From the first frame to the last, s.
    pub duration_s: f64,
    /// Mean line position of the frames used, px.
    pub px: f64,
    /// Standard uncertainty of `px`, allowing for correlated frames, px.
    pub uncertainty_px: f64,
    /// Frame-to-frame standard deviation of the frames used, px.
    pub scatter_px: f64,
    /// Frames in the sample.
    pub frames: usize,
    /// Frames with a beam.
    pub fixes: usize,
    /// Frames with a beam that were not outliers.
    pub used: usize,
    pub flags: Flags,
}

struct Frame {
    t: f64,
    px: f64,
    unc: f64,
    warnings: Warnings,
}

impl Reading {
    /// `None` when no frame had a beam.
    pub fn from_subsamples(subs: &[Subsample]) -> Option<Reading> {
        let mut fixes: Vec<Frame> = subs
            .iter()
            .filter_map(|s| match s.frame {
                FrameResult::Fix { px, uncertainty_px, warnings } => {
                    Some(Frame { t: s.t_s, px, unc: uncertainty_px, warnings })
                }
                FrameResult::NoBeam { .. } => None,
            })
            .collect();
        if fixes.is_empty() {
            return None;
        }
        fixes.sort_by(|a, b| a.t.total_cmp(&b.t));
        let n_fixes = fixes.len();

        let px: Vec<f64> = fixes.iter().map(|f| f.px).collect();
        let unc: Vec<f64> = fixes.iter().map(|f| f.unc).collect();
        let med = stats::median(&px)?;
        let spread = stats::mad_sigma(&px)?.max(stats::median(&unc)?);
        let used: Vec<&Frame> = if spread > 0.0 {
            fixes.iter().filter(|f| (f.px - med).abs() <= OUTLIER_SIGMAS * spread).collect()
        } else {
            fixes.iter().collect()
        };
        let n = used.len();
        let nf = n as f64;
        let (mean, uncertainty_px, sd) = mean_se(&used);

        // A step or a slope inside the sample inflates the correlation of the whole sample, so the
        // halves are judged against their own scatter.
        let half = n / 2;
        let moving = n >= MOVING_FRAMES && {
            let (a, ua, _) = mean_se(&used[..half]);
            let (b, ub, _) = mean_se(&used[half..]);
            (b - a).abs() > MOVING_SE * ua.hypot(ub)
        };
        let share = |count: usize, of: usize| count as f64 > FLAG_SHARE * of as f64;
        let warned = |w: fn(&Warnings) -> bool| share(used.iter().filter(|f| w(&f.warnings)).count(), n);
        let flags = Flags {
            clipped: warned(|w| w.clipped),
            tilted: warned(|w| w.tilted),
            near_edge: warned(|w| w.near_edge),
            dropouts: share(subs.len() - n_fixes, subs.len()),
            outliers: share(n_fixes - n, n_fixes),
            moving,
            few_frames: n < MIN_FRAMES,
            drift_unchecked: false,
        };
        let (first, last) = (fixes[0].t, fixes[n_fixes - 1].t);
        Some(Reading {
            t_s: used.iter().map(|f| f.t).sum::<f64>() / nf,
            duration_s: last - first,
            px: mean,
            uncertainty_px,
            scatter_px: sd,
            frames: subs.len(),
            fixes: n_fixes,
            used: n,
            flags,
        })
    }
}

/// Mean, its standard uncertainty (allowing for correlated frames, and no less than the frames'
/// own uncertainty allows) and the scatter.
fn mean_se(fs: &[&Frame]) -> (f64, f64, f64) {
    let nf = fs.len() as f64;
    let mean = fs.iter().map(|f| f.px).sum::<f64>() / nf;
    let mean_unc = fs.iter().map(|f| f.unc).sum::<f64>() / nf;
    let resid: Vec<f64> = fs.iter().map(|f| f.px - mean).collect();
    let sd = if fs.len() > 1 { (resid.iter().map(|r| r * r).sum::<f64>() / (nf - 1.0)).sqrt() } else { 0.0 };
    let se = sd * (stats::correlation_inflation(&resid) / nf).sqrt();
    (mean, se.max(mean_unc / nf.sqrt()), sd)
}

/// When a sample ends.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SampleRule {
    /// Shortest sample, s. With no target, every sample is this long.
    pub min_s: f64,
    /// Longest sample, s. It ends here even when the target is not met.
    pub max_s: f64,
    /// End after `min_s` as soon as the uncertainty is at most this, µm.
    pub target_um: Option<f64>,
}

impl Default for SampleRule {
    fn default() -> Self {
        Self { min_s: 2.0, max_s: 10.0, target_um: None }
    }
}

impl SampleRule {
    pub fn done(&self, elapsed_s: f64, reading: Option<&Reading>, um_per_px: f64) -> bool {
        if elapsed_s >= self.max_s {
            return true;
        }
        if elapsed_s < self.min_s {
            return false;
        }
        match (self.target_um, reading) {
            (None, _) => true,
            (Some(target), Some(r)) => !r.flags.few_frames && r.uncertainty_px * um_per_px <= target,
            (Some(_), None) => false,
        }
    }
}

/// Collects the frames of one sample.
#[derive(Clone, Debug, PartialEq)]
pub struct Sampler {
    start_s: f64,
    subs: Vec<Subsample>,
}

impl Sampler {
    pub fn new(start_s: f64) -> Self {
        Self { start_s, subs: Vec::new() }
    }

    pub fn push(&mut self, t_s: f64, result: &Result<Fix, NoBeam>) {
        self.subs.push(Subsample::new(t_s, result));
    }

    pub fn subsamples(&self) -> &[Subsample] {
        &self.subs
    }

    pub fn reading(&self) -> Option<Reading> {
        Reading::from_subsamples(&self.subs)
    }

    pub fn done(&self, now_s: f64, rule: &SampleRule, um_per_px: f64) -> bool {
        rule.done(now_s - self.start_s, self.reading().as_ref(), um_per_px)
    }

    pub fn finish(self) -> Vec<Subsample> {
        self.subs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    fn frame(t_s: f64, px: f64) -> Subsample {
        Subsample { t_s, frame: FrameResult::Fix { px, uncertainty_px: 0.05, warnings: Warnings::default() } }
    }

    /// Frames that move together (lag-1 correlation 0.8): the reported uncertainty still matches
    /// the scatter of the readings about the truth.
    #[test]
    fn uncertainty_is_honest_for_correlated_frames() {
        let mut rng = Rng::new(11);
        let (rho, sd) = (0.8_f64, 0.5);
        let mut z = Vec::new();
        for _ in 0..300 {
            let mut e = sd * rng.normal();
            let subs: Vec<Subsample> = (0..60)
                .map(|k| {
                    e = rho * e + sd * (1.0 - rho * rho).sqrt() * rng.normal();
                    frame(f64::from(k) / 20.0, 100.0 + e)
                })
                .collect();
            let r = Reading::from_subsamples(&subs).unwrap();
            z.push((r.px - 100.0) / r.uncertainty_px);
            assert!(!r.flags.few_frames && !r.flags.outliers, "{r:?}");
        }
        let rms = (z.iter().map(|v| v * v).sum::<f64>() / z.len() as f64).sqrt();
        assert!((0.75..1.35).contains(&rms), "z rms {rms}");
    }

    #[test]
    fn outliers_and_dropouts() {
        let mut subs: Vec<Subsample> =
            (0..40).map(|k| frame(f64::from(k) * 0.05, 50.0 + 0.01 * f64::from(k % 3))).collect();
        subs[7] = frame(0.35, 80.0);
        for s in subs.iter_mut().skip(30) {
            s.frame = FrameResult::NoBeam { reason: NoBeam::Flat };
        }
        let r = Reading::from_subsamples(&subs).unwrap();
        assert_eq!((r.frames, r.fixes, r.used), (40, 30, 29));
        assert!((r.px - 50.01).abs() < 0.01, "{r:?}");
        assert!(r.flags.dropouts && !r.flags.outliers, "{r:?}");
        let none: Vec<Subsample> = subs[30..].to_vec();
        assert_eq!(Reading::from_subsamples(&none), None);
    }

    #[test]
    fn a_bump_during_the_sample_is_moving() {
        let mut rng = Rng::new(3);
        let subs: Vec<Subsample> =
            (0..40).map(|k| frame(f64::from(k) * 0.05, if k < 20 { 0.0 } else { 2.0 } + 0.2 * rng.normal())).collect();
        assert!(Reading::from_subsamples(&subs).unwrap().flags.moving);
        let calm: Vec<Subsample> = (0..40).map(|k| frame(f64::from(k) * 0.05, 0.2 * rng.normal())).collect();
        assert!(!Reading::from_subsamples(&calm).unwrap().flags.moving);
    }

    #[test]
    fn sample_rule() {
        let rule = SampleRule { min_s: 2.0, max_s: 10.0, target_um: Some(0.1) };
        let mut s = Sampler::new(0.0);
        let mut rng = Rng::new(5);
        let mut ended = None;
        for k in 0..400 {
            let t = f64::from(k) * 0.05;
            let fix = Fix {
                position_px: 10.0 + rng.normal(),
                uncertainty_px: 0.05,
                tilt: 0.0,
                fwhm_px: 100.0,
                peak: 100.0,
                snr: 50.0,
                strips_used: 16,
                warnings: Warnings::default(),
            };
            s.push(t, &Ok(fix));
            if s.done(t, &rule, 1.0) {
                ended = Some(t);
                break;
            }
        }
        // White noise of 1 px reaches 0.1 px after about 100 frames, 5 s.
        let t = ended.unwrap();
        assert!((3.0..10.0).contains(&t), "ended at {t}");
        assert!(SampleRule { target_um: None, ..rule }.done(2.0, None, 1.0));
        assert!(rule.done(10.0, None, 1.0));
    }
}
