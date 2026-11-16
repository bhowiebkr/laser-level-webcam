//! A simulated camera: a laser line on a lensless sensor, drifting as the set-up warms up, with
//! controls that act the way a webcam's do. It opens with the exposure held at 1/64 s, so the
//! beam is not clipped. Auto exposure, when it is switched on, meters the dark frame and clips the
//! beam; auto white balance makes a red beam's brightness wander; gain raises the noise;
//! sharpening rings beside the beam; room-light flicker rolls bands through the frame unless the
//! exposure is a whole number of flicker periods.
//!
//! It runs anywhere, the browser included, so the whole tool can be tried with no camera. While
//! measuring, the app moves a simulated [`Block`] over a surface, and the line moves with it, so
//! runs and their results have a shape to show.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use laser_core::camera::{
    ControlId, ControlInfo, ControlMode, ControlSetting, ControlUnit, Mains, PixelFormat, VideoMode, pick_mode,
};
use laser_core::frame::LineAxis;
use laser_core::rng::Rng;
use laser_core::synth::WarmUp;

use crate::{Backend, CameraError, DeviceInfo, Frame, FrameSource};

pub const NAME: &str = "Synthetic camera";

pub fn device() -> DeviceInfo {
    DeviceInfo { name: NAME.into(), id: "synthetic".into(), backend: Backend::Synthetic }
}

/// Width across the line that the scene's pixel numbers refer to.
const SCENE_ACROSS_PX: f64 = 1920.0;
/// Room-light ripple in the background, grey levels, before the exposure averages it.
const FLICKER_GREY: f64 = 6.0;
/// Size of the table of normal deviates the noise is drawn from.
const NOISE_LEN: usize = 1 << 20;

/// A simulated block on a surface, under the laser. A synthetic camera and the app that moves the
/// block share it; the app runs on another thread. Clones are the same block.
#[derive(Clone, Debug, Default)]
pub struct Block(Arc<AtomicU64>);

impl Block {
    /// Move the block: the line moves by `shift_px` from the next frame on, px across it in the
    /// mode open.
    pub fn place(&self, shift_px: f64) {
        self.0.store(shift_px.to_bits(), Ordering::Relaxed);
    }

    /// The shift the block puts on the line, px.
    pub fn shift_px(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::Relaxed))
    }
}

impl PartialEq for Block {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// What is in front of the sensor.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub axis: LineAxis,
    /// Line position across the sensor over time, in px of a 1920-wide frame.
    pub drift: WarmUp,
    pub sigma_px: f64,
    /// Change of position per px along the line.
    pub tilt: f64,
    /// Beam peak, grey levels, at 1/64 s, no gain, white balance 4600 K.
    pub beam: f64,
    /// Ambient light, grey levels, at 1/64 s.
    pub ambient: f64,
    pub read_noise: f64,
    /// Shot noise variance per grey level of light.
    pub shot_gain: f64,
    /// Mains frequency of the room lights.
    pub mains: Mains,
}

impl Default for Scene {
    /// Warming up from 10 px off to 960 px with a 90 s time constant: at 3 µm per px, the drift
    /// monitor says Ready after about 5 minutes, for a 10 minute run and 1 µm.
    fn default() -> Self {
        Self {
            axis: LineAxis::Vertical,
            drift: WarmUp { start: 970.0, settled: 960.0, tau_s: 90.0, steps: Vec::new(), jitter: 0.05 },
            sigma_px: 60.0,
            tilt: 0.0,
            beam: 150.0,
            ambient: 14.0,
            read_noise: 1.5,
            shot_gain: 0.4,
            mains: Mains::Hz50,
        }
    }
}

pub struct Synthetic {
    device: DeviceInfo,
    modes: Vec<VideoMode>,
    mode: VideoMode,
    pub scene: Scene,
    controls: Vec<ControlInfo>,
    noise: Vec<f32>,
    rng: Rng,
    seq: u64,
    /// Wait for each frame's time, like a camera. Off for tests and in the browser.
    pub paced: bool,
    /// The simulated block the line moves with. [`crate::open`] gives the camera one.
    pub block: Option<Block>,
    #[cfg(not(target_arch = "wasm32"))]
    started: Option<std::time::Instant>,
}

