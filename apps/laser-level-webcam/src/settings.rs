//! What the app remembers between runs: the camera and its controls, the sensor, the finder, the
//! sample rule and the planned run. Saved by eframe (a file natively, local storage in the
//! browser). The measurement itself is kept apart, in the session.

use laser_core::beam::FinderConfig;
use laser_core::camera::{CameraSettings, Sensor};
use laser_core::frame::Orientation;
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
}

impl AppSettings {
    /// The finder reads across the line the way the orientation has it.
    pub fn normalized(mut self) -> Self {
        self.finder.axis = self.orientation.axis();
        self
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
