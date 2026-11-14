//! From sensor pixels to micrometres, and which way is up.
//!
//! The sensor has no lens, so one pixel across the line is one pixel pitch of beam movement. The
//! pitch from the data sheet is a start. A gauge-block step checks it: the block is read on the
//! surface and on a gauge block of known height, and the change in pixels gives the scale and the
//! sign (#35, #13, #7). Alternating readings (off, on, off, ...) also remove a steady drift.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::reading::Reading;
use crate::stats;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "from", rename_all = "snake_case")]
pub enum ScaleSource {
    /// Pixel pitch from the sensor data sheet.
    #[default]
    PixelPitch,
    /// A gauge-block step of this height.
    GaugeBlock { step_um: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scale {
    /// Size of one pixel across the line, µm.
    pub um_per_px: f64,
    /// Relative standard uncertainty of `um_per_px`. 0 when it is not known.
    #[serde(default)]
    pub uncertainty_rel: f64,
    /// Heights go up when the reading in px goes down. Depends on how the sensor sits in the block.
    #[serde(default)]
    pub invert: bool,
    #[serde(default)]
    pub source: ScaleSource,
}

impl Scale {
    pub fn from_pixel_pitch(um_per_px: f64) -> Self {
        Self { um_per_px, uncertainty_rel: 0.0, invert: false, source: ScaleSource::PixelPitch }
    }

    /// From the sensor's active width and its width in pixels.
    pub fn from_sensor(width_mm: f64, width_px: usize) -> Self {
        Self::from_pixel_pitch(width_mm * 1000.0 / width_px as f64)
    }

    /// A distance in px as µm, without the sign.
    pub fn um(&self, px: f64) -> f64 {
        px * self.um_per_px
    }

    /// Height in µm of a reading `px` above a zero reading `zero_px`.
    pub fn height_um(&self, px: f64, zero_px: f64) -> f64 {
        let sign = if self.invert { -1.0 } else { 1.0 };
        sign * (px - zero_px) * self.um_per_px
    }

    pub fn is_valid(&self) -> bool {
        self.um_per_px.is_finite() && self.um_per_px > 0.0 && self.uncertainty_rel.is_finite()
    }

    /// Scale and sign from readings with the block on the surface and on a gauge block of height
    /// `step_um`. With three readings or more, a steady drift is fitted as well, so off, on, off
    /// gives a step free of linear drift.
    pub fn from_gauge_block(step_um: f64, readings: &[GaugeReading]) -> Result<Scale, CalibrationError> {
        if !(step_um.is_finite() && step_um > 0.0) {
            return Err(CalibrationError::BadStep);
        }
        let on = readings.iter().filter(|g| g.on_block).count();
        if on == 0 || on == readings.len() {
            return Err(CalibrationError::NeedBoth);
        }
        let t_mean = readings.iter().map(|g| g.reading.t_s).sum::<f64>() / readings.len() as f64;
        let y: Vec<f64> = readings.iter().map(|g| g.reading.px).collect();
        let w: Vec<f64> = readings.iter().map(|g| 1.0 / g.reading.uncertainty_px.max(1e-9).powi(2)).collect();
        let step = |coef: f64, var: f64, chi2: f64, dof: usize| {
            let birge = if dof > 0 { (chi2 / dof as f64).max(1.0) } else { 1.0 };
            (coef, (var * birge).sqrt())
        };
        let with_drift = (readings.len() >= 3)
            .then(|| {
                let rows: Vec<[f64; 3]> =
                    readings.iter().map(|g| [1.0, f64::from(u8::from(g.on_block)), g.reading.t_s - t_mean]).collect();
                stats::weighted_lsq(&rows, &y, &w).map(|f| step(f.coef[1], f.cov[1][1], f.chi2, readings.len() - 3))
            })
            .flatten();
        let (step_px, u_px) = match with_drift {
            Some(s) => s,
            None => {
                let rows: Vec<[f64; 2]> = readings.iter().map(|g| [1.0, f64::from(u8::from(g.on_block))]).collect();
                let f = stats::weighted_lsq(&rows, &y, &w).ok_or(CalibrationError::NeedBoth)?;
                step(f.coef[1], f.cov[1][1], f.chi2, readings.len() - 2)
            }
        };
        if step_px.abs() <= 5.0 * u_px {
            return Err(CalibrationError::StepNotSeen { step_px, uncertainty_px: u_px });
        }
        Ok(Scale {
            um_per_px: step_um / step_px.abs(),
            uncertainty_rel: u_px / step_px.abs(),
            invert: step_px < 0.0,
            source: ScaleSource::GaugeBlock { step_um },
        })
    }
}

/// A reading for the gauge-block calibration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GaugeReading {
    /// The block stands on the gauge block.
    pub on_block: bool,
    pub reading: Reading,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CalibrationError {
    /// The gauge-block height must be a positive number.
    BadStep,
    /// Needs readings both on and off the gauge block.
    NeedBoth,
    /// The step is lost in the noise.
    StepNotSeen { step_px: f64, uncertainty_px: f64 },
}

impl fmt::Display for CalibrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadStep => write!(f, "the gauge-block height must be a positive number"),
            Self::NeedBoth => write!(f, "take readings both on and off the gauge block"),
            Self::StepNotSeen { step_px, uncertainty_px } => write!(
                f,
                "the step ({step_px:.3} ± {uncertainty_px:.3} px) is lost in the noise; use a taller gauge block or longer samples"
            ),
        }
    }
}

