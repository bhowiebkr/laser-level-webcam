//! The settings panel of the set-up step: camera, orientation, sensor and beam finder. The other
//! steps keep their few settings on their own screens.

use eframe::egui::{self, RichText};
use laser_core::camera::{
    self, ControlId, ControlInfo, ControlMode, ControlSetting, ControlUnit, Mains, ModeScaling, PixelFormat, VideoMode,
};
use laser_core::frame::Roi;

use crate::app::{App, SensorAction};
use crate::driver::Request;
use crate::engine::{CameraState, CameraView, Engine};
use crate::{sensors, setup, theme, visuals};

pub fn draw(app: &mut App, ui: &mut egui::Ui, e: &mut Engine) {
    section(ui, "Camera", true, CAMERA_INTRO, |ui| camera_section(app, ui, e));
    section(ui, "Orientation", true, ORIENTATION_INTRO, |ui| orientation_section(app, ui, e));
    section(ui, "Sensor", false, SENSOR_INTRO, |ui| sensor_section(app, ui, e));
    section(ui, "Beam finder", false, FINDER_INTRO, |ui| finder_section(app, ui, e));
}

const CAMERA_INTRO: &str = "The camera the laser line falls on, and how its frames reach this app. \
Pick the camera and a mode, press Lock for measuring so nothing automatic changes the beam while you \
measure, then set the exposure so the beam's peak sits just under clipping.";
const ORIENTATION_INTRO: &str = "Which way the camera's picture is turned, so the laser line runs \
across the view and heights are read up and down it. Turn it until the line lies level, then check \
the sign with a shim under the block as shown below.";
const SENSOR_INTRO: &str = "The camera chip's real size, which turns pixels into micrometres: every \
reading depends on it. Enter the pixel pitch and full resolution from the sensor's data sheet, and \
say whether the camera's smaller modes scale the whole chip down or crop a window of it.";
const FINDER_INTRO: &str = "How the beam's centre is worked out in each frame. The defaults suit a \
clean line: raise Least signal if noise is read as a beam, and drag on the live view to leave out \
dust or reflections.";

fn section(ui: &mut egui::Ui, title: &str, open: bool, intro: &str, body: impl FnOnce(&mut egui::Ui)) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::CollapsingHeader::new(theme::heading(title)).default_open(open).show_unindented(ui, |ui| {
            ui.label(theme::dim(intro));
            ui.add_space(4.0);
            body(ui);
        });
    });
    ui.add_space(4.0);
}

/// Where a mode's frames come from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    /// macOS supplies decoded luma; the USB encoding is not exposed here.
    #[cfg(target_os = "macos")]
    MacOS,
    /// The camera sends it: it is in the camera's USB descriptors.
    Camera,
    /// Windows makes it, by decoding the camera's MJPEG.
    Windows,
}

fn source_of(m: &VideoMode, cam: &CameraView) -> Source {
    #[cfg(target_os = "macos")]
    if cam.device.as_ref().is_some_and(|d| d.backend == laser_camera::Backend::AvFoundation) {
        return Source::MacOS;
    }
    let made = match &cam.usb {
        Some(u) => !u.sends(m),
        // No descriptors: an NV12 mode with an MJPEG twin is Windows decoding it.
        None => {
            m.format == PixelFormat::Nv12
                && cam.modes.iter().any(|o| {
                    o.format == PixelFormat::Mjpeg
                        && (o.width, o.height) == (m.width, m.height)
                        && (o.fps - m.fps).abs() < 0.5
                })
        }
    };
    if made { Source::Windows } else { Source::Camera }
}

fn aspect(w: u32, h: u32) -> String {
    let r = f64::from(w) / f64::from(h.max(1));
    let known = [
        ("16:9", 16.0 / 9.0),
        ("16:10", 1.6),
        ("5:3", 5.0 / 3.0),
        ("3:2", 1.5),
        ("4:3", 4.0 / 3.0),
        ("5:4", 1.25),
        ("11:9", 11.0 / 9.0),
    ];
    known.iter().find(|(_, v)| (r / v - 1.0).abs() < 0.015).map_or_else(|| format!("{r:.2}:1"), |(n, _)| (*n).into())
}

