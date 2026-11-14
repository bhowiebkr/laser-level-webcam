//! Waiting for the set-up to settle. A laser, a camera and a stand that have just been switched on
//! or moved warm up and drift as they do, fast at first and then slower. This screen shows the
//! position over time, the settling curve fitted to it, and when the drift over the planned run
//! will be inside the allowed drift.

use eframe::egui::{self, Align2, RichText, Stroke};
use laser_core::monitor::{Readiness, Status};
use laser_core::units::Unit;

use crate::app::{App, Screen};
use crate::engine::{Engine, TRACE_S};
use crate::plot::{self, Axes, clock};
use crate::setup::{stat, with_unit};
use crate::theme;
use crate::visuals::LASER;

pub fn draw(app: &mut App, ui: &mut egui::Ui, e: &mut Engine) {
    let Some(status) = e.status.clone() else {
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("No readings yet").size(20.0).strong());
            ui.label(theme::dim("The drift estimate starts once the beam is found. See Set up."));
            ui.add_space(4.0);
            plan_row(app, ui);
        });
        return;
    };
    // The graphs share the height left. In a short window they keep a usable height and the
    // screen scrolls.
    let full = ui.available_height();
    let mut restart = false;
    theme::scroll(ui, |ui| {
        let top = ui.cursor().top();
        let left = |ui: &egui::Ui| full - (ui.cursor().top() - top);
        if banner(app, ui, &status) {
            restart = true;
        }
        ui.add_space(6.0);
        tiles(app, ui, e, &status);
        ui.add_space(6.0);
        let trace_h = 160.0;
        history(app, ui, e, &status, (left(ui) - trace_h - 48.0).max(200.0));
        ui.add_space(6.0);
        trace(app, ui, e, (left(ui) - 28.0).max(110.0));
    });
    if restart {
        e.restart_monitor();
    }
}

/// The planned run, which sets when the set-up counts as ready. Says whether to start the
/// estimate again.
fn plan_row(app: &mut App, ui: &mut egui::Ui) -> bool {
    let p = &mut app.settings.plan;
    let mut restart = false;
    ui.horizontal_wrapped(|ui| {
        ui.label("Planned run").on_hover_text(
            "How long your measurement will take: stations × time at each. Ready is when the drift expected over a run this long is inside the allowed drift.",
        );
        ui.add(egui::DragValue::new(&mut p.run_min).range(1.0..=480.0).speed(0.5).suffix(" min"));
        ui.add_space(8.0);
        ui.label("Allowed drift").on_hover_text("How far the reading may drift over the run.");
        ui.add(egui::DragValue::new(&mut p.tolerance_um).range(0.01..=100.0).speed(0.01).suffix(" µm"));
        ui.add_space(8.0);
        restart = ui
            .button("Start the estimate again")
            .on_hover_text("After a knock, or a change to the set-up: forget the readings so far.")
            .clicked();
    });
    restart
}

