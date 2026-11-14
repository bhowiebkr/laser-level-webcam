//! The window: the workflow across the top (set up, wait, measure, results), settings on the
//! left, the screen for the current step in the middle, the camera's state along the bottom.

use std::sync::{Arc, Mutex};

use eframe::egui::{self, Margin, RichText};
use laser_camera::DeviceInfo;
use laser_core::beam::{FinderConfig, NoBeam};
use laser_core::scale::Scale;
use laser_core::survey::Session;
use serde::{Deserialize, Serialize};

use crate::driver::{Driver, Request};
use crate::engine::{CameraState, Engine, lock};
use crate::files::{self, Inbox};
use crate::sensors::{self, CameraId, SensorLibrary};
use crate::settings::{AppSettings, KEY, SESSION_KEY};
use crate::{measure, results, setup, side, theme, wait};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Screen {
    #[default]
    SetUp,
    Wait,
    Measure,
    Results,
}

impl Screen {
    pub const ALL: [Screen; 4] = [Screen::SetUp, Screen::Wait, Screen::Measure, Screen::Results];

    pub fn label(self) -> &'static str {
        match self {
            Self::SetUp => "Set up",
            Self::Wait => "Wait",
            Self::Measure => "Measure",
            Self::Results => "Results",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::SetUp => "Camera, exposure, beam and scale",
            Self::Wait => "Heat drift: wait until the set-up is calm",
            Self::Measure => "Stations, zero and samples",
            Self::Results => "Straightness or flatness, scrape and shim, export",
        }
    }
}

/// Something asked for from the session menu, done once the engine is unlocked.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FileAction {
    New,
    Open,
    Save,
}

/// Something asked for from the saved sensors, done once the engine is unlocked.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SensorAction {
    Export,
    Import,
}

/// The camera the sensor in the settings was last used with.
const SENSOR_CAMERA_KEY: &str = "sensor_camera";

