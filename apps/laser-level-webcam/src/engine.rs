//! What the app knows, apart from how it is drawn. The camera driver feeds it frames, natively on
//! its own thread so that drift tracking and samples carry on while the window is hidden; the
//! window reads it.
//!
//! Times are session seconds: from [`Session::started_unix`], so a session's readings keep one
//! clock when the camera is reopened or the app is restarted.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

use laser_camera::synthetic::Block;
use laser_camera::{Applied, CameraError, DeviceInfo, Frame};
use laser_core::beam::{self, FinderConfig, Fix, Inspection, NoBeam};
use laser_core::camera::{ControlInfo, Sensor, VideoMode};
use laser_core::frame::Luma;
use laser_core::monitor::{Monitor, MonitorConfig, RunPlan, Status};
use laser_core::reading::{Reading, SampleRule, Sampler};
use laser_core::rng::Rng;
use laser_core::scale::{Scale, ScaleSource};
use laser_core::survey::{Run, Session};
use laser_core::synth::Surface;

/// Longest side of the preview image, px.
const PREVIEW_PX: usize = 1280;
/// How long the live position trace keeps, s.
pub const TRACE_S: f64 = 30.0;
/// How often the drift status is worked out again, s.
const STATUS_EVERY_S: f64 = 1.0;
/// Time for the frames from before the simulated block moved to pass, s.
const BLOCK_SETTLE_S: f64 = 0.3;

/// Unix time now, s.
pub fn unix_now() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() / 1000.0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
    }
}

pub fn lock(engine: &Mutex<Engine>) -> MutexGuard<'_, Engine> {
    engine.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One frame, worked on outside the engine's lock.
pub struct Processed {
    /// By the camera's clock, s.
    pub t_s: f64,
    pub seq: u64,
    pub width: usize,
    pub height: usize,
    pub fix: Result<Fix, NoBeam>,
    pub detail: Option<Detail>,
}

/// What the window shows of a frame, made for a few frames a second.
pub struct Detail {
    pub preview: Preview,
    pub inspection: Inspection,
    /// Pixels at each grey level.
    pub histogram: Box<[u32; 256]>,
    pub width: usize,
    pub height: usize,
}

impl Detail {
    pub fn peak(&self) -> u8 {
        self.histogram.iter().rposition(|&n| n > 0).unwrap_or(0) as u8
    }

    /// Share of the pixels at or above `level`.
    pub fn share_at_or_above(&self, level: u8) -> f64 {
        let total: u64 = self.histogram.iter().map(|&n| u64::from(n)).sum();
        let above: u64 = self.histogram[usize::from(level)..].iter().map(|&n| u64::from(n)).sum();
        above as f64 / total.max(1) as f64
    }
}

/// The frame shrunk by a whole factor, each pixel the mean of the block it covers.
pub struct Preview {
    pub width: usize,
    pub height: usize,
    /// Frame px per preview px.
    pub factor: usize,
    pub data: Vec<u8>,
}

impl Preview {
    fn of(frame: &Frame) -> Preview {
        let factor = frame.width.max(frame.height).div_ceil(PREVIEW_PX).max(1);
        let (w, h) = (frame.width / factor, frame.height / factor);
        if factor == 1 {
            return Preview { width: w, height: h, factor, data: frame.data.clone() };
        }
        let area = (factor * factor) as u32;
        let mut data = Vec::with_capacity(w * h);
        let mut acc = vec![0u32; w];
        for y in 0..h {
            acc.fill(0);
            for dy in 0..factor {
                let row = &frame.data[(y * factor + dy) * frame.width..][..w * factor];
                for (a, block) in acc.iter_mut().zip(row.chunks_exact(factor)) {
                    *a += block.iter().map(|&p| u32::from(p)).sum::<u32>();
                }
            }
            data.extend(acc.iter().map(|&a| (a / area) as u8));
        }
        Preview { width: w, height: h, factor, data }
    }
}

