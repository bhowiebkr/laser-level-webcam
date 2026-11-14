//! Where the laser line crosses the sensor, with its uncertainty, or why there is no reading.
//!
//! The finder assumes nothing about the beam shape except that it is roughly symmetric. A real
//! beam on a sensor with no lens is not a Gaussian (speckle, diffraction, a dirty window), so there
//! is no model fit.
//!
//! 1. The frame is summed into strips along the line. Their sum is one profile across the sensor.
//! 2. On that profile: the peak, its half-maximum crossings (FWHM), the background (a line fitted
//!    outside the beam, so an ambient gradient does not pull the centre) and the noise. A profile
//!    whose brightest part runs into the sensor edge is no reading: it is a beam off the side, or
//!    only an ambient gradient.
//! 3. In each strip: a centroid in a window of ± `window_sigmas` beam sigmas, moved until it is
//!    centred on its own centroid. If that window does not fit on the sensor, the middle of the
//!    half-maximum crossings is used instead, with a warning.
//! 4. A line through the strip centres gives the position at the middle of the sensor, the tilt,
//!    and the uncertainty from their scatter.
//!
//! The position stays a float from end to end. There is no smoothing in the reading, so nothing
//! changes the scale.

use serde::{Deserialize, Serialize};

use crate::frame::{LineAxis, Luma, Roi};
use crate::stats;

/// FWHM over sigma for a Gaussian.
const FWHM_PER_SIGMA: f64 = 2.354_820_045_030_949;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FinderConfig {
    pub axis: LineAxis,
    /// Strips along the line. At least 3.
    pub strips: usize,
    /// Lowest peak height over the profile noise that counts as a beam.
    pub min_snr: f64,
    /// Half width of the centroid window, in beam sigmas (FWHM / 2.355).
    pub window_sigmas: f64,
    /// The background is fitted farther than this from the beam centre, in beam sigmas, where
    /// the beam's tails no longer add light.
    pub background_sigmas: f64,
    /// Grey level at which a pixel counts as clipped.
    pub clip_level: u8,
    /// Fraction of clipped pixels in the beam window that raises the clipped warning.
    pub max_clipped: f64,
    /// Tilt (px per px) above which the tilted warning is raised.
    pub max_tilt: f64,
    /// Widest beam, as a fraction of the sensor, that counts as a beam and not as ambient light.
    pub max_width: f64,
    /// Part of the frame to search. Positions are still in whole-frame pixels. `None` is the whole frame.
    pub roi: Option<Roi>,
}

impl Default for FinderConfig {
    fn default() -> Self {
        Self {
            axis: LineAxis::Vertical,
            strips: 16,
            min_snr: 10.0,
            window_sigmas: 2.5,
            background_sigmas: 4.0,
            clip_level: 254,
            max_clipped: 0.0005,
            max_tilt: 0.02,
            max_width: 0.6,
            roi: None,
        }
    }
}

/// Why a frame gives no reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoBeam {
    /// No peak stands above the noise.
    Flat,
    /// The brightest part runs into the sensor edge: a beam off the side, or only an ambient gradient.
    Edge,
    /// The bright part is wider than `max_width` of the sensor: ambient light, or a beam far too wide.
    TooWide,
    /// Fewer than 3 strips gave a centre.
    FewStrips,
}

impl NoBeam {
    pub fn text(self) -> &'static str {
        match self {
            Self::Flat => "no beam on the sensor",
            Self::Edge => "the beam is off the edge of the sensor, or there is only ambient light",
            Self::TooWide => "the bright part is too wide to be the beam",
            Self::FewStrips => "the beam was found in too few strips",
        }
    }
}

/// Things that make a reading less trustworthy. The reading is still given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warnings {
    /// Pixels in the beam are at full scale. Lower the exposure, the gain or the laser power.
    pub clipped: bool,
    /// The line is not square to the sensor.
    pub tilted: bool,
    /// The centroid window does not fit on the sensor, so the centre comes from the half-maximum
    /// crossings, which a lopsided beam pulls.
    pub near_edge: bool,
    /// Some strips gave no centre or were dropped as outliers.
    pub strips_dropped: bool,
}

