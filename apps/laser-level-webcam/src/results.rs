//! Results: how straight or flat the surface is, where it is high and low, and what to take off or
//! put under to fix it.

use eframe::egui::{self, Align2, Color32, RichText, Stroke};
use laser_core::results::{Point, Reference, Results};
use laser_core::survey::{DriftCorrection, Layout};
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
                ui.label(theme::heading(format!("No results for {} yet", run.name)))
                    .on_hover_text("Results need the zero station and at least one other station measured.");
                ui.label(theme::dim(format!("{why}."))).on_hover_text("What is missing before results can be worked out.");
                ui.add_space(8.0);
                if ui.add(theme::primary("Measure")).on_hover_text("Go to Measure to take the readings this run still needs.").clicked() {
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
        ui.label("Show").on_hover_text("Which heights the plot and the colours show.");
        for v in View::ALL {
            ui.selectable_value(&mut view, v, v.label()).on_hover_text(v.help());
        }
        ui.separator();
        ui.label("Units").on_hover_text("The unit heights are shown in, here and in the CSV.");
        for u in Unit::ALL {
            let name = match u {
                Unit::Um => "micrometres (µm), a thousandth of a millimetre",
                Unit::Mm => "millimetres (mm)",
                Unit::Inch => "inches (in)",
            };
            ui.selectable_value(&mut app.settings.unit, u, u.label()).on_hover_text(format!("Show heights in {name}."));
        }
        ui.separator();
        ui.label("Drift").on_hover_text(
            "How the zero visits take the drift out. Interpolate when the drift is smooth; use the last zero when the visits are far apart.",
        );
        ui.selectable_value(&mut app.settings.drift_correction, DriftCorrection::Interpolate, "Interpolate")
            .on_hover_text("The zero for each reading is interpolated in time between the zero visits around it.");
        ui.selectable_value(&mut app.settings.drift_correction, DriftCorrection::LastZero, "Last zero")
            .on_hover_text("Each reading against the last zero visit before it.");
    });
    ui.label(theme::dim(view.help())).on_hover_text("What the heights in the plot below are measured from.");
    ui.data_mut(|m| m.insert_temp(view_id, view));
    let plot_h = (ui.available_height() * 0.5).clamp(200.0, 420.0);
    match results.layout {
        Layout::Line => line_plot(ui, unit, &results, view, plot_h),
        Layout::Grid => heat_map(ui, unit, &results, view, plot_h),
    }
    ui.add_space(6.0);
    export_bar(ui, unit, &run.name, &results, e.session.drift_correction);
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
        let pv_help = format!(
            "{what}: the height between the highest and the lowest station, after the laser's tilt is taken out (the Shape view). Smaller is {}.",
            if r.layout == Layout::Line { "straighter" } else { "flatter" }
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{what} of {name}")).size(16.0).color(theme::TEXT_DIM))
                .on_hover_text(pv_help.as_str());
            ui.label(RichText::new(with_unit(unit, r.pv_um)).size(26.0).strong().color(LASER))
                .on_hover_text(pv_help.as_str());
            ui.label(theme::dim("peak to valley"))
                .on_hover_text("Peak to valley: highest point minus lowest point. It is the usual number for straightness and flatness.");
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
        }))
        .on_hover_text(
            "Noise: the peak to valley that random reading noise alone would give with this many stations. A result near it means the surface is better than this set-up can show. Scale: the part left open by the uncertainty of the µm-per-pixel scale.",
        );
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
                theme::card_chip(ui, format!("{flagged} station(s) with warnings"), theme::ACCENT).on_hover_text(
                    "Stations whose readings raised a check. They are orange in the plot; the Checks column in the table says why. Measure them again to clear it.",
                );
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
    let shape = match view {
        View::Scrape => "Red bars: how much to take off each station.".to_owned(),
        View::Shim => "Blue bars: how much shim to put under each station.".to_owned(),
        View::Measured => "Red dots and line: each station's height against the zero station. Dashed blue line: the straight line that fits them best; its slope is the laser's tilt.".to_owned(),
        View::Flattened => "Red dots and line: each station's height against the best-fit straight line. Zero is on the line.".to_owned(),
    };
    let resp = resp.on_hover_text(format!(
        "{shape}\nAcross: distance along the line, mm. Up: height, {}.\nWhite bars: ±1 standard error, how sure each station's height is. Orange dots: stations with warnings.\nMove the mouse over the plot to read the nearest station at the top right.",
        unit.label()
    ));
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
    let (ax, painter, resp) = Axes::allocate(ui, height, x, y);
    ax.grid(&painter, |v| format!("{v:.0} mm"), |v| format!("{v:.0} mm"));
    let max = r.points.iter().map(|p| view.value(p).abs()).fold(0.0, f64::max).max(1e-9);
    let signed = matches!(view, View::Flattened | View::Measured);
    let colours = match view {
        View::Scrape => "Grey: nothing to take off; red: the most to take off.",
        View::Shim => "Grey: no shim needed; blue: the most shim.",
        View::Flattened | View::Measured => "Blue: low; grey: zero; red: high.",
    };
    resp.on_hover_text(format!(
        "The surface seen from above. Each square is a station, at its place along and across in mm.\nThe number in it is its height in {}. {colours}\nOrange border: a station with warnings.",
        unit.label()
    ));
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
    ui.label(theme::dim(legend))
        .on_hover_text("The height that gets the strongest colour. Squares in between get a mix toward grey.");
}

