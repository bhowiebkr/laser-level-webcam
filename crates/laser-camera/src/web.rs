//! The browser's camera, through `getUserMedia`.
//!
//! The browser gives decoded colour frames, not the sensor's luma plane, so each frame is drawn
//! into a canvas, read back as RGBA and mixed to luma with the BT.601 weights. The browser's own
//! colour processing stays in the numbers; lock the controls all the same.
//!
//! Everything here is asynchronous in the browser: opening asks the user for the camera, the
//! device names come only after that, and a control setting takes effect later. So [`open`] is an
//! `async fn`, [`FrameSource::try_frame`] gives a frame only when a new one has been shown, and a
//! control's real state arrives through [`FrameSource::settled`].
//!
//! Controls follow the Media Capture Image spec (`exposureMode` and `exposureTime` in 100 µs,
//! `whiteBalanceMode` and `colorTemperature`, brightness, contrast, saturation, sharpness, focus,
//! zoom), as far as the browser and the camera give them. Browsers have no gain or mains filter.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::{Array, Function, Object, Promise, Reflect};
use laser_core::camera::{
    CameraSettings, ControlId, ControlInfo, ControlMode, ControlSetting, ControlUnit, PixelFormat, VideoMode,
};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    CanvasRenderingContext2d, HtmlCanvasElement, HtmlVideoElement, MediaDeviceInfo, MediaDeviceKind, MediaStream,
    MediaStreamTrack,
};

use crate::{Applied, Backend, CameraError, DeviceInfo, Frame, FrameSource};

/// The camera the browser picks, or the one the user picks in its prompt.
pub const ANY: &str = "Browser camera";

/// Sizes offered in the mode list, when the camera's range takes them.
const SIZES: [(u32, u32); 6] = [(3840, 2160), (2592, 1944), (1920, 1080), (1280, 960), (1280, 720), (640, 480)];

/// The size asked for when the settings name none.
const DEFAULT_SIZE: (u32, u32) = (1920, 1080);

/// The controls the spec has, with their mode and value names, and the unit of the value.
const CONTROLS: [(ControlId, Option<&str>, &str, ControlUnit); 8] = [
    (ControlId::Exposure, Some("exposureMode"), "exposureTime", ControlUnit::HundredMicroseconds),
    (ControlId::WhiteBalance, Some("whiteBalanceMode"), "colorTemperature", ControlUnit::Kelvin),
    (ControlId::Brightness, None, "brightness", ControlUnit::Raw),
    (ControlId::Contrast, None, "contrast", ControlUnit::Raw),
    (ControlId::Saturation, None, "saturation", ControlUnit::Raw),
    (ControlId::Sharpness, None, "sharpness", ControlUnit::Raw),
    (ControlId::Focus, Some("focusMode"), "focusDistance", ControlUnit::Raw),
    (ControlId::Zoom, None, "zoom", ControlUnit::Raw),
];

thread_local! {
    /// The browser's cameras, as last listed. Their names are empty until the user allows one.
    static DEVICES: RefCell<Vec<DeviceInfo>> = const { RefCell::new(Vec::new()) };
}

fn js_err(what: &str, e: &JsValue) -> CameraError {
    let name = Reflect::get(e, &"name".into()).ok().and_then(|v| v.as_string()).unwrap_or_default();
    let message = Reflect::get(e, &"message".into()).ok().and_then(|v| v.as_string()).unwrap_or_default();
    match name.as_str() {
        "NotAllowedError" => {
            CameraError::Os(format!("{what}: the camera was not allowed. Allow it in the browser, or pick the synthetic camera"))
        }
        "NotFoundError" | "OverconstrainedError" => CameraError::NotFound(format!("{what}: no camera fits ({message})")),
        "NotReadableError" => CameraError::Os(format!("{what}: the camera is in use by another program")),
        _ if message.is_empty() => CameraError::Os(format!("{what}: {e:?}")),
        _ => CameraError::Os(format!("{what}: {message}")),
    }
}

fn get(obj: &JsValue, key: &str) -> Option<JsValue> {
    Reflect::get(obj, &key.into()).ok().filter(|v| !v.is_undefined() && !v.is_null())
}

fn num(obj: &JsValue, key: &str) -> Option<f64> {
    get(obj, key).and_then(|v| v.as_f64())
}

