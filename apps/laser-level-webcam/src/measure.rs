//! Measuring. The block with the camera goes on each station in turn and a sample is taken at
//! each; the zero station is visited first, again now and then, and last, so the drift between
//! zero visits can be taken out of the heights.

use eframe::egui::{self, Align2, RichText, Stroke};
use laser_core::monitor::Readiness;
use laser_core::reading::SampleRule;
use laser_core::survey::{Heights, Layout, Run, Station};
use laser_core::units::Unit;

use crate::app::{App, Screen};
use crate::engine::{CameraState, Engine};
use crate::plot::{self, Axes, clock};
use crate::setup::{stat, with_unit};
use crate::theme;
use crate::visuals::LASER;

/// The station picked by hand, until the next visit is recorded.
#[derive(Clone, Copy)]
struct Pick {
    run: usize,
    station: usize,
    visits: usize,
}

#[derive(Clone)]
struct NewRun {
    grid: bool,
    count: usize,
    spacing_mm: f64,
    nx: usize,
    ny: usize,
    dx_mm: f64,
    dy_mm: f64,
}

impl Default for NewRun {
    fn default() -> Self {
        Self { grid: false, count: 5, spacing_mm: 100.0, nx: 3, ny: 3, dx_mm: 100.0, dy_mm: 100.0 }
    }
}

/// Changes asked for while drawing, made once the drawing is done.
enum Action {
    Pick(usize),
    Take(usize, Option<usize>),
    Keep,
    Cancel,
    Delete(usize),
    SetZero(usize),
    Edit(usize, Station),
    AddStation,
    RemoveStation(usize),
    NewRun(Run),
    SelectRun(usize),
    Rename(String),
}

pub fn draw(app: &mut App, ui: &mut egui::Ui, e: &mut Engine) {
    let mut actions = Vec::new();
    let pick_id = ui.scope_id().with("pick");
    run_bar(app, ui, e, &mut actions);
    if let Some(run) = e.current_run() {
        let pick: Option<Pick> = ui.data(|m| m.get_temp(pick_id));
        let station = pick
            .filter(|p| p.run == e.run && p.visits == run.visits.len() && p.station < run.stations.len())
            .map_or_else(|| run.next_station(), |p| p.station);
        let heights = run.heights(&e.run_scale(run), e.session.drift_correction);
        ui.add_space(6.0);
        sample_card(app, ui, e, run, station, &mut actions);
        ui.add_space(6.0);
        let plot_h = (ui.available_height() * 0.42).clamp(180.0, 360.0);
        match run.layout {
            Layout::Line => line_plot(app, ui, run, heights.as_ref().ok(), station, plot_h, &mut actions),
            Layout::Grid => grid_plot(app, ui, run, heights.as_ref().ok(), station, plot_h, &mut actions),
        }
        if let Err(why) = &heights {
            ui.label(theme::dim(format!("Heights: {why}.")))
                .on_hover_text("Why the plot has no heights yet. Heights are worked out against the zero station, so it needs a sample first.");
        }
        ui.add_space(6.0);
        let heights = heights.as_ref().ok();
        let busy = e.sampling.is_some();
        // Wide: the visits on the left, the run's figures and its stations beside them. Narrow:
        // one under the other.
        let w = ui.available_width();
        if w >= 900.0 {
            let side_w = (w * 0.34).clamp(340.0, 560.0);
            let gap = 12.0;
            let h = ui.available_height();
            let down = egui::Layout::top_down(egui::Align::Min);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.allocate_ui_with_layout(egui::vec2(w - side_w - gap, h), down, |ui| {
                    ui.push_id("visits", |ui| {
                        theme::scroll(ui, |ui| visits_table(app, ui, e, run, heights, &mut actions))
                    });
                });
                ui.allocate_ui_with_layout(egui::vec2(side_w, h), down, |ui| {
                    ui.push_id("run", |ui| {
                        theme::scroll(ui, |ui| {
                            run_figures(app, ui, heights);
                            ui.add_space(8.0);
                            stations_editor(ui, run, busy, &mut actions);
                        })
                    });
                });
            });
        } else {
            theme::scroll(ui, |ui| {
                visits_table(app, ui, e, run, heights, &mut actions);
                ui.add_space(8.0);
                run_figures(app, ui, heights);
                ui.add_space(8.0);
                stations_editor(ui, run, busy, &mut actions);
            });
        }
    }
    for a in actions {
        apply(e, a, ui, pick_id);
    }
}

