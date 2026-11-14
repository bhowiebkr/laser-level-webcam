//! The browser version as one HTML file.
//!
//! trunk builds the app into a page, a JavaScript file and a WebAssembly file. A browser does not
//! let a page that is opened from a folder load other files, so that page needs a web server. This
//! puts the JavaScript and the WebAssembly (compressed, as base64) into the page itself. The file
//! then starts with a double-click, from any folder, with no server.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::icons;

const GLUE_EXPORT: &str = "export { initSync, __wbg_init as default };";

/// Starts the app from the WebAssembly in the page. On a failure, the page says why instead of
/// "Loading…".
const BOOT: &str = r#"
try {
    const text = atob(document.getElementById("app-wasm").textContent.trim());
    const packed = new Uint8Array(text.length);
    for (let i = 0; i < text.length; i++) packed[i] = text.charCodeAt(i);
    const stream = new Blob([packed]).stream().pipeThrough(new DecompressionStream("gzip"));
    const bytes = await new Response(stream).arrayBuffer();
    await __wbg_init({ module_or_path: bytes });
} catch (e) {
    const box = document.getElementById("loading");
    if (box) box.textContent = "The app could not start: " + e;
    throw e;
}
"#;

pub fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= chunk.len() { A[(n >> shift & 63) as usize] as char } else { '=' });
        }
    }
    out
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(bytes).expect("gzip into memory");
    enc.finish().expect("gzip into memory")
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The one file in `dir` whose name ends in `suffix`.
fn only(dir: &Path, suffix: &str) -> Result<PathBuf, String> {
    let found: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(suffix)))
        .collect();
    match found.as_slice() {
        [one] => Ok(one.clone()),
        _ => Err(format!("{} has {} files ending in {suffix}; expected 1", dir.display(), found.len())),
    }
}

/// Builds the app with trunk and writes `dist/laser-level-webcam-<version>-web.html`.
pub fn build(root: &Path, run_trunk: bool) -> Result<PathBuf, String> {
    let app = root.join("apps/laser-level-webcam");
    if run_trunk {
        let status = Command::new("trunk")
            .args(["build", "--release"])
            .current_dir(&app)
            .status()
            .map_err(|e| format!("running trunk: {e}. Install it with `cargo install trunk --locked`"))?;
        if !status.success() {
            return Err("trunk build failed".into());
        }
    }
    let built = app.join("dist");
    let glue = String::from_utf8(read(&only(&built, ".js")?)?).map_err(|e| format!("the JavaScript file: {e}"))?;
    if !glue.contains(GLUE_EXPORT) || glue.contains("</script") || glue.lines().any(|l| l.starts_with("import ")) {
        return Err("the JavaScript file from wasm-bindgen has changed shape; check web.rs".into());
    }
    let glue = glue.replace(GLUE_EXPORT, "");
    let wasm = read(&only(&built, "_bg.wasm")?)?;
    let packed = base64(&gzip(&wasm));

    let source = String::from_utf8(read(&app.join("index.html"))?).map_err(|e| format!("index.html: {e}"))?;
    let page: String = source
        .lines()
        .filter(|l| !l.contains("data-trunk") && !l.contains("trunk serve"))
        .map(|l| format!("{l}\n"))
        .collect();
    let favicon =
        format!("    <link rel=\"icon\" type=\"image/png\" href=\"data:image/png;base64,{}\">\n", base64(&icons::png(&icons::icon(64))));
    let scripts = format!(
        "    <script id=\"app-wasm\" type=\"application/octet-stream\">{packed}</script>\n    <script type=\"module\">\n{glue}\n{BOOT}</script>\n"
    );
    let (Some(head), Some(body)) = (page.find("</head>"), page.find("</body>")) else {
        return Err("index.html has no </head> or </body>".into());
    };
    let page = format!("{}{favicon}{}{scripts}{}", &page[..head], &page[head..body], &page[body..]);

    let out = root.join("dist").join(format!("laser-level-webcam-{}-web.html", env!("CARGO_PKG_VERSION")));
    std::fs::create_dir_all(root.join("dist")).map_err(|e| format!("dist: {e}"))?;
    std::fs::write(&out, &page).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("wrote {} ({} bytes; WebAssembly {} bytes)", out.display(), page.len(), wasm.len());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xfb, 0xff]), "+/8=");
    }
}
