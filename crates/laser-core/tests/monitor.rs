//! Heat drift monitor against simulated warm-up logs with known answers.

use laser_core::monitor::{Monitor, MonitorConfig, Readiness, RunPlan, Status};
use laser_core::rng::Rng;
use laser_core::synth::WarmUp;

const FPS: f64 = 20.0;

/// Feed readings at 20 per second from `from_s` up to `to_s`.
fn feed(m: &mut Monitor, w: &WarmUp, rng: &mut Rng, from_s: f64, to_s: f64) {
    let mut k = (from_s * FPS).round() as u64;
    loop {
        let t = k as f64 / FPS;
        if t >= to_s {
            break;
        }
        m.push(t, w.sample(t, rng));
        k += 1;
    }
}

fn plan() -> RunPlan {
    RunPlan { run_s: 600.0, tolerance: 0.5 }
}

fn status_after(w: &WarmUp, seconds: f64, seed: u64) -> (Monitor, Status) {
    let mut m = Monitor::new(MonitorConfig::default());
    feed(&mut m, w, &mut Rng::new(seed), 0.0, seconds);
    let s = m.status(&plan());
    (m, s)
}

fn flat(jitter: f64) -> WarmUp {
    WarmUp { start: 0.0, settled: 0.0, tau_s: 1.0, steps: vec![], jitter }
}

#[test]
fn warm_up_gives_a_ready_time_then_ready() {
    let w = WarmUp { start: 20.0, settled: 0.0, tau_s: 300.0, steps: vec![], jitter: 0.3 };
    let truth = w.true_ready_s(600.0, 0.5);
    let (_, s) = status_after(&w, 600.0, 1);
    let Readiness::Settling { in_s, earliest_in_s, latest_in_s } = s.readiness else {
        panic!("expected settling: {s:?}");
    };
    let left = truth - s.t_s;
    assert!(earliest_in_s <= left + 60.0 && latest_in_s.unwrap() >= left - 60.0, "{s:?}, truth in {left}");
    assert!((in_s - left).abs() < 120.0, "{s:?}, truth in {left}");
    let fit = s.settle.unwrap();
    assert!(fit.tau_low_s <= 300.0 && fit.tau_high_s.unwrap() >= 300.0, "{fit:?}");

    let (_, s) = status_after(&w, 1300.0, 2);
    assert_eq!(s.readiness, Readiness::Ready, "{s:?}");
}

/// A slow warm-up watched for an hour and more: the settle fit takes the bins in runs of 4 to 7
/// and still finds the time constant and the ready time.
#[test]
fn a_long_wait_fits_on_averaged_bins() {
    let w = WarmUp { start: 30.0, settled: 0.0, tau_s: 1800.0, steps: vec![], jitter: 0.3 };
    let truth = w.true_ready_s(600.0, 0.5);
    let (_, s) = status_after(&w, 3600.0, 8);
    let fit = s.settle.clone().unwrap();
    assert!(fit.points <= 900, "{fit:?}");
    assert!(fit.tau_low_s <= 1800.0 && fit.tau_high_s.unwrap() >= 1800.0, "{fit:?}");
    let Readiness::Settling { in_s, .. } = s.readiness else {
        panic!("expected settling: {s:?}");
    };
    let left = truth - s.t_s;
    assert!((in_s - left).abs() < 300.0, "{s:?}, truth in {left}");

    let (_, s) = status_after(&w, 6000.0, 9);
    assert_eq!(s.readiness, Readiness::Ready, "{s:?}");
}

#[test]
fn a_bump_is_a_step_and_restarts_the_estimate() {
    let w = WarmUp { steps: vec![(200.0, 3.0)], ..flat(0.3) };
    let (m, s) = status_after(&w, 260.0, 3);
    assert_eq!(m.steps().len(), 1, "{:?}", m.steps());
    let step = m.steps()[0];
    assert!((step.t_s - 200.0).abs() <= 2.0 && (step.size - 3.0).abs() < 0.3, "{step:?}");
    assert!((s.since_s - 200.0).abs() <= 2.0);
    assert!(matches!(s.readiness, Readiness::Collecting { .. }), "{s:?}");
}

#[test]
fn a_fast_warm_up_is_not_a_step() {
    let w = WarmUp { start: 50.0, settled: 0.0, tau_s: 60.0, steps: vec![], jitter: 0.3 };
    let (m, _) = status_after(&w, 600.0, 4);
    assert!(m.steps().is_empty(), "{:?}", m.steps());
}

#[test]
fn steady_drift_rate() {
    let mut m = Monitor::new(MonitorConfig::default());
    let mut rng = Rng::new(5);
    for k in 0..(300.0 * FPS) as u64 {
        let t = k as f64 / FPS;
        m.push(t, 2.0 * t / 60.0 + 0.3 * rng.normal());
    }
    let s = m.status(&plan());
    let rate = s.rate_per_min.unwrap();
    assert!((rate - 2.0).abs() < 0.1, "{s:?}");
    assert!(matches!(s.readiness, Readiness::Drifting { .. } | Readiness::Settling { .. }), "{s:?}");
}

#[test]
fn calm_and_steady_is_ready_in_a_few_minutes() {
    let (_, s) = status_after(&flat(0.3), 30.0, 6);
    assert!(matches!(s.readiness, Readiness::Collecting { .. }), "{s:?}");
    let (_, s) = status_after(&flat(0.3), 150.0, 6);
    assert_eq!(s.readiness, Readiness::Ready, "{s:?}");
}

/// Statuses for a steady drift of `drift_per_run` over a 20 min run, with `bin_noise` on each
/// one-second bin: one status every `every_s` from the first minute to 30 min.
fn steady_drift_statuses(drift_per_run: f64, bin_noise: f64, seed: u64, every_s: f64) -> Vec<Readiness> {
    let plan = RunPlan { run_s: 1200.0, tolerance: 0.5 };
    let rate = drift_per_run / plan.run_s;
    let mut m = Monitor::new(MonitorConfig::default());
    let mut rng = Rng::new(seed);
    let mut next = 60.0;
    let mut out = Vec::new();
    for k in 0..(1800.0 * FPS) as u64 {
        let t = k as f64 / FPS;
        m.push(t, rate * t + bin_noise * FPS.sqrt() * rng.normal());
        if t >= next {
            out.push(m.status(&plan).readiness);
            next += every_s;
        }
    }
    out
}

#[test]
fn a_steady_drift_over_the_allowed_is_never_ready() {
    for (drift, noise) in [(2.4, 0.4), (2.4, 0.2), (1.2, 0.4)] {
        for seed in 1..=2 {
            let seen = steady_drift_statuses(drift, noise, seed, 30.0);
            assert!(!seen.contains(&Readiness::Ready), "{drift} per run, noise {noise}, seed {seed}: {seen:?}");
            assert!(matches!(seen.last(), Some(Readiness::Drifting { .. })), "{seen:?}");
        }
    }
}

#[test]
fn a_small_steady_drift_ends_ready() {
    let seen = steady_drift_statuses(0.1, 0.2, 9, 60.0);
    assert_eq!(seen.last(), Some(&Readiness::Ready), "{seen:?}");
}

#[test]
fn a_restless_beam_is_noisy() {
    let (_, s) = status_after(&flat(5.0), 90.0, 7);
    assert!(matches!(s.readiness, Readiness::Noisy { .. }), "{s:?}");
}