fn apply(e: &mut Engine, a: Action, ui: &egui::Ui, pick_id: egui::Id) {
    let run_index = e.run;
    let visits = e.current_run().map_or(0, |r| r.visits.len());
    let result = match a {
        Action::Pick(station) => {
            ui.data_mut(|m| m.insert_temp(pick_id, Pick { run: run_index, station, visits }));
            Ok(())
        }
        Action::Take(station, replace) => {
            e.take_sample(station, replace);
            Ok(())
        }
        Action::Keep => {
            e.finish_sample();
            Ok(())
        }
        Action::Cancel => {
            e.sampling = None;
            Ok(())
        }
        Action::NewRun(run) => {
            e.session.runs.push(run);
            e.run = e.session.runs.len() - 1;
            Ok(())
        }
        Action::SelectRun(i) => {
            e.run = i;
            Ok(())
        }
        a => match e.session.runs.get_mut(run_index) {
            None => Ok(()),
            Some(run) => match a {
                Action::Delete(v) => run.delete(v).map(|_| ()),
                Action::SetZero(s) => run.set_zero(s),
                Action::Edit(s, station) => {
                    if let Some(slot) = run.stations.get_mut(s) {
                        *slot = station;
                    }
                    Ok(())
                }
                Action::AddStation => {
                    let last = run.stations.last().cloned().unwrap_or_default();
                    let step = match run.stations.len() {
                        0 | 1 => 100.0,
                        n => run.stations[n - 1].x_mm - run.stations[n - 2].x_mm,
                    };
                    run.add_station(Station { x_mm: last.x_mm + step, y_mm: last.y_mm, label: String::new() });
                    Ok(())
                }
                Action::RemoveStation(s) => run.remove_station(s).map(|_| ()),
                Action::Rename(name) => {
                    run.name = name;
                    Ok(())
                }
                _ => Ok(()),
            },
        },
    };
    if let Err(why) = result {
        e.survey_error = Some(why.to_string());
    }
}

fn run_bar(app: &mut App, ui: &mut egui::Ui, e: &Engine, actions: &mut Vec<Action>) {
    let busy = e.sampling.is_some();
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!busy, |ui| {
            let current = e.current_run().map_or("No run", |r| r.name.as_str());
            egui::ComboBox::from_id_salt("run")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (i, r) in e.session.runs.iter().enumerate() {
                        let text = format!("{}  ·  {} stations, {} visits", r.name, r.stations.len(), r.visits.len());
                        if ui.selectable_label(i == e.run, text).on_hover_text("Measure this run.").clicked() {
                            actions.push(Action::SelectRun(i));
                        }
                    }
                })
                .response
                .on_hover_text("The run being measured. A run is one line or surface: its stations and every visit to them.");
            if let Some(run) = e.current_run() {
                let id = ui.scope_id().with(("run_name", e.run));
                let mut name: String = ui.data(|m| m.get_temp(id)).unwrap_or_else(|| run.name.clone());
                let resp = ui
                    .add(egui::TextEdit::singleline(&mut name).desired_width(140.0).hint_text("Run name"))
                    .on_hover_text("The run's name. Type a new one and press Enter or click away to keep it.");
                if resp.changed() {
                    ui.data_mut(|m| m.insert_temp(id, name.clone()));
                }
                if resp.lost_focus() && !name.trim().is_empty() && name != run.name {
                    actions.push(Action::Rename(name.trim().to_owned()));
                }
                if !resp.has_focus() && !resp.changed() {
                    ui.data_mut(|m| m.remove::<String>(id));
                }
            }
            ui.menu_button("New run", |ui| new_run_form(ui, e, actions))
                .response
                .on_hover_text("Lay out a new line or surface to measure. The runs you have stay in the session.");
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("Results")
                .on_hover_text("Go to Results: this run's heights as a profile or a map, with straightness or flatness.")
                .clicked()
            {
                app.screen = Screen::Results;
            }
        });
    });
}