impl Warnings {
    pub fn any(&self) -> bool {
        self.clipped || self.tilted || self.near_edge || self.strips_dropped
    }
}

/// A reading from one frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fix {
    /// Line position across the sensor, at the middle of the sensor along the line, px.
    /// Pixel i covers i − 0.5 to i + 0.5.
    pub position_px: f64,
    /// Standard uncertainty of `position_px` within this frame, from the scatter of the strip
    /// centres, px. Beam motion between frames is not in it; the monitor measures that.
    pub uncertainty_px: f64,
    /// Change of position per px along the line.
    pub tilt: f64,
    /// Full width at half maximum of the summed profile, px.
    pub fwhm_px: f64,
    /// Peak height above the background, grey levels.
    pub peak: f64,
    /// Peak height over the profile noise.
    pub snr: f64,
    pub strips_used: usize,
    pub warnings: Warnings,
}

/// Find the line in one frame, inside the region of interest if there is one.
pub fn find(frame: &Luma<'_>, cfg: &FinderConfig) -> Result<Fix, NoBeam> {
    let (view, offset) = region(frame, cfg);
    find_in(&view, cfg).map(|fix| Fix { position_px: fix.position_px + offset as f64, ..fix })
}

/// The part of the frame to search, and the whole-frame px of its first px across the line.
fn region<'a>(frame: &Luma<'a>, cfg: &FinderConfig) -> (Luma<'a>, usize) {
    match cfg.roi.and_then(|r| r.clamped(frame.width(), frame.height())) {
        Some(roi) => (
            frame.crop(roi),
            match cfg.axis {
                LineAxis::Vertical => roi.x,
                LineAxis::Horizontal => roi.y,
            },
        ),
        None => (*frame, 0),
    }
}

/// The finder's view of a frame, for drawing: the profile across the line, summed along the
/// whole line, and the zones the finder uses on it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Inspection {
    /// Whole-frame px of `profile[0]`: the region's edge across the line, or 0.
    pub offset_px: usize,
    /// Mean grey level of each px across the line.
    pub profile: Vec<f64>,
    pub zones: Result<Zones, NoBeam>,
}

/// Where the finder looked, in px of the profile (add `offset_px` for whole-frame px).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Zones {
    pub centre: f64,
    pub fwhm: f64,
    /// Peak height above the background, grey levels.
    pub peak: f64,
    pub snr: f64,
    /// Half width of the centroid window.
    pub half_width: f64,
    /// The centroid window fits on the sensor. When it does not, each strip's centre comes from
    /// the half-maximum crossings instead.
    pub centroid_fits: bool,
    /// The background is fitted on `[0, background_left)` and `[background_right, n)`.
    pub background_left: usize,
    pub background_right: usize,
    /// Background level at px i of the profile: `background.0 + background.1 · i`.
    pub background: (f64, f64),
}

/// Look at a frame the way [`find`] does, for drawing. It sums the frame again, so call it
/// for the frames that are shown, not for every frame.
pub fn inspect(frame: &Luma<'_>, cfg: &FinderConfig) -> Inspection {
    let (view, offset_px) = region(frame, cfg);
    let profile = Strips::accumulate(&view, cfg.axis, cfg.strips.max(3)).total();
    let zones = Shape::of(&profile, cfg).map(|s| {
        let n = profile.len() as f64;
        let half_width = cfg.window_sigmas * s.sigma();
        let base = stats::quantile(&profile, 0.1).unwrap_or(0.0);
        let bg = Background::fit(&profile, s.left_end, s.right_start).unwrap_or(Background::flat(base));
        Zones {
            centre: s.centre,
            fwhm: s.fwhm,
            peak: s.peak,
            snr: s.snr,
            half_width,
            centroid_fits: s.centre - half_width >= -0.5 && s.centre + half_width <= n - 0.5,
            background_left: s.left_end,
            background_right: s.right_start,
            background: (bg.a, bg.b),
        }
    });
    Inspection { offset_px, profile, zones }
}

