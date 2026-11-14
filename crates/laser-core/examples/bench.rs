//! The beam finder on the frames the Python baseline used (`bench/python/synthetic.py` on the
//! `python` branch, spec v1), with the same tests: time per frame, a line at the middle, near the
//! edge and missing, sub-pixel steps, positions across the sensor, spread at one position, and the
//! scale. Also the frames saved from the live camera, if `bench/data/live` has them.
//!
//! ```text
//! cargo run --release -p laser-core --example bench [-- --dump frame0.y8]
//! ```
//!
//! `--dump` writes frame 0 so its SHA-256 can be checked against the Python run's.

use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

use laser_core::beam::{self, FinderConfig, Fix, NoBeam};
use laser_core::frame::Luma;

const W: usize = 1920;
const H: usize = 1080;
/// Sensor width 5.9 mm over 1920 px, as in the Python baseline.
const UM_PER_PX: f64 = 5900.0 / 1920.0;

#[derive(Clone, Copy)]
struct Scene {
    centre: f64,
    sigma: f64,
    amplitude: f64,
}

const V1: Scene = Scene { centre: 960.0, sigma: 80.0, amplitude: 160.0 };

fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Frame `n` of spec v1: background 16, read noise 2, shot gain 0.5, seed 1.
fn render(s: Scene, n: u64) -> Vec<u8> {
    let (background, read_noise, shot_gain, seed) = (16.0, 2.0, 0.5, 1u64);
    let cols: Vec<(f64, f64)> = (0..W)
        .map(|x| {
            let d = (x as f64 - s.centre) / s.sigma;
            let signal = background + s.amplitude * (-0.5 * (d * d)).exp();
            (signal, (read_noise * read_noise + shot_gain * signal).sqrt())
        })
        .collect();
    let key = splitmix64((seed << 32) ^ n);
    let root3 = 3.0f64.sqrt();
    let mut out = vec![0u8; W * H];
    for (p, px) in out.iter_mut().enumerate() {
        let h = splitmix64(key.wrapping_add(p as u64));
        let u = |j: u32| (((h >> (16 * j)) & 0xFFFF) as f64 + 0.5) / 65536.0;
        let z = ((((u(0) + u(1)) + u(2)) + u(3)) - 2.0) * root3;
        let (signal, sd) = cols[p % W];
        *px = (signal + sd * z + 0.5).floor().clamp(0.0, 255.0) as u8;
    }
    out
}

fn find(data: &[u8]) -> Result<Fix, NoBeam> {
    beam::find(&Luma::new(W, H, W, data).unwrap(), &FinderConfig::default())
}

struct Stats {
    mean: f64,
    sd: f64,
    p99: f64,
    max: f64,
}

