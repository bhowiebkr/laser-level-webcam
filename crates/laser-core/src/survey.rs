//! The measurement: stations along a line or on a grid, readings at each, all relative to a zero
//! station.
//!
//! The block is carried from station to station under a fixed laser reference. Each visit is a
//! [`Reading`]. Heights are relative to the zero station, so a drift of the laser or the camera adds
//! straight into every height. Visits to the zero station measure that drift: with
//! [`DriftCorrection::Interpolate`], the zero for each reading is interpolated in time between the
//! zero visits around it, which removes a steady drift. A reading outside the span of the zero
//! visits is flagged, because nothing checks its drift. Stations visited more than once (repeat
//! runs, forward and back) show the repeatability.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::beam::FinderConfig;
use crate::camera::{CameraSettings, Sensor};
use crate::frame::{LineAxis, Orientation};
use crate::reading::{Flags, Reading, SampleRule, Subsample};
use crate::scale::{Scale, ScaleSource};
use crate::units::Unit;

/// Version of the session file. A file from a newer version is refused.
pub const FORMAT_VERSION: u32 = 1;

/// Everything about one measurement, saved and opened as a file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub format: u32,
    pub scale: Scale,
    /// Unix time, s, that the times of the session's readings count from, so they stay small
    /// numbers. 0 in files that predate it.
    #[serde(default)]
    pub started_unix: f64,
    #[serde(default)]
    pub unit: Unit,
    #[serde(default)]
    pub sensor: Sensor,
    #[serde(default)]
    pub camera: CameraSettings,
    #[serde(default)]
    pub finder: FinderConfig,
    /// How the camera's image was turned to show the world. Files that predate it have the
    /// default; [`Session::orientation`] works it out from the finder and the scale then.
    #[serde(default, rename = "orientation")]
    pub view: Option<Orientation>,
    #[serde(default)]
    pub sample: SampleRule,
    #[serde(default)]
    pub drift_correction: DriftCorrection,
    #[serde(default)]
    pub runs: Vec<Run>,
    #[serde(default)]
    pub notes: String,
}

impl Session {
    /// How the camera's image was turned: as saved, or, for files from before that, the one
    /// that matches the finder's line axis and the scale's sign.
    pub fn orientation(&self) -> Orientation {
        self.view.unwrap_or_else(|| Orientation::from_axis(self.finder.axis, self.scale.invert))
    }

    pub fn new(scale: Scale) -> Self {
        Self {
            format: FORMAT_VERSION,
            scale,
            started_unix: 0.0,
            unit: Unit::default(),
            sensor: Sensor::default(),
            camera: CameraSettings::default(),
            finder: FinderConfig::default(),
            view: None,
            sample: SampleRule::default(),
            drift_correction: DriftCorrection::default(),
            runs: Vec::new(),
            notes: String::new(),
        }
    }

    /// Check a session read from a file before it is used.
    pub fn check(&self) -> Result<(), SurveyError> {
        if self.format > FORMAT_VERSION {
            return Err(SurveyError::Format(self.format));
        }
        if !self.scale.is_valid() {
            return Err(SurveyError::BadScale);
        }
        if !self.sensor.is_valid() {
            return Err(SurveyError::BadSensor);
        }
        self.runs.iter().try_for_each(Run::check)
    }