fn new_run_form(ui: &mut egui::Ui, e: &Engine, actions: &mut Vec<Action>) {
    let id = ui.scope_id().with("new_run");
    let mut f: NewRun = ui.data(|m| m.get_temp(id)).unwrap_or_default();
    ui.set_min_width(260.0);
    const STATIONS: &str = "How many places the block is put down. More stations show more of the shape, and take longer.";
    const SPACING: &str = "Distance from one station to the next. Make it about the block's length, so the block's feet land where the last station was.";
    ui.horizontal(|ui| {
        ui.selectable_value(&mut f.grid, false, "Along a line")
            .on_hover_text("Stations in a row: straightness of one line.");
        ui.selectable_value(&mut f.grid, true, "Across a surface")
            .on_hover_text("Stations in a grid: flatness of an area.");
    });
    ui.label(theme::dim(if f.grid {
        "Straightness in two directions: flatness of a table or plate."
    } else {
        "Straightness along one line: a way, a rail, a beam."
    }));
    egui::Grid::new("new_run_grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        if f.grid {
            ui.label("Stations").on_hover_text(STATIONS);
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut f.nx).range(2..=50).suffix(" across"))
                    .on_hover_text(format!("{STATIONS}\nStations across, along the laser line."));
                ui.add(egui::DragValue::new(&mut f.ny).range(2..=50).suffix(" deep"))
                    .on_hover_text(format!("{STATIONS}\nRows deep, away from the laser."));
            });
            ui.end_row();
            ui.label("Spacing").on_hover_text(SPACING);
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut f.dx_mm).range(1.0..=10_000.0).suffix(" mm"))
                    .on_hover_text(format!("{SPACING}\nThis is the spacing across."));
                ui.add(egui::DragValue::new(&mut f.dy_mm).range(1.0..=10_000.0).suffix(" mm"))
                    .on_hover_text(format!("{SPACING}\nThis is the spacing between rows."));
            });
            ui.end_row();
        } else {
            ui.label("Stations").on_hover_text(STATIONS);
            ui.add(egui::DragValue::new(&mut f.count).range(2..=200)).on_hover_text(STATIONS);
            ui.end_row();
            ui.label("Spacing").on_hover_text(SPACING);
            ui.add(egui::DragValue::new(&mut f.spacing_mm).range(1.0..=10_000.0).suffix(" mm"))
                .on_hover_text(SPACING);
            ui.end_row();
        }
    });
    ui.label(theme::dim(
        "Spacing is about the block's length: the block rests on two feet, and each station's height is where the block sits.",
    ));
    if ui
        .add(theme::primary("Create"))
        .on_hover_text("Make the run and start measuring it. The first station is the zero, which every height is measured from.")
        .clicked()
    {
        let n = e.session.runs.len() + 1;
        let run = if f.grid {
            Run::grid(&format!("Surface {n}"), f.nx, f.ny, f.dx_mm, f.dy_mm)
        } else {
            Run::line(&format!("Line {n}"), f.count, f.spacing_mm)
        };
        actions.push(Action::NewRun(run));
        ui.close();
    }
    ui.data_mut(|m| m.insert_temp(id, f));
}

fn station_name(run: &Run, i: usize) -> String {
    let s = &run.stations[i];
    let name = if s.label.is_empty() { format!("{}", i + 1) } else { s.label.clone() };
    if i == run.zero { format!("{name} (zero)") } else { name }
}