fn set(obj: &Object, key: &str, value: impl Into<JsValue>) {
    // Setting a field on a plain object made here does not fail.
    let _ = Reflect::set(obj, &key.into(), &value.into());
}

fn call0(obj: &JsValue, method: &str) -> Option<JsValue> {
    get(obj, method)?.dyn_into::<Function>().ok()?.call0(obj).ok()
}

/// The browser's cameras as last listed, then the one the browser picks.
pub fn devices() -> Vec<DeviceInfo> {
    let mut out = DEVICES.with(|d| d.borrow().clone());
    out.push(DeviceInfo { name: ANY.into(), id: String::new(), backend: Backend::Browser });
    out
}

/// List the browser's cameras again; [`devices`] has them once it is done.
pub async fn rescan() -> Result<(), CameraError> {
    let media = web_sys::window()
        .ok_or_else(|| CameraError::Os("no window".into()))?
        .navigator()
        .media_devices()
        .map_err(|e| js_err("no camera access in this browser", &e))?;
    let promise = media.enumerate_devices().map_err(|e| js_err("listing cameras", &e))?;
    let list: Array = JsFuture::from(promise).await.map_err(|e| js_err("listing cameras", &e))?.unchecked_into();
    let mut found = Vec::new();
    for item in list.iter() {
        let info: MediaDeviceInfo = item.unchecked_into();
        // Without the user's leave, the names are empty and there is nothing to pick by.
        if info.kind() == MediaDeviceKind::Videoinput && !info.label().is_empty() {
            found.push(DeviceInfo { name: info.label(), id: info.device_id(), backend: Backend::Browser });
        }
    }
    DEVICES.with(|d| *d.borrow_mut() = found);
    Ok(())
}

/// The track constraints for this device and mode.
fn constraints(device: &DeviceInfo, mode: Option<&VideoMode>) -> Object {
    let video = Object::new();
    if !device.id.is_empty() {
        let exact = Object::new();
        set(&exact, "exact", device.id.as_str());
        set(&video, "deviceId", exact);
    }
    let (w, h) = mode.map_or(DEFAULT_SIZE, |m| (m.width, m.height));
    set(&video, "width", w);
    set(&video, "height", h);
    if let Some(m) = mode {
        set(&video, "frameRate", m.fps);
    }
    // The camera's own sizes only: a size the browser scales to averages the beam's pixels.
    set(&video, "resizeMode", "none");
    video
}

/// Ask for the camera the settings name (the one the browser picks if they name none or
/// [`ANY`]), and apply their controls.
pub async fn open(
    settings: &CameraSettings,
    repaint: impl Fn() + 'static,
) -> Result<(WebCamera, Vec<Applied>), CameraError> {
    let mut device = DeviceInfo { name: ANY.into(), id: String::new(), backend: Backend::Browser };
    if let Some(name) = settings.device.as_deref().filter(|n| *n != ANY) {
        // A name saved from an earlier visit: the list may not be read yet.
        let _ = rescan().await;
        device = devices().into_iter().find(|d| d.name == name).ok_or_else(|| CameraError::NotFound(name.to_owned()))?;
    }
    let window = web_sys::window().ok_or_else(|| CameraError::Os("no window".into()))?;
    let media = window.navigator().media_devices().map_err(|e| js_err("no camera access in this browser", &e))?;
    let video_c = constraints(&device, settings.mode.as_ref());
    let all = Object::new();
    set(&all, "video", &video_c);
    set(&all, "audio", false);
    let promise = media.get_user_media_with_constraints(all.unchecked_ref()).map_err(|e| js_err("opening", &e))?;
    let stream: MediaStream = JsFuture::from(promise).await.map_err(|e| js_err("opening", &e))?.unchecked_into();
    let track: MediaStreamTrack =
        stream.get_video_tracks().get(0).dyn_into().map_err(|_| CameraError::Os("the stream has no video".into()))?;
    // Now the names can be read.
    let _ = rescan().await;

    let document = window.document().ok_or_else(|| CameraError::Os("no document".into()))?;
    let video: HtmlVideoElement = document
        .create_element("video")
        .map_err(|e| js_err("making a video element", &e))?
        .unchecked_into();
    video.set_muted(true);
    let _ = video.set_attribute("playsinline", "");
    video.set_src_object(Some(&stream));
    let playing = video.play().map_err(|e| js_err("starting the video", &e))?;
    JsFuture::from(playing).await.map_err(|e| js_err("starting the video", &e))?;

    let (w, h) = (video.video_width(), video.video_height());
    if w == 0 || h == 0 {
        return Err(CameraError::Os("the camera sends no picture".into()));
    }
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .map_err(|e| js_err("making a canvas", &e))?
        .unchecked_into();
    canvas.set_width(w);
    canvas.set_height(h);
    let options = Object::new();
    set(&options, "willReadFrequently", true);
    let ctx: CanvasRenderingContext2d = canvas
        .get_context_with_context_options("2d", &options)
        .map_err(|e| js_err("making a canvas", &e))?
        .ok_or_else(|| CameraError::Os("no 2D canvas".into()))?
        .unchecked_into();

    let track_settings = call0(&track, "getSettings").unwrap_or(JsValue::UNDEFINED);
    let fps = num(&track_settings, "frameRate").filter(|f| *f > 0.0).unwrap_or(30.0);
    let mode = VideoMode { width: w, height: h, fps, format: PixelFormat::Rgba };
    let caps = call0(&track, "getCapabilities").unwrap_or(JsValue::UNDEFINED);
    let modes = offered_modes(&caps, mode);
    let opened = DeviceInfo {
        name: if device.name == ANY { format!("{} (browser)", track.label()) } else { device.name },
        id: get(&track_settings, "deviceId").and_then(|v| v.as_string()).unwrap_or(device.id),
        backend: Backend::Browser,
    };

    let mut cam = WebCamera {
        device: opened,
        modes,
        mode,
        stream,
        track,
        video,
        ctx,
        base: video_c,
        wanted: Vec::new(),
        shown: Rc::new(Cell::new(None)),
        stopped: Rc::new(Cell::new(false)),
        last: None,
        first: None,
        settled: Rc::new(RefCell::new(Vec::new())),
        pending: Rc::new(Cell::new(0)),
    };
    cam.watch_frames(Rc::new(repaint));
    let applied = crate::apply(&mut cam, &settings.controls);
    Ok((cam, applied))
}