fn find_in(frame: &Luma<'_>, cfg: &FinderConfig) -> Result<Fix, NoBeam> {
    let strips = Strips::accumulate(frame, cfg.axis, cfg.strips.max(3));
    if strips.count < 3 {
        return Err(NoBeam::FewStrips);
    }
    let total = strips.total();
    let shape = Shape::of(&total, cfg)?;
    let n = total.len() as f64;
    let half_width = cfg.window_sigmas * shape.sigma();
    let centroid_fits = shape.centre - half_width >= -0.5 && shape.centre + half_width <= n - 0.5;

    let mut along = Vec::with_capacity(strips.count);
    let mut centres = Vec::with_capacity(strips.count);
    for s in 0..strips.count {
        let p = strips.profile(s);
        let bg = Background::fit(&p, shape.left_end, shape.right_start)
            .unwrap_or_else(|| Background::flat(stats::quantile(&p, 0.1).unwrap_or(0.0)));
        let centre = if centroid_fits {
            centroid(&p, &bg, shape.centre, half_width)
        } else {
            half_max_centre(&box_smooth(&p, shape.smooth), &bg, shape.centre, shape.fwhm)
        };
        if let Some(c) = centre {
            along.push(strips.along_mid[s]);
            centres.push(c);
        }
    }
    let line = LineFit::of(&along, &centres, (strips.along_len as f64 - 1.0) / 2.0).ok_or(NoBeam::FewStrips)?;

    let clipped = clipped_fraction(frame, cfg.axis, line.position, half_width.max(shape.fwhm), cfg.clip_level);
    let warnings = Warnings {
        clipped: clipped > cfg.max_clipped,
        tilted: line.slope.abs() > cfg.max_tilt,
        near_edge: !centroid_fits,
        strips_dropped: line.used < strips.count,
    };
    Ok(Fix {
        position_px: line.position,
        uncertainty_px: line.uncertainty,
        tilt: line.slope,
        fwhm_px: shape.fwhm,
        peak: shape.peak,
        snr: shape.snr,
        strips_used: line.used,
        warnings,
    })
}

/// The frame summed into strips along the line.
struct Strips {
    count: usize,
    across: usize,
    along_len: usize,
    sums: Vec<u32>,
    pixels: Vec<u32>,
    along_mid: Vec<f64>,
}

impl Strips {
    fn accumulate(frame: &Luma<'_>, axis: LineAxis, wanted: usize) -> Self {
        let (w, h) = (frame.width(), frame.height());
        let (across, along_len) = match axis {
            LineAxis::Vertical => (w, h),
            LineAxis::Horizontal => (h, w),
        };
        let count = wanted.clamp(1, along_len);
        let strip_of = |l: usize| l * count / along_len;
        let start_of = |s: usize| (s * along_len).div_ceil(count);

        let mut pixels = vec![0u32; count];
        let mut along_sum = vec![0f64; count];
        for l in 0..along_len {
            pixels[strip_of(l)] += 1;
            along_sum[strip_of(l)] += l as f64;
        }
        let mut sums = vec![0u32; count * across];
        match axis {
            LineAxis::Vertical => {
                for y in 0..h {
                    let s = strip_of(y);
                    let dst = &mut sums[s * across..(s + 1) * across];
                    for (d, &p) in dst.iter_mut().zip(frame.row(y)) {
                        *d += u32::from(p);
                    }
                }
            }
            LineAxis::Horizontal => {
                for y in 0..h {
                    let row = frame.row(y);
                    for s in 0..count {
                        let part = &row[start_of(s)..start_of(s + 1)];
                        sums[s * across + y] += part.iter().map(|&p| u32::from(p)).sum::<u32>();
                    }
                }
            }
        }
        let along_mid = along_sum.iter().zip(&pixels).map(|(sum, &n)| sum / f64::from(n.max(1))).collect();
        Self { count, across, along_len, sums, pixels, along_mid }
    }

