//! What the camera is asked for, and what its sensor is. Plain data, saved with the session; the
//! `laser-camera` crate applies it to a real camera.
//!
//! A webcam is made for faces, and each of its automatics works against a measurement. Auto
//! exposure and auto gain change the beam's brightness from frame to frame. Auto white balance
//! changes the red gain, and the luma of a red beam with it. Sharpening rings at the beam's edges
//! and pulls the centroid. Room lights flicker at twice the mains frequency. [`for_measuring`]
//! locks them all, starting from what the camera reports.

use serde::{Deserialize, Serialize};

use crate::frame::LineAxis;
use crate::scale::Scale;

/// How the frames arrive. Only NV12, YUYV and grey carry the luma plane as it is; MJPEG has to be
/// decoded first, and RGBA (from a browser) has to be mixed back to luma.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    Nv12,
    Yuyv,
    Grey,
    Mjpeg,
    Rgba,
}

impl PixelFormat {
    /// The luma comes straight from the buffer, with no decoding and no colour mixing.
    pub fn is_raw_luma(self) -> bool {
        matches!(self, Self::Nv12 | Self::Yuyv | Self::Grey)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Nv12 => "NV12",
            Self::Yuyv => "YUYV",
            Self::Grey => "Grey",
            Self::Mjpeg => "MJPEG",
            Self::Rgba => "RGBA",
        }
    }
}

/// A size, frame rate and pixel format the camera offers.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoMode {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub format: PixelFormat,
}

impl VideoMode {
    pub fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// Time between frames, s.
    pub fn frame_period_s(&self) -> f64 {
        1.0 / self.fps
    }
}

impl std::fmt::Display for VideoMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}×{} {} @ {:.0} fps", self.width, self.height, self.format.name(), self.fps)
    }
}

/// The mode to open: the one asked for if the camera has it, else the most pixels with the luma
/// as it is, then the highest frame rate. A scaled-down mode averages pixels together and hides
/// the beam's shape; MJPEG adds block artefacts at the beam's edges.
pub fn pick_mode(modes: &[VideoMode], wanted: Option<&VideoMode>) -> Option<VideoMode> {
    if let Some(m) = wanted.and_then(|w| find_mode(modes, w)) {
        return Some(m);
    }
    modes.iter().copied().max_by(|a, b| {
        (a.format.is_raw_luma(), a.pixels(), a.fps >= 5.0)
            .cmp(&(b.format.is_raw_luma(), b.pixels(), b.fps >= 5.0))
            .then(a.fps.total_cmp(&b.fps))
    })
}

/// The mode in the list with the same size, format and frame rate.
pub fn find_mode(modes: &[VideoMode], wanted: &VideoMode) -> Option<VideoMode> {
    modes.iter().copied().find(|m| {
        m.width == wanted.width
            && m.height == wanted.height
            && m.format == wanted.format
            && (m.fps - wanted.fps).abs() < 0.5
    })
}

/// A camera setting the measurement cares about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlId {
    Exposure,
    Gain,
    WhiteBalance,
    PowerLineFrequency,
    Sharpness,
    BacklightCompensation,
    Focus,
    Gamma,
    Brightness,
    Contrast,
    Saturation,
    Hue,
    Zoom,
}