fn stats(v: &[f64]) -> Stats {
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    let sd = (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0)).sqrt();
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let p99 = s[((s.len() - 1) as f64 * 0.99).round() as usize];
    Stats { mean, sd, p99, max: s[s.len() - 1] }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn describe(r: &Result<Fix, NoBeam>, truth: f64) -> String {
    match r {
        Ok(f) => {
            let w = &f.warnings;
            let flags: Vec<&str> = [
                (w.near_edge, "near edge"),
                (w.clipped, "clipped"),
                (w.tilted, "tilted"),
                (w.strips_dropped, "strips dropped"),
            ]
            .into_iter()
            .filter_map(|(on, s)| on.then_some(s))
            .collect();
            format!(
                "{:.3} px (error {:+.3}, ±{:.3}){}",
                f.position_px,
                f.position_px - truth,
                f.uncertainty_px,
                if flags.is_empty() { String::new() } else { format!(", warns: {}", flags.join(", ")) }
            )
        }
        Err(e) => format!("no reading: {}", e.text()),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--dump") {
        let path = args.get(i + 1).expect("--dump needs a path");
        std::fs::write(path, render(V1, 0)).unwrap();
        println!("wrote frame 0 to {path}");
    }

    println!("== Time per frame (spec v1, 1920x1080, line at 960 px) ==");
    let frames: Vec<Vec<u8>> = (0..16).map(|n| render(V1, n)).collect();
    for f in &frames[..4] {
        black_box(find(f).ok());
    }
    let mut find_ms = Vec::new();
    for k in 0..600 {
        let t = Instant::now();
        black_box(find(black_box(&frames[k % frames.len()])).ok());
        find_ms.push(ms(t));
    }
    let s = stats(&find_ms);
    println!("beam::find            mean {:.3} ms  sd {:.3}  p99 {:.3}  max {:.3}", s.mean, s.sd, s.p99, s.max);
    println!("  highest rate on one thread: {:.0} fps", 1e3 / s.mean);
    let mut detail_ms = Vec::new();
    for k in 0..200 {
        let data = &frames[k % frames.len()];
        let t = Instant::now();
        let luma = Luma::new(W, H, W, data).unwrap();
        black_box(beam::inspect(&luma, &FinderConfig::default()));
        let mut histogram = [0u32; 256];
        for &p in data {
            histogram[usize::from(p)] += 1;
        }
        black_box(histogram);
        detail_ms.push(ms(t));
    }
    let d = stats(&detail_ms);
    println!("inspect + histogram   mean {:.3} ms  p99 {:.3}  (the app does this on 1 frame in 15)", d.mean, d.p99);

    println!("\n== A line at the middle, near the edge, and none ==");
    for (name, scene, truth) in [
        ("middle (960 px)", V1, 960.0),
        ("edge (1728 px)", Scene { centre: 1728.0, ..V1 }, 1728.0),
        ("no line", Scene { amplitude: 0.0, ..V1 }, f64::NAN),
    ] {
        let fs: Vec<Vec<u8>> = (0..10).map(|n| render(scene, n)).collect();
        let mut t_ms = Vec::new();
        let mut last = None;
        for k in 0..100 {
            let t = Instant::now();
            let r = find(&fs[k % fs.len()]);
            t_ms.push(ms(t));
            last = Some(r);
        }
        let s = stats(&t_ms);
        println!("{name:<16} {:.3} ms (p99 {:.3})  {}", s.mean, s.p99, describe(&last.unwrap(), truth));
    }

    println!("\n== Sub-pixel steps, 960.0 to 961.0 px, 10 frames each ==");
    let mut all_err = Vec::new();
    for i in 0..=10 {
        let truth = 960.0 + i as f64 / 10.0;
        let errs: Vec<f64> =
            (0..10).map(|n| find(&render(Scene { centre: truth, ..V1 }, n)).unwrap().position_px - truth).collect();
        let s = stats(&errs);
        println!("{truth:.1} px  error mean {:+.4}  sd {:.4} px", s.mean, s.sd);
        all_err.extend(errs);
    }
    let worst = all_err.iter().fold(0.0f64, |a, e| a.max(e.abs()));
    println!("worst {worst:.4} px = {:.3} µm", worst * UM_PER_PX);

    println!("\n== Positions across the sensor, 5 % to 95 %, 5 frames each ==");
    for sigma in [20.0, 80.0, 200.0] {
        let (mut worst, mut good, mut warned, mut none) = (0.0f64, Vec::new(), 0, Vec::new());
        for i in 0..20 {
            let truth = 96.0 + i as f64 * (1728.0 / 19.0);
            let scene = Scene { centre: truth, sigma, ..V1 };
            let rs: Vec<_> = (0..5).map(|n| find(&render(scene, n))).collect();
            if let Some(Err(e)) = rs.iter().find(|r| r.is_err()) {
                none.push(format!("{truth:.0} ({e:?})"));
                continue;
            }
            let err = rs.iter().map(|r| (r.as_ref().unwrap().position_px - truth).abs()).fold(0.0, f64::max);
            if rs.iter().any(|r| r.as_ref().unwrap().warnings.any()) {
                warned += 1;
            }
            worst = worst.max(err);
            if err < 0.1 {
                good.push(truth);
            }
        }
        println!(
            "sigma {sigma:>3} px: within 0.1 px at {} of 20, worst {worst:.3} px, {warned} with a warning, no reading at {}",
            good.len(),
            if none.is_empty() { "none".to_string() } else { none.join(", ") }
        );
    }

    println!("\n== Spread at 960.37 px, 300 frames ==");
    let truth = 960.37;
    let ps: Vec<f64> = (0..300).map(|n| find(&render(Scene { centre: truth, ..V1 }, n)).unwrap().position_px).collect();
    let s = stats(&ps);
    println!("one frame: error {:+.4} px, sd {:.4} px = {:.4} µm", s.mean - truth, s.sd, s.sd * UM_PER_PX);
    let means: Vec<f64> = ps.chunks(10).map(|c| c.iter().sum::<f64>() / c.len() as f64).collect();
    let m = stats(&means);
    println!("means of 10: error {:+.4} px, sd {:.4} px = {:.4} µm", m.mean - truth, m.sd, m.sd * UM_PER_PX);

    println!("\n== Scale: zero at 700 px, sample at 1200 px ==");
    let at = |c: f64| find(&render(Scene { centre: c, ..V1 }, 0)).unwrap().position_px;
    let reported = (at(1200.0) - at(700.0)) * UM_PER_PX;
    let true_um = 500.0 * UM_PER_PX;
    println!(
        "reported {reported:.1} µm, true {true_um:.1} µm, error {:+.2} µm ({:+.3} %)",
        reported - true_um,
        100.0 * (reported - true_um) / true_um
    );

    let live = Path::new("bench/data/live");
    let real: Vec<Vec<u8>> = (0..100)
        .map(|i| live.join(format!("frame_{i:04}.y8")))
        .take_while(|p| p.exists())
        .filter_map(|p| std::fs::read(p).ok())
        .filter(|d| d.len() == W * H)
        .collect();
    if !real.is_empty() {
        println!("\n== {} frames saved from the live camera ==", real.len());
        let rs: Vec<_> = real.iter().map(|d| find(d)).collect();
        let ok: Vec<f64> = rs.iter().filter_map(|r| r.as_ref().ok().map(|f| f.position_px)).collect();
        if ok.len() >= 2 {
            let s = stats(&ok);
            println!(
                "{} readings: mean {:.3} px, frame-to-frame sd {:.4} px = {:.3} µm",
                ok.len(),
                s.mean,
                s.sd,
                s.sd * UM_PER_PX
            );
        }
        if let Some(Err(e)) = rs.iter().find(|r| r.is_err()) {
            println!("{} frames gave no reading, e.g. {}", rs.len() - ok.len(), e.text());
        }
        println!("first frame: {}", describe(&rs[0], f64::NAN));
        let n = real.len();
        let mut t_ms = Vec::new();
        for k in 0..300 {
            let t = Instant::now();
            black_box(find(black_box(&real[k % n])).ok());
            t_ms.push(ms(t));
        }
        let s = stats(&t_ms);
        println!("beam::find on real frames: mean {:.3} ms, p99 {:.3}", s.mean, s.p99);
    }
}