    /// Mean grey level across the sensor in strip `s`.
    fn profile(&self, s: usize) -> Vec<f64> {
        let n = f64::from(self.pixels[s].max(1));
        self.sums[s * self.across..(s + 1) * self.across].iter().map(|&v| f64::from(v) / n).collect()
    }

    /// Mean grey level across the sensor over the whole frame.
    fn total(&self) -> Vec<f64> {
        let mut t = vec![0f64; self.across];
        for s in 0..self.count {
            for (ti, &v) in t.iter_mut().zip(&self.sums[s * self.across..(s + 1) * self.across]) {
                *ti += f64::from(v);
            }
        }
        let n = self.along_len as f64;
        t.iter_mut().for_each(|v| *v /= n);
        t
    }
}

/// The beam in the summed profile.
struct Shape {
    centre: f64,
    fwhm: f64,
    peak: f64,
    snr: f64,
    /// The background is fitted on `[0, left_end)` and `[right_start, n)`.
    left_end: usize,
    right_start: usize,
    /// Half width of the box filter used to find the peak and the crossings.
    smooth: usize,
}

impl Shape {
    fn sigma(&self) -> f64 {
        self.fwhm / FWHM_PER_SIGMA
    }

    fn of(p: &[f64], cfg: &FinderConfig) -> Result<Self, NoBeam> {
        let n = p.len();
        if n < 16 {
            return Err(NoBeam::Flat);
        }
        let smooth = (n / 400).max(1);
        let sm = box_smooth(p, smooth);
        let base = stats::quantile(p, 0.1).unwrap_or(0.0);
        let noise0 = stats::diff_noise(p).unwrap_or(0.0).max(1e-3);
        let (imax, top) = sm.iter().copied().enumerate().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap_or((0, 0.0));
        let height = top - base;
        if height < cfg.min_snr * noise0 {
            return Err(NoBeam::Flat);
        }
        let half = base + 0.5 * height;
        let left = crossing(&sm, imax, half, -1).ok_or(NoBeam::Edge)?;
        let right = crossing(&sm, imax, half, 1).ok_or(NoBeam::Edge)?;
        let fwhm = right - left;
        if fwhm > cfg.max_width * n as f64 {
            return Err(NoBeam::TooWide);
        }
        let centre = 0.5 * (left + right);
        let reach = cfg.window_sigmas.max(cfg.background_sigmas) * fwhm / FWHM_PER_SIGMA + 1.0;
        let left_end = (centre - reach).floor().clamp(0.0, n as f64) as usize;
        let right_start = ((centre + reach).ceil() + 1.0).clamp(0.0, n as f64) as usize;

        let bg = Background::fit(p, left_end, right_start).unwrap_or(Background::flat(base));
        let noise = bg.noise(p, left_end, right_start).unwrap_or(noise0).max(1e-3);
        let peak = top - bg.at(imax as f64);
        let snr = peak / noise;
        if snr < cfg.min_snr {
            return Err(NoBeam::Flat);
        }
        Ok(Self { centre, fwhm, peak, snr, left_end, right_start, smooth })
    }
}

/// Background level `a + b·i` across the sensor.
#[derive(Clone, Copy, Debug)]
struct Background {
    a: f64,
    b: f64,
}

impl Background {
    fn flat(level: f64) -> Self {
        Self { a: level, b: 0.0 }
    }

    fn at(&self, i: f64) -> f64 {
        self.a + self.b * i
    }