fn format_text(f: PixelFormat, source: Source) -> (&'static str, &'static str) {
    match (f, source) {
        #[cfg(target_os = "macos")]
        (PixelFormat::Yuyv, Source::MacOS) => (
            "YUYV source, macOS luma",
            "AVFoundation reports a YUYV source format and supplies NV12 luma. USB transport descriptors are unavailable; this is not raw sensor data.",
        ),
        #[cfg(target_os = "macos")]
        (PixelFormat::Mjpeg, Source::MacOS) => (
            "MJPEG, decoded by macOS",
            "AVFoundation decodes this source format and supplies NV12 luma. JPEG compression can change the beam profile.",
        ),
        #[cfg(target_os = "macos")]
        (PixelFormat::Nv12, Source::MacOS) => (
            "NV12, supplied by macOS",
            "Luma from AVFoundation. The camera's USB encoding is unknown and may include compression; this is not raw sensor data.",
        ),
        (PixelFormat::Yuyv, _) => (
            "Uncompressed (YUYV)",
            "Every pixel as the camera's processor puts it out, nothing compressed: the best for measuring. Large sizes only come slowly, because the USB link carries no more.",
        ),
        (PixelFormat::Nv12, Source::Windows) => (
            "NV12, decoded by Windows",
            "Windows decodes the camera's MJPEG and hands it on as NV12. The JPEG compression is still in it: it is not raw.",
        ),
        (PixelFormat::Nv12, Source::Camera) => {
            ("Uncompressed (NV12)", "Every pixel as the camera puts it out, nothing compressed.")
        }
        (PixelFormat::Grey, _) => ("Uncompressed grey", "Every pixel as the camera puts it out, nothing compressed."),
        (PixelFormat::Mjpeg, _) => (
            "MJPEG",
            "Each frame compressed to a JPEG in the camera, so full size comes fast. JPEG works in 8×8 blocks and can bend the beam's edges. This app decodes it itself.",
        ),
        (PixelFormat::Rgba, _) => ("RGBA", "Colour from the browser, mixed back to grey."),
    }
}

/// The mode among `modes` closest to `cur`: the nearest size first, then the nearest frame rate.
fn nearest(modes: impl Iterator<Item = VideoMode>, cur: &VideoMode) -> Option<VideoMode> {
    let cost = |m: &VideoMode| {
        let size = (m.pixels() as f64 / cur.pixels().max(1) as f64).ln().abs();
        size * 1000.0 + (m.fps - cur.fps).abs() - m.fps * 1e-3
    };
    modes.min_by(|a, b| cost(a).total_cmp(&cost(b)))
}

