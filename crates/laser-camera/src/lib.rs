//! Cameras for the laser level webcam.
//!
//! A [`FrameSource`] gives the camera's luma plane as it comes off the sensor (NV12, YUYV or
//! grey, never decoded or colour mixed), stamped with the camera's own frame time. Its
//! controls are read and set through [`laser_core::camera`]'s types, and every setting is read
//! back, because some cameras accept a manual setting and stay on automatic (see [`apply`]).
//!
//! Backends:
//! - [`synthetic`]: a simulated sensor with a laser line, warm-up drift and real-looking
//!   controls. Pure Rust; runs anywhere, the browser included.
//! - `mf` (Windows): Media Foundation for the frames, with the DirectShow camera control and
//!   video proc amp interfaces and the kernel-streaming power-line property for the controls.
//! - `v4l2` (Linux): Video4Linux2, memory-mapped streaming and the node's controls.
//! - `avf` (macOS): AVFoundation for the frames. macOS gives no camera controls.
//! - `web` (the browser): `getUserMedia`, opened with the asynchronous `web::open`.
//!
//! A source is used on the thread that opened it; [`capture`] runs one on its own thread.

use std::fmt;

use laser_core::camera::{CameraSettings, ControlId, ControlInfo, ControlMode, ControlSetting, VideoMode};
use laser_core::frame::{FrameError, Luma};

#[cfg(target_os = "macos")]
mod avf;
mod browser;
#[cfg(not(target_arch = "wasm32"))]
pub mod capture;
#[cfg(windows)]
mod mf;
pub mod synthetic;
#[cfg(windows)]
mod usb;
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
mod v4l2;
#[cfg(target_arch = "wasm32")]
pub mod web;

/// One frame's luma plane, with no row padding.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
    /// When the frame was exposed, s, by the camera's clock, from the first frame.
    pub t_s: f64,
    /// Frame number from 0, counted in frame times. A gap is frame times with no frame: lost, or
    /// the camera slowing down because an automatic exposure grew past the frame period.
    pub seq: u64,
}

impl Frame {
    pub fn luma(&self) -> Result<Luma<'_>, FrameError> {
        Luma::new(self.width, self.height, self.width, &self.data)
    }
}

/// A camera that can be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    /// What the backend opens it by; stable while it stays plugged in.
    pub id: String,
    pub backend: Backend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Synthetic,
    MediaFoundation,
    /// Video4Linux2 (Linux); the id is the `/dev/video*` node.
    V4l2,
    /// AVFoundation (macOS); the id is the device's unique ID.
    AvFoundation,
    /// The browser's `getUserMedia`; opened only through `web::open`.
    Browser,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CameraError {
    /// No camera by that name.
    NotFound(String),
    /// The camera has no mode with a format this tool reads.
    NoMode,
    /// The camera has no such control.
    NoControl(ControlId),
    /// The camera was unplugged or stopped sending frames.
    Lost(String),
    /// The operating system refused, with its message.
    Os(String),
}