/// The usual sizes inside the camera's range, at its highest frame rate up to 30 fps, and the
/// mode it is in.
fn offered_modes(caps: &JsValue, current: VideoMode) -> Vec<VideoMode> {
    let range = |key: &str| {
        let r = get(caps, key);
        (r.as_ref().and_then(|r| num(r, "min")), r.as_ref().and_then(|r| num(r, "max")))
    };
    let (Some(w_max), Some(h_max)) = (range("width").1, range("height").1) else { return vec![current] };
    let fps = range("frameRate").1.map_or(current.fps, |f| f.min(30.0));
    let mut out: Vec<VideoMode> = SIZES
        .iter()
        .filter(|&&(w, h)| f64::from(w) <= w_max && f64::from(h) <= h_max)
        .map(|&(width, height)| VideoMode { width, height, fps, format: PixelFormat::Rgba })
        .collect();
    if !out.iter().any(|m| (m.width, m.height) == (current.width, current.height)) {
        out.insert(0, current);
    }
    out
}

/// What the frame callback last saw: the frame's media time, s, and frames shown so far.
type Shown = Option<(f64, u64)>;

/// An open browser camera. Its frames are read on the window's thread.
pub struct WebCamera {
    device: DeviceInfo,
    modes: Vec<VideoMode>,
    mode: VideoMode,
    stream: MediaStream,
    track: MediaStreamTrack,
    video: HtmlVideoElement,
    ctx: CanvasRenderingContext2d,
    /// The constraints it was opened with; every control change sends them again, with all the
    /// controls asked for so far, since a new set replaces the old one.
    base: Object,
    wanted: Vec<ControlSetting>,
    shown: Rc<Cell<Shown>>,
    stopped: Rc<Cell<bool>>,
    /// The last frame read, as `shown` had it.
    last: Shown,
    /// The first frame's time and count, for `t_s` and `seq` from 0.
    first: Shown,
    /// Controls read back after the browser applied them.
    settled: Rc<RefCell<Vec<Applied>>>,
    /// Control changes the browser has not finished.
    pending: Rc<Cell<u32>>,
}

