//! Opens the real window with each renderer and draws a few frames of the app:
//!
//! `cargo test -p laser-level-webcam -- --ignored start_check --nocapture`
//!
//! The window opens off the screen, with no taskbar button, and does not take the focus. It uses
//! the Synthetic camera and a temporary settings folder, so it does not touch the user's camera
//! or settings.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use eframe::egui;
use eframe::egui_wgpu::{WgpuSetup, wgpu};
use winit::platform::windows::EventLoopBuilderExtWindows as _;

use crate::app::App;
use crate::renderer::Renderer;
use crate::settings::AppSettings;

/// Frames to draw before the window closes.
const FRAMES: u32 = 5;

/// The app, with a count of its frames. It closes the window after [`FRAMES`].
struct Probe {
    app: App,
    frames: u32,
    /// The adapter that drew the frames.
    adapter: Rc<RefCell<Option<wgpu::AdapterInfo>>>,
}

impl eframe::App for Probe {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        self.frames += 1;
        if self.frames == FRAMES {
            *self.adapter.borrow_mut() = frame.wgpu_render_state().map(|s| s.adapter.get_info());
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.ctx().request_repaint();
    }

    fn on_exit(&mut self) {
        self.app.on_exit();
    }
}

#[test]
#[ignore = "opens a window on the desktop"]
fn start_check() {
    // A window that never draws would stop the check for ever.
    let done = Arc::new(AtomicBool::new(false));
    let watch = Arc::clone(&done);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(120));
        if !watch.load(Ordering::Relaxed) {
            eprintln!("start check: no result after 120 s");
            std::process::exit(1);
        }
    });

    let dir = std::env::temp_dir().join(format!("laser-start-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // The fallbacks in order, then OpenGL alone, which the Vulkan-or-OpenGL try does not reach
    // on a computer with Vulkan.
    let tries =
        [(Renderer::First, false), (Renderer::Warp, false), (Renderer::VulkanGl, false), (Renderer::VulkanGl, true)];
    let mut report = Vec::new();
    for (renderer, gl_only) in tries {
        let mut options = crate::native_options(renderer);
        if gl_only && let WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
            setup.instance_descriptor.backends = wgpu::Backends::GL;
        }
        options.viewport =
            options.viewport.with_position([-20_000.0, -20_000.0]).with_active(false).with_taskbar(false);
        // The file itself, not its folder.
        options.persistence_path = Some(dir.join("app.ron"));
        options.persist_window = false;
        // Used once: the event loop stays for the next window.
        options.event_loop_builder = Some(Box::new(|b| {
            b.with_any_thread(true);
        }));

        let adapter = Rc::new(RefCell::new(None));
        let seen = Rc::clone(&adapter);
        let result = eframe::run_native(
            crate::APP_ID,
            options,
            Box::new(move |cc| {
                crate::theme::apply(&cc.egui_ctx);
                let mut settings = AppSettings::default();
                settings.camera.device = Some(laser_camera::synthetic::NAME.into());
                Ok(Box::new(Probe { app: App::with(&cc.egui_ctx, settings, None), frames: 0, adapter: seen }))
            }),
        );
        let name = if gl_only { "OpenGL alone".to_owned() } else { renderer.to_string() };
        let drew = adapter.borrow().as_ref().map(|i| format!("{:?}, {} ({:?})", i.backend, i.name, i.device_type));
        report.push((name, result.map_err(|e| e.to_string()), drew));
    }
    done.store(true, Ordering::Relaxed);
    let _ = std::fs::remove_dir_all(&dir);

    for (name, result, drew) in &report {
        println!(
            "{name}: {}",
            match (result, drew) {
                (Ok(()), Some(drew)) => format!("{FRAMES} frames with {drew}"),
                (Ok(()), None) => "closed before it drew".to_owned(),
                (Err(e), _) => format!("did not open: {e}"),
            }
        );
    }
    assert!(report.iter().all(|(_, result, drew)| result.is_ok() && drew.is_some()), "{report:#?}");
}