/// Source, format, size and frame rate, from the modes the camera opened with.
fn mode_picker(app: &mut App, ui: &mut egui::Ui, cam: &CameraView) {
    let Some(cur) = cam.mode else { return };
    let src = source_of(&cur, cam);
    let in_src: Vec<VideoMode> = cam.modes.iter().copied().filter(|m| source_of(m, cam) == src).collect();
    let windows = cam.modes.iter().any(|m| source_of(m, cam) == Source::Windows);
    let mut pick: Option<VideoMode> = None;

    // One row per choice, the label first. Rows wrap in a narrow panel rather than overlap.
    if windows {
        ui.horizontal_wrapped(|ui| {
            ui.label("Frames from").on_hover_text(
                "Who makes the frames. The camera's own list is read from its USB descriptors; Windows adds modes it makes by decoding the camera's MJPEG.",
            );
            let choices = [
                (Source::Camera, "The camera", "Only what the camera itself sends over the cable."),
                (
                    Source::Windows,
                    "Windows",
                    "Modes Windows makes from the camera's MJPEG: the same picture as the camera's MJPEG, decoded by Windows. Not raw.",
                ),
            ];
            for (s, text, help) in choices {
                if ui.selectable_label(src == s, text).on_hover_text(help).clicked() && src != s {
                    let all: Vec<VideoMode> = cam.modes.iter().copied().filter(|m| source_of(m, cam) == s).collect();
                    let raw = all.iter().any(|m| m.format.is_raw_luma());
                    pick = nearest(all.into_iter().filter(|m| !raw || m.format.is_raw_luma()), &cur);
                }
            }
        });
    }

    let mut formats: Vec<PixelFormat> = Vec::new();
    for m in &in_src {
        if !formats.contains(&m.format) {
            formats.push(m.format);
        }
    }
    formats.sort_by_key(|f| !f.is_raw_luma());
    ui.horizontal_wrapped(|ui| {
        ui.label("Format").on_hover_text("How the pixels travel from the camera.");
        for f in formats {
            let (text, help) = format_text(f, src);
            if ui.selectable_label(cur.format == f, text).on_hover_text(help).clicked() && cur.format != f {
                pick = nearest(in_src.iter().copied().filter(|m| m.format == f), &cur);
            }
        }
    });

    let of_format: Vec<VideoMode> = in_src.iter().copied().filter(|m| m.format == cur.format).collect();
    let mut sizes: Vec<(u32, u32)> = Vec::new();
    for m in &of_format {
        if !sizes.contains(&(m.width, m.height)) {
            sizes.push((m.width, m.height));
        }
    }
    sizes.sort_by_key(|&(w, h)| std::cmp::Reverse(u64::from(w) * u64::from(h)));
    let size_text = |(w, h): (u32, u32)| format!("{w}×{h}   {}", aspect(w, h));
    ui.horizontal(|ui| {
        ui.label("Size").on_hover_text(
            "Frame size in pixels, and its shape. A size of another shape can show a different part of the sensor.",
        );
        egui::ComboBox::from_id_salt("mode_size")
            .width(ui.available_width())
            .selected_text(size_text((cur.width, cur.height)))
            .show_ui(ui, |ui| {
                for wh in sizes {
                    if ui.selectable_label((cur.width, cur.height) == wh, size_text(wh)).clicked()
                        && (cur.width, cur.height) != wh
                    {
                        pick = nearest(of_format.iter().copied().filter(|m| (m.width, m.height) == wh), &cur);
                    }
                }
            });
    });

    let mut rates: Vec<VideoMode> =
        of_format.iter().copied().filter(|m| (m.width, m.height) == (cur.width, cur.height)).collect();
    rates.sort_by(|a, b| b.fps.total_cmp(&a.fps));
    ui.horizontal_wrapped(|ui| {
        ui.label("Frame rate").on_hover_text("The frame rates the camera lists for this format and size.");
        for m in rates {
            if ui.selectable_label(m == cur, format!("{:.0} fps", m.fps)).clicked() && m != cur {
                pick = Some(m);
            }
        }
    });

    if let Some(u) = &cam.usb
        && src == Source::Camera
        && cur.format.is_raw_luma()
        && let (Some(need), Some(have)) = (u.raw_bytes_per_s(&cur), u.max_bytes_per_s)
    {
        let frac = need as f32 / have.max(1) as f32;
        ui.add(egui::ProgressBar::new(frac.min(1.0)).text(format!(
            "{:.1} of the {:.1} MB/s the USB link carries",
            need as f64 / 1e6,
            have as f64 / 1e6
        )))
        .on_hover_text(
            "Uncompressed frames have to fit the camera's share of the USB link. That is why a large size only comes at a low frame rate.",
        );
    }
    if let Some(u) = &cam.usb {
        usb_report(ui, u);
    }
    if let Some(m) = pick {
        app.settings.camera.mode = Some(m);
        app.request(Request::Open(app.settings.camera.clone()));
    }
}

