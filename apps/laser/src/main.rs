//! `laser`: the beam finder, the heat-drift monitor and the survey results from the command line.
//! Every command prints JSON, so scripts and tests can read the answer.

mod report;

use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Parser, Subcommand, ValueEnum};
use laser_camera::Applied;
use laser_core::beam::{self, FinderConfig, Fix, NoBeam};
use laser_core::camera::{self, CameraSettings, ControlId, ControlMode, ControlSetting, Mains, PixelFormat, VideoMode};
use laser_core::frame::{LineAxis, LumaImage};
use laser_core::monitor::{Monitor, MonitorConfig, RunPlan, Status};
use laser_core::results::Results;
use laser_core::rng::Rng;
use laser_core::scale::Scale;
use laser_core::survey::{DriftCorrection, Run, Session};
use laser_core::synth::{LineSpec, SurveySim, WarmUp};
use laser_core::units::Unit;
use serde::Serialize;

#[derive(Parser)]
#[command(
    name = "laser",
    version,
    about = "Laser level webcam: find the beam in a frame, watch the heat drift settle, and work out straightness and flatness. Prints JSON."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct CameraArgs {
    /// Camera name; the first camera if left out.
    name: Option<String>,
    /// Print only the cameras' names.
    #[arg(long)]
    list: bool,
    /// Print what the camera itself sends, read from its USB descriptors (works while another
    /// program streams it), and nothing else.
    #[arg(long)]
    usb: bool,
    /// Set a control first, as `id=value` or `id=auto`, e.g. `exposure=-7`, `white_balance=auto`.
    #[arg(long = "set", value_parser = parse_setting)]
    set: Vec<ControlSetting>,
    /// Lock the camera for measuring (exposure, white balance and focus held; gain, sharpness and backlight off).
    #[arg(long)]
    lock: bool,
    /// Mains frequency for --lock's power-line filter.
    #[arg(long, value_enum, default_value_t = MainsArg::Hz50)]
    mains: MainsArg,
    /// Grab this many frames and report the frame rate by the camera's clock.
    #[arg(long, default_value_t = 0)]
    frames: u64,
    /// Open this mode, as `WIDTHxHEIGHT[@FPS][:FORMAT]`, e.g. `1280x960` or `640x480@30:yuyv`.
    /// Left out: the largest raw-luma mode.
    #[arg(long, value_parser = parse_mode)]
    mode: Option<ModeArg>,
    /// Save the last frame's luma as PGM. With no --frames, 30 frames are grabbed first so
    /// automatic exposure can settle.
    #[arg(long)]
    save: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// List the cameras, or show a camera's modes and controls (setting some first with --set).
    Camera(CameraArgs),
    /// Find the beam in a PGM frame (P5 or P2, 8 or 16 bit).
    Find {
        frame: PathBuf,
        /// Direction the laser line runs on the sensor.
        #[arg(long, value_enum, default_value_t = Axis::Vertical)]
        axis: Axis,
        /// Strips along the line, each read on its own.
        #[arg(long, default_value_t = FinderConfig::default().strips)]
        strips: usize,
    },
    /// Write a synthetic frame with a known beam position as PGM. Prints the spec, which is the truth.
    Synth {
        out: PathBuf,
        #[command(flatten)]
        spec: SpecArgs,
    },
    /// Run the heat-drift monitor over a log of `t_s,x` lines and print its status.
    Settle {
        log: PathBuf,
        /// Length of the planned measuring run, s.
        #[arg(long, default_value_t = 600.0)]
        run_s: f64,
        /// Largest drift allowed over the run, in the units of x.
        #[arg(long, default_value_t = 0.5)]
        tol: f64,
        /// Also print the status every N seconds of log, one JSON object per line.
        #[arg(long)]
        every_s: Option<f64>,
    },
    /// Write a synthetic warm-up log of `t_s,x` lines: x = settled + (start − settled)·e^(−t/τ).
    WarmUp {
        out: PathBuf,
        #[arg(long, default_value_t = 20.0)]
        start: f64,
        #[arg(long, default_value_t = 0.0)]
        settled: f64,
        #[arg(long, default_value_t = 300.0)]
        tau_s: f64,
        /// Reading noise, standard deviation.
        #[arg(long, default_value_t = 0.3)]
        jitter: f64,
        /// Length of the log, s.
        #[arg(long, default_value_t = 1800.0)]
        seconds: f64,
        #[arg(long, default_value_t = 20.0)]
        fps: f64,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// A bump of SIZE at T seconds, as T:SIZE. Repeat for more.
        #[arg(long = "step", value_parser = parse_step)]
        steps: Vec<(f64, f64)>,
    },
    /// Write a simulated survey of a known surface as a session file. Prints the true heights.
    SurveySim {
        out: PathBuf,
        /// Stations along a line.
        #[arg(long, default_value_t = 11)]
        stations: usize,
        /// A grid of NX by NY stations instead of a line, as NXxNY.
        #[arg(long, value_parser = parse_grid)]
        grid: Option<(usize, usize)>,
        /// Distance between stations, mm.
        #[arg(long, default_value_t = 100.0)]
        spacing_mm: f64,
        /// Bow of the surface, µm: half a sine wave along the line (and across, on a grid).
        #[arg(long, default_value_t = 10.0, allow_negative_numbers = true)]
        bow_um: f64,
        /// Slope of the laser reference to the surface, µm/m.
        #[arg(long, default_value_t = 50.0, allow_negative_numbers = true)]
        tilt_um_per_m: f64,
        /// Steady drift of the laser or the camera, µm/min.
        #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
        drift_um_per_min: f64,
        /// Spread of where the block sits at a station, µm.
        #[arg(long, default_value_t = 0.0)]
        placement_um: f64,
        #[arg(long, default_value_t = 2.0)]
        um_per_px: f64,
        /// Come back along the line too, so every station is read twice.
        #[arg(long)]
        forward_back: bool,
        #[arg(long, default_value_t = 1)]
        seed: u64,
    },
    /// Straightness or flatness of a run in a session file, with scrape and shim per station.
    Results {
        session: PathBuf,
        /// Run by name or by number from 1. The last run when left out.
        #[arg(long)]
        run: Option<String>,
        /// Drift correction. The session's when left out.
        #[arg(long, value_enum)]
        drift: Option<DriftArg>,
        /// Also write the results as CSV to this file.
        #[arg(long)]
        csv: Option<PathBuf>,
        /// Unit of the CSV. The session's when left out.
        #[arg(long, value_enum)]
        unit: Option<UnitArg>,
    },
    /// Write an issue report, check it, search for duplicates and file it with gh. Sends
    /// nothing without --submit.
    Report {
        #[command(subcommand)]
        action: report::Action,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Axis {
    Vertical,
    Horizontal,
}

impl From<Axis> for LineAxis {
    fn from(a: Axis) -> Self {
        match a {
            Axis::Vertical => LineAxis::Vertical,
            Axis::Horizontal => LineAxis::Horizontal,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum DriftArg {
    /// Between the zero readings before and after, in time.
    Interpolate,
    /// Against the last zero reading before.
    LastZero,
}

impl From<DriftArg> for DriftCorrection {
    fn from(d: DriftArg) -> Self {
        match d {
            DriftArg::Interpolate => DriftCorrection::Interpolate,
            DriftArg::LastZero => DriftCorrection::LastZero,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum UnitArg {
    Um,
    Mm,
    In,
}

impl From<UnitArg> for Unit {
    fn from(u: UnitArg) -> Self {
        match u {
            UnitArg::Um => Unit::Um,
            UnitArg::Mm => Unit::Mm,
            UnitArg::In => Unit::Inch,
        }
    }
}

/// Changes to the reference synthetic frame (1920×1080, vertical line at 960 px, sigma 80 px).
#[derive(Args)]
struct SpecArgs {
    #[arg(long)]
    width: Option<usize>,
    #[arg(long)]
    height: Option<usize>,
    #[arg(long, value_enum)]
    axis: Option<Axis>,
    /// Beam centre across the line, px.
    #[arg(long)]
    centre_px: Option<f64>,
    /// Beam width as a Gaussian sigma, px.
    #[arg(long)]
    sigma_px: Option<f64>,
    /// Slope of the line, px across per px along.
    #[arg(long)]
    tilt: Option<f64>,
    /// Beam peak above the background, grey levels.
    #[arg(long)]
    amplitude: Option<f64>,
    #[arg(long)]
    background: Option<f64>,
    /// Ambient light slope across the sensor, grey levels per px.
    #[arg(long)]
    gradient: Option<f64>,
    #[arg(long)]
    read_noise: Option<f64>,
    #[arg(long)]
    shot_gain: Option<f64>,
    #[arg(long)]
    seed: Option<u64>,
    /// Background only, no beam.
    #[arg(long)]
    no_line: bool,
}

impl SpecArgs {
    fn spec(&self) -> LineSpec {
        let v1 = LineSpec::v1();
        let spec = LineSpec {
            width: self.width.unwrap_or(v1.width),
            height: self.height.unwrap_or(v1.height),
            axis: self.axis.map_or(v1.axis, LineAxis::from),
            centre_px: self.centre_px.unwrap_or(v1.centre_px),
            tilt: self.tilt.unwrap_or(v1.tilt),
            sigma_px: self.sigma_px.unwrap_or(v1.sigma_px),
            amplitude: self.amplitude.unwrap_or(v1.amplitude),
            background: self.background.unwrap_or(v1.background),
            gradient: self.gradient.unwrap_or(v1.gradient),
            read_noise: self.read_noise.unwrap_or(v1.read_noise),
            shot_gain: self.shot_gain.unwrap_or(v1.shot_gain),
            seed: self.seed.unwrap_or(v1.seed),
        };
        if self.no_line { spec.without_line() } else { spec }
    }
}

/// What `find` prints: a fix, or why there is no reading.
#[derive(Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
enum Reading {
    Fix(Fix),
    NoBeam { reason: NoBeam, text: &'static str },
}

impl From<Result<Fix, NoBeam>> for Reading {
    fn from(r: Result<Fix, NoBeam>) -> Self {
        match r {
            Ok(fix) => Reading::Fix(fix),
            Err(reason) => Reading::NoBeam { reason, text: reason.text() },
        }
    }
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Camera(args) => camera(args),
        Command::Find { frame, axis, strips } => {
            let img = read_pgm(&frame)?;
            let cfg = FinderConfig { axis: axis.into(), strips, ..FinderConfig::default() };
            print_pretty(&Reading::from(beam::find(&img.view()?, &cfg)))
        }
        Command::Synth { out, spec } => {
            let spec = spec.spec();
            ensure!(spec.width > 0 && spec.height > 0, "the frame needs a width and a height");
            write_pgm(&out, &spec.render())?;
            print_pretty(&spec)
        }
        Command::Settle { log, run_s, tol, every_s } => {
            ensure!(run_s > 0.0 && tol > 0.0, "--run-s and --tol must be above zero");
            ensure!(every_s.is_none_or(|e| e > 0.0), "--every-s must be above zero");
            let text = fs::read_to_string(&log).with_context(|| format!("reading {}", log.display()))?;
            let rows = parse_log(&text).with_context(|| format!("reading {}", log.display()))?;
            let plan = RunPlan { run_s, tolerance: tol };
            let mut out = BufWriter::new(io::stdout().lock());
            let status = settle(&rows, &plan, every_s, |s| {
                serde_json::to_writer(&mut out, s)?;
                writeln!(out)?;
                Ok(())
            })?;
            out.flush()?;
            drop(out);
            if every_s.is_none() { print_pretty(&status) } else { Ok(()) }
        }
        Command::WarmUp { out, start, settled, tau_s, jitter, seconds, fps, seed, steps } => {
            ensure!(tau_s > 0.0 && seconds > 0.0 && fps > 0.0, "--tau-s, --seconds and --fps must be above zero");
            let w = WarmUp { start, settled, tau_s, steps, jitter };
            write_log(&out, &w, seconds, fps, seed)
        }
        Command::SurveySim {
            out,
            stations,
            grid,
            spacing_mm,
            bow_um,
            tilt_um_per_m,
            drift_um_per_min,
            placement_um,
            um_per_px,
            forward_back,
            seed,
        } => {
            ensure!(spacing_mm > 0.0 && um_per_px > 0.0, "--spacing-mm and --um-per-px must be above zero");
            ensure!(placement_um >= 0.0, "--placement-um cannot be negative");
            let mut run = match grid {
                Some((nx, ny)) => {
                    ensure!(nx >= 2 && ny >= 2, "a grid needs at least 2x2 stations");
                    Run::grid("simulated", nx, ny, spacing_mm, spacing_mm)
                }
                None => {
                    ensure!(stations >= 3, "a line needs at least 3 stations");
                    Run::line("simulated", stations, spacing_mm)
                }
            };
            let n = run.stations.len();
            let (lx, ly) = run.stations.iter().fold((0.0_f64, 0.0_f64), |(a, b), s| (a.max(s.x_mm), b.max(s.y_mm)));
            let half_sine = |v: f64, l: f64| if l > 0.0 { (std::f64::consts::PI * v / l).sin() } else { 1.0 };
            let mut sim = SurveySim::flat(n);
            sim.scale = Scale::from_pixel_pitch(um_per_px);
            sim.surface_um =
                run.stations.iter().map(|s| bow_um * half_sine(s.x_mm, lx) * half_sine(s.y_mm, ly)).collect();
            sim.reference = [0.0, tilt_um_per_m / 1000.0, 0.0];
            sim.drift_px_per_s = drift_um_per_min / 60.0 / um_per_px;
            sim.placement_um = placement_um;
            let mut order: Vec<usize> = (0..n).collect();
            if forward_back {
                order.extend((1..n - 1).rev());
            }
            order.push(0);
            let end_s = sim.survey(&mut run, &order, 0.0, &mut Rng::new(seed))?;
            let truth: Vec<TrueHeight> = run
                .stations
                .iter()
                .enumerate()
                .map(|(i, s)| TrueHeight {
                    station: i + 1,
                    x_mm: s.x_mm,
                    y_mm: s.y_mm,
                    height_um: sim.true_height_um(&run, i),
                })
                .collect();
            let mut session = Session::new(sim.scale);
            session.runs.push(run);
            write_atomic(&out, session.to_json().as_bytes())?;
            print_pretty(&SimTruth { end_s, readings: order.len(), stations: truth })
        }
        Command::Results { session, run, drift, csv, unit } => {
            let text = fs::read_to_string(&session).with_context(|| format!("reading {}", session.display()))?;
            let s = Session::from_json(&text).with_context(|| format!("opening {}", session.display()))?;
            let r = pick_run(&s, run.as_deref())?;
            let mode = drift.map_or(s.drift_correction, DriftCorrection::from);
            let results = Results::compute(r, &r.scale(&s.scale, &s.sensor), mode).with_context(|| format!("run {:?}", r.name))?;
            if let Some(path) = csv {
                let unit = unit.map_or(s.unit, Unit::from);
                write_atomic(&path, results.csv(unit).as_bytes())?;
            }
            print_pretty(&RunResults { run: &r.name, drift_correction: mode, results })
        }
        Command::Report { action } => {
            let (out, ok) = report::run(action)?;
            print_pretty(&out)?;
            if !ok {
                io::stdout().flush()?;
                std::process::exit(1);
            }
            Ok(())
        }
    }
}

#[derive(Serialize)]
struct TrueHeight {
    station: usize,
    x_mm: f64,
    y_mm: f64,
    /// Above the zero station, under the tilted reference: what the survey should read, µm.
    height_um: f64,
}

#[derive(Serialize)]
struct SimTruth {
    /// Time of the last reading, s.
    end_s: f64,
    readings: usize,
    stations: Vec<TrueHeight>,
}

#[derive(Serialize)]
struct RunResults<'a> {
    run: &'a str,
    drift_correction: DriftCorrection,
    #[serde(flatten)]
    results: Results,
}

/// A run by name, or by number from 1; the last run when `None`.
fn pick_run<'a>(s: &'a Session, which: Option<&str>) -> Result<&'a Run> {
    let Some(which) = which else {
        return s.runs.last().context("the session has no runs");
    };
    if let Some(r) = s.runs.iter().find(|r| r.name == which) {
        return Ok(r);
    }
    match which.parse::<usize>() {
        Ok(k) if (1..=s.runs.len()).contains(&k) => Ok(&s.runs[k - 1]),
        _ => bail!("no run {which:?}; the session has {} run(s)", s.runs.len()),
    }
}

/// Write to a file beside `path`, then rename it over `path`, so a failed write leaves the old file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

fn parse_grid(s: &str) -> Result<(usize, usize), String> {
    let (x, y) = s.split_once(['x', 'X']).ok_or("expected NXxNY, such as 5x5")?;
    let n = |v: &str| v.trim().parse::<usize>().map_err(|e| format!("{v:?}: {e}"));
    Ok((n(x)?, n(y)?))
}

#[derive(Clone, Copy, ValueEnum)]
enum MainsArg {
    #[value(name = "50")]
    Hz50,
    #[value(name = "60")]
    Hz60,
}

fn parse_setting(s: &str) -> std::result::Result<ControlSetting, String> {
    let (id, value) = s.split_once('=').ok_or("expected id=value")?;
    let id: ControlId = serde_json::from_value(serde_json::Value::String(id.trim().into()))
        .map_err(|_| format!("unknown control {id:?}; the controls are {}", control_names()))?;
    let mode = match value.trim() {
        "auto" => ControlMode::Auto,
        v => ControlMode::Manual(v.parse().map_err(|_| format!("{v:?} is not a whole number or auto"))?),
    };
    Ok(ControlSetting { id, mode })
}

/// A camera mode asked for on the command line; what is left out is chosen from the camera's modes.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ModeArg {
    width: u32,
    height: u32,
    fps: Option<f64>,
    format: Option<PixelFormat>,
}

impl ModeArg {
    /// The camera's best mode that matches: raw luma first, then the highest frame rate.
    fn pick(&self, modes: &[VideoMode]) -> Option<VideoMode> {
        modes
            .iter()
            .copied()
            .filter(|m| m.width == self.width && m.height == self.height)
            .filter(|m| self.fps.is_none_or(|f| (m.fps - f).abs() < 0.5))
            .filter(|m| self.format.is_none_or(|f| m.format == f))
            .max_by(|a, b| a.format.is_raw_luma().cmp(&b.format.is_raw_luma()).then(a.fps.total_cmp(&b.fps)))
    }
}

fn parse_mode(s: &str) -> std::result::Result<ModeArg, String> {
    let (rest, format) = match s.split_once(':') {
        Some((rest, f)) => {
            let format = serde_json::from_value(serde_json::Value::String(f.trim().to_lowercase()))
                .map_err(|_| format!("unknown format {f:?}; the formats are nv12, yuyv, grey, mjpeg, rgba"))?;
            (rest, Some(format))
        }
        None => (s, None),
    };
    let (size, fps) = match rest.split_once('@') {
        Some((size, f)) => (size, Some(f.trim().parse().map_err(|_| format!("{f:?} is not a frame rate"))?)),
        None => (rest, None),
    };
    let (w, h) = size.split_once(['x', 'X']).ok_or("expected WIDTHxHEIGHT")?;
    let n = |v: &str| v.trim().parse().map_err(|_| format!("{v:?} is not a whole number"));
    Ok(ModeArg { width: n(w)?, height: n(h)?, fps, format })
}

fn control_names() -> String {
    ControlId::ALL
        .iter()
        .filter_map(|id| serde_json::to_value(id).ok()?.as_str().map(str::to_owned))
        .collect::<Vec<_>>()
        .join(", ")
}

fn applied_json(a: &Applied) -> serde_json::Value {
    match &a.result {
        Ok(info) => serde_json::json!({ "setting": a.setting, "held": a.held(), "now": info }),
        Err(e) => serde_json::json!({ "setting": a.setting, "held": false, "error": e.to_string() }),
    }
}

fn camera(args: CameraArgs) -> Result<()> {
    let CameraArgs { name, list, usb, set, lock, mains, frames, mode, save } = args;
    if list {
        let names: Vec<_> = laser_camera::devices().into_iter().map(|d| d.name).collect();
        return print_pretty(&names);
    }
    if usb {
        let all = laser_camera::devices();
        let device = match &name {
            Some(n) => all.iter().find(|d| &d.name == n).ok_or_else(|| anyhow::anyhow!("no camera {n:?}"))?,
            None => all.first().ok_or_else(|| anyhow::anyhow!("no camera"))?,
        };
        let u = laser_camera::usb(device)?;
        println!(
            "{}  {:04x}:{:04x}  USB {:x}.{:02x}  {}",
            device.name,
            u.vendor_id,
            u.product_id,
            u.usb_version >> 8,
            u.usb_version & 0xff,
            u.speed.map_or("speed unknown", |s| s.name())
        );
        match u.max_bytes_per_s {
            Some(b) => println!("video link carries at most {:.1} MB/s", b as f64 / 1e6),
            None => println!("bulk video endpoint (no fixed bandwidth)"),
        }
        for f in &u.formats {
            println!("format {}: {}  {} bits/pixel", f.index, f.encoding.name(), f.bits_per_pixel);
            for fr in &f.frames {
                let fps: Vec<String> = fr.fps.iter().map(|r| format!("{r:.1}")).collect();
                println!(
                    "  {}x{}  fps {}  (max {:.1} Mbit/s)",
                    fr.width,
                    fr.height,
                    fps.join(" "),
                    f64::from(fr.max_bit_rate) / 1e6
                );
            }
        }
        return Ok(());
    }
    let mut settings = CameraSettings { device: name, controls: set, ..CameraSettings::default() };
    let (mut cam, mut applied) = laser_camera::open(&settings)?;
    if let Some(want) = mode {
        let Some(found) = want.pick(cam.modes()) else {
            let mut sizes: Vec<_> = cam.modes().iter().map(|m| (m.width, m.height)).collect();
            sizes.sort_unstable_by(|a, b| b.cmp(a));
            sizes.dedup();
            let sizes: Vec<_> = sizes.iter().map(|(w, h)| format!("{w}x{h}")).collect();
            bail!("the camera has no mode {want:?}; its sizes are {}", sizes.join(", "));
        };
        if found != cam.mode() {
            // The mode is chosen when the camera opens, so open it again.
            drop(cam);
            settings.mode = Some(found);
            (cam, applied) = laser_camera::open(&settings)?;
            ensure!(cam.mode() == found, "the camera opened in {:?}, not {found:?}", cam.mode());
        }
    }
    let frames = if save.is_some() && frames == 0 { 30 } else { frames };
    if lock {
        let mains = match mains {
            MainsArg::Hz50 => Mains::Hz50,
            MainsArg::Hz60 => Mains::Hz60,
        };
        let infos = cam.controls();
        applied.extend(laser_camera::apply(cam.as_mut(), &camera::for_measuring(&infos, mains)));
    }
    let mut grabbed = serde_json::Value::Null;
    if frames > 0 {
        let first = cam.next_frame()?;
        let mut last = first.clone();
        for _ in 0..frames {
            last = cam.next_frame()?;
        }
        let span = last.t_s - first.t_s;
        grabbed = serde_json::json!({
            "frames": frames,
            "lost": (last.seq - first.seq).saturating_sub(frames),
            "fps": if span > 0.0 { (last.seq - first.seq) as f64 / span } else { 0.0 },
            "peak": last.data.iter().copied().max().unwrap_or(0),
        });
        if let Some(path) = &save {
            write_pgm(path, &LumaImage { width: last.width, height: last.height, data: last.data })?;
        }
    }
    print_pretty(&serde_json::json!({
        "device": cam.device().name,
        "mode": cam.mode(),
        "modes": cam.modes(),
        "controls": cam.controls(),
        "applied": applied.iter().map(applied_json).collect::<Vec<_>>(),
        "grabbed": grabbed,
    }))
}

fn print_pretty<T: Serialize>(value: &T) -> Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, value)?;
    writeln!(out)?;
    Ok(())
}

/// Feed the log to a monitor; with `every_s`, hand over a status at each multiple of it. Returns
/// the status at the end.
fn settle(
    rows: &[(f64, f64)],
    plan: &RunPlan,
    every_s: Option<f64>,
    mut emit: impl FnMut(&Status) -> Result<()>,
) -> Result<Status> {
    let mut m = Monitor::new(MonitorConfig::default());
    let mut next = every_s.map(|e| rows[0].0 + e);
    for &(t, x) in rows {
        if let (Some(at), Some(e)) = (next, every_s)
            && t >= at
        {
            emit(&m.status(plan))?;
            next = Some(at + e * ((t - at) / e).floor() + e);
        }
        m.push(t, x);
    }
    let status = m.status(plan);
    if every_s.is_some() {
        emit(&status)?;
    }
    Ok(status)
}

/// `t_s,x` per line. Commas, semicolons, tabs or spaces separate; `#` lines and a header are skipped.
fn parse_log(text: &str) -> Result<Vec<(f64, f64)>> {
    let mut rows: Vec<(f64, f64)> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split([',', ';', '\t', ' ']).filter(|c| !c.is_empty());
        let parsed = match (cols.next(), cols.next()) {
            (Some(t), Some(x)) => t.parse::<f64>().ok().zip(x.parse::<f64>().ok()),
            _ => None,
        };
        match parsed {
            Some((t, x)) if t.is_finite() && x.is_finite() => {
                if let Some(&(last, _)) = rows.last() {
                    ensure!(t >= last, "line {}: time {t} is before {last}", i + 1);
                }
                rows.push((t, x));
            }
            None if rows.is_empty() => {}
            _ => bail!("line {}: {line:?} is not a time and a reading", i + 1),
        }
    }
    ensure!(!rows.is_empty(), "no readings");
    Ok(rows)
}

fn write_log(path: &Path, w: &WarmUp, seconds: f64, fps: f64, seed: u64) -> Result<()> {
    let file = fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut out = BufWriter::new(file);
    let mut rng = Rng::new(seed);
    writeln!(out, "t_s,x")?;
    let mut k = 0u64;
    loop {
        let t = k as f64 / fps;
        if t >= seconds {
            break;
        }
        writeln!(out, "{t:.3},{:.4}", w.sample(t, &mut rng))?;
        k += 1;
    }
    out.flush()?;
    Ok(())
}

fn read_pgm(path: &Path) -> Result<LumaImage> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    parse_pgm(&bytes).with_context(|| format!("{} is not a PGM frame", path.display()))
}

