//! Results in the terms of the job: the heights with the laser reference taken out, straightness or
//! flatness, and how much to scrape or shim at each station.
//!
//! The laser reference is never square to the surface, so the least-squares line (or plane, for a
//! grid) through the heights is removed first. Unweighted, so that the zero station, which has no
//! uncertainty by definition, does not pull the fit. What is left is the flattened profile:
//! straightness or flatness is its peak to valley. Scrape (Above Min) is how far each station is
//! above the lowest; shim (Below Max) is how far it is below the highest.
//!
//! Noise alone gives a peak to valley too, and it grows with the number of stations. The results
//! give it, so a surface that is straight within the noise reads as such.

use serde::{Deserialize, Serialize};

use crate::reading::Flags;
use crate::scale::Scale;
use crate::stats;
use crate::survey::{Drift, DriftCorrection, Heights, Layout, Repeatability, Run, SurveyError};
use crate::units::Unit;

/// The laser reference relative to the surface: the fitted line or plane.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reference {
    /// `height = offset + slope · (x − x_mean)`.
    Line { offset_um: f64, slope_um_per_m: f64, x_mean_mm: f64 },
    /// `height = offset + slope_x · (x − x_mean) + slope_y · (y − y_mean)`.
    Plane { offset_um: f64, slope_x_um_per_m: f64, slope_y_um_per_m: f64, x_mean_mm: f64, y_mean_mm: f64 },
}

