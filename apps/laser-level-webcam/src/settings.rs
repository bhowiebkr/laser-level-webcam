//! What the app remembers between runs: the camera and its controls, the sensor, the finder, the
//! sample rule and the planned run. Saved by eframe (a file natively, local storage in the
//! browser). The measurement itself is kept apart, in the session.

use laser_core::beam::FinderConfig;
use laser_core::camera::{CameraSettings, ControlSetting, Sensor, VideoMode};
use laser_core::frame::{Orientation, Roi};
use laser_core::monitor::RunPlan;
use laser_core::reading::SampleRule;
use laser_core::survey::DriftCorrection;
use laser_core::units::Unit;
use serde::{Deserialize, Serialize};

pub const KEY: &str = "settings";
pub const SESSION_KEY: &str = "session";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct AppSettings {
    pub camera: CameraSettings,
    pub sensor: Sensor,
    pub finder: FinderConfig,
    pub sample: SampleRule,
    pub plan: Plan,
    pub unit: Unit,
    /// How the camera's image is turned to show the world. It sets the finder's line axis and
    /// the sign of the heights.
    pub orientation: Orientation,
    pub drift_correction: DriftCorrection,
    /// The camera that `camera`'s mode and controls, `orientation` and `finder.roi` belong to.
    /// `None` in settings saved before each camera kept its own.
    pub setup_for: Option<String>,
    /// Every other camera's setup, kept for when that camera is used again.
    pub cameras: Vec<CameraSetup>,
}

/// What belongs to one camera and not to the next: its mode, its controls (a manual exposure
/// or zoom from one camera is wrong on another), how its image is turned, and the region
/// drawn on its frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraSetup {
    /// The camera's name, as the camera list shows it.
    pub name: String,
    pub mode: Option<VideoMode>,
    pub controls: Vec<ControlSetting>,
    pub orientation: Orientation,
    pub roi: Option<Roi>,
}

impl AppSettings {
    /// The finder reads across the line the way the orientation has it.
    pub fn normalized(mut self) -> Self {
        self.finder.axis = self.orientation.axis();
        self
    }

    /// Change the setup to the camera `name`'s. The setup in use is kept for the camera it
    /// belongs to ([`Self::setup_for`], or `was` for settings from before cameras kept their
    /// own), and `name` gets its own setup back, or the defaults if it has never been used.
    /// Returns whether the mode or the controls changed, which only a camera opened again
    /// takes in.
    pub fn use_setup_of(&mut self, name: &str, was: Option<&str>) -> bool {
        let Some(from) = self.setup_for.clone().or_else(|| was.map(str::to_owned)) else {
            // Nothing says whose the setup is: it is this camera's.
            self.setup_for = Some(name.to_owned());
            return false;
        };
        if from == name {
            self.setup_for = Some(from);
            return false;
        }
        let kept = CameraSetup {
            name: from.clone(),
            mode: self.camera.mode,
            controls: std::mem::take(&mut self.camera.controls),
            orientation: self.orientation,
            roi: self.finder.roi,
        };
        let next = match self.cameras.iter().position(|c| c.name == name) {
            Some(i) => self.cameras.remove(i),
            None => CameraSetup { name: name.to_owned(), ..CameraSetup::default() },
        };
        let changed = kept.mode != next.mode || kept.controls != next.controls;
        self.cameras.retain(|c| c.name != from);
        self.cameras.push(kept);
        self.camera.mode = next.mode;
        self.camera.controls = next.controls;
        self.orientation = next.orientation;
        self.finder.roi = next.roi;
        self.finder.axis = self.orientation.axis();
        self.setup_for = Some(name.to_owned());
        changed
    }
}

/// The run the user is about to measure, which says how calm the set-up must be.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Plan {
    pub run_min: f64,
    /// Drift allowed over the run, µm.
    pub tolerance_um: f64,
}

impl Default for Plan {
    fn default() -> Self {
        Self { run_min: 10.0, tolerance_um: 1.0 }
    }
}

