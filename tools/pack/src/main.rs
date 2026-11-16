//! Builds the release files.
//!
//! ```text
//! cargo run -p pack -- icons             the app icon and the installer pictures
//! cargo run -p pack -- web [--no-trunk]  the browser version as one HTML file in dist/
//! cargo run -p pack -- licenses [--target T]... [--out FILE]
//!                                        the licences of the packages in the programs, for
//!                                        these targets (default: this computer), in one text file
//! cargo run -p pack -- check [FILE] [--show] [--timeout S]
//!                                        open that file from its folder in Chrome with a fake
//!                                        camera, and check that the app starts and gets frames
//! ```

mod check;
mod icons;
mod licenses;
mod web;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

/// `canonicalize` without the `\\?\` that Windows puts in front.
fn full(path: &Path) -> std::io::Result<PathBuf> {
    let p = path.canonicalize()?;
    Ok(p.to_str().and_then(|s| s.strip_prefix(r"\\?\")).map(PathBuf::from).unwrap_or(p))
}

fn root() -> PathBuf {
    full(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).expect("the repository folder")
}

fn default_page(root: &Path) -> PathBuf {
    root.join("dist").join(format!("laser-level-webcam-{}-web.html", env!("CARGO_PKG_VERSION")))
}

fn run(args: &[String]) -> Result<(), String> {
    let root = root();
    match args.first().map(String::as_str) {
        Some("icons") => icons::write_all(&root),
        Some("web") => web::build(&root, !args.iter().any(|a| a == "--no-trunk")).map(|_| ()),
        Some("licenses") => {
            let (mut targets, mut out) = (Vec::new(), root.join("target").join("THIRD-PARTY-LICENSES.txt"));
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--target" => targets.push(it.next().ok_or("--target needs a target")?.clone()),
                    "--out" => out = PathBuf::from(it.next().ok_or("--out needs a file")?),
                    other => return Err(format!("unknown argument {other}")),
                }
            }
            if targets.is_empty() {
                targets.push(licenses::host()?);
            }
            let text = licenses::text(&root, &targets)?;
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            std::fs::write(&out, &text).map_err(|e| format!("{}: {e}", out.display()))?;
            println!("wrote {} ({} bytes)", out.display(), text.len());
            Ok(())
        }
        Some("check") => {
            let mut page = default_page(&root);
            let (mut show, mut timeout) = (false, Duration::from_secs(60));
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--show" => show = true,
                    "--timeout" => {
                        let s: u64 = it.next().and_then(|s| s.parse().ok()).ok_or("--timeout needs seconds")?;
                        timeout = Duration::from_secs(s);
                    }
                    other => page = PathBuf::from(other),
                }
            }
            let page = full(&page).map_err(|e| format!("{}: {e}", page.display()))?;
            println!("opening {} from its folder in Chrome", page.display());
            let ok = check::run(&page, show, timeout)?;
            println!("PASS {ok}");
            Ok(())
        }
        _ => Err("usage: pack icons | pack web [--no-trunk] | pack licenses [--target T]... [--out FILE] | pack check [FILE] [--show] [--timeout S]".into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("FAIL {e}");
            ExitCode::FAILURE
        }
    }
}