fn banner(app: &mut App, ui: &mut egui::Ui, s: &Status) -> bool {
    let unit = app.settings.unit;
    let plan = app.settings.plan;
    let tol = with_unit(unit, plan.tolerance_um);
    let (title, detail, color) = match s.readiness {
        Readiness::Collecting { have_s, need_s } => (
            "Learning the drift".to_owned(),
            format!("{} of the {} of readings the first estimate needs.", clock(have_s), clock(need_s)),
            theme::TEXT,
        ),
        Readiness::Noisy { noise } => (
            "Too noisy to tell".to_owned(),
            format!(
                "The reading wanders {} from one second to the next, more than the allowed drift of {tol}. Steady the stand, shade the beam from air currents, or allow more drift.",
                with_unit(unit, noise)
            ),
            theme::ERROR,
        ),
        Readiness::Settling { in_s, earliest_in_s, latest_in_s } => (
            format!("Settling: ready in about {}", clock(in_s)),
            format!(
                "Somewhere between {} and {} from now, the drift over a {:.0} min run will be under {tol}.",
                clock(earliest_in_s),
                latest_in_s.map_or_else(|| "a time not known yet".to_owned(), clock),
                plan.run_min
            ),
            theme::ACCENT,
        ),
        Readiness::Drifting { rate_per_min } => (
            "Drifting".to_owned(),
            format!(
                "{} per minute and not slowing down yet. A set-up that is warming up slows down; one that keeps going has something loose or a heat source nearby.",
                with_unit(unit, rate_per_min)
            ),
            theme::ACCENT,
        ),
        Readiness::Ready => (
            "Ready to measure".to_owned(),
            format!("The drift expected over a {:.0} min run is under {tol}.", plan.run_min),
            theme::OK,
        ),
    };
    theme::card()
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 7.0, color);
                ui.label(RichText::new(title).color(color).size(20.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let go = if s.readiness == Readiness::Ready {
                        theme::primary("Measure")
                    } else {
                        egui::Button::new("Measure anyway")
                    };
                    if ui.add(go).clicked() {
                        app.screen = Screen::Measure;
                    }
                });
            });
            ui.label(theme::dim(detail));
            if let Readiness::Collecting { have_s, need_s } = s.readiness {
                ui.add(egui::ProgressBar::new((have_s / need_s.max(1e-9)) as f32).desired_height(6.0));
            }
            ui.add_space(4.0);
            plan_row(app, ui)
        })
        .inner
}

fn tiles(app: &App, ui: &mut egui::Ui, e: &Engine, s: &Status) {
    let unit = app.settings.unit;
    let run_s = app.settings.plan.run_min * 60.0;
    let tol = app.settings.plan.tolerance_um;
    let bin_s = e.monitor.config().bin_s;
    theme::cards(ui, |ui| {
        if let Some(x) = s.latest {
            stat(
                ui,
                "Now",
                with_unit(unit, x),
                theme::TEXT,
                "The line's position across the sensor, from the sensor's scale.",
            );
        }
        if let Some(rate) = s.rate_per_min {
            let sigma = s.rate_sigma_per_min.map_or_else(String::new, |sd| format!(" ± {}", unit.format(sd)));
            stat(
                ui,
                "Drift",
                format!("{}{sigma} {}/min", unit.format(rate), unit.label()),
                theme::TEXT,
                "How fast the reading moves now, from a straight line through the last minutes.",
            );
        }
        if let Some(noise) = s.noise {
            stat(
                ui,
                &format!("Noise per {bin_s:.0} s"),
                with_unit(unit, noise),
                if noise > tol { theme::ERROR } else { theme::TEXT },
                "Scatter of the one-second averages about the drift: vibration, air currents, camera noise.",
            );
        }
        if let Some(a) = s.allan_best {
            stat(
                ui,
                "Steadiest average",
                format!("{} over {}", with_unit(unit, a.deviation), clock(a.tau_s)),
                theme::TEXT,
                "Averaging over this long gives the steadiest reading (the bottom of the Allan deviation). Longer lets the drift in; a sample near this length is best.",
            );
        }
        let over = match (&s.settle, s.rate_per_min) {
            (Some(f), _) => Some(f.drift_over(s.t_s, run_s)),
            (None, Some(rate)) => Some(rate.abs() * run_s / 60.0),
            _ => None,
        };
        if let Some(d) = over {
            stat(
                ui,
                &format!("Drift over {:.0} min", run_s / 60.0),
                with_unit(unit, d),
                if d <= tol { theme::OK } else { theme::ACCENT },
                "The drift expected if the run started now, against the allowed drift.",
            );
        }
        if let Some(f) = &s.settle {
            let high = f.tau_high_s.map_or_else(|| "?".to_owned(), clock);
            stat(
                ui,
                "Time constant",
                format!("{} ({}–{high})", clock(f.tau_s), clock(f.tau_low_s)),
                theme::TEXT,
                "Time for the drift still to come to fall to 37 %. After three time constants 5 % is left.",
            );
        }
        stat(
            ui,
            "Estimate from",
            format!("{} ago", clock(s.t_s - s.since_s)),
            theme::TEXT,
            "The estimate uses the readings since the first one, the last knock, or the last restart.",
        );
        if !s.steps.is_empty() {
            theme::card_chip(
                ui,
                format!("{} knock{}", s.steps.len(), if s.steps.len() == 1 { "" } else { "s" }),
                theme::ERROR,
            )
            .on_hover_text("Sudden jumps in the reading. The estimate starts again after each.");
        }
    });
}

