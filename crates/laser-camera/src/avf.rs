//! Mac cameras through AVFoundation.
//!
//! A camera is an `AVCaptureDevice` for video. The modes are its formats, each at the fastest and
//! slowest ends of its frame rates. Frames come from an `AVCaptureVideoDataOutput` as bi-planar
//! 4:2:0, so the luma is the first plane as it is: for a camera that sends YUV the luma is the
//! camera's own, and only an MJPEG mode is decoded (by macOS) on the way. Supported exposure,
//! focus and white-balance auto/lock modes are queried per device; numeric values are unavailable.
#![allow(unsafe_code)]

use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::time::Duration;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use laser_core::camera::{
    ControlId, ControlInfo, ControlMode, ControlSetting, ControlUnit, PixelFormat, VideoMode, pick_mode,
};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_av_foundation::{
    AVAuthorizationStatus, AVCaptureConnection, AVCaptureDevice, AVCaptureDeviceFormat, AVCaptureDeviceInput,
    AVCaptureExposureMode, AVCaptureFocusMode, AVCaptureOutput, AVCaptureSession, AVCaptureVideoDataOutput,
    AVCaptureVideoDataOutputSampleBufferDelegate, AVCaptureWhiteBalanceMode, AVMediaTypeVideo,
};
use objc2_core_media::{CMSampleBuffer, CMTime, CMVideoFormatDescriptionGetDimensions};
use objc2_core_video::{
    CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetHeightOfPlane,
    CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidthOfPlane, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress, kCVPixelBufferHeightKey, kCVPixelBufferPixelFormatTypeKey,
    kCVPixelBufferWidthKey, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
};
use objc2_foundation::{NSBundle, NSDictionary, NSNumber, NSString, ns_string};

use crate::{Backend, CameraError, Clock, DeviceInfo, Frame, FrameSource};

/// Frames waiting to be read. More than this and the newest are dropped, which the frame numbers show.
const QUEUE: usize = 3;

const fn fourcc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

/// What a format sends, as this tool names it. Formats it cannot read give `None`.
fn format_of(subtype: u32) -> Option<PixelFormat> {
    match subtype {
        x if x == fourcc(b"420v") || x == fourcc(b"420f") => Some(PixelFormat::Nv12),
        // YUY2 and UYVY: the luma is every other byte either way.
        x if x == fourcc(b"yuvs") || x == fourcc(b"2vuy") => Some(PixelFormat::Yuyv),
        x if x == fourcc(b"dmb1") || x == fourcc(b"jpeg") => Some(PixelFormat::Mjpeg),
        _ => None,
    }
}

fn video() -> &'static NSString {
    // SAFETY: AVMediaTypeVideo is a constant string AVFoundation always defines.
    unsafe { AVMediaTypeVideo }.expect("AVFoundation defines AVMediaTypeVideo")
}

fn seconds(t: CMTime) -> f64 {
    if t.timescale > 0 { t.value as f64 / f64::from(t.timescale) } else { 0.0 }
}

fn os(message: impl ToString) -> CameraError {
    CameraError::Os(message.to_string())
}

fn native<T>(f: impl FnOnce() -> Result<T, CameraError>) -> Result<T, CameraError> {
    objc2::exception::catch(std::panic::AssertUnwindSafe(f))
        .map_err(|e| os(format!("AVFoundation: {}", e.map_or_else(|| "unknown exception".into(), |e| e.to_string()))))?
}

struct ConfigurationLock(Retained<AVCaptureDevice>);
impl ConfigurationLock {
    unsafe fn new(camera: &Retained<AVCaptureDevice>) -> Result<Self, CameraError> {
        unsafe {
            camera.lockForConfiguration().map_err(os)?;
        }
        Ok(Self(camera.clone()))
    }
}
impl Drop for ConfigurationLock {
    fn drop(&mut self) {
        // SAFETY: this guard exists only after a successful lock on this thread.
        unsafe {
            self.0.unlockForConfiguration();
        }
    }
}

/// One mode, with the format and frame time that give it.
struct Offer {
    mode: VideoMode,
    format: Retained<AVCaptureDeviceFormat>,
    duration: CMTime,
}