pub struct App {
    pub settings: AppSettings,
    pub engine: Arc<Mutex<Engine>>,
    pub driver: Driver,
    pub screen: Screen,
    pub devices: Vec<DeviceInfo>,
    /// Requests made while drawing, sent once the engine is unlocked.
    pub requests: Vec<Request>,
    /// The settings the engine has.
    synced: AppSettings,
    preview: Option<(egui::TextureHandle, u64)>,
    file_action: Option<FileAction>,
    /// A session file the user picked, once read.
    inbox: Inbox,
    /// What happened to the last file saved or opened.
    file_msg: Option<String>,
    /// The sensors the user has saved.
    pub sensors: SensorLibrary,
    /// Whether the saved sensors are written back to the computer (not in tests).
    keep_sensors: bool,
    /// The camera open now, as the saved sensors know cameras.
    pub camera_id: Option<CameraId>,
    /// The camera the sensor in the settings was last used with.
    sensor_camera: Option<CameraId>,
    pub sensor_action: Option<SensorAction>,
    /// A sensor list the user picked to import, once read.
    sensor_inbox: Inbox,
    /// What happened to the saved sensors last.
    pub sensor_msg: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let settings: AppSettings = cc.storage.and_then(|s| eframe::get_value(s, KEY)).unwrap_or_default();
        let session = cc
            .storage
            .and_then(|s| eframe::get_value::<String>(s, SESSION_KEY))
            .and_then(|text| Session::from_json(&text).ok());
        let mut app = Self::with(&cc.egui_ctx, settings, session);
        let loaded = sensors::load();
        app.sensors = loaded.library;
        app.sensor_msg = loaded.problem;
        app.keep_sensors = true;
        app.sensor_camera = cc.storage.and_then(|s| eframe::get_value(s, SENSOR_CAMERA_KEY));
        app
    }

    /// The app with these settings, and this session or a new one.
    pub fn with(ctx: &egui::Context, settings: AppSettings, session: Option<Session>) -> Self {
        let settings = settings.normalized();
        let session = session.unwrap_or_else(|| Session::new(Scale::from_pixel_pitch(settings.sensor.pixel_pitch_um)));
        let engine = Arc::new(Mutex::new(Engine::new(session)));
        let repaint = ctx.clone();
        let mut driver = Driver::start(Arc::clone(&engine), move || repaint.request_repaint());
        sync(&mut lock(&engine), &settings, None);
        driver.send(Request::Open(settings.camera.clone()));
        Self {
            synced: settings.clone(),
            settings,
            engine,
            driver,
            screen: Screen::SetUp,
            devices: laser_camera::devices(),
            requests: Vec::new(),
            preview: None,
            file_action: None,
            inbox: Inbox::default(),
            file_msg: None,
            sensors: SensorLibrary::default(),
            keep_sensors: false,
            camera_id: None,
            sensor_camera: None,
            sensor_action: None,
            sensor_inbox: Inbox::default(),
            sensor_msg: None,
        }
    }

    /// A camera has opened. A camera other than the one the sensor was last used with gets the
    /// sensor saved for it; the same camera keeps the sensor as it is, edits and all.
    pub fn camera_opened(&mut self, id: &CameraId) {
        let same = self.sensor_camera.as_ref().is_some_and(|c| c.matches(id));
        if !same {
            match self.sensors.for_camera(id) {
                Some(saved) if saved.sensor != self.settings.sensor => {
                    self.settings.sensor = saved.sensor.clone();
                    self.sensor_msg = Some(format!("Picked the saved sensor {} for {}.", saved.sensor.name, id.name));
                }
                Some(_) => {}
                None if self.sensor_camera.is_some() => {
                    self.sensor_msg = Some(format!(
                        "No sensor is saved for {} yet. Check the sensor and save it.",
                        id.name
                    ));
                }
                None => {}
            }
        }
        self.sensor_camera = Some(id.clone());
    }

    /// Save the sensor in the settings, for the camera that is open.
    pub fn save_sensor(&mut self) {
        let camera = self.camera_id.clone();
        self.sensors.save(&self.settings.sensor, camera.as_ref());
        if camera.is_some() {
            self.sensor_camera = camera;
        }
        self.sensor_msg = None;
        self.keep_sensor_list();
    }

    pub fn delete_sensor(&mut self, name: &str) {
        if self.sensors.remove(name) {
            self.sensor_msg = Some(format!("Deleted {}. The settings still hold it until changed.", name.trim()));
            self.keep_sensor_list();
        }
    }

    /// Write the saved sensors back to the computer.
    fn keep_sensor_list(&mut self) {
        if self.keep_sensors
            && let Err(why) = sensors::store(&self.sensors)
        {
            self.sensor_msg = Some(why);
        }
    }

    /// The open camera, if it has changed since the last frame.
    fn watch_camera(&mut self, e: &Engine) {
        let id = e
            .camera
            .device
            .as_ref()
            .filter(|_| e.camera.state == CameraState::Open)
            .map(|d| CameraId::new(&d.name, e.camera.usb.as_ref().map(|u| (u.vendor_id, u.product_id))));
        if id != self.camera_id {
            if let Some(id) = &id {
                // A camera other than the one the setup is for (the first camera, when none is
                // picked) gets its own setup, and is opened again if its mode or controls differ.
                if self.settings.use_setup_of(&id.name, None) {
                    self.request(Request::Open(self.settings.camera.clone()));
                }
                self.camera_opened(id);
            }
            self.camera_id = id;
        }
    }

    /// Draw one frame.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        self.driver.pump();
        // The browser names its cameras only once one is allowed; reading the list is cheap there.
        #[cfg(target_arch = "wasm32")]
        {
            self.devices = laser_camera::devices();
        }
        let engine = Arc::clone(&self.engine);
        let mut e = lock(&engine);
        self.watch_camera(&e);
        self.upload_preview(ui.ctx(), &e);

        egui::Panel::top("steps")
            .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(12, 8)))
            .show(ui, |ui| self.top_bar(ui, &e));
        egui::Panel::bottom("camera_status")
            .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(12, 4)))
            .show(ui, |ui| status_bar(ui, &e));
        let screen_w = ui.ctx().content_rect().width();
        // Only the set-up step has enough settings for a column; the others keep theirs on screen.
        if self.screen == Screen::SetUp {
            egui::Panel::left("settings")
                .resizable(true)
                .default_size((screen_w * 0.27).clamp(320.0, 560.0))
                .min_size(290.0)
                .max_size((screen_w * 0.5).max(300.0))
                .frame(egui::Frame::new().fill(theme::BACKGROUND).inner_margin(Margin::symmetric(10, 10)))
                .show(ui, |ui| {
                    theme::scroll(ui, |ui| side::draw(self, ui, &mut e));
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BACKGROUND).inner_margin(Margin::same(10)))
            .show(ui, |ui| match self.screen {
                Screen::SetUp => setup::draw(self, ui, &mut e),
                Screen::Wait => wait::draw(self, ui, &mut e),
                Screen::Measure => measure::draw(self, ui, &mut e),
                Screen::Results => results::draw(self, ui, &mut e),
            });

        self.settings.finder.axis = self.settings.orientation.axis();
        if self.settings != self.synced {
            sync(&mut e, &self.settings, Some(&self.synced));
            self.synced = self.settings.clone();
        }
        drop(e);
        for r in std::mem::take(&mut self.requests) {
            self.driver.send(r);
        }
        self.files(ui.ctx());
    }

    /// Do what the session menu asked for, and take in a session file once it has been read.
    fn files(&mut self, ctx: &egui::Context) {
        match self.file_action.take() {
            Some(FileAction::New) => {
                let scale = Scale::from_pixel_pitch(self.settings.sensor.pixel_pitch_um);
                lock(&self.engine).load_session(Session::new(scale));
                self.file_msg = None;
            }
            Some(FileAction::Open) => {
                let repaint = ctx.clone();
                self.file_msg = self.inbox.open("json", move || repaint.request_repaint());
            }
            Some(FileAction::Save) => {
                let session = self.session_to_save();
                let name = session.runs.first().map_or("session", |r| r.name.as_str()).to_owned();
                let msg = files::save_text(
                    &format!("{}.json", files::file_stem(&name)),
                    "application/json",
                    &session.to_json(),
                );
                self.file_msg = Some(msg).filter(|m| !m.is_empty());
            }
            None => {}
        }
        if let Some((name, text)) = self.inbox.take() {
            match Session::from_json(&text) {
                Ok(session) => {
                    self.settings.unit = session.unit;
                    self.settings.sensor = session.sensor.clone();
                    self.settings.finder = session.finder.clone();
                    self.settings.orientation = session.orientation();
                    self.settings = std::mem::take(&mut self.settings).normalized();
                    self.settings.sample = session.sample;
                    self.settings.drift_correction = session.drift_correction;
                    let mut e = lock(&self.engine);
                    e.load_session(session);
                    sync(&mut e, &self.settings, None);
                    self.synced = self.settings.clone();
                    self.file_msg = Some(format!("Opened {name}"));
                }
                Err(why) => self.file_msg = Some(format!("Could not open {name}: {why}")),
            }
        }
        self.sensor_files(ctx);
    }

    /// Export or import the saved sensors.
    fn sensor_files(&mut self, ctx: &egui::Context) {
        match self.sensor_action.take() {
            Some(SensorAction::Export) => {
                let msg = files::save_text("sensors.json", "application/json", &self.sensors.to_json());
                self.sensor_msg = Some(msg).filter(|m| !m.is_empty());
            }
            Some(SensorAction::Import) => {
                let repaint = ctx.clone();
                self.sensor_msg = self.sensor_inbox.open("json", move || repaint.request_repaint());
            }
            None => {}
        }
        if let Some((name, text)) = self.sensor_inbox.take() {
            self.sensor_msg = Some(match SensorLibrary::from_json(&text) {
                Ok(list) => match self.sensors.merge(list) {
                    0 => format!("No saved sensors in {name}."),
                    n => {
                        self.keep_sensor_list();
                        format!("Imported {n} sensor{} from {name}.", if n == 1 { "" } else { "s" })
                    }
                },
                Err(why) => format!("Could not read {name}: {why}"),
            });
        }
    }

    /// Ask the driver for something once the frame is drawn.
    pub fn request(&mut self, r: Request) {
        self.requests.push(r);
    }

    /// The preview texture, uploaded when the engine has a newer one.
    pub fn preview(&self) -> Option<&egui::TextureHandle> {
        self.preview.as_ref().map(|(t, _)| t)
    }

    fn upload_preview(&mut self, ctx: &egui::Context, e: &Engine) {
        let Some(d) = &e.live.detail else { return };
        let version = e.live.detail_version;
        if self.preview.as_ref().is_some_and(|(_, v)| *v == version) {
            return;
        }
        let image = egui::ColorImage::from_gray([d.preview.width, d.preview.height], &d.preview.data);
        match &mut self.preview {
            Some((tex, v)) => {
                tex.set(image, egui::TextureOptions::LINEAR);
                *v = version;
            }
            None => self.preview = Some((ctx.load_texture("preview", image, egui::TextureOptions::LINEAR), version)),
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, e: &Engine) {
        ui.horizontal_wrapped(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            ui.label(RichText::new("Laser Level Webcam").strong().size(16.0));
            ui.menu_button("Session", |ui| {
                if ui
                    .button("New session")
                    .on_hover_text("Start again with no runs. Save this one first to keep it.")
                    .clicked()
                {
                    self.file_action = Some(FileAction::New);
                }
                if ui.button("Open session…").clicked() {
                    self.file_action = Some(FileAction::Open);
                }
                if ui
                    .button("Save session…")
                    .on_hover_text("Every run and visit, with the settings they were measured with.")
                    .clicked()
                {
                    self.file_action = Some(FileAction::Save);
                }
            });
            ui.menu_button("View", |ui| {
                ui.label(theme::dim(format!("UI size {:.0} %", ui.ctx().zoom_factor() * 100.0)));
                egui::gui_zoom::zoom_menu_buttons(ui);
            })
            .response
            .on_hover_text("Make everything larger or smaller. Ctrl + and Ctrl − do the same.");
            if let Some(msg) = &self.file_msg {
                ui.label(RichText::new(msg).small().color(theme::TEXT_DIM));
            }
            ui.add_space(16.0);
            for (k, screen) in Screen::ALL.into_iter().enumerate() {
                let on = self.screen == screen;
                let text = RichText::new(format!("{}  {}", k + 1, screen.label()));
                let button = if on {
                    egui::Button::new(text.color(theme::ON_ACCENT).strong()).fill(theme::ACCENT)
                } else {
                    egui::Button::new(text)
                };
                if ui.add(button).on_hover_text(screen.hint()).clicked() {
                    self.screen = screen;
                }
            }
            // The status chips sit at the right end, or on a row of their own when the window is
            // too narrow for them beside the steps.
            let width_id = ui.scope_id().with("chips_width");
            let need: f32 = ui.data(|d| d.get_temp(width_id)).unwrap_or(320.0);
            let fps = (e.camera.state == CameraState::Open).then(|| format!("{:.1} fps", e.live.fps));
            let used = if ui.available_size_before_wrap().x >= need + 12.0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    beam_chip(ui, e);
                    if let Some(fps) = &fps {
                        theme::chip(ui, fps.clone(), theme::TEXT_DIM);
                    }
                    camera_chip(ui, e);
                })
                .response
                .rect
                .width()
            } else {
                ui.end_row();
                ui.horizontal(|ui| {
                    camera_chip(ui, e);
                    if let Some(fps) = &fps {
                        theme::chip(ui, fps.clone(), theme::TEXT_DIM);
                    }
                    beam_chip(ui, e);
                })
                .response
                .rect
                .width()
            };
            ui.data_mut(|d| d.insert_temp(width_id, used));
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }

    fn on_exit(&mut self) {
        self.driver.send(Request::Close);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, KEY, &self.settings);
        let session = self.session_to_save();
        eframe::set_value(storage, SESSION_KEY, &session.to_json_compact());
        eframe::set_value(storage, SENSOR_CAMERA_KEY, &self.sensor_camera);
    }
}

