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
use laser_core::camera::{CameraSettings, ControlId, ControlInfo, ControlMode, ControlSetting, PixelFormat, VideoMode};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    CanvasRenderingContext2d, HtmlCanvasElement, HtmlVideoElement, MediaDeviceInfo, MediaDeviceKind, MediaStream,
    MediaStreamTrack,
};

use crate::browser::{self, Ask, Value};
use crate::{Applied, Backend, CameraError, DeviceInfo, Frame, FrameSource};

/// The camera the browser picks, or the one the user picks in its prompt.
pub const ANY: &str = "Browser camera";

thread_local! {
    /// The browser's cameras, as last listed. Their names are empty until the user allows one.
    static DEVICES: RefCell<Vec<DeviceInfo>> = const { RefCell::new(Vec::new()) };
    /// Sizes a camera (by its id) gave another size for, so they are not offered again.
    static MISSING: RefCell<Vec<(String, (u32, u32))>> = const { RefCell::new(Vec::new()) };
}

fn js_err(what: &str, e: &JsValue) -> CameraError {
    let name = err_name(e);
    let message = Reflect::get(e, &"message".into()).ok().and_then(|v| v.as_string()).unwrap_or_default();
    if name.is_empty() && message.is_empty() {
        return CameraError::Os(if what.is_empty() { format!("{e:?}") } else { format!("{what}: {e:?}") });
    }
    browser::error(what, &name, &message)
}

/// The name of a browser error, such as `NotReadableError`.
fn err_name(e: &JsValue) -> String {
    Reflect::get(e, &"name".into()).ok().and_then(|v| v.as_string()).unwrap_or_default()
}

/// Wait this long, letting the browser run.
async fn sleep(ms: i32) {
    let promise = Promise::new(&mut |resolve, _| {
        let waited =
            web_sys::window().and_then(|w| w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms).ok());
        if waited.is_none() {
            let _ = resolve.call0(&JsValue::NULL);
        }
    });
    let _ = JsFuture::from(promise).await;
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

fn set_value(obj: &Object, key: &str, value: Value) {
    match value {
        Value::Text(t) => set(obj, key, t),
        Value::Number(n) => set(obj, key, n),
    }
}