fn sample_card(app: &mut App, ui: &mut egui::Ui, e: &Engine, run: &Run, station: usize, actions: &mut Vec<Action>) {
    let unit = app.settings.unit;
    let now = e.frame_time().unwrap_or_else(|| e.now());
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        if let Some(s) = &e.sampling {
            let rule = e.sample_rule;
            let elapsed = (now - s.started_s).max(0.0);
            let verb = if s.replace.is_some() { "Measuring again" } else { "Measuring" };
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{verb} station {}", station_name(run, s.station))).size(18.0).strong().color(LASER));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").on_hover_text("Stop this sample and throw it away.").clicked() {
                        actions.push(Action::Cancel);
                    }
                    let enough = elapsed >= rule.min_s.min(1.0);
                    if ui
                        .add_enabled(enough, theme::primary("Keep it now"))
                        .on_hover_text("End the sample now and keep what it has. A shorter sample is less sure.")
                        .on_disabled_hover_text("A sample needs at least a second of frames.")
                        .clicked()
                    {
                        actions.push(Action::Keep);
                    }
                });
            });
            let progress = (elapsed / rule.max_s.max(1e-9)).min(1.0) as f32;
            ui.add(egui::ProgressBar::new(progress).text(format!("{:.1} s", elapsed)).desired_height(14.0))
                .on_hover_text(format!(
                    "Time into the sample. The bar is full at the longest a sample runs, {:.0} s; it can end sooner.",
                    rule.max_s
                ));
            let mut rule_text = format!("At least {:.0} s, at most {:.0} s", rule.min_s, rule.max_s);
            if let Some(t) = rule.target_um {
                rule_text.push_str(&format!(", or once the reading is good to {}", with_unit(unit, t)));
            }
            ui.label(theme::dim(rule_text + ". Keep hands and breath away from the beam."))
                .on_hover_text("When this sample stops. Warm air and touching the block move the beam, so keep still until it ends.");
            if let Some(r) = e.sample_so_far() {
                let scale = &e.run_scale(run);
                let zero_px = run.visits.iter().rev().find(|v| v.station == run.zero).map(|v| v.reading.px);
                theme::cards(ui, |ui| {
                    if let Some(z) = zero_px.filter(|_| s.station != run.zero) {
                        stat(ui, "Above the zero", with_unit(unit, scale.height_um(r.px, z)), theme::TEXT, "Against the last zero visit, before the drift is taken out.");
                    }
                    let unc = scale.um(r.uncertainty_px).abs();
                    let good = rule.target_um.is_none_or(|t| unc <= t);
                    stat(ui, "Good to ±", with_unit(unit, unc), if good { theme::OK } else { theme::TEXT }, "Standard error of the sample's mean, from the scatter of its one-second parts.");
                    stat(ui, "Scatter", with_unit(unit, scale.um(r.scatter_px).abs()), theme::TEXT, "Standard deviation of the frames about the mean.");
                    stat(ui, "Frames", format!("{} of {}", r.used, r.frames), theme::TEXT, "Frames used, of the frames seen. Frames with no beam or far from the rest are left out.");
                    for name in r.flags.names() {
                        theme::card_chip(ui, name, theme::ACCENT).on_hover_text(flag_help(name));
                    }
                });
            }
            return;
        }

        let open = e.camera.state == CameraState::Open;
        let grid_now = e.grid();
        let conflict = run.grid_conflict(grid_now);
        let can_take = open && conflict.is_none();
        ui.horizontal(|ui| {
            let st = &run.stations[station];
            let at = match run.layout {
                Layout::Line => format!("{:.0} mm", st.x_mm),
                Layout::Grid => format!("{:.0}, {:.0} mm", st.x_mm, st.y_mm),
            };
            ui.label(RichText::new(format!("Station {}", station_name(run, station))).size(18.0).strong())
                .on_hover_text("The station the next sample is for. (zero) marks the station every height is measured from.");
            ui.label(theme::dim(at)).on_hover_text(match run.layout {
                Layout::Line => "Where the station is along the line, from the first station.",
                Layout::Grid => "Where the station is, across and deep, from the first station.",
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let take = ui.add_enabled(can_take, theme::primary("Take sample")).on_hover_text("Space bar");
                let space = ui.input(|i| i.key_pressed(egui::Key::Space)) && ui.memory(|m| m.focused().is_none());
                if can_take && (take.clicked() || space) {
                    actions.push(Action::Take(station, None));
                }
            });
        });
        let next = run.next_station();
        ui.label(theme::dim(if station == next {
            "Put the block on this station, let it sit still, then take the sample. Click a station on the plot to pick another."
        } else {
            "Picked by hand. The plan's next station comes back after this sample."
        }))
        .on_hover_text("What to do at this station. The station marked orange on the plot is where the block goes.");
        sample_rule(ui, &mut app.settings.sample, unit);
        ui.horizontal_wrapped(|ui| {
            if !open {
                theme::chip(ui, "No camera", theme::ERROR)
                    .on_hover_text("No camera is open, so no sample can be taken. Pick one in the side panel.");
            }
            if let Some(was) = conflict {
                theme::chip(ui, "Camera mode or orientation changed", theme::ERROR).on_hover_text(format!(
                    "This run was read at {was}. The camera now gives {grid_now}. A reading in pixels gives the same height only with the same frame size and line direction. Set the camera mode and orientation back, or start a new run."
                ));
            }
            match e.status.as_ref().map(|s| &s.readiness) {
                Some(Readiness::Ready) | None => {}
                Some(Readiness::Settling { in_s, .. }) => {
                    theme::chip(ui, format!("Still settling, ready in about {}", clock(*in_s)), theme::ACCENT)
                        .on_hover_text("Readings taken now drift more than the plan allows. See Wait.");
                }
                Some(Readiness::Collecting { .. }) => {
                    theme::chip(ui, "Drift not known yet", theme::TEXT_DIM)
                        .on_hover_text("The drift monitor is still collecting. Samples can be taken; see Wait for how long it needs.");
                }
                Some(Readiness::Noisy { .. }) => {
                    theme::chip(ui, "Too noisy for the plan", theme::ACCENT).on_hover_text(
                        "The beam is too noisy to tell whether the drift stays inside the allowed drift. See Wait: allow more drift, or steady the set-up.",
                    );
                }
                Some(Readiness::Drifting { .. }) => {
                    theme::chip(ui, "Still drifting", theme::ACCENT).on_hover_text(
                        "The beam still moves more than the plan allows and is not settling. Measure the zero often, or see Wait.",
                    );
                }
            }
            let rate = e.status.as_ref().and_then(|s| s.rate_per_min).map(|r| r / 60.0);
            if let Some(rz) = run.rezero(&e.run_scale(run), now, rate, app.settings.plan.tolerance_um) {
                let text = format!(
                    "{} since the zero, about {} of drift",
                    clock(rz.since_s),
                    with_unit(unit, rz.predicted_um)
                );
                let from = if rz.from_run { "the last two zero visits" } else { "the drift monitor" };
                if rz.due && station != run.zero {
                    theme::chip(ui, format!("Measure the zero again: {text}"), theme::ACCENT)
                        .on_hover_text(format!("Drift rate {} per minute, from {from}.", with_unit(unit, rz.rate_um_per_min)));
                    if ui
                        .small_button("Go to the zero")
                        .on_hover_text("Pick the zero station for the next sample, so the drift since the last zero visit can be taken out.")
                        .clicked()
                    {
                        actions.push(Action::Pick(run.zero));
                    }
                } else {
                    ui.label(theme::dim(text))
                        .on_hover_text(format!("Drift rate {} per minute, from {from}.", with_unit(unit, rz.rate_um_per_min)));
                }
            }
        });
        if let Some(why) = &e.survey_error {
            ui.colored_label(theme::ERROR, why).on_hover_text("The last change to the run that did not work, and why.");
        }
    });
}