/// PGM, binary (P5) or text (P2). Frames deeper than 8 bits are scaled to 0..=255.
fn parse_pgm(b: &[u8]) -> Result<LumaImage> {
    let mut pos = 0;
    let binary = match token(b, &mut pos)? {
        b"P5" => true,
        b"P2" => false,
        _ => bail!("it does not start with P5 or P2"),
    };
    let width = number(b, &mut pos)?;
    let height = number(b, &mut pos)?;
    let maxval = number(b, &mut pos)?;
    ensure!(width > 0 && height > 0, "the frame is empty");
    ensure!((1..=65535).contains(&maxval), "the maximum value {maxval} is not 1 to 65535");
    let n = width.checked_mul(height).context("the frame is too large")?;
    let scale = |v: usize| ((v.min(maxval) * 255 + maxval / 2) / maxval) as u8;
    let data = if binary {
        pos += 1; // the single whitespace byte after the maximum value
        let wide = maxval > 255;
        let need = if wide { n.checked_mul(2).context("the frame is too large")? } else { n };
        let px =
            b.get(pos..pos.saturating_add(need)).with_context(|| format!("it has fewer than {need} pixel bytes"))?;
        if wide {
            px.chunks_exact(2).map(|c| scale(usize::from(u16::from_be_bytes([c[0], c[1]])))).collect()
        } else if maxval == 255 {
            px.to_vec()
        } else {
            px.iter().map(|&v| scale(usize::from(v))).collect()
        }
    } else {
        (0..n).map(|_| number(b, &mut pos).map(scale)).collect::<Result<_>>()?
    };
    Ok(LumaImage { width, height, data })
}

