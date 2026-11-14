//! Stamps the commit and the target into the program for `laser report`. On Windows, also puts
//! the icon and the file details (name, version, description, copyright) into the exe, so
//! Explorer and the file's Properties show them.

use std::path::Path;
use std::process::Command;

/// The trimmed output of a git command, if it ran and printed something.
fn git(args: &[&str]) -> Option<String> {
    let o = Command::new("git").args(args).output().ok().filter(|o| o.status.success())?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn stamp() {
    let commit = git(&["rev-parse", "--short=10", "HEAD"])
        .or_else(|| std::env::var("GITHUB_SHA").ok().map(|s| s.chars().take(10).collect()));
    if let Some(c) = commit {
        println!("cargo:rustc-env=LASER_COMMIT={c}");
    }
    if let Ok(t) = std::env::var("TARGET") {
        println!("cargo:rustc-env=LASER_TARGET={t}");
    }
    // Build again when the commit changes, and only then.
    let head_ref = git(&["symbolic-ref", "-q", "HEAD"]);
    for name in ["HEAD", "packed-refs"].into_iter().map(str::to_string).chain(head_ref) {
        if let Some(path) = git(&["rev-parse", "--git-path", &name]).filter(|p| Path::new(p).exists()) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    stamp();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/icon.ico")
        .set("ProductName", "Laser Level Webcam")
        .set("FileDescription", "Laser Level Webcam command line")
        .set("CompanyName", "Bryan Howard")
        .set("LegalCopyright", "Copyright (c) 2023-2026 Bryan Howard. MIT License.")
        .set("OriginalFilename", "laser.exe")
        .set("InternalName", "laser")
        .set("Comments", "Command line for the laser level webcam. Every command prints JSON.");
    if let Err(e) = res.compile() {
        panic!("could not add the icon and file details to the exe: {e}");
    }
}