/// Station heights by height, from the zero station, µm.
fn station_heights(heights: Option<&Heights>) -> Vec<Option<(f64, f64)>> {
    let Some(h) = heights else { return Vec::new() };
    let n = h.stations.iter().map(|s| s.station + 1).max().unwrap_or(0);
    let mut out = vec![None; n];
    for s in &h.stations {
        out[s.station] = Some((s.height_um, s.uncertainty_um));
    }
    out
}

fn line_plot(
    app: &App,
    ui: &mut egui::Ui,
    run: &Run,
    heights: Option<&Heights>,
    station: usize,
    height: f32,
    actions: &mut Vec<Action>,
) {
    let unit = app.settings.unit;
    let per_station = station_heights(heights);
    let xs: Vec<f64> = run.stations.iter().map(|s| s.x_mm).collect();
    let mut x = plot::span(xs.iter().copied(), 0.06);
    if x[1] - x[0] < 1.0 {
        x = [x[0] - 50.0, x[1] + 50.0];
    }
    let mut ys = vec![0.0];
    for (h, u) in per_station.iter().flatten() {
        ys.push(unit.from_um(h + u));
        ys.push(unit.from_um(h - u));
    }
    if let Some(h) = heights {
        ys.extend(h.visits.iter().map(|v| unit.from_um(v.height_um)));
    }
    let y = plot::span(ys, 0.12);
    let legend = format!(
        "Each station's height against the zero station, in {}, with the drift taken out. Across: distance along the line.\n\
         Red dots and line: each station's mean height. Red bars: ±1 standard error, how sure the mean is.\n\
         Faint dots: each visit on its own. Orange band: the station picked for the next sample.\n\
         The numbers along the bottom are the stations; 0 is the zero. Click near a station to pick it.",
        unit.label()
    );
    ui.horizontal(|ui| {
        ui.label(theme::heading("Heights so far")).on_hover_text(&legend);
        ui.label(theme::dim("against the zero station, drift taken out; the bars are ±1 standard error"))
            .on_hover_text(&legend);
    });
    let (mut ax, painter, resp) = Axes::allocate(ui, height, x, y);
    ax.reserve_bottom_row(&painter); // for the station numbers
    let resp = resp.on_hover_text(&legend);
    ax.xband(
        &painter,
        run.stations[station].x_mm - (x[1] - x[0]) * 0.01,
        run.stations[station].x_mm + (x[1] - x[0]) * 0.01,
        theme::faded(theme::ACCENT, 50),
    );
    let step = (y[1] - y[0]) / 5.0;
    ax.grid(
        &painter,
        |v| format!("{v:.0} mm"),
        |v| format!("{} {}", plot::tick_text(v, step), if v == 0.0 { unit.label() } else { "" }),
    );
    ax.hline(&painter, 0.0, Stroke::new(1.0, theme::faded(theme::TEXT, 60)));
    for (i, s) in run.stations.iter().enumerate() {
        let color = if i == station { theme::ACCENT } else { theme::TEXT_DIM };
        ax.label(
            &painter,
            [s.x_mm, ax.y[0]],
            Align2::CENTER_BOTTOM,
            if i == run.zero { "0".to_owned() } else { format!("{}", i + 1) },
            color,
        );
    }
    if let Some(h) = heights {
        ax.dots(
            &painter,
            h.visits.iter().map(|v| [run.stations[v.station].x_mm, unit.from_um(v.height_um)]),
            2.0,
            theme::faded(theme::TEXT, 120),
        );
    }
    let mut means: Vec<[f64; 2]> = per_station
        .iter()
        .enumerate()
        .filter_map(|(i, hu)| hu.map(|(h, _)| [run.stations[i].x_mm, unit.from_um(h)]))
        .collect();
    means.sort_by(|a, b| a[0].total_cmp(&b[0]));
    ax.line(&painter, means.iter().copied(), Stroke::new(1.5, theme::faded(LASER, 160)));
    for (i, hu) in per_station.iter().enumerate() {
        if let Some((h, u)) = hu {
            let xm = run.stations[i].x_mm;
            painter.line_segment(
                [ax.pos(xm, unit.from_um(h - u)), ax.pos(xm, unit.from_um(h + u))],
                Stroke::new(1.5, LASER),
            );
        }
    }
    ax.dots(&painter, means.iter().copied(), 3.5, LASER);
    pick_on_click(&ax, &resp, run, actions, |s| [s.x_mm, 0.0], true);
}

