//! Beam finder against synthetic frames with known answers. Several tests are the known bugs of
//! the Python tool (numbers from PLAN.md).

use laser_core::beam::{FinderConfig, Fix, NoBeam, find, inspect};
use laser_core::frame::{LineAxis, Roi};
use laser_core::rng::Rng;
use laser_core::synth::LineSpec;

fn run(spec: &LineSpec) -> Result<Fix, NoBeam> {
    let cfg = FinderConfig { axis: spec.axis, ..FinderConfig::default() };
    find(&spec.render().view().unwrap(), &cfg)
}

fn short(spec: LineSpec) -> LineSpec {
    LineSpec { height: 240, ..spec }
}

/// A reading is right when it is within 4 of its own uncertainties of the truth. The synthetic
/// frames carry read and shot noise, so a fixed limit tighter than the noise would fail at random.
fn assert_reads(fix: &Fix, truth: f64, what: &str) {
    let err = fix.position_px - truth;
    assert!(fix.uncertainty_px > 0.0 && fix.uncertainty_px < 0.1, "{what}: {fix:?}");
    assert!(err.abs() <= 4.0 * fix.uncertainty_px, "{what}: error {err} {fix:?}");
}

/// Bug 1: the Python tool cut the centre to an integer. The position stays a float here.
#[test]
fn sub_pixel_sweep() {
    for k in 0..=10 {
        let centre = 960.0 + 0.1 * f64::from(k);
        let fix = run(&LineSpec { centre_px: centre, seed: 100 + k as u64, ..LineSpec::v1() }).unwrap();
        assert_reads(&fix, centre, &format!("centre {centre}"));
        assert!(fix.uncertainty_px < 0.02, "{fix:?}");
        assert!(!fix.warnings.any(), "{fix:?}");
    }
}

/// Averaged over many frames, the error at each sub-pixel position goes to zero: no pull toward
/// whole pixels and no offset.
#[test]
fn no_bias_over_many_frames() {
    let frames = 12;
    let mut all = 0.0;
    for k in 0..10 {
        let centre = 960.0 + 0.1 * f64::from(k);
        let (mut err, mut unc) = (0.0, 0.0);
        for f in 0..frames {
            let spec = short(LineSpec { centre_px: centre, seed: 1000 + 100 * k as u64 + f, ..LineSpec::v1() });
            let fix = run(&spec).unwrap();
            err += fix.position_px - centre;
            unc += fix.uncertainty_px;
        }
        let n = frames as f64;
        let (mean, se) = (err / n, unc / n / n.sqrt());
        assert!(mean.abs() <= 4.0 * se, "centre {centre}: mean error {mean}, standard error {se}");
        all += mean;
    }
    assert!((all / 10.0).abs() < 0.005, "mean error {}", all / 10.0);
}

/// Bug 18: the Python fit was right only near the middle of the sensor and gave wrong numbers
/// elsewhere. Here every position gives a right number, a near-edge warning, or no reading.
#[test]
fn linear_across_the_sensor() {
    for sigma in [20.0, 80.0, 200.0] {
        let reach = 2.5 * sigma + 1.0;
        for k in 0..20 {
            let centre = 96.0 + f64::from(k) * 1728.0 / 19.0;
            let spec = short(LineSpec { centre_px: centre, sigma_px: sigma, seed: k as u64, ..LineSpec::v1() });
            match run(&spec) {
                Ok(fix) => {
                    let what = format!("sigma {sigma} centre {centre}");
                    if fix.warnings.near_edge {
                        let err = fix.position_px - centre;
                        assert!(err.abs() < 0.5, "{what}: error {err} {fix:?}");
                    } else {
                        assert_reads(&fix, centre, &what);
                    }
                    let inside = centre > reach && centre < 1919.0 - reach;
                    assert_eq!(fix.warnings.near_edge, !inside, "sigma {sigma} centre {centre}");
                }
                Err(e) => {
                    assert_eq!(e, NoBeam::Edge, "sigma {sigma} centre {centre}");
                    assert!(centre < 2.0 * sigma || centre > 1919.0 - 2.0 * sigma, "sigma {sigma} centre {centre}");
                }
            }
        }
    }
}

/// The Python tool reported 992 px with no line.
#[test]
fn no_line_is_no_reading() {
    assert_eq!(run(&LineSpec::v1().without_line()), Err(NoBeam::Flat));
}

/// Bug 19: on the live frames with no laser, only an ambient gradient, the Python tool reported
/// 1718 px.
#[test]
fn ambient_gradient_is_no_reading() {
    let spec = LineSpec { gradient: 0.03, ..LineSpec::v1().without_line() };
    assert_eq!(run(&spec), Err(NoBeam::Edge));
}

#[test]
fn gradient_under_the_line_does_not_pull_it() {
    let fix = run(&LineSpec { centre_px: 830.4, gradient: 0.03, ..LineSpec::v1() }).unwrap();
    assert_reads(&fix, 830.4, "gradient");
}

#[test]
fn clipping_is_flagged() {
    let fix = run(&LineSpec { amplitude: 400.0, ..LineSpec::v1() }).unwrap();
    assert!(fix.warnings.clipped, "{fix:?}");
    assert_reads(&fix, 960.0, "clipped");
}

#[test]
fn tilt_is_measured() {
    let fix = run(&LineSpec { centre_px: 900.2, tilt: 0.01, ..LineSpec::v1() }).unwrap();
    assert!((fix.tilt - 0.01).abs() < 1e-3, "{fix:?}");
    assert_reads(&fix, 900.2, "tilted");
    assert!(!fix.warnings.tilted);
    let fix = run(&LineSpec { tilt: 0.03, ..LineSpec::v1() }).unwrap();
    assert!(fix.warnings.tilted, "{fix:?}");
}

