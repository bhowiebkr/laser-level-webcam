//! macOS capture. AVFoundation supplies decoded NV12, not USB transport descriptors or raw
//! sensor data. Its macOS API exposes auto/lock modes, but not numeric exposure, focus or
//! white-balance values. Report those honestly as mode-only controls.
#![allow(unsafe_code)]

use std::sync::{Condvar, Mutex, mpsc};
use std::time::Duration;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_av_foundation::*;
use objc2_core_media::{CMSampleBuffer, CMTime, CMVideoFormatDescriptionGetDimensions};
use objc2_core_video::*;
use objc2_foundation::{NSArray, NSBundle, NSDictionary, NSNumber, NSObject, NSObjectProtocol, NSString, ns_string};

use crate::{Backend, CameraError, DeviceInfo, Frame, FrameSource};
use laser_core::camera::{
    ControlId, ControlInfo, ControlMode, ControlSetting, ControlUnit, PixelFormat, VideoMode, pick_mode,
};

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

pub fn devices() -> Vec<DeviceInfo> {
    autoreleasepool(|_| {
        // SAFETY: discovery is synchronous; the returned objects stay alive while read. The
        // external-unknown name is the pre-macOS-14 alias and also works on newer systems.
        unsafe {
            #[allow(deprecated)]
            let types =
                NSArray::from_slice(&[AVCaptureDeviceTypeBuiltInWideAngleCamera, AVCaptureDeviceTypeExternalUnknown]);
            AVCaptureDeviceDiscoverySession::discoverySessionWithDeviceTypes_mediaType_position(
                &types,
                AVMediaTypeVideo,
                AVCaptureDevicePosition::Unspecified,
            )
            .devices()
            .iter()
            .map(|d| DeviceInfo {
                name: d.localizedName().to_string(),
                id: d.uniqueID().to_string(),
                backend: Backend::AvFoundation,
            })
            .collect()
        }
    })
}

fn authorize() -> Result<(), CameraError> {
    // SAFETY: only video is requested. Both shipped executables embed NSCameraUsageDescription.
    // The completion block owns its sender and can safely outlive a timed-out request.
    unsafe {
        let video = AVMediaTypeVideo.ok_or_else(|| os("AVFoundation video capture is unavailable"))?;
        let status = AVCaptureDevice::authorizationStatusForMediaType(video);
        if status == AVAuthorizationStatus::Authorized {
            return Ok(());
        }
        if status == AVAuthorizationStatus::NotDetermined {
            if NSBundle::mainBundle().objectForInfoDictionaryKey(ns_string!("NSCameraUsageDescription")).is_none() {
                return Err(os(
                    "This executable must include NSCameraUsageDescription before requesting camera access",
                ));
            }
            let (tx, rx) = mpsc::channel();
            let block = RcBlock::new(move |allowed: Bool| {
                let _ = tx.send(allowed.as_bool());
            });
            AVCaptureDevice::requestAccessForMediaType_completionHandler(video, &block);
            match rx.recv_timeout(Duration::from_secs(60)) {
                Ok(true) => return Ok(()),
                Err(_) => {
                    return Err(os(
                        "Camera permission is still pending. Allow access in the macOS prompt, then open the camera again.",
                    ));
                }
                Ok(false) => {}
            }
        }
        Err(os(
            "Camera access is denied or restricted. Allow this app (or its terminal) in System Settings > Privacy & Security > Camera, then reopen it.",
        ))
    }
}

/// Pick common rates within each native format's advertised ranges, including both endpoints.
fn rates(min: f64, max: f64) -> Vec<f64> {
    if !min.is_finite() || !max.is_finite() || min <= 0.0 || max < min {
        return Vec::new();
    }
    let mut out = vec![min, max];
    out.extend([5., 10., 15., 24., 25., 30., 60.].into_iter().filter(|r| *r >= min && *r <= max));
    out.sort_by(f64::total_cmp);
    out.dedup_by(|a, b| (*a - *b).abs() < 0.001);
    out
}

struct NativeMode {
    mode: VideoMode,
    format: Retained<AVCaptureDeviceFormat>,
    duration: CMTime,
}

