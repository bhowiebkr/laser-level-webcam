//! The dark theme: colours, text sizes and a few frames used across the app.

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Stroke};

pub const ACCENT: Color32 = Color32::from_rgb(232, 163, 61);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(58, 50, 32);
pub const BACKGROUND: Color32 = Color32::from_rgb(22, 24, 27);
pub const CANVAS: Color32 = Color32::from_rgb(18, 20, 23);
pub const PANEL: Color32 = Color32::from_rgb(31, 34, 38);
pub const LINE: Color32 = Color32::from_rgb(53, 58, 65);
pub const NODE: Color32 = Color32::from_rgb(39, 43, 48);
pub const TEXT: Color32 = Color32::from_rgb(230, 232, 234);
pub const TEXT_DIM: Color32 = Color32::from_rgb(154, 161, 169);
pub const ERROR: Color32 = Color32::from_rgb(224, 108, 90);
pub const OK: Color32 = Color32::from_rgb(126, 196, 122);
pub const BLUE: Color32 = Color32::from_rgb(110, 160, 220);
/// Text on an accent fill.
pub const ON_ACCENT: Color32 = Color32::from_rgb(27, 20, 6);

/// `c` at alpha `a`, for fills under lines.
pub fn faded(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(egui::Theme::Dark);
    let mut v = egui::Visuals::dark();
    v.panel_fill = BACKGROUND;
    v.window_fill = PANEL;
    v.extreme_bg_color = CANVAS;
    v.faint_bg_color = PANEL;
    v.code_bg_color = NODE;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.text_cursor.stroke = Stroke::new(1.0, ACCENT);
    v.override_text_color = Some(TEXT);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.bg_fill = NODE;
        w.weak_bg_fill = NODE;
        w.bg_stroke = Stroke::new(1.0, LINE);
        w.corner_radius = 5.into();
    }
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, TEXT_DIM);
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        for (kind, size) in [
            (egui::TextStyle::Body, 14.0),
            (egui::TextStyle::Button, 14.0),
            (egui::TextStyle::Small, 12.0),
            (egui::TextStyle::Heading, 16.0),
            (egui::TextStyle::Monospace, 13.0),
        ] {
            let family = if kind == egui::TextStyle::Monospace {
                egui::FontFamily::Monospace
            } else {
                egui::FontFamily::Proportional
            };
            s.text_styles.insert(kind, egui::FontId::new(size, family));
        }
        s.spacing.item_spacing = egui::vec2(10.0, 8.0);
        s.spacing.button_padding = egui::vec2(8.0, 4.0);
        s.spacing.interact_size = egui::vec2(40.0, 26.0);
        s.spacing.slider_width = 150.0;
    });
}

/// The system's UI fonts where they are known to be; egui's own fonts everywhere else.
fn fonts() -> egui::FontDefinitions {
    #[allow(unused_mut)]
    let mut fonts = egui::FontDefinitions::default();
    #[cfg(windows)]
    for (name, path, family) in [
        ("segoe", "C:/Windows/Fonts/segoeui.ttf", egui::FontFamily::Proportional),
        ("consolas", "C:/Windows/Fonts/consola.ttf", egui::FontFamily::Monospace),
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(name.into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
            fonts.families.entry(family).or_default().insert(0, name.into());
        }
    }
    fonts
}

/// A raised panel.
pub fn card() -> Frame {
    Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(14, 12))
}

/// The main action on a screen.
pub fn primary(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).color(ON_ACCENT).strong()).fill(ACCENT)
}

/// The padding inside a stat card.
pub const STAT_MARGIN: Margin = Margin::symmetric(10, 6);
const CHIP_MARGIN: Margin = Margin::symmetric(8, 2);

/// Stat cards in a row that wraps to the width, their tops in line.
pub fn cards<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let size = egui::vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y);
    let layout = egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true);
    ui.allocate_ui_with_layout(size, layout, add).inner
}