    /// The text of a session file.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a session always serialises")
    }

    /// The same text without line breaks or indents, for the copy the app keeps between runs.
    /// Every frame of every visit is in it, and the browser keeps at most a few megabytes for a
    /// page, so the indents matter.
    pub fn to_json_compact(&self) -> String {
        serde_json::to_string(self).expect("a session always serialises")
    }

    /// Read and check a session file.
    pub fn from_json(text: &str) -> Result<Session, LoadError> {
        // The version first, so a file from a newer version says so instead of failing on a field
        // that version added.
        #[derive(Deserialize)]
        struct Version {
            format: u32,
        }
        let json = |e: serde_json::Error| LoadError::Json(e.to_string());
        let v: Version = serde_json::from_str(text).map_err(json)?;
        if v.format > FORMAT_VERSION {
            return Err(LoadError::Invalid(SurveyError::Format(v.format)));
        }
        let session: Session = serde_json::from_str(text).map_err(json)?;
        session.check().map_err(LoadError::Invalid)?;
        Ok(session)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// Stations along a line; only `x_mm` is used. Straightness after the least-squares line.
    #[default]
    Line,
    /// Stations on a surface. Flatness after the least-squares plane.
    Grid,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftCorrection {
    /// Each reading against the last zero visit before it. Re-zeroing starts again from there.
    LastZero,
    /// The zero is interpolated in time between the zero visits around each reading.
    #[default]
    Interpolate,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Station {
    /// Position along the line, or across the surface, mm.
    pub x_mm: f64,
    #[serde(default)]
    pub y_mm: f64,
    /// Name shown in tables. Empty: the station number.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

impl Station {
    pub fn at(x_mm: f64) -> Self {
        Self { x_mm, ..Self::default() }
    }

    pub fn at_xy(x_mm: f64, y_mm: f64) -> Self {
        Self { x_mm, y_mm, ..Self::default() }
    }
}

/// One visit of the block to a station.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Visit {
    pub station: usize,
    pub reading: Reading,
    /// The frames behind the reading, so it can be checked or worked out again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subsamples: Vec<Subsample>,
}

/// The pixels a run's readings are on: the frame size and the way the line runs on the sensor.
/// A reading in px means the same height only on the same grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PixelGrid {
    pub width: u32,
    pub height: u32,
    pub axis: LineAxis,
}

impl fmt::Display for PixelGrid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let axis = match self.axis {
            LineAxis::Vertical => "the line top to bottom on the sensor",
            LineAxis::Horizontal => "the line left to right on the sensor",
        };
        write!(f, "{}×{} with {axis}", self.width, self.height)
    }
}

/// One line or one grid, measured against one zero station.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Run {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub layout: Layout,
    pub stations: Vec<Station>,
    /// Index of the zero station.
    #[serde(default)]
    pub zero: usize,
    /// In the order they were taken.
    #[serde(default)]
    pub visits: Vec<Visit>,
    /// The pixel grid the visits were read on. None in files that predate it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<PixelGrid>,
}

impl Run {
    /// `count` stations `spacing_mm` apart from 0, the first one the zero.
    pub fn line(name: &str, count: usize, spacing_mm: f64) -> Self {
        let stations = (0..count).map(|i| Station::at(i as f64 * spacing_mm)).collect();
        Self { name: name.to_owned(), layout: Layout::Line, stations, zero: 0, visits: Vec::new(), grid: None }
    }

    /// `nx` by `ny` stations, row by row, the first one the zero.
    pub fn grid(name: &str, nx: usize, ny: usize, dx_mm: f64, dy_mm: f64) -> Self {
        let stations =
            (0..ny).flat_map(|j| (0..nx).map(move |i| Station::at_xy(i as f64 * dx_mm, j as f64 * dy_mm))).collect();
        Self { name: name.to_owned(), layout: Layout::Grid, stations, zero: 0, visits: Vec::new(), grid: None }
    }

    /// The run's grid when frames on `grid` would not fit its visits: another frame size or
    /// line axis would mix pixel sizes and directions in one run.
    pub fn grid_conflict(&self, grid: PixelGrid) -> Option<PixelGrid> {
        self.grid.filter(|g| *g != grid && !self.visits.is_empty())
    }

    /// Take readings on `grid` from now on. Refused while the run has visits on another grid.
    pub fn use_grid(&mut self, grid: PixelGrid) -> Result<(), SurveyError> {
        if let Some(run) = self.grid_conflict(grid) {
            return Err(SurveyError::OtherGrid { run, now: grid });
        }
        self.grid = Some(grid);
        Ok(())
    }

    /// The scale for this run's heights. A scale from the pixel pitch depends on the frame size,
    /// so it comes from the grid of the run's visits when it has some; a gauge-block scale is kept.
    pub fn scale(&self, scale: &Scale, sensor: &Sensor) -> Scale {
        match (scale.source, self.grid) {
            (ScaleSource::PixelPitch, Some(g)) if sensor.is_valid() && !self.visits.is_empty() => {
                Scale { um_per_px: sensor.um_per_px(g.width, g.height), ..*scale }
            }
            _ => *scale,
        }
    }

    fn check(&self) -> Result<(), SurveyError> {
        if !self.stations.is_empty() && self.zero >= self.stations.len() {
            return Err(SurveyError::NoStation(self.zero));
        }
        match self.visits.iter().find(|v| v.station >= self.stations.len()) {
            Some(v) => Err(SurveyError::NoStation(v.station)),
            None => Ok(()),
        }
    }

    fn station_ok(&self, i: usize) -> Result<(), SurveyError> {
        if i < self.stations.len() { Ok(()) } else { Err(SurveyError::NoStation(i)) }
    }

