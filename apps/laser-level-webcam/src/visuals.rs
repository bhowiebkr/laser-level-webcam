//! Drawings that show what a setting means: the exposure against the frame period and the
//! flicker of the room lights, the grey levels of the frame against the clip level, the sensor's
//! pixels against a µm ruler, the strips the finder cuts the frame into, and the zones it fits
//! the beam in.

use std::f64::consts::PI;

use eframe::egui::{self, Align2, Color32, FontId, Painter, Rect, RichText, Sense, Shape, Stroke, pos2, vec2};
use laser_core::beam::FinderConfig;
use laser_core::camera::{self, ControlInfo, Mains, ModeScaling, Sensor, VideoMode};
use laser_core::frame::{LineAxis, Orientation};

use crate::engine::Detail;
use crate::{plot, theme};

/// The colour of the laser in the drawings.
pub const LASER: Color32 = Color32::from_rgb(240, 82, 64);

fn small(text: impl Into<String>, color: Color32) -> RichText {
    RichText::new(text).small().color(color)
}

/// A drawing area across the panel, with a well behind it.
/// A drawing as wide as the panel. It grows taller as the panel widens, up to 1.6 times `height`.
fn canvas(ui: &mut egui::Ui, height: f32) -> (Rect, Painter) {
    let width = ui.available_width();
    let height = height * (width / 320.0).clamp(1.0, 1.6);
    let (response, painter) = ui.allocate_painter(vec2(width, height), Sense::hover());
    let r = response.rect;
    painter.rect_filled(r, 6.0, theme::CANVAS);
    painter.rect_stroke(r, 6.0, Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside);
    (r.shrink(8.0), painter)
}

fn text(painter: &Painter, at: egui::Pos2, align: Align2, s: impl ToString, color: Color32) {
    painter.text(at, align, s.to_string(), FontId::proportional(11.0), color);
}

fn ms(s: f64) -> String {
    if s < 0.01 { format!("{:.2} ms", s * 1000.0) } else { format!("{:.1} ms", s * 1000.0) }
}

/// The exposure against the frame period and the room lights' flicker, then the frame's grey
/// levels. Returns an exposure value to set when the button to step it is clicked.
pub fn exposure(
    ui: &mut egui::Ui,
    info: &ControlInfo,
    mode: Option<&VideoMode>,
    mains: Mains,
    detail: Option<&Detail>,
    clip_level: u8,
) -> Option<i64> {
    let period = mode.map_or(1.0 / 30.0, VideoMode::frame_period_s);
    let flicker = mains.flicker_period_s();
    if let Some(exp) = info.exposure_s(info.value) {
        timeline(ui, exp, period, flicker);
        let cycles = exp / flicker;
        let (line, color) = if exp > period {
            ("Longer than the frame period: the camera slows down or drops frames.".to_owned(), theme::ERROR)
        } else if cycles < 0.95 {
            (
                format!(
                    "Shorter than one flicker of the room lights ({}), so each frame catches a different part of it. The beam does not flicker; the background does.",
                    ms(flicker)
                ),
                theme::TEXT_DIM,
            )
        } else if (cycles - cycles.round()).abs() < 0.05 {
            (
                format!("{:.0} whole flicker cycles: the room light is the same in every frame.", cycles.round()),
                theme::OK,
            )
        } else {
            let k = cycles.round().max(1.0);
            (
                format!(
                    "{cycles:.2} flicker cycles: the room light changes from frame to frame. {k:.0} cycles is {}.",
                    ms(k * flicker)
                ),
                theme::TEXT_DIM,
            )
        };
        ui.label(small(line, color));
    }
    let detail = detail?;
    let zones = detail.inspection.zones.ok();
    // The beam's top in grey levels, and the band its peak should be in, above the background.
    let beam = zones.map(|z| (z.background.0 + z.background.1 * z.centre, z.peak));
    histogram(ui, detail, clip_level, beam);
    let share = detail.share_at_or_above(clip_level);
    ui.label(small(
        format!("Brightest pixel {}; {:.3} % at or above the clip level {clip_level}", detail.peak(), share * 100.0),
        if share > 0.0 { theme::ACCENT } else { theme::TEXT_DIM },
    ));
    let (bg, peak) = beam?;
    let clipped = bg + peak >= f64::from(clip_level);
    let manual = info.auto != Some(true);
    match camera::next_exposure(info, info.value, peak, clipped) {
        Some(v) => {
            let why = if clipped || peak >= camera::PEAK_HIGH { "too bright" } else { "faint" };
            let button = egui::Button::new(format!("Step the exposure (beam {why})"));
            let r = ui
                .add_enabled(manual, button)
                .on_hover_text(
                    "One step towards a beam peak in the green band. Repeat until it says the exposure is good.",
                )
                .on_disabled_hover_text("The exposure is automatic: lock the camera for measuring first.");
            r.clicked().then_some(v)
        }
        None => {
            let good = (camera::PEAK_LOW..camera::PEAK_HIGH).contains(&peak) && !clipped;
            if good {
                ui.label(small(format!("Exposure is good: beam peak {peak:.0} above the background."), theme::OK));
            }
            None
        }
    }
}