fn offers(device: &AVCaptureDevice) -> Vec<Offer> {
    let mut out: Vec<Offer> = Vec::new();
    // SAFETY: plain property reads on a live device.
    for format in unsafe { device.formats() }.iter() {
        let desc = unsafe { format.formatDescription() };
        let Some(pixel) = format_of(unsafe { desc.media_sub_type() }) else {
            continue;
        };
        let dims = unsafe { CMVideoFormatDescriptionGetDimensions(&desc) };
        let (Ok(width), Ok(height)) = (u32::try_from(dims.width), u32::try_from(dims.height)) else {
            continue;
        };
        if width == 0 || height == 0 {
            continue;
        }
        for range in unsafe { format.videoSupportedFrameRateRanges() }.iter() {
            let ends = unsafe {
                [(range.maxFrameRate(), range.minFrameDuration()), (range.minFrameRate(), range.maxFrameDuration())]
            };
            for (fps, duration) in ends {
                if fps < 0.5 {
                    continue;
                }
                let mode = VideoMode { width, height, fps, format: pixel };
                let same = |o: &Offer| {
                    o.mode.width == width
                        && o.mode.height == height
                        && o.mode.format == pixel
                        && (o.mode.fps - fps).abs() < 0.5
                };
                if !out.iter().any(same) {
                    out.push(Offer { mode, format: format.clone(), duration });
                }
            }
        }
    }
    // Some devices also advertise NV12 aliases of their packed-YUV modes. These can
    // acknowledge a frame duration while continuing to deliver at another rate. Prefer
    // the equivalent packed-YUV source; the output still supplies NV12 luma in either case.
    let packed: Vec<_> = out.iter().filter(|o| o.mode.format == PixelFormat::Yuyv).map(|o| o.mode).collect();
    out.retain(|o| {
        o.mode.format != PixelFormat::Nv12
            || !packed
                .iter()
                .any(|m| m.width == o.mode.width && m.height == o.mode.height && (m.fps - o.mode.fps).abs() < 0.5)
    });
    out
}

fn video_devices() -> Vec<Retained<AVCaptureDevice>> {
    // SAFETY: a class method with a valid media type. It is deprecated for the discovery
    // session, which needs a list of device types per macOS version; this one lists them all.
    #[allow(deprecated)]
    let all = unsafe { AVCaptureDevice::devicesWithMediaType(video()) };
    all.iter().collect()
}

/// The cameras macOS lists, by name, with the ones this tool cannot read left out.
pub fn devices() -> Vec<DeviceInfo> {
    autoreleasepool(|_| {
        let mut out: Vec<DeviceInfo> = Vec::new();
        for d in video_devices() {
            if offers(&d).is_empty() {
                continue;
            }
            // SAFETY: plain property reads.
            let (mut name, id) = unsafe { (d.localizedName().to_string(), d.uniqueID().to_string()) };
            let base = name.clone();
            let mut n = 2;
            while out.iter().any(|o| o.name == name) {
                name = format!("{base} ({n})");
                n += 1;
            }
            out.push(DeviceInfo { name, id, backend: Backend::AvFoundation });
        }
        out
    })
}

/// Wait for macOS to say whether this app may use the camera, asking the user the first time.
fn allowed() -> Result<(), CameraError> {
    let denied = || {
        CameraError::Os(
            "macOS has not allowed this app the camera: turn it on in System Settings, Privacy & Security, Camera"
                .into(),
        )
    };
    // SAFETY: a class method with a valid media type.
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(video()) } {
        AVAuthorizationStatus::Authorized => Ok(()),
        AVAuthorizationStatus::NotDetermined => {
            if NSBundle::mainBundle().objectForInfoDictionaryKey(ns_string!("NSCameraUsageDescription")).is_none() {
                return Err(os(
                    "This executable must include NSCameraUsageDescription before requesting camera access",
                ));
            }
            let (tx, rx) = sync_channel::<bool>(1);
            let answer = RcBlock::new(move |granted: Bool| {
                let _ = tx.try_send(granted.as_bool());
            });
            // SAFETY: the block lives until macOS calls it; it only sends on a channel.
            unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(video(), &answer) };
            match rx.recv_timeout(Duration::from_secs(600)) {
                Ok(true) => Ok(()),
                _ => Err(denied()),
            }
        }
        _ => Err(denied()),
    }
}

