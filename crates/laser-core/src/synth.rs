//! Synthetic frames and warm-up drift with known answers, for tests and for the CLI.
//!
//! Spec v1 ([`LineSpec::v1`]) is the frame the Python baseline used: 1920x1080, a vertical
//! Gaussian line, sigma 80 px, background 16, amplitude 160, read noise 2, shot gain 0.5, seed 1.
//! Spec v2 adds what real set-ups have: tilt, an ambient gradient, clipping, and warm-up drift
//! ([`WarmUp`]) with a known time constant and steps. [`SurveySim`] makes whole surveys of a
//! surface with a known shape, and [`Surface`] is the slightly out-of-flat surface the synthetic
//! camera is measured on.

use serde::{Deserialize, Serialize};

use crate::frame::{LineAxis, LumaImage};
use crate::reading::{FrameResult, Subsample};
use crate::rng::Rng;
use crate::scale::Scale;
use crate::survey::{Run, Station, SurveyError};

/// One synthetic frame with a Gaussian laser line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineSpec {
    pub width: usize,
    pub height: usize,
    pub axis: LineAxis,
    /// Line position across the sensor at the middle of the sensor along the line, px.
    pub centre_px: f64,
    /// Change of position per px along the line.
    pub tilt: f64,
    /// Gaussian sigma across the line, px.
    pub sigma_px: f64,
    /// Peak height above the background, grey levels. Above 255 minus the background, it clips.
    pub amplitude: f64,
    pub background: f64,
    /// Background change per px across the sensor, from the first pixel: an ambient gradient.
    pub gradient: f64,
    /// Read noise, grey levels.
    pub read_noise: f64,
    /// Shot noise variance per grey level of light.
    pub shot_gain: f64,
    pub seed: u64,
}

impl LineSpec {
    /// The Python baseline frame.
    pub fn v1() -> Self {
        Self {
            width: 1920,
            height: 1080,
            axis: LineAxis::Vertical,
            centre_px: 960.0,
            tilt: 0.0,
            sigma_px: 80.0,
            amplitude: 160.0,
            background: 16.0,
            gradient: 0.0,
            read_noise: 2.0,
            shot_gain: 0.5,
            seed: 1,
        }
    }

    /// The frame with the background and noise only.
    pub fn without_line(&self) -> Self {
        Self { amplitude: 0.0, ..self.clone() }
    }

    pub fn render(&self) -> LumaImage {
        self.render_with(&mut Rng::new(self.seed))
    }

    pub fn render_with(&self, rng: &mut Rng) -> LumaImage {
        let (across_len, along_len) = match self.axis {
            LineAxis::Vertical => (self.width, self.height),
            LineAxis::Horizontal => (self.height, self.width),
        };
        let mid = (along_len as f64 - 1.0) / 2.0;
        let reach = 8.0 * self.sigma_px;
        let light = |across: usize, along: usize| -> f64 {
            let a = across as f64;
            let c = self.centre_px + self.tilt * (along as f64 - mid);
            let d = a - c;
            let line = if d.abs() < reach { self.amplitude * (-0.5 * (d / self.sigma_px).powi(2)).exp() } else { 0.0 };
            self.background + self.gradient * a + line
        };
        // With no tilt every line along the sensor is the same, so work it out once.
        let flat: Option<Vec<f64>> = (self.tilt == 0.0).then(|| (0..across_len).map(|a| light(a, 0)).collect());

        let mut img = LumaImage::new(self.width, self.height, 0);
        for y in 0..self.height {
            for x in 0..self.width {
                let (across, along) = match self.axis {
                    LineAxis::Vertical => (x, y),
                    LineAxis::Horizontal => (y, x),
                };
                let mean = match &flat {
                    Some(f) => f[across],
                    None => light(across, along),
                };
                let sd = (self.read_noise * self.read_noise + self.shot_gain * mean.max(0.0)).sqrt();
                let v = mean + sd * rng.normal();
                img.data[y * self.width + x] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
        img
    }
}

/// Reading over time while the set-up warms up: an exponential settle, steps (a bump or a
/// re-level), and white jitter from air and vibration. The unit is the caller's (px or µm).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarmUp {
    /// Reading at t = 0.
    pub start: f64,
    /// Reading it settles to.
    pub settled: f64,
    /// Time constant, s.
    pub tau_s: f64,
    /// (time s, size): a step added from that time on.
    pub steps: Vec<(f64, f64)>,
    /// Standard deviation of the jitter on each reading.
    pub jitter: f64,
}

impl WarmUp {
    /// The reading with no jitter.
    pub fn mean_at(&self, t_s: f64) -> f64 {
        let settle = self.settled + (self.start - self.settled) * (-t_s / self.tau_s).exp();
        let steps: f64 = self.steps.iter().filter(|(ts, _)| *ts <= t_s).map(|(_, size)| size).sum();
        settle + steps
    }