    fn visit_ok(&self, i: usize) -> Result<(), SurveyError> {
        if i < self.visits.len() { Ok(()) } else { Err(SurveyError::NoVisit(i)) }
    }

    pub fn add_station(&mut self, station: Station) -> usize {
        self.stations.push(station);
        self.stations.len() - 1
    }

    /// Remove a station and its visits. The zero station cannot be removed; choose another first.
    pub fn remove_station(&mut self, i: usize) -> Result<Vec<Visit>, SurveyError> {
        self.station_ok(i)?;
        if i == self.zero {
            return Err(SurveyError::ZeroStation);
        }
        // Unnamed stations show their number; keep the numbers the user has seen.
        for (k, s) in self.stations.iter_mut().enumerate() {
            if s.label.is_empty() {
                s.label = (k + 1).to_string();
            }
        }
        self.stations.remove(i);
        if self.zero > i {
            self.zero -= 1;
        }
        let (gone, kept): (Vec<Visit>, Vec<Visit>) = self.visits.drain(..).partition(|v| v.station == i);
        self.visits = kept
            .into_iter()
            .map(|mut v| {
                if v.station > i {
                    v.station -= 1;
                }
                v
            })
            .collect();
        Ok(gone)
    }

    pub fn set_zero(&mut self, i: usize) -> Result<(), SurveyError> {
        self.station_ok(i)?;
        self.zero = i;
        Ok(())
    }

    /// Add a visit from the frames of one sample. Returns the visit index.
    pub fn record(&mut self, station: usize, subsamples: Vec<Subsample>) -> Result<usize, SurveyError> {
        self.station_ok(station)?;
        let reading = Reading::from_subsamples(&subsamples).ok_or(SurveyError::NoReading)?;
        self.visits.push(Visit { station, reading, subsamples });
        Ok(self.visits.len() - 1)
    }

    /// Take a visit again: same station, new frames. Nothing changes when it fails (bug 8).
    pub fn replace(&mut self, visit: usize, subsamples: Vec<Subsample>) -> Result<(), SurveyError> {
        self.visit_ok(visit)?;
        let reading = Reading::from_subsamples(&subsamples).ok_or(SurveyError::NoReading)?;
        let v = &mut self.visits[visit];
        v.reading = reading;
        v.subsamples = subsamples;
        Ok(())
    }

    /// Delete a visit. Nothing changes when it fails (bug 8).
    pub fn delete(&mut self, visit: usize) -> Result<Visit, SurveyError> {
        self.visit_ok(visit)?;
        Ok(self.visits.remove(visit))
    }

    /// Station to measure next: the zero first, then each station in order, then the zero again to
    /// close the run.
    pub fn next_station(&self) -> usize {
        let visited = |s: usize| self.visits.iter().any(|v| v.station == s);
        if !visited(self.zero) {
            return self.zero;
        }
        (0..self.stations.len()).find(|&s| !visited(s)).unwrap_or(self.zero)
    }

    /// Zero visits as (time, px, uncertainty px), in time order.
    fn zeros(&self) -> Vec<(f64, f64, f64)> {
        let mut z: Vec<(f64, f64, f64)> = self
            .visits
            .iter()
            .filter(|v| v.station == self.zero)
            .map(|v| (v.reading.t_s, v.reading.px, v.reading.uncertainty_px))
            .collect();
        z.sort_by(|a, b| a.0.total_cmp(&b.0));
        z
    }

