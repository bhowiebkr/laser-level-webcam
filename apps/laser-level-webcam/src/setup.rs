//! Setting up: what the finder makes of the frame, the live view with the line it found, and the
//! profile across the line with the zones it used.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, pos2, vec2};
use laser_core::beam::Fix;
use laser_core::frame::{LineAxis, Orientation, Roi};
use laser_core::units::Unit;

use crate::app::{App, no_beam_short};
use crate::engine::{CameraState, Detail, Engine};
use crate::plot::Axes;
use crate::theme::{self, Icon};
use crate::visuals::LASER;

pub fn draw(app: &mut App, ui: &mut egui::Ui, e: &mut Engine) {
    readouts(app, ui, e);
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        orientation_buttons(ui, &mut app.settings.orientation);
        ui.label(theme::dim(format!(
            "{}. Lift the block: the line should move down.",
            app.settings.orientation.describe()
        )))
        .on_hover_text(
            "How the camera's picture is turned now. To check the sign, lift the camera's block a little: the line should move down in the view. If it moves up, mirror the picture.",
        );
    });
    ui.add_space(4.0);

    // The view as large as it goes, with the profile below it or, when the view is upright,
    // beside it on its side, whichever leaves the larger view.
    let avail = ui.available_size();
    let (fw, fh) = e.live.detail.as_ref().map(|d| (d.width, d.height)).or(e.live.size).unwrap_or((1920, 1080));
    let (vw, vh) = app.settings.orientation.view_size(fw as f64, fh as f64);
    let (vw, vh) = (vw as f32, vh as f32);
    let gap = 8.0;
    let profile_h = (avail.y * 0.3).clamp(150.0, 320.0);
    let profile_w = (avail.x * 0.28).clamp(200.0, 440.0);
    let below = ((avail.x - 8.0) / vw).min((avail.y - profile_h - gap - 8.0) / vh).max(0.0);
    let beside = ((avail.x - profile_w - gap - 8.0) / vw).min((avail.y - 48.0) / vh).max(0.0);
    if beside > below * 1.02 {
        let view_w = (vw * beside + 8.0).max(160.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let placed = live_view(app, ui, e, vec2(view_w, avail.y));
            profile_beside(app, ui, e, vec2((avail.x - view_w - gap).max(120.0), avail.y), placed);
        });
    } else {
        let view_h = (vh * below + 8.0).clamp(140.0, (avail.y - profile_h - gap).max(140.0));
        live_view(app, ui, e, vec2(avail.x, view_h));
        ui.add_space(gap);
        profile(app, ui, e, ui.available_height().max(130.0));
    }
}

/// Where the live view drew the frame: the screen rect it fills and screen points per frame px.
#[derive(Clone, Copy)]
struct Placed {
    img: Rect,
    k: f32,
}

pub fn with_unit(unit: Unit, um: f64) -> String {
    format!("{} {}", unit.format(um), unit.label())
}

/// A small tile: a name over a value.
/// The value is right-aligned, and the tile keeps the widest it has been with room for a minus
/// sign, so neither it nor the tiles after it move as the number changes.
pub fn stat(ui: &mut egui::Ui, name: &str, value: impl Into<String>, color: Color32, help: &str) {
    let value: String = value.into();
    let font = |s: &str| RichText::new(s).monospace().size(15.0);
    let sign = if value.starts_with('-') { 0.0 } else { theme::text_width(ui, font("-")) };
    let key = name;
    let name = RichText::new(name).small().color(theme::TEXT_DIM);
    let value = font(&value).color(color);
    let width = theme::text_width(ui, name.clone()).max(theme::text_width(ui, value.clone()) + sign);
    let width = theme::steady_width(ui, key, width);
    theme::wrap_before(ui, width + 22.0);
    theme::card()
        .inner_margin(theme::STAT_MARGIN)
        .show(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            // The card takes the row's direction: stack the name over the value inside it.
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.label(name);
                ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| ui.label(value));
            });
        })
        .response
        .on_hover_text(help);
}