/// The camera's own list, from its USB descriptors.
fn usb_report(ui: &mut egui::Ui, u: &laser_core::uvc::UvcDevice) {
    use laser_core::uvc::UvcEncoding;
    egui::CollapsingHeader::new("What the camera reports").id_salt("usb_report").show(ui, |ui| {
        let link = u
            .max_bytes_per_s
            .map_or_else(|| "a bulk video link".to_owned(), |b| format!("video link up to {:.1} MB/s", b as f64 / 1e6));
        ui.label(theme::dim(format!(
            "USB {:04x}:{:04x}, {}, {link}.",
            u.vendor_id,
            u.product_id,
            u.speed.map_or("speed unknown", |s| s.name()),
        )));
        ui.label(theme::dim(
            "Read from the camera's USB descriptors: every format, size and frame rate it sends itself. Anything else Windows lists is made from these.",
        ));
        let bayer = ["BA81", "GBRG", "GRBG", "RGGB", "BYR2", "BG16", "GB16", "GR16", "RG16"];
        let raw_sensor = u.formats.iter().any(
            |f| matches!(&f.encoding, UvcEncoding::Uncompressed { fourcc } if bayer.contains(&fourcc.trim_end())),
        );
        if !raw_sensor {
            ui.label(theme::dim(
                "It lists no raw sensor (Bayer) format. The least processed it sends is uncompressed, after the camera's own processing; sharpness, gamma, gain and the other controls set that processing.",
            ));
        }
        egui::Grid::new("usb_formats").num_columns(3).striped(true).spacing([12.0, 2.0]).show(ui, |ui| {
            ui.label(RichText::new("Format").strong());
            ui.label(RichText::new("Size").strong());
            ui.label(RichText::new("fps").strong());
            ui.end_row();
            for f in &u.formats {
                let name = match &f.encoding {
                    UvcEncoding::Uncompressed { fourcc } => format!("{} {} bit", fourcc.trim_end(), f.bits_per_pixel),
                    UvcEncoding::Mjpeg => "MJPEG".into(),
                    UvcEncoding::FrameBased { fourcc } => fourcc.trim_end().to_owned(),
                };
                for fr in &f.frames {
                    ui.label(&name);
                    ui.label(format!(
                        "{}×{}  {}",
                        fr.width,
                        fr.height,
                        aspect(fr.width.into(), fr.height.into())
                    ));
                    let fps: Vec<String> = fr.fps.iter().map(|r| format!("{r:.0}")).collect();
                    ui.label(fps.join(" "));
                    ui.end_row();
                }
            }
        });
        egui::CollapsingHeader::new("Descriptor bytes").id_salt("usb_bytes").show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                ui.label(RichText::new(u.hex()).monospace().small());
            });
        });
    });
}

