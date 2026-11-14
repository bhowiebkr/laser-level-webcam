//! Core of the laser level webcam: a webcam sensor with no lens, in a rigid block, reads where a
//! laser line (or a collimated beam) crosses it. Readings at stations, relative to a zero station,
//! give straightness and flatness.
//!
//! This crate has no I/O and builds for `wasm32-unknown-unknown`.
//!
//! - [`frame`]: a view of an 8-bit luma plane.
//! - [`camera`]: the camera's mode and controls, locked for measuring, and the sensor's pixel pitch.
//! - [`beam`]: where the line crosses the sensor, with its uncertainty, or why there is no reading.
//! - [`monitor`]: readings over time; heat drift, steps, and when the set-up is ready to measure.
//! - [`settle`]: the exponential settle fit behind the ready estimate.
//! - [`allan`]: Allan deviation, which says how long to average.
//! - [`reading`]: the frames of one sample averaged into a reading with an honest uncertainty.
//! - [`scale`]: pixels to µm, and the gauge-block calibration.
//! - [`survey`]: stations, runs, the zero station, drift correction, repeatability; the session.
//! - [`results`]: the reference removed, straightness or flatness, scrape and shim, CSV.
//! - [`units`]: µm, mm and inch for display.
//! - [`synth`]: synthetic frames, warm-up drift and surveys with known answers, for tests.

pub mod allan;
pub mod beam;
pub mod camera;
pub mod frame;
pub mod monitor;
pub mod reading;
pub mod results;
pub mod rng;
pub mod scale;
pub mod settle;
pub mod stats;
pub mod survey;
pub mod synth;
pub mod units;
pub mod uvc;