fn readouts(app: &App, ui: &mut egui::Ui, e: &Engine) {
    let unit = app.settings.unit;
    let k = e.um_per_px();
    theme::cards(ui, |ui| match &e.live.fix {
        Some(Ok(fix)) => {
            let color = if fix.warnings.any() { theme::ACCENT } else { theme::TEXT };
            stat(
                ui,
                "Position",
                format!("{:.2} px", fix.position_px),
                color,
                "Where the line crosses the middle of the searched region, in frame px from the edge.",
            );
            stat(
                ui,
                "Position, scaled",
                with_unit(unit, fix.position_px * k),
                color,
                "The same, from the sensor's scale.",
            );
            stat(
                ui,
                "Within the frame ±",
                with_unit(unit, fix.uncertainty_px * k),
                theme::TEXT,
                "Standard uncertainty from the scatter of the strip centres in this frame. The wait screen measures the frame-to-frame noise.",
            );
            stat(ui, "FWHM", format!("{:.1} px", fix.fwhm_px), theme::TEXT, "Width of the beam at half its height.");
            stat(
                ui,
                "Signal",
                format!("{:.0}×", fix.snr),
                if fix.snr < 2.0 * app.settings.finder.min_snr { theme::ACCENT } else { theme::TEXT },
                "Beam peak over the noise of the profile.",
            );
            stat(ui, "Peak", format!("{:.0}", fix.peak), theme::TEXT, "Beam height above the background, grey levels.");
            stat(
                ui,
                "Tilt",
                format!("{:.1} mm/m", fix.tilt * 1000.0),
                if fix.warnings.tilted { theme::ACCENT } else { theme::TEXT },
                "How far the line is from square to the sensor. Turn the camera to square it.",
            );
            stat(
                ui,
                "Strips",
                format!("{}/{}", fix.strips_used, app.settings.finder.strips),
                if fix.warnings.strips_dropped { theme::ACCENT } else { theme::TEXT },
                "Strips that gave a centre that agrees with the others.",
            );
            warnings(ui, fix);
        }
        Some(Err(why)) => {
            theme::chip(ui, no_beam_short(*why), theme::ERROR)
                .on_hover_text("The finder found no beam in this frame, so the frame gives no reading. The reason is beside it.");
            ui.label(RichText::new(why.text()).color(theme::TEXT_DIM));
        }
        None => {
            ui.label(theme::dim("No frame yet."))
                .on_hover_text("The readings start with the first frame from the camera. Pick a camera in the side panel.");
        }
    });
}

fn warnings(ui: &mut egui::Ui, fix: &Fix) {
    let w = fix.warnings;
    let list = [
        (
            w.clipped,
            "Clipped",
            "Pixels in the beam are at full scale. Lower the exposure, the gain or the laser power.",
        ),
        (w.tilted, "Tilted", "The line is not square to the sensor."),
        (
            w.near_edge,
            "Near the edge",
            "The fit window does not fit on the frame, so the centre comes from the half-height crossings, which a lopsided beam pulls.",
        ),
        (
            w.strips_dropped,
            "Strips dropped",
            "Some strips gave no centre or disagreed with the rest: dust, a reflection, or a broken beam.",
        ),
    ];
    for (on, name, help) in list {
        if on {
            theme::card_chip(ui, name, theme::ACCENT).on_hover_text(help);
        }
    }
}

/// Buttons that turn the view a quarter turn and flip it, as it is shown.
pub fn orientation_buttons(ui: &mut egui::Ui, o: &mut Orientation) {
    let now = *o;
    let buttons = [
        (Icon::TurnLeft, "Turn left", "Turn the view 90° anticlockwise.", now.turned_ccw()),
        (Icon::TurnRight, "Turn right", "Turn the view 90° clockwise.", now.turned_cw()),
        (Icon::FlipLeftRight, "Flip sides", "Flip the view left to right.", now.flipped_left_right()),
        (
            Icon::FlipUpDown,
            "Flip top to bottom",
            "Flip the view top to bottom. Do it when lifting the block moves the line up the view.",
            now.flipped_up_down(),
        ),
    ];
    for (icon, text, help, next) in buttons {
        if theme::icon_button(ui, icon, text).on_hover_text(help).clicked() {
            *o = next;
        }
    }
}