impl App {
    /// The session with the settings it was measured with.
    pub fn session_to_save(&self) -> Session {
        let e = lock(&self.engine);
        let mut s = e.session.clone();
        s.unit = self.settings.unit;
        s.sensor = self.settings.sensor.clone();
        s.camera = self.settings.camera.clone();
        s.finder = self.settings.finder.clone();
        s.view = Some(self.settings.orientation);
        s.sample = self.settings.sample;
        s.drift_correction = self.settings.drift_correction;
        s
    }
}

/// Give the engine the settings it works with.
fn sync(e: &mut Engine, s: &AppSettings, old: Option<&AppSettings>) {
    let axis = s.orientation.axis();
    let axis_changed = old.is_some_and(|o| o.orientation.axis() != axis);
    e.finder = FinderConfig { axis, ..s.finder.clone() };
    e.sensor = s.sensor.clone();
    e.plan = s.plan.run_plan();
    e.sample_rule = s.sample;
    e.invert = s.orientation.invert();
    e.session.view = Some(s.orientation);
    e.session.drift_correction = s.drift_correction;
    e.sync_scale();
    if axis_changed {
        e.restart_monitor();
    }
}

fn camera_chip(ui: &mut egui::Ui, e: &Engine) {
    let name = e.camera.device.as_ref().map_or("Camera", |d| d.name.as_str());
    let (text, color) = match &e.camera.state {
        CameraState::Closed => ("No camera".to_owned(), theme::TEXT_DIM),
        CameraState::Opening(n) => (format!("Opening {n}"), theme::ACCENT),
        CameraState::Open => (name.to_owned(), theme::OK),
        CameraState::Failed(why) => (format!("Camera: {why}"), theme::ERROR),
    };
    theme::chip(ui, text, color);
}