impl WebCamera {
    /// Note each frame the video shows. Without `requestVideoFrameCallback` (older Firefox),
    /// the video's clock is read instead, at most once a frame time.
    fn watch_frames(&self, repaint: Rc<dyn Fn()>) {
        if get(&self.video, "requestVideoFrameCallback").is_none() {
            return;
        }
        let video = self.video.clone();
        let shown = Rc::clone(&self.shown);
        let stopped = Rc::clone(&self.stopped);
        let slot: Rc<RefCell<Option<Closure<dyn FnMut(f64, JsValue)>>>> = Rc::new(RefCell::new(None));
        let again = Rc::clone(&slot);
        *slot.borrow_mut() = Some(Closure::new(move |_now: f64, meta: JsValue| {
            if stopped.get() {
                // Let the closure go; it is not called again.
                again.borrow_mut().take();
                return;
            }
            let t = num(&meta, "mediaTime").unwrap_or(0.0);
            let n = num(&meta, "presentedFrames").unwrap_or(0.0) as u64;
            shown.set(Some((t, n)));
            repaint();
            if let Some(cb) = again.borrow().as_ref() {
                request_frame(&video, cb);
            }
        }));
        if let Some(cb) = slot.borrow().as_ref() {
            request_frame(&self.video, cb);
        }
    }

    /// The control's range and state, read from the track.
    fn read(&self, id: ControlId) -> Option<ControlInfo> {
        let &(_, mode_key, value_key, unit) = CONTROLS.iter().find(|c| c.0 == id)?;
        let caps = call0(&self.track, "getCapabilities")?;
        let now = call0(&self.track, "getSettings")?;
        let range = get(&caps, value_key)?;
        let (min, max) = (num(&range, "min")?, num(&range, "max")?);
        let step = num(&range, "step").filter(|s| *s > 0.0).unwrap_or(1.0);
        let value = num(&now, value_key).unwrap_or(min);
        let auto = mode_key.and_then(|key| {
            let modes: Array = get(&caps, key)?.dyn_into().ok()?;
            let names: Vec<String> = modes.iter().filter_map(|m| m.as_string()).collect();
            let has_auto = names.iter().any(|m| m == "continuous" || m == "single-shot");
            (has_auto && names.iter().any(|m| m == "manual"))
                .then(|| get(&now, key).and_then(|m| m.as_string()).is_none_or(|m| m != "manual"))
        });
        Some(ControlInfo {
            id,
            min: min.round() as i64,
            max: max.round() as i64,
            step: (step.round() as i64).max(1),
            default: value.round() as i64,
            value: value.round() as i64,
            auto,
            unit,
        })
    }

    /// Send the open constraints again with every control asked for so far.
    fn send_controls(&self) -> Result<Promise, CameraError> {
        let advanced = Object::new();
        for s in &self.wanted {
            let Some(&(_, mode_key, value_key, _)) = CONTROLS.iter().find(|c| c.0 == s.id) else { continue };
            match (s.mode, mode_key) {
                (ControlMode::Auto, Some(key)) => set(&advanced, key, "continuous"),
                (ControlMode::Auto, None) => {}
                (ControlMode::Manual(v), key) => {
                    if let Some(key) = key {
                        set(&advanced, key, "manual");
                    }
                    set(&advanced, value_key, v as f64);
                }
            }
        }
        let all = Object::assign(&Object::new(), &self.base);
        set(&all, "advanced", Array::of1(&advanced));
        self.track.apply_constraints_with_constraints(all.unchecked_ref()).map_err(|e| js_err("setting a control", &e))
    }
}

fn request_frame(video: &HtmlVideoElement, cb: &Closure<dyn FnMut(f64, JsValue)>) {
    if let Some(f) = get(video, "requestVideoFrameCallback").and_then(|f| f.dyn_into::<Function>().ok()) {
        let _ = f.call1(video, cb.as_ref());
    }
}

