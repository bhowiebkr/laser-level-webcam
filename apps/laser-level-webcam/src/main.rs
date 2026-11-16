//! Laser Level Webcam: measure the straightness and flatness of a surface with a laser line and a
//! webcam sensor.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
#[cfg(not(target_arch = "wasm32"))]
mod crash;
mod driver;
mod engine;
mod files;
#[cfg(test)]
mod layout_check;
mod measure;
mod plot;
mod results;
mod sensors;
mod settings;
mod setup;
mod side;
mod theme;
mod visuals;
mod wait;

/// The name the desktop app keeps its settings and saved sensors under.
#[cfg(not(target_arch = "wasm32"))]
pub const APP_ID: &str = "laser-level-webcam";

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    crash::install();
    match std::panic::catch_unwind(run) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            crash::stopped(crash::Stop::Start, &e.to_string());
            Err(e)
        }
        Err(payload) => {
            crash::stopped(crash::Stop::App, crash::payload_text(payload.as_ref()));
            std::panic::resume_unwind(payload)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run() -> eframe::Result {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png")).expect("the app icon is a PNG");
    let mut options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Laser Level Webcam")
            .with_icon(icon)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    #[cfg(windows)]
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        use eframe::egui_wgpu::wgpu::Backends;
        setup.instance_descriptor.backends = Backends::from_env().unwrap_or(Backends::DX12);
    }
    #[cfg(not(windows))]
    let _ = &mut options;
    eframe::run_native(APP_ID, options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast;

    wasm_bindgen_futures::spawn_local(async {
        let canvas = eframe::web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("laser_canvas"))
            .and_then(|c| c.dyn_into::<eframe::web_sys::HtmlCanvasElement>().ok())
            .expect("a canvas with id laser_canvas");
        let started = eframe::WebRunner::new()
            .start(canvas, eframe::WebOptions::default(), Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
            .await;
        if let Err(e) = started {
            eframe::web_sys::console::error_1(&e);
            // Say why on the page, so it does not show "Loading…" for ever.
            let why = e.as_string().or_else(|| e.dyn_ref::<js_sys::Error>().map(|e| String::from(e.message()))).unwrap_or_default();
            if let Some(b) = eframe::web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id("loading")) {
                b.set_text_content(Some(&format!("The app could not start: {why}")));
            }
        }
    });
}