    pub fn sample(&self, t_s: f64, rng: &mut Rng) -> f64 {
        self.mean_at(t_s) + self.jitter * rng.normal()
    }

    /// Earliest time after which the drift over a run of `run_s` stays at most `tolerance`
    /// (steps not counted).
    pub fn true_ready_s(&self, run_s: f64, tolerance: f64) -> f64 {
        let a = (self.start - self.settled).abs() * (1.0 - (-run_s / self.tau_s).exp());
        if a <= tolerance { 0.0 } else { self.tau_s * (a / tolerance).ln() }
    }
}

/// A simulated survey: the block carried along a surface of known shape under a laser reference,
/// read while the set-up drifts. Each visit gives frames like the beam finder's.
#[derive(Clone, Debug, PartialEq)]
pub struct SurveySim {
    /// True height of the surface at each station of the run, µm.
    pub surface_um: Vec<f64>,
    /// Laser reference: height at x = y = 0, µm, and its slopes in x and y, µm per mm.
    pub reference: [f64; 3],
    pub scale: Scale,
    /// Reading at t = 0 with the block at height 0 under the reference at height 0, px.
    pub zero_px: f64,
    /// Warm-up drift of the reading, px. Its jitter is the white frame-to-frame jitter.
    pub warm_up: WarmUp,
    /// Steady drift of the reading, px per s.
    pub drift_px_per_s: f64,
    /// Standard deviation of where the block sits each time it is put down, µm.
    pub placement_um: f64,
    /// Within-frame uncertainty reported with each frame, px.
    pub frame_uncertainty_px: f64,
    pub fps: f64,
    /// Length of each visit, s.
    pub visit_s: f64,
    /// Time to move the block to the next station, s.
    pub move_s: f64,
}

impl SurveySim {
    /// A flat surface, a level reference, no drift: 2 µm per px, 0.3 px of jitter at 20 frames per
    /// second, 2 s visits 10 s apart.
    pub fn flat(stations: usize) -> Self {
        Self {
            surface_um: vec![0.0; stations],
            reference: [0.0; 3],
            scale: Scale::from_pixel_pitch(2.0),
            zero_px: 960.0,
            warm_up: WarmUp { start: 0.0, settled: 0.0, tau_s: 1.0, steps: vec![], jitter: 0.3 },
            drift_px_per_s: 0.0,
            placement_um: 0.0,
            frame_uncertainty_px: 0.05,
            fps: 20.0,
            visit_s: 2.0,
            move_s: 10.0,
        }
    }

    /// Height of station `i` against the reference, µm: what the sensor sees.
    fn seen_um(&self, run: &Run, i: usize) -> f64 {
        let s = &run.stations[i];
        let [r0, rx, ry] = self.reference;
        self.surface_um[i] - (r0 + rx * s.x_mm + ry * s.y_mm)
    }

    /// Height of station `i` above the zero station as a perfect run would measure it, µm.
    pub fn true_height_um(&self, run: &Run, i: usize) -> f64 {
        self.seen_um(run, i) - self.seen_um(run, run.zero)
    }

    /// Frames of one visit to station `i` from `t0_s`.
    pub fn visit(&self, run: &Run, i: usize, t0_s: f64, rng: &mut Rng) -> Vec<Subsample> {
        let sign = if self.scale.invert { -1.0 } else { 1.0 };
        let placed = self.seen_um(run, i) + self.placement_um * rng.normal();
        let base = self.zero_px + sign * placed / self.scale.um_per_px;
        let frames = (self.visit_s * self.fps).round().max(1.0) as usize;
        (0..frames)
            .map(|k| {
                let t = t0_s + k as f64 / self.fps;
                let px = base + self.warm_up.sample(t, rng) + self.drift_px_per_s * t;
                Subsample {
                    t_s: t,
                    frame: FrameResult::Fix {
                        px,
                        uncertainty_px: self.frame_uncertainty_px,
                        warnings: Default::default(),
                    },
                }
            })
            .collect()
    }

    /// Visit the stations in `order` from `start_s`, recording each into `run`. Returns the time
    /// after the last visit.
    pub fn survey(&self, run: &mut Run, order: &[usize], start_s: f64, rng: &mut Rng) -> Result<f64, SurveyError> {
        let mut t = start_s;
        for &i in order {
            let subs = self.visit(run, i, t, rng);
            run.record(i, subs)?;
            t += self.visit_s + self.move_s;
        }
        Ok(t)
    }
}

/// A surface a little out of flat, under a laser that is not quite square to it: what the block
/// sits on when the synthetic camera is measured with. The shape is a bow, a twist, a long wave
/// and some roughness, scaled to the run's own size, so a line or a grid of any spacing gets the
/// same kind of surface. The same station always has the same height.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Surface {
    /// Rise of the middle over the ends, µm: half a sine wave along the run, and across a grid.
    pub bow_um: f64,
    /// Rise of two opposite corners over the other two, µm. A line has none.
    pub twist_um: f64,
    /// Size of a long wave across the run, µm.
    pub wave_um: f64,
    /// Standard deviation of the roughness from station to station, µm.
    pub rough_um: f64,
    /// Slope of the laser to the surface, µm per m, in x and in y.
    pub tilt_um_per_m: [f64; 2],
    /// Standard deviation of where the block sits each time it is put down, µm.
    pub placement_um: f64,
}

