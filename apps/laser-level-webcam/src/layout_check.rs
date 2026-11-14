//! The window drawn headless at the sizes it is used at, from the smallest it allows to 4K, with a
//! live camera and a measured run. Each screen is checked for text drawn over other text, text cut
//! off by the edge of its panel or canvas, and words that move while only the numbers change.

use eframe::egui::{self, Rect, Shape, epaint::ClippedShape, pos2, vec2};
use laser_camera::FrameSource;
use laser_camera::synthetic::Synthetic;
use laser_core::reading::SampleRule;

use crate::app::{App, Screen};
use crate::engine::{Engine, lock, process};
use crate::settings::AppSettings;
use crate::theme;

/// Window sizes in points: the smallest the window allows, laptops, 1080p, 1440p and 4K at 100 %.
const SIZES: [[f32; 2]; 6] =
    [[960.0, 600.0], [1100.0, 700.0], [1280.0, 800.0], [1920.0, 1080.0], [2560.0, 1440.0], [3840.0, 2160.0]];

/// Plot labels that sit on the data and so move with it.
const FOLLOWS_DATA: [&str; 2] = ["ready", "where it settles"];

#[derive(Clone, Debug)]
struct Text {
    s: String,
    /// Each line's ink, without the space a wrapped label leaves before its first line.
    rows: Vec<Rect>,
    rect: Rect,
    clip: Rect,
    /// Where a one-line text sits, and its font size, for lining it up with its neighbours.
    baseline: Option<f32>,
    size: f32,
}

fn texts(shapes: &[ClippedShape]) -> Vec<Text> {
    fn walk(shape: &Shape, clip: Rect, out: &mut Vec<Text>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, out)),
            Shape::Text(t) => {
                let s = t.galley.text().trim().to_string();
                if s.is_empty() || t.fallback_color.a() == 0 {
                    return;
                }
                let rows: Vec<Rect> = if t.angle == 0.0 {
                    let at = t.pos.to_vec2();
                    let rows = t.galley.rows.iter().filter(|r| !r.row.glyphs.is_empty());
                    rows.map(|r| r.rect_without_leading_space().translate(at)).collect()
                } else {
                    vec![t.visual_bounding_rect()]
                };
                let rect = rows.iter().fold(Rect::NOTHING, |a, r| a.union(*r));
                let mut placed = t.galley.rows.iter().filter(|r| !r.row.glyphs.is_empty());
                let baseline = match (placed.next(), placed.next()) {
                    (Some(r), None) if t.angle == 0.0 => Some(t.pos.y + r.pos.y + r.row.glyphs[0].pos.y),
                    _ => None,
                };
                let size = t.galley.job.sections.first().map_or(0.0, |s| s.format.font_id.size);
                out.push(Text { s, rows, rect, clip, baseline, size });
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in shapes {
        walk(&c.shape, c.clip_rect, &mut out);
    }
    out
}

/// One frame: its words, and the views of its scroll areas.
struct Frame {
    texts: Vec<Text>,
    views: Vec<Rect>,
}

fn draw(app: &mut App, ctx: &egui::Context, size: [f32; 2]) -> Frame {
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(size[0], size[1]))),
        ..Default::default()
    };
    ctx.data_mut(|d| d.remove::<Vec<Rect>>(theme::scroll_views()));
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    out.textures_delta.clear();
    let views = ctx.data(|d| d.get_temp::<Vec<Rect>>(theme::scroll_views())).unwrap_or_default();
    Frame { texts: texts(&out.shapes), views }
}

/// Frames for `seconds` of camera time, from a camera that is not paced.
fn feed(e: &mut Engine, cam: &mut Synthetic, seconds: f64) {
    let fps = cam.mode().fps;
    for k in 0..(seconds * fps) as usize {
        let frame = cam.next_frame().unwrap();
        let finder = e.finder.clone();
        e.absorb(process(&frame, &finder, k % 15 == 0));
    }
}

/// An app with a live camera, a status, every station measured and a sample under way.
fn busy_app(ctx: &egui::Context) -> (App, Synthetic) {
    let mut settings = AppSettings::default();
    settings.camera.device = Some("No such camera".into());
    let app = App::with(ctx, settings, None);
    crate::app::wait_until_failed(&app);
    let small = Synthetic::open(None).modes()[3];
    let mut cam = Synthetic::open(Some(small));
    cam.paced = false;
    {
        let mut e = lock(&app.engine);
        let (device, modes, mode, controls) = (cam.device().clone(), cam.modes().to_vec(), cam.mode(), cam.controls());
        e.opened(device, modes, mode, controls, Vec::new());
        feed(&mut e, &mut cam, 90.0);
        e.sample_rule = SampleRule { min_s: 0.5, max_s: 1.0, target_um: None };
        let stations = e.current_run().unwrap().stations.len();
        for station in std::iter::once(0).chain(1..stations).chain([0]) {
            e.take_sample(station, None);
            feed(&mut e, &mut cam, 1.5);
        }
        e.take_sample(1, None);
        feed(&mut e, &mut cam, 0.3);
    }
    (app, cam)
}