impl ControlId {
    pub const ALL: [ControlId; 13] = [
        Self::Exposure,
        Self::Gain,
        Self::WhiteBalance,
        Self::PowerLineFrequency,
        Self::Sharpness,
        Self::BacklightCompensation,
        Self::Focus,
        Self::Gamma,
        Self::Brightness,
        Self::Contrast,
        Self::Saturation,
        Self::Hue,
        Self::Zoom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Exposure => "Exposure",
            Self::Gain => "Gain",
            Self::WhiteBalance => "White balance",
            Self::PowerLineFrequency => "Power line filter",
            Self::Sharpness => "Sharpness",
            Self::BacklightCompensation => "Backlight compensation",
            Self::Focus => "Focus",
            Self::Gamma => "Gamma",
            Self::Brightness => "Brightness",
            Self::Contrast => "Contrast",
            Self::Saturation => "Saturation",
            Self::Hue => "Hue",
            Self::Zoom => "Zoom",
        }
    }

    /// What the setting does to the reading.
    pub fn help(self) -> &'static str {
        match self {
            Self::Exposure => {
                "How long each pixel collects light. Too long and the beam's top clips flat, which hides its \
                 centre; too short and the beam sinks into the noise. Auto exposure changes it between frames, \
                 so the beam's brightness wanders. Lock it, then set it so the peak sits below white."
            }
            Self::Gain => {
                "Amplification after the pixel. It raises the noise with the signal, so it never helps the \
                 reading. Keep it at the minimum and use exposure instead."
            }
            Self::WhiteBalance => {
                "Colour gains. A red beam's luma is mostly the red gain, so auto white balance changes the beam's \
                 brightness when the room light changes. Lock it."
            }
            Self::PowerLineFrequency => {
                "Room lights flicker at twice the mains frequency. Under auto exposure the filter keeps the \
                 exposure a whole number of flicker periods, which stops bright and dark bands rolling through \
                 the frame. Under manual exposure it does nothing: pick an exposure of whole flicker periods \
                 (10 ms at 50 Hz, 8.3 ms at 60 Hz), or light the target with the laser only."
            }
            Self::Sharpness => {
                "Edge enhancement. It adds dark rings beside the beam, which change its width and upset the \
                 background fit; on a lopsided beam they move the centre. Set it to the minimum."
            }
            Self::BacklightCompensation => {
                "Brightens dark parts of a scene with a bright light in it. It reshapes the beam's profile. \
                 Turn it off."
            }
            Self::Focus => {
                "With no lens it does nothing to the image, but an autofocus motor that hunts can shake the \
                 sensor. Lock it."
            }
            Self::Gamma => {
                "The curve from light to grey level. Any curve that is not straight moves the centroid of a \
                 lopsided beam a little. Leave it at the camera's default unless the camera offers a linear \
                 setting."
            }
            Self::Brightness => {
                "An offset added to every grey level. The finder fits the background, so it has little effect."
            }
            Self::Contrast => "Scales the grey levels about the middle. Too high clips the beam's top.",
            Self::Saturation => "Colour only; the luma does not change.",
            Self::Hue => "Colour only; the luma does not change.",
            Self::Zoom => "Digital zoom crops and scales the image, which changes the µm per pixel. Keep it off.",
        }
    }
}

/// What the numbers of a control mean.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlUnit {
    /// Only auto/lock is available. Manual(0) locks the current value; no numeric value is known.
    ModeOnly,
    /// A number with no unit given.
    #[default]
    Raw,
    /// Exposure as log₂ seconds: −6 is 1/64 s (Windows).
    Log2Seconds,
    /// Exposure in steps of 100 µs (Linux `exposure_time_absolute`).
    HundredMicroseconds,
    /// Colour temperature, K.
    Kelvin,
    /// 0 off, 1 is 50 Hz, 2 is 60 Hz, 3 is auto (USB video class).
    PowerLineMenu,
}

/// A control as the camera reports it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ControlInfo {
    pub id: ControlId,
    pub min: i64,
    pub max: i64,
    pub step: i64,
    pub default: i64,
    pub value: i64,
    /// `Some(on)` when the control has an automatic mode, and whether it is on now.
    pub auto: Option<bool>,
    #[serde(default)]
    pub unit: ControlUnit,
}

impl ControlInfo {
    /// `v` inside the range and on a step.
    pub fn snap(&self, v: i64) -> i64 {
        let step = self.step.max(1);
        let v = v.clamp(self.min, self.max);
        let snapped = self.min + ((v - self.min) as f64 / step as f64).round() as i64 * step;
        if snapped > self.max { snapped - step } else { snapped }
    }

    /// Exposure time in seconds for the value `v`, when the unit says.
    pub fn exposure_s(&self, v: i64) -> Option<f64> {
        match self.unit {
            ControlUnit::Log2Seconds => Some(2f64.powi(v as i32)),
            ControlUnit::HundredMicroseconds => Some(v as f64 * 1e-4),
            _ => None,
        }
    }

    /// The value for an exposure `factor` times as long as `v`.
    fn exposure_times(&self, v: i64, factor: f64) -> i64 {
        let next = match self.unit {
            ControlUnit::Log2Seconds => v + factor.log2().round() as i64,
            _ => (v as f64 * factor).round() as i64,
        };
        self.snap(next)
    }
}

/// Automatic, or held at a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlMode {
    Auto,
    Manual(i64),
}

/// One control to set when the camera opens. A control not listed is left as the camera has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlSetting {
    pub id: ControlId,
    pub mode: ControlMode,
}