fn modes(device: &AVCaptureDevice) -> Vec<NativeMode> {
    let mut out: Vec<NativeMode> = Vec::new();
    // SAFETY: these getters return retained metadata; dimensions are read from a video device.
    unsafe {
        for format in device.formats() {
            let size = CMVideoFormatDescriptionGetDimensions(&format.formatDescription());
            if size.width <= 0 || size.height <= 0 {
                continue;
            }
            for range in format.videoSupportedFrameRateRanges() {
                for fps in rates(range.minFrameRate(), range.maxFrameRate()) {
                    let mode = VideoMode {
                        width: size.width as u32,
                        height: size.height as u32,
                        fps,
                        format: PixelFormat::Nv12,
                    };
                    // Preserve exact endpoint rationals: USB cameras often advertise e.g.
                    // 1000000/30000030 seconds, and reject a rounded value just outside it.
                    let duration = if fps == range.maxFrameRate() {
                        range.minFrameDuration()
                    } else if fps == range.minFrameRate() {
                        range.maxFrameDuration()
                    } else {
                        CMTime::with_seconds(1.0 / fps, 1_000_000_000)
                    };
                    if !out.iter().any(|m| m.mode == mode) {
                        out.push(NativeMode { mode, format: format.clone(), duration });
                    }
                }
            }
        }
    }
    out
}

#[derive(Default)]
struct Mailbox {
    latest: Option<Result<Frame, CameraError>>,
    clock: FrameClock,
}

/// Use presentation times, not callback arrival times, so lost frames remain visible as gaps.
#[derive(Default)]
struct FrameClock {
    first: Option<f64>,
    last_t: f64,
    last_seq: u64,
}

impl FrameClock {
    fn stamp(&mut self, timestamp: f64, fps: f64) -> Result<(f64, u64), CameraError> {
        if !timestamp.is_finite() {
            return Err(os("The camera supplied an invalid frame timestamp"));
        }
        let Some(first) = self.first else {
            self.first = Some(timestamp);
            return Ok((0.0, 0));
        };
        let t = timestamp - first;
        if t <= self.last_t {
            return Err(os("The camera's frame clock stopped or moved backwards"));
        }
        let seq = ((t * fps).round() as u64).max(self.last_seq + 1);
        self.last_t = t;
        self.last_seq = seq;
        Ok((t, seq))
    }
}

struct DelegateState {
    mailbox: Mutex<Mailbox>,
    ready: Condvar,
    fps: f64,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "LaserVideoDelegate"]
    #[ivars = DelegateState]
    struct VideoDelegate;

    unsafe impl NSObjectProtocol for VideoDelegate {}
    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for VideoDelegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        unsafe fn frame(&self, _output: &AVCaptureOutput, sample: &CMSampleBuffer, _connection: &AVCaptureConnection) {
            // SAFETY: AVFoundation lends the sample for this callback; copy pixels before returning.
            let result = unsafe { copy_frame(sample) };
            let state = self.ivars();
            let Ok(mut mail) = state.mailbox.lock() else { return };
            let result = result.and_then(|(mut frame, timestamp)| {
                let (t, seq) = mail.clock.stamp(timestamp, state.fps)?;
                frame.t_s = t;
                frame.seq = seq;
                Ok(frame)
            });
            // Keep at most one full frame. A slow reader gets the newest; timestamp gaps count loss.
            mail.latest = Some(result);
            state.ready.notify_one();
        }
    }
);

/// Copy a padded Y plane without interpreting its chroma or modifying its range.
fn copy_rows(bytes: &[u8], width: usize, height: usize, stride: usize) -> Result<Vec<u8>, CameraError> {
    let needed = height.checked_sub(1).and_then(|h| h.checked_mul(stride)).and_then(|n| n.checked_add(width));
    if width == 0 || stride < width || needed.is_none_or(|n| n > bytes.len()) {
        return Err(os("Invalid camera luma plane dimensions"));
    }
    let mut data = Vec::with_capacity(width.checked_mul(height).ok_or_else(|| os("Camera frame is too large"))?);
    for row in 0..height {
        data.extend_from_slice(&bytes[row * stride..row * stride + width]);
    }
    Ok(data)
}