/// Decimals for tick labels across a span.
fn tick_label(unit: Unit, span: f64) -> impl Fn(f64) -> String {
    let step = span / 5.0;
    move |v| format!("{} {}", plot::tick_text(v, step), if v == 0.0 { unit.label() } else { "" })
}

fn history(app: &App, ui: &mut egui::Ui, e: &Engine, s: &Status, height: f32) {
    let unit = app.settings.unit;
    let run_s = app.settings.plan.run_min * 60.0;
    let bins = e.monitor.bins();
    ui.horizontal(|ui| {
        ui.label(theme::heading("Position over time"));
        ui.label(theme::dim("one-second averages, against the reading now"));
    });
    if bins.len() < 2 {
        ui.label(theme::dim("Collecting…"));
        return;
    }
    let reference = s.latest.unwrap_or(bins[bins.len() - 1].mean);
    let y_of = |um: f64| unit.from_um(um - reference);
    let t0 = bins[0].t_s;
    // Look ahead to when it will be ready, or over the planned run when it is.
    let ahead = match s.readiness {
        Readiness::Settling { in_s, latest_in_s, .. } => latest_in_s.unwrap_or(in_s * 1.5).max(in_s * 1.1),
        Readiness::Ready => run_s,
        _ => 0.0,
    }
    .min((s.t_s - t0).max(60.0) * 3.0);
    let x = [t0, s.t_s + ahead];
    let mut ys: Vec<f64> = bins.iter().map(|b| y_of(b.mean)).collect();
    if let Some(f) = &s.settle {
        ys.push(y_of(f.value_at(x[1])));
        ys.push(y_of(f.value_at(f.t0_s.max(t0))));
    }
    let y = plot::span(ys, 0.08);
    let (ax, painter, resp) = Axes::allocate(ui, height, x, y);

    let mut ready_past = false;
    match s.readiness {
        Readiness::Settling { in_s, earliest_in_s, latest_in_s } => {
            ax.xband(
                &painter,
                s.t_s + earliest_in_s,
                latest_in_s.map_or(x[1], |l| s.t_s + l),
                theme::faded(theme::OK, 30),
            );
            ax.vline(&painter, s.t_s + in_s, Stroke::new(1.5, theme::OK));
            if s.t_s + in_s <= x[1] {
                ax.label(&painter, [s.t_s + in_s, y[1]], Align2::LEFT_TOP, " ready", theme::OK);
            } else {
                ready_past = true;
                let text = format!("ready in about {}, past the graph ", clock(in_s));
                ax.label(&painter, [x[1], y[1]], Align2::RIGHT_TOP, text, theme::OK);
            }
        }
        Readiness::Ready => {
            ax.xband(&painter, s.t_s, s.t_s + run_s, theme::faded(theme::OK, 30));
            ax.label(&painter, [s.t_s, y[1]], Align2::LEFT_TOP, " the planned run", theme::OK);
        }
        _ => {}
    }
    ax.grid(&painter, clock, tick_label(unit, y[1] - y[0]));
    if s.since_s > t0 {
        ax.vline(&painter, s.since_s, Stroke::new(1.0, theme::TEXT_DIM));
        ax.label(&painter, [s.since_s, y[0]], Align2::LEFT_BOTTOM, " estimate from here", theme::TEXT_DIM);
    }
    for step in &s.steps {
        ax.vline(&painter, step.t_s, Stroke::new(1.0, theme::ERROR));
        ax.label(
            &painter,
            [step.t_s, y[1]],
            Align2::RIGHT_TOP,
            format!("knock {} ", with_unit(unit, step.size)),
            theme::ERROR,
        );
    }
    ax.vline(&painter, s.t_s, Stroke::new(1.0, theme::faded(theme::TEXT, 90)));
    ax.line(&painter, bins.iter().map(|b| [b.t_s, y_of(b.mean)]), Stroke::new(1.0, theme::faded(theme::TEXT, 120)));
    ax.dots(&painter, bins.iter().map(|b| [b.t_s, y_of(b.mean)]), 1.8, theme::TEXT);
    if let Some(f) = &s.settle {
        let from = f.t0_s.max(t0);
        let curve = |a: f64, b: f64| (0..=120).map(move |i| a + (b - a) * f64::from(i) / 120.0);
        ax.line(&painter, curve(from, s.t_s).map(|t| [t, y_of(f.value_at(t))]), Stroke::new(2.0, LASER));
        ax.line(
            &painter,
            curve(s.t_s, x[1]).map(|t| [t, y_of(f.value_at(t))]),
            Stroke::new(1.5, theme::faded(LASER, 110)),
        );
        ax.dashed(
            &painter,
            [x[0], y_of(f.settled)],
            [x[1], y_of(f.settled)],
            Stroke::new(1.0, theme::faded(LASER, 130)),
        );
        // On the right, clear of the tick labels; under the late "ready" note when that is there.
        let level = y_of(f.settled);
        let color = theme::faded(LASER, 200);
        let line = 16.0 * (y[1] - y[0]) / f64::from(ax.rect.height());
        if level < y[0] {
            ax.label(&painter, [x[1], y[0]], Align2::RIGHT_BOTTOM, "settles below the graph ", color);
        } else if level > y[1] {
            let top = if ready_past { y[1] - line } else { y[1] };
            ax.label(&painter, [x[1], top], Align2::RIGHT_TOP, "settles above the graph ", color);
        } else {
            ax.label(&painter, [x[1], level], Align2::RIGHT_BOTTOM, "where it settles ", color);
        }
    }
    if let Some(p) = resp.hover_pos().filter(|p| ax.rect.contains(*p)) {
        let t = ax.data_x(p.x);
        let near = bins.iter().min_by(|a, b| (a.t_s - t).abs().total_cmp(&(b.t_s - t).abs()));
        if let Some(b) = near.filter(|b| (b.t_s - t).abs() < 5.0) {
            painter.text(
                ax.rect.right_top() + egui::vec2(-4.0, 4.0),
                Align2::RIGHT_TOP,
                format!("{}  {}", clock(b.t_s), with_unit(unit, b.mean - reference)),
                egui::FontId::proportional(11.0),
                theme::TEXT,
            );
        }
    }
}