/// What the points are copied or saved as.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Format {
    #[default]
    Csv,
    Json,
}

impl Format {
    const ALL: [Format; 2] = [Format::Csv, Format::Json];

    fn label(self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::Json => "JSON",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Self::Csv => "A table for a spreadsheet, in the unit shown, one row per station.",
            Self::Json => {
                "Everything in the result for a script or another program: the stations, the fitted line or plane, straightness or flatness, drift and repeatability. Heights in µm, positions in mm."
            }
        }
    }

    fn ext(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }

    fn mime(self) -> &'static str {
        match self {
            Self::Csv => "text/csv",
            Self::Json => "application/json",
        }
    }
}

fn export_bar(ui: &mut egui::Ui, unit: Unit, name: &str, r: &Results, drift: DriftCorrection) {
    let msg_id = ui.scope_id().with("export_msg");
    let format_id = ui.scope_id().with("export_format");
    let mut format: Format = ui.data(|m| m.get_temp(format_id)).unwrap_or_default();
    let text = |f: Format| match f {
        Format::Csv => r.csv(unit),
        Format::Json => r.json(name, drift),
    };
    ui.horizontal(|ui| {
        ui.label(theme::heading("Points"))
            .on_hover_text("Copy or save every station's result as a CSV table or as JSON.");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Save…").on_hover_text(format.help()).clicked() {
                let file = format!("{}.{}", files::file_stem(name), format.ext());
                let msg = files::save_text(&file, format.mime(), &text(format));
                ui.data_mut(|m| m.insert_temp(msg_id, msg));
            }
            if ui.button("Copy").on_hover_text("Paste it into a spreadsheet, a script or another program.").clicked() {
                ui.ctx().copy_text(text(format));
                ui.data_mut(|m| m.insert_temp(msg_id, format!("Copied as {}.", format.label())));
            }
            for f in Format::ALL.iter().rev() {
                if ui.selectable_value(&mut format, *f, f.label()).on_hover_text(f.help()).changed() {
                    ui.data_mut(|m| m.remove::<String>(msg_id));
                }
            }
            if let Some(msg) = ui.data(|m| m.get_temp::<String>(msg_id)) {
                ui.label(theme::dim(msg)).on_hover_text("What the last copy or save did.");
            }
        });
    });
    ui.data_mut(|m| m.insert_temp(format_id, format));
}

fn table(ui: &mut egui::Ui, unit: Unit, r: &Results) {
    let grid = r.layout == Layout::Grid;
    let u = unit.label();
    egui::Grid::new("points").striped(true).num_columns(10).spacing([14.0, 4.0]).show(ui, |ui| {
        let heads = [
            ("Station".to_owned(), "The station's name, or its number when it has none."),
            ("Along".to_owned(), "How far along the surface the station is, mm."),
            (
                if grid { "Across".to_owned() } else { String::new() },
                "How far across the surface the station is, mm.",
            ),
            (format!("Measured, {u}"), "Height against the zero station, as the laser saw it."),
            (format!("Shape, {u}"), "Height against the best-fit line or plane: the surface's shape with the laser's tilt taken out."),
            (format!("Scrape, {u}"), "How far above the lowest station: the metal to take off."),
            (format!("Shim, {u}"), "How far below the highest station: the shim to put under."),
            (format!("±, {u}"), "±1 standard error: how sure the station's height is, with the zero's own uncertainty included."),
            ("Visits".to_owned(), "How many times the station was measured. Its height is the average of the visits."),
            ("Checks".to_owned(), "ok, or the checks the station's readings raised. Hover over one to read what it means."),
        ];
        for (h, help) in heads {
            if h.is_empty() {
                ui.label("");
            } else {
                ui.label(theme::dim(h)).on_hover_text(help);
            }
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
                ui.colored_label(theme::OK, "ok").on_hover_text("The station's readings raised no checks.");
            } else {
                let help: Vec<&str> = names.iter().map(|n| crate::measure::flag_help(n)).collect();
                ui.colored_label(theme::ACCENT, names.join(", ")).on_hover_text(help.join("\n"));
            }
            ui.end_row();
        }
    });
}