/// The mains frequency where the measurement is made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mains {
    #[default]
    Hz50,
    Hz60,
}

impl Mains {
    pub fn hz(self) -> f64 {
        match self {
            Self::Hz50 => 50.0,
            Self::Hz60 => 60.0,
        }
    }

    /// Lights flicker at twice the mains frequency.
    pub fn flicker_period_s(self) -> f64 {
        0.5 / self.hz()
    }

    /// The value of a USB video class power-line control.
    pub fn menu_value(self) -> i64 {
        match self {
            Self::Hz50 => 1,
            Self::Hz60 => 2,
        }
    }
}

/// Which camera to open, in which mode, and how to set it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraSettings {
    /// The camera's name. `None` opens the first one.
    pub device: Option<String>,
    /// `None` lets [`pick_mode`] choose.
    pub mode: Option<VideoMode>,
    pub mains: Mains,
    pub controls: Vec<ControlSetting>,
}

impl CameraSettings {
    pub fn get(&self, id: ControlId) -> Option<ControlMode> {
        self.controls.iter().find(|c| c.id == id).map(|c| c.mode)
    }

    pub fn set(&mut self, id: ControlId, mode: ControlMode) {
        match self.controls.iter_mut().find(|c| c.id == id) {
            Some(c) => c.mode = mode,
            None => {
                self.controls.push(ControlSetting { id, mode });
                self.controls.sort_by_key(|c| c.id);
            }
        }
    }

    /// Leave the control as the camera has it.
    pub fn clear(&mut self, id: ControlId) {
        self.controls.retain(|c| c.id != id);
    }
}

/// The settings that lock the camera for measuring, from the controls it reports. Exposure, white
/// balance and focus are held where they are now; gain and sharpness go to the minimum,
/// backlight compensation off, the power line filter to `mains`. Gamma, brightness, contrast
/// and the colour controls are left alone.
pub fn for_measuring(infos: &[ControlInfo], mains: Mains) -> Vec<ControlSetting> {
    let mut out: Vec<ControlSetting> = infos
        .iter()
        .filter_map(|c| {
            let v = match c.id {
                ControlId::Exposure | ControlId::WhiteBalance | ControlId::Focus => c.value,
                ControlId::Gain | ControlId::Sharpness | ControlId::BacklightCompensation => c.min,
                ControlId::Zoom => c.default.min(c.value),
                ControlId::PowerLineFrequency => match c.unit {
                    ControlUnit::PowerLineMenu => mains.menu_value(),
                    _ => return None,
                },
                _ => return None,
            };
            Some(ControlSetting { id: c.id, mode: ControlMode::Manual(c.snap(v)) })
        })
        .collect();
    out.sort_by_key(|c| c.id);
    out
}

/// Brightest grey level the beam's peak should reach, leaving room below white.
pub const PEAK_HIGH: f64 = 230.0;
/// Dimmest peak worth having; below it the noise counts for more.
pub const PEAK_LOW: f64 = 120.0;
/// Where a change of exposure aims.
pub const PEAK_TARGET: f64 = 180.0;

/// The next exposure value to bring the beam's peak (above the background) between
/// [`PEAK_LOW`] and [`PEAK_HIGH`], or `None` when it is there already or the control can go no
/// further. A clipped beam halves the exposure, because its real peak is not known.
pub fn next_exposure(info: &ControlInfo, current: i64, peak: f64, clipped: bool) -> Option<i64> {
    let factor = if clipped || peak >= PEAK_HIGH {
        (PEAK_TARGET / peak.max(PEAK_HIGH)).min(0.5)
    } else if peak < PEAK_LOW {
        PEAK_TARGET / peak.max(1.0)
    } else {
        return None;
    };
    let mut next = info.exposure_times(current, factor);
    if next == current {
        // Rounding kept it where it was; take one step the right way.
        let step = info.step.max(1);
        next = info.snap(if factor < 1.0 { current - step } else { current + step });
    }
    (next != current).then_some(next)
}

/// How a mode smaller than the sensor is made from it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeScaling {
    /// The whole sensor is scaled down: one frame pixel covers more than one sensor pixel.
    #[default]
    Scaled,
    /// A window of the sensor is read: one frame pixel is one sensor pixel.
    Cropped,
}

