//! The measurement model against simulated surveys of surfaces with known shape.

use laser_core::reading::{FrameResult, Subsample};
use laser_core::results::{Reference, Results};
use laser_core::rng::Rng;
use laser_core::scale::Scale;
use laser_core::stats;
use laser_core::survey::{DriftCorrection, Layout, LoadError, Run, Session, SurveyError};
use laser_core::synth::{SurveySim, WarmUp};
use laser_core::units::Unit;

const INTERP: DriftCorrection = DriftCorrection::Interpolate;

/// 11 stations over a metre; a 20 µm bow.
fn bow(sim: &mut SurveySim, run: &Run) {
    sim.surface_um = run.stations.iter().map(|s| 20.0 * (std::f64::consts::PI * s.x_mm / 1000.0).sin()).collect();
}

/// The zero, every station in order, the zero again.
fn closed(n: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    order.push(0);
    order
}

/// Flattened truth: the true heights with their own least-squares line removed.
fn true_flattened(sim: &SurveySim, run: &Run) -> Vec<f64> {
    let x: Vec<f64> = run.stations.iter().map(|s| s.x_mm).collect();
    let h: Vec<f64> = (0..run.stations.len()).map(|i| sim.true_height_um(run, i)).collect();
    let (a, b) = stats::line_fit(&x, &h).unwrap();
    x.iter().zip(&h).map(|(x, h)| h - a - b * x).collect()
}

fn max_height_error(sim: &SurveySim, run: &Run, mode: DriftCorrection) -> f64 {
    let h = run.heights(&sim.scale, mode).unwrap();
    h.stations.iter().map(|s| (s.height_um - sim.true_height_um(run, s.station)).abs()).fold(0.0, f64::max)
}

#[test]
fn straight_surface_under_a_tilted_reference() {
    let mut run = Run::line("tilted", 11, 100.0);
    let mut sim = SurveySim::flat(11);
    sim.reference = [5.0, 0.05, 0.0];
    sim.scale.invert = true;
    sim.survey(&mut run, &closed(11), 0.0, &mut Rng::new(1)).unwrap();
    let r = Results::compute(&run, &sim.scale, INTERP).unwrap();
    let Reference::Line { slope_um_per_m, .. } = r.reference else { panic!("{:?}", r.reference) };
    assert!((slope_um_per_m + 50.0).abs() < 0.5, "{:?}", r.reference);
    assert!(r.pv_noise_um > 0.1 && r.pv_noise_um < 1.0, "{r:?}");
    assert!(r.pv_um < 3.0 * r.pv_noise_um, "pv {} noise {}", r.pv_um, r.pv_noise_um);
    assert!(r.drift.closed && r.drift.closure_um.unwrap().abs() < 0.6, "{:?}", r.drift);
    assert!(r.points.iter().all(|p| !p.flags.any()), "{:?}", r.points);
}

#[test]
fn bow_reads_true_with_scrape_and_shim() {
    let mut run = Run::line("bow", 11, 100.0);
    let mut sim = SurveySim::flat(11);
    bow(&mut sim, &run);
    sim.reference = [0.0, 0.03, 0.0];
    sim.survey(&mut run, &closed(11), 0.0, &mut Rng::new(2)).unwrap();
    let r = Results::compute(&run, &sim.scale, INTERP).unwrap();
    let truth = true_flattened(&sim, &run);
    for (p, t) in r.points.iter().zip(&truth) {
        assert!((p.flattened_um - t).abs() < 0.6, "station {}: {} vs {t}", p.station, p.flattened_um);
    }
    let pv_true = truth.iter().copied().fold(f64::MIN, f64::max) - truth.iter().copied().fold(f64::MAX, f64::min);
    assert!((r.pv_um - pv_true).abs() < 0.6, "pv {} vs {pv_true}", r.pv_um);
    let top = r.points.iter().max_by(|a, b| a.flattened_um.total_cmp(&b.flattened_um)).unwrap();
    assert_eq!(top.station, 5);
    assert!(top.below_max_um == 0.0 && (top.above_min_um - r.pv_um).abs() < 1e-12, "{top:?}");
}

/// A steady drift of 0.02 µm/s: the closure shows it and interpolation between the zero visits
/// removes it. Against the first zero only, the last stations are off by over 2 µm.
#[test]
fn steady_drift_is_closed_and_removed() {
    let mut run = Run::line("drift", 11, 100.0);
    let mut sim = SurveySim::flat(11);
    bow(&mut sim, &run);
    sim.drift_px_per_s = 0.01;
    let end = sim.survey(&mut run, &closed(11), 0.0, &mut Rng::new(3)).unwrap();

    let h = run.heights(&sim.scale, INTERP).unwrap();
    let span = h.drift.zeros.last().unwrap().t_s - h.drift.zeros[0].t_s;
    assert!((h.drift.closure_um.unwrap() - 0.02 * span).abs() < 0.4, "{:?}", h.drift);
    assert!((h.drift.rate_um_per_min.unwrap() - 1.2).abs() < 0.2, "{:?}", h.drift);
    assert!(h.drift.closed);
    let interp = max_height_error(&sim, &run, INTERP);
    let last = max_height_error(&sim, &run, DriftCorrection::LastZero);
    assert!(interp < 0.6, "interpolated: {interp}");
    assert!(last > 1.5, "last zero: {last}");

    let advice = run.rezero(&sim.scale, end + 100.0, None, 1.0).unwrap();
    assert!(advice.from_run && advice.due, "{advice:?}");
    assert!((advice.rate_um_per_min - 1.2).abs() < 0.2, "{advice:?}");
}