fn trace(app: &App, ui: &mut egui::Ui, e: &Engine, height: f32) {
    let unit = app.settings.unit;
    let n = e.trace.len();
    let mean = if n > 0 { e.trace.iter().map(|p| p[1]).sum::<f64>() / n as f64 } else { 0.0 };
    let sd =
        if n > 1 { (e.trace.iter().map(|p| (p[1] - mean).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt() } else { 0.0 };
    ui.horizontal(|ui| {
        ui.label(theme::heading(format!("The last {TRACE_S:.0} s, every frame")));
        ui.label(theme::dim(format!("scatter {} about the mean", with_unit(unit, sd))));
    });
    if n < 2 {
        return;
    }
    let now = e.trace.back().map_or(0.0, |p| p[0]);
    let pts: Vec<[f64; 2]> = e.trace.iter().map(|p| [p[0] - now, unit.from_um(p[1] - mean)]).collect();
    let y = plot::span(pts.iter().map(|p| p[1]), 0.1);
    let (ax, painter, _) = Axes::allocate(ui, height, [-TRACE_S, 0.0], y);
    ax.grid(&painter, |t| format!("{t:.0} s"), tick_label(unit, y[1] - y[0]));
    ax.line(&painter, pts, Stroke::new(1.0, theme::BLUE));
}
