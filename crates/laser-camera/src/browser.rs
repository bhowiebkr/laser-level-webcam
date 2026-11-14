//! The browser camera's rules that need no browser: the controls and their ranges, the
//! constraints sent to open the camera and to set its controls, which failed opens are worth
//! asking again for less, and the error text. `web` uses them; they live here so they are tested
//! on any machine.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use laser_core::camera::{ControlId, ControlInfo, ControlMode, ControlSetting, ControlUnit, PixelFormat, VideoMode};

use crate::CameraError;

/// The controls the Media Capture Image spec has, with their mode and value names, and the unit
/// of the value. Browsers have no gain or mains filter.
pub const CONTROLS: [(ControlId, Option<&str>, &str, ControlUnit); 8] = [
    (ControlId::Exposure, Some("exposureMode"), "exposureTime", ControlUnit::HundredMicroseconds),
    (ControlId::WhiteBalance, Some("whiteBalanceMode"), "colorTemperature", ControlUnit::Kelvin),
    (ControlId::Brightness, None, "brightness", ControlUnit::Raw),
    (ControlId::Contrast, None, "contrast", ControlUnit::Raw),
    (ControlId::Saturation, None, "saturation", ControlUnit::Raw),
    (ControlId::Sharpness, None, "sharpness", ControlUnit::Raw),
    (ControlId::Focus, Some("focusMode"), "focusDistance", ControlUnit::Raw),
    (ControlId::Zoom, None, "zoom", ControlUnit::Raw),
];

/// Sizes offered in the mode list, when the camera's range takes them.
pub const SIZES: [(u32, u32); 6] = [(3840, 2160), (2592, 1944), (1920, 1080), (1280, 960), (1280, 720), (640, 480)];

/// The size asked for when the settings name none.
pub const DEFAULT_SIZE: (u32, u32) = (1920, 1080);

/// The mode and value names of a control.
pub fn keys(id: ControlId) -> Option<(Option<&'static str>, &'static str, ControlUnit)> {
    CONTROLS.iter().find(|c| c.0 == id).map(|&(_, mode, value, unit)| (mode, value, unit))
}

/// The whole numbers inside the browser's range. The browser gives some ends as fractions (the
/// exposure, made from the camera's powers of two) and refuses a value past them, so the ends
/// are rounded inwards.
pub fn whole_range(min: f64, max: f64) -> Option<(i64, i64)> {
    if !(min.is_finite() && max.is_finite()) {
        return None;
    }
    let (lo, hi) = (min.ceil(), max.floor());
    (lo <= hi).then_some((lo as i64, hi as i64))
}

/// The exposure range in the camera's own unit, log2 seconds, when the browser gives it as whole
/// powers of two seconds in 100 µs (`10000 * 2^k`, from 1.22 for `2^-13` s). Chromium on Windows
/// does: it sets the camera in whole powers of two and reads back only those, so a time between
/// them is never held. `None` for any other range.
pub fn log2_exposure(min: f64, max: f64) -> Option<(i64, i64)> {
    let power = |v: f64| {
        let k = (v / 1e4).log2();
        (k.is_finite() && (k - k.round()).abs() < 1e-9).then_some(k.round() as i64)
    };
    let (lo, hi) = (power(min)?, power(max)?);
    (lo < hi).then_some((lo, hi))
}

/// A control as the app sees it, from the browser's range, step and value for it.
pub fn control_info(
    id: ControlId,
    unit: ControlUnit,
    (lo, hi): (f64, f64),
    step: Option<f64>,
    now: Option<f64>,
    auto: Option<bool>,
) -> Option<ControlInfo> {
    if id == ControlId::Exposure
        && let Some((min, max)) = log2_exposure(lo, hi)
    {
        let value = now.map_or(min, |v| (v / 1e4).log2().round() as i64).clamp(min, max);
        return Some(ControlInfo {
            id,
            min,
            max,
            step: 1,
            default: value,
            value,
            auto,
            unit: ControlUnit::Log2Seconds,
        });
    }
    let (min, max) = whole_range(lo, hi)?;
    let step = step.filter(|s| *s > 0.0).map_or(1, |s| (s.round() as i64).max(1));
    let value = (now.unwrap_or(lo).round() as i64).clamp(min, max);
    Some(ControlInfo { id, min, max, step, default: value, value, auto, unit })
}