fn camera_section(app: &mut App, ui: &mut egui::Ui, e: &mut Engine) {
    let current = app.settings.camera.device.clone().or_else(|| e.camera.device.as_ref().map(|d| d.name.clone()));
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Rescan").on_hover_text("Look for cameras again").clicked() {
                // The browser lists its cameras later; the list is read again each frame there.
                #[cfg(target_arch = "wasm32")]
                {
                    let ctx = ui.ctx().clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        let _ = laser_camera::web::rescan().await;
                        ctx.request_repaint();
                    });
                }
                app.devices = laser_camera::devices();
            }
            egui::ComboBox::from_id_salt("camera")
                .width(ui.available_width())
                .selected_text(current.clone().unwrap_or_else(|| "First camera".into()))
                .show_ui(ui, |ui| {
                    for d in app.devices.clone() {
                        if ui.selectable_label(current.as_deref() == Some(d.name.as_str()), &d.name).clicked() {
                            app.settings.camera.device = Some(d.name.clone());
                            app.settings.camera.mode = None;
                            app.request(Request::Open(app.settings.camera.clone()));
                        }
                    }
                });
        });
    });
    if !e.camera.modes.is_empty() {
        mode_picker(app, ui, &e.camera);
    }
    ui.horizontal(|ui| {
        ui.label("Mains").on_hover_text(ControlId::PowerLineFrequency.help());
        for (m, text) in [(Mains::Hz50, "50 Hz"), (Mains::Hz60, "60 Hz")] {
            if ui.selectable_label(app.settings.camera.mains == m, text).clicked() && app.settings.camera.mains != m {
                app.settings.camera.mains = m;
                if e.camera.control(ControlId::PowerLineFrequency).is_some() {
                    set_control(app, ControlId::PowerLineFrequency, ControlMode::Manual(m.menu_value()));
                }
            }
        }
    });
    match &e.camera.state {
        CameraState::Open => {}
        CameraState::Failed(why) => {
            ui.label(RichText::new(why).color(theme::ERROR));
            if ui.button("Try again").clicked() {
                app.request(Request::Open(app.settings.camera.clone()));
            }
            return;
        }
        _ => return,
    }

    ui.add_space(4.0);
    #[cfg(target_os = "macos")]
    if e.camera.device.as_ref().is_some_and(|d| d.backend == laser_camera::Backend::AvFoundation) {
        ui.label(theme::dim(if e.camera.controls.is_empty() {
            "This camera exposes no lock controls on macOS. Automatic image changes cannot be ruled out."
        } else {
            "macOS can lock only the controls listed below. Numeric adjustment and unlisted controls are unavailable."
        }));
    }
    let lock = ui
        .add_enabled(!e.camera.controls.is_empty(), theme::primary("Lock for measuring"))
        .on_hover_text("Lock the controls exposed by this camera, turn sharpening off and set the mains filter where supported. Unlisted controls cannot be verified.");
    if lock.clicked() {
        let settings = camera::for_measuring(&e.camera.controls, app.settings.camera.mains);
        for s in &settings {
            app.settings.camera.set(s.id, s.mode);
        }
        app.request(Request::Apply(settings));
    }
    let autos: Vec<&str> = e.camera.controls.iter().filter(|c| c.auto == Some(true)).map(|c| c.id.label()).collect();
    if !autos.is_empty() {
        ui.label(RichText::new(format!("Still automatic: {}", autos.join(", "))).small().color(theme::ACCENT));
    }
    for a in &e.camera.refused {
        let why = match &a.result {
            Ok(info) => format!("{} did not hold (reads {})", a.setting.id.label(), info.value),
            Err(err) => err.to_string(),
        };
        ui.label(RichText::new(why).small().color(theme::ERROR));
    }

    if let Some(exposure) = e.camera.control(ControlId::Exposure).cloned() {
        let mode = e.camera.mode;
        let step = visuals::exposure(
            ui,
            &exposure,
            mode.as_ref(),
            app.settings.camera.mains,
            e.live.detail.as_ref(),
            app.settings.finder.clip_level,
        );
        if let Some(v) = step {
            set_control(app, ControlId::Exposure, ControlMode::Manual(v));
        }
    }

    ui.add_space(4.0);
    for info in e.camera.controls.clone() {
        if info.id == ControlId::PowerLineFrequency {
            continue;
        }
        control_row(app, ui, &info);
    }
    if ui.small_button("Read the controls again").clicked() {
        app.request(Request::Refresh);
    }
}

fn set_control(app: &mut App, id: ControlId, mode: ControlMode) {
    app.settings.camera.set(id, mode);
    app.request(Request::Set(ControlSetting { id, mode }));
}