#[test]
fn horizontal_line() {
    let spec = LineSpec { width: 480, axis: LineAxis::Horizontal, centre_px: 500.7, sigma_px: 60.0, ..LineSpec::v1() };
    assert_reads(&run(&spec).unwrap(), 500.7, "horizontal");
}

/// The reported uncertainty matches the frame-to-frame scatter within a factor of about 2.
#[test]
fn uncertainty_matches_the_scatter() {
    let spec = short(LineSpec { centre_px: 960.3, ..LineSpec::v1() });
    let cfg = FinderConfig::default();
    let mut rng = Rng::new(42);
    let fixes: Vec<Fix> = (0..30).map(|_| find(&spec.render_with(&mut rng).view().unwrap(), &cfg).unwrap()).collect();
    let n = fixes.len() as f64;
    let mean = fixes.iter().map(|f| f.position_px).sum::<f64>() / n;
    let sd = (fixes.iter().map(|f| (f.position_px - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
    let reported = fixes.iter().map(|f| f.uncertainty_px).sum::<f64>() / n;
    let ratio = reported / sd;
    assert!((0.4..2.5).contains(&ratio), "reported {reported}, scatter {sd}");
}

/// Bug 4: smoothing in the Python tool changed the scale (+5.5 % at the default, 27 px on this
/// move). There is no smoothing in the reading here, so a 500 px move reads as 500 px.
#[test]
fn a_move_reads_true() {
    let a = run(&LineSpec { centre_px: 700.0, ..LineSpec::v1() }).unwrap();
    let b = run(&LineSpec { centre_px: 1200.0, seed: 2, ..LineSpec::v1() }).unwrap();
    let unc = a.uncertainty_px.hypot(b.uncertainty_px);
    assert!((b.position_px - a.position_px - 500.0).abs() <= 4.0 * unc, "{a:?} {b:?}");
}

/// A speck of dust lights up a patch beside the line in one strip. That strip is dropped and
/// flagged, and the reading comes from the rest.
#[test]
fn a_speck_drops_one_strip() {
    let spec = LineSpec { centre_px: 960.4, ..LineSpec::v1() };
    let mut img = spec.render();
    for y in 0..60 {
        for x in 1050..1070 {
            let p = &mut img.data[y * img.width + x];
            *p = p.saturating_add(60);
        }
    }
    let fix = find(&img.view().unwrap(), &FinderConfig::default()).unwrap();
    assert!(fix.warnings.strips_dropped && fix.strips_used == 15, "{fix:?}");
    assert_reads(&fix, 960.4, "speck");
}

#[test]
fn region_of_interest_keeps_frame_pixels() {
    let spec = LineSpec { centre_px: 700.0, sigma_px: 20.0, ..LineSpec::v1() };
    let img = spec.render();
    let frame = img.view().unwrap();
    let whole = find(&frame, &FinderConfig::default()).unwrap();
    let roi = Roi { x: 400, y: 100, width: 700, height: 880 };
    let cropped = find(&frame, &FinderConfig { roi: Some(roi), ..FinderConfig::default() }).unwrap();
    assert_reads(&cropped, 700.0, "inside the region");
    assert!((cropped.position_px - whole.position_px).abs() < 3.0 * whole.uncertainty_px.hypot(cropped.uncertainty_px));
    // A region away from the line has no beam in it.
    let away = Roi { x: 1200, y: 0, width: 600, height: 1080 };
    assert!(find(&frame, &FinderConfig { roi: Some(away), ..FinderConfig::default() }).is_err());
    // A region off the frame is ignored.
    let off = Roi { x: 5000, y: 0, width: 10, height: 10 };
    assert_eq!(find(&frame, &FinderConfig { roi: Some(off), ..FinderConfig::default() }).unwrap(), whole);
}

#[test]
fn inspection_shows_what_the_finder_used() {
    let spec = LineSpec { centre_px: 700.0, sigma_px: 20.0, ..LineSpec::v1() };
    let img = spec.render();
    let frame = img.view().unwrap();
    for roi in [None, Some(Roi { x: 400, y: 100, width: 700, height: 880 })] {
        let cfg = FinderConfig { roi, ..FinderConfig::default() };
        let fix = find(&frame, &cfg).unwrap();
        let seen = inspect(&frame, &cfg);
        assert_eq!(seen.offset_px, roi.map_or(0, |r| r.x));
        assert_eq!(seen.profile.len(), roi.map_or(frame.width(), |r| r.width));
        let z = seen.zones.unwrap();
        // The summed profile's centre is the line's centre (it has no tilt), within a px.
        assert!((z.centre + seen.offset_px as f64 - fix.position_px).abs() < 1.0, "{z:?} {fix:?}");
        assert!((z.fwhm - fix.fwhm_px).abs() < 1e-9 && z.centroid_fits);
        assert!(
            z.background_left as f64 <= z.centre - z.half_width && z.background_right as f64 >= z.centre + z.half_width
        );
        // The background line sits under the profile away from the beam.
        let at = |i: usize| z.background.0 + z.background.1 * i as f64;
        assert!((seen.profile[5] - at(5)).abs() < 3.0, "{} vs {}", seen.profile[5], at(5));
    }
    let away = FinderConfig { roi: Some(Roi { x: 1200, y: 0, width: 600, height: 1080 }), ..FinderConfig::default() };
    assert!(inspect(&frame, &away).zones.is_err());
}
