//! A crash log, and on Windows a message, when the desktop app stops on an error. A release build
//! on Windows has no console, so without this the window closes and says nothing.

use std::any::Any;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// The log starts again when it is larger than this, so that it cannot fill the disk.
const MAX_LOG: u64 = 1 << 20;

/// What stopped, for the message.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stop {
    /// The window's thread: the app closes.
    App,
    /// Another thread, such as the camera's: the window stays open.
    Part,
    /// The window did not open.
    Start,
}

/// `crash.log` in the folder of the app's settings, or in the temporary folder.
pub fn log_path() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| eframe::storage_dir(crate::APP_ID).unwrap_or_else(std::env::temp_dir).join("crash.log"))
}

/// Writes each panic to the log. A panic on another thread also shows the message, once.
/// A panic on the window's thread comes back out of `run_native`; [`stopped`] shows that one.
pub fn install() {
    hook(log_path().to_owned(), show);
}

fn hook(log: PathBuf, show: fn(&str)) {
    let previous = panic::take_hook();
    let shown = AtomicBool::new(false);
    panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        let what = payload_text(info.payload());
        let at = info.location().map(ToString::to_string).unwrap_or_default();
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let written = append(&log, &entry(name, what, &at, &backtrace)).is_ok();
        previous(info);
        if name != "main" && !shown.swap(true, Ordering::Relaxed) {
            let text = message(Stop::Part, what, &log, written);
            // On a new thread, so that this one unwinds now and lets go of its locks.
            let _ = std::thread::Builder::new().name("crash message".into()).spawn(move || show(&text));
        }
    }));
}

/// Shows the message for a panic that came out of `run_native`, or for an error that kept the
/// window from opening (that one is also written to the log).
pub fn stopped(stop: Stop, what: &str) {
    let written = match stop {
        Stop::Start => append(log_path(), &entry("main", what, "", "")).is_ok(),
        Stop::App | Stop::Part => log_path().exists(),
    };
    show(&message(stop, what, log_path(), written));
}

/// Writes a problem that the app got past, such as a renderer that could not open the window, to
/// the log. No message.
pub fn note(what: &str) {
    let _ = append(log_path(), &entry("main", what, "", ""));
}

/// The text of a panic's payload.
pub fn payload_text(payload: &(dyn Any + Send)) -> &str {
    payload.downcast_ref::<&str>().copied().or_else(|| payload.downcast_ref::<String>().map(String::as_str)).unwrap_or("no message")
}

fn entry(thread: &str, what: &str, at: &str, backtrace: &str) -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut text = format!(
        "=== Laser Level Webcam {}, {} ===\nSystem: {} {}\nThread: {thread}\nError: {what}\n",
        env!("CARGO_PKG_VERSION"),
        utc_text(secs),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    if !at.is_empty() {
        text += &format!("At: {at}\n");
    }
    if !backtrace.is_empty() {
        text += &format!("Backtrace:\n{backtrace}\n");
    }
    text + "\n"
}

/// Adds `text` to the end of the log, or starts the log again when it is too large.
fn append(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let restart = fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG);
    let mut file = OpenOptions::new().create(true).write(true).append(!restart).truncate(restart).open(path)?;
    file.write_all(text.as_bytes())
}

fn message(stop: Stop, what: &str, log: &Path, written: bool) -> String {
    let head = match stop {
        Stop::App => format!("Laser Level Webcam stopped because of an error:\n\n{what}"),
        Stop::Part => format!(
            "A part of Laser Level Webcam stopped because of an error:\n\n{what}\n\n\
             The camera can stop. Save your session, then close the app and open it again."
        ),
        Stop::Start => format!("Laser Level Webcam could not start:\n\n{what}"),
    };
    let log = log.display();
    if written {
        format!("{head}\n\nThe details are in this file:\n{log}\n\nTo report the problem, attach this file to an issue on GitHub.")
    } else {
        format!("{head}\n\nThe app could not write the details to {log}.")
    }
}

#[cfg(windows)]
fn show(text: &str) {
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Laser Level Webcam")
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Only the log: Linux and macOS apps start from a terminal more often, which shows the panic.
#[cfg(not(windows))]
fn show(_text: &str) {}

/// `YYYY-MM-DD hh:mm:ss UTC` from Unix seconds.
fn utc_text(secs: u64) -> String {
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days to a civil date: H. Hinnant, "chrono-Compatible Low-Level Date Algorithms".
    let z = days + 719_468;
    let (era, doe) = (z / 146_097, z % 146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC", rest / 3600, rest / 60 % 60, rest % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(utc_text(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc_text(951_782_400), "2000-02-29 00:00:00 UTC");
        assert_eq!(utc_text(1_794_809_110), "2026-11-16 06:05:10 UTC");
    }

    #[test]
    fn the_log_grows_then_starts_again() {
        let dir = std::env::temp_dir().join(format!("laser-crash-test-{}", std::process::id()));
        let path = dir.join("sub").join("crash.log");
        let text = entry("driver", "index out of bounds", "src/engine.rs:1:2", "0: main");
        assert!(text.starts_with(&format!("=== Laser Level Webcam {}, ", env!("CARGO_PKG_VERSION"))));
        assert!(text.contains("\nThread: driver\nError: index out of bounds\nAt: src/engine.rs:1:2\nBacktrace:\n0: main\n"));
        append(&path, &text).unwrap();
        append(&path, &text).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), text.repeat(2));
        fs::write(&path, vec![b'x'; MAX_LOG as usize + 1]).unwrap();
        append(&path, &text).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_panic_on_another_thread_is_logged_and_shown_once() {
        static SHOWN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
        fn record(text: &str) {
            SHOWN.lock().unwrap().push(text.to_owned());
        }
        let dir = std::env::temp_dir().join(format!("laser-crash-hook-{}", std::process::id()));
        let log = dir.join("crash.log");
        hook(log.clone(), record);
        let crash = |name: &str| std::thread::Builder::new().name(name.into()).spawn(|| panic!("test panic")).unwrap().join();
        assert!(crash("camera").is_err());
        assert!(crash("driver").is_err());
        drop(panic::take_hook()); // back to the standard hook
        let start = std::time::Instant::now();
        while SHOWN.lock().unwrap().is_empty() && start.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let shown = SHOWN.lock().unwrap().clone();
        assert_eq!(shown.len(), 1, "{shown:?}");
        assert!(shown[0].starts_with("A part of Laser Level Webcam stopped because of an error:\n\ntest panic"));
        assert!(shown[0].contains(&log.display().to_string()));
        let text = fs::read_to_string(&log).unwrap();
        assert!(text.contains("Thread: camera\nError: test panic\nAt: apps"), "{text}");
        assert!(text.contains("Thread: driver\n"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn messages() {
        let log = Path::new("crash.log");
        let part = message(Stop::Part, "boom", log, true);
        assert!(part.starts_with("A part of Laser Level Webcam stopped because of an error:\n\nboom\n\n"));
        assert!(part.contains("Save your session") && part.ends_with("crash.log\n\nTo report the problem, attach this file to an issue on GitHub."));
        assert!(message(Stop::Start, "no adapter", log, false).ends_with("could not start:\n\nno adapter\n\nThe app could not write the details to crash.log."));
        let payload: Box<dyn Any + Send> = Box::new(String::from("owned"));
        assert_eq!(payload_text(payload.as_ref()), "owned");
        let payload: Box<dyn Any + Send> = Box::new(7);
        assert_eq!(payload_text(payload.as_ref()), "no message");
    }
}