fn control_row(app: &mut App, ui: &mut egui::Ui, info: &ControlInfo) {
    let id = info.id;
    let auto_on = info.auto == Some(true);
    // While dragging, show the value asked for rather than the last one read back.
    let mut v = match app.settings.camera.get(id) {
        Some(ControlMode::Manual(v)) if !auto_on => info.snap(v),
        _ => info.value,
    };
    let mut asked = None;
    ui.horizontal(|ui| {
        ui.label(id.label()).on_hover_text(id.help());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if info.auto.is_some()
                && ui.selectable_label(auto_on, "Auto").on_hover_text("Let the camera set it").clicked()
            {
                asked = Some(if auto_on { ControlMode::Manual(info.value) } else { ControlMode::Auto });
            }
        });
    });
    let exposure = *info;
    if info.unit == ControlUnit::ModeOnly {
        ui.label(theme::dim(if auto_on {
            "Automatic; click Auto to lock the current value"
        } else {
            "Current value locked; numeric adjustment unavailable"
        }));
        if let Some(mode) = asked {
            set_control(app, id, mode);
        }
        return;
    }
    let text = move |x: f64| match exposure.exposure_s(x as i64) {
        Some(s) if s < 1.0 => format!("{} ({:.2} ms)", x as i64, s * 1000.0),
        Some(s) => format!("{} ({s:.2} s)", x as i64),
        None => format!("{}", x as i64),
    };
    let widest = [text(info.min as f64), text(info.max as f64)];
    let slider = egui::Slider::new(&mut v, info.min..=info.max)
        .step_by(info.step.max(1) as f64)
        .custom_formatter(move |x, _| text(x));
    if theme::wide_slider(ui, !auto_on, &widest, slider).on_hover_text(id.help()).changed() {
        asked = Some(ControlMode::Manual(v));
    }
    if let Some(mode) = asked {
        set_control(app, id, mode);
    }
}

/// The sensors the user has saved: pick one, save this one, move the list to another computer.
fn saved_sensors(app: &mut App, ui: &mut egui::Ui) {
    let mut current = app.settings.sensor.clone();
    current.name = current.name.trim().to_owned();
    let saved = app.sensors.get(&current.name);
    let changed = saved.is_some_and(|s| s.sensor != current);
    let camera = app.camera_id.as_ref();
    let linked = saved.is_some_and(|s| camera.is_some_and(|c| s.cameras.iter().any(|k| k.matches(c))));
    let (mut pick, mut save, mut delete, mut action) = (None, false, false, None);
    ui.horizontal_wrapped(|ui| {
        ui.label("Saved");
        let shown = match (saved, app.sensors.sensors.is_empty()) {
            (_, true) => "None yet".to_owned(),
            (Some(_), false) => current.name.clone(),
            (None, false) => "Pick one".to_owned(),
        };
        egui::ComboBox::from_id_salt("saved_sensors")
            .selected_text(shown)
            .width(150.0)
            .show_ui(ui, |ui| {
                for s in &app.sensors.sensors {
                    let k = &s.sensor;
                    let text = format!("{}   {} µm, {} × {}", k.name, k.pixel_pitch_um, k.width_px, k.height_px);
                    let on = saved.is_some_and(|x| std::ptr::eq(x, s));
                    let names: Vec<&str> = s.cameras.iter().map(|c| c.name.as_str()).collect();
                    let hover = if names.is_empty() {
                        "Saved for no camera.".to_owned()
                    } else {
                        format!("Picked when {} opens.", names.join(" or "))
                    };
                    if ui.selectable_label(on, text).on_hover_text(hover).clicked() {
                        pick = Some(k.clone());
                    }
                }
            })
            .response
            .on_hover_text("The sensors you have saved. Pick one to use it.");
        let (label, enabled) = match saved {
            _ if current.name.is_empty() => ("Save", false),
            None => ("Save", true),
            Some(_) if changed => ("Save changes", true),
            Some(_) if camera.is_some() && !linked => ("Save for this camera", true),
            Some(_) => ("Saved", false),
        };
        let hover = match camera {
            _ if current.name.is_empty() => "Give the sensor a name first.".to_owned(),
            Some(c) => format!("Keep this sensor in the list. It is picked whenever {} opens.", c.name),
            None => "Keep this sensor in the list.".to_owned(),
        };
        save = ui.add_enabled(enabled, egui::Button::new(label)).on_hover_text(hover).clicked();
        delete = ui
            .add_enabled(saved.is_some(), egui::Button::new("Delete"))
            .on_hover_text("Take this sensor out of the list.")
            .clicked();
        if ui
            .add_enabled(!app.sensors.sensors.is_empty(), egui::Button::new("Export…"))
            .on_hover_text(
                "Save the list to a file: a copy to keep, or to take to another computer or the browser version.",
            )
            .clicked()
        {
            action = Some(SensorAction::Export);
        }
        if ui
            .button("Import…")
            .on_hover_text("Add the sensors from an exported list. A sensor of the same name is replaced.")
            .clicked()
        {
            action = Some(SensorAction::Import);
        }
    });
    let state = match (saved, camera) {
        _ if current.name.is_empty() => "Give the sensor a name to save it.".to_owned(),
        (None, _) => "Not saved yet.".to_owned(),
        _ if changed => "Changed since it was saved.".to_owned(),
        (Some(_), Some(c)) if linked => format!("Saved. Picked whenever {} opens.", c.name),
        (Some(_), Some(c)) => format!("Saved, but not for {}.", c.name),
        (Some(_), None) => "Saved.".to_owned(),
    };
    ui.label(theme::dim(state)).on_hover_text(sensors::place());
    if let Some(msg) = &app.sensor_msg {
        ui.label(theme::dim(msg));
    }
    if let Some(k) = pick {
        app.settings.sensor = k;
        app.sensor_msg = None;
    }
    if save {
        app.save_sensor();
    }
    if delete {
        app.delete_sensor(&current.name);
    }
    if action.is_some() {
        app.sensor_action = action;
    }
}