/// The image sensor, from its data sheet. It gives the scale before a gauge block checks it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sensor {
    pub name: String,
    /// Centre-to-centre distance of the sensor's pixels, µm.
    pub pixel_pitch_um: f64,
    /// Active pixels at full resolution.
    pub width_px: u32,
    pub height_px: u32,
    pub scaling: ModeScaling,
}

impl Default for Sensor {
    /// A common 1080p webcam sensor with 3 µm pixels.
    fn default() -> Self {
        Self { name: String::new(), pixel_pitch_um: 3.0, width_px: 1920, height_px: 1080, scaling: ModeScaling::Scaled }
    }
}

impl Sensor {
    pub fn width_mm(&self) -> f64 {
        self.pixel_pitch_um * f64::from(self.width_px) / 1000.0
    }

    pub fn height_mm(&self) -> f64 {
        self.pixel_pitch_um * f64::from(self.height_px) / 1000.0
    }

    /// Size of one frame pixel across the line, µm, for frames `width` × `height`.
    pub fn um_per_px(&self, width: u32, height: u32, axis: LineAxis) -> f64 {
        let (frame, full) = match axis {
            LineAxis::Vertical => (width, self.width_px),
            LineAxis::Horizontal => (height, self.height_px),
        };
        match self.scaling {
            ModeScaling::Cropped => self.pixel_pitch_um,
            ModeScaling::Scaled => self.pixel_pitch_um * f64::from(full) / f64::from(frame.max(1)),
        }
    }

    /// The scale from the data sheet, for frames `width` × `height`.
    pub fn scale(&self, width: u32, height: u32, axis: LineAxis) -> Scale {
        Scale::from_pixel_pitch(self.um_per_px(width, height, axis))
    }

