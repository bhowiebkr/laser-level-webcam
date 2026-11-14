//! The checks, run in the browser on the app's own camera code.
//!
//! Each result goes to the page and, as one line, to the server that runs the check:
//! `PASS name: detail`, `FAIL name: detail` or `NOTE name: detail`, then `DONE pass=N fail=M`.

use std::cell::Cell;
use std::fmt::Display;

use js_sys::{Date, Function, JSON, Promise, Reflect};
use laser_camera::web::{self, WebCamera};
use laser_camera::{Applied, CameraError, FrameSource};
use laser_core::camera::{
    CameraSettings, ControlId, ControlInfo, ControlMode, ControlSetting, Mains, VideoMode, for_measuring,
};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

thread_local! {
    /// Checks passed and failed so far.
    static COUNTS: Cell<(u32, u32)> = const { Cell::new((0, 0)) };
}

/// Frames read in each frame check.
const FRAMES: usize = 10;

pub fn start() {
    std::panic::set_hook(Box::new(|info| {
        fail("panic", info);
        done();
    }));
    wasm_bindgen_futures::spawn_local(async {
        run(query("camera").as_deref()).await;
        done();
    });
}

/// Show a line on the page and send it to the server. The request waits for the answer, so the
/// lines arrive in order, and a panic can still send one.
fn post(line: &str) {
    let document = web_sys::window().and_then(|w| w.document());
    if let Some(log) = document.and_then(|d| d.get_element_by_id("log")) {
        let text = log.text_content().unwrap_or_default();
        log.set_text_content(Some(&format!("{text}{line}\n")));
    }
    if let Ok(xhr) = web_sys::XmlHttpRequest::new()
        && xhr.open_with_async("POST", "/result", false).is_ok()
    {
        let _ = xhr.send_with_opt_str(Some(line));
    }
}

fn count(passed: bool) {
    let (p, f) = COUNTS.get();
    COUNTS.set(if passed { (p + 1, f) } else { (p, f + 1) });
}

fn pass(name: &str, detail: impl Display) {
    count(true);
    post(&format!("PASS {name}: {detail}"));
}

fn fail(name: &str, detail: impl Display) {
    count(false);
    post(&format!("FAIL {name}: {detail}"));
}

fn note(name: &str, detail: impl Display) {
    post(&format!("NOTE {name}: {detail}"));
}

fn check(ok: bool, name: &str, detail: impl Display) -> bool {
    if ok {
        pass(name, detail)
    } else {
        fail(name, detail)
    }
    ok
}

fn done() {
    let (p, f) = COUNTS.get();
    post(&format!("DONE pass={p} fail={f}"));
}

/// A value from the page's address, such as `?camera=logi`.
fn query(key: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search).ok()?.get(key).filter(|v| !v.is_empty())
}

async fn sleep(ms: i32) {
    let promise = Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = JsFuture::from(promise).await;
}

/// A browser object as JSON, by calling one of its methods.
fn call_json(obj: &JsValue, method: &str) -> String {
    Reflect::get(obj, &method.into())
        .ok()
        .and_then(|f| f.dyn_into::<Function>().ok())
        .and_then(|f| f.call0(obj).ok())
        .and_then(|v| JSON::stringify(&v).ok())
        .and_then(|s| s.as_string())
        .unwrap_or_else(|| "(none)".into())
}

fn settings(device: Option<&str>, mode: Option<VideoMode>, controls: Vec<ControlSetting>) -> CameraSettings {
    CameraSettings { device: device.map(str::to_owned), mode, mains: Mains::Hz50, controls }
}

async fn open(settings: &CameraSettings) -> Result<WebCamera, CameraError> {
    web::open(settings, || {}).await.map(|(cam, _)| cam)
}

fn size(m: &VideoMode) -> String {
    format!("{}x{}", m.width, m.height)
}

fn describe(cam: &WebCamera) -> String {
    let m = cam.mode();
    let modes: Vec<String> = cam.modes().iter().map(size).collect();
    format!("{} at {} {:.0} fps; modes {}", cam.device().name, size(&m), m.fps, modes.join(", "))
}

fn show_mode(mode: ControlMode) -> String {
    match mode {
        ControlMode::Auto => "automatic".into(),
        ControlMode::Manual(v) => v.to_string(),
    }
}

fn show_info(c: &ControlInfo) -> String {
    let auto = match c.auto {
        Some(true) => " automatic",
        Some(false) => " manual",
        None => "",
    };
    format!("{} {}..{} step {} now {}{auto}", c.id.label(), c.min, c.max, c.step, c.value)
}

fn show_applied(a: &Applied) -> String {
    match &a.result {
        Ok(info) => format!("asked {}, read back {}", show_mode(a.setting.mode), show_info(info)),
        Err(e) => format!("asked {}: {e}", show_mode(a.setting.mode)),
    }
}