fn sensor_section(app: &mut App, ui: &mut egui::Ui, e: &Engine) {
    saved_sensors(app, ui);
    ui.add_space(6.0);
    let s = &mut app.settings.sensor;
    egui::Grid::new("sensor").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Name");
        ui.add(egui::TextEdit::singleline(&mut s.name).hint_text("from the data sheet").desired_width(f32::INFINITY));
        ui.end_row();
        ui.label("Pixel pitch").on_hover_text("Centre-to-centre distance of the sensor's pixels, from its data sheet.");
        ui.add(egui::DragValue::new(&mut s.pixel_pitch_um).range(0.3..=30.0).speed(0.01).suffix(" µm").max_decimals(3));
        ui.end_row();
        ui.label("Full size").on_hover_text("Active pixels at the sensor's full resolution.");
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut s.width_px).range(16..=20000));
            ui.label("×");
            ui.add(egui::DragValue::new(&mut s.height_px).range(16..=20000));
        });
        ui.end_row();
        ui.label("Smaller modes").on_hover_text("How the camera makes a mode smaller than the sensor.");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut s.scaling, ModeScaling::Scaled, "Scaled")
                .on_hover_text("The whole sensor, scaled down: a frame pixel covers several sensor pixels.");
            ui.selectable_value(&mut s.scaling, ModeScaling::Cropped, "Cropped")
                .on_hover_text("A window of the sensor: a frame pixel is a sensor pixel.");
        });
        ui.end_row();
    });
    ui.label(theme::dim(format!(
        "Sensor {:.2} × {:.2} mm, {:.3} µm per frame px",
        s.width_mm(),
        s.height_mm(),
        e.um_per_px()
    )));
    visuals::sensor(ui, &app.settings.sensor, e.camera.mode.as_ref(), app.settings.orientation.axis());
}

/// Which way round the camera's image is shown. Turned so the line runs level, the view reads
/// heights up and down it, and the finder and the sign of the heights follow.
fn orientation_section(app: &mut App, ui: &mut egui::Ui, e: &Engine) {
    ui.horizontal_wrapped(|ui| setup::orientation_buttons(ui, &mut app.settings.orientation));
    visuals::orientation(ui, &app.settings.sensor, e.camera.mode.as_ref(), app.settings.orientation);
}

