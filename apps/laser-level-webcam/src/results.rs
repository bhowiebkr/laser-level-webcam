//! Results: how straight or flat the surface is, where it is high and low, and what to take off or
//! put under to fix it.

use eframe::egui::{self, Align2, Color32, RichText, Stroke};
use laser_core::results::{Point, Reference, Results};
use laser_core::survey::Layout;
use laser_core::units::Unit;

use crate::app::{App, Screen};
use crate::engine::Engine;
use crate::files;
use crate::plot::{self, Axes};
use crate::setup::{stat, with_unit};
use crate::theme;
use crate::visuals::LASER;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum View {
    /// Against the fitted line or plane: the shape of the surface.
    #[default]
    Flattened,
    /// Against the zero station: what the laser saw.
    Measured,
    /// How far below the highest station: what to put under.
    Shim,
    /// How far above the lowest station: what to take off.
    Scrape,
}

impl View {
    const ALL: [View; 4] = [View::Flattened, View::Measured, View::Scrape, View::Shim];

    fn label(self) -> &'static str {
        match self {
            Self::Flattened => "Shape",
            Self::Measured => "As measured",
            Self::Scrape => "Scrape",
            Self::Shim => "Shim",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Self::Flattened => {
                "Heights against the straight line (or flat plane) that fits them best. The laser need not be level or parallel to the surface; this takes its tilt out."
            }
            Self::Measured => {
                "Heights against the zero station, as the laser saw them, with the fitted line or plane dashed."
            }
            Self::Scrape => "How far each station is above the lowest one: the metal to take off to bring it down.",
            Self::Shim => "How far each station is below the highest one: the shim to put under to bring it up.",
        }
    }

    fn value(self, p: &Point) -> f64 {
        match self {
            Self::Flattened => p.flattened_um,
            Self::Measured => p.measured_um,
            Self::Scrape => p.above_min_um,
            Self::Shim => p.below_max_um,
        }
    }
}

pub fn draw(app: &mut App, ui: &mut egui::Ui, e: &mut Engine) {
    let Some(run) = e.current_run() else { return };
    let unit = app.settings.unit;
    let results = match Results::compute(run, &e.session.scale, e.session.drift_correction) {
        Ok(r) => r,
        Err(why) => {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(theme::heading(format!("No results for {} yet", run.name)));
                ui.label(theme::dim(format!("{why}.")));
                ui.add_space(8.0);
                if ui.add(theme::primary("Measure")).clicked() {
                    app.screen = Screen::Measure;
                }
            });
            return;
        }
    };
    let view_id = ui.scope_id().with("view");
    let mut view: View = ui.data(|m| m.get_temp(view_id)).unwrap_or_default();

    summary(ui, unit, &run.name, &results);
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        for v in View::ALL {
            ui.selectable_value(&mut view, v, v.label()).on_hover_text(v.help());
        }
        ui.label(theme::dim(view.help()));
    });
    ui.data_mut(|m| m.insert_temp(view_id, view));
    let plot_h = (ui.available_height() * 0.5).clamp(200.0, 420.0);
    match results.layout {
        Layout::Line => line_plot(ui, unit, &results, view, plot_h),
        Layout::Grid => heat_map(ui, unit, &results, view, plot_h),
    }
    ui.add_space(6.0);
    export_bar(ui, unit, &run.name, &results);
    ui.add_space(4.0);
    theme::scroll(ui, |ui| table(ui, unit, &results));
}