fn grid_plot(
    app: &App,
    ui: &mut egui::Ui,
    run: &Run,
    heights: Option<&Heights>,
    station: usize,
    height: f32,
    actions: &mut Vec<Action>,
) {
    let unit = app.settings.unit;
    let per_station = station_heights(heights);
    let x = plot::span(run.stations.iter().map(|s| s.x_mm), 0.12);
    let y = plot::span(run.stations.iter().map(|s| s.y_mm), 0.15);
    let legend = format!(
        "The stations seen from above, placed in mm across and deep.\n\
         Square: the zero station. Red: measured, with its height against the zero in {} (drift taken out). Grey: not measured yet, with its name.\n\
         Orange ring: the station picked for the next sample. Click a station to pick it.",
        unit.label()
    );
    ui.horizontal(|ui| {
        ui.label(theme::heading("Stations")).on_hover_text(&legend);
        ui.label(theme::dim("seen from above; heights against the zero station, drift taken out")).on_hover_text(&legend);
    });
    let (ax, painter, resp) = Axes::allocate(ui, height, x, y);
    let resp = resp.on_hover_text(&legend);
    ax.grid(&painter, |v| format!("{v:.0} mm"), |v| format!("{v:.0} mm"));
    for (i, s) in run.stations.iter().enumerate() {
        let p = ax.pos(s.x_mm, s.y_mm);
        let visited = per_station.get(i).copied().flatten();
        let fill = if visited.is_some() { LASER } else { theme::NODE };
        if i == run.zero {
            painter.rect_filled(egui::Rect::from_center_size(p, egui::vec2(10.0, 10.0)), 1.0, fill);
        } else {
            painter.circle_filled(p, 5.0, fill);
        }
        if i == station {
            painter.circle_stroke(p, 9.0, Stroke::new(2.0, theme::ACCENT));
        }
        let text = match visited {
            Some((h, _)) => with_unit(unit, h),
            None => station_name(run, i),
        };
        painter.text(
            p + egui::vec2(0.0, -11.0),
            Align2::CENTER_BOTTOM,
            text,
            egui::FontId::proportional(11.0),
            theme::TEXT,
        );
    }
    pick_on_click(&ax, &resp, run, actions, |s| [s.x_mm, s.y_mm], false);
}

/// Pick the station nearest a click.
fn pick_on_click(
    ax: &Axes,
    resp: &egui::Response,
    run: &Run,
    actions: &mut Vec<Action>,
    at: impl Fn(&Station) -> [f64; 2],
    x_only: bool,
) {
    if !resp.clicked() {
        return;
    }
    let Some(p) = resp.interact_pointer_pos() else { return };
    let distance = |s: &Station| {
        let q = at(s);
        let q = ax.pos(q[0], q[1]);
        if x_only { (q.x - p.x).abs() } else { q.distance(p) }
    };
    if let Some((i, d)) = run.stations.iter().map(distance).enumerate().min_by(|a, b| a.1.total_cmp(&b.1))
        && d < 40.0
    {
        actions.push(Action::Pick(i));
    }
}