/// A frame as it left the capture queue.
struct Shot {
    width: usize,
    height: usize,
    data: Vec<u8>,
    t_s: f64,
}

struct Ivars {
    tx: SyncSender<Shot>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Frames does not implement Drop.
    #[unsafe(super(NSObject))]
    #[name = "LaserLevelWebcamFrames"]
    #[ivars = Ivars]
    struct Frames;

    unsafe impl NSObjectProtocol for Frames {}

    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for Frames {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        fn did_output(&self, _output: &AVCaptureOutput, buffer: &CMSampleBuffer, _connection: &AVCaptureConnection) {
            if let Some(shot) = copy_luma(buffer) {
                // Full: the reader is behind, so this frame is dropped.
                let _ = self.ivars().tx.try_send(shot);
            }
        }
    }
);

impl Frames {
    fn new(tx: SyncSender<Shot>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { tx });
        // SAFETY: NSObject's init on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }
}

/// The luma plane of a bi-planar 4:2:0 frame, row by row without the padding.
fn copy_luma(buffer: &CMSampleBuffer) -> Option<Shot> {
    // SAFETY: the sample buffer is live for this call; the pixel buffer is locked while read.
    unsafe {
        let t_s = seconds(buffer.presentation_time_stamp());
        let image = buffer.image_buffer()?;
        let kind = CVPixelBufferGetPixelFormatType(&image);
        if kind != kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
            && kind != kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
        {
            return Some(Shot { width: 0, height: 0, data: Vec::new(), t_s });
        }
        if CVPixelBufferLockBaseAddress(&image, CVPixelBufferLockFlags::ReadOnly) != 0 {
            return None;
        }
        let width = CVPixelBufferGetWidthOfPlane(&image, 0);
        let height = CVPixelBufferGetHeightOfPlane(&image, 0);
        let stride = CVPixelBufferGetBytesPerRowOfPlane(&image, 0);
        let base = CVPixelBufferGetBaseAddressOfPlane(&image, 0).cast::<u8>();
        let mut data = Vec::new();
        if !base.is_null() && stride >= width {
            data.reserve_exact(width * height);
            for y in 0..height {
                data.extend_from_slice(std::slice::from_raw_parts(base.add(y * stride), width));
            }
        }
        CVPixelBufferUnlockBaseAddress(&image, CVPixelBufferLockFlags::ReadOnly);
        Some(Shot { width, height, data, t_s })
    }
}

pub struct AvfCamera {
    device: DeviceInfo,
    modes: Vec<VideoMode>,
    mode: VideoMode,
    session: Retained<AVCaptureSession>,
    output: Retained<AVCaptureVideoDataOutput>,
    _frames: Retained<Frames>,
    _queue: DispatchRetained<DispatchQueue>,
    rx: Receiver<Shot>,
    clock: Clock,
    camera: Retained<AVCaptureDevice>,
    _configuration: ConfigurationLock,
}

impl AvfCamera {
    pub fn open(device: &DeviceInfo, wanted: Option<&VideoMode>) -> Result<Self, CameraError> {
        native(|| autoreleasepool(|_| Self::start(device, wanted)))
    }