fn finder_section(app: &mut App, ui: &mut egui::Ui, e: &Engine) {
    let f = &mut app.settings.finder;
    egui::Grid::new("finder").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Strips").on_hover_text("The frame is cut into strips along the line. Each strip gives a centre; a strip that disagrees (dust, a reflection) is dropped and the line's tilt comes from the rest.");
        theme::wide_slider(ui, true, &["64".into()], egui::Slider::new(&mut f.strips, 1..=64));
        ui.end_row();
        ui.label("Fit window").on_hover_text("Half width of the window the centre is worked out in, in beam sigmas. Wider uses more of the beam and more of the noise.");
        theme::wide_slider(ui, true, &["5.0 σ".into()], egui::Slider::new(&mut f.window_sigmas, 1.0..=5.0).fixed_decimals(1).suffix(" σ"));
        ui.end_row();
        ui.label("Background from").on_hover_text("The background is fitted outside this many beam sigmas from the centre.");
        theme::wide_slider(ui, true, &["8.0 σ".into()], egui::Slider::new(&mut f.background_sigmas, 2.0..=8.0).fixed_decimals(1).suffix(" σ"));
        ui.end_row();
        ui.label("Least signal").on_hover_text("A beam fainter than this many times the noise is no reading.");
        theme::wide_slider(ui, true, &["50".into()], egui::Slider::new(&mut f.min_snr, 3.0..=50.0).fixed_decimals(0));
        ui.end_row();
        ui.label("Clipped at").on_hover_text("Grey level counted as clipped. A clipped beam has a flat top and its centre is less sure.");
        theme::wide_slider(ui, true, &["255".into()], egui::Slider::new(&mut f.clip_level, 200..=255));
        ui.end_row();
    });
    egui::CollapsingHeader::new("Warning limits").show(ui, |ui| {
        egui::Grid::new("limits").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Clipped pixels")
                .on_hover_text("Share of the beam's pixels that may clip before the reading is flagged.");
            theme::wide_slider(
                ui,
                true,
                &["1.00 %".into()],
                egui::Slider::new(&mut f.max_clipped, 0.0..=0.01)
                    .custom_formatter(|v, _| format!("{:.2} %", v * 100.0)),
            );
            ui.end_row();
            ui.label("Tilt").on_hover_text("A line tilted more than this across the sensor is flagged.");
            theme::wide_slider(
                ui,
                true,
                &["100.0 mm/m".into()],
                egui::Slider::new(&mut f.max_tilt, 0.001..=0.1)
                    .custom_formatter(|v, _| format!("{:.1} mm/m", v * 1000.0)),
            );
            ui.end_row();
            ui.label("Beam width").on_hover_text("A beam wider than this share of the frame is no reading.");
            theme::wide_slider(
                ui,
                true,
                &["100 %".into()],
                egui::Slider::new(&mut f.max_width, 0.05..=1.0).custom_formatter(|v, _| format!("{:.0} %", v * 100.0)),
            );
            ui.end_row();
        });
    });
    let size = e.live.size.unwrap_or((1920, 1080));
    visuals::strips(ui, f, size);
    visuals::fit_zones(ui, f);
    let mut on = f.roi.is_some();
    if ui
        .checkbox(&mut on, "Use part of the frame")
        .on_hover_text("Drag on the live view to draw it; double-click the view for the whole frame.")
        .changed()
    {
        f.roi = on.then_some(Roi { x: size.0 / 4, y: 0, width: size.0 / 2, height: size.1 });
    }
    if let Some(r) = &mut f.roi {
        ui.horizontal(|ui| {
            ui.label("x");
            ui.add(egui::DragValue::new(&mut r.x).range(0..=size.0.saturating_sub(16)));
            ui.label("y");
            ui.add(egui::DragValue::new(&mut r.y).range(0..=size.1.saturating_sub(16)));
        });
        ui.horizontal(|ui| {
            ui.label("w");
            ui.add(egui::DragValue::new(&mut r.width).range(16..=size.0));
            ui.label("h");
            ui.add(egui::DragValue::new(&mut r.height).range(16..=size.1));
        });
    }
}
