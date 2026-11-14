//! On Windows, puts the icon and the file details (name, version, description, copyright) into
//! the exe, so Explorer, the taskbar and the file's Properties show them.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let plist = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("../../packaging/macos/camera-usage.plist");
        println!("cargo:rerun-if-changed={}", plist.display());
        println!("cargo:rustc-link-arg=-Wl,-sectcreate,__TEXT,__info_plist,{}", plist.display());
    }
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/icon.ico")
        .set("ProductName", "Laser Level Webcam")
        .set("FileDescription", "Laser Level Webcam")
        .set("CompanyName", "Bryan Howard")
        .set("LegalCopyright", "Copyright (c) 2023-2026 Bryan Howard. MIT License.")
        .set("OriginalFilename", "laser-level-webcam.exe")
        .set("InternalName", "laser-level-webcam")
        .set("Comments", "Straightness and flatness from a laser line on a lensless webcam.");
    if let Err(e) = res.compile() {
        panic!("could not add the icon and file details to the exe: {e}");
    }
}
