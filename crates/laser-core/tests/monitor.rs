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
fn calm_and_steady_is_ready_after_a_minute() {
    let (_, s) = status_after(&flat(0.3), 30.0, 6);
    assert!(matches!(s.readiness, Readiness::Collecting { .. }), "{s:?}");
    let (_, s) = status_after(&flat(0.3), 90.0, 6);
    assert_eq!(s.readiness, Readiness::Ready, "{s:?}");
}

#[test]
fn a_restless_beam_is_noisy() {
    let (_, s) = status_after(&flat(5.0), 90.0, 7);
    assert!(matches!(s.readiness, Readiness::Noisy { .. }), "{s:?}");
}