impl Plan {
    pub fn run_plan(&self) -> RunPlan {
        RunPlan { run_s: self.run_min * 60.0, tolerance: self.tolerance_um }
    }
}

#[cfg(test)]
mod tests {
    use laser_core::camera::{ControlId, ControlMode, PixelFormat};

    use super::*;

    /// The setup of a camera used for a while: a mode, a manual exposure and zoom, a turned
    /// view and a drawn region.
    fn used(name: &str) -> AppSettings {
        let mut s = AppSettings::default();
        s.camera.device = Some(name.into());
        s.camera.mode = Some(VideoMode { width: 1920, height: 1080, fps: 5.0, format: PixelFormat::Yuyv });
        s.camera.set(ControlId::Exposure, ControlMode::Manual(-7));
        s.camera.set(ControlId::Zoom, ControlMode::Manual(300));
        s.orientation = s.orientation.turned_cw();
        s.finder.roi = Some(Roi { x: 480, y: 0, width: 960, height: 1080 });
        s.setup_for = Some(name.into());
        s.normalized()
    }

    #[test]
    fn a_new_camera_starts_from_the_defaults_not_the_last_cameras_setup() {
        let mut s = used("C270");
        let c270 = s.clone();
        assert!(s.use_setup_of("B525", None), "the controls change, so the camera opens again");
        assert_eq!(s.camera.mode, None);
        assert!(s.camera.controls.is_empty());
        assert_eq!(s.orientation, Orientation::default());
        assert_eq!(s.finder.roi, None);
        assert_eq!(s.finder.axis, Orientation::default().axis());
        assert_eq!(s.setup_for.as_deref(), Some("B525"));

        // Back to the first camera: everything it had comes back.
        s.orientation = s.orientation.flipped_left_right();
        assert!(s.use_setup_of("C270", None));
        assert_eq!(s.camera.mode, c270.camera.mode);
        assert_eq!(s.camera.controls, c270.camera.controls);
        assert_eq!(s.orientation, c270.orientation);
        assert_eq!(s.finder.roi, c270.finder.roi);
        assert_eq!(s.finder.axis, c270.orientation.axis());

        // And the second camera keeps what it was given, once each.
        assert_eq!(s.cameras.len(), 1);
        assert_eq!(s.cameras[0].name, "B525");
        assert_eq!(s.cameras[0].orientation, Orientation::default().flipped_left_right());
    }

    #[test]
    fn the_same_camera_keeps_its_setup() {
        let mut s = used("C270");
        let before = s.clone();
        assert!(!s.use_setup_of("C270", Some("B525")));
        assert_eq!(s, before);
    }

    #[test]
    fn settings_from_before_belong_to_the_camera_that_was_open() {
        // Saved before cameras kept their own setups: nothing says whose they are.
        let mut s = used("C270");
        s.setup_for = None;
        let before = s.clone();

        // The first camera that opens takes them as they are.
        assert!(!s.use_setup_of("C270", None));
        assert_eq!(s.setup_for.as_deref(), Some("C270"));
        assert_eq!(
            (s.camera.controls.clone(), s.orientation, s.finder.roi),
            (before.camera.controls, before.orientation, before.finder.roi)
        );

        // Picking another camera before any has opened: the setup is the one in the list's.
        let mut s = used("C270");
        s.setup_for = None;
        s.use_setup_of("B525", Some("C270"));
        assert!(s.camera.controls.is_empty());
        assert_eq!(s.cameras[0].name, "C270");
    }

    #[test]
    fn settings_saved_without_the_camera_setups_still_load() {
        let mut old = serde_json::to_value(used("C270")).unwrap();
        let map = old.as_object_mut().unwrap();
        map.remove("setup_for");
        map.remove("cameras");
        let s: AppSettings = serde_json::from_value(old).unwrap();
        assert_eq!(s.setup_for, None);
        assert!(s.cameras.is_empty());
        assert_eq!(s.camera.controls, used("C270").camera.controls);
    }
}