    fn start(info: &DeviceInfo, wanted: Option<&VideoMode>) -> Result<Self, CameraError> {
        // SAFETY: a class method with a valid string.
        let device = unsafe { AVCaptureDevice::deviceWithUniqueID(&NSString::from_str(&info.id)) }
            .ok_or_else(|| CameraError::NotFound(info.name.clone()))?;
        let offers = offers(&device);
        let modes: Vec<VideoMode> = offers.iter().map(|o| o.mode).collect();
        let mode = pick_mode(&modes, wanted).ok_or(CameraError::NoMode)?;
        let offer = offers.iter().find(|o| o.mode == mode).ok_or(CameraError::NoMode)?;
        allowed()?;

        let (tx, rx) = sync_channel(QUEUE);
        let frames = Frames::new(tx);
        let queue = DispatchQueue::new("laser-level-webcam.camera", None);
        // SAFETY: the calls below configure objects this function owns, in the order AVFoundation
        // documents: input and output inside a configuration, then the format with the device
        // locked across startRunning, so the session does not put its own preset back.
        unsafe {
            let session = AVCaptureSession::new();
            let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device).map_err(|e| os(&e))?;
            let output = AVCaptureVideoDataOutput::new();
            output.setAlwaysDiscardsLateVideoFrames(true);
            let key = |k: &'static objc2_core_foundation::CFString| -> &'static NSString {
                // CFString and NSString are the same object (toll-free bridged).
                &*(std::ptr::from_ref(k).cast::<NSString>())
            };
            let pixel = NSNumber::new_u32(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange);
            let width = NSNumber::new_u32(mode.width);
            let height = NSNumber::new_u32(mode.height);
            let settings = NSDictionary::<NSString, AnyObject>::from_slices(
                &[key(kCVPixelBufferPixelFormatTypeKey), key(kCVPixelBufferWidthKey), key(kCVPixelBufferHeightKey)],
                &[&*pixel, &*width, &*height],
            );
            output.setVideoSettings(Some(&settings));
            output.setSampleBufferDelegate_queue(Some(ProtocolObject::from_ref(&*frames)), Some(&queue));

            session.beginConfiguration();
            if !session.canAddInput(&input) {
                session.commitConfiguration();
                return Err(CameraError::Os(format!("{} cannot be opened (is another app using it?)", info.name)));
            }
            session.addInput(&input);
            if !session.canAddOutput(&output) {
                session.commitConfiguration();
                return Err(CameraError::Os("macOS refused the frame output".into()));
            }
            session.addOutput(&output);
            session.commitConfiguration();

            let configuration = ConfigurationLock::new(&device)?;
            device.setActiveFormat(&offer.format);
            device.setActiveVideoMinFrameDuration(offer.duration);
            device.setActiveVideoMaxFrameDuration(offer.duration);
            let duration = offer.duration;
            let capture = Self {
                device: info.clone(),
                modes,
                mode,
                session,
                output,
                _frames: frames,
                _queue: queue,
                rx,
                clock: Clock::default(),
                camera: device,
                // Keep measurement settings stable until the session has stopped.
                _configuration: configuration,
            };
            // macOS applies an independent frame-rate limit on the output connection.
            let connection = capture
                .output
                .connectionWithMediaType(video())
                .ok_or_else(|| os("No video connection for this camera"))?;
            #[allow(deprecated)] // Deprecated on iOS; supported on macOS.
            {
                if connection.isVideoMinFrameDurationSupported() {
                    connection.setVideoMinFrameDuration(duration);
                }
                if connection.isVideoMaxFrameDurationSupported() {
                    connection.setVideoMaxFrameDuration(duration);
                }
            }
            capture.session.startRunning();
            if !capture.session.isRunning() {
                return Err(os("AVFoundation could not start the camera"));
            }
            let actual = CMVideoFormatDescriptionGetDimensions(&capture.camera.activeFormat().formatDescription());
            if (actual.width as u32, actual.height as u32) != (mode.width, mode.height) {
                return Err(os(format!(
                    "The camera selected {}x{} instead of {}x{}",
                    actual.width, actual.height, mode.width, mode.height
                )));
            }
            Ok(capture)
        }
    }
}

impl FrameSource for AvfCamera {
    fn device(&self) -> &DeviceInfo {
        &self.device
    }

    fn modes(&self) -> &[VideoMode] {
        &self.modes
    }

    fn mode(&self) -> VideoMode {
        self.mode
    }