    /// Heights of every visited station above the zero station.
    pub fn heights(&self, scale: &Scale, mode: DriftCorrection) -> Result<Heights, SurveyError> {
        self.check()?;
        let zeros = self.zeros();
        let (Some(first), Some(last)) = (zeros.first().copied(), zeros.last().copied()) else {
            return Err(SurveyError::NoZero);
        };
        let k = scale.um_per_px;

        let visits: Vec<VisitHeight> = self
            .visits
            .iter()
            .enumerate()
            .filter(|(_, v)| v.station != self.zero)
            .map(|(i, v)| {
                let r = &v.reading;
                let (zpx, zu) = zero_at(&zeros, r.t_s, mode);
                let unchecked = zeros.len() < 2 || r.t_s < first.0 || r.t_s > last.0;
                VisitHeight {
                    visit: i,
                    station: v.station,
                    t_s: r.t_s,
                    height_um: scale.height_um(r.px, zpx),
                    uncertainty_um: k * r.uncertainty_px.hypot(zu),
                    flags: Flags { drift_unchecked: unchecked, ..r.flags },
                }
            })
            .collect();

        let mut stations = Vec::new();
        let (mut chi2, mut ss, mut dof) = (0.0, 0.0, 0usize);
        for (s, st) in self.stations.iter().enumerate() {
            if s == self.zero {
                let flags = self
                    .visits
                    .iter()
                    .filter(|v| v.station == s)
                    .fold(Flags::default(), |f, v| f.union(v.reading.flags));
                stations.push(StationHeight {
                    station: s,
                    x_mm: st.x_mm,
                    y_mm: st.y_mm,
                    height_um: 0.0,
                    uncertainty_um: 0.0,
                    visits: zeros.len(),
                    t_s: last.0,
                    flags,
                });
                continue;
            }
            let vs: Vec<&VisitHeight> = visits.iter().filter(|v| v.station == s).collect();
            if vs.is_empty() {
                continue;
            }
            let w: Vec<f64> = vs.iter().map(|v| 1.0 / v.uncertainty_um.max(1e-9).powi(2)).collect();
            let sw: f64 = w.iter().sum();
            let mean = vs.iter().zip(&w).map(|(v, w)| w * v.height_um).sum::<f64>() / sw;
            for (v, w) in vs.iter().zip(&w) {
                let d = v.height_um - mean;
                chi2 += w * d * d;
                ss += d * d;
            }
            dof += vs.len() - 1;
            stations.push(StationHeight {
                station: s,
                x_mm: st.x_mm,
                y_mm: st.y_mm,
                height_um: mean,
                uncertainty_um: sw.sqrt().recip(),
                visits: vs.len(),
                t_s: vs.iter().map(|v| v.t_s).fold(f64::MIN, f64::max),
                flags: vs.iter().fold(Flags::default(), |f, v| f.union(v.flags)),
            });
        }
        let repeatability =
            (dof > 0).then(|| Repeatability { pooled_sd_um: (ss / dof as f64).sqrt(), dof, ratio: chi2 / dof as f64 });

        let zero_visits: Vec<ZeroVisit> = zeros
            .iter()
            .map(|&(t, px, u)| ZeroVisit {
                t_s: t,
                height_um: scale.height_um(px, first.1),
                uncertainty_um: if t == first.0 { 0.0 } else { k * u.hypot(first.2) },
            })
            .collect();
        let span = last.0 - first.0;
        let closure_um = (zeros.len() >= 2).then(|| scale.height_um(last.1, first.1));
        let drift = Drift {
            rate_um_per_min: closure_um.filter(|_| span > 0.0).map(|c| c / span * 60.0),
            closure_um,
            closed: zeros.len() >= 2 && visits.iter().all(|v| !v.flags.drift_unchecked),
            zeros: zero_visits,
        };
        Ok(Heights { stations, visits, repeatability, drift })
    }

    /// Whether to measure the zero station again now: the drift predicted since the last zero visit
    /// against `target_um`. The rate is the run's own, from its last two zero visits; with only one
    /// zero visit, `fallback_um_per_s` (the drift monitor's rate). `None` with no zero visit or no
    /// rate.
    pub fn rezero(&self, scale: &Scale, now_s: f64, fallback_um_per_s: Option<f64>, target_um: f64) -> Option<Rezero> {
        let zeros = self.zeros();
        let last = *zeros.last()?;
        let from_run = zeros.len() >= 2 && last.0 > zeros[zeros.len() - 2].0;
        let rate = if from_run {
            let prev = zeros[zeros.len() - 2];
            scale.height_um(last.1, prev.1).abs() / (last.0 - prev.0)
        } else {
            fallback_um_per_s?.abs()
        };
        let since_s = (now_s - last.0).max(0.0);
        let predicted_um = rate * since_s;
        Some(Rezero { since_s, rate_um_per_min: rate * 60.0, predicted_um, due: predicted_um >= target_um, from_run })
    }
}

/// Zero reading and its uncertainty at time `t`.
fn zero_at(zeros: &[(f64, f64, f64)], t: f64, mode: DriftCorrection) -> (f64, f64) {
    let after = zeros.partition_point(|z| z.0 <= t);
    match mode {
        DriftCorrection::LastZero => {
            let z = zeros[after.saturating_sub(1)];
            (z.1, z.2)
        }
        DriftCorrection::Interpolate => {
            if after == 0 {
                (zeros[0].1, zeros[0].2)
            } else if after == zeros.len() {
                let z = zeros[after - 1];
                (z.1, z.2)
            } else {
                let (a, b) = (zeros[after - 1], zeros[after]);
                let w = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 0.5 };
                ((1.0 - w) * a.1 + w * b.1, ((1.0 - w) * a.2).hypot(w * b.2))
            }
        }
    }
}