    pub fn is_valid(&self) -> bool {
        self.pixel_pitch_um.is_finite() && self.pixel_pitch_um > 0.0 && self.width_px > 0 && self.height_px > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exposure_log2(value: i64) -> ControlInfo {
        ControlInfo {
            id: ControlId::Exposure,
            min: -13,
            max: -1,
            step: 1,
            default: -6,
            value,
            auto: Some(true),
            unit: ControlUnit::Log2Seconds,
        }
    }

    fn plain(id: ControlId, min: i64, max: i64, value: i64) -> ControlInfo {
        ControlInfo { id, min, max, step: 1, default: value, value, auto: None, unit: ControlUnit::Raw }
    }

    #[test]
    fn measuring_locks_the_automatics() {
        let infos = [
            plain(ControlId::Contrast, 0, 100, 50),
            plain(ControlId::Gain, 0, 100, 37),
            ControlInfo {
                auto: Some(true),
                unit: ControlUnit::Kelvin,
                ..plain(ControlId::WhiteBalance, 2800, 6500, 4600)
            },
            exposure_log2(-6),
            plain(ControlId::Sharpness, 0, 7, 3),
            ControlInfo { unit: ControlUnit::PowerLineMenu, ..plain(ControlId::PowerLineFrequency, 0, 2, 0) },
        ];
        let s = for_measuring(&infos, Mains::Hz60);
        let mode = |id| s.iter().find(|c| c.id == id).map(|c| c.mode);
        assert_eq!(mode(ControlId::Exposure), Some(ControlMode::Manual(-6)));
        assert_eq!(mode(ControlId::WhiteBalance), Some(ControlMode::Manual(4600)));
        assert_eq!(mode(ControlId::Gain), Some(ControlMode::Manual(0)));
        assert_eq!(mode(ControlId::Sharpness), Some(ControlMode::Manual(0)));
        assert_eq!(mode(ControlId::PowerLineFrequency), Some(ControlMode::Manual(2)));
        assert_eq!(mode(ControlId::Contrast), None);
        assert!(s.windows(2).all(|w| w[0].id < w[1].id));
    }

    #[test]
    fn exposure_units_and_steps() {
        let e = exposure_log2(-6);
        assert_eq!(e.exposure_s(-6), Some(1.0 / 64.0));
        let linux = ControlInfo { unit: ControlUnit::HundredMicroseconds, min: 1, max: 5000, ..e };
        assert!((linux.exposure_s(100).unwrap() - 0.01).abs() < 1e-12);
        assert_eq!(plain(ControlId::Gain, 0, 10, 0).exposure_s(3), None);

        // Clipped: half as long, one step of log2.
        assert_eq!(next_exposure(&e, -6, 255.0, true), Some(-7));
        // Dim: 180/50 = 3.6, log2 = 1.85, two steps.
        assert_eq!(next_exposure(&e, -6, 50.0, false), Some(-4));
        assert_eq!(next_exposure(&e, -6, 170.0, false), None);
        // At the end of the range there is nowhere to go.
        assert_eq!(next_exposure(&e, -13, 255.0, true), None);
        // Linear: 100 × 180/60 = 300.
        assert_eq!(next_exposure(&linux, 100, 60.0, false), Some(300));
        // A small change that rounds to nothing still takes a step.
        assert_eq!(next_exposure(&linux, 1, 119.0, false), Some(2));
    }

    #[test]
    fn mode_only_exposure_locks_without_claiming_a_duration_or_numeric_step() {
        let c = ControlInfo {
            id: ControlId::Exposure,
            min: 0,
            max: 0,
            step: 1,
            default: 0,
            value: 0,
            auto: Some(true),
            unit: ControlUnit::ModeOnly,
        };
        assert_eq!(
            for_measuring(&[c], Mains::Hz50),
            [ControlSetting { id: ControlId::Exposure, mode: ControlMode::Manual(0) }]
        );
        assert_eq!(c.exposure_s(0), None);
        assert_eq!(next_exposure(&c, 0, 255.0, true), None);
        assert_eq!(next_exposure(&c, 0, 10.0, false), None);
    }

    #[test]
    fn snap_keeps_the_range_and_step() {
        let c = ControlInfo { step: 5, ..plain(ControlId::Gain, 2, 20, 2) };
        assert_eq!(c.snap(9), 7);
        assert_eq!(c.snap(10), 12);
        assert_eq!(c.snap(99), 17);
        assert_eq!(c.snap(-5), 2);
    }

    #[test]
    fn mode_choice() {
        let m = |width, height, fps, format| VideoMode { width, height, fps, format };
        let modes = [
            m(1920, 1080, 30.0, PixelFormat::Mjpeg),
            m(1920, 1080, 5.0, PixelFormat::Nv12),
            m(1280, 720, 30.0, PixelFormat::Nv12),
            m(1920, 1080, 2.0, PixelFormat::Nv12),
        ];
        assert_eq!(pick_mode(&modes, None), Some(modes[1]));
        assert_eq!(pick_mode(&modes, Some(&modes[2])), Some(modes[2]));
        let missing = m(640, 480, 30.0, PixelFormat::Yuyv);
        assert_eq!(pick_mode(&modes, Some(&missing)), Some(modes[1]));
        assert_eq!(pick_mode(&[], None), None);
    }

    #[test]
    fn sensor_scale_follows_the_mode() {
        let s = Sensor::default();
        assert!((s.width_mm() - 5.76).abs() < 1e-12);
        assert_eq!(s.um_per_px(1920, 1080, LineAxis::Vertical), 3.0);
        assert_eq!(s.um_per_px(1280, 720, LineAxis::Vertical), 4.5);
        assert_eq!(s.um_per_px(1280, 720, LineAxis::Horizontal), 4.5);
        let cropped = Sensor { scaling: ModeScaling::Cropped, ..s };
        assert_eq!(cropped.um_per_px(1280, 720, LineAxis::Vertical), 3.0);
        assert_eq!(cropped.scale(640, 480, LineAxis::Vertical).um_per_px, 3.0);
    }

    #[test]
    fn settings_edit_and_round_trip() {
        let mut c = CameraSettings::default();
        c.set(ControlId::Gain, ControlMode::Manual(0));
        c.set(ControlId::Exposure, ControlMode::Auto);
        c.set(ControlId::Exposure, ControlMode::Manual(-7));
        assert_eq!(c.controls.len(), 2);
        assert_eq!(c.controls[0].id, ControlId::Exposure);
        assert_eq!(c.get(ControlId::Exposure), Some(ControlMode::Manual(-7)));
        c.mode = Some(VideoMode { width: 1920, height: 1080, fps: 30.0, format: PixelFormat::Nv12 });
        let text = serde_json::to_string(&c).unwrap();
        assert!(text.contains(r#"{"id":"exposure","mode":{"manual":-7}}"#), "{text}");
        assert_eq!(serde_json::from_str::<CameraSettings>(&text).unwrap(), c);
        c.clear(ControlId::Gain);
        assert_eq!(c.get(ControlId::Gain), None);
        assert_eq!(Mains::Hz50.flicker_period_s(), 0.01);
    }
}