impl FrameSource for WebCamera {
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
        CONTROLS.iter().filter_map(|c| self.read(c.0)).collect()
    }

    /// Asks the browser for the setting and answers with it as asked; the browser's own answer,
    /// read back from the track, comes later through [`FrameSource::settled`].
    fn set_control(&mut self, setting: ControlSetting) -> Result<ControlInfo, CameraError> {
        let mut info = self.read(setting.id).ok_or(CameraError::NoControl(setting.id))?;
        match setting.mode {
            ControlMode::Auto if info.auto.is_none() => return Err(CameraError::NoControl(setting.id)),
            ControlMode::Auto => info.auto = Some(true),
            ControlMode::Manual(v) => {
                info.value = info.snap(v);
                info.auto = info.auto.map(|_| false);
            }
        }
        let asked = ControlSetting { mode: if let ControlMode::Manual(_) = setting.mode { ControlMode::Manual(info.value) } else { ControlMode::Auto }, ..setting };
        self.wanted.retain(|s| s.id != setting.id);
        self.wanted.push(asked);
        let promise = self.send_controls()?;
        let track = self.track.clone();
        let settled = Rc::clone(&self.settled);
        let pending = Rc::clone(&self.pending);
        pending.set(pending.get() + 1);
        wasm_bindgen_futures::spawn_local(async move {
            let result = JsFuture::from(promise).await;
            pending.set(pending.get().saturating_sub(1));
            let back = match result {
                Ok(_) => {
                    let cam = Readback { track };
                    cam.read(setting.id).ok_or(CameraError::NoControl(setting.id))
                }
                Err(e) => Err(js_err("setting a control", &e)),
            };
            settled.borrow_mut().push(Applied { setting: asked, result: back });
        });
        Ok(info)
    }

    /// Browsers give frames only as they show them; this one waits for none.
    fn next_frame(&mut self) -> Result<Frame, CameraError> {
        self.try_frame()?.ok_or_else(|| CameraError::Os("no frame yet".into()))
    }

    fn try_frame(&mut self) -> Result<Option<Frame>, CameraError> {
        if self.track.ready_state() == web_sys::MediaStreamTrackState::Ended {
            return Err(CameraError::Lost("the browser ended the camera's stream".into()));
        }
        let shown = if get(&self.video, "requestVideoFrameCallback").is_some() {
            self.shown.get()
        } else {
            // The video's clock, counted in frame times.
            let t = self.video.current_time();
            Some((t, (t / self.mode.frame_period_s()).floor() as u64))
        };
        let Some((t, n)) = shown else { return Ok(None) };
        if self.last.is_some_and(|(_, last_n)| last_n >= n) {
            return Ok(None);
        }
        self.last = Some((t, n));
        let (t0, n0) = *self.first.get_or_insert((t, n));
        let (w, h) = (self.mode.width, self.mode.height);
        self.ctx
            .draw_image_with_html_video_element(&self.video, 0.0, 0.0)
            .map_err(|e| js_err("reading a frame", &e))?;
        let rgba = self
            .ctx
            .get_image_data(0.0, 0.0, f64::from(w), f64::from(h))
            .map_err(|e| js_err("reading a frame", &e))?
            .data();
        Ok(Some(Frame {
            width: w as usize,
            height: h as usize,
            data: luma(&rgba),
            t_s: (t - t0).max(0.0),
            seq: n.saturating_sub(n0),
        }))
    }

    fn settled(&mut self) -> Vec<Applied> {
        std::mem::take(&mut *self.settled.borrow_mut())
    }
}

/// Reads a control off a track, for a setting the browser finished after the camera moved on.
struct Readback {
    track: MediaStreamTrack,
}

impl Readback {
    fn read(&self, id: ControlId) -> Option<ControlInfo> {
        let &(_, mode_key, value_key, unit) = CONTROLS.iter().find(|c| c.0 == id)?;
        let caps = call0(&self.track, "getCapabilities")?;
        let now = call0(&self.track, "getSettings")?;
        let range = get(&caps, value_key)?;
        let value = num(&now, value_key)?;
        let auto = mode_key.and_then(|key| get(&now, key)).and_then(|m| m.as_string()).map(|m| m != "manual");
        Some(ControlInfo {
            id,
            min: num(&range, "min")?.round() as i64,
            max: num(&range, "max")?.round() as i64,
            step: (num(&range, "step").unwrap_or(1.0).round() as i64).max(1),
            default: value.round() as i64,
            value: value.round() as i64,
            auto,
            unit,
        })
    }
}

impl Drop for WebCamera {
    fn drop(&mut self) {
        self.stopped.set(true);
        for t in self.stream.get_tracks().iter() {
            if let Ok(t) = t.dyn_into::<MediaStreamTrack>() {
                t.stop();
            }
        }
        self.video.set_src_object(None);
    }
}

/// BT.601 luma from RGBA, which is how the browser's YUV to RGB step was made for webcams.
pub fn luma(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .map(|p| ((77 * u32::from(p[0]) + 150 * u32::from(p[1]) + 29 * u32::from(p[2]) + 128) >> 8) as u8)
        .collect()
}
