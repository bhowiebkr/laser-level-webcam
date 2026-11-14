//! Saving and opening files: a dialog on the desktop, a download and a file picker in the browser.

use std::sync::{Arc, Mutex};

/// A file name from a run or session name, with nothing a file system refuses.
pub fn file_stem(name: &str) -> String {
    let s: String =
        name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ' | '.') { c } else { '_' }).collect();
    let s = s.trim().trim_matches('.').to_owned();
    if s.is_empty() { "laser-level".to_owned() } else { s }
}

/// Save `text` as a file the user picks, named `name` to start with. Says what happened.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_text(name: &str, _mime: &str, text: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("txt").to_owned();
    let Some(path) = rfd::FileDialog::new().set_file_name(name).add_filter(ext.to_uppercase(), &[&ext]).save_file()
    else {
        return String::new();
    };
    match std::fs::write(&path, text) {
        Ok(()) => format!("Saved {}", path.display()),
        Err(e) => format!("Could not save {}: {e}", path.display()),
    }
}

/// Save `text` as a download named `name`. Says what happened.
#[cfg(target_arch = "wasm32")]
pub fn save_text(name: &str, mime: &str, text: &str) -> String {
    match download(name, mime, text) {
        Ok(()) => format!("Downloaded {name}"),
        Err(e) => format!("Could not download {name}: {e:?}"),
    }
}

#[cfg(target_arch = "wasm32")]
fn download(name: &str, mime: &str, text: &str) -> Result<(), wasm_bindgen::JsValue> {
    use wasm_bindgen::JsCast;
    let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(text));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(&format!("{mime};charset=utf-8"));
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &options)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;
    let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a")?.dyn_into()?;
    a.set_href(&url);
    a.set_download(name);
    // Firefox follows only a link on the page.
    let body = document.body().ok_or("no body")?;
    body.append_child(&a)?;
    a.click();
    a.remove();
    // Some browsers read the file after the click returns; let it go a minute later.
    let free = wasm_bindgen::closure::Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    web_sys::window()
        .ok_or("no window")?
        .set_timeout_with_callback_and_timeout_and_arguments_0(free.unchecked_ref(), 60_000)?;
    Ok(())
}

/// A text file the user picked, once it has been read.
#[derive(Clone, Default)]
pub struct Inbox(Arc<Mutex<Option<(String, String)>>>);

impl Inbox {
    /// The file's name and text, once.
    pub fn take(&self) -> Option<(String, String)> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }

    fn put(&self, name: String, text: String) {
        *self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((name, text));
    }

    /// Ask for a file with extension `ext`; its text arrives here.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(&self, ext: &str, _repaint: impl Fn() + 'static) -> Option<String> {
        let path = rfd::FileDialog::new().add_filter(ext.to_uppercase(), &[ext]).pick_file()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                self.put(name, text);
                None
            }
            Err(e) => Some(format!("Could not read {}: {e}", path.display())),
        }
    }

    /// Ask for a file with extension `ext`; its text arrives here.
    #[cfg(target_arch = "wasm32")]
    pub fn open(&self, ext: &str, repaint: impl Fn() + 'static) -> Option<String> {
        let inbox = self.clone();
        let ext = ext.to_owned();
        wasm_bindgen_futures::spawn_local(async move {
            let Some(file) =
                rfd::AsyncFileDialog::new().add_filter(ext.to_uppercase(), &[ext.as_str()]).pick_file().await
            else {
                return;
            };
            let bytes = file.read().await;
            inbox.put(file.file_name(), String::from_utf8_lossy(&bytes).into_owned());
            repaint();
        });
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_are_safe() {
        assert_eq!(file_stem("Line 1"), "Line 1");
        assert_eq!(file_stem("a/b:c"), "a_b_c");
        assert_eq!(file_stem("  ..  "), "laser-level");
    }
}