/// The searched region, in frame px.
fn region(app: &App, d: &Detail) -> Roi {
    app.settings.finder.roi.and_then(|r| r.clamped(d.width, d.height)).unwrap_or(Roi {
        x: 0,
        y: 0,
        width: d.width,
        height: d.height,
    })
}

fn live_view(app: &mut App, ui: &mut egui::Ui, e: &Engine, size: egui::Vec2) -> Option<Placed> {
    let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let outer = resp.rect;
    painter.rect_filled(outer, 6.0, theme::CANVAS);
    let (Some(tex), Some(d)) = (app.preview(), &e.live.detail) else {
        let msg = match &e.camera.state {
            CameraState::Closed => "No camera open".to_owned(),
            CameraState::Opening(n) => format!("Opening {n}…"),
            CameraState::Failed(why) => why.clone(),
            CameraState::Open => "Waiting for a frame…".to_owned(),
        };
        painter.text(outer.center(), Align2::CENTER_CENTER, msg, FontId::proportional(15.0), theme::TEXT_DIM);
        return None;
    };
    // The frame turned and flipped as the orientation says. Everything drawn over it is worked
    // out in frame px and mapped into the view.
    let o = app.settings.orientation;
    let (fw, fh) = (d.width as f64, d.height as f64);
    let (vw, vh) = o.view_size(fw, fh);
    let k = ((outer.width() - 8.0) / vw as f32).min((outer.height() - 8.0) / vh as f32);
    let img = Rect::from_center_size(outer.center(), vec2(vw as f32 * k, vh as f32 * k));
    let uv = |vx: f64, vy: f64| {
        let (x, y) = o.to_image(vx, vy, fw, fh);
        pos2((x / fw) as f32, (y / fh) as f32)
    };
    let mut mesh = egui::Mesh::with_texture(tex.id());
    for (pos, (vx, vy)) in [
        (img.left_top(), (0.0, 0.0)),
        (img.right_top(), (vw, 0.0)),
        (img.right_bottom(), (vw, vh)),
        (img.left_bottom(), (0.0, vh)),
    ] {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv: uv(vx, vy), color: Color32::WHITE });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
    let screen = |x: f64, y: f64| {
        let (vx, vy) = o.to_view(x, y, fw, fh);
        pos2(img.left() + vx as f32 * k, img.top() + vy as f32 * k)
    };
    let frame_at = |p: Pos2| o.to_image(f64::from((p.x - img.left()) / k), f64::from((p.y - img.top()) / k), fw, fh);

    // The searched region and its strips.
    let roi = region(app, d);
    let (x0, y0, x1, y1) = (roi.x as f64, roi.y as f64, (roi.x + roi.width) as f64, (roi.y + roi.height) as f64);
    let r = Rect::from_two_pos(screen(x0, y0), screen(x1, y1));
    if r != img {
        let shade = theme::faded(Color32::BLACK, 130);
        for part in [
            Rect::from_min_max(img.min, pos2(img.right(), r.top())),
            Rect::from_min_max(pos2(img.left(), r.bottom()), img.max),
            Rect::from_min_max(pos2(img.left(), r.top()), pos2(r.left(), r.bottom())),
            Rect::from_min_max(pos2(r.right(), r.top()), pos2(img.right(), r.bottom())),
        ] {
            painter.rect_filled(part, 0.0, shade);
        }
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
    }
    let axis = o.axis();
    let n = app.settings.finder.strips.clamp(1, 64);
    let edge = Stroke::new(1.0, theme::faded(theme::TEXT_DIM, 50));
    for i in 1..n {
        let t = i as f64 / n as f64;
        let seg = match axis {
            LineAxis::Vertical => [screen(x0, y0 + t * (y1 - y0)), screen(x1, y0 + t * (y1 - y0))],
            LineAxis::Horizontal => [screen(x0 + t * (x1 - x0), y0), screen(x0 + t * (x1 - x0), y1)],
        };
        painter.line_segment(seg, edge);
    }

    // The line found, and its width at half height.
    if let Some(Ok(fix)) = &e.live.fix {
        let color = if fix.warnings.any() { theme::ACCENT } else { theme::OK };
        let clip = painter.with_clip_rect(r);
        for (offset, stroke) in [
            (0.0, Stroke::new(1.5, color)),
            (-fix.fwhm_px / 2.0, Stroke::new(1.0, theme::faded(color, 110))),
            (fix.fwhm_px / 2.0, Stroke::new(1.0, theme::faded(color, 110))),
        ] {
            let ends = match axis {
                LineAxis::Vertical => {
                    let mid = roi.y as f64 + roi.height as f64 / 2.0;
                    let x = |y: f64| fix.position_px + offset + fix.tilt * (y - mid) + 0.5;
                    let (y0, y1) = (roi.y as f64, (roi.y + roi.height) as f64);
                    [screen(x(y0), y0), screen(x(y1), y1)]
                }
                LineAxis::Horizontal => {
                    let mid = roi.x as f64 + roi.width as f64 / 2.0;
                    let y = |x: f64| fix.position_px + offset + fix.tilt * (x - mid) + 0.5;
                    let (x0, x1) = (roi.x as f64, (roi.x + roi.width) as f64);
                    [screen(x0, y(x0)), screen(x1, y(x1))]
                }
            };
            clip.line_segment(ends, stroke);
        }
    }

    // Draw a region by dragging; a double click searches the whole frame.
    let drag_id = resp.id.with("roi_start");
    if resp.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin())
    {
        ui.data_mut(|m| m.insert_temp(drag_id, p));
    }
    let start: Option<Pos2> = ui.data(|m| m.get_temp(drag_id));
    if let (Some(a), Some(b)) = (start, resp.interact_pointer_pos()) {
        let sel = Rect::from_two_pos(a, b).intersect(img);
        if resp.dragged() {
            painter.rect_filled(sel, 0.0, theme::faded(theme::ACCENT, 30));
            painter.rect_stroke(sel, 0.0, Stroke::new(1.5, theme::ACCENT), egui::StrokeKind::Inside);
        }
        if resp.drag_stopped() {
            let (ax, ay) = frame_at(sel.min);
            let (bx, by) = frame_at(sel.max);
            let (x0, x1, y0, y1) = (ax.min(bx), ax.max(bx), ay.min(by), ay.max(by));
            let roi = Roi {
                x: x0.max(0.0) as usize,
                y: y0.max(0.0) as usize,
                width: (x1 - x0).max(0.0) as usize,
                height: (y1 - y0).max(0.0) as usize,
            };
            if roi.width >= 16 && roi.height >= 16 {
                app.settings.finder.roi = roi.clamped(d.width, d.height);
            }
            ui.data_mut(|m| m.remove::<Pos2>(drag_id));
        }
    }
    if resp.double_clicked() {
        app.settings.finder.roi = None;
    }

    let font = FontId::proportional(11.0);
    let hint = painter.layout(
        "Drag to search part of the frame · double-click for all of it".into(),
        font.clone(),
        theme::faded(theme::TEXT, 170),
        (outer.width() - 16.0).max(40.0),
    );
    painter.galley(outer.left_top() + vec2(8.0, 6.0), hint, theme::TEXT);
    if let Some(p) = resp.hover_pos().filter(|p| img.contains(*p)) {
        let (x, y) = frame_at(p);
        let (x, y) = (x.clamp(0.0, fw - 1.0), y.clamp(0.0, fh - 1.0));
        let f = d.preview.factor.max(1);
        let (px, py) = ((x as usize / f).min(d.preview.width - 1), (y as usize / f).min(d.preview.height - 1));
        let grey = d.preview.data.get(py * d.preview.width + px).copied().unwrap_or(0);
        painter.text(
            outer.left_bottom() + vec2(8.0, -6.0),
            Align2::LEFT_BOTTOM,
            format!("x {x:.0}  y {y:.0}  grey {grey}"),
            font,
            theme::TEXT,
        );
    }
    resp.on_hover_text(
        "The camera's picture, turned as Orientation says.\n\
         Solid line: the beam's centre found in this frame. Green when all is well, orange when a warning is up. The faint lines either side mark its width at half height.\n\
         Orange box: the part of the frame searched; the shaded part is left out. The faint lines across it cut it into strips.\n\
         Drag to search part of the frame; double-click for all of it. The bottom-left corner reads the pixel under the pointer (grey 0 to 255).",
    );
    Some(Placed { img, k })
}