impl std::error::Error for CalibrationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading::Flags;

    fn reading(t_s: f64, px: f64, uncertainty_px: f64) -> Reading {
        Reading {
            t_s,
            duration_s: 2.0,
            px,
            uncertainty_px,
            scatter_px: 0.3,
            frames: 40,
            fixes: 40,
            used: 40,
            flags: Flags::default(),
        }
    }

    /// 100 µm block at 2.2 µm/px with a sensor that reads down for up, and a drift of 0.01 px/s.
    #[test]
    fn gauge_block_with_drift() {
        let px = |t: f64, on: bool| 500.0 - if on { 100.0 / 2.2 } else { 0.0 } + 0.01 * t;
        let g = |t: f64, on: bool| GaugeReading { on_block: on, reading: reading(t, px(t, on), 0.02) };
        let s = Scale::from_gauge_block(100.0, &[g(0.0, false), g(30.0, true), g(60.0, false)]).unwrap();
        assert!((s.um_per_px - 2.2).abs() < 1e-9, "{s:?}");
        assert!(s.invert && s.uncertainty_rel > 0.0 && s.uncertainty_rel < 0.002, "{s:?}");
        assert!((s.height_um(px(0.0, true), px(0.0, false)) - 100.0).abs() < 1e-9);

        // Two readings cannot see the drift: 0.3 px of drift in 30 s.
        let two = Scale::from_gauge_block(100.0, &[g(0.0, false), g(30.0, true)]).unwrap();
        assert!((two.um_per_px - 2.2).abs() > 0.01, "{two:?}");
    }

    #[test]
    fn gauge_block_errors() {
        let off = GaugeReading { on_block: false, reading: reading(0.0, 10.0, 0.1) };
        let on = GaugeReading { on_block: true, reading: reading(5.0, 10.2, 0.1) };
        assert_eq!(Scale::from_gauge_block(-1.0, &[off.clone(), on.clone()]), Err(CalibrationError::BadStep));
        assert_eq!(Scale::from_gauge_block(10.0, &[off.clone(), off.clone()]), Err(CalibrationError::NeedBoth));
        assert!(matches!(Scale::from_gauge_block(10.0, &[off, on]), Err(CalibrationError::StepNotSeen { .. })));
    }
}