/// Wait for the browser to finish the control changes asked for, and take its answers.
async fn settle(cam: &mut WebCamera) -> Vec<Applied> {
    let start = Date::now();
    while cam.busy() && Date::now() - start < 5000.0 {
        sleep(20).await;
    }
    if cam.busy() {
        note("settle", "the browser took longer than 5 s");
    }
    cam.settled()
}

async fn run(only: Option<&str>) {
    match web::rescan().await {
        // Before the user allows a camera, the browser gives no names.
        Ok(()) => pass("list cameras", format!("{} named before the camera is allowed", web::devices().len() - 1)),
        Err(e) => fail("list cameras", e),
    }
    match open(&settings(None, None, Vec::new())).await {
        Ok(cam) => {
            pass("open the browser's pick", describe(&cam));
            drop(cam);
        }
        Err(e) => {
            fail("open the browser's pick", e);
            return;
        }
    }
    sleep(500).await;
    let names: Vec<String> = web::devices()
        .into_iter()
        .map(|d| d.name)
        .filter(|n| n != web::ANY)
        .filter(|n| only.is_none_or(|o| n.to_lowercase().contains(&o.to_lowercase())))
        .collect();
    check(!names.is_empty(), "cameras named once allowed", names.join(", "));
    for name in names {
        camera(&name).await;
        sleep(500).await;
    }
}

/// Every check on one camera.
async fn camera(name: &str) {
    let tag = |what: &str| format!("{name}: {what}");
    let mut cam = match open(&settings(Some(name), None, Vec::new())).await {
        Ok(cam) => {
            pass(&tag("open"), describe(&cam));
            cam
        }
        Err(e) => {
            fail(&tag("open"), e);
            return;
        }
    };
    frames(&mut cam, &tag("frames")).await;
    note(&tag("browser capabilities"), call_json(cam.track(), "getCapabilities"));
    note(&tag("browser settings"), call_json(cam.track(), "getSettings"));
    let infos = cam.controls();
    let shown: Vec<String> = infos.iter().map(show_info).collect();
    note(&tag("controls"), if shown.is_empty() { "none".into() } else { shown.join("; ") });
    for info in &infos {
        control(&mut cam, info, &tag(info.id.label())).await;
    }

    let lock = for_measuring(&cam.controls(), Mains::Hz50);
    let asked: Vec<String> = lock.iter().map(|s| format!("{} {}", s.id.label(), show_mode(s.mode))).collect();
    note(&tag("lock for measuring"), if asked.is_empty() { "nothing to lock".into() } else { asked.join(", ") });
    let _ = laser_camera::apply(&mut cam, &lock);
    let back = settle(&mut cam).await;
    all_held(&tag("lock"), &lock, &back);
    sleep(1000).await;
    still_held(&tag("lock 1 s later"), &lock, &cam.controls());
    frames(&mut cam, &tag("frames when locked")).await;

    // The handle: the track's own `clone` would start a second track.
    let track = Clone::clone(cam.track());
    let device = cam.device().name.clone();
    let modes = cam.modes().to_vec();
    drop(cam);
    let ended = track.ready_state() == web_sys::MediaStreamTrackState::Ended;
    check(ended, &tag("close stops the camera"), format!("track {:?}", track.ready_state()));
    let videos = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector_all("video").ok())
        .map_or(u32::MAX, |l| l.length());
    check(videos == 0, &tag("close takes the video off the page"), format!("{videos} left"));

    sleep(500).await;
    match open(&settings(Some(&device), None, lock.clone())).await {
        Ok(mut cam) => {
            let back = settle(&mut cam).await;
            all_held(&tag("open again with the lock"), &lock, &back);
        }
        Err(e) => fail(&tag("open again with the lock"), e),
    }

    for mode in &modes {
        sleep(500).await;
        let what = tag(&format!("open {}", size(mode)));
        match open(&settings(Some(&device), Some(*mode), Vec::new())).await {
            Ok(mut cam) => {
                let got = cam.mode();
                let ran = format!("runs at {} {:.0} fps", size(&got), got.fps);
                if (got.width, got.height) == (mode.width, mode.height) {
                    pass(&what, ran);
                } else {
                    // The browser does not list the camera's own sizes: one it lacks must
                    // drop out of the list.
                    let offered = cam.modes().iter().any(|m| (m.width, m.height) == (mode.width, mode.height));
                    let listed: Vec<String> = cam.modes().iter().map(size).collect();
                    check(!offered, &what, format!("the camera has no such size; {ran}; offers {}", listed.join(", ")));
                }
                frames(&mut cam, &format!("{what} frames")).await;
            }
            Err(e) => fail(&what, e),
        }
    }
}