impl fmt::Display for CameraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(name) => write!(f, "no camera called {name:?}"),
            Self::NoMode => write!(f, "the camera offers no NV12, YUYV or grey mode"),
            Self::NoControl(id) => write!(f, "the camera has no {} control", id.label().to_lowercase()),
            Self::Lost(why) => write!(f, "the camera stopped: {why}"),
            Self::Os(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for CameraError {}

/// An open camera.
pub trait FrameSource {
    fn device(&self) -> &DeviceInfo;

    /// The modes the camera offers that this tool can read.
    fn modes(&self) -> &[VideoMode];

    /// The mode it is running in.
    fn mode(&self) -> VideoMode;

    /// The simulated block the line moves with. Only the synthetic camera has one.
    fn block(&self) -> Option<synthetic::Block> {
        None
    }

    /// What the camera itself says it sends, from its USB descriptors, if it is a USB camera.
    fn usb(&self) -> Option<&laser_core::uvc::UvcDevice> {
        None
    }

    /// The controls as they are now. Controls the camera lacks are left out.
    fn controls(&mut self) -> Vec<ControlInfo>;

    /// Set one control, and read it back.
    fn set_control(&mut self, setting: ControlSetting) -> Result<ControlInfo, CameraError>;

    /// The next frame. Blocks until it arrives.
    fn next_frame(&mut self) -> Result<Frame, CameraError>;

    /// A new frame if one has come, without waiting. A source that only blocks gives the next one.
    fn try_frame(&mut self) -> Result<Option<Frame>, CameraError> {
        self.next_frame().map(Some)
    }

    /// Settings the camera finished after [`FrameSource::set_control`] answered, as read back.
    /// Only the browser's camera has any.
    fn settled(&mut self) -> Vec<Applied> {
        Vec::new()
    }
}

/// The cameras on this machine (the browser's, in the browser), then the synthetic one.
pub fn devices() -> Vec<DeviceInfo> {
    #[allow(unused_mut)]
    let mut out: Vec<DeviceInfo> = Vec::new();
    #[cfg(windows)]
    out.extend(mf::devices().unwrap_or_default());
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    out.extend(v4l2::devices());
    #[cfg(target_os = "macos")]
    out.extend(avf::devices());
    #[cfg(target_arch = "wasm32")]
    out.extend(web::devices());
    out.push(synthetic::device());
    out
}

/// What the camera itself says it sends, from its USB descriptors. Works while it streams.
pub fn usb(device: &DeviceInfo) -> Result<laser_core::uvc::UvcDevice, CameraError> {
    match device.backend {
        #[cfg(windows)]
        Backend::MediaFoundation => usb::uvc(&device.id),
        #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
        Backend::V4l2 => v4l2::uvc(&device.id),
        _ => Err(CameraError::Os(format!("{} is not a USB camera", device.name))),
    }
}

/// Open the camera the settings name (the first one if they name none) in their mode (or the
/// best one), and apply their controls.
pub fn open(settings: &CameraSettings) -> Result<(Box<dyn FrameSource>, Vec<Applied>), CameraError> {
    let all = devices();
    let device = match &settings.device {
        Some(name) => all.into_iter().find(|d| &d.name == name).ok_or_else(|| CameraError::NotFound(name.clone()))?,
        None => all.into_iter().next().ok_or_else(|| CameraError::NotFound(String::new()))?,
    };
    let mut source: Box<dyn FrameSource> = match device.backend {
        Backend::Synthetic => {
            let mut cam = synthetic::Synthetic::open(settings.mode);
            cam.block = Some(synthetic::Block::default());
            Box::new(cam)
        }
        #[cfg(windows)]
        Backend::MediaFoundation => Box::new(mf::MfCamera::open(&device, settings.mode.as_ref())?),
        #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
        Backend::V4l2 => Box::new(v4l2::V4l2Camera::open(&device, settings.mode.as_ref())?),
        #[cfg(target_os = "macos")]
        Backend::AvFoundation => Box::new(avf::AvfCamera::open(&device, settings.mode.as_ref())?),
        #[allow(unreachable_patterns)]
        Backend::MediaFoundation | Backend::V4l2 | Backend::AvFoundation => {
            return Err(CameraError::NotFound(device.name));
        }
        Backend::Browser => return Err(CameraError::Os(format!("{} opens only through web::open", device.name))),
    };
    let applied = apply(source.as_mut(), &settings.controls);
    Ok((source, applied))
}

/// The luma plane of one of the camera's JPEGs, decoded here (the Y samples as coded, no colour
/// conversion). Empty if it does not decode to `w`×`h`.
#[cfg(any(windows, all(target_os = "linux", target_pointer_width = "64")))]
fn jpeg_luma(bytes: &[u8], w: usize, h: usize) -> Vec<u8> {
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::Luma);
    let mut d = zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(bytes), options);
    match d.decode() {
        Ok(y) if y.len() == w * h => y,
        _ => Vec::new(),
    }
}

/// Frame times and numbers from the camera's own timestamps.
#[cfg(any(target_os = "macos", all(target_os = "linux", target_pointer_width = "64")))]
#[derive(Default)]
struct Clock {
    first: Option<f64>,
    last: f64,
    seq: u64,
}

#[cfg(any(target_os = "macos", all(target_os = "linux", target_pointer_width = "64")))]
impl Clock {
    fn started(&self) -> bool {
        self.first.is_some()
    }

    /// The time from the first frame, s, and the frame number ([`Frame::seq`]: frame times with
    /// no frame count too) of a frame stamped `t` s at `fps`.
    fn stamp(&mut self, t: f64, fps: f64) -> (f64, u64) {
        let first = *self.first.get_or_insert(t);
        if t != first {
            self.seq += ((t - self.last) * fps.max(1.0)).round().max(1.0) as u64;
        }
        self.last = t;
        (t - first, self.seq)
    }
}

/// What happened to one setting.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    pub setting: ControlSetting,
    /// The control as read back, or why it could not be set.
    pub result: Result<ControlInfo, CameraError>,
}

impl Applied {
    /// The camera did what it was asked: a manual value is off automatic and at the value
    /// (or the nearest step to it); automatic is on.
    pub fn held(&self) -> bool {
        let Ok(info) = &self.result else { return false };
        match self.setting.mode {
            ControlMode::Auto => info.auto == Some(true),
            ControlMode::Manual(v) => info.auto != Some(true) && info.value == info.snap(v),
        }
    }
}

/// Set each control and read it back. A camera that keeps a control on automatic after a
/// manual setting shows as not [`Applied::held`].
pub fn apply(source: &mut dyn FrameSource, settings: &[ControlSetting]) -> Vec<Applied> {
    settings.iter().map(|&setting| Applied { setting, result: source.set_control(setting) }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use laser_core::camera::{Mains, for_measuring};

    #[test]
    fn synthetic_opens_and_locks() {
        let mut settings = CameraSettings { device: Some(synthetic::NAME.into()), ..CameraSettings::default() };
        let (mut cam, applied) = open(&settings).unwrap();
        assert!(applied.is_empty());
        let infos = cam.controls();
        settings.controls = for_measuring(&infos, Mains::Hz50);
        let applied = apply(cam.as_mut(), &settings.controls);
        assert!(!applied.is_empty());
        assert!(applied.iter().all(Applied::held), "{applied:?}");
        let f = cam.next_frame().unwrap();
        assert_eq!((f.width, f.height), (cam.mode().width as usize, cam.mode().height as usize));
        assert!(f.luma().is_ok());
        let unknown = CameraSettings { device: Some("no such camera".into()), ..CameraSettings::default() };
        assert!(matches!(open(&unknown), Err(CameraError::NotFound(_))));
    }
}
