//! Opens the one-file browser version from a folder in Chrome, the same way a user does with a
//! double-click, and checks that the app starts and gets frames from a camera.
//!
//! Chrome gets a fake camera and a new empty profile. The check copies the page and adds a script
//! that watches the start-up and puts the state in the page title. Chrome's debugging port gives
//! the title back, so the check needs no other tools.

use std::io::{Read, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Runs before the app. It records what the app gets from `getUserMedia` and reports in the title.
const WATCH: &str = r#"<script>
(() => {
    const report = (s) => { document.title = "CHECK " + s; };
    let camera = "not asked";
    const md = navigator.mediaDevices;
    if (md && md.getUserMedia) {
        const real = md.getUserMedia.bind(md);
        md.getUserMedia = async (c) => {
            camera = "asked";
            try {
                const s = await real(c);
                const t = s.getVideoTracks()[0];
                const st = t ? t.getSettings() : {};
                camera = `live ${st.width}x${st.height}`;
                return s;
            } catch (e) { camera = "refused: " + e.name; throw e; }
        };
    }
    let failed = null;
    addEventListener("error", (e) => { failed ??= e.message; });
    addEventListener("unhandledrejection", (e) => { failed ??= String(e.reason); });
    const t0 = performance.now();
    const tick = () => {
        const box = document.getElementById("loading");
        const ms = Math.round(performance.now() - t0);
        const canvas = document.getElementById("laser_canvas");
        if (failed || (box && box.textContent.startsWith("The app could not start"))) {
            report("FAILED " + (failed || box.textContent));
        } else if (!box && camera.startsWith("live")) {
            report(`OK the app started in ${ms} ms; canvas ${canvas.width}x${canvas.height}; camera ${camera}`);
        } else {
            report(`WAIT loading=${!!box} camera=${camera} ${ms} ms`);
            setTimeout(tick, 100);
        }
    };
    setTimeout(tick, 0);
})();
</script>
"#;

fn find_chrome() -> Result<PathBuf, String> {
    let mut places = vec![
        PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
        PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
        PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"),
        PathBuf::from("/usr/bin/google-chrome"),
        PathBuf::from("/usr/bin/chromium"),
        PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
    ];
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        places.insert(0, Path::new(&local).join(r"Google\Chrome\Application\chrome.exe"));
    }
    places.into_iter().find(|p| p.is_file()).ok_or_else(|| "no Chrome or Edge found".into())
}

/// `file:///` address of a local path.
fn file_url(path: &Path) -> String {
    let p = path.to_string_lossy().replace('\\', "/");
    let p = p.trim_start_matches('/');
    let mut url = String::from("file:///");
    for b in p.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b':' | b'-' | b'_' | b'.' | b'~' => url.push(b as char),
            _ => url.push_str(&format!("%{b:02X}")),
        }
    }
    url
}

fn http_get(port: u16, path: &str) -> Option<String> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").ok()?;
    // Chrome can keep the connection open, so read only as far as Content-Length.
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = s.read(&mut buf).ok()?;
        data.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&data);
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let len = head
                .lines()
                .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("content-length")))
                .and_then(|(_, v)| v.trim().parse::<usize>().ok());
            if n == 0 || len.is_some_and(|len| body.len() >= len) {
                return Some(body.to_owned());
            }
        } else if n == 0 {
            return None;
        }
    }
}

/// The title of the page that Chrome has open.
fn title(port: u16) -> Option<String> {
    let list: serde_json::Value = serde_json::from_str(&http_get(port, "/json/list")?).ok()?;
    list.as_array()?.iter().find(|p| p["type"] == "page").and_then(|p| p["title"].as_str()).map(str::to_owned)
}

struct Browser {
    child: Child,
    work: PathBuf,
    copy: PathBuf,
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.copy);
        // Chrome lets go of its profile a moment after it exits.
        for _ in 0..50 {
            if !self.work.exists() || std::fs::remove_dir_all(&self.work).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("could not remove {}", self.work.display());
    }
}

/// Checks `page`. With `show`, Chrome opens a window; otherwise it runs headless.
pub fn run(page: &Path, show: bool, timeout: Duration) -> Result<String, String> {
    let chrome = find_chrome()?;
    let work = std::env::temp_dir().join(format!("laser-pack-check-{}", std::process::id()));
    let profile = work.join("profile");
    std::fs::create_dir_all(&profile).map_err(|e| format!("{}: {e}", profile.display()))?;
    let html = std::fs::read_to_string(page).map_err(|e| format!("{}: {e}", page.display()))?;
    let at = html.find("<body>").map(|i| i + "<body>".len()).ok_or("the page has no <body>")?;
    // The copy sits next to the page, so it sees the same folder that the page sees.
    let stem = page.file_stem().and_then(|s| s.to_str()).ok_or("no file name")?;
    let copy = page.with_file_name(format!("{stem}.check-{}.html", std::process::id()));
    std::fs::write(&copy, format!("{}\n{WATCH}{}", &html[..at], &html[at..])).map_err(|e| format!("{}: {e}", copy.display()))?;

    let mut args = vec![
        format!("--user-data-dir={}", profile.display()),
        "--remote-debugging-port=0".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--disable-extensions".into(),
        "--use-fake-device-for-media-stream".into(),
        "--use-fake-ui-for-media-stream".into(),
    ];
    if !show {
        args.extend(["--headless=new".into(), "--enable-unsafe-swiftshader".into()]);
    }
    args.push(file_url(&copy));
    let child = Command::new(&chrome)
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            let _ = std::fs::remove_file(&copy);
            let _ = std::fs::remove_dir_all(&work);
            format!("{}: {e}", chrome.display())
        })?;
    let browser = Browser { child, work, copy };

    let start = Instant::now();
    let port_file = profile.join("DevToolsActivePort");
    let mut port = None;
    let mut last = String::from("Chrome did not start");
    while start.elapsed() < timeout {
        std::thread::sleep(Duration::from_millis(200));
        if port.is_none() {
            port = std::fs::read_to_string(&port_file).ok().and_then(|t| t.lines().next()?.trim().parse().ok());
            if port.is_some() {
                last = "Chrome started; no page title yet".into();
            }
            continue;
        }
        if let Some(t) = port.and_then(title) {
            last = t;
            if last.starts_with("CHECK OK") {
                drop(browser);
                return Ok(last.trim_start_matches("CHECK OK ").to_owned());
            }
            if last.starts_with("CHECK FAILED") {
                break;
            }
        }
    }
    drop(browser);
    Err(format!("{} after {:.0} s", last.trim_start_matches("CHECK "), start.elapsed().as_secs_f64()))
}