impl Synthetic {
    pub fn open(wanted: Option<VideoMode>) -> Self {
        let m = |width, height, fps, format| VideoMode { width, height, fps, format };
        let modes = vec![
            m(1920, 1080, 30.0, PixelFormat::Nv12),
            m(1920, 1080, 15.0, PixelFormat::Nv12),
            m(1280, 720, 30.0, PixelFormat::Nv12),
            m(640, 480, 30.0, PixelFormat::Yuyv),
        ];
        let mode = pick_mode(&modes, wanted.as_ref()).unwrap_or(modes[0]);
        let mut rng = Rng::new(0x5eed);
        let noise = (0..NOISE_LEN).map(|_| rng.normal() as f32).collect();
        Self {
            device: device(),
            modes,
            mode,
            scene: Scene::default(),
            controls: default_controls(),
            noise,
            rng,
            seq: 0,
            paced: !cfg!(test) && !cfg!(target_arch = "wasm32"),
            block: None,
            #[cfg(not(target_arch = "wasm32"))]
            started: None,
        }
    }

    fn control(&self, id: ControlId) -> &ControlInfo {
        self.controls.iter().find(|c| c.id == id).expect("every synthetic control exists")
    }

    /// Exposure as a multiple of 1/64 s, as the camera would choose or hold it.
    fn exposure_factor(&mut self, t_s: f64) -> f64 {
        let e = *self.control(ControlId::Exposure);
        if e.auto == Some(true) {
            // Auto exposure meters the whole frame, which is nearly all dark, so it opens up
            // until the background is mid grey, and hunts a little.
            let target = 100.0 / self.scene.ambient.max(1.0);
            let lo = 2f64.powi((e.min + 6) as i32);
            let hi = 2f64.powi((e.max + 6) as i32);
            let f = target.clamp(lo, hi) * (1.0 + 0.03 * (t_s * 1.7).sin());
            let c = self.controls.iter_mut().find(|c| c.id == ControlId::Exposure).expect("exists");
            c.value = c.snap(f.log2().round() as i64 - 6);
            f
        } else {
            2f64.powi((e.value + 6) as i32)
        }
    }