impl Reference {
    pub fn at(&self, x_mm: f64, y_mm: f64) -> f64 {
        match *self {
            Self::Line { offset_um, slope_um_per_m, x_mean_mm } => {
                offset_um + slope_um_per_m * (x_mm - x_mean_mm) / 1000.0
            }
            Self::Plane { offset_um, slope_x_um_per_m, slope_y_um_per_m, x_mean_mm, y_mean_mm } => {
                offset_um + (slope_x_um_per_m * (x_mm - x_mean_mm) + slope_y_um_per_m * (y_mm - y_mean_mm)) / 1000.0
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub station: usize,
    pub label: String,
    pub x_mm: f64,
    pub y_mm: f64,
    /// Height above the zero station, µm.
    pub measured_um: f64,
    /// Height above the fitted reference, µm.
    pub flattened_um: f64,
    /// Shim: how far below the highest station, µm.
    pub below_max_um: f64,
    /// Scrape: how far above the lowest station, µm.
    pub above_min_um: f64,
    pub uncertainty_um: f64,
    pub visits: usize,
    pub t_s: f64,
    pub flags: Flags,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Results {
    pub layout: Layout,
    pub points: Vec<Point>,
    pub reference: Reference,
    /// Straightness (line) or flatness (grid): peak to valley of the flattened heights, µm.
    pub pv_um: f64,
    /// Peak to valley that the noise alone would give with this many stations, µm: the station
    /// uncertainties, raised by the repeatability ratio when that is above 1.
    pub pv_noise_um: f64,
    /// Part of `pv_um` that the scale uncertainty leaves open, µm.
    pub pv_scale_um: f64,
    pub repeatability: Option<Repeatability>,
    pub drift: Drift,
}

impl Results {
    pub fn compute(run: &Run, scale: &Scale, mode: DriftCorrection) -> Result<Results, SurveyError> {
        let heights = run.heights(scale, mode)?;
        Self::from_heights(run, scale, heights)
    }

    pub fn from_heights(run: &Run, scale: &Scale, heights: Heights) -> Result<Results, SurveyError> {
        let st = &heights.stations;
        let (need, p) = match run.layout {
            Layout::Line => (3, 2),
            Layout::Grid => (4, 3),
        };
        if st.len() < need {
            return Err(SurveyError::TooFewStations { have: st.len(), need });
        }
        let n = st.len() as f64;
        let xm = st.iter().map(|s| s.x_mm).sum::<f64>() / n;
        let ym = st.iter().map(|s| s.y_mm).sum::<f64>() / n;
        let h: Vec<f64> = st.iter().map(|s| s.height_um).collect();
        let reference = match run.layout {
            Layout::Line => {
                let x: Vec<f64> = st.iter().map(|s| s.x_mm - xm).collect();
                let (a, b) = stats::line_fit(&x, &h).ok_or(SurveyError::Degenerate)?;
                Reference::Line { offset_um: a, slope_um_per_m: b * 1000.0, x_mean_mm: xm }
            }
            Layout::Grid => {
                let rows: Vec<[f64; 3]> = st.iter().map(|s| [1.0, s.x_mm - xm, s.y_mm - ym]).collect();
                let f = stats::weighted_lsq(&rows, &h, &vec![1.0; st.len()]).ok_or(SurveyError::Degenerate)?;
                Reference::Plane {
                    offset_um: f.coef[0],
                    slope_x_um_per_m: f.coef[1] * 1000.0,
                    slope_y_um_per_m: f.coef[2] * 1000.0,
                    x_mean_mm: xm,
                    y_mean_mm: ym,
                }
            }
        };
        let flat: Vec<f64> = st.iter().map(|s| s.height_um - reference.at(s.x_mm, s.y_mm)).collect();
        let hi = flat.iter().copied().fold(f64::MIN, f64::max);
        let lo = flat.iter().copied().fold(f64::MAX, f64::min);
        let points = st
            .iter()
            .zip(&flat)
            .map(|(s, &e)| {
                let label = run.stations[s.station].label.clone();
                Point {
                    station: s.station,
                    label: if label.is_empty() { (s.station + 1).to_string() } else { label },
                    x_mm: s.x_mm,
                    y_mm: s.y_mm,
                    measured_um: s.height_um,
                    flattened_um: e,
                    below_max_um: hi - e,
                    above_min_um: e - lo,
                    uncertainty_um: s.uncertainty_um,
                    visits: s.visits,
                    t_s: s.t_s,
                    flags: s.flags,
                }
            })
            .collect();

        let others: Vec<f64> = st.iter().filter(|s| s.station != run.zero).map(|s| s.uncertainty_um).collect();
        let rms = (others.iter().map(|u| u * u).sum::<f64>() / others.len().max(1) as f64).sqrt();
        let birge = heights.repeatability.map_or(1.0, |r| r.ratio.max(1.0).sqrt());
        let sigma = rms * birge * ((n - p as f64) / n).sqrt();
        let pv_um = hi - lo;
        Ok(Results {
            layout: run.layout,
            points,
            reference,
            pv_um,
            pv_noise_um: stats::expected_range(st.len()) * sigma,
            pv_scale_um: pv_um * scale.uncertainty_rel,
            repeatability: heights.repeatability,
            drift: heights.drift,
        })
    }

    /// CSV with a header row. The first four columns are the Python tool's, in its order.
    pub fn csv(&self, unit: Unit) -> String {
        let u = unit.ascii();
        let mut head: Vec<String> = ["Measured", "Flattened", "Below Max", "Above Min", "Uncertainty"]
            .iter()
            .map(|c| format!("{c} ({u})"))
            .collect();
        head.push("Station".into());
        match self.layout {
            Layout::Line => head.push("Position (mm)".into()),
            Layout::Grid => head.extend(["X (mm)".into(), "Y (mm)".into()]),
        }
        head.extend(["Visits".into(), "Time (s)".into(), "Flags".into()]);
        let mut out = head.join(",");
        out.push('\n');
        for p in &self.points {
            let mut row: Vec<String> =
                [p.measured_um, p.flattened_um, p.below_max_um, p.above_min_um, p.uncertainty_um]
                    .iter()
                    .map(|&v| unit.format(v))
                    .collect();
            row.push(csv_field(&p.label));
            row.push(format!("{:.3}", p.x_mm));
            if self.layout == Layout::Grid {
                row.push(format!("{:.3}", p.y_mm));
            }
            row.push(p.visits.to_string());
            row.push(format!("{:.2}", p.t_s));
            row.push(p.flags.names().collect::<Vec<_>>().join(";"));
            out.push_str(&row.join(","));
            out.push('\n');
        }
        out
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_owned() }
}