fn timeline(ui: &mut egui::Ui, exp: f64, period: f64, flicker: f64) {
    let (r, p) = canvas(ui, 96.0);
    let span = (period.max(exp) * 1.12).max(2.0 * flicker);
    let x = |t: f64| r.left() + 34.0 + (t / span) as f32 * (r.width() - 34.0);
    let row = |k: f32| r.top() + k;

    // Frame starts.
    text(&p, pos2(r.left(), row(8.0)), Align2::LEFT_CENTER, "frames", theme::TEXT_DIM);
    let mut k = 0.0;
    while k * period <= span {
        let sx = x(k * period);
        p.line_segment([pos2(sx, row(2.0)), pos2(sx, row(14.0))], Stroke::new(1.0, theme::TEXT_DIM));
        k += 1.0;
    }
    p.line_segment([pos2(x(0.0), row(8.0)), pos2(x(period.min(span)), row(8.0))], Stroke::new(1.0, theme::TEXT_DIM));
    text(
        &p,
        pos2((x(0.0) + x(period.min(span))) / 2.0, row(16.0)),
        Align2::CENTER_TOP,
        format!("every {}", ms(period)),
        theme::TEXT_DIM,
    );

    // The exposure.
    let over = exp > period;
    let fill = if over { theme::ERROR } else { theme::ACCENT };
    text(&p, pos2(r.left(), row(39.0)), Align2::LEFT_CENTER, "open", theme::TEXT_DIM);
    let bar = Rect::from_min_max(pos2(x(0.0), row(32.0)), pos2(x(exp.min(span)).max(x(0.0) + 2.0), row(46.0)));
    p.rect_filled(bar, 2.0, theme::faded(fill, 200));
    let label = format!("exposure {}", ms(exp));
    if bar.width() > 90.0 {
        text(&p, bar.center(), Align2::CENTER_CENTER, label, theme::ON_ACCENT);
    } else {
        text(&p, pos2(bar.right() + 4.0, bar.center().y), Align2::LEFT_CENTER, label, fill);
    }

    // The room light, flickering at twice the mains frequency, and the part the exposure catches.
    let (top, base) = (row(54.0), r.bottom());
    text(&p, pos2(r.left(), (top + base) / 2.0), Align2::LEFT_CENTER, "light", theme::TEXT_DIM);
    let level = |t: f64| base - (((PI * t / flicker).sin()).abs() as f32) * (base - top);
    let n = 160;
    let step = span / f64::from(n);
    for i in 0..n {
        let (t0, t1) = (f64::from(i) * step, f64::from(i + 1) * step);
        if t0 < exp {
            let r = Rect::from_min_max(pos2(x(t0), level((t0 + t1) / 2.0)), pos2(x(t1.min(exp)), base));
            p.rect_filled(r, 0.0, theme::faded(theme::ACCENT, 70));
        }
    }
    let pts: Vec<_> = (0..=n).map(|i| pos2(x(f64::from(i) * step), level(f64::from(i) * step))).collect();
    p.add(Shape::line(pts, Stroke::new(1.5, theme::BLUE)));
}