/// A warm-up drift (exponential) is not a straight line, but zero visits every three stations
/// follow it closely. With one zero at the start, every height carries the drift and is flagged.
#[test]
fn warm_up_drift_with_re_zeros() {
    let mut sim = SurveySim::flat(10);
    sim.warm_up = WarmUp { start: 50.0, settled: 0.0, tau_s: 200.0, steps: vec![], jitter: 0.3 };
    let mut run = Run::line("rezero", 10, 100.0);
    bow(&mut sim, &run);
    let order = [0, 1, 2, 3, 0, 4, 5, 6, 0, 7, 8, 9, 0];
    sim.survey(&mut run, &order, 300.0, &mut Rng::new(4)).unwrap();
    let err = max_height_error(&sim, &run, INTERP);
    assert!(err < 0.8, "re-zeroed: {err}");

    let mut once = Run::line("once", 10, 100.0);
    sim.survey(&mut once, &(0..10).collect::<Vec<_>>(), 300.0, &mut Rng::new(4)).unwrap();
    assert!(max_height_error(&sim, &once, INTERP) > 5.0);
    let h = once.heights(&sim.scale, INTERP).unwrap();
    assert!(!h.drift.closed && h.visits.iter().all(|v| v.flags.drift_unchecked), "{:?}", h.drift);
    assert_eq!(h.drift.closure_um, None);
    // One zero visit: the advice needs the monitor's rate.
    assert_eq!(once.rezero(&sim.scale, 500.0, None, 1.0), None);
    let advice = once.rezero(&sim.scale, 500.0, Some(-0.01), 1.0).unwrap();
    assert!(!advice.from_run && (advice.rate_um_per_min - 0.6).abs() < 1e-9, "{advice:?}");
}

/// Forward and back: every station twice. With honest uncertainties the ratio is near 1. Where the
/// block sits adds 1.5 µm that no single reading sees; the repeats show it, and the noise floor of
/// the peak to valley rises with it.
#[test]
fn forward_and_back_shows_placement() {
    let order: Vec<usize> = (0..11).chain((1..11).rev()).chain([0]).collect();
    let survey = |placement_um: f64, seed: u64| {
        let mut run = Run::line("fb", 11, 100.0);
        let sim = SurveySim { placement_um, ..SurveySim::flat(11) };
        sim.survey(&mut run, &order, 0.0, &mut Rng::new(seed)).unwrap();
        Results::compute(&run, &sim.scale, INTERP).unwrap()
    };
    let calm = survey(0.0, 5);
    let rep = calm.repeatability.unwrap();
    assert_eq!(rep.dof, 10);
    assert!((0.2..2.5).contains(&rep.ratio), "{rep:?}");

    let placed = survey(1.5, 6);
    let rep = placed.repeatability.unwrap();
    assert!(rep.ratio > 10.0 && (0.8..2.4).contains(&rep.pooled_sd_um), "{rep:?}");
    assert!(placed.pv_noise_um > 3.0 * calm.pv_noise_um, "{} vs {}", placed.pv_noise_um, calm.pv_noise_um);
}

#[test]
fn grid_plane_with_a_bump() {
    let mut run = Run::grid("plate", 5, 5, 100.0, 100.0);
    let mut sim = SurveySim::flat(25);
    sim.surface_um[12] = 8.0;
    sim.reference = [0.0, 0.04, -0.02];
    sim.survey(&mut run, &closed(25), 0.0, &mut Rng::new(7)).unwrap();
    let r = Results::compute(&run, &sim.scale, INTERP).unwrap();
    let Reference::Plane { slope_x_um_per_m, slope_y_um_per_m, .. } = r.reference else { panic!() };
    assert!((slope_x_um_per_m + 40.0).abs() < 1.0 && (slope_y_um_per_m - 20.0).abs() < 1.0, "{:?}", r.reference);
    // An 8 µm bump at the centre of 25 points: the plane rises 8/25, so PV = 8.
    assert!((r.pv_um - 8.0).abs() < 0.6, "{}", r.pv_um);
    let top = r.points.iter().max_by(|a, b| a.flattened_um.total_cmp(&b.flattened_um)).unwrap();
    assert_eq!(top.station, 12);
    let csv = r.csv(Unit::Um);
    assert!(csv.lines().next().unwrap().contains("X (mm),Y (mm)"), "{csv}");
}

