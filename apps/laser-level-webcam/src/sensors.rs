//! Sensors the user has saved, so each camera's sensor is entered once. Each saved sensor names
//! the cameras it belongs to, and is picked when one of them opens.
//!
//! The desktop app keeps the list in `sensors.json` in its data folder; the browser keeps it in
//! its local storage (a cookie would not do: Chrome keeps no cookies for a page opened from disk).
//! The list can be exported to a file and imported again, to move it between the two or to
//! another computer.

use laser_core::camera::Sensor;
use serde::{Deserialize, Serialize};

/// A camera, as far as it can be told apart from other models.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraId {
    /// The name the system gives it, without the USB ids a browser adds to it.
    pub name: String,
    /// USB vendor and product id, as "046d:0825", when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb: Option<String>,
}

impl CameraId {
    /// From the camera's name and, on the desktop, the ids in its USB descriptors. A browser puts
    /// the ids at the end of the name instead, as "HD Webcam C270 (046d:0825)".
    pub fn new(name: &str, usb: Option<(u16, u16)>) -> Self {
        let (bare, in_name) = split_usb(name);
        Self { name: bare.to_owned(), usb: usb.map(|(v, p)| format!("{v:04x}:{p:04x}")).or(in_name) }
    }

    /// The same camera model: the same USB ids when both have them, otherwise the same name.
    pub fn matches(&self, other: &Self) -> bool {
        match (&self.usb, &other.usb) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            _ => !self.name.is_empty() && self.name.eq_ignore_ascii_case(&other.name),
        }
    }
}

/// "HD Webcam C270 (046d:0825)" → ("HD Webcam C270", "046d:0825").
fn split_usb(name: &str) -> (&str, Option<String>) {
    let t = name.trim();
    if let Some(open) = t.strip_suffix(')').and_then(|r| r.rfind('(')) {
        let ids = &t[open + 1..t.len() - 1];
        let hex = |s: &str| s.len() == 4 && s.chars().all(|c| c.is_ascii_hexdigit());
        if let Some((v, p)) = ids.split_once(':')
            && hex(v)
            && hex(p)
        {
            return (t[..open].trim_end(), Some(ids.to_ascii_lowercase()));
        }
    }
    (t, None)
}

/// A sensor the user saved, and the cameras that get it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedSensor {
    #[serde(flatten)]
    pub sensor: Sensor,
    pub cameras: Vec<CameraId>,
}

/// Every saved sensor, sorted by name.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SensorLibrary {
    pub sensors: Vec<SavedSensor>,
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

impl SensorLibrary {
    pub fn get(&self, name: &str) -> Option<&SavedSensor> {
        self.sensors.iter().find(|s| same_name(&s.sensor.name, name))
    }

    /// The saved sensor for this camera, if one names it.
    pub fn for_camera(&self, camera: &CameraId) -> Option<&SavedSensor> {
        self.sensors.iter().find(|s| s.cameras.iter().any(|c| c.matches(camera)))
    }

    /// Save `sensor` under its name, over one of the same name. `camera`, if given, gets this
    /// sensor from now on, and no other.
    pub fn save(&mut self, sensor: &Sensor, camera: Option<&CameraId>) {
        let mut sensor = sensor.clone();
        sensor.name = sensor.name.trim().to_owned();
        let mut cameras = self.get(&sensor.name).map(|s| s.cameras.clone()).unwrap_or_default();
        if let Some(cam) = camera {
            cameras.retain(|c| !c.matches(cam));
            cameras.push(cam.clone());
        }
        self.put(SavedSensor { sensor, cameras });
    }

    /// Add `saved`, over one of the same name; its cameras leave every other saved sensor.
    fn put(&mut self, saved: SavedSensor) {
        self.sensors.retain(|s| !same_name(&s.sensor.name, &saved.sensor.name));
        for other in &mut self.sensors {
            other.cameras.retain(|c| !saved.cameras.iter().any(|n| n.matches(c)));
        }
        self.sensors.push(saved);
        self.sensors.sort_by_key(|s| s.sensor.name.to_lowercase());
    }

    /// Remove the sensor saved as `name`. Says whether there was one.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.sensors.len();
        self.sensors.retain(|s| !same_name(&s.sensor.name, name));
        self.sensors.len() < before
    }

    /// Take in the sensors of an exported list, over any of the same name. Returns how many.
    pub fn merge(&mut self, other: SensorLibrary) -> usize {
        let mut n = 0;
        for s in other.sensors {
            if !s.sensor.name.trim().is_empty() && s.sensor.is_valid() {
                self.put(s);
                n += 1;
            }
        }
        n
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let mut lib: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        lib.sensors.retain(|s| s.sensor.is_valid());
        lib.sensors.sort_by_key(|s| s.sensor.name.to_lowercase());
        Ok(lib)
    }
}

