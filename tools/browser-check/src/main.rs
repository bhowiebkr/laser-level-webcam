//! Checks the browser camera in a real browser.
//!
//! `cargo run -p browser-check` builds the check page with trunk, serves it on this machine,
//! opens it in Chrome with the camera allowed, and prints each result the page sends back.
//! The page runs the app's own browser camera code (`laser_camera::web`) on every camera the
//! browser lists: it opens each, reads frames, sets each control to its lowest, highest and
//! middle value, past both ends and to automatic, locks the camera for measuring, closes it,
//! opens it again with the lock, and opens each mode it offers.
//!
//! `--fake` uses the browser's own test camera in place of the real ones.

#[cfg(target_arch = "wasm32")]
mod check;
#[cfg(not(target_arch = "wasm32"))]
mod run;

#[cfg(target_arch = "wasm32")]
fn main() {
    check::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    run::main()
}