    fn outside(p: &[f64], left_end: usize, right_start: usize) -> impl Iterator<Item = (f64, f64)> + '_ {
        let right = right_start.min(p.len());
        (0..left_end.min(right)).chain(right..p.len()).map(|i| (i as f64, p[i]))
    }

    /// A line through both sides of the beam when each side has a few pixels, a level from
    /// whatever is there otherwise, or `None` with too few pixels.
    fn fit(p: &[f64], left_end: usize, right_start: usize) -> Option<Self> {
        let nl = left_end.min(p.len());
        let nr = p.len().saturating_sub(right_start);
        if nl + nr < 8 {
            return None;
        }
        let (x, y): (Vec<f64>, Vec<f64>) = Self::outside(p, left_end, right_start).unzip();
        if nl >= 4 && nr >= 4 {
            let (a, b) = stats::line_fit(&x, &y)?;
            Some(Self { a, b })
        } else {
            Some(Self::flat(stats::median(&y)?))
        }
    }

    fn noise(&self, p: &[f64], left_end: usize, right_start: usize) -> Option<f64> {
        let r: Vec<f64> = Self::outside(p, left_end, right_start).map(|(x, y)| y - self.at(x)).collect();
        if r.len() < 8 { None } else { stats::mad_sigma(&r) }
    }
}

/// Moving average with half width `r`, clipped at the ends.
fn box_smooth(p: &[f64], r: usize) -> Vec<f64> {
    let n = p.len();
    let mut cum = Vec::with_capacity(n + 1);
    cum.push(0.0);
    for v in p {
        cum.push(cum.last().copied().unwrap_or(0.0) + v);
    }
    (0..n)
        .map(|i| {
            let lo = i.saturating_sub(r);
            let hi = (i + r + 1).min(n);
            (cum[hi] - cum[lo]) / (hi - lo) as f64
        })
        .collect()
}

/// Where `p` first drops below `level`, walking from `start` in direction `dir` (−1 or +1),
/// interpolated between pixels. `None` if it reaches the edge first.
fn crossing(p: &[f64], start: usize, level: f64, dir: isize) -> Option<f64> {
    let mut prev = start;
    loop {
        let next = prev.checked_add_signed(dir).filter(|&i| i < p.len())?;
        if p[next] < level {
            let f = (p[prev] - level) / (p[prev] - p[next]);
            return Some(prev as f64 + dir as f64 * f);
        }
        prev = next;
    }
}

/// Centroid above the background in a window of ± `half_width` px, moved until it is centred on
/// its own centroid. Pixels cut by the window edge count by the part inside, so the result moves
/// smoothly with the beam. `None` if the window leaves the sensor or holds no light.
fn centroid(p: &[f64], bg: &Background, start: f64, half_width: f64) -> Option<f64> {
    let n = p.len() as f64;
    let mut c = start;
    for _ in 0..100 {
        let (lo, hi) = (c - half_width, c + half_width);
        if lo < -0.5 || hi > n - 0.5 {
            return None;
        }
        let i0 = (lo + 0.5).floor() as usize;
        let i1 = ((hi + 0.5).floor() as usize).min(p.len() - 1);
        let (mut sw, mut swx) = (0.0, 0.0);
        for (i, &v) in p.iter().enumerate().take(i1 + 1).skip(i0) {
            let x = i as f64;
            let w = (hi.min(x + 0.5) - lo.max(x - 0.5)).max(0.0);
            let s = (v - bg.at(x)) * w;
            sw += s;
            swx += s * x;
        }
        if sw <= 0.0 {
            return None;
        }
        let next = swx / sw;
        if (next - c).abs() < 1e-7 {
            return Some(next);
        }
        c = next;
    }
    Some(c)
}

/// Middle of the half-maximum crossings of the peak near `start`.
fn half_max_centre(sm: &[f64], bg: &Background, start: f64, fwhm: f64) -> Option<f64> {
    let n = sm.len();
    let lo = (start - 0.5 * fwhm).floor().clamp(0.0, (n - 1) as f64) as usize;
    let hi = (start + 0.5 * fwhm).ceil().clamp(0.0, (n - 1) as f64) as usize;
    let (imax, top) = (lo..=hi).map(|i| (i, sm[i])).max_by(|a, b| a.1.total_cmp(&b.1))?;
    // A window whose top is not above the background has no half maximum; without this check
    // `crossing` divides by zero on a flat run below the level.
    let floor = bg.at(imax as f64);
    if top <= floor {
        return None;
    }
    let level = 0.5 * (top + floor);
    let left = crossing(sm, imax, level, -1)?;
    let right = crossing(sm, imax, level, 1)?;
    Some(0.5 * (left + right))
}