/// Find the beam in a frame; with `detail`, also what the window shows of it.
pub fn process(frame: &Frame, cfg: &FinderConfig, detail: bool) -> Processed {
    let (fix, detail) = match frame.luma() {
        Ok(luma) => {
            let fix = beam::find(&luma, cfg);
            (fix, detail.then(|| details(frame, &luma, cfg)))
        }
        // A frame always fits its own size; this is a backend bug, shown as no beam.
        Err(_) => (Err(NoBeam::Flat), None),
    };
    Processed { t_s: frame.t_s, seq: frame.seq, width: frame.width, height: frame.height, fix, detail }
}

fn details(frame: &Frame, luma: &Luma<'_>, cfg: &FinderConfig) -> Detail {
    let mut histogram = Box::new([0u32; 256]);
    for &p in &frame.data {
        histogram[usize::from(p)] += 1;
    }
    Detail {
        preview: Preview::of(frame),
        inspection: beam::inspect(luma, cfg),
        histogram,
        width: frame.width,
        height: frame.height,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum CameraState {
    #[default]
    Closed,
    Opening(String),
    Open,
    Failed(String),
}

/// The camera as the driver last heard of it.
#[derive(Default)]
pub struct CameraView {
    pub state: CameraState,
    pub device: Option<DeviceInfo>,
    pub modes: Vec<VideoMode>,
    pub mode: Option<VideoMode>,
    pub controls: Vec<ControlInfo>,
    /// Settings the camera did not keep, newest last.
    pub refused: Vec<Applied>,
    /// What the camera itself says it sends, from its USB descriptors.
    pub usb: Option<laser_core::uvc::UvcDevice>,
}

impl CameraView {
    pub fn control(&self, id: laser_core::camera::ControlId) -> Option<&ControlInfo> {
        self.controls.iter().find(|c| c.id == id)
    }

    fn note(&mut self, applied: Applied) {
        if let Ok(info) = &applied.result
            && let Some(c) = self.controls.iter_mut().find(|c| c.id == info.id)
        {
            *c = *info;
        }
        self.refused.retain(|a| a.setting.id != applied.setting.id);
        if !applied.held() {
            self.refused.push(applied);
        }
    }
}

/// The newest frame and the frame rate.
#[derive(Default)]
pub struct Live {
    pub fix: Option<Result<Fix, NoBeam>>,
    pub size: Option<(usize, usize)>,
    pub detail: Option<Detail>,
    /// Goes up with each new detail, so the window knows to upload the preview.
    pub detail_version: u64,
    pub fps: f64,
    pub frames: u64,
    /// Frame times with no frame.
    pub gaps: u64,
    /// Frames dropped because the app was behind.
    pub dropped: u64,
    last: Option<(f64, u64)>,
}

impl Live {
    fn count(&mut self, p: &Processed) {
        self.frames += 1;
        if let Some((t, seq)) = self.last
            && p.seq > seq
        {
            self.gaps += p.seq - seq - 1;
            let dt = p.t_s - t;
            if dt > 0.0 {
                let fps = 1.0 / dt;
                self.fps = if self.fps > 0.0 { 0.9 * self.fps + 0.1 * fps } else { fps };
            }
        }
        self.last = Some((p.t_s, p.seq));
    }
}

/// A sample being taken at a station.
pub struct Sampling {
    pub sampler: Sampler,
    pub station: usize,
    /// The visit it replaces.
    pub replace: Option<usize>,
    pub started_s: f64,
}

/// What the synthetic camera measures: a surface a little out of flat, and the block on it. The
/// block goes to each station as it is sampled, and the line moves by that station's height.
struct Bench {
    block: Block,
    surface: Surface,
    /// Where the block lands each time it is put down.
    rng: Rng,
    /// (run, station) the block is on.
    at: Option<(usize, usize)>,
    /// Session time the block last moved, s.
    moved_s: f64,
}

impl Bench {
    fn new(block: Block) -> Self {
        block.place(0.0);
        Self { block, surface: Surface::default(), rng: Rng::new(0xb10c), at: None, moved_s: f64::NEG_INFINITY }
    }
}

pub struct Engine {
    pub finder: FinderConfig,
    pub sensor: Sensor,
    pub plan: RunPlan,
    pub sample_rule: SampleRule,
    pub invert: bool,
    pub camera: CameraView,
    pub live: Live,
    pub monitor: Monitor,
    /// Recent positions: session s, µm.
    pub trace: VecDeque<[f64; 2]>,
    pub status: Option<Status>,
    status_t: f64,
    pub session: Session,
    /// The run being measured.
    pub run: usize,
    pub sampling: Option<Sampling>,
    /// The last thing that went wrong with the survey, for the window to show.
    pub survey_error: Option<String>,
    /// Session time of the camera's first frame.
    base_s: f64,
    /// The synthetic camera is open.
    bench: Option<Bench>,
}

impl Engine {
    pub fn new(session: Session) -> Self {
        let session = prepared(session);
        let run = session.runs.len() - 1;
        Self {
            finder: session.finder.clone(),
            sensor: session.sensor.clone(),
            plan: RunPlan { run_s: 600.0, tolerance: 1.0 },
            sample_rule: session.sample,
            invert: session.scale.invert,
            camera: CameraView::default(),
            live: Live::default(),
            monitor: Monitor::new(MonitorConfig::default()),
            trace: VecDeque::new(),
            status: None,
            status_t: f64::NEG_INFINITY,
            session,
            run,
            sampling: None,
            survey_error: None,
            base_s: 0.0,
            bench: None,
        }
    }

    /// Session time now, s.
    pub fn now(&self) -> f64 {
        unix_now() - self.session.started_unix
    }

    /// Session time of the newest frame.
    pub fn frame_time(&self) -> Option<f64> {
        self.live.last.map(|(t, _)| self.base_s + t)
    }

    /// µm per px across the line in the current mode.
    pub fn um_per_px(&self) -> f64 {
        let (w, h) = self
            .live
            .size
            .or(self.camera.mode.map(|m| (m.width as usize, m.height as usize)))
            .unwrap_or((self.sensor.width_px as usize, self.sensor.height_px as usize));
        self.sensor.um_per_px(w as u32, h as u32, self.finder.axis)
    }

    /// The scale the session's heights use: the sensor's, unless a gauge block set it.
    pub fn sync_scale(&mut self) {
        if matches!(self.session.scale.source, ScaleSource::PixelPitch) {
            self.session.scale = Scale { invert: self.invert, ..Scale::from_pixel_pitch(self.um_per_px()) };
        } else {
            self.session.scale.invert = self.invert;
        }
    }

    pub fn opening(&mut self, name: Option<&str>) {
        self.camera.state = CameraState::Opening(name.unwrap_or("the first camera").to_owned());
        self.bench = None;
    }

    pub fn opened(
        &mut self,
        device: DeviceInfo,
        modes: Vec<VideoMode>,
        mode: VideoMode,
        controls: Vec<ControlInfo>,
        applied: Vec<Applied>,
    ) {
        let size_changed = self.camera.mode.is_some_and(|m| (m.width, m.height) != (mode.width, mode.height));
        self.bench = None;
        self.camera = CameraView {
            state: CameraState::Open,
            device: Some(device),
            modes,
            mode: Some(mode),
            controls,
            refused: Vec::new(),
            usb: None,
        };
        for a in applied {
            self.camera.note(a);
        }
        self.base_s = self.now();
        self.live = Live::default();
        self.trace.clear();
        if size_changed {
            self.restart_monitor();
        }
        self.sync_scale();
    }

    pub fn controls(&mut self, controls: Vec<ControlInfo>) {
        self.camera.controls = controls;
    }

    pub fn control(&mut self, applied: Applied) {
        self.camera.note(applied);
    }

    pub fn failed(&mut self, error: &CameraError) {
        self.camera.state = CameraState::Failed(error.to_string());
        self.sampling = None;
    }

    pub fn closed(&mut self) {
        self.camera.state = CameraState::Closed;
        self.sampling = None;
    }

    /// Start the drift estimate again from now: after a knock, or a change to the set-up.
    pub fn restart_monitor(&mut self) {
        let t = self.frame_time().unwrap_or_else(|| self.now());
        self.monitor.restart(t);
        self.status = None;
        self.status_t = f64::NEG_INFINITY;
    }

    pub fn absorb(&mut self, p: Processed) {
        self.live.count(&p);
        let t = self.base_s + p.t_s;
        self.live.size = Some((p.width, p.height));
        if let Some(d) = p.detail {
            self.live.detail = Some(d);
            self.live.detail_version += 1;
        }
        let um_per_px = self.um_per_px();
        if let Ok(fix) = &p.fix {
            let um = fix.position_px * um_per_px;
            self.trace.push_back([t, um]);
            while self.trace.front().is_some_and(|q| q[0] < t - TRACE_S) {
                self.trace.pop_front();
            }
            self.monitor.push(t, um);
        }
        if t - self.status_t >= STATUS_EVERY_S && !self.monitor.bins().is_empty() {
            self.status = Some(self.monitor.status(&self.plan));
            self.status_t = t;
        }
        if let Some(s) = &mut self.sampling
            && t >= s.started_s
        {
            s.sampler.push(t, &p.fix);
            if s.sampler.done(t, &self.sample_rule, um_per_px) {
                self.finish_sample();
            }
        }
        self.live.fix = Some(p.fix);
    }

    /// Start a sample at `station`, replacing visit `replace` when given.
    pub fn take_sample(&mut self, station: usize, replace: Option<usize>) {
        let mut t = self.frame_time().unwrap_or_else(|| self.now());
        self.place_block(station);
        if let Some(b) = &self.bench {
            // Leave out the frames from before the block moved.
            t = t.max(b.moved_s + BLOCK_SETTLE_S);
        }
        self.survey_error = None;
        self.sampling = Some(Sampling { sampler: Sampler::new(t), station, replace, started_s: t });
    }

    /// The sample so far.
    pub fn sample_so_far(&self) -> Option<Reading> {
        self.sampling.as_ref().and_then(|s| s.sampler.reading())
    }

    /// End the sample now and keep what it has.
    pub fn finish_sample(&mut self) {
        let Some(s) = self.sampling.take() else { return };
        let subs = s.sampler.finish();
        let Some(run) = self.session.runs.get_mut(self.run) else { return };
        let result = match s.replace {
            Some(visit) => run.replace(visit, subs),
            None => run.record(s.station, subs).map(|_| ()),
        };
        if let Err(e) = result {
            self.survey_error = Some(e.to_string());
        }
        if let Some(next) = self.current_run().map(Run::next_station) {
            self.place_block(next);
        }
    }

    pub fn current_run(&self) -> Option<&Run> {
        self.session.runs.get(self.run)
    }

    /// Measure a simulated surface from now on: `block` moves the synthetic camera's line by the
    /// height of each station sampled.
    pub fn simulate(&mut self, block: Block) {
        self.bench = Some(Bench::new(block));
    }

    /// Put the synthetic camera's block on `station` of the run being measured, when it is not
    /// there already. The line moves by the station's height on the simulated surface.
    fn place_block(&mut self, station: usize) {
        // On the frames' clock: frames already on their way were made before the move.
        let now = self.frame_time().unwrap_or_else(|| self.now());
        let (Some(bench), Some(run)) = (&mut self.bench, self.session.runs.get(self.run)) else { return };
        if bench.at == Some((self.run, station)) || station >= run.stations.len() {
            return;
        }
        let surface = &bench.surface;
        let um = surface.seen_um(run, station) + surface.placement_um * bench.rng.normal();
        let scale = &self.session.scale;
        let sign = if scale.invert { -1.0 } else { 1.0 };
        bench.block.place(sign * um / scale.um_per_px);
        bench.at = Some((self.run, station));
        bench.moved_s = now;
    }

    /// Work on `session` from now on: a saved one, or a new one. The clock carries on from when
    /// the session was started, so new visits fit in with its old ones.
    pub fn load_session(&mut self, session: Session) {
        let session = prepared(session);
        let shift = self.session.started_unix - session.started_unix;
        self.base_s += shift;
        if let Some(b) = &mut self.bench {
            b.at = None;
            b.moved_s += shift;
        }
        self.run = session.runs.len() - 1;
        self.invert = session.scale.invert;
        self.session = session;
        self.sampling = None;
        self.survey_error = None;
        self.trace.clear();
        self.monitor = Monitor::new(self.monitor.config().clone());
        self.restart_monitor();
        self.sync_scale();
    }
}

/// A session ready to work on: with a start time and at least one run.
fn prepared(mut session: Session) -> Session {
    if session.started_unix <= 0.0 {
        session.started_unix = unix_now();
    }
    if session.runs.is_empty() {
        session.runs.push(Run::line("Line 1", 5, 100.0));
    }
    session
}

#[cfg(test)]
mod tests {
    use laser_camera::FrameSource;
    use laser_camera::synthetic::Synthetic;
    use laser_core::scale::Scale;
    use laser_core::survey::DriftCorrection;

    use super::*;

    #[test]
    fn preview_averages_blocks() {
        let mut data = vec![0u8; 2560 * 4];
        data[0] = 200;
        data[1] = 100;
        data[2560] = 100;
        let frame = Frame { width: 2560, height: 4, data, t_s: 0.0, seq: 0 };
        let p = Preview::of(&frame);
        assert_eq!((p.width, p.height, p.factor), (1280, 2, 2));
        assert_eq!(p.data[0], 100);
        assert_eq!(p.data.len(), 1280 * 2);
    }

    #[test]
    fn frames_feed_the_monitor_and_a_sample() {
        let mut cam = Synthetic::open(None);
        cam.paced = false;
        let mut engine = Engine::new(Session::new(Scale::from_pixel_pitch(3.0)));
        let (device, modes, mode, controls) = (cam.device().clone(), cam.modes().to_vec(), cam.mode(), cam.controls());
        engine.opened(device, modes, mode, controls, Vec::new());
        engine.sample_rule = SampleRule { min_s: 0.2, max_s: 0.5, target_um: None };
        engine.take_sample(0, None);
        // 1.3 s at 30 fps: past the first one-second bin.
        for k in 0..40 {
            let frame = cam.next_frame().unwrap();
            let p = process(&frame, &engine.finder, k % 5 == 0);
            engine.absorb(p);
        }
        assert_eq!(engine.live.frames, 40);
        assert!(engine.live.detail.is_some() && engine.live.detail_version == 8);
        assert!(engine.status.is_some());
        assert!(engine.sampling.is_none(), "the sample should have ended");
        assert_eq!(engine.current_run().unwrap().visits.len(), 1, "{:?}", engine.survey_error);
        assert!(engine.trace.len() >= 19);
    }

    #[test]
    fn the_synthetic_camera_reads_a_surface_station_by_station() {
        let small = Synthetic::open(None).modes()[3];
        let mut cam = Synthetic::open(Some(small));
        cam.paced = false;
        let block = Block::default();
        cam.block = Some(block.clone());
        let mut engine = Engine::new(Session::new(Scale::from_pixel_pitch(3.0)));
        let (device, modes, mode, controls) = (cam.device().clone(), cam.modes().to_vec(), cam.mode(), cam.controls());
        engine.opened(device, modes, mode, controls, Vec::new());
        engine.simulate(block.clone());
        engine.sample_rule = SampleRule { min_s: 1.0, max_s: 1.0, target_um: None };
        let stations = engine.current_run().unwrap().stations.len();
        // The zero, every station, and the zero again.
        for k in 0..stations + 1 {
            let station = engine.current_run().unwrap().next_station();
            engine.take_sample(station, None);
            while engine.sampling.is_some() {
                let frame = cam.next_frame().unwrap();
                engine.absorb(process(&frame, &engine.finder, false));
            }
            assert_eq!(engine.current_run().unwrap().visits.len(), k + 1, "{:?}", engine.survey_error);
        }
        let run = engine.current_run().unwrap();
        let heights = run.heights(&engine.session.scale, DriftCorrection::Interpolate).unwrap();
        let surface = Surface::default();
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for h in &heights.stations {
            let want = surface.seen_um(run, h.station);
            assert!((h.height_um - want).abs() < 1.5, "station {}: {} µm, not {want}", h.station, h.height_um);
            (lo, hi) = (lo.min(h.height_um), hi.max(h.height_um));
        }
        assert!(hi - lo > 3.0, "the surface should show: {lo}..{hi} µm");
    }
}