/// The list as it was last saved, and what went wrong reading it, if anything.
pub struct Loaded {
    pub library: SensorLibrary,
    pub problem: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
pub use disk::{load, place, store};

#[cfg(target_arch = "wasm32")]
pub use browser::{load, place, store};

#[cfg(not(target_arch = "wasm32"))]
mod disk {
    use std::path::{Path, PathBuf};

    use super::{Loaded, SensorLibrary};

    fn path() -> Option<PathBuf> {
        eframe::storage_dir(crate::APP_ID).map(|d| d.join("sensors.json"))
    }

    /// Where the list is kept, to show the user.
    pub fn place() -> String {
        path().map_or_else(
            || "This system has no folder for app data, so the list is not kept.".into(),
            |p| format!("The saved sensors are kept in {}", p.display()),
        )
    }

    pub fn load() -> Loaded {
        match path() {
            Some(p) => load_from(&p),
            None => Loaded { library: SensorLibrary::default(), problem: None },
        }
    }

    pub fn store(lib: &SensorLibrary) -> Result<(), String> {
        store_to(&path().ok_or("this system has no folder for app data")?, lib)
    }

    /// A file that cannot be read is kept beside the new list, not written over.
    pub(super) fn load_from(p: &Path) -> Loaded {
        let text = match std::fs::read_to_string(p) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Loaded { library: SensorLibrary::default(), problem: None };
            }
            Err(e) => {
                return Loaded {
                    library: SensorLibrary::default(),
                    problem: Some(format!("Could not read {}: {e}", p.display())),
                };
            }
        };
        match SensorLibrary::from_json(&text) {
            Ok(library) => Loaded { library, problem: None },
            Err(why) => {
                let kept = p.with_extension("unreadable.json");
                let _ = std::fs::rename(p, &kept);
                Loaded {
                    library: SensorLibrary::default(),
                    problem: Some(format!(
                        "Could not read the saved sensors ({why}); the file is kept as {}",
                        kept.display()
                    )),
                }
            }
        }
    }

    /// Written to a new file first, so a crash part way leaves the old list whole.
    pub(super) fn store_to(p: &Path, lib: &SensorLibrary) -> Result<(), String> {
        let err = |e: std::io::Error| format!("Could not save {}: {e}", p.display());
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        let new = p.with_extension("json.new");
        std::fs::write(&new, lib.to_json()).map_err(err)?;
        std::fs::rename(&new, p).map_err(err)
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::{Loaded, SensorLibrary};

    const KEY: &str = "laser-level-webcam.sensors";

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    pub fn place() -> String {
        "The saved sensors are kept in this browser, and go if its site data is cleared. \
Export a copy to keep them safe."
            .into()
    }

    pub fn load() -> Loaded {
        let Some(text) = storage().and_then(|s| s.get_item(KEY).ok().flatten()) else {
            return Loaded { library: SensorLibrary::default(), problem: None };
        };
        match SensorLibrary::from_json(&text) {
            Ok(library) => Loaded { library, problem: None },
            Err(why) => {
                let kept = format!("{KEY}.unreadable");
                if let Some(s) = storage() {
                    let _ = s.set_item(&kept, &text);
                    let _ = s.remove_item(KEY);
                }
                Loaded {
                    library: SensorLibrary::default(),
                    problem: Some(format!("Could not read the saved sensors ({why}); they are kept under {kept}")),
                }
            }
        }
    }

    pub fn store(lib: &SensorLibrary) -> Result<(), String> {
        let s = storage().ok_or("this browser gives no storage to the page")?;
        s.set_item(KEY, &lib.to_json()).map_err(|e| format!("Could not save the sensors: {e:?}"))
    }
}

#[cfg(test)]
mod tests {
    use laser_core::camera::ModeScaling;

    use super::*;

    fn sensor(name: &str, pitch: f64) -> Sensor {
        Sensor { name: name.into(), pixel_pitch_um: pitch, width_px: 1920, height_px: 1080, scaling: ModeScaling::Scaled }
    }

    #[test]
    fn a_browser_name_gives_the_usb_ids() {
        let c = CameraId::new("HD Webcam C270 (046D:0825)", None);
        assert_eq!(c, CameraId { name: "HD Webcam C270".into(), usb: Some("046d:0825".into()) });
        let d = CameraId::new("HD Webcam C270", Some((0x046d, 0x0825)));
        assert_eq!(c, d);
        assert_eq!(CameraId::new("Cam (front)", None).usb, None);
        assert_eq!(CameraId::new("Cam (front)", None).name, "Cam (front)");
    }