unsafe fn copy_frame(sample: &CMSampleBuffer) -> Result<(Frame, f64), CameraError> {
    // SAFETY: called only with the live sample supplied by AVFoundation. A successful buffer
    // lock is paired with an unlock even on validation failure; the borrowed bytes never escape.
    unsafe {
        let buffer = sample.image_buffer().ok_or_else(|| os("The camera supplied no pixel buffer"))?;
        let format = CVPixelBufferGetPixelFormatType(&buffer);
        if ![kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange]
            .contains(&format)
            || CVPixelBufferGetPlaneCount(&buffer) != 2
        {
            return Err(os("AVFoundation did not deliver the requested NV12 frames"));
        }
        if CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags::ReadOnly) != 0 {
            return Err(os("Could not lock camera pixels"));
        }
        let result = (|| {
            let width = CVPixelBufferGetWidthOfPlane(&buffer, 0);
            let height = CVPixelBufferGetHeightOfPlane(&buffer, 0);
            let stride = CVPixelBufferGetBytesPerRowOfPlane(&buffer, 0);
            let ptr = CVPixelBufferGetBaseAddressOfPlane(&buffer, 0);
            let len = height
                .checked_sub(1)
                .and_then(|h| h.checked_mul(stride))
                .and_then(|n| n.checked_add(width))
                .filter(|n| *n <= isize::MAX as usize)
                .ok_or_else(|| os("Invalid camera frame size"))?;
            if ptr.is_null() || stride < width {
                return Err(os("Invalid camera pixel buffer"));
            }
            let data = copy_rows(std::slice::from_raw_parts(ptr.cast::<u8>(), len), width, height, stride)?;
            Ok((Frame { width, height, data, t_s: 0.0, seq: 0 }, sample.presentation_time_stamp().seconds()))
        })();
        CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags::ReadOnly);
        result
    }
}

pub struct AvCamera {
    device: DeviceInfo,
    camera: Retained<AVCaptureDevice>,
    session: Retained<AVCaptureSession>,
    output: Retained<AVCaptureVideoDataOutput>,
    delegate: Retained<VideoDelegate>,
    queue: DispatchRetained<DispatchQueue>,
    modes: Vec<VideoMode>,
    mode: VideoMode,
    // macOS may renegotiate the sensor format when capture starts if this lock is released.
    // Measurement needs stable settings; release the lock only after stopping the session.
    _configuration: ConfigurationLock,
}

impl AvCamera {
    pub fn open(device: &DeviceInfo, wanted: Option<&VideoMode>) -> Result<Self, CameraError> {
        native(authorize)?;
        native(|| {
            autoreleasepool(|_| {
                // SAFETY: all configuration stays on the opening thread. Session, device, delegate
                // and serial callback queue are retained for the whole capture lifetime.
                unsafe {
                    let camera = AVCaptureDevice::deviceWithUniqueID(&NSString::from_str(&device.id))
                        .ok_or_else(|| CameraError::NotFound(device.name.clone()))?;
                    let native = modes(&camera);
                    let modes: Vec<_> = native.iter().map(|m| m.mode).collect();
                    let mode = pick_mode(&modes, wanted).ok_or(CameraError::NoMode)?;
                    if let Some(wanted) = wanted.filter(|w| laser_core::camera::find_mode(&modes, w).is_none()) {
                        return Err(os(format!("The camera does not offer {wanted} through AVFoundation")));
                    }
                    let selected = native.iter().find(|m| m.mode == mode).ok_or(CameraError::NoMode)?;
                    let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&camera).map_err(os)?;
                    let session = AVCaptureSession::new();
                    let output = AVCaptureVideoDataOutput::new();
                    session.beginConfiguration();
                    if !session.canAddInput(&input) {
                        return Err(os("Cannot add this camera to a capture session"));
                    }
                    session.addInput(&input);
                    if !session.canAddOutput(&output) {
                        return Err(os("Cannot add video output to the capture session"));
                    }
                    session.addOutput(&output);
                    session.commitConfiguration();
                    // InputPriority is an iOS-only preset. On macOS configure the device directly.
                    let configuration = ConfigurationLock::new(&camera)?;
                    camera.setActiveFormat(&selected.format);
                    let duration = selected.duration;
                    camera.setActiveVideoMinFrameDuration(duration);
                    camera.setActiveVideoMaxFrameDuration(duration);
                    let available = output.availableVideoCVPixelFormatTypes();
                    let pixel = [
                        kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
                        kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                    ]
                    .into_iter()
                    .find(|p| available.iter().any(|v| v.unsignedIntValue() == *p))
                    .ok_or(CameraError::NoMode)?;
                    let key = NSString::from_str(&kCVPixelBufferPixelFormatTypeKey.to_string());
                    let width_key = NSString::from_str(&kCVPixelBufferWidthKey.to_string());
                    let height_key = NSString::from_str(&kCVPixelBufferHeightKey.to_string());
                    let value = NSNumber::new_u32(pixel);
                    let width = NSNumber::new_u32(mode.width);
                    let height = NSNumber::new_u32(mode.height);
                    output.setVideoSettings(Some(&NSDictionary::from_slices(
                        &[&*key, &*width_key, &*height_key],
                        &[&*value as &objc2::runtime::AnyObject, &*width, &*height],
                    )));
                    output.setAlwaysDiscardsLateVideoFrames(true);
                    // On macOS the output connection has independent rate limits. Device-only
                    // limits can still deliver 30 fps when the selected output mode is 5 fps.
                    let connection = output
                        .connectionWithMediaType(AVMediaTypeVideo.ok_or(CameraError::NoMode)?)
                        .ok_or_else(|| os("No video connection for this camera"))?;
                    #[allow(deprecated)] // Deprecated on iOS, supported on macOS.
                    {
                        if connection.isVideoMinFrameDurationSupported() {
                            connection.setVideoMinFrameDuration(duration);
                        }
                        if connection.isVideoMaxFrameDurationSupported() {
                            connection.setVideoMaxFrameDuration(duration);
                        }
                    }
                    let queue = DispatchQueue::new("laser.camera.frames", None);
                    let allocated = VideoDelegate::alloc().set_ivars(DelegateState {
                        mailbox: Mutex::new(Mailbox::default()),
                        ready: Condvar::new(),
                        fps: mode.fps,
                    });
                    let delegate: Retained<VideoDelegate> = msg_send![super(allocated), init];
                    output.setSampleBufferDelegate_queue(Some(ProtocolObject::from_ref(&*delegate)), Some(&queue));
                    let capture = Self {
                        device: device.clone(),
                        camera,
                        session,
                        output,
                        delegate,
                        queue,
                        modes,
                        mode,
                        _configuration: configuration,
                    };
                    capture.session.startRunning();
                    if !capture.session.isRunning() {
                        return Err(os("AVFoundation could not start the camera"));
                    }
                    let actual =
                        CMVideoFormatDescriptionGetDimensions(&capture.camera.activeFormat().formatDescription());
                    if (actual.width as u32, actual.height as u32) != (mode.width, mode.height) {
                        return Err(os(format!(
                            "The camera selected {}x{} instead of {}x{}",
                            actual.width, actual.height, mode.width, mode.height
                        )));
                    }
                    Ok(capture)
                }
            })
        })
    }
}