const PROFILE: &str = "The mean grey level (0 to 255) of each pixel across the line, over the searched part of the frame.\n\
    White curve: the profile. Orange band: the fit window the centre is worked out in.\n\
    Blue bands: where the background is fitted; the dashed blue line is that background.\n\
    Red line and dot: the centre and the peak. White bar: the width at half height (FWHM).\n\
    Red dashed line: the clip level. A beam that reaches it has a flat top.";

/// The profile across the line on its side, beside an upright view. Px across the line run down
/// the page the way the view shows them, so the beam's peak sits level with the line in the view.
fn profile_beside(app: &App, ui: &mut egui::Ui, e: &Engine, size: egui::Vec2, placed: Option<Placed>) {
    let (resp, painter) = ui.allocate_painter(size, Sense::hover());
    let outer = resp.rect;
    resp.on_hover_text(format!(
        "{PROFILE}\nOn its side here: grey level runs left to right, and each pixel sits level with the same place in the view."
    ));
    painter.rect_filled(outer, 6.0, theme::CANVAS);
    painter.rect_stroke(outer, 6.0, Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside);
    let font = FontId::proportional(11.0);
    painter.text(
        outer.left_top() + vec2(8.0, 4.0),
        Align2::LEFT_TOP,
        "Profile across the line",
        FontId::proportional(13.0),
        theme::ACCENT,
    );
    let (Some(d), Some(pl)) = (&e.live.detail, placed) else { return };
    let ins = &d.inspection;
    let n = ins.profile.len();
    if n < 2 {
        return;
    }
    let o = app.settings.orientation;
    let (fw, fh) = (d.width as f64, d.height as f64);
    let axis = o.axis();
    // Screen y of a px across the line, the centre of px i at i + 0.5, as the view has it.
    let sy = |px: f64| {
        let v = match axis {
            LineAxis::Vertical => o.to_view(px + 0.5, 0.0, fw, fh).1,
            LineAxis::Horizontal => o.to_view(0.0, px + 0.5, fw, fh).1,
        };
        pl.img.top() + v as f32 * pl.k
    };
    let off = ins.offset_px as f64;
    let plot = Rect::from_min_max(pos2(outer.left() + 10.0, pl.img.top()), pos2(outer.right() - 12.0, pl.img.bottom()));
    let clip_level = f64::from(app.settings.finder.clip_level);
    let top = ins.profile.iter().copied().fold(0.0, f64::max);
    let g_max = if top > clip_level * 0.8 { 262.0 } else { (top * 1.12).max(16.0) };
    let sx = |g: f64| plot.left() + (g / g_max) as f32 * plot.width();
    let clipped = painter.with_clip_rect(plot);
    let band = |a: f64, b: f64, fill: Color32| {
        let (ya, yb) = (sy(a), sy(b));
        clipped.rect_filled(Rect::from_x_y_ranges(plot.x_range(), ya.min(yb)..=ya.max(yb)), 0.0, fill);
    };
    let zones = ins.zones.ok();
    if let Some(z) = zones {
        let blue = theme::faded(theme::BLUE, 40);
        band(off - 0.5, off + z.background_left as f64 - 0.5, blue);
        band(off + z.background_right as f64 - 0.5, off + n as f64 - 0.5, blue);
        band(off + z.centre - z.half_width, off + z.centre + z.half_width, theme::faded(theme::ACCENT, 45));
    }
    let grid = Stroke::new(1.0, theme::faded(theme::LINE, 160));
    for g in crate::plot::ticks([0.0, g_max], (plot.width() / 70.0).max(2.0) as usize) {
        let x = sx(g);
        painter.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], grid);
        painter.text(
            pos2(x, outer.bottom() - 4.0),
            Align2::CENTER_BOTTOM,
            format!("{g:.0}"),
            font.clone(),
            theme::TEXT_DIM,
        );
    }
    if g_max >= clip_level {
        let x = sx(clip_level);
        clipped.extend(egui::Shape::dashed_line(
            &[pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(1.0, theme::ERROR),
            6.0,
            4.0,
        ));
    }
    if let Some(z) = zones {
        let bg = |i: f64| z.background.0 + z.background.1 * i;
        let ends = [pos2(sx(bg(0.0)), sy(off)), pos2(sx(bg(n as f64 - 1.0)), sy(off + n as f64 - 1.0))];
        clipped.extend(egui::Shape::dashed_line(&ends, Stroke::new(1.0, theme::BLUE), 6.0, 4.0));
        let c = sy(off + z.centre);
        clipped.line_segment([pos2(plot.left(), c), pos2(plot.right(), c)], Stroke::new(1.0, LASER));
        let half = sx(bg(z.centre) + z.peak / 2.0);
        let (a, b) = (sy(off + z.centre - z.fwhm / 2.0), sy(off + z.centre + z.fwhm / 2.0));
        clipped.line_segment([pos2(half, a), pos2(half, b)], Stroke::new(2.0, theme::TEXT));
        painter.text(
            pos2(half + 4.0, c),
            Align2::LEFT_BOTTOM,
            format!("FWHM {:.1} px", z.fwhm),
            font.clone(),
            theme::TEXT,
        );
    }
    let pts: Vec<Pos2> = ins.profile.iter().enumerate().map(|(i, v)| pos2(sx(*v), sy(off + i as f64))).collect();
    clipped.add(egui::Shape::line(pts, Stroke::new(1.6, theme::TEXT)));
    if let Some(z) = zones {
        clipped.circle_filled(
            pos2(sx(z.background.0 + z.background.1 * z.centre + z.peak), sy(off + z.centre)),
            3.5,
            LASER,
        );
    }
}