    #[test]
    fn cameras_match_by_usb_ids_then_by_name() {
        let a = CameraId::new("USB Camera", Some((0x0c45, 0x6366)));
        let b = CameraId::new("USB Camera", Some((0x05a3, 0x9230)));
        assert!(!a.matches(&b), "the same name with other ids is another camera");
        assert!(a.matches(&CameraId::new("usb camera", None)));
        assert!(!CameraId::default().matches(&CameraId::default()));
    }

    #[test]
    fn saving_replaces_by_name_and_a_camera_has_one_sensor() {
        let cam = CameraId::new("HD Webcam C270", Some((0x046d, 0x0825)));
        let mut lib = SensorLibrary::default();
        lib.save(&sensor("OV2710", 3.0), Some(&cam));
        lib.save(&sensor(" ov2710 ", 2.8), None);
        assert_eq!(lib.sensors.len(), 1);
        assert_eq!(lib.sensors[0].sensor.pixel_pitch_um, 2.8);
        assert_eq!(lib.sensors[0].cameras, std::slice::from_ref(&cam), "saving again keeps its cameras");

        lib.save(&sensor("IMX291", 2.9), Some(&cam));
        assert_eq!(lib.for_camera(&cam).unwrap().sensor.name, "IMX291");
        assert!(lib.get("OV2710").unwrap().cameras.is_empty());
        // The name is as last typed, trimmed.
        assert_eq!(lib.sensors.iter().map(|s| s.sensor.name.as_str()).collect::<Vec<_>>(), ["IMX291", "ov2710"]);
        assert!(lib.remove("imx291"));
        assert!(lib.for_camera(&cam).is_none());
    }

    #[test]
    fn the_file_is_plain_json_and_reads_back() {
        let mut lib = SensorLibrary::default();
        lib.save(&sensor("OV2710", 3.0), Some(&CameraId::new("USB Camera (05a3:9230)", None)));
        let text = lib.to_json();
        assert!(text.contains("\"pixel_pitch_um\": 3.0"), "{text}");
        assert!(text.contains("\"usb\": \"05a3:9230\""), "{text}");
        assert_eq!(SensorLibrary::from_json(&text).unwrap(), lib);
        // Missing fields take their defaults; a sensor with no size is left out.
        let old = r#"{"sensors":[{"name":"A","pixel_pitch_um":1.4},{"name":"B","pixel_pitch_um":0}]}"#;
        let read = SensorLibrary::from_json(old).unwrap();
        assert_eq!(read.sensors.len(), 1);
        assert_eq!(read.sensors[0].sensor.width_px, 1920);
        assert!(read.sensors[0].cameras.is_empty());
    }

    #[test]
    fn importing_merges_by_name() {
        let cam = CameraId::new("USB Camera", None);
        let mut lib = SensorLibrary::default();
        lib.save(&sensor("A", 3.0), Some(&cam));
        lib.save(&sensor("B", 2.0), None);
        let mut other = SensorLibrary::default();
        other.save(&sensor("B", 2.2), Some(&cam));
        other.save(&sensor("C", 1.1), None);
        assert_eq!(lib.merge(other), 2);
        assert_eq!(lib.sensors.len(), 3);
        assert_eq!(lib.get("B").unwrap().sensor.pixel_pitch_um, 2.2);
        assert_eq!(lib.for_camera(&cam).unwrap().sensor.name, "B");
        assert!(lib.get("A").unwrap().cameras.is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_file_is_written_whole_and_a_bad_one_is_kept() {
        let dir = std::env::temp_dir().join(format!("laser-sensors-test-{}", std::process::id()));
        let file = dir.join("sub").join("sensors.json");
        assert!(disk::load_from(&file).library.sensors.is_empty());
        let mut lib = SensorLibrary::default();
        lib.save(&sensor("OV2710", 3.0), None);
        disk::store_to(&file, &lib).unwrap();
        let back = disk::load_from(&file);
        assert!(back.problem.is_none());
        assert_eq!(back.library, lib);

        std::fs::write(&file, "{ not json").unwrap();
        let bad = disk::load_from(&file);
        assert!(bad.library.sensors.is_empty());
        assert!(bad.problem.unwrap().contains("kept as"));
        assert!(!file.exists());
        assert_eq!(std::fs::read_to_string(file.with_extension("unreadable.json")).unwrap(), "{ not json");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