/// A vertical scroll area over the space left. In tests it notes its clip and its box, so the
/// layout check can tell words scrolled out of sight from words lost past the edge of a panel, and
/// words in two scroll areas side by side from words in one row.
pub fn scroll<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let out = egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        #[cfg(test)]
        {
            let view = ui.clip_rect();
            ui.data_mut(|d| d.get_temp_mut_or_default::<Vec<egui::Rect>>(scroll_views()).push(view));
        }
        add(ui)
    });
    #[cfg(test)]
    ui.data_mut(|d| d.get_temp_mut_or_default::<Vec<egui::Rect>>(scroll_boxes()).push(out.inner_rect));
    out.inner
}

/// The clips of the scroll areas drawn. A vertical scroll area clips only up and down.
#[cfg(test)]
pub fn scroll_views() -> egui::Id {
    egui::Id::unique("scroll views")
}

/// The boxes of the scroll areas drawn, left to right as well as up and down.
#[cfg(test)]
pub fn scroll_boxes() -> egui::Id {
    egui::Id::unique("scroll boxes")
}

/// A small coloured label in an outline.
pub fn chip(ui: &mut egui::Ui, text: impl Into<String>, color: Color32) -> egui::Response {
    chip_lowered(ui, text.into(), color, 0)
}

/// A chip in a row of stat cards, its words in line with the cards' names.
pub fn card_chip(ui: &mut egui::Ui, text: impl Into<String>, color: Color32) -> egui::Response {
    chip_lowered(ui, text.into(), color, STAT_MARGIN.top - CHIP_MARGIN.top)
}

fn chip_lowered(ui: &mut egui::Ui, text: String, color: Color32, drop: i8) -> egui::Response {
    // Chips with the same words and different numbers share a width.
    let key: String = text.chars().filter(|c| !c.is_ascii_digit() && *c != '-').collect();
    let text = RichText::new(text).small().color(color);
    let width = steady_width(ui, &key, text_width(ui, text.clone()));
    wrap_before(ui, width + 18.0);
    Frame::new()
        .stroke(Stroke::new(1.0, faded(color, 140)))
        .fill(faded(color, 22))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(CHIP_MARGIN)
        .outer_margin(Margin { top: drop, ..Margin::ZERO })
        .show(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            ui.set_width(width);
            ui.vertical_centered(|ui| ui.label(text)).inner
        })
        .response
}

/// The width a tile showing changing numbers keeps: never narrower than it has been, so the tile
/// and the ones after it do not jump about as the numbers change.
pub fn steady_width(ui: &egui::Ui, key: &str, width: f32) -> f32 {
    let id = egui::Id::unique(("steady_width", key));
    let width = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(0.0).max(width);
    ui.data_mut(|d| d.insert_temp(id, width));
    width
}

/// Width of `text` on one line.
pub fn text_width(ui: &egui::Ui, text: impl Into<egui::WidgetText>) -> f32 {
    text.into().into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body).size().x
}

/// In a wrapping row, starts a new row when a tile `width` wide no longer fits on this one, so
/// tiles move down whole instead of being squeezed.
pub fn wrap_before(ui: &mut egui::Ui, width: f32) {
    let free = ui.available_size_before_wrap().x;
    if ui.layout().main_wrap && free < width && free < ui.max_rect().width() - 1.0 {
        ui.end_row();
    }
}

/// A slider as wide as the row, its value box at the right edge. `widest` holds the longest texts
/// the box shows, so the slider keeps its length as the value changes.
pub fn wide_slider(ui: &mut egui::Ui, enabled: bool, widest: &[String], slider: egui::Slider<'_>) -> egui::Response {
    ui.scope(|ui| {
        let style = ui.style().drag_value_text_style.clone();
        let text = widest
            .iter()
            .map(|t| {
                egui::WidgetText::from(t.as_str())
                    .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, style.clone())
                    .size()
                    .x
            })
            .fold(0.0, f32::max);
        let s = ui.spacing();
        let value_box = (text + 2.0 * s.button_padding.x + 2.0).max(s.interact_size.x);
        let width = ui.available_width() - value_box - s.item_spacing.x;
        ui.spacing_mut().slider_width = width.max(60.0);
        ui.add_enabled(enabled, slider)
    })
    .inner
}