    fn render(&mut self, t_s: f64) -> Frame {
        let (w, h) = (self.mode.width as usize, self.mode.height as usize);
        let s = self.scene.clone();
        let (across_len, along_len) = match s.axis {
            LineAxis::Vertical => (w, h),
            LineAxis::Horizontal => (h, w),
        };
        let px = across_len as f64 / SCENE_ACROSS_PX;
        let block = self.block.as_ref().map_or(0.0, Block::shift_px);
        let centre = s.drift.sample(t_s, &mut self.rng) * px + block;
        let sigma = s.sigma_px * px;
        let mid = (along_len as f64 - 1.0) / 2.0;

        let exposure = self.exposure_factor(t_s);
        let gain = 1.0 + self.control(ControlId::Gain).value as f64 / 25.0;
        let wb = self.control(ControlId::WhiteBalance);
        let red = if wb.auto == Some(true) {
            1.0 + 0.04 * (t_s * std::f64::consts::TAU / 40.0).sin()
        } else {
            wb.value as f64 / 4600.0
        };
        let sharp = self.control(ControlId::Sharpness).value as f64 / 7.0 * 0.5;
        let filter = self.control(ControlId::PowerLineFrequency).value;
        let beam = s.beam * exposure * red;
        let ambient = s.ambient * exposure;

        // Light averaged over the exposure: the flicker's ripple shrinks as sinc(e/T) and goes
        // when the exposure is a whole number of periods. Under auto exposure with the filter
        // set to the mains, the camera picks such an exposure.
        let period = s.mains.flicker_period_s();
        let e_s = exposure / 64.0;
        let filtered = filter == s.mains.menu_value() && self.control(ControlId::Exposure).auto == Some(true);
        let ripple = if filtered {
            0.0
        } else {
            let x = std::f64::consts::PI * e_s / period;
            FLICKER_GREY * exposure.min(4.0) / 4.0 * (x.sin() / x).abs()
        };
        let row_time = self.mode.frame_period_s() / h as f64;

        let reach = 8.0 * sigma;
        let line = |d: f64| -> f64 {
            if d.abs() >= 2.0 * reach {
                return 0.0;
            }
            let g = (-0.5 * (d / sigma).powi(2)).exp();
            let wide = (-0.5 * (d / (2.0 * sigma)).powi(2)).exp();
            (1.0 + sharp) * g - sharp * wide
        };

        let mut data = vec![0u8; w * h];
        let mut light = vec![0f64; w];
        for y in 0..h {
            let band = ripple * (std::f64::consts::TAU * (t_s + y as f64 * row_time) / period).sin();
            match s.axis {
                LineAxis::Vertical => {
                    let c = centre + s.tilt * (y as f64 - mid);
                    for (x, l) in light.iter_mut().enumerate() {
                        *l = ambient + band + beam * line(x as f64 - c);
                    }
                }
                LineAxis::Horizontal => {
                    for (x, l) in light.iter_mut().enumerate() {
                        let c = centre + s.tilt * (x as f64 - mid);
                        *l = ambient + band + beam * line(y as f64 - c);
                    }
                }
            }
            let start = (self.rng.next_u64() as usize) % (NOISE_LEN - w);
            let noise = &self.noise[start..start + w];
            for ((out, &l), &n) in data[y * w..(y + 1) * w].iter_mut().zip(&light).zip(noise) {
                let sd = gain * (s.read_noise * s.read_noise + s.shot_gain * l.max(0.0)).sqrt();
                *out = (gain * l + sd * f64::from(n)).round().clamp(0.0, 255.0) as u8;
            }
        }
        Frame { width: w, height: h, data, t_s, seq: self.seq }
    }
}

fn default_controls() -> Vec<ControlInfo> {
    let c = |id, min, max, step, default, auto, unit| ControlInfo {
        id,
        min,
        max,
        step,
        default,
        value: default,
        auto,
        unit,
    };
    vec![
        // Held at 1/64 s, so the demo opens with a beam that is not clipped. Auto is there to show
        // why Lock for measuring holds it.
        c(ControlId::Exposure, -11, -3, 1, -6, Some(false), ControlUnit::Log2Seconds),
        c(ControlId::Gain, 0, 100, 1, 0, None, ControlUnit::Raw),
        c(ControlId::WhiteBalance, 2800, 6500, 10, 4600, Some(true), ControlUnit::Kelvin),
        c(ControlId::PowerLineFrequency, 0, 2, 1, 0, None, ControlUnit::PowerLineMenu),
        c(ControlId::Sharpness, 0, 7, 1, 3, None, ControlUnit::Raw),
        c(ControlId::BacklightCompensation, 0, 1, 1, 1, None, ControlUnit::Raw),
        c(ControlId::Focus, 0, 255, 1, 0, Some(true), ControlUnit::Raw),
    ]
}

impl FrameSource for Synthetic {
    fn device(&self) -> &DeviceInfo {
        &self.device
    }

    fn block(&self) -> Option<Block> {
        self.block.clone()
    }

    fn modes(&self) -> &[VideoMode] {
        &self.modes
    }

    fn mode(&self) -> VideoMode {
        self.mode
    }

    fn controls(&mut self) -> Vec<ControlInfo> {
        self.controls.clone()
    }

    fn set_control(&mut self, setting: ControlSetting) -> Result<ControlInfo, CameraError> {
        let c = self.controls.iter_mut().find(|c| c.id == setting.id).ok_or(CameraError::NoControl(setting.id))?;
        match setting.mode {
            ControlMode::Auto if c.auto.is_some() => c.auto = Some(true),
            ControlMode::Auto => {}
            ControlMode::Manual(v) => {
                c.value = c.snap(v);
                if c.auto.is_some() {
                    c.auto = Some(false);
                }
            }
        }
        Ok(*c)
    }