fn summary(ui: &mut egui::Ui, unit: Unit, name: &str, r: &Results) {
    let what = match r.layout {
        Layout::Line => "Straightness",
        Layout::Grid => "Flatness",
    };
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{what} of {name}")).size(16.0).color(theme::TEXT_DIM));
            ui.label(RichText::new(with_unit(unit, r.pv_um)).size(26.0).strong().color(LASER));
            ui.label(theme::dim("peak to valley"));
        });
        let within = r.pv_um <= r.pv_noise_um * 1.5;
        ui.label(theme::dim(if within {
            format!(
                "The noise alone would give about {}. The surface is {} to within what this set-up can see; measure more visits per station to see further.",
                with_unit(unit, r.pv_noise_um),
                if r.layout == Layout::Line { "straight" } else { "flat" }
            )
        } else {
            format!(
                "The noise alone would give about {}, so the shape is real. The scale leaves ±{} of it open.",
                with_unit(unit, r.pv_noise_um),
                with_unit(unit, r.pv_scale_um)
            )
        }));
        theme::cards(ui, |ui| {
            stat(ui, "Stations", format!("{}", r.points.len()), theme::TEXT, "Stations with at least one visit.");
            match r.reference {
                Reference::Line { slope_um_per_m, .. } => {
                    stat(ui, "Laser tilt", format!("{} /m", with_unit(unit, slope_um_per_m)), theme::TEXT, "Slope of the fitted line: how far the laser is from parallel to the surface. Taken out of the shape.");
                }
                Reference::Plane { slope_x_um_per_m, slope_y_um_per_m, .. } => {
                    stat(ui, "Laser tilt along", format!("{} /m", with_unit(unit, slope_x_um_per_m)), theme::TEXT, "Slope of the fitted plane along x. Taken out of the shape.");
                    stat(ui, "Laser tilt across", format!("{} /m", with_unit(unit, slope_y_um_per_m)), theme::TEXT, "Slope of the fitted plane along y. Taken out of the shape.");
                }
            }
            if let Some(rep) = r.repeatability {
                stat(
                    ui,
                    "Repeatability",
                    with_unit(unit, rep.pooled_sd_um),
                    if rep.ratio > 3.0 { theme::ACCENT } else { theme::TEXT },
                    "Scatter of repeat visits to one station. Well above the reading uncertainty: the block does not sit the same way twice.",
                );
            }
            if let Some(c) = r.drift.closure_um {
                stat(ui, "Zero closure", with_unit(unit, c), theme::TEXT, "How far the zero moved from its first visit to its last. Taken out of the heights.");
            }
            if !r.drift.closed {
                theme::card_chip(ui, "Run not closed on the zero", theme::ACCENT)
                    .on_hover_text("Visits after the last zero visit have no drift check. Measure the zero once more.");
            }
            let flagged = r.points.iter().filter(|p| p.flags.any()).count();
            if flagged > 0 {
                theme::card_chip(ui, format!("{flagged} station(s) with warnings"), theme::ACCENT);
            }
        });
    });
}

fn y_label(unit: Unit, span: f64) -> impl Fn(f64) -> String {
    let step = span / 5.0;
    move |v| format!("{} {}", plot::tick_text(v, step), if v == 0.0 { unit.label() } else { "" })
}

