//! The settings panel. It shows the settings of the current step: camera, sensor and finder while
//! setting up; the planned run while waiting; the sample rule while measuring; units and drift
//! correction for the results.

use eframe::egui::{self, RichText};
use laser_core::camera::{self, ControlId, ControlInfo, ControlMode, ControlSetting, Mains, ModeScaling, VideoMode};
use laser_core::frame::Roi;
use laser_core::survey::DriftCorrection;
use laser_core::units::Unit;

use crate::app::{App, Screen};
use crate::driver::Request;
use crate::engine::{CameraState, Engine};
use crate::{setup, theme, visuals};

pub fn draw(app: &mut App, ui: &mut egui::Ui, e: &mut Engine) {
    match app.screen {
        Screen::SetUp => {
            section(ui, "Camera", true, |ui| camera_section(app, ui, e));
            section(ui, "Orientation", true, |ui| orientation_section(app, ui, e));
            section(ui, "Sensor", false, |ui| sensor_section(app, ui, e));
            section(ui, "Beam finder", false, |ui| finder_section(app, ui, e));
        }
        Screen::Wait => {
            section(ui, "Planned run", true, |ui| plan_section(app, ui, e));
        }
        Screen::Measure => {
            section(ui, "Samples", true, |ui| sample_section(app, ui, e));
        }
        Screen::Results => {
            section(ui, "Results", true, |ui| results_section(app, ui));
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::CollapsingHeader::new(theme::heading(title)).default_open(open).show_unindented(ui, body);
    });
    ui.add_space(4.0);
}

fn mode_text(m: &VideoMode) -> String {
    format!("{}×{}  {:.0} fps  {}", m.width, m.height, m.fps, m.format.name())
}

fn camera_section(app: &mut App, ui: &mut egui::Ui, e: &mut Engine) {
    let current = app.settings.camera.device.clone().or_else(|| e.camera.device.as_ref().map(|d| d.name.clone()));
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Rescan").on_hover_text("Look for cameras again").clicked() {
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
        let mode = e.camera.mode;
        egui::ComboBox::from_id_salt("mode")
            .width(ui.available_width())
            .selected_text(mode.as_ref().map_or_else(String::new, mode_text))
            .show_ui(ui, |ui| {
                for m in e.camera.modes.clone() {
                    if ui.selectable_label(mode == Some(m), mode_text(&m)).clicked() {
                        app.settings.camera.mode = Some(m);
                        app.request(Request::Open(app.settings.camera.clone()));
                    }
                }
            });
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
    let lock = ui
        .add(theme::primary("Lock for measuring"))
        .on_hover_text("Every automatic control to manual, sharpening off, and the mains filter set. An automatic control moves the beam's brightness or shape while measuring.");
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

fn sensor_section(app: &mut App, ui: &mut egui::Ui, e: &Engine) {
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

fn plan_section(app: &mut App, ui: &mut egui::Ui, e: &mut Engine) {
    let p = &mut app.settings.plan;
    egui::Grid::new("plan").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Run length").on_hover_text("How long the measurement will take: stations × time at each.");
        ui.add(egui::DragValue::new(&mut p.run_min).range(1.0..=480.0).speed(0.5).suffix(" min"));
        ui.end_row();
        ui.label("Allowed drift").on_hover_text(
            "How far the reading may drift over the run. Ready means the drift expected over the run is below this.",
        );
        ui.add(egui::DragValue::new(&mut p.tolerance_um).range(0.01..=100.0).speed(0.01).suffix(" µm"));
        ui.end_row();
    });
    ui.label(theme::dim(
        "The set-up warms up after the laser and camera are switched on, and drifts as it does. Ready is when the drift over the planned run is inside the allowed drift.",
    ));
    if ui.button("Start the estimate again").on_hover_text("After a knock, or a change to the set-up.").clicked() {
        e.restart_monitor();
    }
}

fn sample_section(app: &mut App, ui: &mut egui::Ui, e: &Engine) {
    let s = &mut app.settings.sample;
    egui::Grid::new("sample").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Shortest").on_hover_text("A sample is at least this long.");
        ui.add(egui::DragValue::new(&mut s.min_s).range(0.2..=120.0).speed(0.1).suffix(" s"));
        ui.end_row();
        ui.label("Longest").on_hover_text("A sample ends after this long, sure enough or not.");
        ui.add(egui::DragValue::new(&mut s.max_s).range(0.5..=600.0).speed(0.1).suffix(" s"));
        ui.end_row();
        let mut target = s.target_um.is_some();
        ui.checkbox(&mut target, "Stop when sure to")
            .on_hover_text("End the sample once its uncertainty is below this.");
        let mut t = s.target_um.unwrap_or(0.1);
        ui.add_enabled(target, egui::DragValue::new(&mut t).range(0.001..=10.0).speed(0.001).suffix(" µm"));
        s.target_um = target.then_some(t);
        ui.end_row();
    });
    if s.max_s < s.min_s {
        s.max_s = s.min_s;
    }
    let _ = e;
}

fn results_section(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label("Units");
        for u in Unit::ALL {
            ui.selectable_value(&mut app.settings.unit, u, u.label());
        }
    });
    ui.horizontal(|ui| {
        ui.label("Drift").on_hover_text("How the zero visits correct the drift.");
        ui.selectable_value(&mut app.settings.drift_correction, DriftCorrection::Interpolate, "Interpolate")
            .on_hover_text("The zero for each reading is interpolated in time between the zero visits around it.");
        ui.selectable_value(&mut app.settings.drift_correction, DriftCorrection::LastZero, "Last zero")
            .on_hover_text("Each reading against the last zero visit before it.");
    });
}