fn profile(app: &mut App, ui: &mut egui::Ui, e: &Engine, height: f32) {
    let Some(d) = &e.live.detail else { return };
    let ins = &d.inspection;
    let n = ins.profile.len();
    if n < 2 {
        return;
    }
    let off = ins.offset_px as f64;
    let zoom_id = ui.scope_id().with("profile_zoom");
    let mut zoom: bool = ui.data(|m| m.get_temp(zoom_id)).unwrap_or(true);
    let top = ui.cursor().top();
    ui.horizontal_wrapped(|ui| {
        let legend = format!("{PROFILE}\nHover over the graph to read a pixel in its top-right corner.");
        ui.label(theme::heading("Profile across the line")).on_hover_text(&legend);
        ui.selectable_value(&mut zoom, true, "Around the beam")
            .on_hover_text("Show the beam and the background zones beside it.");
        ui.selectable_value(&mut zoom, false, "Whole width").on_hover_text("Show every pixel across the line.");
        ui.label(theme::dim("mean grey of each px across the line")).on_hover_text(&legend);
    });
    ui.data_mut(|m| m.insert_temp(zoom_id, zoom));
    let height = (height - (ui.cursor().top() - top)).max(100.0);

    let zones = ins.zones.ok();
    let mut x = [off - 0.5, off + n as f64 - 0.5];
    if let (true, Some(z)) = (zoom, zones) {
        let reach =
            (z.centre - z.background_left as f64).max(z.background_right as f64 - z.centre).max(3.0 * z.fwhm) * 1.4;
        x = [(off + z.centre - reach).max(x[0]), (off + z.centre + reach).min(x[1])];
    }
    let (i0, i1) = (((x[0] - off).floor().max(0.0)) as usize, ((x[1] - off).ceil() as usize + 1).min(n));
    let top = ins.profile[i0..i1].iter().copied().fold(0.0, f64::max);
    let clip = f64::from(app.settings.finder.clip_level);
    let y = [0.0, if top > clip * 0.8 { 262.0 } else { (top * 1.12).max(16.0) }];
    let (ax, painter, resp) = Axes::allocate(ui, height, x, y);

    if let Some(z) = zones {
        let blue = theme::faded(theme::BLUE, 40);
        ax.xband(&painter, off - 0.5, off + z.background_left as f64 - 0.5, blue);
        ax.xband(&painter, off + z.background_right as f64 - 0.5, off + n as f64 - 0.5, blue);
        ax.xband(
            &painter,
            off + z.centre - z.half_width,
            off + z.centre + z.half_width,
            theme::faded(theme::ACCENT, 45),
        );
    }
    ax.grid(&painter, |v| format!("{v:.0}"), |v| format!("{v:.0}"));
    if y[1] >= clip {
        ax.dashed(&painter, [x[0], clip], [x[1], clip], Stroke::new(1.0, theme::ERROR));
        ax.label(&painter, [x[0], clip], Align2::LEFT_BOTTOM, " clip", theme::ERROR);
    }
    if let Some(z) = zones {
        let bg = |i: f64| z.background.0 + z.background.1 * i;
        ax.dashed(&painter, [off, bg(0.0)], [off + n as f64 - 1.0, bg(n as f64 - 1.0)], Stroke::new(1.0, theme::BLUE));
        ax.vline(&painter, off + z.centre, Stroke::new(1.0, LASER));
        let half = bg(z.centre) + z.peak / 2.0;
        ax.line(
            &painter,
            [[off + z.centre - z.fwhm / 2.0, half], [off + z.centre + z.fwhm / 2.0, half]],
            Stroke::new(2.0, theme::TEXT),
        );
        ax.label(
            &painter,
            [off + z.centre + z.fwhm / 2.0, half],
            Align2::LEFT_CENTER,
            format!("  FWHM {:.1} px", z.fwhm),
            theme::TEXT,
        );
        if !z.centroid_fits {
            ax.label(
                &painter,
                [x[0], y[1]],
                Align2::LEFT_TOP,
                " fit window off the frame: centre from the half-height crossings",
                theme::ACCENT,
            );
        }
    }
    let pts = ins.profile[i0..i1].iter().enumerate().map(|(i, v)| [off + (i0 + i) as f64, *v]);
    ax.line(&painter, pts, Stroke::new(1.6, theme::TEXT));
    if let Some(z) = zones {
        ax.dots(&painter, [[off + z.centre, z.background.0 + z.background.1 * z.centre + z.peak]], 3.5, LASER);
    }
    if let Some(p) = resp.hover_pos().filter(|p| ax.rect.contains(*p)) {
        let px = ax.data_x(p.x).round();
        if let Some(v) = ins.profile.get((px - off).max(0.0) as usize) {
            ax.vline(&painter, px, Stroke::new(1.0, theme::faded(theme::TEXT, 80)));
            painter.text(
                ax.rect.right_top() + vec2(-4.0, 4.0),
                Align2::RIGHT_TOP,
                format!("px {px:.0}  grey {v:.1}"),
                FontId::proportional(11.0),
                theme::TEXT,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui;

    /// Every symbol the window shows outside plain ASCII, which the built-in fonts must draw. They
    /// have no arrows or shapes, so the turn and flip buttons and the status dot are painted.
    const SYMBOLS: &str = "\u{b0}\u{b1}\u{b5}\u{b7}\u{d7}\u{3c3}\u{2013}\u{2026}\u{2212}";

    #[test]
    fn every_symbol_the_window_shows_is_in_the_font() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        for font in [egui::FontId::proportional(14.0), egui::FontId::monospace(14.0)] {
            let missing: Vec<String> = SYMBOLS
                .chars()
                .filter(|c| !ctx.fonts_mut(|f| f.has_glyph(&font, *c)))
                .map(|c| format!("U+{:04X}", c as u32))
                .collect();
            assert!(missing.is_empty(), "{:?} lacks {missing:?}", font.family);
        }
    }

    /// Lists the window's strings and fails on a symbol outside plain ASCII that is not in SYMBOLS,
    /// so a new one is checked against the font before it ships as a box.
    #[test]
    fn no_unchecked_symbols_in_the_window_text() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        let mut found = Vec::new();
        for entry in std::fs::read_dir(dir).expect("src dir") {
            let path = entry.expect("entry").path();
            let text = std::fs::read_to_string(&path).expect("source");
            let code = text.split("#[cfg(test)]").next().unwrap_or_default();
            for (n, line) in code.lines().enumerate() {
                let line = line.split("//").next().unwrap_or_default();
                for c in line.chars().filter(|c| !c.is_ascii() && !SYMBOLS.contains(*c)) {
                    found.push(format!("{}:{} U+{:04X}", path.display(), n + 1, c as u32));
                }
            }
        }
        assert!(found.is_empty(), "check these against the font: {found:?}");
    }
}