fn histogram(ui: &mut egui::Ui, detail: &Detail, clip_level: u8, beam: Option<(f64, f64)>) {
    let (r, p) = canvas(ui, 78.0);
    let r = Rect::from_min_max(r.min, pos2(r.right(), r.bottom() - 12.0));
    let x = |level: f64| r.left() + (level / 256.0) as f32 * r.width();
    if let Some((bg, _)) = beam {
        let band =
            Rect::from_x_y_ranges(x(bg + camera::PEAK_LOW)..=x((bg + camera::PEAK_HIGH).min(256.0)), r.y_range());
        p.rect_filled(band, 0.0, theme::faded(theme::OK, 40));
    }
    let max = detail.histogram.iter().copied().max().unwrap_or(1).max(1);
    let scale = f64::from(max).ln_1p();
    let w = r.width() / 256.0;
    for (level, &n) in detail.histogram.iter().enumerate() {
        if n == 0 {
            continue;
        }
        let h = (f64::from(n).ln_1p() / scale) as f32 * r.height();
        let color = if level >= usize::from(clip_level) { theme::ERROR } else { theme::TEXT_DIM };
        let left = x(level as f64);
        p.rect_filled(Rect::from_min_max(pos2(left, r.bottom() - h), pos2(left + w.max(1.0), r.bottom())), 0.0, color);
    }
    let clip = x(f64::from(clip_level));
    p.extend(Shape::dashed_line(
        &[pos2(clip, r.top()), pos2(clip, r.bottom())],
        Stroke::new(1.0, theme::ERROR),
        4.0,
        3.0,
    ));
    text(&p, pos2(clip - 3.0, r.top()), Align2::RIGHT_TOP, "clip", theme::ERROR);
    if let Some((bg, peak)) = beam {
        let top = x(bg + peak);
        p.line_segment([pos2(top, r.top()), pos2(top, r.bottom())], Stroke::new(1.5, theme::ACCENT));
        let size = p.layout_no_wrap("beam".into(), FontId::proportional(11.0), theme::ACCENT).size();
        let wide = size.x;
        let (at, align) =
            if top + 3.0 + wide <= r.right() { (top + 3.0, Align2::LEFT_TOP) } else { (top - 3.0, Align2::RIGHT_TOP) };
        text(&p, pos2(at, r.top() + size.y + 1.0), align, "beam", theme::ACCENT);
    }
    for level in [0.0, 64.0, 128.0, 192.0, 255.0] {
        text(&p, pos2(x(level), r.bottom() + 2.0), Align2::CENTER_TOP, format!("{level:.0}"), theme::TEXT_DIM);
    }
}