fn beam_chip(ui: &mut egui::Ui, e: &Engine) {
    if e.camera.state != CameraState::Open {
        return;
    }
    match &e.live.fix {
        Some(Ok(fix)) if fix.warnings.any() => {
            theme::chip(ui, format!("Beam {:.2} px, check warnings", fix.position_px), theme::ACCENT);
        }
        Some(Ok(fix)) => {
            theme::chip(ui, format!("Beam {:.2} px", fix.position_px), theme::OK);
        }
        Some(Err(why)) => {
            theme::chip(ui, no_beam_short(*why), theme::ERROR).on_hover_text(why.text());
        }
        None => {}
    }
}

pub fn no_beam_short(why: NoBeam) -> &'static str {
    match why {
        NoBeam::Flat => "No beam",
        NoBeam::Edge => "Beam at the edge",
        NoBeam::TooWide => "Beam too wide",
        NoBeam::FewStrips => "Beam broken up",
    }
}

fn status_bar(ui: &mut egui::Ui, e: &Engine) {
    ui.horizontal(|ui| {
        let small = |t: String| RichText::new(t).small().color(theme::TEXT_DIM);
        match (&e.camera.device, e.camera.mode) {
            (Some(d), Some(m)) => {
                ui.label(small(format!(
                    "{}  ·  {}×{} {:.0} fps {}",
                    d.name,
                    m.width,
                    m.height,
                    m.fps,
                    m.format.name()
                )));
            }
            _ => {
                ui.label(small("No camera open".into()));
            }
        }
        ui.label(small(format!("frames {}", e.live.frames)));
        let gaps = RichText::new(format!("gaps {}", e.live.gaps)).small();
        ui.label(if e.live.gaps > 0 { gaps.color(theme::ACCENT) } else { gaps.color(theme::TEXT_DIM) }).on_hover_text(
            "Frame times with no frame: lost by the camera, or an automatic exposure longer than the frame period.",
        );
        let dropped = RichText::new(format!("dropped {}", e.live.dropped)).small();
        ui.label(if e.live.dropped > 0 { dropped.color(theme::ACCENT) } else { dropped.color(theme::TEXT_DIM) })
            .on_hover_text("Frames this computer was too busy to read.");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(small(format!("session {}", crate::plot::clock(e.now()))));
        });
    });
}