/// What is wrong with one drawn frame.
fn problems(frame: &Frame, screen: Rect) -> Vec<String> {
    let texts = &frame.texts;
    let mut found = Vec::new();
    for t in texts {
        if t.s.starts_with("Double use of") || t.s.starts_with("First use of") {
            found.push(format!("ID clash: {}", t.s));
        }
        // Above or below a scroll area's view is out of sight but a scroll away. Its clip is the
        // view, a part of it, or nothing where a canvas clipped to the view is scrolled away.
        let scrolled = frame.views.iter().any(|v| {
            let v = v.expand(0.5);
            let inside = v.contains_rect(t.clip) || !t.clip.is_positive();
            let across = t.rect.right() > v.left() && t.rect.left() < v.right();
            inside && across && (t.rect.bottom() <= v.top() || t.rect.top() >= v.bottom())
        });
        if scrolled {
            continue;
        }
        if !t.rect.intersects(screen) {
            found.push(format!("{:?} drawn off screen at {:.0},{:.0}", t.s, t.rect.left(), t.rect.top()));
            continue;
        }
        if !t.rect.intersects(t.clip) {
            found.push(format!("{:?} drawn outside its panel at {:.0},{:.0}", t.s, t.rect.left(), t.rect.top()));
            continue;
        }
        let clip = t.clip.intersect(screen);
        let level = t.rect.top() < clip.bottom() - 2.0 && t.rect.bottom() > clip.top() + 2.0;
        let (l, r) = (t.rect.left() < clip.left() - 1.0, t.rect.right() > clip.right() + 1.0);
        if level && (l || r) {
            let (side, past) =
                if l { ("left", clip.left() - t.rect.left()) } else { ("right", t.rect.right() - clip.right()) };
            found.push(format!("{:?} cut off at the {side} by {past:.0}", t.s));
        }
    }
    // The ink of each line as it shows: inside its clip and the window, less a point of slack.
    let seen = |t: &Text| -> Vec<Rect> {
        t.rows.iter().map(|r| r.intersect(t.clip).intersect(screen).shrink(1.0)).filter(|r| r.is_positive()).collect()
    };
    let lines: Vec<Vec<Rect>> = texts.iter().map(seen).collect();
    for (i, a) in texts.iter().enumerate() {
        for (b, rb) in texts.iter().zip(&lines).skip(i + 1) {
            let hit = lines[i]
                .iter()
                .flat_map(|ra| rb.iter().map(|r| ra.intersect(*r)))
                .find(|r| r.is_positive() && r.area() > 4.0);
            if let Some(both) = hit {
                found.push(format!("{:?} over {:?} at {:.0},{:.0}", a.s, b.s, both.left(), both.top()));
            }
            if let Some(off) = off_baseline(a, b) {
                found.push(format!("{:?} and {:?} share a line but sit {off:.1} apart", a.s, b.s));
            }
        }
    }
    found
}

/// How far apart the baselines of two neighbouring words of about the same size on one line are,
/// when that shows: a table row or a label beside its value.
fn off_baseline(a: &Text, b: &Text) -> Option<f32> {
    let (ba, bb) = (a.baseline?, b.baseline?);
    let alike = a.size > 0.0 && (a.size / b.size - 1.0).abs() < 0.12;
    let shared = a.rect.max.y.min(b.rect.max.y) - a.rect.min.y.max(b.rect.min.y);
    let level = shared > 0.6 * a.rect.height().min(b.rect.height());
    let gap = (b.rect.min.x - a.rect.max.x).max(a.rect.min.x - b.rect.max.x);
    let near = (0.0..64.0).contains(&gap);
    (a.clip == b.clip && alike && level && near && (ba - bb).abs() > 1.0).then_some((ba - bb).abs())
}

/// Words that appear once in both frames and moved between them.
fn moved(before: &[Text], after: &[Text]) -> Vec<String> {
    let once = |texts: &[Text], s: &str| {
        let mut it = texts.iter().filter(|t| t.s == s);
        match (it.next(), it.next()) {
            (Some(t), None) => Some(t.rect),
            _ => None,
        }
    };
    let mut found = Vec::new();
    for t in before {
        if t.s.chars().filter(|c| c.is_alphabetic()).count() < 3 || FOLLOWS_DATA.contains(&t.s.as_str()) {
            continue;
        }
        if let (Some(a), Some(b)) = (once(before, &t.s), once(after, &t.s))
            && (a.min - b.min).length() > 0.5
        {
            found.push(format!("{:?} moved from {:.1},{:.1} to {:.1},{:.1}", t.s, a.min.x, a.min.y, b.min.x, b.min.y));
        }
    }
    found
}

#[test]
fn every_screen_lays_out_at_every_size() {
    let mut report = Vec::new();
    let (mut app, mut cam) = busy_app(&egui::Context::default());
    for size in SIZES {
        // A new window each time, so panel widths start from their defaults for the size.
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(size[0], size[1]));
        for which in Screen::ALL {
            app.screen = which;
            // New layouts measure on their first passes.
            for _ in 0..3 {
                draw(&mut app, &ctx, size);
            }
            let before = draw(&mut app, &ctx, size);
            let tag = format!("{:.0}x{:.0} {which:?}", size[0], size[1]);
            report.extend(problems(&before, screen).into_iter().map(|p| format!("{tag}: {p}")));
            // A little more camera time: the numbers change and nothing else should move.
            feed(&mut lock(&app.engine), &mut cam, 0.5);
            draw(&mut app, &ctx, size);
            let after = draw(&mut app, &ctx, size);
            report.extend(moved(&before.texts, &after.texts).into_iter().map(|p| format!("{tag}: {p}")));
        }
    }
    report.dedup();
    assert!(report.is_empty(), "{} layout problems:\n{}", report.len(), report.join("\n"));
}