fn line_plot(ui: &mut egui::Ui, unit: Unit, r: &Results, view: View, height: f32) {
    let mut pts: Vec<&Point> = r.points.iter().collect();
    pts.sort_by(|a, b| a.x_mm.total_cmp(&b.x_mm));
    let x = plot::span(pts.iter().map(|p| p.x_mm), 0.05);
    let mut ys = vec![0.0];
    for p in &pts {
        let v = view.value(p);
        ys.push(unit.from_um(v + p.uncertainty_um));
        ys.push(unit.from_um(v - p.uncertainty_um));
    }
    if view == View::Measured {
        ys.push(unit.from_um(r.reference.at(x[0], 0.0)));
        ys.push(unit.from_um(r.reference.at(x[1], 0.0)));
    }
    let y = plot::span(ys, 0.1);
    let (ax, painter, resp) = Axes::allocate(ui, height, x, y);
    ax.grid(&painter, |v| format!("{v:.0} mm"), y_label(unit, y[1] - y[0]));
    ax.hline(&painter, 0.0, Stroke::new(1.0, theme::faded(theme::TEXT, 70)));
    let bars = matches!(view, View::Shim | View::Scrape);
    if view == View::Measured {
        ax.dashed(
            &painter,
            [x[0], unit.from_um(r.reference.at(x[0], 0.0))],
            [x[1], unit.from_um(r.reference.at(x[1], 0.0))],
            Stroke::new(1.0, theme::BLUE),
        );
        ax.label(
            &painter,
            [x[1], unit.from_um(r.reference.at(x[1], 0.0))],
            Align2::RIGHT_BOTTOM,
            "fitted line ",
            theme::BLUE,
        );
    }
    if bars {
        let w = pts.windows(2).map(|p| ax.sx(p[1].x_mm) - ax.sx(p[0].x_mm)).fold(f32::INFINITY, f32::min);
        let half = if w.is_finite() { (w * 0.3).clamp(2.0, 18.0) } else { 10.0 };
        for p in &pts {
            let top = ax.pos(p.x_mm, unit.from_um(view.value(p)));
            let base = ax.pos(p.x_mm, 0.0);
            let rect = egui::Rect::from_two_pos(egui::pos2(top.x - half, top.y), egui::pos2(base.x + half, base.y));
            painter.rect_filled(rect, 1.0, theme::faded(if view == View::Scrape { LASER } else { theme::BLUE }, 150));
        }
    } else {
        ax.line(
            &painter,
            pts.iter().map(|p| [p.x_mm, unit.from_um(view.value(p))]),
            Stroke::new(1.5, theme::faded(LASER, 170)),
        );
    }
    for p in &pts {
        let v = view.value(p);
        painter.line_segment(
            [ax.pos(p.x_mm, unit.from_um(v - p.uncertainty_um)), ax.pos(p.x_mm, unit.from_um(v + p.uncertainty_um))],
            Stroke::new(1.5, theme::TEXT),
        );
        let color = if p.flags.any() { theme::ACCENT } else { LASER };
        painter.circle_filled(ax.pos(p.x_mm, unit.from_um(v)), 3.5, color);
    }
    if let Some(m) = resp.hover_pos().filter(|m| ax.rect.contains(*m))
        && let Some(p) = pts.iter().min_by(|a, b| (ax.sx(a.x_mm) - m.x).abs().total_cmp(&(ax.sx(b.x_mm) - m.x).abs()))
    {
        let name = if p.label.is_empty() { format!("{}", p.station + 1) } else { p.label.clone() };
        painter.text(
            ax.rect.right_top() + egui::vec2(-4.0, 4.0),
            Align2::RIGHT_TOP,
            format!(
                "station {name} at {:.0} mm: {} ± {}",
                p.x_mm,
                with_unit(unit, view.value(p)),
                unit.format(p.uncertainty_um)
            ),
            egui::FontId::proportional(11.0),
            theme::TEXT,
        );
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let c = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(c(a.r(), b.r()), c(a.g(), b.g()), c(a.b(), b.b()))
}

/// Smallest gap between distinct values, or `fallback`.
fn min_gap(values: impl Iterator<Item = f64>, fallback: f64) -> f64 {
    let mut v: Vec<f64> = values.collect();
    v.sort_by(f64::total_cmp);
    let gap = v.windows(2).map(|w| w[1] - w[0]).filter(|d| *d > 1e-6).fold(f64::INFINITY, f64::min);
    if gap.is_finite() { gap } else { fallback }
}

fn heat_map(ui: &mut egui::Ui, unit: Unit, r: &Results, view: View, height: f32) {
    let dx = min_gap(r.points.iter().map(|p| p.x_mm), 100.0);
    let dy = min_gap(r.points.iter().map(|p| p.y_mm), 100.0);
    let x = plot::span(r.points.iter().flat_map(|p| [p.x_mm - dx / 2.0, p.x_mm + dx / 2.0]), 0.02);
    let y = plot::span(r.points.iter().flat_map(|p| [p.y_mm - dy / 2.0, p.y_mm + dy / 2.0]), 0.02);
    let (ax, painter, _) = Axes::allocate(ui, height, x, y);
    ax.grid(&painter, |v| format!("{v:.0} mm"), |v| format!("{v:.0} mm"));
    let max = r.points.iter().map(|p| view.value(p).abs()).fold(0.0, f64::max).max(1e-9);
    let signed = matches!(view, View::Flattened | View::Measured);
    for p in &r.points {
        let v = view.value(p);
        let t = (v / max) as f32;
        let fill = if !signed {
            mix(theme::NODE, if view == View::Scrape { LASER } else { theme::BLUE }, t)
        } else if t >= 0.0 {
            mix(theme::NODE, LASER, t)
        } else {
            mix(theme::NODE, theme::BLUE, -t)
        };
        let rect = egui::Rect::from_two_pos(
            ax.pos(p.x_mm - dx * 0.47, p.y_mm - dy * 0.47),
            ax.pos(p.x_mm + dx * 0.47, p.y_mm + dy * 0.47),
        );
        painter.rect_filled(rect, 3.0, fill);
        if p.flags.any() {
            painter.rect_stroke(rect, 3.0, Stroke::new(1.5, theme::ACCENT), egui::StrokeKind::Inside);
        }
        painter.text(rect.center(), Align2::CENTER_CENTER, unit.format(v), egui::FontId::monospace(12.0), theme::TEXT);
    }
    let legend = if signed {
        format!("blue low, red high; ±{} at full colour", with_unit(unit, max))
    } else {
        format!("{} at full colour", with_unit(unit, max))
    };
    ui.label(theme::dim(legend));
}

fn export_bar(ui: &mut egui::Ui, unit: Unit, name: &str, r: &Results) {
    let msg_id = ui.scope_id().with("export_msg");
    ui.horizontal(|ui| {
        ui.label(theme::heading("Points"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Copy as CSV").on_hover_text("For a spreadsheet: paste it in.").clicked() {
                ui.ctx().copy_text(r.csv(unit));
                ui.data_mut(|m| m.insert_temp(msg_id, "Copied.".to_owned()));
            }
            if ui.button("Save CSV…").clicked() {
                let msg = files::save_text(&format!("{}.csv", files::file_stem(name)), "text/csv", &r.csv(unit));
                ui.data_mut(|m| m.insert_temp(msg_id, msg));
            }
            if let Some(msg) = ui.data(|m| m.get_temp::<String>(msg_id)) {
                ui.label(theme::dim(msg));
            }
        });
    });
}