/// Whether a control is on automatic, from the modes the camera offers and the one it is in.
/// `None` when it cannot be switched between automatic and manual.
pub fn auto_state(offered: &[String], now: Option<&str>) -> Option<bool> {
    let has_auto = offered.iter().any(|m| m == "continuous" || m == "single-shot");
    let has_manual = offered.iter().any(|m| m == "manual");
    (has_auto && has_manual).then_some(now != Some("manual"))
}

/// One value in a constraint set.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(&'static str),
    Number(f64),
}

/// How much a request asks of the camera. Some cameras do not start on the first, so an open
/// asks for less until one starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// The size and rate, from the camera's own sizes only: a size the browser scales to
    /// averages the beam's pixels.
    Native,
    /// The size and rate, scaled by the browser if need be.
    Size,
    /// The camera, in whatever it starts in.
    Device,
}

impl Ask {
    /// In the order an open tries them.
    pub const ALL: [Ask; 3] = [Ask::Native, Ask::Size, Ask::Device];
}

/// The size and rate constraints of a request, besides the device.
pub fn size_fields(mode: Option<&VideoMode>, ask: Ask) -> Vec<(&'static str, Value)> {
    if ask == Ask::Device {
        return Vec::new();
    }
    let (w, h) = mode.map_or(DEFAULT_SIZE, |m| (m.width, m.height));
    let mut out = vec![("width", Value::Number(f64::from(w))), ("height", Value::Number(f64::from(h)))];
    if let Some(m) = mode {
        out.push(("frameRate", Value::Number(m.fps)));
    }
    if ask == Ask::Native {
        out.push(("resizeMode", Value::Text("none")));
    }
    out
}

/// Whether an open that failed with this browser error is worth asking again for less. The
/// user's refusal, or no camera at all, stays so.
pub fn ask_less_after(error: &str) -> bool {
    matches!(error, "NotReadableError" | "OverconstrainedError" | "AbortError")
}

/// Ask for a setting: it takes the place of an earlier one for the same control.
pub fn want(wanted: &mut Vec<ControlSetting>, setting: ControlSetting) {
    wanted.retain(|s| s.id != setting.id);
    wanted.push(setting);
}

/// The camera refused a setting: let it go, so it does not fail the next set. A later setting
/// for the same control stays.
pub fn refused(wanted: &mut Vec<ControlSetting>, setting: ControlSetting) {
    wanted.retain(|s| *s != setting);
}

/// The constraint fields for the settings asked so far. `range` gives the browser's range of a
/// value name; each value is kept inside it, as one value past it fails the whole set.
pub fn control_fields(
    wanted: &[ControlSetting],
    range: impl Fn(&str) -> Option<(f64, f64)>,
) -> Vec<(&'static str, Value)> {
    let mut out = Vec::new();
    for s in wanted {
        let Some((mode_key, value_key, _)) = keys(s.id) else { continue };
        match (s.mode, mode_key) {
            (ControlMode::Auto, Some(key)) => out.push((key, Value::Text("continuous"))),
            (ControlMode::Auto, None) => {}
            (ControlMode::Manual(v), key) => {
                if let Some(key) = key {
                    out.push((key, Value::Text("manual")));
                }
                let range = range(value_key);
                let n = match range {
                    // Shown in log2 seconds: back to 100 µs.
                    Some((lo, hi)) if s.id == ControlId::Exposure && log2_exposure(lo, hi).is_some() => {
                        1e4 * 2f64.powi(v.clamp(-64, 64) as i32)
                    }
                    _ => v as f64,
                };
                out.push((value_key, Value::Number(range.map_or(n, |(lo, hi)| n.clamp(lo, hi)))));
            }
        }
    }
    out
}