/// The sensor, the part of it a frame covers, and a few frame pixels against a µm ruler.
pub fn sensor(ui: &mut egui::Ui, sensor: &Sensor, mode: Option<&VideoMode>, axis: LineAxis) {
    if !sensor.is_valid() {
        return;
    }
    let (fw, fh) = mode.map_or((sensor.width_px, sensor.height_px), |m| (m.width, m.height));
    let um_per_px = sensor.um_per_px(fw, fh, axis);
    let (r, p) = canvas(ui, 132.0);
    let split = r.left() + r.width() * 0.58;

    // The sensor to scale, and the frame on it.
    let area = Rect::from_min_max(pos2(r.left() + 4.0, r.top() + 4.0), pos2(split - 18.0, r.bottom() - 16.0));
    let (w_mm, h_mm) = (sensor.width_mm(), sensor.height_mm());
    let k = (area.width() / w_mm as f32).min(area.height() / h_mm as f32);
    let chip = Rect::from_center_size(area.center(), vec2(w_mm as f32 * k, h_mm as f32 * k));
    p.rect_filled(chip, 2.0, theme::NODE);
    let frame = match sensor.scaling {
        ModeScaling::Scaled => chip,
        ModeScaling::Cropped => {
            let pitch = sensor.pixel_pitch_um as f32 / 1000.0 * k;
            Rect::from_center_size(chip.center(), vec2(fw as f32 * pitch, fh as f32 * pitch)).intersect(chip)
        }
    };
    p.rect_filled(frame, 1.0, theme::faded(theme::ACCENT, 50));
    p.rect_stroke(frame, 1.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
    p.rect_stroke(chip, 2.0, Stroke::new(1.0, theme::LINE), egui::StrokeKind::Outside);
    text(&p, pos2(chip.center().x, chip.bottom() + 2.0), Align2::CENTER_TOP, format!("{w_mm:.2} mm"), theme::TEXT_DIM);
    text(&p, pos2(chip.right() + 3.0, chip.center().y), Align2::LEFT_CENTER, format!("{h_mm:.2}"), theme::TEXT_DIM);
    // The laser line and the direction of the reading.
    let c = frame.center();
    let arrow = Stroke::new(1.5, theme::TEXT);
    match axis {
        LineAxis::Vertical => {
            p.line_segment([pos2(c.x, frame.top()), pos2(c.x, frame.bottom())], Stroke::new(2.0, LASER));
            let (a, b) = (pos2(c.x - 22.0, c.y), pos2(c.x + 22.0, c.y));
            p.arrow(c, a - c, arrow);
            p.arrow(c, b - c, arrow);
        }
        LineAxis::Horizontal => {
            p.line_segment([pos2(frame.left(), c.y), pos2(frame.right(), c.y)], Stroke::new(2.0, LASER));
            let (a, b) = (pos2(c.x, c.y - 18.0), pos2(c.x, c.y + 18.0));
            p.arrow(c, a - c, arrow);
            p.arrow(c, b - c, arrow);
        }
    }
    text(&p, pos2(frame.left() + 3.0, frame.top() + 2.0), Align2::LEFT_TOP, format!("frame {fw}×{fh}"), theme::ACCENT);

    // A magnifier: 3 × 3 frame pixels, the sensor pixels under them, and a ruler.
    let size = (r.right() - split).min(r.height() - 22.0);
    let mag = Rect::from_min_size(pos2(split + (r.right() - split - size) / 2.0, r.top()), vec2(size, size));
    let span_um = 3.0 * um_per_px;
    let s = |um: f64| (um / span_um) as f32 * size;
    p.rect_filled(mag, 0.0, theme::NODE);
    let painter = p.with_clip_rect(mag);
    let pitch = sensor.pixel_pitch_um;
    let fine = Stroke::new(1.0, theme::faded(theme::TEXT_DIM, 70));
    if s(pitch) >= 3.0 {
        let mut u = 0.0;
        while u <= span_um + 1e-9 {
            painter.line_segment([pos2(mag.left() + s(u), mag.top()), pos2(mag.left() + s(u), mag.bottom())], fine);
            painter.line_segment([pos2(mag.left(), mag.top() + s(u)), pos2(mag.right(), mag.top() + s(u))], fine);
            u += pitch;
        }
    }
    let bold = Stroke::new(1.5, theme::ACCENT);
    for i in 0..=3 {
        let d = s(f64::from(i) * um_per_px);
        painter.line_segment([pos2(mag.left() + d, mag.top()), pos2(mag.left() + d, mag.bottom())], bold);
        painter.line_segment([pos2(mag.left(), mag.top() + d), pos2(mag.right(), mag.top() + d)], bold);
    }
    // A round length for the ruler, about two thirds of the magnifier.
    let ruler = plot::ticks([0.0, span_um * 0.7], 1).last().copied().filter(|v| *v > 0.0).unwrap_or(um_per_px);
    let y = mag.bottom() + 8.0;
    let (a, b) = (mag.left(), mag.left() + s(ruler));
    p.line_segment([pos2(a, y), pos2(b, y)], Stroke::new(2.0, theme::TEXT));
    p.line_segment([pos2(a, y - 4.0), pos2(a, y + 4.0)], Stroke::new(1.0, theme::TEXT));
    p.line_segment([pos2(b, y - 4.0), pos2(b, y + 4.0)], Stroke::new(1.0, theme::TEXT));
    text(&p, pos2(b + 4.0, y), Align2::LEFT_CENTER, format!("{} µm", plot::tick_text(ruler, ruler)), theme::TEXT);

    let per = um_per_px / pitch;
    let line = match sensor.scaling {
        ModeScaling::Cropped => format!("A frame px is a sensor px: {um_per_px:.3} µm."),
        ModeScaling::Scaled if (per - 1.0).abs() < 1e-6 => {
            format!("Full resolution: a frame px is a sensor px, {um_per_px:.3} µm.")
        }
        ModeScaling::Scaled => {
            format!("A frame px (orange) covers {per:.2} sensor px (grey) across the line: {um_per_px:.3} µm.")
        }
    };
    ui.label(small(line, theme::TEXT_DIM));
}

/// The frame as the live view shows it: turned so the line runs level and heights read up and
/// down, with the corner the camera's first pixel is in and the height the frame covers.
pub fn orientation(ui: &mut egui::Ui, sensor: &Sensor, mode: Option<&VideoMode>, o: Orientation) {
    let (fw, fh) = mode.map_or((sensor.width_px, sensor.height_px), |m| (m.width, m.height));
    let (vw, vh) = o.view_size(f64::from(fw), f64::from(fh));
    let um = if sensor.is_valid() { sensor.um_per_px(fw, fh, o.axis()) } else { 0.0 };
    let range_mm = vh * um / 1000.0;
    let (r, p) = canvas(ui, 150.0);

    // The frame, as large as fits beside the height bracket.
    let side = 84.0;
    let area = Rect::from_min_max(pos2(r.left() + 4.0, r.top() + 14.0), pos2(r.right() - side, r.bottom() - 2.0));
    let k = (area.width() / vw as f32).min(area.height() / vh as f32);
    let frame = Rect::from_center_size(area.center(), vec2(vw as f32 * k, vh as f32 * k));
    p.rect_filled(frame, 2.0, theme::NODE);
    p.rect_stroke(frame, 2.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
    text(&p, pos2(frame.center().x, r.top()), Align2::CENTER_TOP, "up", theme::TEXT_DIM);

    // The laser line runs level. Lifting the block lowers it in the view, and the height goes up.
    let y = frame.center().y;
    let ends = [pos2(frame.left(), y), pos2(frame.right(), y)];
    p.line_segment(ends, Stroke::new(6.0, theme::faded(LASER, 50)));
    p.line_segment(ends, Stroke::new(2.0, LASER));
    let x = frame.left() + frame.width() * 0.3;
    let drop = (frame.height() * 0.32).clamp(10.0, 30.0);
    p.arrow(pos2(x, y + 4.0), vec2(0.0, drop), Stroke::new(1.5, theme::TEXT));
    text(&p, pos2(x + 6.0, y + 4.0 + drop / 2.0), Align2::LEFT_CENTER, "block lifted", theme::TEXT);

    // The corner the camera's first pixel is in: it shows how the image was turned.
    let (cx, cy) = o.to_view(0.0, 0.0, f64::from(fw), f64::from(fh));
    let (left, top) = (cx < vw / 2.0, cy < vh / 2.0);
    let corner = pos2(frame.left() + cx as f32 * k, frame.top() + cy as f32 * k);
    let inward = vec2(if left { 5.0 } else { -5.0 }, if top { 5.0 } else { -5.0 });
    p.circle_filled(corner + inward, 3.0, theme::BLUE);
    let align = match (left, top) {
        (true, true) => Align2::LEFT_TOP,
        (false, true) => Align2::RIGHT_TOP,
        (true, false) => Align2::LEFT_BOTTOM,
        (false, false) => Align2::RIGHT_BOTTOM,
    };
    text(&p, corner + inward * 2.2, align, "px 0,0", theme::BLUE);

    // The height the frame covers.
    let bx = frame.right() + 12.0;
    let bracket = Stroke::new(1.5, theme::TEXT);
    p.line_segment([pos2(bx, frame.top()), pos2(bx, frame.bottom())], bracket);
    for y in [frame.top(), frame.bottom()] {
        p.line_segment([pos2(bx - 4.0, y), pos2(bx + 4.0, y)], bracket);
    }
    if um > 0.0 {
        text(&p, pos2(bx + 7.0, frame.center().y - 1.0), Align2::LEFT_BOTTOM, format!("{range_mm:.2} mm"), theme::TEXT);
    }
    text(&p, pos2(bx + 7.0, frame.center().y + 1.0), Align2::LEFT_TOP, format!("{vh:.0} px"), theme::TEXT_DIM);

    let mut line = format!("{}. Heights are read up and down the view", o.describe());
    if um > 0.0 {
        line += &format!(", over {range_mm:.2} mm");
    }
    ui.label(small(line + ".", theme::TEXT_DIM));
    if vw > vh * 1.05 && um > 0.0 {
        let other = match o.axis() {
            LineAxis::Vertical => LineAxis::Horizontal,
            LineAxis::Horizontal => LineAxis::Vertical,
        };
        let turned_mm = vw * sensor.um_per_px(fw, fh, other) / 1000.0;
        ui.label(small(
            format!("The sensor's short side reads the heights. Turn the camera 90° on the block, then turn the view, to read over {turned_mm:.2} mm."),
            theme::ACCENT,
        ));
    }
    ui.label(small(
        "Check with a shim under the block: the line should move down the view. If it moves up, flip top to bottom.",
        theme::TEXT_DIM,
    ));
}

/// The frame cut into strips along the line, the part of it searched, and the centres the
/// strips give, with one dropped.
pub fn strips(ui: &mut egui::Ui, cfg: &FinderConfig, size: (usize, usize)) {
    let (r, p) = canvas(ui, 120.0);
    let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
    let k = (r.width() / w).min(r.height() / h);
    let frame = Rect::from_center_size(r.center(), vec2(w * k, h * k));
    p.rect_filled(frame, 0.0, theme::NODE);
    let region = cfg
        .roi
        .and_then(|roi| roi.clamped(size.0, size.1))
        .map(|roi| {
            Rect::from_min_size(
                pos2(frame.left() + roi.x as f32 * k, frame.top() + roi.y as f32 * k),
                vec2(roi.width as f32 * k, roi.height as f32 * k),
            )
        })
        .unwrap_or(frame);
    if region != frame {
        // Shade what is not searched.
        let shade = theme::faded(Color32::BLACK, 120);
        for part in [
            Rect::from_min_max(frame.min, pos2(frame.right(), region.top())),
            Rect::from_min_max(pos2(frame.left(), region.bottom()), frame.max),
            Rect::from_min_max(pos2(frame.left(), region.top()), pos2(region.left(), region.bottom())),
            Rect::from_min_max(pos2(region.right(), region.top()), pos2(frame.right(), region.bottom())),
        ] {
            p.rect_filled(part, 0.0, shade);
        }
        p.rect_stroke(region, 0.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
    }
    let n = cfg.strips.clamp(1, 64);
    let vertical = cfg.axis == LineAxis::Vertical;
    let along = if vertical { region.height() } else { region.width() };
    let strip = along / n as f32;
    let dropped = n / 3;
    let line_at = |t: f32| {
        // A slightly tilted line through the middle of the region; t runs 0..1 along it.
        let across = 0.5 + 0.04 * (t - 0.5);
        if vertical {
            pos2(region.left() + across * region.width(), region.top() + t * region.height())
        } else {
            pos2(region.left() + t * region.width(), region.top() + across * region.height())
        }
    };
    p.line_segment([line_at(0.0), line_at(1.0)], Stroke::new(3.0, theme::faded(LASER, 140)));
    let edge = Stroke::new(1.0, theme::faded(theme::TEXT_DIM, 90));
    for i in 0..n {
        let a = i as f32 * strip;
        if i > 0 {
            let seg = if vertical {
                [pos2(region.left(), region.top() + a), pos2(region.right(), region.top() + a)]
            } else {
                [pos2(region.left() + a, region.top()), pos2(region.left() + a, region.bottom())]
            };
            p.line_segment(seg, edge);
        }
        let mut c = line_at((i as f32 + 0.5) / n as f32);
        if n >= 3 && i == dropped {
            let off = (if vertical { region.width() } else { region.height() }) * 0.18;
            if vertical {
                c.x += off
            } else {
                c.y += off
            }
            let d = 3.5;
            let x = Stroke::new(1.5, theme::ERROR);
            p.line_segment([c - vec2(d, d), c + vec2(d, d)], x);
            p.line_segment([c + vec2(-d, d), c + vec2(d, -d)], x);
        } else if strip >= 3.0 {
            p.circle_filled(c, 2.5, theme::OK);
        }
    }
    let px = if vertical { size.1 } else { size.0 } as f32
        * (along / if vertical { frame.height() } else { frame.width() })
        / n as f32;
    ui.label(small(
        format!("{n} strips of about {px:.0} px. Each gives a centre (green); one that disagrees is dropped (red) and the line comes from the rest."),
        theme::TEXT_DIM,
    ));
}

/// The zones of a beam profile the finder uses, drawn for an ideal beam with the settings given.
pub fn fit_zones(ui: &mut egui::Ui, cfg: &FinderConfig) {
    let (r, p) = canvas(ui, 120.0);
    let r = Rect::from_min_max(r.min, pos2(r.right(), r.bottom() - 12.0));
    let reach = (cfg.background_sigmas + 2.0).max(6.0);
    let (bg, peak) = (0.12, 0.78);
    let x = |s: f64| r.left() + ((s + reach) / (2.0 * reach)) as f32 * r.width();
    let y = |v: f64| r.bottom() - v as f32 * r.height();

    // Background zones.
    let blue = theme::faded(theme::BLUE, 45);
    p.rect_filled(Rect::from_x_y_ranges(r.left()..=x(-cfg.background_sigmas), r.y_range()), 0.0, blue);
    p.rect_filled(Rect::from_x_y_ranges(x(cfg.background_sigmas)..=r.right(), r.y_range()), 0.0, blue);
    // The fit window.
    p.rect_filled(
        Rect::from_x_y_ranges(x(-cfg.window_sigmas)..=x(cfg.window_sigmas), r.y_range()),
        0.0,
        theme::faded(theme::ACCENT, 45),
    );
    // The noise the beam must stand above: its peak is at least min_snr times the noise.
    let noise = peak / cfg.min_snr.max(1.0);
    p.rect_filled(
        Rect::from_x_y_ranges(r.x_range(), y(bg + noise)..=y(bg - noise.min(bg))),
        0.0,
        theme::faded(theme::TEXT_DIM, 40),
    );
    p.extend(Shape::dashed_line(
        &[pos2(r.left(), y(bg)), pos2(r.right(), y(bg))],
        Stroke::new(1.0, theme::BLUE),
        5.0,
        3.0,
    ));

    let n = 200;
    let pts: Vec<_> = (0..=n)
        .map(|i| {
            let s = -reach + 2.0 * reach * f64::from(i) / f64::from(n);
            pos2(x(s), y(bg + peak * (-s * s / 2.0).exp()))
        })
        .collect();
    p.add(Shape::line(pts, Stroke::new(2.0, LASER)));
    // Full width at half maximum.
    let half = (2.0 * 2f64.ln()).sqrt();
    let hy = y(bg + peak / 2.0);
    p.line_segment([pos2(x(-half), hy), pos2(x(half), hy)], Stroke::new(1.5, theme::TEXT));
    text(&p, pos2(x(half) + 4.0, hy), Align2::LEFT_CENTER, "FWHM", theme::TEXT);

    text(
        &p,
        pos2((x(-cfg.window_sigmas) + x(cfg.window_sigmas)) / 2.0, r.top() + 2.0),
        Align2::CENTER_TOP,
        format!("fit ±{:.1}σ", cfg.window_sigmas),
        theme::ACCENT,
    );
    text(&p, pos2(r.left() + 3.0, r.top() + 2.0), Align2::LEFT_TOP, "background", theme::BLUE);
    text(
        &p,
        pos2(r.right() - 3.0, y(bg + noise) - 2.0),
        Align2::RIGHT_BOTTOM,
        format!("noise × {:.0}", cfg.min_snr),
        theme::TEXT_DIM,
    );
    for s in [-cfg.background_sigmas, -cfg.window_sigmas, 0.0, cfg.window_sigmas, cfg.background_sigmas] {
        let label = if s == 0.0 { "0".to_owned() } else { format!("{s:+.1}σ") };
        text(&p, pos2(x(s), r.bottom() + 2.0), Align2::CENTER_TOP, label, theme::TEXT_DIM);
    }
    ui.label(small(
        "The centre is worked out inside the orange window; the background (dashed) is fitted in the blue zones; the beam must rise above the grey noise band.",
        theme::TEXT_DIM,
    ));
}