fn table(ui: &mut egui::Ui, unit: Unit, r: &Results) {
    let grid = r.layout == Layout::Grid;
    let u = unit.label();
    egui::Grid::new("points").striped(true).num_columns(10).spacing([14.0, 4.0]).show(ui, |ui| {
        let heads = [
            "Station".to_owned(),
            "Along".to_owned(),
            if grid { "Across".to_owned() } else { String::new() },
            format!("Measured, {u}"),
            format!("Shape, {u}"),
            format!("Scrape, {u}"),
            format!("Shim, {u}"),
            format!("±, {u}"),
            "Visits".to_owned(),
            "Checks".to_owned(),
        ];
        for h in heads {
            ui.label(theme::dim(h));
        }
        ui.end_row();
        for p in &r.points {
            ui.label(if p.label.is_empty() { format!("{}", p.station + 1) } else { p.label.clone() });
            ui.label(format!("{:.0} mm", p.x_mm));
            ui.label(if grid { format!("{:.0} mm", p.y_mm) } else { String::new() });
            for v in [p.measured_um, p.flattened_um, p.above_min_um, p.below_max_um, p.uncertainty_um] {
                ui.label(unit.format(v));
            }
            ui.label(format!("{}", p.visits));
            let names: Vec<&str> = p.flags.names().collect();
            if names.is_empty() {
                ui.colored_label(theme::OK, "ok");
            } else {
                ui.colored_label(theme::ACCENT, names.join(", "));
            }
            ui.end_row();
        }
    });
}