/// The usual sizes inside the camera's largest size, at its highest frame rate up to 30 fps,
/// and the mode it is in. The browser does not list a camera's own sizes, so `missing` has the
/// ones asked for that it gave another size for.
pub fn offered_modes(
    max_size: Option<(f64, f64)>,
    max_fps: Option<f64>,
    current: VideoMode,
    missing: &[(u32, u32)],
) -> Vec<VideoMode> {
    let Some((w_max, h_max)) = max_size else { return vec![current] };
    let fps = max_fps.map_or(current.fps, |f| f.min(30.0));
    let mut out: Vec<VideoMode> = SIZES
        .iter()
        .filter(|&&(w, h)| f64::from(w) <= w_max && f64::from(h) <= h_max && !missing.contains(&(w, h)))
        .map(|&(width, height)| VideoMode { width, height, fps, format: PixelFormat::Rgba })
        .collect();
    if !out.iter().any(|m| (m.width, m.height) == (current.width, current.height)) {
        out.insert(0, current);
    }
    out
}

/// What to tell the user about a browser error (`name` such as `NotReadableError`, with the
/// browser's message) while doing `what`.
pub fn error(what: &str, name: &str, message: &str) -> CameraError {
    let text = match name {
        "NotAllowedError" => {
            format!("{what}: the camera was not allowed. Allow it in the browser, or pick the synthetic camera")
        }
        "NotFoundError" => format!("{what}: the browser found no camera ({message})"),
        "OverconstrainedError" => format!("{what}: the camera does not take it ({message})"),
        // The browser says this for a camera another program holds, and for one that does not
        // start at all.
        "NotReadableError" | "AbortError" => format!(
            "{what}: the camera did not start ({message}). Close any other program that uses it, or \
             disconnect it and connect it again"
        ),
        _ if message.is_empty() => format!("{what}: {name}"),
        _ => format!("{what}: {message}"),
    };
    CameraError::Os(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manual(id: ControlId, v: i64) -> ControlSetting {
        ControlSetting { id, mode: ControlMode::Manual(v) }
    }

    fn auto(id: ControlId) -> ControlSetting {
        ControlSetting { id, mode: ControlMode::Auto }
    }

    #[test]
    fn ranges_round_inwards() {
        // Chromium on Windows gives the exposure as powers of two in 100 µs: 2^-13 s is 1.22.
        assert_eq!(whole_range(1.220703125, 5000.0), Some((2, 5000)));
        assert_eq!(whole_range(-64.0, 64.0), Some((-64, 64)));
        assert_eq!(whole_range(0.4, 0.6), None);
        assert_eq!(whole_range(f64::NAN, 1.0), None);
        assert_eq!(whole_range(3.0, 3.0), Some((3, 3)));
    }

    #[test]
    fn exposure_in_powers_of_two_is_shown_in_log2_seconds() {
        // Chromium on Windows: 2^-13 s to 2^-1 s, in 100 µs.
        assert_eq!(log2_exposure(1.220703125, 5000.0), Some((-13, -1)));
        assert_eq!(log2_exposure(10.0, 100.0), None);
        assert_eq!(log2_exposure(1.0, 5000.0), None);
        assert_eq!(log2_exposure(5000.0, 5000.0), None);
        assert_eq!(log2_exposure(0.0, 5000.0), None);
        let info = |now| {
            control_info(
                ControlId::Exposure,
                ControlUnit::HundredMicroseconds,
                (1.220703125, 5000.0),
                Some(1.220703125),
                now,
                Some(false),
            )
            .unwrap()
        };
        let e = info(Some(19.53125));
        assert_eq!((e.min, e.max, e.step, e.value, e.unit), (-13, -1, 1, -9, ControlUnit::Log2Seconds));
        assert_eq!(info(Some(0.0)).value, -13);
        assert_eq!(info(None).value, -13);
        // Each value goes back as the time the browser reads back, so it is held.
        let range = |_: &str| Some((1.220703125, 5000.0));
        for v in -13..=-1 {
            let sent = control_fields(&[manual(ControlId::Exposure, v)], range);
            let Value::Number(t) = sent[1].1 else { panic!("{sent:?}") };
            assert_eq!(info(Some(t)).value, v);
        }
        // Any other range stays in its own unit, on whole numbers.
        let linear = control_info(
            ControlId::Exposure,
            ControlUnit::HundredMicroseconds,
            (10.0, 100.0),
            Some(5.0),
            Some(52.4),
            None,
        );
        let l = linear.unwrap();
        assert_eq!((l.min, l.max, l.step, l.value, l.unit), (10, 100, 5, 52, ControlUnit::HundredMicroseconds));
        let wb = control_info(
            ControlId::WhiteBalance,
            ControlUnit::Kelvin,
            (2800.0, 6500.0),
            None,
            Some(4600.0),
            Some(true),
        );
        assert_eq!(wb.map(|w| (w.step, w.value, w.auto)), Some((1, 4600, Some(true))));
        assert_eq!(control_info(ControlId::Zoom, ControlUnit::Raw, (0.4, 0.6), None, None, None), None);
    }

    #[test]
    fn auto_needs_both_modes() {
        let modes = |m: &[&str]| m.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(auto_state(&modes(&["manual", "continuous"]), Some("continuous")), Some(true));
        assert_eq!(auto_state(&modes(&["manual", "continuous"]), Some("manual")), Some(false));
        assert_eq!(auto_state(&modes(&["manual", "single-shot"]), None), Some(true));
        assert_eq!(auto_state(&modes(&["continuous"]), Some("continuous")), None);
        assert_eq!(auto_state(&modes(&["manual"]), Some("manual")), None);
        assert_eq!(auto_state(&[], None), None);
    }

    #[test]
    fn opens_ask_for_less_each_time() {
        let mode = VideoMode { width: 1280, height: 720, fps: 30.0, format: PixelFormat::Rgba };
        let names = |f: Vec<(&'static str, Value)>| f.into_iter().map(|(k, _)| k).collect::<Vec<_>>();
        assert_eq!(names(size_fields(Some(&mode), Ask::Native)), ["width", "height", "frameRate", "resizeMode"]);
        assert_eq!(names(size_fields(Some(&mode), Ask::Size)), ["width", "height", "frameRate"]);
        assert!(size_fields(Some(&mode), Ask::Device).is_empty());
        // No mode: the default size, at the camera's own rate.
        assert_eq!(size_fields(None, Ask::Size), [("width", Value::Number(1920.0)), ("height", Value::Number(1080.0))]);
        assert_eq!(Ask::ALL, [Ask::Native, Ask::Size, Ask::Device]);
    }

    #[test]
    fn only_a_camera_that_did_not_start_is_asked_again() {
        for name in ["NotReadableError", "OverconstrainedError", "AbortError"] {
            assert!(ask_less_after(name), "{name}");
        }
        for name in ["NotAllowedError", "NotFoundError", "SecurityError", "TypeError", ""] {
            assert!(!ask_less_after(name), "{name}");
        }
    }

    #[test]
    fn a_setting_takes_the_place_of_the_last_for_its_control() {
        let mut wanted = Vec::new();
        want(&mut wanted, manual(ControlId::Exposure, 10));
        want(&mut wanted, auto(ControlId::WhiteBalance));
        want(&mut wanted, manual(ControlId::Exposure, 20));
        assert_eq!(wanted, [auto(ControlId::WhiteBalance), manual(ControlId::Exposure, 20)]);
    }

    #[test]
    fn a_refused_setting_does_not_block_the_next() {
        let mut wanted = Vec::new();
        want(&mut wanted, manual(ControlId::Exposure, 1));
        want(&mut wanted, manual(ControlId::WhiteBalance, 4600));
        refused(&mut wanted, manual(ControlId::Exposure, 1));
        assert_eq!(wanted, [manual(ControlId::WhiteBalance, 4600)]);
        // The answer to an earlier setting does not drop a later one for the same control.
        want(&mut wanted, manual(ControlId::Exposure, 5));
        refused(&mut wanted, manual(ControlId::Exposure, 1));
        assert_eq!(wanted, [manual(ControlId::WhiteBalance, 4600), manual(ControlId::Exposure, 5)]);
    }

    #[test]
    fn control_values_stay_inside_the_browsers_range() {
        let range = |key: &str| match key {
            "exposureTime" => Some((1.220703125, 5000.0)),
            "colorTemperature" => Some((2800.0, 6500.0)),
            _ => None,
        };
        let wanted = [
            // Log2 seconds: 2^-14 s is past the end, 2^-13 s is 1.22.
            manual(ControlId::Exposure, -14),
            manual(ControlId::WhiteBalance, 9000),
            manual(ControlId::Brightness, -3),
            auto(ControlId::Focus),
            // No mode to switch: automatic sends nothing.
            auto(ControlId::Zoom),
        ];
        assert_eq!(
            control_fields(&wanted, range),
            [
                ("exposureMode", Value::Text("manual")),
                ("exposureTime", Value::Number(1.220703125)),
                ("whiteBalanceMode", Value::Text("manual")),
                ("colorTemperature", Value::Number(6500.0)),
                ("brightness", Value::Number(-3.0)),
                ("focusMode", Value::Text("continuous")),
            ]
        );
    }

    #[test]
    fn modes_fit_the_camera() {
        let current = VideoMode { width: 1920, height: 1080, fps: 30.0, format: PixelFormat::Rgba };
        let sizes = |m: Vec<VideoMode>| m.into_iter().map(|m| (m.width, m.height)).collect::<Vec<_>>();
        assert_eq!(
            sizes(offered_modes(Some((1920.0, 1080.0)), Some(60.0), current, &[])),
            [(1920, 1080), (1280, 960), (1280, 720), (640, 480)]
        );
        assert!(offered_modes(Some((1920.0, 1080.0)), Some(60.0), current, &[]).iter().all(|m| m.fps == 30.0));
        // A size the camera did not give is no longer offered; the one it gave is.
        let instead = VideoMode { width: 1280, height: 720, ..current };
        assert_eq!(
            sizes(offered_modes(Some((1920.0, 1080.0)), None, instead, &[(1280, 960)])),
            [(1920, 1080), (1280, 720), (640, 480)]
        );
        // A size not in the list is kept, first.
        let odd = VideoMode { width: 1600, height: 1200, ..current };
        assert_eq!(sizes(offered_modes(Some((1600.0, 1200.0)), None, odd, &[]))[0], (1600, 1200));
        // No range: only the mode it is in.
        assert_eq!(offered_modes(None, None, current, &[]), [current]);
    }

    #[test]
    fn errors_say_what_happened() {
        let text = |name: &str, message: &str| error("opening", name, message).to_string();
        assert!(text("NotAllowedError", "").contains("not allowed"));
        let not_read = text("NotReadableError", "Could not start video source");
        assert!(not_read.contains("did not start (Could not start video source)"), "{not_read}");
        assert!(!not_read.contains("no camera called"));
        let over = error("setting a control", "OverconstrainedError", "exposureTime setting out of range").to_string();
        assert_eq!(over, "setting a control: the camera does not take it (exposureTime setting out of range)");
        assert_eq!(text("TypeError", "bad"), "opening: bad");
        assert_eq!(text("SomeError", ""), "opening: SomeError");
    }
}