impl Drop for AvCamera {
    fn drop(&mut self) {
        // SAFETY: stop new callbacks and drain the serial queue before releasing the delegate.
        unsafe {
            self.output.setSampleBufferDelegate_queue(None, None);
            self.session.stopRunning();
        }
        self.queue.exec_sync(|| {});
    }
}

impl FrameSource for AvCamera {
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
        let state = self.delegate.ivars();
        let mail = state.mailbox.lock().map_err(|_| os("Camera frame queue failed"))?;
        let (mut mail, _) = state
            .ready
            .wait_timeout_while(mail, Duration::from_secs(3), |m| m.latest.is_none())
            .map_err(|_| os("Camera frame queue failed"))?;
        let frame = mail.latest.take().ok_or_else(|| {
            CameraError::Lost("no frames for 3 seconds; check the connection and camera permission".into())
        })??;
        if (frame.width, frame.height) != (self.mode.width as usize, self.mode.height as usize) {
            return Err(os(format!(
                "AVFoundation delivered {}x{} instead of {}x{}; select a supported camera mode again",
                frame.width, frame.height, self.mode.width, self.mode.height
            )));
        }
        Ok(frame)
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
    #[test]
    fn padded_luma_rows_are_copied_without_padding() {
        assert_eq!(copy_rows(&[1, 2, 99, 99, 3, 4], 2, 2, 4).unwrap(), [1, 2, 3, 4]);
        assert!(copy_rows(&[1, 2], 2, 2, 4).is_err());
        assert!(copy_rows(&[1, 2], 2, 1, 1).is_err());
        assert!(copy_rows(&[], 0, 0, 0).is_err());
        assert!(copy_rows(&[], 2, usize::MAX, 4).is_err());
    }
    #[test]
    fn presentation_clock_preserves_gaps_and_rejects_bad_times() {
        let mut clock = FrameClock::default();
        assert_eq!(clock.stamp(10.0, 30.).unwrap(), (0., 0));
        assert_eq!(clock.stamp(10.1, 30.).unwrap().1, 3);
        assert!(clock.stamp(10.1, 30.).is_err());
        assert!(clock.stamp(9., 30.).is_err());
        assert!(clock.stamp(f64::NAN, 30.).is_err());
    }
    #[test]
    fn rates_stay_in_the_advertised_range() {
        assert_eq!(rates(7.5, 29.97), [7.5, 10., 15., 24., 25., 29.97]);
        assert_eq!(rates(30., 30.), [30.]);
        assert!(rates(0., 30.).is_empty());
        assert!(rates(30., 5.).is_empty());
    }
}