/// A capability's lowest and highest value.
fn range(caps: &JsValue, key: &str) -> Option<(f64, f64)> {
    let r = get(caps, key)?;
    let (min, max) = (num(&r, "min")?, num(&r, "max")?);
    (min <= max).then_some((min, max))
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
fn constraints(device: &DeviceInfo, mode: Option<&VideoMode>, ask: Ask) -> Object {
    let video = Object::new();
    if !device.id.is_empty() {
        let exact = Object::new();
        set(&exact, "exact", device.id.as_str());
        set(&video, "deviceId", exact);
    }
    for (key, value) in browser::size_fields(mode, ask) {
        set_value(&video, key, value);
    }
    video
}

/// Ask the browser for the camera, asking for less each time it will not start.
async fn get_stream(
    media: &web_sys::MediaDevices,
    device: &DeviceInfo,
    mode: Option<&VideoMode>,
) -> Result<(MediaStream, Object), CameraError> {
    let mut failed = JsValue::UNDEFINED;
    for ask in Ask::ALL {
        let video_c = constraints(device, mode, ask);
        let all = Object::new();
        set(&all, "video", &video_c);
        set(&all, "audio", false);
        let promise = media.get_user_media_with_constraints(all.unchecked_ref()).map_err(|e| js_err("", &e))?;
        match JsFuture::from(promise).await {
            Ok(stream) => return Ok((stream.unchecked_into(), video_c)),
            // Refused, or no such camera: asking for less does not help.
            Err(e) if !browser::ask_less_after(&err_name(&e)) => {
                return Err(js_err("", &e));
            }
            Err(e) => failed = e,
        }
        // Give the camera a moment to let go of the try that failed.
        sleep(300).await;
    }
    Err(js_err("", &failed))
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
        device =
            devices().into_iter().find(|d| d.name == name).ok_or_else(|| CameraError::NotFound(name.to_owned()))?;
    }
    let window = web_sys::window().ok_or_else(|| CameraError::Os("no window".into()))?;
    let media = window.navigator().media_devices().map_err(|e| js_err("no camera access in this browser", &e))?;
    let (stream, video_c) = get_stream(&media, &device, settings.mode.as_ref()).await?;
    let track: MediaStreamTrack =
        stream.get_video_tracks().get(0).dyn_into().map_err(|_| CameraError::Os("the stream has no video".into()))?;
    // An open that fails from here on leaves nothing running. `Clone::clone` copies the handle;
    // the stream's own `clone` method would start a second stream that nothing stops.
    let mut undo = Undo { stream: Clone::clone(&stream), video: None };
    // Now the names can be read.
    let _ = rescan().await;

    let document = window.document().ok_or_else(|| CameraError::Os("no document".into()))?;
    let video: HtmlVideoElement =
        document.create_element("video").map_err(|e| js_err("making a video element", &e))?.unchecked_into();
    video.set_muted(true);
    let _ = video.set_attribute("playsinline", "");
    // On the page, but out of sight: some browsers call back for shown frames only then.
    let _ = video.style().set_property("position", "fixed");
    let _ = video.style().set_property("left", "0");
    let _ = video.style().set_property("top", "0");
    let _ = video.style().set_property("width", "2px");
    let _ = video.style().set_property("height", "2px");
    let _ = video.style().set_property("opacity", "0");
    let _ = video.style().set_property("pointer-events", "none");
    if let Some(body) = document.body() {
        let _ = body.append_child(&video);
    }
    undo.video = Some(video.clone());
    video.set_src_object(Some(&stream));
    let playing = video.play().map_err(|e| js_err("starting the video", &e))?;
    JsFuture::from(playing).await.map_err(|e| js_err("starting the video", &e))?;

    let (w, h) = (video.video_width(), video.video_height());
    if w == 0 || h == 0 {
        return Err(CameraError::Os("the camera sends no picture".into()));
    }
    let canvas: HtmlCanvasElement =
        document.create_element("canvas").map_err(|e| js_err("making a canvas", &e))?.unchecked_into();
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
    let opened = DeviceInfo {
        // The name it has in the list, so the list shows it picked.
        name: if device.name == ANY && !track.label().is_empty() { track.label() } else { device.name },
        id: get(&track_settings, "deviceId").and_then(|v| v.as_string()).unwrap_or(device.id),
        backend: Backend::Browser,
    };
    let missing: Vec<(u32, u32)> = MISSING.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(asked) = settings.mode.filter(|a| (a.width, a.height) != (w, h)) {
            let entry = (opened.id.clone(), (asked.width, asked.height));
            if !m.contains(&entry) {
                m.push(entry);
            }
        }
        m.iter().filter(|(id, _)| *id == opened.id).map(|&(_, size)| size).collect()
    });
    let max = |key: &str| get(&caps, key).and_then(|r| num(&r, "max"));
    let max_size = max("width").zip(max("height"));
    let modes = browser::offered_modes(max_size, max("frameRate"), mode, &missing);

    let mut cam = WebCamera {
        device: opened,
        modes,
        mode,
        stream,
        track,
        video,
        ctx,
        base: video_c,
        wanted: Rc::new(RefCell::new(Vec::new())),
        shown: Rc::new(Cell::new(None)),
        stopped: Rc::new(Cell::new(false)),
        last: None,
        first: None,
        settled: Rc::new(RefCell::new(Vec::new())),
        pending: Rc::new(Cell::new(0)),
    };
    // The camera undoes itself from now on.
    std::mem::forget(undo);
    cam.watch_frames(Rc::new(repaint));
    let applied = crate::apply_saved(&mut cam, &settings.controls);
    Ok((cam, applied))
}

/// Stops the stream and takes the video off the page when an open fails half way.
struct Undo {
    stream: MediaStream,
    video: Option<HtmlVideoElement>,
}

impl Drop for Undo {
    fn drop(&mut self) {
        stop(&self.stream, self.video.as_ref());
    }
}

/// Let go of the camera and take the video off the page.
fn stop(stream: &MediaStream, video: Option<&HtmlVideoElement>) {
    for t in stream.get_tracks().iter() {
        if let Ok(t) = t.dyn_into::<MediaStreamTrack>() {
            t.stop();
        }
    }
    if let Some(v) = video {
        v.set_src_object(None);
        v.remove();
    }
}

/// What the frame callback last saw: the frame's media time, s, and frames shown so far.
type Shown = Option<(f64, u64)>;

