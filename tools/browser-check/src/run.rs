//! Builds the check page, serves it, opens it in a browser and prints what it sends back.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

const HELP: &str = "\
Checks the browser camera and its controls in a real browser.

Usage: browser-check [OPTIONS]

Options:
  --fake              Use the browser's test camera, not the real ones
  --camera <TEXT>     Check only the cameras whose name has this text
  --browser <NAME>    chrome (the default), edge, brave, or the path to one
  --show              Show the browser window
  --no-build          Use the page built last time
  --timeout <S>       Give up after this many seconds [default: 600]
  -h, --help          Print this help";

struct Options {
    browser: String,
    fake: bool,
    show: bool,
    build: bool,
    camera: Option<String>,
    timeout: Duration,
}

fn options() -> Result<Option<Options>, String> {
    let mut o = Options {
        browser: "chrome".into(),
        fake: false,
        show: false,
        build: true,
        camera: None,
        timeout: Duration::from_secs(600),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--fake" => o.fake = true,
            "--show" => o.show = true,
            "--no-build" => o.build = false,
            "--camera" => o.camera = Some(value()?),
            "--browser" => o.browser = value()?,
            "--timeout" => {
                let s: u64 = value()?.parse().map_err(|_| "--timeout needs a whole number of seconds")?;
                o.timeout = Duration::from_secs(s);
            }
            "-h" | "--help" => return Ok(None),
            _ => return Err(format!("unknown argument {arg}; try --help")),
        }
    }
    Ok(Some(o))
}

pub fn main() -> ExitCode {
    let o = match options() {
        Ok(Some(o)) => o,
        Ok(None) => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("browser-check: {e}");
            return ExitCode::from(2);
        }
    };
    match run(&o) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("browser-check: {e}");
            ExitCode::from(2)
        }
    }
}

fn run(o: &Options) -> Result<bool, String> {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    if o.build {
        let status = Command::new("trunk")
            .args(["build", "--release"])
            .current_dir(here)
            .status()
            .map_err(|e| format!("running trunk: {e}. Install it with `cargo install trunk`"))?;
        if !status.success() {
            return Err("trunk build failed".into());
        }
    }
    let dist = here.join("dist");
    if !dist.join("index.html").is_file() {
        return Err(format!("{} has no built page; run without --no-build", dist.display()));
    }

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("serving the page: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || serve(&listener, &dist, &tx));

    let mut url = format!("http://127.0.0.1:{port}/");
    if let Some(c) = &o.camera {
        url.push_str("?camera=");
        url.push_str(&encode(c));
    }
    let profile = std::env::temp_dir().join(format!("laser-browser-check-{}", std::process::id()));
    let mut args = vec![
        format!("--user-data-dir={}", profile.display()),
        "--use-fake-ui-for-media-stream".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--disable-background-timer-throttling".into(),
        "--disable-renderer-backgrounding".into(),
    ];
    if !o.show {
        args.push("--headless=new".into());
    }
    if o.fake {
        args.push("--use-fake-device-for-media-stream".into());
    }
    args.push(url.clone());
    let exe = browser(&o.browser)?;
    println!("Opening {url} in {}", exe.display());
    let mut child = Command::new(&exe)
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("starting {}: {e}", exe.display()))?;

    let start = Instant::now();
    let (mut passed, mut failed, mut finished) = (0u32, 0u32, false);
    while let Some(left) = o.timeout.checked_sub(start.elapsed()) {
        let Ok(line) = rx.recv_timeout(left) else { break };
        println!("{line}");
        if line.starts_with("PASS ") {
            passed += 1;
        } else if line.starts_with("FAIL ") {
            failed += 1;
        } else if line.starts_with("DONE ") {
            finished = true;
            break;
        }
    }

    stop(&mut child);
    remove(&profile);
    if !finished {
        println!("FAIL browser-check: the page did not finish in {} s", o.timeout.as_secs());
        failed += 1;
    }
    println!("{passed} passed, {failed} failed");
    Ok(failed == 0)
}

/// The browser to run: a known name, or a path.
fn browser(name: &str) -> Result<PathBuf, String> {
    let known: &[&str] = match name {
        "chrome" => &[
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/usr/bin/google-chrome",
            "/usr/bin/chromium",
        ],
        "edge" => &[
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/usr/bin/microsoft-edge",
        ],
        "brave" => &[
            r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            "/usr/bin/brave-browser",
        ],
        path => {
            let p = PathBuf::from(path);
            return if p.is_file() { Ok(p) } else { Err(format!("no browser at {path}")) };
        }
    };
    let mut places: Vec<PathBuf> = known.iter().map(PathBuf::from).collect();
    if name == "chrome"
        && let Some(local) = std::env::var_os("LOCALAPPDATA")
    {
        places.push(Path::new(&local).join(r"Google\Chrome\Application\chrome.exe"));
    }
    places.into_iter().find(|p| p.is_file()).ok_or_else(|| format!("{name} is not installed; try --browser <path>"))
}

/// Stop the browser and every process it started.
fn stop(child: &mut std::process::Child) {
    #[cfg(windows)]
    let _ = Command::new("taskkill")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

/// Remove the browser's throwaway profile; its processes may hold files for a moment.
fn remove(profile: &Path) {
    for _ in 0..40 {
        if !profile.exists() || std::fs::remove_dir_all(profile).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    eprintln!("browser-check: could not remove {}", profile.display());
}

/// Percent-encode a value for the page's address.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => char::from(b).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn serve(listener: &TcpListener, dist: &Path, tx: &Sender<String>) {
    for stream in listener.incoming().flatten() {
        let (dist, tx) = (dist.to_owned(), tx.clone());
        std::thread::spawn(move || {
            let _ = answer(stream, &dist, &tx);
        });
    }
}

/// Answer one request: a built file, or a result line from the page.
fn answer(stream: TcpStream, dist: &Path, tx: &Sender<String>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut first = String::new();
    reader.read_line(&mut first)?;
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut parts = first.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let path = target.split('?').next().unwrap_or("/");
    let mut out = stream;
    if method == "POST" && path == "/result" {
        let mut body = vec![0; length];
        reader.read_exact(&mut body)?;
        for line in String::from_utf8_lossy(&body).lines() {
            let _ = tx.send(line.to_owned());
        }
        return out.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
    }
    let rel = if path == "/" { "index.html" } else { path.trim_start_matches('/') };
    let file = Path::new(rel);
    let inside = file.components().all(|c| matches!(c, Component::Normal(_)));
    match std::fs::read(dist.join(file)).ok().filter(|_| method == "GET" && inside) {
        Some(body) => {
            let kind = match file.extension().and_then(|e| e.to_str()) {
                Some("html") => "text/html; charset=utf-8",
                Some("js") => "text/javascript",
                Some("wasm") => "application/wasm",
                Some("css") => "text/css",
                _ => "application/octet-stream",
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                body.len()
            );
            out.write_all(head.as_bytes())?;
            out.write_all(&body)
        }
        None => out.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
    }
}