/// Wait until the driver has given up on a camera that is not there, so the frames a test feeds are
/// the only ones and the screens stop changing on their own.
#[cfg(test)]
pub(crate) fn wait_until_failed(app: &App) {
    use std::time::{Duration, Instant};
    let end = Instant::now() + Duration::from_secs(20);
    while !matches!(lock(&app.engine).camera.state, CameraState::Failed(_)) {
        assert!(Instant::now() < end, "the camera that is not there did not fail");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use laser_camera::FrameSource;
    use laser_camera::synthetic::Synthetic;
    use laser_core::reading::SampleRule;

    use super::*;
    use crate::engine::process;

    fn input() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
            ..Default::default()
        }
    }

    fn draw_all(app: &mut App, ctx: &egui::Context) {
        for screen in Screen::ALL {
            app.screen = screen;
            // Twice: the first pass of a new layout only measures.
            for _ in 0..2 {
                let mut out = ctx.run_ui(input(), |ui| app.draw(ui));
                out.textures_delta.clear();
            }
        }
    }

    /// Frames for `seconds` of camera time, from a camera that is not paced.
    fn feed(e: &mut Engine, cam: &mut Synthetic, seconds: f64) {
        let fps = cam.mode().fps;
        for k in 0..(seconds * fps) as usize {
            let frame = cam.next_frame().unwrap();
            let finder = e.finder.clone();
            e.absorb(process(&frame, &finder, k % 15 == 0));
        }
    }

    #[test]
    fn every_screen_draws_empty_and_full() {
        let ctx = egui::Context::default();
        let mut settings = AppSettings::default();
        // A camera that is not there, so the driver stays idle and the test feeds the frames.
        settings.camera.device = Some("No such camera".into());
        // Short samples, in the settings so the engine keeps them when the settings change.
        settings.sample = SampleRule { min_s: 0.5, max_s: 1.0, target_um: None };
        let mut app = App::with(&ctx, settings, None);
        app.sensors.save(&app.settings.sensor.clone(), None);
        wait_until_failed(&app);
        draw_all(&mut app, &ctx);

        let small = Synthetic::open(None).modes()[3];
        let mut cam = Synthetic::open(Some(small));
        cam.paced = false;
        {
            let mut e = lock(&app.engine);
            let (device, modes, mode, controls) =
                (cam.device().clone(), cam.modes().to_vec(), cam.mode(), cam.controls());
            e.opened(device, modes, mode, controls, Vec::new());
            feed(&mut e, &mut cam, 90.0);
            assert!(e.status.is_some());
        }
        draw_all(&mut app, &ctx);

        // Zero, every station, zero again; then one station again.
        let stations = lock(&app.engine).current_run().unwrap().stations.len();
        for station in std::iter::once(0).chain(1..stations).chain([0, 2]) {
            let mut e = lock(&app.engine);
            e.take_sample(station, None);
            feed(&mut e, &mut cam, 1.5);
            assert!(e.sampling.is_none());
            assert!(e.survey_error.is_none(), "{:?}", e.survey_error);
        }
        // A sample under way as well.
        lock(&app.engine).take_sample(1, None);
        feed(&mut lock(&app.engine), &mut cam, 0.3);
        draw_all(&mut app, &ctx);
        let e = lock(&app.engine);
        assert_eq!(e.current_run().unwrap().visits.len(), stations + 2);
        assert!(e.sampling.is_some());
    }

    #[test]
    fn a_camera_gets_the_sensor_saved_for_it() {
        use laser_core::camera::Sensor;
        let ctx = egui::Context::default();
        let mut settings = AppSettings::default();
        settings.camera.device = Some("No such camera".into());
        let mut app = App::with(&ctx, settings, None);
        let c270 = CameraId::new("HD Webcam C270 (046d:0825)", None);
        let other = CameraId::new("USB Camera", Some((0x0c45, 0x6366)));
        let ov = Sensor { name: "OV2710".into(), pixel_pitch_um: 3.0, ..Sensor::default() };
        let imx = Sensor { name: "IMX291".into(), pixel_pitch_um: 2.9, ..Sensor::default() };

        app.settings.sensor = ov.clone();
        app.camera_id = Some(c270.clone());
        app.save_sensor();
        app.settings.sensor = imx.clone();
        app.camera_id = Some(other.clone());
        app.save_sensor();
        assert_eq!(app.sensors.sensors.len(), 2);

        app.camera_opened(&CameraId::new("HD Webcam C270", Some((0x046d, 0x0825))));
        assert_eq!(app.settings.sensor, ov, "another camera gets its own sensor");
        assert!(app.sensor_msg.as_deref().unwrap().contains("OV2710"));

        app.settings.sensor.pixel_pitch_um = 2.95;
        app.camera_opened(&c270);
        assert_eq!(app.settings.sensor.pixel_pitch_um, 2.95, "the same camera keeps the edits");

        app.camera_opened(&CameraId::new("Unknown cam", None));
        assert_eq!(app.settings.sensor.pixel_pitch_um, 2.95, "a camera with no saved sensor changes nothing");
        assert!(app.sensor_msg.as_deref().unwrap().contains("No sensor is saved for Unknown cam"));

        app.camera_opened(&other);
        assert_eq!(app.settings.sensor, imx);
        app.delete_sensor("imx291");
        assert!(app.sensors.get("IMX291").is_none());
    }

    #[test]
    fn a_saved_session_opens_with_its_visits() {
        let ctx = egui::Context::default();
        let mut settings = AppSettings::default();
        settings.camera.device = Some("No such camera".into());
        let app = App::with(&ctx, settings, None);
        let mut e = lock(&app.engine);
        let mut saved = e.session.clone();
        saved.started_unix -= 3600.0;
        saved.runs[0].name = "Old run".into();
        let text = saved.to_json();
        e.load_session(Session::from_json(&text).unwrap());
        assert_eq!(e.current_run().unwrap().name, "Old run");
        assert!(e.now() >= 3600.0, "the clock carries on from the saved start");
    }
}