/// Bugs 7 and 8: delete and replace with no valid selection change nothing; results always come
/// from the data, so a delete is in the next result.
#[test]
fn edits_are_checked() {
    let mut run = Run::line("edit", 5, 100.0);
    let mut sim = SurveySim::flat(5);
    sim.surface_um = vec![0.0, 1.0, 5.0, 1.0, 0.0];
    assert_eq!(run.next_station(), 0);
    assert_eq!(run.heights(&sim.scale, INTERP), Err(SurveyError::NoZero));
    sim.survey(&mut run, &[0, 1, 2], 0.0, &mut Rng::new(8)).unwrap();
    assert_eq!(run.next_station(), 3);
    sim.survey(&mut run, &[3, 4], 100.0, &mut Rng::new(9)).unwrap();
    assert_eq!(run.next_station(), 0);

    let before = run.clone();
    assert_eq!(run.delete(99), Err(SurveyError::NoVisit(99)));
    let no_beam = vec![Subsample { t_s: 0.0, frame: FrameResult::NoBeam { reason: laser_core::beam::NoBeam::Flat } }];
    assert_eq!(run.replace(99, no_beam.clone()), Err(SurveyError::NoVisit(99)));
    assert_eq!(run.replace(1, no_beam.clone()), Err(SurveyError::NoReading));
    assert_eq!(run.record(9, no_beam), Err(SurveyError::NoStation(9)));
    assert_eq!(run.remove_station(0), Err(SurveyError::ZeroStation));
    assert_eq!(run, before);

    let pv = Results::compute(&run, &sim.scale, INTERP).unwrap().pv_um;
    run.delete(2).unwrap();
    let r = Results::compute(&run, &sim.scale, INTERP).unwrap();
    assert!(r.points.iter().all(|p| p.station != 2) && r.pv_um < pv - 2.0, "{} vs {pv}", r.pv_um);

    let gone = run.remove_station(1).unwrap();
    assert_eq!(gone.len(), 1);
    assert_eq!(run.stations.len(), 4);
    assert!(run.visits.iter().all(|v| v.station < 4));
    let r = Results::compute(&run, &sim.scale, INTERP).unwrap();
    let labels: Vec<&str> = r.points.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["1", "4", "5"]);
    run.remove_station(2).unwrap();
    assert_eq!(
        Results::compute(&run, &sim.scale, INTERP).unwrap_err(),
        SurveyError::TooFewStations { have: 2, need: 3 }
    );
}

#[test]
fn csv_keeps_the_python_columns_first() {
    let mut run = Run::line("csv", 4, 250.0);
    let sim = SurveySim::flat(4);
    sim.survey(&mut run, &[0, 1, 2, 3], 0.0, &mut Rng::new(10)).unwrap();
    let csv = Results::compute(&run, &sim.scale, INTERP).unwrap().csv(Unit::Mm);
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(
        lines[0],
        "Measured (mm),Flattened (mm),Below Max (mm),Above Min (mm),Uncertainty (mm),Station,Position (mm),Visits,Time (s),Flags"
    );
    assert_eq!(lines.len(), 5);
    let zero: Vec<&str> = lines[1].split(',').collect();
    assert_eq!((zero[0], zero[5], zero[6]), ("0.00000", "1", "0.000"));
    // One zero visit: the other stations are flagged.
    assert!(lines[2].ends_with(",drift_unchecked"), "{}", lines[2]);
}

#[test]
fn session_round_trip_and_checks() {
    let mut session = Session::new(Scale::from_pixel_pitch(1.4));
    let mut run = Run::line("a", 4, 100.0);
    SurveySim::flat(4).survey(&mut run, &closed(4), 0.0, &mut Rng::new(11)).unwrap();
    session.runs.push(run);
    session.runs.push(Run::grid("b", 2, 2, 50.0, 50.0));
    let back = Session::from_json(&session.to_json()).unwrap();
    assert_eq!(back, session);
    assert_eq!(back.runs[1].layout, Layout::Grid);

    let mut bad = session.clone();
    bad.runs[0].visits[1].station = 99;
    assert_eq!(Session::from_json(&bad.to_json()), Err(LoadError::Invalid(SurveyError::NoStation(99))));
    // A newer file is refused by its version, even with fields this version does not know.
    let newer = r#"{"format":2,"scale":{"um_per_px":2.0},"lens":{"focal_mm":4}}"#;
    assert_eq!(Session::from_json(newer), Err(LoadError::Invalid(SurveyError::Format(2))));
    assert!(matches!(Session::from_json("{\"scale\":1}"), Err(LoadError::Json(_))));
    let minimal = Session::from_json(r#"{"format":1,"scale":{"um_per_px":2.0}}"#).unwrap();
    assert_eq!(minimal.drift_correction, INTERP);
    let zero_scale = r#"{"format":1,"scale":{"um_per_px":0.0}}"#;
    assert_eq!(Session::from_json(zero_scale), Err(LoadError::Invalid(SurveyError::BadScale)));
}
