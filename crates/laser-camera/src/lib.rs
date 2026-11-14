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
//!
//! A source is used on the thread that opened it; [`capture`] runs one on its own thread.

use std::fmt;

use laser_core::camera::{CameraSettings, ControlId, ControlInfo, ControlMode, ControlSetting, VideoMode};
use laser_core::frame::{FrameError, Luma};

#[cfg(not(target_arch = "wasm32"))]
pub mod capture;
#[cfg(windows)]
mod mf;
pub mod synthetic;
#[cfg(windows)]
mod usb;

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
}

/// The cameras on this machine, then the synthetic one.
pub fn devices() -> Vec<DeviceInfo> {
    #[allow(unused_mut)]
    let mut out: Vec<DeviceInfo> = Vec::new();
    #[cfg(windows)]
    out.extend(mf::devices().unwrap_or_default());
    out.push(synthetic::device());
    out
}

/// What the camera itself says it sends, from its USB descriptors. Works while it streams.
pub fn usb(device: &DeviceInfo) -> Result<laser_core::uvc::UvcDevice, CameraError> {
    match device.backend {
        #[cfg(windows)]
        Backend::MediaFoundation => usb::uvc(&device.id),
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
        Backend::Synthetic => Box::new(synthetic::Synthetic::open(settings.mode)),
        #[cfg(windows)]
        Backend::MediaFoundation => Box::new(mf::MfCamera::open(&device, settings.mode.as_ref())?),
        #[cfg(not(windows))]
        Backend::MediaFoundation => return Err(CameraError::NotFound(device.name)),
    };
    let applied = apply(source.as_mut(), &settings.controls);
    Ok((source, applied))
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