/// Next whitespace-separated token, skipping `#` comments. Leaves `pos` on the byte after it.
fn token<'a>(b: &'a [u8], pos: &mut usize) -> Result<&'a [u8]> {
    loop {
        match b.get(*pos) {
            Some(c) if c.is_ascii_whitespace() => *pos += 1,
            Some(b'#') => {
                while b.get(*pos).is_some_and(|&c| c != b'\n') {
                    *pos += 1;
                }
            }
            Some(_) => break,
            None => bail!("the header or pixels end early"),
        }
    }
    let start = *pos;
    while b.get(*pos).is_some_and(|c| !c.is_ascii_whitespace()) {
        *pos += 1;
    }
    Ok(&b[start..*pos])
}

fn number(b: &[u8], pos: &mut usize) -> Result<usize> {
    let t = token(b, pos)?;
    std::str::from_utf8(t)
        .ok()
        .and_then(|s| s.parse().ok())
        .with_context(|| format!("{:?} is not a number", String::from_utf8_lossy(t)))
}

fn write_pgm(path: &Path, img: &LumaImage) -> Result<()> {
    let file = fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut out = BufWriter::new(file);
    write!(out, "P5\n{} {}\n255\n", img.width, img.height)?;
    out.write_all(&img.data)?;
    out.flush()?;
    Ok(())
}