fn visits_table(
    app: &App,
    ui: &mut egui::Ui,
    e: &Engine,
    run: &Run,
    heights: Option<&Heights>,
    actions: &mut Vec<Action>,
) {
    let unit = app.settings.unit;
    let busy = e.sampling.is_some();
    ui.label(theme::heading(format!("Visits ({})", run.visits.len())))
        .on_hover_text("Every sample taken in this run, newest at the top. A station visited more than once has a row for each visit.");
    if run.visits.is_empty() {
        ui.label(theme::dim("None yet. The first sample goes on the zero station."));
        return;
    }
    let by_visit = |i: usize| heights.and_then(|h| h.visits.iter().find(|v| v.visit == i));
    let heads = [
        ("#", "The visit's number, in the order taken.".to_owned()),
        ("Station", "The station the block was on.".to_owned()),
        ("Time", "When the sample was taken, as session time.".to_owned()),
        ("Height", format!("Height against the zero station, in {}, with the drift taken out.", unit.label())),
        ("±", format!("Standard error of the height, in {}: how sure it is.", unit.label())),
        ("Frames", "Frames used, of the frames seen. Frames with no beam or far from the rest are left out.".to_owned()),
        ("Checks", "ok, or what makes the visit less sure. Hover over a visit's checks for what each one means.".to_owned()),
        ("", String::new()),
    ];
    egui::Grid::new("visits").striped(true).num_columns(8).spacing([14.0, 4.0]).show(ui, |ui| {
        for (head, help) in &heads {
            let r = ui.label(theme::dim(*head));
            if !help.is_empty() {
                r.on_hover_text(help);
            }
        }
        ui.end_row();
        for (i, v) in run.visits.iter().enumerate().rev() {
            ui.label(format!("{}", i + 1));
            ui.label(station_name(run, v.station));
            ui.label(clock(v.reading.t_s));
            match by_visit(i) {
                Some(h) => {
                    ui.label(unit.format(h.height_um)).on_hover_text(&heads[3].1);
                    ui.label(unit.format(h.uncertainty_um)).on_hover_text(&heads[4].1);
                }
                None => {
                    ui.label("–").on_hover_text("No height yet: the run needs a zero visit first.");
                    ui.label("–");
                }
            }
            ui.label(format!("{}/{}", v.reading.used, v.reading.frames)).on_hover_text(&heads[5].1);
            let names: Vec<&str> = v.reading.flags.names().collect();
            if names.is_empty() {
                ui.colored_label(theme::OK, "ok").on_hover_text("No checks failed. This visit can be trusted.");
            } else {
                let help: Vec<String> = names.iter().map(|n| format!("{n}: {}", flag_help(n))).collect();
                ui.colored_label(theme::ACCENT, names.join(", ")).on_hover_text(help.join("\n"));
            }
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .small_button("Measure again")
                        .on_hover_text("Take this visit again on the same station. The new sample takes its place.")
                        .clicked()
                    {
                        actions.push(Action::Take(v.station, Some(i)));
                    }
                    if ui.small_button("Delete").on_hover_text("Throw this visit away.").clicked() {
                        actions.push(Action::Delete(i));
                    }
                });
            });
            ui.end_row();
        }
    });
}

/// How long each sample is averaged. Longer samples average away more noise.
fn sample_rule(ui: &mut egui::Ui, s: &mut SampleRule, unit: Unit) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Each sample").on_hover_text(
            "How long each reading is averaged. Longer samples average away more noise; a target ends a sample early once it is sure enough.",
        );
        ui.label(theme::dim("at least")).on_hover_text("A sample is at least this long.");
        ui.add(egui::DragValue::new(&mut s.min_s).range(0.2..=120.0).speed(0.1).suffix(" s"))
            .on_hover_text("A sample is at least this long.");
        ui.label(theme::dim("at most")).on_hover_text("A sample ends after this long, sure enough or not.");
        ui.add(egui::DragValue::new(&mut s.max_s).range(0.5..=600.0).speed(0.1).suffix(" s"))
            .on_hover_text("A sample ends after this long, sure enough or not.");
        let mut target = s.target_um.is_some();
        ui.checkbox(&mut target, "or once good to")
            .on_hover_text("End the sample once its uncertainty is below this.");
        let mut t = unit.from_um(s.target_um.unwrap_or(0.1));
        let drag = egui::DragValue::new(&mut t)
            .range(unit.from_um(0.001)..=unit.from_um(10.0))
            .speed(unit.from_um(0.001))
            .max_decimals(unit.decimals() + 1)
            .suffix(format!(" {}", unit.label()));
        ui.add_enabled(target, drag)
            .on_hover_text("The sample ends once its standard error is below this, after the shortest time.")
            .on_disabled_hover_text("Tick \"or once good to\" to end samples early once they are sure enough.");
        s.target_um = target.then_some(unit.to_um(t).clamp(0.001, 10.0));
    });
    if s.max_s < s.min_s {
        s.max_s = s.min_s;
    }
}

/// The run as a whole: drift seen by the zero visits, and how well repeat visits agree.
fn run_figures(app: &App, ui: &mut egui::Ui, heights: Option<&Heights>) {
    let unit = app.settings.unit;
    ui.label(theme::heading("This run"))
        .on_hover_text("Figures for the whole run: the drift the zero visits saw, and how well repeat visits agree.");
    let Some(h) = heights else {
        ui.label(theme::dim("Figures for the run come once it has heights."));
        return;
    };
    {
        theme::cards(ui, |ui| {
            if let Some(c) = h.drift.closure_um {
                stat(
                    ui,
                    "Zero closure",
                    with_unit(unit, c),
                    theme::TEXT,
                    "How far the zero station moved between its first and last visit: the drift taken out.",
                );
            }
            if let Some(r) = h.drift.rate_um_per_min {
                stat(
                    ui,
                    "Drift between zeros",
                    format!("{} /min", with_unit(unit, r)),
                    theme::TEXT,
                    "The drift seen by the zero visits.",
                );
            }
            if let Some(r) = h.repeatability {
                stat(
                    ui,
                    "Repeatability",
                    with_unit(unit, r.pooled_sd_um),
                    if r.ratio > 3.0 { theme::ACCENT } else { theme::TEXT },
                    "Scatter of repeat visits to the same station. Far above the reading uncertainty means the block does not sit the same way twice: dirt, burrs, or how it is put down.",
                );
            }
            if !h.drift.closed {
                theme::card_chip(ui, "Finish on the zero to close the run", theme::TEXT_DIM)
                    .on_hover_text("Without a last zero visit the drift after the previous one is not known.");
            }
        });
    }
}

