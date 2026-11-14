//! Runs the camera and feeds the engine.
//!
//! Natively the camera has its own thread ([`laser_camera::capture`]) and this driver has another,
//! which finds the beam in each frame and hands the result to the engine, so measuring carries on
//! while the window is hidden. In the browser there are no threads: the window calls
//! [`Driver::pump`] each frame, and the synthetic camera makes the frames that are due.

use std::sync::{Arc, Mutex};

use laser_core::camera::{CameraSettings, ControlSetting};

use crate::engine::Engine;

/// The window asks for the preview and inspection this often, s.
const DETAIL_S: f64 = 1.0 / 15.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Open the camera the settings name; closes the open one first.
    Open(CameraSettings),
    Close,
    Set(ControlSetting),
    Apply(Vec<ControlSetting>),
    /// Read the controls again.
    Refresh,
}

/// Whether this frame should carry detail for the window.
fn wants_detail(last: &mut f64, t_s: f64) -> bool {
    if t_s - *last >= DETAIL_S || t_s < *last {
        *last = t_s;
        true
    } else {
        false
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::Driver;
#[cfg(target_arch = "wasm32")]
pub use web::Driver;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use laser_camera::capture::{Capture, Command, Event};

    use super::*;
    use crate::engine::{lock, process};

    pub struct Driver {
        requests: Sender<Request>,
        thread: Option<JoinHandle<()>>,
    }

    impl Driver {
        pub fn start(engine: Arc<Mutex<Engine>>, repaint: impl Fn() + Send + 'static) -> Driver {
            let (requests, rx) = mpsc::channel();
            let thread = thread::Builder::new()
                .name("driver".into())
                .spawn(move || run(&engine, &rx, &repaint))
                .expect("a thread for the camera driver");
            Driver { requests, thread: Some(thread) }
        }

        pub fn send(&mut self, request: Request) {
            let _ = self.requests.send(request);
        }

        /// Nothing to do: the driver's thread does the work.
        pub fn pump(&mut self) {}
    }

    impl Drop for Driver {
        fn drop(&mut self) {
            let (closed, _) = mpsc::channel();
            drop(std::mem::replace(&mut self.requests, closed));
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    fn run(engine: &Mutex<Engine>, requests: &Receiver<Request>, repaint: &dyn Fn()) {
        let mut cap: Option<Capture> = None;
        let mut last_detail = f64::NEG_INFINITY;
        loop {
            // With no camera, wait for a request; with one, take the requests waiting.
            loop {
                let request = if cap.is_some() {
                    match requests.try_recv() {
                        Ok(r) => r,
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                } else {
                    match requests.recv() {
                        Ok(r) => r,
                        Err(_) => return,
                    }
                };
                match request {
                    Request::Open(settings) => {
                        if let Some(c) = cap.take() {
                            c.stop();
                        }
                        lock(engine).opening(settings.device.as_deref());
                        match Capture::start(settings) {
                            Ok(c) => cap = Some(c),
                            Err(e) => lock(engine).failed(&e),
                        }
                        last_detail = f64::NEG_INFINITY;
                    }
                    Request::Close => {
                        if let Some(c) = cap.take() {
                            c.stop();
                        }
                        lock(engine).closed();
                    }
                    Request::Set(s) => send(cap.as_ref(), Command::Set(s)),
                    Request::Apply(s) => send(cap.as_ref(), Command::Apply(s)),
                    Request::Refresh => send(cap.as_ref(), Command::Refresh),
                }
                repaint();
            }
            let Some(c) = &cap else { continue };
            match c.recv_timeout(Duration::from_millis(100)) {
                Ok(Event::Frame(frame)) => {
                    let detail = wants_detail(&mut last_detail, frame.t_s);
                    let cfg = lock(engine).finder.clone();
                    let p = process(&frame, &cfg, detail);
                    let mut e = lock(engine);
                    e.live.dropped = c.dropped();
                    e.absorb(p);
                    drop(e);
                    if detail {
                        repaint();
                    }
                }
                Ok(Event::Opened { device, modes, mode, controls, applied }) => {
                    lock(engine).opened(device, modes, mode, controls, applied);
                    repaint();
                }
                Ok(Event::Controls(controls)) => {
                    lock(engine).controls(controls);
                    repaint();
                }
                Ok(Event::Control(applied)) => {
                    lock(engine).control(applied);
                    repaint();
                }
                Ok(Event::Error(e)) => {
                    lock(engine).failed(&e);
                    repaint();
                }
                // The camera's thread has ended; its error came first.
                Err(true) => cap = None,
                Err(false) => {}
            }
        }
    }

    fn send(cap: Option<&Capture>, command: Command) {
        if let Some(c) = cap {
            c.send(command);
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use laser_camera::FrameSource;

    use super::*;
    use crate::engine::{lock, process};

    /// Frames made in one window frame at most; the camera's clock falls behind beyond that.
    const MAX_PER_PUMP: usize = 2;

    pub struct Driver {
        engine: Arc<Mutex<Engine>>,
        repaint: Box<dyn Fn()>,
        source: Option<Box<dyn FrameSource>>,
        /// Session time the camera opened.
        opened_s: f64,
        last_detail: f64,
    }

    impl Driver {
        pub fn start(engine: Arc<Mutex<Engine>>, repaint: impl Fn() + Send + 'static) -> Driver {
            Driver { engine, repaint: Box::new(repaint), source: None, opened_s: 0.0, last_detail: f64::NEG_INFINITY }
        }

        pub fn send(&mut self, request: Request) {
            let mut e = lock(&self.engine);
            match request {
                Request::Open(settings) => {
                    e.opening(settings.device.as_deref());
                    match laser_camera::open(&settings) {
                        Ok((mut source, applied)) => {
                            let (device, modes, mode) =
                                (source.device().clone(), source.modes().to_vec(), source.mode());
                            e.opened(device, modes, mode, source.controls(), applied);
                            self.opened_s = e.now();
                            self.source = Some(source);
                            self.last_detail = f64::NEG_INFINITY;
                        }
                        Err(err) => e.failed(&err),
                    }
                }
                Request::Close => {
                    self.source = None;
                    e.closed();
                }
                Request::Set(s) => {
                    if let Some(src) = &mut self.source {
                        for a in laser_camera::apply(src.as_mut(), &[s]) {
                            e.control(a);
                        }
                    }
                }
                Request::Apply(s) => {
                    if let Some(src) = &mut self.source {
                        for a in laser_camera::apply(src.as_mut(), &s) {
                            e.control(a);
                        }
                    }
                }
                Request::Refresh => {
                    if let Some(src) = &mut self.source {
                        e.controls(src.controls());
                    }
                }
            }
            (self.repaint)();
        }

        /// Make the frames that are due.
        pub fn pump(&mut self) {
            let Some(src) = &mut self.source else { return };
            let period = src.mode().frame_period_s();
            let elapsed = lock(&self.engine).now() - self.opened_s;
            for _ in 0..MAX_PER_PUMP {
                let done = lock(&self.engine).live.frames;
                if (done as f64) * period > elapsed {
                    break;
                }
                let frame = match src.next_frame() {
                    Ok(f) => f,
                    Err(err) => {
                        lock(&self.engine).failed(&err);
                        self.source = None;
                        return;
                    }
                };
                let detail = wants_detail(&mut self.last_detail, frame.t_s);
                let cfg = lock(&self.engine).finder.clone();
                let p = process(&frame, &cfg, detail);
                lock(&self.engine).absorb(p);
            }
            (self.repaint)();
        }
    }
}