impl Default for Surface {
    /// About 8 µm out of flat, under a laser 35 µm per m off square: a good surface plate.
    fn default() -> Self {
        Self {
            bow_um: 6.0,
            twist_um: 3.0,
            wave_um: 1.2,
            rough_um: 0.4,
            tilt_um_per_m: [35.0, -20.0],
            placement_um: 0.25,
        }
    }
}

impl Surface {
    /// Height of the surface at station `i` of `run`, µm.
    pub fn height_um(&self, run: &Run, i: usize) -> f64 {
        use std::f64::consts::PI;
        let s = &run.stations[i];
        // Where the station is across the run, 0 to 1, in x and in y; none when the run has no
        // size that way.
        let across = |f: fn(&Station) -> f64| {
            let (lo, hi) = run.stations.iter().map(f).fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(v), b.max(v)));
            (hi > lo).then(|| (f(s) - lo) / (hi - lo))
        };
        let (u, v) = (across(|s| s.x_mm), across(|s| s.y_mm));
        let (u0, v0) = (u.unwrap_or(0.5), v.unwrap_or(0.5));
        let bow = |w: Option<f64>| w.map_or(1.0, |w| (PI * w).sin());
        let twist = match (u, v) {
            (Some(u), Some(v)) => (2.0 * u - 1.0) * (2.0 * v - 1.0),
            _ => 0.0,
        };
        let wave = (2.0 * PI * (1.3 * u0 + 0.7 * v0) + 0.6).sin();
        // The roughness comes from the station's place, so each station keeps its own.
        let place = |mm: f64| (mm * 1000.0).round() as i64 as u64;
        let rough = Rng::new(place(s.x_mm) ^ place(s.y_mm).rotate_left(32) ^ 0x5eed).normal();
        self.bow_um * bow(u) * bow(v) + self.twist_um * twist + self.wave_um * wave + self.rough_um * rough
    }

    /// Height of station `i` against the laser, above the zero station, µm: what the camera sees
    /// when the block moves from the zero station to station `i`.
    pub fn seen_um(&self, run: &Run, i: usize) -> f64 {
        let against_laser = |i: usize| {
            let s = &run.stations[i];
            self.height_um(run, i) - (self.tilt_um_per_m[0] * s.x_mm + self.tilt_um_per_m[1] * s.y_mm) / 1000.0
        };
        against_laser(i) - against_laser(run.zero)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak_to_valley(run: &Run, surface: &Surface) -> f64 {
        let h: Vec<f64> = (0..run.stations.len()).map(|i| surface.height_um(run, i)).collect();
        h.iter().copied().fold(f64::MIN, f64::max) - h.iter().copied().fold(f64::MAX, f64::min)
    }

    #[test]
    fn surface_is_a_little_out_of_flat_on_a_line_and_a_grid() {
        let surface = Surface::default();
        let line = Run::line("line", 21, 50.0);
        let grid = Run::grid("grid", 9, 7, 50.0, 50.0);
        for run in [&line, &grid] {
            let pv = peak_to_valley(run, &surface);
            assert!((3.0..15.0).contains(&pv), "{} p-v {pv}", run.name);
            assert_eq!(surface.seen_um(run, run.zero), 0.0);
        }
        // A line has no twist: the far end of the line sees the laser's slope and the surface.
        let last = line.stations.len() - 1;
        let tilt = surface.tilt_um_per_m[0] * line.stations[last].x_mm / 1000.0;
        let seen = surface.seen_um(&line, last);
        let surface_part = surface.height_um(&line, last) - surface.height_um(&line, line.zero);
        assert!((seen - (surface_part - tilt)).abs() < 1e-9, "{seen} {surface_part} {tilt}");
    }

    #[test]
    fn a_station_keeps_its_height() {
        let surface = Surface::default();
        let grid = Run::grid("grid", 5, 4, 100.0, 100.0);
        let again = Run::grid("grid", 5, 4, 100.0, 100.0);
        for i in 0..grid.stations.len() {
            assert_eq!(surface.height_um(&grid, i), surface.height_um(&again, i));
        }
        // Neighbours differ: the roughness is not the same everywhere.
        assert_ne!(
            surface.height_um(&grid, 1) - surface.height_um(&grid, 0),
            surface.height_um(&grid, 2) - surface.height_um(&grid, 1)
        );
    }
}