/// Read frames: each the mode's size, in order by count and time.
async fn frames(cam: &mut WebCamera, what: &str) {
    let mode = cam.mode();
    let (w, h) = (mode.width as usize, mode.height as usize);
    let mut got: Vec<(u64, f64, f64)> = Vec::new();
    let start = Date::now();
    while got.len() < FRAMES && Date::now() - start < 5000.0 {
        match cam.try_frame() {
            Ok(Some(f)) => {
                if (f.width, f.height) != (w, h) || f.data.len() != w * h {
                    fail(
                        what,
                        format!("a {}x{} frame of {} bytes in mode {}", f.width, f.height, f.data.len(), size(&mode)),
                    );
                    return;
                }
                let mean = f.data.iter().map(|&v| u64::from(v)).sum::<u64>() as f64 / f.data.len() as f64;
                got.push((f.seq, f.t_s, mean));
            }
            Ok(None) => sleep(5).await,
            Err(e) => {
                fail(what, e);
                return;
            }
        }
    }
    let (Some(first), Some(last)) = (got.first(), got.last()) else {
        fail(what, "no frame in 5 s");
        return;
    };
    if got.len() < FRAMES {
        fail(what, format!("{} frames in 5 s", got.len()));
        return;
    }
    let ordered = got.windows(2).all(|p| p[1].0 > p[0].0 && p[1].1 >= p[0].1);
    let span = last.1 - first.1;
    let fps = if span > 0.0 { (got.len() - 1) as f64 / span } else { 0.0 };
    let mean = got.iter().map(|g| g.2).sum::<f64>() / got.len() as f64;
    check(
        ordered,
        what,
        format!("{FRAMES} frames, seq {}..{}, {fps:.0} fps by their times, mean grey {mean:.0}", first.0, last.0),
    );
}

/// Set one control through its range and back.
async fn control(cam: &mut WebCamera, info: &ControlInfo, what: &str) {
    let span = info.max - info.min;
    let mut tries = vec![
        ("lowest", ControlMode::Manual(info.min)),
        ("highest", ControlMode::Manual(info.max)),
        ("middle", ControlMode::Manual(info.snap(info.min + span / 2))),
        ("below the range", ControlMode::Manual(info.min - span - 10)),
        ("above the range", ControlMode::Manual(info.max + span + 10)),
    ];
    // Chrome's test camera lists an automatic mode but stays manual.
    if info.auto.is_some() && what.starts_with("fake_device") {
        note(&format!("{what} automatic"), "not tried: Chrome's test camera stays manual");
    } else if info.auto.is_some() {
        tries.push(("automatic", ControlMode::Auto));
    }
    for (label, mode) in tries {
        set(cam, ControlSetting { id: info.id, mode }, &format!("{what} {label}")).await;
    }
    let before = if info.auto == Some(true) { ControlMode::Auto } else { ControlMode::Manual(info.value) };
    set(cam, ControlSetting { id: info.id, mode: before }, &format!("{what} put back")).await;
}

/// Set a control, check the browser's answer, and check it a moment later.
async fn set(cam: &mut WebCamera, setting: ControlSetting, what: &str) {
    if let Err(e) = cam.set_control(setting) {
        fail(what, e);
        return;
    }
    let back = settle(cam).await;
    let Some(applied) = back.iter().rev().find(|a| a.setting.id == setting.id).cloned() else {
        fail(what, "the browser gave no answer");
        return;
    };
    sleep(400).await;
    let later = cam.controls().into_iter().find(|c| c.id == setting.id).ok_or(CameraError::NoControl(setting.id));
    let stayed = Applied { setting: applied.setting, result: later };
    match (applied.held(), stayed.held()) {
        (true, true) => pass(what, show_applied(&stayed)),
        (true, false) => fail(what, format!("did not stay: {}", show_applied(&stayed))),
        (false, true) => fail(what, format!("first answer {}, then {}", show_applied(&applied), show_applied(&stayed))),
        (false, false) => {
            fail(what, format!("{}; browser settings {}", show_applied(&stayed), call_json(cam.track(), "getSettings")))
        }
    }
}

fn find(back: &[Applied], id: ControlId) -> Option<&Applied> {
    back.iter().rev().find(|a| a.setting.id == id)
}

fn all_held(what: &str, lock: &[ControlSetting], back: &[Applied]) {
    for s in lock {
        let name = format!("{what}: {}", s.id.label());
        match find(back, s.id) {
            Some(a) => {
                check(a.held(), &name, show_applied(a));
            }
            None => fail(&name, "the browser gave no answer"),
        }
    }
}

fn still_held(what: &str, lock: &[ControlSetting], infos: &[ControlInfo]) {
    for s in lock {
        let result = infos.iter().find(|c| c.id == s.id).copied().ok_or(CameraError::NoControl(s.id));
        let a = Applied { setting: *s, result };
        check(a.held(), &format!("{what}: {}", s.id.label()), show_applied(&a));
    }
}