    fn controls(&mut self) -> Vec<ControlInfo> {
        // SAFETY: getters only; unsupported modes are never exposed as settable controls.
        unsafe {
            [
                (
                    ControlId::Exposure,
                    self.camera.isExposureModeSupported(AVCaptureExposureMode::Locked),
                    self.camera.isExposureModeSupported(AVCaptureExposureMode::ContinuousAutoExposure),
                    self.camera.exposureMode() == AVCaptureExposureMode::Locked,
                ),
                (
                    ControlId::WhiteBalance,
                    self.camera.isWhiteBalanceModeSupported(AVCaptureWhiteBalanceMode::Locked),
                    self.camera.isWhiteBalanceModeSupported(AVCaptureWhiteBalanceMode::ContinuousAutoWhiteBalance),
                    self.camera.whiteBalanceMode() == AVCaptureWhiteBalanceMode::Locked,
                ),
                (
                    ControlId::Focus,
                    self.camera.isFocusModeSupported(AVCaptureFocusMode::Locked),
                    self.camera.isFocusModeSupported(AVCaptureFocusMode::ContinuousAutoFocus),
                    self.camera.focusMode() == AVCaptureFocusMode::Locked,
                ),
            ]
            .into_iter()
            .filter(|(_, lock, auto, _)| *lock && *auto)
            .map(|(id, _, _, locked)| ControlInfo {
                id,
                min: 0,
                max: 0,
                step: 1,
                default: 0,
                value: 0,
                auto: Some(!locked),
                unit: ControlUnit::ModeOnly,
            })
            .collect()
        }
    }
    fn set_control(&mut self, setting: ControlSetting) -> Result<ControlInfo, CameraError> {
        if !self.controls().iter().any(|c| c.id == setting.id) {
            return Err(CameraError::NoControl(setting.id));
        }
        if matches!(setting.mode, ControlMode::Manual(v) if v != 0) {
            return Err(os(
                "macOS supports locking the current value only; use manual 0 to lock or auto to release it",
            ));
        }
        let auto = setting.mode == ControlMode::Auto;
        // SAFETY: the desired modes were checked above and the configuration lock is held.
        native(|| unsafe {
            match setting.id {
                ControlId::Exposure => self.camera.setExposureMode(if auto {
                    AVCaptureExposureMode::ContinuousAutoExposure
                } else {
                    AVCaptureExposureMode::Locked
                }),
                ControlId::WhiteBalance => self.camera.setWhiteBalanceMode(if auto {
                    AVCaptureWhiteBalanceMode::ContinuousAutoWhiteBalance
                } else {
                    AVCaptureWhiteBalanceMode::Locked
                }),
                ControlId::Focus => self.camera.setFocusMode(if auto {
                    AVCaptureFocusMode::ContinuousAutoFocus
                } else {
                    AVCaptureFocusMode::Locked
                }),
                _ => {}
            }
            Ok(())
        })?;
        self.controls().into_iter().find(|c| c.id == setting.id).ok_or(CameraError::NoControl(setting.id))
    }
    fn next_frame(&mut self) -> Result<Frame, CameraError> {
        // Long enough for the camera to start (and for the user to allow it), and for a few
        // frames at its slowest.
        let wait_s = if self.clock.started() { (5.0 / self.mode.fps.max(0.1)).max(3.0) } else { 10.0 };
        let mut bad = 0;
        loop {
            let shot = match self.rx.recv_timeout(Duration::from_secs_f64(wait_s)) {
                Ok(s) => s,
                Err(RecvTimeoutError::Timeout) => return Err(CameraError::Lost(format!("no frame for {wait_s:.0} s"))),
                Err(RecvTimeoutError::Disconnected) => return Err(CameraError::Lost("the capture stopped".into())),
            };
            if shot.width == 0 || shot.data.len() != shot.width * shot.height {
                bad += 1;
                if bad >= 10 {
                    return Err(CameraError::Lost("macOS sends frames this tool cannot read".into()));
                }
                continue;
            }
            if (shot.width, shot.height) != (self.mode.width as usize, self.mode.height as usize) {
                return Err(os(format!(
                    "AVFoundation delivered {}x{} instead of {}x{}",
                    shot.width, shot.height, self.mode.width, self.mode.height
                )));
            }
            let (t_s, seq) = self.clock.stamp(shot.t_s, self.mode.fps);
            return Ok(Frame { width: shot.width, height: shot.height, data: shot.data, t_s, seq });
        }
    }
}

impl Drop for AvfCamera {
    fn drop(&mut self) {
        // SAFETY: stopRunning waits until the session has stopped; then no more frames come.
        unsafe {
            self.output.setSampleBufferDelegate_queue(None, None);
            self.session.stopRunning();
        }
        self._queue.exec_sync(|| {});
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn objective_c_configuration_exceptions_become_camera_errors() {
        let error = native::<()>(|| {
            objc2_foundation::NSException::new(
                ns_string!("LaserCameraTest"),
                Some(ns_string!("unsupported mode")),
                None,
            )
            .unwrap()
            .raise()
        })
        .unwrap_err();
        assert!(error.to_string().contains("unsupported mode"));
    }
}