/// Called by the video with the time and the frame's details, for each frame it shows.
type FrameCallback = Closure<dyn FnMut(f64, JsValue)>;

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
    /// The controls asked for so far. One the camera refuses is let go, so it does not fail
    /// the next.
    wanted: Rc<RefCell<Vec<ControlSetting>>>,
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
        let slot: Rc<RefCell<Option<FrameCallback>>> = Rc::new(RefCell::new(None));
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

    /// Send the open constraints again with every control asked for so far.
    fn send_controls(&self) -> Result<Promise, CameraError> {
        let caps = call0(&self.track, "getCapabilities").unwrap_or(JsValue::UNDEFINED);
        let advanced = Object::new();
        for (key, value) in browser::control_fields(&self.wanted.borrow(), |key| range(&caps, key)) {
            set_value(&advanced, key, value);
        }
        let all = Object::assign(&Object::new(), &self.base);
        set(&all, "advanced", Array::of1(&advanced));
        self.track.apply_constraints_with_constraints(all.unchecked_ref()).map_err(|e| js_err("setting a control", &e))
    }

    /// Control changes the browser has not finished; their answers come through
    /// [`FrameSource::settled`].
    pub fn busy(&self) -> bool {
        self.pending.get() > 0
    }

    /// The browser's track, for checks that read it directly.
    pub fn track(&self) -> &MediaStreamTrack {
        &self.track
    }
}

/// A control's range and state, read from the track.
fn read_control(track: &MediaStreamTrack, id: ControlId) -> Option<ControlInfo> {
    let (mode_key, value_key, unit) = browser::keys(id)?;
    let caps = call0(track, "getCapabilities")?;
    let now = call0(track, "getSettings")?;
    let step = get(&caps, value_key).and_then(|r| num(&r, "step"));
    let auto = mode_key.and_then(|key| {
        let modes: Array = get(&caps, key)?.dyn_into().ok()?;
        let offered: Vec<String> = modes.iter().filter_map(|m| m.as_string()).collect();
        browser::auto_state(&offered, get(&now, key).and_then(|m| m.as_string()).as_deref())
    });
    browser::control_info(id, unit, range(&caps, value_key)?, step, num(&now, value_key), auto)
}

fn request_frame(video: &HtmlVideoElement, cb: &FrameCallback) {
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
        browser::CONTROLS.iter().filter_map(|c| read_control(&self.track, c.0)).collect()
    }

    /// Asks the browser for the setting and answers with it as asked; the browser's own answer,
    /// read back from the track, comes later through [`FrameSource::settled`].
    fn set_control(&mut self, setting: ControlSetting) -> Result<ControlInfo, CameraError> {
        let mut info = read_control(&self.track, setting.id).ok_or(CameraError::NoControl(setting.id))?;
        match setting.mode {
            ControlMode::Auto if info.auto.is_none() => {
                return Err(CameraError::Os(format!("the camera has no automatic {}", setting.id.label().to_lowercase())));
            }
            ControlMode::Auto => info.auto = Some(true),
            ControlMode::Manual(v) => {
                info.value = info.snap(v);
                info.auto = info.auto.map(|_| false);
            }
        }
        let asked = ControlSetting {
            mode: if let ControlMode::Manual(_) = setting.mode {
                ControlMode::Manual(info.value)
            } else {
                ControlMode::Auto
            },
            ..setting
        };
        browser::want(&mut self.wanted.borrow_mut(), asked);
        let promise = match self.send_controls() {
            Ok(p) => p,
            Err(e) => {
                browser::refused(&mut self.wanted.borrow_mut(), asked);
                return Err(e);
            }
        };
        // The handle, not the track's own `clone`, which starts a second track.
        let track = Clone::clone(&self.track);
        let settled = Rc::clone(&self.settled);
        let pending = Rc::clone(&self.pending);
        let wanted = Rc::clone(&self.wanted);
        pending.set(pending.get() + 1);
        wasm_bindgen_futures::spawn_local(async move {
            let result = JsFuture::from(promise).await;
            pending.set(pending.get().saturating_sub(1));
            let back = match result {
                Ok(_) => read_control(&track, setting.id).ok_or(CameraError::NoControl(setting.id)),
                Err(e) => {
                    browser::refused(&mut wanted.borrow_mut(), asked);
                    Err(js_err("setting a control", &e))
                }
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

impl Drop for WebCamera {
    fn drop(&mut self) {
        self.stopped.set(true);
        stop(&self.stream, Some(&self.video));
    }
}

/// BT.601 luma from RGBA, which is how the browser's YUV to RGB step was made for webcams.
pub fn luma(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .map(|p| ((77 * u32::from(p[0]) + 150 * u32::from(p[1]) + 29 * u32::from(p[2]) + 128) >> 8) as u8)
        .collect()
}