pub fn heading(text: impl Into<String>) -> RichText {
    RichText::new(text).strong().color(ACCENT)
}

pub fn dim(text: impl Into<String>) -> RichText {
    RichText::new(text).color(TEXT_DIM)
}

/// Icons drawn as shapes: the built-in fonts have no arrows or symbols to label buttons with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    TurnLeft,
    TurnRight,
    FlipLeftRight,
    FlipUpDown,
}

/// Draws `icon` to fill the square in the middle of `rect`.
pub fn paint_icon(p: &egui::Painter, rect: egui::Rect, icon: Icon, color: Color32) {
    use egui::{Shape, vec2};
    let c = rect.center();
    let s = rect.width().min(rect.height()) / 2.0;
    let stroke = Stroke::new((s * 0.17).max(1.3), color);
    match icon {
        Icon::TurnLeft | Icon::TurnRight => {
            // Three quarters of a circle, open at the bottom, with the head where it ends.
            // Screen angles grow clockwise; mirroring left to right turns it the other way.
            let side = if icon == Icon::TurnRight { 1.0 } else { -1.0 };
            let r = s * 0.62;
            let at = |a: f32| c + vec2(side * r * a.cos(), r * a.sin());
            let (start, end) = (150f32.to_radians(), 400f32.to_radians());
            let points: Vec<_> = (0..=24).map(|i| at(start + (end - start) * i as f32 / 24.0)).collect();
            p.add(Shape::line(points, stroke));
            let way = vec2(-side * end.sin(), end.cos());
            let across = vec2(-way.y, way.x);
            let h = s * 0.42;
            let tip = at(end) + way * h * 0.8;
            let back = at(end) - way * h * 0.35;
            p.add(Shape::convex_polygon(
                vec![tip, back + across * h * 0.7, back - across * h * 0.7],
                color,
                Stroke::NONE,
            ));
        }
        Icon::FlipLeftRight | Icon::FlipUpDown => {
            // A dashed mirror line between a solid shape and its outlined reflection.
            let pt = |u: f32, v: f32| match icon {
                Icon::FlipLeftRight => c + vec2(u, v) * s,
                _ => c + vec2(v, u) * s,
            };
            let thin = Stroke::new((s * 0.12).max(1.0), color);
            p.extend(Shape::dashed_line(&[pt(0.0, -1.0), pt(0.0, 1.0)], thin, s * 0.22, s * 0.18));
            let shape = |side: f32| vec![pt(side * 0.95, 0.0), pt(side * 0.22, -0.72), pt(side * 0.22, 0.72)];
            p.add(Shape::convex_polygon(shape(-1.0), color, Stroke::NONE));
            p.add(Shape::closed_line(shape(1.0), thin));
        }
    }
}

/// A button with a drawn icon before its text.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, text: &str) -> egui::Response {
    use egui::{Rect, Sense, TextStyle, pos2, vec2};
    let pad = ui.spacing().button_padding;
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Button,
    );
    let side = ui.text_style_height(&TextStyle::Button);
    let gap = ui.spacing().icon_spacing.max(4.0);
    let size = vec2(
        pad.x * 2.0 + side + gap + galley.size().x,
        (pad.y * 2.0 + galley.size().y.max(side)).max(ui.spacing().interact_size.y),
    );
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), text));
    if ui.is_rect_visible(rect) {
        let v = ui.style().interact(&resp);
        ui.painter().rect(
            rect.expand(v.expansion),
            v.corner_radius,
            v.weak_bg_fill,
            v.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let spot = Rect::from_min_size(pos2(rect.left() + pad.x, rect.center().y - side / 2.0), vec2(side, side));
        paint_icon(ui.painter(), spot, icon, v.text_color());
        let at = pos2(spot.right() + gap, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(at, galley, v.text_color());
    }
    resp
}