fn stations_editor(ui: &mut egui::Ui, run: &Run, busy: bool, actions: &mut Vec<Action>) {
    egui::CollapsingHeader::new(theme::heading(format!("Stations ({})", run.stations.len())))
        .id_salt("stations_editor")
        .show(ui, |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                let grid = run.layout == Layout::Grid;
                const NAME: &str = "A name for the station, such as a mark on the part. Empty: its number.";
                const ALONG: &str = "Where the station is along the line, in mm from the first station.";
                const ACROSS: &str = "Where the station is deep, in mm from the first row.";
                const ZERO: &str = "The station every height is measured from. Visit it at the start, the end, and now and then between to measure the drift.";
                egui::Grid::new("stations").striped(true).num_columns(6).spacing([10.0, 4.0]).show(ui, |ui| {
                    for (head, help) in [
                        ("#", "The station's number."),
                        ("Name", NAME),
                        ("Along", ALONG),
                        (if grid { "Across" } else { "" }, if grid { ACROSS } else { "" }),
                        ("Zero", ZERO),
                        ("", ""),
                    ] {
                        let r = ui.label(theme::dim(head));
                        if !help.is_empty() {
                            r.on_hover_text(help);
                        }
                    }
                    ui.end_row();
                    for (i, s) in run.stations.iter().enumerate() {
                        let mut st = s.clone();
                        ui.label(format!("{}", i + 1));
                        let mut changed = ui
                            .add(
                                egui::TextEdit::singleline(&mut st.label)
                                    .desired_width(80.0)
                                    .hint_text(format!("{}", i + 1)),
                            )
                            .on_hover_text(NAME)
                            .changed();
                        changed |=
                            ui.add(egui::DragValue::new(&mut st.x_mm).speed(1.0).suffix(" mm")).on_hover_text(ALONG).changed();
                        if grid {
                            changed |= ui
                                .add(egui::DragValue::new(&mut st.y_mm).speed(1.0).suffix(" mm"))
                                .on_hover_text(ACROSS)
                                .changed();
                        } else {
                            ui.label("");
                        }
                        if changed {
                            actions.push(Action::Edit(i, st));
                        }
                        if ui
                            .radio(i == run.zero, "")
                            .on_hover_text("The station every height is measured from")
                            .clicked()
                            && i != run.zero
                        {
                            actions.push(Action::SetZero(i));
                        }
                        let n = run.visits.iter().filter(|v| v.station == i).count();
                        let remove = ui
                            .add_enabled(i != run.zero, egui::Button::new("Remove").small())
                            .on_disabled_hover_text("The zero station cannot be removed. Make another station the zero first.");
                        let remove = if n > 0 {
                            remove.on_hover_text(format!("Take this station out of the run. Its {n} visit(s) go too."))
                        } else {
                            remove.on_hover_text("Take this station out of the run.")
                        };
                        if remove.clicked() {
                            actions.push(Action::RemoveStation(i));
                        }
                        ui.end_row();
                    }
                });
                if ui
                    .button("Add a station")
                    .on_hover_text("Add a station after the last one, as far on as the last two are apart.")
                    .clicked()
                {
                    actions.push(Action::AddStation);
                }
            });
        })
        .header_response
        .on_hover_text("The run's stations: names, places and which one is the zero. Click to open or close.");
}

/// What a check on a reading means, by the name the reading gives it.
pub fn flag_help(name: &str) -> &'static str {
    match name {
        "clipped" => "Frames were clipped: the beam's top was flat at white. Lower the exposure, the gain or the laser power.",
        "tilted" => "The line is not square to the sensor. Turn the camera or the laser until the line runs level.",
        "near_edge" => "The beam is close to the edge of the frame. Move the camera so the beam sits nearer the middle.",
        "dropouts" => "Some frames had no beam. Check for shadows, dust or a hand in the beam.",
        "outliers" => "Frames far from the rest were left out. A knock or a flicker can do this.",
        "moving" => "The reading moved during the sample: the block was still settling, or it was bumped. Measure it again.",
        "few_frames" => "Too few frames for a trustworthy uncertainty. Make the sample longer.",
        "drift_unchecked" => "Taken before the first or after the last zero visit, so the drift since the zero is not checked. Measure the zero station again.",
        _ => "A check on this reading did not pass.",
    }
}