    fn next_frame(&mut self) -> Result<Frame, CameraError> {
        let t_s = self.seq as f64 * self.mode.frame_period_s();
        #[cfg(not(target_arch = "wasm32"))]
        if self.paced {
            let start = *self.started.get_or_insert_with(std::time::Instant::now);
            let due = start + std::time::Duration::from_secs_f64(t_s);
            if let Some(wait) = due.checked_duration_since(std::time::Instant::now()) {
                std::thread::sleep(wait);
            }
        }
        let frame = self.render(t_s);
        self.seq += 1;
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use laser_core::beam::{FinderConfig, find};
    use laser_core::camera::{Mains, for_measuring};

    use super::*;

    fn lock(cam: &mut Synthetic) {
        let infos = cam.controls();
        for s in for_measuring(&infos, Mains::Hz50) {
            cam.set_control(s).unwrap();
        }
    }

    #[test]
    fn auto_exposure_clips_and_locking_fixes_it() {
        let mut cam = Synthetic::open(None);
        let cfg = FinderConfig::default();
        // It opens with the exposure held, so the first frame is not clipped.
        let f = cam.next_frame().unwrap();
        let fix = find(&f.luma().unwrap(), &cfg).unwrap();
        assert!(!fix.warnings.clipped, "the Synthetic camera should open unclipped: {fix:?}");

        cam.set_control(ControlSetting { id: ControlId::Exposure, mode: ControlMode::Auto }).unwrap();
        let f = cam.next_frame().unwrap();
        let fix = find(&f.luma().unwrap(), &cfg).unwrap();
        assert!(fix.warnings.clipped, "auto exposure should clip the beam: {fix:?}");

        lock(&mut cam);
        cam.set_control(ControlSetting { id: ControlId::Exposure, mode: ControlMode::Manual(-6) }).unwrap();
        let f = cam.next_frame().unwrap();
        let fix = find(&f.luma().unwrap(), &cfg).unwrap();
        assert!(!fix.warnings.clipped, "{fix:?}");
        let truth = cam.scene.drift.mean_at(f.t_s);
        assert!((fix.position_px - truth).abs() < 0.5, "{} vs {truth}", fix.position_px);
        assert_eq!(f.seq, 2);
    }

    #[test]
    fn modes_scale_the_scene() {
        let small = VideoMode { width: 640, height: 480, fps: 30.0, format: PixelFormat::Yuyv };
        let mut cam = Synthetic::open(Some(small));
        assert_eq!(cam.mode(), small);
        lock(&mut cam);
        cam.scene.drift = WarmUp { start: 960.0, settled: 960.0, tau_s: 1.0, steps: Vec::new(), jitter: 0.0 };
        let f = cam.next_frame().unwrap();
        assert_eq!(f.data.len(), 640 * 480);
        let fix = find(&f.luma().unwrap(), &FinderConfig::default()).unwrap();
        assert!((fix.position_px - 320.0).abs() < 0.5, "{fix:?}");
    }

    #[test]
    fn controls_snap_and_refuse_unknown() {
        let mut cam = Synthetic::open(None);
        let wb =
            cam.set_control(ControlSetting { id: ControlId::WhiteBalance, mode: ControlMode::Manual(4604) }).unwrap();
        assert_eq!((wb.value, wb.auto), (4600, Some(false)));
        let back = cam.set_control(ControlSetting { id: ControlId::WhiteBalance, mode: ControlMode::Auto }).unwrap();
        assert_eq!(back.auto, Some(true));
        let err = cam.set_control(ControlSetting { id: ControlId::Zoom, mode: ControlMode::Manual(1) });
        assert_eq!(err, Err(CameraError::NoControl(ControlId::Zoom)));
    }
}