/// A line through the strip centres, with outliers dropped.
struct LineFit {
    position: f64,
    slope: f64,
    uncertainty: f64,
    used: usize,
}

/// Strips farther than this from the line, in robust sigmas, are dropped.
const OUTLIER_SIGMAS: f64 = 6.0;

impl LineFit {
    fn of(along: &[f64], centres: &[f64], mid: f64) -> Option<Self> {
        let x: Vec<f64> = along.iter().map(|l| l - mid).collect();
        let (a, b) = stats::line_fit(&x, centres)?;
        let resid: Vec<f64> = x.iter().zip(centres).map(|(xi, c)| c - a - b * xi).collect();
        // The spread from 16 strips is rough, so the cut is wide: dust or a speck moves a strip by
        // many sigmas, while smaller pulls stay in and show in the uncertainty.
        let spread = stats::mad_sigma(&resid).unwrap_or(0.0);
        let keep: Vec<usize> = if spread > 0.0 {
            (0..x.len()).filter(|&i| resid[i].abs() <= OUTLIER_SIGMAS * spread).collect()
        } else {
            (0..x.len()).collect()
        };
        let keep = if keep.len() >= 3 { keep } else { (0..x.len()).collect() };
        if keep.len() < 3 {
            return None;
        }
        let xk: Vec<f64> = keep.iter().map(|&i| x[i]).collect();
        let ck: Vec<f64> = keep.iter().map(|&i| centres[i]).collect();
        let (a, b) = stats::line_fit(&xk, &ck)?;
        let m = xk.len() as f64;
        let rss: f64 = xk.iter().zip(&ck).map(|(xi, c)| (c - a - b * xi).powi(2)).sum();
        let mean_x = xk.iter().sum::<f64>() / m;
        let sxx: f64 = xk.iter().map(|xi| (xi - mean_x).powi(2)).sum();
        let s2 = rss / (m - 2.0);
        let uncertainty = (s2 * (1.0 / m + mean_x * mean_x / sxx)).sqrt();
        Some(Self { position: a, slope: b, uncertainty, used: keep.len() })
    }
}

/// Fraction of pixels at or above `level` in a band of ± `half_width` px around the line.
fn clipped_fraction(frame: &Luma<'_>, axis: LineAxis, centre: f64, half_width: f64, level: u8) -> f64 {
    let across_len = match axis {
        LineAxis::Vertical => frame.width(),
        LineAxis::Horizontal => frame.height(),
    };
    let lo = (centre - half_width).floor().clamp(0.0, (across_len - 1) as f64) as usize;
    let hi = (centre + half_width).ceil().clamp(0.0, (across_len - 1) as f64) as usize;
    let (mut hits, mut total) = (0usize, 0usize);
    match axis {
        LineAxis::Vertical => {
            for y in 0..frame.height() {
                let band = &frame.row(y)[lo..=hi];
                hits += band.iter().filter(|&&p| p >= level).count();
                total += band.len();
            }
        }
        LineAxis::Horizontal => {
            for y in lo..=hi {
                let row = frame.row(y);
                hits += row.iter().filter(|&&p| p >= level).count();
                total += row.len();
            }
        }
    }
    if total == 0 { 0.0 } else { hits as f64 / total as f64 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A strip whose window lies below the background has no half maximum. Before the check,
    /// two equal pixels below the level gave 0/0 and a NaN centre.
    #[test]
    fn half_max_centre_needs_a_peak_above_the_background() {
        let flat = [5.0; 9];
        assert_eq!(half_max_centre(&flat, &Background::flat(10.0), 4.0, 2.0), None);
        let peak = [0.0, 0.0, 2.0, 8.0, 10.0, 8.0, 2.0, 0.0, 0.0];
        let c = half_max_centre(&peak, &Background::flat(0.0), 4.0, 4.0).unwrap();
        assert!((c - 4.0).abs() < 1e-12, "{c}");
    }
}