/// Height of one visit above the zero station.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VisitHeight {
    pub visit: usize,
    pub station: usize,
    pub t_s: f64,
    pub height_um: f64,
    pub uncertainty_um: f64,
    pub flags: Flags,
}

/// Height of one station above the zero station: the weighted mean of its visits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationHeight {
    pub station: usize,
    pub x_mm: f64,
    pub y_mm: f64,
    pub height_um: f64,
    /// From the reading uncertainties. Block placement is not in it; [`Repeatability`] shows it.
    pub uncertainty_um: f64,
    pub visits: usize,
    /// Time of the last visit, s.
    pub t_s: f64,
    pub flags: Flags,
}

/// How well repeat visits to the same station agree.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Repeatability {
    /// Pooled standard deviation of one visit about its station's mean, µm.
    pub pooled_sd_um: f64,
    /// Degrees of freedom: visits minus stations, over the stations visited more than once.
    pub dof: usize,
    /// Scatter of the repeats over what the reading uncertainties predict (chi-squared per degree of
    /// freedom). Near 1: the uncertainties are honest. Well above 1: something the readings do not
    /// see, such as where the block sits or dirt under it, adds to every height.
    pub ratio: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZeroVisit {
    pub t_s: f64,
    /// Zero reading against the first zero visit, µm.
    pub height_um: f64,
    pub uncertainty_um: f64,
}

/// The drift the zero visits show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drift {
    pub zeros: Vec<ZeroVisit>,
    /// Last zero visit against the first, µm. `None` with one zero visit.
    pub closure_um: Option<f64>,
    pub rate_um_per_min: Option<f64>,
    /// Every reading lies between two zero visits, so its drift is bounded.
    pub closed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Heights {
    /// Visited stations, the zero station at height 0, in station order.
    pub stations: Vec<StationHeight>,
    /// Every visit outside the zero station, in the order taken.
    pub visits: Vec<VisitHeight>,
    pub repeatability: Option<Repeatability>,
    pub drift: Drift,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rezero {
    pub since_s: f64,
    pub rate_um_per_min: f64,
    /// Drift predicted since the last zero visit, µm.
    pub predicted_um: f64,
    pub due: bool,
    /// The rate is the run's own, not the monitor's.
    pub from_run: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurveyError {
    NoStation(usize),
    NoVisit(usize),
    /// No frame of the sample had a beam.
    NoReading,
    /// The zero station has no reading yet.
    NoZero,
    /// The zero station cannot be removed.
    ZeroStation,
    TooFewStations {
        have: usize,
        need: usize,
    },
    /// The stations do not span a line (all at one position) or a plane (all on one line).
    Degenerate,
    BadScale,
    /// The sensor has no size or no pixel pitch.
    BadSensor,
    /// The file is from a newer version.
    Format(u32),
    /// The run's visits are on another pixel grid than the frames now.
    OtherGrid {
        run: PixelGrid,
        now: PixelGrid,
    },
}

impl fmt::Display for SurveyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoStation(i) => write!(f, "there is no station {}", i + 1),
            Self::NoVisit(i) => write!(f, "there is no reading {}; select a reading first", i + 1),
            Self::NoReading => write!(f, "no frame of the sample had a beam"),
            Self::NoZero => write!(f, "measure the zero station first"),
            Self::ZeroStation => write!(f, "the zero station cannot be removed; choose another zero first"),
            Self::TooFewStations { have, need } => write!(f, "{have} stations measured, {need} needed"),
            Self::Degenerate => write!(f, "the stations do not span a line or a plane"),
            Self::BadScale => write!(f, "the scale must be a positive number"),
            Self::BadSensor => write!(f, "the sensor needs a positive pixel pitch and size"),
            Self::Format(v) => write!(f, "the session file is version {v}, newer than this program ({FORMAT_VERSION})"),
            Self::OtherGrid { run, now } => write!(
                f,
                "this run was read at {run}, and the camera now gives {now}; set the camera mode and orientation back, or start a new run"
            ),
        }
    }
}

impl std::error::Error for SurveyError {}

/// Why a session file could not be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// Not a session file: the JSON parser's message.
    Json(String),
    /// A session file whose contents do not hold together.
    Invalid(SurveyError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "not a session file: {e}"),
            Self::Invalid(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for LoadError {}