fn parse_step(s: &str) -> Result<(f64, f64), String> {
    let (t, size) = s.split_once(':').ok_or("expected T:SIZE")?;
    let t = t.trim().parse().map_err(|_| format!("{t:?} is not a time"))?;
    let size = size.trim().parse().map_err(|_| format!("{size:?} is not a size"))?;
    Ok((t, size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use laser_core::monitor::Readiness;

    #[test]
    fn binary_pgm_round_trip_finds_the_beam() {
        let spec = LineSpec { centre_px: 700.3, ..LineSpec::v1() };
        let img = spec.render();
        let mut bytes = format!("P5\n# a comment\n{} {}\n255\n", img.width, img.height).into_bytes();
        bytes.extend_from_slice(&img.data);
        let back = parse_pgm(&bytes).unwrap();
        assert_eq!(back.data, img.data);
        let Reading::Fix(fix) = Reading::from(beam::find(&back.view().unwrap(), &FinderConfig::default())) else {
            panic!("no fix");
        };
        assert!((fix.position_px - 700.3).abs() < 0.1, "{fix:?}");
    }

    #[test]
    fn text_and_16_bit_pgm() {
        let text = parse_pgm(b"P2\n3 1\n15\n0 15 7\n").unwrap();
        assert_eq!(text.data, [0, 255, 119]);
        let wide = parse_pgm(b"P5 2 1 65535\n\xff\xff\x00\x00").unwrap();
        assert_eq!(wide.data, [255, 0]);
    }

    #[test]
    fn bad_pgm_is_an_error() {
        assert!(parse_pgm(b"P6\n1 1\n255\n\0\0\0").is_err());
        assert!(parse_pgm(b"P5\n4 4\n255\n\0\0").is_err());
        assert!(parse_pgm(b"P5\n0 4\n255\n").is_err());
    }

    #[test]
    fn log_with_header_and_comments() {
        let rows = parse_log("t_s,x\n# warm start\n0,1.5\n0.5;1.25\n1\t1.0\n").unwrap();
        assert_eq!(rows, [(0.0, 1.5), (0.5, 1.25), (1.0, 1.0)]);
        assert!(parse_log("0,1\n2,oops\n").is_err());
        assert!(parse_log("2,1\n1,1\n").is_err());
        assert!(parse_log("t,x\n").is_err());
    }

    #[test]
    fn settle_prints_on_the_interval_and_ends_ready() {
        let w = WarmUp { start: 0.0, settled: 0.0, tau_s: 1.0, steps: vec![], jitter: 0.1 };
        let mut rng = Rng::new(3);
        let rows: Vec<(f64, f64)> =
            (0..1800).map(|k| f64::from(k) / 20.0).map(|t| (t, w.sample(t, &mut rng))).collect();
        let mut seen = Vec::new();
        let plan = RunPlan { run_s: 600.0, tolerance: 0.5 };
        let last = settle(&rows, &plan, Some(30.0), |s| {
            seen.push(s.t_s);
            Ok(())
        })
        .unwrap();
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert_eq!(last.readiness, Readiness::Ready, "{last:?}");
    }

    #[test]
    fn step_argument() {
        assert_eq!(parse_step("200:3.5"), Ok((200.0, 3.5)));
        assert!(parse_step("200").is_err());
    }

    #[test]
    fn grid_argument_and_run_choice() {
        assert_eq!(parse_grid("5x3"), Ok((5, 3)));
        assert_eq!(parse_grid(" 4 X 2 "), Ok((4, 2)));
        assert!(parse_grid("5").is_err() && parse_grid("ax2").is_err());

        let mut s = Session::new(Scale::from_pixel_pitch(2.0));
        assert!(pick_run(&s, None).is_err());
        s.runs.push(Run::line("12", 3, 10.0));
        s.runs.push(Run::line("bed", 3, 10.0));
        assert_eq!(pick_run(&s, None).unwrap().name, "bed");
        assert_eq!(pick_run(&s, Some("bed")).unwrap().name, "bed");
        assert_eq!(pick_run(&s, Some("1")).unwrap().name, "12");
        // A name wins over a number.
        assert_eq!(pick_run(&s, Some("12")).unwrap().name, "12");
        assert!(pick_run(&s, Some("3")).is_err() && pick_run(&s, Some("0")).is_err());
    }
}
